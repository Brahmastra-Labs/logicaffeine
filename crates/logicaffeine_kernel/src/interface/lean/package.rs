//! The portable proof package (`logos-lean-package/1` and `/2`) and its replay.
//!
//! Pure: a package is a map from published relative path to bytes. Writing it to disk atomically
//! and running [`replay`] in a fresh process are the caller's (`largo lean`); everything a replay
//! decides is decided here, from the bytes alone, with no importer state.
//!
//! # Two proof contracts
//!
//! The export decides which contract a package states, never the manifest. When the target's
//! closure, as LOGOS checks it, re-derives Lean's `native_decide` steps by kernel evaluation
//! ([`native_decide`](super::native_decide)), the package is `logos-lean-package/2`; otherwise it
//! is `logos-lean-package/1`.
//!
//! * `/1`, the export's proof as exported. `lean_print_axioms`, when the producing run observed
//!   Lean, names exactly the `transitive` assumptions.
//! * `/2`, the export's proof with every native decision of the target's closure re-derived. Its
//!   two assumption sets are kept apart: `lean_print_axioms` is Lean's own report for the proof it
//!   replaced, required, naming [`ELIMINATED_AXIOM`](super::native_decide::ELIMINATED_AXIOM); and
//!   `transitive` is what the checked replacement rests on, which is exactly Lean's report without
//!   that escape. Every declaration holding re-derived steps is disclosed as one
//!   [`INFORMATION_CATEGORY`](super::native_decide::INFORMATION_CATEGORY) information change.
//!
//! The schema is the result identity's domain, so one set of files never carries both contracts'
//! identities.
//!
//! # What a package holds
//!
//! ```text
//! manifest.json          schema, result id, the SHA-256 of every other file, source identity,
//!                        target (Lean name, kernel name, kind, universe parameters, statement,
//!                        LOGOS name), toolchain, transitive assumptions, information losses, the
//!                        trusted computing base, and the producing run's receipt
//! <Target>.lg            the LOGOS document: literate prose, English forms and kernel vernacular
//! certificate.json       the environment certificate (`logos-environment-certificate/1` or `/2`)
//! correspondence.json    Lean name ↔ kernel name ↔ LOGOS name (and document lines)
//! source/export.ndjson   the exact export the result was produced from
//! source/*.lean          the original Lean source, when the producing run had it
//! ```
//!
//! # What a replay establishes, and what it does not
//!
//! A replay reads nothing but the package. Before any checking work it enforces the file-count,
//! byte, path and term limits, verifies every digest and the result identity, and requires every
//! JSON file to be in its one canonical encoding. Then:
//!
//! * `primary_check` — the certificate's environment is rebuilt from the bare kernel, every
//!   inductive, constructor, definition, recursor, opaque theorem, structure η and quotient
//!   primitive kernel-validated under exact (not cumulative) conversion, with no obligation left.
//! * `independent_recheck` — every checked term was also inferred by the second kernel
//!   (`recheck`) to its declared type. A term outside that checker's fragment makes the stage
//!   `unsupported`, never `passed`.
//! * `translated` — the statement is BOUND to the source: the reference translator (which shares no
//!   code with the importer) reads the target's type from `source/export.ndjson`, and it must be
//!   exactly convertible with the certificate's statement. Every other certificate declaration
//!   with a source counterpart is bound the same way, and the correspondence must agree. The
//!   manifest states the contract the carried export decides: its schema, and exactly the
//!   native re-derivations recomputed from the export.
//! * `assumption_policy` — the transitive assumptions of statement AND proof, traced through opaque
//!   theorems and definitions, are exactly Lean's `propext`, `Quot.sound` and `Classical.choice` in
//!   their canonical form; an admission (`sorryAx` or anything of its universal-inhabitant shape)
//!   and an axiom stating the goal are refused by structure, not by spelling. Lean's recorded
//!   report agrees with them as the contract says, and the manifest lists exactly them.
//! * `emission_reparsed` — the LOGOS document is exactly what the emitter writes for the rebuilt
//!   environment, it reparses in a fresh LOGOS session, and its target states the reference
//!   translation.
//! * `fresh_replay` — every stage above passed, from the package bytes alone.
//!
//! `lean_accepted` is `not_run`: a replay never runs Lean. The producing run's receipt is in the
//! manifest, and a replay repeats what it says without adopting it.

use super::native_decide::{Elimination, NativeDecisions};
use super::receipt::{Receipt, Stage, StageStatus, STAGES};
use super::reference::{Reference, ReferenceKind};
use super::{declined, rejected, DiagnosticCode, Result, Verdict};
use crate::certificate::{
    read_certificate, AnyCertificate, CaptureRequest, ConversionDiscipline, EnvironmentBase,
    EnvironmentCertificate, EnvironmentEntry, EnvironmentError, EnvironmentErrorCode,
    EnvironmentLimits, IndependentVerdict, RebuiltEnvironment, ENVIRONMENT_SCHEMA,
    OPAQUE_ENVIRONMENT_SCHEMA, PRELUDE_VERSION,
};
use crate::{Term, Universe};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema of a package whose proof is the export's own, checked as exported.
pub const PACKAGE_SCHEMA: &str = "logos-lean-package/1";
/// The schema of a package whose proof is the export's with the native decisions of the target's
/// closure re-derived by kernel evaluation.
pub const NATIVE_REDERIVED_SCHEMA: &str = "logos-lean-package/2";
/// Every package schema this crate writes and replays.
pub const PACKAGE_SCHEMAS: [&str; 2] = [PACKAGE_SCHEMA, NATIVE_REDERIVED_SCHEMA];
pub const REPLAY_SCHEMA: &str = "logos-lean-replay/1";
pub const CORRESPONDENCE_SCHEMA: &str = "logos-lean-correspondence/1";

/// A package's files, keyed by published relative path.
pub type PackageFiles = BTreeMap<String, Vec<u8>>;

pub const MANIFEST: &str = "manifest.json";
pub const CERTIFICATE: &str = "certificate.json";
pub const CORRESPONDENCE: &str = "correspondence.json";
pub const EXPORT: &str = "source/export.ndjson";

/// Lean's three standard axioms, in the kernel's spelling of their Lean names — the only
/// assumptions a verified result may rest on.
pub const ALLOWED_AXIOMS: [(&str, &str); 3] = [
    ("propext", "/s:\"propext\""),
    ("Quot.sound", "/s:\"Quot\"/s:\"sound\""),
    ("Classical.choice", "/s:\"Classical\"/s:\"choice\""),
];

/// The trusted computing base shared by the checks a package's receipt names. Two checkers that
/// share this much are two implementations of typing, not two independent theorem provers, and the
/// manifest says so in every package.
pub const TRUSTED_BASE: [&str; 8] = [
    "The LOGOS kernel crate (logicaffeine-kernel): its Term, Universe and Context representations, reduction and definitional equality, and the primary type checker.",
    "The independent re-checker (`recheck`) is a separately written de Bruijn type checker in the same crate. It shares the Term, Universe and Context representations and the environment the primary check built; it corroborates typing, it is not an independent theorem prover.",
    "Exact (Lean) conversion is enforced by the Lean reader's exact typing traversal over the kernel's own reduction.",
    "The statement binding reads the export with the reference translator, which shares no code with the importer's conversion, sharing or caching machinery; it shares the kernel's Term type and the serde_json parser.",
    "Structure η, quotient primitives and the kernel's literal Nat are admitted only in their checked canonical forms.",
    "The LOGOS document is checked by the LOGOS parser and `Repl::execute_document` over the same kernel.",
    "SHA-256 digests identify bytes; they establish no semantic correspondence. The binding and the kernel do.",
    "That the pinned Lean accepted the source is recorded by the producing run and is not re-established by replay.",
];

// -------------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), for content digests. Kept here so the kernel stays free of dependencies.
// -------------------------------------------------------------------------------------------------

mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    fn compress(h: &mut [u32; 8], block: &[u8]) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    pub fn digest(message: &[u8]) -> [u8; 32] {
        let mut h = H0;
        let mut chunks = message.chunks_exact(64);
        for block in chunks.by_ref() {
            compress(&mut h, block);
        }
        let rest = chunks.remainder();
        let mut tail = [0u8; 128];
        tail[..rest.len()].copy_from_slice(rest);
        tail[rest.len()] = 0x80;
        let length = if rest.len() < 56 { 64 } else { 128 };
        tail[length - 8..length]
            .copy_from_slice(&((message.len() as u64).wrapping_mul(8)).to_be_bytes());
        for block in tail[..length].chunks_exact(64) {
            compress(&mut h, block);
        }
        let mut out = [0u8; 32];
        for (i, word) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

/// The lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// -------------------------------------------------------------------------------------------------
// The manifest and correspondence formats.
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    /// `sha256:<hex>` of the target's Lean name and every other file's path and digest.
    pub result_id: String,
    /// The SHA-256 of every file in the package except this manifest, by path.
    pub files: BTreeMap<String, String>,
    pub source: SourceIdentity,
    pub target: TargetIdentity,
    /// The LOGOS document's path, or `None` when the emitter could not write one.
    pub document: Option<String>,
    pub emission: EmissionRecord,
    pub toolchain: Toolchain,
    pub assumptions: AssumptionRecord,
    pub information_losses: Vec<InformationLoss>,
    pub trusted_base: Vec<String>,
    /// The producing run's receipt, stage by stage.
    pub receipt: BTreeMap<String, StageRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub export: String,
    pub export_sha256: String,
    pub format: String,
    pub lean_version: String,
    pub lean_githash: Option<String>,
    pub exporter: String,
    /// Original Lean source files carried in the package (under `source/`).
    pub lean_sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetIdentity {
    /// The Lean name, as Lean writes it.
    pub lean: String,
    /// The kernel's spelling, the certificate's target.
    pub kernel: String,
    /// `thm`, `def` or `opaque`.
    pub kind: String,
    pub level_params: Vec<String>,
    /// The statement, rendered for reading (the certificate carries the term).
    pub statement: String,
    /// The LOGOS name in the document, when there is one.
    pub logos: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmissionRecord {
    /// `emitted`, `unsupported` or `failed`.
    pub status: String,
    pub detail: String,
    /// Declarations whose literate form was omitted, with reasons.
    pub omissions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub logos: String,
    pub prelude: String,
    pub certificate_schema: String,
    pub lean: String,
    pub exporter: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssumptionRecord {
    /// The policy: the assumptions a verified result may rest on (Lean names).
    pub allowed: Vec<String>,
    /// The transitive assumptions of the statement and the checked proof (kernel names), sorted —
    /// Lean's `#print axioms` semantics: an axiom reached counts, the axioms its own TYPE mentions
    /// do not; definitions and opaque theorems are traced through their types and bodies.
    pub transitive: Vec<String>,
    /// What the pinned Lean's own `#print axioms` reported for the target (Lean names), when the
    /// producing run compiled the source: the original evidence, recorded as Lean wrote it. Under
    /// `logos-lean-package/1` it names exactly `transitive`; under `/2` it is required, and it names
    /// `transitive` and the eliminated escape.
    pub lean_print_axioms: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InformationLoss {
    pub category: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    pub status: String,
    pub code: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrespondenceFile {
    pub schema: String,
    /// The target's kernel name.
    pub target: String,
    pub rows: Vec<CorrespondenceRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrespondenceRow {
    /// The Lean name as Lean writes it (for a reader-introduced helper, the reader's own name).
    pub lean: String,
    /// The kernel name, as the certificate declares it.
    pub kernel: String,
    /// The LOGOS name in the document, when the document declares it.
    pub logos: Option<String>,
    pub kind: String,
    /// `authored`, `helper`, `synthesized` or `prelude`.
    pub origin: String,
    /// For a helper, the authored declaration it serves.
    pub helper_of: Option<String>,
    /// `literate`, `vernacular` or `prelude`, when the document carries it.
    pub form: Option<String>,
    /// Universe parameters: (kernel name, LOGOS name).
    pub levels: Vec<(String, String)>,
    /// The document's 1-based inclusive line range.
    pub lines: Option<(usize, usize)>,
}

/// The one canonical encoding of a package JSON file: pretty-printed, newline-terminated.
pub fn canonical_json<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("package JSON serializes");
    bytes.push(b'\n');
    bytes
}

/// The certificate's canonical encoding: compact, newline-terminated.
pub fn canonical_certificate(certificate: &EnvironmentCertificate) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(certificate).expect("a certificate serializes");
    bytes.push(b'\n');
    bytes
}

/// The result identity of a `logos-lean-package/1` package: the target and every non-manifest
/// file's path and digest.
pub fn result_id(target_lean: &str, files: &BTreeMap<String, String>) -> String {
    result_id_in(PACKAGE_SCHEMA, target_lean, files)
}

/// The result identity under `schema`, which is the preimage's domain: the same files under two
/// contracts have two identities.
pub fn result_id_in(schema: &str, target_lean: &str, files: &BTreeMap<String, String>) -> String {
    let mut preimage = format!("{schema}\0{target_lean}\0");
    for (path, digest) in files {
        preimage.push_str(path);
        preimage.push('\0');
        preimage.push_str(digest);
        preimage.push('\n');
    }
    format!("sha256:{}", sha256_hex(preimage.as_bytes()))
}

/// The document's file name for a Lean target: its name with anything outside letters, digits,
/// `_`, `-` and `.` written as `_`, then `.lg`.
pub fn document_path(target_lean: &str) -> String {
    let stem: String = target_lean
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_start_matches('.');
    format!("{}.lg", if stem.is_empty() { "target" } else { stem })
}

// -------------------------------------------------------------------------------------------------
// Limits.
// -------------------------------------------------------------------------------------------------

/// Limits enforced on a package before any work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageLimits {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub max_path_bytes: usize,
    pub environment: EnvironmentLimits,
}

impl Default for PackageLimits {
    fn default() -> Self {
        PackageLimits {
            max_files: 64,
            max_file_bytes: 512 * 1024 * 1024,
            max_total_bytes: 1024 * 1024 * 1024,
            max_path_bytes: 255,
            environment: EnvironmentLimits::default(),
        }
    }
}

/// Why a path cannot be a package path, or `None` when it can.
fn unsafe_path(path: &str, limits: &PackageLimits) -> Option<&'static str> {
    if path.is_empty() {
        return Some("an empty path");
    }
    if path.len() > limits.max_path_bytes {
        return Some("a path longer than the limit");
    }
    if path.starts_with('/') || path.contains('\\') || path.contains('\0') || path.contains(':') {
        return Some("an absolute or non-portable path");
    }
    if path
        .split('/')
        .any(|c| c.is_empty() || c == "." || c == ".." || c.starts_with('.'))
    {
        return Some("a path with an empty, dot or hidden component");
    }
    if path.chars().any(char::is_control) {
        return Some("a path with a control character");
    }
    None
}

// -------------------------------------------------------------------------------------------------
// Building a package.
// -------------------------------------------------------------------------------------------------

/// What the producing run knows that the export does not say.
#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// Original Lean source files, by project-relative path (published under `source/`).
    pub lean_sources: BTreeMap<String, Vec<u8>>,
    /// The pinned Lean's verdict on the source, when the producing run compiled it.
    pub lean_run: Option<LeanRun>,
}

/// A pinned Lean toolchain's run over the original source.
#[derive(Clone, Debug)]
pub struct LeanRun {
    pub toolchain: String,
    pub exporter: String,
    /// Whether Lean accepted the source (compiled, with the target's `#print axioms` within policy).
    pub accepted: bool,
    pub detail: String,
}

/// A built package and the producing run's receipt. A build never claims more than its receipt:
/// [`assemble`] is the door that refuses to hand out a package whose receipt is not clean.
pub struct Build {
    pub files: PackageFiles,
    pub receipt: Receipt,
    /// The assumption policy over the checked proof alone: the audit of what it rests on, before
    /// the package's contract holds that against Lean's report. The receipt's `assumption_policy`
    /// is this audit and then the contract's agreement.
    pub audit: Stage,
}

/// Build the package for `target` from a Lean export, and require that every stage the producing
/// run can establish passed. A package is returned only when it would replay clean.
pub fn assemble(export: &str, target: &str) -> Result<PackageFiles> {
    assemble_with(export, target, &BuildOptions::default())
}

/// [`assemble`] with the producing run's own evidence.
pub fn assemble_with(export: &str, target: &str, options: &BuildOptions) -> Result<PackageFiles> {
    assemble_observed(export, target, options, None)
}

/// [`assemble_with`], holding the result's assumptions against what the pinned Lean's own
/// `#print axioms` reported for the target (Lean names), as the package's contract says: any
/// difference in either direction, and a native-rederived proof without Lean's report, is an
/// `assumption_disagreement`, and no package is handed out.
pub fn assemble_observed(
    export: &str,
    target: &str,
    options: &BuildOptions,
    lean_print_axioms: Option<&[String]>,
) -> Result<PackageFiles> {
    publishable(build_observed(export, target, options, lean_print_axioms)?)
}

/// The package of `build`, when every stage the producing run can establish passed and the
/// package replays clean in this process; otherwise the first stage that did not pass, as a refusal.
pub fn publishable(build: Build) -> Result<PackageFiles> {
    // In the order a failure explains the rest: a stage not run behind a failed one is not the news.
    for stage in [
        "primary_check",
        "independent_recheck",
        "translated",
        "assumption_policy",
        "emission_reparsed",
    ] {
        let record = build
            .receipt
            .stage(stage)
            .expect("every stage is on the receipt");
        if record.status != StageStatus::Passed {
            let code = match (stage, record.status) {
                _ if record.code.as_deref() == Some("resource_limit") => {
                    DiagnosticCode::ResourceLimit
                }
                ("assumption_policy", StageStatus::Failed)
                    if record.code.as_deref() == Some("assumption_disagreement") =>
                {
                    DiagnosticCode::AssumptionDisagreement
                }
                ("assumption_policy", StageStatus::Failed) => DiagnosticCode::DisallowedAssumption,
                (_, StageStatus::Unsupported | StageStatus::NotRun) => {
                    DiagnosticCode::UnsupportedFeature
                }
                (_, _) => DiagnosticCode::InvalidProof,
            };
            let message = format!("{stage} {}: {}", record.status.as_str(), record.detail);
            return Err(
                if matches!(
                    code,
                    DiagnosticCode::UnsupportedFeature | DiagnosticCode::ResourceLimit
                ) {
                    declined(code, message)
                } else {
                    rejected(code, message)
                },
            );
        }
    }
    let check = replay(&build.files);
    if !check.replayed() {
        let failure = check
            .first_failure()
            .map(|s| format!("{}: {}", s.name, s.detail))
            .unwrap_or_default();
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            format!("the assembled package does not replay: {failure}"),
        ));
    }
    Ok(build.files)
}

