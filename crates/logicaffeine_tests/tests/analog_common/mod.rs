//! Shared helpers for the analog lock family — the ONE copy of the deterministic battery
//! machinery every `analog_*_lock` uses. Helpers only: no assertions live here, so this file
//! can never weaken a lock; each lock's corpus, seeds-XOR patterns, and assertions stay in the
//! lock that owns them.
#![allow(dead_code)]

/// Fixed forever — the campaign-wide base seed of every deterministic battery.
pub const BASE_SEED: u64 = 0xA11A_00C0_FFEE_5EED;

/// SplitMix64 — bit-reproducible on every platform, no `rand` crate, no clock.
pub fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A deterministic f64 point inside `[lo, hi]`.
pub fn point_in(state: &mut u64, lo: f64, hi: f64) -> f64 {
    let u = (splitmix(state) >> 11) as f64 / (1u64 << 53) as f64;
    lo + u * (hi - lo)
}

/// A deterministic integer inside `[lo, hi]`.
pub fn int_in(state: &mut u64, lo: i64, hi: i64) -> i64 {
    lo + (splitmix(state) % (hi - lo + 1) as u64) as i64
}

/// The VM's answer for a single-`Show` program, parsed as a float (LOGOS float formatting is
/// shortest-round-trip, so the parse is exact).
pub fn vm_float(src: &str) -> f64 {
    let out = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("VM output is not a float: {:?}", out.output))
}

/// The VM's answer for a single-`Show` program, parsed as an integer.
pub fn vm_int(src: &str) -> i64 {
    let out = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(out.error, None, "corpus program must run on the VM:\n{src}");
    out.output
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("VM output is not an int: {:?}", out.output))
}

pub fn have_ngspice() -> bool {
    std::process::Command::new("ngspice")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
