//! Proofs that the frame-form integer stencils meet their specs for every input.
//!
//! [`prove_stencil`] decodes a stencil's machine code with [`crate::x86`] and executes every
//! path through it symbolically over Z3 bitvectors. The sixteen general registers hold BV64
//! terms; the frame is an `Array(BV64 → BV64)` read and written through `[rdi + slot*8]`;
//! each constant hole is a free BV64, so the operand and destination slot indices are
//! symbolic and may alias. A conditional jump forks the path on its condition. The
//! frame-form lowering binds hole 0 to the left operand's slot, hole 1 to the right
//! operand's, hole 2 to the destination's, continuation 0 to the next op and continuation 1
//! to the deopt terminal; [`check_lowering`] confirms the JIT binds them so.
//!
//! With `a = frame[hole0]`, `b = frame[hole1]` and the spec's `pre`/`result`, the theorem is
//! discharged by refutation, one obligation at a time:
//!
//! - every path ends in a jump through continuation 0 or 1, and the paths' conditions
//!   cover every input;
//! - a path to continuation 1 is taken only when `pre(a, b)` fails, and leaves the frame
//!   as it found it;
//! - a path to continuation 0 is taken only when `pre(a, b)` holds, and leaves
//!   `store(frame, hole2, result(a, b))`;
//! - no `div` or `idiv` on any path raises #DE;
//! - `rdi`, `rsi`, `rsp`, the pinned `rdx`/`rcx`/`r8`/`r9` and the callee-saved `rbx`,
//!   `rbp`, `r12`–`r15` hold their entry values at the exit jump.
//!
//! Since the paths cover every input and each ends in one of the two continuations, the
//! side exit is taken exactly when the precondition fails.
//!
//! # The machine model
//!
//! Flags OF, SF, ZF and CF follow the Intel manual for each instruction that writes them; a
//! flag the manual leaves undefined becomes a fresh unconstrained Boolean, so nothing can be
//! proved from it. PF and AF are not modelled, and the decoder rejects every instruction
//! that reads them. A shift count is masked to six bits, and a masked count of zero leaves
//! the flags alone. `imul` sets CF and OF when the double-width product differs from the
//! sign extension of the truncated one, built exactly as [`crate::spec`] builds the `mul`
//! precondition, so the two terms coincide and Z3 never bit-blasts the 128-bit product.
//!
//! A 32-bit `div`/`idiv` divides the 64-bit `edx:eax` exactly, faulting on a zero divisor
//! or a quotient that does not fit. A 64-bit one is modelled only when `rdx` holds the
//! zero (`div`) or sign (`idiv`) extension of `rax`; that is part of its no-fault obligation,
//! so a dividend outside the model fails the proof rather than being misread.
//!
//! A register holds a data word, the frame pointer (`rdi` on entry), or a hole's address
//! (loaded from its GOT slot); `xor r, r` and `sub r, r` zero it whichever it holds. The only addresses are a GOT slot (`[rip + disp]` under a
//! `GotRel32` relocation), a constant hole's literal (`[reg]` holding its address) and a
//! frame slot (`[frame + index*8 + 8k]`). Any other address, arithmetic on a pointer, a
//! store outside the frame, a jump out of the stencil or a loop is
//! [`ProofError::Unmodelled`]. Slot indices are BV64 terms, which assumes a frame smaller
//! than 2^61 slots, as every frame is.
//!
//! # Narrowing
//!
//! Bit-blasting does not equate a 32-bit unsigned divider with a 64-bit signed one in useful
//! time: the division stencils' 32-bit fast path runs past fifteen minutes. On a path that
//! executed a 32-bit divide, each operand `t` that the path condition confines to 32 bits
//! (`π ⇒ t >> 32 = 0`, itself proved) is replaced by the zero extension of a fresh 32-bit
//! variable before the obligation is checked. That is sound: under `π` the operand equals
//! its replacement, and the replaced obligation is proved for every value of the fresh
//! variable. Both dividers then reduce to one 64-bit unsigned divide of the same terms.

use std::collections::BTreeMap;
use std::fmt;

use logicaffeine_forge::jit::{compile_straightline, MicroOp};
use logicaffeine_forge::{HoleId, Reloc, RelocKind, Stencil};
use z3::ast::{Array, Ast, Bool, BV};
use z3::{SatResult, Solver, Sort};

use crate::spec::OpSpec;
use crate::x86::{
    self, AluOp, Base, Cond, DecodeError, Gpr, Insn, Mem, Op, Operand, ShiftCount, ShiftOp, Width,
};

/// The registers whose entry values must survive to the exit jump: the frame pointer
/// `rdi`, the operand-stack pointer `rsi`, `rsp`, the pinned `rdx`/`rcx`/`r8`/`r9`, and the
/// callee-saved `rbx`, `rbp`, `r12`–`r15`.
pub const PRESERVED: [Gpr; 13] = [
    Gpr::RDI,
    Gpr::RSI,
    Gpr::RSP,
    Gpr::RDX,
    Gpr::RCX,
    Gpr::R8,
    Gpr::R9,
    Gpr::RBX,
    Gpr::RBP,
    Gpr::R12,
    Gpr::R13,
    Gpr::R14,
    Gpr::R15,
];

const MAX_PATHS: usize = 64;

/// A proved stencil, with the shape of its proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StencilProof {
    pub op: &'static str,
    pub stencil: String,
    pub instructions: usize,
    /// Paths ending in continuation 0.
    pub continue_paths: usize,
    /// Paths ending in continuation 1, the side exit.
    pub exit_paths: usize,
    /// `div`/`idiv` executions across all paths.
    pub divisions: usize,
    /// Paths proved with narrowed operands.
    pub narrowed_paths: usize,
    /// Obligations discharged.
    pub obligations: usize,
}

