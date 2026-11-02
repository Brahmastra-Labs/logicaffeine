//! The bounded-accumulator pass's interval analysis stays inside `i128` on every expression, and a loop it
//! looks at still prints what the VM prints.
//!
//! `optimize::accum_bound` bounds each arithmetic node of a `While` loop in `i128` under "every variable is
//! in `[-B, B]`" (`B` up to `2^62 - 1`). A product of that band with an `i64`-edge literal is already near
//! `2^125`; one more edge factor leaves `i128`. Each row here is a loop whose accumulator is a nest of
//! `+`, `-`, `*`, `/`, `%` and `abs` over a parameter and edge literals (`i64::MAX`, `i64::MIN`), at depths
//! 2 to 4, in the shapes the pass takes: a closed update after a `Let` preamble, and a linear `sum + term`
//! accumulator. Every row must compile through the optimizer and its optimized program must print, on the
//! VM, exactly what the unoptimized VM prints for the source. Every row leaves `i64` under every candidate
//! band, so none may be versioned onto raw arithmetic: the AOT backends would run it past the word.
//!
//! The analysis must still accept what it can prove: loops whose every bound stays inside `i64` are versioned
//! onto raw arithmetic (the pass is seen firing) and print what the VM prints.

use logicaffeine_compile::compile::{compile_to_program_optimized, program_outcome, vm_outcome};
use logicaffeine_compile::optimize::{begin_fired_trace, end_fired_trace};
use logicaffeine_language::optimization::Opt;

const MAX: &str = "9223372036854775807";
const MIN: &str = "(0 - 9223372036854775807 - 1)";

/// The accumulator update of the closed shape: `s` is set to `rhs` on every trip of a three-trip loop. The
/// counter steps modulo 1000, so its own update stays inside the band and the loop is the analysis's to accept
/// or refuse on `rhs` alone.
fn closed(rhs: &str) -> String {
    format!(
        "## To f (x: Int) -> Int:\n    Let mutable s be x.\n    Let mutable c be 0.\n    While c is less than 3:\n        Set s to {rhs}.\n        Set c to (c + 1) % 1000.\n    Return s.\n\n## Main\n    Show f(2).\n    Show f(0 - 3).\n    Show f(0).\n"
    )
}

/// The linear shape: `sum` grows by `term` on each of `n` trips.
fn linear(term: &str) -> String {
    format!(
        "## To f (x: Int, n: Int) -> Int:\n    Let mutable sum be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Set sum to sum + {term}.\n        Set i to i + 1.\n    Return sum.\n\n## Main\n    Show f(2, 3).\n    Show f(0 - 3, 2).\n    Show f(0, 4).\n"
    )
}

/// Every left-nested chain `((x op L) op L) …` of `depth` operators from `+ - *`, over the literal `lit`.
fn chains(depth: usize, lit: &str) -> Vec<String> {
    let mut out = vec!["x".to_string()];
    for _ in 0..depth {
        out = out.iter().flat_map(|e| ["+", "-", "*"].map(|op| format!("({e} {op} {lit})"))).collect();
    }
    out
}

/// The expressions the lock runs: chains at depths 2 to 4 over both edges; sums, differences and products
/// of two depth-2 chains; `abs` of the deepest products; and quotients and remainders of the chains with a
/// product, whose interval the analysis resets to the band, so only the escape mark on the product inside
/// keeps the loop from being versioned onto raw arithmetic.
fn family() -> Vec<String> {
    let mut rows = Vec::new();
    for depth in 2..=4 {
        for lit in [MAX, MIN] {
            rows.extend(chains(depth, lit));
        }
    }
    let two = chains(2, MAX);
    for a in &two {
        for b in &two {
            for op in ["+", "-", "*"] {
                // `a - a` simplifies to 0 before the pass sees it, and 0 is bounded.
                if !(op == "-" && a == b) {
                    rows.push(format!("({a} {op} {b})"));
                }
            }
        }
    }
    for e in chains(3, MAX).into_iter().filter(|e| e.matches('*').count() >= 2) {
        rows.push(format!("abs({e})"));
    }
    for depth in 2..=3 {
        for lit in [MAX, MIN] {
            for e in chains(depth, lit).into_iter().filter(|e| e.contains('*')) {
                rows.push(format!("({e} / 7)"));
                rows.push(format!("({e} % 1000)"));
            }
        }
    }
    rows
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "a non-string panic".to_string())
}

