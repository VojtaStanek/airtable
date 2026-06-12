use crate::Record;

pub trait AirtableTable {
    const TABLE_NAME: &'static str;
}

#[macro_export]
macro_rules! airtable_table {
    ($vis:vis $struct_name:ident(table: $table_name:literal)) => {
        $vis struct $struct_name;

        impl $crate::AirtableTable for $struct_name {
            const TABLE_NAME: &'static str = $table_name;
        }
    };
}

pub trait QueriedRecord {
    type Table: AirtableTable;

    fn queried_fields() -> Vec<&'static str>;
    fn from_record(
        record: &Record<serde_json::Value>,
    ) -> std::result::Result<Self, RecordParsingError>
    where
        Self: std::marker::Sized;
}

pub mod filters {
    use super::*;

    pub trait TableFilter {
        type Table: AirtableTable;

        fn filter_formula(&self) -> crate::formulas::Expression;
    }

    pub struct AndFilter<T>(pub T);
    pub struct OrFilter<T>(pub T);

    /// Matches records whose Airtable record id is one of the given ids
    /// (`OR(RECORD_ID() = id, …)`). An empty `ids` yields an invalid `OR()`,
    /// so callers must guard against it.
    pub struct ByRecordIds<T> {
        pub ids: Vec<String>,
        _table: std::marker::PhantomData<fn() -> T>,
    }

    impl<T> ByRecordIds<T> {
        pub fn new(ids: Vec<String>) -> Self {
            Self {
                ids,
                _table: std::marker::PhantomData,
            }
        }
    }

    impl<T: AirtableTable> TableFilter for ByRecordIds<T> {
        type Table = T;

        fn filter_formula(&self) -> crate::formulas::Expression {
            crate::formulas::Expression::FunctionCall(crate::formulas::Function::Or(
                self.ids
                    .iter()
                    .map(|id| {
                        crate::formulas::Expression::Infix(
                            crate::formulas::Operator::Equal,
                            Box::new(crate::formulas::Expression::FunctionCall(
                                crate::formulas::Function::RecordId,
                            )),
                            Box::new(crate::formulas::Expression::TextLiteral(id.clone())),
                        )
                    })
                    .collect(),
            ))
        }
    }

    macro_rules! impl_tuple_filter {
        ($($name:ident)+) => {
            impl<T: TableFilter, $($name: TableFilter<Table = T::Table>,)+> TableFilter for AndFilter<(T, $($name,)+)> {
                type Table = T::Table;

                fn filter_formula(&self) -> crate::formulas::Expression {
                    #[allow(non_snake_case)]
                    let (t, $($name,)+) = &self.0;
                    crate::formulas::Expression::FunctionCall(crate::formulas::Function::And(vec![
                        t.filter_formula(),
                        $($name.filter_formula(),)+
                    ]))
                }
            }

            impl<T: TableFilter, $($name: TableFilter<Table = T::Table>,)+> TableFilter for OrFilter<(T, $($name,)+)> {
                type Table = T::Table;

                fn filter_formula(&self) -> crate::formulas::Expression {
                    #[allow(non_snake_case)]
                    let (t, $($name,)+) = &self.0;
                    crate::formulas::Expression::FunctionCall(crate::formulas::Function::Or(vec![
                        t.filter_formula(),
                        $($name.filter_formula(),)+
                    ]))
                }
            }
        };
    }

    impl_tuple_filter!(A);
    impl_tuple_filter!(A B);
    impl_tuple_filter!(A B C);
    impl_tuple_filter!(A B C D);
    impl_tuple_filter!(A B C D E);
    impl_tuple_filter!(A B C D E F);
    impl_tuple_filter!(A B C D E F G);
    impl_tuple_filter!(A B C D E F G H);
    impl_tuple_filter!(A B C D E F G H I);
}

