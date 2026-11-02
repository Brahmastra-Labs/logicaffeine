//! Per-function WebAssembly lowering: a region of VM bytecode (`&[Op]`, a register machine)
//! → a WebAssembly function (a stack machine with i64 locals).
//!
//! ## Control flow
//!
//! VM bytecode uses absolute jumps; WebAssembly has only *structured* control flow. The
//! lowering splits the region into basic blocks and emits the standard **dispatch loop**:
//! a `loop` wrapping `block`s nested one-per-basic-block, with a `br_table` on a "next
//! block" local at the bottom. Each block runs its arithmetic, then sets the next-block
//! index and re-dispatches (`br` to the loop) — or `return`s. This translates *any*
//! reducible (and irreducible) control flow correctly, not just recognized patterns.
//!
//! ## Eligibility (Phase 6 deopt-at-concurrency, mirrored)
//!
//! Only integer regions are emitted: const/move/arith/compare/jump/return. Any op outside
//! that set — every concurrency / channel / networking op included — makes
//! [`compile_region_to_wasm`] return `None`, so the region stays on the bytecode tier (a
//! yield-free emitted module by construction, exactly like the native backend's deny-list).
//!
//! This is the byte emitter both the browser WASM-JIT tier (`super::region_jit`) and the
//! direct AOT backend consume; it depends only on [`super::encode`], not on the `wasm-jit`
//! feature.

use super::cfg::{assemble_dispatch_loop, goto_block, Blocks};
use super::encode::*;
use crate::vm::instruction::{CompiledProgram, Constant, Op};

/// Whether a region op is WASM-JIT-eligible (the integer fragment). Everything else —
/// notably every concurrency / channel / networking op — is rejected so the region stays
/// on the bytecode tier.
fn is_supported(op: &Op) -> bool {
    matches!(
        op,
        Op::LoadConst { .. }
            | Op::Move { .. }
            | Op::Add { .. }
            | Op::Sub { .. }
            | Op::Mul { .. }
            | Op::Div { .. }
            | Op::Mod { .. }
            | Op::BitXor { .. }
            | Op::BitAnd { .. }
            | Op::BitOr { .. }
            | Op::Shl { .. }
            | Op::Shr { .. }
            | Op::Lt { .. }
            | Op::Gt { .. }
            | Op::LtEq { .. }
            | Op::GtEq { .. }
            | Op::Eq { .. }
            | Op::NotEq { .. }
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
    )
}

