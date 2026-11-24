use std::collections::HashSet;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, MatchArm, Stmt, StringPart};
use crate::intern::Symbol;

/// Collect all symbols read by an expression.
/// Returns false if the expression contains opaque parts (Escape, Call, CallExpr)
/// whose reads cannot be fully determined.
fn collect_expr_reads(expr: &Expr, reads: &mut HashSet<Symbol>) -> bool {
    match expr {
        // A page query READS the live DOM: it is never dead, and never foldable.
        Expr::Dom(q) => match q.selector() {
            Some(sel) => collect_expr_reads(sel, reads),
            None => false,
        },
        Expr::View(v) => {
            let (view_exprs, view_blocks) = v.exprs_and_blocks();
            let mut ok = true;
            for e in view_exprs {
                ok &= collect_expr_reads(e, reads);
            }
            for b in view_blocks {
                for s in b.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
            ok
        }
        Expr::Identifier(sym) => {
            reads.insert(*sym);
            true
        }
        Expr::Literal(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. } => {
            let a = collect_expr_reads(left, reads);
            let b = collect_expr_reads(right, reads);
            a && b
        }
        Expr::Length { collection }
        | Expr::Not {
            operand: collection,
        }
        | Expr::Copy { expr: collection }
        | Expr::Give { value: collection }
        | Expr::OptionSome { value: collection }
        | Expr::ManifestOf { zone: collection } => collect_expr_reads(collection, reads),
        Expr::Index { collection, index }
        | Expr::Contains {
            collection,
            value: index,
        }
        | Expr::Union {
            left: collection,
            right: index,
        }
        | Expr::Intersection {
            left: collection,
            right: index,
        }
        | Expr::Range {
            start: collection,
            end: index,
        }
        | Expr::WithCapacity {
            value: collection,
            capacity: index,
        }
        | Expr::ChunkAt {
            index,
            zone: collection,
        } => {
            let a = collect_expr_reads(collection, reads);
            let b = collect_expr_reads(index, reads);
            a && b
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            let a = collect_expr_reads(collection, reads);
            let b = collect_expr_reads(start, reads);
            let c = collect_expr_reads(end, reads);
            a && b && c
        }
        Expr::FieldAccess { object, .. } => collect_expr_reads(object, reads),
        Expr::List(items) | Expr::Tuple(items) => {
            let mut ok = true;
            for item in items {
                ok &= collect_expr_reads(item, reads);
            }
            ok
        }
        Expr::New { init_fields, .. } => {
            let mut ok = true;
            for (_, val) in init_fields {
                ok &= collect_expr_reads(val, reads);
            }
            ok
        }
        Expr::NewVariant { fields, .. } => {
            let mut ok = true;
            for (_, val) in fields {
                ok &= collect_expr_reads(val, reads);
            }
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
            for arg in args {
                collect_expr_reads(arg, reads);
            }
            false
        }
        Expr::CallExpr { callee, args } => {
            collect_expr_reads(callee, reads);
            for arg in args {
                collect_expr_reads(arg, reads);
            }
            false
        }
        Expr::Closure { .. } | Expr::Escape { .. } => false,
    }
}

/// Collect all symbols read by a statement (recursively into sub-blocks).
fn collect_stmt_reads(stmt: &Stmt, reads: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::ComponentDef(def) => {
            for f in &def.state {
                collect_expr_reads(f.init, reads);
            }
            for ev in &def.events {
                for s in ev.body.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
            for a in &def.actions {
                for s in a.body.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
            if let Some(view) = &def.view {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    collect_expr_reads(e, reads);
                }
                for b in view_blocks {
                    for s in b.iter() {
                        collect_stmt_reads(s, reads);
                    }
                }
            }
        }
        Stmt::DirectiveDef { body, removed, .. } => {
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
            if let Some(r) = removed {
                for s in r.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
        }
        Stmt::RouteDef { body, .. } => match &**body {
            logicaffeine_language::ast::component::RouteBody::Statements(b) => {
                for s in b.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
            logicaffeine_language::ast::component::RouteBody::View(view) => {
                let (view_exprs, view_blocks) = view.exprs_and_blocks();
                for e in view_exprs {
                    collect_expr_reads(e, reads);
                }
                for b in view_blocks {
                    for s in b.iter() {
                        collect_stmt_reads(s, reads);
                    }
                }
            }
        },
        Stmt::MiddlewareDef { body, .. } => {
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::RedirectDef { .. } => {}
        Stmt::ServeFetches | Stmt::Replay { .. } => {}
        Stmt::GivenFetch {
            url, status, body, ..
        } => {
            collect_expr_reads(url, reads);
            collect_expr_reads(status, reads);
            collect_expr_reads(body, reads);
        }
        Stmt::Fetch { url, body, .. } => {
            collect_expr_reads(url, reads);
            if let Some(b) = body {
                collect_expr_reads(b, reads);
            }
        }
        Stmt::Ui(u) => {
            for x in u.exprs() {
                collect_expr_reads(x, reads);
            }
        }
        Stmt::Splice { body } | Stmt::Reversibly { body } => {
            for inner in body.iter() {
                collect_stmt_reads(inner, reads);
            }
        }
        Stmt::Let { value, .. } => {
            collect_expr_reads(value, reads);
        }
        Stmt::Set { value, .. } => {
            collect_expr_reads(value, reads);
        }
        // `Swap a and b` reads (and writes) both variables — both are live, never dead.
        Stmt::Swap { a, b } => {
            reads.insert(*a);
            reads.insert(*b);
        }
        Stmt::Show { object, recipient } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(recipient, reads);
        }
        Stmt::Push { collection, value }
        | Stmt::Add { collection, value }
        | Stmt::Remove { collection, value } => {
            collect_expr_reads(collection, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::Pop { collection, .. } => {
            collect_expr_reads(collection, reads);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            collect_expr_reads(collection, reads);
            collect_expr_reads(index, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::SetField { object, value, .. } => {
            collect_expr_reads(object, reads);
            collect_expr_reads(value, reads);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                collect_expr_reads(v, reads);
            }
        }
        Stmt::Call { args, .. } => {
            for arg in args {
                collect_expr_reads(arg, reads);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_expr_reads(cond, reads);
            for s in then_block.iter() {
                collect_stmt_reads(s, reads);
            }
            if let Some(eb) = else_block {
                for s in eb.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            collect_expr_reads(cond, reads);
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
            if let Some(d) = decreasing {
                collect_expr_reads(d, reads);
            }
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_expr_reads(iterable, reads);
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::RuntimeAssert {
            condition,
            reason: Some(reason),
            ..
        } => {
            collect_expr_reads(condition, reads);
            collect_expr_reads(reason, reads);
        }
        Stmt::Fail { reason } => {
            collect_expr_reads(reason, reads);
        }
        Stmt::RuntimeAssert { condition, .. }
        | Stmt::Sleep {
            milliseconds: condition,
        }
        | Stmt::StopTask { handle: condition }
        | Stmt::CloseChannel { channel: condition }
        | Stmt::AwaitTask {
            handle: condition, ..
        } => {
            collect_expr_reads(condition, reads);
        }
        Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
            collect_expr_reads(address, reads);
            if let Some(bind) = secure {
                // The pad path is an expression too (a var read must not be pruned as dead).
                collect_expr_reads(bind.pad, reads);
            }
        }
        Stmt::Serve { port, handler } => {
            collect_expr_reads(port, reads);
            collect_expr_reads(handler, reads);
        }
        Stmt::Give { object, recipient }
        | Stmt::MergeCrdt {
            source: object,
            target: recipient,
        }
        | Stmt::JoinCrdt {
            source: object,
            target: recipient,
        }
        | Stmt::SendMessage {
            message: object,
            destination: recipient,
            ..
        }
        | Stmt::StreamMessage {
            values: object,
            destination: recipient,
        }
        | Stmt::WriteFile {
            content: object,
            path: recipient,
        }
        | Stmt::SendPipe {
            value: object,
            pipe: recipient,
        }
        | Stmt::TrySendPipe {
            value: object,
            pipe: recipient,
            ..
        }
        | Stmt::AppendToSequence {
            sequence: recipient,
            value: object,
        } => {
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
                for s in arm.body.iter() {
                    collect_stmt_reads(s, reads);
                }
            }
        }
        Stmt::Zone { body, .. }
        | Stmt::Concurrent { tasks: body }
        | Stmt::Parallel { tasks: body } => {
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::FunctionDef { .. }
        | Stmt::StructDef { .. }
        | Stmt::Theorem(..)
        | Stmt::Definition(..)
        | Stmt::Axiom(..)
        | Stmt::Theory(..)
        | Stmt::Escape { .. }
        | Stmt::Require { .. }
        | Stmt::Break
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Check { .. }
        | Stmt::Spawn { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::Mount { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::LetPeerAgent { .. }
        | Stmt::Sync { .. }
        | Stmt::TestDef { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::GivenFile { .. } => {}
        // Observation payloads are LIVE reads (a var named in a `Note` must not
        // be pruned as dead); `During` recurses its body.
        Stmt::Observe { payloads, .. } => {
            for (_, e) in payloads {
                collect_expr_reads(e, reads);
            }
        }
        Stmt::During { body, .. } => {
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::ForAll {
            constraint, body, ..
        } => {
            if let Some(c) = constraint {
                collect_expr_reads(c, reads);
            }
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::Expect { condition } => {
            collect_expr_reads(condition, reads);
        }
        Stmt::ExpectFail { body, .. } => {
            for s in body.iter() {
                collect_stmt_reads(s, reads);
            }
        }
        Stmt::AdvanceTime { millis } => {
            collect_expr_reads(millis, reads);
        }
        Stmt::LaunchTask { args, .. } | Stmt::LaunchTaskWithHandle { args, .. } => {
            for arg in args {
                collect_expr_reads(arg, reads);
            }
        }
        Stmt::AwaitMessage { source, .. }
        | Stmt::ReceivePipe { pipe: source, .. }
        | Stmt::TryReceivePipe { pipe: source, .. } => {
            collect_expr_reads(source, reads);
        }
        Stmt::Select { branches } => {
            for branch in branches {
                match branch {
                    crate::ast::stmt::SelectBranch::Receive { pipe, body, .. } => {
                        collect_expr_reads(pipe, reads);
                        for s in body.iter() {
                            collect_stmt_reads(s, reads);
                        }
                    }
                    crate::ast::stmt::SelectBranch::Timeout { milliseconds, body } => {
                        collect_expr_reads(milliseconds, reads);
                        for s in body.iter() {
                            collect_stmt_reads(s, reads);
                        }
                    }
                }
            }
        }
        // An opaque failure-site marker: an id, no expression to read from.
        Stmt::Site(_) => {}
    }
}

/// Whether evaluating `e` can neither fail nor do anything: names, literals, `+ - *` (exact, they never fail),
/// comparisons, `and`/`or`, `not` and lengths of names. Every division, a power (a negative exponent fails) and every
/// other operator are left out.
fn infallible(e: &Expr) -> bool {
    match e {
        Expr::Identifier(_) | Expr::Literal(_) => true,
        Expr::Not { operand } => infallible(operand),
        Expr::Length { collection } => matches!(collection, Expr::Identifier(_)),
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::And
                    | BinaryOpKind::Or
            ) && infallible(left)
                && infallible(right)
        }
        _ => false,
    }
}

/// Dead-store elimination, walking `stmts` backward. `dead` holds the names every path after the block overwrites
/// before it reads them; the statements kept come back with what is dead before the block. A `Set` whose value calls
/// nothing and whose target is dead goes. An `If`'s arms are walked with what is dead after it, so a store an arm
/// leaves for the code after the `If` to overwrite goes too, and an `If` left with nothing to do goes whole when its
/// guard can neither fail nor do anything ([`infallible`]). A `Let` reads its value and kills nothing (never a `Let` —
/// Let+Set merge conflicts with codegen peepholes — and a `Let` may shadow); a loop, a call and every other statement
/// may read any name.
fn drop_dead_stores<'a>(
    stmts: Vec<Stmt<'a>>,
    mut dead: HashSet<Symbol>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> (Vec<Stmt<'a>>, HashSet<Symbol>) {
    let mut kept = Vec::with_capacity(stmts.len());
    for stmt in stmts.into_iter().rev() {
        let mut reads = HashSet::new();
        match &stmt {
            Stmt::Set { target, value } if collect_expr_reads(value, &mut reads) => {
                if dead.contains(target) {
                    continue;
                }
                dead.insert(*target);
                dead.retain(|s| !reads.contains(s));
            }
            Stmt::Let { value, .. } if collect_expr_reads(value, &mut reads) => {
                dead.retain(|s| !reads.contains(s));
            }
            Stmt::If { cond, then_block, else_block } => {
                let analyzable = collect_expr_reads(cond, &mut reads);
                let (then_kept, then_dead) = drop_dead_stores(then_block.to_vec(), dead.clone(), stmt_arena);
                let (else_kept, else_dead) = match else_block {
                    Some(block) => {
                        let (kept, dead_before) = drop_dead_stores(block.to_vec(), dead.clone(), stmt_arena);
                        (Some(kept), dead_before)
                    }
                    None => (None, dead.clone()),
                };
                let else_idle = else_kept.as_ref().is_none_or(|kept| kept.is_empty());
                if then_kept.is_empty() && else_idle && infallible(cond) {
                    continue;
                }
                // An `If` left with nothing to do under a guard that may fail, or with an empty first arm and a
                // second that still works, stays as it was written.
                if then_kept.is_empty() {
                    dead.clear();
                    kept.push(stmt);
                    continue;
                }
                dead = if analyzable {
                    then_dead.intersection(&else_dead).filter(|s| !reads.contains(s)).copied().collect()
                } else {
                    HashSet::new()
                };
                let then_block = if then_kept.len() == then_block.len() {
                    *then_block
                } else {
                    &*stmt_arena.alloc_slice(then_kept)
                };
                let else_block = match (else_block, else_kept) {
                    (_, Some(kept)) if kept.is_empty() => None,
                    (Some(block), Some(kept)) if kept.len() == block.len() => Some(*block),
                    (_, Some(kept)) => Some(&*stmt_arena.alloc_slice(kept)),
                    (_, None) => None,
                };
                kept.push(Stmt::If { cond: *cond, then_block, else_block });
                continue;
            }
            _ => dead.clear(),
        }
        kept.push(stmt);
    }
    kept.reverse();
    (kept, dead)
}

pub fn eliminate_dead_code<'a>(
    stmts: Vec<Stmt<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Vec<Stmt<'a>> {
    // The outermost call sees the whole program: what its functions and closures reach of Main, for every block
    // below it.
    if REACHED.with(|r| r.borrow().is_some()) {
        return eliminate_dead_code_in(stmts, stmt_arena, expr_arena);
    }
    let _program = Program::install(&stmts);
    eliminate_dead_code_in(stmts, stmt_arena, expr_arena)
}

/// The program [`eliminate_dead_code`] works on, installed for its length: cleared when it ends, unwinding included.
struct Program;

impl Program {
    fn install(stmts: &[Stmt]) -> Self {
        REACHED.with(|r| *r.borrow_mut() = Some(Reached::of_program(stmts)));
        IN_FUNCTION.with(|f| f.set(false));
        Program
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        REACHED.with(|r| *r.borrow_mut() = None);
        IN_FUNCTION.with(|f| f.set(false));
    }
}

/// The names of Main a call may read or write: what a function or closure body reads or writes without declaring
/// it, or every name when foreign code runs in one.
struct Reached {
    globals: super::writes::CalleeGlobals,
}

impl Reached {
    fn of_program(stmts: &[Stmt]) -> Self {
        Reached { globals: super::writes::CalleeGlobals::of_program(stmts) }
    }

    fn contains(&self, sym: Symbol) -> bool {
        self.globals.opaque || self.globals.reads.contains(&sym) || self.globals.symbols.contains(&sym)
    }
}

thread_local! {
    /// The program's [`Reached`], for the length of one [`eliminate_dead_code`].
    static REACHED: std::cell::RefCell<Option<Reached>> = const { std::cell::RefCell::new(None) };
    /// Whether the block being eliminated is inside a function's body, whose bindings no call can reach.
    static IN_FUNCTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn eliminate_dead_code_in<'a>(
    stmts: Vec<Stmt<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Vec<Stmt<'a>> {
    // First: dead store elimination (what is dead after the block is not known here)
    let (stmts, _) = drop_dead_stores(stmts, HashSet::new(), stmt_arena);
    #[cfg(feature = "codegen")]
    let stmts = counted_to_the_end(stmts, stmt_arena, expr_arena);
    // Then: dead branch elimination
    let mut result = Vec::with_capacity(stmts.len());

    for stmt in stmts {
        match stmt {
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                if let Expr::Literal(Literal::Boolean(val)) = cond {
                    // The branch that runs takes the If's place. A branch that declares names of its own
                    // stays a block, `If true:` around it, so those names end with it (see
                    // `capture::own_names`); one that declares none is spliced.
                    let taken = if *val { Some(then_block) } else { else_block };
                    if let Some(branch) = taken {
                        if super::capture::own_names(branch).is_empty() {
                            result.extend(branch.iter().cloned());
                        } else {
                            result.push(Stmt::If {
                                cond: expr_arena.alloc(Expr::Literal(Literal::Boolean(true))),
                                then_block: dce_block(branch, stmt_arena, expr_arena),
                                else_block: None,
                            });
                        }
                    }
                } else {
                    let then_dce = dce_block(then_block, stmt_arena, expr_arena);
                    let else_dce = else_block.map(|b| dce_block(b, stmt_arena, expr_arena));
                    result.push(Stmt::If {
                        cond,
                        then_block: then_dce,
                        else_block: else_dce,
                    });
                }
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
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
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                result.push(Stmt::Repeat {
                    pattern,
                    iterable,
                    body: dce_block(body, stmt_arena, expr_arena),
                });
            }
            Stmt::FunctionDef {
                name,
                params,
                generics,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => {
                let outside = IN_FUNCTION.with(|f| f.replace(true));
                let body = dce_block(body, stmt_arena, expr_arena);
                IN_FUNCTION.with(|f| f.set(outside));
                result.push(Stmt::FunctionDef {
                    name,
                    params,
                    generics,
                    body,
                    return_type,
                    is_native,
                    native_path,
                    is_exported,
                    export_target,
                    opt_flags,
                });
            }
            Stmt::Zone {
                name,
                capacity,
                source_file,
                body,
            } => {
                result.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: dce_block(body, stmt_arena, expr_arena),
                });
            }
            // Each task is a block of its own: a `Return` ends that task alone, never the tasks after it.
            Stmt::Concurrent { tasks } => {
                result.push(Stmt::Concurrent {
                    tasks: dce_tasks(tasks, stmt_arena, expr_arena),
                });
            }
            Stmt::Parallel { tasks } => {
                result.push(Stmt::Parallel {
                    tasks: dce_tasks(tasks, stmt_arena, expr_arena),
                });
            }
            Stmt::Reversibly { body } => {
                result.push(Stmt::Reversibly {
                    body: dce_block(body, stmt_arena, expr_arena),
                });
            }
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let arms_dce: Vec<MatchArm<'a>> = arms
                    .into_iter()
                    .map(|arm| MatchArm {
                        enum_name: arm.enum_name,
                        variant: arm.variant,
                        bindings: arm.bindings,
                        body: dce_block(arm.body, stmt_arena, expr_arena),
                    })
                    .collect();
                result.push(Stmt::Inspect {
                    target,
                    arms: arms_dce,
                    has_otherwise,
                });
            }
            other => result.push(other),
        }
    }

    if let Some(pos) = result.iter().position(|s| matches!(s, Stmt::Return { .. })) {
        result.truncate(pos + 1);
    }

    dead_variable_elimination(result, stmt_arena)
}

/// Each counted `While` of `stmts` that does nothing but count ([`crate::loop_shape::extract_counted_while`], with an
/// empty body and a bound that can neither fail nor do anything) as the value it leaves the counter: `If <its guard>:
/// Set i to <the bound>`, the bound plus one for an inclusive loop. A loop that never runs leaves the start. The
/// counted-loop shape is the codegen feature's, so this is too.
#[cfg(feature = "codegen")]
fn counted_to_the_end<'a>(
    mut stmts: Vec<Stmt<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> Vec<Stmt<'a>> {
    for k in 0..stmts.len().saturating_sub(1) {
        let counted = {
            let refs: Vec<&Stmt<'a>> = stmts.iter().collect();
            crate::loop_shape::extract_counted_while(&refs, k)
        };
        let Some((counted, _)) = counted else { continue };
        let Stmt::While { cond, .. } = &stmts[k + 1] else { continue };
        if !counted.body_without_increment.is_empty() || !infallible(counted.limit) || !infallible(cond) {
            continue;
        }
        let end = if counted.inclusive {
            &*expr_arena.alloc(Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: counted.limit,
                right: expr_arena.alloc(Expr::Literal(Literal::Number(1))),
            })
        } else {
            counted.limit
        };
        stmts[k + 1] = Stmt::If {
            cond: *cond,
            then_block: stmt_arena.alloc_slice(vec![Stmt::Set { target: counted.var, value: end }]),
            else_block: None,
        };
    }
    stmts
}

fn dce_block<'a>(
    block: &'a [Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a [Stmt<'a>] {
    let stmts: Vec<Stmt<'a>> = block.iter().cloned().collect();
    let dce_result = eliminate_dead_code_in(stmts, stmt_arena, expr_arena);
    stmt_arena.alloc_slice(dce_result)
}

/// Each task of an `Attempt all of the following:` or `Simultaneously:` block, dead code eliminated alone.
fn dce_tasks<'a>(
    tasks: &'a [Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a [Stmt<'a>] {
    let each: Vec<Stmt<'a>> = tasks
        .iter()
        .flat_map(|task| match task {
            // A task's binding is the block's: what follows the block reads it.
            Stmt::Let { .. } => vec![task.clone()],
            _ => eliminate_dead_code_in(vec![task.clone()], stmt_arena, expr_arena),
        })
        .collect();
    stmt_arena.alloc_slice(each)
}

/// Remove each `Let` of a literal that nothing reads: every later statement of the block that mentions the name only
/// stores to it ([`only_stored`]), and no call can reach it (a binding inside a function's body, or one of Main that no
/// function or closure names as a global). Its stores go with it — the counter an unrolled loop leaves behind. A
/// literal can neither fail nor do anything, so the binding is all there is to it; a refinement-typed `Let` keeps the
/// check its type makes.
fn dead_variable_elimination<'a>(mut stmts: Vec<Stmt<'a>>, stmt_arena: &'a Arena<Stmt<'a>>) -> Vec<Stmt<'a>> {
    let in_function = IN_FUNCTION.with(|f| f.get());
    let reached = |sym: Symbol| {
        !in_function && REACHED.with(|r| r.borrow().as_ref().is_none_or(|r| r.contains(sym)))
    };
    let mut k = 0;
    while k < stmts.len() {
        let dead = match &stmts[k] {
            Stmt::Let { var, value: Expr::Literal(_), ty, .. }
                if !matches!(ty, Some(crate::ast::stmt::TypeExpr::Refinement { .. }))
                    && only_stored(&stmts[k + 1..], *var)
                    && !reached(*var) =>
            {
                Some(*var)
            }
            _ => None,
        };
        match dead {
            Some(var) => {
                let rest = stmts.split_off(k + 1);
                stmts.pop();
                stmts.extend(without_stores(rest, var, stmt_arena));
            }
            None => k += 1,
        }
    }
    stmts
}

/// Whether `stmts` only store to `var`: every statement that mentions it is a `Set` of it to a value that can neither
/// fail nor do anything ([`infallible`]; the value may read `var` itself, to compute what is stored in it), or an `If`
/// or `While` whose own guard does not mention it and whose blocks only store to it.
fn only_stored(stmts: &[Stmt], var: Symbol) -> bool {
    use super::mentions::{expr_touches, stmt_touches};
    stmts.iter().all(|stmt| match stmt {
        Stmt::Set { target, value } if *target == var => infallible(value),
        Stmt::If { cond, then_block, else_block } => {
            !expr_touches(cond, var)
                && only_stored(then_block, var)
                && else_block.is_none_or(|block| only_stored(block, var))
        }
        Stmt::While { cond, body, decreasing } => {
            !expr_touches(cond, var)
                && decreasing.is_none_or(|d| !expr_touches(d, var))
                && only_stored(body, var)
        }
        other => !stmt_touches(other, var),
    })
}

/// `stmts` without their stores to `var`, which they only store to ([`only_stored`]).
fn without_stores<'a>(stmts: Vec<Stmt<'a>>, var: Symbol, stmt_arena: &'a Arena<Stmt<'a>>) -> Vec<Stmt<'a>> {
    fn block<'a>(block: &'a [Stmt<'a>], var: Symbol, stmt_arena: &'a Arena<Stmt<'a>>) -> &'a [Stmt<'a>] {
        stmt_arena.alloc_slice(without_stores(block.to_vec(), var, stmt_arena))
    }
    stmts
        .into_iter()
        .filter_map(|stmt| match stmt {
            Stmt::Set { target, .. } if target == var => None,
            Stmt::If { cond, then_block, else_block } => Some(Stmt::If {
                cond,
                then_block: block(then_block, var, stmt_arena),
                else_block: else_block.map(|b| block(b, var, stmt_arena)),
            }),
            Stmt::While { cond, body, decreasing } => {
                Some(Stmt::While { cond, body: block(body, var, stmt_arena), decreasing })
            }
            other => Some(other),
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/dce/tests.rs"]
mod tests;
