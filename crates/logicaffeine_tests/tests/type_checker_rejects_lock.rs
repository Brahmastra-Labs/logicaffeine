//! Wave 2 — a statically typed language must reject programs that cannot possibly work.
//!
//! The v1.0 readiness audit ran a set of plainly broken programs through the shipped `largo 0.10.1`
//! and found that `largo check` PASSED almost all of them: an undefined variable, a call with the
//! wrong number of arguments, `double("abc")` against a `n: Int` parameter, a call to a function
//! that does not exist, a read of a field the struct does not have, a non-exhaustive `Inspect`.
//! A language that advertises static types and accepts those is not doing the job its users think
//! it is doing — the error simply moves to run time, or to a wrong answer.
//!
//! THE TWO HALVES, AND WHY BOTH ARE HERE. Making a checker stricter is easy and worthless if it
//! also starts rejecting good programs, so this lock carries its own negative space: every real
//! program the project owns — the 687-program gauntlet corpus, which includes every benchmark —
//! must still be accepted. A strictness fix that breaks a working program fails this file just as
//! loudly as a missed rejection. That pairing is the whole point; neither half is safe alone.
//!
//! HOW IT STAYS HONEST. `WRONGLY_ACCEPTED` lists the programs the analysis does not yet catch, and
//! the assertion is an EXACT SET: a newly-missed program fails the lock, and a newly-caught one
//! fails it too, forcing the ledger to shrink deliberately rather than drift. The ledger is the
//! Wave 2 worklist, in priority order, and it should reach empty.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

#[path = "gauntlet/mod.rs"]
mod gauntlet;

/// One program that must not survive static analysis.
struct IllTyped {
    /// Ledger key — renaming one is a deliberate ledger edit.
    name: &'static str,
    src: &'static str,
    /// The specific thing that is wrong. This is what a reader needs to judge whether the program
    /// really is ill-typed, rather than taking the file's word for it.
    fault: &'static str,
}

