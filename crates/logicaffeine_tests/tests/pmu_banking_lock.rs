//! ═══════════════════════════════════════════════════════════════════════
//! PMU BANKING LOCK — P2 of the Plasticine lane: the Pattern Memory Unit model (ISCA'17 §3.2) is what the paper
//! describes AND has an executable meaning.
//!
//! A. ISCA'17 PMU: 16 banks × 4096 words = 256 KB; N-buffering splits it into disjoint, exhaustive regions;
//!    a tile that does not fit one buffer is refused, typed.
//! B. Banking arithmetic: at unit granularity a stride-s 16-lane access conflicts gcd(s,16) ways; the strided
//!    configuration for s is conflict-free for EVERY s; FIFO is conflict-free and order-preserving; duplication
//!    serves any gather in one cycle at broadcast-write cost; a line buffer spreads a row's lanes.
//! C. BIJECTION: banked storage under every mode reads back exactly what a flat heap would — the banking never
//!    changes a value, only its cost.
//! D. The address datapath: an affine address over d counters (+1 vectorized) needs d(+1) FU stages; the
//!    4-stage PMU refuses deeper nests, typed.
//! E. The memory II feeds the stage II (`max(resource, recurrence, memory, 1)`).
//!
//! Honest boundary: MODELED from the published parameters; no Plasticine simulation.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(pmu_banking_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::metapipeline::Stage;
use logicaffeine_compile::codegen_sva::pmu::{
    address_datapath_stages, address_fits, bank_of, conflicts, memory_ii, strided_banking_for,
    AffineAccess, BankingMode, Pmu, PmuSpec,
};

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn prng(seed: u64, i: usize) -> u64 {
    let mut x = seed
        .wrapping_add(0x9E3779B97F4A7C15)
        .wrapping_mul(i as u64 + 1)
        ^ 0xD1B54A32D192ED03;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x
}

#[test]
fn isca17_pmu_capacity_and_n_buffer_regions_are_disjoint_and_exhaustive() {
    let p = PmuSpec::isca17();
    assert_eq!((p.banks, p.words_per_bank, p.stages), (16, 4096, 4));
    assert_eq!(
        p.capacity_words(),
        65_536,
        "16 banks x 16 KB of 32-bit words = 256 KB"
    );
    assert_eq!(p.capacity_words() * 4, 256 * 1024);
    for n in 1..=4 {
        let spec = PmuSpec { n_buffers: n, ..p };
        let regions = spec.n_buffer_regions();
        assert_eq!(regions.len(), n);
        assert_eq!(regions[0].0, 0);
        assert_eq!(
            regions[n - 1].1,
            spec.capacity_words(),
            "regions are exhaustive"
        );
        for w in regions.windows(2) {
            assert_eq!(w[0].1, w[1].0, "regions are contiguous and disjoint");
        }
        assert_eq!(spec.buffer_words(), 65_536 / n);
        spec.tile_fits(65_536 / n)
            .expect("a buffer-sized tile fits");
        assert!(
            spec.tile_fits(65_536 / n + 1).is_err(),
            "one word more is refused"
        );
    }
}

#[test]
fn unit_granularity_conflicts_are_gcd_of_stride_and_banks_and_strided_banking_removes_them() {
    let p = PmuSpec::isca17();
    for stride in 1..=64usize {
        let a = AffineAccess {
            base: 0,
            stride,
            lanes: 16,
        };
        let unit = conflicts(&a, BankingMode::Strided { granularity: 1 }, &p);
        assert_eq!(
            unit,
            gcd(stride, 16),
            "stride {stride} at unit granularity conflicts gcd(s,16) ways"
        );
        let tuned = conflicts(&a, strided_banking_for(stride), &p);
        assert_eq!(
            tuned, 1,
            "strided banking at granularity {stride} is conflict-free"
        );
        for base in [0usize, 7, 1000, 4095] {
            let ab = AffineAccess {
                base,
                stride,
                lanes: 16,
            };
            assert_eq!(
                conflicts(&ab, strided_banking_for(stride), &p),
                1,
                "conflict-free for any base"
            );
        }
    }
    // More lanes than banks must conflict at least ceil(lanes/banks) ways under any mode but duplication.
    let wide = AffineAccess {
        base: 0,
        stride: 1,
        lanes: 32,
    };
    assert_eq!(
        conflicts(&wide, BankingMode::Strided { granularity: 1 }, &p),
        2
    );
    assert_eq!(conflicts(&wide, BankingMode::Duplication, &p), 1);
    assert_eq!(
        memory_ii(
            &[
                AffineAccess {
                    base: 0,
                    stride: 2,
                    lanes: 16
                },
                AffineAccess {
                    base: 0,
                    stride: 3,
                    lanes: 16
                }
            ],
            BankingMode::Strided { granularity: 1 },
            &p
        ),
        2
    );
}

