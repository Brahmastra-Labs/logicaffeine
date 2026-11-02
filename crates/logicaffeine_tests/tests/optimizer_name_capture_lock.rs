//! An optimized program reads the names its source reads.
//!
//! A function's frame sees its parameters and its own locals, then the program's bindings — never its
//! caller's locals (`Interpreter::push_frame`). An inliner that substitutes a callee's body at a call site
//! moves the callee's FREE names — the program bindings it reads, or names it reads that nothing binds —
//! into the caller's scope, where a local of the same name captures them: `## To peek -> Int: Return x.`
//! inlined into `## To outer` that has `Let x be 7` answers 7 where the interpreter answers the program's
//! `x`. Both engines of the run path receive the one optimized program, so the shadow oracle cannot see
//! it; `optimize_program` feeds every compiled tier the same way. A pass that carries what it knows of a
//! name — constant propagation's literal for `x` — must forget it wherever the source binds `x` again: a
//! later `Let`, a `Receive`, a loop's pattern, a match arm's bindings. A specialization's arguments are the
//! caller's names too: a caller's `beta` passed where the callee's own `beta` receives a literal is not that
//! literal.
//!
//! Each program runs on the tree-walker, unoptimized, for the reference, then on the run path
//! (`largo run --interpret`: the optimized program), on the C tier (`compile_to_c` optimizes too), and
//! compiled to Rust.

mod common;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_c, tw_outcome};
use logicaffeine_compile::interpret_for_ui_sync_with_args;

/// What a run printed, and whether it failed.
#[derive(Debug, PartialEq)]
struct Outcome {
    lines: Vec<String>,
    failed: bool,
}

fn reference(src: &str) -> Outcome {
    let tw = tw_outcome(src);
    Outcome {
        lines: tw.output.lines().map(str::to_string).collect(),
        failed: tw.error.is_some(),
    }
}

fn run_path(src: &str) -> Outcome {
    let r = interpret_for_ui_sync_with_args(src, &[]);
    Outcome {
        lines: r.lines,
        failed: r.error.is_some(),
    }
}

