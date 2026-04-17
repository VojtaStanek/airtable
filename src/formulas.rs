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
    Or(Vec<Expression>),
    Find(Box<Expression>, Box<Expression>),
    ArrayJoin(Box<Expression>),
}

impl Function {
    fn to_formula(&self) -> anyhow::Result<String> {
        match self {
            Function::RecordId => Ok("RECORD_ID()".to_string()),
            Function::And(args) => {
                if args.is_empty() {
                    return Ok("TRUE()".to_string());
                }
                let args_str = args
                    .iter()
                    .map(|arg| arg.to_formula())
                    .collect::<anyhow::Result<Vec<String>>>()?
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
                    .collect::<anyhow::Result<Vec<String>>>()?
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
    Infix(Operator, Box<Expression>, Box<Expression>),
    TextLiteral(String),
    NumberLiteral(i64),
    FieldReference(String),
    FunctionCall(Function),
}

const ALLOWED_CHARS: [char; 98] = [
    'A', 'Á', 'B', 'C', 'Č', 'D', 'Ď', 'E', 'É', 'Ě', 'F', 'G', 'H', 'I', 'Í', 'J', 'K', 'L', 'M',
    'N', 'Ň', 'O', 'Ó', 'P', 'Q', 'R', 'Ř', 'S', 'Š', 'T', 'Ť', 'U', 'Ú', 'Ů', 'V', 'W', 'X', 'Y',
    'Ý', 'Z', 'Ž', 'a', 'á', 'b', 'c', 'č', 'd', 'ď', 'e', 'é', 'ě', 'f', 'g', 'h', 'i', 'í', 'j',
    'k', 'l', 'm', 'n', 'ň', 'o', 'ó', 'p', 'q', 'r', 'ř', 's', 'š', 't', 'ť', 'u', 'ú', 'ů', 'v',
    'w', 'x', 'y', 'ý', 'z', 'ž', '-', '_', ' ', '@', '+', '.', '0', '1', '2', '3', '4', '5', '6',
    '7', '8', '9',
];

impl Expression {
    pub fn to_formula(&self) -> anyhow::Result<String> {
        match self {
            Expression::Infix(op, left, right) => Ok(format!(
                "({} {} {})",
                left.to_formula()?,
                op.to_str(),
                right.to_formula()?
            )),
            Expression::TextLiteral(s) => {
                if !s.chars().all(|c| ALLOWED_CHARS.contains(&c)) {
                    return Err(anyhow::anyhow!("Invalid text literal: {s:?}"));
                }
                Ok(format!("\"{}\"", s))
            }
            Expression::NumberLiteral(n) => Ok(n.to_string()),
            Expression::FieldReference(name) => {
                if !name.chars().all(|c| ALLOWED_CHARS.contains(&c)) {
                    return Err(anyhow::anyhow!("Invalid field name: {name:?}"));
                }
                Ok(format!("{{{}}}", name))
            }
            Expression::FunctionCall(func) => func.to_formula(),
        }
    }
}
