/-!
@case      T09-011
@tier      9 · Metaprogramming and syntax
@title     Unicode identifiers and `«guillemet names»` — the kernel accepts them, every LOGOS
           SURFACE does not, and one surface reclassifies them by their first character's case
@lean      unicode identifiers (α, β, Γ, ℕ), «guillemet names», subscripts, primes, dotted names
@expect    degrade
@target    `Term::Var(String)` / `Term::Global(String)` (kernel/src/term.rs:449, :452) — the
           kernel's names are plain Rust `String`s and hold any UTF-8. The degradation is at the
           three vernacular SURFACES and in the flat namespace, not in the term language.
@evidence  kernel/src/term.rs:449, :452 (`Var(String)` and `Global(String)` — NAMED, not de
           Bruijn, and `String` is UTF-8, so `α` is a perfectly good kernel name);
           kernel/src/context.rs:20-80 (every map is keyed by flat `String`: there is no `Name`
           type, no namespace structure, and no escaping convention — so `Foo.bar` and
           `Foo_bar` both key to whatever the translator flattens them to, see T02-020);
           proof/src/tactic_script.rs:89-100 (the script lexer's word rule is
           `c.is_alphanumeric() || c == '_'`. Rust's `char::is_alphanumeric` IS Unicode-aware,
           so `α` and `Γ` DO lex as words — but `«`, `»`, `'`, `!`, `?` and every space inside a
           guillemet name do not, and fall to the `other =>` arm at :101 with
           `ScriptError("unexpected character …")`);
           proof/src/formula.rs:158-166 (Surface B's lexer, same rule and the same
           `unexpected character '{other}'` failure);
           proof/src/formula.rs:437-445 (`ident_to_term` — THE RECLASSIFICATION: an
           identifier is a `Constant` iff its first character satisfies
           `c.is_uppercase() || c.is_ascii_digit()`, otherwise it is a `Variable`. This is
           `char::is_uppercase`, which is Unicode-aware: `Α` (U+0391 GREEK CAPITAL ALPHA) is
           uppercase and `α` (U+03B1) is not);
           base/src/intern.rs:34-39 (`pub struct Symbol(u32)` — the language-side interner is a
           u32 handle over arbitrary `String`s; interning itself imposes no character
           restriction)
@loss      SURFACE_LEXICALITY. Kernel-level names survive intact. Names that must appear in a
           SCRIPT — a hypothesis name in `intro h` / `exact h` / `cases h` / `rewrite h`, or a
           witness in `exists w` — must be `[A-Za-z0-9_]`-shaped plus whatever Unicode letters
           `is_alphanumeric` admits, and must contain no `«`, `»`, `'`, `?`, `!` or space. Lean
           names like `h'`, `«my hyp»` and `h✝` therefore have to be RENAMED before they can be
           referenced from a script, and the renaming must be applied consistently to every
           later reference.
@loss      NAMESPACE_FLATTENING. Lean's hierarchical `Name` becomes a flat `String` key
           (context.rs:20-80). The separator the translator chooses is a naming convention with
           no support from LOGOS, and collisions are possible — see T02-011 and T02-020.
@guard     THE CASE-RECLASSIFICATION TRAP, and the reason this case is not simply "names are
           strings". On Surface B (`formula.rs:437-445`) an identifier's ROLE is decided by its
           first character's case: uppercase or digit ⟹ `ProofTerm::Constant`, lowercase ⟹
           `ProofTerm::Variable`. Lean's conventional type variables are Greek lowercase — `α`,
           `β`, `γ` — and `char::is_uppercase('α')` is FALSE, so they become VARIABLES. But
           `Γ`, `Δ`, `Σ`, equally conventional in Lean for contexts and sums, ARE uppercase and
           become CONSTANTS. Two names the Lean source treats identically are split into
           different FOL categories by nothing but their codepoint. Since `assumption` and
           `exact` compare `ProofExpr`s structurally (proof/src/tactic.rs:363, :380), a
           `Constant("Γ")` never matches a `Variable("Γ")`, and the resulting failure names a
           hypothesis rather than a lexical rule. Compare T08-017, where the same
           `Constant`/`Variable` split bites for a different reason.
