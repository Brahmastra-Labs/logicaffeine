use super::*;

fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

fn rand_heap(rows: usize, words: usize, seed: u64) -> Vec<Row> {
    let mut rng = lcg(seed);
    (0..rows)
        .map(|_| (0..words).map(|_| rng()).collect())
        .collect()
}

fn sample_prim(kind: PrimKind) -> Prim {
    match kind {
        PrimKind::Copy => Prim::Copy { dst: 2, src: 0 },
        PrimKind::And => Prim::And { dst: 2, a: 0, b: 1 },
        PrimKind::Or => Prim::Or { dst: 2, a: 0, b: 1 },
        PrimKind::Not => Prim::Not { dst: 2, src: 0 },
        PrimKind::Xor => Prim::Xor { dst: 2, a: 0, b: 1 },
        PrimKind::Popcount => Prim::Popcount { dst: 2, src: 0 },
    }
}

/// Drift lock (Ambit tier): `exec_dram_ambit` realizes EXACTLY the descriptor's `dram_ambit` ops —
/// claiming a tier the executor can't run, or running one the descriptor forbids, fails here.
#[test]
fn dram_ambit_supports_exactly_the_descriptor_ambit_ops() {
    for d in PRIMS {
        let mut heap = rand_heap(3, 4, 0x5151 + d.kind as u64);
        let realized = exec_dram_ambit(&mut heap, &sample_prim(d.kind)).is_some();
        assert_eq!(
            realized, d.dram_ambit,
            "ambit realization for {:?} disagrees with the descriptor",
            d.kind
        );
    }
}

/// The Ambit DRAM tier agrees op-for-op with the VM reference over fuzzed rows — NOT and XOR
/// included (the boolean basis the Ambit cells add on top of unmodified ComputeDRAM).
#[test]
fn the_ambit_dram_tier_agrees_with_the_vm_reference() {
    for seed in 0..40u64 {
        let base = rand_heap(3, 8, 0x9000 + seed);
        for d in PRIMS.iter().filter(|d| d.dram_ambit) {
            let prim = sample_prim(d.kind);
            let mut ha = base.clone();
            let mut hr = base.clone();
            assert!(exec_dram_ambit(&mut ha, &prim).is_some());
            exec_reference(&mut hr, &prim);
            assert_eq!(
                ha[2], hr[2],
                "ambit {:?} diverged from the VM reference",
                d.kind
            );
        }
    }
}

/// SINGLE SOURCE OF TRUTH: the DRAM executor supports EXACTLY the ops the descriptor advertises as
/// `dram: true` — no more, no less. Add an op as DRAM-capable without wiring the executor and this
/// fails; claim the executor does an op the descriptor forbids and this fails.
#[test]
fn dram_executor_supports_exactly_the_descriptor_dram_ops() {
    let mut heap = rand_heap(8, 4, 1);
    for d in PRIMS {
        let probe = match d.kind {
            PrimKind::Copy => Prim::Copy { dst: 0, src: 1 },
            PrimKind::And => Prim::And { dst: 0, a: 1, b: 2 },
            PrimKind::Or => Prim::Or { dst: 0, a: 1, b: 2 },
            PrimKind::Not => Prim::Not { dst: 0, src: 1 },
            PrimKind::Xor => Prim::Xor { dst: 0, a: 1, b: 2 },
            PrimKind::Popcount => Prim::Popcount { dst: 0, src: 1 },
        };
        assert_eq!(
            exec_dram(&mut heap, &probe).is_some(),
            d.dram,
            "DRAM support for {:?} must match the descriptor",
            d.kind
        );
    }
}

/// Every capable plane computes the SAME result as the VM reference, over fuzzed operands — the
/// core equivalence that makes a placed program ≡VM regardless of where each op lands.
#[test]
fn all_capable_planes_agree_with_the_vm_reference() {
    for seed in 1..40u64 {
        let base = rand_heap(8, 5, seed);
        let prims = [
            Prim::Copy { dst: 0, src: 1 },
            Prim::And { dst: 0, a: 1, b: 2 },
            Prim::Or { dst: 0, a: 1, b: 2 },
            Prim::Not { dst: 0, src: 1 },
            Prim::Xor { dst: 0, a: 1, b: 2 },
            Prim::Popcount { dst: 0, src: 1 },
        ];
        for prim in &prims {
            let mut want = base.clone();
            exec_reference(&mut want, prim);
            let d = desc(prim.kind());
            if d.host_cpu {
                let mut h = base.clone();
                exec_on(Plane::HostCpu, &mut h, prim);
                assert_eq!(h, want, "host-CPU disagrees on {:?}", prim.kind());
            }
            if d.fabric {
                let mut h = base.clone();
                exec_on(Plane::Fabric, &mut h, prim);
                assert_eq!(h, want, "fabric disagrees on {:?}", prim.kind());
            }
            if d.dram {
                let mut h = base.clone();
                exec_on(Plane::Dram, &mut h, prim);
                assert_eq!(h, want, "DRAM disagrees on {:?}", prim.kind());
            }
        }
    }
}

