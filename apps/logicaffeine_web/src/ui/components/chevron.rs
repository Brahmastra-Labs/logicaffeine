//! The Studio's one drawn chevron: the file tree's folders and the collapsible blocks turn it to
//! say whether they are open.

use dioxus::prelude::*;

/// VS Code's chevron shape, 12px along its long side on a 2px stroke — two thirds larger than the
/// codicon at 10px, and a pixel heavier — in a 12px square, so it centres on its row in any font
/// and turns about its middle to point down.
pub const CHEVRON_SVG: &str = r#"<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 1 8.5 6 3.5 11"/></svg>"#;

/// The chevron's look wherever it is drawn: muted, filling its box, and a quarter turn to point
/// down while what it marks is open. A component that draws chevrons carries this once.
pub const CHEVRON_STYLE: &str = r#"
.chevron {
    display: flex;
    flex: none;
    color: rgba(255, 255, 255, 0.6);
    transition: transform 0.15s ease;
}

.chevron.open {
    transform: rotate(90deg);
}
"#;

/// The drawn chevron: pointing right while `open` is false, down while it is true.
#[component]
pub fn Chevron(open: bool) -> Element {
    rsx! {
        span { class: if open { "chevron open" } else { "chevron" }, dangerous_inner_html: CHEVRON_SVG }
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/chevron/tests.rs"]
mod tests;
