//! xlg: corpus tool.  xlg manifest|roundtrip|recalc [corpus-dir] [out-dir]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use xl_golden::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let root = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "corpus".into()));
    let out = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "out".into()));
    let files = if cmd == "recalc-one" { Vec::new() } else { corpus_files(&root) };
    if cmd != "recalc-one" {
        fs::create_dir_all(&out).unwrap();
    }
    std::panic::set_hook(Box::new(|_| {}));
    match cmd {
        "manifest" => {
            let mut s = String::from("sha256\tbytes\tsource\tlicense\text\tpath\n");
            for f in &files {
                let data = fs::read(&f.path).unwrap();
                writeln!(s, "{}\t{}\t{}\t{}\t{}\t{}", sha256_hex(&data), data.len(), f.source, f.license, f.ext, f.rel).unwrap();
            }
            fs::write(root.join("manifest.tsv"), s).unwrap();
            println!("{} files -> {}", files.len(), root.join("manifest.tsv").display());
        }
        "roundtrip" => {
            let mut rows = String::from("status\tparts\trels_parts\tuntyped\tdangling\tmodel_mismatch\tmodel_errors\tpath\tdetail\n");
            let mut by: BTreeMap<(&str, &str), u64> = BTreeMap::new();
            let (mut parts, mut rels, mut untyped, mut dangling, mut mm, mut me) = (0, 0, 0, 0, 0, 0);
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str())) {
                let data = fs::read(&f.path).unwrap();
                let r = guarded(|| roundtrip(&data)).unwrap_or_else(|p| RoundTrip { status: "panic", detail: p, ..Default::default() });
                *by.entry((f.source, r.status)).or_default() += 1;
                parts += r.parts;
                rels += r.rels_parts;
                untyped += r.untyped_parts;
                dangling += r.dangling_rels;
                mm += r.model_mismatch;
                me += r.model_errors;
                writeln!(rows, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.status, r.parts, r.rels_parts, r.untyped_parts, r.dangling_rels, r.model_mismatch, r.model_errors, f.rel, r.detail.replace(['\t', '\n'], " ")).unwrap();
            }
            fs::write(out.join("roundtrip.tsv"), rows).unwrap();
            for ((src, st), n) in &by {
                println!("{src:16} {st:12} {n}");
            }
            println!("parts {parts}, rels parts {rels}, untyped parts {untyped}, dangling targets {dangling}, model mismatch {mm}, model errors {me}");
        }
        "recalc-one" => {
            // Child process: one file, result as one line on stdout.
            let data = fs::read(&root).unwrap();
            let evaluate = true;
            let legacy = args.get(3).filter(|p| !p.is_empty()).map(|p| fs::read(p).unwrap());
            let r = guarded(|| recalc::recalc_ironcalc(&data, evaluate, legacy.as_deref())).unwrap_or_else(|p| recalc::RecalcStats { status: "panic", detail: p, ..Default::default() });
            let fx: Vec<String> = r.by_function.iter().map(|(k, v)| format!("{k}:{}:{}", v.0, v.1)).collect();
            let clean = |s: &str| s.replace(['\t', '\n', '\r', '\u{1}'], " ");
            println!("@@RESULT\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.status, r.formula_cells, r.skipped, r.exact, r.close, r.diff, r.unsupported, clean(&r.detail).chars().take(200).collect::<String>(), fx.join(","), r.examples.iter().map(|e| clean(e)).collect::<Vec<_>>().join("\u{1}"), r.loose);
        }
        "recalc" => {
            // Each file runs in its own process with a time limit: a hang or crash in the
            // engine under test must not take the harness down.
            let limit = std::time::Duration::from_secs(args.get(4).and_then(|s| s.parse().ok()).unwrap_or(60));
            // "legacy": expected values come from the legacy original (Excel's cached values)
            // instead of the values LibreOffice computed during conversion.
            let use_legacy = args.get(5).map(String::as_str) == Some("legacy");
            let only = args.get(6).cloned().unwrap_or_default();
            let exe = std::env::current_exe().unwrap();
            let mut rows = String::from("status\tformula_cells\tskipped\texact\tclose\tdiff\tunsupported\tseconds\tpath\tdetail\tloose\n");
            let mut total = recalc::RecalcStats::default();
            let mut status: BTreeMap<String, u64> = BTreeMap::new();
            let mut examples = String::new();
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str()) && f.rel.starts_with(&only)) {
                let t0 = std::time::Instant::now();
                let legacy = if use_legacy { legacy_original(&root, &f.rel) } else { None };
                if use_legacy && legacy.is_none() {
                    continue;
                }
                let mut child = std::process::Command::new(&exe).arg("recalc-one").arg(&f.path).arg(legacy.unwrap_or_default()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
                let mut pipe = child.stdout.take().unwrap();
                let reader = std::thread::spawn(move || {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut pipe, &mut s).ok();
                    s
                });
                let finished = loop {
                    match child.try_wait().unwrap() {
                        Some(_) => break true,
                        None if t0.elapsed() > limit => {
                            child.kill().ok();
                            child.wait().ok();
                            break false;
                        }
                        None => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                };
                let s = reader.join().unwrap_or_default();
                // The engine under test may print to stdout; only the marked line counts.
                let line = match s.lines().rev().find_map(|l| l.strip_prefix("@@RESULT\t")) {
                    Some(l) => l.to_string(),
                    None if !finished => format!("timeout\t0\t0\t0\t0\t0\t0\tno result after {}s\t\t", limit.as_secs()),
                    None => "crash\t0\t0\t0\t0\t0\t0\tprocess died\t\t".to_string(),
                };
                let c: Vec<&str> = line.trim_end_matches('\n').split('\t').collect();
                let n = |i: usize| c.get(i).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                *status.entry(c[0].to_string()).or_default() += 1;
                total.formula_cells += n(1);
                total.skipped += n(2);
                total.exact += n(3);
                total.close += n(4);
                total.diff += n(5);
                total.unsupported += n(6);
                total.loose += n(10);
                for item in c.get(8).unwrap_or(&"").split(',').filter(|s| !s.is_empty()) {
                    let mut it = item.rsplitn(3, ':');
                    let (d, k, name) = (it.next().unwrap_or("0"), it.next().unwrap_or("0"), it.next().unwrap_or(""));
                    let e = total.by_function.entry(name.to_string()).or_default();
                    e.0 += k.parse::<u64>().unwrap_or(0);
                    e.1 += d.parse::<u64>().unwrap_or(0);
                }
                for e in c.get(9).unwrap_or(&"").split('\u{1}').filter(|s| !s.is_empty()) {
                    writeln!(examples, "{}\t{}", f.rel, e).unwrap();
                }
                writeln!(rows, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{}\t{}\t{}", c[0], n(1), n(2), n(3), n(4), n(5), n(6), t0.elapsed().as_secs_f64(), f.rel, c.get(7).unwrap_or(&""), n(10)).unwrap();
            }
            fs::write(out.join("recalc.tsv"), rows).unwrap();
            fs::write(out.join("recalc-examples.tsv"), examples).unwrap();
            let mut fx = String::from("function\tcells\tdiff\n");
            let mut v: Vec<_> = total.by_function.iter().collect();
            v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (k, (n, d)) in &v {
                writeln!(fx, "{k}\t{n}\t{d}").unwrap();
            }
            fs::write(out.join("recalc-functions.tsv"), fx).unwrap();
            println!("files: {status:?}");
            let compared = total.exact + total.close + total.loose + total.diff;
            println!("formula cells {}, skipped {}, compared {}", total.formula_cells, total.skipped, compared);
            println!("exact {} ({:.2}%), close {}, loose {}, diff {} (unsupported {})", total.exact, 100.0 * total.exact as f64 / compared.max(1) as f64, total.close, total.loose, total.diff, total.unsupported);
            println!("distinct functions {}", v.len());
        }
        "formulas" => {
            // Gate for the formula parser: every formula in the corpus parses and prints back identically.
            use calamine::Reader;
            let (mut total, mut ok, mut mismatch, mut failed, mut unreadable) = (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut reasons: BTreeMap<String, (u64, String)> = BTreeMap::new();
            let mut functions: BTreeMap<String, u64> = BTreeMap::new();
            let mut fails = String::from("kind\tpath\tformula\tdetail\n");
            // Formulas that point into another workbook, and files that contain any.
            let (mut external, mut files_external, mut files_with_formulas) = (0u64, 0u64, 0u64);
            let only = args.get(4).cloned().unwrap_or_default();
            let mut per_file = String::from("path\texternal\n");
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str()) && f.rel.starts_with(&only)) {
                let Ok(data) = fs::read(&f.path) else { continue };
                let mut file_external = false;
                let formulas = guarded(move || {
                    let mut out: Vec<String> = Vec::new();
                    let Ok(mut wb) = calamine::Xlsx::new(std::io::Cursor::new(data)) else { return None };
                    for name in wb.sheet_names() {
                        if let Ok(range) = wb.worksheet_formula(&name) {
                            out.extend(range.cells().filter(|c| !c.2.is_empty()).map(|c| c.2.clone()));
                        }
                    }
                    out.extend(wb.defined_names().iter().filter(|(_, f)| !f.is_empty()).map(|(_, f)| f.clone()));
                    Some(out)
                });
                let Ok(Some(formulas)) = formulas else {
                    unreadable += 1;
                    continue;
                };
                files_with_formulas += !formulas.is_empty() as u64;
                for text in formulas {
                    total += 1;
                    let first_time = distinct.insert(text.clone());
                    match xl_formula::parse(&text) {
                        Ok(e) if e.to_string() == text => {
                            ok += 1;
                            let mut ext = false;
                            e.walk(&mut |n| {
                                if let xl_formula::Expr::Ref { prefix: Some(p), .. } | xl_formula::Expr::Name { prefix: Some(p), .. } = n {
                                    ext |= p.contains('[');
                                }
                            });
                            external += ext as u64;
                            file_external |= ext;
                            if first_time {
                                for n in e.functions() {
                                    *functions.entry(n).or_default() += 1;
                                }
                            }
                        }
                        Ok(e) => {
                            mismatch += 1;
                            writeln!(fails, "mismatch\t{}\t{}\t{}", f.rel, text.replace(['\t', '\n'], " "), e.to_string().replace(['\t', '\n'], " ")).unwrap();
                        }
                        Err(e) => {
                            failed += 1;
                            let r = reasons.entry(e.message.clone()).or_insert((0, text.chars().take(90).collect()));
                            r.0 += 1;
                            if first_time {
                                writeln!(fails, "error\t{}\t{}\t{}", f.rel, text.replace(['\t', '\n'], " ").chars().take(300).collect::<String>(), e).unwrap();
                            }
                        }
                    }
                }
                files_external += file_external as u64;
                writeln!(per_file, "{}\t{}", f.rel, file_external as u8).unwrap();
            }
            fs::write(out.join("formulas-files.tsv"), per_file).unwrap();
            println!("files with formulas {files_with_formulas}, of these with references into other workbooks {files_external}; such formulas {external}");
            fs::write(out.join("formulas-failed.tsv"), fails).unwrap();
            let mut fx: Vec<_> = functions.iter().collect();
            fx.sort_by(|a, b| b.1.cmp(a.1));
            fs::write(out.join("formulas-functions.tsv"), fx.iter().fold(String::from("function\tdistinct_formulas\n"), |mut s, (k, v)| {
                writeln!(s, "{k}\t{v}").unwrap();
                s
            })).unwrap();
            println!("formulas {total} (distinct {}), identical {ok} ({:.3}%), mismatch {mismatch}, parse errors {failed}, unreadable files {unreadable}", distinct.len(), 100.0 * ok as f64 / total.max(1) as f64);
            println!("distinct functions {}", fx.len());
            let mut r: Vec<_> = reasons.iter().collect();
            r.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (msg, (n, example)) in r {
                println!("  {n:6} {msg}: {example}");
            }
        }
        "cells" => {
            // Cell reader against calamine, plus read → write → read.
            let only = args.get(4).cloned().unwrap_or_default();
            let mut rows = String::from("path\tstatus\tcells\tvalue_diff\tonly_ours\tonly_calamine\tformulas\tformula_diff\tformula_only_ours\tformula_only_calamine\troundtrip\texamples\n");
            let mut t = xl_golden::cells::CellStats::default();
            let mut by_status: std::collections::BTreeMap<String, u64> = Default::default();
            let (mut clean, mut rt_ok, mut rt_n) = (0u64, 0u64, 0u64);
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str()) && f.rel.starts_with(&only)) {
                let Ok(data) = fs::read(&f.path) else { continue };
                if std::env::var_os("XLG_TRACE").is_some() {
                    eprintln!("{}", f.rel);
                }
                let st = guarded(move || xl_golden::cells::compare(&data)).unwrap_or_else(|e| xl_golden::cells::CellStats { status: "panic".into(), examples: vec![e], ..Default::default() });
                *by_status.entry(st.status.clone()).or_default() += 1;
                let rt = st.roundtrip.map(|b| if b { "same" } else { "DIFF" }).unwrap_or("-");
                writeln!(rows, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", f.rel, st.status, st.cells, st.value_diff, st.only_ours, st.only_calamine, st.formulas, st.formula_diff, st.formula_only_ours, st.formula_only_calamine, rt, st.examples.join(" | ").replace(['\t', '\n', '\r'], " ")).unwrap();
                if st.status == "ok" && st.value_diff + st.only_ours + st.only_calamine + st.formula_diff + st.formula_only_ours + st.formula_only_calamine == 0 {
                    clean += 1;
                }
                if let Some(b) = st.roundtrip {
                    rt_n += 1;
                    rt_ok += b as u64;
                }
                t.cells += st.cells;
                t.whitespace += st.whitespace;
                t.skipped_sparse += st.skipped_sparse;
                t.value_diff += st.value_diff;
                t.only_ours += st.only_ours;
                t.only_calamine += st.only_calamine;
                t.formulas += st.formulas;
                t.formula_diff += st.formula_diff;
                t.formula_only_ours += st.formula_only_ours;
                t.formula_only_calamine += st.formula_only_calamine;
            }
            fs::write(out.join("cells.tsv"), rows).unwrap();
            println!("files by status: {by_status:?}; identical to calamine: {clean}; round trip same: {rt_ok} of {rt_n}");
            println!("values: {} compared, {} differ, {} only ours, {} only calamine; {} differ only by calamine's whitespace trimming; {} sheets too sparse for calamine", t.cells, t.value_diff, t.only_ours, t.only_calamine, t.whitespace, t.skipped_sparse);
            println!("formulas: {} compared, {} differ, {} only ours, {} only calamine", t.formulas, t.formula_diff, t.formula_only_ours, t.formula_only_calamine);
        }
        "rewrite" => {
            // Read every corpus workbook and write it anew, for checks by other implementations.
            let only = args.get(4).cloned().unwrap_or_default();
            let dir = out.join("rewritten");
            fs::create_dir_all(&dir).unwrap();
            let mut index = String::from("file\tsource\n");
            let mut n = 0u32;
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str()) && f.rel.starts_with(&only)) {
                let Ok(data) = fs::read(&f.path) else { continue };
                if data.len() > 3_000_000 {
                    continue;
                }
                let Ok(Some(bytes)) = guarded(move || {
                    let mut wb = xl_cells::read_bytes(data).ok()?;
                    if wb.sheets.iter().any(|s| s.kind != xl_cells::SheetKind::Worksheet) {
                        return None;
                    }
                    if wb.sheets.iter().all(|s| s.visibility != xl_cells::Visibility::Visible) {
                        wb.sheets.first_mut()?.visibility = xl_cells::Visibility::Visible;
                    }
                    xl_cells::write_bytes(&wb).ok()
                }) else {
                    continue;
                };
                n += 1;
                let name = format!("{n:05}.xlsx");
                fs::write(dir.join(&name), bytes).unwrap();
                writeln!(index, "{name}\t{}", f.rel).unwrap();
            }
            fs::write(dir.join("index.tsv"), index).unwrap();
            println!("{n} workbooks rewritten into {}", dir.display());
        }
        "shape" => {
            // How column-shaped are the formulas? Basis for the dataframe question.
            let only = args.get(4).cloned().unwrap_or_default();
            let mut total = shape::ShapeStats::default();
            let (mut files_n, mut files_columnar) = (0u64, 0u64);
            let mut rows = String::from("path\tformula_cells\tin_vertical_runs\n");
            for f in files.iter().filter(|f| OOXML_EXT.contains(&f.ext.as_str()) && f.rel.starts_with(&only)) {
                let Ok(data) = fs::read(&f.path) else { continue };
                let Ok(Some(st)) = guarded(move || shape::analyse(&data)) else { continue };
                if st.formula_cells == 0 {
                    continue;
                }
                files_n += 1;
                let vertical: u64 = st.by_kind.values().map(|v| v.0).sum();
                files_columnar += (vertical * 10 >= st.formula_cells * 9) as u64;
                writeln!(rows, "{}\t{}\t{}", f.rel, st.formula_cells, vertical).unwrap();
                total.formula_cells += st.formula_cells;
                total.vertical_runs += st.vertical_runs;
                total.sheets += st.sheets;
                total.sheets_fully_columnar += st.sheets_fully_columnar;
                for (k, v) in st.by_kind {
                    let e = total.by_kind.entry(k).or_default();
                    e.0 += v.0;
                    e.1 += v.1;
                    e.2 += v.2;
                }
            }
            fs::write(out.join("shape.tsv"), rows).unwrap();
            let n = total.formula_cells.max(1) as f64;
            println!("files with formulas {files_n}, at least 90% in vertical runs: {files_columnar}; sheets {} (fully columnar {})", total.sheets, total.sheets_fully_columnar);
            println!("formula cells {}, vertical runs {}", total.formula_cells, total.vertical_runs);
            println!("{:16} {:>10} {:>7} {:>10} {:>7} {:>10} {:>7}", "kind", "vertical", "%", "horizontal", "%", "single", "%");
            let (mut v, mut h, mut s1) = (0u64, 0u64, 0u64);
            for (k, c) in &total.by_kind {
                println!("{:16} {:>10} {:>6.1}% {:>10} {:>6.1}% {:>10} {:>6.1}%", k.label(), c.0, 100.0 * c.0 as f64 / n, c.1, 100.0 * c.1 as f64 / n, c.2, 100.0 * c.2 as f64 / n);
                v += c.0;
                h += c.1;
                s1 += c.2;
            }
            println!("{:16} {:>10} {:>6.1}% {:>10} {:>6.1}% {:>10} {:>6.1}%", "all", v, 100.0 * v as f64 / n, h, 100.0 * h as f64 / n, s1, 100.0 * s1 as f64 / n);
        }
        _ => eprintln!("usage: xlg manifest|roundtrip|recalc|formulas [corpus-dir] [out-dir]"),
    }
}
