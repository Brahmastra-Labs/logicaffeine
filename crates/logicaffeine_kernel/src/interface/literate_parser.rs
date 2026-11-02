//! Literate Specification Parser for the Kernel.
//!
//! This module parses English-like "Literate" syntax and emits the same
//! `Command` and `Term` structures as the Coq-style parser.
//!
//! # Supported Syntax
//!
//! ## Inductive Types
//! ```text
//! A Bool is either Yes or No.
//! A Nat is either:
//!     Zero.
//!     a Successor with pred: Nat.
//! ```
//!
//! ## Function Definitions (with implicit recursion)
//! ```text
//! ## To add (n: Nat) and (m: Nat) -> Nat:
//!     Consider n:
//!         When Zero: Yield m.
//!         When Successor k: Yield Successor (add k m).
//! ```
//!
//! ## Pattern Matching
//! ```text
//! Consider x:
//!     When Zero: Yield 0.
//!     When Successor k: Yield k.
//! ```
//!
//! Several columns at once, separated by `and` — Lean's `| 0, _ => …` — with the overlap split
//! top-to-bottom by the equation compiler:
//! ```text
//! Consider a and b:
//!     When Zero and _: Yield 1.
//!     When _ and Zero: Yield 2.
//!     When _ and _: Yield 3.
//! ```
//!
//! ## Lambda Expressions
//! ```text
//! given x: Nat yields Successor x
//! |x: Nat| -> Successor x
//! ```

use super::command::Command;
use super::error::ParseError;
use super::match_compile::{compile_clauses_reporting, pattern_binder_types, Clause, Pattern};
use super::surface::{respell_names, Name, Namespaces, Resolution};
use super::with_env;
use crate::{infer_type, normalize, Context, Literal, Term, Universe};
use std::collections::HashSet;

/// Build the reflected, inert list of tactic names for a proof script:
/// `SApp (SApp (SName "step") (SName n0)) ( … (SName "end"))` — decoded by `extract_script_names`.
fn build_script_name_list(names: &[String]) -> Term {
    let sname = |s: &str| {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    };
    let sapp = |x: Term, y: Term| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(x),
            )),
            Box::new(y),
        )
    };
    let mut result = sname("end");
    for n in names.iter().rev() {
        result = sapp(sapp(sname("step"), sname(n)), result);
    }
    result
}

/// The closed one-shot tactic vocabulary of the literate `Proof:` surface: the keyword a
/// mathematician writes, and the reflected tactic it applies to the reified goal. Adding a
/// tactic is a row here — `parse_proof_tactic` and `parse_single_tactic` both read this table,
/// so the two doors can never drift apart again.
pub const ONE_SHOT_TACTICS: &[(&str, &str)] = &[
    ("ring", "try_ring"),
    ("refl", "try_refl"),
    ("lia", "try_lia"),
    ("cc", "try_cc"),
    ("simp", "try_simp"),
    ("omega", "try_omega"),
    ("assumption", "try_assumption"),
    ("split", "try_split"),
    ("left", "try_left"),
    ("right", "try_right"),
    ("decide", "try_decide"),
    ("auto", "try_auto"),
];

/// `f x` where `f` is a prelude global.
fn apply1(global: &str, arg: Term) -> Term {
    Term::App(Box::new(Term::Global(global.to_string())), Box::new(arg))
}

/// The eigenvariable marker.
///
/// A peeled `forall` binder is reified as a CONSTANT standing for a fixed-but-arbitrary element,
/// and the ∀-introduction rule is only sound if that constant is FRESH. Without a mark it is not:
/// `forall add : Nat, (add 1 2) equals 3` would reify its bound variable as `SName "add"` — the
/// ring's own addition — and `ring` would certify a statement about an opaque `Nat` as though it
/// were a statement about the operator.
///
/// The mark alone does NOT make it fresh, and that was the claim here. `«n#eig»` is a
/// `«quoted name»`, which `scan_name` accepts everywhere an identifier is read, so a statement
/// can NAME the eigenvariable the surface is about to introduce — and a `Definition «n#eig»`
/// can give it a VALUE. Both collapse to one `SName` and the constant stops being arbitrary:
/// `∀n:Nat, n equals «n#eig»` reifies as `x = x` and `refl` closes it. So the name is checked
/// fresh against the statement and against the context — see [`to_eigenvariables`].
const EIGENVARIABLE_MARK: &str = "#eig";

/// Rename each peeled binder to its eigenvariable, capture-avoidingly: an inner binder that
/// rebinds the name shadows it, and is left alone.
///
/// ★ THE EIGENVARIABLE IS CHECKED FRESH, NOT ASSUMED FRESH. The candidate `n#eig` is stepped
/// (`n#eig1`, `n#eig2`, …) until it is a name the STATEMENT does not spell and the CONTEXT does
/// not define. Both halves matter and neither implies the other: a statement that writes
/// `«n#eig»` makes the quantified variable and that constant one atom, and a context that
/// DEFINES `«n#eig»` gives the "fixed but arbitrary" element a value the reflected evaluator
/// can unfold — so `∀n:Nat, n equals Zero` would be certified by `simp` against a constant that
/// happens to be `Zero`. A `ctx` of `None` is the text-only door, which can still check the
/// statement.
fn to_eigenvariables(
    ctx: Option<&Context>,
    body: &Term,
    binders: &[(String, Term)],
    except: Option<&str>,
) -> Term {
    let mut taken: HashSet<String> = HashSet::new();
    super::spelled_names(body, &mut taken);
    for (name, ty) in binders {
        taken.insert(name.clone());
        super::spelled_names(ty, &mut taken);
    }

    let mut acc = body.clone();
    for (name, _) in binders
        .iter()
        .filter(|(name, _)| Some(name.as_str()) != except)
    {
        let mut candidate = format!("{name}{EIGENVARIABLE_MARK}");
        let mut step = 0usize;
        while taken.contains(&candidate) || ctx.is_some_and(|c| c.get_global(&candidate).is_some())
        {
            step += 1;
            candidate = format!("{name}{EIGENVARIABLE_MARK}{step}");
        }
        taken.insert(candidate.clone());
        acc = rename_free_var(&acc, name, &candidate);
    }
    acc
}

/// Rename the FREE occurrences of `from` to `to`.
fn rename_free_var(term: &Term, from: &str, to: &str) -> Term {
    let go = |t: &Term| rename_free_var(t, from, to);
    match term {
        Term::Var(n) if n == from => Term::Var(to.to_string()),
        Term::App(f, a) => Term::App(Box::new(go(f)), Box::new(go(a))),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(go(param_type)),
            body: if param == from {
                body.clone()
            } else {
                Box::new(go(body))
            },
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(go(param_type)),
            body_type: if param == from {
                body_type.clone()
            } else {
                Box::new(go(body_type))
            },
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: if name == from {
                body.clone()
            } else {
                Box::new(go(body))
            },
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), if n == from { b.clone() } else { go(b) }))
                .collect(),
            index: *index,
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(go(discriminant)),
            motive: Box::new(go(motive)),
            cases: cases.iter().map(&go).collect(),
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(go(ty)),
            value: Box::new(go(value)),
            body: if name == from {
                body.clone()
            } else {
                Box::new(go(body))
            },
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(go(term)),
        },
        other => other.clone(),
    }
}

/// The free NAME `name` of an unquantified statement as a variable: every `Global(name)` becomes
/// `Var(name)`. `None` when a binder inside the statement is spelled `name` — turning the free
/// name into a variable there would let that binder capture it.
fn free_name_as_variable(term: &Term, name: &str) -> Option<Term> {
    let go = |t: &Term| free_name_as_variable(t, name);
    Some(match term {
        Term::Global(n) if n == name => Term::Var(name.to_string()),
        Term::App(f, a) => Term::App(Box::new(go(f)?), Box::new(go(a)?)),
        Term::Lambda { param, .. } | Term::Pi { param, .. } | Term::Let { name: param, .. }
            if param == name =>
        {
            return None
        }
        Term::Fix { name: n, .. } if n == name => return None,
        Term::MutualFix { defs, .. } if defs.iter().any(|(n, _)| n == name) => return None,
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(go(param_type)?),
            body: Box::new(go(body)?),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(go(param_type)?),
            body_type: Box::new(go(body_type)?),
        },
        Term::Fix { name: n, body } => Term::Fix {
            name: n.clone(),
            body: Box::new(go(body)?),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| Some((n.clone(), go(b)?)))
                .collect::<Option<_>>()?,
            index: *index,
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(go(discriminant)?),
            motive: Box::new(go(motive)?),
            cases: cases.iter().map(&go).collect::<Option<_>>()?,
        },
        Term::Let {
            name: n,
            ty,
            value,
            body,
        } => Term::Let {
            name: n.clone(),
            ty: Box::new(go(ty)?),
            value: Box::new(go(value)?),
            body: Box::new(go(body)?),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(go(term)?),
        },
        other => other.clone(),
    })
}

/// Peel the prenex `forall` binders off a statement.
///
/// Each peeled binder becomes an EIGENVARIABLE — a free name that no tactic can instantiate —
/// which is ∀-introduction, and is the reading this surface already gives every free variable:
/// `ring` proves `(add x y) equals (add y x)` precisely because `x` and `y` are opaque atoms. The
/// `implies` desugaring binds `"_"` and is NOT a quantifier, so it stops the peel.
fn split_universal_prefix(statement: &Term) -> (Vec<(String, Term)>, &Term) {
    let mut binders = Vec::new();
    let mut body = statement;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = body
    {
        if param == "_" {
            break;
        }
        binders.push((param.clone(), param_type.as_ref().clone()));
        body = body_type;
    }
    (binders, body)
}

/// `SApp (SApp (SName "Forall") ind_type) motive` — the quantified proposition an elimination
/// concludes, in the exact shape `try_delim_conclude` returns it.
fn forall_syntax(ind_type: &Term, motive: &Term) -> Term {
    let sapp = |f: Term, x: Term| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(f),
            )),
            Box::new(x),
        )
    };
    let sname_forall = Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text("Forall".to_string()))),
    );
    sapp(sapp(sname_forall, ind_type.clone()), motive.clone())
}

/// The goal the `i`-th constructor's bullet must close: `induction_step_goal ind motive i`,
/// which the kernel reduces against the real constructor list. The index is a `Nat` literal in
/// constructor form, because that is what `induction_step_goal` decodes.
fn case_goal_term(ind_type: &Term, motive: &Term, index: usize) -> Term {
    let mut idx = Term::Global("Zero".to_string());
    for _ in 0..index {
        idx = Term::App(Box::new(Term::Global("Succ".to_string())), Box::new(idx));
    }
    Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("induction_step_goal".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive.clone()),
        )),
        Box::new(idx),
    )
}

/// Whether a reflected term is `SName "<name>"`.
fn is_sname(term: &Term, name: &str) -> bool {
    matches!(term, Term::App(f, x)
        if matches!(f.as_ref(), Term::Global(n) if n == "SName")
            && matches!(x.as_ref(), Term::Lit(Literal::Text(s)) if s == name))
}

/// Match the reflected two-argument application `SApp (SApp (SName head) a) b`.
fn as_sapp2(term: &Term, head: &str) -> Option<(Term, Term)> {
    let Term::App(f, b) = term else { return None };
    let Term::App(sapp_outer, inner) = f.as_ref() else {
        return None;
    };
    if !matches!(sapp_outer.as_ref(), Term::Global(n) if n == "SApp") {
        return None;
    }
    let Term::App(g, a) = inner.as_ref() else {
        return None;
    };
    let Term::App(sapp_inner, sname) = g.as_ref() else {
        return None;
    };
    if !matches!(sapp_inner.as_ref(), Term::Global(n) if n == "SApp") {
        return None;
    }
    if !is_sname(sname, head) {
        return None;
    }
    Some(((**a).clone(), (**b).clone()))
}

/// The body of `SLam T body`.
fn as_slam_body(term: &Term) -> Option<Term> {
    let Term::App(f, body) = term else {
        return None;
    };
    let Term::App(slam, _ty) = f.as_ref() else {
        return None;
    };
    if !matches!(slam.as_ref(), Term::Global(n) if n == "SLam") {
        return None;
    }
    Some((**body).clone())
}

