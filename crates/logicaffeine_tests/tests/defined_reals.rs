//! Real numbers a program defines by a function: `the real number whose digits in base 3 are f`
//! (`f(i)` the digit in place `i`, worth `3^-i`) and `the real number approximated by f` (`f(n)`
//! within `1/2^n` of the number). Each is an `ExactReal`: it reads its function only as far as a
//! comparison, a floor or a printed digit needs, on every tier the same way, and the function it
//! reads must read nothing but its argument and do nothing but answer.

mod common;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(300);

fn assert_all_tiers(source: &str, expected: &str) {
    common::assert_all_tiers_within(source, expected, DEADLINE)
}

/// The program is refused before it runs: its first statement shows `ran`, and it prints nothing and stops naming
/// `needle`.
fn refused_before_it_runs(source: &str, needle: &str) {
    let out = common::run_all_tiers_within(source, DEADLINE);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected a refusal containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "{err}");
    assert_eq!(out.output.trim(), "", "the program ran before it was refused with {err}");
}

fn fails_with(source: &str, needle: &str) {
    let out = common::run_all_tiers_within(source, DEADLINE);
    let err = out
        .error
        .unwrap_or_else(|| panic!("expected an error containing {needle:?}; the program printed {:?}", out.output));
    assert!(err.contains(needle), "{err}");
}

/// The digits of 1/7 in base 10: `0.142857…`, digit 0 the units.
const SEVENTHS: &str = "## To seventh (i: Int) -> Int:
    If i = 0:
        Return 0.
    Return item ((i - 1) % 6 + 1) of [1, 4, 2, 8, 5, 7].
";

/// The square root of two by bisection: the lower end after `n + 1` halvings of `[1, 2]`.
const ROOT_TWO: &str = "## To rootTwoWithin (n: Int) -> Rational:
    Let lo: Rational be 1.
    Let hi: Rational be 2.
    Let k be 0.
    While k < n + 1:
        Let mid: Rational be (lo + hi) / 2.
        If mid * mid <= 2:
            Set lo to mid.
        Otherwise:
            Set hi to mid.
        Set k to k + 1.
    Return lo.
";

// ── digits ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_real_number_written_by_its_digits_prints_and_orders_as_that_number() {
    assert_all_tiers(
        &format!(
            "{SEVENTHS}
## Main
Let r be the real number whose digits in base 10 are seventh.
Let seventh: Rational be 1 / 7.
Let tiny: Rational be 1 / 10 ** 40.
Let almost: Rational be 999 / 1000.
Show r.
Show r > seventh - tiny.
Show r < seventh + tiny.
Show floor(r * 1000000).
Show r * 7 > almost."
        ),
        "0.142857142857142857142857142857\ntrue\ntrue\n142857\ntrue",
    );
}

#[test]
fn a_digit_outside_its_base_stops_the_program_naming_the_digit() {
    fails_with(
        "## To bits (i: Int) -> Int:
    If i = 2:
        Return 3.
    Return 1.

## Main
Let r be the real number whose digits in base 2 are bits.
Show \"before\".
Show r > 1.",
        "digit 2 of the real number whose digits in base 2 are bits is 3: a digit in base 2 is a whole number from 0 to 1",
    );
}

#[test]
fn a_base_below_two_is_refused_before_the_program_runs() {
    refused_before_it_runs(
        &format!(
            "{SEVENTHS}
## Main
Show \"ran\".
Let r be the real number whose digits in base 1 are seventh.
Show r."
        ),
        "a real number's digits are in a base from 2 up",
    );
}

// ── approximations ────────────────────────────────────────────────────────────────────────────

#[test]
fn a_real_number_given_by_its_approximations_is_that_number_to_every_digit() {
    assert_all_tiers(
        &format!(
            "{ROOT_TWO}
## Main
Let r be the real number approximated by rootTwoWithin.
Let below: Rational be 1414 / 1000.
Let above: Rational be 1415 / 1000.
Show r.
Show r > below.
Show r < above.
Show floor(r * 1000000).
Show r * 3.
Show r * r."
        ),
        "1.41421356237309504880168872421\ntrue\ntrue\n1414213\n4.24264068711928514640506617263\n2.00000000000000000000000000000",
    );
}

