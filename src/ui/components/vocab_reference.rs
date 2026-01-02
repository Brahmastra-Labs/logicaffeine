//! Vocabulary Reference component
//!
//! A collapsible reference panel showing all common logic symbols
//! and vocabulary terms, accessible from anywhere in the learning experience.

use dioxus::prelude::*;

const VOCAB_REFERENCE_STYLE: &str = r#"
.vocab-reference-toggle {
    position: fixed;
    bottom: 24px;
    right: 24px;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: #667eea;
    border: none;
    color: #fff;
    font-size: 24px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 4px 20px rgba(102, 126, 234, 0.3);
    transition: transform 0.2s ease, box-shadow 0.2s ease;
    z-index: 1000;
}

.vocab-reference-toggle:hover {
    transform: scale(1.05);
    box-shadow: 0 6px 24px rgba(96, 165, 250, 0.4);
}

.vocab-reference-toggle.active {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary);
}

/* Attention animation - pulses after delay */
.vocab-reference-toggle.attention {
    animation: attentionPulse 2s ease-in-out 3;
}

@keyframes attentionPulse {
    0%, 100% {
        transform: scale(1);
        box-shadow: 0 4px 20px rgba(102, 126, 234, 0.3);
    }
    50% {
        transform: scale(1.15);
        box-shadow: 0 6px 30px rgba(102, 126, 234, 0.6);
    }
}

.vocab-reference-panel {
    position: fixed;
    bottom: 84px;
    right: 24px;
    width: 360px;
    max-height: 70vh;
    background: #12161c;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 12px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    z-index: 999;
    overflow: hidden;
    animation: slideUp 0.2s ease;
}

@keyframes slideUp {
    from {
        opacity: 0;
        transform: translateY(16px);
    }
    to {
        opacity: 1;
        transform: translateY(0);
    }
}

.vocab-panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--spacing-lg);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(255, 255, 255, 0.03);
}

.vocab-panel-title {
    font-size: var(--font-body-lg);
    font-weight: 700;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
}

.vocab-panel-close {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: var(--text-secondary);
    font-size: 14px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
}

.vocab-panel-close:hover {
    background: rgba(255, 255, 255, 0.15);
    color: var(--text-primary);
}

.vocab-panel-content {
    padding: var(--spacing-lg);
    max-height: calc(70vh - 60px);
    overflow-y: auto;
}

.vocab-section {
    margin-bottom: var(--spacing-xl);
}

.vocab-section:last-child {
    margin-bottom: 0;
}

.vocab-section-title {
    font-size: var(--font-caption-md);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: var(--spacing-md);
    display: flex;
    align-items: center;
    gap: var(--spacing-xs);
}

.vocab-section-title.quantifiers {
    color: #60a5fa;
}

.vocab-section-title.connectives {
    color: #f472b6;
}

.vocab-section-title.predicates {
    color: #4ade80;
}

.vocab-section-title.terms {
    color: #a78bfa;
}

.vocab-items {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-xs);
}

.vocab-item {
    display: flex;
    align-items: center;
    gap: var(--spacing-md);
    padding: var(--spacing-sm) var(--spacing-md);
    background: rgba(255, 255, 255, 0.03);
    border-radius: var(--radius-md);
    transition: background 0.15s ease;
}

.vocab-item:hover {
    background: rgba(255, 255, 255, 0.06);
}

.vocab-symbol {
    font-family: var(--font-mono);
    font-size: var(--font-body-lg);
    min-width: 36px;
    text-align: center;
    font-weight: 600;
}

.vocab-symbol.quantifier {
    color: #60a5fa;
}

.vocab-symbol.connective {
    color: #f472b6;
}

.vocab-symbol.predicate {
    color: #4ade80;
}

.vocab-info {
    flex: 1;
}

.vocab-name {
    font-size: var(--font-body-sm);
    font-weight: 600;
    color: var(--text-primary);
}

