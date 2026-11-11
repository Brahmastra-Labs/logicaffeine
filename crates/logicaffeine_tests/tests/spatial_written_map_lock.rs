//! spatial_written_map_lock — a counted loop the program writes, whose every turn computes one element of a tensor it
//! declared, is lifted as ONE map and lowered on the SN50 as one section of the general form, computing what the VM
//! prints: an op nobody wrote a sugar or a kernel for is never refused for being new (SCHED-10's edge case,
//! `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! A turn is read by running it symbolically — `Let`s bind terms, `Set item` of the written tensor at the turn's own
//! place replaces the element's term, a branch joins its arms under its condition — in the VM's own types: an Int
//! quotient truncates, `//` floors, `%` is the truncated remainder, a Real meets an Int by widening it. A turn that reads
//! the written tensor where the turn one step before along the outer loop wrote it, on an arm the first turn never
//! takes, is a recurrence: linear in its carry it lowers as a chunked scan, otherwise turn by turn, and a branch on the
//! outer position resets it in segments. Whatever else a loop does — writes a name outside it, reads another turn's
//! place, writes two tensors — is no map, and the walk lists it.
//!
//!   cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_written_map_lock

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
#[allow(dead_code)]
mod tensor_probes;
#[path = "common/written_programs.rs"]
#[allow(dead_code)]
mod written_programs;

use std::collections::BTreeMap;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use written_programs::{filled, ints, lifted, on_the_sn50, seeded, shows, shows_exact, special_compute, walk};

/// The kinds the walk lifts from `src`.
fn kinds(src: &str) -> Vec<&'static str> {
    lifted(src).iter().map(|l| l.0).collect()
}

#[test]
fn integer_arithmetic_is_the_vms() {
    // Every place of a row of 24 from −12 to 11, divided, floored, taken modulo and powered as the VM's Ints are: a
    // quotient truncated toward zero, a floored one, a remainder with the dividend's sign, an exact power.
    let values: Vec<i64> = (-12..12).collect();
    for (what, expr) in [
        ("a truncated quotient", "(item i of A) / 5"),
        ("a floored quotient", "(item i of A) // 5"),
        ("a remainder", "(item i of A) % 5"),
        ("a quotient by a negative", "(item i of A) / (0 - 3)"),
        ("a power", "(item i of A) ** 3"),
        ("an absolute value and a bound", "max(abs(item i of A), 4) - min(item i of A, 2)"),
    ] {
        let src = format!(
            "## Main\n{}Let Y be a 1 by 24 tensor of Int.\nRepeat for i from 1 to 24:\n    Set item i of Y to {expr}.\n{}",
            ints("A", &values),
            shows_exact("Y", 24)
        );
        assert_eq!(kinds(&src), vec!["Map"], "{what}");
        on_the_sn50(&src, what, true);
    }
}

#[test]
fn branches_are_decided_by_values_as_the_vm_decides_them() {
    // Every comparison, on values that meet it at equality and on either side.
    let values: Vec<i64> = (0..32).map(|i| (i % 7) - 3).collect();
    for (cmp, what) in [
        ("is greater than", ">"),
        ("is at least", ">="),
        ("is less than", "<"),
        ("is at most", "<="),
        ("equals", "="),
        ("is not", "!="),
    ] {
        let src = format!(
            "## Main\n{}Let Y be a 1 by 32 tensor of Int.\nRepeat for i from 1 to 32:\n    Let a be item i of A.\n\
             \x20   If a {cmp} 1:\n        Set item i of Y to a * 10.\n    Otherwise:\n        Set item i of Y to 7 - a.\n{}",
            ints("A", &values),
            shows_exact("Y", 32)
        );
        assert_eq!(kinds(&src), vec!["Map"], "{what}");
        on_the_sn50(&src, &format!("a branch on {what}"), true);
    }
    // Conditions joined, negated, and nested in a later branch; a name rebound on one arm.
    let src = format!(
        "## Main\n{}Let Y be a 1 by 32 tensor of Int.\nRepeat for i from 1 to 32:\n    Let a be item i of A.\n    Let mutable b be a + i.\n\
         \x20   If a is greater than 0 and not (i is less than 8):\n        Set b to b * 2.\n\
         \x20   If b is at least 10 or a equals 0 - 3:\n        Set item i of Y to b.\n    Otherwise:\n        Set item i of Y to 0 - b.\n{}",
        ints("A", &values),
        shows_exact("Y", 32)
    );
    on_the_sn50(&src, "joined, negated and nested branches", true);
}

