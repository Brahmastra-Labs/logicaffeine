//! Phase 11 (work/FINISH_INTERPRETER.md) — translation validation for concurrency.
//!
//! The TV encoder (`symexec`) must encode the *determinate* concurrency fragment — channels
//! and task spawn — so the meta-soundness check (`check_encoder_sound`) proves the encoder
//! agrees with the tree-walking interpreter on concurrent programs, not just sequential
//! ones. Determinate programs have schedule-independent output (Kahn), so a single modeled
//! schedule is canonical. Nondeterministic constructs (`Select`/`Try*`) stay honestly
//! `Unsupported` until the seeded-replay path (next increment).

use logicaffeine_tv::{check_encoder_sound, SoundnessReport};

#[test]
fn tv_determinate_producer_consumer_agrees() {
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive first from jobs.\n\
        \x20   Receive second from jobs.\n\
        \x20   Show first.\n\
        \x20   Show second.\n";
    assert_eq!(
        check_encoder_sound(src),
        SoundnessReport::Agrees,
        "the determinate producer/consumer must be encoded and proven equivalent"
    );
}

#[test]
fn tv_select_seeded_replay_agrees() {
    // A `Select` with one ready receive: the receive wins (no draw), so the seeded encoder
    // matches the seeded interpreter at every seed.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Send 1 into ch.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show 0.\n";
    assert_eq!(
        check_encoder_sound(src),
        SoundnessReport::SeedReplayAgrees,
        "a Select with a ready receive must validate by seeded replay"
    );
}

#[test]
fn tv_select_two_ready_arms_seeded_replay_agrees() {
    // Both channels hold a value → both receive arms are ready → the winner is `below(2)`
    // drawn from the SplitMix64 the scheduler uses. The seeded encoder must pick the SAME
    // winner as the interpreter at every seed (the genuine seeded-choice alignment).
    let src = "## Main\n\
        \x20   Let a be a Pipe of Int.\n\
        \x20   Let b be a Pipe of Int.\n\
        \x20   Send 10 into a.\n\
        \x20   Send 20 into b.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from a:\n\
        \x20           Show x.\n\
        \x20       Receive y from b:\n\
        \x20           Show y.\n";
    assert_eq!(
        check_encoder_sound(src),
        SoundnessReport::SeedReplayAgrees,
        "the seeded encoder must pick the same Select winner as the interpreter at every seed"
    );
}

#[test]
fn tv_seed_sweep_refinement() {
    // Refinement: the COMPILED/encoded outcome-set across the seed sweep ⊆ the interpreter's
    // allowed-set. This program is GENUINELY nondeterministic — the two-ready-arms `Select`
    // winner is drawn from the scheduler's SplitMix64, so across seeds the interpreter
    // produces BOTH "10" and "20". The encoder, pinned to the same draw, agrees at EVERY seed
    // ⇒ it never emits an outcome the interpreter couldn't (here the sets are equal). The
    // per-seed exact agreement is *stronger* than mere set-inclusion, so `SeedReplayAgrees`
    // on a proven-multi-outcome program IS the refinement guarantee — no weaker verdict needed.
    let src = "## Main\n\
        \x20   Let a be a Pipe of Int.\n\
        \x20   Let b be a Pipe of Int.\n\
        \x20   Send 10 into a.\n\
        \x20   Send 20 into b.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from a:\n\
        \x20           Show x.\n\
        \x20       Receive y from b:\n\
        \x20           Show y.\n";

    // (1) The program is genuinely multi-outcome — the interpreter yields ≥2 distinct results
    //     over the seed sweep, so this is a real refinement scenario, not a disguised constant.
    let mut outcomes = std::collections::BTreeSet::new();
    for seed in [0u64, 1, 2, 7, 42] {
        let run = logicaffeine_compile::run_treewalker_concurrent_seeded(src, seed);
        outcomes.insert(run.lines.join("\n"));
    }
    assert!(
        outcomes.len() >= 2,
        "the two-ready-arms Select must be genuinely nondeterministic across seeds; got {outcomes:?}"
    );

    // (2) The encoder agrees at every seed ⇒ its outcome-set refines (equals) the interpreter's
    //     allowed-set — no compiled outcome lies outside what the interpreter admits.
    assert_eq!(
        check_encoder_sound(src),
        SoundnessReport::SeedReplayAgrees,
        "the encoder's seeded outcomes must refine the interpreter's allowed-set"
    );
}

#[test]
fn tv_error_agreement_division_by_zero() {
    // The error-AGREEMENT path (untested until now): the interpreter RAISES (division by
    // zero), so the validator must prove the encoder *also* errors — it must NOT rubber-stamp
    // `Agrees` on a program that errors, nor falsely `Disagrees`. Edge case: a value the
    // semantics rejects.
    let src = "## Main\n   Let x be 1 / 0.\n   Show x.\n";
    let report = check_encoder_sound(src);
    assert!(
        matches!(report, SoundnessReport::Agrees),
        "the encoder must prove the interpreter's div-by-zero error (error agreement), got {report:?}"
    );
}

#[test]
fn tv_try_marked_unsupported() {
    // A non-blocking `Try to receive` is outside the modeled fragment. The encoder must
    // honestly report `Unsupported` — NEVER a false `Agrees`, which would "prove" a program
    // it never actually modeled (the most dangerous failure mode for a validator).
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show 0.\n";
    assert!(
        matches!(check_encoder_sound(src), SoundnessReport::Unsupported { .. }),
        "a `Try` program must be honestly Unsupported, got {:?}",
        check_encoder_sound(src)
    );
}

#[test]
fn tv_try_send_marked_unsupported() {
    // Edge case: the non-blocking `Try to send` twin is equally outside the fragment.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to send 7 into ch.\n\
        \x20   Show 0.\n";
    assert!(
        matches!(check_encoder_sound(src), SoundnessReport::Unsupported { .. }),
        "a `Try to send` program must be honestly Unsupported, got {:?}",
        check_encoder_sound(src)
    );
}
