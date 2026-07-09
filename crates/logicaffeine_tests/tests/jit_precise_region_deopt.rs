//! Soundness net for PRECISE region deopt (the worklist tier-up path).
//!
//! A region that does a `ListPush` beside an in-place `SetIndex` (BFS / generic
//! worklist) cannot use the classic discard-replay-from-head deopt: the
//! truncate rolls the pushes back but the in-place write persists, so a
//! self-gated re-push is skipped and worklist entries vanish. Such a region now
//! tiers up with PRECISE deopt — on a side exit it materializes the frame's
//! scalars into the VM registers and resumes the bytecode AT the faulting op,
//! so completed iterations' effects stand and the faulting op re-runs exactly
//! once.
//!
//! For an all-int region the only reachable mid-region side exit is a checked
//! out-of-bounds access (errors). This drives the precise path directly: the
//! region runs hot (tiers up native), processes many valid push+write
//! iterations, then OOBs — the native run side-exits through the precise
//! terminal, the VM materializes the registers and re-executes the faulting
//! load on bytecode, which raises the exact error. If the materialization were
//! wrong, the re-executed load would read a DIFFERENT index (in bounds) and the
//! VM would diverge from the tree-walker. So an exact `(output, error)` match
//! between the bytecode-VM-with-JIT and the tree-walker certifies that the
//! precise materialize-and-resume is correct AND that the completed pushes were
//! neither lost nor duplicated.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `src` on BOTH the bytecode VM (forge JIT installed) and the tree-walker;
/// assert identical output AND identical error. Unlike the float-pin harness,
/// this does NOT require success — a precise deopt that ends in an error must
/// raise the SAME error as the reference.
fn assert_engines_agree(src: &str) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM+JIT diverged from the tree-walker on:\n{src}"
    );
}

/// Run on a 256 MiB stack — the BFS/permutation rebuild shapes recurse deep in
/// the tree-walker reference path.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

/// Run `src` on BOTH engines; assert bit-identical output+error AND return the
/// number of PRECISE regions that tiered through the CONTIGUOUS regalloc backend.
/// A precise push+SetIndex region (the fannkuch / graph_bfs worklist shape) must
/// go through the register-allocating backend, not the per-piece stencil tier —
/// `regalloc_precise_region_count` counts EXACTLY that (it ignores a program's
/// non-precise loops, so it cannot be satisfied by an incidental build/reset
/// loop the way `regalloc_region_count` could).
fn agree_with_precise_regalloc(src: &str) -> u32 {
    let src = src.to_string();
    on_big_stack(move || {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(&src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "VM+JIT diverged from the tree-walker on:\n{src}"
        );
        tier.regalloc_precise_region_count()
    })
}

/// A hot `push + SetIndex` loop that runs valid for >100 iterations (tiering up
/// the precise region native), then a crafted out-of-bounds load fires the
/// precise side exit AFTER many completed pushes. Both engines must raise the
/// same out-of-bounds error — proving the precise materialize/resume re-runs
/// the faulting load with the right index and that the completed pushes did not
/// corrupt the pre-error state.
#[test]
fn precise_region_oob_after_pushes_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let n be 200.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Let mutable idx be k + 1.\n\
         \x20   If k is at least 150:\n\
         \x20       Set idx to idx + n.\n\
         \x20   If item idx of arr equals 0:\n\
         \x20       Set item idx of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20   Set k to k + 1.\n\
         Show length of q.\n",
    );
}

/// LATENT SOUNDNESS BUG — in-place RMW + recoverable deopt, NO push. The region
/// does an in-place read-modify-write `arr[i] = arr[i] + 1` (shared Rc buffer)
/// beside a `Map of Int to Float` read that misses the int fast lane and
/// recoverably deopts every iteration. Precise (resume-at-op, no-replay) deopt
/// is gated on `ListPush`, so this region uses the CLASSIC discard-replay-from-
/// head deopt. A scalar accumulator survives that (its native value lived in the
/// discarded private frame), but the array RMW already landed in the SHARED
/// buffer — so the bytecode replay double-applies it. Without per-array
/// snapshot/restore on deopt, the VM's checksum exceeds the tree-walker's.
#[test]
fn rmw_region_with_recoverable_deopt_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let mutable m be a new Map of Int to Float.\n\
         Let mutable k be 0.\n\
         While k is less than 30:\n\
         \x20   Set item k of m to k * 1.5.\n\
         \x20   Set k to k + 1.\n\
         Let mutable arr be a new Seq of Int.\n\
         Set k to 0.\n\
         While k is less than 30:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set k to k + 1.\n\
         Let mutable s be 0.0.\n\
         Let mutable r be 0.\n\
         While r is less than 5000:\n\
         \x20   Let mutable i be 0.\n\
         \x20   While i is less than 30:\n\
         \x20       Set item (i + 1) of arr to item (i + 1) of arr + 1.\n\
         \x20       Set s to s + item i of m.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set r to r + 1.\n\
         Let mutable sum be 0.\n\
         Let mutable j be 1.\n\
         While j is at most 30:\n\
         \x20   Set sum to sum + item j of arr.\n\
         \x20   Set j to j + 1.\n\
         Show sum.\n",
    );
}

