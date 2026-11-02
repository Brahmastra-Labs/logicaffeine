//! The equation compiler: surface patterns → the kernel's positional `Match` tree.
//!
//! The kernel's `Term::Match` is deliberately primitive. Its `cases` vector is indexed by
//! CONSTRUCTOR, in the inductive's registration order, one case per constructor, each case a
//! λ-telescope over that constructor's value fields. It has no patterns, no nesting, no
//! wildcard and no notion of order among alternatives. That is the right primitive — every
//! extra feature in an eliminator is another thing the trusted base has to be right about —
//! but it is not how mathematics is written. Lean writes
//!
//! ```text
//! def f : Nat → Nat → Nat
//!   | 0,      _      => 1
//!   | _,      0      => 2
//!   | .succ _, .succ _ => 3
//! ```
//!
//! and compiles it. This module is that compiler: Maranget/Augustsson column-based
//! pattern-matrix compilation, with Lean's top-to-bottom first-match semantics, producing a
//! tree of positional `Match` nodes the kernel already certifies.
//!
//! # What it guarantees
//!
//! * **Registration order, never source order.** A clause is placed in the slot of the
//!   constructor it NAMES. `Bool` is registered `true` then `false`; writing the `false` clause
//!   first must not silently compute the other function. For every equal-arity inductive —
//!   every enumeration — this is the difference between a correct program and a wrong one that
//!   type-checks.
//! * **First match wins.** Overlapping equations are split into disjoint constructor cases by
//!   specialising the matrix top-to-bottom, which is the only reading under which `pick 0 0`
//!   is `1` rather than `2`.
//! * **Totality.** A constructor no clause can reach is an error naming the missing case, not a
//!   silently-invented value and not a partial function. The kernel is total; so is its surface —
//!   and so is Lean's: `def classify : Nat → Nat | 0 => 0 | Nat.succ (Nat.succ k) => k` answers
//!   `error: missing cases: (Nat.succ Nat.zero)` (`Lean/Elab/Match.lean:991-992`).
//! * **Numerals.** `When 0:` is the unary spelling of `0` in whatever inductive the column has —
//!   Lean's `expandNatValuePattern` (`Lean/Meta/Match/Match.lean:629`), except that the zero and
//!   the successor are derived from the declaration instead of hard-coded to `Nat`.
//! * **Nothing is relaxed at the kernel.** This is a SURFACE pass. It emits exactly the
//!   `Term::Match` the kernel already accepts, one case per constructor at every node. A
//!   hand-written match with the wrong case count is still `WrongNumberOfCases`.
//!
//! # What it deliberately does not do
//!
//! The compiled tree is non-dependent: every node's motive is the CONSTANT `λ_:I. T` at the
//! same result type `T`. That is what a surface equation set means (`f : A → B`, one result
//! type for every branch). A genuinely dependent elimination is written with an explicit motive
//! against `Term::Match` directly, which this module never touches.

use super::spelled_names;
use crate::type_checker::substitute;
use crate::{
    infer_type, is_subtype, normalize, surface_elaborate_against, Context, KernelError,
    KernelResult, Term,
};
use std::collections::HashSet;

/// The binder prefix for a field the compiler introduces.
///
/// The prefix alone does NOT make the name fresh. `«#fld0»` is a `«quoted name»`, which
/// `scan_name` accepts everywhere an identifier is read, so a clause binder can be spelled
/// exactly like one of these — and the variable and catch-all rules bind a pattern variable by
/// SUBSTITUTING it, column by column, so a collision makes one column's substitution rewrite
/// what the previous column's had just introduced. `When Pair a «#fld0»: Yield a.` returned
/// the SECOND field: a well-typed term, certified by both kernels, denoting a different
/// function. Freshness is therefore CHECKED, against every name the clause set and the
/// constructor telescopes can spell — see [`State::next_name`].
const FIELD_MARK: &str = "#fld";

