//! WebAssembly binary-encoding primitives — the shared byte layer beneath both the browser
//! WASM-JIT tier (`super::region_jit`) and the direct AOT backend ([`super::module`]).
//!
//! Hand-rolled LEB128 + section/instruction encoders (no external wasm crate): the same
//! routines `wasm_jit.rs` proved correct against `wasmi` and real V8. Nothing here depends on
//! the `wasm-jit` feature — byte emission is target- and feature-independent; only *running*
//! the emitted modules (`super::region_jit`) touches `wasmi`/`js_sys`.

use crate::vm::instruction::Reg;

pub(crate) const I64: u8 = 0x7E;
pub(crate) const I32: u8 = 0x7F;
pub(crate) const F64: u8 = 0x7C;
pub(crate) const VOID_BLOCKTYPE: u8 = 0x40;

pub(crate) fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

pub(crate) fn leb_i64(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7; // arithmetic shift — sign-extends
        let sign = byte & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

pub(crate) fn section(out: &mut Vec<u8>, id: u8, content: &[u8]) {
    out.push(id);
    leb_u32(out, content.len() as u32);
    out.extend_from_slice(content);
}

pub(crate) fn local_get(out: &mut Vec<u8>, idx: u32) {
    out.push(0x20);
    leb_u32(out, idx);
}

pub(crate) fn local_set(out: &mut Vec<u8>, idx: u32) {
    out.push(0x21);
    leb_u32(out, idx);
}

pub(crate) fn local_tee(out: &mut Vec<u8>, idx: u32) {
    out.push(0x22);
    leb_u32(out, idx);
}

pub(crate) fn global_get(out: &mut Vec<u8>, idx: u32) {
    out.push(0x23);
    leb_u32(out, idx);
}

pub(crate) fn global_set(out: &mut Vec<u8>, idx: u32) {
    out.push(0x24);
    leb_u32(out, idx);
}

/// Signed LEB128 of an `i32` (for `i32.const`).
pub(crate) fn leb_i32(out: &mut Vec<u8>, mut v: i32) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7; // arithmetic shift — sign-extends
        let sign = byte & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

pub(crate) fn i32_const(out: &mut Vec<u8>, v: i32) {
    out.push(0x41);
    leb_i32(out, v);
}

/// `<base on stack> <opcode> align offset` — a linear-memory load/store. `align` is the
/// natural-alignment log2 (2 for i32/f32, 3 for i64/f64).
fn mem_op(out: &mut Vec<u8>, opcode: u8, align: u32, offset: u32) {
    out.push(opcode);
    leb_u32(out, align);
    leb_u32(out, offset);
}

pub(crate) fn i32_load(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x28, 2, offset);
}
pub(crate) fn i32_store(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x36, 2, offset);
}
pub(crate) fn i64_load(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x29, 3, offset);
}
pub(crate) fn i64_store(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x37, 3, offset);
}
pub(crate) fn f64_load(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x2B, 3, offset);
}
pub(crate) fn f64_store(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x39, 3, offset);
}
/// `i32.load8_u` / `i32.store8` — single-byte access (alignment 0), for UTF-8 `Text` buffers.
pub(crate) fn i32_load8_u(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x2D, 0, offset);
}
pub(crate) fn i32_store8(out: &mut Vec<u8>, offset: u32) {
    mem_op(out, 0x3A, 0, offset);
}

/// `local.get lhs; local.get rhs; <opcode>; local.set dst`.
pub(crate) fn arith(out: &mut Vec<u8>, opcode: u8, dst: Reg, lhs: Reg, rhs: Reg) {
    local_get(out, lhs as u32);
    local_get(out, rhs as u32);
    out.push(opcode);
    local_set(out, dst as u32);
}

/// A signed i64 comparison producing a 0/1 *i64* (matching the VM's truthy-Int booleans):
/// `local.get lhs; local.get rhs; <cmp i32>; i64.extend_i32_u; local.set dst`.
pub(crate) fn compare(out: &mut Vec<u8>, cmp_opcode: u8, dst: Reg, lhs: Reg, rhs: Reg) {
    local_get(out, lhs as u32);
    local_get(out, rhs as u32);
    out.push(cmp_opcode);
    out.push(0xAD); // i64.extend_i32_u
    local_set(out, dst as u32);
}

