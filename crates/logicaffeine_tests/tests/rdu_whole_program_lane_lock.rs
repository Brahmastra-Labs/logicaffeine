//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU WHOLE-PROGRAM-LANE LOCK — any LOGOS program the fabric's decision datapath covers (if/else,
//! collections on the state plane, structs, helper calls inlined, straight-line `Main`s) lowered onto the
//! SN50 descriptor as a `Scalar` stage applied lane-wise over an input vector (`y[i] = f(x[i])`), verified,
//! certified, and handed to the RDU backend as the neutral placement.
//!
//!  A. `a_decision_program_lowers_to_scalar_stages_on_the_rdu_and_equals_the_vm` — the demo `fib.lg`
//!     (fast doubling, ALU-only), `primes.lg` (a Seq + an inlined helper + if-converted `and`), an
//!     if/else program, a collections program and a TWO-input program each lower to a Verified program
//!     with one `Scalar` stage; `eval_choreography` over 64 lanes equals the VM at every lane; the per-PE
//!     certificate rechecks where the body is ALU + select and is honestly `Uncertified` (by name) where a
//!     functional unit, the heap, or a computed predicate is involved; `RduSpatialBackend` emits the
//!     SDK-gated neutral artifact whose choreography certificate rechecks, on both chips.
//!  B. `a_main_program_lowers_as_the_show_tape_and_equals_the_vm` — a `## Main` of `Show`s lowers as the
//!     tape-select stage (lane i emits Show i) and equals the VM's stdout line for line; a `Show` of Text
//!     is refused by name.
//!  C. `recursion_and_loops_are_refused_by_name_never_lowered_wrong` — a recursive `fib` is refused naming
//!     recursion and the call stack (the decision front end would otherwise take the FIRST-RETURN PATH
//!     ONLY, silently), and the whole-program call-stack engine runs it ≡ VM meanwhile; a `While`, a
//!     counted loop (pointed at the loop lane) and an early return are refused by name.
//!  D. `an_op_on_the_frontier_is_refused_by_name_not_silently_dropped` — `tcpConnect` (`Op::Tcp`) is refused
//!     as a missing compute capability naming `Tcp`, through `fabric_tile::frontier_reason`.
//!  E. `the_lane_is_printed_and_is_a_function_of_the_program` — `lane_of` over the demo corpus and the
//!     README's table, the printed spellings, and the shape rules (tensor sugar / recognized loop nest →
//!     tensor; first function a counted loop → loop; otherwise whole-program).
//!  F. `the_qwen38_tiny_demo_is_the_generated_program` — `examples/rdu_demo/qwen38_tiny.lg` is byte-equal to
//!     its generator (regenerate with `LOGOS_UPDATE_FIXTURES=1`), never typed.
//!
//! ★ MUTATION PROBES (each run once, 2026-09-18; the lock went red, the impl was restored byte-identical):
//!   - drop the `escaped_first_return` gate in `lower_whole_program_lane` → C: `sign` LOWERS (to its
//!     first-return path, `0 - 1`) instead of being refused by name;
//!   - drop the recursion scan → C: the recursive fib falls through to the early-return gate and is
//!     refused with the WRONG reason (`an early return: a branch jumps past the first Return…`), which
//!     the lock catches on `the refusal names recursion and what it needs`;
//!   - map `Op::Tcp` to a no-op tile in `fabric_tile` → D: `frontier_reason` is derived from
//!     `fabric_tile`, so the ledger stops calling `Tcp` a frontier op at all and the lane would lower the
//!     program with the socket silently dropped.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_whole_program_lane_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::path::PathBuf;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_filled_lg, Qwen38Cfg};
use logicaffeine_compile::codegen_sva::fabric_tile::frontier_reason;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::ComputeStageCert;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram, StageKind, TensorRole};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::rdu_lanes::{
    first_function, lane_of, lower_whole_program_lane, RduLane,
};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::vm::Op;
use spatial_support::{gemm_program, output_id, prng};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn demo(name: &str) -> String {
    let p = repo_root().join("examples/rdu_demo").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The VM's answer for `fn_name(args)` in `source` (its own `## Main` replaced by the call).
fn vm_f(source: &str, fn_name: &str, args: &[i64]) -> i64 {
    let call = args
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let funcs = match source.find("## Main") {
        Some(idx) => &source[..idx],
        None => source,
    };
    let prog = format!("{funcs}## Main\n    Show {fn_name}({call}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error for {fn_name}({call}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("an integer")
        .parse()
        .expect("i64")
}

fn ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(_) => panic!("an integer tensor"),
    }
}

