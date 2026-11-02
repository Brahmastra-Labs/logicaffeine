//! Independently checked natural recurrence acceleration for the de Bruijn kernel.
use super::*;
use std::collections::HashMap;

// Compact values belong to conversion observations. Ordinary WHNF retains its
// established Peano syntax and prefix-cache allocation behavior.
thread_local! { static OBSERVE:std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
pub(super) struct ConversionScope(bool);
impl ConversionScope {
    pub(super) fn enter() -> Self {
        Self(OBSERVE.with(|v| v.replace(true)))
    }
}
impl Drop for ConversionScope {
    fn drop(&mut self) {
        OBSERVE.with(|v| v.set(self.0));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Sum,
    Difference,
    Product,
    Power,
}
fn global(n: &str) -> Db {
    Db::Global(n.into())
}
fn apply(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn binary(f: &str, a: Db, b: Db) -> Db {
    apply(apply(global(f), a), b)
}

/// A base equation authorizes only the current observation at a zero right
/// operand. It is never an arithmetic recurrence or a dependency certificate.
#[derive(Clone, Copy)]
enum Certificate {
    Recurrence(Op),
    RightZeroIdentity,
}

fn certify(
    genv: &Context,
    name: &str,
    depth: usize,
    seen: &mut HashMap<String, Option<Op>>,
) -> Option<Op> {
    match certify_observation(genv, name, depth, seen, false)? {
        Certificate::Recurrence(op) => Some(op),
        Certificate::RightZeroIdentity => None,
    }
}
fn certify_observation(
    genv: &Context,
    name: &str,
    depth: usize,
    seen: &mut HashMap<String, Option<Op>>,
    right_is_zero: bool,
) -> Option<Certificate> {
    if depth >= 8 || WHNF_EXHAUSTED.with(|x| x.get()) {
        return None;
    }
    if let Some(op) = seen.get(name) {
        return op.map(Certificate::Recurrence);
    }
    if seen.len() >= 64 || !nat_order_tick() {
        return None;
    }
    seen.insert(name.into(), None);
    let result = certify_body(genv, name, depth, seen, right_is_zero);
    // Keep the memo restricted to full recurrences, including in-progress and
    // refused entries. A partial observation must not certify any dependency.
    if let Some(Certificate::Recurrence(op)) = result {
        seen.insert(name.into(), Some(op));
    }
    result
}
/// The syntactic admission every recurrence certificate needs: a monomorphic,
/// checked, transparent definition declared exactly `Nat → Nat → Nat`. It reads
/// only the declaration header, so it costs no reduction work.
fn binary_nat_definition<'a>(genv: &'a Context, name: &str) -> Option<&'a Term> {
    if genv.poly_declaration(name).is_some()
        || genv.is_constructor(name)
        || genv.is_inductive(name)
        || genv.has_pending_dependency(&Term::Global(name.into()))
    {
        return None;
    }
    let ty = genv.get_definition_type(name)?;
    if genv.get_global(name) != Some(ty) {
        return None;
    }
    let mut result = ty;
    for _ in 0..2 {
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = result
        else {
            return None;
        };
        if !matches!(&**param_type,Term::Global(n) if n=="Nat") {
            return None;
        }
        result = body_type;
    }
    if !matches!(result,Term::Global(n) if n=="Nat") {
        return None;
    }
    genv.get_definition_body(name)
}
fn certify_body(
    genv: &Context,
    name: &str,
    depth: usize,
    seen: &mut HashMap<String, Option<Op>>,
    right_is_zero: bool,
) -> Option<Certificate> {
    let body = binary_nat_definition(genv, name)?;
    let limit = WHNF_FUEL.with(|x| x.get().min(ADMISSION));
    let mut admission = LocalAdmission::new(limit);
    let free = admission.free_names(body);
    if !nat_order_spend(limit - admission.remaining + 1)
        || admission.max_depth > MAX_RECHECK_WHNF_DEPTH
        || !free.ok()?.is_empty()
    {
        return None;
    }
    let body = to_db(body, &mut Vec::new()).ok()?;
    let ctx = genv.order_recurrence_context();
    let locals = vec![global("Nat"), global("Nat")];
    let x = Db::Var(1);
    let y = Db::Var(0);
    let zero = global("Zero");
    let base = binary(name, x.clone(), zero.clone());
    let step = binary(name, x.clone(), apply(global("Succ"), y.clone()));
    let previous = binary(name, x.clone(), y);
    if def_eq(&ctx, &locals, &base, &x) {
        // Prove the equation over a free Nat, not just the concrete operand.
        // Its instantiation suffices at zero even if the successor case is
        // unrelated or expensive. All admission/proof work remains charged.
        if right_is_zero && !WHNF_EXHAUSTED.with(|x| x.get()) {
            return Some(Certificate::RightZeroIdentity);
        }
        if def_eq(
            &ctx,
            &locals,
            &step,
            &apply(global("Succ"), previous.clone()),
        ) {
            return (!WHNF_EXHAUSTED.with(|x| x.get())).then_some(Certificate::Recurrence(Op::Sum));
        }
        // Truncated subtraction: `f x (n+1) = p (f x n)` for a program `p` named in the body that
        // this kernel certifies is the predecessor (`p 0 = 0`, `p (k+1) = k`; Lean compiles
        // `n - (m+1)` to `Nat.pred (n - m)`). By induction on `n`, `f x n` is `x` with `n`
        // successors removed and none below zero.
        for p in candidate_names(genv, &body, limit)? {
            if predecessor(genv, &p)
                && def_eq(&ctx, &locals, &step, &apply(global(&p), previous.clone()))
            {
                return (!WHNF_EXHAUSTED.with(|x| x.get()))
                    .then_some(Certificate::Recurrence(Op::Difference));
            }
        }
    }
    let op = if def_eq(&ctx, &locals, &base, &zero) {
        Op::Product
    } else if def_eq(&ctx, &locals, &base, &apply(global("Succ"), zero)) {
        Op::Power
    } else {
        return None;
    };
    let dependency = if op == Op::Product {
        Op::Sum
    } else {
        Op::Product
    };
    for n in candidate_names(genv, &body, limit)? {
        if certify(genv, &n, depth + 1, seen) != Some(dependency) {
            continue;
        }
        if (def_eq(
            &ctx,
            &locals,
            &step,
            &binary(&n, previous.clone(), x.clone()),
        ) || def_eq(
            &ctx,
            &locals,
            &step,
            &binary(&n, x.clone(), previous.clone()),
        )) && !WHNF_EXHAUSTED.with(|x| x.get())
        {
            return Some(Certificate::Recurrence(op));
        }
    }
    None
}

/// The names an admitted body mentions — the candidates a recurrence's dependency is chosen from.
/// The scan never invokes the named kernel or treats any spelling as an arithmetic primitive: a
/// candidate counts only after its own certificate and the step equation are proved by conversion.
/// Lean compiles structural recursion through helpers (`Nat.mul := brecOn .. Nat.mul._f`), so the
/// scan also reads the bodies of closed monomorphic helpers it meets.
fn candidate_names(genv: &Context, body: &Db, limit: usize) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut helpers = vec![body.clone()];
    let mut read = 0usize;
    while let Some(helper) = helpers.pop() {
        let mut pending = vec![&helper];
        while let Some(t) = pending.pop() {
            if !nat_order_tick() {
                return None;
            }
            match t {
                Db::Global(n) => {
                    if !names.contains(n) {
                        names.push(n.clone());
                    }
                }
                Db::App(a, b) | Db::Pi(a, b) | Db::Lam(a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                Db::Fix(b) => pending.push(b),
                Db::MutualFix { defs, .. } => pending.extend(defs),
                Db::Let(a, b, c) => {
                    pending.push(a);
                    pending.push(b);
                    pending.push(c);
                }
                Db::Match {
                    disc,
                    motive,
                    cases,
                } => {
                    pending.push(disc);
                    pending.push(motive);
                    pending.extend(cases);
                }
                Db::Sort(_) | Db::Var(_) | Db::Const { .. } | Db::Lit(_) => {}
            }
        }
        while read < names.len() && helpers.is_empty() {
            let candidate = names[read].clone();
            read += 1;
            if names.len() > ARITHMETIC_SCAN_NAMES {
                break;
            }
            if let Some(helper) = helper_body(genv, &candidate, limit) {
                helpers.push(helper);
            }
        }
    }
    Some(names)
}

/// Whether the monomorphic definition `name : Nat → Nat` is the predecessor, by this kernel's own
/// proof of its two equations `name 0 = 0` and `name (k+1) = k` for a free `k`.
fn predecessor(genv: &Context, name: &str) -> bool {
    if genv.poly_declaration(name).is_some()
        || genv.is_constructor(name)
        || genv.is_inductive(name)
        || genv.get_definition_body(name).is_none()
    {
        return false;
    }
    let Some(ty) = genv.get_definition_type(name) else {
        return false;
    };
    if genv.get_global(name) != Some(ty)
        || !matches!(ty,Term::Pi{param_type,body_type,..} if matches!(param_type.as_ref(),Term::Global(n) if n=="Nat")
            && matches!(body_type.as_ref(),Term::Global(n) if n=="Nat"))
    {
        return false;
    }
    let ctx = genv.order_recurrence_context();
    def_eq(
        &ctx,
        &Vec::<Db>::new(),
        &apply(global(name), global("Zero")),
        &global("Zero"),
    ) && def_eq(
        &ctx,
        &vec![global("Nat")],
        &apply(global(name), apply(global("Succ"), Db::Var(0))),
        &Db::Var(0),
    ) && !WHNF_EXHAUSTED.with(|x| x.get())
}

/// Whether `name` is certified as truncated subtraction by its own recurrence equations.
pub(super) fn certifies_difference(genv: &Context, name: &str) -> bool {
    certify(genv, name, 0, &mut HashMap::new()) == Some(Op::Difference)
}

/// Whether compact numerals are being observed: this kernel accelerates only inside a conversion.
pub(super) fn observing() -> bool {
    OBSERVE.with(|v| v.get())
}

/// Candidate names read through helpers before the dependency search stops looking.
const ARITHMETIC_SCAN_NAMES: usize = 64;
/// The lowered body of a closed, monomorphic, transparent definition (not an inductive or
/// constructor), admitted under the same bounds as the operation's own body.
fn helper_body(genv: &Context, name: &str, limit: usize) -> Option<Db> {
    if genv.poly_declaration(name).is_some()
        || genv.is_constructor(name)
        || genv.is_inductive(name)
        || genv.has_pending_dependency(&Term::Global(name.into()))
    {
        return None;
    }
    let body = genv.get_definition_body(name)?;
    let mut admission = LocalAdmission::new(limit);
    let free = admission.free_names(body);
    if !nat_order_spend(limit - admission.remaining + 1)
        || admission.max_depth > MAX_RECHECK_WHNF_DEPTH
        || !free.ok()?.is_empty()
    {
        return None;
    }
    to_db(body, &mut Vec::new()).ok()
}

// Charge a conservative schoolbook bound before allocating the product.
// This implementation shares integer storage, not a certificate or evaluator.
fn primitive(op: Op, a: &crate::BigInt, b: &crate::BigInt) -> Option<crate::BigInt> {
    if a.is_negative() || b.is_negative() {
        return None;
    }
    let aa = a.to_le_bytes().1.len().div_ceil(8);
    let bb = b.to_le_bytes().1.len().div_ceil(8);
    let (size, work) = match op {
        Op::Sum => (aa.max(bb).checked_add(1)?, aa.max(bb).max(1)),
        Op::Difference => (aa, aa.max(bb).max(1)),
        Op::Product => (aa.checked_add(bb)?, aa.max(1).checked_mul(bb.max(1))?),
        Op::Power => return None,
    };
    if size > 16_384 || !nat_order_spend(work) {
        return None;
    }
    Some(match op {
        Op::Sum => a.add(b),
        Op::Difference => {
            if a < b {
                crate::BigInt::zero()
            } else {
                a.sub(b)
            }
        }
        _ => a.mul(b),
    })
}
fn calculate(op: Op, a: &crate::BigInt, b: &crate::BigInt) -> Option<crate::BigInt> {
    if a.is_negative() || b.is_negative() {
        return None;
    }
    if op != Op::Power {
        return primitive(op, a, b);
    }
    let unit = crate::BigInt::from_u64(1);
    if b.is_zero() || *a == unit {
        return Some(unit);
    }
    if a.is_zero() {
        return Some(crate::BigInt::zero());
    }
    let exponent = u32::try_from(b.to_i64()?).ok()?;
    let mut value = unit;
    // Most-significant-bit fold; the main evaluator uses a different,
    // least-significant-bit accumulator and squared-base loop.
    for shift in (0..(u32::BITS - exponent.leading_zeros())).rev() {
        value = primitive(Op::Product, &value, &value)?;
        if (exponent >> shift) & 1 != 0 {
            value = primitive(Op::Product, &value, a)?;
        }
    }
    Some(value)
}
pub(super) fn numeral(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<crate::BigInt> {
    let nodes = nat_order_admit(t)?;
    if !nat_order_spend(nodes.checked_add(1)?) {
        return None;
    }
    let mut current = whnf_in(genv, lctx, t);
    let mut count = crate::BigInt::zero();
    loop {
        if !nat_order_tick() || WHNF_EXHAUSTED.with(|x| x.get()) {
            return None;
        }
        match current {
            Db::Lit(Literal::Nat(n)) if !n.is_negative() => return primitive(Op::Sum, &count, &n),
            Db::Global(n) if n == "Zero" => return Some(count),
            Db::App(f, a) if matches!(&*f,Db::Global(n) if n=="Succ") => {
                count = primitive(Op::Sum, &count, &crate::BigInt::from_u64(1))?;
                current = whnf_in_owned(genv, lctx, a.into_db());
            }
            _ => return None,
        }
    }
}
pub(super) fn reduce(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    if !OBSERVE.with(|v| v.get())
        || genv.checking_order_recurrence
        || !genv.simp_rules().is_empty()
        || WHNF_EXHAUSTED.with(|x| x.get())
        || genv.constructor_inventory_upper_bound("Nat") > 4096
        || !crate::int_inductive::canonical_nat_environment(genv)
    {
        return None;
    }
    let Db::App(f, b) = t else {
        return None;
    };
    let Db::App(f, a) = f.as_ref() else {
        return None;
    };
    let Db::Global(name) = f.as_ref() else {
        return None;
    };
    let parent = WHNF_DEPTH.with(|x| x.get());
    if parent == 0 || parent >= MAX_RECHECK_WHNF_DEPTH {
        return None;
    }
    let _guard = WhnfDepthGuard(parent);
    WHNF_DEPTH.with(|x| x.set(parent + 1));
    // Admit the operands before certifying the operation. The base-equation
    // probes can each spend the caller's whole allowance (a well-founded
    // definition unfolds its accessibility proof even for an open argument), and
    // a certificate is useless unless both operands are numerals. The header
    // check first keeps operands of non-arithmetic heads unevaluated.
    binary_nat_definition(genv, name)?;
    let a = numeral(genv, lctx, a)?;
    let b = numeral(genv, lctx, b)?;
    // At a zero right operand the base equation alone answers, and the successor case is never
    // probed; any other observation needs the full recurrence, certified once per registry
    // generation.
    let certificate = if b.is_zero() {
        certify_observation(genv, name, 0, &mut HashMap::new(), true)?
    } else {
        Certificate::Recurrence(recurrence(genv, name)?)
    };
    match certificate {
        Certificate::RightZeroIdentity => Some(Db::Lit(Literal::Nat(a))),
        Certificate::Recurrence(op) => calculate(op, &a, &b).map(|n| Db::Lit(Literal::Nat(n))),
    }
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Sum => "sum",
            Op::Difference => "difference",
            Op::Product => "product",
            Op::Power => "power",
        }
    }
    fn named(name: &str) -> Option<Op> {
        [Op::Sum, Op::Difference, Op::Product, Op::Power]
            .into_iter()
            .find(|op| op.name() == name)
    }
}

/// The recurrence this kernel certifies `name` computes, established once per registry
/// generation, refusals included. A refusal is retained only when nothing under the
/// certification was cut short and its admission allowance was never the caller's remaining fuel
/// (`certify_body` admits at most `min(fuel, 4096)`): the certification is then a function of the
/// registries alone.
fn recurrence(genv: &Context, name: &str) -> Option<Op> {
    let concluded = genv.cached_independent_arithmetic(name, || {
        let op = certify(genv, name, 0, &mut HashMap::new());
        #[cfg(test)]
        CERTIFICATIONS.with(|n| n.set(n.get() + 1));
        let definitive =
            !WHNF_EXHAUSTED.with(|x| x.get()) && WHNF_FUEL.with(|x| x.get()) >= ADMISSION;
        (op.map(Op::name), definitive)
    })?;
    Op::named(concluded)
}

/// The admission allowance of one recurrence certificate when fuel is not scarcer.
const ADMISSION: usize = 4096;

#[cfg(test)]
thread_local! {
    /// The recurrence certifications this thread has run: the work the per-generation cache holds.
    static CERTIFICATIONS:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/arithmetic/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/arithmetic/base_equation_controls.rs"]
mod base_equation_controls;