/// Capture `kernel_target`'s environment certificate from ONE import that retained every checked
/// proof (an opaque theorem publishes only its header) and carried Lean's binder names. A proof the
/// capture needs and the import did not retain is `MissingEvidence`, never an axiom.
fn capture_with_proofs(
    imported: &super::ImportedContext,
    kernel_target: &str,
) -> Result<(EnvironmentCertificate, BTreeMap<String, Term>)> {
    let proofs: BTreeMap<String, Term> = imported
        .retained_declarations()
        .map(|(name, _, body)| (name.to_string(), body.clone()))
        .collect();
    let request = CaptureRequest {
        context: imported,
        base: EnvironmentBase::Kernel,
        conversion: ConversionDiscipline::Exact,
        target: kernel_target,
        opaque_proofs: &proofs,
    };
    let certificate = EnvironmentCertificate::capture(&request).map_err(environment_verdict)?;
    // Keep only the proofs the certificate carries: re-emission reads exactly these.
    let carried: BTreeSet<&str> = certificate
        .entries
        .iter()
        .filter_map(|e| match e {
            EnvironmentEntry::Theorem { name, .. } | EnvironmentEntry::Opaque { name, .. } => {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect();
    let proofs = proofs
        .into_iter()
        .filter(|(name, _)| carried.contains(name.as_str()))
        .collect();
    Ok((certificate, proofs))
}

fn environment_verdict(error: EnvironmentError) -> Verdict {
    let code = match error.code {
        EnvironmentErrorCode::ResourceLimit => DiagnosticCode::ResourceLimit,
        EnvironmentErrorCode::Unsupported | EnvironmentErrorCode::MissingEvidence => {
            DiagnosticCode::UnsupportedFeature
        }
        _ => DiagnosticCode::InvalidProof,
    };
    let message = error.to_string();
    if matches!(code, DiagnosticCode::InvalidProof) {
        rejected(code, message)
    } else {
        declined(code, message)
    }
}

/// Build a package and its producing receipt. Refuses only when no package can be formed at all
/// (an export the importer does not accept, a target it does not declare); a package whose checks
/// fail is returned with the failure on its receipt, so a refusal can be inspected and replayed.
pub fn build(export: &str, target: &str, options: &BuildOptions) -> Result<Build> {
    build_observed(export, target, options, None)
}

/// [`build`], with the pinned Lean's `#print axioms` for the target when the producing run has it.
pub fn build_observed(
    export: &str,
    target: &str,
    options: &BuildOptions,
    lean_print_axioms: Option<&[String]>,
) -> Result<Build> {
    // Source terms and their drops need the same bounded worker as certificate checking.
    on_kernel_stack(|| build_observed_on_stack(export, target, options, lean_print_axioms))?
}

fn build_observed_on_stack(
    export: &str,
    target: &str,
    options: &BuildOptions,
    lean_print_axioms: Option<&[String]>,
) -> Result<Build> {
    let header: serde_json::Value = serde_json::from_str(export.lines().next().unwrap_or(""))
        .map_err(|e| {
            declined(
                DiagnosticCode::MalformedExport,
                format!("export header: {e}"),
            )
        })?;
    let reference = Reference::read(export)?;
    let source_limits = EnvironmentLimits::default();
    reference.preflight(
        &[(target.to_string(), false)],
        source_limits.max_depth,
        source_limits.max_tree_nodes,
    )?;
    let kernel_target = reference.source_name(target)?;
    let mut source_work = SourceWork::new(&source_limits)?;
    let (imported, report) = super::import_with(
        export,
        super::ImportRequest {
            audit: true,
            retain: super::RetainSource::EveryProof,
            lean_binder_names: true,
        },
    )?;
    let declaration = source_work.declaration(&*imported, &reference, &kernel_target)?;
    let kind = match declaration.kind {
        ReferenceKind::Theorem => "thm",
        ReferenceKind::Definition => "def",
        ReferenceKind::Opaque => "opaque",
        other => {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                format!(
                    "{} is a Lean {}, and a package proves a theorem or definition",
                    declaration.lean_name,
                    other.as_str()
                ),
            ))
        }
    };
    for (name, bytes) in &options.lean_sources {
        // Project-relative, so a multi-module project keeps its layout (`Demo/Defs.lean`).
        if unsafe_path(&format!("source/{name}"), &PackageLimits::default()).is_some()
            || !name.ends_with(".lean")
        {
            return Err(declined(
                DiagnosticCode::Infrastructure,
                format!("{name:?} is not a project-relative Lean source path"),
            ));
        }
        if std::str::from_utf8(bytes).is_err() {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                format!("{name} is not UTF-8"),
            ));
        }
    }
    let (certificate, proofs) = capture_with_proofs(&imported, &declaration.name)?;
    let mut receipt = Receipt::unstarted();
    match &options.lean_run {
        Some(run) => receipt.set(
            "lean_accepted",
            if run.accepted {
                StageStatus::Passed
            } else {
                StageStatus::Failed
            },
            format!("{} with {}: {}", run.toolchain, run.exporter, run.detail),
        ),
        None => receipt.set(
            "lean_accepted",
            StageStatus::NotRun,
            "this run read a pre-exported artifact and did not run Lean",
        ),
    }
    source_binding_metrics(
        &reference,
        &certificate,
        &declaration.name,
        lean_print_axioms.unwrap_or(&[]),
        &source_limits,
    )?;
    let native = NativeDecisions::read(export).closure_of(&declaration.name);
    let schema = contract_schema(&native);
    let mut checks = check_environment(
        &certificate,
        &reference,
        &declaration.name,
        &source_limits,
        &mut source_work,
    );
    checks.rebuilt = None;
    let audit = stage("assumption_policy", &checks.policy);
    if checks.primary.0 == StageStatus::Passed {
        if let Err(detail) =
            agree_with_lean(&reference, &native, lean_print_axioms, &checks.transitive)
        {
            checks.policy = failed("assumption_disagreement", detail);
        }
    }
    checks.record(&mut receipt);

    // The LOGOS document, written by the emitter from the imported environment and checked in a
    // fresh LOGOS session against the reference translation.
    let proof_of = |key: &str| proofs.get(key).cloned();
    let document = match super::emit::emit_context(&*imported, &proof_of, &declaration.name) {
        Ok(document) => match super::emit::check_statement(&document, &declaration) {
            Ok(()) => {
                receipt.set("emission_reparsed", StageStatus::Passed, "the document reparsed in a fresh LOGOS session and states the reference translation");
                Some(document)
            }
            Err(fault) => {
                receipt.set_with_code(
                    "emission_reparsed",
                    StageStatus::Failed,
                    "emission_mismatch",
                    fault.to_string(),
                );
                None
            }
        },
        Err(verdict) => {
            let unsupported = matches!(verdict, Verdict::Decline(_));
            receipt.set_with_code(
                "emission_reparsed",
                if unsupported {
                    StageStatus::Unsupported
                } else {
                    StageStatus::Failed
                },
                verdict.reason().code().as_str(),
                verdict.reason().message().to_string(),
            );
            None
        }
    };
    receipt.set(
        "fresh_replay",
        StageStatus::NotRun,
        "established only by a replay of the package in a fresh process",
    );

    let mut files = PackageFiles::new();
    files.insert(CERTIFICATE.into(), canonical_certificate(&certificate));
    files.insert(EXPORT.into(), export.as_bytes().to_vec());
    let rows = correspondence_rows(&certificate, &reference, document.as_ref())?;
    let correspondence = CorrespondenceFile {
        schema: CORRESPONDENCE_SCHEMA.into(),
        target: declaration.name.clone(),
        rows,
    };
    files.insert(CORRESPONDENCE.into(), canonical_json(&correspondence));
    let document_file = document
        .as_ref()
        .map(|_| document_path(&declaration.lean_name));
    if let (Some(path), Some(document)) = (&document_file, &document) {
        files.insert(path.clone(), document.source.as_bytes().to_vec());
    }
    let mut lean_sources = Vec::new();
    for (name, bytes) in &options.lean_sources {
        let path = format!("source/{name}");
        files.insert(path.clone(), bytes.clone());
        lean_sources.push(path);
    }
    let digests: BTreeMap<String, String> = files
        .iter()
        .map(|(p, b)| (p.clone(), sha256_hex(b)))
        .collect();
    let emission = match (&document, receipt.stage("emission_reparsed")) {
        (Some(doc), _) => EmissionRecord {
            status: "emitted".into(),
            detail: "written by the emitter and reparsed".into(),
            omissions: doc.omissions.clone(),
        },
        (None, Some(stage)) => EmissionRecord {
            status: if stage.status == StageStatus::Unsupported {
                "unsupported".into()
            } else {
                "failed".into()
            },
            detail: stage.detail.clone(),
            omissions: Vec::new(),
        },
        (None, None) => unreachable!("every stage is on the receipt"),
    };
    let mut losses: Vec<InformationLoss> = report
        .metadata_losses
        .iter()
        .map(|loss| InformationLoss {
            category: "SOURCE_METADATA".into(),
            detail: format!(
                "expression {}: {} = {}",
                loss.expression, loss.key, loss.value
            ),
        })
        .collect();
    losses.extend(
        native
            .information_changes()
            .into_iter()
            .map(|(category, detail)| InformationLoss { category, detail }),
    );
    losses.push(InformationLoss {
        category: "SOURCE_POSITIONS".into(),
        detail: "a Lean export records no source positions, so the correspondence maps names, not spans, on the Lean side".into(),
    });
    let meta = &header["meta"];
    let manifest = Manifest {
        schema: schema.into(),
        result_id: result_id_in(schema, &declaration.lean_name, &digests),
        source: SourceIdentity {
            export: EXPORT.into(),
            export_sha256: digests[EXPORT].clone(),
            format: meta["format"]["version"]
                .as_str()
                .unwrap_or_default()
                .into(),
            lean_version: meta["lean"]["version"].as_str().unwrap_or_default().into(),
            lean_githash: meta["lean"]["githash"].as_str().map(str::to_string),
            exporter: format!(
                "{} {}",
                meta["exporter"]["name"].as_str().unwrap_or("unknown"),
                meta["exporter"]["version"].as_str().unwrap_or("unknown")
            ),
            lean_sources,
        },
        files: digests,
        target: TargetIdentity {
            lean: declaration.lean_name.clone(),
            kernel: declaration.name.clone(),
            kind: kind.into(),
            level_params: declaration.level_params.clone(),
            statement: declaration.ty.to_string(),
            logos: document.as_ref().map(|d| d.target.clone()),
        },
        document: document_file,
        emission,
        toolchain: Toolchain {
            logos: env!("CARGO_PKG_VERSION").into(),
            prelude: PRELUDE_VERSION.into(),
            certificate_schema: certificate.schema.clone(),
            lean: options
                .lean_run
                .as_ref()
                .map(|r| r.toolchain.clone())
                .unwrap_or_else(|| {
                    format!(
                        "leanprover/lean4:v{}",
                        meta["lean"]["version"].as_str().unwrap_or("unknown")
                    )
                }),
            exporter: options
                .lean_run
                .as_ref()
                .map(|r| r.exporter.clone())
                .unwrap_or_else(|| {
                    meta["exporter"]["name"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string()
                }),
        },
        assumptions: AssumptionRecord {
            allowed: ALLOWED_AXIOMS
                .iter()
                .map(|(lean, _)| lean.to_string())
                .collect(),
            transitive: checks.transitive.clone(),
            lean_print_axioms: lean_print_axioms.map(|a| a.to_vec()),
        },
        information_losses: losses,
        trusted_base: TRUSTED_BASE.iter().map(|s| s.to_string()).collect(),
        receipt: receipt_records(&receipt),
    };
    files.insert(MANIFEST.into(), canonical_json(&manifest));
    Ok(Build {
        files,
        receipt,
        audit,
    })
}

/// The schema of the contract `native`, the re-derivations of a target's closure, decides.
fn contract_schema(native: &Elimination) -> &'static str {
    if native.eliminates() {
        NATIVE_REDERIVED_SCHEMA
    } else {
        PACKAGE_SCHEMA
    }
}

/// The manifest states the contract its export decides: the schema, and exactly the re-derivations
/// recomputed from the export as its information changes.
fn states_contract(
    manifest: &Manifest,
    native: &Elimination,
) -> std::result::Result<(), (&'static str, String)> {
    let schema = contract_schema(native);
    if manifest.schema != schema {
        return Err((
            "native_rederivation_schema",
            if native.eliminates() {
                format!(
                "the export's proof of the target re-derives native decisions, so its package is {schema}, but the manifest declares {}",
                manifest.schema
            )
            } else {
                format!(
                "the export's proof of the target re-derives no native decision, so its package is {schema}, but the manifest declares {}",
                manifest.schema
            )
            },
        ));
    }
    native
        .disclosed_by(
            manifest
                .information_losses
                .iter()
                .map(|loss| (loss.category.as_str(), loss.detail.as_str())),
        )
        .map_err(|detail| ("native_rederivation_disclosure", detail))
}

/// Lean's `#print axioms` for the target (Lean names), held against the transitive assumptions the
/// kernel computed for the checked proof (kernel names). Each name Lean reports is read through the
/// export's own name table, so a numeric component or an escaped string cannot be misread by
/// splitting on dots, and must be declared there, once.
///
/// A proof checked as exported rests on exactly what Lean reports, and a run that did not observe
/// Lean holds nothing against it. A proof whose native decisions were re-derived (`native`) rests on
/// Lean's report without the eliminated escape, and Lean's report must name it: that report is the
/// only evidence of what the replaced proof rested on, so its absence is a disagreement too.
fn agree_with_lean(
    reference: &Reference,
    native: &Elimination,
    lean: Option<&[String]>,
    transitive: &[String],
) -> std::result::Result<(), String> {
    let Some(lean) = lean else {
        if !native.eliminates() {
            return Ok(());
        }
        let replaced: Vec<&str> = native
            .rederivations
            .iter()
            .map(|r| r.lean.as_str())
            .collect();
        return Err(format!(
            "the proof re-derives the native decisions of {}, and a native-rederived package states what Lean's own proof \
             rested on, but this run recorded no #print axioms from Lean",
            replaced.join(", ")
        ));
    };
    let mut kernel_of: BTreeMap<&str, String> = BTreeMap::new();
    let mut reported = BTreeSet::new();
    for axiom in lean {
        let kernel = reference.source_lean_name(axiom).map_err(|_| {
            format!("Lean's #print axioms names {axiom}, which the export does not declare")
        })?;
        if !reported.insert(kernel.clone()) {
            return Err(format!("Lean's #print axioms names {axiom} twice"));
        }
        kernel_of.insert(axiom, kernel);
    }
    let expected_lean = native.lean_after_rederivation(lean)?;
    let expected: BTreeSet<String> = expected_lean
        .iter()
        .map(|axiom| kernel_of[axiom.as_str()].clone())
        .collect();
    let computed: BTreeSet<String> = transitive.iter().cloned().collect();
    if expected == computed {
        return Ok(());
    }
    let lean_names = |names: &mut dyn Iterator<Item = &String>| {
        names
            .map(|n| super::intake::lean_name(n))
            .collect::<Vec<_>>()
    };
    let rests_on = lean_names(&mut computed.iter());
    let only_lean = lean_names(&mut expected.difference(&computed));
    let only_here = lean_names(&mut computed.difference(&expected));
    let rederived = if native.eliminates() {
        format!(", so with its native decisions re-derived the statement and proof must rest on {expected_lean:?}")
    } else {
        String::new()
    };
    Err(format!(
        "Lean's #print axioms reports {lean:?}{rederived}, but the statement and proof rest on {rests_on:?} (only in \
         Lean: {only_lean:?}; only here: {only_here:?})"
    ))
}

/// A receipt stage of `outcome`.
fn stage(name: &'static str, (status, code, detail): &StageOutcome) -> Stage {
    Stage {
        name,
        status: *status,
        detail: detail.clone(),
        code: code.clone(),
    }
}

fn receipt_records(receipt: &Receipt) -> BTreeMap<String, StageRecord> {
    receipt
        .stages
        .iter()
        .map(|s| {
            (
                s.name.to_string(),
                StageRecord {
                    status: s.status.as_str().into(),
                    code: s.code.clone(),
                    detail: s.detail.clone(),
                },
            )
        })
        .collect()
}

