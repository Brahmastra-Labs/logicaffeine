//! The front page: the promise and the ways in, the Studio itself, then how LOGOS compares, what
//! comes with it, and the FAQ.
//!
//! The page leads with the heading ("Code we can all read. Logic and mathematics built in. Bring
//! your own agent."), one line on how to use LOGOS ([`content::how`]), the box for a reader who
//! installs largo by hand (the shell and PowerShell commands, each with its copy), and three buttons,
//! one per system with its mark, each copying the prompt for a coding agent on that system: five
//! lines, the guide behind them served at /agent.txt. The prompt stays off the page; it shows,
//! selected, only when the browser refuses to copy, so a reader can copy it by hand. Below the words
//! stands the Studio, in a frame (`studio_workbench::StudioFrame`): the same workbench as `/studio`,
//! its Files tree a shelf of shipped examples ([`content::shelf`]), opened on a web page beside its
//! live preview. What the page says is data in
//! [`content`]. Below the hero: the comparison ([`content::compare`]), eight cards each with a Try it
//! that opens a shelf file in the frame above, the library and what 0.11.0 brings
//! ([`content::library`], [`content::features`]), and the FAQ ([`content::faq`]). The prerendered page
//! is the app's first frame: nothing animates in, and the frame's first paint depends on nothing
//! measured, so the takeover changes nothing a reader sees.
//!
//! # Route
//!
//! Accessed via [`Route::Landing`].

pub mod content;

use dioxus::prelude::*;

use crate::ui::clipboard;
use crate::ui::components::footer::Footer;
use crate::ui::components::icon::{Icon, IconSize, IconVariant};
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::pages::studio_workbench::{use_shelf, StudioFrame, WorkbenchState, FRAME_ID};
use crate::ui::router::Route;
use crate::ui::seo::{faq_schema, organization_schema, pages as seo_pages, website_schema, JsonLdMultiple, PageHead};
use content::{compare, compare_lead, faq, features, how, install_commands, library, prompt, prompt_text, said, Install, Line, System, COMPARE_HEADING, HEADING, INSTALL_HEADING, INSTRUCTIONS_ID, LIBRARY_HEADING};

const LANDING_STYLE: &str = r#"
.landing {
  min-height: 100vh;
  color: var(--text-primary);
  font-family: var(--font-sans);
  /* The page's emerald: the one colour for what a reader is asked to press. */
  --landing-emerald: #34d399;
  --landing-emerald-deep: #10b981;
  --landing-emerald-ink: #052e22;
}

.landing-main { padding: 0 0 96px; }

.landing [id] { scroll-margin-top: 96px; }

.landing a:focus-visible,
.landing button:focus-visible,
.landing pre:focus-visible {
  outline: 2px solid var(--accent-primary);
  outline-offset: 3px;
}

/* The words first: the heading, the one line on how, the install box and the three copy buttons,
   then the Studio across the page below them. One column at every width. */
.landing-intro {
  max-width: 1200px;
  margin: 0 auto;
  padding: 40px 16px 36px;
}

.landing-heading {
  margin: 0;
  font-size: clamp(38px, 9vw, 68px);
  line-height: 1.04;
  letter-spacing: -0.03em;
  font-weight: 800;
  color: var(--text-primary);
}

.landing-heading span { display: block; text-wrap: balance; }
.landing-heading span + span { color: var(--text-tertiary); }

.landing-how {
  margin: 20px 0 0;
  max-width: 62ch;
  color: var(--text-secondary);
  font-size: var(--font-body-lg);
  line-height: 1.5;
}

.landing-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  margin-top: 28px;
}

.landing-primary,
.landing-secondary {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  min-height: 48px;
  padding: 0 20px;
  border-radius: var(--radius-md);
  font: 650 var(--font-body-md)/1 var(--font-sans);
  text-decoration: none;
  cursor: pointer;
}

