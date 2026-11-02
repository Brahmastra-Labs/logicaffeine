//! Independently re-checkable proof certificates — the De Bruijn criterion.
//!
//! A proof in LOGOS is a [`Term`]. A *certificate* bundles that term with the
//! proposition it claims to prove, so anyone can re-validate it against the
//! prelude axioms — with **no** access to the parser, the proof-search engine,
//! the certifier, or any oracle. The entire trusted surface of a re-check is
//! this crate (`logicaffeine_kernel`) plus its single dependency
//! (`logicaffeine_base`).
//!
//! # The integrity rule
//!
//! A certificate carries **only** the proof term and the claimed type — never a
//! context. `recheck` rebuilds the trusted axiom context *itself* via
//! [`StandardLibrary::register`], so a malicious certificate cannot smuggle in a
//! bogus axiom (e.g. a free proof of `False`). You trust the prelude's primitives
//! and the type-checker; you do not trust the certificate's provenance.
//!
//! # Three kinds, one family
//!
//! * [`Certificate`] — a proof in the bare prelude.
//! * [`TheoremCertificate`] — a proof from a disclosed environment delta over the prelude.
//! * [`EnvironmentCertificate`] (`logos-environment-certificate/1` or `/2`) — a proof together with every
//!   declaration it needs (inductives, constructors, recursors, universe-polymorphic definitions,
//!   opaque theorems carried with their proofs, structure η, quotient primitives, assumptions),
//!   rebuilt from a declared base with every entry kernel-validated. This is what a Lean import is
//!   packaged as.
//!
//! [`read_certificate`] reads any of them by one door. Every struct refuses unknown fields, and a
//! certificate minted against a retired prelude (`logos-coc-1` …) is refused as incompatible — the
//! tag is never rewritten to fit.
//!
//! This module is available under the `serde` feature.

use serde::{Deserialize, Serialize};

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::prelude::StandardLibrary;
use crate::{
    double_check, infer_type, is_subtype, Context, DoubleCheck, KernelError, Term, Universe,
};

/// The prelude/axiom-set version a certificate was produced against. A
/// re-checker may refuse certificates minted against an axiom set it does not
/// recognise. Bump this whenever the trusted prelude changes.
///
/// `logos-coc-6` DEFINES `Quot_rec`. The dependent quotient eliminator is a checked
/// definition — `Quot_lift` into Lean's `PSigma`, `Quot_ind`, and a transport — so it
/// leaves the trusted base, and its axiom closure is `Quot_sound`, read through its
/// body. `Quot_lift`'s target is `Sort v`, as Lean's is, which that definition needs.
/// `Quot`, `Quot_mk`, `Quot_lift` and `Quot_ind` keep the registered primitive
/// identities tied to their computation rules and are not reported as assumption
/// leaves while that identity is live. `Quot_sound`, `propext`, `Classical_choice`
/// and the opaque `Float` operation surface remain assumptions.
///
/// Earlier tags, including `logos-coc-5`, are refused even when a particular
/// proof would still check: a certificate claims to have been checked against
/// its stated assumptions, and no validated migration rewrites that claim.
/// The wire schemas and entry encodings are unchanged; only the trusted prelude
/// version changes with its primitive computation and assumption accounting.
pub const PRELUDE_VERSION: &str = "logos-coc-6";

/// A self-contained, re-checkable proof certificate.
///
/// `proof_term` is claimed to have type `claimed_type` in the standard prelude
/// context. Nothing else is trusted: the re-checker supplies its own axioms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    /// The proof (a lambda-calculus term).
    pub proof_term: Term,
    /// The proposition the proof claims to establish (a type).
    pub claimed_type: Term,
    /// The axiom-set version this certificate was produced against.
    pub prelude_version: String,
}

impl Certificate {
    /// Bundle a proof term with the proposition it claims to prove.
    pub fn new(proof_term: Term, claimed_type: Term) -> Self {
        Self {
            proof_term: proof_term.into_stripped(),
            claimed_type: claimed_type.into_stripped(),
            prelude_version: PRELUDE_VERSION.to_string(),
        }
    }

    /// The axioms this certificate's proof actually rests on — our `#print axioms`.
    ///
    /// Computed against the trusted base the certificate would be RE-CHECKED in, which is the
    /// only context whose answer is worth anything: a closure taken against the minting side's
    /// environment reports what the prover believed, not what the verifier is being asked to
    /// accept.
    ///
    /// Over the proof term AND the claimed type, because a certificate is both. An axiom named
    /// in the theorem's own STATEMENT is a real dependency — `Π(h : Nonempty Nat). Eq Nat
    /// (Classical_choice Nat h) Zero` rests on choice whatever the proof term looks like — and
    /// Lean reports it: `CollectAxioms` visits a constant's `type` as well as its `value`.
    pub fn axioms_used(&self) -> AxiomUsage {
        let ctx = StandardLibrary::context();
        let mut usage = axiom_dependencies(ctx, &self.proof_term);
        usage.absorb(axiom_dependencies(ctx, &self.claimed_type));
        usage
    }
}

/// Independently re-validate a certificate.
///
/// Builds a *fresh* trusted context from the standard library, infers the type
/// of the certificate's proof term, and requires it to match the claimed type
/// (up to definitional equality). Returns `Ok(())` only if the proof genuinely
/// establishes the claimed proposition.
///
/// This function is the whole trusted core of an independent check. It does not
/// look at where the certificate came from.
pub fn recheck(cert: &Certificate) -> Result<(), KernelError> {
    if cert.prelude_version != PRELUDE_VERSION {
        return Err(KernelError::CertificationError(format!(
            "certificate was minted against prelude '{}', but this checker is '{}'",
            cert.prelude_version, PRELUDE_VERSION
        )));
    }

    // The trusted axiom context is this checker's own standard library registration — never a
    // context the certificate might carry.
    let ctx = StandardLibrary::context();

    // BEFORE type-checking: what does this proof rest on? An ADMITTED step — `sorryAx` and its
    // relatives — type-checks perfectly and proves anything, so "it type-checks" is precisely the
    // wrong question to ask first. The closure is transitive, so a `sorry` three lemmas upstream
    // is caught by the same check as one written in the proof term itself.
    let mut usage = axiom_dependencies(ctx, &cert.proof_term);
    usage.absorb(axiom_dependencies(ctx, &cert.claimed_type));
    if usage.is_admitted() {
        return Err(KernelError::CertificationError(
            usage.render("this certificate"),
        ));
    }

    // The term must type-check at all...
    let inferred = infer_type(ctx, &cert.proof_term)?;

    // ...and its type must be the claimed proposition.
    if is_subtype(ctx, &inferred, &cert.claimed_type) {
        Ok(())
    } else {
        Err(KernelError::CertificationError(format!(
            "certificate term proves a different proposition: has type {:?}, claims {:?}",
            inferred, cert.claimed_type
        )))
    }
}

// ---------------------------------------------------------------------------
// Theorem certificates — a proof discharged FROM disclosed premises.
// ---------------------------------------------------------------------------
//
// The prelude-only [`Certificate`] above proves `⊢ goal` in the bare axiom set:
// perfect for a purely logical theorem like `∀(P:Prop). P → P`. But a real
// `## Theorem` block is proved `Γ ⊢ goal`, where `Γ` names the theorem's own
// vocabulary (its predicates and constants) and its stated premises as
// hypotheses — e.g. `Socrates is a man` becomes a declaration `h : man Socrates`,
// and the proof term `h2 Socrates h1` cites those hypotheses. Such a proof cannot
// re-check against the bare prelude: its free names are unbound there.
//
// A [`TheoremCertificate`] therefore carries the theorem's ENVIRONMENT DELTA — the
// declarations (premise hypotheses + declared predicates/constants) and
// transparent definitions the theorem introduced beyond the standard prelude —
// EXPLICITLY, so an independent checker can rebuild the exact context and type-check
// the proof term against the claimed goal WITHOUT ever running proof search.
//
// # Honesty
//
// The delta is DISCLOSED, not trusted-blind: it IS the theorem's statement (its
// signature and premises), which a reader inspects. The trusted core stays the
// standard prelude plus the type-checker; the checker rebuilds the prelude ITSELF
// and only then replays the disclosed delta. The certificate proves the honest
// judgement `premises ⊢ goal`, never the stronger `⊢ goal` — the hypotheses it
// assumes are laid out in the open. A verifier who does not accept a stated premise
// simply does not accept the theorem; they can never be tricked about WHICH premises
// were assumed, because every one is enumerated in `hypotheses`.

/// A declaration in a theorem's disclosed environment: a `name : type` assumption
/// (a premise hypothesis, or a declared predicate/constant) that the proof cites.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertDeclaration {
    pub name: String,
    pub ty: Term,
}

/// A transparent definition in a theorem's disclosed environment: `name : type := body`,
/// δ-unfoldable during re-checking exactly as the prover registered it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertDefinition {
    pub name: String,
    pub ty: Term,
    pub body: Term,
}

/// A self-contained, re-checkable certificate for a theorem proved FROM premises.
///
/// `proof_term` is claimed to have type `claimed_type` in the standard prelude
/// EXTENDED by the disclosed `declarations` (assumptions) and `definitions`
/// (transparent). The re-checker rebuilds the prelude itself and replays only this
/// disclosed delta, then type-checks — it never trusts, or even sees, how the proof
/// was found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TheoremCertificate {
    /// The theorem's identity (its `## Theorem: Name`), for provenance in reports.
    pub theorem: String,
    /// The proof (a lambda-calculus term).
    pub proof_term: Term,
    /// The proposition the proof claims to establish (a type) — the theorem's goal.
    pub claimed_type: Term,
    /// The disclosed premise hypotheses + declared vocabulary the proof assumes.
    /// Every assumption the theorem rests on is enumerated here, in the open.
    pub declarations: Vec<CertDeclaration>,
    /// The disclosed transparent definitions the proof unfolds.
    pub definitions: Vec<CertDefinition>,
    /// The axiom-set version this certificate was produced against.
    pub prelude_version: String,
}

/// Every AXIOM a term actually depends on — the `#print axioms` analogue.
///
/// Lean's single most important trust tool is the ability to ask a finished theorem which
/// assumptions it really rests on. `TheoremCertificate` discloses only the delta a caller chose to
/// pass, never the prelude axioms the proof cites, so "which theorems depend on `Classical_choice`?"
/// — or on a smuggled `sorry` — was unanswerable at any surface.
///
/// The implementation lives in [`crate::prelude`], NOT here, and is re-exported: this module is
/// behind the `serde` feature, and an audit trail that disappears when certificates are not being
/// serialised is not an audit trail. The REPL's `Print Axioms`, the kernel's own trusted-base
/// diagnostics and `logicaffeine_proof`'s verdicts all need it in the default build, and all three
/// must read the SAME closure — a second copy is a second answer.
pub use crate::prelude::{
    axiom_dependencies, axiom_dependencies_of, is_admitted_axiom, AxiomUsage, ADMITTED_AXIOMS,
};

impl TheoremCertificate {
    /// The axioms this certificate's proof actually rests on, re-computed against the trusted base
    /// it would be re-checked in — NOT against whatever the minting side chose to disclose.
    ///
    /// Over the proof term AND the claimed goal, for the same reason [`Certificate::axioms_used`]
    /// is: the statement is half of what a certificate asserts, and an assumption it names is one
    /// the reader is being asked to accept. A report that walked only the proof term would say a
    /// theorem ABOUT `Classical_choice` rests on nothing, and `axiom_dependencies_of` — the door
    /// the REPL comes in by — already answers the same question the other way, so the two doors
    /// would disagree about one graph.
    pub fn axioms_used(&self) -> AxiomUsage {
        let ctx = self.rebuild_context().0;
        theorem_axioms(&ctx, &self.proof_term, &self.claimed_type)
    }
}

/// The closure of a theorem: its proof term and its statement, merged. One function, so
/// [`TheoremCertificate::axioms_used`] and [`recheck_theorem`] can never come to report different
/// axiom sets for the same certificate — which is the failure that makes an audit trail worthless,
/// because the reader has no way to tell which door they came in by.
fn theorem_axioms(ctx: &Context, proof_term: &Term, claimed_type: &Term) -> AxiomUsage {
    let mut usage = axiom_dependencies(ctx, proof_term);
    usage.absorb(axiom_dependencies(ctx, claimed_type));
    usage
}

