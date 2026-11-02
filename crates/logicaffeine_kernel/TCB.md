# The Trusted Base

Every axiom `StandardLibrary::register` puts into a `Context`, named, classified and argued.

An **axiom** is a thing we *assume* instead of *proving*. Every one is a place where a reader has
to take our word for something, so the set is small enough to read, and this file is the reading.
`crates/logicaffeine_tests/tests/tcb_ledger_lock.rs` enumerates the live `Context` and holds this
document to it: every axiom the prelude registers is classified here, nothing is listed that it
does not register, and the count below is the live count.

## The comparison

Lean 4's trusted base is **three** axioms: `propext`, `Classical.choice`, `Quot.sound`. Across the
5,827 declarations `scripts/lean/conformance-lean-oracle.sh` compiled with Lean 4.14.0, those three plus
`sorryAx` (the deliberate `sorry` cases), `Lean.ofReduceBool`, `Lean.trustCompiler` and `lcProof`
are everything Lean reached for.

The measured LOGOS inventory is **63** assumptions. Checked integer division and remainder,
and the proof of multiplication monotonicity under nonnegative factors, discharge the last three
entries in the `Definable` category. Both kernels check the actual bodies of all sixteen public
integer laws and seven division helpers/aliases; all have empty axiom and unresolved-dependency
closures.

| class | measured count | what it means |
|---|---|---|
| `Primitive` | 15 | Lean has an equivalent kernel primitive. |
| `Definable` | 0 | Lean defines it and we assume it: nothing. All 22 original assumptions in this category, and `Quot_rec`, now have checked definitions. |
| `Reflection` | 37 | An axiomatised operational semantics for the deep `Syntax`/`Derivation` embedding. Lean has no counterpart. |
| `DomainSpecific` | 11 | No Lean counterpart; temporal arithmetic for the programming language. |

This closes the classified integer-definition debt. It does not establish complete Lean parity
or remove the reflection and domain-specific assumptions.

---

## Primitive — Lean assumes, or kernel-primitivises, the same thing

### Lean's own axioms, under LOGOS spellings

* **`propext`** — `Π(p q : Prop). Iff p q → Eq Prop p q`. One of Lean's three. Unprovable:
  `Prop` has no extensionality rule, and without it there is no `Set`, no quotient
  extensionality and no `simp` on `↔`.
* **`Classical_choice`** — `Π(A:Type). Nonempty A → A`. One of Lean's three (`Classical.choice`).
  `Nonempty α` is a *proposition* and `α` is *data*; turning one into the other is exactly the
  content of choice. LOGOS registers `Nonempty` at `Prop` with a data-carrying constructor
  precisely so the kernel refuses to large-eliminate it and this declared axiom is the only route
  to a witness — deriving it instead (by putting `Nonempty` in `Type 0`) would be choice the
  census could never see.
* **`Quot_sound`** — `Π(A)(r)(a b). r a b → Eq (Quot A r) (Quot_mk A r a) (Quot_mk A r b)`. One of
  Lean's three (`Quot.sound`).

`funext` is **not** on this list, and that is deliberate: Lean does not assume it, and neither do
we. Neither is `dne` — `Π(P:Prop). Not (Not P) → P` — which used to stand where Lean puts
`Classical.em`: Lean PROVES excluded middle (Diaconescu) and proves `Classical.byContradiction`
downstream of it, and as of wave 4 so do we. Both are in *What we used to assume and now prove*,
below. **These three ARE the foundational fragment.** Everything classical in the kernel —
`Classical_em`, `Classical_propDecidable`, `Classical_indefiniteDescription`, `Classical_choose`,
`Classical_choose_spec`, `Classical_axiomOfChoice` and `dne` — is a kernel-checked DEFINITION whose
axiom closure is `propext`, `Classical_choice` and the quotient primitives, and nothing else.

### Quotients

`Quot`, `Quot_mk`, `Quot_lift`, `Quot_ind` are kernel primitives in Lean too (`Quot`, `Quot.mk`,
`Quot.lift`, `Quot.ind`). `Quot_rec` is not a primitive and not an assumption: Lean 4.14.0 prints
`Quot.rec` as `@[reducible] protected def Quot.rec.{u, v} … := fun f h q => Eq.ndrecOn ⋯ (Quot.lift
(Quot.indep f) ⋯ q).snd`, with `#print axioms Quot.rec` = `[Quot.sound]` — a dependent eliminator
DEFINED from `Quot.lift` into `PSigma` and `Quot.ind` — and ours is that definition. See *What we
used to assume and now prove*.

