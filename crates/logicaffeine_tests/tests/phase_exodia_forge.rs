//! EXODIA Phase 2 gate — the FORGE's Z3 layer (D11 a + b, M15).
//!
//! Specs over 64-bit bitvectors for the JIT's integer micro-ops, proved
//! satisfiable and algebraically lawful, then GROUNDED: Z3-chosen witness
//! inputs (plus the adversarial corner battery) run through the actual
//! copy-and-patch machine code, the forge's reference interpreter, and
//! the spec itself — three independent evaluators that must agree. A
//! deliberate-bug canary proves the harness can fail.
//!
//! Z3-backed: behind the `verification` feature like every solver test.

#![cfg(all(not(target_arch = "wasm32"), feature = "verification"))]

use logicaffeine_synth::spec::{
    all_specs, prove_commutative, prove_min_div_wraps, prove_spec_satisfiable,
};
use logicaffeine_synth::witness::check_spec_with_witnesses;

/// Family-completeness guard preserving the original `all_specs_satisfiable`'s
/// `n >= 13` check: the integer spec family must be the full set. Per-spec
/// inhabitance itself is proved (and parallelized) by the `all_specs_satisfiable_s*`
/// shards in the sharding section below.
#[test]
fn all_specs_family_is_complete() {
    let n = all_specs().len();
    assert!(n >= 13, "expected the full integer spec family, got {n}");
}

/// The algebra the Architect's kernel-certified rules lean on, re-proved
/// at the BITVECTOR level: add/mul/and/or/xor/eq commute.
#[test]
fn commutativity_proofs() {
    for name in ["add", "mul", "and", "or", "xor", "eq"] {
        prove_commutative(name).unwrap_or_else(|e| panic!("{e}"));
    }
}

/// Subtraction must NOT prove commutative — the prover is not a rubber
/// stamp.
#[test]
fn sub_is_not_commutative() {
    assert!(prove_commutative("sub").is_err(), "sub commuting would be a prover bug");
}

/// The locked wrapping rim: ⌊MIN / −1⌋ wraps to MIN in the spec model,
/// exactly as `wrapping_div` does at runtime.
#[test]
fn min_div_minus_one_wraps_in_the_model() {
    prove_min_div_wraps().unwrap_or_else(|e| panic!("{e}"));
}

// ── Sharding ─────────────────────────────────────────────────────────────────
// THE GROUNDING GATE below is multi-minute and Z3-bound; nextest parallelizes
// ACROSS tests, never within one, so it is split into independent `#[test]`
// shards over the SAME deterministic `all_specs()` list, selected by
// `index % FORGE_SHARDS`. Each spec's Z3 query already builds its own fresh
// context, so per-spec shards are fully isolated. `forge_partition_tiles_specs`
// proves the shards tile `all_specs()` exactly — every spec checked by exactly
// one shard, none dropped or double-checked. Sharding changes scheduling only.

/// Number of parallel shards the per-spec grounding gate is split across.
const FORGE_SHARDS: usize = 4;

/// The indices into `all_specs()` owned by `shard` — every `i` with
/// `i % FORGE_SHARDS == shard`. Derived from the one shared spec list, so a shard
/// can never invent or drop a spec.
fn forge_shard(shard: usize) -> impl Iterator<Item = usize> {
    (0..all_specs().len()).filter(move |i| i % FORGE_SHARDS == shard)
}

/// The wide-arithmetic specs whose symbolic Z3 reasoning is a multi-minute
/// bit-blast bench: 64-bit `mul`'s no-overflow precondition forces Z3 to bit-blast
/// a full 128-bit product (the worst case for a bit-vector solver — ~280s witness /
/// ~273s satisfiability). These are grounded ONLY under the `#[ignore]`d `*_heavy`
/// tests, so the `--no-ignored` fast loop skips them while the light specs still run
/// inline. Keyed by op name, so any future heavy op is opted in here explicitly.
/// (`div` is left in the fast path — it measures ~12s, well under the suite floor.)
fn is_heavy(name: &str) -> bool {
    matches!(name, "mul")
}

/// One shard of THE GROUNDING GATE: for each owned LIGHT spec, Z3 witnesses + the
/// corner battery agree across machine code, reference interpreter, and spec —
/// including the side-exit agreement for checked ops at excluded inputs. Heavy
/// specs are skipped here and grounded by `witness_three_way_agreement_heavy`.
fn run_witness_shard(shard: usize) {
    let specs = all_specs();
    let mut owned = 0usize;
    for i in forge_shard(shard) {
        owned += 1;
        if is_heavy(specs[i].name) {
            continue;
        }
        let report = check_spec_with_witnesses(&specs[i], 8)
            .unwrap_or_else(|e| panic!("witness harness: {e}"));
        assert!(
            report.inputs_checked >= 100,
            "spec '{}' checked only {} inputs",
            report.spec,
            report.inputs_checked
        );
    }
    assert!(owned > 0, "witness shard {shard}/{FORGE_SHARDS} owns no specs");
}