impl StencilProof {
    pub fn paths(&self) -> usize {
        self.continue_paths + self.exit_paths
    }
}

/// Why a stencil is not proved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofError {
    /// The spec's micro-op has no frame-form stencil.
    NoStencil { op: String },
    /// The bytes leave the decoder's subset.
    Decode { stencil: String, error: DecodeError },
    /// The bytes decode but do something the machine model does not cover.
    Unmodelled {
        stencil: String,
        offset: usize,
        reason: String,
    },
    /// Z3 found an input that breaks an obligation.
    Refuted {
        stencil: String,
        obligation: String,
        counterexample: String,
    },
    /// Z3 could not decide an obligation.
    Undecided {
        stencil: String,
        obligation: String,
        reason: String,
    },
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProofError::NoStencil { op } => write!(f, "{op} has no frame-form stencil"),
            ProofError::Decode { stencil, error } => write!(f, "{stencil}: {error}"),
            ProofError::Unmodelled {
                stencil,
                offset,
                reason,
            } => write!(f, "{stencil} +{offset:#x}: {reason}"),
            ProofError::Refuted {
                stencil,
                obligation,
                counterexample,
            } => write!(
                f,
                "{stencil}: refuted — {obligation}; counterexample {counterexample}"
            ),
            ProofError::Undecided {
                stencil,
                obligation,
                reason,
            } => write!(f, "{stencil}: undecided — {obligation} ({reason})"),
        }
    }
}

impl std::error::Error for ProofError {}

/// The frame-form stencil the JIT lowers `spec`'s micro-op to.
pub fn stencil_for(spec: &OpSpec) -> Result<&'static Stencil, ProofError> {
    use logicaffeine_forge as forge;
    Ok(match (spec.build)() {
        MicroOp::Add { .. } => &forge::ST_ADD3,
        MicroOp::Sub { .. } => &forge::ST_SUB3,
        MicroOp::Mul { .. } => &forge::ST_MUL3,
        MicroOp::Div { .. } => &forge::ST_DIV3C,
        MicroOp::Mod { .. } => &forge::ST_MOD3C,
        MicroOp::BitAnd { .. } => &forge::ST_AND3,
        MicroOp::BitOr { .. } => &forge::ST_OR3,
        MicroOp::BitXor { .. } => &forge::ST_XOR3,
        MicroOp::Shl { .. } => &forge::ST_SHL3,
        MicroOp::Shr { .. } => &forge::ST_SHR3,
        MicroOp::Lt { .. } => &forge::ST_LT3,
        MicroOp::LtEq { .. } => &forge::ST_LE3,
        MicroOp::Eq { .. } => &forge::ST_EQ3,
        other => {
            return Err(ProofError::NoStencil {
                op: format!("{other:?}"),
            })
        }
    })
}

/// Prove the stencil the JIT lowers `spec`'s micro-op to.
pub fn prove_op(spec: &OpSpec) -> Result<StencilProof, ProofError> {
    prove_stencil(spec, stencil_for(spec)?)
}

/// Prove `stencil` meets `spec`.
pub fn prove_stencil(spec: &OpSpec, stencil: &Stencil) -> Result<StencilProof, ProofError> {
    prove_code(spec, stencil.name, stencil.code, stencil.relocs)
}

/// Prove the machine code `code`, with its relocations, meets `spec`. `name` labels errors.
pub fn prove_code(
    spec: &OpSpec,
    name: &str,
    code: &[u8],
    relocs: &[Reloc],
) -> Result<StencilProof, ProofError> {
    let program = Program::new(name, code, relocs)?;
    let init = State::entry();
    let mut obligations = Vec::new();
    let exits = program.explore(init.clone(), &mut obligations)?;
    let divisions = obligations.len();

    let frame0 = init.frame.clone();
    let lhs = hole_value(0);
    let rhs = hole_value(1);
    let dst = hole_value(2);
    let a = select(&frame0, &lhs);
    let b = select(&frame0, &rhs);
    let pre = (spec.pre)(&a, &b);
    let result = (spec.result)(&a, &b);

    let mut continue_paths = 0;
    let mut exit_paths = 0;
    let mut narrowed_paths = 0;
    let mut covers = Vec::with_capacity(exits.len());
    for (i, exit) in exits.iter().enumerate() {
        let st = &exit.state;
        let at = exit.at;
        let ob = |label: String, goal: Bool| Obligation {
            label,
            path: st.path.clone(),
            goal,
            exact: None,
            narrow: st.narrow,
        };
        let frame_ob = |label: String, expected: &[(BV, BV)]| {
            let expected_frame = expected
                .iter()
                .fold(frame0.clone(), |f, (index, value)| f.store(index, value));
            let exact = st.frame.eq(&expected_frame);
            let (goal, exact) = if st.stores.len() == expected.len() {
                let entries: Vec<Bool> = st
                    .stores
                    .iter()
                    .zip(expected)
                    .map(|((i, v), (ei, ev))| Bool::and(&[&i.eq(ei), &v.eq(ev)]))
                    .collect();
                (Bool::and(&entries), Some(exact))
            } else {
                (exact, None)
            };
            Obligation {
                label,
                path: st.path.clone(),
                goal,
                exact,
                narrow: st.narrow,
            }
        };
        covers.push(Bool::and(&st.path));
        narrowed_paths += usize::from(st.narrow);
        match exit.cont {
            1 => {
                exit_paths += 1;
                obligations.push(ob(
                    format!("path {i} side-exits at +{at:#x} only when the precondition fails"),
                    pre.not(),
                ));
                obligations.push(frame_ob(
                    format!("path {i} side-exits at +{at:#x} with the frame unchanged"),
                    &[],
                ));
            }
            0 => {
                continue_paths += 1;
                obligations.push(ob(
                    format!("path {i} continues at +{at:#x} only when the precondition holds"),
                    pre.clone(),
                ));
                obligations.push(frame_ob(
                    format!(
                        "path {i} continues at +{at:#x} with frame[dst] = result and every other slot unchanged"
                    ),
                    &[(dst.clone(), result.clone())],
                ));
            }
            n => {
                return Err(program.unmodelled(
                    at,
                    format!("path {i} jumps through continuation {n}, which the frame-form lowering does not wire"),
                ))
            }
        }
        for r in PRESERVED {
            let goal = match (
                &st.regs[r.number() as usize],
                &init.regs[r.number() as usize],
            ) {
                (Value::Frame, Value::Frame) => continue,
                (Value::Word(now), Value::Word(entry)) => now.eq(entry),
                _ => {
                    return Err(ProofError::Refuted {
                        stencil: program.name.clone(),
                        obligation: format!(
                            "path {i} exits at +{at:#x} with {} holding its entry value",
                            r.name(Width::Qword)
                        ),
                        counterexample: format!(
                            "every input: {} holds a different kind of value",
                            r.name(Width::Qword)
                        ),
                    })
                }
            };
            obligations.push(ob(
                format!(
                    "path {i} exits at +{at:#x} with {} holding its entry value",
                    r.name(Width::Qword)
                ),
                goal,
            ));
        }
    }
    obligations.push(Obligation {
        label: "the paths cover every input".to_string(),
        path: Vec::new(),
        goal: Bool::or(&covers),
        exact: None,
        narrow: false,
    });

    let operands = [("a", a), ("b", b)];
    let named = [("lhs slot", lhs), ("rhs slot", rhs), ("dst slot", dst)];
    for ob in &obligations {
        program.discharge(ob, &operands, &named)?;
    }

    Ok(StencilProof {
        op: spec.name,
        stencil: program.name.clone(),
        instructions: program.insns.len(),
        continue_paths,
        exit_paths,
        divisions,
        narrowed_paths,
        obligations: obligations.len(),
    })
}

