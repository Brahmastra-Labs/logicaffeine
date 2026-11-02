/-!
@case      T02-019
@tier      2 · Declarations
@title     `open … in` — name resolution completes before translation begins
@lean      open, open … in, open scoped, `Lean.Elab.OpenDecl`, ambiguous-name resolution
@target    the resolved `Term::Global(<flattened canonical name>)` — `open` itself emits
           nothing
@expect    translate
@evidence  kernel/src/term.rs:452-458 (`Term::Global(String)` and `Term::Const { name, .. }`
           carry a single flat, absolute name — there is no relative or scope-dependent
           reference form); kernel/src/context.rs:449-464 (`get_global` is a plain map lookup:
           the name in the term must already be the key); kernel/src/context.rs:20-100 (no
           open-namespace stack, no alias table, no `export` map);
           kernel/src/elaborate.rs:1285 (even *dot notation* is resolved by string
           concatenation against the receiver's type head, not by scope)
@guard     Catches emitting the *short* name that appeared in the source. `open Foo in def g :=
           bar` must emit `Global("Foo_bar")`, not `Global("bar")`. Because `add_definition`
           does not typecheck (context.rs:442-444), a term carrying the unresolved short name
           is registered happily, and `get_global("bar")` returns `None` only when something
           later normalizes through it.
@guard     Catches the far worse version of the same bug: the short name *does* resolve, to
           something else. LOGOS's global namespace is flat and already populated by the
           prelude (`Nat`, `Bool`, `true`, `false`, `Eq`, `refl`, `And`, `Or`, `Ex`, `True`,
           `False`, `Acc`, `Decidable`, `Int`, `TList`, `Quot`, …, prelude.rs:533-1460). A
           source that writes `succ` under `open Nat` and is translated as `Global("succ")`
           silently binds to the **prelude's** `Succ`-adjacent names rather than to the Lean
           declaration it meant. Nothing errors; the proof is simply about a different
           constant.
@guard     Catches re-resolving names at translation time. Lean's resolution is order- and
           scope-sensitive and can be *ambiguous* (two opens providing the same short name,
           disambiguated by elaboration or by an explicit prefix). A translator must read the
           already-resolved `Expr.const` from the elaborated term and must never re-run
           resolution against its own model of the namespaces.
@guard     Catches treating `open` as a declaration. It produces no environment entry, so it
           must produce no LOGOS registration — in particular no alias definition
           `add_definition("bar", ty, Global("Foo_bar"))`, which would create a *second*
           delta-transparent name for the same thing and double the surface for T02-020
           collisions.
@guard     Catches `open scoped`. It brings scoped notation and scoped instances into view;
           the notation is expanded before translation (T09-001), but a scoped *instance*
           becoming visible changes which instance an obligation resolves to — and LOGOS's
           instance list is global and permanent (T02-012).
@oracle    No LOGOS registration corresponds to any `open` command; every emitted `Global`/
           `Const` name is the flattened *canonical* Lean name of the constant the elaborated
           term refers to; and the emitted term is unchanged if the source's `open`s are
           rewritten to fully qualified names.
-/

namespace T02_019

namespace Alpha
def token : Nat := 1
def shared : Nat := 10
end Alpha

namespace Beta
def token : Nat := 2
def shared : Nat := 20
def betaOnly : Nat := 99
end Beta

section UsingAlpha
open Alpha

/-- `token` here is `Alpha.token`. Emitting `Global("token")` loses which one. -/
def fromAlpha : Nat := token

end UsingAlpha

example : fromAlpha = 1 := rfl

-- `open … in` scopes the opening to a single declaration. `token` here is `Beta.token`,
-- while three lines above the same short string denoted `Alpha.token`: the source text is
-- identical, the emitted `Global` must not be.
open Beta in
def fromBeta : Nat := token

example : fromBeta = 2 := rfl

-- Both namespaces open at once. `shared` is ambiguous, so Lean requires disambiguation;
-- the elaborated term names exactly one constant.
open Alpha Beta in
def disambiguated : Nat := Alpha.shared

example : disambiguated = 10 := rfl

-- A short name that collides with a LOGOS *prelude* global. Under `open Nat` the source
-- writes `succ`; the canonical name is `Nat.succ`. Emitting `Global("succ")` would target
-- the prelude's namespace instead.
open Nat in
def usesSucc (n : Nat) : Nat := succ n

example : usesSucc 1 = 2 := rfl

-- `open ... renaming` gives a constant a second short name in scope. The canonical name is
-- unchanged and is the only thing that may be emitted.
open Alpha renaming token -> alphaTok in
def viaRenaming : Nat := alphaTok

example : viaRenaming = 1 := rfl

-- `open ... hiding` removes a short name from scope without touching the environment:
-- `Beta.token` stays unreachable by its short name here, but `Beta.betaOnly` does not.
open Beta hiding token in
def viaHiding : Nat := betaOnly

example : viaHiding = 99 := rfl

end T02_019