/// A surface pattern, exactly as a `When` clause spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// `_` — matches anything, binds nothing.
    Wildcard,
    /// `k` — matches anything, binds the name to the whole matched value.
    Var(String),
    /// `Succ (Succ k)` — a constructor applied to one sub-pattern per VALUE field.
    Ctor { name: String, args: Vec<Pattern> },
    /// `0`, `2`, `17` — a NUMERAL, which is the unary spelling of that number in the inductive
    /// the column has. Lean's `| 0 => … | 1 => …` (`Lean/Meta/Match/Match.lean:629`
    /// `expandNatValuePattern`, reached through `isNatValueTransition` at :171) expands a `Nat`
    /// literal pattern into `Nat.zero`/`Nat.succ` applications before the matrix ever sees it,
    /// and so does this compiler — see `peano_shape`, which derives the pair from the
    /// declaration rather than hard-coding `Nat`, so a user's own unary numeral type works too.
    Numeral(u64),
}

impl Pattern {
    /// Every name this pattern binds, left to right.
    pub fn binders(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_binders(&mut out);
        out
    }

    fn collect_binders(&self, out: &mut Vec<String>) {
        match self {
            Pattern::Wildcard | Pattern::Numeral(_) => {}
            Pattern::Var(n) => out.push(n.clone()),
            Pattern::Ctor { args, .. } => {
                for a in args {
                    a.collect_binders(out);
                }
            }
        }
    }

    /// How this pattern reads back, for a diagnostic.
    pub fn render(&self) -> String {
        match self {
            Pattern::Wildcard => "_".to_string(),
            Pattern::Numeral(n) => n.to_string(),
            Pattern::Var(n) => n.clone(),
            Pattern::Ctor { name, args } if args.is_empty() => name.clone(),
            Pattern::Ctor { name, args } => {
                let rendered: Vec<String> = args
                    .iter()
                    .map(|a| match a {
                        Pattern::Ctor { args, .. } if !args.is_empty() => {
                            format!("({})", a.render())
                        }
                        _ => a.render(),
                    })
                    .collect();
                format!("{name} {}", rendered.join(" "))
            }
        }
    }
}

/// One equation: one pattern per column, and the body those patterns' variables are free in.
#[derive(Clone, Debug)]
pub struct Clause {
    pub patterns: Vec<Pattern>,
    pub body: Term,
}

/// What the compiler noticed on the way through, beyond the term itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileReport {
    /// Clause indices no input can ever select, because an earlier clause already covers
    /// everything they do. First-match semantics makes them dead rather than wrong, so this is
    /// information for the surface to surface.
    ///
    /// ★ WHAT LEAN DOES WITH THE SAME FACT, and it is not what the previous sentence here
    /// claimed. Lean does NOT quietly accept a redundant alternative: `Lean/Elab/Match.lean:999`
    /// calls `logError "redundant alternative"` for every index in `result.unusedAltIdxs`, and
    /// the run on this box confirms it — `def h : Hue → Bool | .ruby => true | .jade => false |
    /// _ => true` answers `error: redundant alternative`. The error is non-fatal (`#eval h
    /// Hue.ruby` still prints `true`, so the declaration is added), and
    /// `Lean/Elab/Match.lean:985-988` declares the option that turns it off:
    /// `match.ignoreUnusedAlts`, default `false`.
    ///
    /// So the MEANING is the same in both compilers and only the default severity differs: LOGOS
    /// behaves as Lean does under `set_option match.ignoreUnusedAlts true`. What is still missing
    /// is the report reaching a reader — `literate_parser::parse_consider` discards this census,
    /// so a dead clause is silent rather than merely non-fatal. That needs a warning channel on
    /// `Command`, which this module does not own.
    pub unreachable_clauses: Vec<usize>,
}

/// Compile a clause matrix against a list of scrutinees.
///
/// `result_type` is the match's motive at every node: a raw type, which the kernel wraps as the
/// constant motive `λ_:I. result_type`.
///
/// Returns the compiled term. Use [`compile_clauses_reporting`] when the caller wants the
/// unreachable-clause census as well.
pub fn compile_clauses(
    ctx: &Context,
    scrutinees: &[Term],
    result_type: &Term,
    clauses: &[Clause],
) -> KernelResult<Term> {
    compile_clauses_reporting(ctx, scrutinees, result_type, clauses).map(|(t, _)| t)
}