fn c_tier(name: &str, src: &str) -> Result<Outcome, String> {
    let c = compile_to_c(src).map_err(|e| format!("does not lower to C: {e:?}"))?;
    let dir = std::env::temp_dir().join(format!("logos_capture_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, bin) = (dir.join("prog.c"), dir.join("prog"));
    std::fs::write(&file, &c).unwrap();
    let out = Command::new("gcc")
        .args(["-O1", "-o"])
        .arg(&bin)
        .arg(&file)
        .arg("-lm")
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "does not build as C:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let run = Command::new(&bin).output().unwrap();
    Ok(Outcome {
        lines: String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        failed: !run.status.success(),
    })
}

/// The rows the C tier refuses, in the words of its refusal — each still refused, or it leaves this list
/// for the comparison it then owes.
const C_REFUSES: &[(&str, &str)] = &[];

/// The rows the C tier still answers otherwise, each with its cause — each still differs, or it leaves this
/// list for the comparison it then owes.
const C_DIFFERS: &[(&str, &str)] = &[];

/// `(name, source, the reference's lines, whether it fails)` — each pinned, so a reference that drifts
/// fails here before any tier is compared with it.
const CORPUS: &[(&str, &str, &[&str], bool)] = &[
    (
        // The tiny inliner's shape: a one-expression helper reading the program's binding.
        "a_tiny_helper_reads_the_programs_binding_not_its_callers_local",
        "## To peek -> Int:\n    Return x.\n\n## To outer -> Int:\n    Let x be 7.\n    Return peek().\n\n\
         ## Main\nLet x be 1.\nShow outer().\nShow x.\n",
        &["1", "1"],
        false,
    ),
    (
        // A binding in a block of Main is that block's: the helper reading it fails, inlined or not.
        "a_tiny_helper_never_reads_a_block_of_mains_binding",
        "## To peek -> Int:\n    Return inner.\n\n## Main\nIf true:\n    Let inner be 5.\n    Show peek().\n",
        &[],
        true,
    ),
    (
        // An argument the caller passes is the caller's; the helper's own free name is the program's.
        "an_argument_and_a_free_name_of_one_spelling_stay_two_names",
        "## To plus_x (p: Int) -> Int:\n    Return p + x.\n\n## To outer -> Int:\n    Let x be 7.\n    Return plus_x(x).\n\n\
         ## Main\nLet x be 1.\nShow outer().\n",
        &["8"],
        false,
    ),
    (
        // The leaf inliner's shape: a straight-line prefix, then a returned expression.
        "a_leaf_helper_reads_the_programs_binding_not_its_callers_local",
        "## To peek -> Int:\n    Let y be x + 0.\n    Return y.\n\n## To outer -> Int:\n    Let x be 7.\n    Return peek().\n\n\
         ## Main\nLet x be 1.\nShow outer().\nShow x.\n",
        &["1", "1"],
        false,
    ),
    (
        // A loop variable of the caller is one of its locals too.
        "a_helper_called_in_a_loop_never_reads_the_loop_variable",
        "## To peek -> Int:\n    Return i.\n\n## To outer -> Int:\n    Let mutable total be 0.\n    \
         Repeat for i from 1 to 3:\n        Set total to total + peek().\n    Return total.\n\n\
         ## Main\nLet i be 10.\nShow outer().\n",
        &["30"],
        false,
    ),
    (
        // A branch whose condition is known is still a block: its binding ends with it.
        "a_known_branchs_binding_ends_with_the_branch",
        "## To f -> Int:\n    Let x be 1.\n    If true:\n        Let x be 2.\n        Show x.\n    Return x.\n\n\
         ## Main\nShow f().\n",
        &["2", "1"],
        false,
    ),
    (
        // A known branch of Main binds for the branch, not for the program a function reads.
        "a_known_branch_of_main_binds_nothing_a_function_reads",
        "## To peek -> Int:\n    Return y.\n\n## Main\nLet y be 1.\nIf true:\n    Let y be 2.\n    Show y.\nShow peek().\n",
        &["2", "1"],
        false,
    ),
    (
        // A branch taken on a loop's first iteration keeps its binding inside the branch.
        "a_peeled_iterations_binding_stays_in_its_branch",
        "## Main\nLet x be 1.\nLet mutable i be 0.\nWhile i is less than 2:\n    If i equals 0:\n        Let x be 99.\n        \
         Show x.\n    Otherwise:\n        Show 0.\n    Show x.\n    Set i to i + 1.\n",
        &["99", "1", "0", "1"],
        false,
    ),
    (
        // A branch on a loop-invariant condition keeps its binding inside the branch.
        "an_unswitched_branchs_binding_stays_in_its_branch",
        "## Main\nLet x be 1.\nLet flag be length of args() is greater than 0.\nLet mutable i be 0.\n\
         While i is less than length of args() + 1:\n    If flag:\n        Let x be 5.\n        Show x.\n    Otherwise:\n        Show 0.\n    \
         Show x.\n    Set i to i + 1.\n",
        &["5", "1", "5", "1"],
        false,
    ),
    (
        // A helper that sets a program binding runs where its call is: an expression's earlier operand reads the
        // binding before the call writes it.
        "a_helper_that_writes_the_programs_binding_runs_where_it_is_called",
        "## To next -> Int:\n    Set counter to counter + 1.\n    Return counter.\n\n\
         ## Main\nLet mutable counter be 0.\nShow counter + next().\nShow counter.\n",
        &["1", "1"],
        false,
    ),
    (
        // A binding a loop's body makes is that pass's: it hides the program's constant of one spelling
        // until the pass ends.
        "a_loop_bodys_binding_hides_the_constant_it_shadows",
        "## Main\nLet x be 1.\nLet mutable i be 0.\nWhile i is less than 2:\n    Let x be i + 10.\n    Show x.\n    \
         Set i to i + 1.\nShow x.\n",
        &["10", "11", "1"],
        false,
    ),
    (
        // A loop variable hides the constant of its spelling inside the loop, and only there.
        "a_loop_variable_hides_the_constant_it_shadows",
        "## Main\nLet i be 5.\nRepeat for i from 1 to 2:\n    Show i.\nShow i.\n",
        &["1", "2", "5"],
        false,
    ),
    (
        // A later binding of one name in the same block replaces the constant for what follows.
        "a_later_binding_replaces_the_constant_for_what_follows",
        "## To f -> Int:\n    Let x be 1.\n    Let x be length of args() + 3.\n    Return x.\n\n## Main\nShow f().\n",
        &["4"],
        false,
    ),
    (
        // A receive binds its name like a `Let`: the constant it replaces is gone.
        "a_receive_replaces_the_constant_it_rebinds",
        "## Main\nLet x be 5.\nLet ch be a Pipe of Int.\nSend 7 into ch.\nReceive x from ch.\nShow x.\n",
        &["7"],
        false,
    ),
    (
        // A match arm's binding hides the constant of its spelling inside the arm.
        "a_match_arms_binding_hides_the_constant_it_shadows",
        "## A Shape is either:\n    a Circle with radius Int.\n    a Square with side Int.\n\n\
         ## Main\nLet radius be 1.\nLet s be a new Circle with radius 9.\nInspect s:\n    When Circle (radius):\n        \
         Show radius.\n    When Square (side):\n        Show side.\nShow radius.\n",
        &["9", "1"],
        false,
    ),
    (
        // A known branch kept as a block ends its binding with it, for every pass after dead-branch elimination.
        "a_known_branchs_binding_never_reaches_the_statements_after_it",
        "## Main\nLet x be 1.\nIf true:\n    Let x be 2.\n    Show x.\nShow x.\n",
        &["2", "1"],
        false,
    ),
    (
        // Both arms binding one name to one value still bind it only inside themselves.
        "both_arms_binding_one_value_never_reach_the_statements_after_them",
        "## Main\nLet x be 1.\nLet flag be length of args() is greater than 5.\nIf flag:\n    Let x be 2.\n    Show x.\n\
         Otherwise:\n    Let x be 2.\n    Show x.\nShow x.\n",
        &["2", "1"],
        false,
    ),
    (
        // A closure's parameter binds inside the closure, never inside the helper it calls.
        "a_closure_parameter_never_binds_the_helpers_free_name",
        "## To peek -> Int:\n    Return x.\n\n## Main\nLet x be 1.\nLet f be (x: Int) -> peek() + x.\nShow f(10).\n",
        &["11"],
        false,
    ),
    (
        // A name a match arm sets holds what the arm set, not what it held before the match.
        "a_match_arms_assignment_replaces_the_constant",
        "## A Shape is either:\n    a Circle with radius Int.\n    a Square with side Int.\n\n\
         ## Main\nLet mutable x be 0.\nSet x to 5.\nLet s be a new Circle with radius 3.\nInspect s:\n    When Circle (r):\n        \
         Set x to r.\n    When Square (q):\n        Set x to q.\nShow x.\n",
        &["3"],
        false,
    ),
    (
        // `Pop … into` binds its name like a `Set`: the constant it replaces is gone.
        "a_pop_into_replaces_the_constant_it_rebinds",
        "## Main\nLet mutable xs be [1, 2, 3].\nLet mutable y be 0.\nSet y to 7.\nPop from xs into y.\nShow y.\n",
        &["3"],
        false,
    ),
    (
        // A loop that runs no pass leaves what its body sets as it was.
        "a_loop_that_never_runs_leaves_what_its_body_sets",
        "## Main\nLet mutable t be 1.\nLet xs be a new Seq of Int.\nRepeat for x in xs:\n    Set t to 5.\nShow t.\n",
        &["1"],
        false,
    ),
    (
        // Each name a tuple pattern binds hides the constant of its spelling inside the loop, and only there.
        "a_tuple_patterns_names_hide_the_constants_they_shadow",
        "## Main\nLet k be \"zero\".\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\n\
         Repeat for (k, v) in m:\n    Show k.\nShow k.\n",
        &["a", "zero"],
        false,
    ),
    (
        // A specialization's arguments are the caller's: the caller's `beta` passed as the helper's `alpha` is not
        // the helper's own `beta`, which this call binds to a literal.
        "a_specialized_clone_reads_the_callers_argument_not_its_own_parameter",
        "## To g (seed: Int, alpha: Real, beta: Real) -> Real:\n    Let mutable s be seed.\n    Let mutable i be 0.\n    \
         While i is less than 3:\n        Set s to s + 1.\n        Set i to i + 1.\n    Return alpha * 10.0 + beta.\n\n\
         ## To f (seed: Int, alpha: Real, beta: Real) -> Real:\n    Let y be g(seed + 1, beta, 1.0).\n    Return y.\n\n\
         ## Main\nShow f(42, 2.0, 3.0).\n",
        &["31"],
        false,
    ),
    (
        // Two arguments named like two parameters the call binds to literals, with one argument left unknown.
        "a_specialized_clone_reads_two_arguments_named_like_its_parameters",
        "## To h (a: Int, b: Int, c: Int, d: Int, e: Int) -> Int:\n    Let mutable k be 0.\n    While k is less than 2:\n        \
         Set k to k + 1.\n    Return e * 10000 + a * 1000 + b * 100 + c * 10 + d.\n\n\
         ## To caller (c: Int, d: Int, e: Int) -> Int:\n    Return h(d, c, 7, 8, e).\n\n## Main\nShow caller(1, 2, 3).\n",
        &["32178"],
        false,
    ),
    (
        // The stdlib's shape: `randBeta` draws its second gamma at the caller's `beta`, with a literal scale.
        "a_stdlib_specialization_reads_the_callers_argument",
        "## Main\nShow floor(randBeta(42, 2.0, 3.0) * 1000.0).\n",
        &["201"],
        false,
    ),
    (
        // A branch taken for certain still sets the enclosing binding it names.
        "a_known_branchs_assignment_reaches_the_statements_after_it",
        "## To f -> Int:\n    Let mutable total be 1.\n    If true:\n        Let step be 4.\n        Set total to total + step.\n    \
         Return total.\n\n## Main\nShow f().\n",
        &["5"],
        false,
    ),
];

#[test]
fn the_reference_shows_what_each_row_pins() {
    for (name, src, lines, fails) in CORPUS {
        let want = Outcome {
            lines: lines.iter().map(|l| l.to_string()).collect(),
            failed: *fails,
        };
        assert_eq!(
            reference(src),
            want,
            "{name}: the tree-walker no longer shows what the row pins"
        );
    }
}

#[test]
fn the_run_path_reads_what_the_interpreter_reads() {
    let mut captured = Vec::new();
    for (name, src, _, _) in CORPUS {
        let (want, got) = (reference(src), run_path(src));
        if got != want {
            captured.push(format!(
                "{name}:\n  interpreter {want:?}\n  run path    {got:?}"
            ));
        }
    }
    assert!(
        captured.is_empty(),
        "the optimized run path reads another name:\n\n{}",
        captured.join("\n\n")
    );
}

#[test]
fn the_c_tier_reads_what_the_interpreter_reads() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut captured = Vec::new();
    for (name, src, _, fails) in CORPUS {
        if *fails {
            continue;
        }
        let refused = C_REFUSES
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, words)| *words);
        let differs = C_DIFFERS.iter().any(|(n, _)| n == name);
        match (c_tier(name, src), refused) {
            (Err(why), Some(words)) if why.contains(words) => {}
            (Err(_), None) if differs => {}
            (Err(why), _) => captured.push(format!("{name}: {why}")),
            (Ok(_), Some(_)) => {
                captured.push(format!("{name}: now builds as C — take it off C_REFUSES"))
            }
            (Ok(got), None) => {
                let want = reference(src);
                match (got == want, differs) {
                    (true, true) => {
                        captured.push(format!("{name}: now agrees on C — take it off C_DIFFERS"))
                    }
                    (false, false) => captured.push(format!(
                        "{name}:\n  interpreter {want:?}\n  C           {got:?}"
                    )),
                    _ => {}
                }
            }
        }
    }
    assert!(
        captured.is_empty(),
        "the C tier reads another name:\n\n{}",
        captured.join("\n\n")
    );
}

