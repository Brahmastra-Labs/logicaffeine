//! Deforestation (Stream Fusion).
//!
//! Detects producer-consumer loop chains over intermediate collections and fuses
//! them into a single loop, eliminating intermediate allocations.
//!
//! Pattern:
//!   Let mutable temp = new Seq of T.
//!   Repeat for x in source: ... Push <expr> to temp ...
//!   Repeat for y in temp: <body>
//!
//! Becomes:
//!   Repeat for x in source: ... Let y = <expr>. <body> ...

use std::collections::HashSet;

use crate::arena::Arena;
use crate::ast::stmt::{Block, Expr, Pattern, StringPart, Stmt};
use crate::intern::{Interner, Symbol};

pub fn deforest_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    // Run iteratively for multi-stage fusion (A→B→C fuses in 2 passes)
    let mut current = stmts;
    for _ in 0..4 {
        let prev_len = current.len();
        current = deforest_pass(current, expr_arena, stmt_arena, interner);
        if current.len() >= prev_len {
            break;
        }
    }
    current
}

fn deforest_pass<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());
    let mut i = 0;

    while i < stmts.len() {
        if let Some((skip_to, fused)) = try_deforest(&stmts, i, expr_arena, stmt_arena, interner) {
            result.extend(fused);
            i = skip_to;
            continue;
        }

        let stmt = stmts[i].clone();
        result.push(recurse_deforest(stmt, expr_arena, stmt_arena, interner));
        i += 1;
    }

    result
}

