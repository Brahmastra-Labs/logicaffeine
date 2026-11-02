//! Division and modulus on naturals, accelerated from a checked well-founded recurrence.
//!
//! Lean's `Nat.div` and `Nat.modCore` are well-founded definitions: `u (x, y)` for a unary program
//! `u := WellFounded.fix … F` on pairs of naturals. The kernel cannot unroll them cheaply — the
//! unrolling runs `Acc.rec` on a proof, and the recursion subtracts one divisor per step — so Lean's
//! kernel computes them natively, by name. Here no name grants anything. A unary program `u` on a
//! pair type is computed natively only after the kernel proves its ONE-STEP UNFOLDING and that the
//! unfolding is exactly the division or the modulus recurrence:
//!
//! * modulus: `u (x, y) ≡ ite (0 < y ∧ y ≤ x) (u (x - y, y)) x`;
//! * quotient: `u (x, y) ≡ ite (0 < y ∧ y ≤ x) (u (x - y, y) + 1) 0`;
//!
//! for fresh `x y : Nat`, where every piece is itself checked, not named: `ite` is any program the
//! kernel proves to select its first branch on `isTrue` and its second on `isFalse` of a two-constructor
//! `Decidable` family; the guard is the standard conjunction of the standard `Nat.le` (their
//! inductive declarations are checked constructor by constructor); `x - y` reduces to a program the
//! arithmetic certificate proves is truncated subtraction; `+ 1` is checked by conversion.
//!
//! The unfolding itself: `u (x, y)` is reduced at the head — β, ζ, δ, ι — until a structural
//! recursion or a `match` is stuck on a PROOF of a one-constructor proposition (`Acc r p`, not
//! reducible to `Acc.intro`). That proof is replaced by the constructor applied to canonical
//! fields, `Acc.intro p (λ y hy. a[p := y])` for the proof `a p`, after the kernel checks that the
//! replacement is a proof of the same proposition. By proof irrelevance the two terms are
//! definitionally equal, so every step is one of the kernel's rules, and reduction continues until
//! the selector appears.
//!
//! Why the native answer is the program's value on closed operands (canonicity): for closed
//! `x₀ y₀` the guard's decision is a closed term of `Decidable P₀`; it reduces to `isTrue` exactly
//! when `P₀` holds, which for the standard `And` and `Nat.le` is `1 ≤ y₀ ∧ y₀ ≤ x₀`. So `u (x₀, y₀)`
//! is `u (x₀ - y₀, y₀)` (plus one for the quotient) while `1 ≤ y₀ ≤ x₀`, and `x₀` (or `0`) after:
//! the modulus is `x₀ mod y₀` (and `x₀` when `y₀ = 0`), the quotient `x₀ div y₀` (and `0` when
//! `y₀ = 0`), by induction on `x₀`. Open operands are never accelerated: the evaluator computes
//! only on observed natural numbers.
use super::*;
use crate::reduction::whnf_core::{delta_step, whnf, whnf_core, Allowance};
use crate::type_checker::def_eq;

/// A certified division program: the operation, the pair constructor and its parameter count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Division {
    pub(super) op: Arithmetic,
    pub(super) pair: String,
    pub(super) parameters: usize,
}

const X: &str = "division.certificate.x";
const Y: &str = "division.certificate.y";
const STEPS: usize = 1 << 12;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apply(head: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(head, app)
}

fn spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut head = t;
    let mut args = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    (head, args)
}

fn constant_name(t: &Term) -> Option<&str> {
    match t {
        Term::Global(name) | Term::Const { name, .. } => Some(name),
        _ => None,
    }
}

/// The same constant at another name (same universe levels, if any).
fn sibling(t: &Term, name: &str) -> Term {
    match t {
        Term::Const { levels, .. } => Term::Const {
            name: name.into(),
            levels: levels.clone(),
        },
        _ => g(name),
    }
}

/// `ty` with its leading `Π`s instantiated at `args`, in order.
fn instantiate(ty: &Term, args: &[Term]) -> Option<Term> {
    let mut ty = ty.clone();
    for arg in args {
        let Term::Pi {
            param, body_type, ..
        } = ty
        else {
            return None;
        };
        ty = crate::substitute(&body_type, &param, arg);
    }
    Some(ty)
}