/// Lower an integer region to a WebAssembly module exporting one function `f` of
/// `num_params` `i64` parameters returning `i64`. VM registers map 1:1 to WASM locals
/// (`0..num_params` params, `num_params..num_regs` declared i64 locals); one extra i32
/// local at index `num_regs` is the dispatch "next block" index.
///
/// Returns `None` — the region stays on the bytecode tier — for any op outside the integer
/// fragment (control flow, calls, and *all* concurrency / channel / networking ops), a
/// non-`Int` constant, an out-of-range jump target, or a region without a reachable
/// `Return`.
pub fn compile_region_to_wasm(
    ops: &[Op],
    constants: &[Constant],
    num_params: u32,
    num_regs: u32,
) -> Option<Vec<u8>> {
    let n = ops.len();
    if n == 0 {
        return None;
    }

    // ---- 1. Eligibility: the integer fragment, and a reachable `Return` ----
    let mut has_return = false;
    for op in ops {
        if !is_supported(op) {
            return None;
        }
        if matches!(op, Op::Return { .. }) {
            has_return = true;
        }
    }
    if !has_return {
        return None;
    }

    let blocks = Blocks::new(ops)?;
    let num_blocks = blocks.num_blocks();
    let pc_local = num_regs; // the dispatch "next block" index lives just past the registers

    // ---- 2. Emit each block's body + terminator ----
    let mut blocks_code: Vec<Vec<u8>> = Vec::with_capacity(num_blocks);
    for k in 0..num_blocks {
        let start = blocks.start(k);
        let end = blocks.end(k);
        let mut code = Vec::new();
        let mut terminated = false;
        for pc in start..end {
            match ops[pc] {
                Op::LoadConst { dst, idx } => {
                    let v = match constants.get(idx as usize)? {
                        Constant::Int(x) => *x,
                        _ => return None,
                    };
                    code.push(0x42); // i64.const
                    leb_i64(&mut code, v);
                    local_set(&mut code, dst as u32);
                }
                Op::Move { dst, src } => {
                    local_get(&mut code, src as u32);
                    local_set(&mut code, dst as u32);
                }
                // Add/Sub/Mul are CHECKED: they trap on signed overflow so the task falls
                // back to the VM (which promotes to BigInt) — never a silent wrap. This keeps
                // every tier in sync on the EXACT-integer contract.
                Op::Add { dst, lhs, rhs } => emit_checked_addsub(&mut code, false, dst, lhs, rhs),
                Op::Sub { dst, lhs, rhs } => emit_checked_addsub(&mut code, true, dst, lhs, rhs),
                Op::Mul { dst, lhs, rhs } => emit_checked_mul(&mut code, dst, lhs, rhs),
                // Div/Mod already trap on their edge cases (div-by-zero, i64::MIN / -1) in WASM
                // → host error → VM fallback, so no extra guard is needed.
                Op::Div { dst, lhs, rhs } => arith(&mut code, 0x7F, dst, lhs, rhs), // i64.div_s
                Op::Mod { dst, lhs, rhs } => arith(&mut code, 0x81, dst, lhs, rhs), // i64.rem_s
                // Bitwise / shift: pure wrapping, no overflow concern. The VM masks the shift
                // count mod 64 (`wrapping_shl`/`wrapping_shr`); WASM's i64.shl/shr_s do the
                // same, and shr is ARITHMETIC (signed) to match `wrapping_shr` on i64.
                Op::BitXor { dst, lhs, rhs } => arith(&mut code, 0x85, dst, lhs, rhs), // i64.xor
                Op::BitAnd { dst, lhs, rhs } => arith(&mut code, 0x83, dst, lhs, rhs), // i64.and
                Op::BitOr { dst, lhs, rhs } => arith(&mut code, 0x84, dst, lhs, rhs),  // i64.or
                Op::Shl { dst, lhs, rhs } => arith(&mut code, 0x86, dst, lhs, rhs),    // i64.shl
                Op::Shr { dst, lhs, rhs } => arith(&mut code, 0x87, dst, lhs, rhs),    // i64.shr_s
                Op::Lt { dst, lhs, rhs } => compare(&mut code, 0x53, dst, lhs, rhs),   // i64.lt_s
                Op::Gt { dst, lhs, rhs } => compare(&mut code, 0x55, dst, lhs, rhs),   // i64.gt_s
                Op::LtEq { dst, lhs, rhs } => compare(&mut code, 0x57, dst, lhs, rhs), // i64.le_s
                Op::GtEq { dst, lhs, rhs } => compare(&mut code, 0x59, dst, lhs, rhs), // i64.ge_s
                Op::Eq { dst, lhs, rhs } => compare(&mut code, 0x51, dst, lhs, rhs),   // i64.eq
                Op::NotEq { dst, lhs, rhs } => compare(&mut code, 0x52, dst, lhs, rhs), // i64.ne
                Op::Jump { target } => {
                    goto_block(&mut code, pc_local, &blocks, k, blocks.block_of(target));
                    terminated = true;
                    break;
                }
                Op::JumpIfFalse { cond, target } => {
                    emit_cond_jump(
                        &mut code,
                        cond,
                        true,
                        blocks.block_of(target),
                        blocks.block_of(pc + 1),
                        pc_local,
                        blocks.br_loop(k),
                    );
                    terminated = true;
                    break;
                }
                Op::JumpIfTrue { cond, target } => {
                    emit_cond_jump(
                        &mut code,
                        cond,
                        false,
                        blocks.block_of(target),
                        blocks.block_of(pc + 1),
                        pc_local,
                        blocks.br_loop(k),
                    );
                    terminated = true;
                    break;
                }
                Op::Return { src } => {
                    local_get(&mut code, src as u32);
                    code.push(0x0F); // return
                    terminated = true;
                    break;
                }
                Op::ReturnNothing => {
                    // The compiler appends a `ReturnNothing` after a value `Return` as a
                    // fallthrough safety terminator. For an i64-returning function it is
                    // unreachable; encode it as a trap so any (erroneous) reachable path
                    // diverges from the VM and is caught by the differential.
                    code.push(0x00); // unreachable
                    terminated = true;
                    break;
                }
                _ => return None,
            }
        }
        if !terminated {
            // Fell off the block end into the next block: set next, re-dispatch.
            goto_block(&mut code, pc_local, &blocks, k, blocks.block_of(end));
        }
        blocks_code.push(code);
    }

    // ---- 3. Assemble the function body: dispatch loop ----
    let mut body = assemble_dispatch_loop(pc_local, &blocks_code);
    body.push(0x00); // unreachable (only reached via $exit / default — never on a Return path)
    body.push(0x0B); // end function

    // ---- 4. Module sections ----
    let mut module = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

    let mut ty = Vec::new();
    leb_u32(&mut ty, 1);
    ty.push(0x60);
    leb_u32(&mut ty, num_params);
    for _ in 0..num_params {
        ty.push(I64);
    }
    leb_u32(&mut ty, 1);
    ty.push(I64);
    section(&mut module, 1, &ty);

    let mut func = Vec::new();
    leb_u32(&mut func, 1);
    leb_u32(&mut func, 0);
    section(&mut module, 3, &func);

    let mut export = Vec::new();
    leb_u32(&mut export, 1);
    leb_u32(&mut export, 1);
    export.push(b'f');
    export.push(0x00);
    leb_u32(&mut export, 0);
    section(&mut module, 7, &export);

    // Code section: locals = (num_regs - num_params) i64 + one i32 (the dispatch index).
    let num_i64_locals = num_regs.saturating_sub(num_params);
    let mut entry = Vec::new();
    leb_u32(&mut entry, if num_i64_locals > 0 { 2 } else { 1 }); // local groups
    if num_i64_locals > 0 {
        leb_u32(&mut entry, num_i64_locals);
        entry.push(I64);
    }
    leb_u32(&mut entry, 1);
    entry.push(I32);
    entry.extend_from_slice(&body);
    let mut code_sec = Vec::new();
    leb_u32(&mut code_sec, 1);
    leb_u32(&mut code_sec, entry.len() as u32);
    code_sec.extend_from_slice(&entry);
    section(&mut module, 10, &code_sec);

    Some(module)
}

