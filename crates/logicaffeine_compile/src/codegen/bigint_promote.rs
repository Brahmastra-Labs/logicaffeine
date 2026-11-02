//! Which integer variables must be stored as the overflow-promoting `LogosInt`
//! (rather than a bare `i64`) so the compiled AOT matches the tree-walker's
//! unbounded-integer semantics.
//!
//! A variable is *promotable* when some assignment to it can produce a value
//! beyond `i64`:
//!   - a compile-time bignum constant (`2 ** 100`),
//!   - an expression referencing an already-promotable variable (`big * big`),
//!   - a multiplicative / exponentiating self-accumulator (`p = p * i`), or
//!   - a doubling additive accumulator (`x = x + x`).
//!
//! Linear counters (`i = i + 1`) are deliberately NOT promoted — they stay on
//! the `i64` fast path. Promotion propagates to a fixpoint, so a chain of
//! promotable derivations all become `LogosInt`.

use std::collections::{HashMap, HashSet};

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

/// ★ EVERY PROMOTION FACT, COMPUTED ONCE, TO A FIXPOINT.
///
/// LOGOS has ONE integer type and it is arbitrary-precision; this tier has TWO representations of
/// it (`i64` and `LogosInt`), and which one a SLOT gets is decided here. The decision is mutually
/// recursive — a promoted FIELD promotes the functions that read it, which promotes the fields
/// THEY build, which promotes the arguments handed to them — and it used to be run **exactly
/// twice** and stopped, in-line in `codegen_program_inner`.
///
/// Two iterations is enough for the shapes anybody had tried and not for the next one, so the same
/// defect kept coming back wearing a different program: `expected i64, found LogosInt` in generated
/// code that rustc rejects on a type, with no `compile_error!` anywhere and every other tier
/// perfectly happy, because the tree-walker and the VM have one integer type and never notice.
///
/// Worse, the facts were installed for STRUCT DEFINITION from a narrow seed and recomputed WIDER
/// afterwards for everything else, under a comment asserting the two were identical. They were
/// not: a field could be typed `i64` while the value filling it was typed `LogosInt`, by
/// construction rather than by accident.
///
/// So: one function, one fixpoint, installed once, before anything is emitted. Every set here only
/// GROWS across iterations (each helper is monotone in its input), so the loop terminates on a
/// finite program — and the answer no longer depends on how many times someone remembered to run it.
///
/// Promotion stays DEMAND-DRIVEN: a field is promoted because some construction feeds it a value
/// that can exceed `i64`, never because it is an `Int`. A linear counter and an ordinary record
/// field keep the machine word.
pub(super) struct PromotionFacts {
    /// Functions whose RETURN is a `LogosInt` — every call site treats the result as one.
    pub bigint_fns: HashSet<Symbol>,
    /// `(struct, field)` pairs stored as `LogosInt`.
    pub bigint_fields: HashSet<(Symbol, Symbol)>,
    /// Functions whose integer parameters (and closure params) are exactified.
    pub exact_hofs: HashSet<Symbol>,
    /// Functions returning `Result of Int and E` whose PAYLOAD is exact. Distinct from
    /// `bigint_fns`: the result is a `Result`, not a number, and telling call sites otherwise
    /// emitted `LogosInt::from(<a Result>)`.
    pub exact_result_fns: HashSet<Symbol>,
    /// ★ THE ONE SET EVERY "can this produce a value beyond i64" QUESTION IS ASKED AGAINST — the
    /// fixpoint's own seed. A LOCAL's representation and a FIELD's must be decided from the same
    /// answer, or a decoder's local is a `LogosInt` while the record it fills holds an `i64` and
    /// the generated program does not compile. That is the disagreement that kept coming back.
    pub promo_fns: HashSet<Symbol>,
}