/// Confirm the JIT lowers `spec`'s micro-op to [`stencil_for`]'s stencil with the bindings
/// [`prove_stencil`] assumes.
pub fn check_lowering(spec: &OpSpec) -> Result<(), String> {
    check_lowering_to(spec, stencil_for(spec).map_err(|e| e.to_string())?)
}

/// Confirm the JIT lowers `spec`'s micro-op to `stencil` with the bindings [`prove_stencil`]
/// assumes: compile the op followed by a `Return`, then check the first piece's bytes are
/// the stencil's, that holes 0, 1 and 2 hold the left, right and destination slots, that
/// continuation 0 enters the next piece (`ST_RET2`) and continuation 1 the deopt terminal
/// (`ST_DEOPT`).
pub fn check_lowering_to(spec: &OpSpec, stencil: &Stencil) -> Result<(), String> {
    use logicaffeine_forge::{ST_DEOPT, ST_RET2};
    let op = (spec.build)();
    let (lhs, rhs, dst) = match op {
        MicroOp::Add { dst, lhs, rhs }
        | MicroOp::Sub { dst, lhs, rhs }
        | MicroOp::Mul { dst, lhs, rhs }
        | MicroOp::Div { dst, lhs, rhs }
        | MicroOp::Mod { dst, lhs, rhs }
        | MicroOp::BitAnd { dst, lhs, rhs }
        | MicroOp::BitOr { dst, lhs, rhs }
        | MicroOp::BitXor { dst, lhs, rhs }
        | MicroOp::Shl { dst, lhs, rhs }
        | MicroOp::Shr { dst, lhs, rhs }
        | MicroOp::Lt { dst, lhs, rhs }
        | MicroOp::LtEq { dst, lhs, rhs }
        | MicroOp::Gt { dst, lhs, rhs }
        | MicroOp::GtEq { dst, lhs, rhs }
        | MicroOp::Eq { dst, lhs, rhs }
        | MicroOp::Neq { dst, lhs, rhs } => (i64::from(lhs), i64::from(rhs), i64::from(dst)),
        other => return Err(format!("{other:?} is not a frame-form binop")),
    };
    if lhs == rhs || rhs == dst || lhs == dst {
        return Err(format!(
            "{}: the op's slots ({lhs}, {rhs}, {dst}) must be distinct to tell the holes apart",
            spec.name
        ));
    }
    let chain = compile_straightline(&[op, MicroOp::Return { src: dst as u16 }])
        .map_err(|e| format!("{}: {e:?}", spec.name))?;
    let bytes = chain.bytes();
    let base = bytes.as_ptr() as u64;
    let next = stencil.code.len().div_ceil(16) * 16;
    let deopt = (next + ST_RET2.code.len()).div_ceil(16) * 16;
    piece_matches(bytes, 0, stencil)?;
    piece_matches(bytes, next, &ST_RET2)?;

    let read = |at: usize, n: usize| -> Result<&[u8], String> {
        bytes
            .get(at..at + n)
            .ok_or_else(|| format!("{}: read past the chain at {at:#x}", stencil.name))
    };
    for reloc in stencil.relocs {
        if reloc.kind != RelocKind::GotRel32 {
            return Err(format!(
                "{}: relocation {:?} at +{:#x} is not GotRel32",
                stencil.name, reloc.kind, reloc.offset
            ));
        }
        let site = reloc.offset as usize;
        let disp = i32::from_le_bytes(read(site, 4)?.try_into().expect("4 bytes"));
        let slot = (site as i64 + i64::from(disp) - reloc.addend) as usize;
        let pointer = u64::from_le_bytes(read(slot, 8)?.try_into().expect("8 bytes"));
        let target = pointer.wrapping_sub(base) as usize;
        match reloc.target {
            HoleId::ConstI64(n) => {
                let value = i64::from_le_bytes(read(target, 8)?.try_into().expect("8 bytes"));
                let expected = match n {
                    0 => lhs,
                    1 => rhs,
                    2 => dst,
                    _ => return Err(format!("{}: unexpected constant hole {n}", stencil.name)),
                };
                if value != expected {
                    return Err(format!(
                        "{}: hole {n} holds {value}, but the lowering's slot for it is {expected}",
                        stencil.name
                    ));
                }
            }
            HoleId::Cont(0) if target == next => {}
            HoleId::Cont(1) if target == deopt => piece_matches(bytes, deopt, &ST_DEOPT)?,
            HoleId::Cont(n) => {
                return Err(format!(
                    "{}: continuation {n} enters +{target:#x}, not the piece the proof assumes",
                    stencil.name
                ))
            }
        }
    }
    Ok(())
}

