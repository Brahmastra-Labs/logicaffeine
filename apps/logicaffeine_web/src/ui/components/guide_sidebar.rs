//! Sidebar navigation component for the Programmer's Guide.
//!
//! Features:
//! - Sticky positioning
//! - Sections grouped by Part
//! - Active section highlighting
//! - Click navigation with anchor links

use dioxus::prelude::*;

const SIDEBAR_STYLE: &str = r#"
.guide-sidebar {
    position: sticky;
    top: 90px;
    width: 260px;
    max-height: calc(100vh - 120px);
    overflow-y: auto;
    flex-shrink: 0;
    padding: 4px 0;

    /* Custom scrollbar */
    scrollbar-width: thin;
    scrollbar-color: rgba(255,255,255,0.1) transparent;
}

.guide-sidebar::-webkit-scrollbar {
    width: 6px;
}

.guide-sidebar::-webkit-scrollbar-track {
    background: transparent;
}

.guide-sidebar::-webkit-scrollbar-thumb {
    background: rgba(255,255,255,0.1);
    border-radius: 3px;
}

.guide-sidebar::-webkit-scrollbar-thumb:hover {
    background: rgba(255,255,255,0.2);
}

.sidebar-part {
    margin-bottom: 24px;
}

.sidebar-part:last-child {
    margin-bottom: 0;
}

.sidebar-part-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.8px;
    color: rgba(229,231,235,0.45);
    padding: 0 16px;
    margin-bottom: 10px;
}

.sidebar-section {
    display: block;
    padding: 9px 16px;
    margin: 2px 8px;
    border-radius: 0 4px 4px 0;
    color: rgba(229,231,235,0.65);
    font-size: 13px;
    font-weight: 500;
    text-decoration: none;
    transition: all 0.18s ease;
    cursor: pointer;
    border-left: 2px solid transparent;
}

.sidebar-section:hover {
    background: rgba(255,255,255,0.05);
    color: rgba(229,231,235,0.9);
}

.sidebar-section.active {
    background: rgba(167,139,250,0.12);
    color: #a78bfa;
    border-left-color: #a78bfa;
    font-weight: 600;
}

.sidebar-section-number {
    display: inline-block;
    min-width: 22px;
    color: rgba(229,231,235,0.35);
    font-weight: 500;
}

.sidebar-section.active .sidebar-section-number {
    color: rgba(167,139,250,0.7);
}

@media (max-width: 1024px) {
    .guide-sidebar {
        display: none;
    }
}