.vocab-meaning {
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.vocab-term-item {
    padding: var(--spacing-md);
    background: rgba(255, 255, 255, 0.03);
    border-radius: var(--radius-md);
    margin-bottom: var(--spacing-sm);
}

.vocab-term-name {
    font-weight: 600;
    color: var(--color-accent-purple);
    margin-bottom: var(--spacing-xs);
}

.vocab-term-def {
    font-size: var(--font-body-sm);
    color: var(--text-secondary);
    line-height: 1.5;
}

/* Search box */
.vocab-search {
    margin-bottom: var(--spacing-lg);
}

.vocab-search-input {
    width: 100%;
    padding: var(--spacing-sm) var(--spacing-md);
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    font-size: var(--font-body-sm);
    outline: none;
    transition: border-color 0.2s ease;
}

.vocab-search-input:focus {
    border-color: var(--color-accent-blue);
}

.vocab-search-input::placeholder {
    color: var(--text-tertiary);
}
"#;

/// Symbol entry for the reference
struct SymbolRef {
    symbol: &'static str,
    name: &'static str,
    meaning: &'static str,
    category: &'static str,
}

/// Vocabulary term entry
struct VocabTerm {
    term: &'static str,
    definition: &'static str,
}

/// Get all reference symbols
fn get_symbols() -> Vec<SymbolRef> {
    vec![
        // Quantifiers
        SymbolRef { symbol: "∀", name: "Universal", meaning: "for all", category: "quantifier" },
        SymbolRef { symbol: "∃", name: "Existential", meaning: "there exists", category: "quantifier" },
        SymbolRef { symbol: "∃!", name: "Unique", meaning: "there exists exactly one", category: "quantifier" },
        // Connectives
        SymbolRef { symbol: "∧", name: "Conjunction", meaning: "and", category: "connective" },
        SymbolRef { symbol: "∨", name: "Disjunction", meaning: "or", category: "connective" },
        SymbolRef { symbol: "→", name: "Implication", meaning: "if...then", category: "connective" },
        SymbolRef { symbol: "↔", name: "Biconditional", meaning: "if and only if", category: "connective" },
        SymbolRef { symbol: "¬", name: "Negation", meaning: "not", category: "connective" },
        // Modal
        SymbolRef { symbol: "□", name: "Necessity", meaning: "necessarily", category: "connective" },
        SymbolRef { symbol: "◇", name: "Possibility", meaning: "possibly", category: "connective" },
        // Identity
        SymbolRef { symbol: "=", name: "Identity", meaning: "equals", category: "predicate" },
        SymbolRef { symbol: "≠", name: "Non-identity", meaning: "not equal to", category: "predicate" },
    ]
}

/// Get vocabulary terms
fn get_vocab_terms() -> Vec<VocabTerm> {
    vec![
        VocabTerm {
            term: "Predicate",
            definition: "A property or relation, written as a capitalized name with arguments in parentheses. E.g., Happy(alice)"
        },
        VocabTerm {
            term: "Constant",
            definition: "A name for a specific individual, written in lowercase. E.g., alice, bob, socrates"
        },
        VocabTerm {
            term: "Variable",
            definition: "A placeholder that can refer to any individual, typically x, y, z"
        },
        VocabTerm {
            term: "Valid Argument",
            definition: "An argument where the conclusion necessarily follows from the premises"
        },
        VocabTerm {
            term: "Sound Argument",
            definition: "A valid argument with premises that are actually true"
        },
        VocabTerm {
            term: "Domain",
            definition: "The set of all objects that variables can refer to"
        },
        VocabTerm {
            term: "Scope",
            definition: "The part of a formula to which a quantifier or connective applies"
        },
    ]
}

/// Props for VocabReference
#[derive(Props, Clone, PartialEq)]
pub struct VocabReferenceProps {
    /// Whether to show the panel initially
    #[props(default = false)]
    pub initial_open: bool,
}

/// Vocabulary Reference floating panel
#[component]
pub fn VocabReference(props: VocabReferenceProps) -> Element {
    let mut is_open = use_signal(|| props.initial_open);
    let mut search_query = use_signal(|| String::new());
    let mut show_attention = use_signal(|| false);

    // Trigger attention animation after 10 seconds if not opened
    use_effect(move || {
        spawn(async move {
            // Wait 10 seconds
            gloo_timers::future::TimeoutFuture::new(10_000).await;
            // Only show attention if panel hasn't been opened
            if !*is_open.peek() {
                show_attention.set(true);
            }
        });
    });

    let symbols = get_symbols();
    let vocab_terms = get_vocab_terms();

    // Filter based on search
    let query = search_query.read().to_lowercase();
    let filtered_symbols: Vec<_> = if query.is_empty() {
        symbols.iter().collect()
    } else {
        symbols.iter().filter(|s| {
            s.symbol.to_lowercase().contains(&query) ||
            s.name.to_lowercase().contains(&query) ||
            s.meaning.to_lowercase().contains(&query)
        }).collect()
    };

    let filtered_terms: Vec<_> = if query.is_empty() {
        vocab_terms.iter().collect()
    } else {
        vocab_terms.iter().filter(|t| {
            t.term.to_lowercase().contains(&query) ||
            t.definition.to_lowercase().contains(&query)
        }).collect()
    };

    // Group symbols by category
    let quantifiers: Vec<_> = filtered_symbols.iter().filter(|s| s.category == "quantifier").collect();
    let connectives: Vec<_> = filtered_symbols.iter().filter(|s| s.category == "connective").collect();
    let predicates: Vec<_> = filtered_symbols.iter().filter(|s| s.category == "predicate").collect();

    rsx! {
        style { "{VOCAB_REFERENCE_STYLE}" }

        // Toggle button
        button {
            class: {
                let open = *is_open.read();
                let attention = *show_attention.read();
                if open {
                    "vocab-reference-toggle active"
                } else if attention {
                    "vocab-reference-toggle attention"
                } else {
                    "vocab-reference-toggle"
                }
            },
            onclick: move |_| {
                let current = *is_open.read();
                is_open.set(!current);
                // Stop attention animation once clicked
                show_attention.set(false);
            },
            title: "Symbol & Vocabulary Reference",
            if *is_open.read() { "×" } else { "📖" }
        }

        // Panel
        if *is_open.read() {
            div { class: "vocab-reference-panel",
                // Header
                div { class: "vocab-panel-header",
                    div { class: "vocab-panel-title",
                        "📖 Reference"
                    }
                    button {
                        class: "vocab-panel-close",
                        onclick: move |_| is_open.set(false),
                        "×"
                    }
                }

                // Content
                div { class: "vocab-panel-content",
                    // Search
                    div { class: "vocab-search",
                        input {
                            class: "vocab-search-input",
                            r#type: "text",
                            placeholder: "Search symbols or terms...",
                            value: "{search_query}",
                            oninput: move |e| search_query.set(e.value()),
                        }
                    }

                    // Quantifiers
                    if !quantifiers.is_empty() {
                        div { class: "vocab-section",
                            div { class: "vocab-section-title quantifiers", "Quantifiers" }
                            div { class: "vocab-items",
                                for sym in quantifiers {
                                    div { class: "vocab-item",
                                        span { class: "vocab-symbol quantifier", "{sym.symbol}" }
                                        div { class: "vocab-info",
                                            div { class: "vocab-name", "{sym.name}" }
                                            div { class: "vocab-meaning", "{sym.meaning}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Connectives
                    if !connectives.is_empty() {
                        div { class: "vocab-section",
                            div { class: "vocab-section-title connectives", "Connectives" }
                            div { class: "vocab-items",
                                for sym in connectives {
                                    div { class: "vocab-item",
                                        span { class: "vocab-symbol connective", "{sym.symbol}" }
                                        div { class: "vocab-info",
                                            div { class: "vocab-name", "{sym.name}" }
                                            div { class: "vocab-meaning", "{sym.meaning}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Predicates
                    if !predicates.is_empty() {
                        div { class: "vocab-section",
                            div { class: "vocab-section-title predicates", "Predicates & Relations" }
                            div { class: "vocab-items",
                                for sym in predicates {
                                    div { class: "vocab-item",
                                        span { class: "vocab-symbol predicate", "{sym.symbol}" }
                                        div { class: "vocab-info",
                                            div { class: "vocab-name", "{sym.name}" }
                                            div { class: "vocab-meaning", "{sym.meaning}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Vocabulary Terms
                    if !filtered_terms.is_empty() {
                        div { class: "vocab-section",
                            div { class: "vocab-section-title terms", "Key Terms" }
                            for term in filtered_terms {
                                div { class: "vocab-term-item",
                                    div { class: "vocab-term-name", "{term.term}" }
                                    div { class: "vocab-term-def", "{term.definition}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
