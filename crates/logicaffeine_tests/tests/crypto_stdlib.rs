//! The crypto stdlib module (`crates/logicaffeine_compile/assets/std/crypto.lg`) ships the
//! ChaCha20 quarter-round as a DEMAND-IMPORTED LOGOS function: a program that merely calls
//! `quarterRound` pulls the module in with no explicit import. The result must equal the
//! RFC 8439 §2.2.1 test vector — proving the SHIPPED stdlib source (not a test fixture) is
//! bit-exact, and that crypto now genuinely lives in the standard library.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Reference quarter-round over native `u32` wrapping — the oracle. Validated against the
/// RFC's own published output inside the test (hex literals, no hand math).
fn qr(mut a: u32, mut b: u32, mut c: u32, mut d: u32) -> (u32, u32, u32, u32) {
    a = a.wrapping_add(b);
    d = (d ^ a).rotate_left(16);
    c = c.wrapping_add(d);
    b = (b ^ c).rotate_left(12);
    a = a.wrapping_add(b);
    d = (d ^ a).rotate_left(8);
    c = c.wrapping_add(d);
    b = (b ^ c).rotate_left(7);
    (a, b, c, d)
}

const A0: u32 = 0x1111_1111;
const B0: u32 = 0x0102_0304;
const C0: u32 = 0x9b8d_6f43;
const D0: u32 = 0x0123_4567;

fn program() -> String {
    format!(
        "## Main\n\
         Let mutable state be [word32({A0}), word32({B0}), word32({C0}), word32({D0})].\n\
         Set state to quarterRound(state, 0, 1, 2, 3).\n\
         Show item 1 of state.\n\
         Show item 2 of state.\n\
         Show item 3 of state.\n\
         Show item 4 of state.\n"
    )
}

#[test]
fn quarter_round_from_stdlib_matches_rfc8439() {
    let (ea, eb, ec, ed) = qr(A0, B0, C0, D0);
    assert_eq!(
        (ea, eb, ec, ed),
        (0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb),
        "the oracle must match RFC 8439 §2.2.1"
    );

    let r = tw_outcome(&program());
    assert_eq!(r.error, None, "stdlib quarterRound compiles + runs without error: {:?}", r.error);
    let expected = format!("{ea}\n{eb}\n{ec}\n{ed}");
    assert_eq!(
        r.output.trim(),
        expected,
        "the demand-imported stdlib quarter-round must equal the RFC 8439 vector"
    );
}

#[test]
fn quarter_round_stdlib_tw_vm_byte_identical() {
    // Normalize trailing whitespace exactly as the house tw/vm differential (`norm`) does — the
    // tiers agree on every VALUE; only a trailing newline differs.
    fn norm(s: &str) -> Vec<String> {
        s.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
    }
    let tw = tw_outcome(&program());
    let vm = vm_outcome(&program());
    assert_eq!(tw.error, None, "tw runs clean: {:?}", tw.error);
    assert_eq!(vm.error, None, "vm runs clean: {:?}", vm.error);
    assert_eq!(
        norm(&tw.output),
        norm(&vm.output),
        "tw == vm for the demand-imported stdlib quarter-round (Word32 survives the Seq on both tiers)"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the COMPILED stdlib crypto gate; run on demand"]
fn quarter_round_stdlib_aot_matches_treewalker() {
    // The whole way down: the demand-imported `assets/std/crypto.lg` quarter-round over
    // `Seq of Word32` (a mutating Word32-param function) compiled LOGOS → Rust → native binary,
    // its output byte-identical to the tree-walker. This is the compiled crypto that races libcrux.
    fn norm(s: &str) -> String {
        s.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
    }
    let tw = tw_outcome(&program());
    assert_eq!(tw.error, None, "tree-walker runs clean: {:?}", tw.error);

    let aot = common::run_logos_with_args(&program(), &[]);
    assert!(
        aot.success,
        "AOT compile+run of the stdlib quarter-round failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(
        norm(&aot.stdout),
        norm(&tw.output),
        "the COMPILED stdlib ChaCha20 quarter-round must equal the tree-walker (RFC 8439, through AOT)"
    );
}
