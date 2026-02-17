use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::ast::logic::LogicExpr;
use crate::ast::stmt::Stmt;
use crate::intern::{Interner, Symbol};

use super::{codegen_assertion, codegen_expr};

// =============================================================================
// Refinement Type Enforcement
// =============================================================================

/// Tracks refinement type constraints across scopes for mutation enforcement.
///
/// When a variable with a refinement type is defined, its constraint is registered
/// in the current scope. When that variable is mutated via `Set`, the assertion is
/// re-emitted to ensure the invariant is preserved.
///
/// # Scope Management
///
/// The context maintains a stack of scopes to handle nested blocks:
///
/// ```text
/// ┌─────────────────────────────┐
/// │ Global Scope               │ ← x: { it > 0 }
/// │  ┌──────────────────────┐  │
/// │  │ Zone Scope           │  │ ← y: { it < 100 }
/// │  │  ┌────────────────┐  │  │
/// │  │  │ If Block Scope │  │  │ ← z: { it != 0 }
/// │  │  └────────────────┘  │  │
/// │  └──────────────────────┘  │
/// └─────────────────────────────┘
/// ```
///
/// # Variable Type Tracking
///
/// The context also tracks variable types for capability resolution. This allows
/// statements like `Check that user can publish the document` to resolve "the document"
/// to a variable named `doc` of type `Document`.
pub struct RefinementContext<'a> {
    /// Stack of scopes. Each scope maps variable Symbol to (bound_var, predicate).
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,

    /// Maps variable name Symbol to Rust type name (for capability resolution and optimization).
    variable_types: HashMap<Symbol, String>,

    /// Stack of scopes tracking which bindings came from boxed enum fields.
    /// When these are used in expressions, they need to be dereferenced with `*`.
    boxed_binding_scopes: Vec<HashSet<Symbol>>,

    /// Tracks variables that are known to be String type.
    /// Used for proper string concatenation codegen (format! vs +).
    string_vars: HashSet<Symbol>,
}

impl<'a> RefinementContext<'a> {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: HashMap::new(),
            boxed_binding_scopes: vec![HashSet::new()],
            string_vars: HashSet::new(),
        }
    }

    /// Create a RefinementContext seeded from a TypeEnv.
    pub fn from_type_env(type_env: &crate::analysis::types::TypeEnv) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: type_env.to_legacy_variable_types(),
            boxed_binding_scopes: vec![HashSet::new()],
            string_vars: type_env.to_legacy_string_vars(),
        }
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.boxed_binding_scopes.push(HashSet::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
        self.boxed_binding_scopes.pop();
    }

    /// Register a binding that came from a boxed enum field.
    /// These need `*` dereferencing when used in expressions.
    pub(super) fn register_boxed_binding(&mut self, var: Symbol) {
        if let Some(scope) = self.boxed_binding_scopes.last_mut() {
            scope.insert(var);
        }
    }

    /// Check if a variable is a boxed binding (needs dereferencing).
    pub(super) fn is_boxed_binding(&self, var: Symbol) -> bool {
        for scope in self.boxed_binding_scopes.iter().rev() {
            if scope.contains(&var) {
                return true;
            }
        }
        false
    }

    /// Register a variable as having String type.
    pub(super) fn register_string_var(&mut self, var: Symbol) {
        self.string_vars.insert(var);
    }

    /// Check if a variable is known to be a String.
    pub(super) fn is_string_var(&self, var: Symbol) -> bool {
        self.string_vars.contains(&var)
    }

    /// Get a reference to the string_vars set for expression codegen.
    pub(super) fn get_string_vars(&self) -> &HashSet<Symbol> {
        &self.string_vars
    }

    pub(super) fn register(&mut self, var: Symbol, bound_var: Symbol, predicate: &'a LogicExpr<'a>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, (bound_var, predicate));
        }
    }

    pub(super) fn get_constraint(&self, var: Symbol) -> Option<(Symbol, &'a LogicExpr<'a>)> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(&var) {
                return Some(*entry);
            }
        }
        None
    }

    /// Register a variable with its type for capability resolution.
    pub(super) fn register_variable_type(&mut self, var: Symbol, type_name: String) {
        self.variable_types.insert(var, type_name);
    }

    /// Get variable type map for expression codegen optimization.
    pub(super) fn get_variable_types(&self) -> &HashMap<Symbol, String> {
        &self.variable_types
    }

    /// Find a variable name by its type (for resolving "the document" to "doc").
    pub(super) fn find_variable_by_type(&self, type_name: &str, interner: &Interner) -> Option<String> {
        let type_lower = type_name.to_lowercase();
        for (var_sym, var_type) in &self.variable_types {
            if var_type.to_lowercase() == type_lower {
                return Some(interner.resolve(*var_sym).to_string());
            }
        }
        None
    }
}

