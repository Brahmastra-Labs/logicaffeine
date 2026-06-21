//! Contiguous register-allocated region codegen — the EXODIA 3.1 "closer"
//! foundation (WS-G).
//!
//! Where [`crate::jit::compile_straightline_coded`] lowers ONE stencil PIECE
//! per [`MicroOp`] (each piece re-establishing operands from the frame, doing
//! its op, storing the result, and tail-calling the next), this backend emits
//! the WHOLE supported region as ONE contiguous x86-64 function. A global
//! register assignment keeps the hottest slots resident in physical registers
//! across every op, so a reg-resident operand is read with a register move (or
//! used directly) rather than a frame round-trip — eliminating the per-piece
//! ABI/operand overhead that holds the tiered cluster at 3-6× V8.
//!
//! ## Soundness contract (the differential gate is sacred)
//!
//! The emitted function is BIT-IDENTICAL to [`crate::jit::reference_eval`] and
//! thus to the tree-walker:
//! - i64 wrapping arithmetic (x86 `add`/`sub`/`imul` wrap natively);
//! - the kernel's exact shift spec (count truncates to the low 6 bits via `cl`);
//! - `Div`/`Mod` side-EXIT on a zero divisor BEFORE any effect (a
//!   [`ChainOutcome::Deopt`] through the shared status cell, replayed on
//!   bytecode where the kernel raises the precise error), and reproduce
//!   `MIN / -1 = MIN` / `MIN % -1 = 0` without the `#DE` overflow trap.
//!
//! On exit (every `Return`) ALL reg-resident slots are flushed back to the
//! frame, so the frame the caller observes is consistent with the tree-walker's
//! full frame state — the region tier can resume / deopt from frame slots.
//!
//! ## Assignment model
//!
//! A slot is either RESIDENT (lives in one fixed physical register for the
//! whole function) or SPILLED (lives in its frame slot). Slots are ranked by
//! reference count; the top ones that fit get the allocatable callee/caller
//! saved GPRs. This is a *global* per-slot assignment (no per-point churn), so
//! it is trivially correct across loops and back-edges: a slot is always in the
//! same place. `rax`/`rdx`/`rcx` are reserved scratch (arithmetic, division,
//! shift count); `r15` holds the frame base pointer.

#![cfg(target_arch = "x86_64")]

use std::sync::atomic::AtomicI64;

use crate::buffer::JitChain;
use crate::jit::{Cmp, CompiledChain, MicroOp, Slot};
use crate::x64asm::{Asm, Cond, LabelId, Reg, Xmm};

/// Where a slot's value lives for the whole function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loc {
    /// Resident in a physical general-purpose register (an INT-class slot).
    Reg(Reg),
    /// Resident in a physical XMM register (a FLOAT-class slot). The slot's f64
    /// value lives here for the whole function; never simultaneously GP-resident.
    Xmm(Xmm),
    /// Spilled to `frame[slot]` (byte displacement = `slot * 8`). Both classes
    /// load/store the SAME 8 raw bytes here, so a slot used by both an int and a
    /// float op (or whose class is ambiguous) is kept in the frame, where bits
    /// are reinterpreted freely.
    Frame,
}

/// The frame base register (`*mut i64`, the chain ABI's first argument).
const BASE: Reg = Reg::R15;
/// Primary arithmetic scratch / division quotient / return value.
const S0: Reg = Reg::Rax;
/// Secondary arithmetic scratch / division remainder.
const S1: Reg = Reg::Rdx;
/// Shift-count scratch (the only register `shl`/`sar` by `cl` can use).
const SC: Reg = Reg::Rcx;

/// Primary XMM scratch (float arithmetic accumulator / load target).
const FS0: Xmm = Xmm::Xmm14;
/// Secondary XMM scratch (float second operand / abs-mask staging).
const FS1: Xmm = Xmm::Xmm15;

/// The BYTE offset of the `capacity` and `len` fields WITHIN a `Vec<T>`,
/// discovered by PROBING the running binary's actual `Vec` layout — NEVER a
/// hardcoded assumption.
///
/// `Vec<T>`'s field order (`ptr`/`cap`/`len`) is an implementation detail the
/// Rust language does NOT guarantee (it differs across `std` builds — on the
/// current toolchain the order is `cap, ptr, len`). The inline [`ArrPush`] fast
/// path needs `cap` (the realloc test) and must keep the real `Vec::len` field
/// coherent (so a later helper push / drop / deopt sees the appended element), so
/// it reads/writes those two fields at these offsets. Probing the LIVE layout —
/// constructing a real `Vec` and matching each word against the known
/// `capacity()`/`len()` — makes the inline path bit-identical to `Vec::push` on
/// EXACTLY this binary, with no cross-version layout assumption. The probe is
/// computed once. The `ptr` field is never read inline (the mirrored frame
/// `ptr_slot` is the source of truth and never moves on the fast path), so only
/// these two offsets are needed.
///
/// The offsets are T-INDEPENDENT: `Vec<T>` is `RawVec<T> { ptr: NonNull<T>, cap }`
/// + `len`, all word-sized regardless of `T`, so one `Vec<i64>` probe serves the
/// Int / Float / Bool element buffers alike.
struct VecLayout {
    cap_off: i32,
    len_off: i32,
}

fn vec_layout() -> &'static VecLayout {
    use std::sync::OnceLock;
    static LAYOUT: OnceLock<VecLayout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        // Distinct, recognizable cap and len so each maps to exactly one word.
        let mut v: Vec<i64> = Vec::with_capacity(8);
        // SAFETY: capacity is 8 ≥ 3, so setting len to 3 is in-bounds; the
        // backing memory is uninitialized but never read (we forget `v`).
        unsafe { v.set_len(3) };
        let cap = v.capacity(); // 8
        let len = v.len(); // 3
        debug_assert_eq!(std::mem::size_of::<Vec<i64>>(), 24, "Vec is a 3-word triple");
        let base = &v as *const Vec<i64> as *const usize;
        // SAFETY: a `Vec<i64>` is exactly three `usize`-sized words.
        let words = unsafe { std::slice::from_raw_parts(base, 3) };
        let cap_off = words
            .iter()
            .position(|&w| w == cap)
            .expect("Vec capacity field located") as i32
            * 8;
        let len_off = words
            .iter()
            .position(|&w| w == len)
            .expect("Vec len field located") as i32
            * 8;
        // Restore len so the (forgotten) Vec drops its real (empty) contents
        // without touching the uninitialized tail.
        unsafe { v.set_len(0) };
        VecLayout { cap_off, len_off }
    })
}

/// XMM registers we may assign to FLOAT-class slots. All are caller-saved under
/// SysV (no prologue save/restore); `xmm14`/`xmm15` are reserved as scratch
/// (`FS0`/`FS1`). Ordered low→high so the assignment is deterministic.
const FLOAT_REGS: [Xmm; 14] = [
    Xmm::Xmm0, Xmm::Xmm1, Xmm::Xmm2, Xmm::Xmm3, Xmm::Xmm4, Xmm::Xmm5, Xmm::Xmm6,
    Xmm::Xmm7, Xmm::Xmm8, Xmm::Xmm9, Xmm::Xmm10, Xmm::Xmm11, Xmm::Xmm12, Xmm::Xmm13,
];

/// Callee-saved registers we may assign (must be saved/restored).
const CALLEE_SAVED: [Reg; 4] = [Reg::Rbx, Reg::R12, Reg::R13, Reg::R14];
/// Caller-saved registers we may assign freely (no save/restore needed).
/// `rdi`/`rsi` are the incoming `base`/`sp` args — `base` is copied to r15 in
/// the prologue and `sp` is unused by the int subset, so both are free after
/// entry.
const CALLER_SAVED: [Reg; 6] = [Reg::R8, Reg::R9, Reg::R10, Reg::R11, Reg::Rdi, Reg::Rsi];

/// Whether the contiguous register-allocating backend is selected. Default ON:
/// it is bit-identical to the stencil tier (proven by the full corpus
/// differential with the flag on) and 4-6x faster on the regions it supports,
/// falling back to the per-piece tier on any unsupported op. `LOGOS_REGALLOC=0`
/// is the kill-switch. Read once.
pub fn regalloc_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_REGALLOC").map_or(true, |v| v != "0"))
}

/// Whether the CALL-LOCALITY ranking bonus is applied (Fix 1). Default ON: it is
/// a pure register-assignment reorder (bit-identical, proven by the corpus
/// differential) that lands call-surviving values in callee-saved registers so a
/// call-heavy loop pays no per-call spill/reload. `LOGOS_CALL_WEIGHT=0` is the
/// kill-switch (also used to A/B the lever's effect on a quiet box). Read once.
fn call_weight_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_CALL_WEIGHT").map_or(true, |v| v != "0"))
}

/// Whether the INLINE `ArrPush` fast path is emitted (Wave 25). Default ON: it is
/// bit-identical to the helper-call lowering (proven by the corpus differential
/// and the dedicated `inline_push_*` tests), eliminating the per-push spill+call
/// in the common (`len < cap`) case. `LOGOS_NO_INLINE_PUSH=1` falls back to the
/// always-call lowering — the kill-switch AND the A/B toggle for the relative
/// on/off timing on a noisy box. Read once.
fn inline_push_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_NO_INLINE_PUSH").map_or(true, |v| v == "0"))
}

/// Whether the SCALED-INDEX CSE is emitted (Wave 26). A straight-line run of
/// array accesses sharing one index slot recomputes the 0-based index `im1 =
/// idx - 1` (and its scaled byte offset `im1 * 8`) once into reserved
/// caller-saved register(s), then reuses it for each `base + off` address — the
/// LLVM/V8 "compute the scaled offset once, reuse across base pointers" idiom.
/// nbody's inner force loop reads/writes 7 co-indexed arrays at the same `i`
/// (and another set at `j`); matrix_mult / spectral_norm / prefix_sum likewise
/// touch several arrays at a shared index. Default ON: it is bit-identical to
/// the per-access recompute (the cache is recomputed whenever the index slot is
/// reassigned, at every jump target, and across any SysV call), proven by the
/// corpus differential and the dedicated `off_cse_*` tests. `LOGOS_NO_OFF_CSE=1`
/// falls back to the per-access recompute — the kill-switch AND the A/B toggle
/// for the relative on/off timing on a noisy box. Read once.
fn off_cse_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_NO_OFF_CSE").map_or(true, |v| v != "1"))
}

/// Whether the LOOP-INVARIANT ARRAY PTR/LEN HOIST is emitted (Wave 27a). When an
/// array's ptr/len handle slots are FRAME-resident and are never written inside a
/// loop (only in-place `ArrLoad`/`ArrStore` of that array — no reallocating
/// `ArrPush`/`ListClear`/`NewList`/`ListTriple`, no scalar write to the slot),
/// the per-access `emit_arr_addr` reload of those slots from the frame is
/// loop-invariant. The hoist loads them ONCE into loop-persistent CALLEE-SAVED
/// registers at the loop pre-header, so the inner accesses read the register
/// instead of re-touching the frame every iteration (the knapsack inner `w`-loop
/// reads `prev`'s ptr/len every iteration though `prev` only changes on the OUTER
/// loop's `Set prev to curr`). Default ON: bit-identical to the per-access reload
/// (the hoisted register holds exactly `frame[slot]`, which is invariant over the
/// span, and the bounds check still runs per access against the hoisted length),
/// proven by the corpus differential and the dedicated `ptr_hoist_*` tests.
/// `LOGOS_NO_PTR_HOIST=1` falls back to the per-access reload — the kill-switch
/// AND the A/B toggle for the relative on/off timing on a noisy box. Read once.
fn ptr_hoist_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_NO_PTR_HOIST").map_or(true, |v| v != "1"))
}

/// Whether the LOOP-INVARIANT CONSTANT HOIST is emitted (Wave 28). A
/// `LoadConst { dst, value }` INSIDE a natural loop, whose `dst` is written by no
/// other op in the span (so the value is invariant across iterations) and lands
/// in a GP register, has its `mov reg, imm` re-executed every iteration in the
/// body. The hoist emits it ONCE in the loop pre-header instead, leaving the
/// register resident across the back-edge — the V8/LLVM loop-invariant-code-
/// motion idiom (collatz −7%, loop_sum, every pure-GP scalar loop with an
/// invariant constant).
///
/// FLOAT (XMM-resident) consts are EXCLUDED — the investigation began with
/// mandelbrot's `2.0`/`4.0` GP→XMM reloads, the obvious target, but hoisting them
/// into the SATURATED XMM file is bit-identical yet measured ~8% SLOWER (the
/// per-iteration `movabs;movq` is FREE out-of-order work that overlaps the float
/// chain; pinning the invariant in a read-heavy XMM register serializes worse).
/// See the `const_hoist_*` tests and the call site's exclusion comment.
///
/// Default ON: bit-identical to the per-iteration `LoadConst` (the register holds
/// exactly the same constant bits, the body simply stops re-loading them, and the
/// exit flush of a written slot is unchanged), proven by the corpus differential
/// and the dedicated `const_hoist_*` tests. `LOGOS_NO_CONST_HOIST=1` falls back to
/// the per-iteration materialization — the kill-switch AND the A/B toggle for the
/// relative on/off timing on a noisy box. Read LIVE (not cached) so a single
/// process can A/B the two emissions for the structural gate.
fn const_hoist_enabled() -> bool {
    std::env::var("LOGOS_NO_CONST_HOIST").map_or(true, |v| v != "1")
}

/// The maximum LOGOS call depth, baked into the self-call codegen exactly as it
/// is baked into `logos_stencil_call_self` — the kernel's locked
/// `MAX_CALL_DEPTH`. Reusing the single forge constant keeps the contiguous
/// FUNCTION backend's depth guard in lockstep with the stencil tier and the
/// kernel (a drift would diverge the depth-exceeded error). See the
/// `max-call-depth-sync-jit-forge` invariant.
const SELF_CALL_DEPTH_LIMIT: i64 = crate::jit::BAKED_CALL_DEPTH;

/// Is this op in the supported subset of the contiguous backend? An op outside
/// it makes [`compile_region_regalloc`] return `None`, so the caller falls back
/// to the per-piece tier and behavior is unchanged.
fn supported(op: &MicroOp) -> bool {
    matches!(
        op,
        MicroOp::Move { .. }
            | MicroOp::LoadConst { .. }
            | MicroOp::Add { .. }
            | MicroOp::Sub { .. }
            | MicroOp::Mul { .. }
            | MicroOp::Lt { .. }
            | MicroOp::Gt { .. }
            | MicroOp::LtEq { .. }
            | MicroOp::GtEq { .. }
            | MicroOp::Eq { .. }
            | MicroOp::Neq { .. }
            | MicroOp::BitAnd { .. }
            | MicroOp::BitOr { .. }
            | MicroOp::BitXor { .. }
            | MicroOp::Shl { .. }
            | MicroOp::Shr { .. }
            | MicroOp::NotInt { .. }
            | MicroOp::NotBool { .. }
            | MicroOp::Div { .. }
            | MicroOp::Mod { .. }
            | MicroOp::DivPow2 { .. }
            | MicroOp::MagicDivU { .. }
            | MicroOp::Branch { .. }
            | MicroOp::Jump { .. }
            | MicroOp::JumpIfFalse { .. }
            | MicroOp::JumpIfTrue { .. }
            | MicroOp::Return { .. }
            // Integer (8-byte element) array load/store. A FLOAT array element
            // is also an 8-byte (`byte: false`) slot: the raw bits load into /
            // store from a frame slot, and the float arithmetic reinterprets
            // them — no separate "float array" op is needed.
            | MicroOp::ArrLoad { byte: false, .. }
            | MicroOp::ArrStore { byte: false, .. }
            // BYTE (1-byte element) array load/store — the `Seq of Bool` buffer
            // (sieve's `flags`, graph_bfs would-be `visited`). A `byte: true`
            // load is a zero-extended `movzx` (`u8 as i64`); a `byte: true`
            // store writes the BOOLEAN NORMALIZATION `(v != 0) as u8`. The
            // address machinery and OOB side-exit are the 8-byte path with a
            // unit stride — bit-identical to `ST_ARRLDB`/`ST_ARRSTB`.
            | MicroOp::ArrLoad { byte: true, .. }
            | MicroOp::ArrStore { byte: true, .. }
            // FLOAT (f64) ops — the XMM register class (wave 11). Arithmetic is
            // IEEE; `FmaF` is TWO roundings (mulsd then addsd, NOT a fused
            // `vfmadd`); ordering compares are exact IEEE (NaN → false); `EqF`/
            // `NeqF` use the kernel's `|a-b| < EPSILON` rule; `DivF` side-exits
            // on a `0.0` divisor like the integer `Div`.
            | MicroOp::AddF { .. }
            | MicroOp::SubF { .. }
            | MicroOp::MulF { .. }
            | MicroOp::DivF { .. }
            | MicroOp::SqrtF { .. }
            | MicroOp::IntToFloat { .. }
            | MicroOp::FmaF { .. }
            | MicroOp::LtF { .. }
            | MicroOp::GtF { .. }
            | MicroOp::LtEqF { .. }
            | MicroOp::GtEqF { .. }
            | MicroOp::EqF { .. }
            | MicroOp::NeqF { .. }
            | MicroOp::BranchF { .. }
            // LIST MUTATION (wave 13) — helper calls into the JIT runtime.
            // `ArrPush` appends through `logos_rt_push_*` (the call MAY
            // reallocate, so the pinned ptr/len are refreshed in the frame
            // after); `ListClear` truncates a SOLE-OWNED buffer in place
            // through `logos_rt_clear_*` (the alias-safety of the reuse is
            // proven UPSTREAM in the micro-op lowering — a `ListClear` reaching
            // here is unaliased). Both keep their vec/ptr/len handle slots
            // FRAME-resident (the helper reads/writes those frame cells) and
            // spill the caller-saved residents across the SysV call.
            | MicroOp::ArrPush { .. }
            | MicroOp::ListClear { .. }
            // MUTABLE-TEXT append (`Set text to text + ch`). A helper call into
            // the JIT runtime that grows the accumulator THROUGH the pinned
            // `*mut Value` cell handle with the VM's exact `add_assign`
            // (in-place / copy-on-write) semantics; the handle slot is forced
            // frame-resident and the caller-saved residents are spilled across
            // the SysV call, exactly like `ArrPush`.
            | MicroOp::StrAppend { .. }
    )
}

/// Is this op in the supported subset of the contiguous FUNCTION backend
/// ([`compile_function_regalloc`])? It is the region subset PLUS the DIRECT
/// self-call ops (`CallSelf`/`CallSelfCopy`). A cross-function `Call`, list/map
/// ops, byte arrays, etc. remain unsupported → the function falls back to the
/// per-piece stencil tier, byte-identical to today.
fn supported_function(op: &MicroOp) -> bool {
    supported(op) || matches!(op, MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })
}

/// Is this op in the supported subset of the contiguous PRECISE (mode-B)
/// FUNCTION backend ([`compile_function_regalloc_precise`])? This spans two
/// recursion shapes:
///
///  - the IN-PLACE-ARRAY shape (heap_sort's `siftDown`, quicksort's `qs`):
///    scalar arithmetic, checked int/byte array load/store (the in-place
///    mutation), and the precise self-`Call` (windowed disjointly, resuming AT
///    the faulting op on a side exit); and
///  - the fresh-list-RETURN shape (mergesort: allocate fresh `left`/`right`/
///    `result`, push into them, and return a new list): the registry-owned
///    allocation `NewList`, the reallocating `ArrPush`, the in-place `ListClear`,
///    and the `ListTriple` pin refresh from a live handle (a self-call's returned
///    list, seen from the caller).
///
/// SOUNDNESS of the list-return shape: the regalloc body shares the SAME
/// `adapt_function` micro stream, per-op `deopt_codes` table, runtime helpers
/// (`logos_rt_alloc_list_i64` / push / triple), and `ChainFn::call`+`materialize`
/// boundary as the per-piece precise stencil tier — only the body codegen
/// changes. `NewList`/`ArrPush`/`ListTriple` each force their vec/ptr/len triple
/// FRAME-resident (the helper's frame writes are the single source of truth, so a
/// reallocating push's refreshed pointer/length is always read by the next
/// access), every mode-B machinery slot (`>= rc`) stays frame-resident, and every
/// precise side exit flushes the resident-written scalars to the frame. So a
/// registry-owned fresh list materializes (detaches) on a precise resume exactly
/// as in the stencil tier; the differential gate proves it bit-identical.
///
/// It still does NOT admit the classic `CallSelf`/`CallSelfCopy` (those replay
/// from head — sound only for the scalar mode-A path), nor maps/cross-function
/// calls.
fn supported_function_precise(op: &MicroOp) -> bool {
    supported(op)
        || matches!(
            op,
            MicroOp::Call { .. } | MicroOp::ListTriple { .. } | MicroOp::NewList { .. }
        )
}

/// Whether `op` is a CHECKED operation needing the deopt side-exit channel (the
/// status cell): a divisor-check on `Div`/`Mod`, or a bounds-check on a checked
/// integer array load/store. A self-call also writes the status cell (its depth/
/// arena/entry guards and the propagation of an in-callee deopt), so it too
/// requires the channel.
fn needs_deopt(op: &MicroOp) -> bool {
    matches!(
        op,
        MicroOp::Div { .. }
            | MicroOp::Mod { .. }
            | MicroOp::DivF { .. }
            | MicroOp::ArrLoad { checked: true, .. }
            | MicroOp::ArrStore { checked: true, .. }
            | MicroOp::CallSelf { .. }
            | MicroOp::CallSelfCopy { .. }
            // The mode-B precise self-call writes the status cell (its depth/
            // arena/entry guards and the propagation of an in-callee deopt).
            | MicroOp::Call { .. }
    )
}

/// Every slot index an op reads or writes (for reference-count ranking and the
/// max-slot frame bound).
fn slots_of(op: &MicroOp, out: &mut Vec<Slot>) {
    match *op {
        MicroOp::Move { dst, src } | MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src } => {
            out.extend([dst, src])
        }
        MicroOp::LoadConst { dst, .. } => out.push(dst),
        MicroOp::Add { dst, lhs, rhs }
        | MicroOp::Sub { dst, lhs, rhs }
        | MicroOp::Mul { dst, lhs, rhs }
        | MicroOp::Lt { dst, lhs, rhs }
        | MicroOp::Gt { dst, lhs, rhs }
        | MicroOp::LtEq { dst, lhs, rhs }
        | MicroOp::GtEq { dst, lhs, rhs }
        | MicroOp::Eq { dst, lhs, rhs }
        | MicroOp::Neq { dst, lhs, rhs }
        | MicroOp::BitAnd { dst, lhs, rhs }
        | MicroOp::BitOr { dst, lhs, rhs }
        | MicroOp::BitXor { dst, lhs, rhs }
        | MicroOp::Shl { dst, lhs, rhs }
        | MicroOp::Shr { dst, lhs, rhs }
        | MicroOp::Div { dst, lhs, rhs }
        | MicroOp::Mod { dst, lhs, rhs }
        | MicroOp::AddF { dst, lhs, rhs }
        | MicroOp::SubF { dst, lhs, rhs }
        | MicroOp::MulF { dst, lhs, rhs }
        | MicroOp::DivF { dst, lhs, rhs }
        | MicroOp::LtF { dst, lhs, rhs }
        | MicroOp::GtF { dst, lhs, rhs }
        | MicroOp::LtEqF { dst, lhs, rhs }
        | MicroOp::GtEqF { dst, lhs, rhs }
        | MicroOp::EqF { dst, lhs, rhs }
        | MicroOp::NeqF { dst, lhs, rhs } => out.extend([dst, lhs, rhs]),
        MicroOp::FmaF { dst, a, b, c } => out.extend([dst, a, b, c]),
        MicroOp::IntToFloat { dst, src } | MicroOp::SqrtF { dst, src } => out.extend([dst, src]),
        MicroOp::DivPow2 { dst, lhs, .. } => out.extend([dst, lhs]),
        MicroOp::MagicDivU { dst, lhs, .. } => out.extend([dst, lhs]),
        MicroOp::Branch { lhs, rhs, .. } | MicroOp::BranchF { lhs, rhs, .. } => out.extend([lhs, rhs]),
        MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => out.push(cond),
        MicroOp::Return { src } => out.push(src),
        MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, checked, .. } => {
            out.extend([dst, idx, ptr_slot]);
            if checked {
                out.push(len_slot);
            }
        }
        MicroOp::ArrStore { src, idx, ptr_slot, len_slot, checked, .. } => {
            out.extend([src, idx, ptr_slot]);
            if checked {
                out.push(len_slot);
            }
        }
        // A push reads its `src` value (rank it — it may be register/XMM
        // resident) and the vec/ptr/len handle TRIPLE, which is forced
        // frame-resident below (the helper reads/writes those frame cells), so
        // we list the triple only to extend `max_slot`, never to rank it.
        MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([src, vec_slot, ptr_slot, len_slot])
        }
        MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([vec_slot, ptr_slot, len_slot])
        }
        // A fresh-list allocation writes the vec/ptr/len handle TRIPLE (forced
        // frame-resident below — the helper writes those frame cells); list it
        // only to extend `max_slot`, never to rank it (the triple stays in the
        // frame, so it competes for no register).
        MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([vec_slot, ptr_slot, len_slot])
        }
        // A str-append reads its handle slot (forced frame-resident below — the
        // helper indexes it) and, for the byte form, the value slot (ranked: it
        // may be register-resident). The const form has no extra slot.
        MicroOp::StrAppend { text_handle_slot, src, .. } => {
            out.push(text_handle_slot);
            if let crate::jit::StrSrc::Byte(s) = src {
                out.push(s);
            }
        }
        // The mode-B precise self-call writes `dst` and reads `limit_slot` (the
        // arena bound). The callee window (args_start..) is forced frame-
        // resident below; the staging Moves into it account those slots already,
        // and `extend_self_call_slots` is not used (the disjoint window's extent
        // is covered by the Move ops the adapter emits into it).
        MicroOp::Call { dst, limit_slot, .. } => out.extend([dst, limit_slot]),
        // A triple-plant reads `handle_slot` and writes the vec/ptr/len triple
        // (all forced frame-resident); `handle_slot` is ranked so a register-
        // resident handle is still read correctly (the codegen spills it to its
        // forced frame cell so the helper sees it).
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([handle_slot, vec_slot, ptr_slot, len_slot])
        }
        // A self-call writes `dst` and reads `limit_slot` (the arena bound). The
        // callee window (args_start..) and a `CallSelfCopy`'s source block
        // (src_start..) are accounted by `extend_self_call_slots` so `max_slot`
        // covers them; the staging slots stay FRAME-resident (the callee reads
        // them from the frame) and so are NOT ranked for a register here.
        MicroOp::CallSelf { dst, limit_slot, .. } => out.extend([dst, limit_slot]),
        MicroOp::CallSelfCopy { dst, src_start, arg_count, limit_slot, .. } => {
            out.extend([dst, limit_slot]);
            // The source args are READ; rank them (they may be register-resident
            // — the codegen reads them from wherever they live, then stages).
            for j in 0..arg_count {
                out.push(src_start + j);
            }
        }
        MicroOp::Jump { .. } => {}
        _ => {}
    }
}

/// The control-flow TARGET of a transfer op (`Jump`/`JumpIf*`/`Branch*`), if
/// any. A target whose index is `<` the op's own index is a loop BACK-EDGE.
fn op_target(op: &MicroOp) -> Option<usize> {
    match *op {
        MicroOp::Jump { target }
        | MicroOp::JumpIfFalse { target, .. }
        | MicroOp::JumpIfTrue { target, .. }
        | MicroOp::Branch { target, .. }
        | MicroOp::BranchF { target, .. } => Some(target),
        _ => None,
    }
}

/// The LOOP NESTING DEPTH of every op in the region. A loop is the span
/// `[target, idx]` of a BACK-EDGE — a transfer op at `idx` whose `target <= idx`
/// (so its body runs repeatedly). The depth of op `j` is the number of such
/// spans that contain `j`; ops inside a doubly-nested loop have depth 2, etc.
/// This is a structural over-approximation (it counts every back-edge span, not
/// just reducible natural loops), which is exactly what the spill heuristic
/// wants: a value referenced under a back-edge is hot and should win a register.
fn loop_depths(ops: &[MicroOp]) -> Vec<u32> {
    let n = ops.len();
    let mut depth = vec![0u32; n];
    for (idx, op) in ops.iter().enumerate() {
        if let Some(target) = op_target(op) {
            if target <= idx {
                for d in depth.iter_mut().take(idx + 1).skip(target) {
                    *d += 1;
                }
            }
        }
    }
    depth
}

