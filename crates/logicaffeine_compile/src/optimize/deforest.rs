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
//!
//! The fused loop runs consumer iteration k before producer iterations k+1..n, so the two loops fuse only
//! when those actions commute ([`fusion_commutes`]): the intermediate is touched only by the producer's
//! pushes and the consumer's walk, neither loop writes a name the other touches, the two do not both print,
//! a failure is not reordered against a print or another failure, and a consumer that can leave early
//! leaves behind nothing the producer would still have done. Names are compared as names
//! ([`super::mentions`]).

use std::collections::HashSet;

use super::mentions::Mentions;
use super::writes::{CalleeGlobals, WriteSet};
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Pattern, Stmt, StringPart};
use crate::intern::{Interner, Symbol};

pub fn deforest_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    let globals = CalleeGlobals::of_program(&stmts);
    deforest_block(stmts, expr_arena, stmt_arena, interner, &globals)
}

fn deforest_block<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
    globals: &CalleeGlobals,
) -> Vec<Stmt<'a>> {
    // Run iteratively for multi-stage fusion (A→B→C fuses in 2 passes)
    let mut current = stmts;
    for _ in 0..4 {
        let prev_len = current.len();
        current = deforest_pass(current, expr_arena, stmt_arena, interner, globals);
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
    globals: &CalleeGlobals,
) -> Vec<Stmt<'a>> {
    let mut result = Vec::with_capacity(stmts.len());
    let mut i = 0;

    while i < stmts.len() {
        if let Some((skip_to, fused)) =
            try_deforest(&stmts, i, expr_arena, stmt_arena, interner, globals)
        {
            result.extend(fused);
            i = skip_to;
            continue;
        }

        let stmt = stmts[i].clone();
        result.push(recurse_deforest(
            stmt, expr_arena, stmt_arena, interner, globals,
        ));
        i += 1;
    }

    result
}

