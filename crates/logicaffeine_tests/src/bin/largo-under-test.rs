//! `largo`, as the CLI suites spawn it: the shipped binary's one call (`apps/logicaffeine_cli/src/main.rs`).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    logicaffeine_cli::largo_main();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
