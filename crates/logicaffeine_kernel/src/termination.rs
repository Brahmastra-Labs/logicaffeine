//! Termination checking for fixpoints.
//!
//! This module implements the syntactic guard condition that ensures all recursive functions
//! terminate. Without this check, the type system would be unsound — we could "prove" `False`
//! by writing `fix f. f`.
//!
//! # The measure
//!
//! The guard decides the LEXICOGRAPHIC product of the structural-subterm order `◁` over an
//! ORDERED list of COLUMNS:
//!
//! * **Column 0** is the Giménez structural argument — the binder the body's `match`
//!   discriminates on, or, when the body does not match on a binder, the first inductive-typed
//!   one. This is what lets an INDEXED family recurse: `le.rec`'s fixpoint is
//!   `fix rec. λm:Nat. λh:le n m. match h …` and the structural argument is the PROOF `h`
//!   (position 1), not the `Nat` index `m`.
//! * **Columns 1…** are the remaining inductive-typed binders of the λ-telescope, in
//!   positional order.
//!
//! A recursive call — to this fixpoint or a sibling in the same mutual block — is admitted iff
//! there is a column `i` whose argument is a strict subterm of column `i`'s parameter and, for
//! every earlier column `j < i`, the argument is a strict subterm OR definitionally that
//! column's parameter. `Succ m1` inside `match m with | Succ m1 => …` is definitionally `m`, so
//! Ackermann's inner call `ack (Succ m1) n2` holds column 0 EQUAL and decreases column 1 —
//! which is exactly `termination_by m n => (m, n)`.
//!
//! The lexicographic product of finitely many copies of a well-founded relation is
//! well-founded, and `◁` (strict structural subterm) is well-founded on well-typed values, so
//! an admitted call chain cannot be infinite. Note that `◁` is one relation over ALL terms —
//! it does not depend on the column's type — which is what makes a mutual block's members share
//! a single measure even when their columns are typed differently.
//!
//! # What is NOT in the measure
//!
//! "Smaller in SOME column" is not a decrease: a subterm of column 1 says nothing about
//! column 0, and a measure that let one discharge the other would accept `f n2 m1`, which grows.
//! The scan therefore stops at the first column that is neither smaller nor equal.
//!
//! # The subterm analysis
//!
//! Deciding `t ◁ p` is more than "is `t` a variable the match bound". Following Coq's
//! `subterm_specif`, the guard also walks β-redexes, δ-unfolds a transparent definition
//! applied to arguments, and analyses a `match` on a term already known `≤ p` as the MEET over
//! its branches (each branch's fields being themselves subterms of `p`). `pred n` δ-unfolds to
//! `match n with | Zero => Zero | Succ k => k`, whose `Zero` branch yields `Zero` — NOT a
//! strict subterm of `n` when `n` IS `Zero` — so the meet refuses it, exactly as Coq does.
//!
//! # The refutation refinement — `decreasing_by`, discharged in the kernel
//!
//! One branch of that meet can be RULED OUT. In case `isFalse` of a `match` whose discriminant
//! is a δ-transparent, kernel-checked DECISION PROCEDURE applied to the recursion's own data,
//! the case binds `hn : ¬(x = C)` for a NULLARY constructor `C`. Reaching that branch requires
//! the discriminant to have ι-reduced to `isFalse _`; since the decision procedure is a closed,
//! kernel-checked definition, its reduction at a closed `x ↝ C` would have to produce a closed
//! inhabitant of `¬(C = C)` — a closed proof of `False`. So where the axioms in scope are
//! consistent, the branch is unreachable at `x ↝ C` and the `C` branch of the meet drops out.
//! For a two-constructor inductive that pins `x`'s head to the other constructor, and `pred n`
//! becomes `n`'s `Succ` field — a genuine strict subterm.
//!
//! This is the ONE rule here whose soundness is conditional on the consistency of the axioms in
//! scope rather than unconditional, so it is fenced hard: the discriminant's head must be a
//! `Context::is_definition` (never a bound variable, never an axiom), every argument of its
//! spine must be a column parameter, a known subterm, a constructor application or a literal,
//! the refuted constructor must be NULLARY, the refined variable must be a live column's own
//! parameter, and it must be the discriminant's ONLY free variable. The argument above needs
//! the decision closed at `x ↝ C`: with a second open argument, a hypothesis `e : False` say, a
//! checked procedure can answer `isFalse (λh. e)` at `C` under `e`'s binder, the branch is
//! reached, and the recursion never ends there. `crates/logicaffeine_tests/tests/G_recursor_extra.rs`
//! pins each fence with the neighbouring shape it must still refuse, and
//! `crates/logicaffeine_tests/tests/kernel_paradox_corpus.rs` the closedness fence.

use std::collections::{HashMap, HashSet};

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::term::Term;

/// How deep the subterm analysis may δ-unfold and descend through `match` branches before
/// giving up. A budget is required, not merely prudent: a recursive definition unfolds
/// forever, and the guard must terminate even when its subject does not.
const SUBTERM_FUEL: u32 = 8;

/// One column of the lexicographic structural measure.
#[derive(Clone)]
struct Column {
    /// The λ-binder's name in the CURRENT body.
    name: String,
    /// Its argument POSITION in the λ-telescope, so a call's spine can be indexed.
    pos: usize,
    /// The inductive this column's type heads — the constructors a `match` on it expects.
    ind: String,
    /// The binder's full type (`TList Nat`), so the inductive's PARAMETERS are available when
    /// the branch's rebuilt constructor application is compared against a call's argument.
    ty: Term,
    /// False once an inner binder shadows `name`: the column's parameter is then no longer
    /// reachable, so nothing can be shown equal to it or smaller than it.
    live: bool,
}

/// How an argument compares with one column's parameter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rel {
    /// A strict structural subterm — this column decreases.
    Smaller,
    /// Definitionally the column's parameter — this column is held.
    Equal,
    /// Neither could be established. The lexicographic scan must stop here.
    Unknown,
}

/// Context for termination checking.
#[derive(Clone)]
struct GuardContext {
    /// Each fixpoint name in the (possibly mutual) block → the ordered argument POSITIONS of
    /// its own columns. A call to ANY member must decrease, and the argument it supplies at
    /// THAT member's column `c` is compared against the CURRENT body's column `c`.
    fix_columns: HashMap<String, Vec<usize>>,
    /// The CURRENT body's columns, in measure order.
    columns: Vec<Column>,
    /// How many columns the comparison may use: the MINIMUM column count over the block, so
    /// every member's measure lands in the same `Terms^K` and the lexicographic order is one
    /// order rather than one per member.
    lex_width: usize,
    /// Variables known to be strict structural SUBTERMS of a column's parameter:
    /// name → (column index, the inductive its own type heads, or `""` when that type heads
    /// no inductive).
    ///
    /// The subterm relation is TRANSITIVE — a field of a field of `n` is a subterm of `n` — so
    /// a `match` on a variable already known smaller contributes its own constructor fields at
    /// the SAME column. That is what makes `| n+2 => …` structurally admissible, and it is why
    /// the inductive name has to be recorded: the nested match's constructors come from the
    /// SCRUTINEE's type, which need not be the column's.
    smaller_than: HashMap<String, (usize, String)>,
    /// A live column's parameter re-expressed as the constructor application the branch
    /// destructured it into (`m ≡ Succ m1`), keyed by column index. This is the EQUAL half of
    /// the lexicographic order — without it, `ack (Succ m1) n2` looks like an increase.
    rebuilt: HashMap<usize, Term>,
    /// A column parameter whose constructor HEAD the branch's refutation hypothesis pins.
    known_head: HashMap<String, String>,
    /// Fixpoint names shadowed by an inner binder — a call to such a name refers to the
    /// shadowing binding, not our block, so it is not a recursive call.
    shadowed_fix: HashSet<String>,
}

