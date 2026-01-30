//! Marketing landing page.
//!
//! The main entry point for new visitors showcasing the LOGOS platform with:
//! - Hero section with animated gradient orbs
//! - Feature highlights for Logic, Code, and Math modes
//! - Call-to-action buttons for learning and the studio
//! - Live interactive demos
//!
//! # Route
//!
//! Accessed via [`Route::Landing`].

use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::components::icon::{Icon, IconVariant, IconSize};
use crate::ui::components::code_editor::{CodeEditor, Language};
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, website_schema, faq_schema, pages as seo_pages};
use crate::ui::state::StudioMode;
use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui, interpret_for_ui, generate_rust_code};
use logicaffeine_kernel::interface::Repl;
use crate::ui::examples::{
    CODE_HELLO, CODE_FIBONACCI, CODE_CRDT_COUNTERS, CODE_CRDT_TALLY,
    LOGIC_LEIBNIZ, LOGIC_BARBER, LOGIC_SIMPLE, LOGIC_QUANTIFIERS,
    MATH_GODEL_LITERATE, MATH_INCOMPLETENESS_LITERATE, MATH_AUTO, MATH_NAT, MATH_BOOL, MATH_PROP_LOGIC,
};

struct DemoExample {
    filename: &'static str,
    icon: &'static str,
    content: &'static str,
    output: &'static str,
    compiled: &'static str,
    studio_path: &'static str,
}

const CODE_DEMO_EXAMPLES: [DemoExample; 4] = [
    DemoExample {
        filename: "hello-world.logos",
        icon: "λ",
        content: CODE_HELLO,
        output: "",
        compiled: "",
        studio_path: "examples/code/hello-world.logos",
    },
    DemoExample {
        filename: "fibonacci.logos",
        icon: "λ",
        content: CODE_FIBONACCI,
        output: "",
        compiled: "",
        studio_path: "examples/code/fibonacci.logos",
    },
    DemoExample {
        filename: "counters.logos",
        icon: "λ",
        content: CODE_CRDT_COUNTERS,
        output: "",
        compiled: "",
        studio_path: "examples/code/distributed/counters.logos",
    },
    DemoExample {
        filename: "tally.logos",
        icon: "λ",
        content: CODE_CRDT_TALLY,
        output: "",
        compiled: "",
        studio_path: "examples/code/distributed/tally.logos",
    },
];

const LOGIC_DEMO_EXAMPLES: [DemoExample; 4] = [
    DemoExample {
        filename: "leibniz-identity.logic",
        icon: "∀",
        content: LOGIC_LEIBNIZ,
        output: "",
        compiled: "",
        studio_path: "examples/logic/leibniz-identity.logic",
    },
    DemoExample {
        filename: "barber-paradox.logic",
        icon: "∀",
        content: LOGIC_BARBER,
        output: "",
        compiled: "",
        studio_path: "examples/logic/barber-paradox.logic",
    },
    DemoExample {
        filename: "simple-sentences.logic",
        icon: "∀",
        content: LOGIC_SIMPLE,
        output: "",
        compiled: "",
        studio_path: "examples/logic/simple-sentences.logic",
    },
    DemoExample {
        filename: "quantifiers.logic",
        icon: "∀",
        content: LOGIC_QUANTIFIERS,
        output: "",
        compiled: "",
        studio_path: "examples/logic/quantifiers.logic",
    },
];

const MATH_DEMO_EXAMPLES: [DemoExample; 6] = [
    DemoExample {
        filename: "godel-literate.logos",
        icon: "π",
        content: MATH_GODEL_LITERATE,
        output: "",
        compiled: "",
        studio_path: "examples/math/godel-literate.logos",
    },
    DemoExample {
        filename: "incompleteness-literate.logos",
        icon: "π",
        content: MATH_INCOMPLETENESS_LITERATE,
        output: "",
        compiled: "",
        studio_path: "examples/math/incompleteness-literate.logos",
    },
    DemoExample {
        filename: "auto-tactic.logos",
        icon: "π",
        content: MATH_AUTO,
        output: "",
        compiled: "",
        studio_path: "examples/math/auto-tactic.logos",
    },
    DemoExample {
        filename: "natural-numbers.logos",
        icon: "π",
        content: MATH_NAT,
        output: "",
        compiled: "",
        studio_path: "examples/math/natural-numbers.logos",
    },
    DemoExample {
        filename: "boolean-logic.logos",
        icon: "π",
        content: MATH_BOOL,
        output: "",
        compiled: "",
        studio_path: "examples/math/boolean-logic.logos",
    },
    DemoExample {
        filename: "prop-logic.logos",
        icon: "π",
        content: MATH_PROP_LOGIC,
        output: "",
        compiled: "",
        studio_path: "examples/math/prop-logic.logos",
    },
];

fn execute_math_code(content: &str) -> (Vec<String>, Option<String>) {
    let mut repl = Repl::new();
    let mut lines = Vec::new();
    let mut error = None;
    let statements = parse_math_statements(content);
    for stmt in statements {
        match repl.execute(&stmt) {
            Ok(output) => {
                if !output.is_empty() {
                    lines.push(output);
                }
            }
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    (lines, error)
}

fn parse_math_statements(code: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with("--") {
            i += 1;
            continue;
        }

        if trimmed.starts_with("## To ") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();
                if next_trimmed.is_empty() || next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let is_continuation = next_trimmed.starts_with("Consider ")
                    || next_trimmed.starts_with("When ")
                    || next_trimmed.starts_with("Yield ");
                if is_indented || is_continuation {
                    block.push(' ');
                    block.push_str(next_trimmed);
                    i += 1;
                } else {
                    break;
                }
            }
            statements.push(block);
            continue;
        }

        if trimmed.starts_with("## Theorem:") {
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();
                if next_trimmed.is_empty() || next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let is_theorem_part = next_trimmed.starts_with("Statement:")
                    || next_trimmed.starts_with("Proof:");
                if is_indented || is_theorem_part {
                    block.push('\n');
                    block.push_str(next_line);
                    i += 1;
                    if next_trimmed.starts_with("Proof:") && next_trimmed.ends_with('.') {
                        break;
                    }
                } else {
                    break;
                }
            }
            statements.push(block);
            continue;
        }

        if (trimmed.starts_with("A ") || trimmed.starts_with("An ")) && trimmed.contains(" is either") {
            if trimmed.ends_with('.') && !trimmed.trim_end_matches('.').ends_with(':') {
                statements.push(trimmed.to_string());
                i += 1;
                continue;
            }
            let mut block = String::new();
            block.push_str(trimmed);
            i += 1;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();
                if next_trimmed.is_empty() || next_trimmed.starts_with("--") {
                    i += 1;
                    continue;
                }
                let is_indented = next_line.starts_with(' ') || next_line.starts_with('\t');
                let looks_like_variant = next_trimmed.starts_with("a ")
                    || next_trimmed.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                if is_indented || (looks_like_variant && !next_trimmed.starts_with("A ") && !next_trimmed.starts_with("An ")) {
                    if !block.ends_with(':') {
                        block.push_str(" or ");
                    } else {
                        block.push(' ');
                    }
                    block.push_str(next_trimmed.trim_end_matches('.'));
                    i += 1;
                } else {
                    break;
                }
            }
            if !block.ends_with('.') {
                block.push('.');
            }
            statements.push(block);
            continue;
        }

        let mut current_stmt = String::new();
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") {
                i += 1;
                continue;
            }
            if !current_stmt.is_empty() {
                current_stmt.push(' ');
            }
            current_stmt.push_str(trimmed);
            i += 1;
            if trimmed.ends_with('.') {
                break;
            }
        }
        if !current_stmt.is_empty() {
            statements.push(current_stmt);
        }
    }

    statements
}

fn examples_for_mode(mode: StudioMode) -> &'static [DemoExample] {
    match mode {
        StudioMode::Code => &CODE_DEMO_EXAMPLES,
        StudioMode::Logic => &LOGIC_DEMO_EXAMPLES,
        StudioMode::Math => &MATH_DEMO_EXAMPLES,
    }
}