/* Mobile sidebar toggle - shown on mobile */
.sidebar-mobile-toggle {
    display: none;
    position: fixed;
    bottom: 24px;
    right: 24px;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: linear-gradient(135deg, #60a5fa, #a78bfa);
    border: none;
    color: #060814;
    font-size: 24px;
    cursor: pointer;
    box-shadow: 0 8px 32px rgba(0,0,0,0.4);
    z-index: 100;
    transition: transform 0.2s ease;
}

.sidebar-mobile-toggle:hover {
    transform: scale(1.05);
}

@media (max-width: 1024px) {
    .sidebar-mobile-toggle {
        display: flex;
        align-items: center;
        justify-content: center;
    }
}
"#;

/// Information about a section for the sidebar
#[derive(Clone, PartialEq, Debug)]
pub struct SectionInfo {
    pub id: String,
    pub number: u8,
    pub title: String,
    pub part: String,
    /// Graded exercises in the section (0 = no mastery ring).
    pub exercise_total: usize,
    /// How many the reader has passed at least once (SM-2 deck entries).
    pub exercise_passed: usize,
}

#[derive(Props, Clone, PartialEq)]
pub struct GuideSidebarProps {
    pub sections: Vec<SectionInfo>,
    pub active_section: String,
    pub on_section_click: EventHandler<String>,
    /// Section ids the reader has marked done (persisted per browser).
    pub completed: Vec<String>,
    pub on_toggle_complete: EventHandler<String>,
}

#[component]
pub fn GuideSidebar(props: GuideSidebarProps) -> Element {
    // Group sections by part
    let grouped = group_sections_by_part(&props.sections);

    rsx! {
        style { "{SIDEBAR_STYLE}" }

        nav { class: "guide-sidebar",
            for (part, sections) in grouped {
                div { class: "sidebar-part",
                    h4 { class: "sidebar-part-title", "{part}" }

                    for section in sections {
                        {
                            let section_id = section.id.clone();
                            let is_active = props.active_section == section.id;
                            let is_done = props.completed.iter().any(|c| *c == section.id);
                            let check_id = section.id.clone();
                            let on_toggle = props.on_toggle_complete.clone();
                            let class_name = if is_active {
                                "sidebar-section active"
                            } else {
                                "sidebar-section"
                            };

                            rsx! {
                                div {
                                    style: "display: flex; align-items: center; gap: 2px;",
                                span {
                                    class: if is_done { "guide-section-check done" } else { "guide-section-check" },
                                    style: if is_done { "cursor: pointer; font-size: 11px; color: #22c55e; width: 14px; flex: none;" } else { "cursor: pointer; font-size: 11px; opacity: 0.35; width: 14px; flex: none;" },
                                    title: "Mark section as read",
                                    onclick: move |_| on_toggle.call(check_id.clone()),
                                    if is_done { "✓" } else { "○" }
                                }
                                // Mastery is doing, not scrolling: the ring counts
                                // exercises PASSED, separate from the read-mark.
                                if section.exercise_total > 0 {
                                    span {
                                        class: if section.exercise_passed == section.exercise_total { "guide-mastery-ring done" } else { "guide-mastery-ring" },
                                        style: if section.exercise_passed == section.exercise_total { "flex: none; font-size: 9px; font-weight: 700; color: #22c55e; border: 1px solid rgba(34,197,94,0.5); border-radius: 999px; padding: 1px 5px;" } else { "flex: none; font-size: 9px; font-weight: 700; color: rgba(229,231,235,0.5); border: 1px solid rgba(255,255,255,0.2); border-radius: 999px; padding: 1px 5px;" },
                                        title: "{section.exercise_passed} of {section.exercise_total} exercises passed",
                                        "{section.exercise_passed}/{section.exercise_total}"
                                    }
                                }
                                a {
                                    class: "{class_name}",
                                    style: "flex: 1;",
                                    href: "#{section_id}",
                                    onclick: {
                                        let id = section.id.clone();
                                        let handler = props.on_section_click.clone();
                                        move |evt: Event<MouseData>| {
                                            evt.prevent_default();
                                            handler.call(id.clone());

                                            // Smooth scroll to section
                                            #[cfg(target_arch = "wasm32")]
                                            {
                                                if let Some(window) = web_sys::window() {
                                                    if let Some(document) = window.document() {
                                                        if let Some(element) = document.get_element_by_id(&id) {
                                                            let options = web_sys::ScrollIntoViewOptions::new();
                                                            options.set_behavior(web_sys::ScrollBehavior::Smooth);
                                                            options.set_block(web_sys::ScrollLogicalPosition::Start);
                                                            let _ = element.scroll_into_view_with_scroll_into_view_options(&options);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    span { class: "sidebar-section-number", "{section.number}." }
                                    " {section.title}"
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

/// Groups sections by their part name, preserving order
fn group_sections_by_part(sections: &[SectionInfo]) -> Vec<(String, Vec<SectionInfo>)> {
    let mut result: Vec<(String, Vec<SectionInfo>)> = Vec::new();

    for section in sections {
        if let Some((_, group)) = result.iter_mut().find(|(part, _)| part == &section.part) {
            group.push(section.clone());
        } else {
            result.push((section.part.clone(), vec![section.clone()]));
        }
    }

    result
}

