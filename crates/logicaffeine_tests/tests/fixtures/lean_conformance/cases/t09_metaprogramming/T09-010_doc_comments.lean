/-!
@case      T09-010
@tier      9 · Metaprogramming and syntax
@title     Doc comments and module docs — no LOGOS carrier, and the erasure is the same one that
           erases `Expr.mdata`
@lean      /-- doc comment -/, /-! module doc -/, @[inherit_doc], add_decl_doc, source positions
@expect    degrade
@target    Nothing is emitted for the documentation. The documented declaration translates by
           the ordinary rules.
@evidence  kernel/src/context.rs:20-80 (`Context` — `bindings`, `inductives`, `constructors`,
           `inductive_params`, `constructor_order`, `declarations`, `definitions`, `hints`,
           `universe_polys`, `instances`, `coercions`, per-binder kinds. There is no docstring
           map, no comment store, no source-position table, and no metadata slot of any kind);
           kernel/src/context.rs:434, :442 (`add_declaration(name, ty)` and
           `add_definition(name, ty, body)` — the signatures show there is nowhere to put a
           docstring even if one wanted to);
           kernel/src/term.rs:443-545 (`Term` has NO `mdata` variant — Lean's own carrier for
           per-node annotations, including some documentation and every source position, has no
           counterpart. This is exactly the T00-009 erasure, reached from the command level
           rather than the term level);
           kernel/src/term.rs:13-44 (`Literal::Text(String)` exists — so a translator CAN
           smuggle a docstring in as data, which is the trap below);
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"` — the only
           string-valued metadata in a certificate)
@loss      DOCUMENTATION_ERASURE. Every `/-- … -/`, `/-! … -/`, `@[inherit_doc]` and
           `add_decl_doc` is dropped. Acceptable because documentation is not semantic: no Lean
           term's meaning depends on it, and no LOGOS check consults it. The loss record exists
           so the erasure is a recorded decision rather than an unnoticed omission, and so a
           round-trip tool knows it must re-attach documentation from the Lean side rather than
           expect it back from LOGOS.
@loss      SOURCE_POSITION_ERASURE. The same absence of `mdata` (term.rs:443-545) means source
           positions are gone too, so a LOGOS-side error cannot be pointed at a line of Lean. A
           translator must keep its OWN name→position map outside the LOGOS artefact; the
           artefact cannot carry it. This is the practical cost of the erasure, and it is
           larger than the loss of the prose.
@guard     THE DOCSTRING-AS-DATA TRAP. `Literal::Text` (term.rs:13-44) makes it possible to
           emit a docstring as a term — `add_definition("foo_doc", Global("Text"),
           Lit(Text("…")))`. It typechecks. It also puts a name in the flat global namespace
           (context.rs:20-80) that the source never declared, which can collide with a real
           declaration (T02-020); it is invisible to anything that reads documentation; and
           since LOGOS has NO string operations at all (T10-016), nothing can ever use it. The
           correct output is no declaration.
@guard     Catches the ATTRIBUTE half. `@[inherit_doc]` is an attribute, and LOGOS has no
           attribute system (T02-013). A translator that handles docstrings but not
           `@[inherit_doc]` will report different losses for two declarations that differ only
           in how their documentation was written.
@guard     Catches conflating this with a `reject`. Nothing here blocks translation: a fully
           documented file translates perfectly, minus the documentation. A translator that
           refuses a declaration because it cannot carry its docstring has over-rejected, and
           would refuse essentially all library Lean.
@guard     Catches the DIAGNOSTIC consequence being reported as a translation success. Once
           positions are gone, a LOGOS kernel error names a flat string like `"Foo_bar"` and
           nothing else. If the translator's report says only "translated 40 declarations", the
           user has no way back to the source. The loss record must be per-declaration, naming
           the LOGOS key and the Lean position, so the mapping lives in the REPORT even though
           it cannot live in the artefact.
@guard     Catches assuming the module doc is where the corpus metadata lives on the LOGOS side.
           It is not: this corpus's own `@case`/`@guard` blocks are Lean module docs
           (TEMPLATE.md), and they are erased by exactly the rule this case describes. The
           conformance metadata is an input to the harness, never an output of the translator.
@oracle    The translator emits ZERO declarations for the documentation below, emits correct
           declarations for `documented`, `Documented`, `documentedTheorem` and `inheritsDoc`,
           emits no `Term::Lit(Literal::Text(..))` whose payload is a docstring, and produces a
           per-declaration loss record naming DOCUMENTATION_ERASURE and SOURCE_POSITION_ERASURE
           together with the Lean source position it dropped.
-/

namespace T09_010

/-- A documented definition. The prose is dropped; `documented` itself translates. -/
def documented (n : Nat) : Nat := n + 1

/-- A documented structure, whose FIELDS are documented too. Each field becomes a separate
    LOGOS definition via `add_structure` (context.rs:271-369), and none of them can carry
    documentation either. -/
structure Documented where
  /-- The first component. -/
  fst : Nat
  /-- The second component. -/
  snd : Nat

/-- A documented theorem. -/
theorem documentedTheorem (n : Nat) : documented n = n + 1 := rfl

/-- A documented inductive, whose CONSTRUCTORS are documented. -/
inductive DocumentedEnum where
  /-- The affirmative case. -/
  | yes
  /-- The negative case. -/
  | no

/-- A doc comment containing markup, a code span `documented 3`, and a unicode arrow →,
    none of which has anywhere to go. -/
def markupHeavy : Nat := 0

/-- The target of an `inherit_doc`. -/
def original (n : Nat) : Nat := n * 2

@[inherit_doc original]
def inheritsDoc (n : Nat) : Nat := original n

/-- A documented CLASS, so the documentation sits on something that becomes a structure plus an
    instance list on the LOGOS side (T07-001). -/
class Documentable (α : Type) where
  /-- Produce a tag. -/
  tag : α → Nat

end T09_010

/-! A module-level doc section in the middle of the file. Lean records it in the environment's
documentation extension; LOGOS's `Context` has no such extension (see T09-007) and no place for
prose at all. -/

namespace T09_010

/-- A declaration after the module doc, to show the file keeps translating. -/
def afterModuleDoc : Nat := 1

end T09_010