/// The correspondence rows: the emitter's, when there is a document; otherwise one row per
/// certificate declaration, Lean name ↔ kernel name.
fn correspondence_rows(
    certificate: &EnvironmentCertificate,
    reference: &Reference,
    document: Option<&super::emit::EmittedDocument>,
) -> Result<Vec<CorrespondenceRow>> {
    use super::emit::{Form, Origin};
    if let Some(document) = document {
        return Ok(document
            .correspondence
            .iter()
            .map(|c| {
                let (origin, helper_of) = match &c.origin {
                    Origin::Authored => ("authored", None),
                    Origin::Helper { of } => ("helper", Some(of.clone())),
                    Origin::Synthesized => ("synthesized", None),
                    Origin::Prelude => ("prelude", None),
                };
                CorrespondenceRow {
                    lean: c.lean.clone(),
                    kernel: c.importer_key.clone(),
                    logos: Some(c.logos.clone()),
                    kind: c.kind.as_str().into(),
                    origin: origin.into(),
                    helper_of,
                    form: Some(
                        match c.form {
                            Form::Literate => "literate",
                            Form::Vernacular => "vernacular",
                            Form::Prelude => "prelude",
                        }
                        .into(),
                    ),
                    levels: c.levels.clone(),
                    lines: c.lines,
                }
            })
            .collect());
    }
    let mut rows = Vec::new();
    for entry in &certificate.entries {
        for name in entry.names() {
            let (lean, origin) = match reference.source_spelling(name) {
                Ok((_, lean)) => (lean, "authored"),
                Err(_) => (name.to_string(), "synthesized"),
            };
            rows.push(CorrespondenceRow {
                lean,
                kernel: name.to_string(),
                logos: None,
                kind: entry_kind(entry, name).into(),
                origin: origin.into(),
                helper_of: None,
                form: None,
                levels: Vec::new(),
                lines: None,
            });
        }
    }
    Ok(rows)
}

fn entry_kind(entry: &EnvironmentEntry, name: &str) -> &'static str {
    match entry {
        EnvironmentEntry::Axiom { .. } => "axiom",
        EnvironmentEntry::Definition { .. } => "definition",
        EnvironmentEntry::Theorem { .. } => "theorem",
        EnvironmentEntry::Opaque { .. } => "opaque",
        EnvironmentEntry::Inductive { types, .. } => {
            if types.iter().any(|t| t.name == name) {
                "inductive"
            } else {
                "constructor"
            }
        }
        EnvironmentEntry::Structure { .. } => "structure",
        EnvironmentEntry::Quotient { .. } => "quotient",
    }
}

// -------------------------------------------------------------------------------------------------
// The environment checks a build and a replay share.
// -------------------------------------------------------------------------------------------------

/// The outcome of rebuilding, binding and auditing one certificate.
struct EnvironmentChecks {
    /// `primary_check`, `independent_recheck`, `translated`, `assumption_policy`, as
    /// `(status, code, detail)`.
    primary: (StageStatus, Option<String>, String),
    independent: (StageStatus, Option<String>, String),
    translated: (StageStatus, Option<String>, String),
    policy: (StageStatus, Option<String>, String),
    /// The transitive assumptions of the statement and proof, sorted kernel names.
    transitive: Vec<String>,
    /// The rebuilt target's proofs of opaque theorems, for re-emission.
    rebuilt: Option<RebuiltSummary>,
}

struct RebuiltSummary {
    context: crate::Context,
    proofs: BTreeMap<String, Term>,
}

impl EnvironmentChecks {
    fn record(&self, receipt: &mut Receipt) {
        for (stage, (status, code, detail)) in [
            ("primary_check", &self.primary),
            ("independent_recheck", &self.independent),
            ("translated", &self.translated),
            ("assumption_policy", &self.policy),
        ] {
            match code {
                Some(code) => receipt.set_with_code(stage, *status, code, detail.clone()),
                None => receipt.set(stage, *status, detail.clone()),
            }
        }
    }
}

fn failed(code: &str, detail: impl Into<String>) -> (StageStatus, Option<String>, String) {
    (StageStatus::Failed, Some(code.to_string()), detail.into())
}

fn not_run(why: &str) -> (StageStatus, Option<String>, String) {
    (StageStatus::NotRun, None, why.to_string())
}

/// Run `work` on a thread whose stack holds the kernel's deepest admissible term.
fn on_kernel_stack<R: Send>(work: impl FnOnce() -> R + Send) -> Result<R> {
    let rungs = EnvironmentLimits::default()
        .max_depth
        .saturating_mul(3)
        .saturating_add(crate::term_store::INLINE_DEPTH);
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("lean-package-build".into())
            .stack_size(crate::term_store::stack_bytes_for_depth(rungs))
            .spawn_scoped(scope, work)
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p))
            })
            .map_err(|e| {
                declined(
                    DiagnosticCode::Infrastructure,
                    format!("cannot reserve the package stack: {e}"),
                )
            })
    })
}

/// Bound source syntax before any reference term is allocated. Reader helpers have no
/// source counterpart; recursors have a source type but no transparent `value` field.
fn source_binding_metrics(
    reference: &Reference,
    certificate: &EnvironmentCertificate,
    target: &str,
    other_source_reads: &[String],
    limits: &EnvironmentLimits,
) -> Result<super::reference::SourceMetrics> {
    let source_names: BTreeMap<_, _> = reference.names()?.into_iter().collect();
    let mut declarations = BTreeMap::<String, bool>::new();
    for entry in &certificate.entries {
        for name in entry.names() {
            if let Some(kind) = source_names.get(name) {
                let body = matches!(
                    (entry, kind),
                    (
                        EnvironmentEntry::Definition { .. } | EnvironmentEntry::Opaque { .. },
                        ReferenceKind::Definition | ReferenceKind::Opaque
                    )
                );
                declarations
                    .entry(name.to_string())
                    .and_modify(|existing| *existing |= body)
                    .or_insert(body);
            }
        }
    }
    // A missing target remains the ordinary source-binding error after this term-free pass.
    if source_names.contains_key(target) {
        declarations.entry(target.to_string()).or_insert(false);
    }
    // Manifest correspondence and reported axioms also call Reference::declaration.
    // Resolve names without lowering; absent names keep their existing later diagnostic.
    for name in other_source_reads {
        if let Ok(name) = reference.source_lean_name(name) {
            declarations.entry(name).or_insert(false);
        }
    }
    reference.preflight_kernel(
        &declarations.into_iter().collect::<Vec<_>>(),
        limits.max_depth,
        limits.max_tree_nodes,
    )
}

/// Rebuild the certificate, then bind its statement to the export and audit its assumptions.
fn check_environment(
    certificate: &EnvironmentCertificate,
    reference: &Reference,
    kernel_target: &str,
    limits: &EnvironmentLimits,
    source_work: &mut SourceWork,
) -> EnvironmentChecks {
    let blocked = "the environment did not rebuild";
    let mut canonical = BTreeMap::new();
    let mut body_binding = reference
        .names()
        .map(|names| names.into_iter().collect::<BTreeMap<_, _>>())
        .map_err(|error| {
            binding_error(
                error,
                "missing_source_evidence",
                "source declaration inventory",
            )
        });
    let mut first_binding_failure = body_binding.as_ref().err().cloned();
    let rebuilt =
        match certificate.rebuild_observing_values(limits, &mut |prefix, name, levels, ty, body| {
            if first_binding_failure.is_none() {
                if let Ok(source_names) = &mut body_binding {
                    first_binding_failure = bind_source_body(
                        prefix,
                        reference,
                        source_names,
                        &mut canonical,
                        name,
                        levels,
                        ty,
                        body,
                        source_work,
                    )
                    .err();
                }
            }
        }) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                let code = error.code.as_str();
                let (primary, independent) =
                    if error.code == EnvironmentErrorCode::IndependentDisagreement {
                        (
                        failed(
                            "rebuild_incomplete",
                            "the environment did not finish rebuilding: the two kernels disagreed",
                        ),
                        failed(code, error.message.clone()),
                    )
                    } else {
                        (failed(code, error.message.clone()), not_run(blocked))
                    };
                return EnvironmentChecks {
                    primary,
                    independent,
                    translated: first_binding_failure
                        .map(binding_failure)
                        .unwrap_or_else(|| not_run(blocked)),
                    policy: not_run(blocked),
                    transitive: Vec::new(),
                    rebuilt: None,
                };
            }
        };
    let primary = (
        StageStatus::Passed,
        None,
        format!(
            "{} declarations rebuilt from the bare kernel, each kernel-validated under exact conversion; no obligation owed",
            certificate.entries.len()
        ),
    );
    let unsupported: Vec<String> = rebuilt
        .independent
        .iter()
        .filter_map(|(name, verdict)| match verdict {
            IndependentVerdict::Unsupported(why) => Some(format!("{name} ({why})")),
            IndependentVerdict::Agreed => None,
        })
        .collect();
    let independent = if unsupported.is_empty() {
        (
            StageStatus::Passed,
            None,
            format!(
                "{} checked terms re-inferred by the independent kernel to their declared types",
                rebuilt.independent.len()
            ),
        )
    } else {
        (
            StageStatus::Unsupported,
            Some("outside_recheck_fragment".to_string()),
            format!(
                "the independent kernel does not cover {} term(s): {}",
                unsupported.len(),
                unsupported.join("; ")
            ),
        )
    };
    let translated = match first_binding_failure {
        Some(error) => binding_failure(error),
        None => match bind(certificate, &rebuilt, reference, kernel_target, source_work) {
            Ok(detail) => (StageStatus::Passed, None, detail),
            Err(error) => binding_failure(error),
        },
    };
    let transitive: Vec<String> = rebuilt
        .axioms
        .axioms
        .iter()
        .chain(rebuilt.axioms.unresolved.iter())
        .cloned()
        .collect();
    let policy = match audit_assumptions(&rebuilt) {
        Ok(detail) => (StageStatus::Passed, None, detail),
        Err((code, detail)) => failed(code, detail),
    };
    let proofs = rebuilt.proofs.clone();
    EnvironmentChecks {
        primary,
        independent,
        translated,
        policy,
        transitive,
        rebuilt: Some(RebuiltSummary {
            context: rebuilt.context,
            proofs,
        }),
    }
}