const LANDING_STYLE: &str = r#"
body:has(.landing) {
  overflow: hidden;
}

.landing {
  height: 100vh;
  color: var(--text-primary);
  background:
    radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.18), transparent 60%),
    radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.18), transparent 60%),
    radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.10), transparent 62%),
    linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
  overflow-x: hidden;
  overflow-y: auto;
  font-family: var(--font-sans);
  position: relative;
}

.bg-orb {
  position: absolute;
  inset: auto;
  width: 520px;
  height: 520px;
  border-radius: var(--radius-full);
  filter: blur(42px);
  opacity: 0.22;
  pointer-events: none;
  animation: float 14s ease-in-out infinite, pulse-glow 10s ease-in-out infinite;
}
.orb1 { top: -220px; left: -160px; background: radial-gradient(circle at 30% 30%, var(--color-accent-blue), transparent 60%); animation-delay: 0s; }
.orb2 { top: 120px; right: -200px; background: radial-gradient(circle at 40% 35%, var(--color-accent-purple), transparent 60%); animation-delay: -5s; }
.orb3 { bottom: -260px; left: 20%; background: radial-gradient(circle at 40% 35%, rgba(34,197,94,0.9), transparent 60%); animation-delay: -10s; }

.container {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  padding: 0 var(--spacing-xl);
}

/* Navigation now handled by MainNav component */

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: var(--spacing-md) var(--spacing-lg);
  border-radius: var(--radius-lg);
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.05);
  text-decoration: none;
  font-weight: 650;
  font-size: var(--font-body-md);
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
  background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
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
  color: var(--text-primary);
}

.hero {
  padding: 64px 0 30px;
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
  border-radius: var(--radius-full);
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.10);
  backdrop-filter: blur(18px);
  box-shadow: 0 18px 40px rgba(0,0,0,0.25);
  color: var(--text-primary);
  font-size: var(--font-caption-md);
  font-weight: 650;
}
.badge .dot {
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--color-success);
  box-shadow: 0 0 0 6px rgba(34,197,94,0.12);
  animation: pulse-glow 2s ease-in-out infinite;
}

.hero .badge { animation: fadeInUp 0.6s ease both; }
.hero .h-title { animation: fadeInUp 0.6s ease 0.08s both; }
.hero .h-sub { animation: fadeInUp 0.6s ease 0.16s both; }
.hero .hero-ctas { animation: fadeInUp 0.6s ease 0.24s both; }
.hero .microcopy { animation: fadeInUp 0.6s ease 0.30s both; }
.hero .demo { animation: fadeInUp 0.8s ease 0.44s both; }

.h-title {
  margin: 24px 0 var(--spacing-lg);
  font-size: var(--font-display-xl);
  line-height: 1.15;
  letter-spacing: -2px;
  font-weight: 900;
  background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.h-sub {
  margin: 0 0 var(--spacing-xl);
  max-width: 580px;
  color: var(--text-secondary);
  font-size: var(--font-body-lg);
  line-height: 1.65;
}

.hero-ctas {
  display: flex;
  gap: var(--spacing-md);
  flex-wrap: wrap;
  margin: 18px 0 14px;
}

.microcopy {
  font-size: var(--font-caption-md);
  color: var(--text-tertiary);
}

.demo {
  border-radius: var(--radius-xl);
  border: 1px solid rgba(255,255,255,0.10);
  background: linear-gradient(180deg, rgba(255,255,255,0.06), rgba(255,255,255,0.03));
  backdrop-filter: blur(18px);
  box-shadow: 0 30px 80px rgba(0,0,0,0.55);
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
  padding: 14px var(--spacing-lg);
  border-bottom: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.10);
}

.win-dots { display: flex; gap: var(--spacing-sm); align-items: center; }
.wdot { width: 11px; height: 11px; border-radius: var(--radius-full); opacity: 0.9; }
.wr { background: #ef4444; } .wy { background: #fbbf24; } .wg { background: #22c55e; }

.demo-label {
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
  border: 1px solid rgba(255,255,255,0.10);
  padding: 7px 10px;
  border-radius: var(--radius-full);
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
  margin-bottom: var(--spacing-md);
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
}

.pill {
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
  padding: 6px 10px;
  border-radius: var(--radius-full);
}

.code {
  font-family: var(--font-mono);
  font-size: var(--font-caption-md);
  line-height: 1.6;
  color: var(--text-primary);
  white-space: pre-wrap;
}

.code.logic { color: var(--color-accent-purple); }

.demo-foot {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  padding: 14px var(--spacing-lg);
  border-top: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.12);
  color: var(--text-secondary);
  font-size: var(--font-caption-md);
}

.section {
  padding: 74px 0;
}

.section-title {
  font-size: var(--font-display-md);
  letter-spacing: -1.2px;
  margin: 0 0 14px;
  font-weight: 800;
}
.section-sub {
  margin: 0 0 var(--spacing-xl);
  color: var(--text-secondary);
  line-height: 1.65;
  max-width: 760px;
}
.section-right .section-title,
.section-right .section-sub {
  text-align: right;
}
.section-right .section-sub {
  margin-left: auto;
}
.section-center .section-title,
.section-center .section-sub {
  text-align: center;
}
.section-center .section-sub {
  margin-left: auto;
  margin-right: auto;
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
  border-radius: var(--radius-xl);
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
  border-radius: var(--radius-xl);
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

.icon-box {
  width: 48px; height: 48px;
  border-radius: var(--radius-lg);
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.10);
  margin-bottom: var(--spacing-md);
}

.icon-box .icon {
  width: 24px;
  height: 24px;
}

.card h3 {
  margin: 0 0 var(--spacing-sm);
  font-size: var(--font-body-md);
  letter-spacing: -0.2px;
}
.card p {
  margin: 0;
  color: var(--text-secondary);
  line-height: 1.6;
  font-size: var(--font-body-md);
}

.quote {
  font-size: var(--font-body-md);
  line-height: 1.65;
  color: var(--text-primary);
}
.quoter {
  margin-top: 10px;
  color: var(--text-tertiary);
  font-size: var(--font-caption-md);
}


.tech-stack {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 14px;
}

.tech-badge {
  font-size: var(--font-caption-sm);
  padding: 6px var(--spacing-md);
  border-radius: 6px;
  background: rgba(255,255,255,0.03);
  border: 1px solid rgba(255,255,255,0.08);
  color: var(--text-secondary);
}

.tech-badge.rust {
  background: linear-gradient(135deg, rgba(183,65,14,0.15), rgba(222,165,132,0.10));
  border-color: rgba(222,165,132,0.3);
  color: #dea584;
}

.hello-world-layout {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 40px;
}

.hello-pill-wrap,
.hello-cta-wrap {
  text-align: center;
}

.hello-editor {
  width: 100%;
  max-width: 820px;
  border-radius: var(--radius-lg);
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(0,0,0,0.3);
  overflow: hidden;
  backdrop-filter: blur(8px);
}

.hello-editor-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  background: rgba(255,255,255,0.03);
  border-bottom: 1px solid rgba(255,255,255,0.06);
}

.hello-editor-head .hello-filename {
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
  font-family: var(--font-mono);
}

.hello-run-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 14px;
  border-radius: var(--radius-sm);
  border: 1px solid rgba(52,211,153,0.4);
  background: rgba(52,211,153,0.10);
  color: #34d399;
  font-size: var(--font-caption-sm);
  font-weight: 600;
  cursor: pointer;
  transition: background 0.2s, border-color 0.2s;
}

.hello-run-btn:hover {
  background: rgba(52,211,153,0.20);
  border-color: rgba(52,211,153,0.6);
}

.hello-run-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.hello-editor-body {
  display: flex;
  min-height: 260px;
}

.hello-editor-left {
  flex: 1;
  min-width: 0;
}

