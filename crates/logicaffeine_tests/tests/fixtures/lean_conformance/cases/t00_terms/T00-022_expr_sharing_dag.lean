/-!
@case      T00-022
@tier      0 · Kernel term layer
@title     `Expr` is a hash-consed DAG; LOGOS `Term` is a TREE — sharing cannot be restored
@lean      Expr pointer sharing, maxSharing, whnf caching, repeated subterms after beta/delta
@expect    degrade
@target    A plain structural translation to `Term` (term.rs:443-545), which is a
           `Box`-recursive tree with no interning and no back-references, PLUS a loss record
           naming TERM_SHARING that reports the expansion factor (translated node count divided
           by Lean `Expr` node count under `maxSharing`).
@evidence  kernel/src/term.rs:443-545 (`Term` is an owned `Box`-recursive enum — there is no
           `Rc`/`Arc` inside a `Term`, no hash-consing table, no node identity, and therefore no
           way to express "this subterm is that subterm"; `Arc` appears only in `Context`'s
           top-level maps, kernel/src/context.rs:228-241, i.e. between DECLARATIONS, never
           within a term);
           kernel/src/type_checker.rs:133-142 (`Let` — the one construct that LOOKS like it
           restores sharing — is zeta-substituted into the body before the body is inferred, so
           the checked term is the fully expanded one);
           kernel/src/reduction.rs:298 (`reduce_step` on `Let` does the same, so the expansion
           is in the reduction relation too, not just in checking);
           kernel/src/type_checker.rs:1082-1110 (`substitute_avoiding` COPIES the replacement at
           every occurrence — beta reduction therefore duplicates rather than shares);
           kernel/src/reduction.rs:40 and :52-69 (`normalize`'s fixed 10000-step fuel, silently
           exhausted); kernel/src/type_checker.rs:1060-1071 (`freshen`'s full-body
           `all_var_names` walk, the other half of the quadratic cost — see T00-021)
@loss      TERM_SHARING. Lean's `Expr` is a hash-consed DAG: `f x x` holds ONE `x` node with two
           references, and `whnf` results are cached so a repeated redex reduces once. The
           translated `Term` holds two independent copies, and each is reduced independently.
           For the doubling chains below the factor is 2^k. Acceptable in the sense that the
           DENOTATION is identical — a tree and its DAG share a unique unfolding — and LOGOS has
           no representation in which the sharing could be preserved. Not acceptable to leave
           unrecorded: the expansion factor is precisely what predicts whether a declaration will
           exhaust `normalize`'s fuel, and a translator that cannot report it cannot predict its
           own failures.
@guard     Catches "reintroduce sharing with `Term::Let`". This is the single most natural
           repair and it does nothing. `type_checker.rs:141` zeta-substitutes the let's value
           into the body BEFORE inferring the body's type, so a `let` binding a large value used
           n times is checked as n copies — the same n copies the translator was trying to
           avoid, plus a `Let` node. There is NO construct in LOGOS that makes the kernel see a
           subterm once. The only real sharing boundary is a top-level `add_definition`
           (context.rs:442), because a `Global` reference is not unfolded until `reduce_step`
           chooses to (reduction.rs:36-91) — so the genuine mitigation is to LIFT the shared
           subterm to a named global, which changes the declaration structure and must itself be
           recorded.
@guard     Catches budgeting from the Lean side. A translator that measures a declaration by
           `Expr` node count under `maxSharing` and concludes "small, will check fine" is
           measuring the DAG. The doubling chain below is a dozen `Expr` nodes and a
           thousand-node `Term`. The prediction must be made on the translated tree.
@guard     Catches assuming beta reduction preserves size. `substitute_avoiding`
           (type_checker.rs:1082-1110) copies the argument into every occurrence of the
           parameter, so one beta step on `(fun x => f x x) big` doubles `big`. Lean's kernel
           does the same in principle but its result is hash-consed back down; LOGOS's is not.
           A chain of k such steps is 2^k.
