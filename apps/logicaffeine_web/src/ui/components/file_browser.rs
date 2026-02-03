//! File browser sidebar with tree navigation.
//!
//! Displays a hierarchical file tree for the Studio's virtual file system.
//! Supports collapsible directories, file selection, and responsive layout.
//!
//! # Props
//!
//! - `tree` - Root [`FileNode`] containing the file tree
//! - `selected_path` - Currently selected file path
//! - `on_select` - Callback when a file is selected
//! - `on_toggle_dir` - Callback when a directory is expanded/collapsed
//! - `on_new_file` - Callback for new file button
//!
//! # File Icons
//!
//! Icons are chosen by file extension:
//! - `.logic` files: ∀ (forall symbol)
//! - `.logos` files: λ (lambda symbol)
//! - `.math` files: π (pi symbol)
//! - Other files: document icon

use dioxus::prelude::*;
use crate::ui::state::FileNode;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};

const FILE_BROWSER_STYLE: &str = r#"
.file-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    background: #12161c;
    overflow: hidden;
}

.file-browser-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 14px;
    height: 52px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(255, 255, 255, 0.02);
    flex-shrink: 0;
}

.file-browser-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(255, 255, 255, 0.6);
}

.file-browser-actions {
    display: flex;
    gap: 4px;
}

.file-browser-btn {
    padding: 4px 8px;
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.5);
    font-size: 14px;
    cursor: pointer;
    border-radius: 4px;
    transition: all 0.15s ease;
}

.file-browser-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.9);
}

.file-browser-btn.coming-soon {
    position: relative;
}

.coming-soon-toast {
    position: fixed;
    top: 80px;
    left: 50%;
    transform: translateX(-50%);
    background: linear-gradient(135deg, #00d4ff 0%, #818cf8 100%);
    color: white;
    padding: 12px 20px;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    box-shadow: 0 4px 20px rgba(0, 212, 255, 0.3);
    z-index: 1000;
    animation: toast-fade 2.5s ease-out forwards;
}

@keyframes toast-fade {
    0% { opacity: 0; transform: translateX(-50%) translateY(-10px); }
    10% { opacity: 1; transform: translateX(-50%) translateY(0); }
    80% { opacity: 1; }
    100% { opacity: 0; }
}

.file-tree {
    flex: 1;
    overflow: auto;
    padding: 8px 0;
}

.file-tree-node {
    -webkit-user-select: none;
    user-select: none;
}

.file-tree-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    cursor: pointer;
    color: rgba(255, 255, 255, 0.8);
    font-size: 13px;
    transition: background 0.1s ease;
}

.file-tree-item:hover {
    background: rgba(255, 255, 255, 0.04);
}

.file-tree-item.selected {
    background: rgba(0, 212, 255, 0.12);
    color: #00d4ff;
}

.file-tree-item .icon {
    font-size: 14px;
    opacity: 0.7;
    flex-shrink: 0;
}

.file-tree-item .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.file-tree-item .chevron {
    font-size: 10px;
    opacity: 0.5;
    transition: transform 0.15s ease;
}

.file-tree-item .chevron.expanded {
    transform: rotate(90deg);
}

.file-tree-children {
    padding-left: 16px;
}

.file-tree-empty {
    padding: 20px;
    text-align: center;
    color: rgba(255, 255, 255, 0.4);
    font-size: 13px;
}

/* Mobile: fill the wrapper, which handles positioning */
@media (max-width: 768px) {
    .file-browser {
        width: 100%;
        min-width: 100%;
        max-width: 100%;
    }

    .file-tree-item {
        color: rgba(255, 255, 255, 0.9);
        -webkit-text-fill-color: rgba(255, 255, 255, 0.9);
    }

    .file-tree-item.selected {
        color: #00d4ff;
        -webkit-text-fill-color: #00d4ff;
    }

    .file-tree-item .name {
        color: inherit;
        -webkit-text-fill-color: inherit;
    }
}
"#;

