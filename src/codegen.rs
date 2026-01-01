use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::analysis::policy::{PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition};
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, ReadSource, Stmt, TypeExpr};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

// =============================================================================
// Phase 43C: Refinement Type Enforcement
// =============================================================================

/// Tracks refinement type constraints across scopes for mutation enforcement.
/// When a variable with a refinement type is defined, we register its constraint.
/// When that variable is mutated via `Set`, we re-emit the assertion.
/// Phase 50: Also tracks variable types for capability Check resolution.
pub struct RefinementContext<'a> {
    /// Stack of scopes. Each scope maps variable Symbol to (bound_var, predicate).
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,
    /// Phase 50: Maps variable name Symbol to type name (for capability resolution)
    /// e.g., "doc" -> "Document" allows "Check that user can publish the document" to resolve to &doc
    variable_types: HashMap<Symbol, String>,
}

impl<'a> RefinementContext<'a> {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: HashMap::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn register(&mut self, var: Symbol, bound_var: Symbol, predicate: &'a LogicExpr<'a>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, (bound_var, predicate));
        }
    }

    fn get_constraint(&self, var: Symbol) -> Option<(Symbol, &'a LogicExpr<'a>)> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(&var) {
                return Some(*entry);
            }
        }
        None
    }

    /// Phase 50: Register a variable with its type for capability resolution
    fn register_variable_type(&mut self, var: Symbol, type_name: String) {
        self.variable_types.insert(var, type_name);
    }

    /// Phase 50: Find a variable name by its type (for resolving "the document" to "doc")
    fn find_variable_by_type(&self, type_name: &str, interner: &Interner) -> Option<String> {
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
fn emit_refinement_check(
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
fn replace_word(text: &str, from: &str, to: &str) -> String {
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
// Phase 56: Mount+Sync Detection for Distributed<T>
// =============================================================================

/// Tracks which variables have Mount and/or Sync statements.
/// Used to detect when a variable needs Distributed<T> instead of separate wrappers.
#[derive(Debug, Default)]
pub struct VariableCapabilities {
    /// Variable has a Mount statement
    mounted: bool,
    /// Variable has a Sync statement
    synced: bool,
    /// Path expression for Mount (as generated code string)
    mount_path: Option<String>,
    /// Topic expression for Sync (as generated code string)
    sync_topic: Option<String>,
}

/// Helper to create an empty VariableCapabilities map (for tests).
pub fn empty_var_caps() -> HashMap<Symbol, VariableCapabilities> {
    HashMap::new()
}

/// Pre-scan statements to detect variables that have both Mount and Sync.
/// Returns a map from variable Symbol to its capabilities.
fn analyze_variable_capabilities<'a>(
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

/// Phase 51: Detect if any statements require async execution.
/// Returns true if the program needs #[tokio::main] async fn main().
fn requires_async(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_async_stmt(s))
}

fn requires_async_stmt(stmt: &Stmt) -> bool {
    match stmt {
        // Phase 9: Concurrent blocks use tokio::join!
        Stmt::Concurrent { tasks } => true,
        // Phase 51: Network operations and Sleep are async
        Stmt::Listen { .. } => true,
        Stmt::ConnectTo { .. } => true,
        Stmt::Sleep { .. } => true,
        // Phase 52: Sync is async (GossipSub subscription)
        Stmt::Sync { .. } => true,
        // Phase 53: Mount is async (VFS file operations)
        Stmt::Mount { .. } => true,
        // Phase 53: File I/O is async (VFS operations)
        Stmt::ReadFrom { source: ReadSource::File(_), .. } => true,
        Stmt::WriteFile { .. } => true,
        // Phase 54: Go-like concurrency is async
        Stmt::LaunchTask { .. } => true,
        Stmt::LaunchTaskWithHandle { .. } => true,
        Stmt::SendPipe { .. } => true,
        Stmt::ReceivePipe { .. } => true,
        Stmt::Select { .. } => true,
        // While and Repeat are now always async due to check_preemption()
        // (handled below in recursive check)
        // Recursively check nested blocks
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| requires_async_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| requires_async_stmt(s)))
        }
        Stmt::While { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Repeat { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Zone { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Parallel { tasks } => tasks.iter().any(|s| requires_async_stmt(s)),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        _ => false,
    }
}

/// Phase 53: Detect if any statements require VFS (Virtual File System).
/// Returns true if the program uses file operations or persistent storage.
fn requires_vfs(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_vfs_stmt(s))
}

fn requires_vfs_stmt(stmt: &Stmt) -> bool {
    match stmt {
        // Phase 53: Mount uses VFS for persistent storage
        Stmt::Mount { .. } => true,
        // Phase 53: File I/O uses VFS
        Stmt::ReadFrom { source: ReadSource::File(_), .. } => true,
        Stmt::WriteFile { .. } => true,
        // Recursively check nested blocks
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| requires_vfs_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| requires_vfs_stmt(s)))
        }
        Stmt::While { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Repeat { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Zone { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Concurrent { tasks } => tasks.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Parallel { tasks } => tasks.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        _ => false,
    }
}

/// Grand Challenge: Collect all variables that need `let mut` in Rust.
/// This includes:
/// - Variables that are targets of `Set` statements (reassignment)
/// - Variables that are targets of `Push` statements (mutation via push)
/// - Variables that are targets of `Pop` statements (mutation via pop)
fn collect_mutable_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in stmts {
        collect_mutable_vars_stmt(stmt, &mut targets);
    }
    targets
}