@guard     Catches concluding that the loss is nil because the two terms are `def_eq`. They are
           — that is exactly why this is `degrade` and not `reject`. The loss is in COST and in
           the translator's ability to predict cost, and the failure mode it produces is a
           SILENT fuel exhaustion (reduction.rs:56) reported as a conversion failure. A harness
           that only checks final acceptance will see this case as clean until the day a
           declaration crosses the threshold.
@guard     Catches trying to recover sharing by pointer-comparing translated subterms. `Term`
           derives structural `PartialEq` (term.rs:441) and has no node identity; two equal
           subterms are indistinguishable from one shared subterm, so a post-hoc "re-share" pass
           cannot tell which occurrences Lean actually shared — and it does not matter, because
           no `Term` can record the answer.
@oracle    A loss record with category TERM_SHARING reports, per declaration, the Lean `Expr`
           node count under `maxSharing` and the emitted `Term` node count; the emitted term is
           `def_eq` to the Lean term's translation; and no `Term::Let` is introduced by the
           translator for the purpose of sharing (lets that were in the Lean source are fine).
-/

namespace T00_022

/-- Each application of `dup` doubles its argument under beta reduction, because
    `substitute_avoiding` copies rather than shares. Ten layers is 2^10 copies of `1` in the
    LOGOS normal form and a handful of nodes in Lean's `Expr`. -/
def dup (x : Nat) : Nat := x + x

def dup4 : Nat := dup (dup (dup (dup 1)))
def dup10 : Nat := dup (dup (dup (dup (dup (dup (dup (dup (dup (dup 1)))))))))

example : dup4 = 16 := rfl

/-- Explicit duplication in the BODY rather than through composition: `twice f x` mentions `x`
    twice, so one beta step copies whatever `x` was. -/
def twice (f : Nat → Nat) (x : Nat) : Nat := f x + f x

def twiceOfTwice : Nat := twice (fun k => twice (fun j => j + 1) k) 1

/-- The `let` that looks like it shares and does not. Lean's `Expr.letE` holds `big` once;
    `Term::Let`'s typing rule (type_checker.rs:141) substitutes it into all four occurrences
    before the body is inferred. -/
def f20 (a01 a02 a03 a04 a05 a06 a07 a08 a09 a10
         a11 a12 a13 a14 a15 a16 a17 a18 a19 a20 : Nat) : Nat :=
  a01 + a02 + a03 + a04 + a05 + a06 + a07 + a08 + a09 + a10 +
  a11 + a12 + a13 + a14 + a15 + a16 + a17 + a18 + a19 + a20

def letDoesNotShare : Nat :=
  let big : Nat := f20 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20
  big + big + big + big

/-- Nested lets, where the inner value mentions the outer name: the expansions COMPOSE, so the
    checked term is exponential in the nesting depth. -/
def nestedLetBlowup : Nat :=
  let a : Nat := 1 + 1
  let b : Nat := a + a
  let c : Nat := b + b
  let d : Nat := c + c
  let e : Nat := d + d
  e + e

/-- The genuine mitigation, for contrast: the shared subterm is a top-level definition, so the
    emitted `Term` holds a `Global` reference (term.rs:448) that `reduce_step` unfolds only on
    demand (reduction.rs:36-91). This is the ONLY sharing boundary LOGOS has. -/
def shared20 : Nat := f20 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20

def usesGlobalSharing : Nat := shared20 + shared20 + shared20 + shared20

example : usesGlobalSharing = letDoesNotShare := rfl

/-- Sharing in a TYPE rather than a term: the same large type appears in four binder positions.
    Lean shares the `Expr`; LOGOS copies it into four `Pi.param_type` boxes, and every
    `is_subtype` comparison against it walks a fresh copy. -/
def bigType : Type := (Nat → Nat → Nat → Nat → Nat) → (Nat → Nat → Nat → Nat → Nat) → Nat

def fourBigTypes (_a _b _c _d : bigType) : Nat := 0

end T00_022
