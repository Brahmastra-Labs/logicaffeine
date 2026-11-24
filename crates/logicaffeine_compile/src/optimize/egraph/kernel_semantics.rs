//! The LOGOS operations the e-graph's rewrites name, defined in the kernel, and those rewrites' identities proved
//! over them.
//!
//! The ring and Boolean rules are identities in the standard library's own vocabulary. The rules here rewrite
//! operations it does not have: `/` and `%` on integers, which truncate toward zero, `>>`, `<<`, `&`, `|`, `^`,
//! `copy of`, `length of`, `item i of` and `items a through b of`. Each is a definition with a body and no axiom,
//! transcribed from what the runtime computes; where Lean defines the same operation, the transcription is Lean's
//! (`Int.tdiv`, `Int.tmod`, `Int.shiftRight`, `Nat.bitwise`, `Int.land`, `Int.lor`, `Int.xor`, `List.take`,
//! `List.drop`). A [`KernelCertificate`] carries the definitions its statement mentions, and [`certificate_context`]
//! declares them beside the standard library, where each may add a name and none may rebind one.
//!
//! The kernel checks that a definition is well typed, not that it computes what the runtime does. That link is
//! `egraph_kernel_proof_lock`: it evaluates every definition in [`definitions`] on literal operands and compares each
//! answer with the runtime's own.
//!
//! The standard library's `Nat` operations unfold one successor at a time, so `Nat_div m 2` on a large literal `m`
//! does not finish; its public `Int` operations `add`, `sub`, `mul`, `div` and `mod` compute natively on literals.
//! Every definition here does its arithmetic on a value through those, so the lock can evaluate it on every `i64`.

use logicaffeine_kernel::int_inductive::canonical_int;
use logicaffeine_kernel::tactics::{Goal, GoalState, TacticError, TacticStep};
use logicaffeine_kernel::{BigInt, Literal, Term, Universe};

use super::rules::{certificate_context, KernelCertificate};

/// Integer `/`: the quotient rounded toward zero.
pub const TDIV: &str = "logos_tdiv";
/// Integer `%`: the remainder of `/`, which takes the dividend's sign.
pub const TMOD: &str = "logos_tmod";
/// Integer `>>` by a shift amount: the quotient by `2^amount` rounded toward negative infinity.
pub const SHR: &str = "logos_shr";
/// Integer `<<` by a shift amount: the product by `2^amount`, wrapped to 64 bits.
pub const SHL: &str = "logos_shl";
/// `copy of v`.
pub const COPY: &str = "logos_copy";
/// `length of xs` for a list.
pub const LEN: &str = "logos_len";
/// The item at a 0-based offset of a list, or none.
pub const NTH: &str = "logos_nth";
/// `item i of xs` for a list.
pub const INDEX: &str = "logos_index";
/// The first `count` items of a list, all of them when it has fewer.
pub const TAKE: &str = "List_take";
/// A list without its first `count` items, empty when it has fewer.
pub const DROP: &str = "List_drop";
/// `items start through end of xs` for a list.
pub const SLICE: &str = "logos_slice";
/// A Boolean operation applied bit by bit to two naturals.
pub const BITWISE: &str = "logos_bitwise";
/// Integer `&`.
pub const AND: &str = "logos_and";
/// Integer `|`.
pub const OR: &str = "logos_or";
/// Integer `^`.
pub const XOR: &str = "logos_xor";

// ----- Term builders ----------------------------------------------------

fn g(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn v(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}

fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}

fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}

fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}

fn arrow(domain: Term, codomain: Term) -> Term {
    pi("_", domain, codomain)
}

fn mat(discriminant: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(discriminant), motive: Box::new(motive), cases }
}

fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}

fn nat() -> Term {
    g("Nat")
}

fn int() -> Term {
    g("Int")
}

fn boolean() -> Term {
    g("Bool")
}

fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}

fn one() -> Term {
    succ(g("Zero"))
}

fn of_nat(n: Term) -> Term {
    app(g("Int_ofNat"), n)
}

fn neg_succ(n: Term) -> Term {
    app(g("Int_negSucc"), n)
}

fn list(elem: Term) -> Term {
    app(g("List"), elem)
}

fn option(elem: Term) -> Term {
    app(g("Option"), elem)
}

fn list_length(elem: Term, items: Term) -> Term {
    apps(g("List_length"), &[elem, items])
}

fn eq(carrier: Term, lhs: Term, rhs: Term) -> Term {
    apps(g("Eq"), &[carrier, lhs, rhs])
}

fn nil(elem: Term) -> Term {
    app(g("nil"), elem)
}

fn cons(elem: Term, head: Term, tail: Term) -> Term {
    apps(g("cons"), &[elem, head, tail])
}

fn nat_sub(a: Term, b: Term) -> Term {
    apps(g("Nat_sub"), &[a, b])
}

fn nat_le(a: Term, b: Term) -> Term {
    apps(g("natLe"), &[a, b])
}

fn int_le(a: Term, b: Term) -> Term {
    apps(g("Int_le"), &[a, b])
}

fn int_add(a: Term, b: Term) -> Term {
    apps(g("Int_add"), &[a, b])
}

fn int_sub(a: Term, b: Term) -> Term {
    apps(g("Int_sub"), &[a, b])
}

/// `a · b` through the standard library's public `mul`, which computes natively on literals.
fn mul(a: Term, b: Term) -> Term {
    apps(g("mul"), &[a, b])
}

/// `a + b` through the standard library's public `add`.
fn add(a: Term, b: Term) -> Term {
    apps(g("add"), &[a, b])
}

/// The Euclidean quotient `a / b` through the standard library's public `div`.
fn int_div(a: Term, b: Term) -> Term {
    apps(g("div"), &[a, b])
}

/// The Euclidean remainder `a mod b`, never negative, through the standard library's public `mod`.
fn int_mod(a: Term, b: Term) -> Term {
    apps(g("mod"), &[a, b])
}

/// `-a`.
fn int_neg(a: Term) -> Term {
    app(g("Int_neg"), a)
}

/// The nonnegative integer `n`.
fn int_literal(n: u64) -> Term {
    of_nat(nat_literal(BigInt::from_u64(n)))
}

/// `match b { true ⇒ then | false ⇒ otherwise }`, both of type `ty`.
fn if_bool(b: Term, ty: Term, then: Term, otherwise: Term) -> Term {
    mat(b, lam("_", boolean(), ty), vec![then, otherwise])
}

/// The `Nat` an integer `ofNat r` holds, and 0 for a `negSucc`.
fn nat_of(x: Term) -> Term {
    forced(x, "r", v("r"))
}

/// `match x { ofNat name ⇒ then | negSucc _ ⇒ 0 }` at type `Nat`: `then` reads the value of `x` as the natural `name`,
/// which is a literal once `x` computes to one.
fn forced(x: Term, name: &str, then: Term) -> Term {
    mat(x, lam("_", int(), nat()), vec![lam(name, nat(), then), lam("_", nat(), g("Zero"))])
}

/// `Eq Bool b true`: the Boolean `b` holds.
fn is_true(b: Term) -> Term {
    eq(boolean(), b, g("true"))
}

fn nat_literal(n: BigInt) -> Term {
    Term::Lit(Literal::Nat(n))
}

/// `2^bits` as an exact integer.
fn two_to_the(bits: u64) -> BigInt {
    BigInt::from_u64(1).shl_bits(bits)
}

/// `2^64`, the number of 64-bit words.
fn word_count() -> Term {
    nat_literal(two_to_the(64))
}

/// `2^63`, the least magnitude past `i64::MAX`.
fn word_half() -> Term {
    nat_literal(two_to_the(63))
}

/// `2^64 - 1`, the largest 64-bit word.
fn word_top() -> Term {
    nat_literal(two_to_the(64).sub(&BigInt::from_u64(1)))
}

/// `2^63 - 1`, the magnitude of `i64::MAX`.
fn signed_top() -> Term {
    nat_literal(BigInt::from_i64(i64::MAX))
}

/// `2^n` as a `Nat`.
pub fn power_of_two(n: Term) -> Term {
    apps(g("Nat_pow"), &[succ(succ(g("Zero"))), n])
}

/// `a / b` on integers.
pub fn tdiv(a: Term, b: Term) -> Term {
    apps(g(TDIV), &[a, b])
}

/// `a % b` on integers.
pub fn tmod(a: Term, b: Term) -> Term {
    apps(g(TMOD), &[a, b])
}

/// `a & b` on integers.
pub fn bitand(a: Term, b: Term) -> Term {
    apps(g(AND), &[a, b])
}

/// `a | b` on integers.
pub fn bitor(a: Term, b: Term) -> Term {
    apps(g(OR), &[a, b])
}

/// `a ^ b` on integers.
pub fn bitxor(a: Term, b: Term) -> Term {
    apps(g(XOR), &[a, b])
}

/// `combine` applied bit by bit to the naturals `m` and `n`, with at most `fuel` halvings.
pub fn bitwise(combine: Term, fuel: Term, m: Term, n: Term) -> Term {
    apps(g(BITWISE), &[combine, fuel, m, n])
}

/// A Boolean operation of two bits, the combiner [`bitwise`] takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    /// Both bits.
    And,
    /// Either bit.
    Or,
    /// Exactly one bit.
    Xor,
    /// The first bit and not the second.
    AndNot,
}

impl BitOp {
    /// Every combiner, in declaration order.
    pub const ALL: [BitOp; 4] = [BitOp::And, BitOp::Or, BitOp::Xor, BitOp::AndNot];

    /// The combiner as a kernel `Bool → Bool → Bool`, each case a `match`.
    pub fn combiner(self) -> Term {
        let not = |b: Term| if_bool(b, boolean(), g("false"), g("true"));
        let body = match self {
            BitOp::And => if_bool(v("a"), boolean(), v("b"), g("false")),
            BitOp::Or => if_bool(v("a"), boolean(), g("true"), v("b")),
            BitOp::Xor => if_bool(v("a"), boolean(), not(v("b")), v("b")),
            BitOp::AndNot => if_bool(v("a"), boolean(), not(v("b")), g("false")),
        };
        lam("a", boolean(), lam("b", boolean(), body))
    }
}

/// The halvings [`AND`], [`OR`] and [`XOR`] allow: an `i64` has at most 63 bits past its sign.
pub const WORD_HALVINGS: u64 = 64;

/// The runtime's shift amount for an `Int` operand `b`: its low six bits, `b mod 64`, as a `Nat`.
pub fn shift_amount(b: Term) -> Term {
    nat_of(int_mod(b, int_literal(64)))
}

/// `x >> amount`, the amount a `Nat`.
pub fn shr(x: Term, amount: Term) -> Term {
    apps(g(SHR), &[x, amount])
}

/// `x << amount`, the amount a `Nat`.
pub fn shl(x: Term, amount: Term) -> Term {
    apps(g(SHL), &[x, amount])
}

/// The first `count` items of `items`, a list of `elem`.
pub fn list_take(elem: Term, count: Term, items: Term) -> Term {
    apps(g(TAKE), &[elem, count, items])
}

/// `items` without its first `count` items, a list of `elem`.
pub fn list_drop(elem: Term, count: Term, items: Term) -> Term {
    apps(g(DROP), &[elem, count, items])
}

/// `items start through end of items`, a list of `elem`.
pub fn slice(elem: Term, items: Term, start: Term, end: Term) -> Term {
    apps(g(SLICE), &[elem, items, start, end])
}

/// `copy of value`, the value of type `ty`.
pub fn copy(ty: Term, value: Term) -> Term {
    apps(g(COPY), &[ty, value])
}

