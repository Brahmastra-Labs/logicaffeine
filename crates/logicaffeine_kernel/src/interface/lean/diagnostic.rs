//! Typed, machine-readable reasons for every refusal the Lean reader gives.
//!
//! A [`Verdict`](super::Verdict) says *whether* the kernel refused (`Reject`: the input is wrong;
//! `Decline`: the input may be right but is not accepted). A [`Reason`] says *why*, as a stable
//! [`DiagnosticCode`] a program can dispatch on, plus the human message and, where known, the export
//! line and the Lean declaration it arose in. The message is still what the reader always printed,
//! so the benchmark protocol's text output is unchanged.
//!
//! Every code belongs to exactly one [`Outcome`] of the `logos-lean-result/1` contract, and every
//! outcome has one exit code, so a consumer can dispatch on the outcome and drill into the code.

use std::fmt;

/// Why the reader (or the intake around it) refused. Stable across releases: the wire name is
/// [`DiagnosticCode::as_str`], and a renamed wire name is a protocol break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticCode {
    /// Well-formed input whose mathematics the kernel refutes: an ill-typed body, a type mismatch,
    /// a positivity violation.
    InvalidProof,
    /// The export itself is malformed: bad JSON, a dangling index, a missing mandatory field.
    MalformedExport,
    /// The export declares a format version this reader does not implement.
    UnsupportedExportVersion,
    /// The export or project was produced by a Lean or exporter outside the pinned profile.
    UnsupportedToolchain,
    /// A record, expression or declaration kind the pinned export format does not define.
    UnknownRecordKind,
    /// A field the pinned export format does not define for its record.
    UnknownField,
    /// A metavariable or free variable: the declaration was never fully elaborated.
    UnresolvedMetavariable,
    /// A declaration the input names but never declares, or a target that is not there.
    MissingDeclaration,
    /// A declaration declared twice.
    DuplicateDeclaration,
    /// A Lean project imports a module that is neither in the project nor in Lean core.
    MissingModule,
    /// A Lean project's modules import each other in a cycle.
    ImportCycle,
    /// Lean itself rejected the project's source (syntax or elaboration), at Lean's location.
    LeanRejected,
    /// The input is a pre-exported artifact: this run did not observe Lean accepting its source.
    LeanAcceptanceNotObserved,
    /// Valid Lean that LOGOS does not yet express faithfully; declined, never approximated.
    UnsupportedFeature,
    /// The independent re-checker did not confirm what the primary kernel accepted.
    RecheckRefused,
    /// The transitive assumption audit could not be completed.
    AuditIncomplete,
    /// The declaration depends on an assumption the policy does not allow.
    DisallowedAssumption,
    /// Lean's `#print axioms` and the LOGOS audit disagree on a declaration's assumptions.
    AssumptionDisagreement,
    /// A declared resource budget was exhausted before a decision.
    ResourceLimit,
    /// The environment failed: a file could not be read, a tool is absent, a thread could not be
    /// reserved.
    Infrastructure,
    /// LOGOS's own artifact — the emitted document, the assembled package, its fresh replay —
    /// failed its self-verification after every check of the user's input passed. The fault is
    /// ours, never the user's mathematics.
    InternalInconsistency,
    /// The command was invoked wrongly; nothing ran.
    Usage,
    /// Not yet classified. `lean_diagnostic_code_lock` drives the number of such sites to zero.
    Unclassified,
}

