# Airtable API Client for Rust

This is an unofficial Airtable API client for Rust. It is evolved from a [version by Oxide Computer](https://github.com/oxidecomputer/cio/tree/master/airtable) (now archived repository).

It aims to be type-safe and ergonomic - you define your tables and queries as Rust types.

Example usage:

```rust
airtable::airtable_table! {
    pub(crate) LocationsTable(table: "tblXY1234567890")
}

airtable::airtable_queried_record! {
    pub(crate) LocationQuery(table: LocationsTable) {
        pub(crate) id: ID String,
        pub(crate) name: "Name" Option<String> [default=None]
            => |v| v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        pub(crate) map_url: "Map URL" Option<String> [default=None],
        pub(crate) airtable_url: "Airtable URL" Option<String> [default=None],
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Airtable::new_without_enterprise_id("your_api_key", "your_base_id");
    let records: Vec<LocationQuery> = client
        .list_filtered_querieable_records(
            ByRecordIds::<LocationsTable>::new(vec!["recXXXXXXXXXXXXXX".to_string()]),
            None,
        )
        .await?;

    for record in records {
        if let Some(name) = &record.name {
            println!("- {name}");
        }
    }

    Ok(())
}
```

## Rich text (Markdown)

Long text fields with rich text formatting enabled use [Airtable's Markdown dialect](https://support.airtable.com/articles/3884632156-using-markdown-in-airtable). `airtable::markdown::parse` turns that text into an AST: headings, paragraphs, quotes, nested bullet, ordered and checklist lists, code blocks, bold, italic, strikethrough, inline code and links. Every line is its own block, and blank lines are kept as `Block::Empty`. Parsing never fails; anything that isn't valid syntax stays as literal text. The AST implements `serde::Serialize`, and `Display` writes it back as Airtable Markdown (escaping text so that `parse` reads it back unchanged), so documents built in code can be written to rich text fields.

```rust
use airtable::markdown::{parse, Block};

let doc = parse("# Title\n- **bold** item\n[x] done");
assert!(matches!(doc.blocks[0], Block::Heading { level: 1, .. }));
```
