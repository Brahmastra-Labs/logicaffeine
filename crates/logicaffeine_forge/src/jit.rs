//! J1: the straight-line micro-op compiler — bytecode in, native code out.
//!
//! [`MicroOp`] is the JIT's input IR: the integer subset of the bytecode VM's
//! register operations. [`compile_straightline`] lowers each op to a fixed
//! stencil micro-sequence over the frame/operand-stack machine model and glues
//! the result into one executable chain:
//!
//! ```text
//! LoadConst{dst,v}  →  const(v); slot_set(dst)
//! Move{dst,src}     →  slot_get(src); slot_set(dst)
//! Add{dst,l,r}      →  slot_get(l); slot_get(r); addi; slot_set(dst)
//! …                    (Sub/Mul/Lt/Eq identical shape)
//! Gt{dst,l,r}       →  slot_get(r); slot_get(l); lti; slot_set(dst)   (swap)
//! Return{src}       →  slot_get(src); return
//! ```
//!
//! Anything outside this subset is the caller's tier-up bail (`None` from the
//! adapter) — the VM keeps running it.

use std::sync::atomic::{AtomicI64, Ordering};

use crate::buffer::{HoleValue, JitBuffer, JitChain};
use crate::{
    V_BINOP, V_BRANCH, V_CONST, V_FBINOP, V_FMOV, V_MOV, V_RET, V_SQRTF, V_DIVF, V_I2F, V_BRANCHF,
    V_ARRLD_RPTR, V_ARRLD_RPTR_C, V_ARRST_RPTR, V_ARRST_RPTR_C,
    ST_ADD3, ST_ADDF3, ST_AND3, ST_ARRLD, ST_ARRLD2_ADDF, ST_ARRLD2_MULF, ST_ARRLD2_SUBF, ST_ARRLDB, ST_ARRLDB_U, ST_ARRLD_U, ST_ARRST, ST_ARRSTB, ST_ARRSTB_U, ST_ARRST_U, ST_BREQ, ST_BREQF, ST_BRLEF, ST_BRLT, ST_CALL, ST_PUSH, ST_LIST_CLEAR,
    ST_ARRLD_I32, ST_ARRLD_I32_U, ST_ARRST_I32, ST_ARRST_I32_U,
    ST_ARRLD2_ADD, ST_ARRLD2_ADD_U, ST_ARRLD2_SUB, ST_ARRLD2_SUB_U, ST_ARRLD2_MUL, ST_ARRLD2_MUL_U,
    ST_ARRLDAFF_NONE, ST_ARRLDAFF_NONE_U, ST_ARRLDAFF_ADD, ST_ARRLDAFF_ADD_U,
    ST_ARRLDAFF_SUB, ST_ARRLDAFF_SUB_U, ST_ARRLDAFF_MUL, ST_ARRLDAFF_MUL_U,
    ST_ARRRMW_ADD, ST_ARRRMW_ADD_U, ST_ARRRMW_SUB, ST_ARRRMW_SUB_U, ST_ARRRMW_MUL, ST_ARRRMW_MUL_U,
    ST_ARRRMW_AND, ST_ARRRMW_AND_U, ST_ARRRMW_OR, ST_ARRRMW_OR_U, ST_ARRRMW_XOR, ST_ARRRMW_XOR_U,
    ST_ARRRMW_ADDF, ST_ARRRMW_ADDF_U, ST_ARRRMW_SUBF, ST_ARRRMW_SUBF_U, ST_ARRRMW_MULF, ST_ARRRMW_MULF_U,
    ST_FMAF,
    ST_ARRCONDSWAP_GT, ST_ARRCONDSWAP_GT_U, ST_ARRCONDSWAP_LT, ST_ARRCONDSWAP_LT_U,
    ST_ARRCONDSWAP_GE, ST_ARRCONDSWAP_GE_U, ST_ARRCONDSWAP_LE, ST_ARRCONDSWAP_LE_U,
    ST_ARRSWAP, ST_ARRSWAP_U,
    ST_BRLTF, ST_BRZ, ST_CALL_PRECISE, ST_CONSTST, ST_DEOPT, ST_DEOPT_AT, ST_DIV3C,
    ST_DIVPOW2, ST_MAGICDIV,
    ST_DIVF3C, ST_EQ3, ST_EQF3, ST_I2F2,
    ST_JUMP, ST_LE3, ST_LEF3, ST_LT3, ST_LTF3, ST_MOD3C, ST_MOVSS, ST_MUL3, ST_MULF3, ST_NE3,
    ST_ALLOCLIST, ST_CALL_SELF, ST_CALL_SELF_COPY, ST_LISTTRIPLE, ST_MAPHGET, ST_MAPHHAS, ST_MAPHSET, ST_MEMMEM, ST_NEF3,
    ST_NOTB2,
    ST_NOTI2, ST_OR3, ST_RET2,
    ST_SHL3, ST_SHR3, ST_SQRTF2, ST_SUB3,
    ST_SUBF3, ST_XOR3,
};

/// Frame slot index (a VM register number).
pub type Slot = u16;

/// The kernel's locked maximum LOGOS call depth, BAKED into the precise and
/// self call stencils (their holes are all spoken for, so the limit cannot be
/// a runtime hole). It mirrors `logicaffeine_compile::semantics::MAX_CALL_DEPTH`
/// and the matching constant in `stencils/int_stencils.rs`; the callers in the
/// JIT crate pass that same value, asserted here at stencil selection so a
/// drift fails loudly instead of silently capping native recursion at the wrong
/// depth (which would diverge from the kernel's depth-exceeded error).
pub const BAKED_CALL_DEPTH: i64 = 2_500;

/// Comparison kinds for [`MicroOp::Branch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    /// `lhs < rhs`
    Lt,
    /// `lhs > rhs`
    Gt,
    /// `lhs <= rhs`
    LtEq,
    /// `lhs >= rhs`
    GtEq,
    /// `lhs == rhs`
    Eq,
    /// `lhs != rhs`
    NotEq,
}

impl Cmp {
    /// Evaluate over two i64s.
    pub fn eval(self, a: i64, b: i64) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Gt => a > b,
            Cmp::LtEq => a <= b,
            Cmp::GtEq => a >= b,
            Cmp::Eq => a == b,
            Cmp::NotEq => a != b,
        }
    }

    /// The comparison with the opposite truth value.
    pub fn negated(self) -> Cmp {
        match self {
            Cmp::Lt => Cmp::GtEq,
            Cmp::Gt => Cmp::LtEq,
            Cmp::LtEq => Cmp::Gt,
            Cmp::GtEq => Cmp::Lt,
            Cmp::Eq => Cmp::NotEq,
            Cmp::NotEq => Cmp::Eq,
        }
    }
}

/// The float binary operation of a fused [`MicroOp::ArrLoad2F`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FOp {
    /// IEEE addition.
    Add,
    /// IEEE subtraction.
    Sub,
    /// IEEE multiplication.
    Mul,
}

impl FOp {
    /// Apply over two f64s.
    pub fn eval(self, a: f64, b: f64) -> f64 {
        match self {
            FOp::Add => a + b,
            FOp::Sub => a - b,
            FOp::Mul => a * b,
        }
    }
}

/// The integer binary operation of a fused two-buffer [`MicroOp::ArrLoad2`]:
/// `frame[dst] = a[i-1] <op> b[j-1]`, the two elements loaded from two
/// (possibly distinct) pinned 8-byte int buffers. Add/Mul commute; Sub does
/// not (it only fuses `a[i] - b[j]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IOp {
    /// Wrapping i64 addition.
    Add,
    /// Wrapping i64 subtraction.
    Sub,
    /// Wrapping i64 multiplication.
    Mul,
}

impl IOp {
    /// Apply over two i64s with the kernel's wrapping semantics.
    pub fn eval(self, a: i64, b: i64) -> i64 {
        match self {
            IOp::Add => a.wrapping_add(b),
            IOp::Sub => a.wrapping_sub(b),
            IOp::Mul => a.wrapping_mul(b),
        }
    }
}

/// The index-arithmetic shape folded into a fused [`MicroOp::ArrLoadAffine`].
/// The computed 1-based index is `(frame[a] OP frame[b]).wrapping_add(c)` for a
/// two-slot op, or `frame[a].wrapping_add(c)` for [`AffOp::None`] (a single slot
/// plus a constant). The op wraps with the kernel's exact i64 semantics so the
/// load is bit-identical to the un-fused index arithmetic + `ArrLoad`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AffOp {
    /// No second slot: `idx = frame[a] + c` (the `w + 1` shape).
    None,
    /// `idx = (frame[a] + frame[b]) + c` (the `i*n + j + 1` tail).
    Add,
    /// `idx = (frame[a] - frame[b]) + c` (the `w - wi + 1` shape).
    Sub,
    /// `idx = (frame[a] * frame[b]) + c`.
    Mul,
}

impl AffOp {
    /// Compute the 1-based index with the kernel's wrapping i64 semantics.
    pub fn eval(self, a: i64, b: i64, c: i64) -> i64 {
        match self {
            AffOp::None => a.wrapping_add(c),
            AffOp::Add => a.wrapping_add(b).wrapping_add(c),
            AffOp::Sub => a.wrapping_sub(b).wrapping_add(c),
            AffOp::Mul => a.wrapping_mul(b).wrapping_add(c),
        }
    }
}

/// The operation of a fused read-modify-write [`MicroOp::ArrRMW`]:
/// `buf[idx-1] = buf[idx-1] <op> operand`. The INT ops (Add/Sub/Mul wrap as the
/// kernel's i64 arithmetic; And/Or/Xor bitwise) treat the element and operand as
/// raw i64; the FLOAT ops (AddF/SubF/MulF) reinterpret both as f64 (the nbody
/// velocity/position-update idiom). Sub/SubF are the only non-commutative
/// members — the peephole fuses them only when the loaded element is the LEFT
/// operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RmwOp {
    /// Wrapping addition.
    Add,
    /// Wrapping subtraction (`buf[i] - operand`).
    Sub,
    /// Wrapping multiplication.
    Mul,
    /// Bitwise AND.
    And,
    /// Bitwise OR.
    Or,
    /// Bitwise XOR.
    Xor,
    /// IEEE addition (operands reinterpreted as f64).
    AddF,
    /// IEEE subtraction (`buf[i] - operand`, as f64).
    SubF,
    /// IEEE multiplication (as f64).
    MulF,
}

impl RmwOp {
    /// Whether the op reinterprets its operands as f64.
    pub fn is_float(self) -> bool {
        matches!(self, RmwOp::AddF | RmwOp::SubF | RmwOp::MulF)
    }

    /// Apply over the raw i64 bits of the element and operand — the bit-exact
    /// spec every stencil must match. Float ops reinterpret the bits as f64,
    /// compute, and re-encode (copy-and-patch never reassociates, so ordering is
    /// exact).
    pub fn eval(self, a: i64, b: i64) -> i64 {
        match self {
            RmwOp::Add => a.wrapping_add(b),
            RmwOp::Sub => a.wrapping_sub(b),
            RmwOp::Mul => a.wrapping_mul(b),
            RmwOp::And => a & b,
            RmwOp::Or => a | b,
            RmwOp::Xor => a ^ b,
            RmwOp::AddF => (f64::from_bits(a as u64) + f64::from_bits(b as u64)).to_bits() as i64,
            RmwOp::SubF => (f64::from_bits(a as u64) - f64::from_bits(b as u64)).to_bits() as i64,
            RmwOp::MulF => (f64::from_bits(a as u64) * f64::from_bits(b as u64)).to_bits() as i64,
        }
    }
}

/// The source operand appended by a [`MicroOp::StrAppend`] — the right-hand side
/// of a `Set text to text + <s>` in a pinned mutable-Text build loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrSrc {
    /// A single ASCII byte VALUE living in a frame slot (the `Kind::TextByte`
    /// lane: `text + ch` where `ch` is a 1-char ASCII text). The helper appends
    /// `byte as char` — bit-identical to the VM concatenating the 1-char `Text`.
    Byte(Slot),
    /// A constant ASCII byte slice baked into the chain (`text + "XXXXX"`): a
    /// `'static` pointer + length. The helper appends the whole slice — identical
    /// to the VM's `Text + Text` concatenation of the literal.
    Const {
        /// `'static` pointer to the constant's bytes.
        ptr: i64,
        /// The constant's byte length.
        len: i64,
    },
}

