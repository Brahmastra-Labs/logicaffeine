use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::main_nav::{MainNav, ActivePage};

const LANDING_STYLE: &str = r#"
:root {
  --bg0: #070a12;
  --bg1: #0b1022;
  --card: rgba(255,255,255,0.06);
  --card2: rgba(255,255,255,0.04);
  --border: rgba(255,255,255,0.10);
  --border2: rgba(255,255,255,0.14);
  --text: #e5e7eb;
  --muted: rgba(229,231,235,0.72);
  --muted2: rgba(229,231,235,0.56);
  --brand: #a78bfa;
  --brand2:#60a5fa;
  --ok: #22c55e;
  --shadow: 0 30px 80px rgba(0,0,0,0.55);
}

* { box-sizing: border-box; }
a { color: inherit; }

body:has(.landing) {
  overflow: hidden;
}

.landing {
  height: 100vh;
  color: var(--text);
  background:
    radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.18), transparent 60%),
    radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.18), transparent 60%),
    radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.10), transparent 62%),
    linear-gradient(180deg, var(--bg0), var(--bg1) 55%, #070a12);
  overflow-x: hidden;
  overflow-y: auto;
  font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Inter, "Helvetica Neue", Arial, "Noto Sans", "Apple Color Emoji", "Segoe UI Emoji";
  position: relative;
}

.bg-orb {
  position: absolute;
  inset: auto;
  width: 520px;
  height: 520px;
  border-radius: 999px;
  filter: blur(42px);
  opacity: 0.22;
  pointer-events: none;
  animation: float 14s ease-in-out infinite, pulse-glow 10s ease-in-out infinite;
}
.orb1 { top: -220px; left: -160px; background: radial-gradient(circle at 30% 30%, var(--brand2), transparent 60%); animation-delay: 0s; }
.orb2 { top: 120px; right: -200px; background: radial-gradient(circle at 40% 35%, var(--brand), transparent 60%); animation-delay: -5s; }
.orb3 { bottom: -260px; left: 20%; background: radial-gradient(circle at 40% 35%, rgba(34,197,94,0.9), transparent 60%); animation-delay: -10s; }

.container {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  padding: 0 20px;
}

.nav {
  position: sticky;
  top: 0;
  z-index: 50;
  backdrop-filter: blur(18px);
  background: linear-gradient(180deg, rgba(7,10,18,0.72), rgba(7,10,18,0.44));
  border-bottom: 1px solid rgba(255,255,255,0.06);
}

.nav-inner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 0;
  gap: 14px;
}

.brand {
  display: flex;
  align-items: center;
  gap: 12px;
  text-decoration: none;
}

.logo {
  width: 36px;
  height: 36px;
  border-radius: 12px;
  background:
    radial-gradient(circle at 30% 30%, rgba(96,165,250,0.85), transparent 55%),
    radial-gradient(circle at 65% 60%, rgba(167,139,250,0.85), transparent 55%),
    rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.10);
  box-shadow: 0 14px 35px rgba(0,0,0,0.35);
}

.brand-name {
  display: flex;
  flex-direction: column;
  line-height: 1.05;
}

.brand-name strong {
  font-weight: 800;
  letter-spacing: -0.5px;
  font-size: 14px;
}
.brand-name span {
  font-size: 12px;
  color: var(--muted2);
}

.nav-links {
  display: flex;
  gap: 18px;
  align-items: center;
  color: var(--muted);
  font-size: 14px;
}
.nav-links a {
  text-decoration: none;
  padding: 8px 10px;
  border-radius: 10px;
  transition: background 0.18s ease, color 0.18s ease;
}
.nav-links a:hover {
  background: rgba(255,255,255,0.05);
  color: rgba(255,255,255,0.92);
}

.nav-cta {
  display: flex;
  gap: 10px;
  align-items: center;
}

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 12px 16px;
  border-radius: 14px;
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.05);
  text-decoration: none;
  font-weight: 650;
  font-size: 14px;
  transition: transform 0.18s ease, background 0.18s ease, border-color 0.18s ease;
  will-change: transform;
}
.btn:hover { transform: translateY(-1px); background: rgba(255,255,255,0.07); border-color: rgba(255,255,255,0.18); }
.btn:active { transform: translateY(0px); }

