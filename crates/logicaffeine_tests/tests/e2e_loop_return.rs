//! E2E Codegen Tests: loop-built fixed-size return buffer scalarization.
//!
//! A function whose return value is built by a LOOP over a FIXED-SIZE local
//! array, pushing a constant K elements per iteration, then returned — has a
//! statically-known length N = (array length) × K. It lowers to a `[T; N]` stack
//! array returned by value (a runtime fill cursor `out[__i]=…; __i+=1`), zero
//! heap. This is the streaming-hash DIGEST: `for word in h: push 4 bytes; return
//! out` where `h` is the fixed 4-word state — the last per-call heap allocation.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// `digest4` builds `h` (a fixed 2-slot local), then builds `out` by looping over
/// `h` pushing 2 bytes each (→ 4 slots), and returns it. The chain must ride
/// `[i64; 4]` — no heap `Vec` for the result — and compute 1+2+3+4 = 10.
#[test]
fn loop_built_return_buffer_scalarizes() {
    let code = r#"## To digest4 (a: Int) and (b: Int) -> Seq of Int:
    Let mutable h be a new Seq of Int.
    Push a to h.
    Push b to h.
    Let mutable out be a new Seq of Int.
    Repeat for x in h:
        Push x % 256 to out.
        Push (x / 256) % 256 to out.
    Return out.

## Main
Let d be digest4(513, 1027).
Show (item 1 of d) + (item 2 of d) + (item 3 of d) + (item 4 of d).
"#;
    // h=[513,1027]; out = [513%256=1, 513/256%256=2, 1027%256=3, 1027/256%256=4]; sum=10.
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("-> [i64; 4]"),
        "loop-built return over a fixed-2 array pushing 2/iter should return `[i64; 4]`, got:\n{}",
        rust
    );
    assert!(
        rust.contains("digest4") && !rust.contains("let mut out: Vec"),
        "the return buffer must not be a heap Vec, got:\n{}",
        rust
    );
}

/// A loop-built return over a VARIABLE-length input (a param, runtime length)
/// can't be a fixed array — it stays a heap `Seq`; output stays correct.
#[test]
fn loop_built_return_over_param_does_not_scalarize() {
    let code = r#"## To doubleAll (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Repeat for x in xs:
        Push x * 2 to out.
    Return out.

## Main
Let mutable xs be a new Seq of Int.
Push 3 to xs.
Push 4 to xs.
Push 5 to xs.
Let ys be doubleAll(xs).
Let mutable total be 0.
Repeat for y in ys:
    Set total to total + y.
Show total.
"#;
    // (3+4+5)*2 = 24.
    assert_exact_output(code, "24");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("-> [i64;"),
        "a loop-built return over a runtime-length param must not become a fixed array, got:\n{}",
        rust
    );
}
