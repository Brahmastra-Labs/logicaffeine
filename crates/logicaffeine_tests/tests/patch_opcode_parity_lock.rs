//! ★ LOCK — the browser decodes every opcode the compiler can encode. ★
//!
//! The patch stream is a wire format with an encoder and a Rust decoder in `semantics/patch.rs`
//! and a SECOND decoder, in another language, in `assets/web/runtime.js`. Nothing held them
//! together, and they had already drifted: `OP_PREVENT_DEFAULT = 12` is written by the encoder and
//! read by the Rust decoder, and `runtime.js` decodes `case 0` … `case 11` and then
//! `default: throw`.
//!
//! ★ THE SYMPTOM WOULD BE A BLANK PAGE, not a missing `preventDefault`. That decoder is
//! deliberately all-or-nothing — *"Decode the WHOLE buffer first … so a malformed stream never
//! half-paints"* — so one unknown opcode discards the entire paint.
//!
//! ★ IT IS LATENT, AND THAT IS WHY IT NEEDS A LOCK RATHER THAN A FIX ALONE. LGX pages call the
//! per-op `dom_prevent_default` import today, so opcode 12 never reaches the stream. It arms the
//! moment the emitter moves to the bulk `dom_apply` path, which the architecture calls the
//! direction of travel — a landmine in front of a planned change.
//!
//! ★ AND THE REASON NOTHING CAUGHT IT IS THE REUSABLE PART. `patch_stream_lock::every_op()` — doc
//! comment *"every op kind, so the round-trip covers the whole surface rather than the easy half"*
//! — omitted `PreventDefault`. Its negative test corrupts a byte to `0xEE`, so it can never notice
//! that 12 is unknown either. **A fixture that names itself "every" and is not, is the same silent
//! specification as a fuzzer's alphabet that is ASCII and does not say so.** Both were found the
//! same day, in the same front end, by looking at what was never enumerated rather than at what was
//! decided wrongly.
//!
//! So this file does not test a round trip. It holds the two decoders to the encoder, by reading
//! all three, so opcode 13 cannot repeat this.

/// Every `const OP_… : u8 = N;` the encoder can write, from the source of truth.
fn encoder_opcodes() -> Vec<(String, u8)> {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/src/semantics/patch.rs"),
    )
    .expect("the patch codec");
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("const OP_") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(": u8 = ") else {
            continue;
        };
        let Some(num) = tail.trim_end_matches(';').trim().parse::<u8>().ok() else {
            continue;
        };
        out.push((format!("OP_{name}"), num));
    }
    out.sort_by_key(|(_, n)| *n);
    out
}

/// Whether no emitter writes `code` any more (`patch::RETIRED_OPCODES`): it keeps its number and
/// its `case`, and nothing round-trips it.
fn retired(code: u8) -> bool {
    logicaffeine_compile::semantics::patch::RETIRED_OPCODES
        .iter()
        .any(|(c, _)| *c == code)
}

/// `runtime.js`'s patch decode switch: the one that ends in `unknown patch opcode` — the file has
/// other switches (the apply loop keys off op NAMES, not numbers).
fn browser_decoder_switch() -> String {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/assets/web/runtime.js"),
    )
    .expect("the browser runtime");
    let end = src
        .find("unknown patch opcode")
        .expect("the patch decoder's default arm");
    let start = src[..end]
        .rfind("switch (code)")
        .expect("the patch decoder's switch");
    src[start..end].to_string()
}

/// The opcodes `runtime.js`'s decoder has a `case` for.
fn browser_decoder_cases() -> Vec<u8> {
    let mut out = Vec::new();
    for line in browser_decoder_switch().lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("case ") else {
            continue;
        };
        if let Some(n) = rest
            .split(':')
            .next()
            .and_then(|s| s.trim().parse::<u8>().ok())
        {
            out.push(n);
        }
    }
    out.sort_unstable();
    out
}

#[test]
fn the_browser_decodes_every_opcode_the_compiler_emits() {
    let encoder = encoder_opcodes();
    let browser = browser_decoder_cases();
    assert!(
        encoder.len() >= 13,
        "the opcode table shrank to {}",
        encoder.len()
    );

    let missing: Vec<&(String, u8)> = encoder
        .iter()
        .filter(|(_, n)| !browser.contains(n))
        .collect();
    assert!(
        missing.is_empty(),
        "the compiler can emit opcodes the browser throws on: {missing:?}\n\
         `runtime.js`'s decoder is all-or-nothing, so ONE unknown opcode discards the whole paint \
         and the page goes blank — it does not lose that one operation. Add a `case` for each, and \
         add the op to `patch_stream_lock::every_op()` so the round trip covers it too."
    );
}

#[test]
fn the_browser_invents_no_opcode_the_compiler_cannot_emit() {
    // The other direction. A `case 13` for something the encoder never writes is dead code that
    // reads as coverage — and would make the next real opcode 13 silently mean something else.
    let encoder: Vec<u8> = encoder_opcodes().into_iter().map(|(_, n)| n).collect();
    let extra: Vec<u8> = browser_decoder_cases()
        .into_iter()
        .filter(|n| !encoder.contains(n))
        .collect();
    assert!(
        extra.is_empty(),
        "the browser decodes opcodes nothing emits: {extra:?}"
    );
}

