//! The AST C backend — `compile_to_c`, the C `largo emit c` writes — runs a program exactly as the VM does:
//! the same bytes on standard output, the same bytes on standard error, and the same ending.
//!
//! Every row is one program. Its reference is the VM's own run, taken in-process through the driver the
//! program needs (plain, the scheduler, or the async host) under the fixed clock, and pinned here: a VM that
//! drifts from its pin fails before any C is compared with it. The C is built as a release C build builds it
//! and run in a directory of its own, with the clock pinned to the same instant, and its two streams and its
//! exit status are compared with the reference byte for byte. Nothing is trimmed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_ast_vm_parity_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

#[path = "gauntlet/mod.rs"]
mod gauntlet;

use std::process::{Command, Stdio};

use logicaffeine_compile::compile::{compile_to_c, vm_outcome};

/// `prog.c` built into `prog` in `dir` as a release C build builds it.
fn build_release(dir: &std::path::Path) -> std::process::Output {
    common::build_c_release(&dir.join("prog.c"), &dir.join("prog")).expect("the C compiler starts")
}

const FIXED_DAYS: i64 = 20000;
const FIXED_NANOS: i64 = 20000 * 86400 * 1_000_000_000 + 45_296 * 1_000_000_000;

/// How a run ended: every statement ran, a failure stopped it (exit status 1), or anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    Finished,
    Failed,
    Abnormal(String),
}

/// Everything a run shows: both streams whole, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    stdout: String,
    stderr: String,
    end: End,
}

impl Observed {
    /// An in-process run: each shown line and a newline, then a failure's reason and a newline on stderr.
    fn of_lines(lines: &[String], failure: Option<&str>) -> Self {
        let stdout: String = lines.iter().map(|l| format!("{l}\n")).collect();
        Self::of_stream(stdout, failure)
    }

    fn of_stream(stdout: String, failure: Option<&str>) -> Self {
        match failure {
            None => Observed {
                stdout,
                stderr: String::new(),
                end: End::Finished,
            },
            Some(why) => Observed {
                stdout,
                stderr: format!("{why}\n"),
                end: End::Failed,
            },
        }
    }
}

/// The VM's run of `src`, through the driver the program needs.
fn vm(src: &str) -> Observed {
    use logicaffeine_compile::semantics::temporal::{clear_fixed_clock, set_fixed_clock};
    enum Driver {
        Plain,
        Scheduler,
        Async,
    }
    let driver =
        logicaffeine_compile::ui_bridge::with_parsed_program(
            src,
            |parsed, interner| match parsed {
                Ok((stmts, _, _)) if logicaffeine_compile::concurrency::uses_scheduler(stmts) => {
                    Driver::Scheduler
                }
                Ok((stmts, _, _))
                    if logicaffeine_compile::interpreter::needs_async(stmts, interner) =>
                {
                    Driver::Async
                }
                _ => Driver::Plain,
            },
        );
    set_fixed_clock(FIXED_DAYS as i32, FIXED_NANOS);
    let observed = match driver {
        Driver::Plain => {
            let r = vm_outcome(src);
            Observed::of_stream(r.output, r.error.as_deref())
        }
        Driver::Scheduler => {
            let r = logicaffeine_compile::ui_bridge::run_vm_concurrent(src);
            Observed::of_lines(&r.lines, r.error.as_deref())
        }
        Driver::Async => {
            let r =
                futures::executor::block_on(logicaffeine_compile::ui_bridge::run_vm_net_async(src));
            Observed::of_lines(&r.lines, r.error.as_deref())
        }
    };
    clear_fixed_clock();
    observed
}

/// `src` lowered to C, built as a release C build builds it and run in a directory of its own under the fixed clock,
/// for at most a minute.
fn c(name: &str, src: &str) -> Result<Observed, String> {
    run_c(name, src, None)
}

/// [`c`], the program's address space capped at `cap_mb` megabytes: a program whose memory grows faster than its
/// work fails here.
fn c_capped(name: &str, src: &str, cap_mb: u64) -> Result<Observed, String> {
    run_c(name, src, Some(cap_mb))
}

