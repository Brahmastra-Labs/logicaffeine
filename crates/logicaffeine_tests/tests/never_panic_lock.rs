//! ════════════════════════════════════════════════════════════════════════════
//! NEVER-PANIC LOCK.
//!
//! No byte string fed to a PUBLIC ENTRY POINT — valid, malformed, adversarial,
//! truncated, non-UTF-8, or pathologically nested — may panic, abort, or
//! stack-overflow. Every input must either return a value or a structured error.
//! A panic on any input is the single easiest way to win the "break my code"
//! challenge, so this file is the front-door armour.
//!
//! HOUSE STYLE (mirrors wasm_aot_lock.rs):
//!   • `enum Surface` is an EXHAUSTIVE catalog of every public entry point. The
//!     `dispatch` match has NO wildcard arm — adding a new front door FAILS THE
//!     BUILD until it is classified and swept here. No attack surface can be
//!     added to the language without being placed under the never-panic invariant.
//!   • Every crash a fuzzer or the challenge ever finds is minimized into
//!     `curated_corpus()` as a permanent seed (Invariant VII) — a fixed panic can
//!     never silently regress.
//!
//! What this proves: no *unwinding panic* on the swept corpus. Out-of-process
//! aborts / genuine stack overflows are the job of the nightly cargo-fuzz targets
//! (Wave 5); `catch_unwind` here cannot catch those. The Honesty Ledger (§7) is
//! explicit about the boundary.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use logicaffeine_base::Interner;
use logicaffeine_compile::compile::{compile_to_rust, compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::concurrency::marshal::{
    decode_value_raw, message_from_wire, parse_handshake_frame,
};
use logicaffeine_language::lexer::Lexer;

// ───────────────────────────── panic-classifying probe ─────────────────────────────

/// The only forbidden outcome is `Panicked`. `Returned` covers BOTH a clean value
/// and a clean structured error — both are acceptable; the invariant is only that
/// the entry point returned control at all.
#[derive(Debug)]
enum Verdict {
    Returned,
    Panicked(String),
}

thread_local! {
    /// The panic hook records the location+message here so `probe` can report
    /// WHERE the panic fired, not just that one happened. Thread-local so
    /// concurrent probes never clobber each other.
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

static HOOK: Once = Once::new();

/// Install a global panic hook ONCE that (a) captures the panic site into a
/// thread-local and (b) suppresses the default backtrace spew — so a 5,000-seed
/// sweep does not print 5,000 stack traces. The hook only records; `catch_unwind`
/// still drives control flow.
fn install_quiet_hook() {
    HOOK.call_once(|| {
        panic::set_hook(Box::new(|info| {
            let loc = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "<unknown>".to_string());
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "<non-string panic payload>".to_string()
            };
            LAST_PANIC.with(|c| *c.borrow_mut() = Some(format!("{loc} — {msg}")));
        }));
    });
}

/// Run `f` (a front-door invocation) and classify: did it return, or panic?
fn probe<F: FnOnce()>(f: F) -> Verdict {
    install_quiet_hook();
    LAST_PANIC.with(|c| *c.borrow_mut() = None);
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => Verdict::Returned,
        Err(_) => {
            let site = LAST_PANIC
                .with(|c| c.borrow_mut().take())
                .unwrap_or_else(|| "<hook did not capture>".to_string());
            Verdict::Panicked(site)
        }
    }
}

// ───────────────────────────── the exhaustive surface catalog ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    /// English → FOL (the Logic pipeline front door).
    Fol,
    /// The lexer, hit directly — its byte-slicing is the likeliest panic surface.
    Lex,
    /// English program → tree-walking interpreter (lex + parse + typecheck + run).
    Interpret,
    /// English program → bytecode VM.
    Vm,
    /// English program → generated Rust source (codegen, no execution).
    CompileRust,
    /// English program → WASM bytes (AOT emitter, no execution).
    CompileWasm,
    /// Untrusted wire frame → (channel, value).
    WireMessage,
    /// Untrusted wire bytes → a single runtime value.
    WireValue,
    /// Untrusted handshake frame → (peer id, profile).
    WireHandshake,
}

/// The single source of truth for the catalog. Add a new public entry point here
/// AND in `dispatch` (whose no-wildcard match will refuse to compile until you do).
fn all_surface_variants() -> Vec<Surface> {
    use Surface::*;
    vec![
        Fol,
        Lex,
        Interpret,
        Vm,
        CompileRust,
        CompileWasm,
        WireMessage,
        WireValue,
        WireHandshake,
    ]
}

