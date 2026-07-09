//! Regression pins for Bug Report #1 — FFI / C ABI boundary (BUG-008, BUG-009).

use logicaffeine_compile::compile_program_full;

/// BUG-008: a `Char` param/return must cross the C ABI as `u32` (matching the
/// `uint32_t` in the generated header), validated via `char::from_u32` — never
/// as a bare Rust `char` (an out-of-range `u32` from C materialized as a `char`
/// is instant UB).
#[test]
fn char_export_crosses_abi_as_validated_u32() {
    let src = "## To shift (c: Char) -> Char is exported:\n    Return c.\n\n## Main\nShow 42.\n";
    let out = compile_program_full(src).expect("compile");
    let rust = &out.rust_code;
    assert!(
        rust.contains("char::from_u32"),
        "Char C-export boundary must validate via char::from_u32, not expose a raw `char`:\n{}",
        rust
    );
    assert!(
        rust.contains("logos_shift(c: u32)"),
        "exported Char param must cross the ABI as u32 (matching uint32_t):\n{}",
        rust
    );
}

/// BUG-009: a reference-type handle param must NOT be dereferenced with a
/// panicking `.expect()` OUTSIDE the catch_unwind boundary — a NULL/stale handle
/// from C would unwind across the `extern "C"` frame (UB/abort). It must surface
/// the error and return gracefully, like the standalone accessors.
#[test]
fn ref_param_handle_lookup_never_panics_across_extern_c() {
    let src = "## To total (xs: Seq of Int) -> Int is exported:\n    Return 0.\n\n## Main\nShow 42.\n";
    let out = compile_program_full(src).expect("compile");
    let rust = &out.rust_code;
    assert!(
        !rust.contains(".deref(__id).expect("),
        "handle param must not be dereferenced with a panicking .expect() across extern \"C\":\n{}",
        rust
    );
    assert!(
        rust.contains("InvalidHandle: handle"),
        "a bad handle must be surfaced as an error (graceful return), not a panic:\n{}",
        rust
    );
}
