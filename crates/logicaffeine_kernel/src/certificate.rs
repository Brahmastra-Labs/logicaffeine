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
//! bogus axiom (e.g. a free proof of `False`). You trust the seven ring axioms
//! and the type-checker; you do not trust the certificate's provenance.
//!
//! This module is available under the `serde` feature.

use serde::{Deserialize, Serialize};

use crate::prelude::StandardLibrary;
use crate::{infer_type, is_subtype, Context, KernelError, Term};

/// The prelude/axiom-set version a certificate was produced against. A
/// re-checker may refuse certificates minted against an axiom set it does not
/// recognise. Bump this whenever the trusted prelude changes.
pub const PRELUDE_VERSION: &str = "logos-coc-1";

/// A self-contained, re-checkable proof certificate.
///
/// `proof_term` is claimed to have type `claimed_type` in the standard prelude
/// context. Nothing else is trusted: the re-checker supplies its own axioms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            proof_term,
            claimed_type,
            prelude_version: PRELUDE_VERSION.to_string(),
        }
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

    // Rebuild the trusted axiom context ourselves — never trust a context the
    // certificate might carry.
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // The term must type-check at all...
    let inferred = infer_type(&ctx, &cert.proof_term)?;

    // ...and its type must be the claimed proposition.
    if is_subtype(&ctx, &inferred, &cert.claimed_type) {
        Ok(())
    } else {
        Err(KernelError::CertificationError(format!(
            "certificate term proves a different proposition: has type {:?}, claims {:?}",
            inferred, cert.claimed_type
        )))
    }
}
