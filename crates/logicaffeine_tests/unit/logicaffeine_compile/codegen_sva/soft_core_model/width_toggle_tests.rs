use super::*;

/// The WIDTH TOGGLE (VM⇄Fabric parity): `soft_core_alu_w`/`soft_core_divider_w` at width 32 reproduce the
/// 32-bit core EXACTLY (the tape-out core is unchanged), and at width 64 do FULL i64 arithmetic — closing
/// the exact overflow (`seed * 1103515245`, `% 2^31`) that makes the 32-bit chip diverge from the VM.
#[test]
fn alu_width_toggle_is_32bit_faithful_and_64bit_wide() {
    let ops: [u8; 18] = [
        8, 9, 10, 11, 16, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    ];
    let mut st = 0x1234_5678_9abc_def0u64;
    for _ in 0..50_000 {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let a = (st >> 33) as i32;
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let b = (st >> 33) as i32;
        for &op in &ops {
            assert_eq!(
                soft_core_alu(op, a, b),
                soft_core_alu_w(op, i64::from(a), i64::from(b), 32) as i32,
                "ALU op {op}: width-32 toggle != 32-bit baseline on a={a} b={b}"
            );
        }
        for &op in &[12u8, 13, 14, 15] {
            assert_eq!(
                soft_core_divider(op, a, b),
                soft_core_divider_w(op, i64::from(a), i64::from(b), 32) as i32,
                "divider op {op}: width-32 toggle != 32-bit baseline on a={a} b={b}"
            );
        }
    }

    // width 64 = full i64: the PRNG intermediate `seed * 1103515245` (which OVERFLOWS 32 bits) is computed
    // exactly, matching the VM's BigInt mod 2^64 — where the 32-bit core wraps and diverges.
    let seed = 1_103_527_590i64;
    let prod64 = soft_core_alu_w(11, seed, 1_103_515_245, 64); // Mul
    assert_eq!(
        prod64,
        seed.wrapping_mul(1_103_515_245),
        "width-64 Mul must be full i64 (== VM mod 2^64)"
    );
    assert!(
        prod64 > i64::from(i32::MAX),
        "the product genuinely exceeds 32 bits (the divergence source)"
    );
    assert_ne!(
        soft_core_alu_w(11, seed, 1_103_515_245, 32),
        prod64,
        "width-32 wraps the product where width-64 does not — the toggle is what closes the gap"
    );
    assert_eq!(
        sign_extend_to_width(prod64, 64),
        prod64,
        "width 64 is the identity"
    );
    assert_eq!(
        sign_extend_to_width(prod64, 32),
        i64::from(prod64 as i32),
        "width 32 truncates to i32"
    );
}

/// The WIDTH TOGGLE THREADED THROUGH THE RUN LOOP: a tiny chip-word program whose `Mul` product
/// exceeds `i32::MAX` runs on `run_soft_core_w` at BOTH widths. At `width == 64` the whole fetch/
/// decode/execute datapath computes the full i64 product (== the VM's `Int mod 2^64`); at
/// `width == 32` it wraps exactly as the tape-out core does — and the two DIFFER, proving the toggle
/// reaches the register file and the ALU dispatch, not just the standalone `soft_core_alu_w` primitive.
#[test]
fn run_loop_width_64_does_full_i64() {
    // Chip-word layout: `[opcode:8][dst:8][a:8][b:8]` big-endian in the u32 (opcode is the high byte).
    let w = |op: u8, dst: u8, a: u8, b: u8| {
        (u32::from(op) << 24) | (u32::from(dst) << 16) | (u32::from(a) << 8) | u32::from(b)
    };
    let a: i64 = 1_103_527_590; // < 2^31, so both widths sign-extend it as the same positive value
    let b: i64 = 1_103_515_245; // < 2^31 likewise; only their PRODUCT exceeds 32 bits
    let prog = [
        w(200, 0, 0, 0),
        a as u32, // LoadConstWide r0 = a  (2-word: op then the 32-bit constant)
        w(200, 1, 0, 0),
        b as u32,       // LoadConstWide r1 = b
        w(11, 2, 0, 1), // Mul r2 = r0 * r1  (ra=0, rb=1)
        w(89, 0, 2, 0), // Show r2          (Show reads regs[ra]; ra = byte 1 = 2)
        w(43, 0, 0, 0), // Halt
    ];

    // width 64 = FULL i64: the run loop's LoadConstWide/Mul retire the true product.
    let full = a.wrapping_mul(b);
    assert!(
        full > i64::from(i32::MAX),
        "the product must genuinely exceed 32 bits"
    );
    let got64 = run_soft_core_w(&prog, 100, 64);
    assert_eq!(
        got64,
        vec![full],
        "width-64 run loop must produce the full i64 product (== VM mod 2^64)"
    );

    // width 32 = the tape-out core: the SAME program wraps the product to 32 bits.
    let wrapped = i64::from((a as i32).wrapping_mul(b as i32));
    let got32 = run_soft_core_w(&prog, 100, 32);
    assert_eq!(
        got32,
        vec![wrapped],
        "width-32 run loop must wrap the product to the 32-bit core's value"
    );

    // The toggle is what closes the gap: the two widths DISAGREE on this overflowing program.
    assert_ne!(
        got32[0], got64[0],
        "width-32 and width-64 must differ on the overflowing Mul — the toggle bites"
    );
    // And width 32 == the historical 32-bit oracle (`run_soft_core` is the width-32 wrapper).
    assert_eq!(
        run_soft_core(&prog, 100),
        got32,
        "run_soft_core must equal the width-32 run"
    );
}