fn recurse_deforest<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
    globals: &CalleeGlobals,
) -> Stmt<'a> {
    match stmt {
        Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => {
            let new_body = deforest_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::FunctionDef {
                name,
                generics,
                params,
                body: stmt_arena.alloc_slice(new_body),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            let new_body = deforest_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
            Stmt::While {
                cond,
                body: stmt_arena.alloc_slice(new_body),
                decreasing,
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let new_then = deforest_block(
                then_block.to_vec(),
                expr_arena,
                stmt_arena,
                interner,
                globals,
            );
            let new_else = else_block.map(|eb| {
                let processed =
                    deforest_block(eb.to_vec(), expr_arena, stmt_arena, interner, globals);
                let b: Block = stmt_arena.alloc_slice(processed);
                b
            });
            Stmt::If {
                cond,
                then_block: stmt_arena.alloc_slice(new_then),
                else_block: new_else,
            }
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let new_body = deforest_block(body.to_vec(), expr_arena, stmt_arena, interner, globals);
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
    globals: &CalleeGlobals,
) -> Option<(usize, Vec<Stmt<'a>>)> {
    // Step 1: stmts[start] must be `Let mutable temp = new Seq/List`
    let temp_sym = match_temp_init(&stmts[start], interner)?;
    let temp = interner.resolve(temp_sym);

    // Step 2: stmts[start+1] must be a Repeat that pushes to temp
    let producer_idx = start + 1;
    if producer_idx >= stmts.len() {
        return None;
    }
    if !is_repeat_producer(&stmts[producer_idx], temp_sym) {
        return None;
    }

    // Step 3: Find the consumer Repeat over temp, collecting intermediates
    let mut consumer_idx = None;
    let mut intermediates = Vec::new();
    for j in (producer_idx + 1)..stmts.len() {
        if is_repeat_over_sym(&stmts[j], temp_sym) {
            consumer_idx = Some(j);
            break;
        }
        if Mentions::of_stmt(&stmts[j]).touches(temp, interner) {
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
        if Mentions::of_stmt(&stmts[j]).touches(temp, interner) {
            return None;
        }
    }

    // Step 5: Extract consumer details
    let (pattern_var, consumer_body) = match &stmts[consumer_idx] {
        Stmt::Repeat {
            pattern: Pattern::Identifier(var),
            body,
            ..
        } => (*var, *body),
        _ => return None,
    };

    // Step 6: Safety — the two loops' actions commute, and the intermediates hoisted above the
    // producer can neither see nor shadow what it does.
    if !fusion_commutes(
        &stmts[producer_idx],
        temp,
        pattern_var,
        consumer_body,
        &intermediates,
        interner,
        globals,
    ) {
        return None;
    }

    // Step 7: Build the fused loop
    let fused = build_fused_repeat(
        &stmts[producer_idx],
        temp_sym,
        pattern_var,
        consumer_body,
        expr_arena,
        stmt_arena,
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
        Stmt::Let {
            var,
            value,
            mutable: true,
            ..
        } => {
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
                    if *s == sym {
                        return true;
                    }
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if block_pushes_to(then_block, sym) {
                    return true;
                }
                if let Some(eb) = else_block {
                    if block_pushes_to(eb, sym) {
                        return true;
                    }
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
        Stmt::Repeat {
            pattern: Pattern::Identifier(_),
            iterable,
            ..
        } => {
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

/// What a loop body does, as far as reordering it against another loop's work can tell. Names are
/// spelled names: the parser may mint a distinct symbol for each occurrence of one variable.
struct Actions {
    /// Every name the body reads, declares, assigns or mutates.
    touched: HashSet<String>,
    /// Every name the body declares, assigns or mutates, a callee's global writes included.
    written: HashSet<String>,
    /// The names it writes that outlive one pass: assigned or mutated, not declared in the body.
    outer_writes: HashSet<String>,
    /// It prints, or runs a call (a callee may print).
    shows: bool,
    /// It runs a call: a callee may read a global the other loop writes.
    calls: bool,
    /// It can fail: a division, an index, a call, a `Fail`, a nested `While` that may not finish.
    fails: bool,
    /// It can leave its own loop early: a `Break` of that loop, or a `Return`.
    exits: bool,
    /// It returns from the function.
    returns: bool,
    /// It does something this summary does not model; such a body is never reordered.
    unknown: bool,
}

impl Actions {
    /// The actions of `body`, leaving out what `skip` (the producer's pushes to the intermediate) does
    /// beyond evaluating the pushed value.
    fn of(body: &[Stmt], skip: Option<&str>, interner: &Interner, globals: &CalleeGlobals) -> Self {
        let names = |syms: &mut dyn Iterator<Item = Symbol>| -> HashSet<String> {
            syms.map(|s| interner.resolve(s).to_string()).filter(|n| Some(n.as_str()) != skip).collect()
        };
        let mentions = Mentions::of_block(body);
        let writes = WriteSet::of_block(body).with_callee_globals(globals);
        let declared = names(&mut writes.declared().iter().copied());
        let mut acts = Actions {
            touched: names(&mut mentions.symbols()),
            written: names(&mut writes.symbols()),
            outer_writes: names(&mut writes.assigned().iter().chain(writes.mutated().iter()).copied())
                .into_iter()
                .filter(|n| !declared.contains(n))
                .collect(),
            shows: false,
            calls: writes.has_calls(),
            fails: false,
            exits: false,
            returns: false,
            unknown: mentions.is_opaque() || writes.is_opaque(),
        };
        acts.walk_block(body, 0, skip, interner);
        if acts.calls {
            acts.shows = true;
            acts.fails = true;
        }
        acts
    }

    fn walk_block(&mut self, block: &[Stmt], depth: u32, skip: Option<&str>, interner: &Interner) {
        for stmt in block {
            self.walk_stmt(stmt, depth, skip, interner);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt, depth: u32, skip: Option<&str>, interner: &Interner) {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => self.walk_expr(value),
            Stmt::Push { value, collection } => {
                let skipped = matches!(collection, Expr::Identifier(c) if Some(interner.resolve(*c)) == skip);
                if !skipped {
                    self.walk_expr(collection);
                }
                self.walk_expr(value);
            }
            Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
                self.walk_expr(collection);
                self.walk_expr(value);
            }
            Stmt::SetField { object, value, .. } => {
                self.walk_expr(object);
                self.walk_expr(value);
            }
            Stmt::SetIndex { collection, index, value } => {
                self.fails = true;
                self.walk_expr(collection);
                self.walk_expr(index);
                self.walk_expr(value);
            }
            Stmt::Pop { collection, .. } => {
                self.fails = true;
                self.walk_expr(collection);
            }
            Stmt::Show { object, recipient } => {
                self.shows = true;
                self.walk_expr(object);
                self.walk_expr(recipient);
            }
            Stmt::If { cond, then_block, else_block } => {
                self.walk_expr(cond);
                self.walk_block(then_block, depth, skip, interner);
                if let Some(eb) = else_block {
                    self.walk_block(eb, depth, skip, interner);
                }
            }
            Stmt::Repeat { iterable, body, .. } => {
                self.walk_expr(iterable);
                self.walk_block(body, depth + 1, skip, interner);
            }
            Stmt::While { cond, body, .. } => {
                self.fails = true;
                self.walk_expr(cond);
                self.walk_block(body, depth + 1, skip, interner);
            }
            Stmt::Break => {
                if depth == 0 {
                    self.exits = true;
                }
            }
            Stmt::Return { value } => {
                self.exits = true;
                self.returns = true;
                if let Some(v) = value {
                    self.walk_expr(v);
                }
            }
            Stmt::Fail { reason } => {
                self.fails = true;
                self.walk_expr(reason);
            }
            _ => self.unknown = true,
        }
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => {}
            Expr::BinaryOp { op, left, right } => {
                if matches!(
                    op,
                    BinaryOpKind::Divide
                        | BinaryOpKind::ExactDivide
                        | BinaryOpKind::FloorDivide
                        | BinaryOpKind::Modulo
                        | BinaryOpKind::Pow
                ) {
                    self.fails = true;
                }
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Not { operand }
            | Expr::Length { collection: operand }
            | Expr::Copy { expr: operand }
            | Expr::OptionSome { value: operand }
            | Expr::FieldAccess { object: operand, .. } => self.walk_expr(operand),
            Expr::Contains { collection, value } => {
                self.walk_expr(collection);
                self.walk_expr(value);
            }
            Expr::Union { left, right } | Expr::Intersection { left, right } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Range { start, end } => {
                self.walk_expr(start);
                self.walk_expr(end);
            }
            Expr::Index { collection, index } => {
                self.fails = true;
                self.walk_expr(collection);
                self.walk_expr(index);
            }
            Expr::Slice { collection, start, end } => {
                self.fails = true;
                self.walk_expr(collection);
                self.walk_expr(start);
                self.walk_expr(end);
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for item in items.iter() {
                    self.walk_expr(item);
                }
            }
            Expr::New { init_fields, .. } => {
                for (_, v) in init_fields.iter() {
                    self.walk_expr(v);
                }
            }
            Expr::NewVariant { fields, .. } => {
                for (_, v) in fields.iter() {
                    self.walk_expr(v);
                }
            }
            Expr::InterpolatedString(parts) => {
                for part in parts.iter() {
                    if let StringPart::Expr { value, .. } = part {
                        self.walk_expr(value);
                    }
                }
            }
            Expr::Call { args, .. } => {
                self.calls = true;
                for a in args.iter() {
                    self.walk_expr(a);
                }
            }
            Expr::CallExpr { callee, args } => {
                self.calls = true;
                self.walk_expr(callee);
                for a in args.iter() {
                    self.walk_expr(a);
                }
            }
            _ => self.unknown = true,
        }
    }
}

/// Whether `stmts` touch `temp` only as the target of `Push v to temp` with `v` not touching it, entering
/// the `If`s [`replace_pushes`] rewrites. Any other place `temp` appears needs the list itself.
fn touches_temp_only_by_pushing(stmts: &[Stmt], temp: &str, interner: &Interner) -> bool {
    stmts.iter().all(|stmt| match stmt {
        Stmt::Push { value, collection: Expr::Identifier(c) } if interner.resolve(*c) == temp => {
            !Mentions::of_expr(value).touches(temp, interner)
        }
        Stmt::If { cond, then_block, else_block } => {
            !Mentions::of_expr(cond).touches(temp, interner)
                && touches_temp_only_by_pushing(then_block, temp, interner)
                && else_block.map_or(true, |eb| touches_temp_only_by_pushing(eb, temp, interner))
        }
        other => !Mentions::of_stmt(other).touches(temp, interner),
    })
}

/// Whether fusing `producer` (which fills `temp`) with the consumer `Repeat for pattern_var in temp:
/// consumer_body`, hoisting `intermediates` above the producer, keeps the program's meaning. The fused loop
/// runs the consumer's pass for item k before the producer's passes k+1..n, so every action of the one must
/// commute with every action of the other.
fn fusion_commutes(
    producer: &Stmt,
    temp: &str,
    pattern_var: Symbol,
    consumer_body: &[Stmt],
    intermediates: &[Stmt],
    interner: &Interner,
    globals: &CalleeGlobals,
) -> bool {
    let Stmt::Repeat { pattern, iterable, body } = producer else {
        return false;
    };
    if !touches_temp_only_by_pushing(body, temp, interner) {
        return false;
    }
    let mut p = Actions::of(body, Some(temp), interner, globals);
    let c = Actions::of(consumer_body, None, interner, globals);
    if p.unknown || c.unknown || p.returns {
        return false;
    }
    let source = Mentions::of_expr(iterable);
    if source.is_opaque() || c.touched.contains(temp) {
        return false;
    }
    p.touched.extend(source.symbols().map(|s| interner.resolve(s).to_string()));

    // Data: neither loop writes a name the other touches. The consumer's pass now runs inside the
    // producer's, so its item name must not be one the producer reads, nor the producer's item names
    // ones the consumer reads.
    if c.written.iter().any(|n| p.touched.contains(n)) || p.written.iter().any(|n| c.touched.contains(n)) {
        return false;
    }
    if p.touched.contains(interner.resolve(pattern_var)) {
        return false;
    }
    let producer_items: Vec<Symbol> = match pattern {
        Pattern::Identifier(s) => vec![*s],
        Pattern::Tuple(syms) => syms.clone(),
    };
    if producer_items.iter().any(|s| c.touched.contains(interner.resolve(*s))) {
        return false;
    }
    // A callee may read a global the other loop writes.
    if p.calls && !c.written.is_empty() || c.calls && !p.written.is_empty() {
        return false;
    }

    // Order: two loops that both print would interleave their lines; a failure moved ahead of or past a
    // print, or past another failure, changes what the run shows or which error it reports.
    if p.shows && c.shows || p.fails && (c.shows || c.fails) || c.fails && p.shows {
        return false;
    }
    // A consumer that leaves early also stops the producer's later passes: they must have done nothing.
    if c.exits && (p.shows || p.fails || !p.outer_writes.is_empty()) {
        return false;
    }

    // The intermediates now run before the producer: each must be a `Let` that cannot fail, reads
    // nothing the producer writes, and binds no name the producer touches.
    intermediates.iter().all(|stmt| {
        let Stmt::Let { var, .. } = stmt else {
            return false;
        };
        let acts = Actions::of(std::slice::from_ref(stmt), None, interner, globals);
        !acts.unknown
            && !acts.fails
            && !acts.shows
            && !p.touched.contains(interner.resolve(*var))
            && !acts.touched.iter().any(|n| p.written.contains(n))
    })
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
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            let new_body = replace_pushes(body, temp_sym, pattern_var, consumer_body, stmt_arena);
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
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let new_then =
                    replace_pushes(then_block, temp_sym, pattern_var, consumer_body, stmt_arena);
                let new_else = else_block.map(|eb| {
                    let replaced =
                        replace_pushes(eb, temp_sym, pattern_var, consumer_body, stmt_arena);
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