/// The declared type of a constant as a `Term` at its use's universe levels.
fn type_of_constant(ctx: &Context, head: &Term) -> Option<Term> {
    crate::infer_type(ctx, head).ok()
}

/// A fresh local name for `base` in `ctx`.
fn fresh(ctx: &Context, base: &str) -> String {
    let mut name = base.to_owned();
    while ctx.get(&name).is_some() || ctx.get_global(&name).is_some() {
        name.push('\'');
    }
    name
}

/// The inductive at the head of `ty` after weak-head reduction, with its arguments.
fn inductive_view(ctx: &Context, ty: &Term) -> Option<(Term, Vec<Term>)> {
    let exposed = whnf(ctx, ty, &mut Allowance::new(STEPS)).ok()?;
    let (head, args) = spine(&exposed);
    let name = constant_name(head)?;
    ctx.is_inductive(name)
        .then(|| (head.clone(), args.into_iter().cloned().collect()))
}

/// Whether the inductive `name`'s declared type ends in `Prop`.
fn propositional(ctx: &Context, name: &str) -> bool {
    let declared = match ctx.poly_declaration(name) {
        Some((_, ty)) => ty,
        None => match ctx.get_global(name) {
            Some(ty) => ty,
            None => return false,
        },
    };
    let mut result = declared;
    while let Term::Pi { body_type, .. } = result {
        result = body_type;
    }
    matches!(result, Term::Sort(level) if level.equiv(&crate::Universe::Prop))
}

/// A proof `a` of a one-constructor proposition `I ps t` whose constructor takes the index `t`
/// and a function returning proofs of `I ps` at other indices (`Acc.intro x h`), rebuilt as that
/// constructor: `intro ps t (λ y hy. a[t := y])`, where `t` is `a`'s last argument. `None` unless
/// the kernel checks the replacement proves the same proposition.
fn canonical_proof(ctx: &Context, a: &Term) -> Option<Term> {
    let ty = crate::infer_type(ctx, a).ok()?;
    let (family, args) = inductive_view(ctx, &ty)?;
    let family_name = constant_name(&family)?;
    if !propositional(ctx, family_name) {
        return None;
    }
    let constructors = ctx.get_constructors(family_name);
    let [(constructor, _)] = constructors.as_slice() else {
        return None;
    };
    let parameters = ctx.inductive_num_params(family_name);
    if args.len() != parameters + 1 {
        return None;
    }
    let index = args[parameters].clone();
    let (proof_head, proof_args) = spine(a);
    if proof_args
        .last()
        .is_none_or(|last| !crate::type_checker::alpha_equivalent(last, &index))
    {
        return None;
    }
    let constructor = sibling(&family, constructor);
    let fields = instantiate(&type_of_constant(ctx, &constructor)?, &args[..parameters])?;
    let Term::Pi {
        param: x,
        body_type: rest,
        ..
    } = &fields
    else {
        return None;
    };
    let rest = crate::substitute(rest, x, &index);
    let Term::Pi {
        param_type: h_type, ..
    } = &rest
    else {
        return None;
    };
    let Term::Pi {
        param: y,
        param_type: y_type,
        body_type: after_y,
    } = h_type.as_ref()
    else {
        return None;
    };
    let Term::Pi {
        param_type: hy_type,
        ..
    } = after_y.as_ref()
    else {
        return None;
    };
    let (y_name, hy_name) = (
        fresh(ctx, "division.certificate.y'"),
        fresh(ctx, "division.certificate.hy"),
    );
    let hy_type = crate::substitute(hy_type, y, &v(&y_name));
    let rebuilt = apply(
        proof_head.clone(),
        &proof_args[..proof_args.len() - 1]
            .iter()
            .map(|t| (*t).clone())
            .chain(std::iter::once(v(&y_name)))
            .collect::<Vec<_>>(),
    );
    let h = Term::Lambda {
        param: y_name.clone(),
        param_type: y_type.clone(),
        body: Box::new(Term::Lambda {
            param: hy_name,
            param_type: Box::new(hy_type),
            body: Box::new(rebuilt),
        }),
    };
    let replacement = apply(
        constructor,
        &args[..parameters]
            .iter()
            .cloned()
            .chain([index, h])
            .collect::<Vec<_>>(),
    );
    let replaced_type = crate::infer_type(ctx, &replacement).ok()?;
    def_eq(ctx, &replaced_type, &ty).then_some(replacement)
}