/// Feed `input` to `surface` inside the panic probe. NO WILDCARD ARM — this is the
/// build-time completeness guard (Invariant V).
fn dispatch(surface: Surface, input: &[u8]) -> Verdict {
    let text = String::from_utf8_lossy(input);
    match surface {
        Surface::Fol => probe(|| {
            let _ = logicaffeine_language::compile(&text);
        }),
        Surface::Lex => probe(|| {
            let mut interner = Interner::new();
            let _ = Lexer::new(&text, &mut interner).tokenize();
        }),
        Surface::Interpret => probe(|| {
            let _ = tw_outcome(&text);
        }),
        Surface::Vm => probe(|| {
            let _ = vm_outcome(&text);
        }),
        Surface::CompileRust => probe(|| {
            let _ = compile_to_rust(&text);
        }),
        Surface::CompileWasm => probe(|| {
            let _ = compile_to_wasm(&text);
        }),
        Surface::WireMessage => probe(|| {
            let _ = message_from_wire(input);
        }),
        Surface::WireValue => probe(|| {
            let _ = decode_value_raw(input);
        }),
        Surface::WireHandshake => probe(|| {
            let _ = parse_handshake_frame(input);
        }),
    }
}

// ───────────────────────────── the adversarial corpus ─────────────────────────────

