//! The ONE float-display path.
//!
//! Every engine that renders a LOGOS `Float` — the tree-walker, the bytecode
//! VM, the AOT-compiled binary, and the direct-WASM host — must agree on the
//! decimal string for the same `f64`, or the same program prints different
//! answers depending on how it was run. This module is the single authority.
//!
//! The contract of [`fmt_f64`]:
//!
//! - **Shortest round-trip**: the fewest decimal digits that parse back to the
//!   exact same bits (Rust's `{}` Display, Grisu/Dragon in std). A typed
//!   literal echoes as typed; `1.0 / 3.0` shows all 17 significant digits.
//! - **Never scientific notation**: `0.0000001` renders as `0.0000001`, not
//!   `1e-7` — and a nonzero value NEVER renders as `0`.
//! - **Integral floats stay bare**: `2.0` renders as `2`.
//! - **Bounded width**: the longest possible output (the smallest subnormal,
//!   `5e-324`) is under 340 bytes — the direct-WASM host's scratch size.

/// Renders an `f64` exactly as LOGOS displays it on every engine.
pub fn fmt_f64(f: f64) -> String {
    format!("{f}")
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/fmt/tests.rs"]
mod tests;
