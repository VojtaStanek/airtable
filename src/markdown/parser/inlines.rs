use std::mem;

use super::References;
use crate::markdown::{Inline, Inlines};

/// Parses the inline content of a single line.
pub(super) fn parse(text: &str, references: &References) -> Inlines {
    InlineParser { references }.inlines(text)
}

struct InlineParser<'a> {
    references: &'a References,
}

impl InlineParser<'_> {
    fn inlines(&self, text: &str) -> Inlines {
        let chars: Vec<char> = text.chars().collect();
        self.parse_inlines(&chars)
    }

    fn parse_inlines(&self, chars: &[char]) -> Inlines {
        let mut out = InlineBuilder::default();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let parsed = match c {
                '\\' if is_escapable(chars.get(i + 1)) => {
                    out.text.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                '`' => find_char(chars, i + 1, '`')
                    .map(|end| (Inline::Code(chars[i + 1..end].iter().collect()), end + 1)),
                '[' => self.link(chars, i),
                '*' | '_' | '~' => self.emphasis(chars, i),
                _ => None,
            };
            match parsed {
                Some((node, next)) => {
                    out.push(node);
                    i = next;
                }
                None => {
                    out.text.push(c);
                    i += 1;
                }
            }
        }
        out.finish()
    }

    /// Parses emphasis starting with the delimiter at `chars[i]`, returning the node
    /// and the index just past its closing delimiter.
    fn emphasis(&self, chars: &[char], i: usize) -> Option<(Inline, usize)> {
        let delimiter = chars[i];
        if chars.get(i + 1) == Some(&delimiter) {
            if let Some(parsed) = self.delimited(chars, i, 2) {
                return Some(parsed);
            }
        }
        if delimiter == '~' {
            return None;
        }
        self.delimited(chars, i, 1)
    }

    fn delimited(&self, chars: &[char], i: usize, len: usize) -> Option<(Inline, usize)> {
        let delimiter = chars[i];
        let start = i + len;
        let after = *chars.get(start)?;
        if after.is_whitespace() || (len == 1 && after == delimiter) {
            return None;
        }
        let end = find_closer(chars, start, delimiter, len)?;
        let inner = self.parse_inlines(&chars[start..end]);
        let node = match (delimiter, len) {
            ('~', _) => Inline::Strikethrough(inner),
            (_, 2) => Inline::Bold(inner),
            _ => Inline::Italic(inner),
        };
        Some((node, end + len))
    }

    /// Parses `[text](url)` or `[text][label]` starting at the `[` at `chars[i]`.
    fn link(&self, chars: &[char], i: usize) -> Option<(Inline, usize)> {
        let close = find_bracket_close(chars, i)?;
        let content = &chars[i + 1..close];
        let (url, next) = match chars.get(close + 1) {
            Some('(') => {
                let end = find_char(chars, close + 2, ')')?;
                let url: String = chars[close + 2..end].iter().collect();
                let url = url.trim();
                if url.is_empty() || url.contains(char::is_whitespace) {
                    return None;
                }
                (url.to_string(), end + 1)
            }
            Some('[') => {
                let end = find_char(chars, close + 2, ']')?;
                let label: String = if end == close + 2 {
                    content.iter().collect()
                } else {
                    chars[close + 2..end].iter().collect()
                };
                let url = self.references.get(&label.trim().to_lowercase())?;
                (url.clone(), end + 1)
            }
            _ => return None,
        };
        Some((
            Inline::Link {
                content: self.parse_inlines(content),
                url,
            },
            next,
        ))
    }
}

#[derive(Default)]
struct InlineBuilder {
    nodes: Vec<Inline>,
    text: String,
}

impl InlineBuilder {
    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.nodes.push(Inline::Text(mem::take(&mut self.text)));
        }
    }

    fn push(&mut self, node: Inline) {
        self.flush();
        self.nodes.push(node);
    }

    fn finish(mut self) -> Inlines {
        self.flush();
        Inlines(self.nodes)
    }
}

fn is_escapable(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_ascii_punctuation())
}

fn find_char(chars: &[char], from: usize, needle: char) -> Option<usize> {
    chars
        .get(from..)?
        .iter()
        .position(|&c| c == needle)
        .map(|pos| from + pos)
}

