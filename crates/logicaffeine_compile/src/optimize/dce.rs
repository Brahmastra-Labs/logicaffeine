use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Literal, MatchArm, Stmt, StringPart, ClosureBody};
use crate::intern::Symbol;

/// Collect all symbols read by an expression.
/// Returns false if the expression contains opaque parts (Escape, Call, CallExpr)
/// whose reads cannot be fully determined.
fn collect_expr_reads(expr: &Expr, reads: &mut HashSet<Symbol>) -> bool {
    match expr {
        Expr::Identifier(sym) => { reads.insert(*sym); true }
        Expr::Literal(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. } => {
            let a = collect_expr_reads(left, reads);
            let b = collect_expr_reads(right, reads);
            a && b
        }
        Expr::Length { collection } | Expr::Not { operand: collection }
        | Expr::Copy { expr: collection } | Expr::Give { value: collection }
        | Expr::OptionSome { value: collection } | Expr::ManifestOf { zone: collection } => {
            collect_expr_reads(collection, reads)
        }
        Expr::Index { collection, index } | Expr::Contains { collection, value: index }
        | Expr::Union { left: collection, right: index }
        | Expr::Intersection { left: collection, right: index }
        | Expr::Range { start: collection, end: index }
        | Expr::WithCapacity { value: collection, capacity: index }
        | Expr::ChunkAt { index, zone: collection } => {
            let a = collect_expr_reads(collection, reads);
            let b = collect_expr_reads(index, reads);
            a && b
        }
        Expr::Slice { collection, start, end } => {
            let a = collect_expr_reads(collection, reads);
            let b = collect_expr_reads(start, reads);
            let c = collect_expr_reads(end, reads);
            a && b && c
        }
        Expr::FieldAccess { object, .. } => collect_expr_reads(object, reads),
        Expr::List(items) | Expr::Tuple(items) => {
            let mut ok = true;
            for item in items { ok &= collect_expr_reads(item, reads); }
            ok
        }
        Expr::New { init_fields, .. } => {
            let mut ok = true;
            for (_, val) in init_fields { ok &= collect_expr_reads(val, reads); }
            ok
        }
        Expr::NewVariant { fields, .. } => {
            let mut ok = true;
            for (_, val) in fields { ok &= collect_expr_reads(val, reads); }
            ok
        }
        Expr::InterpolatedString(parts) => {
            let mut ok = true;
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    ok &= collect_expr_reads(value, reads);
                }
            }
            ok
        }
        // Opaque expressions: may read anything
        Expr::Call { args, .. } => {
            for arg in args { collect_expr_reads(arg, reads); }
            false
        }
        Expr::CallExpr { callee, args } => {
            collect_expr_reads(callee, reads);
            for arg in args { collect_expr_reads(arg, reads); }
            false
        }
        Expr::Closure { .. } | Expr::Escape { .. } => false,
        Expr::UnaryOp { .. } => {
            unreachable!("HW-Spec UnaryOp not emitted outside ## Hardware/Property blocks (in DCE read collection)")
        }
        Expr::BitSelect { .. } => {
            unreachable!("HW-Spec BitSelect not emitted outside ## Hardware/Property blocks (in DCE read collection)")
        }
        Expr::PartSelect { .. } => {
            unreachable!("HW-Spec PartSelect not emitted outside ## Hardware/Property blocks (in DCE read collection)")
        }
        Expr::HwConcat { .. } => {
            unreachable!("HW-Spec HwConcat not emitted outside ## Hardware/Property blocks (in DCE read collection)")
        }
    }
}

