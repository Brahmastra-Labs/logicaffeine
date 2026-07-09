//! End-to-end (tree-walker tier) lock for the Word32 wrapping foundation (F1): a Logos
//! program built from the `word32` / `rotl` builtins plus the wrapping operators reproduces
//! the ChaCha20 quarter-round bit-for-bit against the RFC 8439 §2.2.1 test vector.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The reference quarter-round over native `u32` wrapping ops — the oracle the Logos run is
/// measured against. Validated below against the RFC's own published output.
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

#[test]
fn word32_wraps_and_rotates_in_logos() {
    // MAX + 1 wraps to 0 — the ring ℤ/2³², never a BigInt promotion.
    let r = tw_outcome("## Main\nLet a be word32(4294967295).\nLet b be word32(1).\nShow a + b.\n");
    assert_eq!(r.error, None, "no error: {:?}", r.error);
    assert_eq!(r.output.trim(), "0", "0xFFFFFFFF + 1 wraps to 0");

    // rotl(0x12345678, 8) == 0x34567812.
    let r = tw_outcome(&format!("## Main\nLet x be word32({}).\nShow rotl(x, 8).\n", 0x1234_5678u32));
    assert_eq!(r.error, None, "no error: {:?}", r.error);
    assert_eq!(r.output.trim(), format!("{}", 0x3456_7812u32), "rotl by 8");
}

#[test]
fn chacha_quarter_round_matches_rfc8439() {
    let (a0, b0, c0, d0) = (0x1111_1111u32, 0x0102_0304u32, 0x9b8d_6f43u32, 0x0123_4567u32);

    // The oracle, validated against the RFC's published output (hex literals — no hand math).
    let (ea, eb, ec, ed) = qr(a0, b0, c0, d0);
    assert_eq!(
        (ea, eb, ec, ed),
        (0xea2a_92f4u32, 0xcb1c_f8ceu32, 0x4581_472eu32, 0x5881_c4bbu32),
        "the oracle must match RFC 8439 §2.2.1"
    );

    let program = format!(
        "## Main\n\
         Let a be word32({a0}).\n\
         Let b be word32({b0}).\n\
         Let c be word32({c0}).\n\
         Let d be word32({d0}).\n\
         Set a to a + b.\n\
         Set d to rotl(d xor a, 16).\n\
         Set c to c + d.\n\
         Set b to rotl(b xor c, 12).\n\
         Set a to a + b.\n\
         Set d to rotl(d xor a, 8).\n\
         Set c to c + d.\n\
         Set b to rotl(b xor c, 7).\n\
         Show a.\n\
         Show b.\n\
         Show c.\n\
         Show d.\n"
    );

    let r = tw_outcome(&program);
    assert_eq!(r.error, None, "quarter-round runs without error: {:?}", r.error);
    let expected = format!("{ea}\n{eb}\n{ec}\n{ed}");
    assert_eq!(r.output.trim(), expected, "the Logos quarter-round must equal the RFC vector");
}

#[test]
fn word_path_is_byte_identical_on_tree_walker_and_vm() {
    // tw == vm parity for the Word path: the VM (Value::add → arith::add for `+`, builtins via
    // call_builtin for word32/rotl) must be byte-identical to the tree-walker.
    let (a0, b0, c0, d0) = (0x1111_1111u32, 0x0102_0304u32, 0x9b8d_6f43u32, 0x0123_4567u32);
    let quarter = format!(
        "## Main\n\
         Let a be word32({a0}).\n\
         Let b be word32({b0}).\n\
         Let c be word32({c0}).\n\
         Let d be word32({d0}).\n\
         Set a to a + b.\n\
         Set d to rotl(d xor a, 16).\n\
         Set c to c + d.\n\
         Set b to rotl(b xor c, 12).\n\
         Set a to a + b.\n\
         Set d to rotl(d xor a, 8).\n\
         Set c to c + d.\n\
         Set b to rotl(b xor c, 7).\n\
         Show a.\nShow b.\nShow c.\nShow d.\n"
    );
    let progs = [
        "## Main\nLet a be word32(4294967295).\nLet b be word32(1).\nShow a + b.\n".to_string(),
        format!("## Main\nLet x be word32({}).\nShow rotl(x, 8).\n", 0x1234_5678u32),
        quarter,
    ];
    // Compare modulo trailing whitespace, exactly as the house tw/vm differential (`norm`) does.
    fn norm(s: &str) -> Vec<String> {
        s.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
    }
    for src in &progs {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(vm.error, tw.error, "tw/vm error parity for:\n{src}");
        assert_eq!(norm(&vm.output), norm(&tw.output), "tw/vm output parity for:\n{src}");
    }
}