/// [`compile_clauses`], plus what the compiler noticed.
pub fn compile_clauses_reporting(
    ctx: &Context,
    scrutinees: &[Term],
    result_type: &Term,
    clauses: &[Clause],
) -> KernelResult<(Term, CompileReport)> {
    if clauses.is_empty() {
        return Err(KernelError::CertificationError(
            "a pattern match needs at least one clause".to_string(),
        ));
    }
    for (i, c) in clauses.iter().enumerate() {
        if c.patterns.len() != scrutinees.len() {
            return Err(KernelError::CertificationError(format!(
                "clause {} has {} pattern(s) but the match has {} scrutinee(s) — every equation \
                 must cover the same columns",
                i + 1,
                c.patterns.len(),
                scrutinees.len()
            )));
        }
        // A name bound TWICE in one clause is refused, as Lean refuses it. It is not a harmless
        // shadow: each field is bound by SUBSTITUTING the clause's binder, so the second
        // occurrence would find nothing left to replace and the body would silently read the
        // FIRST field for both — a well-typed term denoting a different function. (Writing
        // `When Cons x x:` to mean "the two fields are equal" needs a decidable equality and a
        // guard; that is a different feature, not this one by accident.)
        let mut seen: Vec<String> = Vec::new();
        for pat in &c.patterns {
            for b in pat.binders() {
                if seen.contains(&b) {
                    return Err(KernelError::CertificationError(format!(
                        "clause {} binds `{b}` more than once. A pattern variable names ONE \
                         field; repeating it would silently read the first field for both.",
                        i + 1
                    )));
                }
                seen.push(b);
            }
        }
    }

    let rows: Vec<Row> = clauses
        .iter()
        .enumerate()
        .map(|(origin, c)| Row {
            pats: c.patterns.clone(),
            body: c.body.clone(),
            origin,
        })
        .collect();

    let mut taken: HashSet<String> = HashSet::new();
    for s in scrutinees {
        spelled_names(s, &mut taken);
    }
    spelled_names(result_type, &mut taken);
    for c in clauses {
        for pat in &c.patterns {
            taken.extend(pat.binders());
        }
        spelled_names(&c.body, &mut taken);
    }

    let mut state = State {
        fresh: 0,
        reached: vec![false; clauses.len()],
        taken,
    };
    let term = compile(ctx, scrutinees, result_type, &rows, &[], &mut state)?;
    let report = CompileReport {
        unreachable_clauses: state
            .reached
            .iter()
            .enumerate()
            .filter(|(_, r)| !**r)
            .map(|(i, _)| i)
            .collect(),
    };
    Ok((term, report))
}

/// The types a pattern's binders take when it is matched against a value of `disc_ty`.
///
/// This is the same constructor-telescope walk the compiler itself does, exposed so a parser
/// can put the binders in scope with their real types BEFORE it parses the clause body. Two
/// computations of one fact is how a compiler and its surface drift apart; there is one here.
pub fn pattern_binder_types(
    ctx: &Context,
    disc_ty: &Term,
    pattern: &Pattern,
) -> KernelResult<Vec<(String, Term)>> {
    let mut out = Vec::new();
    walk_binder_types(ctx, disc_ty, pattern, &mut out)?;
    Ok(out)
}

fn walk_binder_types(
    ctx: &Context,
    disc_ty: &Term,
    pattern: &Pattern,
    out: &mut Vec<(String, Term)>,
) -> KernelResult<()> {
    match pattern {
        // A numeral names a closed value, so like `_` it introduces no binder.
        Pattern::Wildcard | Pattern::Numeral(_) => Ok(()),
        Pattern::Var(n) => {
            out.push((n.clone(), disc_ty.clone()));
            Ok(())
        }
        Pattern::Ctor { name, args } => {
            let ty = normalize(ctx, disc_ty);
            let (ind, type_args) = split_type_app(&ty).ok_or_else(|| {
                KernelError::NotAnInductive(format!(
                    "`{}` cannot be matched against the constructor pattern `{}`",
                    ty,
                    pattern.render()
                ))
            })?;
            let ctor_ty = resolve_constructor(ctx, &ind, name, pattern)?;
            let params = &type_args[..ctx.inductive_num_params(&ind).min(type_args.len())];
            let fields = constructor_field_types(&ctor_ty, params, &[]);
            if fields.len() != args.len() {
                return Err(arity_error(name, fields.len(), args.len()));
            }
            // A DEPENDENT constructor's later field types mention its earlier fields. Each
            // earlier field's value is recoverable from the sub-pattern that matched it
            // (`pattern_to_term`); a `_` names nothing, so the dependency is left standing —
            // scope types are the parser's convenience, and `compile` is the authority.
            let mut bound: Vec<(String, Term)> = Vec::new();
            for (i, sub) in args.iter().enumerate() {
                let (orig, mut fty) = fields[i].clone();
                for (prev, value) in &bound {
                    fty = substitute(&fty, prev, value);
                }
                walk_binder_types(ctx, &fty, sub, out)?;
                if let Some(value) = pattern_to_term(sub) {
                    bound.push((orig, value));
                }
                let _ = i;
            }
            Ok(())
        }
    }
}