/// The integer straight-line subset the J1 compiler accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicroOp {
    /// `frame[dst] = value`.
    LoadConst {
        /// Destination slot.
        dst: Slot,
        /// Immediate value.
        value: i64,
    },
    /// `frame[dst] = frame[src]`.
    Move {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// `frame[dst] = frame[lhs] + frame[rhs]` (wrapping).
    Add {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] - frame[rhs]` (wrapping).
    Sub {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] * frame[rhs]` (wrapping).
    Mul {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] < frame[rhs]) as i64`.
    Lt {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] > frame[rhs]) as i64`.
    Gt {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] == frame[rhs]) as i64`.
    Eq {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] / frame[rhs]` (wrapping; `MIN / -1 = MIN`).
    /// A zero divisor SIDE-EXITS the chain ([`ChainOutcome::Deopt`]) before
    /// any effect — the caller replays on bytecode, where the kernel raises
    /// the exact "Division by zero" error.
    Div {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] / 2^k` (signed, round toward zero) via a
    /// sign-correcting shift — bit-exact with [`MicroOp::Div`] but with NO
    /// side-exit (a power-of-two divisor is never `0` or `-1`). Emitted in
    /// place of `Div` when the divisor is a region-constant power of two and
    /// the dividend is integer-typed.
    DivPow2 {
        /// Destination slot.
        dst: Slot,
        /// Dividend slot.
        lhs: Slot,
        /// Shift amount `k` (divisor = `2^k`, `1 <= k <= 62`).
        k: u32,
    },
    /// `frame[dst] = frame[lhs] / c` (`mul_back == 0`) or `frame[lhs] % c`
    /// (`mul_back == c`) by the Granlund–Montgomery / libdivide UNSIGNED
    /// magic-reciprocal sequence (`mulhi(magic, x)` + add-fixup + shift, then
    /// `x - q*c` for the remainder) — a `mul`+`shr` (~3 cycles) instead of
    /// `idiv` (~25). NO side-exit (a literal `c > 0` is never `0` or `-1`).
    /// Emitted in place of `Div`/`Mod` when the divisor is a compile-time
    /// constant non-power-of-two and the dividend is proven Int and NON-NEGATIVE
    /// (the unsigned magic equals the signed truncating result only there). The
    /// `more` byte is the [`logicaffeine_data::LogosDivU64`] encoding (low 6 bits
    /// = shift, `0x40` = the 65-bit add-marker path, `0x80` = pure-shift pow2).
    MagicDivU {
        /// Destination slot.
        dst: Slot,
        /// Dividend slot.
        lhs: Slot,
        /// Precomputed magic multiplier `M`.
        magic: u64,
        /// Shift / path encoding (see [`logicaffeine_data::LogosDivU64`]).
        more: u8,
        /// `0` selects the quotient; otherwise the divisor `c` selects the
        /// remainder `x - (x/c)*c`.
        mul_back: i64,
    },
    /// `frame[dst] = frame[lhs] % frame[rhs]` (wrapping; `MIN % -1 = 0`).
    /// Zero divisor side-exits exactly like [`MicroOp::Div`].
    Mod {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] <= frame[rhs]) as i64`.
    LtEq {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] >= frame[rhs]) as i64`.
    GtEq {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = (frame[lhs] != frame[rhs]) as i64`.
    Neq {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// Fused compare-and-branch: transfer to `target` when `cmp(lhs, rhs)` is
    /// FALSE (the `JumpIfFalse`-on-a-fresh-comparison shape); fall through
    /// when TRUE. No comparison value is materialized — the adapter proves
    /// the scratch dead before fusing.
    Branch {
        /// The comparison.
        cmp: Cmp,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
        /// Micro-op index to transfer to when the comparison is FALSE.
        target: usize,
    },
    /// `frame[dst] = frame[lhs] & frame[rhs]` (bitwise; logical for 0/1 Bools).
    BitAnd {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] | frame[rhs]` (bitwise; logical for 0/1 Bools).
    BitOr {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs] ^ frame[rhs]`.
    BitXor {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs].wrapping_shl(frame[rhs] as u32)` — the
    /// kernel's locked shift spec (count truncates to u32, masks mod 64).
    Shl {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = frame[lhs].wrapping_shr(frame[rhs] as u32)` (arithmetic).
    Shr {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `frame[dst] = !frame[src]` — bitwise NOT (the kernel's Int `not`).
    NotInt {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// `frame[dst] = frame[src] ^ 1` — logical NOT over 0/1 Bools.
    NotBool {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// Float ops: f64 values as raw bits in the i64 slots. Arithmetic is
    /// IEEE; equality is the kernel's EPSILON rule.
    AddF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `bits(f(lhs) - f(rhs))`.
    SubF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `bits(f(lhs) * f(rhs))`.
    MulF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `bits(f(lhs) / f(rhs))`; divisor `== 0.0` side-exits like [`MicroOp::Div`].
    DivF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `(f(lhs) < f(rhs)) as i64` (IEEE, NaN → 0).
    LtF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `(f(lhs) > f(rhs)) as i64`.
    GtF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `(f(lhs) <= f(rhs)) as i64`.
    LtEqF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// `(f(lhs) >= f(rhs)) as i64`.
    GtEqF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// The kernel's epsilon equality as a value.
    EqF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// Negated epsilon equality as a value.
    NeqF {
        /// Destination slot.
        dst: Slot,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
    },
    /// Fused FLOAT compare-and-branch: transfer to `target` when
    /// `cmp(f(lhs), f(rhs))` is FALSE — which, under IEEE, includes every
    /// NaN-unordered comparison (matching the kernel's relations exactly).
    BranchF {
        /// The comparison.
        cmp: Cmp,
        /// Left operand slot.
        lhs: Slot,
        /// Right operand slot.
        rhs: Slot,
        /// Micro-op index to transfer to when the comparison is FALSE.
        target: usize,
    },
    /// `frame[dst] = bits(frame[src] as f64)` — the kernel's Int→Float
    /// promotion.
    IntToFloat {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// `frame[dst] = bits(sqrt(f(frame[src])))` — the kernel's Float sqrt
    /// builtin (IEEE: negative → NaN, no error path).
    SqrtF {
        /// Destination slot.
        dst: Slot,
        /// Source slot.
        src: Slot,
    },
    /// Map get through the pinned storage: an Int hit lands in `dst`; a
    /// miss or non-Int value side-exits (the replay raises the kernel's
    /// exact error or re-runs the boxed path).
    MapGet {
        /// Destination slot.
        dst: Slot,
        /// Key slot.
        key: Slot,
        /// The pinned `*mut MapStorage` slot.
        map_slot: Slot,
        /// Runtime helper address (indirect call, no relocation).
        helper_addr: i64,
    },
    /// Map insert (always succeeds; same storage, same iteration order).
    MapSet {
        /// Value slot.
        src: Slot,
        /// Key slot.
        key: Slot,
        /// The pinned `*mut MapStorage` slot.
        map_slot: Slot,
        /// Runtime helper address.
        helper_addr: i64,
    },
    /// DIRECT self-call (mode A): entry patched post-layout into the
    /// literal pool; static frame-size bound; plain replay deopt.
    CallSelf {
        /// Caller register receiving the result.
        dst: Slot,
        /// First slot of the staged argument window.
        args_start: Slot,
        /// Live-depth cell address.
        depth_addr: i64,
        /// Shared status cell address.
        status_addr: i64,
        /// MY arena-limit slot (the bound source).
        limit_slot: Slot,
        /// The callee's (own) full frame size in slots.
        frame_size: i64,
    },
    /// DIRECT self-call with a FUSED argument copy (Lever A: the pinned-arg
    /// self-call ABI). Same model as [`MicroOp::CallSelf`], but it stages the
    /// `arg_count` contiguous scalar arguments from `src_start..` into the
    /// callee window itself rather than relying on `arg_count` separate
    /// `Move` pieces before the call — one piece replaces `arg_count + 1`.
    /// Emitted only for all-scalar (Int/Bool/Float) self-calls; list-param
    /// and pin-triple staging stay on the per-`Move` path.
    CallSelfCopy {
        /// Caller register receiving the result.
        dst: Slot,
        /// First slot of the staged argument window (the callee frame base).
        args_start: Slot,
        /// First slot of the SOURCE argument block in this frame.
        src_start: Slot,
        /// Number of contiguous scalar arguments to copy.
        arg_count: u16,
        /// Live-depth cell address.
        depth_addr: i64,
        /// Shared status cell address.
        status_addr: i64,
        /// MY arena-limit slot (the bound source).
        limit_slot: Slot,
        /// The callee's (own) full frame size in slots.
        frame_size: i64,
    },
    /// Allocate a registry-owned fresh list and plant its pin triple.
    NewList {
        /// Pin handle slot.
        vec_slot: Slot,
        /// Pin buffer-pointer slot.
        ptr_slot: Slot,
        /// Pin length slot.
        len_slot: Slot,
        /// Runtime allocator address.
        helper_addr: i64,
    },
    /// Plant a pin triple from a list handle already in `handle_slot`
    /// (a self-call's returned list, seen from the caller).
    ListTriple {
        /// Slot holding the `*mut Vec<i64>` handle.
        handle_slot: Slot,
        /// Pin handle slot.
        vec_slot: Slot,
        /// Pin buffer-pointer slot.
        ptr_slot: Slot,
        /// Pin length slot.
        len_slot: Slot,
        /// Runtime helper address.
        helper_addr: i64,
    },
    /// Map membership into `dst` (0/1).
    MapHas {
        /// Destination slot.
        dst: Slot,
        /// Key slot.
        key: Slot,
        /// The pinned `*mut MapStorage` slot.
        map_slot: Slot,
        /// Runtime helper address.
        helper_addr: i64,
    },
    /// Native SELF-CALL through the program's entry table (callee frame
    /// windowed at `base + args_start` like the VM). Missing entry, a
    /// MAX_CALL_DEPTH crossing, or arena overflow side-exits the WHOLE
    /// native stack via the shared status cell.
    Call {
        /// Result slot.
        dst: Slot,
        /// Callee frame offset within the caller's frame.
        args_start: Slot,
        /// Address of the [entry, regcount] table slot pair.
        table_addr: i64,
        /// Address of the shared live-depth cell.
        depth_addr: i64,
        /// Address of the shared deopt-status cell.
        status_addr: i64,
        /// Frame slot holding the arena END address.
        limit_slot: Slot,
        /// MAX_CALL_DEPTH.
        depth_limit: i64,
    },
    /// Pinned-array push. The contiguous regalloc backend lowers this to an
    /// INLINE fast path — `len < cap ? buffer[len++] = v` straight in registers,
    /// reading the capacity from the live `*mut Vec` (a codegen-probed field
    /// offset, never a hardcoded layout) — and calls the runtime helper ONLY on
    /// the cold realloc boundary (`len == cap`), where it reallocates and
    /// refreshes the pinned pointer/length slots. The per-piece stencil tier
    /// always calls the helper.
    ArrPush {
        /// Value slot.
        src: Slot,
        /// Frame slot holding the `*mut Vec` handle.
        vec_slot: Slot,
        /// Pinned pointer slot to refresh.
        ptr_slot: Slot,
        /// Pinned length slot to refresh.
        len_slot: Slot,
        /// The runtime helper's address.
        helper_addr: i64,
        /// 1-BYTE (`Seq of Bool`) element: the inline fast-path store writes the
        /// boolean normalization `(v != 0) as u8` (matching `logos_rt_push_bool`),
        /// not 8 raw bytes. `false` for Int/Float (8-byte raw bits).
        byte: bool,
        /// 4-byte Int element (`ListRepr::IntsI32`): the inline fast-path store
        /// TRUNCATES the value to its low 4 bytes (lossless under the narrowing
        /// proof), matching `logos_rt_push_i32`. Mutually exclusive with `byte`.
        narrow32: bool,
    },
    /// Pinned-array in-place clear through a runtime helper: truncate the buffer
    /// to empty (keep capacity) and refresh the pinned pointer/length slots
    /// (length → 0). Lowers an in-region `NewEmptyList` on a pinned array; an
    /// in-place mutation, so a PRECISE region resumes over it soundly.
    ListClear {
        /// Frame slot holding the `*mut Vec` handle.
        vec_slot: Slot,
        /// Pinned pointer slot to refresh.
        ptr_slot: Slot,
        /// Pinned length slot to refresh (set to 0).
        len_slot: Slot,
        /// The runtime helper's address.
        helper_addr: i64,
    },
    /// Pinned MUTABLE-Text append (`Set text to text + <s>`) through a runtime
    /// helper. `text_handle_slot` holds a `*mut Value` to the VM register cell
    /// holding the accumulator (planted at region entry); the helper grows the
    /// accumulator THROUGH that cell with EXACTLY the VM's `add_assign`
    /// semantics — in place when the `Rc<String>` is sole-owned, copy-on-write
    /// (a fresh `Rc` written back into the cell, the alias untouched) otherwise.
    /// `src` is the appended operand (a 1-char frame byte, or a baked constant
    /// slice). Bit-identical to the tree-walker for every alias case. The buffer
    /// reaches into the VM register file, so — like the other helper calls — it
    /// clobbers the caller-saved registers (the residents are spilled/reloaded).
    StrAppend {
        /// Frame slot holding the `*mut Value` accumulator-cell handle.
        text_handle_slot: Slot,
        /// The appended source operand.
        src: StrSrc,
        /// The runtime helper's address (`logos_rt_str_append`).
        helper_addr: i64,
    },
    /// COUNT OVERLAPPING SUBSTRING MATCHES through a runtime helper — the whole
    /// naive-search nest (string_search) collapsed to one piece. The helper
    /// reads the pinned haystack/needle byte buffers, counts overlapping needle
    /// occurrences over the outer range `[frame[i_slot], textLen - needleLen + 1]`
    /// (1-based, matching the nest), ADDS the count into `frame[count_slot]`, and
    /// advances `frame[i_slot]` to the loop's exit value. On a recoverable
    /// disagreement (a checked needle index past the needle buffer) it touches
    /// nothing and takes the deopt continuation, so the VM replays the exact
    /// nest on bytecode and raises the same error. Holes: 0 = haystack ptr slot,
    /// 1 = haystack len slot, 2 = needle ptr slot, 3 = needle len slot, 4 =
    /// needleLen value slot, 5 = i (start) slot, 6 = count accumulator slot,
    /// 7 = helper address.
    MemMem {
        /// Frame slot holding the pinned haystack buffer pointer.
        h_ptr_slot: Slot,
        /// Frame slot holding the pinned haystack length.
        h_len_slot: Slot,
        /// Frame slot holding the pinned needle buffer pointer.
        n_ptr_slot: Slot,
        /// Frame slot holding the pinned needle length.
        n_len_slot: Slot,
        /// Frame slot holding the program's `needleLen` value (inner bound).
        needle_len_slot: Slot,
        /// Frame slot holding the 1-based outer index `i` (start; written to the
        /// exit value on success).
        i_slot: Slot,
        /// Frame slot holding the running `count` accumulator (added into).
        count_slot: Slot,
        /// The runtime helper's address.
        helper_addr: i64,
    },
    /// Pinned-array load: `frame[dst] = buffer[frame[idx] - 1]` (1-based;
    /// bits regardless of element kind). Out-of-bounds (incl. 0/negative)
    /// SIDE-EXITS before any effect.
    ArrLoad {
        /// Destination slot.
        dst: Slot,
        /// Index slot (1-based value).
        idx: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// 1-byte elements (Bool buffers) instead of 8-byte.
        byte: bool,
        /// 4-byte SIGN-EXTENDED Int elements (`ListRepr::IntsI32`): the load is a
        /// `movsxd` widening the stored `i32` to a full i64 (lossless). Mutually
        /// exclusive with `byte`; both `false` ⇒ the default 8-byte element.
        narrow32: bool,
        /// Bounds-checked (`true`) or elided (`false`, the Oracle proved
        /// the index in range — V8/LLVM bounds-check elimination). Unchecked
        /// loads have no out-of-bounds continuation.
        checked: bool,
    },
    /// FUSED affine-index load: `frame[dst] = buffer[idx - 1]` where the 1-based
    /// `idx = (frame[a] OP frame[b]).wrapping_add(const_offset)` (or
    /// `frame[a] + const_offset` for [`AffOp::None`]) is computed INSIDE the
    /// stencil — the peephole collapses the index-arithmetic chain
    /// (`<binop>(t = a OP b); [Add(t2 = t + Kc);] ArrLoad(t2)`, or
    /// `Add(t = a + Kc); ArrLoad(t)`) plus the load into ONE piece when the
    /// intermediate index temps are single-use scratch. The dominant indexed
    /// inner loops (matrix_mult `i*n+k+1`, knapsack `w - wi + 1`, the `w + 1`
    /// row read) lose worst to V8 precisely on this per-op dispatch; folding the
    /// 2-3 index ops into the load is a direct dispatch-reduction win. Out of
    /// bounds (incl. the 0/negative index the wrapping-sub trick catches)
    /// SIDE-EXITS before any effect, exactly like [`MicroOp::ArrLoad`].
    ArrLoadAffine {
        /// Destination slot.
        dst: Slot,
        /// First index operand slot.
        a: Slot,
        /// The index op (`None` = single slot + const; else two-slot binop).
        op: AffOp,
        /// Second index operand slot (ignored for [`AffOp::None`]).
        b: Slot,
        /// The constant folded into the index (the trailing `+ Kc`; `0` if none).
        const_offset: i64,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// Bounds-checked (`true`) or elided (`false`, the Oracle proved the
        /// computed index in range). Unchecked has no out-of-bounds continuation.
        checked: bool,
    },
    /// FUSED two-load float binop: `frame[dst] = bits(f(buf[i-1]) <op>
    /// f(buf[j-1]))`, BOTH elements from the SAME pinned 8-byte buffer (one
    /// pointer slot, one length slot). The peephole collapses `ArrLoad(t1 =
    /// arr[i]); ArrLoad(t2 = arr[j]); {Add,Sub,Mul}F(dst = t1 OP t2)` when both
    /// loads hit the same array and the two scratch loads are single-use — so
    /// the two f64s never round-trip through the frame. EITHER index out of
    /// bounds SIDE-EXITS before any effect, exactly like [`MicroOp::ArrLoad`].
    ArrLoad2F {
        /// Destination slot.
        dst: Slot,
        /// First index slot (1-based value).
        i: Slot,
        /// Second index slot (1-based value).
        j: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// The float operation applied to the two loaded values.
        op: FOp,
    },
    /// FUSED two-load INTEGER binop: `frame[dst] = a[i-1] <op> b[j-1]`, the two
    /// 8-byte elements loaded from TWO (possibly distinct) pinned int buffers
    /// (each with its own pointer + length slot). The peephole collapses
    /// `ArrLoad(t1 = a[i]); ArrLoad(t2 = b[j]); {Add,Sub,Mul}(dst = t1 OP t2)`
    /// — the matrix-multiply / dot-product idiom `c += a[..] * b[..]` — when the
    /// two scratch loads are single-use, so the loaded i64s never round-trip the
    /// frame and three dispatches collapse to one. EITHER index out of bounds
    /// SIDE-EXITS before any effect, exactly like [`MicroOp::ArrLoad`]. The two
    /// buffers may be the SAME (one self-referential dot product) or distinct.
    ArrLoad2 {
        /// Destination slot.
        dst: Slot,
        /// First index slot (1-based value), addressing buffer `a`.
        i: Slot,
        /// Second index slot (1-based value), addressing buffer `b`.
        j: Slot,
        /// Frame slot holding the pinned pointer of the first buffer (`a`).
        ptr_a: Slot,
        /// Frame slot holding the pinned length of the first buffer (`a`).
        len_a: Slot,
        /// Frame slot holding the pinned pointer of the second buffer (`b`).
        ptr_b: Slot,
        /// Frame slot holding the pinned length of the second buffer (`b`).
        len_b: Slot,
        /// The integer operation applied to the two loaded values.
        op: IOp,
        /// Bounds-checked (`true`) or elided (`false`, the Oracle proved both
        /// indices in range). Unchecked has no out-of-bounds continuation.
        checked: bool,
    },
    /// Pinned-array store: `buffer[frame[idx] - 1] = frame[src]`; bounds
    /// side-exit BEFORE the store.
    ArrStore {
        /// Source slot.
        src: Slot,
        /// Index slot (1-based value).
        idx: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// 1-byte elements (Bool buffers; stores normalize to 0/1).
        byte: bool,
        /// 4-byte SIGN-EXTENDED Int elements (`ListRepr::IntsI32`, the VM's
        /// `LOGOS_NARROW_VM` half-width buffers): a store TRUNCATES the value to
        /// its low 4 bytes (lossless under the narrowing proof). Mutually
        /// exclusive with `byte`; both `false` ⇒ the default 8-byte element.
        narrow32: bool,
        /// Bounds-checked (`true`) or elided (`false`, the Oracle proved
        /// the index in range). Unchecked stores have no side-exit.
        checked: bool,
    },
    /// FUSED read-modify-write on a pinned 8-byte int array:
    /// `buffer[frame[idx] - 1] = buffer[frame[idx] - 1] <op> frame[operand]`
    /// in ONE stencil. The peephole collapses `ArrLoad(t = arr[idx]);
    /// <int ALU>(t2 = t OP operand); ArrStore(arr[idx] = t2)` when both array
    /// ops hit the SAME pinned buffer + index and the two scratch values
    /// (`t`, `t2`) are single-use — so the element never round-trips the frame
    /// and ONE bounds check covers the load+store. Out-of-bounds (incl.
    /// 0/negative) SIDE-EXITS before any effect, exactly like [`MicroOp::ArrStore`].
    ArrRMW {
        /// Index slot (1-based value).
        idx: Slot,
        /// Operand slot (the RHS of the op; `frame[operand]`).
        operand: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// The integer operation applied in place.
        op: RmwOp,
        /// Bounds-checked (`true`) or elided (`false`). Unchecked has no
        /// out-of-bounds continuation.
        checked: bool,
    },
    /// FUSED conditional adjacent swap on a pinned 8-byte int array:
    /// `let a = buf[idx1-1]; let b = buf[idx2-1]; if cmp(a, b) { buf[idx1-1] = b;
    /// buf[idx2-1] = a }` in ONE stencil. The peephole collapses the sort
    /// inner-loop idiom `ArrLoad(a,i1); ArrLoad(b,i2); Branch(cmp,a,b,skip);
    /// ArrStore(i1,b); ArrStore(i2,a)` (bubble/insertion/quick sort) — 5 ops → 1,
    /// the two loaded values never round-trip the frame, the compare-branch is
    /// gone. The swap is ATOMIC (both writes or neither); a checked variant
    /// bounds-checks BOTH indices up front and SIDE-EXITS before any effect, the
    /// unchecked variant (Oracle-proven indices) has no side-exit.
    ArrCondSwap {
        /// First index slot (1-based value).
        idx1: Slot,
        /// Second index slot (1-based value).
        idx2: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// Swap when `cmp(buf[idx1-1], buf[idx2-1])` is TRUE.
        cmp: Cmp,
        /// Bounds-checked (`true`) or elided (`false`).
        checked: bool,
    },
    /// FUSED unconditional adjacent swap on a pinned 8-byte int array:
    /// `let a = buf[idx1-1]; let b = buf[idx2-1]; buf[idx1-1] = b; buf[idx2-1] =
    /// a` in ONE stencil. The peephole collapses the `Let tmp = arr[i]; arr[i] =
    /// arr[j]; arr[j] = tmp` exchange (quicksort/heap_sort/mergesort partition &
    /// sift) — `ArrLoad(a,i); ArrLoad(b,j); ArrStore(i,b); ArrStore(j,a)` — 4 ops
    /// → 1. The swap is ATOMIC; a checked variant bounds-checks BOTH indices up
    /// front and SIDE-EXITS before any effect.
    ArrSwap {
        /// First index slot (1-based value).
        idx1: Slot,
        /// Second index slot (1-based value).
        idx2: Slot,
        /// Frame slot holding the pinned buffer pointer.
        ptr_slot: Slot,
        /// Frame slot holding the pinned length.
        len_slot: Slot,
        /// Bounds-checked (`true`) or elided (`false`).
        checked: bool,
    },
    /// FUSED float multiply-add: `frame[dst] = bits((f(a) * f(b)) + f(c))` — the
    /// peephole collapses `MulF(t = a*b); AddF(d = t + c)` (the product `t`
    /// single-use) into one stencil, cutting a dispatch on the float arithmetic
    /// chains that dominate the float cluster (nbody's `dz*dz + s`, dot
    /// products). The product and the add round SEPARATELY (`(a*b)+c`, two
    /// roundings) — NOT a hardware single-rounding FMA — so it stays bit-identical
    /// to the unfused `MulF`+`AddF`. Mem-form: every operand reads from the frame,
    /// so it preserves the threaded XMM pins (the caller only fuses when the
    /// operands are frame-resident, keeping it a spill-free win).
    FmaF {
        /// Destination slot.
        dst: Slot,
        /// First product operand slot.
        a: Slot,
        /// Second product operand slot.
        b: Slot,
        /// Addend slot.
        c: Slot,
    },
    /// Unconditional transfer to the micro-op at `target` (an index into the
    /// program; forward or backward).
    Jump {
        /// Micro-op index to transfer to.
        target: usize,
    },
    /// Transfer to `target` when frame[cond] is ZERO; fall through otherwise.
    JumpIfFalse {
        /// Condition slot (zero = jump).
        cond: Slot,
        /// Micro-op index to transfer to.
        target: usize,
    },
    /// Transfer to `target` when frame[cond] is NONZERO; fall through
    /// otherwise (the same brz stencil with swapped continuations).
    JumpIfTrue {
        /// Condition slot (nonzero = jump).
        cond: Slot,
        /// Micro-op index to transfer to.
        target: usize,
    },
    /// Terminate the chain, returning frame[src].
    Return {
        /// Slot whose value is returned.
        src: Slot,
    },
}

/// Compile errors — structural, found before any code is emitted.
#[derive(Debug, PartialEq, Eq)]
pub enum JitCompileError {
    /// The program is empty.
    Empty,
    /// Execution can run off the end: the final op must be Return or Jump.
    FallsOffTheEnd,
    /// A jump target is outside the program.
    BadJumpTarget {
        /// Index of the offending op.
        op_index: usize,
        /// The out-of-range target.
        target: usize,
    },
    /// Assembly failed (missing hole/patch/map errors).
    Assembly(String),
    /// An op the per-piece stencil tier has no lowering for (e.g.
    /// [`MicroOp::StrAppend`], which is emitted ONLY for the contiguous regalloc
    /// backend). The caller declines (falls back to bytecode), so this is never a
    /// hard failure — it just routes around the stencil tier.
    Unsupported(&'static str),
}

impl std::fmt::Display for JitCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitCompileError::Empty => write!(f, "jit: empty program"),
            JitCompileError::FallsOffTheEnd => {
                write!(f, "jit: the final op must be Return or Jump")
            }
            JitCompileError::BadJumpTarget { op_index, target } => {
                write!(f, "jit: op {op_index} jumps to {target}, outside the program")
            }
            JitCompileError::Assembly(e) => write!(f, "jit: assembly failed: {e}"),
            JitCompileError::Unsupported(name) => {
                write!(f, "jit: the stencil tier has no lowering for {name}")
            }
        }
    }
}

impl std::error::Error for JitCompileError {}

/// What one chain run produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainOutcome {
    /// The chain reached its Return; this is the returned value.
    Return(i64),
    /// A checked op side-exited; the payload is the RAW status-cell value.
    /// `1` is the plain marker (replay from the entry state — every effect
    /// confined to the private frame). `(pc << 2) | 3` is a PRECISE exit:
    /// effects already landed, resume at bytecode `pc`.
    Deopt(i64),
}

impl ChainOutcome {
    /// The returned value; panics on [`ChainOutcome::Deopt`] — for tests and
    /// callers of chains that contain no checked ops.
    pub fn expect_return(self) -> i64 {
        match self {
            ChainOutcome::Return(v) => v,
            ChainOutcome::Deopt(_) => panic!("chain side-exited (deopt) — no return value"),
        }
    }

    /// True when the run side-exited.
    pub fn is_deopt(&self) -> bool {
        matches!(self, ChainOutcome::Deopt(_))
    }
}

/// A compiled chain plus its side-exit status cell (present only when the
/// program contains checked ops). The deopt stencil stores 1 through the
/// patched cell address; [`CompiledChain::run_with_frame`] reads-and-resets
/// it after every run.
#[derive(Debug)]
pub struct CompiledChain {
    chain: JitChain,
    status: Option<std::sync::Arc<AtomicI64>>,
    /// Extra cells the chain's machine code reads by baked address and that must
    /// therefore outlive the chain (the contiguous FUNCTION backend's self-call
    /// ENTRY cell: an `Arc<AtomicI64>` holding this chain's own entry address,
    /// loaded by every `CallSelf`/`CallSelfCopy` and written after mapping). The
    /// run/deopt contract is unchanged — these are pure keep-alives.
    #[allow(dead_code)]
    keepalive: Vec<std::sync::Arc<AtomicI64>>,
}

impl CompiledChain {
    /// Wrap a raw `JitChain` (from the contiguous regalloc backend) and its
    /// shared status cell as a runnable `CompiledChain` — same run/deopt
    /// contract as a stencil chain.
    pub(crate) fn from_chain(
        chain: JitChain,
        status: Option<std::sync::Arc<AtomicI64>>,
    ) -> Self {
        CompiledChain { chain, status, keepalive: Vec::new() }
    }

    /// Wrap a `JitChain` plus its status cell AND any extra cells the code reads
    /// by baked address (the self-call entry cell), which the `CompiledChain`
    /// then keeps alive for the chain's whole lifetime.
    pub(crate) fn from_chain_keepalive(
        chain: JitChain,
        status: Option<std::sync::Arc<AtomicI64>>,
        keepalive: Vec<std::sync::Arc<AtomicI64>>,
    ) -> Self {
        CompiledChain { chain, status, keepalive }
    }

    /// Run over the given frame (slot 0 = VM register 0, …) with a fresh
    /// operand stack.
    pub fn run_with_frame(&self, frame: &mut [i64]) -> ChainOutcome {
        let v = self.chain.run_with_frame(frame);
        if let Some(cell) = &self.status {
            let raw = cell.swap(0, Ordering::Relaxed);
            if raw != 0 {
                return ChainOutcome::Deopt(raw);
            }
        }
        ChainOutcome::Return(v)
    }

    /// The mapped code+pool bytes (diagnostics/tests).
    pub fn bytes(&self) -> &[u8] {
        self.chain.bytes()
    }

    /// See [`crate::buffer::JitChain::patch_marked`].
    pub fn patch_marked(&self, value: u64) -> Result<(), crate::JitError> {
        self.chain.patch_marked(value)
    }

    /// See [`crate::buffer::JitChain::has_patch_marks`].
    pub fn has_patch_marks(&self) -> bool {
        self.chain.has_patch_marks()
    }

    /// The runtime base address (diagnostics/tests).
    pub fn base(&self) -> u64 {
        self.chain.base()
    }

    /// Stencil pieces in the chain (diagnostics/tests).
    pub fn piece_count(&self) -> usize {
        self.chain.piece_count()
    }
}

/// Lower a straight-line micro-op program into one executable stencil chain.
///
/// Emit ONE memory-form piece (an op outside the variant families) with
/// explicit continuation and deopt labels — the pinned compiler's bridge
/// to the classic stencils. Pinned operands were spilled before this
/// piece and pinned destinations reload after it.
/// The fused read-modify-write stencil for an op + bounds-check mode. The
/// checked twins side-exit through cont 1 on out-of-bounds; the unchecked `_u`
/// twins have no length hole and no out-of-bounds continuation.
fn rmw_stencil(op: RmwOp, checked: bool) -> &'static crate::Stencil {
    match (op, checked) {
        (RmwOp::Add, true) => &ST_ARRRMW_ADD,
        (RmwOp::Add, false) => &ST_ARRRMW_ADD_U,
        (RmwOp::Sub, true) => &ST_ARRRMW_SUB,
        (RmwOp::Sub, false) => &ST_ARRRMW_SUB_U,
        (RmwOp::Mul, true) => &ST_ARRRMW_MUL,
        (RmwOp::Mul, false) => &ST_ARRRMW_MUL_U,
        (RmwOp::And, true) => &ST_ARRRMW_AND,
        (RmwOp::And, false) => &ST_ARRRMW_AND_U,
        (RmwOp::Or, true) => &ST_ARRRMW_OR,
        (RmwOp::Or, false) => &ST_ARRRMW_OR_U,
        (RmwOp::Xor, true) => &ST_ARRRMW_XOR,
        (RmwOp::Xor, false) => &ST_ARRRMW_XOR_U,
        (RmwOp::AddF, true) => &ST_ARRRMW_ADDF,
        (RmwOp::AddF, false) => &ST_ARRRMW_ADDF_U,
        (RmwOp::SubF, true) => &ST_ARRRMW_SUBF,
        (RmwOp::SubF, false) => &ST_ARRRMW_SUBF_U,
        (RmwOp::MulF, true) => &ST_ARRRMW_MULF,
        (RmwOp::MulF, false) => &ST_ARRRMW_MULF_U,
    }
}

/// The fused two-buffer integer-load binop stencil for an op + bounds-check
/// mode. The checked twins side-exit through cont 1 when EITHER index is out of
/// bounds; the unchecked `_u` twins have no length holes and no continuation.
fn ld2_int_stencil(op: IOp, checked: bool) -> &'static crate::Stencil {
    match (op, checked) {
        (IOp::Add, true) => &ST_ARRLD2_ADD,
        (IOp::Add, false) => &ST_ARRLD2_ADD_U,
        (IOp::Sub, true) => &ST_ARRLD2_SUB,
        (IOp::Sub, false) => &ST_ARRLD2_SUB_U,
        (IOp::Mul, true) => &ST_ARRLD2_MUL,
        (IOp::Mul, false) => &ST_ARRLD2_MUL_U,
    }
}

