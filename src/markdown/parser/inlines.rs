use std::mem;

use winnow::{
    combinator::{alt, delimited, fail, peek, preceded, repeat},
    token::{any, one_of, take_till, take_while},
    Parser, Result,
};

use super::References;
use crate::markdown::{Inline, Inlines};

/// Characters that may start something other than plain text.
const SPECIAL: [char; 6] = ['\\', '`', '[', '*', '_', '~'];

/// Parses the inline content of a single line.
pub(super) fn parse(text: &str, references: &References) -> Inlines {
    InlineParser { references }.inlines(text)
}

struct InlineParser<'a> {
    references: &'a References,
}

/// One step of inline parsing: literal text or a parsed node.
enum Piece<'a> {
    Text(&'a str),
    Node(Inline),
}

impl InlineParser<'_> {
    fn inlines<'a>(&self, text: &'a str) -> Inlines {
        repeat(0.., |input: &mut &'a str| self.piece(input))
            .fold(InlineBuilder::default, |mut out, piece| {
                match piece {
                    Piece::Text(text) => out.text.push_str(text),
                    Piece::Node(node) => out.push(node),
                }
                out
            })
            .parse(text)
            .expect("`piece` accepts any character")
            .finish()
    }

    /// Parses a node, falling back to a single literal character when nothing matches.
    fn piece<'a>(&self, input: &mut &'a str) -> Result<Piece<'a>> {
        alt((
            take_till(1.., SPECIAL).map(Piece::Text),
            escape.map(Piece::Text),
            code_span.map(|code: &str| Piece::Node(Inline::Code(code.to_string()))),
            |input: &mut &'a str| self.link(input).map(Piece::Node),
            |input: &mut &'a str| self.emphasis(input).map(Piece::Node),
            any.take().map(Piece::Text),
        ))
        .parse_next(input)
    }

    /// Parses `**bold**`, `__bold__`, `*italic*`, `_italic_` or `~~strikethrough~~`.
    fn emphasis(&self, input: &mut &str) -> Result<Inline> {
        let delimiter = peek(one_of(['*', '_', '~'])).parse_next(input)?;
        if delimiter == '~' {
            return self.delimited(input, delimiter, 2);
        }
        alt((
            move |input: &mut &str| self.delimited(input, delimiter, 2),
            move |input: &mut &str| self.delimited(input, delimiter, 1),
        ))
        .parse_next(input)
    }

    fn delimited(&self, input: &mut &str, delimiter: char, len: usize) -> Result<Inline> {
        take_while(len..=len, delimiter).parse_next(input)?;
        peek(any.verify(|&c: &char| !c.is_whitespace() && (len == 2 || c != delimiter)))
            .parse_next(input)?;
        let Some(end) = find_closer(input, delimiter, len) else {
            return fail(input);
        };
        let inner = take_bytes(input, end);
        take_while(len..=len, delimiter).parse_next(input)?;
        let inner = self.inlines(inner);
        Ok(match (delimiter, len) {
            ('~', _) => Inline::Strikethrough(inner),
            (_, 2) => Inline::Bold(inner),
            _ => Inline::Italic(inner),
        })
    }

    /// Parses `[text](url)` or a reference link `[text][label]`.
    fn link(&self, input: &mut &str) -> Result<Inline> {
        peek('[').parse_next(input)?;
        let Some(close) = find_bracket_close(input) else {
            return fail(input);
        };
        let content = &take_bytes(input, close + 1)[1..close];
        let url = alt((
            delimited('(', take_till(0.., ')'), ')')
                .map(str::trim)
                .verify(|url: &str| !url.is_empty() && !url.contains(char::is_whitespace))
                .map(str::to_string),
            delimited('[', take_till(0.., ']'), ']').verify_map(|label: &str| {
                let label = if label.is_empty() { content } else { label };
                self.references.get(&label.trim().to_lowercase()).cloned()
            }),
        ))
        .parse_next(input)?;
        Ok(Inline::Link {
            content: self.inlines(content),
            url,
        })
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

/// Consumes the first `len` bytes of `input`. Unlike [`winnow::token::take`], which
/// counts characters, this works with the byte offsets returned by `find_closer` and
/// `find_bracket_close`.
fn take_bytes<'a>(input: &mut &'a str, len: usize) -> &'a str {
    let (taken, rest) = input.split_at(len);
    *input = rest;
    taken
}

/// `\` followed by ASCII punctuation, returning the escaped character.
fn escape<'a>(input: &mut &'a str) -> Result<&'a str> {
    preceded('\\', one_of(|c: char| c.is_ascii_punctuation()).take()).parse_next(input)
}

/// `` `code` ``, returning the code.
fn code_span<'a>(input: &mut &'a str) -> Result<&'a str> {
    delimited('`', take_till(0.., '`'), '`').parse_next(input)
}

/// An escape or a code span, whose content can't close emphasis or brackets.
fn opaque<'a>(input: &mut &'a str) -> Result<&'a str> {
    alt((escape, code_span)).take().parse_next(input)
}

/// Finds the byte offset of the closing delimiter in `text`, the input following an
/// emphasis opener, skipping escaped characters and code spans.
fn find_closer(text: &str, delimiter: char, len: usize) -> Option<usize> {
    let mut rest = text;
    let mut prev = None;
    while let Some(c) = rest.chars().next() {
        let offset = text.len() - rest.len();
        if c == '\\' || c == '`' {
            let skipped = alt((opaque, any.take())).parse_next(&mut rest).ok()?;
            prev = skipped.chars().next_back();
            continue;
        }
        if c != delimiter {
            prev = Some(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let run = take_while::<_, _, ()>(1.., delimiter)
            .parse_next(&mut rest)
            .ok()?;
        let left_flanking = prev.is_some_and(|c: char| !c.is_whitespace());
        let intraword = delimiter == '_' && rest.starts_with(char::is_alphanumeric);
        if left_flanking && !intraword {
            // The delimiter is ASCII, so the run's byte length is its character count.
            if len == 2 && run.len() >= 2 {
                return Some(offset + run.len() - 2);
            }
            if len == 1 && run.len() == 1 {
                return Some(offset);
            }
        }
        prev = Some(delimiter);
    }
    None
}

/// Finds the byte offset of the `]` matching the `[` that starts `text`.
fn find_bracket_close(text: &str) -> Option<usize> {
    let mut rest = text;
    let mut depth = 0;
    while let Some(c) = rest.chars().next() {
        let offset = text.len() - rest.len();
        match c {
            '\\' | '`' => {
                alt((opaque, any.take())).parse_next(&mut rest).ok()?;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
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
    fn multibyte_content() {
        assert_eq!(
            inline("**ř**ž [č](u) `é`"),
            vec![
                Inline::Bold(inlines(vec![text("ř")])),
                text("ž "),
                Inline::Link {
                    content: inlines(vec![text("č")]),
                    url: "u".to_string(),
                },
                text(" "),
                Inline::Code("é".to_string()),
            ]
        );
    }

    #[test]
    fn long_unclosed_runs() {
        for delimiter in ["*", "_", "~", "["] {
            let input = delimiter.repeat(2_000);
            assert_eq!(inline(&input), vec![text(&input)], "{delimiter}");
        }
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
