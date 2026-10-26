//! Serializes the AST back to Airtable Markdown, so documents can be built in code and written
//! to rich text fields.
//!
//! Text is escaped so that [`parse`](super::parse) reads it back as the same text. Some trees
//! have no Markdown form and come back normalized:
//! - Line breaks inside text become spaces, and whitespace at the start or end of a line is lost.
//! - A paragraph without content comes back as [`Block::Empty`].
//! - Whitespace at the edges of an emphasis span is moved outside it.
//! - Two adjacent lists of the same kind come back as one.
//! - A code span containing `` ` `` and a code block containing a ```` ``` ```` line can't be
//!   written exactly.

use std::fmt::{self, Display, Write};

use super::{Block, Document, Inline, Inlines, List, ListKind};

/// How far a nested list is indented per level.
const LIST_INDENT: &str = "    ";

impl Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&lines(&self.blocks).join("\n"))
    }
}

impl Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&lines(std::slice::from_ref(self)).join("\n"))
    }
}

impl Display for Inlines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write_inlines(&mut out, self);
        f.write_str(&out)
    }
}

fn lines(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    for block in blocks {
        block_lines(&mut out, block);
    }
    out
}

fn block_lines(out: &mut Vec<String>, block: &Block) {
    match block {
        Block::Heading { level, content } => {
            let hashes = "#".repeat((*level).clamp(1, 3).into());
            out.push(format!("{hashes} {}", content.to_string().trim()));
        }
        Block::Paragraph(content) => out.push(escape_line_start(content.to_string().trim())),
        Block::BlockQuote(blocks) => {
            out.extend(lines(blocks).into_iter().map(|line| format!("> {line}")));
        }
        Block::List(list) => list_lines(out, list, 0),
        Block::CodeBlock { code } => {
            out.push("```".to_string());
            out.extend(code.split('\n').map(str::to_string));
            out.push("```".to_string());
        }
        Block::Empty => out.push(String::new()),
    }
}

fn list_lines(out: &mut Vec<String>, list: &List, depth: usize) {
    let indent = LIST_INDENT.repeat(depth);
    for (i, item) in list.items.iter().enumerate() {
        let marker = match list.kind {
            ListKind::Bullet => "-".to_string(),
            ListKind::Ordered { start } => format!("{}.", start + i as u64),
            ListKind::Checklist if item.checked == Some(true) => "[x]".to_string(),
            ListKind::Checklist => "[ ]".to_string(),
        };
        out.push(format!("{indent}{marker} {}", item.content.to_string().trim()));
        for child in &item.children {
            list_lines(out, child, depth + 1);
        }
    }
}

/// Escapes what would make a paragraph line parse as another block. Inline syntax is already
/// escaped, which also covers code fences, checkboxes and reference definitions.
fn escape_line_start(line: &str) -> String {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if line.starts_with(['#', '>', '-']) {
        format!("\\{line}")
    } else if digits > 0 && line[digits..].starts_with('.') {
        format!("{}\\{}", &line[..digits], &line[digits..])
    } else {
        line.to_string()
    }
}

fn write_inlines(out: &mut String, inlines: &Inlines) {
    for (i, inline) in inlines.iter().enumerate() {
        let next = inlines.get(i + 1).and_then(|next| {
            let mut rendered = String::new();
            write_inline(&mut rendered, next, None, None);
            rendered.chars().next()
        });
        let previous = out.chars().next_back();
        write_inline(out, inline, previous, next);
    }
}

