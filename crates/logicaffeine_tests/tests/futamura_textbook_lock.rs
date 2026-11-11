//! LOCK — the Futamura projections as Futamura defined them: built, executed, Jones-optimal.
//!
//! A residual is a program. It must do what the source does — every output, in order — and only then does its
//! dispatch count mean anything: a residual that drops a `Show` scores zero dispatch and is wrong. This file holds the
//! specializer to that before it holds it to optimality, and its constants only rise.
//!
//! §1 A specialization keeps every effect. Unfolding a call replaces it by what the callee's specialized body
//! computes; when that body does anything besides return a value — shows, loops, binds names of its own — those
//! statements belong to the residual too, in source order, exactly once per call, with the callee's names kept apart
//! from the caller's. A memoized call repeats its effects. An effect a short-circuit or a loop condition guards stays
//! guarded. Each program runs on the tree-walker (the semantic ground truth) and as its first-projection residual —
//! the direct specialization's and the textbook first projection's; they must print the same, and the residual must
//! carry no interpreter dispatch.
//!
//! §2 A residual is a program of the language. An `Inspect` the specializer cannot decide stays an `Inspect` — the
//! construct the language runs — with every arm binding what the pattern wrote, every statement after it kept, and
//! every variable an arm changes isolated from the other arms. One that it can decide leaves the chosen arm and the
//! statements that follow it. A collection changed under an undecided branch is not assumed changed after it, and a
//! copy of a collection does not see later changes to the original.
//!
//! §3 The first projection as Futamura defined it. The specializer specializes the self-interpreter
//! (`core_interp.logos`) to a program whose encoding is a constant: the interpreter's dispatch on statements and
//! expressions, its environment, its value boxes and its loops over the program are all static, so none of them may
//! reach the residual. What remains is the program's own computation, and it prints what the program prints.
//!
//! §4 The self-interpreter is the language. The first projection means `[[mix(sint, p)]] = [[p]]` only when
//! `[[sint]](p) = [[p]]`, so the interpreter the projections specialize must run every program as the tree-walker does:
//! under mutable value semantics a binding, an element read and a by-value argument are copies, a `mutable` parameter
//! is the caller's variable, a closure captures values; it prints every value as the language prints it; a return or a
//! break is control flow, never a value a program could also produce. It must mean that on either host semantics —
//! run inside the reference scope the compiler's own programs use, and run as an ordinary program — so its meaning
//! never borrows the host's aliasing. The textbook first projection and the direct specialization of each program must
//! then run it as the language runs it, with no interpreter dispatch left.
//!
//! §5 The specializer is a program of the language. Self-application specializes the specializer as the language runs
//! it, so it must mean on the language's own value semantics what it means in the reference scope its production runs
//! use: it threads its state through `mutable` parameters, and never reaches it through an alias. Every program of the
//! corpora above specializes to the same residual on both hosts, directly and through the self-interpreter.
//!
//! §6 A value the specializer knows keeps its meaning. A record, a list or a tuple the specializer holds as static
//! while a part of it is a variable's run-time value means that value as it was when the aggregate was made. A later
//! write to the variable — a `Set`, a write in an unrolled loop, under an undecided branch, or through a `mutable`
//! argument — leaves the aggregate as it was: the residual binds the aggregate before the write, and every later read
//! of it, a folded field included, sees what the source sees.
//!
//! §7 The offline specializer. The projections that apply the specializer to itself do so with `mix`
//! (`optimize/mix.logos`), whose every decision is a binding-time annotation the analysis (`optimize/mix_bta.rs`)
//! fixed before it runs, so it terminates in every self-application. Each projection is one application,
//! `[[mix]](program², store)`: a two-level program and the store binding its static inputs. The first,
//! `[[mix]](sint², p)`, holds every corpus above: what the program prints, no interpreter dispatch.
//!
//! §8 The second projection as Futamura defined it. The specializer, annotated with the program it specializes static
//! and its store dynamic (`mix²`), specializes itself to the self-interpreter: `compiler = [[mix]](mix², sint²)`. The
//! compiler is executed: given a program's Core IR it prints the program's first projection, `[[compiler]](p) =
//! [[mix]](sint², p)`, byte for byte, so every property §7 holds that residual to holds for what it compiles. And it is
//! a compiler, not the specializer carrying the self-interpreter: no two-level construct reaches it.
//!
//! §9 The third projection as Futamura defined it. The specializer specializes itself to itself: `cogen = [[mix]](mix²,
//! mix²)`. The compiler generator is executed on the self-interpreter's two-level program and prints the compiler the
//! second projection made, `[[cogen]](sint²) = [[mix]](mix², sint²)`, byte for byte.
//!
//! §10 A failure is the language's. A program that stops — a failing index, a division by zero, `Fail with`, a broken
//! contract, a name read after the block that bound it ends — shows what it showed before it stopped and ends with the
//! language's own report, in the language's words. The self-interpreter stops where the program stops: a failure is
//! control flow that leaves every block, loop and call, never a value a program could show or compute with. Each first
//! projection then stops where the program stops: its residual shows what the program showed and ends with the same
//! report, with no interpreter dispatch left.
//!
//! §11 The projections are programs of the language. The compiler and the compiler generator compile to native code as
//! any program does, each built once and handed its input as data, and run there as they run on the tree-walker: the
//! native compiler compiles every corpus program to exactly its first projection, and the native compiler generator
//! generates exactly the second projection's compiler.

mod common;
#[path = "common/generated_programs.rs"]
mod generated_programs;
mod pe_support;

use pe_support::*;

/// One corpus program and what the tree-walker prints for it.
struct Case {
    name: &'static str,
    program: &'static str,
    expected: &'static str,
}

const fn case(name: &'static str, program: &'static str, expected: &'static str) -> Case {
    Case {
        name,
        program,
        expected,
    }
}

/// Programs whose calls carry effects. A value of `n` computed by a loop longer than the specializer unrolls
/// (64 passes) is unknown when it specializes, which drives the mixed static/dynamic call paths.
const EFFECT_CORPUS: &[Case] = &[
    case(
        "effect_before_the_return",
        "## To loud (n: Int) -> Int:\n    Show n.\n    Return n + 1.\n\n## Main\nLet x be loud(1).\nShow x.",
        "1\n2",
    ),
    case(
        "a_repeated_call_repeats_its_effect",
        "## To loud (n: Int) -> Int:\n    Show n.\n    Return n + 1.\n\n## Main\nLet a be loud(1).\nLet b be loud(1).\nShow a + b.",
        "1\n1\n4",
    ),
    case(
        "an_effect_two_calls_deep",
        "## To inner (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## To outer (n: Int) -> Int:\n    Return inner(n) + 1.\n\n## Main\nShow outer(5).",
        "5\n6",
    ),
    case(
        "a_call_inside_show",
        "## To loud (n: Int) -> Int:\n    Show n.\n    Return n + 1.\n\n## Main\nShow loud(2).",
        "2\n3",
    ),
    case(
        "two_calls_in_one_expression_keep_their_order",
        "## To loud (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nShow loud(1) + loud(2).",
        "1\n2\n3",
    ),
    case(
        "a_loop_in_the_callee",
        "## To countDown (n: Int) -> Int:\n    Let mutable k be n.\n    While k is greater than 0:\n        Show k.\n        Set k to k - 1.\n    Return 0.\n\n## Main\nLet z be countDown(3).\nShow z.",
        "3\n2\n1\n0",
    ),
    case(
        "the_callees_names_stay_its_own",
        "## To loud (n: Int) -> Int:\n    Let x be n * 10.\n    Show x.\n    Return x + 1.\n\n## Main\nLet x be 5.\nLet y be loud(1).\nShow x.\nShow y.",
        "10\n5\n11",
    ),
    case(
        "a_loop_condition_runs_its_call_every_pass",
        "## To tick (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet mutable i be 0.\nWhile tick(i) is less than 3:\n    Set i to i + 1.\nShow \"done\".",
        "0\n1\n2\n3\ndone",
    ),
    case(
        "a_mixed_call_keeps_its_effect_and_its_argument",
        "## To loudAdd (k: Int) and (d: Int) -> Int:\n    Show k.\n    Return k + d.\n\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 100:\n    Set n to n + 1.\nShow loudAdd(7, n).",
        "7\n107",
    ),
    case(
        "a_repeated_mixed_call_repeats_its_effect",
        "## To loudAdd (k: Int) and (d: Int) -> Int:\n    Show k.\n    Return k + d.\n\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 100:\n    Set n to n + 1.\nLet a be loudAdd(7, n).\nLet b be loudAdd(7, n).\nShow a + b.",
        "7\n7\n214",
    ),
    case(
        "a_short_circuit_that_skips_the_call",
        "## To loud (n: Int) -> Bool:\n    Show n.\n    Return true.\n\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 100:\n    Set n to n + 1.\nIf n is greater than 1000 and loud(1):\n    Show \"yes\".\nShow \"end\".",
        "end",
    ),
    case(
        "a_short_circuit_that_takes_the_call",
        "## To loud (n: Int) -> Bool:\n    Show n.\n    Return true.\n\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 100:\n    Set n to n + 1.\nIf n is greater than 10 and loud(1):\n    Show \"yes\".\nShow \"end\".",
        "1\nyes\nend",
    ),
    case(
        "a_call_inside_a_range_loop_keeps_its_function",
        "## To fact (k: Int) -> Int:\n    If k is at most 1:\n        Return 1.\n    Return k * fact(k - 1).\n\n## Main\nLet n be parseInt(\"5\").\nRepeat for i from 1 to n:\n    Show fact(i).",
        "1\n2\n6\n24\n120",
    ),
];

/// Effect programs the first projection specializes soundly and dispatch-free. May only rise.
const EFFECT_PROGRAMS: usize = 13;

#[test]
fn a_specialization_keeps_every_effect() {
    assert_corpus("effect", EFFECT_CORPUS, EFFECT_PROGRAMS, decompile);
}

#[test]
fn the_textbook_first_projection_keeps_every_effect() {
    assert_corpus(
        "effect (textbook first projection)",
        EFFECT_CORPUS,
        EFFECT_PROGRAMS,
        logicaffeine_compile::compile::textbook_projection1,
    );
}

const INSPECT_CORPUS: &[Case] = &[
    case(
        "an_undecided_inspect_keeps_the_statements_after_it",
        concat!(
            "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## To pick (k: Int) -> Shape:\n    If k is greater than 50:\n        Return a new Circle with radius k.\n    Return a new Square with side k.\n\n",
            "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet s be pick(k).\nInspect s:\n    When Circle (r):\n        Show r.\n    When Square (side):\n        Show side * side.\nShow \"after\".",
        ),
        "100\nafter",
    ),
    case(
        "a_decided_inspect_keeps_the_statements_after_it",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## Main\nLet s be a new Circle with radius 5.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Square (side):\n        Show side.\nShow \"after\".",
        "5\nafter",
    ),
    case(
        "an_otherwise_arm_changes_a_variable_the_other_arms_do_not_see",
        concat!(
            "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## To pick (k: Int) -> Shape:\n    If k is greater than 50:\n        Return a new Circle with radius k.\n    Return a new Square with side k.\n\n",
            "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet s be pick(k).\nLet mutable total be 0.\nInspect s:\n    When Circle (r):\n        Set total to r.\n    Otherwise:\n        Set total to 7.\nShow total.",
        ),
        "100",
    ),
    case(
        "variants_without_fields",
        "## A Color is one of:\n    A Red.\n    A Green.\n\n## To pickColor (k: Int) -> Color:\n    If k is greater than 50:\n        Return a new Green.\n    Return a new Red.\n\n## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet c be pickColor(k).\nInspect c:\n    When Red:\n        Show \"red\".\n    When Green:\n        Show \"green\".\nShow \"end\".",
        "green\nend",
    ),
    case(
        "an_inspect_inside_a_loop_with_a_return_in_an_arm",
        "## A Val is one of:\n    A VB with b Bool.\n    A VN.\n\n## To g (v: Val) -> Int:\n    Let mutable loopDone be false.\n    Let mutable count be 0.\n    While not loopDone:\n        Inspect v:\n            When VB (b):\n                If not b:\n                    Set loopDone to true.\n                Otherwise:\n                    Set count to count + 1.\n                    If count is greater than 3:\n                        Return count.\n            Otherwise:\n                Set loopDone to true.\n    Return 0.\n\n## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet v be a new VB with b (k is greater than 50).\nShow g(v).",
        "4",
    ),
    case(
        "a_map_changed_in_an_undecided_branch",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet m be a new Map of Text to Int.\nIf k is greater than 1000:\n    Set item \"a\" of m to 1.\nShow length of m.",
        "0",
    ),
    case(
        "a_copy_does_not_see_later_pushes",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet mutable xs be [1, 2].\nLet ys be xs.\nIf k is greater than 1000:\n    Push 3 to xs.\nPush 4 to xs.\nShow length of ys.\nShow length of xs.",
        "2\n3",
    ),
];

/// Programs whose residual must run as the language runs them. May only rise.
const INSPECT_PROGRAMS: usize = 7;

#[test]
fn the_textbook_first_projection_runs_as_the_language_runs() {
    assert_corpus(
        "inspect (textbook first projection)",
        INSPECT_CORPUS,
        INSPECT_PROGRAMS,
        logicaffeine_compile::compile::textbook_projection1,
    );
}

/// Closed programs the first projection, as Futamura defined it, turns into residuals with nothing of the interpreter
/// left in them.
const TEXTBOOK_P1_CORPUS: &[Case] = &[
    case("arith", "## Main\nShow 2 + 3 * 4.", "14"),
    case(
        "if_else",
        "## Main\nLet x be 7.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".",
        "big",
    ),
    case(
        "while_loop",
        "## Main\nLet mutable i be 0.\nWhile i is less than 5:\n    Set i to i + 1.\nShow i.",
        "5",
    ),
    case(
        "list_ops",
        "## Main\nLet mutable xs be [1, 2, 3].\nPush 4 to xs.\nShow length of xs.\nShow item 2 of xs.",
        "4\n2",
    ),
    case(
        "map_ops",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 3.\nSet item \"b\" of m to 4.\nShow item \"a\" of m + item \"b\" of m.",
        "7",
    ),
    case("interpolation", "## Main\nLet n be 7.\nShow \"n is {n}\".", "n is 7"),
    case(
        "repeat_sum",
        "## Main\nLet mutable total be 0.\nRepeat for x in [1, 2, 3, 4]:\n    Set total to total + x.\nShow total.",
        "10",
    ),
    case("closure", "## Main\nLet f be (n: Int) -> n * 2.\nShow f(21).", "42"),
    case(
        "enum_inspect",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## Main\nLet s be a new Square with side 4.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Square (k):\n        Show k * k.\nShow \"done\".",
        "16\ndone",
    ),
    case(
        "factorial",
        "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).",
        "120",
    ),
    case(
        "an_empty_sequence_argument_and_an_early_return",
        "## To firstOver (xs: Seq of Int) and (t: Int) -> Int:\n    Repeat for x in xs:\n        If x is greater than t:\n            Return x.\n    Return 0.\n\n## Main\nShow firstOver(a new Seq of Int, 3).",
        "0",
    ),
    case("an_index_into_a_known_text", "## Main\nShow item 2 of \"abc\".", "b"),
];

