/-!
@case      T09-009
@tier      9 · Metaprogramming and syntax
@title     `#check` / `#eval` / `#print` — commands that produce no declaration, and must
           produce no LOGOS output either
@lean      #check, #eval, #print, #print axioms, #reduce, #guard
@expect    translate
@target    Nothing. These commands add no constant to the Lean environment, so there is nothing
           to translate and the correct LOGOS output is the empty set of declarations.
@evidence  kernel/src/context.rs:434, :442 (`add_declaration` and `add_definition` — the ONLY
           two ways a name enters a LOGOS `Context`; both require a name and a term, and a
           `#check` has neither); kernel/src/context.rs:20-80 (`Context` — nothing that could
           hold a query result: no message log, no output buffer, no trace);
           kernel/src/term.rs:443-545 (`Term` — nothing that represents "the type of this
           expression, printed", so even the RESULT of a `#check` has no representation beyond
           the type itself, which is an ordinary term);
           kernel/src/reduction.rs:52-69 (`normalize`'s hard fuel of 10000, silently returning
           the partially-reduced term — the closest LOGOS analogue of `#reduce`, and the reason
           a translator must not try to reproduce `#eval`'s answers by normalising);
           kernel/src/prelude.rs:1195-1219 (`reduceBool`/`ofReduceBool` — the ONE place LOGOS
           deliberately wires an evaluator into kernel reduction, and it is gated to `Bool`;
           it is not a general `#eval`)
@guard     THE PHANTOM-DECLARATION TRAP. `#check foo`, `#eval e` and `#print bar` look like
           commands that "do something", and a translator walking the surface command list will
           be tempted to emit *something* for each — an `add_declaration` for the checked type,
           a definition holding the evaluated value, a comment. All of these pollute the flat
           LOGOS namespace (`HashMap<String, _>`, context.rs:20-80) with names the source never
           declared, and since LOGOS keys are flat strings with no namespaces (T02-011), such a
           name can COLLIDE with a real declaration (T02-020). The correct output is nothing.
@guard     Catches reproducing `#eval`'s ANSWER as a definition. `#eval (2 + 2 : Nat)` prints
           `4`; emitting `add_definition("evalResult", Nat, Lit(Nat(4)))` records a fact the
           source never asserted, and one that is only true because Lean's evaluator agreed with
           the kernel. LOGOS's evaluator is wired into reduction only through `reduceBool`
           (prelude.rs:1195-1219), for `Bool` only, so there is no general correspondence to
           reproduce.
@guard     Catches `#eval` WITH SIDE EFFECTS. `#eval (IO.println "hi")` runs `IO`. LOGOS's
           kernel has no IO at all, so this is not merely a no-op command — it is the same
           rejection as T09-012, reached through a `#`-command. A translator must distinguish
           `#eval` of a pure term (nothing to emit) from `#eval` of an `IO` action (reject and
           report), because the second means the source file DID something that the translation
           does not.
@guard     Catches `#print axioms`. Its output is exactly the audit a translator's own report
           should reproduce — which declarations depend on which axioms — and it is the tool
           that reveals a `sorryAx` dependency that produces no compiler warning (see T08-028).
           A translator should compute the same closure itself; it must not emit the `#print`
           command's textual output as data.
@guard     Catches `#reduce` being taken as a specification. `#reduce e` shows Lean's whnf/full
           reduction. LOGOS's `normalize` has a hard fuel of 10000 and SILENTLY returns the
           partially-reduced term when exhausted (reduction.rs:52-69), so the two agree only for
           small terms. A translator that uses `#reduce` output to validate its emission will
           get false failures on anything large; it must compare with `def_eq`, not with printed
           normal forms. See T00-012 and T11-005.
@guard     Catches over-rejecting. A file full of `#check`s is perfectly translatable — the
           commands simply contribute nothing. A translator that refuses a file because it
           contains a command it does not recognise will refuse most real Lean, where `#check`
           and `#print` are ubiquitous in comments and examples.
@oracle    The translator emits ZERO declarations for every `#`-command below, exits zero, and
           the resulting LOGOS `Context` contains exactly the names declared by `realDef` and
           `realTheorem` — no more. The `#eval` of an `IO` action must be reported (see T09-012),
           and `#print axioms` must not appear in the output at all.
-/

namespace T09_009

/-- A real declaration, so the case has something the translator IS expected to emit. -/
def realDef (n : Nat) : Nat := n * 2

theorem realTheorem : realDef 3 = 6 := rfl

-- `#check` — prints a type, declares nothing.
#check realDef

#check (realDef 3 : Nat)

#check fun (n : Nat) => n + 1

-- `#eval` of a PURE term — prints a value, declares nothing.
#eval realDef 21

#eval (2 : Nat) + 2

#eval [1, 2, 3].length

-- `#print` — prints a declaration's definition.
#print realDef

-- `#print axioms` — the audit that reveals axiom dependencies, including `sorryAx`.
#print axioms realTheorem

-- `#reduce` — Lean's normal form. LOGOS's `normalize` has a hard fuel and may stop early.
#reduce realDef 5

-- `#guard` — a compile-time assertion. It fails the BUILD if false, but still declares
-- nothing, so it too translates to nothing.
#guard realDef 4 == 8

-- `#check` on a term mentioning a universe variable.
universe v

#check (Sort v)

-- `#eval` of an `IO` action — NOT a no-op: it runs IO at elaboration time. See T09-012.
#eval (IO.println "translated" : IO Unit)

end T09_009
