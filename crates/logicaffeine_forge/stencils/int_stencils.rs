//! Copy-and-patch stencils, written in Rust and compiled to an object file at
//! build time (see build.rs).
//!
//! # The hole technique
//!
//! Continuations and constants are `extern` symbols with NO definition in this
//! codegen unit, so rustc/LLVM cannot inline or constant-fold them — every
//! reference MUST materialize as a relocation in the object file. Those
//! relocations are the "holes" the JIT patches at runtime:
//!
//! - `logos_hole_cont_N(base, sp)` — a continuation hole. The uniform CPS
//!   signature (`unsafe extern "C" fn(*mut i64, *mut i64) -> i64`, identical
//!   at call site and callee, `panic=abort`, `opt-level=2`, one codegen unit)
//!   makes LLVM emit it as a TAIL CALL (`b` on arm64, `jmp` on x86-64);
//!   build.rs asserts this per stencil so a rustc regression fails the build.
//! - `LOGOS_HOLE_I64_N` — a constant hole, compiled as a PC-relative load
//!   (direct or GOT-indirect). The JIT redirects the load to a literal-pool
//!   slot appended after the stencil code (Mach-O arm64 has no MOVW/MOVK
//!   relocations, so patch-the-immediate is unrepresentable there; the
//!   literal-pool redirect works on every format).
//!
//! # The machine model
//!
//! Two pointers thread through every stencil, pinned in argument registers by
//! the uniform signature:
//!
//! - `base` — the FRAME: a fixed array of i64 slots holding the bytecode
//!   VM's registers for the compiled function. `slot_get`/`slot_set` index it
//!   with a patched constant.
//! - `sp` — one past the top of a grow-up operand stack used for expression
//!   evaluation. Binary ops pop two and push one; `logos_stencil_return` pops
//!   the result and actually returns, terminating every chain.

#![allow(internal_features)]
#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern "C" {
    fn logos_hole_cont_0(
        base: *mut i64,
        sp: *mut i64,
        r0: i64,
        r1: i64,
        r2: i64,
        r3: i64,
        f0: f64,
        f1: f64,
        f2: f64,
        f3: f64,
        f4: f64,
        f5: f64,
    ) -> i64;
    fn logos_hole_cont_1(
        base: *mut i64,
        sp: *mut i64,
        r0: i64,
        r1: i64,
        r2: i64,
        r3: i64,
        f0: f64,
        f1: f64,
        f2: f64,
        f3: f64,
        f4: f64,
        f5: f64,
    ) -> i64;
    static LOGOS_HOLE_I64_0: i64;
    static LOGOS_HOLE_I64_1: i64;
    static LOGOS_HOLE_I64_2: i64;
    static LOGOS_HOLE_I64_3: i64;
    static LOGOS_HOLE_I64_4: i64;
    static LOGOS_HOLE_I64_5: i64;
    static LOGOS_HOLE_I64_6: i64;
    static LOGOS_HOLE_I64_7: i64;
    static LOGOS_HOLE_I64_8: i64;
}

/// The kernel's locked maximum LOGOS call depth, baked into the precise and
/// self call stencils (whose holes are all spoken for, leaving no room for a
/// runtime depth-limit hole). It MUST equal `logicaffeine_compile`'s
/// `semantics::MAX_CALL_DEPTH` — every engine enforces the same cap, so the
/// native tier raising the depth error at a different point than the kernel
/// would diverge. `forge` is a `#![no_std]`, dependency-free codegen unit, so
/// the value is mirrored here; the JIT crate asserts the agreement at
/// stencil-selection time (`logicaffeine_jit::lib::compile_function` /
/// `compile_region` pass `MAX_CALL_DEPTH`, checked against this in `jit.rs`).
const MAX_CALL_DEPTH: i64 = 2_500;

/// The relocation-free leaf proving raw extraction works (the Tier-0 headline:
/// `jit_add(3, 5) == 8`). Kept alongside the CPS set.
#[no_mangle]
pub extern "C" fn logos_stencil_add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

/// Push the patched constant, continue.
///
/// # Safety
/// `sp` must point into an operand stack with at least one free slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_const(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *sp = LOGOS_HOLE_I64_0;
    logos_hole_cont_0(base, sp.add(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Push frame slot [K], continue. K is the patched constant.
///
/// # Safety
/// `base` must point at a frame with more than K slots; `sp` must have a free
/// slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_slot_get(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *sp = *base.add(LOGOS_HOLE_I64_0 as usize);
    logos_hole_cont_0(base, sp.add(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop into frame slot [K], continue. K is the patched constant.
///
/// # Safety
/// `base` must point at a frame with more than K slots; `sp` must point one
/// past at least one live operand slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_slot_set(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *base.add(LOGOS_HOLE_I64_0 as usize) = *sp.sub(1);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two, push their wrapping sum, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_addi(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_add(b);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two, push their wrapping difference, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_subi(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_sub(b);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two, push their wrapping product, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_muli(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_mul(b);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two, push `(a < b) as i64`, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_lti(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = (a < b) as i64;
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two, push `(a == b) as i64`, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_eqi(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = (a == b) as i64;
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

// ════════════════════════════════════════════════════════════════════════
// The 3-ADDRESS stencil family (M3): one piece per micro-op, frame slots in
// and out, no operand stack. Holes: 0 = first operand slot (or source /
// constant), 1 = second operand slot (or destination), 2 = destination.
// ════════════════════════════════════════════════════════════════════════

/// `frame[D] = frame[S]` — hole 0 = S, hole 1 = D.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_movss(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *base.add(LOGOS_HOLE_I64_1 as usize) = *base.add(LOGOS_HOLE_I64_0 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = V` — hole 0 = V (the immediate), hole 1 = D.
///
/// # Safety
/// `base` must point at a frame larger than the patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_constst(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *base.add(LOGOS_HOLE_I64_1 as usize) = LOGOS_HOLE_I64_0;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] + frame[R]` (wrapping) — holes 0/1/2 = L/R/D.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_add3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    // EXACT: signed overflow side-exits (cont_1) so the VM recomputes and promotes.
    match a.checked_add(b) {
        Some(v) => {
            *base.add(LOGOS_HOLE_I64_2 as usize) = v;
            logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
        }
        None => logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5),
    }
}

/// `frame[D] = frame[L] - frame[R]`, EXACT (overflow side-exits via cont_1).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_sub3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    match a.checked_sub(b) {
        Some(v) => {
            *base.add(LOGOS_HOLE_I64_2 as usize) = v;
            logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
        }
        None => logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5),
    }
}

