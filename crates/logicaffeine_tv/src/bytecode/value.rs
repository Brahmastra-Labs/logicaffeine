//! The values both executors compute over, and the runtime's operations on them as Z3 terms.
//!
//! An `Int` is exact, as the runtime's is: an `i64` that overflows becomes a big integer, so a Z3 integer models every
//! value an `Int` variable or register holds. Each operation gives its value and the condition under which the runtime
//! raises instead; past that condition the value is never observed. Bitwise operations and shifts exist on `i64`
//! operands only (a big integer raises), except a low-bit mask on a non-negative big integer, which the runtime
//! computes as the remainder — each is modeled for every operand, not only the ones a program happens to pass.

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use z3::ast::{Ast, Bool, Int, BV};

use crate::symexec::Unsupported;

/// A value a source variable or a VM register holds.
#[derive(Clone, Debug)]
pub enum SymVal {
    /// An integer, exact.
    Int(Int),
    /// A truth value.
    Bool(Bool),
    /// `nothing`: an unwritten register, or what a function without a value returns.
    Nothing,
    /// `a to b`, the inclusive integer range a `Repeat` walks.
    Range(Int, Int),
    /// A register a callee's frame overwrote; no correct program reads one again.
    Clobbered,
}

/// What a value is, without its term: the shape a loop head's havoc state is built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Int,
    Bool,
    Nothing,
    Range,
    Clobbered,
}

impl SymVal {
    pub fn kind(&self) -> Kind {
        match self {
            SymVal::Int(_) => Kind::Int,
            SymVal::Bool(_) => Kind::Bool,
            SymVal::Nothing => Kind::Nothing,
            SymVal::Range(..) => Kind::Range,
            SymVal::Clobbered => Kind::Clobbered,
        }
    }

    /// The constant this value is, when its term is one: what a candidate `register = constant` is built from.
    pub fn constant(&self) -> Option<Lit> {
        match self {
            SymVal::Int(i) => i.simplify().as_i64().map(Lit::Int),
            SymVal::Bool(b) => b.simplify().as_bool().map(Lit::Bool),
            SymVal::Nothing => Some(Lit::Nothing),
            _ => None,
        }
    }

    /// The line `Show` prints for a value whose term is a constant, an integer at any size.
    pub fn shown(&self) -> Option<String> {
        match self {
            SymVal::Int(i) => numeral(&i.simplify().to_string()),
            SymVal::Bool(b) => b.simplify().as_bool().map(|b| b.to_string()),
            _ => None,
        }
    }

    /// The value with its term simplified: a concrete run keeps every value a numeral, so no term grows with the
    /// steps that built it.
    pub fn settled(self) -> SymVal {
        match self {
            SymVal::Int(i) => SymVal::Int(i.simplify()),
            SymVal::Bool(b) => SymVal::Bool(b.simplify()),
            SymVal::Range(a, b) => SymVal::Range(a.simplify(), b.simplify()),
            other => other,
        }
    }

    pub fn int(&self, what: &str) -> Result<Int, Unsupported> {
        match self {
            SymVal::Int(i) => Ok(i.clone()),
            other => Err(Unsupported(format!("{what} is {:?}, not an Int", other.kind()))),
        }
    }
}

/// A Z3 integer numeral (`42`, `(- 42)`) as a decimal, at any size.
pub fn numeral(text: &str) -> Option<String> {
    let t = text.trim();
    let (sign, digits) = match t.strip_prefix("(- ").and_then(|r| r.strip_suffix(')')) {
        Some(neg) => ("-", neg.trim()),
        None => ("", t),
    };
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| format!("{sign}{digits}"))
}

/// A literal: the constants the source writes and the bytecode's constant pool holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lit {
    Int(i64),
    Bool(bool),
    Nothing,
}

impl Lit {
    pub fn value(self) -> SymVal {
        match self {
            Lit::Int(n) => SymVal::Int(Int::from_i64(n)),
            Lit::Bool(b) => SymVal::Bool(Bool::from_bool(b)),
            Lit::Nothing => SymVal::Nothing,
        }
    }
}

/// An operation's value and the condition under which the runtime raises instead.
pub struct Outcome {
    pub value: SymVal,
    pub raises: Bool,
}

impl Outcome {
    fn total(value: SymVal) -> Outcome {
        Outcome {
            value,
            raises: Bool::from_bool(false),
        }
    }
}

/// `2^k` as an integer term.
pub fn pow2(k: u32) -> Int {
    if k > 127 {
        return (pow2(127) * pow2(k - 127)).simplify();
    }
    (1u128 << k)
        .to_string()
        .parse::<Int>()
        .expect("a decimal numeral is an Int")
}

