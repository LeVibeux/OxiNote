//! Automatic updates from the GitHub releases of Oxinote.
//!
//! At most once a day (and on demand from Settings > About), a background
//! thread asks GitHub for the latest release. When it is newer than the
//! running version, the archive of this platform is downloaded, checked
//! against the release's `SHA256SUMS.txt` and swapped in place of the
//! installed files; the new version runs after a restart.
//!
//! Installs that the app cannot rewrite (the `.deb` package under `/usr`, a
//! read-only folder, a translocated macOS app) are only notified.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::AppState;

const LATEST_URL: &str = "https://api.github.com/repos/LeVibeux/Oxinote/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/LeVibeux/Oxinote/releases";
const SUMS_FILE: &str = "SHA256SUMS.txt";
/// Seconds between two automatic checks.
const CHECK_EVERY: u64 = 24 * 3600;
/// Largest archive accepted (the real ones are ~20 MB).
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;
/// Prefix of the staging folder, next to the installed files.
const STAGING: &str = ".oxinote-update";

/// What the app does when a new release is out (Settings > About).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateMode {
    Off,
    /// Only tell the user.
    Notify,
    /// Download and install it; it runs after a restart.
    #[default]
    Install,
}

impl UpdateMode {
    pub const ALL: [UpdateMode; 3] = [UpdateMode::Off, UpdateMode::Notify, UpdateMode::Install];

    pub fn label(self) -> &'static str {
        match self {
            UpdateMode::Off => "Désactivées",
            UpdateMode::Notify => "Me prévenir seulement",
            UpdateMode::Install => "Installer automatiquement",
        }
    }
}

/// A published release newer than the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Tag, e.g. `v0.2.0`.
    pub version: String,
    /// Release page.
    pub page: String,
    /// Archive of this platform (`None`: no build for it).
    pub asset: Option<Asset>,
    /// URL of `SHA256SUMS.txt`.
    pub sums: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// Where the running app lives, i.e. what an update has to replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// Loose binaries in a folder (Linux archive, Windows zip).
    Portable { dir: PathBuf },
    /// `…/Oxinote.app/Contents/MacOS/oxinote-app`.
    MacBundle { app: PathBuf },
    /// Installed by a package manager: notify only.
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing(String),
    /// Installed, runs after a restart.
    Installed(String),
    Failed(String),
}

enum Msg {
    Checked {
        manual: bool,
        result: Result<Option<Release>, String>,
    },
    Installed(Result<String, String>),
}

/// Background checks and installs; lives in [`AppState`].
pub struct UpdateService {
    /// Automatic checks run only in the real app (not in tests, screenshots
    /// or the demo vault).
    pub enabled: bool,
    pub status: UpdateStatus,
    started: bool,
    /// Cached [`self_install_possible`] (it writes a probe file).
    can_install: Option<bool>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for UpdateService {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            enabled: false,
            status: UpdateStatus::Idle,
            started: false,
            can_install: None,
            tx,
            rx,
        }
    }
}

impl UpdateService {
    fn spawn(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    /// Whether this copy of the app can replace its own files.
    pub fn can_install(&mut self) -> bool {
        *self.can_install.get_or_insert_with(self_install_possible)
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Checking | UpdateStatus::Installing(_)
        )
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// `v1.2.3-rc.1` -> `(1, 2, 3, Some("rc.1"))`.
fn parse_version(v: &str) -> Option<(u64, u64, u64, Option<&str>)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let mut it = core.split('.').map(str::parse::<u64>);
    let (a, b, c) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c, pre))
}

/// Whether release `latest` is newer than version `current`. A pre-release
/// is older than the release of the same number.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(l), Some(c)) = (parse_version(latest), parse_version(current)) else {
        return false;
    };
    match (l.0, l.1, l.2).cmp(&(c.0, c.1, c.2)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (l.3, c.3) {
            (None, Some(_)) => true,
            (Some(a), Some(b)) => a > b,
            _ => false,
        },
    }
}

/// Rust target of this build, as used in the archive names.
pub fn target() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else {
        None
    }
}

/// Archive name of `tag` for `target` (see `.github/workflows/release.yml`).
pub fn asset_name(tag: &str, target: &str) -> String {
    let ext = if target.contains("linux") {
        "tar.gz"
    } else {
        "zip"
    };
    format!("oxinote-{tag}-{target}.{ext}")
}

