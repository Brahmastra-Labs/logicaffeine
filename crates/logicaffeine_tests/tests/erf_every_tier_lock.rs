//! ★ LOCK — the error function is the library's `erf` on every backend, and the IR's `Erf` is it, bit for bit. ★
//!
//! `erf` is a function of the standard library (`assets/std/math.lg`), Go's `math.Erf` operation for operation: a
//! rational function of `x²` below 0.84375, one of `|x| − 1` below 1.25, `1 − e^(−z² − 0.5625)·e^((z − |x|)(z + |x|) +
//! R/S)/|x|` below 6 for a rational `R/S` of `1/x²` and `z` the leading 21 bits of `|x|`, and one past 6. `Let Y be erf
//! of X.` maps it over a tensor — exact GELU's `0.5·x·(1 + erf(x/√2))` is written from it — and the walk lifts that
//! sentence as one `Erf` op, which the tensor graph's evaluator and the spatial IR's `SfpuKind::Erf` compute as the
//! library does (`tensor_graph::eval::lib_erf`). A program that writes its own `erf` keeps it: the sentence maps the
//! program's function, and the walk lifts no `Erf` from it.
//!
//! Held here: the library's `erf` equals Go's `math.Erf` bit for bit on a grid of every branch; the tree-walker prints
//! what each program says and every backend prints the tree-walker's bytes; the graph's evaluator computes the
//! library's values bit for bit; and the walk lifts the sentence only where the `erf` it calls is the library's.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test erf_every_tier_lock`

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "codegen")]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use std::collections::BTreeMap;
use tier_parity::{Backend, Program};

/// Go's `math.Erf` at a grid holding points of every branch and its edges, as bits.
const GO: &[(f64, u64)] = &[
    (0.0, 0x0000000000000000),
    (1e-300, 0x01a82e6d98711d3a),
    (-1e-300, 0x81a82e6d98711d3a),
    (3e-09, 0x3e2d13feffa9194b),
    (-3e-09, 0xbe2d13feffa9194b),
    (1e-05, 0x3ee7a9f084b5e44c),
    (-1e-05, 0xbee7a9f084b5e44c),
    (0.1, 0x3fbcca5ea24fb334),
    (-0.1, 0xbfbcca5ea24fb334),
    (0.25, 0x3fd1af54e232d609),
    (-0.25, 0xbfd1af54e232d609),
    (0.5, 0x3fe0a7ef5c18edd2),
    (-0.5, 0xbfe0a7ef5c18edd2),
    (0.84, 0x3fe87c0c914f1a59),
    (-0.84, 0xbfe87c0c914f1a59),
    (0.84375, 0x3fe88d1cd474a2e0),
    (-0.84375, 0xbfe88d1cd474a2e0),
    (0.9, 0x3fe98045a6c8a2e6),
    (-0.9, 0xbfe98045a6c8a2e6),
    (1.0, 0x3feaf767a741088b),
    (-1.0, 0xbfeaf767a741088b),
    (1.1, 0x3fec2aa3d27302c0),
    (-1.1, 0xbfec2aa3d27302c0),
    (1.2, 0x3fed214ac7b2d067),
    (-1.2, 0xbfed214ac7b2d067),
    (1.249, 0x3fed8675356a271e),
    (-1.249, 0xbfed8675356a271e),
    (1.25, 0x3fed8865d98abe00),
    (-1.25, 0xbfed8865d98abe00),
    (1.5, 0x3feeea5557137ae0),
    (-1.5, 0xbfeeea5557137ae0),
    (1.75, 0x3fef92d077f8d56d),
    (-1.75, 0xbfef92d077f8d56d),
    (2.0, 0x3fefd9ae142795e3),
    (-2.0, 0xbfefd9ae142795e3),
    (2.25, 0x3feff404760319b4),
    (-2.25, 0xbfeff404760319b4),
    (2.5, 0x3feffcaa8f4c9bea),
    (-2.5, 0xbfeffcaa8f4c9bea),
    (2.75, 0x3fefff2cfb0453d9),
    (-2.75, 0xbfefff2cfb0453d9),
    (2.857142857142857, 0x3fefff90322bb733),
    (-2.857142857142857, 0xbfefff90322bb733),
    (2.86, 0x3fefff921b559db7),
    (-2.86, 0xbfefff921b559db7),
    (3.0, 0x3fefffd1ac4135f9),
    (-3.0, 0xbfefffd1ac4135f9),
    (3.5, 0x3feffffe710d565e),
    (-3.5, 0xbfeffffe710d565e),
    (4.0, 0x3feffffff7b91176),
    (-4.0, 0xbfeffffff7b91176),
    (4.5, 0x3fefffffffe4fa30),
    (-4.5, 0xbfefffffffe4fa30),
    (5.0, 0x3fefffffffffc9e8),
    (-5.0, 0xbfefffffffffc9e8),
    (5.5, 0x3fefffffffffffbe),
    (-5.5, 0xbfefffffffffffbe),
    (5.9, 0x3fefffffffffffff),
    (-5.9, 0xbfefffffffffffff),
    (5.99, 0x3ff0000000000000),
    (-5.99, 0xbff0000000000000),
    (6.0, 0x3ff0000000000000),
    (-6.0, 0xbff0000000000000),
    (7.0, 0x3ff0000000000000),
    (-7.0, 0xbff0000000000000),
];