/// Finds the closing delimiter for emphasis whose content starts at `start`,
/// skipping escaped characters and code spans.
fn find_closer(chars: &[char], start: usize, delimiter: char, len: usize) -> Option<usize> {
    let mut j = start;
    while j < chars.len() {
        match chars[j] {
            '\\' if is_escapable(chars.get(j + 1)) => {
                j += 2;
                continue;
            }
            '`' => {
                if let Some(end) = find_char(chars, j + 1, '`') {
                    j = end + 1;
                    continue;
                }
            }
            c if c == delimiter => {
                let run_end = chars[j..]
                    .iter()
                    .position(|&c| c != delimiter)
                    .map_or(chars.len(), |pos| j + pos);
                let run_len = run_end - j;
                let left_flanking = j > start && !chars[j - 1].is_whitespace();
                let intraword =
                    delimiter == '_' && chars.get(run_end).is_some_and(|c| c.is_alphanumeric());
                if left_flanking && !intraword {
                    if len == 2 && run_len >= 2 {
                        return Some(run_end - 2);
                    }
                    if len == 1 && run_len == 1 {
                        return Some(j);
                    }
                }
                j = run_end;
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Finds the `]` matching the `[` at `chars[open]`.
fn find_bracket_close(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut j = open;
    while j < chars.len() {
        match chars[j] {
            '\\' if is_escapable(chars.get(j + 1)) => j += 1,
            '`' => {
                if let Some(end) = find_char(chars, j + 1, '`') {
                    j = end;
                }
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::markdown::{Inline, Inlines};

    #[test]
    fn nested_emphasis() {
        assert_eq!(
            inline("_**x**_"),
            vec![Inline::Italic(inlines(vec![Inline::Bold(inlines(vec![
                text("x")
            ]))]))]
        );
        assert_eq!(
            inline("***x***"),
            vec![Inline::Bold(inlines(vec![Inline::Italic(inlines(vec![
                text("x")
            ]))]))]
        );
        assert_eq!(
            inline("*a **b** c*"),
            vec![Inline::Italic(inlines(vec![
                text("a "),
                Inline::Bold(inlines(vec![text("b")])),
                text(" c"),
            ]))]
        );
        assert_eq!(
            inline("__bold__ and _it_"),
            vec![
                Inline::Bold(inlines(vec![text("bold")])),
                text(" and "),
                Inline::Italic(inlines(vec![text("it")])),
            ]
        );
    }

    #[test]
    fn unmatched_delimiters_stay_literal() {
        assert_eq!(inline("a * b * c"), vec![text("a * b * c")]);
        assert_eq!(inline("**open"), vec![text("**open")]);
        assert_eq!(inline("~single~"), vec![text("~single~")]);
        assert_eq!(inline("[no link]"), vec![text("[no link]")]);
        assert_eq!(inline("`unclosed"), vec![text("`unclosed")]);
    }

    #[test]
    fn escapes() {
        assert_eq!(inline(r"\# \> \- \\ \`x\`"), vec![text(r"# > - \ `x`")]);
        assert_eq!(inline(r"\a"), vec![text(r"\a")]);
        assert_eq!(
            inline(r"**a \*\* b**"),
            vec![Inline::Bold(inlines(vec![text("a ** b")]))]
        );
    }

    #[test]
    fn intraword_underscore() {
        assert_eq!(inline("snake_case_name"), vec![text("snake_case_name")]);
        assert_eq!(inline("__a__b"), vec![text("__a__b")]);
    }

    #[test]
    fn intraword_opener() {
        let formatted =
            |node: fn(Inlines) -> Inline| vec![text("foo"), node(inlines(vec![text("bar")]))];
        assert_eq!(inline("foo_bar_"), formatted(Inline::Italic));
        assert_eq!(inline("foo**bar**"), formatted(Inline::Bold));
        assert_eq!(inline("foo~~bar~~"), formatted(Inline::Strikethrough));
    }

    #[test]
    fn code_is_not_parsed() {
        assert_eq!(
            inline("`*a*` *b*"),
            vec![
                Inline::Code("*a*".to_string()),
                text(" "),
                Inline::Italic(inlines(vec![text("b")])),
            ]
        );
        assert_eq!(
            inline("*a `*` b*"),
            vec![Inline::Italic(inlines(vec![
                text("a "),
                Inline::Code("*".to_string()),
                text(" b"),
            ]))]
        );
    }

    #[test]
    fn links() {
        assert_eq!(
            inline("see [**docs**](https://a.b/c) now"),
            vec![
                text("see "),
                Inline::Link {
                    content: inlines(vec![Inline::Bold(inlines(vec![text("docs")]))]),
                    url: "https://a.b/c".to_string(),
                },
                text(" now"),
            ]
        );
    }
}
