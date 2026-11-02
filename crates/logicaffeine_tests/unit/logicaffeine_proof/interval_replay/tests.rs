use super::*;

/// `0.5·x + 0.25` over `x ∈ [-1, 1]`, unit scale — replays to `[-0.25, 0.75]` (± ULPs).
fn affine() -> (Vec<RCell>, RangeClaim) {
    let cells = vec![
        RCell::Gain {
            a: RSrc::In(0),
            k: 0.5,
        },
        RCell::Sum {
            inputs: vec![RSrc::Val(0), RSrc::Const(0.25)],
            weights: vec![1.0, 1.0],
        },
    ];
    let claim = RangeClaim {
        input_lo: vec![-1.0],
        input_hi: vec![1.0],
        scale_log2: vec![0, 0],
        node_lo_v: vec![-0.51, -0.26],
        node_hi_v: vec![0.51, 0.76],
        rail_half_span_v: 0.9,
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    (cells, claim)
}

#[test]
fn a_sound_claim_passes() {
    let (cells, claim) = affine();
    assert!(check_range_claim(&cells, &claim));
}

#[test]
fn a_claim_tighter_than_reality_is_rejected() {
    let (cells, mut claim) = affine();
    claim.node_hi_v[1] = 0.5; // reality reaches 0.75
    assert!(!check_range_claim(&cells, &claim));
}

#[test]
fn a_claim_past_the_rail_is_rejected() {
    let (cells, mut claim) = affine();
    claim.rail_half_span_v = 0.7; // node 1 is claimed to 0.76
    assert!(!check_range_claim(&cells, &claim));
}

#[test]
fn a_starved_node_is_rejected() {
    let (cells, mut claim) = affine();
    claim.noise_floor_v = 0.01;
    claim.min_full_scale_over_noise = 100.0; // needs 1.0 V full scale; node 0 claims 0.51
    assert!(!check_range_claim(&cells, &claim));
}

#[test]
fn scales_multiply_bounds_exactly() {
    let cells = vec![RCell::Gain {
        a: RSrc::In(0),
        k: 1.0,
    }];
    let claim = RangeClaim {
        input_lo: vec![-4.0],
        input_hi: vec![4.0],
        scale_log2: vec![-3], // volt = math / 8 → [-0.5, 0.5]
        node_lo_v: vec![-0.51],
        node_hi_v: vec![0.51],
        rail_half_span_v: 0.72,
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    assert!(check_range_claim(&cells, &claim));
}

#[test]
fn multiplication_uses_all_four_corners() {
    // x ∈ [-2, 1], y ∈ [-3, 1]: x·y ∈ [-6, 6] (the lo·lo corner is the max).
    let cells = vec![RCell::Mul {
        a: RSrc::In(0),
        b: RSrc::In(1),
    }];
    let mut claim = RangeClaim {
        input_lo: vec![-2.0, -3.0],
        input_hi: vec![1.0, 1.0],
        scale_log2: vec![-4], // [-0.375, 0.375]
        node_lo_v: vec![-0.38],
        node_hi_v: vec![0.38],
        rail_half_span_v: 0.72,
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    assert!(check_range_claim(&cells, &claim));
    claim.node_hi_v[0] = 0.2; // misses the (-2)·(-3) corner
    assert!(!check_range_claim(&cells, &claim));
}

#[test]
fn malformed_claims_are_rejected_fail_closed() {
    let (cells, base) = affine();
    for tamper in [
        |c: &mut RangeClaim| c.node_lo_v.pop().map(|_| ()).unwrap(),
        |c: &mut RangeClaim| c.input_hi[0] = f64::NAN,
        |c: &mut RangeClaim| c.node_hi_v[0] = f64::INFINITY,
        |c: &mut RangeClaim| c.rail_half_span_v = 0.0,
        |c: &mut RangeClaim| c.min_full_scale_over_noise = 0.5,
    ] {
        let mut claim = base.clone();
        tamper(&mut claim);
        assert!(
            !check_range_claim(&cells, &claim),
            "tampered claim must be rejected"
        );
    }
    // A dangling reference is rejected, not a panic.
    let dangling = vec![RCell::Gain {
        a: RSrc::Val(7),
        k: 1.0,
    }];
    let claim = RangeClaim {
        scale_log2: vec![0],
        node_lo_v: vec![0.0],
        node_hi_v: vec![0.0],
        ..base
    };
    assert!(!check_range_claim(&dangling, &claim));
}
