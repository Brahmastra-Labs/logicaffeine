use super::*;
use crate::interpreter::RuntimeValue;
use crate::intern::{Interner, Symbol};
use std::collections::HashMap;

fn leak<'a>(e: Expr<'a>) -> &'a Expr<'a> {
    Box::leak(Box::new(e))
}
fn num<'a>(n: i64) -> &'a Expr<'a> {
    leak(Expr::Literal(Literal::Number(n)))
}
fn var<'a>(s: Symbol) -> &'a Expr<'a> {
    leak(Expr::Identifier(s))
}
fn bin<'a>(op: BinaryOpKind, left: &'a Expr<'a>, right: &'a Expr<'a>) -> &'a Expr<'a> {
    leak(Expr::BinaryOp { op, left, right })
}

/// A proof that vouches for everything, so the tests below see the shape rules alone.
struct Everything;
impl WordProof for Everything {
    fn stays_in_word(&self, _: &Expr) -> bool {
        true
    }
    fn is_word(&self, _: &Expr) -> bool {
        true
    }
    fn is_nonnegative(&self, _: &Expr) -> bool {
        true
    }
}

/// A proof built from answers: nodes and operands are named by their addresses.
struct Answers {
    in_word: Vec<*const ()>,
    not_words: Vec<*const ()>,
    nonnegative: bool,
}
impl WordProof for Answers {
    fn stays_in_word(&self, node: &Expr) -> bool {
        self.in_word.contains(&(node as *const Expr as *const ()))
    }
    fn is_word(&self, operand: &Expr) -> bool {
        !self.not_words.contains(&(operand as *const Expr as *const ()))
    }
    fn is_nonnegative(&self, _: &Expr) -> bool {
        self.nonnegative
    }
}
fn addr(e: &Expr) -> *const () {
    e as *const Expr as *const ()
}

fn render(chain: &LowBits, interner: &Interner) -> String {
    match chain {
        LowBits::Word(Expr::Identifier(s)) => interner.resolve(*s).to_string(),
        LowBits::Word(e) => format!("word({e:?})"),
        LowBits::Const(c) => c.to_string(),
        LowBits::Shl(c, s) => format!("({} << {s})", render(c, interner)),
        LowBits::Op(op, l, r) => {
            let symbol = match op {
                LowOp::Add => "+",
                LowOp::Sub => "-",
                LowOp::Mul => "*",
                LowOp::And => "&",
                LowOp::Or => "|",
                LowOp::Xor => "^",
            };
            format!("({} {symbol} {})", render(l, interner), render(r, interner))
        }
    }
}

fn shape(e: &Expr, proof: &dyn WordProof, interner: &Interner) -> Option<(String, u32)> {
    masked_chain(e, proof).map(|m| (render(&m.chain, interner), m.mask))
}

#[test]
fn the_masked_lcg_is_a_chain_under_its_mask() {
    let mut i = Interner::new();
    let seed = i.intern("seed");
    let lcg = bin(
        BinaryOpKind::Add,
        bin(BinaryOpKind::Multiply, var(seed), num(1103515245)),
        num(12345),
    );
    let masked = bin(BinaryOpKind::BitAnd, lcg, num(2147483647));
    assert_eq!(
        shape(masked, &Everything, &i),
        Some(("((seed * 1103515245) + 12345)".into(), 0x7fff_ffff))
    );
    let modded = bin(BinaryOpKind::Modulo, lcg, num(2147483648));
    assert_eq!(
        shape(modded, &Everything, &i),
        Some(("((seed * 1103515245) + 12345)".into(), 0x7fff_ffff)),
        "`x % 2^31` of an `x >= 0` is the same chain under the same mask"
    );
    let flipped = bin(BinaryOpKind::BitAnd, num(65535), lcg);
    assert_eq!(
        shape(flipped, &Everything, &i).map(|s| s.1),
        Some(0xffff),
        "the mask may stand on either side of `&`"
    );
}

