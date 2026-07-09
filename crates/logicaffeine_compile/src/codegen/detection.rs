use std::collections::{HashMap, HashSet};

use crate::analysis::registry::{FieldType, TypeDef, TypeRegistry};
use crate::ast::stmt::{Expr, Literal, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

use super::is_recursive_field;
use super::types::codegen_type_expr;

pub(super) fn is_result_type(ty: &TypeExpr, interner: &Interner) -> bool {
    if let TypeExpr::Generic { base, .. } = ty {
        interner.resolve(*base) == "Result"
    } else {
        false
    }
}

/// Phase 51: Detect if any statements require async execution.
/// Returns true if the program needs #[tokio::main] async fn main().
pub(super) fn requires_async(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_async_stmt(s))
}

pub(super) fn requires_async_stmt(stmt: &Stmt) -> bool {
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
        // Check Inspect arms for async operations
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| requires_async_stmt(s)))
        }
        _ => false,
    }
}

/// Phase 53: Detect if any statements require VFS (Virtual File System).
/// Returns true if the program uses file operations or persistent storage.
pub(super) fn requires_vfs(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_vfs_stmt(s))
}

pub(super) fn requires_vfs_stmt(stmt: &Stmt) -> bool {
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

/// Phase 49b: Extract root identifier from expression for mutability analysis.
/// Works with both simple identifiers and field accesses.
pub(super) fn get_root_identifier_for_mutability(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier_for_mutability(object),
        _ => None,
    }
}

/// Grand Challenge: Collect all variables that need `let mut` in Rust.
/// This includes:
/// - Variables that are targets of `Set` statements (reassignment)
/// - Variables that are targets of `Push` statements (mutation via push)
/// - Variables that are targets of `Pop` statements (mutation via pop)
pub(super) fn collect_mutable_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in stmts {
        collect_mutable_vars_stmt(stmt, &mut targets);
    }
    targets
}

pub(super) fn collect_mutable_vars_stmt(stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            targets.insert(*target);
        }
        Stmt::Push { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Pop { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Add { collection, .. } => {
            // If collection is an identifier (Set) or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Remove { collection, .. } => {
            // If collection is an identifier (Set) or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::SetIndex { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
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
        // Inspect (pattern match) arms may contain mutations
        Stmt::Inspect { arms, .. } => {
            for arm in arms.iter() {
                for s in arm.body.iter() {
                    collect_mutable_vars_stmt(s, targets);
                }
            }
        }
        // Phase 9: Structured Concurrency blocks
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        // Phase 49b: CRDT operations require mutable access
        Stmt::IncreaseCrdt { object, .. } | Stmt::DecreaseCrdt { object, .. } => {
            // Extract root variable from field access (e.g., g.score -> g)
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        Stmt::AppendToSequence { sequence, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(sequence) {
                targets.insert(sym);
            }
        }
        Stmt::ResolveConflict { object, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        // Phase 49b: SetField on MVRegister/LWWRegister uses .set() which requires &mut self
        Stmt::SetField { object, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        _ => {}
    }
}

/// Detect mutable Text variables that are only ever assigned single-character
/// string literals. These can be emitted as `u8` (byte) instead of `String`,
/// eliminating heap allocations for temporary character variables.
///
/// Returns a set of symbols that qualify for the u8 optimization.
///
/// A variable qualifies if:
/// 1. It is declared with `Let mutable ch be "x"` (single-char text literal)
/// 2. Every `Set ch to "y"` assigns a single-char text literal
/// 3. It is never used in a context that requires String (e.g., function call args,
///    Return, field access) — only push_str (→ push), Show, or comparison
pub(super) fn collect_single_char_text_vars(stmts: &[Stmt], interner: &Interner) -> HashSet<Symbol> {
    let mut candidates: HashSet<Symbol> = HashSet::new();
    let mut disqualified: HashSet<Symbol> = HashSet::new();

    // Pass 1: Find candidates (Let mutable with single-char text literal)
    // and check all Set assignments are also single-char.
    scan_single_char_candidates(stmts, interner, &mut candidates, &mut disqualified);

    // Remove disqualified
    for sym in &disqualified {
        candidates.remove(sym);
    }

    // Pass 2: Check usage contexts. If any use requires a String, disqualify.
    let mut usage_disqualified: HashSet<Symbol> = HashSet::new();
    check_single_char_usage(stmts, &candidates, &mut usage_disqualified);

    for sym in &usage_disqualified {
        candidates.remove(sym);
    }

    candidates
}

fn is_single_char_text_literal<'a>(expr: &Expr<'a>, interner: &Interner) -> bool {
    if let Expr::Literal(crate::ast::stmt::Literal::Text(sym)) = expr {
        let text = interner.resolve(*sym);
        text.len() == 1 && text.is_ascii()
    } else {
        false
    }
}

fn scan_single_char_candidates(
    stmts: &[Stmt],
    interner: &Interner,
    candidates: &mut HashSet<Symbol>,
    disqualified: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { var, value, mutable, .. } => {
                if *mutable && is_single_char_text_literal(value, interner) {
                    candidates.insert(*var);
                }
                // Recurse into nested blocks in any case
            }
            Stmt::Set { target, value } => {
                if candidates.contains(target) || !disqualified.contains(target) {
                    if !is_single_char_text_literal(value, interner) {
                        // Non-single-char assignment disqualifies
                        disqualified.insert(*target);
                    }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                scan_single_char_candidates(then_block, interner, candidates, disqualified);
                if let Some(eb) = else_block {
                    scan_single_char_candidates(eb, interner, candidates, disqualified);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                scan_single_char_candidates(body, interner, candidates, disqualified);
            }
            Stmt::FunctionDef { body, .. } => {
                scan_single_char_candidates(body, interner, candidates, disqualified);
            }
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                scan_single_char_candidates(tasks, interner, candidates, disqualified);
            }
            _ => {}
        }
    }
}

/// Check that a candidate variable is only used in contexts compatible with u8:
/// - `Set text to text + ch` (self-append → push)
/// - `Show ch`
/// - Comparison (`ch equals "x"`)
///
/// Disqualify if used in:
/// - Function call arguments
/// - Return value
/// - Assignment to another variable (`Let y be ch`)
/// - Any other expression context that expects String
fn check_single_char_usage(
    stmts: &[Stmt],
    candidates: &HashSet<Symbol>,
    disqualified: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, value } => {
                // `Set ch to "x"` is fine (already validated in pass 1).
                // `Set text to text + ch` is fine (self-append).
                // But if ch appears in a non-append context in value, disqualify.
                if !candidates.contains(target) {
                    // target is not a candidate, check if value uses a candidate
                    // in a non-append-compatible way
                    check_expr_usage(value, candidates, disqualified, true);
                }
            }
            Stmt::Show { object, .. } => {
                // Show ch is fine — we'll emit `println!("{}", ch as char)`
                // But don't check deeper — identifiers in Show are OK
            }
            Stmt::Let { value, .. } => {
                // `Let y be ch` would assign a u8 to y, which may not work.
                // Disqualify if a candidate appears directly as the value.
                check_expr_usage_strict(value, candidates, disqualified);
            }
            Stmt::Return { value: Some(v) } => {
                check_expr_usage_strict(v, candidates, disqualified);
            }
            Stmt::Call { args, .. } => {
                for a in args.iter() {
                    check_expr_usage_strict(a, candidates, disqualified);
                }
            }
            Stmt::Push { value, .. } => {
                // `Push ch to list` — disqualify since list expects String
                check_expr_usage_strict(value, candidates, disqualified);
            }
            Stmt::If { cond, then_block, else_block } => {
                // Comparisons in conditions are fine for u8
                check_single_char_usage(then_block, candidates, disqualified);
                if let Some(eb) = else_block {
                    check_single_char_usage(eb, candidates, disqualified);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                check_single_char_usage(body, candidates, disqualified);
            }
            Stmt::FunctionDef { body, .. } => {
                check_single_char_usage(body, candidates, disqualified);
            }
            _ => {}
        }
    }
}

/// Check an expression for usage of candidates in self-append context.
/// `is_self_append` indicates the expression is the RHS of `Set text to <expr>`
/// where text is a string (not a candidate). In that context, `text + ch` is fine.
fn check_expr_usage(
    expr: &Expr,
    candidates: &HashSet<Symbol>,
    disqualified: &mut HashSet<Symbol>,
    is_self_append: bool,
) {
    match expr {
        Expr::BinaryOp { op: crate::ast::stmt::BinaryOpKind::Add, left, right } if is_self_append => {
            // In self-append: `text + ch` — ch as a direct identifier is fine
            check_expr_usage(left, candidates, disqualified, true);
            // right side: if it's a bare candidate identifier, that's fine (push)
            if !matches!(right, Expr::Identifier(sym) if candidates.contains(sym)) {
                check_expr_usage(right, candidates, disqualified, false);
            }
        }
        Expr::Identifier(sym) if !is_self_append => {
            // Bare candidate in non-append context — disqualify
            if candidates.contains(sym) {
                disqualified.insert(*sym);
            }
        }
        _ => {}
    }
}

/// Strictly disqualify any candidate that appears anywhere in this expression.
fn check_expr_usage_strict(expr: &Expr, candidates: &HashSet<Symbol>, disqualified: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => {
            if candidates.contains(sym) {
                disqualified.insert(*sym);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            check_expr_usage_strict(left, candidates, disqualified);
            check_expr_usage_strict(right, candidates, disqualified);
        }
        Expr::Call { args, .. } => {
            for a in args.iter() {
                check_expr_usage_strict(a, candidates, disqualified);
            }
        }
        Expr::Not { operand } => check_expr_usage_strict(operand, candidates, disqualified),
        Expr::Index { collection, index } => {
            check_expr_usage_strict(collection, candidates, disqualified);
            check_expr_usage_strict(index, candidates, disqualified);
        }
        Expr::Length { collection } => check_expr_usage_strict(collection, candidates, disqualified),
        Expr::List(items) => {
            for item in items.iter() {
                check_expr_usage_strict(item, candidates, disqualified);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_crdt_register_fields(registry: &TypeRegistry, interner: &Interner) -> (HashSet<(String, String)>, HashSet<(String, String)>) {
    let mut lww_fields = HashSet::new();
    let mut mv_fields = HashSet::new();
    for (type_sym, def) in registry.iter_types() {
        if let TypeDef::Struct { fields, .. } = def {
            let type_name = interner.resolve(*type_sym).to_string();
            for field in fields {
                if let FieldType::Generic { base, .. } = &field.ty {
                    let base_name = interner.resolve(*base);
                    let field_name = interner.resolve(field.name).to_string();
                    if base_name == "LastWriteWins" {
                        lww_fields.insert((type_name.clone(), field_name));
                    } else if base_name == "Divergent" || base_name == "MVRegister" {
                        mv_fields.insert((type_name.clone(), field_name));
                    }
                }
            }
        }
    }
    (lww_fields, mv_fields)
}

/// Phase 102: Collect enum fields that need Box<T> for recursion.
/// Returns a set of (EnumName, VariantName, FieldName) tuples.
pub(super) fn collect_boxed_fields(registry: &TypeRegistry, interner: &Interner) -> HashSet<(String, String, String)> {
    let mut boxed_fields = HashSet::new();
    for (type_sym, def) in registry.iter_types() {
        if let TypeDef::Enum { variants, .. } = def {
            let enum_name = interner.resolve(*type_sym);
            for variant in variants {
                let variant_name = interner.resolve(variant.name);
                for field in &variant.fields {
                    if is_recursive_field(&field.ty, enum_name, interner) {
                        let field_name = interner.resolve(field.name).to_string();
                        boxed_fields.insert((
                            enum_name.to_string(),
                            variant_name.to_string(),
                            field_name,
                        ));
                    }
                }
            }
        }
    }
    boxed_fields
}

/// Phase 54: Collect function names that are async.
/// Used by LaunchTask codegen to determine if .await is needed.
///
/// Two-pass analysis:
/// 1. First pass: Collect directly async functions (have Sleep, LaunchTask, etc.)
/// 2. Second pass: Iterate until fixed point - if function calls an async function, mark it async
pub fn collect_async_functions(stmts: &[Stmt]) -> HashSet<Symbol> {
    // First, collect all function definitions
    let mut func_bodies: HashMap<Symbol, &[Stmt]> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            func_bodies.insert(*name, *body);
        }
    }

    // Pass 1: Collect directly async functions
    let mut async_fns = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            if body.iter().any(|s| requires_async_stmt(s)) {
                async_fns.insert(*name);
            }
        }
    }

    // Pass 2: Propagate async-ness through call graph until fixed point
    loop {
        let mut changed = false;
        for (func_name, body) in &func_bodies {
            if async_fns.contains(func_name) {
                continue; // Already marked async
            }
            // Check if this function calls any async function
            if body.iter().any(|s| calls_async_function(s, &async_fns)) {
                async_fns.insert(*func_name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    async_fns
}

/// Helper: Check if a statement calls any function in the async_fns set
pub(super) fn calls_async_function(stmt: &Stmt, async_fns: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Call { function, args } => {
            // Check if the called function is async OR if any argument expression calls an async function
            async_fns.contains(function)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        Stmt::If { cond, then_block, else_block } => {
            calls_async_function_in_expr(cond, async_fns)
                || then_block.iter().any(|s| calls_async_function(s, async_fns))
                || else_block.map_or(false, |b| b.iter().any(|s| calls_async_function(s, async_fns)))
        }
        Stmt::While { cond, body, .. } => {
            calls_async_function_in_expr(cond, async_fns)
                || body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Repeat { iterable, body, .. } => {
            calls_async_function_in_expr(iterable, async_fns)
                || body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Zone { body, .. } => {
            body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            tasks.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::FunctionDef { body, .. } => {
            body.iter().any(|s| calls_async_function(s, async_fns))
        }
        // Check Let statements for async function calls in the value expression
        Stmt::Let { value, .. } => calls_async_function_in_expr(value, async_fns),
        // Check Set statements for async function calls in the value expression
        Stmt::Set { value, .. } => calls_async_function_in_expr(value, async_fns),
        // Check Return statements for async function calls in the return value
        Stmt::Return { value } => {
            value.as_ref().map_or(false, |v| calls_async_function_in_expr(v, async_fns))
        }
        // Check RuntimeAssert condition for async calls
        Stmt::RuntimeAssert { condition, .. } => calls_async_function_in_expr(condition, async_fns),
        // Check Show for async calls
        Stmt::Show { object, .. } => calls_async_function_in_expr(object, async_fns),
        // Check Push for async calls
        Stmt::Push { collection, value } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(value, async_fns)
        }
        // Check SetIndex for async calls
        Stmt::SetIndex { collection, index, value } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(index, async_fns)
                || calls_async_function_in_expr(value, async_fns)
        }
        // Check SendPipe for async calls
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            calls_async_function_in_expr(value, async_fns)
                || calls_async_function_in_expr(pipe, async_fns)
        }
        // Check Inspect arms for async function calls
        Stmt::Inspect { target, arms, .. } => {
            calls_async_function_in_expr(target, async_fns)
                || arms.iter().any(|arm| arm.body.iter().any(|s| calls_async_function(s, async_fns)))
        }
        _ => false,
    }
}

/// Helper: Check if an expression calls any function in the async_fns set
fn calls_async_function_in_expr(expr: &Expr, async_fns: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Call { function, args } => {
            async_fns.contains(function)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        Expr::BinaryOp { left, right, .. } => {
            calls_async_function_in_expr(left, async_fns)
                || calls_async_function_in_expr(right, async_fns)
        }
        Expr::Index { collection, index } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(index, async_fns)
        }
        Expr::FieldAccess { object, .. } => calls_async_function_in_expr(object, async_fns),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| calls_async_function_in_expr(i, async_fns))
        }
        Expr::Closure { body, .. } => {
            match body {
                crate::ast::stmt::ClosureBody::Expression(expr) => calls_async_function_in_expr(expr, async_fns),
                crate::ast::stmt::ClosureBody::Block(_) => false,
            }
        }
        Expr::CallExpr { callee, args } => {
            calls_async_function_in_expr(callee, async_fns)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().any(|p| {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    calls_async_function_in_expr(value, async_fns)
                } else { false }
            })
        }
        _ => false,
    }
}