fn recurse_deforest<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Stmt<'a> {
    match stmt {
        Stmt::FunctionDef { name, generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => {
            let new_body = deforest_stmts(body.to_vec(), expr_arena, stmt_arena, interner);
            Stmt::FunctionDef {
                name, generics, params,
                body: stmt_arena.alloc_slice(new_body),
                return_type, is_native, native_path, is_exported, export_target, opt_flags,
            }
        }
        Stmt::While { cond, body, decreasing } => {
            let new_body = deforest_stmts(body.to_vec(), expr_arena, stmt_arena, interner);
            Stmt::While {
                cond,
                body: stmt_arena.alloc_slice(new_body),
                decreasing,
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let new_then = deforest_stmts(then_block.to_vec(), expr_arena, stmt_arena, interner);
            let new_else = else_block.map(|eb| {
                let processed = deforest_stmts(eb.to_vec(), expr_arena, stmt_arena, interner);
                let b: Block = stmt_arena.alloc_slice(processed);
                b
            });
            Stmt::If {
                cond,
                then_block: stmt_arena.alloc_slice(new_then),
                else_block: new_else,
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            let new_body = deforest_stmts(body.to_vec(), expr_arena, stmt_arena, interner);
            Stmt::Repeat {
                pattern,
                iterable,
                body: stmt_arena.alloc_slice(new_body),
            }
        }
        other => other,
    }
}

fn try_deforest<'a>(
    stmts: &[Stmt<'a>],
    start: usize,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Option<(usize, Vec<Stmt<'a>>)> {
    // Step 1: stmts[start] must be `Let mutable temp = new Seq/List`
    let temp_sym = match_temp_init(&stmts[start], interner)?;

    // Step 2: stmts[start+1] must be a Repeat that pushes to temp
    let producer_idx = start + 1;
    if producer_idx >= stmts.len() { return None; }
    if !is_repeat_producer(&stmts[producer_idx], temp_sym) { return None; }

    // Step 3: Find the consumer Repeat over temp, collecting intermediates
    let mut consumer_idx = None;
    let mut intermediates = Vec::new();
    for j in (producer_idx + 1)..stmts.len() {
        if is_repeat_over_sym(&stmts[j], temp_sym) {
            consumer_idx = Some(j);
            break;
        }
        if stmt_references_symbol(&stmts[j], temp_sym) {
            return None;
        }
        if !is_safe_intermediate(&stmts[j]) {
            return None;
        }
        intermediates.push(stmts[j].clone());
    }
    let consumer_idx = consumer_idx?;

    // Step 4: Verify temp is not referenced after the consumer
    for j in (consumer_idx + 1)..stmts.len() {
        if stmt_references_symbol(&stmts[j], temp_sym) {
            return None;
        }
    }

    // Step 5: Extract consumer details
    let (pattern_var, consumer_body) = match &stmts[consumer_idx] {
        Stmt::Repeat { pattern: Pattern::Identifier(var), body, .. } => (*var, *body),
        _ => return None,
    };

    // Step 6: Safety — consumer body must not write to producer's source symbols
    let producer_source = collect_iterable_symbols(&stmts[producer_idx]);
    let consumer_writes = collect_block_writes(consumer_body);
    if !consumer_writes.is_disjoint(&producer_source) {
        return None;
    }

    // Step 7: Build the fused loop
    let fused = build_fused_repeat(
        &stmts[producer_idx], temp_sym, pattern_var, consumer_body,
        expr_arena, stmt_arena,
    )?;

    // Verify no residual pushes to temp remain
    if fused_still_pushes_to(&fused, temp_sym) {
        return None;
    }

    // Output: intermediates first, then fused loop
    let mut output = intermediates;
    output.push(fused);

    Some((consumer_idx + 1, output))
}

/// Check if stmt is `Let mutable <var> = new Seq/List`
fn match_temp_init(stmt: &Stmt, interner: &Interner) -> Option<Symbol> {
    match stmt {
        Stmt::Let { var, value, mutable: true, .. } => {
            if let Expr::New { type_name, .. } = value {
                let name = interner.resolve(*type_name);
                if name == "Seq" || name == "List" {
                    return Some(*var);
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if stmt is a Repeat loop whose body pushes to `temp_sym`
fn is_repeat_producer(stmt: &Stmt, temp_sym: Symbol) -> bool {
    match stmt {
        Stmt::Repeat { body, .. } => block_pushes_to(body, temp_sym),
        _ => false,
    }
}

/// Check if a block contains at least one Push to the given symbol
fn block_pushes_to(block: &[Stmt], sym: Symbol) -> bool {
    for stmt in block {
        match stmt {
            Stmt::Push { collection, .. } => {
                if let Expr::Identifier(s) = collection {
                    if *s == sym { return true; }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if block_pushes_to(then_block, sym) { return true; }
                if let Some(eb) = else_block {
                    if block_pushes_to(eb, sym) { return true; }
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if stmt is `Repeat for <var> in <sym>`
fn is_repeat_over_sym(stmt: &Stmt, sym: Symbol) -> bool {
    match stmt {
        Stmt::Repeat { pattern: Pattern::Identifier(_), iterable, .. } => {
            matches!(iterable, Expr::Identifier(s) if *s == sym)
        }
        _ => false,
    }
}

/// Check if an intermediate statement is safe to move before the fused loop.
/// Only allow Let bindings (new variables, no mutation of existing ones).
fn is_safe_intermediate(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Let { .. })
}

/// Collect symbols from the producer's iterable expression.
fn collect_iterable_symbols(stmt: &Stmt) -> HashSet<Symbol> {
    let mut syms = HashSet::new();
    if let Stmt::Repeat { iterable, .. } = stmt {
        collect_expr_symbols(iterable, &mut syms);
    }
    syms
}

/// Collect all symbols written (Set targets, Push collections) in a block.
fn collect_block_writes(block: &[Stmt]) -> HashSet<Symbol> {
    let mut writes = HashSet::new();
    for stmt in block {
        collect_stmt_writes(stmt, &mut writes);
    }
    writes
}

fn collect_stmt_writes(stmt: &Stmt, writes: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => { writes.insert(*target); }
        Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
        | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                writes.insert(*sym);
            }
        }
        Stmt::SetIndex { collection, .. } | Stmt::SetField { object: collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                writes.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in then_block.iter() { collect_stmt_writes(s, writes); }
            if let Some(eb) = else_block {
                for s in eb.iter() { collect_stmt_writes(s, writes); }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            for s in body.iter() { collect_stmt_writes(s, writes); }
        }
        _ => {}
    }
}

fn collect_expr_symbols(expr: &Expr, syms: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(s) => { syms.insert(*s); }
        Expr::Literal(_) | Expr::OptionNone => {}
        Expr::BinaryOp { left, right, .. } => {
            collect_expr_symbols(left, syms);
            collect_expr_symbols(right, syms);
        }
        Expr::Not { operand } | Expr::Length { collection: operand }
        | Expr::Copy { expr: operand } | Expr::Give { value: operand }
        | Expr::OptionSome { value: operand } | Expr::ManifestOf { zone: operand }
        | Expr::FieldAccess { object: operand, .. } => {
            collect_expr_symbols(operand, syms);
        }
        Expr::Index { collection, index } | Expr::Contains { collection, value: index }
        | Expr::Union { left: collection, right: index }
        | Expr::Intersection { left: collection, right: index }
        | Expr::Range { start: collection, end: index }
        | Expr::WithCapacity { value: collection, capacity: index }
        | Expr::ChunkAt { index, zone: collection } => {
            collect_expr_symbols(collection, syms);
            collect_expr_symbols(index, syms);
        }
        Expr::Slice { collection, start, end } => {
            collect_expr_symbols(collection, syms);
            collect_expr_symbols(start, syms);
            collect_expr_symbols(end, syms);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items { collect_expr_symbols(item, syms); }
        }
        Expr::New { init_fields, .. } => {
            for (_, val) in init_fields { collect_expr_symbols(val, syms); }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, val) in fields { collect_expr_symbols(val, syms); }
        }
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr { value, .. } = part {
                    collect_expr_symbols(value, syms);
                }
            }
        }
        Expr::Call { args, .. } => {
            for arg in args { collect_expr_symbols(arg, syms); }
        }
        Expr::CallExpr { callee, args } => {
            collect_expr_symbols(callee, syms);
            for arg in args { collect_expr_symbols(arg, syms); }
        }
        Expr::Closure { .. } | Expr::Escape { .. } => {}
    }
}

/// Check if any expression in a statement references a given symbol.
fn stmt_references_symbol(stmt: &Stmt, sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { var, value, .. } => {
            *var == sym || expr_references_symbol(value, sym)
        }
        Stmt::Set { target, value } => {
            *target == sym || expr_references_symbol(value, sym)
        }
        Stmt::Show { object, recipient } => {
            expr_references_symbol(object, sym) || expr_references_symbol(recipient, sym)
        }
        Stmt::Push { collection, value } | Stmt::Add { collection, value }
        | Stmt::Remove { collection, value } => {
            expr_references_symbol(collection, sym) || expr_references_symbol(value, sym)
        }
        Stmt::Pop { collection, into } => {
            expr_references_symbol(collection, sym) ||
            into.map_or(false, |s| s == sym)
        }
        Stmt::SetIndex { collection, index, value } => {
            expr_references_symbol(collection, sym) ||
            expr_references_symbol(index, sym) ||
            expr_references_symbol(value, sym)
        }
        Stmt::SetField { object, value, .. } => {
            expr_references_symbol(object, sym) || expr_references_symbol(value, sym)
        }
        Stmt::Return { value } => {
            value.map_or(false, |v| expr_references_symbol(v, sym))
        }
        Stmt::Call { args, .. } => {
            args.iter().any(|a| expr_references_symbol(a, sym))
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_references_symbol(cond, sym) ||
            then_block.iter().any(|s| stmt_references_symbol(s, sym)) ||
            else_block.map_or(false, |eb| eb.iter().any(|s| stmt_references_symbol(s, sym)))
        }
        Stmt::While { cond, body, .. } => {
            expr_references_symbol(cond, sym) ||
            body.iter().any(|s| stmt_references_symbol(s, sym))
        }
        Stmt::Repeat { iterable, body, pattern } => {
            let pattern_matches = match pattern {
                Pattern::Identifier(s) => *s == sym,
                Pattern::Tuple(syms) => syms.contains(&sym),
            };
            pattern_matches ||
            expr_references_symbol(iterable, sym) ||
            body.iter().any(|s| stmt_references_symbol(s, sym))
        }
        Stmt::Inspect { target, arms, .. } => {
            expr_references_symbol(target, sym) ||
            arms.iter().any(|arm| arm.body.iter().any(|s| stmt_references_symbol(s, sym)))
        }
        // Conservative: unknown statement types are assumed to reference the symbol
        _ => true,
    }
}

fn expr_references_symbol(expr: &Expr, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::Literal(_) | Expr::OptionNone => false,
        Expr::BinaryOp { left, right, .. } => {
            expr_references_symbol(left, sym) || expr_references_symbol(right, sym)
        }
        Expr::Not { operand } | Expr::Length { collection: operand }
        | Expr::Copy { expr: operand } | Expr::Give { value: operand }
        | Expr::OptionSome { value: operand } | Expr::ManifestOf { zone: operand }
        | Expr::FieldAccess { object: operand, .. } => {
            expr_references_symbol(operand, sym)
        }
        Expr::Index { collection, index } | Expr::Contains { collection, value: index }
        | Expr::Union { left: collection, right: index }
        | Expr::Intersection { left: collection, right: index }
        | Expr::Range { start: collection, end: index }
        | Expr::WithCapacity { value: collection, capacity: index }
        | Expr::ChunkAt { index, zone: collection } => {
            expr_references_symbol(collection, sym) || expr_references_symbol(index, sym)
        }
        Expr::Slice { collection, start, end } => {
            expr_references_symbol(collection, sym) ||
            expr_references_symbol(start, sym) ||
            expr_references_symbol(end, sym)
        }
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|item| expr_references_symbol(item, sym))
        }
        Expr::New { init_fields, .. } => {
            init_fields.iter().any(|(_, val)| expr_references_symbol(val, sym))
        }
        Expr::NewVariant { fields, .. } => {
            fields.iter().any(|(_, val)| expr_references_symbol(val, sym))
        }
        Expr::InterpolatedString(parts) => {
            parts.iter().any(|part| {
                if let StringPart::Expr { value, .. } = part {
                    expr_references_symbol(value, sym)
                } else {
                    false
                }
            })
        }
        Expr::Call { args, .. } => args.iter().any(|a| expr_references_symbol(a, sym)),
        Expr::CallExpr { callee, args } => {
            expr_references_symbol(callee, sym) ||
            args.iter().any(|a| expr_references_symbol(a, sym))
        }
        // Conservative for opaque expressions
        Expr::Closure { .. } | Expr::Escape { .. } => true,
    }
}

/// Build the fused Repeat loop. Replaces each `Push val to temp` in the producer
/// body with `Let pattern_var = val. <consumer body>`.
fn build_fused_repeat<'a>(
    producer: &Stmt<'a>,
    temp_sym: Symbol,
    pattern_var: Symbol,
    consumer_body: Block<'a>,
    _expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Option<Stmt<'a>> {
    match producer {
        Stmt::Repeat { pattern, iterable, body } => {
            let new_body = replace_pushes(
                body, temp_sym, pattern_var, consumer_body, stmt_arena,
            );
            Some(Stmt::Repeat {
                pattern: pattern.clone(),
                iterable,
                body: stmt_arena.alloc_slice(new_body),
            })
        }
        _ => None,
    }
}

/// In a block, replace each `Push val to temp` with
/// `Let pattern_var = val. <consumer_body>`.
/// Recurses into If branches to handle filter patterns.
fn replace_pushes<'a>(
    body: &[Stmt<'a>],
    temp_sym: Symbol,
    pattern_var: Symbol,
    consumer_body: Block<'a>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::new();
    for stmt in body {
        match stmt {
            Stmt::Push { value, collection } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == temp_sym {
                        result.push(Stmt::Let {
                            var: pattern_var,
                            ty: None,
                            value,
                            mutable: true,
                        });
                        result.extend(consumer_body.iter().cloned());
                        continue;
                    }
                }
                result.push(stmt.clone());
            }
            Stmt::If { cond, then_block, else_block } => {
                let new_then = replace_pushes(
                    then_block, temp_sym, pattern_var, consumer_body, stmt_arena,
                );
                let new_else = else_block.map(|eb| {
                    let replaced = replace_pushes(
                        eb, temp_sym, pattern_var, consumer_body, stmt_arena,
                    );
                    let b: Block = stmt_arena.alloc_slice(replaced);
                    b
                });
                result.push(Stmt::If {
                    cond,
                    then_block: stmt_arena.alloc_slice(new_then),
                    else_block: new_else,
                });
            }
            other => result.push(other.clone()),
        }
    }
    result
}

/// Check if the fused result still contains pushes to temp (incomplete replacement).
fn fused_still_pushes_to(stmt: &Stmt, temp_sym: Symbol) -> bool {
    match stmt {
        Stmt::Repeat { body, .. } => block_pushes_to(body, temp_sym),
        _ => false,
    }
}