/// A clause's right-hand side, read at the result type the equations declare.
///
/// This is the stage that first knows "this body must have type R", in the scope the body is
/// actually written in — `ctx` here carries the case's field binders. The kernel's `Match` rule
/// elaborates its cases against nothing, because a case is a λ-telescope whose expected type it
/// would have to reconstruct from the constructor; the compiler already has the result type at
/// every leaf. In Lean's terms this is `OfNat` and friends at the expected type: `| true => 1`
/// in a function declared `→ Cel` is the `Cel` whose field is `1`.
///
/// Best-effort by design, in two directions, and both matter:
///
/// * a body that ALREADY fits is returned untouched, so nothing that type-checks today is
///   re-shaped, and the common case costs one `infer_type` rather than a full elaboration;
/// * a body elaboration cannot reconcile — most often one naming the function being defined,
///   whose `fix` binder is tied after compilation — is handed on UNCHANGED, so the mismatch is
///   reported by the kernel against the real expected type. Elaboration may CHOOSE a term; it may
///   never change what one means.
fn elaborate_leaf(ctx: &Context, result_type: &Term, body: &Term) -> Term {
    if matches!(infer_type(ctx, body), Ok(native) if is_subtype(ctx, &native, result_type)) {
        return body.clone();
    }
    surface_elaborate_against(ctx, body, Some(result_type)).unwrap_or_else(|_| body.clone())
}

/// The VALUE a pattern denotes, when it denotes one: a variable is itself, a constructor
/// pattern is its constructor applied to its sub-patterns' values, and `_` denotes nothing.
fn pattern_to_term(pattern: &Pattern) -> Option<Term> {
    match pattern {
        // A numeral's value is known but its SPELLING is not — the constructors it expands into
        // belong to the column's inductive, which this walk does not carry. `compile` expands it
        // before any case is built; here the dependency is simply left standing, which is what
        // `None` already means for `_`.
        Pattern::Wildcard | Pattern::Numeral(_) => None,
        Pattern::Var(n) => Some(Term::Var(n.clone())),
        Pattern::Ctor { name, args } => {
            let mut out = Term::Global(name.clone());
            for a in args {
                out = Term::App(Box::new(out), Box::new(pattern_to_term(a)?));
            }
            Some(out)
        }
    }
}

// ============================================================
// THE MATRIX
// ============================================================

#[derive(Clone, Debug)]
struct Row {
    pats: Vec<Pattern>,
    body: Term,
    origin: usize,
}

struct State {
    fresh: usize,
    reached: Vec<bool>,
    /// Every name the input can spell: the clause set's binders and bodies, the scrutinees, the
    /// result type, and — added as each node resolves its inductive — the constructor
    /// telescopes' own parameter names. A generated binder is stepped until it is outside this
    /// set, which is what makes the substitutions capture-free by CHECK rather than by an
    /// assumption about the lexer.
    taken: HashSet<String>,
}

impl State {
    fn next_name(&mut self) -> String {
        loop {
            let candidate = format!("{FIELD_MARK}{}", self.fresh);
            self.fresh += 1;
            if !self.taken.contains(&candidate) {
                return candidate;
            }
        }
    }
}

/// One step of the constructor path taken to reach a leaf, for the missing-case diagnostic.
#[derive(Clone)]
struct Step {
    ctor: String,
    arity: usize,
}

