//! `store_time FILE`: how long the steps of a store take for a workbook.
use std::time::Instant;
fn main() {
    let path = std::env::args().nth(1).expect("a workbook");
    let bytes = std::fs::read(&path).unwrap();
    let xls = path.to_lowercase().ends_with(".xls");
    let wb = if xls { xl_xls::read_bytes(&bytes).unwrap() } else { xl_cells::read_bytes(bytes.clone()).unwrap() };
    let dir = std::env::temp_dir().join(format!("xl-store-time-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let t = Instant::now();
    let notes = xl_engine::store::notes(&xl_polars::builtin::PolarsExecutor::default(), &wb);
    println!("notes {:?} ({} regions)", t.elapsed(), notes.len());
    let t = Instant::now();
    xl_polars::store::create(&dir, &bytes, &wb, !xls).unwrap();
    println!("create {:?}", t.elapsed());
    let b = xl_polars::Polars::default();
    let files = xl_engine::store::noted_files(&b, &dir).unwrap();
    let t = Instant::now();
    let mut frames: Vec<_> = files.iter().map(|(p, _)| xl_engine::store::Backend::read_parquet(&b, p).unwrap()).collect();
    println!("read {:?}", t.elapsed());
    let t = Instant::now();
    let notes: Vec<_> = files.iter().map(|(_, n)| n.clone()).collect();
    let r = xl_engine::plans::recalculate(xl_engine::store::Backend::executor(&b), &notes, &mut frames).unwrap();
    println!("recalculate {:?}: done {} kept {}", t.elapsed(), r.done.len(), r.kept.len());
    let t = Instant::now();
    for ((p, n), f) in files.iter().zip(&frames) {
        xl_engine::store::Backend::write_parquet(&b, p, f, Some(&n.to_string())).unwrap();
    }
    println!("write {:?}", t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