/// The outcome of independently re-checking a [`TheoremCertificate`].
#[derive(Debug, Clone, PartialEq)]
pub struct TheoremRecheck {
    /// Whether the proof term type-checks to the claimed goal in the rebuilt context **and rests
    /// on nothing ADMITTED**. This is the load-bearing verdict — established by TYPE-CHECKING the
    /// term, never by re-running proof search.
    ///
    /// The second conjunct is the part Lean leaves to the reader. A `sorry`-tainted proof
    /// type-checks, both kernels agree on it, and a caller that asks only "did it check?" is
    /// told yes. So the taint is folded into THIS field rather than offered beside it: there is
    /// no reading of `rechecks == true` under which the proof rests on an admission, and no
    /// command anyone has to remember to run.
    pub rechecks: bool,
    /// The strongest agreement reached: `true` when the SECOND, independent de Bruijn
    /// kernel ([`double_check`]) also verified the term to a definitionally-equal type
    /// (the full de Bruijn criterion). `false` when the term type-checks under the main
    /// kernel but sits outside the re-checker's covered fragment (still a valid proof,
    /// honestly single-checked) or fails to re-check at all.
    pub two_kernel_agreement: bool,
    /// The reason a re-check failed, when it did.
    pub error: Option<String>,
    /// Every axiom the proof rests on, TRANSITIVELY — our `#print axioms`, re-derived against the
    /// trusted base this check rebuilt rather than against whatever the minting side disclosed.
    ///
    /// Populated even when the re-check FAILS, and deliberately: "which assumptions did this
    /// thing want?" is exactly the question a rejected certificate raises, and answering it is
    /// most of the diagnosis.
    ///
    /// EMPTY on exactly two refusals, and in both the honest answer is "no report": a
    /// version mismatch means the names were not checked against THIS base at all, and a delta
    /// that REDEFINES a trusted constant means a report naming `dne` would name the reader's
    /// `dne` and not the certificate's — which is the misreading that refusal exists to prevent.
    pub axioms: AxiomUsage,
}

impl TheoremCertificate {
    /// Bundle a proof term, its goal, and the disclosed environment delta.
    pub fn new(
        theorem: impl Into<String>,
        proof_term: Term,
        claimed_type: Term,
        declarations: Vec<CertDeclaration>,
        definitions: Vec<CertDefinition>,
    ) -> Self {
        Self {
            theorem: theorem.into(),
            proof_term: proof_term.into_stripped(),
            claimed_type: claimed_type.into_stripped(),
            declarations: declarations
                .into_iter()
                .map(|d| CertDeclaration {
                    name: d.name,
                    ty: d.ty.into_stripped(),
                })
                .collect(),
            definitions: definitions
                .into_iter()
                .map(|d| CertDefinition {
                    name: d.name,
                    ty: d.ty.into_stripped(),
                    body: d.body.into_stripped(),
                })
                .collect(),
            prelude_version: PRELUDE_VERSION.to_string(),
        }
    }

    /// Rebuild the trusted context: a fresh standard prelude, then the disclosed delta — and the
    /// delta may only EXTEND that prelude, never replace any part of it.
    ///
    /// Returns the context together with every delta name the trusted base already owned. Those
    /// entries are NOT applied, and [`recheck_theorem`] turns a non-empty list into a refusal.
    ///
    /// # The hole this closes
    ///
    /// `Context::add_declaration` and `Context::add_definition` both begin by DISPLACING the name
    /// — one name means one thing, which is right — so replaying a delta entry named `dne` or
    /// `Classical_choice` does not sit alongside the prelude's, it REPLACES it. A certificate
    /// could therefore disclose `dne : Π(P:Prop). P`, prove `False` from it, and re-check GREEN
    /// with both kernels agreeing; the axiom report would name `dne`, a constant that IS in the
    /// trusted base, and a reader auditing the certificate would see a proof resting on
    /// double-negation elimination. It rests on something else entirely. Disclosure is only
    /// honest if the names in it mean what the reader's own prelude says they mean.
    ///
    /// **Declarations first, definitions second, and the order is load-bearing.** A declaration is
    /// a `name : type` assumption with no body, so nothing about registering one can depend on a
    /// definition; a definition's BODY routinely cites a declared premise — `h1` in
    /// `h2 Socrates h1` is the whole point of the delta. `add_definition` is a CHECKED gateway: it
    /// infers the body's type and refuses a body it can neither check nor defer, so replaying a
    /// definition before the premise it cites can drop the definition on the floor (recorded in
    /// `refused_declarations`, invisible to a caller reading only `rechecks`). The minting side's
    /// emission order is not something a re-checker may depend on, so the re-checker imposes the
    /// one order that always works.
    fn rebuild_context(&self) -> (Context, Vec<String>) {
        let mut ctx = StandardLibrary::context().clone();
        let mut shadowed: Vec<String> = Vec::new();
        for decl in &self.declarations {
            if ctx.get_global(&decl.name).is_some() {
                shadowed.push(decl.name.clone());
                continue;
            }
            ctx.add_declaration(&decl.name, decl.ty.clone());
        }
        for def in &self.definitions {
            if ctx.get_global(&def.name).is_some() {
                shadowed.push(def.name.clone());
                continue;
            }
            ctx.add_definition(def.name.clone(), def.ty.clone(), def.body.clone());
        }
        (ctx, shadowed)
    }
}

/// Independently re-validate a [`TheoremCertificate`].
///
/// Rebuilds the prelude + disclosed delta, infers the proof term's type, and requires
/// it to be the claimed goal (up to definitional equality / cumulativity). The verdict
/// comes from TYPE-CHECKING, not from re-running the prover's backward-chaining search
/// — so the checker is a genuinely independent witness. Where the term lies inside the
/// second kernel's fragment, [`double_check`] corroborates it, giving the full de Bruijn
/// two-kernel guarantee (`two_kernel_agreement`).
///
/// # The admission gate — and why it is a gate rather than a note
///
/// The transitive axiom closure is computed FIRST, against the rebuilt context, and a proof that
/// rests on anything [`is_admitted_axiom`] recognises does not re-check at all. This is the one
/// place LOGOS deliberately does more than Lean, and the reason is the shape of the failure:
/// `sorryAx` proves anything, so a tainted proof type-checks, BOTH kernels agree on it, and
/// `verified == true` really does hold. Lean warns on the declaration that CONTAINS the `sorry`
/// and says nothing about its dependants, so a theorem three lemmas downstream looks finished and
/// `#print axioms` is a command you must remember to run. Here the closure is transitive and the
/// verdict carries it, so the taint cannot be lost by renaming, re-exporting, citing at a distance
/// — or by a caller reading only `rechecks`.
///
/// Tracking is not banning: a proof resting on `Classical_choice`, `propext` or `Quot_sound` is a
/// perfectly good proof and re-checks, with those names reported in `axioms`. The gate is for the
/// names that mark a proof as NOT FINISHED.
pub fn recheck_theorem(cert: &TheoremCertificate) -> TheoremRecheck {
    if cert.prelude_version != PRELUDE_VERSION {
        return TheoremRecheck {
            rechecks: false,
            two_kernel_agreement: false,
            error: Some(format!(
                "certificate was minted against prelude '{}', but this checker is '{}'",
                cert.prelude_version, PRELUDE_VERSION
            )),
            axioms: AxiomUsage::default(),
        };
    }

    let (ctx, shadowed) = cert.rebuild_context();
    if !shadowed.is_empty() {
        return TheoremRecheck {
            rechecks: false,
            two_kernel_agreement: false,
            error: Some(format!(
                "the disclosed delta REDEFINES constants the trusted base already owns: {}. A \
                 certificate may extend the prelude with its own vocabulary and premises; it may \
                 never replace an axiom or definition the prelude already has, because every name \
                 it discloses has to mean what the reader's own prelude says it means",
                shadowed.join(", ")
            )),
            axioms: AxiomUsage::default(),
        };
    }

    // What does this proof rest on? Asked BEFORE "does it type-check", because an admitted step
    // type-checks and the answer would be reassuring and wrong. Over the STATEMENT as well as the
    // proof term: a goal that cites `sorryAx` is not a proposition anyone proved either, and a
    // gate that looked only at the proof term could be walked straight past by moving the
    // admission into the type.
    let axioms = theorem_axioms(&ctx, &cert.proof_term, &cert.claimed_type);
    if axioms.is_admitted() {
        return TheoremRecheck {
            rechecks: false,
            two_kernel_agreement: false,
            error: Some(axioms.render(&cert.theorem)),
            axioms,
        };
    }

    // The term must type-check at all, and to the claimed goal.
    let inferred = match infer_type(&ctx, &cert.proof_term) {
        Ok(t) => t,
        Err(e) => {
            return TheoremRecheck {
                rechecks: false,
                two_kernel_agreement: false,
                error: Some(format!("proof term did not type-check: {:?}", e)),
                axioms,
            };
        }
    };
    if !is_subtype(&ctx, &inferred, &cert.claimed_type) {
        return TheoremRecheck {
            rechecks: false,
            two_kernel_agreement: false,
            error: Some(format!(
                "certificate term proves a different proposition: has type {:?}, claims {:?}",
                inferred, cert.claimed_type
            )),
            axioms,
        };
    }

    // Corroborate with the SECOND, independently-written de Bruijn kernel. Agreement is
    // the full guarantee; an incomplete re-check (a `Fix`/`Hole` fragment) is honestly
    // reported as single-checked, NOT dressed up as agreement. A `Disagree` — one kernel
    // accepting what the other rejects — is a soundness alarm and fails the re-check.
    match double_check(&ctx, &cert.proof_term) {
        DoubleCheck::Agreed => TheoremRecheck {
            rechecks: true,
            two_kernel_agreement: true,
            error: None,
            axioms,
        },
        DoubleCheck::MainOnlyReCheckerIncomplete(_) => TheoremRecheck {
            rechecks: true,
            two_kernel_agreement: false,
            error: None,
            axioms,
        },
        DoubleCheck::Disagree(why) => TheoremRecheck {
            rechecks: false,
            two_kernel_agreement: false,
            error: Some(format!(
                "the two kernels DISAGREED on the proof term: {why}"
            )),
            axioms,
        },
    }
}

// ---------------------------------------------------------------------------
// Reading any certificate — one format family, dispatched by shape and schema.
// ---------------------------------------------------------------------------

/// Every certificate this module defines, read from JSON by one door.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyCertificate {
    /// A [`Certificate`]: a proof in the bare prelude.
    Proof(Certificate),
    /// A [`TheoremCertificate`]: a proof from a disclosed environment delta.
    Theorem(TheoremCertificate),
    /// An [`EnvironmentCertificate`]: a proof together with every declaration it needs.
    Environment(EnvironmentCertificate),
}

/// Prelude tags this checker knows it is NOT. A certificate carrying one of them is refused with
/// an explicit incompatibility — the axiom sets differ and no migration exists — rather than the
/// generic "unknown version" a typo would get. The tag is never rewritten to make it fit.
pub const RETIRED_PRELUDE_VERSIONS: &[&str] = &[
    "logos-coc-1",
    "logos-coc-2",
    "logos-coc-3",
    "logos-coc-4",
    "logos-coc-5",
];

/// Read a certificate of any kind from JSON.
///
/// The kind is decided by the document's own shape: an object with a `schema` field is a versioned
/// certificate and must name a schema this checker implements; otherwise a `theorem` field marks a
/// [`TheoremCertificate`] and its absence a bare [`Certificate`]. Every struct refuses unknown
/// fields, so a theorem certificate can no longer be silently read as a bare one with its
/// environment dropped. The prelude tag is checked here, before anything is re-checked.
pub fn read_certificate(json: &str) -> Result<AnyCertificate, EnvironmentError> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        EnvironmentError::new(
            EnvironmentErrorCode::Malformed,
            format!("not a JSON certificate: {e}"),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        EnvironmentError::new(
            EnvironmentErrorCode::Malformed,
            "a certificate is a JSON object",
        )
    })?;
    let prelude = object
        .get("prelude_version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let certificate = if let Some(schema) = object.get("schema") {
        match schema.as_str() {
            Some(OPAQUE_ENVIRONMENT_SCHEMA) | Some(ENVIRONMENT_SCHEMA) => {
                let environment: EnvironmentCertificate = serde_json::from_value(value.clone()).map_err(|e| {
                    EnvironmentError::new(EnvironmentErrorCode::Malformed, format!("{OPAQUE_ENVIRONMENT_SCHEMA}: {e}"))
                })?;
                environment.check_schema()?;
                AnyCertificate::Environment(environment)
            }
            other => {
                return Err(EnvironmentError::new(
                    EnvironmentErrorCode::UnsupportedSchema,
                    format!("unsupported certificate schema {other:?}; this checker reads {OPAQUE_ENVIRONMENT_SCHEMA}"),
                ))
            }
        }
    } else if object.contains_key("theorem") {
        AnyCertificate::Theorem(serde_json::from_value(value.clone()).map_err(|e| {
            EnvironmentError::new(
                EnvironmentErrorCode::Malformed,
                format!("theorem certificate: {e}"),
            )
        })?)
    } else {
        AnyCertificate::Proof(serde_json::from_value(value.clone()).map_err(|e| {
            EnvironmentError::new(
                EnvironmentErrorCode::Malformed,
                format!("proof certificate: {e}"),
            )
        })?)
    };
    check_prelude_version(prelude.as_deref().unwrap_or(""))?;
    Ok(certificate)
}

