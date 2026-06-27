//! WS6 (FINISH_INTERPRETER Phase 13) — the browser WASM-JIT backend.
//!
//! The native copy-and-patch JIT (`logicaffeine_forge`) patches x86 stencils into
//! executable memory — impossible in a browser, where there is no executable memory to
//! patch and only WebAssembly bytecode runs. The only path to JIT-level speed in WASM is a
//! *second code generator* that emits a fresh WebAssembly module per hot region and
//! instantiates it via the host's `WebAssembly.instantiate`. This module is that backend:
//! it lowers a hot region's VM bytecode (`&[Op]`, a register machine) to a WebAssembly
//! function (a stack machine with i64 locals), reusing the same tier-up *decision*, the
//! same `&[Op]` IR, and the same eligibility gate as the native JIT — only the emitted
//! bytes differ (WASM, not x86).
//!
//! It lives in the VM crate (not `logicaffeine_forge`) because forge is
//! `#![cfg(not(target_arch = "wasm32"))]` — it cannot build for wasm32 — whereas this
//! backend must build for and run on wasm32. The native 2.55×-C x86 JIT is untouched.
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

use std::collections::BTreeSet;

use super::instruction::{CompiledProgram, Constant, Op, Reg};

const I64: u8 = 0x7E;
const I32: u8 = 0x7F;
const VOID_BLOCKTYPE: u8 = 0x40;

fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
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