#[test]
fn only_a_low_mask_of_one_to_thirty_two_bits_narrows() {
    let mut i = Interner::new();
    let x = i.intern("x");
    let chain = bin(BinaryOpKind::Multiply, var(x), num(3));
    let under = |m: i64| shape(bin(BinaryOpKind::BitAnd, chain, num(m)), &Everything, &i).map(|s| s.1);
    for k in 1..=32u32 {
        assert_eq!(under(((1u64 << k) - 1) as i64), Some(((1u64 << k) - 1) as u32), "k = {k}");
    }
    assert_eq!(under(0), None, "a zero mask is no low mask");
    assert_eq!(under((1i64 << 33) - 1), None, "33 bits do not fit the narrowed width");
    assert_eq!(under(i64::MAX), None);
    assert_eq!(under(6), None, "6 is not 2^k - 1");
    assert_eq!(under(0x7fff_fffe), None);
    assert_eq!(under(-1), None, "a negative mask keeps every bit");
    let modded = |d: i64| shape(bin(BinaryOpKind::Modulo, chain, num(d)), &Everything, &i).map(|s| s.1);
    for k in 1..=32u32 {
        assert_eq!(modded(1i64 << k), Some(((1u64 << k) - 1) as u32), "% 2^{k}");
    }
    assert_eq!(modded(1), None, "% 1 is the constant 0");
    assert_eq!(modded(1i64 << 33), None);
    assert_eq!(modded(6), None);
    assert_eq!(modded(-8), None);
    assert_eq!(modded(0), None);
}

#[test]
fn a_remainder_narrows_only_for_a_nonnegative_dividend() {
    let mut i = Interner::new();
    let x = i.intern("x");
    let chain = bin(BinaryOpKind::Multiply, var(x), num(3));
    let modded = bin(BinaryOpKind::Modulo, chain, num(256));
    let signed = Answers {
        in_word: vec![addr(chain)],
        not_words: vec![],
        nonnegative: false,
    };
    assert_eq!(shape(modded, &signed, &i), None, "-7 % 8 is -7, not 1");
    let unsigned = Answers {
        nonnegative: true,
        ..signed
    };
    assert_eq!(shape(modded, &unsigned, &i), Some(("(x * 3)".into(), 255)));
}

#[test]
fn every_arithmetic_node_needs_its_word_proof() {
    let mut i = Interner::new();
    let (a, b) = (i.intern("a"), i.intern("b"));
    let product = bin(BinaryOpKind::Multiply, var(a), var(b));
    let sum = bin(BinaryOpKind::Add, product, num(1));
    let masked = bin(BinaryOpKind::BitAnd, sum, num(255));
    let both = Answers {
        in_word: vec![addr(product), addr(sum)],
        not_words: vec![],
        nonnegative: true,
    };
    assert_eq!(shape(masked, &both, &i), Some(("((a * b) + 1)".into(), 255)));
    let outer_only = Answers {
        in_word: vec![addr(sum)],
        not_words: vec![],
        nonnegative: true,
    };
    assert_eq!(
        shape(masked, &outer_only, &i),
        None,
        "an inner product that may leave i64 is a BigInt the mask may refuse"
    );
    let inner_only = Answers {
        in_word: vec![addr(product)],
        not_words: vec![],
        nonnegative: true,
    };
    assert_eq!(shape(masked, &inner_only, &i), None);
}

#[test]
fn every_operand_must_be_a_machine_word() {
    let mut i = Interner::new();
    let (a, b) = (i.intern("a"), i.intern("b"));
    let (ea, eb) = (var(a), var(b));
    let sum = bin(BinaryOpKind::Add, ea, eb);
    let masked = bin(BinaryOpKind::BitAnd, sum, num(255));
    let big_b = Answers {
        in_word: vec![addr(sum)],
        not_words: vec![addr(eb)],
        nonnegative: true,
    };
    assert_eq!(shape(masked, &big_b, &i), None, "a BigInt operand has no machine word");
}

#[test]
fn a_bitwise_only_chain_is_left_alone() {
    let mut i = Interner::new();
    let (a, b) = (i.intern("a"), i.intern("b"));
    let xor = bin(BinaryOpKind::BitXor, var(a), var(b));
    assert_eq!(shape(bin(BinaryOpKind::BitAnd, xor, num(255)), &Everything, &i), None);
    assert_eq!(shape(bin(BinaryOpKind::BitAnd, var(a), num(255)), &Everything, &i), None);
    let shifted = bin(BinaryOpKind::Shl, var(a), num(3));
    assert_eq!(
        shape(bin(BinaryOpKind::BitAnd, shifted, num(255)), &Everything, &i),
        Some(("(a << 3)".into(), 255)),
        "a shift is 64-bit work"
    );
}

