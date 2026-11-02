//! The stepped tier: a concurrent program whose tasks run as resumable step functions over heap frames
//! instead of coroutines on stacks of their own — what a GPU thread can run, and what the host walker runs.
//!
//! A function is STEPPED when it is launched as a task or reaches a scheduling point through its calls
//! ([`stepped_functions`]); `## Main` of a concurrent program is stepped too, and every other function stays
//! an ordinary C function. A stepped function becomes `logos_frame_<f>` — the frame header, one cell per
//! argument its launch or its caller passes, then one field per binding and temporary of its body — and
//! `logos_step_<f>`, whose body is the ordinary statement lowering under `switch (fr->__hdr.pc)`. Every
//! binding is a field (`fr->x`), so no declaration stands between the switch and a `case`, and a resume may
//! land inside any loop or branch. A scheduling point performs the core's operation (`logos_op_*`), saves the
//! point to resume at and returns `LOGOS_STEP_YIELD`; its `case` reads what the operation delivered. A call
//! to a function that waits pushes the callee's frame and returns `LOGOS_STEP_CALL`; the callee's `Return`
//! pops back to the caller's resume point.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use logicaffeine_language::ast_depth::{expr_children, for_each_node, stmt_expr_children, Visited};

use crate::ast::stmt::{Expr, SelectBranch, Stmt};
use crate::intern::Symbol;

use super::emit::{
    as_returned, c_refuse_expr, codegen_expr, codegen_stmt, emit_binding, pipe_element,
    pipe_packed_value, select_timeout_ticks_c, task_arg_pack, task_arg_unpack,
};
use super::types::{
    c_param_decl, escape_c_ident, resolve_type_expr_with_registry, CContext, CType, Frame,
};

/// The functions that wait — each reaches a scheduling point, directly or through the functions it calls —
/// and the functions launched as tasks. A function that waits runs only as a step function; one launched
/// that never waits is also an ordinary C function for its plain callers.
pub(super) struct Stepped {
    pub(super) waits: HashSet<Symbol>,
    pub(super) launched: HashSet<Symbol>,
}

impl Stepped {
    /// Every function that gets a step function.
    pub(super) fn steps(&self) -> impl Iterator<Item = &Symbol> {
        self.waits
            .iter()
            .chain(self.launched.iter().filter(|f| !self.waits.contains(f)))
    }
}

/// Which functions of the program wait and which are launched.
pub(super) fn stepped_functions(stmts: &[Stmt]) -> Stepped {
    let mut calls: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    let mut waits: HashSet<Symbol> = HashSet::new();
    let mut launched: HashSet<Symbol> = HashSet::new();
    for_each_node(stmts, |node| {
        if let Visited::Stmt(
            Stmt::LaunchTask { function, .. } | Stmt::LaunchTaskWithHandle { function, .. },
        ) = node
        {
            launched.insert(*function);
        }
    });
    for stmt in stmts {
        let Stmt::FunctionDef {
            name,
            body,
            is_native: false,
            ..
        } = stmt
        else {
            continue;
        };
        let mut called = HashSet::new();
        let mut direct = false;
        for_each_node(body, |node| match node {
            Visited::Stmt(s) if super::is_concurrency_stmt(s) => direct = true,
            Visited::Stmt(Stmt::Call { function, .. })
            | Visited::Expr(Expr::Call { function, .. }) => {
                called.insert(*function);
            }
            _ => {}
        });
        if direct {
            waits.insert(*name);
        }
        calls.insert(*name, called);
    }
    loop {
        let reached: Vec<Symbol> = calls
            .iter()
            .filter(|(f, called)| !waits.contains(*f) && called.iter().any(|c| waits.contains(c)))
            .map(|(f, _)| *f)
            .collect();
        if reached.is_empty() {
            break;
        }
        waits.extend(reached);
    }
    launched.retain(|f| calls.contains_key(f));
    Stepped { waits, launched }
}

/// The C name of a function's step and frame.
fn step_of(function: Symbol, ctx: &CContext) -> String {
    escape_c_ident(ctx.interner.resolve(function))
}

