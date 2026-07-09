//! A call-by-value evaluator for the kernel's computational fragment — the engine behind
//! `native_decide`.
//!
//! Unlike [`normalize`](crate::reduction::normalize), which β/ι/δ-reduces by SUBSTITUTION
//! and normalizes under binders to full normal form, this evaluates a CLOSED term to weak
//! head normal form using an ENVIRONMENT (no substitution), stopping as soon as the head
//! constructor is known. So deciding `Eq Nat 10000 10000` is a linear environment walk, not
//! a quadratic substitution blowup — the point of `native_decide`.
//!
//! TRUST: `native_decide` trusts this evaluator to agree with the kernel's reduction (as
//! Lean's `native_decide` trusts its compiler). It is validated by a differential test
//! against `normalize`, and fail-safes to `None` (declining) on anything it cannot decide,
//! but it IS part of the trusted base whenever `native_decide` is used.

use crate::context::Context;
use crate::term::{int_lit, lit_bigint, Literal, Term};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A persistent environment mapping bound variables to already-evaluated values.
enum Scope {
    Empty,
    Bind(String, Val, Rc<Scope>),
}

#[derive(Clone)]
enum Val {
    /// A constructor applied to ALL its arguments (parameters + value arguments), evaluated.
    Ctor(String, Vec<Val>),
    /// A `λ` closure: (parameter name, body) and the captured environment.
    Clos(Rc<(String, Term)>, Rc<Scope>),
    /// A `fix`: (recursive-binder name, body) and the captured environment.
    Fix(Rc<(String, Term)>, Rc<Scope>),
    /// A primitive literal (`Lit(Int)` and friends) — the ALU-computed values.
    Lit(Literal),
    /// A partially-applied arithmetic/comparison primitive (`add`/`le`/…),
    /// accumulating its two arguments; computes exactly as the kernel's
    /// `try_primitive_reduce` does (checked — overflow/div-zero goes Stuck).
    Prim(&'static str, Vec<Val>),
    /// No progress possible (free variable, opaque/axiom global, `Sort`/`Π`/`Hole`, a
    /// stuck application, or fuel exhaustion). A Bool decision that reaches `Stuck` is simply
    /// undecided, so `native_decide` declines — fail-safe.
    Stuck,
}

/// The canonical name of a binary Int primitive, if `n` is one.
fn prim_name(n: &str) -> Option<&'static str> {
    match n {
        "add" => Some("add"),
        "sub" => Some("sub"),
        "mul" => Some("mul"),
        "div" => Some("div"),
        "mod" => Some("mod"),
        "le" => Some("le"),
        "lt" => Some("lt"),
        "ge" => Some("ge"),
        "gt" => Some("gt"),
        _ => None,
    }
}

/// The outcome of a binary Int primitive on two literals.
enum PrimResult {
    Int(Literal),
    Bool(bool),
    Stuck,
}

/// The pure arithmetic/comparison of a binary Int primitive on two literals — the SAME checked
/// semantics as the kernel reduction's `try_primitive_reduce` (overflow and division by zero
/// are `Stuck`, never wrong; a fast i64 path with an exact BigInt fallback for K6). Shared by
/// the tree-walker [`prim_compute`] and the compiled [`cprim_compute`], so both agree with
/// `normalize` — including on big numbers.
fn prim_arith(name: &str, al: &Literal, bl: &Literal) -> PrimResult {
    if let (Literal::Int(x), Literal::Int(y)) = (al, bl) {
        let fast = match name {
            "add" => x.checked_add(*y),
            "sub" => x.checked_sub(*y),
            "mul" => x.checked_mul(*y),
            "div" => x.checked_div(*y),
            "mod" => x.checked_rem(*y),
            _ => None,
        };
        if let Some(r) = fast {
            return PrimResult::Int(Literal::Int(r));
        }
        match name {
            "le" => return PrimResult::Bool(x <= y),
            "lt" => return PrimResult::Bool(x < y),
            "ge" => return PrimResult::Bool(x >= y),
            "gt" => return PrimResult::Bool(x > y),
            _ => {}
        }
    }
    let (Some(xb), Some(yb)) = (lit_bigint(al), lit_bigint(bl)) else {
        return PrimResult::Stuck;
    };
    let big = match name {
        "add" => Some(xb.add(&yb)),
        "sub" => Some(xb.sub(&yb)),
        "mul" => Some(xb.mul(&yb)),
        "div" => xb.div_rem(&yb).map(|(q, _)| q),
        "mod" => xb.div_rem(&yb).map(|(_, r)| r),
        _ => None,
    };
    if let Some(r) = big {
        return PrimResult::Int(int_lit(r));
    }
    match name {
        "le" => PrimResult::Bool(xb <= yb),
        "lt" => PrimResult::Bool(xb < yb),
        "ge" => PrimResult::Bool(xb >= yb),
        "gt" => PrimResult::Bool(xb > yb),
        _ => PrimResult::Stuck,
    }
}