/// The release from GitHub's `releases/latest` JSON, if newer than `current`.
pub fn parse_release(
    json: &str,
    current: &str,
    target: Option<&str>,
) -> Result<Option<Release>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("réponse GitHub illisible : {e}"))?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or("réponse GitHub sans version")?
        .to_string();
    if !is_newer(&tag, current) {
        return Ok(None);
    }
    let assets: Vec<(&str, &str)> = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| Some((x["name"].as_str()?, x["browser_download_url"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    let find = |name: &str| {
        assets
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, u)| (*u).to_string())
    };
    let asset = target.and_then(|t| {
        let name = asset_name(&tag, t);
        find(&name).map(|url| Asset { name, url })
    });
    Ok(Some(Release {
        page: v["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        sums: find(SUMS_FILE),
        asset,
        version: tag,
    }))
}

/// Expected SHA-256 of `name` in a `sha256sum` listing.
pub fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What an update of the executable at `exe` replaces.
pub fn install_kind(exe: &Path) -> InstallKind {
    let dir = exe.parent();
    let macos = dir.filter(|d| d.file_name().is_some_and(|n| n == "MacOS"));
    let contents = macos
        .and_then(Path::parent)
        .filter(|d| d.file_name().is_some_and(|n| n == "Contents"));
    if let Some(app) = contents
        .and_then(Path::parent)
        .filter(|a| a.extension().is_some_and(|e| e == "app"))
    {
        return InstallKind::MacBundle {
            app: app.to_path_buf(),
        };
    }
    // A `cargo build` (…/target/{debug,release}/) must not be replaced.
    let cargo_build = exe.ancestors().any(|a| {
        a.file_name().is_some_and(|n| n == "target")
            && a.parent().is_some_and(|p| p.join("Cargo.toml").is_file())
    });
    match dir {
        Some(d) if !cargo_build && !d.starts_with("/usr") && !d.starts_with("/opt") => {
            InstallKind::Portable {
                dir: d.to_path_buf(),
            }
        }
        _ => InstallKind::Managed,
    }
}

/// Folder that receives the staged files (same file system as the install,
/// so that the swap is a rename).
fn install_parent(kind: &InstallKind) -> Option<&Path> {
    match kind {
        InstallKind::Portable { dir } => Some(dir),
        InstallKind::MacBundle { app } => app.parent(),
        InstallKind::Managed => None,
    }
}

/// Whether the app may rewrite its own files.
pub fn can_self_install(kind: &InstallKind) -> bool {
    let Some(parent) = install_parent(kind) else {
        return false;
    };
    let probe = parent.join(format!("{STAGING}-probe"));
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

fn exe_name(base: &str) -> String {
    format!("{base}{}", std::env::consts::EXE_SUFFIX)
}

/// Unpack `archive` (`.tar.gz` or `.zip`, by `name`) into `dest`.
fn unpack(name: &str, archive: &[u8], dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    if name.ends_with(".tar.gz") {
        let gz = flate2::read::GzDecoder::new(Cursor::new(archive));
        let mut tar = tar::Archive::new(gz);
        tar.set_preserve_permissions(true);
        return tar
            .unpack(dest)
            .map_err(|e| format!("archive illisible : {e}"));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|e| format!("archive illisible : {e}"))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        fs::write(&out, data).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o7777));
        }
    }
    Ok(())
}

/// The single top-level folder of an unpacked archive.
fn top_dir(dest: &Path) -> Result<PathBuf, String> {
    let dirs: Vec<PathBuf> = fs::read_dir(dest)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [d] => Ok(d.clone()),
        _ => Err("contenu d'archive inattendu".to_string()),
    }
}

/// Put `new` at `dst`. The running executable can't be overwritten but can
/// be renamed (Windows) or replaced by a rename (Unix, it keeps its inode).
fn swap_in(new: &Path, dst: &Path) -> Result<(), String> {
    let old = old_path(dst);
    if dst.exists() {
        if old.is_dir() {
            let _ = fs::remove_dir_all(&old);
        } else {
            let _ = fs::remove_file(&old);
        }
        fs::rename(dst, &old).map_err(|e| format!("{} : {e}", dst.display()))?;
    }
    if let Err(e) = fs::rename(new, dst) {
        // Put the old version back.
        let _ = fs::rename(&old, dst);
        return Err(format!("{} : {e}", dst.display()));
    }
    Ok(())
}

