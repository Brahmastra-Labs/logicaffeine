//! A relocatable, self-contained per-function bytecode unit (HOTSWAP §7).
//!
//! A [`CompiledProgram`] holds every function body in ONE flat `code` vector with
//! ABSOLUTE jump targets; `entry_pc` marks where each begins. [`slice_function`]
//! lifts one function out of that vector into a [`FnBytecode`] whose jump targets are
//! 0-relative to its own first op, carrying the frame/param/return metadata needed to
//! run or compile it independently of the program it came from.
//!
//! This is the producer the Axis-1 hot-swap consumes: the WASM-portable warm-bytecode
//! side-table (P11) installs an `FnBytecode` per function, and the OPFS tier cache
//! (P12) serializes it. The native path (P10) feeds the same body to forge (which
//! rebases by `entry_pc` itself, so it takes the program slice directly).

#[allow(unused_imports)]
use super::instruction::{CompiledProgram, Constant, Op};
use super::native_tier::{ParamKind, SlotKind};

/// One function lifted out of a [`CompiledProgram`]: a self-contained, relocatable
/// body with 0-relative jump targets plus the metadata to execute or compile it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)] // consumed by P10 (native swap), P11 (warm side-table), P12 (cache)
pub struct FnBytecode {
    /// The function body, with every jump target rebased to be relative to `code[0]`.
    pub code: Vec<Op>,
    /// The constant pool the code indexes into — the program's pool, copied so the
    /// unit is self-contained; constant indices are preserved unchanged.
    pub constants: Vec<Constant>,
    pub register_count: usize,
    pub param_count: u16,
    pub param_kinds: Vec<Option<ParamKind>>,
    pub ret_kind: Option<SlotKind>,
    /// Which frame registers carry a user-visible name (the region JIT's
    /// observability map; preserved from the source function).
    pub named_regs: Vec<bool>,
}

impl FnBytecode {
    /// Structural self-consistency check (HOTSWAP §P12/P11 robustness). Every jump target
    /// must land inside the body and the body must end in a control-leaving op, so an
    /// installed body can neither fetch past the warm buffer (panic) nor fall through into
    /// the next installed body (silent miscompile). A `slice_function` body always passes;
    /// this rejects a corrupt / foreign / mis-decoded body so the VM declines the install
    /// and falls back to baseline instead of trusting it. `n_funcs` bounds any `Call`
    /// target so a body can't dispatch to a non-existent function index.
    pub fn is_well_formed(&self, n_funcs: usize) -> bool {
        if self.code.is_empty() {
            return false;
        }
        for op in &self.code {
            match *op {
                Op::Jump { target }
                | Op::JumpIfFalse { target, .. }
                | Op::JumpIfTrue { target, .. }
                | Op::JumpIfInt { target, .. } => {
                    if target >= self.code.len() {
                        return false;
                    }
                }
                Op::Call { func, .. } => {
                    if func as usize >= n_funcs {
                        return false;
                    }
                }
                _ => {}
            }
        }
        matches!(
            self.code.last(),
            Some(Op::Return { .. } | Op::ReturnNothing | Op::Halt)
        )
    }
}

/// Shift one op's jump target by `delta` (signed). Only the four control-flow ops
/// carry an absolute pc target; the `target: Reg` ops (`TestArm`/`BindArm`/…) name a
/// register, not a pc, and are left untouched.
pub(crate) fn rebase(op: Op, delta: isize) -> Op {
    let shift = |t: usize| (t as isize + delta) as usize;
    match op {
        Op::Jump { target } => Op::Jump { target: shift(target) },
        Op::JumpIfFalse { cond, target } => Op::JumpIfFalse { cond, target: shift(target) },
        Op::JumpIfTrue { cond, target } => Op::JumpIfTrue { cond, target: shift(target) },
        Op::JumpIfInt { cond, target } => Op::JumpIfInt { cond, target: shift(target) },
        other => other,
    }
}