/// Close a state: save `pc` and return to the scheduler, then open the state the task resumes in.
fn resume(ctx: &mut CContext, output: &mut String, pad: &str) {
    let pc = ctx
        .stepped
        .as_mut()
        .expect("a stepped function")
        .resume_point();
    writeln!(output, "{pad}fr->__hdr.pc = {pc}; return LOGOS_STEP_YIELD;").unwrap();
    writeln!(output, "{pad}case {pc}: ;").unwrap();
}

/// A call's arguments packed as the callee's parameters take them, one scalar cell each; `None` when one
/// is a value no cell carries.
fn packed_args(function: Symbol, args: &[&Expr], ctx: &CContext) -> Option<Vec<String>> {
    let params = ctx.task_params.get(&function).cloned().unwrap_or_default();
    args.iter()
        .enumerate()
        .map(|(i, e)| {
            task_arg_pack(
                params.get(i).unwrap_or(&CType::Int64),
                &codegen_expr(e, ctx),
            )
        })
        .collect()
}

/// The argument array a spawn or a push passes: a block-local array, or a null pointer for none.
fn args_array(packed: &[String]) -> (String, String) {
    if packed.is_empty() {
        (String::new(), "(const long long *)0".to_string())
    } else {
        (
            format!("const long long __args[] = {{{}}}; ", packed.join(", ")),
            "__args".to_string(),
        )
    }
}

/// Lower `stmt` if it is one the stepped tier lowers its own way — a scheduling point, a call to a function
/// that waits, a `Return` — answering whether it did.
pub(super) fn scheduling_point(
    stmt: &Stmt,
    ctx: &mut CContext,
    output: &mut String,
    indent: usize,
) -> bool {
    let pad = "    ".repeat(indent);
    if let Stmt::While { cond, body, .. } = stmt {
        if calls_a_waiter(cond, ctx) {
            // The condition is read afresh every pass and calls a function that waits: each pass runs the
            // call first, then tests.
            writeln!(output, "{pad}while (1) {{").unwrap();
            let settled = hoist_waiting_calls(&[*cond], ctx, output, &format!("{pad}    "));
            writeln!(
                output,
                "{pad}    if (!{}) break;",
                super::emit::c_truthy(cond, ctx)
            )
            .unwrap();
            forget(&settled, ctx);
            ctx.enter_scope();
            ctx.in_loop(|ctx| {
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            });
            ctx.exit_scope();
            writeln!(output, "{pad}}}").unwrap();
            return true;
        }
    }
    // A call to a function that waits inside one of the statement's own expressions: every call they make up
    // to the last such one runs first, in evaluation order, into a cell, and the statement reads the cells.
    let own = stmt_expr_children(stmt);
    let settled = if own.iter().any(|e| calls_a_waiter(e, ctx)) {
        hoist_waiting_calls(&own, ctx, output, &pad)
    } else {
        Vec::new()
    };
    let lowered = lower_stepped(stmt, ctx, output, &pad, indent);
    if !lowered && !settled.is_empty() {
        codegen_stmt(stmt, ctx, output, indent);
    }
    forget(&settled, ctx);
    lowered || !settled.is_empty()
}