/// `length of items`, a list of `elem`.
pub fn len(elem: Term, items: Term) -> Term {
    apps(g(LEN), &[elem, items])
}

/// The item at the 0-based `offset` of `items`, a list of `elem`.
pub fn nth(elem: Term, items: Term, offset: Term) -> Term {
    apps(g(NTH), &[elem, items, offset])
}

/// `item index of items`, a list of `elem`.
pub fn index(elem: Term, items: Term, index: Term) -> Term {
    apps(g(INDEX), &[elem, items, index])
}

fn nat_add(a: Term, b: Term) -> Term {
    apps(g("Nat_add"), &[a, b])
}

fn nat_mul(a: Term, b: Term) -> Term {
    apps(g("Nat_mul"), &[a, b])
}

/// `a + a`.
fn twice(a: Term) -> Term {
    nat_add(a.clone(), a)
}

fn refl(ty: Term, value: Term) -> Term {
    apps(g("refl"), &[ty, value])
}

fn pair_snd(pair: Term) -> Term {
    apps(g("Prod_snd"), &[nat(), nat(), pair])
}

/// The remainder of `n` by `k + 1`, the second component of `Nat_divModSucc n k`.
fn nat_rem(n: Term, k: Term) -> Term {
    pair_snd(nat_divmod(n, k))
}

/// `Nat_divModSucc n k`: the quotient and remainder of `n` by `k + 1`.
fn nat_divmod(n: Term, k: Term) -> Term {
    apps(g("Nat_divModSucc"), &[n, k])
}

fn nat_pair() -> Term {
    apps(g("Prod"), &[nat(), nat()])
}

fn nat_pair_of(q: Term, r: Term) -> Term {
    apps(g("Prod_mk"), &[nat(), nat(), q, r])
}

// ----- The definitions --------------------------------------------------

/// Every LOGOS operation the kernel theorems mention, as `(name, type, body)`, each after the ones it uses.
///
/// - `logos_tdiv : Int → Int → Int` divides the dividend's magnitude by the divisor with the standard library's
///   Euclidean `div`, which rounds toward zero on a nonnegative dividend, and negates the quotient for a negative
///   dividend: the quotient rounded toward zero, as the runtime's `/` on integers is. It is Lean's `Int.tdiv`. A zero
///   divisor gives 0, where the runtime raises; no theorem's statement divides by zero.
/// - `logos_tmod : Int → Int → Int` is the same with the Euclidean `mod`: the remainder `a - b · (a / b)`, which takes
///   the dividend's sign, as the runtime's `%` on integers does. It is Lean's `Int.tmod`. A zero divisor gives the
///   dividend, where the runtime raises; no theorem's statement takes a remainder by zero.
/// - `logos_shr : Int → Nat → Int` is the Euclidean quotient by `2^s`, which for a positive divisor rounds toward
///   negative infinity: the runtime's arithmetic `>>`. It is Lean's `Int.shiftRight` (`Int.shiftRight_eq_div_pow`).
///   The runtime reduces the amount mod 64, so the two agree for amounts below 64; a rewrite shifts by at most 62.
/// - `logos_shl : Int → Nat → Int` is the exact product by `2^s` read back as an `i64`, which is what the runtime's
///   `wrapping_shl` computes: a product `ofNat m` keeps `m mod 2^64`, a product `negSucc k` (that is, `-(k + 1)`)
///   keeps `-((k mod 2^64) + 1)`, and a value outside `[-2^63, 2^63)` moves by `2^64` into it. The runtime reduces the
///   amount mod 64, so the two agree for amounts below 64; a rewrite shifts by at most 62.
/// - `logos_copy : Π (T : Type), T → T` is the identity. The runtime's copy is a deep clone, a fresh value whose
///   contents equal the original's, so as a function from values to values it is the identity.
/// - `logos_len : Π (A : Type), List A → Int` is the number of items.
/// - `logos_nth : Π (A : Type), List A → Nat → Option A` is the item at a 0-based offset, `none` past the end.
/// - `logos_index : Π (A : Type), List A → Int → Option A` reads `item i of xs`: index 1 is the first item, a
///   negative index counts from the back (-1 is the last), and 0 or an index past either end names no item, the
///   runtime's error, which is `none` here.
/// - `List_take : Π (A : Type), Nat → List A → List A` and `List_drop : Π (A : Type), Nat → List A → List A` split a
///   list after `count` items, by recursion on the count. They are Lean's `List.take` and `List.drop`.
/// - `logos_slice : Π (A : Type), List A → Int → Int → List A` reads `items start through end of xs` as the runtime's
///   `logos_slice_range` does: the items at the 0-based offsets from `start - 1` (0 for a start of 0) up to but not
///   including `end`, when `end` is at most the length, and no items otherwise. A negative start or end gives no
///   items: the runtime reads either as a 64-bit word of at least `2^63 - 1`, which no list's length reaches.
/// - `logos_bitwise : (Bool → Bool → Bool) → Nat → Nat → Nat → Nat` is Lean's `Nat.bitwise f m n` with at most
///   `fuel` halvings: the bits of `m` and `n` combined by `f` from the lowest, until an operand runs out, after which
///   the rest of the other is kept when `f` keeps a bit against a 0 and dropped when it does not. It accumulates
///   `acc + w · bit` with the weight `w` doubling at each bit, and reads each quotient, remainder and sum out of the
///   `ofNat` the standard library returns, so every value it passes on is a literal once its operands are.
/// - `logos_and`, `logos_or`, `logos_xor : Int → Int → Int` are Lean's `Int.land`, `Int.lor` and `Int.xor`: the
///   operation on the two's-complement expansions, where `negSucc m`, which is `-(m + 1)`, is the complement of `m`.
///   On two `i64` operands each is the runtime's `&`, `|` or `^`, and its value is again an `i64`. They allow
///   [`WORD_HALVINGS`] halvings, enough for an `i64`, whose bits past the sign number at most 63; the runtime's `&`
///   also takes a nonnegative bignum and a low-bit mask, whose bits run out first.
pub fn definitions() -> Vec<(String, Term, Term)> {
    let element = || v("A");
    let binary = || arrow(int(), arrow(int(), int()));
    vec![
        (TDIV.to_string(), binary(), by_dividend_sign(int_div)),
        (TMOD.to_string(), binary(), by_dividend_sign(int_mod)),
        (SHR.to_string(), arrow(int(), arrow(nat(), int())), shr_body()),
        (SHL.to_string(), arrow(int(), arrow(nat(), int())), shl_body()),
        (
            COPY.to_string(),
            pi("T", type0(), arrow(v("T"), v("T"))),
            lam("T", type0(), lam("value", v("T"), v("value"))),
        ),
        (
            LEN.to_string(),
            pi("A", type0(), arrow(list(element()), int())),
            lam("A", type0(), lam("items", list(element()), of_nat(list_length(element(), v("items"))))),
        ),
        (
            NTH.to_string(),
            pi("A", type0(), arrow(list(element()), arrow(nat(), option(element())))),
            nth_body(),
        ),
        (
            INDEX.to_string(),
            pi("A", type0(), arrow(list(element()), arrow(int(), option(element())))),
            index_body(),
        ),
        (TAKE.to_string(), pi("A", type0(), arrow(nat(), arrow(list(element()), list(element())))), take_body()),
        (DROP.to_string(), pi("A", type0(), arrow(nat(), arrow(list(element()), list(element())))), drop_body()),
        (
            SLICE.to_string(),
            pi("A", type0(), arrow(list(element()), arrow(int(), arrow(int(), list(element()))))),
            slice_body(),
        ),
        (
            BITWISE.to_string(),
            arrow(combiner_type(), arrow(nat(), arrow(nat(), arrow(nat(), nat())))),
            bitwise_body(),
        ),
        (AND.to_string(), binary(), and_body()),
        (OR.to_string(), binary(), or_body()),
        (XOR.to_string(), binary(), xor_body()),
    ]
}

fn combiner_type() -> Term {
    arrow(boolean(), arrow(boolean(), boolean()))
}

/// `λ dividend divisor. match dividend { ofNat p ⇒ op (ofNat p) divisor | negSucc p ⇒ -(op (ofNat (p + 1)) divisor) }`:
/// the Euclidean `op` on the dividend's magnitude, which agrees with truncation there, carrying the dividend's sign.
fn by_dividend_sign(op: fn(Term, Term) -> Term) -> Term {
    lam(
        "dividend",
        int(),
        lam(
            "divisor",
            int(),
            mat(
                v("dividend"),
                lam("_", int(), int()),
                vec![
                    lam("p", nat(), op(of_nat(v("p")), v("divisor"))),
                    lam("p", nat(), int_neg(op(of_nat(succ(v("p"))), v("divisor")))),
                ],
            ),
        ),
    )
}

fn shr_body() -> Term {
    lam("value", int(), lam("amount", nat(), int_div(v("value"), of_nat(power_of_two(v("amount"))))))
}

fn shl_body() -> Term {
    lam("value", int(), lam("amount", nat(), wrapped(mul(v("value"), doubled(v("amount"))))))
}

/// Whether the remainder by 2 that `remainder` computes is 1: the low bit of the dividend.
fn low_bit(remainder: Term) -> Term {
    let nonzero = mat(v("r"), lam("_", nat(), boolean()), vec![g("false"), lam("_", nat(), g("true"))]);
    mat(remainder, lam("_", int(), boolean()), vec![lam("r", nat(), nonzero), lam("_", nat(), g("false"))])
}

/// `ofNat x / 2` and the low bit of `x`, through the standard library's `div` and `mod`.
fn halved(x: Term) -> (Term, Term) {
    (int_div(of_nat(x.clone()), int_literal(2)), low_bit(int_mod(of_nat(x), int_literal(2))))
}

/// The step of the loop of [`BITWISE`] past two nonzero operands `m` and `n`, given each as its half and its low
/// bit: the low bits combined by `combine` add the weight `w` to `acc` when the result is set, and `recur` continues
/// with `fuel` from the halves, the weight doubled. The halves, the new `acc` and the new weight are each read out of
/// the `ofNat` they compute to.
fn bitwise_step(combine: &Term, recur: Term, fuel: Term, m: (Term, Term), n: (Term, Term)) -> Term {
    let ((half_m, low_m), (half_n, low_n)) = (m, n);
    let set = apps(combine.clone(), &[low_m, low_n]);
    let acc = if_bool(set, int(), add(of_nat(v("acc")), of_nat(v("w"))), of_nat(v("acc")));
    let w = add(of_nat(v("w")), of_nat(v("w")));
    let next = apps(recur, &[fuel, v("hm"), v("hn"), v("acc2"), v("w2")]);
    forced(half_m, "hm", forced(half_n, "hn", forced(acc, "acc2", forced(w, "w2", next))))
}

/// The loop of [`BITWISE`] with the combiner `combine`: `fix go. λ fuel m n acc w. …`, whose value is
/// `acc + w · (m combined with n)`.
pub fn bitwise_loop(combine: Term) -> Term {
    let by = |a: Term, b: Term| apps(combine.clone(), &[a, b]);
    // `acc + w · v` once an operand runs out, `v` the rest of the other operand or 0.
    let rest = |keep: Term, other: Term| {
        nat_of(add(of_nat(v("acc")), mul(of_nat(v("w")), of_nat(if_bool(keep, nat(), other, g("Zero"))))))
    };
    let step = bitwise_step(&combine, v("go"), v("k"), halved(v("m")), halved(v("n")));
    let past_n = mat(
        v("n"),
        lam("_", nat(), nat()),
        vec![rest(by(g("true"), g("false")), v("m")), lam("_", nat(), step)],
    );
    let past_m = mat(
        v("m"),
        lam("_", nat(), nat()),
        vec![rest(by(g("false"), g("true")), v("n")), lam("_", nat(), past_n)],
    );
    let by_fuel = mat(v("fuel"), lam("_", nat(), nat()), vec![v("acc"), lam("k", nat(), past_m)]);
    let params = lam(
        "fuel",
        nat(),
        lam("m", nat(), lam("n", nat(), lam("acc", nat(), lam("w", nat(), by_fuel)))),
    );
    Term::Fix { name: "go".to_string(), body: Box::new(params) }
}

