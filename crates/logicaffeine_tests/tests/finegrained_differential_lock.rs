//! ★ LOCK — the two reactivity engines answer the same, over programs nobody wrote by hand. ★
//!
//! There are TWO implementations of what a turn does. The tree-walker tracks dependencies
//! DYNAMICALLY: it renders, watches what each site read, and after an action re-reads whatever
//! moved. The bytecode tier decides STATICALLY: `lgx::deps` computes each site's read set and each
//! action's write set at compile time and emits a patcher. They are different algorithms reaching
//! the same answer, and that is exactly the situation where a hand-written test suite runs out —
//! the cases a person thinks to write are the cases both implementations were written against.
//!
//! So this generates them. A random component — state fields, actions that move them, a view of
//! holes, `{if}`s and `{for each}`s over those fields — then a random sequence of clicks, and the
//! two tiers must render the same document at every step.
//!
//! ★ WHAT A DIVERGENCE WOULD MEAN, and why it is worth generating for: the static engine's read set
//! is CLOSED over derived fields, loop variables and `{let}` bindings, and its write set is a
//! deliberate over-approximation. Every way of getting that wrong is silent — the page simply stops
//! updating, or updates something it should not have — and the failure needs a state field, an
//! action and a site that reads it to line up in one program. That is a shape a generator finds and
//! a person does not.
//!
//! ★ THE LIGHT CORPUS RUNS EVERY TIME. A fuzzer only in the `#[ignore]`d lane is a fuzzer that runs
//! when someone remembers; the small seeded sweep here is cheap enough to be part of the ordinary
//! suite, and the heavy one widens it rather than being the only run.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// xorshift + a multiply — the same small deterministic generator the template fuzzer uses, so a
/// failing case is reproducible from its seed alone.
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

/// One generated program: the page source, and the ids its buttons carry.
struct Page {
    src: String,
    buttons: Vec<String>,
}

/// Build a random component whose state, actions and view are wired to each other.
///
/// The shapes are deliberately the ones the static engine has to REASON about rather than merely
/// render: a hole reading a field directly, a derived field reading another field (so the read set
/// has to close over it), an `{if}` whose condition reads a field (a structural site), and a
/// `{for each}` over a list field (whose loop variable is a window onto that field, not state of
/// its own). An action moves one or more fields, so which sites must update is a real question.
fn gen_page(rng: &mut Rng) -> Page {
    let n_fields = 1 + rng.below(3);
    let mut state = String::new();
    let mut view = String::from("    <main>\n");
    let mut actions = String::new();
    let mut buttons = Vec::new();

    // Scalar fields, each with a hole that shows it.
    for i in 0..n_fields {
        state.push_str(&format!("    Let n{i} be {}.\n", rng.below(5)));
        view.push_str(&format!("      <p id=\"p{i}\">{{n{i}}}</p>\n"));
    }
    // A derived field over the first — the read set must close through it.
    state.push_str("    Let doubled always be n0 * 2.\n");
    view.push_str("      <p id=\"d\">{doubled}</p>\n");

    // A Text field and a list field, so a hole, a condition and a loop all have something to read.
    state.push_str("    Let label be \"start\".\n");
    state.push_str("    Let items be [1, 2].\n");
    view.push_str("      <p id=\"l\">{label}</p>\n");
    if rng.below(2) == 0 {
        // A STRUCTURAL site: the engine must repaint rather than patch when this flips.
        view.push_str("      {if n0 is greater than 1:}<em id=\"big\">big</em>{else:}<em id=\"small\">small</em>{end}\n");
    }
    view.push_str("      <ul>{for each it in items:}<li>{it}</li>{else:}<li>none</li>{end}</ul>\n");

    // Actions: each moves a field or two, and one is wired to a button.
    for i in 0..n_fields {
        let id = format!("b{i}");
        let body = match rng.below(4) {
            0 => format!("        Set n{i} to n{i} + 1.\n"),
            1 => format!("        Set label to \"moved\".\n"),
            2 => format!("        Set n{i} to n{i} + 1.\n        Set label to \"both\".\n"),
            _ => format!("        Push {} to items.\n", 1 + rng.below(9)),
        };
        actions.push_str(&format!("    To act{i}:\n{body}\n"));
        view.push_str(&format!(
            "      <button id=\"{id}\" on-click={{act{i}}}>go</button>\n"
        ));
        buttons.push(id);
    }
    view.push_str("    </main>\n");

    Page {
        src: format!("## Component App\n\nState:\n{state}\nActions:\n{actions}View:\n{view}"),
        buttons,
    }
}

/// Mount the page and click the given buttons in order.
///
/// ★ THE DOCUMENT IS THE ANSWER, not printed output. A page query (`the text of "#d"`) is a
/// reference-tier form the VM does not run, so asking that way would compare one tier against a
/// refusal. Both tiers paint into the SAME recorder, so the rendered tree is the thing to compare —
/// and it is also the thing a browser would show, which printed output is not.
fn program(page: &Page, clicks: &[&str]) -> String {
    let mut main = String::from("## Main\nMount App.\n");
    for c in clicks {
        main.push_str(&format!("Click \"#{c}\".\n"));
    }
    format!("{}\n{main}", page.src)
}