/// Compute a fully-applied Int primitive over tree-walker [`Val`]s.
fn prim_compute(name: &str, a: &Val, b: &Val) -> Val {
    let (Val::Lit(al), Val::Lit(bl)) = (a, b) else {
        return Val::Stuck;
    };
    match prim_arith(name, al, bl) {
        PrimResult::Int(l) => Val::Lit(l),
        PrimResult::Bool(b) => Val::Ctor(if b { "true" } else { "false" }.to_string(), Vec::new()),
        PrimResult::Stuck => Val::Stuck,
    }
}

const FUEL: u64 = 50_000_000;

/// Evaluate a closed Bool term to `true`/`false`, or `None` if it does not reduce to a Bool
/// constructor within the fuel budget (fail-safe).
///
/// FAST PATH: the decision is first COMPILED to a tree of native closures and run
/// ([`native_compile_decide`]) — this handles the full computational fragment (recursion via
/// `Fix`, pattern matching via `Match`, δ-unfolding of definitions), so a recursive Bool
/// decision runs as closure calls with no `Term`-tree walking. Anything the compiler declines
/// falls back to the tree-walking interpreter [`eval_bool_tree`]. The two are differential-
/// tested to give IDENTICAL results, so the trusted `reduceBool` reduction and `native_decide`
/// both get the compiled speedup with no change in meaning.
pub fn eval_bool(ctx: &Context, term: &Term) -> Option<bool> {
    if let Some(b) = native_compile_decide(ctx, term) {
        return Some(b);
    }
    eval_bool_tree(ctx, term)
}

/// The tree-walking interpreter decision — walks the `Term` each step. The fallback for
/// [`eval_bool`] and the differential oracle the compiled path is validated against.
pub fn eval_bool_tree(ctx: &Context, term: &Term) -> Option<bool> {
    let mut fuel = FUEL;
    match eval(ctx, &Rc::new(Scope::Empty), term, &mut fuel) {
        Val::Ctor(c, _) if c == "true" => Some(true),
        Val::Ctor(c, _) if c == "false" => Some(false),
        _ => None,
    }
}

/// Build a `native_decide` proof of `prop` from a `Decidable prop` instance `inst`: if the
/// decision procedure NATIVELY evaluates to `true`, return the proof term
/// `of_decide_eq_true prop inst (ofReduceBool (decide prop inst) true (refl Bool true))`,
/// which the (main) kernel checks by running this evaluator via the `reduceBool` hook —
/// never by re-normalizing the decision procedure. Returns `None` (fail-safe) when the
/// decision is not `true`, so it can only ever produce proofs of genuinely-true goals.
pub fn native_decide(ctx: &Context, prop: &Term, inst: &Term) -> Option<Term> {
    let g = |s: &str| Term::Global(s.to_string());
    let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
    let decide_app = ap(ap(g("decide"), prop.clone()), inst.clone());
    if eval_bool(ctx, &decide_app) != Some(true) {
        return None;
    }
    let refl_true = ap(ap(g("refl"), g("Bool")), g("true"));
    let of_reduce = ap(ap(ap(g("ofReduceBool"), decide_app), g("true")), refl_true);
    Some(ap(ap(ap(g("of_decide_eq_true"), prop.clone()), inst.clone()), of_reduce))
}

