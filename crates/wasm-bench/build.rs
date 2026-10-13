//! Warn when this wasm-only crate is compiled for a non-wasm target, e.g. when
//! `cargo bench` is invoked from the workspace root (cargo does not read this
//! member's `.cargo/config.toml` there, so the wasm target is not applied).

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    if !target.starts_with("wasm32") {
        println!(
            "cargo:warning=mpz-wasm-bench benches only build for wasm32-unknown-unknown; \
             this build is for `{target}` and will fail. Run from the crate directory: \
             `cd crates/wasm-bench && cargo bench`."
        );
    }
}
