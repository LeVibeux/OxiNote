//! Obsidian search query language: tokenizer and recursive-descent parser
//! producing a [`Query`] AST.
//!
//! Grammar (informal):
//!
//! ```text
//! query    := or
//! or       := and ("OR" and)*
//! and      := unary+
//! unary    := "-" unary | primary
//! primary  := "(" or ")" | OPERATOR ":" operand | "[" property "]"
//!           | "\"phrase\"" | "/regex/" | word
//! operand  := "(" or ")" | "\"phrase\"" | "/regex/" | word
//! property := name (":" query)?
//! ```

use std::fmt;

/// A search operator taking an operand (`file:`, `line:(...)`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// `file:` — match against the file name.
    File,
    /// `path:` — match against the vault path.
    Path,
    /// `content:` — match against the content only (not the file name).
    Content,
    /// `line:(...)` — all terms on the same line.
    Line,
    /// `section:(...)` — all terms in the same heading section.
    Section,
    /// `block:(...)` — all terms in the same block (paragraph).
    Block,
    /// `task:` — inside a task (any status).
    Task,
    /// `task-todo:` — inside an unchecked task (`- [ ]`).
    TaskTodo,
    /// `task-done:` — inside a checked task (any non-space status).
    TaskDone,
    /// `match-case:` — case-sensitive matching.
    MatchCase,
    /// `ignore-case:` — case-insensitive matching (the default).
    IgnoreCase,
}

impl Operator {
    /// Parse an operator name (case-insensitive), without the colon.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "file" => Self::File,
            "path" => Self::Path,
            "content" => Self::Content,
            "line" => Self::Line,
            "section" => Self::Section,
            "block" => Self::Block,
            "task" => Self::Task,
            "task-todo" => Self::TaskTodo,
            "task-done" => Self::TaskDone,
            "match-case" => Self::MatchCase,
            "ignore-case" => Self::IgnoreCase,
            _ => return None,
        })
    }
}

/// Parsed search query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    /// A bare word: substring match in the file name or content.
    Term(String),
    /// `"exact phrase"`.
    Phrase(String),
    /// `/regex/`.
    Regex(String),
    /// `tag:#name` (nested children included). Stored without `#`.
    Tag(String),
    /// `[name]` (property present) or `[name:value]`.
    Property {
        /// Property name as written.
        name: String,
        /// Value sub-query, matched against the property value(s).
        value: Option<Box<Query>>,
    },
    /// `operator:operand`.
    Scoped(Operator, Box<Query>),
    /// Implicit conjunction.
    And(Vec<Query>),
    /// `a OR b`.
    Or(Vec<Query>),
    /// `-x`.
    Not(Box<Query>),
}

/// Invalid query (unbalanced quotes/parentheses, missing operand, bad regex...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    /// Human-readable message.
    pub message: String,
    /// Byte offset in the query where the problem was detected.
    pub position: usize,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.position)
    }
}

impl std::error::Error for QueryError {}

