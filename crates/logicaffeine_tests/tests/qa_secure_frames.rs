//! QA — the value-level frame edges, solved. `wireBytes` frames a value to bytes; `fromWireBytes`
//! decodes it straight back (self-describing, no schema) — the round trip a streaming receiver needs.
//! `sealFrame`/`openFrame` fuse wire+authenticated-encrypt into one call per direction, so a
//! structured value becomes a tamper-proof frame and back with no chance of mis-ordering the steps.
//! These lock: every streaming value type round-trips, malformed input is refused, the sealed frame
//! authenticates and is exactly as long as the value's wire form plus its tag, and the whole secure
//! value pipeline survives packet loss — identically on the tree-walker and the VM.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn bytes_lit(f: impl Fn(u32) -> u8, n: u32) -> String {
    (0..n)
        .map(|i| f(i).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
fn key_lit() -> String {
    bytes_lit(|i| (i.wrapping_mul(7).wrapping_add(1)) as u8, 32)
}
fn nonce_lit() -> String {
    bytes_lit(|i| (i.wrapping_mul(5).wrapping_add(3)) as u8, 12)
}

/// Run a program with exactly two `Show`s; assert the two emitted lines match, on BOTH tiers. Splits
/// WITHOUT trimming so an empty value (`Show ""` → a blank line) still compares correctly.
fn assert_two_lines_equal(program: &str, ctx: &str) {
    for (tier, r) in [("tw", tw_outcome(program)), ("vm", vm_outcome(program))] {
        assert_eq!(
            r.error, None,
            "{ctx} [{tier}]: runs clean, got {:?}\n{program}",
            r.error
        );
        let parts: Vec<&str> = r.output.split('\n').collect();
        assert!(
            parts.len() >= 2,
            "{ctx} [{tier}]: expected two Show lines, got {:?}\n{program}",
            r.output
        );
        assert_eq!(
            parts[0], parts[1],
            "{ctx} [{tier}]: values must match\n{program}"
        );
    }
}

/// EDGE — the round trip: `fromWireBytes(wireBytes(v)) == v` for every value type a stream carries.
/// Self-describing, so the receiver reconstructs the value with no type hint.
#[test]
fn wire_round_trips_every_streaming_value_type() {
    let cases = [
        ("int-negative", "Let v be -98765."),
        ("int-zero", "Let v be 0."),
        ("int-max", "Let v be 9223372036854775807."),
        ("text-unicode", "Let v be \"héllo, 世界 🌍\"."),
        ("text-empty", "Let v be \"\"."),
        ("bool-true", "Let v be true."),
        ("bool-false", "Let v be false."),
        ("seq-int", "Let v be [1, 2, 3, -4, 5]."),
        ("seq-text", "Let v be [\"alice\", \"bob\", \"carol\"]."),
        ("seq-nested", "Let v be [[1, 2], [3, 4], [5, 6]]."),
        ("seq-of-seq-text", "Let v be [[\"a\", \"b\"], [\"c\"]]."),
        ("nothing", "Let v be nothing."),
        ("seq-empty", "Let v be []."),
    ];
    for (label, binding) in cases {
        let program = format!(
            "## Main\n{binding}\n\
             Let packed be wireBytes(v).\n\
             Let back be fromWireBytes(packed).\n\
             Show v.\n\
             Show back.\n"
        );
        assert_two_lines_equal(&program, &format!("wire round-trip: {label}"));
    }
}

/// EDGE — a `Shared` struct's value round-trips through the wire codec (rosters, presence records are
/// structs). Uses a counter field so the decoded struct's field renders identically.
#[test]
fn wire_round_trips_a_struct_value() {
    let program = "## Definition\n\
        A Presence is Shared and has:\n\
        \x20   a name, which is Divergent Text.\n\
        \n\
        ## Main\n\
        Let mutable p be a new Presence.\n\
        Set p's name to \"alice\".\n\
        Let packed be wireBytes(p).\n\
        Let back be fromWireBytes(packed).\n\
        Show p's name.\n\
        Show back's name.\n";
    assert_two_lines_equal(program, "wire round-trip: struct");
}

/// EDGE — malformed / trailing-byte / empty wire input is REFUSED, never decoded to a bogus value.
#[test]
fn from_wire_bytes_rejects_malformed_input() {
    let cases = [
        (
            "empty",
            "## Main\nLet out be fromWireBytes([]).\nShow out.\n".to_string(),
        ),
        (
            "trailing-bytes",
            "## Main\n\
             Let good be wireBytes(5).\n\
             Let bad be good followed by [99, 99].\n\
             Let out be fromWireBytes(bad).\n\
             Show out.\n"
                .to_string(),
        ),
    ];
    for (label, program) in cases {
        for (tier, r) in [("tw", tw_outcome(&program)), ("vm", vm_outcome(&program))] {
            assert!(
                r.error.is_some(),
                "[{tier}] fromWireBytes {label}: must refuse malformed input, got {:?}\n{program}",
                r.output
            );
        }
    }
}

/// THE PERFECT SURFACE: `sealFrame`/`openFrame` round-trip a structured value through the full
/// compact-secure-frame in one call each direction.
#[test]
fn seal_frame_then_open_frame_round_trips_a_value() {
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let roster be [\"alice\", \"bob\", \"carol\", \"dave\"].\n\
         Let sealed be sealFrame(key, nonce, roster).\n\
         Let back be openFrame(key, nonce, sealed).\n\
         Show roster.\n\
         Show back.\n",
        key_lit(),
        nonce_lit()
    );
    assert_two_lines_equal(&program, "sealFrame/openFrame round-trip");
}

/// AUTHENTICATION: `openFrame` refuses a frame sealed under a different key — a value frame is
/// tamper-proof just like the byte frame.
#[test]
fn open_frame_rejects_a_wrong_key() {
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let badKey be [{}].\n\
         Let sealed be sealFrame(key, nonce, [1, 2, 3]).\n\
         Let back be openFrame(badKey, nonce, sealed).\n\
         Show back.\n",
        key_lit(),
        nonce_lit(),
        bytes_lit(|i| (i.wrapping_mul(3)) as u8, 32)
    );
    for (tier, r) in [("tw", tw_outcome(&program)), ("vm", vm_outcome(&program))] {
        assert!(
            r.error.is_some(),
            "[{tier}]: a wrong key must fail authentication → {:?}",
            r.output
        );
    }
}

