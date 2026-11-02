//! AOT locks for `Seq`-typed struct fields — the shape a bytes-carrying
//! `Response` (`a bytes, which is Seq of Int`) uses to serve binary content in a
//! single native binary. These compile the LOGOS program through the real Rust
//! codegen AND rustc-compile + run it, proving the `Seq` field is emitted and the
//! constructor/reads are valid Rust — NOT dead-elided (an earlier concern).

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

/// A `Seq of Int` field constructed and read back must round-trip through AOT.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn aot_seq_field_constructs_and_reads() {
    assert_exact_output(
        "## A Blob has:\n    A bytes: Seq of Int.\n\n## Main\nLet b be a new Blob with bytes [72, 73, 74].\nShow item 1 of b's bytes.\nShow item 3 of b's bytes.\n",
        "72\n74",
    );
}

/// The bytes-serving shape: a `Seq of Int` field is SET but never read in LOGOS
/// (only the runtime consumes it). It must not be dead-elided — else the emitted
/// `Payload { …, data: … }` constructor would fail to rustc-compile. (Uses a
/// non-stdlib name so the auto-imported `http` `Response` never collides.)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn aot_unread_seq_field_is_not_elided() {
    assert_exact_output(
        "## A Payload has:\n    A code: Int.\n    A data: Seq of Int.\n\n## Main\nLet p be a new Payload with code 200 and data [1, 2, 3].\nShow p's code.\n",
        "200",
    );
}
