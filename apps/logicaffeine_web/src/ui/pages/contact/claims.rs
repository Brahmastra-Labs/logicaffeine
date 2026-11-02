//! The figures the pricing page states, read from where they are set.
//!
//! The Learn counts come from the curriculum, so a
//! release or a new module moves the page with it.


use crate::content::ContentEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LearnFigures {
    pub modules: usize,
    pub exercises: usize,
}

pub fn learn_figures() -> LearnFigures {
    let modules = ContentEngine::shared().eras().iter().flat_map(|era| &era.modules);
    LearnFigures {
        modules: modules.clone().count(),
        exercises: modules.map(|module| module.exercises.len()).sum(),
    }
}

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/contact/claims/tests.rs"]
mod tests;
