// The engine goes deep on long formulas: the module gets a stack of 16 MB (the default
// of WebAssembly modules built by Rust is 1 MB).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("wasm") {
        println!("cargo:rustc-link-arg=-zstack-size=16777216");
    }
}
