//! The Lean → LOGOS pipeline, one named stage at a time, and the benchmark protocol's dispatch.
//!
//! Parsing and validation ([`intake`](super::intake)), environment import and checking (the reader),
//! audit construction ([`ImportReport`](super::ImportReport)), assumption policy, emission
//! ([`emit`](super::emit)) and packaging ([`package`](super::package)) stay separate here, each
//! recording its own [`receipt`](super::receipt) stage, so a result is never one unexplained
//! Boolean (competition rule R1). The reader interleaves translation and checking per declaration;
//! that is reported honestly — a declaration the kernel refused leaves `translated` `not_run`, since
//! the closure's translation never finished.
//!
//! Everything here is pure: no file, process or clock. Reading the export, running Lean, publishing
//! the package and replaying it in a fresh process are the command's (`largo lean`).

use super::intake::{self, STAGE_A};
use super::receipt::{Receipt, StageStatus, STAGES};
use super::{
    emit, native_decide, package, reference, DiagnosticCode, ImportReport, Outcome, Verdict,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const RESULT_SCHEMA: &str = "logos-lean-result/1";

/// One typed, located diagnostic of a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    /// The receipt stage it concerns.
    pub stage: &'static str,
    /// The Lean declaration (dotted name) it concerns.
    pub declaration: Option<String>,
    /// The 1-based line of the input it concerns.
    pub line: Option<usize>,
    pub message: String,
    outcome: Outcome,
}

impl Diagnostic {
    /// A diagnostic of a classified code. An unclassified or usage code is an unsupported feature.
    pub fn new(stage: &'static str, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            stage,
            declaration: None,
            line: None,
            message: message.into(),
            outcome: Outcome::of_refusal(code, false),
        }
    }
    /// The diagnostic a refusal carries, with its location.
    pub fn of_refusal(stage: &'static str, verdict: &Verdict) -> Self {
        let reason = verdict.reason();
        Diagnostic {
            code: reason.code(),
            stage,
            declaration: reason.declaration().map(str::to_string),
            line: reason.line(),
            message: reason.message().to_string(),
            outcome: verdict.outcome(),
        }
    }
    pub fn with_declaration(mut self, declaration: impl Into<String>) -> Self {
        self.declaration = Some(declaration.into());
        self
    }
    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }
    /// The outcome this diagnostic decides when its stage is the first that did not pass.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code.as_str(),
            "stage": self.stage,
            "declaration": self.declaration,
            "line": self.line,
            "message": self.message,
        })
    }
}

/// Information the result does not carry from the Lean input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    pub category: String,
    pub detail: String,
}

/// A published package, as the result names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageRef {
    pub path: String,
    pub manifest_sha256: String,
}

/// What the run observed of Lean itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeanEvidence {
    /// A pre-exported artifact: Lean was not run, so its acceptance is not established.
    NotObserved,
    /// Lean compiled the project in isolation, and `#print axioms` reported `axioms` (Lean names)
    /// for the target.
    Accepted {
        axioms: Vec<String>,
        /// The original Lean sources, by project-relative path (published under `source/`).
        sources: BTreeMap<String, Vec<u8>>,
        /// The pinned toolchain and exporter identities.
        toolchain: String,
        exporter: String,
    },
}

impl LeanEvidence {
    /// What the package records of the producing run.
    fn build_options(&self) -> package::BuildOptions {
        match self {
            LeanEvidence::NotObserved => package::BuildOptions::default(),
            LeanEvidence::Accepted {
                axioms,
                sources,
                toolchain,
                exporter,
            } => package::BuildOptions {
                lean_sources: sources.clone(),
                lean_run: Some(package::LeanRun {
                    toolchain: toolchain.clone(),
                    exporter: exporter.clone(),
                    accepted: true,
                    detail: format!("compiled in isolation; #print axioms: {axioms:?}"),
                }),
            },
        }
    }
}

/// Budgets declared before any work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_export_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_export_bytes: 1 << 30,
        }
    }
}

