/-!
@case      T12-011
@tier      12 · Vernacular surfaces
@title     Surface selection policy — which of the four targets a given Lean theorem goes to
@lean      the whole of it: this case is the synthesis for tier 12 and the routing rule
           every other case in the tier is a boundary condition of
@expect    degrade
@target    A per-theorem ROUTING DECISION, recorded in the translator's output alongside
           the emitted artefact, chosen by the procedure below. There are FOUR targets,
           not three:

             K. raw kernel      — `logicaffeine_kernel::Term` + `Context::add_definition`
             A. English theorem — `## Theorem:` / `Given:` / `Prove:` / `Proof:`
             B. formal FOL      — `## Theory` / `Axiom` / `Theorem … cites … : … prove …`
             C. literate kernel — `## Theorem:` / `Statement:` / `Proof:`

           THE ANSWER, stated plainly: **for almost any real Lean theorem the target is
           K.** Emit a kernel `Term` for the proposition, emit a kernel `Term` for the
           proof, `infer_type` the latter, and `add_definition` the pair. The three
           English surfaces are entry points for HUMAN-AUTHORED content and are not
           translation targets; each is reachable only by a Lean theorem that has
           already been narrowed to a fragment far smaller than Lean itself.

           The decision procedure, applied to the theorem's ELABORATED type:

             0. Contains `sorryAx`?                                    ⟶ reject (T06-017)
             1. Type mentions a `Sort`/universe, a dependent `Pi` whose
                body uses the binder at TYPE level, a `Lambda`, a `Match`,
                a `Fix`, a structure projection, an inductive family, or
                any typeclass argument?                                ⟶ K
             2. Strip the prenex quantifier block. In the remaining
                propositional skeleton, is every argument of every atom a
                bare variable or constant (no function application)?
                  · NO  ⟶ A and B are impossible (T12-002, T12-007); go to 3.
                  · YES ⟶ go to 4.
             3. Is the ORIGINAL quantifier structure a (possibly empty)
                prenex ∀-block and nothing else — no ∃, no nested ∀, no
                quantifier under a connective — AND is every operation in
                the LOGOS `Int` primitive vocabulary
                (`add sub mul div le lt ge gt`, or their infix spellings)?
                  · YES ⟶ C   (T12-008; record QUANTIFIER_ERASURE, T12-009)
                  · NO  ⟶ K   ← this is where most arithmetic lands
             4. Is the theorem one member of a multi-lemma AXIOMATIC
                DEVELOPMENT over one opaque sort, with named axioms and
                lemma-to-lemma citation?
                  · YES ⟶ B   (T12-006)
             5. Is it a single small entailment over UNARY predicates whose
                names are English nouns/adjectives the LOGOS lexicon
                canonicalises, with ≤ ~10 premises and no equality between
                distinct constants?
                  · YES ⟶ A   (T12-001)
             6. Otherwise                                              ⟶ K

@evidence  WHY K IS THE DEFAULT — the vernacular surfaces cannot hold a Lean type.
           · proof/src/lib.rs:199-244 — Surfaces A and B both terminate in `ProofExpr`:
             `Predicate`/`Identity`/`Atom`, the connectives, and `ForAll`/`Exists` whose
             binder is a bare `variable: String` (lib.rs:235-243). There is no type on a
             binder, no sort, no application in the proposition, no abstraction.
           · proof/src/lib.rs:133-150 — `ProofTerm` is `Constant`/`Variable`/`Function`/
             `Group`/`BoundVarRef`. First-order, untyped.
           · kernel/src/term.rs:443-545 — `Term` has `Sort`, `Pi`, `Lambda`, `App`,
             `Match`, `Fix`, `MutualFix`, `Let`, `Const{levels}`. This is the ONLY
             representation in LOGOS that can hold a Lean `Expr`, and it is reachable
             only through K (and, in the reflected `Syntax` encoding, C).
           · kernel/src/context.rs:442 — `add_definition(name, ty, body)`: "a `theorem`
             is just this with a `Prop` type — there is no separate theorem form." A Lean
             theorem is NATIVELY a kernel definition. The vernacular adds nothing to a
             translated theorem that the kernel path does not already have.
@evidence  WHY THE SURFACES ARE FOR HUMANS, not for translators.
           · language/src/compile.rs:1070-1105 — `compile_theorem` begins by LEXING
             ENGLISH: the input goes through `Lexer`, `DiscoveryPass` and the full
             `Parser` before a `Stmt::Theorem` exists. A Lean `Expr` is already
             elaborated; re-encoding it as English prose so an English parser can
             re-derive a weaker structure is lossy by construction.
           · tests/phase63_theorem_parser.rs:26-29 — predicate identity on Surface A
             depends on the NOUN LEXICON: "Phase 64: Noun canonicalization ensures
             'men' → 'Man' so predicates match." A translator on Surface A does not
             control its own identifiers.
           · language/src/compile.rs:1106-1119 — only the FIRST `## Theorem` block in a
             document is proved (`find_map` over the statements). A Lean file with N
             theorems needs N separate Surface A documents, and there is no mechanism for
             the N-th to cite the first. On K, they are N `add_definition` calls into one
             `Context`, and later ones may use earlier ones by name — which is the
             ordinary structure of a Lean file.
           · proof/src/development.rs:20-22 + proof/src/verify.rs:98-130 — Surface B
             DOES have citation (`cites`), but only within a single `## Theory` body, and
             an unknown cited name is silently treated as an external given
             (verify.rs:110-113) rather than reported.