#[derive(Debug, thiserror::Error)]
pub enum RecordParsingError {
    #[error("Invalid field: {field_name} - {source}")]
    InvalidField {
        field_name: &'static str,
        source: serde_json::Error,
    },
    #[error("Missing field: {field_name}")]
    MissingField { field_name: &'static str },
}

/// Helper macro to define a struct representing a queried Airtable record, along with the implementation of `QueriedRecord`.
///
/// `field: "Airtable Field" Type [default=expr] => |v| transform`
///  
/// `v` is the field's raw `&serde_json::Value` (i.e. `record.fields.get("Airtable Field")`).
/// If the field is null/missing, [default=...] is used and the transform is skipped;
/// otherwise the transform runs. So transforms only see non-null values.
///
///
/// `id: ID String` pulls record.id and is excluded from the fetched field list.
///
/// Example
/// ```rust
/// use airtable::{airtable_table, airtable_queried_record};
///
/// airtable_table!(
///    pub MyTable(table: "tbl123")
/// );
///
/// airtable_queried_record! {
///     pub MyRecord(table: MyTable) {
///         name: "Name" String,
///         optional: "Optional" Option<String> [default=None],
///         transformed: "Transformed" String => |v| v.as_str().unwrap().to_uppercase(),
///         id: ID String,
///     }
/// }
/// ```
#[macro_export]
macro_rules! airtable_queried_record {
    (
        $( #[$struct_attr:meta] )*
        $vis:vis $struct_name:ident(table: $table:ty) {
            $(
                $field_vis:vis $field_name:ident : $field_source:tt $field_type:ty $([default=$default_expr:expr])?
                $(=> |$field_var:ident| $field_transform:expr)?
            ),*
            $(,)?
        }
    ) => {
        #[derive(Debug)]
        $( #[$struct_attr] )*
        $vis struct $struct_name {
            $(
                $field_vis $field_name: $field_type
            ),*
        }

        impl $crate::QueriedRecord for $struct_name {
            type Table = $table;

            fn queried_fields() -> Vec<&'static str> {
                const fn str_eq(a: &str, b: &str) -> bool {
                    let (a, b) = (a.as_bytes(), b.as_bytes());
                    if a.len() != b.len() { return false; }
                    let mut i = 0;
                    while i < a.len() {
                        if a[i] != b[i] { return false; }
                        i += 1;
                    }
                    true
                }

                /// Returns the deduplicated strings in a fixed-size array, plus the true length.
                /// The "tail" of the array beyond `len` is garbage ("").
                pub const fn const_dedup<const N: usize>(
                    input: &[&'static str],
                ) -> ([&'static str; N], usize) {
                    let mut out = [""; N];
                    let mut len = 0usize;
                    let mut i = 0;
                    while i < N {
                        let mut unique = true;
                        let mut j = 0;
                        while j < len {
                            if str_eq(out[j], input[i]) {
                                unique = false;
                                break;
                            }
                            j += 1;
                        }
                        if unique {
                            out[len] = input[i];
                            len += 1;
                        }
                        i += 1;
                    }
                    (out, len)
                }

                const FIELD_NAMES: &[&'static str] = &$crate::airtable_queried_record!(@field_names $($field_source),*);
                const N: usize = FIELD_NAMES.len();
                const DATA: ([&'static str; N], usize) = const_dedup(FIELD_NAMES);
                DATA.0[..DATA.1].to_vec()
            }

            fn from_record(
                #[allow(unused_variables)] // when there are no fields
                record: &$crate::Record<::serde_json::Value>,
            ) -> ::std::result::Result<Self, $crate::RecordParsingError> {
                let out = Self {
                    $(
                        $field_name: $crate::airtable_queried_record!(@build_record record, $field_source, $field_name, $field_type $(, transform=|$field_var| $field_transform)? $(, default=$default_expr)?)?,
                    )*
                };
                ::tracing::debug!("Parsed record {}: {:?}", record.id, out);
                Ok(out)
            }
        }
    };
    (@field_source_name ID) => { "ID" };
    (@field_source_name $field_source:literal) => { $field_source };
    (@field_names $($item:tt),*) => {
        $crate::airtable_queried_record!(@field_names_inner [$($item)*] -> [])
    };
    (@field_names_inner [ID $($rest:tt)*] -> [$($body:tt)*]) => {
        $crate::airtable_queried_record!(@field_names_inner [$($rest)*] -> [$($body)*])
    };
    (@field_names_inner [$field_source:literal $($rest:tt)*] -> [$($body:tt)*]) => {
        $crate::airtable_queried_record!(@field_names_inner [$($rest)*] -> [$($body)* $field_source])
    };
    (@field_names_inner [] -> [$($body:tt)*]) => {
        [$($body),*]
    };
    (@get_field_value $record:ident, ID) => {
        const {
            panic!("ID field is not supported at this place");
            #[expect(unreachable_code)]
            Ok(&serde_json::Value::Null)
        }
    };
    (@get_field_value $record:ident, $field_source:literal) => {
        $record.fields.get($crate::airtable_queried_record!(@field_source_name $field_source))
            // .ok_or_else(|| $crate::RecordParsingError::MissingField { field_name: $crate::airtable_queried_record!(@field_source_name $field_source) })
            .unwrap_or(&serde_json::Value::Null) // treat missing field as null, parsing will fail with InvalidField instead of MissingField
    };
    (@build_record $record:ident, ID, $field_var:ident, $field_type:ty) => {
        {
            ::std::result::Result::Ok($record.id.clone())
        }
    };
    (@get_default $field_source:tt $default_expr:expr) => {
        ::std::result::Result::Ok($default_expr)
    };
    (@get_default $field_source:tt) => {
        ::std::result::Result::Err($crate::RecordParsingError::MissingField {
            field_name: $crate::airtable_queried_record!(@field_source_name $field_source),
        })
    };
    (@parse_field $raw:ident $field_source:tt $field_type:ty | $transform_var:ident | $transform_block:expr) => {
        {
            let $transform_var: &::serde_json::Value = $raw;
            Ok({ $transform_block })
        }
    };
    (@parse_field $raw:ident $field_source:tt $field_type:ty) => {
        ::serde_json::from_value::<$field_type>($raw.clone())
            .map_err(|e| $crate::RecordParsingError::InvalidField {
                field_name: $crate::airtable_queried_record!(@field_source_name $field_source),
                source: e
            })
    };
    (@build_record $record:ident, $field_source:tt, $field_var:ident, $field_type:ty $(, transform=|$transform_var:ident| $transform_block:expr)? $(, default=$default_expr:expr)? ) => {
        {
            let raw: &::serde_json::Value = $crate::airtable_queried_record!(@get_field_value $record, $field_source);
            if raw.is_null() {
                $crate::airtable_queried_record!(@get_default $field_source $( $default_expr )?)
            } else {
                $crate::airtable_queried_record!(@parse_field raw $field_source $field_type $(|$transform_var| $transform_block)?)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        airtable_table!(
            pub SomeTable(table: "tblTable1")
        );

        airtable_queried_record! {
            SomeRecord(table: SomeTable) {
                name: "Jméno" String,
                surname: "Příjmení" String,
                pub foobar: "Foobar" (String, String) => |s| {
                    let mut parts = s.as_str().expect("should be str").splitn(2, '-');
                    let a = parts.next().unwrap_or_default().trim().to_string();
                    let b = parts.next().unwrap_or_default().trim().to_string();
                    (a, b)
                },
                half_foobar: "Foobar" String => |s| {
                    let s = s.as_str().expect("should be str");
                    s.split('-').next().unwrap_or_default().trim().to_string()
                },
                name_length: "Jméno" usize => |s| s.as_str().expect("should be str").len(),
            }
        }

        assert_eq!(
            <SomeRecord as super::QueriedRecord>::Table::TABLE_NAME,
            "tblTable1"
        );
        assert_eq!(
            SomeRecord::queried_fields(),
            vec!["Jméno", "Příjmení", "Foobar"]
        );

        let record = Record {
            id: "rec123".to_string(),
            fields: serde_json::json!({
                "Jméno": "John",
                "Příjmení": "Doe",
                "Foobar": "foo - bar"
            }),
            created_time: None,
        };

        let parsed = SomeRecord::from_record(&record).unwrap();
        assert_eq!(parsed.name, "John");
        assert_eq!(parsed.surname, "Doe");
        assert_eq!(parsed.foobar, ("foo".to_string(), "bar".to_string()));
        assert_eq!(parsed.half_foobar, "foo".to_string());
        assert_eq!(parsed.name_length, 4);
    }

    #[test]
    fn test_id() {
        airtable_table!(
            pub SomeTable(table: "tblTable1")
        );

        airtable_queried_record! {
            SomeRecord(table: SomeTable) {
                id: ID String,
            }
        }

        assert_eq!(
            <SomeRecord as super::QueriedRecord>::Table::TABLE_NAME,
            "tblTable1"
        );
        assert!(SomeRecord::queried_fields().is_empty());

        let record = Record {
            id: "rec123".to_string(),
            fields: serde_json::json!({}),
            created_time: None,
        };

        let parsed = SomeRecord::from_record(&record).unwrap();
        assert_eq!(parsed.id, "rec123");
    }

    #[test]
    fn test_optional_field() {
        airtable_table!(
            pub SomeTable(table: "tblTable1")
        );

        airtable_queried_record! {
            SomeRecord(table: SomeTable) {
                must_have: "must_have" Option<String>,
                optional: "optional" Option<String> [default=None],
            }
        }

        let record = Record {
            id: "rec123".to_string(),
            fields: serde_json::json!({
                "must_have": "I am here",
            }),
            created_time: None,
        };

        let parsed = SomeRecord::from_record(&record).unwrap();
        assert_eq!(parsed.must_have, Some("I am here".to_string()));
        assert_eq!(parsed.optional, None);

        let invalid_record = Record {
            id: "rec123".to_string(),
            fields: serde_json::json!({}),
            created_time: None,
        };

        let error = SomeRecord::from_record(&invalid_record).unwrap_err();
        assert!(matches!(
            error,
            RecordParsingError::MissingField {
                field_name: "must_have"
            }
        ));
    }
}