impl GuardContext {
    /// A stable name for the fixpoint block, for error messages.
    fn block_name(&self) -> String {
        let mut names: Vec<&String> = self.fix_columns.keys().collect();
        names.sort();
        names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The index of the LIVE column whose parameter is named `name`.
    fn live_column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.live && c.name == name)
    }

    /// A one-line description of the current measure, so a refusal says what it measured.
    fn measure(&self) -> String {
        let cols: Vec<String> = self.columns[..self.lex_width.min(self.columns.len())]
            .iter()
            .map(|c| {
                if c.live {
                    c.name.clone()
                } else {
                    format!("{} (shadowed)", c.name)
                }
            })
            .collect();
        cols.join(", ")
    }
}

/// **THE ι-SPECIFICATION — which argument makes a `Fix` unroll.**
///
/// The ORDERED argument positions of the lexicographic measure this guard admitted for `body`
/// (the λ-abstraction under a `Fix`), with column 0 — the Giménez structural argument — first.
/// Empty when `body` has no inductive-typed binder at all, i.e. when the guard would refuse it.
///
/// This exists because a REDUCER has to agree with the guard, and the specification of ι for a
/// fixpoint is:
///
/// > `(fix f. λx₀ … λxₙ. e) a₀ … aₙ` unrolls exactly when the argument at the STRUCTURAL
/// > position — `structural_columns(ctx, body)[0]` — is in constructor form.
///
/// Not "when `a₀` is", which is a different rule and a strictly smaller one: the guard reads the
/// structural column off the body's own `match`, so `fix f. λm. λn. match n with …` — Lean's
/// `Nat.add`, which recurses on its SECOND argument — is admitted with column 0 at position 1. A
/// reducer that insists on position 0 leaves that definition STUCK for every non-constructor
/// first argument, so `n + 0 = n` is not `rfl` here where it is `rfl` in Lean, and `normalize`
/// hands back a term that still has a δι-redex in it. Not "when SOME argument is", either: that
/// unrolls on a non-structural constructor and makes progress the guard never promised.
///
/// The two kernels must each implement that rule from this specification — `reduction.rs` over
/// named terms, `recheck.rs` over de Bruijn levels — and NOT by calling one another or by sharing
/// a reduction routine. What is shared here is only the guard's own answer to "which column",
/// which the guard alone is the authority on: a reducer that guesses it is a reducer that
/// computes a function the guard did not check.
pub fn structural_columns(ctx: &Context, body: &Term) -> Vec<usize> {
    match extract_columns(ctx, body) {
        Ok((columns, _)) => columns.iter().map(|c| c.pos).collect(),
        Err(_) => Vec::new(),
    }
}

/// The single argument POSITION a reducer needs: the Giménez structural column, or `None` when
/// the guard admits no column at all and therefore no ι rule applies to this fixpoint.
///
/// [`structural_columns`] answers with the whole measure because the GUARD needs the whole
/// measure. A reducer needs one number, and asking it for `[0]` is an index into a vector that
/// is empty exactly when the answer is "there is none" — so the two honest outcomes are spelled
/// as `Some`/`None` here rather than left to a caller to remember. `(fix f. λx₀ … λxₙ. e) a₀ … aₙ`
/// unrolls exactly when `a_p` is constructor-headed, for `p` this position.
///
/// ```
/// use logicaffeine_kernel::prelude::StandardLibrary;
/// use logicaffeine_kernel::termination::structural_position;
/// use logicaffeine_kernel::{Term, Universe};
///
/// let ctx = StandardLibrary::context();
/// let g = |n: &str| Term::Global(n.into());
/// let v = |n: &str| Term::Var(n.into());
/// let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
/// let lam = |p: &str, t: Term, b: Term| Term::Lambda { param: p.into(), param_type: Box::new(t), body: Box::new(b) };
///
/// // Lean's `Nat.add` recurses on its SECOND argument: λm. λn. match n with 0 ⇒ m | k+1 ⇒ (add m k)+1.
/// let add = lam("m", g("Nat"), lam("n", g("Nat"), Term::Match {
///     discriminant: Box::new(v("n")),
///     motive: Box::new(lam("_", g("Nat"), g("Nat"))),
///     cases: vec![v("m"), lam("k", g("Nat"), app(g("Succ"), app(app(v("add"), v("m")), v("k"))))],
/// }));
/// assert_eq!(structural_position(ctx, &add), Some(1));
///
/// // No inductive-typed binder: no column, so no ι rule.
/// let id = lam("A", Term::Sort(Universe::Type(0)), lam("a", v("A"), v("a")));
/// assert_eq!(structural_position(ctx, &id), None);
/// ```
///
/// A fixpoint the guard would REFUSE gets `None`, which is the right answer for a reducer too:
/// an unguarded fixpoint has no ι rule, and unrolling one is how a normaliser diverges.
pub fn structural_position(ctx: &Context, body: &Term) -> Option<usize> {
    extract_columns(ctx, body)
        .ok()
        .and_then(|(columns, _)| columns.first().map(|c| c.pos))
}

/// Shared recursion-column policy for named and de Bruijn syntax. Prefer the
/// matched parameter when inductive-typed, otherwise the first inductive one.
pub(crate) fn select_structural_position(
    candidates: impl IntoIterator<Item = (usize, bool)>,
    preferred: Option<usize>,
) -> Option<usize> {
    let mut first = None;
    for (position, inductive) in candidates {
        if inductive {
            if preferred == Some(position) {
                return Some(position);
            }
            first.get_or_insert(position);
        }
    }
    first
}

/// Check that a Fix term satisfies the syntactic guard condition.
///
/// This is the main entry point for termination checking.
pub fn check_termination(ctx: &Context, fix_name: &str, body: &Term) -> KernelResult<()> {
    let (columns, core) = extract_columns(ctx, body)?;
    let positions: Vec<usize> = columns.iter().map(|c| c.pos).collect();
    let lex_width = positions.len();

    // A single fixpoint is a mutual block of one.
    let mut fix_columns = HashMap::new();
    fix_columns.insert(fix_name.to_string(), positions);
    let guard_ctx = seed_context(body, fix_columns, columns, lex_width);

    guard_telescope(ctx, &guard_ctx, body)?;
    check_guarded(ctx, &guard_ctx, core)
}

