use std::{collections::HashMap, mem};

use super::{Block, Document, List, ListItem, ListKind};
use lines::{heading, is_fence, list_item, quote_line, reference_definition, ItemLine};

mod inlines;
mod lines;

/// Reference link definitions, keyed by lowercased label.
type References = HashMap<String, String>;

/// Parses Airtable Markdown into a [`Document`]. Never fails: syntax that can't be
/// matched is kept as literal text.
pub fn parse(input: &str) -> Document {
    let input = input.replace("\r\n", "\n");
    let lines: Vec<&str> = input.split('\n').collect();
    let references = lines
        .iter()
        .filter_map(|line| reference_definition(line))
        .map(|(label, url)| (label.to_lowercase(), url.to_string()))
        .collect();
    let parser = Parser { references };
    Document {
        blocks: parser.blocks(&lines),
    }
}

struct Parser {
    references: References,
}

impl Parser {
    fn blocks(&self, lines: &[&str]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut lists: Vec<(usize, List)> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            if let Some(item) = list_item(line) {
                self.push_list_item(&mut lists, &mut blocks, item);
                i += 1;
                continue;
            }
            while !lists.is_empty() {
                close_list(&mut lists, &mut blocks);
            }

            if is_fence(line) {
                let mut code = Vec::new();
                i += 1;
                while i < lines.len() && !is_fence(lines[i]) {
                    code.push(lines[i]);
                    i += 1;
                }
                blocks.push(Block::CodeBlock {
                    code: code.join("\n"),
                });
            } else if line.trim().is_empty() {
                blocks.push(Block::Empty);
            } else if reference_definition(line).is_some() {
                // Definitions were collected up front and produce no output.
            } else if let Some((level, content)) = heading(line) {
                blocks.push(Block::Heading {
                    level,
                    content: inlines::parse(content, &self.references),
                });
            } else if quote_line(line).is_some() {
                let mut quoted = Vec::new();
                while let Some(rest) = lines.get(i).and_then(|line| quote_line(line)) {
                    quoted.push(rest);
                    i += 1;
                }
                blocks.push(Block::BlockQuote(self.blocks(&quoted)));
                continue;
            } else {
                blocks.push(Block::Paragraph(inlines::parse(
                    line.trim(),
                    &self.references,
                )));
            }
            i += 1;
        }
        while !lists.is_empty() {
            close_list(&mut lists, &mut blocks);
        }
        blocks
    }

    fn push_list_item(
        &self,
        lists: &mut Vec<(usize, List)>,
        blocks: &mut Vec<Block>,
        item: ItemLine,
    ) {
        loop {
            match lists.last() {
                Some((indent, _)) if item.indent < *indent => close_list(lists, blocks),
                Some((indent, list))
                    if item.indent == *indent
                        && mem::discriminant(&list.kind) != mem::discriminant(&item.kind) =>
                {
                    close_list(lists, blocks);
                    lists.push((item.indent, List::new(item.kind)));
                    break;
                }
                Some((indent, _)) if item.indent == *indent => break,
                // Deeper indentation (or no open list) starts a new list.
                _ => {
                    lists.push((item.indent, List::new(item.kind)));
                    break;
                }
            }
        }
        let (_, list) = lists.last_mut().expect("a list was just ensured");
        list.items.push(ListItem {
            checked: item.checked,
            content: inlines::parse(item.text.trim(), &self.references),
            children: Vec::new(),
        });
    }
}

impl List {
    fn new(kind: ListKind) -> Self {
        List {
            kind,
            items: Vec::new(),
        }
    }
}

/// Pops the innermost open list and attaches it to its parent item, or to `blocks`
/// when it is a top-level list.
fn close_list(lists: &mut Vec<(usize, List)>, blocks: &mut Vec<Block>) {
    let Some((_, list)) = lists.pop() else {
        return;
    };
    match lists
        .last_mut()
        .and_then(|(_, parent)| parent.items.last_mut())
    {
        Some(parent) => parent.children.push(list),
        None => blocks.push(Block::List(list)),
    }
}

