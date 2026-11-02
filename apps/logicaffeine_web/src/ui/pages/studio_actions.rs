//! What the Studio's toolbar actions can do for the file in the editor.
//!
//! Every mode has its buttons — Run, Compile and Debug in Code; Execute and Compile in Logic, Math
//! and Hardware — but not every file can use every one: a Verilog design is already hardware, a
//! grid puzzle has no Rust to extract, a program built on live networking cannot run in the
//! browser. A button that can do nothing for the open file stays where it is, disabled, and says
//! why in its tooltip. [`actions_for`] is the one place that decides; the toolbar draws from it.

use crate::ui::state::StudioMode;

/// Whether an action can do something for the open file — and why not, when it cannot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    Ready,
    Unavailable(String),
}

impl Offer {
    pub fn is_ready(&self) -> bool {
        matches!(self, Offer::Ready)
    }

    /// Why the action is unavailable, for its tooltip.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Offer::Ready => None,
            Offer::Unavailable(why) => Some(why),
        }
    }
}

/// The toolbar's actions for one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actions {
    /// Run in Code mode; Execute in Logic, Math and Hardware.
    pub run: Offer,
    pub compile: Offer,
    /// The Code debugger.
    pub debug: Offer,
}

/// The actions the open file can take, from its mode, its path (the registry knows which shipped
/// programs need the compiled runtime) and its text.
pub fn actions_for(mode: StudioMode, path: Option<&str>, content: &str) -> Actions {
    let unavailable = |why: &str| Offer::Unavailable(why.to_string());
    if content.trim().is_empty() {
        let empty = || unavailable("The editor is empty. Write something first.");
        return Actions { run: empty(), compile: empty(), debug: empty() };
    }
    let only_code = || unavailable("The debugger steps through Code programs.");
    match mode {
        StudioMode::Code => {
            let run = match path.and_then(crate::ui::examples::compiled_runtime_notice) {
                Some(notice) => Offer::Unavailable(notice),
                None => Offer::Ready,
            };
            Actions { debug: run.clone(), run, compile: Offer::Ready }
        }
        StudioMode::Logic => Actions {
            run: Offer::Ready,
            compile: match logicaffeine_compile::logic_compile_note(content) {
                Some(note) => unavailable(note),
                None => Offer::Ready,
            },
            debug: only_code(),
        },
        StudioMode::Math => Actions {
            run: Offer::Ready,
            compile: if is_theory(content) {
                unavailable("A theory is certified by Execute. Compile turns Definitions and Inductives into Rust, and a theory declares none.")
            } else if !declares_anything(content) {
                unavailable("Compile turns Definitions and Inductives into Rust, and this file declares none. Execute runs its Checks and Evals.")
            } else {
                Offer::Ready
            },
            debug: only_code(),
        },
        StudioMode::Hardware => Actions {
            run: Offer::Ready,
            compile: match crate::ui::pages::hardware_report::kind_of(content)
                .and_then(crate::ui::pages::hardware_report::nothing_to_compile)
            {
                Some(note) => unavailable(note),
                None => Offer::Ready,
            },
            debug: only_code(),
        },
        StudioMode::Web => {
            let live = || unavailable("The preview runs the app as you type.");
            Actions { run: live(), compile: live(), debug: live() }
        }
    }
}

/// A `## Theory` development: axioms and cited theorems the multi-theorem driver certifies.
fn is_theory(content: &str) -> bool {
    content.lines().any(|line| line.trim_start().starts_with("## Theory"))
}

/// Whether a Math file declares anything extraction could turn into Rust — a statement in one of
/// the kernel's defining forms (`command_parser::parse_command`), not a `Check` or an `Eval`.
fn declares_anything(content: &str) -> bool {
    crate::ui::pages::studio::parse_math_statements(content).iter().any(|statement| {
        let s = statement.trim_start();
        s.starts_with("Definition")
            || s.starts_with("Inductive")
            || s.starts_with("## To ")
            || s.starts_with("## Theorem:")
            || (s.starts_with("Let ") && s.contains(" be "))
            || ((s.starts_with("A ") || s.starts_with("An ")) && s.contains(" is either"))
    })
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_actions/tests.rs"]
mod tests;