/// Lower function `fi` of a compiled program to a WebAssembly module — the integration
/// entry the VM's tier-up uses. A function's body lives in the shared `program.code` from
/// its `entry_pc` until the next function's entry (or the program end), with **absolute**
/// jump targets; this extracts that slice and rebases every jump into the region-local
/// 0-based space [`compile_region_to_wasm`] expects. Returns `None` (stay on bytecode) if
/// the body leaves the integer fragment or a jump escapes the function.
pub fn compile_function_to_wasm(program: &CompiledProgram, fi: usize) -> Option<Vec<u8>> {
    let f = program.functions.get(fi)?;
    // The emitted module returns an i64 that `WasmTier::on_call`'s caller boxes as an `Int`.
    // That is only sound when the function actually returns an Int — a `Bool`-returning
    // function (a comparison result) would be mis-boxed as `Int(1)` instead of `Bool(true)`
    // (`Show` would print `1`, not `true`), and `Float` rides different bits. So tier ONLY
    // declared-`Int` returns; everything else (Bool / Float / inferred-`None`) deopts to the
    // VM, which types the result correctly. Keeps every tier in sync on return types.
    if f.ret_kind != Some(crate::vm::native_tier::SlotKind::Int) {
        return None;
    }
    let entry = f.entry_pc;
    // The function spans [entry, next_entry) — functions are concatenated after Main.
    let end = program
        .functions
        .iter()
        .map(|g| g.entry_pc)
        .filter(|&e| e > entry)
        .min()
        .unwrap_or(program.code.len());
    if entry >= end || end > program.code.len() {
        return None;
    }
    let rebase = |t: usize| -> Option<usize> {
        if t >= entry && t < end {
            Some(t - entry)
        } else {
            None // a jump out of the function body — not a self-contained region
        }
    };
    let mut region: Vec<Op> = Vec::with_capacity(end - entry);
    for &op in &program.code[entry..end] {
        region.push(match op {
            Op::Jump { target } => Op::Jump {
                target: rebase(target)?,
            },
            Op::JumpIfFalse { cond, target } => Op::JumpIfFalse {
                cond,
                target: rebase(target)?,
            },
            Op::JumpIfTrue { cond, target } => Op::JumpIfTrue {
                cond,
                target: rebase(target)?,
            },
            other => other,
        });
    }
    compile_region_to_wasm(
        &region,
        &program.constants,
        f.param_count as u32,
        f.register_count as u32,
    )
}

#[cfg(all(test, feature = "wasm-jit", not(target_arch = "wasm32")))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/func/tests.rs"]
mod tests;
