//! ★ LOCK — every call the LGX lowering emits passes exactly as many arguments as the function
//! it calls declares.
//!
//! ★ THIS EXISTS BECAUSE A REAL BUG SAT BEHIND A FULLY GREEN SUITE. `When a message arrives on
//! "chat" (m: Text):` lowers the handler to a FUNCTION taking `(instance key, payload)`. There
//! are two paths that reach it: the in-process dispatcher, which passes the delivered value, and
//! the closure handed to the relay host, which was built with the instance key ALONE. Every
//! parity, construct and round-trip test drives the dispatcher — so a message from an actual
//! WebSocket arrived one argument short, and NOTHING in the tree could see it. The only
//! instrument that models that path is a browser.
//!
//! ★ AND THE ARITY WAS NOT CHECKED ANYWHERE. A lowered call with too few arguments is not a parse
//! error, and it is not caught downstream — the mismatch survives all the way to the tier that
//! runs it. So the fix is not a row in a table of trigger/path pairs that somebody must remember
//! to add. It is a property every lowered page already has to satisfy, stated once and checked
//! over the whole program: **a call site and its callee agree about how many arguments there
//! are.** A future trigger, verb or delivery path that gets this wrong fails here on the day it
//! lands, whether or not anyone thought to test its host callback.
//!
//! It walks the LOWERED statements — the ones the author never wrote — because that is where
//! these calls live. Author-written code is checked by the ordinary front end long before this.

use logicaffeine_language::ast::stmt::{ClosureBody, Expr, Stmt};
use logicaffeine_language::Interner;
use std::collections::HashMap;

/// Every function the lowered program declares, by name, with its declared parameter count.
fn declared(stmts: &[Stmt<'_>], out: &mut HashMap<String, usize>, i: &Interner) {
    for s in stmts {
        if let Stmt::FunctionDef {
            name, params, body, ..
        } = s
        {
            out.insert(i.resolve(*name).to_string(), params.len());
            declared(body, out, i);
        }
    }
}

/// Every call site in the lowered program, as `(callee, argument count)`.
///
/// It descends through statements AND through expressions, because a closure body is an
/// expression and the relay's callback is exactly that — a call inside a closure inside a call
/// argument. Stopping at the statement level would have missed the bug this lock was written for.
fn call_sites(stmts: &[Stmt<'_>], out: &mut Vec<(String, usize)>, i: &Interner) {
    for s in stmts {
        match s {
            Stmt::Call { function, args } => {
                out.push((i.resolve(*function).to_string(), args.len()));
                for a in args {
                    expr_calls(a, out, i);
                }
            }
            Stmt::FunctionDef { body, .. }
            | Stmt::During { body, .. }
            | Stmt::Splice { body }
            | Stmt::Reversibly { body } => call_sites(body, out, i),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                expr_calls(cond, out, i);
                call_sites(then_block, out, i);
                if let Some(e) = else_block {
                    call_sites(e, out, i);
                }
            }
            Stmt::While { cond, body, .. } => {
                expr_calls(cond, out, i);
                call_sites(body, out, i);
            }
            Stmt::Repeat { iterable, body, .. } => {
                expr_calls(iterable, out, i);
                call_sites(body, out, i);
            }
            Stmt::Inspect { target, arms, .. } => {
                expr_calls(target, out, i);
                for arm in arms {
                    call_sites(arm.body, out, i);
                }
            }
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_calls(value, out, i),
            Stmt::SetField { object, value, .. } => {
                expr_calls(object, out, i);
                expr_calls(value, out, i);
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                expr_calls(collection, out, i);
                expr_calls(index, out, i);
                expr_calls(value, out, i);
            }
            Stmt::Push { value, collection } | Stmt::Add { value, collection } => {
                expr_calls(value, out, i);
                expr_calls(collection, out, i);
            }
            Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
                expr_calls(object, out, i);
                expr_calls(recipient, out, i);
            }
            Stmt::Return { value: Some(e) } => expr_calls(e, out, i),
            _ => {}
        }
    }
}