#[test]
fn approximations_that_disagree_stop_the_program_naming_both() {
    fails_with(
        "## To jumpy (n: Int) -> Int:
    If n < 40:
        Return 0.
    Return 1.

## Main
Let r be the real number approximated by jumpy.
Let tiny: Rational be 1 / 2 ** 60.
Show \"before\".
Show r > tiny.",
        "jumpy approximates no one real number",
    );
}

#[test]
fn an_answer_that_is_no_rational_stops_the_program_naming_it() {
    fails_with(
        "## To halves (n: Int) -> Real:
    Return 0.5.

## Main
Let r be the real number approximated by halves.
Show r > 0.",
        "halves answers Real",
    );
}

// ── what a defined real can do ────────────────────────────────────────────────────────────────

#[test]
fn a_defined_real_travels_through_functions_lists_and_records() {
    assert_all_tiers(
        &format!(
            "{ROOT_TWO}
{SEVENTHS}
## A Pair has:
    a left, which is ExactReal.

## To twice (x: ExactReal) -> ExactReal:
    Return x * 2.

## Main
Let r be the real number approximated by rootTwoWithin.
Let s be the real number whose digits in base 10 are seventh.
Let xs be [r, s].
Show twice(item 2 of xs).
Let p be a new Pair with left r.
Show floor(p's left * 100).
Show r > s."
        ),
        "0.285714285714285714285714285714\n141\ntrue",
    );
}

#[test]
fn a_defined_real_equal_to_another_number_cannot_be_told_from_it_and_says_so() {
    fails_with(
        &format!(
            "{SEVENTHS}
## Main
Let r be the real number whose digits in base 10 are seventh.
Let seventh: Rational be 1 / 7.
Show r = seventh."
        ),
        "cannot separate the real number whose digits in base 10 are seventh",
    );
}

// ── the function a real number is defined by ──────────────────────────────────────────────────

#[test]
fn a_function_that_reads_a_program_variable_is_refused_before_the_program_runs() {
    refused_before_it_runs(
        "## To scaled (i: Int) -> Int:
    Return i * k.

## Main
Show \"ran\".
Let k be 0.
Let r be the real number whose digits in base 2 are scaled.
Show r.",
        "scaled reads `k`",
    );
}

#[test]
fn a_function_that_does_more_than_answer_is_refused_before_the_program_runs() {
    refused_before_it_runs(
        "## To noisy (i: Int) -> Int:
    Show i.
    Return 0.

## Main
Show \"ran\".
Let r be the real number whose digits in base 2 are noisy.
Show r.",
        "noisy does more than answer",
    );
}

#[test]
fn a_function_of_another_shape_is_refused_before_the_program_runs() {
    refused_before_it_runs(
        "## To pairDigit (i: Int) and (j: Int) -> Int:
    Return 0.

## Main
Show \"ran\".
Let r be the real number whose digits in base 2 are pairDigit.
Show r.",
        "pairDigit takes one Int",
    );
    refused_before_it_runs(
        "## Main
Show \"ran\".
Let r be the real number whose digits in base 2 are nowhere.
Show r.",
        "nowhere",
    );
}

#[test]
fn a_defined_real_written_as_a_call_in_any_other_form_is_refused_before_the_program_runs() {
    let refused = "definedReal is written `the real number whose digits in base 10 are f`";
    for call in [
        "definedReal(\"digits\", 10, \"seventh\", (n: Int) -> 5)",
        "definedReal(\"digits\", 10, \"seventh\", (n: Int) -> seventh(n + 1))",
        "definedReal(\"digits\", 10, \"seventh\", (n: Int) -> seventh(k))",
        "definedReal(\"digits\", 10, \"other\", (n: Int) -> seventh(n))",
        "definedReal(\"digits\", 10, \"seventh\", (n: Text) -> seventh(n))",
        "definedReal(\"pixels\", 10, \"seventh\", (n: Int) -> seventh(n))",
        "definedReal(name, 10, \"seventh\", (n: Int) -> seventh(n))",
        "definedReal(\"digits\", k, \"seventh\", (n: Int) -> seventh(n))",
        "definedReal(\"digits\", 10, \"seventh\", f)",
    ] {
        let source = format!(
            "{SEVENTHS}
## Main
Show \"ran\".
Let k be 3.
Let name be \"digits\".
Let f be (n: Int) -> seventh(n).
Let r be {call}.
Show r."
        );
        let out = common::run_all_tiers_within(&source, DEADLINE);
        let err = out
            .error
            .unwrap_or_else(|| panic!("{call} ran and printed {:?}", out.output));
        assert!(err.contains(refused), "{call}: {err}");
        assert_eq!(out.output.trim(), "", "{call} ran before it was refused");
    }
}

#[test]
fn a_defined_real_written_as_its_phrase_s_call_is_that_real() {
    assert_all_tiers(
        &format!(
            "{SEVENTHS}
## Main
Let r be definedReal(\"digits\", 10, \"seventh\", (i: Int) -> seventh(i)).
Show floor(r * 1000000)."
        ),
        "142857",
    );
}

#[test]
fn a_base_the_call_writes_below_two_is_refused_before_the_program_runs() {
    refused_before_it_runs(
        &format!(
            "{SEVENTHS}
## Main
Show \"ran\".
Let r be definedReal(\"digits\", 1, \"seventh\", (i: Int) -> seventh(i)).
Show r."
        ),
        "a real number's digits are in a base from 2 up, got 1",
    );
}

// ── where a defined real may go ───────────────────────────────────────────────────────────────

/// What a program stops with when it hands a task a value holding a real number a function of the program defines.
const STAYS: &str = "cannot send value through a channel: NotSendable(\"a real number a function of the program defines\")";

/// A concurrent program on the tree-walker and on the VM's scheduler: the lines both print and the error both stop
/// with, which must agree.
fn concurrently(source: &str) -> (Vec<String>, Option<String>) {
    let tw = futures::executor::block_on(logicaffeine_compile::interpret_for_ui(source));
    let vm = logicaffeine_compile::run_vm_concurrent(source);
    assert_eq!(
        (&tw.lines, &tw.error),
        (&vm.lines, &vm.error),
        "the tree-walker and the VM differ on:\n{source}"
    );
    (tw.lines, tw.error)
}

#[test]
fn a_defined_real_handed_to_a_task_stays_with_its_program() {
    for (looks_at, launch) in [
        ("x: ExactReal", "Launch a task to look with r."),
        ("xs: Seq of ExactReal", "Let h be Launch a task to look with [r, r * 2]."),
        ("x: Hyperreal", "Let dt be an infinitesimal.\nLet g: Hyperreal be dt + r.\nLaunch a task to look with g."),
    ] {
        let source = format!(
            "{SEVENTHS}
## To look ({looks_at}):
    Show \"looked\".

## Main
Let r be the real number whose digits in base 10 are seventh.
Show \"before\".
{launch}
Show \"after\"."
        );
        let (lines, error) = concurrently(&source);
        assert_eq!(lines, vec!["before"], "{launch}");
        assert_eq!(error.as_deref(), Some(STAYS), "{launch}");
    }
}

#[test]
fn a_defined_real_sent_through_a_pipe_stays_with_its_program() {
    let (lines, error) = concurrently(&format!(
        "{SEVENTHS}
## Main
Let r be the real number whose digits in base 10 are seventh.
Let ch be a Pipe of ExactReal.
Show \"before\".
Send r into ch.
Show \"after\"."
    ));
    assert_eq!(lines, vec!["before"]);
    assert_eq!(error.as_deref(), Some(STAYS));
}

/// Tasks that each define a real number and read it across their waits read their own, however the scheduler
/// interleaves them; a task handed a real of exact forms reads it.
#[test]
fn tasks_that_define_reals_read_their_own_however_they_interleave() {
    let (mut lines, error) = concurrently(&format!(
        "{SEVENTHS}
## To worker (k: Int):
    Let r be the real number whose digits in base 10 are seventh.
    Sleep 1.
    Show floor(r * k).
    Sleep 1.
    Show floor(r * k * 10).

## To look (x: ExactReal):
    Show x * 2.

## Main
Launch a task to worker with 1000.
Launch a task to worker with 100000.
Launch a task to worker with 10.
Let p: ExactReal be pi.
Launch a task to look with p.
Sleep 5.
Show \"done\"."
    ));
    assert_eq!(error, None);
    lines.sort();
    assert_eq!(lines, vec!["1", "14", "142", "1428", "14285", "142857", "2·π", "done"]);
}

#[test]
fn a_parallel_block_reads_a_defined_real() {
    assert_all_tiers(
        &format!(
            "{SEVENTHS}
## Main
Let r be the real number whose digits in base 10 are seventh.
Simultaneously:
    Show floor(r * 1000).
Attempt all of the following:
    Show floor(r * 10000)."
        ),
        "142\n1428",
    );
}
