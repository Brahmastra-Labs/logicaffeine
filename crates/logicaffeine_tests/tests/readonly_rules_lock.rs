//! ★ LOCK — read-only parameter inference is four Datalog rules. ★
//!
//! Codegen borrows a `Seq` parameter as `&[T]` when [`ReadonlyParams`] finds that its function never changes it, by a
//! hand-written fixpoint over the call graph. `analysis::readonly_rules` states the same inference as four rules over
//! facts read off the program (`MutatesDirectly`, `PassesTo`, `SeqParam`, `Plain`) and evaluates them with
//! `logicaffeine_proof::datalog`. Held here: on every program of the benchmark corpus, the examples and the test
//! fixtures, with the standard library they splice, the rules find exactly the parameters the hand-written pass finds;
//! and each rule decides a parameter of its own in a program written for it.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use logicaffeine_compile::analysis::callgraph::CallGraph;
use logicaffeine_compile::analysis::readonly::ReadonlyParams;
use logicaffeine_compile::analysis::readonly_rules::{readonly_by_rules, readonly_facts};
use logicaffeine_compile::analysis::TypeEnv;

/// Each function's read-only parameters by name.
type ByName = BTreeMap<String, BTreeSet<String>>;

/// Both inferences' answers for `src` — `(hand-written, rules)` — and how many `Seq` parameters were candidates.
fn both(src: &str) -> Option<(ByName, ByName, usize)> {
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.ok()?;
        let type_env = TypeEnv::infer_program(stmts, interner, types);
        let callgraph = CallGraph::build(stmts, interner);
        let named = |m: &std::collections::HashMap<_, std::collections::HashSet<_>>| -> ByName {
            m.iter()
                .map(|(f, ps)| {
                    (
                        interner.resolve(*f).to_string(),
                        ps.iter().map(|p| interner.resolve(*p).to_string()).collect(),
                    )
                })
                .collect()
        };
        let hand = named(&ReadonlyParams::analyze(stmts, &callgraph, &type_env).readonly);
        let rules = named(&readonly_by_rules(stmts, &type_env).expect("the rules evaluate"));
        let candidates = readonly_facts(stmts, &type_env).iter().filter(|a| a.predicate == "SeqParam").count();
        Some((hand, rules, candidates))
    })
}

fn read_only(src: &str, function: &str) -> BTreeSet<String> {
    let (hand, rules, _) = both(src).expect("the program parses");
    assert_eq!(hand, rules, "the rules and the hand-written pass differ:\n{src}");
    rules.get(function).cloned().unwrap_or_default()
}

#[test]
fn a_parameter_the_body_changes_is_not_read_only() {
    let src = "## To fill (xs: Seq of Int) and (ys: Seq of Int) -> Int:\n    Push 1 to xs.\n    Return length of ys.\n\n\
               ## Main\nLet mutable a be [1].\nLet b be [2].\nShow fill(a, b).\n";
    assert_eq!(read_only(src, "fill"), BTreeSet::from(["ys".to_string()]));
}

#[test]
fn a_parameter_handed_to_a_function_that_changes_it_is_not_read_only() {
    let src = "## To grow (xs: Seq of Int):\n    Push 0 to xs.\n\n\
               ## To outer (xs: Seq of Int) and (ys: Seq of Int) -> Int:\n    Call grow with xs.\n    Return length of ys.\n\n\
               ## Main\nLet mutable a be [1].\nLet b be [2].\nShow outer(a, b).\n";
    assert_eq!(read_only(src, "outer"), BTreeSet::from(["ys".to_string()]));
    assert!(read_only(src, "grow").is_empty());
}

#[test]
fn mutual_recursion_that_only_reads_keeps_its_parameters_read_only() {
    let src = "## To even (xs: Seq of Int) and (n: Int) -> Bool:\n    If n is 0:\n        Return true.\n    Return odd(xs, n - 1).\n\n\
               ## To odd (xs: Seq of Int) and (n: Int) -> Bool:\n    If n is 0:\n        Return false.\n    Return even(xs, n - 1).\n\n\
               ## Main\nShow even([1, 2], 3).\n";
    assert_eq!(read_only(src, "even"), BTreeSet::from(["xs".to_string()]));
    assert_eq!(read_only(src, "odd"), BTreeSet::from(["xs".to_string()]));
}

#[test]
fn a_mutation_inside_the_recursion_reaches_every_function_of_it() {
    let src = "## To even (xs: Seq of Int) and (n: Int) -> Bool:\n    If n is 0:\n        Push 0 to xs.\n        Return true.\n    Return odd(xs, n - 1).\n\n\
               ## To odd (xs: Seq of Int) and (n: Int) -> Bool:\n    If n is 0:\n        Return false.\n    Return even(xs, n - 1).\n\n\
               ## Main\nLet mutable a be [1].\nShow even(a, 3).\n";
    assert!(read_only(src, "even").is_empty());
    assert!(read_only(src, "odd").is_empty());
}

fn lg_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            lg_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "lg") {
            out.push(path);
        }
    }
}

#[test]
fn the_rules_find_exactly_the_read_only_parameters_the_hand_written_pass_finds() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["benchmarks/programs", "examples", "crates/logicaffeine_tests/tests/fixtures"] {
        lg_files(&root.join(dir), &mut files);
    }
    files.sort();
    let (mut programs, mut read_only, mut candidates) = (0, 0, 0);
    let mut differ = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("read");
        let Some((hand, rules, seq_params)) = both(&src) else { continue };
        programs += 1;
        read_only += hand.values().map(BTreeSet::len).sum::<usize>();
        candidates += seq_params;
        if hand != rules {
            differ.push(format!("{}: hand-written {hand:?}, rules {rules:?}", path.display()));
        }
    }
    assert!(differ.is_empty(), "{} programs differ:\n{}", differ.len(), differ.join("\n"));
    assert!(
        programs >= 100 && read_only >= 50 && candidates >= read_only + 20,
        "the corpus decides both ways: {programs} programs, {read_only} of {candidates} Seq parameters read-only"
    );
}
