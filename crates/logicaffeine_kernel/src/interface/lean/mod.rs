//! Lean 4 kernel export (NDJSON 3.1) as an input surface of the LOGOS kernel.
//!
//! This is one more reader beside `term_parser`, `command_parser` and `literate_parser`: it builds
//! LOGOS kernel terms from Lean's exported declarations and hands every declaration to the kernel,
//! which re-decides it. The reader is untrusted; its verdicts are the kernel's. Unsupported
//! declaration families are declined, never counted as invalid proofs. User names are encoded
//! structurally. Nat and quotient identities are reserved and admitted only through their checked
//! canonical declarations.
use crate::{
    defeq_for_test as definitionally_equal, infer_type, normalize_checked, Context, KernelError,
    Term, Universe,
};
use serde_json::Value;
mod diagnostic;
pub use diagnostic::{DiagnosticCode, Outcome, Reason};
pub mod emit;
pub mod intake;
pub mod native_decide;
pub mod package;
pub mod pipeline;
pub mod receipt;
pub mod reference;
pub mod taint;
pub use pipeline::run_cli;
mod beta;
mod dag;
mod metadata;
// Experimental trace controls do not alter importer production behavior.
#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/lazy_conversion.rs"]
mod lazy_conversion;
mod strings;
use std::collections::{HashMap, HashSet};

/// Charge decimal input and an upper bound on the existing BigInt parser's
/// limb work, rather than placing a machine-word bound on the represented Nat.
/// Each decimal digit needs fewer than four bits; multiplication by ten and
/// addition each visit at most (digits / 16 + 1) 64-bit limbs.
fn parse_natural_literal(digits: &str, budget: &mut usize) -> Result<logicaffeine_base::BigInt> {
    *budget = budget.checked_sub(digits.len()).ok_or_else(|| {
        declined(
            DiagnosticCode::ResourceLimit,
            "natural literal parsing budget",
        )
    })?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(declined(
            DiagnosticCode::MalformedExport,
            "malformed natural literal",
        ));
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Ok(logicaffeine_base::BigInt::zero());
    }
    let n = significant.len();
    let work = n.saturating_mul(n / 16 + 1).saturating_mul(2);
    *budget = budget.checked_sub(work).ok_or_else(|| {
        declined(
            DiagnosticCode::ResourceLimit,
            "natural literal parsing budget",
        )
    })?;
    logicaffeine_base::BigInt::parse_decimal(significant)
        .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "malformed natural literal"))
}

/// The kernel's decision on an input it did not accept. `Reject`: the input is wrong. `Decline`: the
/// input may be right, but this reader does not accept it (and never approximates it).
#[derive(Debug)]
pub enum Verdict {
    Reject(Reason),
    Decline(Reason),
}
impl Verdict {
    pub fn reason(&self) -> &Reason {
        match self {
            Self::Reject(reason) | Self::Decline(reason) => reason,
        }
    }
    /// The benchmark protocol's exit code: 1 for a rejection, 2 for a decline.
    pub fn code(&self) -> i32 {
        match self {
            Self::Reject(_) => 1,
            Self::Decline(_) => 2,
        }
    }
    /// The `logos-lean-result/1` outcome this refusal decides.
    pub fn outcome(&self) -> Outcome {
        Outcome::of_refusal(self.reason().code(), matches!(self, Self::Reject(_)))
    }
    /// The same verdict, recorded as arising at 1-based input `line` (the innermost line wins).
    pub fn at_line(self, line: usize) -> Verdict {
        self.map_reason(|reason| reason.at_line(line))
    }
    /// The same verdict, recorded as arising in the Lean declaration `name` (the innermost wins).
    pub fn in_declaration(self, name: impl Into<String>) -> Verdict {
        let name = name.into();
        self.map_reason(|reason| reason.in_declaration(name))
    }
    fn map_reason(self, f: impl FnOnce(Reason) -> Reason) -> Verdict {
        match self {
            Verdict::Reject(reason) => Verdict::Reject(f(reason)),
            Verdict::Decline(reason) => Verdict::Decline(f(reason)),
        }
    }
}
fn emit_stage(
    sink: Option<StageSink<'_>>,
    declaration: Option<&(usize, String, String, Option<u64>)>,
    stage: &str,
    event: &str,
) {
    if let (Some(sink), Some((line, name, kind, name_index))) = (sink, declaration) {
        sink(&serde_json::json!({
            "schema":"logos-lean-stages/1","line":line,"name":name,
            "kind":kind,"name_index":name_index,"stage":stage,"event":event,
        }));
    }
}
type Result<T> = std::result::Result<T, Verdict>;
impl From<KernelError> for Verdict {
    fn from(error: KernelError) -> Self {
        kernel(error)
    }
}

fn declined(code: DiagnosticCode, s: impl Into<String>) -> Verdict {
    Verdict::Decline(Reason::new(code, s))
}
fn rejected(code: DiagnosticCode, s: impl Into<String>) -> Verdict {
    Verdict::Reject(Reason::new(code, s))
}
impl Verdict {
    /// The same verdict and code, its message prefixed with where it arose.
    fn located(self, prefix: impl std::fmt::Display) -> Verdict {
        self.map_reason(|reason| reason.located(prefix))
    }
}
fn index(v: &Value) -> Result<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "malformed index"))
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array()
        .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "expected array"))
}
/// A verdict the independent kernel did not give, as the reader reports it — always at the
/// `independent_recheck` stage. A resource bound that stopped it is a resource limit; a refutation,
/// or a construct the independent kernel does not cover, is a refused recheck.
fn independent_verdict(error: &crate::ReCheckError, what: impl std::fmt::Display) -> Verdict {
    let code = match error.incomplete() {
        Some(incomplete) if incomplete.is_resource_limit() => DiagnosticCode::ResourceLimit,
        _ => DiagnosticCode::RecheckRefused,
    };
    Verdict::Decline(
        Reason::new(code, format!("{what}: {error:?}")).at_stage("independent_recheck"),
    )
}
/// The two kernels' types for one declaration, compared by Lean's exact conversion. A comparison
/// the conversion could not finish is kept as the resource limit it is; any other failure is a
/// refused recheck.
fn independent_comparison(compared: Result<()>, what: impl std::fmt::Display) -> Result<()> {
    compared.map_err(|verdict| match verdict {
        Verdict::Decline(reason) if reason.code() == DiagnosticCode::ResourceLimit => {
            Verdict::Decline(reason.located(what).at_stage("independent_recheck"))
        }
        _ => declined(DiagnosticCode::RecheckRefused, what.to_string()),
    })
}
fn kernel(e: KernelError) -> Verdict {
    match e {
        KernelError::IndependentRecheck { subject, error } => {
            independent_verdict(&error, format!("Certification error: {subject}"))
        }
        KernelError::UnboundVariable(_)
        | KernelError::NotAFunction(_)
        | KernelError::NotAType(_)
        | KernelError::TypeMismatch { .. }
        | KernelError::PositivityViolation { .. }
        | KernelError::InvalidDeclaration(_) => {
            rejected(DiagnosticCode::InvalidProof, e.to_string())
        }
        _ => declined(DiagnosticCode::UnsupportedFeature, e.to_string()),
    }
}
/// An inductive the kernel cannot decide — a nested-reconstruction boundary, an inconclusive
/// positivity check — is an unsupported feature, declined and never approximated by an axiom;
/// a negative occurrence and every other kernel refusal keep their classification.
fn inductive_verdict(error: KernelError) -> Verdict {
    match error {
        KernelError::CertificationError(message) => {
            declined(DiagnosticCode::UnsupportedFeature, message)
        }
        other => kernel(other),
    }
}
/// An unsafe inductive is valid Lean outside the safe kernel fragment; a missing or
/// non-Boolean flag is a malformed record.
fn inductive_safety(flag: &Value, message: &str) -> Verdict {
    match flag {
        Value::Bool(true) => declined(DiagnosticCode::UnsupportedFeature, message),
        _ => declined(DiagnosticCode::MalformedExport, message),
    }
}
fn normal(ctx: &Context, t: &Term) -> Result<Term> {
    normalize_checked(ctx, t).map_err(|e| {
        let code = match e.1 {
            crate::Unreduced::Cyclic => DiagnosticCode::UnsupportedFeature,
            crate::Unreduced::BudgetSpent | crate::Unreduced::TooDeep => {
                DiagnosticCode::ResourceLimit
            }
        };
        declined(code, format!("normalization unfinished: {e:?}"))
    })
}
fn sort(ctx: &Context, t: &Term) -> Result<Universe> {
    match normal(ctx, &infer_type(ctx, t).map_err(kernel)?)? {
        Term::Sort(u) => Ok(u),
        _ => Err(rejected(
            DiagnosticCode::InvalidProof,
            "declaration type is not a type",
        )),
    }
}

fn exact(ctx: &Context, a: &Term, b: &Term) -> Result<()> {
    // A conversion that answers "not equal" after one of its own reductions stopped for want of a
    // resource compared a partial term: that is an undecided comparison — a resource limit, never
    // evidence that valid Lean is an invalid proof. The conversion reports the stop itself, from
    // inside (`observe_resource_stops`). It is never reconstructed afterwards by normalizing both
    // whole sides again: that costs more than the conversion it audits, and it cannot see what the
    // conversion compared — Arena `refute-cheap-first` is refuted at `false` against `true`
    // without `count #200` ever being evaluated, and reported undecided by a whole-side audit.
    let (equal, stopped) =
        crate::reduction::observe_resource_stops(|| definitionally_equal(ctx, a, b));
    if equal {
        return Ok(());
    }
    Err(match stopped {
        Some(reason) => declined(
            DiagnosticCode::ResourceLimit,
            format!("Lean conversion undecided: reduction stopped ({reason:?})"),
        ),
        None => rejected(
            DiagnosticCode::InvalidProof,
            "Lean conversion mismatch (cumulative subtyping is insufficient)",
        ),
    })
}
/// How many applied functions' types the reader normalized in full to find their `Π`.
#[cfg(test)]
static FUNCTION_TYPE_NORMALIZATIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Keep the host kernel's reduction, substitution and typing. This traversal
// strengthens its cumulative application/let checks to Lean's exact conversion.
pub fn lean_type(ctx: &Context, t: &Term) -> Result<Term> {
    use crate::term_store::{deepen_stack, within_kernel_depth, KernelCall, TermStore, TypeCache};
    if let Some(result) = crate::annotation::erased(t, |t| lean_type(ctx, t)) {
        return result;
    }
    if !within_kernel_depth(t) {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "Lean typing input exceeds the kernel depth limit",
        ));
    }
    // Size the stack before adapter recursion retains telescope frames. Caches
    // belong to the worker call and never cross the scoped-thread boundary.
    if let Some(result) = deepen_stack(t, || lean_type(ctx, t)) {
        return result;
    }
    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let mut cache = TypeCache::new(&call);
    let mut equal = HashMap::new();
    lean_type_cached(ctx, t, &mut store, &mut cache, &mut equal, &call, &[])
}

// The root context is immutable for this call. The environment key records
// every extension made below it, including shadowing, rather than only the
// expression's directly mentioned variables (types can depend on other locals).
fn lean_type_cached<'call>(
    ctx: &Context,
    t: &Term,
    store: &mut crate::term_store::TermStore,
    cache: &mut crate::term_store::TypeCache<'call, Term>,
    equal: &mut HashMap<crate::term_store::EnvId, crate::term_store::ScopedDefEqCache<'call>>,
    call: &'call crate::term_store::KernelCall,
    locals: &[(logicaffeine_base::Symbol, crate::term_store::TermId)],
) -> Result<Term> {
    let id = store.intern(t);
    let env = store.env(locals);
    if let Some(ty) = cache.get(env, id) {
        return Ok(ty.clone());
    }
    let result = (|| {
        match t {
            Term::App(f, a) => {
                let ft = lean_type_cached(ctx, f, store, cache, equal, call, locals)?;
                // Only the head is demanded (Lean's `ensure_pi (whnf (infer f))`, and this
                // kernel's own application rule). Normalizing the whole type would unfold every
                // proof inside it — a `Fin` numeral carries its `Nat.mod_lt` proof, and theorems are
                // transparent — for a `Π` that head reduction already exposes.
                let ft = match crate::type_checker::exposed_function_type(ctx, ft) {
                    Ok(exposed) => exposed,
                    Err(ft) => {
                        #[cfg(test)]
                        FUNCTION_TYPE_NORMALIZATIONS
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        normal(ctx, &ft)?
                    }
                };
                let at = lean_type_cached(ctx, a, store, cache, equal, call, locals)?;
                if let Term::Pi {
                    param,
                    param_type,
                    body_type,
                } = ft
                {
                    lean_exact_cached(ctx, &at, &param_type, store, equal, call, env)?;
                    return Ok(crate::instantiate_codomain(
                        &param,
                        &param_type,
                        &body_type,
                        a,
                    ));
                }
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "application head is not a function",
                ));
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            }
            | Term::Lambda {
                param,
                param_type,
                body: body_type,
            } => {
                let domain_sort =
                    lean_type_cached(ctx, param_type, store, cache, equal, call, locals)?;
                if !matches!(normal(ctx, &domain_sort)?, Term::Sort(_)) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "binder domain is not a type",
                    ));
                }
                let name = store.symbol(param);
                let domain = store.intern(param_type);
                let mut inner = locals.to_vec();
                inner.retain(|(n, _)| *n != name);
                inner.push((name, domain));
                let body = lean_type_cached(
                    &ctx.extend(param, (**param_type).clone()),
                    body_type,
                    store,
                    cache,
                    equal,
                    call,
                    &inner,
                )?;
                if matches!(t, Term::Lambda { .. }) {
                    return Ok(Term::Pi {
                        param: param.clone(),
                        param_type: param_type.clone(),
                        body_type: Box::new(body),
                    });
                }
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                lean_type_cached(ctx, ty, store, cache, equal, call, locals)?;
                let actual = lean_type_cached(ctx, value, store, cache, equal, call, locals)?;
                lean_exact_cached(ctx, &actual, ty, store, equal, call, env)?;
                if ctx.get(name).is_none() {
                    std::thread_local! { static LET_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
                    struct Probe(bool);
                    impl Drop for Probe {
                        fn drop(&mut self) {
                            LET_PROBE.with(|flag| flag.set(self.0));
                        }
                    }
                    let probe = Probe(LET_PROBE.with(|flag| flag.replace(true)));
                    let nested = probe.0;
                    let symbol = store.symbol(name);
                    let domain = store.intern(ty);
                    let mut inner = locals.to_vec();
                    inner.push((symbol, domain));
                    let result = lean_type_cached(
                        &ctx.extend(name, (**ty).clone()),
                        body,
                        store,
                        cache,
                        equal,
                        call,
                        &inner,
                    );
                    drop(probe);
                    match result {
                        Ok(body_type) => {
                            return Ok(crate::instantiate_codomain(name, ty, &body_type, value))
                        }
                        Err(error) if nested => return Err(error),
                        Err(_) => {}
                    }
                }
                return lean_type_cached(
                    ctx,
                    &crate::substitute(body, name, value),
                    store,
                    cache,
                    equal,
                    call,
                    locals,
                );
            }
            _ => {}
        }
        infer_type(ctx, t).map_err(kernel)
    })()?;
    cache.insert(env, id, result.clone());
    Ok(result)
}

// Each environment owns a separate pair cache under the same immutable root.
// This records checked pairs only; it never infers transitive equivalence.
fn lean_exact_cached<'call>(
    ctx: &Context,
    a: &Term,
    b: &Term,
    store: &mut crate::term_store::TermStore,
    equal: &mut HashMap<crate::term_store::EnvId, crate::term_store::ScopedDefEqCache<'call>>,
    call: &'call crate::term_store::KernelCall,
    env: crate::term_store::EnvId,
) -> Result<()> {
    use crate::term_store::DefEqCache;
    let left = store.intern(a);
    let right = store.intern(b);
    if let Some(answer) = equal
        .entry(env)
        .or_insert_with(|| DefEqCache::scoped(call))
        .get(left, right)
    {
        return if answer {
            Ok(())
        } else {
            Err(rejected(
                DiagnosticCode::InvalidProof,
                "Lean conversion mismatch (cumulative subtyping is insufficient)",
            ))
        };
    }
    let answer = exact(ctx, a, b);
    // Only a DECIDED comparison is evidence. An undecided one is asked again next time, never
    // replayed from the cache as a rejection.
    let decided = match &answer {
        Ok(()) => Some(true),
        Err(Verdict::Reject(_)) => Some(false),
        Err(Verdict::Decline(_)) => None,
    };
    if let Some(decided) = decided {
        equal.get_mut(&env).unwrap().insert(left, right, decided);
    }
    answer
}