#[test]
fn fifo_is_conflict_free_and_order_preserving_and_a_line_buffer_spreads_a_row() {
    let p = PmuSpec::isca17();
    let mut f = Pmu::new(p, BankingMode::Fifo);
    let vals: Vec<i64> = (0..1000).map(|i| prng(1, i) as i64 >> 2).collect();
    for &v in &vals {
        assert!(f.push(v));
    }
    let popped: Vec<i64> = (0..1000).filter_map(|_| f.pop()).collect();
    assert_eq!(popped, vals, "pop order == push order");
    assert_eq!(f.pop(), None);
    assert_eq!(
        conflicts(
            &AffineAccess {
                base: 0,
                stride: 1,
                lanes: 16
            },
            BankingMode::Fifo,
            &p
        ),
        1,
        "a 16-wide push lands in 16 banks"
    );
    let mut full = Pmu::new(
        PmuSpec {
            words_per_bank: 2,
            ..p
        },
        BankingMode::Fifo,
    );
    for i in 0..32 {
        assert!(full.push(i));
    }
    assert!(!full.push(99), "a full FIFO refuses a push");
    // Line buffer: a 640-wide row's first 16 lanes spread over 16 banks (conflict-free), as do lanes 16..32.
    let lb = BankingMode::LineBuffer { width: 640 };
    assert_eq!(
        conflicts(
            &AffineAccess {
                base: 640 * 3,
                stride: 1,
                lanes: 16
            },
            lb,
            &p
        ),
        1
    );
    assert_eq!(bank_of(640 * 3 + 5, lb, 16), 5);
}

#[test]
fn duplication_serves_any_gather_in_one_cycle_at_broadcast_write_cost() {
    let p = PmuSpec {
        words_per_bank: 4096,
        ..PmuSpec::isca17()
    };
    let mut d = Pmu::new(p, BankingMode::Duplication);
    let gather = AffineAccess {
        base: 3,
        stride: 16,
        lanes: 16,
    }; // stride 16 = every lane in the SAME bank at unit granularity
    assert_eq!(
        conflicts(&gather, BankingMode::Strided { granularity: 1 }, &p),
        16,
        "the worst gather: 16 lanes, one bank"
    );
    let cost = d.vector_write(&gather, &(0..16).map(|i| i * 11).collect::<Vec<i64>>());
    assert_eq!(
        cost, 16,
        "a duplicated write is broadcast: one round per lane"
    );
    let (vals, cycles) = d.vector_read(&gather);
    assert_eq!(cycles, 1, "any gather reads in one cycle under duplication");
    assert_eq!(vals, (0..16).map(|i| i * 11).collect::<Vec<i64>>());
}

