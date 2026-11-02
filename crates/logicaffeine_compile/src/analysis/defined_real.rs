//! Which functions may define a real number: `the real number whose digits in base 3 are f`, `the
//! real number approximated by f`.
//!
//! A defined real reads its function only as a refinement needs it, and every tier reads it in a
//! context of its own, at whatever moment a comparison, a floor or a printed digit asks. That is
//! sound only for a function whose answer depends on its argument alone and whose call does nothing
//! but answer. The phrase is the builtin `definedReal(kind, base, name, (n: Int) -> name(n))`, and
//! every tier reads its four arguments as the phrase writes them, so a program is refused, before it
//! runs, when it calls `definedReal` in any other form, and when a function it defines a real by:
//!
//! - is not one the program defines;
//! - does not take exactly one `Int`;
//! - declares an answer other than an `Int` (digits) or an `Int` or a `Rational` (approximations);
//! - does more than answer — shows, reads or writes a file, sends, launches, chooses — or calls a
//!   function that does ([`crate::optimize::effects`]);
//! - or reads a variable of the program, itself or through a function it calls.

use std::collections::HashSet;

use crate::ast::stmt::{ClosureBody, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::optimize::mentions::Mentions;
use crate::optimize::writes::WriteSet;

/// The first function the program defines a real number by that cannot define one, refused in
/// words naming it; `Ok` when there is none.
pub fn check(stmts: &[Stmt], interner: &Interner) -> Result<(), String> {
    let Some(defined_real) = interner.lookup("definedReal") else {
        return Ok(());
    };
    let walk = super::known_calls::harvest_calls(stmts);
    let sites: Vec<_> = walk.calls.iter().filter(|c| c.function == defined_real).collect();
    if sites.is_empty() {
        return Ok(());
    }
    let functions: Vec<&Stmt> = stmts.iter().filter(|s| matches!(s, Stmt::FunctionDef { .. })).collect();
    if functions
        .iter()
        .any(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == defined_real))
    {
        return Ok(());
    }
    let main: Vec<Stmt> = stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .cloned()
        .collect();
    let program_variables: HashSet<Symbol> = WriteSet::of_block(&main).declared().clone();
    let effects = crate::optimize::effects::EffectEnv::from_stmts(stmts, interner);
    for site in sites {
        let Some(Phrase { kind, base, function }) = phrase(site.args, interner) else {
            return Err("definedReal is written `the real number whose digits in base 10 are f` or `the real \
                        number approximated by f`"
                .to_string());
        };
        logicaffeine_base::exact_real::Definition::named(kind, base)?;
        let name = interner.resolve(function);
        let Some(Stmt::FunctionDef { params, return_type, .. }) = functions
            .iter()
            .find(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == function))
            .copied()
        else {
            return Err(format!("{name} is no function this program defines"));
        };
        let takes_an_int = matches!(params.as_slice(), [(_, t)] if names(t, interner) == Some("Int"));
        if !takes_an_int {
            return Err(format!(
                "{name} takes one Int to define a real number — the digit's place or the precision — and \
                 it takes {}",
                taken(params, interner)
            ));
        }
        let digits = kind == "digits";
        if let Some(answer) = return_type.and_then(|t| names(t, interner)) {
            let answers = if digits { answer == "Int" } else { matches!(answer, "Int" | "Rational") };
            if !answers {
                return Err(format!(
                    "{name} answers {answer}, and a real number is defined by {}",
                    if digits { "whole digits" } else { "whole or rational answers" }
                ));
            }
        }
        if !effects.function_is_replayable(name) {
            return Err(format!(
                "{name} does more than answer, and a real number's function only computes its answer"
            ));
        }
        if let Some((reader, variable)) = reads_a_variable(function, &functions, &program_variables) {
            let variable = interner.resolve(variable);
            let reader = interner.resolve(reader);
            return Err(if reader == name {
                format!("{name} reads `{variable}`, and a real number's function reads only its argument")
            } else {
                format!(
                    "{name} reads `{variable}` through {reader}, and a real number's function reads only its \
                     argument"
                )
            });
        }
    }
    Ok(())
}

/// What a `definedReal` call says, read only from the form the phrase writes.
struct Phrase<'i> {
    kind: &'i str,
    base: i64,
    function: Symbol,
}

/// The phrase a `definedReal` call writes — its kind, its base, its function's name and the closure
/// `(n: Int) -> name(n)` — or `None` for a call of any other form.
fn phrase<'i>(args: &[&Expr], interner: &'i Interner) -> Option<Phrase<'i>> {
    let [Expr::Literal(Literal::Text(kind)), Expr::Literal(Literal::Number(base)), Expr::Literal(Literal::Text(name)), Expr::Closure {
        params,
        body: ClosureBody::Expression(Expr::Call { function, args: read }),
        return_type: None,
    }] = args
    else {
        return None;
    };
    let [(index, index_type)] = params.as_slice() else {
        return None;
    };
    let reads_its_index = matches!(read.as_slice(), [Expr::Identifier(i)] if i == index);
    let kind = interner.resolve(*kind);
    let named = interner.resolve(*name) == interner.resolve(*function);
    (reads_its_index
        && named
        && names(index_type, interner) == Some("Int")
        && matches!(kind, "digits" | "approximations"))
    .then_some(Phrase {
        kind,
        base: *base,
        function: *function,
    })
}

/// A type written as a single name, like `Int` or `Rational`.
fn names<'i>(t: &TypeExpr, interner: &'i Interner) -> Option<&'i str> {
    match t {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => Some(interner.resolve(*s)),
        _ => None,
    }
}

/// What a function takes, as the refusal says it.
fn taken(params: &[(Symbol, &TypeExpr)], interner: &Interner) -> String {
    match params {
        [] => "nothing".to_string(),
        [(_, t)] => format!("one {}", names(t, interner).unwrap_or("value of another type")),
        more => format!("{} parameters", more.len()),
    }
}

/// The first function reachable from `function` by calls that reads one of the program's own
/// variables, with the variable.
fn reads_a_variable(
    function: Symbol,
    functions: &[&Stmt],
    program_variables: &HashSet<Symbol>,
) -> Option<(Symbol, Symbol)> {
    let mut seen: HashSet<Symbol> = HashSet::new();
    let mut pending = vec![function];
    while let Some(f) = pending.pop() {
        if !seen.insert(f) {
            continue;
        }
        let Some(Stmt::FunctionDef { params, body, .. }) =
            functions.iter().find(|s| matches!(s, Stmt::FunctionDef { name, .. } if *name == f)).copied()
        else {
            continue;
        };
        let mut own: HashSet<Symbol> = params.iter().map(|(p, _)| *p).collect();
        own.extend(WriteSet::of_block(body).declared().iter().copied());
        let touched = Mentions::of_block(body);
        if let Some(v) = touched
            .symbols()
            .find(|s| program_variables.contains(s) && !own.contains(s))
        {
            return Some((f, v));
        }
        for site in super::known_calls::harvest_calls(body).calls {
            pending.push(site.function);
        }
    }
    None
}
