//! Diagnostic bridge for translating Rust errors to LOGOS.
//!
//! Translates Rust borrow checker errors into friendly LOGOS error messages
//! using Socratic phrasing. Parses rustc JSON output and maps errors back
//! to LOGOS source locations using the [`SourceMap`].
//!
//! # Supported Error Codes
//!
//! | Rust Error | LOGOS Translation |
//! |------------|-------------------|
//! | E0382 | "Cannot use 'x' after giving it away" |
//! | E0505 | "Cannot borrow 'x' while it's borrowed elsewhere" |
//! | E0597 | "Reference 'x' cannot escape zone" |
//!
//! # Translation Flow
//!
//! ```text
//! rustc --error-format=json
//!           │
//!           ▼
//! ┌─────────────────────┐
//! │ parse_rustc_json()  │ Parse JSON diagnostics
//! └──────────┬──────────┘
//!            ▼
//! ┌─────────────────────┐
//! │ translate_diagnostics│ Map to LOGOS source
//! └──────────┬──────────┘
//!            ▼
//!    LogosError with friendly message

use crate::intern::Interner;
use crate::sourcemap::{OwnershipRole, SourceMap};
use crate::style::Style;
use crate::token::Span;
use serde::Deserialize;

/// A translated error message for LOGOS users.
#[derive(Debug, Clone)]
pub struct LogosError {
    pub title: String,
    pub explanation: String,
    pub logos_span: Option<Span>,
    pub suggestion: Option<String>,
}

impl std::fmt::Display for LogosError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}: {}", Style::bold_red("ownership error"), self.title)?;
        writeln!(f)?;
        writeln!(f, "{}", self.explanation)?;
        if let Some(suggestion) = &self.suggestion {
            writeln!(f)?;
            writeln!(f, "{}: {}", Style::cyan("suggestion"), suggestion)?;
        }
        Ok(())
    }
}

// =============================================================================
// Rustc JSON Diagnostic Types
// =============================================================================

/// Rustc JSON diagnostic message (subset of fields we need).
#[derive(Debug, Deserialize)]
pub struct RustcDiagnostic {
    pub message: String,
    pub code: Option<RustcCode>,
    pub level: String,
    pub spans: Vec<RustcSpan>,
    #[serde(default)]
    pub children: Vec<RustcDiagnostic>,
}

/// Error code from rustc (e.g., "E0382").
#[derive(Debug, Deserialize)]
pub struct RustcCode {
    /// The error code string (e.g., "E0382" for use-after-move).
    pub code: String,
}

/// Source location information from a rustc diagnostic.
///
/// Describes where in the generated Rust source an error occurred,
/// which is then mapped back to LOGOS source using the [`SourceMap`].
#[derive(Debug, Deserialize)]
pub struct RustcSpan {
    /// Path to the file containing the error.
    pub file_name: String,
    /// Starting line number (1-based).
    pub line_start: u32,
    /// Ending line number (1-based).
    pub line_end: u32,
    /// Starting column number (1-based).
    pub column_start: u32,
    /// Ending column number (1-based).
    pub column_end: u32,
    /// Whether this is the primary error location.
    pub is_primary: bool,
    /// Optional diagnostic label for this span.
    pub label: Option<String>,
    /// Source text lines with highlighting information.
    #[serde(default)]
    pub text: Vec<RustcSpanText>,
}

/// Source text with highlight range from a rustc diagnostic.
#[derive(Debug, Deserialize)]
pub struct RustcSpanText {
    /// The actual source text line.
    pub text: String,
    /// Column where highlighting starts (1-based).
    pub highlight_start: u32,
    /// Column where highlighting ends (1-based).
    pub highlight_end: u32,
}

/// Parsed rustc output: either a diagnostic or artifact info.
#[derive(Debug, Deserialize)]
#[serde(tag = "reason")]
#[serde(rename_all = "kebab-case")]
pub enum RustcMessage {
    CompilerMessage { message: RustcDiagnostic },
    #[serde(other)]
    Other,
}

