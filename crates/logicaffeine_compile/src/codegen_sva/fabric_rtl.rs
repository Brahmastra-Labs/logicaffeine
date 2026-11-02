//! fabric_rtl — Futamura's first projection in silicon: "the specialized residual IS the
//! circuit," end-to-end from the REAL compiler.
//!
//! `PE(interp, program) = compiled_program`: our compiler specializes the evaluator to a
//! program, producing register bytecode with no interpreter dispatch (`count_dispatch == 0`,
//! Jones-optimal). For a straight-line function that residual is a dispatch-free arithmetic
//! trace. This module takes that **actual compiled residual**, lowers it register→SSA to a
//! **fixed spatial dataflow circuit** — one functional unit per op, wired SSA-style, with NO
//! program counter, NO fetch, NO opcode dispatch — and proves it THREE ways: bit-identically
//! to the RUNNING VM under iverilog, across a BATTERY of distinct compiled programs directly
//! against the VM, and — where the residual is multiply-free — EXHAUSTIVELY by our OWN
//! certified SAT against an independent closed form. The circuit does not *interpret* the
//! program; it *is* the program.
//!
//! Width is **64-bit signed wrapping**. The VM's `Int` is arbitrary-precision, so the circuit
//! is its **fixed-width projection**: it computes the VM's EXACT result reduced mod 2^64, and
//! that equality holds for EVERY input, overflow included (no range restriction — the low 64
//! bits always agree). Unbounded precision is the BigInt coprocessor's domain (`bignum_rtl`).
//! Straight-line residuals (`+ - *`) lower here; COUNTED LOOPS lower to a sequential CGRA — a
//! fixed spatial body datapath streamed by a loop counter — in the `LoopFabric` section below.

/// The 64-bit datapath width — the VM's `Int` is `i64`, so the circuit wraps identically.
pub const W: u32 = 64;

/// The straight-line ALU ops a dispatch-free residual is built from (the `alu.v` ops, each
/// already proven ≡ the VM). 64-bit signed, wrapping — bit-identical to the VM's `Int`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum AluOp {
    Add,
    Sub,
    Mul,
    /// Bitwise AND / OR / XOR — combinational, bit-identical to the VM's `Int` bitwise ops (and, for 0/1
    /// Bool operands, to the logical spellings, since `&`/`|`/`^` agree with `&&`/`||`/xor on {0,1}).
    And,
    Or,
    Xor,
    /// Logical shift left `a << (b & 63)` (the VM's `wrapping_shl`, count masked mod 64).
    Shl,
    /// ARITHMETIC shift right `a >>> (b & 63)` (the VM's `wrapping_shr` on the signed `Int`).
    Shr,
    /// LOGICAL (unsigned) shift right `(a as u64) >> (b & 63)` — zero-fill, the Word64/Keccak `word64Shr`
    /// (`wrapping_shr` on the `u64` value). Distinct from [`AluOp::Shr`], which sign-extends: they agree only
    /// while the high bit is clear (why the Word32 lane, always in `0..2³²`, reuses `Shr`), but a Word64 value
    /// with bit 63 set needs this zero-fill form.
    UShr,
}

impl AluOp {
    pub fn verilog(&self, a: &str, b: &str) -> String {
        match self {
            AluOp::Add => format!("$signed({a}) + $signed({b})"),
            AluOp::Sub => format!("$signed({a}) - $signed({b})"),
            AluOp::Mul => format!("$signed({a}) * $signed({b})"),
            AluOp::And => format!("({a}) & ({b})"),
            AluOp::Or => format!("({a}) | ({b})"),
            AluOp::Xor => format!("({a}) ^ ({b})"),
            // Shift counts mask mod 64 (the wrapping spec); `>>>` on a $signed value is arithmetic.
            AluOp::Shl => format!("(({a}) << (({b}) & 64'd63))"),
            AluOp::Shr => format!("($signed({a}) >>> (({b}) & 64'd63))"),
            // LOGICAL right shift — `>>` on an $unsigned value zero-fills (Word64 `wrapping_shr` on `u64`).
            AluOp::UShr => format!("($unsigned({a}) >> (({b}) & 64'd63))"),
        }
    }
    pub fn eval(&self, a: i64, b: i64) -> i64 {
        match self {
            AluOp::Add => a.wrapping_add(b),
            AluOp::Sub => a.wrapping_sub(b),
            AluOp::Mul => a.wrapping_mul(b),
            AluOp::And => a & b,
            AluOp::Or => a | b,
            AluOp::Xor => a ^ b,
            AluOp::Shl => a.wrapping_shl(b as u32),
            AluOp::Shr => a.wrapping_shr(b as u32),
            AluOp::UShr => (a as u64).wrapping_shr(b as u32) as i64,
        }
    }
}

/// A SEQUENTIAL functional unit — signed division and remainder. Unlike an [`AluOp`], division is NOT a
/// combinational tile: on silicon it is the multi-cycle `shared_divider` (already proven ≡ the VM by
/// `rtl.rs::the_signed_divider_is_proven_correct_by_our_sat`), issued as a functional unit. Keeping it a
/// distinct fabric cell lets the emitter wire that sequential unit rather than synthesize a giant
/// combinational divider. Semantics mirror the soft-core's `shared_divider` exactly, INCLUDING the
/// divide-by-zero trap (`b == 0 → 0`).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum FuncUnit {
    /// Truncated signed quotient (`Op::Div` / `Op::ExactDiv`, `a / b` rounding toward zero).
    DivS,
    /// Signed remainder taking the dividend's sign (`Op::Mod`, `a % b`).
    ModS,
    /// Floor division (`Op::FloorDiv`, `a // b` rounding toward negative infinity).
    FloorDivS,
    /// Exponentiation (`Op::Pow`, `a ** b`) — a SEQUENTIAL multiplier (b multiplies) at the RTL level; the
    /// eval is the closed-form wrapping power. The VM promotes to bigint on overflow, so a fabric witness
    /// keeps `a**b` inside i64 (a small const exponent over the bounded fuzz range).
    Pow,
    /// UNSIGNED high-half multiply `(a·b) >> 64` (both operands as `u64`) — the reciprocal step of
    /// `Op::MagicDivU`'s magic division. On silicon it is the high word of the 64×64 multiplier; the eval
    /// takes the top 64 bits of the 128-bit unsigned product. No divider trap applies (it is a multiply).
    MulHiU,
    /// APPROXIMATE equality (`Op::ApproxEq`, `a is approximately b`) over NUMERIC operands: both converted to
    /// `f64`, compared by the relative+absolute tolerance `logos_approx_eq`. Yields 0/1. (Float-typed operands
    /// need the float lane; the fabric refuses them at the `Constant::Float` load, so only the numeric-int
    /// case reaches here.) No divider trap — it is a comparison.
    ApproxEqF,
    /// The FLOAT LANE — IEEE-754 `f64` operating on the bit-pattern each operand carries (`f64::from_bits`).
    /// Binary `FAddF`/`FSubF`/`FMulF`/`FDivF` return the result's bits; unary `FSqrtF` returns bits; `FFloorF`
    /// returns the floored value as an `Int` (the float→int bridge, matching the `floor` builtin). No divider
    /// trap (float `/0.0` is `±inf`, not the integer trap).
    FAddF,
    FSubF,
    FMulF,
    FDivF,
    FSqrtF,
    FFloorF,
    /// Signed 128-bit MULTIPLY OVERFLOW flag: `1` iff `(a as i128) * (b as i128)` does NOT fit `i64` (the
    /// `BigIntWide` lane's "did this product promote to a BigInt?" test — the VM's `Value::mul` returns an
    /// `Int` when it fits and a `BigInt` when it does not). A comparison, so no divider trap.
    MulOvfS,
    /// INT→FLOAT promotion: reinterpret the operand as a signed `i64` value and return the bit pattern of its
    /// `f64` widening (`(a as f64).to_bits()`). This is the fabric's static realization of the VM's dynamic
    /// `Int ∘ Float → Float` coercion (`arith::multiply` etc. do `*a as f64` when one side is a `Float`): when a
    /// float op has an integer operand, the fabric promotes that operand through this unit first so the float
    /// functional unit sees two genuine `f64` bit patterns. Unary (`b` ignored); no divider trap.
    IntToF,
    /// FLOAT COMPARISONS over two `f64` bit patterns, yielding the `0/1` predicate the terminator / a `Select`
    /// consumes. A genuine IEEE-754 compare (`f64::from_bits(a) < f64::from_bits(b)` etc.) — NOT the integer
    /// bit-pattern ordering a plain `Select` would use, which is only sound for same-sign operands and silently
    /// wrong once a negative float appears. `FEqF`/`FNeF` follow IEEE equality (so `NaN != NaN`). No divider trap.
    FLtF,
    FGtF,
    FLeF,
    FGeF,
    FEqF,
    FNeF,
    /// POPCOUNT (`count_ones`): the number of set bits in the operand read as a `u64`. Unary (`b` ignored),
    /// combinational (a bit-adder tree), yields an `Int` in `0..=64` — no divider trap. The static realization
    /// of the VM's `count_ones` builtin / the `popcount_leaf` hot path.
    PopCount,
}

impl FuncUnit {
    pub fn eval(&self, a: i64, b: i64) -> i64 {
        // Pow is handled BEFORE the divider trap: `a ** 0 == 1`, not 0. Negative exponents clamp to 0 (the
        // fabric models only the integer-power domain the witness exercises).
        if let FuncUnit::Pow = self {
            return a.wrapping_pow(b.max(0) as u32);
        }
        // Unsigned high-half multiply — a multiply, not a divider, so no `b == 0` trap (and `mulhi(x, 0)`
        // is naturally 0). Both operands reinterpreted as `u64` (the magic reciprocal is a `u64` constant).
        if let FuncUnit::MulHiU = self {
            return (((a as u64 as u128) * (b as u64 as u128)) >> 64) as i64;
        }
        // Approximate equality over numeric operands — reuses the kernel's exact tolerance, so the fabric
        // result is bit-identical to the VM's `is approximately`. A comparison, so no `b == 0` trap.
        if let FuncUnit::ApproxEqF = self {
            return i64::from(logicaffeine_data::ops::logos_approx_eq(a as f64, b as f64));
        }
        // The float lane — operands are f64 BIT PATTERNS. Binary ops return the result's bits; `FSqrtF`
        // returns bits; `FFloorF` returns the floored value as an integer (the float→int bridge).
        match self {
            FuncUnit::FAddF => {
                return (f64::from_bits(a as u64) + f64::from_bits(b as u64)).to_bits() as i64
            }
            FuncUnit::FSubF => {
                return (f64::from_bits(a as u64) - f64::from_bits(b as u64)).to_bits() as i64
            }
            FuncUnit::FMulF => {
                return (f64::from_bits(a as u64) * f64::from_bits(b as u64)).to_bits() as i64
            }
            FuncUnit::FDivF => {
                return (f64::from_bits(a as u64) / f64::from_bits(b as u64)).to_bits() as i64
            }
            FuncUnit::FSqrtF => return f64::from_bits(a as u64).sqrt().to_bits() as i64,
            FuncUnit::FFloorF => return f64::from_bits(a as u64).floor() as i64,
            FuncUnit::MulOvfS => {
                let p = (a as i128) * (b as i128);
                return i64::from(p < i64::MIN as i128 || p > i64::MAX as i128);
            }
            // INT→FLOAT promotion: widen the signed integer operand to its `f64` bit pattern (the VM's implicit
            // `*a as f64` when an Int meets a Float). `b` is ignored (unary).
            FuncUnit::IntToF => return (a as f64).to_bits() as i64,
            // FLOAT comparisons — decode both operands to `f64` and apply the genuine IEEE-754 relation.
            FuncUnit::FLtF => {
                return i64::from(f64::from_bits(a as u64) < f64::from_bits(b as u64))
            }
            FuncUnit::FGtF => {
                return i64::from(f64::from_bits(a as u64) > f64::from_bits(b as u64))
            }
            FuncUnit::FLeF => {
                return i64::from(f64::from_bits(a as u64) <= f64::from_bits(b as u64))
            }
            FuncUnit::FGeF => {
                return i64::from(f64::from_bits(a as u64) >= f64::from_bits(b as u64))
            }
            FuncUnit::FEqF => {
                return i64::from(f64::from_bits(a as u64) == f64::from_bits(b as u64))
            }
            FuncUnit::FNeF => {
                return i64::from(f64::from_bits(a as u64) != f64::from_bits(b as u64))
            }
            // POPCOUNT — the set-bit count of the operand read as a `u64` (unary, `b` ignored).
            FuncUnit::PopCount => return (a as u64).count_ones() as i64,
            _ => {}
        }
        if b == 0 {
            return 0; // the divider's hardware trap — matches `soft_core_divider` (b == 0 → 0)
        }
        match self {
            FuncUnit::DivS => a.wrapping_div(b),
            FuncUnit::ModS => a.wrapping_rem(b),
            FuncUnit::FloorDivS => {
                let q = a.wrapping_div(b);
                if a.wrapping_rem(b) != 0 && ((a < 0) != (b < 0)) {
                    q - 1
                } else {
                    q
                }
            }
            FuncUnit::Pow => unreachable!("Pow handled above"),
            FuncUnit::MulHiU => unreachable!("MulHiU handled above"),
            FuncUnit::ApproxEqF
            | FuncUnit::FAddF
            | FuncUnit::FSubF
            | FuncUnit::FMulF
            | FuncUnit::FDivF
            | FuncUnit::FSqrtF
            | FuncUnit::FFloorF
            | FuncUnit::MulOvfS
            | FuncUnit::IntToF
            | FuncUnit::FLtF
            | FuncUnit::FGtF
            | FuncUnit::FLeF
            | FuncUnit::FGeF
            | FuncUnit::FEqF
            | FuncUnit::FNeF
            | FuncUnit::PopCount => unreachable!("float/approx/overflow/popcount handled above"),
        }
    }
    /// A COMBINATIONAL-correct Verilog lowering with the `b == 0 → 0` guard — semantically identical to
    /// the sequential `shared_divider` (whose FSM instantiation is the area refinement). 64-bit signed,
    /// so iverilog's truncated division/remainder is bit-identical to the VM's `Int`.
    pub fn verilog(&self, a: &str, b: &str) -> String {
        match self {
            FuncUnit::DivS => format!("(($signed({b}) == 0) ? 64'sd0 : $signed({a}) / $signed({b}))"),
            FuncUnit::ModS => format!("(($signed({b}) == 0) ? 64'sd0 : $signed({a}) % $signed({b}))"),
            FuncUnit::FloorDivS => format!(
                "(($signed({b}) == 0) ? 64'sd0 : (($signed({a}) % $signed({b}) != 0 && \
                 (($signed({a}) < 0) != ($signed({b}) < 0))) ? ($signed({a}) / $signed({b})) - 64'sd1 \
                 : $signed({a}) / $signed({b})))"
            ),
            FuncUnit::Pow => panic!(
                "Pow RTL is a sequential-multiplier functional unit (a `b`-step multiply FSM), not a \
                 combinational wire — it is emitted by `emit_pow_datapath_verilog` (the sequenced power \
                 lane) and iverilog-exercised by fabric_pow_lock; the combinational tape does not emit it"
            ),
            // High-half of the 64×64 UNSIGNED product: zero-extend both operands to 128 bits (so the
            // multiply is full-width, not truncated to 64), then take bits [127:64].
            FuncUnit::MulHiU => format!("(({{64'h0, {a}}} * {{64'h0, {b}}}) >> 64)"),
            // Signed 128-bit multiply OVERFLOW flag: `1` iff `(a·b)` does NOT fit `i64`. Sign-extend both
            // operands to 128 bits (so the product is full-width, not truncated), then range-check against
            // `[i64::MIN, i64::MAX]`. Bit-identical to `FuncUnit::eval`'s i128 test (`p < MIN || p > MAX`).
            FuncUnit::MulOvfS => format!(
                "((($signed({{{{64{{{a}[63]}}}}, {a}}}) * $signed({{{{64{{{b}[63]}}}}, {b}}})) \
                 <= 128'sh0000_0000_0000_0000_7FFF_FFFF_FFFF_FFFF && \
                 ($signed({{{{64{{{a}[63]}}}}, {a}}}) * $signed({{{{64{{{b}[63]}}}}, {b}}})) \
                 >= -128'sh0000_0000_0000_0000_8000_0000_0000_0000) ? 64'sd0 : 64'sd1)"
            ),
            // APPROXIMATE equality over NUMERIC (integer) operands, bit-identical to `logos_approx_eq(a,b)`:
            // `a == b`, OR `|a-b| ≤ max(1e-9·max(|a|,|b|), 1e-12)`. Both operands are integers, so `|a-b|` and
            // `max(|a|,|b|)` are integers and the `1e-12` floor never dominates (`max(|a|,|b|) ≥ 1` whenever
            // `a ≠ b`), leaving the exact-rational test `|a-b|·10^9 ≤ max(|a|,|b|)` (evaluated in 128 bits so
            // the ×10^9 scale cannot overflow). Yields 0/1 — no divider trap (it is a comparison).
            FuncUnit::ApproxEqF => format!(
                "((($signed({a}) == $signed({b})) || \
                 ($signed(((($signed({a}) - $signed({b})) < 0) ? -($signed({a}) - $signed({b})) \
                 : ($signed({a}) - $signed({b}))) * 128'sd1000000000) <= \
                 $signed((((($signed({a}) < 0) ? -$signed({a}) : $signed({a})) > \
                 (($signed({b}) < 0) ? -$signed({b}) : $signed({b}))) ? \
                 ((($signed({a}) < 0) ? -$signed({a}) : $signed({a}))) : \
                 ((($signed({b}) < 0) ? -$signed({b}) : $signed({b}))))))) ? 64'sd1 : 64'sd0)"
            ),
            // The float→int FLOOR bridge (`FuncUnit::FFloorF`): `floor(f64_bits(a)) as i64`, bit-identical to
            // the eval (`f64::from_bits(a).floor() as i64`). Combinational IEEE-754 decode: unbias the exponent,
            // rebuild the 53-bit significand (implicit leading bit), then case on the exponent — `|a| < 1 → 0`;
            // `0 ≤ exp < 52 →` shift the significand right by `52-exp` (dropping the fraction); `exp ≥ 52 →`
            // shift left (already integral); inf/nan/`exp ≥ 63 →` the saturating `as i64` cast. For a negative
            // value, `floor = -(truncated magnitude) - (had a fraction ? 1 : 0)`. The operand `a` must be a
            // plain 64-bit wire/const (a float bit-pattern), never a wrapped expression (it is bit-indexed).
            FuncUnit::FFloorF => ffloor_verilog(a),
            // POPCOUNT — a synthesizable bit-adder tree (the 64-bit zero-extended sum of the operand's bits),
            // bit-identical to the eval `(a as u64).count_ones()`.
            FuncUnit::PopCount => popcount_verilog(a),
            FuncUnit::FAddF
            | FuncUnit::FSubF
            | FuncUnit::FMulF
            | FuncUnit::FDivF
            | FuncUnit::FSqrtF
            | FuncUnit::IntToF
            | FuncUnit::FLtF
            | FuncUnit::FGtF
            | FuncUnit::FLeF
            | FuncUnit::FGeF
            | FuncUnit::FEqF
            | FuncUnit::FNeF => panic!(
                "the float-lane RTL (an FPU functional unit binding fpu.v) is emitted by \
                 `emit_float_datapath_verilog` (the sequenced float lane), not the combinational tape — its \
                 semantics are pinned by the eval + differential + iverilog (float_soc_lock)"
            ),
        }
    }
}

/// Why a cell's fixed-width value is not the value the VM computes for the same operands. The fabric's
/// arithmetic is the 64-bit projection of the VM's; the VM is exact and total in a way no fixed-width datapath
/// is, so every point where the two part is named here, and a checked evaluation reports it instead of
/// producing the wrapped, zeroed or saturated word.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Divergence {
    /// The exact integer result leaves `i64`: the VM promotes it to a `BigInt`.
    IntOverflow,
    /// An integer division or remainder by zero: the VM fails `Division by zero` / `Modulo by zero`.
    DivideByZero,
    /// An integer base raised to a negative exponent: the VM fails.
    NegativeExponent,
    /// An integer exponent beyond `u32`: the VM fails `exponent too large`.
    ExponentTooLarge,
    /// A float divided by zero of either sign: the VM fails `Division by zero`.
    FloatDivideByZero,
}

impl Divergence {
    pub const ALL: [Divergence; 5] = [
        Divergence::IntOverflow,
        Divergence::DivideByZero,
        Divergence::NegativeExponent,
        Divergence::ExponentTooLarge,
        Divergence::FloatDivideByZero,
    ];

    /// The stable code a device reports for this divergence (0 is reserved for "none").
    pub fn code(self) -> u32 {
        match self {
            Divergence::IntOverflow => 1,
            Divergence::DivideByZero => 2,
            Divergence::NegativeExponent => 3,
            Divergence::ExponentTooLarge => 4,
            Divergence::FloatDivideByZero => 5,
        }
    }

    /// The divergence a device code names.
    pub fn from_code(code: u32) -> Option<Divergence> {
        Divergence::ALL.into_iter().find(|d| d.code() == code)
    }

    /// The stable name used in reports and accounting.
    pub fn name(self) -> &'static str {
        match self {
            Divergence::IntOverflow => "int-overflow",
            Divergence::DivideByZero => "divide-by-zero",
            Divergence::NegativeExponent => "negative-exponent",
            Divergence::ExponentTooLarge => "exponent-too-large",
            Divergence::FloatDivideByZero => "float-divide-by-zero",
        }
    }
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl AluOp {
    /// The VM's value for this op on two `Int`s, or the divergence that keeps the word from being it. `+ − ×`
    /// are exact in the VM (they promote); the bitwise ops and the masked shifts are the VM's own `i64` ops.
    pub fn eval_checked(&self, a: i64, b: i64) -> Result<i64, Divergence> {
        match self {
            AluOp::Add => a.checked_add(b).ok_or(Divergence::IntOverflow),
            AluOp::Sub => a.checked_sub(b).ok_or(Divergence::IntOverflow),
            AluOp::Mul => a.checked_mul(b).ok_or(Divergence::IntOverflow),
            AluOp::And | AluOp::Or | AluOp::Xor | AluOp::Shl | AluOp::Shr | AluOp::UShr => {
                Ok(self.eval(a, b))
            }
        }
    }
}

impl FuncUnit {
    /// The VM's value for this unit, or the divergence that keeps the word from being it. Operands follow the
    /// unit's convention (integers by value, floats as `f64` bit patterns); so does the result.
    pub fn eval_checked(&self, a: i64, b: i64) -> Result<i64, Divergence> {
        match self {
            FuncUnit::DivS => {
                if b == 0 {
                    Err(Divergence::DivideByZero)
                } else {
                    a.checked_div(b).ok_or(Divergence::IntOverflow)
                }
            }
            FuncUnit::ModS => {
                if b == 0 {
                    Err(Divergence::DivideByZero)
                } else {
                    Ok(a.wrapping_rem(b))
                }
            }
            FuncUnit::FloorDivS => {
                if b == 0 {
                    Err(Divergence::DivideByZero)
                } else if a == i64::MIN && b == -1 {
                    Err(Divergence::IntOverflow)
                } else {
                    Ok(self.eval(a, b))
                }
            }
            FuncUnit::Pow => {
                if b < 0 {
                    Err(Divergence::NegativeExponent)
                } else {
                    let e = u32::try_from(b).map_err(|_| Divergence::ExponentTooLarge)?;
                    a.checked_pow(e).ok_or(Divergence::IntOverflow)
                }
            }
            FuncUnit::FDivF => {
                if f64::from_bits(b as u64) == 0.0 {
                    Err(Divergence::FloatDivideByZero)
                } else {
                    Ok(self.eval(a, b))
                }
            }
            FuncUnit::MulHiU
            | FuncUnit::ApproxEqF
            | FuncUnit::FAddF
            | FuncUnit::FSubF
            | FuncUnit::FMulF
            | FuncUnit::FSqrtF
            | FuncUnit::FFloorF
            | FuncUnit::MulOvfS
            | FuncUnit::IntToF
            | FuncUnit::FLtF
            | FuncUnit::FGtF
            | FuncUnit::FLeF
            | FuncUnit::FGeF
            | FuncUnit::FEqF
            | FuncUnit::FNeF
            | FuncUnit::PopCount => Ok(self.eval(a, b)),
        }
    }
}

/// The combinational IEEE-754 `floor(f64) → i64` expression for [`FuncUnit::FFloorF`], fully inlined over the
/// operand `a` (a plain 64-bit float bit-pattern wire). Kept a free function so the ~15 repetitions of `a`
/// don't clutter [`FuncUnit::verilog`]. Bit-exact to `f64::from_bits(a).floor() as i64` (validated by iverilog
/// in `fabric_ffloor_lock` over a battery spanning integers, fractions, negatives, and the saturating cast).
fn ffloor_verilog(a: &str) -> String {
    // Reused sub-expressions (inlined because the emit site wants ONE expression, no helper wires).
    let exp = format!("($signed({{2'b0, {a}[62:52]}}) - 13'sd1023)"); // unbiased exponent
    let mant = format!("({a}[62:52]==11'd0 ? {{1'b0,{a}[51:0]}} : {{1'b1,{a}[51:0]}})"); // 53-bit significand
                                                                                         // truncated-toward-zero magnitude of floor(|a|):
    let tmag = format!(
        "({a}[62:52] == 11'd2047 ? 64'h7FFFFFFFFFFFFFFF : \
         ({exp} < 0 ? 64'd0 : \
         ({exp} >= 63 ? 64'hFFFFFFFFFFFFFFFF : \
         ({exp} >= 52 ? ({{11'd0, {mant}}} << ({exp} - 52)) \
         : ({{11'd0, {mant}}} >> (52 - {exp}))))))"
    );
    // whether the truncation dropped any fraction bits (only in the 0 ≤ exp < 52 shift-right branch):
    let had_frac = format!(
        "({a}[62:52] == 11'd2047 ? 1'b0 : \
         ({exp} < 0 ? ({mant} != 53'd0) : \
         ({exp} >= 52 ? 1'b0 : \
         ((({{11'd0, {mant}}}) & ((64'd1 << (52 - {exp})) - 64'd1)) != 64'd0))))"
    );
    // floor: positive → truncation; negative → -(magnitude) - (had_frac ? 1 : 0).
    format!("({a}[63] ? (-$signed({tmag}) - ({had_frac} ? 64'sd1 : 64'sd0)) : $signed({tmag}))")
}

/// The combinational POPCOUNT expression for [`FuncUnit::PopCount`]: the 64-bit zero-extended sum of the
/// operand's individual bits (`{63'd0, a[0]} + {63'd0, a[1]} + …`). A plain bit-adder tree — synthesizable
/// (yosys maps it to LUT adders) and bit-identical to `(a as u64).count_ones()`. The operand `a` must be a
/// plain 64-bit wire/const (it is bit-indexed).
fn popcount_verilog(a: &str) -> String {
    let terms = (0..64)
        .map(|i| format!("{{63'd0, {a}[{i}]}}"))
        .collect::<Vec<_>>()
        .join(" + ");
    format!("({terms})")
}

/// An operand of a residual op: a circuit input (a function parameter), a prior SSA value, or
/// a compile-time constant (already specialized in by the partial evaluator).
#[derive(Clone, Copy, Debug, PartialEq, Hash, serde::Serialize, serde::Deserialize, Eq)]
pub enum Src {
    In(usize),
    Val(usize),
    Const(i64),
}

impl Src {
    fn verilog(&self) -> String {
        match self {
            Src::In(k) => format!("in{k}"),
            Src::Val(k) => format!("v{k}"),
            Src::Const(c) => {
                if *c < 0 {
                    format!("-64'sd{}", (*c as i128).unsigned_abs())
                } else {
                    format!("64'sd{c}")
                }
            }
        }
    }
    pub fn eval(&self, inputs: &[i64], vals: &[i64]) -> i64 {
        match self {
            Src::In(k) => inputs[*k],
            Src::Val(k) => vals[*k],
            Src::Const(c) => *c,
        }
    }
}

/// One residual operation: `v_i = a op b`.
#[derive(Clone, Copy, Debug, PartialEq, Hash, serde::Serialize, serde::Deserialize, Eq)]
pub struct FabricOp {
    pub op: AluOp,
    pub a: Src,
    pub b: Src,
}

/// Emit the residual as a fixed dataflow circuit — one combinational `wire` per SSA value, no
/// PC / fetch / dispatch. `result` names the SSA value driving the output. 64-bit, so the
/// circuit's wrapping arithmetic is bit-identical to the VM's `Int`.
pub fn emit_fabric_verilog(name: &str, n_in: usize, ops: &[FabricOp], result: usize) -> String {
    let inputs = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut body = String::new();
    for (i, op) in ops.iter().enumerate() {
        body.push_str(&format!(
            "  wire [63:0] v{i} = {};\n",
            op.op.verilog(&op.a.verilog(), &op.b.verilog())
        ));
    }
    format!(
        "// fabric_{name} — a compiled dispatch-free residual as a fixed dataflow circuit.\n\
         // No program counter, no fetch, no opcode dispatch: the residual's ops are WIRED\n\
         // (SSA), not interpreted. Futamura P1: the specialized residual IS the circuit.\n\
         // Lowered from the REAL compiler's residual; 64-bit so it wraps identically to the\n\
         // VM's Int; proven == the running VM by iverilog.\n\
         module fabric_{name}({inputs}, output [63:0] result);\n\
         {body}  assign result = v{result};\n\
         endmodule\n"
    )
}

/// Evaluate the residual op-by-op (64-bit wrapping) — the reference the circuit is checked
/// against, identical to the VM's arithmetic.
pub fn eval_fabric(ops: &[FabricOp], inputs: &[i64], result: usize) -> i64 {
    let mut vals: Vec<i64> = Vec::with_capacity(ops.len());
    for op in ops {
        let a = op.a.eval(inputs, &vals);
        let b = op.b.eval(inputs, &vals);
        vals.push(op.op.eval(a, b));
    }
    vals[result]
}

/// **The genuine Futamura P1 lowering.** Compile `src` with the real compiler, take the
/// straight-line register bytecode of function `fn_name` (its dispatch-free residual), and
/// lower it register→SSA to a dataflow op-list. Parameters become circuit inputs; `LoadConst`
/// becomes a baked-in constant; `Add/Sub/Mul` become wired functional units. Also returns the
/// input arity (parameter count). Panics on a non-straight-line op (control flow / unsupported)
/// — the honest boundary of this block.
pub fn residual_from_source(src: &str, fn_name: &str) -> (Vec<FabricOp>, usize, usize) {
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::HashMap;

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _pol) = parsed.expect("parse failed");
        let program = Compiler::compile(stmts, interner).expect("compile failed");
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found in the compiled program"));

        // The function's straight-line body: entry_pc .. first Return/ReturnNothing.
        let mut body: Vec<Op> = Vec::new();
        for op in &program.code[f.entry_pc..] {
            body.push(*op);
            if matches!(op, Op::Return { .. } | Op::ReturnNothing) {
                break;
            }
        }

        // register → Src (SSA). Parameters occupy R0..param_count.
        let mut reg: HashMap<u16, Src> = HashMap::new();
        for p in 0..f.param_count {
            reg.insert(p, Src::In(p as usize));
        }
        let mut ops: Vec<FabricOp> = Vec::new();
        let mut result: Option<Src> = None;
        let get = |reg: &HashMap<u16, Src>, r: &u16| -> Src {
            *reg.get(r)
                .unwrap_or_else(|| panic!("residual reads unwritten register R{r}"))
        };
        for op in &body {
            let binop = |aop: AluOp,
                         dst: &u16,
                         lhs: &u16,
                         rhs: &u16,
                         reg: &mut HashMap<u16, Src>,
                         ops: &mut Vec<FabricOp>| {
                let k = ops.len();
                ops.push(FabricOp {
                    op: aop,
                    a: get(reg, lhs),
                    b: get(reg, rhs),
                });
                reg.insert(*dst, Src::Val(k));
            };
            match op {
                Op::LoadConst { dst, idx } => {
                    let v = match program.constants[*idx as usize] {
                        Constant::Int(n) => n,
                        ref c => panic!("fabric residual: non-integer constant {c:?}"),
                    };
                    reg.insert(*dst, Src::Const(v));
                }
                Op::Move { dst, src } => {
                    let s = get(&reg, src);
                    reg.insert(*dst, s);
                }
                Op::Add { dst, lhs, rhs } => binop(AluOp::Add, dst, lhs, rhs, &mut reg, &mut ops),
                Op::Sub { dst, lhs, rhs } => binop(AluOp::Sub, dst, lhs, rhs, &mut reg, &mut ops),
                Op::Mul { dst, lhs, rhs } => binop(AluOp::Mul, dst, lhs, rhs, &mut reg, &mut ops),
                // Fixed-point rescale: the Q-format `shifted right/left by` — the tiles already
                // exist (AluOp::Shr/Shl), residual extraction just accepts them. Lets a quantized
                // convolution stencil lower to a dataflow circuit.
                Op::Shr { dst, lhs, rhs } => binop(AluOp::Shr, dst, lhs, rhs, &mut reg, &mut ops),
                Op::Shl { dst, lhs, rhs } => binop(AluOp::Shl, dst, lhs, rhs, &mut reg, &mut ops),
                // Bitwise ops complete the fixed-point/branch-free toolkit — e.g. a Sobel edge
                // detector's `|x| = (x ^ (x>>>63)) - (x>>>63)` needs Xor. Tiles already exist.
                Op::BitXor { dst, lhs, rhs } => {
                    binop(AluOp::Xor, dst, lhs, rhs, &mut reg, &mut ops)
                }
                Op::BitAnd { dst, lhs, rhs } => {
                    binop(AluOp::And, dst, lhs, rhs, &mut reg, &mut ops)
                }
                Op::BitOr { dst, lhs, rhs } => binop(AluOp::Or, dst, lhs, rhs, &mut reg, &mut ops),
                Op::Return { src } => result = Some(get(&reg, src)),
                Op::ReturnNothing => {}
                other => panic!(
                    "fabric residual: `{}` is not a straight-line arithmetic op (control flow / \
                     unsupported — loops map to a CGRA, roadmap)",
                    other.name()
                ),
            }
        }
        let result = match result {
            Some(Src::Val(k)) => k,
            other => panic!("fabric residual result is not a computed value: {other:?}"),
        };
        (ops, result, f.param_count as usize)
    })
}

/// Lower a residual op-list to a `BoundedExpr` (bit-vector datapath) at the given width — the
/// bridge that lets our certified SAT reason about the emitted circuit exhaustively.
pub fn fabric_to_bounded(
    ops: &[FabricOp],
    result: usize,
    width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    fn src_expr(s: Src, vals: &[BoundedExpr], width: u32) -> BoundedExpr {
        match s {
            Src::In(k) => BoundedExpr::BitVecVar(format!("in{k}"), width),
            Src::Val(k) => vals[k].clone(),
            Src::Const(c) => BoundedExpr::BitVecConst {
                width,
                value: c as u64,
            },
        }
    }
    let mut vals: Vec<BoundedExpr> = Vec::with_capacity(ops.len());
    for op in ops {
        let a = src_expr(op.a, &vals, width);
        let b = src_expr(op.b, &vals, width);
        let bop = match op.op {
            AluOp::Add => BitVecBoundedOp::Add,
            AluOp::Sub => BitVecBoundedOp::Sub,
            AluOp::Mul => BitVecBoundedOp::Mul,
            AluOp::And => BitVecBoundedOp::And,
            AluOp::Or => BitVecBoundedOp::Or,
            AluOp::Xor => BitVecBoundedOp::Xor,
            AluOp::Shl => BitVecBoundedOp::Shl,
            AluOp::Shr => BitVecBoundedOp::AShr,
            AluOp::UShr => BitVecBoundedOp::Shr,
        };
        vals.push(BoundedExpr::BitVecBinary {
            op: bop,
            left: Box::new(a),
            right: Box::new(b),
        });
    }
    vals[result].clone()
}

/// The demonstration residual: a degree-3 polynomial in Horner form, `2x³ + 5x² − 3x + 100`.
/// Straight-line (`+ - *`), so the compiler residualizes it dispatch-free.
pub const POLY_BODY: &str = "(((2 * x + 5) * x - 3) * x + 100)";

/// The full program, with `Main` chosen by the caller (`Show poly(N).`).
pub fn poly_program(main: &str) -> String {
    format!("## To poly (x: Int) -> Int:\n    Return {POLY_BODY}.\n## Main\n    {main}")
}

/// A self-checking testbench: drive N inputs, compare `result` against the VM oracle from
/// `fabric_{in,out}.hex`. Prints PASS/FAIL. 64-bit values.
pub fn emit_fabric_testbench(name: &str, n: usize) -> String {
    format!(
        r#"// tb_fabric_{name} — the dataflow circuit vs the RUNNING VM's output for the program.
`timescale 1ns/1ps
module tb_fabric_{name};
  reg  [63:0] in0;
  reg  [63:0] oin [0:{last}];
  reg  [63:0] oout [0:{last}];
  wire [63:0] result;
  integer i, errors = 0;
  fabric_{name} dut(.in0(in0), .result(result));
  initial begin
    $readmemh("fabric_in.hex", oin);
    $readmemh("fabric_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      in0 = oin[i]; #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL eval %0d", i); end
    end
    if (errors == 0) $display("PASS: {n} circuit evaluations match the VM (residual = circuit, no interpreter)");
    else $display("FAIL: %0d fabric mismatches", errors);
    $finish;
  end
endmodule
"#,
        name = name,
        last = n - 1,
        n = n,
    )
}

// ===========================================================================================
// CGRA — the counted-loop projection: control flow in silicon (Futamura P1, sequential).
//
// A straight-line residual is a fixed SPATIAL circuit (above). A COUNTED LOOP is a fixed
// spatial BODY datapath plus a sequential controller that streams the loop-carried state
// through it once per induction value — exactly how a Coarse-Grained Reconfigurable Array runs
// a loop: the array is configured ONCE with the body; iteration is TEMPORAL (clock the state
// through the fixed fabric), not fetched/dispatched. `Repeat for i from lo to hi:` compiles to
// a range iterator (`NewRange`/`IterNext`) over an accumulator; we recover the loop-carried
// state, the induction variable, and the straight-line body, and lower them to that CGRA.
//
// Soundness is k-induction made physical: the BODY's one-step next-state is certified ≡ the
// VM's per-iteration transition EXHAUSTIVELY by our own SAT (the induction STEP); the controller
// counts the induction variable across exactly `lo..=hi` (the base + trip count); so the CGRA's
// final state equals the VM's loop result for every input, by induction over the iterations.
// ===========================================================================================

/// A signed comparison (the VM's `Int` order) that gates a predicated select.
#[derive(Clone, Copy, Debug, PartialEq, Hash, serde::Serialize, serde::Deserialize, Eq)]
pub enum CmpKind {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

impl CmpKind {
    pub fn eval(&self, a: i64, b: i64) -> bool {
        match self {
            CmpKind::Lt => a < b,
            CmpKind::Gt => a > b,
            CmpKind::Le => a <= b,
            CmpKind::Ge => a >= b,
            CmpKind::Eq => a == b,
            CmpKind::Ne => a != b,
        }
    }
    /// The signed Verilog relational operator.
    pub fn verilog(&self) -> &'static str {
        match self {
            CmpKind::Lt => "<",
            CmpKind::Gt => ">",
            CmpKind::Le => "<=",
            CmpKind::Ge => ">=",
            CmpKind::Eq => "==",
            CmpKind::Ne => "!=",
        }
    }
}

/// One body cell of the CGRA fabric: an arithmetic tile, or a predicated SELECT (mux) tile whose
/// comparison is folded in. A branch in the loop body is if-CONVERTED to a `Select`: both arms
/// compute, the signed predicate selects — control becomes data, no divergence.
#[derive(Clone, Copy, Debug, PartialEq, Hash, serde::Serialize, serde::Deserialize, Eq)]
pub enum LoopCell {
    /// `v = a op b`.
    Arith { op: AluOp, a: Src, b: Src },
    /// `v = (ca cmp cb) ? t : e` — the CGRA mux tile, the predicate folded in.
    Select {
        cmp: CmpKind,
        ca: Src,
        cb: Src,
        t: Src,
        e: Src,
    },
    /// `v = a `unit` b` — a SEQUENTIAL functional-unit tile (signed division/remainder via the
    /// `shared_divider`). Distinct from `Arith` so the emitter wires the multi-cycle divider, not a
    /// combinational one. See [`FuncUnit`].
    Func { unit: FuncUnit, a: Src, b: Src },
    /// `v = seq[addr]` — a MEMORY-read tile reading the loop's iterated sequence (see
    /// [`LoopFabric::seq_input`]) at `addr` (the induction variable). On silicon this is a scratchpad
    /// read (the induction counter drives the address bus); in the model it indexes the sequence array.
    MemRead { addr: Src },
    /// `v = heap[addr]` — a HEAP-read tile (the shared state plane). The heap mirrors the VM's exact
    /// layout: a collection at pointer `p` is `heap[p] = length` then its elements at `heap[p+1..=p+len]`
    /// (1-based). `length of xs` is `HeapRead(p)`; `item i of xs` is `HeapRead(p + i)`. This is the
    /// collection-read primitive the whole state plane composes from (with `HeapWrite`/`HeapAlloc`).
    HeapRead { addr: Src },
    /// `heap[addr] = val` (side effect); the tile's value is `val`. `SetIndex`/`ListPush` compose from this.
    HeapWrite { addr: Src, val: Src },
    /// Allocate `size` fresh words at the heap top (zeroed); the tile's value is the NEW pointer. `NewList`
    /// composes an alloc + writes; value semantics (COW) are handled upstream by `DeepClone`/`EnsureOwned`.
    HeapAlloc { size: Src },
    /// A packed-SIMD tile: a `VecOp` (lane-wise add / bitwise) over two operand words for lane type `ty`.
    /// First-class placeable — routes to a `CellKind::Simd` tile exactly like `Arith`; combinational; proven
    /// ≡ N independent scalar ops by `simd_tile_lock` (SAT carry-isolation) + iverilog.
    VectorArith {
        op: super::simd::VecOp,
        ty: super::simd::LaneTy,
        a: Src,
        b: Src,
    },
}

/// A counted-loop residual lowered to a CGRA: a fixed BODY datapath (SSA over the current
/// loop-carried state + the induction variable + loop invariants, INCLUDING predicated select
/// tiles for data-dependent branches) driven by a sequential controller that clocks the state
/// through the body once per induction value in the inclusive range `lo..=hi`.
#[derive(Clone, Debug)]
pub struct LoopFabric {
    /// Circuit input arity (the function's parameters become circuit inputs `in0..`).
    pub n_in: usize,
    /// Number of loop-carried state registers (the accumulators).
    pub n_state: usize,
    /// Initial value of each state register, as a `Src` over the circuit inputs / constants.
    pub init: Vec<Src>,
    /// Inclusive induction range: the induction variable takes `lo, lo+1, …, hi`. Over inputs.
    pub lo: Src,
    pub hi: Src,
    /// Loop-invariant values read by the body (fixed across iterations), as `Src` over inputs.
    pub invariants: Vec<Src>,
    /// The body as SSA `LoopCell`s. Body-input convention for `Src::In(k)`: `0..n_state` = the
    /// current state registers, `n_state` = the induction variable, `n_state+1+j` = invariant `j`.
    pub body: Vec<LoopCell>,
    /// For each state register, the `Src` (over body inputs / body vals) giving its NEXT value
    /// after one execution of the body.
    pub next: Vec<Src>,
    /// Which state register holds the loop's result (returned after the loop).
    pub result_state: usize,
    /// `Some(k)` marks a SEQUENCE loop (`Repeat for x in xs`) rather than a numeric range loop: input
    /// `k` is the iterated `Seq` (a memory array), the induction range is `0..=len-1` derived from the
    /// sequence's length at eval time, and the body reads `xs[iv]` through a [`LoopCell::MemRead`] tile.
    /// `None` is a plain counted range loop (`lo`/`hi` are the numeric bounds). This is "collections as a
    /// spatial resource": the induction variable IS the address generator streaming the array.
    pub seq_input: Option<usize>,
    /// The POST-LOOP transform (an epilogue island): straight-line `LoopCell`s over the loop's FINAL
    /// state (`Src::In(i)` = final value of state register `i`), computing the returned value when the
    /// function does more than return an accumulator (e.g. `Return (sum 1..n) * 2`). Empty when the loop
    /// result IS a carried register — then [`Self::result_state`] names it. This is the first CFG
    /// COMPOSITION: a loop island feeding a transform island, sequenced into one whole-function fabric.
    pub epilogue: Vec<LoopCell>,
    /// When `epilogue` is non-empty, the `Src` (over the final state / epilogue vals) giving the result.
    pub epilogue_result: Src,
    /// The PROLOGUE HEAP ISLAND: straight-line `LoopCell`s that build any LOCAL collection the loop iterates
    /// (`Let ps be [(1,2),(3,4)]` → `HeapAlloc`/`HeapWrite`). Evaluated ONCE before the loop into a shared
    /// heap; the loop body then reads/writes that same heap. Empty for a numeric / parameter-sequence loop.
    pub prologue_heap: Vec<LoopCell>,
    /// `Some(ptr)` marks a FOREACH OVER A LOCAL HEAP LIST (`Repeat for (a,b) in ps`, `ps` a local): `ptr`
    /// (a `Src` over the prologue-heap island) is the iterated list's pointer, its length is `heap[ptr]`, and
    /// element `iv` is `heap[ptr + 1 + iv]` — the body destructures it via `HeapRead` with the heap threaded.
    /// Distinct from [`Self::seq_input`] (a scalar parameter array). `ptr` is passed to the body as the FINAL
    /// invariant so the element-address generator can read it.
    pub seq_heap_ptr: Option<Src>,
}

/// Render a `Src` with caller-supplied names for inputs and SSA vals (so the body can name its
/// inputs `s0/iv/inv0` while the straight-line emitter names circuit inputs `in0`).
fn src_named(
    s: &Src,
    name_in: &dyn Fn(usize) -> String,
    name_val: &dyn Fn(usize) -> String,
) -> String {
    match s {
        Src::In(k) => name_in(*k),
        Src::Val(k) => name_val(*k),
        Src::Const(c) => {
            if *c < 0 {
                format!("-64'sd{}", (*c as i128).unsigned_abs())
            } else {
                format!("64'sd{c}")
            }
        }
    }
}

/// A promoted GLOBAL's fabric slot: in straight-line combinational code a module-level `Let` referenced
/// inside a function is a pure DATAFLOW WIRE from its (single, set-before-use) definition to every use — no
/// heap cell. The `Src` is that wire; the metadata (collection static length, text-ness, struct type)
/// rides along exactly as it does through a `Move`, so a global holding a list/text/struct stays typed.
#[derive(Clone, Copy)]
struct GSlot {
    src: Src,
    len: Option<i64>,
    text: bool,
    ty: Option<u32>,
}

/// How a single-field policy `Check` decides its GRANT, extracted from the `PolicyRegistry` at build. Each
/// variant mirrors EXACTLY what the VM's `evaluate_policy_condition` computes for that field/literal, so the
/// fabric grant is bit-identical to the VM's pass/fail — the security property (rule 8).
#[derive(Clone, Debug)]
enum PolicyGrant {
    /// A Bool/Int field compared to a parseable literal: grant ⇔ `subject.field == expected`
    /// (Bool rides its slot as 0/1). The VM's `*n == e` / `*b == e`.
    IntEq { slot: usize, expected: i64 },
    /// A Text field compared to a literal: grant ⇔ `subject.field` (a char-list) equals `expected` byte for
    /// byte. The VM's `s.as_str() == expected`.
    TextEq { slot: usize, expected: Vec<u8> },
    /// The VM's compare is UNSATISFIABLE, so the check always denies: a Bool/Int field against a literal the
    /// VM can't parse to that type (e.g. `level equals 5` — the number is not captured, resolving to the
    /// `"unknown"` sentinel — or a quoted string against a Bool/Int field, where the VM returns `false`).
    /// Modeled as a constant-0 grant so the fabric matches the VM's always-deny instead of refusing.
    AlwaysDeny,
}

/// The temporal kind a register holds, so a date-component builtin (`the hour of …`) picks the right decode:
/// a `Moment` is nanoseconds since the epoch (`LoadNow`), a `Date` is whole days (`LoadToday`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TimeKind {
    Moment,
    Date,
}

thread_local! {
    /// `NewStruct`'s field count keyed by its `type_name` constant index — the ONE piece of program
    /// context `exec_ops` needs that the op itself doesn't carry (`GetField`/`StructInsert` carry the field
    /// index directly). Populated by `decision_fabric_from_source` from `program.struct_types` before it
    /// lowers, read by the `NewStruct` arm. A thread-local (like `NET_OFFLINE`) avoids threading a param
    /// through every `exec_ops`/`dec_convert` call site for a single struct-shape lookup.
    static STRUCT_FIELDS: std::cell::RefCell<std::collections::HashMap<u32, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// `(struct type_name const, field_name const) -> 0-based slot position`. Resolves `GetField`/
    /// `StructInsert`'s `field` (a constant index to the field NAME) to a heap offset — the field lives at
    /// `heap[ptr + 1 + position]`. Populated per program from `struct_types`.
    static STRUCT_FIELD_POS: std::cell::RefCell<std::collections::HashMap<(u32, u32), usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// `register -> struct type_name const` — register TYPE-TRACKING. A register's type is BRANCH-INVARIANT
    /// (a variable has one type on every path), so a single shared map suffices (no per-branch cloning). Set
    /// at `NewStruct`, propagated through `Move`, read at `GetField`/`StructInsert` to pick the field slot.
    static REG_TYPE: std::cell::RefCell<std::collections::HashMap<u16, u32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// The whole program's code + per-function `(entry_pc, param_count)`, so a `Call` can INLINE its callee's
    /// decision fabric (the Futamura projection: specializing the interpreter for the call site). Set by
    /// `decision_fabric_from_source`; `INLINE_DEPTH` guards against a recursive/too-deep call graph (which
    /// would inline forever — a spatial call STACK, not inlining, is the honest answer there).
    static INLINE_CODE: std::cell::RefCell<Vec<crate::vm::Op>> = std::cell::RefCell::new(Vec::new());
    static INLINE_FNS: std::cell::RefCell<Vec<(usize, u16)>> = std::cell::RefCell::new(Vec::new());
    static INLINE_DEPTH: std::cell::RefCell<usize> = std::cell::RefCell::new(0);

    /// `register -> STATIC collection length`, tracked through straight-line construction (`NewList`/
    /// `NewTuple` count, `NewEmptyList` 0, ±1 per push/pop, propagated by `Move`). The combinational heap
    /// model already only supports straight-line heap mutation (a branchy `HeapWrite` fires unconditionally),
    /// so this length is EXACT within that same scope — it lets `Contains` unroll into a bounded OR-scan.
    static COLL_LEN: std::cell::RefCell<std::collections::HashMap<u16, i64>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// The whole-program output TAPE: for each `Show`, the value `Src` and whether it is TEXT (a char-list
    /// pointer, rendered as its string) vs an Int (rendered as decimal). A `Show` is a side effect, so it is
    /// only sound in straight-line code (the whole-program builder refuses a branch).
    static SHOW_OUTPUTS: std::cell::RefCell<Vec<(Src, bool, Option<Src>)>> =
        std::cell::RefCell::new(Vec::new());

    /// Registers currently holding TEXT (a char-list `[len, byte0..]` on the heap). Set at a Text `LoadConst`
    /// / `Concat` / `FormatValue`, propagated by `Move` — so `Show` renders the value as a string, not a
    /// number. (Text type is branch-invariant, like a struct type, so a shared set suffices.)
    static TEXT_REGS: std::cell::RefCell<std::collections::HashSet<u16>> =
        std::cell::RefCell::new(std::collections::HashSet::new());

    /// Registers holding a Text constant, by its constant index: the NAME a struct field is keyed by when a
    /// place mutation writes it back (`SetIndex obj[name] = v`). Set at a Text `LoadConst`, carried by `Move`.
    static NAME_REGS: std::cell::RefCell<std::collections::HashMap<u16, u32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// Registers currently holding a FLOAT (an `f64` bit pattern in an i64 slot). Set at a `Constant::Float`
    /// load / float arithmetic / `Sqrt`, propagated by `Move`. Routes `Add`/`Sub`/`Mul`/`Div` through the
    /// float functional units instead of the integer ALU, and makes `Show` refuse (float→string is a
    /// follow-on). Type is branch-invariant, so a shared set suffices (like `TEXT_REGS`).
    static FLOAT_REGS: std::cell::RefCell<std::collections::HashSet<u16>> =
        std::cell::RefCell::new(std::collections::HashSet::new());

    /// Registers holding a WORD32 (ℤ/2³²) value — set by a `word32()` construction or any word32 builtin
    /// (`word_and`/`word_or`/`word_not`/`rotl`/`rotr`/`word32Shr`), propagated by `Move` and the bitwise ops,
    /// cleared by `intOfWord32`. Makes integer `Add`/`Sub`/`Mul` on a word32 operand WRAP mod 2³² (mask to 32
    /// bits) — the SHA-256 round add. Branch-invariant, so a shared set suffices (like `FLOAT_REGS`).
    static W32_REGS: std::cell::RefCell<std::collections::HashSet<u16>> =
        std::cell::RefCell::new(std::collections::HashSet::new());

    /// Registers holding a collection whose ELEMENTS are floats (a `Seq of Float`). Set when a float value is
    /// pushed / stored into a collection (or a `NewList` of float elements), propagated by `Move`. Read at
    /// `Index`/`ListPop` to mark the read register float — the float-in-Seq bridge, so `item i of v` off a float
    /// Seq flows into the float functional units (nbody/spectral). Branch-invariant, so a shared set suffices.
    static FLOAT_COLLS: std::cell::RefCell<std::collections::HashSet<u16>> =
        std::cell::RefCell::new(std::collections::HashSet::new());

    /// `register -> "is this value a BigInt?" flag Src` for the `BigIntWide` lane. A value PROMOTES to a
    /// BigInt when an `Add`/`Mul` overflows `i64` (the VM's `Value` returns `BigInt` then, and `Low` of a
    /// BigInt is 0). Bigness is MONOTONE under `Add`/`Mul` (only `Sub` could un-promote, which is why `Sub`
    /// is refused), so the fabric tracks a running 0/1 flag; `Low` gates on it, and `Show` refuses a BigInt.
    static BIGINT_BIG: std::cell::RefCell<std::collections::HashMap<u16, Src>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// Promoted globals: `global index -> GSlot`. A module-level `Let` referenced from a function compiles
    /// to `GlobalSet` (at the definition) + `GlobalGet` (at each use). In the straight-line fabric the value
    /// is set before it is read, so the map carries the definition's wire to every reader — a global is a
    /// wire, not a cell (like `Move`, which is why `GlobalGet`/`GlobalSet` classify as `Wire`).
    static GLOBALS: std::cell::RefCell<std::collections::HashMap<u16, GSlot>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// The OFFLINE-networking loopback inbox: a FIFO (same `[read_cursor, count, ..]` shape as a channel)
    /// established by `NetListen`. `NetSend` PUBLISHES into it (output-transparent until drained) and
    /// `NetAwait` DRAINS it — a `Send … to <self> ; Await … from <self>` round-trips locally, exactly the
    /// deterministic single-node loopback the net oracle uses. `None` until the first `Listen`.
    static NET_INBOX: std::cell::RefCell<Option<Src>> = std::cell::RefCell::new(None);

    /// A `select`'s pending arms, in declaration order: `Some((channel_ptr, var))` for a `Receive` arm,
    /// `None` for an `After …` timeout arm. `SelectArmRecv`/`SelectArmTimeout` push; `SelectWait` consumes
    /// them to resolve the winner (first non-empty channel, else the timeout) and drains only the winner.
    static SELECT_ARMS: std::cell::RefCell<Vec<Option<(Src, u16)>>> =
        std::cell::RefCell::new(Vec::new());

    /// Set once a `FailWith` fires: the program has raised an error, so NOTHING it would `Show` afterward
    /// reaches stdout (the VM returns `Err` at that point). The whole-program output tape suppresses every
    /// subsequent `Show`, so the fabric's tape equals the VM's output up to the failure. Reset per build.
    static HALTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

    /// `register -> TimeKind` — a register holding a clock value (`LoadNow` → Moment nanos, `LoadToday` →
    /// Date days), propagated by `Move`. Read by a `CallBuiltin` date-component extractor so `the hour of`
    /// decodes nanos-of-day and `the weekday of` decodes days — the fabric reads the SAME injected clock the
    /// VM does (`temporal::fixed_clock`), so the extracted Int is bit-identical. Type is branch-invariant.
    static REG_TIMEKIND: std::cell::RefCell<std::collections::HashMap<u16, TimeKind>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// A `MakeClosure` result: `register -> (body function index, captured (value, present) pairs)`. The
    /// closure's body is a STATIC function index, so a made-then-called closure inlines like a `Call` whose
    /// callee is resolved from this map (the Futamura projection of dynamic dispatch). Captures are gathered
    /// at `MakeClosure` time (locals from `locals_start`, globals from `GLOBALS`) in the body's capture order.
    #[allow(clippy::type_complexity)]
    static CLOSURES: std::cell::RefCell<std::collections::HashMap<u16, (usize, Vec<(Src, bool)>)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// Per-function capture layout: `function index -> [None for a local capture | Some(global index)]`, in
    /// the body's `captures` order. Populated from `program.functions[*].captures` so `MakeClosure` knows how
    /// many locals to read from `locals_start` and which captures snapshot a global.
    static CLOSURE_CAPTURES: std::cell::RefCell<Vec<Vec<Option<u16>>>> =
        std::cell::RefCell::new(Vec::new());

    /// The program's MODELED policy conditions: `(subject type-name const idx, predicate/action symbol
    /// index, is_capability) -> PolicyGrant`. A `Check` decides a GRANT predicate from the field's value; the
    /// `PolicyRegistry` that carries the conditions lives in the PARSE result (not the `CompiledProgram`), so
    /// `whole_program_fabric_from_source` extracts them here at build. Only the SINGLE-FIELD conditions the
    /// fabric can faithfully model are inserted (`FieldBool` / `FieldEquals` on a Bool/Int/Text field, incl.
    /// the always-deny case for a literal the VM can't parse); a compound (And/Or) / object-cross-field /
    /// predicate-indirection condition is left OUT so the `CheckPolicy` arm PANICS on it rather than silently
    /// granting — a wrong grant is a security hole (rule 8), so an unmodeled check fails loud, never quietly.
    static POLICY_CONDITIONS: std::cell::RefCell<std::collections::HashMap<(u32, u32, bool), PolicyGrant>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// The running policy GRANT guard: an AND-chain (a 0/1 `Src`) of every `CheckPolicy` grant evaluated so
    /// far in program order. `None` = no check yet = unconditionally granted. Each `Show` records the guard
    /// in force at its point; `eval_whole_program` HALTS the output tape at the first `Show` whose guard is
    /// false — the observable equivalent of the VM returning `Err` at a failed `Check` (all later output
    /// suppressed). Grants are monotone-ANDed forward, so once false it stays false (halt = stop at first).
    static POLICY_GUARD: std::cell::RefCell<Option<Src>> = std::cell::RefCell::new(None);
}

/// Resolve a `GetField`/`StructInsert` field (a constant index to the field NAME) to its 0-based heap slot,
/// using the object register's tracked struct type (`REG_TYPE`, set at `NewStruct`) and the per-type
/// name→slot map (`STRUCT_FIELD_POS`). The field then lives at `heap[ptr + 1 + slot]`.
fn struct_field_slot(obj_reg: u16, field_const: u32) -> usize {
    let tn = REG_TYPE.with(|m| m.borrow().get(&obj_reg).copied()).expect(
        "struct field access on a register with no tracked struct type \
                 (param-passed structs need param-type seeding — not yet wired)",
    );
    STRUCT_FIELD_POS
        .with(|m| m.borrow().get(&(tn, field_const)).copied())
        .expect("struct field slot not found for (type, field) — STRUCT_FIELD_POS not populated for this type")
}

/// [`struct_field_slot`] for a field named by the Text constant `name` rather than by the constant a `GetField`
/// carries: the slot of whichever of the object's fields has that name.
fn struct_field_slot_named(obj_reg: u16, name: u32, consts: &[crate::vm::Constant]) -> usize {
    let tn = REG_TYPE
        .with(|m| m.borrow().get(&obj_reg).copied())
        .expect("struct field write on a register with no tracked struct type");
    let text = |i: u32| match consts.get(i as usize) {
        Some(crate::vm::Constant::Text(s)) => Some(s.as_str()),
        _ => None,
    };
    let wanted = text(name).expect("a struct field is keyed by a Text constant");
    STRUCT_FIELD_POS
        .with(|m| {
            m.borrow()
                .iter()
                .find(|((t, f), _)| *t == tn && text(*f) == Some(wanted))
                .map(|(_, &slot)| slot)
        })
        .unwrap_or_else(|| {
            panic!("struct field write: no field named {wanted:?} on the object's type")
        })
}

/// The fabric's numeric tag for an enum constructor / variant NAME (a constant index into `consts`).
/// The VM matches by NAME (`ind.constructor == variant_name`), so the fabric assigns each distinct name a
/// canonical tag = the FIRST const index carrying that Text. Two const indices with the same Text collapse
/// to the same tag, so `NewInductive`'s written tag and `TestArm`'s compared tag agree ⟺ the names agree —
/// exactly the VM's semantics, with no dependence on `enum_types`.
fn canonical_text_tag(consts: &[crate::vm::Constant], idx: u32) -> i64 {
    let name = match &consts[idx as usize] {
        crate::vm::Constant::Text(s) => s.as_str(),
        other => panic!("enum tag: constant {idx} is not Text: {other:?}"),
    };
    consts
        .iter()
        .position(|c| matches!(c, crate::vm::Constant::Text(s) if s == name))
        .expect("enum tag: name not found in constants") as i64
}

/// Build a DECIMAL char-list `[len, ('-')?, d_{k-1}, ..., d_0]` for integer `v` on the heap; returns the
/// pointer `Src` (`heap[ptr] = len`, the RUNTIME digit count). Most-significant-first, '-' for negatives, no
/// leading zeros (0 → "0"). 19 positions cover the full i64 range. This is the int→text rendering `Concat`
/// (string interpolation) performs on a non-text operand — byte-exact with the VM's `n.to_string()`.
fn push_int_to_decimal(cells: &mut Vec<LoopCell>, v: Src) -> Src {
    let ksign = cells.len();
    cells.push(LoopCell::Select {
        cmp: CmpKind::Lt,
        ca: v,
        cb: Src::Const(0),
        t: Src::Const(1),
        e: Src::Const(0),
    });
    let sign = Src::Val(ksign);
    let kneg = cells.len();
    cells.push(LoopCell::Arith {
        op: AluOp::Sub,
        a: Src::Const(0),
        b: v,
    });
    let km = cells.len();
    cells.push(LoopCell::Select {
        cmp: CmpKind::Lt,
        ca: v,
        cb: Src::Const(0),
        t: Src::Val(kneg),
        e: v,
    });
    let m = Src::Val(km);
    let ka = cells.len();
    cells.push(LoopCell::HeapAlloc {
        size: Src::Const(21),
    }); // header + '-' + up to 19 digits
    let ptr = Src::Val(ka);
    let kdaddr = cells.len(); // '-' at heap[ptr+1] (overwritten by the first digit when there is no sign)
    cells.push(LoopCell::Arith {
        op: AluOp::Add,
        a: ptr,
        b: Src::Const(1),
    });
    cells.push(LoopCell::HeapWrite {
        addr: Src::Val(kdaddr),
        val: Src::Const(45),
    });
    let kwp = cells.len(); // write_pos = sign ? 2 : 1
    cells.push(LoopCell::Select {
        cmp: CmpKind::Ne,
        ca: sign,
        cb: Src::Const(0),
        t: Src::Const(2),
        e: Src::Const(1),
    });
    let mut write_pos = Src::Val(kwp);
    let mut p10: i64 = 1;
    for _ in 0..18 {
        p10 *= 10; // 10^18
    }
    for i in (0..19).rev() {
        let kq = cells.len();
        cells.push(LoopCell::Func {
            unit: FuncUnit::DivS,
            a: m,
            b: Src::Const(p10),
        });
        let kr = cells.len();
        cells.push(LoopCell::Func {
            unit: FuncUnit::ModS,
            a: Src::Val(kq),
            b: Src::Const(10),
        });
        let kchar = cells.len();
        cells.push(LoopCell::Arith {
            op: AluOp::Add,
            a: Src::Val(kr),
            b: Src::Const(48),
        }); // '0' + digit
            // output this position iff it is the ones place OR a digit ≥ 10^i is present (i.e. not a leading zero)
        let output = if i == 0 {
            Src::Const(1)
        } else {
            let klt = cells.len();
            cells.push(LoopCell::Select {
                cmp: CmpKind::Lt,
                ca: m,
                cb: Src::Const(p10),
                t: Src::Const(1),
                e: Src::Const(0),
            });
            let kge = cells.len();
            cells.push(LoopCell::Select {
                cmp: CmpKind::Eq,
                ca: Src::Val(klt),
                cb: Src::Const(0),
                t: Src::Const(1),
                e: Src::Const(0),
            });
            Src::Val(kge)
        };
        let kwaddr = cells.len(); // write char at heap[ptr + write_pos] (uncounted if not output)
        cells.push(LoopCell::Arith {
            op: AluOp::Add,
            a: ptr,
            b: write_pos,
        });
        cells.push(LoopCell::HeapWrite {
            addr: Src::Val(kwaddr),
            val: Src::Val(kchar),
        });
        let kwp1 = cells.len();
        cells.push(LoopCell::Arith {
            op: AluOp::Add,
            a: write_pos,
            b: Src::Const(1),
        });
        let kwpn = cells.len(); // write_pos = output ? write_pos+1 : write_pos
        cells.push(LoopCell::Select {
            cmp: CmpKind::Ne,
            ca: output,
            cb: Src::Const(0),
            t: Src::Val(kwp1),
            e: write_pos,
        });
        write_pos = Src::Val(kwpn);
        p10 /= 10;
    }
    let klen = cells.len(); // len = write_pos - 1
    cells.push(LoopCell::Arith {
        op: AluOp::Sub,
        a: write_pos,
        b: Src::Const(1),
    });
    cells.push(LoopCell::HeapWrite {
        addr: ptr,
        val: Src::Val(klen),
    });
    ptr
}

/// Every register a decision-fabric body WRITES (a superset used as the branch-merge carry set — a register
/// equal in both arms never spawns a `Select`). Comparison dsts are predicates, not state, so excluded.
fn carried_regs(body: &[(usize, crate::vm::Op)]) -> Vec<u16> {
    use crate::vm::Op;
    let mut carried: Vec<u16> = Vec::new();
    for (_, op) in body {
        let dst = match op {
            Op::LoadConst { dst, .. }
            | Op::Move { dst, .. }
            | Op::Add { dst, .. }
            | Op::Sub { dst, .. }
            | Op::Mul { dst, .. }
            | Op::Pow { dst, .. }
            | Op::BitAnd { dst, .. }
            | Op::BitOr { dst, .. }
            | Op::BitXor { dst, .. }
            | Op::Shl { dst, .. }
            | Op::Shr { dst, .. }
            | Op::Length { dst, .. }
            | Op::Index { dst, .. }
            | Op::IndexUnchecked { dst, .. }
            | Op::NewList { dst, .. }
            | Op::NewTuple { dst, .. }
            | Op::NewEmptyList { dst, .. }
            | Op::NewEmptyListWithCapacity { dst, .. }
            | Op::NewEmptyListFixed { dst, .. }
            | Op::NewEmptyListI32 { dst, .. }
            | Op::NewEmptySet { dst, .. }
            | Op::NewEmptyMap { dst, .. }
            | Op::NewEmptyMapWithCapacity { dst, .. }
            | Op::ListPop { dst, .. }
            | Op::NewStruct { dst, .. }
            | Op::GetField { dst, .. }
            | Op::NewInductive { dst, .. }
            | Op::BindArm { dst, .. }
            | Op::Not { dst, .. }
            | Op::Call { dst, .. }
            | Op::ChanNew { dst, .. }
            | Op::ChanRecv { dst, .. }
            | Op::ChanTrySend { dst, .. }
            | Op::Contains { dst, .. }
            | Op::DeepClone { dst, .. }
            | Op::SeqConcat { dst, .. }
            | Op::SliceOp { dst, .. }
            | Op::UnionOp { dst, .. }
            | Op::IntersectOp { dst, .. }
            | Op::Concat { dst, .. }
            | Op::AddAssign { dst, .. } => Some(*dst),
            _ => None,
        };
        if let Some(d) = dst {
            if !carried.contains(&d) {
                carried.push(d);
            }
        }
    }
    carried
}

/// Inline a callee's decision fabric at a `Call` site — the Futamura projection made concrete: the callee's
/// combinational datapath is emitted directly into the caller's `cells`, with its parameters SEEDED by the
/// caller's argument `Src`s (so the callee's `Src::In` never appears) and its intermediate `Src::Val` indices
/// naturally absolute in the shared `cells` vector (no remapping). Returns the `Src` driving the callee's
/// return value. A recursive/too-deep call graph is refused loudly (that needs a spatial call stack).
fn inline_callee(
    func_idx: usize,
    arg_srcs: &[Src],
    cells: &mut Vec<LoopCell>,
    consts: &[crate::vm::Constant],
) -> Src {
    use crate::vm::Op;
    let (entry_pc, param_count) = INLINE_FNS.with(|f| {
        *f.borrow()
            .get(func_idx)
            .unwrap_or_else(|| panic!("fabric inline: no function #{func_idx}"))
    });
    let depth = INLINE_DEPTH.with(|d| {
        let mut d = d.borrow_mut();
        *d += 1;
        *d
    });
    assert!(depth < 64, "fabric inline: call graph too deep or recursive — a spatial call STACK is needed, not inlining");

    let code = INLINE_CODE.with(|c| c.borrow().clone());
    let mut body: Vec<(usize, Op)> = Vec::new();
    let mut ret_src: Option<u16> = None;
    for (off, op) in code[entry_pc..].iter().enumerate() {
        if let Op::Return { src } = op {
            ret_src = Some(*src);
            break;
        }
        if matches!(op, Op::ReturnNothing) {
            break;
        }
        body.push((entry_pc + off, *op));
    }
    let ret_src = ret_src.expect("fabric inline: callee does not Return a value");
    let carried = carried_regs(&body);

    let mut reg: std::collections::HashMap<u16, Src> = std::collections::HashMap::new();
    for p in 0..param_count {
        reg.insert(p, arg_srcs[p as usize]); // seed each param with the caller's argument Src
    }
    let mut preds: std::collections::HashMap<u16, (CmpKind, Src, Src)> =
        std::collections::HashMap::new();
    dec_convert(&body, &mut reg, cells, &mut preds, &carried, consts);
    let result = *reg
        .get(&ret_src)
        .unwrap_or_else(|| panic!("fabric inline: callee return register R{ret_src} is unwritten"));

    INLINE_DEPTH.with(|d| *d.borrow_mut() -= 1);
    result
}

/// INLINE a SPAWNED child's straight-line side effects at the spawn point (the Futamura projection of the
/// scheduler). A producer child only ENQUEUES to channels the parent later dequeues, so running its sends at
/// the spawn — filling the buffer before the parent reads — yields the SAME observable as the cooperative
/// schedule (parent blocks on the empty channel, child runs, parent resumes). The child returns nothing, so
/// there is no result; its heap writes (channel enqueues) persist in the shared `cells`. A child that loops
/// is refused by `dec_convert` (loops → CGRA), the honest boundary.
fn inline_spawn(
    func_idx: usize,
    arg_srcs: &[Src],
    cells: &mut Vec<LoopCell>,
    consts: &[crate::vm::Constant],
) {
    use crate::vm::Op;
    let (entry_pc, param_count) = INLINE_FNS.with(|f| {
        *f.borrow()
            .get(func_idx)
            .unwrap_or_else(|| panic!("fabric spawn: no function #{func_idx}"))
    });
    let depth = INLINE_DEPTH.with(|d| {
        let mut d = d.borrow_mut();
        *d += 1;
        *d
    });
    assert!(depth < 64, "fabric spawn: call graph too deep or recursive");

    let code = INLINE_CODE.with(|c| c.borrow().clone());
    let mut body: Vec<(usize, Op)> = Vec::new();
    for (off, op) in code[entry_pc..].iter().enumerate() {
        if matches!(op, Op::Return { .. } | Op::ReturnNothing) {
            break;
        }
        body.push((entry_pc + off, *op));
    }
    let carried = carried_regs(&body);
    let mut reg: std::collections::HashMap<u16, Src> = std::collections::HashMap::new();
    for p in 0..param_count {
        reg.insert(p, arg_srcs[p as usize]); // seed each param with the spawner's argument Src
    }
    let mut preds: std::collections::HashMap<u16, (CmpKind, Src, Src)> =
        std::collections::HashMap::new();
    dec_convert(&body, &mut reg, cells, &mut preds, &carried, consts);
    INLINE_DEPTH.with(|d| *d.borrow_mut() -= 1);
}

/// INLINE a CLOSURE call (`CallValue`): the same body-inlining as `inline_callee`, but the callee's frame is
/// `[params, capture VALUES, capture PRESENT FLAGS]` — args seed the params, and the closure's captured
/// `(value, present)` pairs seed the two capture bands (`param_count + k` and `param_count + cap_count + k`),
/// exactly as the VM binds them. The body index is STATIC (from the `MakeClosure`), so dispatch is resolved.
fn inline_closure(
    func_idx: usize,
    arg_srcs: &[Src],
    captures: &[(Src, bool)],
    cells: &mut Vec<LoopCell>,
    consts: &[crate::vm::Constant],
) -> Src {
    use crate::vm::Op;
    let (entry_pc, param_count) = INLINE_FNS.with(|f| {
        *f.borrow()
            .get(func_idx)
            .unwrap_or_else(|| panic!("fabric closure: no function #{func_idx}"))
    });
    let depth = INLINE_DEPTH.with(|d| {
        let mut d = d.borrow_mut();
        *d += 1;
        *d
    });
    assert!(
        depth < 64,
        "fabric closure: call graph too deep or recursive"
    );

    let code = INLINE_CODE.with(|c| c.borrow().clone());
    let mut body: Vec<(usize, Op)> = Vec::new();
    let mut ret_src: Option<u16> = None;
    for (off, op) in code[entry_pc..].iter().enumerate() {
        if let Op::Return { src } = op {
            ret_src = Some(*src);
            break;
        }
        if matches!(op, Op::ReturnNothing) {
            break;
        }
        body.push((entry_pc + off, *op));
    }
    let ret_src = ret_src.expect("fabric closure: body does not Return a value");
    let carried = carried_regs(&body);

    let mut reg: std::collections::HashMap<u16, Src> = std::collections::HashMap::new();
    for p in 0..param_count {
        reg.insert(p, arg_srcs[p as usize]); // params 0..param_count = the call arguments
    }
    let cap_count = captures.len() as u16;
    for (k, (val, present)) in captures.iter().enumerate() {
        let k = k as u16;
        reg.insert(param_count + k, *val); // capture value band
        reg.insert(param_count + cap_count + k, Src::Const(i64::from(*present)));
        // present-flag band
    }
    let mut preds: std::collections::HashMap<u16, (CmpKind, Src, Src)> =
        std::collections::HashMap::new();
    dec_convert(&body, &mut reg, cells, &mut preds, &carried, consts);
    let result = *reg
        .get(&ret_src)
        .unwrap_or_else(|| panic!("fabric closure: return register R{ret_src} is unwritten"));
    INLINE_DEPTH.with(|d| *d.borrow_mut() -= 1);
    result
}

/// Populate the program-context thread-locals every fabric build reads: the struct field-count and
/// (type,field)→slot maps (robust to duplicate string interning), the reset of the branch-invariant
/// register-type map, the whole program's code + per-function entry for `Call` inlining, and the
/// collection-length + output-tape resets. Shared by `decision_fabric_from_source` and the whole-program
/// builder so both see identical struct/enum/call context.
fn setup_program_thread_locals(program: &crate::vm::CompiledProgram) {
    use std::collections::HashMap;
    // Every const index that names a struct type → its field count (a name can be interned at >1 index).
    STRUCT_FIELDS.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        for (i, c) in program.constants.iter().enumerate() {
            if let crate::vm::Constant::Text(name) = c {
                if let Some(sd) = program.struct_types.iter().find(|s| &s.name == name) {
                    m.insert(i as u32, sd.fields.len());
                }
            }
        }
    });
    // (type-name const, field-name const) → declaration-order slot; cross every matching const-index pair.
    STRUCT_FIELD_POS.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        for sd in &program.struct_types {
            let name_pos: HashMap<&str, usize> = sd
                .fields
                .iter()
                .enumerate()
                .map(|(pos, (fname, _))| (fname.as_str(), pos))
                .collect();
            for (t, ct) in program.constants.iter().enumerate() {
                if !matches!(ct, crate::vm::Constant::Text(n) if *n == sd.name) {
                    continue;
                }
                for (f, cf) in program.constants.iter().enumerate() {
                    if let crate::vm::Constant::Text(fname) = cf {
                        if let Some(&pos) = name_pos.get(fname.as_str()) {
                            m.insert((t as u32, f as u32), pos);
                        }
                    }
                }
            }
        }
        if std::env::var("FABRIC_DUMP").is_ok() {
            let st: Vec<(String, Vec<String>)> = program
                .struct_types
                .iter()
                .map(|s| {
                    (
                        s.name.clone(),
                        s.fields.iter().map(|(f, _)| f.clone()).collect(),
                    )
                })
                .collect();
            let tc: Vec<(usize, String)> = program
                .constants
                .iter()
                .enumerate()
                .filter_map(|(i, c)| match c {
                    crate::vm::Constant::Text(t) => Some((i, t.clone())),
                    _ => None,
                })
                .collect();
            eprintln!("STRUCTDUMP struct_types={st:?}");
            eprintln!("STRUCTDUMP text_consts={tc:?}");
            eprintln!("STRUCTDUMP field_pos={:?}", m.iter().collect::<Vec<_>>());
        }
    });
    REG_TYPE.with(|m| m.borrow_mut().clear());
    INLINE_CODE.with(|c| *c.borrow_mut() = program.code.clone());
    INLINE_FNS.with(|m| {
        *m.borrow_mut() = program
            .functions
            .iter()
            .map(|fun| (fun.entry_pc, fun.param_count))
            .collect();
    });
    CLOSURE_CAPTURES.with(|m| {
        *m.borrow_mut() = program
            .functions
            .iter()
            .map(|fun| fun.captures.iter().map(|(_, gidx)| *gidx).collect())
            .collect();
    });
    CLOSURES.with(|c| c.borrow_mut().clear());
    INLINE_DEPTH.with(|d| *d.borrow_mut() = 0);
    COLL_LEN.with(|m| m.borrow_mut().clear());
    SHOW_OUTPUTS.with(|o| o.borrow_mut().clear());
    TEXT_REGS.with(|t| t.borrow_mut().clear());
    NAME_REGS.with(|t| t.borrow_mut().clear());
    FLOAT_REGS.with(|t| t.borrow_mut().clear());
    W32_REGS.with(|t| t.borrow_mut().clear());
    BIGINT_BIG.with(|m| m.borrow_mut().clear());
    GLOBALS.with(|g| g.borrow_mut().clear());
    NET_INBOX.with(|n| *n.borrow_mut() = None);
    SELECT_ARMS.with(|a| a.borrow_mut().clear());
    HALTED.with(|h| h.set(false));
    // Policy conditions are re-populated per program by `whole_program_fabric_from_program_with_policies`
    // (empty for hand-authored bytecode, which carries no `PolicyRegistry`); the grant guard starts open.
    POLICY_CONDITIONS.with(|m| m.borrow_mut().clear());
    POLICY_GUARD.with(|g| *g.borrow_mut() = None);
    REG_TIMEKIND.with(|m| m.borrow_mut().clear());
}

/// Logical (zero-filling) right shift by a COMPILE-TIME-CONSTANT amount `s` (0..=63), realized with the
/// arithmetic `Shr` tile plus an `And` that clears the sign-extended top `s` bits (the mask is baked at
/// build time). `s == 0` is the identity. This is the `u64 >>` the magic-reciprocal division (`MagicDivU`)
/// needs — the fabric's `AluOp::Shr` is arithmetic, so no separate logical-shift ALU op is required.
fn logical_shr_const(cells: &mut Vec<LoopCell>, src: Src, s: i64) -> Src {
    if s == 0 {
        return src;
    }
    let kshr = cells.len();
    cells.push(LoopCell::Arith {
        op: AluOp::Shr,
        a: src,
        b: Src::Const(s),
    });
    let mask = (((1u128 << (64 - s)) - 1) as u64) as i64; // keep the low (64 - s) bits, zero the rest
    let kand = cells.len();
    cells.push(LoopCell::Arith {
        op: AluOp::And,
        a: Src::Val(kshr),
        b: Src::Const(mask),
    });
    Src::Val(kand)
}

/// Push one `Arith` tile and return its SSA `Src` — a terse builder for the calendar tile chains below.
fn t_arith(cells: &mut Vec<LoopCell>, op: AluOp, a: Src, b: Src) -> Src {
    let k = cells.len();
    cells.push(LoopCell::Arith { op, a, b });
    Src::Val(k)
}
/// Push one `Func` tile and return its SSA `Src`.
fn t_func(cells: &mut Vec<LoopCell>, unit: FuncUnit, a: Src, b: Src) -> Src {
    let k = cells.len();
    cells.push(LoopCell::Func { unit, a, b });
    Src::Val(k)
}
/// Push one `Select` tile (`cmp(ca,cb) ? t : e`) and return its SSA `Src`.
fn t_select(cells: &mut Vec<LoopCell>, cmp: CmpKind, ca: Src, cb: Src, t: Src, e: Src) -> Src {
    let k = cells.len();
    cells.push(LoopCell::Select { cmp, ca, cb, t, e });
    Src::Val(k)
}

/// The GREGORIAN `civil_from_days(z)` (Howard Hinnant) as a combinational tile chain — `days` since the
/// epoch → `(year, month, day)`. Every `/` is TRUNCATING signed division (`DivS`), which the algorithm's
/// `z >= 0 ? z : z - 146096` era-adjust (a `Select`) makes exact for negative day numbers too. Bit-identical
/// to [`logicaffeine_base::temporal::civil_from_days`]. See that function for the derivation.
fn civil_from_days_tiles(cells: &mut Vec<LoopCell>, days: Src) -> (Src, Src, Src) {
    let c = Src::Const;
    let z = t_arith(cells, AluOp::Add, days, c(719468));
    let z_m = t_arith(cells, AluOp::Sub, z, c(146096));
    let era_num = t_select(cells, CmpKind::Ge, z, c(0), z, z_m); // z >= 0 ? z : z - 146096
    let era = t_func(cells, FuncUnit::DivS, era_num, c(146097));
    let era146097 = t_arith(cells, AluOp::Mul, era, c(146097));
    let doe = t_arith(cells, AluOp::Sub, z, era146097); // [0, 146096]
    let doe1460 = t_func(cells, FuncUnit::DivS, doe, c(1460));
    let doe36524 = t_func(cells, FuncUnit::DivS, doe, c(36524));
    let doe146096 = t_func(cells, FuncUnit::DivS, doe, c(146096));
    let yn1 = t_arith(cells, AluOp::Sub, doe, doe1460);
    let yn2 = t_arith(cells, AluOp::Add, yn1, doe36524);
    let yn3 = t_arith(cells, AluOp::Sub, yn2, doe146096);
    let yoe = t_func(cells, FuncUnit::DivS, yn3, c(365)); // [0, 399]
    let era400 = t_arith(cells, AluOp::Mul, era, c(400));
    let y = t_arith(cells, AluOp::Add, yoe, era400);
    let yoe4 = t_func(cells, FuncUnit::DivS, yoe, c(4));
    let yoe100 = t_func(cells, FuncUnit::DivS, yoe, c(100));
    let t365 = t_arith(cells, AluOp::Mul, yoe, c(365));
    let t2 = t_arith(cells, AluOp::Add, t365, yoe4);
    let t3 = t_arith(cells, AluOp::Sub, t2, yoe100);
    let doy = t_arith(cells, AluOp::Sub, doe, t3); // [0, 365]
    let mpn1 = t_arith(cells, AluOp::Mul, doy, c(5));
    let mpn2 = t_arith(cells, AluOp::Add, mpn1, c(2));
    let mp = t_func(cells, FuncUnit::DivS, mpn2, c(153)); // [0, 11]
    let d153 = t_arith(cells, AluOp::Mul, mp, c(153));
    let d1532 = t_arith(cells, AluOp::Add, d153, c(2));
    let d5 = t_func(cells, FuncUnit::DivS, d1532, c(5));
    let dd = t_arith(cells, AluOp::Sub, doy, d5);
    let day = t_arith(cells, AluOp::Add, dd, c(1)); // [1, 31]
    let mp3 = t_arith(cells, AluOp::Add, mp, c(3));
    let mp9 = t_arith(cells, AluOp::Sub, mp, c(9));
    let month = t_select(cells, CmpKind::Lt, mp, c(10), mp3, mp9); // mp < 10 ? mp + 3 : mp - 9
    let y1 = t_arith(cells, AluOp::Add, y, c(1));
    let year = t_select(cells, CmpKind::Le, month, c(2), y1, y); // month <= 2 ? y + 1 : y
    (year, month, day)
}

/// The inverse `days_from_civil(year, month, day)` (Howard Hinnant) as a combinational tile chain —
/// bit-identical to [`logicaffeine_base::temporal::days_from_civil`]. Used by the ISO-week anchor.
fn days_from_civil_tiles(cells: &mut Vec<LoopCell>, year: Src, month: Src, day: Src) -> Src {
    let c = Src::Const;
    let ym1 = t_arith(cells, AluOp::Sub, year, c(1));
    let y = t_select(cells, CmpKind::Le, month, c(2), ym1, year); // month <= 2 ? year - 1 : year
    let ym399 = t_arith(cells, AluOp::Sub, y, c(399));
    let era_num = t_select(cells, CmpKind::Ge, y, c(0), y, ym399); // y >= 0 ? y : y - 399
    let era = t_func(cells, FuncUnit::DivS, era_num, c(400));
    let era400 = t_arith(cells, AluOp::Mul, era, c(400));
    let yoe = t_arith(cells, AluOp::Sub, y, era400); // [0, 399]
    let mm3 = t_arith(cells, AluOp::Sub, month, c(3));
    let mp9 = t_arith(cells, AluOp::Add, month, c(9));
    let mp = t_select(cells, CmpKind::Gt, month, c(2), mm3, mp9); // m > 2 ? m - 3 : m + 9
    let d153 = t_arith(cells, AluOp::Mul, mp, c(153));
    let d1532 = t_arith(cells, AluOp::Add, d153, c(2));
    let d5 = t_func(cells, FuncUnit::DivS, d1532, c(5));
    let dd = t_arith(cells, AluOp::Add, d5, day);
    let doy = t_arith(cells, AluOp::Sub, dd, c(1)); // [0, 365]
    let yoe365 = t_arith(cells, AluOp::Mul, yoe, c(365));
    let yoe4 = t_func(cells, FuncUnit::DivS, yoe, c(4));
    let yoe100 = t_func(cells, FuncUnit::DivS, yoe, c(100));
    let doe1 = t_arith(cells, AluOp::Add, yoe365, yoe4);
    let doe2 = t_arith(cells, AluOp::Sub, doe1, yoe100);
    let doe = t_arith(cells, AluOp::Add, doe2, doy); // [0, 146096]
    let e146097 = t_arith(cells, AluOp::Mul, era, c(146097));
    let d1 = t_arith(cells, AluOp::Add, e146097, doe);
    t_arith(cells, AluOp::Sub, d1, c(719468))
}

/// The ISO-8601 `iso_weekday_from_days(z)` as tiles: `(z + 3) mod 7 + 1` in `1..=7` (Mon..Sun). `ModS` (the
/// dividend-signed remainder) equals `rem_euclid` for `z >= -3` — every real day number since 1970 — matching
/// the existing `weekday`/`hour` lowering's `ModS`-for-`rem_euclid` convention.
fn iso_weekday_from_days_tiles(cells: &mut Vec<LoopCell>, z: Src) -> Src {
    let z3 = t_arith(cells, AluOp::Add, z, Src::Const(3));
    let m = t_func(cells, FuncUnit::ModS, z3, Src::Const(7));
    t_arith(cells, AluOp::Add, m, Src::Const(1))
}

/// Every per-register fact the lowering tracks — Text, float, float-collection, static length, wide
/// bignum, struct type, time kind, closure — dropped for `r`.
fn forget_reg(r: u16) {
    TEXT_REGS.with(|t| t.borrow_mut().remove(&r));
    NAME_REGS.with(|t| t.borrow_mut().remove(&r));
    FLOAT_REGS.with(|t| t.borrow_mut().remove(&r));
    FLOAT_COLLS.with(|t| t.borrow_mut().remove(&r));
    COLL_LEN.with(|t| t.borrow_mut().remove(&r));
    BIGINT_BIG.with(|t| t.borrow_mut().remove(&r));
    REG_TYPE.with(|t| t.borrow_mut().remove(&r));
    REG_TIMEKIND.with(|t| t.borrow_mut().remove(&r));
    CLOSURES.with(|t| t.borrow_mut().remove(&r));
}

/// A register an op WRITES without READING holds nothing it held before: its facts are dropped before
/// the op's arm runs, so the arm derives it from the op alone — a temporary the VM reuses for an Int after
/// a Text no longer reads as Text. A register both read and written (`x = x + 1`) keeps its facts for the
/// arm to read. `captures` is a closure's capture count (`Op::for_each_read_reg`).
fn forget_overwritten(op: &crate::vm::Op, captures: impl Fn(u16) -> usize) {
    let mut read = Vec::new();
    op.for_each_read_reg(captures, |r| read.push(r));
    op.for_each_written_reg(|r| {
        if !read.contains(&r) {
            forget_reg(r);
        }
    });
}

/// A Text literal on the heap: the char-list `[len, byte0, byte1, …]` (UTF-8 bytes, so `length of` is the
/// byte count, matching the VM), with `extra` words of growth capacity past it. Returns its base pointer.
fn heap_text_literal(cells: &mut Vec<LoopCell>, bytes: &[u8], extra: i64) -> Src {
    let n = bytes.len();
    let ka = cells.len();
    cells.push(LoopCell::HeapAlloc {
        size: Src::Const(n as i64 + 1 + extra),
    });
    let ptr = Src::Val(ka);
    cells.push(LoopCell::HeapWrite {
        addr: ptr,
        val: Src::Const(n as i64),
    });
    for (i, b) in bytes.iter().enumerate() {
        let kadd = cells.len();
        cells.push(LoopCell::Arith {
            op: AluOp::Add,
            a: ptr,
            b: Src::Const(i as i64 + 1),
        });
        cells.push(LoopCell::HeapWrite {
            addr: Src::Val(kadd),
            val: Src::Const(*b as i64),
        });
    }
    ptr
}

/// `args()` on a circuit: argv, program name first, as on every tier (`semantics::argv_with_program_name`). A
/// circuit has no host argv, so its argument list is the program name alone — `[1, name]`, the name a heap
/// Text — with `extra` words of growth capacity. Returns the list's base pointer.
fn heap_argv(cells: &mut Vec<LoopCell>, extra: i64) -> Src {
    let name = heap_text_literal(cells, crate::semantics::PROGRAM_NAME.as_bytes(), 0);
    let ka = cells.len();
    cells.push(LoopCell::HeapAlloc {
        size: Src::Const(2 + extra),
    });
    let ptr = Src::Val(ka);
    cells.push(LoopCell::HeapWrite {
        addr: ptr,
        val: Src::Const(1),
    });
    let kadd = cells.len();
    cells.push(LoopCell::Arith {
        op: AluOp::Add,
        a: ptr,
        b: Src::Const(1),
    });
    cells.push(LoopCell::HeapWrite {
        addr: Src::Val(kadd),
        val: name,
    });
    ptr
}

/// Straight-line (jump-free) execution of an op run into the SSA register map + cell list, shared
/// by the counted-loop and `while`-loop body extractors. Comparisons are recorded as pending
/// predicates (folded into a `Select` at a branch merge).
fn exec_ops(
    ops: &[crate::vm::Op],
    reg: &mut std::collections::HashMap<u16, Src>,
    cells: &mut Vec<LoopCell>,
    preds: &mut std::collections::HashMap<u16, (CmpKind, Src, Src)>,
    consts: &[crate::vm::Constant],
) {
    use crate::vm::{Constant, Op};
    let getb = |reg: &std::collections::HashMap<u16, Src>, r: u16| -> Src {
        *reg.get(&r)
            .unwrap_or_else(|| panic!("loop body reads unwritten register R{r}"))
    };
    for op in ops {
        forget_overwritten(op, |_| usize::MAX);
        let cmp = match op {
            Op::Lt { .. } => Some(CmpKind::Lt),
            Op::Gt { .. } => Some(CmpKind::Gt),
            Op::LtEq { .. } => Some(CmpKind::Le),
            Op::GtEq { .. } => Some(CmpKind::Ge),
            Op::Eq { .. } => Some(CmpKind::Eq),
            Op::NotEq { .. } => Some(CmpKind::Ne),
            _ => None,
        };
        if let Some(kind) = cmp {
            if let Op::Lt { dst, lhs, rhs }
            | Op::Gt { dst, lhs, rhs }
            | Op::LtEq { dst, lhs, rhs }
            | Op::GtEq { dst, lhs, rhs }
            | Op::Eq { dst, lhs, rhs }
            | Op::NotEq { dst, lhs, rhs } = op
            {
                preds.insert(*dst, (kind, getb(reg, *lhs), getb(reg, *rhs)));
            }
            continue;
        }
        match op {
            Op::LoadConst { dst, idx: ci } => match &consts[*ci as usize] {
                Constant::Int(n) => {
                    reg.insert(*dst, Src::Const(*n));
                }
                Constant::Bool(b) => {
                    reg.insert(*dst, Src::Const(i64::from(*b))); // Bool rides a scalar slot as 0/1
                }
                Constant::Text(s) => {
                    // A Text literal → its heap char-list, marked TEXT so `Show` renders it as a string, and
                    // remembered as a NAME for a place mutation that keys a struct field by it.
                    let ptr = heap_text_literal(cells, s.as_bytes(), 0);
                    reg.insert(*dst, ptr);
                    TEXT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                    NAME_REGS.with(|t| {
                        t.borrow_mut().insert(*dst, *ci);
                    });
                    COLL_LEN.with(|m| {
                        m.borrow_mut().insert(*dst, s.len() as i64);
                    });
                }
                // A `Duration` rides a scalar slot as its nanosecond count — used only as a `select`
                // timeout's `ticks` (which the fabric's priority resolver ignores: a ready recv always wins).
                Constant::Duration(n) => {
                    reg.insert(*dst, Src::Const(*n));
                }
                // A FLOAT rides a scalar slot as its `f64` BIT PATTERN; the reg is marked so arithmetic routes
                // through the float functional units and `Show` refuses (float→string is a follow-on).
                Constant::Float(f) => {
                    reg.insert(*dst, Src::Const(f.to_bits() as i64));
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                }
                // `Nothing` rides a scalar slot as 0, the word a cleared pointer field holds — a place mutation
                // clears the parent's field to it before mutating the leaf and writing the leaf back.
                Constant::Nothing => {
                    reg.insert(*dst, Src::Const(0));
                }
                c => panic!("loop body: unsupported constant {c:?}"),
            },
            // `now` / `today` — read the clock. A combinational fabric has NO wall clock, so a deterministic
            // differential requires an INJECTED fixed clock (`temporal::set_fixed_clock`, which the VM reads
            // too); the fabric reads the same instant and bakes it as a Const (a value-load, like LoadConst —
            // hence a Wire tile). `LoadNow` yields a Moment (nanos), `LoadToday` a Date (whole days); the kind
            // is tracked so a date-component extractor decodes it correctly. Absent an injected clock the
            // fabric REFUSES rather than fabricating a time — an honest boundary (tests inject the clock).
            Op::LoadNow { dst } => {
                let (_, nanos) = crate::semantics::temporal::fixed_clock().expect(
                    "fabric LoadNow: the combinational fabric cannot read the wall clock — inject a fixed \
                     clock (temporal::set_fixed_clock) so the VM and the fabric read the same instant",
                );
                reg.insert(*dst, Src::Const(nanos));
                REG_TIMEKIND.with(|m| m.borrow_mut().insert(*dst, TimeKind::Moment));
            }
            Op::LoadToday { dst } => {
                let (days, _) = crate::semantics::temporal::fixed_clock().expect(
                    "fabric LoadToday: the combinational fabric cannot read the wall clock — inject a fixed \
                     clock (temporal::set_fixed_clock) so the VM and the fabric read the same instant",
                );
                reg.insert(*dst, Src::Const(i64::from(days)));
                REG_TIMEKIND.with(|m| m.borrow_mut().insert(*dst, TimeKind::Date));
            }
            // `new counter` / `new growing set` / … — create a fresh CRDT value. The replica id the VM draws
            // (`next_replica_id`) is captured nondeterminism that does NOT affect the SINGLE-replica
            // observable (it only breaks merge ties), so the fabric models the deterministic INITIAL state.
            // A fresh counter / PN-counter is 0 (renders as a plain Int, matching the VM's `Show`); a set /
            // seq / register starts empty but renders as a collection (`{}` / `[]`), which needs collection
            // rendering on the output tape — a follow-on, so those kinds are refused loudly.
            Op::NewCrdt { dst, kind } => match kind {
                // a counter / PN-counter (kind 4/5) is a single Int, initially 0.
                4 | 5 => {
                    reg.insert(*dst, Src::Const(0));
                }
                // a set / sequence / register CRDT (kind 0/1/2/3) is a live COLLECTION: its single-replica
                // state is an empty list `[0]` on the heap (grown in place by `CrdtAppend`/`SetAdd`), exactly
                // like `NewEmptyList` — the fabric models the deterministic converged value.
                _ => {
                    let ka = cells.len();
                    cells.push(LoopCell::HeapAlloc {
                        size: Src::Const(1),
                    });
                    let ptr = Src::Val(ka);
                    cells.push(LoopCell::HeapWrite {
                        addr: ptr,
                        val: Src::Const(0),
                    });
                    reg.insert(*dst, ptr);
                }
            },
            Op::Move { dst, src } => {
                let s = getb(reg, *src);
                reg.insert(*dst, s);
                NAME_REGS.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(n) = m.get(src).copied() {
                        m.insert(*dst, n);
                    }
                });
                // a struct type / a collection's static length ride along the move
                REG_TYPE.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(t) = m.get(src).copied() {
                        m.insert(*dst, t);
                    }
                });
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(l) = m.get(src).copied() {
                        m.insert(*dst, l);
                    }
                });
                TEXT_REGS.with(|t| {
                    let mut t = t.borrow_mut();
                    if t.contains(src) {
                        t.insert(*dst);
                    }
                });
                FLOAT_REGS.with(|t| {
                    let mut t = t.borrow_mut();
                    if t.contains(src) {
                        t.insert(*dst);
                    }
                });
                // WORD32-ness rides along a move, so a `+` on the copy still wraps mod 2³² (SHA-256 keeps
                // word32 values in temporaries between the mix and the round add).
                W32_REGS.with(|t| {
                    let mut t = t.borrow_mut();
                    if t.contains(src) {
                        t.insert(*dst);
                    } else {
                        t.remove(dst);
                    }
                });
                REG_TIMEKIND.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(k) = m.get(src).copied() {
                        m.insert(*dst, k);
                    }
                });
            }
            // `Swap a and b` — the zero-garbage reversible register exchange: swap the two Srcs (pure
            // rewiring, no cell) and any collection/text/struct tracking, so the names fully exchange identity.
            Op::RevSwap { a, b } => {
                let sa = getb(reg, *a);
                let sb = getb(reg, *b);
                reg.insert(*a, sb);
                reg.insert(*b, sa);
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    let (va, vb) = (m.remove(a), m.remove(b));
                    if let Some(v) = vb {
                        m.insert(*a, v);
                    }
                    if let Some(v) = va {
                        m.insert(*b, v);
                    }
                });
                REG_TYPE.with(|m| {
                    let mut m = m.borrow_mut();
                    let (va, vb) = (m.remove(a), m.remove(b));
                    if let Some(v) = vb {
                        m.insert(*a, v);
                    }
                    if let Some(v) = va {
                        m.insert(*b, v);
                    }
                });
                TEXT_REGS.with(|t| {
                    let mut t = t.borrow_mut();
                    let (ha, hb) = (t.remove(a), t.remove(b));
                    if hb {
                        t.insert(*a);
                    }
                    if ha {
                        t.insert(*b);
                    }
                });
            }
            // Reversible ALU (the `rev_alu_core` ISA — no source emits them, hand-bytecode witnessed): each
            // updates `place` IN PLACE with a combinational cell, exactly like its irreversible twin. RevXor
            // `p ^= d`, RevAdd `p += d`, RevSub `p -= d`, RevNeg `p = -p` (0 - p). Zero-garbage on silicon;
            // on the combinational fabric they are the same XOR/ADD/SUB tiles writing a fresh SSA value.
            Op::RevXor { place, delta } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Xor,
                    a: getb(reg, *place),
                    b: getb(reg, *delta),
                });
                reg.insert(*place, Src::Val(k));
            }
            Op::RevAdd { place, delta } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: getb(reg, *place),
                    b: getb(reg, *delta),
                });
                reg.insert(*place, Src::Val(k));
            }
            Op::RevSub { place, delta } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Sub,
                    a: getb(reg, *place),
                    b: getb(reg, *delta),
                });
                reg.insert(*place, Src::Val(k));
            }
            Op::RevNeg { place } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Sub,
                    a: Src::Const(0),
                    b: getb(reg, *place),
                });
                reg.insert(*place, Src::Val(k));
            }
            Op::Add { dst, lhs, rhs } => {
                let float =
                    FLOAT_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let w32 = W32_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let (a, b) = (getb(reg, *lhs), getb(reg, *rhs));
                let k = cells.len();
                if float {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::FAddF,
                        a,
                        b,
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    reg.insert(*dst, Src::Val(k));
                } else if w32 {
                    // WORD32 wrapping add: the integer sum masked back to 32 bits (mod 2³²) — SHA-256 rounds.
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a,
                        b,
                    });
                    let km = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(k),
                        b: Src::Const(0xFFFF_FFFF),
                    });
                    reg.insert(*dst, Src::Val(km));
                    W32_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                } else {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a,
                        b,
                    });
                    reg.insert(*dst, Src::Val(k));
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                }
            }
            // Compound accumulate `Set x to x + y` → `x += y`. A FLOAT accumulator routes to the `FAddF`
            // functional unit (mirroring `Op::Add`) so a float loop's `Set s to s + …` is a real float add,
            // not an integer add of two `f64` bit patterns. (The VM's Text fast path is a string append the
            // fabric heap plane doesn't model — scalar accumulate only.)
            Op::AddAssign { dst, src } => {
                let float =
                    FLOAT_REGS.with(|t| t.borrow().contains(dst) || t.borrow().contains(src));
                let (a, b) = (getb(reg, *dst), getb(reg, *src));
                let k = cells.len();
                if float {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::FAddF,
                        a,
                        b,
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                } else {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a,
                        b,
                    });
                }
                reg.insert(*dst, Src::Val(k));
            }
            // Logical not: `!is_truthy(src)` = `(src == 0) ? 1 : 0` (Bool as 0/1), byte-exact with the VM's
            // `not_value`. A Select mux realises it as a combinational value (usable in later arithmetic or,
            // via the value-condition path, as a branch predicate).
            Op::Not { dst, src } => {
                let k = cells.len();
                cells.push(LoopCell::Select {
                    cmp: CmpKind::Eq,
                    ca: getb(reg, *src),
                    cb: Src::Const(0),
                    t: Src::Const(1),
                    e: Src::Const(0),
                });
                reg.insert(*dst, Src::Val(k));
            }
            Op::Sub { dst, lhs, rhs } => {
                let float =
                    FLOAT_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let w32 = W32_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let (a, b) = (getb(reg, *lhs), getb(reg, *rhs));
                let k = cells.len();
                if float {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::FSubF,
                        a,
                        b,
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    reg.insert(*dst, Src::Val(k));
                } else if w32 {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Sub,
                        a,
                        b,
                    });
                    let km = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(k),
                        b: Src::Const(0xFFFF_FFFF),
                    });
                    reg.insert(*dst, Src::Val(km));
                    W32_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                } else {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Sub,
                        a,
                        b,
                    });
                    reg.insert(*dst, Src::Val(k));
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                }
            }
            Op::Mul { dst, lhs, rhs } => {
                let float =
                    FLOAT_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let w32 = W32_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let (a, b) = (getb(reg, *lhs), getb(reg, *rhs));
                let k = cells.len();
                if float {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::FMulF,
                        a,
                        b,
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    reg.insert(*dst, Src::Val(k));
                } else if w32 {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Mul,
                        a,
                        b,
                    });
                    let km = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(k),
                        b: Src::Const(0xFFFF_FFFF),
                    });
                    reg.insert(*dst, Src::Val(km));
                    W32_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                } else {
                    cells.push(LoopCell::Arith {
                        op: AluOp::Mul,
                        a,
                        b,
                    });
                    reg.insert(*dst, Src::Val(k));
                    W32_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                }
            }
            // Bitwise / shift — combinational ALU tiles, bit-identical to the VM's `Int` ops.
            Op::BitAnd { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::And,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            Op::BitOr { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Or,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            Op::BitXor { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Xor,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            Op::Shl { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Shl,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            Op::Shr { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Shr,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            // Division / remainder — a SEQUENTIAL functional-unit tile (the `shared_divider`), not a
            // combinational ALU op. `Op::Div` is the truncated signed quotient (`/`); `Op::Mod` the signed
            // remainder; `Op::FloorDiv` the floored quotient. All trap `b == 0 → 0` (see `FuncUnit`).
            // `Op::ExactDiv` is DELIBERATELY excluded: the VM yields a Rational (`7/2 → 7/2`) that an
            // integer fabric cannot represent, so it falls to the honest `unsupported` arm below.
            Op::Div { dst, lhs, rhs } => {
                let float =
                    FLOAT_REGS.with(|t| t.borrow().contains(lhs) || t.borrow().contains(rhs));
                let (a, b) = (getb(reg, *lhs), getb(reg, *rhs));
                let k = cells.len();
                if float {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::FDivF,
                        a,
                        b,
                    });
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                } else {
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::DivS,
                        a,
                        b,
                    });
                    // An integer quotient CLEARS any stale float tag on `dst` (registers are reused across
                    // blocks, and `FLOAT_REGS` is global — a reg that held an f64 in a prior block is now Int).
                    FLOAT_REGS.with(|t| {
                        t.borrow_mut().remove(dst);
                    });
                }
                reg.insert(*dst, Src::Val(k));
            }
            Op::Mod { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::ModS,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().remove(dst);
                }); // integer remainder — clear stale float tag
            }
            Op::FloorDiv { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::FloorDivS,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().remove(dst);
                }); // integer floor-div — clear stale float tag
            }
            // `a to the power of b` — the sequential power functional unit (RTL: a `b`-step multiply FSM).
            Op::Pow { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::Pow,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            // NUMERIC COPROCESSOR builtins in a combinational loop body — `sqrt`/`floor` on the float
            // functional units (`FSqrtF`/`FFloorF`), `abs` (float = sign-bit clear, int = negate+select),
            // `min`/`max` (integer select). Mirrors the whole-program engine's lowering with `LoopCell` tiles,
            // so `Set s to sqrt(s + …)` / `abs(…)` / `min(…, …)` in a loop become spatial datapath tiles.
            Op::CallBuiltin {
                dst,
                builtin,
                args_start,
                arg_count: _,
            } => {
                use crate::semantics::builtins::BuiltinId;
                let a0 = getb(reg, *args_start);
                let is_f = FLOAT_REGS.with(|t| t.borrow().contains(args_start));
                match builtin {
                    BuiltinId::Sqrt if is_f => {
                        let k = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FSqrtF, a: a0, b: Src::Const(0) });
                        reg.insert(*dst, Src::Val(k));
                        FLOAT_REGS.with(|t| { t.borrow_mut().insert(*dst); });
                    }
                    BuiltinId::Floor if is_f => {
                        let k = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FFloorF, a: a0, b: Src::Const(0) });
                        reg.insert(*dst, Src::Val(k));
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); }); // floor yields an Int
                    }
                    BuiltinId::Floor => {
                        reg.insert(*dst, a0); // `floor` of an integer is the identity
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); });
                    }
                    // `ceil(x) = -floor(-x)`: negate the float, take the FFloorF int bridge, negate the integer.
                    BuiltinId::Ceil if is_f => {
                        let kneg = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FSubF, a: Src::Const(0), b: a0 }); // 0.0 - x
                        let kfl = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FFloorF, a: Src::Val(kneg), b: Src::Const(0) });
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Sub, a: Src::Const(0), b: Src::Val(kfl) }); // -floor(-x)
                        reg.insert(*dst, Src::Val(k));
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); }); // ceil yields an Int
                    }
                    BuiltinId::Ceil => {
                        reg.insert(*dst, a0); // `ceil` of an integer is the identity
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); });
                    }
                    // `round(x)` — IEEE round-HALF-AWAY-FROM-ZERO (Rust `f64::round`): `floor(|x| + 0.5)` gives the
                    // magnitude (a tie like 2.5 rounds UP to 3, never to even), then the sign of `x` is applied via
                    // a float-compare `x < 0` select. `LoopCell::Select` only carries an integer compare, so the
                    // sign test is an `FLtF` unit feeding an integer 0/1 predicate.
                    BuiltinId::Round if is_f => {
                        let kabs = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: a0, b: Src::Const(0x7FFF_FFFF_FFFF_FFFF) }); // |x|
                        let ksum = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FAddF, a: Src::Val(kabs), b: Src::Const(0.5f64.to_bits() as i64) });
                        let kfl = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FFloorF, a: Src::Val(ksum), b: Src::Const(0) }); // floor(|x|+0.5) ≥ 0
                        let kneg = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Sub, a: Src::Const(0), b: Src::Val(kfl) }); // -magnitude
                        let kpred = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::FLtF, a: a0, b: Src::Const(0) }); // x < 0.0
                        let ksel = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Ne,
                            ca: Src::Val(kpred),
                            cb: Src::Const(0),
                            t: Src::Val(kneg),
                            e: Src::Val(kfl),
                        });
                        reg.insert(*dst, Src::Val(ksel));
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); }); // round yields an Int
                    }
                    BuiltinId::Round => {
                        reg.insert(*dst, a0); // `round` of an integer is the identity
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); });
                    }
                    BuiltinId::Abs if is_f => {
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: a0, b: Src::Const(0x7FFF_FFFF_FFFF_FFFF) });
                        reg.insert(*dst, Src::Val(k));
                        FLOAT_REGS.with(|t| { t.borrow_mut().insert(*dst); });
                    }
                    BuiltinId::Abs => {
                        let kneg = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Sub, a: Src::Const(0), b: a0 }); // 0 - a
                        let ksel = cells.len();
                        cells.push(LoopCell::Select { cmp: CmpKind::Lt, ca: a0, cb: Src::Const(0), t: Src::Val(kneg), e: a0 });
                        reg.insert(*dst, Src::Val(ksel));
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); });
                    }
                    BuiltinId::Min | BuiltinId::Max => {
                        let a1 = getb(reg, *args_start + 1);
                        let b_f = FLOAT_REGS.with(|t| t.borrow().contains(&(*args_start + 1)));
                        let is_min = matches!(builtin, BuiltinId::Min);
                        if is_f || b_f {
                            // FLOAT min/max: a float-compare functional unit yields the 0/1 predicate
                            // (`FLtF` for min, `FGtF` for max), and a `Select` on `pred != 0` muxes the two
                            // operands — `LoopCell::Select` only carries an INTEGER compare, so the float
                            // comparison must be a Func tile feeding an integer predicate register.
                            let unit = if is_min { FuncUnit::FLtF } else { FuncUnit::FGtF };
                            let kpred = cells.len();
                            cells.push(LoopCell::Func { unit, a: a0, b: a1 });
                            let ksel = cells.len();
                            cells.push(LoopCell::Select {
                                cmp: CmpKind::Ne,
                                ca: Src::Val(kpred),
                                cb: Src::Const(0),
                                t: a0,
                                e: a1,
                            });
                            reg.insert(*dst, Src::Val(ksel));
                            FLOAT_REGS.with(|t| { t.borrow_mut().insert(*dst); }); // min/max of floats is float
                        } else {
                            let cmp = if is_min { CmpKind::Lt } else { CmpKind::Gt };
                            let k = cells.len();
                            cells.push(LoopCell::Select { cmp, ca: a0, cb: a1, t: a0, e: a1 });
                            reg.insert(*dst, Src::Val(k));
                            FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); });
                        }
                    }
                    // WORD32 (ℤ/2³²) lane — the SHA-256 bit-mixing primitives, each self-masking to 32 bits
                    // (a word32 value rides a POSITIVE i64 in `0..2³²`, so an arithmetic `Shr` == logical).
                    BuiltinId::Word32 => {
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: a0, b: Src::Const(0xFFFF_FFFF) });
                        reg.insert(*dst, Src::Val(k));
                    }
                    BuiltinId::IntOfWord32 => {
                        reg.insert(*dst, a0); // the unsigned value as an Int (identity)
                    }
                    BuiltinId::Wand | BuiltinId::Wor => {
                        let a1 = getb(reg, *args_start + 1);
                        let op = if matches!(builtin, BuiltinId::Wand) { AluOp::And } else { AluOp::Or };
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op, a: a0, b: a1 });
                        reg.insert(*dst, Src::Val(k));
                    }
                    BuiltinId::Wnot => {
                        let kx = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Xor, a: a0, b: Src::Const(-1) }); // ~a
                        let km = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: Src::Val(kx), b: Src::Const(0xFFFF_FFFF) });
                        reg.insert(*dst, Src::Val(km));
                    }
                    BuiltinId::Word32Shr => {
                        let n = getb(reg, *args_start + 1);
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Shr, a: a0, b: n });
                        reg.insert(*dst, Src::Val(k));
                    }
                    BuiltinId::Rotr | BuiltinId::Rotl => {
                        let n = getb(reg, *args_start + 1);
                        // rotr = (w >> n) | (w << (32-n)); rotl swaps the two shift directions. Then mask 32.
                        let (dir_a, dir_b) = if matches!(builtin, BuiltinId::Rotr) {
                            (AluOp::Shr, AluOp::Shl)
                        } else {
                            (AluOp::Shl, AluOp::Shr)
                        };
                        let k1 = cells.len();
                        cells.push(LoopCell::Arith { op: dir_a, a: a0, b: n });
                        let kcomp = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Sub, a: Src::Const(32), b: n }); // 32 - n
                        let k2 = cells.len();
                        cells.push(LoopCell::Arith { op: dir_b, a: a0, b: Src::Val(kcomp) });
                        let kor = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Or, a: Src::Val(k1), b: Src::Val(k2) });
                        let km = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: Src::Val(kor), b: Src::Const(0xFFFF_FFFF) });
                        reg.insert(*dst, Src::Val(km));
                    }
                    // WORD64 (ℤ/2⁶⁴) lane — SHA-512 / Keccak 64-bit mixing. A Word64 value rides the FULL i64 bit
                    // pattern, and the VM's `intOfWord64` is `w.to_u64() as i64` (a pure reinterpretation), so
                    // `word64`/`intOfWord64` are IDENTITY and wrapping `+`/`-`/`×` need NO mask (native i64 arithmetic
                    // already wraps mod 2⁶⁴). The only non-`Arith`-identity op is the LOGICAL right shift, which uses
                    // the zero-fill `AluOp::UShr` (the arithmetic `Shr` would sign-extend a bit-63-set value).
                    BuiltinId::Word64 | BuiltinId::IntOfWord64 => {
                        reg.insert(*dst, a0); // a Word64 IS its i64 bit pattern — no truncation, no mask
                    }
                    BuiltinId::Word64And => {
                        let a1 = getb(reg, *args_start + 1);
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::And, a: a0, b: a1 });
                        reg.insert(*dst, Src::Val(k));
                    }
                    BuiltinId::Word64Shl => {
                        let n = getb(reg, *args_start + 1);
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Shl, a: a0, b: n });
                        reg.insert(*dst, Src::Val(k));
                    }
                    BuiltinId::Word64Shr => {
                        let n = getb(reg, *args_start + 1);
                        let k = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::UShr, a: a0, b: n }); // LOGICAL (zero-fill)
                        reg.insert(*dst, Src::Val(k));
                    }
                    // `count_ones` — the combinational PopCount functional unit (set-bit count of the operand).
                    BuiltinId::CountOnes => {
                        let k = cells.len();
                        cells.push(LoopCell::Func { unit: FuncUnit::PopCount, a: a0, b: Src::Const(0) });
                        reg.insert(*dst, Src::Val(k));
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); }); // popcount yields an Int
                    }
                    // DATE-COMPONENT extractors on a clock value (`the year of today`, `the hour of now`). The arg
                    // is a scalar Moment (nanos) or Date (whole days) tracked by REG_TIMEKIND. Calendar fields
                    // (year/month/day/quarter/ISO-week) lower to the full Gregorian `civil_from_days` tile chain;
                    // time-of-day (hour/minute/second) and weekday are pure div/mod. Every `/` is truncating `DivS`
                    // and every `mod` is `ModS` — == the VM's `div_euclid`/`rem_euclid` for the (positive) instants a
                    // real clock produces. Bit-identical to `civil_from_unix_nanos` / `iso_week_from_days`.
                    BuiltinId::YearOf
                    | BuiltinId::MonthOf
                    | BuiltinId::DayOf
                    | BuiltinId::QuarterOf
                    | BuiltinId::WeekOf
                    | BuiltinId::HourOf
                    | BuiltinId::MinuteOf
                    | BuiltinId::SecondOf
                    | BuiltinId::WeekdayOf => {
                        use logicaffeine_base::temporal::{NANOS_PER_DAY, NANOS_PER_SECOND};
                        let kind = REG_TIMEKIND
                            .with(|m| m.borrow().get(args_start).copied())
                            .unwrap_or_else(|| {
                                panic!(
                                    "fabric CallBuiltin {builtin:?}: register {args_start} is not a tracked clock \
                                     value — a date-component extractor needs a `now`/`today` value"
                                )
                            });
                        // days since epoch: a Moment divides its nanos by NANOS_PER_DAY; a Date already IS days.
                        let days = match kind {
                            TimeKind::Moment => t_func(cells, FuncUnit::DivS, a0, Src::Const(NANOS_PER_DAY)),
                            TimeKind::Date => a0,
                        };
                        let result = match builtin {
                            BuiltinId::WeekdayOf => iso_weekday_from_days_tiles(cells, days),
                            BuiltinId::YearOf => civil_from_days_tiles(cells, days).0,
                            BuiltinId::MonthOf => civil_from_days_tiles(cells, days).1,
                            BuiltinId::DayOf => civil_from_days_tiles(cells, days).2,
                            BuiltinId::QuarterOf => {
                                // (month - 1) / 3 + 1.
                                let month = civil_from_days_tiles(cells, days).1;
                                let m1 = t_arith(cells, AluOp::Sub, month, Src::Const(1));
                                let q = t_func(cells, FuncUnit::DivS, m1, Src::Const(3));
                                t_arith(cells, AluOp::Add, q, Src::Const(1))
                            }
                            BuiltinId::WeekOf => {
                                // iso_week_from_days(z).1: the ISO year's Thursday names the year, whose Jan-4
                                // week-1-Monday anchors the count. week = (z - week1_monday) / 7 + 1.
                                let z = days;
                                let iso_dow = iso_weekday_from_days_tiles(cells, z);
                                let four_minus = t_arith(cells, AluOp::Sub, Src::Const(4), iso_dow);
                                let thursday = t_arith(cells, AluOp::Add, z, four_minus);
                                let iso_year = civil_from_days_tiles(cells, thursday).0;
                                let jan4 = days_from_civil_tiles(cells, iso_year, Src::Const(1), Src::Const(4));
                                let dow_jan4 = iso_weekday_from_days_tiles(cells, jan4);
                                let dm1 = t_arith(cells, AluOp::Sub, dow_jan4, Src::Const(1));
                                let week1_monday = t_arith(cells, AluOp::Sub, jan4, dm1);
                                let diff = t_arith(cells, AluOp::Sub, z, week1_monday);
                                let wk = t_func(cells, FuncUnit::DivS, diff, Src::Const(7));
                                t_arith(cells, AluOp::Add, wk, Src::Const(1))
                            }
                            BuiltinId::HourOf | BuiltinId::MinuteOf | BuiltinId::SecondOf => {
                                assert!(
                                    kind == TimeKind::Moment,
                                    "fabric CallBuiltin {builtin:?}: a Date has no time-of-day (the VM errors) — \
                                     use a Moment"
                                );
                                // seconds-of-day = (nanos % NANOS_PER_DAY) / NANOS_PER_SECOND.
                                let rem = t_func(cells, FuncUnit::ModS, a0, Src::Const(NANOS_PER_DAY));
                                let secs = t_func(cells, FuncUnit::DivS, rem, Src::Const(NANOS_PER_SECOND));
                                match builtin {
                                    BuiltinId::HourOf => t_func(cells, FuncUnit::DivS, secs, Src::Const(3600)),
                                    BuiltinId::MinuteOf => {
                                        let m = t_func(cells, FuncUnit::ModS, secs, Src::Const(3600));
                                        t_func(cells, FuncUnit::DivS, m, Src::Const(60))
                                    }
                                    BuiltinId::SecondOf => t_func(cells, FuncUnit::ModS, secs, Src::Const(60)),
                                    _ => unreachable!(),
                                }
                            }
                            _ => unreachable!(),
                        };
                        reg.insert(*dst, result);
                        FLOAT_REGS.with(|t| { t.borrow_mut().remove(dst); }); // a date component is an Int
                    }
                    other => panic!(
                        "loop body: builtin `{other:?}` unsupported (sqrt/floor/abs/min/max/ceil/round/count_ones \
                         + word32 lane + date-component extractors)"
                    ),
                }
                // Track word32-ness: the constructing / bit-op builtins yield a 32-bit value (so a later `+`
                // wraps); `intOfWord32` yields a plain Int; everything else (sqrt/abs/…) is not word32.
                match builtin {
                    BuiltinId::Word32
                    | BuiltinId::Wand
                    | BuiltinId::Wor
                    | BuiltinId::Wnot
                    | BuiltinId::Rotr
                    | BuiltinId::Rotl
                    | BuiltinId::Word32Shr => {
                        W32_REGS.with(|t| {
                            t.borrow_mut().insert(*dst);
                        });
                    }
                    _ => {
                        W32_REGS.with(|t| {
                            t.borrow_mut().remove(dst);
                        });
                    }
                }
            }
            // `ExactDiv` — the Oracle emits it only when the divisor divides the dividend EXACTLY, so the
            // truncating `DivS` yields that same whole quotient (the VM downsizes its exact Rational to the
            // identical Int). Realized under that exact-operand contract; hand-bytecode witnessed on exact
            // operands. (No source emits it on the no-oracle fabric compile path.)
            Op::ExactDiv { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::DivS,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            // `DivPow2` — `lhs / 2^k`. The VM computes `Div` by the constant `2^k` (truncating, NOT an
            // arithmetic shift — they differ for negative `lhs`), so the fabric divides by that same const.
            Op::DivPow2 { dst, lhs, k: kexp } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::DivS,
                    a: getb(reg, *lhs),
                    b: Src::Const(1i64 << kexp),
                });
                reg.insert(*dst, Src::Val(k));
            }
            // `BigIntWide` — the 256-bit BigInt lane. FromInt/Add/Mul are ring-homomorphic on the low bits,
            // and `Low` extracts `mod 2^32` — but the VM returns 0 for `Low` of a value that OVERFLOWED to a
            // BigInt, so the fabric tracks a `big` flag (BIGINT_BIG) that promotes on signed `i64` overflow.
            // Bigness is monotone under Add/Mul (Sub could un-promote, so it — with Cmp/Div — is refused).
            Op::BigIntWide { kind, dst, a, b } => {
                let big_of = |r: &u16| {
                    BIGINT_BIG
                        .with(|m| m.borrow().get(r).copied())
                        .unwrap_or(Src::Const(0))
                };
                match kind {
                    0 => {
                        // FromInt: dst = the Int value, not yet big.
                        let v = getb(reg, *a);
                        reg.insert(*dst, v);
                        BIGINT_BIG.with(|m| { m.borrow_mut().insert(*dst, Src::Const(0)); });
                    }
                    1 => {
                        // Add: value = a + b (i64); big = a.big | b.big | signed_overflow(a, b).
                        let (av, bv) = (getb(reg, *a), getb(reg, *b));
                        let ksum = cells.len();
                        cells.push(LoopCell::Arith { op: AluOp::Add, a: av, b: bv });
                        let sum = Src::Val(ksum);
                        // signed add overflow: ((a ^ sum) & (b ^ sum)) < 0
                        let kxa = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Xor, a: av, b: sum });
                        let kxb = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Xor, a: bv, b: sum });
                        let kand = cells.len(); cells.push(LoopCell::Arith { op: AluOp::And, a: Src::Val(kxa), b: Src::Val(kxb) });
                        let kovf = cells.len(); cells.push(LoopCell::Select { cmp: CmpKind::Lt, ca: Src::Val(kand), cb: Src::Const(0), t: Src::Const(1), e: Src::Const(0) });
                        let ko1 = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Or, a: big_of(a), b: big_of(b) });
                        let ko2 = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Or, a: Src::Val(ko1), b: Src::Val(kovf) });
                        reg.insert(*dst, sum);
                        BIGINT_BIG.with(|m| { m.borrow_mut().insert(*dst, Src::Val(ko2)); });
                    }
                    6 => {
                        // Mul: value = a * b (i64 low); big = a.big | b.big | (128-bit product overflows i64).
                        let (av, bv) = (getb(reg, *a), getb(reg, *b));
                        let kmul = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Mul, a: av, b: bv });
                        let kovf = cells.len(); cells.push(LoopCell::Func { unit: FuncUnit::MulOvfS, a: av, b: bv });
                        let ko1 = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Or, a: big_of(a), b: big_of(b) });
                        let ko2 = cells.len(); cells.push(LoopCell::Arith { op: AluOp::Or, a: Src::Val(ko1), b: Src::Val(kovf) });
                        reg.insert(*dst, Src::Val(kmul));
                        BIGINT_BIG.with(|m| { m.borrow_mut().insert(*dst, Src::Val(ko2)); });
                    }
                    4 => {
                        // Low: 0 if the value is a BigInt, else its low 32 bits. The result is a plain Int.
                        let av = getb(reg, *a);
                        let kmask = cells.len(); cells.push(LoopCell::Arith { op: AluOp::And, a: av, b: Src::Const(0xFFFF_FFFF) });
                        let klow = cells.len(); cells.push(LoopCell::Select { cmp: CmpKind::Ne, ca: big_of(a), cb: Src::Const(0), t: Src::Const(0), e: Src::Val(kmask) });
                        reg.insert(*dst, Src::Val(klow));
                        BIGINT_BIG.with(|m| { m.borrow_mut().remove(dst); }); // a plain Int now
                    }
                    _ => panic!(
                        "fabric BigIntWide: kind {kind} (Sub/Cmp/Div) is a follow-on — Sub can UN-promote a \
                         BigInt to Int (needs the wide value), Cmp/Div need the full magnitude; \
                         FromInt/Add/Mul/Low are realized"
                    ),
                }
            }
            // `a is approximately b` — the numeric approximate-equality FUNCTIONAL UNIT (both operands to
            // f64, `logos_approx_eq`). Float operands need the float lane (refused at the Float load), so only
            // the numeric-int case reaches here; yields a 0/1 Bool.
            Op::ApproxEq { dst, lhs, rhs } => {
                let k = cells.len();
                cells.push(LoopCell::Func {
                    unit: FuncUnit::ApproxEqF,
                    a: getb(reg, *lhs),
                    b: getb(reg, *rhs),
                });
                reg.insert(*dst, Src::Val(k));
            }
            // `MagicDivU` — division/modulo by a CONSTANT via the precomputed magic reciprocal (emitted for a
            // proven non-negative dividend). This reconstructs `compiler::magic_eval` gate-for-gate as tiles:
            // `hi = mulhi_u(magic, n)`, then the shift/add reduction (all shifts LOGICAL — `logical_shr_const`
            // — since the magic path is `u64`), then `mul_back == 0 ? q : n − q·c` (quotient vs remainder).
            // `magic`/`more`/`mul_back` are compile-time immediates, so the branch is resolved at BUILD time.
            Op::MagicDivU {
                dst,
                lhs,
                magic,
                more,
                mul_back,
                divisor: _,
                wrap: _,
            } => {
                const SHIFT_MASK: u8 = 0x3F;
                const ADD_MARKER: u8 = 0x40;
                const SHIFT_PATH: u8 = 0x80;
                let n = getb(reg, *lhs);
                let shift = (*more & SHIFT_MASK) as i64;
                let q = if *more & SHIFT_PATH != 0 {
                    logical_shr_const(cells, n, shift) // pure power-of-two path: q = n >> shift
                } else {
                    let khi = cells.len(); // hi = high 64 bits of the unsigned magic·n product
                    cells.push(LoopCell::Func {
                        unit: FuncUnit::MulHiU,
                        a: Src::Const(*magic as i64),
                        b: n,
                    });
                    let hi = Src::Val(khi);
                    if *more & ADD_MARKER != 0 {
                        // the "add" correction: t = ((n − hi) >>1) + hi ; q = t >> shift
                        let ksub = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Sub,
                            a: n,
                            b: hi,
                        });
                        let half = logical_shr_const(cells, Src::Val(ksub), 1);
                        let kadd = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: half,
                            b: hi,
                        });
                        logical_shr_const(cells, Src::Val(kadd), shift)
                    } else {
                        logical_shr_const(cells, hi, shift) // q = hi >> shift
                    }
                };
                let result = if *mul_back == 0 {
                    q // the quotient x / c
                } else {
                    // the remainder x % c = x − q·c (mul_back carries the divisor c)
                    let kmul = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Mul,
                        a: q,
                        b: Src::Const(*mul_back),
                    });
                    let kres = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Sub,
                        a: n,
                        b: Src::Val(kmul),
                    });
                    Src::Val(kres)
                };
                reg.insert(*dst, result);
            }
            // ── state-plane READS: the heap mirrors the VM's `[len, e1..eN]` (1-based) layout ──
            // `length of xs` = heap[ptr]; `item i of xs` = heap[ptr + i].
            Op::Length { dst, collection } => {
                let ptr = getb(reg, *collection);
                let k = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr });
                reg.insert(*dst, Src::Val(k));
            }
            Op::Index {
                dst,
                collection,
                index,
            }
            | Op::IndexUnchecked {
                dst,
                collection,
                index,
            } => {
                let ptr = getb(reg, *collection);
                let idx = getb(reg, *index);
                let ka = cells.len(); // addr = ptr + i (1-based element i lives at heap[ptr + i])
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: idx,
                });
                let kr = cells.len();
                cells.push(LoopCell::HeapRead { addr: Src::Val(ka) });
                reg.insert(*dst, Src::Val(kr));
            }
            // ── state-plane ALLOC + WRITE: compose HeapAlloc/HeapWrite into fresh lists + in-place set ──
            // `[a, b, ..]` = alloc(count+1), write len header, write each element at heap[ptr + j] (1-based).
            Op::NewList { dst, start, count } => {
                let n = *count;
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(n as i64 + 1),
                });
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(n as i64),
                }); // length header
                for j in 0..n {
                    let elem = getb(reg, *start + j);
                    let kadd = cells.len(); // element j (0-based) is 1-based item j+1 at heap[ptr + (j+1)]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(j as i64 + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kadd),
                        val: elem,
                    });
                }
                reg.insert(*dst, ptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, n as i64));
            }
            Op::Args { dst } => {
                let ptr = heap_argv(cells, 0);
                reg.insert(*dst, ptr);
                TEXT_REGS.with(|t| {
                    t.borrow_mut().remove(dst);
                });
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, 1));
            }
            // An empty list, set, OR map — the same `[0]` (length-0) heap region. (Dedup / key structure only
            // matters once something is added; the empty collection's length read is identical to a list's.)
            Op::NewEmptyListFixed { dst, capacity } => {
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(i64::from(*capacity) + 1),
                });
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(0),
                });
                reg.insert(*dst, ptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, 0));
            }
            Op::NewEmptyList { dst }
            | Op::NewEmptyListWithCapacity { dst, .. }
            | Op::NewEmptyListI32 { dst }
            | Op::NewEmptySet { dst }
            | Op::NewEmptyMap { dst }
            | Op::NewEmptyMapWithCapacity { dst, .. } => {
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(1),
                });
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(0),
                }); // empty: length 0
                reg.insert(*dst, ptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, 0));
            }
            // A tuple `(a, b, c)` shares the list heap shape `[count, e0..e_{count-1}]`; its `count` is in
            // the op, so no type lookup is needed.
            Op::NewTuple { dst, start, count } => {
                let n = *count;
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(n as i64 + 1),
                });
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(n as i64),
                });
                for j in 0..n {
                    let elem = getb(reg, *start + j);
                    let kadd = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(j as i64 + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kadd),
                        val: elem,
                    });
                }
                reg.insert(*dst, ptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, n as i64));
            }
            // `Let (x, y, ..) be t` — read each 0-based component `heap[ptr + 1 + j]` into `R[start + j]`.
            Op::DestructureTuple { src, start, count } => {
                let ptr = getb(reg, *src);
                for j in 0..*count {
                    let kadd = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(j as i64 + 1),
                    });
                    let kr = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kadd),
                    });
                    reg.insert(*start + j, Src::Val(kr));
                }
            }
            // `SetIndex s[name] = v` on a struct is a field write by NAME — how a place mutation clears a field
            // and writes the mutated leaf back — at the slot `GetField`/`StructInsert` resolve for that name.
            Op::SetIndex {
                collection,
                index,
                value,
            }
            | Op::SetIndexUnchecked {
                collection,
                index,
                value,
            } if REG_TYPE.with(|m| m.borrow().contains_key(collection)) => {
                let ptr = getb(reg, *collection);
                let val = getb(reg, *value);
                let name = NAME_REGS
                    .with(|m| m.borrow().get(index).copied())
                    .unwrap_or_else(|| {
                        panic!("struct SetIndex keyed by R{index}, which holds no constant name")
                    });
                let slot = struct_field_slot_named(*collection, name, consts);
                let kadd = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kadd),
                    val,
                });
            }
            // `Set item i of xs to v` — an in-place write (value-semantics COW is upstream DeepClone).
            Op::SetIndex {
                collection,
                index,
                value,
            }
            | Op::SetIndexUnchecked {
                collection,
                index,
                value,
            } => {
                let ptr = getb(reg, *collection);
                let idx = getb(reg, *index);
                let val = getb(reg, *value);
                let kadd = cells.len(); // 1-based: item i lives at heap[ptr + i]
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: idx,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kadd),
                    val,
                });
            }
            // `Pop xs into v` — read the last element (heap[ptr + len]), then shrink the header to len-1.
            Op::ListPop { list, dst } => {
                let ptr = getb(reg, *list);
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // len = heap[ptr]
                let len = Src::Val(klen);
                let kaddr = cells.len(); // last element (1-based item `len`) at heap[ptr + len]
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: len,
                });
                let kr = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kaddr),
                }); // the popped value
                let kdec = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Sub,
                    a: len,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(kdec),
                }); // heap[ptr] = len - 1
                reg.insert(*dst, Src::Val(kr));
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(l) = m.get(list).copied() {
                        m.insert(*list, l - 1);
                    }
                });
            }
            // `Push v to ys` — grow a (top-of-heap) list: write the new element after the last, bump the len.
            Op::ListPush { list, value } => {
                let ptr = getb(reg, *list);
                let val = getb(reg, *value);
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // len = heap[ptr]
                let len = Src::Val(klen);
                let kp1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                let kaddr = cells.len(); // new element at heap[ptr + 1 + len]
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp1),
                    b: len,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val,
                });
                let knl = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: len,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knl),
                }); // heap[ptr] = len + 1
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(l) = m.get(list).copied() {
                        m.insert(*list, l + 1);
                    }
                });
            }
            // `Append v to xs` — for a LIST seq this is exactly `list_push` (the nested-Crdt seq case only
            // arises with an uncovered `NewCrdt`, which the fabric refuses first — so here `seq` is a list).
            Op::CrdtAppend { seq, value } => {
                let ptr = getb(reg, *seq);
                let val = getb(reg, *value);
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr });
                let len = Src::Val(klen);
                let kp1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp1),
                    b: len,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val,
                });
                let knl = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: len,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knl),
                });
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(l) = m.get(seq).copied() {
                        m.insert(*seq, l + 1);
                    }
                });
            }
            // `Push v to s's field` — the struct field HOLDS a list pointer; read it, then push v to that list
            // (using the list's RUNTIME length, so no COLL_LEN needed for the nested list).
            Op::ListPushField { obj, field, src } => {
                let optr = getb(reg, *obj);
                let slot = struct_field_slot(*obj, *field);
                let val = getb(reg, *src);
                let kfa = cells.len(); // &field = optr + 1 + slot
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: optr,
                    b: Src::Const(slot as i64 + 1),
                });
                let klp = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kfa),
                }); // the list pointer stored in the field
                let lptr = Src::Val(klp);
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: lptr }); // list len = heap[lptr]
                let len = Src::Val(klen);
                let kp1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: lptr,
                    b: Src::Const(1),
                });
                let kaddr = cells.len(); // new element at heap[lptr + 1 + len]
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp1),
                    b: len,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val,
                });
                let knl = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: len,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: lptr,
                    val: Src::Val(knl),
                }); // heap[lptr] = len + 1
            }
            // `xs contains v` — membership as an UNROLLED OR-scan over the collection's static length:
            // `OR_{i=0}^{L-1} (heap[ptr+1+i] == v)` yielding a 0/1 Bool. L (from COLL_LEN) is EXACT for the
            // straight-line construction the combinational heap model already requires, so no runtime-length
            // guard is needed. A collection whose length is not statically known is refused loudly.
            Op::Contains {
                dst,
                collection,
                value,
            } => {
                let ptr = getb(reg, *collection);
                let val = getb(reg, *value);
                let max = COLL_LEN
                    .with(|m| m.borrow().get(collection).copied())
                    .expect("fabric Contains: collection length not statically known (a runtime-length scan needs a sequential unit)");
                // Scan up to the STATIC max, but count only slots below the RUNTIME count (heap[ptr]). For a
                // list `count == max` (all pass); for a set after dedup/remove `count < max`, so the
                // uncounted tail slots are masked out — membership is exact for both.
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr });
                let rlen = Src::Val(klen);
                let mut acc = Src::Const(0);
                for i in 0..max {
                    let kaddr = cells.len(); // 1-based item i+1 at heap[ptr + 1 + i]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(i + 1),
                    });
                    let kelem = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kaddr),
                    });
                    let kg = cells.len(); // i < runtime_count ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Lt,
                        ca: Src::Const(i),
                        cb: rlen,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let keq = cells.len(); // elem == v ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Eq,
                        ca: Src::Val(kelem),
                        cb: val,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let kpres = cells.len(); // present = guard AND eq
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(kg),
                        b: Src::Val(keq),
                    });
                    let kor = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Or,
                        a: acc,
                        b: Src::Val(kpres),
                    });
                    acc = Src::Val(kor);
                }
                reg.insert(*dst, acc);
            }
            // `Add v to s` (SET) — DEDUP append. The set's live count is at heap[ptr] (RUNTIME, data-
            // dependent), bounded above by the static max (COLL_LEN = number of adds so far). Scan the
            // counted region for v — each element guarded by `i < runtime_count`; if absent, write v at the
            // top and bump the count. A duplicate lands beyond the count (uncounted, overwritten by the next
            // add), so the deduped LENGTH stays exact. A bounded unroll + runtime guards — combinational.
            Op::SetAdd { set, value } => {
                let ptr = getb(reg, *set);
                let val = getb(reg, *value);
                let max = COLL_LEN
                    .with(|m| m.borrow().get(set).copied())
                    .expect("fabric SetAdd: set max-length not tracked (build the set from a literal/empty)");
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // runtime count = heap[ptr]
                let rlen = Src::Val(klen);
                let mut found = Src::Const(0);
                for i in 0..max {
                    let kaddr = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(i + 1),
                    });
                    let kelem = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kaddr),
                    });
                    let kguard = cells.len(); // i < runtime_count ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Lt,
                        ca: Src::Const(i),
                        cb: rlen,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let keq = cells.len(); // elem == v ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Eq,
                        ca: Src::Val(kelem),
                        cb: val,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let kpres = cells.len(); // present = guard AND eq
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(kguard),
                        b: Src::Val(keq),
                    });
                    let kor = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Or,
                        a: found,
                        b: Src::Val(kpres),
                    });
                    found = Src::Val(kor);
                }
                // write v at heap[ptr + 1 + runtime_count] (uncounted if already present)
                let kp1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                let kwaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp1),
                    b: rlen,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kwaddr),
                    val,
                });
                // new count = found ? runtime_count : runtime_count + 1
                let krlen1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: rlen,
                    b: Src::Const(1),
                });
                let knew = cells.len();
                cells.push(LoopCell::Select {
                    cmp: CmpKind::Eq,
                    ca: found,
                    cb: Src::Const(0),
                    t: Src::Val(krlen1),
                    e: rlen,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knew),
                });
                COLL_LEN.with(|m| {
                    let mut m = m.borrow_mut();
                    m.insert(*set, max + 1); // the static max grows by one per add (regardless of dedup)
                });
            }
            // `Remove v from s` — COMPACTING removal of the (first/only, for a set) occurrence. A running
            // `past` flag turns on at the match and shifts every later slot down by one (`slot i ← slot i+1`);
            // the runtime count drops by one iff found. Read-ahead + write-current (increasing order) keeps
            // reads on the original values. `retain`-equivalent for a unique-element set.
            Op::RemoveFrom { collection, value } => {
                let ptr = getb(reg, *collection);
                let val = getb(reg, *value);
                let max = COLL_LEN
                    .with(|m| m.borrow().get(collection).copied())
                    .expect("fabric RemoveFrom: collection max-length not tracked");
                let klen = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // runtime count = heap[ptr]
                let rlen = Src::Val(klen);
                let mut past = Src::Const(0);
                for i in 0..max {
                    let kaddr = cells.len(); // slot i address (heap[ptr + 1 + i])
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(i + 1),
                    });
                    let kelem = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kaddr),
                    });
                    let kg = cells.len(); // i < runtime_count ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Lt,
                        ca: Src::Const(i),
                        cb: rlen,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let ke = cells.len(); // elem == v ? 1 : 0
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Eq,
                        ca: Src::Val(kelem),
                        cb: val,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let km = cells.len(); // match = guard AND eq
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(kg),
                        b: Src::Val(ke),
                    });
                    let kpast = cells.len(); // past = past OR match (turns on at the match, stays on)
                    cells.push(LoopCell::Arith {
                        op: AluOp::Or,
                        a: past,
                        b: Src::Val(km),
                    });
                    past = Src::Val(kpast);
                    let kaddr2 = cells.len(); // slot i+1 address (heap[ptr + 2 + i]) — read-ahead, still original
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(i + 2),
                    });
                    let knext = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kaddr2),
                    });
                    let knew = cells.len(); // past ? next : elem
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Ne,
                        ca: past,
                        cb: Src::Const(0),
                        t: Src::Val(knext),
                        e: Src::Val(kelem),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kaddr),
                        val: Src::Val(knew),
                    });
                }
                let krlen1 = cells.len(); // count - 1
                cells.push(LoopCell::Arith {
                    op: AluOp::Sub,
                    a: rlen,
                    b: Src::Const(1),
                });
                let knewc = cells.len(); // new count = found ? count-1 : count
                cells.push(LoopCell::Select {
                    cmp: CmpKind::Ne,
                    ca: past,
                    cb: Src::Const(0),
                    t: Src::Val(krlen1),
                    e: rlen,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knewc),
                });
            }
            // `s union t` — a FRESH set = lhs's elements then each rhs element NOT already in lhs (VM order).
            // Copy lhs, then for each rhs element run a nested lhs-membership scan and conditionally append at
            // the running write-count. Bounded (static maxes) → combinational.
            Op::UnionOp { dst, lhs, rhs } => {
                let lptr = getb(reg, *lhs);
                let rptr = getb(reg, *rhs);
                let lmax = COLL_LEN
                    .with(|m| m.borrow().get(lhs).copied())
                    .expect("fabric UnionOp: lhs max-length not tracked");
                let rmax = COLL_LEN
                    .with(|m| m.borrow().get(rhs).copied())
                    .expect("fabric UnionOp: rhs max-length not tracked");
                let klc = cells.len();
                cells.push(LoopCell::HeapRead { addr: lptr });
                let lcount = Src::Val(klc);
                let krc = cells.len();
                cells.push(LoopCell::HeapRead { addr: rptr });
                let rcount = Src::Val(krc);
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(1 + lmax + rmax),
                });
                let nptr = Src::Val(ka);
                // copy all lmax lhs slots (counted ones matter; the tail is overwritten by rhs appends)
                for i in 0..lmax {
                    let ksa = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: lptr,
                        b: Src::Const(i + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kda = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: nptr,
                        b: Src::Const(i + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kda),
                        val: Src::Val(ks),
                    });
                }
                let mut new_count = lcount; // start after lhs's counted elements
                for j in 0..rmax {
                    let kra = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: rptr,
                        b: Src::Const(j + 1),
                    });
                    let kre = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kra),
                    });
                    let rhs_elem = Src::Val(kre);
                    // in_lhs = OR_i ((i < lcount) AND (lhs[1+i] == rhs_elem))
                    let mut in_lhs = Src::Const(0);
                    for i in 0..lmax {
                        let ksa = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: lptr,
                            b: Src::Const(i + 1),
                        });
                        let ks = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(ksa),
                        });
                        let kg = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Lt,
                            ca: Src::Const(i),
                            cb: lcount,
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let keq = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Eq,
                            ca: Src::Val(ks),
                            cb: rhs_elem,
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let kp = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::And,
                            a: Src::Val(kg),
                            b: Src::Val(keq),
                        });
                        let kor = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Or,
                            a: in_lhs,
                            b: Src::Val(kp),
                        });
                        in_lhs = Src::Val(kor);
                    }
                    let kgj = cells.len();
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Lt,
                        ca: Src::Const(j),
                        cb: rcount,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let knil = cells.len();
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Eq,
                        ca: in_lhs,
                        cb: Src::Const(0),
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let kap = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(kgj),
                        b: Src::Val(knil),
                    });
                    let append = Src::Val(kap);
                    // write rhs_elem at new[1 + new_count] (uncounted if not appending)
                    let kwa1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: nptr,
                        b: Src::Const(1),
                    });
                    let kwa2 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: Src::Val(kwa1),
                        b: new_count,
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kwa2),
                        val: rhs_elem,
                    });
                    let knc1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: new_count,
                        b: Src::Const(1),
                    });
                    let knc = cells.len();
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Ne,
                        ca: append,
                        cb: Src::Const(0),
                        t: Src::Val(knc1),
                        e: new_count,
                    });
                    new_count = Src::Val(knc);
                }
                cells.push(LoopCell::HeapWrite {
                    addr: nptr,
                    val: new_count,
                }); // header = union size
                reg.insert(*dst, nptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, lmax + rmax));
            }
            // `s intersection t` — a FRESH set = lhs's elements that are ALSO in rhs (lhs order). Filter each
            // lhs element by a nested rhs-membership scan; keep it (append) iff present. Bounded → combinational.
            Op::IntersectOp { dst, lhs, rhs } => {
                let lptr = getb(reg, *lhs);
                let rptr = getb(reg, *rhs);
                let lmax = COLL_LEN
                    .with(|m| m.borrow().get(lhs).copied())
                    .expect("fabric IntersectOp: lhs max-length not tracked");
                let rmax = COLL_LEN
                    .with(|m| m.borrow().get(rhs).copied())
                    .expect("fabric IntersectOp: rhs max-length not tracked");
                let klc = cells.len();
                cells.push(LoopCell::HeapRead { addr: lptr });
                let lcount = Src::Val(klc);
                let krc = cells.len();
                cells.push(LoopCell::HeapRead { addr: rptr });
                let rcount = Src::Val(krc);
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(1 + lmax),
                }); // intersection ⊆ lhs
                let nptr = Src::Val(ka);
                let mut new_count = Src::Const(0);
                for i in 0..lmax {
                    let ksa = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: lptr,
                        b: Src::Const(i + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let lhs_elem = Src::Val(ks);
                    // in_rhs = OR_j ((j < rcount) AND (rhs[1+j] == lhs_elem))
                    let mut in_rhs = Src::Const(0);
                    for j in 0..rmax {
                        let kra = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: rptr,
                            b: Src::Const(j + 1),
                        });
                        let kr = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(kra),
                        });
                        let kg = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Lt,
                            ca: Src::Const(j),
                            cb: rcount,
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let keq = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Eq,
                            ca: Src::Val(kr),
                            cb: lhs_elem,
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let kp = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::And,
                            a: Src::Val(kg),
                            b: Src::Val(keq),
                        });
                        let kor = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Or,
                            a: in_rhs,
                            b: Src::Val(kp),
                        });
                        in_rhs = Src::Val(kor);
                    }
                    let kgi = cells.len();
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Lt,
                        ca: Src::Const(i),
                        cb: lcount,
                        t: Src::Const(1),
                        e: Src::Const(0),
                    });
                    let kkeep = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::And,
                        a: Src::Val(kgi),
                        b: in_rhs,
                    });
                    let keep = Src::Val(kkeep);
                    // write lhs_elem at new[1 + new_count] (uncounted if not kept)
                    let kwa1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: nptr,
                        b: Src::Const(1),
                    });
                    let kwa2 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: Src::Val(kwa1),
                        b: new_count,
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kwa2),
                        val: lhs_elem,
                    });
                    let knc1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: new_count,
                        b: Src::Const(1),
                    });
                    let knc = cells.len();
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Ne,
                        ca: keep,
                        cb: Src::Const(0),
                        t: Src::Val(knc1),
                        e: new_count,
                    });
                    new_count = Src::Val(knc);
                }
                cells.push(LoopCell::HeapWrite {
                    addr: nptr,
                    val: new_count,
                }); // header = intersection size
                reg.insert(*dst, nptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, lmax));
            }
            // `Concat` — string interpolation's builder: render each operand to a char-list (text: its own;
            // int: `push_int_to_decimal`) and concatenate into a fresh TEXT region. Runtime lengths, so the
            // rhs copy lands at the runtime offset `llen`; the lhs copy's tail junk is overwritten by it.
            Op::Concat { dst, lhs, rhs } => {
                let (lptr, lmax) = {
                    let is_text = TEXT_REGS.with(|t| t.borrow().contains(lhs));
                    if is_text {
                        (
                            getb(reg, *lhs),
                            COLL_LEN.with(|m| m.borrow().get(lhs).copied().unwrap_or(0)),
                        )
                    } else {
                        (push_int_to_decimal(cells, getb(reg, *lhs)), 20)
                    }
                };
                let (rptr, rmax) = {
                    let is_text = TEXT_REGS.with(|t| t.borrow().contains(rhs));
                    if is_text {
                        (
                            getb(reg, *rhs),
                            COLL_LEN.with(|m| m.borrow().get(rhs).copied().unwrap_or(0)),
                        )
                    } else {
                        (push_int_to_decimal(cells, getb(reg, *rhs)), 20)
                    }
                };
                let kll = cells.len();
                cells.push(LoopCell::HeapRead { addr: lptr });
                let llen = Src::Val(kll);
                let krl = cells.len();
                cells.push(LoopCell::HeapRead { addr: rptr });
                let rlen = Src::Val(krl);
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(1 + lmax + rmax),
                });
                let nptr = Src::Val(ka);
                for i in 0..lmax {
                    let ksa = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: lptr,
                        b: Src::Const(i + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kda = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: nptr,
                        b: Src::Const(i + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kda),
                        val: Src::Val(ks),
                    });
                }
                for j in 0..rmax {
                    let ksa = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: rptr,
                        b: Src::Const(j + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kd1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: nptr,
                        b: Src::Const(j + 1),
                    });
                    let kd2 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: Src::Val(kd1),
                        b: llen,
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kd2),
                        val: Src::Val(ks),
                    });
                }
                let knl = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: llen,
                    b: rlen,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: nptr,
                    val: Src::Val(knl),
                });
                reg.insert(*dst, nptr);
                TEXT_REGS.with(|t| {
                    t.borrow_mut().insert(*dst);
                });
                COLL_LEN.with(|m| {
                    m.borrow_mut().insert(*dst, lmax + rmax);
                });
            }
            // `FormatValue` — render a value to TEXT. The DEFAULT case (no `spec`, no debug prefix) is exactly
            // `to_display_string`: a Text value formats to itself, a scalar renders int→decimal (the same
            // `push_int_to_decimal` interpolation uses). Format-SPECS (width/precision/fill) are a follow-on
            // (they matter for floats/alignment) — the fabric refuses them loudly rather than misformat.
            Op::FormatValue {
                dst,
                src,
                spec,
                debug_prefix,
            } => {
                assert!(
                    *spec == u32::MAX && *debug_prefix == u32::MAX,
                    "fabric FormatValue: format-specs / debug prefixes are a follow-on (default rendering only)"
                );
                if TEXT_REGS.with(|t| t.borrow().contains(src)) {
                    let s = getb(reg, *src); // already Text — formats to itself
                    reg.insert(*dst, s);
                    TEXT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                    COLL_LEN.with(|m| {
                        let mut m = m.borrow_mut();
                        if let Some(l) = m.get(src).copied() {
                            m.insert(*dst, l);
                        }
                    });
                } else {
                    let ptr = push_int_to_decimal(cells, getb(reg, *src)); // scalar → decimal char-list
                    reg.insert(*dst, ptr);
                    TEXT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                }
            }
            // A channel is a heap FIFO `[read_cursor, count, e0, e1, ...]`. For SEQUENTIAL (spawn-free) use —
            // sends before recvs, in the straight-line flow — this reproduces the scheduler's VecDeque exactly
            // (a well-formed program never hits the empty-parking / cross-task-handoff paths). Bounded depth,
            // like a real hardware FIFO. (Concurrent COORDINATION — Spawn/Select/Await — stays uncovered.)
            Op::ChanNew { dst, .. } => {
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(2 + 64),
                }); // read_cursor, count, + depth 64
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(0),
                }); // read_cursor = 0
                let kh1 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kh1),
                    val: Src::Const(0),
                }); // count = 0
                reg.insert(*dst, ptr);
            }
            // `Send v into ch` — enqueue at heap[ptr + 2 + count], then bump the count.
            Op::ChanSend { chan, val } => {
                let ptr = getb(reg, *chan);
                let v = getb(reg, *val);
                let kca = cells.len(); // &count = ptr + 1
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                let kc = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kca),
                });
                let count = Src::Val(kc);
                let kp2 = cells.len(); // elem address = ptr + 2 + count
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(2),
                });
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp2),
                    b: count,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val: v,
                });
                let knc = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: count,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kca),
                    val: Src::Val(knc),
                }); // count += 1
            }
            // `Receive x from ch` — dequeue at heap[ptr + 2 + read_cursor], then advance the cursor.
            // `ChanRecv` dequeues the FIFO's front element. `ChanTryRecv` on a NON-EMPTY channel returns that
            // same value (the FIFO-model contract — the empty→Nothing case is the straight-line model's
            // non-empty assumption, exactly as `ChanRecv` assumes a value is present).
            Op::ChanRecv { dst, chan } | Op::ChanTryRecv { dst, chan } => {
                let ptr = getb(reg, *chan);
                let krc = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // read_cursor = heap[ptr]
                let rc = Src::Val(krc);
                let kp2 = cells.len(); // elem address = ptr + 2 + read_cursor
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(2),
                });
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp2),
                    b: rc,
                });
                let kv = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kaddr),
                });
                let knc = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: rc,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knc),
                }); // read_cursor += 1
                reg.insert(*dst, Src::Val(kv));
            }
            // `Try send v into ch` — non-blocking send: always succeeds on the depth-bounded FIFO, so it
            // enqueues exactly like `ChanSend` and reports `true`.
            Op::ChanTrySend { dst, chan, val } => {
                let ptr = getb(reg, *chan);
                let v = getb(reg, *val);
                let kca = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(1),
                });
                let kc = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kca),
                });
                let count = Src::Val(kc);
                let kp2 = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(2),
                });
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp2),
                    b: count,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val: v,
                });
                let knc = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: count,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kca),
                    val: Src::Val(knc),
                });
                reg.insert(*dst, Src::Const(1)); // always succeeds (Bool true)
            }
            // `ChanClose` — mark a channel closed. It is OUTPUT-TRANSPARENT on the deterministic scheduler
            // (already-buffered values stay receivable; only a recv on an empty+closed channel would see the
            // close, a path the straight-line FIFO model does not take), so on the combinational fabric it is
            // a structural no-op — exactly as `rtl.rs`'s hand-bytecode oracle proves (Show is unaffected).
            Op::ChanClose { .. } => {}
            // OFFLINE networking (single node, no relay) — the deterministic model the AOT already proves
            // (tw == VM == AOT). Connect/Stream have no observable effect; `NetSync` is offline-identity (the
            // local CRDT value stands, so `dst` keeps what it already holds → no cell).
            Op::NetConnect { .. } | Op::NetStream { .. } | Op::NetSync { .. } => {}
            // `NetListen` ESTABLISHES the loopback inbox — a FIFO (same shape as a channel). Idempotent.
            Op::NetListen { .. } => {
                if NET_INBOX.with(|n| n.borrow().is_none()) {
                    let ka = cells.len();
                    cells.push(LoopCell::HeapAlloc {
                        size: Src::Const(2 + 64),
                    });
                    let ptr = Src::Val(ka);
                    cells.push(LoopCell::HeapWrite {
                        addr: ptr,
                        val: Src::Const(0),
                    }); // read_cursor = 0
                    let kh1 = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kh1),
                        val: Src::Const(0),
                    }); // count = 0
                    NET_INBOX.with(|n| *n.borrow_mut() = Some(ptr));
                }
            }
            // `NetSend` — the offline loopback PUBLISH: enqueue the message into the inbox (like ChanTrySend).
            // Output-transparent until an `Await` drains it; the peer/topic key is ignored (single node).
            Op::NetSend { msg, .. } => {
                if let Some(ptr) = NET_INBOX.with(|n| *n.borrow()) {
                    let v = getb(reg, *msg);
                    let kca = cells.len(); // count at ptr+1
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(1),
                    });
                    let kc = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kca),
                    });
                    let count = Src::Val(kc);
                    let kp2 = cells.len(); // element addr = ptr + 2 + count
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(2),
                    });
                    let kaddr = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: Src::Val(kp2),
                        b: count,
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kaddr),
                        val: v,
                    });
                    let knc = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: count,
                        b: Src::Const(1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kca),
                        val: Src::Val(knc),
                    }); // count += 1
                }
            }
            // `NetAwait` — DRAIN the loopback inbox (like ChanRecv): dequeue the front message into `dst`.
            Op::NetAwait { dst, .. } => {
                let ptr = NET_INBOX
                    .with(|n| *n.borrow())
                    .expect("fabric NetAwait: no inbox — an Await needs a prior Listen");
                let krc = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // read_cursor = heap[ptr]
                let rc = Src::Val(krc);
                let kp2 = cells.len(); // elem addr = ptr + 2 + read_cursor
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(2),
                });
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(kp2),
                    b: rc,
                });
                let kv = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kaddr),
                });
                let knc = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: rc,
                    b: Src::Const(1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Val(knc),
                }); // read_cursor += 1
                reg.insert(*dst, Src::Val(kv));
            }
            // `NetMakePeer` — an opaque peer HANDLE derived from an address; offline it is only ever passed
            // to send/await (which ignore the key on the single node), so it wires through from its address.
            Op::NetMakePeer { dst, addr } => {
                let a = getb(reg, *addr);
                reg.insert(*dst, a);
            }
            // `select` — register each arm (a `Receive` binds `(channel, var)`, a timeout binds nothing); the
            // `SelectWait` below resolves them. No cell — just recorded pending state, in declaration order.
            Op::SelectArmRecv { chan, var } => {
                let ch = getb(reg, *chan);
                SELECT_ARMS.with(|a| a.borrow_mut().push(Some((ch, *var))));
            }
            Op::SelectArmTimeout { .. } => {
                SELECT_ARMS.with(|a| a.borrow_mut().push(None));
            }
            // `SelectWait` — the PRIORITY SELECT: the winner is the FIRST arm whose channel is non-empty
            // (`count > read_cursor`), else the timeout arm. `dst_arm` gets the winner's index; ONLY the
            // winner dequeues (a guarded `read_cursor += 1`), and each recv arm's `var` is wired to its front
            // value (dead for the losers — the dispatch runs only the winner's body). Combinational.
            Op::SelectWait { dst_arm } => {
                let arms = SELECT_ARMS.with(|a| std::mem::take(&mut *a.borrow_mut()));
                // Per recv arm: (ptr, read_cursor, ready, front_value, var).
                let mut info: Vec<Option<(Src, Src, Src, Src, u16)>> = Vec::new();
                for arm in &arms {
                    match arm {
                        Some((ptr, var)) => {
                            let ptr = *ptr;
                            let krc = cells.len();
                            cells.push(LoopCell::HeapRead { addr: ptr }); // read_cursor
                            let rc = Src::Val(krc);
                            let kca = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: ptr,
                                b: Src::Const(1),
                            });
                            let kc = cells.len();
                            cells.push(LoopCell::HeapRead {
                                addr: Src::Val(kca),
                            }); // count
                            let krdy = cells.len(); // ready = count > read_cursor
                            cells.push(LoopCell::Select {
                                cmp: CmpKind::Gt,
                                ca: Src::Val(kc),
                                cb: rc,
                                t: Src::Const(1),
                                e: Src::Const(0),
                            });
                            let kp2 = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: ptr,
                                b: Src::Const(2),
                            });
                            let kaddr = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: Src::Val(kp2),
                                b: rc,
                            });
                            let kval = cells.len();
                            cells.push(LoopCell::HeapRead {
                                addr: Src::Val(kaddr),
                            }); // front value
                            info.push(Some((ptr, rc, Src::Val(krdy), Src::Val(kval), *var)));
                        }
                        None => info.push(None),
                    }
                }
                // winner index: build from the back so earlier arms take priority; default = the timeout arm
                // (or arm 0 if none — the witness always makes a recv ready, so that fallback is never hit).
                let timeout_idx = arms.iter().position(Option::is_none).map(|i| i as i64);
                let mut winner = Src::Const(timeout_idx.unwrap_or(0));
                for (i, arm) in info.iter().enumerate().rev() {
                    if let Some((_, _, ready, _, _)) = arm {
                        let kw = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Ne,
                            ca: *ready,
                            cb: Src::Const(0),
                            t: Src::Const(i as i64),
                            e: winner,
                        });
                        winner = Src::Val(kw);
                    }
                }
                reg.insert(*dst_arm, winner);
                // deliver each recv arm's front value to its var; guarded dequeue (only the winner advances).
                for (i, arm) in info.iter().enumerate() {
                    if let Some((ptr, rc, _, val, var)) = arm {
                        reg.insert(*var, *val);
                        let kwon = cells.len(); // won = (winner == i)
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Eq,
                            ca: winner,
                            cb: Src::Const(i as i64),
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let krc1 = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: *rc,
                            b: Src::Const(1),
                        });
                        let knew = cells.len(); // new read_cursor = won ? rc+1 : rc
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Ne,
                            ca: Src::Val(kwon),
                            cb: Src::Const(0),
                            t: Src::Val(krc1),
                            e: *rc,
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: *ptr,
                            val: Src::Val(knew),
                        });
                    }
                }
            }
            // `EnsureOwned` — the call-site COW barrier: before a mutable-borrow callee mutates a collection,
            // isolate it so the write can't leak to an ALIAS (`Let b be a. grow(a). b unchanged`). ALWAYS
            // cloning is observably identical to the VM's clone-iff-shared: an extra copy of a unique
            // collection is invisible (LOGOS has no pointer identity), and rewiring `reg` to the copy is
            // correct either way. A scalar (no tracked length) has no aliasing → no-op.
            Op::EnsureOwned { reg: r } => {
                if let Some(len) = COLL_LEN.with(|m| m.borrow().get(r).copied()) {
                    let sptr = getb(reg, *r);
                    let ka = cells.len();
                    cells.push(LoopCell::HeapAlloc {
                        size: Src::Const(len + 1),
                    });
                    let dptr = Src::Val(ka);
                    cells.push(LoopCell::HeapWrite {
                        addr: dptr,
                        val: Src::Const(len),
                    }); // length header
                    for i in 0..len {
                        let ksa = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: sptr,
                            b: Src::Const(i + 1),
                        });
                        let ks = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(ksa),
                        });
                        let kda = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: dptr,
                            b: Src::Const(i + 1),
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: Src::Val(kda),
                            val: Src::Val(ks),
                        });
                    }
                    reg.insert(*r, dptr); // rewire to the owned copy
                }
            }
            // `copy of xs` — DEEP CLONE into a FRESH heap region: a mutation of the clone can't reach the
            // original (value semantics). Unrolled element copy over the static length `[len, e0..]`.
            Op::DeepClone { dst, src } => {
                let sptr = getb(reg, *src);
                let len = COLL_LEN
                    .with(|m| m.borrow().get(src).copied())
                    .expect("fabric DeepClone: source length not statically known");
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(len + 1),
                });
                let dptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: dptr,
                    val: Src::Const(len),
                }); // length header
                for i in 0..len {
                    let ksa = cells.len(); // src item i+1 at heap[sptr + 1 + i]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: sptr,
                        b: Src::Const(i + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kda = cells.len(); // dst item i+1 at heap[dptr + 1 + i]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: dptr,
                        b: Src::Const(i + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kda),
                        val: Src::Val(ks),
                    });
                }
                reg.insert(*dst, dptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, len));
            }
            // `xs followed by ys` — SEQUENCE CONCATENATION into a fresh `[len_l+len_r, xs.., ys..]` region.
            // Both operand lengths are statically known, so the two element copies unroll.
            Op::SeqConcat { dst, lhs, rhs } => {
                let lptr = getb(reg, *lhs);
                let rptr = getb(reg, *rhs);
                let ll = COLL_LEN
                    .with(|m| m.borrow().get(lhs).copied())
                    .expect("fabric SeqConcat: left length not statically known");
                let lr = COLL_LEN
                    .with(|m| m.borrow().get(rhs).copied())
                    .expect("fabric SeqConcat: right length not statically known");
                let total = ll + lr;
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(total + 1),
                });
                let dptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: dptr,
                    val: Src::Const(total),
                });
                // copy lhs → dst[0..ll], then rhs → dst[ll..ll+lr]
                for (base_ptr, off, count) in [(lptr, 0i64, ll), (rptr, ll, lr)] {
                    for i in 0..count {
                        let ksa = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: base_ptr,
                            b: Src::Const(i + 1),
                        });
                        let ks = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(ksa),
                        });
                        let kda = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: dptr,
                            b: Src::Const(off + i + 1),
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: Src::Val(kda),
                            val: Src::Val(ks),
                        });
                    }
                }
                reg.insert(*dst, dptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, total));
            }
            // `items s through e of xs` — SLICE into a fresh region. VM: 1-based `s` → 0-based
            // `(s-1).saturating`, valid iff `start0 < e && e <= len`, result `v[start0 ..= e-1]` (count
            // `e - start0`) else empty. Const bounds unroll; a runtime bound is refused (needs a seq unit).
            Op::SliceOp {
                dst,
                collection,
                start,
                end,
            } => {
                let cptr = getb(reg, *collection);
                let len = COLL_LEN
                    .with(|m| m.borrow().get(collection).copied())
                    .expect("fabric SliceOp: collection length not statically known");
                let s = match getb(reg, *start) {
                    Src::Const(v) => v,
                    _ => panic!("fabric SliceOp: non-constant start bound (a runtime bound needs a sequential unit)"),
                };
                let e = match getb(reg, *end) {
                    Src::Const(v) => v,
                    _ => panic!("fabric SliceOp: non-constant end bound (a runtime bound needs a sequential unit)"),
                };
                let start0 = (s - 1).max(0);
                let count = if start0 < e && e <= len {
                    e - start0
                } else {
                    0
                };
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(count + 1),
                });
                let dptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: dptr,
                    val: Src::Const(count),
                });
                for j in 0..count {
                    let ksa = cells.len(); // src 0-based (start0+j) → 1-based item at heap[cptr + start0 + j + 1]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: cptr,
                        b: Src::Const(start0 + j + 1),
                    });
                    let ks = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kda = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: dptr,
                        b: Src::Const(j + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kda),
                        val: Src::Val(ks),
                    });
                }
                reg.insert(*dst, dptr);
                COLL_LEN.with(|m| m.borrow_mut().insert(*dst, count));
            }
            // Structs share the list heap shape `[n, f0..f_{n-1}]`. NewStruct's field count comes from the
            // program context (STRUCT_FIELDS); its fields default to 0 (the HeapAlloc zero-fills, matching the
            // VM). GetField/StructInsert carry the 0-based field index directly → heap[ptr + 1 + field].
            Op::NewStruct { dst, type_name } => {
                let n = STRUCT_FIELDS.with(|m| m.borrow().get(type_name).copied().unwrap_or(0));
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(n as i64 + 1),
                });
                let ptr = Src::Val(ka);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(n as i64),
                }); // header = field count
                reg.insert(*dst, ptr);
                REG_TYPE.with(|m| m.borrow_mut().insert(*dst, *type_name)); // this reg now holds a `type_name`
            }
            // `field` is a CONSTANT INDEX to the field NAME; register type-tracking (REG_TYPE, set at
            // NewStruct) recovers the object's struct type, and STRUCT_FIELD_POS maps (type, name)→slot.
            // heap[ptr + 1 + slot] — the same slot NewStruct zero-filled and StructInsert writes.
            Op::GetField { dst, obj, field } => {
                let ptr = getb(reg, *obj);
                let slot = struct_field_slot(*obj, *field);
                let kadd = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                let kr = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kadd),
                });
                reg.insert(*dst, Src::Val(kr));
            }
            Op::StructInsert { obj, field, value } => {
                let ptr = getb(reg, *obj);
                let val = getb(reg, *value);
                let slot = struct_field_slot(*obj, *field);
                let kadd = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kadd),
                    val,
                });
            }
            // `Increase/Decrease s's field by amount` — a CRDT counter bump on a `Shared` struct field:
            // `field ±= amount` (crdt_counter_bump is a wrapping add; a fresh field 0 matches the VM's
            // Nothing→amount). The single-replica value is just the running sum — replica ids don't affect it.
            Op::CrdtBump {
                obj,
                field,
                amount,
                negate,
            } => {
                let ptr = getb(reg, *obj);
                let slot = struct_field_slot(*obj, *field);
                let amt = getb(reg, *amount);
                let kaddr = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                let kcur = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kaddr),
                });
                let knew = cells.len();
                cells.push(LoopCell::Arith {
                    op: if *negate { AluOp::Sub } else { AluOp::Add },
                    a: Src::Val(kcur),
                    b: amt,
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kaddr),
                    val: Src::Val(knew),
                });
            }
            // `Resolve s's field to value` — resolve a plain field: the VM OVERWRITES it (value semantics),
            // so it is exactly a struct field write.
            Op::CrdtResolve { obj, field, value } => {
                let ptr = getb(reg, *obj);
                let slot = struct_field_slot(*obj, *field);
                let val = getb(reg, *value);
                let kadd = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                cells.push(LoopCell::HeapWrite {
                    addr: Src::Val(kadd),
                    val,
                });
            }
            // `Merge source into target` — for a plain-Int-field `Shared` struct, `crdt_merge_field` per
            // field is a wrapping ADD (arith.rs:1089), so merge = per-field `target[i] += source[i]` over the
            // struct's fields. (Nested G-Counter/Set/Map fields need an uncovered NewCrdt, refused first.)
            Op::CrdtMerge { target, source } => {
                let tptr = getb(reg, *target);
                let sptr = getb(reg, *source);
                let tn = REG_TYPE
                    .with(|m| m.borrow().get(target).copied())
                    .expect("fabric CrdtMerge: target struct type not tracked");
                let fcount = STRUCT_FIELDS
                    .with(|m| m.borrow().get(&tn).copied())
                    .expect("fabric CrdtMerge: struct field count not known");
                for i in 0..fcount {
                    let kta = cells.len(); // target slot i at heap[tptr + 1 + i]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: tptr,
                        b: Src::Const(i as i64 + 1),
                    });
                    let ktc = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kta),
                    });
                    let ksa = cells.len(); // source slot i
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: sptr,
                        b: Src::Const(i as i64 + 1),
                    });
                    let ksv = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let ksum = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: Src::Val(ktc),
                        b: Src::Val(ksv),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kta),
                        val: Src::Val(ksum),
                    });
                }
            }
            // `CrdtJoin` — the state-based G-Counter LATTICE JOIN: per-field MAX (idempotent), the fixed-lane
            // join `crdt_merge` computes. Unlike CrdtMerge's ADD, this never double-counts a redelivered
            // state. No LOGOS source emits it (hand-bytecode witnessed) — same struct heap walk, `max` tile.
            Op::CrdtJoin { target, source } => {
                let tptr = getb(reg, *target);
                let sptr = getb(reg, *source);
                let tn = REG_TYPE
                    .with(|m| m.borrow().get(target).copied())
                    .expect("fabric CrdtJoin: target struct type not tracked");
                let fcount = STRUCT_FIELDS
                    .with(|m| m.borrow().get(&tn).copied())
                    .expect("fabric CrdtJoin: struct field count not known");
                for i in 0..fcount {
                    let kta = cells.len(); // target slot i at heap[tptr + 1 + i]
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: tptr,
                        b: Src::Const(i as i64 + 1),
                    });
                    let ktc = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kta),
                    });
                    let ksa = cells.len(); // source slot i
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: sptr,
                        b: Src::Const(i as i64 + 1),
                    });
                    let ksv = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(ksa),
                    });
                    let kmax = cells.len(); // per-field lattice MAX: target[i] = max(target[i], source[i])
                    cells.push(LoopCell::Select {
                        cmp: CmpKind::Gt,
                        ca: Src::Val(ktc),
                        cb: Src::Val(ksv),
                        t: Src::Val(ktc),
                        e: Src::Val(ksv),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kta),
                        val: Src::Val(kmax),
                    });
                }
            }
            // Enums (algebraic data types) share the list/struct heap shape: `[tag, arg0..arg_{count-1}]`.
            // The tag is the constructor's canonical name-id (`canonical_text_tag`), so a `TestArm` tag
            // comparison is exactly the VM's `constructor == variant_name`.
            Op::NewInductive {
                dst,
                type_name: _,
                ctor,
                args_start,
                count,
            } => {
                let ka = cells.len();
                cells.push(LoopCell::HeapAlloc {
                    size: Src::Const(*count as i64 + 1),
                });
                let ptr = Src::Val(ka);
                let tag = canonical_text_tag(consts, *ctor);
                cells.push(LoopCell::HeapWrite {
                    addr: ptr,
                    val: Src::Const(tag),
                }); // header = variant tag
                for k in 0..*count {
                    let av = getb(reg, *args_start + k);
                    let kadd = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(k as i64 + 1),
                    });
                    cells.push(LoopCell::HeapWrite {
                        addr: Src::Val(kadd),
                        val: av,
                    }); // arg k at ptr+1+k
                }
                reg.insert(*dst, ptr);
            }
            // TestArm `(target is variant)` → the tag read compared to the variant's tag, registered as a
            // pending predicate so the enclosing match's `JumpIfFalse` if-converts it into a `Select`.
            Op::TestArm {
                dst,
                target,
                variant,
            } => {
                let ptr = getb(reg, *target);
                let ktag = cells.len();
                cells.push(LoopCell::HeapRead { addr: ptr }); // tag = heap[ptr]
                let vtag = canonical_text_tag(consts, *variant);
                preds.insert(*dst, (CmpKind::Eq, Src::Val(ktag), Src::Const(vtag)));
            }
            // BindArm extracts a payload: an Inductive binds by positional `index`; a Struct target (a
            // `When` on a struct) binds by field NAME via `struct_field_slot`. `heap[ptr + 1 + slot]`.
            Op::BindArm {
                dst,
                target,
                field,
                index,
            } => {
                let ptr = getb(reg, *target);
                let is_struct = REG_TYPE.with(|m| m.borrow().contains_key(target));
                let slot = if is_struct {
                    struct_field_slot(*target, *field)
                } else {
                    *index as usize
                };
                let kadd = cells.len();
                cells.push(LoopCell::Arith {
                    op: AluOp::Add,
                    a: ptr,
                    b: Src::Const(slot as i64 + 1),
                });
                let kr = cells.len();
                cells.push(LoopCell::HeapRead {
                    addr: Src::Val(kadd),
                });
                reg.insert(*dst, Src::Val(kr));
            }
            // Direct call → INLINE the callee's decision fabric (Futamura). The caller's argument registers
            // become the callee's parameter seeds; the callee's datapath emits into the shared `cells`.
            Op::Call {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                let arg_srcs: Vec<Src> = (0..*arg_count)
                    .map(|k| getb(reg, *args_start + k))
                    .collect();
                let result = inline_callee(*func as usize, &arg_srcs, cells, consts);
                reg.insert(*dst, result);
            }
            // `MakeClosure` — capture the closure's free variables in the body's `captures` order: a LOCAL
            // capture reads `locals_start + k`; a GLOBAL capture snapshots `GLOBALS` (present iff defined).
            // The body's function index is STATIC, so `CallValue` can resolve + inline it (dynamic dispatch
            // as a Futamura composition). No cell — a closure is tracked dataflow, like a Move.
            Op::MakeClosure {
                dst,
                func,
                locals_start,
            } => {
                let layout = CLOSURE_CAPTURES
                    .with(|m| m.borrow().get(*func as usize).cloned().unwrap_or_default());
                let mut captures: Vec<(Src, bool)> = Vec::with_capacity(layout.len());
                let mut local_k: u16 = 0;
                for cap in &layout {
                    match cap {
                        None => {
                            captures.push((getb(reg, *locals_start + local_k), true)); // a local, always present
                            local_k += 1;
                        }
                        Some(gidx) => match GLOBALS.with(|g| g.borrow().get(gidx).copied()) {
                            Some(slot) => captures.push((slot.src, true)),
                            None => captures.push((Src::Const(0), false)), // undefined global → not captured
                        },
                    }
                }
                CLOSURES.with(|c| c.borrow_mut().insert(*dst, (*func as usize, captures)));
            }
            // `CallValue` — call a closure value: resolve its (static body, captures) and INLINE with the
            // args seeding the params and the captures seeding the two capture bands of the frame.
            Op::CallValue {
                dst,
                callee,
                args_start,
                arg_count,
                ..
            } => {
                let (func, captures) = CLOSURES
                    .with(|c| c.borrow().get(callee).cloned())
                    .expect("fabric CallValue: callee is not a tracked closure (dynamic-from-data dispatch is unsupported)");
                let arg_srcs: Vec<Src> = (0..*arg_count)
                    .map(|k| getb(reg, *args_start + k))
                    .collect();
                let result = inline_closure(func, &arg_srcs, &captures, cells, consts);
                reg.insert(*dst, result);
            }
            // `Launch f(..)` — SPAWN a child: inline its straight-line side effects at the spawn point (a
            // producer's channel enqueues fill the buffer the parent then drains — the same observable as the
            // cooperative schedule). `SpawnHandle` additionally yields an opaque task handle (never observed
            // as data). Their `TaskAwait` join is instant (the child already ran); `TaskAbort` is a no-op.
            Op::Spawn {
                func,
                args_start,
                arg_count,
            } => {
                let args: Vec<Src> = (0..*arg_count)
                    .map(|k| getb(reg, *args_start + k))
                    .collect();
                inline_spawn(*func as usize, &args, cells, consts);
            }
            Op::SpawnHandle {
                dst,
                func,
                args_start,
                arg_count,
            } => {
                let args: Vec<Src> = (0..*arg_count)
                    .map(|k| getb(reg, *args_start + k))
                    .collect();
                inline_spawn(*func as usize, &args, cells, consts);
                reg.insert(*dst, Src::Const(0)); // opaque task handle
            }
            Op::TaskAwait { dst, .. } => {
                // the spawned child ran to completion at the spawn point; the VM delivers `Nothing` as the
                // (unpropagated) task result, so the join passes control through with a sentinel.
                reg.insert(*dst, Src::Const(0));
            }
            Op::TaskAbort { .. } => {}
            // A promoted global's DEFINITION — remember the definition's wire + its type metadata under the
            // global index. No cell: a global is dataflow, not storage (the straight-line set-before-use
            // invariant makes it a plain wire — the fabric analog of the VM's `globals[idx] = v`).
            Op::GlobalSet { idx, src } => {
                let s = getb(reg, *src);
                let len = COLL_LEN.with(|m| m.borrow().get(src).copied());
                let text = TEXT_REGS.with(|t| t.borrow().contains(src));
                let ty = REG_TYPE.with(|m| m.borrow().get(src).copied());
                GLOBALS.with(|g| {
                    g.borrow_mut().insert(
                        *idx,
                        GSlot {
                            src: s,
                            len,
                            text,
                            ty,
                        },
                    )
                });
            }
            // A promoted global's USE — wire the remembered definition (and its metadata) into `dst`.
            Op::GlobalGet { dst, idx } => {
                let slot = GLOBALS
                    .with(|g| g.borrow().get(idx).copied())
                    .unwrap_or_else(|| {
                        panic!("fabric GlobalGet: global #{idx} read before it was set")
                    });
                reg.insert(*dst, slot.src);
                if let Some(l) = slot.len {
                    COLL_LEN.with(|m| {
                        m.borrow_mut().insert(*dst, l);
                    });
                }
                if slot.text {
                    TEXT_REGS.with(|t| {
                        t.borrow_mut().insert(*dst);
                    });
                }
                if let Some(t) = slot.ty {
                    REG_TYPE.with(|m| {
                        m.borrow_mut().insert(*dst, t);
                    });
                }
            }
            // `Sleep n` — a temporal delay. The combinational fabric has no clock, so in the deterministic
            // execution model a delay is vacuous: no data effect, no cell (the concurrent VM confirms the
            // output is identical with or without it). A structural no-op, exactly like the VM's yield.
            Op::Sleep { .. } => {}
            // `RegionBoundsGuard` — an Oracle bounds-check assertion (the loop index stays in `[min, max]`).
            // It has NO data effect (the VM executes it as `pc += 1`), so on the fabric it is a structural
            // no-op — the guarantee it encodes is what let `IndexUnchecked` drop its check in the first place.
            Op::RegionBoundsGuard { .. } => {}
            // `IterPop` — pop the iterator stack at a loop's end. The fabric's loop model carries the loop
            // index explicitly (no iterator-stack object), so popping it is vacuous — a structural no-op,
            // exactly as the loop-fabric extractor already treats it.
            Op::IterPop => {}
            // `Obs` — a Wave-13 observability event (span/message). It NEVER writes stdout — it feeds a
            // separate trace sink, and only when one is capturing (off by default). So it has no effect on
            // the program's OUTPUT, ever: a structural no-op on the fabric's output-tape model.
            Op::Obs { .. } => {}
            // `FailWith msg` — raise an error: the VM returns `Err` here, so the program produces NO further
            // output. On the fabric this HALTS the output tape (a control effect modeled on the observable):
            // every later `Show` is suppressed, so the tape equals the VM's output up to the failure.
            Op::FailWith { .. } => {
                HALTED.with(|h| h.set(true));
            }
            // `Show v` — append the value + its text-ness to the WHOLE-PROGRAM output tape (straight-line
            // only). Suppressed once a `FailWith` has halted the program (see `HALTED`).
            Op::Show { src } => {
                assert!(
                    !FLOAT_REGS.with(|t| t.borrow().contains(src)),
                    "fabric Show: rendering a FLOAT to a string (IEEE float→decimal) is a follow-on — the \
                     float lane is witnessed through int-returning results (floor/compare), not Show-of-float"
                );
                assert!(
                    !BIGINT_BIG.with(|m| m.borrow().contains_key(src)),
                    "fabric Show: rendering a BigInt to a string is a follow-on — the BigInt lane is witnessed \
                     through `Low` (an Int), not Show-of-BigInt"
                );
                if !HALTED.with(std::cell::Cell::get) {
                    let s = getb(reg, *src);
                    let is_text = TEXT_REGS.with(|t| t.borrow().contains(src));
                    // The policy GRANT guard in force at THIS Show: a failed prior `Check` makes it eval
                    // false, and `eval_whole_program` halts the tape here (VM `Err` semantics). `None` = no
                    // check gates this Show, so it always prints.
                    let guard = POLICY_GUARD.with(|g| g.borrow().clone());
                    SHOW_OUTPUTS.with(|o| o.borrow_mut().push((s, is_text, guard)));
                }
            }
            // `Check subject is/can predicate` — a SECURITY guard. The VM evaluates the policy condition and
            // returns `Err` on denial (so NOTHING prints after a failed check); a granted check is
            // transparent. On the fabric the condition is compiled to a GRANT predicate `subject.field ==
            // expected` (a 0/1 `Select`), AND-chained into the running `POLICY_GUARD`; each later `Show`
            // carries that guard and `eval_whole_program` halts the tape at the first false one — the same
            // observable as the VM's `Err`. Only the SINGLE-FIELD conditions extracted into
            // `POLICY_CONDITIONS` are modeled; an object-bearing (cross-field) or otherwise-unmodeled check
            // PANICS rather than silently granting (a wrong grant is a security hole — rule 8).
            Op::CheckPolicy {
                subject,
                predicate,
                is_capability,
                object,
                source_text,
            } => {
                let _ = source_text; // the VM's `Err` text; the fabric models the OBSERVABLE (tape halts).
                assert!(
                    *object == crate::vm::Reg::MAX,
                    "fabric CheckPolicy: an object-bearing (cross-field) capability is a follow-on — the \
                     modeled conditions are single-field FieldBool/FieldEquals only; refusing rather than \
                     silently granting"
                );
                let tn = REG_TYPE
                    .with(|m| m.borrow().get(subject).copied())
                    .expect("fabric CheckPolicy: subject register has no tracked struct type");
                let key = (tn, predicate.index() as u32, *is_capability);
                let spec = POLICY_CONDITIONS
                    .with(|m| m.borrow().get(&key).cloned())
                    .unwrap_or_else(|| {
                        panic!(
                            "fabric CheckPolicy: no MODELED condition for (type-const {tn}, predicate \
                             {predicate:?}, capability {is_capability}) — a compound (And/Or) / object-cross- \
                             field / predicate-indirection condition is a follow-on; refusing rather than \
                             silently granting (a wrong grant is a security hole)"
                        )
                    });
                // Read `subject.field` (`heap[ptr + 1 + slot]`) — shared by the Int and Text grants.
                let field_val = |cells: &mut Vec<LoopCell>, slot: usize| -> Src {
                    let ptr = getb(reg, *subject);
                    let kaddr = cells.len();
                    cells.push(LoopCell::Arith {
                        op: AluOp::Add,
                        a: ptr,
                        b: Src::Const(slot as i64 + 1),
                    });
                    let kfield = cells.len();
                    cells.push(LoopCell::HeapRead {
                        addr: Src::Val(kaddr),
                    });
                    Src::Val(kfield)
                };
                let grant = match spec {
                    // Always-deny: the VM's compare is unsatisfiable, so a constant-0 grant.
                    PolicyGrant::AlwaysDeny => Src::Const(0),
                    // `subject.field == expected` (Bool/Int).
                    PolicyGrant::IntEq { slot, expected } => {
                        let fv = field_val(cells, slot);
                        let kg = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Eq,
                            ca: fv,
                            cb: Src::Const(expected),
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        Src::Val(kg)
                    }
                    // Text field: `subject.field` is a char-list pointer `[len, b0, b1, …]`; grant ⇔ the length
                    // matches AND every byte matches `expected`. Each check is a 0/1 `Select`, AND-chained by
                    // multiply — a bounded, combinational string-equality, bit-identical to the VM's `==`.
                    PolicyGrant::TextEq { slot, expected } => {
                        let strptr = field_val(cells, slot);
                        let klen = cells.len();
                        cells.push(LoopCell::HeapRead { addr: strptr }); // heap[strptr] = length
                        let klenchk = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Eq,
                            ca: Src::Val(klen),
                            cb: Src::Const(expected.len() as i64),
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        let mut acc = Src::Val(klenchk);
                        for (i, b) in expected.iter().enumerate() {
                            let kba = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: strptr,
                                b: Src::Const(i as i64 + 1),
                            });
                            let kbv = cells.len();
                            cells.push(LoopCell::HeapRead {
                                addr: Src::Val(kba),
                            });
                            let kbc = cells.len();
                            cells.push(LoopCell::Select {
                                cmp: CmpKind::Eq,
                                ca: Src::Val(kbv),
                                cb: Src::Const(i64::from(*b)),
                                t: Src::Const(1),
                                e: Src::Const(0),
                            });
                            let kand = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Mul,
                                a: acc,
                                b: Src::Val(kbc),
                            });
                            acc = Src::Val(kand);
                        }
                        acc
                    }
                };
                POLICY_GUARD.with(|g| {
                    let mut g = g.borrow_mut();
                    *g = Some(match g.take() {
                        None => grant,
                        // AND two 0/1 guards = multiply (1·1 = 1, else 0) — one more residual cell.
                        Some(prev) => {
                            let kand = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Mul,
                                a: prev,
                                b: grant,
                            });
                            Src::Val(kand)
                        }
                    });
                });
            }
            other => panic!(
                "loop body arm: `{}` unsupported (nested control flow is future work)",
                other.name()
            ),
        }
    }
}

/// If-CONVERT a loop body: run any leading straight-line ops, then — if the body branches —
/// execute BOTH arms from the SAME entry state and merge each carried register with a predicated
/// `Select` (the mux tile). Mutates `reg` to the post-body state; returns the body cells. Control
/// becomes data; no divergence. Shared by the counted-loop and `while`-loop extractors.
fn if_convert_body(
    body: &[(usize, crate::vm::Op)],
    reg: &mut std::collections::HashMap<u16, Src>,
    carried: &[u16],
    consts: &[crate::vm::Constant],
    // Pre-seeded cells: a SEQUENCE loop seeds cell 0 = `MemRead(addr = iv)` so the body's element `x`
    // (mapped to `Src::Val(0)`) reads `seq[iv]`. A numeric loop / `while` passes an empty vec.
    seed_cells: Vec<LoopCell>,
) -> Vec<LoopCell> {
    use crate::vm::Op;
    let ops_of = |seg: &[(usize, Op)]| -> Vec<Op> { seg.iter().map(|(_, o)| *o).collect() };
    let idx_of_pc = |target: usize| {
        body.iter()
            .position(|(p, _)| *p == target)
            .unwrap_or(body.len())
    };

    let mut cells: Vec<LoopCell> = seed_cells;
    let mut preds: std::collections::HashMap<u16, (CmpKind, Src, Src)> =
        std::collections::HashMap::new();
    let mut i = 0;
    while i < body.len() {
        if let (_pc, Op::JumpIfFalse { cond, target }) = body[i] {
            let (cmp, ca, cb) = *preds
                .get(&cond)
                .expect("loop fabric: JumpIfFalse condition is not a comparison");
            // The JumpIfFalse target is the merge (if-only) or the else start (if/else, when the
            // op just before it is the then→merge `Jump`). PC-resolved so TRAILING code after the
            // branch is processed too (the cursor resumes at the merge).
            let target_idx = idx_of_pc(target);
            let (then_seg, else_seg, merge_idx) =
                if target_idx > i + 1 && matches!(body[target_idx - 1].1, Op::Jump { .. }) {
                    let m = match body[target_idx - 1].1 {
                        Op::Jump { target } => target,
                        _ => unreachable!(),
                    };
                    (
                        &body[i + 1..target_idx - 1],
                        &body[target_idx..idx_of_pc(m)],
                        idx_of_pc(m),
                    )
                } else {
                    (&body[i + 1..target_idx], &body[0..0], target_idx)
                };
            let entry = reg.clone();
            let mut then_reg = entry.clone();
            exec_ops(
                &ops_of(then_seg),
                &mut then_reg,
                &mut cells,
                &mut preds,
                consts,
            );
            let mut else_reg = entry;
            exec_ops(
                &ops_of(else_seg),
                &mut else_reg,
                &mut cells,
                &mut preds,
                consts,
            );
            for &r in carried {
                let t = then_reg[&r];
                let e = else_reg[&r];
                if t != e {
                    let k = cells.len();
                    cells.push(LoopCell::Select { cmp, ca, cb, t, e });
                    reg.insert(r, Src::Val(k));
                } else {
                    reg.insert(r, t);
                }
            }
            i = merge_idx;
        } else {
            exec_ops(&[body[i].1], reg, &mut cells, &mut preds, consts);
            i += 1;
        }
    }
    cells
}

// ===========================================================================================
// DECISION FABRIC — a branchy straight-line function (no loop) if-CONVERTED to a pure select-DAG.
//
// The HFT strategy's DECISION — given the market-derived scalars (microprice, fair, edge, size,
// position, risk caps) — is nested `If/Otherwise/Set` over mutable state. That whole decision is a
// COMBINATIONAL datapath: every branch becomes a predicated `Select` (mux) tile, so control is data
// and the answer settles in ONE clock — no program counter, no fetch, no dispatch, no loop counter.
// This is the fetch/decode-free "fast path" the soft-core's 866 fetch-decode-execute cycles pay for.
//
// It reuses the CGRA's `CmpKind`/`LoopCell`/`exec_ops`, but adds RECURSIVE if-conversion so NESTED
// branches (the loop-body converter's "future work") lower too — an `if` inside an `if` becomes a
// `Select` whose arms are themselves `Select`s.
// ===========================================================================================

/// A combinational decision datapath: SSA `LoopCell`s (arith + predicated select) over the function
/// parameters (`Src::In`), plus the `Src` that drives the single output.
#[derive(Clone, Debug)]
pub struct DecisionFabric {
    pub n_in: usize,
    pub cells: Vec<LoopCell>,
    pub result: Src,
    /// A branch in the captured body jumps PAST the first `Return` (an early-return diamond /
    /// multi-return match). The fabric is then the FIRST-RETURN PATH only — sound when the
    /// escaping arm is statically dead (diff-verified corpora prove theirs ≡VM), and a
    /// REFUSAL trigger for tolerance-verified consumers (the analog lane), which cannot
    /// detect a dropped branch by diffing.
    pub escaped_first_return: bool,
}

/// Recursively if-convert a straight-line-with-branches segment into `cells`, merging every carried
/// register at each branch join with a `Select`. Nested branches recurse; `preds` carries pending
/// comparisons. Mutates `reg` to the post-segment state.
fn dec_convert(
    body: &[(usize, crate::vm::Op)],
    reg: &mut std::collections::HashMap<u16, Src>,
    cells: &mut Vec<LoopCell>,
    preds: &mut std::collections::HashMap<u16, (CmpKind, Src, Src)>,
    carried: &[u16],
    consts: &[crate::vm::Constant],
) {
    use crate::vm::Op;
    let idx_of_pc = |t: usize| body.iter().position(|(p, _)| *p == t).unwrap_or(body.len());
    let mut i = 0;
    while i < body.len() {
        match body[i].1 {
            Op::JumpIfFalse { cond, target } => {
                // A comparison condition folds its (cmp, a, b) directly; a NON-comparison condition (a Bool
                // VALUE — from `Not`, a Bool literal, or a Bool parameter) branches on `cond != 0`, exactly
                // the VM's `is_truthy` test. So control on a computed Bool is data on the fabric too.
                let (cmp, ca, cb) = preds.get(&cond).copied().unwrap_or_else(|| {
                    (
                        CmpKind::Ne,
                        reg.get(&cond).copied().unwrap_or(Src::In(cond as usize)),
                        Src::Const(0),
                    )
                });
                let target_idx = idx_of_pc(target);
                let (then_seg, else_seg, merge_idx) =
                    if target_idx > i + 1 && matches!(body[target_idx - 1].1, Op::Jump { .. }) {
                        let m = match body[target_idx - 1].1 {
                            Op::Jump { target } => target,
                            _ => unreachable!(),
                        };
                        (
                            &body[i + 1..target_idx - 1],
                            &body[target_idx..idx_of_pc(m)],
                            idx_of_pc(m),
                        )
                    } else {
                        (&body[i + 1..target_idx], &body[0..0], target_idx)
                    };
                let entry = reg.clone();
                let mut then_reg = entry.clone();
                dec_convert(then_seg, &mut then_reg, cells, preds, carried, consts);
                let mut else_reg = entry.clone();
                dec_convert(else_seg, &mut else_reg, cells, preds, carried, consts);
                for &r in carried {
                    // Each arm began as a clone of the entry state, so a register an arm did not
                    // write still holds its entry value; a register present in one arm map but not
                    // the other is defined only on that path (an arm-local variable, dead after the
                    // join and later pruned). The predicated Select fires only when the two paths
                    // genuinely disagree.
                    match (then_reg.get(&r).copied(), else_reg.get(&r).copied()) {
                        (Some(t), Some(e)) if t != e => {
                            let k = cells.len();
                            cells.push(LoopCell::Select { cmp, ca, cb, t, e });
                            reg.insert(r, Src::Val(k));
                        }
                        (Some(v), Some(_)) => {
                            reg.insert(r, v);
                        }
                        (Some(v), None) | (None, Some(v)) => {
                            reg.insert(r, v);
                        }
                        (None, None) => {}
                    }
                }
                i = merge_idx;
            }
            // `JumpIfTrue` — the MIRROR of `JumpIfFalse`, emitted by short-circuit `or`/`and`, which
            // materialise a Bool TEMP via control flow (`R = (a==0) or (b==0)`) that a later `If` then
            // branches on. Here the jump is taken when the condition is TRUE, so the TARGET segment is the
            // true-arm and the FALL-THROUGH is the false-arm (the two arms swap vs `JumpIfFalse`). Nested
            // short-circuits recurse; the merged Bool temp then drives the real `If` as a value condition.
            Op::JumpIfTrue { cond, target } => {
                let (cmp, ca, cb) = preds.get(&cond).copied().unwrap_or_else(|| {
                    (
                        CmpKind::Ne,
                        reg.get(&cond).copied().unwrap_or(Src::In(cond as usize)),
                        Src::Const(0),
                    )
                });
                let target_idx = idx_of_pc(target);
                // Fall-through (cond FALSE) is `body[i+1..]`; if it ends in a `Jump` just before the target,
                // it is a two-armed diamond and the target is the TRUE arm. Otherwise the target is the
                // merge and the true-arm is empty (a bare forward skip).
                let (false_seg, true_seg, merge_idx) =
                    if target_idx > i + 1 && matches!(body[target_idx - 1].1, Op::Jump { .. }) {
                        let m = match body[target_idx - 1].1 {
                            Op::Jump { target } => target,
                            _ => unreachable!(),
                        };
                        (
                            &body[i + 1..target_idx - 1],
                            &body[target_idx..idx_of_pc(m)],
                            idx_of_pc(m),
                        )
                    } else {
                        (&body[i + 1..target_idx], &body[0..0], target_idx)
                    };
                let entry = reg.clone();
                let mut true_reg = entry.clone();
                dec_convert(true_seg, &mut true_reg, cells, preds, carried, consts);
                let mut false_reg = entry.clone();
                dec_convert(false_seg, &mut false_reg, cells, preds, carried, consts);
                for &r in carried {
                    // `t` = the TRUE-arm value, `e` = the FALSE-arm value — so the Select still fires
                    // `cmp ? t : e`, identical polarity to `JumpIfFalse`, only the segments are swapped.
                    match (true_reg.get(&r).copied(), false_reg.get(&r).copied()) {
                        (Some(t), Some(e)) if t != e => {
                            let k = cells.len();
                            cells.push(LoopCell::Select { cmp, ca, cb, t, e });
                            reg.insert(r, Src::Val(k));
                        }
                        (Some(v), Some(_)) => {
                            reg.insert(r, v);
                        }
                        (Some(v), None) | (None, Some(v)) => {
                            reg.insert(r, v);
                        }
                        (None, None) => {}
                    }
                }
                i = merge_idx;
            }
            Op::Jump { .. } | Op::Return { .. } | Op::ReturnNothing => {
                i += 1;
            }
            _ => {
                let start = i;
                while i < body.len()
                    && !matches!(
                        body[i].1,
                        Op::JumpIfFalse { .. }
                            | Op::JumpIfTrue { .. }
                            | Op::Jump { .. }
                            | Op::Return { .. }
                            | Op::ReturnNothing
                    )
                {
                    i += 1;
                }
                let run: Vec<Op> = body[start..i].iter().map(|(_, o)| *o).collect();
                exec_ops(&run, reg, cells, preds, consts);
            }
        }
    }
}

/// The op mnemonics a function's decision body USES (entry `..` first `Return`) — the SAME segment
/// [`decision_fabric_from_source`] lowers. This is the ground truth for "which ops a fabric built from this
/// function actually exercises", so a coverage-corpus program's realized-op claim is welded to a program
/// that genuinely contains the op (not merely declared).
pub fn function_op_names(src: &str, fn_name: &str) -> Vec<&'static str> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _pol) = parsed.expect("parse failed");
        // Inspect the same typed compilation that decision_fabric_from_source lowers.
        // The registry also supplies the fixed-capacity sequence plan.
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compile failed");
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found"));
        let mut names = Vec::new();
        for op in &program.code[f.entry_pc..] {
            names.push(op.name());
            if matches!(
                op,
                crate::vm::Op::Return { .. } | crate::vm::Op::ReturnNothing
            ) {
                break;
            }
        }
        names
    })
}

/// Like [`function_op_names`], but scans the WHOLE compiled program — all functions plus the module-init
/// prologue. Used to weld coverage for a compiler-emitted op that a user function never directly contains:
/// `NewCrdt` is emitted in a `Shared` struct's default-fill, which runs in global struct-init, not the
/// user function's own op stream. So `function_op_names(f)` cannot surface it; this can.
pub fn whole_program_op_names(src: &str) -> Vec<&'static str> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _pol) = parsed.expect("parse failed");
        // `compile_with_types` (not plain `compile`): the discovery-pass type registry resolves the `Shared`
        // struct CRDT field types, which is what makes the default-fill emit `NewCrdt` at all.
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compile failed");
        program.code.iter().map(|op| op.name()).collect()
    })
}

/// **The Futamura P1 lowering for a branchy decision.** Compile `src`, take function `fn_name`'s
/// straight-line-with-branches body (entry `..` `Return`), and if-convert it to a combinational
/// select-DAG: parameters become circuit inputs, every `If` a `Select`. Panics on a shape outside
/// the compare/arith/branch family (the honest boundary — loops map to a CGRA, division to a
/// separate functional unit).
pub fn decision_fabric_from_source(src: &str, fn_name: &str) -> DecisionFabric {
    use crate::vm::Op;
    use std::collections::HashMap;

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _pol) = parsed.expect("parse failed");
        // `compile_with_types` (not plain `compile`): only the discovery-pass type registry resolves the
        // full struct field LAYOUT into `program.struct_types` — the source of the field name→slot map.
        let program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compile failed");
        // struct field-count + (type,field)→slot maps, register-type reset, whole-program code for `Call`
        // inlining, collection-length + output-tape resets — the shared per-program fabric context.
        setup_program_thread_locals(&program);
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found"));

        // (pc, op) body from entry to the FIRST Return (inclusive); capture the returned register.
        let mut body: Vec<(usize, Op)> = Vec::new();
        let mut ret_src: Option<u16> = None;
        for (off, op) in program.code[f.entry_pc..].iter().enumerate() {
            if let Op::Return { src } = op {
                ret_src = Some(*src);
                break;
            }
            if matches!(op, Op::ReturnNothing) {
                break;
            }
            body.push((f.entry_pc + off, *op));
        }
        let ret_src = ret_src.expect("decision fabric: function does not Return a value");

        // Record the early-return-diamond shape (`If …: Return X. … Return Y.`): a jump
        // escaping the captured body means the fabric covers the FIRST-RETURN PATH only.
        // Diff-verified consumers prove their corpora ≡VM anyway; tolerance-verified
        // consumers (the analog lane) refuse on this flag.
        let body_end = f.entry_pc + body.len();
        let escaped_first_return = body.iter().any(|(_, op)| {
            let target = match op {
                Op::Jump { target } => Some(*target as usize),
                Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                    Some(*target as usize)
                }
                _ => None,
            };
            target.is_some_and(|t| t > body_end)
        });

        let carried = carried_regs(&body);

        let mut reg: HashMap<u16, Src> = HashMap::new();
        for p in 0..f.param_count {
            reg.insert(p, Src::In(p as usize));
            // A `Float`-typed parameter carries an f64 BIT-PATTERN in its i64 slot, so mark it float —
            // this routes its `+ - * /` (and `sqrt`) through the float functional units. Unlocks the float
            // lane over PARAMETERS (a battery of float inputs to one datapath), not just float constants.
            if matches!(
                f.param_kinds.get(p as usize),
                Some(Some(crate::vm::ParamKind::Scalar(
                    crate::vm::SlotKind::Float
                )))
            ) {
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().insert(p);
                });
            }
        }
        let mut cells: Vec<LoopCell> = Vec::new();
        let mut preds: HashMap<u16, (CmpKind, Src, Src)> = HashMap::new();
        dec_convert(
            &body,
            &mut reg,
            &mut cells,
            &mut preds,
            &carried,
            &program.constants,
        );

        let result = *reg.get(&ret_src).unwrap_or_else(|| {
            panic!("decision fabric: returned register R{ret_src} is unwritten")
        });
        let (cells, result) = prune_dead_cells(&cells, result);
        DecisionFabric {
            n_in: f.param_count as usize,
            cells,
            result,
            escaped_first_return,
        }
    })
}

/// Drop cells not reachable from the output and compact the SSA numbering — if-conversion can leave
/// an intermediate `Select` dead once an outer branch re-merges the same register. Keeps the emitted
/// circuit tight and lint-clean (no unused wires) without changing what it computes.
fn prune_dead_cells(cells: &[LoopCell], result: Src) -> (Vec<LoopCell>, Src) {
    fn mark(s: Src, stack: &mut Vec<usize>) {
        if let Src::Val(k) = s {
            stack.push(k);
        }
    }
    let mut live = vec![false; cells.len()];
    let mut stack = Vec::new();
    mark(result, &mut stack);
    // Heap WRITES/ALLOCS are SIDE-EFFECTING: their effect flows through the heap, not through an SSA value,
    // so the dataflow reachability above cannot see it. Force them live (and thus their operands too) — a
    // dropped HeapWrite would silently leave a collection unpopulated.
    for (k, c) in cells.iter().enumerate() {
        if matches!(c, LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. }) {
            stack.push(k);
        }
    }
    while let Some(k) = stack.pop() {
        if live[k] {
            continue;
        }
        live[k] = true;
        match cells[k] {
            LoopCell::Arith { a, b, .. } | LoopCell::VectorArith { a, b, .. } => {
                mark(a, &mut stack);
                mark(b, &mut stack);
            }
            LoopCell::Func { a, b, .. } => {
                mark(a, &mut stack);
                mark(b, &mut stack);
            }
            LoopCell::MemRead { addr, .. } | LoopCell::HeapRead { addr, .. } => {
                mark(addr, &mut stack);
            }
            LoopCell::HeapWrite { addr, val } => {
                mark(addr, &mut stack);
                mark(val, &mut stack);
            }
            LoopCell::HeapAlloc { size } => {
                mark(size, &mut stack);
            }
            LoopCell::Select { ca, cb, t, e, .. } => {
                mark(ca, &mut stack);
                mark(cb, &mut stack);
                mark(t, &mut stack);
                mark(e, &mut stack);
            }
        }
    }
    let mut remap = vec![usize::MAX; cells.len()];
    let mut kept: Vec<LoopCell> = Vec::new();
    for (k, c) in cells.iter().enumerate() {
        if live[k] {
            remap[k] = kept.len();
            kept.push(*c);
        }
    }
    fn relabel(s: Src, remap: &[usize]) -> Src {
        match s {
            Src::Val(k) => Src::Val(remap[k]),
            other => other,
        }
    }
    for c in kept.iter_mut() {
        match c {
            LoopCell::Arith { a, b, .. } | LoopCell::VectorArith { a, b, .. } => {
                *a = relabel(*a, &remap);
                *b = relabel(*b, &remap);
            }
            LoopCell::Func { a, b, .. } => {
                *a = relabel(*a, &remap);
                *b = relabel(*b, &remap);
            }
            LoopCell::MemRead { addr, .. } | LoopCell::HeapRead { addr, .. } => {
                *addr = relabel(*addr, &remap);
            }
            LoopCell::HeapWrite { addr, val } => {
                *addr = relabel(*addr, &remap);
                *val = relabel(*val, &remap);
            }
            LoopCell::HeapAlloc { size } => {
                *size = relabel(*size, &remap);
            }
            LoopCell::Select { ca, cb, t, e, .. } => {
                *ca = relabel(*ca, &remap);
                *cb = relabel(*cb, &remap);
                *t = relabel(*t, &remap);
                *e = relabel(*e, &remap);
            }
        }
    }
    (kept, relabel(result, &remap))
}

/// Evaluate the decision DAG (arith + select, 64-bit wrapping) — the reference the emitted circuit
/// is checked against, bit-identical to the VM's `Int`.
pub fn eval_decision_fabric(df: &DecisionFabric, inputs: &[i64]) -> i64 {
    let mut vals: Vec<i64> = Vec::with_capacity(df.cells.len());
    for c in &df.cells {
        let v = match c {
            LoopCell::Arith { op, a, b } => op.eval(a.eval(inputs, &vals), b.eval(inputs, &vals)),
            LoopCell::VectorArith { op, ty, a, b } => op.eval(
                *ty,
                a.eval(inputs, &vals) as u64,
                b.eval(inputs, &vals) as u64,
            ) as i64,
            LoopCell::Func { unit, a, b } => {
                unit.eval(a.eval(inputs, &vals), b.eval(inputs, &vals))
            }
            LoopCell::MemRead { .. }
            | LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead reached an evaluator that does not thread the iterated sequence — \
                 a sequence loop must be evaluated via eval_loop_fabric_seq"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                if cmp.eval(ca.eval(inputs, &vals), cb.eval(inputs, &vals)) {
                    t.eval(inputs, &vals)
                } else {
                    e.eval(inputs, &vals)
                }
            }
        };
        vals.push(v);
    }
    df.result.eval(inputs, &vals)
}

/// Evaluate a decision fabric that reads the shared HEAP (the state plane). `heap` mirrors the VM's exact
/// layout — a collection at pointer `p` is `heap[p] = length` then its elements at `heap[p+1..=p+len]`
/// (1-based) — so this evaluation is bit-identical to the VM's `Int`. A [`LoopCell::HeapRead`] reads
/// `heap[addr]`; `length of xs` is `HeapRead(p)`, `item i of xs` is `HeapRead(p + i)`. Reads are pure given
/// the input heap, so the fabric stays a combinational DAG (heap WRITES/ALLOC are the next increment).
pub fn eval_decision_fabric_heap(df: &DecisionFabric, inputs: &[i64], heap: &mut Vec<i64>) -> i64 {
    let vals = eval_cells(&df.cells, inputs, heap);
    df.result.eval(inputs, &vals)
}

/// Evaluate a straight-line SSA cell list in order into `vals` (one value per cell), threading heap side
/// effects (a `HeapRead` sees every prior `HeapWrite`/`HeapAlloc`) — the imperative reduction the VM does.
/// Returns the per-cell value array so any `Src` (a fabric result, or a `Show`'d output) can then be read.
fn eval_cells(cells: &[LoopCell], inputs: &[i64], heap: &mut Vec<i64>) -> Vec<i64> {
    let mut vals: Vec<i64> = Vec::with_capacity(cells.len());
    for c in cells {
        let v = match c {
            LoopCell::Arith { op, a, b } => op.eval(a.eval(inputs, &vals), b.eval(inputs, &vals)),
            LoopCell::VectorArith { op, ty, a, b } => op.eval(
                *ty,
                a.eval(inputs, &vals) as u64,
                b.eval(inputs, &vals) as u64,
            ) as i64,
            LoopCell::Func { unit, a, b } => {
                unit.eval(a.eval(inputs, &vals), b.eval(inputs, &vals))
            }
            LoopCell::HeapRead { addr } => {
                let a = addr.eval(inputs, &vals);
                heap.get(a as usize).copied().unwrap_or(0)
            }
            LoopCell::HeapWrite { addr, val } => {
                let a = addr.eval(inputs, &vals) as usize;
                let v = val.eval(inputs, &vals);
                if a >= heap.len() {
                    heap.resize(a + 1, 0);
                }
                heap[a] = v;
                v
            }
            LoopCell::HeapAlloc { size } => {
                let s = size.eval(inputs, &vals).max(0) as usize;
                let ptr = heap.len() as i64;
                heap.resize(heap.len() + s, 0);
                ptr
            }
            LoopCell::MemRead { .. } => panic!(
                "MemRead (loop-sequence read) has no meaning in a straight-line heap eval — \
                 sequence loops evaluate via eval_loop_fabric_seq"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                if cmp.eval(ca.eval(inputs, &vals), cb.eval(inputs, &vals)) {
                    t.eval(inputs, &vals)
                } else {
                    e.eval(inputs, &vals)
                }
            }
        };
        vals.push(v);
    }
    vals
}

/// The faithful MODEL of the whole-program SEQUENCED fabric ([`emit_seq_program_verilog`]): walk blocks by a PC,
/// evaluating each block's cells in order (threading ONE shared heap = the state plane) and taking the terminator
/// (Jump/Branch/Return), exactly as the block-walk FSM does in RTL. This is the sequenced fabric's own oracle —
/// distinct from [`run_program_cfg_on_grid`](crate::codegen_sva::fabric_grid::run_program_cfg_on_grid) (which models the GRID realization and cannot place a looping heap
/// body). Bit-identical to the VM's `Int` for a lowered program. `budget` bounds the block walk (a loop that
/// never returns panics, catching a lowering back-edge bug — never a silent hang).
///
/// [`run_program_cfg_on_grid`]: crate::codegen_sva::fabric_grid::run_program_cfg_on_grid
pub fn eval_seq_program(pf: &ProgramCfgFabric, params: &[i64]) -> i64 {
    use std::collections::HashMap;
    let mut file: HashMap<u16, i64> = (0..pf.n_params)
        .map(|p| (p as u16, params.get(p).copied().unwrap_or(0)))
        .collect();
    let mut heap: Vec<i64> = Vec::new();
    let mut blk = 0usize;
    for _ in 0..10_000_000 {
        let b = &pf.blocks[blk];
        let inputs: Vec<i64> = b
            .inputs
            .iter()
            .map(|r| *file.get(r).unwrap_or(&0))
            .collect();
        let vals = eval_cells(&b.body, &inputs, &mut heap);
        for (r, s) in &b.writes {
            file.insert(*r, s.eval(&inputs, &vals));
        }
        match &b.term {
            Term::Jump(t) => blk = *t,
            Term::Branch {
                cond,
                then_blk,
                else_blk,
            } => {
                blk = if cond.eval(&inputs, &vals) != 0 {
                    *then_blk
                } else {
                    *else_blk
                }
            }
            Term::Return(s) => return s.eval(&inputs, &vals),
        }
    }
    panic!("eval_seq_program: block walk did not return within the step budget (a back-edge slipped through?)")
}

/// The whole-program fabric: a straight-line `Main`'s cells plus the ordered `Src`s it `Show`s. Evaluating
/// it yields the program's output TAPE, compared against the VM's stdout — this is what covers `Show` and
/// validates the whole fabric end-to-end (calls inlined, collections/structs/enums on the shared heap).
#[derive(Clone, Debug)]
pub struct WholeProgramFabric {
    pub cells: Vec<LoopCell>,
    /// Each `Show`n value's driving `Src`, whether it renders as TEXT (a char-list) vs an Int, and the
    /// policy GRANT guard (`Some(0/1 Src)`) in force at that `Show` — `eval_whole_program` halts the tape at
    /// the first guard that evaluates false (a denied `Check`). `None` = ungated (always prints).
    pub outputs: Vec<(Src, bool, Option<Src>)>,
}

/// Lower a program's `Main` (`code[0..]` up to its terminating `Halt`) to a whole-program fabric.
/// STRAIGHT-LINE only: a top-level branch would make a `Show` conditional, which the combinational output
/// tape cannot honor, so it is refused loudly. Calls inside Main inline normally (their branches if-convert).
pub fn whole_program_fabric_from_source(src: &str) -> WholeProgramFabric {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, pol) = parsed.expect("parse failed");
        let mut program = crate::vm::Compiler::compile_with_types(stmts, interner, Some(types))
            .expect("compile failed");
        // The `PolicyRegistry` lives in the PARSE result, not the `CompiledProgram`, so a `Check`'s
        // condition is extracted to the owned key form HERE (where the interner + registry are in scope)
        // and threaded into the build; without it a `CheckPolicy` would have no condition to model.
        let policies = extract_policy_conditions(&pol, interner, &program);
        // Coalesce the VM compiler's register-move cascades before spatial lowering — the fabric places
        // final RTL with no downstream optimizer. Applied on the SOURCE→fabric path only (not the public
        // `whole_program_fabric_from_program`, whose hand-built-bytecode tests assert exact op streams).
        // Semantics-preserving IR→IR (peephole_soundness_lock proves `== VM`).
        crate::vm::peephole::peephole(&mut program);
        whole_program_fabric_from_program_with_policies(&program, &policies)
    })
}

/// Extract the SINGLE-FIELD policy conditions the fabric can model from a program's parse-time
/// `PolicyRegistry` into the owned key form the `CheckPolicy` arm looks up: `(subject type-name const idx,
/// predicate/action symbol index, is_capability) -> (subject field slot, expected value)`. A condition is
/// modeled iff it is `FieldBool`, or `FieldEquals` on a Bool/Int field (expected value type-directed by the
/// field's declared `BoundaryType`, matching the VM's type-directed compare); every compound / text /
/// object-cross-field / predicate-indirection condition is DROPPED, so the `CheckPolicy` arm panics on it
/// rather than silently granting (a wrong grant is a security hole — rule 8).
fn extract_policy_conditions(
    registry: &crate::analysis::PolicyRegistry,
    interner: &crate::intern::Interner,
    program: &crate::vm::CompiledProgram,
) -> Vec<((u32, u32, bool), PolicyGrant)> {
    let mut out = Vec::new();
    for (subj_type, preds) in registry.iter_predicates() {
        for pred in preds {
            push_policy_condition(
                &mut out,
                interner,
                program,
                *subj_type,
                pred.predicate_name,
                false,
                &pred.condition,
            );
        }
    }
    for (subj_type, caps) in registry.iter_capabilities() {
        for cap in caps {
            push_policy_condition(
                &mut out,
                interner,
                program,
                *subj_type,
                cap.action,
                true,
                &cap.condition,
            );
        }
    }
    out
}

/// Resolve one policy condition to its modeled `(field slot, expected i64)` and push an entry for EVERY
/// const index that names the subject type (a type name can be interned at more than one index — mirror
/// `STRUCT_FIELD_POS`), so whichever const `REG_TYPE` holds for the subject register resolves. Bool→0/1,
/// Int→the value, type-directed by the field's declared `BoundaryType` exactly as the VM's `match field_val`
/// is. Anything unmodeled (text/float/compound/indirect) returns without pushing — the arm then refuses.
#[allow(clippy::too_many_arguments)]
fn push_policy_condition(
    out: &mut Vec<((u32, u32, bool), PolicyGrant)>,
    interner: &crate::intern::Interner,
    program: &crate::vm::CompiledProgram,
    subj_type: crate::intern::Symbol,
    predicate: crate::intern::Symbol,
    is_capability: bool,
    cond: &crate::analysis::PolicyCondition,
) {
    use crate::analysis::PolicyCondition;
    use crate::vm::BoundaryType;
    let tname = interner.resolve(subj_type);
    let sd = match program.struct_types.iter().find(|s| s.name == tname) {
        Some(sd) => sd,
        None => return,
    };
    // Resolve a field NAME symbol to its 0-based slot in this struct type.
    let slot_of = |field: crate::intern::Symbol| -> Option<usize> {
        let fname = interner.resolve(field);
        sd.fields.iter().position(|(n, _)| n == fname)
    };
    let grant = match cond {
        // `field` (a Bool) is implicitly `== value`. (Not currently emitted by the parser — every bool
        // condition arrives as `FieldEquals … equals true` — but modeled for completeness.)
        PolicyCondition::FieldBool { field, value } => match slot_of(*field) {
            Some(slot) => PolicyGrant::IntEq {
                slot,
                expected: i64::from(*value),
            },
            None => return,
        },
        PolicyCondition::FieldEquals {
            field,
            value,
            is_string_literal,
        } => {
            let slot = match slot_of(*field) {
                Some(s) => s,
                None => return,
            };
            let expected = interner.resolve(*value);
            match &sd.fields[slot].1 {
                // The VM refuses a quoted string against a Bool/Int field (`if *is_string_literal { false }`)
                // and denies any literal it can't parse to that type (`.unwrap_or(false)`) — both always-deny.
                BoundaryType::Bool => {
                    if *is_string_literal {
                        PolicyGrant::AlwaysDeny
                    } else {
                        match expected.parse::<bool>() {
                            Ok(b) => PolicyGrant::IntEq {
                                slot,
                                expected: i64::from(b),
                            },
                            Err(_) => PolicyGrant::AlwaysDeny,
                        }
                    }
                }
                BoundaryType::Int => {
                    if *is_string_literal {
                        PolicyGrant::AlwaysDeny
                    } else {
                        match expected.parse::<i64>() {
                            Ok(n) => PolicyGrant::IntEq { slot, expected: n },
                            Err(_) => PolicyGrant::AlwaysDeny,
                        }
                    }
                }
                // A Text field ALWAYS string-compares on the VM (`s.as_str() == expected`), regardless of
                // `is_string_literal` — a bounded char-list equality on the fabric.
                BoundaryType::Text => PolicyGrant::TextEq {
                    slot,
                    expected: expected.as_bytes().to_vec(),
                },
                _ => return, // Float / handle-typed field equality — a follow-on
            }
        }
        _ => return, // And / Or / Predicate / ObjectFieldEquals / SubjectFieldEqualsObjectField
    };
    for (i, c) in program.constants.iter().enumerate() {
        if matches!(c, crate::vm::Constant::Text(n) if n == tname) {
            out.push((
                (i as u32, predicate.index() as u32, is_capability),
                grant.clone(),
            ));
        }
    }
}

/// Lower an already-COMPILED program to a whole-program fabric. Shared by the source path and by the
/// HAND-AUTHORED-bytecode witnesses (`codegen_sva/rtl.rs`'s precedent): the reversible ALU ops
/// (`RevXor`/`RevNeg`/`RevAdd`/`RevSub`) and the scheduler/CRDT-internal ops (`ChanClose`/`CrdtJoin`) that
/// no LOGOS source emits are still real ISA ops the soft-core executes, so their fabric tile is proven by
/// building a raw `CompiledProgram` (op + `Show`) and differencing the fabric tape against the VM oracle
/// (`Vm::new(&prog).run(); vm.output()`) — never a hand-coded golden. Hand-authored bytecode carries no
/// `PolicyRegistry`, so it runs with NO modeled conditions (a `CheckPolicy` in it would refuse loudly).
pub fn whole_program_fabric_from_program(
    program: &crate::vm::CompiledProgram,
) -> WholeProgramFabric {
    whole_program_fabric_from_program_with_policies(program, &[])
}

/// [`whole_program_fabric_from_program`] plus the program's extracted policy conditions (see
/// [`extract_policy_conditions`]). `setup_program_thread_locals` clears `POLICY_CONDITIONS`, then these are
/// installed so the `CheckPolicy` arm can compile each `Check`'s grant predicate.
fn whole_program_fabric_from_program_with_policies(
    program: &crate::vm::CompiledProgram,
    policies: &[((u32, u32, bool), PolicyGrant)],
) -> WholeProgramFabric {
    use crate::vm::Op;
    use std::collections::HashMap;
    setup_program_thread_locals(program);
    POLICY_CONDITIONS.with(|m| {
        let mut m = m.borrow_mut();
        for (k, v) in policies {
            m.insert(*k, v.clone());
        }
    });

    let mut body: Vec<Op> = Vec::new();
    for op in &program.code {
        match op {
            Op::Halt | Op::ReturnNothing => break,
            Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => panic!(
                "whole-program fabric: Main has a branch — a conditional Show cannot ride the \
                 combinational output tape (straight-line Main only)"
            ),
            _ => body.push(*op),
        }
    }
    let mut reg: HashMap<u16, Src> = HashMap::new();
    let mut cells: Vec<LoopCell> = Vec::new();
    let mut preds: HashMap<u16, (CmpKind, Src, Src)> = HashMap::new();
    exec_ops(&body, &mut reg, &mut cells, &mut preds, &program.constants);
    let outputs = SHOW_OUTPUTS.with(|o| o.borrow().clone());
    WholeProgramFabric { cells, outputs }
}

/// Run a whole-program fabric to its output tape — each `Show`n value RENDERED as the VM would print it: a
/// text char-list decoded to its string, an Int as decimal. The strings are compared to the VM's stdout lines.
pub fn eval_whole_program(wpf: &WholeProgramFabric) -> Vec<String> {
    let mut heap: Vec<i64> = Vec::new();
    let vals = eval_cells(&wpf.cells, &[], &mut heap);
    let mut out = Vec::with_capacity(wpf.outputs.len());
    for (s, is_text, guard) in &wpf.outputs {
        // A DENIED policy `Check` halts the program (VM returns `Err`), so this `Show` and every one after it
        // is suppressed. Guards are monotone-ANDed forward, so the first false one is the halt point.
        if let Some(g) = guard {
            if g.eval(&[], &vals) == 0 {
                break;
            }
        }
        let v = s.eval(&[], &vals);
        out.push(if *is_text {
            let ptr = v.max(0) as usize;
            let len = heap.get(ptr).copied().unwrap_or(0).max(0) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|i| heap.get(ptr + 1 + i).copied().unwrap_or(0) as u8)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            format!("{v}")
        });
    }
    out
}

/// Emit the decision as a fixed COMBINATIONAL dataflow circuit — one `wire` per SSA cell, arith and
/// predicated-select tiles, no clock. 64-bit so its wrapping arithmetic is bit-identical to the VM.
pub fn emit_decision_fabric_verilog(name: &str, df: &DecisionFabric) -> String {
    let name_in = |k: usize| format!("in{k}");
    let name_val = |k: usize| format!("v{k}");
    let inputs = (0..df.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut body = String::new();
    for (i, c) in df.cells.iter().enumerate() {
        let rhs = match c {
            LoopCell::Arith { op, a, b } => {
                op.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::VectorArith { op, ty, a, b } => {
                op.verilog(*ty, &src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::Func { unit, a, b } => unit.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val)),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead RTL emission (a scratchpad-read tile driven by the induction counter) is the \
                 P2 scratchpad follow-on; the sequence-loop LOWERING is proven via eval_loop_fabric_seq"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => format!(
                "($signed({}) {} $signed({})) ? {} : {}",
                src_named(ca, &name_in, &name_val),
                cmp.verilog(),
                src_named(cb, &name_in, &name_val),
                src_named(t, &name_in, &name_val),
                src_named(e, &name_in, &name_val),
            ),
        };
        body.push_str(&format!("  wire [63:0] v{i} = {rhs};\n"));
    }
    format!(
        "// decision_{name} — the HFT decision if-CONVERTED to a combinational select-DAG.\n\
         // Every `If` is a predicated Select (mux) tile; the answer settles in ONE clock — no PC,\n\
         // no fetch, no dispatch, no loop counter. Futamura P1: the decision IS the circuit.\n\
         // Lowered from the REAL compiler; 64-bit, so it wraps identically to the VM's Int; proven\n\
         // == the running VM by iverilog.\n\
         module decision_{name}({inputs}, output [63:0] result);\n\
         {body}  assign result = {res};\n\
         endmodule\n",
        res = src_named(&df.result, &name_in, &name_val),
    )
}

/// A self-checking testbench for a COMBINATIONAL `decision_{name}` fabric: drive `n` input vectors (each
/// `n_in` 64-bit words) from `dec_{name}_in.hex`, compare the settled `result` against the VM oracle in
/// `dec_{name}_out.hex`, and print PASS/FAIL. The combinational analog of [`emit_state_plane_testbench`] — no
/// clock, one `#1` settle per vector. Lets any Func/Cmp/Arith decision tile be iverilog-exercised ≡ the VM.
pub fn emit_decision_fabric_testbench_dummy(name: &str, n_in: usize, n: usize) -> String {
    let decls = (0..n_in)
        .map(|k| format!("  reg [63:0] in{k};\n"))
        .collect::<String>();
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    let drive = (0..n_in)
        .map(|k| format!("      in{k} = oin[i * {n_in} + {k}];\n"))
        .collect::<String>();
    format!(
        r#"// tb_dec_{name} — the combinational decision fabric vs the RUNNING VM's per-vector output.
`timescale 1ns/1ps
module tb_dec_{name};
{decls}  reg [63:0] oin  [0:{in_last}];
  reg [63:0] oout [0:{last}];
  wire [63:0] result;
  integer i, errors = 0;
  decision_{name} dut({ports}.result(result));
  initial begin
    $readmemh("dec_{name}_in.hex", oin);
    $readmemh("dec_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
{drive}      #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL vec %0d: got %0d", i, $signed(result)); end
    end
    if (errors == 0) $display("PASS: {n} decision-fabric evaluations match the VM (the tile IS the circuit)");
    else $display("FAIL: %0d decision mismatches", errors);
    $finish;
  end
endmodule
"#,
        in_last = n_in * n - 1,
        last = n - 1,
    )
}

/// Emit a general SEQUENCED datapath over a `DecisionFabric`'s cells — the LATENCY-INSENSITIVE fabric. A
/// combinational `Arith`/`Select` cell settles in one step; a `Func` DIVISION cell (`DivS`/`ModS`/`FloorDivS`)
/// ISSUES the multi-cycle `shared_divider` and STALLS until its `done` (a variable-latency TILE). This is the
/// same cell stream `emit_decision_fabric_verilog` wires as a pure combinational DAG, but with the sequential
/// functional units as first-class multi-cycle tiles — so it composes with the fpu.v FPU (Phase B) and the
/// block-walk sequencer (Phase C). Proven == `eval_decision_fabric` == the VM by iverilog. (Phase A: Arith/
/// Select/division; float/pow/mem are the following increments and PANIC here as the honest boundary.)
pub fn emit_seq_datapath_verilog(name: &str, df: &DecisionFabric) -> String {
    let name_in = |k: usize| format!("in{k}");
    let name_val = |k: usize| format!("R[{k}]");
    let src = |s: &Src| src_named(s, &name_in, &name_val);
    let m = df.cells.len();
    assert!(m > 0, "a sequenced datapath needs ≥1 cell");
    let inputs = (0..df.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };

    let mut arms = String::new();
    for (i, c) in df.cells.iter().enumerate() {
        arms.push_str(&format!("      {i}: begin {} end\n", seq_cell_arm(c, &src)));
    }

    format!(
        r#"// seq_{name} — a general SEQUENCED datapath over the decision fabric's cells (LATENCY-INSENSITIVE).
// Combinational Arith/Select/IntToF/FFloor settle in one step; a Func DIVISION cell issues the multi-cycle
// shared_divider, and a FLOAT arithmetic cell issues the shared fpu.v unit (fp_addsub/mul/div/sqrt) — each a
// variable-latency TILE that stalls the datapath until its `done`. Proven == eval_decision_fabric == the VM by
// iverilog (fpu.v + shared_divider.v are the real synthesizable units).
module seq_{name}(input clk, input rst, {inputs}output [63:0] result, output done);
  localparam M = {m};
  (* ram_style="registers" *) reg [63:0] R [0:M-1];
  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
{converters}  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
    case (step)
{arms}      default: ;
    endcase
  end
{units}  always @(posedge clk) begin
    if (rst) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;                       // the start pulse fired this cycle
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done   = !running;
  assign result = {res};
endmodule
"#,
        res = src(&df.result),
        converters = I64_F64_CONVERTERS,
        units = seq_units_for(&df.cells),
    )
}

/// The `case(step)` arm body for one `LoopCell` in a sequenced datapath: set `is_comb`/`comb_y` for a
/// combinational cell (Arith/Select/IntToF/FFloor) or `ua`/`ub`/`usel` for a multi-cycle cell (division 1/2/3,
/// float 5..9). Shared by `emit_seq_datapath_verilog` and `emit_seq_block_verilog`.
fn seq_cell_arm(c: &LoopCell, src: &dyn Fn(&Src) -> String) -> String {
    match c {
        LoopCell::Arith { op, a, b } => {
            format!("is_comb=1'b1; comb_y={};", op.verilog(&src(a), &src(b)))
        }
        LoopCell::Select { cmp, ca, cb, t, e } => format!(
            "is_comb=1'b1; comb_y=($signed({}) {} $signed({})) ? {} : {};",
            src(ca),
            cmp.verilog(),
            src(cb),
            src(t),
            src(e)
        ),
        LoopCell::Func { unit, a, b } => match unit {
            FuncUnit::DivS => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd1;", src(a), src(b)),
            FuncUnit::ModS => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd2;", src(a), src(b)),
            FuncUnit::FloorDivS => {
                format!("is_comb=1'b0; ua={}; ub={}; usel=4'd3;", src(a), src(b))
            }
            FuncUnit::FAddF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd5;", src(a), src(b)),
            FuncUnit::FSubF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd6;", src(a), src(b)),
            FuncUnit::FMulF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd7;", src(a), src(b)),
            FuncUnit::FDivF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd8;", src(a), src(b)),
            FuncUnit::FSqrtF => format!("is_comb=1'b0; ua={}; ub=64'd0; usel=4'd9;", src(a)),
            FuncUnit::IntToF => format!("is_comb=1'b1; comb_y=i64_to_f64({});", src(a)),
            FuncUnit::FFloorF => format!("is_comb=1'b1; comb_y=f64_floor_i64({});", src(a)),
            // IEEE-754 float COMPARES → the shared combinational fp_cmp (usel 10..15). Operands are held stable
            // while the step runs, so the compare resolves as a 1-cycle multi-cycle op (no combinational hazard).
            FuncUnit::FLtF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd10;", src(a), src(b)),
            FuncUnit::FGtF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd11;", src(a), src(b)),
            FuncUnit::FLeF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd12;", src(a), src(b)),
            FuncUnit::FGeF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd13;", src(a), src(b)),
            FuncUnit::FEqF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd14;", src(a), src(b)),
            FuncUnit::FNeF => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd15;", src(a), src(b)),
            // integer POWER → the sequential repeat-multiply FSM (usel 4): a `b`-step wrapping-64-bit tile that
            // stalls the datapath until its `done`, exactly like the divider (a variable-latency tile).
            FuncUnit::Pow => format!("is_comb=1'b0; ua={}; ub={}; usel=4'd4;", src(a), src(b)),
            // The COMBINATIONAL tiles settle in ONE step (`is_comb`), reusing the grid's already-iverilog-proven
            // `FuncUnit::verilog()` verbatim: PopCount (bit-adder tree), MulHiU / MulOvfS (128-bit product), and
            // ApproxEqF (exact-rational tolerance). One expression, bit-identical to `FuncUnit::eval`.
            FuncUnit::PopCount | FuncUnit::MulHiU | FuncUnit::MulOvfS | FuncUnit::ApproxEqF => {
                format!("is_comb=1'b1; comb_y={};", unit.verilog(&src(a), &src(b)))
            }
        },
        other => panic!("seq datapath: {other:?} (Mem/Heap) is a state-plane follow-on"),
    }
}

/// Emit the multi-cycle functional units + result muxing for a sequenced datapath — but ONLY the units the
/// `cells` actually use (a division-only fabric gets NO fpu.v, a compare-free float fabric gets no fp_cmp, …),
/// so every fabric instantiates exactly its own tiles. Unused unit outputs are stubbed (`_y=0`, `_done=1`) so
/// the `mc_done`/`mc_y` muxes stay uniform. usel: 1/2/3 div/mod/floordiv, 5..9 fadd/fsub/fmul/fdiv/fsqrt,
/// 10..15 the six IEEE-754 compares (fp_cmp). Drives `mc_done`/`mc_y` off `usel`/`ua`/`ub`/`pulse`.
fn seq_units_for(cells: &[LoopCell]) -> String {
    let (mut div, mut addsub, mut mul, mut fdiv, mut sqrt, mut cmp, mut pow) =
        (false, false, false, false, false, false, false);
    for c in cells {
        if let LoopCell::Func { unit, .. } = c {
            match unit {
                FuncUnit::DivS | FuncUnit::ModS | FuncUnit::FloorDivS => div = true,
                FuncUnit::Pow => pow = true,
                FuncUnit::FAddF | FuncUnit::FSubF => addsub = true,
                FuncUnit::FMulF => mul = true,
                FuncUnit::FDivF => fdiv = true,
                FuncUnit::FSqrtF => sqrt = true,
                FuncUnit::FLtF
                | FuncUnit::FGtF
                | FuncUnit::FLeF
                | FuncUnit::FGeF
                | FuncUnit::FEqF
                | FuncUnit::FNeF => cmp = true,
                _ => {}
            }
        }
    }
    let mut s = String::new();
    s.push_str("  wire is_div   = (usel>=4'd1 && usel<=4'd3);\n");
    s.push_str("  wire is_mod   = (usel==4'd2);\n  wire is_floor = (usel==4'd3);\n");
    s.push_str("  wire is_pow   = (usel==4'd4);\n");
    s.push_str("  wire is_fadd  = (usel==4'd5)||(usel==4'd6);\n  wire is_fsub = (usel==4'd6);\n");
    s.push_str("  wire is_fmul  = (usel==4'd7);\n  wire is_fdiv = (usel==4'd8);\n  wire is_fsqrt = (usel==4'd9);\n");
    s.push_str("  wire is_cmp   = (usel>=4'd10);\n");
    s.push_str("  wire pulse = running && !issued && !is_comb;\n");
    if div {
        s.push_str("  wire d_done; wire [63:0] d_q, d_r;\n");
        s.push_str("  shared_divider #(.W(64)) u_div(.clk(clk), .rst(rst), .start(pulse && is_div && (ub!=64'd0)), .a(ua), .b(ub), .q(d_q), .r(d_r), .done(d_done));\n");
        s.push_str("  wire signed [63:0] d_floor_v = $signed(d_q) - (((d_r!=64'd0) && (ua[63]^ub[63])) ? 64'sd1 : 64'sd0);\n");
        s.push_str("  wire [63:0] div_y = (ub==64'd0) ? 64'd0 : is_mod ? d_r : is_floor ? d_floor_v : d_q;\n");
        s.push_str("  wire div_done_g = (ub==64'd0) ? 1'b1 : d_done;\n");
    } else {
        s.push_str("  wire [63:0] div_y = 64'd0; wire div_done_g = 1'b1;\n");
    }
    if addsub {
        s.push_str("  wire fa_done; wire [63:0] fa_y;\n  fp_addsub u_fa(.clk(clk),.rst(rst),.start(pulse && is_fadd),.a(ua),.b_in(ub),.sub(is_fsub),.y(fa_y),.done(fa_done));\n");
    } else {
        s.push_str("  wire [63:0] fa_y = 64'd0; wire fa_done = 1'b1;\n");
    }
    if mul {
        s.push_str("  wire fm_done; wire [63:0] fm_y;\n  fp_mul u_fm(.clk(clk),.rst(rst),.start(pulse && is_fmul),.a(ua),.b(ub),.y(fm_y),.done(fm_done));\n");
    } else {
        s.push_str("  wire [63:0] fm_y = 64'd0; wire fm_done = 1'b1;\n");
    }
    if fdiv {
        s.push_str("  wire fd_done; wire [63:0] fd_y;\n  fp_div u_fd(.clk(clk),.rst(rst),.start(pulse && is_fdiv),.a(ua),.b(ub),.y(fd_y),.done(fd_done));\n");
    } else {
        s.push_str("  wire [63:0] fd_y = 64'd0; wire fd_done = 1'b1;\n");
    }
    if sqrt {
        s.push_str("  wire fs_done; wire [63:0] fs_y;\n  fp_sqrt u_fs(.clk(clk),.rst(rst),.start(pulse && is_fsqrt),.a(ua),.y(fs_y),.done(fs_done));\n");
    } else {
        s.push_str("  wire [63:0] fs_y = 64'd0; wire fs_done = 1'b1;\n");
    }
    if cmp {
        s.push_str("  wire cmp_swap = (usel==4'd11)||(usel==4'd13);\n");
        s.push_str(
            "  wire [63:0] fcm_a = cmp_swap ? ub : ua; wire [63:0] fcm_b = cmp_swap ? ua : ub;\n",
        );
        s.push_str(
            "  wire fp_lt, fp_eq;\n  fp_cmp u_fcm(.a(fcm_a), .b(fcm_b), .lt(fp_lt), .eq(fp_eq));\n",
        );
        s.push_str("  wire [63:0] cmp_y = (usel==4'd10||usel==4'd11) ? {63'd0, fp_lt} : (usel==4'd12||usel==4'd13) ? {63'd0, fp_lt|fp_eq} : (usel==4'd14) ? {63'd0, fp_eq} : (usel==4'd15) ? {63'd0, ~fp_eq} : 64'd0;\n");
    } else {
        s.push_str("  wire [63:0] cmp_y = 64'd0;\n");
    }
    if pow {
        // Sequential integer POWER — a `b`-step repeat-multiply, wrapping 64-bit (bit-identical to the eval
        // `a.wrapping_pow(max(0,b) as u32)`). `pw_done` is (re-)armed on the issue `pulse` — for `b<=0` it is
        // high the same cycle with `pw_acc==1`, so the stale-done hazard cannot fire (mc_done is first sampled
        // the cycle AFTER pulse). For `b>0` it iterates `b` multiplies then raises `pw_done`.
        s.push_str("  reg [63:0] pw_acc, pw_base, pw_cnt; reg pw_busy, pw_done;\n");
        s.push_str("  always @(posedge clk) begin\n");
        s.push_str("    if (rst) begin pw_busy <= 1'b0; pw_done <= 1'b0; end\n");
        s.push_str("    else if (pulse && is_pow) begin\n");
        s.push_str("      pw_acc  <= 64'd1; pw_base <= ua;\n");
        s.push_str("      pw_cnt  <= ($signed(ub) <= 0) ? 64'd0 : ub;\n");
        s.push_str("      pw_busy <= ($signed(ub) >  0); pw_done <= ($signed(ub) <= 0);\n");
        s.push_str("    end else if (pw_busy) begin\n");
        s.push_str("      pw_acc <= pw_acc * pw_base;\n");
        s.push_str("      if (pw_cnt <= 64'd1) begin pw_busy <= 1'b0; pw_done <= 1'b1; end else pw_cnt <= pw_cnt - 64'd1;\n");
        s.push_str("    end\n  end\n");
    } else {
        s.push_str("  wire [63:0] pw_acc = 64'd0; wire pw_done = 1'b1;\n");
    }
    // A compare's fp_cmp is combinational over the held operands, so it is ready one cycle after issue (mc_done=1).
    s.push_str("  wire        mc_done = is_div ? div_done_g : is_pow ? pw_done : is_cmp ? 1'b1 : is_fadd ? fa_done : is_fmul ? fm_done : is_fdiv ? fd_done : is_fsqrt ? fs_done : 1'b1;\n");
    s.push_str("  wire [63:0] mc_y    = is_div ? div_y : is_pow ? pw_acc : is_cmp ? cmp_y : is_fadd ? fa_y : is_fmul ? fm_y : is_fdiv ? fd_y : is_fsqrt ? fs_y : 64'd0;\n");
    s
}

/// Emit a per-block SEQUENCED datapath (`seqblk_{name}`) for a `CfgBlock`: on a `start` pulse it walks the
/// block's SSA cells (combinational in one step, multi-cycle division/float via the shared units), then holds
/// `done` high with every `needed` output (each register write's `Src`, then the terminator operand) driven
/// from the settled cell/register values. The block-walk sequencer starts it, waits `done`, latches the writes,
/// and evaluates the terminator. Multi-cycle TILES are first-class here.
pub fn emit_seq_block_verilog(name: &str, block: &CfgBlock) -> String {
    emit_seq_block_verilog_sp(name, block, false)
}

/// True iff any cell is a shared-heap STATE-PLANE cell (needs the BRAM heap port).
fn cell_is_heap(c: &LoopCell) -> bool {
    matches!(
        c,
        LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. }
            | LoopCell::MemRead { .. }
    )
}

/// [`emit_seq_block_verilog`] with an optional STATE-PLANE port. When `with_heap`, the block exposes a shared-heap
/// interface (`heap_raddr`→`heap_rdata` async read, `heap_we`/`heap_waddr`/`heap_wdata` sync write, `alloc_req`/
/// `alloc_size`→`hp_in` bump-allocate) that the whole-program top wires to ONE register-file heap threaded across
/// the block walk. A `HeapRead` is a combinational step (`R[i]<=heap_rdata` over the driven `heap_raddr`); a
/// `HeapWrite` asserts the write port for its step (value = the written word); a `HeapAlloc` returns the current
/// top `hp_in` and pulses `alloc_req`. `heap_we`/`alloc_req` are gated by `running`, so an inactive block never
/// mutates the heap. Bit-identical to the VM's `[len, e1..eN]` 1-based layout (proven ≡VM by iverilog).
fn emit_seq_block_verilog_sp(name: &str, block: &CfgBlock, with_heap: bool) -> String {
    let name_in = |k: usize| format!("in{k}");
    let name_val = |k: usize| format!("R[{k}]");
    let src = |s: &Src| src_named(s, &name_in, &name_val);
    let m = block.body.len();
    let n_in = block.inputs.len();
    let mut needed: Vec<Src> = block.writes.iter().map(|(_, s)| *s).collect();
    match &block.term {
        Term::Jump(_) => {}
        Term::Branch { cond, .. } => needed.push(*cond),
        Term::Return(s) => needed.push(*s),
    }
    let inports = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inports = if inports.is_empty() {
        String::new()
    } else {
        format!("{inports}, ")
    };
    let outports = (0..needed.len())
        .map(|j| format!("output [63:0] o{j}"))
        .collect::<Vec<_>>()
        .join(", ");
    let outports = if outports.is_empty() {
        String::new()
    } else {
        format!("{outports}, ")
    };
    let outassign = needed
        .iter()
        .enumerate()
        .map(|(j, s)| format!("  assign o{j} = {};\n", src(s)))
        .collect::<String>();
    // Optional STATE-PLANE port + side-band registers driven per-step by the heap cells.
    let heap_ports = if with_heap {
        "output [63:0] heap_raddr, input [63:0] heap_rdata, output heap_we, output [63:0] heap_waddr, \
         output [63:0] heap_wdata, output alloc_req, output [63:0] alloc_size, input [63:0] hp_in, "
    } else {
        ""
    };
    let (heap_regs, heap_defaults, heap_assigns) = if with_heap {
        (
            "  reg [63:0] heap_raddr_r, heap_waddr_r, heap_wdata_r, alloc_size_r; reg heap_we_r, alloc_req_r;\n",
            "    heap_raddr_r=64'd0; heap_we_r=1'b0; heap_waddr_r=64'd0; heap_wdata_r=64'd0; alloc_req_r=1'b0; alloc_size_r=64'd0;\n",
            "  assign heap_raddr=heap_raddr_r; assign heap_we=running && heap_we_r; assign heap_waddr=heap_waddr_r;\n\
             \x20 assign heap_wdata=heap_wdata_r; assign alloc_req=running && alloc_req_r; assign alloc_size=alloc_size_r;\n",
        )
    } else {
        ("", "", "")
    };
    let mut arms = String::new();
    for (i, c) in block.body.iter().enumerate() {
        // Heap cells settle in one combinational step, driving the side-band port; everything else via seq_cell_arm.
        let arm = match c {
            LoopCell::HeapRead { addr } => {
                format!(
                    "is_comb=1'b1; comb_y=heap_rdata; heap_raddr_r={};",
                    src(addr)
                )
            }
            LoopCell::HeapWrite { addr, val } => format!(
                "is_comb=1'b1; comb_y={v}; heap_we_r=1'b1; heap_waddr_r={a}; heap_wdata_r={v};",
                a = src(addr),
                v = src(val)
            ),
            LoopCell::HeapAlloc { size } => {
                format!(
                    "is_comb=1'b1; comb_y=hp_in; alloc_req_r=1'b1; alloc_size_r={};",
                    src(size)
                )
            }
            _ => seq_cell_arm(c, &src),
        };
        arms.push_str(&format!("      {i}: begin {arm} end\n"));
    }
    // A block whose body is empty (writes/term read inputs directly) is trivially done (running stays 0); its
    // `R[step]` references are dead but must still elaborate, so `R` is always declared with depth ≥ 1.
    let mdecl = format!("localparam M = {m};");
    let rdecl = format!(
        "  (* ram_style=\"registers\" *) reg [63:0] R [0:{}];\n",
        m.max(1) - 1
    );
    format!(
        r#"// seqblk_{name} — a per-block SEQUENCED datapath (LATENCY-INSENSITIVE): start pulse -> walk the cells
// (combinational in one step, division/float on the shared multi-cycle units, heap on the shared state plane) ->
// done + the block's outputs.
module seqblk_{name}(input clk, input rst, input start, {inports}{outports}{heap_ports}output done);
  {mdecl}
{rdecl}  reg [31:0] step;
  reg running, issued;
  reg is_comb; reg [63:0] comb_y; reg [63:0] ua, ub; reg [3:0] usel;
{heap_regs}{converters}  always @(*) begin
    is_comb=1'b1; comb_y=64'd0; ua=64'd0; ub=64'd0; usel=4'd0;
{heap_defaults}    case (step)
{arms}      default: ;
    endcase
  end
{units}  always @(posedge clk) begin
    if (rst) begin running<=1'b0; step<=32'd0; issued<=1'b0; end
    else if (start) begin step<=32'd0; running<=(M>0); issued<=1'b0; end
    else if (running) begin
      if (is_comb) begin
        R[step] <= comb_y;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end else if (!issued) begin
        issued <= 1'b1;
      end else if (mc_done) begin
        R[step] <= mc_y; issued<=1'b0;
        if (step==M-1) running<=1'b0; else step<=step+32'd1;
      end
    end
  end
  assign done = !running;
{heap_assigns}{outassign}endmodule
"#,
        converters = I64_F64_CONVERTERS,
        units = seq_units_for(&block.body),
    )
}

/// Emit the DONE-AWARE whole-program sequenced fabric (`seqprog_{name}`): the per-block sequenced datapaths
/// (`seqblk_*`, each with multi-cycle division/float TILES) + a block-PC FSM that STARTS the current block, waits
/// its `done`, latches the register writes, and takes the terminator (Jump/Branch/Return). This is the
/// unification — one whole program (loop + division + float) on one spatial fabric, latency-insensitive.
/// Proven == `run_program_cfg_on_grid` == the VM by iverilog.
pub fn emit_seq_program_verilog(name: &str, pf: &ProgramCfgFabric) -> String {
    let nb = pf.blocks.len();
    assert!(nb >= 1, "seq program: need ≥1 block");
    // Does the program touch the shared state plane? If so, the top instantiates ONE register-file heap
    // (async read, sync write, bump allocator) threaded across the block walk and every block gets the heap port.
    let has_heap = pf.blocks.iter().any(|b| b.body.iter().any(cell_is_heap));
    // The shared heap is a bounded register file (the fabric is a bounded-heap machine by design). 64 words is
    // ample for the collection/string programs the state plane targets and keeps the FF budget FPGA-feasible
    // (64×64 = 4096 FFs vs 126k on the Nexys A7); a program that overflows it diverges from the VM (a genuine
    // bound, not a silent wrap) and would be caught ≡VM.
    const HEAP_WORDS: usize = 64;
    let aw = (usize::BITS - (HEAP_WORDS - 1).leading_zeros()).max(1) as usize;
    let nreg = {
        let mut max_reg = 0u16;
        for b in &pf.blocks {
            for r in &b.inputs {
                max_reg = max_reg.max(*r);
            }
            for (r, _) in &b.writes {
                max_reg = max_reg.max(*r);
            }
        }
        (max_reg as usize + 1).max(pf.n_params).max(1)
    };
    let pcw = {
        let mut b = 1;
        while (1usize << b) < nb {
            b += 1;
        }
        b
    };

    let mut modules = String::new();
    let mut needs: Vec<(Vec<Src>, usize)> = Vec::with_capacity(nb);
    for (k, b) in pf.blocks.iter().enumerate() {
        modules.push_str(&emit_seq_block_verilog_sp(
            &format!("{name}_b{k}"),
            b,
            has_heap,
        ));
        modules.push('\n');
        let mut needed: Vec<Src> = b.writes.iter().map(|(_, s)| *s).collect();
        let term_off = needed.len();
        match &b.term {
            Term::Jump(_) => {}
            Term::Branch { cond, .. } => needed.push(*cond),
            Term::Return(s) => needed.push(*s),
        }
        needs.push((needed, term_off));
    }

    let mut ports: Vec<String> = vec!["input clk".into(), "input rst".into()];
    for p in 0..pf.n_params {
        ports.push(format!("input [63:0] p{p}"));
    }
    ports.push("output [63:0] result".into());
    ports.push("output done".into());

    let mut body = String::new();
    body.push_str(&format!("  reg [63:0] file [0:{}];\n", nreg - 1));
    body.push_str(&format!("  reg [{}:0] pc;\n", pcw - 1));
    body.push_str("  reg done_r; reg [63:0] result_r; reg started;\n");
    body.push_str("  assign done = done_r; assign result = result_r;\n");
    for r in 0..nreg {
        body.push_str(&format!("  wire [63:0] fr_{r} = file[{r}];\n"));
    }
    // The shared STATE PLANE: ONE register-file heap (async read, sync write) + a bump-pointer `hp`, threaded
    // across the whole block walk. `ram_style=registers` forces real FFs (the open FOSS flow drops write-ported
    // LUT-RAM). The active (pc-selected) block owns the heap port; inactive blocks gate `heap_we`/`alloc_req` low.
    if has_heap {
        body.push_str(&format!(
            "  (* ram_style=\"registers\" *) reg [63:0] heap [0:{}];\n",
            HEAP_WORDS - 1
        ));
        body.push_str("  reg [63:0] hp;\n");
        for k in 0..nb {
            body.push_str(&format!(
                "  wire [63:0] b{k}_heap_raddr, b{k}_heap_waddr, b{k}_heap_wdata, b{k}_alloc_size; wire b{k}_heap_we, b{k}_alloc_req;\n"
            ));
        }
        body.push_str(
            "  reg [63:0] act_raddr, act_waddr, act_wdata, act_size; reg act_we, act_alloc;\n",
        );
        body.push_str("  always @(*) begin\n    act_raddr=64'd0; act_waddr=64'd0; act_wdata=64'd0; act_size=64'd0; act_we=1'b0; act_alloc=1'b0;\n    case (pc)\n");
        for k in 0..nb {
            body.push_str(&format!(
                "      {pcw}'d{k}: begin act_raddr=b{k}_heap_raddr; act_we=b{k}_heap_we; act_waddr=b{k}_heap_waddr; act_wdata=b{k}_heap_wdata; act_alloc=b{k}_alloc_req; act_size=b{k}_alloc_size; end\n"
            ));
        }
        body.push_str("      default: ;\n    endcase\n  end\n");
        body.push_str(&format!(
            "  wire [63:0] heap_rdata_bus = heap[act_raddr[{}:0]];\n",
            aw - 1
        ));
        body.push_str("  integer hj;\n  always @(posedge clk) begin\n");
        body.push_str(&format!("    if (rst) begin hp<=64'd0; for (hj=0; hj<{HEAP_WORDS}; hj=hj+1) heap[hj]<=64'd0; end\n"));
        body.push_str("    else begin\n");
        body.push_str(&format!(
            "      if (act_we) heap[act_waddr[{}:0]] <= act_wdata;\n",
            aw - 1
        ));
        body.push_str("      if (act_alloc) hp <= hp + act_size;\n");
        body.push_str("      else if (act_we && ($signed(act_waddr) + 64'sd1 > $signed(hp))) hp <= act_waddr + 64'sd1;\n");
        body.push_str("    end\n  end\n");
    }
    // Per-block datapath instances (all present; only the pc-selected one is started).
    for (k, b) in pf.blocks.iter().enumerate() {
        let (needed, _) = &needs[k];
        for j in 0..needed.len() {
            body.push_str(&format!("  wire [63:0] b{k}_o{j};\n"));
        }
        body.push_str(&format!("  wire b{k}_done;\n"));
        let mut conns: Vec<String> = vec![
            ".clk(clk)".into(),
            ".rst(rst)".into(),
            format!(".start((pc=={pcw}'d{k}) && !started && !done_r)"),
        ];
        for (j, r) in b.inputs.iter().enumerate() {
            conns.push(format!(".in{j}(fr_{r})"));
        }
        for j in 0..needed.len() {
            conns.push(format!(".o{j}(b{k}_o{j})"));
        }
        if has_heap {
            conns.push(format!(".heap_raddr(b{k}_heap_raddr)"));
            conns.push(".heap_rdata(heap_rdata_bus)".into());
            conns.push(format!(".heap_we(b{k}_heap_we)"));
            conns.push(format!(".heap_waddr(b{k}_heap_waddr)"));
            conns.push(format!(".heap_wdata(b{k}_heap_wdata)"));
            conns.push(format!(".alloc_req(b{k}_alloc_req)"));
            conns.push(format!(".alloc_size(b{k}_alloc_size)"));
            conns.push(".hp_in(hp)".into());
        }
        conns.push(format!(".done(b{k}_done)"));
        body.push_str(&format!(
            "  seqblk_{name}_b{k} u_b{k}({});\n",
            conns.join(", ")
        ));
    }
    // The currently-selected block's done.
    body.push_str("  reg cur_done;\n  always @(*) begin cur_done = 1'b1; case (pc)\n");
    for k in 0..nb {
        body.push_str(&format!("    {pcw}'d{k}: cur_done = b{k}_done;\n"));
    }
    body.push_str("    default: cur_done = 1'b1;\n  endcase end\n");

    // The block-walk FSM: start the block (one cycle), wait cur_done, latch writes, take the terminator.
    body.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    body.push_str(&format!(
        "      pc<={pcw}'d0; done_r<=1'b0; result_r<=64'd0; started<=1'b0;\n"
    ));
    for r in 0..nreg {
        if r < pf.n_params {
            body.push_str(&format!("      file[{r}]<=p{r};\n"));
        } else {
            body.push_str(&format!("      file[{r}]<=64'd0;\n"));
        }
    }
    body.push_str("    end else if (!done_r) begin\n");
    body.push_str("      if (!started) started<=1'b1;\n");
    body.push_str("      else if (cur_done) begin\n        started<=1'b0;\n        case (pc)\n");
    for (k, b) in pf.blocks.iter().enumerate() {
        let (_, term_off) = &needs[k];
        body.push_str(&format!("          {pcw}'d{k}: begin\n"));
        for (i, (r, _)) in b.writes.iter().enumerate() {
            body.push_str(&format!("            file[{r}] <= b{k}_o{i};\n"));
        }
        match &b.term {
            Term::Jump(t) => body.push_str(&format!("            pc <= {pcw}'d{t};\n")),
            Term::Branch { then_blk, else_blk, .. } => body.push_str(&format!(
                "            pc <= (b{k}_o{term_off} != 64'd0) ? {pcw}'d{then_blk} : {pcw}'d{else_blk};\n"
            )),
            Term::Return(_) => {
                body.push_str(&format!("            result_r <= b{k}_o{term_off}; done_r <= 1'b1;\n"))
            }
        }
        body.push_str("          end\n");
    }
    body.push_str("          default: done_r<=1'b1;\n        endcase\n      end\n    end\n  end\n");

    format!(
        "// seqprog_{name} — the whole-program SEQUENCED fabric (LATENCY-INSENSITIVE): per-block datapaths with\n\
         // multi-cycle division/float TILES + a block-PC FSM that starts each block, waits its done, latches the\n\
         // register writes, and takes the terminator. == run_program_cfg_on_grid == the VM.\n\
         {modules}\
         module seqprog_{name}(\n    {ports}\n);\n{body}endmodule\n",
        ports = ports.join(",\n    "),
    )
}

/// The first cell the STANDALONE sequenced decision datapath cannot lower (it is register-only — no heap), as a
/// typed [`LoweringError::StatePlaneUnlowered`]. `Ok(())` ⇒ every cell is a register tile (Arith/Select/Func).
/// A heap-bearing DecisionFabric belongs on [`emit_state_plane_verilog`], not the bare datapath.
fn seq_datapath_cells_supported(cells: &[LoopCell]) -> Result<(), LoweringError> {
    for c in cells {
        if cell_is_heap(c) {
            return Err(LoweringError::StatePlaneUnlowered(format!("{c:?}")));
        }
    }
    Ok(())
}

/// The first cell the WHOLE-PROGRAM sequenced fabric cannot lower. `HeapAlloc`/`HeapRead`/`HeapWrite` now lower via
/// the shared state plane, so only a `MemRead` (the loop-sequence induction scratchpad — a distinct mechanism)
/// remains a typed refusal. `Ok(())` ⇒ the program lowers (register tiles + the state plane).
fn seq_program_cells_supported(cells: &[LoopCell]) -> Result<(), LoweringError> {
    for c in cells {
        if matches!(c, LoopCell::MemRead { .. }) {
            return Err(LoweringError::StatePlaneUnlowered(format!("{c:?}")));
        }
    }
    Ok(())
}

/// Graceful entry to the whole-program sequenced fabric: `Ok(rtl)` when every block lowers (register tiles + the
/// shared state plane for Seq/string programs), or `Err(StatePlaneUnlowered)` for the residual `MemRead`
/// loop-sequence form. The typed companion to the panicking [`emit_seq_program_verilog`].
pub fn try_emit_seq_program_verilog(
    name: &str,
    pf: &ProgramCfgFabric,
) -> Result<String, LoweringError> {
    for b in &pf.blocks {
        seq_program_cells_supported(&b.body)?;
    }
    Ok(emit_seq_program_verilog(name, pf))
}

/// Graceful entry to a single sequenced decision datapath: `Ok(rtl)` when every cell is register-only, or
/// `Err(StatePlaneUnlowered)` for a heap-reading fabric (which belongs on the state-plane emitter). The typed
/// companion to [`emit_seq_datapath_verilog`].
pub fn try_emit_seq_datapath_verilog(
    name: &str,
    df: &DecisionFabric,
) -> Result<String, LoweringError> {
    seq_datapath_cells_supported(&df.cells)?;
    Ok(emit_seq_datapath_verilog(name, df))
}

/// Emit a Nexys A7 SoC wrapper for the whole-program sequenced fabric: a baked vector ROM steps `seqprog_{name}`
/// through each input (drive params → pulse the fabric reset → wait `done` → capture result), streaming a
/// 12-byte record `{0xA5, result[63:0] MSB-first, run_cycles[15:0], 0x5A}` out the UART TX per vector, then
/// looping. Slow /128 core clock (781.25 kHz) + power-on reset. `vectors` are the params per run (n_params each).
/// This is the multi-cycle fabric (division + fpu.v tiles) on real silicon.
pub fn emit_seqprog_soc_verilog(name: &str, pf: &ProgramCfgFabric, vectors: &[Vec<i64>]) -> String {
    let np = pf.n_params;
    let nv = vectors.len();
    assert!(
        nv >= 1 && vectors.iter().all(|v| v.len() == np),
        "each vector must have n_params={np} entries"
    );
    let core = emit_seq_program_verilog(name, pf);

    // The baked vector ROM: case(vec) drives p0..p{np-1}.
    let mut rom = String::new();
    for (i, v) in vectors.iter().enumerate() {
        let sets = (0..np)
            .map(|k| {
                let x = v[k];
                if x < 0 {
                    format!("p{k}=-64'sd{};", -(x as i128))
                } else {
                    format!("p{k}=64'sd{x};")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        rom.push_str(&format!("      {i}: begin {sets} end\n"));
    }
    let pdecls = (0..np)
        .map(|k| format!("  reg signed [63:0] p{k};\n"))
        .collect::<String>();
    let pconn = (0..np)
        .map(|k| format!(".p{k}(p{k}), "))
        .collect::<String>();
    let pdefault = (0..np)
        .map(|k| format!("p{k}=64'sd0;"))
        .collect::<Vec<_>>()
        .join(" ");
    let vecw = std::cmp::max(1, (usize::BITS - (nv.max(2) - 1).leading_zeros()) as usize);

    format!(
        r#"{core}
// nexys_seqprog_{name} — the whole-program SEQUENCED fabric (multi-cycle division/float tiles) on a Nexys A7.
// A baked vector ROM runs seqprog_{name} for each input (drive params -> pulse fab_rst -> wait done -> capture),
// streaming {{0xA5, result[63:0], run_cycles[15:0], 0x5A}} out the UART per vector. 781.25 kHz core clock.
module nexys_seqprog_{name}(input clk, input rst, output reg uart_tx, output [15:0] led);
  localparam CLKS_PER_BIT = 81;   // 781.25e3 / 9600
  localparam NVEC = {nv};
  // /128 core clock on a global buffer.
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core(.I(divcnt[6]), .O(coreclk));
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire soc_rst = rst | ~por[3];

  reg [{vhi}:0] vec;
{pdecls}  always @(*) begin
    {pdefault}
    case (vec)
{rom}      default: ;
    endcase
  end
  reg fab_rst;
  wire [63:0] fab_result; wire fab_done;
  reg [63:0] result_l; reg [15:0] cyc_l; reg [15:0] runcyc;
  seqprog_{name} u_fab(.clk(coreclk), .rst(fab_rst), {pconn}.result(fab_result), .done(fab_done));

  localparam R_POR=3'd0, R_RUN=3'd1, R_WAIT=3'd2, R_LOAD=3'd3, R_START=3'd4, R_DATA=3'd5, R_STOP=3'd6;
  reg [2:0] rstate;
  reg [15:0] clkc; reg [2:0] bidx; reg [3:0] pos; reg [3:0] porc; reg [1:0] rstc;
  reg [7:0] txb;
  assign led = {{rstate, vec[4:0], 8'd0}};
  wire [3:0] rsel = (pos >= 4'd1 && pos <= 4'd8) ? (4'd8 - pos) : 4'd0;
  wire [7:0] res_byte = result_l[rsel * 8 +: 8];
  always @(posedge coreclk) begin
    if (soc_rst) begin
      rstate<=R_POR; uart_tx<=1'b1; clkc<=16'd0; bidx<=3'd0; vec<={vw}'d0; pos<=4'd0; porc<=4'd0;
      fab_rst<=1'b1; rstc<=2'd0; runcyc<=16'd0; result_l<=64'd0; cyc_l<=16'd0;
    end else case (rstate)
      R_POR: if (porc==4'd8) begin rstate<=R_RUN; fab_rst<=1'b1; rstc<=2'd0; end else porc<=porc+4'd1;
      R_RUN: begin
        // hold fab_rst 2 coreclks (load params + reset the fabric), then release to run.
        if (rstc==2'd2) begin fab_rst<=1'b0; runcyc<=16'd0; rstate<=R_WAIT; end else rstc<=rstc+2'd1;
      end
      R_WAIT: begin
        runcyc <= runcyc + 16'd1;
        if (fab_done) begin result_l<=fab_result; cyc_l<=runcyc; pos<=4'd0; rstate<=R_LOAD; end
      end
      R_LOAD: begin
        case (pos)
          4'd0:  txb<=8'hA5;
          4'd9:  txb<=cyc_l[15:8];
          4'd10: txb<=cyc_l[7:0];
          4'd11: txb<=8'h5A;
          default: txb<=res_byte;   // pos 1..8 = result[63:0] MSB-first
        endcase
        clkc<=16'd0; bidx<=3'd0; rstate<=R_START;
      end
      R_START: begin uart_tx<=1'b0; if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; rstate<=R_DATA; end else clkc<=clkc+16'd1; end
      R_DATA: begin uart_tx<=txb[bidx];
        if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0; if (bidx==3'd7) rstate<=R_STOP; else bidx<=bidx+3'd1; end else clkc<=clkc+16'd1;
      end
      R_STOP: begin uart_tx<=1'b1;
        if (clkc==CLKS_PER_BIT-1) begin clkc<=16'd0;
          if (pos==4'd11) begin
            pos<=4'd0;
            if (vec==NVEC-1) vec<={vw}'d0; else vec<=vec+{vw}'d1;
            fab_rst<=1'b1; rstc<=2'd0; rstate<=R_RUN;
          end else begin pos<=pos+4'd1; rstate<=R_LOAD; end
        end else clkc<=clkc+16'd1;
      end
      default: rstate<=R_POR;
    endcase
  end
endmodule
"#,
        vhi = vecw - 1,
        vw = vecw,
    )
}

/// A self-checking clocked testbench for `seqprog_{name}`: per vector, load params, pulse rst, spin on `done`
/// (multi-cycle tiles + the block walk take many clocks), and check `result` == the VM (`seqprog_{name}_out.hex`).
pub fn emit_seq_program_testbench(name: &str, n_params: usize, n: usize) -> String {
    let decls = (0..n_params)
        .map(|k| format!("  reg [63:0] p{k};\n"))
        .collect::<String>();
    let ports = (0..n_params)
        .map(|k| format!(".p{k}(p{k}), "))
        .collect::<String>();
    let drive = (0..n_params)
        .map(|k| format!("      p{k} = oin[i*{n_params}+{k}];\n"))
        .collect::<String>();
    format!(
        r#"// tb_seqprog_{name} — the whole-program sequenced fabric vs the RUNNING VM, over {n} vectors.
`timescale 1ns/1ps
module tb_seqprog_{name};
  reg clk=0, rst=1;
{decls}  reg [63:0] oin  [0:{in_last}];
  reg [63:0] oout [0:{last}];
  wire [63:0] result; wire done;
  integer i, errors=0, timeout;
  seqprog_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("seqprog_{name}_in.hex", oin);
    $readmemh("seqprog_{name}_out.hex", oout);
    for (i=0; i<{n}; i=i+1) begin
{drive}      rst=1'b1; @(posedge clk); #1; rst=1'b0;
      timeout=0;
      while (!done && timeout<2000000) begin @(posedge clk); timeout=timeout+1; end
      #1;
      if (result !== oout[i]) begin errors=errors+1; $display("FAIL vec %0d: got %0d exp %0d", i, $signed(result), $signed(oout[i])); end
    end
    if (errors==0) $display("PASS: {n} whole-program sequenced-fabric runs match the VM");
    else $display("FAIL: %0d whole-program mismatches", errors);
    $finish;
  end
endmodule
"#,
        in_last = if n_params == 0 { 0 } else { n_params * n - 1 },
        last = n - 1,
    )
}

/// The two IEEE-754 int↔float converter Verilog FUNCTIONS shared by the sequenced datapath (and bit-identical
/// to the engine's): `i64_to_f64` (round-to-nearest-ties-even == `FuncUnit::IntToF`) and `f64_floor_i64`
/// (floor toward −inf == `FuncUnit::FFloorF`). Combinational, so IntToF/FFloor cells settle in one step.
const I64_F64_CONVERTERS: &str = r#"  function [63:0] i64_to_f64(input signed [63:0] n);
    reg s; reg [63:0] m; integer lead, i, sh; reg [10:0] be; reg [51:0] frac;
    reg [63:0] shifted, dropped; reg [53:0] sig; reg round_up;
    begin
      if (n == 64'sd0) i64_to_f64 = 64'd0;
      else begin
        s = n[63];
        m = s ? (~n + 64'sd1) : n;
        lead = 0;
        for (i = 63; i >= 0; i = i - 1) if (m[i] && lead == 0) lead = i;
        be = 11'd1023 + lead[10:0];
        if (lead <= 52) begin
          shifted = m << (52 - lead);
          i64_to_f64 = {s, be, shifted[51:0]};
        end else begin
          sh = lead - 52;
          sig = m >> sh;
          dropped = m << (64 - sh);
          round_up = dropped[63] && (|dropped[62:0] || sig[0]);
          if (round_up) sig = sig + 54'd1;
          if (sig[53]) begin be = be + 11'd1; frac = 52'd0; end
          else            frac = sig[51:0];
          i64_to_f64 = {s, be, frac};
        end
      end
    end
  endfunction
  function signed [63:0] f64_floor_i64(input [63:0] x);
    reg s; reg [10:0] e; reg [52:0] sig; integer ue; reg [63:0] mag; reg hasfrac;
    begin
      s = x[63]; e = x[62:52];
      if (e < 11'd1023)
        f64_floor_i64 = (s && !(e == 11'd0 && x[51:0] == 52'd0)) ? -64'sd1 : 64'sd0;
      else begin
        sig = {1'b1, x[51:0]};
        ue = e - 11'd1023;
        if (ue >= 52) begin mag = {11'd0, sig} << (ue - 52); hasfrac = 1'b0; end
        else begin
          mag = {11'd0, sig} >> (52 - ue);
          hasfrac = (({11'd0, sig} << (ue + 12)) != 64'd0);
        end
        if (!s) f64_floor_i64 = mag;
        else    f64_floor_i64 = hasfrac ? -(mag + 64'd1) : -mag;
      end
    end
  endfunction
"#;

/// A self-checking CLOCKED testbench for a `seq_{name}` sequenced datapath: for each of `n` input vectors it
/// pulses `rst`, waits for `done`, and checks `result` == the VM oracle (`seq_{name}_out.hex`). Multi-cycle tiles
/// (the divider / FPU) make a vector take many clocks, so it spins on `done` with a generous timeout.
pub fn emit_seq_datapath_testbench(name: &str, n_in: usize, n: usize) -> String {
    let decls = (0..n_in)
        .map(|k| format!("  reg [63:0] in{k};\n"))
        .collect::<String>();
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    let drive = (0..n_in)
        .map(|k| format!("      in{k} = oin[i*{n_in}+{k}];\n"))
        .collect::<String>();
    format!(
        r#"// tb_seq_{name} — the sequenced datapath vs the RUNNING VM, over the {n} vectors.
`timescale 1ns/1ps
module tb_seq_{name};
  reg clk=0, rst=1;
{decls}  reg [63:0] oin  [0:{in_last}];
  reg [63:0] oout [0:{last}];
  wire [63:0] result; wire done;
  integer i, errors=0, timeout;
  seq_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("seq_{name}_in.hex", oin);
    $readmemh("seq_{name}_out.hex", oout);
    for (i=0; i<{n}; i=i+1) begin
{drive}      rst=1'b1; @(posedge clk); #1; rst=1'b0;
      timeout=0;
      while (!done && timeout<200000) begin @(posedge clk); timeout=timeout+1; end
      #1;
      if (result !== oout[i]) begin errors=errors+1; $display("FAIL vec %0d: got %0d exp %0d", i, $signed(result), $signed(oout[i])); end
    end
    if (errors==0) $display("PASS: {n} sequenced-datapath evaluations match the VM (multi-cycle tiles ≡VM)");
    else $display("FAIL: %0d sequenced-datapath mismatches", errors);
    $finish;
  end
endmodule
"#,
        in_last = n_in * n - 1,
        last = n - 1,
    )
}

// ── STAGE X3b (XPU): the FLOAT LANE as synthesizable RTL. A `DecisionFabric` whose cells are IEEE-754 float
// functional units (`FAddF/FSubF/FMulF/FDivF/FSqrtF`) cannot settle combinationally: each op is the multi-cycle
// `fpu.v` unit (a start/done handshake FSM, round-to-nearest-ties-even). So the float lane lowers to a SEQUENCED
// controller that binds ONE of each `fpu.v` unit (shared across cells) and, per step, muxes the current cell's
// operands to the selected unit, pulses its `start`, waits its `done`, and latches the result register. Float
// VALUES are f64 BIT-PATTERNS on the 64-bit datapath (no separate typed channel). Proven ≡ the model
// (`eval_grid` over f64 bits) ≡ Rust f64 by iverilog; the `fpu.v` units are already bit-exact to the VM
// (`float_heap_matches_vm`). NOT SAT (IEEE-754 is outside QF_BV) — the same layered proof as division/state.

/// True iff the fabric is a pure FLOAT-arithmetic datapath (its cells are all `FuncUnit` float ops over
/// float/Val operands) — lowered by [`emit_float_datapath_verilog`]. It must end in a float result (no
/// `FFloorF` float→int bridge here — the caller floors the streamed f64 downstream) and carry no heap.
pub fn decision_fabric_is_float_arith(df: &DecisionFabric) -> bool {
    !df.cells.is_empty()
        && df.cells.iter().all(|c| {
            matches!(
                c,
                LoopCell::Func {
                    unit: FuncUnit::FAddF
                        | FuncUnit::FSubF
                        | FuncUnit::FMulF
                        | FuncUnit::FDivF
                        | FuncUnit::FSqrtF,
                    ..
                }
            )
        })
}

/// Lower a pure-float-arithmetic `DecisionFabric` to a SEQUENCED float datapath binding `fpu.v` (see the block
/// comment above). Each cell is one multi-cycle FPU step; the streamed `result` is the f64 bit-pattern of the
/// final float. Panics on any non-float-arith cell (the honest boundary — `FFloorF`/`Arith` are not bound here).
pub fn emit_float_datapath_verilog(name: &str, df: &DecisionFabric) -> String {
    let name_in = |k: usize| format!("in{k}");
    let name_val = |k: usize| format!("R[{k}]");
    let src = |s: &Src| src_named(s, &name_in, &name_val);
    let m = df.cells.len();
    assert!(m > 0, "a float datapath must have at least one cell");

    let inputs = (0..df.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };

    // Per-step operand + unit selection: usel 0=add, 1=sub, 2=mul, 3=div, 4=sqrt.
    let mut arms = String::new();
    for (i, c) in df.cells.iter().enumerate() {
        let (usel, a, b) = match c {
            LoopCell::Func { unit, a, b } => match unit {
                FuncUnit::FAddF => (0, src(a), src(b)),
                FuncUnit::FSubF => (1, src(a), src(b)),
                FuncUnit::FMulF => (2, src(a), src(b)),
                FuncUnit::FDivF => (3, src(a), src(b)),
                FuncUnit::FSqrtF => (4, src(a), "64'd0".to_string()),
                other => panic!(
                    "float datapath: {other:?} is not a bound fpu.v unit (add/sub/mul/div/sqrt)"
                ),
            },
            other => panic!("float datapath: cell {other:?} is not a float functional unit"),
        };
        arms.push_str(&format!(
            "        {i}: begin opa = {a}; opb = {b}; usel = 3'd{usel}; end\n"
        ));
    }

    format!(
        "// float_{name} — a pure-float-arithmetic datapath as a SEQUENCED controller binding fpu.v. One of each\n\
         // multi-cycle IEEE-754 unit (fp_addsub/fp_mul/fp_div/fp_sqrt) is SHARED across cells; a micro-sequencer\n\
         // muxes each cell's operands to the selected unit, pulses start, waits done, latches the result. The\n\
         // streamed `result` is the f64 BIT-PATTERN of the final float. Proven == the model (eval_grid over f64\n\
         // bits) == Rust f64 by iverilog; fpu.v is bit-exact to the VM.\n\
         module float_{name}(input clk, input rst, {inputs}output [63:0] result, output done);\n\
         \x20 localparam M = {m};\n\
         \x20 (* ram_style = \"registers\" *) reg [63:0] R [0:M-1];\n\
         \x20 reg [31:0] step;\n\
         \x20 reg running, issued;\n\
         \x20 reg [63:0] opa, opb; reg [2:0] usel;\n\
         \x20 always @(*) begin\n\
         \x20\x20 opa = 64'd0; opb = 64'd0; usel = 3'd0;\n\
         \x20\x20 case (step)\n\
         {arms}\
         \x20\x20\x20 default: ;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         \x20 wire is_addsub = (usel == 3'd0) || (usel == 3'd1);\n\
         \x20 wire is_sub = (usel == 3'd1);\n\
         \x20 wire is_mul = (usel == 3'd2);\n\
         \x20 wire is_div = (usel == 3'd3);\n\
         \x20 wire is_sqrt = (usel == 3'd4);\n\
         \x20 // the start pulse fires for exactly the selected unit in the cycle a step is issued.\n\
         \x20 wire pulse = running && !issued;\n\
         \x20 wire add_done, mul_done, div_done, sqrt_done;\n\
         \x20 wire [63:0] add_y, mul_y, div_y, sqrt_y;\n\
         \x20 fp_addsub u_add(.clk(clk), .rst(rst), .start(pulse && is_addsub), .a(opa), .b_in(opb), .sub(is_sub), .y(add_y), .done(add_done));\n\
         \x20 fp_mul    u_mul(.clk(clk), .rst(rst), .start(pulse && is_mul), .a(opa), .b(opb), .y(mul_y), .done(mul_done));\n\
         \x20 fp_div    u_div(.clk(clk), .rst(rst), .start(pulse && is_div), .a(opa), .b(opb), .y(div_y), .done(div_done));\n\
         \x20 fp_sqrt   u_sqrt(.clk(clk), .rst(rst), .start(pulse && is_sqrt), .a(opa), .y(sqrt_y), .done(sqrt_done));\n\
         \x20 wire sel_done = is_addsub ? add_done : is_mul ? mul_done : is_div ? div_done : sqrt_done;\n\
         \x20 wire [63:0] sel_y = is_addsub ? add_y : is_mul ? mul_y : is_div ? div_y : sqrt_y;\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin step <= 32'd0; running <= (M > 0); issued <= 1'b0; end\n\
         \x20\x20 else if (running) begin\n\
         \x20\x20\x20 if (!issued) issued <= 1'b1;            // the start pulse fired this cycle\n\
         \x20\x20\x20 else if (sel_done) begin\n\
         \x20\x20\x20\x20 R[step] <= sel_y; issued <= 1'b0;\n\
         \x20\x20\x20\x20 if (step == M - 1) running <= 1'b0; else step <= step + 32'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20 end\n\
         \x20 end\n\
         \x20 assign done = !running;\n\
         \x20 assign result = {res};\n\
         endmodule\n",
        res = src(&df.result),
    )
}

// ── STAGE X1c (XPU): the POWER LANE as synthesizable RTL. Exponentiation `a ** b` is NOT combinational: at
// the RTL level it is a `b`-step multiply FSM (the honest sequential functional unit, like the shared_divider
// is for division). So a `DecisionFabric` whose cells are `FuncUnit::Pow` lowers not to a wired DAG but to a
// SEQUENCED controller over the SAME cell stream — one shared multiply-accumulate loop per cell: latch the
// base/exponent, multiply the accumulator `b` times (exponent clamped to ≥ 0), then latch the result and
// advance. 64-bit signed wrapping, so bit-identical to `FuncUnit::eval`'s `a.wrapping_pow(b.max(0) as u32)`.
// Proven ≡ the model (eval) ≡ VM by iverilog (`fabric_pow_lock`); NOT SAT (an unbounded multiply loop is
// outside QF_BV), the same layered proof as division/float/state.

/// True iff the fabric is a pure POWER datapath (every cell is a `FuncUnit::Pow`) — lowered by
/// [`emit_pow_datapath_verilog`]. Pow cannot settle combinationally (`FuncUnit::verilog` refuses it), so a
/// Pow-bearing fabric takes this sequenced lane exactly as a float fabric takes the fpu.v lane.
pub fn decision_fabric_is_pow(df: &DecisionFabric) -> bool {
    !df.cells.is_empty()
        && df.cells.iter().all(|c| {
            matches!(
                c,
                LoopCell::Func {
                    unit: FuncUnit::Pow,
                    ..
                }
            )
        })
}

/// Lower a pure-`Pow` `DecisionFabric` to a SEQUENCED power datapath (see the block comment above). Each cell
/// is one `b`-step multiply loop over a shared accumulator; the streamed `result` is the final power. Panics
/// on any non-Pow cell (the honest boundary — mixed lanes are a follow-on).
pub fn emit_pow_datapath_verilog(name: &str, df: &DecisionFabric) -> String {
    let name_in = |k: usize| format!("in{k}");
    let name_val = |k: usize| format!("R[{k}]");
    let src = |s: &Src| src_named(s, &name_in, &name_val);
    let m = df.cells.len();
    assert!(m > 0, "a power datapath must have at least one cell");

    let inputs = (0..df.n_in)
        .map(|k| format!("input signed [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };

    // Per-step operand selection: the current cell's base + exponent, muxed into the shared MAC loop.
    let mut arms = String::new();
    for (i, c) in df.cells.iter().enumerate() {
        let (a, b) = match c {
            LoopCell::Func {
                unit: FuncUnit::Pow,
                a,
                b,
            } => (src(a), src(b)),
            other => panic!("power datapath: cell {other:?} is not a `Pow` functional unit"),
        };
        arms.push_str(&format!("        {i}: begin base = {a}; expo = {b}; end\n"));
    }

    format!(
        "// pow_{name} — a pure-exponentiation datapath as a SEQUENCED controller. One shared multiply-accumulate\n\
         // loop is time-multiplexed across the cells: per cell, latch (base, exponent), then multiply the\n\
         // accumulator `exponent` times (a negative exponent clamps to 0 → result 1), latch the power, advance.\n\
         // 64-bit signed wrapping, so bit-identical to the VM's `a ** b` (`wrapping_pow`). Proven == the model\n\
         // (eval) == VM by iverilog (fabric_pow_lock); NOT SAT (an unbounded multiply loop is outside QF_BV).\n\
         module pow_{name}(input clk, input rst, {inputs}output signed [63:0] result, output done);\n\
         \x20 localparam M = {m};\n\
         \x20 (* ram_style = \"registers\" *) reg signed [63:0] R [0:M-1];\n\
         \x20 reg [31:0] step;\n\
         \x20 reg running, loading;\n\
         \x20 reg signed [63:0] base, expo;\n\
         \x20 reg signed [63:0] acc;         // running product for the current cell\n\
         \x20 reg [63:0] rem;                // remaining multiplies (exponent, clamped ≥ 0)\n\
         \x20 always @(*) begin\n\
         \x20\x20 base = 64'sd0; expo = 64'sd0;\n\
         \x20\x20 case (step)\n\
         {arms}\
         \x20\x20\x20 default: ;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin step <= 32'd0; running <= (M > 0); loading <= 1'b1; acc <= 64'sd1; rem <= 64'd0; end\n\
         \x20\x20 else if (running) begin\n\
         \x20\x20\x20 if (loading) begin\n\
         \x20\x20\x20\x20 acc <= 64'sd1; rem <= (expo[63] ? 64'd0 : expo); loading <= 1'b0;\n\
         \x20\x20\x20 end else if (rem == 64'd0) begin\n\
         \x20\x20\x20\x20 R[step] <= acc; loading <= 1'b1;\n\
         \x20\x20\x20\x20 if (step == M - 1) running <= 1'b0; else step <= step + 32'd1;\n\
         \x20\x20\x20 end else begin\n\
         \x20\x20\x20\x20 acc <= acc * base; rem <= rem - 64'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20 end\n\
         \x20 end\n\
         \x20 assign done = !running;\n\
         \x20 assign result = {res};\n\
         endmodule\n",
        res = src(&df.result),
    )
}

/// A self-checking testbench for a power datapath: for each of `n` input vectors, pulse `rst`, drive `in{k}`,
/// run until `done` (bounded by `max_cycles`), and compare `result` against the VM oracle loaded from
/// `pow_{name}_{in,out}.hex`. Mirrors [`emit_state_plane_testbench`] (the sequenced clocked harness).
pub fn emit_pow_datapath_testbench(name: &str, n_in: usize, n: usize, max_cycles: usize) -> String {
    let drive = (0..n_in)
        .map(|k| format!("      in{k} = oin[i * {n_in} + {k}];\n"))
        .collect::<String>();
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    let decls = (0..n_in)
        .map(|k| format!("  reg signed [63:0] in{k};\n"))
        .collect::<String>();
    format!(
        r#"// tb_pow_{name} — the sequenced power datapath vs the RUNNING VM over the exponentiation battery.
`timescale 1ns/1ps
module tb_pow_{name};
  reg clk = 0, rst;
{decls}  reg signed [63:0] oin [0:{in_last}];
  reg signed [63:0] oout [0:{last}];
  wire signed [63:0] result;
  wire done;
  integer i, c, errors = 0;
  pow_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("pow_{name}_in.hex", oin);
    $readmemh("pow_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk);
{drive}      rst = 1; @(negedge clk); rst = 0;
      c = 0; while (!done && c < {max_cycles}) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL vec %0d: got %0d", i, result); end
    end
    if (errors == 0) $display("PASS: {n} power evaluations match the VM (a`b = the sequenced MAC lane)");
    else $display("FAIL: %0d pow mismatches", errors);
    $finish;
  end
endmodule
"#,
        in_last = n_in * n - 1,
        last = n - 1,
        max_cycles = max_cycles,
    )
}

// ── STAGE X2b (XPU): the STATE PLANE as synthesizable RTL. A `DecisionFabric` that carries heap cells
// (`HeapRead`/`HeapWrite`/`HeapAlloc` — collections/structs/text) cannot settle in one combinational pass:
// the heap is MUTABLE and the reads/writes are ORDERED. So it lowers not to a wired DAG but to a SEQUENCED
// controller over the SAME cell stream — the honest "dataflow-with-memory" machine. Each cell is one step of
// a tiny hardwired micro-sequencer (no fetch, no decode — the sequence IS the program): pure-compute cells
// latch a combinational result register; heap cells touch ONE shared heap (a LUTRAM: async read, sync write)
// with a bump-pointer allocator. This mirrors `eval_cells`/`eval_grid_with_heap` EXACTLY (row-major = cell
// order = step order; a read sees every prior step's committed write; alloc returns the heap top), so the RTL
// is proven ≡ VM by iverilog (stateful mutable heap is outside QF_BV — NOT SAT). It is the synthesizable
// realization of the Stage-X2a grid (`fabric_grid::eval_grid_with_heap`); both share the golden cell stream.

/// The heap-word budget for a state-plane kernel: enough words for the largest collection any vector touches
/// (a generous fixed LUTRAM — collections in these kernels are a handful of words; addresses index the low
/// `clog2(HEAP)` bits, so an in-bounds pointer always resolves). Kept a power of two for clean address slicing.
pub const STATE_PLANE_HEAP_WORDS: usize = 64;

/// True iff the fabric carries a state-plane (heap) cell — i.e. it must be lowered by
/// [`emit_state_plane_verilog`] (sequenced) rather than [`emit_decision_fabric_verilog`] (combinational).
pub fn decision_fabric_has_state(df: &DecisionFabric) -> bool {
    df.cells.iter().any(|c| {
        matches!(
            c,
            LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. }
        )
    })
}

/// Lower a heap-bearing `DecisionFabric` to a SEQUENCED state-plane controller (see the block comment above).
/// `heap_words` sizes the shared LUTRAM. Pure-compute cells render as combinational `v{i}` wires (over the
/// LATCHED cell registers `R[j]`) latched per step; heap cells access the LUTRAM with a bump-pointer `hp`.
pub fn emit_state_plane_verilog(name: &str, df: &DecisionFabric, heap_words: usize) -> String {
    let name_in = |k: usize| format!("in{k}");
    // Operands read the LATCHED result register of a prior cell (step order guarantees j < i is committed).
    let name_val = |k: usize| format!("R[{k}]");
    let src = |s: &Src| src_named(s, &name_in, &name_val);
    let m = df.cells.len();
    assert!(m > 0, "a state-plane fabric must have at least one cell");
    let aw = (usize::BITS - (heap_words - 1).leading_zeros()).max(1) as usize; // clog2(heap_words)

    let inputs = (0..df.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };

    // Per-cell combinational wires: pure-compute cells get their value `v{i}`; heap cells get their address
    // `addr{i}` (and a written value `wval{i}` for a write). Everything is 64-bit signed, wrapping like the VM.
    let mut wires = String::new();
    // The per-step case body: latch `R[i]` (and mutate heap/hp) for the cell at `step == i`.
    let mut arms = String::new();
    for (i, c) in df.cells.iter().enumerate() {
        match c {
            LoopCell::Arith { op, a, b } => {
                wires.push_str(&format!("  wire [63:0] v{i} = {};\n", op.verilog(&src(a), &src(b))));
                arms.push_str(&format!("        {i}: R[{i}] <= v{i};\n"));
            }
            LoopCell::VectorArith { op, ty, a, b } => {
                wires.push_str(&format!("  wire [63:0] v{i} = {};\n", op.verilog(*ty, &src(a), &src(b))));
                arms.push_str(&format!("        {i}: R[{i}] <= v{i};\n"));
            }
            LoopCell::Func { unit, a, b } => {
                wires.push_str(&format!("  wire [63:0] v{i} = {};\n", unit.verilog(&src(a), &src(b))));
                arms.push_str(&format!("        {i}: R[{i}] <= v{i};\n"));
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                wires.push_str(&format!(
                    "  wire [63:0] v{i} = ($signed({}) {} $signed({})) ? {} : {};\n",
                    src(ca), cmp.verilog(), src(cb), src(t), src(e)
                ));
                arms.push_str(&format!("        {i}: R[{i}] <= v{i};\n"));
            }
            LoopCell::HeapRead { addr } => {
                wires.push_str(&format!("  wire [63:0] addr{i} = {};\n", src(addr)));
                // Async read of the CURRENT heap — sees every prior step's committed (sync) write.
                arms.push_str(&format!("        {i}: R[{i}] <= heap[addr{i}[{hi}:0]];\n", hi = aw - 1));
            }
            LoopCell::HeapWrite { addr, val } => {
                wires.push_str(&format!("  wire [63:0] addr{i} = {};\n", src(addr)));
                wires.push_str(&format!("  wire [63:0] wval{i} = {};\n", src(val)));
                // Sync write; the tile's value is the written word; extend `hp` if the write grew the heap
                // (mirrors `eval_cells`' resize-on-out-of-bounds so a later alloc gets a fresh pointer).
                arms.push_str(&format!(
                    "        {i}: begin heap[addr{i}[{hi}:0]] <= wval{i}; R[{i}] <= wval{i}; \
                     if ($signed(addr{i}) + 64'sd1 > $signed(hp)) hp <= addr{i} + 64'sd1; end\n",
                    hi = aw - 1
                ));
            }
            LoopCell::HeapAlloc { size } => {
                wires.push_str(&format!("  wire [63:0] size{i} = {};\n", src(size)));
                // Bump allocator: the fresh pointer is the current heap top; reserve `size` zeroed words.
                arms.push_str(&format!(
                    "        {i}: begin R[{i}] <= hp; hp <= hp + size{i}; end\n"
                ));
            }
            LoopCell::MemRead { .. } => panic!(
                "state plane: a loop-sequence MemRead is the induction-counter scratchpad (P2), distinct \
                 from the shared-heap state plane"
            ),
        }
    }

    format!(
        "// state_{name} — a COLLECTION/STRUCT program as a SEQUENCED state-plane controller: a hardwired\n\
         // micro-sequencer (no fetch, no decode — the cell sequence IS the program) over ONE shared heap\n\
         // (LUTRAM: async read, sync write) with a bump-pointer allocator. Each cell is one step; pure-compute\n\
         // cells latch a combinational result, heap cells read/write the heap in program order. This is the\n\
         // Futamura projection for STATEFUL code: the residual dataflow-with-memory IS the circuit; proven ==\n\
         // the running VM by iverilog (mutable heap is outside QF_BV — eval+iverilog, not SAT). 64-bit wraps.\n\
         module state_{name}(input clk, input rst, {inputs}output [63:0] result, output done);\n\
         \x20 localparam M = {m};\n\
         \x20 localparam HEAP = {heap_words};\n\
         \x20 // ram_style=registers: the open FOSS flow (yosys+nextpnr-xilinx) drops write-ported LUT-RAM, so\n\
         \x20 // the shared heap + cell registers are forced to real FFs (a small register file) for silicon.\n\
         \x20 (* ram_style = \"registers\" *) reg  [63:0] R [0:M-1];\n\
         \x20 (* ram_style = \"registers\" *) reg  [63:0] heap [0:HEAP-1];\n\
         \x20 reg  [63:0] hp;\n\
         \x20 reg  [31:0] step;\n\
         \x20 reg  running;\n\
         \x20 integer j;\n\
         {wires}\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         \x20     step <= 0; hp <= 0; running <= (M > 0);\n\
         \x20     for (j = 0; j < HEAP; j = j + 1) heap[j] <= 64'd0;\n\
         \x20   end else if (running) begin\n\
         \x20     case (step)\n\
         {arms}\
         \x20       default: ;\n\
         \x20     endcase\n\
         \x20     if (step == M - 1) running <= 1'b0;\n\
         \x20     else step <= step + 1;\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = !running;\n\
         \x20 assign result = {res};\n\
         endmodule\n",
        res = src(&df.result),
    )
}

/// A self-checking testbench for a state-plane controller: for each of `n` input vectors, pulse `rst`, drive
/// `in{k}`, run until `done` (bounded by `max_cycles`), and compare `result` against the VM oracle loaded from
/// `state_{name}_{in,out}.hex`. Mirrors [`emit_loop_fabric_testbench`] (the CGRA clocked harness).
pub fn emit_state_plane_testbench(name: &str, n_in: usize, n: usize, max_cycles: usize) -> String {
    let drive = (0..n_in)
        .map(|k| format!("in{k} = oin[i*{n_in}+{k}];"))
        .collect::<Vec<_>>()
        .join(" ");
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!("{ports}, ")
    };
    let decls = (0..n_in)
        .map(|k| format!("  reg  [63:0] in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"// tb_state_{name} — the sequenced state-plane controller vs the RUNNING VM over the collection program.
`timescale 1ns/1ps
module tb_state_{name};
  reg clk = 0, rst;
{decls}
  reg  [63:0] oin [0:{ilast}];
  reg  [63:0] oout [0:{olast}];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  state_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("state_{name}_in.hex", oin);
    $readmemh("state_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk); {drive} rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < {maxc}) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n} state-plane runs match the VM (collections on the heap, sequenced)");
    else $display("FAIL: %0d state-plane mismatches", errors);
    $finish;
  end
endmodule
"#,
        ilast = (n * n_in).saturating_sub(1),
        olast = n.saturating_sub(1),
        maxc = max_cycles,
    )
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// WHOLE-FUNCTION CFG FABRIC (Phase 3) — the general multi-block controller.
//
// A function's control-flow graph is partitioned into basic blocks (the SAME partition the WASM backend
// proves out, `vm::wasm::cfg::Blocks`). Each block's straight-line body lowers to a WIRED residual island
// (SSA `LoopCell`s over the block's live-in registers — no per-op fetch); a block-sequencing CONTROLLER
// walks the graph, evaluating each block's terminator PREDICATE to pick the successor. The predicate feeds
// the controller, NOT the tile array — the clean separation the plan calls for. This composes islands into
// a whole function and handles branches BETWEEN blocks + early returns — the general control flow the
// single-construct decision/loop fabrics could not. (Loops with back-edges keep the counted/sequence loop
// fabric; this controller is the acyclic branch composition — the missing capstone piece.)

/// A basic block's terminator: where control goes after the block's body runs.
#[derive(Clone, Debug)]
pub enum Term {
    /// Unconditional edge to block `k`.
    Jump(usize),
    /// Predicated edge: `cond != 0 ? then_blk : else_blk`.
    Branch {
        cond: Src,
        then_blk: usize,
        else_blk: usize,
    },
    /// Return the value named by `Src` (over the block's inputs + body vals).
    Return(Src),
}

/// One spatially-lowered basic block: a wired residual body + its live-in registers + the register writes
/// it commits + its terminator.
#[derive(Clone, Debug)]
pub struct CfgBlock {
    /// Live-in registers, in input order: `Src::In(k)` in `body`/`writes`/`term` refers to `inputs[k]`.
    pub inputs: Vec<u16>,
    /// The block's straight-line datapath (SSA cells).
    pub body: Vec<LoopCell>,
    /// Each register the block writes, paired with the `Src` giving its final value.
    pub writes: Vec<(u16, Src)>,
    /// ★ WHAT THIS BLOCK SHOWS, in order — the block's slice of the program's output tape.
    ///
    /// A function RETURNS one value and the terminator carries it. A `## Main` SHOWS a sequence, and a
    /// branchy Main shows a DIFFERENT sequence down each path — which is exactly why the combinational
    /// output-tape path refuses it ("a conditional Show cannot ride the combinational tape") and why the
    /// tape has to live per block here. Empty for every function lowering, so nothing that existed
    /// before this field moves.
    pub shows: Vec<Src>,
    /// Where control goes next.
    pub term: Term,
}

/// A whole function lowered to spatial block islands + a block-sequencing controller.
#[derive(Clone, Debug)]
pub struct ProgramCfgFabric {
    pub n_params: usize,
    pub blocks: Vec<CfgBlock>,
}

/// Lower a whole (acyclic-control-flow) function to a [`ProgramCfgFabric`]. Panics on a back-edge (a loop
/// — use the counted/sequence loop fabric) or an op outside the supported straight-line set (honest bound).
/// Per-collection RESERVED CAPACITY for a dynamically-grown Seq/Map/Set (`NewEmptyList` + `Push`). The flat heap
/// stores a collection contiguously `[len, e0, e1…]`; an empty collection reserves this many element slots UP
/// FRONT so it grows WITHIN its own block instead of at the shared heap frontier — which is what lets MULTIPLE
/// collections grow concurrently without colliding (matrix_mult/knapsack/fannkuch/mergesort). This is a BOUND,
/// consistent with the fabric's bounded-heap nature (the RTL heap is a fixed BRAM): a collection that exceeds it
/// overflows into the next block and the whole-program result diverges from the VM — which the census's ≡VM
/// check catches as a REFUSED benchmark (never a silent wrong answer), so the bound is self-guarding. Sized to
/// clear the largest collection any benchmark builds (analytics' 1024-bucket table) with margin.
pub(crate) const SEQ_RESERVE: i64 = 2048;

thread_local! {
    /// Per-lowering override of the per-collection reserve. `None` → the safe default [`SEQ_RESERVE`] (headroom
    /// for MULTIPLE collections growing concurrently without colliding — matrix_mult / knapsack / analytics).
    /// A program whose collections fill SEQUENTIALLY (each `Seq` completed before the next is allocated — the
    /// transformer's forward pass) needs almost none: `HeapWrite` auto-grows the frontier, so a small reserve
    /// gives the exact right-sized heap. Set via [`with_seq_reserve`]; every lowering is ≡VM-checked at its
    /// chosen reserve, so an under-reserve that would collide is caught as a REFUSED (wrong-answer) program.
    static SEQ_RESERVE_OVERRIDE: std::cell::RefCell<Option<i64>> = const { std::cell::RefCell::new(None) };
}

/// The reserve to use right now (the thread-local override, else the default).
pub(crate) fn seq_reserve() -> i64 {
    SEQ_RESERVE_OVERRIDE.with(|c| c.borrow().unwrap_or(SEQ_RESERVE))
}

/// Run `f` with the per-collection reserve set to `n` (restored after). Use a small `n` for a program that
/// fills its collections sequentially (e.g. the whole-transformer fabric engine) to right-size its heap.
pub fn with_seq_reserve<T>(n: i64, f: impl FnOnce() -> T) -> T {
    let prev = SEQ_RESERVE_OVERRIDE.with(|c| c.replace(Some(n)));
    let out = f();
    SEQ_RESERVE_OVERRIDE.with(|c| *c.borrow_mut() = prev);
    out
}

/// The INT→FLOAT promotion of a float op's operand: if `is_float`, the operand already carries an `f64` bit
/// pattern and is returned unchanged; otherwise it holds an integer value that the VM would coerce (`*a as f64`
/// in `arith::multiply` etc. when an `Int` meets a `Float`), so a [`FuncUnit::IntToF`] cell is appended and its
/// result substituted. This realizes the VM's dynamic `Int ∘ Float → Float` coercion statically, so mixed
/// int/float arithmetic (`2.0 * k`) routes correctly through the float functional units.
fn promote_float_operand(cells: &mut Vec<LoopCell>, s: Src, is_float: bool) -> Src {
    if is_float {
        return s;
    }
    let k = cells.len();
    cells.push(LoopCell::Func {
        unit: FuncUnit::IntToF,
        a: s,
        b: Src::Const(0),
    });
    Src::Val(k)
}

/// Mark a register as holding a plain INTEGER — clearing any stale float / float-collection tracking. VM register
/// slots are reused across int and float temps, so an op that writes an integer result MUST retract a prior
/// float mark, else a later read (e.g. a loop counter or array index that reused a float temp's slot) would be
/// mis-routed through the float lane. Called by every integer-producing arm.
fn mark_int_reg(dst: u16) {
    FLOAT_REGS.with(|t| {
        t.borrow_mut().remove(&dst);
    });
    FLOAT_COLLS.with(|t| {
        t.borrow_mut().remove(&dst);
    });
}

/// Why the whole-program fabric lowering refused a program. A TYPED failure surface: unsupported constructs
/// return `Err(LoweringError)` from the `try_*` entry points instead of panicking, so callers (and the
/// `lowering_coverage_lock` ratchet) can tell "this construct is not lowered yet" apart from a real bug. The
/// `Display` strings are byte-identical to the old `panic!` messages, so the panicking wrappers are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringError {
    /// A CFG-block op with no fabric tile (a loop/channel/error op in a straight-line body, etc.).
    UnsupportedOp(String),
    /// A builtin with no fabric unit wired (only floor/sqrt over the float lane today).
    UnsupportedBuiltin(String),
    /// A loop-fabric body op with no tile (a nested jump the if-converter cannot absorb, etc.).
    UnsupportedLoopBody(String),
    /// A `Mem`/`Heap` STATE-PLANE cell reached the register-only SEQUENCED datapath. The sequenced fabric is a
    /// register-file arithmetic engine (multi-cycle divider + FPU + power tiles); Seq/string programs carry a
    /// shared heap and run on the wpe engine (or the state-plane block form — the named follow-on). Detected up
    /// front by [`try_emit_seq_program_verilog`] / [`try_emit_seq_datapath_verilog`] so a heap program is a
    /// typed refusal, never a raw panic.
    StatePlaneUnlowered(String),
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoweringError::UnsupportedOp(op) => write!(
                f,
                "cfg block: op `{op}` unsupported (straight-line integer + collection body only)"
            ),
            LoweringError::UnsupportedBuiltin(b) => write!(
                f,
                "cfg block: builtin `{b}` unsupported on the whole-program fabric \
                 (only floor/sqrt over the float lane are wired)"
            ),
            LoweringError::UnsupportedLoopBody(op) => {
                write!(f, "loop fabric: body op `{op}` unsupported")
            }
            LoweringError::StatePlaneUnlowered(cell) => write!(
                f,
                "seq datapath: {cell} (Mem/Heap) is a state-plane follow-on"
            ),
        }
    }
}
impl std::error::Error for LoweringError {}

/// Panicking wrapper preserving the original contract for all existing callers: lowers `fn_name`'s CFG to a
/// whole-program fabric, or panics with the same message the old code did. New code that wants to handle an
/// unsupported construct gracefully should call [`try_program_cfg_fabric`].
///
/// A `FailWith` op (a bounds / overflow / divide-by-zero / undefined-variable error trap) is REFUSED — a
/// program that reaches one errors in the VM too, so there is no ≡VM value for the fabric to produce. (This
/// is correct behaviour, not a gap: it must not be "trapped" away, or the fabric would return a value where
/// the VM errors. Realistic error-checked programs whose guards PASS carry no reachable `FailWith` and lower.)
pub fn program_cfg_fabric(src: &str, fn_name: &str) -> ProgramCfgFabric {
    try_program_cfg_fabric(src, fn_name).unwrap_or_else(|e| panic!("{e}"))
}

pub fn try_program_cfg_fabric(src: &str, fn_name: &str) -> Result<ProgramCfgFabric, LoweringError> {
    try_cfg_fabric_at(src, Some(fn_name))
}

/// Lower the top-level `## Main` through the CFG engine.
///
/// ★ WHY THIS EXISTS. The fabric lowers branchy control flow — that is what the CFG engine is — but a
/// branchy `## Main` could not reach it. The combinational output-tape path refuses a Main with a branch
/// ("a conditional Show cannot ride the combinational output tape"), and the CFG path is addressed BY
/// FUNCTION NAME, while the top level is not a named function at all. So the capability was present and
/// unreachable for exactly the programs most people write: 80 of 113 source-emittable ops reach the
/// fabric per-program, and the missing third is not missing tiles, it is a missing route.
///
/// The top level is a region of the same `code` array that functions index into, starting at pc 0 with
/// no parameters, so the region bound below — "up to the first function entry, else the end" — already
/// describes it correctly. Nothing about the lowering changes; only how the region is named.
///
/// ★ WHERE IT STOPS TODAY, MEASURED. A branchy Main now REACHES the lowering and refuses at
/// `UnsupportedOp("Show")` rather than at "function not found" — the route is real and the remaining
/// gap is one tile. The CFG engine lowers functions that RETURN a value (`Term::Return`), while `Show`
/// is an in-block side effect the combinational output-tape path models and this one does not. Closing
/// it is a `CfgBlock.shows: Vec<Src>` (serde-skipped when empty, so the chip serializer's bytes do not
/// move), a `Op::Show` arm, and a sibling evaluator returning the shown SEQUENCE instead of the single
/// `i64` `eval_program_cfg_fabric` returns — with the equivalence obligation being the VM's output
/// stream, not just its final value. That is a feature with a proof to discharge, not a patch.
pub fn try_main_cfg_fabric(src: &str) -> Result<ProgramCfgFabric, LoweringError> {
    try_cfg_fabric_at(src, None)
}

fn try_cfg_fabric_at(src: &str, fn_name: Option<&str>) -> Result<ProgramCfgFabric, LoweringError> {
    use crate::vm::wasm::cfg::Blocks;
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::HashMap;

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        // ★ AND A PROGRAM THAT DOES NOT PARSE IS A REFUSAL, NOT A CRASH. This is the same defect as
        // the "function not found" panic below, one step earlier: a coverage probe asks `try_` about
        // every program in a corpus, and such a corpus deliberately contains programs that do not
        // parse or compile. Panicking on them makes the probe itself the thing that fails, which is
        // how this arrived — the matrix's tape route survives them only because it is wrapped in
        // `catch_unwind`, and a fallible function should not need its caller to defend against it.
        let (stmts, types, _p) = match parsed {
            Ok(v) => v,
            Err(e) => {
                return Err(LoweringError::UnsupportedOp(format!(
                    "the program does not parse: {e:?}"
                )))
            }
        };
        let program = match Compiler::compile_with_types(stmts, interner, Some(types)) {
            Ok(p) => p,
            Err(e) => {
                return Err(LoweringError::UnsupportedOp(format!(
                    "the program does not compile: {e:?}"
                )))
            }
        };
        // ★ THE FALLIBLE FORM MUST ACTUALLY FAIL. Naming a function this program does not define is the
        // most ordinary way for a caller to be wrong, and panicking on it means `try_` cannot be used to
        // ASK whether a function is lowerable — which is exactly what a coverage probe wants to do. The
        // top-level `## Main` is the case that matters: it is not a named function at all, so every
        // caller reaching for it took a panic instead of a refusal it could handle.
        // The region to lower: a named function's entry, or the top-level Main at pc 0.
        let (entry_pc, n_params) = match fn_name {
            None => (0usize, 0u16),
            Some(want) => match program
                .functions
                .iter()
                .find(|f| interner.resolve(f.name) == want)
            {
                Some(f) => (f.entry_pc, f.param_count),
                None => {
                    let mut known: Vec<&str> = program
                        .functions
                        .iter()
                        .map(|f| interner.resolve(f.name))
                        .collect();
                    known.sort_unstable();
                    return Err(LoweringError::UnsupportedOp(format!(
                        "no function named `{want}` in this program (it defines: {}) — the top-level `## Main` is lowered by `try_main_cfg_fabric`, not by name",
                        if known.is_empty() { "nothing".to_string() } else { known.join(", ") }
                    )));
                }
            },
        };
        // The float lane tracks which registers hold f64 bit patterns; start fresh (a prior decision-fabric
        // lowering may have left marks) so this function's arithmetic routes to the right units.
        FLOAT_REGS.with(|t| t.borrow_mut().clear());
        FLOAT_COLLS.with(|t| t.borrow_mut().clear());
        TEXT_REGS.with(|t| t.borrow_mut().clear());
        NAME_REGS.with(|t| t.borrow_mut().clear());
        COLL_LEN.with(|t| t.borrow_mut().clear());
        let entry = entry_pc;
        let end = program
            .functions
            .iter()
            .map(|g| g.entry_pc)
            .filter(|&e| e > entry)
            .min()
            .unwrap_or(program.code.len());
        // Region-local ops (jump targets rebased to 0) so the block partition is self-contained.
        let region: Vec<Op> = (entry..end)
            .map(|pc| {
                let mut op = program.code[pc];
                match &mut op {
                    Op::Jump { target }
                    | Op::JumpIfFalse { target, .. }
                    | Op::JumpIfTrue { target, .. } => *target -= entry,
                    _ => {}
                }
                op
            })
            .collect();
        // Back-edges (loops) are allowed: the block-sequencing controller simply RE-ENTERS the loop header
        // block each iteration, carrying loop state in the register file, until the exit branch fires — a
        // `While` loop is a header block (condition → Branch) with a body block whose terminator jumps BACK.
        // The step-budgeted walkers (`eval_program_cfg_fabric` / `run_program_cfg_on_grid` / the whole-program
        // engine) terminate on a returning path; a non-terminating program exhausts the budget and panics
        // loudly, exactly as the VM would run forever. (Range/`for-each` loops still need `IterNext` lowering.)
        let blocks =
            Blocks::new(&region).expect("cfg fabric: function region is not self-contained");
        let nb = blocks.num_blocks();

        // Read a register into the block's SSA space: a register not yet defined in this block is a LIVE-IN,
        // assigned a fresh input slot; a defined one returns its current `Src`.
        let get = |reg: &mut HashMap<u16, Src>, inputs: &mut Vec<u16>, r: u16| -> Src {
            if let Some(s) = reg.get(&r) {
                return *s;
            }
            let k = inputs.len();
            inputs.push(r);
            let s = Src::In(k);
            reg.insert(r, s);
            s
        };
        let cmp_of = |op: &Op| -> Option<CmpKind> {
            Some(match op {
                Op::Lt { .. } => CmpKind::Lt,
                Op::Gt { .. } => CmpKind::Gt,
                Op::LtEq { .. } => CmpKind::Le,
                Op::GtEq { .. } => CmpKind::Ge,
                Op::Eq { .. } => CmpKind::Eq,
                Op::NotEq { .. } => CmpKind::Ne,
                _ => return None,
            })
        };

        let mut out_blocks: Vec<CfgBlock> = Vec::with_capacity(nb);
        for k in 0..nb {
            let bstart = blocks.start(k);
            let bend = blocks.end(k);
            let ops = &region[bstart..bend];
            let (body_ops, term_op): (&[Op], Option<Op>) = match ops.last() {
                // ★ `Halt` TERMINATES A `## Main` THE WAY `Return` TERMINATES A FUNCTION. The top level
                // does not return a value — it ends, and what it produced is the output tape. Without
                // this it fell through to the body match and refused as an unsupported op, which is why
                // every branchy Main stopped here even once the route existed.
                Some(
                    t @ (Op::Jump { .. }
                    | Op::JumpIfFalse { .. }
                    | Op::JumpIfTrue { .. }
                    | Op::Return { .. }
                    | Op::ReturnNothing
                    | Op::Halt),
                ) => (&ops[..ops.len() - 1], Some(*t)),
                _ => (ops, None), // a fall-through block
            };

            let mut reg: HashMap<u16, Src> = HashMap::new();
            let mut inputs: Vec<u16> = Vec::new();
            let mut cells: Vec<LoopCell> = Vec::new();
            let mut shows: Vec<Src> = Vec::new();
            let mut write_order: Vec<u16> = Vec::new();
            let record = |write_order: &mut Vec<u16>, dst: u16| {
                if !write_order.contains(&dst) {
                    write_order.push(dst);
                }
            };
            for op in body_ops {
                forget_overwritten(op, |func| {
                    program
                        .functions
                        .get(func as usize)
                        .map_or(0, |f| f.captures.len())
                });
                if let Some(kind) = cmp_of(op) {
                    if let Op::Lt { dst, lhs, rhs }
                    | Op::Gt { dst, lhs, rhs }
                    | Op::LtEq { dst, lhs, rhs }
                    | Op::GtEq { dst, lhs, rhs }
                    | Op::Eq { dst, lhs, rhs }
                    | Op::NotEq { dst, lhs, rhs } = op
                    {
                        let ca = get(&mut reg, &mut inputs, *lhs);
                        let cb = get(&mut reg, &mut inputs, *rhs);
                        // A 1-char STRING LITERAL in a comparison (`item i of s equals " "`) is compared by BYTE:
                        // replace its pointer with `heap[ptr + 1]` (its single byte), so it matches an indexed
                        // char (also a byte). A char index is `Op::Index` → a plain byte, so only a bare string
                        // register needs this.
                        let byte_of = |cells: &mut Vec<LoopCell>, r: u16, s: Src| -> Src {
                            let one_char = TEXT_REGS.with(|t| t.borrow().contains(&r))
                                && COLL_LEN.with(|t| t.borrow().get(&r).copied() == Some(1));
                            if one_char {
                                let kadd = cells.len();
                                cells.push(LoopCell::Arith {
                                    op: AluOp::Add,
                                    a: s,
                                    b: Src::Const(1),
                                });
                                let kread = cells.len();
                                cells.push(LoopCell::HeapRead {
                                    addr: Src::Val(kadd),
                                });
                                Src::Val(kread)
                            } else {
                                s
                            }
                        };
                        let ca = byte_of(&mut cells, *lhs, ca);
                        let cb = byte_of(&mut cells, *rhs, cb);
                        let (lhs_f, rhs_f) = FLOAT_REGS
                            .with(|t| (t.borrow().contains(lhs), t.borrow().contains(rhs)));
                        if lhs_f || rhs_f {
                            // A FLOAT comparison — promote an integer operand, then apply the genuine IEEE-754
                            // relation on the two `f64` bit patterns (a plain `Select`'s integer bit-pattern
                            // ordering is only sound for same-sign floats). Yields the `0/1` predicate.
                            let a2 = promote_float_operand(&mut cells, ca, lhs_f);
                            let b2 = promote_float_operand(&mut cells, cb, rhs_f);
                            let unit = match kind {
                                CmpKind::Lt => FuncUnit::FLtF,
                                CmpKind::Gt => FuncUnit::FGtF,
                                CmpKind::Le => FuncUnit::FLeF,
                                CmpKind::Ge => FuncUnit::FGeF,
                                CmpKind::Eq => FuncUnit::FEqF,
                                CmpKind::Ne => FuncUnit::FNeF,
                            };
                            let kk = cells.len();
                            cells.push(LoopCell::Func { unit, a: a2, b: b2 });
                            reg.insert(*dst, Src::Val(kk));
                            FLOAT_REGS.with(|t| {
                                t.borrow_mut().remove(dst); // the 0/1 predicate is an Int, not a float
                            });
                        } else {
                            let kk = cells.len();
                            // A comparison yields the 0/1 predicate value the terminator/successor consumes.
                            cells.push(LoopCell::Select {
                                cmp: kind,
                                ca,
                                cb,
                                t: Src::Const(1),
                                e: Src::Const(0),
                            });
                            reg.insert(*dst, Src::Val(kk));
                        }
                        mark_int_reg(*dst); // a 0/1 predicate is an Int
                        record(&mut write_order, *dst);
                    }
                    continue;
                }
                match op {
                    Op::LoadConst { dst, idx } => match &program.constants[*idx as usize] {
                        Constant::Int(n) => {
                            reg.insert(*dst, Src::Const(*n));
                            mark_int_reg(*dst);
                            record(&mut write_order, *dst);
                        }
                        // A Text literal → a char-list `[len, byte0, byte1, ...]` on the heap (UTF-8 bytes, so
                        // `length of` = byte count, matching the VM), exactly as `exec_ops` lowers it. The
                        // register holds the base pointer; `length of s` reads `heap[ptr]`.
                        Constant::Text(s) => {
                            // Reserve growth capacity (like `NewEmptyList`) so a mutable string can be APPENDED to
                            // in place (`Set text to text + ch`), and record it as Text with its static byte-length
                            // (so an `AddAssign` of a known-length string can be lowered to byte writes).
                            let ptr = heap_text_literal(&mut cells, s.as_bytes(), seq_reserve());
                            reg.insert(*dst, ptr);
                            TEXT_REGS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                            COLL_LEN.with(|t| {
                                t.borrow_mut().insert(*dst, s.len() as i64);
                            });
                            record(&mut write_order, *dst);
                        }
                        // A Float literal → its f64 BIT PATTERN as an integer constant, with the register marked
                        // in FLOAT_REGS so downstream arithmetic routes to the float functional units (the float
                        // lane), exactly as the decision-fabric lowering does — so float benchmarks run.
                        Constant::Float(f) => {
                            reg.insert(*dst, Src::Const(f.to_bits() as i64));
                            FLOAT_REGS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                            record(&mut write_order, *dst);
                        }
                        // A Bool rides a scalar slot as 0/1 (exactly as `exec_ops` lowers it) — so `Seq of Bool`
                        // (`sieve`'s flag array) and boolean scalars run.
                        Constant::Bool(b) => {
                            reg.insert(*dst, Src::Const(i64::from(*b)));
                            record(&mut write_order, *dst);
                        }
                        c => panic!("cfg block: unsupported constant {c:?}"),
                    },
                    Op::Move { dst, src } => {
                        let s = get(&mut reg, &mut inputs, *src);
                        reg.insert(*dst, s);
                        // A copy carries float-ness: `dst` holds `src`'s value, so it is a float iff `src` is
                        // (and ceases to be one otherwise — registers are reused across int and float temps).
                        let src_f = FLOAT_REGS.with(|t| t.borrow().contains(src));
                        FLOAT_REGS.with(|t| {
                            let mut s = t.borrow_mut();
                            if src_f {
                                s.insert(*dst);
                            } else {
                                s.remove(dst);
                            }
                        });
                        // …and float-collection-ness (a moved Seq pointer keeps its element type).
                        let src_fc = FLOAT_COLLS.with(|t| t.borrow().contains(src));
                        FLOAT_COLLS.with(|t| {
                            let mut s = t.borrow_mut();
                            if src_fc {
                                s.insert(*dst);
                            } else {
                                s.remove(dst);
                            }
                        });
                        // …and text-ness + static byte-length (a moved string pointer keeps both).
                        let src_txt = TEXT_REGS.with(|t| t.borrow().contains(src));
                        TEXT_REGS.with(|t| {
                            let mut s = t.borrow_mut();
                            if src_txt {
                                s.insert(*dst);
                            } else {
                                s.remove(dst);
                            }
                        });
                        let src_len = COLL_LEN.with(|t| t.borrow().get(src).copied());
                        COLL_LEN.with(|t| {
                            let mut m = t.borrow_mut();
                            match src_len {
                                Some(l) => {
                                    m.insert(*dst, l);
                                }
                                None => {
                                    m.remove(dst);
                                }
                            }
                        });
                        record(&mut write_order, *dst);
                    }
                    Op::Add { dst, lhs, rhs }
                    | Op::Sub { dst, lhs, rhs }
                    | Op::Mul { dst, lhs, rhs } => {
                        let a = get(&mut reg, &mut inputs, *lhs);
                        let b = get(&mut reg, &mut inputs, *rhs);
                        let (lhs_f, rhs_f) = FLOAT_REGS
                            .with(|t| (t.borrow().contains(lhs), t.borrow().contains(rhs)));
                        if lhs_f || rhs_f {
                            // The FLOAT LANE: `f64`-bit-pattern add/sub/mul via the float functional units. An
                            // integer operand is first promoted (`2.0 * k` → the VM's `k as f64`), so both
                            // operands reach the unit as genuine `f64` bit patterns.
                            let a2 = promote_float_operand(&mut cells, a, lhs_f);
                            let b2 = promote_float_operand(&mut cells, b, rhs_f);
                            let unit = match op {
                                Op::Add { .. } => FuncUnit::FAddF,
                                Op::Sub { .. } => FuncUnit::FSubF,
                                _ => FuncUnit::FMulF,
                            };
                            let kk = cells.len();
                            cells.push(LoopCell::Func { unit, a: a2, b: b2 });
                            reg.insert(*dst, Src::Val(kk));
                            FLOAT_REGS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                        } else {
                            let alu = match op {
                                Op::Add { .. } => AluOp::Add,
                                Op::Sub { .. } => AluOp::Sub,
                                _ => AluOp::Mul,
                            };
                            let kk = cells.len();
                            cells.push(LoopCell::Arith { op: alu, a, b });
                            reg.insert(*dst, Src::Val(kk));
                            mark_int_reg(*dst);
                        }
                        record(&mut write_order, *dst);
                    }
                    Op::Div { dst, lhs, rhs }
                    | Op::Mod { dst, lhs, rhs }
                    | Op::FloorDiv { dst, lhs, rhs } => {
                        let a = get(&mut reg, &mut inputs, *lhs);
                        let b = get(&mut reg, &mut inputs, *rhs);
                        let (lhs_f, rhs_f) = FLOAT_REGS
                            .with(|t| (t.borrow().contains(lhs), t.borrow().contains(rhs)));
                        if lhs_f || rhs_f {
                            // The FLOAT LANE division (`Mod`/`FloorDiv` on floats never arise — the VM keeps `%`
                            // integer-only; `/` on a float is true division). Promote an integer operand first.
                            let a2 = promote_float_operand(&mut cells, a, lhs_f);
                            let b2 = promote_float_operand(&mut cells, b, rhs_f);
                            let kk = cells.len();
                            cells.push(LoopCell::Func {
                                unit: FuncUnit::FDivF,
                                a: a2,
                                b: b2,
                            });
                            reg.insert(*dst, Src::Val(kk));
                            FLOAT_REGS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                        } else {
                            let unit = match op {
                                Op::Mod { .. } => FuncUnit::ModS,
                                Op::FloorDiv { .. } => FuncUnit::FloorDivS,
                                _ => FuncUnit::DivS,
                            };
                            let kk = cells.len();
                            cells.push(LoopCell::Func { unit, a, b });
                            reg.insert(*dst, Src::Val(kk));
                            mark_int_reg(*dst);
                        }
                        record(&mut write_order, *dst);
                    }
                    // bitwise / shift → the same combinational ALU tiles (`op.eval`/`op.verilog` are total over
                    // AluOp, so the grid + SAT paths carry these for free — only the lowering was missing).
                    Op::BitAnd { dst, lhs, rhs }
                    | Op::BitOr { dst, lhs, rhs }
                    | Op::BitXor { dst, lhs, rhs }
                    | Op::Shl { dst, lhs, rhs }
                    | Op::Shr { dst, lhs, rhs } => {
                        let a = get(&mut reg, &mut inputs, *lhs);
                        let b = get(&mut reg, &mut inputs, *rhs);
                        let alu = match op {
                            Op::BitAnd { .. } => AluOp::And,
                            Op::BitOr { .. } => AluOp::Or,
                            Op::BitXor { .. } => AluOp::Xor,
                            Op::Shl { .. } => AluOp::Shl,
                            _ => AluOp::Shr,
                        };
                        let kk = cells.len();
                        cells.push(LoopCell::Arith { op: alu, a, b });
                        reg.insert(*dst, Src::Val(kk));
                        mark_int_reg(*dst);
                        record(&mut write_order, *dst);
                    }
                    // exponentiation → the sequential power functional unit (`FuncUnit::Pow`).
                    Op::Pow { dst, lhs, rhs } => {
                        let a = get(&mut reg, &mut inputs, *lhs);
                        let b = get(&mut reg, &mut inputs, *rhs);
                        let kk = cells.len();
                        cells.push(LoopCell::Func {
                            unit: FuncUnit::Pow,
                            a,
                            b,
                        });
                        reg.insert(*dst, Src::Val(kk));
                        mark_int_reg(*dst);
                        record(&mut write_order, *dst);
                    }
                    Op::AddAssign { dst, src } => {
                        let a = get(&mut reg, &mut inputs, *dst);
                        let b = get(&mut reg, &mut inputs, *src);
                        let (dst_f, src_f) = FLOAT_REGS
                            .with(|t| (t.borrow().contains(dst), t.borrow().contains(src)));
                        let dst_txt = TEXT_REGS.with(|t| t.borrow().contains(dst));
                        if dst_txt {
                            // TEXT APPEND — `Set text to text + s` (the compiler folds this to an in-place `+=`).
                            let klen = cells.len();
                            cells.push(LoopCell::HeapRead { addr: a }); // len = heap[text_ptr]
                            let len = Src::Val(klen);
                            if let Some(l) = COLL_LEN.with(|t| t.borrow().get(src).copied()) {
                                // rhs is a STRING of statically-known byte-length L (a literal / a register carrying
                                // one): copy its L bytes just past `text`'s runtime end, then bump the length.
                                for j in 0..l {
                                    let ksrc = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a: b,
                                        b: Src::Const(j + 1),
                                    });
                                    let kbyte = cells.len();
                                    cells.push(LoopCell::HeapRead {
                                        addr: Src::Val(ksrc),
                                    }); // byte=heap[s_ptr+1+j]
                                    let kd1 = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a,
                                        b: Src::Const(j + 1),
                                    });
                                    let kd2 = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a: Src::Val(kd1),
                                        b: len,
                                    });
                                    cells.push(LoopCell::HeapWrite {
                                        addr: Src::Val(kd2),
                                        val: Src::Val(kbyte),
                                    });
                                }
                                let knl = cells.len();
                                cells.push(LoopCell::Arith {
                                    op: AluOp::Add,
                                    a: len,
                                    b: Src::Const(l),
                                });
                                cells.push(LoopCell::HeapWrite {
                                    addr: a,
                                    val: Src::Val(knl),
                                });
                            } else {
                                // rhs is an INT — append its decimal digits (ITOA), all in STATIC cells. `i >= 0` is
                                // assumed (a negative would need a `-` the VM adds → the ≡VM check catches it). Digit
                                // count `D` is a bounded comparison chain; then D positions are written MSB-first.
                                // Positions for `k >= D` (invalid) READ-AND-WRITE-BACK the existing byte — a no-op —
                                // so nothing is ever corrupted (even a clamped/negative address is harmless).
                                const MAXD: i64 = 19; // covers the full i64 range
                                let mut d = Src::Const(1);
                                let mut p10: i64 = 10;
                                for _ in 1..MAXD {
                                    let kge = cells.len();
                                    cells.push(LoopCell::Select {
                                        cmp: CmpKind::Ge,
                                        ca: b,
                                        cb: Src::Const(p10),
                                        t: Src::Const(1),
                                        e: Src::Const(0),
                                    });
                                    let kadd = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a: d,
                                        b: Src::Val(kge),
                                    });
                                    d = Src::Val(kadd);
                                    p10 = p10.saturating_mul(10);
                                }
                                let klpd = cells.len();
                                cells.push(LoopCell::Arith {
                                    op: AluOp::Add,
                                    a: len,
                                    b: d,
                                }); // len + D
                                let lpd = Src::Val(klpd);
                                let mut p10k: i64 = 1;
                                for k in 0..MAXD {
                                    let kdiv = cells.len();
                                    cells.push(LoopCell::Func {
                                        unit: FuncUnit::DivS,
                                        a: b,
                                        b: Src::Const(p10k),
                                    });
                                    let kmod = cells.len();
                                    cells.push(LoopCell::Func {
                                        unit: FuncUnit::ModS,
                                        a: Src::Val(kdiv),
                                        b: Src::Const(10),
                                    });
                                    let kchar = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a: Src::Val(kmod),
                                        b: Src::Const(48),
                                    });
                                    // heap address of digit k (MSB-first): `ptr + (len + D) - k` — the base pointer
                                    // `a` plus the string offset. `k = 0` (LSB) lands at the last slot, `k = D-1`
                                    // (MSB) at the first new slot.
                                    let ksub = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Sub,
                                        a: lpd,
                                        b: Src::Const(k),
                                    });
                                    let kpos = cells.len();
                                    cells.push(LoopCell::Arith {
                                        op: AluOp::Add,
                                        a,
                                        b: Src::Val(ksub),
                                    });
                                    let pos = Src::Val(kpos);
                                    let kvalid = cells.len();
                                    cells.push(LoopCell::Select {
                                        cmp: CmpKind::Lt,
                                        ca: Src::Const(k),
                                        cb: d,
                                        t: Src::Const(1),
                                        e: Src::Const(0),
                                    });
                                    let kexist = cells.len();
                                    cells.push(LoopCell::HeapRead { addr: pos });
                                    let kwval = cells.len();
                                    cells.push(LoopCell::Select {
                                        cmp: CmpKind::Ne,
                                        ca: Src::Val(kvalid),
                                        cb: Src::Const(0),
                                        t: Src::Val(kchar),
                                        e: Src::Val(kexist),
                                    });
                                    cells.push(LoopCell::HeapWrite {
                                        addr: pos,
                                        val: Src::Val(kwval),
                                    });
                                    p10k = p10k.saturating_mul(10);
                                }
                                cells.push(LoopCell::HeapWrite { addr: a, val: lpd });
                                // heap[text_ptr] = len + D
                            }
                            reg.insert(*dst, a); // in-place grow — the string pointer is unchanged
                            COLL_LEN.with(|t| {
                                t.borrow_mut().remove(dst); // its length is now runtime-dependent
                            });
                        } else if dst_f || src_f {
                            // Float accumulate (`Set sum to sum + …` on a float `sum`) — the float lane, with an
                            // integer operand promoted first, exactly like the binary `Add`.
                            let a2 = promote_float_operand(&mut cells, a, dst_f);
                            let b2 = promote_float_operand(&mut cells, b, src_f);
                            let kk = cells.len();
                            cells.push(LoopCell::Func {
                                unit: FuncUnit::FAddF,
                                a: a2,
                                b: b2,
                            });
                            reg.insert(*dst, Src::Val(kk));
                            FLOAT_REGS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                        } else {
                            let kk = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a,
                                b,
                            });
                            reg.insert(*dst, Src::Val(kk));
                            mark_int_reg(*dst);
                        }
                        record(&mut write_order, *dst);
                    }
                    // ── state plane: collection reads/writes lower to `Mem` (heap) + `Arith` tiles, mirroring
                    // the whole-program model's `exec_ops` exactly (so the heap layout matches the VM). ONE
                    // heap is threaded across the block walk (`eval_program_cfg_fabric` / the sequencer's shared
                    // BRAM). Layout `[len, e1..eN]`, 1-based: `length of xs` = heap[ptr]; `item i` = heap[ptr+i].
                    Op::Length { dst, collection } => {
                        let ptr = get(&mut reg, &mut inputs, *collection);
                        let k = cells.len();
                        cells.push(LoopCell::HeapRead { addr: ptr });
                        reg.insert(*dst, Src::Val(k));
                        mark_int_reg(*dst); // a length is an Int
                        record(&mut write_order, *dst);
                    }
                    Op::Index {
                        dst,
                        collection,
                        index,
                    }
                    | Op::IndexUnchecked {
                        dst,
                        collection,
                        index,
                    } => {
                        let ptr = get(&mut reg, &mut inputs, *collection);
                        let idx = get(&mut reg, &mut inputs, *index);
                        let ka = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: ptr,
                            b: idx,
                        });
                        let kr = cells.len();
                        cells.push(LoopCell::HeapRead { addr: Src::Val(ka) });
                        reg.insert(*dst, Src::Val(kr));
                        // The float-in-Seq bridge: `item i of v` off a float Seq yields an f64 bit pattern, so the
                        // read register is float and flows into the float functional units.
                        let coll_f = FLOAT_COLLS.with(|t| t.borrow().contains(collection));
                        FLOAT_REGS.with(|t| {
                            let mut s = t.borrow_mut();
                            if coll_f {
                                s.insert(*dst);
                            } else {
                                s.remove(dst);
                            }
                        });
                        record(&mut write_order, *dst);
                    }
                    Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } => {
                        let n = *count;
                        // A literal list of floats (`[1.0, 2.0, ...]`) is a float-element collection.
                        if (0..n).any(|j| FLOAT_REGS.with(|t| t.borrow().contains(&(*start + j)))) {
                            FLOAT_COLLS.with(|t| {
                                t.borrow_mut().insert(*dst);
                            });
                        }
                        let ka = cells.len();
                        cells.push(LoopCell::HeapAlloc {
                            size: Src::Const(n as i64 + 1),
                        });
                        let ptr = Src::Val(ka);
                        cells.push(LoopCell::HeapWrite {
                            addr: ptr,
                            val: Src::Const(n as i64),
                        });
                        for j in 0..n {
                            let elem = get(&mut reg, &mut inputs, *start + j);
                            let kadd = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: ptr,
                                b: Src::Const(j as i64 + 1),
                            });
                            cells.push(LoopCell::HeapWrite {
                                addr: Src::Val(kadd),
                                val: elem,
                            });
                        }
                        reg.insert(*dst, ptr);
                        record(&mut write_order, *dst);
                    }
                    Op::Args { dst } => {
                        let ptr = heap_argv(&mut cells, seq_reserve());
                        reg.insert(*dst, ptr);
                        TEXT_REGS.with(|t| {
                            t.borrow_mut().remove(dst);
                        });
                        record(&mut write_order, *dst);
                    }
                    Op::NewEmptyListFixed { dst, capacity } => {
                        // A proved fixed allocation owns its full buffer even under a
                        // smaller general collection reserve.
                        let ka = cells.len();
                        cells.push(LoopCell::HeapAlloc {
                            size: Src::Const(i64::from(*capacity) + 1),
                        });
                        let ptr = Src::Val(ka);
                        cells.push(LoopCell::HeapWrite {
                            addr: ptr,
                            val: Src::Const(0),
                        });
                        reg.insert(*dst, ptr);
                        record(&mut write_order, *dst);
                    }
                    Op::NewEmptyList { dst }
                    | Op::NewEmptyListWithCapacity { dst, .. }
                    | Op::NewEmptyListI32 { dst }
                    | Op::NewEmptySet { dst }
                    | Op::NewEmptyMap { dst }
                    | Op::NewEmptyMapWithCapacity { dst, .. } => {
                        // Reserve the collection's own capacity block up front, so it grows within itself and
                        // several collections can grow concurrently without colliding (see `SEQ_RESERVE`).
                        let ka = cells.len();
                        cells.push(LoopCell::HeapAlloc {
                            size: Src::Const(1 + seq_reserve()),
                        });
                        let ptr = Src::Val(ka);
                        cells.push(LoopCell::HeapWrite {
                            addr: ptr,
                            val: Src::Const(0),
                        });
                        reg.insert(*dst, ptr);
                        record(&mut write_order, *dst);
                    }
                    Op::DestructureTuple { src, start, count } => {
                        let ptr = get(&mut reg, &mut inputs, *src);
                        for j in 0..*count {
                            let kadd = cells.len();
                            cells.push(LoopCell::Arith {
                                op: AluOp::Add,
                                a: ptr,
                                b: Src::Const(j as i64 + 1),
                            });
                            let kr = cells.len();
                            cells.push(LoopCell::HeapRead {
                                addr: Src::Val(kadd),
                            });
                            reg.insert(*start + j, Src::Val(kr));
                            record(&mut write_order, *start + j);
                        }
                    }
                    Op::SetIndex {
                        collection,
                        index,
                        value,
                    }
                    | Op::SetIndexUnchecked {
                        collection,
                        index,
                        value,
                    } => {
                        // Storing a float into a slot marks the collection float-element-typed.
                        if FLOAT_REGS.with(|t| t.borrow().contains(value)) {
                            FLOAT_COLLS.with(|t| {
                                t.borrow_mut().insert(*collection);
                            });
                        }
                        let ptr = get(&mut reg, &mut inputs, *collection);
                        let idx = get(&mut reg, &mut inputs, *index);
                        let val = get(&mut reg, &mut inputs, *value);
                        let kadd = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: ptr,
                            b: idx,
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: Src::Val(kadd),
                            val,
                        });
                    }
                    // `m contains k` on a Map/Set — the DIRECT-ADDRESS table `Set at`/`item` already use (heap[ptr
                    // + key]); membership is `heap[ptr + key] != 0`, matching the VM whenever stored values are
                    // non-zero (two_sum stores `1`) — the same model that makes `collect`'s map get/set run ≡VM.
                    // A key that maps to a stored `0` would read as absent → the census's ≡VM check catches it.
                    Op::Contains {
                        dst,
                        collection,
                        value,
                    } => {
                        let ptr = get(&mut reg, &mut inputs, *collection);
                        let key = get(&mut reg, &mut inputs, *value);
                        let kaddr = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: ptr,
                            b: key,
                        });
                        let kread = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(kaddr),
                        });
                        let ksel = cells.len();
                        cells.push(LoopCell::Select {
                            cmp: CmpKind::Ne,
                            ca: Src::Val(kread),
                            cb: Src::Const(0),
                            t: Src::Const(1),
                            e: Src::Const(0),
                        });
                        reg.insert(*dst, Src::Val(ksel));
                        mark_int_reg(*dst); // a 0/1 membership flag is an Int
                        record(&mut write_order, *dst);
                    }
                    Op::ListPush { list, value } => {
                        // Pushing a float marks the collection as float-element-typed (the float-in-Seq bridge).
                        if FLOAT_REGS.with(|t| t.borrow().contains(value)) {
                            FLOAT_COLLS.with(|t| {
                                t.borrow_mut().insert(*list);
                            });
                        }
                        let ptr = get(&mut reg, &mut inputs, *list);
                        let val = get(&mut reg, &mut inputs, *value);
                        let klen = cells.len();
                        cells.push(LoopCell::HeapRead { addr: ptr });
                        let len = Src::Val(klen);
                        let kp1 = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: ptr,
                            b: Src::Const(1),
                        });
                        let kaddr = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: Src::Val(kp1),
                            b: len,
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: Src::Val(kaddr),
                            val,
                        });
                        let knl = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: len,
                            b: Src::Const(1),
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: ptr,
                            val: Src::Val(knl),
                        });
                    }
                    Op::ListPop { list, dst } => {
                        let ptr = get(&mut reg, &mut inputs, *list);
                        let klen = cells.len();
                        cells.push(LoopCell::HeapRead { addr: ptr });
                        let len = Src::Val(klen);
                        let kaddr = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Add,
                            a: ptr,
                            b: len,
                        });
                        let kr = cells.len();
                        cells.push(LoopCell::HeapRead {
                            addr: Src::Val(kaddr),
                        });
                        let kdec = cells.len();
                        cells.push(LoopCell::Arith {
                            op: AluOp::Sub,
                            a: len,
                            b: Src::Const(1),
                        });
                        cells.push(LoopCell::HeapWrite {
                            addr: ptr,
                            val: Src::Val(kdec),
                        });
                        reg.insert(*dst, Src::Val(kr));
                        // A popped element off a float Seq is itself a float.
                        let coll_f = FLOAT_COLLS.with(|t| t.borrow().contains(list));
                        FLOAT_REGS.with(|t| {
                            let mut s = t.borrow_mut();
                            if coll_f {
                                s.insert(*dst);
                            } else {
                                s.remove(dst);
                            }
                        });
                        record(&mut write_order, *dst);
                    }
                    // ── the FLOAT→INT bridge: `floor`/`sqrt` over a float-typed register run on the float
                    // functional units, mirroring the decision-fabric lowering. `floor` of a float yields an
                    // Int (dst leaves FLOAT_REGS); `sqrt` yields a Float (dst stays float). `floor` of an Int
                    // is the Int itself. This lets float benchmarks that end in `floor(...)` (their Int result)
                    // run on the whole-program engine ≡ VM.
                    Op::CallBuiltin {
                        dst,
                        builtin,
                        args_start,
                        ..
                    } => {
                        use crate::semantics::builtins::BuiltinId;
                        let a0 = get(&mut reg, &mut inputs, *args_start);
                        let is_f = FLOAT_REGS.with(|t| t.borrow().contains(args_start));
                        match builtin {
                            BuiltinId::Sqrt if is_f => {
                                let k = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FSqrtF,
                                    a: a0,
                                    b: Src::Const(0),
                                });
                                reg.insert(*dst, Src::Val(k));
                                FLOAT_REGS.with(|t| {
                                    t.borrow_mut().insert(*dst);
                                });
                            }
                            BuiltinId::Floor if is_f => {
                                let k = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FFloorF,
                                    a: a0,
                                    b: Src::Const(0),
                                });
                                reg.insert(*dst, Src::Val(k));
                                mark_int_reg(*dst); // FFloorF is the float→INT bridge — clear the stale float tag
                            }
                            BuiltinId::Floor => {
                                reg.insert(*dst, a0);
                                mark_int_reg(*dst); // floor of an Int is the identity Int
                            }
                            // Integer abs/min/max: pure select-DAGs — reuse a negate + `Select` (mux) tile,
                            // no new functional unit. Float forms fall through (typed `UnsupportedBuiltin`).
                            BuiltinId::Abs if !is_f => {
                                let kneg = cells.len();
                                cells.push(LoopCell::Arith {
                                    op: AluOp::Sub,
                                    a: Src::Const(0),
                                    b: a0,
                                });
                                let ksel = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Lt,
                                    ca: a0,
                                    cb: Src::Const(0),
                                    t: Src::Val(kneg),
                                    e: a0,
                                });
                                reg.insert(*dst, Src::Val(ksel));
                                mark_int_reg(*dst);
                            }
                            BuiltinId::Min if !is_f => {
                                let a1 = get(&mut reg, &mut inputs, *args_start + 1);
                                let k = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Lt,
                                    ca: a0,
                                    cb: a1,
                                    t: a0,
                                    e: a1,
                                });
                                reg.insert(*dst, Src::Val(k));
                                mark_int_reg(*dst);
                            }
                            BuiltinId::Max if !is_f => {
                                let a1 = get(&mut reg, &mut inputs, *args_start + 1);
                                let k = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Gt,
                                    ca: a0,
                                    cb: a1,
                                    t: a0,
                                    e: a1,
                                });
                                reg.insert(*dst, Src::Val(k));
                                mark_int_reg(*dst);
                            }
                            // Float abs/min/max: an IEEE compare tile (`FLtF`/`FGtF` → 0/1) drives a `Select`
                            // over the float-bit operands; float negate is `FSubF(0.0, v)` (0.0 bits == 0). The
                            // result stays in the float lane.
                            BuiltinId::Abs => {
                                let kneg = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FSubF,
                                    a: Src::Const(0),
                                    b: a0,
                                });
                                let kcmp = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FLtF,
                                    a: a0,
                                    b: Src::Const(0),
                                });
                                let ksel = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Ne,
                                    ca: Src::Val(kcmp),
                                    cb: Src::Const(0),
                                    t: Src::Val(kneg),
                                    e: a0,
                                });
                                reg.insert(*dst, Src::Val(ksel));
                                FLOAT_REGS.with(|t| {
                                    t.borrow_mut().insert(*dst);
                                });
                            }
                            BuiltinId::Min => {
                                let a1 = get(&mut reg, &mut inputs, *args_start + 1);
                                let kcmp = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FLtF,
                                    a: a0,
                                    b: a1,
                                });
                                let ksel = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Ne,
                                    ca: Src::Val(kcmp),
                                    cb: Src::Const(0),
                                    t: a0,
                                    e: a1,
                                });
                                reg.insert(*dst, Src::Val(ksel));
                                FLOAT_REGS.with(|t| {
                                    t.borrow_mut().insert(*dst);
                                });
                            }
                            BuiltinId::Max => {
                                let a1 = get(&mut reg, &mut inputs, *args_start + 1);
                                let kcmp = cells.len();
                                cells.push(LoopCell::Func {
                                    unit: FuncUnit::FGtF,
                                    a: a0,
                                    b: a1,
                                });
                                let ksel = cells.len();
                                cells.push(LoopCell::Select {
                                    cmp: CmpKind::Ne,
                                    ca: Src::Val(kcmp),
                                    cb: Src::Const(0),
                                    t: a0,
                                    e: a1,
                                });
                                reg.insert(*dst, Src::Val(ksel));
                                FLOAT_REGS.with(|t| {
                                    t.borrow_mut().insert(*dst);
                                });
                            }
                            other => {
                                return Err(LoweringError::UnsupportedBuiltin(format!("{other:?}")))
                            }
                        }
                        record(&mut write_order, *dst);
                    }
                    // ★ `Show v` — APPEND TO THIS BLOCK'S OUTPUT TAPE. The op has no destination
                    // register and writes nothing; it contributes a value to the program's output
                    // sequence at the point control passes through this block. That is the whole
                    // difference between a function (one `Return`) and a `## Main` (a sequence of
                    // `Show`s whose contents depend on which way the branches went).
                    //
                    // Float and text renderings are REFUSED by name, matching the output-tape path's
                    // own refusals: the fabric's tape carries integers, and turning an IEEE double or a
                    // heap string into decimal characters is a separate lane, not something to fake
                    // here. A wrong rendering would be a silently different output stream.
                    Op::Show { src } => {
                        if FLOAT_REGS.with(|t| t.borrow().contains(src)) {
                            return Err(LoweringError::UnsupportedOp(
                                "Show of a float (rendering IEEE→decimal is a separate lane)"
                                    .into(),
                            ));
                        }
                        if TEXT_REGS.with(|t| t.borrow().contains(src)) {
                            return Err(LoweringError::UnsupportedOp(
                                "Show of text (rendering a heap string is a separate lane)".into(),
                            ));
                        }
                        let v = get(&mut reg, &mut inputs, *src);
                        shows.push(v);
                    }
                    other => return Err(LoweringError::UnsupportedOp(other.name().to_string())),
                }
            }

            let term = match term_op {
                Some(Op::Return { src }) => Term::Return(get(&mut reg, &mut inputs, src)),
                // A Main that halts yields no value; its result is the tape it wrote on the way here.
                Some(Op::ReturnNothing) | Some(Op::Halt) => Term::Return(Src::Const(0)),
                Some(Op::Jump { target }) => Term::Jump(blocks.block_of(target)),
                Some(Op::JumpIfFalse { cond, target }) => {
                    // JumpIfFalse: cond FALSE → jump to `target` (else); cond TRUE → fall through (then).
                    let c = get(&mut reg, &mut inputs, cond);
                    Term::Branch {
                        cond: c,
                        then_blk: blocks.block_of(bend),
                        else_blk: blocks.block_of(target),
                    }
                }
                Some(Op::JumpIfTrue { cond, target }) => {
                    let c = get(&mut reg, &mut inputs, cond);
                    Term::Branch {
                        cond: c,
                        then_blk: blocks.block_of(target),
                        else_blk: blocks.block_of(bend),
                    }
                }
                None => Term::Jump(blocks.block_of(bend)), // straight fall-through to the next block
                _ => unreachable!(),
            };

            let writes: Vec<(u16, Src)> = write_order.iter().map(|r| (*r, reg[r])).collect();
            out_blocks.push(CfgBlock {
                inputs,
                body: cells,
                writes,
                shows,
                term,
            });
        }

        Ok(ProgramCfgFabric {
            n_params: n_params as usize,
            blocks: out_blocks,
        })
    })
}

/// Evaluate a whole-function CFG fabric: seed the register file with `params`, then let the block-sequencing
/// controller walk the graph — each block runs its wired residual over its live-in registers, commits its
/// writes to the file, and its terminator predicate picks the successor — until a block returns. Equals the
/// VM interpreting the same function (proven by `fabric_cfg_lock`).
pub fn eval_program_cfg_fabric(pf: &ProgramCfgFabric, params: &[i64]) -> i64 {
    eval_program_cfg_fabric_traced(pf, params).0
}

/// The same walk, reporting the returned value AND the sequence of values SHOWN along the path taken.
///
/// ★ ONE DEFINITION, TWO VIEWS. A `## Main` is judged by its output STREAM, not by a final register, so
/// a fabric lowering of Main is only ≡VM if the shown sequence matches — including the fact that a
/// branchy Main shows different things down different paths. Splitting this into a second evaluator
/// would have put the block-walking rules in two places, and two copies of a rule disagree the first
/// time either moves.
pub fn eval_program_cfg_fabric_traced(pf: &ProgramCfgFabric, params: &[i64]) -> (i64, Vec<i64>) {
    use std::collections::HashMap;
    let mut file: HashMap<u16, i64> = (0..pf.n_params).map(|p| (p as u16, params[p])).collect();
    // ONE heap threaded across every block (the state plane): `Mem` cells read/write/alloc against it,
    // mirroring `eval_grid_with_heap`'s `[len, e1..eN]` layout. A stateless program never touches it.
    let mut heap: Vec<i64> = Vec::new();
    let mut shown: Vec<i64> = Vec::new();
    let mut blk = 0usize;
    for _ in 0..10_000_000 {
        let b = &pf.blocks[blk];
        let inputs: Vec<i64> = b
            .inputs
            .iter()
            .map(|r| *file.get(r).unwrap_or(&0))
            .collect();
        let mut vals: Vec<i64> = Vec::with_capacity(b.body.len());
        for cell in &b.body {
            let v = match cell {
                LoopCell::Arith { op, a, b } => op.eval(a.eval(&inputs, &vals), b.eval(&inputs, &vals)),
                LoopCell::VectorArith { op, ty, a, b } => {
                    op.eval(*ty, a.eval(&inputs, &vals) as u64, b.eval(&inputs, &vals) as u64) as i64
                }
                LoopCell::Func { unit, a, b } => unit.eval(a.eval(&inputs, &vals), b.eval(&inputs, &vals)),
                LoopCell::Select { cmp, ca, cb, t, e } => {
                    if cmp.eval(ca.eval(&inputs, &vals), cb.eval(&inputs, &vals)) {
                        t.eval(&inputs, &vals)
                    } else {
                        e.eval(&inputs, &vals)
                    }
                }
                // State plane: the shared heap threaded across blocks (same layout as `eval_grid_with_heap`).
                LoopCell::HeapRead { addr } => {
                    let a = addr.eval(&inputs, &vals);
                    heap.get(a.max(0) as usize).copied().unwrap_or(0)
                }
                LoopCell::HeapWrite { addr, val } => {
                    let a = addr.eval(&inputs, &vals);
                    let bv = val.eval(&inputs, &vals);
                    let ai = a.max(0) as usize;
                    if ai >= heap.len() {
                        heap.resize(ai + 1, 0);
                    }
                    heap[ai] = bv;
                    bv
                }
                LoopCell::HeapAlloc { size } => {
                    let a = size.eval(&inputs, &vals);
                    let ptr = heap.len() as i64;
                    heap.resize(heap.len() + a.max(0) as usize, 0);
                    ptr
                }
                LoopCell::MemRead { .. } => panic!(
                    "cfg block body cannot contain a loop-sequence MemRead (the induction-counter scratchpad \
                     is the loop fabric's mechanism, distinct from the CFG shared-heap state plane)"
                ),
            };
            vals.push(v);
        }
        // The tape this block contributes, read in the block's own SSA space before the writes commit.
        for src in &b.shows {
            shown.push(src.eval(&inputs, &vals));
        }
        for (r, src) in &b.writes {
            file.insert(*r, src.eval(&inputs, &vals));
        }
        match &b.term {
            Term::Jump(t) => blk = *t,
            Term::Branch {
                cond,
                then_blk,
                else_blk,
            } => {
                blk = if cond.eval(&inputs, &vals) != 0 {
                    *then_blk
                } else {
                    *else_blk
                }
            }
            Term::Return(src) => return (src.eval(&inputs, &vals), shown),
        }
    }
    panic!("cfg fabric: controller did not return within the step budget (a back-edge slipped through?)")
}

/// A counted loop's header at `ops[i]`: an `LtEq { cond, counter, bound }` the next op `JumpIfFalse`s on — the
/// shape the compiler gives `Repeat for v from a to b` over Int bounds, the counter running from its prologue value
/// to `bound` inclusive. Answers `(counter, bound, exit)`, `exit` the pc the loop leaves to.
fn counted_header(ops: &[(usize, crate::vm::Op)], i: usize) -> Option<(u16, u16, usize)> {
    use crate::vm::Op;
    match (ops.get(i).map(|(_, o)| *o), ops.get(i + 1).map(|(_, o)| *o)) {
        (Some(Op::LtEq { dst, lhs, rhs }), Some(Op::JumpIfFalse { cond, target }))
            if cond == dst =>
        {
            Some((lhs, rhs, target))
        }
        _ => None,
    }
}

/// Whether `op` steps a counted loop's `counter` by one, `is_one` naming the registers that hold 1.
fn counted_step(op: &crate::vm::Op, counter: u16, is_one: impl Fn(u16) -> bool) -> bool {
    use crate::vm::Op;
    match *op {
        Op::Add { dst, lhs, rhs } => {
            dst == counter && ((lhs == counter && is_one(rhs)) || (rhs == counter && is_one(lhs)))
        }
        Op::AddAssign { dst, src } => dst == counter && is_one(src),
        _ => false,
    }
}

/// How many trailing ops of a counted loop's `body` are its latch, the loop controller's work rather than the
/// datapath's. The compiler leaves at the last value before stepping — `Lt t = counter < bound` and a
/// `JumpIfFalse t` to the loop's `exit`, ahead of the unit step — so no backend forms `bound + 1`; the fabric's
/// controllers stop the same way (`if (iv < hi) iv <= iv + 1; else running <= 0`). 3 for that guarded step, 1 for a
/// bare unit step, 0 when the body does not end in the counter's step.
fn counted_latch(
    body: &[(usize, crate::vm::Op)],
    counter: u16,
    bound: u16,
    exit: usize,
    is_one: impl Fn(u16) -> bool,
) -> usize {
    use crate::vm::Op;
    let n = body.len();
    if n == 0 || !counted_step(&body[n - 1].1, counter, &is_one) {
        return 0;
    }
    if n >= 3 {
        if let (Op::Lt { dst, lhs, rhs }, Op::JumpIfFalse { cond, target }) =
            (body[n - 3].1, body[n - 2].1)
        {
            if lhs == counter && rhs == bound && cond == dst && target == exit {
                return 3;
            }
        }
    }
    1
}

/// **The genuine Futamura P1 lowering for a counted loop.** Compile `src`, recover the range
/// loop in function `fn_name` (a counted header and its unit step, or `NewRange`/`IterPrepare`/`IterNext`, and the
/// back-edge `Jump`), and lower
/// it to a [`LoopFabric`]: the loop-carried accumulators (state), the induction variable, the
/// loop invariants, and the straight-line body (SSA). Panics on any shape outside the counted
/// range-loop family (the honest boundary — nested loops / data-dependent exits are future work).
pub fn loop_fabric_from_source(src: &str, fn_name: &str) -> LoopFabric {
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::{BTreeSet, HashMap};

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _pol) = parsed.expect("parse failed");
        let program = Compiler::compile(stmts, interner).expect("compile failed");
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found"));

        // FLOAT LANE: set up the shared thread-locals (clears + seeds `FLOAT_REGS` etc.) exactly like the
        // decision fabric, and seed the float PARAMETERS. The body executor (`exec_ops`) then routes float
        // arithmetic to `FuncUnit::F*` tiles by consulting `FLOAT_REGS`; the prologue below only has to admit
        // float CONSTANTS and carry float-ness through `Move`. Without this a float loop `panic!`ed on `0.0`.
        setup_program_thread_locals(&program);
        for (p, pt) in f.param_types.iter().enumerate() {
            if matches!(pt, Some(crate::vm::BoundaryType::Float)) {
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().insert(p as u16);
                });
            }
        }

        // The function body: entry_pc .. first Return/ReturnNothing.
        let mut ops: Vec<(usize, Op)> = Vec::new();
        for (i, op) in program.code.iter().enumerate().skip(f.entry_pc) {
            ops.push((i, *op));
            if matches!(op, Op::Return { .. } | Op::ReturnNothing) {
                break;
            }
        }

        // Prologue: reg -> Src (circuit-level, over params). Params occupy R0..param_count.
        let mut pro: HashMap<u16, Src> = HashMap::new();
        for p in 0..f.param_count {
            pro.insert(p, Src::In(p as usize));
        }
        let getp = |pro: &HashMap<u16, Src>, r: u16| -> Src {
            *pro.get(&r)
                .unwrap_or_else(|| panic!("loop prologue reads unwritten register R{r}"))
        };

        // Walk the prologue until IterPrepare, capturing the range bounds; then IterNext gives
        // the induction variable and exit; the body runs to the back-edge Jump.
        let mut lo = None;
        let mut hi = None;
        let mut iv_reg = None;
        let mut header_pc = None;
        // For a SEQUENCE loop (`Repeat for x in xs`) there is NO `NewRange`: `IterPrepare` names the
        // iterated sequence register, and `IterNext`'s dst is the ELEMENT `x` (not a numeric index).
        let mut seq_reg: Option<u16> = None;
        // A LOCAL collection the loop iterates is built here, into a prologue HEAP island (evaluated once
        // before the loop); `pro` maps its register to the list pointer (a `Src::Val` into this island).
        let mut prologue_heap: Vec<LoopCell> = Vec::new();
        let mut ppreds: HashMap<u16, (CmpKind, Src, Src)> = HashMap::new();
        // A counted loop's counter: the induction variable, which its unit step at the end of the body advances.
        let mut counter: Option<u16> = None;
        // A counted loop's bound register and exit pc, which its latch names (`counted_latch`).
        let mut latch_bound: Option<(u16, usize)> = None;
        // The registers holding a sequence's length, by the sequence: a counted loop bounded by one, whose body
        // first reads the counter's item of that sequence, is that sequence's loop (`Repeat for x in xs`).
        let mut lengths: HashMap<u16, u16> = HashMap::new();
        let mut by_element = false;
        let mut idx = 0usize;
        while idx < ops.len() {
            let (_pc, op) = ops[idx];
            if let Some((c, bound, exit)) = counted_header(&ops, idx) {
                latch_bound = Some((bound, exit));
                let element = match (lengths.get(&bound), ops.get(idx + 2).map(|(_, o)| *o)) {
                    (
                        Some(&s),
                        Some(
                            Op::Index {
                                dst,
                                collection,
                                index,
                            }
                            | Op::IndexUnchecked {
                                dst,
                                collection,
                                index,
                            },
                        ),
                    ) if collection == s
                        && index == c
                        && matches!(pro.get(&c), Some(Src::Const(1))) =>
                    {
                        Some((s, dst))
                    }
                    _ => None,
                };
                if let Some((s, x)) = element {
                    by_element = true;
                    seq_reg = Some(s);
                    iv_reg = Some(x);
                    header_pc = Some(ops[idx].0);
                    counter = Some(c);
                    idx += 3;
                    break;
                }
                lo = Some(getp(&pro, c));
                hi = Some(getp(&pro, bound));
                iv_reg = Some(c);
                header_pc = Some(ops[idx].0);
                counter = Some(c);
                idx += 2;
                break;
            }
            match op {
                Op::LoadConst { dst, idx: ci } => match program.constants[ci as usize] {
                    Constant::Int(n) => {
                        pro.insert(dst, Src::Const(n));
                    }
                    // A float literal rides its `f64` BIT PATTERN in the scalar slot; `dst` is marked float so
                    // the body's `exec_ops` routes downstream arithmetic to the float functional units.
                    Constant::Float(fv) => {
                        pro.insert(dst, Src::Const(fv.to_bits() as i64));
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(dst);
                        });
                    }
                    ref c => panic!(
                        "loop fabric: non-integer constant {c:?} (only Int / Float supported)"
                    ),
                },
                Op::Move { dst, src } => {
                    let s = getp(&pro, src);
                    pro.insert(dst, s);
                    // Carry float-ness across a prologue copy (so a moved float const stays a float register).
                    if FLOAT_REGS.with(|t| t.borrow().contains(&src)) {
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(dst);
                        });
                    }
                }
                Op::NewRange { start, end, .. } => {
                    lo = Some(getp(&pro, start));
                    hi = Some(getp(&pro, end));
                }
                Op::IterPrepare { iterable } => {
                    seq_reg = Some(iterable);
                }
                Op::Length { dst, collection } => {
                    lengths.insert(dst, collection);
                }
                Op::IterNext { dst, .. } => {
                    iv_reg = Some(dst);
                    header_pc = Some(ops[idx].0); // the loop header the back-edge jumps to
                    idx += 1;
                    break;
                }
                // A local collection the loop iterates (`Let ps be [(1,2),..]`) → build it in the prologue
                // heap island via `exec_ops` (NewTuple/NewList/Push → HeapAlloc/HeapWrite). `pro` records its
                // register as the list pointer (a `Src::Val` into the island). Straight-line only (no branch).
                other => {
                    exec_ops(
                        &[other],
                        &mut pro,
                        &mut prologue_heap,
                        &mut ppreds,
                        &program.constants,
                    );
                }
            }
            idx += 1;
        }
        // A SEQUENCE loop has no `NewRange`; its bound is the iterated sequence's length (derived at eval
        // time). `seq_input` names a memory-array PARAMETER; `seq_heap_ptr` names a LOCAL heap list built in
        // the prologue island (its length is `heap[ptr]`, element `iv` is `heap[ptr + 1 + iv]`).
        let (seq_input, seq_heap_ptr): (Option<usize>, Option<Src>) = if lo.is_none() {
            let sreg = seq_reg.expect("loop fabric: neither NewRange nor IterPrepare (not a loop)");
            match getp(&pro, sreg) {
                Src::In(k) => (Some(k), None),
                s @ Src::Val(_) => (None, Some(s)), // a local heap list built in the prologue island
                other => panic!("loop fabric: the iterated sequence is not a parameter or a local list ({other:?})"),
            }
        } else {
            (None, None)
        };
        let lo = lo.unwrap_or(Src::Const(0));
        let hi = hi.unwrap_or(Src::Const(0));
        let iv_reg = iv_reg.expect("loop fabric: no IterNext (not a loop)");
        let header_pc = header_pc.expect("loop fabric: no loop header");
        // A sequence loop's `IterNext` dst is the ELEMENT, which inherits the sequence's element
        // kind: over a `Seq of Float` parameter it carries an f64 bit pattern, so it must be
        // float-marked or element×element arithmetic (`x * x`) routes to the integer ALU and
        // multiplies bit patterns. (Float CONSTANTS already mark their registers; this is the
        // element-register half of the same discipline.)
        if let Some(k) = seq_input {
            if matches!(
                f.param_types.get(k),
                Some(Some(crate::vm::BoundaryType::Seq(elem)))
                    if matches!(**elem, crate::vm::BoundaryType::Float)
            ) {
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().insert(iv_reg);
                });
            }
        }

        // Body = ops from here to the BACK-EDGE Jump (the one targeting the header), exclusive.
        // Internal jumps (an if/else's then→merge edge) stay in the body — they are the branch
        // structure the if-conversion below consumes. Anything after is IterPop/Return.
        let mut body_ops: Vec<(usize, Op)> = Vec::new();
        let mut result_reg = None;
        while idx < ops.len() {
            let (pc, op) = ops[idx];
            if matches!(op, Op::Jump { target } if target == header_pc) {
                idx += 1;
                break;
            }
            body_ops.push((pc, op));
            idx += 1;
        }
        // A counted loop's latch ends its body: the loop controller advances the counter, not the datapath.
        if let Some(c) = counter {
            let is_one = |r: u16| matches!(pro.get(&r), Some(Src::Const(1)));
            let (bound, exit) = latch_bound.expect("a counted loop has a header");
            match counted_latch(&body_ops, c, bound, exit, is_one) {
                0 => panic!("loop fabric: a counted loop's body does not end in its unit step"),
                n => body_ops.truncate(body_ops.len() - n),
            }
            // A sequence's loop carries its element, not its index: a body that reads the index is outside it.
            if by_element {
                for (_, op) in &body_ops {
                    op.for_each_read_reg(
                        |_| 0,
                        |r| {
                            assert!(
                                r != c,
                                "loop fabric: a sequence loop's body reads its index"
                            );
                        },
                    );
                }
            }
        }
        // Post-loop ops: `IterPop`/`Return` bracket the loop; any straight-line arithmetic between them
        // is the EPILOGUE — a transform island over the loop's final state (the first CFG composition).
        let mut epilogue_ops: Vec<Op> = Vec::new();
        for (_pc, op) in &ops[idx..] {
            match op {
                Op::IterPop | Op::ReturnNothing => {}
                Op::Return { src } => result_reg = Some(*src),
                Op::LoadConst { .. }
                | Op::Move { .. }
                | Op::Add { .. }
                | Op::Sub { .. }
                | Op::Mul { .. }
                | Op::Div { .. }
                | Op::Mod { .. }
                | Op::FloorDiv { .. }
                | Op::AddAssign { .. } => epilogue_ops.push(*op),
                other => panic!(
                    "loop fabric: unexpected post-loop op `{}` (only a straight-line epilogue transform \
                     over the loop result + Return is supported)",
                    other.name()
                ),
            }
        }
        let result_reg = result_reg.expect("loop fabric: loop has no Return");

        // Classify body registers: reads-before-write in the body that are ALSO written are
        // loop-carried; reads-before-write that are NOT written are loop invariants (bound to
        // their prologue value); the iv register is the induction variable.
        let reg_of = |op: &Op| -> (Option<u16>, Vec<u16>) {
            match op {
                Op::LoadConst { dst, .. } => (Some(*dst), vec![]),
                // `now` / `today` — a clock value-load (reads the injected fixed clock); no register read.
                Op::LoadNow { dst } | Op::LoadToday { dst } => (Some(*dst), vec![]),
                Op::Move { dst, src } => (Some(*dst), vec![*src]),
                // Logical `not` → a `(src == 0) ? 1 : 0` Select mux (one read, like `Move`).
                Op::Not { dst, src } => (Some(*dst), vec![*src]),
                // A numeric-coprocessor builtin (sqrt/floor/abs/min/max) reads its `arg_count` argument
                // registers starting at `args_start`; `exec_ops` lowers it to the matching Func/Select tile.
                Op::CallBuiltin { dst, args_start, arg_count, .. } => {
                    (Some(*dst), (0..*arg_count as u16).map(|k| *args_start + k).collect())
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                // Division / mod / floor-div in the loop body: `exec_ops` lowers each to a functional-unit
                // tile (integer `DivS`/`ModS`/`FloorDivS`, or `FDivF` when an operand is a float) — a
                // combinational Func tile, exactly like the epilogue already admits. So `Set s to s / 2.0`
                // is a real FDivF tile, and integer `x % k` / `x // k` counted loops also lower.
                | Op::Div { dst, lhs, rhs }
                | Op::Mod { dst, lhs, rhs }
                | Op::FloorDiv { dst, lhs, rhs }
                // Bitwise / shifts → combinational Arith tiles (integer): `a & b`, `a | b`, `a xor b`,
                // `a << b`, `a >> b`. `exec_ops` already emits the matching `AluOp`.
                | Op::BitAnd { dst, lhs, rhs }
                | Op::BitOr { dst, lhs, rhs }
                | Op::BitXor { dst, lhs, rhs }
                | Op::Shl { dst, lhs, rhs }
                | Op::Shr { dst, lhs, rhs }
                // Power (`**` / `to the power of`) → the sequential-multiply `Pow` functional-unit tile.
                | Op::Pow { dst, lhs, rhs }
                | Op::Lt { dst, lhs, rhs }
                | Op::Gt { dst, lhs, rhs }
                | Op::LtEq { dst, lhs, rhs }
                | Op::GtEq { dst, lhs, rhs }
                | Op::Eq { dst, lhs, rhs }
                | Op::NotEq { dst, lhs, rhs } => (Some(*dst), vec![*lhs, *rhs]),
                Op::AddAssign { dst, src } => (Some(*dst), vec![*dst, *src]),
                // Jumps carry no STATE register; they are the branch structure the if-conversion
                // below consumes. Anything else in a loop body is unsupported (fail loud).
                Op::Jump { .. } | Op::JumpIfFalse { .. } => (None, vec![]),
                other => panic!("loop fabric: body op `{}` unsupported", other.name()),
            }
        };
        let mut written: BTreeSet<u16> = BTreeSet::new();
        let mut upward: BTreeSet<u16> = BTreeSet::new();
        for (_pc, op) in &body_ops {
            // `DestructureTuple` is MULTI-OUTPUT (`start..start+count`) — it reads the element register and
            // writes every binder, so record it directly (a single-`dst` `reg_of` cannot express it).
            if let Op::DestructureTuple { src, start, count } = op {
                if *src != iv_reg && !written.contains(src) {
                    upward.insert(*src);
                }
                for j in 0..*count {
                    written.insert(*start + j);
                }
                continue;
            }
            let (dst, reads) = reg_of(op);
            for r in reads {
                if r != iv_reg && !written.contains(&r) {
                    upward.insert(r);
                }
            }
            if let Some(d) = dst {
                written.insert(d);
            }
        }
        // result_reg is read after the loop → loop-carried if the body writes it.
        let mut carried: Vec<u16> = upward.intersection(&written).copied().collect();
        if written.contains(&result_reg) && !carried.contains(&result_reg) {
            carried.push(result_reg);
        }
        carried.sort_unstable();
        let invariants_regs: Vec<u16> = upward.difference(&written).copied().collect();

        let n_state = carried.len();
        let state_slot: HashMap<u16, usize> =
            carried.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        let inv_slot: HashMap<u16, usize> = invariants_regs
            .iter()
            .enumerate()
            .map(|(i, r)| (*r, i))
            .collect();

        // Seed the body-SSA register map. Body inputs: 0..n_state = state, n_state = iv,
        // n_state+1+j = invariant j.
        let mut reg: HashMap<u16, Src> = HashMap::new();
        for (r, s) in &state_slot {
            reg.insert(*r, Src::In(*s));
        }
        for (r, s) in &inv_slot {
            reg.insert(*r, Src::In(n_state + 1 + *s));
        }
        // Body input `Src::In(n_state)` is the induction variable. For a NUMERIC loop that IS the value
        // `iv_reg` holds. For a SEQUENCE loop it is the ADDRESS (0..len-1) driving a memory-read tile;
        // `iv_reg` (the element `x`) is that tile's result, so we seed cell 0 = `MemRead(addr = iv)`.
        let init_cells: Vec<LoopCell> = if seq_input.is_some() {
            reg.insert(iv_reg, Src::Val(0));
            vec![LoopCell::MemRead {
                addr: Src::In(n_state),
            }]
        } else if seq_heap_ptr.is_some() {
            // FOREACH over a local heap list: element `iv` = `heap[list_ptr + 1 + iv]`. The list pointer is
            // the FINAL invariant slot; `iv_reg` (the element, a tuple ptr) is the seeded HeapRead's result.
            let list_ptr_slot = n_state + 1 + invariants_regs.len();
            reg.insert(iv_reg, Src::Val(2));
            vec![
                LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::In(list_ptr_slot),
                    b: Src::Const(1),
                }, // list_ptr + 1
                LoopCell::Arith {
                    op: AluOp::Add,
                    a: Src::Val(0),
                    b: Src::In(n_state),
                }, // + iv
                LoopCell::HeapRead { addr: Src::Val(1) }, // heap[…] = element
            ]
        } else {
            reg.insert(iv_reg, Src::In(n_state));
            Vec::new()
        };

        // If-CONVERT the body (shared with the while-loop extractor): straight-line ops, and any
        // data-dependent branch becomes a predicated Select (the mux tile) — control becomes data.
        let body = if_convert_body(
            &body_ops,
            &mut reg,
            &carried,
            &program.constants,
            init_cells,
        );

        let init: Vec<Src> = carried.iter().map(|r| getp(&pro, *r)).collect();
        let next: Vec<Src> = carried.iter().map(|&r| *reg.get(&r).unwrap()).collect();
        let mut invariants: Vec<Src> = invariants_regs.iter().map(|r| getp(&pro, *r)).collect();
        // A local-heap-list foreach passes the iterated list's POINTER as the final invariant (the body's
        // element-address generator reads `heap[list_ptr + 1 + iv]`); `init`/`invariants` evaluate over the
        // prologue-heap island in `eval_loop_fabric_heap`.
        if let Some(ptr) = seq_heap_ptr {
            invariants.push(ptr);
        }
        // Build the EPILOGUE island: a straight-line transform over the loop's FINAL state (input
        // convention `Src::In(i)` = final value of state register `i`). Empty when the loop result is a
        // carried register (then `result_state` names it). The epilogue reads state + constants; reading a
        // parameter post-loop is not yet supported (it would need a distinct input slot) and fails loud.
        let (epilogue, epilogue_result, result_state) = if epilogue_ops.is_empty() {
            let rs = *state_slot
                .get(&result_reg)
                .expect("loop fabric: the returned register is not loop-carried");
            (Vec::new(), Src::Const(0), rs)
        } else {
            let mut ereg: HashMap<u16, Src> = HashMap::new();
            for (r, s) in &state_slot {
                ereg.insert(*r, Src::In(*s));
            }
            let mut ecells: Vec<LoopCell> = Vec::new();
            let mut epreds: HashMap<u16, (CmpKind, Src, Src)> = HashMap::new();
            exec_ops(
                &epilogue_ops,
                &mut ereg,
                &mut ecells,
                &mut epreds,
                &program.constants,
            );
            let eres = *ereg
                .get(&result_reg)
                .expect("loop fabric: the epilogue does not define the returned register");
            (ecells, eres, 0usize)
        };

        LoopFabric {
            n_in: f.param_count as usize,
            n_state,
            init,
            lo,
            hi,
            invariants,
            body,
            next,
            result_state,
            seq_input,
            epilogue,
            epilogue_result,
            prologue_heap,
            seq_heap_ptr,
        }
    })
}

/// Evaluate the CGRA op-by-op the way the sequential controller does: initialise the state, then
/// for each induction value in `lo..=hi` apply the body and latch the next state. 64-bit
/// wrapping — bit-identical to the VM's `Int` reduced mod 2⁶⁴. This is the reference the emitted
/// clocked circuit is checked against.
pub fn eval_loop_fabric(lf: &LoopFabric, params: &[i64]) -> i64 {
    if lf.seq_heap_ptr.is_some() {
        eval_loop_fabric_heap(lf, params)
    } else {
        eval_loop_fabric_seq(lf, params, &[])
    }
}

/// Evaluate a FOREACH OVER A LOCAL HEAP LIST (`seq_heap_ptr.is_some()`). The prologue-heap island builds the
/// list into a shared heap ONCE; then for each `iv` in `0..heap[list_ptr]` the body runs with that heap
/// THREADED — the element `heap[list_ptr + 1 + iv]` is destructured via `HeapRead`, and any collection op in
/// the body reads/writes the same heap (so mutation across iterations persists). Bit-identical to the VM.
pub fn eval_loop_fabric_heap(lf: &LoopFabric, params: &[i64]) -> i64 {
    let mut heap: Vec<i64> = Vec::new();
    let pro_vals = eval_cells(&lf.prologue_heap, params, &mut heap);
    let list_ptr = lf
        .seq_heap_ptr
        .as_ref()
        .expect("eval_loop_fabric_heap: not a heap-list loop")
        .eval(params, &pro_vals);
    let len = heap.get(list_ptr as usize).copied().unwrap_or(0);
    // Initial state + invariants evaluate over the PROLOGUE-HEAP island (a carried init or the list ptr may be
    // a `Src::Val` into it); the final invariant is `list_ptr`.
    let mut state: Vec<i64> = lf.init.iter().map(|s| s.eval(params, &pro_vals)).collect();
    let invs: Vec<i64> = lf
        .invariants
        .iter()
        .map(|s| s.eval(params, &pro_vals))
        .collect();
    for iv in 0..len {
        let mut bin: Vec<i64> = Vec::with_capacity(lf.n_state + 1 + invs.len());
        bin.extend_from_slice(&state);
        bin.push(iv);
        bin.extend_from_slice(&invs);
        let vals = eval_cells(&lf.body, &bin, &mut heap); // heap THREADED through the body
        state = lf.next.iter().map(|s| s.eval(&bin, &vals)).collect();
    }
    if lf.epilogue.is_empty() {
        state[lf.result_state]
    } else {
        let evals = eval_cells(&lf.epilogue, &state, &mut heap);
        lf.epilogue_result.eval(&state, &evals)
    }
}

/// Evaluate ONE execution of a loop fabric's body over `bin` (the body-input convention: `0..n_state` =
/// the current state, `n_state` = the induction variable, then the invariants), returning every SSA value.
/// The single oracle every pipelined / metapipelined model of the body is checked against.
pub fn eval_loop_body(lf: &LoopFabric, bin: &[i64], seq: &[i64]) -> Vec<i64> {
    let mut vals: Vec<i64> = Vec::with_capacity(lf.body.len());
    for cell in &lf.body {
        let v = match cell {
            LoopCell::Arith { op, a, b } => op.eval(a.eval(bin, &vals), b.eval(bin, &vals)),
            LoopCell::VectorArith { op, ty, a, b } => {
                op.eval(*ty, a.eval(bin, &vals) as u64, b.eval(bin, &vals) as u64) as i64
            }
            LoopCell::Func { unit, a, b } => unit.eval(a.eval(bin, &vals), b.eval(bin, &vals)),
            LoopCell::MemRead { addr } => {
                let a = addr.eval(bin, &vals);
                seq.get(a as usize).copied().unwrap_or(0)
            }
            LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "heap tile in a loop body is a state-plane follow-on (collection ops inside a loop); \
                 straight-line collection ops evaluate via eval_decision_fabric_heap"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                if cmp.eval(ca.eval(bin, &vals), cb.eval(bin, &vals)) {
                    t.eval(bin, &vals)
                } else {
                    e.eval(bin, &vals)
                }
            }
        };
        vals.push(v);
    }
    vals
}

/// Evaluate a loop fabric, supplying the iterated SEQUENCE's element values when it is a sequence loop
/// (`seq_input.is_some()`). The induction range is then `0..=seq.len()-1` (an empty seq runs zero
/// iterations, exactly like the VM), and each [`LoopCell::MemRead`] reads `seq[addr]`. For a numeric
/// range loop, `seq` is unused and the bounds come from `lo`/`hi`.
pub fn eval_loop_fabric_seq(lf: &LoopFabric, params: &[i64], seq: &[i64]) -> i64 {
    let empty: [i64; 0] = [];
    let mut state: Vec<i64> = lf.init.iter().map(|s| s.eval(params, &empty)).collect();
    let (lo, hi) = if lf.seq_input.is_some() {
        (0i64, seq.len() as i64 - 1) // empty seq → hi == -1 → zero iterations
    } else {
        (lf.lo.eval(params, &empty), lf.hi.eval(params, &empty))
    };
    let invs: Vec<i64> = lf
        .invariants
        .iter()
        .map(|s| s.eval(params, &empty))
        .collect();
    let mut iv = lo;
    while iv <= hi {
        let mut bin: Vec<i64> = Vec::with_capacity(lf.n_state + 1 + invs.len());
        bin.extend_from_slice(&state);
        bin.push(iv);
        bin.extend_from_slice(&invs);
        let vals = eval_loop_body(lf, &bin, seq);
        state = lf.next.iter().map(|s| s.eval(&bin, &vals)).collect();
        if iv == hi {
            break; // avoid i64 overflow at hi == i64::MAX; the VM stops here too
        }
        iv = iv.wrapping_add(1);
    }
    // Post-loop EPILOGUE island: a straight-line transform over the loop's FINAL state (`Src::In(i)` =
    // final state[i], `Src::Val(j)` = epilogue cell j). Empty ⇒ the result is a carried register.
    if lf.epilogue.is_empty() {
        return state[lf.result_state];
    }
    let mut evals: Vec<i64> = Vec::with_capacity(lf.epilogue.len());
    for cell in &lf.epilogue {
        let v = match cell {
            LoopCell::Arith { op, a, b } => op.eval(a.eval(&state, &evals), b.eval(&state, &evals)),
            LoopCell::VectorArith { op, ty, a, b } => op.eval(
                *ty,
                a.eval(&state, &evals) as u64,
                b.eval(&state, &evals) as u64,
            ) as i64,
            LoopCell::Func { unit, a, b } => {
                unit.eval(a.eval(&state, &evals), b.eval(&state, &evals))
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                if cmp.eval(ca.eval(&state, &evals), cb.eval(&state, &evals)) {
                    t.eval(&state, &evals)
                } else {
                    e.eval(&state, &evals)
                }
            }
            LoopCell::MemRead { .. }
            | LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. } => {
                panic!("epilogue tile cannot be a memory/heap tile — no sequence iteration after the loop")
            }
        };
        evals.push(v);
    }
    lf.epilogue_result.eval(&state, &evals)
}

/// Emit the CGRA as a SEQUENTIAL circuit: a combinational body datapath (one `wire` per SSA
/// value + next-state) plus a controller that clocks the loop-carried state through it once per
/// induction value in `lo..=hi`. No fetch, no opcode dispatch: the body is WIRED (spatial), the
/// loop is a counter (temporal). `done` pulses when the loop is finished; `result` is the
/// returned accumulator.
pub fn emit_loop_fabric_verilog(name: &str, lf: &LoopFabric) -> String {
    emit_loop_fabric_verilog_dp(name, lf, None)
}

/// Emit the CGRA RTL with an INT8-NATIVE datapath: every multiply tile's operands are `mul_bits`-bit signed,
/// so each MAC maps to exactly ONE DSP48E1 (a 25×18 signed multiplier) instead of the ~10 DSPs a 64×64
/// multiply decomposes into. The product is sign-extended back to the 64-bit accumulator, so the ADDS and the
/// carried accumulator stay exact 64-bit — the result is byte-identical to the 64-bit fabric (and the VM)
/// whenever the multiply operands fit in `mul_bits` (which the int8 model guarantees, and the ≡VM locks
/// verify). This is the datapath that makes the spatial array DSP-packable and fast on real silicon.
pub fn emit_loop_fabric_verilog_int8(name: &str, lf: &LoopFabric, mul_bits: u32) -> String {
    assert!(
        (2..=32).contains(&mul_bits),
        "int8 datapath mul_bits must be 2..=32 (got {mul_bits})"
    );
    emit_loop_fabric_verilog_dp(name, lf, Some(mul_bits))
}

/// One body cell of a counted-loop CGRA as Verilog wires, the value named `b{i}`: an INT8-NATIVE multiply when
/// `mul_bits` is set, a sequence read, or one arithmetic, select or functional-unit tile.
fn loop_body_wire(
    i: usize,
    cell: &LoopCell,
    mul_bits: Option<u32>,
    name_in: &dyn Fn(usize) -> String,
    name_val: &dyn Fn(usize) -> String,
    mem_addrw: usize,
) -> String {
    let mut out = String::new();
    // INT8-NATIVE MULTIPLY: narrow signed operands → one DSP48E1. Truncating each 64-bit value to its low
    // `w` bits and reading it signed is exact for operands in [-2^(w-1), 2^(w-1)-1] (the int8 range the
    // model lives in); the 2w-bit product then sign-extends into the 64-bit accumulator, so downstream
    // adds/accumulation are unchanged. Verified byte-identical to the 64-bit path (and the VM) by the locks.
    if let (
        Some(w),
        LoopCell::Arith {
            op: AluOp::Mul,
            a,
            b,
        },
    ) = (mul_bits, cell)
    {
        let a_s = src_named(a, name_in, name_val);
        let b_s = src_named(b, name_in, name_val);
        out.push_str(&format!(
            "  wire signed [{h}:0] ma{i} = {a_s};\n  wire signed [{h}:0] mb{i} = {b_s};\n  \
             wire signed [{ph}:0] mp{i} = ma{i} * mb{i};\n  wire [63:0] b{i} = mp{i};\n",
            h = w - 1,
            ph = 2 * w - 1,
        ));
        return out;
    }
    // SEQUENCE-LOOP OPERAND READ: `mem[addr]` from the on-chip operand buffer (addr is the induction
    // counter for `Repeat for x in xs`). Part-select needs a named wire, so bind the address first.
    if let LoopCell::MemRead { addr } = cell {
        let a_s = src_named(addr, name_in, name_val);
        out.push_str(&format!(
            "  wire [63:0] amr{i} = {a_s};\n  wire [63:0] b{i} = mem[amr{i}[{aw}:0]];\n",
            aw = mem_addrw - 1
        ));
        return out;
    }
    let rhs = match cell {
        LoopCell::Arith { op, a, b } => op.verilog(
            &src_named(a, name_in, name_val),
            &src_named(b, name_in, name_val),
        ),
        LoopCell::VectorArith { op, ty, a, b } => op.verilog(
            *ty,
            &src_named(a, name_in, name_val),
            &src_named(b, name_in, name_val),
        ),
        LoopCell::Func { unit, a, b } => unit.verilog(&src_named(a, name_in, name_val), &src_named(b, name_in, name_val)),
        LoopCell::MemRead { .. } => unreachable!("MemRead is emitted before the match"),
        LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
            "Heap-tile RTL emission (a shared-heap read/write inside a loop) is the state-plane \
             follow-on; a sequence loop's operand read lowers via LoopCell::MemRead"
        ),
        LoopCell::Select { cmp, ca, cb, t, e } => format!(
            "($signed({}) {} $signed({})) ? {} : {}",
            src_named(ca, name_in, name_val),
            cmp.verilog(),
            src_named(cb, name_in, name_val),
            src_named(t, name_in, name_val),
            src_named(e, name_in, name_val),
        ),
    };
    out.push_str(&format!("  wire [63:0] b{i} = {rhs};\n"));
    out
}

fn emit_loop_fabric_verilog_dp(name: &str, lf: &LoopFabric, mul_bits: Option<u32>) -> String {
    let name_in = |k: usize| -> String {
        if k < lf.n_state {
            format!("s{k}")
        } else if k == lf.n_state {
            "iv".to_string()
        } else {
            format!("inv{}", k - lf.n_state - 1)
        }
    };
    let name_val = |k: usize| format!("b{k}");
    // Circuit-input renderer for init/lo/hi/invariants (In(k) = in{k} param).
    let circ = |s: &Src| src_named(s, &|k| format!("in{k}"), &|k| format!("v{k}"));

    // A SEQUENCE loop streams an operand vector from an on-chip buffer: the iterated `Seq` input becomes a
    // memory (a write port for loading it over UART/from a ROM) rather than a scalar wire, the induction range
    // is 0..=seq_len-1, and each `MemRead{addr}` reads `mem[addr]`. This is the operand-fed matmul datapath.
    let seq_k = lf.seq_input;
    const MEM_DEPTH: usize = 512;
    const MEM_ADDRW: usize = 9; // clog2(MEM_DEPTH)
    let inputs = (0..lf.n_in)
        .filter(|k| Some(*k) != seq_k)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    let (mem_ports, mem_decl, lo_str, hi_str) = if seq_k.is_some() {
        (
            format!(
                "input [63:0] seq_len, input mem_we, input [{a}:0] mem_waddr, input signed [63:0] mem_wdata, ",
                a = MEM_ADDRW - 1
            ),
            format!(
                "  reg signed [63:0] mem [0:{d}];\n  \
                 always @(posedge clk) if (mem_we) mem[mem_waddr] <= mem_wdata;\n",
                d = MEM_DEPTH - 1
            ),
            "64'sd0".to_string(),
            "$signed(seq_len) - 64'sd1".to_string(),
        )
    } else {
        (String::new(), String::new(), circ(&lf.lo), circ(&lf.hi))
    };

    let mut invs = String::new();
    for (j, s) in lf.invariants.iter().enumerate() {
        invs.push_str(&format!("  wire [63:0] inv{j} = {};\n", circ(s)));
    }
    let mut bodyw = String::new();
    for (i, cell) in lf.body.iter().enumerate() {
        bodyw.push_str(&loop_body_wire(i, cell, mul_bits, &name_in, &name_val, MEM_ADDRW));
    }
    let mut nexts = String::new();
    for (j, s) in lf.next.iter().enumerate() {
        nexts.push_str(&format!(
            "  wire [63:0] ns{j} = {};\n",
            src_named(s, &name_in, &name_val)
        ));
    }
    let mut reset = String::new();
    let mut step = String::new();
    for (j, s) in lf.init.iter().enumerate() {
        reset.push_str(&format!("      s{j} <= {};\n", circ(s)));
        step.push_str(&format!("      s{j} <= ns{j};\n"));
    }
    let state_decl = (0..lf.n_state)
        .map(|j| format!("s{j}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "// cgra_{name} — a compiled COUNTED LOOP as a CGRA: a fixed spatial body datapath\n\
         // (wired SSA, no fetch/dispatch) streamed by a sequential controller once per induction\n\
         // value in lo..=hi. Futamura P1 for control flow: the body IS the circuit; the loop is a\n\
         // counter. 64-bit, so it wraps identically to the VM's Int; the body's one-step next-state\n\
         // is certified == the VM's per-iteration transition by our own SAT (the induction step).\n\
         module cgra_{name}(input clk, input rst, {inputs}{mem_ports}output [63:0] result, output done);\n\
         \x20 reg [63:0] {state_decl};\n\
         \x20 reg [63:0] iv;\n\
         \x20 reg running;\n\
         {mem_decl}\
         \x20 wire [63:0] lo = {lo};\n\
         \x20 wire [63:0] hi = {hi};\n\
         {invs}{bodyw}{nexts}\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         {reset}\
         \x20     iv <= lo;\n\
         \x20     running <= ($signed(lo) <= $signed(hi));\n\
         \x20   end else if (running) begin\n\
         {step}\
         \x20     if ($signed(iv) < $signed(hi)) iv <= iv + 64'sd1;\n\
         \x20     else running <= 1'b0;\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = !running;\n\
         \x20 assign result = s{result_state};\n\
         endmodule\n",
        lo = lo_str,
        hi = hi_str,
        result_state = lf.result_state,
    )
}

/// The terms one accumulator adds each iteration: `next[j] = s_j + Σ ±terms`, through Add/Sub tiles each read
/// once. `None` when the state's next value is not such a sum, or reads the state more than once.
fn accumulation_terms(lf: &LoopFabric, j: usize, uses: &[usize]) -> Option<(Vec<(Src, bool)>, Vec<usize>)> {
    fn walk(
        lf: &LoopFabric,
        s: Src,
        negated: bool,
        uses: &[usize],
        terms: &mut Vec<(Src, bool)>,
        absorbed: &mut Vec<usize>,
    ) {
        if let Src::Val(k) = s {
            if let LoopCell::Arith { op: op @ (AluOp::Add | AluOp::Sub), a, b } = lf.body[k] {
                if uses[k] == 1 {
                    absorbed.push(k);
                    walk(lf, a, negated, uses, terms, absorbed);
                    walk(lf, b, negated != (op == AluOp::Sub), uses, terms, absorbed);
                    return;
                }
            }
        }
        terms.push((s, negated));
    }
    let mut terms = Vec::new();
    let mut absorbed = Vec::new();
    walk(lf, lf.next[j], false, uses, &mut terms, &mut absorbed);
    let own: Vec<usize> = (0..terms.len()).filter(|&t| terms[t].0 == Src::In(j)).collect();
    if own.len() != 1 || terms[own[0]].1 {
        return None;
    }
    terms.remove(own[0]);
    Some((terms, absorbed))
}

/// A counted loop emitted in the pipelined accumulator form.
#[derive(Clone, Debug)]
pub struct PipelinedLoop {
    pub verilog: String,
    /// The register stages the terms take from the counter to the accumulators: a loop whose body runs finishes this
    /// many clocks after the counter does.
    pub term_stages: usize,
}

/// The logic depth of one body tile in the pipelined loop's stage budget, in units of a 64-bit add.
pub fn loop_cell_delay(cell: &LoopCell, mul_bits: Option<u32>) -> f64 {
    match cell {
        LoopCell::Arith { op: AluOp::Mul, .. } => if mul_bits.is_some() { 2.0 } else { 4.0 },
        LoopCell::Arith { op: AluOp::Add | AluOp::Sub, .. } => 1.0,
        LoopCell::Arith { op: AluOp::Shl | AluOp::Shr | AluOp::UShr, b: Src::Const(_), .. } => 0.0,
        LoopCell::Arith { op: AluOp::Shl | AluOp::Shr | AluOp::UShr, .. } => 1.0,
        LoopCell::Arith { op: AluOp::And | AluOp::Or | AluOp::Xor, .. } => 0.3,
        _ => 1.0,
    }
}

/// Emit the counted-loop CGRA with every accumulator in carry-save form and its terms pipelined ahead of it.
///
/// A loop whose state registers each take `s_j ← s_j + Σ ±terms`, with no term reading any state, carries only
/// two recurrences: the induction counter and the accumulators. The terms (operand arithmetic and the multiplies)
/// compute from the counter and the invariants through as many register stages as `term_budget` (in the units of
/// [`loop_cell_delay`]) asks, each tile in the earliest stage its operands and the budget allow; the counter is
/// delayed to every stage that reads it, and a valid flag travels with each iteration. On the edge after the last
/// term stage each accumulator's sum and carry words absorb the terms through a tree of 3:2 compressors, so the
/// recurrence holds no carry chain. The counter increments through a parallel-prefix AND. The result is the sum of
/// the result register's two words, a loop whose body runs finishes [`PipelinedLoop::term_stages`] clocks after the
/// counter does, and every other behaviour is [`emit_loop_fabric_verilog_int8`]'s (with `mul_bits`) or
/// [`emit_loop_fabric_verilog`]'s.
///
/// Refused, with the reason, for a loop with a sequence input, a heap island, an epilogue, a functional-unit tile,
/// or a state that is not such an accumulator.
pub fn emit_loop_fabric_verilog_pipelined(
    name: &str,
    lf: &LoopFabric,
    mul_bits: Option<u32>,
    term_budget: f64,
) -> Result<PipelinedLoop, String> {
    assert!(term_budget > 0.0, "a term stage budget is positive");
    if lf.seq_input.is_some() || lf.seq_heap_ptr.is_some() {
        return Err("a sequence loop streams its operands from memory; the pipelined form takes counted loops".into());
    }
    if !lf.prologue_heap.is_empty() || !lf.epilogue.is_empty() {
        return Err("a loop with a heap prologue or an epilogue island is not a bare accumulation".into());
    }
    if let Some(w) = mul_bits {
        assert!((2..=32).contains(&w), "int8 datapath mul_bits must be 2..=32 (got {w})");
    }
    for (i, c) in lf.body.iter().enumerate() {
        if !matches!(c, LoopCell::Arith { .. } | LoopCell::Select { .. } | LoopCell::VectorArith { .. }) {
            return Err(format!("body tile b{i} ({c:?}) is not a combinational tile"));
        }
    }
    let ns = lf.n_state;
    let mut uses = vec![0usize; lf.body.len()];
    let srcs = |c: &LoopCell| -> Vec<Src> {
        match *c {
            LoopCell::Arith { a, b, .. } | LoopCell::VectorArith { a, b, .. } => vec![a, b],
            LoopCell::Select { ca, cb, t, e, .. } => vec![ca, cb, t, e],
            _ => Vec::new(),
        }
    };
    for c in &lf.body {
        for s in srcs(c) {
            if let Src::Val(k) = s {
                uses[k] += 1;
            }
        }
    }
    for s in &lf.next {
        if let Src::Val(k) = s {
            uses[*k] += 1;
        }
    }
    let mut accumulations = Vec::with_capacity(ns);
    let mut absorbed = vec![false; lf.body.len()];
    for j in 0..ns {
        let (terms, cells) = accumulation_terms(lf, j, &uses)
            .ok_or_else(|| format!("state s{j} is not an accumulator s{j} + Σ terms"))?;
        for k in cells {
            absorbed[k] = true;
        }
        accumulations.push(terms);
    }
    // No term may depend on any state: the terms compute a stage ahead of the accumulators that read them.
    let mut reads_state = vec![false; lf.body.len()];
    for (i, c) in lf.body.iter().enumerate() {
        if absorbed[i] {
            continue;
        }
        reads_state[i] = srcs(c).iter().any(|s| match s {
            Src::In(k) => *k < ns,
            Src::Val(k) => reads_state[*k],
            Src::Const(_) => false,
        });
    }
    for (j, terms) in accumulations.iter().enumerate() {
        for (t, _) in terms {
            let dep = match t {
                Src::In(k) => *k < ns,
                Src::Val(k) => absorbed[*k] || reads_state[*k],
                Src::Const(_) => false,
            };
            if dep {
                return Err(format!("a term of s{j} reads a state register; it cannot run a stage ahead"));
            }
        }
    }

    // Each term tile takes the earliest stage its operands and the budget allow: the counter arrives at stage 0, an
    // invariant is stable in every stage, a value read in a later stage than its own arrives from a register.
    let mut stage = vec![0usize; lf.body.len()];
    let mut arrival = vec![0.0f64; lf.body.len()];
    for (i, c) in lf.body.iter().enumerate() {
        if absorbed[i] {
            continue;
        }
        let ops = srcs(c);
        let latest = ops
            .iter()
            .filter_map(|s| match s {
                Src::Val(k) if !absorbed[*k] => Some(stage[*k]),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let start = ops
            .iter()
            .filter_map(|s| match s {
                Src::Val(k) if !absorbed[*k] && stage[*k] == latest => Some(arrival[*k]),
                _ => None,
            })
            .fold(0.0, f64::max);
        let d = loop_cell_delay(c, mul_bits);
        if start > 0.0 && start + d > term_budget {
            stage[i] = latest + 1;
            arrival[i] = d;
        } else {
            stage[i] = latest;
            arrival[i] = start + d;
        }
    }
    let last = accumulations
        .iter()
        .flatten()
        .filter_map(|(t, _)| match t {
            Src::Val(k) => Some(stage[*k]),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let term_stages = last + 1;

    // How far past its own stage each value and the counter are read: the delay registers they need.
    let mut reach = vec![0usize; lf.body.len()];
    let mut iv_reach = 0usize;
    let note = |s: Src, at: usize, reach: &mut Vec<usize>, iv_reach: &mut usize| match s {
        Src::Val(k) => reach[k] = reach[k].max(at - stage[k]),
        Src::In(k) if k == ns => *iv_reach = (*iv_reach).max(at),
        _ => {}
    };
    for (i, c) in lf.body.iter().enumerate() {
        if !absorbed[i] {
            for s in srcs(c) {
                note(s, stage[i], &mut reach, &mut iv_reach);
            }
        }
    }
    for (t, _) in accumulations.iter().flatten() {
        note(*t, last, &mut reach, &mut iv_reach);
    }
    let delayed = |base: String, d: usize| if d == 0 { base } else { format!("{base}_d{d}") };

    let circ = |s: &Src| src_named(s, &|k| format!("in{k}"), &|k| format!("v{k}"));
    let inputs = (0..lf.n_in)
        .map(|k| format!("input [63:0] in{k}, "))
        .collect::<String>();
    let mut invs = String::new();
    for (j, s) in lf.invariants.iter().enumerate() {
        invs.push_str(&format!("  wire [63:0] inv{j} = {};\n", circ(s)));
    }
    let mut bodyw = String::new();
    for (i, cell) in lf.body.iter().enumerate() {
        if absorbed[i] {
            continue;
        }
        let at = stage[i];
        let name_in = |k: usize| -> String {
            if k < ns {
                format!("s{k}")
            } else if k == ns {
                delayed("iv".into(), at)
            } else {
                format!("inv{}", k - ns - 1)
            }
        };
        let name_val = |k: usize| delayed(format!("b{k}"), at - stage[k]);
        bodyw.push_str(&loop_body_wire(i, cell, mul_bits, &name_in, &name_val, 9));
    }
    let mut pipe_regs = String::new();
    let mut pipe_shift = String::new();
    for d in 1..=iv_reach {
        pipe_regs.push_str(&format!("  reg [63:0] iv_d{d};\n"));
        pipe_shift.push_str(&format!("      iv_d{d} <= {};\n", delayed("iv".into(), d - 1)));
    }
    for (k, &r) in reach.iter().enumerate() {
        for d in 1..=r {
            pipe_regs.push_str(&format!("  reg [63:0] b{k}_d{d};\n"));
            pipe_shift.push_str(&format!("      b{k}_d{d} <= {};\n", delayed(format!("b{k}"), d - 1)));
        }
    }
    let name_in = |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else if k == ns {
            delayed("iv".into(), last)
        } else {
            format!("inv{}", k - ns - 1)
        }
    };
    let name_val = |k: usize| delayed(format!("b{k}"), last - stage[k]);

    // Each term registers into t{j}_{n}; a subtracted one enters its tree inverted, its +1 folded into the
    // constant word.
    let mut regs = String::new();
    let mut latch = String::new();
    let mut trees = String::new();
    let mut accumulate = String::new();
    let mut reset = String::new();
    for (j, terms) in accumulations.iter().enumerate() {
        regs.push_str(&format!("  reg [63:0] s{j}_sum, s{j}_carry;\n"));
        reset.push_str(&format!("      s{j}_sum <= {}; s{j}_carry <= 64'd0;\n", circ(&lf.init[j])));
        let mut words = vec![format!("s{j}_sum"), format!("s{j}_carry")];
        let mut constant: i64 = 0;
        for (n, (t, negated)) in terms.iter().enumerate() {
            match t {
                Src::Const(c) => {
                    constant = if *negated { constant.wrapping_sub(*c) } else { constant.wrapping_add(*c) };
                }
                _ => {
                    regs.push_str(&format!("  reg [63:0] t{j}_{n};\n"));
                    latch.push_str(&format!("      t{j}_{n} <= {};\n", src_named(t, &name_in, &name_val)));
                    if *negated {
                        words.push(format!("~t{j}_{n}"));
                        constant = constant.wrapping_add(1);
                    } else {
                        words.push(format!("t{j}_{n}"));
                    }
                }
            }
        }
        if constant != 0 {
            words.push(format!("64'h{:016x}", constant as u64));
        }
        let mut level = 0usize;
        while words.len() > 2 {
            let mut next = Vec::with_capacity(words.len());
            for (g, t) in words.chunks(3).enumerate() {
                if t.len() < 3 {
                    next.extend(t.iter().cloned());
                    continue;
                }
                let tag = format!("s{j}_l{level}_{g}");
                trees.push_str(&format!(
                    "  wire [63:0] {tag}_x = {a} ^ {b};\n  wire [63:0] {tag}_s = {tag}_x ^ {c};\n  \
                     wire [63:0] {tag}_c = (({a} & {b}) | ({c} & {tag}_x)) << 1;\n",
                    a = t[0],
                    b = t[1],
                    c = t[2],
                ));
                next.push(format!("{tag}_s"));
                next.push(format!("{tag}_c"));
            }
            words = next;
            level += 1;
        }
        trees.push_str(&format!(
            "  wire [63:0] s{j}_sum_next = {};\n  wire [63:0] s{j}_carry_next = {};\n",
            words[0],
            words.get(1).cloned().unwrap_or_else(|| "64'd0".into())
        ));
        accumulate.push_str(&format!("        s{j}_sum <= s{j}_sum_next; s{j}_carry <= s{j}_carry_next;\n"));
    }
    let mut prefix = String::from("  wire [63:0] ivp0 = iv;\n");
    let mut level = 0;
    let mut span = 1;
    while span < 64 {
        prefix.push_str(&format!(
            "  wire [63:0] ivp{n} = ivp{level} & {{ivp{level}[{hi}:0], {span}'b{ones}}};\n",
            n = level + 1,
            hi = 63 - span,
            ones = "1".repeat(span),
        ));
        level += 1;
        span *= 2;
    }
    prefix.push_str(&format!("  wire [63:0] iv_next = iv ^ {{ivp{level}[62:0], 1'b1}};\n"));
    let state_reads = (0..ns)
        .map(|j| format!("  wire [63:0] s{j} = s{j}_sum + s{j}_carry;\n"))
        .collect::<String>();
    Ok(format!(
        "// cgra_{name} — a compiled COUNTED LOOP as a CGRA, pipelined: the body's terms compute from the induction\n\
         // counter and the invariants through {term_stages} register stage(s); each accumulator holds sum and carry words\n\
         // that absorb them through 3:2 compressors, so neither recurrence carries a carry chain. 64-bit, so it wraps\n\
         // identically to the VM's Int; a loop whose body runs finishes {term_stages} clock(s) after the counter does.\n\
         module cgra_{name}(input clk, input rst, {inputs}output [63:0] result, output done);\n\
         {regs}\
         \x20 reg [63:0] iv;\n\
         \x20 reg running;\n\
         \x20 reg [{live_top}:0] live;\n\
         {pipe_regs}\
         \x20 wire [63:0] lo = {lo};\n\
         \x20 wire [63:0] hi = {hi};\n\
         {invs}{bodyw}{trees}{prefix}{state_reads}\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         {reset}\
         \x20     iv <= lo;\n\
         \x20     running <= ($signed(lo) <= $signed(hi));\n\
         \x20     live <= {{{live_width}{{1'b0}}}};\n\
         \x20   end else begin\n\
         \x20     live <= {live_next};\n\
         {pipe_shift}{latch}\
         \x20     if (live[{live_top}]) begin\n\
         {accumulate}\
         \x20     end\n\
         \x20     if (running) begin\n\
         \x20       if ($signed(iv) < $signed(hi)) iv <= iv_next;\n\
         \x20       else running <= 1'b0;\n\
         \x20     end\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = !running && !(|live);\n\
         \x20 assign result = s{result_state};\n\
         endmodule\n",
        lo = circ(&lf.lo),
        hi = circ(&lf.hi),
        result_state = lf.result_state,
        live_top = term_stages - 1,
        live_width = term_stages,
        live_next = if term_stages == 1 {
            "running".to_string()
        } else {
            format!("{{live[{}:0], running}}", term_stages - 2)
        },
    ))
    .map(|verilog| PipelinedLoop { verilog, term_stages })
}

/// Lower ONE loop-carried state register's next-state (`next[which]`) to a `BoundedExpr` over the
/// body inputs `s0..`, `iv`, `inv0..` at the given width — so our certified SAT can prove the
/// induction STEP (the body transition ≡ the VM's per-iteration semantics) exhaustively.
pub fn loop_next_to_bounded(
    lf: &LoopFabric,
    which: usize,
    width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    let name_in = |k: usize| -> String {
        if k < lf.n_state {
            format!("s{k}")
        } else if k == lf.n_state {
            "iv".to_string()
        } else {
            format!("inv{}", k - lf.n_state - 1)
        }
    };
    fn src_expr(
        s: &Src,
        vals: &[crate::codegen_sva::sva_to_verify::BoundedExpr],
        width: u32,
        name_in: &dyn Fn(usize) -> String,
    ) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
        use crate::codegen_sva::sva_to_verify::BoundedExpr;
        match s {
            Src::In(k) => BoundedExpr::BitVecVar(name_in(*k), width),
            Src::Val(k) => vals[*k].clone(),
            Src::Const(c) => BoundedExpr::BitVecConst {
                width,
                value: *c as u64,
            },
        }
    }
    let slt = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::SLt,
        left: Box::new(l),
        right: Box::new(r),
    };
    let eq = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(l),
        right: Box::new(r),
    };
    let mut vals: Vec<BoundedExpr> = Vec::with_capacity(lf.body.len());
    for cell in &lf.body {
        let expr = match cell {
            LoopCell::Arith { op, a, b } => {
                let a = src_expr(a, &vals, width, &name_in);
                let b = src_expr(b, &vals, width, &name_in);
                let bop = match op {
                    AluOp::Add => BitVecBoundedOp::Add,
                    AluOp::Sub => BitVecBoundedOp::Sub,
                    AluOp::Mul => BitVecBoundedOp::Mul,
                    AluOp::And => BitVecBoundedOp::And,
                    AluOp::Or => BitVecBoundedOp::Or,
                    AluOp::Xor => BitVecBoundedOp::Xor,
                    AluOp::Shl => BitVecBoundedOp::Shl,
                    AluOp::Shr => BitVecBoundedOp::AShr,
                    AluOp::UShr => BitVecBoundedOp::Shr,
                };
                BoundedExpr::BitVecBinary { op: bop, left: Box::new(a), right: Box::new(b) }
            }
            LoopCell::VectorArith { .. } => panic!(
                "a packed-SIMD (Simd) tile in a loop SAT bound is not SAT-lowered here — a Simd-bearing fabric \
                 is proven by eval ≡ VM + iverilog (simd_tile_lock + simd_rtl_lock), like a Func tile"
            ),
            LoopCell::Func { .. } => panic!(
                "division in a counted-loop body is not yet SAT-lowered — the shared_divider's \
                 own certified proof (rtl.rs::the_signed_divider_is_proven_correct_by_our_sat) covers \
                 the arithmetic; loop-carried division SAT is future work"
            ),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead in a loop-carried SAT bound is future work (the memory-loop induction proof)"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let ca = src_expr(ca, &vals, width, &name_in);
                let cb = src_expr(cb, &vals, width, &name_in);
                let t = src_expr(t, &vals, width, &name_in);
                let e = src_expr(e, &vals, width, &name_in);
                // Normalize the SIGNED comparison to `{SLt, Eq}` with a possible branch-swap
                // (`ite(¬p, t, e) = ite(p, e, t)`), so the predicate needs no `Not`.
                let (cond, tt, ee) = match cmp {
                    CmpKind::Lt => (slt(ca, cb), t, e),
                    CmpKind::Gt => (slt(cb, ca), t, e),
                    CmpKind::Ge => (slt(ca, cb), e, t),
                    CmpKind::Le => (slt(cb, ca), e, t),
                    CmpKind::Eq => (eq(ca, cb), t, e),
                    CmpKind::Ne => (eq(ca, cb), e, t),
                };
                BoundedExpr::BitVecIte {
                    cond: Box::new(cond),
                    then: Box::new(tt),
                    else_: Box::new(ee),
                }
            }
        };
        vals.push(expr);
    }
    src_expr(&lf.next[which], &vals, width, &name_in)
}

// ===========================================================================================
// WHILE-loop CGRA — the DATA-DEPENDENT controller: the loop's continue condition is computed by
// the FABRIC (a predicate over the loop-carried state), not a counter. The trip count is decided
// by the datapath itself. Sound by k-induction: body-step ≡ VM iteration AND the fabric predicate
// ≡ the VM's loop condition (both certified by our SAT) ⇒ the fabric runs exactly the VM's
// iterations. Reuses the shared if-conversion (`if_convert_body`) so a `while` body may branch.
// ===========================================================================================

/// A `while`-loop residual lowered to a CGRA: a fixed body datapath streamed by a controller whose
/// CONTINUE predicate is fabric-computed over the loop-carried state (no counter).
#[derive(Clone, Debug)]
pub struct WhileFabric {
    pub n_in: usize,
    pub n_state: usize,
    pub init: Vec<Src>,
    pub invariants: Vec<Src>,
    /// Body cells. `Src::In(k)`: `0..n_state` = state, `n_state+j` = invariant j (NO induction var).
    pub body: Vec<LoopCell>,
    pub next: Vec<Src>,
    /// The continue predicate `(ca cmp cb)` — the loop runs WHILE it holds. Over body inputs.
    pub pred: (CmpKind, Src, Src),
    pub result_state: usize,
}

/// Recover the `while` loop in `fn_name` and lower it to a [`WhileFabric`]. The header (the
/// back-edge target) computes the continue predicate; the body (to the back-edge) if-converts.
pub fn while_fabric_from_source(src: &str, fn_name: &str) -> WhileFabric {
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::{BTreeSet, HashMap};

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parse failed");
        let program = Compiler::compile(stmts, interner).expect("compile failed");
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found"));

        // FLOAT LANE (mirrors the loop fabric): set up the shared thread-locals + seed the float parameters,
        // so the body's `exec_ops` routes float arithmetic to `FuncUnit::F*` tiles. The prologue below then
        // admits float CONSTANTS and carries float-ness through `Move`.
        setup_program_thread_locals(&program);
        for (p, pt) in f.param_types.iter().enumerate() {
            if matches!(pt, Some(crate::vm::BoundaryType::Float)) {
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().insert(p as u16);
                });
            }
        }

        let mut ops: Vec<(usize, Op)> = Vec::new();
        for (i, op) in program.code.iter().enumerate().skip(f.entry_pc) {
            ops.push((i, *op));
            if matches!(op, Op::Return { .. } | Op::ReturnNothing) {
                break;
            }
        }
        // Back-edge = a Jump targeting an EARLIER pc; its target is the loop header.
        let (backedge_idx, header_pc) = ops
            .iter()
            .enumerate()
            .find_map(|(i, (pc, op))| match op {
                Op::Jump { target } if *target < *pc => Some((i, *target)),
                _ => None,
            })
            .expect("while fabric: no back-edge (not a while loop)");

        let mut pro: HashMap<u16, Src> = HashMap::new();
        for p in 0..f.param_count {
            pro.insert(p, Src::In(p as usize));
        }
        let getp = |pro: &HashMap<u16, Src>, r: u16| -> Src {
            *pro.get(&r)
                .unwrap_or_else(|| panic!("while prologue reads unwritten R{r}"))
        };
        // Prologue: everything before the header initialises the loop-carried registers.
        let mut header_start = 0;
        for (i, (pc, op)) in ops.iter().enumerate() {
            if *pc == header_pc {
                header_start = i;
                break;
            }
            match op {
                Op::LoadConst { dst, idx: ci } => match program.constants[*ci as usize] {
                    Constant::Int(n) => {
                        pro.insert(*dst, Src::Const(n));
                    }
                    Constant::Float(fv) => {
                        pro.insert(*dst, Src::Const(fv.to_bits() as i64));
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(*dst);
                        });
                    }
                    ref c => panic!(
                        "while prologue: non-integer constant {c:?} (only Int / Float supported)"
                    ),
                },
                Op::Move { dst, src } => {
                    let s = getp(&pro, *src);
                    if FLOAT_REGS.with(|t| t.borrow().contains(src)) {
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(*dst);
                        });
                    }
                    pro.insert(*dst, s);
                }
                other => panic!("while prologue: `{}` unsupported", other.name()),
            }
        }
        // Header [header_start .. jif) computes the predicate; body (jif .. back-edge) is the loop.
        let jif_idx = ops[header_start..backedge_idx]
            .iter()
            .position(|(_, o)| matches!(o, Op::JumpIfFalse { .. }))
            .map(|x| x + header_start)
            .expect("while fabric: no JumpIfFalse header test");
        let (cond_reg, exit_target) = match ops[jif_idx].1 {
            Op::JumpIfFalse { cond, target } => (cond, target),
            _ => unreachable!(),
        };
        let header_ops: Vec<Op> = ops[header_start..jif_idx].iter().map(|(_, o)| *o).collect();
        let body_ops: Vec<(usize, Op)> = ops[jif_idx + 1..backedge_idx].to_vec();
        let result_reg = ops[backedge_idx + 1..]
            .iter()
            .find_map(|(pc, o)| match o {
                Op::Return { src } if *pc == exit_target => Some(*src),
                _ => None,
            })
            .or_else(|| {
                ops.iter().find_map(|(_, o)| match o {
                    Op::Return { src } => Some(*src),
                    _ => None,
                })
            })
            .expect("while fabric: no Return");

        let reg_of = |op: &Op| -> (Option<u16>, Vec<u16>) {
            match op {
                Op::LoadConst { dst, .. } => (Some(*dst), vec![]),
                // `now` / `today` — a clock value-load (reads the injected fixed clock); no register read.
                Op::LoadNow { dst } | Op::LoadToday { dst } => (Some(*dst), vec![]),
                Op::Move { dst, src } => (Some(*dst), vec![*src]),
                // Logical `not` → a `(src == 0) ? 1 : 0` Select mux (one read, like `Move`).
                Op::Not { dst, src } => (Some(*dst), vec![*src]),
                // A numeric-coprocessor builtin (sqrt/floor/abs/min/max) reads its `arg_count` argument
                // registers starting at `args_start`; `exec_ops` lowers it to the matching Func/Select tile.
                Op::CallBuiltin { dst, args_start, arg_count, .. } => {
                    (Some(*dst), (0..*arg_count as u16).map(|k| *args_start + k).collect())
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                // Division / mod / floor-div → a functional-unit tile (int `DivS`/`ModS`/`FloorDivS`, or
                // `FDivF` when an operand is a float), so a while body's `s / 2.0` is a real FDivF tile.
                | Op::Div { dst, lhs, rhs }
                | Op::Mod { dst, lhs, rhs }
                | Op::FloorDiv { dst, lhs, rhs }
                // Bitwise / shifts → combinational Arith tiles (integer): `a & b`, `a | b`, `a xor b`,
                // `a << b`, `a >> b`. `exec_ops` already emits the matching `AluOp`.
                | Op::BitAnd { dst, lhs, rhs }
                | Op::BitOr { dst, lhs, rhs }
                | Op::BitXor { dst, lhs, rhs }
                | Op::Shl { dst, lhs, rhs }
                | Op::Shr { dst, lhs, rhs }
                // Power (`**` / `to the power of`) → the sequential-multiply `Pow` functional-unit tile.
                | Op::Pow { dst, lhs, rhs }
                | Op::Lt { dst, lhs, rhs }
                | Op::Gt { dst, lhs, rhs }
                | Op::LtEq { dst, lhs, rhs }
                | Op::GtEq { dst, lhs, rhs }
                | Op::Eq { dst, lhs, rhs }
                | Op::NotEq { dst, lhs, rhs } => (Some(*dst), vec![*lhs, *rhs]),
                Op::AddAssign { dst, src } => (Some(*dst), vec![*dst, *src]),
                Op::Jump { .. } | Op::JumpIfFalse { .. } => (None, vec![]),
                other => panic!("while loop: body op `{}` unsupported", other.name()),
            }
        };
        let mut written: BTreeSet<u16> = BTreeSet::new();
        let mut upward: BTreeSet<u16> = BTreeSet::new();
        for op in header_ops.iter().chain(body_ops.iter().map(|(_, o)| o)) {
            let (dst, reads) = reg_of(op);
            for r in reads {
                if !written.contains(&r) {
                    upward.insert(r);
                }
            }
            if let Some(d) = dst {
                written.insert(d);
            }
        }
        let mut carried: Vec<u16> = upward.intersection(&written).copied().collect();
        if written.contains(&result_reg) && !carried.contains(&result_reg) {
            carried.push(result_reg);
        }
        carried.sort_unstable();
        let invariants_regs: Vec<u16> = upward.difference(&written).copied().collect();

        let n_state = carried.len();
        let state_slot: HashMap<u16, usize> =
            carried.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        let inv_slot: HashMap<u16, usize> = invariants_regs
            .iter()
            .enumerate()
            .map(|(i, r)| (*r, i))
            .collect();

        // Body inputs: 0..n_state = state, n_state+j = invariant j (NO induction variable).
        let mut reg: HashMap<u16, Src> = HashMap::new();
        for (r, s) in &state_slot {
            reg.insert(*r, Src::In(*s));
        }
        for (r, s) in &inv_slot {
            reg.insert(*r, Src::In(n_state + *s));
        }

        // The continue predicate: run the header straight-line (it must produce no datapath cells)
        // and read off the comparison feeding the JumpIfFalse.
        let mut hdr_cells: Vec<LoopCell> = Vec::new();
        let mut preds: HashMap<u16, (CmpKind, Src, Src)> = HashMap::new();
        let mut hdr_reg = reg.clone();
        exec_ops(
            &header_ops,
            &mut hdr_reg,
            &mut hdr_cells,
            &mut preds,
            &program.constants,
        );
        assert!(
            hdr_cells.is_empty(),
            "while fabric: arithmetic in the loop predicate is unsupported (G3 scope)"
        );
        let pred = *preds
            .get(&cond_reg)
            .expect("while fabric: the header test is not a comparison");

        let body = if_convert_body(
            &body_ops,
            &mut reg,
            &carried,
            &program.constants,
            Vec::new(),
        );
        let init: Vec<Src> = carried.iter().map(|r| getp(&pro, *r)).collect();
        let next: Vec<Src> = carried.iter().map(|&r| *reg.get(&r).unwrap()).collect();
        let invariants: Vec<Src> = invariants_regs.iter().map(|r| getp(&pro, *r)).collect();
        let result_state = *state_slot
            .get(&result_reg)
            .expect("while fabric: the returned register is not loop-carried");

        WhileFabric {
            n_in: f.param_count as usize,
            n_state,
            init,
            invariants,
            body,
            next,
            pred,
            result_state,
        }
    })
}

/// Evaluate the while-CGRA: initialise the state, then WHILE the fabric predicate holds apply the
/// body and latch. 64-bit wrapping — the VM's `Int` mod 2⁶⁴. A guard bounds non-termination.
pub fn eval_while_fabric(wf: &WhileFabric, params: &[i64]) -> i64 {
    let empty: [i64; 0] = [];
    let mut state: Vec<i64> = wf.init.iter().map(|s| s.eval(params, &empty)).collect();
    let invs: Vec<i64> = wf
        .invariants
        .iter()
        .map(|s| s.eval(params, &empty))
        .collect();
    let holds = |state: &[i64]| -> bool {
        let mut bin: Vec<i64> = state.to_vec();
        bin.extend_from_slice(&invs);
        let (cmp, ca, cb) = &wf.pred;
        cmp.eval(ca.eval(&bin, &empty), cb.eval(&bin, &empty))
    };
    let mut guard: u64 = 0;
    while holds(&state) {
        let mut bin: Vec<i64> = state.clone();
        bin.extend_from_slice(&invs);
        let mut vals: Vec<i64> = Vec::with_capacity(wf.body.len());
        for cell in &wf.body {
            let v = match cell {
                LoopCell::Arith { op, a, b } => op.eval(a.eval(&bin, &vals), b.eval(&bin, &vals)),
                LoopCell::VectorArith { op, ty, a, b } => {
                    op.eval(*ty, a.eval(&bin, &vals) as u64, b.eval(&bin, &vals) as u64) as i64
                }
                LoopCell::Func { unit, a, b } => {
                    unit.eval(a.eval(&bin, &vals), b.eval(&bin, &vals))
                }
                LoopCell::MemRead { .. }
                | LoopCell::HeapRead { .. }
                | LoopCell::HeapWrite { .. }
                | LoopCell::HeapAlloc { .. } => panic!(
                    "MemRead reached an evaluator that does not thread the iterated sequence — \
                     a sequence loop must be evaluated via eval_loop_fabric_seq"
                ),
                LoopCell::Select { cmp, ca, cb, t, e } => {
                    if cmp.eval(ca.eval(&bin, &vals), cb.eval(&bin, &vals)) {
                        t.eval(&bin, &vals)
                    } else {
                        e.eval(&bin, &vals)
                    }
                }
            };
            vals.push(v);
        }
        state = wf.next.iter().map(|s| s.eval(&bin, &vals)).collect();
        guard += 1;
        assert!(
            guard < 100_000_000,
            "while fabric did not terminate within the guard"
        );
    }
    state[wf.result_state]
}

/// Emit the while-CGRA as a SEQUENTIAL circuit: a combinational body datapath + a controller whose
/// `running` flag is the FABRIC PREDICATE — evaluated on the init state (at reset) and on the next
/// state (each step) to decide whether to keep iterating. No counter: the trip count is data.
pub fn emit_while_fabric_verilog(name: &str, wf: &WhileFabric) -> String {
    let ns = wf.n_state;
    let name_in = |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else {
            format!("inv{}", k - ns)
        }
    };
    let name_val = |k: usize| format!("b{k}");
    let circ = |s: &Src| src_named(s, &|k| format!("in{k}"), &|k| format!("v{k}"));

    // Render the predicate with a caller-chosen rendering of the STATE operands.
    let pred_with = |state_name: &dyn Fn(usize) -> String| -> String {
        let (cmp, ca, cb) = &wf.pred;
        let r = |s: &Src| -> String {
            match s {
                Src::In(k) if *k < ns => state_name(*k),
                Src::In(k) => format!("inv{}", k - ns),
                Src::Const(c) => {
                    if *c < 0 {
                        format!("-64'sd{}", (*c as i128).unsigned_abs())
                    } else {
                        format!("64'sd{c}")
                    }
                }
                Src::Val(_) => unreachable!("while predicate has no datapath vals"),
            }
        };
        format!("($signed({}) {} $signed({}))", r(ca), cmp.verilog(), r(cb))
    };

    let inputs = (0..wf.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    let mut invs = String::new();
    for (j, s) in wf.invariants.iter().enumerate() {
        invs.push_str(&format!("  wire [63:0] inv{j} = {};\n", circ(s)));
    }
    let mut bodyw = String::new();
    for (i, cell) in wf.body.iter().enumerate() {
        let rhs = match cell {
            LoopCell::Arith { op, a, b } => {
                op.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::VectorArith { op, ty, a, b } => {
                op.verilog(*ty, &src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::Func { unit, a, b } => unit.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val)),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead RTL emission (a scratchpad-read tile driven by the induction counter) is the \
                 P2 scratchpad follow-on; the sequence-loop LOWERING is proven via eval_loop_fabric_seq"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => format!(
                "($signed({}) {} $signed({})) ? {} : {}",
                src_named(ca, &name_in, &name_val),
                cmp.verilog(),
                src_named(cb, &name_in, &name_val),
                src_named(t, &name_in, &name_val),
                src_named(e, &name_in, &name_val),
            ),
        };
        bodyw.push_str(&format!("  wire [63:0] b{i} = {rhs};\n"));
    }
    let mut nexts = String::new();
    for (j, s) in wf.next.iter().enumerate() {
        nexts.push_str(&format!(
            "  wire [63:0] ns{j} = {};\n",
            src_named(s, &name_in, &name_val)
        ));
    }
    let mut reset = String::new();
    let mut step = String::new();
    for (j, s) in wf.init.iter().enumerate() {
        reset.push_str(&format!("      s{j} <= {};\n", circ(s)));
        step.push_str(&format!("      s{j} <= ns{j};\n"));
    }
    let state_decl = (0..ns)
        .map(|j| format!("s{j}"))
        .collect::<Vec<_>>()
        .join(", ");
    // The continue predicate on the INIT state (reset) and on the NEXT state (each step).
    let pred_init = pred_with(&|k| circ(&wf.init[k]));
    let pred_nxt = pred_with(&|k| format!("ns{k}"));

    format!(
        "// while_{name} — a compiled WHILE loop as a CGRA: a fixed body datapath streamed by a\n\
         // controller whose CONTINUE condition is FABRIC-COMPUTED (a predicate over the state), not\n\
         // a counter — the data-dependent trip count is decided by the datapath. Futamura P1 for a\n\
         // data-dependent loop; 64-bit so it wraps like the VM's Int; body-step and predicate are\n\
         // each certified == the VM by our own SAT.\n\
         module while_{name}(input clk, input rst, {inputs}output [63:0] result, output done);\n\
         \x20 reg [63:0] {state_decl};\n\
         \x20 reg running;\n\
         {invs}{bodyw}{nexts}\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         {reset}\
         \x20     running <= {pred_init};\n\
         \x20   end else if (running) begin\n\
         {step}\
         \x20     running <= {pred_nxt};\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = !running;\n\
         \x20 assign result = s{result_state};\n\
         endmodule\n",
        result_state = wf.result_state,
    )
}

/// Lower ONE while-body state register's next-state to a `BoundedExpr` (over `s0..`, `inv0..`) for
/// the certified body step. No induction variable in the body-input convention.
pub fn while_next_to_bounded(
    wf: &WhileFabric,
    which: usize,
    width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    let ns = wf.n_state;
    let name_in = move |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else {
            format!("inv{}", k - ns)
        }
    };
    fn src_expr(
        s: &Src,
        vals: &[crate::codegen_sva::sva_to_verify::BoundedExpr],
        width: u32,
        name_in: &dyn Fn(usize) -> String,
    ) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
        use crate::codegen_sva::sva_to_verify::BoundedExpr;
        match s {
            Src::In(k) => BoundedExpr::BitVecVar(name_in(*k), width),
            Src::Val(k) => vals[*k].clone(),
            Src::Const(c) => BoundedExpr::BitVecConst {
                width,
                value: *c as u64,
            },
        }
    }
    let slt = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::SLt,
        left: Box::new(l),
        right: Box::new(r),
    };
    let eq = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(l),
        right: Box::new(r),
    };
    let mut vals: Vec<BoundedExpr> = Vec::with_capacity(wf.body.len());
    for cell in &wf.body {
        let expr = match cell {
            LoopCell::Arith { op, a, b } => {
                let a = src_expr(a, &vals, width, &name_in);
                let b = src_expr(b, &vals, width, &name_in);
                let bop = match op {
                    AluOp::Add => BitVecBoundedOp::Add,
                    AluOp::Sub => BitVecBoundedOp::Sub,
                    AluOp::Mul => BitVecBoundedOp::Mul,
                    AluOp::And => BitVecBoundedOp::And,
                    AluOp::Or => BitVecBoundedOp::Or,
                    AluOp::Xor => BitVecBoundedOp::Xor,
                    AluOp::Shl => BitVecBoundedOp::Shl,
                    AluOp::Shr => BitVecBoundedOp::AShr,
                    AluOp::UShr => BitVecBoundedOp::Shr,
                };
                BoundedExpr::BitVecBinary { op: bop, left: Box::new(a), right: Box::new(b) }
            }
            LoopCell::VectorArith { .. } => panic!(
                "a packed-SIMD (Simd) tile in a loop SAT bound is not SAT-lowered here — a Simd-bearing fabric \
                 is proven by eval ≡ VM + iverilog (simd_tile_lock + simd_rtl_lock), like a Func tile"
            ),
            LoopCell::Func { .. } => panic!(
                "division in a counted-loop body is not yet SAT-lowered — the shared_divider's \
                 own certified proof (rtl.rs::the_signed_divider_is_proven_correct_by_our_sat) covers \
                 the arithmetic; loop-carried division SAT is future work"
            ),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead in a loop-carried SAT bound is future work (the memory-loop induction proof)"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let ca = src_expr(ca, &vals, width, &name_in);
                let cb = src_expr(cb, &vals, width, &name_in);
                let t = src_expr(t, &vals, width, &name_in);
                let e = src_expr(e, &vals, width, &name_in);
                let (cond, tt, ee) = match cmp {
                    CmpKind::Lt => (slt(ca, cb), t, e),
                    CmpKind::Gt => (slt(cb, ca), t, e),
                    CmpKind::Ge => (slt(ca, cb), e, t),
                    CmpKind::Le => (slt(cb, ca), e, t),
                    CmpKind::Eq => (eq(ca, cb), t, e),
                    CmpKind::Ne => (eq(ca, cb), e, t),
                };
                BoundedExpr::BitVecIte { cond: Box::new(cond), then: Box::new(tt), else_: Box::new(ee) }
            }
        };
        vals.push(expr);
    }
    src_expr(&wf.next[which], &vals, width, &name_in)
}

/// A self-checking testbench for the while-CGRA: for each vector, pulse reset, clock until `done`,
/// and compare `result` against the VM oracle from `while_{name}_{in,out}.hex`.
pub fn emit_while_fabric_testbench(
    name: &str,
    wf: &WhileFabric,
    n: usize,
    max_cycles: usize,
) -> String {
    let drive = (0..wf.n_in)
        .map(|k| format!("in{k} = oin[i*{ni}+{k}];", ni = wf.n_in, k = k))
        .collect::<Vec<_>>()
        .join(" ");
    let ports = (0..wf.n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!("{ports}, ")
    };
    let decls = (0..wf.n_in)
        .map(|k| format!("  reg  [63:0] in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"// tb_while_{name} — the while-CGRA (fabric-predicate controller) vs the RUNNING VM.
`timescale 1ns/1ps
module tb_while_{name};
  reg clk = 0, rst;
{decls}
  reg  [63:0] oin [0:{ilast}];
  reg  [63:0] oout [0:{olast}];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  while_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("while_{name}_in.hex", oin);
    $readmemh("while_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk); {drive} rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < {maxc}) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n} while-CGRA runs match the VM (fabric predicate = trip count)");
    else $display("FAIL: %0d while-CGRA mismatches", errors);
    $finish;
  end
endmodule
"#,
        name = name,
        n = n,
        maxc = max_cycles,
        ilast = (n * wf.n_in).saturating_sub(1),
        olast = n - 1,
    )
}

// ===========================================================================================
// NESTED-loop CGRA — the HIERARCHICAL controller. A perfectly-nested `Repeat … Repeat …` lowers
// to an outer counter that, each iteration, runs the INNER loop (itself a CGRA) to completion —
// the inner's range may depend on the outer induction variable (`for j from 1 to i`). Sound by
// DOUBLE induction: the inner body-step ≡ the VM's inner iteration (certified by our SAT) ⇒ the
// inner CGRA ≡ the VM inner loop; the outer applies that inner transition once per outer step ⇒
// the nested CGRA ≡ the VM. Reuses `if_convert_body`, so the inner body may itself branch.
// ===========================================================================================

/// A range bound in the nested context: a circuit input, a constant, or the OUTER induction
/// variable (so `for j from 1 to i` is expressible).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NestSrc {
    Param(usize),
    Const(i64),
    OuterIv,
}

impl NestSrc {
    fn eval(&self, params: &[i64], outer_iv: i64) -> i64 {
        match self {
            NestSrc::Param(p) => params[*p],
            NestSrc::Const(c) => *c,
            NestSrc::OuterIv => outer_iv,
        }
    }
    /// Render over the FSM's `oi` register (the outer induction variable).
    fn verilog(&self) -> String {
        match self {
            NestSrc::Param(p) => format!("in{p}"),
            NestSrc::Const(c) => {
                if *c < 0 {
                    format!("-64'sd{}", (*c as i128).unsigned_abs())
                } else {
                    format!("64'sd{c}")
                }
            }
            NestSrc::OuterIv => "oi".to_string(),
        }
    }
}

/// A perfectly-nested counted loop lowered to a HIERARCHICAL CGRA: an outer counter over the shared
/// loop-carried state, whose body is the inner loop (its own body datapath + inner counter), whose
/// range may reference the outer induction variable.
#[derive(Clone, Debug)]
pub struct NestedFabric {
    pub n_in: usize,
    pub n_state: usize,
    pub init: Vec<Src>,
    pub outer_lo: Src,
    pub outer_hi: Src,
    pub inner_lo: NestSrc,
    pub inner_hi: NestSrc,
    /// Inner body cells. `Src::In(k)`: `0..n_state` = state, `n_state` = inner iv (j),
    /// `n_state+1` = outer iv (i), `n_state+2+k` = invariant k.
    pub inner_body: Vec<LoopCell>,
    pub inner_next: Vec<Src>,
    pub invariants: Vec<Src>,
    pub result_state: usize,
}

/// Recover the perfectly-nested loop in `fn_name` and lower it to a [`NestedFabric`].
pub fn nested_fabric_from_source(src: &str, fn_name: &str) -> NestedFabric {
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::{BTreeSet, HashMap};

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parse failed");
        let program = Compiler::compile(stmts, interner).expect("compile failed");
        let f = program
            .functions
            .iter()
            .find(|f| interner.resolve(f.name) == fn_name)
            .unwrap_or_else(|| panic!("function `{fn_name}` not found"));

        // FLOAT LANE (mirrors the loop / while fabrics): set up the shared thread-locals + seed float params
        // so the inner body's `exec_ops` routes float arithmetic to `FuncUnit::F*` tiles; the outer prologue
        // below admits a float accumulator constant and carries float-ness through `Move`. `int_const` stays
        // int-only — it is used for LOOP BOUNDS / structural indices, which are always integers.
        setup_program_thread_locals(&program);
        for (p, pt) in f.param_types.iter().enumerate() {
            if matches!(pt, Some(crate::vm::BoundaryType::Float)) {
                FLOAT_REGS.with(|t| {
                    t.borrow_mut().insert(p as u16);
                });
            }
        }

        let mut ops: Vec<(usize, Op)> = Vec::new();
        for (i, op) in program.code.iter().enumerate().skip(f.entry_pc) {
            ops.push((i, *op));
            if matches!(op, Op::Return { .. } | Op::ReturnNothing) {
                break;
            }
        }
        let int_const = |ci: u32| -> i64 {
            match program.constants[ci as usize] {
                Constant::Int(n) => n,
                ref c => panic!("nested fabric: non-integer constant {c:?}"),
            }
        };

        // ---- Outer prologue + outer IterNext ----
        let mut pro: HashMap<u16, Src> = HashMap::new();
        for p in 0..f.param_count {
            pro.insert(p, Src::In(p as usize));
        }
        let getp = |pro: &HashMap<u16, Src>, r: u16| -> Src {
            *pro.get(&r)
                .unwrap_or_else(|| panic!("nested outer prologue reads R{r}"))
        };
        let (mut outer_lo, mut outer_hi, mut outer_iv_reg, mut outer_header_pc) =
            (None, None, None, None);
        let mut outer_counter: Option<u16> = None;
        let mut outer_latch: Option<(u16, usize)> = None;
        let mut idx = 0;
        while idx < ops.len() {
            let (pc, op) = ops[idx];
            if let Some((c, bound, exit)) = counted_header(&ops, idx) {
                outer_latch = Some((bound, exit));
                outer_lo = Some(getp(&pro, c));
                outer_hi = Some(getp(&pro, bound));
                outer_iv_reg = Some(c);
                outer_header_pc = Some(pc);
                outer_counter = Some(c);
                idx += 2;
                break;
            }
            match op {
                Op::LoadConst { dst, idx: ci } => match program.constants[ci as usize] {
                    Constant::Float(fv) => {
                        pro.insert(dst, Src::Const(fv.to_bits() as i64));
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(dst);
                        });
                    }
                    _ => {
                        pro.insert(dst, Src::Const(int_const(ci)));
                    }
                },
                Op::Move { dst, src } => {
                    let s = getp(&pro, src);
                    if FLOAT_REGS.with(|t| t.borrow().contains(&src)) {
                        FLOAT_REGS.with(|t| {
                            t.borrow_mut().insert(dst);
                        });
                    }
                    pro.insert(dst, s);
                }
                Op::NewRange { start, end, .. } => {
                    outer_lo = Some(getp(&pro, start));
                    outer_hi = Some(getp(&pro, end));
                }
                Op::IterPrepare { .. } => {}
                Op::IterNext { dst, .. } => {
                    outer_iv_reg = Some(dst);
                    outer_header_pc = Some(pc);
                    idx += 1;
                    break;
                }
                other => panic!("nested outer prologue: `{}` unsupported", other.name()),
            }
            idx += 1;
        }
        let outer_lo = outer_lo.expect("nested fabric: no outer NewRange");
        let outer_hi = outer_hi.expect("nested fabric: no outer range end");
        let outer_iv_reg = outer_iv_reg.expect("nested fabric: no outer IterNext");
        let outer_header_pc = outer_header_pc.expect("nested fabric: no outer header");

        let outer_be = ops[idx..]
            .iter()
            .position(|(_pc, o)| matches!(o, Op::Jump { target } if *target == outer_header_pc))
            .map(|x| x + idx)
            .expect("nested fabric: no outer back-edge");
        let result_reg = ops
            .iter()
            .find_map(|(_, o)| match o {
                Op::Return { src } => Some(*src),
                _ => None,
            })
            .expect("nested fabric: no Return");

        // ---- Inner prologue (inside the outer body): NestSrc-tracked so `for j from 1 to i` works.
        let mut pn: HashMap<u16, NestSrc> = HashMap::new();
        for p in 0..f.param_count {
            pn.insert(p, NestSrc::Param(p as usize));
        }
        pn.insert(outer_iv_reg, NestSrc::OuterIv);
        let getn = |pn: &HashMap<u16, NestSrc>, r: u16| -> NestSrc {
            *pn.get(&r)
                .unwrap_or_else(|| panic!("nested inner prologue reads R{r}"))
        };
        let (mut inner_lo, mut inner_hi, mut inner_iv_reg, mut inner_header_pc) =
            (None, None, None, None);
        let mut inner_counter: Option<u16> = None;
        let mut inner_latch: Option<(u16, usize)> = None;
        let mut k = idx;
        while k < outer_be {
            let (pc, op) = ops[k];
            if let Some((c, bound, exit)) = counted_header(&ops, k) {
                inner_latch = Some((bound, exit));
                inner_lo = Some(getn(&pn, c));
                inner_hi = Some(getn(&pn, bound));
                inner_iv_reg = Some(c);
                inner_header_pc = Some(pc);
                inner_counter = Some(c);
                k += 2;
                break;
            }
            match op {
                Op::LoadConst { dst, idx: ci } => {
                    pn.insert(dst, NestSrc::Const(int_const(ci)));
                }
                Op::Move { dst, src } => {
                    let s = getn(&pn, src);
                    pn.insert(dst, s);
                }
                Op::NewRange { start, end, .. } => {
                    inner_lo = Some(getn(&pn, start));
                    inner_hi = Some(getn(&pn, end));
                }
                Op::IterPrepare { .. } => {}
                Op::IterNext { dst, .. } => {
                    inner_iv_reg = Some(dst);
                    inner_header_pc = Some(pc);
                    k += 1;
                    break;
                }
                other => panic!(
                    "nested fabric: the outer body must be a single inner loop; got `{}` \
                     (deeper nesting / mixed bodies are future work)",
                    other.name()
                ),
            }
            k += 1;
        }
        let inner_lo = inner_lo.expect("nested fabric: no inner NewRange (not perfectly nested)");
        let inner_hi = inner_hi.expect("nested fabric: no inner range end");
        let inner_iv_reg = inner_iv_reg.expect("nested fabric: no inner IterNext");
        let inner_header_pc = inner_header_pc.expect("nested fabric: no inner header");

        let inner_be = ops[k..outer_be]
            .iter()
            .position(|(_pc, o)| matches!(o, Op::Jump { target } if *target == inner_header_pc))
            .map(|x| x + k)
            .expect("nested fabric: no inner back-edge");
        let mut inner_body_pcops: Vec<(usize, Op)> = ops[k..inner_be].to_vec();
        // A counted inner loop's latch ends its body: the inner controller advances the counter.
        if let Some(c) = inner_counter {
            let is_one = |r: u16| matches!(pn.get(&r), Some(NestSrc::Const(1)));
            let (bound, exit) = inner_latch.expect("a counted inner loop has a header");
            match counted_latch(&inner_body_pcops, c, bound, exit, is_one) {
                0 => panic!(
                    "nested fabric: a counted inner loop's body does not end in its unit step"
                ),
                n => inner_body_pcops.truncate(inner_body_pcops.len() - n),
            }
        }
        // Everything between the inner back-edge and the outer back-edge is the inner IterPop, then a counted outer
        // loop's latch.
        let mut between: Vec<(usize, Op)> = ops[inner_be + 1..outer_be].to_vec();
        if let (Some(c), Some((bound, exit))) = (outer_counter, outer_latch) {
            let outer_is_one = |r: u16| matches!(pro.get(&r), Some(Src::Const(1)));
            let n = counted_latch(&between, c, bound, exit, outer_is_one);
            between.truncate(between.len() - n);
        }
        for (_, o) in &between {
            if !matches!(o, Op::IterPop) {
                panic!(
                    "nested fabric: the outer body has ops beyond the inner loop: `{}`",
                    o.name()
                );
            }
        }

        let reg_of = |op: &Op| -> (Option<u16>, Vec<u16>) {
            match op {
                Op::LoadConst { dst, .. } => (Some(*dst), vec![]),
                // `now` / `today` — a clock value-load (reads the injected fixed clock); no register read.
                Op::LoadNow { dst } | Op::LoadToday { dst } => (Some(*dst), vec![]),
                Op::Move { dst, src } => (Some(*dst), vec![*src]),
                // Logical `not` → a `(src == 0) ? 1 : 0` Select mux (one read, like `Move`).
                Op::Not { dst, src } => (Some(*dst), vec![*src]),
                // A numeric-coprocessor builtin (sqrt/floor/abs/min/max) reads its `arg_count` argument
                // registers starting at `args_start`; `exec_ops` lowers it to the matching Func/Select tile.
                Op::CallBuiltin { dst, args_start, arg_count, .. } => {
                    (Some(*dst), (0..*arg_count as u16).map(|k| *args_start + k).collect())
                }
                Op::Add { dst, lhs, rhs }
                | Op::Sub { dst, lhs, rhs }
                | Op::Mul { dst, lhs, rhs }
                // Division / mod / floor-div → a functional-unit tile (int `DivS`/`ModS`/`FloorDivS`, or
                // `FDivF` when an operand is a float), so a nested inner body's `s / 2.0` is a real FDivF tile.
                | Op::Div { dst, lhs, rhs }
                | Op::Mod { dst, lhs, rhs }
                | Op::FloorDiv { dst, lhs, rhs }
                // Bitwise / shifts → combinational Arith tiles (integer): `a & b`, `a | b`, `a xor b`,
                // `a << b`, `a >> b`. `exec_ops` already emits the matching `AluOp`.
                | Op::BitAnd { dst, lhs, rhs }
                | Op::BitOr { dst, lhs, rhs }
                | Op::BitXor { dst, lhs, rhs }
                | Op::Shl { dst, lhs, rhs }
                | Op::Shr { dst, lhs, rhs }
                // Power (`**` / `to the power of`) → the sequential-multiply `Pow` functional-unit tile.
                | Op::Pow { dst, lhs, rhs }
                | Op::Lt { dst, lhs, rhs }
                | Op::Gt { dst, lhs, rhs }
                | Op::LtEq { dst, lhs, rhs }
                | Op::GtEq { dst, lhs, rhs }
                | Op::Eq { dst, lhs, rhs }
                | Op::NotEq { dst, lhs, rhs } => (Some(*dst), vec![*lhs, *rhs]),
                Op::AddAssign { dst, src } => (Some(*dst), vec![*dst, *src]),
                Op::Jump { .. } | Op::JumpIfFalse { .. } => (None, vec![]),
                other => panic!("nested inner body: `{}` unsupported", other.name()),
            }
        };
        // The outer induction value under every name the inner prologue gives it: the counter, and the copy the
        // compiler makes of it as the loop's item (`Move item = counter` after the header).
        let outer_iv_regs: BTreeSet<u16> = pn
            .iter()
            .filter(|(_, s)| matches!(s, NestSrc::OuterIv))
            .map(|(r, _)| *r)
            .collect();
        let mut written: BTreeSet<u16> = BTreeSet::new();
        let mut upward: BTreeSet<u16> = BTreeSet::new();
        for (_, op) in &inner_body_pcops {
            let (dst, reads) = reg_of(op);
            for r in reads {
                if r != inner_iv_reg && !outer_iv_regs.contains(&r) && !written.contains(&r) {
                    upward.insert(r);
                }
            }
            if let Some(d) = dst {
                written.insert(d);
            }
        }
        let mut carried: Vec<u16> = upward.intersection(&written).copied().collect();
        if written.contains(&result_reg) && !carried.contains(&result_reg) {
            carried.push(result_reg);
        }
        carried.sort_unstable();
        let invariants_regs: Vec<u16> = upward.difference(&written).copied().collect();
        let n_state = carried.len();
        let state_slot: HashMap<u16, usize> =
            carried.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        let inv_slot: HashMap<u16, usize> = invariants_regs
            .iter()
            .enumerate()
            .map(|(i, r)| (*r, i))
            .collect();

        // Inner body inputs: state, then inner iv, then outer iv, then invariants.
        let mut reg: HashMap<u16, Src> = HashMap::new();
        for (r, s) in &state_slot {
            reg.insert(*r, Src::In(*s));
        }
        reg.insert(inner_iv_reg, Src::In(n_state));
        for r in &outer_iv_regs {
            reg.insert(*r, Src::In(n_state + 1));
        }
        for (r, s) in &inv_slot {
            reg.insert(*r, Src::In(n_state + 2 + *s));
        }
        let inner_body = if_convert_body(
            &inner_body_pcops,
            &mut reg,
            &carried,
            &program.constants,
            Vec::new(),
        );
        let init: Vec<Src> = carried.iter().map(|r| getp(&pro, *r)).collect();
        let inner_next: Vec<Src> = carried.iter().map(|&r| *reg.get(&r).unwrap()).collect();
        let invariants: Vec<Src> = invariants_regs.iter().map(|r| getp(&pro, *r)).collect();
        let result_state = *state_slot
            .get(&result_reg)
            .expect("nested fabric: the returned register is not loop-carried");

        NestedFabric {
            n_in: f.param_count as usize,
            n_state,
            init,
            outer_lo,
            outer_hi,
            inner_lo,
            inner_hi,
            inner_body,
            inner_next,
            invariants,
            result_state,
        }
    })
}

/// Evaluate the nested CGRA: the outer counter drives the inner loop to completion each iteration
/// (the inner range may reference the outer iv). 64-bit wrapping — the VM's `Int` mod 2⁶⁴.
pub fn eval_nested_fabric(nf: &NestedFabric, params: &[i64]) -> i64 {
    let empty: [i64; 0] = [];
    let mut state: Vec<i64> = nf.init.iter().map(|s| s.eval(params, &empty)).collect();
    let invs: Vec<i64> = nf
        .invariants
        .iter()
        .map(|s| s.eval(params, &empty))
        .collect();
    let olo = nf.outer_lo.eval(params, &empty);
    let ohi = nf.outer_hi.eval(params, &empty);
    let mut i = olo;
    while i <= ohi {
        let ilo = nf.inner_lo.eval(params, i);
        let ihi = nf.inner_hi.eval(params, i);
        let mut j = ilo;
        while j <= ihi {
            let mut bin: Vec<i64> = state.clone();
            bin.push(j);
            bin.push(i);
            bin.extend_from_slice(&invs);
            let mut vals: Vec<i64> = Vec::with_capacity(nf.inner_body.len());
            for cell in &nf.inner_body {
                let v = match cell {
                    LoopCell::Arith { op, a, b } => {
                        op.eval(a.eval(&bin, &vals), b.eval(&bin, &vals))
                    }
                    LoopCell::VectorArith { op, ty, a, b } => {
                        op.eval(*ty, a.eval(&bin, &vals) as u64, b.eval(&bin, &vals) as u64) as i64
                    }
                    LoopCell::Func { unit, a, b } => {
                        unit.eval(a.eval(&bin, &vals), b.eval(&bin, &vals))
                    }
                    LoopCell::MemRead { .. }
                    | LoopCell::HeapRead { .. }
                    | LoopCell::HeapWrite { .. }
                    | LoopCell::HeapAlloc { .. } => panic!(
                        "MemRead reached an evaluator that does not thread the iterated sequence — \
                         a sequence loop must be evaluated via eval_loop_fabric_seq"
                    ),
                    LoopCell::Select { cmp, ca, cb, t, e } => {
                        if cmp.eval(ca.eval(&bin, &vals), cb.eval(&bin, &vals)) {
                            t.eval(&bin, &vals)
                        } else {
                            e.eval(&bin, &vals)
                        }
                    }
                };
                vals.push(v);
            }
            state = nf.inner_next.iter().map(|s| s.eval(&bin, &vals)).collect();
            if j == ihi {
                break;
            }
            j = j.wrapping_add(1);
        }
        if i == ohi {
            break;
        }
        i = i.wrapping_add(1);
    }
    state[nf.result_state]
}

/// Emit the nested CGRA as a HIERARCHICAL FSM: an outer counter that, per iteration, sets up the
/// inner range (from the outer iv) and runs the inner counter to completion, streaming the shared
/// state through the inner body datapath. Three states — SETUP (arm inner for `oi`), RUN (inner
/// iterates), and advance-outer — so the inner range recomputes correctly when the outer advances.
pub fn emit_nested_fabric_verilog(name: &str, nf: &NestedFabric) -> String {
    let ns = nf.n_state;
    let name_in = |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else if k == ns {
            "ij".to_string()
        } else if k == ns + 1 {
            "oi".to_string()
        } else {
            format!("inv{}", k - ns - 2)
        }
    };
    let name_val = |k: usize| format!("b{k}");
    let circ = |s: &Src| src_named(s, &|k| format!("in{k}"), &|k| format!("v{k}"));

    let inputs = (0..nf.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    let mut invs = String::new();
    for (j, s) in nf.invariants.iter().enumerate() {
        invs.push_str(&format!("  wire [63:0] inv{j} = {};\n", circ(s)));
    }
    let mut bodyw = String::new();
    for (i, cell) in nf.inner_body.iter().enumerate() {
        let rhs = match cell {
            LoopCell::Arith { op, a, b } => {
                op.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::VectorArith { op, ty, a, b } => {
                op.verilog(*ty, &src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val))
            }
            LoopCell::Func { unit, a, b } => unit.verilog(&src_named(a, &name_in, &name_val), &src_named(b, &name_in, &name_val)),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead RTL emission (a scratchpad-read tile driven by the induction counter) is the \
                 P2 scratchpad follow-on; the sequence-loop LOWERING is proven via eval_loop_fabric_seq"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => format!(
                "($signed({}) {} $signed({})) ? {} : {}",
                src_named(ca, &name_in, &name_val),
                cmp.verilog(),
                src_named(cb, &name_in, &name_val),
                src_named(t, &name_in, &name_val),
                src_named(e, &name_in, &name_val),
            ),
        };
        bodyw.push_str(&format!("  wire [63:0] b{i} = {rhs};\n"));
    }
    let mut nexts = String::new();
    for (j, s) in nf.inner_next.iter().enumerate() {
        nexts.push_str(&format!(
            "  wire [63:0] ns{j} = {};\n",
            src_named(s, &name_in, &name_val)
        ));
    }
    let mut reset = String::new();
    let mut step = String::new();
    for (j, s) in nf.init.iter().enumerate() {
        reset.push_str(&format!("      s{j} <= {};\n", circ(s)));
        step.push_str(&format!("        s{j} <= ns{j};\n"));
    }
    let state_decl = (0..ns)
        .map(|j| format!("s{j}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "// nested_{name} — a compiled NESTED LOOP as a HIERARCHICAL CGRA: an outer counter that\n\
         // runs the INNER loop (its own body datapath + counter) to completion each iteration; the\n\
         // inner range may depend on the outer induction variable. Futamura P1 for nested control\n\
         // flow; 64-bit so it wraps like the VM's Int; the inner body-step is certified == the VM by\n\
         // our own SAT, and the nested loop is sound by double induction.\n\
         module nested_{name}(input clk, input rst, {inputs}output [63:0] result, output done);\n\
         \x20 localparam SETUP = 2'd0, RUN = 2'd1, DONE = 2'd2;\n\
         \x20 reg [1:0] st;\n\
         \x20 reg [63:0] {state_decl};\n\
         \x20 reg [63:0] oi, ij;\n\
         \x20 wire [63:0] olo = {olo};\n\
         \x20 wire [63:0] ohi = {ohi};\n\
         \x20 wire [63:0] ilo = {ilo};\n\
         \x20 wire [63:0] ihi = {ihi};\n\
         {invs}{bodyw}{nexts}\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         {reset}\
         \x20     oi <= olo;\n\
         \x20     st <= ($signed(olo) <= $signed(ohi)) ? SETUP : DONE;\n\
         \x20   end else if (st == SETUP) begin\n\
         \x20     ij <= ilo;\n\
         \x20     if ($signed(ilo) <= $signed(ihi)) st <= RUN;\n\
         \x20     else if ($signed(oi) < $signed(ohi)) oi <= oi + 64'sd1;\n\
         \x20     else st <= DONE;\n\
         \x20   end else if (st == RUN) begin\n\
         {step}\
         \x20     if ($signed(ij) < $signed(ihi)) ij <= ij + 64'sd1;\n\
         \x20     else if ($signed(oi) < $signed(ohi)) begin oi <= oi + 64'sd1; st <= SETUP; end\n\
         \x20     else st <= DONE;\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = (st == DONE);\n\
         \x20 assign result = s{result_state};\n\
         endmodule\n",
        olo = circ(&nf.outer_lo),
        ohi = circ(&nf.outer_hi),
        ilo = nf.inner_lo.verilog(),
        ihi = nf.inner_hi.verilog(),
        result_state = nf.result_state,
    )
}

/// Lower ONE inner-body state register's next-state to a `BoundedExpr` over `s0..`, `ij`, `oi`,
/// `inv0..` — for the certified inner body step (the base of the double induction).
pub fn nested_inner_next_to_bounded(
    nf: &NestedFabric,
    which: usize,
    width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    let ns = nf.n_state;
    let name_in = move |k: usize| -> String {
        if k < ns {
            format!("s{k}")
        } else if k == ns {
            "ij".to_string()
        } else if k == ns + 1 {
            "oi".to_string()
        } else {
            format!("inv{}", k - ns - 2)
        }
    };
    fn src_expr(
        s: &Src,
        vals: &[crate::codegen_sva::sva_to_verify::BoundedExpr],
        width: u32,
        name_in: &dyn Fn(usize) -> String,
    ) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
        use crate::codegen_sva::sva_to_verify::BoundedExpr;
        match s {
            Src::In(k) => BoundedExpr::BitVecVar(name_in(*k), width),
            Src::Val(k) => vals[*k].clone(),
            Src::Const(c) => BoundedExpr::BitVecConst {
                width,
                value: *c as u64,
            },
        }
    }
    let slt = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::SLt,
        left: Box::new(l),
        right: Box::new(r),
    };
    let eq = |l: BoundedExpr, r: BoundedExpr| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(l),
        right: Box::new(r),
    };
    let mut vals: Vec<BoundedExpr> = Vec::with_capacity(nf.inner_body.len());
    for cell in &nf.inner_body {
        let expr = match cell {
            LoopCell::Arith { op, a, b } => {
                let a = src_expr(a, &vals, width, &name_in);
                let b = src_expr(b, &vals, width, &name_in);
                let bop = match op {
                    AluOp::Add => BitVecBoundedOp::Add,
                    AluOp::Sub => BitVecBoundedOp::Sub,
                    AluOp::Mul => BitVecBoundedOp::Mul,
                    AluOp::And => BitVecBoundedOp::And,
                    AluOp::Or => BitVecBoundedOp::Or,
                    AluOp::Xor => BitVecBoundedOp::Xor,
                    AluOp::Shl => BitVecBoundedOp::Shl,
                    AluOp::Shr => BitVecBoundedOp::AShr,
                    AluOp::UShr => BitVecBoundedOp::Shr,
                };
                BoundedExpr::BitVecBinary { op: bop, left: Box::new(a), right: Box::new(b) }
            }
            LoopCell::VectorArith { .. } => panic!(
                "a packed-SIMD (Simd) tile in a loop SAT bound is not SAT-lowered here — a Simd-bearing fabric \
                 is proven by eval ≡ VM + iverilog (simd_tile_lock + simd_rtl_lock), like a Func tile"
            ),
            LoopCell::Func { .. } => panic!(
                "division in a counted-loop body is not yet SAT-lowered — the shared_divider's \
                 own certified proof (rtl.rs::the_signed_divider_is_proven_correct_by_our_sat) covers \
                 the arithmetic; loop-carried division SAT is future work"
            ),
            LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. } | LoopCell::HeapAlloc { .. } => panic!(
                "MemRead in a loop-carried SAT bound is future work (the memory-loop induction proof)"
            ),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let ca = src_expr(ca, &vals, width, &name_in);
                let cb = src_expr(cb, &vals, width, &name_in);
                let t = src_expr(t, &vals, width, &name_in);
                let e = src_expr(e, &vals, width, &name_in);
                let (cond, tt, ee) = match cmp {
                    CmpKind::Lt => (slt(ca, cb), t, e),
                    CmpKind::Gt => (slt(cb, ca), t, e),
                    CmpKind::Ge => (slt(ca, cb), e, t),
                    CmpKind::Le => (slt(cb, ca), e, t),
                    CmpKind::Eq => (eq(ca, cb), t, e),
                    CmpKind::Ne => (eq(ca, cb), e, t),
                };
                BoundedExpr::BitVecIte { cond: Box::new(cond), then: Box::new(tt), else_: Box::new(ee) }
            }
        };
        vals.push(expr);
    }
    src_expr(&nf.inner_next[which], &vals, width, &name_in)
}

/// A self-checking testbench for the nested CGRA vs the VM oracle (`nested_{name}_{in,out}.hex`).
pub fn emit_nested_fabric_testbench(
    name: &str,
    nf: &NestedFabric,
    n: usize,
    max_cycles: usize,
) -> String {
    let drive = (0..nf.n_in)
        .map(|k| format!("in{k} = oin[i*{ni}+{k}];", ni = nf.n_in, k = k))
        .collect::<Vec<_>>()
        .join(" ");
    let ports = (0..nf.n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!("{ports}, ")
    };
    let decls = (0..nf.n_in)
        .map(|k| format!("  reg  [63:0] in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"// tb_nested_{name} — the hierarchical (outer+inner counter) CGRA vs the RUNNING VM.
`timescale 1ns/1ps
module tb_nested_{name};
  reg clk = 0, rst;
{decls}
  reg  [63:0] oin [0:{ilast}];
  reg  [63:0] oout [0:{olast}];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  nested_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("nested_{name}_in.hex", oin);
    $readmemh("nested_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk); {drive} rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < {maxc}) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n} nested-CGRA runs match the VM (outer counter streams the inner loop)");
    else $display("FAIL: %0d nested-CGRA mismatches", errors);
    $finish;
  end
endmodule
"#,
        name = name,
        n = n,
        maxc = max_cycles,
        ilast = (n * nf.n_in).saturating_sub(1),
        olast = n - 1,
    )
}

/// A self-checking testbench for the CGRA: for each recorded input vector, pulse reset, clock
/// until `done`, and compare `result` against the VM oracle from `cgra_{in,out}.hex`.
pub fn emit_loop_fabric_testbench(
    name: &str,
    lf: &LoopFabric,
    n: usize,
    max_cycles: usize,
) -> String {
    let drive = (0..lf.n_in)
        .map(|k| format!("in{k} = oin[i*{ni}+{k}];", ni = lf.n_in, k = k))
        .collect::<Vec<_>>()
        .join(" ");
    let ports = (0..lf.n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!("{ports}, ")
    };
    let decls = (0..lf.n_in)
        .map(|k| format!("  reg  [63:0] in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"// tb_cgra_{name} — the CGRA (clocked body datapath) vs the RUNNING VM over the loop program.
`timescale 1ns/1ps
module tb_cgra_{name};
  reg clk = 0, rst;
{decls}
  reg  [63:0] oin [0:{ilast}];
  reg  [63:0] oout [0:{olast}];
  wire [63:0] result;
  wire done;
  integer i, c, errors = 0;
  cgra_{name} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("cgra_{name}_in.hex", oin);
    $readmemh("cgra_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      @(negedge clk); {drive} rst = 1;
      @(negedge clk); rst = 0;
      c = 0;
      while (!done && c < {maxc}) begin @(negedge clk); c = c + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n} CGRA loop runs match the VM (body = circuit, loop = counter)");
    else $display("FAIL: %0d CGRA mismatches", errors);
    $finish;
  end
endmodule
"#,
        name = name,
        n = n,
        maxc = max_cycles,
        ilast = (n * lf.n_in).saturating_sub(1),
        olast = n - 1,
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fabric_rtl/tests.rs"]
mod tests;