fn piece_matches(bytes: &[u8], at: usize, stencil: &Stencil) -> Result<(), String> {
    let code = stencil.code;
    let placed = bytes.get(at..at + code.len()).ok_or_else(|| {
        format!(
            "{}: the chain ends before the piece at {at:#x}",
            stencil.name
        )
    })?;
    let patched = |i: usize| {
        stencil.relocs.iter().any(|r| {
            let width = if r.kind == RelocKind::Abs64 { 8 } else { 4 };
            (r.offset as usize..r.offset as usize + width).contains(&i)
        })
    };
    match (0..code.len()).find(|&i| !patched(i) && placed[i] != code[i]) {
        None => Ok(()),
        Some(i) => Err(format!(
            "{}: the chain's piece at {at:#x} differs from the stencil at +{i:#x}",
            stencil.name
        )),
    }
}

/// What a register holds.
#[derive(Clone, Debug)]
enum Value {
    Word(BV),
    /// The frame pointer, `rdi` on entry.
    Frame,
    /// A hole's address, as its GOT slot holds it.
    Hole(HoleId),
}

#[derive(Clone, Debug)]
struct Flags {
    cf: Bool,
    of: Bool,
    sf: Bool,
    zf: Bool,
}

impl Flags {
    fn undefined() -> Flags {
        Flags {
            cf: Bool::fresh_const("cf_undefined"),
            of: Bool::fresh_const("of_undefined"),
            sf: Bool::fresh_const("sf_undefined"),
            zf: Bool::fresh_const("zf_undefined"),
        }
    }

    fn test(&self, cond: Cond) -> Bool {
        let lt = self.sf.xor(&self.of);
        match cond {
            Cond::O => self.of.clone(),
            Cond::No => self.of.not(),
            Cond::B => self.cf.clone(),
            Cond::Ae => self.cf.not(),
            Cond::E => self.zf.clone(),
            Cond::Ne => self.zf.not(),
            Cond::Be => Bool::or(&[&self.cf, &self.zf]),
            Cond::A => Bool::and(&[&self.cf.not(), &self.zf.not()]),
            Cond::S => self.sf.clone(),
            Cond::Ns => self.sf.not(),
            Cond::L => lt,
            Cond::Ge => lt.not(),
            Cond::Le => Bool::or(&[&self.zf, &lt]),
            Cond::G => Bool::and(&[&self.zf.not(), &lt.not()]),
        }
    }
}

#[derive(Clone, Debug)]
struct State {
    pc: usize,
    regs: Vec<Value>,
    flags: Flags,
    frame: Array,
    /// The frame stores, in order, as (slot index, value).
    stores: Vec<(BV, BV)>,
    path: Vec<Bool>,
    visited: Vec<usize>,
    /// The path executed a 32-bit divide.
    narrow: bool,
}

impl State {
    fn entry() -> State {
        let regs = Gpr::ALL
            .iter()
            .map(|&r| {
                if r == Gpr::RDI {
                    Value::Frame
                } else {
                    Value::Word(BV::new_const(format!("{}_entry", r.name(Width::Qword)), 64))
                }
            })
            .collect();
        State {
            pc: 0,
            regs,
            flags: Flags {
                cf: Bool::new_const("cf_entry"),
                of: Bool::new_const("of_entry"),
                sf: Bool::new_const("sf_entry"),
                zf: Bool::new_const("zf_entry"),
            },
            frame: Array::new_const("frame", &Sort::bitvector(64), &Sort::bitvector(64)),
            stores: Vec::new(),
            path: Vec::new(),
            visited: Vec::new(),
            narrow: false,
        }
    }
}

struct Exit {
    cont: u8,
    at: usize,
    state: State,
}

/// `path ⇒ goal`, to be proved. A frame obligation's goal compares the path's store log
/// with the expected one entry by entry. That implies the frame equality and keeps the
/// query in bitvectors, which Z3 bit-blasts straight to SAT; an array equality sends it
/// through the array theory, two orders of magnitude slower on the divides. `exact` is the
/// frame equality itself, checked only when the goal is not proved, so a refutation always
/// refutes the equality.
struct Obligation {
    label: String,
    path: Vec<Bool>,
    goal: Bool,
    exact: Option<Bool>,
    narrow: bool,
}

enum Flow {
    Next,
    Goto(usize),
    Fork { cond: Bool, target: usize },
    Exit(u8),
}

enum Location {
    Got(HoleId),
    Literal(u8),
    Slot(BV),
}

struct Program {
    name: String,
    insns: BTreeMap<usize, Insn>,
    /// The hole each `rip`-relative operand's GOT slot holds, by displacement offset.
    sites: BTreeMap<usize, HoleId>,
}

fn hole_value(n: u8) -> BV {
    BV::new_const(format!("hole{n}"), 64)
}

fn select(frame: &Array, index: &BV) -> BV {
    frame
        .select(index)
        .as_bv()
        .expect("the frame's range is BV64")
}