/// Does the region contain a SCALED-INDEX CSE opportunity — two or more array
/// accesses (`ArrLoad`/`ArrStore`) sharing one index slot AND element stride
/// across a straight-line run, with NO intervening invalidation? This is the
/// gate for reserving a cache register: without a reusable run the CSE never
/// fires, so the reservation (which spills the coldest int scalar to the frame)
/// would be a pure cost. The invalidation here MIRRORS the runtime exactly —
/// the cache is dropped at a jump TARGET (control join), when the cached index
/// slot is reassigned, and across a SysV call — so this returns `true` iff at
/// least one access would HIT the cache the codegen builds.
fn has_shared_index_run(ops: &[MicroOp]) -> bool {
    // Precompute jump-target indices (control joins drop the cache).
    let mut is_target = vec![false; ops.len()];
    for op in ops {
        if let Some(t) = op_target(op) {
            if let Some(slot) = is_target.get_mut(t) {
                *slot = true;
            }
        }
    }
    let mut cache: Option<(Slot, bool)> = None; // (idx, byte)
    for (i, op) in ops.iter().enumerate() {
        if is_target[i] {
            cache = None;
        }
        match *op {
            MicroOp::ArrLoad { idx, byte, .. } | MicroOp::ArrStore { idx, byte, .. } => {
                if cache == Some((idx, byte)) {
                    return true; // a reuse hit — the run pays off.
                }
                cache = Some((idx, byte));
            }
            _ => {}
        }
        // Post-op invalidation, mirroring the codegen: a SysV call clobbers the
        // reserved caller-saved register; a write to the cached index slot makes
        // the held `im1` stale.
        if is_sysv_call(op) {
            cache = None;
        } else if let Some((idx, _)) = cache {
            if dest_of(op) == Some(idx) {
                cache = None;
            }
        }
    }
    false
}

/// Whether `op` is a REAL SysV `call` (it clobbers the caller-saved registers per
/// the ABI). This is the universe of ops the call-weight ranking and the
/// per-call-site spill machinery key off: the self-call families (`CallSelf`,
/// `CallSelfCopy`, the mode-B precise `Call`) and the list/string-mutation
/// helpers (`NewList`, `ArrPush`, `ListClear`, `ListTriple`, `StrAppend`), each
/// of which enters JIT-runtime code through a SysV `call`. Mirrors the
/// `has_self_call || has_list_call || has_precise_call` disjunction that defines
/// `has_call`.
fn is_sysv_call(op: &MicroOp) -> bool {
    matches!(
        op,
        MicroOp::CallSelf { .. }
            | MicroOp::CallSelfCopy { .. }
            | MicroOp::Call { .. }
            | MicroOp::NewList { .. }
            | MicroOp::ArrPush { .. }
            | MicroOp::ListClear { .. }
            | MicroOp::ListTriple { .. }
            | MicroOp::StrAppend { .. }
    )
}

/// A loop-invariant array ptr/len hoist (Wave 27a): one array, invariant over one
/// loop span, whose handle slots are loaded ONCE at the loop pre-header into
/// persistent callee-saved registers and reused for every in-span access.
#[derive(Clone, Copy)]
struct HoistPlan {
    /// The loop head op index (the back-edge target). The hoist loads fire at this
    /// op's PRE-HEADER (just before binding the head's label), reached only by the
    /// entry fall-through — the back-edge jumps to the bound label and skips them.
    head: usize,
    /// The last op index INSIDE the loop (the back-edge op). The hoist registers
    /// are live (used by `emit_arr_addr`) for ops in `head..=back`; outside this
    /// span the array falls back to the per-access frame reload.
    back: usize,
    /// The frame-resident ptr slot loaded once and reused for every in-span access.
    ptr_slot: Slot,
    /// The len slot, loaded once and reused for every CHECKED in-span access; only
    /// loaded when `needs_len` (some in-span access of this array is checked).
    len_slot: Slot,
    /// Whether any in-span access of this array is CHECKED (so `len` must hoist).
    needs_len: bool,
    /// How many `ArrLoad`/`ArrStore` of this array occur in the span — the reuse
    /// count the hoist amortizes (one pre-header load serves all of them).
    accesses: u32,
}

/// Whether op `op` WRITES the frame cell of slot `s` — either it is the op's
/// destination (a scalar def, an `ArrLoad` dst, a self-call result), OR a
/// list-mutation helper REFRESHES it through the frame (`ArrPush`/`NewList`/
/// `ListClear`/`ListTriple` write `frame[vec/ptr/len]` after a possible realloc).
/// An array whose ptr/len slot is written this way INSIDE a loop is NOT invariant
/// there and must not be hoisted (its hoisted register would go stale).
fn op_writes_slot(op: &MicroOp, s: Slot) -> bool {
    if dest_of(op) == Some(s) {
        return true;
    }
    match *op {
        MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. }
        | MicroOp::ArrPush { vec_slot, ptr_slot, len_slot, .. }
        | MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
            s == vec_slot || s == ptr_slot || s == len_slot
        }
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
            s == handle_slot || s == vec_slot || s == ptr_slot || s == len_slot
        }
        _ => false,
    }
}

/// Identify every loop-invariant array ptr/len hoist available in `ops`. A
/// candidate is one (`ptr_slot`, `len_slot`) pair that:
///
///  1. is accessed by an `ArrLoad`/`ArrStore` INSIDE a natural loop span
///     `[head, back]` (a back-edge whose `target == head <= back`);
///  2. has NEITHER slot written anywhere inside `[head, back]` (no scalar def, no
///     reallocating push/clear/triple of that array) — so `frame[ptr]`/`frame[len]`
///     are constant across the whole span;
///  3. enters the loop ONLY via the pre-header fall-through — every transfer that
///     targets `head` originates INSIDE `[head, back]` (a back-edge). A forward
///     jump into `head` from outside would skip the pre-header load and read a
///     stale register, so such a loop is rejected (no hoist).
///
/// Frame-residency is NOT decided here (the analysis is purely structural over the
/// op stream); the caller filters to slots that are actually `Loc::Frame` before
/// reserving a register (a slot the linear scan already keeps in a register has
/// no per-access reload to hoist).
///
/// For nested loops the INNERMOST span containing an access is preferred: its
/// pre-header runs once per outer iteration (the highest reuse), and a slot
/// invariant in the inner loop may be written between inner loops at the outer
/// level (knapsack's `Set prev to curr`), so the outer span would not be
/// invariant. We therefore emit one plan per (innermost-loop, array) pair.
fn loop_invariant_array_hoists(ops: &[MicroOp]) -> Vec<HoistPlan> {
    // Every natural-loop span: a back-edge at `back` with `target == head <= back`.
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (back, op) in ops.iter().enumerate() {
        if let Some(head) = op_target(op) {
            if head <= back {
                loops.push((head, back));
            }
        }
    }
    let mut plans: Vec<HoistPlan> = Vec::new();
    for &(head, back) in &loops {
        // Entry-discipline guard: every transfer targeting `head` must originate
        // inside `[head, back]` (a back-edge). A forward jump into `head` from
        // outside the span would skip the pre-header load → reject this loop.
        let entry_ok = ops.iter().enumerate().all(|(i, op)| match op_target(op) {
            Some(t) if t == head => (head..=back).contains(&i),
            _ => true,
        });
        if !entry_ok {
            continue;
        }
        // The array (ptr, len) pairs accessed inside the span, and whether any
        // access of each is checked. A pair is keyed by ptr_slot (the address
        // base); the len_slot rides along (a checked access of a given array
        // always names the same len_slot — the lowering pins one handle triple
        // per array).
        let mut accessed: std::collections::BTreeMap<Slot, (Slot, bool, u32)> =
            std::collections::BTreeMap::new();
        for op in &ops[head..=back] {
            match *op {
                MicroOp::ArrLoad { ptr_slot, len_slot, checked, .. }
                | MicroOp::ArrStore { ptr_slot, len_slot, checked, .. } => {
                    let e = accessed.entry(ptr_slot).or_insert((len_slot, false, 0));
                    e.1 |= checked;
                    e.2 += 1;
                }
                _ => {}
            }
        }
        for (ptr_slot, (len_slot, any_checked, accesses)) in accessed {
            // Invariance: neither handle slot is written anywhere in the span.
            let invariant = !ops[head..=back]
                .iter()
                .any(|op| op_writes_slot(op, ptr_slot) || op_writes_slot(op, len_slot));
            if invariant {
                plans.push(HoistPlan {
                    head,
                    back,
                    ptr_slot,
                    len_slot,
                    needs_len: any_checked,
                    accesses,
                });
            }
        }
    }
    // For an array accessed in several NESTED spans, keep only the innermost (the
    // smallest span) so the pre-header sits at the tightest loop (highest reuse)
    // and no two plans claim the same array with overlapping spans. Two disjoint
    // loops touching the same array each keep their own plan.
    plans.sort_by_key(|p| (p.ptr_slot, p.back - p.head));
    let mut chosen: Vec<HoistPlan> = Vec::new();
    for p in plans {
        let overlaps_kept = chosen.iter().any(|c| {
            c.ptr_slot == p.ptr_slot && c.head <= p.back && p.head <= c.back
        });
        if !overlaps_kept {
            chosen.push(p);
        }
    }
    chosen
}

/// A loop-invariant constant hoist (Wave 28): one `LoadConst` op inside a loop
/// whose `dst` is invariant across the span, materialized ONCE at the loop
/// pre-header instead of every iteration.
#[derive(Clone, Copy)]
struct ConstHoist {
    /// The op index of the `LoadConst` being hoisted. The emit loop SKIPS this op
    /// inside the span (the pre-header already loaded its register).
    op: usize,
    /// The loop head op index (the back-edge target). The hoist materialization
    /// fires at this op's PRE-HEADER, reached only by the entry fall-through.
    head: usize,
    /// The destination slot the constant lands in (register-resident).
    dst: Slot,
    /// The raw 64-bit constant value (`MicroOp::LoadConst::value`).
    value: i64,
}

/// Identify every loop-invariant `LoadConst` hoist available in `ops`. A
/// `LoadConst { dst, value }` at op index `i` is a candidate when:
///
///  1. it sits INSIDE a natural loop span `[head, back]` (a back-edge whose
///     `target == head <= back`) — the INNERMOST such span containing `i` (the
///     tightest pre-header, the highest reuse: mandelbrot's `2.0`/`4.0` sit in
///     the 50-iteration inner loop);
///  2. the loop is entered ONLY via the pre-header fall-through (every transfer
///     targeting `head` originates inside `[head, back]`) — otherwise a forward
///     jump into `head` would skip the pre-header materialization and read a
///     stale register;
///  3. `dst` is written by NO OTHER op anywhere in `[head, back]` (the ONLY
///     writer in the span is this `LoadConst`) — so the register holds the same
///     constant bits on every iteration. A slot reused for two different
///     constants (a scratch `LoadConst {dst,0}` then `LoadConst {dst,1}`) has two
///     writers and is therefore NOT hoisted.
///
/// Register-residency is NOT decided here (the analysis is purely structural over
/// the op stream); the caller filters to `dst`s that are actually `Loc::Reg`/
/// `Loc::Xmm` before materializing (a frame-resident const has no per-iteration
/// register reload to amortize).
fn loop_invariant_const_hoists(ops: &[MicroOp]) -> Vec<ConstHoist> {
    // Every natural-loop span: a back-edge at `back` with `target == head <= back`.
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (back, op) in ops.iter().enumerate() {
        if let Some(head) = op_target(op) {
            if head <= back {
                loops.push((head, back));
            }
        }
    }
    let mut chosen: Vec<ConstHoist> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let MicroOp::LoadConst { dst, value } = *op else { continue };
        // The INNERMOST loop span that contains this op (smallest span). A const is
        // invariant in every enclosing loop, but the innermost pre-header runs the
        // most often, and an enclosing loop might write `dst` between inner loops.
        let inner = loops
            .iter()
            .copied()
            .filter(|&(head, back)| (head..=back).contains(&i))
            .min_by_key(|&(head, back)| back - head);
        let Some((head, back)) = inner else { continue };
        // Entry-discipline: every transfer targeting `head` must originate inside
        // `[head, back]`. A forward jump into `head` from outside would skip the
        // pre-header materialization.
        let entry_ok = ops.iter().enumerate().all(|(j, o)| match op_target(o) {
            Some(t) if t == head => (head..=back).contains(&j),
            _ => true,
        });
        if !entry_ok {
            continue;
        }
        // Invariance: NO other op in the span writes `dst` (this LoadConst is the
        // sole writer). A slot reused for two constants has a second writer here.
        let sole_writer = ops[head..=back]
            .iter()
            .enumerate()
            .all(|(off, o)| head + off == i || dest_of(o) != Some(dst));
        if sole_writer {
            chosen.push(ConstHoist { op: i, head, dst, value });
        }
    }
    chosen
}

/// The largest slot index the region touches, INCLUDING each self-call window's
/// full extent (`args_start + frame_size - 1`). The frame indexing allocates
/// `loc`/`written`/liveness bitvectors up to this bound, so it must cover the
/// callee window staging slots even though those stay frame-resident.
fn max_slot_of(ops: &[MicroOp]) -> usize {
    let mut max_slot: usize = 0;
    let mut buf = Vec::new();
    for op in ops {
        buf.clear();
        slots_of(op, &mut buf);
        for &s in &buf {
            max_slot = max_slot.max(s as usize);
        }
        if let MicroOp::CallSelf { args_start, frame_size, .. }
        | MicroOp::CallSelfCopy { args_start, frame_size, .. } = *op
        {
            let last = (args_start as i64) + frame_size - 1;
            if last >= 0 {
                max_slot = max_slot.max(last as usize);
            }
        }
    }
    max_slot
}

/// Loop-weighted reference ranking with a CALL-LOCALITY tie-break.
///
/// The PRIMARY key is the loop-weighted reference count: each reference at loop
/// depth `d` is worth `base^depth`, so a loop-carried value out-ranks a colder
/// slot referenced more times outside any loop. This key — and therefore the
/// RESIDENT-vs-SPILLED boundary it induces (the first `pool.len()` slots are
/// resident) — is EXACTLY the pre-Fix-1 ranking. The call bonus is a SECONDARY
/// key only, so it can NEVER evict a hotter loop slot from a register: it merely
/// reorders slots that would be resident anyway.
///
/// The SECONDARY key prices CALL-LOCALITY. A slot LIVE ACROSS a SysV call (in
/// `live_after[call_idx]`) pays a caller-saved spill+reload pair on EVERY
/// execution of that call unless it lives in a callee-saved register. The bonus
/// is `call_weight * base^depth(call)` per surviving call (a call inside a hot
/// loop pays the pair per iteration). Because the `has_call` pool puts the four
/// CALLEE-SAVED registers FIRST, and a slot's callee-vs-caller placement is
/// decided by its rank position, lifting call-survivors above call-dead slots OF
/// EQUAL LOOP WEIGHT steers exactly the survivors into the callee-saved regs —
/// eliminating their per-call reload for a one-time prologue push/pop — without
/// disturbing which slots are resident at all.
///
/// This ONLY changes which physical register a resident slot occupies (and, on a
/// primary-key tie, which equally-cold slot spills). A slot still lives in
/// exactly one place for the whole function and its value never changes, so the
/// assignment is bit-identical to any other ranking. `live_after` is `None` for a
/// call-free region (no bonus). Returns the order (descending by the composite
/// key, then ascending by slot for determinism) and the observed `max_slot`. The
/// returned per-slot `u64` is the PRIMARY loop-weight (the secondary key is
/// internal to the sort) — downstream consumes only the slot ORDER.
fn rank_slots(
    ops: &[MicroOp],
    loop_weight_base: u64,
    loop_depth_cap: u32,
    call_weight: u64,
    live_after: Option<&[Vec<bool>]>,
) -> (Vec<(Slot, u64)>, usize) {
    let depths = loop_depths(ops);
    let max_slot = max_slot_of(ops);
    // PRIMARY: loop-weighted reference count.
    let mut refs: std::collections::HashMap<Slot, u64> = std::collections::HashMap::new();
    // SECONDARY: call-survival weight (a separate key, never folded into refs).
    let mut call_surv: std::collections::HashMap<Slot, u64> = std::collections::HashMap::new();
    let mut buf = Vec::new();
    for (idx, op) in ops.iter().enumerate() {
        let weight = loop_weight_base.saturating_pow(depths[idx].min(loop_depth_cap));
        buf.clear();
        slots_of(op, &mut buf);
        for &s in &buf {
            let e = refs.entry(s).or_insert(0);
            *e = e.saturating_add(weight);
        }
    }
    if let Some(la) = live_after {
        for (idx, op) in ops.iter().enumerate() {
            if !is_sysv_call(op) {
                continue;
            }
            let call_depth_weight =
                loop_weight_base.saturating_pow(depths[idx].min(loop_depth_cap));
            let bonus = call_weight.saturating_mul(call_depth_weight);
            for (s, &live) in la[idx].iter().enumerate() {
                if live {
                    // Only slots that ALSO carry loop/ref weight can ever be
                    // resident; recording survival for a slot with no refs is
                    // harmless (it sorts last on the primary key regardless).
                    let e = call_surv.entry(s as Slot).or_insert(0);
                    *e = e.saturating_add(bonus);
                }
            }
        }
    }
    let mut order: Vec<(Slot, u64)> = refs.iter().map(|(&s, &c)| (s, c)).collect();
    order.sort_by(|a, b| {
        // PRIMARY desc, then call-survival desc (the tie-break that picks
        // callee-saved for the survivor), then slot asc for determinism.
        b.1.cmp(&a.1)
            .then_with(|| call_surv.get(&b.0).cmp(&call_surv.get(&a.0)))
            .then(a.0.cmp(&b.0))
    });
    (order, max_slot)
}

/// Every slot an op READS (its use set), for backward liveness. This enumerates
/// the READ operands by POSITION (not by value), so a self-referential op like
/// `Add { dst: i, lhs: i, rhs: 1 }` (`i = i + 1`) correctly reports `i` as a USE
/// even though it is also the destination — a value-based "all-slots minus dst"
/// would wrongly drop it. Every op in the supported subset has a pure write-only
/// destination (the operands `lhs`/`rhs`/`src`/`a`/`b`/`c`/`idx`/`cond`/… are the
/// reads); `dst` is never itself read by the op. A self-call READS `limit_slot`
/// (and a `CallSelfCopy`'s source args) and WRITES `dst`. An
/// `ArrStore`/`ArrPush`/`ListClear`/`ListTriple` has no `dst`, so all its slots
/// are reads. Used ONLY by the per-call-site spill-liveness lever (a pure
/// scheduling optimization): a missed read here would be UNSOUND, so this is the
/// authoritative read set — derived as `slots_of` (read ∪ write) minus the
/// POSITIONAL destination, by removing exactly the dst's first occurrence is
/// wrong; instead we list reads directly.
fn read_slots_of(op: &MicroOp, out: &mut Vec<Slot>) {
    match *op {
        MicroOp::Move { src, .. } | MicroOp::NotInt { src, .. } | MicroOp::NotBool { src, .. } => {
            out.push(src)
        }
        MicroOp::LoadConst { .. } => {}
        MicroOp::Add { lhs, rhs, .. }
        | MicroOp::Sub { lhs, rhs, .. }
        | MicroOp::Mul { lhs, rhs, .. }
        | MicroOp::Lt { lhs, rhs, .. }
        | MicroOp::Gt { lhs, rhs, .. }
        | MicroOp::LtEq { lhs, rhs, .. }
        | MicroOp::GtEq { lhs, rhs, .. }
        | MicroOp::Eq { lhs, rhs, .. }
        | MicroOp::Neq { lhs, rhs, .. }
        | MicroOp::BitAnd { lhs, rhs, .. }
        | MicroOp::BitOr { lhs, rhs, .. }
        | MicroOp::BitXor { lhs, rhs, .. }
        | MicroOp::Shl { lhs, rhs, .. }
        | MicroOp::Shr { lhs, rhs, .. }
        | MicroOp::Div { lhs, rhs, .. }
        | MicroOp::Mod { lhs, rhs, .. }
        | MicroOp::AddF { lhs, rhs, .. }
        | MicroOp::SubF { lhs, rhs, .. }
        | MicroOp::MulF { lhs, rhs, .. }
        | MicroOp::DivF { lhs, rhs, .. }
        | MicroOp::LtF { lhs, rhs, .. }
        | MicroOp::GtF { lhs, rhs, .. }
        | MicroOp::LtEqF { lhs, rhs, .. }
        | MicroOp::GtEqF { lhs, rhs, .. }
        | MicroOp::EqF { lhs, rhs, .. }
        | MicroOp::NeqF { lhs, rhs, .. } => out.extend([lhs, rhs]),
        MicroOp::FmaF { a, b, c, .. } => out.extend([a, b, c]),
        MicroOp::IntToFloat { src, .. } | MicroOp::SqrtF { src, .. } => out.push(src),
        MicroOp::DivPow2 { lhs, .. } => out.push(lhs),
        MicroOp::MagicDivU { lhs, .. } => out.push(lhs),
        MicroOp::Branch { lhs, rhs, .. } | MicroOp::BranchF { lhs, rhs, .. } => out.extend([lhs, rhs]),
        MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => out.push(cond),
        MicroOp::Return { src } => out.push(src),
        MicroOp::ArrLoad { idx, ptr_slot, len_slot, checked, .. } => {
            out.extend([idx, ptr_slot]);
            if checked {
                out.push(len_slot);
            }
        }
        MicroOp::ArrStore { src, idx, ptr_slot, len_slot, checked, .. } => {
            out.extend([src, idx, ptr_slot]);
            if checked {
                out.push(len_slot);
            }
        }
        MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([src, vec_slot, ptr_slot, len_slot])
        }
        MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([vec_slot, ptr_slot, len_slot])
        }
        MicroOp::StrAppend { text_handle_slot, src, .. } => {
            out.push(text_handle_slot);
            if let crate::jit::StrSrc::Byte(s) = src {
                out.push(s);
            }
        }
        MicroOp::Call { limit_slot, .. } => out.push(limit_slot),
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
            out.extend([handle_slot, vec_slot, ptr_slot, len_slot])
        }
        MicroOp::CallSelf { limit_slot, .. } => out.push(limit_slot),
        MicroOp::CallSelfCopy { src_start, arg_count, limit_slot, .. } => {
            out.push(limit_slot);
            for j in 0..arg_count {
                out.push(src_start + j);
            }
        }
        MicroOp::Jump { .. } => {}
        _ => {}
    }
}

/// Backward LIVENESS: `live_after[i]` is the set of slots that may be READ on
/// some control-flow path AFTER op `i` finishes (before being overwritten). A
/// standard iterative backward dataflow to a fixpoint over the op stream's CFG
/// (fall-through `i+1` plus the branch/jump TARGET of `i`):
///
///   live_in[i]  = uses(i) ∪ (live_out[i] \ {def(i)})
///   live_out[i] = ⋃ live_in[succ]   over the successors of i
///
/// This is a SOUND OVER-approximation of "definitely dead after the call": every
/// slot a later op (on any path, including a loop back-edge) reads is marked
/// live, and an unconditional `Jump`/`Return` has no fall-through successor. The
/// result drives the spill-liveness lever, which spills/reloads only the
/// caller-saved residents that are live AFTER a call site; a slot NOT in
/// `live_after[call]` is read by no later op on any path (so the success-path
/// reload is dead) and is never observed by a deopt resume either (classic
/// replay recomputes mid-call temps from the boundary args; a precise resume at
/// a later op re-boxes only its own live slots — a subset of these). Eliding its
/// spill is therefore bit-identical. Returns one bitset (`Vec<bool>` over
/// `0..=max_slot`) per op.
fn liveness_after(ops: &[MicroOp], max_slot: usize) -> Vec<Vec<bool>> {
    let n = ops.len();
    let width = max_slot + 1;
    let mut live_in: Vec<Vec<bool>> = vec![vec![false; width]; n];
    // Precompute each op's use set and def (stable across iterations).
    let mut uses: Vec<Vec<Slot>> = Vec::with_capacity(n);
    let mut defs: Vec<Option<Slot>> = Vec::with_capacity(n);
    for op in ops {
        let mut u = Vec::new();
        read_slots_of(op, &mut u);
        uses.push(u);
        defs.push(dest_of(op));
    }

    let mut changed = true;
    while changed {
        changed = false;
        // Process in reverse for faster convergence (data flows backward).
        for i in (0..n).rev() {
            // live_out[i] = union of live_in over successors.
            let mut out = vec![false; width];
            // Fall-through successor (i+1) unless this op is an unconditional
            // transfer (Jump has no fall-through; Return terminates).
            let unconditional_transfer = matches!(ops[i], MicroOp::Jump { .. } | MicroOp::Return { .. });
            if !unconditional_transfer {
                if let Some(succ) = live_in.get(i + 1) {
                    for (o, &s) in out.iter_mut().zip(succ.iter()) {
                        *o |= s;
                    }
                }
            }
            // Branch/jump target successor.
            if let Some(t) = op_target(&ops[i]) {
                if let Some(succ) = live_in.get(t) {
                    for (o, &s) in out.iter_mut().zip(succ.iter()) {
                        *o |= s;
                    }
                }
            }
            // live_in[i] = uses ∪ (live_out \ def).
            let mut new_in = out.clone();
            if let Some(d) = defs[i] {
                if (d as usize) < width {
                    new_in[d as usize] = false;
                }
            }
            for &u in &uses[i] {
                if (u as usize) < width {
                    new_in[u as usize] = true;
                }
            }
            if new_in != live_in[i] {
                live_in[i] = new_in;
                changed = true;
            }
        }
    }

    // live_after[i] == live_out[i]: rebuild from the converged live_in.
    let mut live_after: Vec<Vec<bool>> = vec![vec![false; width]; n];
    for i in 0..n {
        let unconditional_transfer = matches!(ops[i], MicroOp::Jump { .. } | MicroOp::Return { .. });
        if !unconditional_transfer {
            if let Some(succ) = live_in.get(i + 1) {
                for (o, &s) in live_after[i].iter_mut().zip(succ.iter()) {
                    *o |= s;
                }
            }
        }
        if let Some(t) = op_target(&ops[i]) {
            if let Some(succ) = live_in.get(t) {
                for (o, &s) in live_after[i].iter_mut().zip(succ.iter()) {
                    *o |= s;
                }
            }
        }
    }
    live_after
}

/// The condition for a `Cmp`.
fn cond_of(cmp: Cmp) -> Cond {
    match cmp {
        Cmp::Lt => Cond::Lt,
        Cmp::Gt => Cond::Gt,
        Cmp::LtEq => Cond::Le,
        Cmp::GtEq => Cond::Ge,
        Cmp::Eq => Cond::Eq,
        Cmp::NotEq => Cond::Ne,
    }
}

/// The register CLASS a slot may be resident in. A slot that appears in BOTH an
/// integer role and a float role (or only ever as a raw-bits/neutral operand) is
/// kept frame-resident, where the two classes share the same 8 raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Used only in integer roles → eligible for a GP register.
    Int,
    /// Used only in float (f64) roles → eligible for an XMM register.
    Float,
    /// Used in both classes (or class-ambiguous): frame-resident only.
    Mixed,
}

/// Per-slot role tally for [`Class`] assignment.
#[derive(Clone, Copy, Default)]
struct Roles {
    int: bool,
    float: bool,
}

