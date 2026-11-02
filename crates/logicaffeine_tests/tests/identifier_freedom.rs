//! Identifier freedom: everyday English words must work as variable names.
//!
//! LOGOS keywordizes many common words in imperative mode (`size` for zones,
//! `launch` for tasks, `shifted` for bit-shifts, …). A user writing
//! `Let size be 5.` is not thinking about zone syntax — the word must simply
//! work, in the binder position AND in every reference position (Set targets,
//! interpolation holes, arithmetic, call arguments, loop binders, struct
//! fields). This battery locks that freedom so a new keyword can never again
//! silently steal a name from user programs.
//!
//! Deliberate exclusions (documented, not bugs): literal words (`true`,
//! `false`, `nothing`, `none`, number-words like `one`/`two` — you cannot name
//! a variable `5` either), expression operators that read as prefixes
//! (`length`, `copy`, `item`, `items`, `some`), and the statement skeleton
//! itself (`let`, `set`, `be`, `to`, `if`, `while`, …).

use logicaffeine_compile::{generate_rust_code, interpret_for_ui_baseline_with_args};

fn run(src: &str) -> Result<String, String> {
    if let Err(e) = generate_rust_code(src) {
        return Err(format!("codegen error: {e:?}"));
    }
    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(interpret_for_ui_baseline_with_args(src, &[]));
    match result.error {
        Some(e) => Err(format!("interpreter error: {e}")),
        None => Ok(result.lines.join("\n")),
    }
}

/// Every word here must survive the full round trip:
/// bind → mutate → reference in a hole → reference in arithmetic.
const FREED_WORDS: &[&str] = &[
    // zone / memory syntax words
    "size",
    "zone",
    "inside",
    "called",
    // concurrency syntax words
    "launch",
    "task",
    "pipe",
    "stop",
    "receive",
    "send",
    "spawn",
    "await",
    "attempt",
    "following",
    // networking / persistence syntax words
    "listen",
    "sync",
    "mount",
    // operator / collection syntax words
    "shifted",
    "union",
    "intersection",
    "contains",
    "through",
    "push",
    "pop",
    // IO / misc syntax words
    "try",
    "new",
    "inspect",
    "check",
    // degree morphology — comparatives must name variables/params/functions/types too
    // (`upper`/`lowest` already worked via Adjective/Superlative; `lower`/`higher` are
    // Comparative tokens and were the last degree words still stealing names).
    "lower",
    "higher",
    "greater",
    "smaller",
    "larger",
];