/// The fused affine-index load stencil for an index op + bounds-check mode. The
/// checked twins side-exit through cont 1 when the COMPUTED index is out of
/// bounds; the unchecked `_u` twins have no length hole and no continuation.
/// The pinned-array LOAD stencil for an element width: 1-byte (Bool, zero-ext),
/// 4-byte (`IntsI32`, sign-ext), or the default 8-byte (Int/Float raw bits).
/// `byte` and `narrow32` are mutually exclusive (the adapter never sets both).
fn arrld_stencil(byte: bool, narrow32: bool, checked: bool) -> &'static crate::Stencil {
    match (byte, narrow32, checked) {
        (true, _, true) => &ST_ARRLDB,
        (true, _, false) => &ST_ARRLDB_U,
        (_, true, true) => &ST_ARRLD_I32,
        (_, true, false) => &ST_ARRLD_I32_U,
        (false, false, true) => &ST_ARRLD,
        (false, false, false) => &ST_ARRLD_U,
    }
}

/// The pinned-array STORE stencil for an element width (see [`arrld_stencil`]).
fn arrst_stencil(byte: bool, narrow32: bool, checked: bool) -> &'static crate::Stencil {
    match (byte, narrow32, checked) {
        (true, _, true) => &ST_ARRSTB,
        (true, _, false) => &ST_ARRSTB_U,
        (_, true, true) => &ST_ARRST_I32,
        (_, true, false) => &ST_ARRST_I32_U,
        (false, false, true) => &ST_ARRST,
        (false, false, false) => &ST_ARRST_U,
    }
}

fn affine_stencil(op: AffOp, checked: bool) -> &'static crate::Stencil {
    match (op, checked) {
        (AffOp::None, true) => &ST_ARRLDAFF_NONE,
        (AffOp::None, false) => &ST_ARRLDAFF_NONE_U,
        (AffOp::Add, true) => &ST_ARRLDAFF_ADD,
        (AffOp::Add, false) => &ST_ARRLDAFF_ADD_U,
        (AffOp::Sub, true) => &ST_ARRLDAFF_SUB,
        (AffOp::Sub, false) => &ST_ARRLDAFF_SUB_U,
        (AffOp::Mul, true) => &ST_ARRLDAFF_MUL,
        (AffOp::Mul, false) => &ST_ARRLDAFF_MUL_U,
    }
}

/// The conditional-swap stencil for a comparison + bounds-check mode. The
/// peephole only ever emits the four orderings (Eq/NotEq swaps are nonsensical).
fn condswap_stencil(cmp: Cmp, checked: bool) -> &'static crate::Stencil {
    match (cmp, checked) {
        (Cmp::Gt, true) => &ST_ARRCONDSWAP_GT,
        (Cmp::Gt, false) => &ST_ARRCONDSWAP_GT_U,
        (Cmp::Lt, true) => &ST_ARRCONDSWAP_LT,
        (Cmp::Lt, false) => &ST_ARRCONDSWAP_LT_U,
        (Cmp::GtEq, true) => &ST_ARRCONDSWAP_GE,
        (Cmp::GtEq, false) => &ST_ARRCONDSWAP_GE_U,
        (Cmp::LtEq, true) => &ST_ARRCONDSWAP_LE,
        (Cmp::LtEq, false) => &ST_ARRCONDSWAP_LE_U,
        (Cmp::Eq | Cmp::NotEq, _) => unreachable!("cond-swap peephole never emits Eq/NotEq"),
    }
}

