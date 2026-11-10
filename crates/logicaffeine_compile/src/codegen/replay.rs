//! The deterministic-replay lowering of the scheduler's operations (see `codegen::deterministic_replay_enabled`).
//!
//! A program that runs on the scheduler compiles, for replay, onto `logicaffeine_system::replay`, the
//! interpreter's scheduler ported: every launch, pipe operation, timer and select becomes one awaited
//! operation on that executor, performed where and in the order the interpreter performs it — the
//! arguments of a launch are evaluated before the task starts, a select's arms in declaration order —
//! so the binary makes the interpreter's scheduling decisions one for one. `Concurrent`/`Parallel`
//! blocks run their statements in order, as the interpreter runs them.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{Expr, Literal, SelectBranch, Stmt};
use crate::intern::{Interner, Symbol};

use super::context::{RefinementContext, VariableCapabilities};
use super::expr::{
    call_argument, called_function_path, codegen_expr_boxed_with_types_oracle,
    codegen_expr_with_async, ExprCtx,
};
use super::stmt::codegen_stmt;
use super::types::map_type_to_rust;

const REPLAY: &str = "logicaffeine_system::replay";

/// Everything `codegen_stmt` threads through, so a lowered block body recurses into it unchanged.
pub(super) struct Scope<'s, 'a> {
    pub interner: &'s Interner,
    pub indent: usize,
    pub mutable_vars: &'s HashSet<Symbol>,
    pub ctx: &'s mut RefinementContext<'a>,
    pub lww_fields: &'s HashSet<(String, String)>,
    pub mv_fields: &'s HashSet<(String, String)>,
    pub synced_vars: &'s mut HashSet<Symbol>,
    pub var_caps: &'s HashMap<Symbol, VariableCapabilities>,
    pub async_functions: &'s HashSet<Symbol>,
    pub pipe_vars: &'s HashSet<Symbol>,
    pub boxed_fields: &'s HashSet<(String, String, String)>,
    pub registry: &'s TypeRegistry,
    pub type_env: &'s crate::analysis::types::TypeEnv,
}

impl<'s, 'a> Scope<'s, 'a> {
    fn pad(&self, extra: usize) -> String {
        "    ".repeat(self.indent + extra)
    }

    fn expr(&self, e: &Expr) -> String {
        codegen_expr_with_async(
            e,
            self.interner,
            self.synced_vars,
            self.async_functions,
            self.ctx.get_variable_types(),
        )
    }

    fn value(&self, e: &Expr) -> String {
        let s = codegen_expr_boxed_with_types_oracle(
            e,
            self.interner,
            self.synced_vars,
            self.boxed_fields,
            self.registry,
            self.async_functions,
            self.ctx.get_string_vars(),
            self.ctx.get_variable_types(),
            self.ctx.get_fast_div(),
            self.ctx.oracle(),
        );
        // A named value stays the sender's: the pipe carries a copy of it, as the interpreter's does.
        if matches!(e, Expr::Identifier(_) | Expr::FieldAccess { .. }) {
            format!("({s}).clone()")
        } else {
            s
        }
    }

