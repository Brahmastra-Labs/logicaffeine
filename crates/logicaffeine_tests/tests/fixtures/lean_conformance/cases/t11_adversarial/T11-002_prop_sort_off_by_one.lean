/-!
@case      T11-002
@tier      11 · Adversarial and soundness traps
@title     The sort of `Prop` itself — Lean says `Prop : Type 0`, LOGOS's `succ` says
           `Prop : Type 1`, and LOGOS's OWN level model says `Type 0`
@lean      Prop, Sort 0, Type 0, `#check Prop`, a term whose type is the type of Prop
@expect    reject
@target    (none — there is no level to emit that is right under both of LOGOS's own
           universe procedures)
@evidence  kernel/src/term.rs:196-202 (`Universe::succ`:
           `Universe::SProp | Universe::Prop => Universe::Type(1)`, `Type(n) => Type(n+1)`);
           kernel/src/type_checker.rs:54 (`Term::Sort(u) => Ok(Term::Sort(u.succ()))` — the
           ONLY place a sort's type is computed, so `infer_type(Sort(Prop))` is
           `Sort(Type 1)`); kernel/src/recheck.rs:802 (the de Bruijn re-checker computes it the
           same way, so the second kernel does not catch it);
           kernel/src/term.rs:282-294 (`to_lnat` — the ℕ-ENCODING the decision core runs over:
           `Prop => Const(0)`, `Type(n) => Const(n+1)`, and its own doc comment at :276-281
           says "`Prop = 0`, `Type n = n+1` … This mirrors Lean's level model");
           kernel/src/term.rs:405-416 (`level_leq` = `lnat_leq(simp_lnat(to_lnat(a)),
           simp_lnat(to_lnat(b)))`), and term.rs:243-249 / :250-254
           (`is_subtype_of` and `equiv` are BOTH defined as `level_leq` — so the ℕ-model is the
           sole arbiter of the level ORDER)
@diag      E_TARGET_UNIVERSE_SEMANTICS_INCONSISTENT(Prop) — must state that LOGOS's successor
           operation and LOGOS's own level-ordering procedure disagree about `Prop`, by one,
           and that neither choice of emitted level is faithful to Lean in every position.
           Must produce no output for any declaration whose typing depends on the sort of
           `Prop`.
@guard     **TIER 11 THESIS.** LOGOS's kernel accepting a term is NOT evidence that the
           translation is faithful. Here the target kernel is not merely more permissive than
           Lean (T11-001) — it is INTERNALLY INCONSISTENT, by one, about exactly one universe.
           Walk it:
             · `Universe::succ` (term.rs:196-202) sends `Prop ↦ Type(1)`.
             · `to_lnat` (term.rs:282-294) sends `Prop ↦ 0` and `Type n ↦ n+1`.
             · In that ℕ-model the successor of `Prop` is `0 + 1 = 1`, and `1` is the encoding
               of `Type 0`. So under the model that `level_leq` — hence `is_subtype_of`, hence
               `equiv`, hence EVERY universe comparison the kernel makes — decides over,
               `succ(Prop)` ought to be `Type(0)`.
           `succ` says `Type(1)`. The two procedures disagree by one, about `Prop` only.
           A translator cannot pick a level that is right in both, because the kernel is asking
           two different questions in two different places, and no emitted term changes either.
@guard     Catches "just follow LOGOS: emit `Prop : Type 1`." Lean says `Prop : Type 0`, so
           every Lean declaration whose type mentions the type of `Prop` — a universe-
           polymorphic statement instantiated at `u := 0`, a `Sort u`-valued family, anything
           quantifying over `Sort 0` — comes out one level too high. And because the emitted
           level is too HIGH, cumulativity (T11-001) will usually accept it anyway: `Type 1` is
           a supertype of nothing that helps, but a use site expecting `Type 1` will take a
           `Type 0` happily, so the mismatch surfaces only in the rarer direction.
