//! Symbol Dictionary component for displaying FOL symbol meanings.
//!
//! Automatically extracts symbols from a First-Order Logic formula
//! and displays them grouped by category with descriptions.

use dioxus::prelude::*;
use crate::symbol_dict::{extract_symbols, group_symbols_by_kind, SymbolKind};

const SYMBOL_DICT_STYLE: &str = r#"
.symbol-dictionary {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 12px;
    padding: 16px;
    margin-top: 16px;
}

.symbol-dictionary.collapsed {
    padding: 12px 16px;
    cursor: pointer;
}

.symbol-dictionary.collapsed:hover {
    background: rgba(255, 255, 255, 0.05);
}

.symbol-dict-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229, 231, 235, 0.56);
    margin-bottom: 12px;
}

.symbol-dictionary.collapsed .symbol-dict-header {
    margin-bottom: 0;
}

.symbol-dict-toggle {
    color: rgba(229, 231, 235, 0.4);
    font-size: 14px;
    cursor: pointer;
    transition: transform 0.2s ease;
}

.symbol-dictionary.collapsed .symbol-dict-toggle {
    transform: rotate(-90deg);
}

.symbol-dict-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.symbol-group {
    margin-bottom: 0;
}

.symbol-group-title {
    font-size: 11px;
    font-weight: 600;
    color: #a78bfa;
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.symbol-group-title.quantifier {
    color: #60a5fa;
}

.symbol-group-title.connective {
    color: #f472b6;
}

.symbol-group-title.modal {
    color: #c084fc;
}

.symbol-group-title.predicate {
    color: #4ade80;
}

.symbol-group-title.variable {
    color: #fbbf24;
}

.symbol-group-title.constant {
    color: #fb923c;
}

.symbol-entries {
    display: flex;
    flex-direction: column;
    gap: 4px;
}

.symbol-entry {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px;
    border-radius: 6px;
    transition: background 0.15s ease;
}

.symbol-entry:hover {
    background: rgba(255, 255, 255, 0.05);
}

.symbol-glyph {
    font-size: 18px;
    font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', monospace;
    color: #60a5fa;
    min-width: 32px;
    text-align: center;
}

.symbol-glyph.quantifier {
    color: #60a5fa;
}

.symbol-glyph.connective {
    color: #f472b6;
}

.symbol-glyph.modal {
    color: #c084fc;
}

.symbol-glyph.predicate {
    color: #4ade80;
}

.symbol-glyph.variable {
    color: #fbbf24;
}

.symbol-glyph.constant {
    color: #fb923c;
}

.symbol-desc {
    font-size: 13px;
    color: rgba(229, 231, 235, 0.72);
}

.symbol-dictionary-empty {
    color: rgba(229, 231, 235, 0.4);
    font-size: 13px;
    text-align: center;
    padding: 8px;
}

/* Compact inline variant */
.symbol-dictionary.inline {
    padding: 12px;
    margin-top: 8px;
}

.symbol-dictionary.inline .symbol-dict-header {
    margin-bottom: 8px;
}

.symbol-dictionary.inline .symbol-entries {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
}

.symbol-dictionary.inline .symbol-entry {
    padding: 4px 8px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 4px;
}

.symbol-dictionary.inline .symbol-glyph {
    font-size: 14px;
    min-width: 20px;
}

.symbol-dictionary.inline .symbol-desc {
    font-size: 11px;
}
"#;

/// Props for the SymbolDictionary component
#[derive(Props, Clone, PartialEq)]
pub struct SymbolDictionaryProps {
    /// The FOL formula to extract symbols from
    logic: String,
    /// Whether to start collapsed
    #[props(default = false)]
    collapsed: bool,
    /// Use compact inline variant
    #[props(default = false)]
    inline: bool,
}

/// Symbol Dictionary component that auto-generates from FOL output
#[component]
pub fn SymbolDictionary(props: SymbolDictionaryProps) -> Element {
    let mut is_collapsed = use_signal(|| props.collapsed);

    let symbols = extract_symbols(&props.logic);

    if symbols.is_empty() {
        return rsx! { "" };
    }

    let grouped = group_symbols_by_kind(&symbols);

    let container_class = format!(
        "symbol-dictionary{}{}",
        if *is_collapsed.read() { " collapsed" } else { "" },
        if props.inline { " inline" } else { "" }
    );

    rsx! {
        style { "{SYMBOL_DICT_STYLE}" }
        div {
            class: "{container_class}",
            onclick: move |_| {
                if *is_collapsed.read() {
                    is_collapsed.set(false);
                }
            },

            div { class: "symbol-dict-header",
                span { "Symbol Dictionary" }
                span {
                    class: "symbol-dict-toggle",
                    onclick: move |e| {
                        e.stop_propagation();
                        let current = *is_collapsed.read();
                        is_collapsed.set(!current);
                    },
                    "▼"
                }
            }

            if !*is_collapsed.read() {
                div { class: "symbol-dict-content",
                    for (kind, entries) in grouped {
                        {
                            let kind_class = match kind {
                                SymbolKind::Quantifier => "quantifier",
                                SymbolKind::Connective => "connective",
                                SymbolKind::Modal => "modal",
                                SymbolKind::Predicate => "predicate",
                                SymbolKind::Variable => "variable",
                                SymbolKind::Constant => "constant",
                                SymbolKind::Temporal => "temporal",
                                SymbolKind::Identity => "connective",
                                SymbolKind::Punctuation => "variable",
                            };

                            rsx! {
                                div { class: "symbol-group",
                                    key: "{kind_class}",
                                    div { class: "symbol-group-title {kind_class}",
                                        "{kind.label()}"
                                    }
                                    div { class: "symbol-entries",
                                        for entry in entries {
                                            div { class: "symbol-entry",
                                                key: "{entry.symbol}",
                                                span { class: "symbol-glyph {kind_class}",
                                                    "{entry.symbol}"
                                                }
                                                span { class: "symbol-desc",
                                                    "{entry.description}"
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
        }
    }
}

/// Compact inline symbol legend for quick reference
#[component]
pub fn SymbolLegend(logic: String) -> Element {
    rsx! {
        SymbolDictionary {
            logic: logic,
            inline: true,
            collapsed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kind_class_names() {
        assert_eq!(SymbolKind::Quantifier.label(), "Quantifier");
        assert_eq!(SymbolKind::Connective.label(), "Connective");
    }
}
