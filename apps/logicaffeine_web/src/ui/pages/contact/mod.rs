//! The contact page: what LOGOS is (open source, under MIT), how to install it, the form that
//! reaches the team, and the FAQ.
//!
//! What the page says is data in [`content`]; the Learn counts it quotes are read by [`claims`];
//! the form's rules are [`lead`] and its browser half is `form`. `/contact#<section>` opens at
//! that section (the route models the fragment, so boot normalization keeps it), and `/pricing`,
//! the page's old address, redirects here.
//!
//! # Route
//!
//! Accessed via [`Route::Contact`].

pub mod claims;
pub mod content;
mod form;
pub mod lead;

use dioxus::prelude::*;

use crate::ui::components::footer::Footer;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::fragment::scroll_to_section;
use crate::ui::router::Route;
use crate::ui::seo::{breadcrumb_schema, faq_schema, organization_contact_schema, pages as seo_pages, BreadcrumbItem, JsonLdMultiple, PageHead};
use content::{Faq, CONTACT_HEADING, INSTALL_UNIX, INSTALL_WINDOWS, OPEN_SOURCE_LINE, TEAM_EMAIL};
use form::LeadForm;

const CONTACT_STYLE: &str = r#"
.contact {
  min-height: 100vh;
  color: var(--text-primary);
  font-family: var(--font-sans);
}

.contact-main {
  max-width: 1120px;
  margin: 0 auto;
  padding: 0 var(--spacing-xl) 96px;
}

.contact [id] { scroll-margin-top: 104px; }

.contact h1,
.contact h2,
.contact h3 { color: var(--text-primary); }

.contact h1 {
  font-size: var(--font-display-lg);
  line-height: 1.06;
  letter-spacing: -0.025em;
  font-weight: 800;
  max-width: 13ch;
}

.contact h2 {
  font-size: var(--font-heading-lg);
  line-height: 1.2;
  letter-spacing: -0.01em;
  font-weight: 750;
}

.contact h3 {
  font-size: var(--font-heading-sm);
  line-height: 1.3;
  font-weight: 700;
}

.contact p,
.contact li,
.contact dd,
.contact td {
  font-size: var(--font-body-md);
  line-height: 1.6;
  color: var(--text-secondary);
}

.contact a:focus-visible,
.contact button:focus-visible,
.contact input:focus-visible {
  outline: 2px solid var(--accent-primary);
  outline-offset: 3px;
}

.contact-text-link {
  color: var(--accent-primary);
  text-decoration: underline;
  text-underline-offset: 3px;
  text-decoration-thickness: 1px;
}

.contact-text-link:hover { text-decoration-thickness: 2px; }

/* Calls to act */
.contact-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--spacing-md);
}

.contact-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 44px;
  padding: 0 20px;
  border-radius: var(--radius-md);
  border: 1px solid rgba(255, 255, 255, 0.18);
  background: rgba(255, 255, 255, 0.04);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-body-md);
  font-weight: 650;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.contact-btn:hover {
  background: rgba(255, 255, 255, 0.09);
  border-color: rgba(255, 255, 255, 0.3);
}

.contact-btn.primary {
  background: var(--accent-primary);
  border-color: var(--accent-primary);
  color: var(--bg-gradient-start);
}

.contact-btn.primary:hover {
  background: rgba(var(--accent-primary-rgb), 0.82);
}

.contact-btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

/* Hero */
.contact-hero {
  display: grid;
  grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
  gap: 56px;
  align-items: end;
  padding: 72px 0 64px;
  animation: contact-arrive 0.45s ease-out both;
}

@keyframes contact-arrive {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: none; }
}

.contact-lede {
  margin: 20px 0 28px;
  max-width: 34ch;
}

.contact .contact-lede {
  font-size: var(--font-body-lg);
  color: var(--text-primary);
}

.contact-install {
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: var(--radius-lg);
  background: rgba(0, 0, 0, 0.35);
}

.contact .contact-install-title {
  padding: 14px 18px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  font-size: var(--font-body-md);
  font-weight: 650;
  letter-spacing: 0;
}

.contact-command {
  padding: 14px 18px 16px;
}

.contact-command + .contact-command {
  border-top: 1px solid rgba(255, 255, 255, 0.06);
}

.contact-command-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  margin-bottom: 8px;
}

.contact .contact-command-label {
  font-size: var(--font-caption-md);
  color: var(--text-tertiary);
}

.contact-command-text {
  display: block;
  font-family: var(--font-mono);
  font-size: var(--font-caption-md);
  line-height: 1.55;
  color: var(--text-primary);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.contact-copy {
  flex: none;
  min-width: 76px;
}

.contact-copy.copied {
  border-color: var(--color-success);
  color: var(--color-success);
}

.contact-visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
}

/* Sections */
.contact-section {
  padding-top: 88px;
}

.contact-section-head {
  max-width: 64ch;
  margin-bottom: 32px;
}