fn bv(value: u64, bits: u32) -> BV {
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    BV::from_u64(value & mask, bits)
}

fn low(v: &BV, width: Width) -> BV {
    if width == Width::Qword {
        v.clone()
    } else {
        v.extract(width.bits() - 1, 0)
    }
}

fn msb(v: &BV) -> Bool {
    let top = v.get_size() - 1;
    v.extract(top, top).eq(&bv(1, 1))
}

fn bit0(v: &BV) -> Bool {
    v.extract(0, 0).eq(&bv(1, 1))
}

fn is_zero(v: &BV) -> Bool {
    v.eq(&bv(0, v.get_size()))
}

/// The value and flags of an ALU operation at `x`'s width.
fn alu(op: AluOp, x: &BV, y: &BV) -> (BV, Flags) {
    let f = Bool::from_bool(false);
    let (r, cf, of) = match op {
        AluOp::Add => {
            let r = x.bvadd(y);
            let cf = r.bvult(x);
            let of = Bool::and(&[&msb(x).iff(&msb(y)), &msb(&r).xor(&msb(x))]);
            (r, cf, of)
        }
        AluOp::Sub | AluOp::Cmp => {
            let r = x.bvsub(y);
            let cf = x.bvult(y);
            let of = Bool::and(&[&msb(x).xor(&msb(y)), &msb(&r).xor(&msb(x))]);
            (r, cf, of)
        }
        AluOp::And => (x.bvand(y), f.clone(), f),
        AluOp::Or => (x.bvor(y), f.clone(), f),
        AluOp::Xor => (x.bvxor(y), f.clone(), f),
    };
    let flags = Flags {
        cf,
        of,
        sf: msb(&r),
        zf: is_zero(&r),
    };
    (r, flags)
}

impl Program {
    fn new(name: &str, code: &[u8], relocs: &[Reloc]) -> Result<Program, ProofError> {
        let decoded = x86::decode_all(code).map_err(|error| ProofError::Decode {
            stencil: name.to_string(),
            error,
        })?;
        let mut program = Program {
            name: name.to_string(),
            insns: BTreeMap::new(),
            sites: BTreeMap::new(),
        };
        for insn in decoded {
            if let Some(m) = insn.op.mem() {
                if m.base == Some(Base::Rip) {
                    let at = m
                        .disp_at
                        .expect("a rip-relative operand has a displacement");
                    let reloc =
                        relocs
                            .iter()
                            .find(|r| r.offset as usize == at)
                            .ok_or_else(|| {
                                program.unmodelled(
                                    insn.offset,
                                    format!(
                                        "`{}` reads a rip-relative address no relocation patches",
                                        insn.op
                                    ),
                                )
                            })?;
                    if reloc.kind != RelocKind::GotRel32 {
                        return Err(program.unmodelled(
                            insn.offset,
                            format!("relocation {:?} is not a GOT slot", reloc.kind),
                        ));
                    }
                    let expected = at as i64 - insn.end() as i64;
                    if reloc.addend != expected {
                        return Err(program.unmodelled(
                            insn.offset,
                            format!(
                                "relocation addend {} does not address the hole (expected {expected})",
                                reloc.addend
                            ),
                        ));
                    }
                    program.sites.insert(at, reloc.target);
                }
            }
            program.insns.insert(insn.offset, insn);
        }
        if let Some(stray) = relocs
            .iter()
            .find(|r| !program.sites.contains_key(&(r.offset as usize)))
        {
            return Err(program.unmodelled(
                stray.offset as usize,
                format!(
                    "relocation {:?} for {:?} patches no rip-relative operand",
                    stray.kind, stray.target
                ),
            ));
        }
        Ok(program)
    }

    fn unmodelled(&self, offset: usize, reason: impl Into<String>) -> ProofError {
        ProofError::Unmodelled {
            stencil: self.name.clone(),
            offset,
            reason: reason.into(),
        }
    }

    fn explore(
        &self,
        init: State,
        obligations: &mut Vec<Obligation>,
    ) -> Result<Vec<Exit>, ProofError> {
        let mut work = vec![init];
        let mut exits = Vec::new();
        while let Some(mut st) = work.pop() {
            loop {
                if st.visited.contains(&st.pc) {
                    return Err(self.unmodelled(st.pc, "the path returns here: a loop"));
                }
                st.visited.push(st.pc);
                let insn = *self.insns.get(&st.pc).ok_or_else(|| {
                    self.unmodelled(
                        st.pc,
                        "the path runs off the code or into the middle of an instruction",
                    )
                })?;
                match self.step(&mut st, &insn, obligations)? {
                    Flow::Next => st.pc = insn.end(),
                    Flow::Goto(target) => st.pc = target,
                    Flow::Fork { cond, target } => {
                        let mut taken = st.clone();
                        taken.path.push(cond.clone());
                        taken.pc = target;
                        work.push(taken);
                        st.path.push(cond.not());
                        st.pc = insn.end();
                    }
                    Flow::Exit(cont) => {
                        exits.push(Exit {
                            cont,
                            at: insn.offset,
                            state: st,
                        });
                        break;
                    }
                }
                if exits.len() + work.len() > MAX_PATHS {
                    return Err(self.unmodelled(insn.offset, "more than 64 paths"));
                }
            }
        }
        Ok(exits)
    }

    fn jump_target(&self, insn: &Insn, target: i64) -> Result<usize, ProofError> {
        usize::try_from(target)
            .ok()
            .filter(|t| self.insns.contains_key(t))
            .ok_or_else(|| {
                self.unmodelled(
                    insn.offset,
                    format!(
                        "`{}` jumps to {target:#x}, not an instruction of the stencil",
                        insn.op
                    ),
                )
            })
    }