/// Closed programs whose textbook first projection is Jones-optimal and correct. May only rise.
const TEXTBOOK_P1_PROGRAMS: usize = 12;

/// Every case of `corpus` whose first-projection residual (`residual_of`) prints something other than the tree-walker
/// prints for the source, or keeps interpreter dispatch.
fn specialization_failures(corpus: &[Case], residual_of: fn(&str) -> Result<String, String>) -> Vec<String> {
    let mut failures = Vec::new();
    for c in corpus {
        let residual = match residual_of(c.program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("`{}`: the projection failed: {e}", c.name));
                continue;
            }
        };
        let tw = run_treewalk(c.program);
        let ran = run_treewalk(&residual);
        if let Some(diff) = behavior_diff(&tw, &ran, CmpMode::Strict) {
            failures.push(format!(
                "`{}`: the residual does not do what the source does — {diff}\n--- residual ---\n{residual}",
                c.name
            ));
            continue;
        }
        let d = count_dispatch(&residual);
        if d != 0 {
            failures.push(format!(
                "`{}`: the residual keeps {d} unit(s) of interpreter dispatch\n--- residual ---\n{residual}",
                c.name
            ));
        }
    }
    failures
}

fn assert_corpus(label: &str, corpus: &[Case], floor: usize, residual_of: fn(&str) -> Result<String, String>) {
    assert!(
        corpus.len() >= floor,
        "the {label} corpus shrank to {} below its floor {floor}",
        corpus.len()
    );
    for c in corpus {
        let tw = run_treewalk(c.program);
        assert_eq!(
            tw.output, c.expected,
            "`{}`: the tree-walker prints something else — the corpus entry is wrong, not the specializer: {:?}",
            c.name, tw
        );
    }
    let failures = specialization_failures(corpus, residual_of);
    assert!(
        failures.is_empty(),
        "{} of {} {label} programs are specialized wrongly:\n{}",
        failures.len(),
        corpus.len(),
        failures.join("\n═══\n")
    );
}

#[test]
fn a_residual_runs_as_the_language_runs() {
    assert_corpus("inspect", INSPECT_CORPUS, INSPECT_PROGRAMS, decompile);
}

#[test]
fn the_textbook_first_projection_is_jones_optimal() {
    assert_corpus(
        "textbook first-projection",
        TEXTBOOK_P1_CORPUS,
        TEXTBOOK_P1_PROGRAMS,
        logicaffeine_compile::compile::textbook_projection1,
    );
}