/// Refuse any prelude tag but this checker's own, explicitly for a retired one.
pub fn check_prelude_version(found: &str) -> Result<(), EnvironmentError> {
    if found == PRELUDE_VERSION {
        return Ok(());
    }
    let message = if RETIRED_PRELUDE_VERSIONS.contains(&found) {
        format!(
            "certificate was minted against the retired prelude '{found}'; this checker is \
             '{PRELUDE_VERSION}'. The two axiom sets differ and no validated migration exists, so the \
             certificate is incompatible: re-mint it against '{PRELUDE_VERSION}'"
        )
    } else {
        format!("certificate was minted against prelude '{found}', but this checker is '{PRELUDE_VERSION}'")
    };
    Err(EnvironmentError::new(
        EnvironmentErrorCode::IncompatiblePrelude,
        message,
    ))
}

// ---------------------------------------------------------------------------
// Environment certificates — a proof together with every declaration it needs.
// ---------------------------------------------------------------------------
//
// A [`TheoremCertificate`] discloses premises and transparent definitions over the standard
// prelude. A proof imported from another system needs more: its own inductive types with their
// constructors and recursors, universe-polymorphic declarations, opaque theorems whose proofs are
// evidence but never unfold, certified quotient primitives, structure η, and assumptions under an
// explicit policy. An [`EnvironmentCertificate`] carries all of it, over a declared base.
//
// # Deserializing is not validating
//
// Nothing a certificate says is registered on its word. [`EnvironmentCertificate::rebuild`] starts
// from the declared base (built by the checker itself), and hands every entry to the kernel's
// CHECKED gateways in order: an inductive block goes through positivity and universe checking, a
// definition's body is type-checked against its type, an opaque theorem is checked by both kernels
// before its header is published, a structure's projections are proved to compute, a quotient
// primitive must carry its canonical signature. A name the base already owns is never replaced, a
// reference to a name not yet declared is refused rather than deferred, and a rebuild that ends
// with an owed obligation fails.
//
// # Terms are a table
//
// Terms are written once each, children first, in `nodes`; a node refers to earlier nodes by
// index. A forward or self reference is therefore a malformed certificate rather than a cycle to
// chase, the node count, depth and unfolded size of every root are known BEFORE any term is built,
// and a shared subterm costs one row however often it occurs.

/// The latest environment-certificate format. Version 2 adds checked opaque declarations.
/// Capture writes the oldest supported format that can represent its entries: version 1 for
/// original entry kinds, version 2 when an opaque occurs. Readers never retag existing evidence.
pub const OPAQUE_ENVIRONMENT_SCHEMA: &str = "logos-environment-certificate/2";
/// The original format, supported unchanged for certificates containing its original entry kinds.
pub const ENVIRONMENT_SCHEMA: &str = "logos-environment-certificate/1";

/// A reference to an earlier row of a certificate's term table.
pub type NodeRef = u32;

/// One row of a certificate's term table: a [`Term`] constructor whose children are earlier rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TermNode {
    Sort(Universe),
    Var(String),
    Global(String),
    Const(String, Vec<Universe>),
    Pi(String, NodeRef, NodeRef),
    Lambda(String, NodeRef, NodeRef),
    App(NodeRef, NodeRef),
    Match(NodeRef, NodeRef, Vec<NodeRef>),
    Fix(String, NodeRef),
    MutualFix(Vec<(String, NodeRef)>, usize),
    Let(String, NodeRef, NodeRef, NodeRef),
    Lit(crate::Literal),
    Hole,
}

/// The trusted base an environment is rebuilt on. The checker builds it; the certificate only
/// names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentBase {
    /// The bare kernel: no declarations at all. An imported environment brings everything it uses,
    /// and may not take a name the standard library gives native meaning to.
    Kernel,
    /// The standard LOGOS prelude, `PRELUDE_VERSION`.
    StandardLibrary,
}

/// How declared types are compared with inferred ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionDiscipline {
    /// LOGOS's own judgement: cumulative universes.
    Cumulative,
    /// Exact conversion throughout — every application, let and declaration compared by
    /// definitional equality, never by cumulative subtyping. The discipline of a Lean import.
    Exact,
}

/// Lean's four quotient primitives, as the kernel certifies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotientPrimitive {
    Type,
    Constructor,
    Lift,
    Induction,
}

impl QuotientPrimitive {
    fn kind(self) -> crate::quotient::Kind {
        use crate::quotient::Kind;
        match self {
            QuotientPrimitive::Type => Kind::Type,
            QuotientPrimitive::Constructor => Kind::Constructor,
            QuotientPrimitive::Lift => Kind::Lift,
            QuotientPrimitive::Induction => Kind::Induction,
        }
    }
    fn of(kind: crate::quotient::Kind) -> Self {
        use crate::quotient::Kind;
        match kind {
            Kind::Type => QuotientPrimitive::Type,
            Kind::Constructor => QuotientPrimitive::Constructor,
            Kind::Lift => QuotientPrimitive::Lift,
            Kind::Induction => QuotientPrimitive::Induction,
        }
    }
}

/// One constructor of an inductive type: its name and full type (parameters included).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstructorClaim {
    pub name: String,
    pub ty: NodeRef,
}

/// One member of an inductive block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InductiveType {
    pub name: String,
    /// The arity: parameters, then indices, then the sort.
    pub arity: NodeRef,
    /// How many leading arity binders are uniform parameters.
    pub num_params: usize,
    pub constructors: Vec<ConstructorClaim>,
}

/// One declaration of an environment, in dependency order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentEntry {
    /// An assumption: a name and a type, no proof. Subject to the reader's assumption policy.
    Axiom {
        name: String,
        level_params: Vec<String>,
        ty: NodeRef,
    },
    /// A transparent definition, δ-unfoldable. Recursors and structure projections are
    /// definitions: their bodies are type-checked like any other.
    Definition {
        name: String,
        level_params: Vec<String>,
        ty: NodeRef,
        body: NodeRef,
    },
    /// A theorem. Its proof is carried and checked by both kernels, and the declaration is
    /// published as Lean publishes a theorem: conversion may unfold it at the lowest priority.
    Theorem {
        name: String,
        level_params: Vec<String>,
        ty: NodeRef,
        proof: NodeRef,
    },
    /// A checked opaque value or proof. Its body is evidence and never unfolds in conversion.
    Opaque {
        name: String,
        level_params: Vec<String>,
        ty: NodeRef,
        body: NodeRef,
    },
    /// An inductive block (one member, or a mutual block) and its constructors.
    Inductive {
        level_params: Vec<String>,
        types: Vec<InductiveType>,
    },
    /// Structure η for an earlier single-constructor inductive, through earlier projection
    /// definitions. Registered only after every projection is proved to compute.
    Structure {
        inductive: String,
        projections: Vec<String>,
    },
    /// A certified quotient primitive. `equality` names the equality inductive `lift` respects.
    Quotient {
        primitive: QuotientPrimitive,
        level_params: Vec<String>,
        ty: NodeRef,
        equality: Option<String>,
    },
}

impl EnvironmentEntry {
    /// The names this entry introduces, in order.
    pub fn names(&self) -> Vec<&str> {
        match self {
            EnvironmentEntry::Axiom { name, .. }
            | EnvironmentEntry::Definition { name, .. }
            | EnvironmentEntry::Theorem { name, .. }
            | EnvironmentEntry::Opaque { name, .. } => vec![name.as_str()],
            EnvironmentEntry::Inductive { types, .. } => types
                .iter()
                .flat_map(|t| {
                    std::iter::once(t.name.as_str())
                        .chain(t.constructors.iter().map(|c| c.name.as_str()))
                })
                .collect(),
            EnvironmentEntry::Structure { .. } => Vec::new(),
            EnvironmentEntry::Quotient { primitive, .. } => vec![primitive.kind().name()],
        }
    }

    /// Every term row this entry cites directly.
    pub fn roots(&self) -> Vec<NodeRef> {
        match self {
            EnvironmentEntry::Axiom { ty, .. } | EnvironmentEntry::Quotient { ty, .. } => vec![*ty],
            EnvironmentEntry::Definition { ty, body, .. }
            | EnvironmentEntry::Opaque { ty, body, .. } => vec![*ty, *body],
            EnvironmentEntry::Theorem { ty, proof, .. } => vec![*ty, *proof],
            EnvironmentEntry::Inductive { types, .. } => types
                .iter()
                .flat_map(|t| std::iter::once(t.arity).chain(t.constructors.iter().map(|c| c.ty)))
                .collect(),
            EnvironmentEntry::Structure { .. } => Vec::new(),
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            EnvironmentEntry::Axiom { .. } => "axiom",
            EnvironmentEntry::Definition { .. } => "definition",
            EnvironmentEntry::Theorem { .. } => "theorem",
            EnvironmentEntry::Opaque { .. } => "opaque",
            EnvironmentEntry::Inductive { .. } => "inductive",
            EnvironmentEntry::Structure { .. } => "structure",
            EnvironmentEntry::Quotient { .. } => "quotient",
        }
    }
}

/// A proof together with the complete environment it needs, rebuilt and re-checked from nothing
/// but the declared base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCertificate {
    /// [`ENVIRONMENT_SCHEMA`] for original entry kinds, or [`OPAQUE_ENVIRONMENT_SCHEMA`] with opaque declarations.
    pub schema: String,
    /// The prelude/kernel version this certificate was minted against.
    pub prelude_version: String,
    pub base: EnvironmentBase,
    pub conversion: ConversionDiscipline,
    /// The term table, children before parents.
    pub nodes: Vec<TermNode>,
    /// The declarations, in dependency order: every name an entry cites is the base's or an
    /// earlier entry's.
    pub entries: Vec<EnvironmentEntry>,
    /// The name of the definition or theorem this certificate proves.
    pub target: String,
    /// Source binder annotations of declarations' leading telescopes, by name. Presentation
    /// metadata: it changes no judgement, and a reader of the LOGOS document is told which
    /// annotations the fully explicit kernel terms do not write.
    pub binder_kinds: BTreeMap<String, Vec<BinderKind>>,
}

/// A binder annotation, as Lean's `BinderInfo` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinderKind {
    Explicit,
    Implicit,
    StrictImplicit,
    Instance,
}

impl BinderKind {
    fn of(kind: crate::elaborate::ParamKind) -> Self {
        use crate::elaborate::ParamKind;
        match kind {
            ParamKind::Explicit => BinderKind::Explicit,
            ParamKind::Implicit => BinderKind::Implicit,
            ParamKind::StrictImplicit => BinderKind::StrictImplicit,
            ParamKind::Instance => BinderKind::Instance,
        }
    }
    fn param_kind(self) -> crate::elaborate::ParamKind {
        use crate::elaborate::ParamKind;
        match self {
            BinderKind::Explicit => ParamKind::Explicit,
            BinderKind::Implicit => ParamKind::Implicit,
            BinderKind::StrictImplicit => ParamKind::StrictImplicit,
            BinderKind::Instance => ParamKind::Instance,
        }
    }
}

/// Why an environment certificate was refused. Each code is one kind of refusal, so a caller — and
/// a tamper test — can say not only THAT a certificate failed but which check refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnvironmentErrorCode {
    /// The document names a schema this checker does not implement.
    UnsupportedSchema,
    /// The certificate was minted against another prelude.
    IncompatiblePrelude,
    /// The document is not a well-formed certificate (bad JSON, a dangling or forward row).
    Malformed,
    /// A declared limit was exceeded before any checking work began.
    ResourceLimit,
    /// Declarations cite each other in a cycle.
    DependencyCycle,
    /// A declaration cites a name that is not declared before it, or checking left an obligation.
    UnresolvedObligation,
    /// A declaration takes a name the base owns, a reserved name, or an earlier entry's name.
    TrustedNameReplacement,
    /// The kernel refuses a declaration.
    IllTyped,
    /// A declaration checks only under cumulative subtyping, where exact conversion was declared.
    InexactConversion,
    /// The two kernels disagree.
    IndependentDisagreement,
    /// The target is missing or has no proof.
    TargetNotProved,
    /// Evidence the certificate must carry is missing (an opaque theorem's proof).
    MissingEvidence,
    /// A construct this checker does not rebuild.
    Unsupported,
}