/// The same programs compiled to Rust — `optimize_program`, supercompilation included, then the Rust
/// backend — read what the interpreter reads.
#[test]
fn the_compiled_rust_tier_reads_what_the_interpreter_reads() {
    let mut captured = Vec::new();
    for (name, src, _, fails) in CORPUS {
        if *fails {
            continue;
        }
        let run = common::run_logos(src);
        if !run.success {
            captured.push(format!(
                "{name}: does not build or run as Rust:\n{}",
                run.stderr
            ));
            continue;
        }
        let got = Outcome {
            lines: run.stdout.lines().map(str::to_string).collect(),
            failed: false,
        };
        let want = reference(src);
        if got != want {
            captured.push(format!(
                "{name}:\n  interpreter {want:?}\n  Rust        {got:?}"
            ));
        }
    }
    assert!(
        captured.is_empty(),
        "the compiled Rust tier reads another name:\n\n{}",
        captured.join("\n\n")
    );
}

/// The optimizer passes that move a body across a scope boundary, and the `capture` question each asks: an
/// inliner, whether a callee's free names are bound where it lands; a splicer, whether a block's own names
/// reach the statements it lands in front of.
const MOVERS: &[(&str, &str)] = &[
    ("inline_tiny.rs", "capture::captures"),
    ("inline_leaf.rs", "capture::captures"),
    ("dce.rs", "capture::own_names"),
    ("licm.rs", "capture::splices_before"),
];

