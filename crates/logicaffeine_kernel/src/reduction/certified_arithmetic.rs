//! Arithmetic acceleration certified from universally quantified recurrence equations.
//! No imported or prelude function name grants arithmetic semantics.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arithmetic {
    Add,
    Mul,
    Pow,
    Sub,
    Div,
    Mod,
}

fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn call(f: &Term, a: Term, b: Term) -> Term {
    app(app(f.clone(), a), b)
}

// A bounded positive conversion check. This uses existing beta/delta/iota/zeta
// rules, never arithmetic certificates, full normalization or a finite test set.
pub(super) fn equation(ctx: &Context, a: Term, b: Term) -> bool {
    if recurrence_body_admitted(&a)
        && recurrence_body_admitted(&b)
        && crate::type_checker::bounded_conversion(ctx, &a, &b, &mut 4096)
    {
        return true;
    }
    let mut pairs = vec![(a, b)];
    let mut remaining = 64;
    while let Some((a, b)) = pairs.pop() {
        if remaining == 0 {
            return false;
        }
        remaining -= 1;
        if !recurrence_body_admitted(&a) || !recurrence_body_admitted(&b) {
            return false;
        }
        let mut store = TermStore::new();
        let ai = store.intern(&a);
        let bi = store.intern(&b);
        if store.alpha_same(ai, bi) {
            continue;
        }
        let Some(a) = super::super::bounded_conversion_head(ctx, &a) else {
            return false;
        };
        let Some(b) = super::super::bounded_conversion_head(ctx, &b) else {
            return false;
        };
        let ai = store.intern(&a);
        let bi = store.intern(&b);
        if store.alpha_same(ai, bi)
            || crate::term_store::beta_zeta_equivalent(ctx, &a, &b, 1 << 14)
            || crate::term_store::beta_delta_equivalent(ctx, &a, &b, 1 << 14)
        {
            continue;
        }
        // The canonical Nat literal/constructor bridge is also a definitional
        // rule. The probe context disables arithmetic recurrence acceleration.
        if constructor_equivalent(ctx, &a, &b, 1024) {
            continue;
        }
        match (a, b) {
            (Term::App(af, aa), Term::App(bf, ba)) => {
                pairs.push((*af, *bf));
                pairs.push((*aa, *ba));
            }
            _ => return false,
        }
    }
    true
}

/// Whether the unary program `f : Nat → Nat` is the predecessor: `f 0 = 0` and `f (n+1) = n`.
fn predecessor(context: &Context, f: &Term) -> bool {
    let Some(name) = (match f {
        Term::Global(n) => Some(n.as_str()),
        _ => None,
    }) else {
        return false;
    };
    let Some(ty) = context.get_definition_type(name) else {
        return false;
    };
    if context.get_global(name) != Some(ty)
        || context.poly_declaration(name).is_some()
        || !matches!(ty, Term::Pi {param_type,body_type,..} if **param_type==g("Nat") && **body_type==g("Nat"))
    {
        return false;
    }
    let context = context.extend("arithmetic.certificate.n", g("Nat"));
    let n = Term::Var("arithmetic.certificate.n".into());
    equation(&context, app(f.clone(), g("Zero")), g("Zero"))
        && equation(&context, app(f.clone(), app(g("Succ"), n.clone())), n)
}

impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    pub(super) fn certified_arithmetic(&mut self, head: TermId) -> Option<Arithmetic> {
        self.arithmetic_certificate(head, 0)
    }

    fn arithmetic_certificate(&mut self, head: TermId, depth: usize) -> Option<Arithmetic> {
        if !self.canonical_nat
            || self.preserve_nat_syntax
            || self.ctx.checking_order_recurrence
            || !self.ctx.simp_rules().is_empty()
            || depth >= 8
        {
            return None;
        }
        if let Some(answer) = self.certified_arithmetic.get(&head) {
            return *answer;
        }
        // Signature filtering is not a recurrence probe. In particular, a
        // partially applied head must not starve a later arithmetic program.
        let Some(symbol) = self.arithmetic_signature(head) else {
            self.certified_arithmetic.insert(head, None);
            return None;
        };
        if self.arithmetic_attempts == 0 {
            return None;
        }
        self.arithmetic_attempts -= 1;
        // An in-progress entry cannot certify itself through a dependency cycle.
        self.certified_arithmetic.insert(head, None);
        let answer = self.certify_arithmetic(head, symbol, depth);
        self.certified_arithmetic.insert(head, answer);
        answer
    }

    fn arithmetic_signature(&self, head: TermId) -> Option<Symbol> {
        let symbol = match self.store.node(head) {
            Node::Global(symbol) => *symbol,
            Node::Const { name, levels } if levels.is_empty() => *name,
            _ => return None,
        };
        let name = self.store.name(symbol);
        if self.ctx.poly_declaration(name).is_some()
            || self.ctx.is_constructor(name)
            || self.ctx.is_inductive(name)
        {
            return None;
        }
        let ty = self.ctx.get_definition_type(name)?;
        if self.ctx.get_global(name) != Some(ty) {
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
            if **param_type != g("Nat") {
                return None;
            }
            result = body_type;
        }
        if *result != g("Nat") {
            return None;
        }
        Some(symbol)
    }

    fn certify_arithmetic(
        &mut self,
        head: TermId,
        symbol: Symbol,
        depth: usize,
    ) -> Option<Arithmetic> {
        let name = self.store.name(symbol);
        let op = self.store.extract(head);
        if self.ctx.has_pending_dependency(&op) {
            return None;
        }
        let body = self.ctx.get_definition_body(name)?;
        if !recurrence_body_admitted(body) {
            return None;
        }
        let body = self.store.intern(body);
        if !self.store.free_names_are_empty(body)
            || self.store.shapes(body).contains(NodeShape::Hole)
            || !self.store.eq_faithful(body)
        {
            return None;
        }
        let context = self
            .ctx
            .order_recurrence_context()
            .extend("arithmetic.certificate.x", g("Nat"))
            .extend("arithmetic.certificate.y", g("Nat"));
        let x = Term::Var("arithmetic.certificate.x".into());
        let y = Term::Var("arithmetic.certificate.y".into());
        let zero = g("Zero");
        let one = app(g("Succ"), zero.clone());
        let base = call(&op, x.clone(), zero.clone());
        let step = call(&op, x.clone(), app(g("Succ"), y.clone()));
        let recursive = call(&op, x.clone(), y);
        let base_is_x = equation(&context, base.clone(), x.clone());
        if base_is_x && equation(&context, step.clone(), app(g("Succ"), recursive.clone())) {
            return Some(Arithmetic::Add);
        }
        let mut pending = vec![body];
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if matches!(self.store.node(id), Node::Global(_) | Node::Const { .. }) {
                candidates.push(id);
            }
            self.store
                .node(id)
                .for_each_child(&mut |id| pending.push(id));
        }
        // Truncated subtraction: `f x 0 = x`, `f x (y+1) = pred (f x y)` for a certified
        // predecessor (`pred 0 = 0`, `pred (n+1) = n`). The unique solution is `x ∸ y`.
        if base_is_x {
            for dependency in &candidates {
                let f = self.store.extract(*dependency);
                if predecessor(&self.ctx.order_recurrence_context(), &f)
                    && equation(&context, step.clone(), app(f, recursive.clone()))
                {
                    return Some(Arithmetic::Sub);
                }
            }
            return None;
        }
        let kind = if equation(&context, base.clone(), zero) {
            Arithmetic::Mul
        } else if equation(&context, base, one) {
            Arithmetic::Pow
        } else {
            return None;
        };
        let dependency_kind = if kind == Arithmetic::Mul {
            Arithmetic::Add
        } else {
            Arithmetic::Mul
        };
        for dependency in candidates {
            if self.arithmetic_certificate(dependency, depth + 1) != Some(dependency_kind) {
                continue;
            }
            let f = self.store.extract(dependency);
            // Addition and multiplication commute; both common recurrence
            // orientations have the same unique solution by induction on y.
            if equation(
                &context,
                step.clone(),
                call(&f, recursive.clone(), x.clone()),
            ) || equation(
                &context,
                step.clone(),
                call(&f, x.clone(), recursive.clone()),
            ) {
                return Some(kind);
            }
        }
        None
    }

    pub(super) fn arithmetic_result(
        &mut self,
        op: Arithmetic,
        a: &crate::BigInt,
        b: &crate::BigInt,
    ) -> Option<crate::BigInt> {
        if a.is_negative() || b.is_negative() {
            return None;
        }
        let limbs = |n: &crate::BigInt| n.to_le_bytes().1.len().div_ceil(8) as u64;
        match op {
            Arithmetic::Sub => {
                self.fuel = self.fuel.checked_sub(limbs(a).max(limbs(b)).max(1))?;
                return Some(if a >= b {
                    a.sub(b)
                } else {
                    crate::BigInt::zero()
                });
            }
            // Lean's `n / 0 = 0` and `n % 0 = n`.
            Arithmetic::Div | Arithmetic::Mod => {
                if b.is_zero() {
                    return Some(if op == Arithmetic::Div {
                        crate::BigInt::zero()
                    } else {
                        a.clone()
                    });
                }
                if limbs(a) > 16_384 {
                    return None;
                }
                self.fuel = self
                    .fuel
                    .checked_sub(limbs(a).max(1).checked_mul(limbs(b).max(1))?)?;
                let (quotient, remainder) = a.div_rem(b)?;
                return Some(if op == Arithmetic::Div {
                    quotient
                } else {
                    remainder
                });
            }
            _ => {}
        }
        let one = crate::BigInt::from_u64(1);
        if op == Arithmetic::Pow {
            if b.is_zero() || *a == one {
                return Some(one);
            }
            if a.is_zero() {
                return Some(crate::BigInt::zero());
            }
            let mut exponent = u32::try_from(b.to_i64()?).ok()?;
            let mut result = one;
            let mut base = a.clone();
            while exponent > 0 {
                if exponent & 1 != 0 {
                    result = self.arithmetic_result(Arithmetic::Mul, &result, &base)?;
                }
                exponent >>= 1;
                if exponent > 0 {
                    base = self.arithmetic_result(Arithmetic::Mul, &base, &base)?;
                }
            }
            return Some(result);
        }
        let al = a.to_le_bytes().1.len().div_ceil(8) as u64;
        let bl = b.to_le_bytes().1.len().div_ceil(8) as u64;
        let limbs = if op == Arithmetic::Add {
            al.max(bl).checked_add(1)?
        } else {
            al.checked_add(bl)?
        };
        // Bound both allocation and worst-case schoolbook limb work before the
        // underlying arbitrary-precision operation. Exhaustion is inconclusive.
        if limbs > 16_384 {
            return None;
        }
        let cost = if op == Arithmetic::Add {
            al.max(bl).max(1)
        } else {
            al.max(1).checked_mul(bl.max(1))?
        };
        self.fuel = self.fuel.checked_sub(cost)?;
        Some(if op == Arithmetic::Add {
            a.add(b)
        } else {
            a.mul(b)
        })
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_arithmetic/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_arithmetic/literal_decision_controls.rs"]
mod literal_decision_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_arithmetic/admission_controls.rs"]
mod admission_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_arithmetic/admission_limit_controls.rs"]
mod admission_limit_controls;