The CARRIER is universe-polymorphic as Lean's is — `Quot.{u} : Π(A : Sort u). (A → A → Prop) →
Sort u` — so quotienting a `Type` is ordinary rather than unstateable, and `Quot_lift`'s target is
Lean's second level parameter, `Sort v`. `Quot_rec`'s motive is `Quot A r → Type 0` where Lean's is
`Sort v`: a capability gap, not a trust gap.

### `native_decide`

`reduceBool` and `ofReduceBool` are exactly Lean's `Lean.reduceBool` / `Lean.ofReduceBool` pair —
the same shape, gating the same thing (an evaluator outside the kernel deciding a `Bool`). The
evaluator behind ours differs in content from Lean's; the assumption has identical force.

### `Float`

`Float_add`, `Float_sub`, `Float_mul`, `Float_div`, `Float_lt`, `Float_le`.

Lean's `Float` is opaque and its operations are `@[extern] opaque Float.add : Float → Float →
Float` — they exist, they have types, the kernel never unfolds them, and the compiler implements
them natively. Ours are declarations of exactly those types, and the posture is the same: no
theorem about floating-point arithmetic is provable from them, and their meaning is given outside
the kernel.

**The one honest difference, stated rather than glossed.** Lean's are `opaque` *definitions*, so
they do not appear in Lean's `#print axioms`; ours are *declarations*, so they appear here. That is
because LOGOS's `Float` has no constructors — there is no body to write, and a definition with an
invented body would COMPUTE a wrong answer, which is the one outcome worse than not computing.

Each is nevertheless a **conservative** assumption: every one of them is a *function type* into an
already-declared carrier, so `λ_ _. c` inhabits it for any `c` of the result type and no new
closed proof of `False` can pass through them. The downward path is real and named: give `Float` a
representation (a structure over a 64-bit word), and these six become `Definable` and then
theorems.

`add`/`sub`/`mul` deliberately stay `Int`-only. Reusing them at `Float` would make `add 1.5 2.5`
well-typed and silently reduce it with integer semantics.

---

### Observational equality (registered on request)