.contact-section-head p { margin-top: 10px; }

/* For vibe coders */
.contact .contact-vibe-lead {
  margin: 14px 0 24px;
  max-width: 44ch;
  font-size: var(--font-body-lg);
  color: var(--text-primary);
}

/* Plans */
.contact .contact-plan-price {
  margin-top: 6px;
  font-size: var(--font-heading-md);
  font-weight: 750;
  color: var(--text-primary);
}

/* Use cases */
/* Contact */
.contact-contact {
  display: grid;
  grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr);
  gap: 48px;
}

.contact-contact-intro p { margin-top: 12px; }

.contact .contact-next { color: var(--text-tertiary); }

.lead-form {
  position: relative;
  padding: 28px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--radius-xl);
  background: rgba(255, 255, 255, 0.03);
  display: grid;
  gap: 20px;
}

.lead-field { display: grid; gap: 8px; }

.lead-field label {
  font-size: var(--font-body-sm);
  font-weight: 650;
  color: var(--text-primary);
}

.lead-field .lead-need {
  font-weight: 400;
  color: var(--text-tertiary);
}

.lead-field input {
  width: 100%;
  height: 48px;
  padding: 0 14px;
  border-radius: var(--radius-md);
  border: 1px solid rgba(255, 255, 255, 0.18);
  background: rgba(0, 0, 0, 0.35);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-body-md);
}

.lead-field input[aria-invalid="true"] { border-color: var(--color-error); }

.contact .lead-hint {
  font-size: var(--font-caption-md);
  color: var(--text-tertiary);
}

.contact .lead-error {
  font-size: var(--font-caption-md);
  color: var(--color-error);
}

.lead-trap {
  position: absolute;
  left: -10000px;
  width: 1px;
  height: 1px;
  overflow: hidden;
}

.lead-submit { justify-self: start; }

.contact .lead-privacy,
.contact .lead-fallback { font-size: var(--font-caption-md); }

