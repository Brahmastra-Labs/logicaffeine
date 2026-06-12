# MISSING_ENGLISH.md — Constructions LOGOS does not yet parse to FOL

This catalogs the English phenomena that block "all of English" coverage in the
LOGOS transpiler — the **constructional frontier** (grammar and logical
semantics), the half WordNet/NLTK does not supply. Each entry gives **What it
is**, **What's missing**, **Sentences it would enable** (with target logic), and
an **Implementation** plan.

The canonical meaning is the **typed, intensional, dynamic AST** (35 `LogicExpr`
variants, `crates/logicaffeine_language/src/ast/logic.rs:452-659`); the
FOL/Kripke printers are *exports*; reasoning runs over the export suited to each
phenomenon. The cross-cutting **SOTA principles (P1–P9)** below define the shared
machinery every entry builds on.

Notation: events `∃e` with thematic roles `Agent(e,·)/Theme(e,·)/…`; the
intension operator `^` (for intensional predicates only); structured
propositions `⟨φ⟩` (for attitude objects); kind terms `^Kind`; sorts (`Human`,
`Abstract`, …). Status: **◑** parsed but not correctly modeled · **✗** not
parsed. Reciprocals and metaphor-from-NL are excluded pending confirmation
against the phase tests.

---

## Reuse Map (existing node vs new)

| Phenomenon | Logical home | Work |
|---|---|---|
| Performatives | `SpeechAct{performer,act_type,content}` (logic.rs:579) | parse + reason |
| Imperatives | `Imperative{action}` (logic.rs:574) | enrich roles |
| Belief/opaque | `Intensional{operator,content}` + `Term::Proposition` | wire attitude verbs |
| Causal clauses | `Causal{effect,cause}` (logic.rs:592) | parse `because/since` |
| Counterfactual / inverted | `Counterfactual{antecedent,consequent}` | parse + Kratzer g=similarity |
| Comparatives/equatives | `Comparative` + scale terms | add relation field |
| Clefts | `Focus{kind,focused,scope}` (logic.rs:633) | `FocusKind::Cleft` + exhaustivity |
| Presupposition | `Presupposition{assertion,presupposition}` + DRS | Van der Sandt binding |
| Control | `Control{verb,subject,object,infinitive}` | parse; perception is sibling |
| Cumulative/collective | `Distributive`, `GroupQuantifier` (logic.rs:646-658) | over Link lattice |
| Generics/habituals | `QuantifierKind::Generic` + `Aspectual::Habitual` | parse + non-monotonic layer |
| Proportional/partitive | `QuantifierKind::Most/Few/Many/Cardinal/AtLeast/AtMost` | parse `of the` |
| Evidentials | `Modal` (`ModalFlavor::Evidential`) | flavor + non-assertion |
| Mass nouns | Link lattice (shared with plurals) | sort/lattice work |
| Relational adjectives | `Noun(x) ∧ Rel(x, ^Base)`; `Feature::Relational` + `relational{base,relation,level}` | parse branch + lexicon |
| **New node** | — | Exclamative, Optative, Equative-relation, Vagueness threshold, Metonymy coercion, `Concessive`, `Term::Kind`, `Term::Degree`, `Term::Indexical` |
| **No node (DRS/scope)** | — | Binding theory, scope-in-islands, branching (Skolem), floating Q, non-restrictive RC, coordination variants, implicature |

---

## Integration Spine (the nine touch-points)

Every phenomenon ships through this spine; each entry lists only its deltas.

1. **Lexicon / tokens.** Add words/features to
   `crates/logicaffeine_language/assets/lexicon.json` (rebuilt by `build.rs`);
   add new closed-class markers as a `TokenType` in `src/token.rs` plus a
   `build.rs::generate_lookup_keyword` arm. Tool: `scripts/place-word.py`.
2. **AST node.** Reuse a `LogicExpr` variant (Reuse Map) or add one in
   `ast/logic.rs` (box if >48 bytes — size test logic.rs:683) with a constructor
   in `arena_ctx.rs`. Scope is represented **underspecified** (P7); the printed
   FOL is an export.
3. **Parser.** Build the node in `parser/clause.rs` (clauses/conditionals/
   coordination), `parser/quantifier.rs` (NPs/scope/RCs), `parser/modal.rs`
   (modality), `parser/verb.rs` (argument structure), `parser/pragmatics.rs`
   (focus/presupposition). Discourse-sensitive constructs use `drs.rs`
   (`BoxType`, referents, accessibility).
4. **Semantics.** Meaning postulates in `semantics/axioms.rs` (lexicon-driven via
   `build.rs` `NounAxiom/VerbAxiom/AdjectiveAxiom`); modal lowering in
   `semantics/kripke.rs` reads the Kratzer modal base + ordering source (P1);
   plurals/mass over the Link lattice (P5); degrees over scales (P9).
5. **Print as logic (all 5 formatters).** `transpile.rs::write_logic()` gets a
   match arm; add a `LogicFormatter` method (`formatter.rs:20-243`) implemented
   in **UnicodeFormatter, LatexFormatter, SimpleFOLFormatter, KripkeFormatter,
   RustFormatter** with one symbol per notation.
6. **Exhaustive match sites.** Handle a new variant in `visitor.rs::walk_expr`,
   `debug.rs::DisplayWith`, and `proof_convert.rs::logic_expr_to_proof_expr`.
7. **Reasoning backend (router, not one engine).** `verification.rs` routes each
   construct to the engine that decides it soundly: the **FOL + EUF + LIA core**
   (Z3) for the extensional core; **modal reasoning** via standard translation
   with frame-condition axioms on the modal base / ordering source f,g (T/S4/S5/D)
   (P1); the **non-monotonic layer** (KLM/circumscription/ASP) for defaults,
   generics, implicature (P4); **scales** via LIA/reals (P9); **mereology** via
   the Link lattice axioms (P5). Proof-producing checks pass through
   `proof_convert.rs` to `logicaffeine_proof` and the kernel.