type BindingError = (&'static str, String);

// Conversion/resource incompleteness is evidence of an unfinished comparison, never inequality.
fn binding_error(error: Verdict, mismatch: &'static str, what: &str) -> BindingError {
    let code = match error.reason().code() {
        DiagnosticCode::ResourceLimit
        | DiagnosticCode::UnsupportedFeature
        | DiagnosticCode::RecheckRefused
        | DiagnosticCode::AuditIncomplete
        | DiagnosticCode::Infrastructure => error.reason().code().as_str(),
        _ => mismatch,
    };
    (code, format!("{what}: {}", error.reason().message()))
}

fn binding_failure((code, detail): BindingError) -> StageOutcome {
    let status = if matches!(
        code,
        "unsupported_feature" | "recheck_refused" | "audit_incomplete"
    ) {
        StageStatus::Unsupported
    } else {
        StageStatus::Failed
    };
    (status, Some(code.to_string()), detail)
}

fn source_levels(
    name: &str,
    levels: &[String],
    terms: &[&Term],
    work: &mut SourceWork,
) -> std::result::Result<(), BindingError> {
    if !work
        .levels(levels, terms)
        .map_err(|error| binding_error(error, "statement_mismatch", name))?
    {
        return Err((
            "statement_mismatch",
            format!("{name} has duplicate or undeclared source universe parameters"),
        ));
    }
    Ok(())
}

/// Check source syntax itself, including dead subterms, with both kernels.
fn source_type(
    prefix: &crate::Context,
    name: &str,
    levels: &[String],
    ty: &Term,
    work: &mut SourceWork,
) -> std::result::Result<(), BindingError> {
    source_levels(name, levels, &[ty], work)?;
    let main = super::lean_type(prefix, ty)
        .map_err(|error| binding_error(error, "statement_mismatch", name))?;
    let sort = super::normal(prefix, &main)
        .map_err(|error| binding_error(error, "statement_mismatch", name))?;
    if !matches!(sort, Term::Sort(_)) {
        return Err((
            "statement_mismatch",
            format!("{name}'s source type does not inhabit a sort"),
        ));
    }
    let independent = crate::recheck(prefix, ty).map_err(|error| {
        binding_error(
            super::independent_verdict(&error, name),
            "statement_mismatch",
            name,
        )
    })?;
    super::independent_comparison(super::exact(prefix, &independent, &main), name)
        .map_err(|error| binding_error(error, "statement_mismatch", name))
}

/// Called before each candidate value is registered. A failure is sticky for the entire package;
/// continuing the checked rebuild cannot turn a failed source comparison into source evidence.
fn bind_source_body(
    prefix: &crate::Context,
    reference: &Reference,
    source_names: &BTreeMap<String, ReferenceKind>,
    canonical: &mut BTreeMap<String, super::source_recursor::CanonicalRecursor>,
    name: &str,
    levels: &[String],
    ty: &Term,
    body: &Term,
    work: &mut SourceWork,
) -> std::result::Result<(), BindingError> {
    work.lookup_name(name, source_names.len())
        .map_err(|error| binding_error(error, "missing_source_evidence", name))?;
    let Some(kind) = source_names.get(name) else {
        // Absence is established from the complete raw inventory, not a failed translation.
        // Private checked helpers are permitted only while all sourced observations are bound.
        return if reader_introduced(name) {
            Ok(())
        } else {
            Err((
                "unsourced_declaration",
                format!("the certificate value {name} has no source declaration"),
            ))
        };
    };
    if *kind == ReferenceKind::Recursor {
        work.lookup_name(name, canonical.len())
            .map_err(|error| binding_error(error, "definition_mismatch", name))?;
        if !canonical.contains_key(name) {
            let block = work
                .block(prefix, reference, name)
                .map_err(|error| binding_error(error, "missing_source_evidence", name))?;
            let derived = super::source_recursor::derive_bounded(prefix, &block, &mut work.budget)
                .map_err(|error| binding_error(error, "definition_mismatch", name))?;
            // Cache the COMPLETE block at its first recursor, while no carried sibling exists.
            for recursor in derived {
                let key = work
                    .cache_name(&recursor.name)
                    .map_err(|error| binding_error(error, "definition_mismatch", name))?;
                work.lookup_name(&key, canonical.len())
                    .map_err(|error| binding_error(error, "definition_mismatch", name))?;
                if canonical.insert(key, recursor).is_some() {
                    return Err((
                        "definition_mismatch",
                        "canonical source recursor blocks overlap".into(),
                    ));
                }
            }
        }
        work.lookup_name(name, canonical.len())
            .map_err(|error| binding_error(error, "definition_mismatch", name))?;
        let expected = canonical.get(name).ok_or_else(|| {
            (
                "missing_source_evidence",
                format!("the independently derived block has no {name}"),
            )
        })?;
        if expected.levels != levels {
            return Err((
                "statement_mismatch",
                format!("{name} changes the canonical source universe telescope"),
            ));
        }
        super::exact(prefix, &expected.ty, ty)
            .map_err(|error| binding_error(error, "statement_mismatch", name))?;
        return super::exact(prefix, &expected.body, body)
            .map_err(|error| binding_error(error, "definition_mismatch", name));
    }
    if !matches!(kind, ReferenceKind::Definition | ReferenceKind::Opaque) {
        return Err((
            "kind_mismatch",
            format!("{name} is a source {} but carries a value", kind.as_str()),
        ));
    }
    let source = work
        .declaration(prefix, reference, name)
        .map_err(|error| binding_error(error, "missing_source_evidence", name))?;
    if source.level_params != levels {
        return Err((
            "statement_mismatch",
            format!("{name} changes the source universe telescope"),
        ));
    }
    source_type(prefix, name, levels, &source.ty, work)?;
    super::exact(prefix, &source.ty, ty)
        .map_err(|error| binding_error(error, "statement_mismatch", name))?;
    let source_body = work
        .value(prefix, reference, name)
        .map_err(|error| binding_error(error, "missing_source_evidence", name))?;
    source_levels(name, levels, &[&source_body], work)?;
    let main = super::lean_type(prefix, &source_body)
        .map_err(|error| binding_error(error, "definition_mismatch", name))?;
    super::exact(prefix, &main, &source.ty)
        .map_err(|error| binding_error(error, "definition_mismatch", name))?;
    let independent = crate::recheck(prefix, &source_body).map_err(|error| {
        binding_error(
            super::independent_verdict(&error, name),
            "definition_mismatch",
            name,
        )
    })?;
    super::independent_comparison(super::exact(prefix, &independent, &source.ty), name)
        .map_err(|error| binding_error(error, "definition_mismatch", name))?;
    // Opaque declarations remain sealed: compare their supplied values in the prefix, without
    // registering or unfolding either candidate under its own name. Theorem proofs are excluded.
    super::exact(prefix, &source_body, body)
        .map_err(|error| binding_error(error, "definition_mismatch", name))
}

/// Bind the certificate to the export: its target is the requested Lean declaration and states the
/// reference translation of its Lean type, and every other declaration with a source counterpart
/// is that counterpart.
fn bind(
    certificate: &EnvironmentCertificate,
    rebuilt: &RebuiltEnvironment,
    reference: &Reference,
    kernel_target: &str,
    work: &mut SourceWork,
) -> std::result::Result<String, (&'static str, String)> {
    if certificate.target != kernel_target {
        return Err((
            "target_mismatch",
            format!(
                "the certificate proves {}, not the requested {kernel_target}",
                certificate.target
            ),
        ));
    }
    let ctx = &rebuilt.context;
    let target = work
        .declaration(ctx, reference, kernel_target)
        .map_err(|v| binding_error(v, "missing_source_declaration", kernel_target))?;
    let kind_ok = matches!(
        (
            target.kind,
            rebuilt.target_is_theorem,
            rebuilt.target_is_opaque
        ),
        (ReferenceKind::Theorem, true, false)
            | (ReferenceKind::Opaque, false, true)
            | (ReferenceKind::Definition, false, false)
    );
    if !kind_ok {
        return Err((
            "kind_mismatch",
            format!(
                "{} is a Lean {} but the certificate states a {}",
                target.lean_name,
                target.kind.as_str(),
                if rebuilt.target_is_opaque {
                    "opaque"
                } else if rebuilt.target_is_theorem {
                    "theorem"
                } else {
                    "definition"
                }
            ),
        ));
    }
    if target.level_params != rebuilt.target_level_params {
        return Err((
            "statement_mismatch",
            format!(
                "{} has universe parameters {:?} in the export and {:?} in the certificate",
                target.lean_name, target.level_params, rebuilt.target_level_params
            ),
        ));
    }
    source_type(ctx, kernel_target, &target.level_params, &target.ty, work)?;
    super::exact(ctx, &target.ty, &rebuilt.target_type)
        .map_err(|error| binding_error(error, "statement_mismatch", kernel_target))?;
    let mut bound = 0usize;
    let source_names: BTreeMap<_, _> = reference
        .names()
        .map_err(|error| binding_error(error, "missing_source_evidence", "source inventory"))?
        .into_iter()
        .collect();
    for entry in &certificate.entries {
        for name in entry.names() {
            if name == kernel_target {
                continue;
            }
            match work.declaration(ctx, reference, name) {
                Ok(source) => {
                    let expected_kind = match entry {
                        EnvironmentEntry::Axiom { .. } => {
                            matches!(source.kind, ReferenceKind::Axiom)
                        }
                        EnvironmentEntry::Definition { .. } => matches!(
                            source.kind,
                            ReferenceKind::Definition | ReferenceKind::Recursor
                        ),
                        EnvironmentEntry::Theorem { .. } => {
                            matches!(source.kind, ReferenceKind::Theorem)
                        }
                        EnvironmentEntry::Opaque { .. } => {
                            matches!(source.kind, ReferenceKind::Opaque)
                        }
                        EnvironmentEntry::Inductive { types, .. } => {
                            let is_type = types.iter().any(|t| t.name == name);
                            matches!(source.kind, ReferenceKind::Inductive) && is_type
                                || matches!(source.kind, ReferenceKind::Constructor) && !is_type
                        }
                        EnvironmentEntry::Quotient { .. } => {
                            matches!(source.kind, ReferenceKind::Quotient)
                        }
                        EnvironmentEntry::Structure { .. } => false,
                    };
                    if !expected_kind {
                        return Err((
                            "kind_mismatch",
                            format!(
                                "{} is a Lean {} but the certificate declares it as a {}",
                                source.lean_name,
                                source.kind.as_str(),
                                entry_kind(entry, name)
                            ),
                        ));
                    }
                    if source.kind == ReferenceKind::Inductive {
                        work.family(ctx, reference, name).map_err(|error| {
                            binding_error(error, "family_metadata_mismatch", name)
                        })?;
                    }
                    let declared = ctx
                        .poly_declaration(name)
                        .or_else(|| ctx.get_global(name).map(|t| (&[][..], t)));
                    let Some((levels, ty)) = declared else {
                        return Err((
                            "missing_source_declaration",
                            format!("{name} is not in the rebuilt environment"),
                        ));
                    };
                    if levels != source.level_params.as_slice() {
                        return Err(("statement_mismatch", format!("{} has universe parameters {:?} in the export and {levels:?} in the certificate", source.lean_name, source.level_params)));
                    }
                    source_type(ctx, name, &source.level_params, &source.ty, work)?;
                    super::exact(ctx, &source.ty, ty)
                        .map_err(|error| binding_error(error, "statement_mismatch", name))?;
                    bound += 1;
                }
                Err(error) => {
                    if source_names.contains_key(name) {
                        return Err(binding_error(error, "missing_source_evidence", name));
                    }
                    if !matches!(entry, EnvironmentEntry::Definition { .. })
                        || !reader_introduced(name)
                    {
                        return Err(("unsourced_declaration", format!(
                            "the certificate declares the {} {name}, which the export does not declare",
                            entry_kind(entry, name)
                        )));
                    }
                }
            }
        }
    }
    let detail = format!(
        "{}'s statement is exactly convertible with the reference translation of its Lean type; {bound} further declaration types and every sourced transparent or opaque value bound to their source",
        target.lean_name
    );
    Ok(detail)
}

/// Names the Lean reader introduces itself: sharing helpers and structure projections. They are
/// checked definitions like any other; they simply have no Lean spelling.
fn reader_introduced(name: &str) -> bool {
    name.starts_with("arena.")
}

// -------------------------------------------------------------------------------------------------
// The assumption policy.
// -------------------------------------------------------------------------------------------------

fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn app(f: Term, args: Vec<Term>) -> Term {
    args.into_iter()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}
fn at(name: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: name.into(),
        levels,
    }
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}

const EQ: &str = "/s:\"Eq\"";
const EQ_REFL: &str = "/s:\"Eq\"/s:\"refl\"";
const IFF: &str = "/s:\"Iff\"";
const IFF_INTRO: &str = "/s:\"Iff\"/s:\"intro\"";
const NONEMPTY: &str = "/s:\"Nonempty\"";
const NONEMPTY_INTRO: &str = "/s:\"Nonempty\"/s:\"intro\"";

/// Whether `name` is declared in `ctx` with exactly the given level telescope size, arity, one
/// constructor and constructor type — the canonical shape of an inductive an allowed axiom's
/// meaning depends on.
fn canonical_inductive(
    ctx: &crate::Context,
    name: &str,
    arity: impl Fn(&[String]) -> Term,
    num_params: usize,
    constructor: &str,
    constructor_type: impl Fn(&[String]) -> Term,
) -> bool {
    if !ctx.is_inductive(name) || ctx.inductive_num_params(name) != num_params {
        return false;
    }
    let levels = ctx
        .poly_declaration(name)
        .map(|(p, _)| p.to_vec())
        .unwrap_or_default();
    let constructors = ctx.get_constructors(name);
    constructors.len() == 1
        && constructors[0].0 == constructor
        && ctx
            .get_global(name)
            .is_some_and(|t| crate::defeq_for_test(ctx, t, &arity(&levels)))
        && crate::defeq_for_test(ctx, constructors[0].1, &constructor_type(&levels))
}

fn level(levels: &[String], i: usize) -> Universe {
    levels
        .get(i)
        .map(|l| Universe::Var(l.clone()))
        .unwrap_or(Universe::Prop)
}

fn canonical_eq(ctx: &crate::Context) -> bool {
    canonical_inductive(
        ctx,
        EQ,
        |l| {
            pi(
                "α",
                Term::Sort(level(l, 0)),
                pi("a", v("α"), pi("b", v("α"), prop())),
            )
        },
        2,
        EQ_REFL,
        |l| {
            pi(
                "α",
                Term::Sort(level(l, 0)),
                pi(
                    "a",
                    v("α"),
                    app(at(EQ, vec![level(l, 0)]), vec![v("α"), v("a"), v("a")]),
                ),
            )
        },
    ) && ctx.poly_declaration(EQ).is_some_and(|(p, _)| p.len() == 1)
}

