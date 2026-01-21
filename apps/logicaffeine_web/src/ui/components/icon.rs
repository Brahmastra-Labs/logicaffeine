//! Centralized SVG icon component system.
//!
//! Provides a unified icon component with 30+ variants for consistent iconography
//! across the application. All icons are inline SVGs (24x24 viewBox, stroke-based)
//! for crisp rendering at any size.
//!
//! # Usage
//!
//! ```ignore
//! use crate::ui::components::icon::{Icon, IconVariant, IconSize};
//!
//! rsx! {
//!     Icon { variant: IconVariant::Book, size: IconSize::Medium }
//!     Icon { variant: IconVariant::Fire, size: IconSize::Large, class: "text-orange" }
//! }
//! ```

use dioxus::prelude::*;

/// Icon size variants.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum IconSize {
    /// 16px - for inline text and tight spaces
    Small,
    /// 20px - default size for most UI
    #[default]
    Medium,
    /// 24px - for emphasis and larger displays
    Large,
    /// 32px - for hero sections and large displays
    XLarge,
    /// 48px - for achievement toasts and celebrations
    XXLarge,
}

impl IconSize {
    /// Returns the pixel size as a string.
    pub fn px(&self) -> &'static str {
        match self {
            IconSize::Small => "16px",
            IconSize::Medium => "20px",
            IconSize::Large => "24px",
            IconSize::XLarge => "32px",
            IconSize::XXLarge => "48px",
        }
    }
}

/// All available icon variants.
#[derive(Clone, Copy, PartialEq)]
pub enum IconVariant {
    // Navigation
    Book,
    Package,
    GraduationCap,
    Beaker,
    Newspaper,
    Map,
    Diamond,
    User,

    // Gamification
    Fire,
    Trophy,
    Star,
    Shield,
    HeartBroken,
    Target,
    Lightning,
    Brain,

    // Files
    Folder,
    FolderOpen,
    File,

    // Actions
    Check,
    Close,
    ChevronRight,
    ChevronDown,
    Plus,
    Minus,
    Menu,
    Warning,

    // Misc
    Lock,
    Document,
    Tools,
    Crab,
    Sparkles,
    Owl,
    Clock,
    Github,

    // Theme icons
    Sunrise,
    Moon,
    Wave,
    Mountain,
}