impl EnvironmentErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            EnvironmentErrorCode::UnsupportedSchema => "unsupported_schema",
            EnvironmentErrorCode::IncompatiblePrelude => "incompatible_prelude",
            EnvironmentErrorCode::Malformed => "malformed",
            EnvironmentErrorCode::ResourceLimit => "resource_limit",
            EnvironmentErrorCode::DependencyCycle => "dependency_cycle",
            EnvironmentErrorCode::UnresolvedObligation => "unresolved_obligation",
            EnvironmentErrorCode::TrustedNameReplacement => "trusted_name_replacement",
            EnvironmentErrorCode::IllTyped => "ill_typed",
            EnvironmentErrorCode::InexactConversion => "inexact_conversion",
            EnvironmentErrorCode::IndependentDisagreement => "independent_disagreement",
            EnvironmentErrorCode::TargetNotProved => "target_not_proved",
            EnvironmentErrorCode::MissingEvidence => "missing_evidence",
            EnvironmentErrorCode::Unsupported => "unsupported",
        }
    }
}

/// A refusal: its code and a sentence saying what was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentError {
    pub code: EnvironmentErrorCode,
    pub message: String,
}

impl EnvironmentError {
    pub fn new(code: EnvironmentErrorCode, message: impl Into<String>) -> Self {
        EnvironmentError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvironmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

/// Limits enforced on a certificate BEFORE any term is built or checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvironmentLimits {
    /// Rows in the term table.
    pub max_nodes: usize,
    /// Declarations.
    pub max_entries: usize,
    /// Depth of any term. The kernel refuses deeper terms anyway; refusing here refuses before
    /// work.
    pub max_depth: usize,
    /// The unfolded (tree) size of all roots together.
    pub max_tree_nodes: u64,
    /// Bytes in any one name.
    pub max_name_bytes: usize,
}

impl Default for EnvironmentLimits {
    fn default() -> Self {
        EnvironmentLimits {
            max_nodes: 2_000_000,
            max_entries: 100_000,
            max_depth: crate::type_checker::MAX_TERM_DEPTH,
            max_tree_nodes: 20_000_000,
            max_name_bytes: 4096,
        }
    }
}

/// How one declaration fared under the second, independent kernel ([`crate::recheck`](mod@crate::recheck)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndependentVerdict {
    /// The re-checker inferred a type definitionally equal to the declared one.
    Agreed,
    /// The term lies outside the re-checker's covered fragment. Checked once, not twice.
    Unsupported(String),
}

/// A rebuilt environment: the checked context and what the checks found.
pub struct RebuiltEnvironment {
    /// The context, rebuilt from the base with every entry kernel-checked.
    pub context: Context,
    pub target: String,
    /// The target's universe parameters, type and proof.
    pub target_level_params: Vec<String>,
    pub target_type: Term,
    pub target_proof: Term,
    /// Whether the target is a theorem whose proof may unfold during conversion.
    pub target_is_theorem: bool,
    /// Whether the target is a checked opaque value or proof whose body stays sealed.
    pub target_is_opaque: bool,
    /// Every checked declaration term under the second kernel, by declaration name.
    pub independent: Vec<(String, IndependentVerdict)>,
    /// The transitive assumptions of the target's statement AND proof — `#print axioms`.
    pub axioms: AxiomUsage,
    /// Each assumption the environment declares, with its universe parameters and type.
    pub assumptions: Vec<(String, Vec<String>, Term)>,
    /// The checked proofs of the environment's opaque theorems — evidence the context does not
    /// publish, kept for re-emission.
    pub proofs: BTreeMap<String, Term>,
    /// The maximum term depth in the certificate, which sized the stack the rebuild needs.
    pub depth: usize,
}

impl RebuiltEnvironment {
    /// `true` when every checked term was corroborated by the independent kernel.
    pub fn independently_agreed(&self) -> bool {
        self.independent
            .iter()
            .all(|(_, v)| *v == IndependentVerdict::Agreed)
    }
}

/// The terms of a certificate's table, admitted and interned, extracted root by root on demand.
struct DecodedTable {
    store: crate::term_store::TermStore,
    ids: Vec<crate::term_store::TermId>,
    depth: usize,
}

impl DecodedTable {
    fn term(&self, row: NodeRef) -> Result<Term, EnvironmentError> {
        let id = *self.ids.get(row as usize).ok_or_else(|| {
            EnvironmentError::new(
                EnvironmentErrorCode::Malformed,
                format!("term row {row} does not exist"),
            )
        })?;
        Ok(self.store.extract(id))
    }
}

fn malformed(message: impl Into<String>) -> EnvironmentError {
    EnvironmentError::new(EnvironmentErrorCode::Malformed, message)
}

fn limit(message: impl Into<String>) -> EnvironmentError {
    EnvironmentError::new(EnvironmentErrorCode::ResourceLimit, message)
}

fn ill_typed(message: impl Into<String>) -> EnvironmentError {
    EnvironmentError::new(EnvironmentErrorCode::IllTyped, message)
}

/// The base context, built by the checker itself.
fn base_context(base: EnvironmentBase) -> Context {
    if base == EnvironmentBase::StandardLibrary {
        StandardLibrary::context().clone()
    } else {
        Context::new()
    }
}

fn declared(ctx: &Context, name: &str) -> bool {
    ctx.get_global(name).is_some() || ctx.poly_declaration(name).is_some()
}

/// Every global name a term cites, without native recursion.
fn cited_names(term: &Term) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut work = vec![term];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(n) => {
                out.insert(n.clone());
            }
            Term::Const { name, .. } => {
                out.insert(name.clone());
            }
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
        }
    }
    out
}

