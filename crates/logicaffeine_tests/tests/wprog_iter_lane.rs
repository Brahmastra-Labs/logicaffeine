//! wprog_iter_lane — NATIVE `Repeat for …` loops on the whole-program engine. The engine already runs `While`
//! loops; this lane lowers the two ITERATOR loop forms — range (`Repeat for t from lo to hi`, via `NewRange` +
//! `IterPrepare` + `IterNext` + `IterPop`) and for-each (`Repeat for x in xs`) — WITHOUT a While-rewrite: the
//! iterator is a heap object (`[cursor, limit]` for a range, `[index, coll_ptr]` for a Seq), `IterNext` is a
//! conditional Branch terminator (has-next → body; exhausted → exit) that reads/advances it. So the engine now
//! handles EVERY LOGOS loop form natively — the completeness capstone. Each form is proven ≡ the VM.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_iter_lane`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric,
};
use logicaffeine_compile::compile::vm_outcome;

fn vm_f(src: &str, n: i64) -> i64 {
    let out = vm_outcome(&format!("{src}## Main\n    Show f({n}).\n"));
    assert!(out.error.is_none(), "VM error at n={n}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Assert the whole-program engine (call-stack fabric) runs `f` ≡ the VM for every input.
fn assert_iter(name: &str, src: &str, args: &[i64]) {
    let cfg = program_calls_fabric(src, "f");
    for &n in args {
        let vm = vm_f(src, n);
        let fab = eval_wprog_config(&cfg, &[n]);
        assert_eq!(
            fab, vm,
            "{name}: native-iter fabric {fab} != VM {vm} at n={n}"
        );
    }
    println!("NATIVE ITER `{name}`: runs on the whole-program engine ≡ VM (NewRange/IterPrepare/IterNext/IterPop)");
}

#[test]
fn native_range_loop_runs_on_the_engine() {
    // `Repeat for t from 1 to n` — a range iterator on the heap, stepped by IterNext (a helper forces the
    // call-stack fabric, which owns the iterator lowering).
    assert_iter(
        "range-sum",
        "## To g (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for t from 1 to n:\n        Set s to s + t.\n    Return s.\n## To f (n: Int) -> Int:\n    Return g(n).\n",
        &[0, 1, 5, 10, 100, 1000],
    );
}

#[test]
fn native_nested_range_loops_run_on_the_engine() {
    // Nested range iterators — two heap iterators live at once (the lowering-time iterator stack).
    assert_iter(
        "range-nested",
        "## To g (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to n:\n        Repeat for j from 1 to n:\n            Set s to s + i * j.\n    Return s.\n## To f (n: Int) -> Int:\n    Return g(n).\n",
        &[0, 1, 3, 5, 20],
    );
}

#[test]
fn native_for_each_over_a_seq_runs_on_the_engine() {
    // `Repeat for x in xs` — a Seq iterator `[index, coll_ptr]`; IterNext reads `heap[coll + index + 1]`.
    assert_iter(
        "foreach-sum",
        "## To g (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        Push i * i to xs.\n        Set i to i + 1.\n    Let mutable s be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Return s.\n## To f (n: Int) -> Int:\n    Return g(n).\n",
        &[0, 1, 4, 10, 50],
    );
}