/// A `logos-lean-result/1` document under construction.
#[derive(Clone, Debug, PartialEq)]
pub struct LeanResult {
    pub receipt: Receipt,
    pub target_lean: String,
    pub target_logos: Option<String>,
    pub assumptions: Vec<String>,
    pub information_losses: Vec<Loss>,
    pub diagnostics: Vec<Diagnostic>,
    /// The importer's audit (`logos-lean-import-report/1`), when the import ran.
    pub audit: Option<Value>,
    pub package: Option<PackageRef>,
}

impl LeanResult {
    /// A result for `target` in which nothing has run.
    pub fn new(target_lean: impl Into<String>) -> Self {
        LeanResult {
            receipt: Receipt::unstarted(),
            target_lean: target_lean.into(),
            target_logos: None,
            assumptions: Vec::new(),
            information_losses: Vec::new(),
            diagnostics: Vec::new(),
            audit: None,
            package: None,
        }
    }

    pub fn status(&self, stage: &str) -> StageStatus {
        self.receipt
            .stage(stage)
            .map(|s| s.status)
            .unwrap_or(StageStatus::NotRun)
    }

    pub fn pass(&mut self, stage: &'static str, detail: impl Into<String>) {
        self.receipt.set(stage, StageStatus::Passed, detail);
    }

    /// Record `stage` as `status` for the reason `diagnostic` gives.
    pub fn mark(&mut self, stage: &'static str, status: StageStatus, diagnostic: Diagnostic) {
        self.receipt.set(stage, status, diagnostic.message.clone());
        self.diagnostics.push(Diagnostic {
            stage,
            ..diagnostic
        });
    }

    /// `stage` failed for the reason `diagnostic` gives.
    pub fn fail(&mut self, stage: &'static str, diagnostic: Diagnostic) {
        self.mark(stage, StageStatus::Failed, diagnostic);
    }

    /// The outcome: the first stage that failed decides it; otherwise every stage passed
    /// (`verified`), or the first stage that did not decides it.
    pub fn outcome(&self) -> Outcome {
        let decided_by = |stage: &str| {
            self.diagnostics
                .iter()
                .find(|d| d.stage == stage)
                .map(Diagnostic::outcome)
                .unwrap_or(Outcome::UnsupportedFeature)
        };
        if let Some(stage) = STAGES
            .iter()
            .find(|s| self.status(s) == StageStatus::Failed)
        {
            return decided_by(stage);
        }
        match STAGES
            .iter()
            .find(|s| self.status(s) != StageStatus::Passed)
        {
            None => Outcome::Verified,
            Some(stage) => decided_by(stage),
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.outcome().exit_code()
    }

    /// Explain a stage that did not run where it decides the outcome: when nothing failed, a
    /// `lean_accepted` stage that was never observed is the reason the result is not verified.
    pub fn finish(&mut self) {
        let failed = STAGES.iter().any(|s| self.status(s) == StageStatus::Failed);
        let lean = "lean_accepted";
        if !failed
            && self.status(lean) == StageStatus::NotRun
            && !self.diagnostics.iter().any(|d| d.stage == lean)
        {
            self.diagnostics.push(Diagnostic::new(
                lean,
                DiagnosticCode::LeanAcceptanceNotObserved,
                "the input is a pre-exported artifact: this run did not observe Lean accept its source, so the \
                 result cannot be verified (import the Lean project with --project to establish it)",
            ));
        }
    }

    pub fn to_json(&self) -> Value {
        let stages: serde_json::Map<String, Value> = STAGES
            .iter()
            .map(|s| (s.to_string(), Value::from(self.status(s).as_str())))
            .collect();
        json!({
            "schema": RESULT_SCHEMA,
            "outcome": self.outcome().as_str(),
            "stages": stages,
            "target": { "lean": self.target_lean, "logos": self.target_logos },
            "assumptions": self.assumptions,
            "information_losses": self.information_losses.iter()
                .map(|l| json!({"category": l.category, "detail": l.detail}))
                .collect::<Vec<_>>(),
            "diagnostics": self.diagnostics.iter().map(Diagnostic::to_json).collect::<Vec<_>>(),
            "package": self.package.as_ref().map(|p| json!({"path": p.path, "manifest_sha256": p.manifest_sha256})),
            "audit": self.audit,
        })
    }
}

/// A result and, when every stage up to packaging passed, the package to publish and replay.
pub struct Checked {
    pub result: LeanResult,
    pub package: Option<package::PackageFiles>,
}

/// The stage a refusal from the import belongs to. The reader interleaves translation and checking,
/// so a refusal that names its stage keeps it (every verdict of the independent kernel, a resource
/// limit included, is the recheck's); otherwise a kernel refutation is the primary check's, a
/// refused independent recheck or audit is the recheck's, and everything else — malformed,
/// unsupported, over budget — stopped the translation.
pub(super) fn import_stage(verdict: &Verdict) -> &'static str {
    if let Some(stage) = verdict.reason().stage() {
        return stage;
    }
    match verdict.reason().code() {
        DiagnosticCode::InvalidProof => "primary_check",
        DiagnosticCode::RecheckRefused | DiagnosticCode::AuditIncomplete => "independent_recheck",
        _ => "translated",
    }
}