/// Whether `head` selects `t h` on `isTrue h` and `e h` on `isFalse h` of its decision argument,
/// at the result type `alpha`: `head α c (isTrue h) t e ≡ t h`, `head α c (isFalse h) t e ≡ e h`
/// for fresh `c h t e`, where `isTrue`/`isFalse` are the constructors of the decision's
/// two-constructor family whose field is `c` and `c → ⊥` (an inductive proposition with no
/// constructor) respectively.
fn selects(ctx: &Context, head: &Term, alpha: &Term) -> bool {
    let Some(ty) = type_of_constant(ctx, head) else {
        return false;
    };
    let Some(after_alpha) = instantiate(&ty, std::slice::from_ref(alpha)) else {
        return false;
    };
    let c = fresh(ctx, "division.certificate.c");
    let ctx_c = ctx.extend(&c, Term::Sort(crate::Universe::Prop));
    let Some(Term::Pi {
        param_type: decision,
        ..
    }) = instantiate(&after_alpha, &[v(&c)])
    else {
        return false;
    };
    let Some((family, family_args)) = inductive_view(&ctx_c, &decision) else {
        return false;
    };
    let Some(family_name) = constant_name(&family) else {
        return false;
    };
    let constructors = ctx.get_constructors(family_name);
    if constructors.len() != 2 || ctx.inductive_num_params(family_name) != family_args.len() {
        return false;
    }
    let field = |constructor: &str| -> Option<Term> {
        let ty = instantiate(
            &type_of_constant(&ctx_c, &sibling(&family, constructor))?,
            &family_args,
        )?;
        match ty {
            Term::Pi { param_type, .. } => Some(*param_type),
            _ => None,
        }
    };
    let refutes = |field: &Term| -> bool {
        let Ok(exposed) = whnf(&ctx_c, field, &mut Allowance::new(STEPS)) else {
            return false;
        };
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = exposed
        else {
            return false;
        };
        def_eq(&ctx_c, &param_type, &v(&c))
            && inductive_view(&ctx_c, &body_type).is_some_and(|(empty, _)| {
                constant_name(&empty)
                    .is_some_and(|n| propositional(ctx, n) && ctx.get_constructors(n).is_empty())
            })
    };
    let (Some(first), Some(second)) = (field(constructors[0].0), field(constructors[1].0)) else {
        return false;
    };
    let (is_true, is_false, not_c) = if def_eq(&ctx_c, &first, &v(&c)) && refutes(&second) {
        (constructors[0].0, constructors[1].0, second)
    } else if def_eq(&ctx_c, &second, &v(&c)) && refutes(&first) {
        (constructors[1].0, constructors[0].0, first)
    } else {
        return false;
    };
    let arrow = |a: Term, b: Term| Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    let names = [
        "division.certificate.hp",
        "division.certificate.hn",
        "division.certificate.t",
        "division.certificate.e",
    ]
    .map(|n| fresh(&ctx_c, n));
    let ctx_all = ctx_c
        .extend(&names[0], v(&c))
        .extend(&names[1], not_c.clone())
        .extend(&names[2], arrow(v(&c), alpha.clone()))
        .extend(&names[3], arrow(not_c, alpha.clone()));
    let decide = |constructor: &str, proof: &str| {
        apply(
            sibling(&family, constructor),
            &family_args
                .iter()
                .cloned()
                .chain([v(proof)])
                .collect::<Vec<_>>(),
        )
    };
    let choose = |decision: Term| {
        apply(
            head.clone(),
            &[alpha.clone(), v(&c), decision, v(&names[2]), v(&names[3])],
        )
    };
    def_eq(
        &ctx_all,
        &choose(decide(is_true, &names[0])),
        &app(v(&names[2]), v(&names[0])),
    ) && def_eq(
        &ctx_all,
        &choose(decide(is_false, &names[1])),
        &app(v(&names[3]), v(&names[1])),
    )
}

