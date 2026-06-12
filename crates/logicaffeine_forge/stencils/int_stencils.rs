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
    fn logos_hole_cont_0(base: *mut i64, sp: *mut i64) -> i64;
    fn logos_hole_cont_1(base: *mut i64, sp: *mut i64) -> i64;
    static LOGOS_HOLE_I64_0: i64;
}

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
pub unsafe extern "C" fn logos_stencil_const(base: *mut i64, sp: *mut i64) -> i64 {
    *sp = LOGOS_HOLE_I64_0;
    logos_hole_cont_0(base, sp.add(1))
}

/// Push frame slot [K], continue. K is the patched constant.
///
/// # Safety
/// `base` must point at a frame with more than K slots; `sp` must have a free
/// slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_slot_get(base: *mut i64, sp: *mut i64) -> i64 {
    *sp = *base.add(LOGOS_HOLE_I64_0 as usize);
    logos_hole_cont_0(base, sp.add(1))
}

/// Pop into frame slot [K], continue. K is the patched constant.
///
/// # Safety
/// `base` must point at a frame with more than K slots; `sp` must point one
/// past at least one live operand slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_slot_set(base: *mut i64, sp: *mut i64) -> i64 {
    *base.add(LOGOS_HOLE_I64_0 as usize) = *sp.sub(1);
    logos_hole_cont_0(base, sp.sub(1))
}

/// Pop two, push their wrapping sum, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_addi(base: *mut i64, sp: *mut i64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_add(b);
    logos_hole_cont_0(base, sp.sub(1))
}

/// Pop two, push their wrapping difference, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_subi(base: *mut i64, sp: *mut i64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_sub(b);
    logos_hole_cont_0(base, sp.sub(1))
}

/// Pop two, push their wrapping product, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_muli(base: *mut i64, sp: *mut i64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = a.wrapping_mul(b);
    logos_hole_cont_0(base, sp.sub(1))
}

/// Pop two, push `(a < b) as i64`, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_lti(base: *mut i64, sp: *mut i64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = (a < b) as i64;
    logos_hole_cont_0(base, sp.sub(1))
}

/// Pop two, push `(a == b) as i64`, continue.
///
/// # Safety
/// `sp` must point one past at least two live operand slots.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_eqi(base: *mut i64, sp: *mut i64) -> i64 {
    let b = *sp.sub(1);
    let a = *sp.sub(2);
    *sp.sub(2) = (a == b) as i64;
    logos_hole_cont_0(base, sp.sub(1))
}

/// Unconditional transfer — a pure tail jump to the continuation (used for
/// loop back-edges and join points).
///
/// # Safety
/// Trivially safe; `base`/`sp` pass through untouched.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_jump(base: *mut i64, sp: *mut i64) -> i64 {
    logos_hole_cont_0(base, sp)
}

/// Pop the condition; nonzero continues to hole 0, zero to hole 1.
///
/// # Safety
/// `sp` must point one past at least one live operand slot.
#[no_mangle]
pub unsafe extern "C" fn logos_stencil_branch_if(base: *mut i64, sp: *mut i64) -> i64 {
    let c = *sp.sub(1);
    if c != 0 {
        logos_hole_cont_0(base, sp.sub(1))
    } else {
        logos_hole_cont_1(base, sp.sub(1))
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