fn bitwise_body() -> Term {
    let start = apps(bitwise_loop(v("f")), &[v("fuel"), v("m"), v("n"), g("Zero"), one()]);
    lam("f", combiner_type(), lam("fuel", nat(), lam("m", nat(), lam("n", nat(), start))))
}

/// `combine` on the magnitudes `m` and `n` with [`WORD_HALVINGS`] halvings.
fn on_magnitudes(combine: BitOp, m: &str, n: &str) -> Term {
    bitwise(combine.combiner(), nat_literal(BigInt::from_u64(WORD_HALVINGS)), v(m), v(n))
}

/// A bitwise operation on two's-complement integers, by the signs of its operands as Lean's `Int.land`, `Int.lor`
/// and `Int.xor` take them: `negSucc m` is `-(m + 1)`, the complement `!m`, so each case is an operation on the
/// magnitudes, complemented where the result is negative. `cases[i][j]` is the case for the first operand's sign `i`
/// and the second's `j`, `0` for `ofNat` and `1` for `negSucc`; it names the magnitudes `m` and `n`.
fn by_signs(cases: [[Term; 2]; 2]) -> Term {
    let [[both, second], [first, neither]] = cases;
    let by_second = |nonnegative: Term, negative: Term| {
        mat(v("y"), lam("_", int(), int()), vec![lam("n", nat(), nonnegative), lam("n", nat(), negative)])
    };
    lam(
        "x",
        int(),
        lam(
            "y",
            int(),
            mat(
                v("x"),
                lam("_", int(), int()),
                vec![lam("m", nat(), by_second(both, second)), lam("m", nat(), by_second(first, neither))],
            ),
        ),
    )
}

fn and_body() -> Term {
    by_signs([
        [of_nat(on_magnitudes(BitOp::And, "m", "n")), of_nat(on_magnitudes(BitOp::AndNot, "m", "n"))],
        [of_nat(on_magnitudes(BitOp::AndNot, "n", "m")), neg_succ(on_magnitudes(BitOp::Or, "m", "n"))],
    ])
}

fn or_body() -> Term {
    by_signs([
        [of_nat(on_magnitudes(BitOp::Or, "m", "n")), neg_succ(on_magnitudes(BitOp::AndNot, "n", "m"))],
        [neg_succ(on_magnitudes(BitOp::AndNot, "m", "n")), neg_succ(on_magnitudes(BitOp::And, "m", "n"))],
    ])
}

fn xor_body() -> Term {
    by_signs([
        [of_nat(on_magnitudes(BitOp::Xor, "m", "n")), neg_succ(on_magnitudes(BitOp::Xor, "m", "n"))],
        [neg_succ(on_magnitudes(BitOp::Xor, "m", "n")), of_nat(on_magnitudes(BitOp::Xor, "m", "n"))],
    ])
}

/// `2^n` as an `Int`: `1` doubled `n` times through the standard library's `mul`, so that it computes natively on a
/// literal `n`, where `Nat_pow` unfolds its multiplications one successor at a time.
fn doubled(n: Term) -> Term {
    let two = of_nat(succ(succ(g("Zero"))));
    let step = lam("j", nat(), mul(app(v("power"), v("j")), two));
    let body = lam("k", nat(), mat(v("k"), lam("_", nat(), int()), vec![of_nat(one()), step]));
    app(Term::Fix { name: "power".to_string(), body: Box::new(body) }, n)
}

/// `y` read back as an `i64`: `y` itself on `[-2^63, 2^63)`, moved by a multiple of `2^64` into it elsewhere.
/// `ofNat m` keeps `r = m mod 2^64`, which is `r` below `2^63` and `r - 2^64` from there. `negSucc k` is `-(k + 1)`
/// and keeps `-(s + 1)` for `s = k mod 2^64`, which is `negSucc s` for `s` below `2^63` and `2^64 - 1 - s` from
/// there.
///
/// The remainder and the two differences go through the standard library's `mod` and `sub`, which compute natively
/// on literals. The remainder is read out of the `ofNat` it returns; a remainder by `2^64` is never negative, so the
/// `negSucc` case is never taken.
fn wrapped(y: Term) -> Term {
    mat(
        y,
        lam("_", int(), int()),
        vec![lam("m", nat(), wrapped_from_zero(v("m"))), lam("k", nat(), wrapped_below_zero(v("k")))],
    )
}

/// `ofNat m` read back as an `i64`.
fn wrapped_from_zero(m: Term) -> Term {
    by_remainder(m, of_nat, less_a_word)
}

/// `negSucc k` read back as an `i64`.
fn wrapped_below_zero(k: Term) -> Term {
    by_remainder(k, neg_succ, word_complement)
}

/// With `r = n mod 2^64`: `low r` when `r < 2^63`, and `high r` otherwise.
fn by_remainder(n: Term, low: fn(Term) -> Term, high: fn(Term) -> Term) -> Term {
    let by_half =
        mat(apps(g("natLt"), &[v("r"), word_half()]), lam("_", boolean(), int()), vec![low(v("r")), high(v("r"))]);
    mat(remainder(n), lam("_", int(), int()), vec![lam("r", nat(), by_half), lam("_", nat(), of_nat(g("Zero")))])
}

/// `ofNat n mod 2^64`.
fn remainder(n: Term) -> Term {
    apps(g("mod"), &[of_nat(n), of_nat(word_count())])
}

/// `r - 2^64`.
fn less_a_word(r: Term) -> Term {
    apps(g("sub"), &[of_nat(r), of_nat(word_count())])
}

/// `2^64 - 1 - s`.
fn word_complement(s: Term) -> Term {
    apps(g("sub"), &[of_nat(word_top()), of_nat(s)])
}

fn nth_body() -> Term {
    let element = || v("A");
    let found = || option(element());
    let at_head = mat(
        v("offset"),
        lam("_", nat(), found()),
        vec![
            apps(g("some"), &[element(), v("head")]),
            lam("k", nat(), apps(v("nth"), &[v("tail"), v("k")])),
        ],
    );
    let walk = lam(
        "items",
        list(element()),
        lam(
            "offset",
            nat(),
            mat(
                v("items"),
                lam("_", list(element()), found()),
                vec![app(g("none"), element()), lam("head", element(), lam("tail", list(element()), at_head))],
            ),
        ),
    );
    lam("A", type0(), Term::Fix { name: "nth".to_string(), body: Box::new(walk) })
}

fn index_body() -> Term {
    let element = || v("A");
    let found = || option(element());
    let count = || list_length(element(), v("items"));
    let from_the_front = lam(
        "p",
        nat(),
        mat(
            v("p"),
            lam("_", nat(), found()),
            vec![app(g("none"), element()), lam("k", nat(), nth(element(), v("items"), v("k")))],
        ),
    );
    let from_the_back = lam(
        "k",
        nat(),
        mat(
            apps(g("natLt"), &[v("k"), count()]),
            lam("_", boolean(), found()),
            vec![
                nth(element(), v("items"), apps(g("Nat_sub"), &[count(), succ(v("k"))])),
                app(g("none"), element()),
            ],
        ),
    );
    lam(
        "A",
        type0(),
        lam(
            "items",
            list(element()),
            lam(
                "index",
                int(),
                mat(v("index"), lam("_", int(), found()), vec![from_the_front, from_the_back]),
            ),
        ),
    )
}

/// `λ A. fix name. λ count items. match count { 0 ⇒ at_zero | k + 1 ⇒ match items { [] ⇒ [] | head :: tail ⇒
/// at_cons } }`, where `at_cons` may recur on `k` and `tail`.
fn by_count(name: &str, at_zero: Term, at_cons: Term) -> Term {
    let element = || v("A");
    let items = || list(element());
    let at_successor = mat(
        v("items"),
        lam("_", items(), items()),
        vec![nil(element()), lam("head", element(), lam("tail", items(), at_cons))],
    );
    let walk = lam(
        "count",
        nat(),
        lam("items", items(), mat(v("count"), lam("_", nat(), items()), vec![at_zero, lam("k", nat(), at_successor)])),
    );
    lam("A", type0(), Term::Fix { name: name.to_string(), body: Box::new(walk) })
}

fn take_body() -> Term {
    let kept = cons(v("A"), v("head"), apps(v("take"), &[v("k"), v("tail")]));
    by_count("take", nil(v("A")), kept)
}

fn drop_body() -> Term {
    by_count("drop", v("items"), apps(v("drop"), &[v("k"), v("tail")]))
}

fn slice_body() -> Term {
    let element = || v("A");
    let items = || list(element());
    let none = || nil(element());
    let from = || app(g("Nat_pred"), v("s"));
    let within = mat(
        nat_le(v("e"), list_length(element(), v("items"))),
        lam("_", boolean(), items()),
        vec![list_take(element(), nat_sub(v("e"), from()), list_drop(element(), from(), v("items"))), none()],
    );
    let by_end = mat(v("end"), lam("_", int(), items()), vec![lam("e", nat(), within), lam("_", nat(), none())]);
    let by_start = mat(v("start"), lam("_", int(), items()), vec![lam("s", nat(), by_end), lam("_", nat(), none())]);
    lam("A", type0(), lam("items", items(), lam("start", int(), lam("end", int(), by_start))))
}

/// The definitions named, in the order [`definitions`] gives them.
fn semantics(names: &[&str]) -> Vec<(String, Term, Term)> {
    definitions().into_iter().filter(|(name, _, _)| names.contains(&name.as_str())).collect()
}

// ----- Proof procedures -------------------------------------------------

/// The leading `Π` binders of `statement`, outermost first.
fn binders(statement: &Term) -> Vec<(String, Term)> {
    let mut found = Vec::new();
    let mut rest = statement;
    while let Term::Pi { param, param_type, body_type } = rest {
        found.push((param.clone(), (**param_type).clone()));
        rest = body_type;
    }
    found
}

fn proved_by(
    statement: Term,
    definitions: Vec<(String, Term, Term)>,
    script: &[TacticStep],
) -> Result<KernelCertificate, String> {
    let ctx = certificate_context(&[], &definitions)?;
    let mut state = GoalState::new(&ctx, &statement).map_err(|e| e.to_string())?;
    run(&mut state, script)?;
    let proof = state.qed().map_err(|e| e.to_string())?;
    Ok(KernelCertificate { variables: Vec::new(), definitions, statement, proof })
}

/// `script` run on `state` step by step, each step applied in place. [`GoalState::run`] keeps a snapshot of the whole
/// state before every step to restore when the step fails; a failing step here fails the proof, so there is nothing
/// to restore, and the snapshots, not the steps, are most of a long script's cost.
fn run(state: &mut GoalState, script: &[TacticStep]) -> Result<(), String> {
    for (position, step) in script.iter().enumerate() {
        apply(state, step).map_err(|e| format!("step {position} (`{step}`): {e}"))?;
    }
    Ok(())
}

/// Each case's names, as [`GoalState::induction`] and [`GoalState::cases`] take them.
fn case_names(cases: &[Vec<String>]) -> Vec<Vec<&str>> {
    cases.iter().map(|names| names.iter().map(String::as_str).collect()).collect()
}