8. **Frontend (compile→display).** `compile_for_ui()`
   (`crates/logicaffeine_compile/src/ui_bridge.rs:328`) precomputes Unicode /
   SimpleFOL / Kripke readings; a new formatter symbol flows through. Touch
   `categorize_token()` (~ui_bridge.rs:101) for a new `TokenType` color,
   `expr_to_ast_node()` (~ui_bridge.rs:178) for a new variant in the AST view,
   `logic_output.rs::convert_to_latex` for a new glyph; add a demo in
   `apps/logicaffeine_web/src/ui/examples.rs` and a guide section in
   `ui/pages/guide/content.rs`. The format toggle, multi-reading switching, and
   syntax highlight are automatic.
9. **Tests (RED-first).** A `crates/logicaffeine_tests/tests/phaseNN_<name>.rs`
   asserting the target logic in each format, plus a `--features verification`
   test that the intended entailment holds (or fails). Never edit a RED test.

---

## SOTA principles (P1–P9)

**P1 — Modality: one framework (Kratzer modal base + ordering source).** Every
modal carries a **modal base** f (the worlds under consideration) and an
**ordering source** g (orders them by ideality / normality / likelihood /
similarity). `ModalVector` carries `modal_base` and `ordering_source` symbols;
Kripke lowering reads g to restrict and order worlds; frame conditions
(T/S4/S5/D) are Z3-checkable properties of f,g. This is the single home for
epistemic, deontic, bouletic (§1.2), circumstantial, evidential (§4.3) modality,
counterfactuals (§4.5, g = similarity), imperatives (§1.4), graded modals,
generics (§6.1, g = normalcy), and conditionals (P2).

**P2 — Conditionals restrict.** An if-clause is a **restrictor on the modal /
quantifier / GEN in its consequent** ("if-clauses restrict quantifiers"). Parse
the if-clause as the restrictor argument of the consequent's operator (covert
epistemic necessity when none is overt). One rule covers indicative (§4.x),
counterfactual (§4.5), biscuit (§4.2, speech-act-level restriction), and donkey
conditionals (DRT).

**P3 — Attitudes are hyperintensional.** The object of an attitude verb is the
**structured proposition** `Term::Proposition(&LogicExpr)` — the complement's
syntactic structure, written `⟨φ⟩` — so co-intensional complements stay distinct.
`^` is reserved for intensional *predicates* (seek/rising), never attitude
objects.

**P4 — Defeasible reasoning layer.** Generics (§6.1), habituals (§4.4), and
implicatures (§8.7) license cancellable inferences. They are reasoned in a
dedicated **non-monotonic layer** (KLM/System-P preferential entailment,
circumscription, or answer-set/default logic) over the normality ordering P1
provides; the strict core stays in the monotonic FOL export.

**P5 — One mereology (Link lattice).** Plurals (`Group`, `Distributive`,
`GroupQuantifier`), cumulative readings (§5.2), and mass (§6.2) share one
**join-semilattice** with a part-of relation: count = atomic, mass = non-atomic,
plural = sums of atoms. Cumulativity, distributivity, and collectivity are
operators over this lattice; their axioms are defined once.

**P6 — Branching quantification uses Skolem functions.** Cumulative readings are
first-order over the Link lattice (§5.2). Branching (Hintikka) quantifiers are
emitted in **Skolemized** form with explicit Skolem functions (`Term::Function`).

**P7 — Underspecified scope is canonical.** Scope is stored as one
**underspecified** form with dominance constraints (MRS / Hole / Glue); islands
are declarative dominance constraints; readings are enumerated on demand. The AST
hosts scope holes; enumeration is a consumer.

**P8 — Presupposition = anaphora (Van der Sandt) in the DRS.** A presupposition
**binds** to an accessible antecedent if one exists, else **accommodates** at the
highest accessible box. Projection, filtering, and the proviso problem follow
from the existing `drs.rs` accessibility relation.

**P9 — Degrees are scales.** Degree predicates (§7, §8.5, gradable modals) share
a typed **scale** (intervals over an ordering), reasoned via LIA/reals. The
vagueness threshold θ is a context constant (contextualist semantics).

---

## 1. Clause types (illocutionary force)

### 1.1 Exclamatives ✗
**What it is.** Clauses expressing affective stance toward a *degree*, marked by
`how`/`what` without subject-aux inversion: "How tall she is!", "What a fool he
is!". They presuppose the proposition and assert that a degree is surprisingly
high.
**What's missing.** `how`/`what` route to wh-question handling; no exclamative
force node, no degree-assertion semantics.
**Enables.**
- "How tall she is!" → assert ∃d(Tall(she,d) ∧ d ≫ θ); presupposes Tall(she).
- "What a beautiful painting that is!"

**Implementation.**
- **Node:** `Exclamative{degree_var, body}`.
- **Parse:** `clause.rs` — `how`/`what (a)` + AdjP/NP with no subject-aux inversion.
- **Encode:** `Exclaim(∃d(Tall(she,d) ∧ d ≫ θ)) ∧ ⟨Tall(she)⟩` (presupposition via P8).
- **Reason:** degree scale (P9); presupposition via DRS (P8).
- **Render:** `exclamative()` → `!…!` / `\mathsf{Excl}` / `Exclaim(...)`.
- **Frontend:** `expr_to_ast_node` arm; exclamative demo.
- **Test:** asserts `d ≫ θ` plus the presupposition; resolves separately from wh-questions.