    fn block(&mut self, stmts: &[&Stmt<'a>], extra: usize, out: &mut String) {
        for stmt in stmts {
            out.push_str(&codegen_stmt(
                stmt,
                self.interner,
                self.indent + extra,
                self.mutable_vars,
                self.ctx,
                self.lww_fields,
                self.mv_fields,
                self.synced_vars,
                self.var_caps,
                self.async_functions,
                self.pipe_vars,
                self.boxed_fields,
                self.registry,
                self.type_env,
            ));
        }
    }

    /// The element type a pipe expression carries, when the pipe is a named local this lowering typed.
    fn pipe_element(&self, pipe: &Expr) -> Option<String> {
        let Expr::Identifier(sym) = pipe else {
            return None;
        };
        let ty = self.ctx.get_variable_types().get(sym)?;
        ty.strip_prefix(&format!("{REPLAY}::Pipe<"))?
            .strip_suffix('>')
            .map(str::to_string)
    }

    fn bind(&mut self, var: Symbol, rust_type: Option<String>) {
        if let Some(ty) = rust_type {
            self.ctx.register_variable_type(var, ty);
        }
    }

    /// A duration as the executor reads it: a `Duration` value as itself, anything else as the
    /// `Int` tick count it evaluates to.
    fn ticks(&self, e: &Expr) -> String {
        let s = self.expr(e);
        let is_duration = super::types::infer_logos_type(e, self.interner, self.ctx.get_variable_types())
            == crate::analysis::types::LogosType::Duration;
        if is_duration {
            s
        } else {
            format!("({s}) as i64")
        }
    }
}

/// The deterministic-replay lowering of `stmt`, or `None` when `stmt` is not a scheduler operation.
pub(super) fn replay_stmt<'a>(stmt: &Stmt<'a>, scope: &mut Scope<'_, 'a>) -> Option<String> {
    let names = RustNames::new(scope.interner);
    let pad = scope.pad(0);
    let mut out = String::new();
    match stmt {
        Stmt::LaunchTask { function, args } => {
            launch(stmt, *function, args, None, scope, &names, &mut out);
        }
        Stmt::LaunchTaskWithHandle {
            handle,
            function,
            args,
        } => {
            launch(
                stmt,
                *function,
                args,
                Some(*handle),
                scope,
                &names,
                &mut out,
            );
            scope.bind(*handle, Some(format!("{REPLAY}::Task")));
        }
        Stmt::StopTask { handle } => {
            writeln!(out, "{pad}{}.stop().await;", scope.expr(handle)).unwrap();
        }
        Stmt::AwaitTask { handle, result } => {
            let h = scope.expr(handle);
            match result {
                Some(r) => {
                    writeln!(out, "{pad}let {} = {h}.join().await;", names.ident(*r)).unwrap();
                    scope.bind(*r, Some("logicaffeine_data::Value".to_string()));
                }
                None => writeln!(out, "{pad}{h}.join().await;").unwrap(),
            }
        }
        Stmt::CreatePipe {
            var,
            element_type,
            capacity,
        } => {
            let element = map_type_to_rust(scope.interner.resolve(*element_type));
            let capacity = match capacity {
                Some(c) => format!("Some({c})"),
                None => "None".to_string(),
            };
            writeln!(
                out,
                "{pad}let {} = {REPLAY}::Pipe::<{element}>::open({capacity}).await;",
                names.ident(*var)
            )
            .unwrap();
            scope.bind(*var, Some(format!("{REPLAY}::Pipe<{element}>")));
        }
        Stmt::SendPipe { value, pipe } => {
            let v = scope.value(value);
            writeln!(out, "{pad}{}.send({v}).await;", scope.expr(pipe)).unwrap();
        }
        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => {
            let v = scope.value(value);
            let call = format!("{}.try_send({v}).await", scope.expr(pipe));
            match result {
                Some(r) => {
                    writeln!(out, "{pad}let {} = {call};", names.ident(*r)).unwrap();
                    scope.bind(*r, Some("bool".to_string()));
                }
                None => writeln!(out, "{pad}{call};").unwrap(),
            }
        }
        Stmt::ReceivePipe { var, pipe } => {
            let element = scope.pipe_element(pipe);
            writeln!(
                out,
                "{pad}let {} = {}.recv().await;",
                names.ident(*var),
                scope.expr(pipe)
            )
            .unwrap();
            scope.bind(*var, element);
        }
        Stmt::TryReceivePipe { var, pipe } => {
            let element = scope.pipe_element(pipe);
            writeln!(
                out,
                "{pad}let {} = {}.try_recv().await;",
                names.ident(*var),
                scope.expr(pipe)
            )
            .unwrap();
            scope.bind(*var, element.map(|e| format!("Option<{e}>")));
        }
        Stmt::CloseChannel { channel } => {
            writeln!(out, "{pad}{}.close().await;", scope.expr(channel)).unwrap();
        }
        Stmt::Sleep { milliseconds } => {
            writeln!(
                out,
                "{pad}{REPLAY}::sleep({}).await;",
                scope.ticks(milliseconds)
            )
            .unwrap();
        }
        Stmt::Select { branches } => select(branches, scope, &names, &mut out),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            let tasks: Vec<&Stmt<'a>> = tasks.iter().collect();
            scope.block(&tasks, 0, &mut out);
        }
        _ => return None,
    }
    Some(out)
}

