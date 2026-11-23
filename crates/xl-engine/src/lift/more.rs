//! Lookups by position (`MATCH`, `INDEX`, `LOOKUP`, wide `VLOOKUP`), conditional
//! aggregates, `SUMPRODUCT`, `RANK`, date arithmetic and `TEXT` for dates.

use super::*;

/// A cell as lookups hand it on: number (NaN for an error value) and text; both
/// `None` for an empty cell.
type Entry = (Option<f64>, Option<String>);

/// An expression with as many rows as the frame, also where it reads no column.
pub(super) fn full(e: Expr) -> Expr {
    when(col(ROW).is_not_null()).then(e.clone()).otherwise(e)
}

const MONTH_NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAY_NAMES: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// A criterion of `SUMIF` and its relatives.
enum Crit {
    /// Equal to a value given per row (number, or text without operator or wildcard).
    Eq(T),
    /// A comparison with a number given per row.
    Cmp(BinaryOp, Expr),
    /// Equal or not equal to a fixed text (lower case), wildcards allowed.
    Text { negate: bool, pattern: String },
}

fn wildcard_regex(pattern: &str) -> String {
    let mut out = String::from("(?s)^");
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => match chars.next() {
                Some(n) => out.push_str(&regex_escape(n)),
                None => out.push('~'),
            },
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            other => out.push_str(&regex_escape(other)),
        }
    }
    out.push('$');
    out
}

/// A matcher for the patterns `wildcard_regex` makes: `^`, `$`, `.`, `.*` and escapes.
fn regex_lite(pattern: &str) -> impl Fn(&str) -> bool {
    // Tokens: Some(c) a literal character, None any one character; `star` marks `.*`.
    let mut tokens: Vec<(Option<char>, bool)> = Vec::new();
    let body = pattern.trim_start_matches("(?s)^").trim_end_matches('$');
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => tokens.push((chars.next(), false)),
            '.' if chars.peek() == Some(&'*') => {
                chars.next();
                tokens.push((None, true));
            }
            '.' => tokens.push((None, false)),
            other => tokens.push((Some(other), false)),
        }
    }
    move |text: &str| {
        let t: Vec<char> = text.chars().collect();
        fn go(tokens: &[(Option<char>, bool)], t: &[char]) -> bool {
            match tokens.split_first() {
                None => t.is_empty(),
                Some(((_, true), rest)) => (0..=t.len()).any(|k| go(rest, &t[k..])),
                Some(((Some(c), false), rest)) => t.first() == Some(c) && go(rest, &t[1..]),
                Some(((None, false), rest)) => !t.is_empty() && go(rest, &t[1..]),
            }
        }
        go(&tokens, &t)
    }
}

fn regex_escape(c: char) -> String {
    if "\\.+*?()|[]{}^$#&-~".contains(c) { format!("\\{c}") } else { c.to_string() }
}

/// Excel's 2-digit years: 00-29 are 2000-2029, 30-99 are 1930-1999.
fn century(y: Expr, len: Expr) -> Expr {
    when(len.gt(lit(2u32))).then(y.clone()).otherwise(when(y.clone().lt(lit(30.0))).then(y.clone() + lit(2000.0)).otherwise(y + lit(1900.0)))
}