/// Run every in-process stage over a Lean export for the dotted Lean name `target`: intake, import
/// and check, independent recheck, assumption policy, emission and package assembly. `fresh_replay`
/// is left to the caller, which must replay the published package in a new process.
pub fn check_export(
    export: &str,
    target: &str,
    evidence: &LeanEvidence,
    limits: &Limits,
) -> Checked {
    let mut result = LeanResult::new(target);
    let package = run_stages(&mut result, export, target, evidence, limits);
    Checked { result, package }
}

fn run_stages(
    result: &mut LeanResult,
    export: &str,
    target: &str,
    evidence: &LeanEvidence,
    limits: &Limits,
) -> Option<package::PackageFiles> {
    if let LeanEvidence::Accepted { .. } = evidence {
        result.pass("lean_accepted", "Lean compiled the project in isolation");
    }
    let kernel_target = match intake::kernel_name(target) {
        Ok(name) => name,
        Err(verdict) => {
            result.fail(
                "translated",
                Diagnostic::of_refusal("translated", &verdict).with_declaration(target),
            );
            return None;
        }
    };
    if export.len() as u64 > limits.max_export_bytes {
        result.fail(
            "translated",
            Diagnostic::new(
                "translated",
                DiagnosticCode::ResourceLimit,
                format!(
                    "the export is {} bytes; the declared budget is {} bytes",
                    export.len(),
                    limits.max_export_bytes
                ),
            ),
        );
        return None;
    }
    let read = match intake::read(export, &STAGE_A) {
        Ok(read) => read,
        Err(verdict) => {
            result.fail("translated", Diagnostic::of_refusal("translated", &verdict));
            return None;
        }
    };
    let closure = match read.closure(&kernel_target) {
        Ok(closure) => closure,
        Err(verdict) => {
            result.fail(
                "translated",
                Diagnostic::of_refusal("translated", &verdict).with_declaration(target),
            );
            return None;
        }
    };

    match super::runtime_refusal::check(export, &kernel_target, &read) {
        Ok(Some(audit)) => {
            result.pass("translated", "the runtime bodies were lowered into a conditional judgment; the originals remain quarantined");
            result.pass("primary_check", "the primary kernel checked the conditional judgment under local assumptions; the unsafe target was not proved or imported");
            result.pass("independent_recheck", "the independent kernel checked the same conditional judgment; no unsafe declaration entered the environment");
            result.audit = Some(audit.to_json());
            result.assumptions = audit.assumptions.clone();
            let disagreement = match evidence {
                LeanEvidence::Accepted { axioms, .. } => {
                    let stated: std::collections::BTreeSet<_> = axioms.iter().collect();
                    let observed: std::collections::BTreeSet<_> =
                        audit.assumptions.iter().collect();
                    stated.len() != axioms.len() || stated != observed
                }
                LeanEvidence::NotObserved => false,
            };
            let code = if disagreement {
                DiagnosticCode::AssumptionDisagreement
            } else {
                DiagnosticCode::DisallowedAssumption
            };
            let detail = if disagreement {
                "Lean's reported assumptions differ from the original source dependency closure of the conditionally checked runtime judgment"
            } else {
                "the runtime body's conditional check requires a disallowed assumption; the original unsafe target is not proved, imported, or published"
            };
            result.fail(
                "assumption_policy",
                Diagnostic::new("assumption_policy", code, detail).with_declaration(target),
            );
            result.diagnostics.extend(
                audit
                    .taint
                    .iter()
                    .filter(|path| {
                        !intake::STAGE_A
                            .allowed_axioms
                            .contains(&path.assumption.as_str())
                    })
                    .map(|path| {
                        Diagnostic::new(
                            "assumption_policy",
                            DiagnosticCode::DisallowedAssumption,
                            format!("taint path to {}: {}", path.assumption, path.render()),
                        )
                        .with_declaration(target)
                    }),
            );
            return None;
        }
        Ok(None) => {}
        Err(verdict) => {
            let stage = import_stage(&verdict);
            result.fail(
                stage,
                Diagnostic::of_refusal(stage, &verdict).with_declaration(target),
            );
            return None;
        }
    }

    let (imported, report) = match super::import_retaining_closure(export, &kernel_target, &closure)
    {
        Ok(imported) => imported,
        Err(verdict) => {
            let stage = import_stage(&verdict);
            result.fail(stage, Diagnostic::of_refusal(stage, &verdict));
            return None;
        }
    };
    result.pass(
        "translated",
        format!("{} declarations of the target's closure", closure.len()),
    );
    result.pass(
        "primary_check",
        "the LOGOS kernel accepted every declaration of the export",
    );
    // The kernel core (`dag`, the check the Lean Kernel Arena runs and `largo lean check` runs) decides the whole export
    // too, as the LOGOS kernel read it (each native decision re-derived, `native_decide`): a refusal of what the LOGOS
    // kernel accepted is a disagreement between the kernels, and the import stops; a decline is no verdict, and the
    // recheck stands on the independent kernel.
    let rechecked = report.declarations.len();
    let natives = native_decide::NativeDecisions::read(export);
    match crate::dag::arena::verdict(&natives.rederived_export(export), crate::dag::tc::Engine::Sem) {
        crate::dag::Verdict::Reject(why) => {
            result.fail(
                "independent_recheck",
                Diagnostic::new(
                    "independent_recheck",
                    DiagnosticCode::RecheckRefused,
                    format!("the kernel core refuses the export the LOGOS kernel accepted: {why}"),
                ),
            );
            return None;
        }
        crate::dag::Verdict::Accept => result.pass(
            "independent_recheck",
            format!(
                "{rechecked} definitions, theorems, opaques and axioms rechecked independently, and the whole export \
                 accepted by the kernel core"
            ),
        ),
        crate::dag::Verdict::Decline(why) => result.pass(
            "independent_recheck",
            format!(
                "{rechecked} definitions, theorems, opaques and axioms rechecked independently; the kernel core \
                 declined the export ({why})"
            ),
        ),
    }
    result.audit = Some(report.to_json());
    result.information_losses = losses(&report);

    // Assumption policy: the target's transitive assumptions of statement and proof, computed by the
    // package from the checked environment with Lean's `#print axioms` semantics (an axiom reached
    // counts; the axioms its own type mentions do not). In project mode Lean's own report must agree
    // with them as the package's contract says; a run that did not observe Lean publishes no package,
    // and its result is held to the audit of the checked proof alone.
    let print_axioms = match evidence {
        LeanEvidence::Accepted { axioms, .. } => Some(axioms.clone()),
        LeanEvidence::NotObserved => None,
    };
    let build = match package::build_observed(
        export,
        target,
        &evidence.build_options(),
        print_axioms.as_deref(),
    ) {
        Ok(build) => build,
        Err(verdict) => {
            result.mark(
                "assumption_policy",
                status_of(&verdict),
                self_verification_failure("assumption_policy", &verdict),
            );
            return None;
        }
    };
    let manifest: Option<package::Manifest> = build
        .files
        .get(package::MANIFEST)
        .and_then(|m| serde_json::from_slice(m).ok());
    if let Some(manifest) = &manifest {
        result.assumptions = manifest
            .assumptions
            .transitive
            .iter()
            .map(|a| intake::lean_name(a))
            .collect();
        // The checked proof the result reports is its package's: every re-derived step is disclosed
        // exactly as the package discloses it.
        result.information_losses.extend(
            manifest
                .information_losses
                .iter()
                .filter(|loss| loss.category == native_decide::INFORMATION_CATEGORY)
                .map(|loss| Loss {
                    category: loss.category.clone(),
                    detail: loss.detail.clone(),
                }),
        );
    }
    let policy = match evidence {
        LeanEvidence::Accepted { .. } => build
            .receipt
            .stage("assumption_policy")
            .expect("every stage is on the receipt")
            .clone(),
        LeanEvidence::NotObserved => build.audit.clone(),
    };
    match policy.status {
        StageStatus::Passed => result.pass("assumption_policy", policy.detail.clone()),
        StageStatus::Failed => {
            let code = if policy.code.as_deref() == Some("assumption_disagreement") {
                DiagnosticCode::AssumptionDisagreement
            } else {
                DiagnosticCode::DisallowedAssumption
            };
            result.fail(
                "assumption_policy",
                Diagnostic::new("assumption_policy", code, policy.detail.clone())
                    .with_declaration(target),
            );
            result.diagnostics.extend(super::taint::diagnostics(
                &imported,
                &kernel_target,
                target,
                &result.assumptions,
            ));
            return None;
        }
        StageStatus::NotRun | StageStatus::Unsupported => {
            // The package's own rebuild of the imported environment did not reach the policy.
            let blocker = build
                .receipt
                .first_failure()
                .map(|s| format!("{}: {}", s.name, s.detail))
                .unwrap_or(policy.detail.clone());
            result.fail(
                "assumption_policy",
                Diagnostic::new(
                    "assumption_policy",
                    DiagnosticCode::InternalInconsistency,
                    format!("LOGOS's own package of {target} did not reach its assumption policy: {blocker}"),
                ),
            );
            return None;
        }
    }

    // Emission, reparsed in a fresh context (by the emitter), and the reparsed target statement held
    // against the independent reference translation of the Lean type (rule R8).
    let document = match emit::emit_document(&imported, &kernel_target) {
        Ok(document) => document,
        Err(verdict) => {
            result.mark(
                "emission_reparsed",
                status_of(&verdict),
                self_verification_failure("emission_reparsed", &verdict),
            );
            return None;
        }
    };
    drop(imported);
    result
        .information_losses
        .extend(document.losses.iter().map(|(category, detail)| Loss {
            category: category.clone(),
            detail: detail.clone(),
        }));
    let reference = match reference::reference_declaration(export, &kernel_target) {
        Ok(reference) => reference,
        Err(verdict) => {
            result.mark(
                "emission_reparsed",
                status_of(&verdict),
                self_verification_failure("emission_reparsed", &verdict),
            );
            return None;
        }
    };
    if let Err(fault) = emit::check_statement(&document, &reference) {
        result.fail(
            "emission_reparsed",
            Diagnostic::new(
                "emission_reparsed",
                DiagnosticCode::InternalInconsistency,
                format!("the emitted statement of {target} does not match the reference translation: {fault}"),
            )
            .with_declaration(target),
        );
        return None;
    }
    result.target_logos = Some(document.target.clone());
    result.pass(
        "emission_reparsed",
        "the LOGOS document reparsed in a fresh context to the checked declarations, and its statement is the reference translation's",
    );

    // A package is built only for a result that can be verified: without Lean's acceptance observed
    // it could never be published, and replaying it would decide nothing.
    if result.status("lean_accepted") != StageStatus::Passed {
        return None;
    }
    match package::publishable(build) {
        Ok(files) => Some(files),
        Err(verdict) => {
            result.mark(
                "fresh_replay",
                status_of(&verdict),
                self_verification_failure("fresh_replay", &verdict),
            );
            None
        }
    }
}

