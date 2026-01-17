//! File browser sidebar with tree navigation.
//!
//! Displays a hierarchical file tree for the Studio's virtual file system.
//! Supports collapsible directories, file selection, and responsive layout.
//!
//! # Props
//!
//! - `tree` - Root [`FileNode`] containing the file tree
//! - `selected_path` - Currently selected file path
//! - `collapsed` - Whether the sidebar is collapsed to icons-only
//! - `on_select` - Callback when a file is selected
//! - `on_toggle_dir` - Callback when a directory is expanded/collapsed
//! - `on_new_file` - Callback for new file button
//! - `on_collapse_toggle` - Callback for sidebar collapse toggle
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

const FILE_BROWSER_STYLE: &str = r#"
.file-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #12161c;
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    min-width: 200px;
    max-width: 280px;
    overflow: hidden;
}

.file-browser-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(255, 255, 255, 0.02);
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
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: white;
    padding: 12px 20px;
    border-radius: 8px;
    font-size: 14px;
    font-weight: 500;
    box-shadow: 0 4px 20px rgba(102, 126, 234, 0.4);
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
    background: rgba(102, 126, 234, 0.15);
    color: #667eea;
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

/* Collapsed sidebar state */
.file-browser.collapsed {
    min-width: 48px;
    max-width: 48px;
}

.file-browser.collapsed .file-browser-header {
    justify-content: center;
}

.file-browser.collapsed .file-browser-title,
.file-browser.collapsed .file-tree {
    display: none;
}

/* Mobile overlay */
@media (max-width: 768px) {
    .file-browser {
        position: absolute;
        left: 0;
        top: 0;
        bottom: 0;
        z-index: 100;
        transform: translateX(-100%);
        transition: transform 0.2s ease;
        max-width: 280px;
        min-width: 280px;
    }

    .file-browser.open {
        transform: translateX(0);
        box-shadow: 4px 0 20px rgba(0, 0, 0, 0.3);
    }
}
"#;

#[component]
pub fn FileBrowser(
    tree: FileNode,
    selected_path: Option<String>,
    collapsed: bool,
    on_select: EventHandler<String>,
    on_toggle_dir: EventHandler<String>,
    on_new_file: EventHandler<()>,
    on_collapse_toggle: EventHandler<()>,
) -> Element {
    let mut show_toast = use_signal(|| false);
    let mut toast_key = use_signal(|| 0u32);

    let browser_class = if collapsed {
        "file-browser collapsed"
    } else {
        "file-browser"
    };

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

        aside { class: "{browser_class}",
            // Header
            div { class: "file-browser-header",
                if !collapsed {
                    span { class: "file-browser-title", "Files" }
                }
                div { class: "file-browser-actions",
                    if !collapsed {
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
                    button {
                        class: "file-browser-btn",
                        onclick: move |_| on_collapse_toggle.call(()),
                        title: if collapsed { "Expand sidebar" } else { "Collapse sidebar" },
                        if collapsed { "\u{276F}" } else { "\u{276E}" }
                    }
                }
            }

            // File tree
            if !collapsed {
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

    let icon = if node.is_directory {
        if node.expanded { "\u{1F4C2}" } else { "\u{1F4C1}" } // 📂 open / 📁 closed
    } else {
        // File icons based on extension
        match node.name.rsplit('.').next() {
            Some("logic") => "\u{2200}", // ∀
            Some("logos") => "\u{03BB}", // λ
            Some("math") => "\u{03C0}",  // π
            _ => "\u{1F4C4}",            // 📄
        }
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

                span { class: "icon", "{icon}" }
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
