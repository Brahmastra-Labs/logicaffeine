//! What the contact page says, as data.
//!
//! The install commands, the FAQ and the few lines the page states live here rather than in the
//! markup, so they read and review in one place and every line that states a fact carries the
//! suites that prove it. `contact_page` in logicaffeine_tests holds every suite to existing, the
//! page to showing every claim, and the page to a list of claims the product does not back.
//! Figures come from [`claims`](super::claims), never from here.

use std::sync::LazyLock;


/// The team's address: the page's fallback, where the form's requests arrive, and
/// `public/llms.txt`.
pub const TEAM_EMAIL: &str = "team@datacenter.dev";

/// The install one-liners, exactly as the Syntax Guide's Getting Started section gives them.
pub const INSTALL_UNIX: &str = "curl -fsSL https://logicaffeine.com/install.sh | sh";
pub const INSTALL_WINDOWS: &str = "irm https://logicaffeine.com/install.ps1 | iex";

/// The hero's call to the form, and the form's heading. The prerender smoke tests look for it.
pub const CONTACT_HEADING: &str = "Contact the team";

/// The form's button.
pub const SEND: &str = "Send";

/// The page's one line on what LOGOS is: open source, under MIT, for anyone.
pub const OPEN_SOURCE_LINE: &str = "LOGOS is open source under the MIT License. Use it for anything, for anyone, at any size.";

pub const LICENSE_URL: &str = "https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md";
pub const SOURCE_URL: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

/// The guide an AI assistant reads before it writes LOGOS: every documentation guide in one file,
/// served from `public/`.
pub const ASSISTANT_GUIDE_URL: &str = "https://logicaffeine.com/llms-full.txt";

/// What backs a line on the page.
pub enum Evidence {
    /// The suites under `crates/logicaffeine_tests/tests/` that prove the capability.
    Suites(&'static [&'static str]),
}

/// One line the page states, and what backs it.
pub struct Claim {
    pub text: String,
    pub evidence: Evidence,
}

fn proven(text: impl Into<String>, suites: &'static [&'static str]) -> Claim {
    Claim { text: text.into(), evidence: Evidence::Suites(suites) }
}

#[derive(PartialEq)]
pub struct FaqLink {
    pub text: &'static str,
    pub href: &'static str,
}

#[derive(PartialEq)]
pub struct Faq {
    pub question: String,
    pub answer: String,
    /// A run of the answer shown as a link.
    pub link: Option<FaqLink>,
}

/// Whether LOGOS is free, as the landing page's FAQ answers it: open source, under MIT.
pub fn free_answer() -> String {
    format!("Yes. {OPEN_SOURCE_LINE}")
}

pub fn faq() -> &'static [Faq] {
    static FAQ: LazyLock<Vec<Faq>> = LazyLock::new(|| {
        let plain = |question: &str, answer: String| Faq { question: question.to_string(), answer, link: None };
        vec![
            plain("Is LOGOS free?", free_answer()),
            Faq {
                question: "What is the license?".into(),
                answer: "The MIT License, the whole of it in LICENSE.md.".into(),
                link: Some(FaqLink { text: "LICENSE.md", href: LICENSE_URL }),
            },
            Faq {
                question: "Where is the source?".into(),
                answer: "On GitHub: github.com/Brahmastra-Labs/logicaffeine".into(),
                link: Some(FaqLink { text: "github.com/Brahmastra-Labs/logicaffeine", href: SOURCE_URL }),
            },
            plain("How do I reach the team?", format!("Send the form on this page, or write to {TEAM_EMAIL}.")),
        ]
    });
    &FAQ
}

/// Every line the page states with its evidence: the one line on what LOGOS is.
pub fn every_claim() -> Vec<(String, &'static Evidence)> {
    static OPEN: LazyLock<Claim> = LazyLock::new(|| proven(OPEN_SOURCE_LINE, &["version_lock"]));
    vec![(OPEN.text.clone(), &OPEN.evidence)]
}