    fn locate(&self, st: &State, insn: &Insn, m: &Mem) -> Result<Location, ProofError> {
        match m.base {
            Some(Base::Rip) => {
                let at = m
                    .disp_at
                    .expect("a rip-relative operand has a displacement");
                Ok(Location::Got(self.sites[&at]))
            }
            Some(Base::Gpr(r)) => match &st.regs[r.number() as usize] {
                Value::Hole(HoleId::ConstI64(n)) if m.index.is_none() && m.disp == 0 => {
                    Ok(Location::Literal(*n))
                }
                Value::Frame => {
                    if m.disp % 8 != 0 {
                        return Err(self.unmodelled(
                            insn.offset,
                            format!("`{}` addresses inside a frame slot", insn.op),
                        ));
                    }
                    let k = i64::from(m.disp / 8);
                    let index = match m.index {
                        None => BV::from_i64(k, 64),
                        Some(ix) => {
                            if m.scale != 8 {
                                return Err(self.unmodelled(
                                    insn.offset,
                                    format!("`{}` scales a slot index by {}", insn.op, m.scale),
                                ));
                            }
                            let i = self.word(st, insn, ix, Width::Qword)?;
                            if k == 0 {
                                i
                            } else {
                                i.bvadd(&BV::from_i64(k, 64))
                            }
                        }
                    };
                    Ok(Location::Slot(index))
                }
                _ => Err(self.unmodelled(
                    insn.offset,
                    format!(
                        "`{}` addresses memory through {}, which holds no frame or literal address",
                        insn.op,
                        r.name(Width::Qword)
                    ),
                )),
            },
            None => Err(self.unmodelled(
                insn.offset,
                format!("`{}` addresses absolute memory", insn.op),
            )),
        }
    }

    fn load(&self, st: &State, insn: &Insn, m: &Mem, width: Width) -> Result<Value, ProofError> {
        match self.locate(st, insn, m)? {
            Location::Got(hole) if width == Width::Qword => Ok(Value::Hole(hole)),
            Location::Literal(n) if width == Width::Qword => Ok(Value::Word(hole_value(n))),
            Location::Slot(index) => Ok(Value::Word(low(&select(&st.frame, &index), width))),
            Location::Got(_) | Location::Literal(_) => Err(self.unmodelled(
                insn.offset,
                format!("`{}` reads part of a hole's pool entry", insn.op),
            )),
        }
    }

    fn word(&self, st: &State, insn: &Insn, r: Gpr, width: Width) -> Result<BV, ProofError> {
        match &st.regs[r.number() as usize] {
            Value::Word(v) => Ok(low(v, width)),
            _ => Err(self.unmodelled(
                insn.offset,
                format!(
                    "`{}` uses {}, which holds a pointer, as data",
                    insn.op,
                    r.name(Width::Qword)
                ),
            )),
        }
    }

    fn read(&self, st: &State, insn: &Insn, o: &Operand, width: Width) -> Result<BV, ProofError> {
        match o {
            Operand::Reg(r) => self.word(st, insn, *r, width),
            Operand::Mem(m) => match self.load(st, insn, m, width)? {
                Value::Word(v) => Ok(v),
                _ => Err(self.unmodelled(
                    insn.offset,
                    format!("`{}` uses a hole's address as data", insn.op),
                )),
            },
            Operand::Imm(v) => Ok(bv(*v as u64, width.bits())),
        }
    }

    fn read_value(&self, st: &State, insn: &Insn, o: &Operand) -> Result<Value, ProofError> {
        match o {
            Operand::Reg(r) => Ok(st.regs[r.number() as usize].clone()),
            Operand::Mem(m) => self.load(st, insn, m, Width::Qword),
            Operand::Imm(v) => Ok(Value::Word(BV::from_i64(*v, 64))),
        }
    }

    fn write_reg(
        &self,
        st: &mut State,
        insn: &Insn,
        r: Gpr,
        width: Width,
        v: Value,
    ) -> Result<(), ProofError> {
        let slot = r.number() as usize;
        st.regs[slot] = match (width, v) {
            (Width::Qword, v) => v,
            (Width::Dword, Value::Word(v)) => Value::Word(v.zero_ext(32)),
            (Width::Byte, Value::Word(v)) => {
                let old = self.word(st, insn, r, Width::Qword)?;
                Value::Word(old.extract(63, 8).concat(&v))
            }
            _ => {
                return Err(self.unmodelled(
                    insn.offset,
                    format!("`{}` writes part of a pointer", insn.op),
                ))
            }
        };
        Ok(())
    }

    fn write(
        &self,
        st: &mut State,
        insn: &Insn,
        o: &Operand,
        width: Width,
        v: Value,
    ) -> Result<(), ProofError> {
        match o {
            Operand::Reg(r) => self.write_reg(st, insn, *r, width, v),
            Operand::Mem(m) => {
                let Value::Word(v) = v else {
                    return Err(self.unmodelled(
                        insn.offset,
                        format!("`{}` stores a pointer to memory", insn.op),
                    ));
                };
                match self.locate(st, insn, m)? {
                    Location::Slot(index) => {
                        let merged = if width == Width::Qword {
                            v
                        } else {
                            select(&st.frame, &index)
                                .extract(63, width.bits())
                                .concat(&v)
                        };
                        st.frame = st.frame.store(&index, &merged);
                        st.stores.push((index, merged));
                        Ok(())
                    }
                    _ => Err(self.unmodelled(
                        insn.offset,
                        format!("`{}` stores into the hole pool", insn.op),
                    )),
                }
            }
            Operand::Imm(_) => Err(self.unmodelled(insn.offset, "a write to an immediate")),
        }
    }

