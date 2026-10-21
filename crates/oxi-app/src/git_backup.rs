//! Local Git backup of the vault (like the "Obsidian Git" plugin, without a
//! remote).
//!
//! Everything goes through the system `git` binary, run as a sub-process
//! (`git -C <vault> ...`) on a background thread; results come back through
//! a channel polled by [`AppState::git_tick`]. The UI never blocks on git.
//!
//! This module has no egui code: the interface lives in `ui/git_ui.rs`.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

use oxi_core::NotePath;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Per-vault settings file, in `.obsidian/`.
pub const CONFIG_FILE: &str = "oxinote-git.json";
pub const DEFAULT_TEMPLATE: &str = "vault backup: {{date}}";

/// Delay before retrying when the repository is locked.
const LOCK_RETRY_SECS: f64 = 30.0;
/// Delay between two refreshes of the "N changes" counter.
const STATUS_EVERY_SECS: f64 = 20.0;
/// Above this many entries the ignored-files lookup is skipped.
const MAX_IGNORE_QUERY: usize = 30_000;

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitSettings {
    /// Commit automatically every `interval_minutes` when there are changes.
    pub auto_backup: bool,
    pub interval_minutes: u32,
    /// Commit when the app exits.
    pub backup_on_exit: bool,
    /// Commit message; `{{date}}` and `{{numFiles}}` are replaced.
    pub message_template: String,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            auto_backup: false,
            interval_minutes: 10,
            backup_on_exit: false,
            message_template: DEFAULT_TEMPLATE.into(),
        }
    }
}

impl GitSettings {
    pub fn interval_secs(&self) -> f64 {
        f64::from(self.interval_minutes.clamp(1, 24 * 60)) * 60.0
    }
}

/// Replace `{{date}}` (`YYYY-MM-DD HH:mm`) and `{{numFiles}}`.
pub fn render_message(template: &str, date: &str, num_files: usize) -> String {
    let t = if template.trim().is_empty() {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    t.replace("{{date}}", date)
        .replace("{{numFiles}}", &num_files.to_string())
}

fn now_date_string() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "à l'instant", "il y a 3 min", "il y a 2 h", "il y a 4 j".
pub fn relative_time(now: i64, then: i64) -> String {
    let d = (now - then).max(0);
    match d {
        0..=44 => "à l'instant".into(),
        45..=3599 => format!("il y a {} min", (d / 60).max(1)),
        3600..=86_399 => format!("il y a {} h", d / 3600),
        _ if d < 86_400 * 60 => format!("il y a {} j", d / 86_400),
        _ => format!("il y a {} mois", d / (86_400 * 30)),
    }
}

/// Local date and time of a unix timestamp, `YYYY-MM-DD HH:mm`.
pub fn format_timestamp(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
}

// ---------------------------------------------------------------------------
// Running git
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// The `git` binary cannot be run.
    Missing,
    /// `index.lock` exists: another git operation is running.
    Locked,
    Failed(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Missing => {
                write!(f, "Git est introuvable : installe-le et relance Oxinote.")
            }
            GitError::Locked => write!(f, "Le dépôt est occupé (index.lock)."),
            GitError::Failed(m) => write!(f, "{m}"),
        }
    }
}