fn compile(
    ctx: &Context,
    cols: &[Term],
    result_type: &Term,
    rows: &[Row],
    trail: &[Step],
    state: &mut State,
) -> KernelResult<Term> {
    // No row survives: nothing in the clause set covers this shape. A total language cannot
    // admit a partial function, so this is where Lean's "missing cases" error lives.
    let Some(first) = rows.first() else {
        return Err(KernelError::CertificationError(format!(
            "this definition is not exhaustive: no equation covers `{}`. A LOGOS definition is a \
             TOTAL function, so every constructor of the type being matched needs a clause — add \
             one, or write `When _:` for the rest.",
            render_trail(trail)
        )));
    };

    // Every column consumed: first match wins, and this row is the first.
    if cols.is_empty() {
        state.reached[first.origin] = true;
        return Ok(elaborate_leaf(ctx, result_type, &first.body));
    }

    // The VARIABLE rule: no row inspects this column's constructor, so the column is just a
    // binding. Bind it and move on — splitting here would be sound but would fan the tree out
    // over a distinction no equation makes.
    // A NUMERAL inspects its column exactly as a constructor pattern does — it IS one, once the
    // inductive is known — so it must not fall through to the variable rule.
    if rows
        .iter()
        .all(|r| !matches!(r.pats[0], Pattern::Ctor { .. } | Pattern::Numeral(_)))
    {
        let bound: Vec<Row> = rows
            .iter()
            .map(|r| {
                let body = match &r.pats[0] {
                    Pattern::Var(v) => substitute(&r.body, v, &cols[0]),
                    _ => r.body.clone(),
                };
                Row {
                    pats: r.pats[1..].to_vec(),
                    body,
                    origin: r.origin,
                }
            })
            .collect();
        return compile(ctx, &cols[1..], result_type, &bound, trail, state);
    }

    // The CONSTRUCTOR rule: split this column over the inductive's constructors, in the
    // inductive's OWN order.
    let scrutinee = &cols[0];
    let disc_ty = normalize(ctx, &infer_type(ctx, scrutinee)?);
    let (ind, type_args) = split_type_app(&disc_ty).ok_or_else(|| {
        KernelError::NotAnInductive(format!(
            "`{disc_ty}` is not an inductive type, so it cannot be matched on"
        ))
    })?;
    let constructors: Vec<(String, Term)> = ctx
        .get_constructors(&ind)
        .into_iter()
        .map(|(n, t)| (n.to_string(), t.clone()))
        .collect();
    if constructors.is_empty() {
        return Err(KernelError::NotAnInductive(format!(
            "`{ind}` has no constructors, so there is nothing to match"
        )));
    }

    // An INDEXED discriminant is refused rather than approximated. The kernel types an indexed
    // match by a different rule (`infer_indexed_match`): the motive abstracts the indices as
    // well as the scrutinee, and coverage is over the constructors the indices still ALLOW, not
    // over all of them. A clause list carries no motive and cannot say which indices it means,
    // so compiling one into a plain positional match would emit a tree whose case count is right
    // by luck. Say so instead — an indexed elimination is written against `Term::Match` with the
    // motive supplied.
    let num_params = ctx.inductive_num_params(&ind);
    if type_args.len() > num_params {
        return Err(KernelError::CertificationError(format!(
            "`{ind}` is an INDEXED inductive ({} parameter(s), {} index argument(s) here). An \
             indexed elimination needs an explicit motive over its indices, which a clause list \
             cannot express — write the `match` directly with its `return` clause.",
            num_params,
            type_args.len() - num_params
        )));
    }

    // NUMERALS BECOME CONSTRUCTOR PATTERNS HERE, where the column's inductive is finally known —
    // Lean's `expandNatValuePattern` (`Lean/Meta/Match/Match.lean:629`), at the same point in the
    // same algorithm. Everything below this line sees only `Ctor`/`Var`/`Wildcard`.
    let expanded: Vec<Row> = rows
        .iter()
        .map(|r| match &r.pats[0] {
            Pattern::Numeral(n) => {
                let head = numeral_pattern(ctx, &ind, *n)?;
                let mut pats = vec![head];
                pats.extend_from_slice(&r.pats[1..]);
                Ok(Row {
                    pats,
                    body: r.body.clone(),
                    origin: r.origin,
                })
            }
            _ => Ok(r.clone()),
        })
        .collect::<KernelResult<Vec<Row>>>()?;
    let rows: &[Row] = &expanded;

    // Reject a clause that names something that is not a constructor of THIS inductive before
    // any slotting happens — mis-slotting a foreign name is exactly the silent wrong answer the
    // positional case vector used to allow.
    for row in rows {
        if let Pattern::Ctor { name, args } = &row.pats[0] {
            let ctor_ty = resolve_constructor(ctx, &ind, name, &row.pats[0])?;
            let fields = constructor_field_types(&ctor_ty, &type_args, &[]);
            if fields.len() != args.len() {
                return Err(arity_error(name, fields.len(), args.len()));
            }
        }
    }

    // A constructor's own field names are substituted into its later field TYPES, so they are
    // names this node can spell too — seeded before any binder is generated for it.
    for (_, ctor_ty) in &constructors {
        for (field, fty) in constructor_field_types(ctor_ty, &type_args, &[]) {
            state.taken.insert(field);
            spelled_names(&fty, &mut state.taken);
        }
    }

    let mut cases = Vec::with_capacity(constructors.len());
    for (ctor_name, ctor_ty) in &constructors {
        let fields = constructor_field_types(ctor_ty, &type_args, &[]);

        // One fresh binder per value field, with its type closed into this case's scope: a
        // dependent field type naming an EARLIER field is rewritten to that field's binder.
        // The context is extended lazily so a nullary constructor — most nodes in most trees —
        // costs no clone at all.
        let mut binders: Vec<(String, Term)> = Vec::new();
        let mut extended: Option<Context> = None;
        for (i, (_, fty)) in fields.iter().enumerate() {
            let mut closed = fty.clone();
            for j in 0..i {
                closed = substitute(&closed, &fields[j].0, &Term::Var(binders[j].0.clone()));
            }
            let name = state.next_name();
            let base = extended.as_ref().unwrap_or(ctx);
            extended = Some(base.extend(&name, closed.clone()));
            binders.push((name, closed));
        }
        let inner_ctx: &Context = extended.as_ref().unwrap_or(ctx);

        // `C type_args… field_binders…` — the value this case's scrutinee is known to be, which
        // is what a variable or catch-all head pattern gets bound to.
        let mut ctor_value = Term::Global(ctor_name.clone());
        for ta in &type_args {
            ctor_value = Term::App(Box::new(ctor_value), Box::new(ta.clone()));
        }
        for (name, _) in &binders {
            ctor_value = Term::App(Box::new(ctor_value), Box::new(Term::Var(name.clone())));
        }

        // Specialise: keep the rows this constructor can still reach, in their original order.
        let mut specialised: Vec<Row> = Vec::new();
        for row in rows {
            match &row.pats[0] {
                Pattern::Ctor { name, args } if name == ctor_name => {
                    let mut pats = args.clone();
                    pats.extend_from_slice(&row.pats[1..]);
                    specialised.push(Row { pats, body: row.body.clone(), origin: row.origin });
                }
                Pattern::Ctor { .. } => {}
                Pattern::Wildcard => {
                    let mut pats = vec![Pattern::Wildcard; binders.len()];
                    pats.extend_from_slice(&row.pats[1..]);
                    specialised.push(Row { pats, body: row.body.clone(), origin: row.origin });
                }
                Pattern::Var(v) => {
                    let mut pats = vec![Pattern::Wildcard; binders.len()];
                    pats.extend_from_slice(&row.pats[1..]);
                    specialised.push(Row {
                        pats,
                        body: substitute(&row.body, v, &ctor_value),
                        origin: row.origin,
                    });
                }
                // Unreachable by construction — the head column's numerals were rewritten into
                // constructor patterns the moment `ind` was known, a few lines above. Reported
                // rather than `unreachable!()`d: a kernel surface answers, it does not abort, and
                // a future edit that moves the expansion gets a sentence instead of a panic.
                Pattern::Numeral(n) => {
                    return Err(KernelError::CertificationError(format!(
                        "the numeral pattern `{n}` reached case specialisation for `{ind}`                          without being expanded into its unary spelling"
                    )))
                }
            }
        }

        let mut sub_cols: Vec<Term> = binders.iter().map(|(n, _)| Term::Var(n.clone())).collect();
        sub_cols.extend_from_slice(&cols[1..]);

        let mut sub_trail = trail.to_vec();
        sub_trail.push(Step {
            ctor: ctor_name.clone(),
            arity: binders.len(),
        });

        let mut case = compile(
            inner_ctx,
            &sub_cols,
            result_type,
            &specialised,
            &sub_trail,
            state,
        )?;
        for (name, ty) in binders.iter().rev() {
            case = Term::Lambda {
                param: name.clone(),
                param_type: Box::new(ty.clone()),
                body: Box::new(case),
            };
        }
        cases.push(case);
    }

    Ok(Term::Match {
        discriminant: Box::new(scrutinee.clone()),
        motive: Box::new(constant_motive(disc_ty, result_type, state)),
        cases,
    })
}