/// Mark every slot an op touches with its INT and/or FLOAT roles. Type-NEUTRAL
/// roles (a `Move`/`LoadConst`/`Return` operand, an `ArrLoad` dst / `ArrStore`
/// src — all raw-bits transfers) impose NO class; the slot's class comes from
/// the strongly-typed ops it also appears in. A slot with no typed role at all
/// defaults to INT (the historical behavior).
fn tally_roles(op: &MicroOp, roles: &mut [Roles]) {
    macro_rules! mark_int {
        ($s:expr) => {
            roles[$s as usize].int = true
        };
    }
    match *op {
        // Integer arithmetic / compare / bitwise / shift: all operands + dst int.
        MicroOp::Add { dst, lhs, rhs }
        | MicroOp::Sub { dst, lhs, rhs }
        | MicroOp::Mul { dst, lhs, rhs }
        | MicroOp::Div { dst, lhs, rhs }
        | MicroOp::Mod { dst, lhs, rhs }
        | MicroOp::Lt { dst, lhs, rhs }
        | MicroOp::Gt { dst, lhs, rhs }
        | MicroOp::LtEq { dst, lhs, rhs }
        | MicroOp::GtEq { dst, lhs, rhs }
        | MicroOp::Eq { dst, lhs, rhs }
        | MicroOp::Neq { dst, lhs, rhs }
        | MicroOp::BitAnd { dst, lhs, rhs }
        | MicroOp::BitOr { dst, lhs, rhs }
        | MicroOp::BitXor { dst, lhs, rhs }
        | MicroOp::Shl { dst, lhs, rhs }
        | MicroOp::Shr { dst, lhs, rhs } => {
            mark_int!(dst);
            mark_int!(lhs);
            mark_int!(rhs);
        }
        MicroOp::DivPow2 { dst, lhs, .. } | MicroOp::MagicDivU { dst, lhs, .. } => {
            mark_int!(dst);
            mark_int!(lhs);
        }
        MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src } => {
            mark_int!(dst);
            mark_int!(src);
        }
        MicroOp::Branch { lhs, rhs, .. } => {
            mark_int!(lhs);
            mark_int!(rhs);
        }
        MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => mark_int!(cond),
        // Array address machinery is INT; the loaded/stored value is NEUTRAL.
        MicroOp::ArrLoad { idx, ptr_slot, len_slot, checked, .. } => {
            mark_int!(idx);
            mark_int!(ptr_slot);
            if checked {
                mark_int!(len_slot);
            }
        }
        MicroOp::ArrStore { idx, ptr_slot, len_slot, checked, .. } => {
            mark_int!(idx);
            mark_int!(ptr_slot);
            if checked {
                mark_int!(len_slot);
            }
        }
        // The list-mutation handle slots (vec/ptr/len) are INT machinery; the
        // pushed VALUE is NEUTRAL (its raw bits travel to the buffer regardless
        // of class — a float-list push bit-copies through a GP reg). The handle
        // slots are also forced frame-resident, so their class is moot, but
        // marking them INT keeps the role tally accurate.
        MicroOp::ArrPush { vec_slot, ptr_slot, len_slot, .. } => {
            mark_int!(vec_slot);
            mark_int!(ptr_slot);
            mark_int!(len_slot);
        }
        MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
            mark_int!(vec_slot);
            mark_int!(ptr_slot);
            mark_int!(len_slot);
        }
        // Float arithmetic: all operands + dst float.
        MicroOp::AddF { dst, lhs, rhs }
        | MicroOp::SubF { dst, lhs, rhs }
        | MicroOp::MulF { dst, lhs, rhs }
        | MicroOp::DivF { dst, lhs, rhs } => {
            roles[dst as usize].float = true;
            roles[lhs as usize].float = true;
            roles[rhs as usize].float = true;
        }
        // Float compares: operands float, RESULT (dst) is an int 0/1.
        MicroOp::LtF { dst, lhs, rhs }
        | MicroOp::GtF { dst, lhs, rhs }
        | MicroOp::LtEqF { dst, lhs, rhs }
        | MicroOp::GtEqF { dst, lhs, rhs }
        | MicroOp::EqF { dst, lhs, rhs }
        | MicroOp::NeqF { dst, lhs, rhs } => {
            roles[dst as usize].int = true;
            roles[lhs as usize].float = true;
            roles[rhs as usize].float = true;
        }
        MicroOp::BranchF { lhs, rhs, .. } => {
            roles[lhs as usize].float = true;
            roles[rhs as usize].float = true;
        }
        MicroOp::FmaF { dst, a, b, c } => {
            roles[dst as usize].float = true;
            roles[a as usize].float = true;
            roles[b as usize].float = true;
            roles[c as usize].float = true;
        }
        MicroOp::SqrtF { dst, src } => {
            roles[dst as usize].float = true;
            roles[src as usize].float = true;
        }
        // IntToFloat: src is INT, dst is FLOAT.
        MicroOp::IntToFloat { dst, src } => {
            roles[dst as usize].float = true;
            mark_int!(src);
        }
        // A self-call: `dst` (the result) and `limit_slot` (the arena bound) are
        // INT. The staged scalar args are 8-byte raw copies (NEUTRAL — the copy
        // preserves their bits regardless of class), so they impose no class
        // here; their class comes from the ops that produced them.
        MicroOp::CallSelf { dst, limit_slot, .. } => {
            mark_int!(dst);
            mark_int!(limit_slot);
        }
        MicroOp::CallSelfCopy { dst, limit_slot, .. } => {
            mark_int!(dst);
            mark_int!(limit_slot);
        }
        // The mode-B precise self-call: `dst` (the result handle, an i64) and
        // `limit_slot` are INT. The staged args are NEUTRAL raw copies (their
        // class comes from the producing ops).
        MicroOp::Call { dst, limit_slot, .. } => {
            mark_int!(dst);
            mark_int!(limit_slot);
        }
        // List-machinery handle/triple slots are INT (raw `*mut Vec` words);
        // they are also forced frame-resident, so the class is moot, but the
        // tally stays accurate.
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
            mark_int!(handle_slot);
            mark_int!(vec_slot);
            mark_int!(ptr_slot);
            mark_int!(len_slot);
        }
        // The str-append handle (a raw `*mut Value` word) and a byte-form value
        // (an ASCII byte) are INT. The handle is forced frame-resident, so its
        // class is moot, but the tally stays accurate.
        MicroOp::StrAppend { text_handle_slot, src, .. } => {
            mark_int!(text_handle_slot);
            if let crate::jit::StrSrc::Byte(s) = src {
                mark_int!(s);
            }
        }
        // Move / LoadConst / Return are NEUTRAL — no class imposed.
        MicroOp::Move { .. }
        | MicroOp::LoadConst { .. }
        | MicroOp::Return { .. }
        | MicroOp::Jump { .. } => {}
        _ => {}
    }
}

/// Classify every slot up to `max_slot` into its register [`Class`].
fn classify_slots(ops: &[MicroOp], max_slot: usize) -> Vec<Class> {
    let mut roles = vec![Roles::default(); max_slot + 1];
    for op in ops {
        tally_roles(op, &mut roles);
    }
    roles
        .iter()
        .map(|r| match (r.int, r.float) {
            (_, true) if r.int => Class::Mixed,
            (false, true) => Class::Float,
            // int-only OR neutral-only (default int).
            _ => Class::Int,
        })
        .collect()
}

/// Per-self-call support: the label of the "bare return" epilogue that a
/// PROPAGATED in-callee deopt jumps to — flush the frame and return WITHOUT
/// touching the status cell (which already holds the inner exit code). The
/// per-call-site caller-saved spill/reload lists live on [`Gen`]
/// (`call_gp`/`call_xmm`), shared with the list-mutation helper-call path.
struct SelfCall {
    /// The bare flush+return epilogue (no status write) for deopt PROPAGATION.
    propagate_label: LabelId,
}

/// PRECISE (mode-B) deopt support. Every checked op and the precise self-call's
/// guards side-exit with the op's ENCODED resume tag (`(pc << 2) | 3`) OR'd with
/// the live call depth in the high 32 bits, mirroring `logos_stencil_deopt_at` /
/// `logos_stencil_call_precise`. A checked op routes its bounds/divisor failure
/// to the out-of-line block for its code (`code_blocks[code]`), which stages the
/// tag in `S1` and jumps to the shared `epilogue`. The epilogue reads the live
/// depth, ORs it into the high bits, writes the status cell, flushes every
/// resident-written slot (so the precise-deopt walk reads the right frame), and
/// returns.
struct Precise<'a> {
    /// The per-op resume codes (parallel to the op stream).
    codes: &'a [i64],
    /// Distinct non-plain code → its out-of-line tag-staging block label (each
    /// block stages the tag in `S1` and jumps to the shared precise epilogue).
    code_blocks: &'a std::collections::HashMap<i64, LabelId>,
}

/// The codegen context: the slot→location map plus the assembler.
struct Gen<'a> {
    asm: Asm,
    /// Per-slot location (GP reg / XMM reg / frame). The location already
    /// encodes the slot's register class, so Move/LoadConst/array transfers read
    /// `loc` directly to decide whether to bridge between the GP and XMM classes.
    loc: &'a [Loc],
    /// Op-index → label, for jump targets.
    op_labels: &'a [LabelId],
    /// The label of the deopt epilogue (set status = 1, flush, return), if any.
    deopt_label: Option<LabelId>,
    /// Stack-pointer realignment padding applied in the prologue (so RSP is
    /// 16-aligned at every `call`) and undone in every epilogue. 0 or 8.
    stack_pad: i32,
    /// Self-call support (present only for the FUNCTION backend with self-calls).
    self_call: Option<SelfCall>,
    /// PER-CALL-SITE caller-saved GP residents to spill/reload around the SysV
    /// call at op index `i` — `call_gp[i]` is the global caller-saved-GP-resident
    /// universe INTERSECTED with the slots LIVE AFTER op `i` (so a resident dead
    /// after the call is not spilled/reloaded). Empty for a non-call op. The XMM
    /// counterpart lives in `call_xmm`.
    call_gp: &'a [Vec<Slot>],
    /// PER-CALL-SITE XMM residents to spill/reload around the call at op `i` (all
    /// XMM are caller-saved). The live-after subset, parallel to `call_gp`.
    call_xmm: &'a [Vec<Slot>],
    /// PRECISE (mode-B) deopt support; `None` for the classic region/function
    /// paths (which replay from head). When `Some`, every checked op's side exit
    /// and the precise self-call's guards route through it.
    precise: Option<Precise<'a>>,
    /// SCALED-INDEX CSE (Wave 26). Reserved caller-saved register(s) that hold a
    /// run's shared 0-based index `im1 = idx - 1` (`im1_reg`) and, when a second
    /// register is free, its scaled byte offset `im1 * 8` (`scaled_reg`). Both
    /// are `None` when no caller-saved register is free (CSE disabled for the
    /// region) or the kill-switch is off. The reserved registers are never
    /// assigned to a resident slot and are touched by NOTHING but [`emit_arr_addr`],
    /// so they stay live across intervening arithmetic — invalidated only when the
    /// index slot is reassigned, at a jump target, or across a SysV call.
    off_im1_reg: Option<Reg>,
    /// The second reserved register holding the scaled byte offset, when free.
    off_scaled_reg: Option<Reg>,
    /// The currently-cached run, or `None` when the cache is invalid. `byte`
    /// records the element stride the offset was scaled for (1 vs 8); a different
    /// stride for the same index slot is a cache miss (recompute). `verified`
    /// records whether the FIRST access of the run bounds-checked `im1` (so a
    /// later checked access may reuse the cached scaled offset soundly).
    off_cache: Option<OffCacheState>,
    /// LOOP-INVARIANT PTR/LEN HOIST (Wave 27a). The per-plan reserved registers
    /// holding the invariant `frame[ptr_slot]` (and `frame[len_slot]`) for a loop
    /// span. Each entry is one [`HoistPlan`] plus the callee-saved registers it
    /// claimed. `emit_arr_addr` consults [`Gen::hoisted`] to read the register
    /// instead of reloading the frame, but ONLY while codegen is inside the plan's
    /// span — the active-set is keyed per op index in the emit loop.
    hoists: &'a [HoistEntry],
    /// The ptr/len slots whose hoist registers are ACTIVE at the current op (the
    /// codegen is inside their span). Reset/recomputed per op in the emit loop;
    /// `emit_arr_addr` reads it to decide whether a given ptr/len slot is hoisted
    /// here. Indexed lookups are over a tiny vec (at most a few hoists per loop).
    active_hoists: Vec<usize>,
}

/// A resolved [`HoistPlan`]: the plan plus the callee-saved registers reserved to
/// hold its invariant `frame[ptr_slot]` (`ptr_reg`) and, when `needs_len`, its
/// `frame[len_slot]` (`len_reg`). A plan that could not claim a register is not
/// materialized into a `HoistEntry` (it falls back to the per-access reload).
#[derive(Clone, Copy)]
struct HoistEntry {
    plan: HoistPlan,
    ptr_reg: Reg,
    len_reg: Option<Reg>,
}

/// The live scaled-index cache: the reserved registers hold `im1`/`im1*8` for
/// this `idx` slot at this element stride.
#[derive(Clone, Copy)]
struct OffCacheState {
    idx: Slot,
    byte: bool,
}

impl Gen<'_> {
    /// The label an op at index `idx` jumps to on a CHECKED side exit, or `None`
    /// when no deopt channel exists (an UNCHECKED op in a region with no checked
    /// op — the label is never referenced). In the classic paths this is the
    /// shared `deopt_label` (writes status = 1); in the PRECISE path it is the
    /// out-of-line tag-staging block for this op's code (which stages the precise
    /// tag and jumps to the precise epilogue). A precise op whose code is the
    /// plain marker `1` still uses `deopt_label`. The CHECKED-op emitters
    /// `.expect()` this where a missing label would be a real bug; the unchecked
    /// path never reads it.
    fn checked_exit(&self, idx: usize) -> Option<LabelId> {
        if let Some(p) = &self.precise {
            let code = p.codes[idx];
            if code != 1 {
                return Some(p.code_blocks[&code]);
            }
        }
        self.deopt_label
    }

    /// Drop the scaled-index cache: the reserved register(s) no longer hold a
    /// usable `im1`/offset for any index. Called at a jump target (control could
    /// enter here with a stale register), across a SysV call (which clobbers the
    /// caller-saved reserved register), and whenever the cached index slot is
    /// reassigned (so the next access recomputes from the fresh index value).
    fn invalidate_off_cache(&mut self) {
        self.off_cache = None;
    }

    /// Drop the cache if `op` writes the cached index slot. A write to the slot
    /// the cache keys off makes the held `im1` stale; the next access must
    /// recompute from the new index value. (A write to ANY other slot leaves the
    /// cached `im1` valid, so cross-arithmetic reuse is preserved.)
    fn invalidate_off_cache_if_writes_idx(&mut self, op: &MicroOp) {
        if let Some(state) = self.off_cache {
            if dest_of(op) == Some(state.idx) {
                self.off_cache = None;
            }
        }
    }

    /// The hoisted register holding `frame[ptr_slot]` for an ACTIVE hoist of this
    /// ptr slot, or `None` (use the per-access frame reload). A ptr slot is the
    /// address base, so we key on it; the entry's `len_reg` is consulted via
    /// [`Gen::hoisted_len`]. Only entries whose span is active at the current op
    /// (recorded in `active_hoists`) are eligible — outside the span the register
    /// may hold a stale value (a different loop's array, or the slot was rewritten
    /// at the outer level), so it must not be read.
    fn hoisted_ptr(&self, ptr_slot: Slot) -> Option<Reg> {
        self.active_hoists
            .iter()
            .map(|&k| self.hoists[k])
            .find(|e| e.plan.ptr_slot == ptr_slot)
            .map(|e| e.ptr_reg)
    }

    /// The hoisted register holding `frame[len_slot]` for an ACTIVE hoist whose
    /// `ptr_slot` matches (the array is identified by its ptr base), or `None`.
    /// `None` when the access is unchecked-only-hoisted (no `len_reg` reserved) —
    /// the checked path then reloads `len` from the frame as before.
    fn hoisted_len(&self, ptr_slot: Slot) -> Option<Reg> {
        self.active_hoists
            .iter()
            .map(|&k| self.hoists[k])
            .find(|e| e.plan.ptr_slot == ptr_slot)
            .and_then(|e| e.len_reg)
    }
}

impl Gen<'_> {
    /// Spill the per-call-site live-after caller-saved residents (GP + XMM) for
    /// the call op at index `idx` to their frame slots before a SysV call: the
    /// callee clobbers all caller-saved registers. Reloaded by
    /// [`Gen::reload_volatiles_at`] after the call so loop-carried values survive.
    /// The `call_gp`/`call_xmm` lists are the LIVE-AFTER subset (a resident dead
    /// after the call is read by no later op and not observed by a deopt resume,
    /// so its spill is elided — bit-identical). The slices live on a separate
    /// borrow than `asm`, so this resolves the spill list before mutating the
    /// assembler.
    fn spill_volatiles_at(&mut self, idx: usize) {
        let gp: &[Slot] = self.call_gp.get(idx).map_or(&[], Vec::as_slice);
        for &s in gp {
            if let Loc::Reg(r) = self.loc[s as usize] {
                self.asm.mov_mr(BASE, (s as i32) * 8, r);
            }
        }
        let xmm: &[Slot] = self.call_xmm.get(idx).map_or(&[], Vec::as_slice);
        for &s in xmm {
            if let Loc::Xmm(x) = self.loc[s as usize] {
                self.asm.movsd_mr(BASE, (s as i32) * 8, x);
            }
        }
    }

    /// Reload the per-call-site live-after residents for the call op at `idx`.
    fn reload_volatiles_at(&mut self, idx: usize) {
        let gp: &[Slot] = self.call_gp.get(idx).map_or(&[], Vec::as_slice);
        for &s in gp {
            if let Loc::Reg(r) = self.loc[s as usize] {
                self.asm.mov_rm(r, BASE, (s as i32) * 8);
            }
        }
        let xmm: &[Slot] = self.call_xmm.get(idx).map_or(&[], Vec::as_slice);
        for &s in xmm {
            if let Loc::Xmm(x) = self.loc[s as usize] {
                self.asm.movsd_rm(x, BASE, (s as i32) * 8);
            }
        }
    }
}

impl<'a> Gen<'a> {
    /// Load slot `s` into GP register `r`. An XMM-resident slot is bit-copied
    /// via `movq` (defensive — INT-class slots never land in XMM).
    fn load(&mut self, r: Reg, s: Slot) {
        match self.loc[s as usize] {
            Loc::Reg(src) => self.asm.mov_rr(r, src),
            Loc::Frame => self.asm.mov_rm(r, BASE, (s as i32) * 8),
            Loc::Xmm(x) => self.asm.movq_rx(r, x),
        }
    }

    /// Store GP register `r` into slot `s`. An XMM-resident slot is bit-copied
    /// via `movq` (defensive — INT-class slots never land in XMM).
    fn store(&mut self, s: Slot, r: Reg) {
        match self.loc[s as usize] {
            Loc::Xmm(x) => self.asm.movq_xr(x, r),
            Loc::Reg(dst) => self.asm.mov_rr(dst, r),
            Loc::Frame => self.asm.mov_mr(BASE, (s as i32) * 8, r),
        }
    }

    /// Materialize slot `s` into a register usable as an operand, preferring its
    /// resident register (no move) and falling back to scratch `scratch`. Slot
    /// must be INT-class or frame-resident (an XMM-resident slot would be a
    /// classification bug — caught by `debug_assert`).
    fn operand(&mut self, s: Slot, scratch: Reg) -> Reg {
        match self.loc[s as usize] {
            Loc::Reg(r) => r,
            Loc::Frame => {
                self.asm.mov_rm(scratch, BASE, (s as i32) * 8);
                scratch
            }
            Loc::Xmm(_) => {
                debug_assert!(false, "int operand on an XMM-resident slot {s}");
                self.asm.mov_rm(scratch, BASE, (s as i32) * 8);
                scratch
            }
        }
    }

    /// Load f64 slot `s` into XMM register `x`.
    fn fload(&mut self, x: Xmm, s: Slot) {
        match self.loc[s as usize] {
            Loc::Xmm(src) => self.asm.movsd_rr(x, src),
            Loc::Frame => self.asm.movsd_rm(x, BASE, (s as i32) * 8),
            Loc::Reg(r) => self.asm.movq_xr(x, r), // a Mixed slot can never be here; defensive bit-copy.
        }
    }

    /// Store XMM register `x` into f64 slot `s`.
    fn fstore(&mut self, s: Slot, x: Xmm) {
        match self.loc[s as usize] {
            Loc::Xmm(dst) => self.asm.movsd_rr(dst, x),
            Loc::Frame => self.asm.movsd_mr(BASE, (s as i32) * 8, x),
            Loc::Reg(r) => self.asm.movq_rx(r, x), // defensive (Mixed never lands here).
        }
    }

    /// Materialize f64 slot `s` into an XMM operand, preferring its resident XMM
    /// register and falling back to scratch `scratch`.
    fn foperand(&mut self, s: Slot, scratch: Xmm) -> Xmm {
        match self.loc[s as usize] {
            Loc::Xmm(x) => x,
            Loc::Frame => {
                self.asm.movsd_rm(scratch, BASE, (s as i32) * 8);
                scratch
            }
            Loc::Reg(r) => {
                self.asm.movq_xr(scratch, r);
                scratch
            }
        }
    }
}

/// Compile a region of micro-ops into ONE contiguous register-allocated x86-64
/// function, callable through the [`crate::buffer::JitChain`] ABI. Returns
/// `None` (caller falls back to the stencil tier) when the region contains an
/// unsupported op or has no terminating `Return`/`Jump`.
///
/// A REGION never contains a call, so this gates on the call-free [`supported`]
/// subset and emits no self-call machinery.
pub fn compile_region_regalloc(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
) -> Option<CompiledChain> {
    compile_impl(ops, shared_status, false, 0, None)
}

/// Compile a PRECISE REGION — the in-place-array-mutation shape that ALSO does a
/// reallocating `ArrPush` (the fannkuch permutation rebuild, the graph_bfs BFS
/// frontier) — into ONE contiguous register-allocated x86-64 region with PRECISE
/// deopt. Returns `None` (caller falls back to the per-piece precise stencil
/// tier) on any unsupported op or a missing terminator.
///
/// A precise region is the keystone Wave 13 (`ArrPush` in NON-precise regions)
/// and Wave 15 (PRECISE deopt for in-place-mutating recursion) each handled HALF
/// of: a `ListPush` that REALLOCS the pinned buffer coexisting with an in-place
/// `SetIndex` whose replay-from-head would double-apply. Under the classic
/// discard-replay deopt the truncate rolls the pushes back but the in-place
/// write persists, so the region must instead resume AT the faulting op (no
/// replay). [`compile_region_regalloc`] handles the reallocating push (the
/// helper refreshes the pinned ptr/len in the frame after a possible realloc),
/// but only with the CLASSIC deopt; this entry adds the per-op PRECISE deopt
/// codes so the push+SetIndex region is sound.
///
/// SOUNDNESS of the deopt resume (no double-apply, grown-array materialization):
/// every checked op's side exit stores its encoded resume pc `(pc << 2) | 3`
/// (depth in the high bits) through the shared status cell and the precise
/// epilogue FLUSHES every resident-written scalar to its frame slot — so the
/// VM's region precise resume reads each scalar from the frame and re-boxes it
/// by kind. The grown array needs NO materialization step: the push helper grew
/// the `Vec` IN PLACE inside the same `Rc<RefCell<…>>` the VM register still
/// holds, so the VM keeps that register's live value (the precise re-box kind is
/// `None` for a pinned array) and it already reflects the post-push buffer +
/// length. The resume pc is AFTER the push, so the bytecode never re-runs it —
/// no double-apply, no lost appended element. (The VM's per-array entry snapshot
/// / truncate rollback is gated on a CLASSIC `Deopt`, never a precise `DeoptAt`,
/// so completed pushes stand.)
///
/// `deopt_codes` is the per-op resume table the region adapter built (parallel
/// to `ops`): a plain `1` keeps the op on the ordinary deopt terminal; any other
/// value is the precise tag emitted on that op's side exit. `depth_addr` is the
/// live-depth cell whose value rides the resume tag's high 32 bits.
pub fn compile_region_regalloc_precise(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
    depth_addr: i64,
    deopt_codes: &[i64],
) -> Option<CompiledChain> {
    if deopt_codes.len() != ops.len() {
        return None;
    }
    // A precise region always needs the status channel — every checked op's side
    // exit stores its tagged resume value through it.
    if shared_status.is_none() {
        return None;
    }
    compile_impl(ops, shared_status, false, depth_addr, Some(deopt_codes))
}

/// Compile a recursive FUNCTION's micro-op stream into ONE contiguous
/// register-allocated x86-64 function — including its DIRECT self-calls
/// (`CallSelf`/`CallSelfCopy`) — callable through the [`crate::buffer::JitChain`]
/// ABI. Returns `None` (caller falls back to the per-piece stencil tier) on any
/// unsupported op (a cross-function `Call`, list/map ops, byte arrays, …) or a
/// missing terminator.
///
/// The self-call is a REAL SysV `call` to this same chain's entry. The entry
/// address is unknown until the code is mapped, so it rides an `Arc<AtomicI64>`
/// ENTRY CELL whose address is baked into every call site (`mov rax,[cell];
/// call rax`) and which is written with `chain.base()` AFTER mapping — mirroring
/// `logos_stencil_call_self`'s patched-entry word (an unpatched `0` deopts). The
/// cell is kept alive by the returned [`CompiledChain`].
///
/// `depth_addr` is the live-depth cell's address (the caller's `ctx.depth`),
/// matching the function tier's wiring; the self-call increments/decrements it
/// and side-exits (status = 5) at [`SELF_CALL_DEPTH_LIMIT`].
pub fn compile_function_regalloc(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
    depth_addr: i64,
) -> Option<CompiledChain> {
    // A function with no self-call is just a region; route it through the
    // call-free path (no entry cell, no spill machinery) for uniformity.
    let has_self_call = ops
        .iter()
        .any(|op| matches!(op, MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. }));
    if !has_self_call {
        return compile_impl(ops, shared_status, true, depth_addr, None);
    }
    // A self-call always needs the status channel (its guards write it).
    if shared_status.is_none() {
        return None;
    }
    compile_impl(ops, shared_status, true, depth_addr, None)
}

/// Compile a LIST-PARAMETER (mode-B) recursive FUNCTION — one that mutates a
/// SHARED pinned array in place and may self-call — into ONE contiguous
/// register-allocated x86-64 function with PRECISE deopt. Returns `None`
/// (caller falls back to the per-piece stencil tier) on any unsupported op.
///
/// Where [`compile_function_regalloc`] uses CLASSIC deopt (a side exit replays
/// the whole function from its boundary args — sound only when every effect is
/// confined to the private frame, the scalar mode-A case), a list-param
/// function's in-place array writes land in SHARED state, so a replay would
/// DOUBLE-APPLY them. This entry instead emits PRECISE deopt: every checked op
/// and the self-call's guards side-exit through the shared status cell with the
/// op's ENCODED resume value (`(bytecode_pc << 2) | 3 | (depth << 32)`), exactly
/// as `logos_stencil_deopt_at` / `logos_stencil_call_precise` do, so the VM
/// materializes the native call chain and resumes interpreting AT the faulting
/// op — every prior in-place mutation intact, never re-applied.
///
/// `deopt_codes` is the per-op resume table the function adapter built (parallel
/// to `ops`): a plain `1` keeps the op on the ordinary deopt terminal; any other
/// value is the precise tag emitted on that op's side exit.
///
/// SOUNDNESS: the precise contract requires that on a side exit the native
/// frame mirror the tree-walker's full register state. The backend therefore
/// keeps every mode-B machinery slot (the plant window/resume/dst, the pin
/// triples, the disjoint callee window) FRAME-RESIDENT, every `ListTriple`
/// HANDLE slot frame-resident (the helper reads/writes the frame), and FLUSHES
/// every resident-written VM register to its frame slot at every precise side
/// exit (so `materialize` reads the correct value). All slots at or above the
/// function's register count `rc` are mode-B machinery and stay frame-resident.
pub fn compile_function_regalloc_precise(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
    depth_addr: i64,
    deopt_codes: &[i64],
) -> Option<CompiledChain> {
    if deopt_codes.len() != ops.len() {
        return None;
    }
    // A precise function always needs the status channel (its checked ops and
    // self-call guards write the tagged value through it).
    if shared_status.is_none() {
        return None;
    }
    compile_impl(ops, shared_status, true, depth_addr, Some(deopt_codes))
}

/// The VM register count `rc` of a mode-B precise function, INFERRED from the
/// op stream: the prologue's plant-window invalidation is `LoadConst { dst:
/// rc + 2, value: -1 }` (the adapter's first micro). Every slot `>= rc` is
/// mode-B machinery (plant slots, pin triples, the disjoint callee window) and
/// must stay frame-resident. Returns `None` if the stream does not open with
/// that recognizable prologue (then the precise path declines and the caller
/// falls back).
fn infer_mode_b_rc(ops: &[MicroOp]) -> Option<u16> {
    match ops.first() {
        Some(MicroOp::LoadConst { dst, value: -1 }) if *dst >= 2 => Some(dst - 2),
        _ => None,
    }
}

