use std::collections::{HashMap, HashSet};

use crate::analysis::registry::{FieldType, TypeDef, TypeRegistry};
use crate::ast::stmt::{Expr, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

use super::is_recursive_field;

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
        Stmt::RuntimeAssert { condition } => calls_async_function_in_expr(condition, async_fns),
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

// =============================================================================
// Inline Annotation Detection
// =============================================================================

pub(super) fn should_inline(name: Symbol, body: &[Stmt], is_native: bool, is_exported: bool, is_async: bool) -> bool {
    !is_native && !is_exported && !is_async
        && body.len() <= 5
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
        Expr::Copy { expr: inner } | Expr::Give { value: inner } | Expr::Length { collection: inner } => {
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
