//! ★ LOCK — a trapping turn never leaves half its writes behind, over programs nobody wrote. ★
//!
//! A turn is a TRANSACTION: an action runs to completion or the instance's state goes back to what
//! it was when the turn started. `turn_transaction_lock` proves that on hand-written cases; this
//! generates them, because the failure mode is positional. A rollback that restores the fields an
//! action wrote BEFORE the trap but forgets one it wrote after a branch, or restores a scalar and
//! not a list, or works on the reference tier and not the compiled one, is a page that looks fine
//! and holds a value nobody wrote.
//!
//! ★ THE GENERATOR VARIES WHERE THE TRAP SITS, which is the whole point: before any write, between
//! two writes, inside an `If`, after a list push. A hand-written case fixes one position; the bug
//! lives in the ones you did not think to fix.
//!
//! ★ AND IT CHECKS BOTH DIRECTIONS. A rollback that reverted EVERYTHING — including turns that did
//! not trap — would pass every "state is unchanged after a trap" assertion ever written. So each
//! generated program is run twice, once with the trap armed and once disarmed, and the disarmed run
//! must COMMIT exactly what the armed one discarded.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

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

/// The sentence that traps: reading past the end of a one-element list. It is a RUNTIME failure
/// (the index is only known when it runs), which is what makes it a turn failure rather than a
/// compile error.
const TRAP: &str = "        Set boom to item 9 of [1].\n";

/// Build a component whose action writes some fields and — when `armed` — traps partway through.
///
/// The same program is generated both ways from the same seed, so the two runs differ in exactly
/// one statement and the comparison between them is meaningful.
fn gen(rng: &mut Rng, armed: bool) -> String {
    let writes = 1 + rng.below(3);
    let trap_at = rng.below(writes + 1);

    let mut state =
        String::from("    Let boom be 0.\n    Let note be \"\".\n    Let items be [7].\n");
    for i in 0..writes {
        state.push_str(&format!("    Let f{i} be 0.\n"));
    }

    let mut body = String::new();
    for i in 0..writes {
        if i == trap_at && armed {
            body.push_str(TRAP);
        }
        // A mix of write shapes: a scalar, a Text, and a list push — a rollback that restores only
        // scalars is a rollback that loses a row.
        match rng.below(3) {
            0 => body.push_str(&format!("        Set f{i} to {}.\n", 1 + i)),
            1 => body.push_str(&format!("        Set note to \"w{i}\".\n")),
            _ => body.push_str(&format!("        Push {} to items.\n", 1 + i)),
        }
        // Sometimes the write sits inside a branch that really runs — a rollback keyed on
        // statement position rather than on what was written would miss it.
        if rng.below(3) == 0 {
            body.push_str(&format!(
                "        If f{i} is at least 0:\n            Set f{i} to f{i} + 1.\n"
            ));
        }
    }
    if armed && trap_at == writes {
        body.push_str(TRAP);
    }

    let mut view = String::from("    <main>");
    for i in 0..writes {
        view.push_str(&format!("<p id=\"f{i}\">{{f{i}}}</p>"));
    }
    view.push_str("<p id=\"n\">{note}</p><p id=\"c\">{length of items}</p>");
    view.push_str("<button id=\"go\" on-click={act}>x</button></main>\n");

    format!(
        "## Component App\n\nState:\n{state}\nActions:\n    To act:\n{body}\nView:\n{view}\n## Main\nMount App.\nClick \"#go\".\n"
    )
}

/// The document a tier renders, or `None` if that tier refuses the program.
fn render(src: &str, vm: bool) -> Option<String> {
    dom::reset();
    let out = if vm { vm_outcome(src) } else { tw_outcome(src) };
    out.error.is_none().then(dom::render)
}