// =============================================================================
// Purity Analysis
// =============================================================================

pub(super) fn collect_pure_functions(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut func_bodies: HashMap<Symbol, &[Stmt]> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            func_bodies.insert(*name, *body);
        }
    }

    // Pass 1: Mark functions as impure if they directly contain impure statements
    let mut impure_fns = HashSet::new();
    for (func_name, body) in &func_bodies {
        if body.iter().any(|s| is_directly_impure_stmt(s)) {
            impure_fns.insert(*func_name);
        }
    }

    // Pass 2: Propagate impurity through call graph until fixed point
    loop {
        let mut changed = false;
        for (func_name, body) in &func_bodies {
            if impure_fns.contains(func_name) {
                continue;
            }
            if body.iter().any(|s| calls_impure_function(s, &impure_fns)) {
                impure_fns.insert(*func_name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Pure = all functions NOT in impure set
    let mut pure_fns = HashSet::new();
    for func_name in func_bodies.keys() {
        if !impure_fns.contains(func_name) {
            pure_fns.insert(*func_name);
        }
    }
    pure_fns
}

fn is_directly_impure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Show { .. }
        | Stmt::Give { .. }
        | Stmt::WriteFile { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::Listen { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::SendMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::Sleep { .. }
        | Stmt::Sync { .. }
        | Stmt::Mount { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::ResolveConflict { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::StopTask { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| is_directly_impure_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| is_directly_impure_stmt(s)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            body.iter().any(|s| is_directly_impure_stmt(s))
        }
        Stmt::Zone { body, .. } => {
            body.iter().any(|s| is_directly_impure_stmt(s))
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| is_directly_impure_stmt(s)))
        }
        _ => false,
    }
}

fn calls_impure_function(stmt: &Stmt, impure_fns: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Call { function, args } => {
            impure_fns.contains(function)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        Stmt::Let { value, .. } => expr_calls_impure(value, impure_fns),
        Stmt::Set { value, .. } => expr_calls_impure(value, impure_fns),
        Stmt::Return { value } => value.as_ref().map_or(false, |v| expr_calls_impure(v, impure_fns)),
        Stmt::If { cond, then_block, else_block } => {
            expr_calls_impure(cond, impure_fns)
                || then_block.iter().any(|s| calls_impure_function(s, impure_fns))
                || else_block.map_or(false, |b| b.iter().any(|s| calls_impure_function(s, impure_fns)))
        }
        Stmt::While { cond, body, .. } => {
            expr_calls_impure(cond, impure_fns)
                || body.iter().any(|s| calls_impure_function(s, impure_fns))
        }
        Stmt::Repeat { body, .. } => body.iter().any(|s| calls_impure_function(s, impure_fns)),
        Stmt::Zone { body, .. } => body.iter().any(|s| calls_impure_function(s, impure_fns)),
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| calls_impure_function(s, impure_fns)))
        }
        Stmt::Show { object, .. } => expr_calls_impure(object, impure_fns),
        Stmt::Push { value, collection } | Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            expr_calls_impure(value, impure_fns) || expr_calls_impure(collection, impure_fns)
        }
        _ => false,
    }
}

fn expr_calls_impure(expr: &Expr, impure_fns: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Call { function, args } => {
            impure_fns.contains(function)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_calls_impure(left, impure_fns) || expr_calls_impure(right, impure_fns)
        }
        Expr::Index { collection, index } => {
            expr_calls_impure(collection, impure_fns) || expr_calls_impure(index, impure_fns)
        }
        Expr::FieldAccess { object, .. } => expr_calls_impure(object, impure_fns),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_calls_impure(i, impure_fns)),
        Expr::CallExpr { callee, args } => {
            expr_calls_impure(callee, impure_fns)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().any(|p| {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    expr_calls_impure(value, impure_fns)
                } else { false }
            })
        }
        _ => false,
    }
}

// =============================================================================
// Memoization Detection
// =============================================================================

pub(super) fn count_self_calls(func_name: Symbol, body: &[Stmt]) -> usize {
    let mut count = 0;
    for stmt in body {
        count += count_self_calls_in_stmt(func_name, stmt);
    }
    count
}

fn count_self_calls_in_stmt(func_name: Symbol, stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Return { value: Some(expr) } => count_self_calls_in_expr(func_name, expr),
        Stmt::Let { value, .. } => count_self_calls_in_expr(func_name, value),
        Stmt::Set { value, .. } => count_self_calls_in_expr(func_name, value),
        Stmt::Call { function, args } => {
            let mut c = if *function == func_name { 1 } else { 0 };
            c += args.iter().map(|a| count_self_calls_in_expr(func_name, a)).sum::<usize>();
            c
        }
        Stmt::If { cond, then_block, else_block } => {
            let mut c = count_self_calls_in_expr(func_name, cond);
            c += count_self_calls(func_name, then_block);
            if let Some(else_stmts) = else_block {
                c += count_self_calls(func_name, else_stmts);
            }
            c
        }
        Stmt::While { cond, body, .. } => {
            count_self_calls_in_expr(func_name, cond) + count_self_calls(func_name, body)
        }
        Stmt::Repeat { body, .. } => count_self_calls(func_name, body),
        Stmt::Show { object, .. } => count_self_calls_in_expr(func_name, object),
        _ => 0,
    }
}

fn count_self_calls_in_expr(func_name: Symbol, expr: &Expr) -> usize {
    match expr {
        Expr::Call { function, args } => {
            let mut c = if *function == func_name { 1 } else { 0 };
            c += args.iter().map(|a| count_self_calls_in_expr(func_name, a)).sum::<usize>();
            c
        }
        Expr::BinaryOp { left, right, .. } => {
            count_self_calls_in_expr(func_name, left) + count_self_calls_in_expr(func_name, right)
        }
        Expr::Index { collection, index } => {
            count_self_calls_in_expr(func_name, collection) + count_self_calls_in_expr(func_name, index)
        }
        Expr::FieldAccess { object, .. } => count_self_calls_in_expr(func_name, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().map(|i| count_self_calls_in_expr(func_name, i)).sum()
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().map(|p| {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    count_self_calls_in_expr(func_name, value)
                } else { 0 }
            }).sum()
        }
        _ => 0,
    }
}

pub(super) fn is_hashable_type(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "Int" | "Nat" | "Bool" | "Char" | "Byte" | "Text"
                | "i64" | "u64" | "bool" | "char" | "u8" | "String")
        }
        TypeExpr::Refinement { base, .. } => is_hashable_type(base, interner),
        _ => false,
    }
}

pub(super) fn is_copy_type_expr(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "Int" | "Nat" | "Bool" | "Char" | "Byte"
                | "i64" | "u64" | "bool" | "char" | "u8")
        }
        TypeExpr::Refinement { base, .. } => is_copy_type_expr(base, interner),
        _ => false,
    }
}

pub(super) fn should_memoize(
    name: Symbol,
    body: &[Stmt],
    params: &[(Symbol, &TypeExpr)],
    return_type: Option<&TypeExpr>,
    is_pure: bool,
    interner: &Interner,
) -> bool {
    if !is_pure {
        return false;
    }
    if !body_contains_self_call(name, body) {
        return false;
    }
    if count_self_calls(name, body) < 2 {
        return false;
    }
    if params.is_empty() {
        return false;
    }
    if !params.iter().all(|(_, ty)| is_hashable_type(ty, interner)) {
        return false;
    }
    if return_type.is_none() {
        return false;
    }
    true
}

pub(super) fn body_contains_self_call(func_name: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| stmt_contains_self_call(func_name, s))
}

fn stmt_contains_self_call(func_name: Symbol, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { value: Some(expr) } => expr_contains_self_call(func_name, expr),
        Stmt::Return { value: None } => false,
        Stmt::Let { value, .. } => expr_contains_self_call(func_name, value),
        Stmt::Set { value, .. } => expr_contains_self_call(func_name, value),
        Stmt::Call { function, args } => {
            *function == func_name || args.iter().any(|a| expr_contains_self_call(func_name, a))
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_contains_self_call(func_name, cond)
                || then_block.iter().any(|s| stmt_contains_self_call(func_name, s))
                || else_block.map_or(false, |b| b.iter().any(|s| stmt_contains_self_call(func_name, s)))
        }
        Stmt::While { cond, body, .. } => {
            expr_contains_self_call(func_name, cond)
                || body.iter().any(|s| stmt_contains_self_call(func_name, s))
        }
        Stmt::Repeat { body, .. } => {
            body.iter().any(|s| stmt_contains_self_call(func_name, s))
        }
        Stmt::Show { object, .. } => expr_contains_self_call(func_name, object),
        _ => false,
    }
}

pub(super) fn expr_contains_self_call(func_name: Symbol, expr: &Expr) -> bool {
    match expr {
        Expr::Call { function, args } => {
            *function == func_name || args.iter().any(|a| expr_contains_self_call(func_name, a))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_self_call(func_name, left) || expr_contains_self_call(func_name, right)
        }
        Expr::Index { collection, index } => {
            expr_contains_self_call(func_name, collection) || expr_contains_self_call(func_name, index)
        }
        Expr::FieldAccess { object, .. } => expr_contains_self_call(func_name, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| expr_contains_self_call(func_name, i))
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().any(|p| {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    expr_contains_self_call(func_name, value)
                } else { false }
            })
        }
        _ => false,
    }
}