impl IconVariant {
    /// Returns the SVG path data for this icon.
    fn svg_content(&self) -> &'static str {
        match self {
            // Book - open book icon
            IconVariant::Book => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 6.042A8.967 8.967 0 0 0 6 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 0 1 6 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 0 1 6-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0 0 18 18a8.967 8.967 0 0 0-6 2.292m0-14.25v14.25"/>"#,

            // Package - box/package icon
            IconVariant::Package => r#"<path stroke-linecap="round" stroke-linejoin="round" d="m21 7.5-9-5.25L3 7.5m18 0-9 5.25m9-5.25v9l-9 5.25M3 7.5l9 5.25M3 7.5v9l9 5.25m0-9v9"/>"#,

            // GraduationCap - academic cap
            IconVariant::GraduationCap => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M4.26 10.147a60.438 60.438 0 0 0-.491 6.347A48.62 48.62 0 0 1 12 20.904a48.62 48.62 0 0 1 8.232-4.41 60.46 60.46 0 0 0-.491-6.347m-15.482 0a50.636 50.636 0 0 0-2.658-.813A59.906 59.906 0 0 1 12 3.493a59.903 59.903 0 0 1 10.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.717 50.717 0 0 1 12 13.489a50.702 50.702 0 0 1 7.74-3.342M6.75 15a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm0 0v-3.675A55.378 55.378 0 0 1 12 8.443m-7.007 11.55A5.981 5.981 0 0 0 6.75 15.75v-1.5"/>"#,

            // Beaker - science/lab flask
            IconVariant::Beaker => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.75 3.104v5.714a2.25 2.25 0 0 1-.659 1.591L5 14.5M9.75 3.104c-.251.023-.501.05-.75.082m.75-.082a24.301 24.301 0 0 1 4.5 0m0 0v5.714c0 .597.237 1.17.659 1.591L19.8 15.3M14.25 3.104c.251.023.501.05.75.082M19.8 15.3l-1.57.393A9.065 9.065 0 0 1 12 15a9.065 9.065 0 0 1-6.23.693L5 15.3m14.8 0 .002 1.057a2.254 2.254 0 0 1-1.593 2.15l-1.721.489a2.254 2.254 0 0 1-1.236 0l-1.006-.286a2.25 2.25 0 0 0-1.236 0l-1.006.286a2.254 2.254 0 0 1-1.236 0l-1.72-.489a2.254 2.254 0 0 1-1.594-2.15l.002-1.057"/>"#,

            // Newspaper - news icon
            IconVariant::Newspaper => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 7.5h1.5m-1.5 3h1.5m-7.5 3h7.5m-7.5 3h7.5m3-9h3.375c.621 0 1.125.504 1.125 1.125V18a2.25 2.25 0 0 1-2.25 2.25M16.5 7.5V18a2.25 2.25 0 0 0 2.25 2.25M16.5 7.5V4.875c0-.621-.504-1.125-1.125-1.125H4.125C3.504 3.75 3 4.254 3 4.875V18a2.25 2.25 0 0 0 2.25 2.25h13.5M6 7.5h3v3H6v-3Z"/>"#,

            // Map - roadmap/directions
            IconVariant::Map => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9 6.75V15m0-8.25L6 4.5m3 2.25 3-1.5M9 15l-3 1.5m3-1.5 3-1.5m0 0V6.75m0 6.75 3 1.5m-3-1.5 3-1.5m0 0V6.75m0 6.75 3-1.5M15 6.75l3-1.5M15 15l3 1.5m-3-1.5V6.75m3 8.25V6.75m0 8.25-3-1.5m3 1.5-3 1.5m-9-1.5 3 1.5M6 15V6.75m0 8.25L3 15.75V7.5l3-1.5m12-1.5 3 1.5v8.25l-3-1.5"/>"#,

            // Diamond - premium/pricing
            IconVariant::Diamond => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 2L2 8.5l10 12.5 10-12.5L12 2z M2 8.5h20 M7 8.5l5 12.5 5-12.5 M7 8.5l5-6.5 5 6.5"/>"#,

            // User - profile/person
            IconVariant::User => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z"/>"#,

            // Fire - streak/combo
            IconVariant::Fire => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M15.362 5.214A8.252 8.252 0 0 1 12 21 8.25 8.25 0 0 1 6.038 7.047 8.287 8.287 0 0 0 9 9.601a8.983 8.983 0 0 1 3.361-6.867 8.21 8.21 0 0 0 3 2.48Z"/><path stroke-linecap="round" stroke-linejoin="round" d="M12 18a3.75 3.75 0 0 0 .495-7.468 5.99 5.99 0 0 0-1.925 3.547 5.975 5.975 0 0 1-2.133-1.001A3.75 3.75 0 0 0 12 18Z"/>"#,

            // Trophy - achievement
            IconVariant::Trophy => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M16.5 18.75h-9m9 0a3 3 0 0 1 3 3h-15a3 3 0 0 1 3-3m9 0v-3.375c0-.621-.503-1.125-1.125-1.125h-.871M7.5 18.75v-3.375c0-.621.504-1.125 1.125-1.125h.872m5.007 0H9.497m5.007 0a7.454 7.454 0 0 1-.982-3.172M9.497 14.25a7.454 7.454 0 0 0 .981-3.172M5.25 4.236c-.982.143-1.954.317-2.916.52A6.003 6.003 0 0 0 7.73 9.728M5.25 4.236V4.5c0 2.108.966 3.99 2.48 5.228M5.25 4.236V2.721C7.456 2.41 9.71 2.25 12 2.25c2.291 0 4.545.16 6.75.47v1.516M18.75 4.236c.982.143 1.954.317 2.916.52A6.003 6.003 0 0 1 16.27 9.728M18.75 4.236V4.5c0 2.108-.966 3.99-2.48 5.228m4.645-.228a48.394 48.394 0 0 1-1.414-.071 3.003 3.003 0 0 0-3.48 2.9c-.044.462-.055.928-.033 1.39a3.003 3.003 0 0 0 3.48 2.9c.478-.058.952-.131 1.423-.219"/>"#,

            // Star - rating/favorite
            IconVariant::Star => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M11.48 3.499a.562.562 0 0 1 1.04 0l2.125 5.111a.563.563 0 0 0 .475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 0 0-.182.557l1.285 5.385a.562.562 0 0 1-.84.61l-4.725-2.885a.562.562 0 0 0-.586 0L6.982 20.54a.562.562 0 0 1-.84-.61l1.285-5.386a.562.562 0 0 0-.182-.557l-4.204-3.602a.562.562 0 0 1 .321-.988l5.518-.442a.563.563 0 0 0 .475-.345L11.48 3.5Z"/>"#,

            // Shield - protection/freeze
            IconVariant::Shield => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75 11.25 15 15 9.75m-3-7.036A11.959 11.959 0 0 1 3.598 6 11.99 11.99 0 0 0 3 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285Z"/>"#,

            // HeartBroken - lost streak
            IconVariant::HeartBroken => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5c-2.5-2.5-6.5-2.5-9 0s-2.5 6.5 0 9l9 9 9-9c2.5-2.5 2.5-6.5 0-9s-6.5-2.5-9 0m0 0l-2 4 4 2-2 4"/>"#,

            // Target - goals/objectives
            IconVariant::Target => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 12m-9 0a9 9 0 1 0 18 0 9 9 0 1 0-18 0m9 0m-5 0a5 5 0 1 0 10 0 5 5 0 1 0-10 0m5 0m-1 0a1 1 0 1 0 2 0 1 1 0 1 0-2 0"/>"#,

            // Lightning - speed/action
            IconVariant::Lightning => r#"<path stroke-linecap="round" stroke-linejoin="round" d="m3.75 13.5 10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75Z"/>"#,

            // Brain - intelligence/thinking
            IconVariant::Brain => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904 9 18.75l-.813-2.846a4.5 4.5 0 0 0-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 0 0 3.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 0 0 3.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 0 0-3.09 3.09ZM18.259 8.715 18 9.75l-.259-1.035a3.375 3.375 0 0 0-2.455-2.456L14.25 6l1.036-.259a3.375 3.375 0 0 0 2.455-2.456L18 2.25l.259 1.035a3.375 3.375 0 0 0 2.456 2.456L21.75 6l-1.035.259a3.375 3.375 0 0 0-2.456 2.456ZM16.894 20.567 16.5 21.75l-.394-1.183a2.25 2.25 0 0 0-1.423-1.423L13.5 18.75l1.183-.394a2.25 2.25 0 0 0 1.423-1.423l.394-1.183.394 1.183a2.25 2.25 0 0 0 1.423 1.423l1.183.394-1.183.394a2.25 2.25 0 0 0-1.423 1.423Z"/>"#,

            // Folder - closed folder
            IconVariant::Folder => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12.75V12A2.25 2.25 0 0 1 4.5 9.75h15A2.25 2.25 0 0 1 21.75 12v.75m-8.69-6.44-2.12-2.12a1.5 1.5 0 0 0-1.061-.44H4.5A2.25 2.25 0 0 0 2.25 6v12a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9a2.25 2.25 0 0 0-2.25-2.25h-5.379a1.5 1.5 0 0 1-1.06-.44Z"/>"#,

            // FolderOpen - open folder
            IconVariant::FolderOpen => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 9.776c.112-.017.227-.026.344-.026h15.812c.117 0 .232.009.344.026m-16.5 0a2.25 2.25 0 0 0-1.883 2.542l.857 6a2.25 2.25 0 0 0 2.227 1.932H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-1.883-2.542m-16.5 0V6A2.25 2.25 0 0 1 6 3.75h3.879a1.5 1.5 0 0 1 1.06.44l2.122 2.12a1.5 1.5 0 0 0 1.06.44H18A2.25 2.25 0 0 1 20.25 9v.776"/>"#,

            // File - document file
            IconVariant::File => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z"/>"#,

            // Check - success/complete
            IconVariant::Check => r#"<path stroke-linecap="round" stroke-linejoin="round" d="m4.5 12.75 6 6 9-13.5"/>"#,

            // Close - dismiss/cancel
            IconVariant::Close => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M6 18 18 6M6 6l12 12"/>"#,

            // ChevronRight - expand/navigate
            IconVariant::ChevronRight => r#"<path stroke-linecap="round" stroke-linejoin="round" d="m8.25 4.5 7.5 7.5-7.5 7.5"/>"#,

            // ChevronDown - dropdown/collapse
            IconVariant::ChevronDown => r#"<path stroke-linecap="round" stroke-linejoin="round" d="m19.5 8.25-7.5 7.5-7.5-7.5"/>"#,

            // Plus - add
            IconVariant::Plus => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15"/>"#,

            // Minus - remove
            IconVariant::Minus => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14"/>"#,

            // Menu - hamburger menu
            IconVariant::Menu => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5"/>"#,

            // Warning - alert/caution
            IconVariant::Warning => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z"/>"#,

            // Lock - protected/locked
            IconVariant::Lock => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 1 0-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 0 0 2.25-2.25v-6.75a2.25 2.25 0 0 0-2.25-2.25H6.75a2.25 2.25 0 0 0-2.25 2.25v6.75a2.25 2.25 0 0 0 2.25 2.25Z"/>"#,

            // Document - clipboard/doc
            IconVariant::Document => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M15.666 3.888A2.25 2.25 0 0 0 13.5 2.25h-3c-1.03 0-1.9.693-2.166 1.638m7.332 0c.055.194.084.4.084.612v0a.75.75 0 0 1-.75.75H9.75a.75.75 0 0 1-.75-.75v0c0-.212.03-.418.084-.612m7.332 0c.646.049 1.288.11 1.927.184 1.1.128 1.907 1.077 1.907 2.185V19.5a2.25 2.25 0 0 1-2.25 2.25H6.75A2.25 2.25 0 0 1 4.5 19.5V6.257c0-1.108.806-2.057 1.907-2.185a48.208 48.208 0 0 1 1.927-.184"/>"#,

            // Tools - settings/utilities
            IconVariant::Tools => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M11.42 15.17 17.25 21A2.652 2.652 0 0 0 21 17.25l-5.877-5.877M11.42 15.17l2.496-3.03c.317-.384.74-.626 1.208-.766M11.42 15.17l-4.655 5.653a2.548 2.548 0 1 1-3.586-3.586l6.837-5.63m5.108-.233c.55-.164 1.163-.188 1.743-.14a4.5 4.5 0 0 0 4.486-6.336l-3.276 3.277a3.004 3.004 0 0 1-2.25-2.25l3.276-3.276a4.5 4.5 0 0 0-6.336 4.486c.091 1.076-.071 2.264-.904 2.95l-.102.085m-1.745 1.437L5.909 7.5H4.5L2.25 3.75l1.5-1.5L7.5 4.5v1.409l4.26 4.26m-1.745 1.437 1.745-1.437m6.615 8.206L15.75 15.75M4.867 19.125h.008v.008h-.008v-.008Z"/>"#,

            // Crab - Rust gear logo
            IconVariant::Crab => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"/><path stroke-linecap="round" stroke-linejoin="round" d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1Z"/>"#,

            // Sparkles - celebration/magic
            IconVariant::Sparkles => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904 9 18.75l-.813-2.846a4.5 4.5 0 0 0-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 0 0 3.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 0 0 3.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 0 0-3.09 3.09ZM18.259 8.715 18 9.75l-.259-1.035a3.375 3.375 0 0 0-2.455-2.456L14.25 6l1.036-.259a3.375 3.375 0 0 0 2.455-2.456L18 2.25l.259 1.035a3.375 3.375 0 0 0 2.456 2.456L21.75 6l-1.035.259a3.375 3.375 0 0 0-2.456 2.456ZM16.894 20.567 16.5 21.75l-.394-1.183a2.25 2.25 0 0 0-1.423-1.423L13.5 18.75l1.183-.394a2.25 2.25 0 0 0 1.423-1.423l.394-1.183.394 1.183a2.25 2.25 0 0 0 1.423 1.423l1.183.394-1.183.394a2.25 2.25 0 0 0-1.423 1.423Z"/>"#,

            // Owl - wisdom/learning
            IconVariant::Owl => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 3c-4 0-7 3-7 7v6c0 2.5 2 4.5 4.5 4.5h5c2.5 0 4.5-2 4.5-4.5v-6c0-4-3-7-7-7ZM8 11a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3ZM16 11a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3ZM12 15v2M10 15h4M5 10l-2-3M19 10l2-3"/>"#,

            // Clock - time/schedule
            IconVariant::Clock => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z"/>"#,

            // Github - social
            IconVariant::Github => r#"<path fill-rule="evenodd" clip-rule="evenodd" d="M12 2C6.477 2 2 6.477 2 12c0 4.42 2.865 8.164 6.839 9.489.5.092.682-.217.682-.482 0-.237-.009-.866-.013-1.7-2.782.603-3.369-1.342-3.369-1.342-.454-1.155-1.11-1.462-1.11-1.462-.908-.62.069-.608.069-.608 1.003.07 1.531 1.03 1.531 1.03.892 1.529 2.341 1.087 2.91.831.092-.646.35-1.086.636-1.336-2.22-.253-4.555-1.11-4.555-4.943 0-1.091.39-1.984 1.029-2.683-.103-.253-.446-1.27.098-2.647 0 0 .84-.269 2.75 1.025A9.578 9.578 0 0112 6.836c.85.004 1.705.114 2.504.336 1.909-1.294 2.747-1.025 2.747-1.025.546 1.377.203 2.394.1 2.647.64.699 1.028 1.592 1.028 2.683 0 3.842-2.339 4.687-4.566 4.935.359.309.678.919.678 1.852 0 1.336-.012 2.415-.012 2.743 0 .267.18.578.688.48C19.138 20.161 22 16.418 22 12c0-5.523-4.477-10-10-10z" fill="currentColor" stroke="none"/>"#,

            // Sunrise - theme icon for warm/dawn
            IconVariant::Sunrise => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M12 3v2.25m6.364.386-1.591 1.591M21 12h-2.25m-.386 6.364-1.591-1.591M12 18.75V21m-4.773-4.227-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0Z"/>"#,

            // Moon - theme icon for violet/twilight
            IconVariant::Moon => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M21.752 15.002A9.72 9.72 0 0 1 18 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 0 0 3 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 0 0 9.002-5.998Z"/>"#,

            // Wave - theme icon for ocean
            IconVariant::Wave => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M2 12c1-2 3-3 5-3s4 2 6 2 4-2 6-2 4 1 5 3M2 17c1-2 3-3 5-3s4 2 6 2 4-2 6-2 4 1 5 3"/>"#,

            // Mountain - theme icon for earth/grounded
            IconVariant::Mountain => r#"<path stroke-linecap="round" stroke-linejoin="round" d="M3 19h18M5 19l4-7 3 4 4-8 4 11"/>"#,
        }
    }
}

