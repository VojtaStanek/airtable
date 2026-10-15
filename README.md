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