/// The CANONICAL motive for a non-dependent node: `λ_:I. T`, never the bare `T`.
///
/// The kernel's `Match` rule accepts both — it wraps a `Sort`-typed motive as this very
/// constant before it checks the branches — so from inside the kernel the two are the same
/// term. They are not the same DECLARATION. Only the λ records which inductive the node
/// eliminates, and that fact is unrecoverable from a stored tree: the discriminant a compiled
/// node splits on is a `Var`, whose type needs the local telescope that a consumer walking the
/// term does not have. Program extraction reads the motive to enumerate the constructors, and a
/// bare result type left it with none — an arm-less `match n { }` the kernel accepts and no
/// backend can emit. The compiler holds `disc_ty` at every node already (it cannot split the
/// column without it), so the richer form costs nothing and is stated once, here, rather than
/// re-derived differently by each consumer.
///
/// The binder is `_` — the spelling [`crate::elaborate::infer_match_motive`] gives the same
/// motive — stepped to a generated name in the one case where the result type has a free
/// variable actually called `_`, so the constant can never capture.
fn constant_motive(disc_ty: Term, result_type: &Term, state: &mut State) -> Term {
    let param = if crate::type_checker::occurs_free(result_type, "_") {
        state.next_name()
    } else {
        "_".to_string()
    };
    Term::Lambda {
        param,
        param_type: Box::new(disc_ty),
        body: Box::new(result_type.clone()),
    }
}