fn canonical_iff(ctx: &crate::Context) -> bool {
    canonical_inductive(
        ctx,
        IFF,
        |_| pi("a", prop(), pi("b", prop(), prop())),
        2,
        IFF_INTRO,
        |_| {
            pi(
                "a",
                prop(),
                pi(
                    "b",
                    prop(),
                    pi(
                        "mp",
                        pi("x", v("a"), v("b")),
                        pi(
                            "mpr",
                            pi("y", v("b"), v("a")),
                            app(g(IFF), vec![v("a"), v("b")]),
                        ),
                    ),
                ),
            )
        },
    )
}

fn canonical_nonempty(ctx: &crate::Context) -> bool {
    canonical_inductive(
        ctx,
        NONEMPTY,
        |l| pi("α", Term::Sort(level(l, 0)), prop()),
        1,
        NONEMPTY_INTRO,
        |l| {
            pi(
                "α",
                Term::Sort(level(l, 0)),
                pi(
                    "val",
                    v("α"),
                    app(at(NONEMPTY, vec![level(l, 0)]), vec![v("α")]),
                ),
            )
        },
    ) && ctx
        .poly_declaration(NONEMPTY)
        .is_some_and(|(p, _)| p.len() == 1)
}

/// The canonical statement of an allowed axiom at its own universe parameters, and whether the
/// inductives its meaning depends on are canonical in `ctx`.
fn allowed_statement(ctx: &crate::Context, name: &str, levels: &[String]) -> Option<Term> {
    match name {
        "/s:\"propext\"" if levels.is_empty() && canonical_eq(ctx) && canonical_iff(ctx) => {
            Some(pi(
                "a",
                prop(),
                pi(
                    "b",
                    prop(),
                    pi(
                        "h",
                        app(g(IFF), vec![v("a"), v("b")]),
                        app(
                            at(EQ, vec![Universe::Type(0)]),
                            vec![prop(), v("a"), v("b")],
                        ),
                    ),
                ),
            ))
        }
        "/s:\"Quot\"/s:\"sound\"" if levels.len() == 1 && canonical_eq(ctx) => {
            let u = Universe::Var(levels[0].clone());
            let quotient = app(
                at("kernel.quot.type", vec![u.clone()]),
                vec![v("α"), v("r")],
            );
            let mk = |x: Term| {
                app(
                    at("kernel.quot.mk", vec![u.clone()]),
                    vec![v("α"), v("r"), x],
                )
            };
            Some(pi(
                "α",
                Term::Sort(u.clone()),
                pi(
                    "r",
                    pi("x", v("α"), pi("y", v("α"), prop())),
                    pi(
                        "a",
                        v("α"),
                        pi(
                            "b",
                            v("α"),
                            pi(
                                "h",
                                app(v("r"), vec![v("a"), v("b")]),
                                app(
                                    at(EQ, vec![u.clone()]),
                                    vec![quotient, mk(v("a")), mk(v("b"))],
                                ),
                            ),
                        ),
                    ),
                ),
            ))
        }
        "/s:\"Classical\"/s:\"choice\"" if levels.len() == 1 && canonical_nonempty(ctx) => {
            let u = Universe::Var(levels[0].clone());
            Some(pi(
                "α",
                Term::Sort(u.clone()),
                pi("h", app(at(NONEMPTY, vec![u]), vec![v("α")]), v("α")),
            ))
        }
        _ => None,
    }
}

/// An assumption that inhabits every type of some sort — `Π (α : Sort u) …, α`, whose other
/// premises say nothing about `α` — proves anything, whatever it is called. This is the structure
/// of `sorryAx` (its one other premise is a `Bool`) and of every alias of it. A premise that
/// constrains `α`, as `Nonempty α` does for `Classical.choice`, is what separates a choice
/// principle from an admission.
fn universal_inhabitant(ctx: &crate::Context, ty: &Term) -> bool {
    let Ok(mut t) = crate::normalize_checked(ctx, ty) else {
        return false;
    };
    let mut sorts: BTreeSet<String> = BTreeSet::new();
    let mut counter = 0usize;
    loop {
        match t {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let fresh = format!("policy.binder.{counter}");
                counter += 1;
                let domain = crate::normalize_checked(ctx, &param_type).unwrap_or(*param_type);
                if matches!(domain, Term::Sort(_)) {
                    sorts.insert(fresh.clone());
                } else if sorts
                    .iter()
                    .any(|s| crate::term_store::occurs_free_in_term(&domain, s))
                {
                    return false;
                }
                t = crate::substitute(&body_type, &param, &Term::Var(fresh));
                if let Ok(normal) = crate::normalize_checked(ctx, &t) {
                    t = normal;
                }
            }
            Term::Var(ref name) => return sorts.contains(name),
            _ => return false,
        }
    }
}

/// Audit the transitive assumptions of the target's statement and proof.
fn audit_assumptions(
    rebuilt: &RebuiltEnvironment,
) -> std::result::Result<String, (&'static str, String)> {
    let ctx = &rebuilt.context;
    if !rebuilt.axioms.unresolved.is_empty() {
        return Err((
            "unresolved_assumption",
            format!(
                "the proof rests on names the environment does not declare: {:?}",
                rebuilt.axioms.unresolved
            ),
        ));
    }
    let declared: BTreeMap<&str, (&Vec<String>, &Term)> = rebuilt
        .assumptions
        .iter()
        .map(|(n, l, t)| (n.as_str(), (l, t)))
        .collect();
    let mut admitted = Vec::new();
    let mut goal = Vec::new();
    let mut disallowed = Vec::new();
    let mut allowed = Vec::new();
    for name in &rebuilt.axioms.axioms {
        let Some((levels, ty)) = declared.get(name.as_str()).copied() else {
            disallowed.push(format!(
                "{name} (not an assumption this environment declares)"
            ));
            continue;
        };
        // A standard axiom in its canonical form, over canonical datatypes, is allowed — even when a
        // theorem restates it: it is not new. Its name alone earns nothing.
        if allowed_statement(ctx, name, levels)
            .is_some_and(|expected| crate::defeq_for_test(ctx, ty, &expected))
        {
            allowed.push(name.clone());
            continue;
        }
        let admitted_by_name = crate::certificate::ADMITTED_AXIOMS.iter().any(|lean| {
            let components: Vec<&str> = lean.split('.').collect();
            super::reference::kernel_name_of_components(&components) == *name || lean == name
        });
        if admitted_by_name || universal_inhabitant(ctx, ty) {
            admitted.push(name.clone());
            continue;
        }
        let states_goal = if levels.len() == rebuilt.target_level_params.len() {
            let renamed = crate::instantiate_universes(
                ty,
                &levels
                    .iter()
                    .cloned()
                    .zip(
                        rebuilt
                            .target_level_params
                            .iter()
                            .cloned()
                            .map(Universe::Var),
                    )
                    .collect(),
            );
            crate::defeq_for_test(ctx, &renamed, &rebuilt.target_type)
        } else {
            false
        };
        if states_goal {
            goal.push(name.clone());
        } else {
            disallowed.push(name.clone());
        }
    }
    if !admitted.is_empty() {
        return Err(("admitted_proof", format!(
            "the proof rests on an admission — {} — reached through the transitive closure of its statement and proof (opaque theorems and definitions included); an admitted step proves anything and is never a solution",
            admitted.join(", ")
        )));
    }
    if !goal.is_empty() {
        return Err(("goal_axiom", format!(
            "the proof rests on an axiom stating the goal itself — {} — and an assumed goal is never a solution",
            goal.join(", ")
        )));
    }
    if !disallowed.is_empty() {
        return Err(("disallowed_assumption", format!(
            "the proof rests on assumptions outside the policy (propext, Quot.sound, Classical.choice in canonical form): {}",
            disallowed.join(", ")
        )));
    }
    Ok(if allowed.is_empty() {
        "the statement and proof rest on no assumption".to_string()
    } else {
        format!(
            "the statement and proof rest only on the allowed, canonical {}",
            allowed.join(", ")
        )
    })
}

// -------------------------------------------------------------------------------------------------
// Replay.
// -------------------------------------------------------------------------------------------------

/// Replay a package from its bytes alone, under the default limits.
pub fn replay(files: &PackageFiles) -> Receipt {
    replay_with(files, &PackageLimits::default())
}

/// Replay a package from its bytes alone, under `limits`.
pub fn replay_with(files: &PackageFiles, limits: &PackageLimits) -> Receipt {
    let mut receipt = Receipt::unstarted();
    let admitted = match admit(files, limits) {
        Ok(admitted) => admitted,
        Err((code, detail)) => {
            receipt.set(
                "lean_accepted",
                StageStatus::NotRun,
                "a replay never runs Lean",
            );
            receipt.set_with_code("fresh_replay", StageStatus::Failed, code, detail);
            finish_unreached(&mut receipt);
            return receipt;
        }
    };
    let producing = admitted
        .manifest
        .receipt
        .get("lean_accepted")
        .map(|r| r.status.clone())
        .unwrap_or_else(|| "absent".into());
    receipt.set(
        "lean_accepted",
        StageStatus::NotRun,
        format!("a replay never runs Lean; the producing run recorded lean_accepted = {producing}"),
    );
    // Every limit on the certificate's terms is enforced here, before any checking work.
    let depth = match admitted.certificate.admitted_depth(&limits.environment) {
        Ok(depth) => depth,
        Err(error) => {
            receipt.set_with_code(
                "fresh_replay",
                StageStatus::Failed,
                error.code.as_str(),
                error.message,
            );
            finish_unreached(&mut receipt);
            return receipt;
        }
    };
    // Reference::read holds raw JSON only. Preflight every source root that binding will
    // lower before allocating a Term or choosing the replay worker's stack size.
    let source = Reference::read(&admitted.export).and_then(|reference| {
        let target = reference.source_lean_name(&admitted.manifest.target.lean)?;
        let mut reads: Vec<String> = admitted
            .correspondence
            .rows
            .iter()
            .filter(|row| {
                row.origin == "authored" || row.origin == "prelude" && !row.lean.is_empty()
            })
            .map(|row| row.lean.clone())
            .collect();
        if let Some(axioms) = &admitted.manifest.assumptions.lean_print_axioms {
            reads.extend(axioms.iter().cloned());
        }
        source_binding_metrics(
            &reference,
            &admitted.certificate,
            &target,
            &reads,
            &limits.environment,
        )
        .map(|metrics| (reference, metrics))
    });
    let (reference, source_metrics) = match source {
        Ok(source) => source,
        Err(verdict) => {
            let code = verdict.reason().code().as_str();
            let detail = verdict.reason().message().to_string();
            receipt.set_with_code("translated", StageStatus::Failed, code, detail.clone());
            receipt.set_with_code("fresh_replay", StageStatus::Failed, code, detail);
            finish_unreached(&mut receipt);
            return receipt;
        }
    };
    let source_depth = if source_metrics.has_projection || source_metrics.has_generated {
        // Projections and canonical recursors may generate deeper terms than their source
        // expressions. Their generators must enforce this output envelope before allocation.
        // Reserve it for all subsequent checking and term destruction as well.
        source_metrics.depth.max(limits.environment.max_depth)
    } else {
        source_metrics.depth
    };
    let depth = depth.max(source_depth);
    let rungs = depth
        .saturating_mul(3)
        .saturating_add(crate::term_store::INLINE_DEPTH);
    let run = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("lean-package-replay".into())
            .stack_size(crate::term_store::stack_bytes_for_depth(rungs))
            .spawn_scoped(scope, || replay_admitted(&admitted, &reference, limits))
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p))
            })
    });
    match run {
        Ok(stages) => {
            for (name, (status, code, detail)) in stages {
                match code {
                    Some(code) => receipt.set_with_code(name, status, &code, detail),
                    None => receipt.set(name, status, detail),
                }
            }
        }
        Err(e) => receipt.set_with_code(
            "fresh_replay",
            StageStatus::Failed,
            "infrastructure",
            format!("cannot reserve the replay stack: {e}"),
        ),
    }
    let blocking = STAGES
        .iter()
        .filter(|s| !matches!(**s, "lean_accepted" | "fresh_replay"))
        .map(|s| (*s, receipt.status(s)))
        .find(|(_, status)| *status != StageStatus::Passed);
    finish_unreached(&mut receipt);
    match blocking {
        None => receipt.set(
            "fresh_replay",
            StageStatus::Passed,
            "every replay stage passed from the package bytes alone, with no importer state",
        ),
        Some((stage, StageStatus::Unsupported)) => receipt.set_with_code(
            "fresh_replay",
            StageStatus::Unsupported,
            "incomplete_replay",
            format!("{stage} is unsupported, so the replay is not complete"),
        ),
        Some((stage, status)) => receipt.set_with_code(
            "fresh_replay",
            StageStatus::Failed,
            "stage_failed",
            format!("{stage} {}", status.as_str()),
        ),
    }
    receipt
}