/// The statements the stepped tier lowers its own way — the scheduling points, a call statement to a
/// function that waits, `Return` — answering whether `stmt` was one.
fn lower_stepped(
    stmt: &Stmt,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
    indent: usize,
) -> bool {
    match stmt {
        Stmt::CreatePipe {
            var,
            capacity,
            element_type,
        } => {
            let element = resolve_type_expr_with_registry(
                &crate::ast::stmt::TypeExpr::Named(*element_type),
                ctx.interner,
                Some(ctx.registry),
            );
            if task_arg_pack(&element, "0").is_none() {
                output.push_str(&c_refuse_expr(&format!(
                    "a pipe of {}, which a pipe's scalar cell cannot carry",
                    ctx.interner.resolve(*element_type)
                )));
                return true;
            }
            let cap: i64 = capacity.map(|c| c as i64).unwrap_or(-1);
            let chan = format!("(int64_t)(intptr_t)logos_op_new_chan(logos_current, {cap}LL)");
            emit_binding(
                *var,
                CType::Pipe(Box::new(element)),
                &chan,
                ctx,
                output,
                pad,
            );
            resume(ctx, output, pad);
        }
        Stmt::SendPipe { value, pipe } => {
            let packed = pipe_packed_value(value, pipe, ctx);
            let chan = codegen_expr(pipe, ctx);
            writeln!(
                output,
                "{pad}logos_op_send(logos_current, (logos_chan *)(intptr_t)({chan}), {packed});"
            )
            .unwrap();
            resume(ctx, output, pad);
        }
        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => {
            let packed = pipe_packed_value(value, pipe, ctx);
            let chan = codegen_expr(pipe, ctx);
            writeln!(output, "{pad}logos_op_try_send(logos_current, (logos_chan *)(intptr_t)({chan}), {packed});").unwrap();
            resume(ctx, output, pad);
            if let Some(res) = result {
                emit_binding(
                    *res,
                    CType::Bool,
                    "logos_current->resume != 0",
                    ctx,
                    output,
                    pad,
                );
            }
        }
        Stmt::ReceivePipe { var, pipe } | Stmt::TryReceivePipe { var, pipe } => {
            let op = if matches!(stmt, Stmt::ReceivePipe { .. }) {
                "logos_op_recv"
            } else {
                "logos_op_try_recv"
            };
            let chan = codegen_expr(pipe, ctx);
            let read = CType::Opt(Box::new(pipe_element(pipe, ctx)));
            writeln!(
                output,
                "{pad}{op}(logos_current, (logos_chan *)(intptr_t)({chan}));"
            )
            .unwrap();
            resume(ctx, output, pad);
            emit_binding(*var, read, "logos_resumed()", ctx, output, pad);
        }
        Stmt::CloseChannel { channel } => {
            let chan = codegen_expr(channel, ctx);
            writeln!(
                output,
                "{pad}logos_op_close(logos_current, (logos_chan *)(intptr_t)({chan}));"
            )
            .unwrap();
            resume(ctx, output, pad);
        }
        Stmt::StopTask { handle } => {
            let h = codegen_expr(handle, ctx);
            writeln!(
                output,
                "{pad}logos_op_abort(logos_current, (logos_task *)(intptr_t)({h}));"
            )
            .unwrap();
            resume(ctx, output, pad);
        }
        Stmt::AwaitTask { handle, result } => {
            let h = codegen_expr(handle, ctx);
            writeln!(
                output,
                "{pad}logos_op_await(logos_current, (logos_task *)(intptr_t)({h}));"
            )
            .unwrap();
            resume(ctx, output, pad);
            if let Some(r) = result {
                // A task answers `nothing`, on the interpreter and here.
                emit_binding(
                    *r,
                    CType::Opt(Box::new(CType::Int64)),
                    "(logos_opt){0, 0}",
                    ctx,
                    output,
                    pad,
                );
            }
        }
        Stmt::Sleep { milliseconds } => {
            // A sleep of no time is no scheduling point: its `case` is reached by falling through.
            let ticks = ctx.temp("__ticks", CType::Int64);
            let head = ctx.decl_head(&CType::Int64, &ticks);
            writeln!(
                output,
                "{pad}{head} = (int64_t)({});",
                select_timeout_ticks_c(milliseconds, ctx)
            )
            .unwrap();
            let pc = ctx
                .stepped
                .as_mut()
                .expect("a stepped function")
                .resume_point();
            writeln!(output, "{pad}if ({ticks} > 0) {{").unwrap();
            writeln!(
                output,
                "{pad}    logos_op_sleep(logos_current, (long long)({ticks}));"
            )
            .unwrap();
            writeln!(
                output,
                "{pad}    fr->__hdr.pc = {pc}; return LOGOS_STEP_YIELD;"
            )
            .unwrap();
            writeln!(output, "{pad}}}").unwrap();
            writeln!(output, "{pad}case {pc}: ;").unwrap();
        }
        Stmt::LaunchTask { function, args } | Stmt::LaunchTaskWithHandle { function, args, .. } => {
            let Some(packed) = packed_args(*function, args, ctx) else {
                output.push_str(&c_refuse_expr(
                    "a task launched with a value a task's scalars cannot carry",
                ));
                return true;
            };
            let step = step_of(*function, ctx);
            let (decl, argv) = args_array(&packed);
            let spawn = format!(
                "(int64_t)logos_spawn(logos_step_{step}, sizeof(logos_frame_{step}), {}, {argv})",
                packed.len()
            );
            writeln!(output, "{pad}{{").unwrap();
            if !decl.is_empty() {
                writeln!(output, "{pad}    {decl}").unwrap();
            }
            match stmt {
                Stmt::LaunchTaskWithHandle { handle, .. } => emit_binding(
                    *handle,
                    CType::Int64,
                    &spawn,
                    ctx,
                    output,
                    &format!("{pad}    "),
                ),
                _ => writeln!(output, "{pad}    (void){spawn};").unwrap(),
            }
            writeln!(output, "{pad}}}").unwrap();
            resume(ctx, output, pad);
        }
        Stmt::Select { branches } => lower_select(branches, ctx, output, indent),
        // A `Return` inside a zone leaves the zone, which the ordinary lowering does.
        Stmt::Return { .. } if ctx.in_a_zone() => return false,
        Stmt::Return { value } => {
            let returns = ctx
                .stepped
                .as_ref()
                .expect("a stepped function")
                .returns
                .clone();
            let result = match value {
                None => "0LL".to_string(),
                Some(v) => {
                    let shape = if returns == CType::Void {
                        super::types::infer_expr_type(v, ctx)
                    } else {
                        returns
                    };
                    let rendered = as_returned(v, codegen_expr(v, ctx), ctx);
                    task_arg_pack(&shape, &rendered).unwrap_or_else(|| {
                        c_refuse_expr(
                            "a function that waits returning a value a scalar cell cannot carry",
                        )
                    })
                }
            };
            writeln!(output, "{pad}{{").unwrap();
            writeln!(output, "{pad}    long long __result = {result};").unwrap();
            writeln!(
                output,
                "{pad}    if (fr->__hdr.caller == 0) return LOGOS_STEP_DONE;"
            )
            .unwrap();
            writeln!(output, "{pad}    logos_call_return(__result);").unwrap();
            writeln!(output, "{pad}    return LOGOS_STEP_CALL;").unwrap();
            writeln!(output, "{pad}}}").unwrap();
        }
        Stmt::Call { function, args } if ctx.stepped_fns.contains(function) => {
            push_call(*function, args, false, ctx, output, pad);
        }
        _ => return false,
    }
    true
}

