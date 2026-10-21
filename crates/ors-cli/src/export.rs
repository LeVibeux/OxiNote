//! `obsirust export`: static HTML site generation.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use ors_core::{NotePath, ParsedNote, VaultRead};
use ors_index::MetadataCache;
use ors_markdown::RenderOptions;
use ors_vault::Vault;

use crate::context::Context;
use crate::html::{self, EmbedSource, Page};

/// Above this number of files, pages only link to the index instead of
/// embedding the whole file tree (keeps the export linear in size).
const SIDEBAR_MAX_FILES: usize = 1_000;

/// What was exported.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExportSummary {
    pub notes: usize,
    pub attachments: usize,
}

/// Check (and create) the output folder. Refuses a non-empty folder unless
/// `force`, and the vault folder itself.
pub fn prepare_out_dir(vault_dir: &Path, out_dir: &Path, force: bool) -> Result<()> {
    if inside_vault(vault_dir, out_dir) {
        bail!("refusing to export into the vault folder or one of its subfolders");
    }
    if out_dir.exists() {
        if !out_dir.is_dir() {
            bail!(
                "output path exists and is not a folder: {}",
                out_dir.display()
            );
        }
        let non_empty = fs::read_dir(out_dir)
            .with_context(|| format!("cannot read {}", out_dir.display()))?
            .next()
            .is_some();
        if non_empty && !force {
            bail!(
                "output folder {} is not empty (use --force to overwrite)",
                out_dir.display()
            );
        }
    }
    fs::create_dir_all(out_dir).with_context(|| format!("cannot create {}", out_dir.display()))
}

/// `true` if `out_dir` is `vault_dir` or lies inside it. `out_dir` may not
/// exist yet: its nearest existing ancestor is canonicalized.
fn inside_vault(vault_dir: &Path, out_dir: &Path) -> bool {
    let Ok(vault) = fs::canonicalize(vault_dir) else {
        return false;
    };
    let abs = if out_dir.is_absolute() {
        out_dir.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(d) => d.join(out_dir),
            Err(_) => return false,
        }
    };
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(p), Some(n)) => {
                rest.push(n.to_os_string());
                existing = p;
            }
            _ => return false,
        }
    }
    let Ok(mut out) = fs::canonicalize(existing) else {
        return false;
    };
    for n in rest.into_iter().rev() {
        out.push(n);
    }
    out.starts_with(&vault)
}

fn write_file(out_dir: &Path, rel: &str, contents: &[u8]) -> Result<()> {
    let dest = out_dir.join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&dest, contents).with_context(|| format!("cannot write {}", dest.display()))
}

/// Name of the index page, avoiding a clash with a note exported as `index.html`.
pub fn index_page_name(files: &[NotePath]) -> &'static str {
    if files.iter().any(|f| html::output_path(f) == "index.html") {
        "vault-index.html"
    } else {
        "index.html"
    }
}

/// Embed source backed by the real vault and markdown renderer.
struct VaultEmbeds<'a> {
    vault: &'a Vault,
    cache: &'a MetadataCache,
    prefix: String,
}

fn render_options(note: &NotePath, prefix: &str, show_frontmatter: bool) -> RenderOptions {
    let mut opts = RenderOptions::new(note.clone());
    opts.link_prefix = prefix.to_string();
    opts.note_extension = Some("html".to_string());
    opts.show_frontmatter = show_frontmatter;
    opts
}

impl EmbedSource for VaultEmbeds<'_> {
    fn resolve(&self, target: &str, from: &NotePath) -> Option<NotePath> {
        use ors_core::LinkResolver;
        self.cache.resolve(target, from)
    }
    fn source(&self, path: &NotePath) -> Option<String> {
        self.vault.read_to_string(path).ok()
    }
    fn parse(&self, source: &str) -> ParsedNote {
        ors_markdown::parse(source)
    }
    fn render(&self, markdown: &str, path: &NotePath) -> String {
        ors_markdown::render_html(
            markdown,
            &render_options(path, &self.prefix, false),
            self.cache,
        )
    }
    fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Backlink sources of `path` (excluding itself) with their link counts.
