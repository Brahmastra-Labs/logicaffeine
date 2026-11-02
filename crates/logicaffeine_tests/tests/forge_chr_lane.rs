//! FORGE `chr` LANE — `chr(code)` tiers via a `ChrGuard` (DEOPTS to the VM when the code is not a valid
//! Unicode scalar, so the VM re-runs `chr` and TRAPS exactly like the tree-walker) plus `RegBox::CharText`
//! (boxes a valid code to a 1-char `Text` at region exit). Correctness = the printed char matches the
//! tree-walker AND an invalid code errors on BOTH tiers (the forge's deopt reproduces the VM's trap).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Assert the forge and the tree-walker agree on both output AND whether the program errored; optionally
/// that the hot region tiered to native.
fn chr_eq_tw(label: &str, src: &str, want_tier: bool) {
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert_eq!(
        forge.output.trim(),
        tw.output.trim(),
        "[{label}] output diverged: forge={:?} tw={:?}",
        forge.output.trim(),
        tw.output.trim()
    );
    assert_eq!(
        forge.error.is_some(),
        tw.error.is_some(),
        "[{label}] error mismatch: forge={:?} tw={:?}",
        forge.error,
        tw.error
    );
    #[cfg(target_arch = "x86_64")]
    if want_tier {
        let (_, region_ok) = tier.region_counts();
        assert!(
            region_ok >= 1,
            "[{label}] did NOT tier its hot region (region={region_ok})"
        );
    }
}

#[test]
fn forge_chr_lane() {
    // chr in a hot loop, its 1-char result CONSUMED to an Int within the region (`length of chr(k)` == 1) —
    // the tierable shape: the guard runs each iteration (validating the code) with no boxed region output.
    chr_eq_tw(
        "chr consumed via length",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 500:\n\
         \x20   Set acc to acc + length of chr(k).\n\x20   Set k to k + 1.\nShow acc.\n",
        true,
    );
    // The whole ASCII + Latin-1 range (all valid) through the guard.
    chr_eq_tw(
        "chr over 1..255 via length",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 255:\n\
         \x20   Set acc to acc + length of chr(k).\n\x20   Set k to k + 1.\nShow acc.\n",
        true,
    );
    // A char produced in a hot loop and SHOWN as the region's live output. A loop-carried Text output is
    // live-in (the loop may run zero times), which the scalar-region gate declines — the forge falls back
    // to the VM and must still print identically to the tree-walker (correctness without tiering).
    chr_eq_tw(
        "chr region output (VM fallback ok)",
        "## Main\nLet mutable c be chr(65).\nLet mutable k be 66.\nWhile k is at most 500:\n\
         \x20   Set c to chr(k).\n\x20   Set k to k + 1.\nShow c.\n",
        false,
    );
    // DEOPT-ON-INVALID: the `length of chr(k)` loop TIERS for the valid codes, then hits a SURROGATE
    // (0xD800 = 55296) — the guard must side-exit so the VM re-runs `chr` and TRAPS, erroring identically
    // to the tree-walker. (This is the case that actually exercises the ChrGuard's `cont_1` deopt.)
    chr_eq_tw(
        "chr hot loop hits a surrogate → deopt + trap",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 100.\nWhile k is at most 100000:\n\
         \x20   Set acc to acc + length of chr(k).\n\x20   Set k to k + 1.\nShow acc.\n",
        false,
    );
    // One-shot invalid codes (not hot → run on the VM) still must trap identically.
    chr_eq_tw(
        "chr surrogate errors",
        "## Main\nLet c be chr(55296).\nShow c.\n",
        false,
    );
    chr_eq_tw(
        "chr past-max errors",
        "## Main\nLet c be chr(1114112).\nShow c.\n",
        false,
    );
    chr_eq_tw(
        "chr negative errors",
        "## Main\nLet c be chr(0 - 1).\nShow c.\n",
        false,
    );
}
