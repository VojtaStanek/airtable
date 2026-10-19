use winnow::{
    ascii::digit1,
    combinator::{alt, eof, opt, preceded, repeat, terminated},
    error::ContextError,
    token::{rest, take_until, take_while},
    Parser, Result,
};

use crate::markdown::ListKind;

pub(super) struct ItemLine<'a> {
    pub(super) indent: usize,
    pub(super) kind: ListKind,
    pub(super) checked: Option<bool>,
    pub(super) text: &'a str,
}

pub(super) fn list_item(line: &str) -> Option<ItemLine<'_>> {
    (indent, marker, rest)
        .map(|(indent, (kind, checked), text)| ItemLine {
            indent,
            kind,
            checked,
            text,
        })
        .parse(line)
        .ok()
}

/// Leading spaces and tabs, a tab counting as four spaces.
fn indent(input: &mut &str) -> Result<usize> {
    repeat(0.., alt((' '.value(1), '\t'.value(4))))
        .fold(|| 0, |total, width| total + width)
        .parse_next(input)
}

/// A list marker, returning the list kind and, for checklists, whether the item is checked.
fn marker(input: &mut &str) -> Result<(ListKind, Option<bool>)> {
    alt((
        preceded(alt(("- ", "* ")), opt(checkbox)).map(|checked| match checked {
            Some(checked) => (ListKind::Checklist, Some(checked)),
            None => (ListKind::Bullet, None),
        }),
        checkbox.map(|checked| (ListKind::Checklist, Some(checked))),
        terminated(digit1.try_map(str::parse), ". ")
            .map(|start| (ListKind::Ordered { start }, None)),
    ))
    .parse_next(input)
}

/// Matches a `[ ]` or `[x]` checkbox followed by a space or the end of the line.
fn checkbox(input: &mut &str) -> Result<bool> {
    terminated(
        alt(("[ ]".value(false), "[x]".value(true), "[X]".value(true))),
        alt((" ", eof)),
    )
    .parse_next(input)
}

pub(super) fn heading(line: &str) -> Option<(u8, &str)> {
    preceded(ws, (take_while(1..=3, '#'), ' ', rest))
        .map(|(hashes, _, text): (&str, _, &str)| (hashes.len() as u8, text.trim()))
        .parse(line)
        .ok()
}

pub(super) fn quote_line(line: &str) -> Option<&str> {
    preceded((ws, '>', opt(' ')), rest).parse(line).ok()
}

pub(super) fn is_fence(line: &str) -> bool {
    preceded((ws, "```"), rest).parse(line).is_ok()
}

/// Matches a reference link definition `[label]: url`.
pub(super) fn reference_definition(line: &str) -> Option<(&str, &str)> {
    (
        preceded('[', take_until(0.., "]:")),
        preceded("]:", rest::<_, ContextError>),
    )
        .map(|(label, url): (&str, &str)| (label.trim(), url.trim()))
        .verify(|(label, url): &(&str, &str)| {
            !label.is_empty() && !url.is_empty() && !url.contains(char::is_whitespace)
        })
        .parse(line.trim())
        .ok()
}

/// Leading whitespace, like [`str::trim_start`].
fn ws<'a>(input: &mut &'a str) -> Result<&'a str> {
    take_while(0.., char::is_whitespace).parse_next(input)
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::markdown::{parse, Block, Inline, Inlines, ListItem, ListKind};

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
}