/// `x` as a LOGOS literal: a decimal point always, a negative as a difference from zero.
fn literal(x: f64) -> String {
    let s = format!("{:?}", x.abs());
    let s = if s.contains('e') && !s.contains('.') { s.replacen('e', ".0e", 1) } else { s };
    if x.is_sign_negative() {
        format!("(0.0 - {s})")
    } else {
        s
    }
}

/// Each program, and what it prints.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "the_library_erf_and_erfc",
        "## Main\nShow erf(0.5).\nShow erf(0.0 - 1.2).\nShow erf(2.5).\nShow erf(4.5).\nShow erf(6.5).\nShow erf(0.0 - 0.0).\nShow erfc(3.0).\n",
        "0.5204998778130465\n-0.9103139782296353\n0.999593047982555\n0.9999999998033839\n1\n0\n0.000022090496998639075\n",
    ),
    (
        "a_tensor_mapped_through_erf",
        "## Main\nLet X be a 1 by 6 tensor of Real.\nSet item 1 of X to -1.5.\nSet item 2 of X to 0.25.\nSet item 3 of X to 1.0.\nSet item 4 of X to 2.25.\nSet item 5 of X to 3.5.\nSet item 6 of X to -0.0625.\nLet Y be erf of X.\nShow item 1 of Y.\nShow item 2 of Y.\nShow item 3 of Y.\nShow item 4 of Y.\nShow item 5 of Y.\nShow item 6 of Y.\n",
        "-0.9661051464753108\n0.27632639016823696\n0.8427007929497149\n0.9985372834133188\n0.9999992569016276\n-0.07043197772238707\n",
    ),
    (
        "exact_gelu_written_from_it",
        "## Main\nLet X be a 1 by 4 tensor of Real.\nSet item 1 of X to -1.0.\nSet item 2 of X to 0.5.\nSet item 3 of X to 1.5.\nSet item 4 of X to 3.0.\nLet S be X scaled by 0.7071067811865476.\nLet E be erf of S.\nLet P be X hadamard E.\nLet Q be X plus P.\nLet G be Q scaled by 0.5.\nShow item 1 of G.\nShow item 2 of G.\nShow item 3 of G.\nShow item 4 of G.\n",
        "-0.15865525393145702\n0.34573123063700656\n1.399789198096713\n2.99595030590511\n",
    ),
    (
        "a_program_that_writes_its_own_erf",
        "## To erf (x: Real) -> Real:\n    Return x * 2.0.\n\n## Main\nLet X be a 1 by 2 tensor of Real.\nSet item 1 of X to 0.5.\nSet item 2 of X to 1.5.\nLet Y be erf of X.\nShow item 1 of Y.\nShow item 2 of Y.\n",
        "1\n3\n",
    ),
];