fn lookup(scope: &Rc<Scope>, name: &str) -> Option<Val> {
    let mut s = scope;
    loop {
        match s.as_ref() {
            Scope::Empty => return None,
            Scope::Bind(n, v, rest) => {
                if n == name {
                    return Some(v.clone());
                }
                s = rest;
            }
        }
    }
}

/// Count the leading `Π`s of a constructor whose domain is a `Sort` — the inductive's
/// parameters, when it did not declare a split (mirrors `reduction::count_type_params`).
fn count_type_params(ty: &Term) -> usize {
    let mut n = 0;
    let mut cur = ty;
    while let Term::Pi { param_type, body_type, .. } = cur {
        if matches!(param_type.as_ref(), Term::Sort(_)) {
            n += 1;
            cur = body_type;
        } else {
            break;
        }
    }
    n
}

fn eval(ctx: &Context, scope: &Rc<Scope>, term: &Term, fuel: &mut u64) -> Val {
    if *fuel == 0 {
        return Val::Stuck;
    }
    *fuel -= 1;
    match term {
        Term::Var(n) => lookup(scope, n).unwrap_or(Val::Stuck),
        Term::Global(n) => {
            if let Some(body) = ctx.get_definition_body(n) {
                // δ-reduction: definitions are closed, so evaluate in the empty environment.
                eval(ctx, &Rc::new(Scope::Empty), body, fuel)
            } else if ctx.is_constructor(n) {
                Val::Ctor(n.clone(), Vec::new())
            } else if let Some(p) = prim_name(n) {
                Val::Prim(p, Vec::new())
            } else {
                Val::Stuck // inductive type, axiom, or other opaque global
            }
        }
        Term::Lambda { param, body, .. } => {
            Val::Clos(Rc::new((param.clone(), (**body).clone())), scope.clone())
        }
        Term::Fix { name, body } => {
            Val::Fix(Rc::new((name.clone(), (**body).clone())), scope.clone())
        }
        Term::App(f, a) => {
            let fv = eval(ctx, scope, f, fuel);
            let av = eval(ctx, scope, a, fuel);
            apply(ctx, fv, av, fuel)
        }
        Term::Match { discriminant, cases, .. } => {
            match eval(ctx, scope, discriminant, fuel) {
                Val::Ctor(c, args) => {
                    // ι-reduction: select the case for `c`, applied to its VALUE arguments
                    // (dropping the inductive's leading parameters).
                    let ind = match ctx.constructor_inductive(&c) {
                        Some(i) => i,
                        None => return Val::Stuck,
                    };
                    let ctors = ctx.get_constructors(ind);
                    let (idx, ctor_ty) = match ctors.iter().position(|(cn, _)| *cn == c) {
                        Some(i) => (i, ctors[i].1.clone()),
                        None => return Val::Stuck,
                    };
                    if idx >= cases.len() {
                        return Val::Stuck;
                    }
                    let nparams = ctx
                        .inductive_declared_params(ind)
                        .unwrap_or_else(|| count_type_params(&ctor_ty));
                    let value_args: &[Val] = if nparams < args.len() { &args[nparams..] } else { &[] };
                    let mut sel = eval(ctx, scope, &cases[idx], fuel);
                    for va in value_args {
                        sel = apply(ctx, sel, va.clone(), fuel);
                    }
                    sel
                }
                _ => Val::Stuck,
            }
        }
        Term::Lit(l) => Val::Lit(l.clone()),
        // Sorts, Πs, holes, universe-poly consts: not part of a Bool computation.
        _ => Val::Stuck,
    }
}

