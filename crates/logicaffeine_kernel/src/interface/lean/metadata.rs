//! Expression metadata and the elaborator `have` (`letFun`) policy.
//!
//! # What is translated, and what is lost
//!
//! Lean's kernel ignores metadata: `mdata` is a transparent wrapper, and the `nondep` flag of a
//! `letE` is "not verified by the kernel — an elaborator feature" (Lean 4.29.1, `Expr.letE`).
//! The reader therefore translates the kernel's term and records every metadata key it passes
//! through as an exact [`MetadataLoss`](super::MetadataLoss) (`SOURCE_METADATA`), never silently.
//!
//! Term-mode `have` reaches the reader in three shapes. Each has one defined behavior:
//!
//! 1. **`letFun v (fun x => b)`** — how Lean 4.14.0 elaborates `have` (`Lean.Meta.mkLetFun`), and
//!    the only spelling in the measured exports: the Mathlib and cedar Lean 4.29.1 exports carry
//!    the `letFun` constant and not one `nondep: true` let. The application is imported exactly as
//!    exported, against the exported `letFun` definition, which Lean's kernel unfolds like any
//!    definition. Nothing kernel-visible is lost: LOGOS accepts precisely the terms Lean's kernel
//!    accepts. What is *not* claimed is the elaborator's opacity — inside a Lean `have` the goal
//!    does not see the value, while a checked term may unfold it. That property never exists in a
//!    kernel term, so no translation of the term can preserve or break it; a source-level
//!    rendering that prints the application as a transparent `let` must say so.
//! 2. **An `mdata` entry keyed `letFun`** — an elaborator annotation claiming opacity on a
//!    transparent term. No contract defines what that obligation means for a kernel check, so the
//!    record is **declined** (`UnsupportedFeature`) rather than stripped like other metadata.
//! 3. **A `letE` with `nondep: true`** — the later elaborator's `have`. The kernel would treat it
//!    as an ordinary `let`, but translating it so would erase the marker without a record. It is
//!    **declined** (`UnsupportedFeature`) until a loss-recording contract and its regression tests
//!    exist; no measured input carries it.
//!
//! The refusals are the policy, not a gap in it: an accepted import never contains a `have`
//! marker that was dropped to obtain acceptance. `lean_semantics_controls` locks all three.

use super::{declined, DiagnosticCode, Result};
use serde_json::Value;

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
        if binding.get("nondep").and_then(Value::as_bool) == Some(true) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "a nondep (elaborator have) let requires the elaborator-have translation contract",
            ));
        }
    }
    Ok(())
}