### 1.2 Optatives ✗
**What it is.** Wish clauses with no asserted truth: "May you prosper!", "If only
it were Friday!", "Long live the king!".
**What's missing.** No parse for `may`-fronting / `if only` / `long live` as a
wish operator; `may` reads only as a modal.
**Enables.**
- "May you prosper!" → Wish(speaker, ⟨Prosper(you)⟩).
- "If only I had known."

**Implementation.**
- **Node:** `Optative{wish}`.
- **Parse:** `clause.rs`/`modal.rs` — `may`-fronting, `if only`, `long live`.
- **Encode:** `Wish(speaker, ⟨Prosper(you)⟩)`.
- **Reason (P1):** bouletic ordering source g (speaker preferences) over the
  doxastic modal base; quantify over the g-best worlds; complement not entailed.
- **Render:** `optative()` → `♢_wish` / `\mathsf{Wish}` / `Wish(...)`.
- **Frontend:** demo + AST arm.
- **Test:** complement not entailed; wish operator present.

### 1.3 Performatives / speech acts ✗
**What it is.** First-person present utterances whose saying is the doing: "I
promise to call", "I now pronounce you married".
**What's missing.** `SpeechAct` exists but the parser never produces it; the
`Performative` lexicon feature is unconsumed.
**Enables.**
- "I promise to call you." → SpeechAct(promise, speaker, ⟨Call(speaker,hearer)⟩).
- "I hereby resign."

**Implementation.**
- **Node:** `SpeechAct{performer,act_type,content}`.
- **Parse:** `pragmatics.rs` — 1sg present + the `Performative` lexicon feature.
- **Encode:** `SpeechAct(promise, speaker, ⟨Call(speaker,hearer)⟩)`; content is a structured proposition (P3).
- **Reason:** context-update — the act enters the discourse commitment state (DRS); axiom `SpeechAct(a,x,p) → Done(a)` at the utterance world.
- **Render:** node renders (transpile.rs:635); complete Simple + Kripke arms across all 5 formatters.
- **Frontend:** performative demo; AST arm.
- **Test:** "I promise…" entails the promising act.

### 1.4 Imperatives ◑
**What it is.** Commands with covert 2nd-person subject: "Close the door!",
"Don't move.", "Let's go."
**What's missing.** `Imperative` is produced but with no addressee agent, no
theme roles, no directive operator; `don't` and `let's` undistinguished.
**Enables.**
- "Close the door." → Directive(hearer, ⟨∃e(Close(e)∧Agent(e,hearer)∧Theme(e,door))⟩).
- "Don't touch that." · "Let's leave."

**Implementation.**
- **Node:** `Imperative{action}` with `action` a full `NeoEvent`, `Agent = hearer`.
- **Parse:** `clause.rs` — bare-verb-initial; `don't` ⇒ negation, `let's` ⇒ hortative flag.
- **Encode:** `Directive(hearer, ⟨∃e(Close(e)∧Agent(e,hearer)∧Theme(e,door))⟩)`.
- **Reason (P1):** the directive updates the addressee's To-Do list — a bouletic/deontic ordering source g over the addressee's action worlds; `Directive(h,p) → O_g p`.
- **Render:** ensure hearer-agent prints; add a hortative glyph.
- **Frontend:** imperative demo; AST arm present.
- **Test:** addressee agent + theme roles present; `don't` flips polarity.

---

## 2. Coordination

### 2.1 Non-constituent / bare-argument coordination ✗
**What it is.** Strings sharing a verb across differing roles: "John gave Mary a
book and Sue a pen".
**What's missing.** Coordination cannot distribute one verb over two role-bundles
with distinct arguments.
**Enables.**
- "John gave Mary a book and Sue a pen." → two Give events sharing Agent john.

**Implementation.**
- **Node:** two `NeoEvent`s under `BinaryOp(And)`.
- **Parse:** `clause.rs` — detect a shared-verb gap with two role bundles; clone verb/agent into a second event.
- **Encode:** `∃e1(Give(e1)∧Ag=john∧Rec=mary∧Th=book) ∧ ∃e2(Give(e2)∧Ag=john∧Rec=sue∧Th=pen)`.
- **Test:** both events emitted with shared agent.

### 2.2 Non-parallel / mixed-category coordination ✗
**What it is.** Conjuncts of unlike category: "She is a doctor and proud of it".
**What's missing.** Coordination assumes parallel categories.
**Enables.**
- "He is wealthy and a philanthropist."

**Implementation.**
- **Node:** type-lift each conjunct to a predicate `λx.P(x)` over the shared subject, then `And`.
- **Parse:** `clause.rs` — permit NP/AdjP/clause conjuncts.
- **Encode:** `Wealthy(john) ∧ Philanthropist(john)`.
- **Test:** mixed AdjP+NP yields a conjunction.

### 2.3 Correlative coordination ✗
**What it is.** Paired connectives: both…and, either…or, neither…nor, not only…but also.
**What's missing.** The lead correlative is not parsed as a scope marker.
**Enables.**
- "Neither John nor Mary came." → ¬Came(john) ∧ ¬Came(mary).
- "Either it rains or it snows." (exclusive reading available)

**Implementation.**
- **Node:** `BinaryOp` with an explicit scope marker; `neither…nor` → `¬∧¬`, `either…or` → exclusive option.
- **Parse:** `TokenType::Both/Either/Neither/NotOnly` as scope-openers in `clause.rs`.
- **Encode:** `¬Came(john) ∧ ¬Came(mary)`; `(p∨q)∧¬(p∧q)` for exclusive `either…or`.
- **Frontend:** `categorize_token` → Connective color.
- **Test:** `neither…nor` double negation; exclusive `either…or`.

### 2.4 Comparative subdeletion ✗
**What it is.** Comparison across two gradable predicates: "The shelf is wider than the door is tall".
**What's missing.** No clausal `than`-complement with its own degree gap.
**Enables.**
- "The desk is longer than the door is wide." → max{d:Long(desk,d)} > max{d':Wide(door,d')}.

