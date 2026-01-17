//! Combo streak indicator component.
//!
//! Displays the current combo count with animated flames and multiplier.
//! Shows "NEW RECORD!" toast when achieving a personal best.
//!
//! # Props
//!
//! - `combo` - Current combo count (0 hides the component)
//! - `multiplier` - XP multiplier (e.g., 1.5x)
//! - `is_new_record` - Whether this combo beats the personal record

use dioxus::prelude::*;

const COMBO_STYLE: &str = r#"
.combo-record {
    position: absolute;
    top: -24px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 12px;
    color: #fbbf24;
    font-weight: 600;
    animation: record-flash 1.5s ease-out forwards;
    white-space: nowrap;
    pointer-events: none;
}

@keyframes record-flash {
    0% { opacity: 0; transform: translateX(-50%) translateY(10px) scale(0.8); }
    20% { opacity: 1; transform: translateX(-50%) translateY(0) scale(1.1); }
    40% { opacity: 1; transform: translateX(-50%) translateY(0) scale(1); }
    100% { opacity: 0; transform: translateX(-50%) translateY(-10px) scale(0.9); }
}
.combo-indicator {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    background: rgba(249, 115, 22, 0.15);
    border: 1px solid rgba(249, 115, 22, 0.3);
    border-radius: 24px;
    animation: combo-pulse 0.3s ease-out;
}

@keyframes combo-pulse {
    0% { transform: scale(1); }
    50% { transform: scale(1.1); }
    100% { transform: scale(1); }
}

.combo-flames {
    display: flex;
    gap: 2px;
}

.flame {
    font-size: 16px;
    animation: flame-dance 0.5s ease-in-out infinite alternate;
}

.flame:nth-child(2) { animation-delay: 0.1s; }
.flame:nth-child(3) { animation-delay: 0.2s; }

@keyframes flame-dance {
    from { transform: translateY(0) scale(1); }
    to { transform: translateY(-2px) scale(1.1); }
}

.combo-count {
    font-size: 20px;
    font-weight: 700;
    color: #f97316;
}

.combo-multiplier {
    font-size: 14px;
    color: #fb923c;
    font-weight: 500;
}

.combo-wrapper {
    position: relative;
    display: inline-flex;
}
"#;

#[component]
pub fn ComboIndicator(combo: u32, multiplier: f64, is_new_record: bool) -> Element {
    let mut show_record = use_signal(|| false);
    let mut last_record_combo = use_signal(|| 0u32);

    if is_new_record && combo > last_record_combo() {
        show_record.set(true);
        last_record_combo.set(combo);

        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(1500).await;
            show_record.set(false);
        });
    }

    if combo == 0 {
        return rsx! {};
    }

    let flame_count = (combo.min(5)) as usize;

    rsx! {
        style { "{COMBO_STYLE}" }
        div { class: "combo-wrapper",
            if show_record() {
                div { class: "combo-record", "NEW RECORD!" }
            }
            div { class: "combo-indicator",
                div { class: "combo-flames",
                    for _ in 0..flame_count {
                        span { class: "flame", "🔥" }
                    }
                }
                span { class: "combo-count", "{combo}x" }
                span { class: "combo-multiplier", "({multiplier:.1}x)" }
            }
        }
    }
}
