//! ★ LOCK — Datalog evaluated by a program written in LOGOS, on every tier. ★
//!
//! The standard library's `datalog` module (`crates/logicaffeine_compile/assets/std/datalog.lg`) computes the least
//! model of a Datalog program — ground facts and Horn rules, with negation over lower strata — semi-naively, in pure
//! LOGOS. Held here against `logicaffeine_proof::datalog`, the Rust engine, on one LOGOS program that evaluates every
//! program of the corpus in turn, each in a procedure of its own, so each tier compiles the evaluator once:
//!
//! - Whaley and Lam's four points-to rules, 120 random stratified programs (recursion, constants in atoms, repeated
//!   variables, nullary and ternary relations, negation of derived relations) and constants an end-to-end encoding of
//!   tuples would confuse give the Rust engine's model, fact for fact and each fact once, on the tree-walker, the VM,
//!   the optimized VM and the JIT;
//! - the given facts come first, each once, in the order given;
//! - a program with no stratified least model is refused in the Rust engine's words;
//! - a sample of them prints the same lines compiled to Rust, to C and to C through bytecode.
//!
//! `wasm_aot_lock::the_datalog_evaluator_runs_on_wasm` runs the sample on the direct WASM backend.

#![cfg(not(target_arch = "wasm32"))]

mod common;
mod datalog_corpus;

use common::{assert_c_output, assert_compiled_equals_interpreted, build_c_release, run_all_tiers};
use datalog_corpus::{
    adversarial_constants, expected, given_facts, logos_programs, points_to, printed, random_program, refused, sampled,
    sections, Printed, SplitMix,
};
use logicaffeine_proof::datalog::{Literal, Program};

/// `programs` evaluated in LOGOS, in one program, on every interpreted tier (which agree with one another), each
/// against the Rust engine: the program, what each printed by name, and the whole output.
fn agree(programs: &[(String, Program)]) -> (String, Vec<(String, String)>, String) {
    let src = logos_programs(programs);
    let run = run_all_tiers(&src);
    assert_eq!(run.error, None, "the corpus runs cleanly:\n{src}");
    let parts = sections(&run.output);
    assert_eq!(parts.len(), programs.len(), "one section per program:\n{}", run.output);
    for ((name, p), (shown, text)) in programs.iter().zip(&parts) {
        assert_eq!(name, shown, "the sections come in order");
        assert_eq!(printed(text), expected(p), "`{name}`: the LOGOS evaluator and the Rust engine differ");
    }
    (src, parts, run.output)
}

#[test]
fn the_evaluator_gives_the_rust_engines_model_on_every_interpreted_tier() {
    let mut twice = points_to();
    let mut facts = twice.facts().to_vec();
    facts.push(facts[0].clone());
    twice = Program::new(facts, twice.rules().to_vec());
    let mut corpus = vec![
        ("points_to".to_string(), points_to()),
        ("a fact given twice".to_string(), twice.clone()),
        ("adversarial constants".to_string(), adversarial_constants()),
    ];
    corpus.extend(refused().into_iter().map(|(n, p)| (n.to_string(), p)));
    let mut rng = SplitMix(0x5EED_DA7A_0001);
    for i in 0..120 {
        corpus.push((format!("random {i}"), random_program(&mut rng)));
    }
    let (_, parts, _) = agree(&corpus);
    let text = |name: &str| parts.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()).expect("a section");

    let Printed::Model(model) = printed(&text("points_to")) else { panic!("points_to has a model") };
    for fact in ["vP(c, h1)", "vP(d, h2)", "vP(d, h3)", "hP(h1, next, h2)", "hP(h1, next, h3)"] {
        assert!(model.contains(fact), "{fact} is derived");
    }
    assert_eq!(model.len(), 15, "the seven statements and eight derived facts");

    let given = given_facts(&twice);
    let shown = text("a fact given twice");
    let lines: Vec<&str> = shown.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(&lines[..given.len()], given.iter().map(String::as_str).collect::<Vec<_>>().as_slice());

    let Printed::Model(adversarial) = printed(&text("adversarial constants")) else { panic!("a model") };
    assert_eq!(adversarial.len(), 10, "five facts and their five swaps stay apart");

    for (name, _) in refused() {
        assert!(matches!(printed(&text(name)), Printed::Refused(_)), "`{name}` is refused");
    }

    let random = &corpus[corpus.len() - 120..];
    let derived = random
        .iter()
        .filter(|(name, p)| matches!(printed(&text(name)), Printed::Model(m) if m.len() > given_facts(p).len()))
        .count();
    let negated = random
        .iter()
        .filter(|(_, p)| p.rules().iter().any(|r| r.body.iter().any(|l| matches!(l, Literal::Neg(_)))))
        .count();
    assert!(derived >= 60 && negated >= 40, "the random corpus derives ({derived}) and negates ({negated})");
}

#[test]
fn compiled_to_rust_c_and_bytecode_c_the_evaluator_prints_the_same_lines() {
    let (src, _, output) = agree(&sampled());
    let lines = output.trim();
    assert_c_output(&src, lines);
    let c = logicaffeine_compile::compile::compile_to_c_via_bytecode(&src).expect("the bytecode C compile");
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(built.status.success(), "the bytecode C builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = std::process::Command::new(&bin).output().expect("run the bytecode C");
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), lines, "bytecode C");
    assert_compiled_equals_interpreted(&src);
}
