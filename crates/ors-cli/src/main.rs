//! `obsirust` command-line interface: headless tools for Obsidian vaults.
//!
//! Every subcommand takes the vault folder as its first argument. Pure
//! computation and formatting live in the modules (unit-tested); this file
//! only parses arguments and wires the vault, the metadata cache and stdout.

mod commands;
mod context;
mod date;
mod export;
mod graph_cmd;
mod html;
mod output;

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

/// Headless tools for Obsidian vaults: statistics, search, links, graph,
/// renaming with link updates, daily notes and static HTML export.
#[derive(Debug, Parser)]
#[command(name = "obsirust", version)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show statistics: notes, attachments, links, tags, orphans, words.
    Stats {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Full-text search with Obsidian's syntax (tag:, path:, file:, "phrase", -word, OR, /regex/ ...).
    ///
    /// Prints `path:line: text` for each matching line (matches in bold when
    /// stdout is a terminal), or only `path` for files matched by name/tag.
    Search {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Search query, e.g. 'tag:#projet "rust"'.
        query: String,
        /// Maximum number of files to show.
        #[arg(long, short = 'n')]
        limit: Option<usize>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the links pointing to a note (backlinks).
    Backlinks {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Note path ("folder/Note.md", "folder/Note") or name ("Note"), resolved like a wikilink.
        note: String,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the outgoing links of a note, marking resolved (✓) and unresolved (✗) ones.
    Links {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Note path or name, resolved like a wikilink.
        note: String,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List link targets that do not exist, with the notes linking to them.
    Unresolved {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List tags with their number of occurrences (nested tags count for their parents).
    Tags {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Show nested tags (`a/b`) as a tree.
        #[arg(long)]
        tree: bool,
        /// Sort by count (descending) instead of by name.
        #[arg(long, conflicts_with = "tree")]
        by_count: bool,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List notes without any incoming or outgoing link.
    Orphans {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fuzzy-find notes by name, path or alias (like the quick switcher).
    Find {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Fuzzy query, e.g. "rsnt" for "Rust notes".
        query: String,
        /// Maximum number of results.
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Export the link graph as JSON or Graphviz DOT.
    Graph {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = GraphFormat::Json)]
        format: GraphFormat,
        /// Include tags as nodes.
        #[arg(long)]
        tags: bool,
        /// Include attachments as nodes.
        #[arg(long)]
        attachments: bool,
        /// Include unresolved link targets as nodes.
        #[arg(long)]
        unresolved: bool,
        /// Hide notes without links.
        #[arg(long)]
        hide_orphans: bool,
        /// Only the local graph around this note (path or name).
        #[arg(long, value_name = "NOTE")]
        local: Option<String>,
        /// Depth of the local graph.
        #[arg(long, default_value_t = 1, requires = "local")]
        depth: usize,
        /// Run the force-directed layout and include x/y coordinates.
        #[arg(long)]
        layout: bool,
    },
    /// Rename or move a note and update every link pointing to it.
    Rename {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Note to rename (path or name).
        from: String,
        /// New vault path, e.g. "Projets/New name" (".md" is added if missing).
        to: String,
        /// Only show what would change.
        #[arg(long)]
        dry_run: bool,
    },
    /// Open (create if needed) the daily note and print its path on disk.
    Daily {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Date as YYYY-MM-DD (default: today, UTC).
        #[arg(long)]
        date: Option<String>,
    },
    /// Export the vault as a static HTML site.
    Export {
        /// Path of the vault folder.
        vault: PathBuf,
        /// Output folder (must be empty unless --force).
        out_dir: PathBuf,
        /// Write into a non-empty output folder, overwriting files.
        #[arg(long)]
        force: bool,
    },
}

/// Output formats of the `graph` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum GraphFormat {
    /// JSON object with `nodes` and `edges`.
    Json,
    /// Graphviz DOT.
    Dot,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // `--help` / `--version` are not errors.
            let code = if e.use_stderr() { 1 } else { 0 };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    let stdout = io::stdout();
    let color = stdout.is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut out = io::BufWriter::new(stdout.lock());
    let result = run(cli, &mut out, color).and_then(|()| Ok(out.flush()?));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            drop(out);
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::BrokenPipe)
    })
}

fn run(cli: Cli, out: &mut dyn Write, color: bool) -> Result<()> {
    match cli.command {
        Command::Stats { vault, json } => {
            let ctx = context::Context::load(&vault)?;
            let stats = commands::Stats::compute(&ctx.cache);
            if json {
                output::write_json(out, &stats)?;
            } else {
                write!(out, "{}", commands::format_stats(&ctx.vault.name(), &stats))?;
            }
        }
        Command::Search {
            vault,
            query,
            limit,
            json,
        } => {
            let ctx = context::Context::load(&vault)?;
            let index = ors_index::SearchIndex::build(&ctx.vault, &ctx.cache)?;
            let mut results = index
                .search(&query, &ctx.cache)
                .map_err(|e| anyhow::anyhow!("invalid query {query:?}: {e}"))?;
            if let Some(limit) = limit {
                results.truncate(limit);
            }
            if json {
                output::write_json(out, &commands::search_json(&results))?;
            } else {
                write!(out, "{}", commands::format_search(&results, color))?;
                if results.is_empty() {
                    eprintln!("no results for {query:?}");
                }
            }
        }
        Command::Backlinks { vault, note, json } => {
            let ctx = context::Context::load(&vault)?;
            let path = ctx.resolve_note(&note)?;
            let backlinks = ctx.cache.backlinks(&path);
            if json {
                output::write_json(out, &commands::backlinks_json(&backlinks))?;
            } else {
                write!(out, "{}", commands::format_backlinks(&backlinks))?;
                if backlinks.is_empty() {
                    eprintln!("no backlinks to {path}");
                }
            }
        }
        Command::Links { vault, note, json } => {
            let ctx = context::Context::load(&vault)?;
            let path = ctx.resolve_note(&note)?;
            let links = ctx.cache.outgoing(&path);
            if json {
                output::write_json(out, &commands::links_json(&links))?;
            } else {
                write!(out, "{}", commands::format_links(&links))?;
                if links.is_empty() {
                    eprintln!("no outgoing links in {path}");
                }
            }
        }
        Command::Unresolved { vault, json } => {
            let ctx = context::Context::load(&vault)?;
            let groups = commands::group_unresolved(&ctx.cache.unresolved());
            if json {
                output::write_json(out, &groups)?;
            } else {
                write!(out, "{}", commands::format_unresolved(&groups))?;
            }
        }
        Command::Tags {
            vault,
            tree,
            by_count,
            json,
        } => {
            let ctx = context::Context::load(&vault)?;
            let tags = ctx.cache.tags();
            if json {
                output::write_json(out, &tags)?;
            } else if tree {
                write!(out, "{}", commands::format_tag_tree(&tags))?;
            } else {
                write!(out, "{}", commands::format_tags(&tags, by_count))?;
            }
        }
        Command::Orphans { vault, json } => {
            let ctx = context::Context::load(&vault)?;
            let orphans = ctx.cache.orphans();
            if json {
                output::write_json(out, &orphans)?;
            } else {
                write!(out, "{}", commands::format_paths(&orphans))?;
            }
        }
        Command::Find {
            vault,
            query,
            limit,
            json,
        } => {
            let ctx = context::Context::load(&vault)?;
            let found = ors_index::fuzzy_find(&ctx.cache, &query, limit);
            if json {
                output::write_json(out, &commands::find_json(&found))?;
            } else {
                write!(out, "{}", commands::format_find(&found))?;
            }
        }
        Command::Graph {
            vault,
            format,
            tags,
            attachments,
            unresolved,
            hide_orphans,
            local,
            depth,
            layout,
        } => {
            let ctx = context::Context::load(&vault)?;
            let options = ors_index::GraphOptions {
                include_tags: tags,
                include_attachments: attachments,
                include_unresolved: unresolved,
                include_orphans: !hide_orphans,
            };
            let mut graph = ctx.cache.graph(&options);
            if let Some(note) = local {
                let path = ctx.resolve_note(&note)?;
                let center = graph
                    .node_index(path.as_str())
                    .ok_or_else(|| anyhow::anyhow!("{path} is not in the graph"))?;
                graph = ors_graph::local_graph(&graph, center, depth);
            }
            let positions = layout.then(|| graph_cmd::layout(&graph));
            let text = match format {
                GraphFormat::Json => graph_cmd::to_json(&graph, positions.as_deref())?,
                GraphFormat::Dot => graph_cmd::to_dot(&graph, positions.as_deref()),
            };
            writeln!(out, "{text}")?;
        }
        Command::Rename {
            vault,
            from,
            to,
            dry_run,
        } => {
            let mut ctx = context::Context::load(&vault)?;
            let report = commands::rename(&mut ctx, &from, &to, dry_run)?;
            write!(out, "{report}")?;
        }
        Command::Daily { vault, date } => {
            let vault_dir = context::check_vault_dir(&vault)?;
            let date = match date {
                Some(s) => date::parse_date(&s)?,
                None => date::today_utc(),
            };
            let v = ors_vault::Vault::open(&vault_dir)
                .map_err(|e| anyhow::anyhow!("cannot open vault {}: {e}", vault.display()))?;
            let path = v
                .open_daily_note(date)
                .map_err(|e| anyhow::anyhow!("cannot open the daily note: {e}"))?;
            writeln!(out, "{}", v.absolute(&path).display())?;
        }
        Command::Export {
            vault,
            out_dir,
            force,
        } => {
            let vault_dir = context::check_vault_dir(&vault)?;
            export::prepare_out_dir(&vault_dir, &out_dir, force)?;
            let ctx = context::Context::load(&vault)?;
            let summary = export::export_site(&ctx, &out_dir)?;
            writeln!(
                out,
                "Exported {} notes and {} attachments to {}",
                summary.notes,
                summary.attachments,
                out_dir.display()
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_graph_flags() {
        let cli = Cli::try_parse_from([
            "obsirust", "graph", "v", "--format", "dot", "--tags", "--layout",
        ])
        .unwrap();
        match cli.command {
            Command::Graph {
                format,
                tags,
                layout,
                ..
            } => {
                assert_eq!(format, GraphFormat::Dot);
                assert!(tags && layout);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(Cli::try_parse_from(["obsirust", "graph", "v", "--depth", "2"]).is_err());
    }
}