.btn-primary {
  background: linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95));
  border-color: rgba(255,255,255,0.20);
  color: #060814;
  box-shadow: 0 18px 40px rgba(96,165,250,0.18);
}
.btn-primary:hover {
  background: linear-gradient(135deg, rgba(96,165,250,1.0), rgba(167,139,250,1.0));
}

.btn-ghost {
  background: rgba(255,255,255,0.03);
}

.btn-icon {
  padding: 10px;
  background: rgba(255,255,255,0.03);
}
.btn-icon svg {
  width: 20px;
  height: 20px;
  fill: currentColor;
}

.github-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: inherit;
  text-decoration: none;
  transition: color 0.2s ease;
}
.github-link:hover {
  color: var(--text);
}

.hero {
  padding: 84px 0 30px;
}

.hero-grid {
  display: grid;
  grid-template-columns: 1.05fr 0.95fr;
  gap: 36px;
  align-items: center;
}

.badge {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-radius: 999px;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.10);
  backdrop-filter: blur(18px);
  box-shadow: 0 18px 40px rgba(0,0,0,0.25);
  color: rgba(255,255,255,0.88);
  font-size: 13px;
  font-weight: 650;
}
.badge .dot {
  width: 8px;
  height: 8px;
  border-radius: 99px;
  background: var(--ok);
  box-shadow: 0 0 0 6px rgba(34,197,94,0.12);
  animation: pulse-glow 2s ease-in-out infinite;
}

.hero .badge { animation: fadeInUp 0.6s ease both; }
.hero .h-title { animation: fadeInUp 0.6s ease 0.08s both; }
.hero .h-sub { animation: fadeInUp 0.6s ease 0.16s both; }
.hero .hero-ctas { animation: fadeInUp 0.6s ease 0.24s both; }
.hero .microcopy { animation: fadeInUp 0.6s ease 0.30s both; }
.hero .kpi { animation: fadeInUp 0.6s ease 0.36s both; }
.hero .demo { animation: fadeInUp 0.8s ease 0.44s both; }

.h-title {
  margin: 18px 0 12px;
  font-size: 62px;
  line-height: 1.04;
  letter-spacing: -2px;
  font-weight: 900;
  background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.h-sub {
  margin: 0 0 20px;
  max-width: 580px;
  color: var(--muted);
  font-size: 18px;
  line-height: 1.65;
}

.hero-ctas {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
  margin: 18px 0 14px;
}

.microcopy {
  font-size: 13px;
  color: var(--muted2);
}

.demo {
  border-radius: 20px;
  border: 1px solid rgba(255,255,255,0.10);
  background: linear-gradient(180deg, rgba(255,255,255,0.06), rgba(255,255,255,0.03));
  backdrop-filter: blur(18px);
  box-shadow: var(--shadow);
  overflow: hidden;
  position: relative;
}

.demo::before {
  content: "";
  position: absolute;
  inset: -2px;
  background: radial-gradient(600px 280px at 10% 10%, rgba(96,165,250,0.22), transparent 55%),
              radial-gradient(520px 240px at 90% 20%, rgba(167,139,250,0.22), transparent 55%);
  opacity: 0.9;
  pointer-events: none;
}

.demo-head {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-bottom: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.10);
}

.win-dots { display: flex; gap: 8px; align-items: center; }
.wdot { width: 11px; height: 11px; border-radius: 99px; opacity: 0.9; }
.wr { background: #ef4444; } .wy { background: #fbbf24; } .wg { background: #22c55e; }

.demo-label {
  font-size: 12px;
  color: rgba(229,231,235,0.78);
  border: 1px solid rgba(255,255,255,0.10);
  padding: 7px 10px;
  border-radius: 999px;
  background: rgba(255,255,255,0.04);
}

.demo-body {
  position: relative;
  display: grid;
  grid-template-columns: 1fr 1fr;
}

.demo-col {
  padding: 18px 18px 22px;
  min-height: 240px;
}

.demo-col + .demo-col {
  border-left: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.18);
}

.demo-kicker {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
  font-size: 12px;
  color: rgba(229,231,235,0.72);
}

.pill {
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
  padding: 6px 10px;
  border-radius: 999px;
}