pub(super) fn promotion_facts(stmts: &[Stmt], interner: &Interner) -> PromotionFacts {
    // A function only RETURNS a bignum if it says it returns a number. `jsonSerialize` mentions
    // the very same binding on its way to building a STRING, and calling that a bignum return
    // emitted `LogosInt::from(<a String>)` — the analysis has to read the declaration, not just
    // follow the name.
    let declared_return = |name: &Symbol| {
        stmts.iter().find_map(|s| {
            let Stmt::FunctionDef {
                name: n,
                return_type: Some(rt),
                ..
            } = s
            else {
                return None;
            };
            (n == name).then_some(*rt)
        })
    };
    let returns_int_ty = |name: &Symbol| {
        declared_return(name).is_some_and(|rt| {
            let is_int = |t: &TypeExpr| {
                matches!(t, TypeExpr::Primitive(x) | TypeExpr::Named(x)
                    if interner.resolve(*x).eq_ignore_ascii_case("Int"))
            };
            is_int(rt)
                || matches!(rt, TypeExpr::Generic { base, params }
                    if interner.resolve(*base).eq_ignore_ascii_case("Result")
                        && params.first().is_some_and(is_int))
        })
    };
    let is_result_ty = |name: &Symbol| {
        declared_return(name).is_some_and(|rt| {
            matches!(rt, TypeExpr::Generic { base, .. }
                if interner.resolve(*base).eq_ignore_ascii_case("Result"))
        })
    };

    let base_hofs = exact_hof_fns(stmts, interner);

    // The seed every field decision is taken against. It is the UNION of what the two former
    // call sites each used one half of — a field promoted for one consumer and not the other is
    // the disagreement this whole function exists to remove.
    let mut bigint_fields: HashSet<(Symbol, Symbol)> = HashSet::new();
    let mut exact_params = exactified_params(stmts, &base_hofs, interner);
    let mut returning: HashSet<Symbol> = HashSet::new();
    let mut base_returns =
        bigint_returning_fns(stmts, interner, &bigint_fields, &returning, &exact_params);
    let mut promo_fns: HashSet<Symbol> = base_returns.clone();
    promo_fns.extend(base_hofs.iter().copied());

    let mut arg_promoted: HashSet<Symbol> = HashSet::new();
    let mut exact_result_fns;
    let mut field_returners;
    loop {
        bigint_fields = bigint_struct_fields(stmts, &promo_fns, &bigint_fields, &exact_params);
        // A function whose RETURN reads a promoted field is bignum-returning, and which fields are
        // promoted is only known once the round above has run — so the base set is recomputed
        // here rather than once, before anything was known. What the last round already knows
        // returns a bignum seeds it: a local bound from such a call holds one.
        base_returns =
            bigint_returning_fns(stmts, interner, &bigint_fields, &returning, &exact_params);
        // A function HANDED a bignum must take one: a `LogosInt` value reaching an `i64`
        // parameter is a generated program that will not compile.
        let handed =
            bigint_arg_fns_with_facts(stmts, &bigint_fields, interner, &promo_fns, &exact_params);
        // …and the other direction: a function that RETURNS a value read from a bignum field.
        let all_field_returners = bigint_field_returning_fns(stmts, &bigint_fields);
        exact_result_fns = all_field_returners
            .iter()
            .copied()
            .filter(|f| returns_int_ty(f) && is_result_ty(f))
            .collect::<HashSet<Symbol>>();
        field_returners = all_field_returners
            .into_iter()
            .filter(|f| returns_int_ty(f) && !is_result_ty(f))
            .collect::<HashSet<Symbol>>();

        // ★ BOTH SETS DECIDE WHEN THIS IS DONE. `bigint_fields` feeds ITSELF — a field filled
        // from a read of another promoted field — so a loop that watched only the function set
        // could stop one round early and leave a field un-promoted, which is the very shape of
        // "ran a fixed number of times" this function exists to replace.
        //
        // An exactified function's `i64` parameters hold bignums, so a local set from one holds
        // one too, and what it returns from one is one: they seed the next round's scans.
        let before = (
            promo_fns.len(),
            bigint_fields.len(),
            arg_promoted.len(),
            returning.len(),
        );
        arg_promoted.extend(handed);
        promo_fns.extend(base_returns.iter().copied());
        promo_fns.extend(exact_result_fns.iter().copied());
        promo_fns.extend(field_returners.iter().copied());
        promo_fns.extend(arg_promoted.iter().copied().filter(|f| returns_int_ty(f)));
        let exactified: HashSet<Symbol> = base_hofs.union(&arg_promoted).copied().collect();
        exact_params = exactified_params(stmts, &exactified, interner);
        returning.extend(base_returns.iter().copied());
        returning.extend(field_returners.iter().copied());
        returning.extend(
            exactified
                .iter()
                .copied()
                .filter(|f| (!arg_promoted.contains(f) || returns_int_ty(f)) && !is_result_ty(f)),
        );
        if (
            promo_fns.len(),
            bigint_fields.len(),
            arg_promoted.len(),
            returning.len(),
        ) == before
        {
            break;
        }
    }

    // ★ PARAMETERS AND RETURNS ARE SEPARATE DECISIONS. A function handed a bignum takes one; that
    // says NOTHING about what it gives back. `jsonRenderDec(mant, scale) -> Text` takes two
    // bignums and returns a STRING, and marking it bignum-RETURNING emitted
    // `LogosInt::from(<a String>)` — one type error traded for another. So a return exactifies
    // only when the function actually declares it gives back a number.
    let mut exact_hofs = base_hofs;
    exact_hofs.extend(arg_promoted.iter().copied());
    let hof_only: Vec<Symbol> = exact_hofs
        .iter()
        .copied()
        .filter(|f| !arg_promoted.contains(f) || returns_int_ty(f))
        .collect();

    let mut bigint_fns = base_returns;
    bigint_fns.extend(field_returners);
    bigint_fns.extend(hof_only);

    PromotionFacts {
        bigint_fns,
        bigint_fields,
        exact_hofs,
        exact_result_fns,
        promo_fns,
    }
}

