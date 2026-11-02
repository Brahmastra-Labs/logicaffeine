#![no_main]
//! Fuzz the full front-end: English → First-Order-Logic (lex → parse → transpile).
//! Must never panic on any input — it returns a structured `ParseError` or an FOL string.
//! This is where the deep-nesting stack-overflow lived (fixed by the `parse_sentence`
//! recursion guard); libFuzzer keeps the whole descent honest.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    let _ = logicaffeine_language::compile(&s);
});
