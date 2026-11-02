#![no_main]
//! Fuzz the lexer (Invariant I): no byte string may panic / abort / stack-overflow it.
//! The lexer's byte-index slicing on truncated or multibyte input was where Wave 1's
//! front-door crashes hid, so this is the highest-ROI target. A crash libFuzzer finds is
//! minimized and committed under `corpus/`/`regressions/` and read back by
//! `never_panic_lock.rs` forever (Invariant VII).

use libfuzzer_sys::fuzz_target;
use logicaffeine_base::Interner;
use logicaffeine_language::lexer::Lexer;

fuzz_target!(|data: &[u8]| {
    // Lossy decode so raw byte sequences (incl. broken multibyte) reach the lexer's
    // char/byte-boundary handling exactly as the never-panic lock feeds them.
    let s = String::from_utf8_lossy(data);
    let mut interner = Interner::new();
    let _ = Lexer::new(&s, &mut interner).tokenize();
});