#[test]
fn the_opcode_numbers_are_dense_and_unique() {
    // A gap or a duplicate in the table is how two ops come to share a byte. Cheap to check, and
    // it makes the two lists above comparable by number at all.
    let ops = encoder_opcodes();
    let nums: Vec<u8> = ops.iter().map(|(_, n)| *n).collect();
    for (i, n) in nums.iter().enumerate() {
        assert_eq!(
            *n as usize, i,
            "opcode numbering is not dense at {}: {ops:?}",
            ops[i].0
        );
    }
}

#[test]
fn the_round_trip_fixture_covers_every_opcode() {
    // ★ THE FIXTURE THAT NAMED ITSELF "EVERY". `patch_stream_lock::every_op()` is the round-trip
    // corpus, and it omitted `PreventDefault` — so the op was encoded, decoded and never once
    // exercised. Held to the opcode table by NUMBER, since that is what the wire carries.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/patch_stream_lock.rs"),
    )
    .expect("the round-trip lock");
    let body = src
        .split("fn every_op()")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("every_op is declared there");
    for (name, code) in encoder_opcodes() {
        if retired(code) {
            continue;
        }
        // `OP_CREATE_TEXT_NODE` → `CreateTextNode`
        let variant: String = name
            .trim_start_matches("OP_")
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_string() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            })
            .collect();
        assert!(
            body.contains(&format!("DomOp::{variant}")),
            "`every_op()` omits `DomOp::{variant}` — a corpus that calls itself every and is not \
             is the same silent specification as a fuzzer alphabet that is ASCII and does not say so"
        );
    }
}

/// The source of the `env` entry `name` in `runtime.js`: from its key up to the next key of the
/// `env` object (a two-space-indented `key: (` line).
fn env_entry<'a>(src: &'a str, name: &str) -> &'a str {
    let key = format!("\n  {name}: (");
    let start = src
        .find(&key)
        .unwrap_or_else(|| panic!("runtime.js implements `{name}`"))
        + 1;
    let rest = &src[start..];
    let is_key = |line: &str| {
        line.starts_with("  ")
            && !line.starts_with("   ")
            && line.trim_start().split_once(": (").is_some_and(|(k, _)| {
                !k.is_empty()
                    && k.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
    };
    let mut offset = 0;
    for (i, line) in rest.lines().enumerate() {
        if i > 0 && is_key(line) {
            return &rest[..offset];
        }
        offset += line.len() + 1;
    }
    rest
}

#[test]
fn both_wire_formats_decode_through_the_one_opcode_switch() {
    // ★ ONE DECODER. The byte format (`dom_apply`) and wire v2 (`dom_apply_words`, what `vdom.lg`
    // hands over) carry the same opcodes with different operand encodings; a second `switch` over
    // the opcode table would be a second place for an opcode to go missing. Both entries read their
    // operands through a format reader and decode through `decodePatches`, the switch the census
    // above holds to the encoder.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/assets/web/runtime.js"),
    )
    .expect("the browser runtime");
    assert_eq!(
        src.matches("unknown patch opcode").count(),
        1,
        "runtime.js decodes patch opcodes in exactly one switch (the one whose default names an unknown opcode)"
    );
    let decoder = src
        .find("const decodePatches = (read) =>")
        .expect("the one decoder");
    let default = src.find("unknown patch opcode").unwrap();
    let switch = src[..default]
        .rfind("switch (code)")
        .expect("the decoder's switch");
    assert!(
        switch > decoder && src[decoder..switch].lines().count() < 8,
        "the one opcode switch is `decodePatches`'s"
    );
    for entry in ["dom_apply", "dom_apply_words"] {
        let body = env_entry(&src, entry);
        assert!(
            body.contains("decodePatches("),
            "`{entry}` decodes through the one decoder:\n{body}"
        );
    }
}

#[test]
fn the_browser_refuses_a_retired_opcode_in_the_rust_decoders_words() {
    // A retired opcode keeps its `case` so a stream an old build emitted is refused by name, as the
    // Rust decoders refuse it — never read as whatever opcode is added next, never an unknown code.
    let switch = browser_decoder_switch();
    for &(code, why) in logicaffeine_compile::semantics::patch::RETIRED_OPCODES {
        let name = logicaffeine_compile::semantics::patch::OPCODES[code as usize];
        let arm = switch
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(&format!("case {code}:")))
            .unwrap_or_else(|| panic!("runtime.js has no `case {code}` for the retired {name}"));
        assert!(
            arm.contains("throw")
                && arm.contains(&format!("patch opcode {code} ({name})"))
                && arm.contains(why),
            "runtime.js's `case {code}` must refuse {name} in the Rust decoders' words: {arm}"
        );
    }
}