/// Programs whose meaning is the language's own: value semantics, `mutable` parameters, how each value prints, how
/// collections iterate, and control flow a value could imitate.
const SEMANTICS_CORPUS: &[Case] = &[
    case(
        "a_standard_library_function_runs",
        "## Main\nShow gcd(12, 18).\nShow lcm(4, 6).\nShow factorial(5).\nShow clamp(15, 0, 10).",
        "6\n12\n120\n10",
    ),
    case(
        "a_standard_library_text_function_runs",
        "## Main\nShow upper(\"hi\").\nShow lower(\"HI\").",
        "HI\nhi",
    ),
    case(
        "operators_power_and_floor",
        "## Main\nShow 2 ** 10.\nShow 7 // 2.\nShow (0 - 7) // 2.\nShow 7.5 // 2.\nShow (2.0 ** 0.5) is approximately 1.4142135623730951.\nShow 9 % 4.",
        "1024\n3\n-4\n3\ntrue\n1",
    ),
    case(
        "operators_join_text_and_lists",
        "## Main\nShow 1 combined with 2.\nShow \"x\" + 2.5.\nShow 2.5 + \"x\".\nShow true combined with 3.\nShow [1, 2] followed by [3].\nShow [1, 2] + [3].\nShow 2 * [1, 2].",
        "12\nx2.5\n2.5x\ntrue3\n[1, 2, 3]\n[1, 2, 3]\n[1, 2, 1, 2]",
    ),
    case(
        "operators_truth_and_bits",
        "## Main\nShow 0 and true.\nShow 1 or false.\nShow 5 xor 3.\nShow true xor false.\nShow 6 & 3.\nShow 6 | 3.\nShow 3 shifted left by 2.",
        "false\ntrue\n6\ntrue\n2\n7\n12",
    ),
    case(
        "operators_on_sets",
        "## Main\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nAdd 2 to a.\nLet mutable b be a new Set of Int.\nAdd 2 to b.\nAdd 3 to b.\nShow a | b.\nShow a & b.\nShow a ^ b.\nShow a - b.",
        "{1, 2, 3}\n{2}\n{1, 3}\n{1}",
    ),
    case(
        "a_copy_by_let_is_its_own",
        "## Main\nLet mutable xs be [1, 2].\nLet ys be xs.\nPush 4 to xs.\nShow length of ys.\nShow length of xs.",
        "2\n3",
    ),
    case(
        "an_element_write_does_not_reach_the_copy",
        "## Main\nLet mutable xs be [1, 2].\nLet ys be xs.\nSet item 1 of xs to 9.\nShow item 1 of ys.\nShow item 1 of xs.",
        "1\n9",
    ),
    case(
        "a_map_copy_is_its_own",
        "## Main\nLet mutable m be a new Map of Text to Int.\nLet n be m.\nSet item \"a\" of m to 1.\nShow length of n.\nShow length of m.",
        "0\n1",
    ),
    case(
        "a_record_copy_is_its_own",
        "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable p be a new Point with x 1.\nLet q be p.\nSet p's x to 5.\nShow q's x.\nShow p's x.",
        "1\n5",
    ),
    case(
        "a_by_value_parameter_is_the_callees_own",
        "## To f (xs: Seq of Int) -> Int:\n    Push 1 to xs.\n    Return length of xs.\n\n## Main\nLet ys be [5, 6].\nShow f(ys).\nShow length of ys.",
        "3\n2",
    ),
    case(
        "a_by_value_map_parameter_is_the_callees_own",
        "## To g (m: Map of Text to Int) -> Int:\n    Set item \"a\" of m to 1.\n    Return length of m.\n\n## Main\nLet mm be a new Map of Text to Int.\nShow g(mm).\nShow length of mm.",
        "1\n0",
    ),
    case(
        "a_mutable_parameter_is_the_callers_variable",
        "## To fill (xs: mutable Seq of Int) -> Int:\n    Push 4 to xs.\n    Return length of xs.\n\n## Main\nLet mutable a be [1, 2, 3].\nShow fill(a).\nShow length of a.",
        "4\n4",
    ),
    case(
        "a_mutable_parameter_called_twice",
        "## To g (xs: mutable Seq of Int):\n    Push 9 to xs.\n\n## Main\nLet mutable a be [1].\ng(a).\ng(a).\nShow length of a.",
        "3",
    ),
    case(
        "a_mutable_parameter_passed_on",
        "## To a1 (xs: mutable Seq of Int):\n    Push 1 to xs.\n\n## To a2 (xs: mutable Seq of Int):\n    a1(xs).\n    a1(xs).\n\n## Main\nLet mutable a be a new Seq of Int.\na2(a).\nShow length of a.",
        "2",
    ),
    case(
        "a_mutable_map_parameter",
        "## To put (m: mutable Map of Text to Int) and (k: Text):\n    Set item k of m to 1.\n\n## Main\nLet mutable m be a new Map of Text to Int.\nput(m, \"a\").\nput(m, \"b\").\nShow length of m.",
        "2",
    ),
    case(
        "a_mutable_record_parameter",
        "## A Point has:\n    An x: Int.\n\n## To bump (p: mutable Point):\n    Set p's x to p's x + 1.\n\n## Main\nLet mutable p be a new Point with x 1.\nbump(p).\nbump(p).\nShow p's x.",
        "3",
    ),
    case(
        "a_mutable_parameter_with_an_early_return",
        "## To grow (xs: mutable Seq of Int) and (n: Int) -> Int:\n    Repeat for i from 1 to n:\n        Push i to xs.\n        If length of xs is greater than 4:\n            Return i.\n    Return 0.\n\n## Main\nLet mutable a be a new Seq of Int.\nShow grow(a, 6).\nShow length of a.",
        "5\n5",
    ),
    case(
        "an_element_read_is_a_copy",
        "## Main\nLet mutable grid be [[1, 2], [3]].\nLet mutable row be item 1 of grid.\nPush 5 to row.\nShow length of item 1 of grid.\nShow length of row.",
        "2\n3",
    ),
    case(
        "a_record_element_read_is_a_copy",
        "## A P has:\n    An x: Int.\n\n## Main\nLet mutable ps be a new Seq of P.\nPush a new P with x 1 to ps.\nLet mutable q be item 1 of ps.\nSet q's x to 9.\nShow (item 1 of ps)'s x.\nShow q's x.",
        "1\n9",
    ),
    case(
        "a_copy_of_an_empty_argument",
        "## To collect (acc: Seq of Int) and (k: Int) -> Int:\n    Let mutable out be copy of acc.\n    Repeat for i from 1 to k:\n        Push i to out.\n        If i equals 2:\n            Return length of out.\n    Return 0.\n\n## Main\nShow collect(a new Seq of Int, 5).",
        "2",
    ),
    case(
        "a_closure_captures_by_value",
        "## Main\nLet mutable xs be [1, 2].\nLet f be (n: Int) -> n + length of xs.\nPush 3 to xs.\nShow f(0).\nShow length of xs.",
        "2\n3",
    ),
    case(
        "a_snapshot_per_pass",
        "## Main\nLet mutable xs be [1].\nLet mutable total be 0.\nRepeat for i from 1 to 3:\n    Let snap be xs.\n    Push i to xs.\n    Set total to total + length of snap.\nShow total.",
        "6",
    ),
    case(
        "show_sequences",
        "## Main\nShow [1, 2, 3].\nShow [\"a\", \"b\"].\nShow [[1, 2], [3]].",
        "[1, 2, 3]\n[a, b]\n[[1, 2], [3]]",
    ),
    case("show_an_empty_sequence", "## Main\nLet xs be a new Seq of Int.\nShow xs.", "[]"),
    case(
        "show_a_map_in_insertion_order",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"b\" of m to 2.\nSet item \"a\" of m to 1.\nShow m.",
        "{b: 2, a: 1}",
    ),
    case(
        "show_a_set",
        "## Main\nLet mutable s be a new Set of Int.\nAdd 3 to s.\nAdd 1 to s.\nShow s.",
        "{3, 1}",
    ),
    case(
        "show_a_record_in_declaration_order",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with y 2 and x 1.\nShow p.",
        "Point { x: 1, y: 2 }",
    ),
    case(
        "show_variants",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nShow a new Circle with radius 3.\nShow a new Dot.",
        "Circle(3)\nDot",
    ),
    case("show_options", "## Main\nLet o be some 5.\nShow o.\nLet n be none.\nShow n.", "5\nnothing"),
    case("show_a_tuple", "## Main\nLet t be (1, \"x\").\nShow t.", "(1, x)"),
    case("show_floats", "## Main\nShow 1.5.\nShow 2.0.\nShow 10.0 / 4.0.", "1.5\n2\n2.5"),
    case(
        "a_maps_length_and_keys",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow length of m.\nShow m contains \"a\".\nShow m contains \"z\".",
        "1\ntrue\nfalse",
    ),
    case(
        "iterate_a_map",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"b\" of m to 2.\nSet item \"a\" of m to 1.\nRepeat for k in m:\n    Show k.",
        "(b, 2)\n(a, 1)",
    ),
    case(
        "iterate_a_set",
        "## Main\nLet mutable s be a new Set of Int.\nAdd 3 to s.\nAdd 1 to s.\nAdd 3 to s.\nRepeat for x in s:\n    Show x.\nShow length of s.",
        "3\n1\n2",
    ),
    case("iterate_text", "## Main\nRepeat for c in \"ab\":\n    Show c.", "a\nb"),
    case(
        "a_map_literal",
        "## Main\nLet m be {\"a\": 1, \"b\": 2}.\nShow item \"b\" of m.\nShow length of m.",
        "2\n2",
    ),
    case(
        "a_returned_text_is_not_control_flow",
        "## To f (n: Int) -> Text:\n    Repeat for i from 1 to n:\n        If i equals 2:\n            Return \"__break__\".\n    Return \"none\".\n\n## Main\nShow f(3).",
        "__break__",
    ),
    case(
        "a_return_from_a_nested_loop",
        "## To f (xs: Seq of Int) -> Int:\n    Repeat for x in xs:\n        Repeat for y in xs:\n            If x + y equals 5:\n                Return x * y.\n    Return 0.\n\n## Main\nShow f([1, 2, 3, 4]).",
        "4",
    ),
    case(
        "break_ends_the_loop",
        "## Main\nLet mutable i be 0.\nWhile true:\n    Set i to i + 1.\n    If i equals 3:\n        Break.\nShow i.",
        "3",
    ),
    case(
        "a_block_binding_ends_with_its_block",
        "## Main\nLet x be 1.\nIf true:\n    Let y be 2.\n    Show x + y.\nShow x.",
        "3\n1",
    ),
    case(
        "an_inner_binding_shadows_until_its_block_ends",
        "## Main\nLet x be 1.\nIf true:\n    Let x be 2.\n    Show x.\nShow x.",
        "2\n1",
    ),
    case(
        "a_loop_bodys_binding_shadows_each_pass",
        "## Main\nLet x be 1.\nRepeat for i from 1 to 2:\n    Let x be i * 10.\n    Show x.\nShow x.",
        "10\n20\n1",
    ),
    case(
        "a_tuple_pattern_binds_every_name",
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"b\" of m to 2.\nSet item \"a\" of m to 1.\nRepeat for (k, v) in m:\n    Show k.\n    Show v.",
        "b\n2\na\n1",
    ),
    case(
        "a_function_reads_a_program_binding",
        "## To peek -> Int:\n    Return g.\n\n## Main\nLet g be 5.\nShow peek().",
        "5",
    ),
    case(
        "a_function_writes_a_program_binding",
        "## To bump:\n    Set g to g + 1.\n\n## Main\nLet mutable g be 5.\nbump().\nbump().\nShow g.",
        "7",
    ),
    case(
        "a_functions_own_binding_shadows_the_programs",
        "## To peek -> Int:\n    Let g be 7.\n    Return g.\n\n## Main\nLet g be 5.\nShow peek().\nShow g.",
        "7\n5",
    ),
    case(
        "a_decided_break_keeps_what_follows_the_loop",
        "## Main\nRepeat for x in [1, 2, 3]:\n    If x equals 2:\n        Break.\n    Show x.\nShow \"after\".",
        "1\nafter",
    ),
    case(
        "an_undecided_break_in_a_range",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nRepeat for x from 1 to 3:\n    If x is greater than k - 99:\n        Break.\n    Show x.\nShow \"after\".",
        "1\nafter",
    ),
    case(
        "an_undecided_break_in_a_while",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet mutable x be 0.\nWhile x is less than 3:\n    Set x to x + 1.\n    If x is greater than k - 99:\n        Break.\n    Show x.\nShow \"after\".",
        "1\nafter",
    ),
    case(
        "an_omitted_field_starts_at_its_types_default",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n    A tag: Text.\n\n## Main\nLet p be a new Point with x 1.\nShow p's y + 5.\nShow length of p's tag.",
        "5\n0",
    ),
    case("literal_braces_in_text", "## Main\nShow \"a {{b}} c\".", "a {b} c"),
    case(
        "a_text_holding_a_brace",
        "## Main\nLet k be \"{{\".\nShow length of k.\nShow k.",
        "1\n{",
    ),
    case(
        "quotes_inside_an_interpolation",
        "## Main\nLet t be \"x\".\nShow \"say \\\"{t}\\\" now\".",
        "say \"x\" now",
    ),
    case(
        "a_write_before_a_break_outlives_the_loop",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nLet mutable ys be a new Seq of Int.\nPush k to ys.\nPush 7 to ys.\nLet mutable found be false.\nRepeat for x in ys:\n    If x equals 100:\n        Set found to true.\n        Break.\nShow found.",
        "true",
    ),
    case(
        "a_record_field_write",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet mutable p be a new Point with x 1 and y 2.\nSet p's x to p's x + 10.\nShow p.\nShow p's x.",
        "Point { x: 11, y: 2 }\n11",
    ),
    case(
        "a_records_fields_act_in_written_order_and_print_by_name",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## To loud (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet p be a new Point with y loud(2) and x loud(1).\nShow p.",
        "2\n1\nPoint { x: 1, y: 2 }",
    ),
    case(
        "a_variants_fields_bind_in_declared_order",
        "## A Shape is one of:\n    A Rect with w Int and h Int.\n\n## Main\nLet r be a new Rect with h 2 and w 1.\nShow r.\nInspect r:\n    When Rect (a, b):\n        Show a.\n        Show b.",
        "Rect(1, 2)\n1\n2",
    ),
    case(
        "a_variants_fields_act_in_written_order",
        "## A Shape is one of:\n    A Rect with w Int and h Int.\n\n## To loud (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## Main\nLet r be a new Rect with h loud(2) and w loud(1).\nShow r.\nInspect r:\n    When Rect (a, b):\n        Show a.\n        Show b.",
        "2\n1\nRect(1, 2)\n1\n2",
    ),
    case(
        "a_decided_and_or_or_never_runs_its_right_side",
        "## To loud (n: Int) -> Bool:\n    Show n.\n    Return true.\n\n## Main\nLet mutable n be 0.\nRepeat for i from 1 to 100:\n    Set n to n + 1.\nIf n is greater than 1000 and loud(1):\n    Show \"and\".\nIf n is less than 1000 or loud(2):\n    Show \"or\".\nShow \"end\".",
        "or\nend",
    ),
    case(
        "an_undecided_break_over_a_list",
        "## Main\nLet mutable k be 0.\nRepeat for i from 1 to 100:\n    Set k to k + 1.\nRepeat for x in [1, 2, 3]:\n    If x + 98 equals k:\n        Break.\n    Show x.\nShow \"after\".",
        "1\nafter",
    ),
    case(
        "a_builtin_comes_before_a_closure_of_its_name",
        "## Main\nLet chr be (n: Int) -> n * 2.\nShow chr(66).\nLet twice be (n: Int) -> n * 2.\nShow twice(21).",
        "B\n42",
    ),
    case(
        "a_push_through_a_field",
        "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPush 5 to b's items.\nPush 9 to b's items.\nShow length of b's items.\nShow item 2 of b's items.",
        "2\n9",
    ),
    case(
        "a_pop_through_a_field",
        "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items [1, 2, 3].\nPop from b's items.\nShow b's items.",
        "[1, 2]",
    ),
    case(
        "an_item_set_through_a_field",
        "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items [1, 2, 3].\nSet item 2 of b's items to 7.\nShow b's items.",
        "[1, 7, 3]",
    ),
    case(
        "a_field_set_through_a_field",
        "## A Pos has:\n    an x, which is Int.\n## A Body has:\n    a pos, which is Pos.\n## Main\nLet mutable p be a new Body with pos (a new Pos with x 1).\nSet p's pos's x to 3.\nShow p's pos's x.",
        "3",
    ),
    case(
        "a_map_entry_set_through_a_field",
        "## A Reg has:\n    a stock, which is Map of Text to Int.\n## Main\nLet mutable r be a new Reg with stock (a new Map of Text to Int).\nSet item \"wood\" of r's stock to 50.\nShow item \"wood\" of r's stock.",
        "50",
    ),
    case(
        "a_pop_into_a_name",
        "## Main\nLet mutable xs be [1, 2, 3].\nPop from xs into v.\nShow v.\nShow xs.",
        "3\n[1, 2]",
    ),
    case(
        "a_pop_through_a_field_into_a_name",
        "## A Bag has:\n    an items, which is Seq of Int.\n## Main\nLet mutable b be a new Bag with items [4, 5].\nPop from b's items into w.\nShow w.\nShow b's items.",
        "5\n[4]",
    ),
    case(
        "the_real_constants_resolve_as_every_tier_resolves_them",
        "## Main\nShow 2.0 * pi.\nShow e * 1.0.",
        "6.283185307179586\n2.718281828459045",
    ),
    case(
        "a_slice_past_the_end_is_empty",
        "## Main\nLet xs be [10, 20, 30].\nLet ys be items 2 through 9 of xs.\nShow length of ys.\nLet zs be items 2 through 3 of xs.\nShow zs.",
        "0\n[20, 30]",
    ),
    case(
        "an_option_is_inspected_by_its_arms",
        "## To exchange (n: Int) -> Int:\n    Let mutable a be some 5.\n    Let mutable b be some 7.\n    Swap a and b.\n    Inspect a:\n        When Some (n):\n            Show n.\n        When None:\n            Show \"none\".\n    Return n.\n\n## Main\nShow exchange(0).",
        "7\n0",
    ),
    case(
        "a_span_runs_its_body_where_it_stands",
        "## Main\nDuring processing:\n    Show 1.\n    Show 2.\nShow 3.",
        "1\n2\n3",
    ),
    case(
        "every_condition_takes_its_values_truth",
        "## Main\nLet n be 5.\nIf n:\n    Show \"yes\".\nLet z be 0.\nIf z:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".\nLet xs be [1].\nIf xs:\n    Show \"listed\".\nLet mutable k be 3.\nWhile k:\n    Show k.\n    Set k to k - 1.\nShow \"done\".",
        "yes\nno\nlisted\n3\n2\n1\ndone",
    ),
    case(
        "a_map_keyed_by_ints_holds_its_entries",
        "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 3 of m to 4.\nSet item 5 of m to 6.\nShow length of m.\nShow item 3 of m.\nShow item 5 of m.",
        "2\n4\n6",
    ),
    case(
        "every_value_has_the_languages_truth",
        "## Main\nLet s be \"\".\nShow s and true.\nLet r be 0.0.\nShow r or false.\nLet xs be a new Seq of Int.\nShow xs and true.\nShow not s.\nLet n be 5.\nShow not n.\nLet word be \"a\".\nShow not word.\nLet ys be a new Set of Int.\nShow ys and true.\nLet zs be a new Map of Text to Int.\nShow zs and true.",
        "false\nfalse\nfalse\ntrue\nfalse\nfalse\nfalse\nfalse",
    ),
    case(
        "a_left_out_field_starts_at_its_declared_default",
        "## A Record has:\n    A count: Int.\n    A label: Text.\n    A done: Bool.\n    A ratio: Float.\n    An items: Seq of Int.\n    A stock: Map of Text to Int.\n    A tags: Set of Text.\n\n## Main\nLet mutable r be a new Record with label \"x\".\nShow r's count.\nShow r's label.\nShow r's done.\nShow r's ratio.\nShow length of r's items.\nPush 5 to r's items.\nShow length of r's items.\nSet item \"wood\" of r's stock to 3.\nShow item \"wood\" of r's stock.\nAdd \"a\" to r's tags.\nShow length of r's tags.",
        "0\nx\nfalse\n0\n0\n1\n3\n1",
    ),
    case(
        "a_construction_naming_no_field_starts_every_slot",
        "## A Counter has:\n    A n: Int.\n\n## Main\nLet mutable c be a new Counter.\nSet c's n to c's n + 1.\nSet c's n to c's n + 1.\nShow c's n.",
        "2",
    ),
    case(
        "a_left_out_field_of_a_known_construction_reads_its_default",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet p be a new Point with y 3.\nShow p's x + p's y.\nLet q be a new Point.\nShow q's x.\nShow q's y * 2.",
        "3\n0\n0",
    ),
    case(
        "a_left_out_field_of_a_struct_type_holds_nothing",
        "## An Inner has:\n    A v: Int.\n\n## An Outer has:\n    An inner: Inner.\n    A k: Int.\n\n## Main\nLet o be a new Outer with k 2.\nShow o's inner.\nShow o's k.",
        "nothing\n2",
    ),
    case(
        "a_left_out_counter_field_is_a_fresh_replica",
        "## Definition\nA Board is Shared and has:\n    hits: ConvergentCount.\n    lives: Tally.\n## Main\n    Let mutable b be a new Board.\n    Show b's hits.\n    Increase b's hits by 2.\n    Increase b's hits by 3.\n    Decrease b's lives by 4.\n    Increase b's lives by 1.\n    Show b's hits.\n    Show b's lives.\n    Show b's hits is greater than 4.\n    Show b's lives equals -3.",
        "0\n5\n-3\ntrue\ntrue",
    ),
    case(
        "merging_replicas_sums_and_converging_is_idempotent",
        "## Definition\nA Score is Shared and has:\n    points: ConvergentCount.\n## Main\n    Let mutable a be a new Score.\n    Increase a's points by 10.\n    Let mutable b be a new Score.\n    Increase b's points by 4.\n    Merge b into a.\n    Merge b into a.\n    Show a's points.\n    Increase b's points by 1.\n    Merge a into b.\n    Show b's points.",
        "14\n15",
    ),
    case(
        "a_replica_made_in_a_function_stays_distinct",
        "## Definition\nA Score is Shared and has:\n    points: ConvergentCount.\n\n## To fresh (n: Int) -> Score:\n    Let mutable s be a new Score.\n    Increase s's points by n.\n    Return s.\n\n## Main\n    Let mutable a be fresh(3).\n    Let b be fresh(5).\n    Merge b into a.\n    Show a's points.",
        "8",
    ),
    case(
        "a_resolve_sets_its_field",
        "## Definition\nA Page is Shared and has:\n    a title, which is a Divergent Text.\n\n## Main\n    Let mutable page be a new Page.\n    Resolve page's title to \"Final\".\n    Show page's title.",
        "Final",
    ),
    case(
        "every_format_spec_formats_as_the_language_does",
        "## Main\nLet x be 3.14159.\nLet y be 3.7.\nLet n be 42.\nLet w be \"hi\".\nShow \"{x:.2}\".\nShow \"{y:.0}\".\nShow \"{n:.1}\".\nShow \"{n:>6}|\".\nShow \"|{w:<5}|\".\nShow \"|{w:^7}|\".\nShow \"{n:5}\".\nShow \"{x:$}\".\nShow \"{n:$}\".\nShow \"{x=:.3}\".\nShow \"{n=}\".",
        "3.14\n4\n42.0\n    42|\n|hi   |\n|  hi   |\n   42\n$3.14\n$42.00\nx=3.142\nn=42",
    ),
    case(
        "a_capability_one_branch_grants_runs_on",
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet alice be a new User with name \"Alice\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that alice can edit doc.\nShow \"edited\".",
        "edited",
    ),
    case(
        "a_shared_sequence_holds_its_appends_in_order",
        "## Definition\nA Log is Shared and has:\n    a entries, which is a SharedSequence of Text.\n## Main\n    Let mutable g be a new Log.\n    Show g's entries.\n    Append \"a\" to g's entries.\n    Append \"b\" to g's entries.\n    Append \"c\" to g's entries.\n    Show g's entries.\n    Show length of g's entries.",
        "[]\n[a, b, c]\n3",
    ),
    case(
        "merged_sequences_order_by_stamp_and_replica",
        "## Definition\nA Doc is Shared and has:\n    lines: SharedSequence of Text.\n## Main\n    Let mutable a be a new Doc.\n    Append \"x\" to a's lines.\n    Append \"y\" to a's lines.\n    Let mutable b be a new Doc.\n    Append \"p\" to b's lines.\n    Append \"q\" to b's lines.\n    Merge b into a.\n    Show a's lines.\n    Show length of a's lines.\n    Repeat for line in a's lines:\n        Show line.\n    Append \"z\" to b's lines.\n    Merge a into b.\n    Show b's lines.",
        "[p, q, x, y]\n4\np\nq\nx\ny\n[p, q, z, x, y]",
    ),
    case(
        "a_character_by_its_code",
        "## Main\nShow chr(65).\nLet q be chr(34).\nLet mutable n be 0.\nRepeat for i from 1 to 10:\n    Set n to n + 1.\nShow \"{q}{chr(n + 87)}{q}\".",
        "A\n\"a\"",
    ),
    case(
        "durations_show_and_add_as_the_language_does",
        "## Main\nShow 3 minutes.\nShow 1500 microseconds.\nShow 2 hours + 30 minutes.\nShow 0ns - 5ns.\nShow 90 seconds is less than 2 minutes.\nShow 3 minutes equals 180 seconds.\nShow 250 milliseconds - 1 seconds.\nLet d be 45 seconds.\nShow d + d + d.\nShow 1500 microseconds equals 1 milliseconds + 500 microseconds.\nShow 90 minutes is greater than 1 hours.\nShow 90 minutes equals 5400 seconds.",
        "3min\n1ms\n2h\n-5ns\ntrue\ntrue\n-750ms\n2min\ntrue\ntrue\ntrue",
    ),
    case(
        "a_moment_moves_by_durations_and_spans",
        "## Main\nLet m be 2026-11-08 at 9:30am.\nShow m.\nShow m + 90 seconds.\nShow m + 1 month.\nShow m - 2 days.\nShow m - 45 minutes.\nShow m is greater than 2026-11-08 at 9am.\nShow m equals 2026-11-08 at 9:30am.\nShow 2024-01-31 at 11:59pm + 1 month.\nShow m + 1500 milliseconds is greater than m + 1 seconds.\nShow m + 1500 milliseconds equals m + 1 seconds + 500 milliseconds.",
        "2026-11-08 09:30\n2026-11-08 09:31\n2026-12-08 09:30\n2026-11-06 09:30\n2026-11-08 08:45\ntrue\ntrue\n2024-02-29 23:59\ntrue\ntrue",
    ),
    case(
        "a_date_moves_by_spans",
        "## Main\nLet d be 2024-02-29.\nShow d.\nShow d + 1 year.\nShow d - 1 month.\nShow d + 2 years and 3 days.\nShow d is less than 2024-03-01.\nShow d equals 2024-02-29.\nShow the year of d.\nShow month_of(d).\nShow day_of(d + 1 day).",
        "2024-02-29\n2025-02-28\n2024-01-29\n2026-03-03\ntrue\ntrue\n2024\n2\n1",
    ),
    case(
        "a_time_of_day_orders_against_a_moment",
        "## Main\nLet t be 9:30am.\nShow t.\nShow 12:00am.\nShow 11:45pm.\nShow t is less than 10am.\nShow t equals 9:30am.\nShow 2026-11-08 at 9:45am is greater than t.\nShow t is at least 2026-11-08 at 9:30am.",
        "09:30:00\n00:00:00\n23:45:00\ntrue\ntrue\ntrue\ntrue",
    ),
    case(
        "spans_show_in_years_months_and_days",
        "## Main\nShow 2 years and 3 days.\nShow 14 months.\nShow 1 year.\nShow 0 days.\nShow 1 month equals 1 month.\nShow 12 months equals 1 year.",
        "2 years and 3 days\n1 year and 2 months\n1 year\n0 days\ntrue\ntrue",
    ),
    case(
        "a_character_shows_and_compares",
        "## Main\nLet c be `x`.\nShow c.\nShow c equals `x`.\nShow c equals `y`.\nShow \"[{c}]\".",
        "x\ntrue\nfalse\n[x]",
    ),
    case(
        "the_calendar_builtins_answer_as_the_language_does",
        "## Main\nLet m be 2024-03-15 at 10:30am + 45 seconds.\nShow m.\nShow format_timestamp(m).\nShow format_timestamp(add_seconds(m, 3600)).\nShow year_of(m).\nShow month_of(m).\nShow day_of(m).\nShow hour_of(m).\nShow minute_of(m).\nShow second_of(m).\nShow weekday_of(m).\nShow week_of(m).\nShow quarter_of(m).\nShow date_of(m).\nShow time_of(m).\nShow year_of(2024-02-29).\nShow weekday_of(date_of(m)).\nShow seconds_between(m, add_seconds(m, 90)).\nShow months_between(2024-01-15 at 12:00am, m).\nShow years_between(2020-03-15 at 12:00am, m).\nShow format_timestamp(m + 123456789ns).",
        "2024-03-15 10:30\n2024-03-15T10:30:45Z\n2024-03-15T11:30:45Z\n2024\n3\n15\n10\n30\n45\n5\n11\n1\n2024-03-15\n10:30:45\n2024\n5\n90\n2\n4\n2024-03-15T10:30:45.123456789Z",
    ),
    case(
        "temporal_values_flow_through_records_lists_and_calls",
        "## A Meeting has:\n    a start: Moment.\n    a length: Duration.\n\n## To endOf (mt: Meeting) -> Moment:\n    Return mt's start + mt's length.\n\n## To nextDay (d: Date) -> Date:\n    Return d + 1 day.\n\n## Main\nLet mt be a new Meeting with start 2026-11-08 at 2pm and length 90 minutes.\nShow endOf(mt).\nShow mt's length.\nLet mutable total be 0ns.\nLet gaps be [10 seconds, 20 seconds, 30 seconds].\nRepeat for g in gaps:\n    Set total to total + g.\nShow total.\nShow nextDay(2026-12-31).\nLet days be [2026-01-01, 2026-06-15].\nShow days.\nShow item 2 of days.",
        "2026-11-08 15:30\n1h\n1min\n2027-01-01\n[2026-01-01, 2026-06-15]\n2026-06-15",
    ),
    case(
        "a_sleep_waits_for_a_duration_or_milliseconds",
        "## Main\nShow 1.\nSleep 2 milliseconds.\nShow 2.\nSleep 1.\nShow 3.",
        "1\n2\n3",
    ),
    case(
        "a_left_out_character_field_starts_at_nul",
        "## A Glyph has:\n    A c: Char.\n    A n: Int.\n\n## Main\nLet g be a new Glyph with n 1.\nShow g's n.\nShow g's c equals `\\0`.",
        "1\ntrue",
    ),
    case(
        "a_character_literal",
        "## Main\nShow 1.\nLet c be `a`.\nShow c.",
        "1\na",
    ),
    case(
        "a_date_literal",
        "## Main\nLet d be 2026-05-20.\nShow d.",
        "2026-05-20",
    ),
    case(
        "a_moment_literal",
        "## Main\nLet m be 2026-05-20 at 9:30am.\nShow m.",
        "2026-05-20 09:30",
    ),
    case(
        "a_span_literal",
        "## Main\nShow 2.\nLet s be 1 year and 3 days.\nShow s.",
        "2\n1 year and 3 days",
    ),
    case(
        "a_time_literal",
        "## Main\nLet t be 4pm.\nShow t.",
        "16:00:00",
    ),
    case(
        "a_date_no_statement_reads",
        "## Main\nLet d be 2026-05-20.\nShow 1.",
        "1",
    ),
    case(
        "a_left_out_character_field",
        "## A Glyph has:\n    A c: Char.\n    A n: Int.\n\n## Main\nLet g be a new Glyph with n 1.\nShow g's n.\nShow g's c.",
        "1\n\0",
    ),
    case(
        "containers_compare_as_the_language_compares_them",
        "## A Point has:\n    an x: Int.\n    a label: Text.\n\n## A Shape is one of:\n    A Dot with size Int.\n    A Line with from Int and to Int.\n\n## Main\nShow [\"1\"] equals [1].\nShow [1, 2] equals [1, 2].\nShow [1] equals [1.0].\nShow [[1, 2], [3]] equals [[1, 2], [3]].\nShow [[1, 2], [3]] equals [[1, 2], [\"3\"]].\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 2 to s.\nLet mutable t be a new Set of Int.\nAdd 2 to t.\nAdd 1 to t.\nShow s equals t.\nLet mutable u be a new Set of Text.\nAdd \"1\" to u.\nAdd \"2\" to u.\nShow s equals u.\nLet p be a new Point with x 1 and label \"a\".\nLet q be a new Point with x 1 and label \"a\".\nLet r be a new Point with x 1 and label \"b\".\nShow p equals q.\nShow p equals r.\nLet d be a new Dot with size 3.\nLet e be a new Dot with size 3.\nLet f be a new Line with from 3 and to 3.\nShow d equals e.\nShow d equals f.\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nLet mutable n be a new Map of Text to Int.\nSet item \"a\" of n to 1.\nShow m equals n.\nSet item \"a\" of n to 2.\nShow m equals n.\nShow \"1\" equals 1.\nShow nothing equals nothing.",
        "false\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\nfalse\ntrue",
    ),
    case(
        "a_map_holds_each_key_as_its_value",
        "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 1 of m to 5.\nSet item 2 of m to 7.\nRepeat for (k, v) in m:\n    Show k + 1.\n    Show v.\nShow m.\nShow item 2 of m.\nShow m contains 1.\nShow m contains 3.\nSet item 1 of m to 9.\nShow m.\nLet mutable t be a new Map of Text to Int.\nSet item \"1\" of t to 3.\nSet item \"2\" of t to 4.\nShow t contains \"1\".\nShow t.\nRemove \"1\" from t.\nShow t.\nShow length of t.\nShow [1, 2] contains 2.\nShow [1, 2] contains \"2\".\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 1 to s.\nAdd 2 to s.\nShow length of s.\nShow s contains 2.\nLet mutable u be a new Set of Text.\nAdd \"2\" to u.\nShow u contains \"2\".",
        "2\n5\n3\n7\n{1: 5, 2: 7}\n7\ntrue\nfalse\n{1: 9, 2: 7}\ntrue\n{1: 3, 2: 4}\n{2: 4}\n1\ntrue\nfalse\n2\ntrue\ntrue",
    ),
    case(
        "two_functions_run_one_block_under_their_own_parameters",
        "## To f (a: Int) -> Int:\n    If a is greater than 100:\n        Show \"big\".\n    Return a + 1.\n\n## To g (b: Int) -> Int:\n    If b is greater than 100:\n        Show \"big\".\n    Return b + 2.\n\n## Main\nShow f(5).\nShow g(5).",
        "6\n7",
    ),
    case(
        "two_closures_passed_to_one_function_keep_their_bodies",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet inc be (n: Int) -> n + 1.\nLet dbl be (n: Int) -> n * 2.\nShow apply(inc, 5).\nShow apply(dbl, 5).",
        "6\n10",
    ),
    case(
        "a_comprehension_binds_its_list_where_it_stands",
        "## Main\nLet xs be [1, 5, 2, 8, 3].\nLet small be every n in xs where n is less than 4.\nShow small.",
        "[1, 2, 3]",
    ),
    case(
        "a_destructuring_let_binds_each_name_where_it_stands",
        "## To pair (n: Int) -> (Int, Int):\n    Return (n, n * 2).\n\n## Main\nLet (a, b) be pair(3).\nShow a + b.",
        "9",
    ),
    case(
        "a_desugaring_binds_in_a_function_a_loop_and_a_closures_block",
        "## To spread (xs: Seq of Int) -> Int:\n    Let big be every x in xs where x is greater than 2.\n    Let (lo, hi) be (item 1 of big, item 2 of big).\n    Return lo + hi.\n\n## Main\nLet mutable total be 0.\nRepeat for i from 1 to 3:\n    Let picked be every x in [1, 2, 3, 4] where x is greater than i.\n    Set total to total + length of picked.\nShow total.\nShow spread([1, 3, 5, 7]).\nLet count be (n: Int) ->:\n    Let above be every x in [1, 2, 3, 4, 5] where x is greater than n.\n    Return length of above.\nShow count(2).",
        "6\n8\n3",
    ),
    case(
        "a_closures_block_runs_its_loop_its_inspect_and_its_desugarings",
        "## A Shape is one of:\n    A Dot with size Int.\n    A Line with from Int and to Int.\n\n## Main\nLet measure be (s: Shape) ->:\n    Let mutable total be 0.\n    Repeat for i from 1 to 3:\n        Set total to total + i.\n    Let (lo, hi) be (total, total * 2).\n    Inspect s:\n        When Dot (size):\n            Return lo + size.\n        When Line (from, to):\n            Return hi + to - from.\nShow measure(a new Dot with size 4).\nShow measure(a new Line with from 1 and to 5).",
        "10\n16",
    ),
    case(
        "a_reversible_block_binds_in_the_block_it_stands_in",
        "## Main\nLet mutable x be 5.\nReversibly:\n    Let y be x + 3.\n    Set x to y xor 1.\nShow x.\nShow y.",
        "9\n8",
    ),
    case(
        "a_tensor_declaration_binds_its_tensor",
        "## Main\nLet A be a 2 by 2 tensor of Int.\nSet item 1 of A to 7.\nSet item 4 of A to 9.\nShow item 1 of A + item 4 of A.",
        "16",
    ),
    case(
        "a_copied_set_holds_items_of_its_own",
        "## Main\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nLet b be copy of a.\nAdd 2 to a.\nShow a.\nShow b.",
        "{1, 2}\n{1}",
    ),
    case(
        "a_set_and_its_copies_change_apart",
        "## Main\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nLet mutable b be copy of a.\nAdd 2 to a.\nAdd 3 to b.\nRemove 1 from a.\nShow a.\nShow b.\nShow length of a.\nShow b contains 2.\nLet c be copy of b.\nAdd 4 to b.\nShow c.",
        "{2}\n{1, 3}\n1\nfalse\n{1, 3}",
    ),
    case(
        "a_set_counts_a_run_time_value_it_already_holds_once",
        "## Main\nLet n be parseInt(\"1\").\nLet mutable b be a new Set of Int.\nAdd 1 to b.\nAdd n to b.\nShow b.\nShow length of b.\nShow b contains 1.",
        "{1}\n1\ntrue",
    ),
    case(
        "a_set_holding_a_run_time_value_answers_for_it",
        "## Main\nLet n be parseInt(\"3\").\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nLet mutable b be copy of a.\nAdd n to b.\nAdd 2 to b.\nShow b.\nShow b contains 3.",
        "{1, 3, 2}\ntrue",
    ),
    case(
        "a_copied_set_grows_under_a_run_time_branch",
        "## Main\nLet n be parseInt(\"4\").\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nLet mutable b be copy of a.\nIf n is greater than 2:\n    Add 5 to b.\nShow b.\nShow a.",
        "{1, 5}\n{1}",
    ),
    case(
        "a_function_grows_a_copy_of_its_set",
        "## To grow (s: Set of Int) -> Int:\n    Let mutable t be copy of s.\n    Add 9 to t.\n    Return length of t.\n\n## Main\nLet mutable a be a new Set of Int.\nAdd 1 to a.\nShow grow(a).",
        "2",
    ),
];