/// Match the reflected arrow `SPi A B`.
fn as_spi(term: &Term) -> Option<(Term, Term)> {
    let Term::App(f, body) = term else {
        return None;
    };
    let Term::App(spi, ty) = f.as_ref() else {
        return None;
    };
    if !matches!(spi.as_ref(), Term::Global(n) if n == "SPi") {
        return None;
    }
    Some(((**ty).clone(), (**body).clone()))
}

/// Peel ONE arrow-like layer off a reflected goal.
///
/// ★ THIS IS `getTargetArity`, NOT `intro`. Lean's `Lean/Meta/Tactic/Induction.lean:97` measures
/// a minor premise with `getTargetArity`, which counts `forallE` layers — and in Lean a `∀` and
/// a `→` are the SAME node, so both are counted, dependent or not. The reflected language splits
/// that node three ways, so all three are peeled here:
///
/// * a typed quantifier `SApp (SApp (SName "Forall") T) (SLam T body)`;
/// * a reflected implication in either spelling — `Implies`, what the case-formula builder
///   emits, and `implies`, what the surface's own infix emits;
/// * `SPi A B`, what a `Pi { param: "_" }` reifies as.
///
/// The script evaluator's `intro` is STRICTER than this: `reduction::as_implication` declines a
/// DEPENDENT `SPi`, whose body names its own binder. That difference cannot matter to the count,
/// and the reason is structural rather than lucky: the layers this count is used for are exactly
/// the ones `build_case_expected` ADDS — one `Forall` per constructor field, one `Implies` per
/// induction hypothesis — and it never emits an `SPi`. Every `SPi` layer belongs to the motive
/// body, and [`LiterateParser::case_binder_count`] subtracts the motive body's own depth away.
/// Peeling unconditionally is also what makes the count stable: whether a reflected `SPi` looks
/// dependent depends on whether its de Bruijn indices have been instantiated yet, and the count
/// must be the same number before and after.
fn peel_one_binder(goal: &Term) -> Option<Term> {
    if let Some((_ty, motive)) = as_sapp2(goal, "Forall") {
        return as_slam_body(&motive);
    }
    for spelling in ["Implies", "implies"] {
        if let Some((_p, q)) = as_sapp2(goal, spelling) {
            return Some(q);
        }
    }
    as_spi(goal).map(|(_a, b)| b)
}

/// How many arrow-like layers a reflected goal carries — Lean's `getTargetArity`.
fn goal_binder_depth(goal: &Term) -> usize {
    let mut depth = 0;
    let mut cur = goal.clone();
    while let Some(next) = peel_one_binder(&cur) {
        depth += 1;
        cur = next;
    }
    depth
}

// ============================================================
// PUBLIC API (called by command_parser.rs dispatcher)
// ============================================================

/// Parse "A \[Name\] is either..." into Command::Inductive
///
/// Supports:
/// - `A Bool is either Yes or No.`
/// - `A Nat is either: Zero. a Successor with pred: Nat.`
/// - `A List of (T: Type) is either: Empty. a Node with head: T and tail: List of T.`
pub fn parse_inductive(input: &str) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new(input);
    let cmd = parser.parse_literate_inductive()?;
    Ok((cmd, parser.pos))
}

/// Parse "## To \[name\] (params) -> RetType:" into Command::Definition
///
/// Handles implicit fixpoints: if the function name appears in the body,
/// the body is automatically wrapped in Term::Fix.
///
/// This door reads TEXT ALONE, so a `Consider` block is limited to what text can carry: one
/// column, one case per clause, in SOURCE order, with flat identifier binders. Use
/// [`parse_definition_in`] wherever a context exists — it compiles the equation set instead.
pub fn parse_definition(input: &str) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new(input);
    let cmd = parser.parse_literate_definition()?;
    Ok((cmd, parser.pos))
}

/// [`parse_definition`], with the kernel context the equation compiler needs.
///
/// With a context, a `Consider` block is a real equation set: nested constructor patterns
/// compile to a tree of positional matches, overlapping equations are split top-to-bottom
/// (first match wins, as in Lean), a `When _:` catch-all fans out over every constructor with no
/// clause of its own, several columns can be matched at once (`Consider a and b:` with
/// `When Zero and _:`), and every clause lands in the slot of the constructor it NAMES rather
/// than the slot its position in the source happens to hit.
pub fn parse_definition_in(ctx: &Context, input: &str) -> Result<(Command, usize), ParseError> {
    parse_definition_scoped(ctx, &Namespaces::new(), input)
}

/// [`parse_definition_in`], reading every name in the namespace scope `scope` — the door the REPL
/// reads through, so a literate definition's names mean exactly what they mean to every other door.
pub fn parse_definition_scoped(
    ctx: &Context,
    scope: &Namespaces,
    input: &str,
) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new_in(input, ctx, scope);
    let cmd = parser.parse_literate_definition()?;
    Ok((parser.respelled(cmd)?, parser.pos))
}

/// Parse "Let \[name\] be \[term\]." into Command::Definition (constant, not function)
///
/// This is for simple constant bindings like:
/// - `Let T be Apply(Name "Not", Variable 0).`
/// - `Let G be the diagonalization of T.`
pub fn parse_let_definition(input: &str) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new(input);
    let cmd = parser.parse_literate_let()?;
    Ok((cmd, parser.pos))
}

/// Parse "## Theorem: \[name\] Statement: \[proposition\]." into Command::Definition
///
/// This is for theorem declarations like:
/// - `## Theorem: Godel_First_Incompleteness\n    Statement: Consistent implies Not(Provable(G)).`
///
/// This door reads TEXT ALONE. A `Proof: induction <v>.` therefore cannot be told how many
/// constructors the variable's type HAS, so a bullet-count mistake surfaces only later, as the
/// elimination failing to conclude anything. Use [`parse_theorem_in`] wherever a context exists.
pub fn parse_theorem(input: &str) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new(input);
    let cmd = parser.parse_literate_theorem()?;
    Ok((cmd, parser.pos))
}

/// [`parse_theorem`], with the kernel context.
///
/// The one thing it buys is the one thing text cannot carry: the constructor list of whatever
/// `induction <v>` is eliminating. With it, a bullet-count mistake is reported AS a bullet-count
/// mistake — naming the inductive, its constructors and the count written — instead of arriving
/// as "the tactic did not establish the stated goal", which is what a reader sees when the case
/// chain is silently the wrong length.
pub fn parse_theorem_in(ctx: &Context, input: &str) -> Result<(Command, usize), ParseError> {
    parse_theorem_scoped(ctx, &Namespaces::new(), input)
}

/// The EXTENT of the `## To` block at the front of `input` — where it ends — read from its syntax
/// alone. See [`measure_theorem`].
pub fn measure_definition(input: &str) -> Result<usize, ParseError> {
    let mut parser = LiterateParser::measuring(input);
    parser.parse_literate_definition()?;
    Ok(parser.pos)
}

/// The EXTENT of the `## Theorem:` block at the front of `input`, read from its syntax alone.
///
/// Where a block ENDS is a fact about its text; what it MEANS is a fact about the context it is
/// read in. A document is split into blocks before any of them runs, so its splitter can only ask
/// the first question: a `## To` block whose equation set is over a function an EARLIER block of the
/// same document declares cannot be compiled yet, and it does not need to be — it is measured here,
/// and compiled when it is read to be run, against the context the blocks before it built. The
/// flat reading's refusals of what only a context can compile (a nested pattern, a catch-all,
/// a second column) are skipped for the same reason: measuring them needs their syntax only.
pub fn measure_theorem(input: &str) -> Result<usize, ParseError> {
    let mut parser = LiterateParser::measuring(input);
    parser.parse_literate_theorem()?;
    Ok(parser.pos)
}

/// [`parse_theorem_in`], reading every name in the namespace scope `scope`. The statement is
/// reflected into syntax only after its names are resolved, so a theorem about `add` in a session
/// that shadows `add` is a theorem about the session's `add`.
pub fn parse_theorem_scoped(
    ctx: &Context,
    scope: &Namespaces,
    input: &str,
) -> Result<(Command, usize), ParseError> {
    let mut parser = LiterateParser::new_in(input, ctx, scope);
    let cmd = parser.parse_literate_theorem()?;
    Ok((parser.respelled(cmd)?, parser.pos))
}

/// The head name of a type application: `Hue` → `"Hue"`, `List A` → `"List"`.
fn type_head_name(ty: &Term) -> Option<&str> {
    let mut cur = ty;
    while let Term::App(f, _) = cur {
        cur = f;
    }
    match cur {
        Term::Global(n) => Some(n),
        Term::Const { name, .. } => Some(name),
        _ => None,
    }
}

/// The flat positional reading of a clause list: one case per clause, in SOURCE order, each body
/// re-abstracted over that clause's binders.
///
/// This is everything a parser holding only a `&str` can do, and it is kept for the context-free
/// door alone. It cannot place a clause by the constructor it names (it does not know the
/// registration order), it cannot nest, it cannot fan a catch-all out, and it cannot split a
/// second column — so it refuses those shapes by name instead of guessing, and says where the
/// compiler lives.
fn flat_positional_match(
    discriminants: Vec<Term>,
    motive: Term,
    clauses: &[Clause],
) -> Result<Term, ParseError> {
    let needs_compiler = |what: &str| {
        ParseError::InvalidConsiderSyntax(format!(
            "{what} is compiled against the inductive's constructor list, which lives in the \
             kernel context — this parser has none. Read the definition through \
             `interface::parse_command_in_ctx` (or `literate_parser::parse_definition_in`), \
             which does."
        ))
    };

    // A SECOND COLUMN is a matrix, and a matrix is split by specialising it constructor by
    // constructor — which needs the constructor list. One `Match` node cannot hold it.
    let [discriminant] = <[Term; 1]>::try_from(discriminants)
        .map_err(|_| needs_compiler("a multi-column `Consider … and …`"))?;

    let mut cases = Vec::with_capacity(clauses.len());
    for clause in clauses {
        if clause.patterns.len() != 1 {
            return Err(needs_compiler("a multi-column equation set"));
        }
        let Pattern::Ctor { args, .. } = &clause.patterns[0] else {
            return Err(needs_compiler("a `When _:` catch-all"));
        };
        let mut binders = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Pattern::Var(n) => binders.push(n.clone()),
                Pattern::Wildcard => binders.push("_".to_string()),
                Pattern::Ctor { .. } => return Err(needs_compiler("a nested constructor pattern")),
                Pattern::Numeral(_) => return Err(needs_compiler("a numeral pattern")),
            }
        }
        let mut body = clause.body.clone();
        for binder in binders.into_iter().rev() {
            body = Term::Lambda {
                param: binder,
                param_type: Box::new(Term::Global("_".to_string())),
                body: Box::new(body),
            };
        }
        cases.push(body);
    }

    Ok(Term::Match {
        discriminant: Box::new(discriminant),
        motive: Box::new(motive),
        cases,
    })
}

// ============================================================
// PARSER STATE
// ============================================================

struct LiterateParser<'a> {
    input: &'a str,
    pos: usize,
    bound_vars: HashSet<String>,
    current_function: Option<String>,
    /// The return type of the current function (used as motive in Consider)
    return_type: Option<Term>,
    /// The kernel context, when the caller has one.
    ///
    /// An equation set is compiled against the discriminant's INDUCTIVE — its constructor list,
    /// their registration order, their arities — and none of that is recoverable from text. With
    /// a context a `Consider` is compiled by [`super::match_compile`]; without one it falls back
    /// to the flat positional reading, which is all a `&str` can support.
    ctx: Option<&'a Context>,
    /// The namespace scope a name the author wrote is resolved in, beside the context's shadow
    /// table. Consulted only with a context: without one a name is kept as written, and the REPL
    /// resolves it when it runs the command.
    scope: Namespaces,
    /// Names in scope with their types: the current function's parameters, then a clause's
    /// pattern binders while that clause's body is being read.
    locals: Vec<(String, Term)>,
    /// Read the block for its EXTENT only: an equation set is parsed and not compiled. See
    /// [`measure_theorem`].
    measuring: bool,
}