// ============================================================
// CONSTRUCTOR SHAPE
// ============================================================

/// Split a type application into its head inductive's name and its arguments.
/// `List A` → `("List", [A])`; `Nat` → `("Nat", [])`.
fn split_type_app(ty: &Term) -> Option<(String, Vec<Term>)> {
    let mut args = Vec::new();
    let mut cur = ty;
    while let Term::App(f, a) = cur {
        args.push((**a).clone());
        cur = f;
    }
    args.reverse();
    match cur {
        Term::Global(n) => Some((n.clone(), args)),
        _ => None,
    }
}

/// A constructor's VALUE fields: its `Π` telescope past the inductive's parameters, with the
/// parameters instantiated by the discriminant's own type arguments.
///
/// This mirrors `type_checker::compute_case_type`, which is what the kernel will check the
/// emitted case against. `extra` lets a caller pre-substitute names already bound.
fn constructor_field_types(
    ctor_ty: &Term,
    type_args: &[Term],
    extra: &[(String, Term)],
) -> Vec<(String, Term)> {
    let mut all: Vec<(String, Term)> = Vec::new();
    let mut cur = ctor_ty;
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        all.push((param.clone(), (**param_type).clone()));
        cur = body_type;
    }
    let split = type_args.len().min(all.len());
    let params: Vec<String> = all[..split].iter().map(|(n, _)| n.clone()).collect();
    all[split..]
        .iter()
        .map(|(n, t)| {
            let mut ty = t.clone();
            for (p, a) in params.iter().zip(type_args.iter()) {
                if p != "_" {
                    ty = substitute(&ty, p, a);
                }
            }
            for (n2, t2) in extra {
                ty = substitute(&ty, n2, t2);
            }
            (n.clone(), ty)
        })
        .collect()
}

