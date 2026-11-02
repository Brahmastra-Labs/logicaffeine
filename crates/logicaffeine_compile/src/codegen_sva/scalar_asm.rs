//! scalar_asm — assemble a compiled LOGOS program (the register bytecode `CompiledProgram`) into the
//! instruction words that `soft_core_scalar.v` executes, so a REAL `.lg` program can be streamed to the
//! Nexys A7 FPGA over UART and run on silicon (see scripts/run-on-nexys-scalar.sh).
//!
//! Scope: the FUNCTION-FREE scalar subset the routable core implements — the reversible/adiabatic core
//! (opcodes 0..4), LoadConst/Move, every arithmetic / bitwise / comparison op (through the proven
//! `alu.v`), the branches, and Show/Halt/FailWith. A strict WHITELIST rejects everything else (user
//! functions — their register-window calling convention needs a translation pass — globals, collections,
//! heap, structs, CRDTs, concurrency), so whatever this encoder accepts is guaranteed to run identically
//! on `run_soft_core` (the Rust model), on the RTL, and on the chip. The op BYTE is always
//! `binisa::opcode(op)` — the append-only ISA ABI a hardware decoder keys on — so the encoder can never
//! silently drift from the silicon; it only lays out the operand fields per the decoder's field usage.
//!
//! Word layout (matches `run_soft_core` / `soft_core_scalar.v`):
//!
//! ```text
//! general : [opcode:8][dst:8][a:8][b:8]     (dst=bits23:16, a=bits15:8, b=bits7:0)
//! LoadConst: [opcode:8][dst:8][imm:16]       (imm sign-extends the low 16 bits)
//! Jump    : [opcode:8][target:8][........]   (target = bits23:16, a 0..255 word index)
//! JumpIf* : [opcode:8][target:8][cond:8][..] (target=bits23:16, cond=bits15:8)
//! Show    : [opcode:8][0][src:8][0]          (src=bits15:8)
//! ```
//!
//! Locked ≡ VM by `rtl.rs::tests::encode_scalar_matches_vm`.

use crate::vm::binisa::opcode;
use crate::vm::{CompiledProgram, Constant, Op, Reg};

/// A register index must fit the 16-entry file (r0..r15).
fn reg(r: Reg) -> Result<u32, String> {
    if r > 15 {
        return Err(format!(
            "register r{r} exceeds the scalar core's 16-register file"
        ));
    }
    Ok(r as u32)
}