.code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  font-size: 13px;
  line-height: 1.6;
  color: rgba(229,231,235,0.90);
  white-space: pre-wrap;
}

.code.logic { color: rgba(167,139,250,0.96); }

.demo-foot {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  padding: 14px 16px;
  border-top: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.12);
  color: rgba(229,231,235,0.70);
  font-size: 13px;
}

.section {
  padding: 74px 0;
}

.section-title {
  font-size: 30px;
  letter-spacing: -0.8px;
  margin: 0 0 10px;
}
.section-sub {
  margin: 0 0 26px;
  color: var(--muted);
  line-height: 1.65;
  max-width: 760px;
}

.grid3 {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 18px;
}
.grid2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 18px;
}

.card {
  position: relative;
  border-radius: 18px;
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
  backdrop-filter: blur(18px);
  padding: 18px;
  transition: transform 0.18s ease, border-color 0.18s ease, background 0.18s ease;
  overflow: hidden;
}
.card::before {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 18px;
  background: linear-gradient(135deg, rgba(96,165,250,0.12), rgba(167,139,250,0.12));
  opacity: 0;
  transition: opacity 0.3s ease;
  pointer-events: none;
}
.card:hover {
  transform: translateY(-3px);
  border-color: rgba(167,139,250,0.28);
  background: rgba(255,255,255,0.06);
}
.card:hover::before {
  opacity: 1;
}

.icon {
  width: 42px; height: 42px;
  border-radius: 14px;
  display: grid;
  place-items: center;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.10);
  margin-bottom: 12px;
}

.card h3 {
  margin: 0 0 8px;
  font-size: 16px;
  letter-spacing: -0.2px;
}
.card p {
  margin: 0;
  color: var(--muted);
  line-height: 1.6;
  font-size: 14px;
}

.quote {
  font-size: 14px;
  line-height: 1.65;
  color: rgba(229,231,235,0.86);
}
.quoter {
  margin-top: 10px;
  color: var(--muted2);
  font-size: 13px;
}

.kpi {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
  margin-top: 18px;
}
.kpi .pill {
  background: rgba(255,255,255,0.04);
}

.tech-stack {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 14px;
}

.tech-badge {
  font-size: 12px;
  padding: 6px 12px;
  border-radius: 6px;
  background: rgba(255,255,255,0.03);
  border: 1px solid rgba(255,255,255,0.08);
  color: var(--muted);
}

.tech-badge.rust {
  background: linear-gradient(135deg, rgba(183,65,14,0.15), rgba(222,165,132,0.10));
  border-color: rgba(222,165,132,0.3);
  color: #dea584;
}

.hello-demo {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 24px;
  flex-wrap: wrap;
  margin: 24px 0;
}

.hello-code, .hello-result {
  flex: 1;
  min-width: 280px;
  max-width: 400px;
  border-radius: 12px;
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(0,0,0,0.3);
  overflow: hidden;
}

.code-header {
  padding: 10px 14px;
  font-size: 12px;
  color: var(--muted);
  background: rgba(255,255,255,0.03);
  border-bottom: 1px solid rgba(255,255,255,0.06);
  font-family: ui-monospace, monospace;
}

.hello-code .code {
  margin: 0;
  padding: 16px;
  font-size: 14px;
  line-height: 1.6;
}

.terminal {
  padding: 16px;
  font-family: ui-monospace, monospace;
  font-size: 14px;
}

.terminal .prompt {
  color: var(--ok);
}

.terminal .output {
  color: var(--text);
}

.hello-arrow {
  font-size: 28px;
  color: var(--brand);
}

.hello-note {
  text-align: center;
  font-size: 14px;
  color: var(--muted);
  margin-top: 8px;
}

.compare-table {
  display: flex;
  flex-direction: column;
  border-radius: 12px;
  border: 1px solid rgba(255,255,255,0.10);
  overflow: hidden;
  max-width: 800px;
  margin: 0 auto;
}

.compare-row {
  display: grid;
  grid-template-columns: 1.2fr repeat(4, 1fr);
}

.compare-row.header {
  background: rgba(255,255,255,0.05);
  font-weight: 600;
  font-size: 13px;
}

.compare-row:not(.header) {
  border-top: 1px solid rgba(255,255,255,0.06);
}

