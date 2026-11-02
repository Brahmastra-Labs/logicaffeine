//! End-of-section "Check yourself" retrieval cards (Roediger & Karpicke).
//!
//! Free recall with a reveal: the reader answers in their head, reveals, and
//! self-marks — the mark feeds the SM-2 deck (`guide_review`) so the card
//! comes back right when it is about to fade. The answer is CONDITIONALLY
//! rendered, never CSS-hidden: a flashcard that leaks its back into the DOM
//! teaches nothing (and the render lock proves it).

use crate::guide_review::record_recall_mark;
use crate::progress::UserProgress;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct RecallCardProps {
    pub id: String,
    pub question: String,
    pub answer: String,
}

#[component]
pub fn RecallCard(props: RecallCardProps) -> Element {
    let mut revealed = use_signal(|| false);
    let mut marked = use_signal(|| false);

    let progress_ctx = try_consume_context::<Signal<UserProgress>>();
    let recall_id = props.id.clone();
    let self_mark = move |quality: u8| {
        let today = crate::ui::pages::guide::today_string();
        match progress_ctx {
            Some(mut progress) => {
                record_recall_mark(&mut progress.write(), &recall_id, quality, &today);
            }
            None => {
                let mut progress = UserProgress::load();
                record_recall_mark(&mut progress, &recall_id, quality, &today);
            }
        }
        marked.set(true);
    };

    rsx! {
        div {
            class: "guide-recall-card",
            id: "{props.id}",
            style: "border: 1px solid rgba(255,255,255,0.10); border-radius: 10px; padding: 12px 16px; background: rgba(255,255,255,0.03);",

            div {
                style: "display: flex; align-items: center; justify-content: space-between; gap: 12px;",
                span {
                    style: "font-size: 14px; color: rgba(229,231,235,0.9); line-height: 1.5;",
                    "{props.question}"
                }
                if !*revealed.read() {
                    button {
                        class: "guide-recall-reveal",
                        style: "flex: none; padding: 5px 12px; border-radius: 8px; border: 1px solid rgba(255,255,255,0.15); background: rgba(255,255,255,0.05); color: rgba(229,231,235,0.75); font-size: 12px; cursor: pointer;",
                        onclick: move |_| revealed.set(true),
                        "Reveal"
                    }
                }
            }

            if *revealed.read() {
                div {
                    style: "margin-top: 10px; padding-top: 10px; border-top: 1px solid rgba(255,255,255,0.08); font-size: 13px; color: #a78bfa; line-height: 1.6;",
                    "{props.answer}"
                }
                if !*marked.read() {
                    div {
                        class: "guide-recall-marks",
                        style: "display: flex; gap: 8px; margin-top: 10px; align-items: center;",
                        span {
                            style: "font-size: 11px; color: rgba(229,231,235,0.45);",
                            "Did you have it?"
                        }
                        button {
                            class: "guide-recall-mark",
                            style: "padding: 4px 10px; border-radius: 6px; border: 1px solid rgba(248,113,113,0.4); background: none; color: #f87171; font-size: 11px; cursor: pointer;",
                            onclick: { let mut m = self_mark.clone(); move |_| m(1) },
                            "Again"
                        }
                        button {
                            class: "guide-recall-mark",
                            style: "padding: 4px 10px; border-radius: 6px; border: 1px solid rgba(251,191,36,0.4); background: none; color: #fbbf24; font-size: 11px; cursor: pointer;",
                            onclick: { let mut m = self_mark.clone(); move |_| m(3) },
                            "Hard"
                        }
                        button {
                            class: "guide-recall-mark",
                            style: "padding: 4px 10px; border-radius: 6px; border: 1px solid rgba(96,165,250,0.4); background: none; color: #60a5fa; font-size: 11px; cursor: pointer;",
                            onclick: { let mut m = self_mark.clone(); move |_| m(4) },
                            "Good"
                        }
                        button {
                            class: "guide-recall-mark",
                            style: "padding: 4px 10px; border-radius: 6px; border: 1px solid rgba(34,197,94,0.4); background: none; color: #22c55e; font-size: 11px; cursor: pointer;",
                            onclick: { let mut m = self_mark.clone(); move |_| m(5) },
                            "Easy"
                        }
                    }
                } else {
                    div {
                        style: "margin-top: 10px; font-size: 11px; color: rgba(229,231,235,0.45);",
                        "Scheduled — it will resurface on the Review page when it's about to fade."
                    }
                }
            }
        }
    }
}

/// A section's recall cards under one "Check yourself" heading.
#[component]
pub fn RecallBlock(cards: Vec<(String, String, String)>) -> Element {
    rsx! {
        div {
            class: "guide-recall",
            style: "margin-top: 24px; display: flex; flex-direction: column; gap: 10px;",
            div {
                style: "font-size: 13px; font-weight: 700; color: rgba(229,231,235,0.85); text-transform: uppercase; letter-spacing: 0.5px;",
                "Check yourself"
            }
            for (id, question, answer) in cards {
                RecallCard { id, question, answer }
            }
        }
    }
}