/// The document after a run that is EXPECTED to report a failed turn, plus whether it did.
///
/// ★ A REPORTED FAILURE IS NOT A REFUSAL, and [`render`] cannot tell them apart — it answers
/// `None` for both. Once the tree-walker began REPORTING an unhandled action failure instead of
/// dropping it, every armed program in this sweep started answering `None`, `tw_rollbacks` stopped
/// incrementing, and the whole tree-walker half of the fuzz vanished while the test still read as
/// a fuzz. That is the failure mode this file's own header warns about one paragraph down.
///
/// It is safe to ignore the error here for one structural reason, which is the sweep's founding
/// premise: the armed and disarmed programs come from ONE draw and differ ONLY in the trap. The
/// caller checks the disarmed twin ran clean, so any error the armed twin reports IS the trap.
fn render_after_trap(src: &str, vm: bool) -> (String, bool) {
    dom::reset();
    let out = if vm { vm_outcome(src) } else { tw_outcome(src) };
    (dom::render(), out.error.is_some())
}

/// The document BEFORE any click — the state a rollback must restore to.
fn untouched(src: &str, vm: bool) -> Option<String> {
    render(&src.replace("Click \"#go\".\n", ""), vm)
}

/// What one seeded sweep actually checked, PER TIER.
///
/// ★ COUNTED SEPARATELY BECAUSE A SKIPPING TIER MUST NOT HIDE. My first version skipped any tier
/// that refused a program and counted one total — and since the bytecode tier refuses EVERY armed
/// program (see `sweep`), the whole fuzz was silently a tree-walker fuzz that still reported a
/// healthy number. A fuzz that can pass with a tier entirely absent is a fuzz that stops covering
/// it the day it breaks.
struct Checked {
    tw_rollbacks: usize,
    vm_commits: usize,
}

/// One seeded sweep, holding each tier to what it actually promises.
///
/// The reference tier CATCHES a turn's trap: state rolls back and the page stays alive, so both
/// halves are checkable there. The bytecode tier's trap is a wasm trap — unrecoverable inside the
/// module — so an armed program ends the run. What is still checkable there, and is checked, is
/// that a turn which does NOT trap commits exactly what the reference tier commits.
fn sweep(seed: u64, cases: usize) -> Checked {
    let mut rng = Rng(seed);
    let mut checked = Checked {
        tw_rollbacks: 0,
        vm_commits: 0,
    };
    for _ in 0..cases {
        // Two programs from ONE draw, so they differ only in the trap.
        let mut a = Rng(rng.next());
        let mut b = Rng(a.0);
        let armed = gen(&mut a, true);
        let disarmed = gen(&mut b, false);

        // ── The reference tier: a trap rolls the turn back and the program survives. ──
        if let (Some(before), Some(after_clean)) =
            (untouched(&armed, false), render(&disarmed, false))
        {
            let (after_trap, reported) = render_after_trap(&armed, false);
            assert_eq!(
                after_trap, before,
                "a TRAPPING turn left state behind — a turn is all-or-nothing:\n{armed}"
            );
            // ★ AND IT SAID SO. An unhandled action failure is reported rather than dropped, so a
            // page whose button silently does nothing cannot pass this sweep. Rolling back and
            // going quiet was the old contract and it hid real defects; both halves are checked
            // here so neither can regress alone.
            assert!(
                reported,
                "a TRAPPING turn rolled back but reported nothing — a silently dead button:\n{armed}"
            );
            // The teeth: the same program WITHOUT the trap really does change the page, so
            // "unchanged" cannot pass by the turn having done nothing.
            assert_ne!(
                after_clean, before,
                "the DISARMED turn changed nothing, so the assertion above proves nothing:\n{disarmed}"
            );
            checked.tw_rollbacks += 1;
        }

        // ── The bytecode tier: a clean turn commits the same document. ──
        if let (Some(tw_clean), Some(vm_clean), Some(vm_before)) = (
            render(&disarmed, false),
            render(&disarmed, true),
            untouched(&disarmed, true),
        ) {
            assert_eq!(
                tw_clean, vm_clean,
                "a clean turn committed DIFFERENT documents on the two tiers:\n{disarmed}"
            );
            assert_ne!(
                vm_clean, vm_before,
                "the clean turn must change the page:\n{disarmed}"
            );
            checked.vm_commits += 1;
        }
    }
    checked
}