.landing-primary {
  border: 1px solid var(--landing-emerald-deep);
  background: var(--landing-emerald-deep);
  color: var(--landing-emerald-ink);
}

.landing-primary:hover { background: var(--landing-emerald); border-color: var(--landing-emerald); }

/* The three copy buttons: each system's mark large, stacked above the words, so the marks are what a
   reader sees first. The three stand in a row and take equal widths; on a phone they stack. */
.landing-copy {
  flex: 1 1 0;
  flex-direction: column;
  gap: 12px;
  min-width: 200px;
  min-height: 0;
  padding: 20px 20px 18px;
  text-align: center;
  line-height: 1.25;
}

.landing-os-mark {
  display: inline-flex;
  width: 44px;
  height: 44px;
  flex: none;
}

.landing-os-mark svg { width: 100%; height: 100%; fill: currentColor; }

@media (max-width: 719px) {
  .landing-copy { flex-basis: 100%; }
}

/* The box for a reader who installs by hand: a straight emerald edge on its left, a command per
   kind of system in the editor's mono, each with its copy. */
.landing-install {
  max-width: 72ch;
  margin: 24px 0 0;
  padding: 12px 16px 14px;
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-left: 3px solid var(--landing-emerald-deep);
  border-radius: 0 var(--radius-md) var(--radius-md) 0;
  background: rgba(255, 255, 255, 0.025);
}

.landing-install-label {
  display: block;
  margin: 0 0 8px;
  font-size: var(--font-body-sm);
  font-weight: 650;
  color: var(--text-secondary);
}

.landing-install-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 12px;
  padding: 6px 0;
}

.landing-install-os {
  flex: 0 0 9.5em;
  color: var(--text-tertiary);
  font-size: var(--font-body-sm);
}

.landing-install-cmd {
  flex: 1 1 24ch;
  min-width: 0;
  overflow-wrap: anywhere;
  font: 13px/1.6 var(--font-mono);
  color: var(--text-primary);
}

.landing-copy-cmd {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid rgba(52, 211, 153, 0.45);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--landing-emerald);
  font: 650 var(--font-body-sm)/1 var(--font-sans);
  cursor: pointer;
}

.landing-copy-cmd:hover { background: rgba(52, 211, 153, 0.12); }

.landing-secondary {
  border: 1px solid rgba(255, 255, 255, 0.16);
  background: transparent;
  color: var(--text-primary);
}

.landing-secondary:hover { border-color: rgba(255, 255, 255, 0.32); }

.landing-status {
  margin: 14px 0 0;
  font-size: var(--font-body-sm);
  color: var(--text-secondary);
}

.landing-visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}

/* The prompt, shown only when the browser refuses to copy: five lines in the editor's mono, wrapping
   rather than scrolling sideways, so a phone reads them whole. */
.landing-prompt { margin-top: 20px; }

.landing-prompt[hidden] { display: none; }

.landing-instructions {
  margin: 0;
  padding: 18px 20px;
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-left: 3px solid var(--accent-primary);
  border-radius: var(--radius-lg);
  background: rgba(255, 255, 255, 0.025);
  font: 13.5px/1.65 var(--font-mono);
  color: var(--text-secondary);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  tab-size: 4;
}

.landing-instructions .step { color: var(--text-primary); font-weight: 700; }
.landing-instructions .run,
.landing-instructions .open { color: var(--accent-primary); }

/* The prompts stay hidden until a browser refuses a copy; then the one copied shows, selected. */
.landing-instructions[hidden] { display: none; }

/* Below the hero: the comparison, the library and the FAQ, each a band with a hairline above. */
.landing-wrap {
  max-width: 1200px;
  margin: 56px auto 0;
  padding: 0 16px;
}

.landing-compare,
.landing-library,
.landing-faq {
  padding: 56px 0;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
}

.landing-compare h2,
.landing-library h2,
.landing-faq h2 {
  margin: 0 0 12px;
  font-size: var(--font-heading-lg);
  line-height: 1.2;
  letter-spacing: -0.01em;
  font-weight: 750;
}