/// `step` applied to `state` in place.
fn apply(state: &mut GoalState, step: &TacticStep) -> Result<(), TacticError> {
    match step {
        TacticStep::Intro(binders) => binders.iter().try_for_each(|name| state.intro(name)),
        TacticStep::Exact(term) => state.exact(term),
        TacticStep::Have(name, fact) => state.have(name, fact),
        TacticStep::Rfl => state.rfl(),
        TacticStep::Change(target) => state.change(target),
        TacticStep::Rewrite(equation, reverse) => state.rewrite(equation, *reverse),
        TacticStep::Exfalso => state.exfalso(),
        TacticStep::Induction(variable, cases) => {
            let names = case_names(cases);
            let names: Vec<&[&str]> = names.iter().map(Vec::as_slice).collect();
            state.induction(variable, &names)
        }
        TacticStep::Cases(variable, cases) => {
            let names = case_names(cases);
            let names: Vec<&[&str]> = names.iter().map(Vec::as_slice).collect();
            state.cases(variable, &names)
        }
        other => state.step(other),
    }
}

/// `statement` proved by computation: every binder introduced, then the two sides of the equation convert.
pub fn prove_by_computation(
    statement: &Term,
    definitions: Vec<(String, Term, Term)>,
) -> Result<KernelCertificate, String> {
    let names = binders(statement).into_iter().map(|(name, _)| name).collect();
    proved_by(statement.clone(), definitions, &[TacticStep::Intro(names), TacticStep::Rfl])
}

/// `statement` proved by cases on the first `Int` it binds: every binder introduced, the integer split into
/// `ofNat` and `negSucc`, and each case closed by computation, either because its two sides convert or because a
/// hypothesis computes to `Eq Bool false true`. The first case neither closes is the error.
pub fn prove_by_int_cases(
    statement: &Term,
    definitions: Vec<(String, Term, Term)>,
) -> Result<KernelCertificate, String> {
    let binders = binders(statement);
    let split = binders
        .iter()
        .find(|(_, ty)| *ty == int())
        .map(|(name, _)| name.clone())
        .ok_or_else(|| "the statement binds no Int to split".to_string())?;
    let ctx = certificate_context(&[], &definitions)?;
    let mut state = GoalState::new(&ctx, statement).map_err(|e| e.to_string())?;
    for (name, _) in &binders {
        state.intro(name).map_err(|e| e.to_string())?;
    }
    state.cases(&split, &[&[], &[]]).map_err(|e| e.to_string())?;
    while let Some(goal) = state.goals().into_iter().next() {
        if state.rfl().is_ok() {
            continue;
        }
        state = refuted(&state, &goal).ok_or_else(|| format!("a case where the sides differ:\n{goal}"))?;
    }
    let proof = state.qed().map_err(|e| e.to_string())?;
    Ok(KernelCertificate { variables: Vec::new(), definitions, statement: statement.clone(), proof })
}

/// `state` with its focused goal closed by a hypothesis that computes to `Eq Bool false true`.
fn refuted(state: &GoalState, goal: &Goal) -> Option<GoalState> {
    goal.hyps.iter().find_map(|(hypothesis, _)| {
        let mut attempt = state.clone();
        attempt.exfalso().ok()?;
        attempt.exact(&app(g("Eq_false_true_absurd"), v(hypothesis))).ok()?;
        Some(attempt)
    })
}

/// The integer pair `div` and `mod` of a nonnegative dividend by a positive divisor read out of the natural pair
/// `pair`: `match pair { (q, r) ⇒ (ofNat q, ofNat r) }`.
fn signed_pair(pair: Term) -> Term {
    let int_pair = apps(g("Prod"), &[int(), int()]);
    let both = apps(g("Prod_mk"), &[int(), int(), of_nat(v("q")), of_nat(v("r"))]);
    mat(pair, lam("_", nat_pair(), int_pair), vec![lam("q", nat(), lam("r", nat(), both))])
}

/// `div (ofNat n) (ofNat (k + 1))` as it unfolds, from `pair = Nat_divModSucc n k`.
fn quotient_of(pair: Term) -> Term {
    apps(g("Prod_fst"), &[int(), int(), signed_pair(pair)])
}

/// `mod (ofNat n) (ofNat (k + 1))` as it unfolds, from `pair = Nat_divModSucc n k`.
fn remainder_of(pair: Term) -> Term {
    apps(g("Prod_snd"), &[int(), int(), signed_pair(pair)])
}

/// `∀ j, Nat_divModSucc j 0 = (j, 0)`.
fn divmod_by_one_statement() -> Term {
    pi("j", nat(), eq(nat_pair(), nat_divmod(v("j"), g("Zero")), nat_pair_of(v("j"), g("Zero"))))
}

/// The proof of [`divmod_by_one_statement`], by induction on `j`.
fn divmod_by_one_script() -> Vec<TacticStep> {
    use TacticStep as T;
    let unfolded_step = mat(
        nat_divmod(v("p"), g("Zero")),
        lam("_", nat_pair(), nat_pair()),
        vec![lam(
            "q",
            nat(),
            lam(
                "r",
                nat(),
                mat(
                    apps(g("natLt"), &[v("r"), g("Zero")]),
                    lam("_", boolean(), nat_pair()),
                    vec![nat_pair_of(v("q"), succ(v("r"))), nat_pair_of(succ(v("q")), g("Zero"))],
                ),
            ),
        )],
    );
    vec![
        T::Intro(names(&["j"])),
        T::Induction("j".to_string(), vec![Vec::new(), names(&["p", "ih"])]),
        T::Rfl,
        T::Change(eq(nat_pair(), unfolded_step, nat_pair_of(succ(v("p")), g("Zero")))),
        T::Rewrite(v("ih"), false),
        T::Rfl,
    ]
}

/// One step of `Nat_divModSucc` by `k + 1` from the quotient and remainder `pair`:
/// `match pair { (q, r) ⇒ if r < k then (q, r + 1) else (q + 1, 0) }`.
fn divmod_step(pair: Term, k: Term) -> Term {
    let advance = mat(
        apps(g("natLt"), &[v("rest"), k]),
        lam("_", boolean(), nat_pair()),
        vec![nat_pair_of(v("quot"), succ(v("rest"))), nat_pair_of(succ(v("quot")), g("Zero"))],
    );
    mat(pair, lam("_", nat_pair(), nat_pair()), vec![lam("quot", nat(), lam("rest", nat(), advance))])
}

/// `2^j - 1` as a `Nat`: 0, then `2 · mask (j - 1) + 1`.
fn mask(j: Term) -> Term {
    let body = lam(
        "j",
        nat(),
        mat(
            v("j"),
            lam("_", nat(), nat()),
            vec![g("Zero"), lam("i", nat(), succ(twice(app(v("mask"), v("i")))))],
        ),
    );
    app(Term::Fix { name: "mask".to_string(), body: Box::new(body) }, j)
}

/// The remainder of `x` by `2 · (q + 1)`, from `pair = Nat_divModSucc x 1`, the half and the low bit of `x`:
/// `2 · (half mod (q + 1)) + low`.
fn remainder_by_double(pair: Term, q: Term) -> Term {
    let doubled = || twice(nat_rem(v("half"), q.clone()));
    let by_low = mat(v("low"), lam("_", nat(), nat()), vec![doubled(), lam("_", nat(), succ(doubled()))]);
    mat(pair, lam("_", nat_pair(), nat()), vec![lam("half", nat(), lam("low", nat(), by_low))])
}

/// The steps that prove `goal(c)`, for a Bool term `c` the goal matches on, from `goal(b)` for a fresh `b`: the case
/// `b = true` by `on_true`, which may cite `known : Eq Bool c true`, and `b = false` by `on_false`, which may cite
/// `known : Eq Bool c false`.
fn by_bool(
    name: &str,
    c: Term,
    goal: impl Fn(Term) -> Term,
    on_true: Vec<TacticStep>,
    on_false: Vec<TacticStep>,
) -> Vec<TacticStep> {
    use TacticStep as T;
    let known = |b: Term| eq(boolean(), c.clone(), b);
    let mut steps = vec![
        T::Have(name.to_string(), pi("b", boolean(), arrow(known(v("b")), goal(v("b"))))),
        T::Intro(names(&["b", "known"])),
        T::Cases("b".to_string(), vec![Vec::new(), Vec::new()]),
    ];
    steps.extend(on_true);
    steps.extend(on_false);
    steps.push(T::Exact(apps(v(name), &[c.clone(), refl(boolean(), c)])));
    steps
}

// ----- The theorems -----------------------------------------------------

/// `div-pow2-shr`: `∀ (x : Int) (n : Nat), 0 ≤ x → x / 2^n = x >> n`. In the `ofNat` case both sides compute to
/// `ofNat (m / 2^n)`; in the `negSucc` case the guard computes to `Eq Bool false true`. Without the guard there is
/// no proof: `-1 / 2` is 0 and `-1 >> 1` is -1.
pub fn div_pow2_shr() -> Result<KernelCertificate, String> {
    let guard = eq(boolean(), apps(g("Int_le"), &[of_nat(g("Zero")), v("x")]), g("true"));
    let identity = eq(int(), tdiv(v("x"), of_nat(power_of_two(v("n")))), shr(v("x"), v("n")));
    let statement = pi("x", int(), pi("n", nat(), pi("h", guard, identity)));
    prove_by_int_cases(&statement, semantics(&[TDIV, SHR]))
}

/// `div-one`: `∀ (x : Int), x / 1 = x`. `x / 1` divides the magnitude of `x` by 1 with the standard library's `div`,
/// whose quotient is the first component of `Nat_divModSucc m 0`, and `Nat_divModSucc j 0 = (j, 0)` by induction on
/// `j`, the step unfolding one recursion of `Nat_divModSucc` and rewriting by the hypothesis. So the quotient is the
/// magnitude, which `x / 1` gives back with the sign of `x`.
pub fn div_one() -> Result<KernelCertificate, String> {
    use TacticStep as T;
    let int_one = || of_nat(one());
    let statement = pi("x", int(), eq(int(), tdiv(v("x"), int_one()), v("x")));
    let mut script = vec![T::Have("divmod_by_one".to_string(), divmod_by_one_statement())];
    script.extend(divmod_by_one_script());
    script.extend([
        T::Have("by_one".to_string(), pi("k", nat(), eq(int(), int_div(of_nat(v("k")), int_one()), of_nat(v("k"))))),
        T::Intro(names(&["k"])),
        T::Change(eq(int(), quotient_of(nat_divmod(v("k"), g("Zero"))), of_nat(v("k")))),
        T::Rewrite(app(v("divmod_by_one"), v("k")), false),
        T::Rfl,
        T::Intro(names(&["x"])),
        T::Cases("x".to_string(), vec![names(&["m"]), names(&["k"])]),
        T::Exact(app(v("by_one"), v("m"))),
        T::Change(eq(int(), int_neg(int_div(of_nat(succ(v("k"))), int_one())), neg_succ(v("k")))),
        T::Rewrite(app(v("by_one"), succ(v("k"))), false),
        T::Rfl,
    ]);
    proved_by(statement, semantics(&[TDIV]), &script)
}

/// `len-copy`: `∀ (A : Type) (xs : List A), length of (copy of xs) = length of xs`, by computation.
pub fn len_copy() -> Result<KernelCertificate, String> {
    let element = || v("A");
    let copied = copy(list(element()), v("xs"));
    let statement = pi(
        "A",
        type0(),
        pi("xs", list(element()), eq(int(), len(element(), copied), len(element(), v("xs")))),
    );
    prove_by_computation(&statement, semantics(&[COPY, LEN]))
}