/// Check if an expression contains an `Expr::Index` that reads from the given collection symbol.
/// Used by SetIndex to decide whether the value expression aliases the collection being written to,
/// which requires a temporary binding to avoid borrow conflicts.
pub(super) fn expr_indexes_collection(expr: &Expr, coll_sym: Symbol) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            let indexes_this = matches!(collection, Expr::Identifier(sym) if *sym == coll_sym);
            indexes_this
                || expr_indexes_collection(collection, coll_sym)
                || expr_indexes_collection(index, coll_sym)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_indexes_collection(left, coll_sym) || expr_indexes_collection(right, coll_sym)
        }
        Expr::Call { args, .. } => {
            args.iter().any(|a| expr_indexes_collection(a, coll_sym))
        }
        Expr::FieldAccess { object, .. } => expr_indexes_collection(object, coll_sym),
        Expr::Length { collection } => expr_indexes_collection(collection, coll_sym),
        Expr::Not { operand } => expr_indexes_collection(operand, coll_sym),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| expr_indexes_collection(i, coll_sym))
        }
        _ => false,
    }
}

/// True if `expr` borrows ANY collection at runtime — an index (`item i of C`),
/// slice, or length read. Under LOGOS reference semantics any such collection
/// may alias the target of a `SetIndex`, so the RHS must be evaluated into a
/// temp BEFORE the target's `borrow_mut()` is taken; otherwise the live
/// `borrow()` and the `borrow_mut()` can land on the same `RefCell` and panic
/// ("already borrowed"). `expr_indexes_collection` only catches same-symbol
/// aliasing; this catches the cross-variable case (e.g. `prev` aliasing `curr`
/// after `Set prev to curr`).
pub(super) fn expr_reads_any_collection(expr: &Expr) -> bool {
    match expr {
        Expr::Index { .. } | Expr::Slice { .. } | Expr::Length { .. } => true,
        Expr::BinaryOp { left, right, .. } => {
            expr_reads_any_collection(left) || expr_reads_any_collection(right)
        }
        Expr::Call { args, .. } => args.iter().any(|a| expr_reads_any_collection(a)),
        Expr::FieldAccess { object, .. } => expr_reads_any_collection(object),
        Expr::Not { operand } => expr_reads_any_collection(operand),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_reads_any_collection(i)),
        _ => false,
    }
}

// =============================================================================
// Inline Annotation Detection
// =============================================================================

pub(super) fn should_inline(name: Symbol, body: &[Stmt], is_native: bool, is_exported: bool, is_async: bool) -> bool {
    !is_native && !is_exported && !is_async
        && body.len() <= 10
        && !body_contains_self_call(name, body)
}

// =============================================================================
// Pipe Detection
// =============================================================================

pub fn collect_pipe_sender_params(body: &[Stmt]) -> HashSet<Symbol> {
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
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_pipe_sender_params_stmt(s, senders);
            }
        }
        _ => {}
    }
}

/// Phase 54: Collect variables that are pipe declarations (created with CreatePipe).
/// These have _tx/_rx suffixes, while pipe parameters don't.
pub fn collect_pipe_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
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

/// Collect all identifier symbols from an expression recursively.
/// Used by Concurrent/Parallel codegen to find variables that need cloning.
pub(super) fn collect_expr_identifiers(expr: &Expr, identifiers: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => {
            identifiers.insert(*sym);
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_expr_identifiers(left, identifiers);
            collect_expr_identifiers(right, identifiers);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        Expr::Index { collection, index } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(index, identifiers);
        }
        Expr::Slice { collection, start, end } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(start, identifiers);
            collect_expr_identifiers(end, identifiers);
        }
        Expr::Copy { expr: inner } | Expr::Give { value: inner } | Expr::Length { collection: inner }
        | Expr::Not { operand: inner } => {
            collect_expr_identifiers(inner, identifiers);
        }
        Expr::Contains { collection, value } | Expr::Union { left: collection, right: value } | Expr::Intersection { left: collection, right: value } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(value, identifiers);
        }
        Expr::ManifestOf { zone } | Expr::ChunkAt { zone, .. } => {
            collect_expr_identifiers(zone, identifiers);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_expr_identifiers(item, identifiers);
            }
        }
        Expr::Range { start, end } => {
            collect_expr_identifiers(start, identifiers);
            collect_expr_identifiers(end, identifiers);
        }
        Expr::FieldAccess { object, .. } => {
            collect_expr_identifiers(object, identifiers);
        }
        Expr::New { init_fields, .. } => {
            for (_, value) in init_fields {
                collect_expr_identifiers(value, identifiers);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, value) in fields {
                collect_expr_identifiers(value, identifiers);
            }
        }
        Expr::OptionSome { value } => {
            collect_expr_identifiers(value, identifiers);
        }
        Expr::WithCapacity { value, capacity } => {
            collect_expr_identifiers(value, identifiers);
            collect_expr_identifiers(capacity, identifiers);
        }
        Expr::Closure { body, .. } => {
            match body {
                crate::ast::stmt::ClosureBody::Expression(expr) => collect_expr_identifiers(expr, identifiers),
                crate::ast::stmt::ClosureBody::Block(_) => {}
            }
        }
        Expr::CallExpr { callee, args } => {
            collect_expr_identifiers(callee, identifiers);
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                    collect_expr_identifiers(value, identifiers);
                }
            }
        }
        Expr::OptionNone => {}
        Expr::Escape { .. } => {}
        Expr::Literal(_) => {}
    }
}

/// Check if a symbol appears anywhere in a slice of statements (expressions or targets).
/// Used by dead-counter elimination to decide if a post-loop counter binding is needed.
pub(super) fn symbol_appears_in_stmts(sym: Symbol, stmts: &[&Stmt]) -> bool {
    stmts.iter().any(|s| symbol_appears_in_stmt(sym, s))
}

pub(super) fn symbol_appears_in_stmt(sym: Symbol, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { value, .. } => symbol_appears_in_expr(sym, value),
        Stmt::Set { target, value, .. } => *target == sym || symbol_appears_in_expr(sym, value),
        Stmt::Show { object, .. } => symbol_appears_in_expr(sym, object),
        Stmt::Return { value } => value.as_ref().map_or(false, |v| symbol_appears_in_expr(sym, v)),
        Stmt::Call { function, args } => *function == sym || args.iter().any(|a| symbol_appears_in_expr(sym, a)),
        Stmt::If { cond, then_block, else_block } => {
            symbol_appears_in_expr(sym, cond)
                || then_block.iter().any(|s| symbol_appears_in_stmt(sym, s))
                || else_block.map_or(false, |b| b.iter().any(|s| symbol_appears_in_stmt(sym, s)))
        }
        Stmt::While { cond, body, .. } => {
            symbol_appears_in_expr(sym, cond)
                || body.iter().any(|s| symbol_appears_in_stmt(sym, s))
        }
        Stmt::Repeat { iterable, body, .. } => {
            symbol_appears_in_expr(sym, iterable)
                || body.iter().any(|s| symbol_appears_in_stmt(sym, s))
        }
        Stmt::Zone { body, .. } => body.iter().any(|s| symbol_appears_in_stmt(sym, s)),
        Stmt::Push { collection, value } => {
            symbol_appears_in_expr(sym, collection) || symbol_appears_in_expr(sym, value)
        }
        Stmt::Pop { collection, .. } => symbol_appears_in_expr(sym, collection),
        Stmt::Add { collection, value } | Stmt::Remove { collection, value } => {
            symbol_appears_in_expr(sym, collection) || symbol_appears_in_expr(sym, value)
        }
        Stmt::SetIndex { collection, index, value } => {
            symbol_appears_in_expr(sym, collection) || symbol_appears_in_expr(sym, index) || symbol_appears_in_expr(sym, value)
        }
        Stmt::Inspect { target, arms, .. } => {
            symbol_appears_in_expr(sym, target)
                || arms.iter().any(|arm| arm.body.iter().any(|s| symbol_appears_in_stmt(sym, s)))
        }
        Stmt::RuntimeAssert { condition, .. } => symbol_appears_in_expr(sym, condition),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| symbol_appears_in_stmt(sym, s)),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            tasks.iter().any(|s| symbol_appears_in_stmt(sym, s))
        }
        _ => false,
    }
}

pub(super) fn symbol_appears_in_expr(sym: Symbol, expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            symbol_appears_in_expr(sym, left) || symbol_appears_in_expr(sym, right)
        }
        Expr::Call { function, args } => {
            *function == sym || args.iter().any(|a| symbol_appears_in_expr(sym, a))
        }
        Expr::Index { collection, index } => {
            symbol_appears_in_expr(sym, collection) || symbol_appears_in_expr(sym, index)
        }
        Expr::Slice { collection, start, end } => {
            symbol_appears_in_expr(sym, collection)
                || symbol_appears_in_expr(sym, start)
                || symbol_appears_in_expr(sym, end)
        }
        Expr::Length { collection } | Expr::Copy { expr: collection } | Expr::Give { value: collection } => {
            symbol_appears_in_expr(sym, collection)
        }
        Expr::Contains { collection, value } | Expr::Union { left: collection, right: value } | Expr::Intersection { left: collection, right: value } => {
            symbol_appears_in_expr(sym, collection) || symbol_appears_in_expr(sym, value)
        }
        Expr::FieldAccess { object, .. } => symbol_appears_in_expr(sym, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| symbol_appears_in_expr(sym, i))
        }
        Expr::Range { start, end } => {
            symbol_appears_in_expr(sym, start) || symbol_appears_in_expr(sym, end)
        }
        Expr::New { init_fields, .. } => {
            init_fields.iter().any(|(_, v)| symbol_appears_in_expr(sym, v))
        }
        Expr::NewVariant { fields, .. } => {
            fields.iter().any(|(_, v)| symbol_appears_in_expr(sym, v))
        }
        Expr::OptionSome { value } => symbol_appears_in_expr(sym, value),
        Expr::WithCapacity { value, capacity } => {
            symbol_appears_in_expr(sym, value) || symbol_appears_in_expr(sym, capacity)
        }
        Expr::CallExpr { callee, args } => {
            symbol_appears_in_expr(sym, callee)
                || args.iter().any(|a| symbol_appears_in_expr(sym, a))
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().any(|p| {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    symbol_appears_in_expr(sym, value)
                } else { false }
            })
        }
        Expr::Closure { body, .. } => {
            match body {
                crate::ast::stmt::ClosureBody::Expression(e) => symbol_appears_in_expr(sym, e),
                crate::ast::stmt::ClosureBody::Block(stmts) => stmts.iter().any(|s| symbol_appears_in_stmt(sym, s)),
            }
        }
        _ => false,
    }
}

/// Check if a TypeExpr is a Vec/Seq/List type (collection that could be borrowed as &[T]).
pub(crate) fn is_vec_type_expr(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Generic { base, .. } => {
            let name = interner.resolve(*base);
            matches!(name, "Seq" | "List" | "Vec")
        }
        _ => false,
    }
}

/// For a function's parameters and body, return the set of parameter indices
/// whose Vec<T> params are read-only (never mutated in the body).
/// These can safely be borrowed as &[T] instead of owned Vec<T>.
pub(super) fn collect_readonly_vec_param_indices(
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    interner: &Interner,
) -> HashSet<usize> {
    let mutable_vars = collect_mutable_vars(body);
    let mut readonly_indices = HashSet::new();
    for (i, (param_sym, param_ty)) in params.iter().enumerate() {
        if is_vec_type_expr(param_ty, interner) && !mutable_vars.contains(param_sym) {
            readonly_indices.insert(i);
        }
    }
    readonly_indices
}

/// Convert a Vec<T> or LogosSeq<T> type string to a slice &[T] type string.
/// E.g., "Vec<i64>" → "&[i64]", "LogosSeq<i64>" → "&[i64]"
pub(super) fn vec_to_slice_type(vec_type: &str) -> String {
    if let Some(inner) = vec_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        format!("&[{}]", inner)
    } else if let Some(inner) = vec_type.strip_prefix("LogosSeq<").and_then(|s| s.strip_suffix('>')) {
        format!("&[{}]", inner)
    } else {
        vec_type.to_string()
    }
}

/// Convert `Vec<T>` or `LogosSeq<T>` to `&mut [T]` for mutable borrow parameters.
pub(super) fn vec_to_mut_slice_type(vec_type: &str) -> String {
    if let Some(inner) = vec_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        format!("&mut [{}]", inner)
    } else if let Some(inner) = vec_type.strip_prefix("LogosSeq<").and_then(|s| s.strip_suffix('>')) {
        format!("&mut [{}]", inner)
    } else {
        vec_type.to_string()
    }
}