/// The same worklist shape but it COMPLETES (no OOB): the push+SetIndex region
/// tiers up and runs natively to the end. Pins the normal (non-deopt) path of
/// the relaxed region — every self-gated cell is written once and pushed once.
#[test]
fn precise_region_worklist_completes_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let n be 300.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   If item (k + 1) of arr equals 0:\n\
         \x20       Set item (k + 1) of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20   Set k to k + 1.\n\
         Show length of q.\n",
    );
}

/// A true BFS (push to the queue, mark `dist[u]` visited) at a size that
/// crosses the region tier-up threshold — the exact shape the precise path was
/// built for. Completes; output must match the tree-walker.
#[test]
fn precise_region_bfs_shape_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let n be 120.\n\
         Let mutable dist be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 - 1 to dist.\n\
         \x20   Set i to i + 1.\n\
         Push 0 to q.\n\
         Set item 1 of dist to 0.\n\
         Let mutable front be 1.\n\
         While front is at most length of q:\n\
         \x20   Let v be item front of q.\n\
         \x20   Let u be (v + 1) % n.\n\
         \x20   If item (u + 1) of dist equals 0 - 1:\n\
         \x20       Set item (u + 1) of dist to item (v + 1) of dist + 1.\n\
         \x20       Push u to q.\n\
         \x20   Set front to front + 1.\n\
         Show length of q.\n",
    );
}

/// A push+SetIndex worklist carrying a FLOAT accumulator — the case the old
/// all-int restriction wrongly disqualified (a float op was present). Precise
/// materialization now re-boxes each register BY ITS KIND, so the region tiers
/// up and the `acc` float is materialized as a Float (not an Int). Output must
/// match the tree-walker.
#[test]
fn precise_region_with_float_scalar_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let n be 200.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable acc be 0.0.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   If item (k + 1) of arr equals 0:\n\
         \x20       Set item (k + 1) of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20       Set acc to acc + 1.5.\n\
         \x20   Set k to k + 1.\n\
         Show \"{acc:.1} \" + length of q.\n",
    );
}

/// A push+SetIndex worklist carrying a BOOL flag — also disqualified by the old
/// all-int rule. The flag re-boxes as a Bool on a precise resume.
#[test]
fn precise_region_with_bool_scalar_matches_reference() {
    assert_engines_agree(
        "## Main\n\
         Let n be 200.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable seen be 0.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Let flag be item (k + 1) of arr is greater than 0.\n\
         \x20   If flag is false:\n\
         \x20       Set item (k + 1) of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20       Set seen to seen + 1.\n\
         \x20   Set k to k + 1.\n\
         Show seen + length of q.\n",
    );
}

// =====================================================================
// WAVE 21: a precise region with an in-place mutation AND a reallocating
// `Push` must tier through the CONTIGUOUS regalloc backend (not the per-piece
// stencil tier). The push reallocs the pinned buffer; the precise materialize
// re-boxes the live grown array from the VM register (kept, kind = None) and
// resumes AT the faulting op — never re-applying the push. These tests prove
// bit-identical parity (incl. on deopt) AND that the region now regallocs.
// =====================================================================

/// RED (regalloc gate): the canonical push+SetIndex worklist must now go
/// through the register-allocating backend. Before Wave 21 a precise region fell
/// to the per-piece stencil tier (4-6× per-piece overhead); now it regallocs.
#[test]
#[cfg(target_arch = "x86_64")]
fn precise_push_setindex_region_uses_regalloc() {
    let ra = agree_with_precise_regalloc(
        "## Main\n\
         Let n be 300.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   If item (k + 1) of arr equals 0:\n\
         \x20       Set item (k + 1) of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20   Set k to k + 1.\n\
         Show length of q.\n",
    );
    assert!(
        ra >= 1,
        "the precise push+SetIndex worklist must tier through the regalloc \
         PRECISE REGION backend (got {ra})"
    );
}