impl EnvironmentCertificate {
    /// Admit the term table under `limits` — row count, name sizes, backward-only references,
    /// depth and unfolded size — and intern it. No term is built before every limit has been
    /// checked.
    fn decode(&self, limits: &EnvironmentLimits) -> Result<DecodedTable, EnvironmentError> {
        if self.nodes.len() > limits.max_nodes {
            return Err(limit(format!(
                "{} term rows exceed the limit of {}",
                self.nodes.len(),
                limits.max_nodes
            )));
        }
        if self.entries.len() > limits.max_entries {
            return Err(limit(format!(
                "{} declarations exceed the limit of {}",
                self.entries.len(),
                limits.max_entries
            )));
        }
        let long_name = |n: &str| n.len() > limits.max_name_bytes;
        let mut depth: Vec<usize> = Vec::with_capacity(self.nodes.len());
        let mut size: Vec<u64> = Vec::with_capacity(self.nodes.len());
        for (row, node) in self.nodes.iter().enumerate() {
            let mut children: Vec<NodeRef> = Vec::new();
            let mut names: Vec<&str> = Vec::new();
            match node {
                TermNode::Sort(_) | TermNode::Lit(_) | TermNode::Hole => {}
                TermNode::Var(n) | TermNode::Global(n) | TermNode::Const(n, _) => names.push(n),
                TermNode::Pi(n, a, b) | TermNode::Lambda(n, a, b) => {
                    names.push(n);
                    children.extend([*a, *b]);
                }
                TermNode::App(a, b) => children.extend([*a, *b]),
                TermNode::Match(d, m, cases) => {
                    children.extend([*d, *m]);
                    children.extend(cases.iter().copied());
                }
                TermNode::Fix(n, b) => {
                    names.push(n);
                    children.push(*b);
                }
                TermNode::MutualFix(defs, index) => {
                    if *index >= defs.len() {
                        return Err(malformed(format!(
                            "term row {row}: mutual fixpoint index out of range"
                        )));
                    }
                    for (n, b) in defs {
                        names.push(n);
                        children.push(*b);
                    }
                }
                TermNode::Let(n, a, b, c) => {
                    names.push(n);
                    children.extend([*a, *b, *c]);
                }
            }
            if let Some(n) = names.iter().find(|n| long_name(n)) {
                return Err(limit(format!(
                    "term row {row}: a {}-byte name exceeds the limit",
                    n.len()
                )));
            }
            let mut d = 0usize;
            let mut s = 1u64;
            for child in children {
                if child as usize >= row {
                    return Err(malformed(format!(
                        "term row {row} cites row {child}: rows cite only EARLIER rows, so a forward or \
                         self reference is a cycle, never a term"
                    )));
                }
                d = d.max(depth[child as usize]);
                s = s.saturating_add(size[child as usize]);
            }
            let d = d + 1;
            if d > limits.max_depth {
                return Err(limit(format!(
                    "term row {row} has depth {d}, beyond the limit of {}",
                    limits.max_depth
                )));
            }
            depth.push(d);
            size.push(s);
        }
        let mut total = 0u64;
        for entry in &self.entries {
            for row in entry.roots() {
                let row_size = *size.get(row as usize).ok_or_else(|| {
                    malformed(format!(
                        "a {} cites term row {row}, which does not exist",
                        entry.kind_name()
                    ))
                })?;
                total = total.saturating_add(row_size);
            }
        }
        if total > limits.max_tree_nodes {
            return Err(limit(format!(
                "the declarations unfold to {total} term nodes, beyond the limit of {}",
                limits.max_tree_nodes
            )));
        }
        for entry in &self.entries {
            for name in entry.names() {
                if long_name(name) {
                    return Err(limit(format!(
                        "a {}-byte declaration name exceeds the limit",
                        name.len()
                    )));
                }
            }
        }
        let mut store = crate::term_store::TermStore::new();
        let mut ids = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let at = |r: &NodeRef| ids[*r as usize];
            let id = match node {
                TermNode::Sort(u) => store.mk_sort(u),
                TermNode::Var(n) => store.mk_var(n),
                TermNode::Global(n) => store.mk_global(n),
                TermNode::Const(n, levels) => store.mk_const(n, levels),
                TermNode::Pi(n, a, b) => {
                    let (a, b) = (at(a), at(b));
                    store.mk_pi(n, a, b)
                }
                TermNode::Lambda(n, a, b) => {
                    let (a, b) = (at(a), at(b));
                    store.mk_lambda(n, a, b)
                }
                TermNode::App(a, b) => {
                    let (a, b) = (at(a), at(b));
                    store.mk_app(a, b)
                }
                TermNode::Match(d, m, cases) => {
                    let cases: Vec<_> = cases.iter().map(at).collect();
                    let (d, m) = (at(d), at(m));
                    store.mk_match(d, m, &cases)
                }
                TermNode::Fix(n, b) => {
                    let b = at(b);
                    store.mk_fix(n, b)
                }
                TermNode::MutualFix(defs, index) => {
                    let defs: Vec<(&str, _)> =
                        defs.iter().map(|(n, b)| (n.as_str(), at(b))).collect();
                    store.mk_mutual_fix(&defs, *index)
                }
                TermNode::Let(n, a, b, c) => {
                    let (a, b, c) = (at(a), at(b), at(c));
                    store.mk_let(n, a, b, c)
                }
                TermNode::Lit(l) => store.mk_lit(l),
                TermNode::Hole => store.mk_hole(),
            };
            ids.push(id);
        }
        Ok(DecodedTable {
            store,
            ids,
            depth: depth.iter().copied().max().unwrap_or(0),
        })
    }

    /// The deepest term in the certificate, after admission. A caller sizes the stack of the
    /// thread that runs [`EnvironmentCertificate::rebuild`] from it.
    pub fn admitted_depth(&self, limits: &EnvironmentLimits) -> Result<usize, EnvironmentError> {
        self.decode(limits).map(|t| t.depth)
    }

    /// The order of the entries' citations, refused when they form a cycle — checked before any
    /// kernel work, so a cycle is named as one rather than surfacing as a missing name.
    fn check_acyclic(&self, table: &DecodedTable) -> Result<(), EnvironmentError> {
        let mut owner: HashMap<String, usize> = HashMap::new();
        for (i, entry) in self.entries.iter().enumerate() {
            for name in entry.names() {
                owner.entry(name.to_string()).or_insert(i);
            }
        }
        let mut edges: Vec<BTreeSet<usize>> = Vec::with_capacity(self.entries.len());
        for (i, entry) in self.entries.iter().enumerate() {
            let mut out = BTreeSet::new();
            let mut cited = BTreeSet::new();
            for row in entry.roots() {
                cited.extend(cited_names(&table.term(row)?));
            }
            if let EnvironmentEntry::Structure {
                inductive,
                projections,
            } = entry
            {
                cited.insert(inductive.clone());
                cited.extend(projections.iter().cloned());
            }
            if let EnvironmentEntry::Quotient {
                equality: Some(eq), ..
            } = entry
            {
                cited.insert(eq.clone());
            }
            for name in cited {
                if let Some(&j) = owner.get(&name) {
                    if j != i {
                        out.insert(j);
                    }
                }
            }
            edges.push(out);
        }
        // Iterative three-colour depth-first search.
        let mut colour = vec![0u8; self.entries.len()];
        for start in 0..self.entries.len() {
            if colour[start] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, Vec<usize>)> =
                vec![(start, edges[start].iter().copied().collect())];
            colour[start] = 1;
            while let Some((node, pending)) = stack.last_mut() {
                if let Some(next) = pending.pop() {
                    match colour[next] {
                        0 => {
                            colour[next] = 1;
                            let children = edges[next].iter().copied().collect();
                            stack.push((next, children));
                        }
                        1 => {
                            let cycle: Vec<String> = stack
                                .iter()
                                .skip_while(|(n, _)| *n != next)
                                .flat_map(|(n, _)| {
                                    self.entries[*n]
                                        .names()
                                        .into_iter()
                                        .take(1)
                                        .map(str::to_string)
                                })
                                .collect();
                            return Err(EnvironmentError::new(
                                EnvironmentErrorCode::DependencyCycle,
                                format!(
                                    "declarations cite each other in a cycle: {}",
                                    cycle.join(" → ")
                                ),
                            ));
                        }
                        _ => {}
                    }
                } else {
                    colour[*node] = 2;
                    stack.pop();
                }
            }
        }
        Ok(())
    }

    /// Rebuild the environment from the declared base, kernel-checking every entry, and return the
    /// checked context with the target's statement and proof.
    ///
    /// Runs on the caller's thread; a caller holding a deep certificate sizes that thread's stack
    /// with [`EnvironmentCertificate::admitted_depth`] and
    /// [`crate::term_store::stack_bytes_for_depth`] first.
    pub fn rebuild(
        &self,
        limits: &EnvironmentLimits,
    ) -> Result<RebuiltEnvironment, EnvironmentError> {
        self.rebuild_observing_definitions(limits, &mut |_, _, _, _, _| {})
    }

    /// Observe each checked transparent definition before registering it. The supplied context
    /// contains only preceding entries, so a source comparison cannot unfold the definition being
    /// compared or a later declaration. Observations never authorize kernel admission.
    pub(crate) fn rebuild_observing_definitions(
        &self,
        limits: &EnvironmentLimits,
        observe: &mut dyn FnMut(&Context, &str, &[String], &Term, &Term),
    ) -> Result<RebuiltEnvironment, EnvironmentError> {
        self.rebuild_observing_values_impl(limits, observe, false)
    }

    /// Observe checked transparent and opaque values in their preceding contexts. Opaque
    /// declarations remain sealed; theorem proof bodies are intentionally not observed.
    pub(crate) fn rebuild_observing_values(
        &self,
        limits: &EnvironmentLimits,
        observe: &mut dyn FnMut(&Context, &str, &[String], &Term, &Term),
    ) -> Result<RebuiltEnvironment, EnvironmentError> {
        self.rebuild_observing_values_impl(limits, observe, true)
    }

    fn rebuild_observing_values_impl(
        &self,
        limits: &EnvironmentLimits,
        observe: &mut dyn FnMut(&Context, &str, &[String], &Term, &Term),
        include_opaque: bool,
    ) -> Result<RebuiltEnvironment, EnvironmentError> {
        self.check_schema()?;
        check_prelude_version(&self.prelude_version)?;
        #[cfg(not(feature = "lean"))]
        if self.conversion == ConversionDiscipline::Exact {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::Unsupported,
                "exact conversion is checked by the `lean` feature's exact typing, which this build lacks",
            ));
        }
        let table = self.decode(limits)?;
        for row in 0..self.nodes.len() {
            if matches!(self.nodes[row], TermNode::Hole) {
                return Err(ill_typed(format!(
                    "term row {row} is a hole: a certificate carries no unelaborated term"
                )));
            }
        }
        self.check_acyclic(&table)?;
        let mut rebuild = Rebuild {
            certificate: self,
            table: &table,
            ctx: base_context(self.base),
            introduced: BTreeSet::new(),
            independent: Vec::new(),
            assumptions: Vec::new(),
            proofs: BTreeMap::new(),
        };
        for entry in &self.entries {
            rebuild.entry(entry, observe, include_opaque)?;
        }
        let mut ctx = rebuild.ctx;
        for (name, kinds) in &self.binder_kinds {
            if !rebuild.introduced.contains(name) {
                return Err(malformed(format!(
                    "binder annotations name {name}, which the certificate does not declare"
                )));
            }
            let ty = ctx
                .poly_declaration(name)
                .map(|(_, t)| t.clone())
                .or_else(|| ctx.get_global(name).cloned());
            let mut telescope = 0usize;
            let mut at = ty.as_ref();
            while let Some(Term::Pi { body_type, .. }) = at {
                telescope += 1;
                at = Some(body_type);
            }
            if kinds.len() > telescope {
                return Err(malformed(format!(
                    "{name} has {} binder annotations for a telescope of {telescope}",
                    kinds.len()
                )));
            }
            ctx.set_binder_kinds(name, kinds.iter().map(|k| k.param_kind()).collect());
        }
        let obligations = ctx.pending_obligations();
        if !obligations.is_empty() {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::UnresolvedObligation,
                format!(
                    "the rebuilt environment still owes {} obligation(s): {:?}",
                    obligations.len(),
                    obligations
                ),
            ));
        }
        if !ctx.refused_declarations().is_empty() {
            return Err(ill_typed(format!(
                "the kernel refused {:?}",
                ctx.refused_declarations()
            )));
        }
        let (target_level_params, target_type, target_proof, target_is_theorem, target_is_opaque) = self
            .entries
            .iter()
            .find_map(|entry| match entry {
                EnvironmentEntry::Definition { name, level_params, ty, body } if *name == self.target => {
                    Some((level_params.clone(), *ty, *body, false, false))
                }
                EnvironmentEntry::Theorem { name, level_params, ty, proof } if *name == self.target => {
                    Some((level_params.clone(), *ty, *proof, true, false))
                }
                EnvironmentEntry::Opaque { name, level_params, ty, body } if *name == self.target => {
                    Some((level_params.clone(), *ty, *body, false, true))
                }
                _ => None,
            })
            .ok_or_else(|| {
                let assumed = self.entries.iter().any(|e| matches!(e, EnvironmentEntry::Axiom { name, .. } if *name == self.target));
                EnvironmentError::new(
                    EnvironmentErrorCode::TargetNotProved,
                    if assumed {
                        format!("the target {} is an assumption: a certificate proves its target, it does not assume it", self.target)
                    } else {
                        format!("the certificate declares no proved target named {}", self.target)
                    },
                )
            })?;
        let target_type = table.term(target_type)?;
        let target_proof = table.term(target_proof)?;
        let mut axioms = axiom_dependencies(&ctx, &target_type);
        axioms.absorb(axiom_dependencies(&ctx, &target_proof));
        // An opaque target publishes only its header; its proof's closure is its checked evidence.
        if let Some(evidence) = axiom_dependencies_of(&ctx, &self.target) {
            axioms.absorb(evidence);
        }
        Ok(RebuiltEnvironment {
            context: ctx,
            target: self.target.clone(),
            target_level_params,
            target_type,
            target_proof,
            target_is_theorem,
            target_is_opaque,
            independent: rebuild.independent,
            axioms,
            assumptions: rebuild.assumptions,
            proofs: rebuild.proofs,
            depth: table.depth,
        })
    }
}

/// The state of one rebuild.
struct Rebuild<'a> {
    certificate: &'a EnvironmentCertificate,
    table: &'a DecodedTable,
    ctx: Context,
    introduced: BTreeSet<String>,
    independent: Vec<(String, IndependentVerdict)>,
    assumptions: Vec<(String, Vec<String>, Term)>,
    proofs: BTreeMap<String, Term>,
}