impl DiagnosticCode {
    pub const ALL: [DiagnosticCode; 23] = [
        DiagnosticCode::InvalidProof,
        DiagnosticCode::MalformedExport,
        DiagnosticCode::UnsupportedExportVersion,
        DiagnosticCode::UnsupportedToolchain,
        DiagnosticCode::UnknownRecordKind,
        DiagnosticCode::UnknownField,
        DiagnosticCode::UnresolvedMetavariable,
        DiagnosticCode::MissingDeclaration,
        DiagnosticCode::DuplicateDeclaration,
        DiagnosticCode::MissingModule,
        DiagnosticCode::ImportCycle,
        DiagnosticCode::LeanRejected,
        DiagnosticCode::LeanAcceptanceNotObserved,
        DiagnosticCode::UnsupportedFeature,
        DiagnosticCode::RecheckRefused,
        DiagnosticCode::AuditIncomplete,
        DiagnosticCode::DisallowedAssumption,
        DiagnosticCode::AssumptionDisagreement,
        DiagnosticCode::ResourceLimit,
        DiagnosticCode::Infrastructure,
        DiagnosticCode::InternalInconsistency,
        DiagnosticCode::Usage,
        DiagnosticCode::Unclassified,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticCode::InvalidProof => "invalid_proof",
            DiagnosticCode::MalformedExport => "malformed_export",
            DiagnosticCode::UnsupportedExportVersion => "unsupported_export_version",
            DiagnosticCode::UnsupportedToolchain => "unsupported_toolchain",
            DiagnosticCode::UnknownRecordKind => "unknown_record_kind",
            DiagnosticCode::UnknownField => "unknown_field",
            DiagnosticCode::UnresolvedMetavariable => "unresolved_metavariable",
            DiagnosticCode::MissingDeclaration => "missing_declaration",
            DiagnosticCode::DuplicateDeclaration => "duplicate_declaration",
            DiagnosticCode::MissingModule => "missing_module",
            DiagnosticCode::ImportCycle => "import_cycle",
            DiagnosticCode::LeanRejected => "lean_rejected",
            DiagnosticCode::LeanAcceptanceNotObserved => "lean_acceptance_not_observed",
            DiagnosticCode::UnsupportedFeature => "unsupported_feature",
            DiagnosticCode::RecheckRefused => "recheck_refused",
            DiagnosticCode::AuditIncomplete => "audit_incomplete",
            DiagnosticCode::DisallowedAssumption => "disallowed_assumption",
            DiagnosticCode::AssumptionDisagreement => "assumption_disagreement",
            DiagnosticCode::ResourceLimit => "resource_limit",
            DiagnosticCode::Infrastructure => "infrastructure",
            DiagnosticCode::InternalInconsistency => "internal_inconsistency",
            DiagnosticCode::Usage => "usage",
            DiagnosticCode::Unclassified => "unclassified",
        }
    }

    /// The code a wire name names.
    pub fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == wire)
    }

    /// The result outcome this code decides. `Usage` precedes any result; `Unclassified` carries no
    /// category of its own (see [`Outcome::of_refusal`]).
    pub fn outcome(self) -> Option<Outcome> {
        use DiagnosticCode as C;
        Some(match self {
            C::InvalidProof
            | C::MalformedExport
            | C::UnresolvedMetavariable
            | C::MissingDeclaration
            | C::DuplicateDeclaration
            | C::MissingModule
            | C::ImportCycle
            | C::LeanRejected => Outcome::InvalidSource,
            C::UnsupportedExportVersion
            | C::UnsupportedToolchain
            | C::UnknownRecordKind
            | C::UnknownField
            | C::LeanAcceptanceNotObserved
            | C::UnsupportedFeature
            | C::RecheckRefused
            | C::AuditIncomplete => Outcome::UnsupportedFeature,
            C::DisallowedAssumption | C::AssumptionDisagreement => Outcome::DisallowedAssumptions,
            C::ResourceLimit => Outcome::ResourceLimit,
            C::Infrastructure | C::InternalInconsistency => Outcome::InfrastructureFailure,
            C::Usage | C::Unclassified => return None,
        })
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of a `logos-lean-result/1` document, with the contract's exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    /// Every stage passed.
    Verified,
    /// A real rejection: the source, the export or the proof is wrong.
    InvalidSource,
    /// Valid input LOGOS does not (yet) carry to a verified result.
    UnsupportedFeature,
    /// The result depends on an assumption outside the policy.
    DisallowedAssumptions,
    /// A declared budget ran out before a decision.
    ResourceLimit,
    /// The environment failed; nothing was decided about the input.
    InfrastructureFailure,
}