    fn step(
        &self,
        st: &mut State,
        insn: &Insn,
        obligations: &mut Vec<Obligation>,
    ) -> Result<Flow, ProofError> {
        match insn.op {
            Op::Mov { width, dst, src } => {
                let v = if width == Width::Qword {
                    self.read_value(st, insn, &src)?
                } else {
                    Value::Word(self.read(st, insn, &src, width)?)
                };
                self.write(st, insn, &dst, width, v)?;
            }
            Op::Movzx { width, dst, src } => {
                let byte = self.read(st, insn, &src, Width::Byte)?;
                let v = byte.zero_ext(width.bits() - 8);
                self.write_reg(st, insn, dst, width, Value::Word(v))?;
            }
            Op::Alu {
                op,
                width,
                dst,
                src,
            } => {
                let zeroing = matches!(op, AluOp::Xor | AluOp::Sub)
                    && matches!(dst, Operand::Reg(_))
                    && dst == src;
                let (x, y) = if zeroing {
                    (bv(0, width.bits()), bv(0, width.bits()))
                } else {
                    (
                        self.read(st, insn, &dst, width)?,
                        self.read(st, insn, &src, width)?,
                    )
                };
                let (r, flags) = alu(op, &x, &y);
                st.flags = flags;
                if op != AluOp::Cmp {
                    self.write(st, insn, &dst, width, Value::Word(r))?;
                }
            }
            Op::Test { width, lhs, rhs } => {
                let x = self.read(st, insn, &lhs, width)?;
                let y = self.read(st, insn, &rhs, width)?;
                st.flags = alu(AluOp::And, &x, &y).1;
            }
            Op::Shift {
                op,
                width,
                dst,
                count,
            } => {
                if width != Width::Qword {
                    return Err(self.unmodelled(insn.offset, "a shift narrower than 64 bits"));
                }
                let x = self.read(st, insn, &dst, width)?;
                let count = match count {
                    ShiftCount::Cl => self
                        .word(st, insn, Gpr::RCX, Width::Byte)?
                        .zero_ext(56)
                        .bvand(&bv(63, 64)),
                    ShiftCount::Imm(n) => bv(u64::from(n & 63), 64),
                };
                let one = bv(1, 64);
                let (r, cf, of_one) = match op {
                    ShiftOp::Shl => {
                        let r = x.bvshl(&count);
                        let cf = bit0(&x.bvlshr(&bv(64, 64).bvsub(&count)));
                        let of = msb(&r).xor(&cf);
                        (r, cf, of)
                    }
                    ShiftOp::Shr => (
                        x.bvlshr(&count),
                        bit0(&x.bvlshr(&count.bvsub(&one))),
                        msb(&x),
                    ),
                    ShiftOp::Sar => (
                        x.bvashr(&count),
                        bit0(&x.bvashr(&count.bvsub(&one))),
                        Bool::from_bool(false),
                    ),
                };
                let unchanged = is_zero(&count);
                let of = count
                    .eq(&one)
                    .ite(&of_one, &Bool::fresh_const("of_undefined"));
                st.flags = Flags {
                    cf: unchanged.ite(&st.flags.cf, &cf),
                    of: unchanged.ite(&st.flags.of, &of),
                    sf: unchanged.ite(&st.flags.sf, &msb(&r)),
                    zf: unchanged.ite(&st.flags.zf, &is_zero(&r)),
                };
                self.write(st, insn, &dst, width, Value::Word(r))?;
            }
            Op::Imul { width, dst, src } => {
                let x = self.word(st, insn, dst, width)?;
                let y = self.read(st, insn, &src, width)?;
                let n = width.bits();
                let r = x.bvmul(&y);
                let exact = x.sign_ext(n).bvmul(&y.sign_ext(n));
                let overflow = exact.eq(&r.sign_ext(n)).not();
                st.flags = Flags {
                    cf: overflow.clone(),
                    of: overflow,
                    ..Flags::undefined()
                };
                self.write_reg(st, insn, dst, width, Value::Word(r))?;
            }
            Op::Neg { width, dst } => {
                let x = self.read(st, insn, &dst, width)?;
                let r = x.bvneg();
                let min = bv(1u64 << (width.bits() - 1), width.bits());
                st.flags = Flags {
                    cf: is_zero(&x).not(),
                    of: x.eq(&min),
                    sf: msb(&r),
                    zf: is_zero(&r),
                };
                self.write(st, insn, &dst, width, Value::Word(r))?;
            }
            Op::Div {
                signed,
                width,
                divisor,
            } => {
                let d = self.read(st, insn, &divisor, width)?;
                let (q, r, safe, label) = match width {
                    Width::Dword => {
                        let lo = self.word(st, insn, Gpr::RAX, Width::Dword)?;
                        let hi = self.word(st, insn, Gpr::RDX, Width::Dword)?;
                        let dividend = hi.concat(&lo);
                        let (q, r, fits) = if signed {
                            let dv = d.sign_ext(32);
                            let q = dividend.bvsdiv(&dv);
                            let fits = q.extract(31, 0).sign_ext(32).eq(&q);
                            (q, dividend.bvsrem(&dv), fits)
                        } else {
                            let dv = d.zero_ext(32);
                            let q = dividend.bvudiv(&dv);
                            let fits = is_zero(&q.extract(63, 32));
                            (q, dividend.bvurem(&dv), fits)
                        };
                        st.narrow = true;
                        (
                            q.extract(31, 0),
                            r.extract(31, 0),
                            Bool::and(&[&is_zero(&d).not(), &fits]),
                            "the divisor is non-zero and the quotient fits",
                        )
                    }
                    Width::Qword => {
                        let lo = self.word(st, insn, Gpr::RAX, Width::Qword)?;
                        let hi = self.word(st, insn, Gpr::RDX, Width::Qword)?;
                        if signed {
                            let overflow =
                                Bool::and(&[&lo.eq(&bv(1u64 << 63, 64)), &d.eq(&bv(u64::MAX, 64))]);
                            (
                                lo.bvsdiv(&d),
                                lo.bvsrem(&d),
                                Bool::and(&[
                                    &hi.eq(&lo.bvashr(&bv(63, 64))),
                                    &is_zero(&d).not(),
                                    &overflow.not(),
                                ]),
                                "rdx:rax is the sign extension of rax, the divisor is non-zero and the quotient fits",
                            )
                        } else {
                            (
                                lo.bvudiv(&d),
                                lo.bvurem(&d),
                                Bool::and(&[&is_zero(&hi), &is_zero(&d).not()]),
                                "rdx is zero and the divisor is non-zero",
                            )
                        }
                    }
                    Width::Byte => {
                        return Err(self.unmodelled(insn.offset, "an 8-bit divide"));
                    }
                };
                obligations.push(Obligation {
                    label: format!(
                        "`{}` at +{:#x} cannot raise #DE: {label}",
                        insn.op, insn.offset
                    ),
                    path: st.path.clone(),
                    goal: safe,
                    exact: None,
                    narrow: st.narrow,
                });
                self.write_reg(st, insn, Gpr::RAX, width, Value::Word(q))?;
                self.write_reg(st, insn, Gpr::RDX, width, Value::Word(r))?;
                st.flags = Flags::undefined();
            }
            Op::SignExtend { width } => {
                let a = self.word(st, insn, Gpr::RAX, width)?;
                let sign = a.bvashr(&bv(u64::from(width.bits() - 1), width.bits()));
                self.write_reg(st, insn, Gpr::RDX, width, Value::Word(sign))?;
            }
            Op::Setcc { cond, dst } => {
                let v = st.flags.test(cond).ite(&bv(1, 8), &bv(0, 8));
                self.write(st, insn, &dst, Width::Byte, Value::Word(v))?;
            }
            Op::Jcc { cond, target } => {
                return Ok(Flow::Fork {
                    cond: st.flags.test(cond),
                    target: self.jump_target(insn, target)?,
                })
            }
            Op::Jmp { target } => return Ok(Flow::Goto(self.jump_target(insn, target)?)),
            Op::JmpIndirect { target } => {
                return match self.read_value(st, insn, &target)? {
                    Value::Hole(HoleId::Cont(n)) => Ok(Flow::Exit(n)),
                    _ => Err(self.unmodelled(
                        insn.offset,
                        format!("`{}` jumps somewhere other than a continuation", insn.op),
                    )),
                }
            }
        }
        Ok(Flow::Next)
    }