impl Rebuild<'_> {
    fn exact(&self) -> bool {
        self.certificate.conversion == ConversionDiscipline::Exact
    }

    /// Refuse a name the base owns, a reserved name, or a name an earlier entry introduced.
    fn claim_name(
        &mut self,
        name: &str,
        canonical_nat: bool,
        quotient: bool,
    ) -> Result<(), EnvironmentError> {
        let replacement =
            |why: String| EnvironmentError::new(EnvironmentErrorCode::TrustedNameReplacement, why);
        if name.is_empty() {
            return Err(malformed("a declaration has an empty name"));
        }
        if self.introduced.contains(name) {
            return Err(replacement(format!("{name} is declared twice")));
        }
        if declared(&self.ctx, name) {
            return Err(replacement(format!(
                "{name} is already declared by the trusted base and may not be replaced"
            )));
        }
        let is_nat = matches!(name, "Nat" | "Zero" | "Succ");
        let is_quotient = name.starts_with("kernel.quot.");
        if is_quotient && !quotient {
            return Err(replacement(format!(
                "{name} is a certified quotient primitive and only a quotient entry may install it"
            )));
        }
        if is_nat && !canonical_nat {
            return Err(replacement(format!("{name} is the kernel's literal Nat and only its canonical inductive may declare it")));
        }
        // Every standard library name carries native meaning somewhere in the kernel (a reduction
        // keyed on `Quot_lift`, a decision procedure keyed on `le`, …), so a kernel-based
        // environment may not declare one itself — except the canonical `Nat`, `Zero` and `Succ`,
        // which the kernel's literals denote and which an imported environment must bring in their
        // canonical shape.
        if self.certificate.base == EnvironmentBase::Kernel
            && !is_nat
            && !is_quotient
            && declared(StandardLibrary::context(), name)
        {
            return Err(replacement(format!(
                "{name} carries native meaning in the standard library; an imported environment may not declare it"
            )));
        }
        self.introduced.insert(name.to_string());
        Ok(())
    }

    /// Every name the terms cite must already be declared (or be one of `own`).
    fn require_declared(
        &self,
        subject: &str,
        terms: &[&Term],
        own: &BTreeSet<String>,
    ) -> Result<(), EnvironmentError> {
        for term in terms {
            for name in cited_names(term) {
                if !own.contains(&name) && !declared(&self.ctx, &name) {
                    return Err(EnvironmentError::new(
                        EnvironmentErrorCode::UnresolvedObligation,
                        format!("{subject} cites {name}, which is not declared before it"),
                    ));
                }
            }
        }
        Ok(())
    }

    fn level_telescope(
        &self,
        subject: &str,
        params: &[String],
        terms: &[&Term],
    ) -> Result<(), EnvironmentError> {
        let bound: BTreeSet<&str> = params.iter().map(String::as_str).collect();
        if bound.len() != params.len() {
            return Err(ill_typed(format!("{subject} repeats a universe parameter")));
        }
        for term in terms {
            for level in crate::term::free_level_vars(term) {
                if !bound.contains(level.as_str()) {
                    return Err(ill_typed(format!(
                        "{subject} uses the unbound universe parameter {level}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The type of `term` under the declared discipline.
    fn infer(&self, term: &Term) -> Result<Term, EnvironmentError> {
        if self.exact() {
            #[cfg(feature = "lean")]
            {
                return crate::interface::lean::lean_type(&self.ctx, term).map_err(|verdict| {
                    let cumulative = crate::infer_type(&self.ctx, term).is_ok();
                    EnvironmentError::new(
                        if cumulative {
                            EnvironmentErrorCode::InexactConversion
                        } else {
                            EnvironmentErrorCode::IllTyped
                        },
                        verdict.reason().message().to_string(),
                    )
                });
            }
        }
        crate::infer_type(&self.ctx, term).map_err(|e| ill_typed(e.to_string()))
    }

    /// `inferred` must be the declared type: definitionally equal under the exact discipline, a
    /// subtype under the cumulative one.
    fn conforms(
        &self,
        subject: &str,
        inferred: &Term,
        declared_ty: &Term,
    ) -> Result<(), EnvironmentError> {
        let ok = if self.exact() {
            crate::type_checker::def_eq(&self.ctx, inferred, declared_ty)
        } else {
            is_subtype(&self.ctx, inferred, declared_ty)
        };
        if ok {
            return Ok(());
        }
        if self.exact() && is_subtype(&self.ctx, inferred, declared_ty) {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::InexactConversion,
                format!("{subject} has its declared type only by cumulative subtyping"),
            ));
        }
        Err(ill_typed(format!(
            "{subject} does not have its declared type"
        )))
    }

    /// A declared type must itself be a type.
    fn check_type(&self, subject: &str, ty: &Term) -> Result<(), EnvironmentError> {
        let sort = self.infer(ty)?;
        let sort = crate::normalize_checked(&self.ctx, &sort).map_err(|e| {
            ill_typed(format!(
                "{subject}: the sort of its type did not normalize: {e:?}"
            ))
        })?;
        if matches!(sort, Term::Sort(_)) {
            Ok(())
        } else {
            Err(ill_typed(format!(
                "{subject}: its declared type is not a type"
            )))
        }
    }

    /// Re-check `term` with the second kernel and record the verdict under `subject`.
    fn independently(
        &mut self,
        subject: &str,
        term: &Term,
        expected: &Term,
    ) -> Result<(), EnvironmentError> {
        match crate::recheck(&self.ctx, term) {
            Ok(inferred) => {
                if crate::type_checker::def_eq(&self.ctx, &inferred, expected) || is_subtype(&self.ctx, &inferred, expected) {
                    self.independent.push((subject.to_string(), IndependentVerdict::Agreed));
                    Ok(())
                } else {
                    Err(EnvironmentError::new(
                        EnvironmentErrorCode::IndependentDisagreement,
                        format!("{subject}: the independent kernel infers a different type"),
                    ))
                }
            }
            Err(crate::ReCheckError::Unsupported(why)) => {
                self.independent.push((subject.to_string(), IndependentVerdict::Unsupported(why.into())));
                Ok(())
            }
            Err(crate::ReCheckError::Ill(why)) => Err(EnvironmentError::new(
                EnvironmentErrorCode::IndependentDisagreement,
                format!("{subject}: the independent kernel refuses what the primary kernel accepted: {why}"),
            )),
        }
    }

    fn entry(
        &mut self,
        entry: &EnvironmentEntry,
        observe: &mut dyn FnMut(&Context, &str, &[String], &Term, &Term),
        include_opaque: bool,
    ) -> Result<(), EnvironmentError> {
        match entry {
            EnvironmentEntry::Axiom {
                name,
                level_params,
                ty,
            } => {
                let ty = self.table.term(*ty)?;
                self.claim_name(name, false, false)?;
                self.require_declared(name, &[&ty], &BTreeSet::new())?;
                self.level_telescope(name, level_params, &[&ty])?;
                self.check_type(name, &ty)?;
                let sort =
                    crate::infer_type(&self.ctx, &ty).map_err(|e| ill_typed(e.to_string()))?;
                self.independently(name, &ty, &sort)?;
                self.ctx
                    .add_universe_poly_declaration(name, level_params.clone(), ty.clone())
                    .map_err(|e| ill_typed(format!("{name}: {e}")))?;
                self.assumptions
                    .push((name.clone(), level_params.clone(), ty));
                Ok(())
            }
            EnvironmentEntry::Definition {
                name,
                level_params,
                ty,
                body,
            } => {
                let (ty, body) = (self.table.term(*ty)?, self.table.term(*body)?);
                self.claim_name(name, false, false)?;
                self.require_declared(name, &[&ty, &body], &BTreeSet::new())?;
                self.level_telescope(name, level_params, &[&ty, &body])?;
                self.check_type(name, &ty)?;
                let inferred = self.infer(&body)?;
                self.conforms(name, &inferred, &ty)?;
                self.independently(name, &body, &ty)?;
                observe(&self.ctx, name, level_params, &ty, &body);
                let registered = if level_params.is_empty() {
                    self.ctx.declare(name, ty, body)
                } else {
                    self.ctx
                        .declare_universe_poly(name, level_params.clone(), ty, body)
                };
                registered.map_err(|e| ill_typed(format!("{name}: {e}")))
            }
            EnvironmentEntry::Theorem {
                name,
                level_params,
                ty,
                proof,
            } => {
                let (ty, proof) = (self.table.term(*ty)?, self.table.term(*proof)?);
                self.claim_name(name, false, false)?;
                self.require_declared(name, &[&ty, &proof], &BTreeSet::new())?;
                self.level_telescope(name, level_params, &[&ty, &proof])?;
                self.check_type(name, &ty)?;
                let inferred = self.infer(&proof)?;
                self.conforms(name, &inferred, &ty)?;
                self.independently(name, &proof, &ty)?;
                // A theorem is rebuilt as the importer publishes it: checked, and unfoldable
                // as Lean's kernel unfolds a theorem.
                self.ctx
                    .declare_theorem_checked(name, level_params.clone(), ty, proof.clone())
                    .map_err(|e| ill_typed(format!("{name}: {e}")))?;
                self.proofs.insert(name.clone(), proof);
                Ok(())
            }
            EnvironmentEntry::Opaque {
                name,
                level_params,
                ty,
                body,
            } => {
                let (ty, body) = (self.table.term(*ty)?, self.table.term(*body)?);
                self.claim_name(name, false, false)?;
                self.require_declared(name, &[&ty, &body], &BTreeSet::new())?;
                self.level_telescope(name, level_params, &[&ty, &body])?;
                self.check_type(name, &ty)?;
                let inferred = self.infer(&body)?;
                self.conforms(name, &inferred, &ty)?;
                self.independently(name, &body, &ty)?;
                if include_opaque {
                    observe(&self.ctx, name, level_params, &ty, &body);
                }
                self.ctx
                    .declare_opaque_checked(name, level_params.clone(), ty, body.clone())
                    .map_err(|e| ill_typed(format!("{name}: {e}")))?;
                self.proofs.insert(name.clone(), body);
                Ok(())
            }
            EnvironmentEntry::Inductive {
                level_params,
                types,
            } => self.inductive(level_params, types),
            EnvironmentEntry::Structure {
                inductive,
                projections,
            } => self.structure(inductive, projections),
            EnvironmentEntry::Quotient {
                primitive,
                level_params,
                ty,
                equality,
            } => {
                let kind = primitive.kind();
                let name = kind.name();
                let ty = self.table.term(*ty)?;
                self.claim_name(name, false, true)?;
                let equality_name = equality.clone().unwrap_or_default();
                if kind == crate::quotient::Kind::Lift && equality.is_none() {
                    return Err(malformed("the quotient lift names no equality"));
                }
                let levels: Vec<Universe> =
                    level_params.iter().cloned().map(Universe::Var).collect();
                let expected = crate::quotient::signature(kind, &levels, &equality_name)
                    .map_err(|e| ill_typed(format!("{name}: {e}")))?;
                if !crate::type_checker::def_eq(&self.ctx, &ty, &expected) {
                    return Err(ill_typed(format!(
                        "{name} does not carry the canonical quotient signature"
                    )));
                }
                crate::quotient::install(&mut self.ctx, kind, level_params.clone(), &equality_name)
                    .map_err(|e| ill_typed(format!("{name}: {e}")))
            }
        }
    }

    fn inductive(
        &mut self,
        level_params: &[String],
        types: &[InductiveType],
    ) -> Result<(), EnvironmentError> {
        if types.is_empty() {
            return Err(malformed("an inductive entry declares no type"));
        }
        let mut own = BTreeSet::new();
        let mut decoded: Vec<(String, Term, usize, Vec<(String, Term)>)> = Vec::new();
        for member in types {
            let arity = self.table.term(member.arity)?;
            let constructors = member
                .constructors
                .iter()
                .map(|c| Ok((c.name.clone(), self.table.term(c.ty)?)))
                .collect::<Result<Vec<_>, EnvironmentError>>()?;
            decoded.push((member.name.clone(), arity, member.num_params, constructors));
        }
        let canonical_nat = self.canonical_nat(level_params, &decoded)?;
        for (name, _, _, constructors) in &decoded {
            self.claim_name(name, canonical_nat, false)?;
            own.insert(name.clone());
            for (c, _) in constructors {
                self.claim_name(c, canonical_nat, false)?;
                own.insert(c.clone());
            }
        }
        for (name, arity, _, constructors) in &decoded {
            let mut terms: Vec<&Term> = vec![arity];
            terms.extend(constructors.iter().map(|(_, t)| t));
            self.require_declared(name, &terms, &own)?;
            self.level_telescope(name, level_params, &terms)?;
        }
        // Types of arities and constructors are checked under the declared discipline in a scratch
        // context carrying the block's headers; the registration below re-decides the whole block.
        let mut headers = self.ctx.clone();
        for (name, arity, num_params, _) in &decoded {
            if level_params.is_empty() {
                headers.add_indexed_inductive(name, arity.clone(), *num_params);
            } else {
                headers.add_universe_poly_inductive(
                    name,
                    level_params.to_vec(),
                    arity.clone(),
                    *num_params,
                );
            }
        }
        let outer = std::mem::replace(&mut self.ctx, headers);
        let checked = decoded
            .iter()
            .try_for_each(|(name, arity, _, constructors)| {
                self.check_type(name, arity)?;
                constructors
                    .iter()
                    .try_for_each(|(c, ty)| self.check_type(c, ty))
            });
        self.ctx = outer;
        checked?;
        let block: Vec<crate::MutualInductive> = decoded
            .iter()
            .map(
                |(name, arity, num_params, constructors)| crate::MutualInductive {
                    name: name.clone(),
                    sort: arity.clone(),
                    num_params: *num_params,
                    constructors: constructors.clone(),
                },
            )
            .collect();
        let registered = if level_params.is_empty() {
            self.ctx.add_mutual_inductives(&block)
        } else {
            self.ctx.add_universe_poly_inductives(level_params, &block)
        };
        registered.map_err(|e| ill_typed(format!("inductive {}: {e}", decoded[0].0)))?;
        for (name, arity, _, constructors) in &decoded {
            let sort = crate::infer_type(&self.ctx, arity).map_err(|e| ill_typed(e.to_string()))?;
            self.independently(name, arity, &sort)?;
            for (c, ty) in constructors {
                let sort =
                    crate::infer_type(&self.ctx, ty).map_err(|e| ill_typed(e.to_string()))?;
                self.independently(c, ty, &sort)?;
            }
        }
        Ok(())
    }

    /// Whether this block is the kernel's literal `Nat`. A block that uses any of the names `Nat`,
    /// `Zero`, `Succ` must be exactly `Nat : Type 0` with `Zero : Nat` and `Succ : Nat → Nat`, or it
    /// is refused: the kernel's literals and their arithmetic denote that datatype.
    fn canonical_nat(
        &self,
        level_params: &[String],
        decoded: &[(String, Term, usize, Vec<(String, Term)>)],
    ) -> Result<bool, EnvironmentError> {
        let reserved = |n: &str| matches!(n, "Nat" | "Zero" | "Succ");
        let uses = decoded
            .iter()
            .any(|(n, _, _, cs)| reserved(n) || cs.iter().any(|(c, _)| reserved(c)));
        if !uses {
            return Ok(false);
        }
        let nat = Term::Global("Nat".into());
        let canonical = level_params.is_empty()
            && decoded.len() == 1
            && decoded[0].0 == "Nat"
            && decoded[0].2 == 0
            && decoded[0].1 == Term::Sort(Universe::Type(0))
            && decoded[0].3.len() == 2
            && decoded[0].3[0].0 == "Zero"
            && decoded[0].3[0].1 == nat
            && decoded[0].3[1].0 == "Succ"
            && matches!(&decoded[0].3[1].1, Term::Pi { param_type, body_type, .. }
                if **param_type == nat && **body_type == nat);
        if canonical {
            Ok(true)
        } else {
            Err(EnvironmentError::new(
                EnvironmentErrorCode::TrustedNameReplacement,
                "a block using the names Nat, Zero or Succ must be the canonical Nat: Nat : Type 0, Zero : Nat, Succ : Nat → Nat",
            ))
        }
    }

    /// Structure η for `inductive` through `projections`, through the kernel's one door
    /// ([`Context::register_structure_checked`]). The certificate adds only its own scoping rule:
    /// the inductive and every projection it names must have been introduced by this certificate,
    /// never borrowed from the base.
    fn structure(
        &mut self,
        inductive: &str,
        projections: &[String],
    ) -> Result<(), EnvironmentError> {
        if self.ctx.is_inductive(inductive) && !self.introduced.contains(inductive) {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::UnresolvedObligation,
                format!(
                    "structure η names {inductive}, which is not an inductive declared before it"
                ),
            ));
        }
        if let Some(borrowed) = projections
            .iter()
            .find(|p| declared(&self.ctx, p) && !self.introduced.contains(p.as_str()))
        {
            return Err(ill_typed(format!(
                "{borrowed} is not a definition declared before the structure"
            )));
        }
        self.ctx
            .register_structure_checked(inductive, projections)
            .map_err(structure_refusal)
    }
}

/// A refused structure η, classified: naming no inductive leaves an obligation unresolved, a
/// second registration would replace trusted η, and every other refusal is ill-typed evidence.
fn structure_refusal(refusal: crate::StructureRefusal) -> EnvironmentError {
    use crate::StructureRefusal as R;
    let code = match refusal {
        R::NotAnInductive { .. } => EnvironmentErrorCode::UnresolvedObligation,
        R::AlreadyAStructure { .. } => EnvironmentErrorCode::TrustedNameReplacement,
        _ => EnvironmentErrorCode::IllTyped,
    };
    EnvironmentError::new(code, refusal.to_string())
}

// ---------------------------------------------------------------------------
// Capturing an environment certificate from a checked context.
// ---------------------------------------------------------------------------

/// What to capture: the target, the context it was checked in, and the evidence the context does
/// not keep.
/// A checked theorem published transparently (as Lean publishes one): its level telescope,
/// statement and proof. Ordinary definitions carry no certification record and are not theorems.
fn checked_theorem<'c>(ctx: &'c Context, name: &str) -> Option<(&'c [String], &'c Term, &'c Term)> {
    ctx.checked_opaque_dependencies(name)?;
    ctx.get_universe_poly(name)
        .map(|(params, ty, proof)| (params.as_slice(), ty, proof))
}

pub struct CaptureRequest<'a> {
    pub context: &'a Context,
    pub base: EnvironmentBase,
    pub conversion: ConversionDiscipline,
    pub target: &'a str,
    /// The checked proofs of the opaque theorems in the target's closure (the target's own
    /// included, when it is one). A checked context publishes only an opaque theorem's header.
    pub opaque_proofs: &'a BTreeMap<String, Term>,
}