/// `Launch a task to f with a, b`: the arguments are evaluated here, in the launching task, as the
/// interpreter evaluates them; the task then calls `f` with the values it owns.
fn launch<'a>(
    stmt: &Stmt<'a>,
    function: Symbol,
    args: &[&'a Expr<'a>],
    handle: Option<Symbol>,
    scope: &mut Scope<'_, 'a>,
    names: &RustNames,
    out: &mut String,
) {
    let pad = scope.pad(0);
    let borrowed = [
        super::FnRole::Borrow,
        super::FnRole::MutBorrow,
        super::FnRole::ValueMutable,
    ]
    .into_iter()
    .any(|role| !super::fn_role_indices(function, role).is_empty());
    let raw = names.raw(function).to_string();
    if borrowed {
        writeln!(
            out,
            "{pad}compile_error!(\"a launched task owns its arguments, and `{raw}` borrows a parameter\");"
        )
        .unwrap();
        return;
    }
    let ecx = ExprCtx {
        async_functions: scope.async_functions,
        variable_types: scope.ctx.get_variable_types(),
        ..ExprCtx::bare(scope.interner, scope.synced_vars)
    };
    let values: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, a)| call_argument(function, i, a, &ecx))
        .collect();
    let path = called_function_path(
        function,
        names.ident(function),
        scope.ctx.get_variable_types(),
    );
    let fish = super::instantiation::for_stmt(stmt);
    let wait = super::expr::Wait::of(function, scope.async_functions);
    let binding = match handle {
        Some(h) => format!("let {} = ", names.ident(h)),
        None => String::new(),
    };
    writeln!(out, "{pad}{binding}{REPLAY}::spawn({{").unwrap();
    let params: Vec<String> = (0..values.len())
        .map(|i| format!("__logos_arg{i}"))
        .collect();
    for (param, value) in params.iter().zip(&values) {
        writeln!(out, "{pad}    let {param} = {value};").unwrap();
    }
    let call = wait.around(format!("{path}{fish}({})", params.join(", ")));
    writeln!(out, "{pad}    async move {{ {call}; }}").unwrap();
    writeln!(out, "{pad}}}).await;").unwrap();
}