.landing-lead {
  margin: 0 0 28px;
  max-width: 62ch;
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.6;
}

/* The cards: a grid that takes as many across as fit, each a kind of work. */
.landing-cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 14px;
}

.landing-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 18px 18px 16px;
  border: 1px solid rgba(255, 255, 255, 0.10);
  border-radius: var(--radius-lg);
  background: rgba(255, 255, 255, 0.025);
}

.landing-card h3 {
  margin: 0;
  font-size: var(--font-body-md);
  font-weight: 700;
}

.landing-usually {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-body-sm);
  line-height: 1.5;
}

.landing-usually-label { color: var(--text-secondary); }

.landing-logos {
  margin: 0;
  flex: 1 1 auto;
  color: var(--text-primary);
  font-size: var(--font-body-md);
  line-height: 1.55;
}

.landing-card-foot {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
}

.landing-tag {
  display: inline-flex;
  align-items: center;
  padding: 3px 8px;
  border: 1px solid rgba(0, 212, 255, 0.35);
  border-radius: 999px;
  color: var(--accent-primary);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.2;
  white-space: nowrap;
}

.landing-try {
  margin-left: auto;
  min-height: 36px;
  padding: 0 14px;
  border: 1px solid var(--landing-emerald-deep);
  border-radius: var(--radius-md);
  background: var(--landing-emerald-deep);
  color: var(--landing-emerald-ink);
  font: 650 var(--font-body-sm)/1 var(--font-sans);
  cursor: pointer;
}

.landing-try:hover { background: var(--landing-emerald); border-color: var(--landing-emerald); }

/* The library: two lines of figures, the release tag, then what 0.11.0 brings, one line each. */
.landing-library-line {
  margin: 0 0 8px;
  max-width: 70ch;
  color: var(--text-primary);
  font-size: var(--font-body-md);
  line-height: 1.6;
}

.landing-library .landing-tag { margin: 4px 0 20px; }

.landing-features {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 8px 32px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.landing-features li {
  position: relative;
  padding-left: 18px;
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.55;
}

.landing-features li::before {
  content: "";
  position: absolute;
  left: 0;
  top: 0.68em;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent-primary);
}

/* The FAQ. */
.landing-faq dl {
  margin: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 28px 48px;
}

.landing-faq dt {
  font-weight: 700;
  font-size: var(--font-body-md);
  margin-bottom: 6px;
}

.landing-faq dd {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-body-md);
  line-height: 1.6;
  max-width: 62ch;
}

.landing-faq dd a { color: var(--accent-primary); }

@media (min-width: 720px) {
  .landing-intro,
  .landing-wrap { padding-left: 24px; padding-right: 24px; }
  .landing-features { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .landing-faq dl { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}

@media (min-width: 1100px) {
  .landing-intro { padding-top: 56px; padding-bottom: 44px; }
  .landing-heading { font-size: clamp(40px, 3.5vw, 54px); }
}

@media (prefers-reduced-motion: reduce) {
  .landing *, .landing *::before, .landing *::after { transition: none !important; animation: none !important; scroll-behavior: auto !important; }
}
"#;

/// A build made with `LOGOS_TOUR` set (the lab's dev server, for the screen on the wall) scrolls
/// the page itself when opened as `/?tour=<seconds per pass>[&rest=<seconds>]`: down over that
/// many seconds, a rest at the bottom, up again, and so on, eased at both ends so it glides.
/// Without the variable the script is not in the page; without the query it does nothing.
const TOUR_SCRIPT: &str = r#"(() => {
  // The router rewrites the address once the app takes over; the address the page was opened at
  // stays in its navigation entry.
  const opened = performance.getEntriesByType('navigation')[0];
  const search = (opened ? new URL(opened.name).search : '') || location.search;
  const m = search.match(/[?&]tour=(\d+)(?:.*&rest=(\d+))?/);
  if (!m) return;
  const period = Number(m[1]) * 1000, rest = (m[2] ? Number(m[2]) : 3) * 1000;
  const scroller = () => [document.scrollingElement, document.body].reduce((a, b) => (b.scrollHeight - b.clientHeight > a.scrollHeight - a.clientHeight ? b : a));
  const max = () => { const s = scroller(); return s.scrollHeight - s.clientHeight; };
  const ease = (u) => (u < 0.5 ? 2 * u * u : 1 - Math.pow(-2 * u + 2, 2) / 2);
  let t0 = null;
  const frame = (now) => {
    if (t0 === null) t0 = now;
    const cycle = 2 * (period + rest), t = (now - t0) % cycle;
    let y;
    if (t < period) y = ease(t / period) * max();
    else if (t < period + rest) y = max();
    else if (t < 2 * period + rest) y = (1 - ease((t - period - rest) / period)) * max();
    else y = 0;
    scroller().scrollTop = y;
    requestAnimationFrame(frame);
  };
  // The app renders this after the document has loaded, so start now; a prerendered page waits for load.
  const start = () => setTimeout(() => requestAnimationFrame(frame), 1500);
  if (document.readyState === 'complete') start(); else addEventListener('load', start);
  window.__logosTour = { period, rest };
})();"#;

