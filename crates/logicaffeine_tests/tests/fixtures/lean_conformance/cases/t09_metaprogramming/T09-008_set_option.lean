/-!
@case      T09-008
@tier      9 · Metaprogramming and syntax
@title     `set_option` — pragmas are erased, but some of them CHANGE THE ELABORATED TERM, so
           erasure is only sound after the elaboration they affected
@lean      set_option, set_option ... in, per-declaration options, options that change the Expr
@expect    degrade
@target    Nothing is emitted for the pragma itself. The declarations it scopes translate by
           the ordinary rules.
@evidence  kernel/src/context.rs:20-80 (`Context` — no options map, no configuration, no
           reducibility settings, no elaboration flags. Every field is a flat map of terms);
           kernel/src/type_checker.rs, kernel/src/reduction.rs (the kernel's behaviour is fixed
           at compile time: `normalize`'s fuel is the CONSTANT 10000
           (reduction.rs:52-69) and not a settable option; there is no
           `maxRecDepth`, no `maxHeartbeats`, no `pp.*`);
           kernel/src/term.rs:443-545 (`Term` — no `mdata` variant, so even Lean's own carrier
           for per-node configuration has nowhere to go, see T00-009);
           kernel/src/context.rs:201 (`set_binder_kinds` — the ONLY per-declaration setting
           LOGOS records, and it stores `ParamKind::{Explicit, Implicit, Instance}`
           (elaborate.rs:1016-1027), nothing else. In particular there is no reducibility hint,
           so `@[reducible]`/`@[irreducible]` are equally uncarriable, see T00-013);
           kernel/src/certificate.rs:28 (`PRELUDE_VERSION = "logos-coc-1"` — the single piece of
           environment metadata a certificate carries)
@loss      PRAGMA_ERASURE. Every `set_option` is dropped. For the large majority — `pp.*`,
           `trace.*`, `maxHeartbeats`, `linter.*`, `diagnostics`, `warningAsError` — this is
           exact, because they affect Lean's presentation, resource limits, or diagnostics, none
           of which the elaborated `Expr` records. Acceptable, and the loss record exists so the
           erasure is visible rather than assumed.
@loss      ELABORATION_AFFECTING_OPTIONS. A minority of options change the `Expr` itself, and
           they must be separated from the cosmetic majority. `autoImplicit` and
           `relaxedAutoImplicit` decide whether an unbound identifier becomes an implicit
           binder, so they change a declaration's ARITY (compare T01-011 and T05-008);
           `structureEta` changes which terms are definitionally equal, so it changes which
           `rfl`s elaborate at all; `bootstrap.*` changes what the elaborator is allowed to
           assume. For these the erasure is sound ONLY because Lean has already elaborated
           under them — the governing rule of this tier restated: **translate the
           post-elaboration `Expr`, never the surface `Syntax`.** A translator that
           re-elaborates anything must carry the options forward or it will produce a
           different term.
@guard     THE "ALL PRAGMAS ARE COSMETIC" TRAP. It is tempting to drop every `set_option` with a
           blanket rule and no record. Most are cosmetic; `autoImplicit` is not. Under
           `autoImplicit true` (Lean's default) the declaration `def idAuto (x : α) : α := x`
           elaborates with `α` bound as an implicit universe-polymorphic type argument; under
           `autoImplicit false` it is an ERROR. The two produce a different declaration and a
           different arity, and the difference is recorded nowhere except in the pragma. A
           translator working from the elaborated `Expr` is safe; one that re-derives binders
           from surface syntax is not.
@guard     Catches losing `set_option ... in`'s SCOPE. The `in` form applies to exactly one
           following declaration; the bare form applies to the rest of the section. A translator
           that records options at all must get the scope right, and one that does not record
           them must at least not report that it did.
@guard     Catches `set_option maxRecDepth` / `maxHeartbeats` as though they were a fidelity
           issue. They are not — but their LOGOS analogue is, and it is not settable: `normalize`
           has a hard fuel of 10000 and SILENTLY RETURNS THE PARTIALLY-REDUCED TERM when
           exhausted (reduction.rs:52-69). So a Lean declaration that needed a raised heartbeat
           limit is exactly the one whose LOGOS normalisation will quietly stop early. The
           translator must record that too; see T00-012 and T11-005.
@guard     Catches `set_option linter.unusedVariables false` and friends being treated as
           semantic. They are not, and a translator that refuses a file because it cannot carry
           a linter setting has over-rejected. This corpus deliberately prefers RENAMING an
           unused binder to `_x` over silencing the linter with a pragma, precisely so that no
           case file's meaning depends on an option.
@guard     Catches `set_option` inside a TACTIC block (`set_option ... in tac`). The option
           affects how the tactic elaborates, and the tactic's output is a term. Same rule: the
           term is what translates.
@oracle    No LOGOS declaration is emitted for any `set_option` below. Every declaration in
           scope of one translates, and the loss record names PRAGMA_ERASURE — plus
           ELABORATION_AFFECTING_OPTIONS for the `autoImplicit` section specifically. A harness
           asserts that `idAuto`'s translated LOGOS arity matches the arity of its elaborated
           Lean `Expr`, not the arity its surface syntax suggests.
-/

namespace T09_008

-- A purely COSMETIC option: pretty-printing. Nothing in the `Expr` changes.
set_option pp.explicit true in
theorem cosmetic (n : Nat) : n = n := rfl

-- A RESOURCE option. Nothing in the `Expr` changes; the LOGOS analogue (normalize's fuel of
-- 10000) is not settable at all.
set_option maxHeartbeats 400000 in
theorem resourceBound (n : Nat) : n + 0 = n := by simp

-- A LINTER option. This corpus prefers `_`-prefixed names to this pragma.
set_option linter.unusedVariables false in
theorem linterSilenced (p : Prop) (h : p) : True := trivial

-- THE ELABORATION-AFFECTING CASE. Under `autoImplicit true` (Lean's default) `α` becomes an
-- implicit binder and the declaration is universe-polymorphic; under `false` this is an
-- "unknown identifier" error. The pragma is the only record of which happened.
set_option autoImplicit true in
def idAuto (x : α) : α := x

theorem idAutoWorks (n : Nat) : idAuto n = n := rfl

-- The explicit spelling of what `autoImplicit` produced, for comparison.
universe u

def idExplicit {α : Sort u} (x : α) : α := x

theorem sameArity (n : Nat) : idExplicit n = idAuto n := rfl

-- The BARE form, scoped to the rest of the section rather than to one declaration.
section BareOption

set_option autoImplicit false

def idNoAuto {β : Type} (x : β) : β := x

theorem idNoAutoWorks (n : Nat) : idNoAuto n = n := rfl

end BareOption

-- An option scoping a TACTIC block: the option affects elaboration, the output is a term.
set_option maxRecDepth 2000 in
theorem inTactic (n : Nat) : n * 1 = n := by simp

end T09_008
