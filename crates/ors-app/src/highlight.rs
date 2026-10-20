//! Lexical markdown highlighting for the source editor.
//!
//! This is a *syntax colouring* pass only (like a code editor's), it does not
//! extract metadata: links/tags shown in the sidebars come from `ors-index`.
//! Every byte of the input is covered by exactly one returned span.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Style {
    /// 1..=6 for heading lines, 0 otherwise.
    pub heading: u8,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub tag: bool,
    pub highlight: bool,
    pub comment: bool,
    pub frontmatter: bool,
    pub quote: bool,
    /// Markup characters (`**`, `[[`, `#`, list markers...), drawn dimmed.
    pub syntax: bool,
}

#[derive(Default, Clone, Copy)]
struct Inline {
    bold: bool,
    italic: bool,
    strike: bool,
    highlight: bool,
}

struct Out {
    spans: Vec<(Range<usize>, Style)>,
}

impl Out {
    fn push(&mut self, range: Range<usize>, style: Style) {
        if range.is_empty() {
            return;
        }
        if let Some((last, s)) = self.spans.last_mut()
            && *s == style
            && last.end == range.start
        {
            last.end = range.end;
            return;
        }
        self.spans.push((range, style));
    }
}

/// Split `text` into styled spans.
pub fn highlight(text: &str) -> Vec<(Range<usize>, Style)> {
    let mut out = Out { spans: Vec::new() };
    let mut offset = 0;
    let mut in_frontmatter = false;
    let mut fence: Option<&str> = None;
    let mut in_comment = false;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\n', '\r']);
        let range = offset..offset + line.len();
        let trimmed = content.trim_start();
        if n == 0 && content == "---" {
            in_frontmatter = true;
            out.push(range, frontmatter_style());
        } else if in_frontmatter {
            if content == "---" || content == "..." {
                in_frontmatter = false;
            }
            out.push(range, frontmatter_style());
        } else if let Some(f) = fence {
            if trimmed.starts_with(f) {
                fence = None;
            }
            out.push(range, code_style());
        } else if !in_comment && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            fence = Some(&trimmed[..3]);
            out.push(range, code_style());
        } else {
            highlight_line(line, offset, &mut in_comment, &mut out);
        }
        offset += line.len();
    }
    out.spans
}

fn frontmatter_style() -> Style {
    Style {
        frontmatter: true,
        ..Style::default()
    }
}

fn code_style() -> Style {
    Style {
        code: true,
        ..Style::default()
    }
}

