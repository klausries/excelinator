use crate::{BinaryOp, Expr, RefKind};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset in the formula text.
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.message, self.pos)
    }
}

impl std::error::Error for ParseError {}

/// Parse a formula as stored in a file (no leading `=`).
pub fn parse(text: &str) -> Result<Expr, ParseError> {
    let mut p = Parser { s: text, pos: 0, depth: 0 };
    let e = p.expr(0, true)?;
    if p.pos < text.len() {
        return Err(p.err("unexpected character"));
    }
    Ok(e)
}

const MAX_DEPTH: usize = 200;
const MAX_COL: u32 = 16_384; // XFD
const MAX_ROW: u64 = 1_048_576;

/// Error literals, longest first so that the longest match wins.
const ERRORS: &[&str] = &[
    "#GETTING_DATA", "#BLOCKED!", "#CONNECT!", "#UNKNOWN!", "#PYTHON!", "#DIV/0!", "#VALUE!", "#SPILL!", "#FIELD!",
    "#NULL!", "#NAME?", "#CALC!", "#BUSY!", "#REF!", "#NUM!", "#N/A",
];

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    depth: usize,
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\n' | '\r' | '\t')
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '\\'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?')
}

/// Characters allowed in an unquoted sheet qualifier.
fn is_sheet_char(c: char) -> bool {
    !(is_ws(c) || c.is_control() || "+-*/^&=<>,;(){}[]\"'!#%@$".contains(c))
}

fn col_number(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let n = letters.bytes().fold(0u32, |n, b| n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32);
    (n <= MAX_COL).then_some(n)
}

fn spaced(before: &str, expr: Expr, after: &str) -> Expr {
    if before.is_empty() && after.is_empty() {
        return expr;
    }
    Expr::Spaced { before: before.to_string(), expr: Box::new(expr), after: after.to_string() }
}

