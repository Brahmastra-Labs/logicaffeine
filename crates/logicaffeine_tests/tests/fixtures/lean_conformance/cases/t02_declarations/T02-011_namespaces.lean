/-!
@case      T02-011
@tier      2 · Declarations
@title     Namespaces and hierarchical `Name` — LOGOS keys are flat `String`s
@lean      namespace, `Lean.Name` (`.str`/`.num`/`.anonymous`), `export`, `_root_`
@target    Context::add_definition / add_declaration / add_inductive under a *flattened*
           `String` key produced by a documented, injective mangling of `Lean.Name`
@expect    degrade
@evidence  kernel/src/context.rs:20-100 (every global map is
           `HashMap<String, …>`: `inductives`, `constructors`, `inductive_params`,
           `constructor_order`, `declarations`, `definitions`, `universe_polys`,
           `binder_kinds`, `implicit_args`, `structures`, `mutual_blocks` — there is no
           `Name` type anywhere in the kernel); kernel/src/context.rs:449-464 (`get_global`
           is a flat map lookup with a fixed precedence, not a namespace resolution);
           kernel/src/term.rs:452-458 (`Term::Global(String)` and
           `Term::Const { name: String, .. }` — the term language itself carries only flat
           strings); kernel/src/elaborate.rs:1285 (dot notation is resolved by string
           concatenation, `format!("{hname}_{field}")`)
@loss      NAME_STRUCTURE. The hierarchical shape of `Lean.Name` is destroyed. After
           flattening there is no way to ask "which declarations are in namespace `Foo`", no
           `open`, no `export`, no `_root_` escape, and no per-namespace resolution — every
           name is global and absolute.
@loss      NUMERIC_AND_HYGIENIC_COMPONENTS. `Name.num` components (macro scopes, hygiene
           markers, `_@._hyg.123`) have no textual counterpart that is guaranteed
           round-trippable. The mangling must be documented and injective, or two hygienically
           distinct Lean names collapse.
@guard     Catches the obvious mangling `Foo.bar ⟼ "Foo_bar"`, which is **not injective**: a
           Lean declaration literally named `Foo_bar` maps to the same key, and
           `add_definition` is a bare `HashMap::insert` (context.rs:442-444) that overwrites
           without error. That collision is the `reject` case T02-020; this case exists to
           make the translator *choose* a mangling and declare it, so that the collision is
           detectable rather than accidental.
@guard     Catches a mangling that collides with LOGOS's own generated names. `add_structure`
           synthesises `{name}_mk` and `{name}_{field}` (context.rs:304,311), and the nested
           compiler synthesises `{elem}${container}`, `{repr}_to_{c}`, `{repr}_from_{c}`
           (inductive_compile.rs:214-216). A Lean `Point.mk`, `Point.x`, or `RTree_to_TList`
           lands on top of them.
@guard     Catches assuming the LOGOS prelude's namespace is free. The prelude already occupies
           `Nat`, `Zero`, `Succ`, `Bool`, `true`, `false`, `Eq`, `refl`, `And`, `Or`, `Ex`,
           `True`, `I`, `False`, `Acc`, `Decidable`, `Int`, `Text`, `EList`, `TList`, `Quot`, …
           (prelude.rs:533-1460). A Lean `Nat` from a different development silently
           *overwrites* the prelude's `Nat` — including the `Literal::Nat` ↔ `Succ`/`Zero`
           bridge that `extract_constructor` (reduction.rs:330-345) hard-codes by name.
@guard     Catches dropping the mapping table. The loss record must include the full
           `Lean.Name → String` map, or nothing downstream can interpret a LOGOS error message.
@oracle    The emitted `Lean.Name → String` mangling is injective over the module's
           declarations, disjoint from the prelude's names and from every LOGOS-generated name
           shape; and the loss report contains NAME_STRUCTURE with that table.
-/

namespace T02_011

namespace Outer

def value : Nat := 1

namespace Inner

def value : Nat := 2

/-- Resolution is namespace-relative in Lean: this `value` is `Outer.Inner.value`. After
    flattening there is no relative resolution at all. -/
def usesInner : Nat := value

/-- `_root_` escapes to the top level; there is no LOGOS analogue because there is no tree. -/
def usesOuter : Nat := Outer.value

end Inner

end Outer

/-- Three distinct Lean names that a careless mangling merges into one LOGOS key. -/
def Outer_value : Nat := 3
def Outer_Inner_value : Nat := 4

example : Outer.value = 1 := rfl
example : Outer.Inner.value = 2 := rfl
example : Outer_value = 3 := rfl
example : Outer_Inner_value = 4 := rfl

-- `export` makes a name visible under a second path. Both paths denote the *same*
-- declaration, so the mangling must be a function of the canonical name, not of the path
-- used at the reference site.
namespace Re
export Outer (value)
end Re

example : Re.value = 1 := rfl

/-- Guillemet names admit characters a naive mangler will not escape. -/
def «odd name.with dots» : Nat := 5

example : «odd name.with dots» = 5 := rfl

-- A name that shadows a LOGOS prelude global. Flattened to `"Nat"` this replaces the
-- prelude's inductive `Nat`, breaking the literal/Peano bridge.
namespace Shadow
def Nat : Type := Bool
end Shadow

end T02_011
