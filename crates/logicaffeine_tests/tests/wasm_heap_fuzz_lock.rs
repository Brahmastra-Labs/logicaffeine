//! ★ LOCK — generated pages churning every counted shape agree with the tree-walker, reclaim on (S11). ★
//!
//! Each seed builds one page whose single action runs a generated sequence of statements over state
//! of every counted shape — a list of Int, a list of Text, a list of records with a list field, a map,
//! a Text built by appending — pushing, overwriting items (a Text item by a longer one), replacing a
//! list by a filtered copy, copying a list into a local and writing the copy, pushing through a
//! record's field, concatenating and appending Text. The page runs on the wasm backend with reclaim
//! on, a safepoint after every turn, and on the tree-walker. Three things must hold for every seed:
//! no turn traps, the census is exact after every safepoint (every counted value's stored holder count
//! equals the in-degree recomputed from the page's globals), and every turn's journal — each field's
//! value before and after, as Text — is the tree-walker's, byte for byte. A holder count one too low
//! frees a value still in use and the journal or the census shows it; one too high shows in the census.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::{dom, world};

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::mount_interactive_checked;

/// A deterministic generator (SplitMix64), so a failing seed reproduces exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One statement of the action body, indented for it; every statement is valid in any state.
fn statement(rng: &mut Rng) -> String {
    let k = rng.below(9) + 1;
    match rng.below(15) {
        0 => format!("Push {k} to xs."),
        1 => format!("Push \"t{k}\" to ts."),
        2 => format!("If length of xs is greater than 0:\n            Set item 1 of xs to {k}."),
        3 => format!(
            "Let mutable kept be a new Seq of Int.\n        Repeat for v in xs:\n            If v is greater than {k}:\n                Push v to kept.\n        Set xs to kept."
        ),
        4 => format!("Push a new Rec with n {k} and tags [\"r{k}\"] to rs."),
        5 => format!("If length of rs is greater than 0:\n            Push \"g{k}\" to (item 1 of rs)'s tags."),
        6 => format!("Set item \"k{k}\" of m to {k}."),
        7 => format!("Let mutable dup be xs.\n        Push {k} to dup.\n        Set n to length of dup."),
        8 => format!(
            "If length of ts is greater than 3:\n            Let mutable fresh be a new Seq of Text.\n            Push \"z{k}\" to fresh.\n            Set ts to fresh."
        ),
        9 => "If length of rs is greater than 2:\n            Set rs to a new Seq of Rec.".to_string(),
        10 => format!("Push \"p{k}\" + \"q\" to ts."),
        11 => format!("If length of ts is greater than 0:\n            Set item 1 of ts to (item 1 of ts) + \"{k}\"."),
        12 => format!("Set s to s + \"{k}\".\n        If length of s is greater than 12:\n            Set s to \"\"."),
        13 => format!("Set item \"w{k}\" of m to length of s."),
        _ => format!("If length of xs is greater than 4:\n            Set xs to [{k}]."),
    }
}

fn page(seed: u64) -> String {
    let mut rng = Rng(seed);
    let body: Vec<String> = (0..6 + rng.below(6))
        .map(|_| format!("        {}", statement(&mut rng)))
        .collect();
    format!(
        "## A Rec has:\n    a n, which is Int.\n    a tags, which is Seq of Text.\n\n\
         ## Component App\nState:\n    Let xs be a new Seq of Int.\n    Let ts be a new Seq of Text.\n    \
         Let rs be a new Seq of Rec.\n    Let m be a new Map of Text to Int.\n    Let n be 0.\n    Let s be \"\".\n\
         Actions:\n    To step:\n{}\n\
         View:\n    <div><button id=\"a\" on-click={{step}}>a</button></div>\n\n## Main\nMount App.\n",
        body.join("\n")
    )
}

const TURNS: usize = 12;
const SEEDS: u64 = 40;

/// Every field's value after each turn, as the tree-walker journals it.
fn reference(src: &str) -> Vec<Vec<(String, String)>> {
    let driven = format!("{src}{}", "Click \"#a\".\n".repeat(TURNS));
    dom::reset();
    world::reset();
    let out = tw_outcome(&driven);
    assert_eq!(out.error, None, "the tree-walker runs the page:\n{driven}");
    world::journal()
        .into_iter()
        .filter(|e| e.action == "step")
        .map(|e| {
            e.changes
                .into_iter()
                .map(|(field, _, after)| (field, after))
                .collect()
        })
        .collect()
}

#[test]
fn every_generated_page_agrees_with_the_tree_walker_and_keeps_an_exact_census() {
    let mut failures = Vec::new();
    for seed in 0..SEEDS {
        let src = page(seed);
        let want = reference(&src);
        let mut page = mount_interactive_checked(&src);
        page.safepoint();
        let button = *page
            .listening("click")
            .last()
            .expect("the page listens for a click");
        let mut got = Vec::new();
        let mut broke = None;
        for turn in 0..TURNS {
            if let Err(e) = page.dispatch(button, "click", "") {
                broke = Some(format!("turn {turn} traps: {e}"));
                break;
            }
            page.safepoint();
            let census = page.census();
            if census != 0 {
                broke = Some(format!(
                    "turn {turn}: the census disagrees at block {census}"
                ));
                break;
            }
        }
        for call in page.journal().into_iter().filter(|c| c.name == "step") {
            got.push(
                call.changes
                    .chunks(3)
                    .map(|t| (t[0].clone(), t[2].clone()))
                    .collect::<Vec<_>>(),
            );
        }
        if broke.is_some() || got != want {
            failures.push(format!(
                "── seed {seed}: {}\n  wasm {got:?}\n  tree-walker {want:?}\n{src}",
                broke.unwrap_or_else(|| "journals differ".to_string())
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {SEEDS} seeds fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
