//! The function JIT's LIST discipline, locked against the tree-walker on programs that each broke it.
//!
//! A VM register is a SLOT, not a variable: the bytecode compiler hands the same register to different
//! values at different points of one function — a block's bindings are recycled when the block closes
//! (`exit_block`), a statement's scratch when the statement ends. And a LOGOS list is a VALUE: the VM shares
//! buffers freely and copies a shared one on its first write, except through the registers it has proven are
//! mutable borrows (`CompiledFunction::mutable_param_regs`). Native code has to honor both. Four defects, all
//! found here, all present before this lock (the first two reproduce with statement reclaim switched off):
//!
//!   1. LIST TYPING WAS FLOW-INSENSITIVE. "Every register that ever holds a list" was treated as a list at
//!      every pc, so after a `Move` out of a slot that held an Int *there*, native code planted a list triple
//!      from the Int and dereferenced it as a `*mut Vec<i64>` — SIGABRT (`misaligned pointer dereference …
//!      0x64`). Decisions now ask the flow-sensitive kind at their own pc (`holds_list_at`).
//!   2. NO COPY-ON-WRITE. An element store went through the pinned buffer even when the list might be a
//!      caller's (a parameter, or a self-call result passing one through) — the caller's list changed under
//!      native code (`item 1 of arr`: 798 where the VM keeps 0). A store into a possibly-shared, non-exempt
//!      list now copies first (`logos_rt_list_own`), as the VM does.
//!   3. OVERFLOW WAS NOT A RESUME POINT. Only a hand-kept list of ops got a deopt re-box row, but Int `*`/`+`
//!      side-exit on overflow (and a fused pair exits at its first op) — the deopt walk panicked (`missing kind
//!      capture`). Every reachable pc now has a row.
//!   4. ALIASES DIED AT DEOPT. Two registers sharing one native list: the first detached it from the
//!      registry, the second then re-boxed as Nothing (`Cannot index into Nothing`). Each handle detaches once
//!      and every alias gets that one (shared) value.
//!
//! Every program here must run on the tiered VM exactly as on the tree-walker AND still tier — a reused slot
//! or a shared list is not a reason to give up native code.
//!
//! ★ MUTATION PROBES: see the bottom of this file.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test jit_register_kind_reuse_lock`

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

/// Run on the tiered VM and the tree-walker; assert they agree exactly; return (output, functions tiered).
fn tiered(src: &str) -> (String, u32) {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let tier = ForgeTier::new();
            let vm = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
            let tw = tw_outcome_with_args(&src, &[]);
            assert_eq!(tw.error, None, "the reference run must succeed:\n{src}");
            assert_eq!(
                (norm(&vm.output), &vm.error),
                (norm(&tw.output), &tw.error),
                "tiered VM diverged from the tree-walker on:\n{src}"
            );
            let (_, fn_ok) = tier.function_counts();
            (norm(&vm.output), fn_ok)
        })
        .expect("spawn")
        .join()
        .expect("test thread panicked")
}

const MAIN: &str = "## Main\n\
Let mutable arr be a new Seq of Int.\n\
Let mutable i be 0.\n\
While i is less than 300:\n\
\x20   Push (i * 7919) % 1009 to arr.\n\
\x20   Set i to i + 1.\n\
Let mutable total be 0.\n\
Let mutable round be 0.\n\
While round is less than 40:\n\
\x20   Let out be walk(arr, 200).\n\
\x20   Set total to (total + item 1 of out + length of out) % 1000000007.\n\
\x20   Set round to round + 1.\n\
Show total.\n";