/// Encode one `Op` into its soft_core_scalar word, given the program's constant pool. Returns `Err` for
/// any op outside the chip's function-free scalar subset (the caller then knows the program isn't
/// chip-runnable as-is).
fn encode_op(op: &Op, constants: &[Constant]) -> Result<u32, String> {
    let oc = opcode(op) as u32;
    // [op][dst][a][b]
    let w = |dst: u32, a: u32, b: u32| (oc << 24) | (dst << 16) | (a << 8) | b;
    Ok(match op {
        // reversible / adiabatic core
        Op::RevAdd { place, delta } | Op::RevSub { place, delta } | Op::RevXor { place, delta } => {
            w(reg(*place)?, reg(*delta)?, 0)
        }
        Op::RevNeg { place } => w(reg(*place)?, 0, 0),
        Op::RevSwap { a, b } => w(reg(*a)?, reg(*b)?, 0),
        // data movement
        Op::Move { dst, src } => w(reg(*dst)?, reg(*src)?, 0),
        Op::LoadConst { dst, idx } => {
            let val = match constants.get(*idx as usize) {
                Some(Constant::Int(v)) => *v,
                Some(Constant::Bool(b)) => *b as i64,
                Some(other) => {
                    return Err(format!("LoadConst of a non-integer constant ({other:?}) is not chip-runnable"))
                }
                None => return Err(format!("LoadConst references missing constant #{idx}")),
            };
            // The scalar core's LoadConst SIGN-extends its 16-bit immediate, so the representable set is the
            // signed-16-bit range. A value in [32768, 65535] (or beyond) needs a wider load the scalar core does
            // not have — reject it (an honest boundary) rather than silently truncate its high bits.
            if !(-32768..=32767).contains(&val) {
                return Err(format!("constant {val} exceeds the signed 16-bit LoadConst immediate range [-32768, 32767]"));
            }
            (oc << 24) | (reg(*dst)? << 16) | (((val as i32) as u32) & 0xffff)
        }
        // Division / modulo / power are NOT in the scalar core's combinational alu.v — the full core issues
        // them to a SEQUENTIAL shared divider the scalar core omits (they would read as 0 on silicon). Reject
        // with an honest reason instead of emitting a word that silently computes the wrong answer.
        Op::Div { .. }
        | Op::Mod { .. }
        | Op::ExactDiv { .. }
        | Op::FloorDiv { .. }
        | Op::DivPow2 { .. }
        | Op::MagicDivU { .. }
        | Op::Pow { .. } => {
            return Err(format!(
                "`{}` needs the sequential divider the scalar core omits — only the combinational alu.v ops \
                 (add/sub/mul, bitwise, shifts, comparisons) run on the chip",
                op.name()
            ))
        }
        // arithmetic / bitwise / comparison — every one goes through the proven alu.v (dst, lhs->a, rhs->b)
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs } => w(reg(*dst)?, reg(*lhs)?, reg(*rhs)?),
        Op::Not { dst, src } => w(reg(*dst)?, reg(*src)?, 0),
        // `Set x to x + y` compiles to AddAssign (dst = dst + src) — an in-place-Text optimization that is
        // plain integer addition for scalars. Lower it to the proven `Add` datapath (dst, lhs=dst, rhs=src)
        // so it uses exactly the path sum_loop already validated on silicon.
        Op::AddAssign { dst, src } => {
            let add = crate::vm::binisa::opcode_of_name("Add").expect("Add is a registered opcode") as u32;
            (add << 24) | (reg(*dst)? << 16) | (reg(*dst)? << 8) | reg(*src)?
        }
        // control flow — the target is an absolute instruction index; for a function-free program that
        // index is the imem word index (the code array maps 1:1), so it lowers directly.
        Op::Jump { target } => {
            if *target > 255 {
                return Err(format!("jump target {target} exceeds the 256-word imem"));
            }
            (oc << 24) | ((*target as u32) << 16)
        }
        Op::JumpIfFalse { cond, target } | Op::JumpIfTrue { cond, target } => {
            if *target > 255 {
                return Err(format!("jump target {target} exceeds the 256-word imem"));
            }
            (oc << 24) | ((*target as u32) << 16) | (reg(*cond)? << 8)
        }
        // runtime
        Op::Show { src } => w(0, reg(*src)?, 0),
        Op::Halt | Op::FailWith { .. } => oc << 24,
        // everything else (functions, globals, collections, heap, structs, CRDTs, concurrency, closures)
        // is outside the routable scalar core — reject so the encoder's output is always chip-faithful.
        other => return Err(format!("`{}` is outside the function-free scalar chip subset", other.name())),
    })
}

/// The registers a scalar-subset op reads and writes — `(reads, writes)` — or `None` outside the subset.
/// The arms are exactly `encode_op`'s accepted ops.
fn scalar_io(op: &Op) -> Option<(Vec<Reg>, Vec<Reg>)> {
    Some(match *op {
        Op::RevAdd { place, delta } | Op::RevSub { place, delta } | Op::RevXor { place, delta } => {
            (vec![place, delta], vec![place])
        }
        Op::RevNeg { place } => (vec![place], vec![place]),
        Op::RevSwap { a, b } => (vec![a, b], vec![a, b]),
        Op::Move { dst, src } | Op::Not { dst, src } => (vec![src], vec![dst]),
        Op::LoadConst { dst, .. } => (vec![], vec![dst]),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs } => (vec![lhs, rhs], vec![dst]),
        Op::AddAssign { dst, src } => (vec![dst, src], vec![dst]),
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => (vec![cond], vec![]),
        Op::Show { src } => (vec![src], vec![]),
        Op::Jump { .. } | Op::Halt | Op::FailWith { .. } => (vec![], vec![]),
        _ => return None,
    })
}

