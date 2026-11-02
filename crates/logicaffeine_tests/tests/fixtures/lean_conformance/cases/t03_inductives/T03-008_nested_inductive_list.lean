/-!
@case      T03-008
@tier      3 · Inductives and structures
@title     Nested inductive through a list-like container — the specialisable fragment
@lean      inductive with a recursive occurrence nested inside a container (`RTree := node :
           List RTree → RTree`), Lean's nested→mutual compilation
@target    Context::add_nested_inductive(&NestedDecl { name, sort: Sort(Type 0), constructors })
           — which compiles to a mutual block plus kernel-checked conversion isos
@expect    degrade
@evidence  kernel/src/context.rs:623-652 (`add_nested_inductive` runs the **untrusted**
           `inductive_compile::compile_nested`, registers the resulting block through the
           trusted mutual machinery, then `infer_type`s every iso and requires it to match its
           declared conversion type both ways);
           kernel/src/inductive_compile.rs:1-22 (the whole strategy: replace `TList RTree` by a
           fresh sibling `RTree$TList` defined mutually with `RTree`, plus isos
           `RTree$TList ↔ TList RTree`);
           kernel/src/inductive_compile.rs:256-270 (**refused unless `decl.sort` is exactly
           `Type 0`**: "specialized siblings are `Type 0`; higher-universe nesting is not
           supported");
           kernel/src/inductive_compile.rs:218-232 (**only pure unary containers**: every
           constructor argument must be the bare element `A` or a recursive `Container A`;
           anything else is `ArgKind::Other` and the compile is refused);
           kernel/src/inductive_compile.rs:118-136 (`analyze_container_ctor` takes
           `params[0]` as *the* element parameter — the container must have exactly one);
           kernel/src/inductive_compile.rs:302-327 (the emitted siblings are `Type 0` with
           `num_params: 0`);
           kernel/src/inductive_compile.rs:288-295 (a declaration with no nested occurrence is
           refused — "register it directly")
@loss      NESTING_IS_NOT_PRIMITIVE. The nested type does not exist in LOGOS as such. What
           exists is a mutual block plus a pair of isomorphisms. Every term the Lean source
           wrote at type `List RTree` must be transported across `RTree$TList_to_TList` /
           `_from_TList`, and those transports are *not* definitional: they are ordinary
           definitions that must be unfolded, under the 10000-step `normalize` fuel
           (reduction.rs:52-69).
@loss      GENERIC_CONTAINER_LEMMAS. The sibling `RTree$TList` is a **different inductive** from
           `TList`. Nothing proved about `TList` in general applies to it; every lemma must be
           re-proved for the specialisation or routed through the isos. Lean's nested
           inductives share the container's API.
@loss      SIBLING_PARAMETERS. Emitted siblings carry `num_params: 0`
           (inductive_compile.rs:322-327). A nested inductive that was *parameterised* in Lean
           (`RTree α := node : List (RTree α) → RTree α`) therefore loses the parameter on the
           sibling, and the block's shared telescope (recursor.rs:208-212) is taken from
           `block[0]` — which the compiler also emits with `num_params: 0`
           (inductive_compile.rs:300-305). The parameter must be monomorphised before the
           nested declaration is attempted.
@loss      CONTAINER_MUST_PRE-EXIST. `specialize` requires the container to be a *registered
           inductive* already (`ctx.is_inductive(c)`, inductive_compile.rs:210). Lean's `List`
           is not in the LOGOS prelude at all (T10-012), so the translator must first declare a
           monomorphic list-like container and then nest through *that*.
@guard     Catches declaring the nested type directly with `add_inductive` +
           `add_constructor_checked`. `check_positivity` treats `TList RTree` as a parameter
           type in which `RTree` occurs, and `is_recursive_arg` accepts a *block member applied
           to its parameters* — so `TList RTree` may well pass positivity while being nothing
           of the kind. Even where it passes, the resulting inductive has no recursor that can
           recurse *through* the list, because `constructor_analysis` (recursor.rs:126) has no
           way to see the nested recursive occurrences. The recursion the Lean source performs
           is then underivable.
@guard     Catches attempting the nesting at a sort other than `Type 0`. `compile_nested`
           refuses outright (inductive_compile.rs:259-270), so this fails loudly — but a
           translator that "helpfully" downgrades the declared sort to `Type 0` to get past the
           check has changed the type.
@guard     Catches dropping the isos. `add_nested_inductive` returns the iso names
           (`NestedInfo`, inductive_compile.rs:58-64); those are the only bridge between the
           specialised sibling and the generic container, and a translator that discards them
           leaves every `List RTree`-typed subterm of the source untranslatable.
@guard     Catches assuming the compile is trusted. It is explicitly untrusted
           (inductive_compile.rs:19-22); soundness comes from the mutual positivity check and
           from `add_nested_inductive`'s own `infer_type` of each iso (context.rs:639-647). A
           translator that bypasses `add_nested_inductive` and registers the compiled block by
           hand skips the iso check.
@oracle    The nested type is registered via `add_nested_inductive`; `mutual_block_of(name)`
           lists the specialised siblings; each returned iso name resolves to a definition whose
           inferred type is its declared conversion type; and the loss report contains
           NESTING_IS_NOT_PRIMITIVE.
-/

namespace T03_008

/-- A local, monomorphic list-like container, standing in for the container LOGOS would need
    to have registered before any nesting is attempted. It is a *pure unary container*: every
    constructor argument is either the bare element or a recursive occurrence — exactly the
    fragment `analyze_container_ctor` accepts. -/
inductive TList (α : Type) where
  | tnil : TList α
  | tcons : α → TList α → TList α

/-- The canonical nested inductive: `RTree` occurs as the *element* of `TList`, not to the
    right of an arrow. -/
inductive RTree where
  | rnode : TList RTree → RTree

-- Recursion through the nesting. Lean compiles this against the mutual block it generated;
-- LOGOS must recurse through the specialised sibling `RTree$TList`.
mutual
  def rsize : RTree → Nat
    | RTree.rnode kids => Nat.succ (rlistSize kids)

  def rlistSize : TList RTree → Nat
    | TList.tnil => 0
    | TList.tcons t rest => rsize t + rlistSize rest
end

def leafR : RTree := RTree.rnode TList.tnil

example : rsize leafR = 1 := rfl
example : rsize (RTree.rnode (TList.tcons leafR (TList.tcons leafR TList.tnil))) = 3 := rfl

/-- **Deeper nesting**: `TList (TList RTree2)`. `specialize` handles arbitrary depth by
    recursive specialisation, emitting one sibling per container level in post-order
    (inductive_compile.rs:256-258, :274-290) — so this declaration produces *two* siblings and
    *two* iso pairs, not one. -/
inductive RTree2 where
  | rnode2 : TList (TList RTree2) → RTree2

def leafR2 : RTree2 := RTree2.rnode2 TList.tnil

/-- A constructor with a nested field **and** an ordinary field. The ordinary field passes
    through unchanged (`is_nested_container_type` is false for it,
    inductive_compile.rs:152-160); only the nested one is specialised. -/
inductive Labelled where
  | lnode : Nat → TList Labelled → Labelled

mutual
  def lsum : Labelled → Nat
    | Labelled.lnode n kids => n + llistSum kids

  def llistSum : TList Labelled → Nat
    | TList.tnil => 0
    | TList.tcons t rest => lsum t + llistSum rest
end

example : lsum (Labelled.lnode 2 (TList.tcons (Labelled.lnode 3 TList.tnil) TList.tnil)) = 5 := rfl

/-- A container application that is **not** a nesting: `TList Nat` does not mention the
    inductive being declared, so it passes through as an ordinary field
    (inductive_compile.rs:152-160). A translator that specialises every container application
    emits a useless sibling and, because a declaration with no genuine nesting is refused
    outright (inductive_compile.rs:288-295), may also reach for `add_nested_inductive` where
    plain registration was required. -/
inductive WithNats where
  | wn : TList Nat → WithNats

/-- A *parameterised* nested inductive. Lean accepts it; the LOGOS compiler emits siblings
    with `num_params: 0` (inductive_compile.rs:322-327), so the parameter must be
    monomorphised first — one LOGOS declaration per instantiation, with the attendant
    T01-008 losses. -/
inductive PRTree (α : Type) where
  | pnode : α → TList (PRTree α) → PRTree α

def pleaf : PRTree Nat := PRTree.pnode 0 TList.tnil

example : (pleaf : PRTree Nat) = PRTree.pnode 0 TList.tnil := rfl

end T03_008