#[component]
pub fn FileBrowser(
    tree: FileNode,
    selected_path: Option<String>,
    on_select: EventHandler<String>,
    on_toggle_dir: EventHandler<String>,
    on_new_file: EventHandler<()>,
) -> Element {
    let mut show_toast = use_signal(|| false);
    let mut toast_key = use_signal(|| 0u32);

    // Debug: log what we received
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&format!(
        "[FileBrowser] Rendering with {} children, selected: {:?}",
        tree.children.len(),
        selected_path
    ).into());

    rsx! {
        style { "{FILE_BROWSER_STYLE}" }

        // Coming Soon toast notification
        if *show_toast.read() {
            div {
                key: "{toast_key}",
                class: "coming-soon-toast",
                "Coming Soon!"
            }
        }

        aside { class: "file-browser",
            // Header
            div { class: "file-browser-header",
                span { class: "file-browser-title", "Files" }
                div { class: "file-browser-actions",
                    button {
                        class: "file-browser-btn coming-soon",
                        onclick: move |_| {
                            // Increment key to restart animation if clicked again
                            toast_key.set(toast_key() + 1);
                            show_toast.set(true);
                        },
                        title: "New file (Coming Soon)",
                        "+"
                    }
                }
            }

            // File tree
            div { class: "file-tree",
                if tree.children.is_empty() {
                    div { class: "file-tree-empty",
                        "No files yet"
                    }
                } else {
                    for child in tree.children.iter() {
                        FileTreeNode {
                            key: "{child.path}",
                            node: child.clone(),
                            selected_path: selected_path.clone(),
                            depth: 0,
                            on_select: on_select.clone(),
                            on_toggle_dir: on_toggle_dir.clone(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FileTreeNode(
    node: FileNode,
    selected_path: Option<String>,
    depth: usize,
    on_select: EventHandler<String>,
    on_toggle_dir: EventHandler<String>,
) -> Element {
    let is_selected = selected_path.as_ref() == Some(&node.path);
    let item_class = if is_selected {
        "file-tree-item selected"
    } else {
        "file-tree-item"
    };

    let icon_variant = if node.is_directory {
        if node.expanded { IconVariant::FolderOpen } else { IconVariant::Folder }
    } else {
        IconVariant::File
    };

    // File-specific display text for logic/math files
    let file_symbol: Option<&'static str> = if !node.is_directory {
        match node.name.rsplit('.').next() {
            Some("logic") => Some("∀"),
            Some("logos") => Some("λ"),
            Some("math") => Some("π"),
            _ => None,
        }
    } else {
        None
    };

    let chevron_class = if node.expanded {
        "chevron expanded"
    } else {
        "chevron"
    };

    let path = node.path.clone();
    let path_for_toggle = node.path.clone();

    rsx! {
        div { class: "file-tree-node",
            div {
                class: "{item_class}",
                style: "padding-left: {14 + depth * 16}px;",
                onclick: move |_| {
                    if node.is_directory {
                        on_toggle_dir.call(path_for_toggle.clone());
                    } else {
                        on_select.call(path.clone());
                    }
                },

                // Chevron for directories
                if node.is_directory {
                    span { class: "{chevron_class}", "\u{276F}" }
                }

                span { class: "icon",
                    if let Some(sym) = file_symbol {
                        span { "{sym}" }
                    } else {
                        Icon { variant: icon_variant, size: IconSize::Small }
                    }
                }
                span { class: "name", "{node.name}" }
            }

            // Children (if directory is expanded)
            if node.is_directory && node.expanded {
                div { class: "file-tree-children",
                    for child in node.children.iter() {
                        FileTreeNode {
                            key: "{child.path}",
                            node: child.clone(),
                            selected_path: selected_path.clone(),
                            depth: depth + 1,
                            on_select: on_select.clone(),
                            on_toggle_dir: on_toggle_dir.clone(),
                        }
                    }
                }
            }
        }
    }
}