/// Say of every stage the replay never reached that it was not reached, and why.
fn finish_unreached(receipt: &mut Receipt) {
    let stop = receipt
        .first_failure()
        .map(|s| format!("{} failed", s.name));
    for name in STAGES {
        let stage = receipt.stage(name).expect("every stage");
        if stage.status == StageStatus::NotRun && stage.detail.is_empty() {
            let why = match &stop {
                Some(stop) => format!("not reached: the replay stopped when {stop}"),
                None => "not reached".to_string(),
            };
            receipt.set(name, StageStatus::NotRun, why);
        }
    }
}

/// A package that passed admission: limits, file set, digests, canonical encodings, formats.
struct Admitted {
    manifest: Manifest,
    certificate: EnvironmentCertificate,
    correspondence: CorrespondenceFile,
    export: String,
    document: Option<String>,
}

/// An unknown field is an extension this replay does not implement; anything else is malformed.
fn json_refusal(error: &serde_json::Error) -> &'static str {
    if error.to_string().contains("unknown field") {
        "unsupported_format"
    } else {
        "malformed"
    }
}

fn admit(
    files: &PackageFiles,
    limits: &PackageLimits,
) -> std::result::Result<Admitted, (&'static str, String)> {
    if files.len() > limits.max_files {
        return Err((
            "resource_limit",
            format!(
                "{} files exceed the limit of {}",
                files.len(),
                limits.max_files
            ),
        ));
    }
    let mut total = 0usize;
    for (path, bytes) in files {
        if let Some(why) = unsafe_path(path, limits) {
            return Err(("unsafe_path", format!("{path:?} is {why}")));
        }
        if bytes.len() > limits.max_file_bytes {
            return Err((
                "resource_limit",
                format!(
                    "{path} is {} bytes, beyond the limit of {}",
                    bytes.len(),
                    limits.max_file_bytes
                ),
            ));
        }
        total = total.saturating_add(bytes.len());
    }
    if total > limits.max_total_bytes {
        return Err((
            "resource_limit",
            format!(
                "the package is {total} bytes, beyond the limit of {}",
                limits.max_total_bytes
            ),
        ));
    }
    let manifest_bytes = files
        .get(MANIFEST)
        .ok_or(("missing_file", format!("the package has no {MANIFEST}")))?;
    let manifest_value: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|e| ("malformed", format!("{MANIFEST}: {e}")))?;
    match manifest_value.get("schema").and_then(|s| s.as_str()) {
        Some(schema) if PACKAGE_SCHEMAS.contains(&schema) => {}
        Some(other) => {
            return Err((
                "unsupported_format",
                format!(
                    "{MANIFEST} declares {other:?}; this replay reads {}",
                    PACKAGE_SCHEMAS.join(" and ")
                ),
            ))
        }
        None => {
            return Err((
                "unsupported_format",
                format!("{MANIFEST} declares no schema"),
            ))
        }
    }
    let manifest: Manifest = serde_json::from_value(manifest_value)
        .map_err(|e| (json_refusal(&e), format!("{MANIFEST}: {e}")))?;
    if canonical_json(&manifest) != *manifest_bytes {
        return Err((
            "non_canonical_encoding",
            format!("{MANIFEST} is not in its canonical encoding"),
        ));
    }
    // The file set is exactly the manifest's, and every digest matches.
    let listed: BTreeSet<&String> = manifest.files.keys().collect();
    let present: BTreeSet<&String> = files.keys().filter(|p| *p != MANIFEST).collect();
    if let Some(extra) = present.difference(&listed).next() {
        return Err((
            "unknown_file",
            format!("{extra} is not listed in the manifest"),
        ));
    }
    if let Some(missing) = listed.difference(&present).next() {
        return Err((
            "missing_file",
            format!("the manifest lists {missing}, which the package does not contain"),
        ));
    }
    for (path, digest) in &manifest.files {
        if sha256_hex(&files[path]) != *digest {
            return Err((
                "digest_mismatch",
                format!("{path}'s SHA-256 is not the manifest's"),
            ));
        }
    }
    if result_id_in(&manifest.schema, &manifest.target.lean, &manifest.files) != manifest.result_id
    {
        return Err((
            "result_id_mismatch",
            "the result identity does not match the schema, target and file digests".into(),
        ));
    }
    // Every listed file has a declared role.
    let mut roles: BTreeSet<String> = [CERTIFICATE, CORRESPONDENCE, EXPORT]
        .iter()
        .map(|s| s.to_string())
        .collect();
    roles.extend(manifest.document.iter().cloned());
    roles.extend(manifest.source.lean_sources.iter().cloned());
    for path in manifest.files.keys() {
        if !roles.contains(path) {
            return Err((
                "unknown_file",
                format!("{path} has no role in {}", manifest.schema),
            ));
        }
    }
    for role in &roles {
        if !manifest.files.contains_key(role) {
            return Err((
                "missing_file",
                format!("the package declares {role} but does not list it"),
            ));
        }
    }
    if let Some(document) = &manifest.document {
        if document.contains('/') || !document.ends_with(".lg") {
            return Err((
                "unsafe_path",
                format!("the document {document:?} is not a top-level .lg file"),
            ));
        }
    }
    for source in &manifest.source.lean_sources {
        if !source.starts_with("source/") || !source.ends_with(".lean") {
            return Err((
                "unsafe_path",
                format!("{source:?} is not a Lean source under source/"),
            ));
        }
    }
    if manifest.source.export != EXPORT
        || manifest.files.get(EXPORT) != Some(&manifest.source.export_sha256)
    {
        return Err((
            "manifest_mismatch",
            "the manifest's source identity does not name the carried export".into(),
        ));
    }
    if manifest.toolchain.prelude != PRELUDE_VERSION
        || ![OPAQUE_ENVIRONMENT_SCHEMA, ENVIRONMENT_SCHEMA]
            .contains(&manifest.toolchain.certificate_schema.as_str())
    {
        return Err(("unsupported_format", format!(
            "the package was produced for prelude {} and certificate schema {}; this replay is {PRELUDE_VERSION} and {OPAQUE_ENVIRONMENT_SCHEMA}",
            manifest.toolchain.prelude, manifest.toolchain.certificate_schema
        )));
    }
    let certificate_bytes = &files[CERTIFICATE];
    let certificate_text = std::str::from_utf8(certificate_bytes)
        .map_err(|_| ("malformed", format!("{CERTIFICATE} is not UTF-8")))?;
    let certificate = match read_certificate(certificate_text) {
        Ok(AnyCertificate::Environment(certificate)) => certificate,
        Ok(_) => {
            return Err((
                "unsupported_format",
                format!("{CERTIFICATE} is not an environment certificate"),
            ))
        }
        Err(error) => {
            let code = match error.code {
                EnvironmentErrorCode::UnsupportedSchema
                | EnvironmentErrorCode::IncompatiblePrelude => {
                    if error.code == EnvironmentErrorCode::IncompatiblePrelude {
                        "incompatible_prelude"
                    } else {
                        "unsupported_format"
                    }
                }
                _ if error.message.contains("unknown field") => "unsupported_format",
                _ => "malformed",
            };
            return Err((code, format!("{CERTIFICATE}: {}", error.message)));
        }
    };
    if manifest.toolchain.certificate_schema != certificate.schema {
        return Err((
            "manifest_mismatch",
            "the manifest's certificate schema differs from the carried certificate".into(),
        ));
    }
    if canonical_certificate(&certificate) != *certificate_bytes {
        return Err((
            "non_canonical_encoding",
            format!("{CERTIFICATE} is not in its canonical encoding"),
        ));
    }
    let correspondence_bytes = &files[CORRESPONDENCE];
    let correspondence: CorrespondenceFile = serde_json::from_slice(correspondence_bytes)
        .map_err(|e| (json_refusal(&e), format!("{CORRESPONDENCE}: {e}")))?;
    if correspondence.schema != CORRESPONDENCE_SCHEMA {
        return Err((
            "unsupported_format",
            format!("{CORRESPONDENCE} declares {:?}", correspondence.schema),
        ));
    }
    if canonical_json(&correspondence) != *correspondence_bytes {
        return Err((
            "non_canonical_encoding",
            format!("{CORRESPONDENCE} is not in its canonical encoding"),
        ));
    }
    let export = String::from_utf8(files[EXPORT].clone())
        .map_err(|_| ("malformed", format!("{EXPORT} is not UTF-8")))?;
    let header: serde_json::Value = serde_json::from_str(export.lines().next().unwrap_or(""))
        .map_err(|e| ("malformed", format!("{EXPORT} header: {e}")))?;
    if header["meta"]["format"]["version"] != "3.1.0" || manifest.source.format != "3.1.0" {
        return Err((
            "unsupported_format",
            format!("{EXPORT} is not a format 3.1.0 export"),
        ));
    }
    let document = match &manifest.document {
        Some(path) => Some(
            String::from_utf8(files[path].clone())
                .map_err(|_| ("malformed", format!("{path} is not UTF-8")))?,
        ),
        None => None,
    };
    Ok(Admitted {
        manifest,
        certificate,
        correspondence,
        export,
        document,
    })
}

type StageOutcome = (StageStatus, Option<String>, String);

