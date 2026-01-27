//! Subject-specific workspace page.
//!
//! A three-panel workspace for working with specific subjects (Logic, English,
//! Coding, Mathematics). Layout includes:
//!
//! - **Sidebar**: Lesson tree with progress indicators
//! - **Main Area**: Chat-based interaction with input area
//! - **Inspector**: AST visualization panel
//!
//! # Props
//!
//! - `subject` - The subject to work with (logic, english, coding, math)
//!
//! # Route
//!
//! Accessed via [`Route::Workspace`](crate::ui::router::Route::Workspace).

use dioxus::prelude::*;
use crate::ui::state::AppState;
use crate::ui::components::chat::ChatDisplay;
use crate::ui::components::input::InputArea;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::seo::PageHead;

const WORKSPACE_STYLE: &str = r#"
.workspace {
    height: 100vh;
    display: flex;
    flex-direction: column;
}

.workspace-header {
    background: rgba(0, 0, 0, 0.3);
    backdrop-filter: blur(10px);
    padding: 16px 24px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.workspace-header .breadcrumb {
    display: flex;
    align-items: center;
    gap: 8px;
}

.workspace-header .breadcrumb a {
    color: #888;
    text-decoration: none;
    font-size: 14px;
}

.workspace-header .breadcrumb a:hover {
    color: #00d4ff;
}

.workspace-header .breadcrumb span {
    color: #666;
}

.workspace-header h1 {
    font-size: 20px;
    font-weight: 600;
    background: linear-gradient(90deg, #00d4ff, #7b2cbf);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}

.workspace-content {
    flex: 1;
    display: flex;
    overflow: hidden;
}

.sidebar {
    width: 260px;
    background: rgba(0, 0, 0, 0.2);
    border-right: 1px solid rgba(255, 255, 255, 0.1);
    padding: 20px;
    overflow-y: auto;
}

.sidebar h3 {
    color: #888;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 16px;
}

.lesson-tree {
    list-style: none;
}

.lesson-tree li {
    padding: 10px 12px;
    margin-bottom: 4px;
    border-radius: 8px;
    color: #aaa;
    font-size: 14px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.lesson-tree li:hover {
    background: rgba(255, 255, 255, 0.05);
    color: #fff;
}

.lesson-tree li.active {
    background: rgba(102, 126, 234, 0.2);
    color: #667eea;
}

.lesson-tree li.locked {
    opacity: 0.4;
    cursor: not-allowed;
}

.main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
}

.inspector {
    width: 300px;
    background: rgba(0, 0, 0, 0.2);
    border-left: 1px solid rgba(255, 255, 255, 0.1);
    padding: 20px;
    overflow-y: auto;
}

.inspector h3 {
    color: #888;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 16px;
}

.inspector-placeholder {
    color: #666;
    font-size: 14px;
    font-style: italic;
    text-align: center;
    padding: 40px 20px;
}

@media (max-width: 900px) {
    .sidebar, .inspector {
        display: none;
    }
}
"#;

#[component]
pub fn Workspace(subject: String) -> Element {
    let mut state = use_context_provider(|| Signal::new(AppState::new()));

    let title = match subject.as_str() {
        "logic" => "First-Order Logic",
        "english" => "English",
        "coding" => "Coding",
        "math" => "Mathematics",
        _ => "Workspace",
    };
    let page_title = format!("{} Workspace - LOGICAFFEINE", title);
    let page_path = format!("/workspace/{}", subject);

    rsx! {
        PageHead {
            title: page_title,
            description: "Interactive workspace for practicing formal logic and reasoning.",
            canonical_path: page_path,
        }
        style { "{WORKSPACE_STYLE}" }

        MainNav { active: ActivePage::Studio, subtitle: Some(title) }

        div { class: "workspace",
            div { class: "workspace-content",
                div { class: "sidebar",
                    h3 { "The Path" }
                    ul { class: "lesson-tree",
                        li { class: "active", "1. Basic Propositions" }
                        li { "2. Connectives" }
                        li { "3. Quantifiers" }
                        li { "4. Predicates" }
                        li { class: "locked", "5. Modal Logic" }
                        li { class: "locked", "6. Temporal Logic" }
                    }

                    h3 { style: "margin-top: 32px;", "History" }
                    p { style: "color: #666; font-size: 13px;", "Your recent sessions will appear here." }
                }

                div { class: "main-area",
                    ChatDisplay { messages: state.read().get_history() }
                    InputArea { on_send: move |text| state.write().add_user_message(text) }
                }

                div { class: "inspector",
                    h3 { "AST Inspector" }
                    div { class: "inspector-placeholder",
                        "Parse a sentence to see its abstract syntax tree visualization here."
                    }
                }
            }
        }
    }
}
