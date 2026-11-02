//! The Learn roadmap: the course as one route, era by era, a stop for every module and, inside the
//! open module, a stop for every lesson.
//!
//! Each module is a native disclosure: the module holding the open lesson starts unfolded, the rest
//! folded, and any can be unfolded by hand. Every lesson is a button whatever its state: nothing on
//! the route is locked. A finished lesson carries `data-done="true"`, the open one
//! `aria-current="step"`. Each module's element carries the module id, where `/learn#<module>` and
//! the site nav's links land.

use dioxus::prelude::*;
use crate::content::ContentEngine;

const ROADMAP_STYLE: &str = r#"
.learn-roadmap {
    --route-line: rgba(var(--accent-secondary-rgb), 0.35);
    scroll-margin-top: 112px;
}

.roadmap-era + .roadmap-era {
    margin-top: var(--spacing-xl);
}

.roadmap-era-title {
    font-size: var(--font-body-md);
    font-weight: 700;
    color: var(--text-secondary);
    margin: 0 0 var(--spacing-sm);
}

.roadmap-modules {
    list-style: none;
    margin: 0;
    padding: 0;
    position: relative;
}

/* The route: one line through every stop of an era. */
.roadmap-modules::before {
    content: "";
    position: absolute;
    left: 11px;
    top: 14px;
    bottom: 14px;
    width: 2px;
    background: var(--route-line);
    border-radius: 1px;
}

.roadmap-module {
    position: relative;
}

.roadmap-module > summary {
    list-style: none;
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: var(--spacing-md);
    padding: 8px 4px 8px 0;
    cursor: pointer;
    border-radius: var(--radius-md);
}

.roadmap-module > summary::-webkit-details-marker {
    display: none;
}

.roadmap-module > summary:focus-visible {
    outline: 2px solid var(--accent-primary);
    outline-offset: 2px;
}

/* A module's stop on the route: hollow ahead, ringed while open, filled once finished. */
.roadmap-stop {
    position: relative;
    z-index: 1;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid var(--route-line);
    background: var(--bg-gradient-start);
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-tertiary);
}

.roadmap-module[data-state="current"] .roadmap-stop {
    border-color: var(--accent-primary);
    color: var(--accent-primary);
    box-shadow: 0 0 0 4px rgba(var(--accent-primary-rgb), 0.14);
}

.roadmap-module[data-state="done"] .roadmap-stop {
    border-color: var(--accent-secondary);
    background: var(--accent-secondary);
    color: var(--bg-gradient-start);
}

.roadmap-module-title {
    margin: 0;
    font-size: var(--font-body-md);
    font-weight: 600;
    line-height: 1.35;
    color: var(--text-primary);
}

.roadmap-module-count {
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
    white-space: nowrap;
}

.roadmap-lessons {
    list-style: none;
    margin: 2px 0 var(--spacing-sm);
    padding: 0 0 0 36px;
}

.roadmap-lesson {
    position: relative;
    display: grid;
    grid-template-columns: 2.6em minmax(0, 1fr);
    align-items: baseline;
    width: 100%;
    min-height: 40px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: none;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--font-body-sm);
    line-height: 1.35;
    text-align: left;
    cursor: pointer;
}

/* A lesson's stop: a small dot on the route, filled once finished. */
.roadmap-lesson::before {
    content: "";
    position: absolute;
    left: -30px;
    top: 50%;
    width: 10px;
    height: 10px;
    margin-top: -5px;
    border-radius: 50%;
    border: 2px solid var(--route-line);
    background: var(--bg-gradient-start);
}

.roadmap-lesson[data-done="true"]::before {
    border-color: var(--accent-secondary);
    background: var(--accent-secondary);
}

.roadmap-lesson:hover {
    background: rgba(255, 255, 255, 0.04);
    color: var(--text-primary);
}

.roadmap-lesson:focus-visible {
    outline: 2px solid var(--accent-primary);
    outline-offset: 1px;
}

.roadmap-lesson[aria-current="step"] {
    background: rgba(var(--accent-primary-rgb), 0.1);
    border-color: rgba(var(--accent-primary-rgb), 0.35);
    color: var(--text-primary);
    font-weight: 600;
}

.roadmap-lesson[aria-current="step"]::before {
    border-color: var(--accent-primary);
    background: var(--accent-primary);
}

.roadmap-lesson-number {
    font-variant-numeric: tabular-nums;
    color: var(--text-tertiary);
}

.roadmap-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
}
"#;

/// The route through every era, module and lesson. `open` is the lesson on screen, `done` the
/// lessons finished; a click on a lesson calls `on_open` with its id.
#[component]
pub fn LearnRoadmap(open: Option<String>, done: Vec<String>, on_open: EventHandler<String>) -> Element {
    let is_done = |id: &str| done.iter().any(|d| d == id);
    let mut number = 0usize;
    let eras: Vec<_> = ContentEngine::shared()
        .eras()
        .iter()
        .map(|era| {
            let modules: Vec<_> = era.modules.iter().map(|m| {
                number += 1;
                (number, m)
            }).collect();
            (era, modules)
        })
        .collect();
    rsx! {
        style { "{ROADMAP_STYLE}" }
        nav { class: "learn-roadmap", id: "roadmap", aria_label: "Roadmap",
            for (era, modules) in eras {
                section { class: "roadmap-era", key: "{era.meta.id}", aria_labelledby: "roadmap-era-{era.meta.id}",
                    h2 { class: "roadmap-era-title", id: "roadmap-era-{era.meta.id}", "{era.meta.title}" }
                    ol { class: "roadmap-modules",
                        for (n, module) in modules {
                            {
                                let lessons = &module.sections;
                                let finished = lessons.iter().filter(|s| is_done(&s.id)).count();
                                let holds_open = lessons.iter().any(|s| open.as_deref() == Some(s.id.as_str()));
                                let state = if holds_open {
                                    "current"
                                } else if finished == lessons.len() && !lessons.is_empty() {
                                    "done"
                                } else {
                                    "ahead"
                                };
                                rsx! {
                                    li { key: "{module.meta.id}",
                                        details {
                                            class: "roadmap-module",
                                            id: "{module.meta.id}",
                                            "data-state": "{state}",
                                            open: holds_open,
                                            summary {
                                                span { class: "roadmap-stop", aria_hidden: "true", "{n}" }
                                                h3 { class: "roadmap-module-title", "{module.meta.title}" }
                                                span { class: "roadmap-module-count",
                                                    "{finished} of {lessons.len()}"
                                                    span { class: "roadmap-sr", " lessons done" }
                                                }
                                            }
                                            ol { class: "roadmap-lessons",
                                                for section in lessons.iter() {
                                                    {
                                                        let id = section.id.clone();
                                                        let current = open.as_deref() == Some(id.as_str());
                                                        let finished = is_done(&id);
                                                        rsx! {
                                                            li { key: "{id}",
                                                                button {
                                                                    class: "roadmap-lesson",
                                                                    r#type: "button",
                                                                    "data-lesson": "{id}",
                                                                    "data-done": "{finished}",
                                                                    aria_current: if current { "step" } else { "false" },
                                                                    onclick: {
                                                                        let id = id.clone();
                                                                        move |_| on_open.call(id.clone())
                                                                    },
                                                                    span { class: "roadmap-lesson-number", "{id}" }
                                                                    span { class: "roadmap-lesson-title",
                                                                        "{section.title}"
                                                                        if finished {
                                                                            span { class: "roadmap-sr", ", done" }
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
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