.hello-editor-right {
  flex: 1;
  min-width: 0;
  border-left: 1px solid rgba(255,255,255,0.06);
  display: flex;
  flex-direction: column;
}

.hello-output-head {
  padding: 8px 14px;
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
  font-family: var(--font-mono);
  background: rgba(255,255,255,0.02);
  border-bottom: 1px solid rgba(255,255,255,0.04);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.hello-output-body {
  padding: var(--spacing-md);
  flex: 1;
  overflow-y: auto;
  font-family: var(--font-mono);
  font-size: var(--font-body-md);
  line-height: 1.6;
}

.hello-output-line {
  margin: 0;
  white-space: pre-wrap;
  color: var(--text-primary);
}

.hello-output-error {
  margin: 0;
  white-space: pre-wrap;
  color: var(--color-error, #f87171);
}

.hello-output-loading {
  color: var(--text-secondary);
  font-style: italic;
}

.hello-output-empty {
  color: var(--text-secondary);
  opacity: 0.5;
}

@media (max-width: 700px) {
  .hello-editor-body {
    flex-direction: column;
  }
  .hello-editor-right {
    border-left: none;
    border-top: 1px solid rgba(255,255,255,0.06);
  }
}

.hello-note {
  text-align: center;
  font-size: var(--font-body-md);
  color: var(--text-secondary);
  display: inline-block;
  padding: 8px 20px;
  border: 1px solid rgba(167,139,250,0.3);
  border-radius: var(--radius-full);
  background: rgba(167,139,250,0.06);
  box-shadow: 0 0 20px rgba(167,139,250,0.12), 0 0 40px rgba(96,165,250,0.08);
}

.compare-table {
  display: flex;
  flex-direction: column;
  border-radius: var(--radius-lg);
  border: 1px solid rgba(255,255,255,0.10);
  overflow: hidden;
  max-width: 800px;
  margin: 0 auto;
}

.compare-row {
  display: grid;
  grid-template-columns: 1.2fr repeat(5, 1fr);
}

.compare-row.header {
  background: rgba(255,255,255,0.05);
  font-weight: 600;
  font-size: var(--font-caption-md);
}

.compare-row:not(.header) {
  border-top: 1px solid rgba(255,255,255,0.06);
}

.compare-cell {
  padding: var(--spacing-md) 14px;
  font-size: var(--font-caption-md);
  color: var(--text-secondary);
  text-align: center;
}

.compare-cell.label {
  text-align: left;
  color: var(--text-primary);
  font-weight: 500;
}

.compare-cell.highlight {
  background: rgba(167,139,250,0.08);
  color: var(--color-accent-purple);
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
  .compare-cell:nth-child(5),
  .compare-cell:nth-child(6) {
    display: none;
  }
}

.faq-item {
  padding: var(--spacing-lg) var(--spacing-lg) 14px;
  border-radius: var(--radius-xl);
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.03);
}
.faq-q { font-weight: 750; margin-bottom: var(--spacing-sm); }
.faq-a { color: var(--text-secondary); line-height: 1.6; font-size: var(--font-body-md); }

.footer {
  padding: 34px 0 44px;
  border-top: 1px solid rgba(255,255,255,0.06);
  color: var(--text-tertiary);
  font-size: var(--font-caption-md);
}

.footer-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-lg);
  flex-wrap: wrap;
}

@media (max-width: 980px) {
  .hero-grid { grid-template-columns: 1fr; }
  .demo-body { grid-template-columns: 1fr; }
  .demo-col + .demo-col { border-left: none; border-top: 1px solid rgba(255,255,255,0.06); }
  .grid3 { grid-template-columns: 1fr; }
  .grid2 { grid-template-columns: 1fr; }
  .h-title { font-size: var(--font-display-lg); }
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
  gap: var(--spacing-xl);
  flex-wrap: wrap;
}

.step {
  flex: 1;
  min-width: 200px;
  max-width: 280px;
  text-align: center;
  padding: var(--spacing-xl);
  border-radius: var(--radius-xl);
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
  margin: 0 auto var(--spacing-lg);
  border-radius: 50%;
  background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
  color: #060814;
  font-weight: 800;
  font-size: var(--font-heading-sm);
  display: grid;
  place-items: center;
}

.step h3 {
  margin: 0 0 var(--spacing-sm);
  font-size: var(--font-body-lg);
}

.step p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.5;
}

.step-arrow {
  font-size: 24px;
  color: var(--text-tertiary);
}

.grid3 .card:nth-child(1) .icon-box { background: rgba(0,212,255,0.15); }
.grid3 .card:nth-child(2) .icon-box { background: rgba(129,140,248,0.15); }
.grid3 .card:nth-child(3) .icon-box { background: rgba(34,197,94,0.15); }
.grid3 .card:nth-child(4) .icon-box { background: rgba(251,191,36,0.15); }
.grid3 .card:nth-child(5) .icon-box { background: rgba(236,72,153,0.15); }
.grid3 .card:nth-child(6) .icon-box { background: rgba(129,140,248,0.15); }