impl<'a> LiterateParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            bound_vars: HashSet::new(),
            current_function: None,
            return_type: None,
            ctx: None,
            scope: Namespaces::new(),
            locals: Vec::new(),
            measuring: false,
        }
    }

    /// A parser that reads a block for its extent alone (see [`measure_theorem`]).
    fn measuring(input: &'a str) -> Self {
        Self {
            measuring: true,
            ..Self::new(input)
        }
    }

    /// [`Self::new`], with the kernel context the equation compiler needs and the namespace scope
    /// the names are read in.
    fn new_in(input: &'a str, ctx: &'a Context, scope: &Namespaces) -> Self {
        Self {
            ctx: Some(ctx),
            scope: scope.clone(),
            ..Self::new(input)
        }
    }

    /// The key a name the AUTHOR wrote denotes, resolved in the scope and through the context's
    /// shadow table — the one resolution every door shares (`surface::Env::resolve`).
    ///
    /// Only a written name, or a surface operator standing for one (`+` is `add`, `equals` is
    /// `Eq`), comes through here. The reflection and derivation vocabulary this parser builds
    /// proofs out of (`SName`, `SApp`, `DCase`, `try_simp`, …) is the prelude's own and is never
    /// looked up: a tactic reads the prelude's `add` whatever a session calls `add`.
    fn denote(&self, written: String) -> Result<String, ParseError> {
        let Some(ctx) = self.ctx else {
            return Ok(written);
        };
        let Some(name) = Name::parse(&written) else {
            return Ok(written);
        };
        match with_env(ctx, &[], |env| env.resolve(&self.scope, &name)) {
            Resolution::Found(key) => Ok(key),
            Resolution::Unresolved(_) => Ok(written),
            Resolution::Ambiguous {
                written,
                candidates,
            } => Err(ParseError::AmbiguousName {
                written,
                candidates,
            }),
        }
    }

    /// The constant a name the author wrote denotes. See [`Self::denote`].
    fn global(&self, written: impl Into<String>) -> Result<Term, ParseError> {
        Ok(Term::Global(self.denote(written.into())?))
    }

    /// A command this parser read with a context, re-spelled so the REPL's own resolution of it
    /// reaches exactly the keys it already holds (`surface::respell_names`): the parse resolved
    /// every name the author wrote, and what it handed the kernel in between — constructors it
    /// compiled against, constants the elaborator inserted, the proof vocabulary — is in key space.
    fn respelled(&self, cmd: Command) -> Result<Command, ParseError> {
        let Some(ctx) = self.ctx else { return Ok(cmd) };
        let respell = |t: Term| -> Result<Term, ParseError> {
            with_env(ctx, &[], |env| {
                respell_names(&self.scope, env, &mut Vec::new(), &t).map(|r| r.into_owned())
            })
        };
        match cmd {
            Command::Definition {
                name,
                ty,
                body,
                is_hint,
                implicit_count,
                proof_goal,
            } => Ok(Command::Definition {
                name,
                ty: ty.map(respell).transpose()?,
                body: respell(body)?,
                is_hint,
                implicit_count,
                proof_goal: proof_goal.map(respell).transpose()?,
            }),
            other => Ok(other),
        }
    }

    /// The context a clause matrix is compiled against: the caller's, extended by every local
    /// currently in scope. `None` when this parser has no context at all.
    fn working_context(&self) -> Option<Context> {
        let base = self.ctx?;
        Some(
            self.locals
                .iter()
                .fold(base.clone(), |acc, (n, t)| acc.extend(n, t.clone())),
        )
    }

    // ============================================================
    // INDUCTIVE TYPE PARSING
    // ============================================================

    /// Parse: A [Name] (params)? is either [variants]
    fn parse_literate_inductive(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "A" or "An"
        if self.try_consume_keyword("An") || self.try_consume_keyword("A") {
            // Good
        } else {
            return Err(ParseError::Expected {
                expected: "'A' or 'An'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the type name
        let name = self.parse_declaration_name()?;

        self.skip_whitespace();

        // Check for type parameters: "of (T: Type)"
        let params = if self.try_consume_keyword("of") {
            self.skip_whitespace();
            self.parse_param_list()?
        } else {
            vec![]
        };

        self.skip_whitespace();

        // Expect "is either"
        if !self.try_consume_keyword("is") {
            return Err(ParseError::Expected {
                expected: "'is'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume_keyword("either") {
            return Err(ParseError::Expected {
                expected: "'either'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse variants - either inline with "or" or block with indentation
        let constructors = if self.peek_char(':') {
            // Block format: "is either:\n    Zero.\n    Successor with pred: Nat."
            self.advance(); // consume ':'
            self.parse_indented_variants(&name, &params)?
        } else {
            // Inline format: "is either Yes or No."
            self.parse_inline_variants(&name, &params)?
        };

        if constructors.is_empty() {
            return Err(ParseError::Missing("constructors".to_string()));
        }

        Ok(Command::Inductive {
            name,
            params,
            sort: Term::Sort(Universe::Type(0)),
            constructors,
        })
    }

    /// Parse inline variants: "Yes or No" or "Yes or No."
    fn parse_inline_variants(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<Vec<(String, Term)>, ParseError> {
        let mut constructors = Vec::new();

        loop {
            self.skip_whitespace();

            if self.at_end() || self.peek_char('.') {
                break;
            }

            // Parse a single variant
            let (ctor_name, ctor_type) = self.parse_variant(inductive_name, params)?;
            constructors.push((ctor_name, ctor_type));

            self.skip_whitespace();

            // Check for "or" separator
            if !self.try_consume_keyword("or") {
                break;
            }
        }

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        Ok(constructors)
    }

    /// Parse indented variants (after "is either:")
    fn parse_indented_variants(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<Vec<(String, Term)>, ParseError> {
        let mut constructors = Vec::new();

        loop {
            self.skip_whitespace_and_newlines();

            if self.at_end() {
                break;
            }

            // Check for end of block (non-indented line or empty)
            if !self.peek_char(' ')
                && !self.peek_char('\t')
                && !self.peek_char('a')
                && !self.peek_char('A')
            {
                // Check if this is a variant starting with capital letter
                if let Some(c) = self.peek() {
                    if !c.is_uppercase() && c != 'a' && c != 'A' {
                        break;
                    }
                }
            }

            // Skip leading whitespace for indentation
            self.skip_whitespace();

            // Check for "a/an" prefix or capital letter
            if self.at_end() {
                break;
            }

            // Parse variant
            let (ctor_name, ctor_type) = self.parse_variant(inductive_name, params)?;
            constructors.push((ctor_name, ctor_type));

            // Consume trailing period
            self.skip_whitespace();
            let _ = self.try_consume(".");
        }

        Ok(constructors)
    }

    /// Parse a single variant: "Zero" or "a Successor with pred: Nat"
    fn parse_variant(
        &mut self,
        inductive_name: &str,
        params: &[(String, Term)],
    ) -> Result<(String, Term), ParseError> {
        self.skip_whitespace();

        // Skip optional "a" or "an" prefix (for readability)
        let _ = self.try_consume_keyword("an") || self.try_consume_keyword("a");
        self.skip_whitespace();

        // Parse constructor name
        let ctor_name = self.parse_declaration_name()?;

        self.skip_whitespace();

        // Build the result type (possibly with parameters applied)
        let result_type = self.build_applied_type(inductive_name, params);

        // Check for "with" clause (fields)
        if self.try_consume_keyword("with") {
            // Parse fields: "field: Type and field2: Type2"
            let fields = self.parse_field_list()?;

            // Build constructor type: field1_type -> field2_type -> ... -> ResultType
            let mut ctor_type = result_type;
            for (_field_name, field_type) in fields.into_iter().rev() {
                ctor_type = Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(field_type),
                    body_type: Box::new(ctor_type),
                };
            }

            Ok((ctor_name, ctor_type))
        } else {
            // Unit constructor (no fields)
            Ok((ctor_name, result_type))
        }
    }

    /// Parse field list: "field: Type" or "field: Type and field2: Type2"
    fn parse_field_list(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut fields = Vec::new();

        loop {
            self.skip_whitespace();

            // Parse field name
            let field_name = self.parse_ident()?;

            self.skip_whitespace();

            // Expect ":"
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            self.skip_whitespace();

            // Parse field type
            let field_type = self.parse_type()?;

            fields.push((field_name, field_type));

            self.skip_whitespace();

            // Check for "and" separator
            if !self.try_consume_keyword("and") {
                break;
            }
        }

        Ok(fields)
    }

    /// Build type with parameters applied: List T
    fn build_applied_type(&self, name: &str, params: &[(String, Term)]) -> Term {
        let mut result = Term::Global(name.to_string());
        for (param_name, _) in params {
            result = Term::App(Box::new(result), Box::new(Term::Var(param_name.clone())));
        }
        result
    }

    // ============================================================
    // DEFINITION PARSING
    // ============================================================

    /// Parse: ## To [name] (params) -> RetType: body
    fn parse_literate_definition(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "## To "
        if !self.try_consume("## To ") && !self.try_consume("##To ") {
            return Err(ParseError::Expected {
                expected: "'## To'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse function name
        let mut name = self.parse_declaration_name()?;

        // Handle predicate syntax: "## To be Provable" means define "Provable", not "be"
        // This allows "## To be X (params) -> Prop:" for predicate definitions
        if name == "be" {
            self.skip_whitespace();
            name = self.parse_declaration_name()?;
        }

        self.current_function = Some(name.clone());

        self.skip_whitespace();

        // Parse parameter groups: (x: T) and (y: U) or (x: T) (y: U)
        // Note: For nullary predicates like "## To be Consistent -> Prop:", params will be empty
        let all_params = self.parse_function_params()?;

        self.skip_whitespace();

        // Parse optional return type: -> RetType
        let return_type = if self.try_consume("->") {
            self.skip_whitespace();
            let ret = self.parse_type()?;
            // Store return type for use as motive in Consider blocks
            self.return_type = Some(ret.clone());
            Some(ret)
        } else {
            None
        };

        self.skip_whitespace();

        // Expect ":"
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        // Add parameters to bound vars, and to the typed local scope the equation compiler
        // reads the discriminant's type out of.
        for (param_name, param_type) in &all_params {
            self.bound_vars.insert(param_name.clone());
            self.locals.push((param_name.clone(), param_type.clone()));
        }

        // Parse body
        self.skip_whitespace_and_newlines();
        let body = self.parse_body()?;

        // Check for self-reference (implicit fixpoint)
        let needs_fix = self.contains_self_reference(&name, &body);

        // Build the function body with lambdas
        let mut func_body = body;
        for (param_name, param_type) in all_params.iter().rev() {
            func_body = Term::Lambda {
                param: param_name.clone(),
                param_type: Box::new(param_type.clone()),
                body: Box::new(func_body),
            };
        }

        // Wrap in Fix if recursive
        if needs_fix {
            func_body = Term::Fix {
                name: name.clone(),
                body: Box::new(func_body),
            };
        }

        // Build the full type annotation if we have a return type
        let ty = if let Some(ret) = return_type {
            let mut full_type = ret;
            for (_, param_type) in all_params.iter().rev() {
                full_type = Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(param_type.clone()),
                    body_type: Box::new(full_type),
                };
            }
            Some(full_type)
        } else {
            None
        };

        self.current_function = None;

        Ok(Command::Definition {
            name,
            ty,
            body: func_body,
            is_hint: false,
            implicit_count: 0,
            proof_goal: None,
        })
    }

    // ============================================================
    // LET DEFINITION PARSING
    // ============================================================

    /// Parse: Let [name] be [term].
    fn parse_literate_let(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "Let "
        if !self.try_consume_keyword("Let") {
            return Err(ParseError::Expected {
                expected: "'Let'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the name
        let name = self.parse_declaration_name()?;

        self.skip_whitespace();

        // Expect "be"
        if !self.try_consume_keyword("be") {
            return Err(ParseError::Expected {
                expected: "'be'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the body term
        let body = self.parse_term()?;

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        Ok(Command::Definition {
            name,
            ty: None,
            body,
            is_hint: false,
            implicit_count: 0,
            proof_goal: None,
        })
    }

    /// Parse: ## Theorem: [name]\n    Statement: [proposition].\n    Proof: [tactic].
    ///
    /// The Proof section is optional. When provided, it applies a tactic like `ring.`
    /// to automatically prove the statement.
    fn parse_literate_theorem(&mut self) -> Result<Command, ParseError> {
        self.skip_whitespace();

        // Consume "## Theorem:"
        if !self.try_consume("## Theorem:") {
            return Err(ParseError::Expected {
                expected: "'## Theorem:'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the theorem name
        let name = self.parse_declaration_name()?;

        self.skip_whitespace_and_newlines();

        // Expect "Statement:"
        if !self.try_consume_keyword("Statement") {
            return Err(ParseError::Expected {
                expected: "'Statement'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        self.skip_whitespace();

        // Parse the statement (proposition)
        let statement = self.parse_term()?;

        // Consume trailing period if present
        self.skip_whitespace();
        let _ = self.try_consume(".");

        self.skip_whitespace_and_newlines();

        // Check for optional "Proof:" section
        let (body, ty, proof_goal) = if self.try_consume_keyword("Proof") {
            self.skip_whitespace();
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            // Parse the proof tactic. It returns BOTH the derivation and the goal, in `Syntax`
            // form, that the derivation must CONCLUDE — the REPL rejects the theorem unless
            // `concludes body` reduces to exactly that. The goal is computed from the STATEMENT
            // (never from the proof), but which rendering of the statement it is depends on the
            // tactic: an ordinary tactic discharges the prenex binders as eigenvariables and is
            // handed the open body, while `induction` keeps the binder and is handed the
            // `Forall`-quantified form. Both are faithful readings of the same proposition; each
            // matches the shape its own derivation concludes.
            let (proof, goal_syntax) = self.parse_proof_tactic(&statement)?;

            // Consume trailing period if present
            self.skip_whitespace();
            let _ = self.try_consume(".");

            (
                proof,
                Some(Term::Global("Derivation".to_string())),
                Some(goal_syntax),
            )
        } else {
            // No proof - existing behavior (statement as body, Prop as type)
            (statement, Some(Term::Sort(Universe::Prop)), None)
        };

        // Check for optional "Attribute: hint." section
        self.skip_whitespace_and_newlines();
        let is_hint = if self.try_consume_keyword("Attribute") {
            self.skip_whitespace();
            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let hint = self.try_consume_keyword("hint");

            // Consume trailing period
            self.skip_whitespace();
            let _ = self.try_consume(".");

            hint
        } else {
            false
        };

        Ok(Command::Definition {
            name,
            ty,
            body,
            is_hint,
            implicit_count: 0,
            proof_goal,
        })
    }

    /// Consume one STEP of a proof script or of a bullet case, returning the step's NAME — the
    /// vocabulary [`eval_script_names`](crate::reduction) reads — or `None`, position unmoved,
    /// when the next token is not a step. Consumes the trailing period.
    ///
    /// The vocabulary is [`ONE_SHOT_TACTICS`] plus `intro`, read off the table rather than
    /// re-listed: a tactic added to the table but forgotten here would be writable as
    /// `Proof: <t>.` and silently unreadable as a script step, which stops the parse mid-proof
    /// and reports the REMAINDER as trailing input — blaming the author's next line for a hole
    /// in ours.
    ///
    /// `intro` may name the variable it introduces, as it does in Lean. The name is a note to
    /// the reader — the script evaluator introduces the antecedent either way — so it is read
    /// and dropped rather than being allowed to derail the parse.
    fn parse_step_name(&mut self, allow_intro_binder: bool) -> Option<String> {
        for kw in std::iter::once("intro").chain(ONE_SHOT_TACTICS.iter().map(|(k, _)| *k)) {
            if self.try_consume_keyword(kw) {
                self.skip_whitespace();
                if allow_intro_binder && kw == "intro" && !self.peek_char('.') && !self.at_end() {
                    let _ = self.parse_ident();
                    self.skip_whitespace();
                }
                let _ = self.try_consume(".");
                return Some(kw.to_string());
            }
        }
        None
    }

    /// One step of a top-level proof SCRIPT (after `intro.`), which also swallows the newline
    /// before the next step.
    fn parse_script_step_name(&mut self) -> Option<String> {
        let name = self.parse_step_name(false)?;
        self.skip_whitespace_and_newlines();
        Some(name)
    }

    /// Parse a proof tactic and return BOTH the derivation and the goal it must conclude.
    ///
    /// The closed tactic vocabulary is [`ONE_SHOT_TACTICS`]; `intro` opens a proof SCRIPT and
    /// `induction <var>` opens a bullet-per-constructor elimination. Every arm is handed the same
    /// reified goal, so a new tactic is one row in the table rather than a new branch.
    fn parse_proof_tactic(&mut self, statement: &Term) -> Result<(Term, Term), ParseError> {
        // The goal as an ORDINARY tactic sees it: the prenex `forall` binders are discharged as
        // eigenvariables (see [`split_universal_prefix`]), which is exactly the reading this
        // surface already gives a free variable — `ring` proves `x + y equals y + x` because `x`
        // and `y` are opaque atoms no tactic can instantiate.
        let (binders, open_body) = split_universal_prefix(statement);
        let open_body = to_eigenvariables(self.ctx, open_body, &binders, None);
        let goal_syntax = self.term_to_syntax(&open_body, &[]);

        for (keyword, builder) in ONE_SHOT_TACTICS {
            if self.try_consume_keyword(keyword) {
                self.skip_whitespace();
                let _ = self.try_consume(".");
                return Ok((apply1(builder, goal_syntax.clone()), goal_syntax));
            }
        }

        if self.try_consume_keyword("intro") {
            // Sequencing: `intro. … closer.` → `try_script <name-list> <goal>`. Collect the tactic
            // names (`intro` first, then the rest of the script); the script is evaluated against
            // the threaded residual goal + hypotheses at conclusion time.
            self.skip_whitespace();
            let _ = self.try_consume(".");
            self.skip_whitespace_and_newlines();
            let mut names = vec!["intro".to_string()];
            while let Some(name) = self.parse_script_step_name() {
                names.push(name);
            }
            let list = build_script_name_list(&names);
            let proof = Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("try_script".to_string())),
                    Box::new(list),
                )),
                Box::new(goal_syntax.clone()),
            );
            return Ok((proof, goal_syntax));
        }

        if self.try_consume_keyword("induction") {
            self.skip_whitespace();
            let var_name = self.parse_ident()?;
            self.skip_whitespace();
            let _ = self.try_consume(".");
            return self.parse_induction(&var_name, statement);
        }

        Err(ParseError::Expected {
            expected: "proof tactic (ring, refl, lia, cc, simp, omega, auto, intro, induction)"
                .to_string(),
            found: self.peek_word().unwrap_or("EOF".to_string()),
        })
    }

    /// Check if the next non-whitespace character is a bullet point
    fn peek_bullet(&mut self) -> bool {
        // Save position
        let saved_pos = self.pos;
        self.skip_whitespace_and_newlines();
        let result = matches!(self.peek(), Some('-') | Some('*') | Some('+'));
        self.pos = saved_pos;
        result
    }

    /// Consume a bullet point character
    fn consume_bullet(&mut self) {
        self.skip_whitespace_and_newlines();
        if matches!(self.peek(), Some('-') | Some('*') | Some('+')) {
            self.advance();
        }
    }

    /// Parse a bullet case's tactics (`simp. auto.`) closing ONE goal.
    ///
    /// A sequence reifies as `try_script <name-list> <goal>` — the SAME mechanism a top-level
    /// `Proof: intro. <closer>.` uses, evaluated by the same `eval_script_names`. A bullet case
    /// is a proof script scoped to one constructor; giving it a second sequencing form would be
    /// two evaluators for one idea, and the one it had (`tact_seq`, plus `try_intro` for the
    /// `intro` step) was declared nowhere and reduced nowhere — so every bullet with more than
    /// one tactic failed with `UnboundVariable` naming a constant the author never wrote.
    ///
    /// A LONE one-shot tactic stays the direct application it has always been: `try_refl <goal>`
    /// carries its own proof object, and wrapping it would change every derivation the suite
    /// already pins.
    ///
    /// `lead_intros` is the count of binders the elimination introduces BEFORE the author's
    /// first tactic runs — see [`LiterateParser::case_binder_count`]. It is 0 for a constructor
    /// with no fields, which is every constructor of every enumeration, so the one-shot form is
    /// unchanged wherever it was reachable before.
    fn parse_tactic_sequence(
        &mut self,
        goal_syntax: &Term,
        lead_intros: usize,
    ) -> Result<Term, ParseError> {
        let mut names: Vec<String> = Vec::new();

        loop {
            self.skip_whitespace();

            // Stop at newline followed by bullet, end of input, or Attribute keyword
            if self.peek_bullet() || self.at_end() || self.peek_keyword("Attribute") {
                break;
            }

            match self.parse_step_name(true) {
                Some(name) => names.push(name),
                None => break,
            }
        }

        if names.is_empty() {
            return Err(ParseError::Missing("tactic in bullet case".to_string()));
        }

        if lead_intros == 0 {
            if let [only] = names.as_slice() {
                if let Some((_, builder)) = ONE_SHOT_TACTICS.iter().find(|(k, _)| k == only) {
                    return Ok(apply1(builder, goal_syntax.clone()));
                }
            }
        } else {
            let mut with_intros: Vec<String> = vec!["intro".to_string(); lead_intros];
            with_intros.extend(names);
            names = with_intros;
        }

        Ok(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("try_script".to_string())),
                Box::new(build_script_name_list(&names)),
            )),
            Box::new(goal_syntax.clone()),
        ))
    }

    /// How many binders the elimination introduces for one bullet, BEFORE the author's tactic
    /// sees the goal.
    ///
    /// ★ THIS IS LEAN'S `introN`, COUNTED LEAN'S WAY. Lean's minor premise for a constructor is
    /// the CLOSED formula `∀f₁ … fₙ, (IH …) → motive (C f⃗)`; the branch the author writes never
    /// sees it. `Lean/Meta/Tactic/Induction.lean:97-106` computes
    /// `nparams := getTargetArity d - initialArity` — the arity of the MINOR PREMISE'S TYPE less
    /// the arity the goal already had — and then `mvarId'.introN nparams …` moves exactly that
    /// many binders into the local context, so `| succ k ih => rfl` is handed the OPEN goal
    /// `motive (Nat.succ k)` with `k` fixed-but-arbitrary and `ih : motive k` in scope.
    ///
    /// This is the same subtraction, against the same authority. The count is read off the case
    /// goal the KERNEL computes (`induction_step_goal ind motive i`, reduced here), less the
    /// depth the motive body already had — never off a second walk of the constructor's
    /// telescope. Two computations of one fact is how a compiler and its checker drift apart;
    /// `build_case_expected` stays the only thing that decides what a case formula IS.
    ///
    /// Without a context the count is 0, and the text-only door keeps exactly the reading it
    /// had: it cannot be told a constructor's field count any more than it can be told the
    /// constructor COUNT, so it emits no introductions and its `induction` closes only cases
    /// whose constructor carries no fields. That is the same limitation [`parse_theorem`]
    /// already documents, not a second one.
    fn case_binder_count(&self, case_goal: &Term, motive_body: &Term) -> usize {
        let Some(ctx) = self.ctx else { return 0 };
        let reduced = normalize(ctx, case_goal);
        let base = normalize(ctx, motive_body);
        goal_binder_depth(&reduced).saturating_sub(goal_binder_depth(&base))
    }

    /// Parse `induction <var>.` plus its bullet cases, and return the elimination derivation
    /// together with the goal it concludes.
    ///
    /// Three things decide the shape, and all three used to be wrong:
    ///
    /// * **The variable is read.** The motive ABSTRACTS it — `SLam T body` with the variable's
    ///   occurrences at de Bruijn 0 — which is what `try_delim_conclude` opens with
    ///   `extract_slam_body`. A motive with no binder makes every literate induction conclude the
    ///   `SName "Error"` sentinel, so it could close no goal of any type.
    /// * **The inductive type is COMPUTED, from the binder's own sort** (`forall n : Colour, …`
    ///   inducts over `Colour`), falling back to `Nat` only when the statement quantifies nothing
    ///   — the case where the variable is a free name and the surface's reading of a free
    ///   variable is "universally quantified".
    /// * **Each bullet gets ITS OWN case goal**, `induction_step_goal ind motive i`, computed by
    ///   the kernel from the i-th constructor. Handing every bullet the whole theorem statement
    ///   made the cases compare against a proposition no constructor case ever equals.
    fn parse_induction(
        &mut self,
        var_name: &str,
        statement: &Term,
    ) -> Result<(Term, Term), ParseError> {
        let (binders, open_body) = split_universal_prefix(statement);
        // The inductive comes from the binder's OWN sort. A statement that quantifies but not over
        // this name is REFUSED rather than defaulted: the default would eliminate over `Nat` with
        // a motive that ignores the binder the author wrote, so the proposition CERTIFIED would be
        // `∀_:Nat, P(h#eig)` while the statement said `∀h:Hue, P h`. Today the `Succ` case blocks
        // every such proof by accident; the moment a ∀-aware bullet step exists it would not, and
        // the divergence between what is written and what is proved would be reachable.
        //
        // A statement with NO prenex binder at all keeps the `Nat` reading: there the surface's
        // own rule for a free variable is "universally quantified", and `Nat` is the quantifier it
        // is given.
        let sort = match binders.iter().find(|(name, _)| name == var_name) {
            Some((_, ty)) => ty.clone(),
            None if binders.is_empty() => Term::Global("Nat".to_string()),
            None => {
                return Err(ParseError::Expected {
                    expected: format!(
                        "a quantified variable to induct on; this statement binds {}",
                        binders
                            .iter()
                            .map(|(n, t)| format!("`{n} : {t}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    found: var_name.to_string(),
                })
            }
        };

        let ind_type = self.term_to_syntax(&sort, &[]);
        // An unquantified statement reads its free names as universally quantified, so the
        // variable inducted on is one of them, spelled as a free NAME: it is abstracted exactly as
        // a quantified one is, or the motive would ignore it and every case would restate the
        // theorem instead of its instance. A declared constant of that spelling is not a free
        // variable, and a binder of that spelling inside the statement would capture it.
        let unquantified;
        let open_body = if binders.is_empty() {
            if self
                .ctx
                .is_some_and(|ctx| ctx.get_global(var_name).is_some())
            {
                return Err(ParseError::InvalidConsiderSyntax(format!(
                    "`induction {var_name}` needs a VARIABLE to eliminate; `{var_name}` is a \
                     declared constant"
                )));
            }
            unquantified = free_name_as_variable(open_body, var_name).ok_or_else(|| {
                ParseError::InvalidConsiderSyntax(format!(
                    "`induction {var_name}`: the statement binds `{var_name}` itself, so its free \
                     `{var_name}` cannot be abstracted without being captured; quantify it with \
                     `forall` instead"
                ))
            })?;
            &unquantified
        } else {
            open_body
        };
        // The variable being eliminated is ABSTRACTED (de Bruijn 0 in the motive); every OTHER
        // prenex binder is discharged as an eigenvariable, exactly as an ordinary tactic's goal is.
        let open_body = to_eigenvariables(self.ctx, open_body, &binders, Some(var_name));
        let bound = [var_name.to_string()];
        let motive_body = self.term_to_syntax(&open_body, &bound);
        let motive = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SLam".to_string())),
                Box::new(ind_type.clone()),
            )),
            Box::new(motive_body.clone()),
        );

        // One bullet per constructor, each closing the goal the kernel derives for that
        // constructor from the motive — with the constructor's fields and induction hypotheses
        // ALREADY INTRODUCED, which is what Lean's `| succ k ih => …` hands its branch.
        let mut cases = Vec::new();
        while self.peek_bullet() {
            self.consume_bullet();
            self.skip_whitespace();
            let case_goal = case_goal_term(&ind_type, &motive, cases.len());
            let lead_intros = self.case_binder_count(&case_goal, &motive_body);
            cases.push(self.parse_tactic_sequence(&case_goal, lead_intros)?);
        }

        // With a context, the elimination is checked against the REAL constructor list here,
        // where the source text is still in hand. `DElim` also checks it, but by then the only
        // thing it can say is that nothing was concluded — the count, the inductive and the
        // constructor names have all been left behind.
        if let Some(ctx) = self.ctx {
            let Some(ind_name) = type_head_name(&sort) else {
                return Err(ParseError::InvalidConsiderSyntax(format!(
                    "`induction {var_name}` needs an INDUCTIVE to eliminate over; `{var_name}` \
                     has type `{sort}`, which is not a type application"
                )));
            };
            let ctors = ctx.get_constructors(ind_name);
            if ctors.is_empty() {
                return Err(ParseError::InvalidConsiderSyntax(format!(
                    "`induction {var_name}` needs an INDUCTIVE to eliminate over; `{ind_name}` \
                     has no constructors"
                )));
            }
            if cases.len() != ctors.len() {
                return Err(ParseError::InvalidConsiderSyntax(format!(
                    "`{ind_name}` has {} constructor(s) — {} — so this proof needs {} bullet \
                     case(s), one per constructor, in that order; it has {}",
                    ctors.len(),
                    ctors.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
                    ctors.len(),
                    cases.len()
                )));
            }
        }

        // `try_induction ind motive cases` — reduces to `DElim`, which verifies every case
        // conclusion against its constructor before conceding `Forall ind motive`.
        let mut case_chain = Term::Global("DCaseEnd".to_string());
        for case_proof in cases.into_iter().rev() {
            case_chain = Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("DCase".to_string())),
                    Box::new(case_proof),
                )),
                Box::new(case_chain),
            );
        }
        let proof = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("try_induction".to_string())),
                    Box::new(ind_type.clone()),
                )),
                Box::new(motive.clone()),
            )),
            Box::new(case_chain),
        );
        Ok((proof, forall_syntax(&ind_type, &motive)))
    }

    /// Parse function parameters: (x: T) and (y: U) or (x: T) (y: U)
    fn parse_function_params(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut params = Vec::new();

        loop {
            self.skip_whitespace();

            // Check for opening paren
            if !self.peek_char('(') {
                break;
            }

            // Parse one parameter group
            self.advance(); // consume '('
            self.skip_whitespace();

            let param_name = self.parse_ident()?;
            self.skip_whitespace();

            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let param_type = self.parse_type()?;
            self.skip_whitespace();

            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            params.push((param_name, param_type));

            self.skip_whitespace();

            // Check for "and" separator
            let _ = self.try_consume_keyword("and");
        }

        Ok(params)
    }

    /// Parse optional type parameters: (T: Type)
    fn parse_param_list(&mut self) -> Result<Vec<(String, Term)>, ParseError> {
        let mut params = Vec::new();

        if !self.peek_char('(') {
            return Ok(params);
        }

        self.advance(); // consume '('
        self.skip_whitespace();

        loop {
            let param_name = self.parse_ident()?;
            self.skip_whitespace();

            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();

            let param_type = self.parse_type()?;
            params.push((param_name, param_type));

            self.skip_whitespace();

            if self.try_consume(")") {
                break;
            }

            if !self.try_consume(",") && !self.try_consume_keyword("and") {
                return Err(ParseError::Expected {
                    expected: "')' or ','".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
        }

        Ok(params)
    }

    // ============================================================
    // BODY / TERM PARSING
    // ============================================================

    /// Parse function body (indented block after ":")
    ///
    /// A body may begin on the line after its `:` — `When Succ k:` with a nested `Consider` written
    /// beneath it is the ordinary layout for a two-level equation set — so the line break is
    /// skipped. What follows must then be a body: a `When` there means the clause before it has
    /// none, and that is said here rather than read as an application of a global named `When`.
    fn parse_body(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace_and_newlines();
        if self.peek_keyword("When") {
            return Err(ParseError::Expected {
                expected:
                    "a clause body (`Yield …` or a nested `Consider …`) before the next `When`"
                        .to_string(),
                found: "When".to_string(),
            });
        }

        // Check for Consider (pattern matching)
        if self.peek_keyword("Consider") {
            return self.parse_consider();
        }

        // Check for Yield (direct return)
        if self.peek_keyword("Yield") {
            return self.parse_yield();
        }

        // Check for "given" lambda
        if self.peek_keyword("given") {
            return self.parse_given_lambda();
        }

        // Check for pipe lambda |x: T| -> body
        if self.peek_char('|') {
            return self.parse_pipe_lambda();
        }

        // Otherwise, parse as a term expression
        self.parse_term()
    }

    /// Parse: `Consider x: When <pattern>: <body>. …`
    ///
    /// This is the equation-set surface, and with a context it is compiled as one:
    /// [`super::match_compile::compile_clauses`] turns the clause matrix into a tree of the
    /// kernel's positional `Match` nodes. Three things that reading follows and the flat one
    /// cannot:
    ///
    /// * A clause goes in the slot of the constructor it NAMES. `Bool` is registered `true` then
    ///   `false`, so `When false: … When true: …` must not compute the other function — which is
    ///   what pushing one case per clause in source order does for every equal-arity inductive.
    /// * A pattern NESTS. `When Succ (Succ k):` is a two-level match, not a parse error.
    /// * `When _:` is a catch-all, fanned out over every constructor with no clause of its own,
    ///   the body replicated and re-abstracted over each one's fields.
    /// * A clause set matches SEVERAL COLUMNS at once. `Consider a and b:` with
    ///   `When Zero and _:` is Lean's `| 0, _ => …`, and it is the shape an overlap is actually
    ///   written in: `pick 0 _ = 1 | _ 0 = 2 | _ _ = 3` splits into `match a with | Zero => 1 |
    ///   Succ _ => match b with | Zero => 2 | Succ _ => 3`. One column is the degenerate case of
    ///   the same matrix, so this is not a second code path.
    ///
    /// Overlapping clauses resolve TOP-TO-BOTTOM, and a constructor no clause can reach is a
    /// non-exhaustiveness error rather than an invented value: a LOGOS definition is total.
    fn parse_consider(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("Consider")?;
        self.skip_whitespace();

        // THE COLUMNS. `and` separates them, exactly as it separates the parameters they usually
        // are (`## To pick (a: Nat) and (b: Nat) -> Nat:`), and nothing in the term grammar reads
        // it — application, comparison and the two infix connectives all stop before it — so a
        // one-column `Consider` parses byte for byte as it always did.
        let mut discriminants = vec![self.parse_term()?];
        loop {
            self.skip_whitespace();
            if !self.try_consume_keyword("and") {
                break;
            }
            self.skip_whitespace();
            discriminants.push(self.parse_term()?);
        }

        self.skip_whitespace();

        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }

        // The result type is the motive at every node of the compiled tree. The type checker
        // wraps a raw type as the constant motive `λ_:I. T`.
        let result_type = self
            .return_type
            .clone()
            .unwrap_or_else(|| Term::Sort(Universe::Type(0)));

        // With a context each column's own type is known here, which is what lets a clause's
        // binders enter scope with their REAL types before its body is read — a nested
        // `Consider` inside a clause body needs exactly that.
        let working = self.working_context();
        let disc_tys: Option<Vec<Term>> = match &working {
            Some(c) => Some(
                discriminants
                    .iter()
                    .map(|d| {
                        infer_type(c, d).map_err(|e| {
                            ParseError::InvalidConsiderSyntax(format!(
                                "`Consider {d}` cannot be compiled: the discriminant has no type \
                                 here ({e})"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => None,
        };

        let mut clauses: Vec<Clause> = Vec::new();
        // THE OFFSIDE RULE. A `Consider`'s first `When` fixes its clause column: its own column
        // when it begins a line, and — when it is written beside the `Consider` — the column just
        // past the indentation of the line they share, so the siblings are the clauses indented
        // beneath that line. A later `When` that begins a line to the LEFT of the fixed column
        // belongs to an enclosing `Consider`, and ends this one. Without it a nested equation set
        // written beneath (or beside) `When Succ k:` would swallow every clause of the set around
        // it, and the outer function would silently lose its remaining cases to the inner match.
        // A `When` that does not begin a line is read as it always was: on one line there is no
        // layout.
        let mut clause_column: Option<usize> = None;

        loop {
            self.skip_whitespace_and_newlines();

            if !self.peek_keyword("When") {
                break;
            }
            let column = self.line_start_column();
            if clauses.is_empty() {
                clause_column = Some(column.unwrap_or_else(|| self.line_indent() + 1));
            } else if let (Some(fixed), Some(here)) = (clause_column, column) {
                if here < fixed {
                    break;
                }
            }

            self.consume_keyword("When")?;
            self.skip_whitespace();

            // ONE PATTERN PER COLUMN, separated the way the columns were.
            let mut patterns = vec![self.parse_clause_pattern()?];
            loop {
                self.skip_whitespace();
                if !self.try_consume_keyword("and") {
                    break;
                }
                self.skip_whitespace();
                patterns.push(self.parse_clause_pattern()?);
            }
            self.skip_whitespace();

            if !self.try_consume(":") {
                return Err(ParseError::Expected {
                    expected: "':'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }

            // A column count mismatch is caught HERE, where the source text is still in hand.
            // `compile_clauses` checks it too and is the authority — but by then the clause is a
            // row of a matrix, and what a reader would get back names neither the `Consider` line
            // nor the `When` that disagrees with it. This is the same division of labour the
            // bullet-count check in `parse_induction` already draws.
            if patterns.len() != discriminants.len() {
                return Err(ParseError::InvalidConsiderSyntax(format!(
                    "`Consider` names {} column(s) — {} — so every `When` needs {} pattern(s) \
                     separated by `and`; this one has {}: `{}`",
                    discriminants.len(),
                    discriminants
                        .iter()
                        .map(|d| format!("`{d}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    discriminants.len(),
                    patterns.len(),
                    patterns
                        .iter()
                        .map(Pattern::render)
                        .collect::<Vec<_>>()
                        .join(" and ")
                )));
            }

            self.skip_whitespace();

            // The binders this clause introduces, with the types the constructor gives them.
            // `pattern_binder_types` is the same telescope walk the compiler itself does — one
            // computation of one fact, so the scope a body is read in and the scope it is
            // compiled in cannot drift.
            let typed: Vec<(String, Term)> = match (&working, &disc_tys) {
                (Some(c), Some(tys)) => {
                    let mut out = Vec::new();
                    for (t, pattern) in tys.iter().zip(&patterns) {
                        out.extend(
                            pattern_binder_types(c, t, pattern)
                                .map_err(|e| ParseError::InvalidConsiderSyntax(e.to_string()))?,
                        );
                    }
                    out
                }
                _ => patterns
                    .iter()
                    .flat_map(Pattern::binders)
                    .map(|n| (n, Term::Global("_".to_string())))
                    .collect(),
            };

            let mark = self.locals.len();
            let mut newly_bound: Vec<String> = Vec::new();
            for (n, t) in &typed {
                if self.bound_vars.insert(n.clone()) {
                    newly_bound.push(n.clone());
                }
                self.locals.push((n.clone(), t.clone()));
            }

            let body = self.parse_body()?;

            // Only the names THIS clause introduced leave scope — an outer binder it shadowed
            // stays bound.
            for n in &newly_bound {
                self.bound_vars.remove(n);
            }
            self.locals.truncate(mark);

            clauses.push(Clause { patterns, body });

            self.skip_whitespace();
            let _ = self.try_consume(".");
        }

        if clauses.is_empty() {
            return Err(ParseError::Missing("When clauses".to_string()));
        }

        if self.measuring {
            return Ok(Term::Hole);
        }
        match working {
            Some(c) => compile_clauses_reporting(&c, &discriminants, &result_type, &clauses)
                .map(|(term, _report)| term)
                .map_err(|e| ParseError::InvalidConsiderSyntax(e.to_string())),
            None => flat_positional_match(discriminants, result_type, &clauses),
        }
    }

    /// Parse one `When` clause's pattern.
    ///
    /// ```text
    /// pattern := '_' | name atom*  | '(' pattern ')'
    /// atom    := '_' | name | '(' pattern ')'
    /// ```
    ///
    /// A bare name is a CONSTRUCTOR if the context says so and a variable otherwise, which is
    /// Lean's rule. Without a context the head is read as a constructor name and an argument as a
    /// binder — the flat reading, unchanged.
    fn parse_clause_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.skip_whitespace();
        if self.peek_char('(') {
            return self.parse_parenthesised_pattern();
        }
        if let Some(n) = self.parse_pattern_numeral() {
            return Ok(Pattern::Numeral(n));
        }
        let name = self.parse_ident()?;
        if name == "_" {
            return Ok(Pattern::Wildcard);
        }

        let mut args = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_end()
                || self.peek_char(':')
                || self.peek_char(')')
                || self.peek_char('.')
                // `and` separates COLUMNS, so it ends this pattern rather than being read as one
                // more field of it. It is a keyword of this surface everywhere else it appears
                // (parameter groups, field lists), so nothing that was a binder stops being one.
                || self.peek_keyword("and")
            {
                break;
            }
            args.push(self.parse_pattern_atom()?);
        }

        // A head with no arguments is a constructor when the name denotes one; a context-free
        // parser has to assume it is one, which is what the flat reading always did.
        match (self.ctx, self.constructor_named(&name)?) {
            (Some(_), Some(key)) => Ok(Pattern::Ctor { name: key, args }),
            (Some(_), None) if args.is_empty() => Ok(Pattern::Var(name)),
            _ => Ok(Pattern::Ctor { name, args }),
        }
    }

    /// The constructor a pattern name denotes, when it denotes one — resolved like every other
    /// name the author wrote, so a clause lands in the slot of the constructor its name MEANS here.
    /// A name that denotes no constructor is a binder, and keeps its spelling as one. A name two
    /// readings of which are constructors is ambiguous and says so.
    fn constructor_named(&self, written: &str) -> Result<Option<String>, ParseError> {
        let Some(ctx) = self.ctx else { return Ok(None) };
        let Some(name) = Name::parse(written) else {
            return Ok(ctx.is_constructor(written).then(|| written.to_string()));
        };
        match with_env(ctx, &[], |env| env.resolve(&self.scope, &name)) {
            Resolution::Found(key) => Ok(ctx.is_constructor(&key).then_some(key)),
            Resolution::Unresolved(_) => Ok(None),
            Resolution::Ambiguous {
                written,
                candidates,
            } => {
                if candidates.iter().any(|c| ctx.is_constructor(c)) {
                    Err(ParseError::AmbiguousName {
                        written,
                        candidates,
                    })
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// A NUMERAL in pattern position — `When 0:`, `When Succ 1:` — read as a run of digits, or
    /// `None` with the position unmoved.
    ///
    /// Lean writes `| 0 => 10 | 1 => 20 | n+2 => n` and expands the literals into constructor
    /// applications inside the matrix compiler (`Lean/Meta/Match/Match.lean:629`
    /// `expandNatValuePattern`). The same split is kept here: this door only RECORDS that a
    /// numeral was written, and [`super::match_compile::Pattern::Numeral`] is expanded where the
    /// column's inductive is finally known. A parser that expanded it would have to guess the
    /// constructor names, which is the guess that mis-slots a clause.
    fn parse_pattern_numeral(&mut self) -> Option<u64> {
        if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        let start = self.pos;
        let mut digits = String::new();
        while let Some(c) = self.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            digits.push(c);
            self.advance();
        }
        match digits.parse::<u64>() {
            Ok(n) => Some(n),
            // A numeral too large for the unary spelling is not a numeral pattern; leave the
            // position untouched so the ordinary reading reports it.
            Err(_) => {
                self.pos = start;
                None
            }
        }
    }

    fn parse_pattern_atom(&mut self) -> Result<Pattern, ParseError> {
        self.skip_whitespace();
        if self.peek_char('(') {
            return self.parse_parenthesised_pattern();
        }
        if let Some(n) = self.parse_pattern_numeral() {
            return Ok(Pattern::Numeral(n));
        }
        let name = self.parse_ident()?;
        if name == "_" {
            return Ok(Pattern::Wildcard);
        }
        match self.constructor_named(&name)? {
            Some(key) => Ok(Pattern::Ctor {
                name: key,
                args: Vec::new(),
            }),
            None => Ok(Pattern::Var(name)),
        }
    }

    fn parse_parenthesised_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.advance(); // '('
        let inner = self.parse_clause_pattern()?;
        self.skip_whitespace();
        if !self.try_consume(")") {
            return Err(ParseError::Expected {
                expected: "')' closing a pattern".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        Ok(inner)
    }

    /// Parse: Yield expression
    fn parse_yield(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("Yield")?;
        self.skip_whitespace();
        self.parse_term()
    }

    /// Parse: given x: Type yields body
    fn parse_given_lambda(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("given")?;
        self.skip_whitespace();

        let param = self.parse_ident()?;
        self.skip_whitespace();

        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        let param_type = self.parse_type()?;
        self.skip_whitespace();

        self.consume_keyword("yields")?;
        self.skip_whitespace();

        self.bound_vars.insert(param.clone());
        let body = self.parse_term()?;
        self.bound_vars.remove(&param);

        Ok(Term::Lambda {
            param,
            param_type: Box::new(param_type),
            body: Box::new(body),
        })
    }

    /// Parse: |x: Type| -> body
    fn parse_pipe_lambda(&mut self) -> Result<Term, ParseError> {
        self.advance(); // consume '|'
        self.skip_whitespace();

        let param = self.parse_ident()?;
        self.skip_whitespace();

        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        let param_type = self.parse_type()?;
        self.skip_whitespace();

        if !self.try_consume("|") {
            return Err(ParseError::Expected {
                expected: "'|'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        if !self.try_consume("->") {
            return Err(ParseError::Expected {
                expected: "'->'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        self.bound_vars.insert(param.clone());
        let body = self.parse_term()?;
        self.bound_vars.remove(&param);

        Ok(Term::Lambda {
            param,
            param_type: Box::new(param_type),
            body: Box::new(body),
        })
    }

    /// Parse a term expression (handles infix operators like `equals` and `implies`)
    fn parse_term(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();
        // The binder form comes first: it is the ONLY way a literate `Statement:` can quantify,
        // and without it every ∀-theorem has to be written with its binder erased and hoped over.
        if self.peek_keyword("forall") || self.peek_char('\u{2200}') {
            return self.parse_forall();
        }

        let lhs = self.parse_comparison()?;

        self.skip_whitespace();

        // Check for "equals" infix operator: X equals Y → Eq Hole X Y
        if self.peek_keyword("equals") {
            self.consume_keyword("equals")?;
            self.skip_whitespace();
            let rhs = self.parse_comparison()?; // Parse RHS as comparison (not full term to avoid recursion issues)
            return Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(self.global("Eq")?),
                        Box::new(Term::Hole), // Type placeholder (implicit argument)
                    )),
                    Box::new(lhs),
                )),
                Box::new(rhs),
            ));
        }

        // Check for "implies" infix operator at term level too: X implies Y → Pi _ : X, Y
        if self.peek_keyword("implies") {
            self.consume_keyword("implies")?;
            self.skip_whitespace();
            let rhs = self.parse_term()?; // Full term for right-associativity
            return Ok(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(lhs),
                body_type: Box::new(rhs),
            });
        }

        Ok(lhs)
    }

    // ============================================================
    // INFIX OPERATOR PARSING (comparison, additive, multiplicative)
    // ============================================================

    /// Parse `forall x : T, body` (also spelled `\u{2200} x : T, body`) into a named `Pi`.
    ///
    /// The type is REQUIRED. A binder with no sort would leave `induction` nothing to eliminate
    /// over and would let `forall n, …` and a bare free `n` be different spellings of the same
    /// thing; demanding the sort is what makes the binder carry information.
    fn parse_forall(&mut self) -> Result<Term, ParseError> {
        if !(self.try_consume_keyword("forall") || self.try_consume("\u{2200}")) {
            return Err(ParseError::Expected {
                expected: "'forall'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        let param = self.parse_ident()?;
        self.skip_whitespace();
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':' — a `forall` binder must name its type".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        let param_type = self.parse_type()?;
        self.skip_whitespace();
        if !self.try_consume(",") {
            return Err(ParseError::Expected {
                expected: "',' after the `forall` binder".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        let was_bound = self.bound_vars.contains(&param);
        self.bound_vars.insert(param.clone());
        let body_type = self.parse_term()?;
        if !was_bound {
            self.bound_vars.remove(&param);
        }

        Ok(Term::Pi {
            param,
            param_type: Box::new(param_type),
            body_type: Box::new(body_type),
        })
    }

    /// Parse comparison operators: x <= y, x < y, x >= y, x > y
    /// Precedence: lower than arithmetic, higher than equals/implies
    /// Non-associative: a < b < c is not valid
    fn parse_comparison(&mut self) -> Result<Term, ParseError> {
        let lhs = self.parse_additive()?;
        self.skip_whitespace();

        // Check for comparison operators (order matters: >= before >, <= before <)
        let op_name = if self.try_consume("<=") || self.try_consume("≤") {
            Some("le")
        } else if self.try_consume(">=") || self.try_consume("≥") {
            Some("ge")
        } else if self.try_consume("<") {
            Some("lt")
        } else if self.try_consume(">") {
            Some("gt")
        } else {
            None
        };

        if let Some(op) = op_name {
            self.skip_whitespace();
            let rhs = self.parse_additive()?; // Non-associative: parse additive, not comparison
            return Ok(Term::App(
                Box::new(Term::App(Box::new(self.global(op)?), Box::new(lhs))),
                Box::new(rhs),
            ));
        }

        Ok(lhs)
    }

    /// Parse additive operators: x + y, x - y
    /// Left-associative: a + b - c = (a + b) - c
    fn parse_additive(&mut self) -> Result<Term, ParseError> {
        let mut result = self.parse_multiplicative()?;

        loop {
            self.skip_whitespace();

            let op_name = if self.try_consume("+") {
                Some("add")
            } else if self.peek_char('-') && !self.peek_arrow() && !self.peek_negative_number() {
                self.advance(); // consume '-'
                Some("sub")
            } else {
                None
            };

            if let Some(op) = op_name {
                self.skip_whitespace();
                let rhs = self.parse_multiplicative()?;
                result = Term::App(
                    Box::new(Term::App(Box::new(self.global(op)?), Box::new(result))),
                    Box::new(rhs),
                );
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Parse multiplicative operators: x * y
    /// Left-associative: a * b * c = (a * b) * c
    fn parse_multiplicative(&mut self) -> Result<Term, ParseError> {
        let mut result = self.parse_app()?;

        loop {
            self.skip_whitespace();

            if self.try_consume("*") {
                self.skip_whitespace();
                let rhs = self.parse_app()?;
                result = Term::App(
                    Box::new(Term::App(Box::new(self.global("mul")?), Box::new(result))),
                    Box::new(rhs),
                );
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Parse application: f x y or f(x, y)
    fn parse_app(&mut self) -> Result<Term, ParseError> {
        let mut func = self.parse_atom()?;

        loop {
            self.skip_whitespace();

            // Check for tuple-style call: f(x, y)
            if self.peek_char('(') {
                self.advance(); // consume '('
                self.skip_whitespace();

                if !self.peek_char(')') {
                    loop {
                        let arg = self.parse_term()?;
                        func = Term::App(Box::new(func), Box::new(arg));
                        self.skip_whitespace();

                        if self.try_consume(",") {
                            self.skip_whitespace();
                        } else {
                            break;
                        }
                    }
                }

                if !self.try_consume(")") {
                    return Err(ParseError::Expected {
                        expected: "')'".to_string(),
                        found: self.peek_word().unwrap_or("EOF".to_string()),
                    });
                }
                continue;
            }

            // Check for curried application (next token is an atom)
            // Stop before infix operators like `equals`, `implies`, `+`, `-`, `*`, `<`, etc.
            if self.at_end()
                || self.peek_char(')')
                || self.peek_char('.')
                || self.peek_char(',')
                || self.peek_char(':')
                || self.peek_char('|')
                || self.peek_keyword("When")
                || self.peek_keyword("Yield")
                || self.peek_keyword("and")
                || self.peek_keyword("or")
                || self.peek_keyword("equals")
                || self.peek_keyword("implies")
                // Stop at infix arithmetic and comparison operators
                || self.peek_char('+')
                || self.peek_char('*')
                || self.peek_comparison_operator()
                || (self.peek_char('-') && !self.peek_arrow() && !self.peek_negative_number())
            {
                break;
            }

            // Try to parse next atom for curried application
            if let Ok(arg) = self.parse_atom() {
                func = Term::App(Box::new(func), Box::new(arg));
            } else {
                break;
            }
        }

        Ok(func)
    }

    /// Parse an atom: identifier, literal, or parenthesized term
    fn parse_atom(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Check for number literal
        if let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                return self.parse_number();
            }
            if c == '-' {
                // Check for negative number
                let saved_pos = self.pos;
                self.advance();
                if let Some(next) = self.peek() {
                    if next.is_ascii_digit() {
                        self.pos = saved_pos;
                        return self.parse_number();
                    }
                }
                self.pos = saved_pos;
            }
        }

        // Check for string literal
        if self.peek_char('"') {
            return self.parse_string();
        }

        // Check for parenthesized term
        if self.peek_char('(') {
            self.advance();
            let term = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            return Ok(term);
        }

        // Check for "the" prefix (allows "the Successor of ...", "the diagonalization of ...", "the Name X")
        if self.try_consume_keyword("the") {
            self.skip_whitespace();
            // Check for "diagonalization of" special syntax
            if self.try_consume_keyword("diagonalization") {
                self.skip_whitespace();
                if !self.try_consume_keyword("of") {
                    return Err(ParseError::Expected {
                        expected: "'of'".to_string(),
                        found: self.peek_word().unwrap_or("EOF".to_string()),
                    });
                }
                self.skip_whitespace();
                let arg = self.parse_atom()?;
                return Ok(Term::App(
                    Box::new(Term::Global("syn_diag".to_string())),
                    Box::new(arg),
                ));
            }
            // Check for "the Name X" syntax → SName X
            if self.peek_keyword("Name") {
                self.consume_keyword("Name")?;
                self.skip_whitespace();
                let arg = self.parse_atom()?;
                return Ok(Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(arg),
                ));
            }
            // Otherwise continue to parse the identifier after "the"
        }

        // Check for "Name" special syntax: Name "X" → SName "X"
        if self.peek_keyword("Name") {
            self.consume_keyword("Name")?;
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            return Ok(Term::App(
                Box::new(Term::Global("SName".to_string())),
                Box::new(arg),
            ));
        }

        // Check for "Variable" special syntax: Variable 0 → SVar 0
        if self.peek_keyword("Variable") {
            self.consume_keyword("Variable")?;
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            return Ok(Term::App(
                Box::new(Term::Global("SVar".to_string())),
                Box::new(arg),
            ));
        }

        // Check for "Apply" special syntax: Apply(f, x) → SApp f x
        if self.peek_keyword("Apply") {
            self.consume_keyword("Apply")?;
            self.skip_whitespace();
            if !self.try_consume("(") {
                return Err(ParseError::Expected {
                    expected: "'('".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
            let func_arg = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(",") {
                return Err(ParseError::Expected {
                    expected: "','".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            self.skip_whitespace();
            let arg_arg = self.parse_term()?;
            self.skip_whitespace();
            if !self.try_consume(")") {
                return Err(ParseError::Expected {
                    expected: "')'".to_string(),
                    found: self.peek_word().unwrap_or("EOF".to_string()),
                });
            }
            return Ok(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("SApp".to_string())),
                    Box::new(func_arg),
                )),
                Box::new(arg_arg),
            ));
        }

        // Check for "there exists" special syntax
        if self.peek_keyword("there") {
            return self.parse_existential();
        }

        // Parse identifier
        let ident = self.parse_ident()?;

        // Check for "of" suffix (e.g., "Successor of x")
        self.skip_whitespace();
        if self.try_consume_keyword("of") {
            self.skip_whitespace();
            let arg = self.parse_atom()?;
            let func = if self.bound_vars.contains(&ident) {
                Term::Var(ident)
            } else {
                self.global(ident)?
            };
            return Ok(Term::App(Box::new(func), Box::new(arg)));
        }

        // Return as Var if bound, Global otherwise
        if self.bound_vars.contains(&ident) || self.current_function.as_ref() == Some(&ident) {
            Ok(Term::Var(ident))
        } else {
            // Check for special sorts
            match ident.as_str() {
                "Prop" => Ok(Term::Sort(Universe::Prop)),
                "Type" => Ok(Term::Sort(Universe::Type(0))),
                _ => self.global(ident),
            }
        }
    }

    /// Parse: there exists a [var]: [Type] such that [body]
    /// → Ex Type (fun var => body)
    fn parse_existential(&mut self) -> Result<Term, ParseError> {
        self.consume_keyword("there")?;
        self.skip_whitespace();
        self.consume_keyword("exists")?;
        self.skip_whitespace();

        // Optional "a" or "an"
        let _ = self.try_consume_keyword("an") || self.try_consume_keyword("a");
        self.skip_whitespace();

        // Parse variable name
        let var_name = self.parse_ident()?;
        self.skip_whitespace();

        // Expect ":"
        if !self.try_consume(":") {
            return Err(ParseError::Expected {
                expected: "':'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        // Parse the type
        let var_type = self.parse_type()?;
        self.skip_whitespace();

        // Expect "such that"
        if !self.try_consume_keyword("such") {
            return Err(ParseError::Expected {
                expected: "'such'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();
        if !self.try_consume_keyword("that") {
            return Err(ParseError::Expected {
                expected: "'that'".to_string(),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            });
        }
        self.skip_whitespace();

        // The variable is in scope for the body, at its type, exactly as a clause's binders are.
        let was_bound = !self.bound_vars.insert(var_name.clone());
        self.locals.push((var_name.clone(), var_type.clone()));
        let body = self.parse_term();
        self.locals.pop();
        if !was_bound {
            self.bound_vars.remove(&var_name);
        }
        let body = body?;

        // `Ex T (λ var : T. body)`: the binder is bound AT the type written for it, so the body is
        // read with `var` in scope at `T` — `concludes d` for a `d : Derivation` is then what it
        // says, and `Eq`'s carrier is solved from it.
        Ok(Term::App(
            Box::new(Term::App(
                Box::new(self.global("Ex")?),
                Box::new(var_type.clone()),
            )),
            Box::new(Term::Lambda {
                param: var_name,
                param_type: Box::new(var_type),
                body: Box::new(body),
            }),
        ))
    }

    /// Parse a type expression (same as term for now)
    fn parse_type(&mut self) -> Result<Term, ParseError> {
        self.skip_whitespace();

        // Handle "List of T" style
        let base = self.parse_ident()?;

        self.skip_whitespace();

        // Check for "of" (polymorphic application)
        if self.try_consume_keyword("of") {
            self.skip_whitespace();
            let arg = self.parse_type()?;
            return Ok(Term::App(Box::new(self.global(base)?), Box::new(arg)));
        }

        // Check for special sorts
        match base.as_str() {
            "Prop" => Ok(Term::Sort(Universe::Prop)),
            "Type" => Ok(Term::Sort(Universe::Type(0))),
            _ => self.global(base),
        }
    }

    /// Parse a number literal
    fn parse_number(&mut self) -> Result<Term, ParseError> {
        let mut num_str = String::new();

        // Handle negative sign
        if self.peek_char('-') {
            num_str.push('-');
            self.advance();
        }

        // Collect digits
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                self.advance();
            } else {
                break;
            }
        }

        let value: i64 = num_str
            .parse()
            .map_err(|_| ParseError::InvalidNumber(num_str))?;

        Ok(Term::Lit(Literal::Int(value)))
    }

    /// Parse a string literal
    fn parse_string(&mut self) -> Result<Term, ParseError> {
        self.advance(); // consume opening '"'

        let mut content = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.peek() {
                        Some('n') => {
                            content.push('\n');
                            self.advance();
                        }
                        Some('t') => {
                            content.push('\t');
                            self.advance();
                        }
                        Some(c) => {
                            content.push(c);
                            self.advance();
                        }
                        None => return Err(ParseError::UnexpectedEof),
                    }
                }
                Some(c) => {
                    content.push(c);
                    self.advance();
                }
                None => return Err(ParseError::UnexpectedEof),
            }
        }

        Ok(Term::Lit(Literal::Text(content)))
    }

    // ============================================================
    // TERM TO SYNTAX REIFICATION
    // ============================================================

    /// Convert a high-level Term to deeply-embedded Syntax representation.
    /// This is used by proof tactics like `ring.` that operate on Syntax.
    fn term_to_syntax(&self, term: &Term, bound_vars: &[String]) -> Term {
        match term {
            Term::Var(name) => {
                // Convert to SVar with de Bruijn index if bound, else SName
                if let Some(idx) = bound_vars.iter().rev().position(|n| n == name) {
                    Term::App(
                        Box::new(Term::Global("SVar".to_string())),
                        Box::new(Term::Lit(Literal::Int(idx as i64))),
                    )
                } else {
                    // Free variable - treat as SName
                    Term::App(
                        Box::new(Term::Global("SName".to_string())),
                        Box::new(Term::Lit(Literal::Text(name.clone()))),
                    )
                }
            }
            Term::Global(name) => {
                // Global names become SName
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(name.clone()))),
                )
            }
            Term::Const { name, .. } => {
                // A universe-polymorphic reference reifies by its name (reflection never
                // sees level arguments).
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(name.clone()))),
                )
            }
            Term::Lit(Literal::Int(n)) => {
                // Integer literals become SLit
                Term::App(
                    Box::new(Term::Global("SLit".to_string())),
                    Box::new(Term::Lit(Literal::Int(*n))),
                )
            }
            Term::Lit(Literal::Float(_f)) => {
                // Float literals - treat as error for ring (ring doesn't handle floats)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Float".to_string()))),
                )
            }
            Term::Lit(Literal::BigInt(_)) => {
                // BigInt literals do not fit the ring-syntax layer's machine-int SLit.
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_BigInt".to_string()))),
                )
            }
            Term::Lit(Literal::Nat(_)) => {
                // Nat literals do not fit the ring-syntax layer's machine-int SLit.
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Nat".to_string()))),
                )
            }
            Term::Lit(Literal::Duration(_d)) => {
                // Duration literals - treat as error for ring (ring doesn't handle durations)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Duration".to_string()))),
                )
            }
            Term::Lit(Literal::Date(_d)) => {
                // Date literals - treat as error for ring (ring doesn't handle dates)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Date".to_string()))),
                )
            }
            Term::Lit(Literal::Moment(_m)) => {
                // Moment literals - treat as error for ring (ring doesn't handle moments)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error_Moment".to_string()))),
                )
            }
            Term::Lit(Literal::Text(s)) => {
                // Text literals become SLit (wrapped in SName for now)
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text(s.clone()))),
                )
            }
            Term::App(f, x) => {
                // Applications become SApp
                let f_syn = self.term_to_syntax(f, bound_vars);
                let x_syn = self.term_to_syntax(x, bound_vars);
                Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("SApp".to_string())),
                        Box::new(f_syn),
                    )),
                    Box::new(x_syn),
                )
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                // Lambdas become SLam (if we have that constructor)
                let ty_syn = self.term_to_syntax(param_type, bound_vars);
                let mut new_bound = bound_vars.to_vec();
                new_bound.push(param.clone());
                let body_syn = self.term_to_syntax(body, &new_bound);
                Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("SLam".to_string())),
                        Box::new(ty_syn),
                    )),
                    Box::new(body_syn),
                )
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let ty_syn = self.term_to_syntax(param_type, bound_vars);
                let mut new_bound = bound_vars.to_vec();
                new_bound.push(param.clone());
                let body_syn = self.term_to_syntax(body_type, &new_bound);
                if param == "_" {
                    // A non-dependent arrow — the `implies` desugaring — reifies as `SPi`.
                    Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("SPi".to_string())),
                            Box::new(ty_syn),
                        )),
                        Box::new(body_syn),
                    )
                } else {
                    // A NAMED binder is a quantifier, and reifies as the `Forall` the elimination
                    // rules speak: `SApp (SApp (SName "Forall") T) (SLam T body)`. Reifying it as
                    // `SPi` would put the proposition in a shape no conclusion can ever equal.
                    let motive = Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("SLam".to_string())),
                            Box::new(ty_syn.clone()),
                        )),
                        Box::new(body_syn),
                    );
                    forall_syntax(&ty_syn, &motive)
                }
            }
            Term::Sort(Universe::Prop) => {
                // Prop sort becomes SSort UProp
                Term::App(
                    Box::new(Term::Global("SSort".to_string())),
                    Box::new(Term::Global("UProp".to_string())),
                )
            }
            Term::Sort(Universe::Type(n)) => {
                // Type sort becomes SSort (UType n)
                Term::App(
                    Box::new(Term::Global("SSort".to_string())),
                    Box::new(Term::App(
                        Box::new(Term::Global("UType".to_string())),
                        Box::new(Term::Lit(Literal::Int(*n as i64))),
                    )),
                )
            }
            Term::Sort(_) => {
                // Universe-polymorphic levels (Var/Succ/Max) don't arise in ring proofs;
                // reify conservatively as an opaque SSort so the conversion stays total.
                Term::App(
                    Box::new(Term::Global("SSort".to_string())),
                    Box::new(Term::Global("UPoly".to_string())),
                )
            }
            Term::Hole => {
                // Holes default to Int type for ring proofs
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Int".to_string()))),
                )
            }
            Term::Match { .. } | Term::Fix { .. } | Term::MutualFix { .. } | Term::Let { .. } => {
                // Match, Fix, MutualFix, and Let are complex - return error marker
                Term::App(
                    Box::new(Term::Global("SName".to_string())),
                    Box::new(Term::Lit(Literal::Text("Error".to_string()))),
                )
            }
            // The embedded syntax is kernel meaning; an annotation is not part of it.
            Term::Annotated { term, .. } => self.term_to_syntax(term, bound_vars),
        }
    }

    // ============================================================
    // SELF-REFERENCE DETECTION
    // ============================================================

    /// Check if the function name appears in the term (for implicit fixpoint)
    fn contains_self_reference(&self, name: &str, term: &Term) -> bool {
        match term {
            Term::Var(v) => v == name,
            Term::Global(_) => false,
            Term::Const { .. } => false,
            Term::Sort(_) => false,
            Term::Lit(_) => false,
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                self.contains_self_reference(name, param_type)
                    || self.contains_self_reference(name, body_type)
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                self.contains_self_reference(name, param_type)
                    || self.contains_self_reference(name, body)
            }
            Term::App(f, a) => {
                self.contains_self_reference(name, f) || self.contains_self_reference(name, a)
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                self.contains_self_reference(name, discriminant)
                    || self.contains_self_reference(name, motive)
                    || cases.iter().any(|c| self.contains_self_reference(name, c))
            }
            Term::Fix { body, .. } => self.contains_self_reference(name, body),
            Term::MutualFix { defs, .. } => defs
                .iter()
                .any(|(_, b)| self.contains_self_reference(name, b)),
            Term::Let {
                ty, value, body, ..
            } => {
                self.contains_self_reference(name, ty)
                    || self.contains_self_reference(name, value)
                    || self.contains_self_reference(name, body)
            }
            Term::Hole => false, // Holes never contain self-references
            Term::Annotated { term, .. } => self.contains_self_reference(name, term),
        }
    }

    // ============================================================
    // LOW-LEVEL UTILITIES
    // ============================================================

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// The column the cursor stands at when only indentation precedes it on its line — when what
    /// follows BEGINS a line — and `None` when something else is written before it on that line.
    fn line_start_column(&self) -> Option<usize> {
        let line_start = self.input[..self.pos].rfind('\n').map_or(0, |at| at + 1);
        let indent = &self.input[line_start..self.pos];
        indent
            .chars()
            .all(|c| c == ' ' || c == '\t')
            .then_some(indent.len())
    }

    /// The indentation of the line the cursor stands on: the width of the whitespace that opens it.
    fn line_indent(&self) -> usize {
        let line_start = self.input[..self.pos].rfind('\n').map_or(0, |at| at + 1);
        self.input[line_start..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count()
    }

    fn skip_whitespace_and_newlines(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn peek_char(&self, c: char) -> bool {
        self.peek() == Some(c)
    }

    fn peek_keyword(&self, keyword: &str) -> bool {
        if !self.input[self.pos..].starts_with(keyword) {
            return false;
        }
        let after = self.pos + keyword.len();
        if after >= self.input.len() {
            return true;
        }
        let next_char = self.input[after..].chars().next();
        !next_char
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false)
    }

    fn peek_word(&self) -> Option<String> {
        let start = self.pos;
        let mut end = start;
        for c in self.input[start..].chars() {
            if c.is_alphanumeric() || c == '_' {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        if end > start {
            Some(self.input[start..end].to_string())
        } else {
            self.peek().map(|c| c.to_string())
        }
    }

    fn advance(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn try_consume(&mut self, s: &str) -> bool {
        if self.input[self.pos..].starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn try_consume_keyword(&mut self, keyword: &str) -> bool {
        if self.peek_keyword(keyword) {
            self.pos += keyword.len();
            true
        } else {
            false
        }
    }

    fn consume_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        if self.try_consume_keyword(keyword) {
            Ok(())
        } else {
            Err(ParseError::Expected {
                expected: format!("'{}'", keyword),
                found: self.peek_word().unwrap_or("EOF".to_string()),
            })
        }
    }

    /// Read a name a DECLARATION will be KEYED AT.
    ///
    /// [`Self::parse_ident`] reads a MENTION: one component, whatever `scan_name` accepts. A
    /// declaration is a different obligation — the kernel context is a map of flat keys, so the
    /// name has to be one a term can spell back, and its key has to read back as itself. Both
    /// are [`super::declaration_name`], the one function every declaration door reads.
    ///
    /// Without this the literate surface could mint a WRITE-ONLY global by two routes the
    /// vernacular door already closes: a word the term door reads as SYNTAX
    /// (`## To match (n: Nat) -> Nat:` keys the context at `match`, which no `Check` can name)
    /// and a quoted component holding a `.` (`## Theorem: «a.b»` keys at `a.b`, which reads back
    /// as the two-component name `a.b`, so the two would share one key).
    fn parse_declaration_name(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        let start = self.pos;
        match super::surface::Name::scan(&self.input[start..]) {
            Some((_, len)) => {
                let name = super::declaration_name(&self.input[start..start + len])?;
                self.pos = start + len;
                Ok(name.key())
            }
            None => match self.peek() {
                Some(c) => Err(ParseError::Expected {
                    expected: "identifier".to_string(),
                    found: c.to_string(),
                }),
                None => Err(ParseError::UnexpectedEof),
            },
        }
    }

    /// One name, read through the SHARED grammar (`term_parser::scan_name`) so a theorem, a
    /// binder and a `Check` all spell a name the same way.
    fn parse_ident(&mut self) -> Result<String, ParseError> {
        self.skip_whitespace();
        match super::term_parser::scan_name(&self.input[self.pos..]) {
            Some((name, len)) => {
                self.pos += len;
                Ok(name)
            }
            None => match self.peek() {
                Some(c) => Err(ParseError::Expected {
                    expected: "identifier".to_string(),
                    found: c.to_string(),
                }),
                None => Err(ParseError::UnexpectedEof),
            },
        }
    }

    // ============================================================
    // INFIX OPERATOR HELPERS
    // ============================================================

    /// Check if current position starts a negative number (- followed by digit)
    fn peek_negative_number(&self) -> bool {
        if !self.peek_char('-') {
            return false;
        }
        self.input
            .get(self.pos + 1..)
            .and_then(|s| s.chars().next())
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    }

    /// Check if current position starts an arrow (->)
    fn peek_arrow(&self) -> bool {
        self.input[self.pos..].starts_with("->")
    }

    /// Check if current position has a comparison operator
    fn peek_comparison_operator(&self) -> bool {
        let rest = &self.input[self.pos..];
        rest.starts_with("<=")
            || rest.starts_with(">=")
            || rest.starts_with("≤")
            || rest.starts_with("≥")
            || rest.starts_with('<')
            || rest.starts_with('>')
    }
}

// ============================================================
// UNIT TESTS
// ============================================================

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/literate_parser/tests.rs"]
mod tests;