.compare-cell {
  padding: 12px 14px;
  font-size: 13px;
  color: var(--muted);
  text-align: center;
}

.compare-cell.label {
  text-align: left;
  color: var(--text);
  font-weight: 500;
}

.compare-cell.highlight {
  background: rgba(167,139,250,0.08);
  color: var(--brand);
  font-weight: 500;
}

.compare-row.header .compare-cell.highlight {
  background: rgba(167,139,250,0.15);
}

@media (max-width: 700px) {
  .compare-row {
    grid-template-columns: 1fr 1fr 1fr;
  }
  .compare-cell:nth-child(4),
  .compare-cell:nth-child(5) {
    display: none;
  }
}

.faq-item {
  padding: 16px 16px 14px;
  border-radius: 16px;
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.03);
}
.faq-q { font-weight: 750; margin-bottom: 8px; }
.faq-a { color: var(--muted); line-height: 1.6; font-size: 14px; }

.footer {
  padding: 34px 0 44px;
  border-top: 1px solid rgba(255,255,255,0.06);
  color: var(--muted2);
  font-size: 13px;
}

.footer-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}

@media (max-width: 980px) {
  .hero-grid { grid-template-columns: 1fr; }
  .demo-body { grid-template-columns: 1fr; }
  .demo-col + .demo-col { border-left: none; border-top: 1px solid rgba(255,255,255,0.06); }
  .grid3 { grid-template-columns: 1fr; }
  .grid2 { grid-template-columns: 1fr; }
  .h-title { font-size: 48px; }
  .nav-links { display: none; }
}

@keyframes fadeInUp {
  from { opacity: 0; transform: translateY(24px); }
  to { opacity: 1; transform: translateY(0); }
}

@keyframes float {
  0%, 100% { transform: translate3d(0, 0, 0); }
  50% { transform: translate3d(0, -20px, 0); }
}

@keyframes pulse-glow {
  0%, 100% { opacity: 0.22; }
  50% { opacity: 0.32; }
}

@keyframes blink {
  50% { opacity: 0; }
}

html { scroll-behavior: smooth; }

.section + .section {
  border-top: 1px solid rgba(255,255,255,0.04);
}

.steps {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 20px;
  flex-wrap: wrap;
}

.step {
  flex: 1;
  min-width: 200px;
  max-width: 280px;
  text-align: center;
  padding: 24px;
  border-radius: 18px;
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  animation: fadeInUp 0.6s ease both;
}

.step:nth-child(1) { animation-delay: 0s; }
.step:nth-child(3) { animation-delay: 0.1s; }
.step:nth-child(5) { animation-delay: 0.2s; }

.step-num {
  width: 48px;
  height: 48px;
  margin: 0 auto 16px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--brand2), var(--brand));
  color: #060814;
  font-weight: 800;
  font-size: 20px;
  display: grid;
  place-items: center;
}

.step h3 {
  margin: 0 0 8px;
  font-size: 18px;
}

.step p {
  margin: 0;
  color: var(--muted);
  font-size: 14px;
  line-height: 1.5;
}

.step-arrow {
  font-size: 24px;
  color: var(--muted2);
}

.grid3 .card:nth-child(1) .icon { background: rgba(96,165,250,0.15); }
.grid3 .card:nth-child(2) .icon { background: rgba(167,139,250,0.15); }
.grid3 .card:nth-child(3) .icon { background: rgba(34,197,94,0.15); }
.grid3 .card:nth-child(4) .icon { background: rgba(251,191,36,0.15); }
.grid3 .card:nth-child(5) .icon { background: rgba(236,72,153,0.15); }
.grid3 .card:nth-child(6) .icon { background: rgba(139,92,246,0.15); }

.demo-col:first-child .code::after {
  content: " ▋";
  animation: blink 1s step-end infinite;
  color: var(--brand2);
}

@media (max-width: 980px) {
  .hero-grid { grid-template-columns: 1fr; }
  .demo-body { grid-template-columns: 1fr; }
  .demo-col + .demo-col { border-left: none; border-top: 1px solid rgba(255,255,255,0.06); }
  .grid3 { grid-template-columns: 1fr; }
  .grid2 { grid-template-columns: 1fr; }
  .h-title { font-size: 48px; }
  .nav-links { display: none; }
  .step-arrow { display: none; }
  .steps { flex-direction: column; }
}