/// The shared codegen core for both the region and the function backends.
/// `is_function` selects the op-support gate ([`supported_function`] adds the
/// self-call ops); `depth_addr` is the live-depth cell (function backend only).
fn compile_impl(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
    is_function: bool,
    depth_addr: i64,
    precise: Option<&[i64]>,
) -> Option<CompiledChain> {
    if ops.is_empty() {
        return None;
    }
    // The PRECISE mode-B FUNCTION path needs the function's VM register count
    // `rc` to know which slots are machinery (the plant window/resume/dst, pin
    // triples, the disjoint callee window — every slot `>= rc`). Infer it from
    // the adapter's prologue micro; decline (fall back) if the stream does not
    // open with it. A precise REGION has NO mode-B machinery (no self-call plant
    // window — its in-place mutation + reallocating push flow through the
    // enclosing frame's pinned arrays, and the precise epilogue flushes every
    // resident-written scalar back to the frame for materialization), so it
    // leaves `mode_b_rc = None` and its scalars are register-allocated freely.
    // The classic paths also leave `mode_b_rc = None`.
    let mode_b_rc: Option<u16> = if precise.is_some() && is_function {
        Some(infer_mode_b_rc(ops)?)
    } else {
        None
    };
    let gate: fn(&MicroOp) -> bool = match (is_function, precise.is_some()) {
        (true, true) => supported_function_precise,
        (true, false) => supported_function,
        // A precise REGION uses the region op-support gate (`supported`, which
        // already admits the reallocating `ArrPush` and `ListClear` — W13); the
        // precise deopt codes ride the same per-op side-exit machinery as the
        // function path.
        _ => supported,
    };
    if !ops.iter().all(gate) {
        return None;
    }
    if !matches!(ops.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return None;
    }

    let has_self_call = ops
        .iter()
        .any(|op| matches!(op, MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. }));
    // The mode-B precise self-call is a REAL SysV call through the entry table.
    let has_precise_call = ops.iter().any(|op| matches!(op, MicroOp::Call { .. }));

    // A list-mutation / list-machinery op (`ArrPush`/`ListClear`/`NewList`/
    // `ListTriple`) is a HELPER CALL into the JIT runtime — a real SysV `call`,
    // so it clobbers the caller-saved registers exactly like a self-call and
    // needs the same spill/reload machinery (the `gp_volatile`/`xmm_volatile`
    // lists + the stack-alignment pad). Treat ANY such helper call, not just a
    // self-call, as requiring the call ABI.
    let has_list_call = ops.iter().any(|op| {
        matches!(
            op,
            MicroOp::NewList { .. }
                | MicroOp::ArrPush { .. }
                | MicroOp::ListClear { .. }
                | MicroOp::ListTriple { .. }
                | MicroOp::StrAppend { .. }
        )
    });
    let has_call = has_self_call || has_list_call || has_precise_call;

    // The vec/ptr/len handle TRIPLE of every list-mutation op MUST stay
    // FRAME-resident: the helper reads `frame[vec_slot]` and WRITES
    // `frame[ptr_slot]`/`frame[len_slot]` (refreshing them after a possible
    // realloc). A register-resident handle slot would be stale after the call
    // (the helper updates the frame, not the register) and would feed a wrong
    // pointer/length into the next access — the exact realloc-coherence trap.
    // Forcing the triple frame-resident makes the helper's frame writes the
    // single source of truth, so the next access reads the fresh pointer.
    let mut force_frame_set: std::collections::HashSet<Slot> = std::collections::HashSet::new();
    for op in ops {
        match *op {
            // `NewList` allocates a registry-owned fresh buffer and WRITES its
            // vec/ptr/len triple through the frame (the helper indexes the frame);
            // the same realloc-coherence rule as `ArrPush` requires the triple be
            // frame-resident so the helper's writes are the single source of
            // truth and every later push/load reads the fresh pointer/length.
            MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. }
            | MicroOp::ArrPush { vec_slot, ptr_slot, len_slot, .. }
            | MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
                force_frame_set.insert(vec_slot);
                force_frame_set.insert(ptr_slot);
                force_frame_set.insert(len_slot);
            }
            // A triple-plant's HANDLE slot is read by the helper FROM THE FRAME,
            // and its vec/ptr/len triple is WRITTEN to the frame — all four must
            // be frame-resident so the helper's view and the next access agree.
            MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
                force_frame_set.insert(handle_slot);
                force_frame_set.insert(vec_slot);
                force_frame_set.insert(ptr_slot);
                force_frame_set.insert(len_slot);
            }
            // The str-append's handle slot (a `*mut Value`) is read by the helper
            // FROM THE FRAME, so it must stay frame-resident (a register-resident
            // handle would not be seen by the SysV call). The accumulator itself
            // lives in the VM register file, off-frame — nothing else here needs
            // forcing.
            MicroOp::StrAppend { text_handle_slot, .. } => {
                force_frame_set.insert(text_handle_slot);
            }
            _ => {}
        }
    }

    // The callee-window staging slots (`args_start..`) of a self-call MUST stay
    // FRAME-resident: the callee reads its parameters from its own frame window,
    // and the codegen stages them by writing those frame cells. A `CallSelfCopy`
    // also carries `frame_size` (the callee's full frame, = limit_slot + 1); the
    // window therefore spans `args_start .. args_start + frame_size`. Force every
    // slot at or above the lowest such window base to the frame.
    let mut force_frame_above: Option<u16> = None;
    for op in ops {
        let args_start = match *op {
            MicroOp::CallSelf { args_start, .. }
            | MicroOp::CallSelfCopy { args_start, .. }
            | MicroOp::Call { args_start, .. } => args_start,
            _ => continue,
        };
        // The callee window starts at args_start; everything at/above it stages
        // a callee frame slot and must stay frame-resident.
        force_frame_above = Some(force_frame_above.map_or(args_start, |a| a.min(args_start)));
    }
    // PRECISE (mode-B): EVERY slot at or above the VM register count `rc` is
    // machinery — the plant window/resume/dst, the pin triples and the disjoint
    // callee window. All must stay FRAME-resident: the precise-deopt walk
    // (`ChainFn::materialize`) reads each ancestor frame's plant slots and pin
    // handles straight from the frame, so a register copy would be stale. This
    // subsumes (and is stricter than) the per-call window forcing above.
    if let Some(rc) = mode_b_rc {
        force_frame_above = Some(force_frame_above.map_or(rc, |a| a.min(rc)));
    }

    // Whether the program has any checked op needing a deopt channel (divisor
    // check, a checked array bounds check, or a self-call guard). When one is
    // present but the caller passed no shared cell, allocate a private one —
    // exactly what the stencil compilers do (`has_checked` ⇒ `unwrap_or_else`).
    let needs_status = ops.iter().any(needs_deopt);
    let status: Option<std::sync::Arc<AtomicI64>> = if needs_status {
        Some(shared_status.unwrap_or_else(|| std::sync::Arc::new(AtomicI64::new(0))))
    } else {
        shared_status
    };

    // The largest slot index — sizes every per-slot vector below (loc / written
    // / liveness). It covers each self-call window's full extent because the
    // frame indexing allocates against it.
    let max_slot = max_slot_of(ops);

    // Backward liveness, computed ONCE when the region has a SysV call and reused
    // by both the call-weight ranking (below) and the per-call-site spill-elision
    // pass (further down). `None` for a call-free region: there are no calls to
    // survive, so no bonus and no spill machinery. Computing it before the
    // ranking is what lets the ranking SEE which slots are live across each call.
    let live_after: Option<Vec<Vec<bool>>> = has_call.then(|| liveness_after(ops, max_slot));

    // Rank slots by LOOP-WEIGHTED reference count, with CALL-LOCALITY as a
    // SECONDARY tie-break; assign the hottest to physical registers. A raw count
    // under-prices loop-carried values: a slot touched once per iteration of a
    // hot loop costs a memory round-trip on EVERY iteration, whereas a slot
    // touched many times in straight-line setup code costs a round-trip only that
    // once. The linear-scan loop weight (each reference at loop depth `d` worth
    // `LOOP_WEIGHT_BASE^d`) closes this so a back-edge value (nbody's dx/dy/dist,
    // mandelbrot's zr/zi) outranks colder slots and STAYS RESIDENT across
    // iterations. This PRIMARY key — and the resident-vs-spilled boundary it
    // induces — is identical to the pre-Fix-1 ranking.
    //
    // The call-weight breaks TIES among equal-primary-weight slots toward those
    // LIVE ACROSS a SysV call. Combined with the `has_call` pool order
    // (callee-saved first), a call-survivor wins a callee-saved register over a
    // call-dead slot of equal weight and pays NO per-call spill/reload — only a
    // one-time prologue push/pop, the dominant cost in a recursion-heavy loop
    // (fib / binary_trees / coins). Because it is only a tie-break it can never
    // evict a hotter loop slot, so it adds the steering without the spill-boundary
    // perturbation an additive bonus would cause.
    //
    // Ranking is semantically inert: it changes only WHICH register a resident
    // slot occupies (and, on a primary-key tie, which equally-cold slot spills). A
    // slot still lives in exactly one place for the whole function and its value
    // never changes, so the result is bit-identical to any other ranking.
    //
    // The base/cap mirror the per-piece tier's `select_pins` loop weight
    // (`BASE=16`, depth capped at 4) so both backends rank loop-carried slots on
    // the SAME frequency model. Every weight is `u64` and saturating, so even a
    // pathologically deep/large region cannot overflow the ranking key (a
    // saturated tie falls back to the call-survival then deterministic slot-index
    // order).
    const LOOP_WEIGHT_BASE: u64 = 16;
    const LOOP_DEPTH_CAP: u32 = 4;
    const CALL_WEIGHT: u64 = 4;
    // A zero call-weight zeroes the secondary key, reducing `rank_slots` to the
    // pure loop-weighted ranking — the kill-switch is exactly pre-Fix-1 behavior.
    let call_weight = if call_weight_enabled() { CALL_WEIGHT } else { 0 };
    let (order, _) = rank_slots(
        ops,
        LOOP_WEIGHT_BASE,
        LOOP_DEPTH_CAP,
        call_weight,
        live_after.as_deref(),
    );

    // Classify each slot. INT-class slots compete for GP registers, FLOAT-class
    // slots for XMM registers; MIXED slots (used by both classes) stay frame-
    // resident. The two pools are disjoint, so the assignment is sound: a slot
    // is always in exactly one place, of the right class, for the whole function.
    let class = classify_slots(ops, max_slot);

    // Register-pool ORDER. Pool order is semantically inert — it only decides
    // WHICH physical register a slot occupies, never where (a slot is still in
    // exactly one place for the whole function), so every ordering is
    // bit-identical. But it changes the prologue/epilogue and per-call spill
    // bill, so we order by what is cheapest for THIS region's call shape:
    //
    //  - CALL-FREE region: a caller-saved register costs NOTHING (no prologue
    //    push/pop, no spill — nothing clobbers it). A callee-saved register
    //    costs a prologue `push` + epilogue `pop` (and a stack-alignment pad
    //    risk). So prefer CALLER-SAVED first: a hot scalar loop (primes,
    //    pi_leibniz, fib_iterative, the matrix/histogram inner loops) pays no
    //    callee-save prologue/epilogue at all when it fits the 6 caller-saved
    //    GP regs.
    //  - region/function WITH a SysV call (self-call or list-mutation helper):
    //    a caller-saved resident is clobbered by every call, so it is
    //    spilled+reloaded around each one (gp_volatile below) — a pair of
    //    memory ops per call. A callee-saved resident survives the call for the
    //    one-time prologue/epilogue push/pop. In a call-heavy loop (fib /
    //    binary_trees / coins recursion) the per-call spill pair dominates the
    //    one-time save, so prefer CALLEE-SAVED first.
    let mut pool: Vec<Reg> = Vec::new();
    if has_call {
        pool.extend_from_slice(&CALLEE_SAVED);
        pool.extend_from_slice(&CALLER_SAVED);
    } else {
        pool.extend_from_slice(&CALLER_SAVED);
        pool.extend_from_slice(&CALLEE_SAVED);
    }

    // SCALED-INDEX CSE register reservation (Wave 26). A region with array
    // accesses RESERVES up to two CALLER-SAVED registers OUT of the resident
    // pool — held for the run-shared `im1 = idx - 1` (and, with the second, its
    // scaled byte offset `im1 * 8`). Reserving up front (rather than scavenging a
    // register the scan happened to leave free) is what makes the CSE fire in the
    // HIGH-PRESSURE loops it targets — nbody/matrix touch many co-indexed
    // arrays, whose ptr/len handle slots would otherwise consume every GP
    // register and leave none free. The cost is that the one or two
    // LOWEST-ranked int slots spill to the frame instead of a register — a good
    // trade in an array-heavy loop, where eliminating the per-access index
    // recompute across N arrays dominates one extra frame round-trip on a cold
    // scalar. CALLER-SAVED registers need no prologue save, and the cache is
    // dropped across every SysV call, so a call's clobber of them is harmless.
    // The reserved registers are removed from `pool`, so the linear scan never
    // assigns them to a resident — they are exclusively the CSE cache's.
    // Reserve ONLY when the region actually has a reusable run — two or more
    // array accesses sharing one index slot with no intervening invalidation.
    // Without a shared-index run the CSE never fires, so reserving a register
    // (spilling a hot scalar) would be a pure cost — exactly the matrix_mult
    // inner loop, whose three accesses use THREE different indices (`i*n+k`,
    // `k*n+j`, `i*n+j`) and so must NOT pay the reservation. The static check
    // mirrors the runtime cache invalidation precisely.
    let (off_im1_reg, off_scaled_reg) = if off_cse_enabled() && has_shared_index_run(ops) {
        // The reservable caller-saved registers still in the pool, in pool order;
        // take from the END so residents keep the higher-priority (earlier)
        // registers and only the coldest int slots lose a register.
        let mut reservable: Vec<Reg> =
            pool.iter().copied().filter(|r| CALLER_SAVED.contains(r)).collect();
        // Leave at least one register for residents when the pool can spare it;
        // reserve TWO only when ≥ 2 caller-saved remain beyond that margin (so
        // the loop's index/counter still gets a register). With a tiny pool a
        // single reservation (a pure array-copy loop with no other live int) is
        // still sound — the reserved register is the cache's alone.
        let want = if reservable.len() >= 3 { 2 } else { usize::from(!reservable.is_empty()) };
        let mut reserved = Vec::new();
        for _ in 0..want {
            if let Some(r) = reservable.pop() {
                reserved.push(r);
            }
        }
        // Remove the reserved registers from the resident pool.
        pool.retain(|r| !reserved.contains(r));
        (reserved.first().copied(), reserved.get(1).copied())
    } else {
        (None, None)
    };

    // LOOP-INVARIANT PTR/LEN HOIST register reservation (Wave 27a). Reserve
    // CALLEE-SAVED registers for each invariant-array plan and load the handle(s)
    // ONCE at the loop pre-header (`g.load(reg, slot)` copies whatever the slot's
    // current home holds — a register or a frame cell — so the pre-header captures
    // the value the OUTER context just set). Callee-saved registers SURVIVE the
    // SysV calls (the `ArrPush`/`ListClear` helpers, a self-call) inside the
    // loop, so a handle that the linear scan would otherwise put in a CALLER-saved
    // register — spilled + reloaded around EVERY in-loop call — instead stays live
    // for a one-time prologue push/pop; and a handle that would land in the FRAME
    // (forced by `force_frame_set`, or out of registers under pressure) avoids its
    // per-access reload. The reserved registers are removed from the resident pool
    // (so the scan never assigns them) and added to `used_callee` (prologue
    // save/restore), always leaving at least one callee-saved register for the
    // scan's own call-surviving residents.
    let mut used_callee: Vec<Reg> = Vec::new();
    let mut hoists: Vec<HoistEntry> = Vec::new();
    if ptr_hoist_enabled() {
        // The callee-saved registers still in the pool, reservable for hoisting.
        // We take from the FRONT (callee-saved lead the pool in a call-heavy
        // region) but always leave AT LEAST ONE callee-saved register for the
        // scan's call-surviving residents — a loop-carried scalar live across the
        // push wants a callee-saved home too. With fewer than two callee-saved
        // available, no hoist is reserved (the trade would starve the loop).
        let mut reservable_callee: Vec<Reg> =
            pool.iter().copied().filter(|r| CALLEE_SAVED.contains(r)).collect();
        let mut reserved_for_hoist: Vec<Reg> = Vec::new();
        let plans = loop_invariant_array_hoists(ops);
        for plan in plans {
            // Hoist only when it amortizes: either the array is accessed at least
            // twice in the span (each access otherwise reloads/uses the handle), or
            // the span contains a SysV call (a register-resident handle is then
            // spilled+reloaded around every call — a callee-saved hoist survives
            // it). A single un-called access has nothing to amortize → skip.
            let span_calls = ops[plan.head..=plan.back].iter().any(is_sysv_call);
            if plan.accesses < 2 && !span_calls {
                continue;
            }
            let need = 1 + usize::from(plan.needs_len);
            // Reserve from the available callee-saved, leaving one for the scan.
            if reservable_callee.len().saturating_sub(need) < 1 {
                continue;
            }
            let ptr_reg = reservable_callee.remove(0);
            reserved_for_hoist.push(ptr_reg);
            let len_reg = if plan.needs_len {
                let r = reservable_callee.remove(0);
                reserved_for_hoist.push(r);
                Some(r)
            } else {
                None
            };
            hoists.push(HoistEntry { plan, ptr_reg, len_reg });
        }
        // Remove the reserved registers from the resident pool and record them for
        // the prologue/epilogue callee-saved save (deterministic order).
        pool.retain(|r| !reserved_for_hoist.contains(r));
        for r in reserved_for_hoist {
            used_callee.push(r);
        }
    }

    let mut loc = vec![Loc::Frame; max_slot + 1];
    let mut pi = 0usize; // GP pool cursor
    let mut fi = 0usize; // XMM pool cursor
    for (s, _) in &order {
        // Force callee-window staging slots to the frame (the callee reads them
        // from its frame; a register-resident window slot would never reach it).
        if let Some(thresh) = force_frame_above {
            if *s >= thresh {
                continue;
            }
        }
        // Force list-mutation handle slots (vec/ptr/len) to the frame — the
        // helper reads/writes them there, so a register copy would go stale.
        if force_frame_set.contains(s) {
            continue;
        }
        match class[*s as usize] {
            Class::Int => {
                if pi >= pool.len() {
                    continue;
                }
                let r = pool[pi];
                pi += 1;
                loc[*s as usize] = Loc::Reg(r);
                if CALLEE_SAVED.contains(&r) {
                    used_callee.push(r);
                }
            }
            Class::Float => {
                if fi >= FLOAT_REGS.len() {
                    continue;
                }
                loc[*s as usize] = Loc::Xmm(FLOAT_REGS[fi]);
                fi += 1;
            }
            Class::Mixed => {} // frame-resident only.
        }
    }

    // The caller-saved residents (GP + XMM) that a SysV call CLOBBERS — a
    // self-call OR a list-mutation helper call both trash the caller-saved
    // registers per the ABI. This is the UNIVERSE; the per-call-site lists below
    // intersect it with each call's live-after set. Callee-saved residents
    // (rbx/r12/r13/r14, BASE=r15) survive a call automatically and need no spill.
    let (gp_volatile, xmm_volatile): (Vec<Slot>, Vec<Slot>) = if has_call {
        let mut gp = Vec::new();
        let mut xmm = Vec::new();
        for s in 0..=max_slot as u16 {
            match loc[s as usize] {
                Loc::Reg(r) if CALLER_SAVED.contains(&r) => gp.push(s),
                Loc::Xmm(_) => xmm.push(s),
                _ => {}
            }
        }
        (gp, xmm)
    } else {
        (Vec::new(), Vec::new())
    };

    // Slots that are resident AND written somewhere must be flushed to the
    // frame on exit. (A resident read-only slot was loaded from the frame at
    // entry, never changed, so the frame is already correct.)
    let mut written: Vec<bool> = vec![false; max_slot + 1];
    for op in ops {
        if let Some(d) = dest_of(op) {
            written[d as usize] = true;
        }
    }

    // PER-CALL-SITE spill liveness. A caller-saved resident is spilled+reloaded
    // around a call iff its value must survive the call's register clobber. That
    // is true exactly when the slot is either:
    //   - WRITTEN somewhere (so it is flushed to the frame at EVERY region exit —
    //     `Return` and every deopt side-exit — which the broader VM may read; its
    //     last-written value MUST be preserved through the clobbering call), OR
    //   - LIVE AFTER this call (read by some later op in the region).
    // A caller-saved resident that is READ-ONLY (never written) AND dead after
    // the call is the ONLY case we elide: its frame cell already holds the value
    // loaded at entry (it never changed), it is read by no later op, and being
    // unwritten it is never flushed at exit — so eliding its spill/reload is
    // bit-identical to the full-frame stencil tier (verified by the
    // `*_matches_stencil` post-frame differentials). This removes the redundant
    // spill/reload of cold read-only operands (e.g. a constant or bound consulted
    // only before the call) from call-heavy loops. For a non-call op the lists
    // are empty. When `has_call` is false the liveness pass is skipped entirely.
    let (call_gp, call_xmm): (Vec<Vec<Slot>>, Vec<Vec<Slot>>) = if let Some(live_after) =
        live_after.as_deref()
    {
        let keep = |s: Slot, la: &[bool]| -> bool {
            written.get(s as usize).copied().unwrap_or(true)
                || la.get(s as usize).copied().unwrap_or(true)
        };
        let mut cgp: Vec<Vec<Slot>> = vec![Vec::new(); ops.len()];
        let mut cxmm: Vec<Vec<Slot>> = vec![Vec::new(); ops.len()];
        for idx in 0..ops.len() {
            let la = &live_after[idx];
            cgp[idx] = gp_volatile.iter().copied().filter(|&s| keep(s, la)).collect();
            cxmm[idx] = xmm_volatile.iter().copied().filter(|&s| keep(s, la)).collect();
        }
        (cgp, cxmm)
    } else {
        (Vec::new(), Vec::new())
    };

    // One label per op (jump targets) plus the deopt epilogue label and the
    // self-call deopt-PROPAGATION (bare flush+return, no status write) label.
    let mut asm = Asm::new();
    let op_labels: Vec<LabelId> = (0..ops.len()).map(|_| asm.new_label()).collect();
    // The plain (classic) deopt epilogue is still needed for any precise op whose
    // code is the plain marker `1` (a deoptable op the adapter chose not to make
    // precise) and for every classic checked op.
    let deopt_label = if needs_status { Some(asm.new_label()) } else { None };
    // A PROPAGATION epilogue (bare flush+return) is needed when ANY real SysV
    // self-call is present — the classic self-call OR the mode-B precise call —
    // because the inner exit code (already in the status cell) must not be
    // overwritten on the way out.
    let propagate_label = (has_self_call || has_precise_call).then(|| asm.new_label());

    // PRECISE (mode-B) deopt machinery: a shared precise epilogue plus one
    // out-of-line tag-staging block per DISTINCT non-plain code (deterministic,
    // first-use order). Each block stages its tag in `S1` and jumps to the
    // epilogue, which reads the live depth, ORs it into the high 32 bits, writes
    // the status cell, flushes every resident-written slot, and returns. This is
    // the contiguous-codegen analog of the stencil tier's `ST_DEOPT_AT` family.
    let mut code_blocks: std::collections::HashMap<i64, LabelId> = std::collections::HashMap::new();
    let precise_epilogue = if precise.is_some() { Some(asm.new_label()) } else { None };
    if let Some(codes) = precise {
        for &c in codes {
            if c != 1 {
                code_blocks.entry(c).or_insert_with(|| asm.new_label());
            }
        }
    }

    // Stack-pointer alignment: at function entry RSP ≡ 8 (mod 16) (the caller's
    // `call` pushed the return address). After `push BASE` + K callee-saved
    // pushes, RSP ≡ 8 + 8*(1+K) (mod 16). A SysV `call` requires RSP ≡ 0 (mod
    // 16) at the call instruction. Pad with one 8-byte slot when (1+K) is even
    // so RSP is 16-aligned at every call (self-call OR list-mutation helper);
    // undo it in every epilogue. (No calls ⇒ no padding needed, none emitted.)
    let pushes = 1 + used_callee.len();
    let stack_pad: i32 = if has_call && pushes % 2 == 0 { 8 } else { 0 };

    // Prologue: save callee-saved regs, move base (rdi) into r15, apply the
    // alignment pad, then load every resident slot from the frame.
    asm.push(BASE);
    for &r in &used_callee {
        asm.push(r);
    }
    asm.mov_rr(BASE, Reg::Rdi);
    if stack_pad != 0 {
        asm.sub_ri(Reg::Rsp, stack_pad);
    }
    {
        // Load resident slots in slot order for determinism. GP-resident slots
        // load via `mov`; XMM-resident (float) slots via `movsd`.
        for s in 0..=max_slot as u16 {
            match loc.get(s as usize) {
                Some(Loc::Reg(r)) => asm.mov_rm(*r, BASE, (s as i32) * 8),
                Some(Loc::Xmm(x)) => asm.movsd_rm(*x, BASE, (s as i32) * 8),
                _ => {}
            }
        }
    }

    let status_addr = status
        .as_ref()
        .map(|c| c.as_ref() as *const AtomicI64 as i64)
        .unwrap_or(0);

    // The self-call ENTRY CELL: an `Arc<AtomicI64>` whose address is baked into
    // every call site and which is written with `chain.base()` after mapping.
    // Needed for BOTH the classic direct self-call and the mode-B precise call
    // (both are REAL SysV calls through this very chain's entry).
    let entry_cell: Option<std::sync::Arc<AtomicI64>> =
        (has_self_call || has_precise_call).then(|| std::sync::Arc::new(AtomicI64::new(0)));
    let entry_addr = entry_cell
        .as_ref()
        .map(|c| c.as_ref() as *const AtomicI64 as i64)
        .unwrap_or(0);

    let self_call = propagate_label.map(|pl| SelfCall { propagate_label: pl });
    let precise_ctx = precise.map(|codes| Precise { codes, code_blocks: &code_blocks });

    let mut g = Gen {
        asm,
        loc: &loc,
        op_labels: &op_labels,
        deopt_label,
        stack_pad,
        self_call,
        call_gp: &call_gp,
        call_xmm: &call_xmm,
        precise: precise_ctx,
        off_im1_reg,
        off_scaled_reg,
        off_cache: None,
        hoists: &hoists,
        active_hoists: Vec::new(),
    };

    // Jump-target indices: an op reached by some transfer's `target` is a control
    // join, so the scaled-index cache must be DROPPED on entry to it (a stale
    // reserved register from the fall-through predecessor must not be reused when
    // control arrives from the branch). Computed once; consulted in the loop.
    let mut is_jump_target = vec![false; ops.len()];
    for op in ops {
        if let Some(t) = op_target(op) {
            if let Some(slot) = is_jump_target.get_mut(t) {
                *slot = true;
            }
        }
    }

    // LOOP-INVARIANT CONSTANT HOIST (Wave 28). A `LoadConst` whose `dst` is the
    // span's sole writer and lands in a GP REGISTER is materialized ONCE in the
    // loop pre-header instead of every iteration. `const_hoist_by_op[i] =
    // Some(head)` marks the op `i` to (a) SKIP in the body and (b) materialize at
    // `head`'s pre-header.
    //
    // FLOAT (XMM-resident) consts are EXCLUDED, and the exclusion is empirical,
    // not incidental. mandelbrot's hot inner loop re-materializes `2.0`/`4.0`
    // every iteration through a GP→XMM bridge; hoisting them into resident XMM
    // registers held across the loop is bit-identical but measured ~8% SLOWER (n
    // =2000: 480ms vs 443ms, reproducible across independent processes; GP-only
    // hoisting is at parity). The XMM register file is SATURATED (14/14 resident
    // in mandelbrot), and holding an invariant float in an XMM register that the
    // float dependency chain repeatedly reads serializes worse than the
    // per-iteration re-materialization, whose independent `movabs;movq` the
    // out-of-order engine overlaps with the float chain — the copy-and-patch /
    // contiguous backend is throughput-bound here, so removing the "redundant"
    // float loads loses free ILP. Frame-resident dsts are also filtered (no
    // register to keep resident — the body would still touch the frame). The
    // structural detection is purely over the op stream; residency is decided
    // here against `loc`.
    let mut const_hoist_by_op: Vec<Option<usize>> = vec![None; ops.len()];
    let mut const_hoist_at_head: Vec<Vec<usize>> = vec![Vec::new(); ops.len()];
    if const_hoist_enabled() {
        for h in loop_invariant_const_hoists(ops) {
            if matches!(loc[h.dst as usize], Loc::Reg(_)) {
                const_hoist_by_op[h.op] = Some(h.head);
                const_hoist_at_head[h.head].push(h.op);
            }
        }
    }

    for (idx, op) in ops.iter().enumerate() {
        // LOOP PRE-HEADER for any hoist whose loop head is THIS op: load the
        // invariant ptr (and len) into the reserved callee-saved register(s) ONCE,
        // BEFORE binding the head's label. The loop's back-edge targets the bound
        // label, so it SKIPS this load (the register stays live across iterations);
        // only the entry fall-through executes it. The entry-discipline guard in
        // `loop_invariant_array_hoists` ensures the head is entered ONLY via this
        // fall-through, so the register is always loaded before the first access.
        for k in 0..g.hoists.len() {
            let e = g.hoists[k];
            if e.plan.head == idx {
                g.load(e.ptr_reg, e.plan.ptr_slot);
                if let Some(lr) = e.len_reg {
                    g.load(lr, e.plan.len_slot);
                }
            }
        }
        // LOOP PRE-HEADER for any loop-invariant CONSTANT whose head is THIS op:
        // materialize the raw bits ONCE into the dst register, BEFORE binding the
        // head's label, so the back-edge (which jumps to the bound label) skips it
        // and the register stays resident across iterations. Mirrors the body
        // `LoadConst` emit exactly (a float dst bridges GP→XMM via `movq`), so the
        // register holds the same bits the per-iteration load would have.
        for ci in 0..const_hoist_at_head[idx].len() {
            let op_i = const_hoist_at_head[idx][ci];
            if let MicroOp::LoadConst { dst, value } = ops[op_i] {
                g.asm.mov_ri(S0, value);
                if let Loc::Xmm(_) = g.loc[dst as usize] {
                    g.asm.movq_xr(FS0, S0);
                    g.fstore(dst, FS0);
                } else {
                    g.store(dst, S0);
                }
            }
        }
        g.asm.bind(op_labels[idx]);
        // The hoists ACTIVE at this op: those whose span `[head, back]` contains
        // `idx`. Inside the span `emit_arr_addr` reads the hoisted register;
        // outside it falls back to the per-access frame reload (the register may
        // hold a stale value from a different loop or a since-rewritten slot).
        g.active_hoists.clear();
        for k in 0..g.hoists.len() {
            let p = g.hoists[k].plan;
            if (p.head..=p.back).contains(&idx) {
                g.active_hoists.push(k);
            }
        }
        // Drop the scaled-index cache at a control join: control could reach this
        // op from a branch where the reserved register holds a different (or no)
        // index, so the next array access here must recompute.
        if is_jump_target[idx] {
            g.invalidate_off_cache();
        }
        // A hoisted loop-invariant `LoadConst`: the pre-header already
        // materialized its register, which stays resident across the back-edge —
        // SKIP the per-iteration re-materialization (the body label is still bound
        // above so any transfer to it lands here correctly). The post-match
        // off-cache invalidation below is still honored: a `LoadConst` is never a
        // SysV call, and `invalidate_off_cache_if_writes_idx` recomputes a cache
        // keyed off this slot — harmless here (the invariant value is unchanged).
        let hoisted_const = const_hoist_by_op[idx].is_some();
        if !hoisted_const {
            match op {
            MicroOp::CallSelf { dst, args_start, limit_slot, frame_size, .. } => {
                emit_self_call(
                    &mut g, idx, *dst, *args_start, None, *limit_slot, *frame_size, status_addr,
                    depth_addr, entry_addr,
                );
            }
            MicroOp::CallSelfCopy {
                dst, args_start, src_start, arg_count, limit_slot, frame_size, ..
            } => {
                emit_self_call(
                    &mut g, idx, *dst, *args_start, Some((*src_start, *arg_count)), *limit_slot,
                    *frame_size, status_addr, depth_addr, entry_addr,
                );
            }
            // The mode-B precise self-`Call`: the disjoint callee window is
            // placed at `args_start == frame_size` (the adapter's invariant), so
            // the callee's frame size — for the arena bound and the limit plant —
            // is exactly `args_start`. The per-op code (precise) drives the
            // guard exits.
            MicroOp::Call { dst, args_start, limit_slot, .. } => {
                let code = g
                    .precise
                    .as_ref()
                    .map(|p| p.codes[idx])
                    .expect("a precise Call requires precise codes");
                emit_precise_call(
                    &mut g, idx, *dst, *args_start, *limit_slot, *args_start as i64, status_addr,
                    depth_addr, entry_addr, code,
                );
            }
            _ => emit_op(&mut g, op, idx, &written, &used_callee)?,
            }
        }
        // A SysV call (self-call, precise call, or a list/string-mutation helper)
        // clobbers the caller-saved reserved register(s) during the call, so the
        // cache cannot survive it. A write to the cached index slot makes the held
        // `im1` stale. Either drops the cache so a later access recomputes.
        if is_sysv_call(op) {
            g.invalidate_off_cache();
        } else {
            g.invalidate_off_cache_if_writes_idx(op);
        }
    }

    // Deopt epilogue (only if some op can side-exit): store 1 to the status
    // cell, flush resident-written slots, restore callee-saved regs, return 0.
    if let Some(dl) = deopt_label {
        g.asm.bind(dl);
        // mov rax, status_addr; mov qword [rax], 1
        g.asm.mov_ri(S0, status_addr);
        g.asm.mov_ri(S1, 1);
        g.asm.mov_mr(S0, 0, S1);
        emit_flush_and_return(&mut g, &written, &used_callee, None);
    }

    // Self-call deopt-PROPAGATION epilogue: the status cell ALREADY holds the
    // inner exit code (5/9/precise/1), so flush and return WITHOUT touching it.
    if let Some(pl) = propagate_label {
        g.asm.bind(pl);
        emit_flush_and_return(&mut g, &written, &used_callee, None);
    }

    // PRECISE (mode-B) deopt epilogue + per-code tag-staging blocks. Each block
    // stages its tag in S1, then falls through (or jumps) to the shared epilogue,
    // which combines the live depth into the high 32 bits, writes the status
    // cell, flushes every resident-written slot, and returns — bit-identical to
    // `logos_stencil_deopt_at` (`status = (pc<<2 | 3) | (depth << 32)`).
    if let Some(ep) = precise_epilogue {
        // The blocks (deterministic order by code value for a stable layout),
        // each `mov S1, code; jmp epilogue`.
        let mut codes_sorted: Vec<(i64, LabelId)> =
            code_blocks.iter().map(|(&c, &l)| (c, l)).collect();
        codes_sorted.sort_by_key(|(c, _)| *c);
        for (c, lbl) in codes_sorted {
            g.asm.bind(lbl);
            g.asm.mov_ri(S1, c);
            g.asm.jmp(ep);
        }
        // The shared epilogue: S1 = tag; rax = *depth; rax <<= 32; rax |= S1;
        // *status = rax; flush; return.
        g.asm.bind(ep);
        g.asm.mov_ri(S0, depth_addr);
        g.asm.mov_rm(S0, S0, 0); // S0 = *depth
        g.asm.shl_ri(S0, 32);
        g.asm.or_rr(S0, S1); // S0 = (depth << 32) | tag
        g.asm.mov_ri(SC, status_addr);
        g.asm.mov_mr(SC, 0, S0); // *status = S0
        emit_flush_and_return(&mut g, &written, &used_callee, None);
    }

    let code = g.asm.resolve();
    let chain = JitChain::from_code(&code, ops.len()).ok()?;

    // Patch the entry cell with the mapped base, so every self-call reaches this
    // very function. An unpatched (0) cell would deopt (matching the stencil's
    // unpatched-entry guard), but we patch it here, before the chain ever runs.
    if let Some(cell) = entry_cell {
        cell.store(chain.base() as i64, std::sync::atomic::Ordering::SeqCst);
        return Some(CompiledChain::from_chain_keepalive(chain, status, vec![cell]));
    }
    Some(CompiledChain::from_chain(chain, status))
}