    /// The substitution that replaces each operand `path` confines to 32 bits by the zero
    /// extension of a fresh 32-bit variable (see the module docs), and the terms that stand
    /// for the operands afterwards.
    fn narrowing(
        &self,
        path: &[Bool],
        operands: &[(&str, BV); 2],
    ) -> (Vec<(BV, BV)>, Vec<(String, BV)>) {
        let mut subs = Vec::new();
        let mut shown = Vec::new();
        for (name, t) in operands {
            let solver = Solver::new();
            for p in path {
                solver.assert(p);
            }
            solver.assert(&is_zero(&t.bvlshr(&bv(32, 64))).not());
            if solver.check() == SatResult::Unsat {
                let narrow = BV::fresh_const(&format!("{name}_low"), 32).zero_ext(32);
                shown.push((name.to_string(), narrow.clone()));
                subs.push((t.clone(), narrow));
            } else {
                shown.push((name.to_string(), t.clone()));
            }
        }
        (subs, shown)
    }

    fn discharge(
        &self,
        ob: &Obligation,
        operands: &[(&str, BV); 2],
        named: &[(&str, BV); 3],
    ) -> Result<(), ProofError> {
        let (subs, shown) = if ob.narrow {
            self.narrowing(&ob.path, operands)
        } else {
            let shown = operands
                .iter()
                .map(|(n, t)| (n.to_string(), t.clone()))
                .collect();
            (Vec::new(), shown)
        };
        let pairs: Vec<(&BV, &BV)> = subs.iter().map(|(from, to)| (from, to)).collect();
        let apply = |b: &Bool| {
            if pairs.is_empty() {
                b.clone()
            } else {
                b.substitute(&pairs)
            }
        };
        let path: Vec<Bool> = ob.path.iter().map(apply).collect();
        match (
            self.refute(ob, &path, &apply(&ob.goal), &shown, named),
            &ob.exact,
        ) {
            (Err(_), Some(exact)) => self.refute(ob, &path, &apply(exact), &shown, named),
            (verdict, _) => verdict,
        }
    }

    fn refute(
        &self,
        ob: &Obligation,
        path: &[Bool],
        goal: &Bool,
        shown: &[(String, BV)],
        named: &[(&str, BV); 3],
    ) -> Result<(), ProofError> {
        let solver = Solver::new();
        for p in path {
            solver.assert(p);
        }
        solver.assert(&goal.not());
        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
                let model = solver.get_model();
                let value = |t: &BV| {
                    model
                        .as_ref()
                        .and_then(|m| m.eval(t, true))
                        .and_then(|v| v.as_u64())
                        .map(|u| (u as i64).to_string())
                        .unwrap_or_else(|| "?".to_string())
                };
                let counterexample = shown
                    .iter()
                    .map(|(n, t)| format!("{n} = {}", value(t)))
                    .chain(named.iter().map(|(n, t)| format!("{n} = {}", value(t))))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(ProofError::Refuted {
                    stencil: self.name.clone(),
                    obligation: ob.label.clone(),
                    counterexample,
                })
            }
            SatResult::Unknown => Err(ProofError::Undecided {
                stencil: self.name.clone(),
                obligation: ob.label.clone(),
                reason: solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "unknown".to_string()),
            }),
        }
    }
}
