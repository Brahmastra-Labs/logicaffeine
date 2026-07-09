//! Bytecode disassembler — renders a [`CompiledProgram`]'s `Op` stream as readable
//! text for the debugger (status line, bytecode tape) and any future
//! `largo --disasm`. Common ops get a clean infix form (constants resolved,
//! registers as `R{n}`, jump targets inline); the long tail falls back to the
//! `Op` `Debug` form, so a newly added op is never a panic — just a plainer line.

use super::instruction::{CompiledProgram, ConstIdx, Constant, Op, Reg};

/// One disassembled instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisasmLine {
    pub pc: usize,
    pub text: String,
    /// Absolute jump/branch targets this op can transfer control to (for the
    /// bytecode tape's arrows). Empty for straight-line ops.
    pub targets: Vec<usize>,
}

/// Disassemble every instruction in `prog.code` (Main first, then the function
/// bodies, which share the one `code` vector).
pub fn disassemble(prog: &CompiledProgram) -> Vec<DisasmLine> {
    prog.code
        .iter()
        .enumerate()
        .map(|(pc, op)| DisasmLine { pc, text: format_op(op, prog), targets: op_targets(op) })
        .collect()
}

/// Render one instruction. Registers print as `R{n}`; constants and global names
/// are resolved against `prog`.
pub fn format_op(op: &Op, prog: &CompiledProgram) -> String {
    let r = |reg: Reg| format!("R{reg}");
    let k = |idx: ConstIdx| format_const(prog.constants.get(idx as usize));
    let g = |idx: u16| {
        prog.globals
            .get(idx as usize)
            .cloned()
            .unwrap_or_else(|| format!("global#{idx}"))
    };
    match *op {
        Op::LoadConst { dst, idx } => format!("LoadConst {} = {}", r(dst), k(idx)),
        Op::Move { dst, src } => format!("Move {} = {}", r(dst), r(src)),
        Op::EnsureOwned { reg } => format!("EnsureOwned {}", r(reg)),
        Op::Add { dst, lhs, rhs } => format!("Add {} = {} + {}", r(dst), r(lhs), r(rhs)),
        Op::AddAssign { dst, src } => format!("AddAssign {} += {}", r(dst), r(src)),
        Op::Sub { dst, lhs, rhs } => format!("Sub {} = {} - {}", r(dst), r(lhs), r(rhs)),
        Op::Mul { dst, lhs, rhs } => format!("Mul {} = {} * {}", r(dst), r(lhs), r(rhs)),
        Op::Div { dst, lhs, rhs } => format!("Div {} = {} / {}", r(dst), r(lhs), r(rhs)),
        Op::FloorDiv { dst, lhs, rhs } => format!("FloorDiv {} = {} // {}", r(dst), r(lhs), r(rhs)),
        Op::Mod { dst, lhs, rhs } => format!("Mod {} = {} % {}", r(dst), r(lhs), r(rhs)),
        Op::Lt { dst, lhs, rhs } => format!("Lt {} = {} < {}", r(dst), r(lhs), r(rhs)),
        Op::Gt { dst, lhs, rhs } => format!("Gt {} = {} > {}", r(dst), r(lhs), r(rhs)),
        Op::LtEq { dst, lhs, rhs } => format!("LtEq {} = {} <= {}", r(dst), r(lhs), r(rhs)),
        Op::GtEq { dst, lhs, rhs } => format!("GtEq {} = {} >= {}", r(dst), r(lhs), r(rhs)),
        Op::Eq { dst, lhs, rhs } => format!("Eq {} = {} == {}", r(dst), r(lhs), r(rhs)),
        Op::NotEq { dst, lhs, rhs } => format!("NotEq {} = {} != {}", r(dst), r(lhs), r(rhs)),
        Op::Not { dst, src } => format!("Not {} = !{}", r(dst), r(src)),
        Op::Concat { dst, lhs, rhs } => format!("Concat {} = {} ++ {}", r(dst), r(lhs), r(rhs)),
        Op::Jump { target } => format!("Jump -> {target}"),
        Op::JumpIfFalse { cond, target } => format!("JumpIfFalse {} -> {target}", r(cond)),
        Op::JumpIfTrue { cond, target } => format!("JumpIfTrue {} -> {target}", r(cond)),
        Op::Call { dst, func, args_start, arg_count } => {
            format!("Call {} = fn#{func}({}..+{arg_count})", r(dst), r(args_start))
        }
        Op::Return { src } => format!("Return {}", r(src)),
        Op::ReturnNothing => "ReturnNothing".to_string(),
        Op::GlobalGet { dst, idx } => format!("GlobalGet {} = {}", r(dst), g(idx)),
        Op::GlobalSet { idx, src } => format!("GlobalSet {} = {}", g(idx), r(src)),
        Op::NewEmptyList { dst } => format!("NewEmptyList {}", r(dst)),
        Op::NewList { dst, start, count } => format!("NewList {} = [{}..+{count}]", r(dst), r(start)),
        Op::NewRange { dst, start, end } => format!("NewRange {} = {}..={}", r(dst), r(start), r(end)),
        Op::ListPush { list, value } => format!("ListPush {} <- {}", r(list), r(value)),
        Op::ListPop { list, dst } => format!("ListPop {} = {}.pop()", r(dst), r(list)),
        Op::Index { dst, collection, index } => format!("Index {} = {}[{}]", r(dst), r(collection), r(index)),
        Op::SetIndex { collection, index, value } => {
            format!("SetIndex {}[{}] = {}", r(collection), r(index), r(value))
        }
        Op::Length { dst, collection } => format!("Length {} = len {}", r(dst), r(collection)),
        Op::Contains { dst, collection, value } => {
            format!("Contains {} = {} contains {}", r(dst), r(collection), r(value))
        }
        Op::IterPrepare { iterable } => format!("IterPrepare {}", r(iterable)),
        Op::IterNext { dst, exit } => format!("IterNext {} (exhausted -> {exit})", r(dst)),
        Op::IterPop => "IterPop".to_string(),
        Op::Show { src } => format!("Show {}", r(src)),
        Op::FailWith { msg } => format!("FailWith {}", k(msg)),
        Op::Halt => "Halt".to_string(),
        // The long tail (CRDT, structs, temporal, concurrency, magic-div, …) is
        // rendered by its derived Debug — correct, just less pretty.
        other => format!("{other:?}"),
    }
}