**Implementation.**
- **Node:** `Comparative` object accepts a degree term from a clausal `than`.
- **Parse:** `pragmatics.rs`/`quantifier.rs` — parse the clausal complement and its dimension.
- **Encode:** `max{d:Long(desk,d)} > max{d':Wide(door,d')}`.
- **Reason (P9):** LIA over the two degree maxima.
- **Render:** extend `write_comparative` for the two-dimension form.
- **Test:** emits two `max` degree terms.

---

## 3. Subordination & embedding

### 3.1 Non-restrictive (appositive) relative clauses ✗
**What it is.** Comma-set RCs that add information without restricting: "John, who loves Mary, left".
**What's missing.** All RCs are treated as restrictive.
**Enables.**
- "John, who loves Mary, left." → Left(john) ∧ Love(john, mary).

**Implementation.**
- **Node:** a conjoined assertion (not an intersective restrictor).
- **Parse:** `quantifier.rs` — comma-delimited RC ⇒ a side-assertion on the head referent; `drs.rs` adds the condition at the main box.
- **Encode:** `Left(john) ∧ Love(john,mary)`.
- **Test:** appositive contributes a top-level conjunct; restrictive RCs unchanged.

### 3.2 Perception complements ✗
**What it is.** Bare-VP vs gerund complements of perception verbs: "John saw the man run" vs "…running".
**What's missing.** No small-clause/bare-infinitive complement; the aspectual contrast is unrepresented.
**Enables.**
- "Mary heard the bell ring." → ∃e(Hear(e)∧Ag=mary∧Th=⟨∃e'(Ring(e')∧complete)⟩).
- "I saw him crossing the street." (progressive complement)

**Implementation.**
- **Node:** `NeoEvent` with `Theme = Term::Proposition(embedded event)`.
- **Parse:** `verb.rs` — perception verb + NP + bare-VP/gerund small clause.
- **Encode:** `∃e(Hear(e)∧Ag=mary∧Th=⟨∃e'(Ring(e')∧complete)⟩)`; gerund ⇒ `Aspectual::Progressive` on the inner event.
- **Test:** bare-VP = complete vs gerund = `Prog` inner aspect.

### 3.3 Secondary predication ✗
**What it is.** A predicate over an argument alongside the verb — resultative "painted the door red", depictive "ate the meat raw".
**What's missing.** The extra AP/NP predicate is dropped or mis-attached.
**Enables.**
- "John painted the door red." → ∃e(Paint(e)∧Agent(e,john)∧Theme(e,door)∧Result(e,Red(door))).
- "He drinks his coffee black."

**Implementation.**
- **Node:** `NeoEvent`; add `ThematicRole::Result` and `ThematicRole::Depictive` (logic.rs:205).
- **Parse:** `verb.rs` — post-object AP/NP.
- **Encode:** resultative `…∧ Result(e, Red(door))`; depictive `…∧ Depictive(e, Raw(meat))`.
- **Render:** roles print via existing role machinery.
- **Test:** "painted the door red" emits `Result(Red(door))`.

### 3.4 Causal / concessive adverbial clauses ✗
**What it is.** Subordinators encoding logical relations: because/since (cause), although/though (concession), so that (purpose), unless, while.
**What's missing.** Only temporal `Precedes` is produced; causal/concessive/purpose links are absent.
**Enables.**
- "John stayed because it rained." → Stay(john) ∧ Cause(Rain, Stay(john)).
- "Although she was tired, she finished." → Finish(she) ∧ Concessive(Tired(she)).

**Implementation.**
- **Node:** `Causal{effect,cause}`; add `Concessive{main,concession}`.
- **Parse:** `clause.rs` subordinators because/since/so that/although/unless/while.
- **Encode:** `Stay(john) ∧ Cause(Rain, Stay(john))`; `Finish(she) ∧ Concessive(Tired(she))`.
- **Reason:** `Cause(a,b) → (a ∧ b)`; concessive presupposes a defeated expectation `a → ¬b`.
- **Render:** add `concessive()` to the trait + all 5 impls.
- **Test:** because→Cause; although→Concessive.

### 3.5 Belief reports / opaque complements ◑
**What it is.** Attitude verbs creating intensional contexts where substitution fails and de re/de dicto split: "John believes Mary left", "John seeks a unicorn".
**What's missing.** That-complements get extensional treatment; the `Opaque`/`IntensionalPredicate` features are unused; the de re/de dicto ambiguity is ungenerated.
**Enables.**
- "John believes Mary left." → Believe(john, ⟨Left(mary)⟩); substitution blocked.
- "John is seeking a unicorn." → de dicto Seek(john, ^λx.Unicorn(x)) vs de re ∃x(Unicorn(x) ∧ Seek(john,x)).

**Implementation.**
- **Node:** `Intensional{operator,content}`; the attitude object is the structured proposition `Term::Proposition(&LogicExpr)` (P3).
- **Parse:** `verb.rs` — attitude verbs (lexicon `Opaque`/`IntensionalPredicate`) ⇒ wrap the complement as `⟨φ⟩`; intensional predicates (seek) ⇒ `^`.
- **Encode:** `Believe(john, ⟨Left(mary)⟩)`; de re/de dicto for `seek` via `enumerate_intensional_readings` (lambda.rs:989) ⇒ parse forest.
- **Reason (P3):** the structured complement keeps co-intensional complements distinct; substitution is blocked by `lambda.rs::substitute_respecting_opacity`.
- **Render:** `Operator[content]` (transpile.rs:547); complete Simple/Kripke arms.
- **Frontend:** both readings surface via `compile_forest`.
- **Test:** `Hesperus=Phosphorus` substitution fails; two readings for "seeks a unicorn".

---

## 4. Conditionals & modality

