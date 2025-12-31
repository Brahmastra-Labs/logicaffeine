use dioxus::prelude::*;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::learn_sidebar::{LearnSidebar, ModuleInfo};
use crate::ui::components::symbol_dictionary::SymbolDictionary;
use crate::ui::components::vocab_reference::VocabReference;
use crate::ui::components::guide_code_block::GuideCodeBlock;
use crate::ui::pages::guide::content::ExampleMode;
use crate::content::ContentEngine;
use crate::generator::{Generator, AnswerType, Challenge};
use crate::grader::check_answer;
use crate::struggle::StruggleDetector;
use crate::unlock::check_module_unlocked;
use crate::progress::UserProgress;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::HashSet;

const LEARN_STYLE: &str = r#"
.learn-page {
    min-height: 100vh;
    color: var(--text-primary);
    background:
        radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.14), transparent 60%),
        radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.14), transparent 60%),
        radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.08), transparent 62%),
        linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
    font-family: var(--font-sans);
}

/* Hero */
.learn-hero {
    max-width: 1280px;
    margin: 0 auto;
    padding: 60px var(--spacing-xl) 40px;
}

.learn-hero h1 {
    font-size: var(--font-display-lg);
    font-weight: 900;
    letter-spacing: -1.5px;
    line-height: 1.1;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-lg);
}

.learn-hero p {
    font-size: var(--font-body-lg);
    color: var(--text-secondary);
    max-width: 600px;
    line-height: 1.6;
    margin: 0;
}

.learn-hero-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--spacing-sm);
    padding: var(--spacing-sm) 14px;
    border-radius: var(--radius-full);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-xl);
}

.learn-hero-badge .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--color-success);
    box-shadow: 0 0 0 4px rgba(34,197,94,0.15);
}

/* Layout */
.learn-layout {
    max-width: 1280px;
    margin: 0 auto;
    display: flex;
    gap: 48px;
    padding: 0 var(--spacing-xl) 80px;
}

/* Main content */
.learn-content {
    flex: 1;
    min-width: 0;
    max-width: 800px;
}

/* Era sections */
.learn-era {
    margin-bottom: 64px;
    scroll-margin-top: 100px;
}

.learn-era-divider {
    margin: 80px 0 48px;
    padding: var(--spacing-xl) 0;
    border-top: 1px solid rgba(255,255,255,0.08);
}

.learn-era-divider h2 {
    font-size: var(--font-body-md);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 1.5px;
    color: var(--text-tertiary);
    margin: 0;
}

.learn-era-header {
    margin-bottom: var(--spacing-xl);
    padding-bottom: var(--spacing-lg);
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.learn-era-header h2 {
    font-size: var(--font-display-md);
    font-weight: 800;
    letter-spacing: -0.8px;
    line-height: 1.2;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.85) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin: 0 0 var(--spacing-sm);
}

.learn-era-header p {
    color: var(--text-secondary);
    font-size: var(--font-body-sm);
    line-height: 1.6;
    margin: 0;
}

/* Module cards */
.learn-modules {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-xl);
}

.learn-module-card {
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-xl);
    padding: var(--spacing-xl);
    transition: all 0.2s ease;
    scroll-margin-top: 100px;
}

.learn-module-card:hover {
    background: rgba(255,255,255,0.06);
    border-color: rgba(255,255,255,0.12);
}

.learn-module-card.locked {
    opacity: 0.6;
    cursor: not-allowed;
    position: relative;
}

.learn-module-card.locked:hover {
    background: rgba(255,255,255,0.04);
    border-color: rgba(255,255,255,0.08);
}

.learn-module-card.locked::after {
    content: '';
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.2);
    border-radius: var(--radius-xl);
    pointer-events: none;
}

.locked-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 12px;
    background: rgba(251, 146, 60, 0.15);
    border: 1px solid rgba(251, 146, 60, 0.3);
    border-radius: var(--radius-full);
    font-size: var(--font-caption-md);
    font-weight: 600;
    color: #fb923c;
}

.locked-badge-icon {
    font-size: 12px;
}

.learn-module-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--spacing-lg);
    margin-bottom: var(--spacing-lg);
}

.learn-module-info {
    flex: 1;
}

.learn-module-title {
    font-size: var(--font-heading-sm);
    font-weight: 700;
    color: var(--text-primary);
    margin: 0 0 6px;
    display: flex;
    align-items: center;
    gap: 10px;
}

.learn-module-number {
    font-size: var(--font-body-md);
    font-weight: 700;
    color: var(--color-accent-purple);
    opacity: 0.8;
}

.learn-module-desc {
    color: var(--text-secondary);
    font-size: var(--font-body-md);
    line-height: 1.5;
    margin: 0;
}

.learn-module-meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--spacing-sm);
}

.learn-exercise-count {
    font-size: var(--font-caption-md);
    color: var(--text-secondary);
    background: rgba(255,255,255,0.05);
    padding: var(--spacing-xs) 10px;
    border-radius: var(--radius-full);
}

.learn-difficulty {
    display: flex;
    gap: 3px;
}

.learn-difficulty-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255,255,255,0.15);
}

.learn-difficulty-dot.filled {
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
}

/* Preview section */
.learn-module-preview {
    margin-top: var(--spacing-lg);
    padding-top: var(--spacing-lg);
    border-top: 1px solid rgba(255,255,255,0.06);
}

.learn-preview-label {
    font-size: var(--font-caption-sm);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-tertiary);
    margin-bottom: var(--spacing-md);
}

/* Action buttons */
.learn-module-actions {
    display: flex;
    gap: 10px;
    margin-top: var(--spacing-xl);
}

.learn-action-btn {
    padding: 10px 18px;
    border-radius: var(--radius-md);
    font-size: var(--font-body-md);
    font-weight: 600;
    cursor: pointer;
    transition: all 0.18s ease;
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid transparent;
}

.learn-action-btn.primary {
    background: linear-gradient(135deg, rgba(96,165,250,0.9), rgba(167,139,250,0.9));
    color: #060814;
    border-color: rgba(255,255,255,0.1);
}

.learn-action-btn.primary:hover {
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
}

.learn-action-btn.secondary {
    background: rgba(255,255,255,0.06);
    color: var(--text-secondary);
    border-color: rgba(255,255,255,0.12);
}

.learn-action-btn.secondary:hover {
    background: rgba(255,255,255,0.10);
    color: var(--text-primary);
}

/* Expanded module state */
.learn-module-card.expanded {
    background: rgba(255,255,255,0.08);
    border-color: rgba(167,139,250,0.3);
    box-shadow: 0 0 40px rgba(167,139,250,0.08);
}

.learn-module-expanded-content {
    margin-top: var(--spacing-xl);
    padding-top: var(--spacing-xl);
    border-top: 1px solid rgba(255,255,255,0.08);
}

/* Mode selector inline card header */
.mode-selector-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--spacing-xl);
    padding: var(--spacing-lg);
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
}

.mode-selector-tabs {
    display: flex;
    gap: var(--spacing-sm);
}