#[test]
fn a_shift_counts_mod_64_and_one_of_32_or_more_is_read_whole() {
    let mut i = Interner::new();
    let a = i.intern("a");
    let under = |s: i64| {
        let shifted = bin(BinaryOpKind::Shl, var(a), num(s));
        shape(
            bin(BinaryOpKind::BitAnd, bin(BinaryOpKind::Add, shifted, var(a)), num(65535)),
            &Everything,
            &i,
        )
        .map(|s| s.0)
    };
    assert_eq!(under(7), Some("((a << 7) + a)".into()));
    assert_eq!(under(31), Some("((a << 31) + a)".into()));
    assert_eq!(under(65), Some("((a << 1) + a)".into()), "`<< 65` shifts by 1");
    assert_eq!(under(64), Some("((a << 0) + a)".into()));
    let whole = under(40).expect("narrows");
    assert!(whole.starts_with("(word("), "`a << 40` is an operand read whole: {whole}");
    let whole = under(-1).expect("narrows");
    assert!(whole.starts_with("(word("), "`<< -1` shifts by 63: {whole}");
    let n = i.intern("n");
    let by_name = bin(BinaryOpKind::Shl, var(a), var(n));
    let masked = bin(BinaryOpKind::BitAnd, bin(BinaryOpKind::Multiply, by_name, num(3)), num(255));
    assert!(
        shape(masked, &Everything, &i).expect("narrows").0.starts_with("(word("),
        "a shift by a name is read whole"
    );
}

#[test]
fn an_inner_power_of_two_remainder_is_a_mask() {
    let mut i = Interner::new();
    let a = i.intern("a");
    let inner = bin(BinaryOpKind::Modulo, bin(BinaryOpKind::Multiply, var(a), num(69069)), num(1048576));
    let outer = bin(BinaryOpKind::BitAnd, bin(BinaryOpKind::Add, inner, num(1)), num(16777215));
    assert_eq!(
        shape(outer, &Everything, &i).map(|s| s.0),
        Some("(((a * 69069) & 1048575) + 1)".into())
    );
    let wide = bin(BinaryOpKind::Modulo, bin(BinaryOpKind::Multiply, var(a), num(3)), num(1i64 << 40));
    let outer = bin(BinaryOpKind::BitAnd, bin(BinaryOpKind::Add, wide, num(1)), num(255));
    assert_eq!(
        shape(outer, &Everything, &i).map(|s| s.0),
        Some("((a * 3) + 1)".into()),
        "a remainder by 2^40 keeps every low 32 bits"
    );
}

/// Deterministic xorshift64*.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn value(&mut self) -> i64 {
        match self.below(6) {
            0 => self.below(20) as i64 - 10,
            1 => self.below(1 << 16) as i64 - (1 << 15),
            2 => self.next() as i64 >> 33,
            3 => self.next() as i64 >> 1,
            4 => [i64::MIN, i64::MAX, i64::MIN + 1, -1, 0, 1, u32::MAX as i64, 1 << 31][self.below(8) as usize],
            _ => self.next() as i64,
        }
    }
}

fn arbitrary<'a>(rng: &mut Rng, names: &[Symbol], depth: u32) -> &'a Expr<'a> {
    if depth == 0 || rng.below(4) == 0 {
        return if rng.below(3) == 0 {
            num(rng.value())
        } else {
            var(names[rng.below(names.len() as u64) as usize])
        };
    }
    let l = arbitrary(rng, names, depth - 1);
    match rng.below(9) {
        0 => bin(BinaryOpKind::Add, l, arbitrary(rng, names, depth - 1)),
        1 => bin(BinaryOpKind::Subtract, l, arbitrary(rng, names, depth - 1)),
        2 => bin(BinaryOpKind::Multiply, l, arbitrary(rng, names, depth - 1)),
        3 => bin(BinaryOpKind::BitAnd, l, arbitrary(rng, names, depth - 1)),
        4 => bin(BinaryOpKind::BitOr, l, arbitrary(rng, names, depth - 1)),
        5 => bin(BinaryOpKind::BitXor, l, arbitrary(rng, names, depth - 1)),
        6 => bin(BinaryOpKind::Shl, l, num(rng.below(80) as i64 - 8)),
        7 => bin(BinaryOpKind::Modulo, l, num(1i64 << (1 + rng.below(62)))),
        _ => bin(BinaryOpKind::Shr, l, num(rng.below(70) as i64)),
    }
}

