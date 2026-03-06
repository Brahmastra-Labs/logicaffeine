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

fn symbol_appears_in_stmt(sym: Symbol, stmt: &Stmt) -> bool {
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
        Stmt::RuntimeAssert { condition } => symbol_appears_in_expr(sym, condition),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| symbol_appears_in_stmt(sym, s)),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            tasks.iter().any(|s| symbol_appears_in_stmt(sym, s))
        }
        _ => false,
    }
}

fn symbol_appears_in_expr(sym: Symbol, expr: &Expr) -> bool {
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
pub(super) fn is_vec_type_expr(ty: &TypeExpr, interner: &Interner) -> bool {
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

/// Convert a Vec<T> type string to a slice &[T] type string.
/// E.g., "Vec<i64>" → "&[i64]", "Vec<String>" → "&[String]"
pub(super) fn vec_to_slice_type(vec_type: &str) -> String {
    if let Some(inner) = vec_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        format!("&[{}]", inner)
    } else {
        vec_type.to_string()
    }
}

/// Convert `Vec<T>` to `&mut [T]` for mutable borrow parameters.
pub(super) fn vec_to_mut_slice_type(vec_type: &str) -> String {
    if let Some(inner) = vec_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        format!("&mut [{}]", inner)
    } else {
        vec_type.to_string()
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
