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
use std::ops::Deref;

use serde::Serialize;

mod parser;
#[cfg(test)]
mod test_helpers;
mod writer;

pub use parser::parse;

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
