//! Symbol palette component for Math mode.

use dioxus::prelude::*;

const SYMBOL_PALETTE_STYLE: &str = r#"
.symbol-palette {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #12161c;
    overflow: hidden;
}

.symbol-palette-header {
    padding: 12px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(255, 255, 255, 0.02);
}

.symbol-palette-title {
    font-size: 14px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.9);
}

.symbol-search {
    margin-top: 8px;
}

.symbol-search input {
    width: 100%;
    padding: 8px 10px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.9);
    font-size: 13px;
}

.symbol-search input::placeholder {
    color: rgba(255, 255, 255, 0.3);
}

.symbol-search input:focus {
    outline: none;
    border-color: rgba(102, 126, 234, 0.5);
}

.symbol-categories {
    flex: 1;
    overflow: auto;
    padding: 8px;
}

.symbol-category {
    margin-bottom: 16px;
}

.symbol-category-header {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(255, 255, 255, 0.4);
    margin-bottom: 8px;
    padding: 0 4px;
}

.symbol-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(36px, 1fr));
    gap: 4px;
}

.symbol-btn {
    aspect-ratio: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.8);
    font-size: 16px;
    cursor: pointer;
    transition: all 0.15s ease;
}

.symbol-btn:hover {
    background: rgba(102, 126, 234, 0.15);
    border-color: rgba(102, 126, 234, 0.3);
    color: #667eea;
}

.symbol-btn:active {
    transform: scale(0.95);
}

.symbol-btn[title]:hover::after {
    content: attr(title);
    position: absolute;
    bottom: 100%;
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 8px;
    background: #1a1f27;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    font-size: 11px;
    white-space: nowrap;
    z-index: 10;
}

/* Mobile adjustments */
@media (max-width: 768px) {
    .symbol-grid {
        grid-template-columns: repeat(auto-fill, minmax(44px, 1fr));
    }

    .symbol-btn {
        font-size: 18px;
    }
}
"#;

/// A symbol category with a name and list of symbols.
struct SymbolCategory {
    name: &'static str,
    symbols: &'static [(&'static str, &'static str)], // (symbol, latex)
}