/// The inductive `le` is the standard order on naturals: one parameter `n`, one index, and exactly
/// the constructors `refl : le n n` and `step : ∀ m, le n m → le n (m + 1)`.
fn standard_le(ctx: &Context, le: &Term) -> bool {
    let Some(name) = constant_name(le) else {
        return false;
    };
    if !propositional(ctx, name) || ctx.inductive_num_params(name) != 1 {
        return false;
    }
    let constructors = ctx.get_constructors(name);
    if constructors.len() != 2 {
        return false;
    }
    let n = fresh(ctx, "division.certificate.n");
    let ctx_n = ctx.extend(&n, g("Nat"));
    let at = |constructor: &str| {
        instantiate(
            &type_of_constant(&ctx_n, &sibling(le, constructor))?,
            &[v(&n)],
        )
    };
    let (Some(refl), Some(step)) = (at(constructors[0].0), at(constructors[1].0)) else {
        return false;
    };
    let le_of = |a: Term, b: Term| apply(le.clone(), &[a, b]);
    let m = fresh(&ctx_n, "division.certificate.m");
    let expected_step = Term::Pi {
        param: m.clone(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(Term::Pi {
            param: "_".into(),
            param_type: Box::new(le_of(v(&n), v(&m))),
            body_type: Box::new(le_of(v(&n), app(g("Succ"), v(&m)))),
        }),
    };
    def_eq(&ctx_n, &refl, &le_of(v(&n), v(&n))) && def_eq(&ctx_n, &step, &expected_step)
}

/// The inductive `and` is the standard conjunction: two proposition parameters `a b` and one
/// constructor with exactly the fields `a` and `b`.
fn standard_and(ctx: &Context, and: &Term) -> bool {
    let Some(name) = constant_name(and) else {
        return false;
    };
    if !propositional(ctx, name) || ctx.inductive_num_params(name) != 2 {
        return false;
    }
    let constructors = ctx.get_constructors(name);
    let [(constructor, _)] = constructors.as_slice() else {
        return false;
    };
    let (a, b) = (
        fresh(ctx, "division.certificate.a"),
        fresh(ctx, "division.certificate.b"),
    );
    let ctx_ab = ctx
        .extend(&a, Term::Sort(crate::Universe::Prop))
        .extend(&b, Term::Sort(crate::Universe::Prop));
    let Some(fields) = type_of_constant(&ctx_ab, &sibling(and, constructor))
        .and_then(|ty| instantiate(&ty, &[v(&a), v(&b)]))
    else {
        return false;
    };
    let expected = Term::Pi {
        param: "_".into(),
        param_type: Box::new(v(&a)),
        body_type: Box::new(Term::Pi {
            param: "_".into(),
            param_type: Box::new(v(&b)),
            body_type: Box::new(apply(and.clone(), &[v(&a), v(&b)])),
        }),
    };
    def_eq(&ctx_ab, &fields, &expected)
}

/// Whether the proposition `guard` is `0 < y ∧ y ≤ x` over the standard conjunction and order.
fn guards_division(ctx: &Context, guard: &Term) -> bool {
    let Some((and, conjuncts)) = inductive_view(ctx, guard) else {
        return false;
    };
    let [left, right] = conjuncts.as_slice() else {
        return false;
    };
    if !standard_and(ctx, &and) {
        return false;
    }
    let order = |t: &Term| -> Option<(Term, Term, Term)> {
        let (le, args) = inductive_view(ctx, t)?;
        let [a, b] = args.as_slice() else { return None };
        standard_le(ctx, &le).then(|| (le, a.clone(), b.clone()))
    };
    let (Some((le1, one, y1)), Some((le2, y2, x2))) = (order(left), order(right)) else {
        return false;
    };
    constant_name(&le1) == constant_name(&le2)
        && def_eq(ctx, &one, &app(g("Succ"), g("Zero")))
        && def_eq(ctx, &y1, &v(Y))
        && def_eq(ctx, &y2, &v(Y))
        && def_eq(ctx, &x2, &v(X))
}

/// Reduce `term` at the head — β, ζ, δ, ι, structural unrolling — replacing a stuck proof of a
/// one-constructor proposition by its canonical constructor, until the head is a selector
/// (`selects`) applied to at least five arguments. `None` when anything else gets stuck.
fn unfold_to_selector(ctx: &Context, term: &Term) -> Option<(Term, Vec<Term>)> {
    let mut term = term.clone();
    let mut selectors: HashMap<String, bool> = HashMap::new();
    for _ in 0..STEPS {
        if crate::type_checker::depth_exceeds(&term, crate::type_checker::MAX_TERM_DEPTH) {
            return None;
        }
        let (head, args) = spine(&term);
        let args: Vec<Term> = args.into_iter().cloned().collect();
        match head {
            Term::Lambda { param, body, .. } if !args.is_empty() => {
                term = apply(crate::substitute(body, param, &args[0]), &args[1..]);
            }
            Term::Let {
                name, value, body, ..
            } => {
                term = apply(crate::substitute(body, name, value), &args);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let branch = match super::super::iota_branch(ctx, discriminant, motive, cases) {
                    Some(branch) => branch,
                    None => {
                        let exposed = expose(ctx, discriminant)?;
                        let reduced = Term::Match {
                            discriminant: Box::new(exposed),
                            motive: motive.clone(),
                            cases: cases.clone(),
                        };
                        term = apply(reduced, &args);
                        continue;
                    }
                };
                term = apply(branch, &args);
            }
            Term::Fix { name, body } => {
                let column = *crate::termination::structural_columns(ctx, body).first()?;
                let major = args.get(column)?;
                if super::super::is_constructor_form(ctx, major) {
                    let unrolled = crate::substitute(body, name, head);
                    term = apply(unrolled, &args);
                } else {
                    let mut replaced = args.clone();
                    replaced[column] = expose(ctx, major)?;
                    term = apply(head.clone(), &replaced);
                }
            }
            Term::Global(name) | Term::Const { name, .. } => {
                if args.len() >= 5 {
                    let is_selector = *selectors
                        .entry(name.clone())
                        .or_insert_with(|| selects(ctx, head, &args[0]));
                    if is_selector {
                        return Some((head.clone(), args));
                    }
                }
                term = delta_step(ctx, &term)?;
            }
            _ => return None,
        }
    }
    None
}

/// One step toward exposing a constructor in a discriminant or structural argument: a stuck proof
/// of a one-constructor proposition becomes its canonical constructor; anything else takes one
/// weak-head step (β, ζ, ι at its head, or one δ).
fn expose(ctx: &Context, t: &Term) -> Option<Term> {
    if let Some(proof) = canonical_proof(ctx, t) {
        return Some(proof);
    }
    let mut allowance = Allowance::new(STEPS);
    let core = whnf_core(ctx, t.clone(), &mut allowance).ok()?;
    if !crate::type_checker::alpha_equivalent(&core, t) {
        return Some(core);
    }
    delta_step(ctx, t)
}

/// The recursion's new dividend: the first field `d` of a pair `mk ps d y'` (with `y' ≡ y`)
/// passed as an argument somewhere in the then-branch (binder annotations are not looked into),
/// other than one convertible to `x`. Only a candidate: the certificate then proves the branch is
/// the program at `(s x y, y)`.
fn recursive_dividend(ctx: &Context, t: &Term, pair: &Term, parameters: usize) -> Option<Term> {
    let (head, args) = spine(t);
    if crate::type_checker::alpha_equivalent(head, pair)
        && args.len() == parameters + 2
        && def_eq(ctx, args[parameters + 1], &v(Y))
        && !def_eq(ctx, args[parameters], &v(X))
    {
        return Some(args[parameters].clone());
    }
    match t {
        Term::App(f, a) => recursive_dividend(ctx, a, pair, parameters)
            .or_else(|| recursive_dividend(ctx, f, pair, parameters)),
        Term::Lambda { body, .. } => recursive_dividend(ctx, body, pair, parameters),
        _ => None,
    }
}

impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    /// The certified division program at `head`, if any (see the module documentation).
    pub(super) fn certified_division(&mut self, head: TermId) -> Option<Division> {
        if !self.canonical_nat
            || self.preserve_nat_syntax
            || self.ctx.checking_order_recurrence
            || !self.ctx.simp_rules().is_empty()
        {
            return None;
        }
        if let Some(answer) = self.certified_divisions.get(&head) {
            return answer.clone();
        }
        let Node::Global(symbol) = self.store.node(head) else {
            return None;
        };
        let name = self.store.name(*symbol).to_owned();
        if !division_signature(self.ctx, &name) {
            self.certified_divisions.insert(head, None);
            return None;
        }
        if self.division_attempts == 0 {
            return None;
        }
        self.division_attempts -= 1;
        self.certified_divisions.insert(head, None);
        let ctx = self.ctx;
        let outcome = ctx.cached_division_recurrence(&name, || {
            let division = certify_division(ctx, &name)?;
            let op = if division.op == Arithmetic::Div {
                "div"
            } else {
                "mod"
            };
            Some([op.to_owned(), division.pair])
        });
        let answer = outcome.and_then(|[op, pair]| {
            let op = match op.as_str() {
                "div" => Arithmetic::Div,
                "mod" => Arithmetic::Mod,
                _ => return None,
            };
            let parameters = ctx.inductive_num_params(ctx.constructor_inductive(&pair)?);
            Some(Division {
                op,
                pair,
                parameters,
            })
        });
        self.certified_divisions.insert(head, answer.clone());
        answer
    }
}

