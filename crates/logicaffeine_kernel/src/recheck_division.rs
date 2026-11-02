//! Division and modulus on naturals in the de Bruijn kernel, from a recurrence this kernel checks.
//!
//! Lean exports `Nat.div` and `Nat.modCore` as unary well-founded programs `u (x, y)` on a pair
//! type (`WellFounded.fix … F`); their unrolling runs `Acc.rec` on a proof, and their recursion
//! subtracts the divisor once per step, so `2^128 / 2^64` is out of reach of reduction. This kernel
//! computes such a program natively only after it has proved, with its own conversion, its own head
//! reduction and its own subtraction certificate, that the program's ONE-STEP UNFOLDING is Lean's
//! recurrence for fresh `x y : Nat`:
//!
//! * modulus: `u (x, y) ≡ sel (0 < y ∧ y ≤ x) (λ_. u (x - y, y)) (λ_. x)`;
//! * quotient: `u (x, y) ≡ sel (0 < y ∧ y ≤ x) (λ_. u (x - y, y) + 1) (λ_. 0)`;
//!
//! where `sel` is any program this kernel proves selects its first branch on the `isTrue` and its
//! second on the `isFalse` of a two-constructor decision family, the guard is the conjunction of the
//! standard `Nat.le` (both inductive declarations checked constructor by constructor), and `x - y`
//! is a program this kernel certifies as truncated subtraction (`arithmetic::certifies_difference`).
//!
//! The unfolding reduces `u (x, y)` at the head — β, ζ, δ, ι, structural unrolling — and, where a
//! `match` or a fixpoint is stuck on a PROOF of a one-constructor proposition (`Acc r p`), replaces
//! that proof by the constructor applied to canonical fields, `intro p (λ y hy. a[p := y])` for the
//! proof `a p`, after checking that the replacement proves the same proposition: the two are equal
//! by proof irrelevance, so every step is one of this kernel's rules.
//!
//! Canonicity then makes the native answer the program's value on closed operands: for closed
//! `x₀ y₀` the guard's decision reduces to `isTrue` exactly when `1 ≤ y₀ ∧ y₀ ≤ x₀`, so `u (x₀, y₀)`
//! is `u (x₀ - y₀, y₀)` (plus one for the quotient) while it holds and `x₀` (or `0`) after — by
//! induction on `x₀`, `x₀ mod y₀` (`x₀` when `y₀ = 0`) and `x₀ div y₀` (`0` when `y₀ = 0`).
//!
//! Nothing here reads the first kernel's certificate: the certificate cache keeps the two issuers
//! apart, and every judgement below is this kernel's own `infer`, `whnf` and `def_eq`.
use super::*;

/// Head-reduction steps one certificate's unfolding may take.
const STEPS: usize = 1 << 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Operation {
    Quotient,
    Remainder,
}