#[cfg(test)]
mod test_helpers {
    use crate::markdown::{parse, Block, Inline, Inlines, List, ListItem, ListKind};

    pub(super) fn text(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    pub(super) fn inlines(nodes: Vec<Inline>) -> Inlines {
        Inlines(nodes)
    }

    pub(super) fn para(nodes: Vec<Inline>) -> Block {
        Block::Paragraph(inlines(nodes))
    }

    pub(super) fn heading(level: u8, s: &str) -> Block {
        Block::Heading {
            level,
            content: inlines(vec![text(s)]),
        }
    }

    pub(super) fn item(s: &str, children: Vec<List>) -> ListItem {
        ListItem {
            checked: None,
            content: inlines(vec![text(s)]),
            children,
        }
    }

    pub(super) fn check(checked: bool, s: &str) -> ListItem {
        ListItem {
            checked: Some(checked),
            ..item(s, vec![])
        }
    }

    pub(super) fn list(kind: ListKind, items: Vec<ListItem>) -> List {
        List { kind, items }
    }

    pub(super) fn inline(s: &str) -> Vec<Inline> {
        match parse(s).blocks.as_slice() {
            [Block::Paragraph(content)] => content.0.clone(),
            other => panic!("expected a single paragraph, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use crate::markdown::{parse, Block, Document, Inline, ListItem, ListKind};

    #[test]
    fn full_example() {
        let input = "**Test**\n**_vfdg_**\n~~přeškrutnuté~~\n`code`\n\n[odkaz](https://example.com)\nProstý text\nDalší \\*prostý\\* text\nDalší ~prostý~ text\nDalší \\*\\*prostý\\*\\* text\nDalší \\[prostý]\\(text)\nDalší \\_\\_prostý\\_\\_text\n\n> Citace\n# h1\n## h2\n### h3\n\n- seznam\n    - zanořený\n- seznam\n1. první\n    1. zanořený\n2. druhý\n    - jinak zanořený\n[ ] checkbox\n[ ] checkbox";

        let expected = Document {
            blocks: vec![
                para(vec![Inline::Bold(inlines(vec![text("Test")]))]),
                para(vec![Inline::Bold(inlines(vec![Inline::Italic(inlines(
                    vec![text("vfdg")],
                ))]))]),
                para(vec![Inline::Strikethrough(inlines(vec![text(
                    "přeškrutnuté",
                )]))]),
                para(vec![Inline::Code("code".to_string())]),
                Block::Empty,
                para(vec![Inline::Link {
                    content: inlines(vec![text("odkaz")]),
                    url: "https://example.com".to_string(),
                }]),
                para(vec![text("Prostý text")]),
                para(vec![text("Další *prostý* text")]),
                para(vec![text("Další ~prostý~ text")]),
                para(vec![text("Další **prostý** text")]),
                para(vec![text("Další [prostý](text)")]),
                para(vec![text("Další __prostý__text")]),
                Block::Empty,
                Block::BlockQuote(vec![para(vec![text("Citace")])]),
                heading(1, "h1"),
                heading(2, "h2"),
                heading(3, "h3"),
                Block::Empty,
                Block::List(list(
                    ListKind::Bullet,
                    vec![
                        item(
                            "seznam",
                            vec![list(ListKind::Bullet, vec![item("zanořený", vec![])])],
                        ),
                        item("seznam", vec![]),
                    ],
                )),
                Block::List(list(
                    ListKind::Ordered { start: 1 },
                    vec![
                        item(
                            "první",
                            vec![list(
                                ListKind::Ordered { start: 1 },
                                vec![item("zanořený", vec![])],
                            )],
                        ),
                        item(
                            "druhý",
                            vec![list(ListKind::Bullet, vec![item("jinak zanořený", vec![])])],
                        ),
                    ],
                )),
                Block::List(list(
                    ListKind::Checklist,
                    vec![check(false, "checkbox"), check(false, "checkbox")],
                )),
            ],
        };

        assert_eq!(parse(input), expected);
    }

    #[test]
    fn code_example() {
        let input = "`Toto `_`je`_` `**`kód`**\n\n**`Toto je něco jiného`**\n\nCode\n```\nNový **kód**\nřádek\n```";
        let code = |s: &str| Inline::Code(s.to_string());

        let expected = Document {
            blocks: vec![
                para(vec![
                    code("Toto "),
                    Inline::Italic(inlines(vec![code("je")])),
                    code(" "),
                    Inline::Bold(inlines(vec![code("kód")])),
                ]),
                Block::Empty,
                para(vec![Inline::Bold(inlines(vec![code(
                    "Toto je něco jiného",
                )]))]),
                Block::Empty,
                para(vec![text("Code")]),
                Block::CodeBlock {
                    code: "Nový **kód**\nřádek".to_string(),
                },
            ],
        };

        assert_eq!(parse(input), expected);
    }

    #[test]
    fn reference_links() {
        let doc = parse("[one][A] and [a][] and [x][missing]\n[a]: https://example.com");
        let link = |s: &str| Inline::Link {
            content: inlines(vec![text(s)]),
            url: "https://example.com".to_string(),
        };
        assert_eq!(
            doc.blocks,
            vec![para(vec![
                link("one"),
                text(" and "),
                link("a"),
                text(" and [x][missing]"),
            ])]
        );
    }

    #[test]
    fn tab_indentation_and_deep_nesting() {
        let doc = parse("- a\n\t- b\n\t\t- c\n- d\n3. e");
        assert_eq!(
            doc.blocks,
            vec![
                Block::List(list(
                    ListKind::Bullet,
                    vec![
                        item(
                            "a",
                            vec![list(
                                ListKind::Bullet,
                                vec![item(
                                    "b",
                                    vec![list(ListKind::Bullet, vec![item("c", vec![])])],
                                )],
                            )],
                        ),
                        item("d", vec![]),
                    ],
                )),
                Block::List(list(
                    ListKind::Ordered { start: 3 },
                    vec![item("e", vec![])]
                )),
            ]
        );
    }

    #[test]
    fn list_items_have_inline_content() {
        let doc = parse("* **bold** item");
        assert_eq!(
            doc.blocks,
            vec![Block::List(list(
                ListKind::Bullet,
                vec![ListItem {
                    checked: None,
                    content: inlines(vec![
                        Inline::Bold(inlines(vec![text("bold")])),
                        text(" item"),
                    ]),
                    children: vec![],
                }],
            ))]
        );
    }

    #[test]
    fn code_block() {
        let doc = parse("before\n```\nlet *x* = 1;\n\n# not heading\n```\nafter");
        assert_eq!(
            doc.blocks,
            vec![
                para(vec![text("before")]),
                Block::CodeBlock {
                    code: "let *x* = 1;\n\n# not heading".to_string()
                },
                para(vec![text("after")]),
            ]
        );
        assert_eq!(
            parse("```\nunterminated").blocks,
            vec![Block::CodeBlock {
                code: "unterminated".to_string()
            }]
        );
    }

    #[test]
    fn quotes_merge() {
        let doc = parse("> one\n>two\n> - item\n\n> other");
        assert_eq!(
            doc.blocks,
            vec![
                Block::BlockQuote(vec![
                    para(vec![text("one")]),
                    para(vec![text("two")]),
                    Block::List(list(ListKind::Bullet, vec![item("item", vec![])])),
                ]),
                Block::Empty,
                Block::BlockQuote(vec![para(vec![text("other")])]),
            ]
        );
    }

    #[test]
    fn crlf() {
        assert_eq!(
            parse("a\r\n\r\nb").blocks,
            vec![para(vec![text("a")]), Block::Empty, para(vec![text("b")])]
        );
    }

    #[test]
    fn serializes_to_json() {
        let json = serde_json::to_value(parse("**a**")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "blocks": [{ "paragraph": [{ "bold": [{ "text": "a" }] }] }] })
        );
    }
}