#[test]
fn every_freed_word_binds_mutates_and_references() {
    let mut failures = Vec::new();
    for word in FREED_WORDS {
        let src = format!(
            "## Main\nLet mutable {word} be 7.\nSet {word} to {word} + 1.\nShow \"{{{word}}}\".\nLet doubled be {word} * 2.\nShow \"{{doubled}}\".\n"
        );
        match run(&src) {
            Ok(out) if out == "8\n16" => {}
            Ok(out) => failures.push(format!("[{word}] wrong output: {out:?}")),
            Err(e) => failures.push(format!("[{word}] {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} freed word(s) failed the round trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn comparatives_work_as_function_names() {
    // `lower`/`higher` lex as Comparative (degree morphology). They must be legal
    // function names — the stdlib `strings.lg` defines `## To lower`, and a user's
    // declaration always wins in name position.
    let src = "## To lower (x: Int) -> Int:\n    Return x - 1.\n## To higher (x: Int) -> Int:\n    Return x + 1.\n\n## Main\nShow lower(5).\nShow higher(5).\n";
    match run(src) {
        Ok(out) => assert_eq!(out, "4\n6", "comparative function names must work"),
        Err(e) => panic!("comparative function names failed: {e}"),
    }
}

#[test]
fn freed_words_work_as_loop_binders_and_params() {
    let src = "## To pay (size: Int) and (task: Int) -> Int:\n    Return size + task.\n\n## Main\nLet mutable total be 0.\nRepeat for launch in [1, 2, 3]:\n    Set total to total + launch.\nShow \"{total}\".\nLet fee be pay(10, 5).\nShow \"{fee}\".\n";
    match run(src) {
        Ok(out) => assert_eq!(out, "6\n15", "wrong output"),
        Err(e) => panic!("loop-binder/param freedom failed: {e}"),
    }
}

#[test]
fn freed_words_work_as_struct_fields() {
    let src = "## Definition\n\nA Rocket has:\n    size: Int.\n    task: Text.\n\n## Main\n\nLet ship be a new Rocket with size 9 and task \"orbit\".\nShow \"{ship's size} for {ship's task}\".\nSet ship's size to 10.\nShow \"{ship's size}\".\n";
    match run(src) {
        Ok(out) => assert_eq!(out, "9 for orbit\n10", "wrong output"),
        Err(e) => panic!("struct-field freedom failed: {e}"),
    }
}

/// Words keywordized for imperative FEATURES (comparison phrases like `is at most`, chunk
/// access `at`, aggregation `count`, equality `equal`) must still name variables, parameters,
/// and functions. A stdlib author writing `(at: Int)` or `Let count be 0` is not invoking an
/// operator — in a NAME position, keywords are contextual, never reserved. This is the
/// universal guarantee `expect_identifier`'s word-keyword catch-all provides.
const FEATURE_KEYWORDS: &[&str] = &[
    "at", "count", "most", "least", "equal", "mod", "than", "match", "step", "day", "week",
    "month", "year", "state", "field", "value", "key", "index",
];

#[test]
fn feature_keywords_name_variables() {
    let mut failures = Vec::new();
    for word in FEATURE_KEYWORDS {
        let src = format!(
            "## Main\nLet mutable {word} be 7.\nSet {word} to {word} + 1.\nShow \"{{{word}}}\".\n"
        );
        match run(&src) {
            Ok(out) if out == "8" => {}
            Ok(out) => failures.push(format!("[{word}] var: wrong output {out:?}")),
            Err(e) => failures.push(format!("[{word}] var: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} feature keyword(s) failed as variable names:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn feature_keywords_name_parameters() {
    // The EXACT shape that bit bytes.lg: a keyword as a function parameter name
    // (`## To bytesMatchAt (… and (at: Int) …)`).
    let mut failures = Vec::new();
    for word in FEATURE_KEYWORDS {
        let src = format!(
            "## To probe ({word}: Int) -> Int:\n    Return {word} + 1.\n\n## Main\nShow probe(41).\n"
        );
        match run(&src) {
            Ok(out) if out == "42" => {}
            Ok(out) => failures.push(format!("[{word}] param: wrong output {out:?}")),
            Err(e) => failures.push(format!("[{word}] param: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} feature keyword(s) failed as parameter names:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn feature_keywords_name_functions() {
    let mut failures = Vec::new();
    for word in FEATURE_KEYWORDS {
        let src = format!(
            "## To {word} (x: Int) -> Int:\n    Return x + 1.\n\n## Main\nShow {word}(41).\n"
        );
        match run(&src) {
            Ok(out) if out == "42" => {}
            Ok(out) => failures.push(format!("[{word}] fn: wrong output {out:?}")),
            Err(e) => failures.push(format!("[{word}] fn: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} feature keyword(s) failed as function names:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn freed_words_work_as_type_names() {
    let src = "## A Status is one of:\n    An Open.\n    A Done.\n\n## Definition\n\nA Task has:\n    label: Text.\n    status: Status.\n\n## Main\n\nLet mutable chores be a new Seq of Task.\nLet milk be a new Task with label \"buy milk\" and status a new Open.\nPush milk to chores.\nLet only be item 1 of chores.\nShow \"{only's label}\".\nInspect only's status:\n    When Open: Show \"still open\".\n    When Done: Show \"done\".\n";
    match run(src) {
        Ok(out) => assert_eq!(out, "buy milk\nstill open", "wrong output"),
        Err(e) => panic!("type-name freedom failed: {e}"),
    }
}
