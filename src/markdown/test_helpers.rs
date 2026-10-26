use crate::markdown::{parse, Block, Inline, Inlines, List, ListItem, ListKind};

pub(crate) fn text(s: &str) -> Inline {
    Inline::Text(s.to_string())
}

pub(crate) fn inlines(nodes: Vec<Inline>) -> Inlines {
    Inlines(nodes)
}

pub(crate) fn para(nodes: Vec<Inline>) -> Block {
    Block::Paragraph(inlines(nodes))
}

pub(crate) fn heading(level: u8, s: &str) -> Block {
    Block::Heading {
        level,
        content: inlines(vec![text(s)]),
    }
}

pub(crate) fn item(s: &str, children: Vec<List>) -> ListItem {
    ListItem {
        checked: None,
        content: inlines(vec![text(s)]),
        children,
    }
}

pub(crate) fn check(checked: bool, s: &str) -> ListItem {
    ListItem {
        checked: Some(checked),
        ..item(s, vec![])
    }
}

pub(crate) fn list(kind: ListKind, items: Vec<ListItem>) -> List {
    List { kind, items }
}

pub(crate) fn inline(s: &str) -> Vec<Inline> {
    match parse(s).blocks.as_slice() {
        [Block::Paragraph(content)] => content.0.clone(),
        other => panic!("expected a single paragraph, got {other:?}"),
    }
}