/// Check that a MUTUAL block of fixpoints terminates — the mutual Giménez guard.
///
/// Every member's columns are computed first; then each body is checked so that a call to ANY
/// member — itself OR a sibling — decreases the SHARED lexicographic measure with respect to
/// the CURRENT body's columns. Any infinite call chain would then demand an infinite strictly
/// descending sequence in `Terms^K`, which cannot exist — so the whole block terminates.
/// `Even.rec`'s fixpoint calling `Odd.rec` on the sub-proof, and vice versa, is exactly what
/// this guards.
///
/// `K` is the MINIMUM column count over the block: every member's measure must land in one
/// common `Terms^K`, or the comparison would be between tuples of different shapes.
pub fn check_termination_mutual(ctx: &Context, defs: &[(String, Term)]) -> KernelResult<()> {
    if defs.is_empty() {
        return Err(KernelError::TerminationViolation {
            fix_name: String::new(),
            reason: "empty mutual fixpoint block".to_string(),
        });
    }
    // 1. Columns of every member.
    let mut fix_columns = HashMap::new();
    let mut per_member: Vec<Vec<Column>> = Vec::with_capacity(defs.len());
    let mut lex_width = usize::MAX;
    for (name, body) in defs {
        let (columns, _) = extract_columns(ctx, body)?;
        fix_columns.insert(
            name.clone(),
            columns.iter().map(|c| c.pos).collect::<Vec<_>>(),
        );
        lex_width = lex_width.min(columns.len());
        per_member.push(columns);
    }
    // 2. Check every body against the whole block.
    for (i, (_, body)) in defs.iter().enumerate() {
        let (_, core) = extract_columns(ctx, body)?;
        let guard_ctx = seed_context(body, fix_columns.clone(), per_member[i].clone(), lex_width);
        guard_telescope(ctx, &guard_ctx, body)?;
        check_guarded(ctx, &guard_ctx, core)?;
    }
    Ok(())
}

/// The guard context a body starts in: its columns, plus the block members its own λ-telescope
/// SHADOWS. A binder named after a block member rebinds that name, so occurrences inside the
/// body denote the binder and are not recursive calls at all.
fn seed_context(
    body: &Term,
    fix_columns: HashMap<String, Vec<usize>>,
    columns: Vec<Column>,
    lex_width: usize,
) -> GuardContext {
    let mut shadowed_fix = HashSet::new();
    let mut cur = body;
    while let Term::Lambda {
        param, body: inner, ..
    } = cur
    {
        if fix_columns.contains_key(param) {
            shadowed_fix.insert(param.clone());
        }
        cur = inner;
    }
    GuardContext {
        fix_columns,
        columns,
        lex_width,
        smaller_than: HashMap::new(),
        rebuilt: HashMap::new(),
        known_head: HashMap::new(),
        shadowed_fix,
    }
}

/// Guard every λ-telescope binder's DOMAIN annotation. The telescope is peeled whole before the
/// body is walked, so without this a recursive occurrence hidden in a parameter's type would
/// never be visited.
///
/// Binder `i`'s domain is checked with every column bound at or after `i` marked DEAD: those
/// parameters are not in scope there, so nothing in that domain may be compared against them.
/// The type checker would reject such a reference anyway, but a guard that can be talked into
/// admitting a call by reading a not-yet-bound column is a guard that only works because
/// something else is watching.
fn guard_telescope(ctx: &Context, guard_ctx: &GuardContext, body: &Term) -> KernelResult<()> {
    let mut cur = body;
    let mut i = 0usize;
    while let Term::Lambda {
        param_type,
        body: inner,
        ..
    } = cur
    {
        let mut scoped = guard_ctx.clone();
        for c in scoped.columns.iter_mut() {
            if c.pos >= i {
                c.live = false;
            }
        }
        check_guarded(ctx, &scoped, param_type)?;
        cur = inner;
        i += 1;
    }
    Ok(())
}

/// Identify the COLUMNS of the lexicographic measure and return the body past the whole
/// λ-telescope.
///
/// Column 0 is the binder the body's `match` actually discriminates on (the scrutinee) when
/// that binder is inductive-typed — this is what lets an INDEXED family recurse on its proof
/// rather than on a `Nat` index — and otherwise the first inductive-typed binder. The
/// remaining inductive-typed binders follow in positional order.
///
/// A binder that REBINDS an earlier column's name kills that column: within the telescope the
/// name already denotes the later binder, so nothing can be compared against the earlier one.
fn extract_columns<'a>(ctx: &Context, body: &'a Term) -> KernelResult<(Vec<Column>, &'a Term)> {
    // Peel the λ-telescope, recording each binder and its type.
    let mut chain: Vec<(&'a str, &'a Term)> = Vec::new();
    let mut cur = body;
    while let Term::Lambda {
        param,
        param_type,
        body: inner,
    } = cur
    {
        chain.push((param.as_str(), param_type.as_ref()));
        cur = inner;
    }

    // The scrutinee: the binder the innermost `match` discriminates on, if any, read through an
    // application spine, so a convoy `(match v … ) w …` recurses on `v` as a bare `match` does.
    let mut matched = cur;
    while let Term::App(function, _) = matched {
        matched = function;
    }
    let scrutinee = match matched {
        Term::Match { discriminant, .. } => match discriminant.as_ref() {
            Term::Var(d) => chain.iter().position(|(n, _)| n == d),
            _ => None,
        },
        _ => None,
    };

    let primary = select_structural_position(
        chain
            .iter()
            .enumerate()
            .map(|(position, (_, ty))| (position, extract_inductive_name(ctx, ty).is_some())),
        scrutinee,
    )
    .ok_or_else(|| KernelError::TerminationViolation {
        fix_name: String::new(),
        reason: "No inductive parameter found for structural recursion".to_string(),
    })?;

    let mut order: Vec<usize> = vec![primary];
    for (i, (_, t)) in chain.iter().enumerate() {
        if i != primary && extract_inductive_name(ctx, t).is_some() {
            order.push(i);
        }
    }
    let mut columns: Vec<Column> = order
        .iter()
        .map(|&i| Column {
            name: chain[i].0.to_string(),
            pos: i,
            ind: extract_inductive_name(ctx, chain[i].1).unwrap(),
            ty: chain[i].1.clone(),
            live: true,
        })
        .collect();
    for (i, (n, _)) in chain.iter().enumerate() {
        for c in columns.iter_mut() {
            if c.pos < i && c.name == *n {
                c.live = false;
            }
        }
    }
    Ok((columns, cur))
}