/// Collect all symbols read by a statement (recursively into sub-blocks).
fn collect_stmt_reads(stmt: &Stmt, reads: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { value, .. } => { collect_expr_reads(value, reads); }
        Stmt::Set { value, .. } => { collect_expr_reads(value, reads); }
        Stmt::Show { object, recipient } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(recipient, reads);
        }
        Stmt::Push { collection, value } | Stmt::Add { collection, value }
        | Stmt::Remove { collection, value } => {
            collect_expr_reads(collection, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::Pop { collection, .. } => { collect_expr_reads(collection, reads); }
        Stmt::SetIndex { collection, index, value } => {
            collect_expr_reads(collection, reads);
            collect_expr_reads(index, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::SetField { object, value, .. } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::Return { value } => {
            if let Some(v) = value { collect_expr_reads(v, reads); }
        }
        Stmt::Call { args, .. } => {
            for arg in args { collect_expr_reads(arg, reads); }
        }
        Stmt::If { cond, then_block, else_block } => {
            collect_expr_reads(cond, reads);
            for s in then_block.iter() { collect_stmt_reads(s, reads); }
            if let Some(eb) = else_block {
                for s in eb.iter() { collect_stmt_reads(s, reads); }
            }
        }
        Stmt::While { cond, body, decreasing } => {
            collect_expr_reads(cond, reads);
            for s in body.iter() { collect_stmt_reads(s, reads); }
            if let Some(d) = decreasing { collect_expr_reads(d, reads); }
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_expr_reads(iterable, reads);
            for s in body.iter() { collect_stmt_reads(s, reads); }
        }
        Stmt::RuntimeAssert { condition } | Stmt::Listen { address: condition }
        | Stmt::ConnectTo { address: condition } | Stmt::Sleep { milliseconds: condition }
        | Stmt::StopTask { handle: condition } => {
            collect_expr_reads(condition, reads);
        }
        Stmt::Give { object, recipient } | Stmt::MergeCrdt { source: object, target: recipient }
        | Stmt::SendMessage { message: object, destination: recipient }
        | Stmt::WriteFile { content: object, path: recipient }
        | Stmt::SendPipe { value: object, pipe: recipient }
        | Stmt::TrySendPipe { value: object, pipe: recipient, .. }
        | Stmt::AppendToSequence { sequence: recipient, value: object } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(recipient, reads);
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(amount, reads);
        }
        Stmt::ResolveConflict { object, value, .. } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::Inspect { target, arms, .. } => {
            collect_expr_reads(target, reads);
            for arm in arms {
                for s in arm.body.iter() { collect_stmt_reads(s, reads); }
            }
        }
        Stmt::Zone { body, .. } | Stmt::Concurrent { tasks: body } | Stmt::Parallel { tasks: body } => {
            for s in body.iter() { collect_stmt_reads(s, reads); }
        }
        Stmt::FunctionDef { .. } | Stmt::StructDef { .. } | Stmt::Theorem(..)
        | Stmt::Escape { .. } | Stmt::Require { .. } | Stmt::Break
        | Stmt::Assert { .. } | Stmt::Trust { .. } | Stmt::Check { .. }
        | Stmt::Spawn { .. } | Stmt::CreatePipe { .. } | Stmt::Mount { .. }
        | Stmt::ReadFrom { .. } | Stmt::LetPeerAgent { .. } | Stmt::Sync { .. } => {}
        Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
            for arg in args { collect_expr_reads(arg, reads); }
        }
        Stmt::AwaitMessage { source, .. } | Stmt::ReceivePipe { pipe: source, .. }
        | Stmt::TryReceivePipe { pipe: source, .. } => {
            collect_expr_reads(source, reads);
        }
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    crate::ast::stmt::SelectBranch::Receive { pipe, body, .. } => {
                        collect_expr_reads(pipe, reads);
                        for s in body.iter() { collect_stmt_reads(s, reads); }
                    }
                    crate::ast::stmt::SelectBranch::Timeout { milliseconds, body } => {
                        collect_expr_reads(milliseconds, reads);
                        for s in body.iter() { collect_stmt_reads(s, reads); }
                    }
                }
            }
        }
    }
}

