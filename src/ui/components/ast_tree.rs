use dioxus::prelude::*;
use crate::AstNode;

const TREE_STYLE: &str = r#"
.ast-tree-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: auto;
    padding: 16px;
    -webkit-overflow-scrolling: touch;
}

.ast-tree-empty {
    color: #666;
    font-style: italic;
    text-align: center;
    padding: 40px 20px;
}

.ast-node {
    margin-left: 16px;
    position: relative;
}

.ast-node:before {
    content: '';
    position: absolute;
    left: -12px;
    top: 0;
    height: 100%;
    width: 1px;
    background: rgba(255, 255, 255, 0.1);
}

.ast-node:last-child:before {
    height: 12px;
}

.ast-node-label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: 4px;
    cursor: pointer;
    transition: background 0.15s ease;
    position: relative;
    -webkit-tap-highlight-color: transparent;
}

.ast-node-label:hover {
    background: rgba(255, 255, 255, 0.05);
}

.ast-node-label:before {
    content: '';
    position: absolute;
    left: -12px;
    top: 50%;
    width: 8px;
    height: 1px;
    background: rgba(255, 255, 255, 0.1);
}

.ast-node-toggle {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    color: #666;
    transition: transform 0.15s ease;
}

.ast-node-toggle.expanded {
    transform: rotate(90deg);
}

.ast-node-text {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
}

.ast-node-type {
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.08);
    color: #888;
    white-space: nowrap;
}

.ast-node-type.quantifier { background: rgba(198, 120, 221, 0.2); color: #c678dd; }
.ast-node-type.predicate { background: rgba(152, 195, 121, 0.2); color: #98c379; }
.ast-node-type.binary_op { background: rgba(198, 120, 221, 0.2); color: #c678dd; }
.ast-node-type.unary_op { background: rgba(224, 108, 117, 0.2); color: #e06c75; }
.ast-node-type.constant { background: rgba(229, 192, 123, 0.2); color: #e5c07b; }
.ast-node-type.variable { background: rgba(97, 175, 239, 0.2); color: #61afef; }
.ast-node-type.modal { background: rgba(86, 182, 194, 0.2); color: #56b6c2; }
.ast-node-type.lambda { background: rgba(224, 108, 117, 0.2); color: #e06c75; }

.ast-children {
    display: none;
}

.ast-children.expanded {
    display: block;
}

.ast-root {
    margin-left: 0;
}

.ast-root:before {
    display: none;
}

.ast-root > .ast-node-label:before {
    display: none;
}

/* Mobile optimizations */
@media (max-width: 768px) {
    .ast-tree-container {
        padding: 12px;
    }

    .ast-tree-empty {
        padding: 30px 16px;
        font-size: 14px;
    }

    /* Larger touch targets for tree nodes */
    .ast-node {
        margin-left: 14px;
    }

    .ast-node-label {
        padding: 8px 10px;
        gap: 8px;
        min-height: 40px;
        border-radius: 6px;
    }

    .ast-node-label:active {
        background: rgba(255, 255, 255, 0.1);
    }

    .ast-node-toggle {
        width: 24px;
        height: 24px;
        font-size: 12px;
    }

    .ast-node-text {
        font-size: 14px;
        word-break: break-word;
    }

    .ast-node-type {
        font-size: 11px;
        padding: 3px 8px;
        border-radius: 4px;
    }

    .ast-node:before {
        left: -10px;
    }

    .ast-node-label:before {
        left: -10px;
        width: 6px;
    }
}

/* Extra small screens */
@media (max-width: 480px) {
    .ast-tree-container {
        padding: 10px;
    }

    .ast-node {
        margin-left: 12px;
    }

    .ast-node-label {
        padding: 6px 8px;
        min-height: 36px;
    }

    .ast-node-text {
        font-size: 13px;
    }

    .ast-node-type {
        font-size: 10px;
        padding: 2px 6px;
    }
}
"#;

#[component]
pub fn AstTree(ast: Option<AstNode>) -> Element {
    rsx! {
        style { "{TREE_STYLE}" }

        div { class: "ast-tree-container",
            if let Some(node) = ast {
                AstNodeView { node: node, is_root: true }
            } else {
                div { class: "ast-tree-empty",
                    "Parse a sentence to see its AST..."
                }
            }
        }
    }
}

#[component]
fn AstNodeView(node: AstNode, is_root: bool) -> Element {
    let mut expanded = use_signal(|| true);
    let has_children = !node.children.is_empty();

    let node_class = if is_root { "ast-node ast-root" } else { "ast-node" };
    let toggle_class = if *expanded.read() { "ast-node-toggle expanded" } else { "ast-node-toggle" };
    let children_class = if *expanded.read() { "ast-children expanded" } else { "ast-children" };
    let type_class = format!("ast-node-type {}", node.node_type);

    rsx! {
        div { class: "{node_class}",
            div {
                class: "ast-node-label",
                onclick: move |_| {
                    if has_children {
                        let current = *expanded.read();
                        expanded.set(!current);
                    }
                },
                if has_children {
                    span { class: "{toggle_class}", "\u{25B6}" }
                } else {
                    span { class: "ast-node-toggle", "\u{2022}" }
                }
                span { class: "ast-node-text", "{node.label}" }
                span { class: "{type_class}", "{node.node_type}" }
            }

            if has_children {
                div { class: "{children_class}",
                    for child in node.children.iter() {
                        AstNodeView { node: child.clone(), is_root: false }
                    }
                }
            }
        }
    }
}