/// What one loop did: whether the pass versioned it, and how its optimized run differs from the VM's (`None`
/// when it prints exactly what the VM prints; a panic in the optimizer or in the optimized run is named).
struct Verdict {
    versioned: bool,
    differs: Option<String>,
}

fn judge(src: &str) -> Verdict {
    let want = vm_outcome(src);
    begin_fired_trace();
    let compiled = std::panic::catch_unwind(|| compile_to_program_optimized(src));
    let versioned = end_fired_trace().is_some_and(|f| f.fired(Opt::AccumBound));
    let program = match compiled {
        Err(panic) => return Verdict { versioned, differs: Some(format!("the optimizer panicked: {}", panic_message(panic))) },
        Ok(Err(e)) => return Verdict { versioned, differs: Some(format!("does not compile: {e:?}")) },
        Ok(Ok(program)) => program,
    };
    let differs = match std::panic::catch_unwind(|| program_outcome(&program)) {
        Err(panic) => Some(format!("the optimized program panicked: {}", panic_message(panic))),
        Ok(got) => (got.output != want.output || got.error != want.error)
            .then(|| format!("optimized {:?} / {:?}, VM {:?} / {:?}", got.output, got.error, want.output, want.error)),
    };
    Verdict { versioned, differs }
}

/// Every loop must print what the VM prints, and none may be versioned: each accumulator escapes `i64` under
/// every candidate band, so a versioned one would run raw arithmetic past the word.
fn sweep(programs: impl IntoIterator<Item = (String, String)>) {
    let mut bad = Vec::new();
    let mut total = 0;
    for (what, src) in programs {
        total += 1;
        let v = judge(&src);
        if let Some(why) = v.differs {
            bad.push(format!("{what}: {why}"));
        } else if v.versioned {
            bad.push(format!("{what}: versioned onto raw arithmetic although it leaves i64 under every band"));
        }
    }
    assert!(bad.is_empty(), "{} of {total} loops fail:\n{}", bad.len(), bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// Loops every bound of which stays inside `i64` under some candidate band: the pass versions each.
fn bounded() -> Vec<(String, String)> {
    let closed_rows = ["(x / 3 + 5) / 7", "((x % 1000) * (x % 1000)) / 7", "(x - 100) / 2", "(abs(x % 1000) * 3) / 7"];
    let linear_rows = ["(x % 1000)", "((x / 7) * 3)", "(x % 1000 + 5)", "((x % 1000) * 3)"];
    closed_rows
        .iter()
        .map(|e| (format!("closed {e}"), closed(e)))
        .chain(linear_rows.iter().map(|e| (format!("linear {e}"), linear(e))))
        .collect()
}

#[test]
fn every_bounded_accumulator_is_still_versioned_and_prints_what_the_vm_prints() {
    let mut bad = Vec::new();
    for (what, src) in bounded() {
        let v = judge(&src);
        if !v.versioned {
            bad.push(format!("{what}: the pass did not version the loop"));
        }
        if let Some(why) = v.differs {
            bad.push(format!("{what}: {why}"));
        }
    }
    assert!(bad.is_empty(), "{} bounded loops fail:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_nested_edge_literal_product_compiles_and_prints_what_the_vm_prints() {
    sweep([("(x * MAX) * MAX".to_string(), closed(&format!("(x * {MAX}) * {MAX}")))]);
}

#[test]
fn every_closed_accumulator_over_edge_literals_stays_guarded_and_prints_what_the_vm_prints() {
    sweep(family().into_iter().map(|e| (format!("closed {e}"), closed(&e))));
}

#[test]
fn every_linear_accumulator_over_edge_literals_stays_guarded_and_prints_what_the_vm_prints() {
    sweep(family().into_iter().map(|e| (format!("linear {e}"), linear(&e))));
}