#[component]
pub fn Landing() -> Element {
    let faq = faq();
    let pairs: Vec<(&str, &str)> = faq.iter().map(|item| (item.question, item.answer.text.as_str())).collect();
    let schemas = vec![organization_schema(), website_schema(), faq_schema(&pairs)];
    let frame = use_shelf(content::shelf());

    rsx! {
        PageHead {
            title: seo_pages::LANDING.title,
            description: seo_pages::LANDING.description,
            canonical_path: seo_pages::LANDING.canonical_path,
        }
        style { "{LANDING_STYLE}" }
        if option_env!("LOGOS_TOUR").is_some() {
            script { dangerous_inner_html: TOUR_SCRIPT }
        }
        JsonLdMultiple { schemas }

        div { class: "landing",
            MainNav { active: ActivePage::Other }
            main { class: "landing-main",
                section { id: "top", class: "landing-hero",
                    div { class: "landing-intro",
                        div { class: "landing-pitch",
                            h1 { class: "landing-heading",
                                // A space between the sentences, so a reader of the text (a crawler,
                                // a copy) gets three sentences, not one run-on word at each seam.
                                for (i, line) in HEADING.iter().enumerate() {
                                    if i > 0 { " " }
                                    span { "{line}" }
                                }
                            }
                            How {}
                            InstallBox {}
                            CopyInstructions {}
                        }
                    }
                    StudioFrame { state: frame }
                }
                div { class: "landing-wrap",
                    Comparison { frame }
                    Library {}
                    Questions {}
                }
            }
            Footer {}
        }
    }
}