#[test]
fn a_trapping_turn_commits_nothing_over_a_light_sweep() {
    // ★ ALWAYS ON, and it carries its own teeth twice over: every case proves the same program
    // WITHOUT the trap really does change the page (so "unchanged" cannot pass by the turn doing
    // nothing), and each tier's count is asserted separately (so neither can quietly stop being
    // covered).
    let c = sweep(0xA076_1D64_78BD_642F, 30);
    assert!(
        c.tw_rollbacks > 20,
        "the rollback sweep must run, got {}",
        c.tw_rollbacks
    );
    assert!(
        c.vm_commits > 20,
        "the bytecode-tier commit sweep must run, got {}",
        c.vm_commits
    );
}

/// ★ THE CONTRACT (a language decision): an unhandled action
/// failure is REPORTED as the program's own error AND the page keeps answering, with the failed
/// turn's own writes rolled back — on every tier. Not "the trap is invisible" (an author who wrote
/// no `When an action fails:` still needs to be told something broke) and not "the trap is fatal"
/// (a page one bad turn can never recover from is not a page): both halves, together, on the
/// reference tier and the bytecode tier alike.
///
/// Proves it two ways: right after the trapping click alone (the turn's own write, `Set n to 5.`,
/// never reached the record — `n` reads back as it started, and the error names the trap by
/// message), and after a second, good click on the same page (the click after the trap still runs
/// and commits, `n` reads 3 — a dead page would never reach this line at all).
#[test]
fn the_page_still_answers_after_a_trapping_turn() {
    let component = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 0.\n\
      \x20   Let boom be 0.\n\
       \n\
       Actions:\n\
      \x20   To bad:\n\
      \x20       Set n to 5.\n\
      \x20       Set boom to item 9 of [1].\n\
       \n\
      \x20   To good:\n\
      \x20       Set n to 3.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"n\">{n}</p><button id=\"b\" on-click={bad}>x</button>\
       <button id=\"g\" on-click={good}>y</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#b\".\n";
    let after_good_too = format!("{component}Click \"#g\".\n");

    for vm in [false, true] {
        let run = |s: &str| if vm { vm_outcome(s) } else { tw_outcome(s) };

        dom::reset();
        let trapped = run(component);
        assert_eq!(
            trapped.error.as_deref(),
            Some("Index 9 out of bounds"),
            "tier vm={vm} must report the trap by name"
        );
        assert!(
            dom::render().contains(">0<"),
            "tier vm={vm} the trapping turn's own write must be rolled back: {}",
            dom::render()
        );

        dom::reset();
        let recovered = run(&after_good_too);
        assert_eq!(
            recovered.error.as_deref(),
            Some("Index 9 out of bounds"),
            "tier vm={vm} the trap stays the program's reported error even after a later turn commits"
        );
        assert!(
            dom::render().contains(">3<"),
            "tier vm={vm} the page must still answer after a trap: {}",
            dom::render()
        );
    }
}

#[test]
#[ignore = "seeded generator heavy — run by the full suite"]
fn a_trapping_turn_commits_nothing_over_a_wide_sweep() {
    let (mut rollbacks, mut commits) = (0, 0);
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0xD1B5_4A32_D192_ED03,
        0xBF58_476D_1CE4_E5B9,
    ] {
        let c = sweep(seed, 120);
        rollbacks += c.tw_rollbacks;
        commits += c.vm_commits;
    }
    assert!(
        rollbacks > 250,
        "the wide rollback sweep must run, got {rollbacks}"
    );
    assert!(
        commits > 250,
        "the wide bytecode-tier sweep must run, got {commits}"
    );
}
