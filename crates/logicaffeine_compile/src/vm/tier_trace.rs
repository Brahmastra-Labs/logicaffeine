//! `LOGOS_TIER_TRACE` — one line per execution-tier hot-swap (HOTSWAP §P13).
//!
//! When the env var is set (and non-empty, non-`"0"`), the VM emits a line to stderr
//! every time a function body is swapped to a hotter tier: baseline bytecode → warm
//! bytecode (Axis-1) → native via forge (Axis-2) → native via AOT cdylib (Axis-3).
//! This is the "we must be able to know and understand" observability hook — it makes
//! the otherwise-invisible tier ladder legible without a debugger. The formatting is a
//! pure function so it can be asserted directly.

use std::sync::atomic::{AtomicU8, Ordering};

/// Tri-state cache of the env check (0 = unread, 1 = on, 2 = off) so the hot path
/// reads an atomic instead of the environment on every transition.
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Whether `LOGOS_TIER_TRACE` requests the trace (set, non-empty, not `"0"`).
pub fn trace_enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let on = std::env::var("LOGOS_TIER_TRACE")
                .map(|v| !v.is_empty() && v != "0")
                .unwrap_or(false);
            ENABLED.store(if on { 1 } else { 2 }, Ordering::Relaxed);
            on
        }
    }
}

/// The execution tier a function body runs at, coldest to hottest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecTier {
    /// The baseline bytecode in `program.code`.
    Bytecode,
    /// A re-optimized body in the warm side-table (Axis-1, browser-portable).
    Warm,
    /// A copy-and-patch native body installed by forge (Axis-2).
    NativeForge,
    /// A native body loaded from an AOT-compiled cdylib (Axis-3).
    NativeAot,
}

impl ExecTier {
    pub fn label(self) -> &'static str {
        match self {
            ExecTier::Bytecode => "bytecode",
            ExecTier::Warm => "warm",
            ExecTier::NativeForge => "native(forge)",
            ExecTier::NativeAot => "native(aot)",
        }
    }
}

/// Format one tier-transition line. Pure — the unit under test.
pub fn format_transition(fi: usize, name: &str, to: ExecTier) -> String {
    if name.is_empty() {
        format!("[tier] fn#{fi} -> {}", to.label())
    } else {
        format!("[tier] fn#{fi} '{name}' -> {}", to.label())
    }
}

/// Emit a transition line iff the trace is enabled.
pub fn trace_transition(fi: usize, name: &str, to: ExecTier) {
    if trace_enabled() {
        eprintln!("{}", format_transition(fi, name, to));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_lines_format_with_and_without_a_name() {
        assert_eq!(
            format_transition(3, "fib", ExecTier::Warm),
            "[tier] fn#3 'fib' -> warm"
        );
        assert_eq!(
            format_transition(7, "quicksort", ExecTier::NativeForge),
            "[tier] fn#7 'quicksort' -> native(forge)"
        );
        assert_eq!(
            format_transition(0, "", ExecTier::NativeAot),
            "[tier] fn#0 -> native(aot)"
        );
        assert_eq!(
            format_transition(1, "main", ExecTier::Bytecode),
            "[tier] fn#1 'main' -> bytecode"
        );
    }
}