/// How LOGOS compares: the heading, the lead, and a card for each kind of work. A card's Try it opens
/// the card's shelf file in the frame above and brings the frame into view.
#[component]
fn Comparison(frame: WorkbenchState) -> Element {
    rsx! {
        section { id: "compare", class: "landing-compare",
            h2 { "{COMPARE_HEADING}" }
            p { class: "landing-lead", "{compare_lead().text}" }
            div { class: "landing-cards",
                for card in compare() {
                    article { class: "landing-card",
                        h3 { "{card.kind}" }
                        p { class: "landing-usually",
                            span { class: "landing-usually-label", "Usually: " }
                            "{card.usually_names()}"
                        }
                        p { class: "landing-logos", "{card.logos.text}" }
                        div { class: "landing-card-foot",
                            for tag in card.tags {
                                span { class: "landing-tag", "{tag.label()}" }
                            }
                            if let Some(file) = card.try_it {
                                button {
                                    r#type: "button",
                                    class: "landing-try",
                                    "data-file": file,
                                    aria_controls: FRAME_ID,
                                    title: "Open {file} in the Studio above",
                                    onclick: move |_| {
                                        frame.choose(file.to_string());
                                        #[cfg(target_arch = "wasm32")]
                                        if let Some(studio) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(FRAME_ID)) {
                                            studio.scroll_into_view();
                                        }
                                    },
                                    "Try it"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What comes with LOGOS: the library's figures, from which release largo has it, and what 0.11.0
/// brings, a line each.
#[component]
fn Library() -> Element {
    rsx! {
        section { id: "library", class: "landing-library",
            h2 { "{LIBRARY_HEADING}" }
            for (i, line) in library().iter().enumerate() {
                p { key: "{i}", class: "landing-library-line", "{line.text}" }
            }
            ul { class: "landing-features",
                for (i, line) in features().iter().enumerate() {
                    li { key: "{i}", "{line.text}" }
                }
            }
        }
    }
}

/// The FAQ, each answer with the run of it that is a link shown as one.
#[component]
fn Questions() -> Element {
    rsx! {
        section { id: "faq", class: "landing-faq",
            h2 { "FAQ" }
            dl {
                for (i, item) in faq().iter().enumerate() {
                    div { key: "{i}",
                        dt { "{item.question}" }
                        dd { {answer(item)} }
                    }
                }
            }
        }
    }
}

/// An answer, with the run of it that is a link shown as one.
fn answer(item: &content::Faq) -> Element {
    match item.link {
        Some((text, href)) => {
            let (before, after) = item.answer.text.split_once(text).unwrap_or((item.answer.text.as_str(), ""));
            rsx! {
                "{before}"
                a { href, "{text}" }
                "{after}"
            }
        }
        None => rsx! { "{item.answer.text}" },
    }
}

/// The one line on how to use LOGOS ([`content::how`]).
#[component]
fn How() -> Element {
    rsx! {
        p { class: "landing-how", "{how().text}" }
    }
}

/// The box for a reader who installs largo by hand: the command for each kind of system, each with
/// its copy, which says Copied for a moment.
#[component]
fn InstallBox() -> Element {
    let mut copied = use_signal(|| None::<usize>);
    let mut presses = use_signal(|| 0u32);
    rsx! {
        div { class: "landing-install",
            span { class: "landing-install-label", "{INSTALL_HEADING}" }
            for (i, (os, command)) in install_commands().into_iter().enumerate() {
                div { key: "{i}", class: "landing-install-row",
                    span { class: "landing-install-os", "{os}" }
                    code { id: "landing-install-cmd-{i}", class: "landing-install-cmd", "{command}" }
                    button {
                        r#type: "button",
                        class: "landing-copy-cmd",
                        title: "Copy the {os} install command",
                        onclick: move |_| {
                            let press = *presses.peek() + 1;
                            presses.set(press);
                            // The write starts in the click: Safari grants the clipboard nowhere else.
                            let pending = clipboard::copy(command);
                            spawn(async move {
                                if pending.await {
                                    copied.set(Some(i));
                                    #[cfg(target_arch = "wasm32")]
                                    gloo_timers::future::TimeoutFuture::new(2400).await;
                                    if *presses.peek() == press {
                                        copied.set(None);
                                    }
                                }
                            });
                        },
                        Icon { variant: if copied() == Some(i) { IconVariant::Check } else { IconVariant::Copy }, size: IconSize::Small }
                        if copied() == Some(i) { "{said::COMMAND_COPIED}" } else { "Copy" }
                    }
                }
            }
        }
    }
}

/// Whether the instructions are on the clipboard.
#[derive(Clone, Copy, PartialEq)]
enum Copied {
    Not,
    Yes,
    /// The browser refused: the prompt is shown and selected for the reader to copy.
    Refused,
}

/// The three copy buttons, one per system with its mark, below the install box, with the ways on
/// beneath them. Each copies the prompt for its system ([`prompt_text`]), which the page keeps off
/// screen until a browser refuses the copy.
#[component]
fn CopyInstructions() -> Element {
    let mut copied = use_signal(|| (System::MacOs, Copied::Not));
    let mut presses = use_signal(|| 0u32);
    let (last, state) = copied();
    let status = match state {
        Copied::Not => "",
        Copied::Yes => said::COPIED_STATUS,
        Copied::Refused => said::COPY_REFUSED,
    };
    rsx! {
        div { class: "landing-actions",
            for system in System::ALL {
                button {
                    key: "{system.slug()}",
                    r#type: "button",
                    class: "landing-primary landing-copy",
                    "data-system": system.slug(),
                    aria_controls: INSTRUCTIONS_ID,
                    onclick: move |_| {
                        let press = *presses.peek() + 1;
                        presses.set(press);
                        let install = system.install();
                        // The write starts in the click: Safari grants the clipboard nowhere else.
                        let pending = clipboard::copy(prompt_text(install));
                        spawn(async move {
                            let done = pending.await;
                            copied.set((system, if done { Copied::Yes } else { Copied::Refused }));
                            if !done {
                                // The prompt shows on the next frame; the selection waits for it.
                                #[cfg(target_arch = "wasm32")]
                                gloo_timers::future::TimeoutFuture::new(50).await;
                                clipboard::select_contents(&prompt_id(install));
                            }
                            if done {
                                #[cfg(target_arch = "wasm32")]
                                gloo_timers::future::TimeoutFuture::new(2400).await;
                                if *presses.peek() == press {
                                    copied.set((system, Copied::Not));
                                }
                            }
                        });
                    },
                    span { class: "landing-os-mark", "aria-hidden": "true",
                        svg { view_box: "0 0 24 24", role: "img", path { d: system.mark() } }
                    }
                    span {
                        if last == system && state == Copied::Yes { "{said::COPIED}" } else { "{system.copy_label()}" }
                    }
                }
            }
        }
        div { class: "landing-actions",
            Link { to: Route::Studio { file: None }, class: "landing-secondary", "Open Studio" }
            Link { to: Route::Learn { module: String::new() }, class: "landing-secondary", "Learn" }
            Link { to: Route::Contact { section: String::new() }, class: "landing-secondary", "Contact" }
        }
        p {
            class: if state == Copied::Refused { "landing-status" } else { "landing-visually-hidden" },
            role: "status",
            "{status}"
        }
        Prompts { revealed: (state == Copied::Refused).then_some(last.install()) }
    }
}

/// The element that holds `install`'s prompt.
fn prompt_id(install: Install) -> String {
    format!("agent-prompt-{}", install.name())
}

/// The prompts, one per install, each one span per line so each kind of line takes its colour, and
/// the text the copy buttons copy, character for character. Every block is hidden until `revealed`
/// names its install (a browser that refused that copy).
#[component]
fn Prompts(revealed: Option<Install>) -> Element {
    rsx! {
        div { id: INSTRUCTIONS_ID, class: "landing-prompt", hidden: revealed.is_none(),
            for install in Install::ALL.iter().copied() {
                pre {
                    key: "{install.name()}",
                    id: prompt_id(install),
                    class: "landing-instructions",
                    "data-install": install.name(),
                    hidden: revealed != Some(install),
                    tabindex: "0",
                    aria_label: "Agent instructions",
                    for (i, line) in prompt(install).iter().enumerate() {
                        span { key: "{i}", class: kind(line), if i + 1 == prompt(install).len() { "{line.text()}" } else { "{line.text()}\n" } }
                    }
                }
            }
        }
    }
}

/// The class a line of the instructions is painted with.
fn kind(line: &Line) -> &'static str {
    match line {
        Line::Step(_) => "step",
        Line::Say(_) => "say",
        Line::Rule(_) => "rule",
        Line::Agree(_) => "agree",
        Line::Run(_) => "run",
        Line::Open(_) => "open",
        Line::Blank => "blank",
    }
}