@evidence  WHY THE THREE SURFACES DO NOT COMPOSE — each lacks what another has, so a
           theorem needing two features has no vernacular home:
           · A has ∀/∃ (through English quantifiers) but NO term grammar
             (language/src/parser/mod.rs:6072, :6136 — `parse_sentence`; T12-002).
           · B has ∀/∃ and n-ary relations but terms are single identifiers
             (proof/src/formula.rs:429-435; T12-007).
           · C has real terms — s-expression application, infix `+ - * < > <= >=`, the
             word `equals` (kernel/src/interface/literate_parser.rs:1217-1290) — but NO
             quantifier at all (literate_parser.rs:519-560, :1217-1252; T12-009).
           The intersection "quantifier AND function term" is empty. `∀ n : Nat, n + 0 = n`
           — the first theorem anyone writes — is expressible on NO vernacular surface.
           It is two lines on K.
@evidence  WHY C IS NOT A KERNEL PATH IN DISGUISE — literate `Proof:` tactics wrap the
           statement in a REFLECTED `Syntax` term and produce a `Derivation`, not a proof
           of the statement: literate_parser.rs:625-690 emits `try_<tactic> ⌜stmt⌝`, and
           prelude.rs:2984-2990 / :3020-3026 / :2685-2691 declare every `try_*` as
           `Syntax -> Derivation` via `add_declaration`. The resulting definition
           typechecks for a false statement as readily as a true one; see T12-008.
@loss      SURFACE_PROVENANCE. Routing to K discards the vernacular entirely: the LOGOS
           artefact is a `Term` in a `Context`, with no `## Theorem` block, no English
           rendering, and no `Proof:` line. Nothing about the Lean theorem's tactic
           script survives, and the LOGOS-side document a human would read does not
           exist. This is the price of the correct answer and must be recorded.
@loss      ROUTING_IS_A_CHOICE. The decision is made by the translator, not forced by
           LOGOS, and it is observable: the same Lean theorem routed to A and to K yields
           different LOGOS artefacts with different trust characteristics (T12-013) and,
           on A, possibly a different engine again (T12-014). The routing decision must
           therefore appear in the machine-readable output for every theorem, with the
           rule number that produced it — otherwise a conformance report cannot be
           compared across translator versions.
@guard     Catches the most natural and most wrong architecture for a Lean→LOGOS
           translator: "LOGOS has a theorem vernacular, so translate Lean theorems into
           it." The vernacular is an English front end whose internal representation is
           untyped first-order logic. Every dependent type, every universe, every
           inductive, every recursor, every typeclass — the entire content of tiers 0–7
           of this corpus — is inexpressible in it. A translator built on that premise
           can handle the Socrates syllogism and essentially nothing from a real Lean
           development.
@guard     Catches the opposite over-correction: routing EVERYTHING to K and never
           emitting vernacular. That is defensible, but it must be a recorded decision,
           because the four cases this tier marks `translate` (T12-001, T12-004, T12-006,
           T12-008) are exactly the ones where a vernacular rendering is both faithful
           and better — a `## Theory` development keeps its axiom names and citation
           graph, which K flattens into anonymous `String` keys (context.rs).
@guard     Catches routing on the Lean TACTIC SCRIPT instead of the Lean TYPE. `by omega`
           does not mean "arithmetic, send it to C"; `by decide` does not mean "send it
           to A with `Proof: Auto.`". The routing input is the elaborated type, because
           that is what determines expressibility. The proof script only influences the
           `Proof:` line once a surface has already been chosen (T12-003).
@guard     Catches per-theorem routing without per-FILE consistency. If `theorem b` uses
           `theorem a`, both must be on K, because there is no way for a Surface A or C
           artefact to be cited from a kernel `Term` and no way for a kernel definition to
           be cited from a Surface A block. Routing must be computed on the dependency
           closure, not on one declaration at a time.
@guard     Catches treating rule 3 as "arithmetic ⟶ C". Rule 3 fires only for a prenex-∀
           statement over the `Int` primitive vocabulary. Lean `Nat` subtraction
           truncates and LOGOS `sub` is the opaque `Int` primitive (prelude.rs:352-515),
           so `n - 1 < n` is a DIFFERENT proposition on C than in Lean (T10-017); and
           `Nat` is an inductive in LOGOS while `Int` is axiomatised (T10-004). Most Lean
           arithmetic therefore falls through rule 3 to K.
