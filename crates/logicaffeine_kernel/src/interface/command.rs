//! Vernacular commands for interacting with the Kernel.

use super::surface::{Name, OpenClause, SurfaceDecl};
use crate::Term;

/// A command in the Vernacular language.
#[derive(Debug, Clone)]
pub enum Command {
    /// Definition name : type := body.
    ///
    /// If `ty` is None, the type is inferred from the body.
    /// If `is_hint` is true, register as a hint for auto tactic.
    Definition {
        name: String,
        ty: Option<Term>,
        body: Term,
        is_hint: bool,
        /// How many leading parameters of `ty` are implicit (`{x:T}` binders).
        implicit_count: usize,
        /// For a literate `## Theorem … Proof: <tactic>.`, the goal (in `Syntax` form) the
        /// tactic derivation must CONCLUDE. `Some` marks this definition as a checked tactic
        /// proof: the REPL rejects it unless `concludes body` reduces to exactly this goal,
        /// so a failed tactic (which yields a well-typed *error* derivation) cannot pass as a
        /// proof. `None` for every ordinary definition (including raw `d : Derivation := …`).
        proof_goal: Option<Term>,
    },

    /// Check term.
    ///
    /// Prints the type of the term.
    Check(Term),

    /// Eval term.
    ///
    /// Normalizes and prints the term.
    Eval(Term),

    /// A parser-extension declaration: notation, fixity, a macro, or a section boundary.
    ///
    /// It extends the SURFACE and never the namespace. That separation is the point: a flat global
    /// namespace with one phantom per notation is a collision waiting to happen, so a command that
    /// declares a way of SPELLING something must have nowhere to put a global even by accident.
    Surface(SurfaceDecl),

    /// `Open Alpha.` / `Open Alpha Beta.` / `Open Alpha (a b).` / `Open Alpha hiding (a).`
    ///
    /// It is a `Command` and not a [`SurfaceDecl`] for the reason [`Command::Export`] is: a clause
    /// that NAMES members (`(a b)`, `hiding (a)`) can only be checked against the ENVIRONMENT, and
    /// a clause naming something the namespace does not declare narrows the opening to nothing
    /// while reading exactly like one that worked. Both forms go through one door so the check can
    /// never be skipped by taking the other.
    Open(Vec<OpenClause>),

    /// `Open Alpha in <command>.` — an opening scoped to exactly ONE command.
    ///
    /// It is a `Command` and not a [`SurfaceDecl`] because it is CONTROL FLOW: the scope has to be
    /// wound back after the command it brackets, which nothing that merely mutates the surface can
    /// do. Lean's `open … in` scopes the same way and for the same reason.
    OpenIn {
        opens: Vec<OpenClause>,
        inner: Box<Command>,
    },

    /// `Export Alpha (token other).` — make `Alpha.token` answer to `token` IN THE CURRENT
    /// NAMESPACE, permanently, rather than only while an `open` is in scope. Lean's alias is a
    /// persistent environment extension, so it outlives the `End` of the namespace it was written
    /// in — which is the whole difference between `export` and `open`.
    ///
    /// It is a `Command` and not a [`SurfaceDecl`] because every one of its failure modes needs the
    /// ENVIRONMENT to detect: a name the source namespace does not declare, and a name the current
    /// namespace already declares. A surface declaration cannot see the context, and an `export`
    /// that could not check would be an alias pointing nowhere with nothing to say so.
    ///
    /// Lean's `export` takes its name list explicitly and so does this. There is no way to enumerate
    /// a namespace's members from the kernel context, and a bare `Export Alpha.` that exported
    /// nothing would be exactly the quiet no-op an unknown `Open` would have been.
    Export { namespace: Name, names: Vec<Name> },