const ICON_STYLE: &str = r#"
.icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    vertical-align: middle;
    flex-shrink: 0;
}
"#;

/// SVG icon component with configurable size and styling.
///
/// # Props
///
/// - `variant` - The icon to display (see [`IconVariant`])
/// - `size` - Icon size (default: Medium/20px)
/// - `class` - Optional additional CSS classes
/// - `color` - Optional color override (uses currentColor by default)
#[component]
pub fn Icon(
    variant: IconVariant,
    #[props(default)]
    size: IconSize,
    #[props(default)]
    class: Option<&'static str>,
    #[props(default)]
    color: Option<&'static str>,
) -> Element {
    let size_px = size.px();
    let svg_content = variant.svg_content();
    let extra_class = class.unwrap_or("");
    let stroke_color = color.unwrap_or("currentColor");

    // GitHub icon uses fill instead of stroke
    let is_filled = matches!(variant, IconVariant::Github);

    rsx! {
        style { "{ICON_STYLE}" }
        if is_filled {
            svg {
                class: "icon {extra_class}",
                xmlns: "http://www.w3.org/2000/svg",
                width: "{size_px}",
                height: "{size_px}",
                view_box: "0 0 24 24",
                fill: "{stroke_color}",
                dangerous_inner_html: "{svg_content}"
            }
        } else {
            svg {
                class: "icon {extra_class}",
                xmlns: "http://www.w3.org/2000/svg",
                width: "{size_px}",
                height: "{size_px}",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "{stroke_color}",
                stroke_width: "1.5",
                dangerous_inner_html: "{svg_content}"
            }
        }
    }
}