/* Mini-Studio */
.mini-studio {
  border-radius: var(--radius-xl);
  border: 1px solid rgba(255,255,255,0.10);
  background: linear-gradient(180deg, rgba(255,255,255,0.06), rgba(255,255,255,0.03));
  backdrop-filter: blur(18px);
  box-shadow: 0 30px 80px rgba(0,0,0,0.55);
  overflow: hidden;
  position: relative;
  display: flex;
  flex-direction: column;
  height: 635px;
}
.mini-studio::before {
  content: "";
  position: absolute;
  inset: -2px;
  background: radial-gradient(600px 280px at 10% 10%, rgba(96,165,250,0.22), transparent 55%),
              radial-gradient(520px 240px at 90% 20%, rgba(167,139,250,0.22), transparent 55%);
  opacity: 0.9;
  pointer-events: none;
}
.mini-studio-head {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 14px var(--spacing-lg);
  border-bottom: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.10);
}
.mini-studio-head .win-dots {
  position: absolute;
  left: var(--spacing-lg);
}
.mini-mode-toggle {
  display: flex;
  gap: 4px;
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-full);
  padding: 3px;
}
.mini-toggle-btn {
  padding: 6px 12px;
  border-radius: var(--radius-full);
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-caption-sm);
  font-weight: 600;
  cursor: pointer;
  transition: background 0.18s, color 0.18s;
  display: flex;
  align-items: center;
  gap: 6px;
  font-family: inherit;
}
.mini-toggle-btn.active {
  background: rgba(96,165,250,0.2);
  color: var(--text-primary);
}
.mini-toggle-btn:hover:not(.active) {
  background: rgba(255,255,255,0.06);
}
.mini-studio-body {
  position: relative;
  display: grid;
  grid-template-columns: 180px 1fr;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.mini-file-tabs {
  display: none;
  gap: 4px;
  padding: 8px 12px;
  overflow-x: auto;
  border-bottom: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.08);
  flex-shrink: 0;
  -webkit-overflow-scrolling: touch;
}
.mini-file-tab {
  flex-shrink: 0;
  padding: 5px 10px;
  border-radius: var(--radius-full);
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
  color: var(--text-secondary);
  font-size: var(--font-caption-sm);
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  font-family: var(--font-mono);
  transition: background 0.15s, color 0.15s, border-color 0.15s;
}
.mini-file-tab:hover {
  background: rgba(255,255,255,0.08);
}
.mini-file-tab.active {
  background: rgba(96,165,250,0.2);
  border-color: rgba(96,165,250,0.35);
  color: var(--text-primary);
}
.mini-explorer {
  border-right: 1px solid rgba(255,255,255,0.06);
  padding: 8px 0;
  background: rgba(0,0,0,0.08);
}
.mini-explorer-label {
  padding: 7px 14px;
  font-size: var(--font-caption-sm);
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  font-weight: 600;
}
.mini-file-item {
  padding: 7px 14px;
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 8px;
  transition: background 0.15s, color 0.15s;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mini-file-item:hover {
  background: rgba(255,255,255,0.04);
}
.mini-file-item.active {
  background: rgba(96,165,250,0.15);
  color: var(--text-primary);
}
.mini-file-item.view-more {
  color: var(--accent-secondary);
  font-style: italic;
  opacity: 0.8;
  margin-top: 4px;
  text-decoration: none;
}
.mini-file-item.view-more:hover {
  opacity: 1;
  background: rgba(96,165,250,0.10);
}
.mini-file-tab.view-more {
  color: var(--accent-secondary);
  font-style: italic;
  border-color: rgba(96,165,250,0.2);
  text-decoration: none;
}
.mini-file-tab.view-more:hover {
  background: rgba(96,165,250,0.10);
}
.mini-file-icon {
  opacity: 0.5;
}
.mini-code-panel {
  padding: 16px;
  overflow-y: auto;
  position: relative;
}
.mini-code-filename {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  font-size: var(--font-caption-sm);
  color: var(--text-secondary);
}
.mini-code-panel .code-editor {
  height: 100%;
  background: transparent;
}
.mini-code-panel .code-editor-input {
  min-height: 0;
}
.mini-code-panel .code-editor-textarea,
.mini-code-panel .code-editor-highlight {
  padding: 0;
  padding-bottom: 40px;
  font-size: var(--font-caption-md);
  line-height: 1.6;
}
.mini-action-bar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 8px 16px;
  border-bottom: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.06);
  position: relative;
  flex-shrink: 0;
}
.mini-exec-btn {
  padding: 6px 14px;
  border: none;
  border-radius: 6px;
  color: white;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
  display: flex;
  align-items: center;
  gap: 6px;
  font-family: inherit;
}
.mini-exec-btn:hover {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(102, 126, 234, 0.3);
}
.mini-exec-btn:active { transform: translateY(0); }
.mini-exec-btn.compile {
  background: linear-gradient(135deg, #56b6c2 0%, #61afef 100%);
}
.mini-exec-btn.run {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}
.mini-terminal {
  border-top: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.20);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  position: relative;
}
.mini-terminal-head {
  display: flex;
  align-items: center;
  padding: 6px 16px;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-tertiary);
  font-weight: 600;
  border-bottom: 1px solid rgba(255,255,255,0.04);
  flex-shrink: 0;
}
.mini-terminal-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 16px 13px;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.5;
}
.mini-output-line {
  margin: 0;
  color: #4ade80;
  white-space: pre-wrap;
  font: inherit;
  line-height: inherit;
}
.mini-output-error {
  margin: 4px 0 0;
  color: #e06c75;
  white-space: pre-wrap;
  font: inherit;
  line-height: inherit;
  padding: 8px;
  background: rgba(224, 108, 117, 0.1);
  border-radius: 4px;
}
.mini-output-empty {
  color: var(--text-tertiary);
  font-style: italic;
}
.mini-output-loading {
  color: #667eea;
  animation: blink 1s step-end infinite;
}
.mini-term-output {
  margin: 0;
  color: #4ade80;
  white-space: pre-wrap;
  font: inherit;
  line-height: inherit;
}
.mini-compiled {
  border-top: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.25);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  position: relative;
}
.mini-compiled-head {
  display: flex;
  align-items: center;
  padding: 6px 16px;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-tertiary);
  font-weight: 600;
  border-bottom: 1px solid rgba(255,255,255,0.04);
  flex-shrink: 0;
}
.mini-compiled-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 16px;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.5;
  color: #e5c07b;
  white-space: pre-wrap;
  margin: 0;
}
.mini-terminal-resizer {
  height: 16px;
  background: transparent;
  cursor: row-resize;
  position: relative;
  flex-shrink: 0;
  z-index: 2;
}
.mini-terminal-resizer::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 40px;
  height: 5px;
  border-radius: 3px;
  background: rgba(255,255,255,0.15);
  transition: background 0.15s ease;
}
.mini-terminal-resizer:hover::after,
.mini-terminal-resizer.active::after {
  background: rgba(96,165,250,0.6);
}
.mini-studio-cta {
  position: relative;
  padding: 12px 16px;
  border-top: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.12);
  text-align: center;
  flex-shrink: 0;
}
.mini-cta-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 10px 24px;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  border: none;
  border-radius: 8px;
  color: white;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.18s ease;
  text-decoration: none;
  font-family: inherit;
}
.mini-cta-btn:hover {
  transform: translateY(-1px);
  box-shadow: 0 6px 20px rgba(59, 130, 246, 0.35);
}