macro_rules! forge_witness_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One shard of the three-way witness agreement gate — see `run_witness_shard`.
            #[test] fn $name() { run_witness_shard($idx); }
        )+
    };
}
forge_witness_shards! {
    witness_three_way_agreement_s0 => 0;
    witness_three_way_agreement_s1 => 1;
    witness_three_way_agreement_s2 => 2;
    witness_three_way_agreement_s3 => 3;
}

/// One shard of the inhabitance gate: each owned LIGHT spec is satisfiable (some
/// (a, b, r) meets pre ∧ post), each proved in its own fresh Z3 context. Heavy
/// specs are skipped here and proved by `all_specs_satisfiable_heavy`.
fn run_satisfiable_shard(shard: usize) {
    let specs = all_specs();
    let mut owned = 0usize;
    for i in forge_shard(shard) {
        owned += 1;
        if is_heavy(specs[i].name) {
            continue;
        }
        prove_spec_satisfiable(&specs[i]).unwrap_or_else(|e| panic!("{e}"));
    }
    assert!(owned > 0, "satisfiable shard {shard}/{FORGE_SHARDS} owns no specs");
}

macro_rules! forge_satisfiable_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One shard of the per-spec inhabitance gate — see `run_satisfiable_shard`.
            #[test] fn $name() { run_satisfiable_shard($idx); }
        )+
    };
}
forge_satisfiable_shards! {
    all_specs_satisfiable_s0 => 0;
    all_specs_satisfiable_s1 => 1;
    all_specs_satisfiable_s2 => 2;
    all_specs_satisfiable_s3 => 3;
}

// ── Heavy tier (mul) ─────────────────────────────────────────────────────────
// The wide-arithmetic specs are a multi-minute Z3 bit-blast bench, so they run in
// the full suite (and the baseline) but NOT the `--no-ignored` fast loop — exactly
// like `vm_fuzz_overnight`/`bench_wire_throughput`. Coverage is unchanged: every
// spec is still grounded — the light specs by the shards above, the heavy specs
// here. The fast shards `continue` past heavy specs, and these two cover them, so
// in the full run each spec is checked exactly once.

/// The grounding gate for the heavy specs (`mul`): Z3 witnesses + the corner
/// battery agree across machine code, reference interpreter, and spec.
#[test]
#[ignore = "heavy: 64-bit mul is a multi-minute Z3 bit-blast bench (~280s); runs in the full suite, not the --no-ignored fast loop"]
fn witness_three_way_agreement_heavy() {
    let mut checked = 0usize;
    for spec in all_specs().iter().filter(|s| is_heavy(s.name)) {
        let report = check_spec_with_witnesses(spec, 8)
            .unwrap_or_else(|e| panic!("witness harness: {e}"));
        assert!(
            report.inputs_checked >= 100,
            "spec '{}' checked only {} inputs",
            report.spec,
            report.inputs_checked
        );
        checked += 1;
    }
    assert!(checked > 0, "no heavy specs found — has `mul` left all_specs()?");
}

/// The inhabitance gate for the heavy specs (`mul`): each is satisfiable.
#[test]
#[ignore = "heavy: 64-bit mul satisfiability is a multi-minute Z3 bit-blast bench (~273s); runs in the full suite, not the --no-ignored fast loop"]
fn all_specs_satisfiable_heavy() {
    let mut checked = 0usize;
    for spec in all_specs().iter().filter(|s| is_heavy(s.name)) {
        prove_spec_satisfiable(spec).unwrap_or_else(|e| panic!("{e}"));
        checked += 1;
    }
    assert!(checked > 0, "no heavy specs found — has `mul` left all_specs()?");
}

/// Coverage guard: the `FORGE_SHARDS` shards tile `all_specs()` exactly — every
/// spec owned by exactly one shard, every shard non-empty. Proves the shard fns
/// together check the identical spec set the original `witness_three_way_agreement`
/// did (no spec dropped or double-counted).
#[test]
fn forge_partition_tiles_specs() {
    let len = all_specs().len();
    let mut hits = vec![0u32; len];
    for shard in 0..FORGE_SHARDS {
        for i in forge_shard(shard) {
            hits[i] += 1;
        }
    }
    assert!(
        hits.iter().all(|&h| h == 1),
        "specs not tiled exactly once by {FORGE_SHARDS} shards: {hits:?}"
    );
    for shard in 0..FORGE_SHARDS {
        assert!(forge_shard(shard).count() > 0, "forge shard {shard} owns no specs");
    }
}

/// The canary: a deliberately WRONG spec (add claiming subtraction) must
/// be caught by the harness — proof the three-way comparison can fail.
#[test]
fn harness_catches_a_deliberate_bug() {
    let wrong = logicaffeine_synth::spec::deliberately_wrong_spec_for_canary();
    assert!(
        check_spec_with_witnesses(&wrong, 4).is_err(),
        "the harness accepted a spec that contradicts the machine code"
    );
}
