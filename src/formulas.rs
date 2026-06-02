#[derive(Copy, Clone, Debug)]
pub enum Operator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

impl Operator {
    fn to_str(self) -> &'static str {
        match self {
            Operator::Equal => "=",
            Operator::NotEqual => "!=",
            Operator::GreaterThan => ">",
            Operator::GreaterThanOrEqual => ">=",
            Operator::LessThan => "<",
            Operator::LessThanOrEqual => "<=",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Function {
    RecordId,
    And(Vec<Expression>),
    /// Logical OR of multiple expressions.
    ///
    /// ```rust
    /// use airtable::formulas::{Expression, Function, Operator};
    ///
    /// assert_eq!(
    ///     Expression::FunctionCall(Function::Or(vec![])).to_formula().unwrap(),
    ///     "FALSE()"
    /// );
    ///
    /// assert_eq!(
    ///     Expression::FunctionCall(Function::Or(vec![
    ///         Expression::Infix(Operator::Equal, Box::new(Expression::FieldReference("Status".to_string())), Box::new(Expression::TextLiteral("Pending".to_string()))),
    ///     ])).to_formula().unwrap(),
    ///     "OR(({Status} = \"Pending\"))"
    /// );
    ///
    /// assert_eq!(
    ///     Expression::FunctionCall(Function::Or(vec![
    ///         Expression::Infix(Operator::Equal, Box::new(Expression::FieldReference("Status".to_string())), Box::new(Expression::TextLiteral("Active".to_string()))),
    ///         Expression::Infix(Operator::Equal, Box::new(Expression::FieldReference("Status".to_string())), Box::new(Expression::TextLiteral("Pending".to_string()))),
    ///     ])).to_formula().unwrap(),
    ///     "OR(({Status} = \"Active\"), ({Status} = \"Pending\"))"
    /// );
    /// ```
    Or(Vec<Expression>),
    Find(Box<Expression>, Box<Expression>),
    ArrayJoin(Box<Expression>),
}

impl Function {
    fn to_formula(&self) -> crate::Result<String> {
        match self {
            Function::RecordId => Ok("RECORD_ID()".to_string()),
            Function::And(args) => {
                if args.is_empty() {
                    return Ok("TRUE()".to_string());
                }
                let args_str = args
                    .iter()
                    .map(|arg| arg.to_formula())
                    .collect::<crate::Result<Vec<String>>>()?
                    .join(", ");
                Ok(format!("AND({})", args_str))
            }
            Function::Or(args) => {
                if args.is_empty() {
                    return Ok("FALSE()".to_string());
                }
                let args_str = args
                    .iter()
                    .map(|arg| arg.to_formula())
                    .collect::<crate::Result<Vec<String>>>()?
                    .join(", ");
                Ok(format!("OR({})", args_str))
            }
            Function::Find(needle, haystack) => Ok(format!(
                "FIND({}, {})",
                needle.to_formula()?,
                haystack.to_formula()?
            )),
            Function::ArrayJoin(arg) => Ok(format!("ARRAYJOIN({})", arg.to_formula()?)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expression {
    /// Infix operator, e.g. `{Status} = "Active"` would be represented as `Infix(Equal, FieldReference("Status"), TextLiteral("Active"))`
    Infix(Operator, Box<Expression>, Box<Expression>),
    /// Text literal, e.g. `"Active"`
    TextLiteral(String),
    /// Number literal, e.g. `42`
    NumberLiteral(i64),
    /// Field reference, e.g. `{Status}`
    FieldReference(String),
    /// Function call, e.g. `RECORD_ID()`, `AND(...)`, `FIND(...)`
    FunctionCall(Function),
}

/// These are characters which are safe to use in Airtable formulas, either as field names or text literals.
///
/// This is not an exhaustive list of all characters allowed by Airtable, but it covers all characters we actually use in our field names and values.
const ALLOWED_CHARS: [char; 99] = [
    'A', 'Á', 'B', 'C', 'Č', 'D', 'Ď', 'E', 'É', 'Ě', 'F', 'G', 'H', 'I', 'Í', 'J', 'K', 'L', 'M',
    'N', 'Ň', 'O', 'Ó', 'P', 'Q', 'R', 'Ř', 'S', 'Š', 'T', 'Ť', 'U', 'Ú', 'Ů', 'V', 'W', 'X', 'Y',
    'Ý', 'Z', 'Ž', 'a', 'á', 'b', 'c', 'č', 'd', 'ď', 'e', 'é', 'ě', 'f', 'g', 'h', 'i', 'í', 'j',
    'k', 'l', 'm', 'n', 'ň', 'o', 'ó', 'p', 'q', 'r', 'ř', 's', 'š', 't', 'ť', 'u', 'ú', 'ů', 'v',
    'w', 'x', 'y', 'ý', 'z', 'ž', '-', '_', ' ', '@', '+', '.', '0', '1', '2', '3', '4', '5', '6',
    '7', '8', '9', '|',
];

impl Expression {
    /// Returns a string representation of this expression that can be used as an Airtable formula.
    ///
    /// ```rust
    /// use airtable::formulas::{Expression, Function, Operator};
    /// let formula = Expression::Infix(
    ///     Operator::Equal,
    ///     Box::new(Expression::FieldReference("Status".to_string())),
    ///     Box::new(Expression::TextLiteral("Active".to_string())),
    /// ).to_formula().unwrap();
    /// assert_eq!(formula, "({Status} = \"Active\")");
    /// ```
    pub fn to_formula(&self) -> crate::Result<String> {
        match self {
            Expression::Infix(op, left, right) => Ok(format!(
                "({} {} {})",
                left.to_formula()?,
                op.to_str(),
                right.to_formula()?
            )),
            Expression::TextLiteral(s) => {
                if !s.chars().all(|c| ALLOWED_CHARS.contains(&c)) {
                    return Err(crate::Error::InvalidFormulaValue(format!(
                        "Invalid text literal: {s:?}"
                    )));
                }
                Ok(format!("\"{}\"", s))
            }
            Expression::NumberLiteral(n) => Ok(n.to_string()),
            Expression::FieldReference(name) => {
                if !name.chars().all(|c| ALLOWED_CHARS.contains(&c)) {
                    return Err(crate::Error::InvalidFormulaValue(format!(
                        "Invalid field name: {name:?}"
                    )));
                }
                Ok(format!("{{{}}}", name))
            }
            Expression::FunctionCall(func) => func.to_formula(),
        }
    }
}
