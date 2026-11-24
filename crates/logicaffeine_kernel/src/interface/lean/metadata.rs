//! Expression metadata and the elaborator `have` policy.
//!
//! # What is translated, and what is lost
//!
//! Lean's kernel ignores metadata: `mdata` is a transparent wrapper, and the `nondep` flag of a
//! `letE` is "not verified by the kernel — an elaborator feature" (Lean 4.28.0 `Expr.lean`, the
//! `letE` constructor). The reader therefore translates the kernel's term and records every
//! metadata key it passes through as an exact [`MetadataLoss`](super::MetadataLoss)
//! (`SOURCE_METADATA`), never silently.
//!
//! Term-mode `have` reaches the reader in three shapes. Each has one defined behavior:
//!
//! 1. **`letFun v (fun x => b)`** — how Lean 4.14.0 elaborates `have` (`Lean.Meta.mkLetFun`). The
//!    application is imported exactly as exported, against the exported `letFun` definition, which
//!    Lean's kernel unfolds like any definition. Nothing kernel-visible is lost: LOGOS accepts
//!    precisely the terms Lean's kernel accepts. What is *not* claimed is the elaborator's opacity —
//!    inside a Lean `have` the goal does not see the value, while a checked term may unfold it. That
//!    property never exists in a kernel term, so no translation of the term can preserve or break
//!    it; a source-level rendering that prints the application as a transparent `let` must say so.
//! 2. **An `mdata` entry keyed `letFun`** — an elaborator annotation claiming opacity on a
//!    transparent term. No contract defines what that obligation means for a kernel check, so the
//!    record is **declined** (`UnsupportedFeature`) rather than stripped like other metadata.
//! 3. **A `letE` with `nondep: true`** — how the later elaborator (every release from 4.28.0 the
//!    registry admits) writes `have`: the elaborator-have translation contract. The let is
//!    translated exactly as Lean's kernel reads it — an ordinary `let`, its type, value, binder
//!    and body unchanged, checked by both kernels exactly as the same let with the flag false — and
//!    the flag is disclosed at its expression as the `SOURCE_METADATA` loss
//!    `letE.nondep = true`. Nothing is asserted about the elaborator's opacity (as in 1), and a
//!    dependent let is never rewritten into a lambda application. A `false` flag is no loss. The
//!    kernel core (`dag`) reads the same let the same way, so the two kernels agree on it, and a
//!    package's replay recomputes every disclosure from the export it carries.
//!
//! An accepted import never contains a `have` marker that was dropped without a record.
//! `lean_semantics_controls` and `lean_nondep_contract` lock all three.

use super::{declined, DiagnosticCode, Result};
use serde_json::Value;

/// The `SOURCE_METADATA` key under which a nondependent let's flag is disclosed.
pub const NONDEP_KEY: &str = "letE.nondep";

/// Validate the metadata an expression record carries before it is admitted to the DAG.
pub(super) fn check(record: &Value) -> Result<()> {
    if let Some(metadata) = record.get("mdata") {
        let fields = metadata.as_object().ok_or_else(|| {
            declined(DiagnosticCode::MalformedExport, "malformed metadata record")
        })?;
        if fields.len() != 2 || !fields.contains_key("expr") || !fields.contains_key("data") {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                "unknown metadata record fields",
            ));
        }
        let data = metadata["data"]
            .as_object()
            .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "malformed metadata map"))?;
        if data.values().any(|v| !v.is_string()) {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                "metadata values must use the exporter's string representation",
            ));
        }
        if data.contains_key("letFun") {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "letFun metadata requires the elaborator-have translation contract",
            ));
        }
    }
    if let Some(binding) = record.get("letE") {
        if binding.get("nondep").is_some_and(|flag| !flag.is_boolean()) {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                "letE.nondep must be a boolean",
            ));
        }
    }
    Ok(())
}

/// Whether an admitted expression record is a nondependent let — the elaborator's `have`.
pub(super) fn nondependent(record: &Value) -> bool {
    record
        .get("letE")
        .and_then(|binding| binding.get("nondep"))
        .and_then(Value::as_bool)
        == Some(true)
}