/// One unit of the dependency order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Item {
    Declaration(String),
    Block(Vec<String>),
}

/// The term table under construction: each distinct row written once, children first.
struct TableWriter {
    nodes: Vec<TermNode>,
    index: HashMap<String, NodeRef>,
    /// The first term the format cannot carry (a non-finite float has no JSON spelling).
    refusal: Option<String>,
}

impl TableWriter {
    fn row(&mut self, node: TermNode) -> NodeRef {
        let key = match serde_json::to_string(&node) {
            Ok(key) => key,
            Err(e) => {
                self.refusal.get_or_insert_with(|| {
                    format!("a term the certificate format cannot carry: {e}")
                });
                format!("{node:?}")
            }
        };
        if let Some(&r) = self.index.get(&key) {
            return r;
        }
        let r = self.nodes.len() as NodeRef;
        self.nodes.push(node);
        self.index.insert(key, r);
        r
    }

    /// Write `term`, post-order and without native recursion.
    fn write(&mut self, term: &Term) -> NodeRef {
        enum Task<'a> {
            Enter(&'a Term),
            Exit(&'a Term),
        }
        let mut work = vec![Task::Enter(term)];
        let mut out: Vec<NodeRef> = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Task::Enter(t) => {
                    work.push(Task::Exit(t));
                    let mut children: Vec<&Term> = Vec::new();
                    match t {
                        Term::Sort(_)
                        | Term::Var(_)
                        | Term::Global(_)
                        | Term::Const { .. }
                        | Term::Lit(_)
                        | Term::Hole => {}
                        Term::Pi {
                            param_type,
                            body_type,
                            ..
                        } => children.extend([&**param_type, &**body_type]),
                        Term::Lambda {
                            param_type, body, ..
                        } => children.extend([&**param_type, &**body]),
                        Term::App(f, a) => children.extend([&**f, &**a]),
                        Term::Match {
                            discriminant,
                            motive,
                            cases,
                        } => {
                            children.extend([&**discriminant, &**motive]);
                            children.extend(cases.iter());
                        }
                        Term::Fix { body, .. } => children.push(body),
                        Term::MutualFix { defs, .. } => {
                            children.extend(defs.iter().map(|(_, b)| b))
                        }
                        Term::Let {
                            ty, value, body, ..
                        } => children.extend([&**ty, &**value, &**body]),
                        Term::Annotated { term, .. } => children.push(term),
                    }
                    for child in children.into_iter().rev() {
                        work.push(Task::Enter(child));
                    }
                }
                Task::Exit(t) => {
                    let mut take = |n: usize| -> Vec<NodeRef> { out.split_off(out.len() - n) };
                    let node = match t {
                        Term::Sort(u) => TermNode::Sort(u.clone()),
                        Term::Var(n) => TermNode::Var(n.clone()),
                        Term::Global(n) => TermNode::Global(n.clone()),
                        Term::Const { name, levels } => {
                            TermNode::Const(name.clone(), levels.clone())
                        }
                        Term::Lit(l) => TermNode::Lit(l.clone()),
                        Term::Hole => TermNode::Hole,
                        Term::Pi { param, .. } => {
                            let k = take(2);
                            TermNode::Pi(param.clone(), k[0], k[1])
                        }
                        Term::Lambda { param, .. } => {
                            let k = take(2);
                            TermNode::Lambda(param.clone(), k[0], k[1])
                        }
                        Term::App(..) => {
                            let k = take(2);
                            TermNode::App(k[0], k[1])
                        }
                        Term::Match { cases, .. } => {
                            let k = take(2 + cases.len());
                            TermNode::Match(k[0], k[1], k[2..].to_vec())
                        }
                        Term::Fix { name, .. } => {
                            let k = take(1);
                            TermNode::Fix(name.clone(), k[0])
                        }
                        Term::MutualFix { defs, index } => {
                            let k = take(defs.len());
                            TermNode::MutualFix(
                                defs.iter().map(|(n, _)| n.clone()).zip(k).collect(),
                                *index,
                            )
                        }
                        Term::Let { name, .. } => {
                            let k = take(3);
                            TermNode::Let(name.clone(), k[0], k[1], k[2])
                        }
                        // A certificate carries kernel meaning only: an annotated term's row is its
                        // term's, already on top of `out`.
                        Term::Annotated { .. } => continue,
                    };
                    let r = self.row(node);
                    out.push(r);
                }
            }
        }
        out.pop().expect("one root")
    }
}