/// The destination slot an op writes, if any.
fn dest_of(op: &MicroOp) -> Option<Slot> {
    match *op {
        MicroOp::Move { dst, .. }
        | MicroOp::LoadConst { dst, .. }
        | MicroOp::Add { dst, .. }
        | MicroOp::Sub { dst, .. }
        | MicroOp::Mul { dst, .. }
        | MicroOp::Lt { dst, .. }
        | MicroOp::Gt { dst, .. }
        | MicroOp::LtEq { dst, .. }
        | MicroOp::GtEq { dst, .. }
        | MicroOp::Eq { dst, .. }
        | MicroOp::Neq { dst, .. }
        | MicroOp::BitAnd { dst, .. }
        | MicroOp::BitOr { dst, .. }
        | MicroOp::BitXor { dst, .. }
        | MicroOp::Shl { dst, .. }
        | MicroOp::Shr { dst, .. }
        | MicroOp::NotInt { dst, .. }
        | MicroOp::NotBool { dst, .. }
        | MicroOp::Div { dst, .. }
        | MicroOp::Mod { dst, .. }
        | MicroOp::ArrLoad { dst, .. }
        | MicroOp::DivPow2 { dst, .. }
        | MicroOp::MagicDivU { dst, .. }
        | MicroOp::AddF { dst, .. }
        | MicroOp::SubF { dst, .. }
        | MicroOp::MulF { dst, .. }
        | MicroOp::DivF { dst, .. }
        | MicroOp::SqrtF { dst, .. }
        | MicroOp::IntToFloat { dst, .. }
        | MicroOp::FmaF { dst, .. }
        | MicroOp::LtF { dst, .. }
        | MicroOp::GtF { dst, .. }
        | MicroOp::LtEqF { dst, .. }
        | MicroOp::GtEqF { dst, .. }
        | MicroOp::EqF { dst, .. }
        | MicroOp::NeqF { dst, .. }
        // A self-call writes its result into `dst` (after a successful return).
        | MicroOp::CallSelf { dst, .. }
        | MicroOp::CallSelfCopy { dst, .. }
        // The mode-B precise self-call also stores its result handle into `dst`.
        | MicroOp::Call { dst, .. } => Some(dst),
        // `ArrStore` writes the BUFFER (through the pinned pointer), not a frame
        // slot, so it has no `dest_of` — its effect is outside the frame.
        _ => None,
    }
}

/// Flush every resident-written slot back to the frame, restore callee-saved
/// registers, and `ret`. When `ret_slot` is `Some`, the return value (slot's
/// value) is loaded into `rax` AFTER flushing; when `None` (deopt), `rax` is
/// already the status-write scratch and the returned value is irrelevant (the
/// status cell signals the side-exit).
fn emit_flush_and_return(
    g: &mut Gen,
    written: &[bool],
    used_callee: &[Reg],
    ret_slot: Option<Slot>,
) {
    // Flush resident-written slots in slot order. GP slots flush via `mov`,
    // XMM (float) slots via `movsd` — both leave the frame consistent with the
    // tree-walker's full frame state (so the region can resume / deopt).
    for s in 0..g.loc.len() {
        if written[s] {
            match g.loc[s] {
                Loc::Reg(r) => g.asm.mov_mr(BASE, (s as i32) * 8, r),
                Loc::Xmm(x) => g.asm.movsd_mr(BASE, (s as i32) * 8, x),
                Loc::Frame => {}
            }
        }
    }
    if let Some(rs) = ret_slot {
        // Load the return value bits into rax from its (now frame-consistent)
        // home. A float return slot lives in an XMM reg — bit-copy via `movq`;
        // the chain ABI returns the raw i64 bits either way.
        match g.loc[rs as usize] {
            Loc::Reg(r) => g.asm.mov_rr(S0, r),
            Loc::Xmm(x) => g.asm.movq_rx(S0, x),
            Loc::Frame => g.asm.mov_rm(S0, BASE, (rs as i32) * 8),
        }
    }
    // Undo the prologue's stack-alignment pad (if any) before restoring regs.
    if g.stack_pad != 0 {
        g.asm.add_ri(Reg::Rsp, g.stack_pad);
    }
    // Restore callee-saved regs in reverse push order, then base.
    for &r in used_callee.iter().rev() {
        g.asm.pop(r);
    }
    g.asm.pop(BASE);
    g.asm.ret();
}

/// Lower a DIRECT self-call (`CallSelf` / `CallSelfCopy`) — a REAL SysV call to
/// THIS chain's own entry. Bit-identical to `logos_stencil_call_self`(`_copy`):
///
/// 1. entry-cell load + zero guard (status = 1, the unpatched-entry marker);
/// 2. live-depth guard `depth >= MAX` (status = 5);
/// 3. arena-bound guard `callee_base + frame_size*8 > arena_end` (status = 9),
///    where `arena_end = frame[limit_slot]` and `callee_base = BASE + args*8`;
/// 4. (`CallSelfCopy`) stage `arg_count` scalar args from `src_start..` into the
///    callee window `callee_base[0..]` — read from wherever the source slots
///    live (register or frame), so a register-resident arg is staged correctly;
/// 5. plant `arena_end` into the callee's own limit slot
///    (`callee_base[frame_size-1]`);
/// 6. `*depth += 1`; spill caller-saved residents; `call [entry]`;
/// 7. `*depth -= 1`; if `*status != 0` → PROPAGATE (bare flush+return, the inner
///    exit code is already in the cell); else reload residents and store the
///    result into `dst`.
///
/// Every side exit writes its marker (or leaves the propagated code) and jumps
/// to the self-call PROPAGATION epilogue, which flushes the frame and returns
/// WITHOUT overwriting the status cell.
#[allow(clippy::too_many_arguments)]
fn emit_self_call(
    g: &mut Gen,
    idx: usize,
    dst: Slot,
    args_start: Slot,
    copy: Option<(Slot, u16)>,
    limit_slot: Slot,
    frame_size: i64,
    status_addr: i64,
    depth_addr: i64,
    entry_addr: i64,
) {
    let propagate = g.self_call.as_ref().expect("self-call without SelfCall context").propagate_label;

    // A small helper: on a failed guard, write `marker` to the status cell, then
    // jump to the propagation epilogue (which flushes + returns, no further
    // status write). Emitted as an out-of-line block reached by a jcc.
    let entry_zero = g.asm.new_label();
    let depth_bad = g.asm.new_label();
    let arena_bad = g.asm.new_label();

    // ---- 1. entry-cell load + zero guard ----
    // S0 = *entry_cell (the patched base, or 0 if somehow unpatched).
    g.asm.mov_ri(S0, entry_addr);
    g.asm.mov_rm(S0, S0, 0);
    g.asm.test_rr(S0, S0);
    g.asm.jcc(Cond::Eq, entry_zero);

    // ---- 2. depth guard: *depth >= MAX ----
    // S1 = *depth.
    g.asm.mov_ri(S1, depth_addr);
    g.asm.mov_rm(S1, S1, 0);
    g.asm.cmp_ri(S1, SELF_CALL_DEPTH_LIMIT as i32);
    g.asm.jcc(Cond::Ge, depth_bad);

    // ---- 3. arena-bound guard ----
    // SC = callee_base = BASE + args_start*8.
    g.asm.mov_rr(SC, BASE);
    g.asm.add_ri(SC, (args_start as i32) * 8);
    // S0 = arena_end = frame[limit_slot].
    g.asm.mov_rm(S0, BASE, (limit_slot as i32) * 8);
    // S1 = callee_base + frame_size*8.
    g.asm.mov_rr(S1, SC);
    g.asm.add_ri(S1, (frame_size as i32) * 8);
    g.asm.cmp_rr(S1, S0); // (callee_base + fs*8) - arena_end
    g.asm.jcc(Cond::Gt, arena_bad); // > arena_end → OOB

    // ---- 4. argument staging (CallSelfCopy) ----
    // SC holds callee_base; S0 holds arena_end (kept for step 5). Stage each
    // source arg through S1, reading the source slot from wherever it lives.
    if let Some((src_start, arg_count)) = copy {
        for j in 0..arg_count {
            g.load(S1, src_start + j); // S1 = frame-or-reg[src_start+j]
            g.asm.mov_mr(SC, (j as i32) * 8, S1); // callee_base[j] = arg
        }
    }

    // ---- 5. plant arena_end into the callee's own limit slot ----
    g.asm.mov_mr(SC, ((frame_size - 1) as i32) * 8, S0); // callee_base[fs-1] = arena_end

    // ---- 6a. *depth += 1 ----
    g.asm.mov_ri(SC, depth_addr);
    g.asm.mov_rm(S0, SC, 0);
    g.asm.add_ri(S0, 1);
    g.asm.mov_mr(SC, 0, S0);

    // ---- 6b. spill caller-saved residents to their frame slots ----
    g.spill_volatiles_at(idx);

    // ---- 6c. set up the SysV call args and CALL ----
    // rdi = callee_base = BASE + args_start*8 (recompute — SC was reused above).
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.add_ri(Reg::Rdi, (args_start as i32) * 8);
    // rsi = sp = 0 (the contiguous backend never reads the operand stack).
    g.asm.mov_ri(Reg::Rsi, 0);
    // rdx/rcx/r8/r9 = 0 (threaded register args enter zeroed; the callee's
    // prologue reloads its pinned registers from its frame).
    g.asm.mov_ri(Reg::Rdx, 0);
    g.asm.mov_ri(Reg::Rcx, 0);
    g.asm.mov_ri(Reg::R8, 0);
    g.asm.mov_ri(Reg::R9, 0);
    // call [entry_cell]: rax = *entry_cell; call rax.
    g.asm.mov_ri(S0, entry_addr);
    g.asm.mov_rm(S0, S0, 0);
    g.asm.call_r(S0); // result -> rax (S0)

    // ---- 7a. *depth -= 1 (uses rcx; rax holds the result) ----
    g.asm.mov_ri(SC, depth_addr);
    g.asm.mov_rm(S1, SC, 0);
    g.asm.sub_ri(S1, 1);
    g.asm.mov_mr(SC, 0, S1);

    // ---- 7b. status check: if *status != 0 propagate (rax irrelevant) ----
    g.asm.mov_ri(S1, status_addr);
    g.asm.mov_rm(S1, S1, 0);
    g.asm.test_rr(S1, S1);
    g.asm.jcc(Cond::Ne, propagate);

    // ---- 7c. reload caller-saved residents (their pre-call loop-carried
    // values, spilled in 6b — the callee never touches the caller's frame
    // slots). Reloads write the resident register directly (no scratch), so
    // rax (the result) survives. Only the LIVE-AFTER subset is reloaded (a
    // resident dead after the call is read by no later op). ----
    g.reload_volatiles_at(idx);

    // ---- 7d. store the result into dst (AFTER reloading, so a volatile-
    // resident dst gets the result, not its stale reloaded value). ----
    g.store(dst, S0);

    // ---- out-of-line guard-failure blocks (write marker, then propagate) ----
    let after = g.asm.new_label();
    g.asm.jmp(after);

    g.asm.bind(entry_zero);
    emit_marker_then_propagate(g, status_addr, 1, propagate);
    g.asm.bind(depth_bad);
    emit_marker_then_propagate(g, status_addr, 5, propagate);
    g.asm.bind(arena_bad);
    emit_marker_then_propagate(g, status_addr, 9, propagate);

    g.asm.bind(after);
}

/// Write `marker` to the status cell, then jump to the self-call propagation
/// epilogue (bare flush + return). Used by the pre-call self-call guards
/// (unpatched entry = 1, depth limit = 5, arena bound = 9) — the same distinct
/// markers `logos_stencil_call_self` writes.
fn emit_marker_then_propagate(g: &mut Gen, status_addr: i64, marker: i64, propagate: LabelId) {
    g.asm.mov_ri(S0, status_addr);
    g.asm.mov_ri(S1, marker);
    g.asm.mov_mr(S0, 0, S1);
    g.asm.jmp(propagate);
}

/// Lower a mode-B PRECISE self-`Call` (a REAL SysV call through the program's
/// entry table) — bit-identical to `logos_stencil_call_precise`:
///
/// 1. entry-cell load + zero guard (precise-tag exit);
/// 2. live-depth guard `depth >= MAX` (precise-tag exit);
/// 3. arena-bound guard `callee_base + (regc+3)*8 > arena_end` (precise-tag exit),
///    where `arena_end = frame[limit_slot]`, `callee_base = BASE + args_start*8`,
///    `regc = frame_size - 3` (the published table regcount);
/// 4. plant `arena_end` into the callee's own limit slot
///    (`callee_base[regc+2] = callee_base[frame_size-1]`);
/// 5. `*depth += 1`; spill caller-saved residents; `call [entry]`;
/// 6. `*depth -= 1`; if `*status != 0` → PRECISE PROPAGATE (bare flush+return,
///    the inner exit code — already a precise tag — stays in the cell); else
///    reload residents and store the result handle into `dst`.
///
/// Unlike the classic self-call, the args are PRE-STAGED by `Move` micros into
/// the disjoint callee window (`args_start = frame_size`) before this op, and the
/// plant window/resume/dst slots were written by the preceding `LoadConst`
/// micros (all frame-resident), so the precise-deopt walk can chain frames. On
/// ANY guard failure the precise epilogue writes `code | (*depth << 32)`.
#[allow(clippy::too_many_arguments)]
fn emit_precise_call(
    g: &mut Gen,
    idx: usize,
    dst: Slot,
    args_start: Slot,
    limit_slot: Slot,
    frame_size: i64,
    status_addr: i64,
    depth_addr: i64,
    entry_addr: i64,
    code: i64,
) {
    let propagate = g
        .self_call
        .as_ref()
        .expect("precise call without a propagate epilogue")
        .propagate_label;
    let fail = g
        .precise
        .as_ref()
        .map(|p| p.code_blocks[&code])
        .expect("precise call without a code block");

    // The published table regcount is frame_size - 3; the callee frame spans
    // (regc + 3) slots, exactly the stencil's `regc.wrapping_add(3)`.
    let bound_slots = frame_size; // == (frame_size - 3) + 3.

    // ---- 1. entry-cell load + zero guard ----
    g.asm.mov_ri(S0, entry_addr);
    g.asm.mov_rm(S0, S0, 0);
    g.asm.test_rr(S0, S0);
    g.asm.jcc(Cond::Eq, fail);

    // ---- 2. depth guard: *depth >= MAX ----
    g.asm.mov_ri(S1, depth_addr);
    g.asm.mov_rm(S1, S1, 0);
    g.asm.cmp_ri(S1, SELF_CALL_DEPTH_LIMIT as i32);
    g.asm.jcc(Cond::Ge, fail);

    // ---- 3. arena-bound guard ----
    // SC = callee_base = BASE + args_start*8.
    g.asm.mov_rr(SC, BASE);
    g.asm.add_ri(SC, (args_start as i32) * 8);
    // S0 = arena_end = frame[limit_slot].
    g.asm.mov_rm(S0, BASE, (limit_slot as i32) * 8);
    // S1 = callee_base + bound_slots*8.
    g.asm.mov_rr(S1, SC);
    g.asm.add_ri(S1, (bound_slots as i32) * 8);
    g.asm.cmp_rr(S1, S0);
    g.asm.jcc(Cond::Gt, fail);

    // ---- 4. plant arena_end into the callee's own limit slot ----
    g.asm.mov_mr(SC, ((frame_size - 1) as i32) * 8, S0); // callee_base[fs-1] = arena_end

    // ---- 5a. *depth += 1 ----
    g.asm.mov_ri(SC, depth_addr);
    g.asm.mov_rm(S0, SC, 0);
    g.asm.add_ri(S0, 1);
    g.asm.mov_mr(SC, 0, S0);

    // ---- 5b. spill the LIVE-AFTER caller-saved residents ----
    // A resident dead after this precise call is read by no later op AND is not
    // observed by a precise resume at this call's PC (the resume reads live-IN
    // slots, a subset of these live-after residents plus the frame-resident
    // args/limit) — so eliding its spill/reload is bit-identical.
    g.spill_volatiles_at(idx);

    // ---- 5c. SysV call args + CALL ----
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.add_ri(Reg::Rdi, (args_start as i32) * 8);
    g.asm.mov_ri(Reg::Rsi, 0);
    g.asm.mov_ri(Reg::Rdx, 0);
    g.asm.mov_ri(Reg::Rcx, 0);
    g.asm.mov_ri(Reg::R8, 0);
    g.asm.mov_ri(Reg::R9, 0);
    g.asm.mov_ri(S0, entry_addr);
    g.asm.mov_rm(S0, S0, 0);
    g.asm.call_r(S0); // result -> rax (S0)

    // ---- 6a. *depth -= 1 (rcx scratch; rax holds the result) ----
    g.asm.mov_ri(SC, depth_addr);
    g.asm.mov_rm(S1, SC, 0);
    g.asm.sub_ri(S1, 1);
    g.asm.mov_mr(SC, 0, S1);

    // ---- 6b. status check: if *status != 0 propagate (rax irrelevant) ----
    g.asm.mov_ri(S1, status_addr);
    g.asm.mov_rm(S1, S1, 0);
    g.asm.test_rr(S1, S1);
    g.asm.jcc(Cond::Ne, propagate);

    // ---- 6c. reload the LIVE-AFTER caller-saved residents ----
    g.reload_volatiles_at(idx);

    // ---- 6d. store the result handle into dst ----
    g.store(dst, S0);
}

/// Lower one supported op.
fn emit_op(
    g: &mut Gen,
    op: &MicroOp,
    idx: usize,
    written: &[bool],
    used_callee: &[Reg],
) -> Option<()> {
    match *op {
        MicroOp::LoadConst { dst, value } => {
            // Materialize the raw bits in a GP scratch; if the dst is XMM-
            // resident (a float const like `0.0`), bridge through `movq`.
            g.asm.mov_ri(S0, value);
            if let Loc::Xmm(_) = g.loc[dst as usize] {
                g.asm.movq_xr(FS0, S0);
                g.fstore(dst, FS0);
            } else {
                g.store(dst, S0);
            }
        }
        MicroOp::Move { dst, src } => {
            // A raw-bits copy. If EITHER end is XMM-resident, route through an
            // XMM scratch (fload/fstore bit-preserve across all locations);
            // otherwise the GP path (a direct reg move when both resident).
            if matches!(g.loc[dst as usize], Loc::Xmm(_))
                || matches!(g.loc[src as usize], Loc::Xmm(_))
            {
                let x = g.foperand(src, FS0);
                g.fstore(dst, x);
            } else {
                let r = g.operand(src, S0);
                g.store(dst, r);
            }
        }
        MicroOp::Add { dst, lhs, rhs } => emit_commutative(g, dst, lhs, rhs, AsmBinop::Add),
        MicroOp::Sub { dst, lhs, rhs } => emit_sub(g, dst, lhs, rhs),
        MicroOp::Mul { dst, lhs, rhs } => emit_commutative(g, dst, lhs, rhs, AsmBinop::Mul),
        MicroOp::BitAnd { dst, lhs, rhs } => emit_commutative(g, dst, lhs, rhs, AsmBinop::And),
        MicroOp::BitOr { dst, lhs, rhs } => emit_commutative(g, dst, lhs, rhs, AsmBinop::Or),
        MicroOp::BitXor { dst, lhs, rhs } => emit_commutative(g, dst, lhs, rhs, AsmBinop::Xor),
        MicroOp::Lt { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Lt),
        MicroOp::Gt { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Gt),
        MicroOp::LtEq { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Le),
        MicroOp::GtEq { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Ge),
        MicroOp::Eq { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Eq),
        MicroOp::Neq { dst, lhs, rhs } => emit_compare(g, dst, lhs, rhs, Cond::Ne),
        MicroOp::NotInt { dst, src } => {
            g.load(S0, src);
            g.asm.not_r(S0);
            g.store(dst, S0);
        }
        MicroOp::NotBool { dst, src } => {
            // dst = src ^ 1
            g.load(S0, src);
            g.asm.mov_ri(S1, 1);
            g.asm.xor_rr(S0, S1);
            g.store(dst, S0);
        }
        MicroOp::Shl { dst, lhs, rhs } => emit_shift(g, dst, lhs, rhs, ShiftKind::Left),
        MicroOp::Shr { dst, lhs, rhs } => emit_shift(g, dst, lhs, rhs, ShiftKind::Right),
        MicroOp::DivPow2 { dst, lhs, k } => emit_div_pow2(g, dst, lhs, k),
        MicroOp::MagicDivU { dst, lhs, magic, more, mul_back } => {
            emit_magic_div(g, dst, lhs, magic, more, mul_back)
        }
        MicroOp::Div { dst, lhs, rhs } => {
            let dl = g.checked_exit(idx).expect("Div without a deopt label");
            emit_div_mod(g, dst, lhs, rhs, true, dl);
        }
        MicroOp::Mod { dst, lhs, rhs } => {
            let dl = g.checked_exit(idx).expect("Mod without a deopt label");
            emit_div_mod(g, dst, lhs, rhs, false, dl);
        }
        MicroOp::Branch { cmp, lhs, rhs, target } => {
            // Transfer to `target` when cmp(lhs, rhs) is FALSE; fall through
            // when TRUE. So jump on the NEGATED condition.
            let a = g.operand(lhs, S0);
            let b = g.operand(rhs, S1);
            g.asm.cmp_rr(a, b);
            let neg = cond_of(cmp.negated());
            g.asm.jcc(neg, g.op_labels[target]);
        }
        MicroOp::Jump { target } => {
            if target != idx + 1 {
                g.asm.jmp(g.op_labels[target]);
            }
        }
        MicroOp::JumpIfFalse { cond, target } => {
            let r = g.operand(cond, S0);
            g.asm.test_rr(r, r);
            g.asm.jcc(Cond::Eq, g.op_labels[target]); // ZF set => value == 0
        }
        MicroOp::JumpIfTrue { cond, target } => {
            let r = g.operand(cond, S0);
            g.asm.test_rr(r, r);
            g.asm.jcc(Cond::Ne, g.op_labels[target]); // ZF clear => value != 0
        }
        MicroOp::ArrLoad { dst, idx: idx_slot, ptr_slot, len_slot, byte: false, checked } => {
            // The exit label is only consulted for a CHECKED access; an unchecked
            // load needs none (and a region with no checked op has no label).
            let dl = g.checked_exit(idx);
            emit_arr_load(g, dst, idx_slot, ptr_slot, len_slot, checked, dl);
        }
        MicroOp::ArrStore { src, idx: idx_slot, ptr_slot, len_slot, byte: false, checked } => {
            let dl = g.checked_exit(idx);
            emit_arr_store(g, src, idx_slot, ptr_slot, len_slot, checked, dl);
        }
        MicroOp::ArrLoad { dst, idx: idx_slot, ptr_slot, len_slot, byte: true, checked } => {
            let dl = g.checked_exit(idx);
            emit_arr_load_byte(g, dst, idx_slot, ptr_slot, len_slot, checked, dl);
        }
        MicroOp::ArrStore { src, idx: idx_slot, ptr_slot, len_slot, byte: true, checked } => {
            let dl = g.checked_exit(idx);
            emit_arr_store_byte(g, src, idx_slot, ptr_slot, len_slot, checked, dl);
        }
        // ---- LIST MUTATION (wave 13) — helper calls into the JIT runtime. ----
        MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, helper_addr, byte } => {
            emit_list_push(g, idx, src, vec_slot, ptr_slot, len_slot, helper_addr, byte);
        }
        MicroOp::ListClear { vec_slot, ptr_slot, len_slot, helper_addr } => {
            emit_list_clear(g, idx, vec_slot, ptr_slot, len_slot, helper_addr);
        }
        MicroOp::StrAppend { text_handle_slot, src, helper_addr } => {
            emit_str_append(g, idx, text_handle_slot, src, helper_addr);
        }
        // ---- MODE-B list machinery (precise path). The fresh-list-RETURN
        // recursion shape (mergesort): `NewList` allocates a registry-owned fresh
        // buffer and plants its pin triple, `ArrPush` (above) appends with a
        // realloc refresh, and `ListTriple` refreshes a triple from a live handle
        // (a self-call's returned list). ----
        MicroOp::NewList { vec_slot, ptr_slot, len_slot, helper_addr } => {
            emit_new_list(g, idx, vec_slot, ptr_slot, len_slot, helper_addr);
        }
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, helper_addr } => {
            emit_list_triple(g, idx, handle_slot, vec_slot, ptr_slot, len_slot, helper_addr);
        }
        // ---- FLOAT (f64 / XMM) ops. IEEE; no FMA; no reassociation. ----
        MicroOp::AddF { dst, lhs, rhs } => emit_fbinop(g, dst, lhs, rhs, FBinop::Add),
        MicroOp::SubF { dst, lhs, rhs } => emit_fbinop(g, dst, lhs, rhs, FBinop::Sub),
        MicroOp::MulF { dst, lhs, rhs } => emit_fbinop(g, dst, lhs, rhs, FBinop::Mul),
        MicroOp::DivF { dst, lhs, rhs } => {
            let dl = g.checked_exit(idx).expect("DivF without a deopt label");
            emit_fdiv(g, dst, lhs, rhs, dl);
        }
        MicroOp::SqrtF { dst, src } => {
            let s = g.foperand(src, FS1);
            g.asm.sqrtsd_rr(FS0, s);
            g.fstore(dst, FS0);
        }
        MicroOp::IntToFloat { dst, src } => {
            // cvtsi2sd needs a GP source; materialize the int into S0.
            let r = g.operand(src, S0);
            g.asm.cvtsi2sd(FS0, r);
            g.fstore(dst, FS0);
        }
        MicroOp::FmaF { dst, a, b, c } => {
            // TWO IEEE roundings (mulsd THEN addsd), bit-identical to the
            // reference `(a*b) + c`. NEVER a fused `vfmadd` (one rounding).
            g.fload(FS0, a); // FS0 = a
            let bv = g.foperand(b, FS1);
            g.asm.mulsd_rr(FS0, bv); // FS0 = a*b (rounded once)
            let cv = g.foperand(c, FS1);
            g.asm.addsd_rr(FS0, cv); // FS0 = (a*b) + c (rounded again)
            g.fstore(dst, FS0);
        }
        MicroOp::LtF { dst, lhs, rhs } => emit_fcompare(g, dst, lhs, rhs, FCmp::Lt),
        MicroOp::GtF { dst, lhs, rhs } => emit_fcompare(g, dst, lhs, rhs, FCmp::Gt),
        MicroOp::LtEqF { dst, lhs, rhs } => emit_fcompare(g, dst, lhs, rhs, FCmp::Le),
        MicroOp::GtEqF { dst, lhs, rhs } => emit_fcompare(g, dst, lhs, rhs, FCmp::Ge),
        MicroOp::EqF { dst, lhs, rhs } => emit_feq(g, dst, lhs, rhs, false),
        MicroOp::NeqF { dst, lhs, rhs } => emit_feq(g, dst, lhs, rhs, true),
        MicroOp::BranchF { cmp, lhs, rhs, target } => {
            emit_branchf(g, cmp, lhs, rhs, g.op_labels[target]);
        }
        MicroOp::Return { src } => {
            emit_flush_and_return(g, written, used_callee, Some(src));
        }
        _ => return None,
    }
    Some(())
}

