/-!
@case      T03-009
@tier      3 · Inductives and structures
@title     Nested inductive through `Prod`/`Array`/a binary container — outside the
           specialisable fragment
@lean      inductive nested inside a non-unary or non-pure container
           (`T := mk : Array T → T`, `T := mk : List (Nat × T) → T`)
@target    (none — no sound translation)
@expect    reject
@diag      E_NESTING_NOT_SPECIALISABLE — must name the inductive, the offending container, and
           the specific constructor argument whose kind is neither the element nor a recursive
           occurrence. Must register nothing: the mutual block and the isos are emitted
           together, and half of them is worse than none.
@evidence  kernel/src/inductive_compile.rs:218-232 (the fail-closed gate: for each of the
           container's constructors, if any argument kind is `ArgKind::Other(_)` the compile
           errors — "only pure containers like `List`/`TList`");
           kernel/src/inductive_compile.rs:92-101 (`ArgKind` has exactly three cases: `Elem`
           (the bare element parameter), `Recursive` (`Container A`), and `Other`);
           kernel/src/inductive_compile.rs:118-136 (`analyze_container_ctor` takes
           `params[0]` as **the** element parameter and classifies every later argument
           against it — a *second* type parameter is therefore `Other`, so no binary container
           is ever specialisable);
           kernel/src/inductive_compile.rs:246-252 (anything that is not a nesting inside a
           *registered* container at all is refused outright);
           kernel/src/inductive_compile.rs:256-270 (and the whole declaration is refused unless
           its sort is exactly `Type 0`);
           kernel/src/context.rs:20-100 (LOGOS has no `Array`, no `Prod`, no `Sigma` — see
           T10-011, T10-013 — so the container would have to be declared first, and declaring
           `Prod` does not make it unary)
@guard     Catches specialising a **binary** container. `Prod A B`'s constructor is
           `Π(A).Π(B). A → B → Prod A B`; `analyze_container_ctor` binds `A` as the element and
           then meets `B`, whose type is `Type 0` — `ArgKind::Other`. The compile is refused.
           A translator that "helps" by picking one of the two parameters as *the* element and
           treating the other as a fixed field produces a sibling that mirrors only half the
           container, and the isos it emits will not typecheck against the generic form — but
           only *after* `add_mutual_inductives` has already registered the block
           (context.rs:634-647), leaving a partial declaration behind.
@guard     Catches specialising a container whose constructor stores a non-element field.
           `Array` in Lean is a structure over `List`; any faithful LOGOS stand-in stores at
           least one field that is neither the element nor a recursive occurrence, so it is
           `Other` and refused. The same is true of a length-indexed vector, a rose tree with
           a label, or any container with a size field.
@guard     Catches nesting under a **function type**. `T := mk : (Nat → T) → T` is
           strictly positive and Lean accepts it — but the head of that argument type is a `Pi`,
           not a `Global` container (`is_nested_container_type`,
           inductive_compile.rs:152-160, tests the head), so `compile_nested` does not treat it
           as a nesting at all. It is an ordinary positive recursive occurrence and must be
           registered *directly* (T03-002), never through `add_nested_inductive` — which would
           refuse it with "no nested container occurrence" (inductive_compile.rs:288-295).
           Getting this backwards in either direction is a real failure mode.
@guard     Catches nesting at a sort other than `Type 0`, including `Prop`. The gate
           (inductive_compile.rs:259-270) is on the *declared* sort of the nested inductive,
           not on the container's, and there is no fallback.
@guard     Catches "degrade instead of reject". There is no loss record for this: the mutual
           block that would have to be emitted does not exist, so there is no term to emit
           alongside a note. Emitting the un-nested approximation — declaring the field as an
           opaque type, or dropping the container — produces an inductive that Lean's source
           never mentioned and whose recursor recurses over nothing.
@oracle    Translator exits non-zero with E_NESTING_NOT_SPECIALISABLE naming the container and
           the offending argument; no member of the would-be block, no sibling, and no iso is
           registered.