/// Whether `e` calls a function that waits, apart from calls already run ahead of their statement.
fn calls_a_waiter(e: &Expr, ctx: &CContext) -> bool {
    if ctx.precomputed.borrow().contains_key(&node_key(e)) {
        return false;
    }
    matches!(e, Expr::Call { function, .. } if ctx.stepped_fns.contains(function))
        || expr_children(e).into_iter().any(|c| calls_a_waiter(c, ctx))
}

/// A call's identity while its statement is lowered: the address of its node.
fn node_key(e: &Expr) -> usize {
    e as *const Expr as usize
}

/// Run, in evaluation order, every call `exprs` make up to the last call to a function that waits — each into
/// a cell of the frame — and record each cell as its call's value for the lowering that follows. Answers the
/// calls settled.
fn hoist_waiting_calls(
    exprs: &[&Expr],
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) -> Vec<usize> {
    let mut calls: Vec<&Expr> = Vec::new();
    for e in exprs {
        calls_in_order(e, ctx, &mut calls);
    }
    let waits =
        |c: &&Expr| matches!(c, Expr::Call { function, .. } if ctx.stepped_fns.contains(function));
    let Some(last) = calls.iter().rposition(waits) else {
        return Vec::new();
    };
    let mut settled = Vec::new();
    for call in &calls[..=last] {
        let Expr::Call { function, args } = call else {
            continue;
        };
        let returns = ctx.funcs.get(function).cloned().unwrap_or(CType::Int64);
        let value = if ctx.stepped_fns.contains(function) {
            let Some(cell) = push_call(*function, args, true, ctx, output, pad) else {
                continue;
            };
            task_arg_unpack(&returns, &cell, ctx.interner)
        } else {
            let cell = ctx.temp("__called", returns.clone());
            writeln!(
                output,
                "{pad}{} = {};",
                ctx.decl_head(&returns, &cell),
                codegen_expr(call, ctx)
            )
            .unwrap();
            cell
        };
        ctx.precomputed.borrow_mut().insert(node_key(call), value);
        settled.push(node_key(call));
    }
    settled
}

