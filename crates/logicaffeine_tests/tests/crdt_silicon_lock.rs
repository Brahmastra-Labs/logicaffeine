//! crdt_silicon_lock — the CRDT merge coprocessor's semantics are pinned to the SHIPPED
//! CRDT. The silicon merge unit (crdt_merge.v, proven ≡ its oracle under iverilog) computes
//! a per-replica `max` join; this proves that join reproduces `logicaffeine_data::crdt::
//! GCounter::merge` exactly. So: RTL == oracle (iverilog) == `merge_lane` (this test) ==
//! `GCounter::merge` (the real convergence). Cited REAL in SILICON_CAPABILITY_MAP.md.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::crdt_rtl::merge_lane;
use logicaffeine_data::crdt::{GCounter, Merge};

/// Build a G-Counter whose per-replica counts are `vals` (replica i holds vals[i]) using
/// only the public CRDT API (one single-replica counter per lane, folded in).
fn gcounter_from(vals: &[u32]) -> GCounter {
    let mut acc = GCounter::with_replica_id(0);
    for (i, &v) in vals.iter().enumerate() {
        let mut c = GCounter::with_replica_id(i as u64);
        c.increment(v as u64);
        acc.merge(&c);
    }
    acc
}

#[test]
fn crdt_merge_unit_matches_the_gcounter_oracle() {
    // Over many random 4-replica states, the silicon merge (per-lane `merge_lane`) yields the
    // exact value of the shipped G-Counter join-semilattice merge.
    let mut st: u64 = 0xA5A5_1234_D00D_5678;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (st >> 40) as u32
    };
    for _ in 0..500 {
        let a = [next(), next(), next(), next()];
        let b = [next(), next(), next(), next()];

        // Silicon model: per-lane max, value = sum of merged lanes.
        let silicon_value: u64 = (0..4).map(|i| merge_lane(a[i], b[i]) as u64).sum();

        // Shipped CRDT: two real G-Counters, merged with the production impl.
        let mut ga = gcounter_from(&a);
        ga.merge(&gcounter_from(&b));

        assert_eq!(
            silicon_value,
            ga.value(),
            "silicon CRDT merge value must equal GCounter::merge for a={a:?} b={b:?}"
        );

        // And the join is the lattice join the CRDT relies on: idempotent + commutative.
        for i in 0..4 {
            assert_eq!(merge_lane(a[i], a[i]), a[i], "idempotent");
            assert_eq!(
                merge_lane(a[i], b[i]),
                merge_lane(b[i], a[i]),
                "commutative"
            );
        }
    }
}