/// Lift function `fi` out of `program` into a self-contained [`FnBytecode`] with
/// 0-relative jumps. The body is `program.code[entry_pc..end]` (where `end` is the
/// next function's entry, or the end of code); its absolute jump targets are rebased
/// by `-entry_pc`, so they index into the returned `code` directly.
#[allow(dead_code)]
pub fn slice_function(program: &CompiledProgram, fi: usize) -> FnBytecode {
    let f = &program.functions[fi];
    let entry = f.entry_pc;
    let end = program
        .functions
        .iter()
        .map(|g| g.entry_pc)
        .filter(|&e| e > entry)
        .min()
        .unwrap_or(program.code.len());
    let code = program.code[entry..end]
        .iter()
        .map(|&op| rebase(op, -(entry as isize)))
        .collect();
    FnBytecode {
        code,
        constants: program.constants.clone(),
        register_count: f.register_count,
        param_count: f.param_count,
        param_kinds: f.param_kinds.clone(),
        ret_kind: f.ret_kind,
        named_regs: f.named_regs.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;
    use crate::vm::instruction::CompiledFunction;

    fn jump_target(op: &Op) -> Option<usize> {
        match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => Some(target),
            _ => None,
        }
    }

    #[test]
    fn slice_rebases_jumps_zero_relative_and_round_trips() {
        let mut it = Interner::new();
        let name = it.intern("f");
        // Main = [Halt]@0 ; function `f` @1 = a self-loop with a forward exit:
        //   @1 JumpIfFalse cond0 -> abs 3   (exit)      -> rel target 2
        //   @2 Jump        -> abs 1          (loop back) -> rel target 0
        //   @3 Halt
        let code = vec![
            Op::Halt,
            Op::JumpIfFalse { cond: 0, target: 3 },
            Op::Jump { target: 1 },
            Op::Halt,
        ];
        let prog = CompiledProgram {
            code,
            functions: vec![CompiledFunction {
                name,
                entry_pc: 1,
                param_count: 0,
                register_count: 1,
                captures: vec![],
                named_regs: vec![true],
                param_kinds: vec![],
                ret_kind: None,
            }],
            ..Default::default()
        };

        let fnbc = slice_function(&prog, 0);
        assert_eq!(fnbc.code.len(), 3, "body is entry..next-entry");

        // Jumps are now 0-relative to the slice.
        assert!(matches!(fnbc.code[0], Op::JumpIfFalse { target: 2, .. }));
        assert!(matches!(fnbc.code[1], Op::Jump { target: 0 }));

        // 0-relative validity: every jump target lands inside the slice.
        for op in &fnbc.code {
            if let Some(t) = jump_target(op) {
                assert!(t < fnbc.code.len(), "jump target {t} out of slice bounds");
            }
        }

        // Round-trip: re-absolutizing by +entry_pc reproduces the original span
        // exactly (Op has no PartialEq, so compare its Debug form).
        let reabs: Vec<String> = fnbc.code.iter().map(|&op| format!("{:?}", rebase(op, 1))).collect();
        let orig: Vec<String> = prog.code[1..4].iter().map(|op| format!("{:?}", op)).collect();
        assert_eq!(reabs, orig, "slice must invert back to the original body");

        // Metadata is carried through.
        assert_eq!(fnbc.register_count, 1);
        assert_eq!(fnbc.named_regs, vec![true]);
    }

    fn body(code: Vec<Op>) -> FnBytecode {
        FnBytecode {
            code,
            constants: vec![],
            register_count: 1,
            param_count: 0,
            param_kinds: vec![],
            ret_kind: None,
            named_regs: vec![],
        }
    }

    #[test]
    fn is_well_formed_accepts_a_valid_body() {
        // JumpIfFalse exits forward, Jump loops back, ends in ReturnNothing — all in range.
        let good = body(vec![
            Op::JumpIfFalse { cond: 0, target: 2 },
            Op::Jump { target: 0 },
            Op::ReturnNothing,
        ]);
        assert!(good.is_well_formed(1));
    }

    #[test]
    fn is_well_formed_rejects_malformed_bodies() {
        // Empty body.
        assert!(!body(vec![]).is_well_formed(1));
        // No terminal op (would fall through into the next warm body / past the buffer).
        assert!(!body(vec![Op::Jump { target: 0 }]).is_well_formed(1));
        // Out-of-range jump target.
        assert!(!body(vec![Op::Jump { target: 99 }, Op::ReturnNothing]).is_well_formed(1));
        // Call to a non-existent function index (func 5 ≥ n_funcs 1).
        assert!(!body(vec![
            Op::Call { dst: 0, func: 5, args_start: 0, arg_count: 0 },
            Op::ReturnNothing,
        ])
        .is_well_formed(1));
        // Same call IS in range when there are enough functions.
        assert!(body(vec![
            Op::Call { dst: 0, func: 5, args_start: 0, arg_count: 0 },
            Op::ReturnNothing,
        ])
        .is_well_formed(6));
    }
}
