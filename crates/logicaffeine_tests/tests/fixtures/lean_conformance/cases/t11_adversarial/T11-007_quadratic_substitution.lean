/-!
@case      T11-007
@tier      11 · Adversarial and soundness traps
@title     Quadratic substitution on a deep telescope — the named representation's cost, and
           how it interacts with the reduction budget
@lean      long dependent telescope, 5000-deep application spine, `variable`-heavy sections,
           a structure with very many fields, Mathlib-style bundled algebra
@expect    degrade
@target    The faithful term, PLUS a loss record naming the substitution-cost class. There is
           no sharing in `Term` and no de Bruijn representation available at the kernel API.
@evidence  kernel/src/type_checker.rs:890-902 (`substitute`: the fast path is
           `if !occurs_free(body, var) { return body.clone(); }`, and the doc comment explains
           it exists to turn "quadratic checking (walk the giant argument at every application)
           into linear" for the NON-dependent case — i.e. the quadratic behaviour is
           acknowledged and only mitigated where the bound variable does not occur);
           kernel/src/type_checker.rs:1088-1140 (`substitute_avoiding` walks the whole body and
           rebuilds it, and `free_vars(replacement)` is computed once per substitution at :902);
           kernel/src/type_checker.rs:1061-1071 (`freshen` calls `all_var_names(body, &mut
           avoid)` — a full traversal of the binder's body — EVERY time a capture-avoiding
           rename is needed); kernel/src/type_checker.rs:163-166 (`infer_type`'s `App` arm:
           `check_type(ctx, arg, &param_type)?` then `substitute(&body_type, &param, arg)` — one
           substitution of the WHOLE argument per application node, so an n-deep spine performs
           n substitutions into a body that grows with each one);
           kernel/src/term.rs:443-545 (`Term` is a `Box`-ed TREE — there is no DAG, no hash
           consing, and no sharing, so a substituted argument is CLONED at every occurrence);
           kernel/src/reduction.rs:52-69 (`normalize`'s 10000-step fuel, which the growing
           term consumes faster and faster — see T11-005)
@loss      SUBSTITUTION_COST. Lean's `Expr` is a hash-consed DAG with de Bruijn indices, so
           instantiation is cheap and sharing is preserved. LOGOS's `Term` is a named tree,
           substitution is O(size of body) per binder, and every occurrence of the replacement
           is a full clone. A telescope of depth n therefore costs O(n · size) to type and the
           term's size can grow multiplicatively. The loss record must name the depth and the
           translator's size estimate; "it may be slow" is not a loss record.
@loss      SHARING_IS_LOST. This is T00-022's loss showing through at scale: a Lean `Expr` in
           which one subterm is referenced 100 times becomes a LOGOS `Term` containing 100
           copies. The two losses compound — more copies means larger bodies means more
           expensive substitutions.
@guard     **TIER 11 THESIS.** LOGOS's kernel accepting a term is not evidence of faithfulness,
           and here it is not even evidence that the kernel finished the job. The interaction
           to guard is with T11-005: substitution cost is paid in `reduce_step` calls, and
           `normalize`'s fuel is a hard 10000 that returns the PARTIALLY reduced term with no
           error (reduction.rs:52-69). So a deep telescope does not merely translate slowly —
           past a threshold it translates to a term whose `def_eq` checks answer "false"
           because the budget ran out, and the translator sees a type error where there is
           none.
@guard     Catches the mitigation being mistaken for a fix. `substitute`'s fast path
           (type_checker.rs:897-899) skips the work only when the bound variable does NOT occur
           in the body — the non-dependent case. A DEPENDENT telescope, which is exactly what
           a bundled algebraic structure or a `variable`-heavy section produces, defeats the
           fast path at every level, because each binder's type mentions the previous binders.
           The optimisation's own doc comment says so.
@guard     Catches the capture-avoidance amplification. When a rename IS needed,
           `freshen` traverses the binder's whole body to collect `all_var_names`
           (type_checker.rs:1061-1071), and then `rename_var` performs ANOTHER full
           substitution pass (:1075-1080) before the real substitution descends. A translator
           that reuses Lean's binder-name hints (T11-006) makes collisions common and therefore
           makes this path the norm rather than the exception — turning a constant factor into
           the dominant cost.
