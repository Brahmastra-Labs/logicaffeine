//! XP reward popup notification.
//!
//! Displays a centered popup showing XP earned with breakdown of bonuses.
//! Auto-dismisses after 2 seconds. Plays sound effect on mount.
//!
//! # Props
//!
//! - `reward` - XP breakdown with base, bonuses, and total
//! - `on_dismiss` - Callback when popup closes (auto or click)
//!
//! # Special Effects
//!
//! - Critical hits (random chance) show gold styling and pulsing glow
//! - Different sound for critical vs normal XP gain

use dioxus::prelude::*;
use crate::game::XpReward;
use crate::audio::{SoundEffect, play_sound};

const XP_POPUP_STYLE: &str = r#"
.xp-popup {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 1000;
    cursor: pointer;
    animation: xp-appear 2s ease-out forwards;
}

@keyframes xp-appear {
    0% { opacity: 0; transform: translate(-50%, -50%) scale(0.5); }
    10% { opacity: 1; transform: translate(-50%, -50%) scale(1.1); }
    20% { opacity: 1; transform: translate(-50%, -50%) scale(1); }
    80% { opacity: 1; transform: translate(-50%, -50%) scale(1); }
    100% { opacity: 0; transform: translate(-50%, -50%) scale(0.9) translateY(-20px); }
}

.xp-popup-content {
    background: rgba(0, 0, 0, 0.9);
    border: 2px solid #667eea;
    border-radius: 16px;
    padding: 24px 40px;
    text-align: center;
    box-shadow: 0 0 40px rgba(102, 126, 234, 0.4);
}

.xp-total {
    font-size: 48px;
    font-weight: 700;
    color: #4ade80;
    margin-bottom: 8px;
}

.xp-total.critical {
    color: #fbbf24;
    text-shadow: 0 0 20px #fbbf24;
    animation: critical-pulse 0.5s ease-in-out infinite alternate;
}

@keyframes critical-pulse {
    from { text-shadow: 0 0 20px #fbbf24; }
    to { text-shadow: 0 0 40px #fbbf24, 0 0 60px #f59e0b; }
}

.xp-breakdown {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 14px;
    color: #888;
}

.xp-line {
    display: flex;
    justify-content: space-between;
    gap: 16px;
}

.xp-line.combo { color: #f97316; }
.xp-line.streak { color: #06b6d4; }
.xp-line.critical { color: #fbbf24; font-weight: 600; }
.xp-line.first-try { color: #a78bfa; }
"#;

#[component]
pub fn XpPopup(reward: XpReward, on_dismiss: EventHandler<()>) -> Element {
    use_effect(move || {
        if reward.is_critical {
            play_sound(SoundEffect::CriticalHit);
        } else {
            play_sound(SoundEffect::XpGain);
        }
    });

    use_effect(move || {
        let handler = on_dismiss.clone();
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(2000).await;
            handler.call(());
        });
    });

    let total_class = if reward.is_critical { "xp-total critical" } else { "xp-total" };

    rsx! {
        style { "{XP_POPUP_STYLE}" }
        div {
            class: "xp-popup",
            onclick: move |_| on_dismiss.call(()),
            div { class: "xp-popup-content",
                div { class: "{total_class}", "+{reward.total} XP" }
                div { class: "xp-breakdown",
                    div { class: "xp-line",
                        span { "Base" }
                        span { "+{reward.base}" }
                    }
                    if reward.combo_bonus > 0 {
                        div { class: "xp-line combo",
                            span { "Combo Bonus" }
                            span { "+{reward.combo_bonus}" }
                        }
                    }
                    if reward.streak_bonus > 0 {
                        div { class: "xp-line streak",
                            span { "Streak Bonus" }
                            span { "+{reward.streak_bonus}" }
                        }
                    }
                    if reward.first_try_bonus > 0 {
                        div { class: "xp-line first-try",
                            span { "First Try" }
                            span { "+{reward.first_try_bonus}" }
                        }
                    }
                    if reward.critical_bonus > 0 {
                        div { class: "xp-line critical",
                            span { "CRITICAL!" }
                            span { "+{reward.critical_bonus}" }
                        }
                    }
                }
            }
        }
    }
}
