//! Gate component for content that draws exercise words from the lexicon.
//!
//! On wasm the lexicon arrives via `/data/lexicon.json` (see
//! [`crate::generator::ensure_lexicon`]); this gate holds its children back until
//! the index is pinned so `Generator::new()` is always safe inside. Native builds
//! resolve instantly, so tests and prerendering render children directly.

use dioxus::prelude::*;

const LEXICON_GATE_STYLE: &str = r#"
.lexicon-gate-status {
    color: rgba(229,231,235,0.72);
    font-size: 16px;
    text-align: center;
    padding: 48px 20px;
}

.lexicon-gate-status.error {
    color: #fca5a5;
}

.lexicon-gate-retry {
    display: block;
    margin: 0 auto;
    padding: 10px 28px;
    border-radius: 10px;
    border: 1px solid rgba(255,255,255,0.2);
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
    font-size: 15px;
    cursor: pointer;
}

.lexicon-gate-retry:hover {
    background: rgba(255,255,255,0.16);
}
"#;

#[component]
pub fn LexiconGate(children: Element) -> Element {
    let mut ready = use_resource(|| crate::generator::ensure_lexicon());
    let state = ready.read_unchecked();
    match &*state {
        Some(Ok(())) => rsx! {
            {children}
        },
        Some(Err(e)) => rsx! {
            style { "{LEXICON_GATE_STYLE}" }
            p { class: "lexicon-gate-status error", "The word bank failed to load: {e}" }
            button {
                class: "lexicon-gate-retry",
                onclick: move |_| ready.restart(),
                "Retry"
            }
        },
        None => rsx! {
            style { "{LEXICON_GATE_STYLE}" }
            p { class: "lexicon-gate-status", "Loading the word bank\u{2026}" }
        },
    }
}