/// O2 de-Rc eligibility: Seq variables that provably never need reference
/// semantics, so codegen can emit a plain `Vec<T>` (no Rc/RefCell) instead of
/// `LogosSeq<T> = Rc<RefCell<Vec<T>>>`.
///
/// A variable qualifies when it is declared with a FRESH allocation of a Seq
/// whose element type is a scalar (Int/Float/Bool/Char/Text — reading an
/// element copies, never shares a handle) and, across the whole scope, it is
/// never aliased by a second live handle (`Let b be a`, `Set b to a`, stored
/// as an element) and never escapes (call arg, return, given away).
///
/// Conservative by construction: anything uncertain stays `LogosSeq`. An
/// unsound de-Rc would make the generated Rust fail to compile (two owners /
/// moved-from use), a loud failure the tests catch — never silent corruption.
pub(crate) fn collect_de_rc_seqs(
    stmts: &[Stmt],
    interner: &Interner,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
    vec_return_fns: &HashSet<Symbol>,
    returns_vec: bool,
) -> HashSet<Symbol> {
    // Kill switch (benchmark A/B): `LOGOS_DERC=0` keeps every Seq as `LogosSeq`.
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unbox) {
        return HashSet::new();
    }
    // Phase 3a/3b: a candidate passed at EITHER a readonly-borrow (`&[T]`) or a
    // mutable-borrow (`&mut [T]`) param slot is only borrowed for the call, not
    // retained — that is not an escape. Both call conventions pass a reference
    // to the de-Rc'd `Vec`, so the use-scan treats both as safe slots. (Codegen
    // distinguishes `&x` vs `&mut x` via its own borrow maps.)
    let borrow_slots: HashMap<Symbol, HashSet<usize>> = if mut_borrow_params.is_empty() {
        borrow_params.clone()
    } else {
        let mut m = borrow_params.clone();
        for (f, idx) in mut_borrow_params {
            m.entry(*f).or_default().extend(idx.iter().copied());
        }
        m
    };
    let borrow_params = &borrow_slots;
    let mut candidates = HashSet::new();
    collect_de_rc_candidates_block(stmts, interner, vec_return_fns, &mut candidates);
    if candidates.is_empty() {
        return candidates;
    }
    // A candidate survives ONLY if every one of its occurrences is in a
    // Vec-safe slot (the collection of push/pop/add/remove/setindex/index/
    // length, or a fresh-Seq decl/reassign). Any other occurrence — call arg,
    // return, given/shown whole, sliced, copied, stored as an element,
    // aliased, or inside any statement/expression kind not explicitly
    // permitted — disqualifies it. Conservative by construction: the worst
    // outcome of the catch-alls is a missed optimization, never an unsound
    // `Vec` used where a `LogosSeq` is required.
    // Phase 2: buffer-reuse swap pairs. `Set outer to inner`, where `inner` is
    // a fresh-per-iteration buffer, is lowered by codegen to `std::mem::swap`
    // (a content exchange, NOT a shared handle), so it must not be treated as
    // aliasing. Collect those (outer, inner) pairs so the use-scan exempts them.
    let mut swaps = HashSet::new();
    collect_buffer_swap_pairs(stmts, interner, &mut swaps);

    let mut disqualified = HashSet::new();
    derc_scan_uses(stmts, &candidates, &swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, &mut disqualified);
    candidates.retain(|c| !disqualified.contains(c));

    // A swap pair must de-Rc together — `std::mem::swap` needs both partners to
    // be the same type. If either is disqualified, drop both. Iterate to a
    // fixpoint so chained pairs settle.
    loop {
        let mut changed = false;
        for (outer, inner) in &swaps {
            if candidates.contains(outer) != candidates.contains(inner) {
                changed |= candidates.remove(outer);
                changed |= candidates.remove(inner);
            }
        }
        if !changed {
            break;
        }
    }
    if !candidates.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Unbox);
    }
    candidates
}

/// Phase 4 — return-type de-Rc. A function declared to return `Seq of T` can
/// instead return `Vec<T>` when EVERY `Return` yields a uniquely-owned value:
/// a freshly-built local Seq (moved out), a readonly-BORROW param (copied via
/// `.to_vec()` — a borrow can't alias), a fresh-Seq expression, or a call to
/// another such function. Returning `Vec` removes the per-call Rc clone +
/// RefCell borrow and the Rc box, and — crucially — makes `Set x to f(...)` at
/// every call site a uniquely-owned fresh value, unblocking de-Rc on `x` (the
/// mergesort `left`/`right`/`result` chain, currently disqualified because the
/// callee returns `LogosSeq`). Fixpoint over the callgraph so a function that
/// returns a call to a not-yet-confirmed peer settles.
pub(super) fn collect_vec_return_fns(
    stmts: &[Stmt],
    interner: &Interner,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
) -> HashSet<Symbol> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unbox) {
        return HashSet::new();
    }
    // Every non-native function declared to return a Seq/List/Vec, with its
    // body + params. Each is a CANDIDATE to instead return an owned `Vec<T>`.
    let mut seq_fns: Vec<(Symbol, &[Stmt], &[(Symbol, &TypeExpr)])> = Vec::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name, body, params, return_type: Some(rt), is_native: false, ..
        } = s
        {
            if is_vec_type_expr(rt, interner) {
                seq_fns.push((*name, body, params));
            }
        }
    }
    if seq_fns.is_empty() {
        return HashSet::new();
    }

    // GREATEST FIXPOINT — return-type de-Rc is an interprocedural aliasing
    // problem co-dependent with each scope's local de-Rc set: a function `f`
    // returns `Vec` only if every return site AND every call site is owned-Vec
    // compatible, while a call site's result de-Rc's only if `f` returns `Vec`.
    // Start OPTIMISTIC (every candidate returns `Vec`), then SHRINK on any
    // soundness violation until stable. Monotone (removals only) ⇒ converges.
    //
    // `f` is removed when ANY holds (each would emit ill-typed `Vec`/`LogosSeq`
    // code, never silent corruption — the violations are type errors):
    //   (a) a `Return` in `f` yields a value that is not an owned Vec — i.e. NOT
    //       a readonly-borrow param (returns `.to_vec()`) and NOT a local that
    //       itself de-Rc's (returns the moved `Vec`). A LogosSeq local return
    //       would be `return os;` into a `-> Vec` signature.
    //   (b) a call site `Let/Set x to f(...)` whose result `x` does not de-Rc —
    //       a `Vec` assigned into a `LogosSeq` variable.
    //   (c) `f(...)` used in ANY inline / non-binding position (struct field,
    //       owned arg, `Return f(x)`, `Show f(x)`, nested expression) — a `Vec`
    //       where a `LogosSeq` is expected. Conservative: only a top-level
    //       `Let/Set x to f(...)` binding consumes the owned `Vec` cleanly.
    let mut vec_fns: HashSet<Symbol> = seq_fns.iter().map(|(n, _, _)| *n).collect();
    loop {
        let mut remove = HashSet::new();

        // (c) any inline use anywhere in the program (recurses into fn bodies).
        flag_inline_vec_calls(stmts, &vec_fns, &mut remove);

        // (a)+(b), per scope, with that scope's de-Rc set computed under the
        // CURRENT candidate set. Main's scope is the whole program slice
        // (`collect_de_rc_seqs` skips nested fn defs); each function's scope is
        // its own body with `returns_vec` = whether it is still a candidate.
        let main_de_rc = collect_de_rc_seqs(stmts, interner, borrow_params, mut_borrow_params, &vec_fns, false);
        collect_unsound_vec_returns(stmts, &vec_fns, &main_de_rc, &mut remove);
        for (name, body, params) in &seq_fns {
            let rv = vec_fns.contains(name);
            let de_rc = collect_de_rc_seqs(body, interner, borrow_params, mut_borrow_params, &vec_fns, rv);
            collect_unsound_vec_returns(body, &vec_fns, &de_rc, &mut remove);
            if rv && !all_returns_vec_ownable(body, params, borrow_params.get(name), &de_rc) {
                remove.insert(*name);
            }
        }

        if remove.is_empty() {
            break;
        }
        vec_fns.retain(|f| !remove.contains(f));
    }
    if !vec_fns.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Unbox);
    }
    vec_fns
}

/// Record any `f ∈ vec_fns` that has a call site `Let/Set x to f(...)` whose
/// result variable `x` is NOT de-Rc'd in this scope — returning `Vec` there
/// would assign a `Vec` into a `LogosSeq` variable.
fn collect_unsound_vec_returns(
    stmts: &[Stmt],
    vec_fns: &HashSet<Symbol>,
    de_rc: &HashSet<Symbol>,
    remove: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        let binding = match stmt {
            Stmt::Let { var, value, .. } => Some((*var, value)),
            Stmt::Set { target, value } => Some((*target, value)),
            _ => None,
        };
        if let Some((x, Expr::Call { function, .. })) = binding {
            if vec_fns.contains(function) && !de_rc.contains(&x) {
                remove.insert(*function);
            }
        }
        match stmt {
            Stmt::If { then_block, else_block, .. } => {
                collect_unsound_vec_returns(then_block, vec_fns, de_rc, remove);
                if let Some(eb) = else_block {
                    collect_unsound_vec_returns(eb, vec_fns, de_rc, remove);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                collect_unsound_vec_returns(body, vec_fns, de_rc, remove)
            }
            _ => {}
        }
    }
}

/// Condition (a): every `Return` in `body` yields a value the codegen can hand
/// back as an owned `Vec<T>`. Only two return shapes are owned-Vec compatible:
///   - a readonly-BORROW param (idx ∈ `borrow_idx`) → `return p.to_vec();` (a
///     copy — a borrow can never alias the caller's value), or
///   - a LOCAL that itself de-Rc's (`x ∈ de_rc`) → `return x;` (the moved Vec).
/// Anything else — a LogosSeq local, a bare fresh/slice/copy expression, an
/// owned param, a valueless `Return` — would emit a value of the wrong type
/// into the `-> Vec<T>` signature, so it disqualifies the function. (Inline
/// peer-call returns like `Return g(x)` are handled by the inline-use check.)
fn all_returns_vec_ownable(
    body: &[Stmt],
    params: &[(Symbol, &TypeExpr)],
    borrow_idx: Option<&HashSet<usize>>,
    de_rc: &HashSet<Symbol>,
) -> bool {
    fn walk(
        stmts: &[Stmt],
        params: &[(Symbol, &TypeExpr)],
        borrow_idx: Option<&HashSet<usize>>,
        de_rc: &HashSet<Symbol>,
        ok: &mut bool,
    ) {
        for s in stmts {
            match s {
                Stmt::Return { value: Some(e) } => {
                    if !is_vec_ownable_return(e, params, borrow_idx, de_rc) {
                        *ok = false;
                    }
                }
                Stmt::Return { value: None } => *ok = false,
                Stmt::If { then_block, else_block, .. } => {
                    walk(then_block, params, borrow_idx, de_rc, ok);
                    if let Some(eb) = else_block {
                        walk(eb, params, borrow_idx, de_rc, ok);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    walk(body, params, borrow_idx, de_rc, ok)
                }
                _ => {}
            }
        }
    }
    let mut ok = true;
    walk(body, params, borrow_idx, de_rc, &mut ok);
    ok
}

fn is_vec_ownable_return(
    e: &Expr,
    params: &[(Symbol, &TypeExpr)],
    borrow_idx: Option<&HashSet<usize>>,
    de_rc: &HashSet<Symbol>,
) -> bool {
    match e {
        Expr::Identifier(x) => match params.iter().position(|(p, _)| p == x) {
            // A readonly-BORROW param: `return p.to_vec();` — a copy, cannot alias.
            Some(idx) => borrow_idx.map_or(false, |b| b.contains(&idx)),
            // A LOCAL that de-Rc's: it is already a `Vec<T>` → `return x;`.
            None => de_rc.contains(x),
        },
        _ => false,
    }
}

/// Condition (c): flag every `f ∈ vec_fns` whose call result is consumed in an
/// inline / non-binding position — anywhere other than the outermost RHS of a
/// top-level `Let/Set x to f(...)`. Such a position (struct field, owned arg,
/// `Return f(x)`, `Show f(x)`, a nested sub-expression, a control condition)
/// expects a `LogosSeq`, so an owned `Vec` return would not type-check.
/// Recurses into nested function bodies. Uses `symbol_appears_in_*` (the same
/// complete walker the de-Rc disqualification rests on) so its coverage matches.
fn flag_inline_vec_calls(
    stmts: &[Stmt],
    vec_fns: &HashSet<Symbol>,
    remove: &mut HashSet<Symbol>,
) {
    let flag_in_expr = |e: &Expr, remove: &mut HashSet<Symbol>| {
        for &f in vec_fns {
            if symbol_appears_in_expr(f, e) {
                remove.insert(f);
            }
        }
    };
    for stmt in stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                // The outermost call of a binding RHS is the one SAFE consuming
                // position (its result is named; `collect_unsound_vec_returns`
                // separately requires that name to de-Rc). Only the call's ARGS
                // are inline positions here.
                if let Expr::Call { args, .. } = value {
                    for a in args.iter() {
                        flag_in_expr(a, remove);
                    }
                } else {
                    flag_in_expr(value, remove);
                }
            }
            Stmt::If { cond, then_block, else_block } => {
                flag_in_expr(cond, remove);
                flag_inline_vec_calls(then_block, vec_fns, remove);
                if let Some(eb) = else_block {
                    flag_inline_vec_calls(eb, vec_fns, remove);
                }
            }
            Stmt::While { cond, body, .. } => {
                flag_in_expr(cond, remove);
                flag_inline_vec_calls(body, vec_fns, remove);
            }
            Stmt::Repeat { iterable, body, .. } => {
                flag_in_expr(iterable, remove);
                flag_inline_vec_calls(body, vec_fns, remove);
            }
            Stmt::Zone { body, .. } | Stmt::FunctionDef { body, .. } => {
                flag_inline_vec_calls(body, vec_fns, remove)
            }
            // Every other statement kind (Return, Show, Give, Push, Add, Remove,
            // SetIndex, Call, Inspect, …): a vec-fn call anywhere inside is an
            // inline use. `symbol_appears_in_stmt` is the complete walker.
            other => {
                for &f in vec_fns {
                    if symbol_appears_in_stmt(f, other) {
                        remove.insert(f);
                    }
                }
            }
        }
    }
}

