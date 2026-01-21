//! Daily streak display component.
//!
//! Shows the user's current daily streak with status indicator and freeze tokens.
//! Different visual states for active, at-risk, frozen, and lost streaks.
//!
//! # Props
//!
//! - `streak` - Current streak day count
//! - `status` - Streak status (Active, AtRisk, Frozen, Lost)
//! - `freezes` - Number of available streak freeze tokens (0-3)

use dioxus::prelude::*;
use crate::game::StreakStatus;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};

const STREAK_STYLE: &str = r#"
.streak-display {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-radius: 20px;
    font-size: 14px;
    font-weight: 500;
}

.streak-active {
    background: rgba(249, 115, 22, 0.15);
    border: 1px solid rgba(249, 115, 22, 0.3);
    color: #f97316;
}

.streak-at-risk {
    background: rgba(248, 113, 113, 0.15);
    border: 1px solid rgba(248, 113, 113, 0.3);
    color: #f87171;
    animation: risk-pulse 1s ease-in-out infinite;
}

@keyframes risk-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
}

.streak-frozen {
    background: rgba(56, 189, 248, 0.15);
    border: 1px solid rgba(56, 189, 248, 0.3);
    color: #38bdf8;
}

.streak-lost {
    background: rgba(107, 114, 128, 0.15);
    border: 1px solid rgba(107, 114, 128, 0.3);
    color: #9ca3af;
}

.streak-icon {
    font-size: 16px;
}

.streak-count {
    font-weight: 600;
}

.streak-label {
    color: #888;
    font-size: 12px;
}

.freeze-tokens {
    display: flex;
    gap: 4px;
    margin-left: 8px;
    padding-left: 8px;
    border-left: 1px solid rgba(255, 255, 255, 0.1);
}

.freeze-token {
    font-size: 12px;
    opacity: 0.8;
}

.freeze-token.empty {
    opacity: 0.3;
}
"#;

#[component]
pub fn StreakDisplay(streak: u32, status: StreakStatus, freezes: u8) -> Element {
    let (class, icon_variant, icon_color, text) = match status {
        StreakStatus::Active { days } => {
            ("streak-display streak-active", IconVariant::Fire, "#f97316", format!("{} day streak", days))
        }
        StreakStatus::AtRisk => {
            ("streak-display streak-at-risk", IconVariant::Warning, "#f87171", "Streak at risk!".to_string())
        }
        StreakStatus::Frozen => {
            ("streak-display streak-frozen", IconVariant::Shield, "#38bdf8", format!("{} days (frozen)", streak))
        }
        StreakStatus::Lost { was } => {
            ("streak-display streak-lost", IconVariant::HeartBroken, "#9ca3af", format!("Lost {} day streak", was))
        }
    };

    rsx! {
        style { "{STREAK_STYLE}" }
        div { class: "{class}",
            span { class: "streak-icon",
                Icon { variant: icon_variant, size: IconSize::Medium, color: icon_color }
            }
            span { class: "streak-count", "{text}" }
            div { class: "freeze-tokens",
                for i in 0..3u8 {
                    span {
                        class: if i < freezes { "freeze-token" } else { "freeze-token empty" },
                        Icon { variant: IconVariant::Shield, size: IconSize::Small, color: "#38bdf8" }
                    }
                }
            }
        }
    }
}