fn highlight_line(line: &str, offset: usize, in_comment: &mut bool, out: &mut Out) {
    let bytes = line.as_bytes();
    let mut base = Style::default();
    let mut i = 0;

    // Block-level prefix.
    if !*in_comment {
        let hashes = bytes.iter().take_while(|b| **b == b'#').count();
        if (1..=6).contains(&hashes)
            && matches!(bytes.get(hashes), Some(b' ' | b'\t' | b'\n') | None)
        {
            base.heading = hashes as u8;
            out.push(
                offset..offset + hashes,
                Style {
                    syntax: true,
                    ..base
                },
            );
            i = hashes;
        } else {
            let indent = bytes
                .iter()
                .take_while(|b| **b == b' ' || **b == b'\t')
                .count();
            let rest = &line[indent..];
            let marker_len = if rest.starts_with('>') {
                base.quote = true;
                1
            } else if rest.starts_with("- [") && rest.len() >= 6 && rest.as_bytes()[4] == b']' {
                6
            } else if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
                2
            } else {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0 && rest[digits..].starts_with(". ") {
                    digits + 2
                } else {
                    0
                }
            };
            if marker_len > 0 {
                out.push(offset..offset + indent, base);
                out.push(
                    offset + indent..offset + indent + marker_len.min(rest.len()),
                    Style {
                        syntax: true,
                        ..base
                    },
                );
                i = indent + marker_len.min(rest.len());
            }
        }
    }

    let mut inline = Inline::default();
    let style_of = |inline: &Inline, base: Style| Style {
        bold: inline.bold,
        italic: inline.italic,
        strike: inline.strike,
        highlight: inline.highlight,
        ..base
    };
    let mut text_start = i;
    macro_rules! flush {
        ($until:expr) => {
            out.push(
                offset + text_start..offset + $until,
                style_of(&inline, base),
            );
        };
    }

    while i < bytes.len() {
        if *in_comment {
            match line[i..].find("%%") {
                Some(j) => {
                    out.push(offset + i..offset + i + j + 2, comment_style(base));
                    i += j + 2;
                    *in_comment = false;
                }
                None => {
                    out.push(offset + i..offset + bytes.len(), comment_style(base));
                    i = bytes.len();
                }
            }
            text_start = i;
            continue;
        }
        let rest = &line[i..];
        let b = bytes[i];
        if rest.starts_with("%%") {
            flush!(i);
            *in_comment = true;
            out.push(offset + i..offset + i + 2, comment_style(base));
            i += 2;
            text_start = i;
            continue;
        }
        if b == b'`' {
            let ticks = rest.bytes().take_while(|c| *c == b'`').count();
            let closing = "`".repeat(ticks);
            if let Some(j) = rest[ticks..].find(&closing) {
                flush!(i);
                let end = i + ticks + j + ticks;
                out.push(offset + i..offset + end, Style { code: true, ..base });
                i = end;
                text_start = i;
                continue;
            }
        }
        let embed = rest.starts_with("![[");
        if (embed || rest.starts_with("[["))
            && let Some(j) = rest.find("]]")
        {
            flush!(i);
            let open = if embed { 3 } else { 2 };
            let link_style = Style {
                link: true,
                ..style_of(&inline, base)
            };
            let syntax = Style {
                syntax: true,
                ..link_style
            };
            out.push(offset + i..offset + i + open, syntax);
            let inner = &rest[open..j];
            match inner.find('|') {
                Some(pipe) => {
                    out.push(offset + i + open..offset + i + open + pipe + 1, syntax);
                    out.push(offset + i + open + pipe + 1..offset + i + j, link_style);
                }
                None => out.push(offset + i + open..offset + i + j, link_style),
            }
            out.push(offset + i + j..offset + i + j + 2, syntax);
            i += j + 2;
            text_start = i;
            continue;
        }
        if b == b'['
            && let Some(close) = rest.find("](")
            && let Some(end) = rest[close..].find(')')
            && !rest[1..close].contains(['[', ']'])
        {
            flush!(i);
            let link_style = Style {
                link: true,
                ..style_of(&inline, base)
            };
            let syntax = Style {
                syntax: true,
                ..link_style
            };
            out.push(offset + i..offset + i + 1, syntax);
            out.push(offset + i + 1..offset + i + close, link_style);
            out.push(offset + i + close..offset + i + close + end + 1, syntax);
            i += close + end + 1;
            text_start = i;
            continue;
        }
        if b == b'#'
            && base.heading == 0
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
            && rest[1..].chars().next().is_some_and(is_tag_char)
        {
            let len = rest[1..]
                .char_indices()
                .find(|(_, c)| !is_tag_char(*c))
                .map_or(rest.len() - 1, |(k, _)| k);
            if !rest[1..1 + len].chars().all(|c| c.is_ascii_digit()) {
                flush!(i);
                out.push(
                    offset + i..offset + i + 1 + len,
                    Style { tag: true, ..base },
                );
                i += 1 + len;
                text_start = i;
                continue;
            }
        }
        let toggle = if rest.starts_with("**") || rest.starts_with("__") {
            Some((2, 0))
        } else if rest.starts_with("==") {
            Some((2, 1))
        } else if rest.starts_with("~~") {
            Some((2, 2))
        } else if (b == b'*' || b == b'_')
            && !(b == b'_'
                && i > 0
                && bytes[i - 1].is_ascii_alphanumeric()
                && bytes.get(i + 1).is_some_and(u8::is_ascii_alphanumeric))
            && (inline.italic || bytes.get(i + 1).is_some_and(|c| !c.is_ascii_whitespace()))
        {
            Some((1, 3))
        } else {
            None
        };
        if let Some((len, which)) = toggle {
            flush!(i);
            // The marker itself gets the "outer" style plus `syntax`.
            let opening = match which {
                0 => !inline.bold,
                1 => !inline.highlight,
                2 => !inline.strike,
                _ => !inline.italic,
            };
            if opening {
                match which {
                    0 => inline.bold = true,
                    1 => inline.highlight = true,
                    2 => inline.strike = true,
                    _ => inline.italic = true,
                }
            }
            out.push(
                offset + i..offset + i + len,
                Style {
                    syntax: true,
                    ..style_of(&inline, base)
                },
            );
            if !opening {
                match which {
                    0 => inline.bold = false,
                    1 => inline.highlight = false,
                    2 => inline.strike = false,
                    _ => inline.italic = false,
                }
            }
            i += len;
            text_start = i;
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    flush!(bytes.len());
}

fn comment_style(base: Style) -> Style {
    Style {
        comment: true,
        heading: base.heading,
        ..Style::default()
    }
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(text: &str, pred: impl Fn(&Style) -> bool) -> Vec<&str> {
        highlight(text)
            .into_iter()
            .filter(|(_, s)| pred(s))
            .map(|(r, _)| &text[r])
            .collect()
    }

    #[test]
    fn covers_every_byte() {
        let t = "---\na: 1\n---\n# Héllo **b** *i* `c` [[L|x]] #tag ==h== %%c%% ~~s~~\n```\ncode\n```\n> q\n- [ ] t\n";
        let spans = highlight(t);
        let mut pos = 0;
        for (r, _) in &spans {
            assert_eq!(r.start, pos);
            pos = r.end;
        }
        assert_eq!(pos, t.len());
    }

    #[test]
    fn inline_styles() {
        let t = "a **bold** *it* `co` [[Link|alias]] #tag/x ==hi== %%hidden%% end";
        assert_eq!(styled(t, |s| s.bold && !s.syntax), vec!["bold"]);
        assert_eq!(styled(t, |s| s.italic && !s.syntax), vec!["it"]);
        assert_eq!(styled(t, |s| s.code), vec!["`co`"]);
        assert_eq!(styled(t, |s| s.link && !s.syntax), vec!["alias"]);
        assert_eq!(styled(t, |s| s.tag), vec!["#tag/x"]);
        assert_eq!(styled(t, |s| s.highlight && !s.syntax), vec!["hi"]);
        assert_eq!(styled(t, |s| s.comment), vec!["%%hidden%%"]);
    }

    #[test]
    fn block_styles() {
        let t = "---\ntags: [a]\n---\n## Head #nottag\n```\n#no [[no]]\n```\nsnake_case_word\n#1 not tag\n";
        assert_eq!(
            styled(t, |s| s.frontmatter).concat(),
            "---\ntags: [a]\n---\n"
        );
        assert!(
            styled(t, |s| s.heading == 2)
                .concat()
                .contains("Head #nottag")
        );
        assert_eq!(styled(t, |s| s.code).concat(), "```\n#no [[no]]\n```\n");
        assert!(styled(t, |s| s.italic).is_empty());
        assert!(styled(t, |s| s.tag).is_empty());
    }

    #[test]
    fn multiline_comment() {
        let t = "a %% start\nmiddle\nend %% b";
        assert_eq!(
            styled(t, |s| s.comment).concat(),
            "%% start\nmiddle\nend %%"
        );
    }

    #[test]
    fn markdown_link_and_embed() {
        let t = "![[img.png]] and [text](Note.md)";
        assert_eq!(styled(t, |s| s.link && !s.syntax), vec!["img.png", "text"]);
    }
}