/// Programs the self-interpreter, the textbook first projection and the direct specialization all run as the
/// language runs them. May only rise.
const SEMANTICS_PROGRAMS: usize = 116;

fn sint_on_the_reference_host(program: &str) -> Result<String, String> {
    let subject = logicaffeine_compile::compile::self_interpreter_run_source(program)?;
    logicaffeine_compile::compile::interpret_bootstrap(&subject).map_err(|e| format!("{e:?}"))
}

fn sint_on_the_value_host(program: &str) -> Result<String, String> {
    let subject = logicaffeine_compile::compile::self_interpreter_run_source(program)?;
    logicaffeine_compile::compile::interpret_program(&subject).map_err(|e| format!("{e:?}"))
}

#[test]
fn the_self_interpreter_runs_the_language() {
    assert!(
        SEMANTICS_CORPUS.len() >= SEMANTICS_PROGRAMS,
        "the semantics corpus shrank to {} below its floor {SEMANTICS_PROGRAMS}",
        SEMANTICS_CORPUS.len()
    );
    let mut failures = Vec::new();
    for c in SEMANTICS_CORPUS {
        let tw = run_treewalk(c.program);
        assert_eq!(
            tw.output, c.expected,
            "`{}`: the tree-walker prints something else — the corpus entry is wrong, not the interpreter: {:?}",
            c.name, tw
        );
        for (host, run) in [
            ("the reference host", sint_on_the_reference_host as fn(&str) -> Result<String, String>),
            ("the value host", sint_on_the_value_host),
        ] {
            let got = run(c.program);
            if got.as_deref() != Ok(c.expected) {
                failures.push(format!("`{}` on {host}: the self-interpreter gives {got:?}, the language {:?}", c.name, c.expected));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} self-interpreter runs differ from the language:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_textbook_first_projection_runs_the_language() {
    assert_corpus(
        "semantics (textbook first projection)",
        SEMANTICS_CORPUS,
        SEMANTICS_PROGRAMS,
        logicaffeine_compile::compile::textbook_projection1,
    );
}

#[test]
fn a_specialization_runs_the_language() {
    assert_corpus("semantics (direct specialization)", SEMANTICS_CORPUS, SEMANTICS_PROGRAMS, decompile);
}

/// Programs that store a value the online specializer knows only in part: its kind is known, a value inside it is the
/// answer of a builtin it does not fold. The specializer binds each such value once, where it is computed, and keeps
/// its kind (D77), so the textbook first projection's residual runs as the language runs and keeps no more interpreter
/// dispatch than the count beside it, which may only fall: a branch on such a value still writes the environment back.
const PARTLY_KNOWN_CORPUS: &[(&str, &str, usize)] = &[
    ("a_parsed_number_stored_then_added", "## Main\nLet n be parseInt(\"15\").\nShow n + 1.\n", 0),
    ("parsed_numbers_in_a_list", "## Main\nLet xs be [parseInt(\"1\"), parseInt(\"2\")].\nShow length of xs.\nShow item 2 of xs.\n", 0),
    ("a_parse_that_fails_after_output", "## Main\nShow 1.\nLet n be parseInt(\"abc\").\nShow 2.\n", 0),
    ("a_branch_on_a_parsed_number", "## Main\nLet n be parseInt(\"7\").\nIf n is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".\n", 3),
    ("the_calendar_builtins_that_can_stop_the_program", "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nShow m.\nShow format_timestamp(m).\nShow makeDate(2024, 3, 15).\nShow makeMoment(2024, 3, 15, 10, 30, 45) equals m.\nShow in_zone(parse_timestamp(\"2024-07-01T12:00:00Z\"), \"America/New_York\").\nShow the hour of timestamp \"2024-07-01T12:00:00Z\" in \"America/New_York\".\nShow format_timestamp(parse_timestamp(\"2024-03-15T10:30:45.123456789Z\")).\n", 0),
    ("a_recursive_procedure_writes_a_parsed_number_back", "## To bumpTimes (x: mutable Int) and (k: Int):\n    If k is at most 0:\n        Return.\n    Set x to x + 1.\n    bumpTimes(x, k - 1).\n\n## Main\nLet n be parseInt(\"3\").\nLet mutable x be n * 10.\nLet a be x.\nbumpTimes(x, n).\nShow a.\nShow x.\n", 0),
];

#[test]
fn a_value_the_specializer_knows_in_part_runs_as_the_language_runs() {
    let mut failures = Vec::new();
    for (name, program, ceiling) in PARTLY_KNOWN_CORPUS {
        let source = run_treewalk(program);
        match logicaffeine_compile::compile::textbook_projection1(program) {
            Ok(residual) => {
                if let Some(diff) = behavior_diff(&source, &run_treewalk(&residual), CmpMode::Strict) {
                    failures.push(format!("`{name}`: the residual does not run as the program runs — {diff}\n{residual}"));
                }
                let d = count_dispatch(&residual);
                if d > *ceiling {
                    failures.push(format!("`{name}`: the residual keeps {d} unit(s) of interpreter dispatch, above its {ceiling}\n{residual}"));
                }
            }
            Err(e) => failures.push(format!("`{name}`: the textbook first projection failed: {e}")),
        }
    }
    assert!(failures.is_empty(), "{} residuals of partly known values misrun:\n{}", failures.len(), failures.join("\n═══\n"));
}

/// Programs whose static aggregates name a variable that a later statement writes. `d` is computed by a loop longer
/// than the specializer unrolls, so it is a run-time value the aggregate can only refer to.
const CAPTURE_CORPUS: &[Case] = &[
    case(
        "a_record_outlives_a_write_to_its_field_source",
        "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet b be a new Box with base 5 and flex d.\nSet d to 0.\nShow b's flex.",
        "5050",
    ),
    case(
        "a_whole_record_outlives_a_write_to_its_field_source",
        "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet b be a new Box with base 5 and flex d.\nSet d to 0.\nShow b.",
        "Box { base: 5, flex: 5050 }",
    ),
    case(
        "a_list_outlives_a_write_to_its_item_source",
        "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet xs be [1, d, 3].\nSet d to 0.\nShow item 2 of xs.",
        "5050",
    ),
    case(
        "a_nested_list_outlives_a_write_to_its_item_source",
        "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet xs be [[d], [2]].\nSet d to 1.\nShow xs.",
        "[[5050], [2]]",
    ),
    case(
        "a_tuple_outlives_a_write_to_its_item_source",
        "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet t be (d, 2).\nSet d to 0.\nShow item 1 of t.",
        "5050",
    ),
    case(
        "a_field_written_from_a_variable_outlives_a_write_to_it",
        "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet mutable b be a new Box with base 1 and flex 7.\nSet b's base to d.\nSet d to 0.\nShow b's base.\nShow b's flex.",
        "5050\n7",
    ),
    case(
        "a_copy_made_in_an_unrolled_loop_outlives_a_later_write",
        "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet mutable x be 1.\nLet mutable k be 0.\nWhile k is less than 1:\n    Set x to d.\n    Set k to k + 1.\nSet d to 0.\nShow x.",
        "5050",
    ),
    case(
        "a_record_outlives_a_write_under_an_undecided_branch",
        "## A Box has:\n    A base: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet b be a new Box with base d.\nIf d is greater than 5:\n    Set d to 1.\nShow b's base.",
        "5050",
    ),
    case(
        "a_list_outlives_a_write_through_a_mutable_argument",
        "## To zero (n: mutable Int):\n    Set n to 0.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet xs be [d].\nzero(d).\nShow item 1 of xs.",
        "5050",
    ),
    case(
        "a_record_made_in_one_unrolled_iteration_outlives_the_next",
        "## A Box has:\n    A base: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet mutable b be a new Box with base 0.\nLet mutable k be 0.\nWhile k is less than 2:\n    Let t be d + k.\n    If k equals 0:\n        Set b to a new Box with base t.\n    Set k to k + 1.\nShow b's base.",
        "5050",
    ),
    case(
        "a_record_a_call_returns_outlives_a_write_to_its_argument",
        "## A Box has:\n    A base: Int.\n\n## To wrap (n: Int) -> Box:\n    Return a new Box with base n.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet b be wrap(d).\nSet d to 0.\nShow b's base.",
        "5050",
    ),
    case(
        "a_list_of_a_copy_outlives_a_write_to_the_original",
        "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + i.\nLet e be d.\nLet xs be [e].\nSet d to 0.\nShow item 1 of xs.\nShow e.",
        "5050\n5050",
    ),
];

/// Programs whose static aggregates keep their meaning across a write. May only rise.
const CAPTURE_PROGRAMS: usize = 12;

#[test]
fn a_known_value_keeps_its_meaning_across_a_write() {
    assert_corpus("capture (direct specialization)", CAPTURE_CORPUS, CAPTURE_PROGRAMS, decompile);
}

#[test]
fn the_textbook_first_projection_keeps_a_known_values_meaning_across_a_write() {
    assert_corpus(
        "capture (textbook first projection)",
        CAPTURE_CORPUS,
        CAPTURE_PROGRAMS,
        logicaffeine_compile::compile::textbook_projection1,
    );
}

/// Programs every pipeline must specialize identically on both hosts: all the corpora above. May only rise.
const HOST_AGNOSTIC_PROGRAMS: usize = 100;

fn every_corpus_program() -> Vec<&'static Case> {
    EFFECT_CORPUS
        .iter()
        .chain(INSPECT_CORPUS)
        .chain(TEXTBOOK_P1_CORPUS)
        .chain(SEMANTICS_CORPUS)
        .chain(CAPTURE_CORPUS)
        .collect()
}

fn specializer_on_the_reference_host(program: &str) -> Result<String, String> {
    let combined = logicaffeine_compile::compile::pe_combined_source("", program)?;
    let raw = logicaffeine_compile::compile::interpret_bootstrap(&combined).map_err(|e| format!("{e:?}"))?;
    logicaffeine_compile::compile::finish_projection1_residual(raw)
}

fn specializer_on_the_value_host(program: &str) -> Result<String, String> {
    let combined = logicaffeine_compile::compile::pe_combined_source("", program)?;
    let raw = logicaffeine_compile::compile::interpret_bootstrap_on_the_value_host(&combined).map_err(|e| format!("{e:?}"))?;
    logicaffeine_compile::compile::finish_projection1_residual(raw)
}

fn textbook_on_the_reference_host(program: &str) -> Result<String, String> {
    let combined = logicaffeine_compile::compile::textbook_projection1_subject_source(program)?;
    let raw = logicaffeine_compile::compile::interpret_bootstrap(&combined).map_err(|e| format!("{e:?}"))?;
    logicaffeine_compile::compile::finish_projection1_residual(raw)
}

fn textbook_on_the_value_host(program: &str) -> Result<String, String> {
    let combined = logicaffeine_compile::compile::textbook_projection1_subject_source(program)?;
    let raw = logicaffeine_compile::compile::interpret_bootstrap_on_the_value_host(&combined).map_err(|e| format!("{e:?}"))?;
    logicaffeine_compile::compile::finish_projection1_residual(raw)
}

type Residual = fn(&str) -> Result<String, String>;

fn assert_hosts_agree(label: &str, on_reference: Residual, on_value: Residual) {
    let programs = every_corpus_program();
    assert!(
        programs.len() >= HOST_AGNOSTIC_PROGRAMS,
        "the corpora shrank to {} below the floor {HOST_AGNOSTIC_PROGRAMS}",
        programs.len()
    );
    let mut failures = Vec::new();
    for c in programs {
        let reference = on_reference(c.program);
        let value = on_value(c.program);
        if reference != value {
            failures.push(format!(
                "`{}`:\n--- reference host ---\n{}\n--- value host ---\n{}",
                c.name,
                reference.unwrap_or_else(|e| e),
                value.unwrap_or_else(|e| e)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} programs specialize differently on the two hosts ({label}):\n{}",
        failures.len(),
        failures.join("\n═══\n")
    );
}

#[test]
fn the_specializer_means_the_same_on_either_host() {
    assert_hosts_agree(
        "direct specialization",
        specializer_on_the_reference_host,
        specializer_on_the_value_host,
    );
}

#[test]
fn the_textbook_first_projection_means_the_same_on_either_host() {
    assert_hosts_agree(
        "textbook first projection",
        textbook_on_the_reference_host,
        textbook_on_the_value_host,
    );
}

/// A program that reads the clock: the specializer never reads it, so the textbook first projection keeps each
/// `today` and `now` a read its residual makes when it runs, with no interpreter dispatch left.
#[test]
fn the_textbook_first_projection_reads_the_clock_when_it_runs() {
    let program = "## Main\nLet d be today.\nLet e be d + 3 days.\nShow e is greater than d.\nLet t be now.\nShow now is at least t.\n";
    let residual = logicaffeine_compile::compile::textbook_projection1(program)
        .unwrap_or_else(|e| panic!("the textbook first projection carries the clock: {e}"));
    assert_eq!(count_dispatch(&residual), 0, "the residual keeps interpreter dispatch:\n{residual}");
    assert!(
        residual.contains("today") && residual.contains("now"),
        "the residual reads the clock itself:\n{residual}"
    );
    assert_eq!(run_treewalk(&residual).output, "true\ntrue", "the residual prints what the program prints:\n{residual}");
}

/// Programs of every corpus above that the first projection through the offline specializer runs as the language runs
/// them, dispatch-free. May only rise.
const MIX_P1_PROGRAMS: usize = 100;

#[test]
fn the_offline_specializer_specializes_the_self_interpreter_away() {
    let corpora: [(&str, &[Case]); 5] = [
        ("effect", EFFECT_CORPUS),
        ("inspect", INSPECT_CORPUS),
        ("textbook", TEXTBOOK_P1_CORPUS),
        ("semantics", SEMANTICS_CORPUS),
        ("capture", CAPTURE_CORPUS),
    ];
    let total: usize = corpora.iter().map(|(_, c)| c.len()).sum();
    assert!(total >= MIX_P1_PROGRAMS, "the corpora shrank to {total} below the floor {MIX_P1_PROGRAMS}");
    let mut failures = Vec::new();
    for (label, corpus) in corpora {
        for f in specialization_failures(corpus, logicaffeine_compile::compile::mix_projection1) {
            failures.push(format!("[{label}] {f}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {total} programs are specialized wrongly through the offline specializer:\n{}",
        failures.len(),
        failures.join("\n═══\n")
    );
}

/// Every residual is printed by the decompiler (`decompile_source.logos`), so every form it prints is the language:
/// each corpus program's Core IR, decompiled — its functions, then its main block — runs as the program runs.
#[test]
fn the_decompiler_prints_the_language() {
    use logicaffeine_compile::compile::{core_types_for_pe_source, decompile_source_text, encode_program_source, interpret_bootstrap};
    let mut failures = Vec::new();
    for c in every_corpus_program() {
        let full = if c.program.contains("## Main") { c.program.to_string() } else { format!("## Main\n{}", c.program) };
        let encoded = encode_program_source(&full).expect("the corpus program encodes");
        let printer = format!(
            "{}\n{}\n## Main\n{encoded}\nRepeat for decompiledF in encodedFuncSeq:\n    Show decompileFunc(decompiledF).\nShow \"## Main\".\nShow decompileBlock(encodedMain, 0).\n",
            core_types_for_pe_source(),
            decompile_source_text()
        );
        let printed = match interpret_bootstrap(&printer) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("`{}`: the decompiler failed: {e:?}", c.name));
                continue;
            }
        };
        let mut definitions = String::new();
        let mut keep = false;
        for line in full.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                keep = header != "Main" && !header.starts_with("To ");
            }
            if keep {
                definitions.push_str(line);
                definitions.push('\n');
            }
        }
        let reprinted = format!("{definitions}{printed}");
        if let Some(diff) = behavior_diff(&run_treewalk(&full), &run_treewalk(&reprinted), CmpMode::Strict) {
            failures.push(format!("`{}`: the decompiled program does not run as the program runs — {diff}\n--- decompiled ---\n{reprinted}", c.name));
        }
    }
    assert!(failures.is_empty(), "{} programs decompile wrongly:\n{}", failures.len(), failures.join("\n═══\n"));
}

/// Programs that build empty collections of declared types, each with the constructions its decompiled form must print.
/// Programs whose specialization tracks an empty collection statically and lifts it back where a dynamic branch
/// writes it: the residual names the collection's declared type, so the compiled tier builds the residual too.
const LIFTED_EMPTY_CORPUS: &[(&str, &str, &[&str])] = &[
    (
        "an_empty_set_of_text_lifted_at_a_branch",
        "## Main\nLet mutable s be a new Set of Text.\nLet n be length of args().\nIf n is greater than 5:\n    Add \"b\" to s.\nShow length of s.\n",
        &["a new Set of Text"],
    ),
    (
        "an_empty_set_of_int_lifted_at_a_branch",
        "## Main\nLet mutable t be a new Set of Int.\nLet n be length of args().\nIf n is greater than 7:\n    Add 3 to t.\nShow length of t.\n",
        &["a new Set of Int"],
    ),
];

#[test]
fn a_lifted_empty_collection_keeps_its_declared_type() {
    let mut failures = Vec::new();
    for (name, program, printed_forms) in LIFTED_EMPTY_CORPUS {
        let residual = match decompile(program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("`{name}`: the specialization failed: {e}"));
                continue;
            }
        };
        for form in *printed_forms {
            if !residual.contains(form) {
                failures.push(format!("`{name}`: the residual does not name `{form}`:\n{residual}"));
            }
        }
        let source = run_treewalk(program);
        if let Some(diff) = behavior_diff(&source, &run_treewalk(&residual), CmpMode::Strict) {
            failures.push(format!("`{name}`: the residual does not run as the program runs — {diff}"));
        }
        let compiled = common::run_user_logos(&residual);
        if !compiled.success || compiled.stdout.trim() != source.output {
            failures.push(format!(
                "`{name}`: the compiled tier does not run the residual as the program runs: {}\n{residual}",
                if compiled.success { compiled.stdout } else { compiled.stderr }
            ));
        }
    }
    assert!(failures.is_empty(), "{} lifted collections lose their type:\n{}", failures.len(), failures.join("\n═══\n"));
}

/// Programs whose static temporal values, some computed by the specializer, reach a value it does not know: the
/// residual holds each as an expression of the language that evaluates to exactly that value, whatever its parts
/// below the minute, below the millisecond or below zero.
const LIFTED_TEMPORAL_CORPUS: &[(&str, &str)] = &[
    ("durations_below_a_millisecond_and_below_zero", "## Main\nLet n be length of args().\nLet mutable d be 1500 microseconds.\nLet mutable e be 0ns - 7ns.\nIf n is greater than 5:\n    Set d to d + 1 seconds.\n    Set e to e + 1 seconds.\nShow d equals 1 milliseconds + 500 microseconds.\nShow e is less than 0ns.\nShow e + 7ns equals 0ns.\nShow d + d.\n"),
    ("a_moment_between_minutes", "## Main\nLet n be length of args().\nLet mutable m be 2026-11-08 at 9:30am + 15500 milliseconds.\nIf n is greater than 5:\n    Set m to m + 1 day.\nShow format_timestamp(m).\nShow m equals 2026-11-08 at 9:30am + 15 seconds + 500 milliseconds.\n"),
    ("a_folded_date_a_time_a_span_and_a_character", "## Main\nLet n be length of args().\nLet mutable d be 2024-02-29 + 1 year.\nLet mutable t be 9:30am.\nLet mutable s be 14 months.\nLet mutable c be `\\n`.\nIf n is greater than 5:\n    Set d to d + 1 day.\n    Set t to 10am.\n    Set s to 1 day.\n    Set c to `x`.\nShow d.\nShow t.\nShow s.\nShow c equals `\\n`.\n"),
];

#[test]
fn a_lifted_temporal_value_keeps_its_exact_value() {
    let mut failures = Vec::new();
    for (name, program) in LIFTED_TEMPORAL_CORPUS {
        let residual = match decompile(program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("`{name}`: the specialization failed: {e}"));
                continue;
            }
        };
        let source = run_treewalk(program);
        if let Some(diff) = behavior_diff(&source, &run_treewalk(&residual), CmpMode::Strict) {
            failures.push(format!("`{name}`: the residual does not run as the program runs — {diff}\n{residual}"));
        }
        let compiled = common::run_user_logos(&residual);
        if !compiled.success || compiled.stdout.trim() != source.output {
            failures.push(format!(
                "`{name}`: the compiled tier does not run the residual as the program runs: {}\n{residual}",
                if compiled.success { compiled.stdout } else { compiled.stderr }
            ));
        }
    }
    assert!(failures.is_empty(), "{} lifted temporal values lose their value:\n{}", failures.len(), failures.join("\n═══\n"));
}

