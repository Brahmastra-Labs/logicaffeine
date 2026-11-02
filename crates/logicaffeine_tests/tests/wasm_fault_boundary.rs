//! The Studio's fault boundary in real wasm, where a panic is not an unwind but a trap.
//!
//! The web build aborts on panic, and in wasm an abort is the `unreachable` instruction: a trap
//! that throws out of every frame of the module at once. Running out of stack is a trap too —
//! the engine's own stack ends in a `RangeError`, the module's shadow stack (the linear-memory
//! stack that holds any local whose address is taken) in an out-of-bounds access once the stack
//! pointer has run below zero. Each is caught here as the Studio meets it, and the module must
//! keep working afterwards: the shadow-stack pointer the trap left behind has to be put back
//! before any Rust runs again, or the next call dies on the same exhausted stack.
//!
//! Only builds on `wasm32`; `scripts/wasm/test-wasm-node.sh` runs it under node with the
//! wasm-bindgen test runner:
//!   cargo test -p logicaffeine-tests --no-default-features --features web-tests \
//!     --target wasm32-unknown-unknown --test wasm_fault_boundary
//! The test binary exports `__stack_pointer` through this crate's `build.rs`, as the web app's
//! own build does.

#![cfg(feature = "web-tests")]

#![cfg(target_arch = "wasm32")]

use logicaffeine_web::ui::fault::{contain, Cause};
use std::hint::black_box;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

/// Recurses `depth` levels deep, each level holding two KiB of shadow stack: the array's address
/// escapes into `black_box`, so it cannot live in wasm locals.
#[inline(never)]
fn burrow(depth: u64) -> u64 {
    let mut frame = [depth; 256];
    black_box(&mut frame);
    if depth == 0 {
        return frame[0];
    }
    frame[0] + burrow(depth - 1) + frame[255]
}

/// Recurses until the engine's own stack runs out, touching no shadow stack.
#[inline(never)]
fn spiral(depth: u64) -> u64 {
    black_box(spiral(black_box(depth + 1))) + 1
}

#[wasm_bindgen_test]
fn work_that_finishes_gives_back_its_value() {
    assert_eq!(contain(|| 6 * 7), Ok(42));
}

#[wasm_bindgen_test]
fn a_panic_is_contained_with_its_message_and_place() {
    let fault = contain(|| -> () { panic!("the kernel lost its place") }).expect_err("the panic was contained");
    assert_eq!(fault.cause, Cause::Panic);
    assert_eq!(fault.message, "the kernel lost its place");
    let location = fault.location.expect("a panic has a location");
    assert!(location.contains("wasm_fault_boundary.rs"), "the location names another file: {location}");
}

#[wasm_bindgen_test]
fn the_module_keeps_working_after_a_contained_panic() {
    let _ = contain(|| -> () { panic!("first") });
    let words: Vec<String> = (0..10_000).map(|n| format!("word {n}")).collect();
    assert_eq!(words[9_999], "word 9999");
    assert_eq!(contain(|| words.len()), Ok(10_000));
    assert_eq!(contain(|| burrow(200)), Ok(burrow(200)));
}

#[wasm_bindgen_test]
fn running_out_of_shadow_stack_is_contained() {
    let fault = contain(|| burrow(1 << 20)).expect_err("a million two-KiB frames cannot fit");
    assert_eq!(fault.cause, Cause::StackExhausted, "{fault:?}");
    assert_eq!(fault.message, "ran out of stack space");
    // The stack is whole again: two hundred levels fit as they did before.
    assert_eq!(contain(|| burrow(200)), Ok(burrow(200)));
}

#[wasm_bindgen_test]
fn running_out_of_the_engines_stack_is_contained() {
    let fault = contain(|| spiral(0)).expect_err("endless recursion cannot finish");
    assert_eq!(fault.cause, Cause::StackExhausted, "{fault:?}");
    assert_eq!(contain(|| burrow(200)), Ok(burrow(200)));
}

#[wasm_bindgen_test]
fn a_thousand_contained_faults_leave_the_stack_whole() {
    for round in 0..1_000u64 {
        let fault = contain(|| -> () { panic!("round {round}") }).expect_err("the panic was contained");
        assert_eq!(fault.message, format!("round {round}"));
    }
    for _ in 0..20 {
        assert_eq!(contain(|| burrow(1 << 20)).map_err(|f| f.cause), Err(Cause::StackExhausted));
    }
    assert_eq!(contain(|| burrow(300)), Ok(burrow(300)));
}

#[wasm_bindgen_test]
fn a_fault_inside_a_fault_boundary_stays_inside() {
    let outer = contain(|| contain(|| -> () { panic!("inner") }));
    let inner = outer.expect("the outer boundary saw the inner fault").expect_err("the inner work faulted");
    assert_eq!(inner.message, "inner");
    assert_eq!(contain(|| "after"), Ok("after"));
}

#[wasm_bindgen_test]
fn the_module_exports_its_stack_pointer_for_the_boundary() {
    let global = js_sys::Reflect::get(&wasm_bindgen::exports(), &JsValue::from_str("__stack_pointer"))
        .expect("the exports object can be read");
    let global: js_sys::WebAssembly::Global = global
        .dyn_into()
        .unwrap_or_else(|v| panic!("__stack_pointer is not an exported global: {v:?}"));
    let value = global.value().as_f64().expect("the stack pointer is a number");
    assert!(value > 0.0, "the stack pointer reads {value}");
}