fn scalar_stage_cert(p: &SpatialProgram) -> &ComputeStageCert {
    let cert = p
        .certificate
        .as_ref()
        .expect("a verified program carries its certificate");
    cert.compute
        .iter()
        .find(|(s, _)| matches!(p.stages[s.idx()].kind, StageKind::Scalar(_)))
        .map(|(_, c)| c)
        .expect("the Scalar stage's certificate")
}

fn lanes(p: &SpatialProgram, inputs: &[Vec<i64>]) -> Vec<i64> {
    let mut ins = Inputs::default();
    for (t, xs) in p
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Input)
        .zip(inputs)
    {
        ins.tensors.insert(t.id, Value::Int(xs.clone()));
    }
    ints(
        &eval_choreography(p, &ins)
            .unwrap_or_else(|f| panic!("{}: {f:?}", p.name))
            .tensors[&output_id(p)],
    )
}

const IF_ELSE: &str = "## To clamp (x: Int) -> Int:\n    Let mutable r be x.\n    If x is less than 0:\n        Set r to 0 - x.\n    If r is greater than 1000:\n        Set r to 1000.\n    Return r.\n\n## Main\n\nShow clamp(7).\n";
const COLLECTIONS: &str = "## To tally (v: Int) -> Int:\n    Let mutable xs be [v, v * 2, 7].\n    Set item 2 of xs to v + 1.\n    Push v * 3 to xs.\n    Return item 2 of xs + length of xs + item 4 of xs.\n\n## Main\n\nShow tally(5).\n";
const TWO_INPUT: &str = "## To mix (a: Int, b: Int) -> Int:\n    Let mutable r be a * 3 - b.\n    If a is less than b:\n        Set r to b - a.\n    Return r.\n\n## Main\n\nShow mix(2, 9).\n";
const COMPUTED_PREDICATE: &str = "## To pick (n: Int) -> Int:\n    Let bit be (n shifted right by 1) & 1.\n    Let mutable r be n * 3.\n    If bit is 1:\n        Set r to n * 5.\n    Return r.\n\n## Main\n\nShow pick(6).\n";
const RECURSIVE_FIB: &str = "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n\nShow fib(10).\n";
const MUTUAL: &str = "## To even (n: Int) -> Int:\n    Let mutable r be 1.\n    If n is greater than 0:\n        Set r to odd(n - 1).\n    Return r.\n\n## To odd (n: Int) -> Int:\n    Let mutable r be 0.\n    If n is greater than 0:\n        Set r to even(n - 1).\n    Return r.\n\n## Main\n\nShow even(4).\n";
const WHILE_LOOP: &str = "## To tri (n: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n\n## Main\n\nShow tri(10).\n";
const EARLY_RETURN: &str = "## To sign (x: Int) -> Int:\n    If x is less than 0:\n        Return 0 - 1.\n    Return 1.\n\n## Main\n\nShow sign(3).\n";
const TCP: &str = "## To poke (n: Int) -> Int:\n    Let c be tcpConnect(\"127.0.0.1\", n).\n    Return n.\n\n## Main\n\nShow poke(9).\n";

// ───────────────────────────── A ─────────────────────────────