/// ── A BLOCK'S LIST, THEN AN INT, IN ONE REGISTER. `seen` is a list bound inside the `If`; the block closes,
/// its register is recycled, and `k` — an Int — is bound into it. `k` is then staged as a call argument.
/// Needs no statement-level reclaim at all: `exit_block` alone produces this bytecode. (`Let m …` breaks the
/// `Let r be walk(…). Return r.` self-tail-call pair, which the VM would otherwise lower to a loop-back —
/// leaving `walk` too cold to tier and the test proving nothing.)
#[test]
fn a_block_list_register_reused_for_an_int_runs_exactly() {
    let src = format!(
        "## To walk (arr: Seq of Int, n: Int) -> Seq of Int:\n\
\x20   If n is at most 0:\n\
\x20       Return arr.\n\
\x20   If n is greater than 1000000:\n\
\x20       Let seen be walk(arr, n - 1).\n\
\x20       Return seen.\n\
\x20   Let k be n - 1.\n\
\x20   Let r be walk(arr, k).\n\
\x20   Let m be length of r.\n\
\x20   If m is less than 0:\n\
\x20       Return arr.\n\
\x20   Return r.\n\n{MAIN}"
    );
    let (out, fn_ok) = tiered(&src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must still tier (got {fn_ok})");
}

/// ── TWO CALLS, ONE ARGUMENT WINDOW. The first call stages the list `result` into its argument window; the
/// next statement's Int arithmetic lands in the same registers once the window is reclaimed. The quicksort
/// tail (`Set result to qs(result, lo, i - 1). Set result to qs(result, i + 1, hi).`) in its smallest form.
#[test]
fn an_argument_window_that_held_a_list_then_holds_an_int_runs_exactly() {
    let src = format!(
        "## To walk (arr: Seq of Int, n: Int) -> Seq of Int:\n\
\x20   If n is at most 1:\n\
\x20       Return arr.\n\
\x20   Let mutable result be arr.\n\
\x20   Set result to walk(result, n - 2).\n\
\x20   Set result to walk(result, n / 2 - 1).\n\
\x20   Return result.\n\n{MAIN}"
    );
    let (out, fn_ok) = tiered(&src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must still tier (got {fn_ok})");
}

/// ── THE QUICKSORT SHAPE, FROM BLOCK RECYCLING ALONE. Inside the loop, `Let t be item j of result.` puts an
/// Int in a scratch register and copies it (`Move`); the loop closes and its registers are recycled; the very
/// statement after `Let u` stages the list `result` into the loop's `Index` register as a call argument. The
/// JIT used to plant a list triple after the in-loop `Move` of the Int. No statement-level reclaim is involved:
/// the bytecode is `Index R10 = R3[R7]; Move R11 = R10` in the loop and `Move R10 = R3` staging after it.
#[test]
fn a_loop_int_copy_whose_register_later_stages_a_list_runs_exactly() {
    let src = format!(
        "## To walk (arr: Seq of Int, n: Int) -> Seq of Int:\n\
\x20   If n is at most 1:\n\
\x20       Return arr.\n\
\x20   Let mutable result be arr.\n\
\x20   Let mutable acc be 0.\n\
\x20   Let mutable j be 1.\n\
\x20   While j is at most 3:\n\
\x20       Let t be item j of result.\n\
\x20       Set acc to acc + t.\n\
\x20       Set j to j + 1.\n\
\x20   Let u be acc.\n\
\x20   Set result to walk(result, n - 1).\n\
\x20   Set item 1 of result to (u + item 1 of result) % 1009.\n\
\x20   Return result.\n\n{MAIN}"
    );
    let (out, fn_ok) = tiered(&src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must still tier (got {fn_ok})");
}

/// ── VALUE SEMANTICS THROUGH A RETURNED PARAMETER (no register reuse involved). The base case returns its
/// list parameter; the caller mutates the result. A LOGOS list is a value: the caller's `arr` must be
/// untouched, so every one of the 40 rounds sees the same input.
#[test]
fn mutating_a_returned_list_parameter_does_not_reach_the_callers_list() {
    let src = "## To walk (arr: Seq of Int, n: Int) -> Seq of Int:\n\
\x20   If n is at most 1:\n\
\x20       Return arr.\n\
\x20   Let mutable result be walk(arr, n - 1).\n\
\x20   Set item 1 of result to (item 1 of result + 1) % 1009.\n\
\x20   Return result.\n\n\
## Main\n\
Let mutable arr be a new Seq of Int.\n\
Let mutable i be 0.\n\
While i is less than 300:\n\
\x20   Push (i * 7919) % 1009 to arr.\n\
\x20   Set i to i + 1.\n\
Let mutable total be 0.\n\
Let mutable round be 0.\n\
While round is less than 40:\n\
\x20   Let out be walk(arr, 200).\n\
\x20   Set total to (total + item 1 of out + length of out) % 1000000007.\n\
\x20   Set round to round + 1.\n\
Show total.\n\
Show item 1 of arr.\n";
    let (out, fn_ok) = tiered(src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must still tier (got {fn_ok})");
}

/// ── A DEOPT AFTER A COPY-ON-WRITE RESUMES WITH THE COPY, EVERY ALIAS INTACT. `poke` writes into its own
/// list parameter (a param it never returns is no mutable borrow), so native code copies before the store;
/// the base case hands that copy back up, so the call result and the parameter register alias ONE native
/// list. Then `root * root` overflows: native code deopts, and the VM resumes mid-function (promoting to a
/// big integer) and reads both. Three things must hold, each of which failed before: the overflow is a
/// resume point with a re-box row (it panicked: `missing kind capture`); the copied parameter re-boxes as
/// the COPY, not the caller's list (the write would vanish); and both aliases re-box as the same list (the
/// second came back `Nothing`: `Cannot index into Nothing`).
#[test]
fn a_deopt_after_a_copy_on_write_resumes_with_the_copy() {
    let src = "## To poke (arr: Seq of Int, n: Int) -> Seq of Int:\n\
\x20   If n is at most 0:\n\
\x20       Return arr.\n\
\x20   Set item 1 of arr to n + 1000.\n\
\x20   Let r be poke(arr, n - 1).\n\
\x20   Let root be n + 3037000499.\n\
\x20   Let big be root * root.\n\
\x20   Let q be big / big.\n\
\x20   Let m be item 1 of arr + q.\n\
\x20   Set item 2 of r to m.\n\
\x20   Return r.\n\n\
## Main\n\
Let mutable arr be a new Seq of Int.\n\
Let mutable i be 0.\n\
While i is less than 50:\n\
\x20   Push i to arr.\n\
\x20   Set i to i + 1.\n\
Let mutable total be 0.\n\
Let mutable round be 0.\n\
While round is less than 60:\n\
\x20   Let out be poke(arr, 25 + round % 7).\n\
\x20   Set total to (total + item 1 of out + item 2 of out) % 1000000007.\n\
\x20   Set round to round + 1.\n\
Show total.\n\
Show item 1 of arr.\n";
    let (out, fn_ok) = tiered(src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must still tier (got {fn_ok})");
}

// ★ MUTATION PROBES (each run 2026-09-23: break one fix in `logicaffeine_jit/src/lib.rs`, observe red, restore):
//   1. Move decision asks `is_list[src]` again (flow-insensitive) → `a_block_list_…` and `a_loop_int_copy_…`
//      SIGABRT, `misaligned pointer dereference … 0x64` — with statement reclaim switched off too, so the bug is
//      reachable from `exit_block` recycling alone.
//   2. Never schedule the copy (`own_before` stays empty) → `mutating_a_returned_…` (798 where the VM keeps 0),
//      `a_loop_int_copy_…` (34459 vs 31120), `a_deopt_after_…` (120282 vs 121794).
//   3. Re-box a copied parameter register as the parameter (identity ignored) → `a_deopt_after_…` (64226 vs
//      121794: the write made to the copy is lost at the resume).
//   4. Capture re-box rows only at the old listed ops → `a_deopt_after_…` panics `missing kind capture for pc 56`.
//   5. Detach every alias separately (no per-handle memo) → `a_deopt_after_…`: `Cannot index into Nothing`.
//   6. Probe 1 on `the_bug_002_…` (the retired stopgap's shape) → SIGABRT `misaligned … 0x91`: BUG-002 was defect 1.
// `an_argument_window_…` is the reclaim-era quicksort tail; no single probe reds it — it pins the shape.

/// ── THE BUG-002 SHAPE. Quicksort where an element is loaded (`Let vj be item j of result.`), survives a
/// branch, and is stored in place (`Set item i of result to vj.`), in a recursive function. The adapter
/// refused to tier this shape behind a "SOUNDNESS STOPGAP (BUG-002)" — it segfaulted, root cause unknown. The
/// root cause is defect 1: the loop body's Int slots are the registers the recursive calls later stage lists
/// through (with defect 1 restored and the stopgap off: SIGABRT, `misaligned … 0x91`). The stopgap and its
/// detector are retired; this shape now tiers.
#[test]
fn the_bug_002_quicksort_shape_runs_exactly() {
    let src = "## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:\n\
\x20   If lo is at least hi:\n\
\x20       Return arr.\n\
\x20   Let pivot be item hi of arr.\n\
\x20   Let mutable result be arr.\n\
\x20   Let mutable i be lo.\n\
\x20   Let mutable j be lo.\n\
\x20   While j is less than hi:\n\
\x20       Let vj be item j of result.\n\
\x20       If vj is at most pivot:\n\
\x20           Let vi be item i of result.\n\
\x20           Set item i of result to vj.\n\
\x20           Set item j of result to vi.\n\
\x20           Set i to i + 1.\n\
\x20       Set j to j + 1.\n\
\x20   Let vi be item i of result.\n\
\x20   Set item i of result to item hi of result.\n\
\x20   Set item hi of result to vi.\n\
\x20   Set result to qs(result, lo, i - 1).\n\
\x20   Set result to qs(result, i + 1, hi).\n\
\x20   Return result.\n\n\
## Main\n\
Let mutable arr be a new Seq of Int.\n\
Let mutable seed be 42.\n\
Let mutable i be 0.\n\
While i is less than 600:\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Push (seed / 65536) % 32768 to arr.\n\
\x20   Set i to i + 1.\n\
Set arr to qs(arr, 1, 600).\n\
Let mutable checksum be 0.\n\
Set i to 1.\n\
While i is at most 600:\n\
\x20   Set checksum to (checksum * 31 + item i of arr) % 1000000007.\n\
\x20   Set i to i + 1.\n\
Show checksum.\n";
    let (out, fn_ok) = tiered(src);
    println!("output {out}, functions tiered {fn_ok}");
    assert!(fn_ok >= 1, "the function must tier (got {fn_ok})");
}