#[test]
fn a_real_map_reads_constants_scalars_functions_and_positions() {
    // A constant bound before the loop, a fixed entry of a table, a function the program defines calling another, the
    // turn's position, the library's `ln` and `tanh`, a power of a constant base, and an arm that keeps the tensor's
    // own value.
    let src = format!(
        "## To sharpened (x: Real) and (k: Real) -> Real:\n    Return tanh(x * k).\n\n\
         ## To blend (x: Real) -> Real:\n    Let s be sharpened(x, 1.5).\n    Return s * s + ln(2.0 + x).\n\n\
         ## Main\n{}{}Let scale be 0.25.\nLet Y be a 1 by 48 tensor of Real.\n\
         Repeat for i from 1 to 48:\n    Let x be item i of X.\n\
         \x20   If i is at most 40:\n        Set item i of Y to blend(x) * scale + item 3 of W + 2.0 ** x - i / 64.0.\n{}",
        filled("X", 1, 48, &|i| seeded("X", i)),
        filled("W", 1, 4, &|i| seeded("W", i)),
        shows("Y", 48)
    );
    assert_eq!(kinds(&src), vec!["Map"]);
    let sp = on_the_sn50(&src, "a map of constants, scalars, functions and positions", false);
    assert!(special_compute(&sp).is_empty(), "the general form alone: {:?}", special_compute(&sp));
}

#[test]
fn a_function_the_program_defines_is_its_own_never_the_librarys() {
    // The library's `exp` is `e^x`, called by its meaning; a program that writes its own `exp` calls that one, which a
    // turn inlines: here it doubles.
    let program = |functions: &str| {
        format!(
            "{functions}## Main\n{}Let Y be a 1 by 32 tensor of Real.\nRepeat for i from 1 to 32:\n    Set item i of Y to exp(item i of X) + 1.0.\n{}",
            filled("X", 1, 32, &|i| seeded("X", i)),
            shows("Y", 32)
        )
    };
    let library = program("");
    assert_eq!(kinds(&library), vec!["Map"], "the library's exp");
    on_the_sn50(&library, "the library's exp", false);
    let own = program("## To exp (x: Real) -> Real:\n    Return x * 2.0.\n\n");
    let vm = vm_outcome(&own);
    assert!(vm.error.is_none(), "the VM calls the program's exp: {:?}", vm.error);
    let first: i64 = vm.output.split_whitespace().next().expect("a value").parse().expect("an integer");
    assert_eq!(first, ((seeded("X", 0) * 2.0 + 1.0) * 1e6).round() as i64, "the VM reads the program's own exp");
    assert_eq!(kinds(&own), vec!["Map"], "the program's own exp");
    on_the_sn50(&own, "the program's own exp", false);
    // An `exp` of the program's whose body is no arithmetic a turn reads — a loop — leaves the map listed: it is never
    // taken for the library's.
    let looped = program("## To exp (x: Real) -> Real:\n    Let mutable r be 1.0.\n    Repeat for k from 1 to 3:\n        Set r to r * x.\n    Return r.\n\n");
    let rp = walk(&looped);
    assert!(rp.stages.is_empty(), "nothing lifts: {:?}", rp.stages.iter().map(|s| s.op.kind()).collect::<Vec<_>>());
    assert!(rp.unrecognized.iter().any(|(_, b)| b == "Repeat"), "the loop is listed: {:?}", rp.unrecognized);
}