/// `op` with every register `r` it names replaced by `to(r)`. Only called on scalar-subset ops.
fn rename(op: &Op, to: &dyn Fn(Reg) -> Reg) -> Op {
    match *op {
        Op::RevAdd { place, delta } => Op::RevAdd {
            place: to(place),
            delta: to(delta),
        },
        Op::RevSub { place, delta } => Op::RevSub {
            place: to(place),
            delta: to(delta),
        },
        Op::RevXor { place, delta } => Op::RevXor {
            place: to(place),
            delta: to(delta),
        },
        Op::RevNeg { place } => Op::RevNeg { place: to(place) },
        Op::RevSwap { a, b } => Op::RevSwap { a: to(a), b: to(b) },
        Op::Move { dst, src } => Op::Move {
            dst: to(dst),
            src: to(src),
        },
        Op::Not { dst, src } => Op::Not {
            dst: to(dst),
            src: to(src),
        },
        Op::LoadConst { dst, idx } => Op::LoadConst { dst: to(dst), idx },
        Op::Add { dst, lhs, rhs } => Op::Add {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Sub { dst, lhs, rhs } => Op::Sub {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Mul { dst, lhs, rhs } => Op::Mul {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Lt { dst, lhs, rhs } => Op::Lt {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Gt { dst, lhs, rhs } => Op::Gt {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::LtEq { dst, lhs, rhs } => Op::LtEq {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::GtEq { dst, lhs, rhs } => Op::GtEq {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Eq { dst, lhs, rhs } => Op::Eq {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::NotEq { dst, lhs, rhs } => Op::NotEq {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::BitXor { dst, lhs, rhs } => Op::BitXor {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::BitAnd { dst, lhs, rhs } => Op::BitAnd {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::BitOr { dst, lhs, rhs } => Op::BitOr {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Shl { dst, lhs, rhs } => Op::Shl {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::Shr { dst, lhs, rhs } => Op::Shr {
            dst: to(dst),
            lhs: to(lhs),
            rhs: to(rhs),
        },
        Op::AddAssign { dst, src } => Op::AddAssign {
            dst: to(dst),
            src: to(src),
        },
        Op::JumpIfFalse { cond, target } => Op::JumpIfFalse {
            cond: to(cond),
            target,
        },
        Op::JumpIfTrue { cond, target } => Op::JumpIfTrue {
            cond: to(cond),
            target,
        },
        Op::Show { src } => Op::Show { src: to(src) },
        other => other,
    }
}

/// The scalar core's register file.
const SCALAR_REGISTERS: usize = 16;

/// Map a function-free scalar program's registers onto the core's sixteen by liveness. The VM numbers
/// registers freely (a fresh one per `Let`, named values above the temporaries that computed them); the
/// chip needs only as many as are ever live at once. Two registers may share a physical one unless one
/// is live where the other is written (a `Move`'s source does not count against its destination), and
/// the interference graph is coloured Chaitin–Briggs style — simplify below sixteen neighbours, else
/// optimistically, then colour in reverse — lowest register first, so the result is deterministic.
///
/// A program already within sixteen registers is returned unchanged, so an image that fits keeps its
/// encoding exactly. `Err` names the instruction where more than sixteen values are live at once; a
/// program with an op outside the scalar subset is also returned unchanged, for the encoder to refuse
/// with that op's own reason.
pub fn allocate_scalar_registers(code: &[Op]) -> Result<Vec<Op>, String> {
    let io: Option<Vec<(Vec<Reg>, Vec<Reg>)>> = code.iter().map(scalar_io).collect();
    let Some(io) = io else {
        return Ok(code.to_vec());
    };
    let used: std::collections::BTreeSet<Reg> = io
        .iter()
        .flat_map(|(r, w)| r.iter().chain(w))
        .copied()
        .collect();
    if used.iter().all(|&r| (r as usize) < SCALAR_REGISTERS) {
        return Ok(code.to_vec());
    }
    let n = code.len();
    let successors = |i: usize| -> Vec<usize> {
        match code[i] {
            Op::Jump { target } => vec![target],
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => vec![target, i + 1],
            Op::Halt | Op::FailWith { .. } => vec![],
            _ => vec![i + 1],
        }
        .into_iter()
        .filter(|&s| s < n)
        .collect()
    };
    use std::collections::{BTreeMap, BTreeSet};
    let mut live_in: Vec<BTreeSet<Reg>> = vec![BTreeSet::new(); n];
    let mut live_out: Vec<BTreeSet<Reg>> = vec![BTreeSet::new(); n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let out: BTreeSet<Reg> = successors(i)
                .into_iter()
                .flat_map(|s| live_in[s].iter().copied())
                .collect();
            let (reads, writes) = &io[i];
            let mut inn: BTreeSet<Reg> = out
                .iter()
                .copied()
                .filter(|r| !writes.contains(r))
                .collect();
            inn.extend(reads.iter().copied());
            if out != live_out[i] || inn != live_in[i] {
                live_out[i] = out;
                live_in[i] = inn;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut adj: BTreeMap<Reg, BTreeSet<Reg>> =
        used.iter().map(|&r| (r, BTreeSet::new())).collect();
    let mut edge = |a: Reg, b: Reg| {
        if a != b {
            adj.get_mut(&a).unwrap().insert(b);
            adj.get_mut(&b).unwrap().insert(a);
        }
    };
    for i in 0..n {
        let (_, writes) = &io[i];
        let moved_from = match code[i] {
            Op::Move { src, .. } => Some(src),
            _ => None,
        };
        for &w in writes {
            for &l in &live_out[i] {
                if Some(l) != moved_from {
                    edge(w, l);
                }
            }
            for &w2 in writes {
                edge(w, w2);
            }
        }
    }
    for a in live_in[0].iter().copied().collect::<Vec<_>>() {
        for b in live_in[0].iter().copied() {
            edge(a, b);
        }
    }
    let crowded = || {
        (0..n)
            .map(|i| {
                (
                    live_out[i]
                        .iter()
                        .chain(&io[i].1)
                        .collect::<BTreeSet<_>>()
                        .len(),
                    i,
                )
            })
            .max()
            .unwrap_or((0, 0))
    };
    let mut remaining: BTreeSet<Reg> = used.clone();
    let mut stack = Vec::with_capacity(used.len());
    while !remaining.is_empty() {
        let degree = |r: Reg| adj[&r].iter().filter(|x| remaining.contains(x)).count();
        let pick = remaining
            .iter()
            .copied()
            .find(|&r| degree(r) < SCALAR_REGISTERS)
            .unwrap_or_else(|| {
                remaining
                    .iter()
                    .copied()
                    .max_by_key(|&r| (degree(r), std::cmp::Reverse(r)))
                    .unwrap()
            });
        remaining.remove(&pick);
        stack.push(pick);
    }
    let mut color: BTreeMap<Reg, Reg> = BTreeMap::new();
    while let Some(r) = stack.pop() {
        let taken: BTreeSet<Reg> = adj[&r]
            .iter()
            .filter_map(|x| color.get(x).copied())
            .collect();
        let Some(c) = (0..SCALAR_REGISTERS as Reg).find(|c| !taken.contains(c)) else {
            let (live, at) = crowded();
            return Err(format!(
                "{live} values are live at once at instruction {at}, more than the scalar core's \
                 {SCALAR_REGISTERS} registers"
            ));
        };
        color.insert(r, c);
    }
    Ok(code.iter().map(|op| rename(op, &|r| color[&r])).collect())
}

/// Assemble a whole compiled program into soft_core_scalar imem words. Function-free only (a program with
/// user functions returns `Err`; their calling-convention translation is a separate pass). At most 256
/// words (the imem depth). Registers are first mapped onto the core's sixteen by liveness
/// ([`allocate_scalar_registers`]).
pub fn encode_soft_core_scalar(program: &CompiledProgram) -> Result<Vec<u32>, String> {
    if !program.functions.is_empty() {
        return Err("program defines user functions — the scalar encoder is function-free (calling-convention translation not yet implemented)".into());
    }
    if program.code.len() > 256 {
        return Err(format!(
            "{} instructions exceed the 256-word scalar imem",
            program.code.len()
        ));
    }
    allocate_scalar_registers(&program.code)?
        .iter()
        .enumerate()
        .map(|(i, op)| {
            encode_op(op, &program.constants).map_err(|e| format!("instruction {i}: {e}"))
        })
        .collect()
}

/// Render imem words as a `.hex` file body (one 8-hex-digit word per line) for scripts/nexys-host.py.
pub fn to_hex(words: &[u32]) -> String {
    let mut s = String::with_capacity(words.len() * 9);
    for w in words {
        s.push_str(&format!("{w:08x}\n"));
    }
    s
}

/// Compile a `.lg` source string all the way to soft_core_scalar imem words, through the SAME front-end
/// and oracle-driven compiler the VM uses (so the chip runs byte-identically to `vm_outcome`). `Err` if it
/// doesn't parse/compile, or uses anything outside the function-free scalar subset.
pub fn compile_to_scalar_words(source: &str) -> Result<Vec<u32>, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed.map_err(|advice| format!("{advice}"))?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program =
            crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))?;
        encode_soft_core_scalar(&program)
    })
}