/// Dead Store Elimination: remove Set statements that are overwritten before being read.
fn dead_store_elimination<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Vec<Stmt<'a>> {
    // Track: symbol → index of last Set (not Let — Let+Set merge conflicts with codegen peepholes)
    let mut last_set: HashMap<Symbol, usize> = HashMap::new();
    let mut dead_indices: HashSet<usize> = HashSet::new();

    for (i, stmt) in stmts.iter().enumerate() {
        // 1. Collect reads from this statement
        let mut reads = HashSet::new();
        collect_stmt_reads(stmt, &mut reads);

        // 2. Mark read variables as alive (previous Set is needed)
        for sym in &reads {
            last_set.remove(sym);
        }

        // 3. Process writes
        match stmt {
            Stmt::Set { target, value } => {
                // Only eliminate if the Set's value is fully analyzable
                let mut value_reads = HashSet::new();
                let value_analyzable = collect_expr_reads(value, &mut value_reads);

                if value_analyzable {
                    if let Some(prev_idx) = last_set.remove(target) {
                        // Previous Set was never read → dead store
                        dead_indices.insert(prev_idx);
                    }
                    last_set.insert(*target, i);
                } else {
                    // Opaque value: conservatively clear all tracking
                    last_set.clear();
                }
            }
            // Control flow: conservatively clear tracking
            Stmt::If { .. } | Stmt::While { .. } | Stmt::Repeat { .. }
            | Stmt::Inspect { .. } | Stmt::Zone { .. } | Stmt::Select { .. }
            | Stmt::Concurrent { .. } | Stmt::Parallel { .. } => {
                last_set.clear();
            }
            // Function calls may read/write anything via side effects
            Stmt::Call { .. } | Stmt::Escape { .. } => {
                last_set.clear();
            }
            _ => {}
        }
    }

    // Rebuild with dead Sets removed
    let mut result = Vec::with_capacity(stmts.len());
    for (i, stmt) in stmts.into_iter().enumerate() {
        if dead_indices.contains(&i) {
            continue;
        }
        result.push(stmt);
    }
    result
}

pub fn eliminate_dead_code<'a>(
    stmts: Vec<Stmt<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Vec<Stmt<'a>> {
    // First: dead store elimination
    let stmts = dead_store_elimination(stmts, expr_arena);
    // Then: dead branch elimination
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::If { cond, then_block, else_block } => {
                if let Expr::Literal(Literal::Boolean(val)) = cond {
                    if *val {
                        result.extend(then_block.iter().cloned());
                    } else if let Some(else_stmts) = else_block {
                        result.extend(else_stmts.iter().cloned());
                    }
                } else {
                    let then_dce = dce_block(then_block, stmt_arena, expr_arena);
                    let else_dce = else_block.map(|b| dce_block(b, stmt_arena, expr_arena));
                    result.push(Stmt::If { cond, then_block: then_dce, else_block: else_dce });
                }
            }
            Stmt::While { cond, body, decreasing } => {
                if let Expr::Literal(Literal::Boolean(false)) = cond {
                    // While false: ... is dead code — eliminate entirely
                } else {
                    result.push(Stmt::While {
                        cond,
                        body: dce_block(body, stmt_arena, expr_arena),
                        decreasing,
                    });
                }
            }
            Stmt::Repeat { pattern, iterable, body } => {
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: dce_block(body, stmt_arena, expr_arena),
                });
            }
            Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
                result.push(Stmt::FunctionDef {
                    name,
                    params,
                    generics,
                    body: dce_block(body, stmt_arena, expr_arena),
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                });
            }
            Stmt::Zone { name, capacity, source_file, body } => {
                result.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: dce_block(body, stmt_arena, expr_arena),
                });
            }
            Stmt::Concurrent { tasks } => {
                result.push(Stmt::Concurrent {
                    tasks: dce_block(tasks, stmt_arena, expr_arena),
                });
            }
            Stmt::Parallel { tasks } => {
                result.push(Stmt::Parallel {
                    tasks: dce_block(tasks, stmt_arena, expr_arena),
                });
            }
            Stmt::Inspect { target, arms, has_otherwise } => {
                let arms_dce: Vec<MatchArm<'a>> = arms.into_iter().map(|arm| {
                    MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings,
                        body: dce_block(arm.body, stmt_arena, expr_arena),
                    }
                }).collect();
                result.push(Stmt::Inspect { target, arms: arms_dce, has_otherwise });
            }
            other => result.push(other),
        }
    }

    if let Some(pos) = result.iter().position(|s| matches!(s, Stmt::Return { .. })) {
        result.truncate(pos + 1);
    }

    dead_variable_elimination(result)
}

fn dce_block<'a>(block: &'a [Stmt<'a>], stmt_arena: &'a Arena<Stmt<'a>>, expr_arena: &'a Arena<Expr<'a>>) -> &'a [Stmt<'a>] {
    let stmts: Vec<Stmt<'a>> = block.iter().cloned().collect();
    let dce_result = eliminate_dead_code(stmts, stmt_arena, expr_arena);
    stmt_arena.alloc_slice(dce_result)
}

fn dead_variable_elimination<'a>(stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
    stmts
}