/// The value fits an `i64`: the runtime holds it as a machine word rather than a big integer.
pub fn in_i64(x: &Int) -> Bool {
    Bool::and(&[x.ge(Int::from_i64(i64::MIN)), x.le(Int::from_i64(i64::MAX))])
}

/// Division rounded toward zero (Rust's `/`, the runtime's `Int / Int`); `b` is not zero.
pub fn tdiv(a: &Int, b: &Int) -> Int {
    a.ge(Int::from_i64(0)).ite(&a.div(b), &-((-a).div(b)))
}

/// The remainder of [`tdiv`], with the dividend's sign (Rust's `%`).
pub fn tmod(a: &Int, b: &Int) -> Int {
    a - &(b * &tdiv(a, b))
}

/// Division rounded toward negative infinity (`//`). Z3's `div` is Euclidean, which agrees with the floor for a
/// positive divisor; for a negative one it is one more than the floor unless the division is exact.
pub fn floor_div(a: &Int, b: &Int) -> Int {
    let euclid = a.div(b);
    let exact = a.modulo(b).eq(Int::from_i64(0));
    Bool::or(&[b.gt(Int::from_i64(0)), exact]).ite(&euclid, &(&euclid - 1i64))
}

/// An `i64`'s low 64 bits.
fn word(x: &Int) -> BV {
    BV::from_int(x, 64)
}

/// A 64-bit pattern read back as a signed integer.
fn signed(bv: &BV) -> Int {
    Int::from_bv(bv, true)
}

/// `x` read as `u64` and back as `i64`: the wrapping a machine word does. A value already in `i64` is itself, said as
/// a case of its own, so a solver bounds the operands to stay in it rather than reasoning about the remainder.
pub fn wrap64(x: &Int) -> Int {
    let shifted = (x + &pow2(63)).modulo(pow2(64));
    in_i64(x).ite(x, &(&shifted - &pow2(63)))
}

/// The runtime's predicate for a low-bit mask: a non-negative `i64` one less than a power of two.
fn is_mask(m: &Int) -> Bool {
    let low = word(m);
    let next = word(&(m + 1i64));
    Bool::and(&[in_i64(m), m.ge(Int::from_i64(0)), (low & next).eq(BV::from_i64(0, 64))])
}

/// `a & b` on integers, as `binary_op` computes it: two's complement on `i64` operands; a non-negative big integer
/// masked by `2^k - 1` gives its remainder; anything else raises.
fn bitand(a: &Int, b: &Int) -> Outcome {
    let zero = Int::from_i64(0);
    for (x, m) in [(a, b), (b, a)] {
        if let Some(n) = m.simplify().as_i64() {
            if n >= 0 && (n as u64).wrapping_add(1).is_power_of_two() {
                let modulus = Int::from_u64(n as u64) + 1i64;
                return Outcome {
                    value: SymVal::Int(x.modulo(&modulus)),
                    raises: Bool::and(&[in_i64(x).not(), x.lt(&zero)]),
                };
            }
        }
    }
    let both = Bool::and(&[in_i64(a), in_i64(b)]);
    let big_a = Bool::and(&[is_mask(b), in_i64(a).not(), a.ge(&zero)]);
    let big_b = Bool::and(&[is_mask(a), in_i64(b).not(), b.ge(&zero)]);
    let value = both.ite(
        &signed(&(word(a) & word(b))),
        &big_a.ite(&a.modulo(&(b + 1i64)), &b.modulo(&(a + 1i64))),
    );
    Outcome {
        value: SymVal::Int(value),
        raises: Bool::or(&[both, big_a, big_b]).not(),
    }
}

/// An operation defined on two `i64`s and raising on a big integer: `|`, `^`, `<<`, `>>`.
fn word_op(a: &Int, b: &Int, op: impl Fn(BV, BV) -> BV) -> Outcome {
    Outcome {
        value: SymVal::Int(signed(&op(word(a), word(b)))),
        raises: Bool::and(&[in_i64(a), in_i64(b)]).not(),
    }
}

/// A shift's amount as the runtime takes it: truncated to `u32`, then masked to 0..63 — `b mod 64`.
fn shift_amount(b: &BV) -> BV {
    b.bvurem(BV::from_i64(64, 64))
}