/// Compute the 1-based array element address into [`S0`], side-exiting to the
/// deopt epilogue on a CHECKED out-of-bounds index — bit-identical to the
/// `ST_ARRLD`/`ST_ARRLDB`/`ST_ARRST`/`ST_ARRSTB` stencils: `im1 = idx - 1`
/// (wrapping); a checked access exits when `(im1 as u64) >= (len as u64)` (the
/// unsigned compare catches both the 0/negative index — `im1` wraps huge — and
/// the over-length case); `addr = ptr + im1 * stride`, where `stride` is 8 for
/// 8-byte int/float elements (`byte == false`) and 1 for 1-byte `Seq of Bool`
/// elements (`byte == true`). After this returns the address is in `S0` and the
/// length/pointer scratch (`S1`) is dead.
fn emit_arr_addr(g: &mut Gen, idx: Slot, ptr_slot: Slot, len_slot: Slot, checked: bool, byte: bool, dl: Option<LabelId>) {
    // SCALED-INDEX CSE FAST PATH. When the reserved register already holds the
    // 0-based index `im1 = frame[idx] - 1` for THIS index slot at THIS element
    // stride (a hit established by an earlier access in this straight-line run),
    // skip the index reload + decrement. The bounds check still runs PER ACCESS
    // (each array has its own length) against the cached TRUE `im1`, so an
    // out-of-bounds index — including the wrapped huge value a 0/negative index
    // produces — side-exits identically; only the `idx - 1` (and, with the second
    // reserved register, the `* 8` scaling) arithmetic is shared.
    if let (Some(im1_reg), Some(state)) = (g.off_im1_reg, g.off_cache) {
        if state.idx == idx && state.byte == byte {
            if checked {
                let dl = dl.expect("checked array op without a deopt label");
                // The invariant length: a hoisted len register (loop pre-header
                // load) when active, else the per-access frame reload.
                let len = g.hoisted_len(ptr_slot).unwrap_or_else(|| g.operand(len_slot, S1));
                g.asm.cmp_rr(im1_reg, len); // im1 - len
                g.asm.jcc(Cond::AeU, dl); // (im1 as u64) >= (len as u64) → OOB
            }
            // addr = ptr + im1 * stride, reusing the cached index. A byte stride
            // is `im1` itself; an 8-byte stride reuses the cached scaled offset
            // (second register) or re-shifts the cached `im1` (one register).
            if byte {
                g.asm.mov_rr(S0, im1_reg);
            } else if let Some(scaled_reg) = g.off_scaled_reg {
                g.asm.mov_rr(S0, scaled_reg); // S0 = im1 * 8 (cached)
            } else {
                g.asm.mov_rr(S0, im1_reg);
                g.asm.shl_ri(S0, 3); // im1 * 8
            }
            // The invariant base pointer: the hoisted ptr register when active,
            // else the per-access frame reload.
            let ptr = g.hoisted_ptr(ptr_slot).unwrap_or_else(|| g.operand(ptr_slot, S1));
            g.asm.add_rr(S0, ptr); // S0 = ptr + im1*stride
            return;
        }
    }

    // COMPUTE PATH (cache miss or CSE disabled): the literal per-access lowering,
    // additionally populating the reserved register(s) so the next access in the
    // run hits the fast path above. Bit-identical to the stencil tier.
    // S0 = idx; S0 -= 1  → im1.
    g.load(S0, idx);
    g.asm.sub_ri(S0, 1);
    // Cache the freshly computed `im1` (the 0-based index) for reuse.
    if let Some(im1_reg) = g.off_im1_reg {
        g.asm.mov_rr(im1_reg, S0);
        g.off_cache = Some(OffCacheState { idx, byte });
    }
    if checked {
        let dl = dl.expect("checked array op without a deopt label");
        let len = g.hoisted_len(ptr_slot).unwrap_or_else(|| g.operand(len_slot, S1));
        g.asm.cmp_rr(S0, len); // im1 - len
        g.asm.jcc(Cond::AeU, dl); // (im1 as u64) >= (len as u64) → OOB
    }
    // addr = ptr + im1 * stride. Byte (Bool) elements are 1-byte: no scaling.
    if !byte {
        g.asm.shl_ri(S0, 3); // im1 * 8 (8-byte i64/f64 elements)
        // Cache the scaled offset too (second register), so a reused 8-byte
        // access skips the shift as well.
        if g.off_im1_reg.is_some() {
            if let Some(scaled_reg) = g.off_scaled_reg {
                g.asm.mov_rr(scaled_reg, S0);
            }
        }
    }
    let ptr = g.hoisted_ptr(ptr_slot).unwrap_or_else(|| g.operand(ptr_slot, S1));
    g.asm.add_rr(S0, ptr); // S0 = ptr + im1*stride
}

/// `frame[dst] = buffer[frame[idx] - 1]` (8-byte int element). The loaded value
/// goes through scratch `S1` so a resident `dst` is written with a register move.
/// A CHECKED out-of-bounds index jumps to `dl` (the classic deopt epilogue or, in
/// the precise path, this op's tag-staging block).
fn emit_arr_load(g: &mut Gen, dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot, checked: bool, dl: Option<LabelId>) {
    emit_arr_addr(g, idx, ptr_slot, len_slot, checked, false, dl);
    g.asm.mov_rm(S1, S0, 0); // S1 = *addr
    g.store(dst, S1);
}

/// `frame[dst] = buffer[frame[idx] - 1] as i64` over 1-BYTE (`Seq of Bool`)
/// elements — bit-identical to `logos_stencil_arrldb`: a zero-extended 1-byte
/// load (`movzx`), so the loaded `u8` (0..=255, in practice 0/1) widens to a
/// NON-NEGATIVE i64. The loaded value goes through scratch `S1` so a resident
/// `dst` is written with a register move; OOB side-exit is identical to the
/// 8-byte path (same `im1`/length unsigned guard).
fn emit_arr_load_byte(g: &mut Gen, dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot, checked: bool, dl: Option<LabelId>) {
    emit_arr_addr(g, idx, ptr_slot, len_slot, checked, true, dl);
    g.asm.movzx_rm8(S1, S0, 0); // S1 = *(u8*)addr (zero-extended)
    g.store(dst, S1);
}

/// `buffer[frame[idx] - 1] = frame[src]` (8 raw bytes). The stored value is the
/// slot's raw bits, so a FLOAT-resident (XMM) source is bit-copied to a GP
/// register via `movq` first — the buffer holds the same 8 bytes either way. A
/// CHECKED store side-exits BEFORE the write (to `dl`), so an out-of-bounds store
/// leaves the buffer untouched (the reference and stencil contract).
fn emit_arr_store(g: &mut Gen, src: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot, checked: bool, dl: Option<LabelId>) {
    emit_arr_addr(g, idx, ptr_slot, len_slot, checked, false, dl);
    // The address is in S0 and the pointer scratch S1 is dead. Materialize the
    // source bits into a GP reg that cannot collide with the address: SC (rcx)
    // for a GP/frame source, or S1 (the dead pointer scratch) for an XMM source.
    let v = match g.loc[src as usize] {
        Loc::Xmm(x) => {
            g.asm.movq_rx(S1, x);
            S1
        }
        _ => g.operand(src, SC),
    };
    g.asm.mov_mr(S0, 0, v); // *addr = value
}

/// `buffer[frame[idx] - 1] = (frame[src] != 0) as u8` over 1-BYTE (`Seq of Bool`)
/// elements — bit-identical to `logos_stencil_arrstb`: the stored byte is the
/// BOOLEAN NORMALIZATION of the value (1 when nonzero, else 0), NOT its raw low
/// byte. A CHECKED store side-exits BEFORE the write (to `dl`), so an
/// out-of-bounds store leaves the buffer untouched.
///
/// The normalization is `test v, v; setne S1l` — `S1` (rdx) is the dead pointer
/// scratch after the address is in `S0`, and its low byte `dl` is REX-free
/// addressable. Only that low byte is then stored. An XMM-resident source is
/// bit-copied to `SC` (rcx) first so its full 64 bits feed the zero test (a
/// nonzero f64 bit pattern — incl. `-0.0` and NaN — stores 1, matching `!= 0`).
fn emit_arr_store_byte(g: &mut Gen, src: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot, checked: bool, dl: Option<LabelId>) {
    emit_arr_addr(g, idx, ptr_slot, len_slot, checked, true, dl);
    // Materialize the source value into a GP reg distinct from the address (S0).
    // SC (rcx) for a GP/frame source; for an XMM source, bit-copy to SC.
    let v = match g.loc[src as usize] {
        Loc::Xmm(x) => {
            g.asm.movq_rx(SC, x);
            SC
        }
        _ => g.operand(src, SC),
    };
    // (v != 0) → S1's low byte via test + setne; store that one byte.
    g.asm.test_rr(v, v);
    g.asm.setcc8(Cond::Ne, S1);
    g.asm.mov_mr8(S0, 0, S1); // *(u8*)addr = (v != 0) as u8
}

/// Lower a pinned-list PUSH (`ArrPush`) with an INLINE FAST PATH plus a cold
/// realloc helper call — the V8-style array-push lowering.
///
/// The common case (the appended element fits the current capacity) runs ENTIRELY
/// in scratch registers, with NO SysV call and NO caller-saved spill/reload:
///
/// ```text
///     vec = frame[vec_slot]            // the live *mut Vec
///     len = frame[len_slot]            // the mirrored length
///     if (len as u64) < (*(vec+cap_off) as u64):    // spare capacity?
///         ptr = frame[ptr_slot]
///         ptr[len] = value             // store (8-byte raw, or (v!=0) byte)
///         len += 1
///         frame[len_slot] = len        // refresh the mirror
///         *(vec+len_off) = len         //   AND the real Vec::len field
///         goto join
///     // else fall through to the cold realloc path:
///     <spill caller-saved>; helper(frame, vec, ptr, len, value); <reload>
///   join:
/// ```
///
/// SOUNDNESS — bit-identical to `logos_rt_push_*` (and thus to `Vec::push` and the
/// tree-walker) in every case:
/// * The capacity test uses the SAME `cap`/`len` the helper would (`(*vec).len() <
///   (*vec).capacity()` is exactly `Vec::push`'s "no grow" condition), read from
///   the LIVE `Vec` at PROBED field offsets ([`vec_layout`]) — never a hardcoded
///   layout — so it is correct on this exact binary.
/// * On the fast path the buffer does NOT move, so `frame[ptr_slot]` stays valid
///   (the helper would not refresh it either) and the store lands at the identical
///   address `ptr + len*stride` the helper's `(*vec).push` writes.
/// * BOTH the mirrored `frame[len_slot]` AND the real `Vec::len` field are bumped,
///   so a later non-fast access (a subsequent realloc helper push, a deopt that
///   materializes/keeps the live `Rc<Vec>`, or the `Vec`'s eventual drop) observes
///   the appended element exactly as after `Vec::push`.
/// * The cold path is the ORIGINAL helper-call lowering verbatim: on a realloc
///   (`len == cap`) the helper reallocates and refreshes `frame[ptr_slot]` /
///   `frame[len_slot]` (and the real `Vec` fields), so the next access reads the
///   fresh pointer — the realloc-coherence contract is unchanged.
/// * The fast path touches ONLY the reserved scratch registers `S0`/`S1`/`SC`
///   (`rax`/`rdx`/`rcx`), which are NEVER assigned to a resident slot, so it
///   clobbers no loop-carried value and needs no spill/reload. The cold path keeps
///   the full caller-saved spill discipline.
/// * The vec/ptr/len triple is forced FRAME-resident (`force_frame_set`), so both
///   paths read/write the same frame cells and the helper's view agrees.
fn emit_list_push(
    g: &mut Gen,
    idx: usize,
    src: Slot,
    vec_slot: Slot,
    ptr_slot: Slot,
    len_slot: Slot,
    helper_addr: i64,
    byte: bool,
) {
    // Kill-switch / A-B toggle: fall back to the always-call helper lowering.
    if !inline_push_enabled() {
        g.spill_volatiles_at(idx);
        g.load(Reg::R8, src);
        g.asm.mov_rr(Reg::Rdi, BASE);
        g.asm.mov_ri(Reg::Rsi, vec_slot as i64);
        g.asm.mov_ri(Reg::Rdx, ptr_slot as i64);
        g.asm.mov_ri(Reg::Rcx, len_slot as i64);
        g.asm.mov_ri(S0, helper_addr);
        g.asm.call_r(S0);
        g.reload_volatiles_at(idx);
        return;
    }

    let layout = vec_layout();
    let cold = g.asm.new_label();
    let join = g.asm.new_label();

    // ---- FAST PATH ---- (scratch-only; no spill, no call when len < cap)
    // S0 = vec handle (kept live for the real-len writeback); S1 = len; SC = cap.
    g.asm.mov_rm(S0, BASE, (vec_slot as i32) * 8);
    g.asm.mov_rm(S1, BASE, (len_slot as i32) * 8);
    g.asm.mov_rm(SC, S0, layout.cap_off); // SC = (*vec).capacity
    g.asm.cmp_rr(S1, SC); // len - cap
    g.asm.jcc(Cond::AeU, cold); // (len as u64) >= (cap as u64) -> realloc (cold)

    // Materialize the pushed value into SC (cap is dead). An XMM-resident
    // float-list value is bit-copied via movq — the raw 8 bytes travel to the
    // buffer exactly as the helper's `f64::from_bits(...).push(...)` round-trip.
    match g.loc[src as usize] {
        Loc::Xmm(x) => g.asm.movq_rx(SC, x),
        _ => {
            let v = g.operand(src, SC);
            if v != SC {
                g.asm.mov_rr(SC, v);
            }
        }
    }
    // addr = ptr + len*stride, computed into S0 (vec dead now; reloaded below for
    // the real-len writeback). S1 still holds `len`.
    g.asm.mov_rm(S0, BASE, (ptr_slot as i32) * 8); // S0 = ptr
    if !byte {
        g.asm.shl_ri(S1, 3); // len * 8 (8-byte i64/f64 elements)
    }
    g.asm.add_rr(S0, S1); // S0 = ptr + len*stride  (S1 now dead)
    if byte {
        // 1-byte (`Seq of Bool`) store: the BOOLEAN NORMALIZATION (v != 0) as u8,
        // matching `logos_rt_push_bool`'s `(*vec).push(value != 0)`.
        g.asm.test_rr(SC, SC);
        g.asm.setcc8(Cond::Ne, S1);
        g.asm.mov_mr8(S0, 0, S1); // *(u8*)addr = (value != 0) as u8
    } else {
        g.asm.mov_mr(S0, 0, SC); // *addr = value (8 raw bytes)
    }
    // Bump the length: new_len = old_len + 1. Refresh BOTH the mirrored frame
    // slot AND the real `Vec::len` field so every later view is coherent.
    g.asm.mov_rm(S1, BASE, (len_slot as i32) * 8); // reload old len
    g.asm.add_ri(S1, 1); // new_len
    g.asm.mov_mr(BASE, (len_slot as i32) * 8, S1); // frame[len_slot] = new_len
    g.asm.mov_rm(S0, BASE, (vec_slot as i32) * 8); // reload vec handle
    g.asm.mov_mr(S0, layout.len_off, S1); // (*vec).len = new_len
    g.asm.jmp(join);

    // ---- COLD PATH ---- (realloc: the original helper-call lowering verbatim).
    g.asm.bind(cold);
    // Spill the LIVE-AFTER caller-saved residents to their frame slots FIRST (the
    // helper clobbers them). This must precede touching any arg register: the arg
    // registers (rdi/rsi/rdx/rcx/r8) are themselves in the caller-saved pool, so a
    // live resident may sit in one of them. Spilling reads each resident's
    // register (still intact) into the frame; only AFTER that is it safe to
    // overwrite those registers with call args.
    g.spill_volatiles_at(idx);
    // Materialize the pushed value into the value-arg register (r8) AFTER the
    // spill (so no resident's value is lost). XMM-resident floats bit-copy.
    g.load(Reg::R8, src);
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.mov_ri(Reg::Rsi, vec_slot as i64);
    g.asm.mov_ri(Reg::Rdx, ptr_slot as i64);
    g.asm.mov_ri(Reg::Rcx, len_slot as i64);
    g.asm.mov_ri(S0, helper_addr);
    g.asm.call_r(S0);
    g.reload_volatiles_at(idx);

    g.asm.bind(join);
}

/// Lower a pinned-list in-place CLEAR (`ListClear`) as a SysV call to
/// `logos_rt_clear_*(frame, vec_slot, ptr_slot, len_slot)`: truncate the
/// SOLE-OWNED buffer to empty (keep its capacity) and refresh
/// `frame[ptr_slot]`/`frame[len_slot]` (length → 0). The buffer-reuse
/// alias-safety is established UPSTREAM in the micro-op lowering (a
/// `Op::NewEmptyList` whose handle escapes via a live `Move` declines to emit
/// `ListClear`), so a `ListClear` reaching here is provably unaliased. Same
/// frame-resident-triple + caller-saved spill discipline as the push, minus the
/// value argument.
fn emit_list_clear(g: &mut Gen, idx: usize, vec_slot: Slot, ptr_slot: Slot, len_slot: Slot, helper_addr: i64) {
    g.spill_volatiles_at(idx);
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.mov_ri(Reg::Rsi, vec_slot as i64);
    g.asm.mov_ri(Reg::Rdx, ptr_slot as i64);
    g.asm.mov_ri(Reg::Rcx, len_slot as i64);
    g.asm.mov_ri(S0, helper_addr);
    g.asm.call_r(S0);
    g.reload_volatiles_at(idx);
}

/// Lower a fresh-list ALLOCATION (`NewList`, mode B) as a SysV call to
/// `logos_rt_alloc_list_i64(frame, vec_slot, ptr_slot, len_slot)`: the helper
/// `Box`-allocates a fresh empty `Vec`, REGISTERS it in the thread-local
/// allocation registry (so a deopt drains it and a `Return` detaches it), and
/// plants the pin triple — writing `frame[vec_slot]` (the live `*mut Vec`),
/// `frame[ptr_slot]` (its buffer pointer) and `frame[len_slot]` (0). The triple
/// is forced FRAME-resident (so the helper's frame writes are the single source
/// of truth and every later push/load reads the fresh pointer/length); the call
/// takes no value argument. Same caller-saved spill discipline as the push/clear
/// helpers — bit-identical to the per-piece tier's `ST_ALLOCLIST` stencil.
fn emit_new_list(g: &mut Gen, idx: usize, vec_slot: Slot, ptr_slot: Slot, len_slot: Slot, helper_addr: i64) {
    g.spill_volatiles_at(idx);
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.mov_ri(Reg::Rsi, vec_slot as i64);
    g.asm.mov_ri(Reg::Rdx, ptr_slot as i64);
    g.asm.mov_ri(Reg::Rcx, len_slot as i64);
    g.asm.mov_ri(S0, helper_addr);
    g.asm.call_r(S0);
    g.reload_volatiles_at(idx);
}

/// Lower a triple-plant (`ListTriple`, mode B) as a SysV call to
/// `logos_rt_list_triple(frame, handle_slot, vec_slot, ptr_slot, len_slot)`: read
/// `frame[handle_slot]` (a live `*mut Vec`) and refresh the pin triple. The
/// handle and triple slots are all forced FRAME-resident (so the helper's frame
/// view is the source of truth). Same caller-saved spill discipline; the handle
/// slot rides the fifth SysV arg (`r8`).
fn emit_list_triple(
    g: &mut Gen,
    idx: usize,
    handle_slot: Slot,
    vec_slot: Slot,
    ptr_slot: Slot,
    len_slot: Slot,
    helper_addr: i64,
) {
    g.spill_volatiles_at(idx);
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.mov_ri(Reg::Rsi, handle_slot as i64);
    g.asm.mov_ri(Reg::Rdx, vec_slot as i64);
    g.asm.mov_ri(Reg::Rcx, ptr_slot as i64);
    g.asm.mov_ri(Reg::R8, len_slot as i64);
    g.asm.mov_ri(S0, helper_addr);
    g.asm.call_r(S0);
    g.reload_volatiles_at(idx);
}

/// Lower a pinned MUTABLE-Text append (`StrAppend`) as a SysV call to the runtime
/// helper `logos_rt_str_append(frame, handle_slot, src, src_len)`: the helper
/// reads `frame[handle_slot]` (a live `*mut Value` to the VM accumulator cell)
/// and grows the accumulator with EXACTLY the VM's `add_assign` (in-place when
/// sole-owned, copy-on-write otherwise) semantics. The handle slot is forced
/// FRAME-resident (the helper indexes it). The source is either a 1-char frame
/// BYTE — read AFTER the spill into the value-arg register `rdx`, with `rcx` =
/// `-1` flagging the byte form — or a baked constant byte slice (`rdx` = the
/// `'static` pointer, `rcx` = its length). Same caller-saved spill discipline as
/// the list-push helper.
fn emit_str_append(
    g: &mut Gen,
    idx: usize,
    handle_slot: Slot,
    src: crate::jit::StrSrc,
    helper_addr: i64,
) {
    // Spill the LIVE-AFTER caller-saved residents FIRST (the helper clobbers
    // them; the arg registers are themselves caller-saved). The byte-form value
    // is read AFTER the spill (so no resident is lost) but BEFORE the arg
    // registers overwrite the register the source might still occupy.
    g.spill_volatiles_at(idx);
    match src {
        crate::jit::StrSrc::Byte(s) => {
            g.load(Reg::Rdx, s);
            g.asm.mov_ri(Reg::Rcx, -1);
        }
        crate::jit::StrSrc::Const { ptr, len } => {
            g.asm.mov_ri(Reg::Rdx, ptr);
            g.asm.mov_ri(Reg::Rcx, len);
        }
    }
    g.asm.mov_rr(Reg::Rdi, BASE);
    g.asm.mov_ri(Reg::Rsi, handle_slot as i64);
    g.asm.mov_ri(S0, helper_addr);
    g.asm.call_r(S0);
    g.reload_volatiles_at(idx);
}

#[derive(Clone, Copy)]
enum AsmBinop {
    Add,
    Mul,
    And,
    Or,
    Xor,
}

/// `dst = lhs OP rhs` for a commutative op. Compute in S0 to avoid clobbering a
/// resident operand that may be read again later.
fn emit_commutative(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, op: AsmBinop) {
    g.load(S0, lhs);
    let b = g.operand(rhs, S1);
    match op {
        AsmBinop::Add => g.asm.add_rr(S0, b),
        AsmBinop::Mul => g.asm.imul_rr(S0, b),
        AsmBinop::And => g.asm.and_rr(S0, b),
        AsmBinop::Or => g.asm.or_rr(S0, b),
        AsmBinop::Xor => g.asm.xor_rr(S0, b),
    }
    g.store(dst, S0);
}

/// `dst = lhs - rhs` (non-commutative). lhs into S0, then `sub S0, rhs`.
fn emit_sub(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot) {
    g.load(S0, lhs);
    let b = g.operand(rhs, S1);
    g.asm.sub_rr(S0, b);
    g.store(dst, S0);
}

/// `dst = (lhs CMP rhs) as i64` — `cmp` then `setcc`/`movzx`.
fn emit_compare(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, cond: Cond) {
    let a = g.operand(lhs, S0);
    let b = g.operand(rhs, S1);
    g.asm.cmp_rr(a, b);
    g.asm.setcc_movzx(cond, S0);
    g.store(dst, S0);
}

#[derive(Clone, Copy)]
enum FBinop {
    Add,
    Sub,
    Mul,
}

/// `dst = lhs OP rhs` for an f64 arithmetic op (`addsd`/`subsd`/`mulsd`).
///
/// The instruction order is ALWAYS the source order — `OP target, second`
/// computes `target OP second` — so a non-commutative `subsd` keeps `lhs - rhs`
/// and the result is bit-identical to the reference regardless of which fast
/// path fires. No reassociation, no FMA.
///
/// When `dst` is XMM-resident we compute IN PLACE in `dst`, avoiding the
/// `fload`→FS0 + `fstore`→dst scratch round-trip (two extra `movsd`s) that the
/// scratch path pays every iteration on a loop-carried accumulator (nbody's
/// dx/dy/dz/dist, mandelbrot's zr/zi):
/// - `dst == lhs` (in-place `x = x OP rhs`): just `OP dst, rhs` — no moves;
/// - `dst != lhs` and `dst != rhs`: `movsd dst, lhs; OP dst, rhs` — the result
///   lands in `dst` directly (one move, no store).
/// The remaining case (`dst` resident, `dst == rhs`, `dst != lhs`) cannot write
/// `dst` first without clobbering `rhs`, so it falls through to FS0 staging.
fn emit_fbinop(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, op: FBinop) {
    let apply = |asm: &mut Asm, target: Xmm, second: Xmm| match op {
        FBinop::Add => asm.addsd_rr(target, second),
        FBinop::Sub => asm.subsd_rr(target, second),
        FBinop::Mul => asm.mulsd_rr(target, second),
    };
    if let Loc::Xmm(d) = g.loc[dst as usize] {
        // Resolve rhs to a concrete register first (its own resident XMM, or
        // FS1 if frame-resident). FS1 is scratch and never a slot's residence,
        // so a frame-resident rhs never collides with `d`.
        let b = g.foperand(rhs, FS1);
        if g.loc[lhs as usize] == Loc::Xmm(d) {
            // In-place accumulate: dst and lhs ARE the same register.
            apply(&mut g.asm, d, b);
            return;
        }
        if d != b {
            // Materialize lhs into dst, then op against rhs — result in dst.
            g.fload(d, lhs);
            apply(&mut g.asm, d, b);
            return;
        }
        // d == b (dst aliases rhs, dst != lhs): writing dst first would lose
        // rhs; fall through to the scratch path, which reads rhs (b == d) before
        // overwriting dst.
        g.fload(FS0, lhs);
        apply(&mut g.asm, FS0, b);
        g.fstore(dst, FS0);
        return;
    }
    g.fload(FS0, lhs);
    let b = g.foperand(rhs, FS1);
    apply(&mut g.asm, FS0, b);
    g.fstore(dst, FS0);
}

