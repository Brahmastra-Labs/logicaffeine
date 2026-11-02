/-!
@case      T10-007
@tier      10 · Literals and primitive types
@title     `Float` — the type and the literal exist in LOGOS, and nothing else does
@lean      Float, float literal, Float.add / Float.mul, OfScientific, Float.ofNat
@expect    degrade
@target    Term::Lit(Literal::Float(f64)) of type `Global("Float")` — and NO operation
@evidence  kernel/src/term.rs:17-18 (`Literal::Float(f64)`);
           kernel/src/type_checker.rs:303 (`Literal::Float(_) => Ok(Global("Float"))`);
           kernel/src/prelude.rs:355 (`ctx.add_inductive("Float", Sort(Type 0))` — a zero-
           constructor opaque carrier); kernel/src/prelude.rs:363-381 (`register_primitives`
           declares `add`/`sub`/`mul`/`div`/`mod` at type `Int → Int → Int` and
           `le`/`lt`/`ge`/`gt` at `Int → Int → Bool` — every one of them is an `Int`
           operation, NONE is `Float`); kernel/src/prelude.rs:92-340 (the 16 axioms are all
           about `Int`; there is no `Float` axiom); kernel/src/reduction.rs:444-506
           (`try_primitive_reduce` matches `Literal::Int`/`Literal::BigInt` operands only via
           `lit_bigint`, term.rs:59-63, which returns `None` for `Literal::Float` — so no
           `Float` expression ever reduces)
@loss      FLOAT_IS_INERT. The type exists and literals inject into it, so a `Float`-typed
           signature survives translation and `Float` values can be passed around and compared
           for literal identity. Everything else is gone: no arithmetic, no comparison, no
           axiom, no reduction rule. `Float` in LOGOS is strictly less than an axiomatised
           type — an axiomatised type at least has laws. It is a labelled atom.
@loss      NAN_AND_SIGNED_ZERO. `Literal::Float` holds an `f64` and definitional equality of
           literals is structural `==` on the `Literal` (type_checker.rs:1485 and the
           surrounding literal arms). Rust's `PartialEq` on `f64` is IEEE-754, under which
           `NaN != NaN` and `0.0 == -0.0`; the derived `Eq` on `Literal` that the kernel relies
           on therefore does not agree with Lean's `Float` equality either, and the
           discrepancy is silent. A translator must record this rather than assume `Float`
           literals behave like `Int` literals.
@guard     Catches the translator that reuses the `Int` primitives for `Float` because their
           names look generic. `add : Int → Int → Int` (prelude.rs:377) applied to a `Float`
           is ILL-TYPED and will be caught — but only if the translator does not ALSO emit a
           coercion or leave a `Term::Hole` in the argument position, because `types_equal`
           treats `Hole` as matching anything (type_checker.rs:1594-1597) and that leniency
           leaks into `is_subtype`'s fast path (:1314). With one `Hole`, `add (1.5 : Float) 2`
           typechecks. See T11-004.
@guard     Catches declaring the `Float` operations as fresh axioms to "fill the gap". That is
           worse than rejecting: the emitted `Float_add` has no reduction rule, so nothing about
           it computes, AND it has no laws, so nothing about it is provable — yet the
           translation reports success and the resulting library contains theorems whose proofs
           are stuck applications. It also enlarges the trusted base, which
           `certificate::recheck` rebuilds from `StandardLibrary::register` alone
           (certificate.rs:63-75), so the certificate will fail with an unbound-variable error
           that reads like a name-resolution bug. Compare T06-009.
@guard     Catches silently rounding the literal. Lean's float literals go through
           `OfScientific`, so `0.1` is a decimal mantissa/exponent pair that Lean resolves to a
           specific `f64`. The translator must emit the SAME `f64` bit pattern, not re-parse the
           decimal with different rounding — because literal equality is bitwise-ish `==` and a
           one-ulp difference makes two occurrences of `0.1` unequal.
@guard     Catches assuming Lean can compute with floats in proofs either. Lean's `Float`
           operations are `@[extern]` opaque constants, so `rfl` does not prove
           `1.5 + 2.5 = 4.0` in Lean any more than in LOGOS. The degrade record must say what
           is *additionally* lost — the operations themselves, and `native_decide`'s ability to
           evaluate them.
@oracle    Emitted term for a float literal is `Lit(Float(f))` with `f` bit-identical to Lean's
           elaborated `f64`; `infer_type` returns `Global("Float")`; the loss record contains
           FLOAT_IS_INERT; and the translator performs ZERO `add_declaration` calls introducing
           a `Float` operation.
-/

namespace T10_007

/-- A float literal. Elaborated through `OfScientific`, so the `Expr` is not a bare
    `Expr.lit` — the translator must see through the instance to the `f64`. -/
def half : Float := 0.5

/-- A decimal whose nearest `f64` is not the decimal: the rounding must match Lean's. -/
def tenth : Float := 0.1

/-- Negative and exponent forms. -/
def negThousandth : Float := -1.0e-3

/-- Float arithmetic. These are `@[extern]` opaque constants in Lean, so they do not reduce in
    Lean's kernel either; in LOGOS the operations do not exist at all. -/
def sum : Float := 1.5 + 2.5

/-- Comparison. LOGOS's `le`/`lt`/`ge`/`gt` are `Int → Int → Bool` (prelude.rs:373-381); there
    is no `Float` ordering. -/
def isSmall (x : Float) : Bool := x < 1.0

/-- The IEEE anomalies that make structural literal equality the wrong equality. -/
example : (0.0 : Float) == (-0.0 : Float) := by native_decide
example : ((0.0 : Float) / 0.0 == (0.0 : Float) / 0.0) = false := by native_decide

/-- A `Float`-typed signature with no arithmetic in it — this much survives translation. -/
def identity (x : Float) : Float := x

end T10_007