/// `a ** k` for a constant exponent, by repeated multiplication.
fn power(a: &Int, k: i64) -> Result<Outcome, Unsupported> {
    if k < 0 {
        return Ok(Outcome {
            value: SymVal::Int(Int::from_i64(0)),
            raises: Bool::from_bool(true),
        });
    }
    if k > 128 {
        return Err(Unsupported(format!("an exponent of {k}")));
    }
    let mut acc = Int::from_i64(1);
    for _ in 0..k {
        acc = &acc * a;
    }
    Ok(Outcome::total(SymVal::Int(acc)))
}

/// A binary operator other than the short-circuit `and`/`or`, as the runtime's `binary_op` computes it.
pub fn binop(op: BinaryOpKind, l: &SymVal, r: &SymVal) -> Result<Outcome, Unsupported> {
    use BinaryOpKind::*;
    use SymVal::{Bool as B, Int as I};
    let zero = Int::from_i64(0);
    Ok(match (op, l, r) {
        (Add, I(a), I(b)) => Outcome::total(I(a + b)),
        (Subtract, I(a), I(b)) => Outcome::total(I(a - b)),
        (Multiply, I(a), I(b)) => Outcome::total(I(a * b)),
        (Divide, I(a), I(b)) => Outcome {
            value: I(tdiv(a, b)),
            raises: b.eq(&zero),
        },
        (Modulo, I(a), I(b)) => Outcome {
            value: I(tmod(a, b)),
            raises: b.eq(&zero),
        },
        (FloorDivide, I(a), I(b)) => Outcome {
            value: I(floor_div(a, b)),
            raises: b.eq(&zero),
        },
        (Pow, I(a), I(b)) => match b.simplify().as_i64() {
            Some(k) => power(a, k)?,
            None => return Err(Unsupported("a power with a symbolic exponent".into())),
        },
        (Lt, I(a), I(b)) => Outcome::total(B(a.lt(b))),
        (Gt, I(a), I(b)) => Outcome::total(B(a.gt(b))),
        (LtEq, I(a), I(b)) => Outcome::total(B(a.le(b))),
        (GtEq, I(a), I(b)) => Outcome::total(B(a.ge(b))),
        (Eq, I(a), I(b)) => Outcome::total(B(a.eq(b))),
        (NotEq, I(a), I(b)) => Outcome::total(B(a.eq(b).not())),
        (Eq, B(a), B(b)) => Outcome::total(B(a.eq(b))),
        (NotEq, B(a), B(b)) => Outcome::total(B(a.eq(b).not())),
        (Eq, SymVal::Nothing, SymVal::Nothing) => Outcome::total(B(Bool::from_bool(true))),
        (NotEq, SymVal::Nothing, SymVal::Nothing) => Outcome::total(B(Bool::from_bool(false))),
        (BitAnd, B(a), B(b)) => Outcome::total(B(Bool::and(&[a, b]))),
        (BitOr, B(a), B(b)) => Outcome::total(B(Bool::or(&[a, b]))),
        (BitXor, B(a), B(b)) => Outcome::total(B(a.xor(b))),
        (BitAnd, I(a), I(b)) => bitand(a, b),
        (BitOr, I(a), I(b)) => word_op(a, b, |x, y| x | y),
        (BitXor, I(a), I(b)) => word_op(a, b, |x, y| x ^ y),
        (Shl, I(a), I(b)) => word_op(a, b, |x, y| x.bvshl(shift_amount(&y))),
        (Shr, I(a), I(b)) => word_op(a, b, |x, y| x.bvashr(shift_amount(&y))),
        (op, l, r) => {
            return Err(Unsupported(format!(
                "{op:?} on {:?} and {:?}",
                l.kind(),
                r.kind()
            )))
        }
    })
}

/// `not v` on a truth value.
pub fn not(v: &SymVal) -> Result<SymVal, Unsupported> {
    match v {
        SymVal::Bool(b) => Ok(SymVal::Bool(b.not())),
        other => Err(Unsupported(format!("`not` on {:?}", other.kind()))),
    }
}

/// Whether a value counts as true where the runtime branches on it (`is_truthy`).
pub fn truth(v: &SymVal) -> Result<Bool, Unsupported> {
    match v {
        SymVal::Bool(b) => Ok(b.clone()),
        SymVal::Int(n) => Ok(n.eq(Int::from_i64(0)).not()),
        SymVal::Nothing => Ok(Bool::from_bool(false)),
        other => Err(Unsupported(format!("a branch on {:?}", other.kind()))),
    }
}

const SHIFT_MASK: u8 = 0x3F;
const ADD_MARKER: u8 = 0x40;
const SHIFT_PATH: u8 = 0x80;