/// Extract the inductive type name from a type.
///
/// Handles both simple inductives like `Nat` and polymorphic ones like `List A`.
fn extract_inductive_name(ctx: &Context, ty: &Term) -> Option<String> {
    match ty {
        Term::Global(name) | Term::Const { name, .. } if ctx.is_inductive(name) => {
            Some(name.clone())
        }
        Term::App(func, _) => extract_inductive_name(ctx, func),
        _ => None,
    }
}

/// Build the guard context for the scope under a binder that introduces `param`.
///
/// The binder shadows any prior meaning of `param`: within its body `param` no longer refers to
/// a column's parameter, a structurally-smaller variable, a refined variable, or the fixpoint
/// itself. Failing to honor this lets an inner `match` on a shadowed name be mistaken for the
/// guarding match (admitting a non-decreasing recursion), or an inner binding of a
/// smaller-marked name keep its "smaller" status after being rebound to an arbitrary value.
fn enter_binder(guard_ctx: &GuardContext, param: &str) -> GuardContext {
    let mut child = guard_ctx.clone();
    child.smaller_than.remove(param);
    child.known_head.remove(param);
    let killed: Vec<usize> = child
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.live && c.name == param)
        .map(|(i, _)| i)
        .collect();
    for i in killed {
        child.columns[i].live = false;
        child.rebuilt.remove(&i);
    }
    if guard_ctx.fix_columns.contains_key(param) {
        child.shadowed_fix.insert(param.to_string());
    }
    child
}

/// Check that all recursive calls in `term` are guarded (decrease the lexicographic measure).
fn check_guarded(ctx: &Context, guard_ctx: &GuardContext, term: &Term) -> KernelResult<()> {
    match term {
        // Application. Peel the spine into (head, args-in-order). The recursive name is allowed
        // to occur ONLY here, as the head of a call that decreases the measure; in that
        // position the head is *consumed* by the application and must NOT be re-descended as a
        // bare value. Every argument is still checked, so a recursive name smuggled into an
        // argument (`g f`) is caught by the `Var` leaf below.
        Term::App(func, arg) => {
            let mut args: Vec<&Term> = vec![arg.as_ref()];
            let mut head = func.as_ref();
            while let Term::App(inner_func, inner_arg) = head {
                args.push(inner_arg.as_ref());
                head = inner_func.as_ref();
            }
            args.reverse();

            if let Term::Var(name) = head {
                if !guard_ctx.shadowed_fix.contains(name) {
                    if let Some(cols) = guard_ctx.fix_columns.get(name) {
                        verify_lex_decrease(ctx, guard_ctx, name, cols, &args)?;
                        for a in &args {
                            check_guarded(ctx, guard_ctx, a)?;
                        }
                        return Ok(());
                    }
                }
            }

            // Not a recursive call at the head: check the head and every argument normally.
            check_guarded(ctx, guard_ctx, head)?;
            for a in &args {
                check_guarded(ctx, guard_ctx, a)?;
            }
            Ok(())
        }

        // A match on a column's parameter (or on a variable already known smaller) introduces
        // that constructor's fields as strict subterms of THAT column, and records the
        // parameter's rebuilt constructor application so a call passing it reads as EQUAL.
        Term::Match { discriminant, motive, cases } => {
            // The return motive is an ordinary subterm (in the current scope) and MUST be
            // guarded too — a recursive occurrence hidden in the return predicate would
            // otherwise evade the check, diverging from the standard CIC guard.
            check_guarded(ctx, guard_ctx, motive)?;
            if let Term::Var(disc_name) = discriminant.as_ref() {
                if let Some(ci) = guard_ctx.live_column(disc_name) {
                    let col = guard_ctx.columns[ci].clone();
                    return check_match_cases_guarded(
                        ctx,
                        guard_ctx,
                        cases,
                        &col.ind,
                        ci,
                        Some(&col.ty),
                    );
                }
                if let Some((ci, ind)) = guard_ctx.smaller_than.get(disc_name).cloned() {
                    if !ind.is_empty() {
                        return check_match_cases_guarded(ctx, guard_ctx, cases, &ind, ci, None);
                    }
                }
            }

            // Not a guarding match. Its branches may still carry a REFUTATION that pins a
            // column parameter's constructor head — Lean's `decreasing_by` hypothesis.
            check_guarded(ctx, guard_ctx, discriminant)?;
            let refinements = decidable_refinements(ctx, guard_ctx, discriminant, cases);
            for (i, case) in cases.iter().enumerate() {
                match refinements.get(&i) {
                    Some((x, ctor)) => {
                        let mut child = guard_ctx.clone();
                        child.known_head.insert(x.clone(), ctor.clone());
                        check_guarded(ctx, &child, case)?;
                    }
                    None => check_guarded(ctx, guard_ctx, case)?,
                }
            }
            Ok(())
        }

        // Lambda: guard the DOMAIN annotation (current scope) as well as the body — a
        // recursive occurrence in a binder's type must not evade the check. The binder may
        // shadow a column's parameter, a smaller variable, or the fixpoint name.
        Term::Lambda { param, param_type, body } => {
            check_guarded(ctx, guard_ctx, param_type)?;
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body)
        }

        // Pi: same domain + binder-shadowing handling as Lambda.
        Term::Pi { param, param_type, body_type } => {
            check_guarded(ctx, guard_ctx, param_type)?;
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body_type)
        }

        // Nested fixpoint: its own name shadows ours; its body gets its own
        // termination check when type-checked.
        Term::Fix { name, body } => {
            let child = enter_binder(guard_ctx, name);
            check_guarded(ctx, &child, body)
        }

        // Nested MUTUAL fixpoint: all of its names shadow ours; each body gets its own
        // mutual termination check when type-checked.
        Term::MutualFix { defs, .. } => {
            let mut child = enter_binder(guard_ctx, &defs[0].0);
            for (n, _) in &defs[1..] {
                child = enter_binder(&child, n);
            }
            for (_, b) in defs {
                check_guarded(ctx, &child, b)?;
            }
            Ok(())
        }

        // Let: the type and value are in the outer scope; the body is under the let-binder,
        // which may shadow the tracked names.
        //
        // A let-bound name whose VALUE is an already-smaller variable INHERITS its smallness,
        // at the SAME column. Zeta makes `let x := v in b` and `b[x := v]` the same term, so if
        // `v` is a structural subterm then so is `x` — refusing the alias rejected a definition
        // purely for how it was spelled, and a lowering pass that introduces a `let` for
        // sharing turned a passing definition into a build error.
        Term::Let { name, ty, value, body, .. } => {
            check_guarded(ctx, guard_ctx, ty)?;
            check_guarded(ctx, guard_ctx, value)?;
            let mut child = enter_binder(guard_ctx, name);
            if let Term::Var(aliased) = value.as_ref() {
                if let Some(entry) = guard_ctx.smaller_than.get(aliased) {
                    child.smaller_than.insert(name.clone(), entry.clone());
                }
            }
            check_guarded(ctx, &child, body)
        }

        // A bare occurrence of a block member's name is the higher-order escape: `f` used
        // as a first-class value (returned, or passed as an argument) rather than applied
        // to a decreasing argument tuple. Only fully-applied decreasing calls are guarded
        // (Giménez 1995; the Coq guard), so reject it. Valid recursive calls never reach
        // here — their head is consumed by the `App` arm above.
        Term::Var(name)
            if guard_ctx.fix_columns.contains_key(name)
                && !guard_ctx.shadowed_fix.contains(name) =>
        {
            Err(KernelError::TerminationViolation {
                fix_name: name.clone(),
                reason: format!(
                    "recursive name '{}' occurs as a first-class value, not applied to a structurally-smaller argument",
                    name
                ),
            })
        }

        // Other leaves: no recursive calls possible.
        Term::Sort(_) | Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole
        | Term::Const { .. } => Ok(()),

        // An annotation hides nothing from the guard: every call under it is checked.
        Term::Annotated { term, .. } => check_guarded(ctx, guard_ctx, term),
    }
}