/// The placement pass covers every op (never unplaceable) and routes by the cost model: wide bulk
/// bitwise/copy → DRAM, XOR (no in-DRAM primitive) → fabric, reductions → host CPU.
#[test]
fn placement_covers_every_op_across_three_planes() {
    // Wide operands: bulk bitwise/copy go in-DRAM; xor → fabric; popcount → host.
    assert_eq!(
        place(&Prim::And { dst: 0, a: 1, b: 2 }, WIDE_WORDS),
        Plane::Dram
    );
    assert_eq!(
        place(&Prim::Or { dst: 0, a: 1, b: 2 }, WIDE_WORDS),
        Plane::Dram
    );
    assert_eq!(
        place(&Prim::Copy { dst: 0, src: 1 }, WIDE_WORDS),
        Plane::Dram
    );
    assert_eq!(
        place(&Prim::Not { dst: 0, src: 1 }, WIDE_WORDS),
        Plane::Fabric
    );
    assert_eq!(
        place(&Prim::Xor { dst: 0, a: 1, b: 2 }, WIDE_WORDS),
        Plane::Fabric
    );
    assert_eq!(
        place(&Prim::Popcount { dst: 0, src: 1 }, WIDE_WORDS),
        Plane::HostCpu
    );
    // Narrow operands: no bytes-not-moved win, so bitwise falls to the fabric, not DRAM.
    assert_eq!(place(&Prim::And { dst: 0, a: 1, b: 2 }, 1), Plane::Fabric);
    // Every op is placeable, and the chosen plane can actually run it (descriptor-consistent).
    for d in PRIMS {
        let p = match d.kind {
            PrimKind::Copy => Prim::Copy { dst: 0, src: 1 },
            PrimKind::And => Prim::And { dst: 0, a: 1, b: 2 },
            PrimKind::Or => Prim::Or { dst: 0, a: 1, b: 2 },
            PrimKind::Not => Prim::Not { dst: 0, src: 1 },
            PrimKind::Xor => Prim::Xor { dst: 0, a: 1, b: 2 },
            PrimKind::Popcount => Prim::Popcount { dst: 0, src: 1 },
        };
        for w in [1usize, WIDE_WORDS, 64] {
            let plane = place(&p, w);
            let ok = match plane {
                Plane::HostCpu => d.host_cpu,
                Plane::Fabric => d.fabric,
                Plane::Dram => d.dram,
            };
            assert!(
                ok,
                "{:?} placed on a plane that cannot run it (w={w})",
                d.kind
            );
        }
    }
}

/// THE FLAGSHIP: a single LOGOS program placed across ALL THREE planes produces byte-identical
/// output to the VM. A bitmap pipeline — intersection (AND, in-DRAM) → symmetric-difference (XOR,
/// fabric) → population count (host) → snapshot copy (in-DRAM) — over wide operands, so the
/// placement genuinely spans host-CPU + fabric + DRAM.
#[test]
fn split_program_across_all_planes_eq_vm() {
    let words = 64; // wide → bulk bitwise/copy land in-DRAM
    let program = [
        Prim::And { dst: 4, a: 0, b: 1 },  // intersection → DRAM
        Prim::Xor { dst: 5, a: 0, b: 1 },  // symmetric difference → fabric
        Prim::Popcount { dst: 6, src: 4 }, // |intersection| → host CPU
        Prim::Copy { dst: 7, src: 5 },     // snapshot → DRAM (RowClone)
        Prim::Or { dst: 3, a: 4, b: 5 },   // union of the two → DRAM
    ];
    // Non-vacuity: the placement really uses all three planes.
    let used: std::collections::BTreeSet<Plane> = program.iter().map(|p| place(p, words)).collect();
    assert!(
        used.contains(&Plane::Dram)
            && used.contains(&Plane::Fabric)
            && used.contains(&Plane::HostCpu),
        "the program must span all three planes, got {used:?}"
    );
    for seed in 1..30u64 {
        let heap0 = rand_heap(8, words, seed);
        let placed = run_placed(&heap0, &program, words);
        let reference = run_reference(&heap0, &program);
        assert_eq!(
            placed, reference,
            "the placed program diverged from the VM (seed {seed})"
        );
    }
}