/// The document both tiers render, held equal. `None` when a page is legitimately refused on BOTH
/// tiers — a generated program may name a shape the bytecode tier does not lower, and matching
/// refusals are agreement, not a gap.
fn agree(src: &str) -> Option<String> {
    dom::reset();
    let tw = tw_outcome(src);
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_doc = dom::render();
    match (&tw.error, &vm.error) {
        (Some(_), Some(_)) => None,
        (a, b) => {
            assert_eq!(
                a.is_some(),
                b.is_some(),
                "one tier refused and the other did not — that is a divergence, not a limit:\n\
                 tree-walker: {a:?}\nVM: {b:?}\n{src}"
            );
            assert_eq!(
                tw_doc, vm_doc,
                "the two reactivity engines rendered DIFFERENT documents:\n{src}"
            );
            Some(tw_doc)
        }
    }
}

/// One seeded sweep: generate pages, click random buttons, hold the tiers equal.
fn sweep(seed: u64, pages: usize, clicks_per_page: usize) -> usize {
    let mut rng = Rng(seed);
    let mut compared = 0;
    for _ in 0..pages {
        let page = gen_page(&mut rng);
        if page.buttons.is_empty() {
            continue;
        }
        let mut clicks: Vec<&str> = Vec::new();
        for _ in 0..clicks_per_page {
            let b = &page.buttons[rng.below(page.buttons.len())];
            clicks.push(b.as_str());
            if agree(&program(&page, &clicks)).is_some() {
                compared += 1;
            }
        }
    }
    compared
}

#[test]
fn the_two_engines_agree_over_a_light_generated_sweep() {
    // ★ ALWAYS ON. Small enough for the ordinary suite, wide enough to cross state × action × site
    // combinations no hand-written case enumerates.
    let compared = sweep(0x5DEE_CE66_D1B5_4A32, 24, 3);
    assert!(
        compared > 40,
        "the sweep must actually compare pages, got {compared}"
    );
}

#[test]
fn a_turn_that_moves_nothing_a_site_shows_still_agrees() {
    // ★ THE CASE THE STATIC ENGINE IS MOST LIKELY TO GET WRONG, pinned by hand beside the sweep: an
    // action writes a field NO site displays. The dynamic tier sees nothing change; the static tier
    // must reach the same conclusion from its write set rather than repainting or, worse, patching
    // a site that reads something else.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let shown be 1.\n\
      \x20   Let hidden be 0.\n\
       \n\
       Actions:\n\
      \x20   To bump:\n\
      \x20       Set hidden to hidden + 1.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"s\">{shown}</p><button id=\"b\" on-click={bump}>go</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#b\".\n";
    let doc = agree(src).expect("both tiers run this");
    assert!(
        doc.contains(">1<"),
        "the shown field must still read 1: {doc}"
    );
}

#[test]
fn a_derived_field_updates_on_both_tiers_when_its_source_moves() {
    // The read set has to CLOSE over the derivation: nothing ever writes `doubled`, so a site
    // showing it must be updated when `n` moves. Getting this wrong makes the page silently stale.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 1.\n\
      \x20   Let doubled always be n * 2.\n\
       \n\
       Actions:\n\
      \x20   To bump:\n\
      \x20       Set n to n + 1.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"d\">{doubled}</p><button id=\"b\" on-click={bump}>go</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#b\".\n";
    let doc = agree(src).expect("both tiers run this");
    assert!(
        doc.contains(">4<"),
        "the derived field must have followed its source: {doc}"
    );
}

#[test]
fn a_structural_change_agrees_on_both_tiers() {
    // An `{if}` flipping is a MOUNT/UNMOUNT, not a value write — the static engine repaints for it
    // rather than patching, and the two tiers must still land on the same document.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 1.\n\
       \n\
       Actions:\n\
      \x20   To bump:\n\
      \x20       Set n to n + 1.\n\
       \n\
       View:\n\
      \x20   <main>{if n is greater than 1:}<em id=\"big\">big</em>{else:}<em id=\"small\">small</em>{end}\
       <button id=\"b\" on-click={bump}>go</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n\
       Click \"#b\".\n";
    let doc = agree(src).expect("both tiers run this");
    assert!(
        doc.contains("big") && !doc.contains("small"),
        "the arm must have flipped: {doc}"
    );
}

#[test]
#[ignore = "seeded generator heavy — run by the full suite"]
fn the_two_engines_agree_over_a_wide_generated_sweep() {
    // The same walk, four seeds and far more pages — the lane where a rare state × action × site
    // alignment has room to show up.
    let mut total = 0;
    for seed in [
        0x9E37_79B9_7F4A_7C15,
        0xD1B5_4A32_D192_ED03,
        0x2545_F491_4F6C_DD1D,
        0xBF58_476D_1CE4_E5B9,
    ] {
        total += sweep(seed, 150, 4);
    }
    assert!(
        total > 1_500,
        "the wide sweep must compare a lot of pages, got {total}"
    );
}