/// A curated corpus of inputs designed to break the front door. Every entry is a
/// permanent seed: when the challenge or a fuzzer finds a new crasher, its
/// minimized form is added here forever (Invariant VII).
fn curated_corpus() -> Vec<Vec<u8>> {
    let mut c: Vec<Vec<u8>> = Vec::new();
    let push = |c: &mut Vec<Vec<u8>>, s: &str| c.push(s.as_bytes().to_vec());

    // Emptiness & whitespace
    push(&mut c, "");
    push(&mut c, " ");
    push(&mut c, "   \n\t\r  ");
    push(&mut c, "\n\n\n\n");

    // Unbalanced / lone delimiters
    for s in [
        "(", ")", "((((((((", "))))", "[", "]", "{", "}", "\"", "'", "`",
    ] {
        push(&mut c, s);
    }
    push(&mut c, "\"unterminated string literal");
    push(&mut c, "\"nested \\\" quote");
    push(&mut c, "((((((((((((((((((((((((((((((");

    // Escapes & interpolation fragments
    for s in [
        "\\",
        "\\\\\\\\",
        "\\n\\t\\",
        "Print \"\\",
        "Print \"\\(",
        "Print \"${",
        "\"\\(x",
    ] {
        push(&mut c, s);
    }

    // Lone punctuation / operator soup
    for s in [
        ".", "...", ",", ";", ":", "::", "=", "==", "!=", "<>", "+-*/%", "->", "=>",
    ] {
        push(&mut c, s);
    }

    // Control characters & ANSI
    c.push(vec![0, 0, 0]);
    c.push(vec![0x01, 0x02, 0x03, 0x1b, b'[', b'3', b'1', b'm']);
    c.push(vec![0x07, 0x08, 0x0c, 0x7f]);

    // Invalid / truncated UTF-8 (bytes that String::from_utf8_lossy mangles)
    c.push(vec![0xff, 0xfe, 0xfd]);
    c.push(vec![0xc3, 0x28]); // invalid 2-byte sequence
    c.push(vec![0xe2, 0x82]); // truncated 3-byte
    c.push(vec![0xf0, 0x9f, 0x98]); // truncated 4-byte emoji
    c.push(vec![0xed, 0xa0, 0x80]); // surrogate half
    c.push(b"caf\xc3".to_vec()); // "caf" + dangling continuation byte

    // Valid multibyte (real Unicode the lexer must slice on char, not byte, boundaries)
    for s in [
        "café",
        "日本語のテスト",
        "🎉🎉🎉",
        "e\u{0301}\u{0301}\u{0301}",
        "𝕏𝕐𝕫",
        "\u{200b}\u{200b}",
        "Ａ Ｂ Ｃ",
    ] {
        push(&mut c, s);
    }

    // Keyword soup (all-keywords, no structure)
    push(
        &mut c,
        "Let be to if while for the a an Print Show Define Return",
    );
    push(&mut c, "the the the the of of of a a a");
    push(&mut c, "Let x be");
    push(&mut c, "Define to");
    push(&mut c, "Show the of a");
    push(&mut c, "If then else");

    // Numeric edge cases
    push(&mut c, "0x");
    push(&mut c, "0b");
    push(&mut c, "0o");
    push(&mut c, "0xZZZZ");
    push(&mut c, "1.2.3.4");
    push(&mut c, &"9".repeat(400));
    push(&mut c, &format!("{}e{}", "9".repeat(300), "9".repeat(300)));
    push(&mut c, "1_000_000_");
    push(&mut c, "-.-.-.");

    // Huge & deeply-nested (must trip the ast_depth gate cleanly, not overflow)
    c.push(vec![b'a'; 50_000]);
    c.push(vec![b'('; 5_000]);
    push(&mut c, &format!("1{}", "+1".repeat(3_000)));
    push(&mut c, &format!("Print {}", "(".repeat(2_000)));
    push(&mut c, &"[".repeat(4_000));

    // Every single byte on its own (0x00 .. 0xff) — cheap, and a classic crasher class
    for b in 0u16..=255 {
        c.push(vec![b as u8]);
    }

    // Short wire-frame-shaped garbage (length-prefix-looking headers)
    c.push(vec![0xff, 0xff, 0xff, 0xff]);
    c.push(vec![0x00]);
    c.push(vec![0x80]);
    c.push(vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for n in [2usize, 3, 5, 7, 15, 31, 63, 127, 255] {
        c.push(vec![0xaa; n]);
        c.push(vec![0x00; n]);
    }

    // ── Permanent regression seeds: inputs a fuzzer once crashed on (Invariant VII) ──
    // lexer.rs try_parse_superlative usize underflow: `ߑest` + tab + control bytes. The
    // 2-byte char `ߑ` (U+07D1) passed a byte-length `>= 2` guard but is only ONE char, so
    // `chars[chars.len() - 2]` underflowed. Found by cargo-fuzz fuzz_lex, fixed by guarding
    // on the char count. This exact input must never crash the lexer again.
    c.push(vec![0xdf, 0x91, 0x65, 0x73, 0x74, 0x09, 0x11, 0x11]);
    // lexer.rs try_parse_comparative — the SAME byte-vs-char underflow in the `-er`
    // comparative sibling of try_parse_superlative. Found by cargo-fuzz fuzz_lex a second
    // time; fixed the same way (guard on char count).
    c.push(vec![
        0xe4, 0xde, 0x8b, 0x8d, 0x9e, 0x91, 0x8c, 0x99, 0x65, 0x72, 0x21, 0x21, 0x45,
    ]);
    // marshal.rs dict decode DECOMPRESSION BOMB: a 6-byte frame with iw==0 requested billions
    // of copies of one string (expands to 4.26 GB) — found by cargo-fuzz fuzz_wire_decode,
    // fixed by an output-size budget (MAX_DECODE_BYTES). This frame must decode to None, not OOM.
    c.push(vec![0x06, 0xff, 0xfd, 0xff, 0xfd, 0x84]);

    c
}

// ───────────────────────────── deterministic randomness ─────────────────────────────

/// SplitMix64 — the same deterministic generator the existing fuzz harnesses use.
struct SplitMix64(u64);
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A rich charset that actually stresses the lexer: delimiters, operators, string
/// machinery, digits, ASCII words, AND multibyte code points (so random truncation
/// lands on char boundaries the byte-slicing must respect).
const SPICY: &[&str] = &[
    "(", ")", "\"", "\\", ".", ",", " ", "\n", "\t", "Let", "be", "Print", "the", "if", "0", "9",
    "x", "café", "日", "🎉", "\u{0301}", "${", "\\(", "[", "]", "-", "+", "*", "=",
];

/// Print the discovered panics to stderr (which bypasses our quiet panic hook,
/// unlike the assert! message, which the hook would otherwise swallow).
fn eprint_failures(failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    eprintln!(
        "\n=== {} NEVER-PANIC case(s) — first 30 ===",
        failures.len()
    );
    for f in failures.iter().take(30) {
        eprintln!("{f}");
    }
}

// ───────────────────────────── the locks ─────────────────────────────

/// The catalog is exhaustive (compile-time via `dispatch`'s no-wildcard match) and
/// the runtime sweep list is complete. Bump the count deliberately when you add a
/// surface — a silent change should never slip through.
#[test]
fn surface_catalog_is_exhaustive() {
    let all = all_surface_variants();
    assert_eq!(
        all.len(),
        9,
        "Surface catalog size changed. Add the new front door to `dispatch` (no-wildcard match), \
         `all_surface_variants`, and this count — never a wildcard."
    );
    // Every surface must at minimum survive the empty input without panicking.
    for s in all {
        assert!(
            !matches!(dispatch(s, b""), Verdict::Panicked(_)),
            "{s:?} panicked on EMPTY input — the most basic robustness failure"
        );
    }
}

/// No curated adversarial input may panic ANY surface.
#[test]
fn no_curated_input_panics_any_surface() {
    let corpus = curated_corpus();
    let mut failures = Vec::new();
    for surface in all_surface_variants() {
        for (idx, input) in corpus.iter().enumerate() {
            if let Verdict::Panicked(site) = dispatch(surface, input) {
                let preview: String = String::from_utf8_lossy(input).chars().take(48).collect();
                failures.push(format!(
                    "  {surface:?} PANICKED on curated[{idx}] ({} bytes): {site}\n      input≈{preview:?}",
                    input.len()
                ));
            }
        }
    }
    eprint_failures(&failures);
    assert!(
        failures.is_empty(),
        "NEVER-PANIC VIOLATED — {} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// No random byte string may panic the byte-level front doors. The lexer and wire
/// decoders are cheap, so they take the HIGH-VOLUME raw-byte fuzz — that is where
/// raw-byte panics (byte-slicing, decoder underflow) actually live. The parser
/// (`Fol`) is heavier per call, so it gets a smaller in-suite sample; heavy parser
/// volume is the nightly cargo-fuzz job (Wave 5). The curated corpus still exercises
/// every surface, including codegen.
#[test]
fn no_random_bytes_panic_the_frontend() {
    let mut rng = SplitMix64::new(0x5EED_0000_1234_ABCD);
    let mut failures = Vec::new();
    let cheap = [
        Surface::Lex,
        Surface::WireMessage,
        Surface::WireValue,
        Surface::WireHandshake,
    ];
    for _ in 0..6_000 {
        let len = (rng.next() % 300) as usize;
        let input: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        for &surface in &cheap {
            if let Verdict::Panicked(site) = dispatch(surface, &input) {
                failures.push(format!(
                    "  {surface:?} PANICKED on random {input:?}: {site}"
                ));
            }
        }
    }
    // A TOKEN raw-byte sample through the (far heavier) logic parser: random word
    // salad triggers ~quadratic NL-parser backtracking, so even a ~40-byte random
    // string costs tens of ms. Parser panics are already covered in-suite by the
    // curated corpus and the (fast, short) spicy-source sweep; heavy raw-byte parser
    // volume is the nightly cargo-fuzz job (Wave 5). We keep just enough here to
    // catch an obvious raw-byte parser regression without dominating the runtime.
    for _ in 0..40 {
        let len = (rng.next() % 48) as usize;
        let input: Vec<u8> = (0..len).map(|_| (rng.next() & 0xff) as u8).collect();
        if let Verdict::Panicked(site) = dispatch(Surface::Fol, &input) {
            failures.push(format!("  Fol PANICKED on random {input:?}: {site}"));
        }
    }
    eprint_failures(&failures);
    assert!(
        failures.is_empty(),
        "NEVER-PANIC VIOLATED on random byte sweep — {} case(s):\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// "Spicy source" sweep — random concatenations of a lexer-stressing charset,
/// sometimes truncated mid-token, fed to the source-consuming front doors. This is
/// the sweep most likely to hit multibyte byte-slicing bugs.
#[test]
fn no_spicy_source_panics_the_frontend() {
    let mut rng = SplitMix64::new(0xABCD_1234_0000_5EED);
    // The lexer plus BOTH parser descents (logic `Fol` + imperative `CompileRust`),
    // which is where multibyte byte-slicing bugs surface. The WASM emitter is heavier
    // and unreachable by non-parsing garbage, so it stays in the curated corpus and
    // the nightly cargo-fuzz volume (Wave 5).
    let source_surfaces = [Surface::Lex, Surface::Fol, Surface::CompileRust];
    let mut failures = Vec::new();
    for _ in 0..2_000 {
        let pieces = (rng.next() % 40) as usize;
        let mut s = String::new();
        for _ in 0..pieces {
            s.push_str(SPICY[(rng.next() as usize) % SPICY.len()]);
        }
        // Truncate to a random byte length — deliberately lands off char boundaries.
        if !s.is_empty() {
            let cut = (rng.next() as usize) % s.len().max(1);
            let bytes = &s.as_bytes()[..cut];
            let input = bytes.to_vec();
            for &surface in &source_surfaces {
                if let Verdict::Panicked(site) = dispatch(surface, &input) {
                    let preview: String =
                        String::from_utf8_lossy(&input).chars().take(48).collect();
                    failures.push(format!(
                        "  {surface:?} PANICKED: {site}\n      input≈{preview:?}"
                    ));
                }
            }
        }
    }
    eprint_failures(&failures);
    assert!(
        failures.is_empty(),
        "NEVER-PANIC VIOLATED on spicy-source sweep — {} case(s):\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