/// Passes that still splice without asking, each with what it waits on — each still does, or it moves to
/// MOVERS with the question it then asks.
const SPLICES_UNASKED: &[(&str, &str, &str)] = &[];

/// Inliners that only ever inline a function into itself: a function's free names are, by definition, bound
/// nowhere in its own frame, so there is nothing to capture.
const SELF_ONLY: &[&str] = &["inline_recursive.rs"];

fn optimize_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_compile/src/optimize")
}

#[test]
fn every_pass_that_moves_a_body_asks_the_capture_question() {
    for (file, question) in MOVERS {
        let text = std::fs::read_to_string(optimize_dir().join(file)).unwrap();
        assert!(
            text.contains(question),
            "{file} moves a body across a scope and never asks `{question}`"
        );
    }
    for (file, question, _) in SPLICES_UNASKED {
        let text = std::fs::read_to_string(optimize_dir().join(file)).unwrap();
        assert!(
            !text.contains(question),
            "{file} asks `{question}` now — move it to MOVERS"
        );
    }
}

#[test]
fn no_pass_inlines_or_splices_a_branch_without_asking() {
    let listed = |f: &str| {
        MOVERS.iter().any(|(m, _)| *m == f)
            || SELF_ONLY.contains(&f)
            || SPLICES_UNASKED.iter().any(|(m, _, _)| *m == f)
    };
    let mut unasked = Vec::new();
    for entry in std::fs::read_dir(optimize_dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.ends_with(".rs") || listed(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        if file.starts_with("inline") {
            unasked.push(format!(
                "{file}: an inliner neither in MOVERS nor SELF_ONLY"
            ));
        }
        for splice in [".extend(then_block", ".extend(else_block", ".extend(branch"] {
            if text.contains(splice) {
                unasked.push(format!(
                    "{file}: splices a branch (`{splice}`) and is not in MOVERS"
                ));
            }
        }
    }
    assert!(
        unasked.is_empty(),
        "a pass moves a body without the capture question:\n{}",
        unasked.join("\n")
    );
}
