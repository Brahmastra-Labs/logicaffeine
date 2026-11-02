use super::*;

fn lr(var: usize, start: i64, end: i64) -> LiveRange {
    LiveRange::new(var, start, end)
}

#[test]
fn low_pressure_block_allocates() {
    //  v0:[0,2)  v1:[1,3)  v2:[3,5)  — peak pressure 2 (v0,v1 overlap), 2 registers fit.
    let ranges = vec![lr(0, 0, 2), lr(1, 1, 3), lr(2, 3, 5)];
    assert_eq!(register_pressure(&ranges), 2);
    match allocate(&ranges, 2) {
        Allocation::Allocated(reg_of) => {
            assert!(is_valid_allocation(&ranges, 2, &reg_of), "{reg_of:?}")
        }
        o => panic!("expected Allocated, got {o:?}"),
    }
}

#[test]
fn high_pressure_block_must_spill_with_a_clique() {
    // Four variables all live across instruction 2; only 3 registers ⇒ one must spill.
    let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7), lr(3, 2, 8)];
    assert_eq!(register_pressure(&ranges), 4);
    match allocate(&ranges, 3) {
        Allocation::Spill {
            pressure,
            must_spill,
        } => {
            assert_eq!(pressure, 4, "needs 4 registers");
            assert!(must_spill.len() > 3, "clique must exceed register count");
            assert!(
                is_spill_certificate(&ranges, 3, &must_spill),
                "{must_spill:?}"
            );
        }
        o => panic!("expected Spill, got {o:?}"),
    }
}

#[test]
fn exactly_at_pressure_fits() {
    // Pressure 3, exactly 3 registers ⇒ allocates (no spill).
    let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7)];
    assert_eq!(register_pressure(&ranges), 3);
    assert!(matches!(allocate(&ranges, 3), Allocation::Allocated(_)));
}

#[test]
fn matches_pressure_oracle_on_random_blocks() {
    let mut s: u64 = 0xC2B2AE3D27D4EB4F;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..500 {
        let n = (next() % 10) as usize + 1;
        let registers = (next() % 5) as usize + 1;
        let ranges: Vec<LiveRange> = (0..n)
            .map(|v| {
                let a = (next() % 12) as i64;
                let len = (next() % 6) as i64 + 1;
                lr(v, a, a + len)
            })
            .collect();
        let pressure = register_pressure(&ranges);
        match allocate(&ranges, registers) {
            Allocation::Allocated(reg_of) => {
                assert!(
                    pressure <= registers,
                    "Allocated but pressure {pressure} > {registers}"
                );
                assert!(
                    is_valid_allocation(&ranges, registers, &reg_of),
                    "invalid: {reg_of:?}"
                );
            }
            Allocation::Spill {
                pressure: p,
                must_spill,
            } => {
                assert!(
                    pressure > registers,
                    "Spill but pressure {pressure} ≤ {registers}"
                );
                assert_eq!(p, pressure);
                assert!(
                    is_spill_certificate(&ranges, registers, &must_spill),
                    "{must_spill:?}"
                );
            }
        }
    }
}

#[test]
fn robustness_edge_cases() {
    // Single variable fits one register.
    assert!(matches!(
        allocate(&[lr(0, 0, 5)], 1),
        Allocation::Allocated(_)
    ));
    // Disjoint live ranges all share one register.
    let disjoint = vec![lr(0, 0, 2), lr(1, 2, 4), lr(2, 4, 6)];
    assert_eq!(register_pressure(&disjoint), 1);
    match allocate(&disjoint, 1) {
        Allocation::Allocated(a) => assert!(is_valid_allocation(&disjoint, 1, &a), "{a:?}"),
        o => panic!("disjoint ranges fit 1 register: {o:?}"),
    }
    // Six identical ranges all mutually interfere → spill over 4 registers, clique of 5.
    let same: Vec<LiveRange> = (0..6).map(|v| lr(v, 0, 10)).collect();
    assert_eq!(register_pressure(&same), 6);
    match allocate(&same, 4) {
        Allocation::Spill {
            pressure,
            must_spill,
        } => {
            assert_eq!(pressure, 6);
            assert_eq!(
                must_spill.len(),
                5,
                "the first overflow is registers+1 wide"
            );
            assert!(is_spill_certificate(&same, 4, &must_spill));
        }
        o => panic!("6 identical ranges over 4 registers must spill: {o:?}"),
    }
    // Huge coordinates do not overflow the sweep.
    let big = vec![lr(0, 0, i64::MAX / 2), lr(1, 1, i64::MAX / 2)];
    assert_eq!(register_pressure(&big), 2);
    assert!(matches!(allocate(&big, 2), Allocation::Allocated(_)));
    // Zero variables is trivially allocatable.
    assert!(matches!(allocate(&[], 3), Allocation::Allocated(_)));
}

#[test]
fn a_bad_spill_certificate_is_rejected() {
    let ranges = vec![lr(0, 0, 5), lr(1, 1, 6), lr(2, 2, 7), lr(3, 2, 8)];
    assert!(
        !is_spill_certificate(&ranges, 3, &[0, 1]),
        "two vars don't exceed 3 registers"
    );
    // v2 and a far-future non-overlapping var would not pairwise interfere.
    assert!(
        is_spill_certificate(&ranges, 3, &[0, 1, 2, 3]),
        "all four are mutually live"
    );
}