@oracle    For every theorem the translator emits, its machine-readable output names the
           target (`K`/`A`/`B`/`C`) and the rule number that selected it. A harness
           checks: (1) the routing matches the expected target recorded in this corpus
           for every tier-12 case; (2) the emitted artefact is well-formed for that
           target and passes that target's own oracle; (3) no theorem is routed to A or B
           whose type has a function application in an atom's argument; (4) no theorem is
           routed to C whose type has a non-prenex quantifier; (5) every theorem in a
           dependency cycle or chain shares one target.
@note      INDEX.md marks this case `degrade`, and that is right, but the reason is worth
           stating: the degradation is not in any one translation, it is in the POLICY.
           There is no routing that carries every Lean theorem faithfully into a LOGOS
           artefact that is also human-readable at the surface, because the readable
           surfaces are strictly weaker than the kernel and do not compose with each
           other. Choosing K everywhere is sound and lossy; choosing a vernacular where
           it fits is lossy in a different place. What is NOT acceptable is choosing
           silently.
-/

namespace T12_011

axiom Entity : Type
axiom Man : Entity → Prop
axiom Mortal : Entity → Prop
axiom socrates : Entity

/-! ### Rule 5 ⟶ A. Unary predicates, English-nameable, no terms, no equality. -/

theorem routeA (hAll : ∀ x, Man x → Mortal x) (hSoc : Man socrates) : Mortal socrates :=
  hAll socrates hSoc

/-! ### Rule 4 ⟶ B. Relational, quantified, opaque sort, atoms take only variables. -/

axiom Point : Type
axiom Cong : Point → Point → Point → Point → Prop

theorem routeB
    (pr : ∀ a b, Cong a b b a)
    (it : ∀ a b c d e f, Cong a b c d → Cong a b e f → Cong c d e f)
    (a b : Point) : Cong a b a b :=
  it b a a b a b (pr b a) (pr b a)

/-! ### Rule 3 ⟶ C. Prenex-∀ over the Int primitive vocabulary, function terms present. -/

theorem routeC (x : Nat) : 2 * x ≤ 2 * x + 1 := Nat.le_succ _

/-! ### Rule 3 falls through ⟶ K. Function terms AND a non-prenex quantifier: rule 2 rules
    out A and B, rule 3's quantifier test fails, so it is K. This is the shape of most
    real arithmetic. -/

theorem routeK_alternating : ∀ x : Nat, ∃ y : Nat, x + 1 ≤ y :=
  fun x => ⟨x + 1, Nat.le_refl _⟩

/-! ### Rule 1 ⟶ K. A universe appears in the type. -/

theorem routeK_universe (α : Type) (P : α → Prop) (h : ∀ x, P x) (a : α) : P a := h a

/-! ### Rule 1 ⟶ K. Dependent Pi: the binder is used at type level in the body. -/

theorem routeK_dependent (P : Nat → Prop) (Q : ∀ n, P n → Prop) :
    ∀ (n : Nat) (h : P n), Q n h → Q n h :=
  fun _ _ hq => hq

/-! ### Rule 1 ⟶ K. An inductive family and a `Match` in the statement's own definition. -/

inductive Vect (α : Type) : Nat → Type where
  | nil : Vect α 0
  | cons : {n : Nat} → α → Vect α n → Vect α (n + 1)

def Vect.length {α : Type} {n : Nat} (_ : Vect α n) : Nat := n

theorem routeK_indexed (α : Type) (n : Nat) (v : Vect α n) : v.length = n := rfl

/-! ### Rule 1 ⟶ K. A typeclass argument. LOGOS has no class concept (T07-001); this is a
    structure plus a flat instance list, which only K can address. -/

class Pointed (α : Type) where
  point : α

theorem routeK_class (α : Type) [inst : Pointed α] : (Pointed.point : α) = inst.point := rfl

/-! ### Rule 6 ⟶ K. Nothing exotic, but equality between distinct constants and a
    function symbol put it outside every vernacular fragment. -/

def double (n : Nat) : Nat := n + n

theorem routeK_default (n : Nat) : double n = 2 * n := by
  simp [double, Nat.two_mul]

/-! ### Dependency closure. `usesRouteA` cites `routeA`, so BOTH must land on the same
    target even though `routeA` alone would qualify for Surface A. There is no way to
    cite a Surface A document from a kernel term. -/

axiom Doomed : Entity → Prop

theorem usesRouteA
    (hAll : ∀ x, Man x → Mortal x) (hSoc : Man socrates)
    (hDoom : ∀ x, Mortal x → Doomed x) : Doomed socrates :=
  hDoom socrates (routeA hAll hSoc)

end T12_011