/// The whole-program set of `Int`-returning functions whose return value can
/// exceed i64 (so their signature becomes `-> LogosInt`). Fixpoint over the call
/// graph: a function returning the result of another bignum function is itself
/// bignum-returning. `known` are functions already known to return a bignum, and
/// `exact_params` each exactified function's bignum parameters.
pub(super) fn bigint_returning_fns(
    stmts: &[Stmt],
    interner: &Interner,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    known: &HashSet<Symbol>,
    exact_params: &HashMap<Symbol, HashSet<Symbol>>,
) -> HashSet<Symbol> {
    let is_int_ret = |rt: Option<&TypeExpr>| {
        matches!(rt, Some(TypeExpr::Primitive(s)) if {
            let n = interner.resolve(*s);
            n == "Int" || n == "Nat"
        })
    };
    let fns: Vec<(Symbol, &[Stmt])> = stmts
        .iter()
        .filter_map(|s| {
            let Stmt::FunctionDef {
                name,
                body,
                return_type,
                ..
            } = s
            else {
                return None;
            };
            is_int_ret(*return_type).then_some((*name, &body[..]))
        })
        .collect();

    let none = HashSet::new();
    let mut bigint: HashSet<Symbol> = known.clone();
    let mut found: HashSet<Symbol> = HashSet::new();
    loop {
        let mut changed = false;
        for &(name, body) in &fns {
            let seeds = exact_params.get(&name).unwrap_or(&none);
            if !found.contains(&name) && returns_bigint(body, &bigint, bigint_fields, seeds) {
                found.insert(name);
                bigint.insert(name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    found
}

/// Each function of `fns` mapped to the parameters codegen exactifies when it is exactified: its
/// machine-word (`i64`) ones.
fn exactified_params(
    stmts: &[Stmt],
    fns: &HashSet<Symbol>,
    interner: &Interner,
) -> HashMap<Symbol, HashSet<Symbol>> {
    stmts
        .iter()
        .filter_map(|s| {
            let Stmt::FunctionDef { name, params, .. } = s else {
                return None;
            };
            fns.contains(name)
                .then(|| (*name, exact_param_seeds(params, interner)))
        })
        .collect()
}

/// The parameters an exactified function takes as bignums: every one codegen would otherwise emit
/// as a machine word (`i64`). Codegen registers exactly these as `LogosInt` (`codegen_function_def`)
/// and asks the analysis with exactly these as seeds, so the two cannot disagree on a parameter.
pub(super) fn exact_param_seeds(
    params: &[(Symbol, &TypeExpr)],
    interner: &Interner,
) -> HashSet<Symbol> {
    params
        .iter()
        .filter(|(_, ty)| exactifies(ty, interner))
        .map(|(p, _)| *p)
        .collect()
}

/// Whether a parameter of this type is one an exactified function takes as a `LogosInt`.
fn exactifies(ty: &TypeExpr, interner: &Interner) -> bool {
    super::types::codegen_type_expr(ty, interner) == "i64"
}

/// A SCALAR higher-order function whose result can overflow through its closure — an `Int`/`Nat`
/// return, a `fn`-typed parameter, and NO sequence parameter (`apply(f, x)`). Its `fn`-param, its
/// integer params, and its return type are exactified to the overflow-promoting `LogosInt`, and a
/// closure passed to it has its integer params exactified too — so an overflowing closure body
/// (`(n) -> n * 2` on `i64::MAX`) promotes to a bignum instead of wrapping/panicking, matching the
/// VM. A SEQ-HOF (`map`/`fold`/`reduceSeq` — a `Seq of T` param) is EXCLUDED: exactifying it would
/// force passing an `i64` sequence element into a `LogosInt` closure param (a type error). This
/// mirrors the AST-C `is_exact_hof` selective rule (`codegen_c/mod.rs`).
///
/// The signature rule is then refined per CALL SITE, because exactification is only correct and
/// only worth its cost where the sites demand it:
///
///   - Exactification requires an INLINE closure literal in every `fn`-typed position of every
///     site: the call-site half (`Expr::Closure` in `codegen/expr.rs`) types an inline closure's
///     integer params `LogosInt` to match, but a closure VARIABLE was already emitted `i64` at its
///     `Let`, and a name used as a VALUE anywhere may flow to sites this analysis cannot see. Such
///     a HOF keeps plain `i64` (the checked runtime still traps loudly on overflow; promoting
///     through a named closure needs the definition-site half before it can be exact).
///   - A HOF whose every site is PROVABLY within `i64` keeps plain `i64` too — exact already,
///     with no bignum tax: every integer argument a literal, the body a pure pass-through (no
///     arithmetic of its own, closures applied only to params/literals), and every closure body
///     evaluated over the actual arguments staying in-range at every step.
///   - Everything else — an inline-closure site whose arithmetic is (or may be) out of range —
///     is exactified, so the overflow promotes == the VM.
pub(crate) fn exact_hof_fns(stmts: &[Stmt], interner: &Interner) -> HashSet<Symbol> {
    let is_int = |ty: &TypeExpr| {
        matches!(unwrap_ty(ty), TypeExpr::Primitive(s) if {
            let n = interner.resolve(*s);
            n == "Int" || n == "Nat"
        })
    };
    let is_int_ret = |rt: Option<&TypeExpr>| rt.map_or(false, |t| is_int(t));
    let is_fn_param = |ty: &TypeExpr| matches!(unwrap_ty(ty), TypeExpr::Function { .. });
    let is_seq_param = |ty: &TypeExpr| {
        matches!(unwrap_ty(ty), TypeExpr::Generic { base, .. } if {
            matches!(interner.resolve(*base), "Seq" | "List" | "Vec")
        })
    };

    let candidates: Vec<(Symbol, &[(Symbol, &TypeExpr)], &[Stmt])> = stmts
        .iter()
        .filter_map(|s| {
            let Stmt::FunctionDef {
                name,
                params,
                body,
                return_type,
                is_native: false,
                ..
            } = s
            else {
                return None;
            };
            (is_int_ret(*return_type)
                && params.iter().any(|(_, ty)| is_fn_param(ty))
                && !params.iter().any(|(_, ty)| is_seq_param(ty)))
            .then_some((*name, params.as_slice(), *body))
        })
        .collect();
    if candidates.is_empty() {
        return HashSet::new();
    }

    let harvest = crate::analysis::known_calls::harvest_calls(stmts);
    let mut sites: HashMap<Symbol, Vec<&[&Expr]>> = HashMap::new();
    for site in &harvest.calls {
        if candidates.iter().any(|(name, ..)| *name == site.function) {
            sites.entry(site.function).or_default().push(site.args);
        }
    }

    candidates
        .into_iter()
        .filter_map(|(name, params, body)| {
            if harvest.value_refs.contains(&name) {
                return None;
            }
            let fn_positions: Vec<usize> = params
                .iter()
                .enumerate()
                .filter(|(_, (_, ty))| is_fn_param(ty))
                .map(|(i, _)| i)
                .collect();
            let name_sites = sites.get(&name).map(Vec::as_slice).unwrap_or(&[]);
            let all_inline = !name_sites.is_empty()
                && name_sites.iter().all(|args| {
                    args.len() == params.len()
                        && fn_positions
                            .iter()
                            .all(|&i| matches!(args[i], Expr::Closure { .. }))
                });
            if !all_inline {
                return None;
            }
            match pass_through_fn_calls(body, params) {
                None => Some(name),
                Some(body_calls) => {
                    let all_bounded = name_sites
                        .iter()
                        .all(|args| site_provably_bounded(args, params, &body_calls));
                    (!all_bounded).then_some(name)
                }
            }
        })
        .collect()
}

/// If the HOF body is a pure PASS-THROUGH — no arithmetic of its own, and its only calls apply
/// `fn`-typed params to the HOF's own params or integer literals — return those applications
/// (`(fn-param, argument exprs)`). `None` means the body is opaque to the boundedness proof.
///
/// The proof reads every node of the body: each statement must be one that computes nothing of its
/// own (`Return`, `Let`, `Show`, `If`, `Break`) and each expression one [`pass_through_node`]
/// accepts, so a body written with any other statement or expression is opaque by construction.
fn pass_through_fn_calls<'a>(
    body: &'a [Stmt<'a>],
    params: &[(Symbol, &TypeExpr)],
) -> Option<Vec<(Symbol, &'a [&'a Expr<'a>])>> {
    let fn_params: HashSet<Symbol> = params
        .iter()
        .filter(|(_, ty)| matches!(unwrap_ty(ty), TypeExpr::Function { .. }))
        .map(|(p, _)| *p)
        .collect();
    let mut calls = Vec::new();
    let mut pass_through = true;
    for_each_node_in_scope(body, |node| {
        pass_through &= match node {
            Visited::Stmt(stmt) => matches!(
                stmt,
                Stmt::Return { .. }
                    | Stmt::Let { .. }
                    | Stmt::Show { .. }
                    | Stmt::If { .. }
                    | Stmt::Break
            ),
            Visited::Expr(expr) => pass_through_node(expr, &fn_params, &mut calls),
        };
    });
    pass_through.then_some(calls)
}

/// One expression node of a pass-through body: a leaf, a negation, a comparison or a logical
/// AND/OR (none can grow past its operands, which are nodes of their own), or an application of a
/// `fn`-typed parameter to parameters and integer literals, recorded in `calls`.
fn pass_through_node<'a>(
    expr: &'a Expr<'a>,
    fn_params: &HashSet<Symbol>,
    calls: &mut Vec<(Symbol, &'a [&'a Expr<'a>])>,
) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::Not { .. } => true,
        Expr::BinaryOp { op, .. } => matches!(
            op,
            BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::Lt
                | BinaryOpKind::Gt
                | BinaryOpKind::LtEq
                | BinaryOpKind::GtEq
                | BinaryOpKind::And
                | BinaryOpKind::Or
        ),
        Expr::Call { function, args } if fn_params.contains(function) => {
            let simple = args
                .iter()
                .all(|a| matches!(a, Expr::Identifier(_) | Expr::Literal(Literal::Number(_))));
            if simple {
                calls.push((*function, args));
            }
            simple
        }
        _ => false,
    }
}