fn backlink_entries(cache: &MetadataCache, path: &NotePath) -> Vec<(NotePath, usize)> {
    let mut counts: BTreeMap<NotePath, usize> = BTreeMap::new();
    for b in cache.backlinks(path) {
        if &b.source != path {
            *counts.entry(b.source).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

/// Export every note as an HTML page, copy attachments, write `style.css`
/// and the index page.
pub fn export_site(ctx: &Context, out_dir: &Path) -> Result<ExportSummary> {
    let files = ctx
        .vault
        .list_files()
        .map_err(|e| anyhow!("cannot list vault files: {e}"))?;
    let vault_name = ctx.vault.name();
    let index_page = index_page_name(&files);
    let with_sidebar = files.len() <= SIDEBAR_MAX_FILES;
    let mut summary = ExportSummary::default();

    for path in &files {
        if !path.is_markdown() {
            let src = ctx.vault.absolute(path);
            let dest = out_dir.join(path.as_str());
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            fs::copy(&src, &dest).with_context(|| format!("cannot copy {path}"))?;
            summary.attachments += 1;
            continue;
        }
        let out_rel = html::output_path(path);
        let prefix = html::root_prefix(&out_rel);
        let source = ctx
            .vault
            .read_to_string(path)
            .map_err(|e| anyhow!("cannot read {path}: {e}"))?;
        let rendered =
            ors_markdown::render_html(&source, &render_options(path, &prefix, true), &ctx.cache);
        let embeds = VaultEmbeds {
            vault: &ctx.vault,
            cache: &ctx.cache,
            prefix: prefix.clone(),
        };
        let content =
            html::inline_embeds(&rendered, path, &embeds, &mut vec![html::page_key(path)], 0);
        let sidebar = if with_sidebar {
            html::file_tree(&files, &prefix, Some(path))
        } else {
            String::new()
        };
        let footer = html::backlinks_section(&backlink_entries(&ctx.cache, path), &prefix);
        let page = html::page(&Page {
            title: path.stem(),
            vault_name: &vault_name,
            prefix: &prefix,
            index_page,
            sidebar: &sidebar,
            content: &content,
            footer: &footer,
            inline_title: true,
        });
        write_file(out_dir, &out_rel, page.as_bytes())?;
        summary.notes += 1;
    }

    write_file(out_dir, "style.css", html::STYLE_CSS.as_bytes())?;
    let tree = html::file_tree(&files, "", None);
    let index_content = format!(
        "<p class=\"vault-summary\">{} notes, {} attachments.</p>\n{tree}",
        summary.notes, summary.attachments
    );
    let index = html::page(&Page {
        title: &vault_name,
        vault_name: &vault_name,
        prefix: "",
        index_page,
        sidebar: "",
        content: &index_content,
        footer: "",
        inline_title: true,
    });
    write_file(out_dir, index_page, index.as_bytes())?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("obsirust-export-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn out_dir_checks() {
        let vault = tmp("vault");
        fs::create_dir_all(&vault).unwrap();
        let out = tmp("out");
        prepare_out_dir(&vault, &out, false).unwrap();
        assert!(out.is_dir());
        // Empty: fine again.
        prepare_out_dir(&vault, &out, false).unwrap();
        fs::write(out.join("x"), "x").unwrap();
        let err = prepare_out_dir(&vault, &out, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not empty"), "{err}");
        prepare_out_dir(&vault, &out, true).unwrap();
        // Exporting into the vault itself is refused, even with --force.
        fs::write(vault.join("a.md"), "a").unwrap();
        let err = prepare_out_dir(&vault, &vault, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("vault folder"), "{err}");
        // So is any folder inside the vault, existing or not.
        assert!(prepare_out_dir(&vault, &vault.join("site"), true).is_err());
        fs::create_dir_all(vault.join("pub")).unwrap();
        assert!(prepare_out_dir(&vault, &vault.join("pub/out"), true).is_err());
        let file = out.join("x");
        assert!(prepare_out_dir(&vault, &file, true).is_err());
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn index_name_avoids_clash() {
        let a = NotePath::new("a.md").unwrap();
        let idx = NotePath::new("index.md").unwrap();
        let sub = NotePath::new("x/index.md").unwrap();
        assert_eq!(index_page_name(&[a.clone(), sub]), "index.html");
        assert_eq!(index_page_name(&[a, idx]), "vault-index.html");
    }

    #[test]
    fn render_options_for_export() {
        let o = render_options(&NotePath::new("a/b.md").unwrap(), "../", true);
        assert_eq!(o.link_prefix, "../");
        assert_eq!(o.note_extension.as_deref(), Some("html"));
        assert!(o.show_frontmatter);
    }
}