### 4.1 Inverted conditionals ✗
**What it is.** Subject-aux inversion replacing `if`: "Had I known, I would have left".
**What's missing.** Only overt `if…then` parses.
**Enables.**
- "Had I known, I would have left." → Know(I) □→ Leave(I).

**Implementation.**
- **Node:** `Counterfactual`.
- **Parse:** `clause.rs::parse_conditional` — aux-inversion (`Had/Were/Should` + subject) is a fronted antecedent (extend `is_counterfactual_context`).
- **Encode/Reason/Render/Test:** §4.5 path (P1+P2).

### 4.2 Biscuit / relevance conditionals ✗
**What it is.** Conditionals whose consequent holds regardless of the antecedent: "If you're hungry, there's pizza in the fridge".
**What's missing.** Parsed as material/strict conditionals, giving the wrong truth conditions.
**Enables.**
- "If you need me, I'm in my office." → asserts In(me, office); antecedent restricts relevance.

**Implementation.**
- **Node (P2):** the if-clause restricts the **speech act**; the consequent is asserted outright. Carry a `relevance` restrictor at the illocutionary level.
- **Parse:** `clause.rs` — detect a speaker-anchored standing-state consequent.
- **Encode:** assert `In(me,office)` + `Relevance(antecedent)` at the speech-act layer.
- **Reason (P2):** indicative `if` is a restrictor on the consequent's (covert epistemic) modal; biscuit is the speech-act-level case.
- **Test:** consequent entailed regardless of antecedent.

### 4.3 Evidentials / perspectival predicates ✗
**What it is.** seem/appear/look as raising verbs marking evidence source: "John seems happy".
**What's missing.** Treated as ordinary verbs; the `Raising` feature is unused.
**Enables.**
- "John seems happy." → Seem(⟨Happy(john)⟩) — complement not asserted.

**Implementation.**
- **Node:** `Modal` with `ModalFlavor::Evidential`.
- **Parse:** `verb.rs`/`modal.rs` — raising verbs (lexicon `Raising`) ⇒ Modal wrap, subject raised.
- **Encode:** `Seem(⟨Happy(john)⟩)`.
- **Reason (P1):** evidential modal base (available evidence) with a non-reflexive frame ⇒ complement not entailed.
- **Render:** `modal()` dispatches on f,g; add the evidential symbol.
- **Test:** "seems happy" does not entail "happy".

### 4.4 Habitual / generic modality ✗
**What it is.** Characterizing statements with adverbs of quantification: "Dogs usually bark", "John smokes".
**What's missing.** `usually/always/often` read as scopal adverbs; no GEN operator.
**Enables.**
- "John smokes." → GEN e[appropriate(e)](Smoke(e)∧Ag=john).

**Implementation.**
- **Node:** `Aspectual::Habitual` and/or `QuantifierKind::Generic`.
- **Parse:** `modal.rs`/`quantifier.rs` — `usually/always/often` ⇒ Habitual; bare-plural subject ⇒ Generic (§6.1).
- **Encode:** `GEN e[appropriate(e)](Smoke(e)∧Ag=john)`.
- **Reason (P1+P4):** GEN over the normality ordering source; reasoned in the non-monotonic layer.
- **Render:** `HAB`/`Gen` exist (formatter.rs:33,268).
- **Test:** habitual/generic operator present, not a bare ∃.

### 4.5 Counterfactual conditionals ◑
**What it is.** Subjunctive conditionals over closest worlds: "If John had studied, he would have passed".
**What's missing.** Compiled to a simplified material-style form; no closest-world semantics.
**Enables.**
- "If John had studied, he would have passed." → Study(john) □→ Pass(john).

**Implementation.**
- **Node:** `Counterfactual` (renders `□→`, formatter.rs:276).
- **Parse:** `clause.rs:394`.
- **Encode:** `Study(john) □→ Pass(john)`.
- **Reason (P1+P2):** `would` is a necessity modal whose ordering source g = similarity and whose modal base is restricted by the if-clause; closest worlds fall out of g. Z3 reasons over the FOL-axiomatizable fragment of the similarity ordering.
- **Render:** `□→` prints in all formatters; Kripke lowering emits the g-restricted form.
- **Test:** counterfactual ≠ material implication truth table.

---

## 5. Quantification & scope

### 5.1 Scope inside islands ✗
**What it is.** Quantifier interactions in scope islands: "Everyone who owns a car insures it".
**What's missing.** Island-internal quantifiers get only surface scope.
**Enables.**
- "Everyone who owns a car insures it." → the wide-`a car` reading available.

**Implementation.**
- **Node (P7):** scope is the underspecified form with dominance constraints; islands are declarative constraints.
- **Parse/Work:** `lambda.rs` — licensed island-internal readings enumerated from the underspecified form.
- **Frontend:** readings surface via `compile_forest`.
- **Test:** the wide-`a car` reading is produced.

### 5.2 Cumulative / branching quantification ✗
**What it is.** Readings irreducible to nesting: "Three boys ate five pizzas" (cumulative); Hintikka branching.
**What's missing.** Only linear nesting is produced.
**Enables.**
- "Three boys lifted five boxes." → cumulative |boys|=3, |boxes|=5.

**Implementation.**
- **Node (P5):** `GroupQuantifier` over the Link lattice; cumulative reference is `R(⊕A, ⊕B)` over plural sums.
- **Work (P6):** cumulative is first-order over the lattice; branching is emitted in Skolemized form with explicit Skolem functions (`Term::Function`).
- **Render:** `GroupQuantifier` (transpile.rs:744) + cumulative shape; Skolem functions print as `Term::Function`.
- **Test:** "three boys lifted five boxes" cumulative; a branching sentence emits Skolem functions.