fn run_c(name: &str, src: &str, cap_mb: Option<u64>) -> Result<Observed, String> {
    let code = compile_to_c(src).map_err(|e| format!("`compile_to_c` refused it: {e:?}"))?;
    let dir = tempfile::Builder::new()
        .prefix(&format!("logos_cast_{name}_"))
        .tempdir()
        .expect("a run directory");
    std::fs::write(dir.path().join("prog.c"), &code).expect("write the C");
    let built = build_release(dir.path());
    if !built.status.success() {
        return Err(format!(
            "the emitted C does not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ));
    }
    let limit = match cap_mb {
        Some(mb) => format!("ulimit -v {} && ", mb * 1024),
        None => String::new(),
    };
    let run = Command::new("sh")
        .arg("-c")
        .arg(format!("{limit}exec timeout 60 ./prog"))
        .current_dir(dir.path())
        .env("LOGOS_FIXED_DAYS", FIXED_DAYS.to_string())
        .env("LOGOS_FIXED_NANOS", FIXED_NANOS.to_string())
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("the program starts");
    let end = match run.status.code() {
        Some(0) => End::Finished,
        Some(1) => End::Failed,
        Some(code) => End::Abnormal(format!("exit status {code}")),
        None => End::Abnormal("killed by a signal".to_string()),
    };
    Ok(Observed {
        stdout: String::from_utf8_lossy(&run.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
        end,
    })
}

/// The VM runs `src` to `stdout`, ending as `failure` says (a reason, or `None` for a finished run), and the
/// C runs it to exactly the same.
fn holds(name: &str, src: &str, stdout: &str, failure: Option<&str>) {
    let pinned = Observed::of_stream(stdout.to_string(), failure);
    let reference = vm(src);
    assert_eq!(
        reference, pinned,
        "the VM no longer runs `{name}` as pinned:\n{src}"
    );
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    match c(name, src) {
        Ok(got) => assert_eq!(
            got, reference,
            "the C tier runs `{name}` otherwise than the VM:\n{src}"
        ),
        Err(why) => panic!("the C tier does not run `{name}`: {why}\n{src}"),
    }
}

macro_rules! rows {
    ($($name:ident: $src:expr => $stdout:expr $(, fails $reason:expr)?;)*) => {
        $(
            #[test]
            fn $name() {
                holds(stringify!($name), $src, $stdout, rows!(@reason $($reason)?));
            }
        )*
    };
    (@reason) => { None };
    (@reason $reason:expr) => { Some($reason) };
}

// ── Leaving a block early: `Break` leaves the innermost loop or zone; `Return` in a zone leaves the zone; ──
// ── outside any, `Break` ends a function's body, and either ends `## Main` as a finished run.           ──
rows! {
    break_leaves_the_innermost_while:
        "## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\n    If i is equal to 4:\n        Break.\nShow i.\n"
        => "4\n";
    break_leaves_a_counted_loop:
        "## Main\nLet mutable n be 0.\nRepeat for i from 1 to 10:\n    If i is 5:\n        Break.\n    Set n to n + i.\nShow n.\n"
        => "10\n";
    break_leaves_a_loop_over_a_sequence:
        "## Main\nLet mutable n be 0.\nRepeat for x in [3, 4, 5, 6]:\n    If x is 5:\n        Break.\n    Set n to n + x.\nShow n.\n"
        => "7\n";
    break_leaves_only_the_inner_of_two_loops:
        "## Main\nLet mutable n be 0.\nRepeat for i from 1 to 3:\n    Repeat for j from 1 to 3:\n        If j is 2:\n            Break.\n        Set n to n + 1.\nShow n.\n"
        => "3\n";
    break_outside_a_loop_ends_the_function:
        "## To f:\n    Show 1.\n    Break.\n    Show 2.\n\n## Main\nCall f.\nShow 3.\n"
        => "1\n3\n";
    break_outside_a_loop_ends_main:
        "## Main\nShow 1.\nBreak.\nShow 2.\n"
        => "1\n";
    return_in_main_ends_the_run_as_finished:
        "## Main\nShow 1.\nReturn.\nShow 2.\n"
        => "1\n";
    return_of_a_value_in_main_ends_the_run_as_finished:
        "## Main\nShow 1.\nReturn 5.\nShow 2.\n"
        => "1\n";
    a_zone_runs_its_body:
        "## Main\nInside a zone called \"Scratch\":\n    Let x be 5.\n    Show x.\nShow \"after\".\n"
        => "5\nafter\n";
    a_zones_binding_ends_with_the_zone:
        "## Main\nLet x be \"outer\".\nInside a zone called \"Scratch\":\n    Let x be 5.\n    Show x + 1.\nShow x.\n"
        => "6\nouter\n";
    break_in_a_zone_leaves_the_zone:
        "## Main\nInside a zone called \"Z\":\n    Show 1.\n    Break.\n    Show 2.\nShow 3.\n"
        => "1\n3\n";
    break_in_a_loop_in_a_zone_leaves_the_loop:
        "## Main\nInside a zone called \"Z\":\n    Repeat for i from 1 to 3:\n        If i is 2:\n            Break.\n        Show i.\n    Show \"rest of zone\".\nShow \"after\".\n"
        => "1\nrest of zone\nafter\n";
    return_in_a_zone_leaves_the_zone_not_the_function:
        "## To f -> Int:\n    Inside a zone called \"Z\":\n        Show \"in\".\n        Return 5.\n    Return 7.\n\n## Main\nShow f().\n"
        => "in\n7\n";
    return_from_a_loop_in_a_zone_leaves_the_zone:
        "## Main\nLet mutable n be 0.\nInside a zone called \"Z\":\n    Repeat for i from 1 to 5:\n        If i is 3:\n            Return.\n        Set n to n + i.\nShow n.\n"
        => "3\n";
    a_zones_name_reads_as_nothing:
        "## Main\nInside a zone called \"scratch\":\n    Show scratch.\nShow 1.\n"
        => "nothing\n1\n";
    a_spawned_agents_name_reads_as_nothing:
        "## Main\nSpawn a Worker called \"w1\".\nShow w1.\n"
        => "nothing\n";
    a_zone_inside_a_loop_keeps_the_loop_running:
        "## Main\nRepeat for i from 1 to 3:\n    Inside a zone called \"Z\":\n        If i is 2:\n            Break.\n        Show i.\nShow \"done\".\n"
        => "1\n3\ndone\n";
}

// ── `Attempt all of the following:` and `Simultaneously:` run each task in order; a task's bindings stay, ──
// ── and a `Break` or `Return` ends only the task it is.                                                   ──
rows! {
    concurrent_tasks_run_in_order_and_keep_their_bindings:
        "## To first -> Int:\n    Return 1.\n\n## To second -> Int:\n    Return 2.\n\n## Main\nAttempt all of the following:\n    Let a be first().\n    Let b be second().\nShow a + b.\n"
        => "3\n";
    parallel_tasks_run_in_order_and_keep_their_bindings:
        "## To first -> Int:\n    Return 1.\n\n## To second -> Int:\n    Return 2.\n\n## Main\nSimultaneously:\n    Let a be first().\n    Let b be second().\nShow a + b.\n"
        => "3\n";
    a_return_task_ends_only_itself:
        "## Main\nAttempt all of the following:\n    Show 1.\n    Return.\n    Show 2.\nShow 3.\n"
        => "1\n2\n3\n";
    a_break_task_in_a_loop_ends_only_itself:
        "## Main\nRepeat for i from 1 to 2:\n    Simultaneously:\n        Show i.\n        Break.\n        Show i * 10.\nShow \"done\".\n"
        => "1\n10\n2\n20\ndone\n";
}

// ── Handing a value to a function: `Give x to f` calls `f` with it, as `Show x to f` does. ──
rows! {
    give_hands_the_value_to_the_function:
        "## To consume (xs: Seq of Int):\n    Show length of xs.\n\n## Main\nLet xs be [1, 2, 3].\nGive xs to consume.\n"
        => "3\n";
    give_to_show_prints_it:
        "## Main\nGive 7 to show.\n"
        => "7\n";
    give_to_a_function_of_two_parameters_fails:
        "## To pair (a: Int, b: Int):\n    Show a + b.\n\n## Main\nShow 1.\nGive 7 to pair.\nShow 2.\n"
        => "1\n", fails "Function pair expects 2 arguments, got 1";
    show_to_a_function_of_two_parameters_fails:
        "## To pair (a: Int, b: Int):\n    Show a + b.\n\n## Main\nShow 1.\nShow 7 to pair.\nShow 2.\n"
        => "1\n", fails "Function pair expects 2 arguments, got 1";
}

// ── Declarations and proof-layer statements do nothing when the program runs. ──
rows! {
    a_trust_does_nothing_at_run_time:
        "## Main\nLet x be 10.\nTrust that x is greater than 0 because \"set to 10\".\nShow x.\n"
        => "10\n";
    a_theorem_does_nothing_at_run_time:
        "## Theorem: Socrates\nGiven: Socrates is a man.\nGiven: Every man is mortal.\nProve: Socrates is mortal.\nProof: Auto.\n\n## Main\nShow 1.\n"
        => "1\n";
    a_requirement_does_nothing_at_run_time:
        "## Requires\n    The \"memchr\" crate version \"2\".\n\n## Main\nShow 1.\n"
        => "1\n";
    a_note_does_nothing_at_run_time:
        "## Main\nNote that starting the run.\nShow \"hi\".\n"
        => "hi\n";
}

// ── A program the VM refuses before it runs — its front end, or its compiler — fails the same way as C: no ──
// ── output, the VM's own words on stderr.                                                                  ──
rows! {
    a_zero_index_is_refused_in_the_vms_words:
        "## Main\nLet xs be [1, 2, 3].\nShow items 0 through 2 of xs.\n"
        => "", fails "This asks for item 0, but LOGOS indices start at 1 \u{2014} in English, 'the 1st item' is the first one; there is no item 0. Which element did you want? The first is 'item 1 of xs' \u{2014} the zero-based habit is the only thing to unlearn.";
}

// ── Sequences: a slice is the elements between two positions, or empty; `contains` asks membership of a ──
// ── Text, a sequence, a set or a map; a loop reads its collection once; a Text iterates by character.    ──
rows! {
    a_list_a_counted_loop_fills_holds_every_element:
        "## Main\nLet mutable dp be a new Seq of Int.\nPush 1 to dp.\nLet mutable i be 1.\nWhile i is at most 2000:\n    Push 0 to dp.\n    If i % 2 equals 0:\n        Push i to dp.\n    Otherwise:\n        Push 0 - i to dp.\n    Set i to i + 1.\nShow length of dp.\nShow item 4001 of dp.\n"
        => "4001\n2000\n";
    a_slice_of_a_literal_is_its_elements:
        "## Main\nShow sumSeq(items 1 through 2 of [1, 2, 3]).\n"
        => "3\n";
    slices_within_and_past_the_end:
        "## Main\nLet xs be [1, 2, 3, 4, 5].\nShow items 2 through 4 of xs.\nShow items 4 through 9 of xs.\nShow items 3 through 2 of xs.\nShow items 1 through 5 of xs.\nLet ws be [\"a\", \"b\", \"c\"].\nShow items 2 through 3 of ws.\n"
        => "[2, 3, 4]\n[]\n[]\n[1, 2, 3, 4, 5]\n[b, c]\n";
    a_literal_contains_an_element:
        "## Main\nShow [1, 2, 3] contains 2.\nShow [1, 2, 3] contains 7.\n"
        => "true\nfalse\n";
    a_text_contains_a_text:
        "## Main\nLet t be \"haystack\".\nShow t contains \"st\".\nShow t contains \"xy\".\nShow contains(\"0123456789\", \"7\").\n"
        => "true\nfalse\ntrue\n";
    a_set_and_a_map_contain_their_members:
        "## Main\nLet mutable s be a new Set of Text.\nAdd \"a\" to s.\nShow s contains \"a\".\nLet mutable m be a new Map of Text to Int.\nSet item \"k\" of m to 1.\nShow m contains \"k\".\nShow m contains \"q\".\n"
        => "true\ntrue\nfalse\n";
    a_text_iterates_by_character:
        "## Main\nLet mutable n be 0.\nRepeat for c in \"h\u{e9}llo\":\n    Show c.\n    Set n to n + 1.\nShow n.\n"
        => "h\n\u{e9}\nl\nl\no\n5\n";
    a_loop_reads_its_collection_once:
        "## To three -> Seq of Int:\n    Show \"called\".\n    Return [1, 2, 3].\n\n## Main\nRepeat for x in three():\n    Show x.\n"
        => "called\n1\n2\n3\n";
    a_manifest_is_the_empty_list:
        "## Main\nLet m be manifest of \"main\".\nShow length of m.\n"
        => "0\n";
}

// ── Tuples and closures: a `Pair of A and B` is the tuple of its parts; a closure bound to a name is called ──
// ── by it; a function written without `->` answers what it returns.                                       ──
rows! {
    a_pair_parameter_takes_a_tuple:
        "## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\n## Main\nLet p be (3, 4).\nShow addpair(p).\n"
        => "7\n";
    a_bound_closure_is_called_by_name:
        "## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"
        => "42\n";
    a_function_without_a_declared_type_answers_its_return:
        "## To twice (n: Int):\n    Return n * 2.\n\n## Main\nShow twice(21).\n"
        => "42\n";
}

// ── A server's routes answer a program's `route(…)` calls: its routes, redirects and guards. ──
rows! {
    a_route_answers_its_path:
        "## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"
        => "hello\n";
    a_redirect_answers_its_status:
        "## Redirect /old -> /new.\n\n## Route GET /new -> Text:\n    Return \"moved\".\n\n## Main\nShow routeStatus(\"GET\", \"/old\").\n"
        => "302\n";
    a_guard_that_declines_lets_the_route_answer:
        "## Before each request (request: Request) -> Option of Response:\n    Return none.\n\n## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"
        => "hello\n";
}

// ── A statement the VM fails when it runs fails the same way on C, after the same output: a name nothing ──
// ── declared, an operator between kinds it does not combine, a position a tuple does not have.            ──
rows! {
    an_inspect_names_the_uncovered_variant_it_met:
        "## A Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Green.\nInspect c:\n    When Red: Show \"r\".\n"
        => "", fails "Inspect has no arm for `Green` (a `Color`) and no Otherwise (matches must be exhaustive) \u{2014} add a `When Green (\u{2026}):` arm, or an `Otherwise:` to cover it.";
    an_inspect_naming_every_variant_needs_no_otherwise:
        "## A Color is either:\n    A Red.\n    A Green.\n\n## Main\nLet c be a new Green.\nInspect c:\n    When Red: Show \"r\".\n    When Green: Show \"g\".\nShow \"done\".\n"
        => "g\ndone\n";
    an_inspect_with_no_arm_for_the_value_fails_in_the_vms_words:
        "## A Color is either:\n    A Red.\n    A Green.\n    A Blue.\n\n## Main\nLet c be a new Blue.\nShow 1.\nInspect c:\n    When Red: Show \"r\".\n    When Green: Show \"g\".\nShow 2.\n"
        => "1\n", fails "Inspect has no arm for `Blue` (a `Color`) and no Otherwise (matches must be exhaustive) \u{2014} add a `When Blue (\u{2026}):` arm, or an `Otherwise:` to cover it.";
    setting_a_name_nothing_declared_fails_when_it_runs:
        "## Main\n    Show 1.\n    Set x to 5.\n    Show 2.\n"
        => "1\n", fails "Undefined variable: x";
    approximately_between_texts_fails:
        "## Main\n    Let a be \"hi\".\n    Let b be \"hi\".\n    Show a is approximately b.\n"
        => "", fails "`is approximately` compares numbers, got Text and Text";
    multiplying_a_text_by_an_int_fails:
        "## Main\nShow 1.\nLet t be \"ab\".\nShow t * 3.\n"
        => "1\n", fails "Cannot multiply Text and Int";
    a_tuples_last_item_counts_from_the_end:
        "## Main\nLet t be (7, 8, 9).\nShow item -1 of t.\nShow item -3 of t.\n"
        => "9\n7\n";
    an_item_past_a_tuples_end_fails:
        "## Main\nLet t be (7, 8, 9).\nShow 1.\nShow item 4 of t.\n"
        => "1\n", fails "Index 4 out of bounds";
    a_texts_items_are_its_characters_counted_from_either_end:
        "## Main\nLet s be \"h\u{e9}llo w\u{f6}rld\".\nShow item 2 of s.\nShow item -1 of s.\nShow item -4 of s.\nShow length of s.\n"
        => "\u{e9}\nd\n\u{f6}\n13\n";
    a_plain_texts_items_count_from_either_end:
        "## Main\nLet s be \"hello world\".\nShow item 2 of s.\nShow item -1 of s.\nShow item -5 of s.\nLet mutable r be \"\".\nRepeat for i from 1 to 3:\n    Set r to r + \"ab\" + i.\nShow item -1 of r.\nShow item -3 of r.\nShow item 4 of r.\n"
        => "e\nd\nw\n3\na\na\n";
    a_built_texts_items_are_its_characters:
        "## Main\nLet mutable r be \"\".\nRepeat for i from 1 to 3:\n    Set r to r + \"\u{e9}\" + i.\nShow item 4 of r.\nShow item -2 of r.\nShow item 1 of r.\nShow length of r.\n"
        => "2\n\u{e9}\n\u{e9}\n9\n";
    an_item_past_a_texts_end_fails:
        "## Main\nLet s be \"abc\".\nShow item 3 of s.\nShow item 4 of s.\n"
        => "c\n", fails "Index 4 out of bounds";
    an_item_before_a_built_texts_start_fails:
        "## Main\nLet mutable r be \"\".\nRepeat for i from 1 to 3:\n    Set r to r + \"ab\".\nShow item -6 of r.\nShow item -7 of r.\n"
        => "a\n", fails "Index -7 out of bounds";
    a_tuple_item_by_a_computed_position:
        "## Main\nLet t be (7, 8, 9).\nLet mutable k be 1.\nRepeat for i from 1 to 3:\n    Show item k of t.\n    Set k to k + 1.\nShow item k of t.\n"
        => "7\n8\n9\n", fails "Index 4 out of bounds";
    destructuring_an_empty_list_runs_no_pass:
        "## Main\nLet pairs be [].\nRepeat for (a, b) in pairs:\n    Show a + b.\nShow \"done\".\n"
        => "done\n";
    destructuring_a_non_tuple_fails:
        "## Main\nLet xs be [1, 2].\nRepeat for (a, b) in xs:\n    Show a + b.\n"
        => "", fails "Expected tuple for pattern, got Int";
    a_closure_value_takes_a_negative_argument:
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * n.\n    Show apply(g, -4).\n"
        => "16\n";
}

// ── `Pop` from an empty list stops the program with one sentence naming the list, after what it printed. ──
rows! {
    popping_an_empty_list_stops_naming_it:
        "## Main\nLet mutable xs be a new Seq of Int.\nShow \"before\".\nPop from xs into v.\nShow v.\n"
        => "before\n", fails "Cannot pop from xs: it is empty";
    popping_past_the_last_element_stops_at_that_pop:
        "## Main\nLet mutable xs be a new Seq of Int.\nPush 7 to xs.\nPop from xs into a.\nShow a.\nPop from xs into b.\nShow b.\n"
        => "7\n", fails "Cannot pop from xs: it is empty";
    popped_texts_and_records_are_themselves:
        "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable ws be a new Seq of Text.\nPush \"a\" to ws.\nPop from ws into w.\nShow w.\nLet mutable ps be a new Seq of Point.\nPush a new Point with x 3 to ps.\nPop from ps into p.\nShow p.\nPop from ps into q.\nShow q.\n"
        => "a\nPoint { x: 3 }\n", fails "Cannot pop from ps: it is empty";
    a_popped_value_takes_part_in_arithmetic:
        "## Main\nLet mutable xs be [4, 5].\nPop from xs into v.\nShow v * 2.\nShow xs.\n"
        => "10\n[4]\n";
    popping_an_empty_list_without_a_name_stops_too:
        "## Main\nLet mutable xs be a new Seq of Int.\nPop from xs.\nShow length of xs.\n"
        => "", fails "Cannot pop from xs: it is empty";
    popping_an_empty_field_list_names_the_field:
        "## A Bag has:\n    an items, which is Seq of Int.\n\n## Main\nLet mutable b be a new Bag with items (a new Seq of Int).\nPush 1 to b's items.\nPop from b's items into v.\nShow v.\nPop from b's items into w.\nShow w.\n"
        => "1\n", fails "Cannot pop from b's items: it is empty";
    a_pop_in_a_function_stops_where_its_list_runs_out:
        "## To drain (n: Int) -> Int:\n    Let mutable xs be [1, 2].\n    Let mutable total be 0.\n    Repeat for i from 1 to n:\n        Pop from xs into v.\n        Set total to total + v.\n    Return total.\n\n## Main\nShow drain(2).\nShow drain(3).\n"
        => "3\n", fails "Cannot pop from xs: it is empty";
}

// ── A name that reads as nothing (a zone's) fails arithmetic in the VM's words and is false, as an empty ──
// ── text is.                                                                                            ──
rows! {
    arithmetic_on_nothing_fails:
        "## Main\nInside a zone called \"scratch\":\n    Show 1.\n    Show scratch + 1.\n"
        => "1\n", fails "Cannot add Nothing and Int";
    nothing_is_false_and_an_empty_text_is_false:
        "## Main\nInside a zone called \"scratch\":\n    If scratch:\n        Show \"v\".\n    Otherwise:\n        Show \"no v\".\nLet t be \"\".\nIf t or false:\n    Show \"t\".\nOtherwise:\n    Show \"no t\".\nShow not t.\n"
        => "no v\nno t\ntrue\n";
}

// ── Function values: a closure keeps what it read from around it as it was when made, and is called ──
// ── through the value; a function's name is no value.                                               ──
rows! {
    a_closure_captures_an_outer_binding:
        "## Main\n    Let base be 1000.\n    Let addBase be (n: Int) -> n + base.\n    Show addBase(7).\n"
        => "1007\n";
    a_closure_keeps_the_value_it_captured:
        "## Main\nLet mutable base be 10.\nLet addBase be (n: Int) -> n + base.\nSet base to 20.\nShow addBase(1).\nShow base.\n"
        => "11\n20\n";
    a_capturing_closure_passes_to_a_helper:
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet k be 3.\nLet times be (n: Int) -> n * k.\nShow apply(times, 5).\n"
        => "15\n";
    a_named_function_is_no_value:
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow 1.\nShow apply(double, 21).\n"
        => "1\n", fails "Undefined variable: double";
    a_closure_over_text_and_a_comparison:
        "## Main\nLet greeting be \"hi \".\nLet greet be (name: Text) -> greeting + name.\nShow greet(\"ann\").\nLet big be (n: Int) -> n is greater than 2.\nShow big(3).\n"
        => "hi ann\ntrue\n";
    a_closure_is_shown_as_a_closure:
        "## Main\nLet f be (x: Int) -> x + 1.\nShow f.\n"
        => "<closure>\n";
}

// ── `a followed by b` is a fresh list of every element of `a`, then every element of `b`. ──
rows! {
    lists_followed_by_lists:
        "## Main\nLet xs be [1, 2].\nLet ys be [3, 4, 5].\nLet zs be xs followed by ys.\nShow zs.\nShow length of zs.\nLet ws be [\"a\"] followed by [\"b\", \"c\"].\nShow item 3 of ws.\n"
        => "[1, 2, 3, 4, 5]\n5\nc\n";
    an_empty_list_followed_by_a_list:
        "## Main\nLet xs be [].\nLet ys be [7, 8].\nShow xs followed by ys.\nShow ys followed by [].\nLet e be [].\nShow length of (e followed by e).\n"
        => "[7, 8]\n[7, 8]\n0\n";
    followed_by_leaves_its_operands_whole:
        "## Main\nLet a be [1, 2].\nLet b be [3].\nLet mutable c be a followed by b.\nPush 9 to c.\nShow a.\nShow c.\n"
        => "[1, 2]\n[1, 2, 3, 9]\n";
}

// ── `Check`: a predicate or a capability holds or the run fails with the VM's policy words; a capability ──
// ── reads the record the sentence names.                                                                ──
rows! {
    a_capability_denied_fails:
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet mallory be a new User with name \"Mallory\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that mallory can edit doc.\nShow \"edited\".\n"
        => "", fails "Security Check Failed: mallory can Edit the doc";
    a_capability_holds_through_the_owner:
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet alice be a new User with name \"Alice\" and role \"guest\".\nLet doc be a new Document with owner \"Alice\".\nCheck that alice can edit doc.\nShow \"edited\".\n"
        => "edited\n";
    a_capability_holds_through_a_predicate:
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet root be a new User with name \"Root\" and role \"admin\".\nLet doc be a new Document with owner \"Alice\".\nCheck that root can edit doc.\nShow \"edited\".\n"
        => "edited\n";
    a_predicate_the_type_lacks_fails_by_name:
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\nA Document has:\n    an owner: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\nA User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n## Main\nLet u be a new User with name \"U\" and role \"x\".\nShow 1.\nCheck that the u is staff.\nShow 2.\n"
        => "1\n", fails "No predicate 'staff' defined for type 'User'";
    a_disjunctive_predicate:
        "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n## Main\nLet u be a new User with role \"mod\".\nCheck that the u is staff.\nShow \"staffok\".\nLet g be a new User with role \"guest\".\nCheck that the g is staff.\nShow \"no\".\n"
        => "staffok\n", fails "Security Check Failed: g is staff";
}

// ── Operands are computed left to right: arguments, operators, interpolation holes, an item and its value. ──
rows! {
    a_quiet_call_beside_a_noisy_one_keeps_its_order:
        "## To square (n: Int) -> Int:\n    Return n * n.\n\n## To noisy (n: Int) -> Int:\n    Show n.\n    Return n.\n\n## To spin (n: Int) -> Int:\n    Let mutable i be 0.\n    While i is less than n:\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow square(3) + noisy(4).\nShow noisy(5) + spin(6).\n"
        => "4\n13\n5\n11\n";
    arguments_are_computed_left_to_right:
        "## To fa (n: Int) -> Int:\n    Show \"a\".\n    Return n.\n\n## To fb (n: Int) -> Int:\n    Show \"b\".\n    Return n.\n\n## To add (x: Int, y: Int) -> Int:\n    Return x + y.\n\n## Main\nShow add(fa(1), fb(2)).\nCall add with fa(3) and fb(4).\n"
        => "a\nb\n3\na\nb\n";
    operands_are_computed_left_to_right:
        "## To fa (n: Int) -> Int:\n    Show \"a\".\n    Return n.\n\n## To fb (n: Int) -> Int:\n    Show \"b\".\n    Return n.\n\n## To add (x: Int, y: Int) -> Int:\n    Return x + y.\n\n## Main\nShow fa(1) + fb(2).\nShow fa(1) is less than fb(2).\nLet xs be [fa(1), fb(2)].\nShow xs.\n"
        => "a\nb\n3\na\nb\ntrue\na\nb\n[1, 2]\n";
    interpolation_holes_are_computed_left_to_right:
        "## To fa (n: Int) -> Int:\n    Show \"a\".\n    Return n.\n\n## To fb (n: Int) -> Int:\n    Show \"b\".\n    Return n.\n\n## To add (x: Int, y: Int) -> Int:\n    Return x + y.\n\n## Main\nShow \"{fa(1)} and {fb(2)}\".\nLet t be \"{fb(5)}-{fa(6)}\".\nShow t.\n"
        => "a\nb\n1 and 2\nb\na\n5-6\n";
    an_item_position_is_computed_before_its_value:
        "## To fa (n: Int) -> Int:\n    Show \"a\".\n    Return n.\n\n## To fb (n: Int) -> Int:\n    Show \"b\".\n    Return n.\n\n## To add (x: Int, y: Int) -> Int:\n    Return x + y.\n\n## Main\nLet mutable xs be [0, 0, 0].\nSet item (fa(2)) of xs to fb(9).\nShow xs.\n"
        => "a\nb\n[0, 9, 0]\n";
    the_first_failing_operand_fails_first:
        "## Main\nLet xs be [1, 2].\nLet z be 0.\nShow 10 / z + item 5 of xs.\n"
        => "", fails "Division by zero";
}

// ── A list, a map, a set and a record holding one are values: a second name, a callee, a container, a loop ──
// ── and a closure each hold their own, so a change through one leaves the others as they were.             ──
rows! {
    a_second_name_holds_its_own_list:
        "## Main\nLet mutable xs be [1, 2, 3].\nLet mutable ys be xs.\nPush 4 to ys.\nSet item 1 of ys to 9.\nShow xs.\nShow ys.\nPush 5 to xs.\nShow ys.\n"
        => "[1, 2, 3]\n[9, 2, 3, 4]\n[9, 2, 3, 4]\n";
    a_callees_change_stays_in_the_callee:
        "## To setat (xs: Seq of Int) (k: Int) -> Seq of Int:\n    Set item k of xs to 99.\n    Return xs.\n\n## Main\n    Let mutable a be [1, 2, 3].\n    Let b be setat(a, 1).\n    Show a.\n    Show b.\n    Set a to setat(a, -1).\n    Show a.\n"
        => "[1, 2, 3]\n[99, 2, 3]\n[1, 2, 99]\n";
    a_loop_walks_its_list_as_it_began:
        "## Main\nLet mutable xs be [1, 2, 3].\nRepeat for x in xs:\n    If x is less than 3:\n        Push x * 10 to xs.\n    Set item 3 of xs to 99.\n    Show x.\nShow xs.\n"
        => "1\n2\n3\n[1, 2, 99, 10, 20]\n";
    a_list_pushed_into_another_is_a_copy:
        "## Main\nLet mutable grid be a new Seq of Seq of Int.\nLet mutable row be [1, 2].\nPush row to grid.\nPush 3 to row.\nShow grid.\nShow row.\nLet mutable first be item 1 of grid.\nPush 7 to first.\nShow grid.\nShow first.\n"
        => "[[1, 2]]\n[1, 2, 3]\n[[1, 2]]\n[1, 2, 7]\n";
    a_record_holding_a_list_is_a_value:
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable p be a new Bag with items [1].\nLet mutable q be p.\nPush 2 to q's items.\nShow p's items.\nShow q's items.\n"
        => "[1]\n[1, 2]\n";
    a_map_given_a_second_name_is_a_copy:
        "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nLet mutable n be m.\nSet item \"b\" of n to 2.\nShow m.\nShow n.\n"
        => "{a: 1}\n{a: 1, b: 2}\n";
    copy_of_a_nested_list_shares_nothing:
        "## Main\nLet mutable g be [[1], [2]].\nLet mutable h be copy of g.\nLet mutable inner be item 1 of h.\nPush 5 to inner.\nSet item 1 of h to inner.\nShow g.\nShow h.\n"
        => "[[1], [2]]\n[[1, 5], [2]]\n";
    a_closure_holds_the_list_it_captured:
        "## Main\nLet mutable xs be [1, 2].\nLet count be (n: Int) -> length of xs + n.\nPush 3 to xs.\nShow count(0).\nShow length of xs.\n"
        => "2\n3\n";
    a_closure_calling_a_function_that_changes_its_list_changes_a_copy:
        "## To bump (xs: Seq of Int) -> Int:\n    Set item 1 of xs to 99.\n    Return item 1 of xs.\n\n## Main\nLet mutable a be [1, 2].\nLet f be (n: Int) -> bump(a) + n.\nShow f(1).\nShow a.\n"
        => "100\n[1, 2]\n";
    a_callee_changes_its_own_copy_of_a_list_the_caller_reads_again:
        "## To f (xs: Seq of Int) -> Int:\n    Let mutable r be xs.\n    Set item 1 of r to 99.\n    Return 0.\n\n## To g (xs: Seq of Int) -> Int:\n    Set item 2 of xs to 77.\n    Return 0.\n\n## Main\nLet mutable a be [1, 2, 3].\nLet z be f(a).\nShow a.\nLet w be g(a).\nShow a.\nLet mutable b be a.\nSet item 3 of b to 55.\nShow a.\nShow b.\n"
        => "[1, 2, 3]\n[1, 2, 3]\n[1, 2, 3]\n[1, 2, 55]\n";
    one_list_handed_twice_is_two_values:
        "## To mix (xs: Seq of Int, ys: Seq of Int) -> Int:\n    Set item 1 of xs to 50.\n    Return item 1 of ys.\n\n## Main\nLet mutable a be [1, 2].\nShow mix(a, a).\nShow a.\n"
        => "1\n[1, 2]\n";
    a_list_handed_back_and_changed_leaves_the_original:
        "## To same (xs: Seq of Int) -> Seq of Int:\n    Show length of xs.\n    Return xs.\n\n## To first (xs: Seq of Int) -> Seq of Int:\n    Let ys be same(xs).\n    Show item 1 of ys.\n    Return ys.\n\n## Main\nLet mutable a be [1, 2].\nLet mutable b be first(a).\nSet item 1 of b to 9.\nShow a.\nShow b.\n"
        => "2\n1\n[1, 2]\n[9, 2]\n";
    a_list_handed_back_is_its_own:
        "## To same (xs: Seq of Int) -> Seq of Int:\n    Return xs.\n\n## Main\nLet mutable a be [1, 2].\nLet mutable b be same(a).\nPush 3 to b.\nShow a.\nShow b.\n"
        => "[1, 2]\n[1, 2, 3]\n";
    a_loop_walks_its_list_as_it_began_while_a_callee_changes_it:
        "## To bump (xs: Seq of Int) -> Seq of Int:\n    Set item 3 of xs to item 3 of xs + 100.\n    Return xs.\n\n## Main\nLet mutable xs be [1, 2, 3].\nRepeat for x in xs:\n    Set xs to bump(xs).\n    Show x.\nShow xs.\n"
        => "1\n2\n3\n[1, 2, 303]\n";
    a_loop_element_handed_to_a_callee_is_its_own:
        "## To mark (xs: Seq of Int) -> Seq of Int:\n    Set item 1 of xs to 9.\n    Return xs.\n\n## Main\nLet mutable grid be [[1], [2]].\nRepeat for row in grid:\n    Let r be mark(row).\n    Show r.\nShow grid.\n"
        => "[9]\n[9]\n[[1], [2]]\n";
    a_program_list_a_function_reads_stays_whole_while_a_callee_changes_its_copy:
        "## To peek -> Int:\n    Return item 1 of g.\n\n## To bump (xs: Seq of Int) -> Int:\n    Set item 1 of xs to 100.\n    Return peek() + item 1 of xs.\n\n## Main\nLet g be [1, 2].\nShow bump(g).\nShow g.\n"
        => "101\n[1, 2]\n";
    a_list_rebound_to_what_a_callee_made_of_it_is_handed_over:
        "## To bump (xs: Seq of Int, i: Int) -> Seq of Int:\n    Let mutable ys be xs.\n    Set item i of ys to item i of ys + i.\n    Return ys.\n\n## Main\nLet mutable xs be [0, 0, 0].\nLet keep be xs.\nRepeat for i from 1 to 3:\n    Set xs to bump(xs, i).\nShow xs.\nShow keep.\n"
        => "[1, 2, 3]\n[0, 0, 0]\n";
    an_appended_text_is_its_own:
        "## To shout (t: Text) -> Text:\n    Set t to t + \"!\".\n    Return t.\n\n## Main\nLet mutable s be \"hi\".\nLet mutable u be \"yo\".\nSet u to s.\nSet u to u + \"?\".\nShow s.\nShow u.\nShow shout(s).\nShow s.\n"
        => "hi\nhi?\nhi!\nhi\n";
}

// ── `Swap a and b` exchanges the two values, whatever their kind, in Main and in a function; a name swapped ──
// ── with itself keeps its value.                                                                           ──
rows! {
    swapping_two_ints_exchanges_them:
        "## Main\nLet mutable a be 1.\nLet mutable b be 2.\nSwap a and b.\nShow a.\nShow b.\n"
        => "2\n1\n";
    swapping_two_reals_exchanges_them:
        "## Main\nLet mutable a be 1.5.\nLet mutable b be 2.25.\nSwap a and b.\nShow a.\nShow b.\n"
        => "2.25\n1.5\n";
    swapping_two_texts_exchanges_them_and_each_grows_on:
        "## Main\nLet mutable a be \"x\".\nLet mutable b be \"y\".\nSet a to a + \"1\".\nSwap a and b.\nSet a to a + \"2\".\nSet b to b + \"3\".\nShow a.\nShow b.\n"
        => "y2\nx13\n";
    swapping_a_growing_text_with_a_fixed_one_lets_the_growing_name_grow_on:
        "## Main\nLet mutable a be \"x\".\nLet mutable b be \"yy\".\nSet a to a + \"1\".\nSwap a and b.\nRepeat for i from 1 to 300:\n    Set a to a + \"z\".\nShow length of a.\nShow item 2 of a.\nShow item 3 of a.\nShow b.\n"
        => "302\ny\nz\nx1\n";
    swapping_two_lists_exchanges_them:
        "## Main\nLet mutable a be [1].\nLet mutable b be [2, 3].\nSwap a and b.\nPush 4 to a.\nShow a.\nShow b.\n"
        => "[2, 3, 4]\n[1]\n";
    swapping_two_maps_exchanges_them:
        "## Main\nLet mutable a be a new Map of Text to Int.\nLet mutable b be a new Map of Text to Int.\nSet item \"a\" of a to 1.\nSet item \"b\" of b to 2.\nSwap a and b.\nShow a.\nShow b.\n"
        => "{b: 2}\n{a: 1}\n";
    swapping_two_sets_exchanges_them:
        "## Main\nLet mutable a be a new Set of Int.\nLet mutable b be a new Set of Int.\nAdd 1 to a.\nAdd 2 to b.\nSwap a and b.\nShow a.\nShow b.\n"
        => "{2}\n{1}\n";
    swapping_two_records_exchanges_them:
        "## A Point has:\n    An x: Int.\n\n## Main\nLet mutable p be a new Point with x 1.\nLet mutable q be a new Point with x 2.\nSwap p and q.\nShow p's x.\nShow q's x.\n"
        => "2\n1\n";
    swapping_two_variants_exchanges_them:
        "## A Shape is either:\n    A Dot.\n    A Line with size Int.\n\n## Main\nLet mutable first be a new Dot.\nLet mutable second be a new Line with size 3.\nSwap first and second.\nInspect first:\n    When Dot: Show \"dot\".\n    When Line (size): Show size.\nInspect second:\n    When Dot: Show \"dot\".\n    When Line (size): Show size.\n"
        => "3\ndot\n";
    swapping_two_tuples_exchanges_them:
        "## Main\nLet mutable a be (1, \"one\").\nLet mutable b be (2, \"two\").\nSwap a and b.\nShow item 2 of a.\nShow item 1 of b.\n"
        => "two\n1\n";
    swapping_two_options_exchanges_them:
        "## Main\nLet mutable a be some 5.\nLet mutable b be none.\nSwap a and b.\nShow a.\nShow b.\n"
        => "nothing\n5\n";
    swapping_a_name_with_itself_keeps_it:
        "## Main\nLet mutable a be [1, 2].\nSwap a and a.\nShow a.\n"
        => "[1, 2]\n";
    swapping_inside_a_function_exchanges_its_names:
        "## A Point has:\n    An x: Int.\n\n## To order (p: Point) and (q: Point) -> Int:\n    Let mutable lo be p.\n    Let mutable hi be q.\n    If lo's x is greater than hi's x:\n        Swap lo and hi.\n    Let mutable xs be [lo's x].\n    Let mutable ys be [hi's x].\n    Swap xs and ys.\n    Return item 1 of xs * 10 + item 1 of ys.\n\n## Main\nShow order(a new Point with x 7, a new Point with x 3).\n"
        => "73\n";
    swapping_exact_integers_keeps_them_exact:
        "## Main\nLet mutable a be 1.\nRepeat for i from 1 to 70:\n    Set a to a * 2.\nLet mutable b be 5.\nSwap a and b.\nShow a.\nShow b.\nSet b to b * 2.\nShow b.\n"
        => "5\n1180591620717411303424\n2361183241434822606848\n";
}

// ── Removing from a set or a map leaves every other value findable and the rest in insertion order; a value ──
// ── added again comes last.                                                                                ──
rows! {
    a_set_keeps_its_order_and_every_value_after_removals:
        "## Main\nLet mutable s be a new Set of Int.\nRepeat for i from 1 to 40:\n    Add i * 16 to s.\nRepeat for i from 1 to 40:\n    If i % 3 equals 0:\n        Remove i * 16 from s.\nLet mutable found be 0.\nRepeat for i from 1 to 40:\n    If s contains i * 16:\n        Set found to found + 1.\nShow found.\nAdd 48 to s.\nShow length of s.\nShow s.\n"
        => "27\n28\n{16, 32, 64, 80, 112, 128, 160, 176, 208, 224, 256, 272, 304, 320, 352, 368, 400, 416, 448, 464, 496, 512, 544, 560, 592, 608, 640, 48}\n";
    a_map_keeps_its_order_and_every_entry_after_removals:
        "## Main\nLet mutable m be a new Map of Int to Int.\nRepeat for i from 1 to 40:\n    Set item (i * 16) of m to i.\nRepeat for i from 1 to 40:\n    If i % 3 equals 0:\n        Remove (i * 16) from m.\nLet mutable total be 0.\nRepeat for i from 1 to 40:\n    If m contains (i * 16):\n        Set total to total + item (i * 16) of m.\nShow total.\nSet item 48 of m to 99.\nShow length of m.\nShow m.\n"
        => "547\n28\n{16: 1, 32: 2, 64: 4, 80: 5, 112: 7, 128: 8, 160: 10, 176: 11, 208: 13, 224: 14, 256: 16, 272: 17, 304: 19, 320: 20, 352: 22, 368: 23, 400: 25, 416: 26, 448: 28, 464: 29, 496: 31, 512: 32, 544: 34, 560: 35, 592: 37, 608: 38, 640: 40, 48: 99}\n";
    a_text_keyed_map_keeps_its_order_and_every_entry_after_removals:
        "## Main\nLet mutable t be a new Map of Text to Int.\nRepeat for i from 1 to 30:\n    Set item (\"k\" + i) of t to i.\nRepeat for i from 1 to 30:\n    If i % 4 equals 0:\n        Remove (\"k\" + i) from t.\nShow length of t.\nShow t contains \"k8\".\nShow t contains \"k9\".\nShow item \"k29\" of t.\nSet item \"k8\" of t to 0.\nShow t.\n"
        => "23\nfalse\ntrue\n29\n{k1: 1, k2: 2, k3: 3, k5: 5, k6: 6, k7: 7, k9: 9, k10: 10, k11: 11, k13: 13, k14: 14, k15: 15, k17: 17, k18: 18, k19: 19, k21: 21, k22: 22, k23: 23, k25: 25, k26: 26, k27: 27, k29: 29, k30: 30, k8: 0}\n";
    a_record_left_unfilled_holds_empty_values:
        "## A Box has:\n    A name: Text.\n    A count: Int.\n    A seen: Set of Int.\n    A index: Map of Text to Int.\n    A items: Seq of Int.\n\n## Main\nLet mutable b be a new Box.\nShow length of b's name.\nShow b's count.\nAdd 3 to b's seen.\nSet item \"a\" of b's index to 1.\nPush 5 to b's items.\nShow b's seen.\nShow b's index.\nShow b's items.\n"
        => "0\n0\n{3}\n{a: 1}\n[5]\n";
    a_record_field_set_takes_additions_and_removals:
        "## A Bag has:\n    A tags: Set of Text.\n\n## Main\nLet mutable b be a new Bag.\nAdd \"x\" to b's tags.\nAdd \"y\" to b's tags.\nRemove \"x\" from b's tags.\nShow b's tags.\n"
        => "{y}\n";
}

// ── An exact integer that fits a word keys a map, joins a set or a list, and fills a map, as the same value ──
// ── as a word does.                                                                                     ──
rows! {
    an_exact_key_asks_whether_a_map_holds_it:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable m be a new Map of Int to Int.\nSet item 243 of m to 7.\nShow m contains k.\nLet j be k + 1.\nShow m contains j.\n"
        => "true\nfalse\n";
    an_exact_key_reads_a_map:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable m be a new Map of Int to Int.\nSet item 243 of m to 7.\nShow item k of m.\n"
        => "7\n";
    an_exact_key_writes_a_map:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable m be a new Map of Int to Int.\nSet item k of m to 9.\nShow item 243 of m.\n"
        => "9\n";
    an_exact_key_leaves_a_map:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable m be a new Map of Int to Int.\nSet item 243 of m to 7.\nRemove k from m.\nShow length of m.\n"
        => "0\n";
    an_exact_key_writes_a_map_of_texts:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable names be a new Map of Int to Text.\nSet item k of names to \"x\".\nShow item 243 of names.\nShow names contains k.\n"
        => "x\ntrue\n";
    an_exact_value_fills_a_map:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable t be a new Map of Text to Int.\nSet item \"a\" of t to k.\nShow item \"a\" of t.\n"
        => "243\n";
    an_exact_element_joins_a_set:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable s be a new Set of Int.\nAdd k to s.\nShow s contains 243.\n"
        => "true\n";
    an_exact_element_asks_whether_a_set_holds_it:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable s be a new Set of Int.\nAdd 243 to s.\nShow s contains k.\n"
        => "true\n";
    an_exact_element_leaves_a_set:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable s be a new Set of Int.\nAdd 243 to s.\nRemove k from s.\nShow length of s.\n"
        => "0\n";
    an_exact_element_joins_a_list:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable xs be [1, 2].\nPush k to xs.\nShow xs.\nShow xs contains k.\n"
        => "[1, 2, 243]\ntrue\n";
    an_exact_element_replaces_a_list_item:
        "## Main\nLet mutable k be 1.\nRepeat for i from 1 to 5:\n    Set k to k * 3.\nLet mutable xs be [1, 2].\nSet item 2 of xs to k.\nShow xs.\n"
        => "[1, 243]\n";
}

// ── An Int is exact: a sum, difference or product no proof keeps within `i64` becomes a big integer when it ──
// ── leaves it, as on the VM, wherever its operands come from.                                             ──
rows! {
    a_closure_body_that_overflows_promotes:
        "## Main\n    Let doubler be (n: Int) -> n * 2.\n    Show doubler(9223372036854775807).\n"
        => "18446744073709551614\n";
    a_helpers_closure_that_overflows_promotes:
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n## Main\n    Let g be (n: Int) -> n * 2.\n    Show apply(g, 9223372036854775807).\n"
        => "18446744073709551614\n";
    a_tuple_return_carries_an_exact_component:
        "## To grow (n: Int) -> (Int, Int):\n    Let mutable h be 1.\n    Let mutable i be 0.\n    While i is less than n:\n        Set h to h * 3.\n        Set i to i + 1.\n    Return (n, h).\n\n## Main\nLet (a, b) be grow(5).\nShow \"\" + a + \" \" + b.\nLet (c, d) be grow(50).\nShow \"\" + c + \" \" + d.\nLet e be d + 1.\nShow e.\n"
        => "5 243\n50 717897987691852588770249\n717897987691852588770250\n";
    a_named_record_return_carries_an_exact_component:
        "## To grow (n: Int) -> (count: Int, sum: Int):\n    Let mutable h be 1.\n    Let mutable i be 0.\n    While i is less than n:\n        Set h to h * 2.\n        Set i to i + 1.\n    Return (n, h).\n\n## Main\nLet r be grow(70).\nShow \"\" + r's count + \" \" + r's sum.\n"
        => "70 1180591620717411303424\n";
    an_exact_component_read_out_of_a_tuple_keeps_an_accumulator_exact:
        "## To grow (n: Int) -> (Int, Int):\n    Let mutable h be 1.\n    Repeat for i from 1 to n:\n        Set h to h * 3.\n    Return (n, h).\n\n## Main\nLet mutable total be 0.\nRepeat for k from 1 to 2:\n    Let (a, b) be grow(40).\n    Set total to total + b.\nShow total.\n"
        => "24315330918113857602\n";
    every_return_of_a_widened_tuple_takes_its_type:
        "## To pick (n: Int) -> (Int, Int):\n    If n is less than 0:\n        Return (n, 0).\n    Let mutable h be 1.\n    Repeat for i from 1 to n:\n        Set h to h * 3.\n    Return (n, h).\n\n## Main\nLet (a, b) be pick(-1).\nShow b.\nLet (c, d) be pick(41).\nShow d.\n"
        => "0\n36472996377170786403\n";
    a_masked_fold_matches:
        "## Main\nLet mutable h be 0.\nRepeat for i from 1 to 10:\n    Let v be i % 3.\n    Set h to (h * 31 + v) & 268435455.\nShow h.\n"
        => "86161282\n";
    a_masked_fold_whose_sum_leaves_a_word_is_exact:
        "## Main\nLet big be 9223372036854775000.\nLet mutable h be 5.\nRepeat for i from 1 to 3:\n    Set h to (h * 31 + big) & 268435455.\nShow h.\n"
        => "267782067\n";
    a_remainder_fold_whose_product_leaves_a_word_is_exact:
        "## Main\nLet mutable seed be 123456789.\nRepeat for i from 1 to 3:\n    Set seed to (seed * 4000000000000 + 1) % 1000000007.\nShow seed.\n"
        => "647864100\n";
    a_mask_over_a_negative_big_integer_fails:
        "## Main\nLet v be 0 - 9223372036854775000.\nShow 1.\nShow (v * 31) & 255.\n"
        => "1\n", fails "`&` requires integer, boolean, or Set operands";
    a_mask_of_no_low_run_over_a_big_integer_fails:
        "## Main\nLet v be 9223372036854775000.\nShow (v + v) & 6.\n"
        => "", fails "`&` requires integer, boolean, or Set operands";
    an_unmasked_accumulator_promotes:
        "## Main\nLet mutable h be 1.\nRepeat for i from 1 to 20:\n    Set h to h * 31 + 7.\nShow h.\n"
        => "828541652210201481177909208641\n";
    a_map_sized_by_a_hint_finds_every_key:
        "## Main\nLet mutable m be a new Map of Int to Int with capacity 1000.\nRepeat for i from 1 to 3000:\n    Set item (i * 7919 % 100003) of m to i.\nShow length of m.\nLet mutable s be 0.\nLet mutable k be 0.\nWhile k is less than 100003:\n    If m contains k:\n        Set s to s + item k of m.\n    Set k to k + 97.\nShow s.\n"
        => "3000\n42738\n";
    a_text_keyed_map_grown_past_its_start_finds_every_key:
        "## Main\nLet mutable t be a new Map of Text to Int.\nRepeat for i from 1 to 2000:\n    Set item (\"k\" + (i * 31 % 5003)) of t to i.\nShow length of t.\nShow item \"k31\" of t.\nShow item \"k62\" of t.\nShow t contains \"k5002\".\n"
        => "2000\n1\n2\nfalse\n";
    a_set_grown_past_its_start_holds_every_value_once:
        "## Main\nLet mutable seen be a new Set of Int.\nRepeat for i from 1 to 5000:\n    Add (i * 17 % 4001) to seen.\nShow length of seen.\n"
        => "4001\n";
    a_collatz_walk_counts_its_steps:
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 1000:\n    Let mutable k be i.\n    While k is not 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n        Set total to total + 1.\nShow total.\n"
        => "59542\n";
}

// ── An exact quotient of integers is the fraction it denotes wherever it stands — inside other arithmetic, an ──
// ── argument to a `Rational` parameter — and a list declared `Seq of Rational` holds its items as fractions.  ──
rows! {
    an_exact_quotient_inside_arithmetic_keeps_its_fraction:
        "## To half (x: Rational) -> Rational:\n    Return x / 2.\n\n## Main\nShow half(0 - 1 / 3).\nLet r: Rational be 1 / 3.\nShow half(r + 1 / 6).\nShow half(2 * (5 / 4)).\n"
        => "-1/6\n1/4\n5/4\n";
    a_list_declared_rational_holds_its_items_as_fractions:
        "## Main\nLet xs: Seq of Rational be [1 / 2, 0 - 1 / 3, 2 * (5 / 4)].\nShow xs.\nLet r: Rational be 1 / 3.\nLet ys be [r, 0 - 1 / 2 + r].\nShow ys.\nSet xs to [3, 4].\nShow xs.\n"
        => "[0, 0, 2]\n[1/3, 1/3]\n[3, 4]\n";
}

/// A text built by appending in a loop grows in linear time and memory: each append copies only what it
/// appends, and reading a character allocates nothing. Copying the whole text at each step needs tens of
/// gigabytes for these loops, and an allocation per character read hundreds of megabytes; either dies at the
/// row's cap (megabytes of address space).
const LINEAR_TEXT: &[(&str, &str, &str, u64)] = &[
    (
        "append_number_and_space",
        "## Main\nLet result be \"\" with capacity 10.\nLet i be 0.\nWhile i is less than 400000:\n    Set result to result + i + \" \".\n    Set i to i + 1.\nShow length of result.\nShow item 3 of result.\n",
        "2688890\n1\n",
        256,
    ),
    (
        "append_two_texts",
        "## Main\nLet mutable result be \"\".\nRepeat for i from 1 to 400000:\n    Set result to result + \"ab\" + \" \".\nShow length of result.\n",
        "1200000\n",
        256,
    ),
    (
        "append_one_text",
        "## Main\nLet mutable result be \"\".\nRepeat for i from 1 to 400000:\n    Set result to result + \"abc\".\nShow length of result.\n",
        "1200000\n",
        256,
    ),
    (
        "combined_with_appends",
        "## Main\nLet mutable result be \"x\".\nRepeat for i from 1 to 400000:\n    Set result to result combined with i.\nShow length of result.\n",
        "2288896\n",
        256,
    ),
    (
        "count_the_spaces_of_a_built_text",
        "## Main\nLet result be \"\" with capacity 600000.\nLet i be 0.\nWhile i is less than 100000:\n    Set result to result + i + \" \".\n    Set i to i + 1.\nLet count be 0.\nSet i to 1.\nWhile i is at most length of result:\n    If item i of result equals \" \":\n        Set count to count + 1.\n    Set i to i + 1.\nShow count.\n",
        "100000\n",
        256,
    ),
    (
        "read_the_characters_of_a_built_text",
        "## Main\nLet mutable result be \"\".\nRepeat for i from 1 to 1000:\n    Set result to result + \"a b \".\nLet mutable count be 0.\nRepeat for n from 1 to 3000000:\n    If item ((n % 4000) + 1) of result equals \" \":\n        Set count to count + 1.\nShow count.\n",
        "1500000\n",
        64,
    ),
    (
        "read_the_characters_of_a_text",
        "## Main\nLet s be \"a b c d\".\nLet mutable count be 0.\nRepeat for n from 1 to 3000000:\n    If item ((n % 7) + 1) of s equals \" \":\n        Set count to count + 1.\nShow count.\n",
        "1285715\n",
        64,
    ),
];

/// Every benchmark's program as the suite runs it (`main.lg`, reading its size from the command line), by name.
fn benchmark_mains() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs");
    let mut mains: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("the benchmark programs")
        .flatten()
        .filter_map(|e| {
            let main = e.path().join("main.lg");
            let src = std::fs::read_to_string(&main).ok()?;
            Some((
                format!("benchmark {}", e.file_name().to_string_lossy()),
                src,
            ))
        })
        .collect();
    mains.sort();
    mains
}

/// No program of the corpus — the harvested programs, every benchmark as the suite runs it and as the
/// interpreter suite runs it — lowers to C holding a placeholder where a construct should be
/// (`/* unsupported … */`, which drops the construct silently) or a refusal (`#error`): every statement and
/// expression kind lowers to what it does.
#[test]
fn no_program_lowers_to_a_placeholder_or_a_refusal() {
    let mut programs: Vec<(String, String)> = gauntlet::all()
        .into_iter()
        .map(|p| (p.name, p.src))
        .collect();
    programs.extend(benchmark_mains());
    assert!(
        programs.len() > 700,
        "the corpus shrank to {} programs",
        programs.len()
    );
    let mut found = Vec::new();
    for (name, src) in &programs {
        let Ok(code) = compile_to_c(src) else {
            continue;
        };
        for marker in ["/* unsupported", "#error"] {
            if let Some(at) = code.find(marker) {
                let line = code[at..].lines().next().unwrap_or_default();
                found.push(format!("{name}: {line}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "programs lowering to a placeholder or a refusal:\n{}",
        found.join("\n")
    );
}

/// The C of `src` after the runtime: the program's own code.
fn user_code(src: &str) -> String {
    let code = compile_to_c(src).expect("the program lowers to C");
    let (_, user) = code
        .split_once("LOGOS user code")
        .expect("the C marks where the program's own code begins");
    user.to_string()
}

/// A fold kept within a word by a mask or a remainder — the checksum most benchmarks end with — runs on machine
/// words: its operands' ranges prove the step never leaves one, so no exact integer and no check appears in the
/// hot loop. An exact counter may retain a guarded cold fallback for arbitrary-precision semantics. The same fold
/// unmasked grows without bound, and runs on exact integers.
#[test]
fn a_bounded_fold_runs_on_machine_words() {
    let masked = "## To fold (n: Int) -> (Int, Int):\n    Let mutable h be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Let mutable v be 0.\n        If i % 2 equals 0:\n            Set v to 1.\n        Set h to (h * 31 + v) & 268435455.\n        Set i to i + 1.\n    Return (n, h).\n\n## Main\nLet (a, b) be fold(5).\nShow \"\" + a + \" \" + b.\n";
    let code = user_code(masked);
    let hot = code
        .split_once("exact counted loop: native-word fast path, exact fallback")
        .and_then(|(_, rest)| rest.split_once("} else {").map(|(fast, _)| fast))
        .unwrap_or(&code);
    assert!(
        !hot.contains("logos_lint_")
            && !hot.contains("lint_expect_i64")
            && !hot.contains("logos_exact_")
            && !hot.contains("__over"),
        "the masked fold's hot loop left machine words:\n{code}"
    );
    let seeded = "## Main\nLet mutable seed be 42.\nLet mutable total be 0.\nRepeat for i from 1 to 100:\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Set total to (total + seed) % 1000000007.\nShow total.\n";
    let code = user_code(seeded);
    assert!(
        !code.contains("lint_") && !code.contains("__over"),
        "the remainder folds left machine words:\n{code}"
    );
    let unmasked = "## Main\nLet mutable h be 1.\nRepeat for i from 1 to 20:\n    Set h to h * 31 + 7.\nShow h.\n";
    assert!(
        user_code(unmasked).contains("lint_mul"),
        "the unbounded fold is not exact"
    );
}

/// Modulus deferral versions an accumulator loop behind a range guard and marks the
/// fresh additions as RawIntOp. Direct C consumes that proof just like Rust: the hot
/// branch keeps the bounded accumulator in a machine word, while the untouched cold
/// sibling remains the exact fallback.
#[test]
fn a_guarded_modulus_deferral_keeps_direct_c_on_machine_words() {
    let src = include_str!("../../../benchmarks/programs/loop_sum/main.lg");
    let code = user_code(src);
    assert!(
        code.contains("int64_t sum = 0LL"),
        "the guarded, modulo-bounded accumulator stayed exact:\n{code}"
    );
    let hot = code
        .split_once("if (((((n >=")
        .and_then(|(_, rest)| rest.split_once("} else {").map(|(hot, _)| hot))
        .expect("modulus deferral emits its guarded fast branch");
    assert!(
        !hot.contains("logos_lint_add(sum") && !hot.contains("LInt sum"),
        "the guarded fast branch rebuilt exact accumulator arithmetic:\n{hot}"
    );
}

/// Calls to a function that neither fails nor acts — only arithmetic that cannot fail over its own parameters and
/// calls to such functions — need no order among themselves, so the C leaves the compiler free to schedule them;
/// a call beside one that shows a line still runs in the VM's order (see the operand-order rows).
#[test]
fn quiet_calls_are_left_unordered() {
    let src = "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(20).\n";
    let code = user_code(src);
    assert!(
        !code.contains("__seq"),
        "two quiet calls were put in an order:\n{code}"
    );
}

/// A loop that may leave before its bound is not sized for the bound: the list it pushes to holds only what it
/// pushed, within an address space far smaller than the bound's worth of elements.
#[test]
fn a_loop_that_leaves_early_is_not_sized_for_its_bound() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    let src = "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 4000000000000000:\n    Push i to xs.\n    If i equals 5:\n        Break.\n    Set i to i + 1.\nShow length of xs.\n";
    assert!(
        !user_code(src).contains("LOGOS_SEQ_RESERVE"),
        "a loop that may leave early is sized for its bound"
    );
    let reference = vm(src);
    assert_eq!(
        reference,
        Observed::of_stream("6\n".to_string(), None),
        "the VM no longer runs the leaving loop as pinned"
    );
    match c_capped("leaves_early", src, 64) {
        Ok(got) => assert_eq!(
            got, reference,
            "the C tier runs the leaving loop otherwise than the VM within 64 MB"
        ),
        Err(why) => panic!("the C tier does not run the leaving loop: {why}"),
    }
}

/// A list a counted loop fills — a push on every pass, the pass count fixed by the loop's bounds — is sized
/// once, before the loop, rather than regrown by doubling while it fills.
#[test]
fn a_list_a_counted_loop_fills_is_sized_before_it() {
    let src = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable dp be a new Seq of Int.\nPush 1 to dp.\nLet mutable i be 1.\nWhile i is at most n:\n    Push 0 to dp.\n    Push i to dp.\n    Set i to i + 1.\nShow length of dp.\n";
    let code = user_code(src);
    let reserve = code
        .find("LOGOS_SEQ_RESERVE(&dp")
        .unwrap_or_else(|| panic!("the filled list is not sized before its loop:\n{code}"));
    let head = code.find("while (").expect("the loop");
    assert!(
        reserve < head,
        "the list is sized after its loop begins:\n{code}"
    );
}

/// A function's result is taken as sharing only the arguments it can hand back: a list built fresh inside the
/// function is its own, so binding it copies nothing, while a list the function returns as it came is copied
/// where the caller still reads the original (`a_list_handed_back_is_its_own`).
#[test]
fn a_fresh_result_is_bound_without_a_copy() {
    let src = "## To fresh (xs: Seq of Int) -> Seq of Int:\n    Let mutable out be a new Seq of Int.\n    Push item 1 of xs to out.\n    Return out.\n\n## Main\nLet mutable a be [1, 2].\nLet mutable b be fresh(a).\nPush 3 to b.\nPush 4 to a.\nShow a.\nShow b.\n";
    let code = user_code(src);
    assert!(
        !code.contains("__logos_copy_Seq_i64("),
        "a fresh result was copied:\n{code}"
    );
    holds(
        "a_fresh_result_is_bound_without_a_copy",
        src,
        "[1, 2, 4]\n[1, 3]\n",
        None,
    );
}

/// A shared object that counts a program's heap allocations — `malloc`, `calloc` and `realloc` calls — and writes
/// the count to the file `$ALLOCCOUNT_OUT` when the program exits: a measure of how often it allocates that no
/// load on the machine moves.
const ALLOCATION_COUNTER: &str = "#include <stddef.h>\n#include <stdio.h>\n#include <stdlib.h>\nextern void *__libc_malloc(size_t);\nextern void *__libc_calloc(size_t, size_t);\nextern void *__libc_realloc(void *, size_t);\nstatic unsigned long long calls;\nvoid *malloc(size_t n) { calls++; return __libc_malloc(n); }\nvoid *calloc(size_t a, size_t b) { calls++; return __libc_calloc(a, b); }\nvoid *realloc(void *p, size_t n) { calls++; return __libc_realloc(p, n); }\n__attribute__((destructor)) static void report(void) { const char *path = getenv(\"ALLOCCOUNT_OUT\"); FILE *f = path ? fopen(path, \"w\") : NULL; if (f) { fprintf(f, \"%llu\\n\", calls); fclose(f); } }\n";

/// How many heap allocations the C of `src` makes in one run (see [`ALLOCATION_COUNTER`]), with its stdout.
fn allocations(name: &str, src: &str) -> (u64, String) {
    let code = compile_to_c(src).expect("the program lowers to C");
    let dir = tempfile::Builder::new()
        .prefix(&format!("logos_cast_alloc_{name}_"))
        .tempdir()
        .expect("a run directory");
    std::fs::write(dir.path().join("prog.c"), &code).expect("write the C");
    std::fs::write(dir.path().join("count.c"), ALLOCATION_COUNTER).expect("write the counter");
    let built = build_release(dir.path());
    assert!(
        built.status.success(),
        "the emitted C does not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let shared = Command::new(common::c_compiler())
        .args(["-O2", "-shared", "-fPIC", "-o", "count.so", "count.c"])
        .current_dir(dir.path())
        .output()
        .expect("the C compiler starts");
    assert!(
        shared.status.success(),
        "the counter does not build:\n{}",
        String::from_utf8_lossy(&shared.stderr)
    );
    let run = Command::new(dir.path().join("prog"))
        .current_dir(dir.path())
        .env("LD_PRELOAD", dir.path().join("count.so"))
        .env("ALLOCCOUNT_OUT", dir.path().join("count.txt"))
        .stdin(Stdio::null())
        .output()
        .expect("the program starts");
    let count = std::fs::read_to_string(dir.path().join("count.txt")).expect("the counter reports");
    (
        count.trim().parse().expect("a count"),
        String::from_utf8_lossy(&run.stdout).into_owned(),
    )
}

/// A list an inner counted loop fills, once per pass of an outer loop, grows geometrically: each pass's
/// reservation at least doubles it, so the passes take a logarithmic number of allocations, not one each.
#[test]
fn a_list_reserved_again_and_again_grows_geometrically() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    let src = "## Main\nLet mutable xs be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than 3000:\n    Let mutable j be 0.\n    While j is less than 300:\n        Push j to xs.\n        Set j to j + 1.\n    Set i to i + 1.\nShow length of xs.\n";
    let (count, stdout) = allocations("reserved_again", src);
    assert_eq!(stdout, "900000\n");
    assert!(
        count < 100,
        "{count} allocations filling one list over 3000 passes"
    );
}

/// A map or set probes its table with a mask of its capacity, which is always a power of two, never with a
/// division: the maps and sets the runtime and the program's own map families carry.
#[test]
fn a_hash_table_probes_by_mask() {
    let src = "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 3 of m to 4.\nLet mutable t be a new Map of Text to Int.\nSet item \"a\" of t to 1.\nLet mutable u be a new Map of Text to Text.\nSet item \"a\" of u to \"b\".\nLet mutable v be a new Map of Int to Text.\nSet item 1 of v to \"b\".\nLet mutable w be a new Map of Text to Seq of Int.\nSet item \"a\" of w to [1].\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nLet mutable r be a new Set of Text.\nAdd \"a\" to r.\nShow length of m + length of t + length of u + length of v + length of w + length of s + length of r.\n";
    let code = compile_to_c(src).expect("the program lowers to C");
    let divisions: Vec<&str> = code
        .lines()
        .filter(|l| {
            l.contains("% cap")
                || l.contains("% nc")
                || l.contains("% m->cap")
                || l.contains("% s->cap")
        })
        .collect();
    assert!(
        divisions.is_empty(),
        "hash tables probing by division:\n{}",
        divisions.join("\n")
    );
}

/// An exact integer's arithmetic runs its machine-word path inline, so a loop over one that stays small costs a
/// check per step, not a call: the Collatz walk's `k` is exact (nothing bounds `3 * k + 1`), and its loop calls
/// no out-of-line exact operation.
#[test]
fn an_exact_integer_runs_its_word_path_inline() {
    let src = "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 1000:\n    Let mutable k be i.\n    While k is not 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n        Set total to total + 1.\nShow total.\n";
    let code = user_code(src);
    assert!(
        code.contains("LInt k"),
        "the Collatz walk's `k` is no longer exact:\n{code}"
    );
    let calls: Vec<&str> = [
        "lint_add(",
        "lint_sub(",
        "lint_mul(",
        "lint_div(",
        "lint_mod(",
        "lint_cmp(",
    ]
    .into_iter()
    .filter(|call| code.split(call).count() > code.split(&format!("logos_{call}")).count())
    .collect();
    assert!(
        calls.is_empty(),
        "the loop calls the out-of-line exact {calls:?}:\n{code}"
    );
}

/// A list a loop hands to a function that changes it, and takes back, is changed in place: the function owns
/// the list the caller no longer reads. A copy per call needs gigabytes for these loops and dies at the cap.
const LINEAR_CALLS: &[(&str, &str, &str, u64)] = &[
    (
        "rebind_to_the_callees_result",
        "## To bump (xs: Seq of Int, i: Int) -> Seq of Int:\n    Let mutable ys be xs.\n    Set item i of ys to item i of ys + 1.\n    Return ys.\n\n## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 20000:\n    Push 0 to xs.\nRepeat for i from 1 to 20000:\n    Set xs to bump(xs, i).\nLet mutable total be 0.\nRepeat for x in xs:\n    Set total to total + x.\nShow total.\n",
        "20000\n",
        256,
    ),
    (
        "recursive_partition_in_place",
        "## To part (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:\n    If lo is at least hi:\n        Return arr.\n    Let mutable result be arr.\n    Set item lo of result to item lo of result + 1.\n    Let mid be (lo + hi) / 2.\n    Set result to part(result, lo, mid).\n    Set result to part(result, mid + 1, hi).\n    Return result.\n\n## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 20000:\n    Push 0 to xs.\nSet xs to part(xs, 1, 20000).\nLet mutable total be 0.\nRepeat for x in xs:\n    Set total to total + x.\nShow total.\n",
        "19999\n",
        256,
    ),
];

#[test]
fn a_list_handed_through_calls_is_changed_in_place() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, src, stdout, cap_mb) in LINEAR_CALLS {
        let reference = vm(src);
        assert_eq!(
            reference,
            Observed::of_stream(stdout.to_string(), None),
            "the VM no longer runs `{name}` as pinned:\n{src}"
        );
        match c_capped(name, src, *cap_mb) {
            Ok(got) => assert_eq!(
                got, reference,
                "the C tier runs `{name}` otherwise than the VM within {cap_mb} MB:\n{src}"
            ),
            Err(why) => panic!("the C tier does not run `{name}`: {why}\n{src}"),
        }
    }
}

#[test]
fn a_text_built_by_appending_grows_linearly() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, src, stdout, cap_mb) in LINEAR_TEXT {
        let reference = vm(src);
        assert_eq!(
            reference,
            Observed::of_stream(stdout.to_string(), None),
            "the VM no longer runs `{name}` as pinned:\n{src}"
        );
        match c_capped(name, src, *cap_mb) {
            Ok(got) => assert_eq!(
                got, reference,
                "the C tier runs `{name}` otherwise than the VM within {cap_mb} MB:\n{src}"
            ),
            Err(why) => panic!("the C tier does not run `{name}`: {why}\n{src}"),
        }
    }
}

/// The knapsack inner loop is the motivating split-loop shape: `w + 1` and
/// `w - wi + 1` are machine-word indexes once the branch proves `w >= wi`.
/// Direct C must consume that proof in the hot loop instead of rebuilding an
/// exact `LInt` counter on every iteration.
#[test]
fn a_guarded_counted_loop_keeps_split_indexes_on_machine_words() {
    let src = include_str!("../../../benchmarks/programs/knapsack/main.lg");
    let code = user_code(src);
    let guard = "&& wi >= 0 && w.small >= wi";
    let fast = code
        .split_once(guard)
        .and_then(|(_, rest)| rest.split_once("} else {").map(|(fast, _)| fast))
        .expect("the split loop carries its lower-bound proof into direct C");
    assert!(
        fast.contains(" - wi) + 1LL"),
        "the guarded split loop did not use native indexes:\n{fast}"
    );
    assert!(
        !fast.contains("lint_index_i64")
            && !fast.contains("logos_lint_add")
            && !fast.contains("logos_lint_sub"),
        "the guarded split loop rebuilt exact index arithmetic:\n{fast}"
    );
}

/// The word-index path is only a representation specialization.  If its
/// lower-bound guard is false, the original exact loop must still promote an
/// index expression beyond `i64` exactly as the VM does.
#[test]
fn a_counted_loop_with_a_negative_floor_uses_its_exact_fallback() {
    let src = "## To native args () -> Seq of Text\n\n## Main\nLet arguments be args().\nLet starts be [9223372036854775806].\nLet floors be [0 - 1].\nLet floor be item 1 of floors.\nLet mutable i be item 1 of starts.\nIf length of arguments is greater than 100:\n    Set i to i * i.\nLet mutable answer be 0.\nWhile i is less than 9223372036854775807:\n    Set answer to i - floor + 1.\n    Set i to i + 1.\nShow answer.\n";
    let code = user_code(src);
    assert!(
        code.contains("&& logos_floor >= 0 && i.small >= logos_floor"),
        "the native loop is missing its exactness guard:\n{code}"
    );
    assert!(
        code.contains("logos_lint_sub(i"),
        "the cold loop lost its exact subtraction fallback:\n{code}"
    );
    holds(
        "counted_loop_negative_floor",
        src,
        "9223372036854775808\n",
        None,
    );
}

#[test]
fn ast_random_integer_bounds_and_failures_match_the_vm() {
    for (name, source, output, fails) in [
        ("singleton", "## Main\nShow randomInt(3, 3).\nShow randomInt(-9223372036854775807, -9223372036854775807).\nShow randomInt(9223372036854775807, 9223372036854775807).\n", "3\n-9223372036854775807\n9223372036854775807\n", false),
        ("range", "## Main\nLet x be randomInt(-5, 6).\nShow x is at least -5 and x is at most 6.\n", "true\n", false),
        ("crossed", "## Main\nShow 1.\nShow randomInt(5, 2).\n", "1\n", true),
    ] {
        let expected = vm(source);
        assert_eq!(expected.stdout, output, "{name}: independent output control");
        assert_eq!(expected.end, if fails { End::Failed } else { End::Finished });
        assert_eq!(c(name, source).expect("randomInt C compiles and runs"), expected, "{name}");
    }
}

#[test]
fn ast_random_integer_declaration_takes_precedence_over_the_builtin() {
    let source = "## To randomInt (lo: Int) and (hi: Int) -> Int:\n    Return lo + hi.\n\n## Main\nShow randomInt(5, 6).\n";
    let expected = vm(source);
    assert_eq!(expected.stdout, "11\n");
    assert_eq!(expected.end, End::Finished);
    assert_eq!(
        c("declared_random", source).expect("declared function compiles"),
        expected
    );
}

// ── A name bound by `Let` in several blocks is a local of each, typed by the value each binds it to: a real one ──
// ── read into a real that started as a real is a real, never an integer phase of it.                          ──
rows! {
    a_running_best_reads_an_accumulator_each_loop_binds_afresh:
        "## Main\nLet best be 0.0 - 1000.0.\nLet bestIdx be 0.\nLet o be 0.\nWhile o is less than 3:\n    Let acc be 0.0.\n    Let i be 0.\n    While i is less than 2:\n        Set acc to acc + 1.5 * (o + i).\n        Set i to i + 1.\n    If acc is greater than best:\n        Set best to acc.\n        Set bestIdx to o.\n    Set o to o + 1.\nLet k be 0.\nWhile k is less than 2:\n    Let acc be 0.5.\n    Set k to k + 1.\nShow best.\nShow bestIdx.\n"
        => "7.5\n2\n";
    a_name_each_loop_binds_to_an_integer_widens_a_real_only_where_its_integer_phase_is_hidden:
        "## Main\nLet mutable total be 0.5.\nRepeat for i from 1 to 2:\n    Let step be i * 2.\n    Set total to total + step.\nRepeat for j from 1 to 2:\n    Let step be j.\n    Set total to total + step.\nShow total.\n"
        => "9.5\n";
}