/// `Await the first of:` — the arms are evaluated in declaration order, the winner among the ready
/// arms is drawn from the run's seeded chooser in the same step the task yields (where the scheduler
/// draws it), and the winning arm's body runs with what it delivered.
fn select<'a>(
    branches: &[SelectBranch<'a>],
    scope: &mut Scope<'_, 'a>,
    names: &RustNames,
    out: &mut String,
) {
    let pad = scope.pad(0);
    writeln!(out, "{pad}{{").unwrap();
    let mut arms = Vec::with_capacity(branches.len());
    for (i, branch) in branches.iter().enumerate() {
        match branch {
            SelectBranch::Receive { pipe, .. } => {
                writeln!(out, "{pad}    let __logos_pipe{i} = {};", scope.expr(pipe)).unwrap();
                arms.push(format!("__logos_pipe{i}.arm()"));
            }
            SelectBranch::Timeout { milliseconds, .. } => {
                let ticks = match milliseconds {
                    Expr::Literal(Literal::Span { months, days }) => {
                        format!(
                            "{}i64",
                            (((*months as i64) * 30 + (*days as i64)) * 86_400).max(0)
                        )
                    }
                    other => scope.ticks(other),
                };
                arms.push(format!("{REPLAY}::Arm::after({ticks})"));
            }
        }
    }
    writeln!(out, "{pad}    let __logos_arms = [{}];", arms.join(", ")).unwrap();
    writeln!(
        out,
        "{pad}    let __logos_ready: Vec<usize> = {REPLAY}::ready_arms(&__logos_arms);"
    )
    .unwrap();
    writeln!(
        out,
        "{pad}    let __logos_pick = (!__logos_ready.is_empty()).then(|| __logos_ready[logicaffeine_system::concurrency::seeded_pick(__logos_ready.len())]);"
    )
    .unwrap();
    writeln!(out, "{pad}    let (__logos_arm, __logos_got) = {REPLAY}::select(&__logos_arms, __logos_pick).await;").unwrap();
    writeln!(out, "{pad}    match __logos_arm {{").unwrap();
    for (i, branch) in branches.iter().enumerate() {
        writeln!(out, "{pad}        {i} => {{").unwrap();
        match branch {
            SelectBranch::Receive { var, pipe, body } => {
                let element = scope.pipe_element(pipe);
                writeln!(
                    out,
                    "{pad}            let {} = __logos_pipe{i}.received(__logos_got);",
                    names.ident(*var)
                )
                .unwrap();
                scope.bind(*var, element);
                let body: Vec<&Stmt<'a>> = body.iter().collect();
                scope.block(&body, 3, out);
            }
            SelectBranch::Timeout { body, .. } => {
                let body: Vec<&Stmt<'a>> = body.iter().collect();
                scope.block(&body, 3, out);
            }
        }
        writeln!(out, "{pad}        }}").unwrap();
    }
    writeln!(out, "{pad}        _ => unreachable!(),").unwrap();
    writeln!(out, "{pad}    }}").unwrap();
    writeln!(out, "{pad}}}").unwrap();
}

/// A parameter's Rust type when replay carries it as a pipe: one declared `Pipe of T`, or one the
/// body sends to, receives from, closes or selects on — declared by its element type
/// (`## To produce (ch: Int)`), as the interpreter, which types nothing here, accepts.
pub(super) fn pipe_param_type(
    param: Symbol,
    declared: &crate::ast::stmt::TypeExpr,
    used_as_pipe: &HashSet<Symbol>,
    interner: &Interner,
) -> Option<String> {
    use crate::ast::stmt::TypeExpr;
    if !super::deterministic_replay_enabled() {
        return None;
    }
    let element = match declared {
        TypeExpr::Generic { base, params }
            if interner.resolve(*base) == "Pipe" && params.len() == 1 =>
        {
            super::types::codegen_type_expr(&params[0], interner)
        }
        _ if used_as_pipe.contains(&param) => super::types::codegen_type_expr(declared, interner),
        _ => return None,
    };
    Some(format!("{REPLAY}::Pipe<{element}>"))
}

/// The names a body uses as pipes: every pipe it sends to, receives from, closes or selects on.
pub(super) fn pipes_used(body: &[Stmt]) -> HashSet<Symbol> {
    fn named(e: &Expr, out: &mut HashSet<Symbol>) {
        if let Expr::Identifier(sym) = e {
            out.insert(*sym);
        }
    }
    fn walk(stmts: &[Stmt], out: &mut HashSet<Symbol>) {
        for stmt in stmts {
            match stmt {
                Stmt::SendPipe { pipe, .. }
                | Stmt::TrySendPipe { pipe, .. }
                | Stmt::ReceivePipe { pipe, .. }
                | Stmt::TryReceivePipe { pipe, .. } => named(pipe, out),
                Stmt::CloseChannel { channel } => named(channel, out),
                Stmt::Select { branches } => {
                    for branch in branches.iter() {
                        match branch {
                            SelectBranch::Receive { pipe, body, .. } => {
                                named(pipe, out);
                                walk(body, out);
                            }
                            SelectBranch::Timeout { body, .. } => walk(body, out),
                        }
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, out);
                    if let Some(b) = else_block {
                        walk(b, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    walk(body, out)
                }
                Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => walk(tasks, out),
                Stmt::Inspect { arms, .. } => {
                    for arm in arms.iter() {
                        walk(arm.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(body, &mut out);
    out
}