impl EnvironmentCertificate {
    fn check_schema(&self) -> Result<(), EnvironmentError> {
        if self.schema != OPAQUE_ENVIRONMENT_SCHEMA && self.schema != ENVIRONMENT_SCHEMA {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::UnsupportedSchema,
                format!("unsupported certificate schema {:?}; this checker reads {OPAQUE_ENVIRONMENT_SCHEMA} and {ENVIRONMENT_SCHEMA}", self.schema),
            ));
        }
        if self.schema == ENVIRONMENT_SCHEMA
            && self
                .entries
                .iter()
                .any(|entry| matches!(entry, EnvironmentEntry::Opaque { .. }))
        {
            return Err(EnvironmentError::new(
                EnvironmentErrorCode::UnsupportedSchema,
                format!("an opaque entry requires {OPAQUE_ENVIRONMENT_SCHEMA}; {ENVIRONMENT_SCHEMA} has no opaque entry"),
            ));
        }
        Ok(())
    }

    /// Capture the target's complete dependency closure from a checked context, in dependency
    /// order. Every captured declaration is exactly what the context checked; capture decides
    /// nothing, and [`EnvironmentCertificate::rebuild`] re-decides everything.
    pub fn capture(
        request: &CaptureRequest<'_>,
    ) -> Result<EnvironmentCertificate, EnvironmentError> {
        let ctx = request.context;
        let base = base_context(request.base);
        let in_base = |n: &str| declared(&base, n);
        let unsupported =
            |why: String| EnvironmentError::new(EnvironmentErrorCode::Unsupported, why);
        // A StandardLibrary certificate reconstructs this semantic base, not
        // a same-spelled set of assumptions with revoked primitive identities.
        if request.base == EnvironmentBase::StandardLibrary
            && !crate::quotient::same_prelude_base(ctx, &base)
        {
            return Err(unsupported(
                "source context does not retain the standard quotient primitive identities".into(),
            ));
        }
        let item_of = |name: &str| -> Result<Option<Item>, EnvironmentError> {
            if request.base == EnvironmentBase::Kernel
                && crate::quotient::prelude_kind(ctx, name).is_some()
            {
                return Err(unsupported(
                    "legacy prelude quotient identities require the StandardLibrary base".into(),
                ));
            }
            if in_base(name) {
                return Ok(None);
            }
            let inductive = if ctx.is_inductive(name) {
                Some(name.to_string())
            } else {
                ctx.constructor_inductive(name).map(str::to_string)
            };
            if let Some(ind) = inductive {
                let members = ctx
                    .mutual_block_of(&ind)
                    .map(|b| b.to_vec())
                    .unwrap_or_else(|| vec![ind.clone()]);
                return Ok(Some(Item::Block(members)));
            }
            if declared(ctx, name) {
                return Ok(Some(Item::Declaration(name.to_string())));
            }
            Err(EnvironmentError::new(
                EnvironmentErrorCode::UnresolvedObligation,
                format!("the context does not declare {name}"),
            ))
        };
        // The terms of one item, for dependency discovery.
        let item_terms = |item: &Item| -> Result<Vec<Term>, EnvironmentError> {
            match item {
                Item::Declaration(name) => {
                    let mut terms = Vec::new();
                    if let Some((_, ty, proof)) = checked_theorem(ctx, name) {
                        terms.extend([ty.clone(), proof.clone()]);
                    } else if let Some((_, ty, body)) = ctx.get_universe_poly(name) {
                        terms.extend([ty.clone(), body.clone()]);
                    } else if let Some(body) = ctx.get_definition_body(name) {
                        terms.push(body.clone());
                        terms.extend(ctx.get_definition_type(name).cloned());
                    } else {
                        let ty = ctx
                            .poly_declaration(name)
                            .map(|(_, t)| t.clone())
                            .or_else(|| ctx.get_global(name).cloned())
                            .expect("declared");
                        terms.push(ty);
                        if ctx.checked_opaque_dependencies(name).is_some() {
                            let proof = request.opaque_proofs.get(name).ok_or_else(|| EnvironmentError::new(
                                EnvironmentErrorCode::MissingEvidence,
                                format!("the checked proof of the opaque theorem {name} was not retained"),
                            ))?;
                            terms.push(proof.clone());
                        }
                        if let Some(certificate) = ctx.quotient_primitives.get(name) {
                            for dependency in &certificate.dependencies {
                                terms.push(Term::Global(dependency.clone()));
                            }
                        }
                    }
                    Ok(terms)
                }
                Item::Block(members) => {
                    let mut terms = Vec::new();
                    for member in members {
                        terms.extend(ctx.get_global(member).cloned());
                        terms.extend(
                            ctx.get_constructors(member)
                                .into_iter()
                                .map(|(_, t)| t.clone()),
                        );
                    }
                    Ok(terms)
                }
            }
        };
        let own_names = |item: &Item| -> BTreeSet<String> {
            match item {
                Item::Declaration(n) => BTreeSet::from([n.clone()]),
                Item::Block(members) => members
                    .iter()
                    .flat_map(|m| {
                        std::iter::once(m.clone()).chain(
                            ctx.get_constructors(m)
                                .into_iter()
                                .map(|(c, _)| c.to_string()),
                        )
                    })
                    .collect(),
            }
        };

        let mut order: Vec<Item> = Vec::new();
        let mut done: BTreeSet<Item> = BTreeSet::new();
        let mut on_stack: BTreeSet<Item> = BTreeSet::new();
        let root = item_of(request.target)?.ok_or_else(|| {
            EnvironmentError::new(
                EnvironmentErrorCode::TargetNotProved,
                format!("{} belongs to the base", request.target),
            )
        })?;
        {
            let mut stack: Vec<(Item, Vec<Item>)> = Vec::new();
            let children = |item: &Item| -> Result<Vec<Item>, EnvironmentError> {
                let own = own_names(item);
                let mut names = BTreeSet::new();
                for term in item_terms(item)? {
                    names.extend(cited_names(&term));
                }
                let mut out = Vec::new();
                for name in names.into_iter().filter(|n| !own.contains(n)) {
                    if let Some(child) = item_of(&name)? {
                        if !out.contains(&child) {
                            out.push(child);
                        }
                    }
                }
                out.reverse();
                Ok(out)
            };
            on_stack.insert(root.clone());
            let first = children(&root)?;
            stack.push((root, first));
            while let Some((item, pending)) = stack.last_mut() {
                if let Some(next) = pending.pop() {
                    if done.contains(&next) {
                        continue;
                    }
                    if on_stack.contains(&next) {
                        return Err(EnvironmentError::new(
                            EnvironmentErrorCode::DependencyCycle,
                            format!("{next:?} depends on itself through {item:?}"),
                        ));
                    }
                    on_stack.insert(next.clone());
                    let grandchildren = children(&next)?;
                    stack.push((next, grandchildren));
                } else {
                    let (item, _) = stack.pop().expect("nonempty");
                    on_stack.remove(&item);
                    done.insert(item.clone());
                    order.push(item.clone());
                    // A structure's projections belong right after it, so its η is in force for
                    // every later declaration exactly as it was when they were checked. They cite
                    // only the structure and what the structure itself cites.
                    if let Item::Block(members) = &item {
                        for member in members {
                            let Some(info) = ctx.struct_info(member) else {
                                continue;
                            };
                            for projection in &info.projections {
                                let projection = Item::Declaration(projection.clone());
                                if done.contains(&projection) {
                                    continue;
                                }
                                if let Some(missing) = children(&projection)?
                                    .into_iter()
                                    .find(|c| !done.contains(c))
                                {
                                    return Err(unsupported(format!(
                                        "the projection {projection:?} of {member} cites {missing:?}, outside its structure"
                                    )));
                                }
                                done.insert(projection.clone());
                                order.push(projection);
                            }
                        }
                    }
                }
            }
        }

        let mut writer = TableWriter {
            nodes: Vec::new(),
            index: HashMap::new(),
            refusal: None,
        };
        let mut entries = Vec::new();
        let mut pending_structures: Vec<(String, Vec<String>)> = Vec::new();
        for item in &order {
            match item {
                Item::Block(members) => {
                    for member in members {
                        if let Some(info) = ctx.struct_info(member) {
                            pending_structures.push((member.clone(), info.projections.clone()));
                        }
                    }
                    let level_params = ctx
                        .poly_declaration(&members[0])
                        .map(|(p, _)| p.to_vec())
                        .unwrap_or_default();
                    let types = members
                        .iter()
                        .map(|member| InductiveType {
                            name: member.clone(),
                            arity: writer.write(ctx.get_global(member).expect("inductive")),
                            num_params: ctx.inductive_num_params(member),
                            constructors: ctx
                                .get_constructors(member)
                                .into_iter()
                                .map(|(name, ty)| ConstructorClaim {
                                    name: name.to_string(),
                                    ty: writer.write(ty),
                                })
                                .collect(),
                        })
                        .collect();
                    entries.push(EnvironmentEntry::Inductive {
                        level_params,
                        types,
                    });
                }
                Item::Declaration(name) => {
                    if let Some(certificate) = ctx.quotient_primitives.get(name) {
                        let (params, ty) = ctx.poly_declaration(name).expect("quotient primitive");
                        let equality = if certificate.kind == crate::quotient::Kind::Lift {
                            certificate
                                .dependencies
                                .iter()
                                .find(|d| ctx.is_inductive(d))
                                .cloned()
                        } else {
                            None
                        };
                        entries.push(EnvironmentEntry::Quotient {
                            primitive: QuotientPrimitive::of(certificate.kind),
                            level_params: params.to_vec(),
                            ty: writer.write(ty),
                            equality,
                        });
                    } else if let Some((level_params, ty, proof)) = checked_theorem(ctx, name) {
                        entries.push(EnvironmentEntry::Theorem {
                            name: name.clone(),
                            level_params: level_params.to_vec(),
                            ty: writer.write(ty),
                            proof: writer.write(proof),
                        });
                    } else if let Some((params, ty, body)) = ctx.get_universe_poly(name) {
                        entries.push(EnvironmentEntry::Definition {
                            name: name.clone(),
                            level_params: params.clone(),
                            ty: writer.write(ty),
                            body: writer.write(body),
                        });
                    } else if let Some(body) = ctx.get_definition_body(name) {
                        let ty = ctx.get_definition_type(name).expect("definition type");
                        entries.push(EnvironmentEntry::Definition {
                            name: name.clone(),
                            level_params: Vec::new(),
                            ty: writer.write(ty),
                            body: writer.write(body),
                        });
                    } else {
                        let level_params = ctx
                            .poly_declaration(name)
                            .map(|(p, _)| p.to_vec())
                            .unwrap_or_default();
                        let ty = ctx
                            .poly_declaration(name)
                            .map(|(_, t)| t)
                            .or_else(|| ctx.get_global(name))
                            .expect("declared");
                        if ctx.checked_opaque_dependencies(name).is_some() {
                            let proof = &request.opaque_proofs[name];
                            entries.push(EnvironmentEntry::Opaque {
                                name: name.clone(),
                                level_params,
                                ty: writer.write(ty),
                                body: writer.write(proof),
                            });
                        } else {
                            entries.push(EnvironmentEntry::Axiom {
                                name: name.clone(),
                                level_params,
                                ty: writer.write(ty),
                            });
                        }
                    }
                    // A structure's η follows its last projection.
                    pending_structures.retain(|(inductive, projections)| {
                        let placed = projections.iter().all(|p| {
                            entries.iter().any(|e| matches!(e, EnvironmentEntry::Definition { name, .. } if name == p))
                        });
                        if placed {
                            entries.push(EnvironmentEntry::Structure {
                                inductive: inductive.clone(),
                                projections: projections.clone(),
                            });
                        }
                        !placed
                    });
                }
            }
        }
        if let Some((inductive, _)) = pending_structures.first() {
            return Err(unsupported(format!(
                "the structure η of {inductive} could not be ordered after its projections"
            )));
        }
        if let Some(why) = writer.refusal {
            return Err(unsupported(why));
        }
        let binder_kinds = entries
            .iter()
            .flat_map(|e| e.names())
            .filter_map(|name| {
                ctx.binder_kinds(name).map(|kinds| {
                    (
                        name.to_string(),
                        kinds.iter().copied().map(BinderKind::of).collect(),
                    )
                })
            })
            .collect();
        Ok(EnvironmentCertificate {
            schema: if entries
                .iter()
                .any(|entry| matches!(entry, EnvironmentEntry::Opaque { .. }))
            {
                OPAQUE_ENVIRONMENT_SCHEMA
            } else {
                ENVIRONMENT_SCHEMA
            }
            .to_string(),
            prelude_version: PRELUDE_VERSION.to_string(),
            base: request.base,
            conversion: request.conversion,
            nodes: writer.nodes,
            entries,
            target: request.target.to_string(),
            binder_kinds,
        })
    }
}

// ---------------------------------------------------------------------------
// The independent re-check tool's door.
// ---------------------------------------------------------------------------

/// How an independent re-check of one certificate document ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecheckOutcome {
    /// Checked by the kernel and corroborated by the independent kernel.
    Verified,
    /// Checked by the kernel; the independent kernel does not cover every term. Never a pass.
    SingleChecked,
    /// Refused.
    Rejected,
}

/// What `recheck <certificate.json>` reports: its outcome and one line per finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecheckReport {
    pub outcome: RecheckOutcome,
    pub lines: Vec<String>,
}

/// Re-check a certificate document of any kind, dispatched by its schema.
///
/// An environment certificate is rebuilt on a stack sized for its deepest term. A certificate by
/// itself establishes that its proof checks against the environment it carries; that the statement
/// is the Lean source's is established by replaying the whole package, which binds it to the export.
pub fn recheck_document(json: &str, limits: &EnvironmentLimits) -> RecheckReport {
    let rejected = |line: String| RecheckReport {
        outcome: RecheckOutcome::Rejected,
        lines: vec![line],
    };
    let certificate = match read_certificate(json) {
        Ok(certificate) => certificate,
        Err(error) => return rejected(format!("REJECTED: {error}")),
    };
    match certificate {
        AnyCertificate::Proof(certificate) => match recheck(&certificate) {
            Ok(()) => RecheckReport {
                outcome: RecheckOutcome::Verified,
                lines: vec!["VERIFIED".into()],
            },
            Err(e) => rejected(format!("REJECTED: {e}")),
        },
        AnyCertificate::Theorem(certificate) => {
            let out = recheck_theorem(&certificate);
            let axioms = out
                .axioms
                .axioms
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            match (out.rechecks, out.two_kernel_agreement) {
                (true, true) => RecheckReport {
                    outcome: RecheckOutcome::Verified,
                    lines: vec![format!("VERIFIED theorem {} (two kernels agree; axioms: [{axioms}])", certificate.theorem)],
                },
                (true, false) => RecheckReport {
                    outcome: RecheckOutcome::SingleChecked,
                    lines: vec![format!(
                        "SINGLE-CHECKED theorem {}: the independent kernel does not cover this proof (axioms: [{axioms}])",
                        certificate.theorem
                    )],
                },
                _ => rejected(format!("REJECTED: {}", out.error.unwrap_or_default())),
            }
        }
        AnyCertificate::Environment(certificate) => {
            let depth = match certificate.admitted_depth(limits) {
                Ok(depth) => depth,
                Err(error) => return rejected(format!("REJECTED: {error}")),
            };
            let rungs = depth
                .saturating_mul(3)
                .saturating_add(crate::term_store::INLINE_DEPTH);
            let report = std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .name("recheck-environment".into())
                    .stack_size(crate::term_store::stack_bytes_for_depth(rungs))
                    .spawn_scoped(scope, || environment_report(&certificate, limits))
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|p| std::panic::resume_unwind(p))
                    })
            });
            report.unwrap_or_else(|e| {
                rejected(format!("REJECTED: cannot reserve the re-check stack: {e}"))
            })
        }
    }
}

fn environment_report(
    certificate: &EnvironmentCertificate,
    limits: &EnvironmentLimits,
) -> RecheckReport {
    let rebuilt = match certificate.rebuild(limits) {
        Ok(rebuilt) => rebuilt,
        Err(error) => {
            return RecheckReport {
                outcome: RecheckOutcome::Rejected,
                lines: vec![format!("REJECTED: {error}")],
            }
        }
    };
    let unsupported: Vec<String> = rebuilt
        .independent
        .iter()
        .filter_map(|(name, verdict)| match verdict {
            IndependentVerdict::Unsupported(why) => Some(format!("{name} ({why})")),
            IndependentVerdict::Agreed => None,
        })
        .collect();
    let outcome = if unsupported.is_empty() {
        RecheckOutcome::Verified
    } else {
        RecheckOutcome::SingleChecked
    };
    let mut lines = vec![
        format!(
            "{} environment certificate for {}: {} declarations rebuilt from the {} base under {} conversion",
            if outcome == RecheckOutcome::Verified { "VERIFIED" } else { "SINGLE-CHECKED" },
            certificate.target,
            certificate.entries.len(),
            match certificate.base { EnvironmentBase::Kernel => "bare kernel", EnvironmentBase::StandardLibrary => "standard library" },
            match certificate.conversion { ConversionDiscipline::Exact => "exact", ConversionDiscipline::Cumulative => "cumulative" },
        ),
        format!("primary check: passed ({} {})", if rebuilt.target_is_theorem { "theorem" } else { "definition" }, rebuilt.target),
        if unsupported.is_empty() {
            format!("independent re-check: passed ({} terms)", rebuilt.independent.len())
        } else {
            format!("independent re-check: unsupported for {}", unsupported.join("; "))
        },
        format!(
            "assumptions (transitive, statement and proof): [{}]",
            rebuilt.axioms.axioms.iter().chain(rebuilt.axioms.unresolved.iter()).cloned().collect::<Vec<_>>().join(", ")
        ),
        "statement binding: not established by a certificate alone; replay its package to bind the statement to the Lean source".into(),
    ];
    if !rebuilt.axioms.unresolved.is_empty() {
        lines.push(format!("unresolved: {:?}", rebuilt.axioms.unresolved));
    }
    RecheckReport { outcome, lines }
}