fn leb_i64(out: &mut Vec<u8>, mut v: i64) {
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

fn section(out: &mut Vec<u8>, id: u8, content: &[u8]) {
    out.push(id);
    leb_u32(out, content.len() as u32);
    out.extend_from_slice(content);
}

fn local_get(out: &mut Vec<u8>, idx: u32) {
    out.push(0x20);
    leb_u32(out, idx);
}

fn local_set(out: &mut Vec<u8>, idx: u32) {
    out.push(0x21);
    leb_u32(out, idx);
}

/// `local.get lhs; local.get rhs; <opcode>; local.set dst`.
fn arith(out: &mut Vec<u8>, opcode: u8, dst: Reg, lhs: Reg, rhs: Reg) {
    local_get(out, lhs as u32);
    local_get(out, rhs as u32);
    out.push(opcode);
    local_set(out, dst as u32);
}

/// A signed i64 comparison producing a 0/1 *i64* (matching the VM's truthy-Int booleans):
/// `local.get lhs; local.get rhs; <cmp i32>; i64.extend_i32_u; local.set dst`.
fn compare(out: &mut Vec<u8>, cmp_opcode: u8, dst: Reg, lhs: Reg, rhs: Reg) {
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
fn emit_checked_addsub(out: &mut Vec<u8>, is_sub: bool, dst: Reg, lhs: Reg, rhs: Reg) {
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
fn emit_checked_mul(out: &mut Vec<u8>, dst: Reg, lhs: Reg, rhs: Reg) {
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

    // ---- 1. Basic-block leaders ----
    let mut leaders: BTreeSet<usize> = BTreeSet::new();
    leaders.insert(0);
    let mut has_return = false;
    for (i, op) in ops.iter().enumerate() {
        if !is_supported(op) {
            return None;
        }
        match *op {
            Op::Jump { target } => {
                if target >= n {
                    return None;
                }
                leaders.insert(target);
                if i + 1 < n {
                    leaders.insert(i + 1);
                }
            }
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                if target >= n {
                    return None;
                }
                leaders.insert(target);
                if i + 1 < n {
                    leaders.insert(i + 1);
                }
            }
            Op::Return { .. } => {
                has_return = true;
                if i + 1 < n {
                    leaders.insert(i + 1);
                }
            }
            _ => {}
        }
    }
    if !has_return {
        return None;
    }

    let leader_vec: Vec<usize> = leaders.iter().copied().collect();
    let num_blocks = leader_vec.len();
    let block_of = |pc: usize| -> usize {
        // Targets and fallthroughs are always leaders by construction.
        leader_vec.binary_search(&pc).expect("leader")
    };
    // The end pc of each block (start of the next leader, or n for the last).
    let block_end = |k: usize| -> usize {
        if k + 1 < num_blocks {
            leader_vec[k + 1]
        } else {
            n
        }
    };

    let pc_local = num_regs; // the dispatch "next block" index lives just past the registers
    let br_loop = |k: usize| (num_blocks - 1 - k) as u32; // depth from block k's code to $loop
    let br_exit = |k: usize| (num_blocks - k) as u32; //     depth from block k's code to $exit

    // ---- 2. Emit each block's body + terminator ----
    let mut blocks_code: Vec<Vec<u8>> = Vec::with_capacity(num_blocks);
    for k in 0..num_blocks {
        let start = leader_vec[k];
        let end = block_end(k);
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
                Op::Shl { dst, lhs, rhs } => arith(&mut code, 0x86, dst, lhs, rhs),   // i64.shl
                Op::Shr { dst, lhs, rhs } => arith(&mut code, 0x87, dst, lhs, rhs),   // i64.shr_s
                Op::Lt { dst, lhs, rhs } => compare(&mut code, 0x53, dst, lhs, rhs), // i64.lt_s
                Op::Gt { dst, lhs, rhs } => compare(&mut code, 0x55, dst, lhs, rhs), // i64.gt_s
                Op::LtEq { dst, lhs, rhs } => compare(&mut code, 0x57, dst, lhs, rhs), // i64.le_s
                Op::GtEq { dst, lhs, rhs } => compare(&mut code, 0x59, dst, lhs, rhs), // i64.ge_s
                Op::Eq { dst, lhs, rhs } => compare(&mut code, 0x51, dst, lhs, rhs), // i64.eq
                Op::NotEq { dst, lhs, rhs } => compare(&mut code, 0x52, dst, lhs, rhs), // i64.ne
                Op::Jump { target } => {
                    code.push(0x41); // i32.const next-block
                    leb_u32(&mut code, block_of(target) as u32);
                    local_set(&mut code, pc_local);
                    code.push(0x0C); // br $loop
                    leb_u32(&mut code, br_loop(k));
                    terminated = true;
                    break;
                }
                Op::JumpIfFalse { cond, target } => {
                    emit_cond_jump(&mut code, cond, true, block_of(target), block_of(pc + 1), pc_local, br_loop(k));
                    terminated = true;
                    break;
                }
                Op::JumpIfTrue { cond, target } => {
                    emit_cond_jump(&mut code, cond, false, block_of(target), block_of(pc + 1), pc_local, br_loop(k));
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
            let next = block_of(end);
            code.push(0x41);
            leb_u32(&mut code, next as u32);
            local_set(&mut code, pc_local);
            code.push(0x0C);
            leb_u32(&mut code, br_loop(k));
            let _ = br_exit; // (reserved for an explicit exit target; unreachable today)
        }
        blocks_code.push(code);
    }

    // ---- 3. Assemble the function body: dispatch loop ----
    let mut body = Vec::new();
    body.push(0x02); // block $exit
    body.push(VOID_BLOCKTYPE);
    body.push(0x03); // loop $loop
    body.push(VOID_BLOCKTYPE);
    for _ in 0..num_blocks {
        body.push(0x02); // block $b_k
        body.push(VOID_BLOCKTYPE);
    }
    // br_table on the next-block index: target[k] = depth k (k-th dispatch block), default = $exit.
    local_get(&mut body, pc_local);
    body.push(0x0E); // br_table
    leb_u32(&mut body, num_blocks as u32); // target count
    for k in 0..num_blocks {
        leb_u32(&mut body, k as u32);
    }
    leb_u32(&mut body, (num_blocks + 1) as u32); // default → $exit (one past $loop)
    // Close each dispatch block, emitting its code after the corresponding `end`.
    for code in &blocks_code {
        body.push(0x0B); // end (closes the innermost remaining dispatch block)
        body.extend_from_slice(code);
    }
    body.push(0x0B); // end $loop
    body.push(0x0B); // end $exit
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
    if f.ret_kind != Some(super::native_tier::SlotKind::Int) {
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
            Op::Jump { target } => Op::Jump { target: rebase(target)? },
            Op::JumpIfFalse { cond, target } => Op::JumpIfFalse { cond, target: rebase(target)? },
            Op::JumpIfTrue { cond, target } => Op::JumpIfTrue { cond, target: rebase(target)? },
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

/// Emit a conditional jump's terminator: select the next block (target vs fallthrough) by
/// the condition's truthiness, then re-dispatch. `jump_when_false` is true for
/// `JumpIfFalse` (jump to `target` when the cond is 0), false for `JumpIfTrue`.
fn emit_cond_jump(
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

/// The runtime WASM-JIT tier: per-function call counters plus a cache of compiled +
/// instantiated modules. When a function crosses the hot threshold it is lowered to WASM and
/// instantiated; subsequent calls dispatch to the compiled module. Functions the codegen
/// declines are remembered as `Ineligible` and stay on the bytecode tier. The VM's
/// `Op::Call` consults this only under `#[cfg(feature = "wasm-jit")]`, so the default build
/// never carries it.
///
/// The host differs by target — and this is the whole point of the WASM backend:
/// - **native**: the pure-Rust [`wasmi`] interpreter. This is also the codegen oracle the
///   differential tests cross-check the emitter against.
/// - **wasm32**: the platform's *real* `WebAssembly` (V8 in the browser / node) via
///   [`js_sys::WebAssembly`]. This is the production tier — a hot function tiers up to a
///   freshly-compiled native WebAssembly module the host JITs, exactly as the spec requires.
///
/// `on_call` and the tier-up bookkeeping are target-independent; only [`instantiate`] and
/// [`ReadyModule::call`] (the host seam) are `#[cfg]`-split.
pub struct WasmTier {
    entries: std::collections::HashMap<u16, TierEntry>,
    threshold: u32,
    hits: u64,
}

enum TierEntry {
    Pending(u32),
    Ready(ReadyModule),
    Ineligible,
}

/// A compiled + instantiated module ready to call. Native holds the `wasmi` store + export;
/// wasm32 holds the host `WebAssembly.Instance`'s exported function.
#[cfg(not(target_arch = "wasm32"))]
struct ReadyModule {
    store: wasmi::Store<()>,
    func: wasmi::Func,
}

#[cfg(target_arch = "wasm32")]
struct ReadyModule {
    func: js_sys::Function,
}

impl ReadyModule {
    /// Call the module's `f` with i64 args, returning its i64 result.
    #[cfg(not(target_arch = "wasm32"))]
    fn call(&mut self, args: &[i64]) -> Option<i64> {
        let argv: Vec<wasmi::Value> = args.iter().map(|&a| wasmi::Value::I64(a)).collect();
        let mut results = [wasmi::Value::I64(0)];
        self.func.call(&mut self.store, &argv, &mut results).ok()?;
        match results[0] {
            wasmi::Value::I64(v) => Some(v),
            _ => None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn call(&mut self, args: &[i64]) -> Option<i64> {
        call_host_func(&self.func, args)
    }
}

impl WasmTier {
    /// A tier that compiles a function after `threshold` calls (clamped to ≥1).
    pub fn new(threshold: u32) -> Self {
        WasmTier {
            entries: std::collections::HashMap::new(),
            threshold: threshold.max(1),
            hits: 0,
        }
    }

    /// How many calls have dispatched to a compiled WASM module — a diagnostic/test hook
    /// proving the tier actually fired.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Run `program.functions[func](args)` on the WASM-JIT tier, or `None` to fall back to
    /// the bytecode tier (not yet hot, or ineligible). A `Some` result is the emitted
    /// module's output — cross-checked against the VM by the differential tests.
    pub fn on_call(&mut self, program: &CompiledProgram, func: u16, args: &[i64]) -> Option<i64> {
        self.entries.entry(func).or_insert(TierEntry::Pending(0));
        // Count the call; cross the threshold ⇒ compile + instantiate (or mark ineligible).
        if let Some(TierEntry::Pending(count)) = self.entries.get_mut(&func) {
            *count += 1;
            if *count < self.threshold {
                return None;
            }
        }
        if matches!(self.entries.get(&func), Some(TierEntry::Pending(_))) {
            match instantiate(program, func) {
                Some(m) => {
                    self.entries.insert(func, TierEntry::Ready(m));
                }
                None => {
                    self.entries.insert(func, TierEntry::Ineligible);
                    return None;
                }
            }
        }
        // Dispatch to the compiled module.
        if let Some(TierEntry::Ready(m)) = self.entries.get_mut(&func) {
            if let Some(v) = m.call(args) {
                self.hits += 1;
                return Some(v);
            }
        }
        None
    }
}

/// Lower function `func` to WASM and instantiate it through the native `wasmi` host.
#[cfg(not(target_arch = "wasm32"))]
fn instantiate(program: &CompiledProgram, func: u16) -> Option<ReadyModule> {
    let bytes = compile_function_to_wasm(program, func as usize)?;
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &bytes[..]).ok()?;
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine)
        .instantiate(&mut store, &module)
        .ok()?
        .start(&mut store)
        .ok()?;
    let f = instance.get_func(&store, "f")?;
    Some(ReadyModule { store, func: f })
}

/// Lower function `func` to WASM and instantiate it through the host's real `WebAssembly`.
#[cfg(target_arch = "wasm32")]
fn instantiate(program: &CompiledProgram, func: u16) -> Option<ReadyModule> {
    let bytes = compile_function_to_wasm(program, func as usize)?;
    Some(ReadyModule { func: instantiate_on_host(&bytes)? })
}

/// Compile + instantiate raw WASM bytes through the host's `WebAssembly` (V8 in the browser
/// / node), returning the module's exported `f`. The constructors `new WebAssembly.Module`
/// and `new WebAssembly.Instance` are synchronous (unlike `WebAssembly.instantiate`), so
/// tier-up stays a plain synchronous step inside the VM's `Op::Call`. wasm32 only.
#[cfg(target_arch = "wasm32")]
fn instantiate_on_host(bytes: &[u8]) -> Option<js_sys::Function> {
    use wasm_bindgen::JsCast;
    let arr = js_sys::Uint8Array::from(bytes);
    let module = js_sys::WebAssembly::Module::new(arr.as_ref()).ok()?;
    let instance = js_sys::WebAssembly::Instance::new(&module, &js_sys::Object::new()).ok()?;
    js_sys::Reflect::get(instance.exports().as_ref(), &wasm_bindgen::JsValue::from_str("f"))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()
}

/// Call a host `WebAssembly` export taking/returning i64. WebAssembly i64 crosses the JS
/// boundary as `BigInt`; args are marshaled in as `BigInt`s and the `BigInt` result is read
/// back losslessly via its base-10 string (no f64 round-trip). wasm32 only.
#[cfg(target_arch = "wasm32")]
fn call_host_func(f: &js_sys::Function, args: &[i64]) -> Option<i64> {
    use wasm_bindgen::JsCast;
    let arr = js_sys::Array::new();
    for &a in args {
        arr.push(&wasm_bindgen::JsValue::from(js_sys::BigInt::from(a)));
    }
    let result = f.apply(&wasm_bindgen::JsValue::NULL, &arr).ok()?;
    let bigint = result.unchecked_into::<js_sys::BigInt>();
    let decimal = wasm_bindgen::JsValue::from(bigint.to_string(10).ok()?).as_string()?;
    decimal.parse::<i64>().ok()
}

/// Instantiate raw WASM bytes through the host's real `WebAssembly` and call export `f` with
/// i64 args — the browser-native execution primitive the WS6 browser tests use to prove the
/// emitted modules run on V8 (not just wasmi). wasm32 only.
#[cfg(target_arch = "wasm32")]
pub fn run_on_host(bytes: &[u8], args: &[i64]) -> Option<i64> {
    call_host_func(&instantiate_on_host(bytes)?, args)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// Run the emitted module's `f` through the pure-Rust wasmi interpreter (instantiation
    /// also validates the bytes), returning `f(arg)` — the end-to-end codegen proof.
    fn run(module: &[u8], arg: i64) -> i64 {
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
        let mut store = wasmi::Store::new(&engine, ());
        let instance = wasmi::Linker::<()>::new(&engine)
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        let f = instance.get_typed_func::<i64, i64>(&store, "f").unwrap();
        f.call(&mut store, arg).unwrap()
    }

    #[test]
    fn wasm_jit_emits_and_runs_straight_line() {
        // f(x) = x*x + x
        let ops = vec![
            Op::Mul { dst: 1, lhs: 0, rhs: 0 },
            Op::Add { dst: 2, lhs: 1, rhs: 0 },
            Op::Return { src: 2 },
        ];
        let module = compile_region_to_wasm(&ops, &[], 1, 3).expect("emits");
        assert_eq!(run(&module, 5), 30);
        assert_eq!(run(&module, 7), 56);
    }

    #[test]
    fn wasm_jit_emits_and_runs_loop() {
        // f(n) = n + (n-1) + ... + 1   (triangular number) via a counted loop:
        //   acc = 0; one = 1; zero = 0;
        //   loop:  cond = n > zero;  if !cond goto exit;  acc += n;  n -= one;  goto loop;
        //   exit:  return acc
        // regs: 0=n, 1=acc, 2=one, 3=zero, 4=cond. consts: [0, 1].
        let ops = vec![
            Op::LoadConst { dst: 1, idx: 0 }, // acc = 0
            Op::LoadConst { dst: 2, idx: 1 }, // one = 1
            Op::LoadConst { dst: 3, idx: 0 }, // zero = 0
            Op::Gt { dst: 4, lhs: 0, rhs: 3 }, // [pc 3] cond = n > 0
            Op::JumpIfFalse { cond: 4, target: 8 }, // if !cond goto 8
            Op::Add { dst: 1, lhs: 1, rhs: 0 }, // acc += n
            Op::Sub { dst: 0, lhs: 0, rhs: 2 }, // n -= 1
            Op::Jump { target: 3 },             // goto 3
            Op::Return { src: 1 },              // [pc 8] return acc
        ];
        let consts = vec![Constant::Int(0), Constant::Int(1)];
        let module = compile_region_to_wasm(&ops, &consts, 1, 5).expect("loop region emits");
        assert_eq!(run(&module, 5), 15); // 5+4+3+2+1
        assert_eq!(run(&module, 10), 55);
        assert_eq!(run(&module, 1), 1);
        assert_eq!(run(&module, 0), 0);
        assert_eq!(run(&module, 100), 5050);
    }

    #[test]
    fn wasm_jit_emits_and_runs_branch() {
        // f(x) = if x < 10 { x * 2 } else { x + 100 }
        // regs: 0=x, 1=ten, 2=cond, 3=two, 4=result, 5=hundred. consts: [10, 2, 100].
        let ops = vec![
            Op::LoadConst { dst: 1, idx: 0 },       // ten = 10
            Op::Lt { dst: 2, lhs: 0, rhs: 1 },      // cond = x < 10
            Op::JumpIfFalse { cond: 2, target: 6 }, // if !cond goto else(6)
            Op::LoadConst { dst: 3, idx: 1 },       // two = 2
            Op::Mul { dst: 4, lhs: 0, rhs: 3 },     // result = x * 2
            Op::Return { src: 4 },
            Op::LoadConst { dst: 5, idx: 2 },       // [pc 6] hundred = 100
            Op::Add { dst: 4, lhs: 0, rhs: 5 },     // result = x + 100
            Op::Return { src: 4 },
        ];
        let consts = vec![Constant::Int(10), Constant::Int(2), Constant::Int(100)];
        let module = compile_region_to_wasm(&ops, &consts, 1, 6).expect("branch region emits");
        assert_eq!(run(&module, 3), 6);
        assert_eq!(run(&module, 9), 18);
        assert_eq!(run(&module, 10), 110);
        assert_eq!(run(&module, 50), 150);
    }

    #[test]
    fn wasm_jit_rejects_concurrency_op() {
        let ops = vec![Op::ChanRecv { dst: 1, chan: 0 }, Op::Return { src: 1 }];
        assert!(
            compile_region_to_wasm(&ops, &[], 1, 2).is_none(),
            "concurrency ops are WASM-JIT-ineligible"
        );
    }

    #[test]
    fn wasm_jit_requires_a_return() {
        let ops = vec![Op::Add { dst: 1, lhs: 0, rhs: 0 }];
        assert!(compile_region_to_wasm(&ops, &[], 1, 2).is_none());
    }

    /// `Not` / `AndEager` / `OrEager` are TYPE-OVERLOADED in the VM — `Not` is logical (`!b`)
    /// for a Bool but bitwise (`!n`) for an Int, and the Eager and/or use truthiness for
    /// non-Int operands. The WASM-JIT sees only i64 and cannot tell which, so it MUST reject
    /// these (region deopts to the VM) rather than miscompile. This locks the soundness
    /// boundary: the integer fragment stops exactly where the semantics become ambiguous.
    #[test]
    fn wasm_jit_rejects_type_ambiguous_ops() {
        for op in [
            Op::Not { dst: 1, src: 0 },
            Op::AndEager { dst: 1, lhs: 0, rhs: 0 },
            Op::OrEager { dst: 1, lhs: 0, rhs: 0 },
        ] {
            let ops = vec![op, Op::Return { src: 1 }];
            assert!(
                compile_region_to_wasm(&ops, &[], 1, 2).is_none(),
                "type-overloaded op must be WASM-JIT-ineligible (deopt, not miscompile): {:?}",
                ops[0]
            );
        }
    }
}