const TYPED_EMPTY_CORPUS: &[(&str, &str, &[&str])] = &[
    (
        "a_sequence_of_text",
        "## Main\nLet names be a new Seq of Text.\nPush \"ada\" to names.\nShow names.\n",
        &["a new Seq of Text"],
    ),
    (
        "a_set_of_int",
        "## Main\nLet seen be a new Set of Int.\nAdd 3 to seen.\nAdd 3 to seen.\nShow length of seen.\n",
        &["a new Set of Int"],
    ),
    (
        "a_map_of_text_to_real",
        "## Main\nLet weights be a new Map of Text to Real.\nSet item \"w\" of weights to 0.5.\nShow item \"w\" of weights.\n",
        &["a new Map of Text to Real"],
    ),
    (
        "a_sequence_of_sequences",
        "## Main\nLet grid be a new Seq of (Seq of Int).\nLet row be a new Seq of Int.\nPush 1 to row.\nPush row to grid.\nShow length of grid.\n",
        &["a new Seq of (Seq of Int)", "a new Seq of Int"],
    ),
    (
        "an_empty_sequence_returned",
        "## To empty -> Seq of Text:\n    Return a new Seq of Text.\n\n## Main\nShow length of empty().\n",
        &["a new Seq of Text"],
    ),
];

