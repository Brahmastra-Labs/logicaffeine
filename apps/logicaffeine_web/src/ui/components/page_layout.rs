//! Page layout wrapper component.
//!
//! Provides consistent structure for pages with:
//! - MainNav header with configurable options
//! - Responsive footer (optional, based on variant)
//! - Proper min-height for full viewport coverage
//!
//! # Variants
//! - `Standard`: Full header + full footer (most pages)
//! - `Minimal`: Full header + minimal footer (legal pages)
//! - `NoFooter`: Full header + no footer (Studio, Workspace)
//! - `Landing`: Landing page variant with custom footer handling

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::{Footer, FooterVariant};

const PAGE_LAYOUT_STYLES: &str = r#"
.page-layout {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
    background: var(--bg-dark, #060814);
}

.page-layout-content {
    flex: 1;
    display: flex;
    flex-direction: column;
}

/* For full-height app pages (Studio, Workspace) that need no footer */
.page-layout.no-footer .page-layout-content {
    flex: 1;
    overflow: hidden;
}
"#;

/// Layout variant determining header/footer configuration
#[derive(Clone, Copy, PartialEq, Default)]
pub enum LayoutVariant {
    #[default]
    Standard,
    Minimal,
    NoFooter,
    Landing,
}

#[component]
pub fn PageLayout(
    #[props(default)]
    active_page: ActivePage,
    #[props(default)]
    subtitle: Option<&'static str>,
    #[props(default = true)]
    show_nav_links: bool,
    #[props(default)]
    variant: LayoutVariant,
    children: Element,
) -> Element {
    let layout_class = match variant {
        LayoutVariant::NoFooter => "page-layout no-footer",
        _ => "page-layout",
    };

    rsx! {
        style { "{PAGE_LAYOUT_STYLES}" }
        div { class: layout_class,
            MainNav {
                active: active_page,
                subtitle: subtitle,
                show_nav_links: show_nav_links,
            }

            main { class: "page-layout-content",
                {children}
            }

            // Render footer based on variant
            match variant {
                LayoutVariant::Standard => rsx! { Footer { variant: FooterVariant::Full } },
                LayoutVariant::Minimal => rsx! { Footer { variant: FooterVariant::Minimal } },
                LayoutVariant::NoFooter => rsx! {},
                LayoutVariant::Landing => rsx! {}, // Landing has its own custom footer
            }
        }
    }
}