/// `value` is a call to a function that returns an owned `Vec` (Phase 4) — so
/// `Set x to value` binds `x` to a uniquely-owned fresh value, not a shared
/// callee handle.
fn is_vec_return_call(value: &Expr, vec_return_fns: &HashSet<Symbol>) -> bool {
    matches!(value, Expr::Call { function, .. } if vec_return_fns.contains(function))
}

/// Phase 3b: `value` is a call `f(..., target, ...)` where `f` mutably borrows
/// `target` at the position `target` appears. Codegen lowers `Set target to
/// f(...)` to the in-place call `f(&mut target, ...)`, so it is a mutation of
/// `target`, not a rebinding — `target` keeps its identity and de-Rc eligibility.
fn is_mut_borrow_inplace_call(
    target: Symbol,
    value: &Expr,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
) -> bool {
    if let Expr::Call { function, args } = value {
        if let Some(slots) = mut_borrow_params.get(function) {
            return args.iter().enumerate().any(|(i, a)| {
                slots.contains(&i) && matches!(a, Expr::Identifier(s) if *s == target)
            });
        }
    }
    false
}

/// Collect `(outer, inner)` pairs where `Set outer to inner` is a buffer-reuse
/// swap: `inner` is declared `Let mutable inner = <fresh Seq>` earlier in the
/// same loop body and is not referenced after the `Set`. These exactly mirror
/// `detect_buffer_reuse_in_body`'s shape, so codegen lowers each to
/// `std::mem::swap` — content exchange, not aliasing.
fn collect_buffer_swap_pairs(
    stmts: &[Stmt],
    interner: &Interner,
    out: &mut HashSet<(Symbol, Symbol)>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                detect_swap_in_body(body, interner, out);
                collect_buffer_swap_pairs(body, interner, out);
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_buffer_swap_pairs(then_block, interner, out);
                if let Some(eb) = else_block {
                    collect_buffer_swap_pairs(eb, interner, out);
                }
            }
            Stmt::Zone { body, .. } => collect_buffer_swap_pairs(body, interner, out),
            _ => {}
        }
    }
}

fn detect_swap_in_body(body: &[Stmt], interner: &Interner, out: &mut HashSet<(Symbol, Symbol)>) {
    let mut fresh_inner: HashSet<Symbol> = HashSet::new();
    for stmt in body {
        if let Stmt::Let { var, value, mutable: true, .. } = stmt {
            if is_fresh_seq_value(value, interner) {
                fresh_inner.insert(*var);
            }
        }
    }
    if fresh_inner.is_empty() {
        return;
    }
    for (idx, stmt) in body.iter().enumerate() {
        if let Stmt::Set { target, value: Expr::Identifier(src) } = stmt {
            if *target != *src && fresh_inner.contains(src) {
                let used_after = body[idx + 1..]
                    .iter()
                    .any(|s| symbol_appears_in_stmt(*src, s));
                if !used_after {
                    out.insert((*target, *src));
                }
            }
        }
    }
}

/// Walk every statement, disqualifying any candidate that appears outside a
/// Vec-safe slot. Buffer-reuse swap pairs are exempt from the alias rule.
fn derc_scan_uses(
    stmts: &[Stmt],
    cands: &HashSet<Symbol>,
    swaps: &HashSet<(Symbol, Symbol)>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
    vec_return_fns: &HashSet<Symbol>,
    returns_vec: bool,
    interner: &Interner,
    dq: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        derc_scan_stmt(stmt, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq);
    }
}

/// Phase 3: a candidate passed at a READONLY-borrow-param position (`&[T]`) is
/// only borrowed, never retained — that is NOT an escape, so it stays eligible
/// and the call site passes `&vec`. A candidate at any other arg position
/// (owned param, or a callee with no borrow info) escapes and is disqualified.
fn derc_scan_call_args(
    function: Symbol,
    args: &[&Expr],
    cands: &HashSet<Symbol>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    dq: &mut HashSet<Symbol>,
) {
    let slots = borrow_params.get(&function);
    for (i, a) in args.iter().enumerate() {
        match a {
            Expr::Identifier(s) if cands.contains(s) => {
                let is_borrow_slot = slots.map_or(false, |set| set.contains(&i));
                if !is_borrow_slot {
                    dq.insert(*s);
                }
            }
            _ => derc_scan_expr(a, cands, borrow_params, dq),
        }
    }
}

/// True if `value` is a fresh Seq/List allocation (a safe decl/reassign source).
fn is_fresh_seq_value(value: &Expr, interner: &Interner) -> bool {
    match value {
        Expr::New { type_name, init_fields, .. } if init_fields.is_empty() => {
            matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec")
        }
        Expr::WithCapacity { value: inner, .. } => is_fresh_seq_value(inner, interner),
        Expr::List(_) => true,
        _ => false,
    }
}

fn derc_scan_stmt(
    stmt: &Stmt,
    cands: &HashSet<Symbol>,
    swaps: &HashSet<(Symbol, Symbol)>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
    vec_return_fns: &HashSet<Symbol>,
    returns_vec: bool,
    interner: &Interner,
    dq: &mut HashSet<Symbol>,
) {
    match stmt {
        // A fresh-Seq decl is the safe origin; otherwise the initializer is a
        // general use of whatever it references.
        Stmt::Let { value, .. } => {
            if is_vec_return_call(value, vec_return_fns) {
                // Phase 4: `Let r be f(...)` with f returning an OWNED Vec — r
                // captures a uniquely-owned fresh value, not a shared handle.
                // Still scan the call's args.
                if let Expr::Call { function, args } = value {
                    derc_scan_call_args(*function, args, cands, borrow_params, dq);
                }
            } else if !is_fresh_seq_value(value, interner) {
                derc_scan_expr(value, cands, borrow_params, dq);
            }
        }
        Stmt::Set { target, value } => {
            if let Expr::Identifier(src) = value {
                if swaps.contains(&(*target, *src)) {
                    // Buffer-reuse swap → content exchange (`std::mem::swap`),
                    // not aliasing. Both partners stay eligible.
                } else if target != src {
                    // `Set x to y` rebinds x onto y's allocation — both lose
                    // unique ownership.
                    dq.insert(*src);
                    dq.insert(*target);
                }
            } else if is_vec_return_call(value, vec_return_fns) {
                // Phase 4: `Set x to f(...)` where f returns an OWNED Vec — x
                // captures a uniquely-owned fresh value, not a shared callee
                // handle, so x stays eligible. Still scan the call's args (a
                // candidate passed to a NON-borrow param would escape).
                if let Expr::Call { function, args } = value {
                    derc_scan_call_args(*function, args, cands, borrow_params, dq);
                }
            } else if is_mut_borrow_inplace_call(*target, value, mut_borrow_params) {
                // Phase 3b: `Set x to f(..., x, ...)` where `f` mut-borrows `x`
                // at x's position is lowered to an IN-PLACE call `f(&mut x, ...)`
                // — the "result" is x itself, not a reassignment, so x keeps its
                // unique ownership and stays eligible. Scan the OTHER args (a
                // candidate passed to a non-borrow slot still escapes).
                if let Expr::Call { function, args } = value {
                    derc_scan_call_args(*function, args, cands, borrow_params, dq);
                }
            } else if !is_fresh_seq_value(value, interner) {
                // `Set x to <non-fresh value>` (a call result, slice, copy, …)
                // rebinds x to a value that is NOT a uniquely-owned fresh Vec —
                // e.g. `Set right to mergeSort(right)` binds right to a callee's
                // returned `LogosSeq`. Disqualify the target.
                dq.insert(*target);
                derc_scan_expr(value, cands, borrow_params, dq);
            }
        }
        // Collection slots are safe for a bare candidate; the value/index are
        // general uses.
        Stmt::Push { value, collection } | Stmt::Add { value, collection } => {
            derc_scan_collection_slot(collection, cands, borrow_params, dq);
            derc_scan_expr(value, cands, borrow_params, dq);
        }
        Stmt::Pop { collection, .. } => derc_scan_collection_slot(collection, cands, borrow_params, dq),
        Stmt::Remove { value, collection } => {
            derc_scan_collection_slot(collection, cands, borrow_params, dq);
            derc_scan_expr(value, cands, borrow_params, dq);
        }
        Stmt::SetIndex { collection, index, value } => {
            derc_scan_collection_slot(collection, cands, borrow_params, dq);
            derc_scan_expr(index, cands, borrow_params, dq);
            derc_scan_expr(value, cands, borrow_params, dq);
        }
        Stmt::If { cond, then_block, else_block } => {
            derc_scan_expr(cond, cands, borrow_params, dq);
            derc_scan_uses(then_block, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq);
            if let Some(eb) = else_block {
                derc_scan_uses(eb, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq);
            }
        }
        Stmt::While { cond, body, .. } => {
            derc_scan_expr(cond, cands, borrow_params, dq);
            derc_scan_uses(body, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq);
        }
        Stmt::Repeat { iterable, body, .. } => {
            // Iterating a de-Rc'd Vec is not lowered in v1 → treat as a use.
            derc_scan_expr(iterable, cands, borrow_params, dq);
            derc_scan_uses(body, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq);
        }
        Stmt::Zone { body, .. } => derc_scan_uses(body, cands, swaps, borrow_params, mut_borrow_params, vec_return_fns, returns_vec, interner, dq),
        // Statements that carry expressions which may legitimately pass a
        // candidate to a borrow-param: scan position-aware.
        Stmt::Call { function, args } => {
            derc_scan_call_args(*function, args, cands, borrow_params, dq);
        }
        // Phase 4: in a Vec-returning function, returning a candidate LOCAL
        // moves it out as the owned Vec result — not a disqualifying escape.
        Stmt::Return { value: Some(e) } => {
            if !(returns_vec && matches!(e, Expr::Identifier(x) if cands.contains(x))) {
                derc_scan_expr(e, cands, borrow_params, dq);
            }
        }
        Stmt::Return { value: None } => {}
        Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
            derc_scan_expr(object, cands, borrow_params, dq);
            derc_scan_expr(recipient, cands, borrow_params, dq);
        }
        // A nested function is a SEPARATE scope with its own de-Rc analysis. Its
        // parameter/local symbols are distinct bindings even when they reuse an
        // outer name (the interner gives `arr` one Symbol, but main's `arr` and a
        // function's `arr` param are different variables). The outer scan must
        // NOT descend — a candidate can only reach a function through explicit
        // call args, which are checked at the call site. Skipping keeps a
        // name-collision from spuriously disqualifying an outer candidate.
        Stmt::FunctionDef { .. } => {}
        // Every other statement kind (SetField, Concurrent, …): any candidate
        // that appears anywhere in it is an unsafe use. `symbol_appears_in_stmt`
        // is a complete walker.
        other => {
            for &c in cands {
                if symbol_appears_in_stmt(c, other) {
                    dq.insert(c);
                }
            }
        }
    }
}

/// The collection position of an indexing/mutation op: a bare candidate
/// identifier here is the SAFE Vec target; a nested expression is scanned.
fn derc_scan_collection_slot(
    collection: &Expr,
    cands: &HashSet<Symbol>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    dq: &mut HashSet<Symbol>,
) {
    match collection {
        Expr::Identifier(_) => {}
        other => derc_scan_expr(other, cands, borrow_params, dq),
    }
}

