//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A BRANCHY `## Main` REACHES THE SPATIAL FABRIC — and shows what the VM shows.
//!
//! The fabric has always lowered control flow; `fabric_sequencer_lock` runs branchy FUNCTIONS on it.
//! What it could not do was lower the top-level `## Main`, which is how almost every LOGOS program is
//! actually written. Two things stood in the way and neither was a missing tile:
//!
//!   * the combinational output-tape path refuses a Main with a branch outright — "a conditional Show
//!     cannot ride the combinational output tape" — and that is correct, a tape is straight-line;
//!   * the CFG path, which DOES handle branches, is addressed by FUNCTION NAME, and the top level is
//!     not a named function, so it could not be asked for.
//!
//! So a third of the ISA was unreachable on the fabric for branchy programs because of ROUTING. With
//! `try_main_cfg_fabric` the region is addressable, and with a per-block output tape a Main can show a
//! DIFFERENT sequence down each path — which is the thing a straight-line tape cannot represent and the
//! whole reason the refusal existed.
//!
//! ★ THE OBLIGATION IS THE STREAM, NOT THE VALUE. A function is judged by what it returns; a `## Main`
//! is judged by what it PRINTS, in order. Every program below is checked against the VM's own output
//! sequence, so a lowering that returns the right final register while printing the wrong things — or
//! the right things in the wrong order, or the right things down the wrong branch — fails here.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_branchy_main_lock`
//!
//! ★ MUTATION PROBE: drop the `shows.push(v)` in the `Op::Show` arm — every program still lowers and
//! still returns, and every case here reds on an empty stream. Push the show AFTER the block's writes
//! commit instead of before — `loop_accumulate` reds, because the value shown would be the one the
//! block left behind rather than the one it computed.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_program_cfg_fabric_traced, try_main_cfg_fabric,
};

/// `(label, source, the output the VM produces)`.
const PROGRAMS: &[(&str, &str, &[i64])] = &[
    ("if_then", "## Main\nLet x be 7.\nIf x is greater than 5:\n    Show 1.\nShow x.\n", &[1, 7]),
    ("if_else_taken", "## Main\nLet x be 2.\nIf x is greater than 5:\n    Show 100.\nShow x.\n", &[2]),
    (
        "loop_accumulate",
        "## Main\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most 5:\n    Set acc to acc + i.\n    Set i to i + 1.\nShow acc.\n",
        &[15],
    ),
    (
        "show_inside_loop",
        "## Main\nLet mutable i be 1.\nWhile i is at most 3:\n    Show i.\n    Set i to i + 1.\n",
        &[1, 2, 3],
    ),
    (
        "branch_picks_a_different_stream",
        "## Main\nLet mutable n be 4.\nIf n is greater than 3:\n    Show 10.\n    Show 20.\nShow n.\n",
        &[10, 20, 4],
    ),
];

/// ── A BRANCHY MAIN LOWERS, AND ITS TAPE IS THE VM'S.
#[test]
fn a_branchy_main_lowers_to_the_fabric_and_shows_what_the_vm_shows() {
    let mut lowered = 0usize;
    let mut refused: Vec<String> = Vec::new();
    for (label, src, expect) in PROGRAMS {
        match try_main_cfg_fabric(src) {
            Ok(pf) => {
                let (_ret, shown) = eval_program_cfg_fabric_traced(&pf, &[]);
                assert_eq!(
                    &shown[..],
                    *expect,
                    "{label}: the fabric's output tape is not the VM's — lowered {} block(s)",
                    pf.blocks.len()
                );
                lowered += 1;
                println!("  {label}: {} blocks, tape {:?}", pf.blocks.len(), shown);
            }
            Err(e) => refused.push(format!("{label}: {e}")),
        }
    }
    assert!(
        refused.is_empty(),
        "{}/{} branchy Mains still refuse the fabric:\n  {}",
        refused.len(),
        PROGRAMS.len(),
        refused.join("\n  ")
    );
    assert_eq!(lowered, PROGRAMS.len());
}

/// ── AND THE STRAIGHT-LINE CASE STILL WORKS THROUGH THE SAME ROUTE.
#[test]
fn a_straight_line_main_lowers_through_the_same_route() {
    let pf = try_main_cfg_fabric("## Main\nLet x be 7.\nShow x + 1.\nShow x.\n")
        .expect("a straight-line Main lowers");
    let (_ret, shown) = eval_program_cfg_fabric_traced(&pf, &[]);
    assert_eq!(shown, vec![8, 7], "the tape is the VM's, in order");
}

/// ── A RENDERING THE FABRIC CANNOT DO IS REFUSED BY NAME, never shown wrongly.
#[test]
fn showing_something_the_tape_cannot_render_is_refused_by_name() {
    let e = try_main_cfg_fabric("## Main\nShow \"hello\".\n").err();
    match e {
        Some(err) => {
            let m = format!("{err}");
            assert!(
                m.contains("text") || m.contains("Show") || m.contains("unsupported"),
                "a rendering the fabric cannot do must be refused by NAME, got: {m}"
            );
            println!("  text Show refused: {m}");
        }
        None => panic!("showing a string must be refused, not rendered by guesswork"),
    }
}