/// The pair constructor and parameter count of a division program's domain, when `name` has the
/// signature one has ([`division_signature`]) and its constructor takes the parameters and two fields.
pub(super) fn division_pair(ctx: &Context, name: &str) -> Option<(String, usize)> {
    if !division_signature(ctx, name) {
        return None;
    }
    let Term::Pi {
        param_type: domain, ..
    } = ctx.get_definition_type(name)?
    else {
        return None;
    };
    let (family, parameters) = spine(domain);
    let family = constant_name(family)?;
    let constructors = ctx.get_constructors(family);
    let [(pair, pair_type)] = constructors.as_slice() else {
        return None;
    };
    let mut fields = 0usize;
    let mut rest = *pair_type;
    while let Term::Pi { body_type, .. } = rest {
        fields += 1;
        rest = body_type;
    }
    (ctx.inductive_num_params(family) == parameters.len() && fields == parameters.len() + 2)
        .then(|| (pair.to_string(), parameters.len()))
}

/// The signature a division program has: a checked definition `Π (p : I ps). Nat` on a non-`Prop`
/// inductive `I` with one constructor. A cheap filter, before any certification work.
fn division_signature(ctx: &Context, name: &str) -> bool {
    if ctx.poly_declaration(name).is_some() || ctx.get_definition_body(name).is_none() {
        return false;
    }
    let Some(ty) = ctx.get_definition_type(name) else {
        return false;
    };
    if ctx.get_global(name) != Some(ty) {
        return false;
    }
    let Term::Pi {
        param_type: domain,
        body_type: codomain,
        ..
    } = ty
    else {
        return false;
    };
    if **codomain != g("Nat") {
        return false;
    }
    let (family, _) = spine(domain);
    constant_name(family).is_some_and(|family| {
        ctx.is_inductive(family)
            && !propositional(ctx, family)
            && ctx.get_constructors(family).len() == 1
    })
}

