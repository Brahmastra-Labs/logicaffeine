//! `logos-sat`, as its suite spawns it: the shipped binary's one call (`crates/logicaffeine_proof/src/bin/logos-sat.rs`).

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    logicaffeine_proof::satcli::main()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