/// Absolute control-transfer targets of an op (for tape arrows / reachability).
pub fn op_targets(op: &Op) -> Vec<usize> {
    match *op {
        Op::Jump { target } => vec![target],
        Op::JumpIfFalse { target, .. }
        | Op::JumpIfTrue { target, .. } => vec![target],
        Op::IterNext { exit, .. } => vec![exit],
        _ => Vec::new(),
    }
}

/// The registers an op reads (its sources) and the one it writes (its destination).
/// Drives the debugger's datapath animation — which register cells light up, flow into
/// the engine, and receive the result — and the plain-English narration. Empty for the
/// long-tail ops (no animation/narration), which fall back to the disassembly text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpIo {
    pub writes: Option<Reg>,
    pub reads: Vec<Reg>,
}

/// Compute the [`OpIo`] of a bytecode op.
pub fn op_io(op: &Op) -> OpIo {
    let rw = |dst: Reg, reads: Vec<Reg>| OpIo { writes: Some(dst), reads };
    let ro = |reads: Vec<Reg>| OpIo { writes: None, reads };
    match *op {
        Op::LoadConst { dst, .. } => rw(dst, vec![]),
        Op::Move { dst, src } => rw(dst, vec![src]),
        Op::EnsureOwned { reg } => rw(reg, vec![reg]),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::ExactDiv { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::Concat { dst, lhs, rhs }
        | Op::Pow { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs } => rw(dst, vec![lhs, rhs]),
        Op::AddAssign { dst, src } => rw(dst, vec![dst, src]),
        Op::Not { dst, src } => rw(dst, vec![src]),
        Op::Show { src } => ro(vec![src]),
        Op::Return { src } => ro(vec![src]),
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => {
            ro(vec![cond])
        }
        Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
            rw(dst, vec![collection, index])
        }
        Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
            rw(collection, vec![index, value])
        }
        Op::Length { dst, collection } => rw(dst, vec![collection]),
        Op::Contains { dst, collection, value } => rw(dst, vec![collection, value]),
        Op::ListPush { list, value } => rw(list, vec![value]),
        _ => OpIo::default(),
    }
}

/// The constant at `idx`, formatted for narration (e.g. `6`, `"hi"`, `nothing`).
pub fn format_constant(prog: &CompiledProgram, idx: ConstIdx) -> String {
    format_const(prog.constants.get(idx as usize))
}

fn format_const(c: Option<&Constant>) -> String {
    match c {
        Some(Constant::Int(n)) => n.to_string(),
        Some(Constant::Float(f)) => format!("{f}"),
        Some(Constant::Bool(b)) => b.to_string(),
        Some(Constant::Text(s)) => format!("{s:?}"),
        Some(Constant::Char(ch)) => format!("'{ch}'"),
        Some(Constant::Nothing) => "nothing".to_string(),
        Some(other) => format!("{other:?}"),
        None => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog() -> CompiledProgram {
        CompiledProgram {
            constants: vec![Constant::Int(6), Constant::Int(7), Constant::Text("hi".into())],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Mul { dst: 2, lhs: 0, rhs: 1 },
                Op::JumpIfFalse { cond: 2, target: 7 },
                Op::Show { src: 2 },
                Op::Jump { target: 7 },
                Op::LoadConst { dst: 3, idx: 2 },
                Op::Halt,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn renders_common_ops_with_resolved_operands() {
        let d = disassemble(&prog());
        assert_eq!(d[0].text, "LoadConst R0 = 6");
        assert_eq!(d[1].text, "LoadConst R1 = 7");
        assert_eq!(d[2].text, "Mul R2 = R0 * R1");
        assert_eq!(d[3].text, "JumpIfFalse R2 -> 7");
        assert_eq!(d[4].text, "Show R2");
        assert_eq!(d[5].text, "Jump -> 7");
        assert_eq!(d[6].text, "LoadConst R3 = \"hi\"");
        assert_eq!(d[7].text, "Halt");
    }

    #[test]
    fn extracts_jump_targets() {
        let d = disassemble(&prog());
        assert_eq!(d[3].targets, vec![7]); // JumpIfFalse
        assert_eq!(d[5].targets, vec![7]); // Jump
        assert!(d[2].targets.is_empty()); // straight-line Mul
    }

    #[test]
    fn op_io_identifies_reads_and_writes() {
        assert_eq!(op_io(&Op::Add { dst: 2, lhs: 0, rhs: 1 }), OpIo { writes: Some(2), reads: vec![0, 1] });
        assert_eq!(op_io(&Op::LoadConst { dst: 0, idx: 0 }), OpIo { writes: Some(0), reads: vec![] });
        assert_eq!(op_io(&Op::Show { src: 3 }), OpIo { writes: None, reads: vec![3] });
        assert_eq!(op_io(&Op::Move { dst: 1, src: 0 }), OpIo { writes: Some(1), reads: vec![0] });
    }

    #[test]
    fn pc_indices_are_sequential() {
        let d = disassemble(&prog());
        for (i, line) in d.iter().enumerate() {
            assert_eq!(line.pc, i);
        }
    }
}
