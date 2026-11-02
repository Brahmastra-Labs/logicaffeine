/-!
@case      T02-020
@tier      2 · Declarations
@title     Flat-name collision — `Foo.bar` and `Foo_bar` become the same LOGOS key
@lean      hierarchical `Lean.Name`, namespaces, auto-generated names (`.mk`, `.rec`, field
           projections, `where` auxiliaries, derive handlers)
@target    (none — no sound translation for the colliding pair)
@expect    reject
@diag      E_FLAT_NAME_COLLISION — must name **both** Lean declarations, the single `String`
           key they map to, and which LOGOS map(s) each would land in. Must emit no
           registration for either. A rename is *not* an acceptable automatic repair: the
           chosen key is part of the translator's public contract (every downstream LOGOS
           term, error message and certificate refers to it), so the collision has to be
           surfaced to a human.
@evidence  kernel/src/context.rs:20-100 (every global map is `HashMap<String, …>`; there is
           no `Name` type, no namespace component, no visibility, nothing to distinguish two
           Lean names that flatten alike); kernel/src/context.rs:442-444 (`add_definition` is
           `Arc::make_mut(&mut self.definitions).insert(name, (ty, body))` — a plain
           `HashMap::insert`, so a second registration under the same key **silently returns
           the old value and discards it**; no error, no warning); the same is true of
           `add_inductive` (:253-255), `add_constructor` (:419-428), `add_declaration`
           (:434-436) and `set_inductive_params` (:380-382);
           kernel/src/context.rs:449-464 (`get_global` precedence — inductives →
           constructors → definitions → declarations);
           kernel/src/reduction.rs:304-313 (delta reduction reads `get_definition_body`, i.e.
           the `definitions` map **directly**, bypassing that precedence);
           kernel/src/context.rs:304,311 (`add_structure` synthesises `{name}_mk` and
           `{name}_{field}`); kernel/src/elaborate.rs:1285 (dot notation is resolved by
           `format!("{hname}_{field}")`); kernel/src/inductive_compile.rs:211-216 (the nested
           compiler synthesises `{elem}${container}`, `{repr}_to_{c}`, `{repr}_from_{c}`)
@guard     The same-map case: two Lean definitions, `Foo.bar` and `Foo_bar`, both key to
           `"Foo_bar"`. The second `add_definition` **overwrites** the first. Every term the
           translator already emitted that refers to the first now refers to the second,
           silently, with a type that may well still check — LOGOS is cumulative
           (type_checker.rs:1330-1360), so a `Nat`-valued global replaced by another
           `Nat`-valued global is completely invisible to the kernel. This is the canonical way
           a translated proof library ends up proving a *different theorem* than the source.
@guard     The cross-map case, which is strictly worse. A Lean **inductive** `Foo_bar` and a
           Lean **definition** `Foo.bar` do not overwrite each other — they land in *different*
           maps. `get_global("Foo_bar")` then returns the **inductive's** sort (inductives are
           checked first, context.rs:449-453), while `normalize(Global("Foo_bar"))` unfolds to
           the **definition's** body (reduction.rs:304-313 reads `definitions` directly). The
           kernel's typing and its reduction now disagree about what one name means. Nothing
           reports an error; conversion checks simply start succeeding or failing for reasons
           that have no relation to the source.
