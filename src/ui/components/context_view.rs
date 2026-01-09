//! Context view panel for Code mode - displays definitions and types.

use dioxus::prelude::*;

const CONTEXT_VIEW_STYLE: &str = r#"
.context-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #12161c;
    font-family: 'SF Mono', 'Fira Code', monospace;
}

.context-view-header {
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(255, 255, 255, 0.02);
}

.context-view-title {
    font-size: 14px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.9);
}

.context-sections {
    flex: 1;
    overflow: auto;
    padding: 12px;
}

.context-section {
    margin-bottom: 20px;
}

.context-section-header {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(255, 255, 255, 0.4);
    margin-bottom: 8px;
    padding-bottom: 4px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.context-item {
    padding: 8px 10px;
    margin-bottom: 4px;
    background: rgba(255, 255, 255, 0.02);
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.5;
}

.context-item-name {
    color: #61afef;
    font-weight: 500;
}

.context-item-type {
    color: rgba(255, 255, 255, 0.6);
    margin-left: 8px;
}

.context-item-type::before {
    content: ": ";
    color: rgba(255, 255, 255, 0.3);
}

.context-item-body {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px dashed rgba(255, 255, 255, 0.06);
    color: rgba(255, 255, 255, 0.5);
    font-size: 11px;
}

.context-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: rgba(255, 255, 255, 0.4);
    text-align: center;
    padding: 20px;
    font-size: 13px;
}

/* Type keywords highlighting */
.type-keyword {
    color: #c678dd;
}

.type-name {
    color: #e5c07b;
}

.type-arrow {
    color: rgba(255, 255, 255, 0.4);
}
"#;

/// A definition entry in the context.
#[derive(Clone, PartialEq)]
pub struct ContextEntry {
    pub name: String,
    pub ty: String,
    pub body: Option<String>,
    pub kind: EntryKind,
}

#[derive(Clone, Copy, PartialEq)]
pub enum EntryKind {
    Definition,
    Inductive,
    Constructor,
}

#[component]
pub fn ContextView(
    definitions: Vec<ContextEntry>,
    inductives: Vec<ContextEntry>,
) -> Element {
    rsx! {
        style { "{CONTEXT_VIEW_STYLE}" }

        div { class: "context-view",
            // Header
            div { class: "context-view-header",
                span { class: "context-view-title", "Context" }
            }

            // Sections
            div { class: "context-sections",
                if definitions.is_empty() && inductives.is_empty() {
                    div { class: "context-empty",
                        "No definitions in scope"
                    }
                } else {
                    // Inductive types section
                    if !inductives.is_empty() {
                        div { class: "context-section",
                            div { class: "context-section-header", "Inductive Types" }
                            for entry in inductives.iter() {
                                ContextItem {
                                    key: "{entry.name}",
                                    entry: entry.clone(),
                                }
                            }
                        }
                    }

                    // Definitions section
                    if !definitions.is_empty() {
                        div { class: "context-section",
                            div { class: "context-section-header", "Definitions" }
                            for entry in definitions.iter() {
                                ContextItem {
                                    key: "{entry.name}",
                                    entry: entry.clone(),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ContextItem(entry: ContextEntry) -> Element {
    rsx! {
        div { class: "context-item",
            div {
                span { class: "context-item-name", "{entry.name}" }
                span { class: "context-item-type", "{entry.ty}" }
            }
            if let Some(body) = &entry.body {
                div { class: "context-item-body", ":= {body}" }
            }
        }
    }
}