impl Context<'_> {
    pub(super) fn entry(&self, sheet: u32, row: u32, col: u32) -> Entry {
        let c = &self.wb.cells;
        match self.peek(&(sheet, row - 1, col - 1)) {
            None => (None, None),
            Some(&i) => match c.kind[i] {
                Kind::Blank if c.formula[i].is_some() => (None, Some(String::new())),
                Kind::Blank => (None, None),
                Kind::Number | Kind::Bool => (Some(c.num[i]), None),
                Kind::Text => (None, Some(c.text[i].as_deref().unwrap_or("").to_string())),
                Kind::Error => (Some(f64::NAN), None),
            },
        }
    }

    pub(super) fn key(&self, sheet: u32, row: u32, col: u32) -> Key {
        let c = &self.wb.cells;
        match self.peek(&(sheet, row - 1, col - 1)) {
            None => Key::Blank,
            Some(&i) => match c.kind[i] {
                Kind::Blank if c.formula[i].is_some() => Key::Text(String::new()),
                Kind::Blank => Key::Blank,
                Kind::Number => Key::Num(c.num[i]),
                Kind::Text => Key::Text(c.text[i].as_deref().unwrap_or("").to_lowercase()),
                Kind::Bool | Kind::Error => Key::Other,
            },
        }
    }

    /// Rows where the translation does not hold: those cells are left to the cell
    /// evaluator (the run is then not counted as lifted).
    pub(super) fn guard(&mut self, reason: &str, rows: Expr) {
        let rows = bind(&mut self.lets, full(rows.fill_null(lit(false))));
        self.guards.push((reason.to_string(), rows));
    }

    /// Pick, per row, one of known cells by its number `flat`; where `valid` is false
    /// the result is an error value (#REF!, #N/A).
    pub(super) fn pick(&mut self, flat: Expr, valid: Expr, entries: Vec<(f64, Entry)>) -> T {
        let flat = bind(&mut self.lets, full(flat));
        let any_text = entries.iter().any(|(_, (_, t))| t.is_some());
        let entries: Vec<(f64, Entry)> = entries.into_iter().filter(|(_, (n, t))| n.is_some() || t.is_some()).collect();
        let (number, text) = if entries.is_empty() {
            (null_num(), null_text())
        } else {
            let keys = Series::new("k".into(), entries.iter().map(|e| e.0).collect::<Vec<f64>>());
            let numbers = Series::new("n".into(), entries.iter().map(|e| e.1 .0).collect::<Vec<Option<f64>>>());
            let texts = Series::new("t".into(), entries.iter().map(|e| e.1 .1.as_deref()).collect::<Vec<Option<&str>>>());
            (
                flat.clone().replace_strict(lit(keys.clone()), lit(numbers), Some(null_num()), Some(DataType::Float64)),
                flat.replace_strict(lit(keys), lit(texts), Some(null_text()), Some(DataType::String)),
            )
        };
        let valid = bind(&mut self.lets, full(valid.fill_null(lit(false))));
        let num = bind(&mut self.lets, when(valid.clone()).then(number).otherwise(lit(f64::NAN)));
        let text = any_text.then(|| bind(&mut self.lets, when(valid).then(text).otherwise(null_text())));
        T::Dyn(Dyn { num, text, blank: true, nan: true, parsed: None })
    }

    /// The 1-based position of `key` among `keys`; null where it is not found.
    /// `mode` as in MATCH: 0 the first equal, 1 the last not above (keys ascending),
    /// -1 the last not below (keys descending).
    pub(super) fn position(&mut self, key: T, keys: &[Key], mode: i8) -> Lifted<Expr> {
        let (by_number, by_text) = match key.bound(&mut self.lets) {
            T::Num(e) => (Some(e), None),
            T::Text(e) => (None, Some(e)),
            T::Bool(_) => return Err("lookup of a truth value".into()),
            T::Dyn(d) => (Some(d.num.clone()), d.text.clone()),
        };
        // Excel matches text keys with wildcards in exact lookups.
        if let (Some(t), 0) = (&by_text, mode) {
            self.guard("wildcard in a lookup key", t.clone().str().contains(lit(r"[*?~]"), false));
        }
        let by_text = by_text.map(|t| bind(&mut self.lets, t.str().to_lowercase()));
        let mut found: Vec<(Expr, Expr)> = Vec::new();
        if mode == 0 {
            let (mut numbers, mut texts) = (Vec::<(f64, f64)>::new(), Vec::<(&str, f64)>::new());
            let (mut seen_n, mut seen_t) = (std::collections::HashSet::new(), std::collections::HashSet::new());
            for (p, k) in keys.iter().enumerate() {
                match k {
                    Key::Num(v) if seen_n.insert((v + 0.0).to_bits()) => numbers.push((v + 0.0, p as f64 + 1.0)),
                    Key::Text(t) if seen_t.insert(t.as_str()) => texts.push((t.as_str(), p as f64 + 1.0)),
                    _ => {}
                }
            }
            if let (Some(k), false) = (&by_number, numbers.is_empty()) {
                let old = Series::new("k".into(), numbers.iter().map(|x| x.0).collect::<Vec<f64>>());
                let new = Series::new("p".into(), numbers.iter().map(|x| x.1).collect::<Vec<f64>>());
                found.push((k.clone().is_not_null(), full(k.clone()).replace_strict(lit(old), lit(new), Some(null_num()), Some(DataType::Float64))));
            }
            if let (Some(k), false) = (&by_text, texts.is_empty()) {
                let old = Series::new("k".into(), texts.iter().map(|x| x.0).collect::<Vec<&str>>());
                let new = Series::new("p".into(), texts.iter().map(|x| x.1).collect::<Vec<f64>>());
                found.push((k.clone().is_not_null(), full(k.clone()).replace_strict(lit(old), lit(new), Some(null_num()), Some(DataType::Float64))));
            }
        } else {
            let ascending = mode > 0;
            // Excel halves through the keys and compares only keys of the kind sought;
            // taken here as: among the keys of that kind (in order), the last not above
            // (not below) the value. Other cells (empty, errors, the other kind) are passed over.
            let numbers: Vec<(f64, f64)> = keys.iter().enumerate().filter_map(|(p, k)| if let Key::Num(v) = k { Some((p as f64 + 1.0, *v)) } else { None }).collect();
            let texts: Vec<(f64, &str)> = keys.iter().enumerate().filter_map(|(p, k)| if let Key::Text(t) = k { Some((p as f64 + 1.0, t.as_str())) } else { None }).collect();
            let sorted_numbers = numbers.windows(2).all(|w| if ascending { w[0].1 <= w[1].1 } else { w[0].1 >= w[1].1 });
            let sorted_texts = texts.windows(2).all(|w| if ascending { w[0].1 <= w[1].1 } else { w[0].1 >= w[1].1 });
            if !sorted_numbers || !sorted_texts {
                if !ascending {
                    return Err("approximate lookup in keys that are not in order".into());
                }
                // Keys out of order: Excel's halving search still gives an answer. As a
                // function of the value it changes only at the keys, so it is worked out
                // here for every stretch between keys and looked up by stretch.
                let mut found: Vec<(Expr, Expr)> = Vec::new();
                if let (Some(k), false) = (&by_number, numbers.is_empty()) {
                    let values: Vec<f64> = numbers.iter().map(|x| x.1).collect();
                    let positions: Vec<f64> = numbers.iter().map(|x| x.0).collect();
                    let mut cuts = values.clone();
                    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    cuts.dedup();
                    let halve = |x: f64| halving(&values, |v| v.partial_cmp(&x).unwrap()).map(|i| positions[i]);
                    found.push((k.clone().is_not_null(), stretches(k.clone(), &cuts, halve)));
                }
                if let (Some(k), false) = (&by_text, texts.is_empty()) {
                    let values: Vec<String> = texts.iter().map(|x| x.1.to_string()).collect();
                    let positions: Vec<f64> = texts.iter().map(|x| x.0).collect();
                    let mut cuts = values.clone();
                    cuts.sort();
                    cuts.dedup();
                    // Text stretches: at each key and just after it ("key" + a char above all).
                    let mut points: Vec<(String, Option<f64>)> = Vec::new();
                    points.push((String::new(), halving(&values, |v| v.as_str().cmp("")).map(|i| positions[i])));
                    for c in &cuts {
                        points.push((c.clone(), halving(&values, |v| v.as_str().cmp(c.as_str())).map(|i| positions[i])));
                    }
                    let keys = Series::new("c".into(), cuts.clone());
                    // Index of the last cut not above the value, 0 if none.
                    let at = lit(keys).search_sorted(full(k.clone()), SearchSortedSide::Right, false).cast(DataType::Float64);
                    let exact_cut = {
                        let ks = Series::new("k".into(), cuts.clone());
                        let one = Series::new("o".into(), vec![1.0f64; cuts.len()]);
                        full(k.clone()).replace_strict(lit(ks), lit(one), Some(lit(0.0)), Some(DataType::Float64))
                    };
                    // Between cuts the answer is the one at the lower cut, unless the halving
                    // differs between a key and the values just above it.
                    let mut between: Vec<Option<f64>> = Vec::new();
                    let mut at_cut: Vec<Option<f64>> = Vec::new();
                    for (i, c) in cuts.iter().enumerate() {
                        at_cut.push(points[i + 1].1);
                        let above = format!("{c}\u{10FFFF}");
                        between.push(halving(&values, |v| v.as_str().cmp(above.as_str())).map(|i| positions[i]));
                    }
                    let idx = Series::new("i".into(), (1..=cuts.len()).map(|i| i as f64).collect::<Vec<f64>>());
                    let on = full(at.clone()).replace_strict(lit(idx.clone()), lit(Series::new("a".into(), at_cut)), Some(null_num()), Some(DataType::Float64));
                    let off = full(at.clone()).replace_strict(lit(idx), lit(Series::new("b".into(), between)), Some(null_num()), Some(DataType::Float64));
                    let below = points[0].1.map(lit).unwrap_or_else(null_num);
                    let p = when(at.clone().eq(lit(0.0))).then(below).otherwise(when(exact_cut.eq(lit(1.0))).then(on).otherwise(off));
                    found.push((k.clone().is_not_null(), p));
                }
                let mut result = null_num();
                for (applies, p) in found {
                    result = when(applies).then(p).otherwise(result);
                }
                return Ok(bind(&mut self.lets, result));
            }
            // The position among the keys of a kind, mapped to the position in the table.
            let place = |found_at: Expr, positions: Vec<f64>| -> Expr {
                let n = positions.len();
                let k = Series::new("k".into(), (1..=n).map(|i| i as f64).collect::<Vec<f64>>());
                full(found_at).replace_strict(lit(k), lit(Series::new("p".into(), positions)), Some(null_num()), Some(DataType::Float64))
            };
            if let (Some(k), false) = (&by_number, numbers.is_empty()) {
                let n = numbers.len() as f64;
                let mut sorted: Vec<f64> = numbers.iter().map(|x| x.1).collect();
                let at = if ascending {
                    lit(Series::new("k".into(), sorted)).search_sorted(full(k.clone()), SearchSortedSide::Right, false).cast(DataType::Float64)
                } else {
                    sorted.reverse();
                    lit(n) - lit(Series::new("k".into(), sorted)).search_sorted(full(k.clone()), SearchSortedSide::Left, false).cast(DataType::Float64)
                };
                found.push((k.clone().is_not_null(), place(at, numbers.iter().map(|x| x.0).collect())));
            }
            if let (Some(k), false) = (&by_text, texts.is_empty()) {
                let n = texts.len() as f64;
                let mut sorted: Vec<&str> = texts.iter().map(|x| x.1).collect();
                let at = if ascending {
                    lit(Series::new("k".into(), sorted)).search_sorted(full(k.clone()), SearchSortedSide::Right, false).cast(DataType::Float64)
                } else {
                    sorted.reverse();
                    lit(n) - lit(Series::new("k".into(), sorted)).search_sorted(full(k.clone()), SearchSortedSide::Left, false).cast(DataType::Float64)
                };
                found.push((k.clone().is_not_null(), place(at, texts.iter().map(|x| x.0).collect())));
            }
            for (_, p) in found.iter_mut() {
                *p = when(p.clone().lt_eq(lit(0.0))).then(null_num()).otherwise(p.clone());
            }
        }
        let mut result = null_num();
        for (applies, p) in found {
            result = when(applies).then(p).otherwise(result);
        }
        Ok(bind(&mut self.lets, result))
    }

    /// Whether the cell numbered `k` (row by row from 1) of a block holds #N/A.
    fn is_na_entry(&self, b: Block, k: f64) -> bool {
        let w = b.c2 - b.c1 + 1;
        let k = k as u32 - 1;
        let (r, c) = (b.r1 + k / w, b.c1 + k % w);
        self.peek(&(b.sheet, r - 1, c - 1)).is_some_and(|&i| self.wb.cells.kind[i] == Kind::Error && self.wb.cells.text[i].as_deref().is_some_and(|t| t.eq_ignore_ascii_case("#N/A")))
    }

    /// A block of another workbook must be in the file's cache: if none of its cells
    /// is, the file does not carry what the formula reads.
    pub(super) fn kept(&self, b: Block) -> Lifted<()> {
        if b.sheet < self.external_from {
            return Ok(());
        }
        let any = (b.r1..=b.r2.min(b.r1 + 100_000)).any(|r| (b.c1..=b.c2).any(|c| self.peek_has(&(b.sheet, r - 1, c - 1))));
        if any { Ok(()) } else { Err("cells of another workbook not kept in the file".into()) }
    }

    /// The keys of one row or column of a block (1-based index within the block).
    fn line_keys(&self, b: Block, vertical: bool, which: u32) -> Vec<Key> {
        if vertical {
            (b.r1..=b.r2).map(|r| self.key(b.sheet, r, b.c1 + which - 1)).collect()
        } else {
            (b.c1..=b.c2).map(|c| self.key(b.sheet, b.r1 + which - 1, c)).collect()
        }
    }

    /// The cells of a block numbered row by row from 1.
    fn block_entries(&self, b: Block) -> Lifted<Vec<(f64, Entry)>> {
        let (h, w) = (b.r2 - b.r1 + 1, b.c2 - b.c1 + 1);
        if h as u64 * w as u64 > 4_000_000 {
            // Mostly empty (whole rows, A1:XFD999): only the cells the file holds.
            let c = &self.wb.cells;
            let mut out: Vec<(f64, Entry)> = (0..c.len())
                .filter(|&i| c.sheet[i] == b.sheet && (b.r1..=b.r2).contains(&(c.row[i] + 1)) && (b.c1..=b.c2).contains(&(c.col[i] + 1)))
                .filter_map(|i| {
                    let (r, col) = (c.row[i] + 1 - b.r1, c.col[i] + 1 - b.c1);
                    let e = self.entry(b.sheet, c.row[i] + 1, c.col[i] + 1);
                    (e.0.is_some() || e.1.is_some()).then(|| ((r as u64 * w as u64 + col as u64) as f64 + 1.0, e))
                })
                .collect();
            if out.len() > 4_000_000 {
                return Err("very large table".into());
            }
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            return Ok(out);
        }
        let mut out = Vec::new();
        for r in 0..h {
            for c in 0..w {
                let e = self.entry(b.sheet, b.r1 + r, b.c1 + c);
                if e.0.is_some() || e.1.is_some() {
                    out.push(((r * w + c) as f64 + 1.0, e));
                }
            }
        }
        Ok(out)
    }

    /// VLOOKUP / HLOOKUP through a position, for any column index.
    pub(super) fn lookup_at(&mut self, key: T, b: Block, index: Expr, vertical: bool, exact: bool) -> Lifted<T> {
        self.lookup_in(key, b, index, vertical, exact, None)
    }

    /// A lookup table with one end fixed and the other moving with the row: the block
    /// all rows together cover, and per row the first and last position in it (1-based).
    pub(super) fn moving_table(&mut self, e: &Fx) -> Lifted<Option<(Block, Expr, Expr)>> {
        let Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } = core(e) else { return Ok(None) };
        let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (core(lhs), core(rhs)) else { return Ok(None) };
        let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
        if p.abs_row == q.abs_row || self.in_name > 0 {
            return Ok(None);
        }
        let sheet = self.sheet_of(prefix)?;
        let (fixed, moving) = if p.abs_row { (p.row as i64, q.row as i64) } else { (q.row as i64, p.row as i64) };
        let offset = moving - self.row as i64;
        let (first, last) = (moving, moving + self.len as i64 - 1);
        let r1 = fixed.min(first).max(1);
        let r2 = fixed.max(last);
        let block = Block { sheet, r1: r1 as u32, r2: r2 as u32, c1: p.col.min(q.col), c2: p.col.max(q.col) };
        self.not_own_cells(block)?;
        let here = col(ROW).cast(DataType::Float64) + lit(offset as f64);
        let lo = when(here.clone().lt(lit(fixed as f64))).then(here.clone()).otherwise(lit(fixed as f64)) - lit(r1 as f64 - 1.0);
        let hi = when(here.clone().gt(lit(fixed as f64))).then(here).otherwise(lit(fixed as f64)) - lit(r1 as f64 - 1.0);
        Ok(Some((block, bind(&mut self.lets, lo), bind(&mut self.lets, hi))))
    }

    /// VLOOKUP in the rows `window` (first and last position, per row) of a block.
    pub(super) fn lookup_in(&mut self, key: T, b: Block, index: Expr, vertical: bool, exact: bool, window: Option<(Expr, Expr)>) -> Lifted<T> {
        self.kept(b)?;
        let keys = self.line_keys(b, vertical, 1);
        if b.sheet >= self.external_from && keys.iter().all(|k| *k == Key::Blank) {
            return Err("cells of another workbook not kept in the file".into());
        }
        let key = key.bound(&mut self.lets);
        let key_error = bind(&mut self.lets, key.is_error());
        let mut pos = self.position(key, &keys, if exact { 0 } else { 1 })?;
        if let Some((lo, hi)) = window {
            if exact {
                // With every key once, the first equal key in the window is the one found.
                let mut seen = std::collections::HashSet::new();
                for k in &keys {
                    let id = match k {
                        Key::Num(v) => format!("n{}", (v + 0.0).to_bits()),
                        Key::Text(t) => format!("t{t}"),
                        _ => continue,
                    };
                    if !seen.insert(id) {
                        return Err("lookup table that moves with the row, with repeated keys".into());
                    }
                }
                pos = when(pos.clone().gt_eq(lo.clone()).and(pos.clone().lt_eq(hi))).then(pos).otherwise(null_num());
            } else {
                // The last key not above the value in the window: the last one found
                // overall, held to the window's end, moved back to a key of the same kind.
                let kinds: Vec<u8> = keys.iter().map(|k| match k {
                    Key::Num(_) => 1,
                    Key::Text(_) => 2,
                    _ => 0,
                }).collect();
                let held = when(pos.clone().gt(hi.clone())).then(hi.clone()).otherwise(pos.clone());
                let held = bind(&mut self.lets, held);
                let mut back: Vec<Option<f64>> = Vec::with_capacity(keys.len());
                for kind in [1u8, 2u8] {
                    let mut last = None;
                    let mut map = Vec::with_capacity(keys.len());
                    for (p, k) in kinds.iter().enumerate() {
                        if *k == kind {
                            last = Some(p as f64 + 1.0);
                        }
                        map.push(last);
                    }
                    back = if kind == 1 { map } else {
                        // Which kind the key found has decides the list used.
                        let numbers = std::mem::take(&mut back);
                        let positions = Series::new("p".into(), (1..=keys.len()).map(|p| p as f64).collect::<Vec<f64>>());
                        let kind_at = Series::new("k".into(), kinds.iter().map(|k| *k as f64).collect::<Vec<f64>>());
                        let kind_found = full(pos.clone()).replace_strict(lit(positions.clone()), lit(kind_at), Some(null_num()), Some(DataType::Float64));
                        let by_n = full(held.clone()).replace_strict(lit(positions.clone()), lit(Series::new("a".into(), numbers)), Some(null_num()), Some(DataType::Float64));
                        let by_t = full(held.clone()).replace_strict(lit(positions), lit(Series::new("b".into(), map)), Some(null_num()), Some(DataType::Float64));
                        let moved = when(kind_found.eq(lit(2.0))).then(by_t).otherwise(by_n);
                        let moved = bind(&mut self.lets, moved);
                        pos = when(moved.clone().gt_eq(lo.clone())).then(moved).otherwise(null_num());
                        Vec::new()
                    };
                }
            }
            pos = bind(&mut self.lets, pos);
        }
        let (h, w) = (b.r2 - b.r1 + 1, b.c2 - b.c1 + 1);
        let index = bind(&mut self.lets, index.floor());
        let width = if vertical { w } else { h } as f64;
        // An error as the key is the result, whatever a sorted search would land on.
        let valid = pos.clone().is_not_null().and(key_error.clone().not()).and(index.clone().gt_eq(lit(1.0))).and(index.clone().lt_eq(lit(width)));
        let flat = if vertical { (pos.clone() - lit(1.0)) * lit(w as f64) + index.clone() } else { (index.clone() - lit(1.0)) * lit(w as f64) + pos.clone() };
        let flat = bind(&mut self.lets, flat);
        let entries = self.block_entries(b)?;
        // Which results are #N/A: the key not found, or a #N/A cell picked; an error in
        // the key is the key's own (filled in by the caller, see `last_na`).
        let na_cells: Vec<f64> = entries.iter().filter(|(_, (n, _))| n.is_some_and(f64::is_nan)).map(|(k, _)| *k).filter(|k| self.is_na_entry(b, *k)).collect();
        let picked_na = if na_cells.is_empty() {
            lit(false)
        } else {
            let ones = Series::new("o".into(), vec![1.0f64; na_cells.len()]);
            full(flat.clone()).replace_strict(lit(Series::new("k".into(), na_cells)), lit(ones), Some(lit(0.0)), Some(DataType::Float64)).eq(lit(1.0))
        };
        let in_range = index.clone().gt_eq(lit(1.0)).and(index.lt_eq(lit(width)));
        let na = pos.is_null().and(in_range.clone()).or(valid.clone().and(picked_na));
        self.last_na = Some((key_error, bind(&mut self.lets, na.fill_null(lit(false)))));
        Ok(self.pick(flat, valid, entries))
    }

    /// A block given by a reference, a name, or a range whose ends are fixed cells;
    /// `None` where the reference is an error value (`#REF!`).
    pub(super) fn table(&mut self, e: &Fx) -> Lifted<Option<Block>> {
        self.baked.set(self.baked.get() | BAKED_BLOCK);
        match core(e) {
            Fx::Error(_) => Ok(None),
            Fx::Name { prefix, text } => {
                let formula = self.name(&qualified(prefix, text))?;
                if matches!(core(&formula), Fx::Error(_)) {
                    return Ok(None);
                }
                self.in_name += 1;
                let result = self.table(&formula);
                self.in_name -= 1;
                result
            }
            Fx::Ref { prefix, kind: RefKind::Cell, text } => {
                let r = parse_cell_ref(text).ok_or("reference form")?;
                if !r.abs_row {
                    return Err("lookup table that moves with the row".into());
                }
                let sheet = self.sheet_of(prefix)?;
                Ok(Some(Block { sheet, r1: r.row, r2: r.row, c1: r.col, c2: r.col }))
            }
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if !matches!((core(lhs), core(rhs)), (Fx::Ref { .. }, Fx::Ref { .. })) => {
                let (Some(a), Some(b)) = (self.table(lhs)?, self.table(rhs)?) else { return Ok(None) };
                if a.sheet != b.sheet {
                    return Err("range across sheets".into());
                }
                Ok(Some(Block { sheet: a.sheet, r1: a.r1.min(b.r1), r2: a.r2.max(b.r2), c1: a.c1.min(b.c1), c2: a.c2.max(b.c2) }))
            }
            Fx::Ref { prefix, kind: RefKind::Rows, text } => {
                let sheet = self.sheet_of(prefix)?;
                let (a, b) = text.split_once(':').ok_or("row range form")?;
                let row = |t: &str| t.trim_start_matches('$').parse::<u32>().map_err(|_| "row range form".to_string());
                let (r1, r2) = (row(a)?, row(b)?);
                let last_col = self.wb.cells.col.iter().zip(&self.wb.cells.sheet).filter(|(_, s)| **s == sheet).map(|(c, _)| *c + 1).max().unwrap_or(1);
                let last_row = self.extent(sheet).max(1);
                Ok(Some(Block { sheet, r1: r1.min(r2), r2: r1.max(r2).min(last_row), c1: 1, c2: last_col }))
            }
            Fx::Call { name, args } if name.eq_ignore_ascii_case("CHOOSE") && args.len() > 1 && self.is_fixed(&args[0], 0) => match self.fixed_number(&args[0])? {
                Some(v) if v >= 1.0 && (v as usize) < args.len() => self.table(&args[v as usize]),
                _ => Ok(None),
            },
            Fx::Call { name, args } if name.eq_ignore_ascii_case("OFFSET") => match self.offset(args)? {
                Target::Fixed(b) => Ok(Some(b)),
                Target::Nowhere => Ok(None),
                _ => Err("lookup table that moves with the row".into()),
            },
            other => self.block(other).map(Some),
        }
    }

    pub(super) fn call_more(&mut self, name: &str, args: &[Fx]) -> Lifted<T> {
        let arity = |low: usize, high: usize| if (low..=high).contains(&args.len()) { Ok(()) } else { Err(format!("{name} with {} arguments", args.len())) };
        Ok(match name {
            "MATCH" => {
                arity(2, 3)?;
                // Left out: 1 (sorted); written but empty (`MATCH(x,r,)`): 0, exact.
                let mode = match args.get(2).map(core) {
                    None => 1,
                    Some(Fx::Missing) => 0,
                    Some(a) => {
                        if !self.is_fixed(a, 0) {
                            return Err("MATCH with a computed kind of match".into());
                        }
                        match self.fixed_number(a)? {
                            Some(v) if v > 0.0 => 1,
                            Some(v) if v < 0.0 => -1,
                            Some(_) => 0,
                            None => return Ok(T::Num(null_num())),
                        }
                    }
                };
                let key = self.translate(&args[0])?;
                let Some(b) = self.table(&args[1])? else { return Ok(T::Num(null_num())) };
                self.kept(b)?;
                let vertical = b.c1 == b.c2;
                if !vertical && b.r1 != b.r2 {
                    return Ok(T::Num(null_num()));
                }
                let keys = self.line_keys(b, vertical, 1);
                let key = key.bound(&mut self.lets);
                let key_error = bind(&mut self.lets, key.is_error());
                let pos = self.position(key, &keys, mode)?;
                self.last_na = Some((key_error, pos.clone().is_null()));
                T::Num(pos)
            }
            "INDEX" => {
                arity(2, 4)?;
                if args.len() == 4 {
                    return Err("INDEX with an area number".into());
                }
                let Some(b) = self.table(&args[0])? else { return Ok(T::Num(null_num())) };
                self.kept(b)?;
                let (h, w) = (b.r2 - b.r1 + 1, b.c2 - b.c1 + 1);
                let given = |ctx: &mut Self, a: Option<&Fx>| -> Lifted<Option<Expr>> {
                    match a.map(core) {
                        None | Some(Fx::Missing) => Ok(None),
                        Some(_) => Ok(Some(ctx.number(a.unwrap())?.floor())),
                    }
                };
                let first = given(self, args.get(1))?;
                let second = given(self, args.get(2))?;
                let (row, column) = match (first, second) {
                    (Some(r), Some(c)) => (r, c),
                    (Some(n), None) if h == 1 => (lit(1.0), n),
                    (Some(n), None) if w == 1 => (n, lit(1.0)),
                    (None, Some(c)) if h == 1 => (lit(1.0), c),
                    _ => return Err("INDEX of a whole row or column".into()),
                };
                // 0 asks for a whole row or column; in a cell that is the one in the formula's
                // own row or column (implicit intersection), if the block has it.
                let own_row = if (b.r1..=b.r2).contains(&self.row) || self.len > 1 {
                    col(ROW).cast(DataType::Float64) - lit(b.r1 as f64 - 1.0)
                } else {
                    lit(f64::NAN)
                };
                let own_col = if (b.c1..=b.c2).contains(&self.run_col) { lit((self.run_col - b.c1 + 1) as f64) } else { lit(f64::NAN) };
                let row = when(row.clone().eq(lit(0.0))).then(own_row).otherwise(row);
                let column = when(column.clone().eq(lit(0.0))).then(own_col).otherwise(column);
                let (row, column) = (bind(&mut self.lets, row), bind(&mut self.lets, column));
                let valid = row.clone().gt_eq(lit(1.0)).and(row.clone().lt_eq(lit(h as f64))).and(column.clone().gt_eq(lit(1.0))).and(column.clone().lt_eq(lit(w as f64)));
                let flat = (row - lit(1.0)) * lit(w as f64) + column;
                let entries = self.block_entries(b)?;
                self.pick(flat, valid, entries)
            }
            "LOOKUP" => {
                arity(2, 3)?;
                let key = self.translate(&args[0])?;
                let Some(b) = self.table(&args[1])? else { return Ok(T::Num(null_num())) };
                let (h, w) = (b.r2 - b.r1 + 1, b.c2 - b.c1 + 1);
                let (keys, result) = match args.get(2) {
                    Some(r) => {
                        let Some(rb) = self.table(r)? else { return Ok(T::Num(null_num())) };
                        if h != 1 && w != 1 {
                            return Err("LOOKUP in a table with a result vector".into());
                        }
                        (self.line_keys(b, w == 1, 1), rb)
                    }
                    None => {
                        let vertical = h >= w;
                        let last = if vertical { w } else { h };
                        let keys = self.line_keys(b, vertical, 1);
                        let rb = if vertical { Block { c1: b.c1 + last - 1, c2: b.c1 + last - 1, ..b } } else { Block { r1: b.r1 + last - 1, r2: b.r1 + last - 1, ..b } };
                        (keys, rb)
                    }
                };
                let pos = self.position(key, &keys, 1)?;
                let entries = self.block_entries(result)?;
                let count = ((result.r2 - result.r1 + 1) * (result.c2 - result.c1 + 1)) as f64;
                let valid = pos.clone().is_not_null().and(pos.clone().lt_eq(lit(count)));
                self.pick(pos, valid, entries)
            }
            "ROW" | "COLUMN" => {
                arity(0, 1)?;
                let rows = name == "ROW";
                match args.first().map(core) {
                    None => T::Num(if rows { col(ROW).cast(DataType::Float64) } else { lit(self.run_col as f64) }),
                    Some(Fx::Ref { kind: RefKind::Cell, text, .. }) => {
                        let r = parse_cell_ref(text).ok_or("reference form")?;
                        if rows {
                            T::Num(if r.abs_row { lit(r.row as f64) } else { col(ROW).cast(DataType::Float64) + lit(r.row as f64 - self.row as f64) })
                        } else {
                            T::Num(lit(r.col as f64))
                        }
                    }
                    Some(other) => {
                        let Some(b) = self.table(other)? else { return Ok(T::Num(null_num())) };
                        T::Num(lit(if rows { b.r1 } else { b.c1 } as f64))
                    }
                }
            }
            "ROWS" | "COLUMNS" => {
                arity(1, 1)?;
                let Some(b) = self.table(&args[0])? else { return Ok(T::Num(null_num())) };
                T::Num(lit(if name == "ROWS" { b.r2 - b.r1 + 1 } else { b.c2 - b.c1 + 1 } as f64))
            }
            "SUMPRODUCT" => {
                if args.is_empty() {
                    return Err("SUMPRODUCT without arguments".into());
                }
                let mut lists: Vec<Vec<T>> = Vec::new();
                for a in args {
                    let plain = matches!(core(a), Fx::Ref { .. } | Fx::Name { .. }) || matches!(core(a), Fx::Binary { op: BinaryOp::Range, .. });
                    if plain {
                        lists.push(self.cells_of(a)?);
                    } else {
                        // An expression over ranges, cell by cell (SUMPRODUCT takes arrays).
                        let mut list = Vec::new();
                        for element in self.elements(a)? {
                            list.push(self.translate(&element)?);
                        }
                        lists.push(list);
                    }
                }
                let n = lists[0].len();
                if lists.iter().any(|l| l.len() != n) {
                    return Ok(T::Num(null_num()));
                }
                let mut sum = lit(0.0);
                let mut error: Option<Expr> = None;
                for k in 0..n {
                    let mut product = lit(1.0);
                    for list in &lists {
                        let (value, err) = array_number(&mut self.lets, list[k].clone());
                        product = product * value;
                        error = Some(match error {
                            Some(e) => e.or(err),
                            None => err,
                        });
                    }
                    sum = sum + product;
                }
                let error = bind(&mut self.lets, error.unwrap_or(lit(false)));
                T::Num(when(error).then(null_num()).otherwise(sum))
            }
            "SUMIF" | "COUNTIF" | "AVERAGEIF" | "SUMIFS" | "COUNTIFS" | "AVERAGEIFS" => self.conditional(name, args)?,
            "RANK" | "RANK.EQ" => {
                arity(2, 3)?;
                let x = self.number(&args[0])?;
                let Some(b) = self.table(&args[1])? else { return Ok(T::Num(null_num())) };
                self.not_own_cells(b)?;
                let ascending = match args.get(2) {
                    Some(a) if self.is_fixed(a, 0) => self.fixed_number(a)?.is_some_and(|v| v != 0.0),
                    Some(_) => return Err("RANK with a computed order".into()),
                    None => false,
                };
                let mut values: Vec<f64> = Vec::new();
                for r in b.r1..=b.r2 {
                    for c in b.c1..=b.c2 {
                        if let (Some(v), None) = self.entry(b.sheet, r, c) {
                            if v.is_nan() {
                                return Ok(T::Num(null_num()));
                            }
                            values.push(v);
                        }
                    }
                }
                values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let n = values.len() as f64;
                let mut distinct = values.clone();
                distinct.dedup();
                let x = bind(&mut self.lets, full(x));
                let ones = Series::new("o".into(), vec![1.0f64; distinct.len()]);
                let member = x.clone().replace_strict(lit(Series::new("v".into(), distinct)), lit(ones), Some(null_num()), Some(DataType::Float64)).is_not_null();
                let sorted = lit(Series::new("v".into(), values));
                let rank = if ascending {
                    sorted.search_sorted(x.clone(), SearchSortedSide::Left, false).cast(DataType::Float64) + lit(1.0)
                } else {
                    lit(n) - sorted.search_sorted(x.clone(), SearchSortedSide::Right, false).cast(DataType::Float64) + lit(1.0)
                };
                T::Num(when(member.fill_null(lit(false))).then(rank).otherwise(null_num()))
            }
            "COUNTA" => {
                let mut count = lit(0.0);
                for a in args {
                    match core(a) {
                        // A fixed range: its cells that are not empty, counted now.
                        Fx::Binary { op: BinaryOp::Range, .. } | Fx::Name { .. } | Fx::Ref { kind: RefKind::Columns | RefKind::Rows, .. } if self.is_fixed(a, 0) && matches!(self.table(a), Ok(Some(_))) => {
                            let Ok(Some(b)) = self.table(a) else { unreachable!() };
                            self.not_own_cells(b)?;
                            let c = &self.wb.cells;
                            let present = |i: usize| !matches!(c.kind[i], Kind::Blank) || c.formula[i].is_some();
                            let held = if (b.r2 - b.r1 + 1) as u64 * (b.c2 - b.c1 + 1) as u64 <= 200_000 {
                                (b.r1..=b.r2).flat_map(|r| (b.c1..=b.c2).map(move |col| (r, col))).filter(|&(r, col)| self.peek(&(b.sheet, r - 1, col - 1)).is_some_and(|&i| present(i))).count()
                            } else {
                                (0..c.len())
                                    .filter(|&i| c.sheet[i] == b.sheet && (b.r1..=b.r2).contains(&(c.row[i] + 1)) && (b.c1..=b.c2).contains(&(c.col[i] + 1)))
                                    .filter(|&i| present(i))
                                    .count()
                            };
                            count = count + lit(held as f64);
                        }
                        Fx::Ref { .. } | Fx::Binary { op: BinaryOp::Range, .. } | Fx::Name { .. } => {
                            for cell in self.cells_of(a)? {
                                count = count + present(&cell);
                            }
                        }
                        _ => count = count + lit(1.0),
                    }
                }
                T::Num(count)
            }
            "EDATE" | "EOMONTH" => {
                arity(2, 2)?;
                let start = self.number(&args[0])?.floor();
                let start = bind(&mut self.lets, start);
                let months = self.number(&args[1])?;
                let months = bind(&mut self.lets, whole(months));
                let (y, m, d) = self.ymd(start.clone());
                let target = m + months;
                let days = if name == "EOMONTH" {
                    days_from_civil(&mut self.lets, y, target + lit(1.0), lit(1.0)) - lit(1.0)
                } else {
                    let first = days_from_civil(&mut self.lets, y.clone(), target.clone(), lit(1.0));
                    let first = bind(&mut self.lets, first);
                    let next = days_from_civil(&mut self.lets, y, target + lit(1.0), lit(1.0));
                    let length = next - first.clone();
                    let day = when(d.clone().gt(length.clone())).then(length).otherwise(d);
                    first + day - lit(1.0)
                };
                let serial = self.serial_of(days);
                let serial = bind(&mut self.lets, serial);
                T::Num(when(start.lt(lit(0.0)).or(serial.clone().lt(lit(0.0)))).then(null_num()).otherwise(serial))
            }
            "WORKDAY" | "NETWORKDAYS" => {
                arity(2, 3)?;
                let holidays = match args.get(2) {
                    None => Vec::new(),
                    Some(h) => {
                        let Some(b) = self.table(h)? else { return Ok(T::Num(null_num())) };
                        let mut days = Vec::new();
                        for r in b.r1..=b.r2 {
                            for c in b.c1..=b.c2 {
                                match self.entry(b.sheet, r, c) {
                                    (Some(v), None) if v.is_nan() => return Ok(T::Num(null_num())),
                                    (Some(v), None) => days.push(v.floor()),
                                    (None, None) => {}
                                    _ => return Err("holiday that is text".into()),
                                }
                            }
                        }
                        // Only holidays on weekdays move anything.
                        days.retain(|&d| weekday_index(d) < 5);
                        days.sort_by(|a, b| a.partial_cmp(b).unwrap());
                        days.dedup();
                        days
                    }
                };
                let start = self.number(&args[0])?.floor();
                let start = bind(&mut self.lets, start);
                let second = self.number(&args[1])?;
                if name == "NETWORKDAYS" {
                    let end = bind(&mut self.lets, second.floor());
                    let (lo, hi) = (bind(&mut self.lets, when(start.clone().lt_eq(end.clone())).then(start.clone()).otherwise(end.clone())), bind(&mut self.lets, when(start.clone().lt_eq(end.clone())).then(end.clone()).otherwise(start.clone())));
                    let mut count = weekdays_before(hi.clone() + lit(1.0)) - weekdays_before(lo.clone());
                    if !holidays.is_empty() {
                        let h = lit(Series::new("h".into(), holidays));
                        count = count - (h.clone().search_sorted(full(hi), SearchSortedSide::Right, false).cast(DataType::Float64) - h.search_sorted(full(lo), SearchSortedSide::Left, false).cast(DataType::Float64));
                    }
                    let signed = when(start.clone().lt_eq(end.clone())).then(count.clone()).otherwise(lit(0.0) - count);
                    T::Num(when(start.lt(lit(0.0)).or(end.lt(lit(0.0)))).then(null_num()).otherwise(signed))
                } else {
                    let n = bind(&mut self.lets, whole(second));
                    let mut result = bind(&mut self.lets, workday(start.clone(), n.clone()));
                    if !holidays.is_empty() {
                        let h = lit(Series::new("h".into(), holidays.clone()));
                        let before = |x: Expr, side| h.clone().search_sorted(full(x), side, false).cast(DataType::Float64);
                        // Each round moves on past the holidays the last step went over.
                        let mut from = start.clone();
                        for _ in 0..=holidays.len().min(60) {
                            let forward = n.clone().gt_eq(lit(0.0));
                            let passed = when(forward.clone())
                                .then(before(result.clone(), SearchSortedSide::Right) - before(from.clone(), SearchSortedSide::Right))
                                .otherwise(before(from.clone(), SearchSortedSide::Left) - before(result.clone(), SearchSortedSide::Left));
                            let passed = bind(&mut self.lets, passed);
                            let step = when(forward).then(passed.clone()).otherwise(lit(0.0) - passed);
                            from = result.clone();
                            result = bind(&mut self.lets, workday(result, step));
                        }
                    }
                    T::Num(when(start.lt(lit(0.0)).or(result.clone().lt(lit(0.0)))).then(null_num()).otherwise(result))
                }
            }
            "YEARFRAC" => {
                arity(2, 3)?;
                let basis = match args.get(2) {
                    None => 0,
                    Some(a) if self.is_fixed(a, 0) => match self.fixed_number(a)? {
                        Some(v) => v.trunc() as i64,
                        None => return Ok(T::Num(null_num())),
                    },
                    Some(_) => return Err("YEARFRAC with a computed basis".into()),
                };
                let a = self.number(&args[0])?.floor();
                let b = self.number(&args[1])?.floor();
                let (a, b) = (bind(&mut self.lets, a), bind(&mut self.lets, b));
                let s = bind(&mut self.lets, when(a.clone().lt_eq(b.clone())).then(a.clone()).otherwise(b.clone()));
                let e = bind(&mut self.lets, when(a.clone().lt_eq(b.clone())).then(b.clone()).otherwise(a.clone()));
                let value = match basis {
                    0 | 4 => {
                        let (y1, m1, d1) = self.ymd(s.clone());
                        let (y2, m2, d2) = self.ymd(e.clone());
                        let (d1, d2) = if basis == 4 {
                            (when(d1.clone().eq(lit(31.0))).then(lit(30.0)).otherwise(d1), when(d2.clone().eq(lit(31.0))).then(lit(30.0)).otherwise(d2))
                        } else {
                            let last_feb = |ctx: &mut Self, y: Expr, m: Expr, d: Expr| {
                                let march = days_from_civil(&mut ctx.lets, y.clone(), lit(3.0), lit(1.0));
                                let feb = days_from_civil(&mut ctx.lets, y, lit(2.0), lit(1.0));
                                m.eq(lit(2.0)).and(d.eq(march - feb))
                            };
                            let f1 = last_feb(self, y1.clone(), m1.clone(), d1.clone());
                            let f2 = last_feb(self, y2.clone(), m2.clone(), d2.clone());
                            let (f1, f2) = (bind(&mut self.lets, f1), bind(&mut self.lets, f2));
                            let both31 = d1.clone().eq(lit(31.0)).and(d2.clone().eq(lit(31.0)));
                            let nd1 = when(both31.clone()).then(lit(30.0))
                                .otherwise(when(d1.clone().eq(lit(31.0))).then(lit(30.0))
                                .otherwise(when(d1.clone().eq(lit(30.0)).and(d2.clone().eq(lit(31.0)))).then(d1.clone())
                                .otherwise(when(f1.clone().and(f2.clone())).then(lit(30.0))
                                .otherwise(when(f1.clone()).then(lit(30.0)).otherwise(d1.clone())))));
                            let nd2 = when(both31).then(lit(30.0))
                                .otherwise(when(d1.clone().eq(lit(31.0))).then(d2.clone())
                                .otherwise(when(d1.eq(lit(30.0)).and(d2.clone().eq(lit(31.0)))).then(lit(30.0))
                                .otherwise(when(f1.and(f2)).then(lit(30.0)).otherwise(d2))));
                            (nd1, nd2)
                        };
                        ((y2 - y1) * lit(360.0) + (m2 - m1) * lit(30.0) + (d2 - d1)) / lit(360.0)
                    }
                    2 => (e.clone() - s.clone()) / lit(360.0),
                    3 => (e.clone() - s.clone()) / lit(365.0),
                    _ => return Err(format!("YEARFRAC basis {basis}")),
                };
                T::Num(when(a.lt(lit(0.0)).or(b.lt(lit(0.0)))).then(null_num()).otherwise(value))
            }
            "TEXT" => {
                arity(2, 2)?;
                let format = match core(&args[1]) {
                    Fx::Str(s) => unquote(s),
                    _ => return Err("TEXT with a computed format".into()),
                };
                let value = self.translate(&args[0])?.bound(&mut self.lets);
                let original = match &value {
                    T::Text(t) => Some(t.clone()),
                    T::Dyn(Dyn { text: Some(t), .. }) => Some(t.clone()),
                    _ => None,
                };
                let x = bind(&mut self.lets, value.num());
                let formatted = if format.contains(['0', '#']) { self.number_format(x.clone(), &format)? } else { self.date_text(x.clone(), &format)? };
                let dates = !format.contains(['0', '#']);
                // Text that is no number stays as it is.
                let result = match original {
                    Some(t) => when(x.clone().is_null().and(t.clone().is_not_null())).then(t).otherwise(formatted),
                    None => formatted,
                };
                T::Text(if dates { when(x.clone().lt(lit(0.0))).then(null_text()).otherwise(result) } else { result })
            }
            "DOLLAR" => {
                arity(1, 2)?;
                let decimals = match args.get(1) {
                    None => 2,
                    Some(a) if self.is_fixed(a, 0) => match self.fixed_number(a)? {
                        Some(v) if (0.0..=15.0).contains(&v.trunc()) => v.trunc() as usize,
                        Some(_) => return Err("DOLLAR with negative or many decimals".into()),
                        None => return Ok(T::Text(null_text())),
                    },
                    Some(_) => return Err("DOLLAR with computed decimals".into()),
                };
                let x = self.number(&args[0])?;
                let pattern = if decimals == 0 { "$#,##0".to_string() } else { format!("$#,##0.{}", "0".repeat(decimals)) };
                let shown = self.number_format(x.clone().abs(), &pattern)?;
                let rounded_zero = (x.clone().abs() * lit(10f64.powi(decimals as i32)) + lit(0.5)).floor().eq(lit(0.0));
                T::Text(when(x.lt(lit(0.0)).and(rounded_zero.not())).then(lit("(") + shown.clone() + lit(")")).otherwise(shown))
            }
            "DATEVALUE" => {
                arity(1, 1)?;
                let text = match self.translate(&args[0])? {
                    T::Text(t) => t,
                    T::Dyn(Dyn { text: Some(t), .. }) => t,
                    _ => return Ok(T::Num(null_num())),
                };
                let text = bind(&mut self.lets, text.str().strip_chars(lit(NULL)));
                T::Num(self.date_value(text))
            }
            "PMT" | "PV" | "FV" => {
                arity(3, 5)?;
                let rate = self.number(&args[0])?;
                let n = self.number(&args[1])?;
                let third = self.number(&args[2])?;
                let fourth = match args.get(3).map(core) {
                    None | Some(Fx::Missing) => lit(0.0),
                    Some(_) => self.number(&args[3])?,
                };
                let kind = match args.get(4).map(core) {
                    None | Some(Fx::Missing) => lit(0.0),
                    Some(_) => {
                        let k = self.number(&args[4])?;
                        when(k.clone().neq(lit(0.0))).then(lit(1.0)).otherwise(lit(0.0))
                    }
                };
                let growth = bind(&mut self.lets, (lit(1.0) + rate.clone()).pow(n.clone()));
                let zero = rate.clone().eq(lit(0.0));
                let value = match name {
                    // PMT(rate, nper, pv, fv, type)
                    "PMT" => when(zero).then(lit(0.0) - (third.clone() + fourth.clone()) / n.clone()).otherwise(
                        lit(0.0) - rate.clone() * (fourth.clone() + third.clone() * growth.clone()) / ((lit(1.0) + rate.clone() * kind.clone()) * (growth.clone() - lit(1.0))),
                    ),
                    // PV(rate, nper, pmt, fv, type)
                    "PV" => when(zero).then(lit(0.0) - (fourth.clone() + third.clone() * n.clone())).otherwise(
                        lit(0.0) - (fourth.clone() + third.clone() * (lit(1.0) + rate.clone() * kind.clone()) * (growth.clone() - lit(1.0)) / rate.clone()) / growth.clone(),
                    ),
                    // FV(rate, nper, pmt, pv, type)
                    _ => when(zero).then(lit(0.0) - (fourth.clone() + third.clone() * n.clone())).otherwise(
                        lit(0.0) - (fourth.clone() * growth.clone() + third.clone() * (lit(1.0) + rate.clone() * kind) * (growth - lit(1.0)) / rate),
                    ),
                };
                T::Num(when(value.clone().is_finite()).then(value).otherwise(null_num()))
            }
            "SLN" => {
                arity(3, 3)?;
                let (cost, salvage) = (self.number(&args[0])?, self.number(&args[1])?);
                let life = self.number(&args[2])?;
                T::Num(divide(&mut self.lets, cost - salvage, life))
            }
            "NPV" => {
                if args.len() < 2 {
                    return Err("NPV without values".into());
                }
                let rate = self.number(&args[0])?;
                let rate = bind(&mut self.lets, rate);
                let mut values = Vec::new();
                for a in &args[1..] {
                    values.extend(self.cells_of(a)?);
                }
                let mut sum = lit(0.0);
                let mut error = lit(false);
                let mut position = lit(0.0);
                for v in values {
                    // Empty cells and text are passed over; each number counts one period.
                    let counts = bind(&mut self.lets, is_number(&v).cast(DataType::Float64));
                    position = bind(&mut self.lets, position + counts.clone());
                    let (number, err) = array_number(&mut self.lets, v);
                    sum = sum + counts * number / (lit(1.0) + rate.clone()).pow(position.clone());
                    error = error.or(err);
                }
                T::Num(when(error.or(rate.eq(lit(-1.0)))).then(null_num()).otherwise(sum))
            }
            "XNPV" | "XIRR" => {
                let (values, dates, rest) = if name == "XNPV" {
                    arity(3, 3)?;
                    (&args[1], &args[2], Some(&args[0]))
                } else {
                    arity(2, 3)?;
                    (&args[0], &args[1], args.get(2))
                };
                let values = self.cells_of(values)?;
                let dates = self.cells_of(dates)?;
                if values.len() != dates.len() || values.is_empty() {
                    return Ok(T::Num(null_num()));
                }
                let mut error = lit(false);
                let (mut v, mut d) = (Vec::new(), Vec::new());
                for (x, t) in values.into_iter().zip(dates) {
                    let (xn, xe) = (bind(&mut self.lets, x.clone().num()), x.is_error());
                    let tn = t.clone().num();
                    let tn = bind(&mut self.lets, tn.floor());
                    error = error.or(xe).or(xn.clone().is_null()).or(tn.clone().is_null()).or(t.is_error());
                    v.push(xn);
                    d.push(tn);
                }
                let error = bind(&mut self.lets, error);
                let span: Vec<Expr> = d.iter().map(|t| (t.clone() - d[0].clone()) / lit(365.0)).collect();
                let span: Vec<Expr> = span.into_iter().map(|e| bind(&mut self.lets, e)).collect();
                let npv = |lets: &mut Lets, r: Expr| -> (Expr, Expr) {
                    let r = bind(lets, r);
                    let (mut f, mut g) = (lit(0.0), lit(0.0));
                    for (x, t) in v.iter().zip(&span) {
                        let disc = (lit(1.0) + r.clone()).pow(lit(0.0) - t.clone());
                        f = f + x.clone() * disc.clone();
                        g = g - t.clone() * x.clone() * disc / (lit(1.0) + r.clone());
                    }
                    (bind(lets, f), bind(lets, g))
                };
                if name == "XNPV" {
                    let rate = self.number(rest.unwrap())?;
                    let (f, _) = npv(&mut self.lets, rate.clone());
                    T::Num(when(error.or(rate.lt_eq(lit(-1.0)))).then(null_num()).otherwise(f))
                } else {
                    // Newton's method from the guess (0.1), as Excel; no settling is #NUM!.
                    let mut r = match rest {
                        Some(g) => self.number(g)?,
                        None => lit(0.1),
                    };
                    let mut last = r.clone();
                    for _ in 0..60 {
                        let (f, g) = npv(&mut self.lets, r.clone());
                        last = bind(&mut self.lets, r.clone());
                        r = bind(&mut self.lets, last.clone() - f / g);
                    }
                    let settled = (r.clone() - last).abs().lt(lit(1e-9) * (r.clone().abs() + lit(1e-9)));
                    T::Num(when(error.or(settled.not().fill_null(lit(true))).or(r.clone().is_finite().not())).then(null_num()).otherwise(r))
                }
            }
            "CORREL" | "PEARSON" => {
                arity(2, 2)?;
                let (xs, ys) = (self.cells_of(&args[0])?, self.cells_of(&args[1])?);
                if xs.len() != ys.len() {
                    return Ok(T::Num(null_num()));
                }
                let mut pairs = Vec::new();
                let mut error = lit(false);
                for (x, y) in xs.into_iter().zip(ys) {
                    let both = bind(&mut self.lets, is_number(&x).and(is_number(&y)));
                    error = error.or(x.is_error()).or(y.is_error());
                    let xv = bind(&mut self.lets, when(both.clone()).then(x.num()).otherwise(lit(0.0)));
                    let yv = bind(&mut self.lets, when(both.clone()).then(y.num()).otherwise(lit(0.0)));
                    pairs.push((both.cast(DataType::Float64), xv, yv));
                }
                let n = bind(&mut self.lets, pairs.iter().fold(lit(0.0), |a, p| a + p.0.clone()));
                let mx = bind(&mut self.lets, pairs.iter().fold(lit(0.0), |a, p| a + p.1.clone()) / n.clone());
                let my = bind(&mut self.lets, pairs.iter().fold(lit(0.0), |a, p| a + p.2.clone()) / n.clone());
                let (mut sxy, mut sxx, mut syy) = (lit(0.0), lit(0.0), lit(0.0));
                for (m, x, y) in &pairs {
                    let (dx, dy) = ((x.clone() - mx.clone()) * m.clone(), (y.clone() - my.clone()) * m.clone());
                    sxy = sxy + dx.clone() * dy.clone();
                    sxx = sxx + dx.clone() * dx;
                    syy = syy + dy.clone() * dy;
                }
                let denominator = bind(&mut self.lets, (sxx * syy).pow(lit(0.5)));
                T::Num(when(error.or(denominator.clone().eq(lit(0.0))).or(n.lt(lit(1.0)))).then(null_num()).otherwise(sxy / denominator))
            }
            "MAXA" | "MINA" => {
                let mut items = Vec::new();
                for a in args {
                    for cell in self.cells_of(a)? {
                        let value = match &cell {
                            T::Dyn(d) => {
                                let text = d.text.clone().map(|t| t.is_not_null()).unwrap_or(lit(false));
                                when(text).then(lit(0.0)).otherwise(d.num.clone())
                            }
                            T::Text(_) => lit(0.0),
                            other => other.clone().num(),
                        };
                        items.push(value);
                    }
                }
                if items.is_empty() {
                    return Err(format!("{name} without arguments"));
                }
                let error = items.iter().fold(lit(false), |a, x| a.or(x.clone().is_nan().fill_null(lit(false))));
                let combined = if name == "MAXA" { max_horizontal(items) } else { min_horizontal(items) };
                T::Num(when(error).then(null_num()).otherwise(combined.map_err(|e| e.to_string())?.fill_null(lit(0.0))))
            }
            "DSUM" | "DCOUNT" | "DAVERAGE" | "DMAX" | "DMIN" | "DCOUNTA" | "DPRODUCT" => {
                arity(3, 3)?;
                let Some(db) = self.table(&args[0])? else { return Ok(T::Num(null_num())) };
                let Some(criteria) = self.table(&args[2])? else { return Ok(T::Num(null_num())) };
                self.not_own_cells(db)?;
                self.not_own_cells(criteria)?;
                let Some(rows) = self.database_rows(db, criteria)? else { return Ok(T::Num(null_num())) };
                // Per field (column of the database): the result over the rows that meet the criteria.
                let mut by_index = Vec::new();
                let mut by_header = Vec::new();
                for c in db.c1..=db.c2 {
                    let mut numbers = Vec::new();
                    let mut present = 0.0;
                    let mut error = false;
                    for &r in &rows {
                        match self.entry(db.sheet, r, c) {
                            (Some(v), None) if v.is_nan() => error = true,
                            (Some(v), None) => {
                                numbers.push(v);
                                present += 1.0;
                            }
                            (None, Some(_)) => present += 1.0,
                            _ => {}
                        }
                    }
                    let value = match name {
                        "DSUM" if !error => Some(numbers.iter().sum()),
                        "DCOUNT" => Some(numbers.len() as f64),
                        "DCOUNTA" => Some(present),
                        "DAVERAGE" if !error && !numbers.is_empty() => Some(numbers.iter().sum::<f64>() / numbers.len() as f64),
                        "DMAX" if !error => Some(numbers.iter().cloned().fold(f64::NEG_INFINITY, f64::max)).map(|v| if v.is_finite() { v } else { 0.0 }),
                        "DMIN" if !error => Some(numbers.iter().cloned().fold(f64::INFINITY, f64::min)).map(|v| if v.is_finite() { v } else { 0.0 }),
                        "DPRODUCT" if !error => Some(if numbers.is_empty() { 0.0 } else { numbers.iter().product() }),
                        _ => None,
                    };
                    by_index.push(((c - db.c1 + 1) as f64, value));
                    if let Key::Text(h) = self.key(db.sheet, db.r1, c) {
                        if !by_header.iter().any(|(k, _): &(String, Option<f64>)| *k == h) {
                            by_header.push((h, value));
                        }
                    }
                }
                let field = self.translate(&args[1])?.bound(&mut self.lets);
                let (fn_, ft) = match field {
                    T::Num(e) => (Some(e), None),
                    T::Text(t) => (None, Some(t)),
                    T::Dyn(d) => (Some(d.num.clone()), d.text.clone()),
                    T::Bool(_) => return Err("database field that is a truth value".into()),
                };
                let mut result = null_num();
                if let Some(n) = fn_ {
                    let keys = Series::new("k".into(), by_index.iter().map(|x| x.0).collect::<Vec<f64>>());
                    let vals = Series::new("v".into(), by_index.iter().map(|x| x.1).collect::<Vec<Option<f64>>>());
                    let looked = full(n.clone().floor()).replace_strict(lit(keys), lit(vals), Some(null_num()), Some(DataType::Float64));
                    result = when(n.is_not_null()).then(looked).otherwise(result);
                }
                if let (Some(t), false) = (ft, by_header.is_empty()) {
                    let keys = Series::new("k".into(), by_header.iter().map(|x| x.0.clone()).collect::<Vec<String>>());
                    let vals = Series::new("v".into(), by_header.iter().map(|x| x.1).collect::<Vec<Option<f64>>>());
                    let looked = full(t.clone().str().to_lowercase().str().strip_chars(lit(NULL))).replace_strict(lit(keys), lit(vals), Some(null_num()), Some(DataType::Float64));
                    result = when(t.is_not_null()).then(looked).otherwise(result);
                }
                T::Num(bind(&mut self.lets, result))
            }
            "SUBTOTAL" => {
                if args.len() < 2 {
                    return Err("SUBTOTAL without values".into());
                }
                if !self.is_fixed(&args[0], 0) {
                    return Err("SUBTOTAL with a computed function number".into());
                }
                let code = self.fixed_number(&args[0])?.map(|v| v.trunc() as i64);
                let function = match code {
                    Some(1) => "AVERAGE",
                    Some(2) => "COUNT",
                    Some(3) => "COUNTA",
                    Some(4) => "MAX",
                    Some(5) => "MIN",
                    Some(6) => "PRODUCT",
                    Some(7) => "STDEV",
                    Some(8) => "STDEVP",
                    Some(9) => "SUM",
                    Some(10) => "VAR",
                    Some(11) => "VARP",
                    Some(101..=111) => return Err("SUBTOTAL that leaves out hidden rows".into()),
                    _ => return Ok(T::Num(null_num())),
                };
                // SUBTOTAL leaves out rows a filter hides (with every function number);
                // which hidden rows a filter hid the file does not say: refuse them all.
                for a in &args[1..] {
                    let sheet = self.table(a)?.map(|b| (b.sheet, Some((b.r1, b.r2)))).unwrap_or((self.sheet, None));
                    let hidden = self.wb.sheets.get(sheet.0 as usize).is_some_and(|s| {
                        s.rows.iter().any(|r| r.hidden && sheet.1.is_none_or(|(r1, r2)| r.row + 1 >= r1 && r.row < r2))
                    });
                    if hidden {
                        return Err("SUBTOTAL over hidden rows (a filter leaves them out)".into());
                    }
                }
                // SUBTOTAL passes over cells that hold SUBTOTAL themselves: a fixed range
                // with such cells is taken as its other cells one by one.
                let mut refs = Vec::new();
                for a in &args[1..] {
                    if !self.holds_subtotal(a)? {
                        refs.push(a.clone());
                        continue;
                    }
                    let Some(b) = self.table(a)? else { return Err("SUBTOTAL over cells that hold SUBTOTAL".into()) };
                    // The stretches of each column between SUBTOTAL cells, as ranges.
                    let sheet_name = format!("'{}'!", self.wb.sheets[b.sheet as usize].name.replace('\'', "''"));
                    let range = |c: u32, from: u32, to: u32| {
                        let cell = |r: u32| Fx::Ref { prefix: None, kind: RefKind::Cell, text: format!("${}${}", xl_formula::col_name(c), r) };
                        let Fx::Ref { text, .. } = cell(from) else { unreachable!() };
                        Fx::Binary { op: BinaryOp::Range, op_text: ":".into(), lhs: Box::new(Fx::Ref { prefix: Some(sheet_name.clone()), kind: RefKind::Cell, text }), rhs: Box::new(cell(to)) }
                    };
                    let before = refs.len();
                    for c in b.c1..=b.c2 {
                        let mut start = None;
                        for r in b.r1..=b.r2 + 1 {
                            let skip = r > b.r2 || self.peek(&(b.sheet, r - 1, c - 1)).is_some_and(|&i| self.wb.cells.formula[i].as_deref().is_some_and(|f| f.to_ascii_uppercase().contains("SUBTOTAL(")));
                            match (skip, start) {
                                (false, None) => start = Some(r),
                                (true, Some(from)) => {
                                    refs.push(range(c, from, r - 1));
                                    start = None;
                                }
                                _ => {}
                            }
                        }
                    }
                    if refs.len() - before > 5_000 {
                        return Err("SUBTOTAL over a large range with SUBTOTAL cells".into());
                    }
                }
                if refs.is_empty() {
                    return Ok(T::Num(lit(0.0)));
                }
                let inner = Fx::Call { name: function.into(), args: refs };
                self.translate(&inner)?
            }
            "PRODUCT" => {
                let mut product = lit(1.0);
                let mut error = lit(false);
                let mut any = lit(false);
                for a in args {
                    for cell in self.cells_of(a)? {
                        let n = is_number(&cell);
                        let n = bind(&mut self.lets, n);
                        error = error.or(cell.is_error());
                        product = product * when(n.clone()).then(cell.num()).otherwise(lit(1.0));
                        any = any.or(n);
                    }
                }
                T::Num(when(error).then(null_num()).otherwise(when(any).then(product).otherwise(lit(0.0))))
            }
            "IRR" => {
                arity(1, 2)?;
                let values = self.cells_of(&args[0])?;
                let mut v = Vec::new();
                let mut error = lit(false);
                for x in values {
                    error = error.or(x.is_error());
                    let n = bind(&mut self.lets, is_number(&x));
                    let value = bind(&mut self.lets, when(n.clone()).then(x.num()).otherwise(lit(0.0)));
                    v.push((n, value));
                }
                // Periods count numbers only.
                let mut periods = Vec::new();
                let mut t = lit(0.0);
                for (n, _) in &v {
                    periods.push(t.clone());
                    t = bind(&mut self.lets, t + n.clone().cast(DataType::Float64));
                }
                let mut r = match args.get(1) {
                    Some(g) => self.number(g)?,
                    None => lit(0.1),
                };
                let mut last = r.clone();
                for _ in 0..60 {
                    let rr = bind(&mut self.lets, r.clone());
                    let (mut f, mut g) = (lit(0.0), lit(0.0));
                    for ((n, x), p) in v.iter().zip(&periods) {
                        let disc = (lit(1.0) + rr.clone()).pow(lit(0.0) - p.clone());
                        let x = when(n.clone()).then(x.clone()).otherwise(lit(0.0));
                        f = f + x.clone() * disc.clone();
                        g = g - p.clone() * x * disc / (lit(1.0) + rr.clone());
                    }
                    last = rr.clone();
                    r = bind(&mut self.lets, rr - f / g);
                }
                let settled = (r.clone() - last).abs().lt(lit(1e-9) * (r.clone().abs() + lit(1e-9)));
                T::Num(when(error.or(settled.not().fill_null(lit(true))).or(r.clone().is_finite().not())).then(null_num()).otherwise(r))
            }
            "SUMSQ" | "AVERAGEA" => {
                let mut sum = lit(0.0);
                let mut count = lit(0.0);
                let mut error = lit(false);
                for a in args {
                    let reference = matches!(core(a), Fx::Ref { .. } | Fx::Name { .. } | Fx::Binary { op: BinaryOp::Range, .. });
                    for cell in self.cells_of(a)? {
                        error = error.or(cell.is_error());
                        if name == "SUMSQ" {
                            let n = bind(&mut self.lets, is_number(&cell));
                            let v = when(n).then(cell.num()).otherwise(lit(0.0));
                            sum = sum + v.clone() * v;
                        } else {
                            // AVERAGEA: text in a reference counts as 0, empty cells not at all.
                            let (value, counts) = match &cell {
                                T::Dyn(d) => {
                                    let text = d.text.clone().map(|t| t.is_not_null()).unwrap_or(lit(false));
                                    let n = bind(&mut self.lets, is_number(&cell));
                                    (when(n.clone()).then(d.num.clone()).otherwise(lit(0.0)), n.or(text).cast(DataType::Float64))
                                }
                                T::Text(_) if reference => (lit(0.0), lit(1.0)),
                                other => (other.clone().num(), lit(1.0)),
                            };
                            sum = sum + value;
                            count = count + counts;
                        }
                    }
                }
                let result = if name == "SUMSQ" { sum } else { divide(&mut self.lets, sum, count) };
                T::Num(when(error).then(null_num()).otherwise(result))
            }
            "TRIM" => {
                arity(1, 1)?;
                let t = self.string(&args[0])?;
                T::Text(t.str().strip_chars(lit(" ")).str().replace_all(lit(" {2,}"), lit(" "), false))
            }
            "CHAR" => {
                arity(1, 1)?;
                let n = self.number(&args[0])?.floor();
                let codes: Vec<f64> = (1..=255).map(|c| c as f64).collect();
                let chars: Vec<String> = (1u32..=255).map(windows_1252).collect();
                T::Text(full(n).replace_strict(lit(Series::new("k".into(), codes)), lit(Series::new("v".into(), chars)), Some(null_text()), Some(DataType::String)))
            }
            "ADDRESS" => {
                arity(2, 5)?;
                let row = self.number(&args[0])?.floor();
                let column = self.number(&args[1])?.floor();
                let kind = match args.get(2).map(core) {
                    None | Some(Fx::Missing) => 1,
                    Some(a) if self.is_fixed(a, 0) => self.fixed_number(a)?.map(|v| v.trunc() as i64).unwrap_or(0),
                    Some(_) => return Err("ADDRESS with a computed kind".into()),
                };
                if let Some(a) = args.get(3).filter(|a| !matches!(core(a), Fx::Missing)) {
                    if !self.is_fixed(a, 0) || self.fixed_number(a)?.is_some_and(|v| v == 0.0) {
                        return Err("ADDRESS in R1C1 form".into());
                    }
                }
                if !(1..=4).contains(&kind) {
                    return Ok(T::Text(null_text()));
                }
                let (row, column) = (bind(&mut self.lets, row), bind(&mut self.lets, column));
                let numbers: Vec<f64> = (1..=16_384).map(|c| c as f64).collect();
                let letters: Vec<String> = (1..=16_384u32).map(xl_formula::col_name).collect();
                let letters = full(column.clone()).replace_strict(lit(Series::new("k".into(), numbers)), lit(Series::new("v".into(), letters)), Some(null_text()), Some(DataType::String));
                let dollar = |on: bool| lit(if on { "$" } else { "" });
                let mut text = dollar(kind == 1 || kind == 3) + letters + dollar(kind <= 2) + row.clone().cast(DataType::Int64).cast(DataType::String);
                if let Some(a) = args.get(4).filter(|a| !matches!(core(a), Fx::Missing)) {
                    let sheet = self.string(a)?;
                    let quoted = when(sheet.clone().str().contains(lit(r"[^A-Za-z0-9_.]"), false)).then(lit("'") + sheet.clone().str().replace_all(lit("'"), lit("''"), true) + lit("'")).otherwise(sheet);
                    text = quoted + lit("!") + text;
                }
                let valid = row.clone().gt_eq(lit(1.0)).and(row.lt_eq(lit(1_048_576.0))).and(column.clone().gt_eq(lit(1.0))).and(column.lt_eq(lit(16_384.0)));
                T::Text(when(valid).then(text).otherwise(null_text()))
            }
            "CELL" => {
                arity(1, 2)?;
                let info = match core(&args[0]) {
                    Fx::Str(s) => unquote(s).to_ascii_lowercase(),
                    _ => return Err("CELL with a computed kind of information".into()),
                };
                let Some(r) = args.get(1) else { return Err("CELL about the cell last changed".into()) };
                let (rows, cols) = match core(r) {
                    Fx::Ref { kind: RefKind::Cell, text, .. } => {
                        let c = parse_cell_ref(text).ok_or("reference form")?;
                        (if c.abs_row { lit(c.row as f64) } else { col(ROW).cast(DataType::Float64) + lit(c.row as f64 - self.row as f64) }, c.col)
                    }
                    _ => return Err("CELL of a range".into()),
                };
                match info.as_str() {
                    "col" => T::Num(lit(cols as f64)),
                    "row" => T::Num(rows),
                    // On another sheet the address names the workbook file too.
                    "address" if matches!(core(r), Fx::Ref { prefix: Some(_), .. }) => return Err("CELL(\"address\") of another sheet names the file".into()),
                    "address" => T::Text(lit(format!("${}$", xl_formula::col_name(cols))) + rows.cast(DataType::Int64).cast(DataType::String)),
                    "contents" => self.translate(r)?,
                    "filename" => return Err("CELL(\"filename\"): the value is where the file was stored".into()),
                    other => return Err(format!("CELL(\"{other}\")")),
                }
            }
            "NORMDIST" | "NORM.DIST" | "NORMSDIST" | "NORM.S.DIST" => {
                let standard = name.starts_with("NORMS") || name.starts_with("NORM.S");
                let (x, mean, sd, cumulative) = if standard {
                    arity(1, 2)?;
                    let cumulative = match args.get(1) {
                        None => true,
                        Some(a) if self.is_fixed(a, 0) => self.fixed_number(a)?.is_some_and(|v| v != 0.0),
                        Some(_) => return Err("NORMSDIST with a computed kind".into()),
                    };
                    (self.number(&args[0])?, lit(0.0), lit(1.0), cumulative)
                } else {
                    arity(4, 4)?;
                    let cumulative = match core(&args[3]) {
                        Fx::Bool(b) => b.eq_ignore_ascii_case("TRUE"),
                        Fx::Call { name: f, args: none } if none.is_empty() => f.eq_ignore_ascii_case("TRUE"),
                        a if self.is_fixed(a, 0) => self.fixed_number(a)?.is_some_and(|v| v != 0.0),
                        _ => return Err("NORMDIST with a computed kind".into()),
                    };
                    (self.number(&args[0])?, self.number(&args[1])?, self.number(&args[2])?, cumulative)
                };
                let sd = bind(&mut self.lets, sd);
                let z = bind(&mut self.lets, (x - mean) / sd.clone());
                let value = if cumulative {
                    self.normal_cdf(z)
                } else {
                    (lit(0.0) - z.clone() * z / lit(2.0)).exp() / (sd.clone() * lit((2.0 * std::f64::consts::PI).sqrt()))
                };
                T::Num(when(sd.lt_eq(lit(0.0))).then(null_num()).otherwise(value))
            }
            "IPMT" | "PPMT" => {
                arity(4, 6)?;
                let rate = self.number(&args[0])?;
                let per = self.number(&args[1])?;
                let n = self.number(&args[2])?;
                let pv = self.number(&args[3])?;
                let fv = match args.get(4).map(core) {
                    None | Some(Fx::Missing) => lit(0.0),
                    Some(_) => self.number(&args[4])?,
                };
                let kind = match args.get(5).map(core) {
                    None | Some(Fx::Missing) => lit(0.0),
                    Some(_) => {
                        let k = self.number(&args[5])?;
                        when(k.neq(lit(0.0))).then(lit(1.0)).otherwise(lit(0.0))
                    }
                };
                let (rate, per, n, pv, fv, kind) = (bind(&mut self.lets, rate), bind(&mut self.lets, per), bind(&mut self.lets, n), bind(&mut self.lets, pv), bind(&mut self.lets, fv), bind(&mut self.lets, kind));
                let growth = |k: Expr| (lit(1.0) + rate.clone()).pow(k);
                let pmt = bind(
                    &mut self.lets,
                    when(rate.clone().eq(lit(0.0))).then(lit(0.0) - (pv.clone() + fv.clone()) / n.clone()).otherwise(
                        lit(0.0) - rate.clone() * (fv.clone() + pv.clone() * growth(n.clone())) / ((lit(1.0) + rate.clone() * kind.clone()) * (growth(n.clone()) - lit(1.0))),
                    ),
                );
                // The balance after k periods (FV of the loan so far).
                let balance = |k: Expr, t: Expr| lit(0.0) - (pv.clone() * growth(k.clone()) + pmt.clone() * (lit(1.0) + rate.clone() * t) * (growth(k) - lit(1.0)) / rate.clone());
                let interest = when(kind.clone().eq(lit(0.0)))
                    .then(balance(per.clone() - lit(1.0), lit(0.0)) * rate.clone())
                    .otherwise(when(per.clone().eq(lit(1.0))).then(lit(0.0)).otherwise((balance(per.clone() - lit(2.0), lit(1.0)) - pmt.clone()) * rate.clone()));
                let interest = bind(&mut self.lets, when(rate.clone().eq(lit(0.0))).then(lit(0.0)).otherwise(interest));
                let value = if name == "IPMT" { interest } else { pmt - interest };
                let bad = per.clone().lt(lit(1.0)).or(per.gt(n));
                T::Num(when(bad).then(null_num()).otherwise(value))
            }
            "GROWTH" => {
                arity(1, 3)?;
                let ys = self.cells_of(&args[0])?;
                let xs = match args.get(1).filter(|a| !matches!(core(a), Fx::Missing)) {
                    Some(a) => self.cells_of(a)?,
                    None => (1..=ys.len()).map(|k| T::Num(lit(k as f64))).collect(),
                };
                if xs.len() != ys.len() || ys.is_empty() {
                    return Ok(T::Num(null_num()));
                }
                let new_x = match args.get(2) {
                    Some(a) => self.number(a)?,
                    None => return Err("GROWTH without new x".into()),
                };
                let mut error = lit(false);
                let mut pts = Vec::new();
                for (x, y) in xs.into_iter().zip(ys) {
                    error = error.or(x.is_error()).or(y.is_error()).or(is_number(&x).not()).or(is_number(&y).not());
                    let yv = bind(&mut self.lets, y.num());
                    error = error.or(yv.clone().lt_eq(lit(0.0)));
                    let ly = bind(&mut self.lets, when(yv.clone().gt(lit(0.0))).then(yv.log(lit(std::f64::consts::E))).otherwise(lit(0.0)));
                    pts.push((bind(&mut self.lets, x.num()), ly));
                }
                let n = pts.len() as f64;
                let mx = bind(&mut self.lets, pts.iter().fold(lit(0.0), |a, p| a + p.0.clone()) / lit(n));
                let my = bind(&mut self.lets, pts.iter().fold(lit(0.0), |a, p| a + p.1.clone()) / lit(n));
                let (mut sxy, mut sxx) = (lit(0.0), lit(0.0));
                for (x, ly) in &pts {
                    let dx = x.clone() - mx.clone();
                    sxy = sxy + dx.clone() * (ly.clone() - my.clone());
                    sxx = sxx + dx.clone() * dx;
                }
                let slope = bind(&mut self.lets, sxy / sxx);
                let value = (my - slope.clone() * mx + slope * new_x).exp();
                T::Num(when(error.fill_null(lit(true))).then(null_num()).otherwise(value))
            }
            "PERCENTILE" | "PERCENTILE.INC" => {
                arity(2, 2)?;
                let Some(b) = self.table(&args[0])? else { return Ok(T::Num(null_num())) };
                self.not_own_cells(b)?;
                let mut values = Vec::new();
                for r in b.r1..=b.r2 {
                    for c in b.c1..=b.c2 {
                        match self.entry(b.sheet, r, c) {
                            (Some(v), None) if v.is_nan() => return Ok(T::Num(null_num())),
                            (Some(v), None) => values.push(v),
                            _ => {}
                        }
                    }
                }
                if values.is_empty() {
                    return Ok(T::Num(null_num()));
                }
                values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let k = self.number(&args[1])?;
                let k = bind(&mut self.lets, k);
                let h = bind(&mut self.lets, k.clone() * lit(values.len() as f64 - 1.0));
                let i = bind(&mut self.lets, h.clone().floor());
                let idx: Vec<f64> = (0..values.len()).map(|v| v as f64).collect();
                let next: Vec<f64> = (0..values.len()).map(|v| values[(v + 1).min(values.len() - 1)]).collect();
                let at = full(i.clone()).replace_strict(lit(Series::new("i".into(), idx.clone())), lit(Series::new("v".into(), values.clone())), Some(null_num()), Some(DataType::Float64));
                let after = full(i.clone()).replace_strict(lit(Series::new("i".into(), idx)), lit(Series::new("w".into(), next)), Some(null_num()), Some(DataType::Float64));
                let at = bind(&mut self.lets, at);
                let value = at.clone() + (h - i) * (after - at);
                T::Num(when(k.clone().lt(lit(0.0)).or(k.gt(lit(1.0)))).then(null_num()).otherwise(value))
            }
            "VDB" | "DDB" => {
                // Worked out at translation from fixed arguments (in a single cell every
                // reference is fixed): the method loops over periods.
                arity(if name == "VDB" { 5 } else { 4 }, if name == "VDB" { 7 } else { 5 })?;
                let mut v = Vec::new();
                for a in args {
                    if matches!(core(a), Fx::Missing) {
                        v.push(None);
                        continue;
                    }
                    if !self.is_fixed(a, 0) {
                        return Err(format!("{name} with arguments that change with the row"));
                    }
                    match self.fixed_number(a)? {
                        Some(x) => v.push(Some(x)),
                        None => return Ok(T::Num(null_num())),
                    }
                }
                let get = |k: usize, d: f64| v.get(k).copied().flatten().unwrap_or(d);
                let value = if name == "DDB" {
                    ddb(get(0, 0.0), get(1, 0.0), get(2, 0.0), get(3, 0.0), get(4, 2.0))
                } else {
                    vdb(get(0, 0.0), get(1, 0.0), get(2, 0.0), get(3, 0.0), get(4, 0.0), get(5, 2.0), get(6, 0.0) != 0.0)
                };
                T::Num(value.map(lit).unwrap_or_else(null_num))
            }
            "DDE" => return Err("live data link (DDE): the value comes from another program".into()),
            "TODAY" | "NOW" | "RAND" | "RANDBETWEEN" | "RANDARRAY" => return Err("volatile function: the value depends on the moment or on chance; it cannot be checked against the file".into()),
            "TABLE" => return Err("data table: each cell recalculates the model with another input".into()),
            other => return Err(format!("function {other}")),
        })
    }

    /// The cells of a reference, one translated value per cell, row by row.
    pub(super) fn cells_of(&mut self, e: &Fx) -> Lifted<Vec<T>> {
        // Read as values that go into the translation: the cells are constants, noted as read.
        let was = self.param.replace(false);
        let r = self.cells_of_in(e);
        self.param.set(was);
        r
    }

    fn cells_of_in(&mut self, e: &Fx) -> Lifted<Vec<T>> {
        self.baked.set(self.baked.get() | BAKED_BLOCK);
        match core(e) {
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if matches!((core(lhs), core(rhs)), (Fx::Ref { kind: RefKind::Cell, .. }, Fx::Ref { kind: RefKind::Cell, prefix: None, .. })) => {
                let (Fx::Ref { prefix, text: a, .. }, Fx::Ref { text: b, .. }) = (core(lhs), core(rhs)) else { unreachable!() };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                if p.abs_row != q.abs_row {
                    return Err("range fixed at one end in an array function".into());
                }
                let (r1, r2, c1, c2) = (p.row.min(q.row), p.row.max(q.row), p.col.min(q.col), p.col.max(q.col));
                if (r2 - r1 + 1) as u64 * (c2 - c1 + 1) as u64 > 2_000 {
                    return Err("large range in an array function".into());
                }
                if p.abs_row {
                    self.not_own_cells(Block { sheet, r1, r2, c1, c2 })?;
                }
                let mut out = Vec::new();
                for r in r1..=r2 {
                    for c in c1..=c2 {
                        out.push(if p.abs_row { self.constant(sheet, r, c)? } else { T::Dyn(self.column(sheet, c, r as i64 - self.row as i64)?) });
                    }
                }
                Ok(out)
            }
            Fx::Ref { kind: RefKind::Cell, .. } => Ok(vec![self.translate(e)?]),
            Fx::Name { .. } | Fx::Binary { op: BinaryOp::Range, .. } | Fx::Ref { .. } | Fx::Error(_) => {
                let Some(b) = self.table(e)? else { return Ok(vec![T::Num(null_num())]) };
                if (b.r2 - b.r1 + 1) as u64 * (b.c2 - b.c1 + 1) as u64 > 2_000 {
                    return Err("large range in an array function".into());
                }
                self.not_own_cells(b)?;
                let mut out = Vec::new();
                for r in b.r1..=b.r2 {
                    for c in b.c1..=b.c2 {
                        out.push(self.constant(b.sheet, r, c)?);
                    }
                }
                Ok(out)
            }
            Fx::Call { name, args } if name.eq_ignore_ascii_case("OFFSET") => {
                let _ = args;
                let Some(b) = self.table(e)? else { return Ok(vec![T::Num(null_num())]) };
                let mut out = Vec::new();
                for r in b.r1..=b.r2 {
                    for c in b.c1..=b.c2 {
                        out.push(self.constant(b.sheet, r, c)?);
                    }
                }
                Ok(out)
            }
            _ => Ok(vec![self.translate(e)?]),
        }
    }

    /// Whether the cells of a reference (over all the run's rows) hold a SUBTOTAL formula.
    fn holds_subtotal(&mut self, e: &Fx) -> Lifted<bool> {
        let blocks: Vec<Block> = match core(e) {
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } if matches!((core(lhs), core(rhs)), (Fx::Ref { kind: RefKind::Cell, .. }, Fx::Ref { kind: RefKind::Cell, .. })) => {
                let (Fx::Ref { prefix, text: a, .. }, Fx::Ref { text: b, .. }) = (core(lhs), core(rhs)) else { unreachable!() };
                let sheet = self.sheet_of(prefix)?;
                let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
                let (mut r1, mut r2) = (p.row.min(q.row), p.row.max(q.row));
                if !p.abs_row || !q.abs_row {
                    r2 += self.len - 1;
                }
                r1 = r1.max(1);
                vec![Block { sheet, r1, r2, c1: p.col.min(q.col), c2: p.col.max(q.col) }]
            }
            Fx::Ref { kind: RefKind::Cell, .. } => return Ok(false),
            _ => match self.table(e)? {
                Some(b) => vec![b],
                None => return Ok(false),
            },
        };
        for b in blocks {
            if (b.r2 - b.r1 + 1) as u64 * (b.c2 - b.c1 + 1) as u64 > 4_000_000 {
                return Err("very large range".into());
            }
            for r in b.r1..=b.r2 {
                for c in b.c1..=b.c2 {
                    if let Some(&i) = self.peek(&(b.sheet, r - 1, c - 1)) {
                        if self.wb.cells.formula[i].as_deref().is_some_and(|f| f.to_ascii_uppercase().contains("SUBTOTAL(")) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    /// A fixed block must not hold the run's own cells: their stored values are what
    /// is to be computed.
    pub(super) fn not_own_cells(&self, b: Block) -> Lifted<()> {
        let (first, last) = (self.row, self.row + self.len - 1);
        if b.sheet == self.sheet && b.c1 <= self.run_col && self.run_col <= b.c2 && b.r1 <= last && first <= b.r2 {
            return Err("fixed range that holds the formula's own cells".into());
        }
        Ok(())
    }

    /// Year, month and day of a serial day number (from 1 March 1900 on).
    pub(super) fn ymd(&mut self, serial: Expr) -> (Expr, Expr, Expr) {
        // Excel's own calendar: serial 0 is the 0th of January 1900, serial 60 the
        // 29th of February 1900, which never was.
        let (days, zero, leap) = self.serial_days(serial);
        let days = bind(&mut self.lets, days);
        let (zero, leap) = (bind(&mut self.lets, zero), bind(&mut self.lets, leap));
        let (y, m, d) = civil(&mut self.lets, days);
        let pick = |z: f64, l: f64, v: Expr| when(zero.clone()).then(lit(z)).otherwise(when(leap.clone()).then(lit(l)).otherwise(v));
        (pick(1900.0, 1900.0, y), pick(1.0, 2.0, m), pick(0.0, 29.0, d))
    }

    /// The serial day number of a day count since 1970-01-01 (from 1 March 1900 on).
    pub(super) fn serial_of(&self, days: Expr) -> Expr {
        if self.wb.date1904 {
            days + lit(24_107.0)
        } else {
            // Up to 28 February 1900 Excel's count is one behind.
            when(days.clone().lt(lit(-25_508.0))).then(days.clone() + lit(25_568.0)).otherwise(days + lit(25_569.0))
        }
    }

    /// A date written out by a format such as `mm/dd/yyyy` or `ddd`.
    fn date_text(&mut self, x: Expr, format: &str) -> Lifted<Expr> {
        let lower = format.to_ascii_lowercase();
        if lower.contains(['0', '#', '?', '@', '[', '"', '\\', ';', '%', '.', 'e', 'a']) || lower.is_empty() {
            return Err("TEXT with a format other than a date".into());
        }
        // The value to the second (as Excel shows it), then day and time of day.
        let seconds = bind(&mut self.lets, (x * lit(86_400.0) + lit(0.5)).floor());
        let serial = bind(&mut self.lets, (seconds.clone() / lit(86_400.0)).floor());
        let of_day = bind(&mut self.lets, seconds - serial.clone() * lit(86_400.0));
        let hour = bind(&mut self.lets, (of_day.clone() / lit(3600.0)).floor());
        let minute = bind(&mut self.lets, ((of_day.clone() - hour.clone() * lit(3600.0)) / lit(60.0)).floor());
        let second = bind(&mut self.lets, of_day - hour.clone() * lit(3600.0) - minute.clone() * lit(60.0));
        let (y, m, d) = self.ymd(serial.clone());
        let (y, m, d) = (bind(&mut self.lets, y), bind(&mut self.lets, m), bind(&mut self.lets, d));
        let weekday = bind(&mut self.lets, (serial.clone() - lit(1.0) - ((serial - lit(1.0)) / lit(7.0)).floor() * lit(7.0)) + lit(if self.wb.date1904 { 5.0 } else { 0.0 }));
        let weekday = bind(&mut self.lets, weekday.clone() - (weekday / lit(7.0)).floor() * lit(7.0));
        let int_text = |e: Expr| e.cast(DataType::Int64).cast(DataType::String);
        let two = |e: Expr| when(e.clone().lt(lit(10.0))).then(lit("0") + int_text(e.clone())).otherwise(int_text(e));
        let names = |e: Expr, list: &[&str], short: bool, from: f64| {
            let keys = Series::new("k".into(), (0..list.len()).map(|i| i as f64 + from).collect::<Vec<f64>>());
            let values = Series::new("v".into(), list.iter().map(|n| if short { n[..3].to_string() } else { n.to_string() }).collect::<Vec<String>>());
            full(e).replace_strict(lit(keys), lit(values), Some(null_text()), Some(DataType::String))
        };
        let mut out: Option<Expr> = None;
        let chars: Vec<char> = lower.chars().collect();
        let mut i = 0;
        // m right after h (or before s) is minutes.
        let mut after_hour = false;
        let minutes_follow = |from: usize| chars[from..].iter().find(|c| c.is_alphabetic()).is_some_and(|c| *c == 's');
        let push = |piece: Expr, out: &mut Option<Expr>| {
            *out = Some(match out.take() {
                Some(o) => o + piece,
                None => piece,
            })
        };
        while i < chars.len() {
            let c = chars[i];
            let mut n = 1;
            while i + n < chars.len() && chars[i + n] == c {
                n += 1;
            }
            let minutes = c == 'm' && (after_hour || minutes_follow(i + n));
            if c.is_alphabetic() {
                after_hour = c == 'h';
            }
            let piece = match (c, n) {
                ('m', 1) if minutes => int_text(minute.clone()),
                ('m', 2) if minutes => two(minute.clone()),
                ('h', 1) => int_text(hour.clone()),
                ('h', _) => two(hour.clone()),
                ('s', 1) => int_text(second.clone()),
                ('s', _) => two(second.clone()),
                ('m', 1) => int_text(m.clone()),
                ('m', 2) => two(m.clone()),
                ('m', 3) => names(m.clone(), &MONTH_NAMES, true, 1.0),
                ('m', 4) => names(m.clone(), &MONTH_NAMES, false, 1.0),
                ('m', 5) => names(m.clone(), &MONTH_NAMES, false, 1.0).str().head(lit(1i64)),
                ('d', 1) => int_text(d.clone()),
                ('d', 2) => two(d.clone()),
                ('d', 3) => names(weekday.clone(), &DAY_NAMES, true, 0.0),
                ('d', _) => names(weekday.clone(), &DAY_NAMES, false, 0.0),
                ('y', 1) | ('y', 2) => two(y.clone() - (y.clone() / lit(100.0)).floor() * lit(100.0)),
                ('y', _) => int_text(y.clone()),
                (other, _) if !other.is_alphanumeric() => {
                    let piece: String = std::iter::repeat_n(format.chars().nth(i).unwrap_or(other), n).collect();
                    lit(piece)
                }
                _ => return Err("TEXT with a format other than a date".into()),
            };
            push(piece, &mut out);
            i += n;
        }
        Ok(out.unwrap_or_else(|| lit("")))
    }

    /// A number written by a format of digit places (`0`, `#`), an optional decimal point,
    /// thousands commas, `%` and other characters as they stand (`###-##-####`).
    pub(super) fn number_format(&mut self, x: Expr, format: &str) -> Lifted<Expr> {
        if format.contains([';', '"', '\\', '?', 'E', 'e', '[', '@', '*', '_']) {
            return Err("TEXT with a format of another kind".into());
        }
        let percent = format.matches('%').count() as i32;
        let (int_pattern, frac_pattern) = match format.split_once('.') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (format.to_string(), String::new()),
        };
        if frac_pattern.contains(',') {
            return Err("TEXT with a format of another kind".into());
        }
        let decimals = frac_pattern.chars().filter(|c| *c == '0' || *c == '#').count() as i32;
        let grouping = int_pattern.contains(',');
        let x = bind(&mut self.lets, x * lit(100f64.powi(percent)));
        let scale = 10f64.powi(decimals);
        let scaled = x.clone().abs() * lit(scale);
        let rounded = bind(&mut self.lets, (scaled.clone() + lit(0.5) + scaled * lit(2e-15)).floor());
        let whole = bind(&mut self.lets, (rounded.clone() / lit(scale)).floor());
        let fraction = bind(&mut self.lets, rounded.clone() - whole.clone() * lit(scale));
        let int_text = |e: Expr| e.cast(DataType::Int64).cast(DataType::String);
        let pad = |e: Expr, width: usize| zfill(int_text(e), width);
        // The digits of the whole part, grouped by thousands where asked.
        let digits = if grouping {
            let mut groups = Vec::new();
            for k in 0..5 {
                let g = bind(&mut self.lets, (whole.clone() / lit(1000f64.powi(k))).floor() - (whole.clone() / lit(1000f64.powi(k + 1))).floor() * lit(1000.0));
                groups.push(g);
            }
            // Built from the top: the leading group plain, the others padded to three digits.
            let mut built = lit("");
            for top in (0..5).rev() {
                let mut t = int_text(groups[top].clone());
                for k in (0..top).rev() {
                    t = t + lit(",") + pad(groups[k].clone(), 3);
                }
                let here = if top == 0 { lit(true) } else { (whole.clone() / lit(1000f64.powi(top as i32))).floor().gt_eq(lit(1.0)) };
                let higher = (whole.clone() / lit(1000f64.powi(top as i32 + 1))).floor().gt_eq(lit(1.0));
                built = when(here.and(higher.not())).then(t).otherwise(built);
            }
            bind(&mut self.lets, built)
        } else {
            bind(&mut self.lets, int_text(whole.clone()))
        };
        let places: Vec<char> = int_pattern.chars().collect();
        let zeros = places.iter().filter(|c| **c == '0').count();
        let holders: Vec<usize> = (0..places.len()).filter(|&k| places[k] == '0' || places[k] == '#').collect();
        let inner_literals = holders.first().zip(holders.last()).is_some_and(|(a, b)| places[*a..=*b].iter().any(|c| *c != '0' && *c != '#' && *c != ','));
        let mut out = String::new();
        let mut pieces: Vec<Expr> = Vec::new();
        let flush = |out: &mut String, pieces: &mut Vec<Expr>| {
            if !out.is_empty() {
                pieces.push(lit(std::mem::take(out)));
            }
        };
        if inner_literals {
            if grouping {
                return Err("TEXT with a format of another kind".into());
            }
            // Fill the places from the right; the leftmost takes what is left. A whole part
            // of 0 has no digits: `#` places stay empty, `0` places show 0.
            let digits = bind(&mut self.lets, when(whole.clone().eq(lit(0.0))).then(lit("")).otherwise(digits.clone()));
            let len = bind(&mut self.lets, digits.clone().str().len_chars().cast(DataType::Int64));
            let count = holders.len() as i64;
            for (k, c) in places.iter().enumerate() {
                if let Some(pos) = holders.iter().position(|h| *h == k) {
                    flush(&mut out, &mut pieces);
                    let from_right = count - pos as i64; // 1 = last place
                    let zero = lit(if *c == '0' { "0" } else { "" });
                    let piece = if pos == 0 {
                        let take = len.clone() - lit(count - 1);
                        when(take.clone().gt(lit(0i64))).then(digits.clone().str().head(take)).otherwise(zero)
                    } else {
                        let at = len.clone() - lit(from_right);
                        when(at.clone().gt_eq(lit(0i64))).then(digits.clone().str().slice(at, lit(1u64))).otherwise(zero)
                    };
                    pieces.push(piece);
                } else {
                    out.push(*c);
                }
            }
        } else {
            let first = holders.first().copied();
            for (k, c) in places.iter().enumerate() {
                if Some(k) == first {
                    flush(&mut out, &mut pieces);
                    // At least as many digits as there are 0 places; none for a whole 0 without them.
                    let shown = if zeros > 0 { zfill(digits.clone(), zeros) } else { when(whole.clone().eq(lit(0.0))).then(lit("")).otherwise(digits.clone()) };
                    pieces.push(shown);
                } else if c != &'0' && c != &'#' && c != &',' {
                    out.push(*c);
                }
            }
        }
        if decimals > 0 {
            out.push('.');
            flush(&mut out, &mut pieces);
            pieces.push(pad(fraction, decimals as usize));
        }
        for c in frac_pattern.chars().filter(|c| *c != '0' && *c != '#') {
            out.push(c);
        }
        flush(&mut out, &mut pieces);
        let body = pieces.into_iter().reduce(|a, b| a + b).unwrap_or_else(|| lit(""));
        let negative = x.lt(lit(0.0)).and(rounded.gt(lit(0.0)));
        Ok(when(negative).then(lit("-") + body.clone()).otherwise(body))
    }

    /// The standard normal distribution function, to about 15 digits: the series of erf
    /// near 0, the continued fraction of erfc further out.
    fn normal_cdf(&mut self, z: Expr) -> Expr {
        let z = bind(&mut self.lets, z);
        let x = bind(&mut self.lets, z.clone().abs() / lit(std::f64::consts::SQRT_2));
        // erf(x) = 2/sqrt(pi) * sum (-1)^n x^(2n+1) / (n! (2n+1))
        let x2 = bind(&mut self.lets, x.clone() * x.clone());
        let mut term = x.clone();
        let mut sum = x.clone();
        for n in 1..=60 {
            term = bind(&mut self.lets, term * (lit(0.0) - x2.clone()) / lit(n as f64));
            sum = sum + term.clone() / lit(2.0 * n as f64 + 1.0);
        }
        let erf_small = bind(&mut self.lets, sum * lit(2.0 / std::f64::consts::PI.sqrt()));
        // erfc(x) = exp(-x^2)/sqrt(pi) / (x + (1/2)/(x + 1/(x + (3/2)/(x + ...))))
        let mut t = x.clone();
        for k in (1..=80).rev() {
            t = bind(&mut self.lets, x.clone() + lit(k as f64 / 2.0) / t);
        }
        let erfc_large = bind(&mut self.lets, (lit(0.0) - x2).exp() / (lit(std::f64::consts::PI.sqrt()) * t));
        let half = bind(&mut self.lets, when(x.lt(lit(2.0))).then(lit(1.0) - erf_small).otherwise(erfc_large) / lit(2.0));
        when(z.lt(lit(0.0))).then(half.clone()).otherwise(lit(1.0) - half)
    }

    /// DATEVALUE of a text: `m/d/y`, `y-m-d` and `d-mon-y`; the day without the time.
    fn date_value(&mut self, text: Expr) -> Expr {
        let lower = bind(&mut self.lets, text.str().to_lowercase());
        let group = |pattern: &str, k: usize| lower.clone().str().extract(lit(pattern.to_string()), k);
        let number = |e: Expr| e.cast(DataType::Float64);
        let mdy = r"^(\d{1,2})[/-](\d{1,2})[/-](\d{1,4})$";
        let ymd = r"^(\d{4})[/-](\d{1,2})[/-](\d{1,2})$";
        let dmy = r"^(\d{1,2})[- ]([a-z]{3,9})[- ](\d{1,4})$";
        let month_of = |e: Expr| {
            let keys = Series::new("k".into(), MONTH_NAMES.iter().map(|n| n[..3].to_ascii_lowercase()).collect::<Vec<String>>());
            let values = Series::new("v".into(), (1..=12).map(|v| v as f64).collect::<Vec<f64>>());
            full(e.str().head(lit(3i64))).replace_strict(lit(keys), lit(values), Some(null_num()), Some(DataType::Float64))
        };
        let (m1, d1, y1) = (number(group(mdy, 1)), number(group(mdy, 2)), group(mdy, 3));
        let y1 = century(number(y1.clone()), y1.str().len_chars());
        let (y2, m2, d2) = (number(group(ymd, 1)), number(group(ymd, 2)), number(group(ymd, 3)));
        let (d3, m3, y3) = (number(group(dmy, 1)), month_of(group(dmy, 2)), group(dmy, 3));
        let y3 = century(number(y3.clone()), y3.str().len_chars());
        let y = bind(&mut self.lets, when(y1.clone().is_not_null()).then(y1).otherwise(when(y2.clone().is_not_null()).then(y2).otherwise(y3)));
        let m = bind(&mut self.lets, when(m1.clone().is_not_null()).then(m1).otherwise(when(m2.clone().is_not_null()).then(m2).otherwise(m3)));
        let d = bind(&mut self.lets, when(d1.clone().is_not_null()).then(d1).otherwise(when(d2.clone().is_not_null()).then(d2).otherwise(d3)));
        let days = days_from_civil(&mut self.lets, y.clone(), m.clone(), d.clone());
        let days = bind(&mut self.lets, days);
        // The date must exist: going back to year, month and day gives the same.
        let (by, bm, bd) = civil(&mut self.lets, days.clone());
        let exists = by.eq(y.clone()).and(bm.eq(m.clone())).and(bd.eq(d)).and(m.clone().gt_eq(lit(1.0))).and(m.lt_eq(lit(12.0)));
        let serial = self.serial_of(days);
                let serial = bind(&mut self.lets, serial);
        when(exists.fill_null(lit(false)).and(serial.clone().lt_eq(lit(2_958_465.0)))).then(serial).otherwise(null_num())
    }

    /// SUMIF, COUNTIF, AVERAGEIF and the IFS forms.
    fn conditional(&mut self, name: &str, args: &[Fx]) -> Lifted<T> {
        let plural = name.ends_with("IFS");
        let counting = name.starts_with("COUNT");
        let (pairs, values): (Vec<(&Fx, &Fx)>, Option<&Fx>) = if plural {
            let rest = if counting { args } else { args.get(1..).ok_or("IFS without arguments")? };
            if rest.is_empty() || rest.len() % 2 != 0 {
                return Err(format!("{name} with {} arguments", args.len()));
            }
            (rest.chunks(2).map(|p| (&p[0], &p[1])).collect(), if counting { None } else { Some(&args[0]) })
        } else {
            match (counting, args.len()) {
                (true, 2) => (vec![(&args[0], &args[1])], None),
                (false, 2) => (vec![(&args[0], &args[1])], Some(&args[0])),
                (false, 3) => (vec![(&args[0], &args[1])], Some(&args[2])),
                _ => return Err(format!("{name} with {} arguments", args.len())),
            }
        };
        if let Some(result) = self.conditional_growing(name, &pairs, values)? {
            return Ok(result);
        }
        if let Some(result) = self.conditional_table(name, &pairs, values)? {
            return Ok(result);
        }
        let mut lists = Vec::new();
        let mut crits = Vec::new();
        for (range, crit) in &pairs {
            lists.push(self.cells_of(range)?);
            crits.push(self.criterion(crit)?);
        }
        let n = lists[0].len();
        // The values summed: a range of the same size (SUMIF takes the size of its first range).
        let summed = match values {
            Some(v) => {
                let list = self.cells_of(v)?;
                #[allow(clippy::needless_return)]
                if list.len() != n {
                    if plural {
                        return Ok(T::Num(null_num()));
                    }
                    // SUMIF sums the block of the first range's size from the sum range's corner.
                    let Some(sum) = self.table(v)? else { return Ok(T::Num(null_num())) };
                    let Some(range) = self.table(pairs[0].0)? else { return Ok(T::Num(null_num())) };
                    let corner = Block { sheet: sum.sheet, r1: sum.r1, c1: sum.c1, r2: sum.r1 + (range.r2 - range.r1), c2: sum.c1 + (range.c2 - range.c1) };
                    self.not_own_cells(corner)?;
                    let mut cells = Vec::new();
                    for r in corner.r1..=corner.r2 {
                        for c in corner.c1..=corner.c2 {
                            cells.push(self.constant(corner.sheet, r, c)?);
                        }
                    }
                    Some(cells)
                } else {
                    Some(list)
                }
            }
            None => None,
        };
        if lists.iter().any(|l| l.len() != n) {
            return Ok(T::Num(null_num()));
        }
        let (mut sum, mut count) = (lit(0.0), lit(0.0));
        let mut error: Option<Expr> = None;
        for k in 0..n {
            let mut all: Option<Expr> = None;
            for (list, crit) in lists.iter().zip(&crits) {
                let m = matches(&list[k], crit);
                all = Some(match all {
                    Some(a) => a.and(m),
                    None => m,
                });
            }
            let hit = bind(&mut self.lets, all.unwrap().fill_null(lit(false)));
            if let Some(list) = &summed {
                // Only numbers are summed (and averaged); an error among them is the result.
                let (number, is_error) = summed_number(&list[k]);
                count = count + hit.clone().and(is_number(&list[k])).cast(DataType::Float64);
                sum = sum + when(hit.clone()).then(number).otherwise(lit(0.0));
                let e = hit.and(is_error);
                error = Some(match error {
                    Some(x) => x.or(e),
                    None => e,
                });
            }
        }
        if summed.is_none() {
            count = lit(0.0);
            for k in 0..n {
                let mut all: Option<Expr> = None;
                for (list, crit) in lists.iter().zip(&crits) {
                    let m = matches(&list[k], crit);
                    all = Some(match all {
                        Some(a) => a.and(m),
                        None => m,
                    });
                }
                count = count + all.unwrap().fill_null(lit(false)).cast(DataType::Float64);
            }
        }
        let count = bind(&mut self.lets, count);
        let result = match (counting, name.starts_with("AVERAGE")) {
            (true, _) => count,
            (_, true) => divide(&mut self.lets, sum, count),
            _ => sum,
        };
        Ok(T::Num(match error {
            Some(e) => when(e).then(null_num()).otherwise(result),
            None => result,
        }))
    }

    /// One column from a fixed row down to the formula's row (plus an offset): the cells
    /// of every frame row, the fixed row and the offset.
    fn growing_column(&mut self, e: &Fx) -> Lifted<Option<(T, u32, i64)>> {
        let Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } = core(e) else { return Ok(None) };
        let (Fx::Ref { prefix, kind: RefKind::Cell, text: a }, Fx::Ref { prefix: None, kind: RefKind::Cell, text: b }) = (core(lhs), core(rhs)) else { return Ok(None) };
        let (p, q) = (parse_cell_ref(a).ok_or("range form")?, parse_cell_ref(b).ok_or("range form")?);
        if p.abs_row == q.abs_row {
            return Ok(None);
        }
        if p.col != q.col {
            return Err("growing range of several columns in a conditional aggregate".into());
        }
        let sheet = self.sheet_of(prefix)?;
        let (anchor, moving) = if p.abs_row { (p.row, q.row) } else { (q.row, p.row) };
        if moving < anchor || self.in_name > 0 || (sheet == self.sheet && p.col == self.run_col) {
            return Err("growing range of another form in a conditional aggregate".into());
        }
        let offset = moving as i64 - self.row as i64;
        let last = self.row as i64 - 1 + offset + self.len as i64 - 1;
        self.columns.insert((sheet, p.col));
        self.reach = (self.reach.0.min(anchor as i64 - self.row as i64), self.reach.1.max(offset));
        let (has_text, has_error) = self.holds(sheet, p.col, anchor as i64 - 1, last);
        let text = has_text.then(|| {
            self.text_columns.insert((sheet, p.col));
            col(text_column_name(sheet, p.col))
        });
        Ok(Some((T::Dyn(Dyn { num: col(column_name(sheet, p.col)), text, blank: true, nan: has_error, parsed: None }), anchor, offset)))
    }

    /// Conditional aggregates over ranges that grow with the row, with fixed criteria:
    /// running sums of the matches.
    fn conditional_growing(&mut self, name: &str, pairs: &[(&Fx, &Fx)], values: Option<&Fx>) -> Lifted<Option<T>> {
        let mut columns = Vec::new();
        for (range, crit) in pairs {
            let Some(g) = self.growing_column(range)? else { return Ok(None) };
            if !self.is_fixed(crit, 0) {
                return Err("criterion per row over a growing range".into());
            }
            columns.push((g, self.criterion(crit)?));
        }
        let summed = match values {
            Some(v) => match self.growing_column(v)? {
                Some(g) => Some(g),
                None => return Err("growing ranges of different forms".into()),
            },
            None => None,
        };
        let (anchor, offset) = (columns[0].0 .1, columns[0].0 .2);
        if columns.iter().any(|((_, a, o), _)| (*a, *o) != (anchor, offset)) || summed.as_ref().is_some_and(|(_, a, o)| (*a, *o) != (anchor, offset)) {
            return Err("growing ranges of different forms".into());
        }
        let inside = col(ROW).gt_eq(lit(anchor as i64));
        let mut hit = inside.clone();
        for ((cell, _, _), crit) in &columns {
            hit = hit.and(matches(cell, crit).fill_null(lit(false)));
        }
        let hit = bind(&mut self.lets, hit);
        let at_offset = |e: Expr| if offset == 0 { e } else { e.shift(lit(-offset)) };
        let running = |e: Expr| at_offset(e.cum_sum(false));
        let (count, sum, error) = match &summed {
            Some((cell, _, _)) => {
                let (number, is_error) = summed_number(cell);
                (
                    running(hit.clone().and(is_number(cell)).cast(DataType::Float64)),
                    running(when(hit.clone()).then(number).otherwise(lit(0.0))),
                    Some(running(hit.and(is_error).cast(DataType::Float64)).gt(lit(0.0))),
                )
            }
            None => (running(hit.cast(DataType::Float64)), lit(0.0), None),
        };
        let count = bind(&mut self.lets, count);
        let result = if name.starts_with("COUNT") {
            count
        } else if name.starts_with("AVERAGE") {
            divide(&mut self.lets, sum, count)
        } else {
            sum
        };
        Ok(Some(T::Num(match error {
            Some(e) => when(e).then(null_num()).otherwise(result),
            None => result,
        })))
    }

    /// SUMIF and its relatives over a fixed range with one criterion that is a value per
    /// row: the sums for every key in the range are worked out once and looked up.
    fn conditional_table(&mut self, name: &str, pairs: &[(&Fx, &Fx)], values: Option<&Fx>) -> Lifted<Option<T>> {
        if pairs.len() != 1 {
            return Ok(None);
        }
        let plain = |e: &Fx| matches!(core(e), Fx::Binary { op: BinaryOp::Range, .. } | Fx::Name { .. } | Fx::Ref { kind: RefKind::Columns | RefKind::Rows, .. } | Fx::Call { .. });
        if !plain(pairs[0].0) || values.is_some_and(|v| !plain(v)) {
            return Ok(None);
        }
        let Ok(Some(range)) = self.table(pairs[0].0) else { return Ok(None) };
        let summed = match values {
            Some(v) => match self.table(v) {
                Ok(Some(b)) => Some(b),
                _ => return Ok(None),
            },
            None => None,
        };
        let (h, w) = (range.r2 - range.r1 + 1, range.c2 - range.c1 + 1);
        // The size as written: rows past the sheet's last cell are empty but count.
        let written = match core(pairs[0].0) {
            Fx::Binary { op: BinaryOp::Range, lhs, rhs, .. } => match (core(lhs), core(rhs)) {
                (Fx::Ref { kind: RefKind::Cell, text: a, .. }, Fx::Ref { kind: RefKind::Cell, text: b, .. }) => match (parse_cell_ref(a), parse_cell_ref(b)) {
                    (Some(p), Some(q)) => (p.row.abs_diff(q.row) as u64 + 1) * (p.col.abs_diff(q.col) as u64 + 1),
                    _ => h as u64 * w as u64,
                },
                _ => h as u64 * w as u64,
            },
            _ => h as u64 * w as u64,
        };
        // A small range with a criterion that varies goes the way of an expression per row;
        // a fixed criterion is worked out now, whatever the size.
        if written <= 400 && !self.is_fixed(pairs[0].1, 0) {
            return Ok(None);
        }
        if (h as u64) * (w as u64) > 4_000_000 {
            return Err("very large range in a conditional aggregate".into());
        }
        self.not_own_cells(range)?;
        if let Some(b) = summed {
            self.not_own_cells(b)?;
        }
        if self.is_fixed(pairs[0].1, 0) {
            let beyond = written.saturating_sub(h as u64 * w as u64) as f64;
            return self.conditional_fixed(name, range, summed, pairs[0].1, beyond).map(Some);
        }
        let crit = self.criterion(pairs[0].1)?;
        let Crit::Eq(value) = crit else { return Ok(None) };
        // Per key: sum of the numbers summed, count of matches, count of numbers summed, error.
        let mut by_number: HashMap<u64, (f64, f64, f64, bool)> = HashMap::new();
        let mut by_text: HashMap<String, (f64, f64, f64, bool)> = HashMap::new();
        for r in 0..h {
            for c in 0..w {
                let key = self.key(range.sheet, range.r1 + r, range.c1 + c);
                let slot = match key {
                    Key::Num(v) => by_number.entry((v + 0.0).to_bits()).or_default(),
                    Key::Text(t) => by_text.entry(t).or_default(),
                    _ => continue,
                };
                slot.1 += 1.0;
                if let Some(b) = summed {
                    if let (Some(v), None) = self.entry(b.sheet, b.r1 + r, b.c1 + c) {
                        if v.is_nan() {
                            slot.3 = true;
                        } else {
                            slot.0 += v;
                            slot.2 += 1.0;
                        }
                    }
                }
            }
        }
        let counting = name.starts_with("COUNT");
        let averaging = name.starts_with("AVERAGE");
        let result_of = |x: &(f64, f64, f64, bool)| -> Option<f64> {
            if x.3 && !counting {
                None
            } else if counting {
                Some(x.1)
            } else if averaging {
                if x.2 > 0.0 { Some(x.0 / x.2) } else { None }
            } else {
                Some(x.0)
            }
        };
        let missing = if averaging { None } else { Some(0.0) };
        let (vn, vt) = match value.bound(&mut self.lets) {
            T::Num(e) => (Some(e), None),
            T::Text(t) => (None, Some(t)),
            T::Dyn(d) => (Some(d.num.clone()), d.text.clone()),
            T::Bool(_) => return Err("criterion that is a truth value".into()),
        };
        let mut result = missing.map(lit).unwrap_or_else(null_num);
        if let (Some(v), false) = (&vn, by_number.is_empty()) {
            let keys: Vec<f64> = by_number.keys().map(|b| f64::from_bits(*b)).collect();
            let vals: Vec<Option<f64>> = by_number.values().map(result_of).collect();
            let looked = full(v.clone()).replace_strict(lit(Series::new("k".into(), keys)), lit(Series::new("v".into(), vals)), Some(missing.map(lit).unwrap_or_else(null_num)), Some(DataType::Float64));
            result = when(v.clone().is_not_null()).then(looked).otherwise(result);
        }
        if let (Some(t), false) = (&vt, by_text.is_empty()) {
            let keys: Vec<String> = by_text.keys().cloned().collect();
            let vals: Vec<Option<f64>> = by_text.values().map(result_of).collect();
            let looked = full(t.clone().str().to_lowercase()).replace_strict(lit(Series::new("k".into(), keys)), lit(Series::new("v".into(), vals)), Some(missing.map(lit).unwrap_or_else(null_num)), Some(DataType::Float64));
            result = when(t.clone().is_not_null()).then(looked).otherwise(result);
        }
        Ok(Some(T::Num(bind(&mut self.lets, result))))
    }

    /// The rows of a database (below its header row) that meet a criteria range: a row
    /// of headers, then rows of conditions; a row's conditions must all hold, any row will do.
    fn database_rows(&self, db: Block, criteria: Block) -> Lifted<Option<Vec<u32>>> {
        let header = |c: u32| match self.key(db.sheet, db.r1, c) {
            Key::Text(t) => Some(t.trim().to_string()),
            _ => None,
        };
        let mut conditions: Vec<Vec<(u32, String, Option<f64>)>> = Vec::new();
        for r in criteria.r1 + 1..=criteria.r2 {
            let mut row = Vec::new();
            for c in criteria.c1..=criteria.c2 {
                let (number, text) = self.entry(criteria.sheet, r, c);
                if number.is_none() && text.as_deref().is_none_or(|t| t.is_empty()) {
                    continue;
                }
                let label = match self.key(criteria.sheet, criteria.r1, c) {
                    Key::Text(name) => Some(name),
                    _ => None,
                };
                let column = label.as_deref().and_then(|name| (db.c1..=db.c2).find(|&k| header(k).as_deref() == Some(name.trim())));
                // A condition under a label the database does not have: #VALUE!.
                let Some(column) = column else { return Ok(None) };
                if number.is_some_and(f64::is_nan) {
                    return Err("criterion that is an error".into());
                }
                row.push((column, text.unwrap_or_default(), number));
            }
            conditions.push(row);
        }
        let meets = |r: u32, (column, text, number): &(u32, String, Option<f64>)| -> Lifted<bool> {

            let (n, t) = self.entry(db.sheet, r, *column);
            if let Some(v) = number {
                return Ok(n == Some(*v));
            }
            let s = text.trim_start();
            let (op, rest) = ["<=", ">=", "<>", "<", ">", "="].iter().find_map(|o| s.strip_prefix(o).map(|r| (*o, r))).unwrap_or(("", s));
            if let Some(v) = text_as_number(rest, false).filter(|_| !rest.trim().is_empty()) {
                let Some(x) = n.filter(|x| !x.is_nan()) else { return Ok(op == "<>") };
                return Ok(match op {
                    "<" => x < v,
                    ">" => x > v,
                    "<=" => x <= v,
                    ">=" => x >= v,
                    "<>" => x != v,
                    _ => x == v,
                });
            }
            let cell = t.map(|x| x.to_lowercase());
            let pattern = rest.to_lowercase();
            Ok(match op {
                // Text alone matches what begins with it; with = only what equals it.
                "" | "=" => {
                    let whole = if op.is_empty() { format!("{pattern}*") } else { pattern.clone() };
                    let re = regex_lite(&wildcard_regex(&whole));
                    match (cell, op) {
                        (Some(c), _) => re(&c),
                        (None, "=") => pattern.is_empty() && n.is_none(),
                        _ => false,
                    }
                }
                "<>" => match cell {
                    Some(c) => !regex_lite(&wildcard_regex(&pattern))(&c),
                    None => !(pattern.is_empty() && n.is_none()),
                },
                _ => return Err("criterion ordering text".into()),
            })
        };
        let mut out = Vec::new();
        for r in db.r1 + 1..=db.r2 {
            let mut any = conditions.is_empty();
            for row in &conditions {
                let mut all = true;
                for cond in row {
                    if !meets(r, cond)? {
                        all = false;
                        break;
                    }
                }
                if all {
                    any = true;
                    break;
                }
            }
            if any {
                out.push(r);
            }
        }
        Ok(Some(out))
    }

    /// SUMIF and its relatives over a fixed range with a fixed criterion: worked out now.
    fn conditional_fixed(&mut self, name: &str, range: Block, summed: Option<Block>, crit: &Fx, beyond: f64) -> Lifted<T> {
        let number = self.fixed_number(crit).ok().flatten();
        let text = match core(crit) {
            Fx::Str(_) => self.fixed_text_of(crit)?,
            _ => if number.is_none() { self.fixed_text_of(crit)? } else { None },
        };
        // What a cell must be: a number compared, a text pattern, or empty.
        enum Want {
            Number(BinaryOp, f64),
            Text(bool, String),
        }
        let want = match (number, text) {
            (_, Some(t)) => match fixed_criterion(&t)? {
                Crit::Cmp(op, _) => {
                    let rest = t.trim_start().trim_start_matches(['<', '>', '=']);
                    Want::Number(op, text_as_number(rest, false).ok_or("criterion form")?)
                }
                Crit::Text { negate, pattern } => Want::Text(negate, pattern),
                Crit::Eq(_) => return Err("criterion form".into()),
            },
            (Some(v), None) => Want::Number(BinaryOp::Eq, v),
            // An empty cell as criterion counts as 0.
            (None, None) => Want::Number(BinaryOp::Eq, 0.0),
        };
        let matcher = match &want {
            Want::Text(_, p) if p.contains(['*', '?', '~']) => Some(regex_lite(&wildcard_regex(p))),
            _ => None,
        };
        let (h, w) = (range.r2 - range.r1 + 1, range.c2 - range.c1 + 1);
        let (mut sum, mut count, mut numbers, mut error) = (0.0, 0.0, 0.0, false);
        for r in 0..h {
            for c in 0..w {
                let (n, t) = self.entry(range.sheet, range.r1 + r, range.c1 + c);
                let hit = match &want {
                    Want::Number(op, v) => n.filter(|x| !x.is_nan()).is_some_and(|x| match op {
                        BinaryOp::Lt => x < *v,
                        BinaryOp::Gt => x > *v,
                        BinaryOp::Le => x <= *v,
                        BinaryOp::Ge => x >= *v,
                        BinaryOp::Ne => x != *v,
                        _ => x == *v,
                    }) || (matches!(op, BinaryOp::Ne) && n.is_none_or(f64::is_nan)),
                    Want::Text(negate, pattern) => {
                        let hit = match (&t, pattern.is_empty()) {
                            (Some(s), true) => s.is_empty(),
                            (None, true) => n.is_none(),
                            (Some(s), false) => match &matcher {
                                Some(m) => m(&s.to_lowercase()),
                                None => s.to_lowercase() == *pattern,
                            },
                            (None, false) => false,
                        };
                        hit != *negate
                    }
                };
                if !hit {
                    continue;
                }
                count += 1.0;
                if let Some(b) = summed {
                    if let (Some(v), None) = self.entry(b.sheet, b.r1 + r, b.c1 + c) {
                        if v.is_nan() {
                            error = true;
                        } else {
                            sum += v;
                            numbers += 1.0;
                        }
                    }
                }
            }
        }
        // Cells past the sheet's last row are empty: they count where an empty cell meets the criterion.
        let empty_meets = match &want {
            Want::Number(op, _) => matches!(op, BinaryOp::Ne),
            Want::Text(negate, pattern) => pattern.is_empty() != *negate,
        };
        if empty_meets {
            count += beyond;
        }
        let value = if name.starts_with("COUNT") {
            Some(count)
        } else if error {
            None
        } else if name.starts_with("AVERAGE") {
            (numbers > 0.0).then(|| sum / numbers)
        } else {
            Some(sum)
        };
        Ok(T::Num(value.map(lit).unwrap_or_else(null_num)))
    }

    /// A criterion: fixed text such as `">5"` or `"ab*"`, or a value given per row.
    fn criterion(&mut self, e: &Fx) -> Lifted<Crit> {
        // An operator joined to a value per row: ">"&A2.
        if let Fx::Binary { op: BinaryOp::Concat, lhs, rhs, .. } = core(e) {
            if let Fx::Str(s) = core(lhs) {
                let op = match unquote(s).as_str() {
                    ">" => Some(BinaryOp::Gt),
                    "<" => Some(BinaryOp::Lt),
                    ">=" => Some(BinaryOp::Ge),
                    "<=" => Some(BinaryOp::Le),
                    "=" => Some(BinaryOp::Eq),
                    "<>" => Some(BinaryOp::Ne),
                    _ => None,
                };
                if let Some(op) = op {
                    if !self.is_fixed(rhs, 0) {
                        let value = self.translate(rhs)?;
                        if value.can_be_text() {
                            return Err("criterion comparing with text per row".into());
                        }
                        return Ok(Crit::Cmp(op, value.num()));
                    }
                }
            }
        }
        if self.is_fixed(e, 0) {
            if let Fx::Str(s) = core(e) {
                return fixed_criterion(&unquote(s));
            }
            if let Some(text) = self.fixed_text(e)? {
                return fixed_criterion(&text);
            }
        }
        let value = self.translate(e)?.bound(&mut self.lets);
        if let T::Bool(_) = value {
            return Err("criterion that is a truth value".into());
        }
        let text = match &value {
            T::Text(t) => Some(t.clone()),
            T::Dyn(Dyn { text: Some(t), .. }) => Some(t.clone()),
            _ => None,
        };
        if let Some(t) = text {
            // Operators, wildcards, numbers written as text and the empty text mean more than equality.
            let special = t.clone().str().contains(lit(r"^\s*([<>=]|$)|[*?~]|^\s*[-+]?(\d|\.\d)"), false);
            self.guard("criterion with an operator, wildcard or number as text", special);
        }
        // An empty cell as criterion counts as 0.
        let value = match value {
            T::Dyn(d) if d.blank => {
                let num = bind(&mut self.lets, when(d.is_blank()).then(lit(0.0)).otherwise(d.num.clone()));
                T::Dyn(Dyn { num, blank: false, ..d })
            }
            other => other,
        };
        Ok(Crit::Eq(value))
    }

    pub(super) fn fixed_text_of(&mut self, e: &Fx) -> Lifted<Option<String>> {
        if let Fx::Str(s) = core(e) {
            return Ok(Some(unquote(s)));
        }
        self.fixed_text(e)
    }

    /// The text a fixed expression gives; `None` if it is a number.
    fn fixed_text(&mut self, e: &Fx) -> Lifted<Option<String>> {
        // Read as values that go into the translation: the cells are constants, noted as read.
        let was = self.param.replace(false);
        let r = self.fixed_text_in(e);
        self.param.set(was);
        r
    }

    fn fixed_text_in(&mut self, e: &Fx) -> Lifted<Option<String>> {
        self.baked.set(self.baked.get() | BAKED_DECISION);
        let saved = std::mem::take(&mut self.lets);
        let read = (self.columns.len(), self.text_columns.len());
        let translated = self.translate(e);
        if (self.columns.len(), self.text_columns.len()) != read {
            self.lets = saved;
            return Err("fixed expression that reads the row".into());
        }
        let lets = std::mem::replace(&mut self.lets, saved);
        let t = translated?;
        if !t.can_be_text() {
            return Ok(None);
        }
        let expr = match t {
            T::Text(x) => x,
            T::Dyn(Dyn { text: Some(x), .. }) => x,
            _ => return Ok(None),
        };
        let out = fixed_value(self.ex, self.row, lets, expr.cast(DataType::String))?;
        Ok(out.str(0).map(|s| s.to_string()))
    }
}

/// Excel's halving search for the last key not above a value: `cmp(key)` compares a
/// key with the value. Returns the index found, if any.
fn halving<K>(keys: &[K], cmp: impl Fn(&K) -> std::cmp::Ordering) -> Option<usize> {
    let (mut lo, mut hi) = (0i64, keys.len() as i64 - 1);
    let mut found = None;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        if cmp(&keys[mid as usize]) != std::cmp::Ordering::Greater {
            found = Some(mid as usize);
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    found
}

/// A function of a number that changes only at `cuts` (ascending), given by `at`:
/// its value below the first cut, at each cut and between cuts, looked up per row.
fn stretches(x: Expr, cuts: &[f64], at: impl Fn(f64) -> Option<f64>) -> Expr {
    let n = cuts.len();
    let below = at(cuts[0] - 1.0 - cuts[0].abs());
    let mut on = Vec::with_capacity(n);
    let mut between = Vec::with_capacity(n);
    for i in 0..n {
        on.push(at(cuts[i]));
        let next = if i + 1 < n { (cuts[i] + cuts[i + 1]) / 2.0 } else { cuts[i] + 1.0 + cuts[i].abs() };
        between.push(at(next));
    }
    let x = full(x);
    let index = lit(Series::new("c".into(), cuts.to_vec())).search_sorted(x.clone(), SearchSortedSide::Right, false).cast(DataType::Float64);
    let ones = Series::new("o".into(), vec![1.0f64; n]);
    let is_cut = x.replace_strict(lit(Series::new("c".into(), cuts.to_vec())), lit(ones), Some(lit(0.0)), Some(DataType::Float64));
    let idx = Series::new("i".into(), (1..=n).map(|i| i as f64).collect::<Vec<f64>>());
    let at_cut = full(index.clone()).replace_strict(lit(idx.clone()), lit(Series::new("a".into(), on)), Some(null_num()), Some(DataType::Float64));
    let after = full(index.clone()).replace_strict(lit(idx), lit(Series::new("b".into(), between)), Some(null_num()), Some(DataType::Float64));
    when(index.eq(lit(0.0))).then(below.map(lit).unwrap_or_else(null_num)).otherwise(when(is_cut.eq(lit(1.0))).then(at_cut).otherwise(after))
}

/// A count Excel takes as a whole number: cut toward zero, at 15 significant digits
/// (12*(30/360) is 1, not 0.9999999999999996).
pub(super) fn whole(x: Expr) -> Expr {
    let hair = x.clone().abs() * lit(2e-15);
    when(x.clone().lt(lit(0.0))).then((x.clone() - hair.clone()).ceil()).otherwise((x + hair).floor())
}

/// Declining balance for one period (LibreOffice's ScGetGDA, as Excel).
fn gda(value: f64, rest: f64, period: f64, per: f64, factor: f64) -> f64 {
    let mut rate = factor / period;
    let old;
    if rate >= 1.0 {
        rate = 1.0;
        old = if per == 1.0 { value } else { 0.0 };
    } else {
        old = value * (1.0 - rate).powf(per - 1.0);
    }
    let new = value * (1.0 - rate).powf(per);
    let g = if new < rest { old - rest } else { old - new };
    g.max(0.0)
}

fn ddb(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> Option<f64> {
    if cost < 0.0 || salvage < 0.0 || factor <= 0.0 || salvage > cost || period <= 0.0 || period > life {
        return None;
    }
    Some(gda(cost, salvage, life, period, factor))
}

fn inter_vdb(cost: f64, salvage: f64, life: f64, life1: f64, period: f64, factor: f64) -> f64 {
    let mut vdb = 0.0;
    let int_end = period.ceil();
    let loop_end = int_end as i64;
    let mut rest = cost - salvage;
    let mut now_sln = false;
    let mut sln = 0.0;
    for i in 1..=loop_end {
        let term;
        if !now_sln {
            let d = gda(cost, salvage, life, i as f64, factor);
            sln = rest / (life1 - (i - 1) as f64);
            if sln > d {
                term = sln;
                now_sln = true;
            } else {
                term = d;
                rest -= d;
            }
        } else {
            term = sln;
        }
        vdb += if i == loop_end { term * (period + 1.0 - int_end) } else { term };
    }
    vdb
}

/// VDB: the declining balance switching to straight line, from start to end.
fn vdb(cost: f64, salvage: f64, life: f64, start: f64, end: f64, factor: f64, no_switch: bool) -> Option<f64> {
    if start < 0.0 || end < start || end > life || cost < 0.0 || salvage > cost || factor <= 0.0 {
        return None;
    }
    let (int_start, int_end) = (start.floor(), end.ceil());
    let mut vdb = 0.0;
    if no_switch {
        let (s, e) = (int_start as i64 + 1, int_end as i64);
        for i in s..=e {
            let mut term = gda(cost, salvage, life, i as f64, factor);
            if i == s {
                term *= end.min(int_start + 1.0) - start;
            } else if i == e {
                term *= end + 1.0 - int_end;
            }
            vdb += term;
        }
    } else {
        let (mut start, mut end) = (start, end);
        if start != start.floor() && factor > 1.0 && start >= life / 2.0 {
            let part = start - life / 2.0;
            start = life / 2.0;
            end -= part;
        }
        // The depreciation up to the end less that up to the start.
        vdb = inter_vdb(cost, salvage, life, life, end, factor) - inter_vdb(cost, salvage, life, life, start, factor);
    }
    vdb.is_finite().then_some(vdb)
}

/// The character of a code in the Windows character set (Excel's CHAR).
fn windows_1252(code: u32) -> String {
    const HIGH: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C,
        0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let c = if (128..160).contains(&code) { HIGH[(code - 128) as usize] } else { code };
    char::from_u32(c).map(String::from).unwrap_or_default()
}

/// Text padded with zeros on the left to at least `width` characters.
fn zfill(s: Expr, width: usize) -> Expr {
    if width == 0 {
        return s;
    }
    let len = s.clone().str().len_chars().cast(DataType::Int64);
    let missing = lit(width as i64) - len.clone();
    when(len.lt(lit(width as i64))).then(lit("0".repeat(width)).str().head(missing) + s.clone()).otherwise(s)
}

/// Weekday of a serial day number, Monday 0 to Sunday 6 (serial 2 is a Monday).
fn weekday_index(serial: f64) -> i64 {
    (serial - 2.0).rem_euclid(7.0) as i64
}

fn modulo(e: Expr, m: f64) -> Expr {
    e.clone() - (e / lit(m)).floor() * lit(m)
}

/// Weekdays from serial 2 (a Monday) up to the day before `x`.
fn weekdays_before(x: Expr) -> Expr {
    let t = x - lit(2.0);
    let rest = modulo(t.clone(), 7.0);
    (t / lit(7.0)).floor() * lit(5.0) + when(rest.clone().gt(lit(5.0))).then(lit(5.0)).otherwise(rest)
}

/// The workday `n` workdays after (before, if negative) `start`, without holidays.
fn workday(start: Expr, n: Expr) -> Expr {
    let w = modulo(start.clone() - lit(2.0), 7.0); // Monday 0
    let forward = n.clone().gt(lit(0.0));
    // From a weekend day count from the Friday before (forward) or the Monday after.
    let base_forward = when(w.clone().gt_eq(lit(5.0))).then(start.clone() - (w.clone() - lit(4.0))).otherwise(start.clone());
    let base_back = when(w.clone().gt_eq(lit(5.0))).then(start.clone() + (lit(7.0) - w.clone())).otherwise(start.clone());
    let k = n.clone().abs();
    let weeks = (k.clone() / lit(5.0)).floor();
    let rest = k - weeks.clone() * lit(5.0);
    let wf = modulo(base_forward.clone() - lit(2.0), 7.0);
    let wb = modulo(base_back.clone() - lit(2.0), 7.0);
    let ahead = base_forward + weeks.clone() * lit(7.0) + rest.clone() + when((wf + rest.clone()).gt_eq(lit(5.0))).then(lit(2.0)).otherwise(lit(0.0));
    let back = base_back - weeks * lit(7.0) - rest.clone() - when((wb - rest).lt(lit(0.0))).then(lit(2.0)).otherwise(lit(0.0));
    when(n.clone().eq(lit(0.0))).then(start).otherwise(when(forward).then(ahead).otherwise(back))
}

/// 1 where a cell is not empty.
fn present(t: &T) -> Expr {
    match t {
        T::Dyn(d) => {
            let n = d.num.clone().is_not_null();
            let any = match &d.text {
                Some(x) => n.or(x.clone().is_not_null()),
                None => n,
            };
            any.cast(DataType::Float64)
        }
        _ => lit(1.0),
    }
}

/// A cell as a number in SUMPRODUCT: text and empty count as 0; and whether it is an error.
fn array_number(lets: &mut Lets, t: T) -> (Expr, Expr) {
    match t {
        T::Num(e) => {
            let e = bind(lets, e);
            (e.clone().fill_null(lit(0.0)), e.is_null())
        }
        T::Bool(_) | T::Text(_) => (lit(0.0), lit(false)),
        T::Dyn(d) => {
            let error = d.is_error();
            let n = d.num.clone();
            (when(n.clone().is_finite().fill_null(lit(false))).then(n).otherwise(lit(0.0)), error)
        }
    }
}

/// The number SUMIF adds for a cell (only numbers count) and whether it is an error.
fn summed_number(t: &T) -> (Expr, Expr) {
    match t {
        T::Num(e) => (e.clone().fill_null(lit(0.0)), e.clone().is_null()),
        T::Bool(_) | T::Text(_) => (lit(0.0), lit(false)),
        T::Dyn(d) => {
            let n = d.num.clone();
            let text = d.text.clone().map(|t| t.is_not_null()).unwrap_or(lit(false));
            (when(n.clone().is_finite().fill_null(lit(false)).and(text.clone().not())).then(n).otherwise(lit(0.0)), d.is_error())
        }
    }
}

/// Whether a cell holds a number.
fn is_number(t: &T) -> Expr {
    match t {
        T::Num(e) => e.clone().is_not_null(),
        T::Bool(_) | T::Text(_) => lit(false),
        T::Dyn(d) => {
            let n = d.num.clone().is_finite().fill_null(lit(false));
            match &d.text {
                Some(x) => n.and(x.clone().is_null()),
                None => n,
            }
        }
    }
}

fn fixed_criterion(text: &str) -> Lifted<Crit> {
    let t = text.trim_start();
    let (op, rest) = if let Some(r) = t.strip_prefix(">=") {
        (Some(BinaryOp::Ge), r)
    } else if let Some(r) = t.strip_prefix("<=") {
        (Some(BinaryOp::Le), r)
    } else if let Some(r) = t.strip_prefix("<>") {
        (Some(BinaryOp::Ne), r)
    } else if let Some(r) = t.strip_prefix('>') {
        (Some(BinaryOp::Gt), r)
    } else if let Some(r) = t.strip_prefix('<') {
        (Some(BinaryOp::Lt), r)
    } else if let Some(r) = t.strip_prefix('=') {
        (Some(BinaryOp::Eq), r)
    } else {
        (None, text)
    };
    if let Some(v) = text_as_number(rest, false).filter(|_| !rest.trim().is_empty()) {
        return Ok(Crit::Cmp(op.unwrap_or(BinaryOp::Eq), lit(v)));
    }
    Ok(match op {
        None | Some(BinaryOp::Eq) => Crit::Text { negate: false, pattern: rest.to_lowercase() },
        Some(BinaryOp::Ne) => Crit::Text { negate: true, pattern: rest.to_lowercase() },
        Some(_) => return Err("criterion ordering text".into()),
    })
}

/// Whether a cell meets a criterion.
fn matches(cell: &T, crit: &Crit) -> Expr {
    let (num, text, blank) = match cell {
        T::Num(e) => (Some(e.clone()), None, None),
        T::Text(t) => (None, Some(t.clone()), None),
        T::Bool(_) => (None, None, None),
        T::Dyn(d) => (Some(d.num.clone()), d.text.clone(), Some(d.is_blank())),
    };
    let number = num.clone().map(|n| when(n.clone().is_finite().fill_null(lit(false))).then(n).otherwise(null_num()));
    match crit {
        // "<>5" is met by every cell that is not the number 5, empty and text ones too.
        Crit::Cmp(op, v) => match number {
            Some(n) => compare_op(op, n, v.clone()).fill_null(lit(matches!(op, BinaryOp::Ne))),
            None => lit(matches!(op, BinaryOp::Ne)),
        },
        Crit::Text { negate, pattern } => {
            let lower = text.map(|t| t.str().to_lowercase());
            let hit = if pattern.is_empty() {
                // "" or "=" matches empty cells (and the empty text).
                let empty_text = lower.clone().map(|t| t.eq(lit(""))).unwrap_or(lit(false));
                match &blank {
                    Some(b) => b.clone().or(empty_text),
                    None => empty_text,
                }
            } else if pattern.contains(['*', '?', '~']) {
                lower.map(|t| t.str().contains(lit(wildcard_regex(pattern)), false)).unwrap_or(lit(false))
            } else {
                lower.map(|t| t.eq(lit(pattern.clone()))).unwrap_or(lit(false))
            };
            let hit = hit.fill_null(lit(false));
            if *negate {
                hit.not()
            } else {
                hit
            }
        }
        Crit::Eq(value) => {
            let (vn, vt) = match value {
                T::Num(e) => (Some(e.clone()), None),
                T::Text(t) => (None, Some(t.clone())),
                T::Dyn(d) => (Some(d.num.clone()), d.text.clone()),
                T::Bool(_) => (None, None),
            };
            let by_number = match (number, vn) {
                (Some(n), Some(v)) => n.eq(v).fill_null(lit(false)),
                _ => lit(false),
            };
            let by_text = match (text, vt) {
                (Some(t), Some(v)) => t.str().to_lowercase().eq(v.str().to_lowercase()).fill_null(lit(false)),
                _ => lit(false),
            };
            by_number.or(by_text)
        }
    }
}