fn emit_mem_form(
    buf: &mut JitBuffer,
    op: &MicroOp,
    next: crate::buffer::Label,
    deopt_piece: usize,
    status: &Option<std::sync::Arc<AtomicI64>>,
) {
    match *op {
        MicroOp::Div { dst, lhs, rhs } | MicroOp::Mod { dst, lhs, rhs } => {
            let stencil = if matches!(op, MicroOp::Div { .. }) { &ST_DIV3C } else { &ST_MOD3C };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, rhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                    HoleValue::Cont(1, buf.label(deopt_piece)),
                ],
            );
        }
        MicroOp::DivPow2 { dst, lhs, k } => {
            buf.push_stencil(
                &ST_DIVPOW2,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, k as i64),
                    HoleValue::Const(2, (1i64 << k) - 1),
                    HoleValue::Const(3, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::MagicDivU { dst, lhs, magic, more, mul_back } => {
            buf.push_stencil(
                &ST_MAGICDIV,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, magic as i64),
                    HoleValue::Const(2, more as i64),
                    HoleValue::Const(3, mul_back),
                    HoleValue::Const(4, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::AddF { dst, lhs, rhs }
        | MicroOp::SubF { dst, lhs, rhs }
        | MicroOp::MulF { dst, lhs, rhs } => {
            let stencil = match op {
                MicroOp::AddF { .. } => &ST_ADDF3,
                MicroOp::SubF { .. } => &ST_SUBF3,
                _ => &ST_MULF3,
            };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, rhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::DivF { dst, lhs, rhs } => {
            buf.push_stencil(
                &ST_DIVF3C,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, rhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                    HoleValue::Cont(1, buf.label(deopt_piece)),
                ],
            );
        }
        MicroOp::LtF { dst, lhs, rhs }
        | MicroOp::LtEqF { dst, lhs, rhs }
        | MicroOp::EqF { dst, lhs, rhs }
        | MicroOp::NeqF { dst, lhs, rhs } => {
            let stencil = match op {
                MicroOp::LtF { .. } => &ST_LTF3,
                MicroOp::LtEqF { .. } => &ST_LEF3,
                MicroOp::EqF { .. } => &ST_EQF3,
                _ => &ST_NEF3,
            };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, rhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        // a > b ⇔ b < a; a >= b ⇔ b <= a (Lever 2a): a VALUE-form float ordering
        // compare lowers mem-form by SWAPPING operands and reusing the LtF/LeF
        // stencil — IEEE-exact, NaN-false on both sides of the swap, bit-identical
        // to the tree-walker. (Float EQUALITY is epsilon-fuzzy and stays on its
        // own EqF/NeqF path — never swapped here.) This gives the pinned compiler
        // a lowering for `GtF`/`GtEqF`, so a region carrying a value-form float
        // `>`/`>=` no longer has to drop ALL its XMM pins.
        MicroOp::GtF { dst, lhs, rhs } | MicroOp::GtEqF { dst, lhs, rhs } => {
            let stencil = match op {
                MicroOp::GtF { .. } => &ST_LTF3,
                _ => &ST_LEF3,
            };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, rhs as i64),
                    HoleValue::Const(1, lhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src }
        | MicroOp::IntToFloat { dst, src } | MicroOp::SqrtF { dst, src } => {
            let stencil = match op {
                MicroOp::NotInt { .. } => &ST_NOTI2,
                MicroOp::NotBool { .. } => &ST_NOTB2,
                MicroOp::SqrtF { .. } => &ST_SQRTF2,
                _ => &ST_I2F2,
            };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, src as i64),
                    HoleValue::Const(1, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::Call {
            dst,
            args_start,
            table_addr,
            depth_addr,
            status_addr,
            limit_slot,
            depth_limit,
        } => {
            debug_assert!(status.is_some(), "calls require the status cell");
            buf.push_stencil(
                &ST_CALL,
                &[
                    HoleValue::Const(0, table_addr),
                    HoleValue::Const(1, args_start as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Const(3, depth_addr),
                    HoleValue::Const(4, status_addr),
                    HoleValue::Const(5, limit_slot as i64),
                    HoleValue::Const(6, depth_limit),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::MapGet { dst, key, map_slot, helper_addr } => {
            buf.push_stencil(
                &ST_MAPHGET,
                &[
                    HoleValue::Const(0, map_slot as i64),
                    HoleValue::Const(1, key as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Const(3, helper_addr),
                    HoleValue::Cont(0, next),
                    HoleValue::Cont(1, buf.label(deopt_piece)),
                ],
            );
        }
        MicroOp::MapSet { src, key, map_slot, helper_addr } => {
            buf.push_stencil(
                &ST_MAPHSET,
                &[
                    HoleValue::Const(0, map_slot as i64),
                    HoleValue::Const(1, key as i64),
                    HoleValue::Const(2, src as i64),
                    HoleValue::Const(3, helper_addr),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::MemMem {
            h_ptr_slot,
            h_len_slot,
            n_ptr_slot,
            n_len_slot,
            needle_len_slot,
            i_slot,
            count_slot,
            helper_addr,
        } => {
            debug_assert!(status.is_some(), "memmem requires the status cell for its deopt exit");
            buf.push_stencil(
                &ST_MEMMEM,
                &[
                    HoleValue::Const(0, h_ptr_slot as i64),
                    HoleValue::Const(1, h_len_slot as i64),
                    HoleValue::Const(2, n_ptr_slot as i64),
                    HoleValue::Const(3, n_len_slot as i64),
                    HoleValue::Const(4, needle_len_slot as i64),
                    HoleValue::Const(5, i_slot as i64),
                    HoleValue::Const(6, count_slot as i64),
                    HoleValue::Const(7, helper_addr),
                    HoleValue::Cont(0, next),
                    HoleValue::Cont(1, buf.label(deopt_piece)),
                ],
            );
        }
        MicroOp::CallSelf { dst, args_start, depth_addr, status_addr, limit_slot, frame_size } => {
            buf.push_stencil(
                &ST_CALL_SELF,
                &[
                    HoleValue::Const(0, 0),
                    HoleValue::Const(1, args_start as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Const(3, depth_addr),
                    HoleValue::Const(4, status_addr),
                    HoleValue::Const(5, limit_slot as i64),
                    HoleValue::Const(6, frame_size),
                    HoleValue::Cont(0, next),
                ],
            );
            buf.mark_patch_hole(0);
        }
        MicroOp::CallSelfCopy {
            dst,
            args_start,
            src_start,
            arg_count,
            depth_addr,
            status_addr,
            limit_slot,
            frame_size,
        } => {
            buf.push_stencil(
                &ST_CALL_SELF_COPY,
                &[
                    HoleValue::Const(0, 0),
                    HoleValue::Const(1, args_start as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Const(3, depth_addr),
                    HoleValue::Const(4, status_addr),
                    HoleValue::Const(5, limit_slot as i64),
                    HoleValue::Const(6, frame_size),
                    HoleValue::Const(7, src_start as i64),
                    HoleValue::Const(8, arg_count as i64),
                    HoleValue::Cont(0, next),
                ],
            );
            buf.mark_patch_hole(0);
        }
        MicroOp::NewList { vec_slot, ptr_slot, len_slot, helper_addr } => {
            buf.push_stencil(
                &ST_ALLOCLIST,
                &[
                    HoleValue::Const(0, vec_slot as i64),
                    HoleValue::Const(1, ptr_slot as i64),
                    HoleValue::Const(2, len_slot as i64),
                    HoleValue::Const(3, helper_addr),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, helper_addr } => {
            buf.push_stencil(
                &ST_LISTTRIPLE,
                &[
                    HoleValue::Const(0, handle_slot as i64),
                    HoleValue::Const(1, vec_slot as i64),
                    HoleValue::Const(2, ptr_slot as i64),
                    HoleValue::Const(3, len_slot as i64),
                    HoleValue::Const(4, helper_addr),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        MicroOp::MapHas { dst, key, map_slot, helper_addr } => {
            buf.push_stencil(
                &ST_MAPHHAS,
                &[
                    HoleValue::Const(0, map_slot as i64),
                    HoleValue::Const(1, key as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Const(3, helper_addr),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        // Array load/store: 1-based, bounds-checked. A failed bound jumps to
        // the shared deopt terminal (region replay / function side-exit).
        // The pinned mem-form path has already spilled the pinned operands
        // (idx/ptr/len/src) to the frame, so these read frame slots exactly
        // like the unpinned chain.
        MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
            if checked {
                buf.push_stencil(
                    arrld_stencil(byte, narrow32, true),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(2, len_slot as i64),
                        HoleValue::Const(3, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    arrld_stencil(byte, narrow32, false),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(3, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrLoadAffine { dst, a, op, b, const_offset, ptr_slot, len_slot, checked } => {
            if checked {
                buf.push_stencil(
                    affine_stencil(op, true),
                    &[
                        HoleValue::Const(0, a as i64),
                        HoleValue::Const(1, b as i64),
                        HoleValue::Const(2, const_offset),
                        HoleValue::Const(3, ptr_slot as i64),
                        HoleValue::Const(4, len_slot as i64),
                        HoleValue::Const(5, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    affine_stencil(op, false),
                    &[
                        HoleValue::Const(0, a as i64),
                        HoleValue::Const(1, b as i64),
                        HoleValue::Const(2, const_offset),
                        HoleValue::Const(3, ptr_slot as i64),
                        HoleValue::Const(5, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrLoad2F { dst, i: ix, j: jx, ptr_slot, len_slot, op } => {
            let stencil = match op {
                FOp::Add => &ST_ARRLD2_ADDF,
                FOp::Sub => &ST_ARRLD2_SUBF,
                FOp::Mul => &ST_ARRLD2_MULF,
            };
            buf.push_stencil(
                stencil,
                &[
                    HoleValue::Const(0, ix as i64),
                    HoleValue::Const(1, jx as i64),
                    HoleValue::Const(2, ptr_slot as i64),
                    HoleValue::Const(3, len_slot as i64),
                    HoleValue::Const(4, dst as i64),
                    HoleValue::Cont(0, next),
                    HoleValue::Cont(1, buf.label(deopt_piece)),
                ],
            );
        }
        MicroOp::ArrLoad2 { dst, i: ix, j: jx, ptr_a, len_a, ptr_b, len_b, op, checked } => {
            if checked {
                buf.push_stencil(
                    ld2_int_stencil(op, true),
                    &[
                        HoleValue::Const(0, ix as i64),
                        HoleValue::Const(1, jx as i64),
                        HoleValue::Const(2, ptr_a as i64),
                        HoleValue::Const(3, len_a as i64),
                        HoleValue::Const(4, ptr_b as i64),
                        HoleValue::Const(5, len_b as i64),
                        HoleValue::Const(6, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    ld2_int_stencil(op, false),
                    &[
                        HoleValue::Const(0, ix as i64),
                        HoleValue::Const(1, jx as i64),
                        HoleValue::Const(2, ptr_a as i64),
                        HoleValue::Const(3, ptr_b as i64),
                        HoleValue::Const(4, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrStore { src, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
            if checked {
                buf.push_stencil(
                    arrst_stencil(byte, narrow32, true),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(2, len_slot as i64),
                        HoleValue::Const(3, src as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    arrst_stencil(byte, narrow32, false),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(3, src as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, op, checked } => {
            if checked {
                buf.push_stencil(
                    rmw_stencil(op, true),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(2, len_slot as i64),
                        HoleValue::Const(3, operand as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    rmw_stencil(op, false),
                    &[
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(3, operand as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrCondSwap { idx1, idx2, ptr_slot, len_slot, cmp, checked } => {
            if checked {
                buf.push_stencil(
                    condswap_stencil(cmp, true),
                    &[
                        HoleValue::Const(0, idx1 as i64),
                        HoleValue::Const(1, idx2 as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Const(3, len_slot as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    condswap_stencil(cmp, false),
                    &[
                        HoleValue::Const(0, idx1 as i64),
                        HoleValue::Const(1, idx2 as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::ArrSwap { idx1, idx2, ptr_slot, len_slot, checked } => {
            if checked {
                buf.push_stencil(
                    &ST_ARRSWAP,
                    &[
                        HoleValue::Const(0, idx1 as i64),
                        HoleValue::Const(1, idx2 as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Const(3, len_slot as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(deopt_piece)),
                    ],
                );
            } else {
                buf.push_stencil(
                    &ST_ARRSWAP_U,
                    &[
                        HoleValue::Const(0, idx1 as i64),
                        HoleValue::Const(1, idx2 as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
        MicroOp::FmaF { dst, a, b, c } => {
            buf.push_stencil(
                &ST_FMAF,
                &[
                    HoleValue::Const(0, a as i64),
                    HoleValue::Const(1, b as i64),
                    HoleValue::Const(2, c as i64),
                    HoleValue::Const(3, dst as i64),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        // Helper-call push; refreshes the pinned ptr/len after a possible
        // realloc (the mem-form path reloads them from the frame after).
        MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, helper_addr, byte: _, narrow32: _ } => {
            buf.push_stencil(
                &ST_PUSH,
                &[
                    HoleValue::Const(0, src as i64),
                    HoleValue::Const(1, vec_slot as i64),
                    HoleValue::Const(2, ptr_slot as i64),
                    HoleValue::Const(3, len_slot as i64),
                    HoleValue::Const(4, helper_addr),
                    HoleValue::Cont(0, next),
                ],
            );
        }
        ref other => unreachable!("emit_mem_form: unsupported op {other:?} (pinned chains exclude it)"),
    }
}

/// REGISTER-THREADING compilation (EXODIA 3.1): up to four frame slots are
/// PINNED into the threaded registers r0..r3 for the whole chain. Pinned
/// operands ride registers through the generated location-variant
/// stencils; ops outside the variant families (Div, arrays, calls, …)
/// spill the pinned operands they read and reload the pinned slots they
/// write, so every memory-form piece still sees a coherent frame.
///
/// Frame-coherence contract: a pinned slot's FRAME cell is stale between
/// its spills — callers must only use pinning where nothing outside the
/// chain reads the frame mid-run (mode-A functions: replay re-enters from
/// the boundary arguments and the result returns by register).
pub fn compile_straightline_pinned(
    ops: &[MicroOp],
    pins: &[u16],
) -> Result<CompiledChain, JitCompileError> {
    compile_straightline_pinned_with(ops, pins, None)
}

/// [`compile_straightline_pinned`] with a SHARED status cell (the function
/// tier's deopt/call seam). Integer/bool slots only — no XMM float pins.
pub fn compile_straightline_pinned_with(
    ops: &[MicroOp],
    pins: &[u16],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
) -> Result<CompiledChain, JitCompileError> {
    compile_straightline_pinned_float(ops, pins, &[], shared_status)
}

/// The full register-threading compiler with BOTH integer pins (`pins`,
/// threaded through the GP registers r0..r3) and FLOAT pins (`fpins`,
/// threaded through the XMM registers f0..f3). The two budgets are
/// independent — the threaded ABI carries `fn(base, sp, r0..r3, f0..f3)`, so
/// base/sp consume 2 of the 6 GP arg registers (leaving r0..r3) while all
/// four XMM pins sit in the 8 free XMM arg registers and never compete with
/// the integer pins.
///
/// A FLOAT slot pinned to `fN` keeps its f64 live in that XMM register across
/// every stencil — the pure float arith variants (V_FBINOP) read and write it
/// directly, and the mem-form float stencils (sqrt, divf, the float compares,
/// array load/store) THREAD f0..f3 unchanged, so any float pin they do not
/// themselves touch survives with no frame traffic. The ones a mem-form op
/// does read/write are spilled before / reloaded after, exactly like the GP
/// pins.
pub fn compile_straightline_pinned_float(
    ops: &[MicroOp],
    pins: &[u16],
    fpins: &[u16],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
) -> Result<CompiledChain, JitCompileError> {
    if pins.is_empty() && fpins.is_empty() {
        return compile_straightline_coded(ops, shared_status, None, 0);
    }
    assert!(pins.len() <= 4, "at most four threaded GP registers");
    assert!(fpins.len() <= 6, "at most six threaded XMM registers (f0..f5)");
    debug_assert!(
        pins.iter().all(|p| !fpins.contains(p)),
        "a slot cannot be both GP- and float-pinned"
    );
    if ops.is_empty() {
        return Err(JitCompileError::Empty);
    }
    if !matches!(ops.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return Err(JitCompileError::FallsOffTheEnd);
    }
    let loc = |slot: u16| -> usize {
        pins.iter().position(|&p| p == slot).map(|i| i + 1).unwrap_or(0)
    };
    // The XMM-pin location of a slot: 0 = frame, 1..4 = f0..f3.
    let floc = |slot: u16| -> usize {
        fpins.iter().position(|&p| p == slot).map(|i| i + 1).unwrap_or(0)
    };
    // Which pinned registers an op forces through MEMORY (spill before /
    // reload after) because it has no variant family.
    let mem_form_touch = |op: &MicroOp| -> (Vec<u16>, Vec<u16>) {
        let mut reads: Vec<u16> = Vec::new();
        let mut writes: Vec<u16> = Vec::new();
        match *op {
            MicroOp::Div { dst, lhs, rhs } | MicroOp::Mod { dst, lhs, rhs } => {
                reads.extend([lhs, rhs]);
                writes.push(dst);
            }
            MicroOp::DivPow2 { dst, lhs, .. } | MicroOp::MagicDivU { dst, lhs, .. } => {
                reads.push(lhs);
                writes.push(dst);
            }
            // Float add/sub/mul are register-threaded (V_FBINOP) — NOT
            // mem-form. DivF (div-by-zero side-exit) and the float comparisons
            // (no variant yet) stay mem-form.
            MicroOp::DivF { dst, lhs, rhs }
            | MicroOp::LtF { dst, lhs, rhs }
            | MicroOp::LtEqF { dst, lhs, rhs }
            | MicroOp::GtF { dst, lhs, rhs }
            | MicroOp::GtEqF { dst, lhs, rhs }
            | MicroOp::EqF { dst, lhs, rhs }
            | MicroOp::NeqF { dst, lhs, rhs } => {
                reads.extend([lhs, rhs]);
                writes.push(dst);
            }
            MicroOp::NotInt { dst, src }
            | MicroOp::NotBool { dst, src }
            | MicroOp::IntToFloat { dst, src }
            | MicroOp::SqrtF { dst, src } => {
                reads.push(src);
                writes.push(dst);
            }
            MicroOp::BranchF { lhs, rhs, .. } => reads.extend([lhs, rhs]),
            // ptr_slot/len_slot are read DIRECTLY from the frame by the array
            // stencil (or, when the array's base pointer is GP-pinned, from its
            // register via the RPTR variant) — never through the threaded-pin
            // spill path, so they are NOT listed as spill-reads here. (They are
            // loop-invariant; the array never moves unless pushed-to, and a
            // pushed array is not ptr-pinned.)
            MicroOp::ArrLoad { dst, idx, .. } => {
                reads.push(idx);
                writes.push(dst);
            }
            MicroOp::ArrLoadAffine { dst, a, op, b, .. } => {
                reads.push(a);
                if op != AffOp::None {
                    reads.push(b);
                }
                writes.push(dst);
            }
            MicroOp::ArrLoad2F { dst, i: ix, j: jx, .. } => {
                reads.extend([ix, jx]);
                writes.push(dst);
            }
            MicroOp::ArrLoad2 { dst, i: ix, j: jx, .. } => {
                reads.extend([ix, jx]);
                writes.push(dst);
            }
            MicroOp::ArrStore { src, idx, .. } => {
                reads.extend([src, idx]);
            }
            MicroOp::ArrRMW { idx, operand, .. } => {
                reads.extend([idx, operand]);
            }
            MicroOp::ArrCondSwap { idx1, idx2, .. } => {
                reads.extend([idx1, idx2]);
            }
            MicroOp::ArrSwap { idx1, idx2, .. } => {
                reads.extend([idx1, idx2]);
            }
            MicroOp::FmaF { dst, a, b, c } => {
                reads.extend([a, b, c]);
                writes.push(dst);
            }
            MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => {
                reads.extend([src, vec_slot, ptr_slot, len_slot]);
                writes.extend([ptr_slot, len_slot]);
            }
            MicroOp::MapGet { dst, key, map_slot, .. } => {
                reads.extend([key, map_slot]);
                writes.push(dst);
            }
            MicroOp::MapSet { src, key, map_slot, .. } => {
                reads.extend([src, key, map_slot]);
            }
            MicroOp::MapHas { dst, key, map_slot, .. } => {
                reads.extend([key, map_slot]);
                writes.push(dst);
            }
            MicroOp::MemMem {
                h_ptr_slot,
                h_len_slot,
                n_ptr_slot,
                n_len_slot,
                needle_len_slot,
                i_slot,
                count_slot,
                ..
            } => {
                // The helper reads every input slot from the frame and writes
                // `i_slot`/`count_slot` back — so a pinned operand must spill
                // first and a pinned destination reload after. (The recognizer
                // runs MemMem regions UNPINNED, so in practice these retain to
                // empty; kept for correctness if a pin ever survives.)
                reads.extend([
                    h_ptr_slot,
                    h_len_slot,
                    n_ptr_slot,
                    n_len_slot,
                    needle_len_slot,
                    i_slot,
                    count_slot,
                ]);
                writes.extend([i_slot, count_slot]);
            }
            MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. } => {
                writes.extend([vec_slot, ptr_slot, len_slot]);
            }
            MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
                reads.push(handle_slot);
                writes.extend([vec_slot, ptr_slot, len_slot]);
            }
            MicroOp::Call { dst, .. } | MicroOp::CallSelf { dst, .. } => {
                // The callee window was staged through ordinary Moves; the
                // result lands in the frame.
                writes.push(dst);
            }
            MicroOp::CallSelfCopy { dst, src_start, arg_count, .. } => {
                // The fused stencil reads the SOURCE block from the FRAME, so
                // any source slot held in a pinned register must spill to its
                // frame cell before the copy; the result lands in the frame.
                reads.extend((src_start..src_start + arg_count).collect::<Vec<_>>());
                writes.push(dst);
            }
            _ => {}
        }
        // A touched slot needs spill/reload when it is pinned in EITHER
        // budget (GP r0..r3 or XMM f0..f3) — the spill/reload primitives
        // dispatch on which.
        let pinned = |s: u16| loc(s) != 0 || floc(s) != 0;
        reads.retain(|&r| pinned(r));
        reads.dedup();
        writes.retain(|&w| pinned(w));
        writes.dedup();
        (reads, writes)
    };
    let has_variant = |op: &MicroOp| -> bool {
        matches!(
            op,
            MicroOp::Add { .. }
                | MicroOp::Sub { .. }
                | MicroOp::Mul { .. }
                | MicroOp::BitAnd { .. }
                | MicroOp::BitOr { .. }
                | MicroOp::BitXor { .. }
                | MicroOp::Shl { .. }
                | MicroOp::Shr { .. }
                | MicroOp::Lt { .. }
                | MicroOp::LtEq { .. }
                | MicroOp::Eq { .. }
                | MicroOp::Neq { .. }
                | MicroOp::Gt { .. }
                | MicroOp::GtEq { .. }
                | MicroOp::Move { .. }
                | MicroOp::LoadConst { .. }
                | MicroOp::Return { .. }
                | MicroOp::Branch { .. }
                // Float add/sub/mul/div + sqrt — register-threaded through the
                // XMM-pin variant tables (V_FBINOP / V_DIVF / V_SQRTF), one
                // piece each. DivF still side-exits on a zero divisor; its
                // variant carries the deopt continuation.
                | MicroOp::AddF { .. }
                | MicroOp::SubF { .. }
                | MicroOp::MulF { .. }
                | MicroOp::DivF { .. }
                | MicroOp::SqrtF { .. }
                | MicroOp::IntToFloat { .. }
                | MicroOp::BranchF { .. }
        )
    };

    let n_pin = pins.len() + fpins.len();
    // PASS 1: piece index per op (prologue reloads first).
    let mut op_piece: Vec<usize> = Vec::with_capacity(ops.len());
    let mut next_piece = n_pin;
    for op in ops {
        op_piece.push(next_piece);
        next_piece += match *op {
            // EPILOGUE (Phase 2 write-set pinning): a success exit spills every
            // pinned slot (GP and XMM) back to its frame cell so the VM's
            // write-back sees the loop-carried values — `n_pin` spill pieces +
            // the return.
            MicroOp::Return { .. } => n_pin + 1,
            _ if has_variant(op) => 1,
            MicroOp::Jump { .. } => 1,
            MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => {
                if loc(cond) != 0 { 2 } else { 1 }
            }
            _ => {
                let (r, w) = mem_form_touch(op);
                r.len() + 1 + w.len()
            }
        };
    }
    let total_op_pieces = next_piece;
    let has_checked = ops.iter().any(|op| {
        matches!(
            op,
            // Integer add/sub/mul side-exit (`cont_1`) on signed overflow so the exact
            // tier promotes to BigInt — they need the status cell + deopt terminal.
            MicroOp::Add { .. }
                | MicroOp::Sub { .. }
                | MicroOp::Mul { .. }
                | MicroOp::Div { .. }
                | MicroOp::Mod { .. }
                | MicroOp::DivF { .. }
                | MicroOp::ArrLoad { .. }
                | MicroOp::ArrLoadAffine { checked: true, .. }
                | MicroOp::ArrLoad2F { .. }
                | MicroOp::ArrLoad2 { checked: true, .. }
                | MicroOp::ArrStore { .. }
                | MicroOp::ArrRMW { checked: true, .. }
                | MicroOp::ArrCondSwap { checked: true, .. }
                | MicroOp::ArrSwap { checked: true, .. }
                | MicroOp::Call { .. }
                | MicroOp::CallSelf { .. }
                | MicroOp::CallSelfCopy { .. }
                | MicroOp::MapGet { .. }
                | MicroOp::MemMem { .. }
        )
    });
    let status: Option<std::sync::Arc<AtomicI64>> = if has_checked {
        Some(shared_status.unwrap_or_else(|| std::sync::Arc::new(AtomicI64::new(0))))
    } else {
        None
    };
    let deopt_piece = total_op_pieces;

    // PASS 2: emit.
    let mut buf = JitBuffer::new();
    // Prologue: reload each pinned slot from the frame — the GP pins into
    // r0..r3 (V_MOV), then the float pins into f0..f3 (V_FMOV). The pieces
    // are laid out GP-first, so piece index `i` for `i < pins.len()` reloads
    // GP pin `i`, and `pins.len() + j` reloads float pin `j`.
    for (i, &slot) in pins.iter().enumerate() {
        buf.push_stencil(
            V_MOV[i + 1][0],
            &[HoleValue::Const(0, slot as i64), HoleValue::Cont(0, buf.label(i + 1))],
        );
    }
    for (j, &slot) in fpins.iter().enumerate() {
        let here = pins.len() + j;
        buf.push_stencil(
            V_FMOV[j + 1][0],
            &[HoleValue::Const(0, slot as i64), HoleValue::Cont(0, buf.label(here + 1))],
        );
    }
    // Spill/reload a pinned slot — dispatching on its budget. A GP pin moves
    // through V_MOV (r0..r3 ↔ frame i64); a float pin through V_FMOV (f0..f3
    // ↔ frame f64 bits). A slot is in exactly one budget.
    let spill = |buf: &mut JitBuffer, slot: u16, after: usize| {
        if floc(slot) != 0 {
            buf.push_stencil(
                V_FMOV[0][floc(slot)],
                &[HoleValue::Const(1, slot as i64), HoleValue::Cont(0, buf.label(after))],
            );
        } else {
            buf.push_stencil(
                V_MOV[0][loc(slot)],
                &[HoleValue::Const(1, slot as i64), HoleValue::Cont(0, buf.label(after))],
            );
        }
    };
    let reload = |buf: &mut JitBuffer, slot: u16, after: usize| {
        if floc(slot) != 0 {
            buf.push_stencil(
                V_FMOV[floc(slot)][0],
                &[HoleValue::Const(0, slot as i64), HoleValue::Cont(0, buf.label(after))],
            );
        } else {
            buf.push_stencil(
                V_MOV[loc(slot)][0],
                &[HoleValue::Const(0, slot as i64), HoleValue::Cont(0, buf.label(after))],
            );
        }
    };
    // Jump threading: a continuation that lands on an unconditional `Jump`
    // follows it straight to the target, so the predecessor branches directly
    // and the per-iteration back-edge jmp every loop pays (`…; Jump(top)`) is
    // skipped. The Jump piece is still EMITTED (left as unreferenced dead code)
    // so piece indices — and every `op_piece[..]` label — stay valid.
    let no_thread = std::env::var_os("LOGOS_NOTHREAD").is_some();
    let thread = |start: usize| -> usize {
        if no_thread {
            return start;
        }
        let mut idx = start;
        for _ in 0..=ops.len() {
            match ops.get(idx) {
                Some(MicroOp::Jump { target }) => idx = *target,
                _ => break,
            }
        }
        idx
    };
    for (i, op) in ops.iter().enumerate() {
        let here = op_piece[i];
        debug_assert_eq!(
            buf.pieces_pushed(),
            here,
            "PASS1/PASS2 piece divergence before op {i} ({op:?}); pins={pins:?} fpins={fpins:?}"
        );
        let next_op = op_piece.get(thread(i + 1)).copied().unwrap_or(total_op_pieces);
        let lbl = |t: usize| -> usize { op_piece[thread(t)] };
        match *op {
            MicroOp::LoadConst { dst, value } => {
                let d = loc(dst);
                if d == 0 {
                    buf.push_stencil(
                        &ST_CONSTST,
                        &[
                            HoleValue::Const(0, value),
                            HoleValue::Const(1, dst as i64),
                            HoleValue::Cont(0, buf.label(next_op)),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        V_CONST[d - 1],
                        &[HoleValue::Const(0, value), HoleValue::Cont(0, buf.label(next_op))],
                    );
                }
            }
            MicroOp::Move { dst, src } => {
                let (fd, fs) = (floc(dst), floc(src));
                if fd != 0 || fs != 0 {
                    // FLOAT move: at least one operand rides an XMM pin, so this
                    // is an f64 copy — thread it through V_FMOV (register f→f, or
                    // a frame↔register half when the other end is frame-resident).
                    // A float-pinned slot is never GP-pinned, so `loc` plays no
                    // part. Register ends read/write the XMM directly (no hole);
                    // a frame end uses hole 0 (src) or hole 1 (dst).
                    let mut holes: Vec<HoleValue> = Vec::new();
                    if fs == 0 {
                        holes.push(HoleValue::Const(0, src as i64));
                    }
                    if fd == 0 {
                        holes.push(HoleValue::Const(1, dst as i64));
                    }
                    holes.push(HoleValue::Cont(0, buf.label(next_op)));
                    buf.push_stencil(V_FMOV[fd][fs], &holes);
                } else {
                    let (d, sl) = (loc(dst), loc(src));
                    if d == 0 && sl == 0 {
                        buf.push_stencil(
                            &ST_MOVSS,
                            &[
                                HoleValue::Const(0, src as i64),
                                HoleValue::Const(1, dst as i64),
                                HoleValue::Cont(0, buf.label(next_op)),
                            ],
                        );
                    } else {
                        let mut holes: Vec<HoleValue> = Vec::new();
                        if sl == 0 {
                            holes.push(HoleValue::Const(0, src as i64));
                        }
                        if d == 0 {
                            holes.push(HoleValue::Const(1, dst as i64));
                        }
                        holes.push(HoleValue::Cont(0, buf.label(next_op)));
                        buf.push_stencil(V_MOV[d][sl], &holes);
                    }
                }
            }
            MicroOp::Add { dst, lhs, rhs }
            | MicroOp::Sub { dst, lhs, rhs }
            | MicroOp::Mul { dst, lhs, rhs }
            | MicroOp::BitAnd { dst, lhs, rhs }
            | MicroOp::BitOr { dst, lhs, rhs }
            | MicroOp::BitXor { dst, lhs, rhs }
            | MicroOp::Shl { dst, lhs, rhs }
            | MicroOp::Shr { dst, lhs, rhs }
            | MicroOp::Lt { dst, lhs, rhs }
            | MicroOp::LtEq { dst, lhs, rhs }
            | MicroOp::Eq { dst, lhs, rhs }
            | MicroOp::Neq { dst, lhs, rhs }
            | MicroOp::Gt { dst, lhs, rhs }
            | MicroOp::GtEq { dst, lhs, rhs } => {
                // Gt/GtEq map onto lt/le with swapped operands; Neq is ne.
                let (fam, a, b) = match *op {
                    MicroOp::Add { .. } => (0usize, lhs, rhs),
                    MicroOp::Sub { .. } => (1, lhs, rhs),
                    MicroOp::Mul { .. } => (2, lhs, rhs),
                    MicroOp::BitAnd { .. } => (3, lhs, rhs),
                    MicroOp::BitOr { .. } => (4, lhs, rhs),
                    MicroOp::BitXor { .. } => (5, lhs, rhs),
                    MicroOp::Shl { .. } => (6, lhs, rhs),
                    MicroOp::Shr { .. } => (7, lhs, rhs),
                    MicroOp::Lt { .. } => (8, lhs, rhs),
                    MicroOp::Gt { .. } => (8, rhs, lhs),
                    MicroOp::LtEq { .. } => (9, lhs, rhs),
                    MicroOp::GtEq { .. } => (9, rhs, lhs),
                    MicroOp::Eq { .. } => (10, lhs, rhs),
                    MicroOp::Neq { .. } => (11, lhs, rhs),
                    _ => unreachable!(),
                };
                let (d, l, r) = (loc(dst), loc(a), loc(b));
                let mut holes: Vec<HoleValue> = Vec::new();
                if l == 0 {
                    holes.push(HoleValue::Const(0, a as i64));
                }
                if r == 0 {
                    holes.push(HoleValue::Const(1, b as i64));
                }
                if d == 0 {
                    holes.push(HoleValue::Const(2, dst as i64));
                }
                holes.push(HoleValue::Cont(0, buf.label(next_op)));
                // add/sub/mul (fam 0/1/2) are the CHECKED families: their generated
                // stencil takes cont_1 to the shared deopt terminal on signed overflow,
                // so the exact tier recomputes and promotes to BigInt.
                if fam <= 2 {
                    holes.push(HoleValue::Cont(1, buf.label(deopt_piece)));
                }
                buf.push_stencil(V_BINOP[fam][d][l][r], &holes);
            }
            // FLOAT 3-address (add/sub/mul): same location encoding, float
            // variant table.
            MicroOp::AddF { dst, lhs, rhs }
            | MicroOp::SubF { dst, lhs, rhs }
            | MicroOp::MulF { dst, lhs, rhs } => {
                let fam = match *op {
                    MicroOp::AddF { .. } => 0usize,
                    MicroOp::SubF { .. } => 1,
                    MicroOp::MulF { .. } => 2,
                    _ => unreachable!(),
                };
                // Float operands are XMM-pinned → index by the FLOAT pin
                // location (floc), NOT the GP location (loc). Using loc would
                // resolve an XMM-pinned float to frame (location 0) and read a
                // STALE frame cell instead of the live f-pin.
                let (d, l, r) = (floc(dst), floc(lhs), floc(rhs));
                let mut holes: Vec<HoleValue> = Vec::new();
                if l == 0 {
                    holes.push(HoleValue::Const(0, lhs as i64));
                }
                if r == 0 {
                    holes.push(HoleValue::Const(1, rhs as i64));
                }
                if d == 0 {
                    holes.push(HoleValue::Const(2, dst as i64));
                }
                holes.push(HoleValue::Cont(0, buf.label(next_op)));
                buf.push_stencil(V_FBINOP[fam][d][l][r], &holes);
            }
            // FLOAT sqrt threaded through XMM pins (no side-exit).
            MicroOp::SqrtF { dst, src } => {
                let (d, s) = (floc(dst), floc(src));
                let mut holes: Vec<HoleValue> = Vec::new();
                if s == 0 {
                    holes.push(HoleValue::Const(0, src as i64));
                }
                if d == 0 {
                    holes.push(HoleValue::Const(1, dst as i64));
                }
                holes.push(HoleValue::Cont(0, buf.label(next_op)));
                buf.push_stencil(V_SQRTF[d][s], &holes);
            }
            // FLOAT divide threaded through XMM pins; a zero divisor side-exits
            // to the deopt terminal (cont 1), exactly like the mem-form form.
            MicroOp::DivF { dst, lhs, rhs } => {
                let (d, l, r) = (floc(dst), floc(lhs), floc(rhs));
                let mut holes: Vec<HoleValue> = Vec::new();
                if l == 0 {
                    holes.push(HoleValue::Const(0, lhs as i64));
                }
                if r == 0 {
                    holes.push(HoleValue::Const(1, rhs as i64));
                }
                if d == 0 {
                    holes.push(HoleValue::Const(2, dst as i64));
                }
                holes.push(HoleValue::Cont(0, buf.label(next_op)));
                holes.push(HoleValue::Cont(1, buf.label(deopt_piece)));
                buf.push_stencil(V_DIVF[d][l][r], &holes);
            }
            // Int→Float threaded through XMM pins: int src at GP loc, f64 dst
            // at XMM pin.
            MicroOp::IntToFloat { dst, src } => {
                let (d, s) = (floc(dst), loc(src));
                let mut holes: Vec<HoleValue> = Vec::new();
                if s == 0 {
                    holes.push(HoleValue::Const(0, src as i64));
                }
                if d == 0 {
                    holes.push(HoleValue::Const(1, dst as i64));
                }
                holes.push(HoleValue::Cont(0, buf.label(next_op)));
                buf.push_stencil(V_I2F[d][s], &holes);
            }
            // FUSED float compare-and-branch threaded through XMM pins. Same
            // NaN-correct cmp→family mapping as the unpinned form (brlef for
            // <=/>=, never brltf-swapped — NaN makes `a<=b` ≠ `!(b<a)`).
            MicroOp::BranchF { cmp, lhs, rhs, target } => {
                let t = buf.label(lbl(target));
                let n = buf.label(next_op);
                let (fam, a, b) = match cmp {
                    Cmp::Lt => (0usize, lhs, rhs),
                    Cmp::Gt => (0, rhs, lhs),
                    Cmp::LtEq => (1, lhs, rhs),
                    Cmp::GtEq => (1, rhs, lhs),
                    Cmp::Eq => (2, lhs, rhs),
                    Cmp::NotEq => (2, lhs, rhs),
                };
                let (c0, c1) = if cmp == Cmp::NotEq { (t, n) } else { (n, t) };
                let (l, r) = (floc(a), floc(b));
                let mut holes: Vec<HoleValue> = Vec::new();
                if l == 0 {
                    holes.push(HoleValue::Const(0, a as i64));
                }
                if r == 0 {
                    holes.push(HoleValue::Const(1, b as i64));
                }
                holes.push(HoleValue::Cont(0, c0));
                holes.push(HoleValue::Cont(1, c1));
                buf.push_stencil(V_BRANCHF[fam][l][r], &holes);
            }
            MicroOp::Branch { cmp, lhs, rhs, target } => {
                // TRUE -> fall through (cont 0 of the variant is TAKEN when
                // the condition holds; mirror the unpinned mapping: jump
                // when FALSE).
                let t = buf.label(lbl(target));
                let n = buf.label(next_op);
                let (fam, a, b, c_true, c_false) = match cmp {
                    Cmp::Lt => (0usize, lhs, rhs, n, t),
                    Cmp::GtEq => (0, lhs, rhs, t, n),
                    Cmp::Gt => (0, rhs, lhs, n, t),
                    Cmp::LtEq => (0, rhs, lhs, t, n),
                    Cmp::Eq => (2, lhs, rhs, n, t),
                    Cmp::NotEq => (2, lhs, rhs, t, n),
                };
                let (l, r) = (loc(a), loc(b));
                let mut holes: Vec<HoleValue> = Vec::new();
                if l == 0 {
                    holes.push(HoleValue::Const(0, a as i64));
                }
                if r == 0 {
                    holes.push(HoleValue::Const(1, b as i64));
                }
                holes.push(HoleValue::Cont(0, c_true));
                holes.push(HoleValue::Cont(1, c_false));
                buf.push_stencil(V_BRANCH[fam][l][r], &holes);
            }
            MicroOp::Jump { target } => {
                buf.push_stencil(&ST_JUMP, &[HoleValue::Cont(0, buf.label(lbl(target)))]);
            }
            MicroOp::JumpIfFalse { cond, target } | MicroOp::JumpIfTrue { cond, target } => {
                let c = loc(cond);
                let mut brz_piece = here;
                if c != 0 {
                    spill(&mut buf, cond, here + 1);
                    brz_piece = here + 1;
                }
                let _ = brz_piece;
                let (c0, c1) = if matches!(op, MicroOp::JumpIfFalse { .. }) {
                    (buf.label(next_op), buf.label(lbl(target)))
                } else {
                    (buf.label(lbl(target)), buf.label(next_op))
                };
                buf.push_stencil(
                    &ST_BRZ,
                    &[HoleValue::Const(0, cond as i64), HoleValue::Cont(0, c0), HoleValue::Cont(1, c1)],
                );
            }
            MicroOp::Return { src } => {
                // EPILOGUE: spill every pinned slot to its frame cell before
                // returning, so the VM's success write-back (which reads the
                // frame) observes loop-carried values held in registers. Both
                // budgets spill — the GP pins through V_MOV, the float pins
                // through V_FMOV (the `spill` closure dispatches on which) —
                // matching the `n_pin + 1` pieces reserved for this return.
                // The deopt terminal is separate and discards the frame, so it
                // needs no spill — only this success path does.
                let mut cursor = here;
                for &slot in pins.iter().chain(fpins.iter()) {
                    spill(&mut buf, slot, cursor + 1);
                    cursor += 1;
                }
                let sl = loc(src);
                if sl == 0 {
                    buf.push_stencil(&ST_RET2, &[HoleValue::Const(0, src as i64)]);
                } else {
                    buf.push_stencil(V_RET[sl - 1], &[]);
                }
            }
            ref other => {
                // Memory-form op: spill pinned reads, run, reload pinned
                // writes — emitted as its own piece run.
                let (reads, writes) = mem_form_touch(other);
                let mut cursor = here;
                for &slot in &reads {
                    spill(&mut buf, slot, cursor + 1);
                    cursor += 1;
                }
                let after_op = cursor + 1;
                let next_for_op = if writes.is_empty() { next_op } else { after_op };
                let next_label = buf.label(next_for_op);
                // REGISTER-FORM array access: a word array whose base pointer is
                // GP-pinned reads the pointer from its register (no per-access
                // frame load). The index/length/value stay in the frame exactly
                // as the mem-form path, so the spill/reload bookkeeping above is
                // unchanged (mem_form_touch excludes ptr/len). Falls back to the
                // frame-pointer mem-form for byte arrays / unpinned pointers.
                let rptr = |buf: &mut JitBuffer, table: &[&'static crate::Stencil; 5],
                            n: usize, idx: u16, vslot: u16, len_slot: u16, checked: bool| {
                    let mut holes = vec![
                        HoleValue::Const(0, idx as i64),
                        HoleValue::Const(3, vslot as i64),
                        HoleValue::Cont(0, next_label),
                    ];
                    if checked {
                        holes.push(HoleValue::Const(2, len_slot as i64));
                        holes.push(HoleValue::Cont(1, buf.label(deopt_piece)));
                    }
                    buf.push_stencil(table[n], &holes);
                };
                let used_rptr = match *other {
                    MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte: false, narrow32: false, checked }
                        if loc(ptr_slot) != 0 =>
                    {
                        let n = loc(ptr_slot);
                        let t = if checked { &V_ARRLD_RPTR_C } else { &V_ARRLD_RPTR };
                        rptr(&mut buf, t, n, idx, dst, len_slot, checked);
                        true
                    }
                    MicroOp::ArrStore { src, idx, ptr_slot, len_slot, byte: false, narrow32: false, checked }
                        if loc(ptr_slot) != 0 =>
                    {
                        let n = loc(ptr_slot);
                        let t = if checked { &V_ARRST_RPTR_C } else { &V_ARRST_RPTR };
                        rptr(&mut buf, t, n, idx, src, len_slot, checked);
                        true
                    }
                    _ => false,
                };
                if !used_rptr {
                    emit_mem_form(&mut buf, other, next_label, deopt_piece, &status);
                }
                cursor += 1;
                for (k, &slot) in writes.iter().enumerate() {
                    let after = if k + 1 == writes.len() { next_op } else { cursor + 1 };
                    reload(&mut buf, slot, after);
                    cursor += 1;
                }
            }
        }
    }
    if let Some(cell) = &status {
        let addr = cell.as_ref() as *const AtomicI64 as i64;
        buf.push_stencil(&ST_DEOPT, &[HoleValue::Const(0, addr)]);
    }
    let chain = buf.finish().map_err(|e| JitCompileError::Assembly(e.to_string()))?;
    Ok(CompiledChain { chain, status, keepalive: Vec::new() })
}

/// The returned chain is run with [`CompiledChain::run_with_frame`]; the
/// frame's slots are the program's registers (inputs pre-loaded by the
/// caller, outputs visible after the run). Programs containing checked ops
/// (Div/Mod) get a status cell and a shared side-exit terminal; their runs
/// can report [`ChainOutcome::Deopt`].
pub fn compile_straightline(ops: &[MicroOp]) -> Result<CompiledChain, JitCompileError> {
    compile_straightline_with(ops, None)
}

/// [`compile_straightline`] with a caller-supplied (per-program) status
/// cell, so every chain of one program — and the call stencils between them
/// — share a single side-exit channel. The cell's address is what
/// `MicroOp::Call::status_addr` must carry.
pub fn compile_straightline_with(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
) -> Result<CompiledChain, JitCompileError> {
    compile_straightline_coded(ops, shared_status, None, 0)
}

/// Like [`compile_straightline_with`], with an optional per-op DEOPT CODE
/// table (parallel to `ops`). An op whose code is not the plain marker `1`
/// side-exits through its own terminal piece storing that encoded value —
/// the function tier encodes `(bytecode_pc << 2) | 3` so the VM can resume
/// precisely. `None` (and code `1`) keep the shared replay terminal.
pub fn compile_straightline_coded(
    ops: &[MicroOp],
    shared_status: Option<std::sync::Arc<AtomicI64>>,
    deopt_codes: Option<&[i64]>,
    depth_addr: i64,
) -> Result<CompiledChain, JitCompileError> {
    if ops.is_empty() {
        return Err(JitCompileError::Empty);
    }
    if !matches!(ops.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return Err(JitCompileError::FallsOffTheEnd);
    }
    for (i, op) in ops.iter().enumerate() {
        match *op {
            MicroOp::Jump { target }
            | MicroOp::JumpIfFalse { target, .. }
            | MicroOp::JumpIfTrue { target, .. }
            | MicroOp::Branch { target, .. }
            | MicroOp::BranchF { target, .. } => {
                if target >= ops.len() {
                    return Err(JitCompileError::BadJumpTarget { op_index: i, target });
                }
            }
            _ => {}
        }
    }

    // 3-address lowering: exactly ONE piece per micro-op (piece index ==
    // op index), so jump targets need no remapping. Programs with checked
    // ops share ONE side-exit terminal placed after every op piece; its
    // patched constant is the status cell's address.
    let has_checked = ops.iter().any(|op| {
        matches!(
            op,
            // Integer add/sub/mul side-exit (`cont_1`) on signed overflow so the exact
            // tier promotes to BigInt — they need the status cell + deopt terminal.
            MicroOp::Add { .. }
                | MicroOp::Sub { .. }
                | MicroOp::Mul { .. }
                | MicroOp::Div { .. }
                | MicroOp::Mod { .. }
                | MicroOp::DivF { .. }
                | MicroOp::ArrLoad { .. }
                | MicroOp::ArrLoadAffine { checked: true, .. }
                | MicroOp::ArrLoad2F { .. }
                | MicroOp::ArrLoad2 { checked: true, .. }
                | MicroOp::ArrStore { .. }
                | MicroOp::ArrRMW { checked: true, .. }
                | MicroOp::ArrCondSwap { checked: true, .. }
                | MicroOp::ArrSwap { checked: true, .. }
                | MicroOp::Call { .. }
                | MicroOp::CallSelf { .. }
                | MicroOp::CallSelfCopy { .. }
                | MicroOp::MapGet { .. }
                | MicroOp::MemMem { .. }
        )
    });
    let status: Option<std::sync::Arc<AtomicI64>> = if has_checked {
        Some(shared_status.unwrap_or_else(|| std::sync::Arc::new(AtomicI64::new(0))))
    } else {
        None
    };
    let deopt_piece = ops.len();
    // Distinct non-plain codes get their own terminal pieces after the
    // shared one, in first-use order (deterministic).
    let mut code_pieces: Vec<i64> = Vec::new();
    if let Some(codes) = deopt_codes {
        debug_assert_eq!(codes.len(), ops.len());
        for (i, op) in ops.iter().enumerate() {
            let coded = matches!(
                op,
                // add/sub/mul are CHECKED now (overflow → exact promotion): inside a
                // region their side-exit must carry the op's PRECISE deopt code so the
                // VM resumes with the right rollback, exactly like the other checked
                // ops — so their codes need terminal pieces too.
                MicroOp::Add { .. }
                    | MicroOp::Sub { .. }
                    | MicroOp::Mul { .. }
                    | MicroOp::Div { .. }
                    | MicroOp::Mod { .. }
                    | MicroOp::DivF { .. }
                    | MicroOp::ArrLoad { .. }
                    | MicroOp::ArrLoadAffine { checked: true, .. }
                    | MicroOp::ArrLoad2F { .. }
                    | MicroOp::ArrLoad2 { checked: true, .. }
                    | MicroOp::ArrStore { .. }
                    | MicroOp::MapGet { .. }
            );
            if coded && codes[i] != 1 && !code_pieces.contains(&codes[i]) {
                code_pieces.push(codes[i]);
            }
        }
    }
    let piece_for = |code: i64| -> usize {
        if code == 1 {
            deopt_piece
        } else {
            deopt_piece + 1 + code_pieces.iter().position(|&c| c == code).unwrap()
        }
    };
    let code_at = |i: usize| -> i64 { deopt_codes.map(|c| c[i]).unwrap_or(1) };

    let mut buf = JitBuffer::new();
    for (i, op) in ops.iter().enumerate() {
        let next = buf.label(i + 1);
        match *op {
            MicroOp::LoadConst { dst, value } => {
                buf.push_stencil(
                    &ST_CONSTST,
                    &[
                        HoleValue::Const(0, value),
                        HoleValue::Const(1, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::Move { dst, src } => {
                buf.push_stencil(
                    &ST_MOVSS,
                    &[
                        HoleValue::Const(0, src as i64),
                        HoleValue::Const(1, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::Add { dst, lhs, rhs }
            | MicroOp::Sub { dst, lhs, rhs }
            | MicroOp::Mul { dst, lhs, rhs }
            | MicroOp::Lt { dst, lhs, rhs }
            | MicroOp::LtEq { dst, lhs, rhs }
            | MicroOp::Eq { dst, lhs, rhs }
            | MicroOp::Neq { dst, lhs, rhs } => {
                let stencil = match op {
                    MicroOp::Add { .. } => &ST_ADD3,
                    MicroOp::Sub { .. } => &ST_SUB3,
                    MicroOp::Mul { .. } => &ST_MUL3,
                    MicroOp::Lt { .. } => &ST_LT3,
                    MicroOp::LtEq { .. } => &ST_LE3,
                    MicroOp::Eq { .. } => &ST_EQ3,
                    MicroOp::Neq { .. } => &ST_NE3,
                    _ => unreachable!(),
                };
                let mut holes = vec![
                    HoleValue::Const(0, lhs as i64),
                    HoleValue::Const(1, rhs as i64),
                    HoleValue::Const(2, dst as i64),
                    HoleValue::Cont(0, next),
                ];
                // add/sub/mul are CHECKED: their stencil takes cont_1 to this op's
                // deopt terminal on signed overflow so the exact tier promotes. The
                // comparisons (lt/le/eq/ne) never overflow → cont_0 only.
                if matches!(op, MicroOp::Add { .. } | MicroOp::Sub { .. } | MicroOp::Mul { .. }) {
                    holes.push(HoleValue::Cont(1, buf.label(piece_for(code_at(i)))));
                }
                buf.push_stencil(stencil, &holes);
            }
            // a > b ⇔ b < a; a >= b ⇔ b <= a: swap operands, reuse lt3/le3
            // (IEEE-exact for the float twins too — NaN is false on both
            // sides of the swap).
            MicroOp::Gt { dst, lhs, rhs }
            | MicroOp::GtEq { dst, lhs, rhs }
            | MicroOp::GtF { dst, lhs, rhs }
            | MicroOp::GtEqF { dst, lhs, rhs } => {
                let stencil = match op {
                    MicroOp::Gt { .. } => &ST_LT3,
                    MicroOp::GtEq { .. } => &ST_LE3,
                    MicroOp::GtF { .. } => &ST_LTF3,
                    _ => &ST_LEF3,
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, rhs as i64),
                        HoleValue::Const(1, lhs as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::BitAnd { dst, lhs, rhs }
            | MicroOp::BitOr { dst, lhs, rhs }
            | MicroOp::BitXor { dst, lhs, rhs }
            | MicroOp::Shl { dst, lhs, rhs }
            | MicroOp::Shr { dst, lhs, rhs }
            | MicroOp::AddF { dst, lhs, rhs }
            | MicroOp::SubF { dst, lhs, rhs }
            | MicroOp::MulF { dst, lhs, rhs }
            | MicroOp::LtF { dst, lhs, rhs }
            | MicroOp::LtEqF { dst, lhs, rhs }
            | MicroOp::EqF { dst, lhs, rhs }
            | MicroOp::NeqF { dst, lhs, rhs } => {
                let stencil = match op {
                    MicroOp::BitAnd { .. } => &ST_AND3,
                    MicroOp::BitOr { .. } => &ST_OR3,
                    MicroOp::BitXor { .. } => &ST_XOR3,
                    MicroOp::Shl { .. } => &ST_SHL3,
                    MicroOp::Shr { .. } => &ST_SHR3,
                    MicroOp::AddF { .. } => &ST_ADDF3,
                    MicroOp::SubF { .. } => &ST_SUBF3,
                    MicroOp::MulF { .. } => &ST_MULF3,
                    MicroOp::LtF { .. } => &ST_LTF3,
                    MicroOp::LtEqF { .. } => &ST_LEF3,
                    MicroOp::EqF { .. } => &ST_EQF3,
                    _ => &ST_NEF3,
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, lhs as i64),
                        HoleValue::Const(1, rhs as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::DivF { dst, lhs, rhs } => {
                buf.push_stencil(
                    &ST_DIVF3C,
                    &[
                        HoleValue::Const(0, lhs as i64),
                        HoleValue::Const(1, rhs as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                    ],
                );
            }
            MicroOp::Call {
                dst,
                args_start,
                table_addr,
                depth_addr,
                status_addr,
                limit_slot,
                depth_limit,
            } => {
                let code = code_at(i);
                if code != 1 {
                    // Precise variant: hole 6 carries the encoded deopt
                    // value; the depth limit is BAKED at the kernel's
                    // locked MAX_CALL_DEPTH (the adapter verified it).
                    debug_assert_eq!(depth_limit, BAKED_CALL_DEPTH);
                    buf.push_stencil(
                        &ST_CALL_PRECISE,
                        &[
                            HoleValue::Const(0, table_addr),
                            HoleValue::Const(1, args_start as i64),
                            HoleValue::Const(2, dst as i64),
                            HoleValue::Const(3, depth_addr),
                            HoleValue::Const(4, status_addr),
                            HoleValue::Const(5, limit_slot as i64),
                            HoleValue::Const(6, code),
                            HoleValue::Cont(0, next),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        &ST_CALL,
                        &[
                            HoleValue::Const(0, table_addr),
                            HoleValue::Const(1, args_start as i64),
                            HoleValue::Const(2, dst as i64),
                            HoleValue::Const(3, depth_addr),
                            HoleValue::Const(4, status_addr),
                            HoleValue::Const(5, limit_slot as i64),
                            HoleValue::Const(6, depth_limit),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::CallSelf { dst, args_start, depth_addr, status_addr, limit_slot, frame_size } => {
                buf.push_stencil(
                    &ST_CALL_SELF,
                    &[
                        HoleValue::Const(0, 0),
                        HoleValue::Const(1, args_start as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Const(3, depth_addr),
                        HoleValue::Const(4, status_addr),
                        HoleValue::Const(5, limit_slot as i64),
                        HoleValue::Const(6, frame_size),
                        HoleValue::Cont(0, next),
                    ],
                );
                buf.mark_patch_hole(0);
            }
            MicroOp::CallSelfCopy {
                dst,
                args_start,
                src_start,
                arg_count,
                depth_addr,
                status_addr,
                limit_slot,
                frame_size,
            } => {
                buf.push_stencil(
                    &ST_CALL_SELF_COPY,
                    &[
                        HoleValue::Const(0, 0),
                        HoleValue::Const(1, args_start as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Const(3, depth_addr),
                        HoleValue::Const(4, status_addr),
                        HoleValue::Const(5, limit_slot as i64),
                        HoleValue::Const(6, frame_size),
                        HoleValue::Const(7, src_start as i64),
                        HoleValue::Const(8, arg_count as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
                buf.mark_patch_hole(0);
            }
            MicroOp::NewList { vec_slot, ptr_slot, len_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_ALLOCLIST,
                    &[
                        HoleValue::Const(0, vec_slot as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(2, len_slot as i64),
                        HoleValue::Const(3, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_LISTTRIPLE,
                    &[
                        HoleValue::Const(0, handle_slot as i64),
                        HoleValue::Const(1, vec_slot as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Const(3, len_slot as i64),
                        HoleValue::Const(4, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::MapGet { dst, key, map_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_MAPHGET,
                    &[
                        HoleValue::Const(0, map_slot as i64),
                        HoleValue::Const(1, key as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Const(3, helper_addr),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                    ],
                );
            }
            MicroOp::MapSet { src, key, map_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_MAPHSET,
                    &[
                        HoleValue::Const(0, map_slot as i64),
                        HoleValue::Const(1, key as i64),
                        HoleValue::Const(2, src as i64),
                        HoleValue::Const(3, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::MemMem {
                h_ptr_slot,
                h_len_slot,
                n_ptr_slot,
                n_len_slot,
                needle_len_slot,
                i_slot,
                count_slot,
                helper_addr,
            } => {
                buf.push_stencil(
                    &ST_MEMMEM,
                    &[
                        HoleValue::Const(0, h_ptr_slot as i64),
                        HoleValue::Const(1, h_len_slot as i64),
                        HoleValue::Const(2, n_ptr_slot as i64),
                        HoleValue::Const(3, n_len_slot as i64),
                        HoleValue::Const(4, needle_len_slot as i64),
                        HoleValue::Const(5, i_slot as i64),
                        HoleValue::Const(6, count_slot as i64),
                        HoleValue::Const(7, helper_addr),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                    ],
                );
            }
            MicroOp::MapHas { dst, key, map_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_MAPHHAS,
                    &[
                        HoleValue::Const(0, map_slot as i64),
                        HoleValue::Const(1, key as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Const(3, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, helper_addr, byte: _, narrow32: _ } => {
                buf.push_stencil(
                    &ST_PUSH,
                    &[
                        HoleValue::Const(0, src as i64),
                        HoleValue::Const(1, vec_slot as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Const(3, len_slot as i64),
                        HoleValue::Const(4, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::ListClear { vec_slot, ptr_slot, len_slot, helper_addr } => {
                buf.push_stencil(
                    &ST_LIST_CLEAR,
                    &[
                        HoleValue::Const(0, vec_slot as i64),
                        HoleValue::Const(1, ptr_slot as i64),
                        HoleValue::Const(2, len_slot as i64),
                        HoleValue::Const(3, helper_addr),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
                if checked {
                    buf.push_stencil(
                        arrld_stencil(byte, narrow32, true),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(2, len_slot as i64),
                            HoleValue::Const(3, dst as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    // Bounds-check eliminated (Oracle-proven): no length hole,
                    // no out-of-bounds continuation.
                    buf.push_stencil(
                        arrld_stencil(byte, narrow32, false),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(3, dst as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::ArrLoadAffine { dst, a, op, b, const_offset, ptr_slot, len_slot, checked } => {
                if checked {
                    buf.push_stencil(
                        affine_stencil(op, true),
                        &[
                            HoleValue::Const(0, a as i64),
                            HoleValue::Const(1, b as i64),
                            HoleValue::Const(2, const_offset),
                            HoleValue::Const(3, ptr_slot as i64),
                            HoleValue::Const(4, len_slot as i64),
                            HoleValue::Const(5, dst as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        affine_stencil(op, false),
                        &[
                            HoleValue::Const(0, a as i64),
                            HoleValue::Const(1, b as i64),
                            HoleValue::Const(2, const_offset),
                            HoleValue::Const(3, ptr_slot as i64),
                            HoleValue::Const(5, dst as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::ArrStore { src, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
                if checked {
                    buf.push_stencil(
                        arrst_stencil(byte, narrow32, true),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(2, len_slot as i64),
                            HoleValue::Const(3, src as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        arrst_stencil(byte, narrow32, false),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(3, src as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, op, checked } => {
                if checked {
                    buf.push_stencil(
                        rmw_stencil(op, true),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(2, len_slot as i64),
                            HoleValue::Const(3, operand as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        rmw_stencil(op, false),
                        &[
                            HoleValue::Const(0, idx as i64),
                            HoleValue::Const(1, ptr_slot as i64),
                            HoleValue::Const(3, operand as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::ArrCondSwap { idx1, idx2, ptr_slot, len_slot, cmp, checked } => {
                if checked {
                    buf.push_stencil(
                        condswap_stencil(cmp, true),
                        &[
                            HoleValue::Const(0, idx1 as i64),
                            HoleValue::Const(1, idx2 as i64),
                            HoleValue::Const(2, ptr_slot as i64),
                            HoleValue::Const(3, len_slot as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        condswap_stencil(cmp, false),
                        &[
                            HoleValue::Const(0, idx1 as i64),
                            HoleValue::Const(1, idx2 as i64),
                            HoleValue::Const(2, ptr_slot as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::ArrSwap { idx1, idx2, ptr_slot, len_slot, checked } => {
                if checked {
                    buf.push_stencil(
                        &ST_ARRSWAP,
                        &[
                            HoleValue::Const(0, idx1 as i64),
                            HoleValue::Const(1, idx2 as i64),
                            HoleValue::Const(2, ptr_slot as i64),
                            HoleValue::Const(3, len_slot as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        &ST_ARRSWAP_U,
                        &[
                            HoleValue::Const(0, idx1 as i64),
                            HoleValue::Const(1, idx2 as i64),
                            HoleValue::Const(2, ptr_slot as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::FmaF { dst, a, b, c } => {
                buf.push_stencil(
                    &ST_FMAF,
                    &[
                        HoleValue::Const(0, a as i64),
                        HoleValue::Const(1, b as i64),
                        HoleValue::Const(2, c as i64),
                        HoleValue::Const(3, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::ArrLoad2F { dst, i: ix, j: jx, ptr_slot, len_slot, op } => {
                let stencil = match op {
                    FOp::Add => &ST_ARRLD2_ADDF,
                    FOp::Sub => &ST_ARRLD2_SUBF,
                    FOp::Mul => &ST_ARRLD2_MULF,
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, ix as i64),
                        HoleValue::Const(1, jx as i64),
                        HoleValue::Const(2, ptr_slot as i64),
                        HoleValue::Const(3, len_slot as i64),
                        HoleValue::Const(4, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                    ],
                );
            }
            MicroOp::ArrLoad2 { dst, i: ix, j: jx, ptr_a, len_a, ptr_b, len_b, op, checked } => {
                if checked {
                    buf.push_stencil(
                        ld2_int_stencil(op, true),
                        &[
                            HoleValue::Const(0, ix as i64),
                            HoleValue::Const(1, jx as i64),
                            HoleValue::Const(2, ptr_a as i64),
                            HoleValue::Const(3, len_a as i64),
                            HoleValue::Const(4, ptr_b as i64),
                            HoleValue::Const(5, len_b as i64),
                            HoleValue::Const(6, dst as i64),
                            HoleValue::Cont(0, next),
                            HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                        ],
                    );
                } else {
                    buf.push_stencil(
                        ld2_int_stencil(op, false),
                        &[
                            HoleValue::Const(0, ix as i64),
                            HoleValue::Const(1, jx as i64),
                            HoleValue::Const(2, ptr_a as i64),
                            HoleValue::Const(3, ptr_b as i64),
                            HoleValue::Const(4, dst as i64),
                            HoleValue::Cont(0, next),
                        ],
                    );
                }
            }
            MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src }
            | MicroOp::IntToFloat { dst, src } | MicroOp::SqrtF { dst, src } => {
                let stencil = match op {
                    MicroOp::NotInt { .. } => &ST_NOTI2,
                    MicroOp::NotBool { .. } => &ST_NOTB2,
                    MicroOp::SqrtF { .. } => &ST_SQRTF2,
                    _ => &ST_I2F2,
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, src as i64),
                        HoleValue::Const(1, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::BranchF { cmp, lhs, rhs, target } => {
                // Jump to target when cmp is FALSE — under IEEE, the false
                // path also catches every NaN-unordered pair, so the mapping
                // may ONLY swap operands (a>b ⇔ b<a; a>=b ⇔ b<=a), never
                // continuations on the truth side.
                let t = buf.label(target);
                let (stencil, a, b) = match cmp {
                    Cmp::Lt => (&ST_BRLTF, lhs, rhs),
                    Cmp::Gt => (&ST_BRLTF, rhs, lhs),
                    Cmp::LtEq => (&ST_BRLEF, lhs, rhs),
                    Cmp::GtEq => (&ST_BRLEF, rhs, lhs),
                    Cmp::Eq => (&ST_BREQF, lhs, rhs),
                    Cmp::NotEq => (&ST_BREQF, lhs, rhs),
                };
                let (c0, c1) = if cmp == Cmp::NotEq {
                    // NotEq: jump when eps-equal (the TRUE path of breqf).
                    (t, next)
                } else {
                    (next, t)
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, a as i64),
                        HoleValue::Const(1, b as i64),
                        HoleValue::Cont(0, c0),
                        HoleValue::Cont(1, c1),
                    ],
                );
            }
            MicroOp::Div { dst, lhs, rhs } | MicroOp::Mod { dst, lhs, rhs } => {
                let stencil = match op {
                    MicroOp::Div { .. } => &ST_DIV3C,
                    _ => &ST_MOD3C,
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, lhs as i64),
                        HoleValue::Const(1, rhs as i64),
                        HoleValue::Const(2, dst as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(piece_for(code_at(i)))),
                    ],
                );
            }
            MicroOp::DivPow2 { dst, lhs, k } => {
                buf.push_stencil(
                    &ST_DIVPOW2,
                    &[
                        HoleValue::Const(0, lhs as i64),
                        HoleValue::Const(1, k as i64),
                        HoleValue::Const(2, (1i64 << k) - 1),
                        HoleValue::Const(3, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
            MicroOp::Jump { target } => {
                buf.push_stencil(&ST_JUMP, &[HoleValue::Cont(0, buf.label(target))]);
            }
            MicroOp::JumpIfFalse { cond, target } => {
                // brz: nonzero → cont 0 (fall through), zero → cont 1 (target).
                buf.push_stencil(
                    &ST_BRZ,
                    &[
                        HoleValue::Const(0, cond as i64),
                        HoleValue::Cont(0, next),
                        HoleValue::Cont(1, buf.label(target)),
                    ],
                );
            }
            MicroOp::JumpIfTrue { cond, target } => {
                // Same stencil, swapped continuations: nonzero → target.
                buf.push_stencil(
                    &ST_BRZ,
                    &[
                        HoleValue::Const(0, cond as i64),
                        HoleValue::Cont(0, buf.label(target)),
                        HoleValue::Cont(1, next),
                    ],
                );
            }
            MicroOp::Branch { cmp, lhs, rhs, target } => {
                // Jump to target when cmp is FALSE; fall through when TRUE.
                // brlt/breq put the TRUE outcome on cont 0; operand and
                // continuation swaps express all six comparison kinds.
                let t = buf.label(target);
                let (stencil, a, b, c0, c1) = match cmp {
                    Cmp::Lt => (&ST_BRLT, lhs, rhs, next, t),
                    Cmp::GtEq => (&ST_BRLT, lhs, rhs, t, next),
                    Cmp::Gt => (&ST_BRLT, rhs, lhs, next, t),
                    Cmp::LtEq => (&ST_BRLT, rhs, lhs, t, next),
                    Cmp::Eq => (&ST_BREQ, lhs, rhs, next, t),
                    Cmp::NotEq => (&ST_BREQ, lhs, rhs, t, next),
                };
                buf.push_stencil(
                    stencil,
                    &[
                        HoleValue::Const(0, a as i64),
                        HoleValue::Const(1, b as i64),
                        HoleValue::Cont(0, c0),
                        HoleValue::Cont(1, c1),
                    ],
                );
            }
            MicroOp::Return { src } => {
                buf.push_stencil(&ST_RET2, &[HoleValue::Const(0, src as i64)]);
            }
            // The mutable-Text append has NO per-piece stencil — it is emitted
            // ONLY for the contiguous regalloc backend (where it lowers to a
            // SysV helper call). Decline so the caller falls back to bytecode,
            // which is always correct.
            MicroOp::StrAppend { .. } => {
                return Err(JitCompileError::Unsupported("StrAppend"));
            }
            // The magic reciprocal: a single `mul`-high + shift piece (the
            // contiguous regalloc backend also lowers it, but the stencil tier
            // owns the mixed regions regalloc declines — e.g. a `% c` next to a
            // map insert). Holes: 0 = lhs, 1 = magic, 2 = more, 3 = mul_back,
            // 4 = dst. No deopt continuation (a literal `c > 0` never faults).
            MicroOp::MagicDivU { dst, lhs, magic, more, mul_back } => {
                buf.push_stencil(
                    &ST_MAGICDIV,
                    &[
                        HoleValue::Const(0, lhs as i64),
                        HoleValue::Const(1, magic as i64),
                        HoleValue::Const(2, more as i64),
                        HoleValue::Const(3, mul_back),
                        HoleValue::Const(4, dst as i64),
                        HoleValue::Cont(0, next),
                    ],
                );
            }
        }
    }

    if let Some(cell) = &status {
        let addr = cell.as_ref() as *const AtomicI64 as i64;
        buf.push_stencil(&ST_DEOPT, &[HoleValue::Const(0, addr)]);
        for &code in &code_pieces {
            buf.push_stencil(
                &ST_DEOPT_AT,
                &[
                    HoleValue::Const(0, addr),
                    HoleValue::Const(1, code),
                    HoleValue::Const(2, depth_addr),
                ],
            );
        }
    }

    let chain = buf.finish().map_err(|e| JitCompileError::Assembly(e.to_string()))?;
    Ok(CompiledChain { chain, status, keepalive: Vec::new() })
}

/// Evaluate the unsigned magic reciprocal for [`MicroOp::MagicDivU`]:
/// `x / c` when `mul_back == 0`, else `x % c` (`mul_back == c`). The dividend is
/// reinterpreted as `u64`, sound because the op is emitted ONLY for a proven
/// non-negative `x` (the i64 bit pattern equals the value, and unsigned and
/// signed-truncating `/`/`%` agree there). The remainder is `x - q*c` in
/// wrapping i64 (its low 64 bits equal the u64 difference). The `more` encoding
/// is the [`logicaffeine_data::LogosDivU64`] one: low 6 bits = shift, `0x40`
/// = the 65-bit add-marker path, `0x80` = the pure-shift power-of-two path.
#[inline]
pub fn magic_eval(x: i64, magic: u64, more: u8, mul_back: i64) -> i64 {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;
    let n = x as u64;
    let q = if more & SHIFT_PATH != 0 {
        n >> (more & SHIFT_MASK)
    } else {
        let hi = (((magic as u128) * (n as u128)) >> 64) as u64;
        if more & ADD_MARKER != 0 {
            let t = (n.wrapping_sub(hi) >> 1).wrapping_add(hi);
            t >> (more & SHIFT_MASK)
        } else {
            hi >> (more & SHIFT_MASK)
        }
    };
    if mul_back == 0 {
        q as i64
    } else {
        x.wrapping_sub((q as i64).wrapping_mul(mul_back))
    }
}

/// The reference evaluator — the independent model every differential runs
/// against. Deliberately the dumbest possible implementation: a pc loop.
/// `fuel` bounds the step count (None when caught in a loop that long).
/// A zero divisor also yields None — the reference's model of the chain's
/// side exit (differentials feed nonzero divisors when comparing values, and
/// assert Deopt agreement when not).
pub fn reference_eval(ops: &[MicroOp], frame: &mut [i64], mut fuel: u64) -> Option<i64> {
    let mut pc = 0usize;
    while pc < ops.len() {
        if fuel == 0 {
            return None;
        }
        fuel -= 1;
        match ops[pc] {
            MicroOp::LoadConst { dst, value } => frame[dst as usize] = value,
            MicroOp::Move { dst, src } => frame[dst as usize] = frame[src as usize],
            // EXACT integer arithmetic: signed overflow returns None — the JIT's
            // side-exit (deopt) is the matching signal, so the differential stays
            // consistent (overflow ⟺ reference None ⟺ JIT deopt).
            MicroOp::Add { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].checked_add(frame[rhs as usize])?
            }
            MicroOp::Sub { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].checked_sub(frame[rhs as usize])?
            }
            MicroOp::Mul { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].checked_mul(frame[rhs as usize])?
            }
            MicroOp::Div { dst, lhs, rhs } => {
                let b = frame[rhs as usize];
                if b == 0 {
                    return None;
                }
                frame[dst as usize] = frame[lhs as usize].wrapping_div(b)
            }
            MicroOp::DivPow2 { dst, lhs, k } => {
                let x = frame[lhs as usize];
                let mask = (1i64 << k) - 1;
                frame[dst as usize] = x.wrapping_add((x >> 63) & mask) >> k;
            }
            MicroOp::MagicDivU { dst, lhs, magic, more, mul_back } => {
                frame[dst as usize] = magic_eval(frame[lhs as usize], magic, more, mul_back);
            }
            MicroOp::Mod { dst, lhs, rhs } => {
                let b = frame[rhs as usize];
                if b == 0 {
                    return None;
                }
                frame[dst as usize] = frame[lhs as usize].wrapping_rem(b)
            }
            MicroOp::Lt { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] < frame[rhs as usize]) as i64
            }
            MicroOp::Gt { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] > frame[rhs as usize]) as i64
            }
            MicroOp::LtEq { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] <= frame[rhs as usize]) as i64
            }
            MicroOp::GtEq { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] >= frame[rhs as usize]) as i64
            }
            MicroOp::Eq { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] == frame[rhs as usize]) as i64
            }
            MicroOp::Neq { dst, lhs, rhs } => {
                frame[dst as usize] = (frame[lhs as usize] != frame[rhs as usize]) as i64
            }
            MicroOp::BitAnd { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize] & frame[rhs as usize]
            }
            MicroOp::BitOr { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize] | frame[rhs as usize]
            }
            MicroOp::BitXor { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize] ^ frame[rhs as usize]
            }
            MicroOp::Shl { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].wrapping_shl(frame[rhs as usize] as u32)
            }
            MicroOp::Shr { dst, lhs, rhs } => {
                frame[dst as usize] = frame[lhs as usize].wrapping_shr(frame[rhs as usize] as u32)
            }
            MicroOp::NotInt { dst, src } => frame[dst as usize] = !frame[src as usize],
            MicroOp::NotBool { dst, src } => frame[dst as usize] = frame[src as usize] ^ 1,
            MicroOp::AddF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a + b).to_bits() as i64
            }
            MicroOp::SubF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a - b).to_bits() as i64
            }
            MicroOp::MulF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a * b).to_bits() as i64
            }
            MicroOp::DivF { dst, lhs, rhs } => {
                let b = f64::from_bits(frame[rhs as usize] as u64);
                if b == 0.0 {
                    return None;
                }
                let a = f64::from_bits(frame[lhs as usize] as u64);
                frame[dst as usize] = (a / b).to_bits() as i64
            }
            MicroOp::LtF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a < b) as i64
            }
            MicroOp::GtF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a > b) as i64
            }
            MicroOp::LtEqF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a <= b) as i64
            }
            MicroOp::GtEqF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = (a >= b) as i64
            }
            MicroOp::EqF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = ((a - b).abs() < f64::EPSILON) as i64
            }
            MicroOp::NeqF { dst, lhs, rhs } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                frame[dst as usize] = !((a - b).abs() < f64::EPSILON) as i64
            }
            MicroOp::BranchF { cmp, lhs, rhs, target } => {
                let a = f64::from_bits(frame[lhs as usize] as u64);
                let b = f64::from_bits(frame[rhs as usize] as u64);
                let truth = match cmp {
                    Cmp::Lt => a < b,
                    Cmp::Gt => a > b,
                    Cmp::LtEq => a <= b,
                    Cmp::GtEq => a >= b,
                    Cmp::Eq => (a - b).abs() < f64::EPSILON,
                    Cmp::NotEq => !((a - b).abs() < f64::EPSILON),
                };
                if !truth {
                    pc = target;
                    continue;
                }
            }
            MicroOp::IntToFloat { dst, src } => {
                frame[dst as usize] = (frame[src as usize] as f64).to_bits() as i64
            }
            MicroOp::SqrtF { dst, src } => {
                frame[dst as usize] =
                    f64::from_bits(frame[src as usize] as u64).sqrt().to_bits() as i64
            }
            // Calls, pushes and map traffic touch live runtime state the
            // reference cannot model — VM-level differentials own their
            // coverage.
            MicroOp::Call { .. }
            | MicroOp::CallSelf { .. }
            | MicroOp::CallSelfCopy { .. }
            | MicroOp::ArrPush { .. }
            | MicroOp::ListClear { .. }
            | MicroOp::StrAppend { .. }
            | MicroOp::MapGet { .. }
            | MicroOp::MapSet { .. }
            | MicroOp::MapHas { .. }
            | MicroOp::NewList { .. }
            | MicroOp::ListTriple { .. } => return None,
            MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
                let i = frame[idx as usize];
                let len = frame[len_slot as usize];
                let im1 = i.wrapping_sub(1);
                // The reference keeps the bounds guard either way (an
                // unchecked load that the Oracle proved in-bounds is in
                // range by hypothesis; refusing OOB here is just safety).
                let _ = checked;
                if (im1 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: differentials pin a live buffer, like the chain.
                frame[dst as usize] = unsafe {
                    if byte {
                        *(frame[ptr_slot as usize] as *const u8).add(im1 as usize) as i64
                    } else if narrow32 {
                        *(frame[ptr_slot as usize] as *const i32).add(im1 as usize) as i64
                    } else {
                        *(frame[ptr_slot as usize] as *const i64).add(im1 as usize)
                    }
                };
            }
            MicroOp::ArrLoadAffine { dst, a, op, b, const_offset, ptr_slot, len_slot, checked } => {
                let _ = checked;
                let idx = op.eval(frame[a as usize], frame[b as usize], const_offset);
                let len = frame[len_slot as usize];
                let im1 = idx.wrapping_sub(1);
                if (im1 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: differentials pin a live 8-byte buffer, like the chain.
                frame[dst as usize] =
                    unsafe { *(frame[ptr_slot as usize] as *const i64).add(im1 as usize) };
            }
            MicroOp::ArrLoad2F { dst, i: ix, j: jx, ptr_slot, len_slot, op } => {
                let len = frame[len_slot as usize];
                let im1 = frame[ix as usize].wrapping_sub(1);
                let jm1 = frame[jx as usize].wrapping_sub(1);
                if (im1 as u64) >= (len as u64) || (jm1 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: differentials pin a live 8-byte buffer, like the chain.
                let (a, b) = unsafe {
                    let ptr = frame[ptr_slot as usize] as *const i64;
                    (
                        f64::from_bits(*ptr.add(im1 as usize) as u64),
                        f64::from_bits(*ptr.add(jm1 as usize) as u64),
                    )
                };
                frame[dst as usize] = op.eval(a, b).to_bits() as i64;
            }
            MicroOp::ArrLoad2 { dst, i: ix, j: jx, ptr_a, len_a, ptr_b, len_b, op, checked } => {
                let _ = checked;
                let lena = frame[len_a as usize];
                let lenb = frame[len_b as usize];
                let im1 = frame[ix as usize].wrapping_sub(1);
                let jm1 = frame[jx as usize].wrapping_sub(1);
                if (im1 as u64) >= (lena as u64) || (jm1 as u64) >= (lenb as u64) {
                    return None;
                }
                // SAFETY: differentials pin live 8-byte buffers, like the chain.
                let (a, b) = unsafe {
                    let pa = frame[ptr_a as usize] as *const i64;
                    let pb = frame[ptr_b as usize] as *const i64;
                    (*pa.add(im1 as usize), *pb.add(jm1 as usize))
                };
                frame[dst as usize] = op.eval(a, b);
            }
            MicroOp::ArrStore { src, idx, ptr_slot, len_slot, byte, narrow32, checked } => {
                // The reference always validates — a sound proof never trips it.
                let _ = checked;
                let i = frame[idx as usize];
                let len = frame[len_slot as usize];
                let im1 = i.wrapping_sub(1);
                if (im1 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: see ArrLoad.
                unsafe {
                    if byte {
                        *(frame[ptr_slot as usize] as *mut u8).add(im1 as usize) =
                            (frame[src as usize] != 0) as u8;
                    } else if narrow32 {
                        *(frame[ptr_slot as usize] as *mut i32).add(im1 as usize) =
                            frame[src as usize] as i32;
                    } else {
                        *(frame[ptr_slot as usize] as *mut i64).add(im1 as usize) =
                            frame[src as usize];
                    }
                }
            }
            MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, op, checked } => {
                let _ = checked;
                let i = frame[idx as usize];
                let len = frame[len_slot as usize];
                let im1 = i.wrapping_sub(1);
                if (im1 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: see ArrLoad — a live 8-byte buffer is pinned.
                unsafe {
                    let cell = (frame[ptr_slot as usize] as *mut i64).add(im1 as usize);
                    *cell = op.eval(*cell, frame[operand as usize]);
                }
            }
            MicroOp::ArrCondSwap { idx1, idx2, ptr_slot, len_slot, cmp, checked } => {
                let _ = checked;
                let len = frame[len_slot as usize];
                let m1 = frame[idx1 as usize].wrapping_sub(1);
                let m2 = frame[idx2 as usize].wrapping_sub(1);
                if (m1 as u64) >= (len as u64) || (m2 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: see ArrLoad — a live 8-byte buffer is pinned.
                unsafe {
                    let ptr = frame[ptr_slot as usize] as *mut i64;
                    let a = *ptr.add(m1 as usize);
                    let b = *ptr.add(m2 as usize);
                    if cmp.eval(a, b) {
                        *ptr.add(m1 as usize) = b;
                        *ptr.add(m2 as usize) = a;
                    }
                }
            }
            MicroOp::ArrSwap { idx1, idx2, ptr_slot, len_slot, checked } => {
                let _ = checked;
                let len = frame[len_slot as usize];
                let m1 = frame[idx1 as usize].wrapping_sub(1);
                let m2 = frame[idx2 as usize].wrapping_sub(1);
                if (m1 as u64) >= (len as u64) || (m2 as u64) >= (len as u64) {
                    return None;
                }
                // SAFETY: see ArrLoad — a live 8-byte buffer is pinned.
                unsafe {
                    let ptr = frame[ptr_slot as usize] as *mut i64;
                    let a = *ptr.add(m1 as usize);
                    let b = *ptr.add(m2 as usize);
                    *ptr.add(m1 as usize) = b;
                    *ptr.add(m2 as usize) = a;
                }
            }
            MicroOp::FmaF { dst, a, b, c } => {
                let av = f64::from_bits(frame[a as usize] as u64);
                let bv = f64::from_bits(frame[b as usize] as u64);
                let cv = f64::from_bits(frame[c as usize] as u64);
                frame[dst as usize] = ((av * bv) + cv).to_bits() as i64;
            }
            MicroOp::Jump { target } => {
                pc = target;
                continue;
            }
            MicroOp::JumpIfFalse { cond, target } => {
                if frame[cond as usize] == 0 {
                    pc = target;
                    continue;
                }
            }
            MicroOp::JumpIfTrue { cond, target } => {
                if frame[cond as usize] != 0 {
                    pc = target;
                    continue;
                }
            }
            MicroOp::Branch { cmp, lhs, rhs, target } => {
                if !cmp.eval(frame[lhs as usize], frame[rhs as usize]) {
                    pc = target;
                    continue;
                }
            }
            MicroOp::MemMem {
                h_ptr_slot,
                h_len_slot,
                n_ptr_slot,
                n_len_slot,
                needle_len_slot,
                i_slot,
                count_slot,
                ..
            } => {
                // Reference model of the naive-search collapse: count overlapping
                // matches over `[i, h_len - needle_len + 1]`, add to count, and
                // advance `i` to the exit value — the same contract the runtime
                // helper implements. A needle index past its buffer would deopt
                // (the nest's checked needle `Index`): None, like the divisors.
                let h_len = frame[h_len_slot as usize];
                let n_buf_len = frame[n_len_slot as usize];
                let needle_len = frame[needle_len_slot as usize];
                let start = frame[i_slot as usize];
                if needle_len > n_buf_len {
                    return None;
                }
                let bound = h_len - needle_len + 1;
                let mut count = 0i64;
                if needle_len == 0 {
                    if start <= bound {
                        count = bound - start + 1;
                    }
                } else {
                    // SAFETY: the pinned byte buffers are live for the eval.
                    let hay = frame[h_ptr_slot as usize] as *const u8;
                    let ndl = frame[n_ptr_slot as usize] as *const u8;
                    let mut p = start; // 1-based
                    while p <= bound {
                        let mut m = true;
                        for j in 0..needle_len {
                            let hb = unsafe { *hay.add((p + j - 1) as usize) };
                            let nb = unsafe { *ndl.add(j as usize) };
                            if hb != nb {
                                m = false;
                                break;
                            }
                        }
                        if m {
                            count += 1;
                        }
                        p += 1;
                    }
                }
                frame[count_slot as usize] += count;
                frame[i_slot as usize] = core::cmp::max(start, bound + 1);
            }
            MicroOp::Return { src } => return Some(frame[src as usize]),
        }
        pc += 1;
    }
    unreachable!("validated programs cannot fall off the end")
}