@guard     Catches "just inline it" as a size remedy. Inlining a definition instead of emitting
           one `add_definition` removes a delta step but multiplies term size, which makes
           every enclosing substitution more expensive and brings the fuel boundary closer. The
           encodings chosen for T10-012 (emitted `List`), T03-011 (projection as a
           `Match`-bodied definition) and T04-010 (wildcard expanded to every constructor) all
           trade in this currency, and the translator must budget across them rather than
           locally.
@guard     Catches reporting this as a `translate` with a footnote. The verdict is `degrade`
           because the emitted term is faithful but the target cannot carry the source's
           sharing, and because past a threshold the faithful term is not CHECKABLE. A
           translator that emits it and reports success without the loss record has told the
           user that a declaration was translated when what was actually produced may not
           typecheck in a reasonable budget.
@oracle    The emitted term is the faithful one, AND a loss record with categories
           SUBSTITUTION_COST and SHARING_IS_LOST naming the telescope depth and the emitted
           term's node count. A harness asserts that the translator's predicted node count is
           within a factor of two of the actual emitted tree size, and that every declaration
           it predicted would typecheck within budget did so.
-/

namespace T11_007

/-- A dependent telescope: every binder's type mentions the previous ones, so `substitute`'s
    non-occurrence fast path (type_checker.rs:897-899) is defeated at every level. -/
def deepDependent : Type :=
  ∀ (a : Nat) (_ : Fin a) (b : Nat) (_ : Fin (a + b)) (c : Nat) (_ : Fin (a + b + c))
    (d : Nat) (_ : Fin (a + b + c + d)) (e : Nat) (_ : Fin (a + b + c + d + e)), Nat

-- The same shape produced the way real Lean produces it — by a `section`/`variable` block, so
-- the binders are inserted by the elaborator and are invisible in each declaration's source.
section
variable (a : Nat) (b : Nat) (c : Nat)

def usesAll : Nat := a + b + c

theorem allPositive (ha : 0 < a) : 0 < a + b + c := by omega
end

/-- A bundled structure whose later fields' types mention earlier fields — the Mathlib
    algebraic-hierarchy shape. `add_structure` emits one `Match`-bodied projection DEFINITION
    per field (context.rs:271-369), each of which must be substituted into at every use. -/
structure Bundled where
  carrier : Type
  zero : carrier
  op : carrier → carrier → carrier
  leftId : ∀ x, op zero x = x
  rightId : ∀ x, op x zero = x
  assoc : ∀ x y z, op (op x y) z = op x (op y z)

/-- An instance, so the projections are actually applied. -/
def natBundled : Bundled where
  carrier := Nat
  zero := 0
  op := (· + ·)
  leftId := fun x => Nat.zero_add x
  rightId := fun x => Nat.add_zero x
  assoc := fun x y z => Nat.add_assoc x y z

/-- Every projection here is, in LOGOS, a `Match`-bodied global definition applied to the
    structure's parameters and then the receiver (context.rs:310-362). -/
example (x : natBundled.carrier) : natBundled.op natBundled.zero x = x := natBundled.leftId x

/-- A long application spine. `infer_type`'s `App` arm performs one `substitute` of the whole
    argument per node (type_checker.rs:163-166), into a body that has already been substituted
    into by every outer node. -/
def spine10 (a b c d e f g h i j : Nat) : Nat := a + b + c + d + e + f + g + h + i + j

example : spine10 1 2 3 4 5 6 7 8 9 10 = 55 := rfl

/-- A term with heavy SHARING in Lean's DAG — `shared` occurs eight times and Lean stores one
    node. LOGOS's tree stores eight copies, and each copy is substituted into independently. -/
def shared : Nat := 2 + 3

def usesSharedManyTimes : Nat :=
  shared + shared + shared + shared + shared + shared + shared + shared

example : usesSharedManyTimes = 40 := rfl

/-- Sharing under a binder, where each copy also carries the binder's substitution cost. -/
def sharedUnderBinder (n : Nat) : Nat :=
  let m := n + n
  m + m + m + m

example : sharedUnderBinder 3 = 24 := rfl

/-- A binder whose name collides with a name already in the body, forcing the `freshen` path
    (type_checker.rs:1061-1071) — a full extra traversal per substitution. -/
def forcesFreshen (x : Nat) : Nat → Nat := fun y => (fun x => x + y) (x + 1)

example : forcesFreshen 1 2 = 4 := rfl

end T11_007