/// RED (deopt soundness + regalloc): a push+SetIndex region that runs hot, lands
/// many completed pushes+writes, then takes an OOB load AFTER a push. The
/// regalloc precise side exit must store the resume pc, the VM must materialize
/// the grown array (kept from the register) and the flushed scalars, and resume
/// AT the faulting load — raising the EXACT tree-walker error with the grown
/// array intact and NO double-applied push. If the regalloc materialize were
/// wrong the re-run load would read a different index and diverge.
#[test]
#[cfg(target_arch = "x86_64")]
fn precise_regalloc_oob_after_push_matches_reference() {
    let ra = agree_with_precise_regalloc(
        "## Main\n\
         Let n be 200.\n\
         Let mutable arr be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 to arr.\n\
         \x20   Set i to i + 1.\n\
         Let mutable k be 0.\n\
         While k is less than n:\n\
         \x20   Let mutable idx be k + 1.\n\
         \x20   If k is at least 150:\n\
         \x20       Set idx to idx + n.\n\
         \x20   If item idx of arr equals 0:\n\
         \x20       Set item idx of arr to 1.\n\
         \x20       Push k to q.\n\
         \x20   Set k to k + 1.\n\
         Show length of q.\n",
    );
    assert!(
        ra >= 1,
        "the precise push+SetIndex worklist (OOB shape) must tier through the \
         regalloc PRECISE REGION backend (got {ra})"
    );
}

/// RED (fannkuch shape): the permutation rebuild + in-place reverse loop. Each
/// outer iteration allocates a fresh `perm` (NewEmptyList), rebuilds it by
/// pushing every element of `perm1`, then reverses a prefix IN PLACE
/// (SetIndex swaps). The push+SetIndex coexistence is precise; it must
/// regalloc and stay bit-identical.
#[test]
#[cfg(target_arch = "x86_64")]
fn precise_fannkuch_rebuild_shape_uses_regalloc() {
    let ra = agree_with_precise_regalloc(
        "## Main\n\
         Let n be 8.\n\
         Let mutable perm1 be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push i to perm1.\n\
         \x20   Set i to i + 1.\n\
         Let mutable checksum be 0.\n\
         Let mutable rounds be 0.\n\
         While rounds is less than 500:\n\
         \x20   Let mutable perm be a new Seq of Int.\n\
         \x20   Set i to 1.\n\
         \x20   While i is at most n:\n\
         \x20       Push item i of perm1 to perm.\n\
         \x20       Set i to i + 1.\n\
         \x20   Let mutable lo be 1.\n\
         \x20   Let mutable hi be n.\n\
         \x20   While lo is less than hi:\n\
         \x20       Let tmp be item lo of perm.\n\
         \x20       Set item lo of perm to item hi of perm.\n\
         \x20       Set item hi of perm to tmp.\n\
         \x20       Set lo to lo + 1.\n\
         \x20       Set hi to hi - 1.\n\
         \x20   Set checksum to checksum + item 1 of perm.\n\
         \x20   Set rounds to rounds + 1.\n\
         Show checksum.\n",
    );
    assert!(
        ra >= 1,
        "the fannkuch rebuild+reverse loop must tier through the regalloc \
         PRECISE REGION backend (got {ra})"
    );
}

/// RED (graph_bfs frontier shape): a real BFS — push to the queue, mark
/// `dist[u]` in place — sized to cross the region tier-up threshold. The
/// push+SetIndex frontier loop must regalloc and stay bit-identical.
#[test]
#[cfg(target_arch = "x86_64")]
fn precise_graph_bfs_frontier_uses_regalloc() {
    let ra = agree_with_precise_regalloc(
        "## Main\n\
         Let n be 500.\n\
         Let mutable dist be a new Seq of Int.\n\
         Let mutable q be a new Seq of Int.\n\
         Let mutable i be 0.\n\
         While i is less than n:\n\
         \x20   Push 0 - 1 to dist.\n\
         \x20   Set i to i + 1.\n\
         Push 0 to q.\n\
         Set item 1 of dist to 0.\n\
         Let mutable front be 1.\n\
         While front is at most length of q:\n\
         \x20   Let v be item front of q.\n\
         \x20   Let a be (v * 2 + 1) % n.\n\
         \x20   Let b be (v * 2 + 2) % n.\n\
         \x20   If item (a + 1) of dist equals 0 - 1:\n\
         \x20       Set item (a + 1) of dist to item (v + 1) of dist + 1.\n\
         \x20       Push a to q.\n\
         \x20   If item (b + 1) of dist equals 0 - 1:\n\
         \x20       Set item (b + 1) of dist to item (v + 1) of dist + 1.\n\
         \x20       Push b to q.\n\
         \x20   Set front to front + 1.\n\
         Show length of q.\n",
    );
    assert!(
        ra >= 1,
        "the graph_bfs frontier loop must tier through the regalloc PRECISE \
         REGION backend (got {ra})"
    );
}