#[test]
fn a_nest_of_two_writes_rows_by_their_positions() {
    // A causal ramp over a 12 by 12 block: each row's columns up to its own scaled by their distance, the rest zero.
    let src = format!(
        "## Main\n{}Let M be a 12 by 12 tensor of Real.\nRepeat for r from 1 to 12:\n    Repeat for c from 1 to 12:\n\
         \x20       If c is at most r:\n            Set item ((r - 1) * 12 + c) of M to (item ((r - 1) * 12 + c) of S) * (r - c + 1).\n\
         \x20       Otherwise:\n            Set item ((r - 1) * 12 + c) of M to 0.0.\n{}",
        filled("S", 12, 12, &|i| seeded("S", i)),
        shows("M", 144)
    );
    assert_eq!(kinds(&src), vec!["Map"]);
    on_the_sn50(&src, "a causal ramp", false);
}

#[test]
fn a_recurrence_runs_turn_by_turn_or_chunked() {
    let program = |step: &str| {
        format!(
            "## Main\n{}Let H be a 12 by 4 tensor of Real.\nRepeat for t from 1 to 12:\n    Repeat for d from 1 to 4:\n\
             \x20       Let x be item ((t - 1) * 4 + d) of X.\n\
             \x20       If t is greater than 1:\n            Let h be item ((t - 2) * 4 + d) of H.\n            Set item ((t - 1) * 4 + d) of H to {step}.\n\
             \x20       Otherwise:\n            Set item ((t - 1) * 4 + d) of H to x.\n{}",
            filled("X", 12, 4, &|i| seeded("X", i) / 2.0),
            shows("H", 48)
        )
    };
    // A running sum and a decayed one are linear in the carry: chunked. A carry squared is not: turn by turn.
    for (step, chunked, what) in [
        ("h + x", true, "a running sum"),
        ("0.5 * h - x / 4.0", true, "a decayed sum"),
        ("h * h * 0.5 + x", false, "a carry squared"),
        ("max(h, x)", false, "a running max"),
    ] {
        let src = program(step);
        assert_eq!(kinds(&src), vec!["Scan"], "{what}");
        let sp = on_the_sn50(&src, what, false);
        let names: Vec<&str> = sp.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.iter().any(|n| n.contains("scan_carry")), chunked, "{what}: chunked {chunked}: {names:?}");
        assert_eq!(names.iter().any(|n| n.contains("scan_turn")), !chunked, "{what}: turn by turn {}: {names:?}", !chunked);
    }
}

#[test]
fn a_segmented_recurrence_resets_at_each_segment() {
    // Segments of 4 turns, each starting from its own input: the carry crosses no segment's first turn. The first turn
    // starts a segment, so it never reads a carry.
    let program = |step: &str| {
        format!(
            "## Main\n{}Let H be a 12 by 4 tensor of Real.\nRepeat for t from 1 to 12:\n    Repeat for d from 1 to 4:\n\
             \x20       Let x be item ((t - 1) * 4 + d) of X.\n\
             \x20       If (t - 1) % 4 equals 0:\n            Set item ((t - 1) * 4 + d) of H to x.\n\
             \x20       Otherwise:\n            Let h be item ((t - 2) * 4 + d) of H.\n            Set item ((t - 1) * 4 + d) of H to {step}.\n{}",
            filled("X", 12, 4, &|i| seeded("X", i)),
            shows("H", 48)
        )
    };
    for (step, chunked, what) in [("0.5 * h + x", true, "a decayed sum in segments"), ("max(h, x) - 0.25", false, "a running max in segments")] {
        let src = program(step);
        assert_eq!(kinds(&src), vec!["Scan"], "{what}");
        let sp = on_the_sn50(&src, what, false);
        let names: Vec<&str> = sp.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.iter().any(|n| n.contains("scan_carry")), chunked, "{what}: chunked {chunked}: {names:?}");
    }
    // A segment's reset that the first turn does not take leaves the first turn reading a carry: no recurrence.
    let src = program("0.5 * h + x").replace("(t - 1) % 4 equals 0", "(t - 1) % 4 equals 1");
    assert!(walk(&src).stages.is_empty(), "a first turn that reads a carry lifts nothing");
}

