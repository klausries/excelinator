//! xl-formula: lossless parser and printer for spreadsheet formulas as stored in
//! SpreadsheetML (function names in English, comma as argument separator).
//!
//! The tree keeps every character of the source: whitespace, spelling of numbers,
//! case of names. `parse(text)?.to_string() == text` holds for every formula the
//! parser accepts. Semantics (values, reference resolution) are built on top.
//!
//! Operator precedence, highest first (as in Excel):
//!   range `:`  >  intersection (space)  >  union `,`  >  unary `-` `+` `@`  >  `%`
//!   >  `^`  >  `*` `/`  >  `+` `-`  >  `&`  >  `=` `<>` `<` `>` `<=` `>=`
//! All binary operators associate to the left, including `^`.

mod parser;
mod print;
mod refs;

pub use parser::{parse, ParseError};
pub use refs::{col_name, parse_cell_ref, shift_ref, to_r1c1, CellRef, MAX_COL, MAX_ROW};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Range,
    Intersect,
    Union,
    Pow,
    Mul,
    Div,
    Add,
    Sub,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `A1`, `$B$2`
    Cell,
    /// `A:C`, `$A:$A`
    Columns,
    /// `1:3`, `$1:$1`
    Rows,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Number literal exactly as written, e.g. `1.50`, `1E+3`.
    Number(String),
    /// String literal including its quotes and doubled inner quotes.
    Str(String),
    /// `TRUE` or `FALSE` in the case written.
    Bool(String),
    /// Error literal such as `#N/A` or `#REF!`.
    Error(String),
    /// Cell or whole row/column reference. `prefix` is the sheet or workbook
    /// qualifier including the `!`, e.g. `Sheet1!`, `'My Sheet'!`, `[1]Sheet1!`.
    Ref { prefix: Option<String>, kind: RefKind, text: String },
    /// Defined name, optionally qualified.
    Name { prefix: Option<String>, text: String },
    /// Structured reference: optional table name plus the bracket part, e.g.
    /// `Table1[[#This Row],[Amount]]`, `[@Amount]`.
    Structured { table: Option<String>, spec: String },
    /// Function call. Empty arguments are `Expr::Missing`.
    Call { name: String, args: Vec<Expr> },
    /// Call of a computed function value: `LAMBDA(x,x+1)(5)`.
    Apply { callee: Box<Expr>, args: Vec<Expr> },
    /// Prefix operator `-`, `+` or `@`.
    Unary { op: char, expr: Box<Expr> },
    /// Postfix operator `%` or `#` (spill range).
    Postfix { expr: Box<Expr>, op: char },
    /// Binary operation. `op_text` is the operator as written; for an intersection
    /// it is the whitespace between the operands.
    Binary { op: BinaryOp, op_text: String, lhs: Box<Expr>, rhs: Box<Expr> },
    Paren(Box<Expr>),
    /// Array constant `{1,2;3,4}`: rows of elements.
    Array(Vec<Vec<Expr>>),
    /// An omitted function argument.
    Missing,
    /// Whitespace around an expression that carries no meaning.
    Spaced { before: String, expr: Box<Expr>, after: String },
}

impl Expr {
    /// The expression without insignificant whitespace wrappers.
    pub fn unspaced(&self) -> &Expr {
        match self {
            Expr::Spaced { expr, .. } => expr.unspaced(),
            other => other,
        }
    }

    /// Visit this node and all children, parents first.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Call { args, .. } => args.iter().for_each(|a| a.walk(f)),
            Expr::Apply { callee, args } => {
                callee.walk(f);
                args.iter().for_each(|a| a.walk(f));
            }
            Expr::Unary { expr, .. } | Expr::Postfix { expr, .. } | Expr::Paren(expr) | Expr::Spaced { expr, .. } => expr.walk(f),
            Expr::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            Expr::Array(rows) => rows.iter().flatten().for_each(|e| e.walk(f)),
            _ => {}
        }
    }

    /// Names of all functions called, upper case, in order of first appearance.
    pub fn functions(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.walk(&mut |e| {
            if let Expr::Call { name, .. } = e {
                let n = name.to_ascii_uppercase();
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        });
        out
    }
}
