//! In-bounds sequence indexing licensed by universally quantified computation laws.
use super::*;

pub(super) const SPINE_LIMIT: usize = 4096;
pub(super) struct ObservedSpine {
    pub fields: Vec<Lazy>,
    pub frontier: Lazy,
}
pub(super) type Spine = Rc<RefCell<ObservedSpine>>;
pub(super) struct Lookup {
    pub head: TermId,
    pub cons: TermId,
    pub args: [Lazy; 4],
    pub spine: Option<Spine>,
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn var(s: &str) -> Term {
    Term::Var(s.into())
}

impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    pub(super) fn certified_index(&mut self, head: TermId) -> Option<TermId> {
        if !self.canonical_nat
            || self.preserve_nat_syntax
            || self.ctx.checking_order_recurrence
            || !self.ctx.simp_rules().is_empty()
        {
            return None;
        }
        if let Some(result) = self.certified_indices.get(&head) {
            return *result;
        }
        // Reject unrelated function shapes before spending a certification attempt.
        let symbol = match self.store.node(head) {
            Node::Global(name) | Node::Const { name, .. } => *name,
            _ => return None,
        };
        let name = self.store.name(symbol);
        let ty = self
            .ctx
            .get_universe_poly(name)
            .map(|(_, ty, _)| ty)
            .or_else(|| self.ctx.get_definition_type(name));
        let candidate = ty.is_some_and(|ty| {
            let Term::Pi {
                param_type,
                body_type,
                ..
            } = ty
            else {
                return false;
            };
            if !matches!(&**param_type, Term::Sort(_)) {
                return false;
            }
            let Term::Pi { body_type, .. } = &**body_type else {
                return false;
            };
            let Term::Pi {
                param_type,
                body_type,
                ..
            } = &**body_type
            else {
                return false;
            };
            if !matches!(&**param_type, Term::App(..)) {
                return false;
            }
            let Term::Pi {
                param_type,
                body_type,
                ..
            } = &**body_type
            else {
                return false;
            };
            matches!(&**param_type,Term::Global(n) if n=="Nat")
                && matches!(&**body_type, Term::Var(_))
        });
        self.certified_indices.insert(head, None);
        if !candidate || self.index_attempts == 0 {
            return None;
        }
        self.index_attempts -= 1;
        let answer = self.certify_index(head);
        self.certified_indices.insert(head, answer);
        answer
    }