fn collect_mutable_vars_stmt(stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            targets.insert(*target);
        }
        Stmt::Push { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::Pop { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::Add { collection, .. } => {
            // If collection is an identifier (Set), it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::Remove { collection, .. } => {
            // If collection is an identifier (Set), it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::SetIndex { collection, .. } => {
            // If collection is an identifier, it needs to be mutable
            if let Expr::Identifier(sym) = collection {
                targets.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_mutable_vars_stmt(s, targets);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_mutable_vars_stmt(s, targets);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Zone { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        // Phase 9: Structured Concurrency blocks
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Phase 50: Policy Method Generation
// =============================================================================

/// Generate impl blocks with predicate and capability methods for security policies.
fn codegen_policy_impls(policies: &PolicyRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Collect all types that have policies
    let mut type_predicates: HashMap<Symbol, Vec<&PredicateDef>> = HashMap::new();
    let mut type_capabilities: HashMap<Symbol, Vec<&CapabilityDef>> = HashMap::new();

    for (type_sym, predicates) in policies.iter_predicates() {
        type_predicates.entry(*type_sym).or_insert_with(Vec::new).extend(predicates.iter());
    }

    for (type_sym, capabilities) in policies.iter_capabilities() {
        type_capabilities.entry(*type_sym).or_insert_with(Vec::new).extend(capabilities.iter());
    }

    // Get all types that have any policies
    let mut all_types: HashSet<Symbol> = HashSet::new();
    all_types.extend(type_predicates.keys().copied());
    all_types.extend(type_capabilities.keys().copied());

    // Generate impl block for each type
    for type_sym in all_types {
        let type_name = interner.resolve(type_sym);

        writeln!(output, "impl {} {{", type_name).unwrap();

        // Generate predicate methods
        if let Some(predicates) = type_predicates.get(&type_sym) {
            for pred in predicates {
                let pred_name = interner.resolve(pred.predicate_name).to_lowercase();
                writeln!(output, "    pub fn is_{}(&self) -> bool {{", pred_name).unwrap();
                let condition_code = codegen_policy_condition(&pred.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        // Generate capability methods
        if let Some(capabilities) = type_capabilities.get(&type_sym) {
            for cap in capabilities {
                let action_name = interner.resolve(cap.action).to_lowercase();
                let object_type = interner.resolve(cap.object_type);
                let object_param = object_type.to_lowercase();

                writeln!(output, "    pub fn can_{}(&self, {}: &{}) -> bool {{",
                         action_name, object_param, object_type).unwrap();
                let condition_code = codegen_policy_condition(&cap.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Generate Rust code for a policy condition.
fn codegen_policy_condition(condition: &PolicyCondition, interner: &Interner) -> String {
    match condition {
        PolicyCondition::FieldEquals { field, value, is_string_literal } => {
            let field_name = interner.resolve(*field);
            let value_str = interner.resolve(*value);
            if *is_string_literal {
                format!("self.{} == \"{}\"", field_name, value_str)
            } else {
                format!("self.{} == {}", field_name, value_str)
            }
        }
        PolicyCondition::FieldBool { field, value } => {
            let field_name = interner.resolve(*field);
            format!("self.{} == {}", field_name, value)
        }
        PolicyCondition::Predicate { subject: _, predicate } => {
            let pred_name = interner.resolve(*predicate).to_lowercase();
            format!("self.is_{}()", pred_name)
        }
        PolicyCondition::ObjectFieldEquals { subject: _, object, field } => {
            let object_name = interner.resolve(*object).to_lowercase();
            let field_name = interner.resolve(*field);
            format!("self == &{}.{}", object_name, field_name)
        }
        PolicyCondition::Or(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} || {}", left_code, right_code)
        }
        PolicyCondition::And(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} && {}", left_code, right_code)
        }
    }
}

/// Collect LWWRegister field paths for special handling in SetField codegen.
/// Returns a set of (type_name, field_name) pairs where the field is an LWWRegister.
fn collect_lww_fields(registry: &TypeRegistry, interner: &Interner) -> HashSet<(String, String)> {
    let mut lww_fields = HashSet::new();
    for (type_sym, def) in registry.iter_types() {
        if let TypeDef::Struct { fields, .. } = def {
            let type_name = interner.resolve(*type_sym).to_string();
            for field in fields {
                if let FieldType::Generic { base, .. } = &field.ty {
                    let base_name = interner.resolve(*base);
                    if base_name == "LastWriteWins" {
                        let field_name = interner.resolve(field.name).to_string();
                        lww_fields.insert((type_name.clone(), field_name));
                    }
                }
            }
        }
    }
    lww_fields
}

/// Phase 54: Collect function names that are async.
/// Used by LaunchTask codegen to determine if .await is needed.
fn collect_async_functions(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut async_fns = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            if body.iter().any(|s| requires_async_stmt(s)) {
                async_fns.insert(*name);
            }
        }
    }
    async_fns
}

/// Phase 54: Collect parameters that are used as pipe senders in function body.
/// If a param appears in `SendPipe { pipe: Expr::Identifier(param) }`, it's a sender.
fn collect_pipe_sender_params(body: &[Stmt]) -> HashSet<Symbol> {
    let mut senders = HashSet::new();
    for stmt in body {
        collect_pipe_sender_params_stmt(stmt, &mut senders);
    }
    senders
}

fn collect_pipe_sender_params_stmt(stmt: &Stmt, senders: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::SendPipe { pipe, .. } | Stmt::TrySendPipe { pipe, .. } => {
            if let Expr::Identifier(sym) = pipe {
                senders.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_pipe_sender_params_stmt(s, senders);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_pipe_sender_params_stmt(s, senders);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                collect_pipe_sender_params_stmt(s, senders);
            }
        }
        _ => {}
    }
}

/// Phase 54: Collect variables that are pipe declarations (created with CreatePipe).
/// These have _tx/_rx suffixes, while pipe parameters don't.
fn collect_pipe_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut pipe_vars = HashSet::new();
    for stmt in stmts {
        collect_pipe_vars_stmt(stmt, &mut pipe_vars);
    }
    pipe_vars
}

fn collect_pipe_vars_stmt(stmt: &Stmt, pipe_vars: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::CreatePipe { var, .. } => {
            pipe_vars.insert(*var);
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_pipe_vars_stmt(s, pipe_vars);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
        }
        _ => {}
    }
}

/// Generate complete Rust program with struct definitions and main function.
///
/// Phase 31: Structs are wrapped in `mod user_types` to enforce visibility.
/// Phase 32: Function definitions are emitted before main.
/// Phase 50: Accepts PolicyRegistry to generate security predicate methods.
pub fn codegen_program(stmts: &[Stmt], registry: &TypeRegistry, policies: &PolicyRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Prelude
    writeln!(output, "use logos_core::prelude::*;\n").unwrap();

    // Phase 49: Collect LWWRegister fields for special SetField handling
    let lww_fields = collect_lww_fields(registry, interner);

    // Phase 54: Collect async functions for Launch codegen
    let async_functions = collect_async_functions(stmts);

    // Phase 54: Collect pipe declarations (variables with _tx/_rx suffixes)
    let main_pipe_vars = collect_pipe_vars(stmts);

    // Collect user-defined structs from registry (Phase 34: generics, Phase 47: is_portable, Phase 49: is_shared)
    let structs: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Struct { fields, generics, is_portable, is_shared } = def {
                if !fields.is_empty() || !generics.is_empty() {
                    Some((*name, fields.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Phase 33/34: Collect user-defined enums from registry (generics, Phase 47: is_portable, Phase 49: is_shared)
    let enums: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Enum { variants, generics, is_portable, is_shared } = def {
                if !variants.is_empty() || !generics.is_empty() {
                    Some((*name, variants.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Emit struct and enum definitions in user_types module if any exist
    if !structs.is_empty() || !enums.is_empty() {
        writeln!(output, "pub mod user_types {{").unwrap();
        writeln!(output, "    use super::*;\n").unwrap();

        for (name, fields, generics, is_portable, is_shared) in &structs {
            output.push_str(&codegen_struct_def(*name, fields, generics, *is_portable, *is_shared, interner, 4));
        }

        for (name, variants, generics, is_portable, is_shared) in &enums {
            output.push_str(&codegen_enum_def(*name, variants, generics, *is_portable, *is_shared, interner, 4));
        }

        writeln!(output, "}}\n").unwrap();
        writeln!(output, "use user_types::*;\n").unwrap();
    }

    // Phase 50: Generate policy impl blocks with predicate and capability methods
    output.push_str(&codegen_policy_impls(policies, interner));

    // Phase 32/38: Emit function definitions before main
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native } = stmt {
            output.push_str(&codegen_function_def(*name, params, body, return_type.as_ref().copied(), *is_native, interner, &lww_fields, &async_functions));
        }
    }

    // Grand Challenge: Collect variables that need to be mutable
    let main_stmts: Vec<&Stmt> = stmts.iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .collect();
    let mut main_mutable_vars = HashSet::new();
    for stmt in &main_stmts {
        collect_mutable_vars_stmt(stmt, &mut main_mutable_vars);
    }

    // Main function
    // Phase 51: Use async main when async operations are present
    if requires_async(stmts) {
        writeln!(output, "#[tokio::main]").unwrap();
        writeln!(output, "async fn main() {{").unwrap();
    } else {
        writeln!(output, "fn main() {{").unwrap();
    }
    // Phase 53: Inject VFS when file operations or persistence is used
    if requires_vfs(stmts) {
        writeln!(output, "    let vfs = logos_core::fs::NativeVfs::new(\".\");").unwrap();
    }
    let mut main_ctx = RefinementContext::new();
    let mut main_synced_vars = HashSet::new();  // Phase 52: Track synced variables in main
    // Phase 56: Pre-scan for Mount+Sync combinations
    let main_var_caps = analyze_variable_capabilities(stmts, interner);
    for stmt in stmts {
        // Skip function definitions - they're already emitted above
        if matches!(stmt, Stmt::FunctionDef { .. }) {
            continue;
        }
        output.push_str(&codegen_stmt(stmt, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars));
    }
    writeln!(output, "}}").unwrap();
    output
}

/// Phase 32/38: Generate a function definition.
/// Phase 38: Updated for native functions and TypeExpr types.
/// Phase 49: Accepts lww_fields for LWWRegister SetField handling.
fn codegen_function_def(
    name: Symbol,
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    return_type: Option<&TypeExpr>,
    is_native: bool,
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    async_functions: &HashSet<Symbol>,  // Phase 54
) -> String {
    let mut output = String::new();
    let func_name = interner.resolve(name);

    // Phase 54: Detect which parameters are used as pipe senders
    let pipe_sender_params = collect_pipe_sender_params(body);

    // Build parameter list using TypeExpr
    let params_str: Vec<String> = params.iter()
        .map(|(param_name, param_type)| {
            let name = interner.resolve(*param_name);
            let ty = codegen_type_expr(param_type, interner);
            // Phase 54: If param is used as a pipe sender, wrap type in Sender<T>
            if pipe_sender_params.contains(param_name) {
                format!("{}: tokio::sync::mpsc::Sender<{}>", name, ty)
            } else {
                format!("{}: {}", name, ty)
            }
        })
        .collect();

    // Get return type string from TypeExpr or infer from body
    let return_type_str = return_type
        .map(|t| codegen_type_expr(t, interner))
        .or_else(|| infer_return_type_from_body(body, interner));

    // Phase 51: Check if function body requires async
    let is_async = body.iter().any(|s| requires_async_stmt(s));
    let fn_keyword = if is_async { "async fn" } else { "fn" };

    // Build function signature
    let signature = if let Some(ref ret_ty) = return_type_str {
        if ret_ty != "()" {
            format!("{} {}({}) -> {}", fn_keyword, func_name, params_str.join(", "), ret_ty)
        } else {
            format!("{} {}({})", fn_keyword, func_name, params_str.join(", "))
        }
    } else {
        format!("{} {}({})", fn_keyword, func_name, params_str.join(", "))
    };

    // Phase 38: Handle native functions
    if is_native {
        let (module, core_fn) = map_native_function(func_name);
        writeln!(output, "{} {{", signature).unwrap();

        // Generate call to logos_core
        let arg_names: Vec<&str> = params.iter()
            .map(|(n, _)| interner.resolve(*n))
            .collect();

        writeln!(output, "    logos_core::{}::{}({})", module, core_fn, arg_names.join(", ")).unwrap();
        writeln!(output, "}}\n").unwrap();
    } else {
        // Non-native: emit body
        // Grand Challenge: Collect mutable vars for this function
        let func_mutable_vars = collect_mutable_vars(body);
        writeln!(output, "{} {{", signature).unwrap();
        let mut func_ctx = RefinementContext::new();
        let mut func_synced_vars = HashSet::new();  // Phase 52: Track synced variables in function
        // Phase 56: Pre-scan for Mount+Sync combinations in function body
        let func_var_caps = analyze_variable_capabilities(body, interner);

        // Phase 50: Register parameter types for capability Check resolution
        for (param_name, param_type) in params {
            let type_name = codegen_type_expr(param_type, interner);
            func_ctx.register_variable_type(*param_name, type_name);
        }

        // Phase 54: Functions receive pipe senders as parameters, no local pipe declarations
        let func_pipe_vars = HashSet::new();

        for stmt in body {
            output.push_str(&codegen_stmt(stmt, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars));
        }
        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Phase 38: Map native function names to logos_core module paths.
fn map_native_function(name: &str) -> (&'static str, &'static str) {
    match name {
        "read" => ("file", "read"),
        "write" => ("file", "write"),
        "now" => ("time", "now"),
        "sleep" => ("time", "sleep"),
        "randomInt" => ("random", "randomInt"),
        "randomFloat" => ("random", "randomFloat"),
        "get" => ("env", "get"),
        "args" => ("env", "args"),
        _ => panic!("Unknown native function: {}. Add mapping to map_native_function().", name),
    }
}

/// Phase 38: Convert TypeExpr to Rust type string.
fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => {
            map_type_to_rust(interner.resolve(*sym))
        }
        TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            // Check for common mappings
            map_type_to_rust(name)
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let params_str: Vec<String> = params.iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();

            match base_name {
                "Result" => {
                    if params_str.len() == 2 {
                        format!("Result<{}, {}>", params_str[0], params_str[1])
                    } else if params_str.len() == 1 {
                        format!("Result<{}, String>", params_str[0])
                    } else {
                        "Result<(), String>".to_string()
                    }
                }
                "Option" => {
                    if !params_str.is_empty() {
                        format!("Option<{}>", params_str[0])
                    } else {
                        "Option<()>".to_string()
                    }
                }
                "Seq" | "List" | "Vec" => {
                    if !params_str.is_empty() {
                        format!("Vec<{}>", params_str[0])
                    } else {
                        "Vec<()>".to_string()
                    }
                }
                "Map" | "HashMap" => {
                    if params_str.len() >= 2 {
                        format!("std::collections::HashMap<{}, {}>", params_str[0], params_str[1])
                    } else {
                        "std::collections::HashMap<String, String>".to_string()
                    }
                }
                "Set" | "HashSet" => {
                    if !params_str.is_empty() {
                        format!("std::collections::HashSet<{}>", params_str[0])
                    } else {
                        "std::collections::HashSet<()>".to_string()
                    }
                }
                other => {
                    if params_str.is_empty() {
                        other.to_string()
                    } else {
                        format!("{}<{}>", other, params_str.join(", "))
                    }
                }
            }
        }
        TypeExpr::Function { inputs, output } => {
            let inputs_str: Vec<String> = inputs.iter()
                .map(|i| codegen_type_expr(i, interner))
                .collect();
            let output_str = codegen_type_expr(output, interner);
            format!("fn({}) -> {}", inputs_str.join(", "), output_str)
        }
        // Phase 43C: Refinement types use the base type for Rust type annotation
        // The constraint predicate is handled separately via debug_assert!
        TypeExpr::Refinement { base, .. } => {
            codegen_type_expr(base, interner)
        }
        // Phase 53: Persistent storage wrapper
        TypeExpr::Persistent { inner } => {
            let inner_type = codegen_type_expr(inner, interner);
            format!("logos_core::storage::Persistent<{}>", inner_type)
        }
    }
}

/// Infer return type from function body by looking at Return statements.
fn infer_return_type_from_body(body: &[Stmt], _interner: &Interner) -> Option<String> {
    for stmt in body {
        if let Stmt::Return { value: Some(_) } = stmt {
            // For now, assume i64 for any expression return
            // TODO: Implement proper type inference
            return Some("i64".to_string());
        }
    }
    None
}

/// Map LOGOS type names to Rust types.
fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" => "i64".to_string(),
        "Nat" => "u64".to_string(),
        "Text" => "String".to_string(),
        "Bool" | "Boolean" => "bool".to_string(),
        "Real" => "f64".to_string(),
        "Char" => "char".to_string(),
        "Byte" => "u8".to_string(),
        "Unit" | "()" => "()".to_string(),
        other => other.to_string(),
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now supports is_shared for CRDT Merge impl.
fn codegen_struct_def(name: Symbol, fields: &[FieldDef], generics: &[Symbol], is_portable: bool, is_shared: bool, interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    // Phase 47: Add Serialize, Deserialize derives if portable
    // Phase 50: Add PartialEq for policy equality comparisons
    // Phase 52: Shared types also need Serialize/Deserialize for Synced<T>
    if is_portable || is_shared {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub struct {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for field in fields {
        let vis = if field.is_public { "pub " } else { "" };
        let rust_type = codegen_field_type(&field.ty, interner);
        writeln!(output, "{}    {}{}: {},", ind, vis, interner.resolve(field.name), rust_type).unwrap();
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Phase 49: Generate Merge impl for Shared structs
    if is_shared {
        output.push_str(&codegen_merge_impl(name, fields, generics, interner, indent));
    }

    output
}

/// Phase 49: Generate impl Merge for a Shared struct.
fn codegen_merge_impl(name: Symbol, fields: &[FieldDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let name_str = interner.resolve(name);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    writeln!(output, "{}impl{} logos_core::crdt::Merge for {}{} {{", ind, generic_str, name_str, generic_str).unwrap();
    writeln!(output, "{}    fn merge(&mut self, other: &Self) {{", ind).unwrap();

    for field in fields {
        let field_name = interner.resolve(field.name);
        // Only merge fields that implement Merge (CRDT types)
        if is_crdt_field_type(&field.ty, interner) {
            writeln!(output, "{}        self.{}.merge(&other.{});", ind, field_name, field_name).unwrap();
        }
    }

    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();

    output
}

/// Phase 49: Check if a field type is a CRDT type that implements Merge.
fn is_crdt_field_type(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "ConvergentCount" | "GCounter")
        }
        FieldType::Generic { base, .. } => {
            let name = interner.resolve(*base);
            matches!(name, "LastWriteWins" | "LWWRegister")
        }
        _ => false,
    }
}

/// Phase 33/34: Generate enum definition with optional generic parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now accepts is_shared parameter (enums don't generate Merge impl yet).
fn codegen_enum_def(name: Symbol, variants: &[VariantDef], generics: &[Symbol], is_portable: bool, _is_shared: bool, interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    // Phase 47: Add Serialize, Deserialize derives if portable
    if is_portable {
        writeln!(output, "{}#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Debug, Clone)]", ind).unwrap();
    }
    writeln!(output, "{}pub enum {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            // Unit variant
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
        } else {
            // Struct variant with named fields
            let fields_str: Vec<String> = variant.fields.iter()
                .map(|f| {
                    let rust_type = codegen_field_type(&f.ty, interner);
                    format!("{}: {}", interner.resolve(f.name), rust_type)
                })
                .collect();
            writeln!(output, "{}    {} {{ {} }},", ind, variant_name, fields_str.join(", ")).unwrap();
        }
    }

    writeln!(output, "{}}}\n", ind).unwrap();
    output
}

/// Convert FieldType to Rust type string.
fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        FieldType::Primitive(sym) => {
            match interner.resolve(*sym) {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),
                "Text" => "String".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Real" => "f64".to_string(),
                "Char" => "char".to_string(),
                "Byte" => "u8".to_string(),
                "Unit" => "()".to_string(),
                other => other.to_string(),
            }
        }
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                // Phase 49: CRDT type mapping
                "ConvergentCount" => "logos_core::crdt::GCounter".to_string(),
                _ => name.to_string(),
            }
        }
        FieldType::Generic { base, params } => {
            let base_str = match interner.resolve(*base) {
                "List" | "Seq" => "Vec",
                "Set" => "std::collections::HashSet",
                "Map" => "std::collections::HashMap",
                "Option" => "Option",
                "Result" => "Result",
                // Phase 49: CRDT generic type
                "LastWriteWins" => "logos_core::crdt::LWWRegister",
                other => other,
            };
            let param_strs: Vec<String> = params.iter()
                .map(|p| codegen_field_type(p, interner))
                .collect();
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        // Phase 34: Type parameter reference (T, U, etc.)
        FieldType::TypeParam(sym) => interner.resolve(*sym).to_string(),
    }
}

pub fn codegen_stmt<'a>(
    stmt: &Stmt<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,  // Phase 52: Track synced variables
    var_caps: &HashMap<Symbol, VariableCapabilities>,  // Phase 56: Mount+Sync detection
    async_functions: &HashSet<Symbol>,  // Phase 54: Functions that are async
    pipe_vars: &HashSet<Symbol>,  // Phase 54: Pipe declarations (have _tx/_rx suffixes)
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();

    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let var_name = interner.resolve(*var);
            let value_str = codegen_expr(value, interner, synced_vars);
            let type_annotation = ty.map(|t| codegen_type_expr(t, interner));

            // Grand Challenge: Variable is mutable if explicitly marked OR if it's a Set target
            let is_mutable = *mutable || mutable_vars.contains(var);

            match (is_mutable, type_annotation) {
                (true, Some(t)) => writeln!(output, "{}let mut {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (true, None) => writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap(),
                (false, Some(t)) => writeln!(output, "{}let {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (false, None) => writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap(),
            }

            // Phase 43C: Handle refinement type
            if let Some(TypeExpr::Refinement { base: _, var: bound_var, predicate }) = ty {
                emit_refinement_check(var_name, *bound_var, predicate, interner, &indent_str, &mut output);
                ctx.register(*var, *bound_var, predicate);
            }
        }

        Stmt::Set { target, value } => {
            let target_name = interner.resolve(*target);
            let value_str = codegen_expr(value, interner, synced_vars);
            writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();

            // Phase 43C: Check if this variable has a refinement constraint
            if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
                emit_refinement_check(target_name, bound_var, predicate, interner, &indent_str, &mut output);
            }
        }

        Stmt::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner, synced_vars)).collect();
            writeln!(output, "{}{}({});", indent_str, func_name, args_str.join(", ")).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, interner, synced_vars);
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body, decreasing: _ } => {
            // decreasing is compile-time only, ignored at runtime
            let cond_str = codegen_expr(cond, interner, synced_vars);
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Repeat { var, iterable, body } => {
            let var_name = interner.resolve(*var);
            let iter_str = codegen_expr(iterable, interner, synced_vars);
            writeln!(output, "{}for {} in {} {{", indent_str, var_name, iter_str).unwrap();
            ctx.push_scope();
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Return { value } => {
            if let Some(v) = value {
                let value_str = codegen_expr(v, interner, synced_vars);
                writeln!(output, "{}return {};", indent_str, value_str).unwrap();
            } else {
                writeln!(output, "{}return;", indent_str).unwrap();
            }
        }

        Stmt::Assert { proposition } => {
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        // Phase 35: Trust with documented justification
        Stmt::Trust { proposition, justification } => {
            let reason = interner.resolve(*justification);
            // Strip quotes if present (string literals include their quotes)
            let reason_clean = reason.trim_matches('"');
            writeln!(output, "{}// TRUST: {}", indent_str, reason_clean).unwrap();
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        Stmt::RuntimeAssert { condition } => {
            let cond_str = codegen_expr(condition, interner, synced_vars);
            writeln!(output, "{}debug_assert!({});", indent_str, cond_str).unwrap();
        }

        // Phase 50: Security Check - mandatory runtime guard (NEVER optimized out)
        Stmt::Check { subject, predicate, is_capability, object, source_text, span } => {
            let subj_name = interner.resolve(*subject);
            let pred_name = interner.resolve(*predicate).to_lowercase();

            let call = if *is_capability {
                let obj_sym = object.expect("capability must have object");
                let obj_word = interner.resolve(obj_sym);

                // Phase 50: Type-based resolution
                // "Check that user can publish the document" -> find variable of type Document
                // First try to find a variable whose type matches the object word
                let obj_name = ctx.find_variable_by_type(obj_word, interner)
                    .unwrap_or_else(|| obj_word.to_string());

                format!("{}.can_{}(&{})", subj_name, pred_name, obj_name)
            } else {
                format!("{}.is_{}()", subj_name, pred_name)
            };

            writeln!(output, "{}if !({}) {{", indent_str, call).unwrap();
            writeln!(output, "{}    logos_core::panic_with(\"Security Check Failed at line {}: {}\");",
                     indent_str, span.start, source_text).unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 51: P2P Networking - Listen on network address
        Stmt::Listen { address } => {
            let addr_str = codegen_expr(address, interner, synced_vars);
            // Pass &str instead of String
            writeln!(output, "{}logos_core::network::listen(&{}).await.expect(\"Failed to listen\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Connect to remote peer
        Stmt::ConnectTo { address } => {
            let addr_str = codegen_expr(address, interner, synced_vars);
            // Pass &str instead of String
            writeln!(output, "{}logos_core::network::connect(&{}).await.expect(\"Failed to connect\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Create PeerAgent remote handle
        Stmt::LetPeerAgent { var, address } => {
            let var_name = interner.resolve(*var);
            let addr_str = codegen_expr(address, interner, synced_vars);
            // Pass &str instead of String
            writeln!(output, "{}let {} = logos_core::network::PeerAgent::new(&{}).expect(\"Invalid address\");",
                     indent_str, var_name, addr_str).unwrap();
        }

        // Phase 51: Sleep for milliseconds
        Stmt::Sleep { milliseconds } => {
            let ms_str = codegen_expr(milliseconds, interner, synced_vars);
            // Use tokio async sleep
            writeln!(output, "{}tokio::time::sleep(std::time::Duration::from_millis({} as u64)).await;",
                     indent_str, ms_str).unwrap();
        }

        // Phase 52/56: Sync CRDT variable on topic
        Stmt::Sync { var, topic } => {
            let var_name = interner.resolve(*var);
            let topic_str = codegen_expr(topic, interner, synced_vars);

            // Phase 56: Check if this variable is also mounted
            if let Some(caps) = var_caps.get(var) {
                if caps.mounted {
                    // Both Mount and Sync: use Distributed<T>
                    // Mount statement will handle the Distributed::mount call
                    // Here we just track it as synced
                    synced_vars.insert(*var);
                    return output;  // Skip - Mount will emit Distributed<T>
                }
            }

            // Sync-only: use Synced<T>
            writeln!(
                output,
                "{}let {} = logos_core::crdt::Synced::new({}, &{}).await;",
                indent_str, var_name, var_name, topic_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // Phase 53/56: Mount persistent CRDT from journal
        Stmt::Mount { var, path } => {
            let var_name = interner.resolve(*var);
            let path_str = codegen_expr(path, interner, synced_vars);

            // Phase 56: Check if this variable is also synced
            if let Some(caps) = var_caps.get(var) {
                if caps.synced {
                    // Both Mount and Sync: use Distributed<T>
                    let topic_str = caps.sync_topic.as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("\"default\"");
                    writeln!(
                        output,
                        "{}let {} = logos_core::distributed::Distributed::mount(std::sync::Arc::new(vfs.clone()), &{}, Some({}.to_string())).await.expect(\"Failed to mount\");",
                        indent_str, var_name, path_str, topic_str
                    ).unwrap();
                    synced_vars.insert(*var);
                    return output;
                }
            }

            // Mount-only: use Persistent<T>
            writeln!(
                output,
                "{}let {} = logos_core::storage::Persistent::mount(&vfs, &{}).await.expect(\"Failed to mount\");",
                indent_str, var_name, path_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // =====================================================================
        // Phase 54: Go-like Concurrency Codegen
        // =====================================================================

        Stmt::LaunchTask { function, args } => {
            let fn_name = interner.resolve(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr(a, interner, synced_vars)
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}tokio::spawn(async move {{ {}({}){await_suffix}; }});",
                indent_str, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::LaunchTaskWithHandle { handle, function, args } => {
            let handle_name = interner.resolve(*handle);
            let fn_name = interner.resolve(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr(a, interner, synced_vars)
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ {}({}){await_suffix} }});",
                indent_str, handle_name, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::CreatePipe { var, element_type, capacity } => {
            let var_name = interner.resolve(*var);
            let type_name = interner.resolve(*element_type);
            let cap = capacity.unwrap_or(32);
            // Map LOGOS types to Rust types
            let rust_type = match type_name {
                "Int" => "i64",
                "Nat" => "u64",
                "Text" => "String",
                "Bool" => "bool",
                _ => type_name,
            };
            writeln!(
                output,
                "{}let ({}_tx, mut {}_rx) = tokio::sync::mpsc::channel::<{}>({});",
                indent_str, var_name, var_name, rust_type, cap
            ).unwrap();
        }

        Stmt::SendPipe { value, pipe } => {
            let val_str = codegen_expr(value, interner, synced_vars);
            let pipe_str = codegen_expr(pipe, interner, synced_vars);
            // Phase 54: Check if pipe is a local declaration (has _tx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}{}_tx.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            }
        }

        Stmt::ReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr(pipe, interner, synced_vars);
            // Phase 54: Check if pipe is a local declaration (has _rx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}let {} = {}_rx.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let {} = {}.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            }
        }

        Stmt::TrySendPipe { value, pipe, result } => {
            let val_str = codegen_expr(value, interner, synced_vars);
            let pipe_str = codegen_expr(pipe, interner, synced_vars);
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_tx" } else { "" };
            if let Some(res) = result {
                let res_name = interner.resolve(*res);
                writeln!(
                    output,
                    "{}let {} = {}{}.try_send({}).is_ok();",
                    indent_str, res_name, pipe_str, suffix, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let _ = {}{}.try_send({});",
                    indent_str, pipe_str, suffix, val_str
                ).unwrap();
            }
        }

        Stmt::TryReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr(pipe, interner, synced_vars);
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_rx" } else { "" };
            writeln!(
                output,
                "{}let {} = {}{}.try_recv().ok();",
                indent_str, var_name, pipe_str, suffix
            ).unwrap();
        }

        Stmt::StopTask { handle } => {
            let handle_str = codegen_expr(handle, interner, synced_vars);
            writeln!(output, "{}{}.abort();", indent_str, handle_str).unwrap();
        }

        Stmt::Select { branches } => {
            use crate::ast::stmt::SelectBranch;

            writeln!(output, "{}tokio::select! {{", indent_str).unwrap();
            for branch in branches {
                match branch {
                    SelectBranch::Receive { var, pipe, body } => {
                        let var_name = interner.resolve(*var);
                        let pipe_str = codegen_expr(pipe, interner, synced_vars);
                        writeln!(
                            output,
                            "{}    {} = {}_rx.recv() => {{",
                            indent_str, var_name, pipe_str
                        ).unwrap();
                        writeln!(
                            output,
                            "{}        if let Some({}) = {} {{",
                            indent_str, var_name, var_name
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 3, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}        }}", indent_str).unwrap();
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        let ms_str = codegen_expr(milliseconds, interner, synced_vars);
                        // Convert seconds to milliseconds if the value looks like seconds
                        writeln!(
                            output,
                            "{}    _ = tokio::time::sleep(std::time::Duration::from_secs({} as u64)) => {{",
                            indent_str, ms_str
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                }
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Give { object, recipient } => {
            // Move semantics: pass ownership without borrowing
            let obj_str = codegen_expr(object, interner, synced_vars);
            let recv_str = codegen_expr(recipient, interner, synced_vars);
            writeln!(output, "{}{}({});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::Show { object, recipient } => {
            // Borrow semantics: pass immutable reference
            let obj_str = codegen_expr(object, interner, synced_vars);
            let recv_str = codegen_expr(recipient, interner, synced_vars);
            writeln!(output, "{}{}(&{});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::SetField { object, field, value } => {
            let obj_str = codegen_expr(object, interner, synced_vars);
            let field_name = interner.resolve(*field);
            let value_str = codegen_expr(value, interner, synced_vars);

            // Phase 49: Check if this field is an LWWRegister - use .set() instead of =
            // We check if ANY type has this field as LWW (heuristic - works for most cases)
            let is_lww = lww_fields.iter().any(|(_, f)| f == field_name);
            if is_lww {
                writeln!(output, "{}{}.{}.set({});", indent_str, obj_str, field_name, value_str).unwrap();
            } else {
                writeln!(output, "{}{}.{} = {};", indent_str, obj_str, field_name, value_str).unwrap();
            }
        }

        Stmt::StructDef { .. } => {
            // Struct definitions are handled in codegen_program, not here
        }

        Stmt::FunctionDef { .. } => {
            // Function definitions are handled in codegen_program, not here
        }

        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr(target, interner, synced_vars);
            writeln!(output, "{}match {} {{", indent_str, target_str).unwrap();

            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    // Get the enum name from the arm, or fallback to just variant name
                    let enum_prefix = arm.enum_name
                        .map(|e| format!("{}::", interner.resolve(e)))
                        .unwrap_or_default();

                    if arm.bindings.is_empty() {
                        // Unit variant pattern
                        writeln!(output, "{}    {}{} => {{", indent_str, enum_prefix, variant_name).unwrap();
                    } else {
                        // Pattern with bindings
                        let bindings_str: Vec<String> = arm.bindings.iter()
                            .map(|(field, binding)| {
                                let field_name = interner.resolve(*field);
                                let binding_name = interner.resolve(*binding);
                                if field_name == binding_name {
                                    field_name.to_string()
                                } else {
                                    format!("{}: {}", field_name, binding_name)
                                }
                            })
                            .collect();
                        writeln!(output, "{}    {}{} {{ {} }} => {{", indent_str, enum_prefix, variant_name, bindings_str.join(", ")).unwrap();
                    }
                } else {
                    // Otherwise (wildcard) pattern
                    writeln!(output, "{}    _ => {{", indent_str).unwrap();
                }

                ctx.push_scope();
                for stmt in arm.body {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
                }
                ctx.pop_scope();
                writeln!(output, "{}    }}", indent_str).unwrap();
            }

            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Push { value, collection } => {
            let val_str = codegen_expr(value, interner, synced_vars);
            let coll_str = codegen_expr(collection, interner, synced_vars);
            writeln!(output, "{}{}.push({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Pop { collection, into } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            match into {
                Some(var) => {
                    let var_name = interner.resolve(*var);
                    // Unwrap the Option returned by pop() - panics if empty
                    writeln!(output, "{}let {} = {}.pop().expect(\"Pop from empty collection\");", indent_str, var_name, coll_str).unwrap();
                }
                None => {
                    writeln!(output, "{}{}.pop();", indent_str, coll_str).unwrap();
                }
            }
        }

        Stmt::Add { value, collection } => {
            let val_str = codegen_expr(value, interner, synced_vars);
            let coll_str = codegen_expr(collection, interner, synced_vars);
            writeln!(output, "{}{}.insert({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr(value, interner, synced_vars);
            let coll_str = codegen_expr(collection, interner, synced_vars);
            writeln!(output, "{}{}.remove(&{});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::SetIndex { collection, index, value } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            let index_str = codegen_expr(index, interner, synced_vars);
            let value_str = codegen_expr(value, interner, synced_vars);
            // Phase 57: Polymorphic indexing via trait
            writeln!(output, "{}LogosIndexMut::logos_set(&mut {}, {}, {});", indent_str, coll_str, index_str, value_str).unwrap();
        }

        // Phase 8.5: Zone (memory arena) block
        Stmt::Zone { name, capacity, source_file, body } => {
            let zone_name = interner.resolve(*name);

            // Generate zone creation based on type
            if let Some(path_sym) = source_file {
                // Memory-mapped file zone
                let path = interner.resolve(*path_sym);
                writeln!(
                    output,
                    "{}let {} = logos_core::memory::Zone::new_mapped(\"{}\").expect(\"Failed to map file\");",
                    indent_str, zone_name, path
                ).unwrap();
            } else {
                // Heap arena zone
                let cap = capacity.unwrap_or(4096); // Default 4KB
                writeln!(
                    output,
                    "{}let {} = logos_core::memory::Zone::new_heap({});",
                    indent_str, zone_name, cap
                ).unwrap();
            }

            // Open block scope
            writeln!(output, "{}{{", indent_str).unwrap();
            ctx.push_scope();

            // Generate body statements
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars));
            }

            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 9: Concurrent execution block (async, I/O-bound)
        // Generates tokio::join! for concurrent task execution
        // Phase 51: Variables used across multiple tasks are cloned to avoid move issues
        Stmt::Concurrent { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            // Collect variables used in Call statements across all tasks
            let used_vars: HashSet<String> = tasks.iter().flat_map(|s| {
                if let Stmt::Call { args, .. } = s {
                    args.iter().filter_map(|arg| {
                        if let Expr::Identifier(sym) = arg {
                            Some(interner.resolve(*sym).to_string())
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>()
                } else {
                    vec![]
                }
            }).collect();

            if !let_bindings.is_empty() {
                // Generate tuple destructuring for concurrent Let bindings
                writeln!(output, "{}let ({}) = tokio::join!(", indent_str, let_bindings.join(", ")).unwrap();
            } else {
                writeln!(output, "{}tokio::join!(", indent_str).unwrap();
            }

            for (i, stmt) in tasks.iter().enumerate() {
                let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars);

                // Convert call statements to awaited calls for async context
                let inner_awaited = if let Stmt::Call { .. } = stmt {
                    // Add .await before the semicolon for function calls
                    inner.trim().trim_end_matches(';').to_string() + ".await;"
                } else {
                    inner.trim().to_string()
                };

                // For tasks that use shared variables, wrap in a block that clones them
                if !used_vars.is_empty() && i < tasks.len() - 1 {
                    // Clone variables for all tasks except the last one
                    let clones: Vec<String> = used_vars.iter()
                        .map(|v| format!("let {} = {}.clone();", v, v))
                        .collect();
                    write!(output, "{}    {{ {} async move {{ {} }} }}",
                           indent_str, clones.join(" "), inner_awaited).unwrap();
                } else {
                    // Last task can use original variables
                    write!(output, "{}    async {{ {} }}", indent_str, inner_awaited).unwrap();
                }

                if i < tasks.len() - 1 {
                    writeln!(output, ",").unwrap();
                } else {
                    writeln!(output).unwrap();
                }
            }

            writeln!(output, "{});", indent_str).unwrap();
        }

        // Phase 9: Parallel execution block (CPU-bound)
        // Generates rayon::join for two tasks, or thread::spawn for 3+ tasks
        Stmt::Parallel { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            if tasks.len() == 2 {
                // Use rayon::join for exactly 2 tasks
                if !let_bindings.is_empty() {
                    writeln!(output, "{}let ({}) = rayon::join(", indent_str, let_bindings.join(", ")).unwrap();
                } else {
                    writeln!(output, "{}rayon::join(", indent_str).unwrap();
                }

                for (i, stmt) in tasks.iter().enumerate() {
                    let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars);
                    write!(output, "{}    || {{ {} }}", indent_str, inner.trim()).unwrap();
                    if i == 0 {
                        writeln!(output, ",").unwrap();
                    } else {
                        writeln!(output).unwrap();
                    }
                }
                writeln!(output, "{});", indent_str).unwrap();
            } else {
                // For 3+ tasks, use thread::spawn pattern
                writeln!(output, "{}{{", indent_str).unwrap();
                writeln!(output, "{}    let handles: Vec<_> = vec![", indent_str).unwrap();
                for stmt in *tasks {
                    let inner = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, synced_vars, var_caps, async_functions, pipe_vars);
                    writeln!(output, "{}        std::thread::spawn(move || {{ {} }}),",
                             indent_str, inner.trim()).unwrap();
                }
                writeln!(output, "{}    ];", indent_str).unwrap();
                writeln!(output, "{}    for h in handles {{ h.join().unwrap(); }}", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            }
        }

        // Phase 10: Read from console or file
        // Phase 53: File reads now use async VFS
        Stmt::ReadFrom { var, source } => {
            let var_name = interner.resolve(*var);
            match source {
                ReadSource::Console => {
                    writeln!(output, "{}let {} = logos_core::io::read_line();", indent_str, var_name).unwrap();
                }
                ReadSource::File(path_expr) => {
                    let path_str = codegen_expr(path_expr, interner, synced_vars);
                    // Phase 53: Use VFS with async
                    writeln!(
                        output,
                        "{}let {} = vfs.read_to_string(&{}).await.expect(\"Failed to read file\");",
                        indent_str, var_name, path_str
                    ).unwrap();
                }
            }
        }

        // Phase 10: Write to file
        // Phase 53: File writes now use async VFS
        Stmt::WriteFile { content, path } => {
            let content_str = codegen_expr(content, interner, synced_vars);
            let path_str = codegen_expr(path, interner, synced_vars);
            // Phase 53: Use VFS with async
            writeln!(
                output,
                "{}vfs.write(&{}, {}.as_bytes()).await.expect(\"Failed to write file\");",
                indent_str, path_str, content_str
            ).unwrap();
        }

        // Phase 46: Spawn an agent
        Stmt::Spawn { agent_type, name } => {
            let type_name = interner.resolve(*agent_type);
            let agent_name = interner.resolve(*name);
            // Generate agent spawn with tokio channel
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ /* {} agent loop */ }});",
                indent_str, agent_name, type_name
            ).unwrap();
        }

        // Phase 46: Send message to agent
        Stmt::SendMessage { message, destination } => {
            let msg_str = codegen_expr(message, interner, synced_vars);
            let dest_str = codegen_expr(destination, interner, synced_vars);
            writeln!(
                output,
                "{}{}.send({}).await.expect(\"Failed to send message\");",
                indent_str, dest_str, msg_str
            ).unwrap();
        }

        // Phase 46: Await response from agent
        Stmt::AwaitMessage { source, into } => {
            let src_str = codegen_expr(source, interner, synced_vars);
            let var_name = interner.resolve(*into);
            writeln!(
                output,
                "{}let {} = {}.recv().await.expect(\"Failed to receive message\");",
                indent_str, var_name, src_str
            ).unwrap();
        }

        // Phase 49: Merge CRDT state
        Stmt::MergeCrdt { source, target } => {
            let src_str = codegen_expr(source, interner, synced_vars);
            let tgt_str = codegen_expr(target, interner, synced_vars);
            writeln!(
                output,
                "{}{}.merge(&{});",
                indent_str, tgt_str, src_str
            ).unwrap();
        }

        // Phase 49: Increment GCounter
        // Phase 52: If object is synced, wrap in .mutate() for auto-publish
        Stmt::IncreaseCrdt { object, field, amount } => {
            let field_name = interner.resolve(*field);
            let amount_str = codegen_expr(amount, interner, synced_vars);

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.increment({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    ).unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr(object, interner, synced_vars);
            writeln!(
                output,
                "{}{}.{}.increment({} as u64);",
                indent_str, obj_str, field_name, amount_str
            ).unwrap();
        }
    }

    output
}

/// Phase 52: Extract the root identifier from an expression.
/// For `x.field.subfield`, returns `x`.
fn get_root_identifier(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier(object),
        _ => None,
    }
}

pub fn codegen_expr(expr: &Expr, interner: &Interner, synced_vars: &HashSet<Symbol>) -> String {
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),

        Expr::BinaryOp { op, left, right } => {
            let left_str = codegen_expr(left, interner, synced_vars);
            let right_str = codegen_expr(right, interner, synced_vars);
            // Phase 53: String concatenation requires special handling
            if matches!(op, BinaryOpKind::Concat) {
                return format!("format!(\"{{}}{{}}\", {}, {})", left_str, right_str);
            }
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => unreachable!(), // Handled above
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = interner.resolve(*function);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, interner, synced_vars)).collect();
            format!("{}({})", func_name, args_str.join(", "))
        }

        Expr::Index { collection, index } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            let index_str = codegen_expr(index, interner, synced_vars);
            // Phase 57: Polymorphic indexing via trait
            format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            let start_str = codegen_expr(start, interner, synced_vars);
            let end_str = codegen_expr(end, interner, synced_vars);
            // Phase 43D: 1-indexed inclusive to 0-indexed exclusive
            // "items 1 through 3" → &items[0..3] (elements at indices 0, 1, 2)
            format!("&{}[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
        }

        Expr::Copy { expr } => {
            let expr_str = codegen_expr(expr, interner, synced_vars);
            // Phase 43D: Explicit clone to owned Vec
            format!("{}.to_vec()", expr_str)
        }

        Expr::Length { collection } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::Contains { collection, value } => {
            let coll_str = codegen_expr(collection, interner, synced_vars);
            let val_str = codegen_expr(value, interner, synced_vars);
            // Use LogosContains trait for unified contains across List, Set, Map, Text
            format!("{}.logos_contains(&{})", coll_str, val_str)
        }

        Expr::Union { left, right } => {
            let left_str = codegen_expr(left, interner, synced_vars);
            let right_str = codegen_expr(right, interner, synced_vars);
            format!("{}.union(&{}).cloned().collect::<std::collections::HashSet<_>>()", left_str, right_str)
        }

        Expr::Intersection { left, right } => {
            let left_str = codegen_expr(left, interner, synced_vars);
            let right_str = codegen_expr(right, interner, synced_vars);
            format!("{}.intersection(&{}).cloned().collect::<std::collections::HashSet<_>>()", left_str, right_str)
        }

        // Phase 48: Sipping Protocol expressions
        Expr::ManifestOf { zone } => {
            let zone_str = codegen_expr(zone, interner, synced_vars);
            format!("logos_core::network::FileSipper::from_zone(&{}).manifest()", zone_str)
        }

        Expr::ChunkAt { index, zone } => {
            let zone_str = codegen_expr(zone, interner, synced_vars);
            let index_str = codegen_expr(index, interner, synced_vars);
            // LOGOS uses 1-indexed, Rust uses 0-indexed
            format!("logos_core::network::FileSipper::from_zone(&{}).get_chunk(({} - 1) as usize)", zone_str, index_str)
        }

        Expr::List(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| codegen_expr(i, interner, synced_vars))
                .collect();
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Range { start, end } => {
            let start_str = codegen_expr(start, interner, synced_vars);
            let end_str = codegen_expr(end, interner, synced_vars);
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let field_name = interner.resolve(*field);

            // Phase 52: Check if root object is synced - use .get().await
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    let obj_name = interner.resolve(sym);
                    return format!("{}.get().await.{}", obj_name, field_name);
                }
            }

            let obj_str = codegen_expr(object, interner, synced_vars);
            format!("{}.{}", obj_str, field_name)
        }

        Expr::New { type_name, type_args, init_fields } => {
            let type_str = interner.resolve(*type_name);
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20, ..Default::default() }
                // Always add ..Default::default() to handle partial initialization (e.g., CRDT fields)
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = interner.resolve(*name);
                        let value_str = codegen_expr(value, interner, synced_vars);
                        format!("{}: {}", field_name, value_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {}, ..Default::default() }}", type_str, fields_str)
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                let args_str = type_args.iter()
                    .map(|s| map_type_to_rust(interner.resolve(*s)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{}>::default()", type_str, args_str)
            }
        }

        Expr::NewVariant { enum_name, variant, fields } => {
            let enum_str = interner.resolve(*enum_name);
            let variant_str = interner.resolve(*variant);
            if fields.is_empty() {
                // Unit variant: Shape::Point
                format!("{}::{}", enum_str, variant_str)
            } else {
                // Struct variant: Shape::Circle { radius: 10 }
                let fields_str: Vec<String> = fields.iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);
                        let val = codegen_expr(value, interner, synced_vars);
                        format!("{}: {}", name, val)
                    })
                    .collect();
                format!("{}::{} {{ {} }}", enum_str, variant_str, fields_str.join(", "))
            }
        }
    }
}

fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        // String literals are converted to String for consistent Text type handling
        Literal::Text(sym) => format!("String::from(\"{}\")", interner.resolve(*sym)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
        // Character literals
        Literal::Char(c) => {
            // Handle escape sequences for special characters
            match c {
                '\n' => "'\\n'".to_string(),
                '\t' => "'\\t'".to_string(),
                '\r' => "'\\r'".to_string(),
                '\\' => "'\\\\'".to_string(),
                '\'' => "'\\''".to_string(),
                '\0' => "'\\0'".to_string(),
                c => format!("'{}'", c),
            }
        }
    }
}

/// Converts a LogicExpr to a Rust boolean expression for debug_assert!().
/// Uses RustFormatter to unify all logic-to-Rust translation.
pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    let mut registry = SymbolRegistry::new();
    let formatter = RustFormatter;
    let mut buf = String::new();

    match expr.write_logic(&mut buf, &mut registry, interner, &formatter) {
        Ok(_) => buf,
        Err(_) => "/* error generating assertion */ false".to_string(),
    }
}

pub fn codegen_term(term: &Term, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) => interner.resolve(*sym).to_string(),
        Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Value { kind, .. } => match kind {
            NumberKind::Integer(n) => n.to_string(),
            NumberKind::Real(f) => f.to_string(),
            NumberKind::Symbolic(sym) => interner.resolve(*sym).to_string(),
        },
        Term::Function(name, args) => {
            let args_str: Vec<String> = args.iter()
                .map(|a| codegen_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed { possessor, possessed } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> = members.iter()
                .map(|m| codegen_term(m, interner))
                .collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_number() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner, &synced_vars), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner, &synced_vars), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars), "()");
    }
}