/// The calls an expression makes, each after the calls in its own arguments, left to right — the order they
/// run in. A call already settled is a value; a closure's body runs later.
fn calls_in_order<'e, 'a>(e: &'e Expr<'a>, ctx: &CContext, out: &mut Vec<&'e Expr<'a>>) {
    if ctx.precomputed.borrow().contains_key(&node_key(e)) {
        return;
    }
    for child in expr_children(e) {
        calls_in_order(child, ctx, out);
    }
    if matches!(e, Expr::Call { .. }) {
        out.push(e);
    }
}

fn forget(settled: &[usize], ctx: &mut CContext) {
    for key in settled {
        ctx.precomputed.borrow_mut().remove(key);
    }
}

/// Push a frame for `function` with `args`, its result bound for a fresh cell when `wants_result`, and open
/// the state the caller resumes in once the callee returns. Answers the result cell, or `None` when the call
/// was refused or its result is not wanted.
fn push_call(
    function: Symbol,
    args: &[&Expr],
    wants_result: bool,
    ctx: &mut CContext,
    output: &mut String,
    pad: &str,
) -> Option<String> {
    let settled = hoist_waiting_calls(args, ctx, output, pad);
    let packed = packed_args(function, args, ctx);
    forget(&settled, ctx);
    let Some(packed) = packed else {
        output.push_str(&c_refuse_expr(
            "a call passing a value a scalar cell cannot carry to a function that waits",
        ));
        return None;
    };
    let cell = wants_result.then(|| ctx.temp("__returned", CType::Int64));
    let slot = cell.as_ref().map_or("(long long *)0".to_string(), |c| {
        format!("(long long *)&{c}")
    });
    let step = step_of(function, ctx);
    let (decl, argv) = args_array(&packed);
    let pc = ctx
        .stepped
        .as_mut()
        .expect("a stepped function")
        .resume_point();
    writeln!(output, "{pad}fr->__hdr.pc = {pc};").unwrap();
    writeln!(
        output,
        "{pad}{{ {decl}logos_call_push(logos_step_{step}, sizeof(logos_frame_{step}), {slot}, {}, {argv}); }}",
        packed.len()
    )
    .unwrap();
    writeln!(output, "{pad}return LOGOS_STEP_CALL;").unwrap();
    writeln!(output, "{pad}case {pc}: ;").unwrap();
    cell
}

/// `Await the first of:` — the arms go to the core in declaration order; it resolves a ready arm (the seed
/// breaking a tie) or parks the task on all of them. On resume the task's `selected_arm` names the winner,
/// and a receive arm binds what it read before its body runs.
fn lower_select(branches: &[SelectBranch], ctx: &mut CContext, output: &mut String, indent: usize) {
    let pad = "    ".repeat(indent);
    let mut kinds = Vec::new();
    let mut chans = Vec::new();
    let mut ticks = Vec::new();
    for branch in branches {
        match branch {
            SelectBranch::Receive { pipe, .. } => {
                kinds.push("LOGOS_ARM_RECV".to_string());
                chans.push(format!(
                    "(logos_chan *)(intptr_t)({})",
                    codegen_expr(pipe, ctx)
                ));
                ticks.push("0LL".to_string());
            }
            SelectBranch::Timeout { milliseconds, .. } => {
                kinds.push("LOGOS_ARM_AFTER".to_string());
                chans.push("(logos_chan *)0".to_string());
                ticks.push(format!(
                    "(long long)({})",
                    select_timeout_ticks_c(milliseconds, ctx)
                ));
            }
        }
    }
    writeln!(output, "{pad}{{").unwrap();
    writeln!(
        output,
        "{pad}    const int __k[] = {{{}}};",
        kinds.join(", ")
    )
    .unwrap();
    writeln!(
        output,
        "{pad}    logos_chan *const __c[] = {{{}}};",
        chans.join(", ")
    )
    .unwrap();
    writeln!(
        output,
        "{pad}    const long long __t[] = {{{}}};",
        ticks.join(", ")
    )
    .unwrap();
    writeln!(
        output,
        "{pad}    logos_op_select(logos_current, {}, __k, __c, __t);",
        branches.len()
    )
    .unwrap();
    writeln!(output, "{pad}}}").unwrap();
    resume(ctx, output, &pad);
    for (i, branch) in branches.iter().enumerate() {
        let kw = if i == 0 { "if" } else { "} else if" };
        writeln!(output, "{pad}{kw} (logos_current->selected_arm == {i}) {{").unwrap();
        ctx.enter_scope();
        let body = match branch {
            SelectBranch::Receive { var, pipe, body } => {
                let read = CType::Opt(Box::new(pipe_element(pipe, ctx)));
                emit_binding(
                    *var,
                    read,
                    "logos_resumed()",
                    ctx,
                    output,
                    &format!("{pad}    "),
                );
                *body
            }
            SelectBranch::Timeout { body, .. } => *body,
        };
        for s in body {
            codegen_stmt(s, ctx, output, indent + 1);
        }
        ctx.exit_scope();
    }
    if !branches.is_empty() {
        writeln!(output, "{pad}}}").unwrap();
    }
}

