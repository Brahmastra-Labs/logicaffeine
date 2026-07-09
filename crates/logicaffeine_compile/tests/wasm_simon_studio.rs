//! Reproduction harness for the Studio "Simon" freeze (browser / wasm32).
//!
//! The Studio loads a `.logic` example by calling `compile_theorem_for_ui` SYNCHRONOUSLY
//! on the wasm main thread (see `apps/logicaffeine_web/src/ui/pages/studio.rs`). On native
//! that solve is wrapped in a 512 MiB-stack worker thread; on wasm there are no threads, so
//! it runs inline on the module's link-time stack (wasm-ld default ~1 MiB — the same stack
//! the deployed Studio uses). This test runs the EXACT shipped Simon document through that
//! same call under node's V8, so a stack blow-up here is the browser freeze, reproduced
//! without a browser.
//!
//! `wasm_small_theorem_compiles` is the control: the logic path itself works on wasm.
//! `wasm_full_simon_compiles` is the suspect: the full 4-category / 6-clue PuzzleBaron grid.
//!
//! Only builds on `wasm32`; inert on a normal `cargo test`.

#![cfg(target_arch = "wasm32")]

use logicaffeine_compile::compile_theorem_for_ui;
use wasm_bindgen_test::*;

/// The exact Logic-mode "Simon" example shipped by the Studio
/// (`apps/logicaffeine_web/src/ui/examples.rs` :: `LOGIC_SIMON`).
const LOGIC_SIMON: &str = r#"## Theorem: Simon
Given: Alpha, Beta, Gamma, and Delta are four different trips.
Given: 2001, 2002, 2003, and 2004 are four different years.
Given: Connecticut, Florida, Kentucky, and Maine are four different states.
Given: Bill, Lillie, Neal, and Yvonne are four different friends.
Given: Cycling, hunting, kayaking, and skydiving are four different activities.
Given: Every trip is in 2001 or in 2002 or in 2003 or in 2004.
Given: Exactly one trip is in 2001.
Given: Exactly one trip is in 2002.
Given: Exactly one trip is in 2003.
Given: Exactly one trip is in 2004.
Given: Every trip is in Connecticut or in Florida or in Kentucky or in Maine.
Given: Exactly one trip is in Connecticut.
Given: Exactly one trip is in Florida.
Given: Exactly one trip is in Kentucky.
Given: Exactly one trip is in Maine.
Given: Every trip is with Bill or with Lillie or with Neal or with Yvonne.
Given: Exactly one trip is with Bill.
Given: Exactly one trip is with Lillie.
Given: Exactly one trip is with Neal.
Given: Exactly one trip is with Yvonne.
Given: Every trip is cycling or hunting or kayaking or skydiving.
Given: Exactly one trip is cycling.
Given: Exactly one trip is hunting.
Given: Exactly one trip is kayaking.
Given: Exactly one trip is skydiving.
Given: Alpha is in 2001.
Given: Beta is in 2002.
Given: Gamma is in 2003.
Given: Delta is in 2004.
Given: Of the hunting trip and the 2004 trip, one was with Neal and the other was in Connecticut.
Given: The Florida trip was the hunting trip.
Given: Neither the trip with Bill nor the Florida trip is the 2001 trip.
Given: The trip with Yvonne is not in Kentucky.
Given: Of the skydiving trip and the Maine trip, one was in 2003 and the other was with Bill.
Given: The 2003 trip is not the cycling trip.
Prove: Beta is in Florida.
Proof: Auto.
"#;

/// Control: a small finite syllogism compiles through the Studio theorem path on wasm.
#[wasm_bindgen_test]
fn wasm_small_theorem_compiles() {
    let src = "## Theorem: Socrates\n\
        Given: All men are mortal.\n\
        Given: Socrates is a man.\n\
        Prove: Socrates is mortal.\n\
        Proof: Auto.\n";
    let r = compile_theorem_for_ui(src);
    assert!(r.error.is_none(), "small theorem errored: {:?}", r.error);
    assert!(r.verified, "small theorem must verify on wasm");
}

/// Suspect: the full Simon grid through the exact Studio path. If the certified-derivation
/// recursion overruns the wasm stack, this test traps — the browser freeze, reproduced.
#[wasm_bindgen_test]
fn wasm_full_simon_compiles() {
    let r = compile_theorem_for_ui(LOGIC_SIMON);
    assert!(r.error.is_none(), "simon parse/compile errored: {:?}", r.error);
    assert!(
        r.verified,
        "full Simon must verify on wasm; verification_error: {:?}",
        r.verification_error
    );
}
