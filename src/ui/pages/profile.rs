//! User Profile page
//!
//! Displays user statistics, progress, and achievements.

use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::progress::UserProgress;
use crate::content::ContentEngine;

const PROFILE_STYLE: &str = r#"
.profile-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
    font-family: var(--font-sans);
}

.profile-hero {
    max-width: 1000px;
    margin: 0 auto;
    padding: 60px var(--spacing-xl) 40px;
    text-align: center;
}

.profile-avatar {
    width: 100px;
    height: 100px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 48px;
    margin: 0 auto var(--spacing-lg);
    box-shadow: 0 8px 32px rgba(96, 165, 250, 0.3);
}

.profile-name {
    font-size: var(--font-heading-lg);
    font-weight: 900;
    margin-bottom: var(--spacing-sm);
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}

.profile-title {
    font-size: var(--font-body-lg);
    color: var(--color-accent-purple);
    font-weight: 600;
}

.profile-content {
    max-width: 1000px;
    margin: 0 auto;
    padding: 0 var(--spacing-xl) 80px;
}

.profile-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--spacing-lg);
    margin-bottom: var(--spacing-xxl);
}

.stat-card {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-xl);
    padding: var(--spacing-xl);
    text-align: center;
    transition: all 0.2s ease;
}

.stat-card:hover {
    background: rgba(255, 255, 255, 0.06);
    transform: translateY(-2px);
}

.stat-value {
    font-size: var(--font-display-md);
    font-weight: 900;
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: var(--spacing-xs);
}

.stat-value.xp {
    background: linear-gradient(135deg, #fbbf24, #f59e0b);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}

.stat-value.streak {
    background: linear-gradient(135deg, #4ade80, #22c55e);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}

.stat-value.level {
    background: linear-gradient(135deg, var(--color-accent-purple), #c084fc);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
}

.stat-label {
    font-size: var(--font-body-md);
    color: var(--text-secondary);
    font-weight: 500;
}

.profile-section {
    margin-bottom: var(--spacing-xxl);
}

.profile-section-title {
    font-size: var(--font-heading-sm);
    font-weight: 700;
    margin-bottom: var(--spacing-lg);
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
}

.progress-card {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
    margin-bottom: var(--spacing-md);
}

.progress-era-name {
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-sm);
}

.progress-bar-container {
    height: 8px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-full);
    overflow: hidden;
    margin-bottom: var(--spacing-xs);
}

.progress-bar {
    height: 100%;
    background: linear-gradient(90deg, var(--color-accent-blue), var(--color-accent-purple));
    border-radius: var(--radius-full);
    transition: width 0.3s ease;
}

.progress-text {
    font-size: var(--font-caption-md);
    color: var(--text-tertiary);
}

.achievements-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: var(--spacing-md);
}

.achievement-badge {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
    text-align: center;
    transition: all 0.2s ease;
}

.achievement-badge:hover {
    background: rgba(255, 255, 255, 0.06);
    transform: scale(1.02);
}

.achievement-badge.locked {
    opacity: 0.4;
    filter: grayscale(100%);
}

.achievement-icon {
    font-size: 32px;
    margin-bottom: var(--spacing-sm);
}

.achievement-name {
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--text-primary);
}

.empty-state {
    text-align: center;
    padding: var(--spacing-xxl);
    color: var(--text-tertiary);
}
"#;

#[component]
pub fn Profile() -> Element {
    let progress = UserProgress::new(); // TODO: Load from storage
    let engine = ContentEngine::new();

    // Calculate totals
    let total_exercises: usize = engine.eras()
        .iter()
        .flat_map(|e| e.modules.iter())
        .map(|m| m.exercises.len())
        .sum();

    let completed_modules = progress.modules.values().filter(|m| m.completed).count();
    let total_modules: usize = engine.eras().iter().map(|e| e.modules.len()).sum();

    rsx! {
        style { "{PROFILE_STYLE}" }
        div { class: "profile-page",
            MainNav { active: ActivePage::Profile }

            // Hero section
            div { class: "profile-hero",
                div { class: "profile-avatar", "L" }
                h1 { class: "profile-name", "Logic Learner" }
                p { class: "profile-title",
                    if progress.title.is_some() {
                        "{progress.title.as_ref().unwrap()}"
                    } else {
                        "Apprentice Logician"
                    }
                }
            }

            // Content
            div { class: "profile-content",
                // Stats grid
                div { class: "profile-stats",
                    div { class: "stat-card",
                        div { class: "stat-value xp", "{progress.xp}" }
                        div { class: "stat-label", "Total XP" }
                    }
                    div { class: "stat-card",
                        div { class: "stat-value level", "Level {progress.level}" }
                        div { class: "stat-label", "Current Level" }
                    }
                    div { class: "stat-card",
                        div { class: "stat-value streak", "{progress.streak_days}" }
                        div { class: "stat-label", "Day Streak" }
                    }
                    div { class: "stat-card",
                        div { class: "stat-value", "{completed_modules}/{total_modules}" }
                        div { class: "stat-label", "Modules Completed" }
                    }
                }

                // Era Progress
                div { class: "profile-section",
                    h2 { class: "profile-section-title", "Progress by Era" }

                    for era in engine.eras() {
                        {
                            let era_modules = era.modules.len();
                            let era_completed = era.modules.iter()
                                .filter(|m| progress.modules.get(&m.meta.id).map_or(false, |p| p.completed))
                                .count();
                            let percent = if era_modules > 0 { (era_completed * 100) / era_modules } else { 0 };

                            rsx! {
                                div { class: "progress-card",
                                    div { class: "progress-era-name", "{era.meta.title}" }
                                    div { class: "progress-bar-container",
                                        div {
                                            class: "progress-bar",
                                            style: "width: {percent}%;",
                                        }
                                    }
                                    div { class: "progress-text", "{era_completed} of {era_modules} modules completed" }
                                }
                            }
                        }
                    }
                }

                // Achievements
                div { class: "profile-section",
                    h2 { class: "profile-section-title", "Achievements" }

                    if progress.achievements.is_empty() {
                        div { class: "empty-state",
                            p { "Complete modules and practice exercises to earn achievements!" }
                        }
                    } else {
                        div { class: "achievements-grid",
                            for achievement in progress.achievements.iter() {
                                div { class: "achievement-badge",
                                    div { class: "achievement-icon", "🏆" }
                                    div { class: "achievement-name", "{achievement}" }
                                }
                            }
                        }
                    }

                    // Show locked achievements
                    div { class: "achievements-grid", style: "margin-top: var(--spacing-lg);",
                        div { class: "achievement-badge locked",
                            div { class: "achievement-icon", "🎯" }
                            div { class: "achievement-name", "First Blood" }
                        }
                        div { class: "achievement-badge locked",
                            div { class: "achievement-icon", "🔥" }
                            div { class: "achievement-name", "7-Day Streak" }
                        }
                        div { class: "achievement-badge locked",
                            div { class: "achievement-icon", "💯" }
                            div { class: "achievement-name", "Perfect Score" }
                        }
                        div { class: "achievement-badge locked",
                            div { class: "achievement-icon", "🧠" }
                            div { class: "achievement-name", "Logic Master" }
                        }
                    }
                }
            }
        }
    }
}