fn err<T>(message: impl Into<String>, position: usize) -> Result<T, QueryError> {
    Err(QueryError {
        message: message.into(),
        position,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    LParen,
    RParen,
    Minus,
    Or,
    Word(String),
    Phrase(String),
    Regex(String),
    Op(Operator),
    /// `tag:`
    TagOp,
    Property(String),
}

fn is_word_end(c: char) -> bool {
    c.is_whitespace() || c == '(' || c == ')' || c == '"'
}

fn tokenize(input: &str) -> Result<Vec<(Tok, usize)>, QueryError> {
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let pos = |i: usize| chars.get(i).map_or(input.len(), |&(p, _)| p);
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push((Tok::LParen, start));
                i += 1;
            }
            ')' => {
                toks.push((Tok::RParen, start));
                i += 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                let mut closed = false;
                while let Some(ch) = at(i) {
                    match ch {
                        '\\' if matches!(at(i + 1), Some('"' | '\\')) => {
                            if let Some(n) = at(i + 1) {
                                s.push(n);
                            }
                            i += 2;
                        }
                        '"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        ch => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return err("unbalanced quote", start);
                }
                toks.push((Tok::Phrase(s), start));
            }
            '[' => {
                let mut s = String::new();
                let mut in_quote = false;
                let mut closed = false;
                i += 1;
                while let Some(ch) = at(i) {
                    i += 1;
                    match ch {
                        '"' => {
                            in_quote = !in_quote;
                            s.push(ch);
                        }
                        ']' if !in_quote => {
                            closed = true;
                            break;
                        }
                        ch => s.push(ch),
                    }
                }
                if !closed {
                    return err("unbalanced bracket", start);
                }
                toks.push((Tok::Property(s), start));
            }
            '-' if at(i + 1).is_some_and(|n| !n.is_whitespace() && n != ')') => {
                toks.push((Tok::Minus, start));
                i += 1;
            }
            '/' => {
                // Regex if a closing unescaped `/` exists.
                let mut j = i + 1;
                let mut pat = String::new();
                let mut closed = false;
                while let Some(ch) = at(j) {
                    match ch {
                        '\\' if at(j + 1) == Some('/') => {
                            pat.push('/');
                            j += 2;
                        }
                        '\\' => {
                            pat.push('\\');
                            if let Some(n) = at(j + 1) {
                                pat.push(n);
                            }
                            j += 2;
                        }
                        '/' => {
                            closed = true;
                            j += 1;
                            break;
                        }
                        ch => {
                            pat.push(ch);
                            j += 1;
                        }
                    }
                }
                if closed && !pat.is_empty() {
                    toks.push((Tok::Regex(pat), start));
                    i = j;
                } else {
                    let (w, next) = read_word(&chars, i);
                    toks.push((Tok::Word(w), start));
                    i = next;
                }
            }
            _ => {
                // Operator?
                let mut j = i;
                while let Some(ch) = at(j) {
                    if is_word_end(ch) || ch == ':' {
                        break;
                    }
                    j += 1;
                }
                if at(j) == Some(':') {
                    let name = &input[start..pos(j)];
                    if name.eq_ignore_ascii_case("tag") {
                        toks.push((Tok::TagOp, start));
                        i = j + 1;
                        continue;
                    }
                    if let Some(op) = Operator::from_name(name) {
                        toks.push((Tok::Op(op), start));
                        i = j + 1;
                        continue;
                    }
                }
                let (w, next) = read_word(&chars, i);
                if w == "OR" {
                    toks.push((Tok::Or, start));
                } else {
                    toks.push((Tok::Word(w), start));
                }
                i = next;
            }
        }
    }
    Ok(toks)
}