/// Every backend that runs a program: the engines, the generated Rust, both C lowerings, both WebAssembly modules
/// and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn program((id, source, _): &(&str, &str, &str)) -> Program {
    Program::new(format!("erf/{id}"), *source)
}

#[test]
fn the_library_erf_is_gos_bit_for_bit() {
    let mut src = String::from("## Main\n");
    for (x, _) in GO {
        src += &format!("Show erf({}).\n", literal(*x));
    }
    let run = tw_outcome(&src);
    assert_eq!(run.error, None, "the tree-walker runs the grid");
    let printed: Vec<&str> = run.output.lines().collect();
    assert_eq!(printed.len(), GO.len(), "a value a point");
    let apart: Vec<String> = GO
        .iter()
        .zip(&printed)
        .filter_map(|((x, go), line)| {
            let v: f64 = line.trim().parse().unwrap_or_else(|_| panic!("erf({x}) printed `{line}`"));
            (v.to_bits() != *go).then(|| format!("erf({x:?}) = {v:?}, Go's {:?}", f64::from_bits(*go)))
        })
        .collect();
    assert!(apart.is_empty(), "{} of {} values are not Go's:\n{}", apart.len(), GO.len(), apart.join("\n"));
}

#[test]
fn the_graphs_erf_is_the_librarys_bit_for_bit() {
    let mut src = format!("## Main\nLet X be a 1 by {} tensor of Real.\n", GO.len());
    for (i, (x, _)) in GO.iter().enumerate() {
        // A literal, sign and all: a value the walk reads as data, where an expression would be a write it cannot lift.
        let v = format!("{x:?}");
        let v = if v.contains('e') && !v.contains('.') { v.replacen('e', ".0e", 1) } else { v };
        src += &format!("Set item {} of X to {v}.\n", i + 1);
    }
    src += "Let Y be erf of X.\n";
    let rp = recognize_stages(&src).expect("the walk reads the program");
    let kinds: Vec<&str> = rp.stages.iter().map(|s| s.op.kind()).collect();
    assert_eq!(kinds, ["Erf"], "the sentence is one Erf");
    let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).expect("the graph");
    let env = eval(&g, &BTreeMap::new()).expect("the evaluator runs it");
    let Some(Data::Real(y)) = env.get("Y") else { panic!("the evaluator computes Y as Reals") };
    let apart: Vec<String> = GO
        .iter()
        .zip(y)
        .filter(|((_, go), v)| v.to_bits() != *go)
        .map(|((x, go), v)| format!("erf({x:?}) = {v:?}, Go's {:?}", f64::from_bits(*go)))
        .collect();
    assert!(apart.is_empty(), "the graph's erf is not the library's at {} points:\n{}", apart.len(), apart.join("\n"));
}

#[test]
fn the_walk_lifts_the_sentence_only_where_erf_is_the_librarys() {
    let kinds = |src: &str| -> Vec<&'static str> { recognize_stages(src).expect("the walk").stages.iter().map(|s| s.op.kind()).collect() };
    let library = PROGRAMS[1].1;
    assert_eq!(kinds(library), ["Erf"], "the library's erf is one Erf");
    let theirs = PROGRAMS[3].1;
    assert!(!kinds(theirs).contains(&"Erf"), "a program's own erf is not the library's: {:?}", kinds(theirs));
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> =
        PROGRAMS.iter().map(program).filter(|p| tier_parity::rejection(&p.source).is_some()).map(|p| p.id).collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn the_tree_walker_prints_what_each_program_says() {
    for (id, source, printed) in PROGRAMS {
        let run = tw_outcome(source);
        assert_eq!(run.error, None, "{id}: the tree-walker refused:\n{source}");
        assert_eq!(run.output.trim_end(), printed.trim_end(), "{id}:\n{source}");
    }
}

#[test]
fn every_backend_prints_the_tree_walkers_bytes() {
    let programs: Vec<Program> = PROGRAMS.iter().map(program).collect();
    let rows = tier_parity::sweep(&programs, BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the tree-walker:\n{}", bad.len(), bad.join("\n"));
}
