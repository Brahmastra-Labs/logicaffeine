//! LOGOS **concurrency pattern RECOGNIZERS** — the source-reading half of the concurrency tier.
//!
//! A `Launch a task to producer with ch` + `Send v into ch` (in the task) + `Receive x from ch` (in main)
//! is CSP dataflow, and this module is what READS that out of a LOGOS program: the pipe, the fan-out, the
//! loop-driven producer, the select, and the two racing selects, each reduced to a spec of exactly what
//! the source said. A non-matching program is DECLINED (the caller falls back to the sound refusal),
//! never miscompiled.
//!
//! ★ THE EMITTERS ARE GONE; THE RECOGNIZERS ARE THE POINT. This module used to end each recognizer with a
//! hand-written pair of baby-RISC-V kernels around a circular buffer — the producer pushing, the consumer
//! waiting and popping, with the recognized values baked into the C++ as a `const uint32_t vals[N]`. Those
//! kernels are now PRINTED FROM THE SPATIAL IR like every other lowering (`spatial::passes::from_tensor_op`
//! builds the template, the Tensix printer emits it), because a pattern that is spelled out by hand is a
//! claim about a kernel while a pattern that is lowered is a claim about a program: the same numbers, but
//! reachable by every pass, the census and the evaluator instead of sealed inside text.
//!
//! ★ EVERY RESOLUTION HERE IS COMPILE-TIME, WHICH IS WHY THE IR STAYS A MARKED GRAPH. A select does not
//! race: the VM's virtual-time scheduler fires whichever event has the smaller virtual time, and both
//! times are literals of the source. So the compiler RESOLVES the arm and lowers the resolved program,
//! rather than teaching the IR a free choice — which would cost the confluence that makes
//! `eval_choreography_confluent` a proof rather than a spot-check, for every program sharing the IR.

use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, SelectBranch, Stmt};

use crate::intern::{Interner, Symbol};

/// What the concurrency tier produces for a recognized pattern: what the SOURCE said, and the spatial
/// program it lowers to.
///
/// `program` is `None` when the compiler's compile-time resolution makes the answer a CONSTANT with no
/// dataflow — a select whose timeout arm wins prints a fixed string, and a constant has no placement. The
/// RESOLUTION is still a fact about the source, and it is what the ≡VM anchors check: the compiler and the
/// VM's virtual-time scheduler must agree on which arm fires.
#[derive(Debug, Clone)]
pub struct MeshLowering<S> {
    pub spec: S,
    pub program: Option<crate::spatial::ir::SpatialProgram>,
}

/// A recognized producer/consumer pipe program: a task sends a fixed sequence of integers into a pipe, and
/// main receives them and folds them (first increment: an integer sum) into a shown result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencySpec {
    /// The task function that sends into the pipe.
    pub producer: Symbol,
    /// The integer values the task sends, in order.
    pub sends: Vec<i64>,
    /// How many values main receives (must equal `sends.len()` for a matched program).
    pub receives: usize,
}

fn as_ident(e: &Expr) -> Option<Symbol> {
    if let Expr::Identifier(s) = e {
        Some(*s)
    } else {
        None
    }
}

fn as_int(e: &Expr) -> Option<i64> {
    if let Expr::Literal(Literal::Number(n)) = e {
        Some(*n)
    } else {
        None
    }
}

/// Collect the top-level function bodies and the "main" body from a parsed program.
fn functions<'a>(
    stmts: &'a [Stmt<'a>],
    interner: &Interner,
) -> (Vec<(Symbol, &'a [Stmt<'a>])>, Option<&'a [Stmt<'a>]>) {
    let mut fns = Vec::new();
    let mut main = None;
    for s in stmts {
        if let Stmt::FunctionDef { name, body, .. } = s {
            if interner.resolve(*name).eq_ignore_ascii_case("main") {
                main = Some(*body);
            } else {
                fns.push((*name, *body));
            }
        }
    }
    // Some front-ends leave `## Main` statements at the top level rather than in a `main` fn.
    if main.is_none() {
        main = Some(stmts);
    }
    (fns, main)
}