fn read_word(chars: &[(usize, char)], mut i: usize) -> (String, usize) {
    let mut w = String::new();
    while let Some(&(_, ch)) = chars.get(i) {
        if is_word_end(ch) {
            break;
        }
        w.push(ch);
        i += 1;
    }
    (w, i)
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    i: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|(t, _)| t)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.i).map_or(self.end, |&(_, p)| p)
    }

    fn next(&mut self) -> Option<(Tok, usize)> {
        let t = self.toks.get(self.i).cloned();
        if t.is_some() {
            self.i += 1;
        }
        t
    }

    fn parse_or(&mut self) -> Result<Query, QueryError> {
        let mut alts = vec![self.parse_and()?];
        while self.peek() == Some(&Tok::Or) {
            self.i += 1;
            alts.push(self.parse_and()?);
        }
        Ok(if alts.len() == 1 {
            alts.remove(0)
        } else {
            Query::Or(alts)
        })
    }

    fn parse_and(&mut self) -> Result<Query, QueryError> {
        let mut items = Vec::new();
        while let Some(t) = self.peek() {
            if matches!(t, Tok::Or | Tok::RParen) {
                break;
            }
            items.push(self.parse_unary()?);
        }
        match items.len() {
            0 => match self.peek() {
                Some(Tok::Or) => err("OR without left operand", self.pos()),
                Some(Tok::RParen) => err("empty group", self.pos()),
                _ => err("expected a search term", self.pos()),
            },
            1 => Ok(items.remove(0)),
            _ => Ok(Query::And(items)),
        }
    }

    fn parse_unary(&mut self) -> Result<Query, QueryError> {
        if self.peek() == Some(&Tok::Minus) {
            self.i += 1;
            if self.peek().is_none() {
                return err("nothing to negate after '-'", self.pos());
            }
            return Ok(Query::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_group(&mut self, open: usize) -> Result<Query, QueryError> {
        let q = self.parse_or()?;
        match self.next() {
            Some((Tok::RParen, _)) => Ok(q),
            _ => err("unbalanced parenthesis", open),
        }
    }

    fn parse_primary(&mut self) -> Result<Query, QueryError> {
        let pos = self.pos();
        match self.next() {
            Some((Tok::LParen, p)) => self.parse_group(p),
            Some((Tok::RParen, p)) => err("unbalanced parenthesis", p),
            Some((Tok::TagOp, p)) => {
                let operand = self.parse_operand(p, "tag")?;
                to_tag_query(operand, p)
            }
            Some((Tok::Op(op), p)) => {
                let operand = self.parse_operand(p, "operator")?;
                Ok(Query::Scoped(op, Box::new(operand)))
            }
            Some((Tok::Word(w), _)) => Ok(Query::Term(w)),
            Some((Tok::Phrase(s), _)) => Ok(Query::Phrase(s)),
            Some((Tok::Regex(r), _)) => Ok(Query::Regex(r)),
            Some((Tok::Property(raw), p)) => parse_property(&raw, p),
            Some((Tok::Or, p)) => err("unexpected OR", p),
            Some((Tok::Minus, p)) => err("unexpected '-'", p),
            None => err("expected a search term", pos),
        }
    }

    fn parse_operand(&mut self, op_pos: usize, what: &str) -> Result<Query, QueryError> {
        match self.peek() {
            Some(Tok::LParen) => {
                let (_, p) = self.next().unwrap_or((Tok::LParen, op_pos));
                self.parse_group(p)
            }
            Some(Tok::Word(_)) => self.parse_primary(),
            Some(Tok::Phrase(_) | Tok::Regex(_) | Tok::Minus) => self.parse_unary(),
            _ => err(format!("missing value for {what}"), op_pos),
        }
    }
}

fn to_tag_query(q: Query, pos: usize) -> Result<Query, QueryError> {
    Ok(match q {
        Query::Term(t) | Query::Phrase(t) => {
            let t = t.trim().trim_start_matches('#').to_string();
            if t.is_empty() {
                return err("empty tag", pos);
            }
            Query::Tag(t)
        }
        Query::And(v) => Query::And(
            v.into_iter()
                .map(|q| to_tag_query(q, pos))
                .collect::<Result<_, _>>()?,
        ),
        Query::Or(v) => Query::Or(
            v.into_iter()
                .map(|q| to_tag_query(q, pos))
                .collect::<Result<_, _>>()?,
        ),
        Query::Not(q) => Query::Not(Box::new(to_tag_query(*q, pos)?)),
        _ => return err("tag: expects a tag name", pos),
    })
}

fn parse_property(raw: &str, pos: usize) -> Result<Query, QueryError> {
    let mut in_quote = false;
    let mut split = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ':' if !in_quote => {
                split = Some(i);
                break;
            }
            _ => {}
        }
    }
    let (name, value) = match split {
        Some(i) => (&raw[..i], Some(&raw[i + 1..])),
        None => (raw, None),
    };
    let name = name.trim().trim_matches('"').trim();
    if name.is_empty() {
        return err("empty property name", pos);
    }
    let value = match value.map(str::trim) {
        None | Some("") => None,
        Some(v) => Some(Box::new(parse_query(v).map_err(|e| QueryError {
            message: format!("in property value: {}", e.message),
            position: pos,
        })?)),
    };
    Ok(Query::Property {
        name: name.to_string(),
        value,
    })
}