/// Verify one recursive call decreases the lexicographic measure.
///
/// Scan the columns left to right against the CURRENT body's parameters. A strictly smaller
/// argument admits the call outright — every later column is then free, which is what
/// lexicographic means. An EQUAL argument holds that column and the scan continues. Anything
/// else STOPS the scan: a column that cannot be shown `≤` may have grown, and then no later
/// column's decrease is a decrease of the tuple. A call that does not even supply the column
/// stops there for the same reason.
fn verify_lex_decrease(
    ctx: &Context,
    guard_ctx: &GuardContext,
    callee: &str,
    callee_columns: &[usize],
    args: &[&Term],
) -> KernelResult<()> {
    let width = guard_ctx
        .lex_width
        .min(callee_columns.len())
        .min(guard_ctx.columns.len());
    for (c, position) in callee_columns.iter().enumerate().take(width) {
        let Some(arg) = args.get(*position) else {
            break;
        };
        match classify(ctx, guard_ctx, c, arg) {
            Rel::Smaller => return Ok(()),
            Rel::Equal => continue,
            Rel::Unknown => break,
        }
    }
    Err(KernelError::TerminationViolation {
        fix_name: guard_ctx.block_name(),
        reason: format!(
            "recursive call to '{callee}' does not decrease the structural measure ({}): no \
             column is passed a strict subterm of its parameter with every earlier column held \
             equal",
            guard_ctx.measure()
        ),
    })
}

/// Compare one argument against one column's parameter.
fn classify(ctx: &Context, guard_ctx: &GuardContext, col: usize, arg: &Term) -> Rel {
    let Some(column) = guard_ctx.columns.get(col) else {
        return Rel::Unknown;
    };
    // A shadowed column's parameter is not reachable from here, so nothing can be shown
    // equal to it or smaller than it.
    if !column.live {
        return Rel::Unknown;
    }
    // EQUAL — the column's own parameter, passed through unchanged.
    if matches!(arg, Term::Var(v) if *v == column.name) {
        return Rel::Equal;
    }
    // EQUAL — the parameter re-expressed as the constructor this branch destructured it into.
    if guard_ctx.rebuilt.get(&col).is_some_and(|r| r == arg) {
        return Rel::Equal;
    }
    if subterm_of(
        ctx,
        guard_ctx,
        col,
        arg,
        &guard_ctx.smaller_than,
        SUBTERM_FUEL,
    ) {
        return Rel::Smaller;
    }
    Rel::Unknown
}

/// Decide whether `t` is a strict structural SUBTERM of column `col`'s parameter.
///
/// Four admissible shapes, following Coq's `subterm_specif`:
///
/// * a bare variable bound by matching on that column's parameter (`x` in
///   `match n with Succ x => … rec x …`), or transitively on such a variable;
/// * an APPLICATION `h a₁ … aₙ` whose HEAD `h` is such a variable (Giménez's rule). This is
///   what makes recursion over an accessibility proof terminate: `Acc_intro`'s field
///   `h : Π(y). R y x → Acc A R y` is bound by the match and marked smaller, and `h y hr` is a
///   sub-`Acc`-proof it contains. The applied form is SOUND precisely because strict positivity
///   (`positivity.rs`) guarantees every functional constructor field places the inductive only
///   in a codomain-result position — so applying such a field can only yield a proper SUBTERM,
///   never a larger one. A field placing the inductive in a DOMAIN (the `(Bad → …) → Bad`
///   paradox) is rejected at inductive-registration, so it never reaches this guard;
/// * a β-redex or a δ-transparent DEFINITION applied to arguments, unfolded one step; and
/// * a `match` on a term already known `≤` the parameter, whose EVERY reachable branch yields
///   a subterm — the meet.
fn subterm_of(
    ctx: &Context,
    guard_ctx: &GuardContext,
    col: usize,
    t: &Term,
    smaller: &HashMap<String, (usize, String)>,
    fuel: u32,
) -> bool {
    if fuel == 0 {
        return false;
    }
    let mut head = t;
    let mut args: Vec<&Term> = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    args.reverse();

    match head {
        Term::Var(v) => smaller.get(v).is_some_and(|(c, _)| *c == col),
        Term::Global(n) => unfold_subterm(ctx, guard_ctx, col, n, &args, smaller, fuel),
        Term::Const { name, .. } => unfold_subterm(ctx, guard_ctx, col, name, &args, smaller, fuel),
        Term::Lambda { .. } if !args.is_empty() => match beta_apply(head, &args) {
            Some(reduced) => subterm_of(ctx, guard_ctx, col, &reduced, smaller, fuel - 1),
            None => false,
        },
        Term::Match {
            discriminant,
            cases,
            ..
        } if args.is_empty() => {
            match_subterm(ctx, guard_ctx, col, discriminant, cases, smaller, fuel)
        }
        _ => false,
    }
}

/// δ-unfold a transparent definition applied to `args` and re-analyse the result. An axiom, a
/// constructor and an inductive have no body, so they are inert — and a constructor
/// application is precisely NOT a subterm of anything it was built from.
fn unfold_subterm(
    ctx: &Context,
    guard_ctx: &GuardContext,
    col: usize,
    name: &str,
    args: &[&Term],
    smaller: &HashMap<String, (usize, String)>,
    fuel: u32,
) -> bool {
    if !ctx.is_definition(name) {
        return false;
    }
    let Some(body) = ctx.get_definition_body(name) else {
        return false;
    };
    let body = body.clone();
    // A definition's body must be CLOSED for its unfolding to mean anything here: a free
    // variable in it would be captured by whatever name happens to be in scope at the call
    // site, and the analysis would be reasoning about a term the reducer never produces.
    if !free_vars(&body).is_empty() {
        return false;
    }
    match beta_apply(&body, args) {
        Some(reduced) => subterm_of(ctx, guard_ctx, col, &reduced, smaller, fuel - 1),
        None => false,
    }
}

