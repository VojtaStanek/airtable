use std::{collections::HashMap, mem};

use winnow::{
    combinator::{alt, eof, preceded, repeat, repeat_till},
    error::ContextError,
    token::any,
    Parser, Result,
};

use super::{Block, Document, Inlines, List, ListItem};
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
    let parser = BlockParser { references };
    Document {
        blocks: parser.blocks(&lines, 0),
    }
}

/// Parses blocks from a stream of lines.
struct BlockParser {
    references: References,
}

/// How deep quotes and lists may nest. Each level recurses, so this bounds stack
/// usage on hostile input. Deeper `>` markers are kept as literal text and deeper list
/// items become sibling lists at the deepest level.
const MAX_NESTING: usize = 64;

impl BlockParser {
    /// Parses `lines` nested inside `depth` quotes and lists.
    fn blocks(&self, lines: &[&str], depth: usize) -> Vec<Block> {
        repeat(0.., |input: &mut &[&str]| self.block(input, depth))
            .fold(Vec::new, |mut blocks, block| {
                blocks.extend(block);
                blocks
            })
            .parse(lines)
            .expect("a paragraph accepts any line")
    }

    /// Parses one block, or `None` for lines that produce no output.
    fn block(&self, input: &mut &[&str], depth: usize) -> Result<Option<Block>> {
        let can_nest = depth < MAX_NESTING;
        alt((
            (|input: &mut &[&str]| self.list(input, None, depth))
                .map(|list| Some(Block::List(list))),
            code_block.map(Some),
            line(|line| line.trim().is_empty().then_some(Some(Block::Empty))),
            // Definitions were collected up front and produce no output.
            line(reference_definition).value(None),
            line(heading).map(|(level, content)| {
                Some(Block::Heading {
                    level,
                    content: self.inlines(content),
                })
            }),
            repeat(1.., line(move |line| quote_line(line).filter(|_| can_nest)))
                .map(|quoted: Vec<&str>| Some(Block::BlockQuote(self.blocks(&quoted, depth + 1)))),
            any.map(|line: &str| Some(Block::Paragraph(self.inlines(line.trim())))),
        ))
        .parse_next(input)
    }

    /// Parses a list whose items are indented deeper than `parent`, the indentation of
    /// the item it is nested in.
    fn list<'a>(
        &self,
        input: &mut &[&'a str],
        parent: Option<usize>,
        depth: usize,
    ) -> Result<List> {
        let first = line(list_item)
            .verify(|item: &ItemLine| {
                depth < MAX_NESTING && parent.is_none_or(|parent| item.indent > parent)
            })
            .parse_next(input)?;
        let (indent, kind) = (first.indent, first.kind);
        let mut items = vec![self.item(input, first, depth)?];
        let siblings: Vec<ListItem> = repeat(0.., |input: &mut &[&'a str]| {
            let item = line(list_item)
                .verify(|item: &ItemLine| {
                    item.indent == indent
                        && mem::discriminant(&item.kind) == mem::discriminant(&kind)
                })
                .parse_next(input)?;
            self.item(input, item, depth)
        })
        .parse_next(input)?;
        items.extend(siblings);
        Ok(List { kind, items })
    }

    /// Builds an item of a list nested `depth` levels deep from its line and the lists
    /// nested under it.
    fn item<'a>(
        &self,
        input: &mut &[&'a str],
        line: ItemLine<'a>,
        depth: usize,
    ) -> Result<ListItem> {
        let children = repeat(0.., |input: &mut &[&'a str]| {
            self.list(input, Some(line.indent), depth + 1)
        })
        .parse_next(input)?;
        Ok(ListItem {
            checked: line.checked,
            content: self.inlines(line.text.trim()),
            children,
        })
    }

    fn inlines(&self, text: &str) -> Inlines {
        inlines::parse(text, &self.references)
    }
}

/// Lines between ```` ``` ```` fences. An unterminated fence runs to the end of the input.
fn code_block(input: &mut &[&str]) -> Result<Block> {
    let fence = || line(|line| is_fence(line).then_some(()));
    preceded(fence(), repeat_till(0.., any, alt((fence(), eof.void()))))
        .map(|(code, ()): (Vec<&str>, ())| Block::CodeBlock {
            code: code.join("\n"),
        })
        .parse_next(input)
}

/// Matches one line for which `f` returns `Some`.
fn line<'i, 'a: 'i, O>(
    f: impl Fn(&'a str) -> Option<O>,
) -> impl Parser<&'i [&'a str], O, ContextError> {
    any.verify_map(f)
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
    use std::mem;

    use super::{test_helpers::*, MAX_NESTING};
    use crate::markdown::{parse, Block, Document, Inline, List, ListItem, ListKind};

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
    fn item_between_indents_nests_under_parent() {
        let bullets = |s: &str| list(ListKind::Bullet, vec![item(s, vec![])]);
        assert_eq!(
            parse("- a\n    - b\n  - c").blocks,
            vec![Block::List(list(
                ListKind::Bullet,
                vec![item("a", vec![bullets("b"), bullets("c")])],
            ))]
        );
    }

    #[test]
    fn nested_kind_change_starts_sibling_list() {
        assert_eq!(
            parse("- a\n  - b\n  1. c\n- d").blocks,
            vec![Block::List(list(
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
            ))]
        );
    }

    #[test]
    fn nesting_is_capped() {
        // Far deeper than the stack would allow without the cap.
        const DEPTH: usize = 10_000;
        // Indentation makes nested list input quadratic in size, so use fewer levels.
        const LIST_DEPTH: usize = 1_000;

        let input: Vec<String> = (0..LIST_DEPTH)
            .map(|i| format!("{}- x", " ".repeat(i)))
            .collect();
        let mut level: Vec<List> = parse(&input.join("\n"))
            .blocks
            .into_iter()
            .map(|block| match block {
                Block::List(list) => list,
                other => panic!("expected a list, got {other:?}"),
            })
            .collect();
        let mut widths = Vec::new();
        while !level.is_empty() {
            widths.push(level.len());
            level = level
                .into_iter()
                .flat_map(|list| list.items)
                .flat_map(|item| item.children)
                .collect();
        }
        assert_eq!(widths.len(), MAX_NESTING);
        assert!(widths[..MAX_NESTING - 1].iter().all(|&width| width == 1));
        // Items that don't fit become sibling lists at the deepest level, one each.
        assert_eq!(widths[MAX_NESTING - 1], LIST_DEPTH - MAX_NESTING + 1);

        let mut blocks = parse(&format!("{}x", "> ".repeat(DEPTH))).blocks;
        let mut depth = 0;
        while let [Block::BlockQuote(inner)] = blocks.as_mut_slice() {
            depth += 1;
            blocks = mem::take(inner);
        }
        assert_eq!(depth, MAX_NESTING);
        let rest = format!("{}x", "> ".repeat(DEPTH - MAX_NESTING));
        assert_eq!(blocks, vec![para(vec![text(rest.trim())])]);
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