struct Out {
    ok: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

impl Out {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    /// Best message for an error: stderr, else stdout.
    fn message(&self) -> String {
        let e = self.stderr.trim();
        if e.is_empty() {
            self.text().trim().to_string()
        } else {
            e.to_string()
        }
    }
}

/// Run `git -C dir [-c cfg]... args`, optionally feeding `stdin`.
fn run(dir: &Path, cfg: &[String], args: &[&str], stdin: Option<Vec<u8>>) -> Result<Out, GitError> {
    let mut cmd = Command::new("git");
    // `check-ignore` does not accept pathspec magic.
    if args.first() != Some(&"check-ignore") {
        cmd.arg("--literal-pathspecs");
    }
    cmd.arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.quotepath=false"]);
    for c in cfg {
        cmd.arg("-c").arg(c);
    }
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::Missing
        } else {
            GitError::Failed(e.to_string())
        }
    })?;
    // Feed stdin from another thread so a full stdout pipe cannot deadlock.
    let writer = stdin.and_then(|data| {
        child.stdin.take().map(|mut s| {
            std::thread::spawn(move || {
                let _ = s.write_all(&data);
            })
        })
    });
    let out = child
        .wait_with_output()
        .map_err(|e| GitError::Failed(e.to_string()))?;
    if let Some(w) = writer {
        let _ = w.join();
    }
    Ok(Out {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Like [`run`], failing on a non-zero exit status.
fn run_ok(dir: &Path, cfg: &[String], args: &[&str]) -> Result<Out, GitError> {
    let out = run(dir, cfg, args, None)?;
    if out.ok {
        Ok(out)
    } else if out.stderr.contains("index.lock") {
        Err(GitError::Locked)
    } else {
        Err(GitError::Failed(out.message()))
    }
}

/// `git --version` (also tells whether git can be run at all).
pub fn git_version() -> Result<String, GitError> {
    let out = run_ok(Path::new("."), &[], &["--version"])?;
    Ok(out.text().trim().to_string())
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

/// The repository holding the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub toplevel: PathBuf,
    /// Path of the vault inside the repository: empty when the vault is the
    /// repository root, else `sub/dir/` (with a trailing slash).
    pub prefix: String,
}

impl RepoInfo {
    /// The repository root is a parent folder of the vault.
    pub fn is_nested(&self) -> bool {
        !self.prefix.is_empty()
    }
}

/// The repository containing `vault`, if any.
pub fn detect_repo(vault: &Path) -> Result<Option<RepoInfo>, GitError> {
    let out = run(
        vault,
        &[],
        &["rev-parse", "--show-toplevel", "--show-prefix"],
        None,
    )?;
    if !out.ok {
        if out.stderr.contains("not a git repository") {
            return Ok(None);
        }
        return Err(GitError::Failed(out.message()));
    }
    let text = out.text();
    let mut lines = text.lines();
    let top = lines.next().unwrap_or("").trim();
    if top.is_empty() {
        return Ok(None);
    }
    Ok(Some(RepoInfo {
        toplevel: PathBuf::from(top),
        prefix: lines.next().unwrap_or("").trim().to_string(),
    }))
}

/// `git init` the vault (branch `main`), create the default `.gitignore`
/// and make a first commit.
pub fn init_repo(vault: &Path) -> Result<RepoInfo, GitError> {
    run_ok(vault, &["init.defaultBranch=main".into()], &["init", "-q"])?;
    ensure_default_gitignore(vault)?;
    let info = detect_repo(vault)?.ok_or_else(|| GitError::Failed("git init a échoué".into()))?;
    backup(vault, &info, "Initial backup: {{date}}")?;
    Ok(info)
}

fn lock_exists(vault: &Path) -> bool {
    match run(vault, &[], &["rev-parse", "--git-path", "index.lock"], None) {
        Ok(o) if o.ok => {
            let p = PathBuf::from(o.text().trim());
            let p = if p.is_absolute() { p } else { vault.join(p) };
            p.exists()
        }
        _ => false,
    }
}

/// `-c user.name=... -c user.email=...` when the git config has no identity.
fn identity_cfg(vault: &Path) -> Vec<String> {
    let has = |key: &str| {
        run(vault, &[], &["config", key], None)
            .map(|o| o.ok && !o.text().trim().is_empty())
            .unwrap_or(false)
    };
    if has("user.name") && has("user.email") {
        Vec::new()
    } else {
        vec![
            "user.name=Oxinote".into(),
            "user.email=oxinote@localhost".into(),
        ]
    }
}

/// Number of changed entries (modified, added, deleted, untracked) inside
/// the vault.
pub fn status_count(vault: &Path) -> Result<usize, GitError> {
    let out = run_ok(vault, &[], &["status", "--porcelain", "-uall", "--", "."])?;
    Ok(out.text().lines().filter(|l| !l.trim().is_empty()).count())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupOutcome {
    /// No change: nothing was committed.
    Nothing,
    Committed {
        sha: String,
        files: usize,
        message: String,
    },
}

/// Stage and commit every change of the vault. Does nothing (no empty
/// commit) when there is no change.
pub fn backup(vault: &Path, info: &RepoInfo, template: &str) -> Result<BackupOutcome, GitError> {
    if lock_exists(vault) {
        return Err(GitError::Locked);
    }
    let n = status_count(vault)?;
    if n == 0 {
        return Ok(BackupOutcome::Nothing);
    }
    run_ok(vault, &[], &["add", "-A", "--", "."])?;
    let message = render_message(template, &now_date_string(), n);
    let cfg = identity_cfg(vault);
    let mut args = vec!["commit", "-q", "-m", message.as_str()];
    if info.is_nested() {
        // Only the vault's files, whatever else is staged in the repository.
        args.extend(["--", "."]);
    }
    let out = run(vault, &cfg, &args, None)?;
    if !out.ok {
        let all = format!("{}{}", out.text(), out.stderr);
        if all.contains("nothing to commit") || all.contains("nothing added to commit") {
            return Ok(BackupOutcome::Nothing);
        }
        if all.contains("index.lock") {
            return Err(GitError::Locked);
        }
        return Err(GitError::Failed(out.message()));
    }
    let sha = run_ok(vault, &[], &["rev-parse", "--short", "HEAD"])?
        .text()
        .trim()
        .to_string();
    Ok(BackupOutcome::Committed {
        sha,
        files: n,
        message,
    })
}

/// Unix time of the last commit touching the vault.
pub fn last_commit_time(vault: &Path) -> Option<i64> {
    let out = run(vault, &[], &["log", "-1", "--format=%ct", "--", "."], None).ok()?;
    if !out.ok {
        return None;
    }
    out.text().trim().parse().ok()
}

// ---------------------------------------------------------------------------
// .gitignore
// ---------------------------------------------------------------------------

pub const GITIGNORE: &str = ".gitignore";

/// Default `.gitignore` written by "Initialiser le dépôt Git du vault".
pub const DEFAULT_GITIGNORE: &str = "\
# Fichiers locaux d'Oxinote / Obsidian : état de l'espace de travail
.obsidian/workspace.json
.obsidian/workspace-mobile.json
.obsidian/workspace-oxinote.json
# Caches
.obsidian/cache
# Corbeille et fichiers système
.trash/
.DS_Store
";

/// Rules a user can switch on and off in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    ObsidianConfig,
    HeavyAttachments,
    Trash,
    Workspace,
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::ObsidianConfig,
        Preset::HeavyAttachments,
        Preset::Trash,
        Preset::Workspace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::ObsidianConfig => "Ignorer la configuration .obsidian",
            Preset::HeavyAttachments => "Ignorer les pièces jointes lourdes (*.pdf, *.mp4, *.zip)",
            Preset::Trash => "Ignorer la corbeille",
            Preset::Workspace => "Ignorer l'état de l'espace de travail",
        }
    }

    pub fn rules(self) -> &'static [&'static str] {
        match self {
            Preset::ObsidianConfig => &[".obsidian/"],
            Preset::HeavyAttachments => &["*.pdf", "*.mp4", "*.zip"],
            Preset::Trash => &[".trash/"],
            Preset::Workspace => &[
                ".obsidian/workspace.json",
                ".obsidian/workspace-mobile.json",
                ".obsidian/workspace-oxinote.json",
            ],
        }
    }
}

fn has_rule(text: &str, rule: &str) -> bool {
    text.lines().any(|l| l.trim() == rule)
}