-/

namespace T03_009

/-- A **binary** container. Its constructor's second argument is a value of the *second*
    type parameter, which `analyze_container_ctor` classifies as `ArgKind::Other`. -/
inductive TPair (α : Type) (β : Type) where
  | tpair : α → β → TPair α β

/-- A unary container that nonetheless stores a non-element field — the label. Also
    `ArgKind::Other`, also refused. -/
inductive Tagged (α : Type) where
  | tnil : Tagged α
  | tcons : Nat → α → Tagged α → Tagged α

/-- The pure unary container, for contrast: this one *is* specialisable (T03-008). -/
inductive TList (α : Type) where
  | tnil : TList α
  | tcons : α → TList α → TList α

/-- **Case 1 — nesting inside a binary container.** `PTree` occurs as the second argument of
    `TPair`. Lean accepts it. No LOGOS specialisation exists. -/
inductive PTree where
  | pstop : PTree
  | pnode : TPair Nat PTree → PTree

def pOne : PTree := PTree.pnode (TPair.tpair 0 PTree.pstop)
def pTwo : PTree := PTree.pnode (TPair.tpair 1 pOne)

/-- Recursion through the binary container — the recursion Lean derives from its generated
    mutual block, and which has no LOGOS sibling to recurse through. -/
def pdepth : PTree → Nat
  | PTree.pstop => 0
  | PTree.pnode (TPair.tpair _ t) => Nat.succ (pdepth t)

example : pdepth pTwo = 2 := rfl

/-- **Case 2 — nesting inside a container with a non-element field.** `LTree` occurs as the
    element of `Tagged`, but `Tagged`'s `tcons` also stores a `Nat`, which makes the whole
    container non-pure. -/
inductive LTree where
  | lnode : Tagged LTree → LTree

def lleaf : LTree := LTree.lnode Tagged.tnil

mutual
  def lsize : LTree → Nat
    | LTree.lnode kids => Nat.succ (ltagSize kids)

  def ltagSize : Tagged LTree → Nat
    | Tagged.tnil => 0
    | Tagged.tcons _ t rest => lsize t + ltagSize rest
end

example : lsize lleaf = 1 := rfl
example : lsize (LTree.lnode (Tagged.tcons 9 lleaf Tagged.tnil)) = 2 := rfl

/-- **Case 3 — nesting inside a *doubly* parameterised container where the inductive is the
    first argument.** Still binary, still refused; the element/non-element split is not the
    issue, the container's arity is. -/
inductive QTree where
  | qstop : QTree
  | qnode : TPair QTree Bool → QTree

def qOne : QTree := QTree.qnode (TPair.tpair QTree.qstop true)

def qdepth : QTree → Nat
  | QTree.qstop => 0
  | QTree.qnode (TPair.tpair t _) => Nat.succ (qdepth t)

example : qdepth qOne = 1 := rfl

/-- **Case 4 — the boundary that must NOT be routed through `add_nested_inductive`.** The
    recursive occurrence is under a `Pi`, not under a container. It is strictly positive, it
    is an ordinary recursive inductive, and `compile_nested` would refuse it with "no nested
    container occurrence". -/
inductive FnTree where
  | fnode : (Nat → FnTree) → FnTree
  | fstop : FnTree

def fdepth : FnTree → Nat
  | FnTree.fstop => 0
  | FnTree.fnode f => Nat.succ (fdepth (f 0))

example : fdepth FnTree.fstop = 0 := rfl
example : fdepth (FnTree.fnode (fun _ => FnTree.fstop)) = 1 := rfl

/-- **Case 5 — nesting in a container applied to a *non*-`Type 0` element.** Even if the
    container were pure, the declared sort gate (inductive_compile.rs:259-270) refuses
    anything but `Type 0`. -/
inductive BigTree : Type 1 where
  | bnode : TList Nat → BigTree
  | bty : Type → BigTree

end T03_009