/// Disqualify any candidate that appears in `expr` outside an index/length
/// collection slot. Expression kinds that may legitimately read a candidate by
/// index (BinaryOp, Not, Call args) are recursed position-aware; every other
/// kind is treated conservatively — any candidate inside is an unsafe use.
fn derc_scan_expr(
    expr: &Expr,
    cands: &HashSet<Symbol>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    dq: &mut HashSet<Symbol>,
) {
    match expr {
        Expr::Identifier(s) => {
            if cands.contains(s) {
                dq.insert(*s);
            }
        }
        Expr::Index { collection, index } => {
            derc_scan_collection_slot(collection, cands, borrow_params, dq);
            derc_scan_expr(index, cands, borrow_params, dq);
        }
        Expr::Length { collection } => derc_scan_collection_slot(collection, cands, borrow_params, dq),
        Expr::BinaryOp { left, right, .. } => {
            derc_scan_expr(left, cands, borrow_params, dq);
            derc_scan_expr(right, cands, borrow_params, dq);
        }
        Expr::Not { operand } => derc_scan_expr(operand, cands, borrow_params, dq),
        Expr::Call { function, args } => {
            derc_scan_call_args(*function, args, cands, borrow_params, dq);
        }
        // Slice, Copy, FieldAccess, interpolation, New init-fields, and any
        // other kind: a candidate appearing inside is an unsafe use.
        // `symbol_appears_in_expr` is a complete walker.
        other => {
            for &c in cands {
                if symbol_appears_in_expr(c, other) {
                    dq.insert(c);
                }
            }
        }
    }
}

/// Rust element type of a homogeneous **scalar-literal** list, or `None` when the list is
/// empty, mixed-kind, or has any non-scalar-literal element. `[1,2,3]`→`i64`, `[1.0,…]`→`f64`,
/// `[true,…]`→`bool`, `['a',…]`→`char`. Such a list is a uniquely-owned fresh value with no
/// borrowed handle inside it, so it de-Rc's from `LogosSeq<T>` to a plain `Vec<T>`. Detection
/// (`fresh_scalar_seq_elem`) and codegen (`derc_vec_decl`) BOTH route through this one helper
/// so their eligibility can never drift.
pub(crate) fn homogeneous_scalar_literal_elem(items: &[&Expr]) -> Option<String> {
    fn elem_ty(e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Literal(Literal::Number(_)) => Some("i64"),
            Expr::Literal(Literal::Float(_)) => Some("f64"),
            Expr::Literal(Literal::Boolean(_)) => Some("bool"),
            Expr::Literal(Literal::Char(_)) => Some("char"),
            _ => None,
        }
    }
    let first = elem_ty(items.first()?)?;
    if items.iter().all(|i| elem_ty(i) == Some(first)) {
        Some(first.to_string())
    } else {
        None
    }
}

/// The Rust element type string if `value` freshly allocates a Seq of scalars.
fn fresh_scalar_seq_elem(value: &Expr, interner: &Interner) -> Option<String> {
    match value {
        Expr::New { type_name, type_args, init_fields } if init_fields.is_empty() => {
            match interner.resolve(*type_name) {
                "Seq" | "List" | "Vec" => {
                    let elem = type_args.first()?;
                    let ty = codegen_type_expr(elem, interner);
                    if is_scalar_elem_type(&ty) { Some(ty) } else { None }
                }
                _ => None,
            }
        }
        Expr::WithCapacity { value: inner, .. } => fresh_scalar_seq_elem(inner, interner),
        // A homogeneous SCALAR-literal list (`[1,2,3]`, `[1.0,…]`, `[true,…]`, `['a',…]`) is a
        // uniquely-owned fresh Seq — de-Rc it to a plain `Vec<T>` so indexed access skips the
        // RefCell borrow + the Rc box. The use-scan still disqualifies any occurrence that
        // escapes a Vec-safe slot.
        Expr::List(items) => homogeneous_scalar_literal_elem(items),
        _ => None,
    }
}

/// Scalar element types whose reads copy (no shared handle): safe to de-Rc.
fn is_scalar_elem_type(ty: &str) -> bool {
    matches!(
        ty,
        "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "isize"
            | "f64" | "f32" | "bool" | "char" | "String"
            // Fixed-width word newtypes are Copy scalars (repr(transparent)) → de-Rc to `Vec<WordN>`.
            | "Word8" | "Word16" | "Word32" | "Word64"
    )
}

fn collect_de_rc_candidates_block(
    stmts: &[Stmt],
    interner: &Interner,
    vec_return_fns: &HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { var, value, .. } => {
                // A fresh-Seq decl, or a binding to a Phase-4 Vec-returning call
                // (`Let r be buildSeq(4)`) — both capture a uniquely-owned value.
                if fresh_scalar_seq_elem(value, interner).is_some()
                    || is_vec_return_call(value, vec_return_fns)
                {
                    out.insert(*var);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_de_rc_candidates_block(then_block, interner, vec_return_fns, out);
                if let Some(eb) = else_block {
                    collect_de_rc_candidates_block(eb, interner, vec_return_fns, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_de_rc_candidates_block(body, interner, vec_return_fns, out);
            }
            Stmt::Zone { body, .. } => {
                collect_de_rc_candidates_block(body, interner, vec_return_fns, out);
            }
            _ => {}
        }
    }
}

/// Collect locally-created Seq/List variables that escape the function body.
/// A variable "escapes" if it is:
/// - Passed to a function call (without `copy of` wrapper)
/// - Returned from the function
/// Variables NOT in the returned set can safely use `Vec<T>` instead of `LogosSeq<T>`.
pub(super) fn collect_escaping_collection_vars(stmts: &[Stmt], interner: &Interner) -> HashSet<Symbol> {
    let mut escaped = HashSet::new();
    collect_escaping_vars_block(stmts, &mut escaped);
    escaped
}

fn collect_escaping_vars_block(stmts: &[Stmt], escaped: &mut HashSet<Symbol>) {
    for stmt in stmts {
        collect_escaping_vars_stmt(stmt, escaped);
    }
}

fn collect_escaping_vars_stmt(stmt: &Stmt, escaped: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Call { args, .. } => {
            for arg in args.iter() {
                collect_escaping_vars_from_call_arg(arg, escaped);
            }
        }
        Stmt::Return { value: Some(expr) } => {
            if let Expr::Identifier(sym) = expr {
                escaped.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            collect_escaping_vars_block(then_block, escaped);
            if let Some(else_stmts) = else_block {
                collect_escaping_vars_block(else_stmts, escaped);
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            collect_escaping_vars_block(body, escaped);
        }
        Stmt::Let { value, .. } => {
            collect_escaping_vars_from_expr_calls(value, escaped);
        }
        Stmt::Set { value, .. } => {
            collect_escaping_vars_from_expr_calls(value, escaped);
        }
        _ => {}
    }
}

/// Check if an expression is a direct call arg (not wrapped in Copy).
fn collect_escaping_vars_from_call_arg(expr: &Expr, escaped: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => { escaped.insert(*sym); }
        Expr::Copy { .. } => { /* copy of X does not cause X to escape */ }
        _ => {
            // For complex expressions, check nested identifiers
            collect_escaping_vars_from_expr_ids(expr, escaped);
        }
    }
}

/// Collect identifiers that appear as direct args in Call expressions within an expression.
fn collect_escaping_vars_from_expr_calls(expr: &Expr, escaped: &mut HashSet<Symbol>) {
    match expr {
        Expr::Call { args, .. } => {
            for arg in args.iter() {
                collect_escaping_vars_from_call_arg(arg, escaped);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_escaping_vars_from_expr_calls(left, escaped);
            collect_escaping_vars_from_expr_calls(right, escaped);
        }
        _ => {}
    }
}

/// Collect direct identifier references from a non-Copy expression used as a call arg.
fn collect_escaping_vars_from_expr_ids(expr: &Expr, escaped: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => { escaped.insert(*sym); }
        Expr::BinaryOp { left, right, .. } => {
            collect_escaping_vars_from_expr_ids(left, escaped);
            collect_escaping_vars_from_expr_ids(right, escaped);
        }
        _ => {}
    }
}

/// Extract a debug prefix string from an expression for `{var=}` format.
pub(super) fn expr_debug_prefix(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        _ => "expr".to_string(),
    }
}

/// Collect identifiers from a statement's expressions (for Concurrent/Parallel variable capture).
pub(super) fn collect_stmt_identifiers(stmt: &Stmt, identifiers: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { value, .. } => {
            collect_expr_identifiers(value, identifiers);
        }
        Stmt::Call { args, .. } => {
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        _ => {}
    }
}

/// Collect parameter indices where any call site passes `Give` as the argument.
/// If a caller writes `Call f with Give x`, index 0 must not be borrowed as &[T].
pub(super) fn collect_give_arg_indices(fn_sym: Symbol, stmts: &[Stmt]) -> HashSet<usize> {
    let mut result = HashSet::new();
    for stmt in stmts {
        scan_give_args_stmt(fn_sym, stmt, &mut result);
    }
    result
}

