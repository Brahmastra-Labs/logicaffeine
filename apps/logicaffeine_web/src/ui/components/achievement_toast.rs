//! Achievement unlock celebration overlay.
//!
//! Displays a full-screen modal when the user unlocks an achievement,
//! with particle effects and sound feedback.
//!
//! # Props
//!
//! - `achievement` - The achievement that was unlocked
//! - `on_dismiss` - Callback when the user dismisses the overlay

use dioxus::prelude::*;
use crate::achievements::Achievement;
use crate::audio::{SoundEffect, play_sound};
use crate::ui::components::icon::{Icon, IconVariant, IconSize};

const ACHIEVEMENT_STYLE: &str = r#"
.achievement-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: overlay-fade 0.3s ease-out;
}

@keyframes overlay-fade {
    from { opacity: 0; }
    to { opacity: 1; }
}

.achievement-card {
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
    border: 2px solid #fbbf24;
    border-radius: 20px;
    padding: 40px 60px;
    text-align: center;
    box-shadow: 0 0 60px rgba(251, 191, 36, 0.4);
    animation: card-appear 0.5s ease-out;
}

@keyframes card-appear {
    0% { transform: scale(0.5) translateY(50px); opacity: 0; }
    70% { transform: scale(1.05) translateY(0); }
    100% { transform: scale(1) translateY(0); opacity: 1; }
}

.achievement-icon {
    font-size: 64px;
    margin-bottom: 16px;
    animation: icon-bounce 0.5s ease-out 0.3s both;
}

@keyframes icon-bounce {
    0% { transform: scale(0); }
    50% { transform: scale(1.3); }
    100% { transform: scale(1); }
}

.achievement-label {
    font-size: 14px;
    color: #fbbf24;
    text-transform: uppercase;
    letter-spacing: 2px;
    margin-bottom: 8px;
}

.achievement-title {
    font-size: 32px;
    font-weight: 700;
    color: #fff;
    margin-bottom: 12px;
}

.achievement-description {
    font-size: 16px;
    color: #888;
    margin-bottom: 24px;
}

.achievement-reward {
    font-size: 24px;
    color: #4ade80;
    font-weight: 600;
    margin-bottom: 16px;
}

.achievement-title-unlock {
    background: linear-gradient(90deg, #fbbf24, #f59e0b);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    font-size: 18px;
    font-weight: 600;
    margin-bottom: 24px;
}

.achievement-dismiss {
    padding: 12px 32px;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    border: none;
    border-radius: 8px;
    color: white;
    font-size: 16px;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.2s ease;
}

.achievement-dismiss:hover {
    transform: scale(1.05);
}

.particles {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    pointer-events: none;
    overflow: hidden;
}

.particle {
    position: absolute;
    font-size: 24px;
    animation: particle-fall 2s ease-out forwards;
}

@keyframes particle-fall {
    0% { transform: translateY(-50px) rotate(0deg); opacity: 1; }
    100% { transform: translateY(100vh) rotate(720deg); opacity: 0; }
}
"#;

#[component]
pub fn AchievementToast(achievement: &'static Achievement, on_dismiss: EventHandler<()>) -> Element {
    use_effect(move || {
        play_sound(SoundEffect::Achievement);
    });

    rsx! {
        style { "{ACHIEVEMENT_STYLE}" }
        div {
            class: "achievement-overlay",
            onclick: move |_| on_dismiss.call(()),
            div { class: "particles",
                for i in 0..20 {
                    span {
                        class: "particle",
                        style: "left: {(i * 5) % 100}%; animation-delay: {i as f32 * 0.1}s;",
                        if i % 2 == 0 {
                            Icon { variant: IconVariant::Star, size: IconSize::Large, color: "#fbbf24" }
                        } else {
                            Icon { variant: IconVariant::Sparkles, size: IconSize::Large, color: "#a78bfa" }
                        }
                    }
                }
            }
            div { class: "achievement-card",
                div { class: "achievement-icon",
                    Icon { variant: IconVariant::Trophy, size: IconSize::XXLarge, color: "#fbbf24" }
                }
                div { class: "achievement-label", "Achievement Unlocked" }
                div { class: "achievement-title", "{achievement.title}" }
                div { class: "achievement-description", "{achievement.description}" }
                div { class: "achievement-reward", "+{achievement.xp_reward} XP" }
                if let Some(title) = achievement.unlocks_title {
                    div { class: "achievement-title-unlock",
                        "Title Unlocked: {title}"
                    }
                }
                if achievement.grants_freeze {
                    div { class: "achievement-title-unlock",
                        style: "display: flex; align-items: center; gap: 8px; justify-content: center;",
                        Icon { variant: IconVariant::Shield, size: IconSize::Medium, color: "#38bdf8" }
                        "+1 Streak Freeze!"
                    }
                }
                button {
                    class: "achievement-dismiss",
                    onclick: move |e| {
                        e.stop_propagation();
                        on_dismiss.call(());
                    },
                    "Continue"
                }
            }
        }
    }
}