#[test]
fn banked_storage_is_a_bijection_of_the_flat_heap_under_every_mode() {
    let p = PmuSpec {
        words_per_bank: 256,
        ..PmuSpec::isca17()
    }; // 4096 words, fast
    for mode in [
        BankingMode::Strided { granularity: 1 },
        BankingMode::Strided { granularity: 3 },
        BankingMode::Strided { granularity: 20 },
        BankingMode::Fifo,
        BankingMode::LineBuffer { width: 40 },
        BankingMode::LineBuffer { width: 640 },
        BankingMode::Duplication,
    ] {
        let cap = p.usable_words(mode);
        match mode {
            BankingMode::LineBuffer { width: 40 } => assert_eq!(
                cap,
                (256 / 3) * 40,
                "a 40-wide line buffer: 3 columns per bank per row, 85 whole rows"
            ),
            BankingMode::LineBuffer { width: 640 } => assert_eq!(
                cap,
                (256 / 40) * 640,
                "a 640-wide line buffer: 40 columns per bank per row, 6 whole rows"
            ),
            BankingMode::Strided { granularity: 3 } => assert_eq!(
                cap,
                (256 / 3) * 3 * 16,
                "granularity 3: 85 whole 3-word chunks per bank"
            ),
            BankingMode::Strided { granularity: 20 } => assert_eq!(
                cap,
                (256 / 20) * 20 * 16,
                "granularity 20: 12 whole 20-word chunks per bank"
            ),
            BankingMode::Duplication => assert_eq!(
                cap, 256,
                "duplication replicates one bank's worth into every bank"
            ),
            _ => assert_eq!(cap, p.capacity_words(), "{mode:?} addresses the whole SRAM"),
        }
        let mut pmu = Pmu::new(p, mode);
        let mut flat = vec![0i64; cap];
        // Slot injectivity for the non-duplicating modes: no two addresses share (bank, offset).
        if mode != BankingMode::Duplication {
            let mut seen = std::collections::HashSet::new();
            for a in 0..cap {
                let (b, off) = pmu.slot(a);
                assert!(
                    b < p.banks && off < p.words_per_bank,
                    "{mode:?}: slot of {a} out of range ({b},{off})"
                );
                assert!(
                    seen.insert((b, off)),
                    "{mode:?}: addresses collide at slot ({b},{off}) — the banking would lose data"
                );
            }
        }
        for i in 0..20_000usize {
            let r = prng(7, i);
            let a = (r as usize) % cap;
            if r & 1 == 0 {
                let v = (r >> 8) as i64;
                pmu.write(a, v);
                flat[a] = v;
            } else {
                assert_eq!(
                    pmu.read(a),
                    flat[a],
                    "{mode:?}: read at {a} differs from the flat heap"
                );
            }
        }
        for a in 0..cap {
            assert_eq!(
                pmu.read(a),
                flat[a],
                "{mode:?}: final contents differ at {a}"
            );
        }
    }
}

#[test]
fn the_address_datapath_fits_the_pmu_stages_or_refuses_typed() {
    let p = PmuSpec::isca17();
    assert_eq!(address_datapath_stages(1, false), 1);
    assert_eq!(
        address_datapath_stages(2, true),
        3,
        "base + s1*i + s2*j + lane: three chained multiply-adds"
    );
    assert_eq!(
        address_fits(&p, 3, true).unwrap(),
        4,
        "a 3-D vectorized address exactly fills the 4 stages"
    );
    let e = address_fits(&p, 4, true).unwrap_err();
    assert!(
        e.contains("5 FU stages") && e.contains("4"),
        "a 4-D vectorized address is refused, naming the depths: {e}"
    );
    assert_eq!(address_fits(&p, 4, false).unwrap(), 4);
}

#[test]
fn the_memory_ii_feeds_the_stage_ii() {
    let base = Stage {
        name: "s".into(),
        resource_ii: 1,
        recurrence_ii: 1,
        latency: 3,
        cells: 2,
        memory_ii: 1,
    };
    assert_eq!(base.ii(), 1);
    let banked = Stage {
        memory_ii: 4,
        ..base.clone()
    };
    assert_eq!(
        banked.ii(),
        4,
        "a 4-way bank conflict per iteration sets the II"
    );
    let rec = Stage {
        recurrence_ii: 5,
        ..banked
    };
    assert_eq!(
        rec.ii(),
        5,
        "the II is the max of resource, recurrence, and memory"
    );
}
