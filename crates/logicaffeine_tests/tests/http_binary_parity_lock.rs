//! G8 — binary HTTP response parity, the codegen half (fast canary).
//!
//! The interpreter tier honors a `Response`'s non-empty `bytes` field as the raw
//! body (a `.wasm`, an image — `value_to_http_response`), and `readBytes` runs on
//! the interpreter/VM tiers. The native-AOT lowering must match: a compiled LOGOS
//! service that serves binary content has to ship the SAME bytes, and a program
//! that passed `check` and ran on the interpreter must not fail `cargo build`
//! because `readBytes` was emitted as an unresolved bare call.
//!
//! These are compile-only canaries so the default suite catches a regression
//! cheaply; the ground truth is the live differential
//! (`crates/logicaffeine_tests/tests/http_differential.rs`,
//! `interpreter_and_aot_serve_binary_bodies_byte_identical`), which probes both
//! running tiers for byte-identical status + headers + body across empty, text,
//! WASM, and arbitrary binary responses.

use logicaffeine_compile::compile::compile_to_rust;

const SERVE_BYTES_PROGRAM: &str = "\
# Site

## To handle (request: Request) -> Response:
    If request's path is equal to \"/wasm\":
        Return bytesResponse(\"application/wasm\", [0, 97, 115, 109, 1, 0, 0, 0]).
    Return plain(\"text\").

## Main
Serve on port 8099 with handle.
";

/// The emitted `Serve` closure must consult the handler's `bytes` field when
/// marshaling the reply — a non-empty byte body wins over the text `body`,
/// exactly as the interpreter's `value_to_http_response` decides it.
#[test]
fn the_serve_lowering_marshals_the_response_bytes_field() {
    let rust = compile_to_rust(SERVE_BYTES_PROGRAM).expect("the serve program compiles");
    assert!(
        rust.contains("__resp.bytes"),
        "the AOT `Serve` lowering never reads `__resp.bytes` — binary bodies \
         (bytesResponse: .wasm, images) are silently dropped on the compiled tier \
         while the interpreter serves them.\n--- generated Rust ---\n{rust}"
    );
}

const READ_BYTES_PROGRAM: &str = "\
## Main
Let b be readBytes(\"/tmp/logos_g8_probe.bin\").
Show item 1 of b.
";

/// `readBytes` is a real builtin on the interpreter/VM tiers (`std::fs::read` →
/// `Seq of Int`); the AOT expression lowering must emit a real file read, not a
/// bare `readBytes(...)` call that fails `cargo build` with E0425.
#[test]
fn read_bytes_lowers_to_a_real_file_read_in_aot_rust() {
    let rust = compile_to_rust(READ_BYTES_PROGRAM).expect("the readBytes program compiles");
    assert!(
        rust.contains("std::fs::read"),
        "the AOT lowering for `readBytes` does not emit a file read — the \
         generated Rust cannot compile (unresolved name), so a program that is \
         green on the interpreter fails native build.\n--- generated Rust ---\n{rust}"
    );
}