### 5.3 Proportional / partitive quantifiers ✗
**What it is.** Quantifiers over a presupposed set: "two of the three students", "most of the water".
**What's missing.** `of the` not parsed as restricting to a salient superset; proportional `most/half` lack cardinality semantics.
**Enables.**
- "Two of the three students passed." → |{x:Student(x)∧Pass(x)}|=2 within the 3-set.

**Implementation.**
- **Node:** `QuantifierKind::Most/Few/Cardinal`; partitive restricts to a salient set.
- **Parse:** `quantifier.rs` — `of the` ⇒ restrictor is a contextual definite set (a DRS referent).
- **Encode:** `|{x:Student(x)∧Pass(x)}|=2` within the 3-set; `most` → >50%.
- **Reason (P9):** cardinality via LIA on set sizes.
- **Render:** `MOST/FEW/∃=n` exist.
- **Test:** "two of the three" — cardinality + superset restriction.

### 5.4 Floating quantifiers ✗
**What it is.** Stranded `all/each/both`: "The boys all left", "They each received a prize".
**What's missing.** The stranded quantifier is not re-associated to the subject NP.
**Enables.**
- "The students each solved a problem." → ∀x∈students ∃problem, distributive.

**Implementation.**
- **Node:** re-associate the stranded quantifier to the subject NP; `each` ⇒ `Distributive`.
- **Parse:** `quantifier.rs`/`clause.rs` — post-subject quantifier binds the subject.
- **Encode:** `∀x∈students ∃problem …`.
- **Test:** "the students each solved a problem" distributes.

---

## 6. Nominal semantics

### 6.1 Generics / kind reference ✗
**What it is.** Statements about kinds: "Dogs are animals", "The dodo is extinct".
**What's missing.** Bare plurals/kind-definites get existential or plain universal quantification; no kind term or GEN operator.
**Enables.**
- "Dogs are animals." → GEN x[Dog(x)]Animal(x).
- "The dodo is extinct." → Extinct(^dodo) (kind, no individual).

**Implementation.**
- **Node:** `QuantifierKind::Generic`; add `Term::Kind(Symbol)` for kind-denoting NPs.
- **Parse:** `quantifier.rs` — bare plural / kind-definite ⇒ Generic or a Kind term.
- **Encode:** `GEN x[Dog(x)]Animal(x)`; "the dodo is extinct" → `Extinct(^dodo)`.
- **Reason (P1+P4):** GEN over the normality ordering source, reasoned in the non-monotonic layer; strict subsumption `Dog ⊑ Animal` (lexicon hypernyms) stays in the FOL core.
- **Render:** `Gen` exists; kind term renders as `^Kind`.
- **Test:** GEN present; a penguin counter-instance does not falsify `GEN x[Bird(x)]Fly(x)`; kind predication has no ∃ individual.

### 6.2 Mass vs. count semantics ◑
**What it is.** Mass nouns denote cumulative, non-atomic stuff; count nouns have atoms.
**What's missing.** Mass is a lexical feature but semantically inert; "Water is transparent" and a count reading compile identically.
**Enables.**
- "Water is wet." → cumulative kind predication.
- "John drank a water." → Portion(p)∧Water(p) (coercion).

**Implementation.**
- **Node (P5):** mass and plural live in the same Link join-semilattice — mass = non-atomic part, count = atomic, plural = sums.
- **Parse:** `quantifier.rs` — mass noun ⇒ no atom/∃-individual; `much` allowed; `a/two` coerces to an atom-introducing portion.
- **Encode:** "water is wet" → cumulative predication over the lattice; "a water" → `Portion(p)∧Water(p)`.
- **Reason (P5):** cumulativity/divisiveness are properties of the lattice `⊕`/part-of relation (`Water(a)∧Water(b) → Water(a⊕b)`), shared with plurals and §5.2.
- **Test:** mass vs count compile differently; coercion on count use; cumulativity holds.

---

## 7. Degree

### 7.1 Equatives (as…as) ✗
**What it is.** Equality/at-least degree comparison: "John is as tall as Mary".
**What's missing.** The `as…as` frame isn't parsed.
**Enables.**
- "John is as tall as Mary." → max{d:Tall(john,d)} ≥ max{d:Tall(mary,d)}.

**Implementation.**
- **Node:** `Comparative` with a `relation: GT|GE|EQ` field.
- **Parse:** `pragmatics.rs` — the `as ADJ as` frame.
- **Encode:** `max{d:Tall(john,d)} ≥ max{d:Tall(mary,d)}`.
- **Reason (P9):** LIA on degree maxima.
- **Render:** `write_comparative` emits the relation symbol (≥).
- **Test:** equative emits ≥, not >.

### 7.2 Implicit comparison class / standard ✗
**What it is.** Bare gradable predication relies on a contextual standard: "John is tall".
**What's missing.** "John is tall" yields `Tall(john)` with no degree or standard.
**Enables.**
- "John is tall." → ∃d(Tall(john,d) ∧ d > θ_C).

**Implementation.**
- **Node:** `Term::Degree(value)`; bare gradables become `∃d(Adj(x,d) ∧ d > θ_C)`.
- **Parse:** `quantifier.rs`/adjective path — bare gradable ⇒ degree var + context standard `θ_C`; "for a jockey" sets C.
- **Encode:** `∃d(Tall(john,d) ∧ d > θ_C)`.
- **Reason (P9):** θ_C is a context constant (Skolem constant); LIA comparisons.
- **Render:** degree-term + `θ` across all formatters.
- **Test:** bare "John is tall" gains a degree var + standard.

---

## 8. Discourse & pragmatics

### 8.1 Binding theory ✗
**What it is.** Structural constraints on anaphora — Principle A (reflexives bound locally), B (pronouns free locally), C (R-expressions free).
**What's missing.** Binding constraints aren't enforced; coreference can be assigned where grammar forbids it.
**Enables.**
- "John saw himself." → See(john, john).
- "John saw him." → See(john, y), y ≠ john.