/// A stepped function's frame and step: `params` are its parameters with their C types, `returns` the type its
/// `Return` hands back.
pub(super) fn emit_step(
    cname: &str,
    params: &[(Symbol, CType)],
    returns: CType,
    body: &[Stmt],
    ctx: &mut CContext,
) -> (String, String) {
    let saved_vars = ctx.vars.clone();
    let saved_renamed = ctx.renamed.clone();
    let mut changed = super::values::mutated_roots(body, &ctx.fn_inout);
    changed.extend(ctx.changed_statics.iter().copied());
    let saved_changed = std::mem::replace(&mut ctx.mutated, changed);
    ctx.stepped = Some(Frame::returning(returns));
    ctx.enter_scope();
    let mut code = String::new();
    for (i, (p, ty)) in params.iter().enumerate() {
        let field = ctx.bind_in_place(*p, ty.clone());
        let arrived = task_arg_unpack(ty, &format!("fr->__arg{i}"), ctx.interner);
        // A parameter the body changes in place is the body's own copy.
        let arrived = if *ty == CType::String {
            if super::emit::builds_text(*p, ctx) {
                format!("logos_text_builder({arrived}, 0)")
            } else {
                arrived
            }
        } else if ctx.mutated.contains(p) {
            super::values::copy_value(ty, &arrived, ctx)
        } else {
            arrived
        };
        writeln!(code, "        {field} = {arrived};").unwrap();
    }
    for s in body {
        codegen_stmt(s, ctx, &mut code, 2);
    }
    ctx.exit_scope();
    ctx.mutated = saved_changed;
    let frame = ctx.stepped.take().expect("the frame just lowered");
    ctx.vars = saved_vars;
    ctx.renamed = saved_renamed;

    let mut typedef = String::new();
    writeln!(typedef, "typedef struct logos_frame_{cname} {{").unwrap();
    writeln!(typedef, "    logos_frame_hdr __hdr;").unwrap();
    for i in 0..params.len() {
        writeln!(typedef, "    long long __arg{i};").unwrap();
    }
    for (name, ty) in &frame.fields {
        writeln!(typedef, "    {};", c_param_decl(ty, name, ctx.interner)).unwrap();
    }
    writeln!(typedef, "}} logos_frame_{cname};").unwrap();

    let mut step = String::new();
    writeln!(step, "static int logos_step_{cname}(void *fp) {{").unwrap();
    writeln!(
        step,
        "    logos_frame_{cname} *fr = (logos_frame_{cname} *)fp;"
    )
    .unwrap();
    writeln!(step, "    LOGOS_STEP_ENTRY").unwrap();
    writeln!(step, "    switch (fr->__hdr.pc) {{").unwrap();
    writeln!(step, "    case 0: ;").unwrap();
    step.push_str(&code);
    writeln!(step, "    }}").unwrap();
    // Falling off the end is a value-less `Return`: a task's root frame completes, a callee pops.
    writeln!(
        step,
        "    if (fr->__hdr.caller == 0) return LOGOS_STEP_DONE;"
    )
    .unwrap();
    writeln!(step, "    logos_call_return(0LL);").unwrap();
    writeln!(step, "    return LOGOS_STEP_CALL;").unwrap();
    writeln!(step, "}}").unwrap();
    (typedef, step)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_c/stepped/tests.rs"]
mod tests;