.mode-tab-btn {
    padding: 8px 16px;
    border-radius: var(--radius-md);
    font-size: var(--font-body-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all 0.18s ease;
    border: 1px solid transparent;
    background: rgba(255,255,255,0.05);
    color: var(--text-secondary);
}

.mode-tab-btn:hover {
    background: rgba(255,255,255,0.08);
    color: var(--text-primary);
}

.mode-tab-btn.active {
    background: linear-gradient(135deg, rgba(96,165,250,0.2), rgba(167,139,250,0.2));
    border-color: rgba(96,165,250,0.3);
    color: var(--color-accent-blue);
}

.mode-tab-btn.test {
    background: rgba(251, 191, 36, 0.15);
    border-color: rgba(251, 191, 36, 0.3);
    color: #fbbf24;
}

.mode-stats {
    display: flex;
    gap: var(--spacing-lg);
}

.mode-stat {
    display: flex;
    align-items: center;
    gap: var(--spacing-xs);
    font-size: var(--font-body-sm);
}

.mode-stat-value {
    font-weight: 700;
    color: var(--color-accent-blue);
}

.mode-stat-value.streak {
    color: #fbbf24;
}

.mode-stat-label {
    color: var(--text-tertiary);
}

.combo-multiplier {
    font-weight: 700;
    color: #4ade80;
    margin-left: 2px;
}

.mode-stat.combo {
    background: rgba(251, 191, 36, 0.1);
    padding: 4px 10px;
    border-radius: var(--radius-md);
    border: 1px solid rgba(251, 191, 36, 0.2);
}

/* Content tab selector */
.content-tabs {
    display: flex;
    gap: var(--spacing-sm);
    margin-bottom: var(--spacing-xl);
    border-bottom: 1px solid rgba(255,255,255,0.08);
    padding-bottom: var(--spacing-md);
}

.content-tab-btn {
    padding: 8px 16px;
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    font-size: var(--font-body-sm);
    font-weight: 600;
    cursor: pointer;
    transition: all 0.18s ease;
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
}

.content-tab-btn:hover {
    color: var(--text-primary);
}

.content-tab-btn.active {
    color: var(--color-accent-blue);
    border-bottom-color: var(--color-accent-blue);
}

/* Practice tab - green accent */
.content-tab-btn.practice {
    color: var(--color-success);
}

.content-tab-btn.practice:hover {
    color: var(--color-success);
    background: rgba(74, 222, 128, 0.08);
}

.content-tab-btn.practice.active {
    color: var(--color-success);
    border-bottom-color: var(--color-success);
    background: rgba(74, 222, 128, 0.1);
}

/* Test tab - orange/yellow accent */
.content-tab-btn.test {
    color: #fbbf24;
}

.content-tab-btn.test:hover {
    color: #fbbf24;
    background: rgba(251, 191, 36, 0.08);
}

.content-tab-btn.test.active {
    color: #fbbf24;
    border-bottom-color: #fbbf24;
    background: rgba(251, 191, 36, 0.1);
}

/* Lesson section styling */
.lesson-section {
    margin-bottom: var(--spacing-xxl);
    padding-bottom: var(--spacing-xl);
    border-bottom: 1px solid rgba(255,255,255,0.06);
}

.lesson-section:last-child {
    border-bottom: none;
}

.lesson-section-title {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: var(--spacing-lg);
    display: flex;
    align-items: center;
    gap: var(--spacing-sm);
}

.lesson-section-number {
    font-size: 1rem;
    color: var(--color-accent-blue);
    font-weight: 600;
}

.lesson-paragraph {
    font-size: 1rem;
    color: rgba(255, 255, 255, 0.85);
    line-height: 1.75;
    margin-bottom: var(--spacing-lg);
}

.lesson-definition {
    background: rgba(167, 139, 250, 0.08);
    border: 1px solid rgba(167, 139, 250, 0.2);
    border-radius: var(--radius-md);
    padding: var(--spacing-lg);
    margin-bottom: var(--spacing-lg);
}

.lesson-definition-term {
    font-size: 1rem;
    font-weight: 700;
    color: var(--color-accent-purple);
    margin-bottom: var(--spacing-xs);
}

.lesson-definition-text {
    font-size: 1rem;
    color: rgba(255, 255, 255, 0.8);
    line-height: 1.65;
}

.lesson-example {
    background: rgba(96, 165, 250, 0.08);
    border: 1px solid rgba(96, 165, 250, 0.2);
    border-radius: var(--radius-md);
    padding: var(--spacing-lg);
    margin-bottom: var(--spacing-lg);
}

.lesson-example-title {
    font-size: 1rem;
    font-weight: 600;
    color: var(--color-accent-blue);
    margin-bottom: var(--spacing-md);
}

.lesson-example-premise {
    font-size: 1rem;
    color: rgba(255, 255, 255, 0.8);
    padding-left: var(--spacing-lg);
    margin-bottom: var(--spacing-xs);
    line-height: 1.6;
}

.lesson-example-conclusion {
    font-size: 1rem;
    color: var(--text-primary);
    font-weight: 500;
    margin-top: var(--spacing-md);
    padding-left: var(--spacing-lg);
}

.lesson-example-note {
    margin-top: var(--spacing-md);
    padding-top: var(--spacing-md);
    border-top: 1px solid rgba(255,255,255,0.08);
    color: rgba(255, 255, 255, 0.5);
    font-size: 0.9rem;
    font-style: italic;
    line-height: 1.5;
}

/* Symbol glossary block */
.lesson-symbols {
    background: rgba(96, 165, 250, 0.08);
    border: 1px solid rgba(96, 165, 250, 0.2);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
    margin: var(--spacing-lg) 0;
}

.lesson-symbols-title {
    font-weight: 700;
    color: #60a5fa;
    margin-bottom: var(--spacing-md);
    font-size: 0.9rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.lesson-symbols-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--spacing-md);
}

.lesson-symbol-item {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-sm);
    padding: var(--spacing-md);
    background: rgba(255,255,255,0.03);
    border-radius: var(--radius-md);
    text-align: center;
}

.lesson-symbol-glyph {
    font-size: 2rem;
    font-family: var(--font-mono);
    color: #60a5fa;
    text-align: center;
}

.lesson-symbol-name {
    font-weight: 600;
    color: var(--text-primary);
    font-size: 0.9rem;
}

.lesson-symbol-meaning {
    color: rgba(255, 255, 255, 0.6);
    font-size: 0.85rem;
}

.lesson-symbol-example {
    color: rgba(255, 255, 255, 0.5);
    font-size: 0.8rem;
    font-style: italic;
    margin-top: 4px;
}

/* Quiz block */
.lesson-quiz {
    background: rgba(167, 139, 250, 0.08);
    border: 1px solid rgba(167, 139, 250, 0.2);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
    margin: var(--spacing-lg) 0;
}

.lesson-quiz-question {
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--spacing-md);
    font-size: 1rem;
}

.lesson-quiz-options {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-sm);
}

.lesson-quiz-option {
    text-align: left;
    padding: var(--spacing-md);
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s ease;
}

.lesson-quiz-option:hover:not(:disabled) {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.2);
    color: var(--text-primary);
}

.lesson-quiz-option:disabled {
    cursor: default;
}

.lesson-quiz-option.correct {
    background: rgba(74, 222, 128, 0.15);
    border-color: var(--color-success);
    color: var(--color-success);
}

.lesson-quiz-option.incorrect {
    background: rgba(248, 113, 113, 0.15);
    border-color: var(--color-error);
    color: var(--color-error);
}

.lesson-quiz-option.answered {
    opacity: 0.5;
}

.lesson-quiz-explanation {
    margin-top: var(--spacing-md);
    padding-top: var(--spacing-md);
    border-top: 1px solid rgba(255,255,255,0.08);
    color: rgba(255, 255, 255, 0.7);
    font-size: 0.9rem;
    display: none;
}

.lesson-quiz-explanation.visible {
    display: block;
}

.learn-module-close {
    position: absolute;
    top: var(--spacing-lg);
    right: var(--spacing-lg);
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: rgba(255,255,255,0.08);
    border: 1px solid rgba(255,255,255,0.12);
    color: var(--text-secondary);
    font-size: 18px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
}

.learn-module-close:hover {
    background: rgba(255,255,255,0.15);
    color: var(--text-primary);
}

/* Tab content panels */
.tab-panel {
    padding: var(--spacing-xl) 0;
}

.tab-panel-lesson {
    color: var(--text-primary);
    line-height: 1.7;
}

.tab-panel-lesson h3 {
    font-size: var(--font-heading-sm);
    font-weight: 700;
    margin: var(--spacing-xl) 0 var(--spacing-md);
    color: var(--text-primary);
}

.tab-panel-lesson p {
    margin-bottom: var(--spacing-lg);
    color: var(--text-secondary);
}

/* Examples panel */
.tab-panel-examples {
    padding: var(--spacing-lg) 0;
}

.examples-intro {
    margin-bottom: var(--spacing-xl);
}

.examples-intro h3 {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: var(--spacing-sm);
}

.examples-intro p {
    font-size: 1rem;
    color: rgba(255, 255, 255, 0.7);
    line-height: 1.6;
}

.examples-list {
    display: flex;
    flex-direction: column;
    gap: var(--spacing-xl);
}

.example-card {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-lg);
    padding: var(--spacing-lg);
}