/* Mode Stories */
.mode-stories {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 18px;
}
.mode-story {
  border-radius: var(--radius-xl);
  border: 1px solid rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
  backdrop-filter: blur(18px);
  padding: 24px;
  transition: transform 0.18s ease, border-color 0.18s ease, background 0.18s ease;
  overflow: hidden;
}
.mode-story:hover {
  transform: translateY(-3px);
  border-color: rgba(167,139,250,0.28);
  background: rgba(255,255,255,0.06);
}
.mode-story-icon {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-lg);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
  font-weight: 700;
  margin-bottom: var(--spacing-md);
  border: 1px solid rgba(255,255,255,0.10);
}
.mode-story:nth-child(1) .mode-story-icon { background: rgba(96,165,250,0.15); color: #60a5fa; }
.mode-story:nth-child(2) .mode-story-icon { background: rgba(167,139,250,0.15); color: #a78bfa; }
.mode-story:nth-child(3) .mode-story-icon { background: rgba(34,197,94,0.15); color: #22c55e; }
.mode-story h3 {
  margin: 0 0 var(--spacing-md);
  font-size: var(--font-body-lg);
}
.mode-story-demo {
  border-radius: var(--radius-lg);
  background: rgba(0,0,0,0.3);
  padding: 12px;
  margin-bottom: var(--spacing-md);
  font-family: var(--font-mono);
  font-size: var(--font-caption-md);
  line-height: 1.6;
}
.mode-story-row {
  display: flex;
  gap: 8px;
  align-items: baseline;
}
.mode-story-label {
  color: var(--text-tertiary);
  font-size: var(--font-caption-sm);
  min-width: 54px;
  flex-shrink: 0;
}
.mode-story-value {
  color: var(--text-primary);
}
.mode-story-value.logic {
  color: var(--color-accent-purple);
}
.mode-story-value.success {
  color: var(--color-success);
}
.mode-story-arrow {
  text-align: center;
  color: var(--text-tertiary);
  padding: 4px 0;
  font-size: var(--font-caption-sm);
}
.mode-story > p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.6;
}

/* Security Demo */
.security-demo {
  margin-top: var(--spacing-xl);
  border-radius: var(--radius-xl);
  border: 1px solid rgba(167,139,250,0.20);
  background: linear-gradient(180deg, rgba(255,255,255,0.06), rgba(255,255,255,0.02));
  overflow: hidden;
  box-shadow:
    0 0 40px rgba(167,139,250,0.08),
    0 0 80px rgba(96,165,250,0.06),
    0 20px 60px rgba(0,0,0,0.4);
  position: relative;
}
.security-demo::before {
  content: "";
  position: absolute;
  inset: -1px;
  border-radius: var(--radius-xl);
  background: radial-gradient(400px 200px at 20% 0%, rgba(167,139,250,0.15), transparent 60%),
              radial-gradient(400px 200px at 80% 100%, rgba(96,165,250,0.12), transparent 60%);
  pointer-events: none;
}
.security-demo-head {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px var(--spacing-lg);
  border-bottom: 1px solid rgba(255,255,255,0.08);
  background: rgba(0,0,0,0.12);
  font-size: var(--font-body-md);
  font-weight: 700;
  color: var(--text-primary);
  letter-spacing: -0.2px;
}
.security-demo-body {
  position: relative;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: stretch;
}
.security-demo-col {
  padding: 20px 24px;
}
.security-demo-col + .security-demo-col {
  border-left: 1px solid rgba(255,255,255,0.06);
}
.security-demo-arrow {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 16px;
  font-size: 28px;
  color: var(--color-accent-purple);
  border-left: 1px solid rgba(255,255,255,0.06);
  border-right: 1px solid rgba(255,255,255,0.06);
  background: rgba(0,0,0,0.06);
  text-shadow: 0 0 12px rgba(167,139,250,0.5);
}
.security-demo-label {
  display: inline-block;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  font-weight: 600;
  color: #4ade80;
  margin-bottom: var(--spacing-sm);
  padding: 4px 10px;
  border: 1px solid rgba(74,222,128,0.3);
  border-radius: var(--radius-full);
  background: rgba(74,222,128,0.08);
}

@media (max-width: 980px) {
  .hero-grid { grid-template-columns: 1fr; }
  .demo-body { grid-template-columns: 1fr; }
  .demo-col + .demo-col { border-left: none; border-top: 1px solid rgba(255,255,255,0.06); }
  .grid3 { grid-template-columns: 1fr; }
  .grid2 { grid-template-columns: 1fr; }
  .h-title { font-size: var(--font-display-lg); }
  .step-arrow { display: none; }
  .steps { flex-direction: column; }
  .mini-studio { height: 89vh !important; }
  .mini-studio-body { grid-template-columns: 1fr; }
  .mini-explorer { display: none; }
  .mini-file-tabs { display: flex; }
  .mini-terminal { max-height: 35vh !important; }
  .mini-compiled { max-height: 30vh !important; }
  .mode-stories { grid-template-columns: 1fr; }
  .security-demo-body { grid-template-columns: 1fr; }
  .security-demo-arrow { display: none; }
  .security-demo-col + .security-demo-col { border-left: none; border-top: 1px solid rgba(255,255,255,0.06); }
}

@media (max-width: 768px) {
  .mini-studio { height: 89vh !important; }
  .mini-studio-head {
    justify-content: flex-end;
  }
  .mini-studio-head .win-dots {
    display: none;
  }
  .mini-exec-btn {
    padding: 10px 14px;
    min-height: 44px;
    font-size: 12px;
  }
  .mini-cta-btn {
    min-height: 44px;
    width: 100%;
    justify-content: center;
  }
  .mini-action-bar { padding: 8px 12px; }
}

@media (max-width: 480px) {
  .mini-studio { height: 89vh !important; }
  .mini-exec-btn .btn-label { display: none; }
}

@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
}
"#;

#[component]
pub fn Landing() -> Element {
    let schemas = vec![
        organization_schema(),
        website_schema(),
        faq_schema(&[
            ("Is it really free?", "Yes — free for individuals. Teams and commercial use should use the licensing options on the Pricing page."),
            ("Do I need to know logic already?", "No. Start in Learn. The system introduces concepts progressively and uses examples to teach scope, quantifiers, and structure."),
            ("Is this an AI that guesses?", "The goal is the opposite: to force explicit structure. When language is ambiguous, the tutor prompts clarifying questions."),
            ("Where do I begin?", "If you want speed, open Studio. If you want mastery, Start Learning and follow the lessons."),
            ("What is LOGOS written in?", "Rust. The entire transpiler, parser, and runtime are written in Rust for maximum performance and safety."),
            ("How fast is it?", "Native speed. LOGOS compiles to Rust, which then compiles via LLVM to optimized machine code. Zero interpreter overhead."),
        ]),
    ];

    let mut demo_mode = use_signal(|| StudioMode::Code);
    let mut active_index = use_signal(|| 0usize);
    let mut cycling_paused = use_signal(|| false);
    let mut timer_started = use_signal(|| false);
    let mut terminal_height = use_signal(|| 135.0f64);
    let mut resizing_terminal = use_signal(|| false);
    let mut compiled_height = use_signal(|| 140.0f64);
    let mut show_compiled = use_signal(|| false);
    let mut resizing_compiled = use_signal(|| false);
    let mut code_content = use_signal(|| String::new());
    let mut output_lines = use_signal(|| Vec::<String>::new());
    let mut output_error = use_signal(|| Option::<String>::None);
    let mut compiled_output = use_signal(|| String::new());
    let mut is_running = use_signal(|| false);

    let mut hello_code = use_signal(|| CODE_DEMO_EXAMPLES[0].content.to_string());
    let mut hello_output = use_signal(|| CODE_DEMO_EXAMPLES[0].output.lines().map(|l| l.to_string()).collect::<Vec<_>>());
    let mut hello_error = use_signal(|| Option::<String>::None);
    let mut hello_running = use_signal(|| false);

    use_effect(move || {
        let mode = *demo_mode.read();
        let idx = *active_index.read();
        let examples = examples_for_mode(mode);
        let clamped = idx.min(examples.len().saturating_sub(1));
        let ex = &examples[clamped];
        let content = ex.content.to_string();
        code_content.set(content.clone());
        output_error.set(None);
        compiled_output.set(ex.compiled.to_string());

        match mode {
            StudioMode::Code => {
                output_lines.set(Vec::new());
                spawn(async move {
                    let result = interpret_for_ui(&content).await;
                    output_lines.set(result.lines);
                    output_error.set(result.error);
                });
            }
            StudioMode::Logic => {
                let mut lines: Vec<String> = Vec::new();

                if content.contains("## Theorem:") {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if let Some(sentence) = trimmed.strip_prefix("Given:") {
                            let fol = compile_for_ui(sentence.trim());
                            if let Some(logic) = fol.logic {
                                lines.push(logic);
                            }
                        } else if let Some(sentence) = trimmed.strip_prefix("Prove:") {
                            let fol = compile_for_ui(sentence.trim());
                            if let Some(logic) = fol.logic {
                                lines.push(format!("Goal: {}", logic));
                            }
                        }
                    }

                    let theorem_result = compile_theorem_for_ui(&content);
                    if let Some(ref err) = theorem_result.error {
                        output_error.set(Some(err.clone()));
                    } else {
                        lines.push(String::new());
                        if theorem_result.derivation.is_some() {
                            lines.push(format!("Theorem: {} ✓", theorem_result.name));
                        } else {
                            lines.push(format!("Theorem: {} — not proved", theorem_result.name));
                        }
                    }
                } else {
                    let result = compile_for_ui(&content);
                    if let Some(logic) = result.logic {
                        for line in logic.lines() {
                            lines.push(line.to_string());
                        }
                    }
                    if let Some(ref err) = result.error {
                        output_error.set(Some(err.clone()));
                    }
                }

                output_lines.set(lines);
            }
            StudioMode::Math => {
                let (lines, err) = execute_math_code(&content);
                output_lines.set(lines);
                output_error.set(err);
            }
        }
    });

    use_effect(move || {
        if *timer_started.read() { return; }
        timer_started.set(true);
        #[cfg(target_arch = "wasm32")]
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(7_000).await;
                if !*cycling_paused.read() {
                    let mode = *demo_mode.read();
                    let count = examples_for_mode(mode).len();
                    let next = (*active_index.read() + 1) % count;
                    active_index.set(next);
                }
            }
        });
    });

    rsx! {
        PageHead {
            title: seo_pages::LANDING.title,
            description: seo_pages::LANDING.description,
            canonical_path: seo_pages::LANDING.canonical_path,
        }
        style { "{LANDING_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "landing",
            div { class: "bg-orb orb1" }
            div { class: "bg-orb orb2" }
            div { class: "bg-orb orb3" }

            MainNav { active: ActivePage::Other }

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
                                "Write Code, Logic, and Math in plain English. LOGOS compiles your words into programs, proofs, and formal systems — no symbols required."
                            }

                            div { class: "hero-ctas",
                                Link { to: Route::Learn {}, class: "btn btn-primary", "Start Learning" }
                                Link { to: Route::Studio {}, class: "btn", "Open Studio" }
                                Link { to: Route::Pricing {}, class: "btn btn-ghost", "See Pricing" }
                            }

                            p { class: "microcopy",
                                "Students, engineers, researchers, and attorneys — anyone who thinks for a living."
                            }

                            div { class: "tech-stack",
                                span { class: "tech-badge rust",
                                    "Rust-Powered 🦀"
                                }
                                span { class: "tech-badge", "WASM Ready" }
                                span { class: "tech-badge", "Markdown Source" }
                                span { class: "tech-badge", "Proof-Checked" }
                            }
                        }

                        div {
                            class: "mini-studio",
                            id: "product",
                            style: if *resizing_terminal.read() || *resizing_compiled.read() { "user-select: none;" } else { "" },
                            onmouseenter: move |_| { cycling_paused.set(true); },
                            onmouseleave: move |_| {
                                cycling_paused.set(false);
                                resizing_terminal.set(false);
                                resizing_compiled.set(false);
                            },
                            onmousemove: move |evt| {
                                let window = web_sys::window().unwrap();
                                let document = window.document().unwrap();
                                let cta_height: f64 = 45.0;
                                if *resizing_terminal.read() {
                                    if let Some(el) = document.get_element_by_id("product") {
                                        let rect = el.get_bounding_client_rect();
                                        let coords = evt.data().client_coordinates();
                                        let client_y: f64 = coords.y;
                                        let below = cta_height
                                            + if *show_compiled.read() { *compiled_height.read() + 6.0 } else { 0.0 };
                                        let new_height = rect.bottom() - client_y - below;
                                        terminal_height.set(new_height.clamp(60.0, 300.0));
                                    }
                                } else if *resizing_compiled.read() {
                                    if let Some(el) = document.get_element_by_id("product") {
                                        let rect = el.get_bounding_client_rect();
                                        let coords = evt.data().client_coordinates();
                                        let client_y: f64 = coords.y;
                                        let new_height = rect.bottom() - client_y - cta_height;
                                        let max_compiled = rect.height() - *terminal_height.read() - cta_height - 6.0 - 120.0;
                                        compiled_height.set(new_height.clamp(60.0, max_compiled.max(60.0)));
                                    }
                                }
                            },
                            onmouseup: move |_| {
                                resizing_terminal.set(false);
                                resizing_compiled.set(false);
                            },
                            ontouchmove: move |evt| {
                                let window = web_sys::window().unwrap();
                                let document = window.document().unwrap();
                                let cta_height: f64 = 45.0;
                                if *resizing_terminal.read() || *resizing_compiled.read() {
                                    evt.prevent_default();
                                    let touches = evt.data().touches();
                                    if let Some(touch) = touches.first() {
                                        if let Some(el) = document.get_element_by_id("product") {
                                            let rect = el.get_bounding_client_rect();
                                            let coords = touch.client_coordinates();
                                            let client_y: f64 = coords.y;
                                            if *resizing_terminal.read() {
                                                let below = cta_height
                                                    + if *show_compiled.read() { *compiled_height.read() + 6.0 } else { 0.0 };
                                                let new_height = rect.bottom() - client_y - below;
                                                terminal_height.set(new_height.clamp(60.0, 300.0));
                                            } else {
                                                let new_height = rect.bottom() - client_y - cta_height;
                                                let max_compiled = rect.height() - *terminal_height.read() - cta_height - 6.0 - 120.0;
                                                compiled_height.set(new_height.clamp(60.0, max_compiled.max(60.0)));
                                            }
                                        }
                                    }
                                }
                            },
                            ontouchend: move |_| {
                                resizing_terminal.set(false);
                                resizing_compiled.set(false);
                            },

                            div { class: "mini-studio-head",
                                div { class: "win-dots",
                                    div { class: "wdot wr" }
                                    div { class: "wdot wy" }
                                    div { class: "wdot wg" }
                                }
                                div { class: "mini-mode-toggle",
                                    button {
                                        class: if *demo_mode.read() == StudioMode::Code { "mini-toggle-btn active" } else { "mini-toggle-btn" },
                                        onclick: move |_| {
                                            demo_mode.set(StudioMode::Code);
                                            active_index.set(0);
                                        },
                                        span { "λ" }
                                        span { class: "mini-toggle-label", "Code" }
                                    }
                                    button {
                                        class: if *demo_mode.read() == StudioMode::Logic { "mini-toggle-btn active" } else { "mini-toggle-btn" },
                                        onclick: move |_| {
                                            demo_mode.set(StudioMode::Logic);
                                            active_index.set(0);
                                            show_compiled.set(false);
                                        },
                                        span { "∀" }
                                        span { class: "mini-toggle-label", "Logic" }
                                    }
                                    button {
                                        class: if *demo_mode.read() == StudioMode::Math { "mini-toggle-btn active" } else { "mini-toggle-btn" },
                                        onclick: move |_| {
                                            demo_mode.set(StudioMode::Math);
                                            active_index.set(0);
                                            show_compiled.set(false);
                                        },
                                        span { "π" }
                                        span { class: "mini-toggle-label", "Math" }
                                    }
                                }
                            }

                            div { class: "mini-action-bar",
                                if *demo_mode.read() == StudioMode::Code {
                                    button {
                                        class: "mini-exec-btn compile",
                                        onclick: move |_| {
                                            let code = code_content.read().clone();

                                            match generate_rust_code(&code) {
                                                Ok(rust) => compiled_output.set(rust),
                                                Err(e) => compiled_output.set(format!("// Compile error: {:?}", e)),
                                            }

                                            let current = *show_compiled.read();
                                            if !current {
                                                show_compiled.set(true);
                                            }
                                        },
                                        "🦀"
                                        span { class: "btn-label", "Compile to Rust" }
                                    }
                                }
                                button {
                                    class: "mini-exec-btn run",
                                    onclick: move |_| {
                                        let code = code_content.read().clone();
                                        let mode = *demo_mode.read();
                                        is_running.set(true);
                                        output_lines.set(Vec::new());
                                        output_error.set(None);

                                        match mode {
                                            StudioMode::Code => {
                                                spawn(async move {
                                                    let result = interpret_for_ui(&code).await;
                                                    output_lines.set(result.lines);
                                                    output_error.set(result.error);
                                                    is_running.set(false);
                                                });
                                            }
                                            StudioMode::Logic => {
                                                let mut lines: Vec<String> = Vec::new();

                                                if code.contains("## Theorem:") {
                                                    // FOL transpilation: compile each premise sentence individually
                                                    for line in code.lines() {
                                                        let trimmed = line.trim();
                                                        if let Some(sentence) = trimmed.strip_prefix("Given:") {
                                                            let sentence = sentence.trim();
                                                            let fol = compile_for_ui(sentence);
                                                            if let Some(logic) = fol.logic {
                                                                lines.push(logic);
                                                            }
                                                        } else if let Some(sentence) = trimmed.strip_prefix("Prove:") {
                                                            let sentence = sentence.trim();
                                                            let fol = compile_for_ui(sentence);
                                                            if let Some(logic) = fol.logic {
                                                                lines.push(format!("Goal: {}", logic));
                                                            }
                                                        }
                                                    }

                                                    // Proof verification
                                                    let theorem_result = compile_theorem_for_ui(&code);
                                                    if let Some(ref err) = theorem_result.error {
                                                        output_error.set(Some(err.clone()));
                                                    } else {
                                                        lines.push(String::new());
                                                        let proved = theorem_result.derivation.is_some();
                                                        if proved {
                                                            lines.push(format!("Theorem: {} ✓", theorem_result.name));
                                                        } else {
                                                            lines.push(format!("Theorem: {} — not proved", theorem_result.name));
                                                        }
                                                    }
                                                } else {
                                                    let result = compile_for_ui(&code);
                                                    if let Some(logic) = result.logic {
                                                        for line in logic.lines() {
                                                            lines.push(line.to_string());
                                                        }
                                                    }
                                                    if let Some(ref err) = result.error {
                                                        output_error.set(Some(err.clone()));
                                                    }
                                                }

                                                output_lines.set(lines);
                                                is_running.set(false);
                                            }
                                            StudioMode::Math => {
                                                let (lines, err) = execute_math_code(&code);
                                                output_lines.set(lines);
                                                output_error.set(err);
                                                is_running.set(false);
                                            }
                                        }
                                    },
                                    "▶"
                                    span { class: "btn-label",
                                        if *demo_mode.read() == StudioMode::Code { "Run" } else { "Execute" }
                                    }
                                }
                            }

                            div { class: "mini-file-tabs",
                                for i in 0..examples_for_mode(*demo_mode.read()).len() {
                                    button {
                                        key: "{i}",
                                        class: if *active_index.read() == i { "mini-file-tab active" } else { "mini-file-tab" },
                                        onclick: move |_| {
                                            active_index.set(i);
                                            cycling_paused.set(true);
                                        },
                                        "{examples_for_mode(*demo_mode.read())[i].filename}"
                                    }
                                }
                                a {
                                    class: "mini-file-tab view-more",
                                    href: "/studio",
                                    "View more..."
                                }
                            }

                            div { class: "mini-studio-body",
                                div { class: "mini-explorer",
                                    div { class: "mini-explorer-label", "FILES" }
                                    for i in 0..examples_for_mode(*demo_mode.read()).len() {
                                        div {
                                            key: "{i}",
                                            class: if *active_index.read() == i { "mini-file-item active" } else { "mini-file-item" },
                                            onclick: move |_| {
                                                active_index.set(i);
                                                cycling_paused.set(true);
                                            },
                                            span { class: "mini-file-icon", "●" }
                                            span { "{examples_for_mode(*demo_mode.read())[i].filename}" }
                                        }
                                    }
                                    a {
                                        class: "mini-file-item view-more",
                                        href: "/studio",
                                        "View more..."
                                    }
                                }
                                div { class: "mini-code-panel",
                                    {
                                        let mode = *demo_mode.read();
                                        let examples = examples_for_mode(mode);
                                        let idx = (*active_index.read()).min(examples.len().saturating_sub(1));
                                        let ex = &examples[idx];
                                        rsx! {
                                            div { class: "mini-code-filename",
                                                span { "{ex.filename}" }
                                                span { "  {ex.icon}" }
                                            }
                                            CodeEditor {
                                                value: code_content.read().clone(),
                                                on_change: move |v: String| code_content.set(v),
                                                language: match mode {
                                                    StudioMode::Code => Language::Logos,
                                                    StudioMode::Logic => Language::Logos,
                                                    StudioMode::Math => Language::Vernacular,
                                                },
                                                placeholder: "Enter code...".to_string(),
                                            }
                                        }
                                    }
                                }
                            }

                            div {
                                class: if *resizing_terminal.read() { "mini-terminal-resizer active" } else { "mini-terminal-resizer" },
                                onmousedown: move |e| {
                                    e.prevent_default();
                                    resizing_terminal.set(true);
                                },
                                ontouchstart: move |e| {
                                    e.prevent_default();
                                    resizing_terminal.set(true);
                                },
                            }

                            div { class: "mini-terminal", style: "height: {terminal_height}px;",
                                div { class: "mini-terminal-head", "OUTPUT" }
                                div { class: "mini-terminal-body",
                                    if *is_running.read() {
                                        div { class: "mini-output-loading", "Running..." }
                                    }
                                    {
                                        let lines = output_lines.read().clone();
                                        let error = output_error.read().clone();
                                        rsx! {
                                            for (i, line) in lines.iter().enumerate() {
                                                pre { key: "{i}", class: "mini-output-line", "{line}" }
                                            }
                                            if let Some(ref err) = error {
                                                pre { class: "mini-output-error", "{err}" }
                                            }
                                            if lines.is_empty() && error.is_none() && !*is_running.read() {
                                                div { class: "mini-output-empty", "Click Run to see output" }
                                            }
                                        }
                                    }
                                }
                            }

                            if *show_compiled.read() {
                                div {
                                    class: if *resizing_compiled.read() { "mini-terminal-resizer active" } else { "mini-terminal-resizer" },
                                    onmousedown: move |e| {
                                        e.prevent_default();
                                        resizing_compiled.set(true);
                                    },
                                    ontouchstart: move |e| {
                                        e.prevent_default();
                                        resizing_compiled.set(true);
                                    },
                                }
                                div { class: "mini-compiled", style: "height: {compiled_height}px;",
                                    div { class: "mini-compiled-head", "COMPILED RUST" }
                                    pre { class: "mini-compiled-body", "{compiled_output}" }
                                }
                            }

                            div { class: "mini-studio-cta",
                                {
                                    let mode = *demo_mode.read();
                                    let examples = examples_for_mode(mode);
                                    let idx = (*active_index.read()).min(examples.len().saturating_sub(1));
                                    let studio_url = format!("/studio?file={}", examples[idx].studio_path);
                                    rsx! {
                                        a { href: "{studio_url}", class: "mini-cta-btn",
                                            "Try it in the Studio →"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                section { class: "section how-it-works section-center",
                    h2 { class: "section-title", "How it works" }
                    p { class: "section-sub",
                        "Three modes. One language: English."
                    }

                    div { class: "mode-stories",
                        div { class: "mode-story",
                            div { class: "mode-story-icon", "λ" }
                            h3 { "Write a program" }
                            div { class: "mode-story-demo",
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Input" }
                                    span { class: "mode-story-value", "Let x be 10. Show x + 5." }
                                }
                                div { class: "mode-story-arrow", "↓" }
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Output" }
                                    span { class: "mode-story-value success", "15" }
                                }
                            }
                            p { "Type readable definitions. Get compiled programs — Rust under the hood, English on the surface." }
                        }

                        div { class: "mode-story",
                            div { class: "mode-story-icon", "∀" }
                            h3 { "Formalize an argument" }
                            div { class: "mode-story-demo",
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Input" }
                                    span { class: "mode-story-value", "Every cat sleeps." }
                                }
                                div { class: "mode-story-arrow", "↓" }
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Output" }
                                    span { class: "mode-story-value logic", "∀x(Cat(x) → Sleep(x))" }
                                }
                            }
                            p { "Turn plain language into First-Order Logic. Every reading surfaced — no guessing." }
                        }

                        div { class: "mode-story",
                            div { class: "mode-story-icon", "π" }
                            h3 { "Prove a theorem" }
                            div { class: "mode-story-demo",
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Input" }
                                    span { class: "mode-story-value", "Theorem: ∀n, n + 0 = n." }
                                }
                                div { class: "mode-story-arrow", "↓" }
                                div { class: "mode-story-row",
                                    span { class: "mode-story-label", "Output" }
                                    span { class: "mode-story-value success", "Proof: by induction. ✓" }
                                }
                            }
                            p { "Define types, state theorems, and prove them with automated tactics." }
                        }
                    }
                }

                section { class: "section hello-world section-center hello-world-layout",
                    h2 { class: "section-title", "Hello World in LOGOS" }

                    div { class: "hello-editor",
                        div { class: "hello-editor-head",
                            span { class: "hello-filename", "hello-world.logos" }
                            button {
                                class: "hello-run-btn",
                                disabled: *hello_running.read(),
                                onclick: move |_| {
                                    let code = hello_code.read().clone();
                                    hello_running.set(true);
                                    hello_output.set(Vec::new());
                                    hello_error.set(None);
                                    spawn(async move {
                                        let result = interpret_for_ui(&code).await;
                                        hello_output.set(result.lines);
                                        hello_error.set(result.error);
                                        hello_running.set(false);
                                    });
                                },
                                if *hello_running.read() { "Running..." } else { "▶ Run" }
                            }
                        }
                        div { class: "hello-editor-body",
                            div { class: "hello-editor-left",
                                CodeEditor {
                                    value: hello_code.read().clone(),
                                    on_change: move |v: String| hello_code.set(v),
                                    language: Language::Logos,
                                    placeholder: "Enter code...".to_string(),
                                }
                            }
                            div { class: "hello-editor-right",
                                div { class: "hello-output-head", "Output" }
                                div { class: "hello-output-body",
                                    if *hello_running.read() {
                                        div { class: "hello-output-loading", "Running..." }
                                    }
                                    {
                                        let lines = hello_output.read().clone();
                                        let error = hello_error.read().clone();
                                        rsx! {
                                            for (i, line) in lines.iter().enumerate() {
                                                pre { key: "{i}", class: "hello-output-line", "{line}" }
                                            }
                                            if let Some(ref err) = error {
                                                pre { class: "hello-output-error", "{err}" }
                                            }
                                            if lines.is_empty() && error.is_none() && !*hello_running.read() {
                                                div { class: "hello-output-empty", "Click Run to see output" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "hello-pill-wrap",
                        p { class: "hello-note", "Compiles to a native binary via Rust. Zero runtime overhead." }
                    }
                    div { class: "hello-cta-wrap",
                        a { href: "/studio?file=examples/code/hello-world.logos", class: "btn btn-primary",
                            "Open in Studio →"
                        }
                    }
                }

                section { class: "section",
                    h2 { class: "section-title", "What you get" }
                    p { class: "section-sub",
                        "LOGICAFFEINE translates intuition into structure — so you can test it, teach it, or ship it."
                    }

                    div { class: "grid3",
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Lightning, size: IconSize::Large, color: "#00d4ff" }
                            }
                            h3 { "Instant Transpilation" }
                            p { "Type normal English. Get programs, logic, and math output in seconds — readable enough to learn from, strict enough to verify." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Brain, size: IconSize::Large, color: "#818cf8" }
                            }
                            h3 { "Socratic Tutor" }
                            p { "When your statement is ambiguous, the tutor asks questions that force clarity instead of guessing." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Document, size: IconSize::Large, color: "#22c55e" }
                            }
                            h3 { "Assumption Surfacing" }
                            p { "Reveal missing premises, hidden quantifiers, and scope mistakes — the usual sources of bad arguments." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Beaker, size: IconSize::Large, color: "#fbbf24" }
                            }
                            h3 { "Consistency & Validity Checks" }
                            p { "Spot contradictions, invalid inferences, and rule collisions across Code, Logic, and Math modes — before they hit production or policy." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Tools, size: IconSize::Large, color: "#ec4899" }
                            }
                            h3 { "Studio + Curriculum" }
                            p { "Explore freely in Studio, then build mastery in Learn with structured lessons and practice." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Lock, size: IconSize::Large, color: "#8b5cf6" }
                            }
                            h3 { "Commercial-Ready" }
                            p { "Licensing options for teams and enterprises — with a path toward governance and controlled deployments." }
                        }
                    }
                }

                section { class: "section section-center",
                    h2 { class: "section-title", "Security & Policies" }
                    p { class: "section-sub",
                        "Capability-based security with policy blocks. Define who can do what in plain English."
                    }

                    div { class: "grid2",
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Shield, size: IconSize::Large, color: "#60a5fa" }
                            }
                            h3 { "Policy Blocks" }
                            p { "Define security rules as readable policy sections. Who can access what — stated plainly." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Lock, size: IconSize::Large, color: "#a78bfa" }
                            }
                            h3 { "Capabilities" }
                            p { "Role-based access control expressed in English. No annotation soup." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Beaker, size: IconSize::Large, color: "#22c55e" }
                            }
                            h3 { "Check Guards" }
                            p { "Runtime guard checks that enforce your policies. \"Check that the user is admin.\"" }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Brain, size: IconSize::Large, color: "#fbbf24" }
                            }
                            h3 { "Predicates" }
                            p { "Define custom predicates: \"A User is admin if the user's role equals 'admin'.\"" }
                        }
                    }

                    div { class: "security-demo",
                        div { class: "security-demo-head",
                            span { "Policy → Compiled Output" }
                        }
                        div { class: "security-demo-body",
                            div { class: "security-demo-col",
                                div { class: "security-demo-label", "LOGOS Policy" }
                                pre { class: "code",
"## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin\n    if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is admin.\nShow \"Access granted\"." }
                            }
                            div { class: "security-demo-arrow", "→" }
                            div { class: "security-demo-col",
                                div { class: "security-demo-label", "Compiled Output" }
                                pre { class: "code",
"struct User {{\n    role: String,\n}}\n\nimpl User {{\n    fn is_admin(&self) -> bool {{\n        self.role == \"admin\"\n    }}\n}}\n\nfn main() {{\n    let u = User {{ role: \"admin\".into() }};\n    assert!(u.is_admin());\n    println!(\"Access granted\");\n}}" }
                            }
                        }
                    }
                }

                section { class: "section", id: "for",
                    h2 { class: "section-title", style: "padding: 50px 0; font-size: var(--font-display-lg);",
                        "For people who want their reasoning to survive contact with reality."
                    }

                    div { class: "grid3",
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::GraduationCap, size: IconSize::Large, color: "#00d4ff" }
                            }
                            h3 { "Students & Educators" }
                            p { "Teach formal reasoning with feedback that's immediate, concrete, and harder to game than multiple choice." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Shield, size: IconSize::Large, color: "#818cf8" }
                            }
                            h3 { "Law, Policy, Compliance" }
                            p { "Translate policy language into verifiable rules. Reduce ambiguity. Make reviews faster and safer." }
                        }
                        div { class: "card",
                            div { class: "icon-box",
                                Icon { variant: IconVariant::Tools, size: IconSize::Large, color: "#22c55e" }
                            }
                            h3 { "Engineering & Research" }
                            p { "Specify systems, constraints, and invariants in a form you can test — without forcing everyone into formal syntax." }
                        }
                    }
                }

                section { class: "section compare-section section-center",
                    h2 { class: "section-title", "How LOGOS Compares" }
                    p { class: "section-sub",
                        "A new approach to formal reasoning."
                    }

                    div { class: "compare-table",
                        div { class: "compare-row header",
                            div { class: "compare-cell", "Feature" }
                            div { class: "compare-cell highlight", "LOGOS" }
                            div { class: "compare-cell", "Python" }
                            div { class: "compare-cell", "Lean 4" }
                            div { class: "compare-cell", "Rust" }
                            div { class: "compare-cell", "Elixir" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Syntax" }
                            div { class: "compare-cell highlight", "English prose" }
                            div { class: "compare-cell", "Symbols" }
                            div { class: "compare-cell", "Lean DSL" }
                            div { class: "compare-cell", "Symbols" }
                            div { class: "compare-cell", "Symbols" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "File Format" }
                            div { class: "compare-cell highlight", "Markdown (.md)" }
                            div { class: "compare-cell", ".py" }
                            div { class: "compare-cell", ".lean" }
                            div { class: "compare-cell", ".rs" }
                            div { class: "compare-cell", ".ex" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Performance" }
                            div { class: "compare-cell highlight", "Native (via Rust)" }
                            div { class: "compare-cell", "Interpreted" }
                            div { class: "compare-cell", "Native" }
                            div { class: "compare-cell", "Native" }
                            div { class: "compare-cell", "BEAM VM" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Proofs" }
                            div { class: "compare-cell highlight", "Built-in" }
                            div { class: "compare-cell", "None" }
                            div { class: "compare-cell", "Required" }
                            div { class: "compare-cell", "Optional" }
                            div { class: "compare-cell", "None" }
                        }
                        div { class: "compare-row",
                            div { class: "compare-cell label", "Memory" }
                            div { class: "compare-cell highlight", "Ownership (English)" }
                            div { class: "compare-cell", "GC" }
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
                            "Start with the Curriculum, or explore any mode in the Studio. Code, Logic, Math — your call."
                        }
                        div { class: "hero-ctas",
                            Link { to: Route::Learn {}, class: "btn btn-primary", "Start Learning" }
                            Link { to: Route::Pricing {}, class: "btn btn-ghost", "View Licenses" }
                        }
                    }
                }

                Footer {}
            }
        }
    }
}