/// Certify `name` as the modulus or the quotient program (see the module documentation).
fn certify_division(ctx: &Context, name: &str) -> Option<Division> {
    if ctx.poly_declaration(name).is_some() {
        return None;
    }
    let ty = ctx.get_definition_type(name)?;
    if ctx.get_global(name) != Some(ty) {
        return None;
    }
    let Term::Pi {
        param_type: domain,
        body_type: codomain,
        ..
    } = ty
    else {
        return None;
    };
    if **codomain != g("Nat") {
        return None;
    }
    let (pair_family, parameters) = spine(domain);
    let family = constant_name(pair_family)?;
    if !ctx.is_inductive(family)
        || propositional(ctx, family)
        || ctx.inductive_num_params(family) != parameters.len()
    {
        return None;
    }
    let constructors = ctx.get_constructors(family);
    let [(pair_name, _)] = constructors.as_slice() else {
        return None;
    };
    let pair = sibling(pair_family, pair_name);
    let parameters: Vec<Term> = parameters.into_iter().cloned().collect();
    let ctx = ctx
        .order_recurrence_context()
        .extend(X, g("Nat"))
        .extend(Y, g("Nat"));
    // The constructor's two fields are naturals: `mk ps (x : Nat) (y : Nat)`.
    let fields = instantiate(&type_of_constant(&ctx, &pair)?, &parameters)?;
    let Term::Pi {
        param: first,
        param_type: first_type,
        body_type: rest,
    } = fields
    else {
        return None;
    };
    let rest = crate::substitute(&rest, &first, &v(X));
    let Term::Pi {
        param_type: second_type,
        ..
    } = rest
    else {
        return None;
    };
    if !def_eq(&ctx, &first_type, &g("Nat")) || !def_eq(&ctx, &second_type, &g("Nat")) {
        return None;
    }
    let make = |a: Term, b: Term| {
        apply(
            pair.clone(),
            &parameters.iter().cloned().chain([a, b]).collect::<Vec<_>>(),
        )
    };
    let program = g(name);
    let (_selector, args) = unfold_to_selector(&ctx, &app(program.clone(), make(v(X), v(Y))))?;
    let [alpha, guard, _decision, then_branch, else_branch] = &args[..] else {
        return None;
    };
    if !def_eq(&ctx, alpha, &g("Nat")) || !guards_division(&ctx, guard) {
        return None;
    }
    let Term::Lambda {
        param: h,
        body: then_body,
        ..
    } = then_branch
    else {
        return None;
    };
    let Term::Lambda {
        param: h_not,
        param_type: not_guard,
        body: else_body,
    } = else_branch
    else {
        return None;
    };
    let ctx_then = ctx.extend(h, guard.clone());
    let ctx_else = ctx.extend(h_not, (**not_guard).clone());
    let dividend = recursive_dividend(&ctx_then, then_body, &pair, parameters.len())?;
    let subtraction = subtraction_program(&ctx, &dividend)?;
    let recursive = app(program, make(apply(subtraction, &[v(X), v(Y)]), v(Y)));
    if def_eq(&ctx_then, then_body, &recursive) && def_eq(&ctx_else, else_body, &v(X)) {
        return Some(Division {
            op: Arithmetic::Mod,
            pair: pair_name.to_string(),
            parameters: parameters.len(),
        });
    }
    if def_eq(&ctx_then, then_body, &app(g("Succ"), recursive))
        && def_eq(&ctx_else, else_body, &g("Zero"))
    {
        return Some(Division {
            op: Arithmetic::Div,
            pair: pair_name.to_string(),
            parameters: parameters.len(),
        });
    }
    None
}

/// The program `s` with `dividend ≡ s x y` that the arithmetic certificate proves is truncated
/// subtraction, found by δ-unfolding `dividend` until such a head appears.
fn subtraction_program(ctx: &Context, dividend: &Term) -> Option<Term> {
    let mut t = dividend.clone();
    for _ in 0..64 {
        t = whnf_core(ctx, t, &mut Allowance::new(STEPS)).ok()?;
        let (head, args) = spine(&t);
        if let (Term::Global(name), [x, y]) = (head, args.as_slice()) {
            if def_eq(ctx, x, &v(X)) && def_eq(ctx, y, &v(Y)) && certified_subtraction(ctx, name) {
                return Some(head.clone());
            }
        }
        t = delta_step(ctx, &t)?;
    }
    None
}

/// Whether the arithmetic certificate proves `name` is truncated subtraction, in the full context
/// (the recurrence context disables every certificate).
fn certified_subtraction(ctx: &Context, name: &str) -> bool {
    let mut full = ctx.clone();
    full.checking_order_recurrence = false;
    let mut probe = HeadReducer::new(&full, 1 << 16);
    let head = probe.store.intern(&g(name));
    probe.certified_arithmetic(head) == Some(Arithmetic::Sub)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_division/controls.rs"]
mod controls;
