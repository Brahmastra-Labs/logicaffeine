//! W13 — `obs_tier_differential`: observation across the execution tiers.
//!
//! This lock joins the tiers on the
//! observation channel. Two invariants must hold for every tier, forever:
//!
//!   1. **Program output is tier-identical** for an observation-bearing program.
//!      The presence of `Note`/`Warn`/`Record`/`During` may never make two tiers
//!      disagree about what the program *did*. A divergence here is a COMPILER
//!      bug (never the user's) — this is the safety net the whole wave rests on.
//!
//!   2. **The emission floor only rises.** The observation *channel* is a
//!      profile-scoped, elidable overlay (constitution §0.7): a tier that has
//!      not yet learned to emit simply ELIDES observation — it never diverges on
//!      the program's real work. The tree-walker is the current floor (it emits);
//!      the register VM currently elides. This lock pins that floor as a ratchet:
//!      the row where a tier moves from "elides" to "emits" can only ever climb,
//!      and until it does, elision must stay perfectly output-transparent.
//!
//! When the VM (then JIT/AOT/WASM) learns to emit, the `vm_elides_*` row is
//! promoted to a `vm_emits_*` row and the floor rises — this file is where that
//! ratchet is recorded.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::observe;

/// A spread of observation shapes over shared state — the fixture both
/// invariants are proven against. Nested spans, observation inside a loop,
/// payloads referencing mutable locals: the awkward cases, on purpose.
const OBSERVED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Note that the run began with total.\n\
    \x20   During accumulating the values:\n\
    \x20       Repeat for i from 1 to 4:\n\
    \x20           Set total to total + i.\n\
    \x20           During folding one step:\n\
    \x20               Record that a step folded with i and total.\n\
    \x20   Warn that the fold is complete with total.\n\
    \x20   Show total.\n";

/// The identical program with every observation statement and span stripped —
/// only the real work remains.
const STRIPPED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Repeat for i from 1 to 4:\n\
    \x20       Set total to total + i.\n\
    \x20   Show total.\n";

// ── Invariant 1: program output is tier-identical ──────────────────────────

/// Both tiers agree on the program's output when observation is present. The
/// tree-walker and the VM differ only by a documented, observation-independent
/// convention — the VM terminates `Show` with a trailing newline, the
/// tree-walker does not — so agreement is asserted modulo that trailing
/// whitespace. What matters for THIS wave: observation introduces no NEW
/// divergence. If the observation overlay ever changed the program's meaning on
/// one tier, the trimmed outputs would part — the exact bug this wave forbids.
#[test]
fn program_output_is_tier_identical_under_observation() {
    let tw = tw_outcome(OBSERVED);
    let vm = vm_outcome(OBSERVED);
    assert_eq!(
        tw.error, None,
        "tree-walker runs the observed program cleanly: {:?}",
        tw.error
    );
    assert_eq!(
        vm.error, None,
        "VM runs the observed program cleanly: {:?}",
        vm.error
    );
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tiers must agree on program output (mod the documented trailing-newline \
         convention) — observation is not allowed to diverge them"
    );
    // 1+2+3+4 = 10, and only the program-output channel carries it.
    assert_eq!(
        tw.output.trim_end(),
        "10",
        "and the arithmetic is actually done"
    );
}

/// Elision is output-transparent on EVERY tier: the observed program prints
/// exactly what the stripped program prints, tier by tier. This is the
/// two-channel contract restated as a tier-differential — proven against each
/// tier's own stripped baseline so any tier-local formatting cancels.
#[test]
fn elision_is_output_transparent_on_every_tier() {
    assert_eq!(
        tw_outcome(OBSERVED).output,
        tw_outcome(STRIPPED).output,
        "tree-walker: observation elides to nothing on the output channel"
    );
    assert_eq!(
        vm_outcome(OBSERVED).output,
        vm_outcome(STRIPPED).output,
        "VM: observation elides to nothing on the output channel"
    );
}

// ── Invariant 2: the emission floor (a ratchet that only rises) ─────────────

/// The tree-walker is the current emission floor: it emits a structured event
/// for every executed observation site. The fixture's sites, unrolled: 1 `Note`
/// + (4 loop iterations × 1 `Record`) + 1 `Warn` = 6 message events, plus the
/// balanced span boundaries (outer `During` + 4 inner `During`s = 5 enter/exit
/// pairs). If a future change drops the floor, this count catches it.
#[test]
fn tree_walker_is_the_emission_floor() {
    observe::install();
    let out = tw_outcome(OBSERVED);
    let events = observe::drain();
    assert_eq!(out.error, None, "clean run: {:?}", out.error);

    let messages = events
        .iter()
        .filter(|e| matches!(e.kind, observe::EventKind::Message(_)))
        .count();
    let enters = events
        .iter()
        .filter(|e| matches!(e.kind, observe::EventKind::SpanEnter))
        .count();
    let exits = events
        .iter()
        .filter(|e| matches!(e.kind, observe::EventKind::SpanExit))
        .count();

    assert_eq!(
        messages, 6,
        "the tree-walker emits every executed observation site"
    );
    assert_eq!(
        enters, 5,
        "one span enter per executed During (outer + 4 inner)"
    );
    assert_eq!(
        exits, enters,
        "spans are balanced — every enter has its exit"
    );
}

/// The register VM currently ELIDES the observation channel — it emits no events
/// — yet it runs the span bodies and every observed statement's real work in
/// full. This documents the VM's floor as "elides, does not diverge": promoting
/// this to `vm_emits_the_full_event_stream` is how the floor rises, and this
/// lock guards that until then the VM must never silently drop program work.
#[test]
fn vm_elides_the_channel_without_dropping_work() {
    observe::install();
    let out = vm_outcome(OBSERVED);
    let events = observe::drain();
    assert_eq!(out.error, None, "clean VM run: {:?}", out.error);

    // The floor: the VM does not yet emit. (Raising this to a non-empty
    // assertion is the ratchet climbing — a deliberate, reviewed promotion.)
    assert_eq!(
        events.len(),
        0,
        "VM emission floor: currently elided (ratchet: only rises)"
    );

    // But the work behind the observation — the loop inside the spans — ran to
    // completion, identical to the tree-walker (mod the trailing-newline
    // convention).
    assert_eq!(
        out.output.trim_end(),
        "10",
        "the VM did all the real work the spans wrapped"
    );
    assert_eq!(
        out.output.trim_end(),
        tw_outcome(OBSERVED).output.trim_end(),
        "and agrees with the oracle tier"
    );
}
