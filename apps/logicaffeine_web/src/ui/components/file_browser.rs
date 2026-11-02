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
//! - `on_new_file` - New file, where the host keeps the reader's files; without it the header offers none
//!
//! # File Icons
//!
//! Icons are chosen by file extension:
//! - `.logic` files: ∀ (forall symbol)
//! - `.logos` files: λ (lambda symbol)
//! - `.math` files: π (pi symbol)
//! - Other files: document icon

use crate::ui::components::icon::{Icon, IconSize, IconVariant};
use crate::ui::components::tree::{Tree, TreeNode};
use crate::ui::state::FileNode;
use dioxus::prelude::*;

const FILE_BROWSER_STYLE: &str = r#"
.file-browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
    background: var(--studio-surface, #12161c);
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

.file-browser-heading {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
}

/* A fact about the files beside their title, in the theme's secondary text. */
.file-browser-note {
    font-size: 12px;
    font-weight: 400;
    color: var(--studio-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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

.file-browser-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.9);
}

/* New file waits while the reader's files load: it keeps its place, dimmed, and says why. */
.file-browser-btn:disabled {
    opacity: 0.5;
    cursor: default;
}

/* The file tree is the Studio's tree (`components::tree`): it takes the sidebar's room and scrolls. */
.file-tree {
    flex: 1;
    overflow: auto;
}

.file-tree-item .icon {
    flex: none;
    width: 16px;
    margin-right: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    opacity: 0.7;
}

.file-tree-item .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    /// New file, where the host keeps the reader's files. A host that keeps none (a frame on another
    /// page) gives none, and the header offers no New file.
    #[props(default)]
    on_new_file: Option<EventHandler<()>>,
    #[props(default = false)] show_private_mode: bool,
    /// Why New file cannot make a file yet, such as the reader's files still loading. While there
    /// is a reason, the button stays in place, disabled, and its tooltip gives the reason.
    #[props(default)]
    new_file_waits: Option<String>,
) -> Element {
    rsx! {
        style { "{FILE_BROWSER_STYLE}" }

        aside { class: "file-browser",
            // Header
            div { class: "file-browser-header",
                div { class: "file-browser-heading",
                    span { class: "file-browser-title panel-title", "Files" }
                    // Without OPFS the files live in IndexedDB for this session only.
                    if show_private_mode {
                        span {
                            class: "file-browser-note",
                            title: "Temporary storage: files last for this browser session",
                            "Private mode"
                        }
                    }
                }
                div { class: "file-browser-actions",
                    if let Some(new_file) = on_new_file {
                        button {
                            class: "file-browser-btn",
                            disabled: new_file_waits.is_some(),
                            onclick: move |_| new_file.call(()),
                            title: new_file_waits.clone().unwrap_or_else(|| "New file".to_string()),
                            "+"
                        }
                    }
                }
            }

            // File tree
            Tree { kind: "file-tree", label: "Files",
                if tree.children.is_empty() {
                    div { class: "file-tree-empty",
                        "No files yet"
                    }
                } else {
                    for (i, child) in tree.children.iter().enumerate() {
                        FileTreeNode {
                            key: "{child.path}",
                            node: child.clone(),
                            selected_path: selected_path.clone(),
                            depth: 0,
                            last: i + 1 == tree.children.len(),
                            stop: i == 0 && !selected_path.as_deref().is_some_and(|path| draws(&tree, path)),
                            on_select: on_select.clone(),
                            on_toggle_dir: on_toggle_dir.clone(),
                        }
                    }
                }
            }
        }
    }
}

/// Whether the tree draws the row of `path`: a node of it whose folders are all open.
fn draws(node: &FileNode, path: &str) -> bool {
    node.children.iter().any(|child| child.path == path || (child.is_directory && child.expanded && draws(child, path)))
}

#[component]
fn FileTreeNode(
    node: FileNode,
    selected_path: Option<String>,
    depth: usize,
    /// The last of its siblings: its tree line stops at its own branch.
    last: bool,
    /// The tree's tab stop, since the tree draws no selected row.
    #[props(default)]
    stop: bool,
    on_select: EventHandler<String>,
    on_toggle_dir: EventHandler<String>,
) -> Element {
    let icon_variant = if node.is_directory {
        if node.expanded {
            IconVariant::FolderOpen
        } else {
            IconVariant::Folder
        }
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

    let path = node.path.clone();
    let is_directory = node.is_directory;
    let count = node.children.len();

    rsx! {
        TreeNode {
            depth,
            last,
            expandable: is_directory,
            open: node.expanded,
            selected: selected_path.as_ref() == Some(&node.path),
            stop,
            title: node.path.clone(),
            on_press: Some(EventHandler::new(move |_| {
                if is_directory {
                    on_toggle_dir.call(path.clone());
                } else {
                    on_select.call(path.clone());
                }
            })),
            row: rsx! {
                span { class: "icon",
                    if let Some(sym) = file_symbol {
                        span { "{sym}" }
                    } else {
                        Icon { variant: icon_variant, size: IconSize::Small }
                    }
                }
                span { class: "name", "{node.name}" }
            },
            for (i, child) in node.children.iter().enumerate() {
                FileTreeNode {
                    key: "{child.path}",
                    node: child.clone(),
                    selected_path: selected_path.clone(),
                    depth: depth + 1,
                    last: i + 1 == count,
                    on_select: on_select.clone(),
                    on_toggle_dir: on_toggle_dir.clone(),
                }
            }
        }
    }
}
