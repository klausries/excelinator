//! What the tests of the server share.
#![allow(dead_code)]
use std::io::Write;
use xl_cells::{Sheet, Value, Workbook};

/// One HTTP request to the server; returns (status, body).
pub fn http(addr: &str, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    use std::io::Read;
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
    stream.write_all(body).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut data = raw[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        let mut rest = &data[..];
        while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
            let n = usize::from_str_radix(String::from_utf8_lossy(&rest[..end]).trim(), 16).unwrap_or(0);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&rest[end + 2..end + 2 + n]);
            rest = &rest[end + 2 + n + 2..];
        }
        data = out;
    }
    (status, data)
}


/// What the server says to a request written out in full.
pub fn raw(addr: &str, request: &str) -> String {
    use std::io::Read;
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

/// A sheet from lines of cells: `=…` is a formula, a number a number, `-` nothing.
pub fn sheet(wb: &mut Workbook, name: &str, lines: &[&str]) {
    let s = wb.sheets.len() as u32;
    wb.sheets.push(Sheet { name: name.into(), ..Default::default() });
    for (r, line) in lines.iter().enumerate() {
        for (c, text) in line.split('|').map(str::trim).enumerate() {
            let (r, c) = (r as u32, c as u32);
            match (text, text.strip_prefix('='), text.parse::<f64>()) {
                ("-" | "", _, _) => {}
                (_, Some(f), _) => wb.cells.push(s, r, c, Value::Blank, Some(f.to_string()), 0),
                (_, _, Ok(n)) => wb.cells.push(s, r, c, Value::Number(n), None, 0),
                _ => wb.cells.push(s, r, c, Value::Text(text.to_string()), None, 0),
            }
        }
    }
}

/// What a cell of a workbook holds: its value as text, and ` =formula` if it has one.
pub fn cell(wb: &Workbook, sheet: &str, at: &str) -> String {
    let s = wb.sheets.iter().position(|x| x.name == sheet).expect("the sheet") as u32;
    let r = xl_formula::parse_cell_ref(at).expect("an address");
    let c = &wb.cells;
    let Some(i) = (0..c.len()).rfind(|&i| (c.sheet[i], c.row[i] + 1, c.col[i] + 1) == (s, r.row, r.col)) else { return String::new() };
    let value = match c.value(i) {
        Value::Number(n) => format!("{}", (n * 1e9).round() / 1e9),
        Value::Text(t) | Value::Error(t) => t,
        Value::Bool(b) => b.to_string(),
        Value::Blank => String::new(),
    };
    format!("{value}{}", c.formula[i].as_ref().map(|f| format!(" ={f}")).unwrap_or_default())
}