/// A sealed frame's length says nothing about its value but the length of its wire form: a value is
/// never compressed before it is encrypted, since a ciphertext as long as the compressed plaintext tells
/// how compressible the plaintext was, and a frame that carries a secret beside anything an attacker
/// chooses gives the secret away a guess at a time (CRIME, BREACH). A repetitive value and a varied one
/// seal to their wire length plus the 16-byte tag, each.
#[test]
fn a_sealed_frame_is_its_values_wire_form_and_a_tag() {
    let repetitive = (0..400)
        .map(|_| "7".to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let varied = (0..400)
        .map(|i| ((i * 2654435761u64) % 1000).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let redundant be [{repetitive}].\n\
         Let varied be [{varied}].\n\
         Show (length of sealFrame(key, nonce, redundant)) - (length of wireBytes(redundant)).\n\
         Show (length of sealFrame(key, nonce, varied)) - (length of wireBytes(varied)).\n",
        key_lit(),
        nonce_lit()
    );
    for (tier, r) in [("tw", tw_outcome(&program)), ("vm", vm_outcome(&program))] {
        assert_eq!(r.error, None, "[{tier}] runs clean: {:?}", r.error);
        assert_eq!(
            r.output.trim(),
            "16\n16",
            "[{tier}] each sealed frame is its value's wire form plus the tag\n{program}"
        );
    }
}

/// THE WHOLE STORY, end to end: a structured value → `sealFrame` → FEC → lose a packet on the potato
/// link → restore → `openFrame` → the exact value back. One call each side, secure and loss-resilient.
#[test]
fn full_secure_value_pipeline_survives_loss() {
    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let state be [100, 200, 300, 400, 500, 600, 700, 800].\n\
         Let sealed be sealFrame(key, nonce, state).\n\
         Let packets be fecProtect(sealed, 3, 2).\n\
         Let p1 be item 1 of packets.\n\
         Let p2 be item 2 of packets.\n\
         Let p3 be item 3 of packets.\n\
         Let p5 be item 5 of packets.\n\
         Let received be [p1, p2, p3, nothing, p5].\n\
         Let recovered be fecRestore(received, 3).\n\
         Let back be openFrame(key, nonce, recovered).\n\
         Show state.\n\
         Show back.\n",
        key_lit(),
        nonce_lit()
    );
    assert_two_lines_equal(&program, "full secure value pipeline with loss");
}
