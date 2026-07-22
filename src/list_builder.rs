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
    /// If true, the response will use field ids instead of field names. `returnFieldsByFieldId` parameter of Airtable API.
    pub return_fields_by_id: bool,
}

pub struct GetRecord {
    pub table_name: String,
    pub record_id: String,
    /// If true, the response will use field ids instead of field names. `returnFieldsByFieldId` parameter of Airtable API.
    pub return_fields_by_id: bool,
}