**Implementation.**
- **Node:** constraints in `drs.rs::resolve_pronoun`.
- **Work:** Principles A/B/C over the existing accessibility — reflexives bind in the local box; pronouns excluded from a local antecedent; R-expressions free.
- **Encode:** "John saw himself" → `See(john,john)`; "John saw him" → `See(john,y), y≠john`.
- **Test:** forced coref for the reflexive; exclusion for the pronoun.

### 8.2 Presupposition projection ✗
**What it is.** How presuppositions survive embedding operators: "John didn't stop smoking" still presupposes he smoked.
**What's missing.** Triggers are stored but presuppositions aren't projected through `¬/Modal/Quantifier/If`; failures aren't detected.
**Enables.**
- "Mary doesn't regret lying." → presupposes Lied(mary), which projects.
- "The king of France is bald." → presupposition-failure flag.

**Implementation.**
- **Node:** `Presupposition{assertion,presupposition}` (logic.rs:627) + DRS.
- **Work (P8):** Van der Sandt in `drs.rs` — a presupposition binds to an accessible antecedent if one exists, else accommodates at the highest accessible box; projection, filtering, and the proviso problem follow from the accessibility relation.
- **Encode:** "doesn't regret lying" → assert `¬Regret…`; `Lied(mary)` accommodates above the ¬ box and projects.
- **Reason:** failed accommodation (no consistent host) ⇒ a failure flag; accommodated presuppositions enter the Z3 context.
- **Render:** `[Presup: …]` (transpile.rs:718) at its host box.
- **Test:** projects through negation; the binding case ("If John has children, his children…") does not project; "king of France" failure flag.

### 8.3 Clefts & pseudo-clefts ✗
**What it is.** Focus-marking with exhaustivity: "It was John who left", "What John lost was his keys".
**What's missing.** Parsed as plain predication; focus/background split and exhaustivity lost.
**Enables.**
- "It was John who broke the vase." → Break(john,vase) ∧ ∃!x Break(x,vase) ∧ exhaustive(john).

**Implementation.**
- **Node:** `Focus` with `FocusKind::Cleft`.
- **Parse:** `pragmatics.rs` — `it was X who…` / `what … was Y`.
- **Encode:** `Break(john,vase) ∧ ∃!x Break(x,vase) ∧ exhaustive(john)`.
- **Reason:** exhaustivity `∀x(Break(x,vase) → x=john)` (Z3-checkable).
- **Render:** `Focus` (transpile.rs:725) + cleft symbol.
- **Test:** cleft adds exhaustivity + existence presupposition.

### 8.4 Deixis / indexicals ✗
**What it is.** Context-dependent reference: I, you, here, now, today, this.
**What's missing.** Indexicals are parsed literally with no context anchor.
**Enables.**
- "I will meet you here tomorrow." → speaker/addressee/place/day+1 anchors.

**Implementation.**
- **Node:** `Term::Indexical(kind)` (I/you/here/now/today) resolved against a context record.
- **Work:** `compile.rs`/`drs.rs` — an utterance-context record; character (context→content) resolution, kept symbolic across reported speech.
- **Encode:** "I will meet you here tomorrow" → resolved speaker/addressee/place/day+1.
- **Render:** resolved constants or `@speaker` tokens across formatters.
- **Frontend:** optional context inputs in the studio; default speaker/hearer.
- **Test:** indexical resolves to a context constant.

### 8.5 Vagueness ✗
**What it is.** Predicates with borderline cases and sorites behavior: bald, tall, heap.
**What's missing.** Treated as crisp; no threshold or penumbra.
**Enables.**
- "John is bald." with borderline tolerance.

**Implementation.**
- **Node (P9):** degree machinery (§7.2) + a `Borderline` predicate.
- **Work:** vague predicate ⇒ `∃d(Bald(x,d) ∧ d > θ)`; penumbra `θ_low < d < θ_high`.
- **Reason (P9):** θ is a context constant; sorites step flagged.
- **Test:** emits a threshold; sorites step flagged.

### 8.6 Metonymy ✗
**What it is.** Reference by association: "Washington announced…", "We read Shakespeare".
**What's missing.** No coercion from the literal sort to the intended referent.
**Enables.**
- "The White House said no." → Say(government-of(white_house), ¬…).

**Implementation.**
- **Node:** `Term::Coercion{literal, target_sort}`.
- **Work:** `ontology.rs`/sort checker — on a sort clash, coerce via a relation (`government-of`, `works-of`).
- **Encode:** "White House said no" → `Say(government-of(white_house), ¬…)`.
- **Reason:** coercion functions are axioms; sort-checking preserved.
- **Test:** metonym coerces, no sort error.

### 8.7 Conversational implicature ✗
**What it is.** Meaning via Gricean reasoning: scalar "some" → "not all"; indirect speech acts.
**What's missing.** Only literal truth conditions; no scalar strengthening, no indirect speech acts.
**Enables.**
- "Some students passed." → +implicature ¬(all passed).

**Implementation.**
- **Node:** an exhaustification layer over the alternative sets carried by `Focus`.
- **Work (P4):** an **`exh` operator** (grammatical exhaustification) over a focus/Horn-alternative set computed from the lexicon scale: `exh(φ) = φ ∧ ⋀{¬ψ : ψ ∈ Alt(φ), ψ stronger}`; scales to every scalar item, embedded and free-choice cases. Implicatures are defeasible and live in the non-monotonic layer.
- **Encode:** "some students passed" → assert `∃`; `exh` adds defeasible `¬(all passed)`.
- **Frontend:** show the implicature as a separate labeled line in `logic_output.rs`.
- **Test:** implicature derived from alternatives; cancellable; base meaning unchanged; an embedded case works.

