//! Live Preview decorations: which markdown markers to hide on lines that do
//! not hold the caret, and which widgets (bullets, checkboxes, image embeds,
//! rules, quote bars, callout boxes) to draw in their place.
//!
//! The result only depends on the text; the editor decides per frame which
//! lines are "active" (raw markdown shown) and caches the rest.

use std::ops::Range;

use super::format::{ListKind, parse_prefix};
use crate::highlight::{Style, highlight};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conceal {
    /// Zero width, invisible.
    Hide,
    /// Keeps its width but is not drawn (a widget is painted over it).
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Frontmatter,
    /// ```` ``` ```` fence line.
    CodeFence,
    Code,
    Heading(u8),
    Rule,
    Quote,
    Callout { kind: String, header: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineInfo {
    /// Byte range without the line break.
    pub range: Range<usize>,
    pub kind: LineKind,
    /// Checked task: content drawn muted and struck through.
    pub done: bool,
    /// Byte where the content starts (after list/quote/heading markers).
    pub content: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Widget {
    /// `-`/`*`/`+` drawn as `•` (at the marker byte).
    Bullet { at: usize },
    /// `[ ]` / `[x]`: `range` is the bracket part, `status_at` the status char.
    Checkbox {
        range: Range<usize>,
        status_at: usize,
        checked: bool,
    },
    /// `![[target]]` (whole range).
    Embed { range: Range<usize>, target: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// `[[target#sub|alias]]`: target and optional subpath (without `#`).
    Wiki {
        target: String,
        subpath: Option<String>,
    },
    /// `[text](dest)`.
    Markdown(String),
    /// `#tag` (without `#`).
    Tag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    pub range: Range<usize>,
    pub target: LinkTarget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decor {
    pub lines: Vec<LineInfo>,
    /// Sorted, non-overlapping; `.2` is the line index.
    pub conceals: Vec<(Range<usize>, Conceal, usize)>,
    /// With their line index.
    pub widgets: Vec<(Widget, usize)>,
    pub links: Vec<LinkSpan>,
}

impl Decor {
    /// Index of the line containing byte `at`.
    pub fn line_at(&self, at: usize) -> usize {
        self.lines
            .partition_point(|l| l.range.end < at)
            .min(self.lines.len().saturating_sub(1))
    }

    /// Link under byte `at`.
    pub fn link_at(&self, at: usize) -> Option<&LinkSpan> {
        self.links
            .iter()
            .find(|l| l.range.start <= at && at < l.range.end)
    }
}

/// Is `t` (trimmed line) a thematic break (`---`, `***`, `___`, `- - -`)?
pub fn is_rule(t: &str) -> bool {
    let t = t.trim();
    let Some(c) = t.chars().next() else {
        return false;
    };
    matches!(c, '-' | '*' | '_')
        && t.chars().all(|x| x == c || x == ' ')
        && t.chars().filter(|x| *x == c).count() >= 3
}

const INLINE_MARKERS: &[&str] = &["**", "__", "*", "_", "==", "~~"];

pub fn decorate(text: &str) -> Decor {
    let spans = highlight(text);
    let mut d = Decor::default();
    let mut off = 0;
    let mut in_fm = false;
    let mut fence: Option<&str> = None;
    let mut callout: Option<String> = None;
    let mut si = 0;
    for (n, raw) in text.split_inclusive('\n').chain(
        // A trailing empty line after a final '\n'.
        text.ends_with('\n').then_some("").into_iter(),
    )
    .enumerate()
    {
        let content = raw.trim_end_matches(['\n', '\r']);
        let range = off..off + content.len();
        let trimmed = content.trim_start();
        let mut info = LineInfo {
            range: range.clone(),
            kind: LineKind::Normal,
            done: false,
            content: range.start,
        };
        if n == 0 && content == "---" {
            in_fm = true;
            info.kind = LineKind::Frontmatter;
        } else if in_fm {
            if content == "---" || content == "..." {
                in_fm = false;
            }
            info.kind = LineKind::Frontmatter;
        } else if let Some(f) = fence {
            if trimmed.starts_with(f) {
                fence = None;
                info.kind = LineKind::CodeFence;
            } else {
                info.kind = LineKind::Code;
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            info.kind = LineKind::CodeFence;
        } else {
            // Spans of this line.
            while si < spans.len() && spans[si].0.end <= range.start {
                si += 1;
            }
            let line_spans: Vec<(Range<usize>, Style)> = spans[si..]
                .iter()
                .take_while(|(r, _)| r.start < range.end)
                .map(|(r, s)| (r.start.max(range.start)..r.end.min(range.end), *s))
                .filter(|(r, _)| !r.is_empty())
                .collect();
            decorate_line(text, n, &mut info, &line_spans, &mut callout, &mut d);
        }
        if !matches!(info.kind, LineKind::Quote | LineKind::Callout { .. }) {
            callout = None;
        }
        d.lines.push(info);
        off += raw.len();
    }
    d.conceals.sort_by_key(|(r, _, _)| r.start);
    d
}

fn decorate_line(
    text: &str,
    n: usize,
    info: &mut LineInfo,
    spans: &[(Range<usize>, Style)],
    callout: &mut Option<String>,
    d: &mut Decor,
) {
    let ls = info.range.start;
    let line = &text[info.range.clone()];
    let in_comment = spans.first().is_some_and(|(_, s)| s.comment);
    if in_comment {
        return;
    }
    if is_rule(line) && !line.starts_with("    ") {
        info.kind = LineKind::Rule;
        d.conceals.push((info.range.clone(), Conceal::Blank, n));
        return;
    }
    let mut content = ls;
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && matches!(line.as_bytes().get(hashes), Some(b' ' | b'\t') | None)
    {
        info.kind = LineKind::Heading(hashes as u8);
        let spaces = line[hashes..].len() - line[hashes..].trim_start().len();
        content = ls + hashes + spaces;
        d.conceals.push((ls..content, Conceal::Hide, n));
    } else if let Some(p) = parse_prefix(line) {
        match p.kind {
            ListKind::Quote => {
                let rest = &line[p.marker_end..];
                let header = rest
                    .strip_prefix("[!")
                    .and_then(|r| r.find(']').map(|i| (r[..i].to_string(), i)));
                match header {
                    Some((kind, i)) => {
                        let mut end = p.marker_end + 2 + i + 1;
                        if matches!(line.as_bytes().get(end), Some(b'+' | b'-')) {
                            end += 1;
                        }
                        if line.as_bytes().get(end) == Some(&b' ') {
                            end += 1;
                        }
                        *callout = Some(kind.to_lowercase());
                        info.kind = LineKind::Callout {
                            kind: kind.to_lowercase(),
                            header: true,
                        };
                        content = ls + end;
                    }
                    None => {
                        info.kind = match callout {
                            Some(k) => LineKind::Callout {
                                kind: k.clone(),
                                header: false,
                            },
                            None => LineKind::Quote,
                        };
                        content = ls + p.marker_end;
                    }
                }
                d.conceals.push((ls + p.indent..content, Conceal::Hide, n));
            }
            ListKind::Bullet(_) | ListKind::Ordered { .. } => {
                content = ls + p.end;
                if let Some(status) = p.task {
                    let bullet = matches!(p.kind, ListKind::Bullet(_));
                    if bullet {
                        d.conceals
                            .push((ls + p.indent..ls + p.marker_end, Conceal::Hide, n));
                    }
                    let br = ls + p.marker_end..ls + p.task_at + status.len_utf8() + 1;
                    d.conceals.push((br.clone(), Conceal::Blank, n));
                    let checked = status != ' ';
                    info.done = checked;
                    d.widgets.push((
                        Widget::Checkbox {
                            range: br,
                            status_at: ls + p.task_at,
                            checked,
                        },
                        n,
                    ));
                } else if matches!(p.kind, ListKind::Bullet(_)) {
                    d.conceals
                        .push((ls + p.indent..ls + p.indent + 1, Conceal::Blank, n));
                    d.widgets.push((Widget::Bullet { at: ls + p.indent }, n));
                }
            }
        }
    }
    info.content = content;
    let line_end = info.range.end;
    let mut skip_until = content;
    for (r, s) in spans {
        if r.start < skip_until {
            continue;
        }
        let t = &text[r.clone()];
        if s.comment {
            continue;
        }
        if s.code {
            let lead = t.bytes().take_while(|b| *b == b'`').count();
            let trail = t.bytes().rev().take_while(|b| *b == b'`').count();
            if lead > 0 && trail > 0 && lead + trail < t.len() {
                d.conceals.push((r.start..r.start + lead, Conceal::Hide, n));
                d.conceals.push((r.end - trail..r.end, Conceal::Hide, n));
            }
            continue;
        }
        if s.tag {
            d.links.push(LinkSpan {
                range: r.clone(),
                target: LinkTarget::Tag(t.trim_start_matches('#').to_string()),
            });
            continue;
        }
        if s.syntax && s.link && (t.starts_with("[[") || t.starts_with("![[")) {
            let open = if t.starts_with("![[") { 3 } else { 2 };
            let Some(rel) = text[r.start + open..line_end].find("]]") else {
                continue;
            };
            let close = r.start + open + rel;
            let inner = &text[r.start + open..close];
            let (target_part, alias) = match inner.find('|') {
                Some(p) => (&inner[..p], Some(p)),
                None => (inner, None),
            };
            let (target, subpath) = match target_part.split_once('#') {
                Some((t, sp)) => (t.to_string(), Some(sp.to_string())),
                None => (target_part.to_string(), None),
            };
            let whole = r.start..close + 2;
            d.conceals.push((r.start..r.start + open, Conceal::Hide, n));
            if let Some(p) = alias {
                d.conceals
                    .push((r.start + open..r.start + open + p + 1, Conceal::Hide, n));
            }
            d.conceals.push((close..close + 2, Conceal::Hide, n));
            if open == 3 {
                d.widgets.push((
                    Widget::Embed {
                        range: whole.clone(),
                        target: target.clone(),
                    },
                    n,
                ));
            }
            d.links.push(LinkSpan {
                range: whole.clone(),
                target: LinkTarget::Wiki { target, subpath },
            });
            skip_until = whole.end;
            continue;
        }
        if s.syntax && s.link && t == "[" {
            let rest = &text[r.start..line_end];
            if let Some(close) = rest.find("](")
                && let Some(end) = rest[close..].find(')')
            {
                let dest = &rest[close + 2..close + end];
                let end_b = r.start + close + end + 1;
                d.conceals.push((r.start..r.start + 1, Conceal::Hide, n));
                d.conceals
                    .push((r.start + close..end_b, Conceal::Hide, n));
                d.links.push(LinkSpan {
                    range: r.start..end_b,
                    target: LinkTarget::Markdown(dest.to_string()),
                });
                skip_until = end_b;
            }
            continue;
        }
        if s.syntax && !s.link && INLINE_MARKERS.contains(&t) {
            d.conceals.push((r.clone(), Conceal::Hide, n));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text with Hide ranges removed (what a non-active line shows).
    fn shown(text: &str, d: &Decor) -> String {
        let mut out = String::new();
        let mut pos = 0;
        for (r, c, _) in &d.conceals {
            out.push_str(&text[pos..r.start]);
            if *c == Conceal::Blank {
                out.push_str(&"·".repeat(text[r.clone()].chars().count()));
            }
            pos = r.end;
        }
        out.push_str(&text[pos..]);
        out
    }

    #[test]
    fn hides_inline_markers_utf8() {
        let t = "## Idées **gras** *ité* ~~barré~~ ==surligné== `code` fin";
        let d = decorate(t);
        assert_eq!(d.lines[0].kind, LineKind::Heading(2));
        assert_eq!(shown(t, &d), "Idées gras ité barré surligné code fin");
        // Every conceal is on char boundaries.
        for (r, _, _) in &d.conceals {
            assert!(t.is_char_boundary(r.start) && t.is_char_boundary(r.end));
        }
    }

    #[test]
    fn links_and_tags() {
        let t = "Voir [[Projets/ObsiRust|le projet]], [[Idées#Fonc]] et [doc](https://x.y) #tag/é";
        let d = decorate(t);
        assert_eq!(shown(t, &d), "Voir le projet, Idées#Fonc et doc #tag/é");
        assert_eq!(d.links.len(), 4);
        let at = t.find("le projet").unwrap();
        assert_eq!(
            d.link_at(at).map(|l| &l.target),
            Some(&LinkTarget::Wiki {
                target: "Projets/ObsiRust".into(),
                subpath: None
            })
        );
        let at = t.find("Fonc").unwrap();
        assert!(matches!(
            &d.link_at(at).unwrap().target,
            LinkTarget::Wiki { subpath: Some(s), .. } if s == "Fonc"
        ));
        assert_eq!(
            d.link_at(t.find("doc").unwrap()).map(|l| &l.target),
            Some(&LinkTarget::Markdown("https://x.y".into()))
        );
        assert_eq!(
            d.link_at(t.find("#tag").unwrap()).map(|l| &l.target),
            Some(&LinkTarget::Tag("tag/é".into()))
        );
    }

    #[test]
    fn lists_tasks_and_embeds() {
        let t = "- puce\n- [ ] à faire\n- [x] fait\n1. un\n![[diagramme.png]]\n";
        let d = decorate(t);
        assert_eq!(
            shown(t, &d),
            "· puce\n··· à faire\n··· fait\n1. un\ndiagramme.png\n"
        );
        assert!(d.lines[2].done);
        assert!(!d.lines[1].done);
        assert_eq!(d.lines.len(), 6);
        let boxes: Vec<_> = d
            .widgets
            .iter()
            .filter_map(|(w, l)| match w {
                Widget::Checkbox {
                    checked, status_at, ..
                } => Some((*l, *checked, *status_at)),
                _ => None,
            })
            .collect();
        assert_eq!(boxes.len(), 2);
        assert_eq!(&t[boxes[0].2..boxes[0].2 + 1], " ");
        assert_eq!(&t[boxes[1].2..boxes[1].2 + 1], "x");
        assert!(d.widgets.iter().any(|(w, l)| *l == 4
            && matches!(w, Widget::Embed { target, .. } if target == "diagramme.png")));
        assert!(matches!(d.widgets[0].0, Widget::Bullet { at: 0 }));
    }

    #[test]
    fn blocks() {
        let t = "---\ntags: [a]\n---\n> [!tip] Astuce\n> corps **b**\n\n> citation\n---\n```rust\nlet **x** = 1;\n```\n%% **c** %%";
        let d = decorate(t);
        let kinds: Vec<&LineKind> = d.lines.iter().map(|l| &l.kind).collect();
        assert_eq!(kinds[0], &LineKind::Frontmatter);
        assert_eq!(kinds[2], &LineKind::Frontmatter);
        assert_eq!(
            kinds[3],
            &LineKind::Callout {
                kind: "tip".into(),
                header: true
            }
        );
        assert_eq!(
            kinds[4],
            &LineKind::Callout {
                kind: "tip".into(),
                header: false
            }
        );
        assert_eq!(kinds[6], &LineKind::Quote);
        assert_eq!(kinds[7], &LineKind::Rule);
        assert_eq!(kinds[8], &LineKind::CodeFence);
        assert_eq!(kinds[9], &LineKind::Code);
        let s = shown(t, &d);
        assert!(s.contains("\nAstuce\ncorps b\n"), "{s}");
        assert!(s.contains("let **x** = 1;"), "code untouched: {s}");
        assert!(s.contains("%% **c** %%"), "comments untouched: {s}");
        assert!(s.contains("\n···\n```rust"), "rule blanked: {s}");
        assert_eq!(d.line_at(0), 0);
        assert_eq!(d.line_at(t.find("corps").unwrap()), 4);
    }

    #[test]
    fn rules() {
        assert!(is_rule("---"));
        assert!(is_rule("* * *"));
        assert!(!is_rule("--"));
        assert!(!is_rule("-- x"));
    }
}