/// `frame[D] = frame[L] * frame[R]`, EXACT (overflow side-exits via cont_1).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_mul3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    match a.checked_mul(b) {
        Some(v) => {
            *base.add(LOGOS_HOLE_I64_2 as usize) = v;
            logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
        }
        None => logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5),
    }
}

/// `frame[D] = (frame[L] < frame[R]) as i64`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_lt3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a < b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = (frame[L] <= frame[R]) as i64`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_le3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a <= b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = (frame[L] == frame[R]) as i64`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_eq3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a == b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = (frame[L] != frame[R]) as i64`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_ne3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a != b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] / frame[R]` (wrapping); zero divisor exits to
/// continuation 1 before any effect.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_div3c(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    if b == 0 {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a.wrapping_div(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] % frame[R]` (wrapping); zero divisor exits to
/// continuation 1 before any effect.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_mod3c(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    if b == 0 {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a.wrapping_rem(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] / 2^k` (signed, round toward zero) via the
/// sign-correcting shift idiom `(x + ((x>>63) & (2^k-1))) >> k` — bit-exact
/// with `wrapping_div` for ALL signs, replacing a hardware `idiv`. NO deopt:
/// a power-of-two divisor is never `0` and never `-1`, so neither the
/// div-by-zero nor the `i64::MIN/-1` overflow exits of `div3c` can fire.
/// Holes: 0 = L (dividend slot), 1 = k (shift, immediate), 2 = 2^k-1 (mask,
/// immediate), 3 = D (dst slot). The compiler emits this ONLY when the divisor
/// slot is a region-constant power of two and the dividend is integer-typed.
///
/// # Safety
/// `base` must point at a frame larger than every patched slot index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_divpow2(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let x = *base.add(LOGOS_HOLE_I64_0 as usize);
    let k = LOGOS_HOLE_I64_1;
    let mask = LOGOS_HOLE_I64_2;
    let bias = (x >> 63) & mask;
    *base.add(LOGOS_HOLE_I64_3 as usize) = x.wrapping_add(bias) >> k;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] / c` (`mul_back == 0`) or `frame[L] % c`
/// (`mul_back == c`) via the Granlund–Montgomery / libdivide UNSIGNED magic
/// reciprocal — a `mul`-high + shift, replacing a hardware `idiv`. NO deopt: a
/// literal `c > 0` is never `0` and never `-1`. Bit-exact with the kernel's
/// `wrapping_div`/`wrapping_rem` for the proven non-negative dividend the
/// compiler gates on (the unsigned and signed truncating results agree there).
/// Holes: 0 = L (dividend slot), 1 = magic (immediate), 2 = more (immediate,
/// the `LogosDivU64` encoding: low 6 bits = shift, `0x40` = the 65-bit
/// add-marker path, `0x80` = pure-shift pow2), 3 = mul_back (immediate, `0` for
/// the quotient or `c` for the remainder), 4 = D (dst slot).
///
/// # Safety
/// `base` must point at a frame larger than every patched slot index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_magicdiv(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let x = *base.add(LOGOS_HOLE_I64_0 as usize);
    let magic = LOGOS_HOLE_I64_1 as u64;
    let more = LOGOS_HOLE_I64_2 as u8;
    let mul_back = LOGOS_HOLE_I64_3;
    let shift = more & 0x3f;
    let n = x as u64;
    let q = if more & 0x80 != 0 {
        n >> shift
    } else {
        let hi = (((magic as u128) * (n as u128)) >> 64) as u64;
        if more & 0x40 != 0 {
            let t = (n.wrapping_sub(hi) >> 1).wrapping_add(hi);
            t >> shift
        } else {
            hi >> shift
        }
    };
    let out = if mul_back == 0 {
        q as i64
    } else {
        x.wrapping_sub((q as i64).wrapping_mul(mul_back))
    };
    *base.add(LOGOS_HOLE_I64_4 as usize) = out;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Fused compare-and-branch: `frame[L] < frame[R]` → continuation 0, else 1.
/// No comparison value is materialized — the caller proved the scratch dead.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_brlt(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    if a < b {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Fused compare-and-branch: `frame[L] == frame[R]` → continuation 0, else 1.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_breq(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    if a == b {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Value branch: `frame[C] != 0` → continuation 0, else 1 (the JumpIfFalse
/// shape when the condition is a stored Bool rather than a fresh comparison).
///
/// # Safety
/// `base` must point at a frame larger than the patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_brz(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    if *base.add(LOGOS_HOLE_I64_0 as usize) != 0 {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Terminal: return `frame[S]`.
///
/// # Safety
/// `base` must point at a frame larger than the patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_ret2(base: *mut i64, _sp: *mut i64) -> i64 {
    *base.add(LOGOS_HOLE_I64_0 as usize)
}

// ════════════════════════════════════════════════════════════════════════
// Int bitwise / shift family. Shift semantics transcribe the kernel's
// locked wrapping spec: the count truncates to u32 then masks mod 64;
// Shr is ARITHMETIC (sign-extending).
// ════════════════════════════════════════════════════════════════════════

/// `frame[D] = frame[L] & frame[R]` — bitwise for Int×Int; for the VM's 0/1
/// Bool representation this is also exactly logical AND.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_and3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a & b;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] | frame[R]` (see and3 for the Bool note).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_or3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a | b;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L] ^ frame[R]`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_xor3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a ^ b;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L].wrapping_shl(frame[R] as u32)`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_shl3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a.wrapping_shl(b as u32);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[L].wrapping_shr(frame[R] as u32)` (arithmetic).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_shr3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    *base.add(LOGOS_HOLE_I64_2 as usize) = a.wrapping_shr(b as u32);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = !frame[S]` — bitwise NOT (the kernel's Int `not`).
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_noti2(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *base.add(LOGOS_HOLE_I64_1 as usize) = !*base.add(LOGOS_HOLE_I64_0 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = frame[S] ^ 1` — logical NOT over the 0/1 Bool representation.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_notb2(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    *base.add(LOGOS_HOLE_I64_1 as usize) = *base.add(LOGOS_HOLE_I64_0 as usize) ^ 1;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

// ════════════════════════════════════════════════════════════════════════
// Float family: f64 values travel as raw bits in the i64 frame slots.
// Relational comparisons are IEEE (NaN unordered → false); equality is the
// kernel's EPSILON rule `(a - b).abs() < f64::EPSILON` (NaN-safe: false).
// ════════════════════════════════════════════════════════════════════════

/// `frame[D] = bits(f(L) + f(R))`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_addf3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a + b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = bits(sqrt(f(S)))` — IEEE-754 sqrt (negative input → NaN),
/// which is the kernel's Float sqrt builtin exactly. `f64::sqrt` lives in
/// std (libm), unavailable under `#![no_std]`, so the single hardware
/// instruction is written directly — the piece stays a call-free leaf.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_sqrtf2(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let v = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let r: f64;
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!("sqrtsd {r}, {v}", r = out(xmm_reg) r, v = in(xmm_reg) v);
    #[cfg(target_arch = "aarch64")]
    core::arch::asm!("fsqrt {r:d}, {v:d}", r = out(vreg) r, v = in(vreg) v);
    *base.add(LOGOS_HOLE_I64_1 as usize) = r.to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = bits(f(L) - f(R))`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_subf3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a - b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = bits(f(L) * f(R))`.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_mulf3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a * b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = bits(f(L) / f(R))`; a divisor `== 0.0` (covering -0.0, per
/// IEEE equality — the kernel's exact check) exits to continuation 1 before
/// any effect.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_divf3c(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    if b == 0.0 {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a / b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = (f(L) < f(R)) as i64` (IEEE: NaN → 0).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_ltf3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a < b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = (f(L) <= f(R)) as i64` (IEEE: NaN → 0).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_lef3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_2 as usize) = (a <= b) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = ((f(L) - f(R)).abs() < f64::EPSILON) as i64` — the kernel's
/// epsilon equality (NaN-safe: false).
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_eqf3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    // |a-b| < EPSILON via the bit ordering of non-negative IEEE floats
    // (identical result for every input incl. NaN/inf/denormals) — integer
    // immediates only, so LLVM emits no constant-pool reference and the
    // leaf-purity build gate holds.
    let d = ((a - b).to_bits() & 0x7FFF_FFFF_FFFF_FFFF) < f64::EPSILON.to_bits();
    *base.add(LOGOS_HOLE_I64_2 as usize) = d as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = !epsilon_eq as i64` — the kernel's float not-equals.
///
/// # Safety
/// `base` must point at a frame larger than every patched index.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_nef3(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    let d = ((a - b).to_bits() & 0x7FFF_FFFF_FFFF_FFFF) < f64::EPSILON.to_bits();
    *base.add(LOGOS_HOLE_I64_2 as usize) = !d as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Fused float compare-and-branch: `f(L) < f(R)` → continuation 0, else 1
/// (NaN → 1).
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_brltf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    if a < b {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Fused float compare-and-branch: `f(L) <= f(R)` → continuation 0, else 1.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_brlef(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    if a <= b {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Fused float compare-and-branch on the kernel's EPSILON equality.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_breqf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);
    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);
    if ((a - b).to_bits() & 0x7FFF_FFFF_FFFF_FFFF) < f64::EPSILON.to_bits() {
        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Pinned-array load: `frame[D] = buffer[frame[I] - 1]` (1-based; element
/// width 8 regardless of kind — f64 rides as bits). Holes: 0 = I (index
/// slot), 1 = pointer slot, 2 = length slot, 3 = D. An out-of-bounds index
/// (incl. 0 and negatives, via the wrapping-sub trick — exactly the
/// kernel's `i == 0 || i > len`) exits to continuation 1 BEFORE any effect.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller for
/// the duration of the run.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(im1 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pinned-array load WITHOUT a bounds check — the Oracle proved the index
/// in `[1, length]` (V8/LLVM bounds-check elimination, M9 range analysis).
/// Holes: 0 = I (1-based index slot), 1 = pointer slot, 3 = dst slot. There
/// is no length hole and no out-of-bounds continuation; the compiler emits
/// this ONLY behind `index_provably_in_bounds`.
///
/// # Safety
/// As `logos_stencil_arrld`, AND the index must be a proven-in-bounds value
/// — emitted solely from `Op::IndexUnchecked`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(i.wrapping_sub(1) as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Byte-array unchecked load (the `ST_ARRLDB` twin without bounds check).
///
/// # Safety
/// As `logos_stencil_arrld_u`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldb_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const u8;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(i.wrapping_sub(1) as usize) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load float ADD: `frame[D] = bits(f(buf[I-1]) + f(buf[J-1]))`,
/// both elements loaded from the SAME pinned buffer (one pointer slot, one
/// length slot). Holes: 0 = I (1-based index), 1 = J (1-based index),
/// 2 = pointer slot, 3 = length slot, 4 = D. EITHER index out of bounds
/// (incl. 0/negatives, via the wrapping-sub trick) exits to continuation 1
/// BEFORE any effect — the same deopt continuation as `arrld`. This collapses
/// the `ArrLoad; ArrLoad; AddF` triple so the two loaded f64s stay in
/// registers instead of round-tripping through the frame.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller for
/// the duration of the run.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_addf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let len = *base.add(LOGOS_HOLE_I64_3 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) || (jm1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let a = f64::from_bits(*ptr.add(im1 as usize) as u64);
    let b = f64::from_bits(*ptr.add(jm1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_4 as usize) = (a + b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load float SUB: `frame[D] = bits(f(buf[I-1]) - f(buf[J-1]))`.
/// See [`logos_stencil_arrld2_addf`] for the hole layout and bounds contract.
///
/// # Safety
/// See [`logos_stencil_arrld2_addf`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_subf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let len = *base.add(LOGOS_HOLE_I64_3 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) || (jm1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let a = f64::from_bits(*ptr.add(im1 as usize) as u64);
    let b = f64::from_bits(*ptr.add(jm1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_4 as usize) = (a - b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load float MUL: `frame[D] = bits(f(buf[I-1]) * f(buf[J-1]))`.
/// See [`logos_stencil_arrld2_addf`] for the hole layout and bounds contract.
///
/// # Safety
/// See [`logos_stencil_arrld2_addf`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_mulf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let len = *base.add(LOGOS_HOLE_I64_3 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) || (jm1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let a = f64::from_bits(*ptr.add(im1 as usize) as u64);
    let b = f64::from_bits(*ptr.add(jm1 as usize) as u64);
    *base.add(LOGOS_HOLE_I64_4 as usize) = (a * b).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER ADD across TWO buffers:
/// `frame[D] = a[I-1] + b[J-1]` (wrapping i64). Holes: 0 = I (1-based index
/// into `a`), 1 = J (1-based index into `b`), 2 = a-pointer slot, 3 = a-length
/// slot, 4 = b-pointer slot, 5 = b-length slot, 6 = D. EITHER index out of
/// bounds (incl. 0/negatives, via the wrapping-sub trick) exits to continuation
/// 1 BEFORE any effect — the same deopt continuation as `arrld`. This collapses
/// the `ArrLoad; ArrLoad; Add` triple so the two loaded i64s stay in registers
/// instead of round-tripping the frame (the matmul / dot-product idiom).
///
/// # Safety
/// Both pointer/length slots must hold live buffers pinned by the caller for
/// the duration of the run.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_add(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let lena = *base.add(LOGOS_HOLE_I64_3 as usize);
    let lenb = *base.add(LOGOS_HOLE_I64_5 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (lena as u64) || (jm1 as u64) >= (lenb as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_4 as usize) as *const i64;
    let a = *pa.add(im1 as usize);
    let b = *pb.add(jm1 as usize);
    *base.add(LOGOS_HOLE_I64_6 as usize) = a.wrapping_add(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER SUB across TWO buffers: `frame[D] = a[I-1] - b[J-1]`.
/// See [`logos_stencil_arrld2_add`] for the hole layout and bounds contract.
///
/// # Safety
/// See [`logos_stencil_arrld2_add`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_sub(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let lena = *base.add(LOGOS_HOLE_I64_3 as usize);
    let lenb = *base.add(LOGOS_HOLE_I64_5 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (lena as u64) || (jm1 as u64) >= (lenb as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_4 as usize) as *const i64;
    let a = *pa.add(im1 as usize);
    let b = *pb.add(jm1 as usize);
    *base.add(LOGOS_HOLE_I64_6 as usize) = a.wrapping_sub(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER MUL across TWO buffers: `frame[D] = a[I-1] * b[J-1]`.
/// See [`logos_stencil_arrld2_add`] for the hole layout and bounds contract.
///
/// # Safety
/// See [`logos_stencil_arrld2_add`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_mul(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let lena = *base.add(LOGOS_HOLE_I64_3 as usize);
    let lenb = *base.add(LOGOS_HOLE_I64_5 as usize);
    let im1 = i.wrapping_sub(1);
    let jm1 = j.wrapping_sub(1);
    if (im1 as u64) >= (lena as u64) || (jm1 as u64) >= (lenb as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_4 as usize) as *const i64;
    let a = *pa.add(im1 as usize);
    let b = *pb.add(jm1 as usize);
    *base.add(LOGOS_HOLE_I64_6 as usize) = a.wrapping_mul(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER ADD WITHOUT bounds checks — both indices proven in
/// range (V8/LLVM BCE). Holes: 0 = I, 1 = J, 2 = a-pointer slot, 3 = b-pointer
/// slot, 4 = D. No length holes and no out-of-bounds continuation; emitted
/// only behind `index_provably_in_bounds` for BOTH indices.
///
/// # Safety
/// As [`logos_stencil_arrld2_add`], AND both indices must be proven in bounds.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_add_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let a = *pa.add(i.wrapping_sub(1) as usize);
    let b = *pb.add(j.wrapping_sub(1) as usize);
    *base.add(LOGOS_HOLE_I64_4 as usize) = a.wrapping_add(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER SUB WITHOUT bounds checks.
/// See [`logos_stencil_arrld2_add_u`] for the hole layout.
///
/// # Safety
/// See [`logos_stencil_arrld2_add_u`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_sub_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let a = *pa.add(i.wrapping_sub(1) as usize);
    let b = *pb.add(j.wrapping_sub(1) as usize);
    *base.add(LOGOS_HOLE_I64_4 as usize) = a.wrapping_sub(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED two-load INTEGER MUL WITHOUT bounds checks.
/// See [`logos_stencil_arrld2_add_u`] for the hole layout.
///
/// # Safety
/// See [`logos_stencil_arrld2_add_u`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld2_mul_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let j = *base.add(LOGOS_HOLE_I64_1 as usize);
    let pa = *base.add(LOGOS_HOLE_I64_2 as usize) as *const i64;
    let pb = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let a = *pa.add(i.wrapping_sub(1) as usize);
    let b = *pb.add(j.wrapping_sub(1) as usize);
    *base.add(LOGOS_HOLE_I64_4 as usize) = a.wrapping_mul(b);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED affine-index load `frame[D] = buf[(frame[A] + C) - 1]` — the single-
/// slot-plus-constant index shape (`w + 1`). Holes: 0 = A (index operand slot),
/// 2 = C (the constant offset VALUE, not a slot), 3 = pointer slot, 4 = length
/// slot, 5 = D. The 1-based index is `frame[A].wrapping_add(C)`; an out-of-bounds
/// result (incl. 0/negatives, via the wrapping-sub trick) exits to continuation
/// 1 BEFORE any effect — the same deopt continuation as `arrld`.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_none(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let c = LOGOS_HOLE_I64_2;
    let len = *base.add(LOGOS_HOLE_I64_4 as usize);
    let idx = a.wrapping_add(c);
    let im1 = idx.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(im1 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// As `logos_stencil_arrldaff_none` WITHOUT a bounds check (Oracle-proven in
/// range). Holes: 0 = A, 2 = C, 3 = pointer slot, 5 = D. No length hole, no
/// out-of-bounds continuation.
///
/// # Safety
/// As `logos_stencil_arrldaff_none`, AND the computed index must be in range.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_none_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let c = LOGOS_HOLE_I64_2;
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(a.wrapping_add(c).wrapping_sub(1) as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED affine-index load `frame[D] = buf[((frame[A] + frame[B]) + C) - 1]` —
/// the two-slot-plus-constant index shape (`i*n + j + 1`, `i*n + j`). Holes:
/// 0 = A, 1 = B (the two index operand slots), 2 = C (constant offset VALUE),
/// 3 = pointer slot, 4 = length slot, 5 = D. The 1-based index wraps with the
/// kernel's i64 semantics; out of bounds exits to continuation 1 before effect.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_add(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let len = *base.add(LOGOS_HOLE_I64_4 as usize);
    let idx = a.wrapping_add(b).wrapping_add(c);
    let im1 = idx.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(im1 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// As `logos_stencil_arrldaff_add` WITHOUT a bounds check. Holes: 0 = A, 1 = B,
/// 2 = C, 3 = pointer slot, 5 = D.
///
/// # Safety
/// As `logos_stencil_arrldaff_add`, AND the computed index must be in range.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_add_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let idx = a.wrapping_add(b).wrapping_add(c);
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(idx.wrapping_sub(1) as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED affine-index load `frame[D] = buf[((frame[A] - frame[B]) + C) - 1]` —
/// the `w - wi + 1` index shape. Holes as `logos_stencil_arrldaff_add`.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_sub(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let len = *base.add(LOGOS_HOLE_I64_4 as usize);
    let idx = a.wrapping_sub(b).wrapping_add(c);
    let im1 = idx.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(im1 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// As `logos_stencil_arrldaff_sub` WITHOUT a bounds check.
///
/// # Safety
/// As `logos_stencil_arrldaff_sub`, AND the computed index must be in range.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_sub_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let idx = a.wrapping_sub(b).wrapping_add(c);
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(idx.wrapping_sub(1) as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// FUSED affine-index load `frame[D] = buf[((frame[A] * frame[B]) + C) - 1]`.
/// Holes as `logos_stencil_arrldaff_add`.
///
/// # Safety
/// The pointer/length slots must hold a live buffer pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_mul(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let len = *base.add(LOGOS_HOLE_I64_4 as usize);
    let idx = a.wrapping_mul(b).wrapping_add(c);
    let im1 = idx.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(im1 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// As `logos_stencil_arrldaff_mul` WITHOUT a bounds check.
///
/// # Safety
/// As `logos_stencil_arrldaff_mul`, AND the computed index must be in range.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldaff_mul_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let a = *base.add(LOGOS_HOLE_I64_0 as usize);
    let b = *base.add(LOGOS_HOLE_I64_1 as usize);
    let c = LOGOS_HOLE_I64_2;
    let ptr = *base.add(LOGOS_HOLE_I64_3 as usize) as *const i64;
    let idx = a.wrapping_mul(b).wrapping_add(c);
    *base.add(LOGOS_HOLE_I64_5 as usize) = *ptr.add(idx.wrapping_sub(1) as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pinned-array store: `buffer[frame[I] - 1] = frame[S]`. Holes: 0 = I,
/// 1 = pointer slot, 2 = length slot, 3 = S. Bounds exit like arrld.
///
/// # Safety
/// See [`logos_stencil_arrld`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrst(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;
    *ptr.add(im1 as usize) = *base.add(LOGOS_HOLE_I64_3 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pinned-array store WITHOUT a bounds check — the Oracle proved the index in
/// `[1, length]` (V8/LLVM bounds-check elimination). Holes: 0 = I, 1 = pointer
/// slot, 3 = S. No length hole, no out-of-bounds continuation; the compiler
/// emits this ONLY behind `index_provably_in_bounds` (from
/// `Op::SetIndexUnchecked`).
///
/// # Safety
/// As `logos_stencil_arrst`, AND the index must be a proven-in-bounds value.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrst_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;
    *ptr.add(i.wrapping_sub(1) as usize) = *base.add(LOGOS_HOLE_I64_3 as usize);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Byte-array unchecked store (the `ST_ARRSTB` twin without bounds check).
///
/// # Safety
/// As `logos_stencil_arrst_u`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrstb_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut u8;
    *ptr.add(i.wrapping_sub(1) as usize) = (*base.add(LOGOS_HOLE_I64_3 as usize) != 0) as u8;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Native SELF-CALL through the program's entry table. Holes:
/// 0 = table slot address ([entry, callee regcount] pair),
/// 1 = args_start (callee frame = base + args_start — VM windowing),
/// 2 = dst slot, 3 = depth cell address, 4 = status cell address,
/// 5 = frame slot holding the arena END address, 6 = MAX_CALL_DEPTH.
/// Deopt (no entry yet / depth crossing / arena overflow) stores 1 to the
/// status cell and UNWINDS — every caller up the native stack checks the
/// same cell after its call returns, so the whole tree exits and the
/// outermost `try_native` replays on bytecode.
///
/// The entry is read at RUNTIME through the table slot (an indirect call
/// through a register — no relocation, so the tail-call build gate only
/// sees the genuine continuation).
///
/// # Safety
/// The table/depth/status addresses must point at the program's live
/// `NativeCtx` cells; the arena-end slot must bound the caller's frame.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_call(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let table = LOGOS_HOLE_I64_0 as *const i64;
    let entry_raw = core::ptr::read_volatile(table);
    let depth = LOGOS_HOLE_I64_3 as *mut i64;
    let regc = core::ptr::read_volatile(table.add(1));
    let callee_base = base.add(LOGOS_HOLE_I64_1 as usize);
    let arena_end = *base.add(LOGOS_HOLE_I64_5 as usize);
    // The callee frame is regcount slots + 2 conversion scratches + its own
    // arena-limit slot (the shared frame layout convention).
    if entry_raw == 0
        || *depth >= LOGOS_HOLE_I64_6
        || (callee_base as i64).wrapping_add(regc.wrapping_add(3).wrapping_mul(8)) > arena_end
    {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 1);
        return 0;
    }
    // Plant the callee's own arena-limit slot before entering it.
    *callee_base.add(regc.wrapping_add(2) as usize) = arena_end;
    *depth += 1;
    let f: unsafe extern "C" fn(*mut i64, *mut i64, i64, i64, i64, i64, f64, f64, f64, f64) -> i64 =
        core::mem::transmute(entry_raw);
    // The callee's own entry reloads its pinned registers from its frame —
    // fresh zeros are correct here. This is a REAL call: SysV makes
    // rdx/rcx/r8/r9 caller-saved, so LLVM preserves OUR live r0..r3 around
    // it automatically.
    let r = f(callee_base, sp, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0.0);
    *depth -= 1;
    if core::ptr::read_volatile(LOGOS_HOLE_I64_4 as *const i64) != 0 {
        return 0;
    }
    *base.add(LOGOS_HOLE_I64_2 as usize) = r;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Helper-call push: `vec.push(frame[S])` through a runtime helper whose
/// ADDRESS is hole 4 (an indirect call — no relocation). The helper refreshes
/// the pinned pointer/length slots after a possible realloc. Holes:
/// 0 = S (value slot), 1 = vec-handle slot, 2 = pointer slot, 3 = length
/// slot, 4 = helper address.
///
/// # Safety
/// The vec-handle slot must hold a live `*mut Vec<…>` pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_push(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(*mut i64, i64, i64, i64, i64) =
        core::mem::transmute(LOGOS_HOLE_I64_4);
    helper(
        base,
        LOGOS_HOLE_I64_1,
        LOGOS_HOLE_I64_2,
        LOGOS_HOLE_I64_3,
        *base.add(LOGOS_HOLE_I64_0 as usize),
    );
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Helper-call in-place list clear: `vec.clear()` through a runtime helper whose
/// ADDRESS is hole 3 (indirect call — no relocation). The helper truncates the
/// pinned buffer to empty and refreshes the pinned pointer/length slots. Holes:
/// 0 = vec-handle slot, 1 = pointer slot, 2 = length slot, 3 = helper address.
///
/// # Safety
/// The vec-handle slot must hold a live `*mut Vec<…>` pinned by the caller.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_list_clear(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(*mut i64, i64, i64, i64) =
        core::mem::transmute(LOGOS_HOLE_I64_3);
    helper(base, LOGOS_HOLE_I64_0, LOGOS_HOLE_I64_1, LOGOS_HOLE_I64_2);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Byte-stride pinned-array load (Bool buffers): `frame[D] = buf[I-1] as i64`
/// over 1-byte elements. Holes/bounds exactly like arrld.
///
/// # Safety
/// See [`logos_stencil_arrld`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrldb(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const u8;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(im1 as usize) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Byte-stride pinned-array store (Bool buffers): `buf[I-1] = frame[S] != 0`.
///
/// # Safety
/// See [`logos_stencil_arrld`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrstb(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut u8;
    *ptr.add(im1 as usize) = (*base.add(LOGOS_HOLE_I64_3 as usize) != 0) as u8;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// 4-byte SIGN-EXTENDED pinned-array load (`ListRepr::IntsI32`): `frame[D] =
/// *(i32*)(ptr + (I-1)*4) as i64`. Holes/bounds identical to `arrld`; the only
/// difference is the 4-byte element stride and the sign-extending widen.
///
/// # Safety
/// See [`logos_stencil_arrld`]; the pinned buffer must hold 4-byte `i32`
/// elements (the `IntsI32` repr the VM pins for this lane).
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld_i32(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const i32;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(im1 as usize) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// 4-byte sign-extended UNCHECKED load (the `arrld_i32` twin without the bounds
/// check — Oracle-proven in range). Holes 0/1/3 only.
///
/// # Safety
/// See [`logos_stencil_arrld_i32`]; the index is proven in bounds.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrld_i32_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let im1 = i.wrapping_sub(1);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *const i32;
    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(im1 as usize) as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// 4-byte TRUNCATING pinned-array store (`ListRepr::IntsI32`): `*(i32*)(ptr +
/// (I-1)*4) = frame[S] as i32`. The narrowing proof guarantees the value fits
/// `i32`, so the truncation is lossless. Holes/bounds identical to `arrst`.
///
/// # Safety
/// See [`logos_stencil_arrld_i32`].
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrst_i32(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let len = *base.add(LOGOS_HOLE_I64_2 as usize);
    let im1 = i.wrapping_sub(1);
    if (im1 as u64) >= (len as u64) {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i32;
    *ptr.add(im1 as usize) = *base.add(LOGOS_HOLE_I64_3 as usize) as i32;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// 4-byte truncating UNCHECKED store (the `arrst_i32` twin without the bounds
/// check). Holes 0/1/3 only.
///
/// # Safety
/// See [`logos_stencil_arrst_i32`]; the index is proven in bounds.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_arrst_i32_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let i = *base.add(LOGOS_HOLE_I64_0 as usize);
    let im1 = i.wrapping_sub(1);
    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i32;
    *ptr.add(im1 as usize) = *base.add(LOGOS_HOLE_I64_3 as usize) as i32;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// `frame[D] = bits(frame[S] as f64)` — the kernel's Int→Float promotion.
///
/// # Safety
/// `base` must point at a frame larger than both patched indices.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_i2f2(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let v = *base.add(LOGOS_HOLE_I64_0 as usize);
    *base.add(LOGOS_HOLE_I64_1 as usize) = (v as f64).to_bits() as i64;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two; zero divisor exits to continuation 1 (the deopt piece) BEFORE
/// any effect, else push the wrapping quotient and continue to 0.
/// `wrapping_div` transcribes the kernel's locked Int spec: `i64::MIN / -1`
/// wraps to `i64::MIN`, never traps.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_divi_checked(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    if b == 0 {
        return logos_hole_cont_1(base, sp.sub(2), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    *sp.sub(2) = a.wrapping_div(b);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop two; zero divisor exits to continuation 1, else push the wrapping
/// remainder (`i64::MIN % -1` wraps to 0, like the kernel) and continue to 0.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_modi_checked(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    if b == 0 {
        return logos_hole_cont_1(base, sp.sub(2), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    *sp.sub(2) = a.wrapping_rem(b);
    logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// The side-exit terminal: store 1 into the chain's status cell (the patched
/// constant is the cell's ADDRESS) and unwind with 0. The caller reads the
/// cell, discards every native effect (its private frame), and replays from
/// the entry point on bytecode — where the kernel raises the exact error.
///
/// # Safety
/// The patched constant must be the address of a live, 8-byte-aligned i64
/// owned by the chain (it outlives every run).
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_deopt(_base: *mut i64, _sp: *mut i64, _r0: i64, _r1: i64, _r2: i64, _r3: i64, _f0: f64, _f1: f64, _f2: f64, _f3: f64, _f4: f64, _f5: f64) -> i64 {
    core::ptr::write_volatile(LOGOS_HOLE_I64_0 as *mut i64, 1);
    0
}

/// Precise side-exit: stores an ENCODED deopt value (hole 1 carries the
/// bytecode pc of the faulting op, `(pc << 2) | 3`) so the VM can
/// materialize the native call chain and resume interpreting AT that op —
/// effects already landed stay landed (region-grade deopt for functions).
///
/// # Safety
/// The patched status address must be the chain's live status cell.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_deopt_at(_base: *mut i64, _sp: *mut i64, _r0: i64, _r1: i64, _r2: i64, _r3: i64, _f0: f64, _f1: f64, _f2: f64, _f3: f64, _f4: f64, _f5: f64) -> i64 {
    let depth = core::ptr::read_volatile(LOGOS_HOLE_I64_2 as *const i64);
    core::ptr::write_volatile(LOGOS_HOLE_I64_0 as *mut i64, LOGOS_HOLE_I64_1 | (depth << 32));
    0
}

/// The PRECISE-DEOPT variant of the self-call stencil: identical machine
/// model, but a failed entry check stores hole 6's ENCODED value (the
/// bytecode pc of this Call, `(pc << 2) | 3`) instead of the plain replay
/// marker — the VM resumes at the Call op with every prior effect intact.
/// The depth limit is BAKED (the kernel's `MAX_CALL_DEPTH`) because the
/// encoded value occupies its hole; the adapter asserts the contract before
/// selecting this stencil.
///
/// # Safety
/// As for `logos_stencil_call`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_call_precise(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let table = LOGOS_HOLE_I64_0 as *const i64;
    let entry_raw = core::ptr::read_volatile(table);
    let depth = LOGOS_HOLE_I64_3 as *mut i64;
    let regc = core::ptr::read_volatile(table.add(1));
    let callee_base = base.add(LOGOS_HOLE_I64_1 as usize);
    let arena_end = *base.add(LOGOS_HOLE_I64_5 as usize);
    if entry_raw == 0
        || *depth >= MAX_CALL_DEPTH
        || (callee_base as i64).wrapping_add(regc.wrapping_add(3).wrapping_mul(8)) > arena_end
    {
        core::ptr::write_volatile(
            LOGOS_HOLE_I64_4 as *mut i64,
            LOGOS_HOLE_I64_6 | (*depth << 32),
        );
        return 0;
    }
    *callee_base.add(regc.wrapping_add(2) as usize) = arena_end;
    *depth += 1;
    let f: unsafe extern "C" fn(*mut i64, *mut i64, i64, i64, i64, i64, f64, f64, f64, f64) -> i64 =
        core::mem::transmute(entry_raw);
    // The callee's own entry reloads its pinned registers from its frame —
    // fresh zeros are correct here. This is a REAL call: SysV makes
    // rdx/rcx/r8/r9 caller-saved, so LLVM preserves OUR live r0..r3 around
    // it automatically.
    let r = f(callee_base, sp, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0.0);
    *depth -= 1;
    if core::ptr::read_volatile(LOGOS_HOLE_I64_4 as *const i64) != 0 {
        return 0;
    }
    *base.add(LOGOS_HOLE_I64_2 as usize) = r;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Map GET through a runtime helper (address = hole 3, an indirect call):
/// `helper(map_ptr, key, &mut out)` returns nonzero on an INT hit (out is
/// the value) and zero on a miss or a non-Int value — the side exit
/// (continuation 1) replays on bytecode where the kernel raises the exact
/// `Key not found` error or hands back the boxed value. Holes: 0 = map
/// pointer slot, 1 = key slot, 2 = destination slot, 3 = helper address.
///
/// # Safety
/// The map-pointer slot must hold a live `*mut MapStorage` pinned by the
/// caller for the whole run.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_maphget(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    // The helper writes the destination FRAME slot itself — no local
    // out-parameter, so both continuations stay sibling calls.
    let helper: unsafe extern "C" fn(i64, i64, *mut i64, i64) -> i64 =
        core::mem::transmute(LOGOS_HOLE_I64_3);
    let map_ptr = *base.add(LOGOS_HOLE_I64_0 as usize);
    let key = *base.add(LOGOS_HOLE_I64_1 as usize);
    if helper(map_ptr, key, base, LOGOS_HOLE_I64_2) == 0 {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Map SET through a runtime helper: `helper(map_ptr, key, value)` —
/// inserts always succeed. Holes: 0 = map pointer slot, 1 = key slot,
/// 2 = value slot, 3 = helper address.
///
/// # Safety
/// As for `logos_stencil_maphget`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_maphset(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(i64, i64, i64) =
        core::mem::transmute(LOGOS_HOLE_I64_3);
    helper(
        *base.add(LOGOS_HOLE_I64_0 as usize),
        *base.add(LOGOS_HOLE_I64_1 as usize),
        *base.add(LOGOS_HOLE_I64_2 as usize),
    );
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Map CONTAINS through a runtime helper: `helper(map_ptr, key)` yields
/// 0/1 into the destination. Holes: 0 = map pointer slot, 1 = key slot,
/// 2 = destination slot, 3 = helper address.
///
/// # Safety
/// As for `logos_stencil_maphget`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_maphhas(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(i64, i64) -> i64 =
        core::mem::transmute(LOGOS_HOLE_I64_3);
    let hit = helper(
        *base.add(LOGOS_HOLE_I64_0 as usize),
        *base.add(LOGOS_HOLE_I64_1 as usize),
    );
    *base.add(LOGOS_HOLE_I64_2 as usize) = hit;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// COUNT OVERLAPPING SUBSTRING MATCHES through a runtime helper (the whole
/// naive-search nest collapsed to one piece): `helper(frame, h_ptr, h_len,
/// n_ptr, n_len, needle_len, i, count)` reads the pinned haystack/needle pins,
/// counts overlapping needle occurrences over the outer range, ADDS them into
/// the count slot, and advances the `i` slot to the loop's exit value. It
/// returns nonzero on success and zero on the deopt sentinel — the side exit
/// (continuation 1) replays the exact nest on bytecode (e.g. a checked needle
/// index past the needle buffer raises the kernel's `index out of bounds`).
/// Holes: 0 = haystack pointer slot, 1 = haystack length slot, 2 = needle
/// pointer slot, 3 = needle length slot, 4 = needleLen value slot, 5 = i (start)
/// slot, 6 = count slot, 7 = helper address.
///
/// # Safety
/// The pin slots must hold live pinned buffer pointers/lengths borrowed for the
/// whole run; the helper must be the runtime's frame-driven memmem counter.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_memmem(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(*mut i64, i64, i64, i64, i64, i64, i64, i64) -> i64 =
        core::mem::transmute(LOGOS_HOLE_I64_7);
    if helper(
        base,
        LOGOS_HOLE_I64_0,
        LOGOS_HOLE_I64_1,
        LOGOS_HOLE_I64_2,
        LOGOS_HOLE_I64_3,
        LOGOS_HOLE_I64_4,
        LOGOS_HOLE_I64_5,
        LOGOS_HOLE_I64_6,
    ) == 0
    {
        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);
    }
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Fresh-list allocation through a runtime helper: plants the pin triple
/// (vec handle, buffer pointer, length 0) for a registry-owned list.
/// Holes: 0 = vec slot, 1 = pointer slot, 2 = length slot, 3 = helper.
///
/// # Safety
/// The helper must be the runtime's allocator taking `(frame, vec, ptr,
/// len)` slot indexes.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_alloclist(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(*mut i64, i64, i64, i64) =
        core::mem::transmute(LOGOS_HOLE_I64_3);
    helper(base, LOGOS_HOLE_I64_0, LOGOS_HOLE_I64_1, LOGOS_HOLE_I64_2);
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Plant a pin triple for a list HANDLE already in a frame slot (the
/// caller's view of a self-call's returned list). Holes: 0 = handle slot,
/// 1 = vec slot, 2 = pointer slot, 3 = length slot, 4 = helper.
///
/// # Safety
/// `frame[handle slot]` must hold a live `*mut Vec<i64>`.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_listtriple(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let helper: unsafe extern "C" fn(*mut i64, i64, i64, i64, i64) =
        core::mem::transmute(LOGOS_HOLE_I64_4);
    helper(
        base,
        LOGOS_HOLE_I64_0,
        LOGOS_HOLE_I64_1,
        LOGOS_HOLE_I64_2,
        LOGOS_HOLE_I64_3,
    );
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// The DIRECT self-call (mode A): the callee entry is a const hole whose
/// literal-pool word is PATCHED with this very chain's base after layout —
/// no table indirection, and the frame bound is static (hole 6 carries
/// the callee frame size; self-recursion means it is OUR OWN). The depth
/// limit is baked at the kernel's locked MAX_CALL_DEPTH. An unpatched
/// (zero) entry deopts — the chain never runs before its patch, but the
/// check keeps the failure mode safe.
///
/// # Safety
/// As for `logos_stencil_call`; hole 0's pool word must be patched before
/// the chain runs.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_call_self(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let entry_raw = LOGOS_HOLE_I64_0;
    let depth = LOGOS_HOLE_I64_3 as *mut i64;
    let callee_base = base.add(LOGOS_HOLE_I64_1 as usize);
    let frame_size = LOGOS_HOLE_I64_6;
    let arena_end = *base.add(LOGOS_HOLE_I64_5 as usize);
    // Distinct plain-deopt markers (all with low bits 01, never the precise
    // 11): 1 = unpatched entry, 5 = depth limit, 9 = arena bound.
    if entry_raw == 0 {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 1);
        return 0;
    }
    if *depth >= MAX_CALL_DEPTH {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 5);
        return 0;
    }
    if (callee_base as i64).wrapping_add(frame_size.wrapping_mul(8)) > arena_end {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 9);
        return 0;
    }
    *callee_base.add(frame_size.wrapping_sub(1) as usize) = arena_end;
    *depth += 1;
    let f: unsafe extern "C" fn(*mut i64, *mut i64, i64, i64, i64, i64, f64, f64, f64, f64) -> i64 =
        core::mem::transmute(entry_raw);
    let r = f(callee_base, sp, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0.0);
    *depth -= 1;
    if core::ptr::read_volatile(LOGOS_HOLE_I64_4 as *const i64) != 0 {
        return 0;
    }
    *base.add(LOGOS_HOLE_I64_2 as usize) = r;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// The DIRECT self-call with a FUSED argument copy (Lever A: the pinned-arg
/// self-call ABI). Identical machine model to `logos_stencil_call_self`, but
/// it stages the call's scalar arguments itself — copying the contiguous
/// source block `base[hole7 + j]` into the callee's parameter slots
/// `callee_base[j]` for `j` in `0..hole8` — instead of relying on `hole8`
/// separate frame-to-frame `Move` pieces ahead of the call. On this
/// dispatch/piece-bound engine that removes `hole8` pieces per self-call
/// (the dispatch-reduction win), and the result is bit-identical to the
/// per-`Move` staging: the same callee frame slots receive the same values
/// and the identical self-call is made. The copy is a raw 8-byte move, so it
/// carries Int, Bool, and f64 (bit-pattern) arguments alike.
///
/// Holes mirror `logos_stencil_call_self` (0 = patched entry, 1 = callee
/// base offset = the argument window, 2 = result slot, 3 = live-depth cell,
/// 4 = status cell, 5 = arena-limit slot, 6 = callee frame size) and add
/// 7 = source argument block start, 8 = argument count.
///
/// # Safety
/// As for `logos_stencil_call_self`; additionally the source block
/// `base[hole7 .. hole7 + hole8]` and the callee window
/// `base[hole1 .. hole1 + hole8]` must be live frame slots, and the source
/// block must not overlap the callee window (the JIT stages the window past
/// the caller's live registers, so they are disjoint).
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_call_self_copy(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let entry_raw = LOGOS_HOLE_I64_0;
    let depth = LOGOS_HOLE_I64_3 as *mut i64;
    let callee_base = base.add(LOGOS_HOLE_I64_1 as usize);
    let frame_size = LOGOS_HOLE_I64_6;
    let arena_end = *base.add(LOGOS_HOLE_I64_5 as usize);
    // Distinct plain-deopt markers, matching logos_stencil_call_self.
    if entry_raw == 0 {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 1);
        return 0;
    }
    if *depth >= MAX_CALL_DEPTH {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 5);
        return 0;
    }
    if (callee_base as i64).wrapping_add(frame_size.wrapping_mul(8)) > arena_end {
        core::ptr::write_volatile(LOGOS_HOLE_I64_4 as *mut i64, 9);
        return 0;
    }
    // Stage the arguments into the callee window in-piece (replaces the
    // per-argument Move stencils). Source and destination are disjoint and
    // the destination is the higher block, so an ascending copy is correct.
    let src = base.add(LOGOS_HOLE_I64_7 as usize);
    let mut j: i64 = 0;
    while j < LOGOS_HOLE_I64_8 {
        *callee_base.add(j as usize) = *src.add(j as usize);
        j += 1;
    }
    *callee_base.add(frame_size.wrapping_sub(1) as usize) = arena_end;
    *depth += 1;
    let f: unsafe extern "C" fn(*mut i64, *mut i64, i64, i64, i64, i64, f64, f64, f64, f64) -> i64 =
        core::mem::transmute(entry_raw);
    let r = f(callee_base, sp, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0.0);
    *depth -= 1;
    if core::ptr::read_volatile(LOGOS_HOLE_I64_4 as *const i64) != 0 {
        return 0;
    }
    *base.add(LOGOS_HOLE_I64_2 as usize) = r;
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Unconditional transfer — a pure tail jump to the continuation (used for
/// loop back-edges and join points).
///
/// # Safety
/// Trivially safe; `base`/`sp` pass through untouched.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_jump(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
}

/// Pop the condition; nonzero continues to hole 0, zero to hole 1.
///
/// # Safety
/// `sp` must point one past at least one live operand slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_branch_if(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {
    let c = *sp.sub(1);
    if c != 0 {
        logos_hole_cont_0(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    } else {
        logos_hole_cont_1(base, sp.sub(1), r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)
    }
}

/// Pop the result and RETURN it — the terminal of every stencil chain
/// (relocation-free by construction).
///
/// # Safety
/// `sp` must point one past at least one live operand slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_return(_base: *mut i64, sp: *mut i64) -> i64 {
    *sp.sub(1)
}