/// Programs that a static type checker must reject. Every one of these is broken in a way that no
/// runtime input could rescue.
const ILL_TYPED: &[IllTyped] = &[
    IllTyped {
        name: "undefined_variable",
        src: "## Main\n    Let total be 5.\n    Show totl.\n",
        fault: "`totl` is a typo; no such binding exists",
    },
    IllTyped {
        name: "undefined_variable_in_arithmetic",
        src: "## Main\n    Let x be 1.\n    Show x + missing.\n",
        fault: "`missing` is never bound",
    },
    IllTyped {
        name: "argument_type_mismatch",
        src: "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\n    Show double(\"abc\").\n",
        fault: "a Text is passed where the parameter is declared Int",
    },
    IllTyped {
        name: "argument_type_mismatch_real_for_text",
        src: "## To greet (who: Text) -> Text:\n    Return who.\n\n## Main\n    Show greet(42).\n",
        fault: "an Int is passed where the parameter is declared Text",
    },
    IllTyped {
        name: "too_few_arguments",
        src: "## To add (a: Int, b: Int) -> Int:\n    Return a + b.\n\n## Main\n    Show add(1).\n",
        fault: "two parameters, one argument",
    },
    IllTyped {
        name: "too_many_arguments",
        src: "## To add (a: Int, b: Int) -> Int:\n    Return a + b.\n\n## Main\n    Show add(1, 2, 3).\n",
        fault: "two parameters, three arguments",
    },
    IllTyped {
        name: "unknown_function",
        src: "## Main\n    Show frobnicate(1).\n",
        fault: "no such function is declared anywhere, including the prelude",
    },
    IllTyped {
        name: "unknown_struct_field",
        src: "## A P has:\n    An x: Int.\n\n## Main\n    Let p be a new P with x 5.\n    Show p's z.\n",
        fault: "`P` has no field `z`",
    },
    IllTyped {
        name: "unknown_field_in_constructor",
        src: "## A P has:\n    An x: Int.\n\n## Main\n    Let p be a new P with z 5.\n    Show p's x.\n",
        fault: "the constructor names a field `P` does not have",
    },
    IllTyped {
        name: "struct_field_type_mismatch",
        src: "## A P has:\n    An x: Int.\n\n## Main\n    Let p be a new P with x \"text\".\n    Show p's x.\n",
        fault: "a Text is stored in a field declared Int",
    },
    IllTyped {
        name: "let_annotation_mismatch",
        src: "## Main\n    Let x: Int be \"hello\".\n    Show x.\n",
        fault: "the annotation says Int and the value is a Text",
    },
    IllTyped {
        name: "declared_return_type_mismatch",
        src: "## To f (n: Int) -> Int:\n    Return \"text\".\n\n## Main\n    Show f(1).\n",
        fault: "the body returns Text from a function declared `-> Int`",
    },
    IllTyped {
        name: "reassignment_changes_type",
        src: "## Main\n    Let mutable x be 1.\n    Set x to \"text\".\n    Show x.\n",
        fault: "an Int binding is reassigned to a Text",
    },
    IllTyped {
        name: "text_minus_text",
        src: "## Main\n    Show \"a\" - \"b\".\n",
        fault: "subtraction is not defined on Text",
    },
    IllTyped {
        name: "text_times_text",
        src: "## Main\n    Show \"a\" * \"b\".\n",
        fault: "multiplication is not defined on Text",
    },
    IllTyped {
        name: "bool_arithmetic",
        src: "## Main\n    Show true + 1.\n",
        fault: "a Bool is not a number",
    },
    IllTyped {
        name: "condition_is_not_boolean",
        src: "## Main\n    If 5:\n        Show 1.\n",
        fault: "an Int is used where a condition must be Bool",
    },
    IllTyped {
        name: "seq_element_type_mismatch",
        src: "## Main\n    Let mutable xs be a new Seq of Int.\n    Push \"text\" to xs.\n    Show xs.\n",
        fault: "a Text is pushed into a Seq of Int",
    },
    IllTyped {
        name: "map_value_type_mismatch",
        src: "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to \"text\".\n    Show m.\n",
        fault: "a Text is stored where the map's value type is Int",
    },
    IllTyped {
        name: "map_key_type_mismatch",
        src: "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item 1 of m to 2.\n    Show m.\n",
        fault: "an Int key is used where the map's key type is Text",
    },
    IllTyped {
        name: "field_access_on_a_number",
        src: "## Main\n    Let x be 5.\n    Show x's field.\n",
        fault: "an Int has no fields",
    },
    IllTyped {
        name: "calling_a_variable",
        src: "## Main\n    Let x be 5.\n    Show x(1).\n",
        fault: "`x` is an Int, not a function",
    },
    IllTyped {
        name: "unknown_type_annotation",
        src: "## Main\n    Let x: Bogus be 1.\n    Show x.\n",
        fault: "`Bogus` is not a declared type",
    },
    IllTyped {
        name: "unknown_parameter_type",
        src: "## To f (n: Bogus) -> Int:\n    Return 1.\n\n## Main\n    Show f(1).\n",
        fault: "a parameter is annotated with a type that does not exist",
    },
    IllTyped {
        name: "unknown_return_type",
        src: "## To f (n: Int) -> Bogus:\n    Return 1.\n\n## Main\n    Show f(1).\n",
        fault: "the return type does not exist",
    },
    IllTyped {
        name: "return_value_from_a_procedure",
        src: "## To p (n: Int):\n    Return n.\n\n## Main\n    p(1).\n",
        fault: "a procedure with no declared return type returns a value",
    },
    IllTyped {
        name: "unknown_function_in_a_function_body",
        src: "## To f (n: Int) -> Int:\n    Return nope(n).\n\n## Main\n    Show f(1).\n",
        fault: "the undefined call is nested inside a function rather than in Main",
    },
    IllTyped {
        name: "undefined_variable_inside_a_loop",
        src: "## Main\n    Let mutable i be 0.\n    While i is less than 3:\n        Show unbound.\n        Set i to i + 1.\n",
        fault: "the undefined name is inside a loop body, not at the top level",
    },
    IllTyped {
        name: "wrong_argument_type_through_a_variable",
        src: "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\n    Let s be \"abc\".\n    Show double(s).\n",
        fault: "the Text reaches the Int parameter through a binding rather than a literal",
    },
    IllTyped {
        name: "comparing_text_to_int",
        src: "## Main\n    If \"a\" is less than 1:\n        Show 1.\n",
        fault: "ordering a Text against an Int",
    },
    IllTyped {
        name: "seq_of_int_assigned_a_text_seq",
        src: "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable ys be a new Seq of Text.\n    Push \"a\" to ys.\n    Set xs to ys.\n    Show xs.\n",
        fault: "a Seq of Text is assigned to a Seq of Int binding",
    },
    IllTyped {
        name: "function_used_as_a_value_of_the_wrong_type",
        src: "## To f (n: Int) -> Int:\n    Return n.\n\n## Main\n    Let x: Text be f(1).\n    Show x.\n",
        fault: "an Int-returning call initialises a Text binding",
    },
];