@guard     Catches guillemet names reaching a surface. `«my theorem»` is a single Lean
           identifier containing a space. The script lexer (tactic_script.rs:89-101) will lex
           `my` as a word, `theorem` as another word, and fail on `«` before either. There is no
           escaping convention in any LOGOS surface, so the only sound handling is to RENAME and
           record the mapping.
@guard     Catches the PRIME. `h'` is overwhelmingly common in Lean (`h`, `h'`, `h''`). The
           apostrophe is not alphanumeric and is not `_`, so `exact h'` lexes as the atom
           `exact h` followed by the character `'`, which errors — meaning the script both
           fails AND, before failing, would have referenced the wrong hypothesis if the lexer
           had been more permissive. Renaming `h'` to `h_1` is the obvious fix and collides
           with the name `cases` generates for a conjunct (tactic.rs:516). The renaming scheme
           must avoid the machine-generated namespace: `h_1`, `h_2`, `h_l`, `h_r`, `h_w`,
           `W{n}`, `K{n}`, `ih`, `hp{i}`.
@guard     Catches Lean's INACCESSIBLE names. A bare `intro` produces `a✝`, and `✝` (U+271D) is
           not alphanumeric. Those names cannot be referenced from Lean either, but they DO
           appear in the elaborated `Expr`'s binder names, so a translator that copies binder
           names through will put a `✝` into a `Term::Var` — legal in the kernel, unusable in
           any script that must name it. See T08-001.
@guard     Catches assuming the kernel is the constraint. It is not: `Term::Var(String)` holds
           anything. A translator that "fixes" unicode names at the KERNEL level — mangling `α`
           to `alpha` in emitted terms — has done unnecessary damage and made its output harder
           to relate to the source, while leaving the actual surface problem unsolved.
@oracle    Every name the translator emits into a SCRIPT matches `[A-Za-z0-9_]+` and is not in
           the machine-generated namespace listed above; the translator publishes a
           Lean-name → LOGOS-name mapping for every name it renamed; kernel-level `Term::Var` /
           `Term::Global` names are NOT mangled; and the loss record names SURFACE_LEXICALITY
           and NAMESPACE_FLATTENING. A harness additionally asserts that no two distinct Lean
           names map to the same flat LOGOS key.
-/

namespace T09_011

/-- Greek LOWERCASE type variables — Surface B would read these as FOL variables. -/
def idAlpha {α : Type} (x : α) : α := x

/-- Greek UPPERCASE — same role in Lean, but `char::is_uppercase` is true, so Surface B would
    read these as FOL constants. -/
def idGamma {Γ : Type} (x : Γ) : Γ := x

/-- Both in one declaration, so the split happens inside a single type. -/
def pairAlphaGamma {α Γ : Type} (a : α) (g : Γ) : α × Γ := (a, g)

/-- Unicode SYMBOLS in a name (not letters): `ℕ` is `is_alphabetic` but not ASCII. -/
def ℕplus (n : Nat) : Nat := n + 1

/-- SUBSCRIPTS, which Lean treats as part of the identifier. -/
def x₁ : Nat := 1
def x₂ : Nat := 2

theorem subscripted : x₁ + x₂ = 3 := rfl

/-- PRIMES — the most common unicode-adjacent hazard, and the one that collides with LOGOS's
    machine-generated `h_1` naming when renamed naively. -/
theorem primed (p : Prop) (h : p) : p ∧ p := by
  have h' := h
  have h'' := h'
  exact ⟨h', h''⟩

/-- A «GUILLEMET NAME» containing a space — one Lean identifier, three script tokens. -/
def «a name with spaces» : Nat := 7

theorem usesGuillemet : «a name with spaces» = 7 := rfl

/-- A guillemet name containing characters no LOGOS surface can lex at all. -/
def «weird?!name» : Nat := 8

-- A DOTTED name, which flattens into the same `String` keyspace as everything else.
namespace Inner
def value : Nat := 9
end Inner

theorem dotted : Inner.value = 9 := rfl

/-- A hypothesis whose name would collide with LOGOS's `cases` output if renamed to `h_1`. -/
theorem collisionHazard (p q : Prop) (h : p ∧ q) (h_1 : p) : p := by
  obtain ⟨hp, _⟩ := h
  exact h_1

end T09_011