fn old_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Install the unpacked archive `archive` (`name`) over `kind`.
pub fn install_archive(name: &str, archive: &[u8], kind: &InstallKind) -> Result<(), String> {
    let parent = install_parent(kind).ok_or("installation gérée par le système")?;
    let staging = parent.join(format!("{STAGING}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let result = unpack(name, archive, &staging)
        .and_then(|()| top_dir(&staging))
        .and_then(|top| {
            let cli = exe_name("oxinote");
            match kind {
                InstallKind::Portable { dir } => {
                    let app = exe_name("oxinote-app");
                    let new_app = top.join(&app);
                    if !new_app.is_file() {
                        return Err(format!("{app} absent de l'archive"));
                    }
                    swap_in(&new_app, &dir.join(&app))?;
                    if top.join(&cli).is_file() && dir.join(&cli).exists() {
                        swap_in(&top.join(&cli), &dir.join(&cli))?;
                    }
                }
                InstallKind::MacBundle { app } => {
                    let new_app = top.join("Oxinote.app");
                    if !new_app.is_dir() {
                        return Err("Oxinote.app absent de l'archive".to_string());
                    }
                    swap_in(&new_app, app)?;
                    let _ = fs::remove_dir_all(old_path(app));
                    if top.join(&cli).is_file() && parent.join(&cli).exists() {
                        swap_in(&top.join(&cli), &parent.join(&cli))?;
                    }
                }
                InstallKind::Managed => {}
            }
            Ok(())
        });
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Remove what a previous update left behind (`*.old` binaries on Windows,
/// an interrupted staging folder).
pub fn cleanup(kind: &InstallKind) {
    let Some(parent) = install_parent(kind) else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let stale_bin = name.starts_with("oxinote") && name.ends_with(".old");
        if name.starts_with(STAGING) || stale_bin {
            let p = e.path();
            let _ = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

fn get(url: &str) -> Result<Vec<u8>, String> {
    // The system's certificates, so that proxies with their own authority
    // (universities, companies) work.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).build().into();
    let mut resp = agent
        .get(url)
        .header("User-Agent", concat!("Oxinote/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json, */*")
        .call()
        .map_err(|e| format!("connexion impossible : {e}"))?;
    resp.body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| format!("téléchargement interrompu : {e}"))
}

fn check_latest() -> Result<Option<Release>, String> {
    let body = get(LATEST_URL)?;
    parse_release(
        &String::from_utf8_lossy(&body),
        env!("CARGO_PKG_VERSION"),
        target(),
    )
}

fn download_and_install(release: &Release) -> Result<String, String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("pas d'archive pour ce système")?;
    let sums_url = release.sums.as_ref().ok_or("SHA256SUMS.txt manquant")?;
    let sums = String::from_utf8_lossy(&get(sums_url)?).into_owned();
    let want = expected_sum(&sums, &asset.name).ok_or("somme de contrôle manquante")?;
    let data = get(&asset.url)?;
    if sha256_hex(&data) != want {
        return Err("somme de contrôle incorrecte : archive rejetée".to_string());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    install_archive(&asset.name, &data, &install_kind(&exe))?;
    Ok(release.version.clone())
}

/// Whether this copy of the app can update itself.
fn self_install_possible() -> bool {
    std::env::current_exe().is_ok_and(|exe| can_self_install(&install_kind(&exe)))
}

/// Start the new version and let the caller close this one.
pub fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = match install_kind(&exe) {
        InstallKind::MacBundle { app } => {
            let mut c = std::process::Command::new("open");
            c.arg("-n").arg(app);
            c
        }
        _ => std::process::Command::new(exe),
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub const fn releases_page() -> &'static str {
    RELEASES_PAGE
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// AppState glue
// ---------------------------------------------------------------------------

impl AppState {
    /// Look for a new release now (Settings > About), whatever the mode.
    pub fn check_for_updates(&mut self) {
        if self.updates.busy() {
            return;
        }
        self.updates.status = UpdateStatus::Checking;
        self.settings.update_checked_at = now_secs();
        self.updates.spawn(|| Msg::Checked {
            manual: true,
            result: check_latest(),
        });
    }

    /// Download and install `release` in the background.
    pub fn install_update(&mut self, release: Release) {
        if self.updates.busy() {
            return;
        }
        self.updates.status = UpdateStatus::Installing(release.version.clone());
        self.updates
            .spawn(move || Msg::Installed(download_and_install(&release)));
    }

    /// Start the daily check and handle finished jobs. Cheap: called on
    /// every frame.
    pub fn update_tick(&mut self) {
        if self.updates.enabled && !self.updates.started {
            self.updates.started = true;
            if let Ok(exe) = std::env::current_exe() {
                cleanup(&install_kind(&exe));
            }
            let due = now_secs().saturating_sub(self.settings.update_checked_at) >= CHECK_EVERY;
            if self.settings.update_mode != UpdateMode::Off && due {
                self.settings.update_checked_at = now_secs();
                self.updates.status = UpdateStatus::Checking;
                self.updates.spawn(|| Msg::Checked {
                    manual: false,
                    result: check_latest(),
                });
            }
        }
        while let Ok(msg) = self.updates.rx.try_recv() {
            match msg {
                Msg::Checked { manual, result } => self.on_checked(manual, result),
                Msg::Installed(Ok(v)) => {
                    self.info(format!(
                        "Oxinote {v} est installé : redémarrez pour l'utiliser (Réglages › About)."
                    ));
                    self.updates.status = UpdateStatus::Installed(v);
                }
                Msg::Installed(Err(e)) => {
                    self.error(format!("Mise à jour impossible : {e}"));
                    self.updates.status = UpdateStatus::Failed(e);
                }
            }
        }
    }

    fn on_checked(&mut self, manual: bool, result: Result<Option<Release>, String>) {
        match result {
            Ok(None) => self.updates.status = UpdateStatus::UpToDate,
            Ok(Some(r)) => {
                let auto = !manual && self.settings.update_mode == UpdateMode::Install;
                if auto && r.asset.is_some() && self.updates.can_install() {
                    self.updates.status = UpdateStatus::Idle;
                    self.install_update(r);
                } else {
                    self.info(format!(
                        "Oxinote {} est disponible (Réglages › About).",
                        r.version
                    ));
                    self.updates.status = UpdateStatus::Available(r);
                }
            }
            // A failed background check stays quiet; a manual one says why.
            Err(e) => {
                if manual {
                    self.error(format!("Vérification impossible : {e}"));
                }
                self.updates.status = UpdateStatus::Failed(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_like_semver() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.0", "0.1.0-rc.1"));
        assert!(is_newer("v0.1.0-rc.2", "0.1.0-rc.1"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0-rc.1", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }

    #[test]
    fn asset_names_match_the_release_workflow() {
        assert_eq!(
            asset_name("v0.2.0", "x86_64-unknown-linux-gnu"),
            "oxinote-v0.2.0-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            asset_name("v0.2.0", "aarch64-apple-darwin"),
            "oxinote-v0.2.0-aarch64-apple-darwin.zip"
        );
        assert_eq!(
            asset_name("v0.2.0", "x86_64-pc-windows-msvc"),
            "oxinote-v0.2.0-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn parses_the_github_release() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/LeVibeux/Oxinote/releases/tag/v0.2.0",
            "assets": [
                {"name": "oxinote-v0.2.0-x86_64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://x/linux"},
                {"name": "oxinote-v0.2.0-x86_64-pc-windows-msvc.zip", "browser_download_url": "https://x/win"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/sums"}
            ]
        }"#;
        let r = parse_release(json, "0.1.0", Some("x86_64-pc-windows-msvc"))
            .expect("parse")
            .expect("newer");
        assert_eq!(r.version, "v0.2.0");
        assert_eq!(r.asset.expect("asset").url, "https://x/win");
        assert_eq!(r.sums.as_deref(), Some("https://x/sums"));
        // No build for this target: still reported, without an asset.
        let r = parse_release(json, "0.1.0", Some("aarch64-apple-darwin"))
            .expect("parse")
            .expect("newer");
        assert!(r.asset.is_none());
        assert_eq!(parse_release(json, "0.2.0", None).expect("parse"), None);
        assert!(parse_release("{}", "0.1.0", None).is_err());
    }

    #[test]
    fn reads_sha256sums() {
        let sums = "ABC123  oxinote-v0.2.0-x.zip\ndef456 *oxinote_0.2.0_amd64.deb\n";
        assert_eq!(
            expected_sum(sums, "oxinote-v0.2.0-x.zip").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            expected_sum(sums, "oxinote_0.2.0_amd64.deb").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_sum(sums, "other"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn detects_the_install_kind() {
        assert_eq!(
            install_kind(Path::new(
                "/Applications/Oxinote.app/Contents/MacOS/oxinote-app"
            )),
            InstallKind::MacBundle {
                app: PathBuf::from("/Applications/Oxinote.app")
            }
        );
        assert_eq!(
            install_kind(Path::new("/usr/bin/oxinote-app")),
            InstallKind::Managed
        );
        let repo = tempfile::tempdir().expect("tmp");
        fs::write(repo.path().join("Cargo.toml"), b"").expect("write");
        assert_eq!(
            install_kind(&repo.path().join("target/release/oxinote-app")),
            InstallKind::Managed
        );
        assert_eq!(
            install_kind(Path::new("/home/me/oxinote/oxinote-app")),
            InstallKind::Portable {
                dir: PathBuf::from("/home/me/oxinote")
            }
        );
    }

    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, path, *data).expect("append");
        }
        b.into_inner().expect("tar").finish().expect("gz")
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for (path, data) in files {
            z.start_file(*path, opts).expect("start");
            std::io::Write::write_all(&mut z, data).expect("write");
        }
        z.finish().expect("zip").into_inner()
    }

    #[test]
    fn installs_a_portable_archive_over_the_old_binaries() {
        let dir = tempfile::tempdir().expect("tmp");
        let app = exe_name("oxinote-app");
        let cli = exe_name("oxinote");
        fs::write(dir.path().join(&app), b"old app").expect("write");
        fs::write(dir.path().join(&cli), b"old cli").expect("write");
        fs::write(dir.path().join("notes.txt"), b"mine").expect("write");
        let top = "oxinote-v0.2.0-x86_64-unknown-linux-gnu";
        let archive = tar_gz(&[
            (&format!("{top}/{app}"), b"new app"),
            (&format!("{top}/{cli}"), b"new cli"),
            (&format!("{top}/README.md"), b"readme"),
        ]);
        let kind = InstallKind::Portable {
            dir: dir.path().to_path_buf(),
        };
        assert!(can_self_install(&kind));
        install_archive("x.tar.gz", &archive, &kind).expect("install");
        cleanup(&kind);
        assert_eq!(fs::read(dir.path().join(&app)).expect("read"), b"new app");
        assert_eq!(fs::read(dir.path().join(&cli)).expect("read"), b"new cli");
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .expect("ls")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want = vec![app, cli, "notes.txt".to_string()];
        want.sort();
        assert_eq!(left, want);
    }

    #[test]
    fn installs_a_macos_bundle() {
        let dir = tempfile::tempdir().expect("tmp");
        let app = dir.path().join("Oxinote.app");
        fs::create_dir_all(app.join("Contents/MacOS")).expect("mkdir");
        fs::write(app.join("Contents/MacOS/oxinote-app"), b"old").expect("write");
        fs::write(app.join("Contents/Info.plist"), b"old plist").expect("write");
        let top = "oxinote-v0.2.0-aarch64-apple-darwin";
        let archive = zip_of(&[
            (
                &format!("{top}/Oxinote.app/Contents/MacOS/oxinote-app"),
                b"new",
            ),
            (
                &format!("{top}/Oxinote.app/Contents/Info.plist"),
                b"new plist",
            ),
            (&format!("{top}/oxinote"), b"cli"),
        ]);
        let kind = InstallKind::MacBundle { app: app.clone() };
        install_archive("x.zip", &archive, &kind).expect("install");
        assert_eq!(
            fs::read(app.join("Contents/MacOS/oxinote-app")).expect("read"),
            b"new"
        );
        assert!(!old_path(&app).exists());
        // No CLI was installed next to the app: none is added.
        assert!(!dir.path().join("oxinote").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(app.join("Contents/MacOS/oxinote-app"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn a_bad_archive_leaves_the_install_untouched() {
        let dir = tempfile::tempdir().expect("tmp");
        let app = exe_name("oxinote-app");
        fs::write(dir.path().join(&app), b"old app").expect("write");
        let kind = InstallKind::Portable {
            dir: dir.path().to_path_buf(),
        };
        assert!(install_archive("x.tar.gz", b"not an archive", &kind).is_err());
        let archive = tar_gz(&[("top/README.md", b"no binary")]);
        assert!(install_archive("x.tar.gz", &archive, &kind).is_err());
        assert_eq!(fs::read(dir.path().join(&app)).expect("read"), b"old app");
        assert_eq!(fs::read_dir(dir.path()).expect("ls").count(), 1);
        assert!(install_archive("x.zip", b"", &InstallKind::Managed).is_err());
    }
}