/// Prove one call site of a pass-through HOF stays inside `i64`: bind the site's integer
/// literal arguments, then evaluate each closure application over its actual arguments with
/// every intermediate in range. Anything non-literal, captured, or outside the evaluable
/// expression grammar fails the proof (the caller then keeps the HOF exact).
fn site_provably_bounded(
    site_args: &[&Expr],
    params: &[(Symbol, &TypeExpr)],
    body_calls: &[(Symbol, &[&Expr])],
) -> bool {
    let env: HashMap<Symbol, i128> = params
        .iter()
        .zip(site_args)
        .filter_map(|((p, _), a)| match a {
            Expr::Literal(Literal::Number(n)) => Some((*p, *n as i128)),
            _ => None,
        })
        .collect();
    body_calls.iter().all(|(fn_param, call_args)| {
        let Some(pos) = params.iter().position(|(p, _)| p == fn_param) else {
            return false;
        };
        let Expr::Closure {
            params: closure_params,
            body,
            ..
        } = site_args[pos]
        else {
            return false;
        };
        let crate::ast::stmt::ClosureBody::Expression(closure_body) = body else {
            return false;
        };
        if closure_params.len() != call_args.len() {
            return false;
        }
        let mut closure_env: HashMap<Symbol, i128> = HashMap::new();
        for ((cp, _), arg) in closure_params.iter().zip(call_args.iter()) {
            let value = match arg {
                Expr::Identifier(s) => match env.get(s) {
                    Some(v) => *v,
                    None => return false,
                },
                Expr::Literal(Literal::Number(n)) => *n as i128,
                _ => return false,
            };
            closure_env.insert(*cp, value);
        }
        eval_in_i64_range(closure_body, &closure_env).is_some()
    })
}

/// Evaluate a pure integer expression over literal bindings, requiring every node's value to
/// fit `i64`. Only `+ - *` are evaluated (one such op over in-range operands cannot overflow
/// `i128`); the division family is excluded so no backend-semantics question can creep into
/// the proof. `None` = not provable.
fn eval_in_i64_range(expr: &Expr, env: &HashMap<Symbol, i128>) -> Option<i128> {
    let value = match expr {
        Expr::Literal(Literal::Number(n)) => *n as i128,
        Expr::Identifier(s) => *env.get(s)?,
        Expr::BinaryOp { op, left, right } => {
            let l = eval_in_i64_range(left, env)?;
            let r = eval_in_i64_range(right, env)?;
            match op {
                BinaryOpKind::Add => l + r,
                BinaryOpKind::Subtract => l - r,
                BinaryOpKind::Multiply => l * r,
                _ => return None,
            }
        }
        _ => return None,
    };
    ((i64::MIN as i128)..=(i64::MAX as i128))
        .contains(&value)
        .then_some(value)
}