fn apply(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn telescope_len(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}
// Count binders without evaluating their domains or the arguments of an
// already-known inductive result. Normalize only a hidden telescope head.
fn resolved_telescope_len(ctx: &Context, term: &Term) -> Result<usize> {
    let mut ctx = ctx.clone();
    let mut term = term.clone();
    let mut count = 0;
    loop {
        if let Term::Pi {
            param,
            param_type,
            body_type,
        } = term
        {
            ctx = ctx.extend(&param, *param_type);
            term = *body_type;
            count += 1;
            continue;
        }
        let mut head = &term;
        while let Term::App(f, _) = head {
            head = f;
        }
        if matches!(head, Term::Global(n) | Term::Const {name:n, ..} if ctx.is_inductive(n))
            || matches!(head, Term::Sort(_))
        {
            return Ok(count);
        }
        term = normal(&ctx, &term)?;
        if !matches!(term, Term::Pi { .. }) {
            return Ok(count);
        }
    }
}
/// An inductive's arity as Lean declares it, with its telescope exposed (the kernel's
/// `exposed_arity`): binder domains keep `outParam` and every other written form.
fn declared_arity(ctx: &Context, arity: &Term) -> Result<Term> {
    crate::context::exposed_arity(ctx, arity, &mut |ctx, t| normal(ctx, t))
}
fn open_binder(t: Term, name: &str) -> Result<(Term, Term)> {
    if let Term::Pi {
        param,
        param_type,
        body_type,
    } = t
    {
        Ok((
            *param_type,
            crate::substitute(&body_type, &param, &Term::Var(name.into())),
        ))
    } else {
        Err(rejected(
            DiagnosticCode::InvalidProof,
            "short declaration telescope",
        ))
    }
}
fn ctor_rule(
    ctx: &Context,
    rec: &str,
    levels: &[String],
    rec_ty: &Term,
    ctor: &str,
    ctor_ty: &Term,
    nparams: usize,
    prefix: usize,
) -> Result<Term> {
    let ctor_value = match ctx.poly_declaration(ctor) {
        Some((parameters, _)) => Term::Const {
            name: ctor.into(),
            levels: parameters.iter().cloned().map(Universe::Var).collect(),
        },
        None => Term::Global(ctor.into()),
    };
    ctor_rule_applied(
        ctx,
        rec,
        levels,
        rec_ty,
        ctor_value,
        ctor_ty.clone(),
        nparams,
        prefix,
        nparams,
    )
}

fn ctor_rule_applied(
    ctx: &Context,
    rec: &str,
    levels: &[String],
    rec_ty: &Term,
    mut ctor_value: Term,
    mut cty: Term,
    nparams: usize,
    prefix: usize,
    result_parameters: usize,
) -> Result<Term> {
    let mut binders = Vec::new();
    let mut ty = rec_ty.clone();
    for i in 0..prefix {
        let name = format!("arena.rule.{i}");
        let (domain, body) = open_binder(ty, &name)?;
        binders.push((name, domain));
        ty = body;
    }
    for (name, _) in binders.iter().take(nparams) {
        let variable = Term::Var(name.clone());
        if let Term::Pi {
            param, body_type, ..
        } = cty
        {
            cty = crate::substitute(&body_type, &param, &variable);
        } else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "constructor parameter telescope is too short",
            ));
        }
        ctor_value = apply(ctor_value, variable);
    }
    while matches!(cty, Term::Pi { .. }) {
        let name = format!("arena.rule.{}", binders.len());
        let (domain, body) = open_binder(cty, &name)?;
        ctor_value = apply(ctor_value, Term::Var(name.clone()));
        binders.push((name, domain));
        cty = body;
    }
    let mut result_args = Vec::new();
    let mut head = &cty;
    while let Term::App(f, a) = head {
        result_args.push((**a).clone());
        head = f;
    }
    result_args.reverse();
    let mut rhs = if levels.is_empty() {
        Term::Global(rec.into())
    } else {
        Term::Const {
            name: rec.into(),
            levels: levels.iter().cloned().map(Universe::Var).collect(),
        }
    };
    for (name, _) in binders.iter().take(prefix) {
        rhs = apply(rhs, Term::Var(name.clone()));
    }
    for index in result_args.into_iter().skip(result_parameters) {
        rhs = apply(rhs, index);
    }
    rhs = apply(rhs, ctor_value);
    for (name, domain) in binders.into_iter().rev() {
        rhs = Term::Lambda {
            param: name,
            param_type: Box::new(domain),
            body: Box::new(rhs),
        };
    }
    // The rule comes from applying the checked recursor to its own constructor.
    infer_type(ctx, &rhs).map_err(|e| {
        declined(
            DiagnosticCode::UnsupportedFeature,
            format!("derived rule failed to typecheck: {e}"),
        )
    })?;
    Ok(rhs)
}

// Projection lowering shares the kernel's checked match and iota rules. The
// source projection's additional Prop restriction is checked before lowering.
//
// Lean's kernel (`infer_proj`) accepts a projection out of any fully applied
// one-constructor family, indexed or not: it instantiates the constructor's
// PARAMETERS from the value's type and walks the fields; index arguments are never
// substituted into a field type. The lowering is the same checked `match`, whose
// motive abstracts the family's indices when it has any. Indices are kept, never
// erased: the motive ranges over every index, and the result is that motive at the
// value's own indices.
/// The reduction steps a projected value's type may take to reach its normal form before the
/// projection reads the type's weak-head form instead.
const PROJECTION_NORMAL_FORM_STEPS: usize = 1 << 12;
fn projection(
    ctx: &Context,
    name: &str,
    index: usize,
    disc: Term,
    budget: &mut usize,
) -> Result<Term> {
    if *budget == 0 {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "projection expansion budget",
        ));
    }
    *budget -= 1;
    let prefix = format!("arena.projection.{}", *budget);
    // The family is read from the normal form of the value's type — whenever that normal form is
    // reached within a small allowance, so every ordinary projection lowers exactly as before.
    // A type whose normal form does not arrive in time (`Nat.below motive (n + 4)` inside
    // `Nat.log2_terminates` unfolds well-founded division forever) is read as Lean's `infer_proj`
    // reads every type, `whnf (infer_type e)`: the head is exposed without unfolding the rest,
    // and its β-redexes are contracted. Only a head the bounded probe cannot reach keeps the full
    // normalization, exactly as before.
    let value_type = lean_type(ctx, &disc)?;
    let ty = match crate::reduction::normalize_checked_within(
        ctx,
        &value_type,
        PROJECTION_NORMAL_FORM_STEPS,
    ) {
        Ok(normal_form) => normal_form,
        Err((_, crate::Unreduced::BudgetSpent)) => {
            match crate::type_checker::exposed_inductive_type(ctx, value_type) {
                Ok(exposed) => match crate::reduction::beta_normal_form(&exposed, &mut (1 << 12)) {
                    Some(contracted) => contracted,
                    None => normal(ctx, &exposed)?,
                },
                Err(value_type) => normal(ctx, &value_type)?,
            }
        }
        Err(_) => normal(ctx, &value_type)?,
    };
    let mut head = &ty;
    let mut args = Vec::new();
    while let Term::App(f, a) = head {
        args.push((**a).clone());
        head = f;
    }
    args.reverse();
    let (parent, levels) = match head {
        Term::Global(n) => (n.as_str(), Vec::new()),
        Term::Const { name, levels } => (name.as_str(), levels.clone()),
        _ => {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "projection of a non-inductive value",
            ))
        }
    };
    if parent != name || !ctx.is_inductive(name) {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "projection parent mismatch",
        ));
    }
    let ctors = ctx.get_constructors(name);
    if ctors.len() != 1 {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "projection requires exactly one constructor",
        ));
    }
    let parameters = ctx.inductive_num_params(name);
    let arity = ctx
        .get_global(name)
        .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "missing projection parent"))?
        .clone();
    if args.len() != telescope_len(&arity) || args.len() < parameters {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "projection parent is not fully applied",
        ));
    }
    let family = |arguments: Vec<Term>| {
        arguments.into_iter().fold(
            if levels.is_empty() {
                Term::Global(name.into())
            } else {
                Term::Const {
                    name: name.into(),
                    levels: levels.clone(),
                }
            },
            apply,
        )
    };
    let ctor = if levels.is_empty() {
        Term::Global(ctors[0].0.into())
    } else {
        Term::Const {
            name: ctors[0].0.into(),
            levels: levels.clone(),
        }
    };
    let mut cty = infer_type(ctx, &ctor).map_err(kernel)?;
    let mut arity_rest = if levels.is_empty() {
        arity
    } else {
        infer_type(ctx, &family(Vec::new())).map_err(kernel)?
    };
    for arg in &args[..parameters] {
        let (_, body) = open_binder(cty, &format!("{prefix}.parameter"))?;
        cty = crate::substitute(&body, &format!("{prefix}.parameter"), arg);
        let (_, rest) = open_binder(arity_rest, &format!("{prefix}.parameter"))?;
        arity_rest = crate::substitute(&rest, &format!("{prefix}.parameter"), arg);
    }
    // The motive ranges over the family at its parameters and ARBITRARY indices.
    let mut motive_context = ctx.clone();
    let mut index_binders = Vec::new();
    for i in parameters..args.len() {
        let binder = format!("{prefix}.index.{}", i - parameters);
        let (domain, rest) = open_binder(arity_rest, &binder)?;
        motive_context = motive_context.extend(&binder, domain.clone());
        index_binders.push((binder, domain));
        arity_rest = rest;
    }
    let generic = if index_binders.is_empty() {
        ty.clone()
    } else {
        family(
            args[..parameters]
                .iter()
                .cloned()
                .chain(
                    index_binders
                        .iter()
                        .map(|(binder, _)| Term::Var(binder.clone())),
                )
                .collect(),
        )
    };
    let maybe_prop = !Universe::Type(0).is_subtype_of(&sort(ctx, &ty)?);
    let mut field_context = ctx.clone();
    let mut fields = Vec::new();
    while matches!(cty, Term::Pi { .. }) {
        let field = format!("{prefix}.field.{}", fields.len());
        let (domain, body) = open_binder(cty, &field)?;
        // A previous data field needed anywhere in the remaining telescope
        // would itself require an illegal projection out of a proof.
        if maybe_prop
            && fields.len() < index
            && crate::substitute(
                &body,
                &field,
                &Term::Global("arena.projection.marker".into()),
            ) != body
            && !sort(&field_context, &domain)?.equiv(&Universe::Prop)
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "dependent projection traverses a propositional data field",
            ));
        }
        field_context = field_context.extend(&field, domain.clone());
        fields.push((field, domain));
        cty = body;
    }
    let (selected, field_type) = fields.get(index).ok_or_else(|| {
        rejected(
            DiagnosticCode::InvalidProof,
            "projection index out of range",
        )
    })?;
    let motive_name = format!("{prefix}.value");
    let motive_var = Term::Var(motive_name.clone());
    let local = motive_context.extend(&motive_name, generic.clone());
    let mut output = field_type.clone();
    for (i, (field, _)) in fields.iter().enumerate().take(index).rev() {
        let marked = crate::substitute(&output, field, &motive_var);
        if marked != output {
            let previous = projection(&local, name, i, motive_var.clone(), budget)?;
            output = crate::substitute(&output, field, &previous);
        }
    }
    if maybe_prop && !sort(&local, &output)?.equiv(&Universe::Prop) {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "a possibly propositional structure cannot project data",
        ));
    }
    let mut case = Term::Var(selected.clone());
    for (field, domain) in fields.into_iter().rev() {
        case = Term::Lambda {
            param: field,
            param_type: Box::new(domain),
            body: Box::new(case),
        };
    }
    let mut motive = Term::Lambda {
        param: motive_name,
        param_type: Box::new(generic),
        body: Box::new(output),
    };
    for (binder, domain) in index_binders.into_iter().rev() {
        motive = Term::Lambda {
            param: binder,
            param_type: Box::new(domain),
            body: Box::new(motive),
        };
    }
    let result = Term::Match {
        discriminant: Box::new(disc),
        motive: Box::new(motive),
        cases: vec![case],
    };
    infer_type(ctx, &result).map_err(kernel)?;
    Ok(result)
}

/// The recursor of a registered non-recursive structure, as Lean's kernel reduces it:
/// `λ params motives minors major. minor (proj₀ params major) … (projₙ params major)`, where the
/// recursor's telescope has `telescope` binders, the major premise is the last, and `minor` is the
/// position of the structure's one constructor's minor premise — `nparams + 1` for a lone
/// structure, after every motive and every earlier member's minors for a member of a block.
///
/// Each projection is written out as its checked body, instantiated at the universe instance
/// of the major premise's type, and never as a reference to the reader's internal projection
/// constant. The recursor is a published declaration, and its body must mean the same thing in
/// any environment that has the structure: a printed or replayed recursor cannot depend on a
/// name that only this reader's context declares.
fn structure_recursor_body(
    ctx: &Context,
    rec_ty: &Term,
    nparams: usize,
    minor: usize,
    telescope: usize,
    projections: &[String],
) -> Result<Term> {
    if nparams >= minor || minor + 1 >= telescope {
        return Err(rejected(
            DiagnosticCode::InvalidProof,
            "structure recursor minor premise outside its telescope",
        ));
    }
    let mut binders = Vec::new();
    let mut ty = rec_ty.clone();
    for i in 0..telescope {
        let name = format!("arena.structure.rec.{i}");
        let (domain, body) = open_binder(ty, &name)?;
        binders.push((name, domain));
        ty = body;
    }
    let (_, major_type) = &binders[telescope - 1];
    let mut head = major_type;
    while let Term::App(f, _) = head {
        head = f;
    }
    let levels = match head {
        Term::Global(_) => Vec::new(),
        Term::Const { levels, .. } => levels.clone(),
        _ => {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "structure recursor major premise is not the structure",
            ))
        }
    };
    let major = Term::Var(binders[telescope - 1].0.clone());
    let mut result = Term::Var(binders[minor].0.clone());
    for projection in projections {
        let body = if levels.is_empty() {
            ctx.get_definition_body(projection).cloned()
        } else {
            ctx.get_universe_poly(projection)
                .map(|(declared, _, body)| {
                    let subst = declared
                        .iter()
                        .cloned()
                        .zip(levels.iter().cloned())
                        .collect();
                    crate::instantiate_universes(body, &subst)
                })
        };
        let Some(mut projected) = body else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "structure projection has no checked body",
            ));
        };
        let arguments = binders[..nparams]
            .iter()
            .map(|(param, _)| Term::Var(param.clone()))
            .chain(std::iter::once(major.clone()));
        for argument in arguments {
            let Term::Lambda { param, body, .. } = projected else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "structure projection arity mismatch",
                ));
            };
            projected = crate::substitute(&body, &param, &argument);
        }
        result = apply(result, projected);
    }
    for (name, domain) in binders.into_iter().rev() {
        result = Term::Lambda {
            param: name,
            param_type: Box::new(domain),
            body: Box::new(result),
        };
    }
    Ok(result)
}

fn source_binder_kind(binder: &Value) -> Result<Option<crate::elaborate::ParamKind>> {
    use crate::elaborate::ParamKind;
    match binder.get("binderInfo") {
        None => Ok(None),
        Some(value) => Ok(Some(match value.as_str() {
            Some("default") => ParamKind::Explicit,
            Some("implicit") => ParamKind::Implicit,
            Some("strictImplicit") => ParamKind::StrictImplicit,
            Some("instImplicit") => ParamKind::Instance,
            _ => {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unsupported binder information",
                ))
            }
        })),
    }
}

type StageSink<'a> = &'a (dyn Fn(&Value) + Sync);

