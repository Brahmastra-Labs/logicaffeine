//! Vernacular commands for interacting with the Kernel.

use crate::kernel::Term;

/// A command in the Vernacular language.
#[derive(Debug, Clone)]
pub enum Command {
    /// Definition name : type := body.
    ///
    /// If `ty` is None, the type is inferred from the body.
    /// If `is_hint` is true, register as a hint for auto tactic.
    Definition {
        name: String,
        ty: Option<Term>,
        body: Term,
        is_hint: bool,
    },

    /// Check term.
    ///
    /// Prints the type of the term.
    Check(Term),

    /// Eval term.
    ///
    /// Normalizes and prints the term.
    Eval(Term),

    /// Inductive Name (params) := C1 : T1 | C2 : T2.
    ///
    /// Defines an inductive type with its constructors.
    /// Supports optional type parameters for polymorphic inductives.
    Inductive {
        name: String,
        /// Type parameters: (param_name, param_type)
        /// e.g., for `List (A : Type)`, params = [("A", Type)]
        params: Vec<(String, Term)>,
        sort: Term,
        constructors: Vec<(String, Term)>,
    },
}