    /// `Protected Definition Foo.x : …` — Lean's `protected def`, which declares an ordinary
    /// constant that stops answering to its SHORT name.
    ///
    /// It is a WRAPPER rather than a flag on the declaration for the reason `Open … in` is one: the
    /// mark is applied AFTER the inner command succeeds, so a declaration that does not check
    /// protects nothing, and no declaration door has to grow a field it would then have to
    /// remember to pass on. Corpus case T02-012 names this exact behaviour as the thing a
    /// translator must not assume is inert — "it changes which short name resolves at a use site".
    Protected(Box<Command>),

    /// `Print Axioms <name>.` — what a declaration RESTS ON: the axioms it cites transitively
    /// through its own definition graph, and the names that resolve to nothing in this context.
    ///
    /// Lean's `#print axioms`, and it is the report that makes an admission visible. Lean warns on
    /// the declaration that CONTAINS a `sorry` and says nothing about its dependants, so a theorem
    /// three lemmas downstream of one looks finished; this reads the transitive closure, so the
    /// taint is a property of the proof rather than of a warning somebody might have missed, and it
    /// cannot be lost by renaming, re-exporting or citing at a distance.
    ///
    /// It carries a [`Name`] rather than a key because it is a REFERENCE, and a reference reads in
    /// the namespace scope it is written in, exactly as a term's does.
    PrintAxioms(Name),

    /// A documentation block (`/-- … -/`, `/-! … -/`) or a line comment (`-- …`).
    ///
    /// Carries NO payload, by construction. `Literal::Text` exists, so a docstring *could* be
    /// emitted as a term — it would typecheck, put a name the source never declared into the flat
    /// namespace where it can collide, and be useless, since there are no string operations. A
    /// variant with nothing in it makes that mistake unrepresentable: prose is tolerated beside a
    /// declaration and is never data.
    Comment,

    /// `Theorem name : P := proof.` — a PROPOSITION and its proof.
    ///
    /// The statement must be a proposition, the proof is checked by both kernels against it by
    /// exact conversion, and the name is then published as Lean publishes a theorem
    /// (`Context::declare_theorem_checked`): later conversion may unfold it, at the lowest delta
    /// priority. The proof travels in the command as the evidence a reader re-checks.
    Theorem { name: String, ty: Term, proof: Term },

    /// `Theorem name : P := by tac; tac.` — a proposition proved by a tactic script over the kernel's
    /// own goals ([`crate::tactics::GoalState`]). The script's finished proof term enters through the
    /// door a [`Command::Theorem`]'s proof does; the script is never trusted.
    TheoremBy {
        name: String,
        ty: Term,
        script: Vec<crate::tactics::TacticStep>,
    },

    /// `Opaque name : T := value.` — the checked-then-opaque gateway (Lean's `opaque`): checked
    /// like a `Theorem`, with no requirement that `T` be a proposition, and never unfolded.
    Opaque { name: String, ty: Term, value: Term },

    /// `Axiom name : T.` — an explicit assumption. Its type is checked to be a type, it never has a
    /// body, and `Print Axioms` lists it on itself and on everything resting on it.
    Axiom { name: String, ty: Term },

    /// `Inductive Name (params) : arity := C1 : T1 | … with Name2 (params) : arity := … .`
    ///
    /// An inductive BLOCK registered exactly as declared, through the kernel's checked block door
    /// (`Context::add_mutual_inductives` / `add_universe_poly_inductives`) — the door the Lean
    /// importer uses. The written parameters ARE the parameters (never re-inferred), the rest of the
    /// arity is indices, and a block of several members is one mutual block whether or not they
    /// mention each other.
    ///
    /// Written whenever a header declares its arity, names universe parameters, or joins members
    /// with `with`; a header that does none of those is the legacy [`Command::Inductive`].
    InductiveBlock { members: Vec<InductiveMember> },

