/-!
@case      T07-008
@tier      7 · Typeclasses
@title     Instance priority — LOGOS has none; resolution is first-match-wins in INSERTION order
@lean      `instance (priority := 900)`, `@[instance]` with priority, `attribute [-instance]`,
           `local instance`, Lean's later-declared-wins default
@expect    degrade
@target    A total ORDER on the emitted `add_instance` calls, chosen by the translator to
           reproduce Lean's resolution outcome, plus a loss record pinning that order. LOGOS
           offers exactly one lever: the sequence in which `add_instance` is called. Lean's
           default is "higher priority first, then LATER declaration first"; LOGOS's is
           "EARLIER insertion first". So the naive emission order — source order — inverts
           Lean's tie-breaking.
@evidence  kernel/src/context.rs:71 (`instances: Arc<Vec<(Term, Term)>>` — a pair, with no
           priority field and no room for one); kernel/src/context.rs:184-186 (`add_instance`
           does `Arc::make_mut(&mut self.instances).push((ty, value))` — APPEND, no key, no
           sort, no dedup); kernel/src/context.rs:210-213 (`instances()` hands out the raw
           slice in insertion order); kernel/src/elaborate.rs:1331-1340 (`for (inst_ty,
           inst_val) in ctx.instances() { … if let Some(result) = try_instance(…) { *mctx =
           trial; return Some(result); } }` — the scan returns at the FIRST candidate that fully
           resolves, and never looks at the rest); kernel/src/elaborate.rs:1190-1197 (doc: "The
           first instance that fully resolves wins; failed trials never pollute `mctx`")
@loss      NO_PRIORITY. Lean's priorities are a partial specification of resolution order that
           the author wrote down deliberately; LOGOS can only approximate them with a linear
           order. Where Lean's priorities are equal and it falls back to declaration order, the
           two systems disagree by DEFAULT: Lean prefers the later declaration, LOGOS the
           earlier. The translator must therefore emit instances of equal priority in REVERSE
           source order, and the loss record must say so — because the resulting LOGOS
           development no longer resembles the Lean source's declaration sequence, and a human
           reading it will draw the wrong conclusion about which instance wins.
@loss      NO_REMOVAL. `attribute [-instance] foo` and `local instance` have no counterpart:
           there is no API to remove or scope an entry once `add_instance` has pushed it
           (context.rs:184-186 is the only mutator). A Lean development that activates an
           instance in one section and deactivates it in the next cannot be expressed; the
           translator must either refuse the file or emit the union, and the union may resolve
           differently everywhere. `localOnly` below is the instance in question.
@loss      NO_DIAGNOSABILITY. Because resolution returns at the first success
           (elaborate.rs:1334-1339), LOGOS never discovers that a second instance would also
           have matched. There is no "ambiguous instance" report and no way to ask after the
           fact which instances were candidates. Everything a reviewer needs to check the
           translator's ordering decision must be in the loss record; it cannot be recovered
           from the LOGOS context.
@guard     Catches emitting instances in source order and assuming it reproduces Lean. It
           inverts the default tie-break. `laterWins` below is the discriminating case: two
           equal-priority instances at the same type, where Lean picks the SECOND and a
           source-ordered emission makes LOGOS pick the FIRST. The translated program computes a
           different number and typechecks perfectly.
@guard     Catches sorting by priority and ignoring declaration order. Lean's rule is
           lexicographic — priority first, then declaration order (later first) — so a stable
           sort by descending priority over the SOURCE sequence still inverts every tie. The
           correct emission is: sort by descending priority, and within each priority band
           reverse the source order.
@guard     Catches assuming a more SPECIFIC instance wins. It does not: specificity plays no
           part in `resolve_instance_at`. A general instance `Show (Box α)` emitted before a
           specific `Show (Box Nat)` captures the specific goal, because `try_instance` freshens
           the general instance's `α` to a metavariable and unifies successfully
           (elaborate.rs:1360-1375). `generalBeatsSpecific` below is that case, and it is the
           one a translator is most likely to get wrong because Lean's behaviour here looks like
           specificity even though it is priority-plus-order.
@guard     Catches emitting a class's instances interleaved with another class's. Order matters
           only WITHIN a class in Lean, but the LOGOS vector is global and `class_heads` is
           recomputed from the whole vector on every resolution (elaborate.rs:1330,1178-1189).
           Interleaving makes the emission order harder to audit and makes any later edit to one
           class silently re-order another. Emit per class, contiguously, and record the blocks.
@oracle    For every method call in this file, the instance LOGOS resolves is the one Lean
           resolves (checked by comparing the reduced values); the emission order is recorded in
           the loss record; the loss record names NO_PRIORITY and, for `localOnly`, NO_REMOVAL.
-/

namespace T07_008

class Tagged (α : Type) where
  tag : Nat

/-- THE DISCRIMINATING CASE. Two equal-priority instances at the same type. Lean picks the
    LATER one; a source-ordered LOGOS emission picks the EARLIER one. -/
instance firstDeclared : Tagged Nat := ⟨1⟩
instance secondDeclared : Tagged Nat := ⟨2⟩

def laterWins : Nat := Tagged.tag (α := Nat)

/-- Lean's answer. LOGOS, emitted in source order, would give 1. -/
example : laterWins = 2 := rfl

/-- Explicit priorities. Higher wins regardless of declaration order. -/
class Ranked (α : Type) where
  rank : Nat

instance (priority := 100) lowPriority : Ranked Bool := ⟨10⟩
instance (priority := 900) highPriority : Ranked Bool := ⟨90⟩

def priorityWins : Nat := Ranked.rank (α := Bool)

example : priorityWins = 90 := rfl

/-- Priority INVERTING declaration order: the earlier declaration has the higher priority. -/
class Inverted (α : Type) where
  value : Nat

instance (priority := 900) earlyHigh : Inverted Nat := ⟨7⟩
instance (priority := 100) lateLow : Inverted Nat := ⟨8⟩

def invertedWins : Nat := Inverted.value (α := Nat)

example : invertedWins = 7 := rfl

inductive Box (α : Type) where
  | mk : α → Box α

class Render (α : Type) where
  render : α → Nat

instance : Render Nat := ⟨fun n => n⟩

/-- A GENERAL instance declared before a SPECIFIC one. Lean prefers the later (specific) one;
    LOGOS, in source order, matches the general one first because `try_instance` freshens its
    `α` to a metavariable and unification succeeds. -/
instance generalBox {α : Type} [Render α] : Render (Box α) :=
  ⟨fun b => match b with | Box.mk a => Render.render a⟩

instance specificBox : Render (Box Nat) := ⟨fun _ => 999⟩

def generalBeatsSpecific : Nat := Render.render (Box.mk (5 : Nat))

/-- Lean's answer, via the later-declared specific instance. -/
example : generalBeatsSpecific = 999 := rfl

-- A `local instance` — active only in this section. LOGOS has no removal API at all.
section
class Scoped (α : Type) where
  scopedValue : Nat

local instance localOnly : Scoped Nat := ⟨5⟩

def usesLocal : Nat := Scoped.scopedValue (α := Nat)

example : usesLocal = 5 := rfl
end

end T07_008