fn scan_give_args_stmt(fn_sym: Symbol, stmt: &Stmt, result: &mut HashSet<usize>) {
    match stmt {
        Stmt::Call { function, args } => {
            if *function == fn_sym {
                for (i, arg) in args.iter().enumerate() {
                    if matches!(arg, Expr::Give { .. }) {
                        result.insert(i);
                    }
                }
            }
            for arg in args {
                scan_give_args_expr(fn_sym, arg, result);
            }
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
            scan_give_args_expr(fn_sym, value, result);
        }
        Stmt::Return { value: Some(v) } => scan_give_args_expr(fn_sym, v, result),
        Stmt::FunctionDef { body, .. } => {
            for s in *body {
                scan_give_args_stmt(fn_sym, s, result);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                scan_give_args_stmt(fn_sym, s, result);
            }
            if let Some(b) = else_block {
                for s in *b {
                    scan_give_args_stmt(fn_sym, s, result);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                scan_give_args_stmt(fn_sym, s, result);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                scan_give_args_stmt(fn_sym, s, result);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms.iter() {
                for s in arm.body.iter() {
                    scan_give_args_stmt(fn_sym, s, result);
                }
            }
        }
        _ => {}
    }
}

fn scan_give_args_expr(fn_sym: Symbol, expr: &Expr, result: &mut HashSet<usize>) {
    match expr {
        Expr::Call { function, args } => {
            if *function == fn_sym {
                for (i, arg) in args.iter().enumerate() {
                    if matches!(arg, Expr::Give { .. }) {
                        result.insert(i);
                    }
                }
            }
            for arg in args {
                scan_give_args_expr(fn_sym, arg, result);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            scan_give_args_expr(fn_sym, left, result);
            scan_give_args_expr(fn_sym, right, result);
        }
        Expr::FieldAccess { object, .. }
        | Expr::Give { value: object }
        | Expr::Copy { expr: object }
        | Expr::Length { collection: object } => {
            scan_give_args_expr(fn_sym, object, result);
        }
        Expr::Index { collection, index } => {
            scan_give_args_expr(fn_sym, collection, result);
            scan_give_args_expr(fn_sym, index, result);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                scan_give_args_expr(fn_sym, item, result);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Closed-Form Double Recursion Detection
// =============================================================================

/// Result of detecting a closed-form double recursion pattern.
/// For f(0) = base, f(d) = k + f(d-1) + f(d-1), the closed form is:
/// f(d) = ((base + k) << d) - k
pub(super) struct ClosedFormInfo {
    pub base: i64,
    pub k: i64,
}

/// Detect "double recursion with constant addend" pattern:
///   f(0) = base
///   f(d) = k + f(d-1) + f(d-1)
/// where base and k are integer constants. Returns the closed-form parameters.
///
/// GCC/LLVM recognize this pattern and replace it with bit shifts at -O2.
/// This detection lets Logos emit the same closed form.
pub(super) fn detect_double_recursion_closed_form<'a>(
    func_name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: &'a [Stmt<'a>],
    interner: &Interner,
) -> Option<ClosedFormInfo> {
    use crate::ast::stmt::BinaryOpKind;

    if params.len() != 1 {
        return None;
    }
    let param_sym = params[0].0;

    // The closed form emits `((base + k) << d) - k`, which requires the parameter
    // `d` to be an integer (a `<<` on a float operand does not type-check). A bare
    // `0`/`1` base literal parses as an integer regardless of the declared
    // parameter type, so a Float-typed double recursion would otherwise still
    // match and emit `i64 << f64`. Decline anything but an integer parameter.
    let is_integer_param = matches!(
        params[0].1,
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym)
            if matches!(interner.resolve(*sym), "Int" | "Nat" | "Byte")
    );
    if !is_integer_param {
        return None;
    }

    // Body must be exactly: If { param == 0 → Return base }, Return(recursive_expr)
    if body.len() != 2 {
        return None;
    }

    // First statement: base case check
    let base_value = match &body[0] {
        Stmt::If { cond, then_block, else_block } => {
            if else_block.is_some() {
                return None;
            }
            if then_block.len() != 1 {
                return None;
            }
            let base = match &then_block[0] {
                Stmt::Return { value: Some(expr) } => cf_extract_literal_int(expr)?,
                _ => return None,
            };
            match cond {
                Expr::BinaryOp { op: BinaryOpKind::Eq, left, right } => {
                    let ok = (cf_is_identifier(left, param_sym) && cf_is_literal_int(right, 0))
                        || (cf_is_literal_int(left, 0) && cf_is_identifier(right, param_sym));
                    if !ok { return None; }
                }
                _ => return None,
            }
            base
        }
        _ => return None,
    };

    // Second statement: recursive case Return
    let recursive_expr = match &body[1] {
        Stmt::Return { value: Some(expr) } => *expr,
        _ => return None,
    };

    // Flatten the Add tree and analyze: must have exactly 2 self-calls with
    // arg (param - 1), and any number of integer literal addends.
    let mut self_call_count = 0usize;
    let mut constant_sum = 0i64;
    if !cf_analyze_add_tree(recursive_expr, func_name, param_sym, &mut self_call_count, &mut constant_sum) {
        return None;
    }

    if self_call_count != 2 {
        return None;
    }

    Some(ClosedFormInfo { base: base_value, k: constant_sum })
}

fn cf_extract_literal_int(expr: &Expr) -> Option<i64> {
    if let Expr::Literal(crate::ast::stmt::Literal::Number(n)) = expr {
        Some(*n)
    } else {
        None
    }
}

fn cf_is_identifier(expr: &Expr, sym: Symbol) -> bool {
    matches!(expr, Expr::Identifier(s) if *s == sym)
}

fn cf_is_literal_int(expr: &Expr, val: i64) -> bool {
    matches!(expr, Expr::Literal(crate::ast::stmt::Literal::Number(n)) if *n == val)
}

fn cf_is_self_call_with_decrement(expr: &Expr, func_name: Symbol, param_sym: Symbol) -> bool {
    use crate::ast::stmt::BinaryOpKind;
    if let Expr::Call { function, args } = expr {
        if *function != func_name || args.len() != 1 {
            return false;
        }
        if let Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right } = args[0] {
            cf_is_identifier(left, param_sym) && cf_is_literal_int(right, 1)
        } else {
            false
        }
    } else {
        false
    }
}

/// Walk an Add-tree, counting self-calls and summing literal constants.
/// Returns false if any leaf is neither a self-call(param-1) nor an integer literal.
fn cf_analyze_add_tree(
    expr: &Expr,
    func_name: Symbol,
    param_sym: Symbol,
    self_calls: &mut usize,
    constant_sum: &mut i64,
) -> bool {
    use crate::ast::stmt::BinaryOpKind;
    match expr {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            cf_analyze_add_tree(left, func_name, param_sym, self_calls, constant_sum)
                && cf_analyze_add_tree(right, func_name, param_sym, self_calls, constant_sum)
        }
        _ if cf_is_self_call_with_decrement(expr, func_name, param_sym) => {
            *self_calls += 1;
            true
        }
        _ => {
            if let Some(n) = cf_extract_literal_int(expr) {
                *constant_sum += n;
                true
            } else {
                false
            }
        }
    }
}

// =============================================================================
// O3: small fixed-size Seq scalarization (`Seq` → `[T; N]`).
// =============================================================================

/// A `Seq` local proven to have a compile-time-constant size, built by
/// straight-line pushes and thereafter only indexed/length-queried, never
/// resized, aliased, or escaped. Codegen emits it as a Rust `[T; N]` array —
/// C's representation for nbody's bodies: stack-allocated, statically bounded.
pub(crate) struct ScalarizableSeq {
    pub elem_ty: String,
    pub len: usize,
}

struct ScalarCand {
    elem_ty: String,
    len: usize,
    /// A non-push use of the handle has been seen — later pushes disqualify.
    seen_use: bool,
    disqualified: bool,
}

/// `a new Seq of {Int|Nat|Float|Bool}` → the Rust element type, else None.
fn scalar_seq_elem_ty(value: &Expr, interner: &Interner) -> Option<String> {
    if let Expr::New { type_name, type_args, .. } = value {
        if matches!(interner.resolve(*type_name), "Seq" | "List") && type_args.len() == 1 {
            if let TypeExpr::Primitive(t) | TypeExpr::Named(t) = &type_args[0] {
                return match interner.resolve(*t) {
                    "Int" | "Nat" => Some("i64".to_string()),
                    "Float" => Some("f64".to_string()),
                    "Bool" => Some("bool".to_string()),
                    _ => None,
                };
            }
        }
    }
    None
}

/// Maximum scalarized array length (keeps generated arrays small).
const MAX_SCALARIZE_LEN: usize = 64;

/// Find scalarizable Seqs among the given block's locals (v1: the block
/// passed is Main; function bodies are not scanned). Conservative: any
/// appearance of a candidate outside an allowed position disqualifies it.
pub(crate) fn collect_scalarizable_seqs(
    stmts: &[Stmt],
    interner: &Interner,
) -> HashMap<Symbol, ScalarizableSeq> {
    // Kill switch for A/B measurement (`LOGOS_SCALARIZE=0`).
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize) {
        return HashMap::new();
    }
    let mut cand: HashMap<Symbol, ScalarCand> = HashMap::new();
    scalar_walk_block(stmts, true, &mut cand, interner);
    let result: HashMap<Symbol, ScalarizableSeq> = cand
        .into_iter()
        .filter_map(|(sym, c)| {
            if c.disqualified || c.len == 0 || c.len > MAX_SCALARIZE_LEN {
                None
            } else {
                Some((sym, ScalarizableSeq { elem_ty: c.elem_ty, len: c.len }))
            }
        })
        .collect();
    if !result.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Scalarize);
    }
    result
}

/// The set of Seq variables that qualify for fixed-size scalarization (`[T; N]`).
///
/// Exposed to the AOT loop-unroller (`optimize::unroll`): it only unrolls a loop
/// when every collection the loop indexes is one of these. Unrolling a loop over
/// a reference-semantics `LogosSeq` (a push-grown buffer or DP row) would destroy
/// the loop shapes the de-Rc buffer-reuse and borrow-hoist passes pattern-match
/// on — and yields no SROA benefit there anyway, since only scalarized arrays
/// promote to registers.
pub(crate) fn scalarizable_seq_symbols(
    stmts: &[Stmt],
    interner: &Interner,
) -> std::collections::HashSet<Symbol> {
    collect_scalarizable_seqs(stmts, interner)
        .into_keys()
        .collect()
}

fn scalar_disq(cand: &mut HashMap<Symbol, ScalarCand>, sym: Symbol) {
    if let Some(c) = cand.get_mut(&sym) {
        c.disqualified = true;
    }
}

// =============================================================================
// Co-indexed array interleaving (struct-of-arrays → array-of-structs).
// =============================================================================

/// A group of same-type, same-length scalarizable Seqs built by an interleaved
/// (round-robin) push pattern — the columns of an array-of-structs. Codegen
/// fuses them into one `[[T; W]; N]` backing so per-entity fields are adjacent
/// (C's `struct Body[N]` layout), letting LLVM pack them with `movupd` instead
/// of gathering separate arrays with shuffles.
pub(super) struct InterleaveGroup {
    /// Members in column order: column `c` is `members[c]`.
    pub members: Vec<Symbol>,
    /// `N` — the common length (number of round-robin rounds).
    pub len: usize,
    /// The common Rust element type ("f64" / "i64" / "bool").
    pub elem_ty: String,
}

/// Detect co-indexed array groups for AoS interleaving. GENERAL over W (number
/// of co-indexed arrays) and N (length). v1 recognizes a single round-robin
/// cycle that covers all scalarizable pushes, with W >= 2 distinct members of
/// equal element type and length — the canonical AoS-init pattern
/// (`Push x to ax; Push y to ay; …` repeated per entity).
pub(super) fn collect_interleaved_groups(
    stmts: &[Stmt],
    scalarizable: &HashMap<Symbol, ScalarizableSeq>,
    _interner: &Interner,
) -> Vec<InterleaveGroup> {
    // Kill switch for A/B measurement (`LOGOS_AOS=0`).
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Interleave) {
        return Vec::new();
    }
    if scalarizable.len() < 2 {
        return Vec::new();
    }
    // Ordered sequence of top-level pushes targeting scalarizable Seqs (their
    // pushes are straight-line top-level by the scalarization rule).
    let mut push_seq: Vec<Symbol> = Vec::new();
    for s in stmts {
        if let Stmt::Push { collection, .. } = s {
            if let Expr::Identifier(sym) = collection {
                if scalarizable.contains_key(sym) {
                    push_seq.push(*sym);
                }
            }
        }
    }
    if push_seq.len() < 2 {
        return Vec::new();
    }
    // Period W = index of the first repeat of the first pushed array.
    let first = push_seq[0];
    let w = match push_seq.iter().skip(1).position(|&s| s == first) {
        Some(p) => p + 1,
        None => return Vec::new(),
    };
    if w < 2 || push_seq.len() % w != 0 {
        return Vec::new();
    }
    let group: Vec<Symbol> = push_seq[..w].to_vec();
    // The W columns must be distinct symbols.
    let mut seen = std::collections::HashSet::new();
    if !group.iter().all(|s| seen.insert(*s)) {
        return Vec::new();
    }
    // Every round must repeat the same column order exactly.
    let rounds = push_seq.len() / w;
    for r in 0..rounds {
        for c in 0..w {
            if push_seq[r * w + c] != group[c] {
                return Vec::new();
            }
        }
    }
    // The group must cover the whole scalarizable set (no stragglers), and all
    // members must share element type and length (== rounds).
    if group.len() != scalarizable.len() {
        return Vec::new();
    }
    let elem_ty = scalarizable[&group[0]].elem_ty.clone();
    for m in &group {
        let info = &scalarizable[m];
        if info.elem_ty != elem_ty || info.len != rounds {
            return Vec::new();
        }
    }
    // Regime gate: AoS pays off only while the group stays MEMORY-RESIDENT —
    // accessed by variable indices in rolled loops, where adjacent fields load
    // packed (C's `movupd`) and beat SoA's gather. If ANY member is read/written
    // at a CONSTANT literal index, the kernel has been unrolled (or will SROA
    // into registers), where the layout is moot and SoA+unroll wins outright —
    // AoS there only perturbs LLVM's SLP for the worse (measured: nbody
    // 1.12×→1.19×). Leave those to the unroller.
    // `LOGOS_AOS_FORCE=1` bypasses the regime gate for A/B measurement.
    let force = std::env::var("LOGOS_AOS_FORCE").map(|v| v == "1").unwrap_or(false);
    let members: std::collections::HashSet<Symbol> = group.iter().copied().collect();
    if !force && block_has_const_member_index(stmts, &members) {
        // A const-index access means the kernel has been unrolled / will SROA into
        // registers — Unroll/Scalarize claim it, preempting AoS interleaving.
        crate::optimize::mark_preempted(
            crate::optimization::Opt::Scalarize,
            crate::optimization::Opt::Interleave,
        );
        crate::optimize::mark_preempted(
            crate::optimization::Opt::Unroll,
            crate::optimization::Opt::Interleave,
        );
        return Vec::new();
    }
    crate::optimize::mark_fired(crate::optimization::Opt::Interleave);
    vec![InterleaveGroup { members: group, len: rounds, elem_ty }]
}

/// True if any member of `members` is indexed by a constant literal anywhere in
/// `stmts` (evidence the kernel unrolled / will SROA — the register regime).
fn block_has_const_member_index(stmts: &[Stmt], members: &std::collections::HashSet<Symbol>) -> bool {
    stmts.iter().any(|s| stmt_has_const_member_index(s, members))
}

fn is_member_const_index(collection: &Expr, index: &Expr, members: &std::collections::HashSet<Symbol>) -> bool {
    matches!(collection, Expr::Identifier(s) if members.contains(s))
        && matches!(index, Expr::Literal(Literal::Number(_)))
}

