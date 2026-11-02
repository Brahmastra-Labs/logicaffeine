/-!
@case      T00-009
@tier      0 · Kernel term layer
@title     `Expr.mdata` is erased — LOGOS `Term` has no metadata carrier
@lean      Expr.mdata, Lean.mkAnnotation, pp annotations, source positions, doc comments
@expect    degrade
@target    The mdata WRAPPER is dropped and its payload expression is translated in its place:
           `mdata k e` ⟼ `translate e`. A loss record naming SOURCE_METADATA is emitted per
           distinct mdata key encountered.
@evidence  kernel/src/term.rs:443-545 (the complete `Term` enum: `Sort`, `Var`, `Global`,
           `Const`, `Pi`, `Lambda`, `App`, `Match`, `Fix`, `MutualFix`, `Let`, `Lit`, `Hole` —
           there is NO `mdata`, no annotation node, and no side table keyed on terms);
           kernel/src/context.rs (the `Context` maps carry name → type/body only; there is no
           attribute map, no position map, no docstring map — LOGOS_FACTS "no namespaces, no
           `Name` type, no visibility, no attributes, no reducibility hints");
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION` is the only provenance a certificate
           carries — there is nowhere to record a source span)
@loss      SOURCE_METADATA. Everything Lean threads through `Expr.mdata` disappears: source
           positions (so no LOGOS diagnostic can ever point back at Lean source), pretty-printer
           hints (`pp_using_anonymous_constructor`, `pp_nodot`), the `letFun` marker that
           distinguishes `have` from `let` (T00-019 — that one is NOT acceptable to drop
           silently), and inlining/compiler pragmas. Acceptable for the pretty-printer and
           position keys because they have no semantic content in Lean either; the erasure must
           still be RECORDED, because a translator that cannot say which annotations it dropped
           cannot later prove it dropped only the inert ones.
@guard     Catches translating `Expr.mdata` as an APPLICATION of some identity function, which
           is the tempting way to "keep" the annotation. There is no `id` in the LOGOS prelude
           (LOGOS_FACTS: the registered inductives and axioms are enumerated and `id` is not
           among them), so `Global("id")` fails `get_global` (context.rs:449) with
           `UnboundVariable`; and if the translator instead DEFINES an `id` to wrap with, every
           annotated subterm gains a delta-redex that changes term size, changes `normalize`'s
           step count against the 10000-step fuel (reduction.rs:52-69), and blocks the
           syntactic `Term::Lambda`/`Term::Global` matches that `try_structure_eta`
           (type_checker.rs:1391-1417) and the termination guard (termination.rs) rely on.
@guard     Catches recursing into `mdata` as if it were an opaque leaf. `Expr.mdata` WRAPS a
           real subterm; a translator that pattern-matches `Expr` without an `mdata` arm will
           either fall into a catch-all that emits `Term::Hole` (see T00-016, where a `Hole`
           makes `types_equal` match anything) or throw on a term shape that is completely
           routine. mdata can appear at ANY position, including as the head of an application
           and as a binder's domain, so `Expr.consumeMData` must be applied before every
           structural test, not once at the top.
@guard     Catches erasing mdata *uniformly*. The `letFun` annotation is semantically load-
           bearing (it is how `have` is distinguished from `let` in the elaborated `Expr`), and
           dropping it turns an opaque `have` into a zeta-transparent `Let`, which makes LOGOS
           accept proofs Lean rejects. The mdata arm must therefore INSPECT the key and branch,
           not blanket-strip. See T00-019.
@guard     Catches claiming the loss is nil because "the term is unchanged". The term is
           unchanged; the DIAGNOSTIC SURFACE is not. After erasure there is no way to map a
           LOGOS `KernelError` back to a Lean source span, so every downstream failure in this
           corpus is reported against a term the user never wrote.
@oracle    `Expr.consumeMData` applied to the Lean side and the emitted `Term` agree
           structurally; a loss record with category SOURCE_METADATA exists and enumerates the
           distinct mdata keys erased; and no emitted term contains a wrapper introduced solely
           to carry an annotation.
-/

namespace T00_009

/-- Anonymous-constructor notation: Lean annotates the resulting `Expr` with
    `pp_using_anonymous_constructor` so it prints back as `⟨1, 2⟩`. Pure pretty-printer
    metadata — safe to erase, must be recorded. -/
def anonPair : Nat × Nat := ⟨1, 2⟩

/-- The same value written without the notation: the SAME term modulo mdata. A translator that
    erases correctly makes these two indistinguishable; one that wraps them differently does
    not, and then `def_eq` on them can fail. -/
def explicitPair : Nat × Nat := Prod.mk 1 2

example : anonPair = explicitPair := rfl

-- A doc comment. Lean stores it in an environment extension, not in the `Expr`, but it is the
-- same class of information and LOGOS has no carrier for it either.
/-- This docstring has nowhere to go in LOGOS. -/
def documented : Nat := 1

/-- `have` in term mode elaborates to `letFun` under an mdata annotation. This is the mdata key
    that must NOT be blanket-erased — see T00-019. -/
def annotatedHave : Nat :=
  have k : Nat := 3
  k + 1

/-- mdata under a binder and in the HEAD position of an application, so the erasure pass has to
    be structural rather than top-level. -/
def annotatedUnderBinder : Nat → Nat :=
  fun n => (have k : Nat := n; k) + 1

def annotatedHead : Nat :=
  (have f : Nat → Nat := fun k => k + 1; f) 3

/-- A type ascription, another position the elaborator may annotate. -/
def ascribed : Nat := (3 : Nat)

end T00_009