/// `dst = lhs / rhs` (f64), side-EXITING to the deopt epilogue when the divisor
/// is `0.0` — bit-identical to the reference (`b == 0.0 -> None`). The IEEE
/// `0.0 == -0.0` makes BOTH zeros trip it: the divisor bits are compared to 0.0
/// via `ucomisd`, and `ucomisd -0.0, 0.0` sets ZF (equal), so `je deopt` fires
/// for `-0.0` too. NaN divisor is NOT zero (ucomisd sets PF, ZF — but `je` on a
/// NaN-vs-0 compare: NaN is unordered → ZF=1 → would wrongly deopt). Guard the
/// NaN case: only deopt when the unordered flag is CLEAR (an ordered equality).
fn emit_fdiv(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, dl: LabelId) {
    // Zero-divisor side-exit: compare rhs to 0.0. Build 0.0 in FS1 (xorps would
    // need a new encoder; instead movq from a zeroed GP reg).
    let b = g.foperand(rhs, FS1);
    // FS0 = 0.0 via movq from a zeroed GP scratch.
    g.asm.mov_ri(S0, 0);
    g.asm.movq_xr(FS0, S0);
    g.asm.ucomisd_rr(b, FS0); // compare rhs, 0.0
    // An ORDERED equality (rhs == 0.0, incl -0.0) sets ZF=1, PF=0. A NaN divisor
    // is unordered: ZF=1, PF=1 — must NOT deopt (NaN/x is a valid IEEE result).
    // So skip the deopt when PF=1 (unordered), then `je` on the ordered equal.
    let not_zero = g.asm.new_label();
    g.asm.jcc(Cond::ParityEven, not_zero); // unordered (NaN) -> not a zero divisor
    g.asm.jcc(Cond::Eq, dl); // ordered equal to 0.0 -> side-exit
    g.asm.bind(not_zero);
    // Normal divide: FS0 = lhs; FS0 /= rhs.
    g.fload(FS0, lhs);
    let b2 = g.foperand(rhs, FS1);
    g.asm.divsd_rr(FS0, b2);
    g.fstore(dst, FS0);
}

#[derive(Clone, Copy)]
enum FCmp {
    Lt,
    Gt,
    Le,
    Ge,
}

/// `dst = (lhs CMP rhs) as i64` for an f64 ORDERING compare, exact IEEE: a NaN
/// operand makes the result FALSE (0). Implemented via `ucomisd` + an UNSIGNED
/// setcc — for `>`/`>=` directly (`a` then `b`), for `<`/`<=` by SWAPPING the
/// operands (so `a < b` ≡ `b > a`). The unordered (NaN) case sets ZF=CF=1, which
/// the `seta`/`setae` family reads as FALSE, matching the reference.
fn emit_fcompare(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, cmp: FCmp) {
    // Decide operand order + the unsigned condition.
    let (a, b, cond) = match cmp {
        FCmp::Gt => (lhs, rhs, Cond::AU),  // a > b   : ucomisd a,b ; seta
        FCmp::Ge => (lhs, rhs, Cond::AeU), // a >= b  : ucomisd a,b ; setae
        FCmp::Lt => (rhs, lhs, Cond::AU),  // a < b ≡ b > a : ucomisd b,a ; seta
        FCmp::Le => (rhs, lhs, Cond::AeU), // a <= b ≡ b >= a : ucomisd b,a ; setae
    };
    let xa = g.foperand(a, FS0);
    let xb = g.foperand(b, FS1);
    g.asm.ucomisd_rr(xa, xb);
    g.asm.setcc_movzx(cond, S0);
    g.store(dst, S0);
}

/// `dst = (|lhs - rhs| < f64::EPSILON) as i64` (when `neg` is false) or its
/// negation (`neg` true) — the kernel's epsilon equality. Computed exactly like
/// the reference: `d = lhs - rhs`; `|d|` by clearing the sign bit (a GP `and`
/// with `0x7FFF…`); compare `|d|` against EPSILON via `ucomisd EPS, |d|` then
/// `setb` (EPS > |d| ⟺ |d| < EPS). NaN `|d|` is unordered → `setb` false → 0
/// (not equal), matching `(NaN).abs() < EPSILON == false`.
fn emit_feq(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, neg: bool) {
    // FS0 = lhs - rhs.
    g.fload(FS0, lhs);
    let b = g.foperand(rhs, FS1);
    g.asm.subsd_rr(FS0, b);
    // |d|: move bits to GP, clear sign bit, move back to FS0.
    g.asm.movq_rx(S0, FS0);
    g.asm.mov_ri(S1, i64::MAX); // 0x7FFF_FFFF_FFFF_FFFF
    g.asm.and_rr(S0, S1);
    g.asm.movq_xr(FS0, S0); // FS0 = |d|
    // FS1 = EPSILON.
    g.asm.mov_ri(S1, f64::EPSILON.to_bits() as i64);
    g.asm.movq_xr(FS1, S1);
    // ucomisd EPS, |d| ; setb gives EPS < |d|? No: CF=1 means EPS below |d|.
    // We want |d| < EPS ⟺ EPS > |d| ⟺ (after ucomisd EPS,|d|) seta (CF=0&&ZF=0).
    g.asm.ucomisd_rr(FS1, FS0); // compare EPS, |d|
    let cond = if neg { Cond::BeU } else { Cond::AU }; // eq: EPS>|d| (seta); neq: !(..) (setbe)
    g.asm.setcc_movzx(cond, S0);
    g.store(dst, S0);
}

/// Fused f64 compare-and-branch: transfer to `target` when `cmp(lhs, rhs)` is
/// FALSE — which, under IEEE, includes every NaN-unordered comparison (the
/// reference's `BranchF`). Ordering compares use `ucomisd` + the negated
/// unsigned jcc; epsilon (Eq/NotEq) reuses the `emit_feq` value then tests it.
fn emit_branchf(g: &mut Gen, cmp: Cmp, lhs: Slot, rhs: Slot, target: LabelId) {
    match cmp {
        Cmp::Lt | Cmp::Gt | Cmp::LtEq | Cmp::GtEq => {
            // For each, pick (a, b) and the "FALSE" unsigned jcc. The branch is
            // taken when the comparison is FALSE; NaN (unordered: CF=ZF=1) must
            // TAKE the false branch.
            //   a > b  true = seta(CF=0&&ZF=0) ; false = jbe(CF=1||ZF=1)
            //   a >= b true = setae(CF=0)       ; false = jb (CF=1)
            let (a, b, false_cond) = match cmp {
                Cmp::Gt => (lhs, rhs, Cond::BeU),  // !(a>b)
                Cmp::GtEq => (lhs, rhs, Cond::BU), // !(a>=b)
                Cmp::Lt => (rhs, lhs, Cond::BeU),  // a<b ≡ b>a ; !(b>a)
                Cmp::LtEq => (rhs, lhs, Cond::BU), // a<=b ≡ b>=a ; !(b>=a)
                _ => unreachable!(),
            };
            let xa = g.foperand(a, FS0);
            let xb = g.foperand(b, FS1);
            g.asm.ucomisd_rr(xa, xb);
            g.asm.jcc(false_cond, target);
        }
        Cmp::Eq | Cmp::NotEq => {
            // Compute the epsilon (in)equality into S0, then branch when FALSE.
            // For Eq: false = (eq == 0). For NotEq: false = (neq == 0).
            // Reuse emit_feq by writing the boolean into a dead path: inline it.
            g.fload(FS0, lhs);
            let b = g.foperand(rhs, FS1);
            g.asm.subsd_rr(FS0, b);
            g.asm.movq_rx(S0, FS0);
            g.asm.mov_ri(S1, i64::MAX);
            g.asm.and_rr(S0, S1);
            g.asm.movq_xr(FS0, S0); // |d|
            g.asm.mov_ri(S1, f64::EPSILON.to_bits() as i64);
            g.asm.movq_xr(FS1, S1);
            g.asm.ucomisd_rr(FS1, FS0); // EPS vs |d|
            // truth(Eq) = |d| < EPS = seta. truth(NotEq) = !that = setbe.
            // Branch on truth FALSE.
            let truth_cond = if matches!(cmp, Cmp::Eq) { Cond::AU } else { Cond::BeU };
            g.asm.setcc_movzx(truth_cond, S0);
            g.asm.test_rr(S0, S0);
            g.asm.jcc(Cond::Eq, target); // truth == 0 → jump (false branch)
        }
    }
}

#[derive(Clone, Copy)]
enum ShiftKind {
    Left,
    Right,
}

/// `dst = lhs <</>> rhs` with the kernel's shift spec: the count is the low
/// bits of `rhs` (x86 masks the count mod 64 for 64-bit operands, matching
/// `wrapping_shl`/`wrapping_shr`'s `as u32 % 64`). The value goes in S0, the
/// count in `cl` (rcx).
fn emit_shift(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, kind: ShiftKind) {
    g.load(S0, lhs);
    g.load(SC, rhs);
    match kind {
        ShiftKind::Left => g.asm.shl_cl(S0),
        ShiftKind::Right => g.asm.sar_cl(S0),
    }
    g.store(dst, S0);
}

/// `dst = lhs / 2^k` (signed, round toward zero) via the sign-correcting shift,
/// bit-exact with `reference_eval`'s `DivPow2`:
/// `(x + ((x >> 63) & ((1<<k)-1))) >> k`.
fn emit_div_pow2(g: &mut Gen, dst: Slot, lhs: Slot, k: u32) {
    g.load(S0, lhs); // x
    // S1 = x >> 63 (sign mask, arithmetic).
    g.asm.mov_rr(S1, S0);
    g.asm.mov_ri(SC, 63);
    g.asm.sar_cl(S1);
    // S1 &= (1<<k) - 1
    let mask = (1i64 << k) - 1;
    g.asm.mov_ri(SC, mask);
    g.asm.and_rr(S1, SC);
    // S0 = x + S1
    g.asm.add_rr(S0, S1);
    // S0 >>= k (arithmetic)
    g.asm.mov_ri(SC, k as i64);
    g.asm.sar_cl(S0);
    g.store(dst, S0);
}

/// `dst = lhs / c` (`mul_back == 0`) or `dst = lhs % c` (`mul_back == c`) via the
/// Granlund–Montgomery / libdivide UNSIGNED magic reciprocal — bit-exact with
/// `reference_eval`'s `MagicDivU` and the VM's `magic_eval`. Emitted only for a
/// proven non-negative dividend, so the unsigned arithmetic equals the signed
/// truncating result.
///
/// Register choreography: RAX/RDX/RCX (S0/S1/SC) are reserved scratch — no slot
/// is ever resident there, so `mul`'s implicit RDX:RAX clobber is safe. The
/// dividend's home slot (`lhs`) is never written, so it is RELOADED whenever the
/// add-fixup or the remainder needs `x` again.
///
/// Quotient: `q_hi = mulhi_u(magic, x)`; with the add-marker path
/// `q = ((x - q_hi) >> 1 + q_hi) >> shift`, else `q = q_hi >> shift`. The
/// quotient lands in S0 (RAX). Remainder: `r = x - q*c` (S0 ← x - q*c).
fn emit_magic_div(g: &mut Gen, dst: Slot, lhs: Slot, magic: u64, more: u8, mul_back: i64) {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;
    let shift = (more & SHIFT_MASK) as u8;

    if more & SHIFT_PATH != 0 {
        // Pure power-of-two path (never emitted by the compiler for the non-pow2
        // gate, but executed identically here for completeness): q = x >> shift,
        // LOGICAL — sound because x is non-negative.
        g.load(S0, lhs);
        if shift != 0 {
            g.asm.shr_ri(S0, shift);
        }
        finish_magic(g, dst, lhs, mul_back);
        return;
    }

    // q_hi = high 64 bits of the UNSIGNED product magic * x.
    g.asm.mov_ri(SC, magic as i64); // RCX = magic
    g.load(S0, lhs); // RAX = x
    g.asm.mul_r(SC); // RDX:RAX = x * magic; RDX = q_hi

    if more & ADD_MARKER != 0 {
        // q = ((x - q_hi) >> 1).wrapping_add(q_hi) >> shift.
        g.load(SC, lhs); // RCX = x (reload; lhs home is untouched)
        g.asm.sub_rr(SC, S1); // RCX = x - q_hi
        g.asm.shr_ri(SC, 1); // RCX = (x - q_hi) >> 1  (logical)
        g.asm.add_rr(SC, S1); // RCX = t
        if shift != 0 {
            g.asm.shr_ri(SC, shift); // RCX = q
        }
        g.asm.mov_rr(S0, SC); // RAX = q
    } else {
        // q = q_hi >> shift.
        if shift != 0 {
            g.asm.shr_ri(S1, shift); // RDX = q
        }
        g.asm.mov_rr(S0, S1); // RAX = q
    }
    finish_magic(g, dst, lhs, mul_back);
}

/// Common tail: the quotient is in S0 (RAX). For division store it; for modulo
/// compute `r = x - q*c` (`mul_back == c`) and store that.
fn finish_magic(g: &mut Gen, dst: Slot, lhs: Slot, mul_back: i64) {
    if mul_back == 0 {
        g.store(dst, S0); // quotient
    } else {
        // r = x - q*c.  RAX holds q.
        g.asm.mov_ri(SC, mul_back); // RCX = c
        g.asm.imul_rr(S0, SC); // RAX = q * c (low 64 bits suffice)
        g.load(SC, lhs); // RCX = x (reload)
        g.asm.sub_rr(SC, S0); // RCX = x - q*c
        g.store(dst, SC); // remainder
    }
}