fn apply(ctx: &Context, fv: Val, av: Val, fuel: &mut u64) -> Val {
    if *fuel == 0 {
        return Val::Stuck;
    }
    match fv {
        Val::Clos(pb, cenv) => {
            let ext = Rc::new(Scope::Bind(pb.0.clone(), av, cenv));
            eval(ctx, &ext, &pb.1, fuel)
        }
        Val::Ctor(c, mut args) => {
            args.push(av);
            Val::Ctor(c, args)
        }
        Val::Prim(name, mut args) => {
            args.push(av);
            if args.len() < 2 {
                Val::Prim(name, args)
            } else {
                prim_compute(name, &args[0], &args[1])
            }
        }
        Val::Fix(rb, fenv) => {
            // Unfold only when the argument is in constructor form — matches the kernel's
            // guarded `Fix` reduction and guarantees progress on kernel-checked terms.
            if matches!(av, Val::Ctor(..)) {
                let ext = Rc::new(Scope::Bind(rb.0.clone(), Val::Fix(rb.clone(), fenv.clone()), fenv));
                let unfolded = eval(ctx, &ext, &rb.1, fuel);
                apply(ctx, unfolded, av, fuel)
            } else {
                Val::Stuck
            }
        }
        // A literal is not a function.
        Val::Lit(_) => Val::Stuck,
        Val::Stuck => Val::Stuck,
    }
}

// ---------------------------------------------------------------------------
// Native closure-compiled decisions (N) — compile a ground Bool decision ONCE
// into a tree of rustc-compiled native closures and run it, with NO Term-tree
// walking at run time. The closures ARE machine code (rustc compiled their
// bodies), so this is genuine "compile the decision to native code and run it",
// the essence of `native_decide` — sound via the same `ofReduceBool` gate that
// wraps [`native_decide`]. Reused decisions pay the compile cost once.
// ---------------------------------------------------------------------------

type IntThunk = Box<dyn Fn() -> i128>;
type BoolThunk = Box<dyn Fn() -> bool>;

fn as_binop(term: &Term) -> Option<(&str, &Term, &Term)> {
    if let Term::App(fx, y) = term {
        if let Term::App(op, x) = fx.as_ref() {
            if let Term::Global(name) = op.as_ref() {
                return Some((name.as_str(), x.as_ref(), y.as_ref()));
            }
        }
    }
    None
}

/// Compile a ground integer expression (`Int`/`BigInt` literals, `add`/`sub`/`mul`) into a
/// native closure. `None` if the expression falls outside the compilable fragment.
fn compile_int(term: &Term) -> Option<IntThunk> {
    match term {
        Term::Lit(Literal::Int(n)) => {
            let n = *n as i128;
            Some(Box::new(move || n))
        }
        Term::Lit(Literal::BigInt(n)) => {
            let n = n.to_i64().map(|v| v as i128)?;
            Some(Box::new(move || n))
        }
        _ => {
            let (op, x, y) = as_binop(term)?;
            let cx = compile_int(x)?;
            let cy = compile_int(y)?;
            match op {
                "add" => Some(Box::new(move || cx() + cy())),
                "sub" => Some(Box::new(move || cx() - cy())),
                "mul" => Some(Box::new(move || cx() * cy())),
                _ => None,
            }
        }
    }
}

/// Compile a ground Bool decision (`true`/`false`, comparisons of compiled integer
/// expressions, and `&&`/`||`-shaped `and`/`or`) into a native closure.
fn compile_bool(term: &Term) -> Option<BoolThunk> {
    match term {
        Term::Global(n) if n == "true" => Some(Box::new(|| true)),
        Term::Global(n) if n == "false" => Some(Box::new(|| false)),
        _ => {
            let (op, x, y) = as_binop(term)?;
            match op {
                "le" | "lt" | "ge" | "gt" => {
                    let cx = compile_int(x)?;
                    let cy = compile_int(y)?;
                    Some(match op {
                        "le" => Box::new(move || cx() <= cy()),
                        "lt" => Box::new(move || cx() < cy()),
                        "ge" => Box::new(move || cx() >= cy()),
                        _ => Box::new(move || cx() > cy()),
                    })
                }
                "and" | "or" => {
                    let cx = compile_bool(x)?;
                    let cy = compile_bool(y)?;
                    Some(if op == "and" {
                        Box::new(move || cx() && cy())
                    } else {
                        Box::new(move || cx() || cy())
                    })
                }
                _ => None,
            }
        }
    }
}