@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
}
"#;

#[component]
pub fn Landing() -> Element {
    rsx! {
        style { "{LANDING_STYLE}" }

        div { class: "landing",
            div { class: "bg-orb orb1" }
            div { class: "bg-orb orb2" }
            div { class: "bg-orb orb3" }

            MainNav { active: ActivePage::Home }

            main { class: "container",
                section { class: "hero",
                    div { class: "hero-grid",
                        div {
                            div { class: "badge",
                                div { class: "dot" }
                                span { "Free for individuals • Commercial licenses available" }
                            }

                            h1 { class: "h-title", "Debug Your Thoughts." }

                            p { class: "h-sub",
                                "Turn everyday English into rigorous First-Order Logic. Verify arguments, surface hidden assumptions, and build rule systems that actually hold up."
                            }

                            div { class: "hero-ctas",
                                Link { to: Route::Learn {}, class: "btn btn-primary", "Start Learning" }
                                Link { to: Route::Studio {}, class: "btn", "Open Studio" }
                                Link { to: Route::Pricing {}, class: "btn btn-ghost", "See Pricing" }
                            }

                            p { class: "microcopy",
                                "Built for people who take thinking seriously: students, researchers, engineers, analysts, and attorneys."
                            }

                            div { class: "kpi",
                                span { class: "pill", "Plain English in" }
                                span { class: "pill", "Formal logic out" }
                                span { class: "pill", "Zero guesswork" }
                            }

                            div { class: "tech-stack",
                                span { class: "tech-badge rust", "Rust-Powered 🦀" }
                                span { class: "tech-badge", "WASM Ready" }
                                span { class: "tech-badge", "Markdown Source" }
                                span { class: "tech-badge", "Proof-Checked" }
                            }
                        }

                        div { class: "demo", id: "product",
                            div { class: "demo-head",
                                div { class: "win-dots",
                                    div { class: "wdot wr" }
                                    div { class: "wdot wy" }
                                    div { class: "wdot wg" }
                                }
                                div { class: "demo-label", "Live Transpilation Preview" }
                            }

                            div { class: "demo-body",
                                div { class: "demo-col",
                                    div { class: "demo-kicker",
                                        span { "Input (English)" }
                                        span { class: "pill", "Plain language" }
                                    }
                                    div { class: "code",
r#"Every user who has a key enters the room.
If a user enters the room, the alarm triggers.
No user who lacks a key can enter the room."# }
                                }

                                div { class: "demo-col",
                                    div { class: "demo-kicker",
                                        span { "Output (First-Order Logic)" }
                                        span { class: "pill", "Machine-checkable" }
                                    }
                                    div { class: "code logic",
r#"1) ∀x((User(x) ∧ ∃y(Key(y) ∧ Has(x,y))) → Enter(x, Room))
2) ∀x((User(x) ∧ Enter(x, Room)) → Trigger(Alarm))
3) ∀x((User(x) ∧ ¬∃y(Key(y) ∧ Has(x,y))) → ¬Enter(x, Room))"# }
                                }
                            }

                            div { class: "demo-foot",
                                span { "Your logic, formalized in milliseconds." }
                            }
                        }
                    }
                }

                section { class: "section how-it-works",
                    h2 { class: "section-title", "How it works" }
                    p { class: "section-sub",
                        "Three steps from thought to proof."
                    }

                    div { class: "steps",
                        div { class: "step",
                            div { class: "step-num", "1" }
                            h3 { "Write in English" }
                            p { "Type your argument, rule, or statement in plain language." }
                        }
                        div { class: "step-arrow", "→" }
                        div { class: "step",
                            div { class: "step-num", "2" }
                            h3 { "Get formal logic" }
                            p { "Instantly see the First-Order Logic representation." }
                        }
                        div { class: "step-arrow", "→" }
                        div { class: "step",
                            div { class: "step-num", "3" }
                            h3 { "Validate & refine" }
                            p { "The tutor surfaces ambiguities. You fix them." }
                        }
                    }
                }

                section { class: "section hello-world",
                    h2 { class: "section-title", "Hello World in LOGOS" }
                    p { class: "section-sub",
                        "Markdown files compile directly to native binaries. No ceremony, no boilerplate."
                    }

                    div { class: "hello-demo",
                        div { class: "hello-code",
                            div { class: "code-header", "hello.md" }
                            pre { class: "code",
r#"# Hello World

To run:
    Show "Hello, World!" to the console."# }
                        }
                        div { class: "hello-arrow", "→" }
                        div { class: "hello-result",
                            div { class: "code-header", "Output" }
                            div { class: "terminal",
                                span { class: "prompt", "$ " }
                                span { "logos run hello.md" }
                                br {}
                                span { class: "output", "Hello, World!" }
                            }
                        }
                    }
                    p { class: "hello-note", "Compiles to a native binary via Rust. Zero runtime overhead." }
                }

                section { class: "section",
                    h2 { class: "section-title", "What you get" }
                    p { class: "section-sub",
                        "LOGICAFFEINE translates intuition into structure — so you can test it, teach it, or ship it."
                    }

                    div { class: "grid3",
                        div { class: "card",
                            div { class: "icon", "⚡" }
                            h3 { "Instant Transpilation" }
                            p { "Type normal English. Get precise logic in seconds — readable enough to learn from, strict enough to verify." }
                        }
                        div { class: "card",
                            div { class: "icon", "🧠" }
                            h3 { "Socratic Tutor" }
                            p { "When your statement is ambiguous, the tutor asks questions that force clarity instead of guessing." }
                        }
                        div { class: "card",
                            div { class: "icon", "🧾" }
                            h3 { "Assumption Surfacing" }
                            p { "Reveal missing premises, hidden quantifiers, and scope mistakes — the usual sources of bad arguments." }
                        }
                        div { class: "card",
                            div { class: "icon", "🧪" }
                            h3 { "Consistency & Validity Checks" }
                            p { "Spot contradictions, invalid inferences, and rule collisions early — before they hit production or policy." }
                        }
                        div { class: "card",
                            div { class: "icon", "🧰" }
                            h3 { "Studio + Curriculum" }
                            p { "Explore freely in Studio, then build mastery in Learn with structured lessons and practice." }
                        }
                        div { class: "card",
                            div { class: "icon", "🔒" }
                            h3 { "Commercial-Ready" }
                            p { "Licensing options for teams and enterprises — with a path toward governance and controlled deployments." }
                        }
                    }
                }

                section { class: "section", id: "for",
                    h2 { class: "section-title", "Who uses LOGICAFFEINE" }
                    p { class: "section-sub",
                        "For people who want their reasoning to survive contact with reality."
                    }

                    div { class: "grid3",
                        div { class: "card",
                            div { class: "icon", "🎓" }
                            h3 { "Students & Educators" }
                            p { "Teach formal reasoning with feedback that's immediate, concrete, and harder to game than multiple choice." }
                        }
                        div { class: "card",
                            div { class: "icon", "⚖️" }
                            h3 { "Law, Policy, Compliance" }
                            p { "Translate policy language into verifiable rules. Reduce ambiguity. Make reviews faster and safer." }
                        }
                        div { class: "card",
                            div { class: "icon", "🛠️" }
                            h3 { "Engineering & Research" }
                            p { "Specify systems, constraints, and invariants in a form you can test — without forcing everyone into formal syntax." }
                        }
                    }
                }

                section { class: "section compare-section",
                    h2 { class: "section-title", "How LOGOS Compares" }
                    p { class: "section-sub",
                        "A new approach to formal reasoning."
                    }

                    div { class: "compare-table",
                        div { class: "compare-row header",
                            div { class: "compare-cell", "Feature" }
                            div { class: "compare-cell highlight", "LOGOS" }
                            div { class: "compare-cell", "Lean 4" }
                            div { class: "compare-cell", "Rust" }
                            div { class: "compare-cell", "Python" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Syntax" }
                            div { class: "compare-cell highlight", "English prose" }
                            div { class: "compare-cell", "Lean DSL" }
                            div { class: "compare-cell", "Symbols" }
                            div { class: "compare-cell", "Symbols" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "File Format" }
                            div { class: "compare-cell highlight", "Markdown (.md)" }
                            div { class: "compare-cell", ".lean" }
                            div { class: "compare-cell", ".rs" }
                            div { class: "compare-cell", ".py" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Performance" }
                            div { class: "compare-cell highlight", "Native (via Rust)" }
                            div { class: "compare-cell", "Native" }
                            div { class: "compare-cell", "Native" }
                            div { class: "compare-cell", "Interpreted" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Proofs" }
                            div { class: "compare-cell highlight", "Built-in" }
                            div { class: "compare-cell", "Required" }
                            div { class: "compare-cell", "Optional" }
                            div { class: "compare-cell", "None" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Memory" }
                            div { class: "compare-cell highlight", "Ownership (English)" }
                            div { class: "compare-cell", "GC" }
                            div { class: "compare-cell", "Ownership" }
                            div { class: "compare-cell", "GC" }
                        }
                    }
                }

                section { class: "section", id: "faq",
                    h2 { class: "section-title", "FAQ" }
                    p { class: "section-sub",
                        "Common questions about LOGICAFFEINE."
                    }

                    div { class: "grid2",
                        div { class: "faq-item",
                            div { class: "faq-q", "Is it really free?" }
                            div { class: "faq-a", "Yes — free for individuals. Teams and commercial use should use the licensing options on the Pricing page." }
                        }
                        div { class: "faq-item",
                            div { class: "faq-q", "Do I need to know logic already?" }
                            div { class: "faq-a", "No. Start in Learn. The system introduces concepts progressively and uses examples to teach scope, quantifiers, and structure." }
                        }
                        div { class: "faq-item",
                            div { class: "faq-q", "Is this an AI that \"guesses\"?" }
                            div { class: "faq-a", "The goal is the opposite: to force explicit structure. When language is ambiguous, the tutor prompts clarifying questions." }
                        }
                        div { class: "faq-item",
                            div { class: "faq-q", "Where do I begin?" }
                            div { class: "faq-a", "If you want speed, open Studio. If you want mastery, Start Learning and follow the lessons." }
                        }
                        div { class: "faq-item",
                            div { class: "faq-q", "What is LOGOS written in?" }
                            div { class: "faq-a", "Rust. The entire transpiler, parser, and runtime are written in Rust for maximum performance and safety." }
                        }
                        div { class: "faq-item",
                            div { class: "faq-q", "How fast is it?" }
                            div { class: "faq-a", "Native speed. LOGOS compiles to Rust, which then compiles via LLVM to optimized machine code. Zero interpreter overhead." }
                        }
                    }
                }

                section {
                    class: "section",
                    style: "padding-bottom: 100px;",
                    div {
                        class: "card",
                        style: "padding: 32px; overflow: visible;",
                        h2 { class: "section-title", "Make your reasoning impossible to ignore." }
                        p {
                            class: "section-sub",
                            style: "margin-bottom: 20px;",
                            "Start with the Curriculum, or jump into the Studio. Either way, the product is built to sharpen your mind."
                        }
                        div { class: "hero-ctas",
                            Link { to: Route::Learn {}, class: "btn btn-primary", "Start Learning" }
                            Link { to: Route::Home {}, class: "btn", "Launch App" }
                            Link { to: Route::Pricing {}, class: "btn btn-ghost", "View Licenses" }
                        }
                    }
                }

                footer { class: "footer",
                    div { class: "footer-row",
                        div { "© 2025 Brahmastra Labs LLC  •  Written in Rust 🦀" }
                        div {
                            a {
                                href: "https://github.com/Brahmastra-Labs/logicaffeine",
                                target: "_blank",
                                class: "github-link",
                                svg {
                                    xmlns: "http://www.w3.org/2000/svg",
                                    width: "16",
                                    height: "16",
                                    view_box: "0 0 24 24",
                                    fill: "currentColor",
                                    path {
                                        d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                                    }
                                }
                                "GitHub"
                            }
                            span { "  •  " }
                            Link { to: Route::Privacy {}, "Privacy Policy" }
                            span { "  •  " }
                            Link { to: Route::Terms {}, "Terms of Use" }
                            span { "  •  " }
                            Link { to: Route::Pricing {}, "Pricing" }
                            span { "  •  " }
                            Link { to: Route::Home {}, "App" }
                            span { "  •  " }
                            Link { to: Route::Learn {}, "Learn" }
                        }
                    }
                }
            }
        }
    }
}
