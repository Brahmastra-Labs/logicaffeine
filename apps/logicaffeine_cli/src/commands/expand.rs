//! `largo expand` — show what an LGX page BECAME.
//!
//! A page is not run as written. A `## Component` is lowered into ordinary LOGOS: a render
//! function that writes the view to `vdom.lg`'s tape (`vdOpen`/`vdText`/`vdClose`), which the
//! commit turns into the DOM calls a browser applies, an action function per action, a
//! mount entry that runs the load handlers once, a server-side HTML builder, and a dispatcher the
//! driving verbs route through; a `## Route` becomes a function plus a synthesized
//! `route(method, path)` walk; a record that is serialized grows a derived `toJson_<T>`.
//!
//! That lowering is the thing that actually runs, and there was no way to look at it. When a page
//! behaves in a way its markup does not explain — a handler that never fires, a component the
//! bytecode tier refuses — the lowering is the answer, and this prints it.
//!
//! ★ IT WALKS THE REAL STATEMENTS. A hand-written summary of what the compiler "does" would be a
//! second description of it, free to drift and to describe a program it no longer emits. So this
//! reads the statements the parser actually produced, and shows a synthesized function's body as
//! the calls it makes.

use std::path::PathBuf;

use logicaffeine_compile::ui_bridge::with_parsed_program;

use crate::diagnostics::{self, Placed};
use logicaffeine_language::ast::stmt::{Block, Expr, Stmt};
use logicaffeine_language::Interner;

/// The prefixes the LGX lowering gives the functions it synthesizes, with what each one is for —
/// so the listing explains itself rather than making the reader recognise a naming convention.
const SYNTHESIZED: &[(&str, &str)] = &[
    ("__render_", "writes the view to the tape its paint commits"),
    ("__mount_", "runs the load handlers once, then paints"),
    (
        "__act_",
        "one action, as a function over the component's state",
    ),
    ("__bind_", "writes a bound input's value back into state"),
    ("__html_", "renders the view to HTML text (the server tier)"),
    (
        "__lgx_dispatch",
        "routes a driving verb to the handler its selector names",
    ),
    ("__route_", "one route's handler"),
    ("toJson_", "a record's derived JSON encoder"),
    ("fromJson_", "a record's derived JSON decoder"),
    (
        "route",
        "the synthesized route walk (method + path → response)",
    ),
    ("routeStatus", "the synthesized route walk's status half"),
];

fn purpose(name: &str) -> Option<&'static str> {
    SYNTHESIZED
        .iter()
        .filter(|(p, _)| {
            if p.ends_with('_') {
                name.starts_with(p)
            } else {
                name == *p
            }
        })
        .map(|(_, why)| *why)
        .next()
}