/// The language's own arithmetic: an `Int`, a promoted `BigInt`, or the refusal.
fn reference(e: &Expr, env: &HashMap<Symbol, i64>) -> Result<RuntimeValue, String> {
    match e {
        Expr::Literal(Literal::Number(n)) => Ok(RuntimeValue::Int(*n)),
        Expr::Identifier(s) => Ok(RuntimeValue::Int(env[s])),
        Expr::BinaryOp { op, left, right } => {
            crate::semantics::arith::binary_op(*op, reference(left, env)?, reference(right, env)?)
        }
        other => panic!("not generated: {other:?}"),
    }
}

/// What the reference computes for this one assignment, taken as the emitter's proof: exactly true facts.
struct Truth<'m> {
    env: &'m HashMap<Symbol, i64>,
}
impl Truth<'_> {
    fn int(&self, e: &Expr) -> Option<i64> {
        match reference(e, self.env) {
            Ok(RuntimeValue::Int(v)) => Some(v),
            _ => None,
        }
    }
    fn words(&self, e: &Expr) -> bool {
        match e {
            Expr::BinaryOp { left, right, .. } => {
                self.int(e).is_some() && self.words(left) && self.words(right)
            }
            _ => self.int(e).is_some(),
        }
    }
}
impl WordProof for Truth<'_> {
    fn stays_in_word(&self, node: &Expr) -> bool {
        self.int(node).is_some()
    }
    fn is_word(&self, operand: &Expr) -> bool {
        self.words(operand)
    }
    fn is_nonnegative(&self, value: &Expr) -> bool {
        self.int(value).is_some_and(|v| v >= 0)
    }
}

fn evaluate(chain: &LowBits, env: &HashMap<Symbol, i64>) -> u32 {
    match chain {
        LowBits::Word(e) => match reference(e, env) {
            Ok(RuntimeValue::Int(v)) => v as u32,
            other => panic!("an operand the proof called a word is {other:?}"),
        },
        LowBits::Const(c) => *c,
        LowBits::Shl(c, s) => evaluate(c, env) << s,
        LowBits::Op(op, l, r) => {
            let (a, b) = (evaluate(l, env), evaluate(r, env));
            match op {
                LowOp::Add => a.wrapping_add(b),
                LowOp::Sub => a.wrapping_sub(b),
                LowOp::Mul => a.wrapping_mul(b),
                LowOp::And => a & b,
                LowOp::Or => a | b,
                LowOp::Xor => a ^ b,
            }
        }
    }
}

/// Soundness: whenever the proofs are true, the 32-bit chain under its mask IS the language's value of the masked
/// expression — over random chains of every operator, every mask width, and values from tiny to `i64::MIN`.
#[test]
fn a_narrowed_chain_computes_the_reference_value_whenever_its_proofs_hold() {
    let mut i = Interner::new();
    let names = [i.intern("a"), i.intern("b"), i.intern("c")];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut narrowed = 0usize;
    let mut by_width = [0usize; 33];
    for round in 0..60_000 {
        let value = arbitrary(&mut rng, &names, 1 + (round % 5) as u32);
        let k = 1 + rng.below(32) as u32;
        let masked = if rng.below(2) == 0 {
            bin(BinaryOpKind::BitAnd, value, num(((1u64 << k) - 1) as i64))
        } else {
            bin(BinaryOpKind::Modulo, value, num(1i64 << k))
        };
        for _ in 0..4 {
            let env: HashMap<Symbol, i64> = names.iter().map(|n| (*n, rng.value())).collect();
            let Some(chain) = masked_chain(masked, &Truth { env: &env }) else {
                continue;
            };
            let expected = match reference(masked, &env) {
                Ok(RuntimeValue::Int(v)) => v,
                other => panic!("a narrowed chain's reference must be an Int, got {other:?} for {masked:?} at {env:?}"),
            };
            assert_eq!(
                (evaluate(&chain.chain, &env) & chain.mask) as i64,
                expected,
                "{masked:?} at {env:?}"
            );
            narrowed += 1;
            by_width[k as usize] += 1;
        }
    }
    assert!(narrowed > 20_000, "the sweep narrowed only {narrowed} cases");
    for k in 1..=32 {
        assert!(by_width[k] > 100, "width {k} narrowed only {} times", by_width[k]);
    }
}
