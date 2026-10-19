//! Parser for the Markdown dialect used by Airtable rich text (long text) fields.
//!
//! See <https://support.airtable.com/articles/3884632156-using-markdown-in-airtable>.
//!
//! Unlike CommonMark, every line is its own block (Airtable starts a new paragraph on
//! each Enter), and blank lines are preserved as [`Block::Empty`].
//!
//! ```rust
//! use airtable::markdown::{parse, Block, Inline};
//!
//! let doc = parse("# Title\n**bold** text");
//! assert_eq!(
//!     doc.blocks[1],
//!     Block::Paragraph(
//!         vec![
//!             Inline::Bold(vec![Inline::Text("bold".to_string())].into()),
//!             Inline::Text(" text".to_string()),
//!         ]
//!         .into()
//!     )
//! );
//! ```
use std::{collections::HashMap, mem, ops::Deref};

use serde::Serialize;

/// A parsed rich text document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// A block-level element. Each line of the input produces at most one block,
/// except for lists, quotes and code blocks, which group consecutive lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Block {
    /// `# h1`, `## h2` or `### h3`.
    Heading {
        level: u8,
        content: Inlines,
    },
    Paragraph(Inlines),
    /// Consecutive lines starting with `>`.
    BlockQuote(Vec<Block>),
    List(List),
    /// Fenced with ```` ``` ````.
    CodeBlock {
        code: String,
    },
    /// A blank line.
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct List {
    pub kind: ListKind,
    pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    /// `- item` or `* item`.
    Bullet,
    /// `1. item`; `start` is the number of the first item.
    Ordered { start: u64 },
    /// `[ ] item` or `[x] item`.
    Checklist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListItem {
    /// `Some` only for checklist items.
    pub checked: Option<bool>,
    pub content: Inlines,
    /// Nested lists. There may be several when the kind of nested list changes.
    pub children: Vec<List>,
}

/// A sequence of inline nodes, the content of a heading, paragraph, list item or span.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Inlines(pub Vec<Inline>);

impl From<Vec<Inline>> for Inlines {
    fn from(nodes: Vec<Inline>) -> Self {
        Inlines(nodes)
    }
}

impl Deref for Inlines {
    type Target = [Inline];

    fn deref(&self) -> &[Inline] {
        &self.0
    }
}

impl IntoIterator for Inlines {
    type Item = Inline;
    type IntoIter = std::vec::IntoIter<Inline>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Inlines {
    type Item = &'a Inline;
    type IntoIter = std::slice::Iter<'a, Inline>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Inline {
    Text(String),
    /// `**text**` or `__text__`.
    Bold(Inlines),
    /// `*text*` or `_text_`.
    Italic(Inlines),
    /// `~~text~~`.
    Strikethrough(Inlines),
    /// `` `code` ``.
    Code(String),
    /// `[text](url)` or a reference link `[text][label]`.
    Link {
        content: Inlines,
        url: String,
    },
}

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
    /// Reference link definitions, keyed by lowercased label.
    references: HashMap<String, String>,
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
                    content: self.inlines(content),
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
                blocks.push(Block::Paragraph(self.inlines(line.trim())));
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
            content: self.inlines(item.text.trim()),
            children: Vec::new(),
        });
    }

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
        if delimiter == '_' && i > 0 && chars[i - 1].is_alphanumeric() {
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

struct ItemLine<'a> {
    indent: usize,
    kind: ListKind,
    checked: Option<bool>,
    text: &'a str,
}

fn list_item(line: &str) -> Option<ItemLine<'_>> {
    let mut indent = 0;
    let mut rest = "";
    for (idx, c) in line.char_indices() {
        match c {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => {
                rest = &line[idx..];
                break;
            }
        }
    }

    let (kind, checked, text) =
        if let Some(text) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
            match checkbox(text) {
                Some((checked, text)) => (ListKind::Checklist, Some(checked), text),
                None => (ListKind::Bullet, None, text),
            }
        } else if let Some((checked, text)) = checkbox(rest) {
            (ListKind::Checklist, Some(checked), text)
        } else {
            let digits = rest.find(|c: char| !c.is_ascii_digit())?;
            let text = rest[digits..].strip_prefix(". ")?;
            let start = rest[..digits].parse().ok()?;
            (ListKind::Ordered { start }, None, text)
        };
    Some(ItemLine {
        indent,
        kind,
        checked,
        text,
    })
}

/// Matches a `[ ]` or `[x]` checkbox followed by a space or the end of the line.
fn checkbox(text: &str) -> Option<(bool, &str)> {
    let (checked, rest) = if let Some(rest) = text.strip_prefix("[ ]") {
        (false, rest)
    } else {
        let rest = text
            .strip_prefix("[x]")
            .or_else(|| text.strip_prefix("[X]"))?;
        (true, rest)
    };
    if rest.is_empty() {
        Some((checked, rest))
    } else {
        rest.strip_prefix(' ').map(|rest| (checked, rest))
    }
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let line = line.trim_start();
    let level = line.chars().take_while(|&c| c == '#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let rest = line[level..].strip_prefix(' ')?;
    Some((level as u8, rest.trim()))
}

fn quote_line(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Matches a reference link definition `[label]: url`.
fn reference_definition(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix('[')?;
    let (label, url) = rest.split_once("]:")?;
    let url = url.trim();
    if label.trim().is_empty() || url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    Some((label.trim(), url))
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
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    fn inlines(nodes: Vec<Inline>) -> Inlines {
        Inlines(nodes)
    }

    fn para(nodes: Vec<Inline>) -> Block {
        Block::Paragraph(inlines(nodes))
    }

    fn heading(level: u8, s: &str) -> Block {
        Block::Heading {
            level,
            content: inlines(vec![text(s)]),
        }
    }

    fn item(s: &str, children: Vec<List>) -> ListItem {
        ListItem {
            checked: None,
            content: inlines(vec![text(s)]),
            children,
        }
    }

    fn check(checked: bool, s: &str) -> ListItem {
        ListItem {
            checked: Some(checked),
            ..item(s, vec![])
        }
    }

    fn list(kind: ListKind, items: Vec<ListItem>) -> List {
        List { kind, items }
    }

    fn inline(s: &str) -> Vec<Inline> {
        match parse(s).blocks.as_slice() {
            [Block::Paragraph(content)] => content.0.clone(),
            other => panic!("expected a single paragraph, got {other:?}"),
        }
    }

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
    fn checklists() {
        let doc = parse("[x] done\n[X] also\n- [ ] dash\n* [x] star\n[ ]");
        assert_eq!(
            doc.blocks,
            vec![Block::List(list(
                ListKind::Checklist,
                vec![
                    check(true, "done"),
                    check(true, "also"),
                    check(false, "dash"),
                    check(true, "star"),
                    ListItem {
                        checked: Some(false),
                        content: Inlines::default(),
                        children: vec![],
                    },
                ],
            ))]
        );
        assert_eq!(parse("[]x").blocks, vec![para(vec![text("[]x")])]);
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
    fn headings() {
        assert_eq!(
            parse("#### four\n#nospace\n# **b**").blocks,
            vec![
                para(vec![text("#### four")]),
                para(vec![text("#nospace")]),
                Block::Heading {
                    level: 1,
                    content: inlines(vec![Inline::Bold(inlines(vec![text("b")]))]),
                },
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