/// Compile a ground Bool decision into native closures and run it — `None` if the term is
/// outside the compilable fragment (the caller falls back to the [`eval_bool`] interpreter).
/// Semantics are IDENTICAL to `eval_bool`/`normalize` (differential-tested), so the
/// `ofReduceBool` certificate `native_decide` builds is equally sound.
pub fn native_compile_bool(term: &Term) -> Option<bool> {
    compile_bool(term).map(|c| c())
}

// ---------------------------------------------------------------------------
// Native RECURSOR-aware compiled decisions (N) — the general backend. The
// arithmetic `compile_bool` above handles only ground ×/+/comparisons; this
// compiles the FULL computational fragment — variables, λ, `fix`, application,
// `match`, and δ-unfolding of definitions — to a tree of native closures, so a
// RECURSIVE Bool decision (e.g. `even 1000`, a list predicate) runs as closure
// calls with NO `Term`-tree walking. Each `Term` shape is dispatched ONCE at
// compile time; the closures ARE the compiled decision. Semantics are IDENTICAL
// to the tree-walking `eval` (differential-tested against it and `normalize`),
// so the `reduceBool` hook that consumes it stays sound.
// ---------------------------------------------------------------------------

/// A run-time environment for the compiled evaluator (bound variables → values).
enum CScope {
    Empty,
    Bind(Rc<str>, CVal, Rc<CScope>),
}

/// A compiled run-time value. `Fun`/`Fix` carry COMPILED bodies (closures), so applying them
/// is a closure call, not a `Term` walk — the essence of the compiled backend.
#[derive(Clone)]
enum CVal {
    /// A constructor applied to all its arguments (parameters + value args).
    Ctor(Rc<str>, Rc<Vec<CVal>>),
    /// A compiled `λ`: apply it to an argument to run its compiled body.
    Fun(Rc<dyn Fn(CVal, &mut u64) -> CVal>),
    /// A compiled `fix`: its compiled body, recursive-binder name, and captured environment.
    Fix(Rc<FixClosure>),
    Lit(Literal),
    /// A partially-applied Int primitive accumulating its two arguments.
    Prim(&'static str, Rc<Vec<CVal>>),
    Stuck,
}

/// The captured pieces of a compiled `fix`, unfolded on application (in [`capply`]).
struct FixClosure {
    body: Code,
    name: Rc<str>,
    env: Rc<CScope>,
}

/// A compiled term: a closure from an environment (and a fuel budget) to a value.
type Code = Rc<dyn Fn(&Rc<CScope>, &mut u64) -> CVal>;

fn clookup(scope: &Rc<CScope>, name: &str) -> CVal {
    let mut s = scope;
    loop {
        match s.as_ref() {
            CScope::Empty => return CVal::Stuck,
            CScope::Bind(n, v, rest) => {
                if n.as_ref() == name {
                    return v.clone();
                }
                s = rest;
            }
        }
    }
}

/// Compute a fully-applied Int primitive over compiled [`CVal`]s — same [`prim_arith`] as the
/// tree-walker, so the compiled path agrees with `eval`/`normalize`.
fn cprim_compute(name: &str, a: &CVal, b: &CVal) -> CVal {
    let (CVal::Lit(al), CVal::Lit(bl)) = (a, b) else {
        return CVal::Stuck;
    };
    match prim_arith(name, al, bl) {
        PrimResult::Int(l) => CVal::Lit(l),
        PrimResult::Bool(b) => CVal::Ctor(Rc::from(if b { "true" } else { "false" }), Rc::new(Vec::new())),
        PrimResult::Stuck => CVal::Stuck,
    }
}

/// Apply a compiled value to an argument — the run-time counterpart of the tree-walker's
/// [`apply`], with the SAME rules: a `fix` unfolds only on a constructor argument (matching the
/// kernel's guarded `Fix` reduction), a partial primitive accumulates then computes. Fuel bounds
/// the reduction so a non-terminating (never kernel-checked) input fails safe to `Stuck`.
fn capply(f: CVal, arg: CVal, fuel: &mut u64) -> CVal {
    if *fuel == 0 {
        return CVal::Stuck;
    }
    *fuel -= 1;
    match f {
        CVal::Fun(func) => func(arg, fuel),
        CVal::Ctor(c, args) => {
            let mut v = (*args).clone();
            v.push(arg);
            CVal::Ctor(c, Rc::new(v))
        }
        CVal::Prim(name, args) => {
            let mut v = (*args).clone();
            v.push(arg);
            if v.len() < 2 {
                CVal::Prim(name, Rc::new(v))
            } else {
                cprim_compute(name, &v[0], &v[1])
            }
        }
        CVal::Fix(fx) => {
            if matches!(arg, CVal::Ctor(..)) {
                let self_val = CVal::Fix(fx.clone());
                let ext = Rc::new(CScope::Bind(fx.name.clone(), self_val, fx.env.clone()));
                let unfolded = (fx.body)(&ext, fuel);
                capply(unfolded, arg, fuel)
            } else {
                CVal::Stuck
            }
        }
        CVal::Lit(_) | CVal::Stuck => CVal::Stuck,
    }
}

/// Compiles `Term`s to [`Code`]. Holds the constructor→(index, params) table (so a compiled
/// `Match` selects a case without touching the `Context` at run time) and a per-definition
/// SLOT map: a definition's body is compiled once into a shared cell that recursive and mutual
/// references read at run time — tying the knot without infinite compile-time recursion.
struct Compiler<'c> {
    ctx: &'c Context,
    ctor_map: Rc<HashMap<String, (usize, usize)>>,
    slots: RefCell<HashMap<String, Rc<RefCell<Option<Code>>>>>,
}