fn global(name: &str) -> Db {
    Db::Global(name.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn apply(head: Db, args: impl IntoIterator<Item = Db>) -> Db {
    args.into_iter().fold(head, app)
}
fn nat() -> Db {
    global("Nat")
}
fn succ(t: Db) -> Db {
    app(global("Succ"), t)
}
/// `t`, written `by` binders deeper.
fn lift(t: &Db, by: usize) -> Db {
    shift(t, by as isize, 0)
}

/// The name of a constant head.
fn constant(t: &Db) -> Option<&str> {
    match t {
        Db::Global(name) | Db::Const { name, .. } => Some(name),
        _ => None,
    }
}

/// The same constant at another name (and the same universe levels).
fn sibling(t: &Db, name: &str) -> Db {
    match t {
        Db::Const { levels, .. } => Db::Const {
            name: name.into(),
            levels: levels.clone(),
        },
        _ => global(name),
    }
}

/// Whether de Bruijn index `k` occurs free in `t`.
fn mentions(t: &Db, k: usize) -> bool {
    match t {
        Db::Var(i) => *i == k,
        Db::Pi(a, b) | Db::Lam(a, b) => mentions(a, k) || mentions(b, k + 1),
        Db::App(f, a) => mentions(f, k) || mentions(a, k),
        Db::Let(ty, value, body) => mentions(ty, k) || mentions(value, k) || mentions(body, k + 1),
        Db::Match {
            disc,
            motive,
            cases,
        } => mentions(disc, k) || mentions(motive, k) || cases.iter().any(|c| mentions(c, k)),
        Db::Fix(body) => mentions(body, k + 1),
        Db::MutualFix { defs, .. } => defs.iter().any(|d| mentions(d, k + defs.len())),
        Db::Sort(_) | Db::Global(_) | Db::Const { .. } | Db::Lit(_) => false,
    }
}

/// Whether the inductive `name`'s declared type ends in `Prop`.
fn propositional(genv: &Context, name: &str) -> bool {
    let declared = match genv.poly_declaration(name) {
        Some((_, ty)) => ty,
        None => match genv.get_global(name) {
            Some(ty) => ty,
            None => return false,
        },
    };
    let mut result = declared;
    while let Term::Pi { body_type, .. } = result {
        result = body_type;
    }
    matches!(result, Term::Sort(level) if level.equiv(&Universe::Prop))
}

/// The inductive family at the head of `ty` after weak-head reduction, with its arguments.
fn inductive_view(genv: &Context, lctx: &dyn LocalTypes, ty: &Db) -> Option<(Db, Vec<Db>)> {
    let exposed = whnf_in(genv, lctx, ty);
    let (head, args) = spine(&exposed);
    genv.is_inductive(constant(&head)?).then_some((head, args))
}

/// The type of `t`, weak-head normalized.
fn exposed_type(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    Some(whnf_in_owned(genv, lctx, infer_reduct(genv, lctx, t).ok()?))
}

/// A proof `a` of a one-constructor proposition `I ps t` whose constructor takes the index `t` and a
/// function returning proofs at other indices (`Acc.intro x h`), rebuilt as that constructor:
/// `intro ps t (λ y hy. a[t := y])`, where `t` is `a`'s last argument. `None` unless this kernel
/// proves the replacement a proof of the same proposition.
fn canonical_proof(genv: &Context, lctx: &dyn LocalTypes, a: &Db) -> Option<Db> {
    let ty = infer_reduct(genv, lctx, a).ok()?;
    let (family, args) = inductive_view(genv, lctx, &ty)?;
    let family_name = constant(&family)?;
    if !propositional(genv, family_name) {
        return None;
    }
    let constructors = genv.get_constructors(family_name);
    let [(constructor, _)] = constructors.as_slice() else {
        return None;
    };
    let parameters = genv.inductive_num_params(family_name);
    if args.len() != parameters + 1 {
        return None;
    }
    let index = args[parameters].clone();
    let (proof_head, proof_args) = spine(a);
    if proof_args.last() != Some(&index) {
        return None;
    }
    let constructor = sibling(&family, constructor);
    let Db::Pi(_, rest) = exposed_type(
        genv,
        lctx,
        &apply(constructor.clone(), args[..parameters].iter().cloned()),
    )?
    else {
        return None;
    };
    let Db::Pi(recursive_field, _) = whnf_in_owned(genv, lctx, beta_open(&rest, &index)) else {
        return None;
    };
    let Db::Pi(y_type, after_y) = recursive_field.as_ref() else {
        return None;
    };
    let Db::Pi(hy_type, _) = after_y.as_ref() else {
        return None;
    };
    let rebuilt = apply(
        lift(&proof_head, 2),
        proof_args[..proof_args.len() - 1]
            .iter()
            .map(|t| lift(t, 2))
            .chain([Db::Var(1)]),
    );
    let h = Db::Lam(
        DbBox::new((**y_type).clone()),
        DbBox::new(Db::Lam(
            DbBox::new((**hy_type).clone()),
            DbBox::new(rebuilt),
        )),
    );
    let replacement = apply(
        constructor,
        args[..parameters].iter().cloned().chain([index, h]),
    );
    let replaced = infer_reduct(genv, lctx, &replacement).ok()?;
    def_eq(genv, lctx, &replaced, &ty).then_some(replacement)
}

/// One δ step at the head of `t`'s application spine.
fn delta(genv: &Context, t: &Db) -> Option<Db> {
    let (head, args) = spine(t);
    let body = match &head {
        Db::Global(name) => definition_body(genv, name)?,
        Db::Const { name, levels } => const_body(genv, name, levels)?,
        _ => return None,
    };
    Some(apply(body, args))
}

/// One step toward exposing a constructor in a discriminant or structural argument: a stuck proof
/// of a one-constructor proposition becomes its canonical constructor; anything else takes this
/// kernel's weak-head reduction without head δ, else one δ step.
fn expose(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    if let Some(proof) = canonical_proof(genv, lctx, t) {
        return Some(proof);
    }
    let core = whnf_core_in(genv, lctx, t.clone());
    if core != *t {
        return Some(core);
    }
    delta(genv, t)
}

/// Whether `head` selects `t h` on `isTrue h` and `e h` on `isFalse h` of its decision argument at
/// the result type `alpha`: `head α c (isTrue h) t e ≡ t h` and `head α c (isFalse h) t e ≡ e h` for
/// fresh `c h t e`, where `isTrue`/`isFalse` are the constructors of the decision's two-constructor
/// family whose field is `c` and `c → ⊥` (an inductive proposition with no constructor).
fn selects(genv: &Context, locals: &[Db], head: &Db, alpha: &Db) -> Option<bool> {
    let Db::Pi(_, after_alpha) = exposed_type(genv, &locals.to_vec(), head)? else {
        return Some(false);
    };
    let Db::Pi(_, after_c) = whnf_in_owned(genv, &locals.to_vec(), beta_open(&after_alpha, alpha))
    else {
        return Some(false);
    };
    // Under `c : Prop`.
    let mut with_c = locals.to_vec();
    with_c.push(Db::Sort(Universe::Prop));
    let Db::Pi(decision, _) = whnf_in_owned(genv, &with_c, (*after_c).clone()) else {
        return Some(false);
    };
    let Some((family, family_args)) = inductive_view(genv, &with_c, &decision) else {
        return Some(false);
    };
    let family_name = constant(&family)?.to_owned();
    let constructors: Vec<String> = genv
        .get_constructors(&family_name)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    if constructors.len() != 2 || genv.inductive_num_params(&family_name) != family_args.len() {
        return Some(false);
    }
    let field = |constructor: &str| -> Option<Db> {
        match exposed_type(
            genv,
            &with_c,
            &apply(sibling(&family, constructor), family_args.iter().cloned()),
        )? {
            Db::Pi(field, _) => Some((*field).clone()),
            _ => None,
        }
    };
    let c = Db::Var(0);
    let refutes = |field: &Db| -> bool {
        let Db::Pi(premise, conclusion) = whnf_in(genv, &with_c, field) else {
            return false;
        };
        let mut under = with_c.clone();
        under.push((*premise).clone());
        def_eq(genv, &with_c, &premise, &c)
            && inductive_view(genv, &under, &conclusion).is_some_and(|(empty, _)| {
                constant(&empty)
                    .is_some_and(|n| propositional(genv, n) && genv.get_constructors(n).is_empty())
            })
    };
    let (first, second) = (field(&constructors[0])?, field(&constructors[1])?);
    let (is_true, is_false, not_c) = if def_eq(genv, &with_c, &first, &c) && refutes(&second) {
        (&constructors[0], &constructors[1], second)
    } else if def_eq(genv, &with_c, &second, &c) && refutes(&first) {
        (&constructors[1], &constructors[0], first)
    } else {
        return Some(false);
    };
    // Fresh `hp : c`, `hn : ¬c`, `t : c → α`, `e : ¬c → α` beneath `c`; at the innermost depth `c` is
    // index 4, `hp` 3, `hn` 2, `t` 1 and `e` 0.
    let arrow = |dom: Db, cod: Db| Db::Pi(DbBox::new(dom), DbBox::new(cod));
    let mut all = with_c.clone();
    all.push(Db::Var(0));
    all.push(lift(&not_c, 1));
    all.push(arrow(Db::Var(2), lift(alpha, 4)));
    all.push(arrow(lift(&not_c, 3), lift(alpha, 5)));
    let decide = |constructor: &str, proof: usize| {
        apply(
            sibling(&lift(&family, 4), constructor),
            family_args
                .iter()
                .map(|a| lift(a, 4))
                .chain([Db::Var(proof)]),
        )
    };
    let choose = |decision: Db| {
        apply(
            lift(head, 5),
            [lift(alpha, 5), Db::Var(4), decision, Db::Var(1), Db::Var(0)],
        )
    };
    Some(
        def_eq(
            genv,
            &all,
            &choose(decide(is_true, 3)),
            &app(Db::Var(1), Db::Var(3)),
        ) && def_eq(
            genv,
            &all,
            &choose(decide(is_false, 2)),
            &app(Db::Var(0), Db::Var(2)),
        ),
    )
}

/// The inductive `le` is the standard order on naturals: one parameter `n`, one index, and exactly
/// the constructors `refl : le n n` and `step : ∀ m, le n m → le n (m + 1)`.
fn standard_le(genv: &Context, locals: &[Db], le: &Db) -> bool {
    let Some(name) = constant(le) else {
        return false;
    };
    if !propositional(genv, name) || genv.inductive_num_params(name) != 1 {
        return false;
    }
    let constructors: Vec<String> = genv
        .get_constructors(name)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    if constructors.len() != 2 {
        return false;
    }
    let mut with_n = locals.to_vec();
    with_n.push(nat());
    let at = |constructor: &str| {
        infer_reduct(genv, &with_n, &app(sibling(le, constructor), Db::Var(0))).ok()
    };
    let (Some(refl), Some(step)) = (at(&constructors[0]), at(&constructors[1])) else {
        return false;
    };
    let le_of = |a: Db, b: Db| apply(le.clone(), [a, b]);
    // Under `n`: `Π(m : Nat). Π(_ : le n m). le n (Succ m)` — `n` is index 1 beneath `m` and 2
    // beneath the premise as well.
    let expected_step = Db::Pi(
        DbBox::new(nat()),
        DbBox::new(Db::Pi(
            DbBox::new(le_of(Db::Var(1), Db::Var(0))),
            DbBox::new(le_of(Db::Var(2), succ(Db::Var(1)))),
        )),
    );
    def_eq(genv, &with_n, &refl, &le_of(Db::Var(0), Db::Var(0)))
        && def_eq(genv, &with_n, &step, &expected_step)
}

/// The inductive `and` is the standard conjunction: two proposition parameters `a b` and one
/// constructor with exactly the fields `a` and `b`.
fn standard_and(genv: &Context, locals: &[Db], and: &Db) -> bool {
    let Some(name) = constant(and) else {
        return false;
    };
    if !propositional(genv, name) || genv.inductive_num_params(name) != 2 {
        return false;
    }
    let constructors = genv.get_constructors(name);
    let [(constructor, _)] = constructors.as_slice() else {
        return false;
    };
    let mut with_ab = locals.to_vec();
    with_ab.push(Db::Sort(Universe::Prop));
    with_ab.push(Db::Sort(Universe::Prop));
    let Ok(fields) = infer_reduct(
        genv,
        &with_ab,
        &apply(sibling(and, constructor), [Db::Var(1), Db::Var(0)]),
    ) else {
        return false;
    };
    // `Π(_ : a). Π(_ : b). and a b`, with `a b` at indices 1 0 beneath no binder.
    let expected = Db::Pi(
        DbBox::new(Db::Var(1)),
        DbBox::new(Db::Pi(
            DbBox::new(Db::Var(1)),
            DbBox::new(apply(and.clone(), [Db::Var(3), Db::Var(2)])),
        )),
    );
    def_eq(genv, &with_ab, &fields, &expected)
}

/// Whether the proposition `guard`, read with `x y` at indices 1 0, is `0 < y ∧ y ≤ x` over the
/// standard conjunction and order.
fn guards_division(genv: &Context, locals: &[Db], guard: &Db) -> bool {
    let lctx = locals.to_vec();
    let Some((and, conjuncts)) = inductive_view(genv, &lctx, guard) else {
        return false;
    };
    let [left, right] = conjuncts.as_slice() else {
        return false;
    };
    if !standard_and(genv, locals, &and) {
        return false;
    }
    let order = |t: &Db| -> Option<(Db, Db, Db)> {
        let (le, args) = inductive_view(genv, &lctx, t)?;
        let [a, b] = args.as_slice() else { return None };
        standard_le(genv, locals, &le).then(|| (le, a.clone(), b.clone()))
    };
    let (Some((le1, one, y1)), Some((le2, y2, x2))) = (order(left), order(right)) else {
        return false;
    };
    constant(&le1) == constant(&le2)
        && def_eq(genv, &lctx, &one, &succ(global("Zero")))
        && def_eq(genv, &lctx, &y1, &Db::Var(0))
        && def_eq(genv, &lctx, &y2, &Db::Var(0))
        && def_eq(genv, &lctx, &x2, &Db::Var(1))
}

/// Reduce `term` at the head — β, ζ, δ, ι, structural unrolling — replacing a stuck proof of a
/// one-constructor proposition by its canonical constructor, until the head is a selector
/// (`selects`) applied to at least five arguments. `None` when anything else gets stuck.
fn unfold_to_selector(genv: &Context, locals: &[Db], term: Db) -> Option<(Db, Vec<Db>)> {
    let lctx = locals.to_vec();
    let mut term = term;
    let mut selectors: HashMap<String, bool> = HashMap::new();
    for _ in 0..STEPS {
        if !nat_order_tick()
            || WHNF_EXHAUSTED.with(|e| e.get())
            || db_forest_size_exceeds(vec![&term], MAX_RECHECK_TERM_NODES)
        {
            return None;
        }
        let (head, args) = spine(&term);
        term = match head {
            Db::Lam(_, body) if !args.is_empty() => {
                apply(beta_open(&body, &args[0]), args[1..].iter().cloned())
            }
            Db::Let(_, value, body) => apply(beta_open(&body, &value), args),
            Db::Match {
                disc,
                motive,
                cases,
            } => match resume_match(
                genv,
                &lctx,
                disc.as_ref().clone(),
                motive.clone(),
                cases.clone(),
            ) {
                WhnfStep::Reduce(branch) => apply(branch, args),
                _ => {
                    let exposed = expose(genv, &lctx, &disc)?;
                    apply(
                        Db::Match {
                            disc: DbBox::new(exposed),
                            motive,
                            cases,
                        },
                        args,
                    )
                }
            },
            Db::Fix(ref body) => {
                let column = fix_structural_column(genv, &term)?;
                let major = &args[column];
                if constructor_value(genv, major) {
                    apply(beta_open(body, &Db::Fix(body.clone())), args)
                } else {
                    let mut replaced = args.clone();
                    replaced[column] = expose(genv, &lctx, major)?;
                    apply(head.clone(), replaced)
                }
            }
            Db::Global(ref name) | Db::Const { ref name, .. } => {
                if args.len() >= 5 {
                    let is_selector = match selectors.get(name) {
                        Some(known) => *known,
                        None => {
                            let known = selects(genv, locals, &head, &args[0]).unwrap_or(false);
                            selectors.insert(name.clone(), known);
                            known
                        }
                    };
                    if is_selector {
                        return Some((head, args));
                    }
                }
                delta(genv, &term)?
            }
            _ => return None,
        };
    }
    None
}

/// The recursion's new dividend: the first field `d` of a pair `mk ps d y'` (with `y' ≡ y`) passed as
/// an argument somewhere in the then-branch (binder annotations are not looked into), other than one
/// convertible to `x`. Read beneath the branch's own binder, where `x y` are indices 2 1. Only a
/// candidate: the certificate then proves the branch is the program at `(s x y, y)`.
fn recursive_dividend(
    genv: &Context,
    lctx: &dyn LocalTypes,
    t: &Db,
    pair: &Db,
    parameters: usize,
) -> Option<Db> {
    let (head, args) = spine(t);
    if head == *pair
        && args.len() == parameters + 2
        && def_eq(genv, lctx, &args[parameters + 1], &Db::Var(1))
        && !def_eq(genv, lctx, &args[parameters], &Db::Var(2))
    {
        return Some(args[parameters].clone());
    }
    match t {
        Db::App(f, a) => recursive_dividend(genv, lctx, a, pair, parameters)
            .or_else(|| recursive_dividend(genv, lctx, f, pair, parameters)),
        _ => None,
    }
}

/// The program `s` with `dividend ≡ s x y` that this kernel certifies as truncated subtraction,
/// found by δ-unfolding `dividend` until such a head appears. `x y` are indices 1 0.
fn subtraction_program(
    genv: &Context,
    probe: &Context,
    locals: &[Db],
    dividend: &Db,
) -> Option<Db> {
    let lctx = locals.to_vec();
    let mut t = dividend.clone();
    for _ in 0..64 {
        t = whnf_core_in(probe, &lctx, t);
        let (head, args) = spine(&t);
        if let (Db::Global(name), [x, y]) = (&head, args.as_slice()) {
            if def_eq(probe, &lctx, x, &Db::Var(1))
                && def_eq(probe, &lctx, y, &Db::Var(0))
                && arithmetic::certifies_difference(genv, name)
            {
                return Some(head);
            }
        }
        t = delta(probe, &t)?;
    }
    None
}

/// The certified division program `name`: its operation, pair constructor and parameter count.
fn certify(genv: &Context, name: &str) -> Option<(Operation, String, usize)> {
    let (pair_family, parameters) = signature(genv, name)?;
    let family = constant(&pair_family)?.to_owned();
    let constructors = genv.get_constructors(&family);
    let [(pair_name, _)] = constructors.as_slice() else {
        return None;
    };
    let pair_name = pair_name.to_string();
    let pair = sibling(&pair_family, &pair_name);
    let probe = genv.order_recurrence_context();
    // `x y : Nat` at indices 1 0.
    let locals = vec![nat(), nat()];
    let (x, y) = (Db::Var(1), Db::Var(0));
    // The constructor's two fields are naturals: `mk ps (x : Nat) (y : Nat)`.
    let Db::Pi(first, rest) = exposed_type(
        &probe,
        &locals,
        &apply(pair.clone(), parameters.iter().cloned()),
    )?
    else {
        return None;
    };
    let Db::Pi(second, _) = whnf_in_owned(&probe, &locals, beta_open(&rest, &x)) else {
        return None;
    };
    if !def_eq(&probe, &locals, &first, &nat()) || !def_eq(&probe, &locals, &second, &nat()) {
        return None;
    }
    let make = |a: Db, b: Db| apply(pair.clone(), parameters.iter().cloned().chain([a, b]));
    let program = global(name);
    let (_selector, args) = unfold_to_selector(
        &probe,
        &locals,
        app(program.clone(), make(x.clone(), y.clone())),
    )?;
    let [alpha, guard, _decision, then_branch, else_branch] = &args[..] else {
        return None;
    };
    if !def_eq(&probe, &locals, alpha, &nat()) || !guards_division(&probe, &locals, guard) {
        return None;
    }
    let Db::Lam(then_domain, then_body) = then_branch else {
        return None;
    };
    let Db::Lam(else_domain, else_body) = else_branch else {
        return None;
    };
    let mut then_locals = locals.clone();
    then_locals.push((**then_domain).clone());
    let mut else_locals = locals.clone();
    else_locals.push((**else_domain).clone());
    let dividend = recursive_dividend(&probe, &then_locals, then_body, &pair, parameters.len())?;
    if mentions(&dividend, 0) {
        return None;
    }
    let subtraction = subtraction_program(genv, &probe, &locals, &shift(&dividend, -1, 0))?;
    let recursive = lift(
        &app(program, make(apply(subtraction, [x.clone(), y.clone()]), y)),
        1,
    );
    let (x1, zero) = (lift(&x, 1), global("Zero"));
    if def_eq(&probe, &then_locals, then_body, &recursive)
        && def_eq(&probe, &else_locals, else_body, &x1)
    {
        return Some((Operation::Remainder, pair_name, parameters.len()));
    }
    if def_eq(&probe, &then_locals, then_body, &succ(recursive))
        && def_eq(&probe, &else_locals, else_body, &zero)
    {
        return Some((Operation::Quotient, pair_name, parameters.len()));
    }
    None
}

/// The signature a division program has: a checked monomorphic definition `Π (p : I ps). Nat` on a
/// non-`Prop` inductive `I` with one constructor. A cheap filter, before any certification work.
fn signature(genv: &Context, name: &str) -> Option<(Db, Vec<Db>)> {
    if genv.poly_declaration(name).is_some()
        || genv.get_definition_body(name).is_none()
        || genv.has_pending_dependency(&Term::Global(name.into()))
    {
        return None;
    }
    let ty = genv.get_definition_type(name)?;
    if genv.get_global(name) != Some(ty) {
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
    if !matches!(codomain.as_ref(), Term::Global(n) if n == "Nat") {
        return None;
    }
    let mut family = domain.as_ref();
    let mut arguments = 0usize;
    while let Term::App(function, _) = family {
        arguments += 1;
        family = function;
    }
    let family_name = match family {
        Term::Global(n) | Term::Const { name: n, .. } => n,
        _ => return None,
    };
    let constructors = genv.get_constructors(family_name);
    let [(_, constructor_type)] = constructors.as_slice() else {
        return None;
    };
    let mut fields = 0usize;
    let mut rest = *constructor_type;
    while let Term::Pi { body_type, .. } = rest {
        fields += 1;
        rest = body_type;
    }
    if !genv.is_inductive(family_name)
        || propositional(genv, family_name)
        || genv.inductive_num_params(family_name) != arguments
        || fields != arguments + 2
    {
        return None;
    }
    let domain = to_db(domain, &mut Vec::new()).ok()?;
    let (family, parameters) = spine(&domain);
    Some((family, parameters))
}

/// `u (mk ps a b)` for a certified division program `u` and closed numerals `a b`: the native
/// quotient or remainder. Operands are observed before any certification work, so an application to
/// open or unknown data never pays for a certificate.
pub(super) fn reduce(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    if !arithmetic::observing()
        || genv.checking_order_recurrence
        || !genv.simp_rules().is_empty()
        || WHNF_EXHAUSTED.with(|e| e.get())
        || genv.constructor_inventory_upper_bound("Nat") > 4096
        || !crate::int_inductive::canonical_nat_environment(genv)
    {
        return None;
    }
    let Db::App(function, pair) = t else {
        return None;
    };
    let Db::Global(name) = function.as_ref() else {
        return None;
    };
    let (family, _) = signature(genv, name)?;
    let parent = WHNF_DEPTH.with(|d| d.get());
    if parent == 0 || parent >= MAX_RECHECK_WHNF_DEPTH {
        return None;
    }
    let _guard = WhnfDepthGuard(parent);
    WHNF_DEPTH.with(|d| d.set(parent + 1));
    let observed = whnf_in(genv, lctx, pair);
    let (head, args) = spine(&observed);
    let family_name = constant(&family)?;
    let (constructor, _) = genv.get_constructors(family_name).into_iter().next()?;
    if constant(&head) != Some(constructor)
        || args.len() != genv.inductive_num_params(family_name) + 2
    {
        return None;
    }
    let dividend = arithmetic::numeral(genv, lctx, &args[args.len() - 2])?;
    let divisor = arithmetic::numeral(genv, lctx, &args[args.len() - 1])?;
    // A refusal is retained only when no reduction under the certification was cut short: the
    // certification is then a function of the registries alone (its own step and size bounds are
    // fixed), and a program shaped like a division that is none — `Nat.gcd._unary` — is not
    // certified again at every observation.
    let certificate = genv.cached_independent_division(name, || {
        let certified = certify(genv, name).map(|(operation, pair, _)| {
            let operation = match operation {
                Operation::Quotient => "div",
                Operation::Remainder => "mod",
            };
            [operation.to_owned(), pair]
        });
        #[cfg(test)]
        CERTIFICATIONS.with(|n| n.set(n.get() + 1));
        (certified, !WHNF_EXHAUSTED.with(|e| e.get()))
    })?;
    if certificate[1] != constructor || WHNF_EXHAUSTED.with(|e| e.get()) {
        return None;
    }
    let (quotient, remainder) = if divisor.is_zero() {
        (crate::BigInt::zero(), dividend)
    } else {
        dividend.div_rem(&divisor)?
    };
    Some(Db::Lit(Literal::Nat(if certificate[0] == "div" {
        quotient
    } else {
        remainder
    })))
}

#[cfg(test)]
thread_local! {
    /// The division certifications this thread has run: the work the per-generation cache holds.
    static CERTIFICATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/division/controls.rs"]
mod controls;