impl Outcome {
    pub const ALL: [Outcome; 6] = [
        Outcome::Verified,
        Outcome::InvalidSource,
        Outcome::UnsupportedFeature,
        Outcome::DisallowedAssumptions,
        Outcome::ResourceLimit,
        Outcome::InfrastructureFailure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Verified => "verified",
            Outcome::InvalidSource => "invalid_source",
            Outcome::UnsupportedFeature => "unsupported_feature",
            Outcome::DisallowedAssumptions => "disallowed_assumptions",
            Outcome::ResourceLimit => "resource_limit",
            Outcome::InfrastructureFailure => "infrastructure_failure",
        }
    }

    /// The contract's exit code: 0 verified · 1 invalid source · 2 unsupported feature ·
    /// 3 disallowed assumptions · 4 resource limit · 5 infrastructure failure.
    pub fn exit_code(self) -> i32 {
        match self {
            Outcome::Verified => 0,
            Outcome::InvalidSource => 1,
            Outcome::UnsupportedFeature => 2,
            Outcome::DisallowedAssumptions => 3,
            Outcome::ResourceLimit => 4,
            Outcome::InfrastructureFailure => 5,
        }
    }

    /// The outcome of a refusal: its code's outcome, or — for a code without one — the verdict
    /// kind's benchmark meaning (a rejection is invalid source, a decline an unsupported feature).
    pub fn of_refusal(code: DiagnosticCode, rejected: bool) -> Outcome {
        code.outcome().unwrap_or(if rejected {
            Outcome::InvalidSource
        } else {
            Outcome::UnsupportedFeature
        })
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A refusal's code and message, and where it arose when that is known. Dereferences to the
/// message, so a reason reads as the text the reader has always reported.
#[derive(Clone, PartialEq, Eq)]
pub struct Reason {
    code: DiagnosticCode,
    message: String,
    line: Option<usize>,
    declaration: Option<String>,
    stage: Option<&'static str>,
}

impl Reason {
    pub fn new(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Reason {
            code,
            message: message.into(),
            line: None,
            declaration: None,
            stage: None,
        }
    }
    pub fn code(&self) -> DiagnosticCode {
        self.code
    }
    /// The receipt stage the refusal arose at, when the code alone does not say: a resource limit
    /// reached by the independent kernel belongs to `independent_recheck`, not to translation.
    pub fn stage(&self) -> Option<&'static str> {
        self.stage
    }
    /// The same reason, recorded as arising at receipt stage `stage`.
    pub fn at_stage(self, stage: &'static str) -> Self {
        Reason {
            stage: Some(stage),
            ..self
        }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    /// The 1-based line of the input the refusal arose at.
    pub fn line(&self) -> Option<usize> {
        self.line
    }
    /// The Lean declaration (dotted name) the refusal arose in.
    pub fn declaration(&self) -> Option<&str> {
        self.declaration.as_deref()
    }
    /// The same reason, its message prefixed with where it arose.
    pub fn located(self, prefix: impl fmt::Display) -> Self {
        Reason {
            message: format!("{prefix}: {}", self.message),
            ..self
        }
    }
    /// The same reason and message, recorded as arising at `line`. The innermost line wins: a
    /// reason already located keeps its line.
    pub fn at_line(self, line: usize) -> Self {
        Reason {
            line: self.line.or(Some(line)),
            ..self
        }
    }
    /// The same reason and message, recorded as arising in `declaration`. The innermost wins.
    pub fn in_declaration(self, declaration: impl Into<String>) -> Self {
        let declaration = self
            .declaration
            .clone()
            .or_else(|| Some(declaration.into()));
        Reason {
            declaration,
            ..self
        }
    }
}

impl std::ops::Deref for Reason {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Prints as the message string did, so `{verdict:?}` output is unchanged for protocol consumers.
impl fmt::Debug for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.message, f)
    }
}