/// Emits a debug_assert for a refinement predicate, substituting the bound variable.
pub(super) fn emit_refinement_check(
    var_name: &str,
    bound_var: Symbol,
    predicate: &LogicExpr,
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let assertion = codegen_assertion(predicate, interner);
    let bound = interner.resolve(bound_var);
    let check = if bound == var_name {
        assertion
    } else {
        replace_word(&assertion, bound, var_name)
    };
    writeln!(output, "{}debug_assert!({});", indent_str, check).unwrap();
}

/// Word-boundary replacement to substitute bound variable with actual variable.
pub(super) fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if !word.is_empty() {
                result.push_str(if word == from { to } else { &word });
                word.clear();
            }
            result.push(c);
        }
    }
    if !word.is_empty() {
        result.push_str(if word == from { to } else { &word });
    }
    result
}

// =============================================================================
// Mount+Sync Detection for Distributed<T>
// =============================================================================

/// Tracks which variables have Mount and/or Sync statements.
///
/// This is used to detect when a variable needs `Distributed<T>` instead of
/// separate persistence and synchronization wrappers. A variable that is both
/// mounted and synced can use the unified `Distributed<T>` type.
///
/// # Detection Flow
///
/// ```text
/// Pre-scan all statements
///       ↓
/// Found "Mount x at path"  →  x.mounted = true, x.mount_path = Some(path)
/// Found "Sync x on topic"  →  x.synced = true, x.sync_topic = Some(topic)
///       ↓
/// If x.mounted && x.synced  →  Use Distributed<T> with both
/// ```
#[derive(Debug, Default)]
pub struct VariableCapabilities {
    /// Variable has a Mount statement (persistence).
    pub(super) mounted: bool,
    /// Variable has a Sync statement (network synchronization).
    pub(super) synced: bool,
    /// Path expression for Mount (as generated code string).
    pub(super) mount_path: Option<String>,
    /// Topic expression for Sync (as generated code string).
    pub(super) sync_topic: Option<String>,
}

/// Helper to create an empty VariableCapabilities map (for tests).
pub fn empty_var_caps() -> HashMap<Symbol, VariableCapabilities> {
    HashMap::new()
}

/// Pre-scan statements to detect variables that have both Mount and Sync.
/// Returns a map from variable Symbol to its capabilities.
pub(super) fn analyze_variable_capabilities<'a>(
    stmts: &[Stmt<'a>],
    interner: &Interner,
) -> HashMap<Symbol, VariableCapabilities> {
    let mut caps: HashMap<Symbol, VariableCapabilities> = HashMap::new();
    let empty_synced = HashSet::new();

    for stmt in stmts {
        match stmt {
            Stmt::Mount { var, path } => {
                let entry = caps.entry(*var).or_default();
                entry.mounted = true;
                entry.mount_path = Some(codegen_expr(path, interner, &empty_synced));
            }
            Stmt::Sync { var, topic } => {
                let entry = caps.entry(*var).or_default();
                entry.synced = true;
                entry.sync_topic = Some(codegen_expr(topic, interner, &empty_synced));
            }
            // Recursively check nested blocks (Block<'a> is &[Stmt<'a>])
            Stmt::If { then_block, else_block, .. } => {
                let nested = analyze_variable_capabilities(then_block, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
                if let Some(else_b) = else_block {
                    let nested = analyze_variable_capabilities(else_b, interner);
                    for (var, cap) in nested {
                        let entry = caps.entry(var).or_default();
                        if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                        if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                let nested = analyze_variable_capabilities(body, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
            }
            _ => {}
        }
    }

    caps
}