/// Peel `Mutable`/`Refinement` wrappers to reach the underlying type for classification.
fn unwrap_ty<'a>(ty: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
    match ty {
        TypeExpr::Mutable { inner } => unwrap_ty(inner),
        TypeExpr::Refinement { base, .. } => unwrap_ty(base),
        other => other,
    }
}

/// The set of integer variables in `stmts` (including nested blocks) that must
/// be stored as `LogosInt`. `bigint_fns` is the set of functions that RETURN a
/// `LogosInt` — a call to one is itself a bignum-producing expression — and `seeds`
/// the names that already hold one (an exactified function's parameters).
pub(super) fn promotable_int_vars(
    stmts: &[Stmt],
    bigint_fns: &HashSet<Symbol>,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    seeds: &HashSet<Symbol>,
) -> HashSet<Symbol> {
    let mut assigns: Vec<(Symbol, &Expr)> = Vec::new();
    collect_assigns(stmts, &mut assigns);

    let swaps = crate::optimize::swapped_pairs(stmts);
    let mut promotable: HashSet<Symbol> = seeds.clone();
    loop {
        let mut changed = false;
        for &(target, value) in &assigns {
            if !promotable.contains(&target)
                && rhs_can_grow(value, target, &promotable, bigint_fns, bigint_fields)
            {
                promotable.insert(target);
                changed = true;
            }
        }
        // A swap hands each name the other's value: one that may hold a big integer makes both `LogosInt`.
        for &(a, b) in &swaps {
            if promotable.contains(&a) != promotable.contains(&b) {
                promotable.insert(a);
                promotable.insert(b);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    promotable
}

/// Does this function body RETURN a value that can exceed i64 — i.e. some
/// `Return <expr>` whose expression reads a promotable local or calls a
/// `bigint_fn`? Used to compute the fixpoint of bignum-returning functions.
pub(super) fn returns_bigint(
    body: &[Stmt],
    bigint_fns: &HashSet<Symbol>,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    seeds: &HashSet<Symbol>,
) -> bool {
    let promotable = promotable_int_vars(body, bigint_fns, bigint_fields, seeds);
    let mut found = false;
    for_each_return(body, |e| {
        if mentions_promotable(e, &promotable)
            || mentions_bigint_call(e, bigint_fns)
            || reads_bigint_field(e, bigint_fields)
        {
            found = true;
        }
    });
    found
}

/// The `(struct_type, field)` pairs that receive a bignum-promoted value at some construction, so the
/// field must be stored as the overflow-promoting `LogosInt` (not `i64`) to match the interpreter's
/// arbitrary-precision `Int` — e.g. json's `JNum { n: m }` where the digit accumulator `m` promotes.
/// A whole-program scan: each `## To` body (and the top-level main) computes its OWN promotable set,
/// then every construction in that scope whose field value is a bignum var/call marks `(type, field)`.
pub(super) fn bigint_struct_fields(
    stmts: &[Stmt],
    bigint_fns: &HashSet<Symbol>,
    known_fields: &HashSet<(Symbol, Symbol)>,
    exact_params: &HashMap<Symbol, HashSet<Symbol>>,
) -> HashSet<(Symbol, Symbol)> {
    let mut out = HashSet::new();
    let mut mark = |promotable: &HashSet<Symbol>, body: &[Stmt]| {
        for_each_new(body, |ty, fields| {
            for (fname, val) in fields {
                // A COMPILE-TIME bignum constant is the plainest source there is, and it was the
                // one case `rhs_can_grow` handled for locals and this did not: `a new Cell with v
                // (2 ** 70)` typed the field a machine word and trapped at run time on a value the
                // reference tier holds exactly.
                if mentions_promotable(val, promotable)
                    || mentions_bigint_call(val, bigint_fns)
                    || reads_bigint_field(val, known_fields)
                    || const_overflows_i64(val)
                {
                    out.insert((ty, *fname));
                }
            }
        });
    };
    let none = HashSet::new();
    for s in stmts {
        if let Stmt::FunctionDef { name, body, .. } = s {
            let seeds = exact_params.get(name).unwrap_or(&none);
            let promotable = promotable_int_vars(body, bigint_fns, known_fields, seeds);
            mark(&promotable, body);
        }
    }
    let promotable = promotable_int_vars(stmts, bigint_fns, known_fields, &none);
    mark(&promotable, stmts);
    out
}

/// Every statement and expression of ONE SCOPE — `## Main`'s statements or a function's body —
/// each once, through `ast_depth`'s exhaustive walk: every block nested in it and every closure
/// written in it (a closure reads and writes the names it captures, so its body is this scope's),
/// and never the body of a function defined in it, which is a scope of its own. Every scan below
/// walks through here, so no statement a program can write is one the analysis cannot see.
fn for_each_node_in_scope<'w, 'a>(stmts: &'w [Stmt<'a>], mut visit: impl FnMut(Visited<'w, 'a>)) {
    walk_nodes(stmts, |node| {
        if let Visited::Stmt(Stmt::FunctionDef { .. }) = node {
            return Walk::Past;
        }
        visit(node);
        Walk::Into
    });
}

/// Every construction in one scope — a record (`Expr::New`, keyed by its type) or an enum variant
/// (`Expr::NewVariant`, keyed by the VARIANT name, which is what `codegen_enum_def` and the
/// construction both look up: json's `Json` is an enum, so `JNum.n` and `JDec.mant` are reached
/// here) — wherever it is written: a binding, a `Show`, a `Push`, an argument, an interpolation.
fn for_each_new<'w, 'a>(
    stmts: &'w [Stmt<'a>],
    mut f: impl FnMut(Symbol, &'w [(Symbol, &'a Expr<'a>)]),
) {
    for_each_node_in_scope(stmts, |node| {
        if let Visited::Expr(Expr::New {
            type_name,
            init_fields,
            ..
        }) = node
        {
            f(*type_name, init_fields);
        } else if let Visited::Expr(Expr::NewVariant {
            variant, fields, ..
        }) = node
        {
            f(*variant, fields);
        }
    });
}

/// Every value one scope's own body returns. A `Return` inside a closure answers the closure, not
/// the function it is written in, so the walk does not go into a closure here.
///
/// ★ AN `Inspect` ARM IS A RETURN SITE, like every other block: most of json.lg, and every derived
/// decoder, answers out of a match.
fn for_each_return<'w, 'a>(stmts: &'w [Stmt<'a>], mut f: impl FnMut(&'w Expr<'a>)) {
    walk_nodes(stmts, |node| {
        if let Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) = node
        {
            return Walk::Past;
        }
        if let Visited::Stmt(Stmt::Return { value: Some(value) }) = node {
            f(value);
        }
        Walk::Into
    });
}