/// A refusal of LOGOS's own artifact — emission, packaging, replay — once every check of the user's
/// input passed. Unsupported capabilities, budgets and environment failures keep their codes; any
/// other refusal (a rejection, a malformed or ill-typed artifact of our own making) is an
/// [`InternalInconsistency`](DiagnosticCode::InternalInconsistency): the fault is ours, and it is never
/// reported as the user's mathematics being wrong.
pub fn self_verification_failure(stage: &'static str, verdict: &Verdict) -> Diagnostic {
    let diagnostic = Diagnostic::of_refusal(stage, verdict);
    match verdict.outcome() {
        Outcome::UnsupportedFeature | Outcome::ResourceLimit | Outcome::InfrastructureFailure => {
            diagnostic
        }
        _ => Diagnostic {
            message: format!(
                "LOGOS's own {stage} artifact failed self-verification ({}): {}",
                diagnostic.code, diagnostic.message
            ),
            ..Diagnostic::new(stage, DiagnosticCode::InternalInconsistency, "")
        },
    }
}

/// A capability that declined is `unsupported`; anything that refused is `failed`.
fn status_of(verdict: &Verdict) -> StageStatus {
    match verdict {
        Verdict::Decline(_) if verdict.outcome() == Outcome::UnsupportedFeature => {
            StageStatus::Unsupported
        }
        _ => StageStatus::Failed,
    }
}