    /// `Quotient Type.{u}.`, `Quotient Constructor.{u}.`, `Quotient Lift.{u, v} using Eq.`,
    /// `Quotient Induction.{u}.` — one of the kernel's four quotient primitives (Lean's `init_quot`).
    ///
    /// The kernel has exactly one certified quotient, keyed at `kernel.quot.type`, `.mk`, `.lift`
    /// and `.ind`; the command names WHICH primitive to install and its universe parameters (always
    /// written, through [`Command::Universes`]), and `Lift` names the equality its respect hypothesis
    /// is stated with. `quotient::install` checks the rest: the order the primitives come in, and
    /// that the named equality is the canonical one.
    Quotient {
        kind: crate::quotient::Kind,
        equality: Option<String>,
    },

    /// `Structure Prod := Prod.fst Prod.snd.` — structure η for an inductive already declared,
    /// through projections already declared.
    ///
    /// It declares nothing: it records that `inductive` is a STRUCTURE, which is what licenses
    /// η (`x ≡ mk p̄ (proj₀ p̄ x) … (projₙ p̄ x)`) and Lean's structure iota. The `Repl` grants it only
    /// once each projection is shown to COMPUTE its field — `projᵢ p̄ (mk p̄ f̄) ≡ fᵢ` for a generic
    /// constructor application — the same check the environment certificate makes before it
    /// registers η, so a document and a certificate cannot disagree about what a structure is.
    Structure {
        inductive: String,
        projections: Vec<String>,
    },

    /// `Definition name.{u, v} …` (and `Theorem`, `Opaque`, `Axiom`, `Inductive`) — universe
    /// parameters BINDING over the declaration they are written on.
    ///
    /// The level variables `params` may occur in the declaration's types and terms and nowhere else;
    /// the declaration registers through the kernel's universe-polymorphic door
    /// (`declare_universe_poly`, `add_universe_poly_declaration`, `declare_opaque_checked` with a
    /// telescope, `add_universe_poly_inductives`), which REFUSES a level variable the telescope
    /// does not bind. It is a wrapper rather than a field for the reason `Protected` and `Open … in`
    /// are wrappers: it scopes one declaration, every declaration door takes it the same way, and a
    /// telescope on a command that declares nothing is refused rather than ignored.
    Universes {
        params: Vec<String>,
        inner: Box<Command>,
    },

    /// Inductive Name (params) := C1 : T1 | C2 : T2.
    ///
    /// Defines an inductive type with its constructors.
    /// Supports optional type parameters for polymorphic inductives.
    Inductive {
        name: String,
        /// Type parameters: (param_name, param_type)
        /// e.g., for `List (A : Type)`, params = [("A", Type)]
        params: Vec<(String, Term)>,
        sort: Term,
        constructors: Vec<(String, Term)>,
    },
}

/// One member of an [`Command::InductiveBlock`].
///
/// The shape is the legacy [`Command::Inductive`]'s: the parameters are split out and bound by
/// name, and `sort` is the DECLARED arity after the parameters — its binders are the indices and
/// its final sort is the universe the type lives in; it refers to a parameter as the global of
/// that name.
///
/// A constructor is written one of two ways, the same for every constructor of a member:
///
/// * `full_constructors == false` — `C : T`, where `T` omits the parameter prefix and refers to a
///   parameter as the global of its name; the registration puts the member's parameters back;
/// * `full_constructors == true` — `C (p₁ : T₁) … (pₙ : Tₙ) : T`, the constructor RESTATING its
///   parameter telescope, and `constructors` holds each FULL type, parameters bound as ordinary
///   binders. The restated types may spell a parameter differently from the member (Lean's
///   `HAdd.mk` binds `γ : outParam (Type w)` where `HAdd`'s arity says `Type w`); the kernel's block
///   door checks them and registers the constructor exactly as written. A member with no
///   parameters has one form only, and the flag is `false`.
#[derive(Debug, Clone)]
pub struct InductiveMember {
    pub name: String,
    pub params: Vec<(String, Term)>,
    pub sort: Term,
    pub constructors: Vec<(String, Term)>,
    pub full_constructors: bool,
}
