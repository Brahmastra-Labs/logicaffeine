/-!
@case      T10-005
@tier      10 · Literals and primitive types
@title     `String` literal — `Literal::Text` carries the bytes and nothing else
@lean      String literal, Expr.lit (Literal.strVal), String.mk, List Char
@expect    degrade
@target    Term::Lit(Literal::Text(s)) of type `Global("Text")`
@evidence  kernel/src/term.rs:19-20 (`Literal::Text(String)` — "UTF-8 string");
           kernel/src/type_checker.rs:304 (`Literal::Text(_) => Ok(Global("Text"))`);
           kernel/src/prelude.rs:356 (`ctx.add_inductive("Text", Sort(Type 0))` — an inductive
           with ZERO constructors, so `Text` is opaque and cannot be matched on);
           kernel/src/prelude.rs:352-381 (`register_primitives` declares arithmetic and
           comparison builtins for `Int` and temporal operations for `Duration`/`Date`/`Moment`
           — and NOTHING for `Text`); a grep of `crates/logicaffeine_kernel/src/prelude.rs` for
           `Text` returns only the registration at :356 and its use as the payload type of the
           reflection constructor `SName` at :2151-2152;
           kernel/src/reduction.rs:444-506 (`try_primitive_reduce` handles
           `add/sub/mul/div/mod/le/lt/ge/gt` only — no string operation reduces)
@loss      STRING_IS_AN_ATOM. Lean's `String` is a STRUCTURE wrapping `List Char`
           (`String.mk : List Char → String`), and every string function in Lean core is
           defined by recursion over that list; `String` literals are definitionally equal to
           their `String.mk` spelling and `decide` can compute over them. LOGOS's `Text` is an
           opaque carrier with a literal injection and no eliminator at all. The translation
           therefore keeps the VALUE of each literal and loses (a) the character-list structure,
           (b) every equation about it, and (c) any ability to compute. Two `Text` literals can
           still be compared by `def_eq` (structural `==` on the `Literal`), so literal equality
           and disequality survive — that is the entire residual semantics.
@loss      NO_CHAR_LAYER. Because `Char` is absent from LOGOS entirely (T10-006), the loss is
           not recoverable by "just declaring `String` as a list of characters": the element
           type does not exist and `Char`'s validity invariant is a `UInt32` predicate, and
           `UInt32` is absent too (T10-009).
@guard     Catches the translator that declares a fresh `String` inductive over an invented
           `Char` and emits the literal as a spelled-out `Cons c1 (Cons c2 …)`. That typechecks
           (`add_inductive`/`add_constructor` are UNCHECKED, context.rs:253/:419), but the
           emitted type is NOT `Text`, so it shares nothing with the prelude, and a literal of
           any realistic length blows past `normalize`'s 10000-step fuel (reduction.rs:52-69)
           during comparison — at which point the kernel returns the PARTIALLY reduced term and
           reports no error, so a disequality can be "proved" by two strings agreeing on their
           first few thousand steps. See T11-005.
@guard     Catches dropping the loss record because "the literal round-trips". It does
           round-trip; what is lost is every THEOREM about it. A Lean file that proves
           `"ab".length = 2` must not be reported as translated.
@guard     Catches confusing `Text` with the reflection layer. The only other use of `Text` in
           the kernel is as the payload of `SName : Text → Syntax` (prelude.rs:2151-2152), the
           name tag of LOGOS's own reified-syntax inductive. A translator that maps Lean
           `String` onto `SName`'s argument position, or that reuses `Syntax`'s machinery
           because it is "the thing that handles strings", has attached the user's data to the
           kernel's metaprogramming representation — where `try_syn_arith_reduce`
           (reduction.rs:1174-1280) will happily reduce it as if it were a syntax node.
@guard     Catches escaping/encoding drift. `Literal::Text` holds a Rust `String`, i.e. valid
           UTF-8; Lean's `String` is also UTF-8, so the value maps across, but the translator
           must emit the DECODED characters, not Lean's source-level escape sequence. `"\n"`
           must become a one-byte `Text`, not a two-character one — and since `def_eq` on
           literals is `==` on the payload, getting this wrong makes `"\n" = "\n"` fail
           whenever the two occurrences were spelled differently in the source.
@oracle    Emitted term is `Lit(Text(s))` with `s` byte-identical to Lean's decoded literal;
           `infer_type` returns `Global("Text")`; and the loss record contains
           STRING_IS_AN_ATOM. A harness asserts that no declaration in the output applies any
           global to a `Text`-typed argument other than equality.
-/

namespace T10_005

/-- A plain literal. In Lean's kernel this is `Expr.lit (Literal.strVal "hello")`. -/
def greeting : String := "hello"

/-- The empty literal. -/
def empty : String := ""

/-- Escapes and non-ASCII: the translator must emit the decoded bytes. -/
def escaped : String := "line\none\ttab"
def unicode : String := "λ ∀ 日本"

/-- Literal equality and disequality — the entire residual semantics in LOGOS. -/
example : "hello" = "hello" := rfl
example : "hello" ≠ "world" := by decide

/-- The structure Lean actually has and LOGOS does not: `String` is a wrapper around
    `List Char`, and the literal is definitionally its `String.mk` spelling. -/
example : "ab" = String.mk ['a', 'b'] := rfl

/-- Computation over the character list. Nothing in LOGOS can do this: `Text` has zero
    constructors, so there is no eliminator, and no `Text` operation is declared at all. -/
example : "ab".data = ['a', 'b'] := rfl

/-- A `Text`-typed variable used opaquely. THIS much translates faithfully: a `Text` parameter
    that is only ever passed along or compared. -/
def echo (s : String) : String := s

example : echo "hi" = "hi" := rfl

end T10_005