#[test]
fn a_decision_program_lowers_to_scalar_stages_on_the_rdu_and_equals_the_vm() {
    struct Case {
        label: &'static str,
        src: String,
        fn_name: &'static str,
        argc: usize,
        range: (i64, i64),
        certified: bool,
        reason_names: &'static str,
    }
    let cases = [
        Case {
            label: "fib.lg (fast doubling)",
            src: demo("fib.lg"),
            fn_name: "fib",
            argc: 1,
            range: (-5, 80),
            certified: true,
            reason_names: "",
        },
        Case {
            label: "primes.lg",
            src: demo("primes.lg"),
            fn_name: "is_prime",
            argc: 1,
            range: (-10, 130),
            certified: false,
            reason_names: "memory",
        },
        Case {
            label: "if/else",
            src: IF_ELSE.into(),
            fn_name: "clamp",
            argc: 1,
            range: (-2000, 2000),
            certified: true,
            reason_names: "",
        },
        Case {
            label: "collections",
            src: COLLECTIONS.into(),
            fn_name: "tally",
            argc: 1,
            range: (-500, 500),
            certified: false,
            reason_names: "memory",
        },
        Case {
            label: "two inputs",
            src: TWO_INPUT.into(),
            fn_name: "mix",
            argc: 2,
            range: (-300, 300),
            certified: true,
            reason_names: "",
        },
        Case {
            label: "computed predicate",
            src: COMPUTED_PREDICATE.into(),
            fn_name: "pick",
            argc: 1,
            range: (-64, 64),
            certified: false,
            reason_names: "computed value",
        },
    ];
    for c in &cases {
        assert_eq!(lane_of(&c.src), RduLane::WholeProgram, "{}", c.label);
        assert_eq!(
            first_function(&c.src),
            Some((c.fn_name.to_string(), c.argc)),
            "{}",
            c.label
        );
        for chip in RduChip::ALL {
            let p = lower_whole_program_lane(&c.src, chip)
                .unwrap_or_else(|e| panic!("{} on {}: {e:?}", c.label, chip.as_str()));
            assert_eq!(p.phase(), Phase::Verified, "{}", c.label);
            assert_eq!(p.name, format!("rdu_whole_program:{}", c.fn_name));
            let scalar = p
                .stages
                .iter()
                .filter(|s| matches!(s.kind, StageKind::Scalar(_)))
                .count();
            assert_eq!(
                scalar, 1,
                "{}: one Scalar stage carries the whole decision program",
                c.label
            );
            let inputs = p
                .tensors
                .iter()
                .filter(|t| t.role == TensorRole::Input)
                .count();
            assert_eq!(
                inputs, c.argc,
                "{}: one input vector per parameter",
                c.label
            );
            let n = 64;
            let xs: Vec<Vec<i64>> = (0..c.argc)
                .map(|k| {
                    (0..n)
                        .map(|i| prng(11 + k as u64, i, c.range.0, c.range.1))
                        .collect()
                })
                .collect();
            let got = lanes(&p, &xs);
            assert_eq!(got.len(), n, "{}", c.label);
            for i in 0..n {
                let args: Vec<i64> = xs.iter().map(|v| v[i]).collect();
                let want = vm_f(&c.src, c.fn_name, &args);
                assert_eq!(
                    got[i],
                    want,
                    "{} on {}: lane {i} ({args:?}) != VM",
                    c.label,
                    chip.as_str()
                );
            }
            match scalar_stage_cert(&p) {
                ComputeStageCert::Sat(pc) => {
                    assert!(
                        c.certified,
                        "{}: expected an honestly uncertified body, got a SAT certificate",
                        c.label
                    );
                    assert!(pc.recheck(), "{}: the per-PE certificate rechecks", c.label);
                    assert!(
                        !pc.with_last_step_removed().recheck(),
                        "{}: a tampered certificate is refused",
                        c.label
                    );
                }
                ComputeStageCert::Uncertified { reason } => {
                    assert!(
                        !c.certified,
                        "{}: an ALU + select body must certify, got `{reason}`",
                        c.label
                    );
                    assert!(
                        reason.contains(c.reason_names),
                        "{}: the reason names why: {reason}",
                        c.label
                    );
                }
            }
            let backend = RduSpatialBackend::for_chip(chip);
            match backend
                .lower(&p)
                .unwrap_or_else(|e| panic!("{} on {}: {e:?}", c.label, chip.as_str()))
            {
                SpatialArtifact::SdkGated(a) => {
                    assert!(
                        a.certificate.recheck(&p),
                        "{}: the choreography certificate rechecks",
                        c.label
                    );
                    let v: serde_json::Value = serde_json::from_str(&a.json).unwrap();
                    assert_eq!(v["lane"], "spatial");
                    assert_eq!(v["target"]["chip"], chip.as_str());
                    assert_eq!(v["target"]["name"], chip.target_name());
                    assert!(
                        a.pretty.contains("stage body Compute"),
                        "{}: the pretty text names the Scalar stage:\n{}",
                        c.label,
                        a.pretty
                    );
                }
                other => panic!("{}: the RDU artifact is SDK-gated, got {other:?}", c.label),
            }
            println!(
                "{} on {}: 64 lanes ≡ VM, certificate {}",
                c.label,
                chip.as_str(),
                if c.certified {
                    "SAT (rechecked)"
                } else {
                    "uncertified by name"
                }
            );
        }
    }
}