`observational::register` adds `Obs`, `Obs_refl`, `Obs_transp` and `Obs_cast` beside the standard library;
the ledger above is the standard library's and does not count them. They are primitives with
computation rules, in the sense the quotient's are: `Obs.{u} A a b : SProp` reduces on the weak-head
normal form of `A` (pointwise at a function type, logical equivalence at `SProp` and `Prop`, by
formers at a universe, by constructors at an inductive family, by the equivalence closure of the
relation at a quotient) and `Obs_cast` reduces where the equality of its two types does. The rules are
Pujet and Tabareau's CC^obs (POPL 2022, 2023), whose set model interprets `SProp` as truth values
and `Obs` as equality of the interpreted values; the full rules are `observational`'s module
documentation. Three guards are part of the rules and each is locked by a test that
derives `SFalse` without it: a rule never compares two proofs by their constructors, never compares
a proposition seen as a type by its former (`Nat → False` and `Bool → False` are one proposition),
and compares an inductive family's arguments only when they are small enough for its universe to
store (Hur's injective `I : (Type → Type) → Type` refutes excluded middle).

With the constants registered, function extensionality and propositional extensionality hold by
conversion, and `Quot_sound` is a theorem (the classes' equality is the equivalence closure of the
relation, squashed). The constants are certified like the quotient: the rules fire only for the
names the certificate holds, displacing any name it rests on revokes it, and the same spelling and
signature without it is an ordinary assumption, reported as one by `axiom_dependencies`.

## Definable — the original 22 assumptions now have checked definitions

The canonical Int carrier has `Int_ofNat` and `Int_negSucc` constructors. Public `add`, `mul`,
`sub`, `le`, `div`, and `mod` are checked definitions over that carrier. Literal acceleration
attests the relevant checked bodies and dependencies and respects unfolding boundaries.

Sixteen public laws are proved: `add_zero`, `mul_zero`, `add_comm`, `mul_one`, `mul_comm`,
`mul_assoc`, `add_assoc`, `mul_distrib_add`, `sub_elim`, `le_refl`, `le_trans`, `le_total`,
`lt_add1_le`, `le_add_mono`, `le_sub`, and `le_mul_nonneg`. Their actual proof bodies pass both
kernels with empty transitive axiom and unresolved-dependency sets. Their public signatures
are preserved.

The combined division audit also checks `Nat_divModSucc`, `Nat_divMod`, `Int_divMod`, `Int_div`,
`Int_mod`, `div`, and `mod`. Checked Euclidean division and remainder include the signed and
zero-divisor cases; their registration does not add a native trusted arithmetic hook.

Of the original 22, the measured `Definable` inventory holds none, and the one entry of another
origin, `Quot_rec`, is now a checked definition as well (see Quotients above).

---

## Reflection — the deep embedding's operational semantics

Thirty-seven axioms, and Lean has no counterpart to compare them against, which is exactly why they
are their own class instead of being counted among the primitives.

`syn_step`, `syn_eval` and `concludes` axiomatise the evaluator of the `Syntax` / `Derivation`
embedding that `phase87_reflection.rs` through `phase95_incompleteness.rs` build quoting, the
diagonal lemma and Gödel's First Incompleteness Theorem on top of. The evaluator reads the global
environment (a step of an applied name is the kernel's own reduction of it), which no recursion
over `Syntax` can state. The algebra under it is defined (*What we used to assume and now prove*).

The `try_*` family (`try_refl`, `try_compute`, `try_cong`, `try_ring`, `try_lia`, `try_cc`,
`try_simp`, `try_omega`, `try_auto`, `try_decide`, `try_script`, `try_induction`, `try_inversion`,
`try_rewrite`, `try_rewrite_rev`, `try_destruct`, `try_apply`, `try_bitblast`, `try_tabulate`,
`try_hw_auto`, `try_assumption`, `try_split`, `try_left`, `try_right`), the induction-goal trio
(`induction_base_goal`, `induction_step_goal`, `induction_num_cases`) and the combinators
(`tact_fail`, `tact_orelse`, `tact_try`, `tact_repeat`, `tact_then`, `tact_first`, `tact_solve`)
are tactics as proof-term builders. `phase98_verified_reflection.rs` states the cost in its own
header and it is stated again here: the decision procedures are native Rust hooked into
`normalize`, and the "recompute in `concludes`" check re-runs the SAME Rust — so roughly 3,500
lines of unverified Rust sit in the TCB and reflective proofs stop at `Syntax` rather than
reaching `Prop`.

Four of these (`try_assumption`, `try_split`, `try_left`, `try_right`) were invisible to a
source-grep census of `prelude.rs` because their registration call form differs. That is why the
lock enumerates the live `Context` instead of parsing the source: a grep reported 86 where the
truth was 90.

---

## DomainSpecific — no Lean counterpart

Temporal and duration arithmetic for the first-class `Duration` / `Date` / `Moment` value types:
`add_duration`, `sub_duration`, `mul_duration`, `div_duration`, `date_add_days`, `date_sub_date`,
`moment_add_duration`, `moment_sub_moment`, `date_lt`, `moment_lt`, `duration_lt`.

These are opaque primitive operations in the same sense as the `Int` ALU ones, and they could be
defined if those types were given constructors. They are tracked separately because, unlike `Int`,
they have no Lean equivalent to be measured against — counting them as debt against Lean would be
measuring against nothing.

---

## What we used to assume and now prove

### The reflection algebra — moved from *seven assumptions* to *definitions*

`syn_size`, `syn_max_var`, `syn_lift`, `syn_subst`, `syn_beta`, `syn_quote` and `syn_diag` were
declared constants whose meaning was native Rust in `normalize`. Each is now a structural recursion
over `Syntax`, checked like any definition (`reflection.rs`), so its meaning is the definition's.
The native code stays as an accelerator the main kernel runs only where the context's definition,
its `Syntax` and `Univ` and the integer operations it computes with are exactly the registered ones
(`reflection::attested`); everywhere else δ unfolds the definition. The independent checker always
computes them by unfolding, so every accelerated use is checked against the definition it stands
for, and `reflection/tests.rs` runs the accelerator and the definition side by side on a generated
corpus.

### `funext` — moved from *axiom* to *theorem*

`funext` is one of the four constants a Lean user reaches for, and it is **not** one of Lean's
three axioms: Lean proves it from `Quot.sound`. An earlier wave of this campaign assumed it, which
bought at a permanent cost to the trusted base something the reference implementation gets for
free. It is now a kernel-checked DEFINITION in `prelude.rs`, and
`tests/tcb_ledger_lock.rs::the_constants_lean_proves_are_never_logos_axioms` locks the door behind
it.

The proof is Lean's. Write `F := Π(x:A). B x` and let `eqv` be *pointwise* equality on `F` — a
relation, not an equality, so quotienting by it assumes nothing. At each point `x`, lift the single
application `λu. u x` through `Quot F eqv`; its respect obligation is
`eqv u w → Eq (B x) (u x) (w x)`, which the hypothesis supplies pointwise, so no extensionality is
needed to build it. That gives `ext : Quot F eqv → F` with

```
ext (Quot_mk F eqv u)  ≡  λx. u x  ≡  u
```

— the first step by the quotient ι-rule, the second by η. Then `Quot_sound F eqv f g h` proves
`Quot_mk F eqv f = Quot_mk F eqv g`, and substituting along it in the motive `λq. f = ext q`
carries `f = ext (Quot_mk F eqv f)` — which is `refl`, by the conversion above — to
`f = ext (Quot_mk F eqv g)`, which IS `f = g` by the same conversion.

Extensionality of functions therefore follows from the one thing quotients already assume. The η
step is load-bearing, which is why this is a proof and not a restatement.

It remains a theorem you APPLY and never a conversion rule the kernel silently gained: `def_eq`'s
`Lambda`/`Lambda` arm stays α-equivalence, so `λn. addz n` and `λn. n` are still not convertible.

### `Quot_rec` — moved from *assumption* to *definition*

The dependent quotient eliminator was declared with `add_declaration`, so a proof by `Quot_rec`
rested on an assumption Lean does not make. It is now Lean's own definition
(`Init/Core.lean:1581-1611`), transcribed by `recursor::derive_quot_rec` and registered through the
checked gateway at the unchanged statement:

```
indep f a             := ⟨Quot_mk a, f a⟩              : PSigma (Quot A r) motive
indepCoherent f h a b := PSigma.eta (Quot_sound p) (h a b p)
L                     := Quot_lift A r (PSigma (Quot A r) motive) indep indepCoherent
liftIndepPr1 q        : PSigma_fst (L q) = q           -- Quot_ind, then refl
Quot_rec … q          := liftIndepPr1 q ▸ PSigma_snd (L q)
```

The pair is `PSigma`, Lean's dependent pair at any sorts, because the carrier is `Sort u`; that
is also why `Quot_lift`'s target is `Sort v`. `Quot_rec … (Quot_mk a) ⤳ f a` holds in both kernels
by δ, `Quot_lift`'s ι-rule, the pair's projection and `Eq_rec_dep` at `refl` — the eliminator has
no reduction rule of its own. Its axiom closure is `Quot_sound`, read through the body, which is
Lean's `#print axioms Quot.rec`. `logos-coc-6` is the prelude version that carries the change.

### The `Int` strict order — moved from *four axioms* to *five theorems*

`lt`, `ge` and `gt` were opaque declarations beside `le`, and `lt_succ_le` — the integer
discreteness fact every `omega` refutation of a strict system runs through — was an axiom. All
four are gone from the trusted base, and a fifth constant that never existed came with them.

Lean does not assume any of them. It writes

```
Int.lt a b := a + 1 ≤ b        GE.ge a b := LE.le b a        GT.gt a b := LT.lt b a
```

and those are now our bodies, verbatim. None of them needs an `Int` constructor, which is why
they could leave while the ring laws cannot: they are definable from `le` alone.

Once `lt a b` *is* `le (a + 1) b`, the discreteness bridge is a CONVERSION rather than a fact:

```
lt_succ_le : Π a b. a < b → (a + 1) ≤ b  :=  λa b h. h
le_succ_lt : Π a b. (a + 1) ≤ b → a < b  :=  λa b h. h
```

`le_succ_lt` is the one that had been missing, and its absence was a capability gap, not a
convenience: of the eight assumed order laws **none concluded a strict `<`** — `lt_succ_le` and
`lt_add1_le` both take one as a *premise* — so a `calc` chain mixing `≤` and `<` had no step that
could close it (T08-021). It arrives as a theorem, so the capability was bought with a proof.

**A body is not a second implementation.** A primitive computation rule fires on a NAME, and both
kernels already refuse to let one outrank a definition of that name (`reduction.rs`'s
`get_definition_body` gate and `recheck.rs::shadowed_by_a_definition`). So `lt 2 3` δβ-unfolds to
`le 3 3` and is decided by `le`'s rule in *both* kernels, from ONE specification instead of two
copies — and the `+ 1` promotes to `BigInt` at `i64::MAX` rather than wrapping, which is the row
`P_prelude_extra.rs`'s adversarial grid exists to pin.

**The trap this does not fall into.** T04-019 warns against closing the `Nat` ordering gap by
reusing the opaque `Int` builtin. Nothing here touches it: `lt : Int → Int → Bool` still refuses a
`Nat`, and `Nat_lt` / `decLtNat` remain the separate, proved, computing layer they were.

### `dne` — moved from *axiom* to *theorem*, and the classical layer with it

This is the one that changes the sentence a Lean reader hears. LOGOS assumed
`dne : Π(P:Prop). ¬¬P → P`, and Lean has no such axiom: `Classical.em` is **Diaconescu's theorem**
(`Init/Classical.lean:32-63`), proved from `Classical.choice`, `funext` and `propext`, and
`Classical.byContradiction` — `dne` under another name — is two steps downstream of it
(`Init/Classical.lean:131`, through `propDecidable` at `:75`). Assuming it was a fourth
foundational assumption standing exactly where the reference implementation has a proof.

The derivation is now ported, in Lean's own order:

```
Classical_choice ─▶ indefiniteDescription ─▶ choose / choose_spec
                              │
funext + propext ─────────────┴─▶ em (Diaconescu) ─▶ propDecidable ─▶ dne
```

`U x := (x = True) ∨ P` and `V x := (x = False) ∨ P` are each inhabited, so `choose` picks `u` and
`v` and `choose_spec` returns `U u` and `V v`. If either lands on the right, `P` holds. If both
land on the left then `u = True` and `v = False`, so `u ≠ v` — and `True ≠ False` is itself proved,
by transporting `I : True` along `Eq Prop True False` into `False`, not assumed. Meanwhile `P`
implies `u = v`: under `hp : P` every `U x` and `V x` is inhabited in both directions, so `propext`
makes them the same proposition pointwise, `funext` makes `U` and `V` the same function, and
substituting along that equality carries `choose U e₁ = choose U e₂` — which is `refl`, because the
two existence proofs prove one proposition and conversion is proof-irrelevant — to
`choose U exU = choose V exV`. Contraposing gives `¬P` in the first case and `P` in the second.

`#print axioms Classical.em` in Lean lists `propext, Classical.choice, Quot.sound`, and ours lists
`propext, Classical_choice, Quot_sound`: the four quotient formers carry registered primitive
identities and are not assumption leaves, as `ConstantInfo.quotInfo` is not in Lean.
`kernel_H_honesty_extra.rs` diffs the two closures over every prelude constant.

`Classical_em`, `Classical_propDecidable`, `Classical_indefiniteDescription`, `Classical_choose`,
`Classical_choose_spec`, `Classical_axiomOfChoice` and `dne` are therefore all definitions.
`Eq_rec_dep` (full J), `Eq_rec`, `Eq_sym`, `Eq_trans`, `Or_rec`, `Ex_rec`, `HEq_rec`, `heq_of_eq`,
`cast`, `cast_refl`, `eq_of_heq`, `Setoid`, `Quotient`, `Quotient_mk`, `Quotient_sound`,
`Quotient_lift`, `Quotient_ind`, `Quotient_rec` and `nonemptyNat` likewise cost the trusted base
nothing.

### `eq_of_heq` — moved from *missing* to *theorem*

The bridge from a heterogeneous equation back to `Eq` was the one gap this document used to record
as unclosable. Lean's proof (`Init/Prelude.lean:464`) eliminates the `HEq` at the motive

```
P B x y _ := Π(hc : Eq (Type 0) A B). Eq B (cast A B hc x) y
```

and closes the `HEq.refl` case with `rfl`, which asks the kernel to accept `cast h a ≡ a` for an
ARBITRARY `h : α = α`. Lean's kernel does that by **K-like ι-reduction**; ours fires the `Eq`
ι-rule on a syntactic `refl` only, and an earlier wave concluded the theorem was out of reach and
left the gap open rather than buy it with Coq's `JMeq_eq`.

It was not out of reach — the same fact comes through a second door. LOGOS's conversion is
PROOF-IRRELEVANT, so `hc : Eq (Type 0) A A` and `refl (Type 0) A` are already definitionally equal,
and therefore

```
refl (Eq (Type 0) A A) hc  :  Eq (Eq (Type 0) A A) hc (refl (Type 0) A)
```

type-checks. That is Streicher's K held as a PROPOSITION rather than as a reduction rule, and a
proposition can be substituted along: rewriting `hc` to `refl` inside `Eq A (cast A A hc a) a`
leaves a cast whose proof is syntactically `refl`, which ι-reduces, and `refl A a` closes it
(`cast_refl`). `eq_of_heq` is Lean's term with `cast_refl` where Lean's kernel uses K. No axiom was
added and no reduction rule was added; the missing step was supplied by the rule Lean's K-like
ι-reduction is itself justified by.

---

## Where we are better than Lean, and where we are not

**Better: the audit trail cannot be lost.** `prelude::axiom_dependencies` computes the transitive
axiom closure of a *term*, following both a definition's body and its type. `Certificate` and
`TheoremCertificate` re-derive it against the trusted base they would be re-checked in — not
against whatever the minting side chose to disclose — and a certificate whose closure contains an
ADMITTED name (`sorryAx`, `sorry`, `admit`, `lcProof`, `Lean.trustCompiler`) does not re-check at
all. Lean warns on the declaration that *contains* the `sorry` and says nothing about its
dependants, so a theorem three lemmas downstream of an admission looks finished and `#print axioms`
is a command you have to remember to run. Here the taint is a property of the proof rather than of
a warning you might have missed, and it cannot be lost by renaming, re-exporting or citing at a
distance.

**Better: the disclosed delta may extend the trusted base and never replace it.** A
`TheoremCertificate` carries the theorem's own vocabulary and premises in the open, and
`Context::add_declaration`/`add_definition` both DISPLACE the name they register — one name means
one thing — so a delta entry named after a prelude constant would replace it: a certificate could
disclose `dne : Π(P:Prop). P`, prove `False`, re-check green with both kernels agreeing, and report
an axiom set naming only `dne` — a constant the reader believes they already trust.
`recheck_theorem` refuses any delta that collides with a name the rebuilt prelude owns, and names
the collision. Disclosure is only honest if the names in it mean what the reader's own
prelude says they mean.

**Better: two kernels.** Every certificate is corroborated by a second, independently written de
Bruijn kernel (`recheck.rs`). Lean has one. A shared *specification* implemented twice catches what
one implementation agreeing with itself never can. The converse holds too: arithmetic SHARED between
the two kernels (a derive, a copied helper) is a shared bug, and `double_check` will say `Agreed` all
the way to the wrong answer, so the second kernel implements each rule from the specification rather
than calling the first.

**Indexed elimination uses the recorded parameter count.** The independent checker's `Db::Match`
reduction drops `Context::inductive_num_params`, so indices are never mistaken for constructor
arguments. This does not establish complete indexed/nested inductive parity.

**Remaining trust debt.** The inventory is 63 assumptions: 15 Primitive, 0 Definable, 37 Reflection
and 11 DomainSpecific. Six public integer operations, sixteen integer laws and `Quot_rec` are
checked definitions. Eliminating the `Definable` category is a concrete trust improvement; it
does not make the total assumption inventory equivalent to Lean's three foundational axioms.
