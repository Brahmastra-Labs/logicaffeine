//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE VM'S COPY-ON-WRITE COUNTS ONLY HOLDERS — reading a collection and then writing it is linear.
//!
//! Under value semantics the VM copies a List, Map or Set before mutating it in place whenever
//! another holder shares its allocation (`Rc` strong count > 1). The compiler reads a collection
//! into a SCRATCH register (`item i of xs` is `GlobalGet r = xs; Index v = r[i]`) and reuses that
//! register statement by statement without clearing it, so the reference outlives the read. When
//! the next mutation of the same collection counted that dead register as a holder, every
//! read-then-write copied the whole collection: a loop touching each element once was quadratic.
//! Every lowered page mutates its `__`-registries this way, once per turn per site.
//!
//! The VM releases a dead scratch register's reference on the path that was about to copy
//! (`Vm::release_dead_aliases`, reading the frame's register liveness), so a real alias still
//! copies and a dead one never does. Pinned here as a growth law, not a time: each shape runs `n`
//! read-then-write steps at a small and a large `n`, and the VM's time may grow at most `HEADROOM`
//! times its `clean` ratio per unit growth of `n` — the ratio measured on this box (i9-14900K,
//! debug build). A quadratic outgrows it by the size ratio itself.
//!
//! Only the VM's `run` is timed: each program is compiled once. The small and the large sample last
//! about as long (the small one repeated as many times as the large is larger), fastest of
//! `ROUNDS`, so load lands on both alike. A red is re-run alone before it is believed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test vm_cow_scaling_timing_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_program;
use logicaffeine_compile::vm::{CompiledProgram, Vm};
use std::time::{Duration, Instant};

/// How far past its clean ratio a shape's time may grow.
const HEADROOM: f64 = 2.0;
const SMALL: usize = 1_000;
const LARGE: usize = 16_000;
const ROUNDS: usize = 3;

/// One shape: a program of `n` read-then-write steps, its expected output, and its clean ratio.
struct Shape {
    what: &'static str,
    source: fn(usize) -> String,
    shows: fn(usize) -> String,
    clean: f64,
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            what: "a global Seq: read an item, then write it",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to __VD_T.\n        Set i to i + 1.\n\n\
                     ## To poke(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Let v be item i of __VD_T.\n        \
                     Set item i of __VD_T to v + 1.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable __VD_T be [0].\nfill({n}).\npoke({n}).\nShow length of __VD_T.\nShow item {n} of __VD_T.\n"
                )
            },
            shows: |n| format!("{}\n{n}", n + 1),
            clean: 1.0,
        },
        Shape {
            what: "a global Seq: read and write in one statement",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to __VD_T.\n        Set i to i + 1.\n\n\
                     ## To bump(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        \
                     Set item i of __VD_T to (item i of __VD_T) + 1.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable __VD_T be [0].\nfill({n}).\nbump({n}).\nShow item 2 of __VD_T.\n"
                )
            },
            shows: |_| "2".to_string(),
            clean: 1.0,
        },
        Shape {
            what: "a global Seq: two different mutations a step",
            source: |n| {
                format!(
                    "## To churn(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to __VD_T.\n        \
                     Set item i of __VD_T to (length of __VD_T).\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable __VD_T be [0].\nchurn({n}).\nShow length of __VD_T.\n"
                )
            },
            shows: |n| format!("{}", n + 1),
            clean: 1.0,
        },
        Shape {
            what: "a global Map: test for a key, read it, write it",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Set item i of __VD_M to i.\n        Set i to i + 1.\n\n\
                     ## To poke(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        If __VD_M contains i:\n            \
                     Let v be item i of __VD_M.\n            Set item i of __VD_M to v + 1.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable __VD_M be a new Map of Int to Int.\nfill({n}).\npoke({n}).\nShow length of __VD_M.\nShow item {n} of __VD_M.\n"
                )
            },
            shows: |n| format!("{n}\n{}", n + 1),
            clean: 1.0,
        },
        Shape {
            what: "an author's global Seq pushed from a function",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to items.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable items be [0].\nfill({n}).\nShow length of items.\n"
                )
            },
            shows: |n| format!("{}", n + 1),
            clean: 1.0,
        },
        Shape {
            what: "an author's global Seq read then written from a function",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to items.\n        Set i to i + 1.\n\n\
                     ## To poke(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Let v be item i of items.\n        \
                     Set item i of items to v + 1.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable items be [0].\nfill({n}).\npoke({n}).\nShow item {n} of items.\n"
                )
            },
            shows: |n| format!("{n}"),
            clean: 1.0,
        },
        Shape {
            what: "an author's global Map set from a function",
            source: |n| {
                format!(
                    "## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Set item i of counts to i.\n        Set i to i + 1.\n\n\
                     ## Main\nLet mutable counts be a new Map of Int to Int.\nfill({n}).\nShow length of counts.\n"
                )
            },
            shows: |n| format!("{n}"),
            clean: 1.0,
        },
        Shape {
            what: "an author's global record's list field pushed from a function",
            source: |n| {
                format!(
                    "## A Box has:\n    a tape, which is Seq of Int.\n\n\
                     ## To fill(n: Int):\n    Let mutable i be 1.\n    While i <= n:\n        Push i to (item 1 of boxes)'s tape.\n        \
                     Set i to i + 1.\n\n\
                     ## Main\nLet mutable boxes be a new Seq of Box.\nPush a new Box with tape (a new Seq of Int) to boxes.\nfill({n}).\n\
                     Show length of (item 1 of boxes)'s tape.\n"
                )
            },
            shows: |n| format!("{n}"),
            clean: 1.0,
        },
        Shape {
            what: "a record's list field: read an item, then write it",
            source: |n| {
                format!(
                    "## A Box has:\n    a tape, which is Seq of Int.\n\n\
                     ## To poke(n: Int) -> Int:\n    Let b be a new Box with tape (a new Seq of Int).\n    Let mutable i be 1.\n    \
                     While i <= n:\n        Push i to b's tape.\n        Set i to i + 1.\n    Set i to 1.\n    While i <= n:\n        \
                     Let v be item i of b's tape.\n        Set item i of b's tape to v + 1.\n        Set i to i + 1.\n    \
                     Return item n of b's tape.\n\n\
                     ## Main\nShow poke({n}).\n"
                )
            },
            shows: |n| format!("{}", n + 1),
            clean: 1.0,
        },
    ]
}