/// The unary spelling of `n` in `ind` — `2` over `Nat` is `Succ (Succ Zero)`.
fn numeral_pattern(ctx: &Context, ind: &str, n: u64) -> KernelResult<Pattern> {
    let (zero, succ) = peano_shape(ctx, ind)?;
    let mut out = Pattern::Ctor {
        name: zero,
        args: Vec::new(),
    };
    for _ in 0..n {
        out = Pattern::Ctor {
            name: succ.clone(),
            args: vec![out],
        };
    }
    Ok(out)
}

/// The two constructors a numeral needs: the one with NO value fields, and the one with exactly
/// ONE field of the inductive's own type.
///
/// Lean hard-codes `Nat.zero`/`Nat.succ` (`Lean/Meta/Match/Match.lean:629`). Deriving the pair
/// from the DECLARATION costs nothing and is strictly more: a user who writes
/// `A Tally is either Nought or a Tick with prev: Tally.` gets numeral patterns for free, and a
/// type that is not unary-numeral-shaped is told so by name instead of silently mis-expanding
/// into constructors that happen to be called `Zero` and `Succ`.
fn peano_shape(ctx: &Context, ind: &str) -> KernelResult<(String, String)> {
    let mut zero: Option<String> = None;
    let mut succ: Option<String> = None;
    let ctors = ctx.get_constructors(ind);
    let shape_error = |why: &str| {
        KernelError::CertificationError(format!(
            "a numeral pattern is the unary spelling of a number, and `{ind}` has no unary              numeral form: {why}. Write the constructors out, or match on a type with one              field-free constructor and one that carries a single `{ind}`."
        ))
    };
    for (name, ty) in &ctors {
        let fields = constructor_field_types(ty, &[], &[]);
        match fields.len() {
            0 if zero.replace((*name).to_string()).is_none() => {}
            1 if matches!(&fields[0].1, Term::Global(g) if g == ind)
                && succ.replace((*name).to_string()).is_none() => {}
            _ => {
                return Err(shape_error(&format!(
                    "`{name}` takes {} field(s) and cannot be a zero or a successor",
                    fields.len()
                )))
            }
        }
    }
    match (zero, succ) {
        (Some(z), Some(s)) => Ok((z, s)),
        (None, _) => Err(shape_error(
            "it has no field-free constructor to be the zero",
        )),
        (_, None) => Err(shape_error(
            "it has no constructor carrying a single value of itself",
        )),
    }
}

fn resolve_constructor(
    ctx: &Context,
    ind: &str,
    name: &str,
    pattern: &Pattern,
) -> KernelResult<Term> {
    match ctx.constructor_inductive(name) {
        Some(owner) if owner == ind => Ok(ctx
            .get_global(name)
            .cloned()
            .expect("a registered constructor always has a type")),
        Some(owner) => Err(KernelError::CertificationError(format!(
            "`{name}` is a constructor of `{owner}`, not of `{ind}` — the pattern `{}` cannot \
             match a value of `{ind}`",
            pattern.render()
        ))),
        None => Err(KernelError::CertificationError(format!(
            "`{name}` is not a constructor of `{ind}`. Its constructors are: {}.",
            ctx.get_constructors(ind)
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn arity_error(name: &str, expected: usize, found: usize) -> KernelError {
    KernelError::CertificationError(format!(
        "the constructor `{name}` takes {expected} field(s), but the pattern gives it {found}"
    ))
}

/// The shape no clause covered, as a pattern a reader can act on.
fn render_trail(trail: &[Step]) -> String {
    let mut acc: Option<String> = None;
    for step in trail.iter().rev() {
        let mut parts: Vec<String> = Vec::new();
        for i in 0..step.arity {
            match (i, &acc) {
                (0, Some(inner)) if inner.contains(' ') => parts.push(format!("({inner})")),
                (0, Some(inner)) => parts.push(inner.clone()),
                _ => parts.push("_".to_string()),
            }
        }
        acc = Some(if parts.is_empty() {
            step.ctor.clone()
        } else {
            format!("{} {}", step.ctor, parts.join(" "))
        });
    }
    acc.unwrap_or_else(|| "_".to_string())
}
