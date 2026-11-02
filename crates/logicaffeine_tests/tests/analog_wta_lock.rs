//! ★ ANALOG WTA LOCK ★ — the winner-take-all margin certificate: a winner is PROVABLE iff
//! the certified top-two margin clears the offset resolution band, shown from both sides by
//! fixed-seed Monte-Carlo.
//!
//! What this file locks:
//!   1. THE THEOREM HOLDS — a provable margin never flips across 256 modeled dies (each
//!      competitor's offset drawn inside the k·σ envelope).
//!   2. THE BAND IS REAL — an unprovable near-tie DOES flip on a real die (anti-vacuity:
//!      remove the band and the certificate would lie).
//!   3. HONEST LEDGER STATE — `ArgMax` stays PLANNED (the ontology lock pins it): the tile
//!      lowers only when this margin logic reaches the independent replay checker.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. The band never narrows without a model change.

mod analog_common;

use analog_common::BASE_SEED;
use logicaffeine_compile::codegen_ana::wta::wta_winner;
use logicaffeine_compile::codegen_ana::{sky130_nonideal, KSIGMA};

/// The same clamped-gaussian class the instance sampler uses (offsets inside ±k·σ).
fn offsets(n: usize, sigma: f64, seed: u64) -> Vec<f64> {
    let mut st = seed ^ 0xD1E5_ED0F_ABBA_C0DE;
    let mut splitmix = || {
        st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = st;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut uniform = move || ((splitmix() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    (0..n)
        .map(|_| {
            let (u1, u2) = (uniform(), uniform());
            let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            g.clamp(-KSIGMA, KSIGMA) * sigma
        })
        .collect()
}

#[test]
fn a_provable_margin_never_flips_across_the_die_battery() {
    let model = sky130_nonideal();
    let s = 1.0;
    let values = [0.10, 0.45, 0.30, 0.02];
    let out = wta_winner(&values, &model, s).expect("judged");
    assert_eq!(out.winner, 1);
    assert!(
        out.provable,
        "margin {} clears band {}",
        out.margin, out.band
    );
    assert!(out.margin > out.band);

    for die in 0..256u64 {
        let off = offsets(values.len(), model.offset_sigma_v.0 / s, BASE_SEED ^ die);
        let physical: Vec<f64> = values.iter().zip(&off).map(|(v, o)| v + o).collect();
        let phys_winner = (0..physical.len())
            .max_by(|a, b| physical[*a].partial_cmp(&physical[*b]).unwrap())
            .unwrap();
        assert_eq!(
            phys_winner, out.winner,
            "die {die}: a provable winner must never flip"
        );
    }
}

#[test]
fn an_unprovable_near_tie_really_flips_on_a_real_die() {
    let model = sky130_nonideal();
    let s = 1.0;
    // A margin much smaller than the band: two competitors one σ apart.
    let near = [0.300, 0.300 + model.offset_sigma_v.0, 0.1];
    let out = wta_winner(&near, &model, s).expect("judged");
    assert_eq!(out.winner, 1, "ideally the second entry wins");
    assert!(!out.provable, "a 1σ margin cannot clear the 2·k·σ band");

    let mut flipped = false;
    for die in 0..256u64 {
        let off = offsets(
            near.len(),
            model.offset_sigma_v.0 / s,
            BASE_SEED ^ (die << 1),
        );
        let physical: Vec<f64> = near.iter().zip(&off).map(|(v, o)| v + o).collect();
        let phys_winner = (0..physical.len())
            .max_by(|a, b| physical[*a].partial_cmp(&physical[*b]).unwrap())
            .unwrap();
        if phys_winner != out.winner {
            flipped = true;
            break;
        }
    }
    assert!(
        flipped,
        "the band is real physics: some die must pick the other competitor"
    );
}

#[test]
fn degenerate_inputs_are_refused_not_guessed() {
    let model = sky130_nonideal();
    assert!(
        wta_winner(&[0.5], &model, 1.0).is_none(),
        "one competitor is not a competition"
    );
    assert!(wta_winner(&[], &model, 1.0).is_none());
    assert!(wta_winner(&[0.1, f64::NAN], &model, 1.0).is_none());
    assert!(
        wta_winner(&[0.1, 0.2], &model, 0.0).is_none(),
        "a zero scale is not a scale"
    );
}