/// The MEET over a `match`'s branches: the whole match is a subterm of column `col`'s parameter
/// when every REACHABLE branch is. The scrutinee must itself be `≤` that parameter — the
/// parameter, or a variable already known smaller at the same column — since the branch's bound
/// fields are only subterms of the parameter when the thing they were destructured from is.
///
/// A branch whose constructor a refutation hypothesis has ruled out contributes nothing.
fn match_subterm(
    ctx: &Context,
    guard_ctx: &GuardContext,
    col: usize,
    discriminant: &Term,
    cases: &[Term],
    smaller: &HashMap<String, (usize, String)>,
    fuel: u32,
) -> bool {
    let Term::Var(d) = discriminant else {
        return false;
    };
    let disc_ind = match guard_ctx.live_column(d) {
        Some(ci) if ci == col => guard_ctx.columns[ci].ind.clone(),
        Some(_) => return false,
        None => match smaller.get(d) {
            Some((c, ind)) if *c == col && !ind.is_empty() => ind.clone(),
            _ => return false,
        },
    };
    let constructors = ctx.get_constructors(&disc_ind);
    if constructors.len() != cases.len() || constructors.is_empty() {
        return false;
    }
    let num_type_params = ctx.inductive_num_params(&disc_ind);
    let mut reachable = 0usize;
    for (i, (case, (cname, ctor_type))) in cases.iter().zip(constructors.iter()).enumerate() {
        let _ = i;
        if guard_ctx.known_head.get(d).is_some_and(|k| k != cname) {
            continue;
        }
        let param_count = count_pi_params(ctor_type).saturating_sub(num_type_params);
        let (fields, body) = extract_lambda_params(case, param_count);
        if fields.len() != param_count {
            return false;
        }
        let field_inductives = field_type_heads(ctx, ctor_type, num_type_params);
        let mut extended = smaller.clone();
        for (j, f) in fields.iter().enumerate() {
            extended.insert(
                f.clone(),
                (col, field_inductives.get(j).cloned().unwrap_or_default()),
            );
        }
        if !subterm_of(ctx, guard_ctx, col, body, &extended, fuel - 1) {
            return false;
        }
        reachable += 1;
    }
    reachable > 0
}

/// β-apply `f` to `args`, refusing (as `None`) any substitution that would CAPTURE a free
/// variable of an argument under one of `f`'s own binders. The analysis is a decision
/// procedure, so an answer it cannot compute safely must be "no", never a wrong "yes".
fn beta_apply(f: &Term, args: &[&Term]) -> Option<Term> {
    let mut cur = f.clone();
    let mut rest = args;
    while !rest.is_empty() {
        match cur {
            Term::Lambda { param, body, .. } => {
                let free = free_vars(rest[0]);
                cur = subst_safe(&body, &param, rest[0], &free)?;
                rest = &rest[1..];
            }
            _ => return Some(apply_all(cur, rest)),
        }
    }
    Some(cur)
}

fn apply_all(f: Term, args: &[&Term]) -> Term {
    args.iter()
        .fold(f, |a, x| Term::App(Box::new(a), Box::new((*x).clone())))
}

/// Substitute `value` for the free occurrences of `x` in `t`, returning `None` if a binder on
/// the way would CAPTURE one of `value`'s free variables.
fn subst_safe(t: &Term, x: &str, value: &Term, free: &HashSet<String>) -> Option<Term> {
    Some(match t {
        Term::Var(n) if n == x => value.clone(),
        Term::Var(_)
        | Term::Global(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => t.clone(),
        Term::App(f, a) => Term::App(
            Box::new(subst_safe(f, x, value, free)?),
            Box::new(subst_safe(a, x, value, free)?),
        ),
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let param_type = Box::new(subst_safe(param_type, x, value, free)?);
            if param == x {
                Term::Lambda {
                    param: param.clone(),
                    param_type,
                    body: body.clone(),
                }
            } else {
                if free.contains(param) {
                    return None;
                }
                Term::Lambda {
                    param: param.clone(),
                    param_type,
                    body: Box::new(subst_safe(body, x, value, free)?),
                }
            }
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let param_type = Box::new(subst_safe(param_type, x, value, free)?);
            if param == x {
                Term::Pi {
                    param: param.clone(),
                    param_type,
                    body_type: body_type.clone(),
                }
            } else {
                if free.contains(param) {
                    return None;
                }
                Term::Pi {
                    param: param.clone(),
                    param_type,
                    body_type: Box::new(subst_safe(body_type, x, value, free)?),
                }
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(subst_safe(discriminant, x, value, free)?),
            motive: Box::new(subst_safe(motive, x, value, free)?),
            cases: cases
                .iter()
                .map(|c| subst_safe(c, x, value, free))
                .collect::<Option<Vec<_>>>()?,
        },
        Term::Fix { name, body } => {
            if name == x {
                t.clone()
            } else {
                if free.contains(name) {
                    return None;
                }
                Term::Fix {
                    name: name.clone(),
                    body: Box::new(subst_safe(body, x, value, free)?),
                }
            }
        }
        Term::MutualFix { defs, index } => {
            if defs.iter().any(|(n, _)| n == x) {
                t.clone()
            } else {
                if defs.iter().any(|(n, _)| free.contains(n)) {
                    return None;
                }
                Term::MutualFix {
                    defs: defs
                        .iter()
                        .map(|(n, b)| Some((n.clone(), subst_safe(b, x, value, free)?)))
                        .collect::<Option<Vec<_>>>()?,
                    index: *index,
                }
            }
        }
        Term::Let {
            name,
            ty,
            value: v,
            body,
        } => {
            let ty = Box::new(subst_safe(ty, x, value, free)?);
            let v = Box::new(subst_safe(v, x, value, free)?);
            if name == x {
                Term::Let {
                    name: name.clone(),
                    ty,
                    value: v,
                    body: body.clone(),
                }
            } else {
                if free.contains(name) {
                    return None;
                }
                Term::Let {
                    name: name.clone(),
                    ty,
                    value: v,
                    body: Box::new(subst_safe(body, x, value, free)?),
                }
            }
        }
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(subst_safe(term, x, value, free)?),
        },
    })
}

/// Every free variable name of `t` — the capture set a substitution must avoid.
fn free_vars(t: &Term) -> HashSet<String> {
    fn go(t: &Term, bound: &mut Vec<String>, out: &mut HashSet<String>) {
        match t {
            Term::Var(n) => {
                if !bound.iter().any(|b| b == n) {
                    out.insert(n.clone());
                }
            }
            Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => {}
            Term::App(f, a) => {
                go(f, bound, out);
                go(a, bound, out);
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                go(param_type, bound, out);
                bound.push(param.clone());
                go(body, bound, out);
                bound.pop();
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                go(param_type, bound, out);
                bound.push(param.clone());
                go(body_type, bound, out);
                bound.pop();
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                go(discriminant, bound, out);
                go(motive, bound, out);
                for c in cases {
                    go(c, bound, out);
                }
            }
            Term::Fix { name, body } => {
                bound.push(name.clone());
                go(body, bound, out);
                bound.pop();
            }
            Term::MutualFix { defs, .. } => {
                for (n, _) in defs {
                    bound.push(n.clone());
                }
                for (_, b) in defs {
                    go(b, bound, out);
                }
                for _ in defs {
                    bound.pop();
                }
            }
            Term::Let {
                name,
                ty,
                value,
                body,
                ..
            } => {
                go(ty, bound, out);
                go(value, bound, out);
                bound.push(name.clone());
                go(body, bound, out);
                bound.pop();
            }
            Term::Annotated { term, .. } => go(term, bound, out),
        }
    }
    let mut out = HashSet::new();
    go(t, &mut Vec::new(), &mut out);
    out
}