// ───────────────────────────── B ─────────────────────────────

#[test]
fn a_main_program_lowers_as_the_show_tape_and_equals_the_vm() {
    let mains = [
        "## Main\n    Let a be 5.\n    Show a * a.\n    Show a + 1.\n    Let xs be [10, 20, 30].\n    Show item 2 of xs.\n    Show length of xs.\n",
        "## Main\n    Let mutable s be {3, 4, 3}.\n    Remove 3 from s.\n    Show length of s.\n    Let t be (7, 8, 9).\n    Show item 1 of t + item 3 of t.\n",
        "## Main\n    Show 42.\n",
    ];
    for src in mains {
        assert_eq!(lane_of(src), RduLane::WholeProgram);
        assert_eq!(first_function(src), None, "a Main-only program");
        let p =
            lower_whole_program_lane(src, RduChip::Sn50).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        assert_eq!(p.name, "rdu_whole_program:main");
        let want: Vec<i64> = vm_outcome(src)
            .output
            .lines()
            .map(|l| l.trim().parse::<i64>().expect("an Int line"))
            .collect();
        let k = want.len();
        let out = p
            .tensors
            .iter()
            .find(|t| t.role == TensorRole::Output)
            .unwrap();
        assert_eq!(out.elems.cols, k, "one lane per Show");
        let idx: Vec<i64> = (0..k as i64).collect();
        let got = lanes(&p, &[idx]);
        assert_eq!(
            got, want,
            "the tape lowered lane-wise != the VM's stdout for:\n{src}"
        );
        match scalar_stage_cert(&p) {
            ComputeStageCert::Sat(pc) => assert!(pc.recheck()),
            ComputeStageCert::Uncertified { reason } => assert!(
                reason.contains("memory")
                    || reason.contains("computed value")
                    || reason.contains("functional-unit"),
                "{reason}"
            ),
        }
    }
    // A helper called from Main inlines (the Futamura projection): the tape reads the callee's datapath.
    let with_call = "## To sq (x: Int) -> Int:\n    Return x * x.\n\n## Main\n    Let a be 6.\n    Show sq(a).\n    Show sq(a + 1) - 1.\n";
    // The first function is `sq`, so the lane's unit is `sq` itself — the tape mode is Main-only programs.
    assert_eq!(first_function(with_call), Some(("sq".to_string(), 1)));
    let p = lower_whole_program_lane(with_call, RduChip::Sn50).unwrap();
    assert_eq!(p.name, "rdu_whole_program:sq");
    // Text on the tape has no Int lane: refused by name.
    let text = "## Main\n    Show \"hello\".\n    Show 42.\n";
    match lower_whole_program_lane(text, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(why.contains("Text"), "{why}"),
        other => panic!("a Show of Text must be refused by name, got {other:?}"),
    }
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn recursion_and_loops_are_refused_by_name_never_lowered_wrong() {
    // The VM's answer, so the refusal is measured against a program that genuinely computes something.
    assert_eq!(vm_outcome(RECURSIVE_FIB).output.trim(), "55");
    match lower_whole_program_lane(RECURSIVE_FIB, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => {
            assert!(why.contains("recursion") && why.contains("call stack"), "the refusal names recursion and what it needs: {why}");
        }
        other => panic!("a recursive function must be refused by name — the decision front end would otherwise take the first-return path only: {other:?}"),
    }
    // Meanwhile the whole-program call-stack engine runs it ≡ VM: the frontier is engine-realized, not
    // stage-realized, and the lane says exactly that.
    let cfg = program_calls_fabric(RECURSIVE_FIB, "fib");
    assert_eq!(
        eval_wprog_config(&cfg, &[10]),
        55,
        "the call-stack engine computes fib(10)"
    );
    match lower_whole_program_lane(MUTUAL, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(
            why.contains("recursion"),
            "mutual recursion is recursion: {why}"
        ),
        other => panic!("mutual recursion must be refused by name: {other:?}"),
    }
    match lower_whole_program_lane(WHILE_LOOP, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(
            why.contains("loop"),
            "a while loop is refused by name: {why}"
        ),
        other => panic!("a while loop must be refused by name: {other:?}"),
    }
    let counted = demo("dot.lg");
    assert_eq!(lane_of(&counted), RduLane::Loop);
    match lower_whole_program_lane(&counted, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(
            why.contains("loop lane"),
            "a counted loop is pointed at the loop lane: {why}"
        ),
        other => panic!(
            "a counted loop on the whole-program lane must point at the loop lane: {other:?}"
        ),
    }
    match lower_whole_program_lane(EARLY_RETURN, RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(why.contains("return"), "an early return is refused by name: {why}"),
        other => panic!("an early-return function must be refused by name (the decision front end takes the first-return path only): {other:?}"),
    }
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn an_op_on_the_frontier_is_refused_by_name_not_silently_dropped() {
    let tcp = Op::Tcp {
        dst: 0,
        kind: 0,
        args_start: 0,
        arg_count: 0,
    };
    let reason = frontier_reason(&tcp).expect("Tcp is on the frontier");
    match lower_whole_program_lane(TCP, RduChip::Sn50) {
        Err(SpatialReject::MissingCompute { needed, target }) => {
            assert!(needed.contains("Tcp"), "the refusal names the op: {needed}");
            assert_eq!(needed, reason, "…through the one frontier ledger");
            assert_eq!(target, "sn50-rdu");
        }
        other => panic!("a program on the fabric frontier must be refused by name, never lowered with the op dropped: {other:?}"),
    }
    match lower_whole_program_lane(TCP, RduChip::Sn40l) {
        Err(SpatialReject::MissingCompute { target, .. }) => assert_eq!(target, "sn40l-rdu"),
        other => panic!("{other:?}"),
    }
}

// ───────────────────────────── E ─────────────────────────────

const DEMO_LANES: &[(&str, RduLane)] = &[
    ("dot.lg", RduLane::Loop),
    ("fib.lg", RduLane::WholeProgram),
    ("primes.lg", RduLane::WholeProgram),
    ("gemm.lg", RduLane::Tensor),
    ("rmsnorm.lg", RduLane::Tensor),
    ("qwen38_tiny.lg", RduLane::Tensor),
];

#[test]
fn the_lane_is_printed_and_is_a_function_of_the_program() {
    assert_eq!(RduLane::Tensor.as_str(), "tensor");
    assert_eq!(RduLane::Loop.as_str(), "loop");
    assert_eq!(RduLane::WholeProgram.as_str(), "whole-program");
    assert_eq!(RduLane::ALL.len(), 3);
    for lane in RduLane::ALL {
        assert_eq!(RduLane::parse(lane.as_str()), Some(lane));
    }
    let readme = demo("README.md");
    let mut files: Vec<String> = std::fs::read_dir(repo_root().join("examples/rdu_demo"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".lg"))
        .collect();
    files.sort();
    let mut expected: Vec<String> = DEMO_LANES.iter().map(|(f, _)| f.to_string()).collect();
    expected.sort();
    assert_eq!(
        files, expected,
        "the corpus IS the directory: every demo program has a pinned lane"
    );
    for (file, lane) in DEMO_LANES {
        let src = demo(file);
        assert_eq!(lane_of(&src), *lane, "{file}");
        assert_eq!(
            lane_of(&src),
            lane_of(&src),
            "{file}: a pure function of the source"
        );
        let row = readme
            .lines()
            .find(|l| l.starts_with(&format!("| `{file}`")))
            .unwrap_or_else(|| panic!("README lists {file}"));
        assert!(
            row.contains(&format!("| `{}` |", lane.as_str())),
            "README names {file}'s lane `{}`: {row}",
            lane.as_str()
        );
    }
    // The shape rules, on programs without sugar: a recognized matmul loop nest is the tensor lane; a
    // counted-loop first function is the loop lane; a decision function or a bare Main is whole-program.
    assert_eq!(
        lane_of(&gemm_program(8, 8, 8)),
        RduLane::Tensor,
        "the recognizer's own matmul nest"
    );
    assert_eq!(lane_of("## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        Set s to s * 3 + i.\n    Return s.\n\n## Main\n\nShow f(10).\n"), RduLane::Loop);
    assert_eq!(lane_of(IF_ELSE), RduLane::WholeProgram);
    assert_eq!(lane_of("## Main\n    Show 1.\n"), RduLane::WholeProgram);
    assert_eq!(
        lane_of(RECURSIVE_FIB),
        RduLane::WholeProgram,
        "the lane is chosen by shape; the lowering refuses recursion by name"
    );
    assert_eq!(
        lane_of("## Main\n    Let A be a 2 by 2 tensor of Int.\n    Show item 1 of A.\n"),
        RduLane::Tensor,
        "a tensor declaration is the tensor lane even before an op is recognized"
    );
    let broken = "## Main\n    Show (1 + .\n";
    assert_eq!(
        lane_of(broken),
        RduLane::WholeProgram,
        "an unparsable source falls to the lane whose lowering reports the parse error"
    );
    match lower_whole_program_lane(broken, RduChip::Sn50) {
        Err(SpatialReject::Parse(_)) => {}
        other => panic!("the parse error is reported typed: {other:?}"),
    }
    match lower_whole_program_lane("this is not a program", RduChip::Sn50) {
        Err(SpatialReject::NotLowered(why)) => assert!(
            why.contains("Show"),
            "free text parses as an empty Main, refused by name: {why}"
        ),
        other => panic!("{other:?}"),
    }
}

// ───────────────────────────── F ─────────────────────────────

#[test]
fn the_qwen38_tiny_demo_is_the_generated_program() {
    let path = repo_root().join("examples/rdu_demo/qwen38_tiny.lg");
    let generated = gen_qwen38_filled_lg(&Qwen38Cfg::tiny(), "/w");
    if std::env::var_os("LOGOS_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &generated).expect("write the demo program");
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} — regenerate with LOGOS_UPDATE_FIXTURES=1",
            path.display()
        )
    });
    assert_eq!(on_disk, generated, "examples/rdu_demo/qwen38_tiny.lg is generated, never typed — regenerate with LOGOS_UPDATE_FIXTURES=1");
    assert!(
        generated.contains(" tensor of Real.")
            && generated.contains("Show round(item 1 of logits * 1000000)."),
        "the tiny forward with its two logit Shows"
    );
    assert!(
        generated
            .lines()
            .filter(|l| l.starts_with("Set item "))
            .count()
            > 100,
        "the weights are filled deterministically"
    );
    assert_eq!(lane_of(&generated), RduLane::Tensor);
    let vm = vm_outcome(&generated);
    assert!(
        vm.error.is_none(),
        "the demo program runs on the VM: {:?}",
        vm.error
    );
    assert_eq!(
        vm.output.split_whitespace().count(),
        2,
        "two logits: {:?}",
        vm.output
    );
}