/// Every function call a block makes, in order, deduplicated — the shape of what a synthesized
/// function actually does, without printing a whole AST at somebody who wanted an overview.
fn calls_in(block: &Block<'_>, interner: &Interner, out: &mut Vec<String>) {
    for stmt in block.iter() {
        match stmt {
            Stmt::Call { function, .. } => push(out, interner.resolve(*function)),
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                calls_in_expr(value, interner, out)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                calls_in_expr(cond, interner, out);
                calls_in(then_block, interner, out);
                if let Some(e) = else_block {
                    calls_in(e, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => calls_in(body, interner, out),
            Stmt::Return { value: Some(v) } => calls_in_expr(v, interner, out),
            _ => {}
        }
    }
}

fn calls_in_expr(expr: &Expr<'_>, interner: &Interner, out: &mut Vec<String>) {
    if let Expr::Call { function, args } = expr {
        push(out, interner.resolve(*function));
        for a in args {
            calls_in_expr(a, interner, out);
        }
    }
}

fn push(out: &mut Vec<String>, name: &str) {
    let name = name.to_string();
    if !out.contains(&name) {
        out.push(name);
    }
}

/// `largo expand <file>` — print the lowering, or say there was none.
///
/// The file is expanded as it compiles: with every module it imports, so a page that says
/// `Import Shared.` expands against the records `Shared` declares. Every error it reports is
/// placed back in the file it is about.
pub(crate) fn cmd_expand(file: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !file.is_file() {
        return Err(format!("{}: no such file", file.display()).into());
    }
    let closure =
        crate::closure::assemble_closure(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let report = expand_source(&closure, &file)?;
    print!("{report}");
    Ok(())
}

/// The report for an assembled file.
///
/// A page that does not parse is an ERROR rather than an empty listing: somebody runs this
/// BECAUSE something is wrong, and an empty list would send them looking at the lowering when the
/// problem is a syntax error — so the error is the parser's own, at its `file:line:col`.
pub(crate) fn expand_source(
    closure: &crate::closure::Closure,
    file: &std::path::Path,
) -> Result<String, String> {
    let source = closure.source.as_str();
    let place = |pe: &logicaffeine_language::ParseError| {
        let at = Placed::locate(pe.span, source, &closure.source_map, file);
        diagnostics::render_parse_error(pe, &at)
    };
    with_parsed_program(source, |parsed, interner| {
        let Ok((stmts, _types, _policies)) = parsed else {
            let why = match logicaffeine_compile::compile::compile_to_program(source) {
                Err(pe) => place(&pe),
                Ok(_) => "this source does not parse".to_string(),
            };
            return Err(format!(
                "there is no lowering to show, because this source does not parse:\n{why}"
            ));
        };
        let mut out = String::new();
        let mut rows: Vec<(String, String, Vec<String>)> = Vec::new();
        // What the source DECLARED, beside what the lowering produced. The two are different
        // questions and the difference is the interesting one: a component that declared itself
        // and synthesized nothing was REFUSED, and reporting only the functions would print
        // "nothing is lowered" at a page full of LGX — which reads as "the compiler ate your
        // program" and is exactly the lie somebody reaches for this tool to resolve.
        let mut declared: Vec<String> = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::ComponentDef(def) => declared.push(interner.resolve(def.name).to_string()),
                Stmt::RouteDef { .. } => declared.push("a route".to_string()),
                Stmt::DirectiveDef { .. } => declared.push("a directive".to_string()),
                Stmt::MiddlewareDef { .. } => declared.push("a middleware guard".to_string()),
                Stmt::RedirectDef { .. } => declared.push("a redirect".to_string()),
                Stmt::FunctionDef { name, body, .. } => {
                    let name = interner.resolve(*name).to_string();
                    let Some(why) = purpose(&name) else { continue };
                    let mut calls = Vec::new();
                    calls_in(body, interner, &mut calls);
                    rows.push((name, why.to_string(), calls));
                }
                _ => {}
            }
        }
        // Source order, which is the order the lowering emitted them — a listing people diff
        // between runs must not reorder itself, and the registries underneath are hash maps.
        if rows.is_empty() && declared.is_empty() {
            out.push_str("No LGX constructs in this source — nothing is lowered, so the program\n");
            out.push_str("you wrote is the program that runs.\n");
            return Ok(out);
        }
        if !declared.is_empty() {
            out.push_str(&format!(
                "{} LGX construct(s) declared: {}\n\n",
                declared.len(),
                declared.join(", ")
            ));
        }
        if rows.is_empty() {
            out.push_str("…and NOTHING was lowered. Every construct above was refused — see the\n");
            out.push_str(
                "reason below, which is what a page behaving unlike its markup looks like.\n\n",
            );
        }
        out.push_str(&format!("{} synthesized function(s):\n\n", rows.len()));
        for (name, why, calls) in &rows {
            out.push_str(&format!("  {name}\n      {why}\n"));
            if !calls.is_empty() {
                out.push_str(&format!("      calls: {}\n", calls.join(", ")));
            }
            out.push('\n');
        }
        // A construct the bytecode tier refuses is exactly what somebody reaches for this to
        // understand, so it is named rather than left silently absent.
        match logicaffeine_compile::compile::compile_to_program(source) {
            Ok(_) => {}
            Err(e) => {
                out.push_str("REFUSED on the bytecode tier — the browser cannot run this page:\n");
                out.push_str(&place(&e));
                out.push('\n');
            }
        }
        Ok(out)
    })
}