/// Emit a CHECKED `add` / `sub`: compute the wrapping i64 result, then `unreachable` (trap)
/// on signed overflow. This keeps the WASM-JIT IN SYNC with the rest of the engine: the VM's
/// integer arithmetic is EXACT (`checked_add`/`checked_sub` → promote to BigInt on overflow,
/// see `semantics/arith.rs`), and the native forge tiers side-exit (`jo`) on overflow for the
/// same reason. A trap propagates as a host error, so `WasmTier::call` returns `None` and the
/// task falls back to the bytecode VM, which promotes correctly — never a silent wrap. Signed
/// overflow test: add ⟺ `(lhs ^ dst) & (rhs ^ dst) < 0`; sub ⟺ `(lhs ^ rhs) & (lhs ^ dst) < 0`.
pub(crate) fn emit_checked_addsub(out: &mut Vec<u8>, is_sub: bool, dst: Reg, lhs: Reg, rhs: Reg) {
    local_get(out, lhs as u32);
    local_get(out, rhs as u32);
    out.push(if is_sub { 0x7D } else { 0x7C }); // i64.sub / i64.add
    local_set(out, dst as u32);
    if is_sub {
        local_get(out, lhs as u32);
        local_get(out, rhs as u32);
        out.push(0x85); // (lhs ^ rhs)
        local_get(out, lhs as u32);
        local_get(out, dst as u32);
        out.push(0x85); // (lhs ^ dst)
    } else {
        local_get(out, lhs as u32);
        local_get(out, dst as u32);
        out.push(0x85); // (lhs ^ dst)
        local_get(out, rhs as u32);
        local_get(out, dst as u32);
        out.push(0x85); // (rhs ^ dst)
    }
    out.push(0x83); // i64.and
    out.push(0x42);
    out.push(0x00); // i64.const 0
    out.push(0x53); // i64.lt_s  → 1 if the overflow word is negative
    out.push(0x04);
    out.push(0x40); // if (void)
    out.push(0x00); // unreachable (overflow → trap → VM fallback)
    out.push(0x0B); // end
}

/// Emit a CHECKED `mul`: wrapping i64 product, then trap on signed overflow. Overflow ⟺
/// `lhs != 0 && dst / lhs != rhs` (the division-check, since WASM has no i128). The
/// `i64.div_s` runs only on the `lhs != 0` branch; the one input that would still trap it
/// (`dst == i64::MIN && lhs == -1`) is itself an overflow case, so the trap is the correct
/// deopt. Same SYNC rationale as [`emit_checked_addsub`] (`checked_mul` → BigInt in the VM).
pub(crate) fn emit_checked_mul(out: &mut Vec<u8>, dst: Reg, lhs: Reg, rhs: Reg) {
    local_get(out, lhs as u32);
    local_get(out, rhs as u32);
    out.push(0x7E); // i64.mul
    local_set(out, dst as u32);
    local_get(out, lhs as u32);
    out.push(0x50); // i64.eqz → 1 if lhs == 0 (then: no overflow possible)
    out.push(0x04);
    out.push(0x40); // if (void)  — lhs == 0: nothing to check
    out.push(0x05); // else       — lhs != 0: verify dst / lhs == rhs
    local_get(out, dst as u32);
    local_get(out, lhs as u32);
    out.push(0x7F); // i64.div_s  (lhs != 0 here)
    local_get(out, rhs as u32);
    out.push(0x52); // i64.ne → 1 if (dst / lhs) != rhs
    out.push(0x04);
    out.push(0x40); // if (void)
    out.push(0x00); // unreachable (overflow → trap → VM fallback)
    out.push(0x0B); // end (inner if)
    out.push(0x0B); // end (outer if/else)
}

/// Emit a conditional jump's terminator: select the next block (target vs fallthrough) by
/// the condition's truthiness, then re-dispatch. `jump_when_false` is true for
/// `JumpIfFalse` (jump to `target` when the cond is 0), false for `JumpIfTrue`.
pub(crate) fn emit_cond_jump(
    code: &mut Vec<u8>,
    cond: Reg,
    jump_when_false: bool,
    target_block: usize,
    fallthrough_block: usize,
    pc_local: u32,
    loop_depth: u32,
) {
    // Stack for `select`: [target, fallthrough, selector] → selector!=0 ? target : fallthrough.
    code.push(0x41); // i32.const target
    leb_u32(code, target_block as u32);
    code.push(0x41); // i32.const fallthrough
    leb_u32(code, fallthrough_block as u32);
    local_get(code, cond as u32);
    if jump_when_false {
        code.push(0x50); // i64.eqz → i32 1 when cond is 0 (falsey) ⇒ pick target
    } else {
        // truthy: cond != 0 ⇒ pick target. (i64.eqz then i32.eqz = "is non-zero".)
        code.push(0x50); // i64.eqz
        code.push(0x45); // i32.eqz
    }
    code.push(0x1B); // select
    local_set(code, pc_local);
    code.push(0x0C); // br $loop
    leb_u32(code, loop_depth);
}