fn losses(report: &ImportReport) -> Vec<Loss> {
    report
        .metadata_losses
        .iter()
        .map(|loss| Loss {
            category: "SOURCE_METADATA".into(),
            detail: format!(
                "expression {}: {} = {}",
                loss.expression, loss.key, loss.value
            ),
        })
        .collect()
}

/// The Lean Kernel Arena benchmark protocol: one input path, `ACCEPT` on stdout and exit 0 on
/// acceptance, exit 1 on rejection, exit 2 on decline. Kept byte-for-byte for benchmark consumers.
pub fn run_cli(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
    stdout: &mut dyn std::io::Write,
    stderr: &mut (dyn std::io::Write + Send),
) -> i32 {
    use super::{check_import, check_import_with_trace, declined};
    let mut path = None;
    let mut trace = false;
    let mut audit_declarations = false;
    let options = (|| {
        for argument in arguments {
            if argument == "--trace-stages" {
                trace = true;
            } else if argument == "--audit-declarations" {
                audit_declarations = true;
            } else if path.is_none() {
                path = Some(std::path::PathBuf::from(argument));
            } else {
                return Err(declined(DiagnosticCode::Usage, "expected one input path"));
            }
        }
        path.ok_or_else(|| declined(DiagnosticCode::Usage, "expected input path"))
    })();
    let input = options.and_then(|p| {
        std::fs::read_to_string(p)
            .map_err(|e| declined(DiagnosticCode::Infrastructure, e.to_string()))
    });
    let result = if trace {
        let writer = std::sync::Mutex::new(&mut *stderr);
        let emit = |event: &Value| {
            if let Ok(mut writer) = writer.lock() {
                let _ = serde_json::to_writer(&mut **writer, event);
                let _ = writer.write_all(b"\n");
                let _ = writer.flush();
            }
        };
        let result = input
            .and_then(|s| check_import_with_trace(&s, audit_declarations, Some(&emit)).map(|_| ()));
        let code = result.as_ref().err().map_or(0, Verdict::code);
        emit(&json!({"schema":"logos-lean-stages/1","event":"verdict",
            "code":code,"outcome":match code {0=>"accept",1=>"reject",_=>"decline"}}));
        result
    } else {
        input.and_then(|s| check_import(&s, audit_declarations).map(|_| ()))
    };
    match result {
        Ok(()) => {
            let _ = writeln!(stdout, "ACCEPT");
            0
        }
        Err(error) => {
            if !trace {
                let _ = writeln!(stderr, "{error:?}");
            }
            error.code()
        }
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/pipeline/native_controls.rs"]
mod native_controls;