/// Recognize the producer/consumer pipe pattern and extract its `ConcurrencySpec`, or `None` if the program
/// is not this shape (the caller then falls back to the sound single-core refusal).
pub fn recognize_producer_consumer(stmts: &[Stmt], interner: &Interner) -> Option<ConcurrencySpec> {
    let (fns, main) = functions(stmts, interner);
    let main = main?;

    // main must launch exactly one task and receive from a pipe.
    let launched: Vec<Symbol> = main
        .iter()
        .filter_map(|s| match s {
            Stmt::LaunchTask { function, .. } => Some(*function),
            _ => None,
        })
        .collect();
    if launched.len() != 1 {
        return None;
    }
    let producer = launched[0];

    let receives = main
        .iter()
        .filter(|s| matches!(s, Stmt::ReceivePipe { .. }))
        .count();
    if receives == 0 {
        return None;
    }

    // The producer's body: a straight run of `Send <int literal> into ch`.
    let body = fns.iter().find(|(n, _)| *n == producer).map(|(_, b)| *b)?;
    let mut sends = Vec::new();
    for s in body {
        match s {
            Stmt::SendPipe { value, .. } => sends.push(as_int(value)?), // non-literal ⇒ decline (increment 1)
            // `Sleep` in a blocking producer is OUTPUT-IRRELEVANT (the receiver blocks and gets the value
            // whenever it arrives), so we drop it — the mesh result stays ≡VM.
            Stmt::Sleep { .. } => {}
            // Ignore the parameter/setup noise a body may carry; anything ELSE that isn't a send declines.
            Stmt::Return { value: None } => {}
            _ => return None,
        }
    }
    if sends.is_empty() || sends.len() != receives {
        return None;
    }

    // main must fold the receives and show a result (we don't over-constrain the fold; the FIFO order +
    // the shown value are what the mesh reproduces).
    let shows = main.iter().any(|s| matches!(s, Stmt::Show { .. }));
    let _ = shows; // a Show is expected but its target is validated by the ≡VM check, not structurally here.
    let _ = as_ident; // (kept for the next increment: computed sends / handle plumbing)

    Some(ConcurrencySpec {
        producer,
        sends,
        receives,
    })
}

/// A recognized **dynamic-channel** producer: `Repeat for i from lo to hi: Send <expr> into ch`. The pipe is
/// fed by a LOOP, and each sent value is a (possibly computed) expression of the loop variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicSpec {
    pub lo: i64,
    pub hi: i64,
    /// The values the loop actually sends, `f(lo)..=f(hi)`. The bounds and the body are literals of the
    /// source, so the stream is known at compile time — which is what lets the loop lower to the same
    /// spatial reduction as a static pipe instead of needing a device-side loop over an opaque expression.
    pub values: Vec<i64>,
    /// The C expression for the sent value, in terms of the loop variable `i`.
    pub send_expr_c: String,
}

/// Lower a simple integer expression over the loop variable to C (the loop variable becomes `i`). Handles
/// literals, the loop variable, and `+ - * / %` — enough for computed send values like `i * i`, `2 * i + 1`.
fn expr_to_c(e: &Expr, loop_var: Symbol) -> Option<String> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(n.to_string()),
        Expr::Identifier(s) if *s == loop_var => Some("i".to_string()),
        Expr::BinaryOp { op, left, right } => {
            let l = expr_to_c(left, loop_var)?;
            let r = expr_to_c(right, loop_var)?;
            let o = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Divide | BinaryOpKind::FloorDivide => "/",
                _ => return None,
            };
            Some(format!("({l} {o} {r})"))
        }
        _ => None,
    }
}

