//! The Studio's status bar, in VS Code's manner: a strip along the bottom edge, under a line,
//! saying how many problems the language service found, where the editor's caret is and what
//! is selected, and which language the editor holds.

use dioxus::prelude::*;

use super::problems::{Problem, Severity};

/// What the Studio's editor tells the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorStatus {
    pub caret: Caret,
    pub problems: ProblemCount,
}

/// How many errors and warnings the language service found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProblemCount {
    pub errors: usize,
    pub warnings: usize,
}

impl ProblemCount {
    /// The errors and warnings among `problems`; hints and notes are not counted.
    pub fn of(problems: &[Problem]) -> ProblemCount {
        let count = |severity: Severity| problems.iter().filter(|p| p.severity == severity).count();
        ProblemCount { errors: count(Severity::Error), warnings: count(Severity::Warning) }
    }
}

/// Where the caret sits — its line and column, both counted from 1, the column in characters —
/// and how many characters the selection spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub line: usize,
    pub column: usize,
    pub selected: usize,
}

impl Default for Caret {
    fn default() -> Self {
        Caret { line: 1, column: 1, selected: 0 }
    }
}

impl Caret {
    /// The caret in `text` for a selection from `start` to `end`, offsets in UTF-16 code units
    /// as the browser counts them. The caret is the selection's moving end: `start` when the
    /// selection was made backwards, `end` otherwise.
    pub fn of(text: &str, start: usize, end: usize, backward: bool) -> Caret {
        let at = if backward { start } else { end };
        let mut caret = Caret::default();
        let mut offset = 0;
        for c in text.chars() {
            if offset >= at.max(end) {
                break;
            }
            if offset < at {
                if c == '\n' {
                    caret.line += 1;
                    caret.column = 1;
                } else {
                    caret.column += 1;
                }
            }
            if offset >= start && offset < end {
                caret.selected += 1;
            }
            offset += c.len_utf16();
        }
        caret
    }

    /// "Ln 3, Col 4", with "(2 selected)" after it while text is selected.
    pub fn label(&self) -> String {
        let place = format!("Ln {}, Col {}", self.line, self.column);
        if self.selected == 0 {
            place
        } else {
            format!("{place} ({} selected)", self.selected)
        }
    }
}

pub const STATUS_BAR_STYLE: &str = r#"
.studio-statusbar {
    flex: none;
    box-sizing: border-box;
    height: 25px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 0 12px;
    border-top: 2px solid var(--studio-resizer);
    background: var(--studio-panel-bg);
    color: var(--studio-text-secondary);
    font: 12px/1 -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    white-space: nowrap;
    overflow: hidden;
    user-select: none;
}
.statusbar-side {
    display: flex;
    align-items: center;
    gap: 16px;
    min-width: 0;
}
.statusbar-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
}
.statusbar-count {
    display: inline-flex;
    align-items: center;
    gap: 3px;
}
.statusbar-count svg {
    width: 13px;
    height: 13px;
    flex: none;
}
.statusbar-count.errors svg { color: #f14c4c; }
.statusbar-count.warnings svg { color: #cca700; }
"#;

/// VS Code's error mark: a circle with a cross.
const ERROR_MARK: &str = r#"<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><circle cx="8" cy="8" r="6.3"/><path d="M5.6 5.6l4.8 4.8M10.4 5.6l-4.8 4.8"/></svg>"#;
/// VS Code's warning mark: a triangle with an exclamation.
const WARNING_MARK: &str = r#"<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" aria-hidden="true"><path d="M8 2.2l6.3 11.3H1.7z"/><path d="M8 6.4v3.4M8 11.6v.2"/></svg>"#;

/// The strip itself. It reads `status` on its own, so a caret move redraws the strip and
/// nothing else. `indent` is the editor's indentation step in spaces. `problems` is false for an
/// editor no language service reads, where a count of none would claim a check that never ran.
#[component]
pub fn StatusBar(status: Signal<EditorStatus>, language: String, indent: usize, problems: bool) -> Element {
    let status = status();
    let ProblemCount { errors, warnings } = status.problems;
    let position = status.caret.label();
    rsx! {
        style { "{STATUS_BAR_STYLE}" }
        footer { class: "studio-statusbar", role: "status",
            div { class: "statusbar-side",
                if problems {
                    span {
                        class: "statusbar-item",
                        "data-status": "problems",
                        "data-errors": "{errors}",
                        "data-warnings": "{warnings}",
                        title: "{errors} errors, {warnings} warnings",
                        span { class: "statusbar-count errors", span { dangerous_inner_html: ERROR_MARK } "{errors}" }
                        span { class: "statusbar-count warnings", span { dangerous_inner_html: WARNING_MARK } "{warnings}" }
                    }
                }
            }
            div { class: "statusbar-side",
                span { class: "statusbar-item", "data-status": "position", "{position}" }
                span { class: "statusbar-item", "data-status": "indent", "Spaces: {indent}" }
                span { class: "statusbar-item", "data-status": "language", "{language}" }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/status_bar/tests.rs"]
mod tests;
