//! coproc_core_silicon_lock — the coprocessor-issuing core's RETIRED results are pinned to the
//! SHIPPED library. The core (coproc_core.v, proven ≡ its model under iverilog, synthesized on
//! the ECP5) fetches a program, issues BADD/BSUB/BMUL/CMERGE to the real coprocessor blocks,
//! and retires to a register file. This proves each retired register equals the production
//! `logicaffeine_base::BigInt` (add/sub/mul) and `logicaffeine_data::crdt::GCounter::merge` —
//! so a CPU-issued coprocessor instruction produces shipped-correct results. Cited REAL in
//! SILICON_CAPABILITY_MAP.md.
//!
//! Proof strength: ≡ the shipped `logicaffeine_base::BigInt` (add/sub/mul) and
//! `logicaffeine_data::crdt::GCounter::merge` libraries — a differential-oracle proof (the silicon
//! retires bit-exact library results), the strongest form for a library-equivalence claim (SAT
//! would only re-encode the oracle). FPGA/Sky130 receipts are separate (reports.json /
//! sky130_signoff.json).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::BigInt;
use logicaffeine_compile::codegen_sva::coproc_core::{
    run_core, COPROC_A, COPROC_B, COPROC_PROGRAM,
};
use logicaffeine_data::crdt::{GCounter, Merge};

fn u64_big(v: u64) -> BigInt {
    let two32 = BigInt::from_i64(1i64 << 32);
    two32
        .mul(&BigInt::from_i64((v >> 32) as i64))
        .add(&BigInt::from_i64((v & 0xFFFF_FFFF) as i64))
}

fn from_limbs(limbs: &[u64]) -> BigInt {
    let two32 = BigInt::from_i64(1i64 << 32);
    let base = two32.mul(&two32); // 2^64
    let mut acc = BigInt::from_i64(0);
    for &limb in limbs.iter().rev() {
        acc = acc.mul(&base).add(&u64_big(limb));
    }
    acc
}

/// The four low-128 lanes (four 32-bit replica counts) of a 256-bit register.
fn lanes(x: &[u64; 4]) -> [u32; 4] {
    [
        x[0] as u32,
        (x[0] >> 32) as u32,
        x[1] as u32,
        (x[1] >> 32) as u32,
    ]
}

/// A G-Counter with per-replica counts `vals`, built via the public CRDT API only.
fn gcounter_from(vals: &[u32; 4]) -> GCounter {
    let mut acc = GCounter::with_replica_id(0);
    for (i, &v) in vals.iter().enumerate() {
        let mut c = GCounter::with_replica_id(i as u64);
        c.increment(v as u64);
        acc.merge(&c);
    }
    acc
}

#[test]
fn coproc_core_retires_shipped_bigint_and_gcounter_results() {
    let r = run_core(&COPROC_A, &COPROC_B, COPROC_PROGRAM);
    let (a, b) = (from_limbs(&COPROC_A), from_limbs(&COPROC_B));

    // BADD → R2 == shipped BigInt::add.
    assert_eq!(
        from_limbs(&r[2]),
        a.add(&b),
        "CPU-issued BADD must retire BigInt::add"
    );

    // BSUB → R3 == shipped BigInt::sub (COPROC_A > COPROC_B ⇒ a non-negative magnitude).
    assert!(
        a > b,
        "the demo keeps A > B so the subtract has a shipped magnitude"
    );
    assert_eq!(
        from_limbs(&r[3]),
        a.sub(&b),
        "CPU-issued BSUB must retire BigInt::sub"
    );

    // BMUL → R4 == shipped BigInt::mul of the low limbs (128-bit product).
    assert_eq!(
        from_limbs(&r[4]),
        u64_big(COPROC_A[0]).mul(&u64_big(COPROC_B[0])),
        "CPU-issued BMUL must retire BigInt::mul"
    );

    // CMERGE → R5's merged lanes sum to the shipped GCounter::merge value.
    let merged_lanes = lanes(&r[5]);
    let silicon_value: u64 = merged_lanes.iter().map(|&x| x as u64).sum();
    let mut g = gcounter_from(&lanes(&COPROC_A));
    g.merge(&gcounter_from(&lanes(&COPROC_B)));
    assert_eq!(
        silicon_value,
        g.value(),
        "CPU-issued CMERGE must retire GCounter::merge"
    );
}
