use crate::formulas::Expression;

pub struct ListRecords {
    pub table_name: String,
    pub view_name: Option<String>,
    /// Maximum number of records to return. For value > 100, pagination will be used.
    /// If None, all records will be returned (with pagination).
    pub max_records: Option<u32>,
    pub fields: Vec<&'static str>,
    /// Formula to filter records. `filterByFormula` parameter of Airtable API.
    pub filter_formula: Option<Expression>,
}

pub struct ListQueryParams {
    /// Maximum number of records to return. For value > 100, pagination will be used.
    /// If None, all records will be returned (with pagination).
    pub max_records: Option<u32>,
    /// Formula to filter records. `filterByFormula` parameter of Airtable API.
    pub filter_formula: Option<Expression>,
}