/// A program's Core IR names the type of every empty collection it builds, so its decompiled form is a program of the
/// language: it prints each construction with its declared types, and it runs as the source runs on the tree-walker and
/// compiled to Rust.
#[test]
fn an_empty_collection_decompiles_with_its_type() {
    use logicaffeine_compile::compile::{core_types_for_pe_source, decompile_source_text, encode_program_source, interpret_bootstrap};
    let mut failures = Vec::new();
    for (name, program, printed_forms) in TYPED_EMPTY_CORPUS {
        let encoded = encode_program_source(program).expect("the program encodes");
        let printer = format!(
            "{}\n{}\n## Main\n{encoded}\nRepeat for decompiledF in encodedFuncSeq:\n    Show decompileFunc(decompiledF).\nShow \"## Main\".\nShow decompileBlock(encodedMain, 0).\n",
            core_types_for_pe_source(),
            decompile_source_text()
        );
        let printed = match interpret_bootstrap(&printer) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("`{name}`: the decompiler failed: {e:?}"));
                continue;
            }
        };
        for form in *printed_forms {
            if !printed.contains(form) {
                failures.push(format!("`{name}`: the decompiled program does not print `{form}`:\n{printed}"));
            }
        }
        let source = run_treewalk(program);
        if let Some(diff) = behavior_diff(&source, &run_treewalk(&printed), CmpMode::Strict) {
            failures.push(format!("`{name}`: the decompiled program does not run as the program runs — {diff}"));
        }
        let compiled = common::run_user_logos(&printed);
        if !compiled.success || compiled.stdout.trim() != source.output {
            failures.push(format!(
                "`{name}`: the compiled tier does not run the decompiled program as the program runs: {}\n{printed}",
                if compiled.success { compiled.stdout } else { compiled.stderr }
            ));
        }
    }
    assert!(failures.is_empty(), "{} empty collections decompile wrongly:\n{}", failures.len(), failures.join("\n═══\n"));
}

/// Programs that return no value or hold `nothing`, each with the forms its decompiled form must print.
const NOTHING_CORPUS: &[(&str, &str, &[&str])] = &[
    (
        "a_procedure_that_returns_early",
        "## To greet (n: Int):\n    If n is less than 0:\n        Return.\n    Show n.\n\n## Main\ngreet(3).\ngreet(0 - 1).\ngreet(5).\n",
        &["Return."],
    ),
    ("a_value_that_is_nothing", "## Main\nLet x be nothing.\nShow x.\n", &["nothing"]),
];

/// A program's Core IR holds `nothing` and a `Return` of no value as themselves, so its decompiled form is a program of
/// the language: it prints them as the source wrote them, and it runs as the source runs on the tree-walker and compiled
/// to Rust.
#[test]
fn nothing_decompiles_as_itself() {
    use logicaffeine_compile::compile::{core_types_for_pe_source, decompile_source_text, encode_program_source, interpret_bootstrap};
    let mut failures = Vec::new();
    for (name, program, printed_forms) in NOTHING_CORPUS {
        let encoded = encode_program_source(program).expect("the program encodes");
        let printer = format!(
            "{}\n{}\n## Main\n{encoded}\nRepeat for decompiledF in encodedFuncSeq:\n    Show decompileFunc(decompiledF).\nShow \"## Main\".\nShow decompileBlock(encodedMain, 0).\n",
            core_types_for_pe_source(),
            decompile_source_text()
        );
        let printed = match interpret_bootstrap(&printer) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("`{name}`: the decompiler failed: {e:?}"));
                continue;
            }
        };
        for form in *printed_forms {
            if !printed.lines().any(|l| l.trim() == *form || l.trim().ends_with(&format!(" {form}."))) {
                failures.push(format!("`{name}`: the decompiled program does not print `{form}`:\n{printed}"));
            }
        }
        let source = run_treewalk(program);
        if let Some(diff) = behavior_diff(&source, &run_treewalk(&printed), CmpMode::Strict) {
            failures.push(format!("`{name}`: the decompiled program does not run as the program runs — {diff}"));
        }
        let compiled = common::run_user_logos(&printed);
        if !compiled.success || compiled.stdout.trim() != source.output {
            failures.push(format!(
                "`{name}`: the compiled tier does not run the decompiled program as the program runs: {}\n{printed}",
                if compiled.success { compiled.stdout } else { compiled.stderr }
            ));
        }
    }
    assert!(failures.is_empty(), "{} programs decompile `nothing` wrongly:\n{}", failures.len(), failures.join("\n═══\n"));
}

/// The offline specializer runs native, as the production specializer does; the tree-walker is the ground truth it is
/// held to. Every corpus program specializes to the same residual on both, byte for byte, and a specialization that
/// fails is the same failure on both, returned, never an aborted host.
#[test]
fn the_native_offline_specializer_specializes_as_the_tree_walker_does() {
    use logicaffeine_compile::compile::{mix_projection1_on, MixEngine};
    let failing = "## Main\nShow parseInt(\"seven\").\n";
    let mut programs: Vec<(&str, &str)> = [EFFECT_CORPUS, INSPECT_CORPUS, TEXTBOOK_P1_CORPUS, SEMANTICS_CORPUS, CAPTURE_CORPUS]
        .iter()
        .flat_map(|c| c.iter().map(|case| (case.name, case.program)))
        .chain(FAILURE_CORPUS.iter().map(|c| (c.name, c.program)))
        .collect();
    programs.push(("a_builtin_that_fails_while_specializing", failing));
    programs.push(("a_free_variable", "## Main\nShow input + 1.\n"));
    assert!(
        mix_projection1_on(MixEngine::Native, failing).is_err(),
        "a builtin that stops the program, computed while specializing, stops the specialization"
    );
    let mut differ = Vec::new();
    for (name, program) in &programs {
        let native = mix_projection1_on(MixEngine::Native, program);
        let tree_walker = mix_projection1_on(MixEngine::TreeWalker, program);
        if native != tree_walker {
            differ.push(format!("`{name}`:\n--- native ---\n{native:?}\n--- tree-walker ---\n{tree_walker:?}"));
        }
    }
    assert!(
        differ.is_empty(),
        "{} of {} programs specialize differently on the native engine:\n{}",
        differ.len(),
        programs.len(),
        differ.join("\n═══\n")
    );
}

/// A two-level program is a function of the program it annotates: annotating the specializer twice gives the same
/// program, its temporaries named alike, so a projection's residual never depends on how a map orders its keys —
/// the third projection's equation is byte for byte.
#[test]
fn the_binding_time_analysis_is_deterministic() {
    use logicaffeine_compile::compile::specializer_two_level;
    let first = specializer_two_level().expect("the specializer annotates").builders().expect("it prints");
    let second = specializer_two_level().expect("the specializer annotates").builders().expect("it prints");
    assert!(first == second, "two annotations of the specializer differ");
}

/// Specialization ends. A static argument the residual's data drives without bound — a count taken up a recursion
/// whose other arguments are dynamic — would ask for a new specialization at every level, so the analysis
/// generalizes it: the first call keeps its static arguments, the recursion runs on the generalized variant. A static
/// argument handed a part of the caller's own (a recursion down a static structure) stays static.
#[test]
fn a_static_argument_driven_without_bound_is_generalized() {
    use logicaffeine_compile::compile::{core_types_for_pe_source, mix_source_text, program_to_core_tree};
    use logicaffeine_compile::optimize::mix_bta::{annotate, StoreEntry};
    let catalog = format!("{}\n{}", core_types_for_pe_source(), mix_source_text());
    let variants = |src: &str, dynamic: &[&str]| -> Vec<String> {
        let tree = program_to_core_tree(src).expect("the program is in the native builder's coverage");
        let two_level = annotate(&tree, &catalog, &[], dynamic).expect("the program annotates");
        let mut names: Vec<String> = two_level
            .store_entries()
            .into_iter()
            .filter_map(|(key, entry)| match entry {
                StoreEntry::Map(items) if key == "mixBFuncs" => Some(items.into_iter().map(|(n, _)| n)),
                _ => None,
            })
            .flatten()
            .collect();
        names.sort();
        names
    };
    let counted = "## To walk (xs: Seq of Int) and (i: Int) and (depth: Int) -> Int:\n    If i is greater than (length of xs):\n        Return depth.\n    Return walk(xs, i + 1, depth + 1).\n\n## Main\nShow walk(input, 1, 0).\n";
    assert_eq!(variants(counted, &["input"]), vec!["walk_bDDD".to_string(), "walk_bDSS".to_string()]);
    let descending = "## A Tree is one of:\n    A Leaf.\n    A Node with left Tree and right Tree.\n\n## To walk (t: Tree) and (depth: Int) and (acc: Int) -> Int:\n    Inspect t:\n        When Leaf:\n            Return acc + depth.\n        When Node (l, r):\n            Return walk(l, depth + 1, acc) + walk(r, depth + 1, acc).\n\n## Main\nShow walk(a new Node with left (a new Leaf) and right (a new Node with left (a new Leaf) and right (a new Leaf)), 0, input).\n";
    assert_eq!(variants(descending, &["input"]), vec!["walk_bSSD".to_string()]);
}

/// Programs of every corpus above the executed second projection's compiler compiles to exactly their first
/// projection. May only rise.
const TEXTBOOK_P2_PROGRAMS: usize = 100;