.example-card .example-sentence {
    font-size: 1rem;
    font-weight: 500;
    color: var(--text-primary);
    margin-bottom: var(--spacing-md);
    padding-bottom: var(--spacing-md);
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.exercise-card {
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: var(--radius-lg);
    padding: var(--spacing-xl);
    margin-bottom: var(--spacing-lg);
}

.exercise-sentence {
    font-size: 1.15rem;
    font-weight: 500;
    color: var(--text-primary);
    margin-bottom: var(--spacing-lg);
    line-height: 1.5;
}

.exercise-input-row {
    display: flex;
    gap: var(--spacing-md);
}

.exercise-input {
    flex: 1;
    padding: var(--spacing-md) var(--spacing-lg);
    font-size: var(--font-body-md);
    font-family: var(--font-mono);
    background: rgba(255,255,255,0.06);
    border: 2px solid rgba(255,255,255,0.12);
    border-radius: var(--radius-md);
    color: var(--text-primary);
    outline: none;
    transition: border-color 0.2s ease;
}

.exercise-input:focus {
    border-color: var(--color-accent-blue);
}

.exercise-input.correct {
    border-color: var(--color-success);
    background: rgba(74, 222, 128, 0.1);
}

.exercise-input.incorrect {
    border-color: var(--color-error);
    background: rgba(248, 113, 113, 0.1);
}

.exercise-submit-btn {
    padding: var(--spacing-md) var(--spacing-xl);
    background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
    border: none;
    border-radius: var(--radius-md);
    color: #060814;
    font-weight: 600;
    cursor: pointer;
    transition: opacity 0.15s ease;
}

.exercise-submit-btn:hover {
    opacity: 0.9;
}

.exercise-feedback {
    margin-top: var(--spacing-lg);
    padding: var(--spacing-md);
    border-radius: var(--radius-md);
}

.exercise-feedback.correct {
    background: rgba(74, 222, 128, 0.15);
    border: 1px solid rgba(74, 222, 128, 0.3);
    color: var(--color-success);
}

.exercise-feedback.incorrect {
    background: rgba(248, 113, 113, 0.15);
    border: 1px solid rgba(248, 113, 113, 0.3);
    color: var(--color-error);
}

.logic-output {
    font-family: var(--font-mono);
    font-size: var(--font-body-lg);
    padding: var(--spacing-lg);
    background: rgba(96, 165, 250, 0.1);
    border: 1px solid rgba(96, 165, 250, 0.2);
    border-radius: var(--radius-md);
    color: var(--color-accent-blue);
    margin: var(--spacing-lg) 0;
}

/* Responsive */
@media (max-width: 1024px) {
    .learn-layout {
        flex-direction: column;
    }

    .learn-hero h1 {
        font-size: var(--font-display-md);
    }

    .learn-hero {
        padding: 40px var(--spacing-xl) var(--spacing-xxl);
    }
}

@media (max-width: 640px) {
    .learn-hero h1 {
        font-size: var(--font-heading-lg);
    }

    .learn-hero p {
        font-size: var(--font-body-md);
    }

    .learn-era-header h2 {
        font-size: var(--font-heading-lg);
    }

    .learn-module-header {
        flex-direction: column;
    }

    .learn-module-meta {
        flex-direction: row;
        align-items: center;
    }

    .learn-module-actions {
        flex-direction: column;
    }

    .learn-action-btn {
        justify-content: center;
    }
}
"#;

/// Module data with preview example
struct ModuleData {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    exercise_count: u32,
    difficulty: u8,
    preview_code: Option<&'static str>,
}

/// Era data structure
struct EraData {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    modules: Vec<ModuleData>,
}

fn get_curriculum_data() -> Vec<EraData> {
    vec![
        // Era 1: First Steps
        EraData {
            id: "first-steps",
            title: "First Steps",
            description: "Get comfortable with logic. Learn what arguments are, how to spot good reasoning, and the classical foundations.",
            modules: vec![
                ModuleData {
                    id: "introduction",
                    title: "Introduction",
                    description: "Learn foundational concepts: what logic is, valid vs. invalid arguments, and sound reasoning.",
                    exercise_count: 5,
                    difficulty: 1,
                    preview_code: Some("All humans are mortal. Socrates is human. Therefore..."),
                },
                ModuleData {
                    id: "syllogistic",
                    title: "Syllogistic Logic",
                    description: "Translate English into syllogistic notation. Master the classical form of logical reasoning.",
                    exercise_count: 98,
                    difficulty: 1,
                    preview_code: Some("All humans are mortal."),
                },
                ModuleData {
                    id: "definitions",
                    title: "Meaning and Definitions",
                    description: "Understand uses of language, types of definitions, and the analytic/synthetic distinction.",
                    exercise_count: 48,
                    difficulty: 2,
                    preview_code: None,
                },
                ModuleData {
                    id: "fallacies",
                    title: "Fallacies and Argumentation",
                    description: "Identify good arguments vs. fallacious reasoning. Master informal fallacies.",
                    exercise_count: 16,
                    difficulty: 2,
                    preview_code: None,
                },
                ModuleData {
                    id: "inductive",
                    title: "Inductive Reasoning",
                    description: "Master probability, analogical reasoning, Mill's methods, and inference to best explanation.",
                    exercise_count: 12,
                    difficulty: 2,
                    preview_code: Some("90% of observed swans are white..."),
                },
            ],
        },
        // Era 2: Building Blocks
        EraData {
            id: "building-blocks",
            title: "Building Blocks",
            description: "Master the core of formal logic. Propositional connectives, truth tables, and proof construction.",
            modules: vec![
                ModuleData {
                    id: "propositional",
                    title: "Basic Propositional Logic",
                    description: "Master AND, OR, NOT, and IF-THEN connectives. Truth tables, S-rules, and I-rules.",
                    exercise_count: 114,
                    difficulty: 2,
                    preview_code: Some("If John runs, then Mary walks."),
                },
                ModuleData {
                    id: "proofs",
                    title: "Propositional Proofs",
                    description: "Construct formal proofs and refutations. Learn natural deduction and truth trees.",
                    exercise_count: 14,
                    difficulty: 3,
                    preview_code: Some("1. P → Q  2. P  ∴ Q"),
                },
            ],
        },
        // Era 3: Expanding Horizons
        EraData {
            id: "expanding-horizons",
            title: "Expanding Horizons",
            description: "Explore richer logical systems. Quantifiers, modality, obligations, and beliefs.",
            modules: vec![
                ModuleData {
                    id: "quantificational",
                    title: "Basic Quantificational Logic",
                    description: "Master universal and existential quantifiers. Translations, proofs, and refutations.",
                    exercise_count: 12,
                    difficulty: 3,
                    preview_code: Some("All birds fly."),
                },
                ModuleData {
                    id: "relations",
                    title: "Relations and Identity",
                    description: "Extend predicate logic with identity and relations. Handle definite descriptions.",
                    exercise_count: 8,
                    difficulty: 3,
                    preview_code: Some("John loves Mary."),
                },
                ModuleData {
                    id: "modal",
                    title: "Basic Modal Logic",
                    description: "Explore possibility and necessity operators. Express what could be or must be true.",
                    exercise_count: 36,
                    difficulty: 3,
                    preview_code: Some("It is possible that John runs."),
                },
                ModuleData {
                    id: "further_modal",
                    title: "Further Modal Systems",
                    description: "Advanced modal systems including quantified modal logic and temporal operators.",
                    exercise_count: 2,
                    difficulty: 4,
                    preview_code: Some("John will run tomorrow."),
                },
                ModuleData {
                    id: "deontic",
                    title: "Deontic and Imperative Logic",
                    description: "Reason about obligation, permission, and prohibition. The logic of ethics and law.",
                    exercise_count: 38,
                    difficulty: 3,
                    preview_code: Some("John ought to leave."),
                },
                ModuleData {
                    id: "belief",
                    title: "Belief Logic",
                    description: "Express beliefs, knowledge, willing, and rationality. Model propositional attitudes.",
                    exercise_count: 15,
                    difficulty: 3,
                    preview_code: Some("John believes that Mary runs."),
                },
            ],
        },
        // Era 4: Mastery
        EraData {
            id: "mastery",
            title: "Mastery",
            description: "Deep understanding. The philosophy, history, and frontiers of logical thought.",
            modules: vec![
                ModuleData {
                    id: "ethics",
                    title: "A Formalized Ethical Theory",
                    description: "Apply logic to ethics: practical reason, consistency, and the golden rule formalized.",
                    exercise_count: 8,
                    difficulty: 4,
                    preview_code: None,
                },
                ModuleData {
                    id: "metalogic",
                    title: "Metalogic",
                    description: "Study logic about logic: soundness, completeness, and Gödel's incompleteness theorem.",
                    exercise_count: 6,
                    difficulty: 4,
                    preview_code: None,
                },
                ModuleData {
                    id: "history",
                    title: "History of Logic",
                    description: "Trace logic from Aristotle through Frege, Russell, and modern developments.",
                    exercise_count: 8,
                    difficulty: 2,
                    preview_code: None,
                },
                ModuleData {
                    id: "deviant",
                    title: "Deviant Logics",
                    description: "Explore non-classical logics: many-valued, paraconsistent, intuitionist, and relevance logic.",
                    exercise_count: 8,
                    difficulty: 4,
                    preview_code: None,
                },
                ModuleData {
                    id: "philosophy",
                    title: "Philosophy of Logic",
                    description: "Examine philosophical foundations: abstract entities, truth, paradoxes, and logic's scope.",
                    exercise_count: 8,
                    difficulty: 4,
                    preview_code: None,
                },
            ],
        },
    ]
}

/// Expanded module key: (era_id, module_id)
type ExpandedModuleKey = Option<(String, String)>;

#[component]
pub fn Learn() -> Element {
    let mut active_module = use_signal(|| None::<String>);
    // Expanded module state: which module is currently expanded inline
    let mut expanded_module = use_signal::<ExpandedModuleKey>(|| None);

    let eras = get_curriculum_data();

    // For module unlock checking
    let content_engine = ContentEngine::new();
    let user_progress = UserProgress::new(); // TODO: Load from storage

    // Build module info for sidebar
    let sidebar_modules: Vec<ModuleInfo> = eras.iter().flat_map(|era| {
        era.modules.iter().map(|m| ModuleInfo {
            era_id: era.id.to_string(),
            era_title: era.title.to_string(),
            module_id: m.id.to_string(),
            module_title: m.title.to_string(),
            exercise_count: m.exercise_count,
            difficulty: m.difficulty,
        })
    }).collect();

    // Collect all module IDs for intersection observer (used in wasm32 target)
    #[allow(unused_variables)]
    let module_ids: Vec<String> = eras.iter()
        .flat_map(|era| era.modules.iter().map(|m| m.id.to_string()))
        .collect();

    // Set up scroll tracking with IntersectionObserver
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let module_ids_for_effect = module_ids.clone();

        use_effect(move || {
            let window = match web_sys::window() {
                Some(w) => w,
                None => return,
            };
            let document = match window.document() {
                Some(d) => d,
                None => return,
            };

            // Create a closure that will be called when elements intersect
            // Use RefCell to allow mutation from within Fn closure
            use std::cell::RefCell;
            use std::rc::Rc;

            let active_module_clone = Rc::new(RefCell::new(active_module.clone()));
            let active_module_for_closure = active_module_clone.clone();

            let callback = Closure::<dyn Fn(js_sys::Array, web_sys::IntersectionObserver)>::new(
                move |entries: js_sys::Array, _observer: web_sys::IntersectionObserver| {
                    // Simple approach: when a module crosses the threshold line (enters from below),
                    // it becomes active. The threshold is set so modules activate when their top
                    // reaches ~100px from the top of the viewport.
                    for i in 0..entries.length() {
                        if let Ok(entry) = entries.get(i).dyn_into::<web_sys::IntersectionObserverEntry>() {
                            // Only activate when element is entering (crossing the threshold)
                            if entry.is_intersecting() {
                                let target = entry.target();
                                let id = target.id();
                                if !id.is_empty() {
                                    active_module_for_closure.borrow_mut().set(Some(id));
                                }
                            }
                        }
                    }
                },
            );

            // Create IntersectionObserver options
            let mut options = web_sys::IntersectionObserverInit::new();
            // Root margin: top offset of -100px means the "viewport" starts 100px below the actual top
            // Bottom margin of -90% means only the top 10% of viewport triggers intersection
            // This creates a thin "tripwire" near the top of the screen
            options.root_margin("-100px 0px -90% 0px");
            // Single threshold at 0 - fires once when element crosses the line
            let thresholds = js_sys::Array::new();
            thresholds.push(&JsValue::from(0.0));
            options.threshold(&thresholds);

            // Create the observer
            let observer = match web_sys::IntersectionObserver::new_with_options(
                callback.as_ref().unchecked_ref(),
                &options,
            ) {
                Ok(obs) => obs,
                Err(_) => return,
            };

            // Observe all module cards
            for module_id in &module_ids_for_effect {
                if let Some(element) = document.get_element_by_id(module_id) {
                    observer.observe(&element);
                }
            }

            // Keep callback alive
            callback.forget();
        });
    }

    // Track first era for divider logic
    let mut is_first_era = true;

    rsx! {
        style { "{LEARN_STYLE}" }

        div { class: "learn-page",
            MainNav { active: ActivePage::Learn }

            // Hero
            header { class: "learn-hero",
                div { class: "learn-hero-badge",
                    div { class: "dot" }
                    span { "Interactive Curriculum" }
                }
                h1 { "Learn Logic" }
                p {
                    "Master first-order logic through progressive challenges. Start with the basics and work your way up to advanced reasoning."
                }
            }

            // Main layout
            div { class: "learn-layout",
                // Sidebar
                LearnSidebar {
                    modules: sidebar_modules,
                    active_module: active_module.read().clone(),
                    on_module_click: move |(_era_id, module_id): (String, String)| {
                        active_module.set(Some(module_id));
                    },
                }

                // Content
                main { class: "learn-content",
                    for era in eras.iter() {
                        {
                            // Show divider for all eras except the first
                            let show_divider = !is_first_era;
                            is_first_era = false;

                            rsx! {
                                // Era divider
                                if show_divider {
                                    div { class: "learn-era-divider",
                                        h2 { "{era.title}" }
                                    }
                                }

                                // Era section
                                section {
                                    class: "learn-era",
                                    div { class: "learn-era-header",
                                        h2 { "{era.title}" }
                                        p { "{era.description}" }
                                    }

                                    div { class: "learn-modules",
                                        for (idx, module) in era.modules.iter().enumerate() {
                                            {
                                                let era_id = era.id.to_string();
                                                let module_id = module.id.to_string();
                                                let module_number = idx + 1;

                                                // Check if this module is locked
                                                let is_unlocked = check_module_unlocked(&user_progress, &content_engine, &era_id, &module_id);

                                                // Check if this module is expanded
                                                let is_expanded = expanded_module.read().as_ref()
                                                    .map(|(e, m)| e == era.id && m == module.id)
                                                    .unwrap_or(false);

                                                let card_class = match (is_expanded, is_unlocked) {
                                                    (true, _) => "learn-module-card expanded",
                                                    (false, true) => "learn-module-card",
                                                    (false, false) => "learn-module-card locked",
                                                };

                                                rsx! {
                                                    div {
                                                        class: "{card_class}",
                                                        id: "{module.id}",
                                                        style: if is_expanded { "position: relative;" } else { "" },

                                                        // Close button when expanded
                                                        if is_expanded {
                                                            {
                                                                let module_id_for_scroll = module_id.clone();
                                                                rsx! {
                                                                    button {
                                                                        class: "learn-module-close",
                                                                        onclick: move |_| {
                                                                            expanded_module.set(None);
                                                                            // Scroll to the module card after collapse
                                                                            #[cfg(target_arch = "wasm32")]
                                                                            {
                                                                                let id = module_id_for_scroll.clone();
                                                                                wasm_bindgen_futures::spawn_local(async move {
                                                                                    // Delay to let the DOM update after collapse
                                                                                    gloo_timers::future::TimeoutFuture::new(150).await;
                                                                                    if let Some(window) = web_sys::window() {
                                                                                        if let Some(document) = window.document() {
                                                                                            if let Some(element) = document.get_element_by_id(&id) {
                                                                                                // Get element position and scroll with offset for nav
                                                                                                let rect = element.get_bounding_client_rect();
                                                                                                let scroll_y = window.scroll_y().unwrap_or(0.0);
                                                                                                let target_y = scroll_y + rect.top() - 100.0; // 100px offset for nav
                                                                                                let _ = window.scroll_to_with_scroll_to_options(
                                                                                                    web_sys::ScrollToOptions::new()
                                                                                                        .top(target_y)
                                                                                                        .behavior(web_sys::ScrollBehavior::Smooth)
                                                                                                );
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                });
                                                                            }
                                                                        },
                                                                        "×"
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        div { class: "learn-module-header",
                                                            div { class: "learn-module-info",
                                                                h3 { class: "learn-module-title",
                                                                    span { class: "learn-module-number", "{module_number}." }
                                                                    "{module.title}"
                                                                }
                                                                p { class: "learn-module-desc", "{module.description}" }
                                                            }

                                                            div { class: "learn-module-meta",
                                                                span { class: "learn-exercise-count", "{module.exercise_count} exercises" }
                                                                div { class: "learn-difficulty",
                                                                    for i in 1..=5u8 {
                                                                        div {
                                                                            class: if i <= module.difficulty { "learn-difficulty-dot filled" } else { "learn-difficulty-dot" }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        // Show preview only when collapsed
                                                        if !is_expanded {
                                                            if is_unlocked {
                                                                if let Some(preview) = module.preview_code {
                                                                    div { class: "learn-module-preview",
                                                                        div { class: "learn-preview-label", "Try an Example" }
                                                                        GuideCodeBlock {
                                                                            id: format!("preview-{}", module.id),
                                                                            label: "Example".to_string(),
                                                                            mode: ExampleMode::Logic,
                                                                            initial_code: preview.to_string(),
                                                                        }
                                                                    }
                                                                }

                                                                // Action buttons (only when collapsed and unlocked)
                                                                div { class: "learn-module-actions",
                                                                    button {
                                                                        class: "learn-action-btn primary",
                                                                        onclick: {
                                                                            let era = era_id.clone();
                                                                            let module = module_id.clone();
                                                                            move |_| {
                                                                                expanded_module.set(Some((era.clone(), module.clone())));
                                                                            }
                                                                        },
                                                                        "Start Learning"
                                                                    }
                                                                }
                                                            } else {
                                                                // Locked module - show lock badge
                                                                div { class: "learn-module-actions",
                                                                    div { class: "locked-badge",
                                                                        span { class: "locked-badge-icon", "\u{1F512}" }
                                                                        "Complete previous module to unlock"
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        // Expanded content - Interactive exercises
                                                        if is_expanded {
                                                            div { class: "learn-module-expanded-content",
                                                                InteractiveExercisePanel {
                                                                    era_id: era_id.clone(),
                                                                    module_id: module_id.clone(),
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

            // Floating vocab reference panel
            VocabReference {}
        }
    }
}

/// What is currently revealed in the exercise - each section can be shown independently
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct RevealState {
    hint: bool,
    answer: bool,
    symbol_dictionary: bool,
}

impl RevealState {
    fn reset(&mut self) {
        self.hint = false;
        self.answer = false;
        self.symbol_dictionary = false;
    }
}

/// Practice mode state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum PracticeMode {
    #[default]
    Practice,
    Test,
}

/// Content view tab state - corresponds to the 4 tabs
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum ContentView {
    #[default]
    Lesson,
    Examples,
    Practice,
    Test,
}

/// Interactive exercise panel with reveal buttons instead of tabs
#[component]
fn InteractiveExercisePanel(era_id: String, module_id: String) -> Element {
    use crate::content::{ContentBlock, Section};

    let engine = ContentEngine::new();
    let generator = Generator::new();

    // Content view state (Lesson vs Practice)
    let mut content_view = use_signal(ContentView::default);

    // Exercise state
    let mut current_exercise_idx = use_signal(|| 0usize);
    let mut user_answer = use_signal(|| String::new());
    let mut reveal_state = use_signal(RevealState::default);
    let mut feedback = use_signal(|| None::<(bool, String)>);
    let mut score = use_signal(|| 0u32);
    let mut streak = use_signal(|| 0u32);
    let mut correct_count = use_signal(|| 0u32);
    let mut practice_mode = use_signal(PracticeMode::default);
    // Track wrong attempts per exercise - each wrong costs 5 XP, max 10 XP available per exercise
    // After 2 wrong attempts, no XP can be earned from that exercise
    let mut exercise_attempts = use_signal(|| std::collections::HashMap::<usize, u32>::new());
    // Track which exercises have been completed (earned XP) - prevents double XP
    let mut completed_exercises = use_signal(|| std::collections::HashSet::<usize>::new());
    // Track which exercises have had their answer revealed (forfeits XP)
    let mut answer_revealed_exercises = use_signal(|| std::collections::HashSet::<usize>::new());

    // Priority queue for wrong answers - re-queue at position +3
    // When wrong, insert exercise index to be shown again after 3 more exercises
    let mut retry_queue = use_signal(|| std::collections::VecDeque::<usize>::new());
    // Count exercises since last retry to trigger queue pop after 3
    let mut exercises_since_retry = use_signal(|| 0usize);

    // Test mode state
    let mut test_question = use_signal(|| 0usize);
    let mut test_answers = use_signal(|| Vec::<bool>::new());
    let mut test_complete = use_signal(|| false);

    // Struggle detection state
    let mut struggle_detector = use_signal(StruggleDetector::default);
    let mut show_socratic_hint = use_signal(|| false);

    // Lesson quiz state - tracks which quizzes have been answered and their result
    // Key: quiz question string, Value: (selected_index, is_correct)
    let mut quiz_answers = use_signal(|| std::collections::HashMap::<String, (usize, bool)>::new());

    // Stable seed per exercise - only set once when component mounts or exercise changes
    // Use a signal to store the base seed so it doesn't change on re-renders
    let base_seed = use_signal(|| {
        #[cfg(target_arch = "wasm32")]
        { js_sys::Date::now() as u64 }
        #[cfg(not(target_arch = "wasm32"))]
        { 42u64 }
    });

    // Generate challenge from exercise using stable seed
    let module_opt = engine.get_module(&era_id, &module_id);
    let current_challenge: Option<Challenge> = module_opt.as_ref().and_then(|module| {
        let idx = *current_exercise_idx.read();
        let seed = *base_seed.read();
        module.exercises.get(idx).and_then(|ex| {
            // Use exercise index as part of seed to get different sentences per exercise
            // but stable within the same exercise
            let mut rng = StdRng::seed_from_u64(seed.wrapping_add((idx * 1000) as u64));
            generator.generate(ex, &mut rng)
        })
    });

    let total_exercises = module_opt.as_ref().map(|m| m.exercises.len()).unwrap_or(0);
    let current_idx = *current_exercise_idx.read();
    let progress_pct = if total_exercises > 0 {
        ((current_idx + 1) * 100) / total_exercises
    } else {
        0
    };

    // Get the golden answer for validation
    let golden_answer = current_challenge.as_ref().and_then(|ch| {
        match &ch.answer {
            AnswerType::FreeForm { golden_logic } => Some(golden_logic.clone()),
            AnswerType::MultipleChoice { options, correct_index } => {
                options.get(*correct_index).cloned()
            }
            AnswerType::Ambiguity { readings } => readings.first().cloned(),
        }
    });

    // Get hint from exercise
    let hint_text = current_challenge.as_ref().and_then(|ch| ch.hint.clone());

    // Test mode constants
    let test_total = 10usize;
    let is_test_mode = *practice_mode.read() == PracticeMode::Test;
    let current_test_q = *test_question.read();

    // Get sections for lesson content
    let sections: Vec<Section> = module_opt.as_ref()
        .map(|m| m.sections.clone())
        .unwrap_or_default();
    let has_sections = !sections.is_empty();
    let current_view = *content_view.read();

    rsx! {
        div { class: "interactive-exercise-panel",
            // Content tabs (Lesson | Examples | Practice | Test)
            div { class: "content-tabs",
                button {
                    class: if current_view == ContentView::Lesson { "content-tab-btn active" } else { "content-tab-btn" },
                    onclick: move |_| content_view.set(ContentView::Lesson),
                    "Lesson"
                }
                button {
                    class: if current_view == ContentView::Examples { "content-tab-btn active" } else { "content-tab-btn" },
                    onclick: move |_| content_view.set(ContentView::Examples),
                    "Examples"
                }
                button {
                    class: if current_view == ContentView::Practice { "content-tab-btn practice active" } else { "content-tab-btn practice" },
                    onclick: move |_| {
                        content_view.set(ContentView::Practice);
                        practice_mode.set(PracticeMode::Practice);
                    },
                    "Practice"
                }
                button {
                    class: if current_view == ContentView::Test { "content-tab-btn test active" } else { "content-tab-btn test" },
                    onclick: move |_| {
                        content_view.set(ContentView::Test);
                        practice_mode.set(PracticeMode::Test);
                        test_question.set(0);
                        test_answers.set(Vec::new());
                        test_complete.set(false);
                        user_answer.set(String::new());
                        feedback.set(None);
                    },
                    "Test"
                }
            }

            // Lesson content view
            if current_view == ContentView::Lesson {
                div { class: "tab-panel-lesson",
                    if has_sections {
                        for section in sections.iter() {
                            div { class: "lesson-section",
                                h3 { class: "lesson-section-title",
                                    span { class: "lesson-section-number", "{section.id}" }
                                    "{section.title}"
                                }

                                // Render content blocks
                                for block in section.content.iter() {
                                    match block {
                                        ContentBlock::Paragraph { text } => rsx! {
                                            p { class: "lesson-paragraph",
                                                dangerous_inner_html: text.replace("**", "<strong>").replace("**", "</strong>").replace("*", "<em>").replace("*", "</em>")
                                            }
                                        },
                                        ContentBlock::Definition { term, definition } => rsx! {
                                            div { class: "lesson-definition",
                                                div { class: "lesson-definition-term", "{term}" }
                                                div { class: "lesson-definition-text", "{definition}" }
                                            }
                                        },
                                        ContentBlock::Example { title, premises, conclusion, note } => rsx! {
                                            div { class: "lesson-example",
                                                div { class: "lesson-example-title", "{title}" }
                                                for premise in premises.iter() {
                                                    div { class: "lesson-example-premise", "• {premise}" }
                                                }
                                                if let Some(concl) = conclusion {
                                                    div { class: "lesson-example-conclusion", "{concl}" }
                                                }
                                                if let Some(n) = note {
                                                    div { class: "lesson-example-note", "{n}" }
                                                }
                                            }
                                        },
                                        ContentBlock::Symbols { title, symbols } => rsx! {
                                            div { class: "lesson-symbols",
                                                div { class: "lesson-symbols-title", "{title}" }
                                                div { class: "lesson-symbols-grid",
                                                    for sym in symbols.iter() {
                                                        div { class: "lesson-symbol-item",
                                                            span { class: "lesson-symbol-glyph", "{sym.symbol}" }
                                                            div { class: "lesson-symbol-info",
                                                                div { class: "lesson-symbol-name", "{sym.name}" }
                                                                div { class: "lesson-symbol-meaning", "{sym.meaning}" }
                                                                if let Some(example) = &sym.example {
                                                                    div { class: "lesson-symbol-example", "Example: {example}" }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                        ContentBlock::Quiz { question, options, correct, explanation } => {
                                            let q_key = question.clone();
                                            let answered = quiz_answers.read().get(&q_key).cloned();
                                            let correct_idx = *correct;
                                            rsx! {
                                                div { class: "lesson-quiz",
                                                    div { class: "lesson-quiz-question", "{question}" }
                                                    div { class: "lesson-quiz-options",
                                                        for (i, opt) in options.iter().enumerate() {
                                                            {
                                                                let q_key_click = q_key.clone();
                                                                let opt_class = match &answered {
                                                                    Some((selected, _)) if *selected == i && i == correct_idx => "lesson-quiz-option correct",
                                                                    Some((selected, _)) if *selected == i => "lesson-quiz-option incorrect",
                                                                    Some(_) if i == correct_idx => "lesson-quiz-option correct",
                                                                    Some(_) => "lesson-quiz-option answered",
                                                                    None => "lesson-quiz-option",
                                                                };
                                                                rsx! {
                                                                    button {
                                                                        class: "{opt_class}",
                                                                        disabled: answered.is_some(),
                                                                        onclick: move |_| {
                                                                            let is_correct = i == correct_idx;
                                                                            quiz_answers.write().insert(q_key_click.clone(), (i, is_correct));
                                                                        },
                                                                        "{opt}"
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    if answered.is_some() {
                                                        if let Some(expl) = explanation {
                                                            div { class: "lesson-quiz-explanation visible", "{expl}" }
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                    }
                                }

                            }
                        }

                        // Start practicing button
                        div { class: "learn-module-actions", style: "justify-content: center; margin-top: var(--spacing-xl);",
                            button {
                                class: "learn-action-btn primary",
                                onclick: move |_| content_view.set(ContentView::Examples),
                                "Try Examples →"
                            }
                        }
                    } else {
                        p { style: "color: var(--text-tertiary); text-align: center; padding: var(--spacing-xl);",
                            "Lesson content coming soon. Click Examples to see interactive demos."
                        }
                    }
                }
            } else if current_view == ContentView::Examples {
                // Examples view - Interactive code execution
                div { class: "tab-panel-examples",
                    div { class: "examples-intro",
                        h3 { "Interactive Examples" }
                        p { "Try translating sentences and see how logic notation works. The symbol dictionary will explain each symbol used." }
                    }

                    // Example sentences to try
                    div { class: "examples-list",
                        {
                            let examples: Vec<(&str, &str)> = vec![
                                ("ex1", "All cats are mammals."),
                                ("ex2", "Socrates is mortal."),
                                ("ex3", "If it rains, the ground is wet."),
                                ("ex4", "Some birds can fly."),
                                ("ex5", "No reptiles are mammals."),
                            ];

                            rsx! {
                                for (id, sentence) in examples {
                                    div { class: "example-card",
                                        key: "{id}",
                                        div { class: "example-sentence", "{sentence}" }
                                        GuideCodeBlock {
                                            id: id.to_string(),
                                            label: "Try it".to_string(),
                                            mode: ExampleMode::Logic,
                                            initial_code: sentence.to_string(),
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Navigation to Practice
                    div { class: "learn-module-actions", style: "justify-content: center; margin-top: var(--spacing-xl);",
                        button {
                            class: "learn-action-btn primary",
                            onclick: move |_| {
                                content_view.set(ContentView::Practice);
                                practice_mode.set(PracticeMode::Practice);
                            },
                            "Start Practice →"
                        }
                    }
                }
            } else {
                // Practice/Test view
                // Stats header
                div { class: "mode-stats",
                    div { class: "mode-stat",
                        span { class: "mode-stat-value", "{score}" }
                        span { class: "mode-stat-label", " XP" }
                    }
                    if *streak.read() > 0 {
                        {
                            let s = *streak.read();
                            let mult = match s {
                                0 => "1x",
                                1 => "1.25x",
                                2 => "1.5x",
                                3 => "1.75x",
                                _ => "2x",
                            };
                            rsx! {
                                div { class: "mode-stat combo",
                                    span { class: "mode-stat-value streak", "{s}" }
                                    span { class: "mode-stat-label", " streak " }
                                    span { class: "combo-multiplier", "({mult})" }
                                }
                            }
                        }
                    }
                    div { class: "mode-stat",
                        span { class: "mode-stat-value", "{correct_count}" }
                        span { class: "mode-stat-label", " correct" }
                    }
                }

            // Test mode header
            if is_test_mode {
                if *test_complete.read() {
                    // Test results
                    div { class: "test-results",
                        style: "text-align: center; padding: var(--spacing-xxl);",
                        h3 { style: "margin-bottom: var(--spacing-lg); font-size: var(--font-heading-lg);",
                            "Test Complete!"
                        }
                        {
                            let answers = test_answers.read();
                            let correct = answers.iter().filter(|&&c| c).count();
                            let pct = (correct * 100) / test_total;
                            let grade = if pct >= 90 { "A" } else if pct >= 80 { "B" } else if pct >= 70 { "C" } else if pct >= 60 { "D" } else { "F" };
                            rsx! {
                                div { style: "font-size: var(--font-display-md); font-weight: 700; color: var(--color-accent-blue);",
                                    "{correct}/{test_total}"
                                }
                                div { style: "font-size: var(--font-heading-lg); color: var(--text-secondary); margin: var(--spacing-md) 0;",
                                    "Grade: {grade} ({pct}%)"
                                }
                            }
                        }
                        div { class: "learn-module-actions", style: "justify-content: center; margin-top: var(--spacing-xl);",
                            button {
                                class: "learn-action-btn secondary",
                                onclick: move |_| {
                                    practice_mode.set(PracticeMode::Practice);
                                    test_complete.set(false);
                                },
                                "Back to Practice"
                            }
                            button {
                                class: "learn-action-btn primary",
                                onclick: move |_| {
                                    test_question.set(0);
                                    test_answers.set(Vec::new());
                                    test_complete.set(false);
                                    user_answer.set(String::new());
                                    feedback.set(None);
                                },
                                "Retake Test"
                            }
                        }
                    }
                } else {
                    // Test mode progress
                    div { class: "exercise-progress",
                        div { class: "exercise-mode-badge test", "TEST MODE" }
                        span { "Question {current_test_q + 1} of {test_total}" }
                        div { class: "progress-bar",
                            div {
                                class: "progress-fill",
                                style: "width: {((current_test_q + 1) * 100) / test_total}%",
                            }
                        }
                    }
                }
            } else {
                // Practice mode progress
                div { class: "exercise-progress",
                    span { "Exercise {current_idx + 1} of {total_exercises}" }
                    div { class: "progress-bar",
                        div {
                            class: "progress-fill",
                            style: "width: {progress_pct}%",
                        }
                    }
                    span { class: "practice-score", "+{score} XP" }
                    if *correct_count.read() > 0 {
                        span { style: "color: var(--color-success); font-size: var(--font-caption-md);",
                            " ({correct_count} correct)"
                        }
                    }
                }
            }

            // Don't show exercise card when test is complete
            if !(is_test_mode && *test_complete.read()) {
            if let Some(challenge) = current_challenge.as_ref() {
                div { class: "exercise-card",
                    div { class: "exercise-sentence", "{challenge.sentence}" }

                    // Answer input based on exercise type
                    match &challenge.answer {
                        AnswerType::FreeForm { .. } => rsx! {
                            div { class: "exercise-input-row",
                                input {
                                    class: match feedback.read().as_ref() {
                                        Some((true, _)) => "exercise-input correct",
                                        Some((false, _)) => "exercise-input incorrect",
                                        None => "exercise-input",
                                    },
                                    r#type: "text",
                                    placeholder: "Enter your logic translation... (Enter to submit)",
                                    value: "{user_answer}",
                                    oninput: {
                                        move |e: Event<FormData>| {
                                            user_answer.set(e.value());
                                            // Record activity to reset inactivity timer
                                            struggle_detector.write().record_activity();
                                        }
                                    },
                                    onkeydown: {
                                        let golden = golden_answer.clone();
                                        move |e: Event<KeyboardData>| {
                                            if e.key() == Key::Enter {
                                                e.prevent_default();
                                                // Trigger submit logic - same as Check button
                                                let answer = user_answer.read().clone();
                                                if !answer.is_empty() {
                                                    if let Some(ref expected) = golden {
                                                        let result = check_answer(&answer, expected);
                                                        if result.correct {
                                                            // Handle correct answer
                                                            let answer_was_revealed = answer_revealed_exercises.read().contains(&current_idx);
                                                            let already_completed = if is_test_mode { false } else { completed_exercises.read().contains(&current_idx) };

                                                            if answer_was_revealed {
                                                                let cc = *correct_count.read();
                                                                correct_count.set(cc + 1);
                                                                completed_exercises.write().insert(current_idx);
                                                                feedback.set(Some((true, "Correct! (no XP - answer was revealed)".to_string())));
                                                            } else if !already_completed {
                                                                let wrong_count = *exercise_attempts.read().get(&current_idx).unwrap_or(&0);
                                                                let base_xp = 10u32.saturating_sub(wrong_count * 5);
                                                                if base_xp > 0 {
                                                                    let cs = *streak.read();
                                                                    let sc = *score.read();
                                                                    let cc = *correct_count.read();
                                                                    let multiplier = match cs { 0 => 1.0, 1 => 1.25, 2 => 1.5, 3 => 1.75, _ => 2.0 };
                                                                    let xp = ((base_xp as f64) * multiplier).round() as u32;
                                                                    score.set(sc + xp);
                                                                    streak.set(cs + 1);
                                                                    correct_count.set(cc + 1);
                                                                    completed_exercises.write().insert(current_idx);
                                                                    let msg = if multiplier > 1.0 { format!("Correct! +{} XP ({}x combo)", xp, multiplier) } else { format!("Correct! +{} XP", xp) };
                                                                    feedback.set(Some((true, msg)));
                                                                } else {
                                                                    let cc = *correct_count.read();
                                                                    correct_count.set(cc + 1);
                                                                    completed_exercises.write().insert(current_idx);
                                                                    feedback.set(Some((true, "Correct! (no XP - too many attempts)".to_string())));
                                                                }
                                                            } else {
                                                                feedback.set(Some((true, "Correct! (already completed)".to_string())));
                                                            }
                                                            struggle_detector.write().record_correct_attempt();
                                                            show_socratic_hint.set(false);
                                                        } else {
                                                            // Wrong answer
                                                            let attempts = exercise_attempts.read().get(&current_idx).copied().unwrap_or(0);
                                                            exercise_attempts.write().insert(current_idx, attempts + 1);
                                                            let remaining = 10u32.saturating_sub((attempts + 1) * 5);
                                                            let penalty_msg = if remaining > 0 { format!(" (-5 XP, {} remaining)", remaining) } else { " (no XP remaining)".to_string() };
                                                            feedback.set(Some((false, format!("{}{}", result.feedback, penalty_msg))));
                                                            struggle_detector.write().record_wrong_attempt();
                                                            show_socratic_hint.set(true);
                                                            streak.set(0);
                                                            if !is_test_mode {
                                                                let mut queue = retry_queue.write();
                                                                if !queue.contains(&current_idx) { queue.push_back(current_idx); }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                }
                                button {
                                    class: "exercise-submit-btn",
                                    onclick: {
                                        let golden = golden_answer.clone();
                                        move |_| {
                                            let answer = user_answer.read().clone();
                                            if !answer.is_empty() {
                                                if let Some(ref expected) = golden {
                                                    // Use the real grader
                                                    let result = check_answer(&answer, expected);
                                                    if result.correct {
                                                        // Check if answer was revealed (forfeits XP)
                                                        let answer_was_revealed = answer_revealed_exercises.read().contains(&current_idx);

                                                        // Only check completed_exercises in practice mode, not test mode
                                                        let already_completed = if is_test_mode {
                                                            false // Test mode always gives fresh XP
                                                        } else {
                                                            completed_exercises.read().contains(&current_idx)
                                                        };

                                                        if answer_was_revealed {
                                                            // Answer was revealed - no XP
                                                            let current_correct = *correct_count.read();
                                                            correct_count.set(current_correct + 1);
                                                            completed_exercises.write().insert(current_idx);
                                                            feedback.set(Some((true, "Correct! (no XP - answer was revealed)".to_string())));
                                                        } else if !already_completed {
                                                            // Calculate XP based on wrong attempts (each wrong costs 5 XP)
                                                            let wrong_count = *exercise_attempts.read().get(&current_idx).unwrap_or(&0);
                                                            let base_xp = 10u32.saturating_sub(wrong_count * 5);

                                                            if base_xp > 0 {
                                                                let current_streak = *streak.read();
                                                                let current_score = *score.read();
                                                                let current_correct = *correct_count.read();

                                                                // Combo multiplier: 1.0x, 1.25x, 1.5x, 1.75x, 2.0x
                                                                let multiplier = match current_streak {
                                                                    0 => 1.0,
                                                                    1 => 1.25,
                                                                    2 => 1.5,
                                                                    3 => 1.75,
                                                                    _ => 2.0, // Max 2x
                                                                };
                                                                let xp = ((base_xp as f64) * multiplier).round() as u32;

                                                                score.set(current_score + xp);
                                                                streak.set(current_streak + 1);
                                                                correct_count.set(current_correct + 1);
                                                                completed_exercises.write().insert(current_idx);

                                                                let msg = if multiplier > 1.0 {
                                                                    format!("Correct! +{} XP ({}x combo)", xp, multiplier)
                                                                } else {
                                                                    format!("Correct! +{} XP", xp)
                                                                };
                                                                feedback.set(Some((true, msg)));
                                                            } else {
                                                                // Too many wrong attempts - no XP but still mark complete
                                                                let current_correct = *correct_count.read();
                                                                correct_count.set(current_correct + 1);
                                                                completed_exercises.write().insert(current_idx);
                                                                feedback.set(Some((true, "Correct! (no XP - too many attempts)".to_string())));
                                                            }
                                                        } else {
                                                            // Already earned XP for this exercise
                                                            feedback.set(Some((true, "Correct! (already completed)".to_string())));
                                                        }
                                                        struggle_detector.write().record_correct_attempt();
                                                        show_socratic_hint.set(false);
                                                    } else {
                                                        // Wrong answer - record attempt and show feedback
                                                        let attempts = exercise_attempts.read().get(&current_idx).copied().unwrap_or(0);
                                                        exercise_attempts.write().insert(current_idx, attempts + 1);

                                                        let remaining = 10u32.saturating_sub((attempts + 1) * 5);
                                                        let penalty_msg = if remaining > 0 {
                                                            format!(" (-5 XP, {} remaining)", remaining)
                                                        } else {
                                                            " (no XP remaining)".to_string()
                                                        };

                                                        feedback.set(Some((false, format!("{}{}", result.feedback, penalty_msg))));
                                                        struggle_detector.write().record_wrong_attempt();
                                                        show_socratic_hint.set(true);
                                                        streak.set(0);

                                                        // In practice mode, add wrong exercise to retry queue (at +3 position)
                                                        if !is_test_mode {
                                                            let mut queue = retry_queue.write();
                                                            // Only add if not already in queue
                                                            if !queue.contains(&current_idx) {
                                                                queue.push_back(current_idx);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    "Check"
                                }
                            }
                        },
                        AnswerType::MultipleChoice { options, correct_index } => rsx! {
                            div { class: "multiple-choice-options",
                                for (idx, option) in options.iter().enumerate() {
                                    {
                                        let is_selected = user_answer.read().as_str() == option;
                                        let is_correct_option = idx == *correct_index;
                                        let option_clone = option.clone();
                                        let show_result = feedback.read().is_some();

                                        let btn_class = if show_result && is_selected {
                                            if is_correct_option { "reveal-btn active correct" } else { "reveal-btn active incorrect" }
                                        } else if is_selected {
                                            "reveal-btn active"
                                        } else {
                                            "reveal-btn"
                                        };

                                        rsx! {
                                            button {
                                                class: "{btn_class}",
                                                onclick: {
                                                    let opt = option_clone.clone();
                                                    let correct = is_correct_option;
                                                    move |_| {
                                                        user_answer.set(opt.clone());
                                                        if correct {
                                                            // Check if answer was revealed (forfeits XP)
                                                            let answer_was_revealed = answer_revealed_exercises.read().contains(&current_idx);

                                                            // Only check completed_exercises in practice mode, not test mode
                                                            let already_completed = if is_test_mode {
                                                                false // Test mode always gives fresh XP
                                                            } else {
                                                                completed_exercises.read().contains(&current_idx)
                                                            };

                                                            if answer_was_revealed {
                                                                // Answer was revealed - no XP
                                                                let current_correct = *correct_count.read();
                                                                correct_count.set(current_correct + 1);
                                                                completed_exercises.write().insert(current_idx);
                                                                feedback.set(Some((true, "Correct! (no XP - answer was revealed)".to_string())));
                                                            } else if !already_completed {
                                                                // Calculate XP based on wrong attempts (each wrong costs 5 XP)
                                                                let wrong_count = *exercise_attempts.read().get(&current_idx).unwrap_or(&0);
                                                                let base_xp = 10u32.saturating_sub(wrong_count * 5);

                                                                if base_xp > 0 {
                                                                    let current_streak = *streak.read();
                                                                    let current_score = *score.read();
                                                                    let current_correct = *correct_count.read();

                                                                    // Combo multiplier: 1.0x, 1.25x, 1.5x, 1.75x, 2.0x
                                                                    let multiplier = match current_streak {
                                                                        0 => 1.0,
                                                                        1 => 1.25,
                                                                        2 => 1.5,
                                                                        3 => 1.75,
                                                                        _ => 2.0, // Max 2x
                                                                    };
                                                                    let xp = ((base_xp as f64) * multiplier).round() as u32;

                                                                    score.set(current_score + xp);
                                                                    streak.set(current_streak + 1);
                                                                    correct_count.set(current_correct + 1);
                                                                    completed_exercises.write().insert(current_idx);

                                                                    let msg = if multiplier > 1.0 {
                                                                        format!("Correct! +{} XP ({}x combo)", xp, multiplier)
                                                                    } else {
                                                                        format!("Correct! +{} XP", xp)
                                                                    };
                                                                    feedback.set(Some((true, msg)));
                                                                } else {
                                                                    // Too many wrong attempts - no XP but still mark complete
                                                                    let current_correct = *correct_count.read();
                                                                    correct_count.set(current_correct + 1);
                                                                    completed_exercises.write().insert(current_idx);
                                                                    feedback.set(Some((true, "Correct! (no XP - too many attempts)".to_string())));
                                                                }
                                                            } else {
                                                                feedback.set(Some((true, "Correct! (already completed)".to_string())));
                                                            }
                                                            struggle_detector.write().record_correct_attempt();
                                                            show_socratic_hint.set(false);
                                                        } else {
                                                            // Wrong answer - record attempt and show feedback
                                                            let attempts = exercise_attempts.read().get(&current_idx).copied().unwrap_or(0);
                                                            exercise_attempts.write().insert(current_idx, attempts + 1);

                                                            let remaining = 10u32.saturating_sub((attempts + 1) * 5);
                                                            let penalty_msg = if remaining > 0 {
                                                                format!(" (-5 XP, {} remaining)", remaining)
                                                            } else {
                                                                " (no XP remaining)".to_string()
                                                            };

                                                            feedback.set(Some((false, format!("Not quite.{}", penalty_msg))));
                                                            struggle_detector.write().record_wrong_attempt();
                                                            show_socratic_hint.set(true);
                                                            streak.set(0);

                                                            // In practice mode, add wrong exercise to retry queue
                                                            if !is_test_mode {
                                                                let mut queue = retry_queue.write();
                                                                if !queue.contains(&current_idx) {
                                                                    queue.push_back(current_idx);
                                                                }
                                                            }
                                                        }
                                                    }
                                                },
                                                "{option}"
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        AnswerType::Ambiguity { readings } => rsx! {
                            div { class: "ambiguity-readings",
                                p { class: "exercise-prompt", "This sentence has {readings.len()} possible interpretations:" }
                                for (i, reading) in readings.iter().enumerate() {
                                    div { class: "revealed-logic",
                                        span { class: "revealed-label", "Reading {i + 1}" }
                                        "{reading}"
                                    }
                                }
                            }
                        },
                    }

                    // Show feedback
                    if let Some((is_correct, msg)) = feedback.read().as_ref() {
                        div {
                            class: if *is_correct { "exercise-feedback correct" } else { "exercise-feedback incorrect" },
                            "{msg}"
                        }
                    }

                    // Socratic hint (triggered by wrong answer or inactivity) - NOT in test mode
                    if !is_test_mode && *show_socratic_hint.read() && hint_text.is_some() {
                        div { class: "socratic-hint-box",
                            div { class: "hint-header",
                                "🦉 Socrates says..."
                            }
                            div { class: "hint-text",
                                if let Some(reason) = struggle_detector.read().reason() {
                                    "{reason.message()} "
                                }
                                if let Some(hint) = hint_text.as_ref() {
                                    "{hint}"
                                }
                            }
                        }
                    }

                    // Interactive reveal buttons - NOT in test mode
                    if !is_test_mode {
                    div { class: "reveal-section",
                        div { class: "reveal-buttons",
                            // Show Hint button - toggles independently
                            button {
                                class: if reveal_state.read().hint { "reveal-btn active" } else { "reveal-btn" },
                                onclick: move |_| {
                                    let current = reveal_state.read().hint;
                                    reveal_state.write().hint = !current;
                                },
                                "💡 Show Hint"
                            }

                            // Show Answer button - toggles independently, forfeits XP when revealed
                            button {
                                class: if reveal_state.read().answer { "reveal-btn active" } else { "reveal-btn" },
                                onclick: move |_| {
                                    let current = reveal_state.read().answer;
                                    if !current {
                                        // Revealing answer for first time - forfeit XP for this exercise
                                        answer_revealed_exercises.write().insert(current_idx);
                                    }
                                    reveal_state.write().answer = !current;
                                },
                                "✓ Show Answer (No XP)"
                            }

                            // Symbol Dictionary button (only for FreeForm/Ambiguity) - toggles independently
                            if matches!(&challenge.answer, AnswerType::FreeForm { .. } | AnswerType::Ambiguity { .. }) {
                                button {
                                    class: if reveal_state.read().symbol_dictionary { "reveal-btn active" } else { "reveal-btn" },
                                    onclick: move |_| {
                                        let current = reveal_state.read().symbol_dictionary;
                                        reveal_state.write().symbol_dictionary = !current;
                                    },
                                    "📖 Symbol Dictionary"
                                }
                            }
                        }

                        // Stacked revealed content - each section shows independently
                        if reveal_state.read().hint {
                            div { class: "revealed-content",
                                div { class: "revealed-label", "Hint" }
                                if let Some(hint) = hint_text.as_ref() {
                                    p { "{hint}" }
                                } else {
                                    p { "No hint available for this exercise." }
                                }
                            }
                        }

                        if reveal_state.read().answer {
                            div { class: "revealed-content",
                                div { class: "revealed-label", "Correct Answer" }
                                if let Some(answer) = golden_answer.as_ref() {
                                    div { class: "revealed-logic", "{answer}" }
                                    // Show explanation if available
                                    if let Some(explanation) = challenge.explanation.as_ref() {
                                        p { style: "margin-top: 12px; color: var(--text-secondary);", "{explanation}" }
                                    }
                                }
                            }
                        }

                        if reveal_state.read().symbol_dictionary {
                            div { class: "revealed-content",
                                if let Some(answer) = golden_answer.as_ref() {
                                    SymbolDictionary { logic: answer.clone() }
                                } else {
                                    p { "No logic output to analyze." }
                                }
                            }
                        }
                    }
                    } // end if !is_test_mode

                    // Action buttons
                    div { class: "learn-module-actions", style: "margin-top: 24px;",
                        if is_test_mode {
                            // Test mode: Submit answer and move to next question
                            button {
                                class: "learn-action-btn primary",
                                onclick: {
                                    let golden = golden_answer.clone();
                                    move |_| {
                                        let answer = user_answer.read().clone();
                                        let is_correct = if let Some(ref expected) = golden {
                                            if !answer.is_empty() {
                                                check_answer(&answer, expected).correct
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        };

                                        // Record answer
                                        test_answers.write().push(is_correct);

                                        // Move to next question or complete
                                        let next_q = current_test_q + 1;
                                        if next_q >= test_total {
                                            test_complete.set(true);
                                        } else {
                                            test_question.set(next_q);
                                            // Also advance the exercise index for variety
                                            let next_idx = (current_idx + 1) % total_exercises;
                                            current_exercise_idx.set(next_idx);
                                        }
                                        user_answer.set(String::new());
                                        feedback.set(None);
                                    }
                                },
                                "Submit Answer →"
                            }
                            button {
                                class: "learn-action-btn secondary",
                                onclick: move |_| {
                                    practice_mode.set(PracticeMode::Practice);
                                },
                                "Exit Test"
                            }
                        } else {
                        // Practice mode buttons
                        button {
                            class: "learn-action-btn secondary",
                            onclick: {
                                move |_| {
                                    // Simple linear progression - skip to next
                                    let next = current_idx + 1;
                                    if next < total_exercises {
                                        current_exercise_idx.set(next);
                                    } else {
                                        current_exercise_idx.set(0); // Loop back to start
                                    }
                                    // Reset state
                                    user_answer.set(String::new());
                                    feedback.set(None);
                                    reveal_state.write().reset();
                                    struggle_detector.write().reset();
                                    show_socratic_hint.set(false);
                                }
                            },
                            "Skip →"
                        }

                        if feedback.read().as_ref().map(|(c, _)| *c).unwrap_or(false) {
                            button {
                                class: "learn-action-btn primary",
                                onclick: {
                                    move |_| {
                                        // Simple linear progression after correct answer
                                        let next = current_idx + 1;
                                        if next < total_exercises {
                                            current_exercise_idx.set(next);
                                        } else {
                                            current_exercise_idx.set(0); // Loop back to start
                                        }
                                        user_answer.set(String::new());
                                        feedback.set(None);
                                        reveal_state.write().reset();
                                        struggle_detector.write().reset();
                                        show_socratic_hint.set(false);
                                    }
                                },
                                "Next Exercise →"
                            }
                        }

                        // Show "Take Test" button after 5 correct answers
                        if *correct_count.read() >= 5 {
                            button {
                                class: "learn-action-btn test-ready",
                                style: "background: linear-gradient(135deg, #fbbf24, #f59e0b); margin-left: auto;",
                                onclick: move |_| {
                                    practice_mode.set(PracticeMode::Test);
                                    test_question.set(0);
                                    test_answers.set(Vec::new());
                                    test_complete.set(false);
                                    user_answer.set(String::new());
                                    feedback.set(None);
                                    reveal_state.write().reset();
                                },
                                "🎯 Take Test"
                            }
                        }
                        } // end else (practice mode)
                    }
                }
            } else {
                div { style: "text-align: center; padding: var(--spacing-xl); color: var(--text-secondary);",
                    p { "No exercises available for this module yet." }
                    p { style: "font-size: var(--font-caption-md); margin-top: var(--spacing-md);",
                        "Total loaded: {total_exercises}"
                    }
                }
            }
            } // end if !(is_test_mode && test_complete)
            } // end else (Practice view)
        }
    }
}
