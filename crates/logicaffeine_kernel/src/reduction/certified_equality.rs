//! Natural equality certified by four universally quantified equations.
use super::*;
impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    pub(super) fn certified_equality(&mut self, head: TermId) -> Option<[TermId; 2]> {
        if !self.canonical_nat
            || self.ctx.checking_order_recurrence
            || !self.ctx.simp_rules().is_empty()
        {
            return None;
        }
        if let Some(answer) = self.certified_equalities.get(&head) {
            return *answer;
        }
        // Only a well-shaped binary natural observer spends a proof attempt.
        let Some((symbol, outcomes)) = self.equality_signature(head) else {
            self.certified_equalities.insert(head, None);
            return None;
        };
        if self.equality_attempts == 0 {
            return None;
        }
        self.equality_attempts -= 1;
        self.certified_equalities.insert(head, None);
        let answer = self.certify_equality(head, symbol, outcomes);
        self.certified_equalities.insert(head, answer);
        answer
    }
    fn equality_signature(&self, head: TermId) -> Option<(Symbol, Vec<Term>)> {
        let Node::Global(symbol) = self.store.node(head) else {
            return None;
        };
        let name = self.store.name(*symbol);
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
            if !matches!(&**param_type,Term::Global(n) if n=="Nat") {
                return None;
            }
            result = body_type;
        }
        let Term::Global(carrier) = result else {
            return None;
        };
        if !self.ctx.is_inductive(carrier)
            || self.ctx.get_global(carrier) != Some(&Term::Sort(crate::Universe::Type(0)))
            || self.ctx.get_definition_body(carrier).is_some()
            || self.ctx.poly_declaration(carrier).is_some()
            || self.ctx.inductive_num_params(carrier) != 0
            || self.ctx.constructor_inventory_upper_bound(carrier) != 2
        {
            return None;
        }
        let ctors = self.ctx.get_constructors(carrier);
        if ctors.len() != 2
            || ctors.iter().any(|(n, t)| {
                *t != result
                    || self.ctx.get_global(n) != Some(result)
                    || self.ctx.constructor_inductive(n) != Some(carrier.as_str())
                    || self.ctx.get_definition_body(n).is_some()
                    || self.ctx.poly_declaration(n).is_some()
            })
        {
            return None;
        }
        let outcomes = ctors
            .iter()
            .map(|(n, _)| Term::Global((*n).into()))
            .collect::<Vec<_>>();
        Some((*symbol, outcomes))
    }

    fn certify_equality(
        &mut self,
        head: TermId,
        symbol: Symbol,
        outcomes: Vec<Term>,
    ) -> Option<[TermId; 2]> {
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
        let g = |s: &str| Term::Global(s.into());
        let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
        let call = |a: Term, b: Term| app(app(op.clone(), a), b);
        let succ = |a: Term| app(g("Succ"), a);
        let ctx = self
            .ctx
            .order_recurrence_context()
            .extend("equality.certificate.m", g("Nat"))
            .extend("equality.certificate.n", g("Nat"));
        let m = Term::Var("equality.certificate.m".into());
        let n = Term::Var("equality.certificate.n".into());
        let eq = |a, b| certified_arithmetic::equation(&ctx, a, b);
        let yes = outcomes
            .iter()
            .position(|out| eq(call(g("Zero"), g("Zero")), out.clone()))?;
        let no = 1 - yes;
        if !eq(call(g("Zero"), succ(n.clone())), outcomes[no].clone())
            || !eq(call(succ(m.clone()), g("Zero")), outcomes[no].clone())
            || !eq(call(succ(m.clone()), succ(n.clone())), call(m, n))
        {
            return None;
        }
        Some([
            self.store.intern(&outcomes[no]),
            self.store.intern(&outcomes[yes]),
        ])
    }
}