fn stmt_has_const_member_index(s: &Stmt, members: &std::collections::HashSet<Symbol>) -> bool {
    match s {
        Stmt::SetIndex { collection, index, value } => {
            is_member_const_index(collection, index, members)
                || expr_has_const_member_index(collection, members)
                || expr_has_const_member_index(index, members)
                || expr_has_const_member_index(value, members)
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_has_const_member_index(value, members),
        Stmt::Push { value, collection } => {
            expr_has_const_member_index(value, members) || expr_has_const_member_index(collection, members)
        }
        Stmt::Show { object, .. } => expr_has_const_member_index(object, members),
        Stmt::SetField { object, value, .. } => {
            expr_has_const_member_index(object, members) || expr_has_const_member_index(value, members)
        }
        Stmt::Give { object, recipient } => {
            expr_has_const_member_index(object, members) || expr_has_const_member_index(recipient, members)
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            expr_has_const_member_index(value, members) || expr_has_const_member_index(collection, members)
        }
        Stmt::RuntimeAssert { condition, .. } => expr_has_const_member_index(condition, members),
        Stmt::Return { value } => matches!(value, Some(v) if expr_has_const_member_index(v, members)),
        Stmt::Call { args, .. } => args.iter().any(|a| expr_has_const_member_index(a, members)),
        Stmt::If { cond, then_block, else_block } => {
            expr_has_const_member_index(cond, members)
                || block_has_const_member_index(then_block, members)
                || matches!(else_block, Some(eb) if block_has_const_member_index(eb, members))
        }
        Stmt::While { cond, body, .. } => {
            expr_has_const_member_index(cond, members) || block_has_const_member_index(body, members)
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_has_const_member_index(iterable, members) || block_has_const_member_index(body, members)
        }
        Stmt::Inspect { target, arms, .. } => {
            expr_has_const_member_index(target, members)
                || arms.iter().any(|a| block_has_const_member_index(a.body, members))
        }
        _ => false,
    }
}

fn expr_has_const_member_index(e: &Expr, members: &std::collections::HashSet<Symbol>) -> bool {
    match e {
        Expr::Index { collection, index } => {
            is_member_const_index(collection, index, members)
                || expr_has_const_member_index(collection, members)
                || expr_has_const_member_index(index, members)
        }
        Expr::Slice { collection, start, end } => {
            expr_has_const_member_index(collection, members)
                || expr_has_const_member_index(start, members)
                || expr_has_const_member_index(end, members)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_has_const_member_index(left, members) || expr_has_const_member_index(right, members)
        }
        Expr::Not { operand } => expr_has_const_member_index(operand, members),
        Expr::Length { collection } => expr_has_const_member_index(collection, members),
        Expr::Contains { collection, value } => {
            expr_has_const_member_index(collection, members) || expr_has_const_member_index(value, members)
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            expr_has_const_member_index(left, members) || expr_has_const_member_index(right, members)
        }
        Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
            args.iter().any(|a| expr_has_const_member_index(a, members))
        }
        Expr::FieldAccess { object, .. } => expr_has_const_member_index(object, members),
        Expr::Copy { expr } => expr_has_const_member_index(expr, members),
        Expr::Give { value } | Expr::OptionSome { value } => expr_has_const_member_index(value, members),
        Expr::WithCapacity { value, capacity } => {
            expr_has_const_member_index(value, members) || expr_has_const_member_index(capacity, members)
        }
        Expr::Range { start, end } => {
            expr_has_const_member_index(start, members) || expr_has_const_member_index(end, members)
        }
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| expr_has_const_member_index(i, members))
        }
        Expr::InterpolatedString(parts) => parts.iter().any(|p| match p {
            crate::ast::stmt::StringPart::Expr { value, .. } => expr_has_const_member_index(value, members),
            _ => false,
        }),
        _ => false,
    }
}

/// A parsed `__aos:<backing>:<col>:<width>:<len>:<elem>` member type tag — one
/// column of a fused array-of-structs. `item i of member` lowers to
/// `backing[(i-1)][col]`; column 0 emits the `[[elem; width]; len]` backing.
pub(super) struct AosTag {
    pub backing: String,
    pub col: usize,
    pub width: usize,
    pub len: usize,
    pub elem: String,
}

/// Parse an AoS member type tag (registered in `codegen_program`). The backing
/// name is the first member's emitted identifier; it contains no `:`, and the
/// element type (`f64`/`i64`/`bool`) contains none either, so `:`-splitting is
/// unambiguous.
pub(super) fn parse_aos_tag(ty: Option<&String>) -> Option<AosTag> {
    let rest = ty?.strip_prefix("__aos:")?;
    let mut parts = rest.split(':');
    let backing = parts.next()?.to_string();
    let col = parts.next()?.parse().ok()?;
    let width = parts.next()?.parse().ok()?;
    let len = parts.next()?.parse().ok()?;
    let elem = parts.next()?.to_string();
    Some(AosTag { backing, col, width, len, elem })
}

fn scalar_mark_use(cand: &mut HashMap<Symbol, ScalarCand>, sym: Symbol) {
    if let Some(c) = cand.get_mut(&sym) {
        c.seen_use = true;
    }
}

/// A collection-position expression: a bare candidate handle is an ALLOWED
/// access (read/length); anything else is a value.
fn scalar_note_access(e: &Expr, cand: &mut HashMap<Symbol, ScalarCand>) {
    if let Expr::Identifier(s) = e {
        scalar_mark_use(cand, *s);
    } else {
        scalar_note_value(e, cand);
    }
}

/// A value-position expression: a bare candidate handle DISQUALIFIES (the
/// handle itself flows somewhere). `item i of x` and `length of x` remain
/// allowed accesses — their results are scalars.
fn scalar_note_value(e: &Expr, cand: &mut HashMap<Symbol, ScalarCand>) {
    match e {
        Expr::Identifier(s) => scalar_disq(cand, *s),
        Expr::Literal(_) | Expr::OptionNone => {}
        Expr::Index { collection, index } => {
            scalar_note_access(collection, cand);
            scalar_note_value(index, cand);
        }
        Expr::Length { collection } => scalar_note_access(collection, cand),
        Expr::Slice { collection, start, end } => {
            // A slice yields a new Seq — the handle escapes into it.
            if let Expr::Identifier(s) = collection {
                scalar_disq(cand, *s);
            } else {
                scalar_note_value(collection, cand);
            }
            scalar_note_value(start, cand);
            scalar_note_value(end, cand);
        }
        Expr::Contains { collection, value } => {
            if let Expr::Identifier(s) = collection {
                scalar_disq(cand, *s);
            } else {
                scalar_note_value(collection, cand);
            }
            scalar_note_value(value, cand);
        }
        Expr::Copy { expr } => {
            if let Expr::Identifier(s) = expr {
                scalar_disq(cand, *s);
            } else {
                scalar_note_value(expr, cand);
            }
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            scalar_note_value(left, cand);
            scalar_note_value(right, cand);
        }
        Expr::Not { operand } => scalar_note_value(operand, cand),
        Expr::FieldAccess { object, .. } => scalar_note_value(object, cand),
        Expr::OptionSome { value } | Expr::Give { value } => scalar_note_value(value, cand),
        Expr::WithCapacity { value, capacity } => {
            scalar_note_value(value, cand);
            scalar_note_value(capacity, cand);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for it in items {
                scalar_note_value(it, cand);
            }
        }
        Expr::New { init_fields, .. } => {
            for (_, v) in init_fields {
                scalar_note_value(v, cand);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, v) in fields {
                scalar_note_value(v, cand);
            }
        }
        Expr::Call { args, .. } => {
            for a in args {
                scalar_note_value(a, cand);
            }
        }
        Expr::CallExpr { callee, args } => {
            scalar_note_value(callee, cand);
            for a in args {
                scalar_note_value(a, cand);
            }
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    scalar_note_value(value, cand);
                }
            }
        }
        Expr::ManifestOf { zone } => scalar_note_value(zone, cand),
        Expr::ChunkAt { index, zone } => {
            scalar_note_value(index, cand);
            scalar_note_value(zone, cand);
        }
        // Closures and escape hatches are opaque — they may capture or
        // reference any handle in scope. Conservatively disqualify every
        // candidate rather than risk scalarizing a captured Seq.
        Expr::Closure { .. } | Expr::Escape { .. } => {
            for c in cand.values_mut() {
                c.disqualified = true;
            }
        }
        _ => {}
    }
}

fn scalar_walk_block(
    stmts: &[Stmt],
    top_level: bool,
    cand: &mut HashMap<Symbol, ScalarCand>,
    interner: &Interner,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { var, value, mutable, .. } => {
                if top_level && *mutable {
                    if let Some(elem) = scalar_seq_elem_ty(value, interner) {
                        cand.insert(
                            *var,
                            ScalarCand { elem_ty: elem, len: 0, seen_use: false, disqualified: false },
                        );
                        continue;
                    }
                }
                // `Let y be x` aliases x; any candidate in the value escapes.
                if let Expr::Identifier(s) = value {
                    scalar_disq(cand, *s);
                } else {
                    scalar_note_value(value, cand);
                }
            }
            Stmt::Push { value, collection } => {
                // The pushed value may carry a candidate handle (it escapes).
                if let Expr::Identifier(s) = value {
                    scalar_disq(cand, *s);
                } else {
                    scalar_note_value(value, cand);
                }
                if let Expr::Identifier(x) = collection {
                    let nested_or_used = {
                        match cand.get(x) {
                            Some(c) => !top_level || c.seen_use || c.disqualified,
                            None => false,
                        }
                    };
                    if cand.contains_key(x) {
                        if nested_or_used {
                            scalar_disq(cand, *x);
                        } else if let Some(c) = cand.get_mut(x) {
                            c.len += 1;
                        }
                    }
                } else {
                    scalar_note_value(collection, cand);
                }
            }
            Stmt::SetIndex { collection, index, value } => {
                if let Expr::Identifier(x) = collection {
                    scalar_mark_use(cand, *x);
                } else {
                    scalar_note_value(collection, cand);
                }
                scalar_note_value(index, cand);
                scalar_note_value(value, cand);
            }
            Stmt::Set { target, value } => {
                // Rebinding a candidate disqualifies it; aliasing escapes the RHS.
                scalar_disq(cand, *target);
                if let Expr::Identifier(s) = value {
                    scalar_disq(cand, *s);
                } else {
                    scalar_note_value(value, cand);
                }
            }
            Stmt::Pop { collection, .. } => {
                if let Expr::Identifier(x) = collection {
                    scalar_disq(cand, *x);
                } else {
                    scalar_note_value(collection, cand);
                }
            }
            Stmt::Add { collection, value } | Stmt::Remove { collection, value } => {
                if let Expr::Identifier(x) = collection {
                    scalar_disq(cand, *x);
                } else {
                    scalar_note_value(collection, cand);
                }
                scalar_note_value(value, cand);
            }
            Stmt::Show { object, recipient } => {
                scalar_note_value(object, cand);
                scalar_note_value(recipient, cand);
            }
            Stmt::Give { object, recipient } => {
                if let Expr::Identifier(s) = object {
                    scalar_disq(cand, *s);
                } else {
                    scalar_note_value(object, cand);
                }
                scalar_note_value(recipient, cand);
            }
            Stmt::Return { value: Some(v) } => {
                if let Expr::Identifier(s) = v {
                    scalar_disq(cand, *s);
                } else {
                    scalar_note_value(v, cand);
                }
            }
            Stmt::SetField { object, value, .. } => {
                scalar_note_value(object, cand);
                scalar_note_value(value, cand);
            }
            Stmt::If { cond, then_block, else_block } => {
                scalar_note_value(cond, cand);
                scalar_walk_block(then_block, false, cand, interner);
                if let Some(eb) = else_block {
                    scalar_walk_block(eb, false, cand, interner);
                }
            }
            Stmt::While { cond, body, .. } => {
                scalar_note_value(cond, cand);
                scalar_walk_block(body, false, cand, interner);
            }
            Stmt::Repeat { iterable, body, .. } => {
                if let Expr::Identifier(x) = iterable {
                    scalar_disq(cand, *x);
                } else {
                    scalar_note_value(iterable, cand);
                }
                scalar_walk_block(body, false, cand, interner);
            }
            Stmt::Inspect { target, arms, .. } => {
                scalar_note_value(target, cand);
                for arm in arms {
                    scalar_walk_block(&arm.body, false, cand, interner);
                }
            }
            Stmt::RuntimeAssert { condition, .. } => scalar_note_value(condition, cand),
            Stmt::Call { args, .. } => {
                for a in args {
                    if let Expr::Identifier(s) = a {
                        scalar_disq(cand, *s);
                    } else {
                        scalar_note_value(a, cand);
                    }
                }
            }
            Stmt::Zone { body, .. } => scalar_walk_block(body, false, cand, interner),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                scalar_walk_block(tasks, false, cand, interner)
            }
            // FunctionDef and other forms: candidates are Main-locals; a
            // function body cannot reference them. Nothing to do.
            _ => {}
        }
    }
}