fn compiled(src: &str) -> CompiledProgram {
    compile_to_program(src).unwrap_or_else(|e| panic!("the shape compiles: {e:?}\n{src}"))
}

/// Run `program` once on the VM, answering what it showed.
fn run(program: &CompiledProgram) -> String {
    let mut vm = Vm::new(program);
    vm.run().unwrap_or_else(|e| panic!("the shape runs: {e}"));
    vm.output().trim_end().to_string()
}

#[test]
fn a_collection_read_then_written_costs_linear_time_on_the_vm() {
    let mut table = format!("\n  shape                                              n={SMALL:<7} n={LARGE:<7} growth   ratio\n");
    let mut red = Vec::new();
    for shape in shapes() {
        let (small, large) = (
            compiled(&(shape.source)(SMALL)),
            compiled(&(shape.source)(LARGE)),
        );
        assert_eq!(
            run(&small),
            (shape.shows)(SMALL),
            "{}: the small program's answer",
            shape.what
        );
        assert_eq!(
            run(&large),
            (shape.shows)(LARGE),
            "{}: the large program's answer",
            shape.what
        );
        let reps = LARGE / SMALL;
        let (mut s, mut l) = (Duration::MAX, Duration::MAX);
        for _ in 0..ROUNDS {
            let t = Instant::now();
            for _ in 0..reps {
                std::hint::black_box(run(&small));
            }
            s = s.min(t.elapsed() / reps as u32);
            let t = Instant::now();
            std::hint::black_box(run(&large));
            l = l.min(t.elapsed());
        }
        let growth = l.as_secs_f64() / s.as_secs_f64().max(1e-9);
        let ratio = growth / (LARGE as f64 / SMALL as f64);
        table.push_str(&format!(
            "  {:<50} {s:>9.2?} {l:>9.2?} {growth:>6.1}x  {ratio:>5.2} (clean {:.2})\n",
            shape.what, shape.clean
        ));
        if ratio > HEADROOM * shape.clean {
            red.push(shape.what);
        }
    }
    println!("{table}");
    assert!(
        red.is_empty(),
        "{red:?}: the VM's time grew faster than the work — a mutation copies the whole collection again (re-run alone \
         before believing it){table}"
    );
}