/// Whether the reciprocal sequence `compiler::magic_eval` runs computes `n / c` for every `u64` `n`, decided exactly.
///
/// The add-marker sequence `((n - hi) >> 1) + hi >> s`, with `hi = n·M >> 64`, is `n·(M + 2^64) >> (65 + s)`, so
/// both sequences are `n·M' >> L` for some `M'` and `L`. Write `M'·c = 2^L + e`. With `n = q·c + r`, `n·M' / 2^L` is
/// `q + r/c + n·e / (c·2^L)`, so the sequence is exact if and only if `e ≥ 0` (else it is short at `n = c`) and
/// `r·2^L + n·e < c·2^L` for every `n`. For a fixed residue the left side grows with `n`, so it suffices to check
/// the largest `n` of residue `(2^64 - 1) mod c` and the largest of residue `c - 1`. The arithmetic is on Z3
/// numerals, exact at any size.
pub fn magic_exact(magic: u64, more: u8, divisor: i64) -> bool {
    if divisor < 2 {
        return false;
    }
    let s = (more & SHIFT_MASK) as u32;
    if more & SHIFT_PATH != 0 {
        return (divisor as u64).is_power_of_two() && divisor as u64 == 1u64 << s;
    }
    let (m, l) = if more & ADD_MARKER != 0 {
        (Int::from_u64(magic) + pow2(64), 65 + s)
    } else {
        (Int::from_u64(magic), 64 + s)
    };
    let c = Int::from_i64(divisor);
    let two_l = pow2(l);
    let n_max = pow2(64) - 1i64;
    let e = &(&m * &c) - &two_l;
    let r0 = n_max.modulo(&c);
    let bound = &c * &two_l;
    let top = (&(&r0 * &two_l) + &(&n_max * &e)).lt(&bound);
    let last = (&c - 1i64).eq(&r0).ite(
        &Bool::from_bool(true),
        &(&(&(&c - 1i64) * &two_l) + &(&(&(&n_max - 1i64) - &r0) * &e)).lt(&bound),
    );
    Bool::and(&[e.ge(Int::from_i64(0)), top, last]).simplify().as_bool() == Some(true)
}

/// The VM's `MagicDivU`: `x / c` (`mul_back == 0`) or `x % c` by the precomputed reciprocal, exactly as
/// `compiler::magic_eval` computes it on the `u64` reading of an `i64` `x` — for every `x`, not only the
/// non-negative ones the oracle's proof admits — and by exact division on a big integer. A `wrap` remainder
/// subtracts `c` once when `x >= c`. When [`magic_exact`] decides the sequence exact, its quotient is `n / c`; when it
/// does not, the sequence is taken step by step.
pub fn magic_div(x: &Int, magic: u64, more: u8, mul_back: i64, divisor: i64, wrap: bool) -> Outcome {
    let zero = Int::from_i64(0);
    let c = Int::from_i64(divisor);
    let machine = if wrap && mul_back != 0 {
        x.ge(&c).ite(&(x - &c), x)
    } else {
        let n = x.ge(&zero).ite(x, &(x + &pow2(64)));
        let shift = pow2((more & SHIFT_MASK) as u32);
        let q = if magic_exact(magic, more, divisor) {
            n.div(&c)
        } else if more & SHIFT_PATH != 0 {
            n.div(&shift)
        } else {
            let hi = (&n * &Int::from_u64(magic)).div(pow2(64));
            if more & ADD_MARKER != 0 {
                let t = (&(&n - &hi).div(Int::from_i64(2)) + &hi).modulo(pow2(64));
                t.div(&shift)
            } else {
                hi.div(&shift)
            }
        };
        let q = q.ge(&pow2(63)).ite(&(&q - &pow2(64)), &q);
        if mul_back == 0 {
            q
        } else {
            wrap64(&(x - &wrap64(&(&q * &Int::from_i64(mul_back)))))
        }
    };
    let exact = if mul_back == 0 { tdiv(x, &c) } else { tmod(x, &c) };
    Outcome::total(SymVal::Int(in_i64(x).ite(&machine, &exact)))
}

/// Two values are the same value: equal terms of one kind. Values of different kinds never are.
pub fn same(a: &SymVal, b: &SymVal) -> Bool {
    match (a, b) {
        (SymVal::Int(x), SymVal::Int(y)) => x.eq(y),
        (SymVal::Bool(x), SymVal::Bool(y)) => x.eq(y),
        (SymVal::Nothing, SymVal::Nothing) => Bool::from_bool(true),
        (SymVal::Range(a1, b1), SymVal::Range(a2, b2)) => Bool::and(&[a1.eq(a2), b1.eq(b2)]),
        _ => Bool::from_bool(false),
    }
}