/// The REFUTATION REFINEMENT — see the module header for the soundness argument and the fences.
///
/// Read off each case of a non-guarding `match` whether its binder's type pins a column
/// parameter's constructor head, returning `case index → (parameter, constructor)`.
fn decidable_refinements(
    ctx: &Context,
    guard_ctx: &GuardContext,
    discriminant: &Term,
    cases: &[Term],
) -> HashMap<usize, (String, String)> {
    let mut out = HashMap::new();
    if !discriminant_is_a_checked_decision(ctx, guard_ctx, discriminant) {
        return out;
    }
    let open = free_vars(discriminant);
    for (i, case) in cases.iter().enumerate() {
        let Term::Lambda { param_type, .. } = case else {
            continue;
        };
        // `hn : Not (Eq I x C)` REFUTES `C`; `hp : Eq I x C` CONFIRMS it.
        let (refuted, prop) = match param_type.as_ref() {
            Term::App(f, p) if head_name(f) == Some("Not") => (true, p.as_ref()),
            p => (false, p),
        };
        let Some((x, ctor)) = equation_against_a_constructor(ctx, prop) else {
            continue;
        };
        if guard_ctx.live_column(&x).is_none() {
            continue;
        }
        // The branch is unreachable at `x ↝ C` only because the decision would then be CLOSED and
        // reduce to a closed proof of `¬(C = C)`. Any other free variable — a hypothesis of an
        // empty type, say — can build that proof under its binder, and the branch is reached.
        if open.iter().any(|v| *v != x) {
            continue;
        }
        let Some(ind) = ctx.constructor_inductive(&ctor) else {
            continue;
        };
        if !refuted {
            out.insert(i, (x, ctor));
            continue;
        }
        // Refuting one constructor pins the head only when there is exactly one other, and
        // only when the refuted one is NULLARY: `x ≠ Succ k` leaves `x` free to be `Succ j`.
        let constructors = ctx.get_constructors(ind);
        if constructors.len() != 2 {
            continue;
        }
        let nullary = constructors
            .iter()
            .find(|(n, _)| *n == ctor)
            .is_some_and(|(_, t)| count_pi_params(t) == ctx.inductive_num_params(ind));
        if !nullary {
            continue;
        }
        if let Some((other, _)) = constructors.iter().find(|(n, _)| *n != ctor) {
            out.insert(i, (x, other.to_string()));
        }
    }
    out
}

/// The refinement's outer fence: the discriminant must be a δ-transparent, kernel-checked
/// DEFINITION applied to the recursion's own data. A bound variable of `Decidable` type is
/// chosen by the CALLER, and an axiom is chosen by whoever wrote it — neither carries the
/// reduction guarantee the refinement rests on.
fn discriminant_is_a_checked_decision(
    ctx: &Context,
    guard_ctx: &GuardContext,
    discriminant: &Term,
) -> bool {
    let mut head = discriminant;
    let mut args: Vec<&Term> = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    let Some(name) = head_name(head) else {
        return false;
    };
    if !ctx.is_definition(name) {
        return false;
    }
    args.iter().all(|a| is_inert_data(ctx, guard_ctx, a))
}

/// Data the recursion already owns: a column's parameter, a known subterm, a literal, or a
/// constructor applied to such things. Anything else could be a `Decidable` value the caller
/// supplied, and the fence exists to keep those out.
fn is_inert_data(ctx: &Context, guard_ctx: &GuardContext, t: &Term) -> bool {
    let mut head = t;
    let mut args: Vec<&Term> = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    match head {
        Term::Var(v) => {
            args.is_empty()
                && (guard_ctx.live_column(v).is_some() || guard_ctx.smaller_than.contains_key(v))
        }
        Term::Lit(_) | Term::Sort(_) => args.is_empty(),
        Term::Global(_) | Term::Const { .. } => {
            head_name(head).is_some_and(|n| ctx.is_constructor(n) || ctx.is_inductive(n))
                && args.iter().all(|a| is_inert_data(ctx, guard_ctx, a))
        }
        _ => false,
    }
}

/// `Eq I x C…` (in either orientation) where `x` is a variable and `C` a constructor.
fn equation_against_a_constructor(ctx: &Context, prop: &Term) -> Option<(String, String)> {
    let mut head = prop;
    let mut args: Vec<&Term> = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    args.reverse();
    if head_name(head) != Some("Eq") || args.len() != 3 {
        return None;
    }
    let ctor_head = |t: &Term| -> Option<String> {
        let mut h = t;
        while let Term::App(f, _) = h {
            h = f.as_ref();
        }
        head_name(h)
            .filter(|n| ctx.is_constructor(n))
            .map(|n| n.to_string())
    };
    match (args[1], args[2]) {
        (Term::Var(x), c) => ctor_head(c).map(|n| (x.clone(), n)),
        (c, Term::Var(x)) => ctor_head(c).map(|n| (x.clone(), n)),
        _ => None,
    }
}

/// The name a `Global` or a universe-polymorphic `Const` refers to. They must never be
/// conflated in the paradox fence, but here both are simply a reference to a declaration.
fn head_name(t: &Term) -> Option<&str> {
    match t {
        Term::Global(n) | Term::Const { name: n, .. } => Some(n.as_str()),
        _ => None,
    }
}