impl<'a> Parser<'a> {
    fn err(&self, message: &str) -> ParseError {
        ParseError { pos: self.pos, message: message.to_string() }
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, pos: usize) -> Option<char> {
        self.s[pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn ws(&mut self) -> &'a str {
        let start = self.pos;
        while self.peek().is_some_and(is_ws) {
            self.pos += 1;
        }
        &self.s[start..self.pos]
    }

    /// Scan while `f` holds, starting at `pos`; returns the end offset.
    fn scan(&self, mut pos: usize, f: impl Fn(char) -> bool) -> usize {
        while let Some(c) = self.peek_at(pos) {
            if !f(c) {
                break;
            }
            pos += c.len_utf8();
        }
        pos
    }

    fn binary_op(&self) -> Option<(BinaryOp, usize, u8)> {
        let r = self.rest();
        let two = |a: &str| r.starts_with(a);
        Some(if two("<>") {
            (BinaryOp::Ne, 2, 1)
        } else if two("<=") {
            (BinaryOp::Le, 2, 1)
        } else if two(">=") {
            (BinaryOp::Ge, 2, 1)
        } else {
            match r.chars().next()? {
                '=' => (BinaryOp::Eq, 1, 1),
                '<' => (BinaryOp::Lt, 1, 1),
                '>' => (BinaryOp::Gt, 1, 1),
                '&' => (BinaryOp::Concat, 1, 2),
                '+' => (BinaryOp::Add, 1, 3),
                '-' => (BinaryOp::Sub, 1, 3),
                '*' => (BinaryOp::Mul, 1, 4),
                '/' => (BinaryOp::Div, 1, 4),
                '^' => (BinaryOp::Pow, 1, 5),
                ',' => (BinaryOp::Union, 1, 8),
                ':' => (BinaryOp::Range, 1, 10),
                _ => return None,
            }
        })
    }

    /// Binary expression with operators of binding power >= `min_bp`.
    /// `union_ok` is false inside function arguments and array constants, where
    /// a comma separates items.
    fn expr(&mut self, min_bp: u8, union_ok: bool) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("formula is nested too deeply"));
        }
        let before = self.ws();
        let mut lhs = self.prefix(union_ok)?;
        lhs = spaced(before, lhs, "");
        loop {
            let save = self.pos;
            let ws = self.ws();
            let found = match self.binary_op() {
                Some((BinaryOp::Union, _, _)) if !union_ok => None,
                Some(op) => Some(op),
                // Whitespace followed by the start of an operand is the intersection operator.
                None if !ws.is_empty() && self.peek().is_some_and(|c| is_name_start(c) || matches!(c, '$' | '\'' | '[' | '(')) => {
                    Some((BinaryOp::Intersect, 0, 9))
                }
                None => None,
            };
            let Some((op, len, bp)) = found else {
                lhs = spaced("", lhs, ws);
                break;
            };
            if bp < min_bp {
                self.pos = save;
                break;
            }
            let op_text = if op == BinaryOp::Intersect {
                ws.to_string()
            } else {
                lhs = spaced("", lhs, ws);
                let t = &self.s[self.pos..self.pos + len];
                self.pos += len;
                t.to_string()
            };
            let rhs = self.expr(bp + 1, union_ok)?;
            lhs = Expr::Binary { op, op_text, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        self.depth -= 1;
        Ok(lhs)
    }

    fn prefix(&mut self, union_ok: bool) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(op @ ('-' | '+' | '@')) => {
                self.pos += 1;
                // Unary operators bind tighter than `^` but looser than reference operators.
                let expr = self.expr(6, union_ok)?;
                Ok(Expr::Unary { op, expr: Box::new(expr) })
            }
            _ => {
                let atom = self.atom()?;
                self.postfix(atom)
            }
        }
    }

    fn postfix(&mut self, mut e: Expr) -> Result<Expr, ParseError> {
        loop {
            match self.peek() {
                Some('%') => {
                    self.pos += 1;
                    e = Expr::Postfix { expr: Box::new(e), op: '%' };
                }
                Some('#') if self.error_at(self.pos).is_none() => {
                    self.pos += 1;
                    e = Expr::Postfix { expr: Box::new(e), op: '#' };
                }
                Some('(') if matches!(e, Expr::Call { .. } | Expr::Apply { .. } | Expr::Paren(_)) => {
                    self.pos += 1;
                    let args = self.args()?;
                    e = Expr::Apply { callee: Box::new(e), args };
                }
                _ => return Ok(e),
            }
        }
    }

    fn error_at(&self, pos: usize) -> Option<&'static str> {
        let r = &self.s[pos..];
        ERRORS.iter().copied().find(|e| r.len() >= e.len() && r.as_bytes()[..e.len()].eq_ignore_ascii_case(e.as_bytes()))
    }

    fn atom(&mut self) -> Result<Expr, ParseError> {
        let Some(c) = self.peek() else { return Err(self.err("unexpected end of formula")) };
        match c {
            '"' => self.string(),
            '{' => self.array(),
            '(' => {
                self.pos += 1;
                let e = self.expr(0, true)?;
                if !self.eat(')') {
                    return Err(self.err("expected ')'"));
                }
                Ok(Expr::Paren(Box::new(e)))
            }
            '#' => match self.error_at(self.pos) {
                Some(e) => {
                    let t = &self.s[self.pos..self.pos + e.len()];
                    self.pos += e.len();
                    Ok(Expr::Error(t.to_string()))
                }
                None => Err(self.err("unknown error literal")),
            },
            '[' => match self.qualifier() {
                Some(prefix) => self.reference(Some(prefix)),
                None => {
                    let spec = self.bracket_spec()?;
                    Ok(Expr::Structured { table: None, spec })
                }
            },
            '\'' => match self.qualifier() {
                Some(prefix) => self.reference(Some(prefix)),
                None => Err(self.err("quoted sheet name without '!'")),
            },
            c if c.is_ascii_digit() || c == '.' => {
                if let Some(r) = self.row_range() {
                    return Ok(r);
                }
                // A sheet name may start with a digit: `2024!A1`.
                if let Some(prefix) = self.qualifier() {
                    return self.reference(Some(prefix));
                }
                self.number()
            }
            c if c == '$' || is_name_start(c) => {
                let prefix = self.qualifier();
                self.reference(prefix)
            }
            _ => Err(self.err("unexpected character")),
        }
    }

    /// Sheet or workbook qualifier including the trailing `!`:
    /// `Sheet1!`, `'My Sheet'!`, `[1]Sheet1!`, `Sheet1:Sheet3!`, `[1]!`.
    fn qualifier(&mut self) -> Option<String> {
        let start = self.pos;
        let mut pos = start;
        if self.peek() == Some('\'') {
            pos += 1;
            loop {
                match self.peek_at(pos)? {
                    '\'' if self.peek_at(pos + 1) == Some('\'') => pos += 2,
                    '\'' => {
                        pos += 1;
                        break;
                    }
                    c => pos += c.len_utf8(),
                }
            }
        } else {
            // Unquoted: sheet names may hold almost any character that is not an operator.
            // A leading `[...]` is a workbook index or path and may contain a quoted part.
            if self.peek() == Some('[') {
                pos += 1;
                loop {
                    match self.peek_at(pos)? {
                        ']' => {
                            pos += 1;
                            break;
                        }
                        '\'' => {
                            pos += 1;
                            while self.peek_at(pos)? != '\'' {
                                pos += self.peek_at(pos)?.len_utf8();
                            }
                            pos += 1;
                        }
                        c if is_ws(c) || matches!(c, '[' | '!' | '(' | ')' | ',' | '"') => return None,
                        c => pos += c.len_utf8(),
                    }
                }
            }
            pos = self.scan(pos, is_sheet_char);
            if pos == start {
                return None;
            }
        }
        if self.peek_at(pos) != Some('!') {
            return None;
        }
        self.pos = pos + 1;
        Some(self.s[start..self.pos].to_string())
    }

    /// `$A$1`-style pieces. Returns the end offset of an optional `$` plus letters.
    fn col_piece(&self, pos: usize) -> Option<usize> {
        let p = if self.peek_at(pos) == Some('$') { pos + 1 } else { pos };
        let end = self.scan(p, |c| c.is_ascii_alphabetic());
        col_number(&self.s[p..end]).map(|_| end)
    }

    fn row_piece(&self, pos: usize) -> Option<usize> {
        let p = if self.peek_at(pos) == Some('$') { pos + 1 } else { pos };
        let end = self.scan(p, |c| c.is_ascii_digit());
        let digits = &self.s[p..end];
        if digits.is_empty() || digits.len() > 7 || digits.starts_with('0') {
            return None;
        }
        (digits.parse::<u64>().ok()? <= MAX_ROW).then_some(end)
    }

    fn ends_token(&self, pos: usize) -> bool {
        !self.peek_at(pos).is_some_and(|c| is_name_char(c) || c == '(' || c == '$')
    }

    fn row_range(&mut self) -> Option<Expr> {
        let a = self.row_piece(self.pos)?;
        if self.peek_at(a) != Some(':') {
            return None;
        }
        let b = self.row_piece(a + 1)?;
        if !self.ends_token(b) {
            return None;
        }
        let text = self.s[self.pos..b].to_string();
        self.pos = b;
        Some(Expr::Ref { prefix: None, kind: RefKind::Rows, text })
    }

    /// Whatever may follow an optional qualifier: reference, name, function call,
    /// structured reference, boolean or (after a qualifier) an error literal.
    fn reference(&mut self, prefix: Option<String>) -> Result<Expr, ParseError> {
        let start = self.pos;
        if prefix.is_some() {
            if let Some(e) = self.error_at(start) {
                self.pos += e.len();
                return Ok(Expr::Error(format!("{}{}", prefix.unwrap(), &self.s[start..self.pos])));
            }
        }
        // `1:3` after a qualifier, `$1:$3` anywhere
        if let Some(Expr::Ref { kind, text, .. }) = self.row_range() {
            return Ok(Expr::Ref { prefix, kind, text });
        }
        // A:C
        if let Some(a) = self.col_piece(start) {
            if self.peek_at(a) == Some(':') {
                if let Some(b) = self.col_piece(a + 1) {
                    if self.ends_token(b) && !self.peek_at(b).is_some_and(|c| c.is_ascii_digit()) {
                        self.pos = b;
                        return Ok(Expr::Ref { prefix, kind: RefKind::Columns, text: self.s[start..b].to_string() });
                    }
                }
            }
            // A1
            if let Some(b) = self.row_piece(a) {
                if self.ends_token(b) {
                    self.pos = b;
                    return Ok(Expr::Ref { prefix, kind: RefKind::Cell, text: self.s[start..b].to_string() });
                }
            }
        }
        if !self.peek().is_some_and(is_name_start) {
            return Err(self.err("expected a reference or name"));
        }
        let end = self.scan(start, is_name_char);
        let ident = &self.s[start..end];
        self.pos = end;
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let args = self.args()?;
                let name = match prefix {
                    Some(p) => format!("{p}{ident}"),
                    None => ident.to_string(),
                };
                Ok(Expr::Call { name, args })
            }
            Some('[') if prefix.is_none() => {
                let spec = self.bracket_spec()?;
                Ok(Expr::Structured { table: Some(ident.to_string()), spec })
            }
            _ if prefix.is_none() && (ident.eq_ignore_ascii_case("TRUE") || ident.eq_ignore_ascii_case("FALSE")) => {
                Ok(Expr::Bool(ident.to_string()))
            }
            _ => Ok(Expr::Name { prefix, text: ident.to_string() }),
        }
    }

    /// Balanced `[...]` of a structured reference. An apostrophe escapes the next character.
    fn bracket_spec(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        let mut depth = 0usize;
        loop {
            match self.bump() {
                Some('[') => depth += 1,
                Some(']') => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.s[start..self.pos].to_string());
                    }
                }
                Some('\'') => {
                    self.bump();
                }
                Some(_) => {}
                None => return Err(self.err("unterminated structured reference")),
            }
        }
    }

    /// Arguments after the opening parenthesis, up to and including `)`.
    fn args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        loop {
            let save = self.pos;
            let ws = self.ws();
            let arg = match self.peek() {
                Some(')') if args.is_empty() && ws.is_empty() => {
                    self.pos += 1;
                    return Ok(args);
                }
                Some(',' | ')') => spaced(ws, Expr::Missing, ""),
                _ => {
                    self.pos = save;
                    self.expr(0, false)?
                }
            };
            args.push(arg);
            match self.bump() {
                Some(',') => {}
                Some(')') => return Ok(args),
                _ => {
                    self.pos = self.pos.min(self.s.len());
                    return Err(self.err("expected ',' or ')'"));
                }
            }
        }
    }

    fn string(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.bump() {
                Some('"') => {
                    if !self.eat('"') {
                        return Ok(Expr::Str(self.s[start..self.pos].to_string()));
                    }
                }
                Some(_) => {}
                None => return Err(self.err("unterminated string")),
            }
        }
    }

    fn number(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        let mut end = self.scan(start, |c| c.is_ascii_digit());
        if self.peek_at(end) == Some('.') {
            end = self.scan(end + 1, |c| c.is_ascii_digit());
        }
        if !self.s[start..end].bytes().any(|b| b.is_ascii_digit()) {
            return Err(self.err("expected a number"));
        }
        if matches!(self.peek_at(end), Some('e' | 'E')) {
            let mut p = end + 1;
            if matches!(self.peek_at(p), Some('+' | '-')) {
                p += 1;
            }
            let e = self.scan(p, |c| c.is_ascii_digit());
            if e > p {
                end = e;
            }
        }
        self.pos = end;
        Ok(Expr::Number(self.s[start..end].to_string()))
    }

    fn array(&mut self) -> Result<Expr, ParseError> {
        self.pos += 1;
        let mut rows = vec![Vec::new()];
        loop {
            // Elements may be empty: `{1,,2;,,}` occurs in defined names of real workbooks.
            let save = self.pos;
            let ws = self.ws();
            let e = if matches!(self.peek(), Some(',' | ';' | '}')) {
                spaced(ws, Expr::Missing, "")
            } else {
                self.pos = save;
                self.expr(0, false)?
            };
            rows.last_mut().unwrap().push(e);
            match self.bump() {
                Some(',') => {}
                Some(';') => rows.push(Vec::new()),
                Some('}') => return Ok(Expr::Array(rows)),
                _ => return Err(self.err("expected ',', ';' or '}' in array constant")),
            }
        }
    }
}
