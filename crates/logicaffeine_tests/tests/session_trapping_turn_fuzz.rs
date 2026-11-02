//! ★ LOCK — a trapping turn does not take the SESSION down with it. ★
//!
//! `turn_rollback_fuzz`'s `the_page_still_answers_after_a_trapping_turn` is `#[ignore]`d because
//! `vm_outcome` compiles a `## Main`'s driving verbs INTO the program: the trap unwinds the whole
//! bytecode run and there is no Rust boundary to catch at. `vm::session::Session` is that boundary
//! — `invoke`/`call_value` return a `Result`, and an `Err` leaves the machine's globals, register
//! file and call stack exactly as a clean call would (`vm_session_lock`, `machine.rs`'s
//! `host_calls` unit tests). `page_host::LoweredPage` drives a `## Test` through that boundary
//! (`guarded`'s one door): a trap becomes a `DriveError::Failed`, and the NEXT driving statement
//! still runs.
//!
//! This sweep generates the same family of programs `turn_rollback_fuzz` does — a trap sitting
//! before, between or after a scalar/Text/list write, sometimes inside an `If` that really runs —
//! and drives each one through [`compare_tests`] on the `LoweredVm` tier: `Click "#bad"` traps,
//! `Click "#good"` must still run and its write must still show. THE GENERATOR VARIES WHERE THE
//! TRAP SITS, which is the point — a hand-written case fixes one position; the recovery bug (if
//! there is one) lives in a position nobody thought to fix by hand.
//!
//! Every axis but `journal` (`noteTurn` is not yet emitted, R3) must
//! agree with the reference on every generated case: the session recovers, and it recovers
//! into the SAME page the tree-walker would show.

use logicaffeine_compile::page_host::{compare_tests, Axis, PageTierKind, TierRun};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The sentence that traps: reading past the end of a one-element list, at runtime.
const TRAP: &str = "        Set boom to item 9 of [1].\n";

/// A component whose `bad` action writes some fields and traps partway through, and whose `good`
/// action is a clean write a subsequent click must still commit.
fn gen(rng: &mut Rng) -> String {
    let writes = 1 + rng.below(3);
    let trap_at = rng.below(writes + 1);

    let mut state = String::from(
        "    Let boom be 0.\n    Let note be \"\".\n    Let items be [7].\n    Let ok be false.\n",
    );
    for i in 0..writes {
        state.push_str(&format!("    Let f{i} be 0.\n"));
    }

    let mut bad = String::new();
    for i in 0..writes {
        if i == trap_at {
            bad.push_str(TRAP);
        }
        // A mix of write shapes: a scalar, a Text, and a list push — a session that only rolls
        // back or only preserves scalars is not what this proves; it proves the NEXT turn runs.
        match rng.below(3) {
            0 => bad.push_str(&format!("        Set f{i} to {}.\n", 1 + i)),
            1 => bad.push_str(&format!("        Set note to \"w{i}\".\n")),
            _ => bad.push_str(&format!("        Push {} to items.\n", 1 + i)),
        }
        // Sometimes the write sits inside a branch that really runs.
        if rng.below(3) == 0 {
            bad.push_str(&format!(
                "        If f{i} is at least 0:\n            Set f{i} to f{i} + 1.\n"
            ));
        }
    }
    if trap_at == writes {
        bad.push_str(TRAP);
    }

    let mut view = String::from("    <main>");
    for i in 0..writes {
        view.push_str(&format!("<p id=\"f{i}\">{{f{i}}}</p>"));
    }
    view.push_str("<p id=\"n\">{note}</p><p id=\"c\">{length of items}</p><p id=\"ok\">{ok}</p>");
    view.push_str(
        "<button id=\"bad\" on-click={act_bad}>x</button>\
         <button id=\"good\" on-click={act_good}>y</button></main>\n",
    );

    format!(
        "## Component App\n\n\
         State:\n{state}\n\
         Actions:\n\
         \x20   To act_bad:\n{bad}\n\
         \x20   To act_good:\n\
         \x20       Set ok to true.\n\n\
         View:\n{view}\n\
         ## Test: the page still answers after a trap\n\
         \x20   Mount App.\n\
         \x20   Click \"#bad\".\n\
         \x20   Click \"#good\".\n\
         \x20   Expect that the text of \"#ok\" is equal to \"true\".\n"
    )
}

const BUDGET: u64 = 50_000_000;

/// One seeded sweep: every generated case must show the SAME recovery on the reference tier and
/// on the session-driven lowered tier.
fn sweep(seed: u64, cases: usize) {
    let mut rng = Rng(seed);
    let mut checked = 0;
    for _ in 0..cases {
        let src = gen(&mut rng);
        let suite_seed = rng.next();
        let comparisons = compare_tests(&src, &[PageTierKind::LoweredVm], suite_seed, BUDGET, None)
            .unwrap_or_else(|e| panic!("the generated program must parse:\n{src}\n=> {e}"));
        assert_eq!(comparisons.len(), 1, "exactly one page scenario:\n{src}");
        let c = &comparisons[0];

        // The generator's own premise: the reference tier really does recover and really does run
        // the good click — otherwise "the session agrees with the reference" proves nothing.
        let last_ref = c
            .reference
            .steps
            .last()
            .unwrap_or_else(|| panic!("a trace: {src}"));
        assert!(
            last_ref.snapshot.document.contains("id=\"ok\">true<"),
            "the reference tier itself did not recover from the trap — the generator's premise is \
             broken:\n{src}\ndocument: {}",
            last_ref.snapshot.document
        );

        match &c.candidates[0].1 {
            TierRun::Refused { statement, reason } => panic!(
                "the session-driven tier REFUSED a trapping-turn scenario instead of recovering \
                 from it, at statement {statement}: {reason}\n{src}"
            ),
            TierRun::Traced(t) => {
                let last = t.steps.last().unwrap_or_else(|| panic!("a trace: {src}"));
                assert!(
                    last.snapshot.document.contains("id=\"ok\">true<"),
                    "the session-driven tier did not recover after the trap — the click after it \
                     never ran:\n{src}\ndocument: {}",
                    last.snapshot.document
                );
                let divergence = c.reference.compare(t);
                for a in &divergence.axes {
                    assert_eq!(
                        a.axis,
                        Axis::Journal,
                        "the trap left the session and the reference disagreeing on {} after \
                         statement {}:\nreference: {:?}\ncandidate: {:?}\n{src}",
                        a.axis.name(),
                        a.statement,
                        a.reference,
                        a.candidate
                    );
                }
            }
        }
        checked += 1;
    }
    assert_eq!(checked, cases);
}

#[test]
fn a_trapping_turn_recovers_on_the_session_tier_over_a_light_sweep() {
    sweep(0x5EED_1054_A55E_7731, 25);
}

#[test]
#[ignore = "seeded generator heavy — run by the full suite"]
fn a_trapping_turn_recovers_on_the_session_tier_over_a_wide_sweep() {
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0xD1B5_4A32_D192_ED03,
        0xBF58_476D_1CE4_E5B9,
    ] {
        sweep(seed, 120);
    }
}