/// `dst = lhs / rhs` (quotient) or `lhs % rhs` (remainder), with a zero-divisor
/// SIDE-EXIT before any effect and the kernel's `MIN op -1` overflow handling
/// (no `#DE` trap): if `rhs == -1` then quotient = `0 - lhs` (== MIN for MIN)
/// and remainder = 0.
fn emit_div_mod(g: &mut Gen, dst: Slot, lhs: Slot, rhs: Slot, is_div: bool, dl: LabelId) {
    // Load divisor into S1 (rdx is overwritten by cqo, so keep divisor in a
    // register that survives; use SC=rcx for the divisor instead).
    g.load(SC, rhs); // divisor in rcx
    // Zero check: side-exit if rcx == 0.
    g.asm.test_rr(SC, SC);
    g.asm.jcc(Cond::Eq, dl);
    // Overflow guard: if divisor == -1, avoid #DE. quotient = -lhs, rem = 0.
    let neg_one = g.asm.new_label();
    let done = g.asm.new_label();
    g.asm.mov_ri(S1, -1);
    g.asm.cmp_rr(SC, S1); // compare divisor with -1
    g.asm.jcc(Cond::Eq, neg_one);
    // Normal path: rax = lhs; cqo; idiv rcx.
    g.load(S0, lhs);
    g.asm.cqo();
    g.asm.idiv_r(SC); // quotient -> rax, remainder -> rdx
    if is_div {
        // result already in rax (S0)
    } else {
        g.asm.mov_rr(S0, S1); // remainder rdx -> rax
    }
    g.asm.jmp(done);
    // divisor == -1 path.
    g.asm.bind(neg_one);
    if is_div {
        // quotient = -lhs  (== lhs * -1; wrapping_div MIN/-1 = MIN = -MIN wrap)
        g.load(S0, lhs);
        g.asm.neg_r(S0);
    } else {
        // remainder = 0
        g.asm.mov_ri(S0, 0);
    }
    g.asm.bind(done);
    g.store(dst, S0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jit::{reference_eval, ChainOutcome};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn run(ops: &[MicroOp], frame: &[i64]) -> (ChainOutcome, Vec<i64>) {
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(ops, Some(status)).expect("supported region compiles");
        let mut f = frame.to_vec();
        let out = chain.run_with_frame(&mut f);
        (out, f)
    }

    #[test]
    fn straightline_arith_matches_reference() {
        let ops = vec![
            MicroOp::Add { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::Mul { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::Sub { dst: 5, lhs: 4, rhs: 0 },
            MicroOp::Return { src: 5 },
        ];
        let frame = vec![7i64, 11, 13, 0, 0, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected));
    }

    #[test]
    fn counting_loop_matches_reference() {
        // sum 0..N via a branch-back loop.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 2, target: 5 }, // while i<N
            MicroOp::Add { dst: 1, lhs: 1, rhs: 0 },                     // acc += i
            MicroOp::LoadConst { dst: 3, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 1 },
        ];
        let frame = vec![0i64, 0, 1000, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected));
        assert_eq!(out, ChainOutcome::Return(499_500));
    }

    #[test]
    fn div_by_zero_side_exits() {
        let status = Arc::new(AtomicI64::new(0));
        let ops = vec![
            MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        let chain = compile_region_regalloc(&ops, Some(status)).unwrap();
        let mut frame = vec![100i64, 0, 0];
        let out = chain.run_with_frame(&mut frame);
        assert!(out.is_deopt(), "div by zero must side-exit, got {out:?}");
    }

    #[test]
    fn div_min_neg_one_no_trap() {
        let ops = vec![
            MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        let frame = vec![i64::MIN, -1, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected)); // MIN / -1 == MIN
        assert_eq!(out, ChainOutcome::Return(i64::MIN));
    }

    #[test]
    fn mod_min_neg_one_zero() {
        let ops = vec![
            MicroOp::Mod { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Return { src: 2 },
        ];
        let frame = vec![i64::MIN, -1, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected)); // MIN % -1 == 0
        assert_eq!(out, ChainOutcome::Return(0));
    }

    #[test]
    fn unsupported_op_returns_none() {
        // A `MapGet` (hash-map probe via a runtime helper) is outside the
        // supported subset, so the region declines and the caller falls back to
        // the per-piece stencil tier. (Byte / `Seq of Bool` array loads/stores
        // ARE now supported — see `byte_array_*` — so they no longer decline.)
        let ops = vec![
            MicroOp::MapGet { dst: 1, key: 0, map_slot: 2, helper_addr: 0 },
            MicroOp::Return { src: 1 },
        ];
        assert!(compile_region_regalloc(&ops, None).is_none());
    }

    // =================================================================
    // WAVE 25: the INLINED `ArrPush` fast path. Each push lowers to an
    // INLINE `len < cap ? buffer[len++] = v` test that calls the runtime
    // helper ONLY on the (cold) realloc boundary. These forge-level tests
    // drive a REAL `Vec` through the compiled region with an instrumented
    // helper, proving (1) bit-identical contents to `Vec::push` across the
    // fast path AND the realloc cold path, and (2) that the per-iteration
    // call vanished on the fast path (the helper's call counter).
    // =================================================================

    /// Call counter for the instrumented test push helper: a fast-path push
    /// (`len < cap`) must NOT touch the helper, so this counts only the realloc
    /// cold-path entries.
    static TEST_PUSH_CALLS: AtomicI64 = AtomicI64::new(0);

    /// A drop-in stand-in for `logos_rt_push_i64` with the SAME SysV ABI and the
    /// SAME `Vec::push` + ptr/len-refresh semantics, plus a call counter. The
    /// inline fast path must reproduce its effect EXACTLY when `len < cap`.
    ///
    /// # Safety
    /// `frame` slots must hold a live `*mut Vec<i64>` (vec_slot) and the mirrored
    /// ptr/len cells, exactly as the region pin convention requires.
    unsafe extern "C" fn test_push_i64(
        frame: *mut i64,
        vec_slot: i64,
        ptr_slot: i64,
        len_slot: i64,
        value: i64,
    ) {
        TEST_PUSH_CALLS.fetch_add(1, Ordering::Relaxed);
        let vec = *frame.add(vec_slot as usize) as *mut Vec<i64>;
        (*vec).push(value);
        *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
        *frame.add(len_slot as usize) = (*vec).len() as i64;
    }

    /// Build a region that pushes `i` for `i` in `[0, n)` into a pinned Int list,
    /// then returns the final length. Frame layout:
    ///   0 = i (induction), 1 = n (bound), 2 = const 1,
    ///   8 = vec handle, 9 = buffer ptr, 10 = length.
    fn push_loop_ops(helper_addr: i64) -> Vec<MicroOp> {
        vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 5 }, // while i < n
            MicroOp::ArrPush {
                src: 0,
                vec_slot: 8,
                ptr_slot: 9,
                len_slot: 10,
                helper_addr,
                byte: false,
            },
            MicroOp::LoadConst { dst: 2, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 10 }, // final length
        ]
    }

    /// RED: pushes that ALL fit the pre-allocated capacity touch the runtime
    /// helper ZERO times — every push took the inline fast path — and the buffer
    /// holds exactly `Vec::push`'s result (same contents, same final pointer,
    /// since no realloc moved it).
    #[test]
    fn inline_push_fast_path_never_calls_helper_within_capacity() {
        let n = 5000i64;
        let mut vec: Vec<i64> = Vec::with_capacity(n as usize);
        let vec_ptr = &mut vec as *mut Vec<i64>;
        let buf_before = vec.as_ptr() as i64;

        let ops = push_loop_ops(test_push_i64 as usize as i64);
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("push region compiles");

        // frame[8] = vec handle, [9] = buffer ptr, [10] = length (all 0 initially).
        let mut frame = vec![0i64; 11];
        frame[1] = n;
        frame[8] = vec_ptr as i64;
        frame[9] = buf_before;
        frame[10] = 0;

        TEST_PUSH_CALLS.store(0, Ordering::Relaxed);
        let out = chain.run_with_frame(&mut frame);

        assert_eq!(out, ChainOutcome::Return(n), "final length must be n");
        assert_eq!(
            TEST_PUSH_CALLS.load(Ordering::Relaxed),
            0,
            "every push fit within capacity — the inline fast path must call the helper ZERO times"
        );
        // Bit-identical to Vec::push: contents and (no-realloc) pointer.
        assert_eq!(vec.len(), n as usize, "real Vec length");
        assert_eq!(vec.as_ptr() as i64, buf_before, "no realloc moved the buffer");
        for (i, &v) in vec.iter().enumerate() {
            assert_eq!(v, i as i64, "buffer[{i}] must be the pushed value");
        }
        // The frame's mirrored len/ptr stay coherent with the real Vec.
        assert_eq!(frame[10], n, "mirrored length");
        assert_eq!(frame[9], buf_before, "mirrored pointer unchanged (no realloc)");
    }

    /// RED: pushes that GROW past the initial capacity stay bit-identical to
    /// `Vec::push` — the cold helper path reallocates and refreshes ptr/len, and
    /// the inline path bumps len in place between reallocs. The helper is called
    /// only on the realloc boundaries (far fewer than `n` times), proving the
    /// per-iteration call is gone.
    #[test]
    fn inline_push_realloc_cold_path_matches_vec_push() {
        let n = 4000i64;
        // Start with a TINY capacity so the cold realloc path fires repeatedly.
        let mut vec: Vec<i64> = Vec::with_capacity(1);
        let vec_ptr = &mut vec as *mut Vec<i64>;

        // The reference: a plain Vec::push of the same sequence from cap 1.
        let mut reference: Vec<i64> = Vec::with_capacity(1);
        for i in 0..n {
            reference.push(i);
        }

        let ops = push_loop_ops(test_push_i64 as usize as i64);
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("push region compiles");

        let mut frame = vec![0i64; 11];
        frame[1] = n;
        frame[8] = vec_ptr as i64;
        frame[9] = vec.as_ptr() as i64;
        frame[10] = 0;

        TEST_PUSH_CALLS.store(0, Ordering::Relaxed);
        let out = chain.run_with_frame(&mut frame);

        assert_eq!(out, ChainOutcome::Return(n), "final length must be n");
        assert_eq!(vec, reference, "compiled push must be bit-identical to Vec::push");
        assert_eq!(frame[10], n, "mirrored length matches");
        assert_eq!(
            frame[9],
            vec.as_ptr() as i64,
            "mirrored pointer matches the (post-realloc) buffer"
        );
        let calls = TEST_PUSH_CALLS.load(Ordering::Relaxed);
        // Geometric growth from cap 1: O(log n) reallocs, vastly fewer than n.
        assert!(
            calls < n / 2,
            "the helper must fire only on realloc boundaries (got {calls} of {n} pushes)"
        );
        assert!(calls >= 1, "at least one realloc must have occurred");
    }

    /// RED: a `Seq of Bool` (1-byte element) push stores the BOOLEAN
    /// NORMALIZATION `(v != 0) as u8` inline — bit-identical to
    /// `logos_rt_push_bool`'s `(*vec).push(value != 0)`.
    #[test]
    fn inline_push_byte_normalizes_like_helper() {
        unsafe extern "C" fn test_push_bool(
            frame: *mut i64,
            vec_slot: i64,
            ptr_slot: i64,
            len_slot: i64,
            value: i64,
        ) {
            let vec = *frame.add(vec_slot as usize) as *mut Vec<bool>;
            (*vec).push(value != 0);
            *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
            *frame.add(len_slot as usize) = (*vec).len() as i64;
        }

        let n = 300i64;
        let mut vec: Vec<bool> = Vec::with_capacity(n as usize);
        let vec_ptr = &mut vec as *mut Vec<bool>;
        let buf_before = vec.as_ptr() as i64;

        // Push `i % 3` (0, 1, 2, 0, 1, 2, …): zero stores false, nonzero true.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 7 }, // while i < n
            MicroOp::LoadConst { dst: 3, value: 3 },
            MicroOp::Mod { dst: 4, lhs: 0, rhs: 3 }, // v = i % 3
            MicroOp::ArrPush {
                src: 4,
                vec_slot: 8,
                ptr_slot: 9,
                len_slot: 10,
                helper_addr: test_push_bool as usize as i64,
                byte: true,
            },
            MicroOp::LoadConst { dst: 2, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 10 },
        ];
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("byte push region compiles");

        let mut frame = vec![0i64; 11];
        frame[1] = n;
        frame[8] = vec_ptr as i64;
        frame[9] = buf_before;
        frame[10] = 0;
        let out = chain.run_with_frame(&mut frame);

        assert_eq!(out, ChainOutcome::Return(n));
        let reference: Vec<bool> = (0..n).map(|i| (i % 3) != 0).collect();
        assert_eq!(vec, reference, "byte push must normalize (v != 0) like the helper");
        assert_eq!(vec.as_ptr() as i64, buf_before, "no realloc within capacity");
    }

    /// RED: an XMM-resident (float-list) push bit-copies the value to the buffer
    /// inline — the raw 8 bytes travel identically to the helper's
    /// `f64::from_bits` round-trip.
    #[test]
    fn inline_push_float_value_bitcopies() {
        unsafe extern "C" fn test_push_f64(
            frame: *mut i64,
            vec_slot: i64,
            ptr_slot: i64,
            len_slot: i64,
            value: i64,
        ) {
            let vec = *frame.add(vec_slot as usize) as *mut Vec<f64>;
            (*vec).push(f64::from_bits(value as u64));
            *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
            *frame.add(len_slot as usize) = (*vec).len() as i64;
        }

        let n = 200i64;
        let mut vec: Vec<f64> = Vec::with_capacity(n as usize);
        let vec_ptr = &mut vec as *mut Vec<f64>;
        let buf_before = vec.as_ptr() as i64;

        // src is a FLOAT slot produced by IntToFloat (lands in XMM): push i as f64.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 6 }, // while i < n
            MicroOp::IntToFloat { dst: 4, src: 0 }, // v = i as f64 (XMM-resident)
            MicroOp::ArrPush {
                src: 4,
                vec_slot: 8,
                ptr_slot: 9,
                len_slot: 10,
                helper_addr: test_push_f64 as usize as i64,
                byte: false,
            },
            MicroOp::LoadConst { dst: 2, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 2 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 10 },
        ];
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc(&ops, Some(status)).expect("float push region compiles");

        let mut frame = vec![0i64; 11];
        frame[1] = n;
        frame[8] = vec_ptr as i64;
        frame[9] = buf_before;
        frame[10] = 0;
        let out = chain.run_with_frame(&mut frame);

        assert_eq!(out, ChainOutcome::Return(n));
        let reference: Vec<f64> = (0..n).map(|i| i as f64).collect();
        assert_eq!(vec, reference, "float push must bit-copy the value like the helper");
        assert_eq!(vec.as_ptr() as i64, buf_before, "no realloc within capacity");
    }

    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    /// Exhaustive differential: every random straight-line program over the
    /// supported families must equal the (location-independent) reference, at
    /// EVERY slot count — small (all resident) and large (forces spills).
    #[test]
    fn random_straightline_matches_reference_all_pressures() {
        for slots in [3u16, 6, 10, 20] {
            for seed in 1..=80u64 {
                let mut s = seed.wrapping_mul(2654435761);
                let len = 8 + (lcg(&mut s) % 24) as usize;
                let mut ops = Vec::with_capacity(len + 1);
                for _ in 0..len {
                    let dst = (lcg(&mut s) % slots as u64) as u16;
                    let lhs = (lcg(&mut s) % slots as u64) as u16;
                    let rhs = (lcg(&mut s) % slots as u64) as u16;
                    let op = match lcg(&mut s) % 13 {
                        0 => MicroOp::Add { dst, lhs, rhs },
                        1 => MicroOp::Sub { dst, lhs, rhs },
                        2 => MicroOp::Mul { dst, lhs, rhs },
                        3 => MicroOp::BitAnd { dst, lhs, rhs },
                        4 => MicroOp::BitOr { dst, lhs, rhs },
                        5 => MicroOp::BitXor { dst, lhs, rhs },
                        6 => MicroOp::Lt { dst, lhs, rhs },
                        7 => MicroOp::Gt { dst, lhs, rhs },
                        8 => MicroOp::LtEq { dst, lhs, rhs },
                        9 => MicroOp::Eq { dst, lhs, rhs },
                        10 => MicroOp::Neq { dst, lhs, rhs },
                        11 => MicroOp::Move { dst, src: lhs },
                        _ => MicroOp::LoadConst {
                            dst,
                            value: (lcg(&mut s) as i64).wrapping_sub(1 << 40),
                        },
                    };
                    ops.push(op);
                }
                let ret = (lcg(&mut s) % slots as u64) as u16;
                ops.push(MicroOp::Return { src: ret });

                let mut frame = vec![0i64; slots as usize];
                for (i, f) in frame.iter_mut().enumerate() {
                    *f = (i as i64 + 1).wrapping_mul(1_000_003) - 7;
                }
                let expected = reference_eval(&ops, &mut frame.clone(), 100_000)
                    .expect("straightline terminates");
                let (out, post) = run(&ops, &frame);
                assert_eq!(
                    out,
                    ChainOutcome::Return(expected),
                    "slots={slots} seed={seed}: return diverged"
                );
                // The full frame must also match the reference's full frame —
                // every resident-written slot was flushed on exit.
                let mut ref_frame = frame.clone();
                reference_eval(&ops, &mut ref_frame, 100_000).unwrap();
                assert_eq!(post, ref_frame, "slots={slots} seed={seed}: frame diverged");
            }
        }
    }

    /// Shifts and unary ops bit-exact against the reference, at spill pressure.
    #[test]
    fn shifts_and_unary_match_reference() {
        let ops = vec![
            MicroOp::Shl { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::Shr { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::NotInt { dst: 5, src: 4 },
            MicroOp::NotBool { dst: 6, src: 2 },
            MicroOp::DivPow2 { dst: 7, lhs: 0, k: 3 },
            MicroOp::Add { dst: 8, lhs: 5, rhs: 7 },
            MicroOp::Return { src: 8 },
        ];
        for &a in &[1i64, -1, 1024, i64::MIN, -255] {
            let frame = vec![a, 5, 2, 0, 0, 0, 0, 0, 0];
            let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
            let (out, _) = run(&ops, &frame);
            assert_eq!(out, ChainOutcome::Return(expected), "a={a}");
        }
    }

    /// The Granlund–Montgomery magic constants for an unsigned divisor `d`
    /// (test-local; mirrors the canonical `LogosDivU64::new` the compiler ships).
    fn magic_parts(d: u64) -> (u64, u8) {
        if d & (d - 1) == 0 {
            return (0, (d.trailing_zeros() as u8) | 0x80);
        }
        let l = 63 - d.leading_zeros();
        let numer = (1u128 << l) << 64;
        let m = (numer / d as u128) as u64;
        let rem = (numer % d as u128) as u64;
        if d - rem < (1u64 << l) {
            (m + 1, l as u8)
        } else {
            let twice = rem.wrapping_mul(2);
            let bump = (twice >= d || twice < rem) as u64;
            (m.wrapping_mul(2).wrapping_add(bump) + 1, (l as u8) | 0x40)
        }
    }

    /// `MagicDivU` (the W24 constant-divisor magic reciprocal) is bit-exact with
    /// `reference_eval` — which itself is bit-exact with the kernel's
    /// `wrapping_div`/`wrapping_rem` for the non-negative dividend it gates on —
    /// for both the quotient and the remainder, across several divisors (the
    /// 64-bit-magic and 65-bit add-marker paths) and a boundary value grid,
    /// emitted into a frame with enough live slots to force spill pressure on
    /// the dst/lhs.
    #[test]
    fn magic_div_matches_reference_div_and_mod() {
        for &c in &[3i64, 7, 100, 1000, 1_000_000_007, 65521] {
            let (magic, more) = magic_parts(c as u64);
            // Two ops (div into 1, mod into 2) plus padding slots 3..16 so the
            // allocator must spill — exercises both register- and frame-resident
            // dst/lhs in emit_magic_div.
            let ops = vec![
                MicroOp::MagicDivU { dst: 1, lhs: 0, magic, more, mul_back: 0 },
                MicroOp::MagicDivU { dst: 2, lhs: 0, magic, more, mul_back: c },
                MicroOp::Add { dst: 3, lhs: 1, rhs: 2 },
                MicroOp::Return { src: 3 },
            ];
            for n in [
                0i64, 1, c - 1, c, c + 1, 2 * c + 3, 123_456_789, i64::MAX, i64::MAX - 1,
                (i64::MAX / c) * c, (i64::MAX / c).saturating_sub(1) * c,
            ] {
                assert!(n >= 0);
                let frame = vec![n, 0, 0, 0];
                let expected = reference_eval(&ops, &mut frame.clone(), 1000).unwrap();
                let (out, _) = run(&ops, &frame);
                assert_eq!(out, ChainOutcome::Return(expected), "c={c} n={n}");
                // And the reference itself equals the kernel for these.
                assert_eq!(
                    expected,
                    n.wrapping_div(c).wrapping_add(n.wrapping_rem(c)),
                    "reference vs kernel c={c} n={n}"
                );
            }
        }
    }

    /// A nested loop with many live slots (forces some spills) — the polynomial
    /// accumulation the ceiling harness times — matches the reference exactly.
    #[test]
    fn poly_loop_matches_reference_under_spill() {
        // s = s*A + i*B - C across i in 0..N, with A,B,C,N,i,s in distinct slots.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 11 }, // i<N
            MicroOp::Mul { dst: 7, lhs: 2, rhs: 3 },                       // t = s*A
            MicroOp::Mul { dst: 8, lhs: 0, rhs: 4 },                       // u = i*B
            MicroOp::Add { dst: 7, lhs: 7, rhs: 8 },                       // t += u
            MicroOp::Sub { dst: 2, lhs: 7, rhs: 5 },                       // s = t - C
            MicroOp::BitXor { dst: 2, lhs: 2, rhs: 6 },                    // s ^= MASK
            MicroOp::Add { dst: 9, lhs: 2, rhs: 0 },                       // (extra live)
            MicroOp::Mul { dst: 2, lhs: 2, rhs: 3 },                       // s *= A again
            MicroOp::LoadConst { dst: 10, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 10 }, // i += 1
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 2 },
        ];
        // slots: 0=i 1=N 2=s 3=A 4=B 5=C 6=MASK 7=t 8=u 9=extra 10=one
        let frame = vec![0i64, 5000, 1, 6364136223846793005u64 as i64, 1442695040888963407u64 as i64, 12345, 0x5DEECE66D, 0, 0, 0, 0];
        let expected = reference_eval(&ops, &mut frame.clone(), 100_000_000).unwrap();
        let (out, _) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected));
    }

    /// `loop_depths` (the spill-weight heuristic's loop detector): a back-edge
    /// `[target, idx]` raises the depth of every op in its span by one, and
    /// nesting accumulates. Straight-line code stays depth 0.
    #[test]
    fn loop_depths_counts_back_edge_nesting() {
        // No back-edge: all depth 0.
        let flat = vec![
            MicroOp::Add { dst: 1, lhs: 0, rhs: 0 },
            MicroOp::Return { src: 1 },
        ];
        assert_eq!(loop_depths(&flat), vec![0, 0]);

        // One loop: a Jump at idx 3 back to 1 wraps ops 1..=3 (depth 1); the
        // guard at 0 and the tail at 4 stay depth 0.
        let single = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 4 }, // 0
            MicroOp::Add { dst: 2, lhs: 2, rhs: 0 },                     // 1
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },                     // 2
            MicroOp::Jump { target: 1 },                                 // 3
            MicroOp::Return { src: 2 },                                  // 4
        ];
        assert_eq!(loop_depths(&single), vec![0, 1, 1, 1, 0]);

        // Nested: inner back-edge 3->2 (depth 2 over ops 2..=3) sits inside the
        // outer back-edge 5->1 (depth 1 over ops 1..=5).
        let nested = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 6 }, // 0
            MicroOp::Move { dst: 4, src: 0 },                            // 1 outer
            MicroOp::Add { dst: 2, lhs: 2, rhs: 4 },                     // 2 inner
            MicroOp::Jump { target: 2 },                                 // 3 inner back-edge
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },                     // 4 outer
            MicroOp::Jump { target: 1 },                                 // 5 outer back-edge
            MicroOp::Return { src: 2 },                                  // 6
        ];
        assert_eq!(loop_depths(&nested), vec![0, 1, 2, 2, 1, 1, 0]);
    }

    /// A loop-carried slot referenced ONCE per iteration but living inside a
    /// loop must out-rank a colder slot referenced MANY times only in
    /// straight-line setup, so it wins a register. We assert this through the
    /// public `loc` assignment indirectly: build a region where slot `H` (hot,
    /// loop-carried, few raw refs) and slot `C` (cold, setup-only, many raw refs)
    /// compete for the SOLE remaining int register, and confirm the loop weight
    /// flips the ranking so the result is still bit-identical to the reference
    /// (the heuristic only moves residency, never correctness).
    #[test]
    fn loop_weight_keeps_carried_slot_resident_and_exact() {
        // Setup touches `C` (slot 5) four times in straight-line code; the loop
        // touches `H` (slot 2, the accumulator) once per iteration. Raw counts
        // would rank C above H; the loop weight (10^1 per loop ref) flips it.
        let ops = vec![
            MicroOp::Add { dst: 5, lhs: 0, rhs: 1 },  // 0 C = a+b   (setup)
            MicroOp::Add { dst: 5, lhs: 5, rhs: 0 },  // 1 C += a    (setup)
            MicroOp::Add { dst: 5, lhs: 5, rhs: 1 },  // 2 C += b    (setup)
            MicroOp::Add { dst: 2, lhs: 2, rhs: 5 },  // 3 seed acc with C (setup)
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 3, rhs: 4, target: 8 }, // 4 while i<N
            MicroOp::Add { dst: 2, lhs: 2, rhs: 3 },  // 5 acc += i  (HOT, depth 1)
            MicroOp::Add { dst: 3, lhs: 3, rhs: 6 },  // 6 i += 1    (HOT, depth 1)
            MicroOp::Jump { target: 4 },              // 7 back-edge
            MicroOp::Return { src: 2 },               // 8
        ];
        // slots: 0=a 1=b 2=acc(H) 3=i 4=N 5=C(cold) 6=one
        let frame = vec![3i64, 4, 0, 0, 100_000, 0, 1];
        let depths = loop_depths(&ops);
        assert_eq!(depths[5], 1, "the accumulator update is inside the loop");
        let expected = reference_eval(&ops, &mut frame.clone(), 10_000_000).unwrap();
        let (out, post) = run(&ops, &frame);
        assert_eq!(out, ChainOutcome::Return(expected), "loop-weighted ranking must stay bit-identical");
        let mut ref_frame = frame.clone();
        reference_eval(&ops, &mut ref_frame, 10_000_000).unwrap();
        assert_eq!(post, ref_frame, "frame diverged under loop-weighted ranking");
    }

    /// Fix 1 (call-weight tie-break): two slots with the SAME primary loop-weight
    /// must be ordered by call-survival — the slot LIVE ACROSS a SysV call ranks
    /// first, so it wins a callee-saved register (paying no per-call spill/reload)
    /// while the call-dead slot of equal weight takes a caller-saved one. The
    /// call-weight is a SECONDARY key: it breaks the tie WITHOUT changing the
    /// primary weights (so it can never evict a hotter loop slot).
    #[test]
    fn call_weight_breaks_tie_toward_live_across_call_slot() {
        // `survivor` (slot 1) and `transient` (slot 2) are each defined once and
        // read once — identical primary loop-weight. `survivor` is read AFTER the
        // call (op 3) → live across it; `transient` is consumed BEFORE the call
        // and never read after → dead across it. Only the secondary key differs.
        // 0: survivor = a + one    (def slot 1)
        // 1: transient = a + one   (def slot 2)
        // 2: sink = transient + a  (last read of transient, def slot 8 — BEFORE call)
        // 3: ListClear             (the SysV call, op 3)
        // 4: r = survivor + one    (reads slot 1 AFTER the call, def slot 4)
        // 5: Return r
        let ops = vec![
            MicroOp::Add { dst: 1, lhs: 0, rhs: 3 },
            MicroOp::Add { dst: 2, lhs: 0, rhs: 3 },
            MicroOp::Add { dst: 8, lhs: 2, rhs: 0 },
            MicroOp::ListClear { vec_slot: 5, ptr_slot: 6, len_slot: 7, helper_addr: 0 },
            MicroOp::Add { dst: 4, lhs: 1, rhs: 3 },
            MicroOp::Return { src: 4 },
        ];
        let max_slot = max_slot_of(&ops);
        let la = liveness_after(&ops, max_slot);
        assert!(la[3][1], "survivor must be live across the call");
        assert!(!la[3][2], "transient must be dead across the call");
        // PRIMARY weights must be equal (both: 1 def-ref + 1 use-ref outside any
        // loop). Slot 1: def@0 + use@4 = 2 refs. Slot 2: def@1 + use@2 = 2 refs.
        let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
        let pos = |slot: Slot| order.iter().position(|&(s, _)| s == slot).unwrap();
        let key = |slot: Slot| order.iter().find(|&&(s, _)| s == slot).unwrap().1;
        assert_eq!(
            key(1),
            key(2),
            "survivor and transient must share the SAME primary weight (got {} vs {})",
            key(1),
            key(2)
        );
        assert!(
            pos(1) < pos(2),
            "on equal primary weight the call-survivor must rank ahead of the call-dead slot"
        );
    }

    /// Fix 1 soundness: the call-survival key NEVER changes the primary
    /// loop-weighted ordering of two slots with DIFFERENT primary weights, so it
    /// cannot evict a hotter loop slot. A loop-hot, call-DEAD slot must still
    /// out-rank a cold, call-SURVIVING slot.
    #[test]
    fn call_weight_never_outranks_a_hotter_loop_slot() {
        // `hot` (slot 2) is referenced every iteration of a loop (high primary
        // weight) but is consumed before the call → dead across it. `cold` (slot
        // 1) is set once outside the loop and read once after the call → live
        // across the call but cold. The loop-hot slot must still win.
        let ops = vec![
            MicroOp::Add { dst: 1, lhs: 0, rhs: 3 },                      // 0 cold = a+one (setup)
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 4, rhs: 5, target: 6 },  // 1 while i<N
            MicroOp::Add { dst: 2, lhs: 2, rhs: 4 },                      // 2 hot += i (HOT)
            MicroOp::Add { dst: 4, lhs: 4, rhs: 3 },                      // 3 i += one (HOT)
            MicroOp::ListClear { vec_slot: 7, ptr_slot: 8, len_slot: 9, helper_addr: 0 }, // 4 call
            MicroOp::Jump { target: 1 },                                  // 5 back-edge
            MicroOp::Add { dst: 10, lhs: 1, rhs: 2 },                     // 6 use cold (live across) + hot
        ];
        // Append a Return so the region is well-formed.
        let mut ops = ops;
        ops.push(MicroOp::Return { src: 10 });
        let max_slot = max_slot_of(&ops);
        let la = liveness_after(&ops, max_slot);
        let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
        let pos = |slot: Slot| order.iter().position(|&(s, _)| s == slot).unwrap();
        assert!(
            pos(2) < pos(1),
            "the loop-hot slot must out-rank the cold call-survivor (primary key dominates)"
        );
    }

    /// Fix 1 soundness: the call bonus is INERT for a call-free region. With
    /// `live_after = None` the ranking is exactly the pre-Fix-1 loop-weighted
    /// ranking — no bonus exists because there is no call to survive.
    #[test]
    fn call_weight_is_inert_without_a_call() {
        let ops = vec![
            MicroOp::Add { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::Mul { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::Sub { dst: 5, lhs: 4, rhs: 0 },
            MicroOp::Return { src: 5 },
        ];
        // With no liveness (call-free) and with an EMPTY all-dead liveness, the
        // bonus contributes nothing, so the two rankings are identical.
        let (no_call, _) = rank_slots(&ops, 16, 4, 4, None);
        let max_slot = max_slot_of(&ops);
        let dead = vec![vec![false; max_slot + 1]; ops.len()];
        let (dead_la, _) = rank_slots(&ops, 16, 4, 4, Some(&dead));
        assert_eq!(no_call, dead_la, "the call bonus must vanish with no live-across-call slot");
    }

    /// Fix 1 (call-weight) must not perturb the SELF-CALL window forcing or the
    /// `max_slot` extent: a region with a `CallSelf` still compiles, and the
    /// ranking ranks the call-surviving non-window slot ahead of a transient. The
    /// runnable end-to-end (bit-identical + tiers) lives in the integration suite
    /// (`jit_regalloc::call_weight_*`) where a real recursive program is built.
    #[test]
    fn call_weight_self_call_region_compiles_and_ranks_survivor() {
        // A function-shaped region: guard, a self-call, an accumulate that reads a
        // value live across the call, return. (Window slots are forced frame-
        // resident regardless; this only checks the ranking + that it compiles.)
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 5 }, // 0 if !(i<N) -> ret
            MicroOp::CallSelf { dst: 3, args_start: 8, depth_addr: 0, status_addr: 0, limit_slot: 2, frame_size: 4 }, // 1 call
            MicroOp::Add { dst: 4, lhs: 4, rhs: 3 }, // 2 acc += call result (acc live across? no — defined here)
            MicroOp::Add { dst: 0, lhs: 0, rhs: 4 }, // 3 i += acc (reads acc 4 → live after the call)
            MicroOp::Jump { target: 0 },             // 4 back-edge
            MicroOp::Return { src: 4 },              // 5
        ];
        let max_slot = max_slot_of(&ops);
        assert!(max_slot >= 11, "self-call window extent (8+4-1=11) must be covered, got {max_slot}");
        let la = liveness_after(&ops, max_slot);
        // slot 4 (acc) is read by op 3 after the call at op 1 → live across it.
        assert!(la[1][4], "acc must be live across the self-call");
        let (order, _) = rank_slots(&ops, 16, 4, 4, Some(&la));
        let key = |slot: Slot| order.iter().find(|&&(s, _)| s == slot).map(|&(_, k)| k).unwrap_or(0);
        // slot 4 (call-survivor, loop-carried) must out-rank the bound slot 1
        // (referenced only at the cold guard, not across the call).
        assert!(key(4) > key(1), "call-surviving loop slot must out-rank a cold bound");
    }

    /// Wave 16 (call-site spill liveness): `liveness_after` is a sound backward
    /// dataflow. A slot WRITTEN before an op and READ only before it (never on any
    /// path AFTER) is DEAD after that op; a slot read by a LATER op (including a
    /// loop back-edge) is LIVE after every op on the path to that read.
    #[test]
    fn liveness_after_is_a_sound_backward_dataflow() {
        // 0: t = a + b      (defines t=2)
        // 1: dead = a * a   (defines dead=3, never read again)
        // 2: r = t - dead   (reads t and dead, defines r=4)
        // 3: Return r
        let ops = vec![
            MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::Mul { dst: 3, lhs: 0, rhs: 0 },
            MicroOp::Sub { dst: 4, lhs: 2, rhs: 3 },
            MicroOp::Return { src: 4 },
        ];
        let la = liveness_after(&ops, 4);
        // After op 1 (`dead = a*a`), `dead` (slot 3) is LIVE (read by op 2) and so
        // is `t` (slot 2, read by op 2).
        assert!(la[1][3], "dead must be live right after it is defined (read by op 2)");
        assert!(la[1][2], "t must still be live after op 1 (read by op 2)");
        // After op 2 (`r = t - dead`), NEITHER `t` nor `dead` is read again — both DEAD.
        assert!(!la[2][2], "t is dead after its last read (op 2)");
        assert!(!la[2][3], "dead is dead after its last read (op 2)");
        // `r` (slot 4) is live after op 2 (the Return reads it).
        assert!(la[2][4], "r is live after op 2 (Return reads it)");
        // After the Return, nothing is live.
        assert!(la[3].iter().all(|&b| !b), "no slot is live after Return");
    }

    /// A loop-carried slot read across a back-edge stays LIVE after every op in
    /// the loop body (the backward dataflow must reach a fixpoint over the
    /// back-edge, not just the forward pass).
    #[test]
    fn liveness_after_propagates_across_loop_back_edge() {
        // counting loop: 0=i 1=acc 2=N 3=one
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 2, target: 5 }, // 0 while i<N
            MicroOp::Add { dst: 1, lhs: 1, rhs: 0 },                     // 1 acc += i
            MicroOp::LoadConst { dst: 3, value: 1 },                     // 2 one = 1
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },                     // 3 i += 1
            MicroOp::Jump { target: 0 },                                 // 4 back-edge
            MicroOp::Return { src: 1 },                                  // 5
        ];
        let la = liveness_after(&ops, 3);
        // `i` (slot 0) is read by the guard (op 0), `acc += i` (op 1), and `i += 1`
        // (op 3); via the back-edge it must be live after op 3 (the increment),
        // after op 1, and after the guard's true edge — i.e. live around the loop.
        assert!(la[3][0], "i must be live after the increment (back-edge re-reads it)");
        assert!(la[1][0], "i must be live after acc += i (used again at op 3)");
        // `acc` (slot 1) is read by op 1 and the final Return → live around the loop.
        assert!(la[3][1], "acc must be live after op 3 (read next iteration / return)");
        assert!(la[1][1], "acc must be live after its own update");
    }

    /// The spill-elision keep-rule: a caller-saved resident is elided around a
    /// call ONLY when it is READ-ONLY (never written → never flushed at exit) AND
    /// dead after the call. A WRITTEN slot is kept even when dead-after-call
    /// (its last value is flushed at every region exit), and a read-only slot
    /// read after the call is kept (the reload feeds that read). This mirrors the
    /// `keep` closure in `compile_impl` and is the soundness contract the
    /// `*_matches_stencil` post-frame differentials enforce.
    #[test]
    fn spill_elision_keeps_written_and_live_drops_readonly_dead() {
        // Slot 5 = `cold`: a frame input READ only at op 0 (never written, never
        //   read after the call) → read-only + dead-after-call → ELIDED.
        // Slot 8 = `hot`: WRITTEN at op 1, dead after the call (op 3 re-reads it,
        //   so it is actually live-after too — but the WRITTEN rule alone keeps
        //   it; flushed at every exit). Slot 1 = the literal-`1` operand: read
        //   only (never written) but READ after the call (op 3) → kept via live.
        // 0: t0 = cold + 1   (reads slot 5 `cold`, defines slot 7)
        // 1: hot = hot + 1   (WRITTEN slot 8)
        // 2: ListClear       (the call op, index 2)
        // 3: keep = hot + 1  (reads slot 8 `hot` and slot 1 `1`, defines slot 9)
        // 4: Return keep
        let ops = vec![
            MicroOp::Add { dst: 7, lhs: 5, rhs: 1 },          // 0 t0 = cold + 1
            MicroOp::Add { dst: 8, lhs: 8, rhs: 1 },          // 1 hot += 1 (written)
            MicroOp::ListClear { vec_slot: 2, ptr_slot: 3, len_slot: 4, helper_addr: 0 }, // 2 call
            MicroOp::Add { dst: 9, lhs: 8, rhs: 1 },          // 3 keep = hot + 1
            MicroOp::Return { src: 9 },                       // 4
        ];
        let max_slot = 9;
        let la = liveness_after(&ops, max_slot);
        let mut written = vec![false; max_slot + 1];
        for op in &ops {
            if let Some(d) = dest_of(op) {
                written[d as usize] = true;
            }
        }
        let keep = |s: Slot| {
            written.get(s as usize).copied().unwrap_or(true)
                || la[2].get(s as usize).copied().unwrap_or(true)
        };
        // `cold` (slot 5): read-only (only ever an operand) and NOT read after the
        // call (op 3 reads `hot`/`1`, not `cold`) → ELIDED.
        assert!(!written[5], "cold is read-only");
        assert!(!la[2][5], "cold is dead after the call");
        assert!(!keep(5), "a read-only, dead-after-call resident must be elided");
        // `hot` (slot 8): WRITTEN → kept regardless (flushed at every exit). It is
        // also live-after here, but the WRITTEN rule alone must keep it.
        assert!(written[8], "hot is written");
        assert!(keep(8), "a written resident must be kept (flushed at exit)");
        // `frame[1]` (the literal `1` operand): read-only but read AFTER the call
        // (op 3 reads it) → kept via the live-after rule.
        assert!(!written[1], "the constant operand is read-only");
        assert!(la[2][1], "the constant operand is read after the call");
        assert!(keep(1), "a read-only resident read after the call must be kept");
    }

    // =================================================================
    // WAVE 21: precise REGION backend (in-place SetIndex + reallocating
    // ArrPush). The fannkuch / graph_bfs worklist shape — a precise region
    // that does NOT have the mode-B function prologue, so `mode_b_rc` is
    // inferred only for FUNCTIONS, never regions.
    // =================================================================

    /// The precise REGION path compiles a push+SetIndex stream (a reallocating
    /// `ArrPush` beside an in-place checked `ArrStore`) into ONE contiguous
    /// register-allocated chain — even though the stream has NO mode-B function
    /// prologue (`LoadConst {-1}`). The per-piece precise stencil tier no longer
    /// monopolizes this shape.
    #[test]
    fn precise_region_with_push_and_setindex_compiles() {
        // 0: i < n?  -> exit (op 6)
        // 1: ArrStore arr[i] = v  (in-place, CHECKED — the precise side exit)
        // 2: ArrPush v -> q       (reallocates q; helper refreshes ptr/len)
        // 3: i += 1
        // 4: Jump head
        // 5: (exit) Return i
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 5 },
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::ArrPush { src: 2, vec_slot: 8, ptr_slot: 9, len_slot: 10, helper_addr: 0, byte: false },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 3 },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 0 },
        ];
        // Per-op precise resume codes (parallel to `ops`): the checked store and
        // the push carry a precise tag `(pc << 2) | 3`; the rest stay plain `1`.
        let codes: Vec<i64> = ops
            .iter()
            .enumerate()
            .map(|(i, op)| match op {
                MicroOp::ArrStore { checked: true, .. } | MicroOp::ArrPush { .. } => {
                    ((i as i64) << 2) | 3
                }
                _ => 1,
            })
            .collect();
        let status = Arc::new(AtomicI64::new(0));
        let chain = compile_region_regalloc_precise(&ops, Some(status), 0, &codes);
        assert!(
            chain.is_some(),
            "a precise push+SetIndex REGION (no mode-B prologue) must compile through \
             the contiguous regalloc backend"
        );
    }

    /// The precise REGION path DECLINES when the per-op `deopt_codes` length does
    /// not match the op stream — a malformed call falls back to the per-piece tier
    /// rather than mis-indexing the resume table.
    #[test]
    fn precise_region_declines_on_codes_length_mismatch() {
        let ops = vec![
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        let codes = vec![1i64]; // one short
        let status = Arc::new(AtomicI64::new(0));
        assert!(
            compile_region_regalloc_precise(&ops, Some(status), 0, &codes).is_none(),
            "a deopt-codes length mismatch must decline"
        );
    }

    /// The precise REGION path requires the shared status channel (every checked
    /// op's side exit stores its tagged resume value through it); a `None` status
    /// declines.
    #[test]
    fn precise_region_declines_without_status() {
        let ops = vec![
            MicroOp::ArrStore { src: 2, idx: 0, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        let codes = vec![3i64, 1];
        assert!(
            compile_region_regalloc_precise(&ops, None, 0, &codes).is_none(),
            "a precise region with no status channel must decline"
        );
    }

    /// A precise region carrying a CROSS-FUNCTION `Call` is unsupported by the
    /// region gate (`supported`) and must decline — the JIT adapter already
    /// disqualifies push-beside-call regions, and the backend is the second line
    /// of defense (a call frame would need the full function-precise walk).
    #[test]
    fn precise_region_declines_on_cross_call() {
        let ops = vec![
            MicroOp::Call {
                dst: 1,
                args_start: 4,
                table_addr: 0,
                depth_addr: 0,
                status_addr: 0,
                limit_slot: 3,
                depth_limit: SELF_CALL_DEPTH_LIMIT,
            },
            MicroOp::Return { src: 1 },
        ];
        let codes = vec![3i64, 1];
        let status = Arc::new(AtomicI64::new(0));
        assert!(
            compile_region_regalloc_precise(&ops, Some(status), 0, &codes).is_none(),
            "a precise REGION with a cross-function Call must decline (region gate)"
        );
    }
}