struct Importer<'a> {
    trace_sink: Option<StageSink<'a>>,
    trace_declaration: Option<(usize, String, String, Option<u64>)>,
    names: HashMap<usize, String>,
    levels: HashMap<usize, Universe>,
    level_vars: HashMap<usize, HashSet<String>>,
    level_depth: HashMap<usize, usize>,
    exprs: HashMap<usize, Value>,
    expr_depth: HashMap<usize, usize>,
    declarations: HashMap<String, usize>,
    ctx: Context,
    report: ImportReport,
    audit_declarations: bool,
    audited_names: HashMap<String, usize>,
    runtime_names: HashMap<String, &'static str>,
    runtime_target_pending: bool,
    retain: RetainSource,
    retained: Vec<(String, Term, Term)>,
    binder_names: bool,
    /// The export header's `meta.lean.version`: the Lean release whose kernel reads the export's
    /// string literals (`strings::Packing`). Empty when the header names none.
    lean_version: String,
}
impl<'a> Importer<'a> {
    fn new(audit_declarations: bool) -> Self {
        Self {
            trace_sink: None,
            trace_declaration: None,
            names: HashMap::from([(0, String::new())]),
            levels: HashMap::from([(0, Universe::Prop)]),
            level_vars: HashMap::from([(0, HashSet::new())]),
            level_depth: HashMap::from([(0, 0)]),
            exprs: HashMap::new(),
            expr_depth: HashMap::new(),
            declarations: HashMap::new(),
            ctx: Context::new(),
            report: ImportReport {
                declaration_audit_requested: audit_declarations,
                ..ImportReport::default()
            },
            audit_declarations,
            audited_names: HashMap::new(),
            runtime_names: HashMap::new(),
            runtime_target_pending: false,
            retain: RetainSource::Nothing,
            retained: Vec::new(),
            binder_names: false,
            lean_version: String::new(),
        }
    }
    fn name(&self, v: &Value) -> Result<String> {
        let name = self
            .names
            .get(&index(v)?)
            .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "unknown name index"))?;
        Ok(canonical_name(name))
    }
    fn level(&self, v: &Value, params: Option<&HashSet<String>>) -> Result<Universe> {
        let i = index(v)?;
        let u = self
            .levels
            .get(&i)
            .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "unknown level index"))?;
        if let Some(bound) = params {
            if !self.level_vars[&i].is_subset(bound) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "unbound universe parameter",
                ));
            }
        }
        Ok(u.clone())
    }
    // Inspect the source DAG before lowering: a let value or function argument
    // may disappear under reduction but remains subject to Lean's safety rule.
    fn check_runtime_references(&self, roots: &[&Value]) -> Result<()> {
        if self.runtime_names.is_empty() {
            return Ok(());
        }
        let mut work = roots
            .iter()
            .map(|id| index(id))
            .collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        while let Some(id) = work.pop() {
            if !seen.insert(id) {
                continue;
            }
            let row = self.exprs.get(&id).ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "unknown safety-dependency expression",
                )
            })?;
            if let Some(constant) = row.get("const") {
                let name = self.name(&constant["name"])?;
                if let Some(safety) = self.runtime_names.get(&name) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        format!("safe declaration references {safety} runtime declaration {name}"),
                    ));
                }
            }
            for child in record_children(row) {
                work.push(index(child)?);
            }
        }
        Ok(())
    }
    /// The kernel name of a binder at `depth` (binders enclosing it). `arena.local.<depth>` is unique
    /// along every path of a declaration, which is what checking relies on; with binder names
    /// requested, Lean's own name follows as `arena.local.<depth>.<name>` — still unique along every
    /// path, so conversion, substitution and sharing see the same binding structure.
    fn binder_param(&self, depth: usize, name: &Value) -> String {
        let base = format!("arena.local.{depth}");
        if !self.binder_names {
            return base;
        }
        match index(name)
            .ok()
            .and_then(|i| self.names.get(&i))
            .filter(|n| !n.is_empty())
        {
            Some(encoded) => format!("{base}.{}", intake::lean_name(encoded)),
            None => base,
        }
    }
    fn term(
        &self,
        id: &Value,
        bound: &mut Vec<(String, Term, Option<Term>)>,
        params: &HashSet<String>,
        budget: &mut usize,
    ) -> Result<Term> {
        self.term_core(
            id,
            bound,
            params,
            budget,
            None,
            &mut ImportReport::default(),
            &mut None,
        )
    }

    fn shared_term(
        &mut self,
        id: &Value,
        bound: &mut Vec<(String, Term, Option<Term>)>,
        params: &HashSet<String>,
        budget: &mut usize,
    ) -> Result<Term> {
        let plan = dag::Plan::new(self, index(id)?, bound.len())?;
        if plan.size(index(id)?) <= *budget && plan.depth(index(id)?) < dag::CLOSED_CHUNK_DEPTH {
            return self.term(id, bound, params, budget);
        }
        let mut context = self.ctx.clone();
        let mut report = ImportReport::default();
        let mut sharing = Some(dag::Sharing::new(plan, params));
        let result = self.term_core(
            id,
            bound,
            params,
            budget,
            Some(&mut context),
            &mut report,
            &mut sharing,
        );
        let term = result?;
        self.ctx = context;
        self.report.internal_helpers.extend(report.internal_helpers);
        Ok(term)
    }

    fn term_core(
        &self,
        id: &Value,
        bound: &mut Vec<(String, Term, Option<Term>)>,
        params: &HashSet<String>,
        budget: &mut usize,
        mut ctx: Option<&mut Context>,
        report: &mut ImportReport,
        sharing: &mut Option<dag::Sharing>,
    ) -> Result<Term> {
        enum Frame<'a> {
            Visit(usize, bool),
            FinishSharing(dag::Capture),
            App(usize),
            BinderType(usize, &'a Value, &'static str),
            BinderValue(usize, &'a Value, Term, usize),
            BetaDomain(usize, &'a Value, &'a Value),
            BetaValue(usize, &'a Value, usize, Term, usize),
            BinderBody {
                source: usize,
                param: String,
                ty: Term,
                ty_depth: usize,
                value: Option<Term>,
                value_depth: usize,
                lowered: bool,
                kind: &'static str,
            },
            Projection(usize, String, usize),
        }
        let initial_scope = bound.len();
        let result = (|| {
            let mut work = vec![Frame::Visit(index(id)?, false)];
            let mut values = Vec::<Term>::new();
            let mut depths = Vec::<usize>::new();
            let mut checked_lets = HashSet::<usize>::new();
            // Source IDs mean one immutable expression/universe environment in
            // this invocation. The transaction only adds fresh checked helpers.
            // Only source-closed annotation/value pairs can reuse this evidence;
            // becoming closed after local substitution is not enough.
            let mut closed_let_checks = HashSet::<(usize, usize)>::new();
            while let Some(frame) = work.pop() {
                let mut completed: Option<(usize, Term, usize)> = None;
                match frame {
                    Frame::Visit(id, unshared) => {
                        if !unshared {
                            if let Some(shared) = sharing.as_mut() {
                                if shared.plan.shared(id) {
                                    let capture = shared.capture_budgeted(id, bound, budget)?;
                                    if let Some(term) = shared.cached(&capture, budget)? {
                                        depths.push(crate::term_store::term_depth(&term));
                                        values.push(term);
                                    } else {
                                        work.push(Frame::FinishSharing(capture));
                                        work.push(Frame::Visit(id, true));
                                    }
                                    continue;
                                }
                            }
                        }
                        if *budget == 0 {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "term expansion budget",
                            ));
                        }
                        *budget -= 1;
                        let row = self.exprs.get(&id).ok_or_else(|| {
                            declined(DiagnosticCode::MalformedExport, "unknown expression index")
                        })?;
                        if let Some(x) = row.get("app") {
                            if let Some(shared) = sharing.as_mut() {
                                let function = index(&x["fn"])?;
                                if let Some(lambda) =
                                    self.exprs.get(&function).and_then(|row| row.get("lam"))
                                {
                                    if shared.beta.is_none() {
                                        shared.beta = Some(beta::Certificates::new(
                                            self.ctx.clone(),
                                            params,
                                            bound,
                                            budget,
                                        )?);
                                    }
                                    let certificates =
                                        shared.beta.as_mut().expect("source certificate pass");
                                    if certificates.certify(self, &shared.plan, id, budget)? {
                                        work.push(Frame::BetaDomain(id, lambda, &x["arg"]));
                                        work.push(Frame::Visit(index(&lambda["type"])?, false));
                                        continue;
                                    }
                                }
                            }
                            work.push(Frame::App(id));
                            work.push(Frame::Visit(index(&x["arg"])?, false));
                            work.push(Frame::Visit(index(&x["fn"])?, false));
                        } else if let Some(x) = row.get("mdata") {
                            // Lean metadata is a transparent wrapper. Preserve its payload
                            // in the import report, without introducing a kernel redex.
                            work.push(Frame::Visit(index(&x["expr"])?, false));
                        } else if let Some(x) = row.get("proj") {
                            work.push(Frame::Projection(
                                id,
                                self.name(&x["typeName"])?,
                                index(&x["idx"])?,
                            ));
                            work.push(Frame::Visit(index(&x["struct"])?, false));
                        } else if let Some((kind, x)) = ["forallE", "lam", "letE"]
                            .into_iter()
                            .find_map(|kind| row.get(kind).map(|x| (kind, x)))
                        {
                            work.push(Frame::BinderType(id, x, kind));
                            work.push(Frame::Visit(index(&x["type"])?, false));
                        } else if row.get("strVal").is_some() {
                            let term = strings::expand(
                                self,
                                row,
                                id,
                                budget,
                                ctx.as_deref_mut(),
                                report,
                                sharing,
                            )?;
                            let depth = crate::term_store::term_depth(&term);
                            completed = Some((id, term, depth));
                        } else {
                            let term = if let Some(index_value) = row.get("bvar") {
                                let position = bound
                                    .len()
                                    .checked_sub(index(index_value)?.saturating_add(1))
                                    .ok_or_else(|| {
                                        rejected(
                                            DiagnosticCode::MalformedExport,
                                            "free bound variable",
                                        )
                                    })?;
                                if checked_lets.contains(&position) {
                                    sharing
                                        .as_mut()
                                        .expect("checked let transaction")
                                        .copy_checked_value(
                                            bound[position].2.as_ref().expect("checked let value"),
                                            budget,
                                        )?
                                } else {
                                    self.term_atom(row, bound, params, budget)?
                                }
                            } else {
                                self.term_atom(row, bound, params, budget)?
                            };
                            let depth = crate::term_store::term_depth(&term);
                            completed = Some((id, term, depth));
                        }
                    }
                    Frame::FinishSharing(capture) => {
                        let body = values.pop().expect("decoded shared source template");
                        depths.pop().expect("shared source depth");
                        let term = sharing.as_mut().expect("active sharing plan").finish(
                            ctx.as_deref_mut().expect("sharing transaction"),
                            report,
                            capture,
                            body,
                            budget,
                        )?;
                        depths.push(crate::term_store::term_depth(&term));
                        values.push(term);
                    }
                    Frame::App(source) => {
                        let arg = values.pop().expect("decoded application argument");
                        let fun = values.pop().expect("decoded application function");
                        let arg_depth = depths.pop().expect("application argument depth");
                        let fun_depth = depths.pop().expect("application function depth");
                        completed = Some((
                            source,
                            Term::App(Box::new(fun), Box::new(arg)),
                            1 + arg_depth.max(fun_depth),
                        ));
                    }
                    Frame::BinderType(source, x, kind) => {
                        let ty = values.pop().expect("decoded binder domain");
                        let ty_depth = depths.pop().expect("binder domain depth");
                        if kind == "letE" {
                            work.push(Frame::BinderValue(source, x, ty, ty_depth));
                            work.push(Frame::Visit(index(&x["value"])?, false));
                        } else {
                            let param = self.binder_param(bound.len(), &x["name"]);
                            if let Some(certificates) =
                                sharing.as_mut().and_then(|s| s.beta.as_mut())
                            {
                                certificates.enter(&ty, budget)?;
                            }
                            bound.push((param.clone(), ty.clone(), None));
                            work.push(Frame::BinderBody {
                                source,
                                param,
                                ty,
                                ty_depth,
                                value: None,
                                value_depth: 0,
                                lowered: false,
                                kind,
                            });
                            work.push(Frame::Visit(index(&x["body"])?, false));
                        }
                    }
                    Frame::BetaDomain(source, lambda, argument) => {
                        let ty = values.pop().expect("certified beta domain");
                        let ty_depth = depths.pop().expect("certified beta domain depth");
                        work.push(Frame::BetaValue(
                            source,
                            lambda,
                            index(argument)?,
                            ty,
                            ty_depth,
                        ));
                        work.push(Frame::Visit(index(argument)?, false));
                    }
                    Frame::BetaValue(source, lambda, value_source, ty, ty_depth) => {
                        let mut value = values.pop().expect("certified beta value");
                        let mut value_depth = depths.pop().expect("certified beta value depth");
                        let lowered = sharing
                            .as_mut()
                            .expect("beta sharing transaction")
                            .check_closed_let(
                                ctx.as_deref().expect("beta transaction"),
                                &ty,
                                &value,
                                budget,
                            )?;
                        if lowered {
                            value = sharing
                                .as_mut()
                                .expect("beta sharing transaction")
                                .share_closed_value(
                                    ctx.as_deref_mut().expect("beta transaction"),
                                    report,
                                    value_source,
                                    value,
                                    budget,
                                )?;
                            value_depth = crate::term_store::term_depth(&value);
                        }
                        // Generic source typing has already been certified. A beta
                        // redex is definitionally this let even if it stays open.
                        let param = self.binder_param(bound.len(), &lambda["name"]);
                        if lowered {
                            checked_lets.insert(bound.len());
                        }
                        if let Some(certificates) = sharing.as_mut().and_then(|s| s.beta.as_mut()) {
                            certificates.enter(&ty, budget)?;
                        }
                        bound.push((param.clone(), ty.clone(), Some(value.clone())));
                        work.push(Frame::BinderBody {
                            source,
                            param,
                            ty,
                            ty_depth,
                            value: Some(value),
                            value_depth,
                            lowered,
                            kind: "letE",
                        });
                        work.push(Frame::Visit(index(&lambda["body"])?, false));
                    }
                    Frame::BinderValue(source, x, ty, ty_depth) => {
                        let mut value = values.pop().expect("decoded let value");
                        let mut value_depth = depths.pop().expect("let value depth");
                        let source_pair = (index(&x["type"])?, index(&x["value"])?);
                        let let_body = index(&x["body"])?;
                        let retained = sharing
                            .as_ref()
                            .is_none_or(|shared| shared.retains_let(let_body));
                        let reusable = sharing.as_ref().is_some_and(|shared| {
                            shared.plan.closed(source_pair.0) && shared.plan.closed(source_pair.1)
                        });
                        let lowered = if retained {
                            false
                        } else if reusable && closed_let_checks.contains(&source_pair) {
                            true
                        } else if let Some(shared) = sharing.as_mut() {
                            let checked = shared.check_closed_let(
                                ctx.as_deref().expect("let transaction"),
                                &ty,
                                &value,
                                budget,
                            )?;
                            if checked && reusable {
                                closed_let_checks.insert(source_pair);
                            }
                            checked
                        } else {
                            false
                        };
                        if lowered {
                            value = sharing
                                .as_mut()
                                .expect("let sharing transaction")
                                .share_closed_value(
                                    ctx.as_deref_mut().expect("let transaction"),
                                    report,
                                    index(&x["value"])?,
                                    value,
                                    budget,
                                )?;
                            value_depth = crate::term_store::term_depth(&value);
                        }
                        let param = self.binder_param(bound.len(), &x["name"]);
                        if lowered {
                            checked_lets.insert(bound.len());
                        }
                        if let Some(certificates) = sharing.as_mut().and_then(|s| s.beta.as_mut()) {
                            certificates.enter(&ty, budget)?;
                        }
                        bound.push((param.clone(), ty.clone(), Some(value.clone())));
                        work.push(Frame::BinderBody {
                            source,
                            param,
                            ty,
                            ty_depth,
                            value: Some(value),
                            value_depth,
                            lowered,
                            kind: "letE",
                        });
                        work.push(Frame::Visit(index(&x["body"])?, false));
                    }
                    Frame::BinderBody {
                        source,
                        param,
                        ty,
                        ty_depth,
                        value,
                        value_depth,
                        lowered,
                        kind,
                    } => {
                        checked_lets.remove(&(bound.len() - 1));
                        if let Some(certificates) = sharing.as_mut().and_then(|s| s.beta.as_mut()) {
                            certificates.leave();
                        }
                        bound.pop();
                        let body = values.pop().expect("decoded binder body");
                        let body_depth = depths.pop().expect("binder body depth");
                        if lowered {
                            completed = Some((source, body, body_depth));
                        } else {
                            let body = Box::new(body);
                            let param_type = Box::new(ty);
                            let term = match kind {
                                "forallE" => Term::Pi {
                                    param,
                                    param_type,
                                    body_type: body,
                                },
                                "lam" => Term::Lambda {
                                    param,
                                    param_type,
                                    body,
                                },
                                _ => Term::Let {
                                    name: param,
                                    ty: param_type,
                                    value: Box::new(value.expect("let value")),
                                    body,
                                },
                            };
                            completed =
                                Some((source, term, 1 + body_depth.max(ty_depth).max(value_depth)));
                        }
                    }
                    Frame::Projection(source, name, field) => {
                        let disc = values.pop().expect("decoded projection discriminant");
                        depths.pop().expect("projection discriminant depth");
                        let mut local = ctx.as_deref().unwrap_or(&self.ctx).clone();
                        for (name, ty, _) in bound.iter() {
                            local = local.extend(name, ty.clone());
                        }
                        let mut trial_budget = *budget;
                        let projected =
                            projection(&local, &name, field, disc.clone(), &mut trial_budget);
                        // The first attempt consumed real work even if a local
                        // definition hid the structure and forces a retry.
                        *budget = trial_budget;
                        match projected {
                            Ok(projected) => {
                                let depth = crate::term_store::term_depth(&projected);
                                completed = Some((source, projected, depth));
                            }
                            Err(error) if !bound.iter().any(|(_, _, value)| value.is_some()) => {
                                return Err(error)
                            }
                            Err(_) => {
                                // A local definition can hide the structure head or
                                // a dependent field type. Expand only this lowering
                                // site, retaining the surrounding checked Let nodes.
                                let expand = |term: &Term, prefix: usize| {
                                    let mut term = term.clone();
                                    for (local, _, value) in bound[..prefix].iter().rev() {
                                        if let Some(value) = value {
                                            term = crate::substitute(&term, local, value);
                                        }
                                    }
                                    term
                                };
                                let mut transparent = ctx.as_deref().unwrap_or(&self.ctx).clone();
                                for (i, (local, ty, _)) in bound.iter().enumerate() {
                                    transparent = transparent.extend(local, expand(ty, i));
                                }
                                let projected = projection(
                                    &transparent,
                                    &name,
                                    field,
                                    expand(&disc, bound.len()),
                                    budget,
                                )?;
                                let depth = crate::term_store::term_depth(&projected);
                                completed = Some((source, projected, depth));
                            }
                        }
                    }
                }
                if let Some((source, mut term, mut depth)) = completed {
                    // The declaration's own term — what reaches the root through nothing but
                    // lowered lets, which pass their body on — is published as the declaration; a
                    // helper for it would only move the same term behind one more name.
                    if depth >= dag::CLOSED_CHUNK_DEPTH
                        && sharing.is_some()
                        && work
                            .iter()
                            .rev()
                            .any(|frame| !matches!(frame, Frame::BinderBody { lowered: true, .. }))
                    {
                        if let Some(shared) = sharing.as_mut() {
                            let (cut, did_cut) = shared.cut_closed(
                                ctx.as_deref_mut().expect("closed chunk transaction"),
                                report,
                                source,
                                term,
                                budget,
                            )?;
                            term = cut;
                            if did_cut {
                                depth = 1;
                            }
                        }
                    }
                    depths.push(depth);
                    values.push(term);
                }
            }
            let term = values.pop().expect("decoded expression");
            debug_assert!(values.is_empty());
            Ok(term)
        })();
        bound.truncate(initial_scope);
        result
    }
    fn term_atom(
        &self,
        r: &Value,
        bound: &[(String, Term, Option<Term>)],
        params: &HashSet<String>,
        budget: &mut usize,
    ) -> Result<Term> {
        if let Some(x) = r.get("sort") {
            return Ok(Term::Sort(self.level(x, Some(params))?));
        }
        if let Some(x) = r.get("bvar") {
            return bound
                .len()
                .checked_sub(index(x)?.saturating_add(1))
                .map(|i| Term::Var(bound[i].0.clone()))
                .ok_or_else(|| rejected(DiagnosticCode::MalformedExport, "free bound variable"));
        }
        if let Some(x) = r.get("const") {
            let name = self.name(&x["name"])?;
            if let Some(safety) = self.runtime_names.get(&name) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    format!("safe term references {safety} runtime declaration {name}"),
                ));
            }
            let arity = self.declarations.get(&name).ok_or_else(|| {
                rejected(
                    DiagnosticCode::MissingDeclaration,
                    "undeclared or forward constant",
                )
            })?;
            let levels = array(&x["us"])?
                .iter()
                .map(|u| self.level(u, Some(params)))
                .collect::<Result<Vec<_>>>()?;
            if levels.len() != *arity {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "constant universe arity mismatch",
                ));
            }
            return Ok(if levels.is_empty() {
                Term::Global(name)
            } else {
                Term::Const { name, levels }
            });
        }
        if let Some(x) = r.get("natVal") {
            let digits = x.as_str().ok_or_else(|| {
                declined(DiagnosticCode::MalformedExport, "malformed natural literal")
            })?;
            let value = parse_natural_literal(digits, budget)?;
            let nat = "Nat";
            let zero = "Zero";
            let succ = "Succ";
            let ctors = self.ctx.get_constructors(nat);
            if self.declarations.get(nat) != Some(&0)
                || ctors.len() != 2
                || ctors[0].0 != zero
                || ctors[1].0 != succ
                || self.ctx.inductive_num_params(nat) != 0
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "natural literal requires canonical checked Nat",
                ));
            }
            let nt = Term::Global(nat.into());
            let declared = self
                .ctx
                .get_global(nat)
                .ok_or_else(|| rejected(DiagnosticCode::MissingDeclaration, "missing Nat"))?;
            exact(&self.ctx, declared, &Term::Sort(Universe::Type(0)))?;
            exact(&self.ctx, ctors[0].1, &nt)?;
            exact(
                &self.ctx,
                ctors[1].1,
                &Term::Pi {
                    param: "arena.nat.value".into(),
                    param_type: Box::new(nt.clone()),
                    body_type: Box::new(nt),
                },
            )?;
            return Ok(Term::Lit(crate::Literal::Nat(value)));
        }
        Err(declined(
            DiagnosticCode::UnknownRecordKind,
            "unsupported expression (metadata or new format)",
        ))
    }

    fn parameters(&self, d: &Value) -> Result<Vec<String>> {
        let levels = array(&d["levelParams"])?
            .iter()
            .map(|x| self.name(x))
            .collect::<Result<Vec<_>>>()?;
        if levels.iter().collect::<HashSet<_>>().len() != levels.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "duplicate universe parameter",
            ));
        }
        Ok(levels)
    }
    fn declaration_term(&self, id: &Value, params: &[String]) -> Result<Term> {
        self.term(
            id,
            &mut Vec::new(),
            &params.iter().cloned().collect(),
            &mut 100_000,
        )
    }
    /// Register `name` as a structure with checked projection functions, returning their names, when
    /// its carrier is surely not a proposition. `None` leaves it an ordinary one-constructor family.
    fn structure_view(
        &mut self,
        name: &str,
        levels: &[String],
        ctor_type: &Term,
    ) -> Result<Option<Vec<String>>> {
        // Lean's `isStructureLike` asks `isRec` of the whole block before anything else: a family
        // whose block is recursive is an ordinary family, and no projection is built for it.
        if self.ctx.block_is_recursive(name) {
            return Ok(None);
        }
        let parameters = self.ctx.inductive_num_params(name);
        let mut ty = ctor_type.clone();
        let mut params = Vec::new();
        let mut local = self.ctx.clone();
        let mut carrier = if levels.is_empty() {
            Term::Global(name.into())
        } else {
            Term::Const {
                name: name.into(),
                levels: levels.iter().cloned().map(Universe::Var).collect(),
            }
        };
        for i in 0..parameters {
            let param = format!("arena.structure.parameter.{i}");
            let (domain, body) = open_binder(ty, &param)?;
            local = local.extend(&param, domain.clone());
            carrier = apply(carrier, Term::Var(param.clone()));
            params.push((param, domain));
            ty = body;
        }
        // A symbolic carrier which might be Prop needs conditional projection
        // rules. Those are handled at each projection use, not by generic metadata.
        if !Universe::Type(0).is_subtype_of(&sort(&local, &carrier)?) {
            return Ok(None);
        }
        let mut fields = Vec::new();
        while matches!(ty, Term::Pi { .. }) {
            let field = format!("arena.structure.field.{}", fields.len());
            let (domain, body) = open_binder(ty, &field)?;
            fields.push((field, domain));
            ty = body;
        }
        let disc_name = "arena.structure.value";
        local = local.extend(disc_name, carrier.clone());
        let mut scratch = self.ctx.clone();
        let mut projections = Vec::new();
        for i in 0..fields.len() {
            let mut body = projection(&local, name, i, Term::Var(disc_name.into()), &mut 100_000)?;
            body = Term::Lambda {
                param: disc_name.into(),
                param_type: Box::new(carrier.clone()),
                body: Box::new(body),
            };
            for (param, domain) in params.iter().rev() {
                body = Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(domain.clone()),
                    body: Box::new(body),
                };
            }
            let ty = infer_type(&scratch, &body).map_err(kernel)?;
            let projection_name = format!("arena.structure.projection.{name}.{i}");
            if levels.is_empty() {
                scratch
                    .declare(&projection_name, ty, body)
                    .map_err(kernel)?;
            } else {
                scratch
                    .declare_universe_poly(&projection_name, levels.to_vec(), ty, body)
                    .map_err(kernel)?;
            }
            projections.push(projection_name);
        }
        // η is granted by the kernel's one door, which re-decides everything built above. A
        // family Lean does not call structure-like (a field reaching its own mutual block) stays
        // an ordinary family; any other refusal means this reader built something the kernel
        // will not certify, and the declaration is declined rather than approximated.
        match scratch.register_structure_checked(name, &projections) {
            Ok(()) => {}
            Err(crate::StructureRefusal::Recursive { .. }) => return Ok(None),
            Err(refusal) => {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    format!("structure η for {name}: {refusal}"),
                ))
            }
        }
        self.ctx = scratch;
        Ok(Some(projections))
    }

    fn quotient(&mut self, d: &Value) -> Result<()> {
        use crate::quotient::{self, Kind};
        let kind = match d["kind"].as_str() {
            Some("type") => Kind::Type,
            Some("ctor") => Kind::Constructor,
            Some("lift") => Kind::Lift,
            Some("ind") => Kind::Induction,
            _ => {
                return Err(rejected(
                    DiagnosticCode::UnknownRecordKind,
                    "unknown quotient primitive kind",
                ))
            }
        };
        let name = self.name(&d["name"])?;
        if name != kind.name() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "quotient primitive name does not match its kind",
            ));
        }
        if self.declarations.contains_key(&name) {
            return Err(rejected(
                DiagnosticCode::DuplicateDeclaration,
                "duplicate quotient declaration",
            ));
        }
        let params = self.parameters(d)?;
        if params.len() != kind.arity() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "quotient primitive universe arity",
            ));
        }
        let ty = self.declaration_term(&d["type"], &params)?;
        let levels = params
            .iter()
            .cloned()
            .map(Universe::Var)
            .collect::<Vec<_>>();
        let equality = "/s:\"Eq\"";
        let expected = quotient::signature(kind, &levels, equality).map_err(kernel)?;
        lean_type(&self.ctx, &ty)?;
        exact(&self.ctx, &ty, &expected)?;
        quotient::install(&mut self.ctx, kind, params.clone(), equality)
            .map_err(|e| rejected(DiagnosticCode::InvalidProof, e.to_string()))?;
        self.declarations.insert(name, params.len());
        Ok(())
    }

    fn install_binder_kinds(&mut self, declaration: &Value) -> Result<()> {
        // The side table records a declaration's PARAMETERS, as LOGOS's own declaration door
        // does: the binders its signature introduces. A definition's parameters are the ones
        // its value abstracts, so a function type in the declared result (`(f : T) : T` with
        // `T := ⦃x⦄ → {y} → Type`) is not flattened into the declaration's own telescope.
        // A declaration without a value records its whole leading telescope.
        let parameters = match declaration.get("value") {
            Some(value) => Some(self.leading_binders(index(value)?, "lam")?),
            None => None,
        };
        let mut expression = index(&declaration["type"])?;
        let mut kinds = Vec::new();
        let mut binders = 0;
        let mut complete = true;
        while parameters.is_none_or(|count| binders < count) {
            let row = self.exprs.get(&expression).ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "missing declaration binder expression",
                )
            })?;
            if let Some(metadata) = row.get("mdata") {
                expression = index(&metadata["expr"])?;
            } else if let Some(binder) = row.get("forallE") {
                match source_binder_kind(binder)? {
                    Some(kind) => kinds.push(kind),
                    None => complete = false,
                }
                binders += 1;
                expression = index(&binder["body"])?;
            } else {
                break;
            }
        }
        // Missing source metadata must not become a fabricated Explicit entry.
        // Only the declaration's leading telescope belongs in this side table;
        // nested function arguments retain their fully elaborated kernel terms.
        if complete {
            let name = self.name(&declaration["name"])?;
            self.ctx.set_binder_kinds(&name, kinds);
        }
        Ok(())
    }

    /// The number of leading `kind` binders of a source expression, seen through metadata.
    fn leading_binders(&self, mut expression: usize, kind: &str) -> Result<usize> {
        let mut count = 0;
        loop {
            let row = self.exprs.get(&expression).ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "missing declaration binder expression",
                )
            })?;
            if let Some(metadata) = row.get("mdata") {
                expression = index(&metadata["expr"])?;
            } else if let Some(binder) = row.get(kind) {
                count += 1;
                expression = index(&binder["body"])?;
            } else {
                return Ok(count);
            }
        }
    }

    fn inductive(&mut self, d: &Value) -> Result<()> {
        let types = array(&d["types"])?;
        let ctors = array(&d["ctors"])?;
        let recs = array(&d["recs"])?;
        let mut source_roots = Vec::new();
        for item in types.iter().chain(ctors).chain(recs) {
            source_roots.push(&item["type"]);
        }
        for rec in recs {
            for rule in array(&rec["rules"])? {
                source_roots.push(&rule["rhs"]);
            }
        }
        self.check_runtime_references(&source_roots)?;

        if types.is_empty() {
            // No Lean environment exports a block without a family: the record is malformed, not a
            // declaration the kernel refutes.
            return Err(declined(
                DiagnosticCode::MalformedExport,
                "empty inductive block",
            ));
        }
        if types.len() > 1 {
            return self.ordinary_mutual_inductive(types, ctors, recs);
        }
        let header = &types[0];
        let name = self.name(&header["name"])?;
        let params = self.parameters(header)?;
        let nested_count = index(&header["numNested"])?;
        for item in types.iter().chain(ctors).chain(recs) {
            if item["isUnsafe"] != false {
                return Err(inductive_safety(
                    &item["isUnsafe"],
                    "unsafe or unspecified inductive safety",
                ));
            }
        }
        let mut names = HashSet::new();
        for item in types.iter().chain(ctors).chain(recs) {
            let n = self.name(&item["name"])?;
            if self.declarations.contains_key(&n) || !names.insert(n) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "duplicate declaration in inductive block",
                ));
            }
        }
        let all = array(&header["all"])?
            .iter()
            .map(|x| self.name(x))
            .collect::<Result<Vec<_>>>()?;
        if all != vec![name.clone()] {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "inconsistent inductive block membership",
            ));
        }
        let nparams = index(&header["numParams"])?;
        let nindices = index(&header["numIndices"])?;
        let arity = self.declaration_term(&header["type"], &params)?;
        lean_type(&self.ctx, &arity)?;
        let arity = declared_arity(&self.ctx, &arity)?;
        let mut result = &arity;
        while let Term::Pi { body_type, .. } = result {
            result = body_type;
        }
        if !matches!(result, Term::Sort(_))
            || nparams.checked_add(nindices) != Some(telescope_len(&arity))
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "inductive arity or parameter/index count is invalid",
            ));
        }
        let declared_ctors = array(&header["ctors"])?
            .iter()
            .map(|x| self.name(x))
            .collect::<Result<Vec<_>>>()?;
        if declared_ctors.len() != ctors.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "constructor inventory mismatch",
            ));
        }
        let old_ctx = self.ctx.clone();
        // Temporary headers permit constructor type references; the checked block
        // registration below validates the complete group before it is used.
        if params.is_empty() {
            self.ctx
                .add_indexed_inductive(&name, arity.clone(), nparams);
        } else {
            self.ctx
                .add_universe_poly_inductive(&name, params.clone(), arity.clone(), nparams);
        }
        self.declarations.insert(name.clone(), params.len());
        let mut checked_ctors = Vec::new();
        for (i, c) in ctors.iter().enumerate() {
            let cname = self.name(&c["name"])?;
            if self.name(&c["induct"])? != name
                || index(&c["cidx"])? != i
                || cname != declared_ctors[i]
                || index(&c["numParams"])? != nparams
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "constructor parent, order or parameter count mismatch",
                ));
            }
            if self.parameters(c)? != params {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "constructor universe telescope mismatch",
                ));
            }
            let cty = self.declaration_term(&c["type"], &params)?;
            lean_type(&self.ctx, &cty)?;
            if nparams.checked_add(index(&c["numFields"])?)
                != Some(resolved_telescope_len(&self.ctx, &cty)?)
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "constructor field count mismatch",
                ));
            }
            // Preserve the constructor's original index expressions. Lean's large-
            // elimination test recognizes bare variables, not normalized inverses.
            checked_ctors.push((cname, cty));
        }
        // Reserved primitive identities must denote exactly the checked Peano
        // datatype, even if this export never uses a literal. Otherwise native
        // reduction rules could act on an unrelated datatype with the same name.
        let reserved = |n: &str| matches!(n, "Nat" | "Zero" | "Succ");
        if reserved(&name) || checked_ctors.iter().any(|(n, _)| reserved(n)) {
            if name != "Nat"
                || !params.is_empty()
                || nparams != 0
                || nindices != 0
                || checked_ctors.len() != 2
                || checked_ctors[0].0 != "Zero"
                || checked_ctors[1].0 != "Succ"
            {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "noncanonical datatype uses primitive Nat identities",
                ));
            }
            let nt = Term::Global("Nat".into());
            let canonical = exact(&self.ctx, &arity, &Term::Sort(Universe::Type(0)))
                .and_then(|_| exact(&self.ctx, &checked_ctors[0].1, &nt))
                .and_then(|_| {
                    exact(
                        &self.ctx,
                        &checked_ctors[1].1,
                        &Term::Pi {
                            param: "arena.nat.field".into(),
                            param_type: Box::new(nt.clone()),
                            body_type: Box::new(nt),
                        },
                    )
                });
            if canonical.is_err() {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "noncanonical datatype uses primitive Nat identities",
                ));
            }
        }
        let block = crate::MutualInductive {
            name: name.clone(),
            sort: arity.clone(),
            num_params: nparams,
            constructors: checked_ctors.clone(),
        };
        let mut verified = old_ctx.clone();
        if params.is_empty() {
            verified
                .add_mutual_inductives(&[block])
                .map_err(inductive_verdict)?;
        } else {
            verified
                .add_universe_poly_inductives(&params, &[block])
                .map_err(inductive_verdict)?;
        }
        let nested_decl = if nested_count > 0 {
            // Parameters, universes, indices and indexed containers are reconstructed exactly,
            // by Lean's own elimination; a shape outside it is declined, never approximated.
            let decl = crate::NestedDecl {
                name: name.clone(),
                sort: arity.clone(),
                constructors: checked_ctors.clone(),
            };
            // Establish the reconstruction boundary before registration.
            crate::inductive_compile::eliminate_nested(
                &old_ctx,
                &crate::inductive_compile::NestedBlock {
                    members: std::slice::from_ref(&decl),
                    num_params: nparams,
                    levels: &params,
                },
            )
            .map_err(inductive_verdict)?;
            Some(decl)
        } else {
            None
        };
        self.ctx = verified;
        for (n, _) in &checked_ctors {
            self.declarations.insert(n.clone(), params.len());
        }
        if let Some(decl) = nested_decl {
            return self.native_nested_recursors(
                std::slice::from_ref(&decl),
                nparams,
                &params,
                recs,
                &all,
                nested_count,
            );
        }
        if recs.len() != 1 {
            // This path has already established a single, non-nested inductive
            // block. Such a block has exactly its one canonical recursor;
            // extra records cannot introduce arbitrary inhabitants as recursors.
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "noncanonical recursor inventory for a single non-nested inductive",
            ));
        }
        let rec = &recs[0];
        let rec_name = self.name(&rec["name"])?;
        if rec_name != format!("{name}/s:\"rec\"") {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "recursor does not have its canonical generated name",
            ));
        }
        if index(&rec["numParams"])? != nparams
            || index(&rec["numIndices"])? != nindices
            || index(&rec["numMotives"])? != 1
            || index(&rec["numMinors"])? != ctors.len()
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "recursor telescope counts mismatch",
            ));
        }
        if array(&rec["all"])?
            .iter()
            .map(|x| self.name(x))
            .collect::<Result<Vec<_>>>()?
            != all
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "recursor block membership mismatch",
            ));
        }
        let expected_k = self.ctx.k_constructor(&name).is_some();
        if rec["k"].as_bool() != Some(expected_k) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "recursor K flag disagrees with its checked inductive",
            ));
        }
        let levels = self.parameters(rec)?;
        let (derived_levels, derived_ty, derived_body) =
            crate::derive_recursor_universe_poly(&self.ctx, &name).map_err(|e| {
                declined(
                    DiagnosticCode::UnsupportedFeature,
                    format!("cannot derive recursor: {e}"),
                )
            })?;
        if levels.len() != derived_levels.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "recursor universe telescope mismatch",
            ));
        }
        let subst = derived_levels
            .into_iter()
            .zip(levels.iter().cloned().map(Universe::Var))
            .collect();
        let derived_ty = crate::instantiate_universes(&derived_ty, &subst);
        let derived_body = crate::instantiate_universes(&derived_body, &subst);
        let mut computation = &derived_body;
        while let Term::Lambda { body, .. } = computation {
            computation = body;
        }
        let nonrecursive = !matches!(computation, Term::Fix { .. } | Term::MutualFix { .. });
        // Lean's iota reduces a structure's recursor on ANY major premise of a non-recursive,
        // never-`Prop` structure type by expanding it to `mk params (proj₀ e) … (projₙ e)`
        // (`toCtorWhenStruct`). Registering the structure first lets the recursor say exactly
        // that: its body applies the minor premise to the projections of the major premise,
        // which both kernels check against the derived type through structure η.
        let projections = if nindices == 0 && nonrecursive && checked_ctors.len() == 1 {
            self.structure_view(&name, &params, &checked_ctors[0].1)?
        } else {
            None
        };
        let derived_body = match &projections {
            Some(projections) => structure_recursor_body(
                &self.ctx,
                &derived_ty,
                nparams,
                nparams + 1,
                nparams + 3,
                projections,
            )?,
            None => derived_body,
        };

        let exported_ty = self.declaration_term(&rec["type"], &levels)?;
        lean_type(&self.ctx, &exported_ty)?;
        exact(&self.ctx, &exported_ty, &derived_ty)?;
        // The recursor is registered at Lean's statement, which the derivation has just been
        // checked to be by exact conversion; its body is the derivation's.
        // Register only the derived implementation. The input's rules are evidence
        // to compare, never executable reducer instructions trusted on their own.
        if levels.is_empty() {
            self.ctx
                .declare(&rec_name, exported_ty.clone(), derived_body)
                .map_err(kernel)?;
        } else {
            self.ctx
                .declare_universe_poly(&rec_name, levels.clone(), exported_ty.clone(), derived_body)
                .map_err(kernel)?;
        }
        self.declarations.insert(rec_name.clone(), levels.len());
        let rules = array(&rec["rules"])?;
        if rules.len() != ctors.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "missing or additional recursor computation rule",
            ));
        }
        let mut seen = HashSet::new();
        for rule in rules {
            let cname = self.name(&rule["ctor"])?;
            let Some(i) = declared_ctors.iter().position(|n| n == &cname) else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "recursor rule names a foreign constructor",
                ));
            };
            if !seen.insert(cname.clone()) || rule["nfields"] != ctors[i]["numFields"] {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "duplicate rule or wrong field count",
                ));
            }
            let expected = ctor_rule(
                &self.ctx,
                &rec_name,
                &levels,
                &derived_ty,
                &cname,
                &checked_ctors[i].1,
                nparams,
                nparams + 1 + ctors.len(),
            )?;
            let actual = self.declaration_term(&rule["rhs"], &levels)?;
            let actual_ty = lean_type(&self.ctx, &actual)?;
            let expected_ty = infer_type(&self.ctx, &expected).map_err(kernel)?;
            exact(&self.ctx, &actual_ty, &expected_ty)?;
            exact(&self.ctx, &actual, &expected)?;
        }
        Ok(())
    }

    fn ordinary_mutual_inductive(
        &mut self,
        types: &[Value],
        ctors: &[Value],
        recs: &[Value],
    ) -> Result<()> {
        use crate::MutualInductive;
        struct Member {
            declaration: MutualInductive,
            indices: usize,
            constructors: Vec<String>,
        }
        let mut names = HashSet::new();
        for item in types.iter().chain(ctors).chain(recs) {
            if item["isUnsafe"] != false {
                return Err(inductive_safety(
                    &item["isUnsafe"],
                    "unsafe or unspecified mutual inductive safety",
                ));
            }
            let name = self.name(&item["name"])?;
            if self.declarations.contains_key(&name) || !names.insert(name.clone()) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "duplicate declaration in mutual block",
                ));
            }
            if matches!(name.as_str(), "Nat" | "Zero" | "Succ") || name.starts_with("kernel.quot.")
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "primitive identities require their canonical declaration",
                ));
            }
        }
        let all = types
            .iter()
            .map(|h| self.name(&h["name"]))
            .collect::<Result<Vec<_>>>()?;
        let params = self.parameters(&types[0])?;
        let nparams = index(&types[0]["numParams"])?;
        let mut members = Vec::new();
        let mut shared_domains: Vec<Term> = Vec::new();
        let mut common_sort: Option<Universe> = None;
        let nested_count = index(&types[0]["numNested"])?;
        for (i, header) in types.iter().enumerate() {
            // Lean records the block's auxiliary count on every member.
            if index(&header["numNested"])? != nested_count {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual members disagree on the block's nested count",
                ));
            }
            if self.parameters(header)? != params || index(&header["numParams"])? != nparams {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual headers disagree on their shared parameters",
                ));
            }
            if array(&header["all"])?
                .iter()
                .map(|n| self.name(n))
                .collect::<Result<Vec<_>>>()?
                != all
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual header block membership mismatch",
                ));
            }
            let arity = self.declaration_term(&header["type"], &params)?;
            lean_type(&self.ctx, &arity)?;
            let arity = declared_arity(&self.ctx, &arity)?;
            let indices = index(&header["numIndices"])?;
            if nparams.checked_add(indices) != Some(telescope_len(&arity)) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual header parameter/index count mismatch",
                ));
            }
            let mut telescope = arity.clone();
            let mut local = self.ctx.clone();
            for j in 0..nparams {
                let name = format!("arena.mutual.parameter.{j}");
                let (domain, body) = open_binder(telescope, &name)?;
                if i == 0 {
                    shared_domains.push(domain.clone());
                } else {
                    exact(&local, &domain, &shared_domains[j])?;
                }
                local = local.extend(&name, domain);
                telescope = body;
            }
            while let Term::Pi { body_type, .. } = telescope {
                telescope = *body_type;
            }
            let Term::Sort(level) = telescope else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual header does not return a sort",
                ));
            };
            if let Some(common) = &common_sort {
                if !level.equiv(common) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual members have different result universes",
                    ));
                }
            } else {
                common_sort = Some(level);
            }
            let constructors = array(&header["ctors"])?
                .iter()
                .map(|n| self.name(n))
                .collect::<Result<Vec<_>>>()?;
            members.push(Member {
                declaration: MutualInductive {
                    name: all[i].clone(),
                    sort: arity,
                    num_params: nparams,
                    constructors: Vec::new(),
                },
                indices,
                constructors,
            });
        }
        let old_ctx = self.ctx.clone();
        for member in &members {
            let declaration = &member.declaration;
            if params.is_empty() {
                self.ctx.add_indexed_inductive(
                    &declaration.name,
                    declaration.sort.clone(),
                    nparams,
                );
            } else {
                self.ctx.add_universe_poly_inductive(
                    &declaration.name,
                    params.clone(),
                    declaration.sort.clone(),
                    nparams,
                );
            }
            self.declarations
                .insert(declaration.name.clone(), params.len());
        }
        let mut used = HashSet::new();
        for member in &mut members {
            for (i, name) in member.constructors.iter().enumerate() {
                let ctor = ctors
                    .iter()
                    .find(|c| self.name(&c["name"]).ok().as_ref() == Some(name))
                    .ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "missing declared mutual constructor",
                        )
                    })?;
                if !used.insert(name.clone())
                    || self.name(&ctor["induct"])? != member.declaration.name
                    || index(&ctor["cidx"])? != i
                    || index(&ctor["numParams"])? != nparams
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual constructor parent/order/parameter mismatch",
                    ));
                }
                if self.parameters(ctor)? != params {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual constructor universe telescope mismatch",
                    ));
                }
                let ty = self.declaration_term(&ctor["type"], &params)?;
                lean_type(&self.ctx, &ty)?;
                if nparams.checked_add(index(&ctor["numFields"])?)
                    != Some(resolved_telescope_len(&self.ctx, &ty)?)
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual constructor field count mismatch",
                    ));
                }
                member.declaration.constructors.push((name.clone(), ty));
            }
        }
        if used.len() != ctors.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "undeclared constructor in mutual block",
            ));
        }
        let block = members
            .iter()
            .map(|m| m.declaration.clone())
            .collect::<Vec<_>>();
        self.ctx = old_ctx;
        if params.is_empty() {
            self.ctx
                .add_mutual_inductives(&block)
                .map_err(inductive_verdict)?;
        } else {
            self.ctx
                .add_universe_poly_inductives(&params, &block)
                .map_err(inductive_verdict)?;
        }
        for member in &members {
            for name in &member.constructors {
                self.declarations.insert(name.clone(), params.len());
            }
        }
        if nested_count > 0 {
            let decls: Vec<crate::NestedDecl> = members
                .iter()
                .map(|m| crate::NestedDecl {
                    name: m.declaration.name.clone(),
                    sort: m.declaration.sort.clone(),
                    constructors: m.declaration.constructors.clone(),
                })
                .collect();
            return self.native_nested_recursors(
                &decls,
                nparams,
                &params,
                recs,
                &all,
                nested_count,
            );
        }
        if recs.len() != members.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "mutual recursor inventory mismatch",
            ));
        }
        // Lean's `isStructureLike` holds for a member with one constructor and no indices whose
        // BLOCK is not recursive (`isRec` is decided for the whole block), and its iota then reduces
        // that member's recursor on any major premise. Every such member is registered through the
        // kernel's one door — which refuses a recursive block, leaving the member an ordinary
        // family — before any recursor, because a structure's recursor relies on its η.
        let mut structures = Vec::with_capacity(members.len());
        for member in &members {
            structures.push(match &member.declaration.constructors[..] {
                [(_, constructor)] if member.indices == 0 => {
                    self.structure_view(&member.declaration.name, &params, constructor)?
                }
                _ => None,
            });
        }
        // Lean gives every motive in an ordinary mutual block one shared
        // elimination universe. A block whose result may be Prop is restricted
        // to Prop; the single-member singleton exception does not apply.
        let large = Universe::Type(0).is_subtype_of(common_sort.as_ref().expect("nonempty block"));
        let motive_offset = usize::from(large);
        let prefix = nparams
            .checked_add(members.len())
            .and_then(|v| v.checked_add(ctors.len()))
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "mutual recursor telescope overflow",
                )
            })?;
        let mut checked = Vec::new();
        let mut minors_before = 0;
        for (member, projections) in members.iter().zip(&structures) {
            let minor = prefix - ctors.len() + minors_before;
            minors_before += member.constructors.len();
            let name = format!("{}/s:\"rec\"", member.declaration.name);
            let position = recs
                .iter()
                .position(|r| self.name(&r["name"]).ok().as_ref() == Some(&name))
                .ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "missing canonical mutual recursor",
                    )
                })?;
            let rec = &recs[position];
            if index(&rec["numParams"])? != nparams
                || index(&rec["numIndices"])? != member.indices
                || index(&rec["numMotives"])? != members.len()
                || index(&rec["numMinors"])? != ctors.len()
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual recursor telescope counts mismatch",
                ));
            }
            if array(&rec["all"])?
                .iter()
                .map(|n| self.name(n))
                .collect::<Result<Vec<_>>>()?
                != all
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual recursor block membership mismatch",
                ));
            }
            if rec["k"].as_bool() != Some(false) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "multi-member recursor cannot have K flag",
                ));
            }
            let levels = self.parameters(rec)?;
            if levels.len() != params.len() + motive_offset {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual recursor universe telescope mismatch",
                ));
            }
            let (derived_levels, ty, body) =
                crate::derive_recursor_universe_poly(&self.ctx, &member.declaration.name).map_err(
                    |e| {
                        declined(
                            DiagnosticCode::UnsupportedFeature,
                            format!("cannot derive mutual recursor: {e}"),
                        )
                    },
                )?;
            let motive_count = derived_levels
                .len()
                .checked_sub(params.len())
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::UnsupportedFeature,
                        "derived mutual universe inventory mismatch",
                    )
                })?;
            if derived_levels[motive_count..] != params {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "derived mutual inductive universe telescope mismatch",
                ));
            }
            let motive = if large {
                Universe::Var(levels[0].clone())
            } else {
                Universe::Prop
            };
            let mut substitution: HashMap<String, Universe> = derived_levels[..motive_count]
                .iter()
                .cloned()
                .map(|name| (name, motive.clone()))
                .collect();
            let parameter_substitution: HashMap<String, Universe> = params
                .iter()
                .cloned()
                .zip(levels[motive_offset..].iter().cloned().map(Universe::Var))
                .collect();
            substitution.extend(parameter_substitution.clone());
            let ty = crate::instantiate_universes(&ty, &substitution);
            let body = match projections {
                Some(projections) => structure_recursor_body(
                    &self.ctx,
                    &ty,
                    nparams,
                    minor,
                    prefix + 1,
                    projections,
                )?,
                None => crate::instantiate_universes(&body, &substitution),
            };
            let exported = self.declaration_term(&rec["type"], &levels)?;
            lean_type(&self.ctx, &exported)?;
            exact(&self.ctx, &exported, &ty)?;
            if levels.is_empty() {
                self.ctx.declare(&name, exported, body).map_err(kernel)?;
            } else {
                self.ctx
                    .declare_universe_poly(&name, levels.clone(), exported, body)
                    .map_err(kernel)?;
            }
            self.declarations.insert(name.clone(), levels.len());
            checked.push((name, levels, ty, position, parameter_substitution));
        }
        // All sibling implementations are now in scope for rule references.
        for (member, (name, levels, ty, position, substitution)) in members.iter().zip(&checked) {
            let rules = array(&recs[*position]["rules"])?;
            if rules.len() != member.constructors.len() {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "mutual computation rule inventory mismatch",
                ));
            }
            let mut seen = HashSet::new();
            for rule in rules {
                let constructor = self.name(&rule["ctor"])?;
                let Some((_, ctor_ty)) = member
                    .declaration
                    .constructors
                    .iter()
                    .find(|(n, _)| n == &constructor)
                else {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual rule names a foreign constructor",
                    ));
                };
                let field_count = resolved_telescope_len(&self.ctx, ctor_ty)?
                    .checked_sub(nparams)
                    .ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "mutual constructor lacks shared parameters",
                        )
                    })?;
                if !seen.insert(constructor.clone()) || index(&rule["nfields"])? != field_count {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "mutual rule duplicate constructor or wrong field count",
                    ));
                }
                let value = if params.is_empty() {
                    Term::Global(constructor)
                } else {
                    Term::Const {
                        name: constructor,
                        levels: params.iter().map(|p| substitution[p].clone()).collect(),
                    }
                };
                let ctor_ty = crate::instantiate_universes(ctor_ty, substitution);
                let expected = ctor_rule_applied(
                    &self.ctx, name, levels, ty, value, ctor_ty, nparams, prefix, nparams,
                )?;
                let actual = self.declaration_term(&rule["rhs"], levels)?;
                let actual_type = lean_type(&self.ctx, &actual)?;
                let expected_type = infer_type(&self.ctx, &expected).map_err(kernel)?;
                exact(&self.ctx, &actual_type, &expected_type)?;
                exact(&self.ctx, &actual, &expected)?;
            }
        }
        Ok(())
    }

    fn declaration_audit(
        &self,
        name: &str,
        kind: &str,
        source: &Value,
        ty: &Term,
        body: Option<&Term>,
    ) -> Result<DeclarationAudit> {
        self.trace_stage("audit", "begin");
        let main_type = infer_type(&self.ctx, ty).map_err(kernel)?;
        let independent_type = crate::recheck(&self.ctx, ty).map_err(|e| {
            independent_verdict(&e, format!("independent declaration-type check for {name}"))
        })?;
        independent_comparison(
            exact(&self.ctx, &main_type, &independent_type),
            format!("independent declaration-type disagreement for {name}"),
        )?;
        // Lean's `#print axioms` (`CollectAxioms`): a definition or theorem rests on the axioms its
        // type and value reach; an axiom rests on ITSELF, and its own type is not visited. The same
        // shared closure (`prelude::axiom_dependencies`) is the package rule's; a declaration this
        // import already audited answers with the axioms recorded for it.
        let known = |dependency: &str| {
            self.audited_names
                .get(dependency)
                .map(|i| self.report.declarations[*i].axioms.clone())
        };
        let mut usage = if kind == "axiom" {
            crate::prelude::AxiomUsage::default()
        } else {
            crate::prelude::axiom_dependencies_given(&self.ctx, ty, &known)
        };
        if let Some(body) = body {
            let independent = crate::recheck(&self.ctx, body).map_err(|e| {
                independent_verdict(&e, format!("independent declaration-body check for {name}"))
            })?;
            independent_comparison(
                exact(&self.ctx, &independent, ty),
                format!("independent declaration-body type disagreement for {name}"),
            )?;
            usage.absorb(crate::prelude::axiom_dependencies_given(&self.ctx, body, &known));
        }
        if !usage.unresolved.is_empty() {
            return Err(declined(
                DiagnosticCode::AuditIncomplete,
                format!(
                    "unresolved declaration audit dependencies for {name}: {:?}",
                    usage.unresolved
                ),
            ));
        }
        let mut axioms = std::collections::BTreeSet::new();
        for dependency in usage.axioms {
            // Checked opaque proofs stay opaque to conversion, while their
            // recorded proof dependencies remain visible to the trust audit.
            if let Some(i) = self.audited_names.get(&dependency) {
                axioms.extend(self.report.declarations[*i].axioms.iter().cloned());
            } else {
                axioms.insert(dependency);
            }
        }
        if kind == "axiom" {
            axioms.insert(name.to_string());
        }
        self.trace_stage("audit", "done");
        Ok(DeclarationAudit {
            name: name.into(),
            kind: kind.into(),
            type_expression: index(&source["type"])?,
            value_expression: body.map(|_| index(&source["value"])).transpose()?,
            axioms: axioms.into_iter().collect(),
        })
    }

    fn native_nested_recursors(
        &mut self,
        members: &[crate::NestedDecl],
        nparams: usize,
        family_levels: &[String],
        recs: &[Value],
        all: &[String],
        nested_count: usize,
    ) -> Result<()> {
        fn compare(ctx: &Context, actual: &Term, expected: &Term, stage: &str) -> Result<()> {
            exact(ctx, actual, expected).map_err(|error| error.located(stage))
        }
        let native = crate::inductive_compile::derive_nested_block_recursors(
            &self.ctx,
            &crate::inductive_compile::NestedBlock {
                members,
                num_params: nparams,
                levels: family_levels,
            },
        )
        .map_err(inductive_verdict)?;
        let motives = native.len();
        if nested_count.checked_add(members.len()) != Some(motives) || recs.len() != motives {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "native nested recursor inventory mismatch",
            ));
        }
        // Carriers and constructor groups are open exactly over the shared parameters; the
        // parameters are in scope for their sorts and telescopes.
        let parameters = native
            .first()
            .map(|rec| rec.parameters.clone())
            .unwrap_or_default();
        if parameters.len() != nparams {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "native nested reconstruction lost the uniform parameter telescope",
            ));
        }
        let mut local = self.ctx.clone();
        for (parameter, domain) in &parameters {
            local = local.extend(parameter, domain.clone());
        }
        // The sort a carrier's family lives in, after its indices.
        let family_sort = |carrier: &Term| -> Result<Universe> {
            let mut ty = normal(&local, &infer_type(&local, carrier).map_err(kernel)?)?;
            while let Term::Pi { body_type, .. } = ty {
                ty = *body_type;
            }
            match ty {
                Term::Sort(level) => Ok(level),
                _ => Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "a nested carrier is not a family of types",
                )),
            }
        };
        // Each group contains original constructors partially applied to the
        // fixed container arguments, with their original field telescope. Every carrier
        // lives in the block's own sort, as every member of Lean's auxiliary block does.
        let block_sort = family_sort(&native[0].carrier)?;
        let mut constructor_groups = Vec::new();
        let mut indices = Vec::new();
        let mut minors = 0usize;
        for rec in &native {
            if !family_sort(&rec.carrier)?.equiv(&block_sort) {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native nested container outside the family's universe",
                ));
            }
            let mut head = &rec.carrier;
            let mut arguments = Vec::new();
            while let Term::App(f, a) = head {
                arguments.push((**a).clone());
                head = f;
            }
            arguments.reverse();
            let (parent, levels) = match head {
                Term::Global(name) => (name, Vec::new()),
                Term::Const { name, levels } => (name, levels.clone()),
                _ => {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "native nested carrier has no inductive head",
                    ))
                }
            };
            if self.ctx.inductive_num_params(parent) != arguments.len() {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "native nested container parameters are not fully fixed",
                ));
            }
            indices.push(self.ctx.inductive_arity(parent) - arguments.len());
            let mut constructors = Vec::new();
            for (name, _) in self.ctx.get_constructors(parent) {
                let mut value = match head {
                    Term::Const { .. } => Term::Const {
                        name: name.into(),
                        levels: levels.clone(),
                    },
                    _ => Term::Global(name.into()),
                };
                let mut ty = infer_type(&self.ctx, &value).map_err(kernel)?;
                for argument in &arguments {
                    let Term::Pi {
                        param, body_type, ..
                    } = ty
                    else {
                        return Err(declined(
                            DiagnosticCode::UnsupportedFeature,
                            "native constructor parameter telescope unavailable",
                        ));
                    };
                    ty = crate::substitute(&body_type, &param, argument);
                    value = apply(value, argument.clone());
                }
                let fields = resolved_telescope_len(&local, &ty)?;
                constructors.push((name.to_string(), value, ty, arguments.len(), fields));
            }
            minors = minors.checked_add(constructors.len()).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "native minor inventory overflow",
                )
            })?;
            constructor_groups.push(constructors);
        }
        let prefix = nparams
            .checked_add(motives)
            .and_then(|n| n.checked_add(minors))
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "native recursor prefix overflow",
                )
            })?;
        let mut checked = Vec::new();
        // All checked implementations enter scope before decoding any exported
        // computation rule, because the rules refer to sibling recursors.
        for (i, rec) in native.into_iter().enumerate() {
            // Each source member's own recursor, then the auxiliaries' as `rec_1, rec_2, …` of
            // the block's first member.
            let (owner, suffix) = if i < members.len() {
                (&members[i].name, "rec".to_string())
            } else {
                (&members[0].name, format!("rec_{}", i + 1 - members.len()))
            };
            let name = format!("{}/s:{}", owner, serde_json::to_string(&suffix).unwrap());
            let position = recs
                .iter()
                .position(|r| self.name(&r["name"]).ok().as_deref() == Some(name.as_str()))
                .ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "missing canonical native nested recursor",
                    )
                })?;
            let exported = &recs[position];
            if index(&exported["numParams"])? != nparams
                || index(&exported["numIndices"])? != indices[i]
                || index(&exported["numMotives"])? != motives
                || index(&exported["numMinors"])? != minors
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested recursor telescope counts mismatch",
                ));
            }
            if array(&exported["all"])?
                .iter()
                .map(|n| self.name(n))
                .collect::<Result<Vec<_>>>()?
                != all
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested recursor block membership mismatch",
                ));
            }
            if exported["k"].as_bool() != Some(false) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "a nested recursor has an invalid K flag",
                ));
            }
            // Lean's auxiliary block is a mutual block: it eliminates into every sort through
            // one shared motive universe when its members are types, and only into `Prop`
            // when they may be propositions (no single-member exception applies).
            let large = Universe::Type(0).is_subtype_of(&block_sort);
            let levels = self.parameters(exported)?;
            if levels.len() != usize::from(large) + family_levels.len() {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested recursor universe telescope mismatch",
                ));
            }
            let derived_motives = rec
                .levels
                .len()
                .checked_sub(family_levels.len())
                .filter(|&m| rec.levels[m..] == *family_levels)
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::UnsupportedFeature,
                        "native reconstruction has unexpected motive universe parameters",
                    )
                })?;
            // The LOGOS generator supports independent motive universes; this checked
            // instantiation supplies Lean's single motive universe (or `Prop`) to all of them
            // and Lean's names to the family's own levels.
            let offset = usize::from(large);
            let substitution = rec
                .levels
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let lean = if i < derived_motives {
                        if large {
                            Universe::Var(levels[0].clone())
                        } else {
                            Universe::Prop
                        }
                    } else {
                        Universe::Var(levels[offset + (i - derived_motives)].clone())
                    };
                    (p.clone(), lean)
                })
                .collect();
            let ty = crate::instantiate_universes(&rec.ty, &substitution);
            let body = crate::instantiate_universes(&rec.body, &substitution);
            let expected = self.declaration_term(&exported["type"], &levels)?;
            lean_type(&self.ctx, &expected)?;
            compare(
                &self.ctx,
                &expected,
                &ty,
                &format!("native recursor {name} type"),
            )?;
            if levels.is_empty() {
                self.ctx.declare(&name, expected, body).map_err(kernel)?;
            } else {
                self.ctx
                    .declare_universe_poly(&name, levels.clone(), expected, body)
                    .map_err(kernel)?;
            }
            self.declarations.insert(name.clone(), levels.len());
            checked.push((name, levels, ty, position));
        }
        // The rule telescope binds the shared parameters first. Constructor groups are
        // instantiated at those binders, never at the canonical names they were built over.
        let rule_parameters: Vec<(String, Term)> = parameters
            .iter()
            .enumerate()
            .map(|(i, (parameter, _))| (parameter.clone(), Term::Var(format!("arena.rule.{i}"))))
            .collect();
        let offset = usize::from(Universe::Type(0).is_subtype_of(&block_sort));
        let at_rule = |term: &Term, levels: &[String]| {
            let universes = family_levels
                .iter()
                .cloned()
                .zip(levels[offset..].iter().cloned().map(Universe::Var))
                .collect();
            rule_parameters.iter().fold(
                crate::instantiate_universes(term, &universes),
                |term, (parameter, binder)| crate::substitute(&term, parameter, binder),
            )
        };
        for ((name, levels, ty, position), constructors) in checked.iter().zip(&constructor_groups)
        {
            let rules = array(&recs[*position]["rules"])?;
            if rules.len() != constructors.len() {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "native nested recursor computation rule inventory mismatch",
                ));
            }
            let mut seen = HashSet::new();
            for rule in rules {
                let constructor = self.name(&rule["ctor"])?;
                let Some((_, value, ctor_ty, result_parameters, fields)) =
                    constructors.iter().find(|(n, ..)| n == &constructor)
                else {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "native nested rule names a foreign constructor",
                    ));
                };
                if !seen.insert(constructor) || index(&rule["nfields"])? != *fields {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "native nested rule has duplicate constructor or wrong field count",
                    ));
                }
                let expected = ctor_rule_applied(
                    &self.ctx,
                    name,
                    levels,
                    ty,
                    at_rule(value, levels),
                    at_rule(ctor_ty, levels),
                    0,
                    prefix,
                    *result_parameters,
                )?;
                let actual = self.declaration_term(&rule["rhs"], levels)?;
                let actual_ty = lean_type(&self.ctx, &actual)?;
                let expected_ty = infer_type(&self.ctx, &expected).map_err(kernel)?;
                compare(
                    &self.ctx,
                    &actual_ty,
                    &expected_ty,
                    &format!("native recursor {name} rule type"),
                )?;
                compare(
                    &self.ctx,
                    &actual,
                    &expected,
                    &format!("native recursor {name} rule value"),
                )?;
            }
        }
        Ok(())
    }

    /// Formatting and identifier allocation are entirely opt-in. The sink is
    /// diagnostic-only: I/O failure cannot change a kernel verdict.
    fn trace_stage(&self, stage: &str, event: &str) {
        emit_stage(
            self.trace_sink,
            self.trace_declaration.as_ref(),
            stage,
            event,
        );
    }

    fn record(&mut self, r: Value, line: usize) -> Result<()> {
        if self.trace_sink.is_none() {
            return self.record_inner(r);
        }
        let declaration = r
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.iter().next())
            .filter(|(kind, _)| {
                matches!(
                    kind.as_str(),
                    "def" | "axiom" | "thm" | "opaque" | "inductive" | "quot"
                )
            });
        if let Some((kind, source)) = declaration {
            let name_source = if kind == "inductive" {
                &source["types"][0]["name"]
            } else {
                &source["name"]
            };
            let name = self
                .name(name_source)
                .unwrap_or_else(|_| "<unresolved>".into());
            self.trace_declaration = Some((line, name, kind.clone(), name_source.as_u64()));
            self.trace_stage("declaration", "begin");
        }
        let result = self.record_inner(r);
        self.trace_stage("declaration", if result.is_ok() { "done" } else { "error" });
        self.trace_declaration = None;
        result
    }

    fn record_inner(&mut self, r: Value) -> Result<()> {
        let obj = r
            .as_object()
            .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "record is not an object"))?;
        if let Some(i) = r.get("in") {
            if obj.len() != 2 {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown name record fields",
                ));
            }
            let i = index(i)?;
            if self.names.contains_key(&i) {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "duplicate name index",
                ));
            }
            let (kind, x) = if let Some(x) = r.get("str") {
                ("s", x)
            } else if let Some(x) = r.get("num") {
                ("n", x)
            } else {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown name kind",
                ));
            };
            let pre = self
                .names
                .get(&index(&x["pre"])?)
                .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "unknown name prefix"))?;
            let part =
                if kind == "s" {
                    serde_json::to_string(x["str"].as_str().ok_or_else(|| {
                        declined(DiagnosticCode::MalformedExport, "bad name string")
                    })?)
                    .unwrap()
                } else {
                    index(&x["i"])?.to_string()
                };
            self.names.insert(i, format!("{pre}/{kind}:{part}"));
            return Ok(());
        }
        if let Some(i) = r.get("il") {
            if obj.len() != 2 {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown level record fields",
                ));
            }
            let i = index(i)?;
            if self.levels.contains_key(&i) {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "duplicate level index",
                ));
            }
            let mut vars = HashSet::new();
            let mut depth = 0;
            let mut get = |v: &Value| -> Result<Universe> {
                let j = index(v)?;
                let u = self.level(v, None)?;
                vars.extend(self.level_vars[&j].iter().cloned());
                depth = depth.max(self.level_depth[&j] + 1);
                if depth > 256 {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "universe depth budget",
                    ));
                }
                Ok(u)
            };
            let u = if let Some(v) = r.get("succ") {
                get(v)?.succ()
            } else if let Some(v) = r.get("param") {
                let n = self.name(v)?;
                vars.insert(n.clone());
                Universe::Var(n)
            } else if let Some(v) = r.get("max").or_else(|| r.get("imax")) {
                let pair = array(v)?;
                if pair.len() != 2 {
                    return Err(declined(DiagnosticCode::MalformedExport, "bad level pair"));
                }
                let a = Box::new(get(&pair[0])?);
                let b = Box::new(get(&pair[1])?);
                if r.get("max").is_some() {
                    Universe::Max(a, b)
                } else {
                    Universe::IMax(a, b)
                }
            } else {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown level kind",
                ));
            };
            self.levels.insert(i, u);
            self.level_vars.insert(i, vars);
            self.level_depth.insert(i, depth);
            return Ok(());
        }
        if let Some(i) = r.get("ie") {
            if obj.len() != 2 {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown expression record fields",
                ));
            }
            let i = index(i)?;
            if self.exprs.contains_key(&i) {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "duplicate expression index",
                ));
            }
            // Elaborator `have` markers are declined, never dropped (see `metadata`).
            metadata::check(&r)?;
            let refs: Vec<&Value> = if let Some(x) = r.get("app") {
                vec![&x["fn"], &x["arg"]]
            } else if let Some(x) = r.get("mdata") {
                vec![&x["expr"]]
            } else if let Some(x) = r.get("proj") {
                vec![&x["struct"]]
            } else if let Some(x) = r.get("letE") {
                vec![&x["type"], &x["value"], &x["body"]]
            } else if let Some(x) = r.get("lam").or_else(|| r.get("forallE")) {
                source_binder_kind(x)?;
                vec![&x["type"], &x["body"]]
            } else if r.get("sort").is_some()
                || r.get("bvar").is_some()
                || r.get("const").is_some()
                || r.get("natVal").is_some()
                || r.get("strVal").is_some()
            {
                vec![]
            } else {
                // Lean's kernel never sees a metavariable or a free variable: one here is a
                // declaration that was never fully elaborated, whatever else the record says.
                let code = if r.get("mvar").is_some() || r.get("fvar").is_some() {
                    DiagnosticCode::UnresolvedMetavariable
                } else {
                    DiagnosticCode::UnknownRecordKind
                };
                return Err(declined(code, "unsupported expression record"));
            };
            let mut depth = if r.get("strVal").is_some() {
                strings::layout(&r)?.1 - 1
            } else {
                0
            };
            for v in refs {
                let j = index(v)?;
                let d = self.expr_depth.get(&j).ok_or_else(|| {
                    declined(
                        DiagnosticCode::MalformedExport,
                        "expression is not a backwards DAG reference",
                    )
                })?;
                depth = depth.max(d + 1);
            }
            if let Some(data) = r.get("mdata").and_then(|x| x["data"].as_object()) {
                for (key, value) in data {
                    self.report.metadata_losses.push(MetadataLoss {
                        expression: i,
                        key: key.clone(),
                        value: value.as_str().expect("validated metadata value").into(),
                    });
                }
            }
            self.expr_depth.insert(i, depth);
            self.exprs.insert(i, r);
            return Ok(());
        }
        if obj.len() != 1 {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                "unknown declaration record fields",
            ));
        }
        let (kind, d) = obj.iter().next().unwrap();
        if kind == "quot" {
            self.trace_stage("registration", "begin");
            self.quotient(d)?;
            self.trace_stage("registration", "done");
            return Ok(());
        }
        if kind == "inductive" {
            self.trace_stage("registration", "begin");
            self.inductive(d)?;
            for field in ["types", "ctors", "recs"] {
                for declaration in array(&d[field])? {
                    self.install_binder_kinds(declaration)?;
                }
            }
            self.trace_stage("registration", "done");
            return Ok(());
        }
        if !["def", "axiom", "thm", "opaque"].contains(&kind.as_str()) {
            return Err(declined(
                DiagnosticCode::UnknownRecordKind,
                format!("unsupported declaration: {kind}"),
            ));
        }
        let mut runtime_safety = match d.get("safety") {
            None => None,
            Some(Value::String(s)) if s == "safe" => None,
            Some(Value::String(s)) if s == "unsafe" => Some("unsafe"),
            Some(Value::String(s)) if s == "partial" => Some("partial"),
            _ => {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown declaration safety",
                ))
            }
        };
        if let Some(flag) = d.get("isUnsafe") {
            let flag = flag.as_bool().ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "malformed declaration safety",
                )
            })?;
            if flag {
                if runtime_safety == Some("partial") {
                    return Err(rejected(
                        DiagnosticCode::MalformedExport,
                        "conflicting declaration safety",
                    ));
                }
                runtime_safety = Some("unsafe");
            }
        }
        if kind == "thm" && runtime_safety.is_some() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "theorem cannot be an unsafe or partial runtime declaration",
            ));
        }
        let name = self.name(&d["name"])?;
        if name.starts_with("kernel.quot.") {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "quotient identities require a canonical primitive declaration",
            ));
        }
        if matches!(name.as_str(), "Nat" | "Zero" | "Succ") {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "primitive Nat identities require a canonical inductive block",
            ));
        }
        if self.declarations.contains_key(&name) {
            return Err(rejected(
                DiagnosticCode::DuplicateDeclaration,
                "duplicate declaration",
            ));
        }
        let params = array(&d["levelParams"])?
            .iter()
            .map(|v| self.name(v))
            .collect::<Result<Vec<_>>>()?;
        let bound = params.iter().cloned().collect::<HashSet<_>>();
        if bound.len() != params.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "duplicate universe parameter",
            ));
        }
        if let Some(safety) = runtime_safety {
            self.trace_stage("runtime.quarantine", "begin");
            let type_expression = index(&d["type"])?;
            let value_expression = if kind == "axiom" {
                None
            } else {
                Some(index(&d["value"])?)
            };
            if !self.exprs.contains_key(&type_expression)
                || value_expression.is_some_and(|id| !self.exprs.contains_key(&id))
            {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    "unknown quarantined runtime expression",
                ));
            }
            // Reserve the source name, but install neither a definition nor an
            // axiom in Context. Its header and body have NOT been kernel checked.
            self.runtime_names.insert(name.clone(), safety);
            self.declarations.insert(name.clone(), params.len());
            self.report.runtime_declarations.push(RuntimeDeclaration {
                name,
                kind: kind.clone(),
                safety: safety.into(),
                type_expression,
                value_expression,
            });
            self.runtime_target_pending = true;
            self.trace_stage("runtime.quarantine", "done");
            return Ok(());
        }
        let roots = if kind == "axiom" {
            vec![&d["type"]]
        } else {
            vec![&d["type"], &d["value"]]
        };
        self.check_runtime_references(&roots)?;
        let mut budget = 100_000;
        self.trace_stage("type.parse", "begin");
        let ty = self.shared_term(&d["type"], &mut Vec::new(), &bound, &mut budget)?;
        self.trace_stage("type.parse", "done");
        self.trace_stage("type.infer", "begin");
        lean_type(&self.ctx, &ty)?;
        let u = sort(&self.ctx, &ty)?;
        self.trace_stage("type.infer", "done");
        if kind == "thm" && !u.equiv(&Universe::Prop) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "theorem type is not a proposition",
            ));
        }
        let mut audit = None;
        if kind == "axiom" {
            if self.audit_declarations {
                audit = Some(self.declaration_audit(&name, kind, d, &ty, None)?);
            }
            self.trace_stage("registration", "begin");
            self.ctx
                .add_universe_poly_declaration(&name, params.clone(), ty)
                .map_err(kernel)?;
        } else {
            self.trace_stage("body.parse", "begin");
            let body = self.shared_term(&d["value"], &mut Vec::new(), &bound, &mut budget)?;
            self.trace_stage("body.parse", "done");
            if kind == "def" && params.is_empty() && !self.audit_declarations {
                let sink = self.trace_sink;
                let declaration = self.trace_declaration.clone();
                let retain = self.retain.covers(&name, kind);
                let mut retained = None;
                self.ctx
                    .declare_with_validation::<Verdict>(&name, ty, body, |ctx, ty, body| {
                        emit_stage(sink, declaration.as_ref(), "body.infer", "begin");
                        let actual = lean_type(ctx, body)?;
                        emit_stage(sink, declaration.as_ref(), "body.infer", "done");
                        emit_stage(sink, declaration.as_ref(), "exact", "begin");
                        exact(ctx, &actual, ty)?;
                        emit_stage(sink, declaration.as_ref(), "exact", "done");
                        if retain {
                            retained = Some((name.clone(), ty.clone(), body.clone()));
                        }
                        emit_stage(sink, declaration.as_ref(), "registration", "begin");
                        Ok(())
                    })?;
                self.retained.extend(retained);
            } else {
                self.trace_stage("body.infer", "begin");
                let actual = lean_type(&self.ctx, &body)?;
                self.trace_stage("body.infer", "done");
                // Lean requires conversion here, not LOGOS's cumulative subtyping.
                self.trace_stage("exact", "begin");
                exact(&self.ctx, &actual, &ty)?;
                self.trace_stage("exact", "done");
                if self.audit_declarations {
                    audit = Some(self.declaration_audit(&name, kind, d, &ty, Some(&body))?);
                }
                // Retain an already checked source term only when the caller explicitly
                // requests it (a target and its closure). Retention never changes publication.
                if self.retain.covers(&name, kind) {
                    self.retained.push((name.clone(), ty.clone(), body.clone()));
                }
                self.trace_stage("registration", "begin");
                // Lean's kernel unfolds a theorem (lowest-priority lazy delta); only `opaque`
                // is sealed. Both are checked in both kernels before publication.
                if kind == "thm" {
                    self.ctx
                        .declare_theorem_checked(&name, params.clone(), ty, body)
                        .map_err(kernel)?;
                } else if kind == "opaque" {
                    self.ctx
                        .declare_opaque_checked(&name, params.clone(), ty, body)
                        .map_err(kernel)?;
                } else if params.is_empty() {
                    self.ctx.declare(&name, ty, body).map_err(kernel)?;
                } else {
                    self.ctx
                        .declare_universe_poly(&name, params.clone(), ty, body)
                        .map_err(kernel)?;
                }
            }
            if !self.ctx.pending_obligations().is_empty() {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "unresolved kernel obligation",
                ));
            }
        }
        self.install_binder_kinds(d)?;
        if let Some(audit) = audit {
            self.audited_names
                .insert(name.clone(), self.report.declarations.len());
            self.report.declarations.push(audit);
        }
        self.declarations.insert(name, params.len());
        // An axiom/header is not a checked safe target. The dependency-ordered
        // export must finish its runtime dependencies with a checked body.
        if kind != "axiom" {
            self.runtime_target_pending = false;
        }
        self.trace_stage("registration", "done");
        Ok(())
    }
}
/// The kernel identity of a structurally encoded Lean name (`/s:"Demo"/s:"andSwap"`).
///
/// The compact Nat representation has a fixed kernel identity. Only this datatype, its
/// constructors, and certified quotient primitives cross this boundary; every literal use checks
/// their complete imported signatures. [`intake::lean_name`] is the inverse.
pub(crate) fn canonical_name(encoded: &str) -> String {
    match encoded {
        "/s:\"Quot\"" => crate::quotient::Kind::Type.name().into(),
        "/s:\"Quot\"/s:\"mk\"" => crate::quotient::Kind::Constructor.name().into(),
        "/s:\"Quot\"/s:\"lift\"" => crate::quotient::Kind::Lift.name().into(),
        "/s:\"Quot\"/s:\"ind\"" => crate::quotient::Kind::Induction.name().into(),
        "/s:\"Nat\"" => "Nat".into(),
        "/s:\"Nat\"/s:\"zero\"" => "Zero".into(),
        "/s:\"Nat\"/s:\"succ\"" => "Succ".into(),
        _ => match encoded.strip_prefix("/s:\"Nat\"/") {
            Some(suffix) => format!("Nat/{suffix}"),
            None => encoded.to_string(),
        },
    }
}
/// The name index a declaration record declares (an inductive block by its first type).
fn declared_name_index(r: &Value) -> Option<usize> {
    let object = r.as_object().filter(|o| o.len() == 1)?;
    let (kind, source) = object.iter().next()?;
    let name = match kind.as_str() {
        "def" | "axiom" | "thm" | "opaque" | "quot" => &source["name"],
        "inductive" => &source["types"][0]["name"],
        _ => return None,
    };
    name.as_u64().and_then(|n| usize::try_from(n).ok())
}
fn record_children(r: &Value) -> Vec<&Value> {
    if let Some(x) = r.get("app") {
        vec![&x["fn"], &x["arg"]]
    } else if let Some(x) = r.get("mdata") {
        vec![&x["expr"]]
    } else if let Some(x) = r.get("proj") {
        vec![&x["struct"]]
    } else if let Some(x) = r.get("letE") {
        vec![&x["type"], &x["value"], &x["body"]]
    } else if let Some(x) = r.get("lam").or_else(|| r.get("forallE")) {
        vec![&x["type"], &x["body"]]
    } else {
        Vec::new()
    }
}
struct ImportOptions<'a> {
    trace: Option<StageSink<'a>>,
    audit_declarations: bool,
    retain: RetainSource,
    binder_names: bool,
    /// Read every `native_decide` leaf as its re-derivation by kernel evaluation ([`native_decide`]).
    native_decisions: bool,
}
/// Which checked declarations an import copies out, type and body, beyond what the context keeps.
/// A theorem's or opaque's proof is opaque in the context; only retention keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RetainSource {
    /// Nothing is copied.
    #[default]
    Nothing,
    /// The named declarations (importer keys) that have a body.
    Named(HashSet<String>),
    /// Every checked theorem and opaque declaration of the export.
    EveryProof,
}
impl RetainSource {
    fn covers(&self, name: &str, kind: &str) -> bool {
        match self {
            RetainSource::Nothing => false,
            RetainSource::Named(names) => names.contains(name),
            RetainSource::EveryProof => kind == "thm" || kind == "opaque",
        }
    }
}
/// How [`import_with`] imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    /// Run the declaration audit (independent recheck of every definition, theorem, opaque and axiom,
    /// with its transitive axioms).
    pub audit: bool,
    pub retain: RetainSource,
    /// Carry Lean's binder names into the kernel terms (`arena.local.<depth>.<name>`, read back by
    /// [`lean_binder_name`]). Checking is identical either way; the benchmark protocol keeps them off
    /// so its refusal messages stay byte-for-byte what they were.
    pub lean_binder_names: bool,
}
impl Default for ImportRequest {
    fn default() -> Self {
        ImportRequest {
            audit: false,
            retain: RetainSource::Nothing,
            lean_binder_names: true,
        }
    }
}
/// The Lean binder name an imported binder carries, when the import kept one.
pub fn lean_binder_name(param: &str) -> Option<&str> {
    let rest = param.strip_prefix("arena.local.")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then_some(())?;
    rest[digits..].strip_prefix('.').filter(|n| !n.is_empty())
}
fn check_records(
    input: &str,
    options: ImportOptions<'_>,
) -> Result<(ImportedContext, ImportReport)> {
    let mut importer = Importer::new(options.audit_declarations);
    importer.trace_sink = options.trace;
    importer.retain = options.retain;
    importer.binder_names = options.binder_names;
    importer.lean_version = input
        .lines()
        .next()
        .and_then(|header| serde_json::from_str::<Value>(header).ok())
        .and_then(|header| {
            header["meta"]["lean"]["version"]
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let native = options
        .native_decisions
        .then(|| native_decide::NativeDecisions::read(input));
    for (line, s) in input.lines().skip(1).enumerate() {
        let line = line + 2;
        let r = serde_json::from_str::<Value>(s).map_err(|e| {
            declined(DiagnosticCode::MalformedExport, format!("line {line}: {e}")).at_line(line)
        })?;
        let r = match &native {
            Some(native) => native.rederive(r),
            None => r,
        };
        let declared = declared_name_index(&r);
        importer.record(r, line).map_err(|verdict| {
            let verdict = match &native {
                Some(native) => native.explain(verdict, declared),
                None => verdict,
            };
            let verdict = verdict.located(format!("line {line}")).at_line(line);
            match declared.and_then(|i| importer.names.get(&i)) {
                Some(name) => verdict.in_declaration(intake::lean_name(&canonical_name(name))),
                None => verdict,
            }
        })?;
    }
    if importer.runtime_target_pending {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            RUNTIME_UNCHECKED,
        ));
    }
    let depth = importer.expr_depth.values().copied().max().unwrap_or(0);
    importer.report.native_decisions = native
        .map(|native| native.rederivations())
        .unwrap_or_default();
    Ok((
        ImportedContext {
            context: Some(importer.ctx),
            depth,
            retained: importer.retained,
            requested: None,
        },
        importer.report,
    ))
}
pub fn check_str(input: &str) -> Result<()> {
    check_str_with_report(input).map(|_| ())
}
pub fn check_str_with_report(input: &str) -> Result<ImportReport> {
    check_import(input, false).map(|(_, report)| report)
}
fn check_import(input: &str, audit_declarations: bool) -> Result<(ImportedContext, ImportReport)> {
    check_import_with_trace(input, audit_declarations, None)
}
fn check_import_with_trace(
    input: &str,
    audit_declarations: bool,
    trace: Option<StageSink<'_>>,
) -> Result<(ImportedContext, ImportReport)> {
    check_import_with_options(
        input,
        ImportOptions {
            audit_declarations,
            trace,
            retain: RetainSource::Nothing,
            binder_names: false,
            native_decisions: false,
        },
    )
}
fn check_import_with_options(
    input: &str,
    options: ImportOptions<'_>,
) -> Result<(ImportedContext, ImportReport)> {
    let mut lines = input.lines();
    let header: Value = serde_json::from_str(
        lines
            .next()
            .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "missing header"))?,
    )
    .map_err(|e| declined(DiagnosticCode::MalformedExport, e.to_string()).at_line(1))?;
    if header["meta"]["format"]["version"] != "3.1.0" {
        return Err(declined(
            DiagnosticCode::UnsupportedExportVersion,
            "unsupported NDJSON format version",
        )
        .at_line(1));
    }
    // Retain only depth numbers in this preliminary pass. Holding every JSON
    // declaration as a Value until checking would duplicate large environments.
    // record() still checks all backwards references and declaration contracts.
    let mut depths = HashMap::<usize, usize>::new();
    let mut depth = 0usize;
    for (line, s) in lines.enumerate() {
        let r = serde_json::from_str::<Value>(s).map_err(|e| {
            declined(
                DiagnosticCode::MalformedExport,
                format!("line {}: {e}", line + 2),
            )
            .at_line(line + 2)
        })?;
        if let Some(id) = r.get("ie").and_then(|v| index(v).ok()) {
            let children = record_children(&r);
            let d = if r.get("strVal").is_some() {
                strings::layout(&r)?.1
            } else {
                children
                    .iter()
                    .filter_map(|v| index(v).ok())
                    .filter_map(|id| depths.get(&id))
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
            };
            depths.insert(id, d);
            depth = depth.max(d);
        }
    }
    drop(depths);
    if depth <= crate::term_store::INLINE_DEPTH {
        return check_records(input, options);
    }
    // Do not enter the kernel's sized-call guard here: individual kernel calls
    // must still size their own stack from the actual terms they receive.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("lean-arena-import".into())
            .stack_size(crate::term_store::stack_bytes_for_depth(depth))
            .spawn_scoped(scope, move || check_records(input, options))
            .map_err(|e| {
                declined(
                    DiagnosticCode::Infrastructure,
                    format!("cannot reserve importer stack: {e}"),
                )
            })?
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub internal_helpers: Vec<InternalHelper>,
    pub metadata_losses: Vec<MetadataLoss>,
    pub declarations: Vec<DeclarationAudit>,
    pub declaration_audit_requested: bool,
    pub runtime_declarations: Vec<RuntimeDeclaration>,
    /// Declarations whose `native_decide` steps were re-derived by kernel evaluation.
    pub native_decisions: Vec<native_decide::Rederivation>,
}
/// Checked conservative definitions introduced to preserve source DAG sharing.
#[derive(Debug, PartialEq, Eq)]
pub struct InternalHelper {
    pub name: String,
    pub source_expression: usize,
    pub level_parameters: Vec<String>,
    pub captured_parameters: usize,
}
/// Source-only runtime metadata. No corresponding axiom or body enters Context.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeDeclaration {
    pub name: String,
    pub kind: String,
    pub safety: String,
    pub type_expression: usize,
    pub value_expression: Option<usize>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct DeclarationAudit {
    pub name: String,
    pub kind: String,
    pub type_expression: usize,
    pub value_expression: Option<usize>,
    pub axioms: Vec<String>,
}
/// Owned imported artifacts keep the importer's stack discipline when released.
pub struct ImportedContext {
    context: Option<Context>,
    depth: usize,
    /// Checked (type, body) copies of the requested declarations, in export order.
    retained: Vec<(String, Term, Term)>,
    /// The one declaration `import_audited_retaining_source` asked for.
    requested: Option<String>,
}
impl ImportedContext {
    /// The checked type and body of the declaration named by `import_audited_retaining_source`.
    pub fn retained_source(&self) -> Option<(&str, &Term, &Term)> {
        let requested = self.requested.as_deref()?;
        self.retained(requested)
            .map(|(ty, body)| (requested, ty, body))
    }
    /// The retained checked type and body of `name` — including an opaque theorem's proof, which
    /// the context itself keeps opaque — when the import was asked to retain it.
    pub fn retained(&self, name: &str) -> Option<(&Term, &Term)> {
        self.retained
            .iter()
            .find(|(n, ..)| n == name)
            .map(|(_, ty, body)| (ty, body))
    }
    /// Every retained declaration with its checked type and body, in export (dependency) order.
    pub fn retained_declarations(&self) -> impl Iterator<Item = (&str, &Term, &Term)> {
        self.retained
            .iter()
            .map(|(name, ty, body)| (name.as_str(), ty, body))
    }
}
impl std::ops::Deref for ImportedContext {
    type Target = Context;
    fn deref(&self) -> &Context {
        self.context.as_ref().expect("live imported context")
    }
}
impl Drop for ImportedContext {
    fn drop(&mut self) {
        let mut context = self.context.take();
        let source = std::mem::take(&mut self.retained);
        if self.depth <= crate::term_store::INLINE_DEPTH {
            drop(context);
            drop(source);
            return;
        }
        // The input DAG already determined the importer's stack reservation.
        // Retaining its context must not move deep Box destruction onto an
        // arbitrary caller stack when that caller finishes inspecting artifacts.
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("lean-import-drop".into())
                .stack_size(crate::term_store::stack_bytes_for_depth(self.depth))
                .spawn_scoped(scope, move || {
                    drop(context.take());
                    drop(source);
                })
                .expect("reserve imported-context destruction stack")
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p));
        });
    }
}
pub fn import_audited(input: &str) -> Result<(ImportedContext, ImportReport)> {
    check_import(input, true)
}
/// Import and additionally retain the checked type and body of one named declaration, including an
/// opaque theorem's proof. Only that declaration is copied; every other import retains nothing extra.
pub fn import_audited_retaining_source(
    input: &str,
    name: &str,
) -> Result<(ImportedContext, ImportReport)> {
    let imported = check_import_with_options(
        input,
        ImportOptions {
            audit_declarations: true,
            trace: None,
            retain: RetainSource::Named([name.to_string()].into_iter().collect()),
            binder_names: false,
            native_decisions: false,
        },
    )?;
    let imported = request(imported, name)?;
    if imported.0.retained(name).is_none() {
        return Err(declined(
            DiagnosticCode::MissingDeclaration,
            "requested source declaration has no checked body",
        ));
    }
    Ok(imported)
}
/// Import with the declaration audit, retaining the checked type and body of every named declaration
/// that has one — a target and its dependency closure ([`intake::Intake::closure`]). Nothing outside
/// `names` is copied: retention costs the closure, never the whole environment.
pub fn import_audited_retaining(
    input: &str,
    names: &[String],
) -> Result<(ImportedContext, ImportReport)> {
    import_with(
        input,
        ImportRequest {
            audit: true,
            retain: RetainSource::Named(names.iter().cloned().collect()),
            lean_binder_names: true,
        },
    )
}
/// Import the export and return its checked context: no audit, nothing retained, Lean's binder
/// names carried.
pub fn import(input: &str) -> Result<(ImportedContext, ImportReport)> {
    import_with(input, ImportRequest::default())
}
/// Import as `request` asks. This is LOGOS's import surface: every `native_decide` step is read as
/// its re-derivation by kernel evaluation and listed in the report's `native_decisions`
/// ([`native_decide`]); the Arena protocol ([`check_str`]) reads the export exactly as exported.
pub fn import_with(input: &str, request: ImportRequest) -> Result<(ImportedContext, ImportReport)> {
    check_import_with_options(
        input,
        ImportOptions {
            audit_declarations: request.audit,
            trace: None,
            retain: request.retain,
            binder_names: request.lean_binder_names,
            native_decisions: true,
        },
    )
}
/// The pipeline's import of `target`: audited, retaining `target`'s dependency closure, with
/// `target` as the [`retained_source`](ImportedContext::retained_source).
pub fn import_retaining_closure(
    input: &str,
    target: &str,
    closure: &[String],
) -> Result<(ImportedContext, ImportReport)> {
    request(import_audited_retaining(input, closure)?, target)
}
/// Why an import that ends on, or is asked for, an unchecked runtime declaration is declined.
const RUNTIME_UNCHECKED: &str =
    "runtime declarations remain unchecked without a subsequent checked safe target";