fn mentions_bigint_call(e: &Expr, bigint_fns: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Call { function, .. } => bigint_fns.contains(function),
        Expr::BinaryOp { left, right, .. } => {
            mentions_bigint_call(left, bigint_fns) || mentions_bigint_call(right, bigint_fns)
        }
        Expr::Not { operand } => mentions_bigint_call(operand, bigint_fns),
        _ => false,
    }
}

/// Every assignment in one scope, as `(name, value)`: a `Let`, a `Set`, and an `Inspect` arm's
/// BINDING, which is an assignment from the scrutinee — how a value decoded out of a parsed document
/// reaches a local at all.
fn collect_assigns<'a>(stmts: &'a [Stmt<'a>], out: &mut Vec<(Symbol, &'a Expr<'a>)>) {
    for_each_node_in_scope(stmts, |node| {
        let Visited::Stmt(stmt) = node else { return };
        if let Stmt::Let { var, value, .. } = stmt {
            out.push((*var, value));
        } else if let Stmt::Set { target, value } = stmt {
            out.push((*target, value));
        } else if let Stmt::Inspect { target, arms, .. } = stmt {
            for arm in arms {
                for (_, binding) in &arm.bindings {
                    out.push((*binding, target));
                }
            }
        }
    });
}

/// Can this assignment RHS produce a value beyond `i64`?
fn rhs_can_grow(
    value: &Expr,
    target: Symbol,
    promotable: &HashSet<Symbol>,
    bigint_fns: &HashSet<Symbol>,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
) -> bool {
    // A read of a field stored as `LogosInt` grows whatever it is assigned to, wherever it sits in
    // the expression — the flow edge out of a record.
    if reads_bigint_field(value, bigint_fields) {
        return true;
    }
    match value {
        // A direct copy of a promotable var (`term = scale`) carries its bignum-ness.
        Expr::Identifier(s) => promotable.contains(s),
        // A call to a bignum-returning function (directly, or inside arithmetic).
        Expr::Call { function, .. } if bigint_fns.contains(function) => true,
        Expr::BinaryOp { op, left, right } => match op {
            // Multiplicative growth / exponentiation: a bignum constant, a
            // promotable operand, a bignum call, or the target itself.
            BinaryOpKind::Multiply | BinaryOpKind::Pow => {
                const_overflows_i64(value)
                    || mentions(left, target)
                    || mentions(right, target)
                    || mentions_promotable(left, promotable)
                    || mentions_promotable(right, promotable)
                    || mentions_bigint_call(left, bigint_fns)
                    || mentions_bigint_call(right, bigint_fns)
            }
            // Additive: a bignum constant, a promotable operand, a bignum call,
            // or a DOUBLING accumulator (`x + x`) — never the linear `i + 1`.
            BinaryOpKind::Add | BinaryOpKind::Subtract => {
                const_overflows_i64(value)
                    || mentions_promotable(left, promotable)
                    || mentions_promotable(right, promotable)
                    || mentions_bigint_call(left, bigint_fns)
                    || mentions_bigint_call(right, bigint_fns)
                    || (mentions(left, target) && mentions(right, target))
            }
            // Division of an ALREADY-bignum DIVIDEND stays bignum: `10^100 / 7` is still ~`10^99`.
            // Only a promotable / bignum-call LEFT operand keeps the magnitude — dividing SHRINKS,
            // never grows, so `x = x / 2` (a binary-decomposition loop, `x & 1`) must NOT self-promote
            // (that would make `x` a `LogosInt` and break its bitwise ops). The fixpoint still keeps a
            // truly-bignum dividend promoted through repeated `term = term / c` via `mentions_promotable`.
            BinaryOpKind::Divide => {
                mentions_promotable(left, promotable) || mentions_bigint_call(left, bigint_fns)
            }
            // Modulo `a % m` is bounded by `m`, so the RESULT is bignum only when the DIVISOR is
            // bignum (`piScaled mod 10^digits` keeps up to `digits` digits). A small modulus yields a
            // small result, so the left operand is irrelevant, and a self `x = x % k` never grows.
            BinaryOpKind::Modulo => {
                mentions_promotable(right, promotable) || mentions_bigint_call(right, bigint_fns)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Is a field of this NAME stored as `LogosInt` in any record?
///
/// ★ ONE RULE, ASKED BY BOTH SIDES. The analysis uses it to decide that a local assigned from a
/// field read must hold a `LogosInt`; the EMITTER uses it to decide that `c's v * 2` is bignum
/// arithmetic rather than an `i64` multiply. Two spellings of "is this field exact" is how the
/// analysis and the emitter came to disagree in the first place, so there is only one.
pub(crate) fn field_name_is_exact(fields: &HashSet<(Symbol, Symbol)>, field: Symbol) -> bool {
    fields.iter().any(|(_, f)| *f == field)
}

/// Does this expression READ a field that is stored as `LogosInt`?
///
/// ★ THE EDGE THAT WAS MISSING. `Set top to m's id` where `Memo.id` is promoted assigned a
/// `LogosInt` into an `i64` local, and the generated Rust did not compile — the last shape of the
/// same defect: a value flowing out of one representation into a slot that was decided separately.
/// A field read is a flow edge like any other, and promotion has to travel along it.
///
/// ★ IT MATCHES BY FIELD NAME, WHICH OVER-APPROXIMATES, AND THAT IS THE SAFE DIRECTION. Resolving
/// `m's id` to the record `m` holds needs the type AT THE SITE, which this analysis does not have.
/// Matching the name promotes a local that reads a same-named field of a DIFFERENT record — which
/// costs that local the `i64` fast path and is never wrong, because `LogosInt` is what a LOGOS
/// `Int` actually is. The opposite error — missing a promotion — is a program that will not build,
/// or worse, one that traps on an overflow the reference tier handles.
fn reads_bigint_field(e: &Expr, fields: &HashSet<(Symbol, Symbol)>) -> bool {
    match e {
        Expr::FieldAccess { object, field } => {
            field_name_is_exact(fields, *field) || reads_bigint_field(object, fields)
        }
        Expr::BinaryOp { left, right, .. } => {
            reads_bigint_field(left, fields) || reads_bigint_field(right, fields)
        }
        Expr::Not { operand } => reads_bigint_field(operand, fields),
        Expr::Call { args, .. } => args.iter().any(|a| reads_bigint_field(a, fields)),
        Expr::Copy { expr } => reads_bigint_field(expr, fields),
        _ => false,
    }
}

fn mentions(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => mentions(left, sym) || mentions(right, sym),
        Expr::Not { operand } => mentions(operand, sym),
        _ => false,
    }
}

fn mentions_promotable(e: &Expr, promotable: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Identifier(s) => promotable.contains(s),
        Expr::BinaryOp { left, right, .. } => {
            mentions_promotable(left, promotable) || mentions_promotable(right, promotable)
        }
        Expr::Not { operand } => mentions_promotable(operand, promotable),
        _ => false,
    }
}

/// True iff `e` is a constant integer expression whose exact value exceeds `i64`.
fn const_overflows_i64(e: &Expr) -> bool {
    const_eval_bigint(e).map_or(false, |v| v.to_i64().is_none())
}

fn const_eval_bigint(e: &Expr) -> Option<logicaffeine_base::BigInt> {
    use logicaffeine_base::BigInt;
    match e {
        Expr::Literal(Literal::Number(n)) => Some(BigInt::from_i64(*n)),
        Expr::BinaryOp { op, left, right } => {
            let l = const_eval_bigint(left)?;
            let r = const_eval_bigint(right)?;
            match op {
                BinaryOpKind::Add => Some(l.add(&r)),
                BinaryOpKind::Subtract => Some(l.sub(&r)),
                BinaryOpKind::Multiply => Some(l.mul(&r)),
                BinaryOpKind::Pow => {
                    let exp = u32::try_from(r.to_i64()?).ok()?;
                    Some(l.pow(exp))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Functions that must take (and return) `LogosInt` because a bignum VALUE flows into them.
///
/// ★ THE GAP THIS CLOSES. A variant field the program feeds a bignum is stored as `LogosInt`
/// (`bigint_struct_fields` — json's `JNum { n }`, `JDec { mant, scale }`). Reading one and handing
/// it to a function whose parameter is a plain `Int` emitted `i64`, so the generated Rust did not
/// compile: `expected i64, found LogosInt`, in stdlib code, pointing at a file the author never
/// wrote. The interpreter never noticed — it has ONE integer type — so this could only ever show
/// up on a build, and only once a program actually reached json.lg's serializer or its typed
/// accessors. A derived JSON decoder is the first thing that does.
///
/// The rule is narrow: a name bound from a bignum field is a bignum; a function it is passed to AT
/// AN INTEGER PARAMETER takes bignums; and since that parameter may be passed on, it is a bignum
/// name inside that function too. Each name is a bignum IN ITS OWN FUNCTION: a bignum `n` in
/// json's serializer says nothing about an `n` anywhere else, and a parameter no bignum reaches
/// only ever holds a machine word, which a call hands on narrowed. A FIXPOINT, so a chain of
/// helpers promotes together instead of half-way — which would move the same error one call
/// deeper rather than fixing it.
pub(super) fn bigint_arg_fns(
    stmts: &[Stmt],
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    interner: &Interner,
) -> HashSet<Symbol> {
    bigint_arg_fns_with_facts(
        stmts,
        bigint_fields,
        interner,
        &HashSet::new(),
        &HashMap::new(),
    )
}

fn bigint_arg_fns_with_facts(
    stmts: &[Stmt],
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    interner: &Interner,
    bigint_fns: &HashSet<Symbol>,
    exact_params: &HashMap<Symbol, HashSet<Symbol>>,
) -> HashSet<Symbol> {
    // Every scope a name lives in: `## Main`'s statements (`None`) and each function's body.
    let mut scopes: Vec<(Option<Symbol>, &[Stmt])> = vec![(None, stmts)];
    // Each function's parameters, `Some` where codegen exactifies it: a machine-word `i64`.
    let mut exactified: HashMap<Symbol, Vec<Option<Symbol>>> = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name, params, body, ..
        } = s
        {
            scopes.push((Some(*name), body));
            exactified.insert(
                *name,
                params
                    .iter()
                    .map(|(p, ty)| exactifies(ty, interner).then_some(*p))
                    .collect(),
            );
        }
    }
    let mut vars: HashMap<Option<Symbol>, HashSet<Symbol>> = HashMap::new();
    for (scope, body) in &scopes {
        let mut seeds = scope
            .and_then(|f| exact_params.get(&f))
            .cloned()
            .unwrap_or_default();
        collect_bigint_bindings(body, bigint_fields, &mut seeds);
        seeds = promotable_int_vars(body, bigint_fns, bigint_fields, &seeds);
        if !seeds.is_empty() {
            vars.insert(*scope, seeds);
        }
    }
    let mut out: HashSet<Symbol> = HashSet::new();
    loop {
        let mut grew = false;
        for (scope, body) in &scopes {
            let seeds = vars.get(scope).cloned().unwrap_or_default();
            let held = promotable_int_vars(body, bigint_fns, bigint_fields, &seeds);
            grew |= held.len() != seeds.len();
            vars.insert(*scope, held.clone());
            let mut reached: Vec<(Symbol, usize)> = Vec::new();
            collect_bigint_arg_calls(body, &held, &mut reached);
            for_each_node_in_scope(body, |node| {
                let (Visited::Expr(Expr::Call { function, args })
                | Visited::Stmt(Stmt::Call { function, args })) = node
                else {
                    return;
                };
                for (at, value) in args.iter().enumerate() {
                    if const_overflows_i64(value)
                        || mentions_promotable(value, &held)
                        || mentions_bigint_call(value, bigint_fns)
                        || reads_bigint_field(value, bigint_fields)
                    {
                        reached.push((*function, at));
                    }
                }
            });
            for (function, at) in reached {
                if let Some(Some(param)) = exactified.get(&function).and_then(|ps| ps.get(at)) {
                    out.insert(function);
                    grew |= vars.entry(Some(function)).or_default().insert(*param);
                }
            }
        }
        if !grew {
            return out;
        }
    }
}

/// Names an `Inspect` arm in one scope binds from a field the program stores as a bignum.
fn collect_bigint_bindings(
    stmts: &[Stmt],
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    out: &mut HashSet<Symbol>,
) {
    for_each_node_in_scope(stmts, |node| {
        let Visited::Stmt(Stmt::Inspect { arms, .. }) = node else {
            return;
        };
        for arm in arms {
            let Some(variant) = arm.variant else { continue };
            for (field, binding) in &arm.bindings {
                if bigint_fields.contains(&(variant, *field)) {
                    out.insert(*binding);
                }
            }
        }
    });
}

/// Every call in one scope that hands one of `vars` over directly, as `(function, argument
/// position)` — wherever the call is written: a statement, a binding, a `Show`, an argument of
/// another call, an interpolation.
fn collect_bigint_arg_calls(
    stmts: &[Stmt],
    vars: &HashSet<Symbol>,
    out: &mut Vec<(Symbol, usize)>,
) {
    let mut handed = |function: Symbol, args: &[&Expr]| {
        for (at, a) in args.iter().enumerate() {
            if matches!(a, Expr::Identifier(s) if vars.contains(s)) {
                out.push((function, at));
            }
        }
    };
    for_each_node_in_scope(stmts, |node| {
        if let Visited::Expr(Expr::Call { function, args }) = node {
            handed(*function, args);
        } else if let Visited::Stmt(Stmt::Call { function, args }) = node {
            handed(*function, args);
        }
    });
}

/// Functions that hand back a value read from a bignum field.
///
/// `bigint_returning_fns` looks for a return that mentions a PROMOTABLE local or calls a
/// bignum-returning function. A value bound straight out of a bignum variant field is neither —
/// `jsonInt`'s `When JNum (n): Return a new Ok with value n.` returns a `LogosInt` while its
/// signature says `i64`, and the generated Rust does not compile. Same root as `bigint_arg_fns`,
/// the other direction: in through the parameters, out through the return.
pub(super) fn bigint_field_returning_fns(
    stmts: &[Stmt],
    bigint_fields: &HashSet<(Symbol, Symbol)>,
) -> HashSet<Symbol> {
    let mut out = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            let mut names = HashSet::new();
            collect_bigint_bindings(body, bigint_fields, &mut names);
            if names.is_empty() {
                continue;
            }
            let mut returns_one = false;
            for_each_return(body, |e| {
                if mentions_any(e, &names) {
                    returns_one = true;
                }
            });
            if returns_one {
                out.insert(*name);
            }
        }
    }
    out
}

/// Does this expression mention any of `names`, including inside a constructed value?
fn mentions_any(e: &Expr, names: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Identifier(s) => names.contains(s),
        Expr::BinaryOp { left, right, .. } => {
            mentions_any(left, names) || mentions_any(right, names)
        }
        Expr::Not { operand } | Expr::Copy { expr: operand } => mentions_any(operand, names),
        Expr::New { init_fields, .. } => init_fields.iter().any(|(_, v)| mentions_any(v, names)),
        Expr::NewVariant { fields, .. } => fields.iter().any(|(_, v)| mentions_any(v, names)),
        Expr::Call { args, .. } => args.iter().any(|a| mentions_any(a, names)),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| mentions_any(i, names)),
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/bigint_promote/bigint_arg_tests.rs"]
mod bigint_arg_tests;