@guard     Catches "just follow Lean: emit `Prop : Type 0`." Then `infer_type` on the emitted
           `Sort(Prop)` returns `Sort(Type 1)` regardless — `type_checker.rs:54` is not
           configurable — so the translator's recorded type and the kernel's inferred type
           differ. Any translator that checks round-tripping ("`infer_type` of the emitted term
           equals the translation of Lean's type") will report a failure it cannot fix, and the
           tempting repair is to stop checking round-tripping. That repair disables the
           faithfulness oracle for the whole corpus.
@guard     Catches hoping the second kernel catches it. `recheck.rs` is a deliberate
           `lean4checker` analogue using de Bruijn indices, and it computes the sort of a sort
           with the SAME `u.succ()` (recheck.rs:802). Independent re-checking does not help
           when both checkers share the level algebra.
@guard     Catches the `SProp` confusion riding along. `Universe::succ` sends `SProp` to
           `Type(1)` too (term.rs:198), and `to_lnat` sends `SProp` to `0` as a "never-hit
           fallback" (term.rs:284-286) while `level_leq` special-cases `SProp` BEFORE the
           ℕ-encoding (term.rs:409-414). So `SProp` has three different treatments in three
           places, in a sort that nothing in LOGOS can construct or declare (T01-013). Anything
           a translator emits at `SProp` is untestable.
@oracle    Translator exits non-zero with E_TARGET_UNIVERSE_SEMANTICS_INCONSISTENT(Prop) for
           every declaration below whose typing depends on the sort of `Prop`, and emits
           nothing. A harness additionally asserts the discrepancy directly against the target:
           `Universe::Prop.succ()` must equal `Universe::Type(1)` while
           `level_leq(Universe::Prop.succ(), Universe::Type(0))` is FALSE and
           `level_leq(Universe::Type(0), Universe::Prop.succ())` is TRUE — i.e. the kernel
           itself reports `succ(Prop)` as strictly above `Type 0`, one level higher than its
           own ℕ-model puts it.
-/

namespace T11_002

/-- Lean's judgement, stated so the harness has a fixed point to compare against:
    `Prop` lives in `Type 0`, not `Type 1`. -/
example : Type 0 := Prop

/-- Equivalently: `Prop` is `Sort 0` and its type is `Sort 1 = Type 0`. -/
example : Prop = Sort 0 := rfl

/-
  def propInTypeOne : Type 1 := Prop

  Lean 4.14 REFUSES this, because it has no cumulativity and `Prop`'s type is exactly
  `Type 0`:

  error: type mismatch
    Prop
  has type
    Type : Type 1
  but is expected to have type
    Type 1 : Type 2

  So Lean pins `Prop : Type 0` exactly. LOGOS's `infer_type(Sort(Prop))` returns
  `Sort(Type 1)` (type_checker.rs:54 via term.rs:198), which is the ascription Lean just
  refused.
-/

/-- The compiling neighbour, and the whole content of the case: the ascription Lean accepts is
    at level 0, and the one LOGOS computes is at level 1. -/
def propInTypeZero : Type 0 := Prop

/-- A `Sort`-polymorphic identity instantiated at `Prop`. This is the realistic shape: the
    level is never written, it is inferred, and the inferred level is where the off-by-one
    lands. -/
def idSort (α : Sort u) (a : α) : α := a

example : idSort Prop True = True := rfl

/-- A family whose index is a `Prop`-valued type. Its formation uses `imax`, and `imax`'s
    `Prop` arm (term.rs:219-236) is consistent with the ℕ-model — so the inconsistency is
    isolated to `succ`, which makes it easy to miss. -/
def propFamily (p : Prop) : Prop := p → p

/-- The `Prop`-versus-`Type 0` confusion seen from the SUBTYPING side, which is T11-001's
    mechanism: LOGOS says `Prop ≤ Type 0` under the very ℕ-model that says `succ(Prop)` should
    be `Type 0`. So LOGOS simultaneously holds that `Prop` is BELOW `Type 0` in the order and
    that `Prop`'s type is ABOVE `Type 0`. Lean holds neither. -/
def propAsLowerBound : Prop := True

/-
  def propIsAType : Type 0 := True

  error: type mismatch
    True
  has type
    Prop : Type
  but is expected to have type
    Type : Type 1

  Lean refuses; LOGOS accepts by cumulativity (type_checker.rs:1333). Together with the
  `succ` discrepancy above, a translator has no single level assignment for `Prop` that makes
  both of LOGOS's procedures agree with Lean.
-/

/-- Universe-polymorphic statement instantiated at `u := 0`, where `Sort u` becomes `Prop`.
    This is where the off-by-one reaches ordinary code. -/
def constSort (α : Sort u) (_ : α) (β : Sort v) : Sort v := β

example : constSort Prop True Nat = Nat := rfl

end T11_002