/// Parse a query. An empty (or blank) query yields `Query::And(vec![])`,
/// which matches nothing when searched.
pub fn parse_query(input: &str) -> Result<Query, QueryError> {
    let toks = tokenize(input)?;
    if toks.is_empty() {
        return Ok(Query::And(Vec::new()));
    }
    let mut p = Parser {
        toks,
        i: 0,
        end: input.len(),
    };
    let q = p.parse_or()?;
    if let Some((t, pos)) = p.next() {
        return match t {
            Tok::RParen => err("unbalanced parenthesis", pos),
            _ => err("unexpected token", pos),
        };
    }
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Query {
        Query::Term(s.into())
    }

    fn p(s: &str) -> Query {
        parse_query(s).unwrap()
    }

    #[test]
    fn words_are_anded() {
        assert_eq!(p("foo bar"), Query::And(vec![t("foo"), t("bar")]));
        assert_eq!(p("  single "), t("single"));
        assert_eq!(p(""), Query::And(vec![]));
    }

    #[test]
    fn or_has_lower_precedence() {
        assert_eq!(
            p("a b OR c"),
            Query::Or(vec![Query::And(vec![t("a"), t("b")]), t("c")])
        );
        // lowercase "or" is a plain word
        assert_eq!(p("a or b"), Query::And(vec![t("a"), t("or"), t("b")]));
    }

    #[test]
    fn groups_and_negation() {
        assert_eq!(
            p("-(a OR b) c"),
            Query::And(vec![
                Query::Not(Box::new(Query::Or(vec![t("a"), t("b")]))),
                t("c")
            ])
        );
        assert_eq!(
            p("-\"x y\""),
            Query::Not(Box::new(Query::Phrase("x y".into())))
        );
        assert_eq!(p("a - b"), Query::And(vec![t("a"), t("-"), t("b")]));
        assert_eq!(p("-a"), Query::Not(Box::new(t("a"))));
        // A lone dash is a plain word.
        assert_eq!(p("-"), t("-"));
    }

    #[test]
    fn phrases_with_escapes() {
        assert_eq!(p(r#""say \"hi\"""#), Query::Phrase("say \"hi\"".into()));
        assert_eq!(
            p(r#""(not a group)""#),
            Query::Phrase("(not a group)".into())
        );
    }

    #[test]
    fn operators() {
        assert_eq!(
            p("file:foo"),
            Query::Scoped(Operator::File, Box::new(t("foo")))
        );
        assert_eq!(
            p("path:\"a b\""),
            Query::Scoped(Operator::Path, Box::new(Query::Phrase("a b".into())))
        );
        assert_eq!(
            p("line:(x y)"),
            Query::Scoped(Operator::Line, Box::new(Query::And(vec![t("x"), t("y")])))
        );
        assert_eq!(
            p("task-todo:write"),
            Query::Scoped(Operator::TaskTodo, Box::new(t("write")))
        );
        assert_eq!(
            p("SECTION:(a)"),
            Query::Scoped(Operator::Section, Box::new(t("a")))
        );
        // Unknown operator stays a word.
        assert_eq!(p("foo:bar"), t("foo:bar"));
        assert_eq!(
            p("-file:x"),
            Query::Not(Box::new(Query::Scoped(Operator::File, Box::new(t("x")))))
        );
    }

    #[test]
    fn tags() {
        assert_eq!(p("tag:#rust"), Query::Tag("rust".into()));
        assert_eq!(p("tag:rust"), Query::Tag("rust".into()));
        assert_eq!(
            p("tag:(a OR #b)"),
            Query::Or(vec![Query::Tag("a".into()), Query::Tag("b".into())])
        );
        assert!(parse_query("tag:").is_err());
        assert!(parse_query("tag:/x/").is_err());
    }

    #[test]
    fn properties() {
        assert_eq!(
            p("[status]"),
            Query::Property {
                name: "status".into(),
                value: None
            }
        );
        assert_eq!(
            p("[status:brouillon]"),
            Query::Property {
                name: "status".into(),
                value: Some(Box::new(t("brouillon")))
            }
        );
        assert_eq!(
            p("[status:\"en cours\"]"),
            Query::Property {
                name: "status".into(),
                value: Some(Box::new(Query::Phrase("en cours".into())))
            }
        );
        assert_eq!(
            p("[s:(a OR b)]"),
            Query::Property {
                name: "s".into(),
                value: Some(Box::new(Query::Or(vec![t("a"), t("b")])))
            }
        );
        assert!(parse_query("[status").is_err());
        assert!(parse_query("[:x]").is_err());
    }

    #[test]
    fn regex() {
        assert_eq!(p("/ab+c/"), Query::Regex("ab+c".into()));
        assert_eq!(p(r"/a\/b/"), Query::Regex("a/b".into()));
        assert_eq!(
            p("path:/^Proj/"),
            Query::Scoped(Operator::Path, Box::new(Query::Regex("^Proj".into())))
        );
        // No closing slash: plain word.
        assert_eq!(p("/usr"), t("/usr"));
    }

    #[test]
    fn errors() {
        for bad in [
            "\"open", "(a b", "a)", "()", "a OR", "OR a", "file:", "line:",
        ] {
            let e = parse_query(bad);
            assert!(e.is_err(), "{bad:?} should fail, got {e:?}");
        }
        let e = parse_query("x \"open").unwrap_err();
        assert_eq!(e.position, 2);
        assert!(e.message.contains("quote"));
        let e = parse_query("(a").unwrap_err();
        assert!(e.message.contains("parenthesis"));
    }
}