/// Append `rule` as its own line unless it is already there.
pub fn add_rule(text: &str, rule: &str) -> String {
    if has_rule(text, rule) {
        return text.to_string();
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(rule);
    out.push('\n');
    out
}

/// Remove every line equal to one of `rules`; `None` when none matched.
pub fn remove_rules(text: &str, rules: &[String]) -> Option<String> {
    let mut found = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let hit = rules.iter().any(|r| l.trim() == r);
            found |= hit;
            !hit
        })
        .collect();
    if !found {
        return None;
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

pub fn preset_active(text: &str, preset: Preset) -> bool {
    preset.rules().iter().all(|r| has_rule(text, r))
}

pub fn set_preset(text: &str, preset: Preset, on: bool) -> String {
    if on {
        preset
            .rules()
            .iter()
            .fold(text.to_string(), |t, r| add_rule(&t, r))
    } else {
        let rules: Vec<String> = preset.rules().iter().map(|r| (*r).to_string()).collect();
        remove_rules(text, &rules).unwrap_or_else(|| text.to_string())
    }
}

fn escape_pattern(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if matches!(c, '*' | '?' | '[' | '\\') || (i == 0 && matches!(c, '#' | '!')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The line added to the `.gitignore` for a file or a folder (trailing `/`).
/// Root-level names are anchored with a leading `/`.
pub fn ignore_rule(path: &str, is_dir: bool) -> String {
    let base = escape_pattern(path.trim_matches('/'));
    let anchored = if base.contains('/') {
        base
    } else {
        format!("/{base}")
    };
    if is_dir {
        format!("{anchored}/")
    } else {
        anchored
    }
}

/// Every spelling of the rule for `path` a user may have written.
pub fn rule_variants(path: &str, is_dir: bool) -> Vec<String> {
    let base = escape_pattern(path.trim_matches('/'));
    let mut v = vec![base.clone(), format!("/{base}")];
    if is_dir {
        v.push(format!("{base}/"));
        v.push(format!("/{base}/"));
    }
    v
}

fn gitignore_path(vault: &Path) -> PathBuf {
    vault.join(GITIGNORE)
}

pub fn read_gitignore(vault: &Path) -> String {
    std::fs::read_to_string(gitignore_path(vault)).unwrap_or_default()
}

pub fn write_gitignore(vault: &Path, text: &str) -> Result<(), GitError> {
    std::fs::write(gitignore_path(vault), text)
        .map_err(|e| GitError::Failed(format!("Impossible d'écrire .gitignore : {e}")))
}

/// Create the default `.gitignore`, or add the missing default rules to an
/// existing one.
pub fn ensure_default_gitignore(vault: &Path) -> Result<(), GitError> {
    let current = read_gitignore(vault);
    if current.is_empty() {
        return write_gitignore(vault, DEFAULT_GITIGNORE);
    }
    let mut text = current.clone();
    for line in DEFAULT_GITIGNORE.lines().filter(|l| !l.starts_with('#')) {
        text = add_rule(&text, line);
    }
    if text != current {
        write_gitignore(vault, &text)?;
    }
    Ok(())
}

/// Paths (among `paths`, relative to the vault, `/`-separated) that git
/// ignores. Folders must end with `/`.
pub fn check_ignore(vault: &Path, paths: &[String]) -> Result<HashSet<String>, GitError> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let out = run(vault, &[], &["check-ignore", "--stdin", "-z"], Some(input))?;
    // Exit status 1 = nothing is ignored.
    if !out.ok && out.code != Some(1) {
        return Err(GitError::Failed(out.message()));
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

/// The `.gitignore` rule responsible for ignoring `path`, with its source.
pub fn why_ignored(vault: &Path, path: &str) -> Option<String> {
    let out = run(vault, &[], &["check-ignore", "-v", "--", path], None).ok()?;
    let text = out.text();
    // `source:line:pattern<TAB>path`
    let head = text.lines().next()?.split('\t').next()?;
    let mut parts = head.splitn(3, ':');
    let source = parts.next()?;
    let _line = parts.next()?;
    let pattern = parts.next()?;
    Some(format!("{pattern} ({source})"))
}

/// Is `path` (file or folder) tracked by git?
pub fn is_tracked(vault: &Path, path: &str) -> bool {
    run(vault, &[], &["ls-files", "--", path], None)
        .map(|o| o.ok && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// `git rm -r --cached`: stop tracking `path`, keep the file on disk.
pub fn untrack(vault: &Path, path: &str) -> Result<(), GitError> {
    run_ok(
        vault,
        &[],
        &["rm", "-r", "--cached", "-q", "--ignore-unmatch", "--", path],
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// History of a file
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub time: i64,
    pub subject: String,
    /// Path of the file in this commit (it differs after a rename).
    pub path: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
}

/// Commits touching `rel` (path relative to the vault), newest first,
/// following renames.
pub fn log_file(
    vault: &Path,
    info: &RepoInfo,
    rel: &str,
    limit: usize,
) -> Result<Vec<Commit>, GitError> {
    let n = format!("-n{limit}");
    let out = run(
        vault,
        &[],
        &[
            "log",
            "--follow",
            "--name-status",
            "--format=%x1e%H%x1f%ct%x1f%s",
            &n,
            "--",
            rel,
        ],
        None,
    )?;
    if !out.ok {
        // A repository without any commit yet has no history.
        if out.stderr.contains("does not have any commits") {
            return Ok(Vec::new());
        }
        return Err(GitError::Failed(out.message()));
    }
    Ok(parse_log(&out.text(), &info.prefix, rel))
}

fn parse_log(text: &str, prefix: &str, rel: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    for record in text.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let Some(head) = lines.next() else { continue };
        let mut f = head.splitn(3, '\u{1f}');
        let (Some(sha), Some(time), Some(subject)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        // `M\tpath` or `R100\told\tnew`: the last field is the path then.
        let path = lines
            .rfind(|l| !l.trim().is_empty())
            .and_then(|l| l.split('\t').next_back())
            .map(|p| p.strip_prefix(prefix).unwrap_or(p).to_string())
            .unwrap_or_else(|| rel.to_string());
        commits.push(Commit {
            sha: sha.to_string(),
            time: time.trim().parse().unwrap_or(0),
            subject: subject.to_string(),
            path,
        });
    }
    commits
}

/// Contents of `path` (relative to the vault) at commit `sha`.
pub fn file_at(vault: &Path, sha: &str, path: &str) -> Result<String, GitError> {
    let out = run_ok(vault, &[], &["show", &format!("{sha}:./{path}")])?;
    Ok(out.text())
}

// ---------------------------------------------------------------------------
// Line diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
    /// Skipped unchanged lines.
    Gap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

fn split_lines(s: &str) -> Vec<&str> {
    let s = s.strip_suffix('\n').unwrap_or(s);
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect()
    }
}

/// Line diff from `old` to `new`, keeping `context` unchanged lines around
/// each change. Line endings are ignored (`core.autocrlf`).
pub fn diff_lines(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let a = split_lines(old);
    let b = split_lines(new);
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);

    let mut script: Vec<(DiffKind, &str)> =
        a[..pre].iter().map(|l| (DiffKind::Context, *l)).collect();
    let (n, m) = (ma.len(), mb.len());
    if n.saturating_mul(m) > 4_000_000 {
        script.extend(ma.iter().map(|l| (DiffKind::Removed, *l)));
        script.extend(mb.iter().map(|l| (DiffKind::Added, *l)));
    } else {
        // Longest common subsequence of the differing middle.
        let w = m + 1;
        let mut t = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * w + j] = if ma[i] == mb[j] {
                    t[(i + 1) * w + j + 1] + 1
                } else {
                    t[(i + 1) * w + j].max(t[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if ma[i] == mb[j] {
                script.push((DiffKind::Context, ma[i]));
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                script.push((DiffKind::Removed, ma[i]));
                i += 1;
            } else {
                script.push((DiffKind::Added, mb[j]));
                j += 1;
            }
        }
        script.extend(ma[i..].iter().map(|l| (DiffKind::Removed, *l)));
        script.extend(mb[j..].iter().map(|l| (DiffKind::Added, *l)));
    }
    script.extend(a[a.len() - suf..].iter().map(|l| (DiffKind::Context, *l)));

    // Keep only the context near changes.
    let changed: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, (k, _))| *k != DiffKind::Context)
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let mut keep = vec![false; script.len()];
    for &c in &changed {
        let from = c.saturating_sub(context);
        let to = (c + context).min(script.len() - 1);
        keep[from..=to].iter_mut().for_each(|k| *k = true);
    }
    let mut out = Vec::new();
    let mut gap = false;
    for (i, (kind, text)) in script.into_iter().enumerate() {
        if keep[i] {
            out.push(DiffLine {
                kind,
                text: text.to_string(),
            });
            gap = false;
        } else if !gap && !out.is_empty() {
            out.push(DiffLine {
                kind: DiffKind::Gap,
                text: String::new(),
            });
            gap = true;
        }
    }
    if out.last().is_some_and(|l| l.kind == DiffKind::Gap) {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Background service
// ---------------------------------------------------------------------------

/// Is the `git` binary usable?
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Availability {
    #[default]
    Unknown,
    Missing,
    Ready,
}

/// Something the user must confirm in a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitConfirm {
    /// Also stop tracking an already tracked file / folder.
    Untrack { path: String, is_dir: bool },
    /// Replace the buffer of `path` (the note of the history) by an old version.
    Restore {
        path: NotePath,
        short: String,
        text: String,
    },
}

/// State of the "Historique de version" panel.
#[derive(Debug, Default)]
pub struct History {
    pub path: Option<NotePath>,
    pub entries: Vec<Commit>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<usize>,
    /// Compare with the previous commit instead of the current text.
    pub against_previous: bool,
    /// Text of the selected version and of the one before it.
    pub version: Option<(String, String)>,
    /// Cached diff and the inputs it was computed from.
    pub diff: Option<(u64, Vec<DiffLine>)>,
    stamp: u64,
    asked: Option<(NotePath, u64)>,
}

enum Msg {
    Detected {
        version: Result<String, GitError>,
        repo: Result<Option<RepoInfo>, GitError>,
        last_commit: Option<i64>,
    },
    Initialized(Result<RepoInfo, GitError>),
    Status(Result<usize, GitError>),
    BackedUp {
        manual: bool,
        result: Result<BackupOutcome, GitError>,
    },
    Ignored(Result<HashSet<String>, GitError>, (usize, usize, u64)),
    Log {
        path: NotePath,
        result: Result<Vec<Commit>, GitError>,
    },
    Version {
        sha: String,
        result: Result<(String, String), GitError>,
    },
    Tracked {
        path: String,
        is_dir: bool,
        tracked: bool,
    },
    Untracked(Result<(), GitError>, String),
    WhyIgnored(String, Option<String>),
}

pub struct GitService {
    pub settings: GitSettings,
    pub avail: Availability,
    pub version: String,
    pub repo: Option<RepoInfo>,
    /// Repository detection finished.
    pub checked: bool,
    detect_started: bool,
    /// Entries changed since the last commit.
    pub changes: Option<usize>,
    /// Unix time of the last commit.
    pub last_commit: Option<i64>,
    /// Background jobs in flight.
    pub running: u32,
    backup_running: bool,
    /// Message of the last error toast (not repeated).
    pub last_error: Option<String>,
    pub init_error: Option<String>,
    settings_before_init: Option<GitSettings>,
    /// Paths git ignores (files, and folders ending with `/`).
    pub ignored: HashSet<String>,
    ignore_asked: Option<(usize, usize, u64)>,
    ignore_running: bool,
    gitignore_stamp: Option<(u64, std::time::SystemTime)>,
    gitignore_checked_at: f64,
    gitignore_version: u64,
    /// Contents of the `.gitignore` editor.
    pub gitignore_text: String,
    /// What the file on disk holds (to tell unsaved edits).
    pub gitignore_saved: String,
    pub gitignore_loaded: bool,
    /// Scroll the settings page to the `.gitignore` section (screenshots).
    pub scroll_gitignore: bool,
    pub history: History,
    pub confirm: Option<GitConfirm>,
    next_status: f64,
    next_backup: f64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for GitService {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            settings: GitSettings::default(),
            avail: Availability::Unknown,
            version: String::new(),
            repo: None,
            checked: false,
            detect_started: false,
            changes: None,
            last_commit: None,
            running: 0,
            backup_running: false,
            last_error: None,
            init_error: None,
            settings_before_init: None,
            ignored: HashSet::new(),
            ignore_asked: None,
            ignore_running: false,
            gitignore_stamp: None,
            gitignore_checked_at: f64::MIN,
            gitignore_version: 0,
            gitignore_text: String::new(),
            gitignore_saved: String::new(),
            gitignore_loaded: false,
            scroll_gitignore: false,
            history: History::default(),
            confirm: None,
            next_status: 0.0,
            next_backup: 0.0,
            tx,
            rx,
        }
    }
}

impl GitService {
    fn spawn(&mut self, job: impl FnOnce() -> Msg + Send + 'static) {
        self.running += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    pub fn is_backing_up(&self) -> bool {
        self.backup_running
    }

    /// Git can be used on this vault.
    pub fn usable(&self) -> bool {
        self.avail == Availability::Ready && self.repo.is_some()
    }

    /// Forget cached results (after a commit, a restore...).
    fn touch(&mut self) {
        self.history.stamp += 1;
    }
}

impl AppState {
    // ----- settings ----------------------------------------------------------

    /// Read the per-vault settings (called once when the vault opens).
    pub fn git_load_settings(&mut self) {
        if let Some(s) = crate::legacy::read_config_file(&*self.vault, CONFIG_FILE)
            .and_then(|s| serde_json::from_str::<GitSettings>(&s).ok())
        {
            self.git.settings = s;
        }
        // A `.gitignore` written by ObsiRust ignores the old workspace file.
        if let Some(root) = self.git_root() {
            let current = read_gitignore(&root);
            let updated = crate::legacy::gitignore_with_new_rules(&current);
            if updated != current
                && let Err(e) = write_gitignore(&root, &updated)
            {
                self.error(e.to_string());
            }
        }
    }

    pub fn git_save_settings(&mut self) {
        let json = serde_json::to_string_pretty(&self.git.settings).unwrap_or_default();
        if let Err(e) = self.vault.write_config_file(CONFIG_FILE, &json) {
            self.error(format!("Impossible d'enregistrer les réglages Git : {e}"));
        }
        // Take the new interval into account.
        self.git.next_backup = self.now + self.git.settings.interval_secs();
    }

    fn git_root(&self) -> Option<PathBuf> {
        self.vault.root()
    }

    // ----- scheduling --------------------------------------------------------

    /// Poll the background jobs and start the due ones. Cheap: called on
    /// every frame.
    pub fn git_tick(&mut self) {
        self.git_poll();
        let Some(root) = self.git_root() else {
            return;
        };
        let now = self.now;
        if !self.git.detect_started {
            self.git.detect_started = true;
            self.git.spawn({
                let root = root.clone();
                move || Msg::Detected {
                    version: git_version(),
                    repo: detect_repo(&root),
                    last_commit: last_commit_time(&root),
                }
            });
            return;
        }
        if !self.git.usable() {
            return;
        }
        let Some(info) = self.git.repo.clone() else {
            return;
        };
        // `.gitignore` edited outside the settings page.
        if now - self.git.gitignore_checked_at >= 2.0 {
            self.git.gitignore_checked_at = now;
            let stamp = std::fs::metadata(gitignore_path(&root))
                .ok()
                .and_then(|m| m.modified().ok().map(|t| (m.len(), t)));
            if stamp != self.git.gitignore_stamp {
                self.git.gitignore_stamp = stamp;
                self.git.gitignore_version += 1;
                self.git.gitignore_loaded = false;
            }
        }
        self.git_refresh_ignored(&root);
        if self.git.backup_running {
            return;
        }
        if self.git.settings.auto_backup && now >= self.git.next_backup {
            self.git.next_backup = now + self.git.settings.interval_secs();
            self.git_start_backup(false, &root, &info);
            return;
        }
        if now >= self.git.next_status {
            self.git.next_status = now + STATUS_EVERY_SECS;
            self.git.spawn(move || Msg::Status(status_count(&root)));
        }
    }

    fn git_refresh_ignored(&mut self, root: &Path) {
        let key = (
            self.files.len(),
            self.folders.len(),
            self.git.gitignore_version,
        );
        if self.git.ignore_asked == Some(key) || self.git.ignore_running {
            return;
        }
        self.git.ignore_asked = Some(key);
        if self.files.len() + self.folders.len() > MAX_IGNORE_QUERY {
            return;
        }
        let mut paths: Vec<String> = self.files.iter().map(|f| f.as_str().to_string()).collect();
        paths.extend(self.folders.iter().map(|f| format!("{f}/")));
        self.git.ignore_running = true;
        let root = root.to_path_buf();
        self.git
            .spawn(move || Msg::Ignored(check_ignore(&root, &paths), key));
    }

    fn git_start_backup(&mut self, manual: bool, root: &Path, info: &RepoInfo) {
        // What is being edited goes to disk first.
        self.save_all();
        self.git.backup_running = true;
        let (root, info) = (root.to_path_buf(), info.clone());
        let template = self.git.settings.message_template.clone();
        self.git.spawn(move || Msg::BackedUp {
            manual,
            result: backup(&root, &info, &template),
        });
    }

    /// "Git : sauvegarder maintenant".
    pub fn git_backup_now(&mut self) {
        let (Some(root), Some(info)) = (self.git_root(), self.git.repo.clone()) else {
            self.info(
                "Le vault n'est pas un dépôt Git : initialise-le dans Réglages > Sauvegarde (Git).",
            );
            self.open_settings(crate::workbench::SettingsPage::GitBackup);
            return;
        };
        if self.git.avail != Availability::Ready {
            self.error(GitError::Missing.to_string());
            return;
        }
        if self.git.backup_running {
            self.info("Une sauvegarde est déjà en cours.");
            return;
        }
        self.git_start_backup(true, &root, &info);
    }

    /// Commit synchronously when the app closes (if enabled).
    pub fn git_backup_on_exit(&mut self) {
        if self.ws.read_only || !self.git.settings.backup_on_exit || !self.git.usable() {
            return;
        }
        let (Some(root), Some(info)) = (self.git_root(), self.git.repo.clone()) else {
            return;
        };
        self.save_all();
        if let Err(e) = backup(&root, &info, &self.git.settings.message_template) {
            eprintln!("git backup on exit: {e}");
        }
    }

    /// "Initialiser le dépôt Git du vault".
    pub fn git_init(&mut self) {
        let Some(root) = self.git_root() else {
            return;
        };
        if self.git.avail != Availability::Ready {
            return;
        }
        self.git.init_error = None;
        self.save_all();
        // Enabled by the init; written before the first commit so that the
        // commit contains it.
        self.git.settings_before_init = Some(self.git.settings.clone());
        self.git.settings.auto_backup = true;
        self.git.settings.backup_on_exit = true;
        self.git_save_settings();
        self.git.spawn(move || Msg::Initialized(init_repo(&root)));
    }

    // ----- results -----------------------------------------------------------

    fn git_report(&mut self, e: GitError) {
        let msg = format!("Git : {e}");
        if self.git.last_error.as_deref() != Some(msg.as_str()) {
            self.git.last_error = Some(msg.clone());
            self.error(msg);
        }
    }

    fn git_poll(&mut self) {
        while let Ok(msg) = self.git.rx.try_recv() {
            self.git.running = self.git.running.saturating_sub(1);
            self.git_handle(msg);
        }
    }

    fn git_handle(&mut self, msg: Msg) {
        let now = self.now;
        match msg {
            Msg::Detected {
                version,
                repo,
                last_commit,
            } => {
                self.git.checked = true;
                match version {
                    Ok(v) => {
                        self.git.avail = Availability::Ready;
                        self.git.version = v;
                    }
                    Err(_) => {
                        self.git.avail = Availability::Missing;
                        return;
                    }
                }
                match repo {
                    Ok(r) => {
                        self.git.repo = r;
                        self.git.last_commit = last_commit;
                        self.git.next_backup = now + self.git.settings.interval_secs();
                        self.git.next_status = now;
                        self.git.touch();
                    }
                    Err(e) => self.git.init_error = Some(e.to_string()),
                }
            }
            Msg::Initialized(res) => match res {
                Ok(info) => {
                    self.git.repo = Some(info);
                    self.git.settings_before_init = None;
                    self.git.next_backup = now + self.git.settings.interval_secs();
                    self.git.last_commit = self.git_root().and_then(|r| last_commit_time(&r));
                    self.git.changes = Some(0);
                    self.git.gitignore_loaded = false;
                    self.git.gitignore_version += 1;
                    self.git.touch();
                    self.info("Dépôt Git initialisé : premier commit créé.");
                }
                Err(e) => {
                    if let Some(prev) = self.git.settings_before_init.take() {
                        self.git.settings = prev;
                        self.git_save_settings();
                    }
                    self.git.init_error = Some(e.to_string());
                    self.git_report(e);
                }
            },
            Msg::Status(res) => match res {
                Ok(n) => {
                    self.git.changes = Some(n);
                    self.git.last_error = None;
                }
                Err(GitError::Locked) => {}
                Err(e) => self.git_report(e),
            },
            Msg::BackedUp { manual, result } => {
                self.git.backup_running = false;
                match result {
                    Ok(BackupOutcome::Nothing) => {
                        self.git.changes = Some(0);
                        self.git.last_error = None;
                        if manual {
                            self.info("Aucune modification à sauvegarder.");
                        }
                    }
                    Ok(BackupOutcome::Committed {
                        sha,
                        files,
                        message,
                    }) => {
                        self.git.changes = Some(0);
                        self.git.last_error = None;
                        self.git.last_commit = Some(unix_now());
                        self.git.touch();
                        if manual {
                            self.info(format!(
                                "Sauvegarde Git {sha} : {files} modification(s) ({message})"
                            ));
                        }
                    }
                    Err(GitError::Locked) => {
                        // Another git operation: try again soon, quietly.
                        self.git.next_backup = now + LOCK_RETRY_SECS;
                        if manual {
                            self.info("Dépôt occupé, nouvelle tentative dans 30 s.");
                        }
                    }
                    Err(e) => self.git_report(e),
                }
            }
            Msg::Ignored(res, key) => {
                self.git.ignore_running = false;
                match res {
                    Ok(set) => {
                        self.git.ignored = set;
                    }
                    Err(e) => {
                        self.git.ignore_asked = Some(key);
                        eprintln!("git check-ignore: {e}");
                    }
                }
                // The inputs changed while it was running: ask again.
                let now_key = (
                    self.files.len(),
                    self.folders.len(),
                    self.git.gitignore_version,
                );
                if now_key != key {
                    self.git.ignore_asked = None;
                }
            }
            Msg::Log { path, result } => {
                if self.git.history.path.as_ref() != Some(&path) {
                    return;
                }
                self.git.history.loading = false;
                match result {
                    Ok(entries) => {
                        self.git.history.error = None;
                        self.git.history.entries = entries;
                        self.git.history.selected = None;
                        self.git.history.version = None;
                        self.git.history.diff = None;
                        if !self.git.history.entries.is_empty() {
                            self.git_select_commit(0);
                        }
                    }
                    Err(e) => self.git.history.error = Some(e.to_string()),
                }
            }
            Msg::Version { sha, result } => {
                let h = &mut self.git.history;
                let current = h.selected.and_then(|i| h.entries.get(i));
                if current.map(|c| c.sha.as_str()) != Some(sha.as_str()) {
                    return;
                }
                match result {
                    Ok(v) => {
                        h.version = Some(v);
                        h.diff = None;
                        h.error = None;
                    }
                    Err(e) => h.error = Some(e.to_string()),
                }
            }
            Msg::Tracked {
                path,
                is_dir,
                tracked,
            } => {
                if tracked {
                    self.git.confirm = Some(GitConfirm::Untrack { path, is_dir });
                }
            }
            Msg::Untracked(res, path) => match res {
                Ok(()) => {
                    self.info(format!("« {path} » n'est plus suivi par Git."));
                    self.git.next_status = now;
                }
                Err(e) => self.git_report(e),
            },
            Msg::WhyIgnored(path, why) => match why {
                Some(w) => self.info(format!(
                    "« {path} » est ignoré par un motif plus général : {w}. Modifie le .gitignore dans les réglages."
                )),
                None => self.info(format!("« {path} » n'est pas ignoré.")),
            },
        }
    }

    // ----- .gitignore --------------------------------------------------------

    /// Load the `.gitignore` into the settings editor if needed.
    pub fn git_load_gitignore(&mut self) {
        if self.git.gitignore_loaded {
            return;
        }
        if let Some(root) = self.git_root() {
            self.git.gitignore_text = read_gitignore(&root);
            self.git.gitignore_saved = self.git.gitignore_text.clone();
        }
        self.git.gitignore_loaded = true;
    }

    /// Save text (from the editor or a preset) as the `.gitignore`.
    pub fn git_write_gitignore(&mut self, text: String) {
        let Some(root) = self.git_root() else {
            return;
        };
        match write_gitignore(&root, &text) {
            Ok(()) => {
                self.git.gitignore_saved = text.clone();
                self.git.gitignore_text = text;
                self.git.gitignore_loaded = true;
                self.git.gitignore_version += 1;
                self.git.gitignore_stamp = std::fs::metadata(gitignore_path(&root))
                    .ok()
                    .and_then(|m| m.modified().ok().map(|t| (m.len(), t)));
                self.git.next_status = self.now;
            }
            Err(e) => self.git_report(e),
        }
    }

    /// Explorer: "Ignorer dans Git".
    pub fn git_ignore_path(&mut self, path: &str, is_dir: bool) {
        let Some(root) = self.git_root() else {
            return;
        };
        let text = add_rule(&read_gitignore(&root), &ignore_rule(path, is_dir));
        self.git_write_gitignore(text);
        self.info(format!("« {path} » ajouté au .gitignore."));
        // Offer to stop tracking it when it already is.
        let tracked_path = path.to_string();
        self.git.spawn(move || Msg::Tracked {
            tracked: is_tracked(&root, &tracked_path),
            path: tracked_path,
            is_dir,
        });
    }

    /// Explorer: "Ne plus ignorer".
    pub fn git_unignore_path(&mut self, path: &str, is_dir: bool) {
        let Some(root) = self.git_root() else {
            return;
        };
        let rules = rule_variants(path, is_dir);
        match remove_rules(&read_gitignore(&root), &rules) {
            Some(text) => {
                self.git_write_gitignore(text);
                self.info(format!("« {path} » n'est plus dans le .gitignore."));
            }
            None => {
                let probe = if is_dir {
                    format!("{path}/")
                } else {
                    path.to_string()
                };
                self.git.spawn(move || {
                    let why = why_ignored(&root, &probe);
                    Msg::WhyIgnored(probe, why)
                });
            }
        }
    }

    /// Confirmed: `git rm --cached`.
    pub fn git_untrack(&mut self, path: &str) {
        let Some(root) = self.git_root() else {
            return;
        };
        let path = path.to_string();
        self.git
            .spawn(move || Msg::Untracked(untrack(&root, &path), path));
    }

    // ----- history -----------------------------------------------------------

    /// Make sure the history of `path` is loaded (or being loaded).
    pub fn git_request_history(&mut self, path: &NotePath) {
        let (Some(root), Some(info)) = (self.git_root(), self.git.repo.clone()) else {
            return;
        };
        let h = &mut self.git.history;
        let key = (path.clone(), h.stamp);
        if h.asked.as_ref() == Some(&key) {
            return;
        }
        if h.path.as_ref() != Some(path) {
            h.entries.clear();
            h.selected = None;
            h.version = None;
            h.diff = None;
        }
        h.asked = Some(key);
        h.path = Some(path.clone());
        h.loading = true;
        let path = path.clone();
        self.git.spawn(move || Msg::Log {
            result: log_file(&root, &info, path.as_str(), 200),
            path,
        });
    }

    /// Select a commit and load its text (and its predecessor's).
    pub fn git_select_commit(&mut self, index: usize) {
        let Some(root) = self.git_root() else {
            return;
        };
        let h = &mut self.git.history;
        let Some(c) = h.entries.get(index).cloned() else {
            return;
        };
        let prev = h.entries.get(index + 1).cloned();
        h.selected = Some(index);
        h.version = None;
        h.diff = None;
        self.git.spawn(move || {
            let result = file_at(&root, &c.sha, &c.path).and_then(|new| {
                let old = match &prev {
                    Some(p) => file_at(&root, &p.sha, &p.path)?,
                    None => String::new(),
                };
                Ok((new, old))
            });
            Msg::Version {
                sha: c.sha.clone(),
                result,
            }
        });
    }

    /// "Git : ouvrir l'historique de la note".
    pub fn git_open_history(&mut self) {
        self.settings.right_open = true;
        self.settings.right_pane = crate::settings::RightPane::History;
        if self.git.repo.is_none() && self.git.checked {
            self.info(
                "Le vault n'est pas un dépôt Git : initialise-le dans Réglages > Sauvegarde (Git).",
            );
        }
    }

    /// Confirmed: replace the buffer of `path` by an old version, whatever
    /// note is active now.
    pub fn git_restore_text(&mut self, path: &NotePath, text: String) {
        self.ensure_buffer(path);
        let Some(b) = self.buffers.get_mut(path) else {
            return;
        };
        b.text = text;
        self.buffer_edited(path);
        self.info("Version restaurée : elle sera sauvegardée au prochain commit.");
    }
}

// ---------------------------------------------------------------------------
// Tests (real repositories in temporary folders)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Skip a test when git cannot be run.
    macro_rules! require_git {
        () => {
            if git_version().is_err() {
                eprintln!("git absent: test ignoré");
                return;
            }
        };
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, text).unwrap();
    }

    /// Vault with a repository and `core.autocrlf` off (stable contents).
    fn repo(dir: &Path) -> RepoInfo {
        let info = init_repo(dir).unwrap();
        run_ok(dir, &[], &["config", "core.autocrlf", "false"]).unwrap();
        info
    }

    fn head_files(dir: &Path) -> Vec<String> {
        let out = run_ok(dir, &[], &["ls-tree", "-r", "--name-only", "HEAD"]).unwrap();
        out.text().lines().map(str::to_string).collect()
    }

    #[test]
    fn message_template() {
        assert_eq!(
            render_message(DEFAULT_TEMPLATE, "2026-01-02 03:04", 5),
            "vault backup: 2026-01-02 03:04"
        );
        assert_eq!(
            render_message("{{numFiles}} files at {{date}}", "D", 3),
            "3 files at D"
        );
        assert_eq!(render_message("  ", "D", 1), "vault backup: D");
    }

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(1000, 990), "à l'instant");
        assert_eq!(relative_time(1000 + 180, 1000), "il y a 3 min");
        assert_eq!(relative_time(10_000, 10_000 - 7300), "il y a 2 h");
        assert_eq!(
            relative_time(1_000_000, 1_000_000 - 4 * 86_400),
            "il y a 4 j"
        );
    }

    #[test]
    fn gitignore_rules() {
        assert_eq!(add_rule("", "a"), "a\n");
        assert_eq!(add_rule("a", "b"), "a\nb\n");
        assert_eq!(add_rule("a\nb\n", "b"), "a\nb\n");
        assert_eq!(
            remove_rules("a\nb\nc\n", &["b".into()]),
            Some("a\nc\n".into())
        );
        assert_eq!(remove_rules("a\n", &["x".into()]), None);
        assert_eq!(ignore_rule("Dossier/a.md", false), "Dossier/a.md");
        assert_eq!(ignore_rule("a.md", false), "/a.md");
        assert_eq!(ignore_rule("Dossier", true), "/Dossier/");
        assert_eq!(ignore_rule("A/B", true), "A/B/");
        assert_eq!(ignore_rule("#x*.md", false), "/\\#x\\*.md");
        assert!(rule_variants("a.md", false).contains(&"/a.md".to_string()));
    }

    #[test]
    fn presets_toggle_rules() {
        let mut t = String::from("# mine\nfoo\n");
        for p in Preset::ALL {
            assert!(!preset_active(&t, p));
            t = set_preset(&t, p, true);
            assert!(preset_active(&t, p));
        }
        assert!(t.contains("*.pdf\n") && t.contains(".trash/\n") && t.starts_with("# mine\nfoo\n"));
        t = set_preset(&t, Preset::HeavyAttachments, false);
        assert!(!preset_active(&t, Preset::HeavyAttachments));
        assert!(!t.contains("*.mp4"));
        assert!(preset_active(&t, Preset::Trash));
        // Switching off what is not there changes nothing.
        assert_eq!(set_preset("x\n", Preset::Trash, false), "x\n");
        // The default `.gitignore` has the workspace and trash presets.
        assert!(preset_active(DEFAULT_GITIGNORE, Preset::Workspace));
        assert!(preset_active(DEFAULT_GITIGNORE, Preset::Trash));
    }

    #[test]
    fn diff_marks_added_and_removed_lines() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\nk\n";
        let d = diff_lines(old, new, 1);
        let kinds: Vec<(DiffKind, &str)> = d.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert!(kinds.contains(&(DiffKind::Removed, "e")));
        assert!(kinds.contains(&(DiffKind::Added, "E")));
        assert!(kinds.contains(&(DiffKind::Added, "k")));
        assert!(kinds.iter().any(|(k, _)| *k == DiffKind::Gap));
        assert!(!kinds.contains(&(DiffKind::Context, "a")));
        assert!(
            diff_lines("same\n", "same\r\n", 3).is_empty(),
            "CRLF ignored"
        );
        let all_new = diff_lines("", "x\ny\n", 3);
        assert_eq!(all_new.len(), 2);
        assert!(all_new.iter().all(|l| l.kind == DiffKind::Added));
    }

    #[test]
    fn log_parsing_follows_renames() {
        let text = "\u{1e}aaa\u{1f}200\u{1f}rename\n\nR100\told.md\tsub/new.md\n\u{1e}bbb\u{1f}100\u{1f}add\n\nA\tsub/old.md\n";
        let c = parse_log(text, "sub/", "new.md");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].path, "new.md");
        assert_eq!(c[0].subject, "rename");
        assert_eq!(c[0].time, 200);
        assert_eq!(c[1].path, "old.md");
    }

    #[test]
    fn init_creates_repo_gitignore_and_first_commit() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "# A\n");
        write(dir.path(), ".obsidian/workspace.json", "{}");
        write(dir.path(), ".obsidian/app.json", "{}");
        assert_eq!(detect_repo(dir.path()).unwrap(), None);
        let info = init_repo(dir.path()).unwrap();
        assert!(dir.path().join(".git").exists());
        assert_eq!(info.prefix, "");
        assert!(!info.is_nested());
        let ignore = read_gitignore(dir.path());
        assert!(ignore.contains(".obsidian/workspace-oxinote.json"));
        assert!(ignore.contains(".trash/") && ignore.contains(".DS_Store"));
        let files = head_files(dir.path());
        assert!(files.contains(&"a.md".to_string()), "{files:?}");
        assert!(files.contains(&".gitignore".to_string()));
        assert!(files.contains(&".obsidian/app.json".to_string()));
        assert!(!files.contains(&".obsidian/workspace.json".to_string()));
        assert_eq!(status_count(dir.path()).unwrap(), 0);
        assert!(last_commit_time(dir.path()).is_some());
    }

    #[test]
    fn no_change_means_no_commit_then_modification_commits() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "one\n");
        let info = repo(dir.path());
        let count = || {
            run_ok(dir.path(), &[], &["rev-list", "--count", "HEAD"])
                .unwrap()
                .text()
                .trim()
                .parse::<usize>()
                .unwrap()
        };
        assert_eq!(count(), 1);
        assert_eq!(
            backup(dir.path(), &info, DEFAULT_TEMPLATE).unwrap(),
            BackupOutcome::Nothing
        );
        assert_eq!(count(), 1, "no empty commit");

        write(dir.path(), "a.md", "two\n");
        write(dir.path(), "b.md", "new\n");
        assert_eq!(status_count(dir.path()).unwrap(), 2);
        match backup(dir.path(), &info, "{{numFiles}} files").unwrap() {
            BackupOutcome::Committed { files, message, .. } => {
                assert_eq!(files, 2);
                assert_eq!(message, "2 files");
            }
            other => panic!("expected a commit, got {other:?}"),
        }
        assert_eq!(count(), 2);
        assert_eq!(status_count(dir.path()).unwrap(), 0);
        let subject = run_ok(dir.path(), &[], &["log", "-1", "--format=%s"]).unwrap();
        assert_eq!(subject.text().trim(), "2 files");
        // The commit has our fallback identity when the user has none, or
        // the user's own; either way it exists.
        let who = run_ok(dir.path(), &[], &["log", "-1", "--format=%an"]).unwrap();
        assert!(!who.text().trim().is_empty());
    }

    #[test]
    fn identity_fallback_only_when_missing() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "x\n");
        let info = repo(dir.path());
        // A local identity: nothing to add.
        run_ok(dir.path(), &[], &["config", "user.name", "Me"]).unwrap();
        run_ok(dir.path(), &[], &["config", "user.email", "me@example.org"]).unwrap();
        assert!(identity_cfg(dir.path()).is_empty());
        write(dir.path(), "b.md", "y\n");
        backup(dir.path(), &info, DEFAULT_TEMPLATE).unwrap();
        let who = run_ok(dir.path(), &[], &["log", "-1", "--format=%an"]).unwrap();
        assert_eq!(who.text().trim(), "Me");
        // The user's config is never modified by the fallback.
        assert_ne!(
            identity_cfg(dir.path()),
            vec!["user.name=Oxinote", "user.email=oxinote@localhost"]
        );
    }

    #[test]
    fn check_ignore_lists_ignored_paths() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "x\n");
        write(dir.path(), "Notes/é.md", "x\n");
        write(dir.path(), "big.pdf", "x");
        write(dir.path(), "Private/s.md", "x\n");
        write_gitignore(dir.path(), "*.pdf\nPrivate/\n").unwrap();
        repo(dir.path());
        // The default rules are appended to the user's existing file.
        assert!(read_gitignore(dir.path()).starts_with("*.pdf\nPrivate/\n"));
        let paths: Vec<String> = ["a.md", "Notes/é.md", "big.pdf", "Private/s.md", "Private/"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let set = check_ignore(dir.path(), &paths).unwrap();
        assert!(set.contains("big.pdf"));
        assert!(set.contains("Private/s.md"));
        assert!(set.contains("Private/"));
        assert!(!set.contains("a.md") && !set.contains("Notes/é.md"));
        assert!(
            check_ignore(dir.path(), &["a.md".to_string()])
                .unwrap()
                .is_empty()
        );
        assert!(
            why_ignored(dir.path(), "big.pdf")
                .unwrap()
                .contains("*.pdf")
        );
    }

    #[test]
    fn ignore_then_untrack_a_tracked_file() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "secret.md", "x\n");
        write(dir.path(), "keep.md", "x\n");
        let info = repo(dir.path());
        assert!(is_tracked(dir.path(), "secret.md"));
        let text = add_rule(
            &read_gitignore(dir.path()),
            &ignore_rule("secret.md", false),
        );
        write_gitignore(dir.path(), &text).unwrap();
        untrack(dir.path(), "secret.md").unwrap();
        assert!(!is_tracked(dir.path(), "secret.md"));
        assert!(dir.path().join("secret.md").exists(), "file kept on disk");
        backup(dir.path(), &info, DEFAULT_TEMPLATE).unwrap();
        assert!(!head_files(dir.path()).contains(&"secret.md".to_string()));
        assert!(
            check_ignore(dir.path(), &["secret.md".into()])
                .unwrap()
                .contains("secret.md")
        );
        // Un-ignoring removes the exact line.
        let rules = rule_variants("secret.md", false);
        let text = remove_rules(&read_gitignore(dir.path()), &rules).unwrap();
        write_gitignore(dir.path(), &text).unwrap();
        assert!(
            check_ignore(dir.path(), &["secret.md".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn file_history_and_old_versions() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "v1\n");
        let info = repo(dir.path());
        write(dir.path(), "a.md", "v2\n");
        backup(dir.path(), &info, "second").unwrap();
        write(dir.path(), "other.md", "o\n");
        backup(dir.path(), &info, "unrelated").unwrap();
        write(dir.path(), "a.md", "v3\n");
        backup(dir.path(), &info, "third").unwrap();

        let log = log_file(dir.path(), &info, "a.md", 50).unwrap();
        let subjects: Vec<&str> = log.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects.len(), 3, "{subjects:?}");
        assert_eq!(subjects[0], "third");
        assert_eq!(subjects[1], "second");
        assert!(log.iter().all(|c| c.path == "a.md" && c.time > 0));
        assert_eq!(file_at(dir.path(), &log[0].sha, "a.md").unwrap(), "v3\n");
        assert_eq!(file_at(dir.path(), &log[1].sha, "a.md").unwrap(), "v2\n");
        assert_eq!(file_at(dir.path(), &log[2].sha, "a.md").unwrap(), "v1\n");
        assert!(file_at(dir.path(), &log[2].sha, "nope.md").is_err());
        assert!(
            log_file(dir.path(), &info, "never.md", 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn history_follows_a_rename() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "old.md",
            "some longer content\nline 2\nline 3\n",
        );
        let info = repo(dir.path());
        std::fs::rename(dir.path().join("old.md"), dir.path().join("new.md")).unwrap();
        backup(dir.path(), &info, "rename").unwrap();
        let log = log_file(dir.path(), &info, "new.md", 10).unwrap();
        assert_eq!(log.len(), 2, "{log:?}");
        assert_eq!(log[0].path, "new.md");
        assert_eq!(log[1].path, "old.md");
        assert!(
            file_at(dir.path(), &log[1].sha, &log[1].path)
                .unwrap()
                .starts_with("some longer")
        );
    }

    #[test]
    fn vault_inside_a_parent_repository() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        run_ok(top, &["init.defaultBranch=main".into()], &["init", "-q"]).unwrap();
        run_ok(top, &[], &["config", "core.autocrlf", "false"]).unwrap();
        write(top, "outside.txt", "1\n");
        write(top, "vault/a.md", "1\n");
        let vault = top.join("vault");
        let info = detect_repo(&vault).unwrap().unwrap();
        assert_eq!(info.prefix, "vault/");
        assert!(info.is_nested());
        // First commit (no HEAD yet) limited to the vault.
        backup(&vault, &info, DEFAULT_TEMPLATE).unwrap();
        let files = head_files(top);
        assert_eq!(files, vec!["vault/a.md".to_string()], "{files:?}");

        // With a HEAD: only the vault's changes, even if the user staged
        // something else.
        write(&vault, "a.md", "2\n");
        write(top, "outside.txt", "2\n");
        run_ok(top, &[], &["add", "outside.txt"]).unwrap();
        backup(&vault, &info, DEFAULT_TEMPLATE).unwrap();
        let files = head_files(top);
        assert_eq!(files, vec!["vault/a.md".to_string()], "{files:?}");
        let st = run_ok(top, &[], &["status", "--porcelain"]).unwrap().text();
        assert!(st.contains("outside.txt"), "outside change untouched: {st}");

        // Paths of the history are relative to the vault.
        let log = log_file(&vault, &info, "a.md", 10).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.iter().all(|c| c.path == "a.md"));
        assert_eq!(file_at(&vault, &log[1].sha, "a.md").unwrap(), "1\n");
        assert_eq!(status_count(&vault).unwrap(), 0, "outside not counted");
    }

    #[test]
    fn nested_vault_in_unborn_parent_ignores_outside_index() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        run_ok(top, &["init.defaultBranch=main".into()], &["init", "-q"]).unwrap();
        run_ok(top, &[], &["config", "core.autocrlf", "false"]).unwrap();
        write(top, "outside.txt", "1\n");
        write(top, "vault/a.md", "1\n");
        // Already staged by the user, outside the vault, before any commit.
        run_ok(top, &[], &["add", "outside.txt"]).unwrap();
        let vault = top.join("vault");
        let info = detect_repo(&vault).unwrap().unwrap();
        assert!(info.is_nested());
        backup(&vault, &info, DEFAULT_TEMPLATE).unwrap();
        let files = head_files(top);
        assert_eq!(files, vec!["vault/a.md".to_string()], "{files:?}");
        let st = run_ok(top, &[], &["status", "--porcelain"]).unwrap().text();
        assert!(st.contains("A  outside.txt"), "still staged: {st}");
    }

    #[test]
    fn locked_repository_is_retried_not_failed() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "1\n");
        let info = repo(dir.path());
        write(dir.path(), "a.md", "2\n");
        std::fs::write(dir.path().join(".git/index.lock"), "").unwrap();
        assert_eq!(
            backup(dir.path(), &info, DEFAULT_TEMPLATE),
            Err(GitError::Locked)
        );
        std::fs::remove_file(dir.path().join(".git/index.lock")).unwrap();
        assert!(matches!(
            backup(dir.path(), &info, DEFAULT_TEMPLATE),
            Ok(BackupOutcome::Committed { .. })
        ));
    }

    #[test]
    fn crlf_files_are_diffed_without_noise() {
        require_git!();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.md", "one\r\ntwo\r\n");
        let info = repo(dir.path());
        write(dir.path(), "a.md", "one\r\ntwo\r\nthree\r\n");
        backup(dir.path(), &info, DEFAULT_TEMPLATE).unwrap();
        let log = log_file(dir.path(), &info, "a.md", 10).unwrap();
        let old = file_at(dir.path(), &log[1].sha, "a.md").unwrap();
        let new = file_at(dir.path(), &log[0].sha, "a.md").unwrap();
        let d = diff_lines(&old, &new, 3);
        let added: Vec<&str> = d
            .iter()
            .filter(|l| l.kind == DiffKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, vec!["three"]);
        assert!(d.iter().all(|l| l.kind != DiffKind::Removed));
    }
}