---

## 9. Adjective classes

The lexicon distinguishes intersective / non-intersective / subsective / gradable
/ event-modifier and handles privative adjectives via axioms; degree adjectives
are §7. The class with no `Feature` variant is the relational/pertainymic class.

### 9.1 Relational / pertainymic adjectives ✗
**What it is.** Denominal, non-predicating adjectives denoting a relation to a
base noun: coastal (coast), dental (tooth), nuclear (nucleus), marine (sea),
postal (post), presidential (president), solar (sun). "a dental procedure" is not
{dental things} ∩ {procedures}; the adjective relates the procedure to teeth.
**What's missing.** The adjective `Feature` set has no `Relational` class, so
these are mis-tagged (as Subsective or Intersective) and lose the base-noun link.
A separate pre-existing bug drops adjectives under indefinite/copular subjects
("A red car is fast." loses `Red`), while universal subjects keep them.
**Enables.**
- "Every dental procedure is expensive." → ∀x((Procedure(x) ∧ Pertains(x, ^Tooth)) → Expensive(x)).
- "Every coastal region is wet." → ∀x((Region(x) ∧ ∃y(Coast(y) ∧ Near(x,y))) → Wet(x)).
- nuclear reactor → `Pertains(x, ^Nucleus)`; marine animal → `Pertains(x, ^Sea)`; postal worker → `Pertains(x, ^Post)`.

**Implementation.**
- **Node:** `Noun(x) ∧ Rel(x, ^Base)` (kind, default) or `Noun(x) ∧ ∃y(Base(y) ∧ Rel(x,y))` (instance, override). The base noun is a **kind term** by default; relational adjectives are predicates of kinds (McNally & Boleda).
- **Lexicon:** `Feature::Relational` in `crates/logicaffeine_lexicon/src/types.rs` + a `relational{ base, relation, level }` substruct (`lookup_relational_adjective` in `build.rs`, mirroring the noun-derivation pattern). `relation` defaults to `Pertains`; `level` defaults to `Kind`. Per-adjective overrides: `coastal → { base: Coast, relation: Near, level: Instance }`, `dental → { base: Tooth }`. WordNet pertainyms supply `base`.
- **Parse:** add a `Relational` branch to a single shared `adjective_restriction(adj, var, noun)` helper used by the adjective-restriction sites in `parser/quantifier.rs` (indefinite ~1494/1505, definite ~1584, universal `parse_restriction`); this shared helper also fixes the indefinite-adjective-drop bug.
- **Encode:** "dental procedure" → `Procedure(x) ∧ Pertains(x, ^Tooth)`; "coastal region" → `Region(x) ∧ ∃y(Coast(y) ∧ Near(x,y))`.
- **Reason:** `Pertains` is axiomatizable — `Pertains(x, ^Coast) → ∃y(Coast(y) ∧ Near(x,y))` where wanted. The `relational{ base, relation, level }` mechanism covers kind-relational (default), instance-relational, named-relation, and flat-predicate as cases.
- **Render:** predicate + kind term `^Base` / existential render through existing machinery; `Pertains` is an ordinary predicate.
- **Frontend:** relational-adjective demo in `examples.rs`.
- **Test:** a kind-level adjective ("dental") introduces no `∃y`; the instance override ("coastal") introduces one. Regression guards stay UNCHANGED — "Every red car is fast." keeps `Red(x)` 1-arg; "Every large mouse is quiet." keeps subsective `Large(x, ^Mouse)`. Sprint detail in `wikis/RELATIONAL_ADJECTIVES_PLAN.md`.

---

## Sequencing (RED-first, cheapest correctness first)

1. **Wire-only (nodes exist):** §3.5 belief reports, §4.4 habitual, §4.5
   counterfactual, §1.3 performatives, §1.4 imperatives, §5.3
   proportional/partitive, §9.1 relational adjectives — mostly parser + axioms.
2. **Reasoning upgrades:** §8.2 presupposition (Van der Sandt), §8.1 binding,
   §6.2 mass (Link lattice), §5.1/§5.2/§5.4 scope — DRS / scope / mereology.
3. **New small nodes:** §7.1 equatives, §8.3 clefts, §3.3 secondary predication,
   §3.4 concessive, §4.3 evidentials.
4. **New frontier nodes:** §1.1 exclamative, §1.2 optative, §7.2/§8.5
   degree + vagueness, §8.4 deixis, §8.6 metonymy, §8.7 implicature.

Coordination (§2.x) and conditional (§4.1/§4.2) parser work interleaves with the
clause-parser extension. The cross-cutting machinery — Kratzer f,g (P1), the
non-monotonic layer (P4), the Link lattice (P5), underspecified scope (P7) — is
built once before the entries that depend on it.

## Verification

- Per phenomenon: a `phaseNN_*.rs` test asserting the target logic in each format
  (Unicode/SimpleFOL/Kripke) + a `--features verification` test that the intended
  entailment holds (or fails — e.g. evidentials must not entail).
- Frame-axiom work (§4.5/§4.3/§1.4) is verified by Z3 entailment tests over the
  modal base / ordering source (T/S4/S5/D).
- Defeasible work (§6.1/§4.4/§8.7) is verified in the non-monotonic layer:
  a counter-instance cancels the default without contradiction.
- Frontend: a `compile_for_ui` round-trip shows each construct in all toggles;
  add a demo per phenomenon in `examples.rs`.
- Full suite green via the CLAUDE.md command before and after each phase.

## How to use this document

Each entry is a RED-test-first work item (CLAUDE.md): write a phase test
asserting the target logic, then implement parser/semantics, then run the full
suite. Build along the Integration Spine, reuse the node named in the Reuse Map,
apply the SOTA principles (P1–P9), and never edit a RED test to make it pass.
