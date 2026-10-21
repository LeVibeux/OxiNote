//! `obsirust-app`: desktop application (eframe/egui) for Obsidian vaults.
//!
//! ```text
//! obsirust-app [VAULT_PATH]
//! ```
//!
//! Without a path a vault picker is shown. Hidden debugging flags (used to
//! capture screenshots headlessly): `--demo`, `--screenshot FILE`,
//! `--frames N`, `--size WxH`, `--open NOTE` (repeatable), `--reading`,
//! `--graph`, `--palette`, `--switcher QUERY`, `--search QUERY`,
//! `--theme dark|light`, `--left files|search|tags|properties`,
//! `--right backlinks|outgoing|outline|properties`, `--fresh`, `--renderer glow|wgpu`.

mod backend;
mod bookmarks;
mod commands;
mod editor;
mod explorer;
mod fake;
mod graph_model;
mod highlight;
mod hotkeys;
mod plugins;
mod properties_model;
mod reading;
mod settings;
mod state;
mod ui;
mod util;
mod views;
mod workbench;
mod workspace;
mod ws_commands;

use std::path::PathBuf;

use settings::{LeftPane, RightPane, ThemeChoice};
use ui::Startup;

const USAGE: &str = "usage: obsirust-app [VAULT_PATH] [--demo]";

struct Cli {
    startup: Startup,
    size: (f32, f32),
    renderer: eframe::Renderer,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli {
        startup: Startup::default(),
        size: (1280.0, 800.0),
        renderer: eframe::Renderer::Glow,
    };
    let mut it = args.into_iter();
    let value = |it: &mut dyn Iterator<Item = String>, flag: &str| {
        it.next().ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(a) = it.next() {
        let s = &mut cli.startup;
        match a.as_str() {
            "-h" | "--help" => return Err(USAGE.into()),
            "--demo" => s.demo = true,
            "--fresh" => s.fresh = true,
            "--reading" => s.reading = true,
            "--source" => s.source = true,
            "--graph" => s.graph = true,
            "--graph-panel" => s.graph_panel = true,
            "--palette" => s.palette = true,
            "--screenshot" => s.screenshot = Some(PathBuf::from(value(&mut it, &a)?)),
            "--frames" => {
                s.frames = value(&mut it, &a)?
                    .parse()
                    .map_err(|_| "--frames expects a number".to_string())?;
            }
            "--open" => s.open.push(value(&mut it, &a)?),
            "--settings" => s.settings = Some(value(&mut it, &a)?),
            "--split" => s.split = true,
            "--split-down" => s.split_down = true,
            "--fold-all" => s.fold_all = true,
            "--switcher" => s.switcher = Some(value(&mut it, &a)?),
            "--search" => s.search = Some(value(&mut it, &a)?),
            "--theme" => {
                s.theme = Some(match value(&mut it, &a)?.as_str() {
                    "dark" => ThemeChoice::Dark,
                    "light" => ThemeChoice::Light,
                    _ => ThemeChoice::System,
                });
            }
            "--left" => {
                s.left = Some(match value(&mut it, &a)?.as_str() {
                    "search" => LeftPane::Search,
                    "tags" => LeftPane::Tags,
                    "properties" => LeftPane::Properties,
                    "bookmarks" => LeftPane::Bookmarks,
                    _ => LeftPane::Files,
                });
            }
            "--right" => {
                s.right = Some(match value(&mut it, &a)?.as_str() {
                    "outgoing" => RightPane::Outgoing,
                    "outline" => RightPane::Outline,
                    "graph" | "localgraph" => RightPane::LocalGraph,
                    "properties" => RightPane::FileProperties,
                    _ => RightPane::Backlinks,
                });
            }
            "--size" => {
                let v = value(&mut it, &a)?;
                let (w, h) = v.split_once('x').ok_or("--size expects WxH")?;
                cli.size = (
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                );
            }
            "--renderer" => {
                cli.renderer = match value(&mut it, &a)?.as_str() {
                    "wgpu" => eframe::Renderer::Wgpu,
                    _ => eframe::Renderer::Glow,
                };
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            path => s.vault = Some(PathBuf::from(path)),
        }
    }
    Ok(cli)
}

fn main() {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    let screenshot_mode = cli.startup.screenshot.is_some();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ObsiRust")
            .with_app_id("obsirust")
            .with_inner_size([cli.size.0, cli.size.1])
            .with_min_inner_size([640.0, 400.0]),
        renderer: cli.renderer,
        persist_window: !screenshot_mode,
        ..Default::default()
    };
    let startup = cli.startup;
    let result = eframe::run_native(
        "ObsiRust",
        options,
        Box::new(move |cc| Ok(Box::new(ui::ObsiApp::new(cc, startup)))),
    );
    if let Err(e) = result {
        eprintln!("obsirust-app: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_cli() {
        let c = parse_args(args(&[
            "vault", "--demo", "--frames", "5", "--size", "800x600", "--open", "A",
        ]))
        .unwrap();
        assert_eq!(c.startup.vault, Some(PathBuf::from("vault")));
        assert!(c.startup.demo);
        assert_eq!(c.startup.frames, 5);
        assert_eq!(c.size, (800.0, 600.0));
        assert_eq!(c.startup.open, vec!["A".to_string()]);
        assert!(parse_args(args(&["--frames"])).is_err());
        assert!(parse_args(args(&["--bogus"])).is_err());
    }
}
