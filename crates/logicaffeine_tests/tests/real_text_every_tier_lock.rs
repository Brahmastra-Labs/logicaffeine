//! ★ LOCK — a `Real` reads as one text however it is shown, on every backend. ★
//!
//! A `Real` shows as the shortest decimal that reads back as the same double, with no exponent: `0.1 + 0.2` is
//! `0.30000000000000004`, π is `3.141592653589793`, a ten-millionth is `0.0000001`. That text is the value's
//! wherever it appears — shown on its own, in a bare interpolation hole, padded to a width in a hole (right, left,
//! centred, or by a bare width), as a record's field, as a list's item and as a map's value, keyed by text or by an
//! integer — and on every backend: a backend that renders any of them with C's six-digit `%g` prints `3.14159` for π,
//! and one that holds a map's `Real` values as integers prints `3`. Each value is written as a literal, which the
//! optimizing tiers fold, and computed from a number read at run time. The tree-walker prints what each program says
//! here, and every backend prints its bytes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test real_text_every_tier_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::tw_outcome;
use tier_parity::{Backend, Program};

/// Each value, written as a literal or an expression over literals, and its text.
const VALUES: &[(&str, &str, &str)] = &[
    ("pi", "3.141592653589793", "3.141592653589793"),
    ("a_tenth_and_a_fifth", "0.1 + 0.2", "0.30000000000000004"),
    ("two_thirds", "2.0 / 3.0", "0.6666666666666666"),
    ("nine_digits_and_an_eighth", "123456789.125", "123456789.125"),
    ("a_ten_millionth", "0.0000001", "0.0000001"),
    ("one_and_a_half", "1.5", "1.5"),
];

const NATIVE: &str = "## To native parseInt (s: Text) -> Int\n\n";

/// The value shown on its own, in a hole, padded in a hole, as a record's field, as a list's items and as a map's
/// value. `x` is bound to `value`.
fn shown(value: &str) -> String {
    format!(
        "## A Point has:\n    An x: Real.\n\n## Main\nLet x be {value}.\nShow x.\nShow \"{{x}}\".\nShow \"x = {{x}}!\".\n\
         Show \"[{{x:>22}}]\".\nShow \"[{{x:<22}}]\".\nShow \"[{{x:^22}}]\".\nShow \"[{{x:22}}]\".\n\
         Let p be a new Point with x x.\nShow p.\nShow [x, x].\nLet mutable m be a new Map of Text to Real.\n\
         Set item \"k\" of m to x.\nShow m.\nLet mutable n be a new Map of Int to Real.\nSet item 7 of n to x.\nShow n.\n"
    )
}

/// What [`shown`] prints for a value whose text is `text`.
fn printed(text: &str) -> String {
    format!(
        "{text}\n{text}\nx = {text}!\n[{text:>22}]\n[{text:<22}]\n[{text:^22}]\n[{text:>22}]\nPoint {{ x: {text} }}\n\
         [{text}, {text}]\n{{k: {text}}}\n{{7: {text}}}\n"
    )
}

/// Each value twice: as written, and multiplied by a one read at run time, so no tier can fold it.
fn cases() -> Vec<(String, String, String)> {
    VALUES
        .iter()
        .flat_map(|(id, value, text)| {
            [
                (format!("real_text/{id}"), shown(value), printed(text)),
                (
                    format!("real_text/{id}/at_run_time"),
                    format!("{NATIVE}{}", shown(&format!("({value}) * parseInt(\"1\")"))),
                    printed(text),
                ),
            ]
        })
        .collect()
}

/// Every backend that runs a program: the engines, the generated Rust, both C lowerings, both WebAssembly modules
/// and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> = cases()
        .into_iter()
        .filter(|(_, source, _)| tier_parity::rejection(source).is_some())
        .map(|(id, _, _)| id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn the_tree_walker_shows_each_value_as_its_shortest_decimal_everywhere() {
    for (id, source, text) in cases() {
        let run = tw_outcome(&source);
        assert_eq!(run.error, None, "{id}: the tree-walker refused:\n{source}");
        assert_eq!(run.output.trim_end(), text.trim_end(), "{id}:\n{source}");
    }
}

#[test]
fn every_backend_prints_the_tree_walkers_bytes() {
    let programs: Vec<Program> = cases().into_iter().map(|(id, source, _)| Program::new(id, source)).collect();
    let rows = tier_parity::sweep(&programs, BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the tree-walker:\n{}", bad.len(), bad.join("\n"));
}
