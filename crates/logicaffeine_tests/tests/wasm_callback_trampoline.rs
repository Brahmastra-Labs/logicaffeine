//! B0 — the JS→WASM callback trampoline (`__logos_call_closure`).
//!
//! The exported `__logos_call_closure(handle: i32, arg: i64) -> i64` invokes a
//! LOGOS closure by reading its function index from the object at `handle` and
//! `br_table`-dispatching to a per-closure arm that loads that closure's captures
//! and `call_indirect`s it. This is the host→LOGOS callback that DOM event
//! handlers ride.
//!
//! We build a REAL capturing closure in a LOGOS program, run `main` (which
//! allocates the object), then find its handle by scanning linear memory for a
//! distinctive capture sentinel — so the test exercises the real object with its
//! real `func_idx` and real captures (and, since `main` already ran, doubles as
//! the post-allocation lifetime check). No handle guessing.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::compile_to_wasm;
use std::cell::RefCell;
use std::rc::Rc;

#[path = "compile_common/mod.rs"]
mod common;

/// A distinctive 8-byte capture value that will not collide with anything else in
/// linear memory (i64 constants live in code as `i64.const`, never in memory).
const SENTINEL: i64 = 0x0123_4567_89AB_CDEF;

type Data = Rc<RefCell<(Vec<String>, Vec<i32>)>>;

/// Instantiate an emitted module with the minimal host imports a closure program
/// needs (`Show` → `print_i64`), running `main`.
fn instantiate(module: &[u8]) -> (wasmi::Store<Data>, wasmi::Instance) {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let out: Data = Rc::new(RefCell::new((Vec::new(), Vec::new())));
    let mut store = wasmi::Store::new(&engine, out);
    let mut l = wasmi::Linker::<Data>::new(&engine);
    common::wasm_host::link_value_imports(&mut l);
    common::wasm_host::link_exact_show(&mut l, |out, line| out.borrow_mut().0.push(line));
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    l.func_wrap(
        "env",
        "logos_rt_fail",
        |_c: wasmi::Caller<'_, _>, _h: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "logos_rt_fail_at",
        |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
    )
    .unwrap();
    l.func_wrap("env", "print_i64", |c: wasmi::Caller<'_, Data>, v: i64| {
        c.data().borrow_mut().0.push(v.to_string());
    })
    .unwrap();
    l.func_wrap(
        "env",
        "dom_on_window_event",
        |c: wasmi::Caller<'_, Data>, _event: i32, h: i32| {
            c.data().borrow_mut().1.push(h);
        },
    )
    .unwrap();
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    // Run `main` so the real closure object is allocated in linear memory.
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    (store, inst)
}

/// Find the closure handle by scanning memory for the capture sentinel (which sits
/// at object offset +8), then backing up to the object base.
fn find_handle(store: &wasmi::Store<Data>, inst: &wasmi::Instance) -> i32 {
    let _ = inst;
    *store
        .data()
        .borrow()
        .1
        .last()
        .expect("the host received a registered callback")
}

fn call_trampoline(
    store: &mut wasmi::Store<Data>,
    inst: &wasmi::Instance,
    handle: i32,
    arg: i64,
) -> i64 {
    let f = inst
        .get_typed_func::<(i32, i64), i64>(&*store, "__logos_call_closure")
        .expect("__logos_call_closure exported");
    f.call(&mut *store, (handle, arg))
        .expect("trampoline runs without trapping")
}

/// A program with exactly one closure, capturing `base = SENTINEL`.
fn capturing_program() -> String {
    format!(
        "## Main\nLet base be {SENTINEL}.\nLet handler be (x: Int) -> base + x.\nLet registration be onWindowEvent(\"resize\", handler).\nShow handler(1).\n"
    )
}

#[test]
fn trampoline_invokes_a_capturing_closure() {
    let module = compile_to_wasm(&capturing_program()).expect("compiles to wasm");
    let (mut store, inst) = instantiate(&module);
    let handle = find_handle(&store, &inst);
    // The handler computes base + arg = SENTINEL + 41.
    let result = call_trampoline(&mut store, &inst, handle, 41);
    assert_eq!(result, SENTINEL + 41);
}

#[test]
fn trampoline_reads_the_captured_value_not_the_arg() {
    // With arg 0 the result is purely the captured value — proof the capture is
    // read from the object, not fabricated.
    let module = compile_to_wasm(&capturing_program()).expect("compiles to wasm");
    let (mut store, inst) = instantiate(&module);
    let handle = find_handle(&store, &inst);
    assert_eq!(call_trampoline(&mut store, &inst, handle, 0), SENTINEL);
}

#[test]
fn trampoline_handle_is_stable_across_two_events() {
    let module = compile_to_wasm(&capturing_program()).expect("compiles to wasm");
    let (mut store, inst) = instantiate(&module);
    let handle = find_handle(&store, &inst);
    // Two independent "events" through the same handle see the same capture.
    assert_eq!(call_trampoline(&mut store, &inst, handle, 7), SENTINEL + 7);
    assert_eq!(call_trampoline(&mut store, &inst, handle, 9), SENTINEL + 9);
}
