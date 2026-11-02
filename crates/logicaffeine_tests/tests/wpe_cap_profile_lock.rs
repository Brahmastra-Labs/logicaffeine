//! wpe_cap_profile_lock — a BIGGER board's engine profile may never be SMALLER in any dimension, and the
//! frame stack must refuse to overflow rather than wrap into a plausible wrong answer.
//!
//! Both invariants come from a real silicon divergence. The same `.wire` programs streamed to the two boards
//! disagreed on exactly the three recursive ones, with no error on either side:
//!
//! | program       | XC7A100T (`WPE_HW`, DEPTH=8) | XC7A200T (`WPE_STREAM`, DEPTH=4) | correct |
//! |---------------|------------------------------|----------------------------------|---------|
//! | bench_fib     | 21                           | 8                                | 21      |
//! | wpe_rec8      | 36                           | 20                               | 36      |
//! | wpe_recursion | 15                           | 11                               | 15      |
//!
//! The bigger part is 20-500x larger in every other dimension, so `WPE_STREAM_DEPTH = 4` reads as a typo
//! rather than a decision — but nothing was watching, because a cap profile was only ever checked against the
//! program being emitted, never against the other profiles. A program that fits the small engine and runs
//! correctly there must not silently produce a different number on the large one.
//!
//! The second invariant is why the first one was survivable in the first place: `wpe_soc_template.v` advances
//! the call stack with a bare `sp <= sp + 1` and no bound against `DEPTH`, so once `sp` reaches `DEPTH` the
//! writes to `sret[sp]`/`sdst[sp]` fall outside the declared range and are discarded, and `sf_ra` indexes past
//! `sfile`. Verilog gives no fault for either. Raising DEPTH moves the cliff; it does not remove it. Any
//! program can exceed any finite frame stack, so overflow has to be reported, not absorbed.
//!
//! Run: `cargo test -p logicaffeine-tests --test wpe_cap_profile_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    WPE_HW_DEPTH, WPE_HW_HEAP, WPE_HW_MAXARG, WPE_HW_MAXBLK, WPE_HW_MAXCELL, WPE_HW_MAXK,
    WPE_HW_MAXLIV, WPE_HW_MAXWR, WPE_HW_NREG, WPE_STREAM_DEPTH, WPE_STREAM_HEAP, WPE_STREAM_MAXARG,
    WPE_STREAM_MAXBLK, WPE_STREAM_MAXCELL, WPE_STREAM_MAXK, WPE_STREAM_MAXLIV, WPE_STREAM_MAXV,
    WPE_STREAM_MAXWR, WPE_STREAM_NREG,
};

/// `WPE_HW`'s cell-value plane is not a separate cap — the register-backed engine sizes `v[]` by `MAXCELL`.
const WPE_HW_MAXV: usize = WPE_HW_MAXCELL;

/// Every dimension of the engine config, smallest profile first. Adding a cap to the engine without adding it
/// here leaves that dimension unwatched, which is exactly how DEPTH regressed — so the count is pinned below.
fn dimensions() -> Vec<(&'static str, usize, usize)> {
    vec![
        ("NREG", WPE_HW_NREG, WPE_STREAM_NREG),
        ("MAXBLK", WPE_HW_MAXBLK, WPE_STREAM_MAXBLK),
        ("MAXCELL", WPE_HW_MAXCELL, WPE_STREAM_MAXCELL),
        ("MAXWR", WPE_HW_MAXWR, WPE_STREAM_MAXWR),
        ("MAXARG", WPE_HW_MAXARG, WPE_STREAM_MAXARG),
        ("MAXLIV", WPE_HW_MAXLIV, WPE_STREAM_MAXLIV),
        ("MAXK", WPE_HW_MAXK, WPE_STREAM_MAXK),
        ("HEAP", WPE_HW_HEAP, WPE_STREAM_HEAP),
        ("DEPTH", WPE_HW_DEPTH, WPE_STREAM_DEPTH),
        ("MAXV", WPE_HW_MAXV, WPE_STREAM_MAXV),
    ]
}

/// The `WPE_STREAM` engine (XC7A200T, config in block RAM) is the large-capacity successor to `WPE_HW`
/// (XC7A100T, config in flip-flops). "Successor" has to mean it in every dimension: a program accepted by
/// `wprog_config_fits_hw_engine` carries no record of the profile it was built for, so the host can stream it
/// into either engine and nothing downstream re-checks it. If a dimension shrinks, that program runs and lies.
#[test]
fn a_bigger_engine_profile_is_never_smaller_in_any_dimension() {
    let dims = dimensions();
    assert_eq!(
        dims.len(),
        10,
        "the engine takes a 10-dimension caps array (WPE_STREAM_CAPS) — every dimension must be compared \
         here, or it regresses unwatched the way DEPTH did"
    );

    let shrunk: Vec<String> = dims
        .iter()
        .filter(|(_, hw, stream)| stream < hw)
        .map(|(name, hw, stream)| format!("{name}: WPE_HW={hw} but WPE_STREAM={stream}"))
        .collect();

    assert!(
        shrunk.is_empty(),
        "the XC7A200T engine profile is SMALLER than the XC7A100T one in {} dimension(s): {}.\n\
         A program that fits the small engine can be streamed straight into the large one — nothing carries \
         the profile it was built for — so a shrunk dimension turns a correct program into a wrong answer \
         with no error. On silicon this cost bench_fib (21 -> 8), wpe_rec8 (36 -> 20) and wpe_recursion \
         (15 -> 11). Grow the dimension, or make the wire image carry its profile so the engine can refuse it.",
        shrunk.len(),
        shrunk.join(", ")
    );
}

/// Raising DEPTH only moves the cliff. `sp` is a 16-bit counter incremented per Call with no comparison
/// against DEPTH, so any recursion deeper than the frame stack silently corrupts: the out-of-range
/// `sret[sp]`/`sdst[sp]` writes are dropped and `sf_ra <= (sp-1)*NREG + ..` runs off the end of `sfile`.
/// Deep recursion is data-dependent, so no static fit-check can rule it out — the engine has to notice.
#[test]
fn the_frame_stack_reports_overflow_instead_of_silently_wrapping() {
    let template = include_str!("../../logicaffeine_compile/src/codegen_sva/wpe_soc_template.v");

    let push = template
        .lines()
        .find(|l| l.contains("sp <= sp + "))
        .expect("the engine template must push a frame with `sp <= sp + ..` on Call");

    assert!(
        push.contains("sp + 16'd1 <= DEPTH") || push.contains("sp < DEPTH") || template.contains("sp_ovf"),
        "the frame-stack push is unguarded:\n    {}\n\
         `sp` advances with no bound against DEPTH, so recursion past the frame stack drops its \
         `sret[sp]`/`sdst[sp]` writes and indexes off the end of `sfile` — Verilog faults on neither, so the \
         engine returns a plausible wrong number instead of failing. Gate the push on `sp` and surface the \
         overflow (an `sp_ovf` flag reported in the result frame), so an over-deep program is refused rather \
         than answered incorrectly.",
        push.trim()
    );
}
