//! The Symbol Dictionary: what each symbol of a First-Order Logic formula means.
//!
//! [`symbols_in`] reads the symbols a formula shows, named as the view that printed it names them;
//! [`SymbolGroups`] lists them, each kind of symbol a group. The Studio sets the groups in a
//! collapsible block of its Logic column.

use crate::ui::components::logic_output::OutputFormat;
use dioxus::prelude::*;
use logicaffeine_language::formatter::{KripkeFormatter, SimpleFOLFormatter, UnicodeFormatter};
use logicaffeine_language::symbol_dict::{
    extract_symbols_for, group_symbols_by_kind, SymbolEntry, SymbolKind,
};

const SYMBOL_GROUPS_STYLE: &str = r#"
.symbol-groups {
    display: flex;
    flex-direction: column;
    gap: 14px;
}

/* Each kind is one group: its name, then every symbol beside what it means. */
.symbol-group-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
}

.symbol-entries {
    display: grid;
    grid-template-columns: max-content 1fr;
    column-gap: 16px;
    row-gap: 4px;
    align-items: baseline;
}

.symbol-entry {
    display: contents;
}

.symbol-glyph {
    font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', monospace;
    font-size: 14px;
    white-space: nowrap;
}

.symbol-desc {
    font-size: 13px;
    line-height: 1.45;
    color: rgba(229, 231, 235, 0.72);
}

.quantifier { color: #60a5fa; }
.connective { color: #f472b6; }
.modal { color: #c084fc; }
.temporal { color: #22d3ee; }
.predicate { color: #4ade80; }
.role { color: #fda4af; }
.variable { color: #fbbf24; }
.constant { color: #fb923c; }
"#;

/// The symbols `logic` shows, named as `view` — the view that printed it — names them: each view
/// names its tense and aspect operators its own way.
pub fn symbols_in(logic: &str, view: OutputFormat) -> Vec<SymbolEntry> {
    match view {
        OutputFormat::Unicode | OutputFormat::LaTeX => {
            extract_symbols_for(logic, &UnicodeFormatter)
        }
        OutputFormat::SimpleFOL => extract_symbols_for(logic, &SimpleFOLFormatter),
        OutputFormat::Kripke => extract_symbols_for(logic, &KripkeFormatter),
    }
}

/// A group's heading and colour class.
fn group_look(kind: SymbolKind) -> (&'static str, &'static str) {
    match kind {
        SymbolKind::Quantifier => ("Quantifiers", "quantifier"),
        SymbolKind::Connective => ("Connectives", "connective"),
        SymbolKind::Modal => ("Modality", "modal"),
        SymbolKind::Temporal => ("Tense & aspect", "temporal"),
        SymbolKind::Identity => ("Identity", "connective"),
        SymbolKind::Predicate => ("Predicates", "predicate"),
        SymbolKind::Role => ("Thematic roles", "role"),
        SymbolKind::Variable => ("Variables", "variable"),
        SymbolKind::Constant => ("Individuals", "constant"),
        SymbolKind::Punctuation => ("Punctuation", "variable"),
    }
}

/// The dictionary's entries: each kind of symbol a group, its name over every symbol beside what
/// it means.
#[component]
pub fn SymbolGroups(symbols: Vec<SymbolEntry>) -> Element {
    let grouped = group_symbols_by_kind(&symbols);
    rsx! {
        style { "{SYMBOL_GROUPS_STYLE}" }
        div { class: "symbol-groups",
            for (kind, entries) in grouped {
                {
                    let (heading, kind_class) = group_look(kind);
                    rsx! {
                        div { class: "symbol-group",
                            key: "{kind_class}-{heading}",
                            div { class: "symbol-group-title {kind_class}", "{heading}" }
                            div { class: "symbol-entries",
                                for entry in entries {
                                    div { class: "symbol-entry",
                                        key: "{entry.symbol}",
                                        span { class: "symbol-glyph {kind_class}", "{entry.usage}" }
                                        span { class: "symbol-desc", "{entry.description}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/symbol_dictionary/tests.rs"]
mod tests;