@guard     The generated-name case. LOGOS generates `Point_mk` and `Point_x` for a structure
           (context.rs:304,311), and `elaborate_dot` *resolves* `p.x` by building that exact
           string (elaborate.rs:1285). A Lean declaration literally named `Point_x` therefore
           does not merely collide — it becomes what every `p.x` in the translated development
           resolves to. Likewise `Point.mk` (Lean's own constructor) and a user's `Point_mk`.
@guard     The prelude case. The LOGOS prelude already owns `Nat`, `Zero`, `Succ`, `Bool`,
           `true`, `false`, `Eq`, `refl`, `And`, `Or`, `Ex`, `True`, `I`, `False`, `Acc`,
           `Decidable`, `Int`, `Text`, `EList`, `TList`, `Quot`, `dne`, `add`, `le`, …
           (prelude.rs:78-1460). A Lean declaration flattening onto any of them overwrites a
           *trusted base* entry. Overwriting `Nat` in particular breaks the hard-coded
           `Literal::Nat` ↔ `Succ`/`Zero` bridge (reduction.rs:330-345,
           type_checker.rs:1480-1493), which is keyed by those literal strings.
@guard     Catches "detect collisions by checking `get_global` before inserting". That check is
           necessary but not sufficient: it misses a collision against a name the translator has
           not emitted *yet*, and it misses the cross-map case for `set_inductive_params`,
           `set_binder_kinds`, `register_struct_info` and `constructor_order`, each of which is
           its own map with its own silent `insert`. Collision detection must be a whole-module
           pre-pass over the mangled key space, including every LOGOS-generated name shape and
           the prelude's names.
@oracle    Translator exits non-zero with E_FLAT_NAME_COLLISION naming both Lean declarations
           and the shared key, and the LOGOS context receives **no** registration from this
           module. In particular, running the translator twice must not produce a context in
           which some name's `get_global` type and `get_definition_body` body come from
           different Lean declarations.
-/

namespace T02_020

/-! ### 1. Same-map collision: two definitions, one key -/

namespace Foo
/-- Canonical Lean name `T02_020.Foo.bar`; flattens to `…Foo_bar`. -/
def bar : Nat := 1
end Foo

/-- Canonical Lean name `T02_020.Foo_bar`; flattens to the **same** `…Foo_bar`. -/
def Foo_bar : Nat := 2

/-- Both are live in Lean, and they are observably different. After flattening only one
    survives, and every reference to the other silently retargets. -/
example : Foo.bar ≠ Foo_bar := by decide
example : Foo.bar = 1 := rfl
example : Foo_bar = 2 := rfl

/-! ### 2. Cross-map collision: an inductive and a definition -/

namespace Cross
/-- A *definition*. Flattens to `…Cross_kind`. -/
def kind : Nat := 7
end Cross

/-- An *inductive* with the same flat key. These land in different LOGOS maps, so neither
    overwrites the other: `get_global` reports this inductive's sort while `normalize`
    unfolds the definition's body. -/
inductive Cross_kind where
  | only

example : Cross.kind = 7 := rfl
example : (Cross_kind.only : Cross_kind) = Cross_kind.only := rfl

/-! ### 3. Collision with LOGOS-generated structure names -/

structure Point where
  x : Nat
  y : Nat

/-- `add_structure Point` generates the projection definition `Point_x` (context.rs:311), and
    `elaborate_dot` resolves `p.x` to exactly that string (elaborate.rs:1285). This user
    declaration takes over that key — so after translation, `p.x` computes *this*. -/
def Point_x (p : Point) : Nat := p.y + 1000

/-- `add_structure` also generates `Point_mk`, which is Lean's `Point.mk` flattened. -/
def Point_mk (a : Nat) (b : Nat) : Nat := a * b

/-- Observably distinct in Lean: real projection vs. the impostor. -/
example : (Point.mk 3 4).x = 3 := rfl
example : Point_x (Point.mk 3 4) = 1004 := rfl
example : Point_mk 3 4 = 12 := rfl

/-! ### 4. Collision with a `where`-lifted auxiliary -/

/-- Lean lifts the `where` body to `T02_020.withAux.go`, which flattens to `…withAux_go`. -/
def withAux (n : Nat) : Nat := go n
  where go (k : Nat) : Nat := k + 1

/-- …and this user declaration claims that key. -/
def withAux_go (k : Nat) : Nat := k + 500

example : withAux 1 = 2 := rfl
example : withAux_go 1 = 501 := rfl

/-! ### 5. Collision with a LOGOS prelude name -/

namespace Shadow
/-- Flattens to `…Shadow_Nat` here, but a translator that strips the enclosing namespaces —
    a common "simplify the names" move — produces the bare key `Nat`, overwriting the
    prelude's inductive and with it the literal/Peano definitional bridge. -/
def Nat : Type := Bool
/-- Same hazard for the constructor names `Zero` and `Succ`. -/
def Succ : Bool := true
end Shadow

/-! ### 6. Collision with a nested-compiler generated name -/

/-- The nested compiler emits siblings named `{elem}${container}` and isos named
    `{repr}_to_{c}` / `{repr}_from_{c}` (inductive_compile.rs:211-216). A Lean declaration
    named `RTree_to_TList` lands on the iso's key — and the iso is *kernel-checked* at
    registration (context.rs:639-647), so the overwrite happens after the check and is
    invisible to it. -/
def RTree_to_TList (n : Nat) : Nat := n

example : RTree_to_TList 5 = 5 := rfl

end T02_020
