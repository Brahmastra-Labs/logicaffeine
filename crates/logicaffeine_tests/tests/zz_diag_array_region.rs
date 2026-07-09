//! TEMP DIAGNOSTIC: does an int-array region tier up?
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

fn run(name: &str, src: &str, args: &[&str]) {
    let src = src.to_string();
    let name = name.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    on_big_stack(move || {
        logicaffeine_jit::REGION_RUNS.store(0, std::sync::atomic::Ordering::Relaxed);
        logicaffeine_jit::REGION_DEOPTS.store(0, std::sync::atomic::Ordering::Relaxed);
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &args, Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &args);
        let (rc_att, rc_ok) = tier.region_counts();
        let runs = logicaffeine_jit::REGION_RUNS.swap(0, std::sync::atomic::Ordering::Relaxed);
        let deopts = logicaffeine_jit::REGION_DEOPTS.swap(0, std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "DIAG {name}: region(att={rc_att}, ok={rc_ok})  RUNS={runs} DEOPTS={deopts}  out={:?} err={:?}",
            vm.output, vm.error
        );
        assert_eq!(vm.output.trim(), tw.output.trim(), "{name}: VM != TW");
    });
}

// A PURE push loop, no Mod/Div/Index — does a push-only region tier?
const PURE_PUSH: &str = "## Main\nLet mutable arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 500000:\n    Push (i + 3) to arr.\n    Set i to i + 1.\nShow item 1 of arr.\n";

// Push + Mod (the array_fill / array_reverse fill-loop shape).
const PUSH_MOD: &str = "## Main\nLet mutable arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 500000:\n    Push ((i * 7 + 3) % 1000000) to arr.\n    Set i to i + 1.\nShow item 1 of arr.\n";

// SetIndex loop (the array_reverse swap shape) reading/writing a pre-filled array.
const SETINDEX: &str = "## Main\nLet mutable arr be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 500000:\n    Push i to arr.\n    Set i to i + 1.\nSet i to 1.\nWhile i is at most 250000:\n    Let tmp be item i of arr.\n    Set item i of arr to item (500001 - i) of arr.\n    Set item (500001 - i) of arr to tmp.\n    Set i to i + 1.\nShow item 1 of arr.\n";

#[test]
fn diag_pure_push() {
    run("pure_push", PURE_PUSH, &["prog"]);
}
#[test]
fn diag_push_mod() {
    run("push_mod", PUSH_MOD, &["prog"]);
}
#[test]
fn diag_setindex() {
    run("setindex", SETINDEX, &["prog"]);
}