impl<'c> Compiler<'c> {
    fn new(ctx: &'c Context) -> Self {
        // Precompute, for every registered constructor, its index within its inductive and that
        // inductive's parameter count — the exact `(case index, value-arg offset)` a `Match`
        // needs, resolved once here instead of per reduction.
        let mut ctor_map = HashMap::new();
        let inductives: Vec<String> = ctx.iter_inductives().map(|(n, _)| n.to_string()).collect();
        for ind in &inductives {
            let declared = ctx.inductive_declared_params(ind);
            for (idx, (cname, cty)) in ctx.get_constructors(ind).iter().enumerate() {
                let nparams = declared.unwrap_or_else(|| count_type_params(cty));
                ctor_map.entry(cname.to_string()).or_insert((idx, nparams));
            }
        }
        Compiler { ctx, ctor_map: Rc::new(ctor_map), slots: RefCell::new(HashMap::new()) }
    }

    fn compile(&self, term: &Term) -> Code {
        match term {
            Term::Var(n) => {
                let n: Rc<str> = Rc::from(n.as_str());
                Rc::new(move |env, _| clookup(env, &n))
            }
            Term::Lit(l) => {
                let l = l.clone();
                Rc::new(move |_, _| CVal::Lit(l.clone()))
            }
            Term::Global(n) => self.compile_global(n),
            Term::Lambda { param, body, .. } => {
                let param: Rc<str> = Rc::from(param.as_str());
                let cbody = self.compile(body);
                Rc::new(move |env, _| {
                    let env = env.clone();
                    let cbody = cbody.clone();
                    let param = param.clone();
                    CVal::Fun(Rc::new(move |arg, fuel| {
                        let ext = Rc::new(CScope::Bind(param.clone(), arg, env.clone()));
                        cbody(&ext, fuel)
                    }))
                })
            }
            Term::Fix { name, body } => {
                let name: Rc<str> = Rc::from(name.as_str());
                let cbody = self.compile(body);
                Rc::new(move |env, _| {
                    CVal::Fix(Rc::new(FixClosure {
                        body: cbody.clone(),
                        name: name.clone(),
                        env: env.clone(),
                    }))
                })
            }
            Term::App(f, a) => {
                let cf = self.compile(f);
                let ca = self.compile(a);
                Rc::new(move |env, fuel| {
                    let fv = cf(env, fuel);
                    let av = ca(env, fuel);
                    capply(fv, av, fuel)
                })
            }
            Term::Match { discriminant, cases, .. } => {
                let cd = self.compile(discriminant);
                let ccases: Vec<Code> = cases.iter().map(|c| self.compile(c)).collect();
                let ctor_map = self.ctor_map.clone();
                Rc::new(move |env, fuel| match cd(env, fuel) {
                    CVal::Ctor(c, args) => {
                        let (idx, nparams) = match ctor_map.get(c.as_ref()) {
                            Some(p) => *p,
                            None => return CVal::Stuck,
                        };
                        if idx >= ccases.len() {
                            return CVal::Stuck;
                        }
                        // ι-reduction: the case is run in the CURRENT environment (it may use
                        // outer variables), then applied to the constructor's VALUE arguments.
                        let mut sel = ccases[idx](env, fuel);
                        let value_args: &[CVal] =
                            if nparams < args.len() { &args[nparams..] } else { &[] };
                        for va in value_args {
                            sel = capply(sel, va.clone(), fuel);
                        }
                        sel
                    }
                    _ => CVal::Stuck,
                })
            }
            // `MutualFix` / `Let` / sorts / `Π` / holes / universe-poly consts are not part of
            // the compiled Bool fragment (the tree-walker declines them too); a `Stuck` node
            // makes `eval_bool` fall back to the interpreter, preserving identical results.
            _ => Rc::new(|_, _| CVal::Stuck),
        }
    }