.lead-privacy a,
.lead-fallback a,
.lead-status a {
  color: var(--accent-primary);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.lead-status {
  padding: 16px 18px;
  border-radius: var(--radius-md);
  border: 1px solid rgba(255, 255, 255, 0.14);
}

/* Empty until the form has something to say: out of the flow, so it leaves no gap, but still in the
   page, so a reader hears it arrive. Turnstile's box likewise, until the widget needs the visitor. */
.lead-status:empty,
.lead-turnstile:empty {
  position: absolute;
  padding: 0;
  border: 0;
}

.lead-status.sent { border-color: var(--color-success); }
.lead-status.failed { border-color: var(--color-error); }
.lead-status:focus { outline: none; }
.lead-status:focus-visible { outline: 2px solid var(--accent-primary); outline-offset: 3px; }

.contact .lead-status-title {
  font-weight: 650;
  color: var(--text-primary);
}

.lead-status p + p { margin-top: 8px; }

/* FAQ */
.contact-faq {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 28px 48px;
}

.contact-faq dt {
  font-size: var(--font-body-lg);
  font-weight: 700;
  line-height: 1.4;
  color: var(--text-primary);
}

.contact-faq dd { margin: 8px 0 0; }

.contact-faq dd a {
  color: var(--accent-primary);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.contact .contact-faq-note {
  margin-top: 28px;
  font-size: var(--font-caption-md);
  color: var(--text-tertiary);
}

@media (max-width: 900px) {
  .contact-hero,
  .contact-vibe,
  .contact-plans,
  .contact-contact,
  .contact-faq {
    grid-template-columns: minmax(0, 1fr);
  }

  .contact-hero { gap: 40px; padding: 48px 0 40px; }
  .contact-vibe,
  .contact-contact { gap: 28px; }
}

@media (max-width: 640px) {
  .contact-main { padding: 0 var(--spacing-lg) 72px; }
  .contact-command,
  .contact .contact-install-title { padding-left: var(--spacing-md); padding-right: var(--spacing-md); }
  .contact h1 { font-size: var(--font-display-md); }
  .contact-section { padding-top: 64px; }
  .contact-cases { grid-template-columns: minmax(0, 1fr); }
  .contact .contact-try-label { flex-basis: 100%; }
  .contact-vibe-file,
  .contact-vibe-output { padding-left: var(--spacing-lg); padding-right: var(--spacing-lg); }
  .contact-vibe-code {
    padding: var(--spacing-lg);
    font-size: var(--font-caption-md);
  }
  .contact-plan,
  .contact-case,
  .lead-form { padding: 20px; }
  .contact-actions .contact-btn,
  .lead-submit { width: 100%; }
  .contact .contact-license-table td:last-child { width: auto; }
}

@media (prefers-reduced-motion: reduce) {
  .contact *,
  .contact *::before,
  .contact *::after {
    animation: none !important;
    transition: none !important;
  }
}
"#;

#[component]
pub fn Contact(section: String) -> Element {
    // `/contact#<section>` opens at that section, on arrival and on any later link to one.
    use_effect(use_reactive!(|section| {
        if !section.is_empty() {
            scroll_to_section(section);
        }
    }));

    let faq_pairs: Vec<(&str, &str)> = content::faq().iter().map(|f| (f.question.as_str(), f.answer.as_str())).collect();
    let schemas = vec![
        organization_contact_schema(TEAM_EMAIL, "/contact#contact"),
        faq_schema(&faq_pairs),
        breadcrumb_schema(&[BreadcrumbItem { name: "Home", path: "/" }, BreadcrumbItem { name: "Contact", path: "/contact" }]),
    ];

    rsx! {
        PageHead {
            title: seo_pages::CONTACT.title,
            description: seo_pages::CONTACT.description,
            canonical_path: seo_pages::CONTACT.canonical_path,
        }
        style { "{CONTACT_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "contact",
            MainNav { active: ActivePage::Contact, subtitle: Some("Contact") }
            main { class: "contact-main",
                Hero {}
                ContactSection {}
                Questions {}
            }
            Footer {}
        }
    }
}

/// A link to a section of this page. Following it also scrolls there, so a second click on a
/// link to the section the address already names still brings it into view (the route does not
/// change then, so the page's own scroll on a new section never runs).
#[component]
fn SectionLink(
    #[props(into)] id: String,
    #[props(into)] class: Option<String>,
    #[props(into)] aria_label: Option<String>,
    children: Element,
) -> Element {
    let target = id.clone();
    rsx! {
        Link {
            to: Route::Contact { section: id },
            class,
            aria_label,
            onclick: move |_| scroll_to_section(target.clone()),
            {children}
        }
    }
}

#[component]
fn Hero() -> Element {
    rsx! {
        section { id: "top", class: "contact-hero",
            div {
                h1 { "Contact the Team" }
                p { class: "contact-lede", "{OPEN_SOURCE_LINE}" }
                div { class: "contact-actions",
                    SectionLink { id: "contact", class: "contact-btn primary", "{CONTACT_HEADING}" }
                    Link { to: Route::Studio { file: None }, class: "contact-btn", "Try it in the browser" }
                }
            }
            div { id: "install", class: "contact-install",
                h2 { class: "contact-install-title", "Install in one line" }
                InstallCommand { platform: "macOS and Linux", command: INSTALL_UNIX }
                InstallCommand { platform: "Windows (PowerShell)", command: INSTALL_WINDOWS }
            }
        }
    }
}

/// One install command with its Copy button. The button copies the constant, never the
/// wrapped text on screen.
#[component]
fn InstallCommand(platform: &'static str, command: &'static str) -> Element {
    let mut copied = use_signal(|| false);
    rsx! {
        div { class: "contact-command",
            div { class: "contact-command-head",
                p { class: "contact-command-label", "{platform}" }
                button {
                    r#type: "button",
                    class: if copied() { "contact-btn contact-copy copied" } else { "contact-btn contact-copy" },
                    aria_label: "Copy the {platform} install command",
                    onclick: move |_| {
                        copy_to_clipboard(command);
                        copied.set(true);
                    },
                    if copied() { "Copied" } else { "Copy" }
                }
            }
            code { class: "contact-command-text", "{command}" }
            span { class: "contact-visually-hidden", aria_live: "polite",
                if copied() { "Copied the {platform} install command" }
            }
        }
    }
}

fn copy_to_clipboard(text: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().clipboard().write_text(text);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = text;
}

#[component]
fn ContactSection() -> Element {
    rsx! {
        section { id: "contact", class: "contact-section contact-contact",
            div { class: "contact-contact-intro",
                h2 { id: "contact-heading", "{CONTACT_HEADING}" }
                p {
                    "Leave your email, and a phone number if you'd like a call. The team will get back to you."
                }
                p { class: "contact-next",
                    "What happens next: an engineer replies, with a few questions about how you plan to use LOGOS."
                }
            }
            LeadForm {}
        }
    }
}

#[component]
fn Questions() -> Element {
    rsx! {
        section { id: "faq", class: "contact-section",
            div { class: "contact-section-head",
                h2 { "FAQ" }
            }
            dl { class: "contact-faq",
                for item in content::faq() {
                    div { class: "contact-faq-item",
                        dt { "{item.question}" }
                        dd { FaqAnswer { item: item } }
                    }
                }
            }
        }
    }
}

/// An answer, with its link run (if any) shown as a link.
#[component]
fn FaqAnswer(item: &'static Faq) -> Element {
    let Some(link) = &item.link else {
        return rsx! { "{item.answer}" };
    };
    let (before, after) = item.answer.split_once(link.text).unwrap_or((item.answer.as_str(), ""));
    rsx! {
        "{before}"
        a { href: link.href, target: "_blank", rel: "noopener noreferrer", "{link.text}" }
        "{after}"
    }
}