    fn certify_index(&mut self, head: TermId) -> Option<TermId> {
        let op = self.store.extract(head);
        if self.ctx.has_pending_dependency(&op) || !recurrence_body_admitted(&op) {
            return None;
        }
        let body = match &op {
            Term::Global(name) if self.ctx.poly_declaration(name).is_none() => {
                self.ctx.get_definition_body(name)?.clone()
            }
            Term::Const { name, levels } => match const_step(self.ctx, name, levels) {
                ConstStep::Delta(subst, body) => crate::term::instantiate_universes(&body, &subst),
                _ => return None,
            },
            _ => return None,
        };
        if !recurrence_body_admitted(&body) {
            return None;
        }
        let body = self.store.intern(&body);
        if !self.store.free_names_are_empty(body)
            || self.store.shapes(body).contains(NodeShape::Hole)
            || !self.store.eq_faithful(body)
        {
            return None;
        }
        let mut context = self.ctx.order_recurrence_context();
        let mut ty = crate::infer_type(&context, &op).ok()?;
        let mut parameters = Vec::new();
        let mut domains = Vec::new();
        for name in [
            "index.certificate.A",
            "index.certificate.default",
            "index.certificate.items",
            "index.certificate.n",
        ] {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = ty
            else {
                return None;
            };
            if !recurrence_body_admitted(&param_type) {
                return None;
            }
            let value = var(name);
            ty = crate::substitute(&body_type, &param, &value);
            domains.push((*param_type).clone());
            context = context.extend(name, *param_type);
            parameters.push(value);
        }
        let a = parameters[0].clone();
        if !matches!(domains[0], Term::Sort(_))
            || ty != a
            || domains[3] != Term::Global("Nat".into())
        {
            return None;
        }
        let Term::App(carrier, element) = &domains[2] else {
            return None;
        };
        if **element != a {
            return None;
        }
        let (name, levels) = match &**carrier {
            Term::Global(name) => (name, None),
            Term::Const { name, levels } => (name, Some(levels)),
            _ => return None,
        };
        if !self.ctx.is_inductive(name)
            || self.ctx.inductive_num_params(name) != 1
            || self.ctx.constructor_inventory_upper_bound(name) != 2
        {
            return None;
        }
        let constructors = self
            .ctx
            .get_constructors(name)
            .iter()
            .map(|(n, _)| (*n).to_string())
            .collect::<Vec<_>>();
        let x = var("index.certificate.element");
        context = context.extend("index.certificate.element", a.clone());
        let xs = parameters[2].clone();
        let n = parameters[3].clone();
        let prefix = app(app(op, a.clone()), parameters[1].clone());
        for name in constructors {
            let ctor = match levels {
                Some(levels) => Term::Const {
                    name,
                    levels: levels.clone(),
                },
                None => Term::Global(name),
            };
            let ctor_id = self.store.intern(&ctor);
            let Some(info) = self.constructor(ctor_id) else {
                continue;
            };
            if info.arity != 3 || info.parameters != 1 {
                continue;
            }
            let cell = app(app(app(ctor, a.clone()), x.clone()), xs.clone());
            let Ok(cell_ty) = crate::infer_type(&context, &cell) else {
                continue;
            };
            if !crate::type_checker::def_eq(&context, &cell_ty, &domains[2]) {
                continue;
            }
            let zero = app(
                app(prefix.clone(), cell.clone()),
                Term::Global("Zero".into()),
            );
            let step = app(
                app(prefix.clone(), cell),
                app(Term::Global("Succ".into()), n.clone()),
            );
            let tail = app(app(prefix.clone(), xs.clone()), n.clone());
            // These are open equations, not a finite test set. Induction on the
            // index licenses each in-bounds step. No nil equation is assumed.
            if crate::type_checker::def_eq(&context, &zero, &x)
                && crate::type_checker::def_eq(&context, &step, &tail)
            {
                return Some(ctor_id);
            }
        }
        None
    }

    pub(super) fn begin_index(&mut self, term: TermId, env: &Env) -> Option<Lookup> {
        let mut head = term;
        let mut arguments = Vec::with_capacity(4);
        for _ in 0..4 {
            let Node::App(f, a) = self.store.node(head) else {
                return None;
            };
            arguments.push(*a);
            head = *f;
        }
        let cons = self.certified_index(head)?;
        arguments.reverse();
        let args = arguments
            .into_iter()
            .map(|a| self.delay(a, env))
            .collect::<Option<Vec<_>>>()?;
        Some(Lookup {
            head,
            cons,
            args: args.try_into().ok()?,
            spine: None,
        })
    }

    pub(super) fn index_start(
        &mut self,
        mut lookup: Lookup,
        index: crate::BigInt,
        stack: &mut Vec<Frame>,
    ) -> Control {
        let key = (lookup.cons, lookup.args[2].id);
        let spine = if let Some(spine) = self.index_spines.get(&key) {
            spine
        } else {
            let spine = Rc::new(RefCell::new(ObservedSpine {
                fields: Vec::new(),
                frontier: lookup.args[2].clone(),
            }));
            self.index_spines.insert(key, spine.clone());
            spine
        };
        let observed = spine.borrow();
        if let Some(index) = index.to_i64().and_then(|n| usize::try_from(n).ok()) {
            if let Some(field) = observed.fields.get(index) {
                return Control::Force(field.clone());
            }
        }
        let remaining = index.sub(&crate::BigInt::from_u64(observed.fields.len() as u64));
        let frontier = observed.frontier.clone();
        drop(observed);
        lookup.spine = Some(spine);
        stack.push(Frame::IndexCell {
            lookup,
            index: remaining,
        });
        Control::Force(frontier)
    }

    pub(super) fn index_fallback(&mut self, lookup: Lookup, stack: &mut Vec<Frame>) -> Control {
        // Reapply the original function to the original lazy arguments. Enter
        // through its head so the failed spine probe cannot re-trigger itself.
        for arg in lookup.args.into_iter().rev() {
            stack.push(Frame::Apply(Argument::Ready(arg)));
        }
        Control::Eval(lookup.head, None)
    }
}