    fn compile_global(&self, n: &str) -> Code {
        if self.ctx.get_definition_body(n).is_some() {
            // δ-reduction: compile the (closed) body once into a shared slot; a recursive or
            // mutual reference reads the same slot at run time, so the knot is tied without
            // infinite compile-time recursion. The body runs in the EMPTY environment.
            let slot = self.def_slot(n);
            Rc::new(move |_env, fuel| match slot.borrow().clone() {
                Some(code) => code(&Rc::new(CScope::Empty), fuel),
                None => CVal::Stuck,
            })
        } else if self.ctx.is_constructor(n) {
            let n: Rc<str> = Rc::from(n);
            Rc::new(move |_, _| CVal::Ctor(n.clone(), Rc::new(Vec::new())))
        } else if let Some(p) = prim_name(n) {
            Rc::new(move |_, _| CVal::Prim(p, Rc::new(Vec::new())))
        } else {
            // Inductive type, axiom, or other opaque global.
            Rc::new(|_, _| CVal::Stuck)
        }
    }

    /// Get-or-create the shared compiled-body slot for definition `n`, compiling its body once.
    /// The slot is registered BEFORE its body is compiled, so a self/mutual reference during
    /// compilation resolves to the same (initially empty) cell and reads it at run time.
    fn def_slot(&self, n: &str) -> Rc<RefCell<Option<Code>>> {
        if let Some(s) = self.slots.borrow().get(n) {
            return s.clone();
        }
        let s = Rc::new(RefCell::new(None));
        self.slots.borrow_mut().insert(n.to_string(), s.clone());
        let body = self.ctx.get_definition_body(n).expect("def_slot on a definition").clone();
        let cbody = self.compile(&body);
        *s.borrow_mut() = Some(cbody);
        s
    }
}

/// Compile a closed Bool decision to native closures and run it — `None` if it does not reduce
/// to a Bool constructor (fail-safe; the caller falls back to the tree-walking interpreter).
/// Unlike [`native_compile_bool`], this handles the FULL computational fragment (recursion,
/// pattern matching, definitions), so recursor-based decisions compile to native code. Its
/// verdict is IDENTICAL to [`eval_bool_tree`]/`normalize` (differential-tested).
pub fn native_compile_decide(ctx: &Context, term: &Term) -> Option<bool> {
    let compiler = Compiler::new(ctx);
    let code = compiler.compile(term);
    let mut fuel = FUEL;
    match code(&Rc::new(CScope::Empty), &mut fuel) {
        CVal::Ctor(c, _) if c.as_ref() == "true" => Some(true),
        CVal::Ctor(c, _) if c.as_ref() == "false" => Some(false),
        _ => None,
    }
}