// =============================================================================
// JSON Parsing
// =============================================================================

/// Parse rustc stderr output from `cargo build --message-format=json`.
pub fn parse_rustc_json(stderr: &str) -> Vec<RustcDiagnostic> {
    let mut diagnostics = Vec::new();

    for line in stderr.lines() {
        // Skip empty lines and non-JSON output
        if !line.starts_with('{') {
            continue;
        }

        match serde_json::from_str::<RustcMessage>(line) {
            Ok(RustcMessage::CompilerMessage { message }) => {
                if message.level == "error" {
                    diagnostics.push(message);
                }
            }
            Ok(RustcMessage::Other) => {} // Ignore artifacts, build-finished, etc.
            Err(_) => {} // Ignore malformed lines
        }
    }

    diagnostics
}

/// Extracts the error code (e.g., "E0382") from a diagnostic.
///
/// Returns `None` if the diagnostic has no associated error code.
pub fn get_error_code(diag: &RustcDiagnostic) -> Option<&str> {
    diag.code.as_ref().map(|c| c.code.as_str())
}

/// Extracts the primary source span from a diagnostic.
///
/// Diagnostics may have multiple spans; this returns the one marked
/// as primary (the main error location).
pub fn get_primary_span(diag: &RustcDiagnostic) -> Option<&RustcSpan> {
    diag.spans.iter().find(|s| s.is_primary)
}

/// Extract variable name from rustc error message.
/// Example: "use of moved value: `data`" -> "data"
fn extract_var_from_message(message: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = message.find(prefix)?;
    let after_prefix = &message[start + prefix.len()..];
    let end = after_prefix.find(suffix)?;
    Some(after_prefix[..end].to_string())
}

// =============================================================================
// Diagnostic Bridge
// =============================================================================

/// Translates rustc diagnostics into user-friendly LOGOS error messages.
///
/// Uses the source map to map Rust source locations back to LOGOS source,
/// and applies Socratic phrasing to explain ownership errors in terms of
/// LOGOS semantics (Give, Show, Zone).
pub struct DiagnosticBridge<'a> {
    /// Source map for translating Rust locations to LOGOS locations.
    source_map: &'a SourceMap,
    /// Interner for resolving symbol names.
    interner: &'a Interner,
}

impl<'a> DiagnosticBridge<'a> {
    pub fn new(source_map: &'a SourceMap, interner: &'a Interner) -> Self {
        Self { source_map, interner }
    }

    /// Translate a rustc diagnostic into a LOGOS error.
    pub fn translate(&self, diag: &RustcDiagnostic) -> Option<LogosError> {
        let code = get_error_code(diag)?;
        let span = get_primary_span(diag);

        match code {
            "E0382" => self.translate_use_after_move(diag, span),
            "E0505" => self.translate_move_while_borrowed(diag, span),
            "E0597" => self.translate_lifetime_error(diag, span),
            _ => self.translate_generic(diag, span),
        }
    }

    /// E0382: "use of moved value: `x`"
    /// LOGOS: "You already gave X away - you can't use it anymore"
    fn translate_use_after_move(&self, diag: &RustcDiagnostic, span: Option<&RustcSpan>) -> Option<LogosError> {
        let var_name = extract_var_from_message(&diag.message, "value: `", "`")
            .or_else(|| extract_var_from_message(&diag.message, "value `", "`"))?;

        let logos_span = span.and_then(|s| self.source_map.find_nearest_span(s.line_start));

        // Look up variable origin if available
        let (logos_name, role) = if let Some(origin) = self.source_map.get_var_origin(&var_name) {
            (self.interner.resolve(origin.logos_name).to_string(), Some(origin.role))
        } else {
            (var_name.clone(), None)
        };

        let explanation = match role {
            Some(OwnershipRole::GiveObject) => format!(
                "You gave '{}' away with a Give statement, so you can't use it anymore.\n\
                In LOGOS, 'Give X to Y' transfers ownership - X moves to Y and leaves your hands.\n\
                This is like handing someone a physical object: once given, you no longer have it.",
                logos_name
            ),
            Some(OwnershipRole::LetBinding) | None => format!(
                "The value '{}' was moved somewhere else and can't be used again.\n\
                Check if you used 'Give' or passed it to a function that took ownership.",
                logos_name
            ),
            _ => format!(
                "The value '{}' has been moved and is no longer available.",
                logos_name
            ),
        };

        let suggestion = Some(format!(
            "If you need to use '{}' after giving it away, either:\n\
             1. Use 'Show {} to Y' instead (this borrows, keeping ownership)\n\
             2. Use 'a copy of {}' before the Give",
            logos_name, logos_name, logos_name
        ));

        Some(LogosError {
            title: format!("Cannot use '{}' after giving it away", logos_name),
            explanation,
            logos_span,
            suggestion,
        })
    }