/// Writes one node; `previous` and `next` are the characters written around it, which decide
/// the emphasis delimiters.
fn write_inline(out: &mut String, inline: &Inline, previous: Option<char>, next: Option<char>) {
    match inline {
        Inline::Text(text) => {
            for c in text.chars() {
                match c {
                    '\\' | '`' | '[' | ']' | '*' | '_' | '~' => {
                        out.push('\\');
                        out.push(c);
                    }
                    '\n' | '\r' => out.push(' '),
                    c => out.push(c),
                }
            }
        }
        Inline::Bold(content) => {
            let rendered = content.to_string();
            // A run of `*` next to the span would merge with its delimiter.
            let next_to_star = previous == Some('*') || next == Some('*');
            let delimiter = if next_to_star && can_use_underscores(&rendered, next) { "__" } else { "**" };
            write_span(out, delimiter, &rendered);
        }
        Inline::Italic(content) => {
            let rendered = content.to_string();
            let delimiter = if can_use_underscores(&rendered, next) { "_" } else { "*" };
            write_span(out, delimiter, &rendered);
        }
        Inline::Strikethrough(content) => write_span(out, "~~", &content.to_string()),
        Inline::Code(code) => {
            out.push('`');
            out.push_str(&code.replace(['\n', '\r'], " "));
            out.push('`');
        }
        Inline::Link { content, url } => {
            let url: String = url
                .trim()
                .chars()
                .map(|c| match c {
                    '(' => "%28".to_string(),
                    ')' => "%29".to_string(),
                    c if c.is_whitespace() => "%20".to_string(),
                    c => c.to_string(),
                })
                .collect();
            if url.is_empty() {
                write_inlines(out, content);
            } else {
                let _ = write!(out, "[{content}]({url})");
            }
        }
    }
}

/// `_` doesn't close a span when a letter or digit follows (`snake_case`), and must not touch
/// another `_` span inside.
fn can_use_underscores(rendered: &str, next: Option<char>) -> bool {
    let core = rendered.trim();
    let after = if rendered.ends_with(char::is_whitespace) { Some(' ') } else { next };
    !core.starts_with('_') && !core.ends_with('_') && !after.is_some_and(char::is_alphanumeric)
}

/// Writes `rendered` content between `delimiter`s. A span can't start or end with whitespace,
/// so that whitespace goes outside it, and an empty span is left out.
fn write_span(out: &mut String, delimiter: &str, rendered: &str) {
    let core = rendered.trim();
    let start = rendered.len() - rendered.trim_start().len();
    out.push_str(&rendered[..start]);
    if !core.is_empty() {
        out.push_str(delimiter);
        out.push_str(core);
        out.push_str(delimiter);
    }
    out.push_str(&rendered[start + core.len()..]);
}

#[cfg(test)]
mod tests {
    use crate::markdown::test_helpers::*;
    use crate::markdown::{parse, Block, Document, Inline, List, ListItem, ListKind};

    fn round_trip(blocks: Vec<Block>) {
        let doc = Document { blocks };
        let written = doc.to_string();
        assert_eq!(parse(&written), doc, "written as:\n{written}");
    }

    fn bold(nodes: Vec<Inline>) -> Inline {
        Inline::Bold(inlines(nodes))
    }

    fn italic(nodes: Vec<Inline>) -> Inline {
        Inline::Italic(inlines(nodes))
    }

    #[test]
    fn blocks_round_trip() {
        round_trip(vec![
            heading(1, "h1"),
            heading(2, "h2"),
            heading(3, "h3"),
            para(vec![text("Prostý text")]),
            Block::Empty,
            Block::BlockQuote(vec![
                para(vec![text("one")]),
                Block::Empty,
                Block::List(list(ListKind::Bullet, vec![item("item", vec![])])),
                Block::BlockQuote(vec![para(vec![text("nested")])]),
            ]),
            Block::CodeBlock {
                code: "let *x* = 1;\n\n# not heading".to_string(),
            },
            para(vec![text("after")]),
        ]);
    }

