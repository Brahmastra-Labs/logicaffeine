//! Which function parameters carry a pipe.
//!
//! A pipe crosses into a function as a parameter declared by its element type (`## To consume (ch: Real)`:
//! the interpreter types nothing here) or as `Pipe of Real`. A parameter is a pipe when its function sends into
//! it, receives from it, closes it or selects on it, or hands it to a call or a launch where the callee's
//! parameter is a pipe. A tier that types a parameter by its declaration asks this first, so a pipe of Real
//! arrives as a pipe, never as a Real.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{for_each_node, Visited};

use crate::ast_map::Arenas;

use crate::ast::stmt::{Expr, SelectBranch, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// The positions of each function's pipe parameters; a function with none is absent.
pub fn pipe_params(stmts: &[Stmt], interner: &Interner) -> HashMap<Symbol, HashSet<usize>> {
    let mut pipes: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    // Each function's hand-offs of a parameter: (callee, argument position, the parameter's position).
    let mut handoffs: Vec<(Symbol, Symbol, usize, usize)> = Vec::new();
    for stmt in stmts {
        let Stmt::FunctionDef {
            name, params, body, ..
        } = stmt
        else {
            continue;
        };
        let position = |sym: Symbol| params.iter().position(|(p, _)| *p == sym);
        let mut own: HashSet<usize> = params
            .iter()
            .enumerate()
            .filter(|(_, (_, ty))| declares_pipe(ty, interner))
            .map(|(i, _)| i)
            .collect();
        let mut pass = |callee: Symbol, args: &[&Expr]| {
            for (j, arg) in args.iter().enumerate() {
                if let Some(i) = identifier(arg).and_then(position) {
                    handoffs.push((*name, callee, j, i));
                }
            }
        };
        for_each_node(body, |node| match node {
            Visited::Stmt(s) => {
                own.extend(
                    pipe_operands(s)
                        .into_iter()
                        .filter_map(identifier)
                        .filter_map(position),
                );
                if let Some((callee, args)) = handoff(s) {
                    pass(callee, args);
                }
            }
            Visited::Expr(Expr::Call { function, args }) => pass(*function, args),
            Visited::Expr(_) => {}
        });
        pipes.insert(*name, own);
    }
    loop {
        let mut grew = false;
        for (f, callee, j, i) in &handoffs {
            let callee_pipe = pipes.get(callee).is_some_and(|ps| ps.contains(j));
            if callee_pipe && pipes.get_mut(f).is_some_and(|ps| ps.insert(*i)) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    pipes.retain(|_, ps| !ps.is_empty());
    pipes
}

/// The program with every pipe parameter declared as one: `ch: Real` a pipe parameter becomes
/// `ch: Pipe of Real`, so each site that types a parameter by its declaration sees the pipe.
pub fn declare_pipe_params<'a>(
    stmts: &[Stmt<'a>],
    arenas: Arenas<'a>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let pipes = pipe_params(stmts, interner);
    if pipes.is_empty() {
        return stmts.to_vec();
    }
    let pipe = interner.intern("Pipe");
    stmts
        .iter()
        .map(|stmt| match stmt {
            Stmt::FunctionDef { name, params, .. } if pipes.contains_key(name) => {
                let at = &pipes[name];
                let params = params
                    .iter()
                    .enumerate()
                    .map(|(i, (p, ty))| {
                        if at.contains(&i) && !declares_pipe(ty, interner) {
                            let element = arenas.types.alloc_slice(vec![(*ty).clone()]);
                            (
                                *p,
                                &*arenas.types.alloc(TypeExpr::Generic {
                                    base: pipe,
                                    params: element,
                                }),
                            )
                        } else {
                            (*p, *ty)
                        }
                    })
                    .collect();
                let mut rewritten = stmt.clone();
                if let Stmt::FunctionDef { params: slot, .. } = &mut rewritten {
                    *slot = params;
                }
                rewritten
            }
            other => other.clone(),
        })
        .collect()
}

/// Whether a declared type is `Pipe of T`.
fn declares_pipe(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Generic { base, .. } if interner.resolve(*base) == "Pipe")
}

fn identifier(e: &&Expr) -> Option<Symbol> {
    match e {
        Expr::Identifier(sym) => Some(*sym),
        _ => None,
    }
}

/// The pipes a statement itself operates on.
fn pipe_operands<'s, 'a>(stmt: &'s Stmt<'a>) -> Vec<&'s &'a Expr<'a>> {
    match stmt {
        Stmt::SendPipe { pipe, .. }
        | Stmt::TrySendPipe { pipe, .. }
        | Stmt::ReceivePipe { pipe, .. }
        | Stmt::TryReceivePipe { pipe, .. } => vec![pipe],
        Stmt::CloseChannel { channel } => vec![channel],
        Stmt::Select { branches } => branches
            .iter()
            .filter_map(|b| match b {
                SelectBranch::Receive { pipe, .. } => Some(pipe),
                SelectBranch::Timeout { .. } => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A statement that hands arguments to a function: a call or a launch.
fn handoff<'s, 'a>(stmt: &'s Stmt<'a>) -> Option<(Symbol, &'s [&'a Expr<'a>])> {
    match stmt {
        Stmt::Call { function, args }
        | Stmt::LaunchTask { function, args }
        | Stmt::LaunchTaskWithHandle { function, args, .. } => Some((*function, args.as_slice())),
        _ => None,
    }
}