/// Check match cases with the constructor's fields marked as strict subterms of `column`'s
/// parameter, and — when the discriminant IS that parameter — the parameter's rebuilt
/// constructor application recorded so a call passing it reads as EQUAL rather than as a
/// growth.
fn check_match_cases_guarded(
    ctx: &Context,
    guard_ctx: &GuardContext,
    cases: &[Term],
    disc_inductive: &str,
    column: usize,
    disc_type: Option<&Term>,
) -> KernelResult<()> {
    // Get constructors for the SCRUTINEE's inductive type — which is the column's own type for
    // a match on its parameter, and the smaller variable's own type for a nested one.
    let constructors = ctx.get_constructors(disc_inductive);

    // A match whose case count does not match the inductive's is not a match this guard can
    // interpret — the positional zip below would pair a case with the WRONG constructor's arity
    // and, worse, would simply DROP any case past the constructor list, leaving a recursive call
    // inside it unvisited. The type checker rejects such a term, but a guard that relies on that
    // is a guard with a hole in it, so fall back to guarding every case in the plain scope.
    if constructors.len() != cases.len() {
        for case in cases {
            check_guarded(ctx, guard_ctx, case)?;
        }
        return Ok(());
    }

    // How many of a constructor's Π-binders are the INDUCTIVE'S PARAMETERS, which the
    // discriminant fixes and the case therefore does NOT bind.
    let num_type_params = ctx.inductive_num_params(disc_inductive);
    // The discriminant's own parameter arguments (`Nat` in `TList Nat`), so the rebuilt
    // constructor application is the one the branch actually destructured.
    let type_args: Vec<Term> = disc_type
        .map(|t| {
            let mut head = t;
            let mut args: Vec<Term> = Vec::new();
            while let Term::App(f, a) = head {
                args.push(a.as_ref().clone());
                head = f.as_ref();
            }
            args.reverse();
            args
        })
        .unwrap_or_default();

    for (case, (ctor_name, ctor_type)) in cases.iter().zip(constructors.iter()) {
        // The case binds only the constructor's VALUE FIELDS — its Π-telescope past the
        // inductive's parameters — exactly as `compute_case_type` computes it.
        //
        // SOUNDNESS: counting the whole telescope instead over-peels on any parameterised
        // inductive. For `TCons : Π(A:Type 0). A → TList A → TList A` the case binds 2 but the
        // full count is 3, so when the motive is a function type the case's third λ — an
        // ordinary function parameter, a subterm of nothing — was marked structurally smaller.
        // That admits a non-terminating fixpoint, and `Fix` at any type then inhabits it: a
        // closed term of type `False` built from nothing but the standard library.
        // `recheck.rs::guard_match_cases` miscounted identically, so the second kernel agreed
        // and the two-kernel guarantee bought nothing. Pinned by tests/lean_conformance_t04_recursion.rs.
        let param_count = count_pi_params(ctor_type).saturating_sub(num_type_params);

        // The case is `λx1. … λxn. body`, where `x1…xn` are the constructor's value fields.
        let (smaller_vars, case_body) = extract_lambda_params(case, param_count);
        // Each field's own type head, so a nested match on one of them knows which
        // constructors to expect. Read off the constructor's telescope past the parameters, in
        // the same order the case binds them.
        let field_inductives = field_type_heads(ctx, ctor_type, num_type_params);

        let mut extended_ctx = guard_ctx.clone();

        // Mark every constructor field as a strict subterm of THIS column's parameter.
        //
        // Soundness: we only reach here when the discriminant is the column's parameter or a
        // variable already known smaller at the same column, so the matched value is
        // `ctor x1 … xn` and each `xi` is a *direct argument* of that constructor — hence a
        // genuine structural subterm. Fields of a foreign type are still marked, but `◁` is one
        // relation over all terms, so the lexicographic descent is sound regardless of type.
        for (i, field) in smaller_vars.iter().enumerate() {
            let ind = field_inductives.get(i).cloned().unwrap_or_default();
            extended_ctx
                .smaller_than
                .insert(field.clone(), (column, ind));
            // A constructor binder may reuse the name of a column's parameter or of the
            // fixpoint; inside this case that name now refers to the (smaller) bound variable,
            // so the original meaning is shadowed.
            let killed: Vec<usize> = extended_ctx
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.live && c.name == *field)
                .map(|(i, _)| i)
                .collect();
            for k in killed {
                extended_ctx.columns[k].live = false;
                extended_ctx.rebuilt.remove(&k);
            }
            extended_ctx.known_head.remove(field);
            if guard_ctx.fix_columns.contains_key(field) {
                extended_ctx.shadowed_fix.insert(field.clone());
            }
        }

        // The branch destructured the column's parameter into THIS constructor, so within it
        // the parameter is definitionally `ctor params… fields…`. That is the EQUAL half of the
        // lexicographic order. It is recorded only when the discriminant really is the
        // parameter, the case binds every field, and no field shadowed the parameter.
        if disc_type.is_some()
            && smaller_vars.len() == param_count
            && type_args.len() >= num_type_params
            && extended_ctx.columns.get(column).is_some_and(|c| c.live)
        {
            // A constructor is applied to the inductive's PARAMETERS and then its fields; the
            // INDICES of the result are computed from those, never passed. So the rebuild reads
            // the parameter prefix of the discriminant's type spine and stops there.
            let mut rebuilt = Term::Global(ctor_name.to_string());
            for a in &type_args[..num_type_params] {
                rebuilt = Term::App(Box::new(rebuilt), Box::new(a.clone()));
            }
            for f in &smaller_vars {
                rebuilt = Term::App(Box::new(rebuilt), Box::new(Term::Var(f.clone())));
            }
            extended_ctx.rebuilt.insert(column, rebuilt);
        } else {
            extended_ctx.rebuilt.remove(&column);
        }

        // The peeled field binders carry DOMAIN annotations of their own, and a recursive
        // occurrence hiding in one of them would never be visited by the `Lambda` arm.
        let mut cur = case;
        for _ in 0..smaller_vars.len() {
            if let Term::Lambda {
                param_type, body, ..
            } = cur
            {
                check_guarded(ctx, guard_ctx, param_type)?;
                cur = body;
            }
        }

        check_guarded(ctx, &extended_ctx, case_body)?;
    }

    Ok(())
}

/// The inductive each of a constructor's VALUE FIELDS is typed by, in order, skipping the
/// inductive's `num_type_params` leading parameter binders. An empty string where the field's
/// type does not head an inductive (a function, a sort, a variable) — such a field is still
/// marked structurally smaller, but nothing can be matched on it.
fn field_type_heads(ctx: &Context, ctor_type: &Term, num_type_params: usize) -> Vec<String> {
    let mut heads = Vec::new();
    let mut cur = ctor_type;
    let mut i = 0usize;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        if i >= num_type_params {
            let mut head = param_type.as_ref();
            while let Term::App(f, _) = head {
                head = f;
            }
            heads.push(match head {
                Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => n.clone(),
                _ => String::new(),
            });
        }
        cur = body_type;
        i += 1;
    }
    heads
}

/// Count the number of Pi parameters in a type.
fn count_pi_params(ty: &Term) -> usize {
    match ty {
        Term::Pi { body_type, .. } => 1 + count_pi_params(body_type),
        _ => 0,
    }
}

/// Extract lambda parameters and return (param_names, body).
fn extract_lambda_params(term: &Term, count: usize) -> (Vec<String>, &Term) {
    if count == 0 {
        return (Vec::new(), term);
    }

    match term {
        Term::Lambda { param, body, .. } => {
            let (mut params, final_body) = extract_lambda_params(body, count - 1);
            params.insert(0, param.clone());
            (params, final_body)
        }
        _ => (Vec::new(), term),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/termination/tests.rs"]
mod tests;
