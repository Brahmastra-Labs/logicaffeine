//! QA control-loop locks — the BEHAVIOR of the elastic controller, not just its bounds. The "runs on
//! a potato" thesis is that quality glides continuously: probe upward on a clean link, back off
//! multiplicatively under loss, deeper the worse the loss, and converge on the capacity without ever
//! overshooting. A controller that stayed in-bounds but never actually moved would pass the bounds
//! tests and still be useless. These lock the DYNAMICS — the AIMD saw-tooth and the jitter buffer's
//! grow/shrink response — over swept conditions.

use logicaffeine_compile::compile::tw_outcome;

struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo + 1) as u64) as usize
    }
}

/// Run a program whose every `Show` line must print `true`, asserting the invariant held every time.
fn assert_all_true(program: &str, ctx: &str) {
    let r = tw_outcome(program);
    assert_eq!(
        r.error, None,
        "{ctx}: runs clean, got {:?}\n---\n{program}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert!(!lines.is_empty(), "{ctx}: produced no output\n{program}");
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(
            *l, "true",
            "{ctx}: invariant #{i} failed (got {l:?})\n---\n{program}"
        );
    }
}

/// AIMD, additive-increase leg: on a CLEAN link with spare capacity the controller probes STRICTLY
/// upward — quality climbs, it never just sits. (`current < cap` ⇒ `adaptRate > current`.)
#[test]
fn a_clean_link_probes_the_rate_strictly_upward() {
    let mut rng = R(0xC1EA_2000_0001);
    let mut prog = String::from("## Main\n");
    for _ in 0..60 {
        let link = rng.range(100, 50_000);
        let cap = (link * 9) / 10;
        let current = rng.range(8, cap - 1); // strictly below the 90% cap
        prog.push_str(&format!(
            "Show adaptRate({current}, {link}, 0) is greater than {current}.\n"
        ));
    }
    assert_all_true(&prog, "clean-link probe-up");
}

/// AIMD, multiplicative-decrease leg: under loss the controller backs off STRICTLY — a congested link
/// always sheds rate. (`loss ≥ 2` and `current ≥ 20` ⇒ `adaptRate < current`.)
#[test]
fn loss_backs_the_rate_off_strictly() {
    let mut rng = R(0xC1EA_2000_0002);
    let mut prog = String::from("## Main\n");
    for _ in 0..60 {
        let link = rng.range(100, 50_000);
        let current = rng.range(20, 40_000);
        let loss = rng.range(2, 100);
        prog.push_str(&format!(
            "Show adaptRate({current}, {link}, {loss}) is less than {current}.\n"
        ));
    }
    assert_all_true(&prog, "loss backoff");
}

/// AIMD severity: the backoff DEEPENS monotonically with loss — worse loss never yields a HIGHER rate.
/// (`lossA < lossB` ⇒ `adaptRate(·,·,lossB) ≤ adaptRate(·,·,lossA)`.)
#[test]
fn backoff_deepens_monotonically_with_loss_severity() {
    let mut rng = R(0xC1EA_2000_0003);
    let mut prog = String::from("## Main\n");
    for _ in 0..60 {
        let link = rng.range(1_000, 50_000);
        let current = rng.range(100, 20_000);
        let a = rng.range(2, 50);
        let b = rng.range(a, 100); // b ≥ a → deeper or equal loss
        prog.push_str(&format!(
            "Show adaptRate({current}, {link}, {b}) is at most adaptRate({current}, {link}, {a}).\n"
        ));
    }
    assert_all_true(&prog, "severity-monotone backoff");
}

/// The elastic GLIDE: iterated on a clean link of fixed capacity, the rate converges EXACTLY on the
/// 90% cap (9000 for a 10 Mbps link) and never overshoots — the saw-tooth's steady state, the whole
/// point of "continuous, never snapping between tiers."
#[test]
fn the_controller_glides_to_the_cap_and_never_overshoots() {
    let prog = "## Main\n\
        Let mutable r be 50.\n\
        Repeat for i from 1 to 80:\n\
        \x20   Set r to adaptRate(r, 10000, 0).\n\
        Show r.\n";
    let out = tw_outcome(prog);
    assert_eq!(out.error, None, "glide runs clean: {:?}", out.error);
    assert_eq!(
        out.output.trim(),
        "9000",
        "the rate must converge on the 90% cap of a 10 Mbps link"
    );

    // Starting ABOVE the cap converges DOWN to the same fixed point — the cap is a true attractor.
    let prog_high = "## Main\n\
        Let mutable r be 9999.\n\
        Repeat for i from 1 to 80:\n\
        \x20   Set r to adaptRate(r, 10000, 0).\n\
        Show r.\n";
    let out_high = tw_outcome(prog_high);
    assert_eq!(
        out_high.error, None,
        "high-start glide runs clean: {:?}",
        out_high.error
    );
    assert_eq!(
        out_high.output.trim(),
        "9000",
        "an over-cap rate must be pulled down to the cap"
    );
}

/// The jitter buffer is elastic BOTH ways: it grows when jitter spikes and shrinks back on a clean,
/// low-jitter link — absorbing bursts without permanently bloating latency.
#[test]
fn the_jitter_buffer_grows_under_jitter_and_shrinks_when_clean() {
    // Jitter well above half the current buffer ⇒ grow.
    let grow = "## Main\nShow elasticBuffer(100, 90, 0) is greater than 100.\n";
    assert_all_true(grow, "jitter buffer grows");
    // Low jitter (< 1/8 of buffer) on a loss-free link ⇒ shrink.
    let shrink = "## Main\nShow elasticBuffer(200, 10, 0) is less than 200.\n";
    assert_all_true(shrink, "jitter buffer shrinks");
}

/// The jitter buffer never leaves its safe envelope [20 ms, 500 ms] under ANY condition — it can't
/// collapse to nothing (constant re-buffering) or balloon unboundedly (dead latency).
#[test]
fn the_jitter_buffer_stays_within_its_safe_envelope() {
    let mut rng = R(0xC1EA_2000_0004);
    let mut prog = String::from("## Main\n");
    for _ in 0..60 {
        let current = rng.range(0, 1_000);
        let jitter = rng.range(0, 1_000);
        let loss = rng.range(0, 100);
        prog.push_str(&format!(
            "Show elasticBuffer({current}, {jitter}, {loss}) is at least 20.\n"
        ));
        prog.push_str(&format!(
            "Show elasticBuffer({current}, {jitter}, {loss}) is less than 501.\n"
        ));
    }
    assert_all_true(&prog, "jitter buffer envelope");
}