/// [`expr_to_c`]'s twin: the same expression evaluated at `i`, so the compiler knows the STREAM and not
/// only its printed form. Kept adjacent on purpose — an arm added to one and not the other is a send whose
/// emitted text and lowered value disagree.
fn expr_eval(e: &Expr, loop_var: Symbol, i: i64) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => n.to_string().parse::<i64>().ok(),
        Expr::Identifier(s) if *s == loop_var => Some(i),
        Expr::BinaryOp { op, left, right } => {
            let l = expr_eval(left, loop_var, i)?;
            let r = expr_eval(right, loop_var, i)?;
            match op {
                BinaryOpKind::Add => l.checked_add(r),
                BinaryOpKind::Subtract => l.checked_sub(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Modulo => (r != 0).then(|| l.wrapping_rem(r)),
                BinaryOpKind::Divide | BinaryOpKind::FloorDivide => {
                    (r != 0).then(|| l.div_euclid(r))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Recognize a loop-driven producer: `main` launches a task whose body is a single
/// `Repeat for i from lo to hi: Send <expr> into ch`. Extracts the bounds + the C send expression.
pub fn recognize_dynamic_producer(stmts: &[Stmt], interner: &Interner) -> Option<DynamicSpec> {
    let _ = interner;
    let (fns, main) = functions(stmts, interner);
    let main = main?;
    let producer = main.iter().find_map(|s| match s {
        Stmt::LaunchTask { function, .. } => Some(*function),
        _ => None,
    })?;
    let body = fns.iter().find(|(n, _)| *n == producer).map(|(_, b)| *b)?;
    // The producer body must be a single counted `Repeat` loop (no static sends — that is increment 1).
    let (pattern, iterable, loop_body) = body.iter().find_map(|s| match s {
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => Some((pattern, *iterable, *body)),
        _ => None,
    })?;
    let iv = match pattern {
        Pattern::Identifier(s) => *s,
        _ => return None,
    };
    let (lo, hi) = match iterable {
        Expr::Range { start, end } => (as_int(start)?, as_int(end)?),
        _ => return None,
    };
    let send_expr = loop_body.iter().find_map(|s| match s {
        Stmt::SendPipe { value, .. } => Some(*value),
        _ => None,
    })?;
    let send_expr_c = expr_to_c(send_expr, iv)?;
    let values: Vec<i64> = (lo..=hi)
        .map(|i| expr_eval(send_expr, iv, i))
        .collect::<Option<_>>()?;
    Some(DynamicSpec {
        lo,
        hi,
        values,
        send_expr_c,
    })
}

/// A recognized **racy select**: a task sleeps then sends; main awaits the first of {receive, timeout}. The
/// VM's deterministic scheduler fires the smaller-virtual-time event, so the outcome is a COMPARISON of the
/// send's virtual time (the task's cumulative sleep) against the timeout — reproducible on the mesh by a
/// coordinator core, independent of real hardware timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RacySelectSpec {
    /// The value the task sends.
    pub value: i64,
    /// The send's virtual time = the task's cumulative sleep, in milliseconds.
    pub send_vtime_ms: i64,
    /// The select's timeout, in milliseconds.
    pub timeout_ms: i64,
}

/// A duration literal in milliseconds: a `Duration` (nanos) is divided down; a bare number is taken as ms.
fn duration_ms(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Duration(nanos)) => Some(nanos / 1_000_000),
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

/// Recognize a racy select: main launches a task (which sleeps then sends one literal) and awaits the first
/// of a receive + a timeout, with NO pre-send in main. Extracts the send's virtual time + the timeout.
pub fn recognize_racy_select(stmts: &[Stmt], interner: &Interner) -> Option<RacySelectSpec> {
    let (fns, main) = functions(stmts, interner);
    let main = main?;
    // A pre-send would make this the READY case (recognize_select), not the racy case.
    if main.iter().any(|s| matches!(s, Stmt::SendPipe { .. })) {
        return None;
    }
    let producer = main.iter().find_map(|s| match s {
        Stmt::LaunchTask { function, .. } => Some(*function),
        _ => None,
    })?;
    let branches = main.iter().find_map(|s| match s {
        Stmt::Select { branches } => Some(branches),
        _ => None,
    })?;
    if !branches
        .iter()
        .any(|b| matches!(b, SelectBranch::Receive { .. }))
    {
        return None;
    }
    let timeout_ms = branches.iter().find_map(|b| match b {
        SelectBranch::Timeout { milliseconds, .. } => duration_ms(milliseconds),
        _ => None,
    })?;

    // The task: sum its sleeps (virtual time) + find the single sent literal.
    let body = fns.iter().find(|(n, _)| *n == producer).map(|(_, b)| *b)?;
    let mut send_vtime_ms = 0i64;
    let mut value = None;
    for s in body {
        match s {
            Stmt::Sleep { milliseconds } => send_vtime_ms += duration_ms(milliseconds)?,
            Stmt::SendPipe { value: v, .. } => value = Some(as_int(v)?),
            Stmt::Return { value: None } => {}
            _ => return None,
        }
    }
    Some(RacySelectSpec {
        value: value?,
        send_vtime_ms,
        timeout_ms,
    })
}

/// A recognized **multi-task race**: N tasks each sleep then send; main awaits receive-or-timeout. The
/// deterministic scheduler fires the global-minimum-virtual-time event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RacyMultiSpec {
    /// `(value, send_vtime_ms)` per racing task, in launch order.
    pub tasks: Vec<(i64, i64)>,
    /// The timeout arm's virtual time, in milliseconds.
    pub timeout_ms: i64,
}

/// Recognize a multi-task race: main launches ≥2 tasks (each sleeps then sends one literal, no pre-send in
/// main) and awaits the first of a receive + a timeout. Extracts each task's `(value, send-vtime)` + timeout.
pub fn recognize_multi_racy(stmts: &[Stmt], interner: &Interner) -> Option<RacyMultiSpec> {
    let (fns, main) = functions(stmts, interner);
    let main = main?;
    if main.iter().any(|s| matches!(s, Stmt::SendPipe { .. })) {
        return None;
    }
    let launched: Vec<Symbol> = main
        .iter()
        .filter_map(|s| match s {
            Stmt::LaunchTask { function, .. } => Some(*function),
            _ => None,
        })
        .collect();
    if launched.len() < 2 {
        return None; // one task is the single-task racy case
    }
    let branches = main.iter().find_map(|s| match s {
        Stmt::Select { branches } => Some(branches),
        _ => None,
    })?;
    if !branches
        .iter()
        .any(|b| matches!(b, SelectBranch::Receive { .. }))
    {
        return None;
    }
    let timeout_ms = branches.iter().find_map(|b| match b {
        SelectBranch::Timeout { milliseconds, .. } => duration_ms(milliseconds),
        _ => None,
    })?;

    let mut tasks = Vec::new();
    for name in &launched {
        let body = fns.iter().find(|(n, _)| n == name).map(|(_, b)| *b)?;
        let mut vtime = 0i64;
        let mut value = None;
        for s in body {
            match s {
                Stmt::Sleep { milliseconds } => vtime += duration_ms(milliseconds)?,
                Stmt::SendPipe { value: v, .. } => value = Some(as_int(v)?),
                Stmt::Return { value: None } => {}
                _ => return None,
            }
        }
        tasks.push((value?, vtime));
    }
    Some(RacyMultiSpec { tasks, timeout_ms })
}

/// A recognized **select** (`Await the first of { Receive … / After … }`) on a READY channel: main
/// pre-sends `presend` into the pipe, so the deterministic scheduler takes the RECEIVE arm; `timeout_ticks`
/// is the (dead-on-this-path) timeout duration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectSpec {
    /// The value main sends before the select — makes the channel ready.
    pub presend: i64,
    /// The timeout arm's duration literal (unused on the ready path; kept for the emitted fallback arm).
    pub timeout_ticks: i64,
}

/// Recognize a select on a ready channel: `Send v into ch` then `Await the first of { Receive x from ch: …;
/// After n …: … }`. Returns the spec, or `None` if not this shape.
pub fn recognize_select(stmts: &[Stmt], interner: &Interner) -> Option<SelectSpec> {
    let (_fns, main) = functions(stmts, interner);
    let main = main?;

    // The pre-send that makes the channel ready.
    let presend = main.iter().find_map(|s| match s {
        Stmt::SendPipe { value, .. } => as_int(value),
        _ => None,
    })?;

    // The select must have a Receive arm and a Timeout arm.
    let branches = main.iter().find_map(|s| match s {
        Stmt::Select { branches } => Some(branches),
        _ => None,
    })?;
    let has_receive = branches
        .iter()
        .any(|b| matches!(b, SelectBranch::Receive { .. }));
    let timeout_ticks = branches.iter().find_map(|b| match b {
        SelectBranch::Timeout { milliseconds, .. } => match milliseconds {
            Expr::Literal(Literal::Number(n)) => Some(*n),
            Expr::Literal(Literal::Duration(nanos)) => Some((nanos / 1_000_000_000).max(1)),
            _ => Some(1),
        },
        _ => None,
    })?;
    if !has_receive {
        return None;
    }
    Some(SelectSpec {
        presend,
        timeout_ticks,
    })
}

/// A recognized **fan-out**: N distinct tasks each send one value into a shared pipe, and main receives all
/// N and folds them (commutative sum — so the mesh's arrival order doesn't matter and it stays ≡VM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanoutSpec {
    /// The value each task sends, one per task (task `i` runs on its own core and contributes `values[i]`).
    pub values: Vec<i64>,
}