const SYMBOL_CATEGORIES: &[SymbolCategory] = &[
    SymbolCategory {
        name: "Greek",
        symbols: &[
            ("\u{03B1}", "\\alpha"),
            ("\u{03B2}", "\\beta"),
            ("\u{03B3}", "\\gamma"),
            ("\u{03B4}", "\\delta"),
            ("\u{03B5}", "\\epsilon"),
            ("\u{03B6}", "\\zeta"),
            ("\u{03B7}", "\\eta"),
            ("\u{03B8}", "\\theta"),
            ("\u{03BB}", "\\lambda"),
            ("\u{03BC}", "\\mu"),
            ("\u{03C0}", "\\pi"),
            ("\u{03C3}", "\\sigma"),
            ("\u{03C6}", "\\phi"),
            ("\u{03C8}", "\\psi"),
            ("\u{03C9}", "\\omega"),
            ("\u{0393}", "\\Gamma"),
            ("\u{0394}", "\\Delta"),
            ("\u{03A3}", "\\Sigma"),
            ("\u{03A6}", "\\Phi"),
            ("\u{03A9}", "\\Omega"),
        ],
    },
    SymbolCategory {
        name: "Logic",
        symbols: &[
            ("\u{2200}", "\\forall"),
            ("\u{2203}", "\\exists"),
            ("\u{2204}", "\\nexists"),
            ("\u{00AC}", "\\neg"),
            ("\u{2227}", "\\land"),
            ("\u{2228}", "\\lor"),
            ("\u{2192}", "\\to"),
            ("\u{2194}", "\\leftrightarrow"),
            ("\u{21D2}", "\\Rightarrow"),
            ("\u{21D4}", "\\Leftrightarrow"),
            ("\u{22A2}", "\\vdash"),
            ("\u{22A8}", "\\models"),
            ("\u{22A4}", "\\top"),
            ("\u{22A5}", "\\bot"),
            ("\u{25A1}", "\\Box"),
            ("\u{25C7}", "\\Diamond"),
        ],
    },
    SymbolCategory {
        name: "Sets",
        symbols: &[
            ("\u{2208}", "\\in"),
            ("\u{2209}", "\\notin"),
            ("\u{2286}", "\\subseteq"),
            ("\u{2287}", "\\supseteq"),
            ("\u{2282}", "\\subset"),
            ("\u{2283}", "\\supset"),
            ("\u{222A}", "\\cup"),
            ("\u{2229}", "\\cap"),
            ("\u{2205}", "\\emptyset"),
            ("\u{2119}", "\\mathbb{P}"),
            ("\u{2115}", "\\mathbb{N}"),
            ("\u{2124}", "\\mathbb{Z}"),
            ("\u{211A}", "\\mathbb{Q}"),
            ("\u{211D}", "\\mathbb{R}"),
            ("\u{2102}", "\\mathbb{C}"),
        ],
    },
    SymbolCategory {
        name: "Relations",
        symbols: &[
            ("\u{2260}", "\\neq"),
            ("\u{2264}", "\\leq"),
            ("\u{2265}", "\\geq"),
            ("\u{226A}", "\\ll"),
            ("\u{226B}", "\\gg"),
            ("\u{2248}", "\\approx"),
            ("\u{2261}", "\\equiv"),
            ("\u{223C}", "\\sim"),
            ("\u{2245}", "\\cong"),
            ("\u{221D}", "\\propto"),
        ],
    },
    SymbolCategory {
        name: "Operators",
        symbols: &[
            ("\u{00D7}", "\\times"),
            ("\u{00F7}", "\\div"),
            ("\u{00B1}", "\\pm"),
            ("\u{2213}", "\\mp"),
            ("\u{22C5}", "\\cdot"),
            ("\u{2218}", "\\circ"),
            ("\u{2295}", "\\oplus"),
            ("\u{2297}", "\\otimes"),
            ("\u{221A}", "\\sqrt{}"),
            ("\u{221E}", "\\infty"),
            ("\u{2202}", "\\partial"),
            ("\u{2207}", "\\nabla"),
            ("\u{222B}", "\\int"),
            ("\u{220F}", "\\prod"),
            ("\u{2211}", "\\sum"),
        ],
    },
];

#[component]
pub fn SymbolPalette(
    on_insert: EventHandler<String>,
) -> Element {
    let mut search = use_signal(String::new);

    let search_lower = search.read().to_lowercase();
    let filtered_categories: Vec<_> = if search_lower.is_empty() {
        SYMBOL_CATEGORIES.iter().collect()
    } else {
        SYMBOL_CATEGORIES
            .iter()
            .filter(|cat| {
                cat.name.to_lowercase().contains(&search_lower)
                    || cat.symbols.iter().any(|(_, latex)| latex.contains(&search_lower))
            })
            .collect()
    };

    rsx! {
        style { "{SYMBOL_PALETTE_STYLE}" }

        div { class: "symbol-palette",
            // Header with search
            div { class: "symbol-palette-header",
                span { class: "symbol-palette-title", "Symbols" }
                div { class: "symbol-search",
                    input {
                        r#type: "text",
                        placeholder: "Search symbols...",
                        value: "{search}",
                        oninput: move |e| search.set(e.value()),
                    }
                }
            }

            // Symbol categories
            div { class: "symbol-categories",
                for category in filtered_categories {
                    div { class: "symbol-category", key: "{category.name}",
                        div { class: "symbol-category-header", "{category.name}" }
                        div { class: "symbol-grid",
                            for (symbol, latex) in category.symbols.iter() {
                                button {
                                    class: "symbol-btn",
                                    key: "{latex}",
                                    title: "{latex}",
                                    onclick: move |_| on_insert.call(latex.to_string()),
                                    "{symbol}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