#[test]
fn the_textbook_compiler_compiles_as_the_first_projection_specializes() {
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    let programs = every_corpus_program();
    let firsts: Vec<Result<String, String>> =
        programs.iter().map(|c| logicaffeine_compile::compile::mix_projection1(c.program)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let compiled_all: Vec<std::sync::Mutex<Option<Result<String, String>>>> =
        programs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(c) = programs.get(i) else { break };
                let compiled = logicaffeine_compile::compile::run_mix_compiler(&compiler, c.program);
                *compiled_all[i].lock().unwrap() = Some(compiled);
            });
        }
    });
    let mut failures = Vec::new();
    for ((c, first), compiled) in programs.iter().zip(firsts).zip(compiled_all) {
        let compiled = compiled.into_inner().unwrap().expect("every program was compiled");
        if compiled != first {
            failures.push(format!(
                "`{}`:\n--- compiled ---\n{}\n--- first projection ---\n{}",
                c.name,
                compiled.unwrap_or_else(|e| e),
                first.unwrap_or_else(|e| e)
            ));
        }
    }
    let agreed = programs.len() - failures.len();
    assert!(
        failures.is_empty() && agreed >= TEXTBOOK_P2_PROGRAMS,
        "{agreed} programs compile to their first projection (floor {TEXTBOOK_P2_PROGRAMS}):\n{}",
        failures.join("\n═══\n")
    );
}

/// The values the generated programs' `n` is fixed to for the offline projections, which refuse a program that reads
/// its arguments.
const GENERATED_KNOWN_INPUTS: [i64; 3] = [0, 7, 29];

/// Generated programs (seed, `n`) the native engine refuses while it specializes them: it computes a value past 2^63,
/// where its `Int` stops and the tree-walker's widens (the Rust tier's exactness, parked 10-13). The compiler, run on
/// the tree-walker, compiles each to the program; the row fails once the native engine specializes it.
const KNOWN_NATIVE_OVERFLOW: &[(u64, i64, &str)] = &[
    (2, 29, "the native engine's Int stops at 2^63 while it specializes, where the tree-walker's widens (the Rust tier's exactness, parked 10-13)"),
];

/// How the generated `program` fails the offline projections, if it does: the second projection's compiler fails on it,
/// or compiles it to a residual that prints other than the program prints or keeps interpreter dispatch, or to anything
/// but its first projection (where the native engine specializes it).
fn generated_failure(compiler: &str, seed: u64, n: i64, program: &str) -> Option<String> {
    let label = format!("seed {seed}, n = {n}");
    let compiled = match logicaffeine_compile::compile::run_mix_compiler(compiler, program) {
        Ok(c) => c,
        Err(e) => return Some(format!("{label}: the compiler failed: {e}
--- program ---
{program}")),
    };
    if let Some(diff) = behavior_diff(&run_treewalk(program), &run_treewalk(&compiled), CmpMode::Strict) {
        return Some(format!(
            "{label}: the compiled residual does not do what the program does — {diff}
--- program ---
{program}--- residual ---
{compiled}"
        ));
    }
    let d = count_dispatch(&compiled);
    if d != 0 {
        return Some(format!("{label}: the compiled residual keeps {d} unit(s) of interpreter dispatch
--- residual ---
{compiled}"));
    }
    let known = KNOWN_NATIVE_OVERFLOW.iter().any(|(s, k, _)| (*s, *k) == (seed, n));
    match (logicaffeine_compile::compile::mix_projection1(program), known) {
        (Ok(first), false) if first == compiled => None,
        (Ok(first), false) => Some(format!(
            "{label}: the compiler's output is not the first projection
--- compiled ---
{compiled}
--- first projection ---
{first}"
        )),
        (Ok(_), true) => Some(format!("{label}: the native engine specializes it now: its KNOWN_NATIVE_OVERFLOW row is stale")),
        (Err(e), true) if e.contains("does not fit a 64-bit Int") => None,
        (Err(e), _) => Some(format!("{label}: the first projection failed: {e}
--- program ---
{program}")),
    }
}

/// The arguments lock's generated programs (`common/generated_programs.rs`), their `n` fixed to each known input,
/// compile by the second projection's compiler to a residual that prints what the program prints and keeps no
/// interpreter dispatch, and that is exactly their first projection. The third projection's compiler is the second's
/// (`the_compiler_generator_generates_the_textbook_compiler`), so this holds of it too.
#[test]
fn generated_programs_compile_to_their_first_projection() {
    for (seed, n, why) in KNOWN_NATIVE_OVERFLOW {
        assert!((1..31).contains(seed) && GENERATED_KNOWN_INPUTS.contains(n), "KNOWN_NATIVE_OVERFLOW names ({seed}, {n}), no generated program");
        assert!(!why.is_empty(), "KNOWN_NATIVE_OVERFLOW ({seed}, {n}) gives no reason");
    }
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    let programs: Vec<(u64, i64, String)> = (1..31u64)
        .flat_map(|seed| {
            GENERATED_KNOWN_INPUTS.iter().map(move |&n| (seed, n, generated_programs::generated_program(seed, &n.to_string())))
        })
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let outcomes: Vec<std::sync::Mutex<Option<String>>> = programs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some((seed, n, program)) = programs.get(i) else { break };
                *outcomes[i].lock().unwrap() = generated_failure(&compiler, *seed, *n, program);
            });
        }
    });
    let failures: Vec<String> = outcomes.into_iter().filter_map(|o| o.into_inner().unwrap()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} generated programs fail the offline projections:
{}",
        failures.len(),
        programs.len(),
        failures.join("
═══
")
    );
}

/// The constructors of the two-level catalog: every form `mix` reads a two-level program in.
fn two_level_constructors() -> Vec<String> {
    let source = logicaffeine_compile::compile::mix_source_text();
    let mut ctors = Vec::new();
    let mut in_two_level = false;
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("## A ") {
            let ty = rest.split_whitespace().next().unwrap_or("");
            in_two_level = matches!(ty, "BExpr" | "BStmt" | "BArm" | "BFunc");
            continue;
        }
        if line.starts_with("## ") {
            in_two_level = false;
            continue;
        }
        if in_two_level {
            if let Some(rest) = line.trim_start().strip_prefix("A ").or_else(|| line.trim_start().strip_prefix("An ")) {
                let ctor: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                if !ctor.is_empty() {
                    ctors.push(ctor);
                }
            }
        }
    }
    ctors
}

#[test]
fn the_textbook_compiler_holds_none_of_the_specializer() {
    let ctors = two_level_constructors();
    assert!(
        ["BStatic", "BDVar", "BSIf", "BDIf", "BSBlock", "BDCallLet", "BFuncDef"].iter().all(|c| ctors.iter().any(|k| k == c)),
        "the two-level catalog lost a form this lock reads: {ctors:?}"
    );
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    let words: std::collections::HashSet<&str> =
        compiler.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    let carried: Vec<&String> = ctors.iter().filter(|c| words.contains(c.as_str())).collect();
    assert!(
        carried.is_empty(),
        "the compiler carries the two-level forms {carried:?}: it still runs the specializer over the self-interpreter"
    );
}

#[test]
fn the_compiler_generator_generates_the_textbook_compiler() {
    let cogen = logicaffeine_compile::compile::mix_projection3().expect("the third projection builds a compiler generator");
    let generated = logicaffeine_compile::compile::run_mix_cogen(&cogen).expect("the compiler generator runs");
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    assert!(
        generated == compiler,
        "the compiler generator's compiler is not the second projection's:\n--- generated ---\n{generated}\n--- second projection ---\n{compiler}"
    );
}

/// Programs of every corpus above the textbook compiler, compiled to native code, compiles to exactly their first
/// projection. May only rise.
const NATIVE_P2_PROGRAMS: usize = 100;

#[test]
fn the_textbook_compiler_compiles_to_native_code() {
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    let native = logicaffeine_compile::compile::NativeMixCompiler::build(&compiler)
        .unwrap_or_else(|e| panic!("the textbook compiler compiles to native code: {e}"));
    let programs = every_corpus_program();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let compiled_all: Vec<std::sync::Mutex<Option<Result<String, String>>>> =
        programs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(c) = programs.get(i) else { break };
                *compiled_all[i].lock().unwrap() = Some(native.compile(c.program));
            });
        }
    });
    let mut failures = Vec::new();
    for (c, compiled) in programs.iter().zip(compiled_all) {
        let compiled = compiled.into_inner().unwrap().expect("every program was compiled");
        let first = logicaffeine_compile::compile::mix_projection1(c.program);
        if compiled != first {
            failures.push(format!(
                "`{}`:\n--- compiled natively ---\n{}\n--- first projection ---\n{}",
                c.name,
                compiled.unwrap_or_else(|e| e),
                first.unwrap_or_else(|e| e)
            ));
        }
    }
    let agreed = programs.len() - failures.len();
    assert!(
        failures.is_empty() && agreed >= NATIVE_P2_PROGRAMS,
        "{agreed} programs compile natively to their first projection (floor {NATIVE_P2_PROGRAMS}):\n{}",
        failures.join("\n═══\n")
    );
}

#[test]
fn the_native_compiler_generator_generates_the_textbook_compiler() {
    let cogen = logicaffeine_compile::compile::mix_projection3().expect("the third projection builds a compiler generator");
    let generated = logicaffeine_compile::compile::run_native_mix_cogen(&cogen)
        .unwrap_or_else(|e| panic!("the compiler generator compiles to native code and runs: {e}"));
    let compiler = logicaffeine_compile::compile::mix_projection2().expect("the second projection builds a compiler");
    assert!(
        generated == compiler,
        "the native compiler generator's compiler is not the second projection's:\n--- generated ---\n{generated}\n--- second projection ---\n{compiler}"
    );
}

/// A program that stops: every line it shows before it stops, and the report it stops with.
struct Stops {
    name: &'static str,
    program: &'static str,
    shown: &'static [&'static str],
    report: &'static str,
}

const fn stops(name: &'static str, program: &'static str, shown: &'static [&'static str], report: &'static str) -> Stops {
    Stops {
        name,
        program,
        shown,
        report,
    }
}