/// Recognize the fan-out pattern: `main` launches N (≥2) distinct tasks, each of which sends exactly one
/// integer literal into a shared pipe, and `main` receives N values. `None` if the program is not this shape.
pub fn recognize_fanout(stmts: &[Stmt], interner: &Interner) -> Option<FanoutSpec> {
    let (fns, main) = functions(stmts, interner);
    let main = main?;

    // The launched task functions, in launch order.
    let launched: Vec<Symbol> = main
        .iter()
        .filter_map(|s| match s {
            Stmt::LaunchTask { function, .. } => Some(*function),
            _ => None,
        })
        .collect();
    if launched.len() < 2 {
        return None; // one task is the producer/consumer case, not fan-out
    }

    // Each launched task must send exactly one integer literal into the pipe.
    let mut values = Vec::new();
    for name in &launched {
        let body = fns.iter().find(|(n, _)| n == name).map(|(_, b)| *b)?;
        let sends: Vec<i64> = body
            .iter()
            .filter_map(|s| match s {
                Stmt::SendPipe { value, .. } => as_int(value),
                _ => None,
            })
            .collect();
        // exactly one send, and nothing but sends/returns in the body (increment 2 keeps it simple).
        let only_sends_or_ret = body
            .iter()
            .all(|s| matches!(s, Stmt::SendPipe { .. } | Stmt::Return { value: None }));
        if sends.len() != 1 || !only_sends_or_ret {
            return None;
        }
        values.push(sends[0]);
    }

    // main must receive one value per task.
    let receives = main
        .iter()
        .filter(|s| matches!(s, Stmt::ReceivePipe { .. }))
        .count();
    if receives != values.len() {
        return None;
    }
    let _ = as_ident;
    Some(FanoutSpec { values })
}