/// `index-copy`: `∀ (A : Type) (xs : List A) (i : Int), item i of (copy of xs) = item i of xs`, as `Option A`, so
/// an index that names no item names none on both sides; by computation.
pub fn index_copy() -> Result<KernelCertificate, String> {
    let element = || v("A");
    let copied = copy(list(element()), v("xs"));
    let identity = eq(
        option(element()),
        index(element(), copied, v("i")),
        index(element(), v("xs"), v("i")),
    );
    let statement = pi("A", type0(), pi("xs", list(element()), pi("i", int(), identity)));
    prove_by_computation(&statement, semantics(&[COPY, NTH, INDEX]))
}

/// `copy-copy`: `∀ (T : Type) (x : T), copy of (copy of x) = copy of x`, by computation.
pub fn copy_copy() -> Result<KernelCertificate, String> {
    let identity = eq(v("T"), copy(v("T"), copy(v("T"), v("x"))), copy(v("T"), v("x")));
    let statement = pi("T", type0(), pi("x", v("T"), identity));
    prove_by_computation(&statement, semantics(&[COPY]))
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// The steps that close a goal from a hypothesis `h` computing to `Eq Bool false true`.
fn refute(h: &str) -> [TacticStep; 2] {
    [TacticStep::Exfalso, TacticStep::Exact(app(g("Eq_false_true_absurd"), v(h)))]
}

/// `slice-full-copy`: `∀ (A : Type) (xs : List A), items 1 through (length of xs) of xs = copy of xs`. The slice
/// unfolds to `take (length xs) xs` once `natLe (length xs) (length xs)` is rewritten to `true` by `Nat_le_refl`, and
/// `take (length ys) ys = ys` by induction on `ys`, the empty list included.
pub fn slice_full_copy() -> Result<KernelCertificate, String> {
    use TacticStep as T;
    let element = || v("A");
    let items = || list(element());
    let length = |xs: Term| list_length(element(), xs);
    let whole = slice(element(), v("xs"), of_nat(one()), len(element(), v("xs")));
    let statement = pi("A", type0(), pi("xs", items(), eq(items(), whole, copy(items(), v("xs")))));
    let take_all = |ys: Term| eq(items(), list_take(element(), length(ys.clone()), ys.clone()), ys);
    let count = || length(v("xs"));
    let unfolded = mat(
        nat_le(count(), count()),
        lam("_", boolean(), items()),
        vec![list_take(element(), count(), v("xs")), nil(element())],
    );
    let script = [
        T::Intro(names(&["A"])),
        T::Have("take_all".to_string(), pi("ys", items(), take_all(v("ys")))),
        T::Intro(names(&["ys"])),
        T::Induction("ys".to_string(), vec![Vec::new(), names(&["head", "tail", "ih"])]),
        T::Rfl,
        T::Change(eq(
            items(),
            cons(element(), v("head"), list_take(element(), length(v("tail")), v("tail"))),
            cons(element(), v("head"), v("tail")),
        )),
        T::Rewrite(v("ih"), false),
        T::Rfl,
        T::Intro(names(&["xs"])),
        T::Change(eq(items(), unfolded, v("xs"))),
        T::Rewrite(app(g("Nat_le_refl"), count()), false),
        T::Exact(app(v("take_all"), v("xs"))),
    ];
    proved_by(statement, semantics(&[COPY, LEN, TAKE, DROP, SLICE]), &script)
}

/// `len-slice-bounds`: `∀ (A : Type) (xs : List A) (a b : Int), 1 ≤ a → a ≤ b + 1 → b ≤ length of xs →
/// length of (items a through b of xs) = b - a + 1`.
///
/// The guards leave `a = p + 1` and `b = q` for naturals `p ≤ q ≤ length xs`, where the slice is
/// `take (q - p) (drop p xs)`. Three lemmas, each by induction: `length (take k ys) = k` for `k ≤ length ys`; the
/// window `take (q - p) (drop p ys)` has length `q - p` for `q ≤ length ys`; and `(q - (p + 1)) + 1 = q - p` over
/// `Int` for `p ≤ q`, from `Int.subNatNat (m + 1) (n + 1) = Int.subNatNat m n`. Each case the guards rule out has a
/// guard computing to `Eq Bool false true`. Without `b ≤ length of xs` the identity is false: items 1 through 4 of a
/// list of 3 is empty.
pub fn len_slice_bounds() -> Result<KernelCertificate, String> {
    use TacticStep as T;
    let element = || v("A");
    let items = || list(element());
    let length = |xs: Term| list_length(element(), xs);
    let int_one = || of_nat(one());
    let window = |count: Term, skip: Term, ys: Term| list_take(element(), count, list_drop(element(), skip, ys));
    let sub_nat_nat = |m: Term, n: Term| apps(g("Int_subNatNat"), &[m, n]);
    let one_past = |q: Term, p: Term| int_add(sub_nat_nat(q, succ(p)), int_one());

    let identity = eq(
        int(),
        len(element(), slice(element(), v("xs"), v("a"), v("b"))),
        int_add(int_sub(v("b"), v("a")), int_one()),
    );
    let guards = pi(
        "h1",
        is_true(int_le(int_one(), v("a"))),
        pi(
            "h2",
            is_true(int_le(v("a"), int_add(v("b"), int_one()))),
            pi("h3", is_true(int_le(v("b"), len(element(), v("xs")))), identity),
        ),
    );
    let statement = pi("A", type0(), pi("xs", items(), pi("a", int(), pi("b", int(), guards))));

    let length_take = pi(
        "k",
        nat(),
        pi(
            "ys",
            items(),
            arrow(
                is_true(nat_le(v("k"), length(v("ys")))),
                eq(nat(), length(list_take(element(), v("k"), v("ys"))), v("k")),
            ),
        ),
    );
    let window_length = pi(
        "p",
        nat(),
        pi(
            "q",
            nat(),
            pi(
                "ys",
                items(),
                arrow(
                    is_true(nat_le(v("q"), length(v("ys")))),
                    eq(nat(), length(window(nat_sub(v("q"), v("p")), v("p"), v("ys"))), nat_sub(v("q"), v("p"))),
                ),
            ),
        ),
    );
    let past_by_one = pi(
        "p",
        nat(),
        pi(
            "q",
            nat(),
            arrow(
                is_true(nat_le(v("p"), v("q"))),
                eq(int(), one_past(v("q"), v("p")), of_nat(nat_sub(v("q"), v("p")))),
            ),
        ),
    );
    let subtracted_from_zero = mat(
        nat_sub(g("Zero"), v("q1")),
        lam("_", nat(), int()),
        vec![of_nat(nat_sub(v("q1"), g("Zero"))), lam("k", nat(), neg_succ(v("k")))],
    );
    let in_bounds = mat(
        nat_le(v("q"), length(v("xs"))),
        lam("_", boolean(), items()),
        vec![window(nat_sub(v("q"), v("p1")), v("p1"), v("xs")), nil(element())],
    );

    let mut script = vec![
        T::Intro(names(&["A"])),
        T::Have("length_take".to_string(), length_take),
        T::Intro(names(&["k"])),
        T::Induction("k".to_string(), vec![names(&["ys0", "fits0"]), names(&["k1", "ih", "ys", "fits"])]),
        T::Rfl,
        T::Cases("ys".to_string(), vec![Vec::new(), names(&["head", "tail"])]),
    ];
    script.extend(refute("fits"));
    script.extend([
        T::Change(eq(nat(), succ(length(list_take(element(), v("k1"), v("tail")))), succ(v("k1")))),
        T::Rewrite(apps(v("ih"), &[v("tail"), v("fits")]), false),
        T::Rfl,
        T::Have("window_length".to_string(), window_length),
        T::Intro(names(&["p"])),
        T::Induction("p".to_string(), vec![names(&["q0", "ys0", "fits0"]), names(&["p1", "ih", "q", "ys", "fits"])]),
        T::Exact(apps(v("length_take"), &[v("q0"), v("ys0"), v("fits0")])),
        T::Cases("q".to_string(), vec![Vec::new(), names(&["q1"])]),
        T::Rewrite(app(g("Nat_sub_zero_left"), succ(v("p1"))), false),
        T::Rfl,
        T::Rewrite(apps(g("Nat_sub_succ_succ"), &[v("q1"), v("p1")]), false),
        T::Cases("ys".to_string(), vec![Vec::new(), names(&["head", "tail"])]),
    ]);
    script.extend(refute("fits"));
    script.extend([
        T::Exact(apps(v("ih"), &[v("q1"), v("tail"), v("fits")])),
        T::Have("past_by_one".to_string(), past_by_one),
        T::Intro(names(&["p"])),
        T::Induction("p".to_string(), vec![names(&["q0", "fits0"]), names(&["p1", "ih", "q", "fits"])]),
        T::Cases("q0".to_string(), vec![Vec::new(), names(&["q1"])]),
        T::Rfl,
        T::Rewrite(apps(g("IntLaw_subNatNat_succ_succ"), &[v("q1"), g("Zero")]), false),
        T::Change(eq(int(), int_add(subtracted_from_zero, int_one()), of_nat(succ(v("q1"))))),
        T::Rewrite(app(g("Nat_sub_zero_left"), v("q1")), false),
        T::Rfl,
        T::Cases("q".to_string(), vec![Vec::new(), names(&["q1"])]),
    ]);
    script.extend(refute("fits"));
    script.extend([
        T::Rewrite(apps(g("IntLaw_subNatNat_succ_succ"), &[v("q1"), succ(v("p1"))]), false),
        T::Rewrite(apps(g("Nat_sub_succ_succ"), &[v("q1"), v("p1")]), false),
        T::Exact(apps(v("ih"), &[v("q1"), v("fits")])),
        T::Intro(names(&["xs", "a", "b", "h1", "h2", "h3"])),
        T::Cases("a".to_string(), vec![names(&["p"]), names(&["k"])]),
        T::Cases("p".to_string(), vec![Vec::new(), names(&["p1"])]),
    ]);
    script.extend(refute("h1"));
    script.extend([
        T::Cases("b".to_string(), vec![names(&["q"]), names(&["j"])]),
        T::Have("fits".to_string(), is_true(nat_le(v("q"), length(v("xs"))))),
        T::Exact(v("h3")),
        T::Have("ordered".to_string(), is_true(nat_le(v("p1"), v("q")))),
        T::Exact(v("h2")),
        T::Change(eq(int(), of_nat(length(in_bounds)), one_past(v("q"), v("p1")))),
        T::Rewrite(v("fits"), false),
        T::Change(eq(
            int(),
            of_nat(length(window(nat_sub(v("q"), v("p1")), v("p1"), v("xs")))),
            one_past(v("q"), v("p1")),
        )),
        T::Rewrite(apps(v("window_length"), &[v("p1"), v("q"), v("xs"), v("fits")]), false),
        T::Rewrite(apps(v("past_by_one"), &[v("p1"), v("q"), v("ordered")]), false),
        T::Rfl,
        T::Cases("j".to_string(), vec![Vec::new(), names(&["j1"])]),
    ]);
    script.extend(refute("h2"));
    script.extend(refute("h2"));
    script.extend(refute("h1"));
    proved_by(statement, semantics(&[LEN, TAKE, DROP, SLICE]), &script)
}

/// `mul-pow2-shl`: `∀ (x : Int) (n : Nat), n ≤ 62 → i64::MIN ≤ x · 2^n → x · 2^n ≤ i64::MAX → x << n = x · 2^n`,
/// with `·` the runtime's exact product.
///
/// `x << n` is `x · 2^n` read back as an `i64`, so the theorem is that the reading leaves a value in
/// `[-2^63, 2^63)` alone, proved for any `y` in place of `x · 2^n`. For `y = ofNat m` with `m < 2^63`, and for
/// `y = negSucc k` with `k < 2^63`, the reading takes the remainder by `2^64`, which is the value itself, and compares
/// it with `2^63`, which it is below. The remainder of `j < 2^64` is `j` because `Nat.divModSucc j (2^64 - 1)` is
/// `(0, j)`, by induction on `j`: each step adds one to a remainder that stays below the divisor. `logos_shl` forms
/// `2^n` by doubling `1`, which is `Nat_pow 2 n` by induction on `n`. Without the range guards the identity is false:
/// `2 << 62` is `i64::MIN`, and `2 · 2^62` is `2^63`.
pub fn mul_pow2_shl() -> Result<KernelCertificate, String> {
    use TacticStep as T;
    let product = || mul(v("x"), of_nat(power_of_two(v("n"))));
    let guards = pi(
        "within",
        is_true(nat_le(v("n"), nat_literal(BigInt::from_i64(62)))),
        pi(
            "above",
            is_true(int_le(canonical_int(i64::MIN), product())),
            pi("below", is_true(int_le(product(), canonical_int(i64::MAX))), eq(int(), shl(v("x"), v("n")), product())),
        ),
    );
    let statement = pi("x", int(), pi("n", nat(), guards));

    let nat_pair = || apps(g("Prod"), &[nat(), nat()]);
    let int_pair = || apps(g("Prod"), &[int(), int()]);
    let pair = |q: Term, r: Term| apps(g("Prod_mk"), &[nat(), nat(), q, r]);
    let divmod = |j: Term| apps(g("Nat_divModSucc"), &[j, word_top()]);
    let below_word = |j: Term| is_true(nat_le(j, word_top()));
    let signed = |j: Term| is_true(nat_le(j, signed_top()));
    let pred_le = pi(
        "p",
        nat(),
        pi("k", nat(), arrow(is_true(nat_le(succ(v("p")), v("k"))), is_true(nat_le(v("p"), v("k"))))),
    );
    let divmod_small =
        pi("j", nat(), arrow(below_word(v("j")), eq(nat_pair(), divmod(v("j")), pair(g("Zero"), v("j")))));
    let mod_small = pi("j", nat(), arrow(signed(v("j")), eq(int(), remainder(v("j")), of_nat(v("j")))));
    let in_range = pi(
        "y",
        int(),
        arrow(
            is_true(int_le(canonical_int(i64::MIN), v("y"))),
            arrow(is_true(int_le(v("y"), canonical_int(i64::MAX))), eq(int(), wrapped(v("y")), v("y"))),
        ),
    );
    let step = mat(
        divmod(v("p")),
        lam("_", nat_pair(), nat_pair()),
        vec![lam(
            "q",
            nat(),
            lam(
                "r",
                nat(),
                mat(
                    nat_le(succ(v("r")), word_top()),
                    lam("_", boolean(), nat_pair()),
                    vec![pair(v("q"), succ(v("r"))), pair(succ(v("q")), g("Zero"))],
                ),
            ),
        )],
    );
    let stepped = mat(
        nat_le(succ(v("p")), word_top()),
        lam("_", boolean(), nat_pair()),
        vec![pair(g("Zero"), succ(v("p"))), pair(succ(g("Zero")), g("Zero"))],
    );
    let signed_remainder = apps(
        g("Prod_snd"),
        &[
            int(),
            int(),
            mat(
                divmod(v("j")),
                lam("_", nat_pair(), int_pair()),
                vec![lam(
                    "q",
                    nat(),
                    lam("r", nat(), apps(g("Prod_mk"), &[int(), int(), of_nat(v("q")), of_nat(v("r"))])),
                )],
            ),
        ],
    );
    let within_word = apps(
        g("Nat_le_trans"),
        &[v("j"), signed_top(), word_top(), v("small"), apps(g("refl"), &[boolean(), g("true")])],
    );
    let compared = |n: Term, low: Term, high: Term| {
        mat(nat_le(n, signed_top()), lam("_", boolean(), int()), vec![low, high])
    };
    let two = || succ(succ(g("Zero")));
    let doubled_is_power = pi("n", nat(), eq(int(), doubled(v("n")), of_nat(power_of_two(v("n")))));

    let mut script = vec![
        T::Have("doubled_is_power".to_string(), doubled_is_power),
        T::Intro(names(&["n"])),
        T::Induction("n".to_string(), vec![Vec::new(), names(&["j", "ih"])]),
        T::Rfl,
        T::Change(eq(
            int(),
            mul(doubled(v("j")), of_nat(two())),
            of_nat(apps(g("Nat_mul"), &[power_of_two(v("j")), two()])),
        )),
        T::Rewrite(v("ih"), false),
        T::Rfl,
        T::Have("pred_le".to_string(), pred_le),
        T::Intro(names(&["p"])),
        T::Induction("p".to_string(), vec![names(&["k0", "fits0"]), names(&["p1", "ih", "k", "fits"])]),
        T::Rfl,
        T::Cases("k".to_string(), vec![Vec::new(), names(&["k1"])]),
    ];
    script.extend(refute("fits"));
    script.extend([
        T::Exact(apps(v("ih"), &[v("k1"), v("fits")])),
        T::Have("divmod_small".to_string(), divmod_small),
        T::Intro(names(&["j"])),
        T::Induction("j".to_string(), vec![names(&["fits0"]), names(&["p", "ih", "fits"])]),
        T::Rfl,
        T::Change(eq(nat_pair(), step, pair(g("Zero"), succ(v("p"))))),
        T::Rewrite(app(v("ih"), apps(v("pred_le"), &[v("p"), word_top(), v("fits")])), false),
        T::Change(eq(nat_pair(), stepped, pair(g("Zero"), succ(v("p"))))),
        T::Rewrite(v("fits"), false),
        T::Rfl,
        T::Have("mod_small".to_string(), mod_small),
        T::Intro(names(&["j", "small"])),
        T::Change(eq(int(), signed_remainder, of_nat(v("j")))),
        T::Rewrite(apps(v("divmod_small"), &[v("j"), within_word]), false),
        T::Rfl,
        T::Have("in_range".to_string(), in_range),
        T::Intro(names(&["y", "above", "below"])),
        T::Cases("y".to_string(), vec![names(&["m"]), names(&["k"])]),
        T::Have("small".to_string(), signed(v("m"))),
        T::Exact(v("below")),
        T::Change(eq(int(), wrapped_from_zero(v("m")), of_nat(v("m")))),
        T::Rewrite(apps(v("mod_small"), &[v("m"), v("small")]), false),
        T::Change(eq(
            int(),
            compared(v("m"), of_nat(v("m")), less_a_word(v("m"))),
            of_nat(v("m")),
        )),
        T::Rewrite(v("small"), false),
        T::Rfl,
        T::Have("small".to_string(), signed(v("k"))),
        T::Exact(v("above")),
        T::Change(eq(int(), wrapped_below_zero(v("k")), neg_succ(v("k")))),
        T::Rewrite(apps(v("mod_small"), &[v("k"), v("small")]), false),
        T::Change(eq(
            int(),
            compared(v("k"), neg_succ(v("k")), word_complement(v("k"))),
            neg_succ(v("k")),
        )),
        T::Rewrite(v("small"), false),
        T::Rfl,
        T::Intro(names(&["x", "n", "within", "above", "below"])),
        T::Change(eq(int(), wrapped(mul(v("x"), doubled(v("n")))), product())),
        T::Rewrite(app(v("doubled_is_power"), v("n")), false),
        T::Exact(apps(v("in_range"), &[product(), v("above"), v("below")])),
    ]);
    proved_by(statement, semantics(&[SHL]), &script)
}

/// `mod-pow2-and`: `∀ (x : Int) (n : Nat), 0 ≤ x → n ≤ 62 → x % 2^n = x & (2^n - 1)`, with `%` the runtime's
/// truncating remainder and `2^n - 1` the integer the rule builds.
///
/// For `x = ofNat m` both sides are naturals: `x % 2^n` is the remainder `m mod 2^n`, the second component of
/// `Nat_divModSucc m (2^n - 1)`, and `x & (2^n - 1)` is the bitwise loop on `m` and the mask `2^n - 1`, which is
/// `mask n` for `mask 0 = 0`, `mask (j + 1) = 2 · mask j + 1`. The loop with the mask `mask j`, from any accumulator
/// `acc` and weight `w`, is `acc + w · (m mod 2^j)`, by induction on `j`: the mask's low bit is 1 and its half is
/// `mask j`, so a step adds `w` to `acc` exactly when `m` is odd and continues on `m / 2` with the weight doubled,
/// and `m mod 2^(j+1) = 2 · ((m / 2) mod 2^j) + m mod 2`. That last identity is by induction on `m`: the remainder
/// by `2 · (q + 1)` counts up as the half and the low bit do, wrapping to 0 exactly when the remainder by `q + 1`
/// does. The negative case has a guard computing to `Eq Bool false true`. The loop allows 64 halvings, and the mask
/// runs out after `n ≤ 62`. Without the guard on `x` the identity is false: `-1 % 2` is -1 and `-1 & 1` is 1.
pub fn mod_pow2_and() -> Result<KernelCertificate, String> {
    use TacticStep as T;
    let power = || of_nat(power_of_two(v("n")));
    let statement = pi(
        "x",
        int(),
        pi(
            "n",
            nat(),
            pi(
                "h",
                is_true(int_le(of_nat(g("Zero")), v("x"))),
                pi(
                    "within",
                    is_true(nat_le(v("n"), nat_literal(BigInt::from_i64(62)))),
                    eq(int(), tmod(v("x"), power()), bitand(v("x"), int_sub(power(), of_nat(one())))),
                ),
            ),
        ),
    );

    let and = || BitOp::And.combiner();
    let looped = |fuel: Term, m: Term, n: Term, acc: Term, w: Term| apps(bitwise_loop(and()), &[fuel, m, n, acc, w]);
    let pair = nat_pair;
    let odd_mask = |q: Term| succ(twice(q));
    let halves = |pair: Term| (quotient_of(pair.clone()), low_bit(remainder_of(pair)));

    let mut script = vec![T::Have("divmod_by_one".to_string(), divmod_by_one_statement())];
    script.extend(divmod_by_one_script());

    // A remainder by `k + 1` is at most `k`.
    script.extend([
        T::Have(
            "remainder_bound".to_string(),
            pi("x", nat(), pi("k", nat(), is_true(nat_le(nat_rem(v("x"), v("k")), v("k"))))),
        ),
        T::Intro(names(&["x"])),
        T::Induction("x".to_string(), vec![names(&["k0"]), names(&["x1", "ih", "k"])]),
        T::Rfl,
        T::Change(is_true(nat_le(pair_snd(divmod_step(nat_divmod(v("x1"), v("k")), v("k"))), v("k")))),
        T::Have(
            "bound_step".to_string(),
            pi(
                "p",
                pair(),
                arrow(
                    is_true(nat_le(pair_snd(v("p")), v("k"))),
                    is_true(nat_le(pair_snd(divmod_step(v("p"), v("k"))), v("k"))),
                ),
            ),
        ),
        T::Intro(names(&["p", "hp"])),
        T::Cases("p".to_string(), vec![names(&["bq", "br"])]),
    ]);
    let stepped = |b: Term| {
        mat(
            b,
            lam("_", boolean(), pair()),
            vec![nat_pair_of(v("bq"), succ(v("br"))), nat_pair_of(succ(v("bq")), g("Zero"))],
        )
    };
    script.extend(by_bool(
        "bound_case",
        apps(g("natLt"), &[v("br"), v("k")]),
        |b| is_true(nat_le(pair_snd(stepped(b)), v("k"))),
        vec![T::Exact(v("known"))],
        vec![T::Rfl],
    ));
    script.push(T::Exact(apps(v("bound_step"), &[nat_divmod(v("x1"), v("k")), app(v("ih"), v("k"))])));

    // The remainder of `x + 1`: one more than the remainder of `x`, or 0 when that was `k`.
    let counted = |r: Term, k: Term| {
        mat(apps(g("natLt"), &[r.clone(), k]), lam("_", boolean(), nat()), vec![succ(r), g("Zero")])
    };
    script.extend([
        T::Have(
            "remainder_step".to_string(),
            pi(
                "x",
                nat(),
                pi(
                    "k",
                    nat(),
                    eq(nat(), nat_rem(succ(v("x")), v("k")), counted(nat_rem(v("x"), v("k")), v("k"))),
                ),
            ),
        ),
        T::Intro(names(&["x", "k"])),
        T::Change(eq(
            nat(),
            pair_snd(divmod_step(nat_divmod(v("x"), v("k")), v("k"))),
            counted(nat_rem(v("x"), v("k")), v("k")),
        )),
        T::Have(
            "step_snd".to_string(),
            pi(
                "p",
                pair(),
                eq(nat(), pair_snd(divmod_step(v("p"), v("k"))), counted(pair_snd(v("p")), v("k"))),
            ),
        ),
        T::Intro(names(&["p"])),
        T::Cases("p".to_string(), vec![names(&["sq", "sr"])]),
    ]);
    let stepped_snd = |b: Term| {
        let next = mat(
            b.clone(),
            lam("_", boolean(), pair()),
            vec![nat_pair_of(v("sq"), succ(v("sr"))), nat_pair_of(succ(v("sq")), g("Zero"))],
        );
        eq(nat(), pair_snd(next), mat(b, lam("_", boolean(), nat()), vec![succ(v("sr")), g("Zero")]))
    };
    script.extend(by_bool(
        "step_case",
        apps(g("natLt"), &[v("sr"), v("k")]),
        stepped_snd,
        vec![T::Rfl],
        vec![T::Rfl],
    ));
    script.push(T::Exact(app(v("step_snd"), nat_divmod(v("x"), v("k")))));

    // Halving `2q` and `2q + 1`.
    script.extend([
        T::Have(
            "half_even".to_string(),
            pi("q", nat(), eq(pair(), nat_divmod(twice(v("q")), one()), nat_pair_of(v("q"), g("Zero")))),
        ),
        T::Intro(names(&["q"])),
        T::Induction("q".to_string(), vec![Vec::new(), names(&["q1", "ih"])]),
        T::Rfl,
        T::Change(eq(
            pair(),
            nat_divmod(succ(nat_add(succ(v("q1")), v("q1"))), one()),
            nat_pair_of(succ(v("q1")), g("Zero")),
        )),
        T::Rewrite(apps(g("Nat_add_succ_left"), &[v("q1"), v("q1")]), false),
        T::Change(eq(
            pair(),
            divmod_step(divmod_step(nat_divmod(twice(v("q1")), one()), one()), one()),
            nat_pair_of(succ(v("q1")), g("Zero")),
        )),
        T::Rewrite(v("ih"), false),
        T::Rfl,
        T::Have(
            "half_odd".to_string(),
            pi("q", nat(), eq(pair(), nat_divmod(odd_mask(v("q")), one()), nat_pair_of(v("q"), one()))),
        ),
        T::Intro(names(&["q"])),
        T::Change(eq(pair(), divmod_step(nat_divmod(twice(v("q")), one()), one()), nat_pair_of(v("q"), one()))),
        T::Rewrite(app(v("half_even"), v("q")), false),
        T::Rfl,
    ]);

    // Doubling preserves the order: `2x ≤ 2q ⟺ x ≤ q` and `2x + 1 ≤ 2q ⟺ x + 1 ≤ q`.
    let le = |a: Term, b: Term| nat_le(a, b);
    let succ_left = |a: &str| apps(g("Nat_add_succ_left"), &[v(a), v(a)]);
    script.extend([
        T::Have(
            "twice_le".to_string(),
            pi("x", nat(), pi("q", nat(), eq(boolean(), le(twice(v("x")), twice(v("q"))), le(v("x"), v("q"))))),
        ),
        T::Intro(names(&["x"])),
        T::Induction("x".to_string(), vec![names(&["q0"]), names(&["x1", "ih", "q"])]),
        T::Rfl,
        T::Change(eq(
            boolean(),
            le(succ(nat_add(succ(v("x1")), v("x1"))), twice(v("q"))),
            le(succ(v("x1")), v("q")),
        )),
        T::Rewrite(succ_left("x1"), false),
        T::Cases("q".to_string(), vec![Vec::new(), names(&["q1"])]),
        T::Rfl,
        T::Change(eq(
            boolean(),
            le(succ(succ(twice(v("x1")))), succ(nat_add(succ(v("q1")), v("q1")))),
            le(succ(v("x1")), succ(v("q1"))),
        )),
        T::Rewrite(succ_left("q1"), false),
        T::Change(eq(boolean(), le(twice(v("x1")), twice(v("q1"))), le(v("x1"), v("q1")))),
        T::Exact(app(v("ih"), v("q1"))),
        T::Have(
            "twice_lt".to_string(),
            pi(
                "x",
                nat(),
                pi("q", nat(), eq(boolean(), le(succ(twice(v("x"))), twice(v("q"))), le(succ(v("x")), v("q")))),
            ),
        ),
        T::Intro(names(&["x"])),
        T::Induction("x".to_string(), vec![names(&["q0"]), names(&["x1", "ih", "q"])]),
        T::Cases("q0".to_string(), vec![Vec::new(), names(&["q1"])]),
        T::Rfl,
        T::Rfl,
        T::Change(eq(
            boolean(),
            le(succ(succ(nat_add(succ(v("x1")), v("x1")))), twice(v("q"))),
            le(succ(succ(v("x1"))), v("q")),
        )),
        T::Rewrite(succ_left("x1"), false),
        T::Cases("q".to_string(), vec![Vec::new(), names(&["q1"])]),
        T::Rfl,
        T::Change(eq(
            boolean(),
            le(succ(succ(succ(twice(v("x1"))))), succ(nat_add(succ(v("q1")), v("q1")))),
            le(succ(succ(v("x1"))), succ(v("q1"))),
        )),
        T::Rewrite(succ_left("q1"), false),
        T::Change(eq(boolean(), le(succ(twice(v("x1"))), twice(v("q1"))), le(succ(v("x1")), v("q1")))),
        T::Exact(app(v("ih"), v("q1"))),
    ]);

    // `x mod 2(q + 1) = 2 · ((x / 2) mod (q + 1)) + x mod 2`, by induction on `x`.
    let wide = || odd_mask(v("q"));
    script.extend([
        T::Have(
            "double_remainder".to_string(),
            pi(
                "q",
                nat(),
                pi(
                    "x",
                    nat(),
                    eq(nat(), nat_rem(v("x"), wide()), remainder_by_double(nat_divmod(v("x"), one()), v("q"))),
                ),
            ),
        ),
        T::Intro(names(&["q", "x"])),
        T::Induction("x".to_string(), vec![Vec::new(), names(&["x1", "ih"])]),
        T::Rfl,
        T::Change(eq(
            nat(),
            pair_snd(divmod_step(nat_divmod(v("x1"), wide()), wide())),
            remainder_by_double(divmod_step(nat_divmod(v("x1"), one()), one()), v("q")),
        )),
        T::Have(
            "double_step".to_string(),
            pi(
                "p1",
                pair(),
                pi(
                    "p2",
                    pair(),
                    arrow(
                        is_true(nat_le(pair_snd(v("p1")), one())),
                        arrow(
                            eq(nat(), pair_snd(v("p2")), remainder_by_double(v("p1"), v("q"))),
                            eq(
                                nat(),
                                pair_snd(divmod_step(v("p2"), wide())),
                                remainder_by_double(divmod_step(v("p1"), one()), v("q")),
                            ),
                        ),
                    ),
                ),
            ),
        ),
        T::Intro(names(&["p1", "p2", "low_bound", "matched"])),
        T::Cases("p1".to_string(), vec![names(&["hq", "hr"])]),
        T::Cases("p2".to_string(), vec![names(&["wq", "wr"])]),
        T::Cases("hr".to_string(), vec![Vec::new(), names(&["hr1"])]),
    ]);
    let rest = || nat_rem(v("hq"), v("q"));
    script.extend([
        T::Have("even".to_string(), eq(nat(), v("wr"), twice(rest()))),
        T::Exact(v("matched")),
        T::Change(eq(
            nat(),
            pair_snd(divmod_step(nat_pair_of(v("wq"), v("wr")), wide())),
            succ(twice(rest())),
        )),
        T::Rewrite(v("even"), false),
        T::Have("below".to_string(), is_true(apps(g("natLt"), &[twice(rest()), wide()]))),
        T::Change(eq(boolean(), le(twice(rest()), twice(v("q"))), g("true"))),
        T::Rewrite(apps(v("twice_le"), &[rest(), v("q")]), false),
        T::Exact(apps(v("remainder_bound"), &[v("hq"), v("q")])),
        T::Change(eq(
            nat(),
            pair_snd(mat(
                apps(g("natLt"), &[twice(rest()), wide()]),
                lam("_", boolean(), pair()),
                vec![
                    nat_pair_of(v("wq"), succ(twice(rest()))),
                    nat_pair_of(succ(v("wq")), g("Zero")),
                ],
            )),
            succ(twice(rest())),
        )),
        T::Rewrite(v("below"), false),
        T::Rfl,
    ]);
    script.extend([T::Cases("hr1".to_string(), vec![Vec::new(), names(&["hr2"])])]);
    let odd_branch = |b: Term| {
        mat(
            b,
            lam("_", boolean(), pair()),
            vec![nat_pair_of(v("wq"), succ(succ(twice(rest())))), nat_pair_of(succ(v("wq")), g("Zero"))],
        )
    };
    script.extend([
        T::Have("odd".to_string(), eq(nat(), v("wr"), succ(twice(rest())))),
        T::Exact(v("matched")),
        T::Change(eq(
            nat(),
            pair_snd(divmod_step(nat_pair_of(v("wq"), v("wr")), wide())),
            twice(nat_rem(succ(v("hq")), v("q"))),
        )),
        T::Rewrite(v("odd"), false),
        T::Rewrite(apps(v("remainder_step"), &[v("hq"), v("q")]), false),
        T::Have(
            "carry".to_string(),
            eq(
                boolean(),
                apps(g("natLt"), &[succ(twice(rest())), wide()]),
                apps(g("natLt"), &[rest(), v("q")]),
            ),
        ),
        T::Change(eq(boolean(), le(succ(twice(rest())), twice(v("q"))), le(succ(rest()), v("q")))),
        T::Exact(apps(v("twice_lt"), &[rest(), v("q")])),
        T::Change(eq(
            nat(),
            pair_snd(odd_branch(apps(g("natLt"), &[succ(twice(rest())), wide()]))),
            twice(counted(rest(), v("q"))),
        )),
        T::Rewrite(v("carry"), false),
    ]);
    let carried = move |b: Term| {
        let counted_b = mat(b.clone(), lam("_", boolean(), nat()), vec![succ(nat_rem(v("hq"), v("q"))), g("Zero")]);
        eq(
            nat(),
            pair_snd(mat(
                b,
                lam("_", boolean(), pair()),
                vec![
                    nat_pair_of(v("wq"), succ(succ(twice(nat_rem(v("hq"), v("q")))))),
                    nat_pair_of(succ(v("wq")), g("Zero")),
                ],
            )),
            twice(counted_b),
        )
    };
    script.extend(by_bool(
        "carry_case",
        apps(g("natLt"), &[rest(), v("q")]),
        carried,
        vec![
            T::Change(eq(
                nat(),
                succ(succ(twice(rest()))),
                succ(nat_add(succ(rest()), rest())),
            )),
            T::Rewrite(apps(g("Nat_add_succ_left"), &[rest(), rest()]), false),
            T::Rfl,
        ],
        vec![T::Rfl],
    ));
    script.extend(refute("low_bound"));
    script.push(T::Exact(apps(
        v("double_step"),
        &[
            nat_divmod(v("x1"), one()),
            nat_divmod(v("x1"), wide()),
            apps(v("remainder_bound"), &[v("x1"), one()]),
            v("ih"),
        ],
    )));

    // Weighted sums: `(w + w) · x = w · (x + x)` and `(acc + w) + (w + w) · x = acc + w · (2x + 1)`.
    let mul_twice = |w: Term, x: Term| eq(nat(), nat_mul(twice(w.clone()), x.clone()), nat_mul(w, twice(x)));
    script.extend([
        T::Have("mul_twice".to_string(), pi("w", nat(), pi("x", nat(), mul_twice(v("w"), v("x"))))),
        T::Intro(names(&["w", "x"])),
        T::Rewrite(apps(g("Nat_mul_comm"), &[twice(v("w")), v("x")]), false),
        T::Rewrite(apps(g("Nat_mul_add"), &[v("x"), v("w"), v("w")]), false),
        T::Rewrite(apps(g("Nat_mul_add"), &[v("w"), v("x"), v("x")]), false),
        T::Rewrite(apps(g("Nat_mul_comm"), &[v("x"), v("w")]), false),
        T::Rfl,
        T::Have(
            "add_mul_twice".to_string(),
            pi(
                "acc",
                nat(),
                pi(
                    "w",
                    nat(),
                    pi(
                        "x",
                        nat(),
                        eq(
                            nat(),
                            nat_add(nat_add(v("acc"), v("w")), nat_mul(twice(v("w")), v("x"))),
                            nat_add(v("acc"), nat_mul(v("w"), succ(twice(v("x"))))),
                        ),
                    ),
                ),
            ),
        ),
        T::Intro(names(&["acc", "w", "x"])),
        T::Change(eq(
            nat(),
            nat_add(nat_add(v("acc"), v("w")), nat_mul(twice(v("w")), v("x"))),
            nat_add(v("acc"), nat_add(nat_mul(v("w"), twice(v("x"))), v("w"))),
        )),
        T::Rewrite(apps(v("mul_twice"), &[v("w"), v("x")]), false),
        T::Rewrite(apps(g("Nat_add_assoc"), &[v("acc"), v("w"), nat_mul(v("w"), twice(v("x")))]), false),
        T::Rewrite(apps(g("Nat_add_comm"), &[v("w"), nat_mul(v("w"), twice(v("x")))]), false),
        T::Rfl,
    ]);

    // The loop with the mask `mask j`: `acc + w · (m mod 2^j)`.
    let masked = |j: Term| {
        pi(
            "k",
            nat(),
            arrow(
                is_true(nat_le(j.clone(), v("k"))),
                pi(
                    "m",
                    nat(),
                    pi(
                        "acc",
                        nat(),
                        pi(
                            "w",
                            nat(),
                            eq(
                                nat(),
                                looped(succ(v("k")), v("m"), mask(j.clone()), v("acc"), v("w")),
                                nat_add(v("acc"), nat_mul(v("w"), nat_rem(v("m"), mask(j)))),
                            ),
                        ),
                    ),
                ),
            ),
        )
    };
    let q = || mask(v("j1"));
    let stepped_loop = |p: Term| {
        bitwise_step(&and(), bitwise_loop(and()), succ(v("k1")), halves(p), halves(nat_divmod(odd_mask(q()), one())))
    };
    let stepped_pair = |p: Term| {
        bitwise_step(&and(), bitwise_loop(and()), succ(v("k1")), halves(p), halves(nat_pair_of(q(), one())))
    };
    script.extend([
        T::Have("loop_mask".to_string(), pi("j", nat(), masked(v("j")))),
        T::Intro(names(&["j"])),
        T::Induction(
            "j".to_string(),
            vec![names(&["k0", "hk0", "m0", "acc0", "w0"]), names(&["j1", "ih", "k", "hk", "m", "acc", "w"])],
        ),
        T::Cases("m0".to_string(), vec![Vec::new(), names(&["m1"])]),
        T::Rfl,
        T::Change(eq(
            nat(),
            looped(succ(v("k0")), succ(v("m1")), g("Zero"), v("acc0"), v("w0")),
            nat_add(v("acc0"), nat_mul(v("w0"), nat_rem(succ(v("m1")), g("Zero")))),
        )),
        T::Rewrite(app(v("divmod_by_one"), succ(v("m1"))), false),
        T::Rfl,
        T::Cases("k".to_string(), vec![Vec::new(), names(&["k1"])]),
    ]);
    script.extend(refute("hk"));
    script.extend([
        T::Change(eq(
            nat(),
            looped(succ(succ(v("k1"))), v("m"), odd_mask(q()), v("acc"), v("w")),
            nat_add(v("acc"), nat_mul(v("w"), nat_rem(v("m"), odd_mask(q())))),
        )),
        T::Cases("m".to_string(), vec![Vec::new(), names(&["m1"])]),
        T::Rfl,
        T::Rewrite(apps(v("double_remainder"), &[q(), succ(v("m1"))]), false),
        T::Change(eq(
            nat(),
            stepped_loop(nat_divmod(succ(v("m1")), one())),
            nat_add(v("acc"), nat_mul(v("w"), remainder_by_double(nat_divmod(succ(v("m1")), one()), q()))),
        )),
        T::Rewrite(app(v("half_odd"), q()), false),
        T::Have(
            "pair_step".to_string(),
            pi(
                "p",
                pair(),
                arrow(
                    is_true(nat_le(pair_snd(v("p")), one())),
                    eq(
                        nat(),
                        stepped_pair(v("p")),
                        nat_add(v("acc"), nat_mul(v("w"), remainder_by_double(v("p"), q()))),
                    ),
                ),
            ),
        ),
        T::Intro(names(&["p", "hp"])),
        T::Cases("p".to_string(), vec![names(&["half", "low"])]),
        T::Cases("low".to_string(), vec![Vec::new(), names(&["low1"])]),
        T::Change(eq(
            nat(),
            looped(succ(v("k1")), v("half"), q(), v("acc"), twice(v("w"))),
            nat_add(v("acc"), nat_mul(v("w"), twice(nat_rem(v("half"), q())))),
        )),
        T::Rewrite(apps(v("ih"), &[v("k1"), v("hk"), v("half"), v("acc"), twice(v("w"))]), false),
        T::Rewrite(apps(v("mul_twice"), &[v("w"), nat_rem(v("half"), q())]), false),
        T::Rfl,
        T::Cases("low1".to_string(), vec![Vec::new(), names(&["low2"])]),
        T::Change(eq(
            nat(),
            looped(succ(v("k1")), v("half"), q(), nat_add(v("acc"), v("w")), twice(v("w"))),
            nat_add(v("acc"), nat_mul(v("w"), succ(twice(nat_rem(v("half"), q()))))),
        )),
        T::Rewrite(apps(v("ih"), &[v("k1"), v("hk"), v("half"), nat_add(v("acc"), v("w")), twice(v("w"))]), false),
        T::Exact(apps(v("add_mul_twice"), &[v("acc"), v("w"), nat_rem(v("half"), q())])),
    ]);
    script.extend(refute("hp"));
    script.push(T::Exact(apps(
        v("pair_step"),
        &[nat_divmod(succ(v("m1")), one()), apps(v("remainder_bound"), &[succ(v("m1")), one()])],
    )));

    // `2^n = mask n + 1`.
    script.extend([
        T::Have("power_mask".to_string(), pi("n", nat(), eq(nat(), power_of_two(v("n")), succ(mask(v("n")))))),
        T::Intro(names(&["n"])),
        T::Induction("n".to_string(), vec![Vec::new(), names(&["i", "ih"])]),
        T::Rfl,
        T::Change(eq(
            nat(),
            nat_add(nat_add(g("Zero"), power_of_two(v("i"))), power_of_two(v("i"))),
            succ(odd_mask(mask(v("i")))),
        )),
        T::Rewrite(v("ih"), false),
        T::Change(eq(
            nat(),
            succ(nat_add(succ(nat_add(g("Zero"), mask(v("i")))), mask(v("i")))),
            succ(odd_mask(mask(v("i")))),
        )),
        T::Rewrite(app(g("Nat_zero_add"), mask(v("i"))), false),
        T::Rewrite(apps(g("Nat_add_succ_left"), &[mask(v("i")), mask(v("i"))]), false),
        T::Rfl,
    ]);

    // The remainder `mod` reads out of `Nat_divModSucc`.
    script.extend([
        T::Have(
            "remainder_read".to_string(),
            pi("p", pair(), eq(int(), remainder_of(v("p")), of_nat(pair_snd(v("p"))))),
        ),
        T::Intro(names(&["p"])),
        T::Cases("p".to_string(), vec![names(&["rq", "rr"])]),
        T::Rfl,
    ]);

    // The theorem.
    let mn = || mask(v("n"));
    let unfolded_mask = mat(
        nat_sub(g("Zero"), mn()),
        lam("_", nat(), int()),
        vec![of_nat(nat_sub(mn(), g("Zero"))), lam("k", nat(), neg_succ(v("k")))],
    );
    let word = || nat_literal(BigInt::from_u64(WORD_HALVINGS - 1));
    script.extend([
        T::Intro(names(&["x", "n", "h", "within"])),
        T::Cases("x".to_string(), vec![names(&["m"]), names(&["k"])]),
        T::Rewrite(app(v("power_mask"), v("n")), false),
        T::Have("mask_int".to_string(), eq(int(), int_sub(of_nat(succ(mn())), of_nat(one())), of_nat(mn()))),
        T::Change(eq(int(), apps(g("Int_subNatNat"), &[succ(mn()), one()]), of_nat(mn()))),
        T::Rewrite(apps(g("IntLaw_subNatNat_succ_succ"), &[mn(), g("Zero")]), false),
        T::Change(eq(int(), unfolded_mask, of_nat(mn()))),
        T::Rewrite(app(g("Nat_sub_zero_left"), mn()), false),
        T::Rfl,
        T::Rewrite(v("mask_int"), false),
        T::Change(eq(
            int(),
            remainder_of(nat_divmod(v("m"), mn())),
            of_nat(looped(succ(word()), v("m"), mn(), g("Zero"), one())),
        )),
        T::Rewrite(app(v("remainder_read"), nat_divmod(v("m"), mn())), false),
        T::Have("fits".to_string(), is_true(nat_le(v("n"), word()))),
        T::Exact(apps(
            g("Nat_le_trans"),
            &[v("n"), nat_literal(BigInt::from_i64(62)), word(), v("within"), refl(boolean(), g("true"))],
        )),
        T::Rewrite(apps(v("loop_mask"), &[v("n"), word(), v("fits"), v("m"), g("Zero"), one()]), false),
        T::Rewrite(app(g("Nat_zero_add"), nat_mul(one(), nat_rem(v("m"), mn()))), false),
        T::Rewrite(apps(g("Nat_mul_comm"), &[one(), nat_rem(v("m"), mn())]), false),
        T::Rewrite(app(g("Nat_mul_one"), nat_rem(v("m"), mn())), false),
        T::Rfl,
    ]);
    script.extend(refute("h"));
    proved_by(statement, semantics(&[TMOD, BITWISE, AND]), &script)
}