/// Programs the language stops, each at a different kind of failure and from a different depth of blocks, loops and
/// calls.
const FAILURE_CORPUS: &[Stops] = &[
    stops(
        "a_check_its_predicate_refuses_stops",
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"guest\".\nCheck that the u is admin.\nShow \"ok\".",
        &[],
        "Security Check Failed: u is admin",
    ),
    stops(
        "a_capability_no_branch_grants_stops",
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet mallory be a new User with name \"Mallory\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that mallory can edit doc.\nShow \"edited\".",
        &[],
        "Security Check Failed: mallory can Edit the doc",
    ),
    stops("an_index_past_a_text", "## Main\nShow item 9 of \"abc\".\n", &[], "Index 9 out of bounds"),
    stops(
        "the_length_of_nothing",
        "## Main\nShow 1.\nLet x be nothing.\nShow length of x.\n",
        &["1"],
        "Cannot get length of Nothing",
    ),
    stops(
        "an_index_past_a_list_after_output",
        "## Main\nShow 1.\nLet xs be [1, 2, 3].\nShow item 4 of xs.\nShow 2.\n",
        &["1"],
        "Index 4 out of bounds",
    ),
    stops(
        "a_computed_index_zero",
        "## Main\nLet xs be [1, 2, 3].\nLet i be 3 - 3.\nShow item i of xs.\n",
        &[],
        "Index 0 out of bounds",
    ),
    stops(
        "a_division_by_a_computed_zero",
        "## Main\nLet d be 5 - 5.\nShow 1.\nShow 10 / d.\nShow 2.\n",
        &["1"],
        "Division by zero",
    ),
    stops("a_modulo_by_a_computed_zero", "## Main\nLet d be 5 - 5.\nShow 10 % d.\n", &[], "Modulo by zero"),
    stops(
        "a_fail_after_output",
        "## Main\nShow \"before\".\nFail with \"boom\".\nShow \"after\".\n",
        &["before"],
        "boom",
    ),
    stops("a_fail_with_an_int", "## Main\nFail with 42.\n", &[], "42"),
    stops(
        "a_fail_in_a_function",
        "## To checked (n: Int) -> Int:\n    If n is less than 0:\n        Fail with \"negative: \" + format(n).\n    Return n.\n\n## Main\nShow checked(3).\nShow checked(0 - 2).\nShow \"unreached\".\n",
        &["3"],
        "negative: -2",
    ),
    stops(
        "a_fail_in_a_counted_loop",
        "## Main\nRepeat for i from 1 to 5:\n    Show i.\n    If i equals 3:\n        Fail with \"stopped at \" + format(i).\nShow \"after\".\n",
        &["1", "2", "3"],
        "stopped at 3",
    ),
    stops(
        "a_fail_in_a_while_loop",
        "## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\n    If i equals 4:\n        Fail with \"four\".\n    Show i.\n",
        &["1", "2", "3"],
        "four",
    ),
    stops(
        "a_fail_two_calls_deep",
        "## To inner (n: Int) -> Int:\n    If n equals 2:\n        Fail with \"inner failed\".\n    Return n.\n\n## To outer (n: Int) -> Int:\n    Let a be inner(n).\n    Show \"outer got \" + format(a).\n    Return a + 1.\n\n## Main\nShow outer(1).\nShow outer(2).\nShow \"unreached\".\n",
        &["outer got 1", "2"],
        "inner failed",
    ),
    stops(
        "a_fail_in_an_inspect_arm",
        "## A Shape is one of:\n    A Circle with r Int.\n    A Square with s Int.\n\n## Main\nLet sh be a new Square with s 4.\nInspect sh:\n    When Circle (r):\n        Show r.\n    When Square (s):\n        Show s.\n        Fail with \"square \" + format(s).\nShow \"after\".\n",
        &["4"],
        "square 4",
    ),
    stops(
        "a_broken_requirement_with_its_reason",
        "## To withdraw (balance: Int, amount: Int) -> Int:\n    Require that amount is at most balance, otherwise \"cannot withdraw \" + format(amount) + \" from \" + format(balance).\n    Return balance - amount.\n\n## Main\nShow withdraw(10, 3).\nShow withdraw(10, 30).\n",
        &["7"],
        "cannot withdraw 30 from 10",
    ),
    stops(
        "a_broken_assertion",
        "## Main\nLet x be 3.\nShow x.\nAssert that x is greater than 5.\nShow \"after\".\n",
        &["3"],
        "Assertion failed",
    ),
    stops(
        "a_broken_assumption_with_its_reason",
        "## Main\nLet t be 200.\nAssume t is at most 100, otherwise \"t out of range: \" + format(t).\nShow t.\n",
        &[],
        "t out of range: 200",
    ),
    stops(
        "a_pop_from_an_empty_list",
        "## Main\nLet mutable xs be [1].\nPop from xs.\nPop from xs.\nShow length of xs.\n",
        &[],
        "Cannot pop from xs: it is empty",
    ),
    stops(
        "a_missing_map_key",
        "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nShow item \"a\" of m.\nShow item \"b\" of m.\n",
        &["1"],
        "Key 'b' not found in map",
    ),
    stops(
        "a_name_read_after_its_if_block",
        "## Main\nLet c be 3.\nIf c is greater than 1:\n    Let inner be 7.\n    Show inner.\nShow inner.\n",
        &["7"],
        "Undefined variable: inner",
    ),
    stops(
        "a_name_read_after_its_while_block",
        "## Main\nLet mutable i be 0.\nWhile i is less than 2:\n    Let step be i.\n    Set i to i + 1.\nShow step.\n",
        &[],
        "Undefined variable: step",
    ),
    stops(
        "a_failing_operand_of_a_condition",
        "## Main\nLet xs be [1, 2].\nIf item 5 of xs is greater than 0:\n    Show \"yes\".\nShow \"after\".\n",
        &[],
        "Index 5 out of bounds",
    ),
    stops(
        "a_failing_argument_stops_before_the_call",
        "## To twice (n: Int) -> Int:\n    Show \"called\".\n    Return n * 2.\n\n## Main\nLet xs be [1].\nShow twice(item 3 of xs).\n",
        &[],
        "Index 3 out of bounds",
    ),
    stops(
        "a_failing_value_is_never_pushed",
        "## Main\nLet mutable ys be [1].\nLet xs be [1].\nPush item 2 of xs to ys.\nShow ys.\n",
        &[],
        "Index 2 out of bounds",
    ),
    stops(
        "texts_do_not_order",
        "## Main\nLet a be \"apple\".\nLet b be \"banana\".\nIf a is less than b:\n    Show \"lt\".\n",
        &[],
        "Cannot compare Text and Text",
    ),
    stops(
        "a_call_to_no_function",
        "## Main\nShow 1.\nShow frobnicate(3).\nShow 2.\n",
        &["1"],
        "Unknown function: frobnicate",
    ),
    stops("a_call_through_a_value", "## Main\nLet x be 5.\nShow 1.\nShow x(3).\n", &["1"], "Unknown function: x"),
    stops(
        "a_list_indexed_by_text",
        "## Main\nLet xs be [1, 2].\nShow 1.\nShow item \"a\" of xs.\n",
        &["1"],
        "Cannot index List with Text",
    ),
    stops("an_int_indexed", "## Main\nLet n be 5.\nShow item 1 of n.\n", &[], "Cannot index Int with Int"),
    stops("a_field_of_an_int", "## Main\nLet n be 5.\nShow n's value.\n", &[], "Cannot access field on Int"),
    stops(
        "a_slice_of_a_text",
        "## Main\nLet s be \"abcdef\".\nShow items 2 through 4 of s.\n",
        &[],
        "Slice requires List and Int indices",
    ),
    stops(
        "a_union_of_lists",
        "## Main\nLet a be [1, 2].\nLet b be [2, 3].\nShow a union b.\n",
        &[],
        "Cannot union List and List",
    ),
    stops(
        "an_intersection_of_lists",
        "## Main\nLet a be [1, 2].\nLet b be [2, 3].\nShow a intersection b.\n",
        &[],
        "Cannot intersect List and List",
    ),
    stops(
        "an_escape_to_rust",
        "## Main\nShow 1.\nEscape to Rust:\n    println!(\"hi\");\nShow 2.\n",
        &["1"],
        logicaffeine_compile::semantics::escape::REFUSAL,
    ),
    stops("a_push_to_an_int", "## Main\nLet mutable n be 5.\nPush 1 to n.\nShow n.\n", &[], "Can only push to a List"),
    stops("an_item_set_in_an_int", "## Main\nLet mutable n be 5.\nSet item 1 of n to 2.\nShow n.\n", &[], "Cannot index into Int"),
    stops(
        "a_list_item_set_by_text",
        "## Main\nLet mutable xs be [1, 2].\nSet item \"a\" of xs to 2.\nShow xs.\n",
        &[],
        "List index must be an integer",
    ),
    stops("a_pop_from_a_text", "## Main\nLet mutable s be \"abc\".\nPop from s.\nShow s.\n", &[], "Can only pop from a List"),
    stops("a_pop_from_a_bool", "## Main\nLet mutable b be true.\nPop from b.\nShow b.\n", &[], "Cannot get length of Bool"),
    stops("an_add_to_a_list", "## Main\nLet mutable xs be [1].\nAdd 2 to xs.\nShow xs.\n", &[], "Can only add to a Set"),
    stops(
        "a_remove_from_a_list",
        "## Main\nLet mutable xs be [1, 2].\nRemove 2 from xs.\nShow xs.\n",
        &[],
        "Can only remove from a Set or Map",
    ),
    stops(
        "a_field_set_in_an_int",
        "## Main\nLet mutable n be 5.\nSet n's value to 2.\nShow n.\n",
        &[],
        "Cannot set field on non-struct value",
    ),
    stops("an_append_to_a_text", "## Main\nLet mutable s be \"ab\".\nAppend \"x\" to s.\nShow s.\n", &[], "Cannot append to Text"),
    stops(
        "an_inspect_with_no_arm_for_its_value",
        "## A Shape is one of:\n    A Circle with r Int.\n    A Square with s Int.\n\n## Main\nLet sh be a new Square with s 4.\nShow 1.\nInspect sh:\n    When Circle (r):\n        Show r.\nShow \"after\".\n",
        &["1"],
        "Inspect has no arm for `Square` (a `Shape`) and no Otherwise (matches must be exhaustive) — add a `When Square (…):` arm, or an `Otherwise:` to cover it.",
    ),
    stops("a_broken_assertion_of_a_number", "## Main\nLet n be 0.\nAssert that n.\nShow \"after\".\n", &[], "Assertion failed"),
    stops(
        "a_duration_times_a_number",
        "## Main\nShow 1.\nShow 5 seconds * 3.",
        &["1"],
        "Cannot multiply Duration and Int",
    ),
    stops(
        "a_date_plus_a_duration",
        "## Main\nLet d be today.\nShow d + 5 seconds.",
        &[],
        "Cannot add Date and Duration",
    ),
    stops(
        "the_hour_of_a_date",
        "## Main\nShow hour_of(2024-03-10).",
        &[],
        "a Date has no time-of-day — use a timestamp/Moment",
    ),
    stops(
        "a_timestamp_that_is_none",
        "## Main\nShow parse_timestamp(\"noon\").",
        &[],
        "Cannot parse 'noon' as an RFC 3339 timestamp",
    ),
    stops(
        "a_moment_minus_a_moment",
        "## Main\nShow 2026-11-08 at 9:30am - 2026-11-08 at 9am.",
        &[],
        "Cannot subtract Moment from Moment",
    ),
    stops(
        "a_sleep_on_text",
        "## Main\nShow 1.\nSleep \"soon\".\nShow 2.",
        &["1"],
        "Sleep requires Duration or Int, got Text",
    ),
    stops(
        "a_date_formatted_as_a_timestamp",
        "## Main\nShow format_timestamp(2024-03-10).",
        &[],
        "format_timestamp() requires a Moment, got Date",
    ),
    stops(
        "a_time_plus_a_duration",
        "## Main\nShow 9:30am + 5 minutes.",
        &[],
        "Cannot add Time and Duration",
    ),
    stops(
        "a_character_plus_a_number",
        "## Main\nShow `a` + 1.",
        &[],
        "Cannot add Char and Int",
    ),
    stops(
        "a_span_plus_a_date",
        "## Main\nLet d be 2024-02-29.\nShow d + 1 year.\nShow 1 year + d.",
        &["2025-02-28"],
        "Cannot add Span and Date",
    ),
];

/// Programs of [`FAILURE_CORPUS`] each engine stops where the language does. May only rise.
const FAILURE_PROGRAMS: usize = 56;

/// What a run showed and the report it ended with, as `(lines, report)`.
type Run = (Vec<String>, Option<String>);

fn lines_of(result: logicaffeine_compile::interpreter::InterpreterResult) -> Run {
    (result.lines, result.error)
}

/// Every program of the failure corpus `run` does not stop as the language does: a different line shown, a different
/// report, or no stop at all.
fn stop_failures(run: impl Fn(&str) -> Result<Run, String>) -> Vec<String> {
    let mut failures = Vec::new();
    for c in FAILURE_CORPUS {
        let want: Run = (c.shown.iter().map(|l| l.to_string()).collect(), Some(c.report.to_string()));
        match run(c.program) {
            Err(why) => failures.push(format!("`{}`: no run: {why}", c.name)),
            Ok(got) if got != want => failures.push(format!("`{}`: ran {got:?}, the language {want:?}", c.name)),
            Ok(_) => {}
        }
    }
    failures
}

fn assert_stops(label: &str, run: impl Fn(&str) -> Result<Run, String>) {
    assert!(
        FAILURE_CORPUS.len() >= FAILURE_PROGRAMS,
        "the failure corpus shrank to {} below its floor {FAILURE_PROGRAMS}",
        FAILURE_CORPUS.len()
    );
    let failures = stop_failures(run);
    assert!(
        failures.is_empty(),
        "{} of {} programs do not stop on {label} as the language stops them:\n{}",
        failures.len(),
        FAILURE_CORPUS.len(),
        failures.join("\n")
    );
}

#[test]
fn the_language_stops_these_programs() {
    assert_stops("the tree-walker", |p| Ok(lines_of(logicaffeine_compile::compile::tw_lines(p))));
}

#[test]
fn the_self_interpreter_stops_where_the_language_stops() {
    assert_stops("the self-interpreter", |p| Ok(lines_of(logicaffeine_compile::compile::self_interpreter_lines(p, &[]))));
}

/// A residual's run, held to the language's, and its dispatch to none.
fn residual_run(residual: Result<String, String>) -> Result<Run, String> {
    let residual = residual?;
    let d = count_dispatch(&residual);
    if d != 0 {
        return Err(format!("the residual keeps {d} unit(s) of interpreter dispatch\n--- residual ---\n{residual}"));
    }
    Ok(lines_of(logicaffeine_compile::compile::tw_lines(&residual)))
}

#[test]
fn the_offline_first_projection_stops_where_the_language_stops() {
    assert_stops("the offline first projection", |p| residual_run(logicaffeine_compile::compile::mix_projection1(p)));
}

#[test]
fn the_textbook_first_projection_stops_where_the_language_stops() {
    assert_stops("the textbook first projection", |p| residual_run(logicaffeine_compile::compile::textbook_projection1(p)));
}

/// Programs the language runs and the self-interpreter does not: each reaches a value of a family the self-interpreter
/// carries none of yet, and the self-interpreter stops there with what it awaits, after the lines before it. The first
/// projections stop where the self-interpreter stops; the direct specialization keeps the construct as the program
/// spelled it, so its residual runs as the language runs. A program leaves this corpus for the semantics corpus when
/// its family lands.
const AWAITS_CORPUS: &[Stops] = &[
    stops(
        "a_left_out_map_replica_field",
        "## Definition\nA Stock is Shared and has:\n    counts: a SharedMap of Text to Int.\n## Main\n    Show 1.\n    Let mutable s be a new Stock.\n    Show 2.",
        &["1"],
        "the self-interpreter holds no `SharedMap`: it awaits the CRDT family",
    ),
    stops(
        "a_decimal",
        "## Main\nShow 1.\nShow decimal(\"1.5\").",
        &["1"],
        "the self-interpreter applies no `decimal`: it awaits a Decimal",
    ),
    stops(
        "a_uuid",
        "## Main\nShow uuid_nil().",
        &[],
        "the self-interpreter applies no `uuid_nil`: it awaits a Uuid",
    ),
    stops(
        "money",
        "## Main\nShow money(10, \"USD\").",
        &[],
        "the self-interpreter applies no `money`: it awaits Money",
    ),
    stops(
        "a_complex_number",
        "## Main\nShow complex(0, 1).",
        &[],
        "the self-interpreter applies no `complex`: it awaits a Complex",
    ),
    stops(
        "a_quantity",
        "## Main\nShow quantity(2, \"inch\").",
        &[],
        "the self-interpreter applies no `quantity`: it awaits a Quantity",
    ),
    stops(
        "a_modular_number",
        "## Main\nShow modular(10, 7).",
        &[],
        "the self-interpreter applies no `modular`: it awaits a Modular",
    ),
    stops(
        "a_left_out_set_replica_field",
        "## Definition\nA Party is Shared and has:\n    guests: a SharedSet of Text.\n## Main\n    Show 1.\n    Let mutable p be a new Party.\n    Show 2.",
        &["1"],
        "the self-interpreter holds no `SharedSet`: it awaits the CRDT family",
    ),
];

/// Programs of the awaits corpus. May only rise until the families land, then each moves to the semantics corpus.
const AWAITS_PROGRAMS: usize = 8;

fn assert_awaits(label: &str, run: impl Fn(&str) -> Result<Run, String>) {
    assert!(
        AWAITS_CORPUS.len() >= AWAITS_PROGRAMS,
        "the awaits corpus shrank to {} below its floor {AWAITS_PROGRAMS}",
        AWAITS_CORPUS.len()
    );
    let mut failures = Vec::new();
    for c in AWAITS_CORPUS {
        let want: Run = (c.shown.iter().map(|l| l.to_string()).collect(), Some(c.report.to_string()));
        match run(c.program) {
            Err(why) => failures.push(format!("`{}`: no run: {why}", c.name)),
            Ok(got) if got != want => failures.push(format!("`{}`: ran {got:?}, awaiting {want:?}", c.name)),
            Ok(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} programs do not stop on {label} where the self-interpreter awaits:\n{}",
        failures.len(),
        AWAITS_CORPUS.len(),
        failures.join("\n")
    );
}

#[test]
fn the_language_runs_what_the_self_interpreter_awaits() {
    for c in AWAITS_CORPUS {
        let (_, error) = lines_of(logicaffeine_compile::compile::tw_lines(c.program));
        assert_eq!(error, None, "`{}`: the language runs it — the corpus entry is wrong, not the self-interpreter", c.name);
    }
}

#[test]
fn the_self_interpreter_stops_on_what_it_awaits() {
    assert_awaits("the self-interpreter", |p| Ok(lines_of(logicaffeine_compile::compile::self_interpreter_lines(p, &[]))));
}

#[test]
fn the_textbook_first_projection_stops_on_what_the_self_interpreter_awaits() {
    assert_awaits("the textbook first projection", |p| residual_run(logicaffeine_compile::compile::textbook_projection1(p)));
}

#[test]
fn the_offline_first_projection_stops_on_what_the_self_interpreter_awaits() {
    assert_awaits("the offline first projection", |p| residual_run(logicaffeine_compile::compile::mix_projection1(p)));
}

#[test]
fn a_specialization_keeps_what_the_self_interpreter_awaits() {
    let mut failures = Vec::new();
    for c in AWAITS_CORPUS {
        let residual = match decompile(c.program) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("`{}`: the specialization failed: {e}", c.name));
                continue;
            }
        };
        let want = lines_of(logicaffeine_compile::compile::tw_lines(c.program));
        let got = lines_of(logicaffeine_compile::compile::tw_lines(&residual));
        if got != want {
            failures.push(format!("`{}`: the residual runs {got:?}, the language {want:?}\n--- residual ---\n{residual}", c.name));
        }
    }
    assert!(failures.is_empty(), "{} residuals do not run as the language runs:\n{}", failures.len(), failures.join("\n═══\n"));
}