    /// E0505: "cannot move out of `x` because it is borrowed"
    /// LOGOS: "You're trying to give X away while someone is still looking at it"
    fn translate_move_while_borrowed(&self, diag: &RustcDiagnostic, span: Option<&RustcSpan>) -> Option<LogosError> {
        let var_name = extract_var_from_message(&diag.message, "out of `", "`")
            .or_else(|| extract_var_from_message(&diag.message, "move out of `", "`"))?;

        let logos_span = span.and_then(|s| self.source_map.find_nearest_span(s.line_start));

        let logos_name = if let Some(origin) = self.source_map.get_var_origin(&var_name) {
            self.interner.resolve(origin.logos_name).to_string()
        } else {
            var_name.clone()
        };

        let explanation = format!(
            "You showed '{}' to someone (creating a temporary view),\n\
            but then tried to give it away before they finished looking.\n\
            In LOGOS, 'Show' creates a promise that the data won't change or disappear\n\
            while being viewed. You can't break that promise by giving it away.",
            logos_name
        );

        let suggestion = Some(format!(
            "Make sure all 'Show' usages of '{}' complete before any 'Give'.\n\
            Alternatively, give away a copy: 'Give a copy of {} to Y'",
            logos_name, logos_name
        ));

        Some(LogosError {
            title: format!("Cannot give '{}' while it's being shown", logos_name),
            explanation,
            logos_span,
            suggestion,
        })
    }

    /// E0597: "borrowed value does not live long enough"
    /// LOGOS: "You can't take a reference outside its zone" (Hotel California)
    fn translate_lifetime_error(&self, diag: &RustcDiagnostic, span: Option<&RustcSpan>) -> Option<LogosError> {
        let logos_span = span.and_then(|s| self.source_map.find_nearest_span(s.line_start));

        // Check if this is zone-related by looking at the message and children
        let is_zone_related = diag.message.contains("borrowed")
            || diag.children.iter().any(|c| c.message.contains("dropped"));

        let explanation = if is_zone_related {
            "A value created inside a Zone cannot be referenced from outside.\n\
            Zones are memory arenas - when the Zone ends, everything inside it is released.\n\
            This is the 'Hotel California' rule: data can check in (be created),\n\
            but references can't check out (escape the Zone).".to_string()
        } else {
            "A borrowed reference is being used after the original value has gone away.\n\
            References are temporary views - they can't outlive what they're viewing.".to_string()
        };

        let suggestion = Some(
            "If you need the data after the Zone ends, either:\n\
             1. Move the data out with 'Give' before the Zone closes\n\
             2. Copy the data: 'Let result be a copy of zone_data'\n\
             3. Restructure so the computation completes inside the Zone".to_string()
        );

        Some(LogosError {
            title: "Reference cannot outlive its data".to_string(),
            explanation,
            logos_span,
            suggestion,
        })
    }

