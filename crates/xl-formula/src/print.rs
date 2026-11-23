use crate::Expr;
use std::fmt;

fn args(f: &mut fmt::Formatter<'_>, args: &[Expr]) -> fmt::Result {
    f.write_str("(")?;
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            f.write_str(",")?;
        }
        write!(f, "{a}")?;
    }
    f.write_str(")")
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Number(t) | Expr::Str(t) | Expr::Bool(t) | Expr::Error(t) => f.write_str(t),
            Expr::Ref { prefix, text, .. } | Expr::Name { prefix, text } => {
                if let Some(p) = prefix {
                    f.write_str(p)?;
                }
                f.write_str(text)
            }
            Expr::Structured { table, spec } => {
                if let Some(t) = table {
                    f.write_str(t)?;
                }
                f.write_str(spec)
            }
            Expr::Call { name, args: a } => {
                f.write_str(name)?;
                args(f, a)
            }
            Expr::Apply { callee, args: a } => {
                write!(f, "{callee}")?;
                args(f, a)
            }
            Expr::Unary { op, expr } => write!(f, "{op}{expr}"),
            Expr::Postfix { expr, op } => write!(f, "{expr}{op}"),
            Expr::Binary { op_text, lhs, rhs, .. } => write!(f, "{lhs}{op_text}{rhs}"),
            Expr::Paren(e) => write!(f, "({e})"),
            Expr::Array(rows) => {
                f.write_str("{")?;
                for (r, row) in rows.iter().enumerate() {
                    if r > 0 {
                        f.write_str(";")?;
                    }
                    for (c, e) in row.iter().enumerate() {
                        if c > 0 {
                            f.write_str(",")?;
                        }
                        write!(f, "{e}")?;
                    }
                }
                f.write_str("}")
            }
            Expr::Missing => Ok(()),
            Expr::Spaced { before, expr, after } => write!(f, "{before}{expr}{after}"),
        }
    }
}
