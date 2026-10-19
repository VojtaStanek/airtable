use crate::markdown::ListKind;

pub(super) struct ItemLine<'a> {
    pub(super) indent: usize,
    pub(super) kind: ListKind,
    pub(super) checked: Option<bool>,
    pub(super) text: &'a str,
}

pub(super) fn list_item(line: &str) -> Option<ItemLine<'_>> {
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

pub(super) fn heading(line: &str) -> Option<(u8, &str)> {
    let line = line.trim_start();
    let level = line.chars().take_while(|&c| c == '#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let rest = line[level..].strip_prefix(' ')?;
    Some((level as u8, rest.trim()))
}

pub(super) fn quote_line(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

pub(super) fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Matches a reference link definition `[label]: url`.
pub(super) fn reference_definition(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix('[')?;
    let (label, url) = rest.split_once("]:")?;
    let url = url.trim();
    if label.trim().is_empty() || url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    Some((label.trim(), url))
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