    #[test]
    fn lists_round_trip() {
        round_trip(vec![
            Block::List(list(
                ListKind::Bullet,
                vec![
                    item(
                        "a",
                        vec![
                            list(ListKind::Bullet, vec![item("b", vec![])]),
                            list(ListKind::Ordered { start: 1 }, vec![item("c", vec![])]),
                        ],
                    ),
                    item("d", vec![]),
                ],
            )),
            Block::List(list(
                ListKind::Ordered { start: 3 },
                vec![item("three", vec![]), item("four", vec![])],
            )),
            Block::List(list(
                ListKind::Checklist,
                vec![check(true, "done"), check(false, "todo")],
            )),
        ]);
    }

    #[test]
    fn inlines_round_trip() {
        let link = |nodes, url: &str| Inline::Link {
            content: inlines(nodes),
            url: url.to_string(),
        };
        round_trip(vec![
            para(vec![
                bold(vec![text("bold")]),
                text(" "),
                italic(vec![text("italic")]),
                text(" "),
                Inline::Strikethrough(inlines(vec![text("struck")])),
                text(" "),
                Inline::Code("co*de".to_string()),
            ]),
            para(vec![bold(vec![italic(vec![text("x")])])]),
            para(vec![italic(vec![bold(vec![text("x")])])]),
            para(vec![bold(vec![text("a"), italic(vec![text("b")])])]),
            para(vec![bold(vec![italic(vec![text("b")]), text("a")])]),
            para(vec![text("foo"), bold(vec![text("bar")]), text("baz")]),
            para(vec![bold(vec![text("s")]), italic(vec![text("e")])]),
            para(vec![italic(vec![text("e")]), bold(vec![text("s")])]),
            para(vec![italic(vec![text("e")]), text("x")]),
            para(vec![text("x"), italic(vec![text("e")]), text(" y")]),
            para(vec![bold(vec![text("a")]), bold(vec![text("b")])]),
            para(vec![bold(vec![text("a")]), italic(vec![text("b")]), text(" c")]),
            para(vec![
                text("see "),
                link(vec![bold(vec![text("docs")])], "https://a.b/c"),
                text(" now"),
            ]),
        ]);
    }

    #[test]
    fn escapes_syntax_in_text() {
        round_trip(vec![
            para(vec![text(r"a *b* _c_ __d__ ~~e~~ `f` [g](h) \ i")]),
            para(vec![text("# not heading")]),
            para(vec![text("> not quote")]),
            para(vec![text("- not list")]),
            para(vec![text("* not list")]),
            para(vec![text("12. not list")]),
            para(vec![text("[ ] not checkbox")]),
            para(vec![text("[a]: not-a-definition")]),
            para(vec![text("```")]),
            para(vec![text("snake_case")]),
            heading(2, "# hashes"),
            Block::List(list(
                ListKind::Bullet,
                vec![ListItem {
                    checked: None,
                    content: inlines(vec![text("[x] not checked")]),
                    children: vec![],
                }],
            )),
        ]);
    }

    #[test]
    fn normalizes_what_markdown_cannot_express() {
        let doc = Document {
            blocks: vec![
                para(vec![
                    bold(vec![text("Zdroj: ")]),
                    text("x"),
                    italic(vec![]),
                ]),
                para(vec![text("a\nb")]),
                Block::Paragraph(inlines(vec![])),
            ],
        };
        assert_eq!(doc.to_string(), "**Zdroj:** x\na b\n");
        assert_eq!(
            Document {
                blocks: vec![para(vec![Inline::Link {
                    content: inlines(vec![text("l")]),
                    url: "https://a.b/x (1)".to_string(),
                }])]
            }
            .to_string(),
            "[l](https://a.b/x%20%281%29)"
        );
    }

    #[test]
    fn display_of_parts() {
        assert_eq!(heading(4, "x").to_string(), "### x");
        assert_eq!(inlines(vec![bold(vec![text("b")])]).to_string(), "**b**");
        assert_eq!(
            Block::List(List {
                kind: ListKind::Bullet,
                items: vec![item("a", vec![list(ListKind::Bullet, vec![item("b", vec![])])])],
            })
            .to_string(),
            "- a\n    - b"
        );
    }
}