/// Everything after admission, on a stack sized for certificate and source terms.
fn replay_admitted(
    admitted: &Admitted,
    reference: &Reference,
    limits: &PackageLimits,
) -> Vec<(&'static str, StageOutcome)> {
    let manifest = &admitted.manifest;
    let mut stages: Vec<(&'static str, StageOutcome)> = Vec::new();
    let target = match reference.source_lean_name(&manifest.target.lean) {
        Ok(name) => name,
        Err(verdict) => {
            stages.push((
                "translated",
                binding_failure(binding_error(
                    verdict,
                    "missing_source_declaration",
                    &manifest.target.lean,
                )),
            ));
            return stages;
        }
    };
    let mut source_work = match SourceWork::new(&limits.environment) {
        Ok(work) => work,
        Err(verdict) => {
            stages.push((
                "translated",
                binding_failure(binding_error(verdict, "missing_source_evidence", &target)),
            ));
            return stages;
        }
    };
    let checks = check_environment(
        &admitted.certificate,
        reference,
        &target,
        &limits.environment,
        &mut source_work,
    );
    let declaration = match &checks.rebuilt {
        Some(rebuilt) => source_work.declaration(&rebuilt.context, reference, &target),
        None => {
            stages.push(("primary_check", checks.primary));
            stages.push(("independent_recheck", checks.independent));
            stages.push(("translated", checks.translated));
            stages.push(("assumption_policy", checks.policy));
            stages.push((
                "emission_reparsed",
                not_run("the environment did not rebuild"),
            ));
            return stages;
        }
    };
    let declaration = match declaration {
        Ok(declaration) => declaration,
        Err(verdict) => {
            stages.push(("primary_check", checks.primary));
            stages.push(("independent_recheck", checks.independent));
            // An earlier body mismatch remains the first source failure.
            let translated = if checks.translated.0 == StageStatus::Passed {
                binding_failure(binding_error(
                    verdict,
                    "missing_source_declaration",
                    &target,
                ))
            } else {
                checks.translated
            };
            stages.push(("translated", translated));
            stages.push(("assumption_policy", checks.policy));
            stages.push((
                "emission_reparsed",
                not_run("the source target did not decode"),
            ));
            return stages;
        }
    };
    let mut translated = checks.translated.clone();
    if translated.0 == StageStatus::Passed {
        if let Err((code, detail)) = bind_manifest(
            manifest,
            &declaration,
            &admitted.correspondence,
            &admitted.certificate,
            &reference,
        ) {
            translated = failed(code, detail);
        }
    }
    // The contract is recomputed from the carried export and the bound target, never read from the
    // manifest or the producing run's report.
    let native = NativeDecisions::read(&admitted.export).closure_of(&target);
    if translated.0 == StageStatus::Passed
        && checks.primary.0 == StageStatus::Passed
        && checks.independent.0 == StageStatus::Passed
    {
        if let Err((code, detail)) = states_contract(manifest, &native) {
            translated = failed(code, detail);
        }
    }
    let mut policy = checks.policy.clone();
    if checks.primary.0 == StageStatus::Passed {
        if let Err(detail) = agree_with_lean(
            &reference,
            &native,
            manifest.assumptions.lean_print_axioms.as_deref(),
            &checks.transitive,
        ) {
            policy = failed("assumption_disagreement", detail);
        }
    }
    if policy.0 == StageStatus::Passed && manifest.assumptions.transitive != checks.transitive {
        policy = failed(
            "manifest_mismatch",
            format!(
                "the manifest lists the assumptions {:?}, but the proof rests on {:?}",
                manifest.assumptions.transitive, checks.transitive
            ),
        );
    }
    stages.push(("primary_check", checks.primary.clone()));
    stages.push(("independent_recheck", checks.independent.clone()));
    stages.push(("translated", translated));
    stages.push(("assumption_policy", policy));
    let emission = match (&checks.rebuilt, &admitted.document) {
        (None, _) => not_run("the environment did not rebuild"),
        (Some(_), None) => (
            StageStatus::Unsupported,
            Some("no_document".into()),
            format!(
                "the package carries no LOGOS document: {}",
                manifest.emission.detail
            ),
        ),
        (Some(rebuilt), Some(document)) => {
            replay_emission(rebuilt, document, &declaration, &reference, admitted)
        }
    };
    stages.push(("emission_reparsed", emission));
    stages
}

/// The manifest's claims about the target, and the correspondence, agree with the certificate and
/// the export.
fn bind_manifest(
    manifest: &Manifest,
    declaration: &super::reference::ReferenceDeclaration,
    correspondence: &CorrespondenceFile,
    certificate: &EnvironmentCertificate,
    reference: &Reference,
) -> std::result::Result<(), (&'static str, String)> {
    let kind = match declaration.kind {
        ReferenceKind::Theorem => "thm",
        ReferenceKind::Definition => "def",
        ReferenceKind::Opaque => "opaque",
        _ => "",
    };
    if manifest.target.kernel != declaration.name
        || certificate.target != declaration.name
        || manifest.target.kind != kind
        || manifest.target.level_params != declaration.level_params
    {
        return Err((
            "manifest_mismatch",
            format!(
                "the manifest's target ({}, {}, {}) is not the export's {}",
                manifest.target.kernel,
                manifest.target.kind,
                manifest.target.lean,
                declaration.lean_name
            ),
        ));
    }
    if correspondence.target != declaration.name {
        return Err((
            "correspondence_mismatch",
            format!(
                "the correspondence's target is {}, not {}",
                correspondence.target, declaration.name
            ),
        ));
    }
    let declared: BTreeSet<&str> = certificate.entries.iter().flat_map(|e| e.names()).collect();
    let mut kernels = BTreeSet::new();
    let mut logos_names = BTreeSet::new();
    let mut target_row = false;
    for row in &correspondence.rows {
        if let Some(logos) = &row.logos {
            if !logos_names.insert(logos.as_str()) {
                return Err((
                    "correspondence_mismatch",
                    format!("the LOGOS name {logos} carries two declarations"),
                ));
            }
        }
        // A constant the fresh LOGOS context derives itself (a recursor family member, an equation)
        // or holds in its prelude has no certificate declaration and no Lean name of its own. It is
        // bound by the emission stage: the document must be exactly the emitter's writing of the
        // rebuilt environment, and the emitter's verification re-derives every such constant.
        let derived =
            matches!(row.origin.as_str(), "synthesized" | "prelude") && row.kernel.is_empty();
        if derived {
            if !row.lean.is_empty() || row.logos.is_none() {
                return Err(("correspondence_mismatch", "a derived correspondence row must name its LOGOS constant and no Lean declaration".into()));
            }
            continue;
        }
        if row.kernel.is_empty() || !kernels.insert(row.kernel.as_str()) {
            return Err((
                "correspondence_mismatch",
                format!(
                    "{:?} has no kernel name, or two correspondence rows",
                    row.kernel
                ),
            ));
        }
        if row.origin != "prelude" && !declared.contains(row.kernel.as_str()) {
            return Err((
                "correspondence_mismatch",
                format!(
                    "the correspondence maps {}, which the certificate does not declare",
                    row.kernel
                ),
            ));
        }
        if row.origin == "authored" || row.origin == "prelude" && !row.lean.is_empty() {
            match reference.source_lean_name(&row.lean) {
                Ok(source) if source == row.kernel => {}
                Ok(source) => {
                    return Err((
                        "correspondence_mismatch",
                        format!(
                            "the correspondence maps Lean {} to {}, but the export's {} is {}",
                            row.lean, row.kernel, row.lean, source
                        ),
                    ))
                }
                Err(_) => {
                    return Err((
                        "correspondence_mismatch",
                        format!(
                            "the correspondence names Lean {}, which the export does not declare",
                            row.lean
                        ),
                    ))
                }
            }
        }
        if row.kernel == declaration.name {
            target_row = true;
            if row.lean != declaration.lean_name || row.logos != manifest.target.logos {
                return Err((
                    "correspondence_mismatch",
                    "the correspondence's target row is not the manifest's target".into(),
                ));
            }
        }
    }
    if !target_row {
        return Err((
            "correspondence_mismatch",
            "the correspondence has no row for the target".into(),
        ));
    }
    Ok(())
}

/// The document is exactly the emitter's writing of the rebuilt environment, its correspondence is
/// the correspondence file, it reparses in a fresh LOGOS session, and its target states the
/// reference translation of the Lean type.
fn replay_emission(
    rebuilt: &RebuiltSummary,
    document: &str,
    declaration: &super::reference::ReferenceDeclaration,
    reference: &Reference,
    admitted: &Admitted,
) -> StageOutcome {
    let proof_of = |key: &str| rebuilt.proofs.get(key).cloned();
    let emitted = match super::emit::emit_context(&rebuilt.context, &proof_of, &declaration.name) {
        Ok(emitted) => emitted,
        Err(verdict) => {
            return (
                if matches!(verdict, Verdict::Decline(_)) {
                    StageStatus::Unsupported
                } else {
                    StageStatus::Failed
                },
                Some(verdict.reason().code().as_str().to_string()),
                format!(
                    "the rebuilt environment could not be written as LOGOS: {}",
                    verdict.reason().message()
                ),
            )
        }
    };
    if emitted.source != document {
        return failed(
            "emission_mismatch",
            "the package's LOGOS document is not the emitter's writing of the rebuilt environment",
        );
    }
    let rows = match correspondence_rows(&admitted.certificate, reference, Some(&emitted)) {
        Ok(rows) => rows,
        Err(verdict) => return failed("emission_mismatch", verdict.reason().message().to_string()),
    };
    if rows != admitted.correspondence.rows {
        return failed(
            "correspondence_mismatch",
            "the correspondence file is not the document's correspondence",
        );
    }
    if admitted.manifest.target.logos.as_deref() != Some(emitted.target.as_str()) {
        return failed(
            "manifest_mismatch",
            "the manifest's LOGOS target is not the document's",
        );
    }
    match super::emit::check_statement(&emitted, declaration) {
        Ok(()) => (
            StageStatus::Passed,
            None,
            format!(
                "{} reparsed in a fresh LOGOS session; {} states the reference translation of {}",
                admitted.manifest.document.as_deref().unwrap_or(""),
                emitted.target,
                declaration.lean_name
            ),
        ),
        Err(fault) => failed("emission_mismatch", fault.to_string()),
    }
}

// -------------------------------------------------------------------------------------------------
// The package on disk.
// -------------------------------------------------------------------------------------------------

/// Read a package directory, enforcing the file-count, size and path limits before reading any
/// content. Symbolic links and special files are refused.
pub fn read_directory(
    dir: &std::path::Path,
    limits: &PackageLimits,
) -> std::result::Result<PackageFiles, String> {
    let mut files = PackageFiles::new();
    let mut total = 0usize;
    let mut work = vec![(dir.to_path_buf(), String::new())];
    while let Some((path, prefix)) = work.pop() {
        let entries = std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|n| format!("{n:?} is not a UTF-8 file name"))?;
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let metadata =
                std::fs::symlink_metadata(entry.path()).map_err(|e| format!("{relative}: {e}"))?;
            if metadata.file_type().is_symlink() {
                return Err(format!("{relative} is a symbolic link"));
            }
            if metadata.is_dir() {
                work.push((entry.path(), relative));
                continue;
            }
            if !metadata.is_file() {
                return Err(format!("{relative} is not a regular file"));
            }
            if let Some(why) = unsafe_path(&relative, limits) {
                return Err(format!("{relative:?} is {why}"));
            }
            if files.len() + 1 > limits.max_files {
                return Err(format!("more than {} files", limits.max_files));
            }
            let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            if size > limits.max_file_bytes {
                return Err(format!(
                    "{relative} is {size} bytes, beyond the limit of {}",
                    limits.max_file_bytes
                ));
            }
            total = total.saturating_add(size);
            if total > limits.max_total_bytes {
                return Err(format!(
                    "the package exceeds {} bytes",
                    limits.max_total_bytes
                ));
            }
            let bytes = std::fs::read(entry.path()).map_err(|e| format!("{relative}: {e}"))?;
            if bytes.len() != size {
                return Err(format!("{relative} changed while it was read"));
            }
            files.insert(relative, bytes);
        }
    }
    Ok(files)
}

/// Write a package to `dir` atomically: every file is written and synced in a staging directory
/// beside it, which is then renamed into place. An existing `dir` is never replaced — a published
/// result is immutable — and a failure leaves nothing behind.
pub fn publish(dir: &std::path::Path, files: &PackageFiles) -> std::io::Result<()> {
    use std::io::Write;
    if dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists", dir.display()),
        ));
    }
    let limits = PackageLimits::default();
    for path in files.keys() {
        if let Some(why) = unsafe_path(path, &limits) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{path:?} is {why}"),
            ));
        }
    }
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stem = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = parent.join(format!(".{stem}.staging-{}-{nonce}", std::process::id()));
    let result = (|| {
        std::fs::create_dir(&staging)?;
        for (path, bytes) in files {
            let target = staging.join(path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut file = std::fs::File::create(&target)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&staging, dir)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

/// The replay receipt as a `logos-lean-replay/1` document.
pub fn receipt_document(receipt: &Receipt, manifest_sha256: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "schema": REPLAY_SCHEMA,
        "replayed": receipt.replayed(),
        "verified": receipt.verified(),
        "manifest_sha256": manifest_sha256,
        "stages": receipt.to_json(),
        "trusted_base": TRUSTED_BASE,
    })
}

#[path = "package_source_work.rs"]
mod source_work;
use source_work::SourceWork;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/package/native_disclosure_controls.rs"]
mod native_disclosure_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/package/native_contract_controls.rs"]
mod native_contract_controls;
