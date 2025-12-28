//! LOGOS entry point
//!
//! Dispatches between CLI mode and web UI based on compile features.

#[cfg(all(not(target_arch = "wasm32"), feature = "cli"))]
fn main() {
    if let Err(e) = logos::cli::run_cli() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(any(target_arch = "wasm32", not(feature = "cli")))]
fn main() {
    dioxus::launch(logos::ui::App);
}