fn expr_calls(e: &Expr<'_>, out: &mut Vec<(String, usize)>, i: &Interner) {
    match e {
        Expr::Call { function, args } => {
            out.push((i.resolve(*function).to_string(), args.len()));
            for a in args {
                expr_calls(a, out, i);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => expr_calls(e, out, i),
            ClosureBody::Block(b) => call_sites(b, out, i),
        },
        Expr::BinaryOp { left, right, .. } => {
            expr_calls(left, out, i);
            expr_calls(right, out, i);
        }
        Expr::Index { collection, index } => {
            expr_calls(collection, out, i);
            expr_calls(index, out, i);
        }
        Expr::Not { operand } => expr_calls(operand, out, i),
        Expr::FieldAccess { object, .. } => expr_calls(object, out, i),
        Expr::Contains { collection, value } => {
            expr_calls(collection, out, i);
            expr_calls(value, out, i);
        }
        Expr::Length { collection } => expr_calls(collection, out, i),
        Expr::List(items) | Expr::Tuple(items) => {
            for x in items {
                expr_calls(x, out, i);
            }
        }
        _ => {}
    }
}

/// The pages this walks. Each names a construct whose handler takes a bound parameter and is
/// reachable by more than one path, which is the shape that hid the original bug.
const PAGES: &[(&str, &str)] = &[
    (
        "message subscription",
        "## Component App\nState:\n    Let last be \"none\".\nActions:\n\
         \x20   When a message arrives on \"chat\" (m: Text):\n        Set last to m.\n\
         \x20   To announce:\n        Publish \"hi\" on \"chat\".\n\
         View:\n    <main><p>{last}</p><button on-click={announce}>a</button></main>\n\n\
         ## Main\nMount App.\n",
    ),
    (
        "file drop",
        "## Component App\nState:\n    Let name be \"none\".\nActions:\n\
         \x20   When a file is dropped on \"#z\" (n: Text):\n        Set name to n.\n\
         View:\n    <main><div id=\"z\">{name}</div></main>\n\n\
         ## Main\nMount App.\n",
    ),
    (
        "action failure",
        "## Component App\nState:\n    Let why be \"none\".\nActions:\n\
         \x20   When an action fails (e: Text):\n        Set why to e.\n\
         View:\n    <main><p>{why}</p></main>\n\n\
         ## Main\nMount App.\n",
    ),
    (
        "key press",
        "## Component App\nState:\n    Let n be 0.\nActions:\n\
         \x20   When the user presses \"Escape\":\n        Set n to n + 1.\n\
         View:\n    <main><p>{n}</p></main>\n\n\
         ## Main\nMount App.\n",
    ),
    (
        "window events and timers",
        "## Component App\nState:\n    Let n be 0.\nActions:\n\
         \x20   When the app goes offline:\n        Set n to n + 1.\n\
         \x20   Every 60 seconds:\n        Set n to n + 1.\n\
         View:\n    <main><p>{n}</p></main>\n\n\
         ## Main\nMount App.\n",
    ),
    (
        "store watcher and wrapper",
        "## Store Feed\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\n\
         ## Component Panel (title: Text)\nView:\n    <section><h2>{title}</h2>{children}</section>\n\n\
         ## Component Chip\nUse Feed.\nState:\n    Let seen be 0.\nActions:\n\
         \x20   When Feed's n changes:\n        Set seen to seen + 1.\n\
         View:\n    <span>{seen}</span>\n\n\
         ## Component App\nView:\n    <main><Panel title=\"T\"><Chip/></Panel></main>\n\n\
         ## Main\nMount App.\n",
    ),
];

#[test]
fn every_lowered_call_matches_its_callee_arity() {
    let mut wrong: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (label, src) in PAGES {
        logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
            let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("`{label}` must parse: {e}"));
            let mut decls = HashMap::new();
            declared(stmts, &mut decls, interner);
            let mut calls = Vec::new();
            call_sites(stmts, &mut calls, interner);
            for (callee, argc) in calls {
                let Some(&want) = decls.get(&callee) else {
                    continue;
                };
                checked += 1;
                if want != argc {
                    wrong.push(format!(
                        "`{label}`: `{callee}` declares {want} parameter(s) and is called with \
                         {argc}"
                    ));
                }
            }
        });
    }

    // ★ VACUITY. A walk that descended into nothing would report no mismatches and look identical
    // to a clean tree. The original bug lived in a closure inside a call argument, so "did this
    // walk reach anything at all" is exactly the question that matters — the count is the proof
    // that the traversal is still finding call sites after someone changes the lowering's shape.
    assert!(
        checked >= 40,
        "the walk resolved only {checked} lowered call(s) against a declaration — it is no longer \
         reaching into the lowered program, so a mismatch would go unreported"
    );

    assert!(
        wrong.is_empty(),
        "\nlowered call/callee arity mismatch(es):\n{}\n",
        wrong.join("\n")
    );
}