/// Programs the static analysis WRONGLY ACCEPTS today. EXACT SET — a newly-missed program fails
/// this lock, and so does a newly-caught one that is still listed. This is the Wave 2 worklist.
///
/// ★ THE ROOT CAUSE, and it was structural rather than a missing rule. `infer_stmt` matched 19
/// statement kinds and ended in `_ => Ok(())`, so `Show`, `Push` and a statement-level call were
/// never type-checked AT ALL. Most of these programs put their error inside a `Show`, which the
/// checker skipped wholesale — `Show totl.`, `Show add(1).`, `Show double("abc").` all sailed
/// through untouched. Adding those three arms, plus checking arity BEFORE `zip` truncates it to
/// the shorter side (which is why `ArityMismatch` was unreachable from source), took the ledger
/// from 26 to 20 in one step and cost the negative space nothing.
///
/// ⚠ THE WILDCARD IS STILL THERE. `_ => Ok(())` remains, so every statement kind not named in
/// `infer_stmt` is still unchecked, and a NEW statement variant will silently join them. The
/// durable fix is an exhaustive, wildcard-free match — the same discipline the op catalogs use —
/// where each arm either infers its expressions or documents why it has none. That is the next
/// piece of this wave, and until it lands this ledger measures the symptom rather than the cause.
///
/// MEASURED, not assumed: 16 of the 32 cases below survive static analysis today. The rest split
/// into two severities, and the split is what should drive the order of work — they are NOT
/// equally bad:
///
///   ★ SILENT WRONG ANSWER — the program runs to completion and prints something. These are the
///     dangerous ones; no error ever appears, on any tier:
///       `unknown_field_in_constructor`   builds `P with z 5` and prints 0 for `x`
///       `argument_type_mismatch_real_for_text`  passes 42 to a Text parameter, prints 42
///       `condition_is_not_boolean`       `If 5:` takes the branch, prints 1
///       `struct_field_type_mismatch`     stores "text" in an Int field
///       `seq_element_type_mismatch`      pushes "text" into a Seq of Int
///       `seq_of_int_assigned_a_text_seq` assigns a Seq of Text to a Seq of Int
///       `map_value_type_mismatch`, `map_key_type_mismatch`   store the wrong types outright
///       `reassignment_changes_type`      an Int binding becomes a Text
///       `return_value_from_a_procedure`  the value vanishes
///
///   ▲ LATE ERROR — caught, but at run time rather than by `check`. Bad but not silent:
///     the undefined-variable trio, both arity cases, `unknown_struct_field`, `calling_a_variable`,
///     `field_access_on_a_number`, `comparing_text_to_int`, `bool_arithmetic`, `text_minus_text`,
///     `text_times_text`, `argument_type_mismatch`, `wrong_argument_type_through_a_variable`.
const WRONGLY_ACCEPTED: &[&str] = &[
    "argument_type_mismatch_real_for_text",
    "bool_arithmetic",
    "calling_a_variable",
    "comparing_text_to_int",
    "condition_is_not_boolean",
    "field_access_on_a_number",
    "reassignment_changes_type",
    "return_value_from_a_procedure",
    "seq_of_int_assigned_a_text_seq",
    "text_minus_text",
    "text_times_text",
    "undefined_variable",
    "undefined_variable_in_arithmetic",
    "undefined_variable_inside_a_loop",
];

/// Does the compiler's static analysis reject this program before anything runs? This is the front
/// half of `largo build` — escape analysis, the unknown-call pass, then `check_program` — so a
/// program that survives it reaches code generation.
fn rejected_statically(src: &str) -> bool {
    compile_to_rust(src).is_err()
}

