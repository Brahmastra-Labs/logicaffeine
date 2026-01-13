//! Typing context for the kernel.
//!
//! A context maps variable names to their types.
//! Used during type checking to track what variables are in scope.

use super::term::Term;
use std::collections::HashMap;

/// Typing context: maps variable names to their types.
///
/// The context is immutable-by-default: `extend` creates a new context
/// with the additional binding, preserving the original.
///
/// Also stores global definitions:
/// - Inductive types (e.g., Nat : Type 0)
/// - Constructors (e.g., Zero : Nat, Succ : Nat -> Nat)
/// - Declarations (e.g., hypotheses like h1 : P -> Q)
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Local variable bindings (from λ and Π)
    bindings: HashMap<String, Term>,

    /// Inductive type definitions: name -> sort (e.g., "Nat" -> Type 0)
    inductives: HashMap<String, Term>,

    /// Constructor definitions: name -> (inductive_name, type)
    constructors: HashMap<String, (String, Term)>,

    /// Order of constructor registration per inductive.
    /// HashMap doesn't preserve insertion order, so we track it explicitly.
    constructor_order: HashMap<String, Vec<String>>,

    /// Declaration bindings (axioms/hypotheses): name -> type
    /// Used for certifying proofs where hypotheses are assumed.
    declarations: HashMap<String, Term>,

    /// Definition bodies: name -> (type, body)
    /// Definitions are transparent - they unfold during normalization.
    /// Distinguished from declarations (axioms) which have no body.
    definitions: HashMap<String, (Term, Term)>,

    /// Hint database: theorem names marked as hints for auto tactic.
    /// When auto fails with decision procedures, it tries to apply these hints.
    hints: Vec<String>,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Context {
            bindings: HashMap::new(),
            inductives: HashMap::new(),
            constructors: HashMap::new(),
            constructor_order: HashMap::new(),
            declarations: HashMap::new(),
            definitions: HashMap::new(),
            hints: Vec::new(),
        }
    }

    /// Add a local binding to this context (mutates in place).
    pub fn add(&mut self, name: &str, ty: Term) {
        self.bindings.insert(name.to_string(), ty);
    }

    /// Look up a local variable's type in the context.
    pub fn get(&self, name: &str) -> Option<&Term> {
        self.bindings.get(name)
    }

    /// Create a new context extended with an additional local binding.
    ///
    /// Does not mutate the original context.
    pub fn extend(&self, name: &str, ty: Term) -> Context {
        let mut new_ctx = self.clone();
        new_ctx.add(name, ty);
        new_ctx
    }

    /// Register an inductive type.
    ///
    /// The `sort` is the type of the inductive (e.g., Type 0 for Nat).
    pub fn add_inductive(&mut self, name: &str, sort: Term) {
        self.inductives.insert(name.to_string(), sort);
    }

    /// Register a constructor for an inductive type.
    ///
    /// The `ty` is the full type of the constructor
    /// (e.g., `Nat` for Zero, `Nat -> Nat` for Succ).
    ///
    /// Constructors are tracked in registration order for match expressions.
    pub fn add_constructor(&mut self, name: &str, inductive: &str, ty: Term) {
        self.constructors
            .insert(name.to_string(), (inductive.to_string(), ty));

        // Track constructor order for this inductive
        self.constructor_order
            .entry(inductive.to_string())
            .or_default()
            .push(name.to_string());
    }

    /// Add a declaration (typed assumption/hypothesis).
    ///
    /// Used for proof certification where hypotheses are assumed.
    /// Example: h1 : P -> Q
    pub fn add_declaration(&mut self, name: &str, ty: Term) {
        self.declarations.insert(name.to_string(), ty);
    }

    /// Register a definition: name : type := body
    ///
    /// Definitions are transparent and unfold during normalization (delta reduction).
    /// This distinguishes them from declarations (axioms) which have no body.
    pub fn add_definition(&mut self, name: String, ty: Term, body: Term) {
        self.definitions.insert(name, (ty, body));
    }

    /// Look up a global definition (inductive, constructor, definition, or declaration).
    ///
    /// Returns the type of the global.
    pub fn get_global(&self, name: &str) -> Option<&Term> {
        // Check inductives first
        if let Some(sort) = self.inductives.get(name) {
            return Some(sort);
        }
        // Check constructors
        if let Some((_, ty)) = self.constructors.get(name) {
            return Some(ty);
        }
        // Check definitions (return type, not body)
        if let Some((ty, _)) = self.definitions.get(name) {
            return Some(ty);
        }
        // Check declarations (axioms)
        self.declarations.get(name)
    }

    /// Check if a name is a definition (has a body that can be unfolded).
    pub fn is_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    /// Get the body of a definition, if it exists.
    ///
    /// Returns None for axioms, constructors, and inductives (only definitions have bodies).
    pub fn get_definition_body(&self, name: &str) -> Option<&Term> {
        self.definitions.get(name).map(|(_, body)| body)
    }

    /// Get the type of a definition, if it exists.
    pub fn get_definition_type(&self, name: &str) -> Option<&Term> {
        self.definitions.get(name).map(|(ty, _)| ty)
    }

    /// Check if a name is a constructor.
    pub fn is_constructor(&self, name: &str) -> bool {
        self.constructors.contains_key(name)
    }

    /// Get the inductive type a constructor belongs to.
    pub fn constructor_inductive(&self, name: &str) -> Option<&str> {
        self.constructors.get(name).map(|(ind, _)| ind.as_str())
    }

    /// Check if a name is an inductive type.
    pub fn is_inductive(&self, name: &str) -> bool {
        self.inductives.contains_key(name)
    }

    /// Get all constructors for an inductive type, in registration order.
    ///
    /// Returns a vector of (constructor_name, constructor_type) pairs.
    pub fn get_constructors(&self, inductive: &str) -> Vec<(&str, &Term)> {
        self.constructor_order
            .get(inductive)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| {
                        self.constructors
                            .get(name)
                            .map(|(_, ty)| (name.as_str(), ty))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Iterate over all declarations (hypotheses).
    ///
    /// Used by the certifier to find hypothesis by type.
    pub fn iter_declarations(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.declarations.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all definitions.
    ///
    /// Used by the UI to display definitions.
    pub fn iter_definitions(&self) -> impl Iterator<Item = (&str, &Term, &Term)> {
        self.definitions.iter().map(|(k, (ty, body))| (k.as_str(), ty, body))
    }

    /// Iterate over all inductive types.
    ///
    /// Used by the UI to display inductive types.
    pub fn iter_inductives(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.inductives.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Add a constructor with strict positivity checking.
    ///
    /// Returns an error if the inductive type appears negatively in the
    /// constructor type. This prevents paradoxes like:
    /// ```text
    /// Inductive Bad := Cons : (Bad -> False) -> Bad
    /// ```
    pub fn add_constructor_checked(
        &mut self,
        name: &str,
        inductive: &str,
        ty: Term,
    ) -> super::error::KernelResult<()> {
        // Check strict positivity first
        super::positivity::check_positivity(inductive, name, &ty)?;

        // If it passes, add the constructor normally
        self.add_constructor(name, inductive, ty);
        Ok(())
    }

    /// Register a theorem as a hint for the auto tactic.
    ///
    /// Hints are theorems that auto will try to apply when decision
    /// procedures fail. This allows auto to "learn" from proven theorems.
    pub fn add_hint(&mut self, name: &str) {
        if !self.hints.contains(&name.to_string()) {
            self.hints.push(name.to_string());
        }
    }

    /// Get all registered hints.
    ///
    /// Returns the names of theorems registered as hints.
    pub fn get_hints(&self) -> &[String] {
        &self.hints
    }

    /// Check if a theorem is registered as a hint.
    pub fn is_hint(&self, name: &str) -> bool {
        self.hints.contains(&name.to_string())
    }
}