/// Name `target` as the declaration an import was made for. A runtime (`unsafe`/`partial`)
/// declaration is quarantined, never kernel checked, so a request for one is declined however many
/// checked declarations the export holds after it.
fn request(
    mut imported: (ImportedContext, ImportReport),
    target: &str,
) -> Result<(ImportedContext, ImportReport)> {
    if imported
        .1
        .runtime_declarations
        .iter()
        .any(|runtime| runtime.name == target)
    {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            RUNTIME_UNCHECKED,
        ));
    }
    imported.0.requested = Some(target.to_string());
    Ok(imported)
}
pub fn check_str_with_declaration_audit(input: &str) -> Result<ImportReport> {
    check_import(input, true).map(|(_, report)| report)
}

#[derive(Debug, PartialEq, Eq)]
pub struct MetadataLoss {
    pub expression: usize,
    pub key: String,
    pub value: String,
}
impl ImportReport {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": "logos-lean-import-report/1",
            "acceptance_scope": "safe_kernel_environment",
            "internal_helpers": self.internal_helpers.iter().map(|h|serde_json::json!({
                "name":h.name, "source_expression":h.source_expression,
                "level_parameters":h.level_parameters, "captured_parameters":h.captured_parameters,
                "status":"independently_checked_definition",
            })).collect::<Vec<_>>(),
            "runtime_declarations": self.runtime_declarations.iter().map(|d| serde_json::json!({
                "name": d.name, "kind": d.kind, "safety": d.safety,
                "type_expression": d.type_expression, "value_expression": d.value_expression,
                "status": "quarantined_unchecked",
            })).collect::<Vec<_>>(),
            "declaration_audit": {
                "requested": self.declaration_audit_requested,
                "scope": ["def", "thm", "opaque", "axiom"],
                "declarations": self.declarations.iter().map(|d| serde_json::json!({
                    "name": d.name, "kind": d.kind, "type_expression": d.type_expression,
                    "value_expression": d.value_expression, "axioms": d.axioms,
                })).collect::<Vec<_>>(),
            },
            "losses": self.metadata_losses.iter().map(|loss| serde_json::json!({
                "category": "SOURCE_METADATA",
                "expression": loss.expression,
                "key": loss.key,
                "exported_value": loss.value,
            })).collect::<Vec<_>>(),
            "native_decisions": self.native_decisions.iter()
                .map(native_decide::Rederivation::to_json).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/lean_type_stack_controls.rs"]
mod lean_type_stack_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/exact_decision_controls.rs"]
mod exact_decision_controls;
#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/function_type_head_controls.rs"]
mod function_type_head_controls;
#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/independent_verdict_controls.rs"]
mod independent_verdict_controls;

// Canonical source reconstruction participates in package source binding.
mod source_recursor;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference_projection.rs"]
mod reference_projection;

mod source_recursor_assembly;

mod reference_projection_shared;

mod runtime_refusal;