#[test]
fn every_ill_typed_program_is_rejected_by_the_static_analysis() {
    let accepted: Vec<&IllTyped> = ILL_TYPED
        .iter()
        .filter(|p| !rejected_statically(p.src))
        .collect();

    let observed: std::collections::BTreeSet<&str> = accepted.iter().map(|p| p.name).collect();
    let declared: std::collections::BTreeSet<&str> = WRONGLY_ACCEPTED.iter().copied().collect();

    let newly_missed: Vec<&str> = observed.difference(&declared).copied().collect();
    let newly_caught: Vec<&str> = declared.difference(&observed).copied().collect();

    if newly_missed.is_empty() && newly_caught.is_empty() {
        return;
    }

    let mut msg = String::from("\nThe static analysis' rejection set changed.\n");
    if !newly_missed.is_empty() {
        msg.push_str(
            "\n  NOT REJECTED — these broken programs now pass static analysis. Fix the checker;\n  \
             listing a program here is an admission, not a resolution.\n",
        );
        for name in &newly_missed {
            let p = ILL_TYPED.iter().find(|p| p.name == *name).unwrap();
            // Say what the program does INSTEAD of being rejected: a wrong answer at run time is a
            // far worse outcome than a late error, and the two need different fixes.
            let tw = tw_outcome(p.src);
            let fate = match tw.error {
                Some(e) => format!("errors at RUN time: {}", e.trim()),
                None => format!("RUNS and prints: {}", tw.output.trim()),
            };
            msg.push_str(&format!(
                "\n    {}  ({})\n      it {fate}\n",
                p.name, p.fault
            ));
        }
    }
    if !newly_caught.is_empty() {
        msg.push_str("\n  NOW REJECTED — delete these from WRONGLY_ACCEPTED; the ledger must match exactly.\n");
        for name in &newly_caught {
            msg.push_str(&format!("    {name}\n"));
        }
    }
    panic!("{msg}");
}

/// THE NEGATIVE SPACE. Every real program the project owns must still be accepted. Without this,
/// any of the fixes above could be "achieved" by rejecting more aggressively, and the checker would
/// become useless in the opposite direction — the failure mode that makes people disable a linter.
#[test]
fn no_real_program_is_rejected_by_the_static_analysis() {
    // The gauntlet's own oracle decides which programs are meant to work: a program the VM runs
    // cleanly is a program the checker has no business refusing.
    let corpus: Vec<gauntlet::GauntletProg> = gauntlet::all()
        .into_iter()
        .filter(|p| !p.expect_err && p.oracle == "plain")
        .collect();
    assert!(
        corpus.len() >= 400,
        "expected the real corpus (harvest + benchmarks + compute), only got {} — is gauntlet/corpus.jsonl present?",
        corpus.len()
    );

    let rejected: Vec<&gauntlet::GauntletProg> = corpus
        .iter()
        .filter(|p| {
            // Only a STATIC rejection counts as a false positive here. A program the VM oracle
            // cannot run either is not evidence about the checker.
            rejected_statically(&p.src)
        })
        .collect();

    assert!(
        rejected.is_empty(),
        "\nThe static analysis now REJECTS {} program(s) that the project ships and the VM runs \
         cleanly.\nA strictness fix that breaks working code is a regression, not progress — \
         narrow the rule.\n\n{}",
        rejected.len(),
        rejected
            .iter()
            .take(10)
            .map(|p| {
                let why = compile_to_rust(&p.src)
                    .err()
                    .map(|e| format!("{e:?}"))
                    .unwrap_or_default();
                format!(
                    "    {}  ({})\n      {}",
                    p.name,
                    p.op,
                    why.chars().take(200).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The ill-typed corpus cannot silently shrink: deleting a case is the easiest way to make the
/// rejection test pass without fixing anything.
#[test]
fn the_ill_typed_corpus_only_grows() {
    const FLOOR: usize = 32;
    assert!(
        ILL_TYPED.len() >= FLOOR,
        "the ill-typed corpus fell to {} (floor {FLOOR}). Add cases, never remove them.",
        ILL_TYPED.len()
    );
    let mut names: Vec<&str> = ILL_TYPED.iter().map(|p| p.name).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        before,
        "ill-typed program names are ledger keys and must be unique"
    );
}