    /// Fallback for other errors - provide the raw message with context.
    fn translate_generic(&self, diag: &RustcDiagnostic, span: Option<&RustcSpan>) -> Option<LogosError> {
        let logos_span = span.and_then(|s| self.source_map.find_nearest_span(s.line_start));

        // Try to extract any variable name
        let var_hint = if let Some(start) = diag.message.find('`') {
            if let Some(end) = diag.message[start + 1..].find('`') {
                Some(&diag.message[start + 1..start + 1 + end])
            } else {
                None
            }
        } else {
            None
        };

        let explanation = if let Some(var) = var_hint {
            format!(
                "The Rust compiler reported an error involving '{}':\n{}",
                var, diag.message
            )
        } else {
            format!("The Rust compiler reported an error:\n{}", diag.message)
        };

        Some(LogosError {
            title: "Compilation error".to_string(),
            explanation,
            logos_span,
            suggestion: None,
        })
    }
}

/// Translates rustc diagnostics to LOGOS errors.
///
/// Iterates through all diagnostics and returns the first successfully
/// translated error. Uses the source map to map Rust error locations
/// back to LOGOS source positions.
pub fn translate_diagnostics(
    diagnostics: &[RustcDiagnostic],
    source_map: &SourceMap,
    interner: &Interner,
) -> Option<LogosError> {
    let bridge = DiagnosticBridge::new(source_map, interner);

    for diag in diagnostics {
        if let Some(error) = bridge.translate(diag) {
            return Some(error);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rustc_json_extracts_errors() {
        let json_output = r#"{"reason":"compiler-message","message":{"message":"use of moved value: `x`","code":{"code":"E0382"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":11,"is_primary":true,"label":null,"text":[]}],"children":[]}}
{"reason":"build-finished","success":false}"#;

        let diagnostics = parse_rustc_json(json_output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "use of moved value: `x`");
        assert_eq!(get_error_code(&diagnostics[0]), Some("E0382"));
    }

    #[test]
    fn extract_var_from_message_works() {
        assert_eq!(
            extract_var_from_message("use of moved value: `data`", "value: `", "`"),
            Some("data".to_string())
        );
        assert_eq!(
            extract_var_from_message("cannot move out of `x` because", "out of `", "`"),
            Some("x".to_string())
        );
    }

    #[test]
    fn translate_e0382_creates_friendly_error() {
        let interner = Interner::new();
        let source_map = SourceMap::new("Let data be 5.\nGive data to processor.".to_string());

        let diag = RustcDiagnostic {
            message: "use of moved value: `data`".to_string(),
            code: Some(RustcCode { code: "E0382".to_string() }),
            level: "error".to_string(),
            spans: vec![RustcSpan {
                file_name: "src/main.rs".to_string(),
                line_start: 3,
                line_end: 3,
                column_start: 10,
                column_end: 14,
                is_primary: true,
                label: None,
                text: vec![],
            }],
            children: vec![],
        };

        let bridge = DiagnosticBridge::new(&source_map, &interner);
        let error = bridge.translate(&diag).expect("Should translate");

        assert!(error.title.contains("data"));
        assert!(error.title.contains("giving it away"));
        assert!(error.explanation.contains("moved"));
        assert!(error.suggestion.is_some());
    }

    #[test]
    fn translate_e0597_creates_hotel_california_error() {
        let interner = Interner::new();
        let source_map = SourceMap::new("Inside a zone:\n    Let x be 5.".to_string());

        let diag = RustcDiagnostic {
            message: "borrowed value does not live long enough".to_string(),
            code: Some(RustcCode { code: "E0597".to_string() }),
            level: "error".to_string(),
            spans: vec![RustcSpan {
                file_name: "src/main.rs".to_string(),
                line_start: 5,
                line_end: 5,
                column_start: 1,
                column_end: 10,
                is_primary: true,
                label: None,
                text: vec![],
            }],
            children: vec![],
        };

        let bridge = DiagnosticBridge::new(&source_map, &interner);
        let error = bridge.translate(&diag).expect("Should translate");

        assert!(error.title.contains("outlive"));
        assert!(error.explanation.contains("Zone") || error.explanation.contains("borrowed"));
    }
}