#[test]
fn what_is_not_a_map_is_listed() {
    let decl = format!("## Main\n{}Let Y be a 1 by 8 tensor of Real.\nLet Z be a 1 by 8 tensor of Real.\n", filled("X", 1, 8, &|i| seeded("X", i)));
    for (what, body) in [
        ("a counter outside the loop", "Let mutable n be 0.\nRepeat for i from 1 to 8:\n    Set n to n + 1.\n    Set item i of Y to item i of X.\n"),
        ("a read of a place another turn writes", "Repeat for i from 1 to 8:\n    Set item i of Y to item 1 of Y + item i of X.\n"),
        ("a read of the next place", "Repeat for i from 1 to 7:\n    Set item i of Y to item (i + 1) of X.\n"),
        ("two tensors written", "Repeat for i from 1 to 8:\n    Set item i of Y to item i of X.\n    Set item i of Z to item i of X.\n"),
        ("a turn that shows", "Repeat for i from 1 to 8:\n    Show item i of X.\n"),
        ("half the tensor written", "Repeat for i from 1 to 4:\n    Set item i of Y to item i of X.\n"),
        ("a carry read on the first turn", "Repeat for i from 1 to 8:\n    Set item i of Y to item (i - 1) of Y + item i of X.\n"),
    ] {
        let src = format!("{decl}{body}Show item 1 of Y.\n");
        let rp = walk(&src);
        assert!(rp.stages.is_empty(), "{what}: nothing lifts: {:?}", rp.stages.iter().map(|s| s.op.kind()).collect::<Vec<_>>());
        assert!(rp.unrecognized.iter().any(|(_, b)| b == "Repeat"), "{what}: the loop is listed: {:?}", rp.unrecognized);
    }
    // A tensor an op has already read is not written again by a map.
    let src = format!("{decl}Let D be Y plus X.\nRepeat for i from 1 to 8:\n    Set item i of Y to item i of X.\nShow item 1 of D.\n");
    assert_eq!(kinds(&src), vec!["Elementwise"], "a map never rewrites a tensor an op has read");
}

#[test]
fn the_evaluator_computes_a_written_map_as_the_vm() {
    // The graph's meaning is the VM's to the last bit, the library's `exp`, `ln` and `tanh` included, and a recurrence's
    // turns in order.
    let maps = [
        format!(
            "## Main\n{}Let Y be a 1 by 32 tensor of Real.\nRepeat for i from 1 to 32:\n    Let x be item i of X.\n\
             \x20   If x is greater than 0.0:\n        Set item i of Y to exp(x) - ln(1.0 + x).\n    Otherwise:\n        Set item i of Y to tanh(x) * sqrt(0.0 - x).\n",
            filled("X", 1, 32, &|i| seeded("X", i) * 2.0)
        ),
        format!(
            "## Main\n{}Let H be a 8 by 4 tensor of Real.\nRepeat for t from 1 to 8:\n    Repeat for d from 1 to 4:\n\
             \x20       Let x be item ((t - 1) * 4 + d) of X.\n        If t equals 1:\n            Set item ((t - 1) * 4 + d) of H to x.\n\
             \x20       Otherwise:\n            Set item ((t - 1) * 4 + d) of H to 0.75 * item ((t - 2) * 4 + d) of H + x.\n",
            filled("X", 8, 4, &|i| seeded("X", i))
        ),
    ];
    for (k, src) in maps.iter().enumerate() {
        let out = if k == 0 { "Y" } else { "H" };
        let rp = walk(src);
        assert!(rp.unrecognized.is_empty(), "map {k}: {:?}", rp.unrecognized);
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("map {k}: {e}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|e| panic!("map {k}: the evaluator: {e}"));
        let Some(Data::Real(values)) = env.get(out) else { panic!("map {k}: no Real `{out}`") };
        let shown: String = (1..=values.len()).map(|i| format!("Show item {i} of {out}.\n")).collect();
        let vm = vm_outcome(&format!("{src}{shown}"));
        assert!(vm.error.is_none(), "map {k}: the VM: {:?}", vm.error);
        let printed: Vec<&str> = vm.output.lines().collect();
        assert_eq!(printed.len(), values.len(), "map {k}");
        for (i, (p, v)) in printed.iter().zip(values).enumerate() {
            assert_eq!(*p, format!("{v}"), "map {k}: element {i}");
        }
    }
}
