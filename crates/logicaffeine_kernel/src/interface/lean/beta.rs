//! Compositional source certificates for closed-type beta specialization.
//!
//! Both existing kernels check the small rule witnesses and closed leaves.
//! This module composes those certificates; it does not claim that either
//! kernel walked the original (possibly very deep) source tree directly.
use super::*;
use crate::term_store::{TermId, TermStore};

// The source-only certificate candidate is experimental: only the lazy-conversion trace
// controls construct it, and the production import path never does.
#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/beta/source.rs"]
mod source;
#[cfg(test)]
pub(super) use source::SourceCertificates;

type EnvId = usize;
#[derive(Clone, Copy)]
struct Jump {
    end: EnvId,
    // Number of bindings skipped after the current run. Excluding the head
    // makes this table immutable/reusable when that run grows by one binder.
    extra_weight: usize,
}
struct Environment {
    jumps: std::rc::Rc<Vec<Jump>>,
    ty: TermId,
    run_length: usize,
}

pub(super) struct Certificates {
    // Frozen for the entire pass, including its universe parameter identity.
    ctx: Context,
    params: HashSet<String>,
    store: TermStore,
    envs: Vec<Option<Environment>>,
    environment_jump_bytes: usize,
    env_ids: HashMap<(EnvId, TermId), EnvId>,
    active: Vec<Option<EnvId>>,
    source: HashMap<(usize, EnvId), Option<TermId>>,
    apps: HashMap<(TermId, TermId), Option<TermId>>,
    lambdas: HashMap<(TermId, TermId), TermId>,
    types: HashSet<TermId>,
}
// Only the plan-free wrapper opts into these materialization admissions.
// The existing Plan-based certificate entry points keep their original policy.
#[derive(Clone, Copy)]
struct RuleAdmission {
    before_copy: fn(&TermStore, TermId, &mut usize) -> Result<()>,
    before_intern: fn(&Term, &mut usize) -> Result<()>,
}
impl Certificates {
    pub(super) fn new(
        ctx: Context,
        params: &HashSet<String>,
        bound: &[(String, Term, Option<Term>)],
        budget: &mut usize,
    ) -> Result<Self> {
        let mut result = Self {
            ctx,
            params: params.clone(),
            store: TermStore::new(),
            envs: vec![None],
            environment_jump_bytes: 0,
            env_ids: HashMap::new(),
            source: HashMap::new(),
            apps: HashMap::new(),
            lambdas: HashMap::new(),
            types: HashSet::new(),
            active: vec![Some(0)],
        };
        for (_, ty, _) in bound {
            result.enter(ty, budget)?;
        }
        Ok(result)
    }
    fn charge(budget: &mut usize, nodes: usize) -> Result<()> {
        if nodes > *budget {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "checked source certificate materialization budget",
            ));
        }
        *budget -= nodes;
        Ok(())
    }
    fn copy(&self, id: TermId, budget: &mut usize) -> Result<Term> {
        let nodes = usize::try_from(self.store.tree_size(id)).unwrap_or(usize::MAX);
        Self::charge(budget, nodes)?;
        Ok(self.store.extract(id))
    }
    fn copy_admitted(
        &self,
        id: TermId,
        budget: &mut usize,
        admission: Option<RuleAdmission>,
    ) -> Result<Term> {
        if let Some(admission) = admission {
            (admission.before_copy)(&self.store, id, budget)?;
        }
        self.copy(id, budget)
    }
    fn closed_id_admitted(
        &mut self,
        term: &Term,
        budget: &mut usize,
        admission: Option<RuleAdmission>,
    ) -> Result<Option<TermId>> {
        if let Some(admission) = admission {
            (admission.before_intern)(term, budget)?;
        }
        Ok(self.closed_id(term))
    }
    fn closed_id(&mut self, term: &Term) -> Option<TermId> {
        let id = self.store.intern(term);
        (self.store.free_vars(id).is_empty()
            && !self
                .store
                .shapes(id)
                .contains(crate::term_store::NodeShape::Hole))
        .then_some(id)
    }
    fn both(&self, term: &Term) -> Result<Term> {
        let ty = lean_type(&self.ctx, term)?;
        let independent = crate::recheck(&self.ctx, term)
            .map_err(|e| independent_verdict(&e, "independent source-rule witness"))?;
        exact(&self.ctx, &ty, &independent)?;
        Ok(ty)
    }
    fn type_id(&mut self, term: &Term) -> Result<Option<TermId>> {
        let Some(id) = self.closed_id(term) else {
            return Ok(None);
        };
        if !self.types.contains(&id) {
            let ty = self.both(term)?;
            if !matches!(normal(&self.ctx, &ty)?, Term::Sort(_)) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "source lambda domain is not a type",
                ));
            }
            self.types.insert(id);
        }
        Ok(Some(id))
    }
    /// Physical capacity retained by the environment vector and distinct jump
    /// allocations. Shared jump tables are counted once; the existing source,
    /// interning, and TermStore tables are separate owners.
    #[cfg(test)]
    fn environment_storage_bytes(&self) -> Option<usize> {
        self.envs
            .capacity()
            .checked_mul(std::mem::size_of::<Option<Environment>>())?
            .checked_add(self.environment_jump_bytes)
    }

    fn extend(&mut self, parent: EnvId, ty: TermId, budget: &mut usize) -> Result<EnvId> {
        Self::charge(budget, 1)?; // exact interning lookup
        if let Some(id) = self.env_ids.get(&(parent, ty)) {
            return Ok(*id);
        }
        if parent >= self.envs.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "invalid closed-type environment parent",
            ));
        }
        let same_run = self.envs[parent].as_ref().filter(|env| env.ty == ty);
        let (run_length, jumps, new_jump_bytes) = if let Some(previous) = same_run {
            let length = previous.run_length.checked_add(1).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "closed-type environment length overflow",
                )
            })?;
            Self::charge(budget, 1)?; // new environment record; jump allocation is shared
            (length, std::rc::Rc::clone(&previous.jumps), 0)
        } else {
            // A new run can gain at most one binary lifting level. Reserving
            // every slot pays for both constructing its constant-size jump
            // record and retaining it, even when the final slot is unused.
            let capacity = self.envs[parent]
                .as_ref()
                .map_or(Some(1), |env| env.jumps.len().checked_add(1))
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::ResourceLimit,
                        "closed-type jump capacity overflow",
                    )
                })?;
            if capacity > usize::BITS as usize + 1 {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "closed-type jump level overflow",
                ));
            }
            Self::charge(
                budget,
                capacity.checked_add(2).ok_or_else(|| {
                    declined(
                        DiagnosticCode::ResourceLimit,
                        "closed-type jump reservation overflow",
                    )
                })?,
            )?;
            // The two additional records are the environment and Rc<Vec<_>>
            // owner. All fallible structural arithmetic precedes publication.
            let mut table = Vec::with_capacity(capacity);
            if table.capacity() > capacity {
                Self::charge(budget, table.capacity() - capacity)?;
            }
            table.push(Jump {
                end: parent,
                extra_weight: 0,
            });
            let mut level = 0;
            loop {
                let previous = table[level];
                let Some(middle) = self.envs[previous.end].as_ref() else {
                    break;
                };
                let Some(next) = middle.jumps.get(level) else {
                    break;
                };
                let extra_weight = previous
                    .extra_weight
                    .checked_add(middle.run_length)
                    .and_then(|weight| weight.checked_add(next.extra_weight))
                    .ok_or_else(|| {
                        declined(
                            DiagnosticCode::ResourceLimit,
                            "closed-type jump weight overflow",
                        )
                    })?;
                if table.len() == capacity {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "closed-type jump exceeded reserved capacity",
                    ));
                }
                table.push(Jump {
                    end: next.end,
                    extra_weight,
                });
                level += 1;
            }
            let bytes = table
                .capacity()
                .checked_mul(std::mem::size_of::<Jump>())
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<Jump>>()))
                .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::ResourceLimit,
                        "closed-type jump byte accounting overflow",
                    )
                })?;
            (1, std::rc::Rc::new(table), bytes)
        };
        let retained = self
            .environment_jump_bytes
            .checked_add(new_jump_bytes)
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "closed-type retained byte accounting overflow",
                )
            })?;
        let id = self.envs.len();
        self.envs.push(Some(Environment {
            jumps,
            ty,
            run_length,
        }));
        self.environment_jump_bytes = retained;
        self.env_ids.insert((parent, ty), id);
        Ok(id)
    }

    fn lookup(
        &self,
        mut env: EnvId,
        mut index: usize,
        budget: &mut usize,
    ) -> Result<Option<TermId>> {
        Self::charge(budget, 1)?;
        let Some(initial) = self.envs.get(env).and_then(Option::as_ref) else {
            return Ok(None);
        };
        if index < initial.run_length {
            return Ok(Some(initial.ty));
        }
        // Greedy binary lifting visits descending levels once. Distances are
        // measured in bindings, whereas each level skips a power-of-two count
        // of runs. Restarting the level search would lose the logarithmic bound.
        for level in (0..initial.jumps.len()).rev() {
            Self::charge(budget, 1)?;
            let Some(current) = self.envs.get(env).and_then(Option::as_ref) else {
                return Ok(None);
            };
            let Some(jump) = current.jumps.get(level) else {
                continue;
            };
            let distance = current
                .run_length
                .checked_add(jump.extra_weight)
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::ResourceLimit,
                        "closed-type lookup distance overflow",
                    )
                })?;
            if index >= distance {
                index -= distance;
                env = jump.end;
                let Some(next) = self.envs.get(env).and_then(Option::as_ref) else {
                    return Ok(None);
                };
                if index < next.run_length {
                    return Ok(Some(next.ty));
                }
            }
        }
        // Every nonempty environment has a level-zero jump, so reaching this
        // point can only mean there was no binding at the requested index.
        Ok(None)
    }
    pub(super) fn enter(&mut self, ty: &Term, budget: &mut usize) -> Result<()> {
        Self::charge(budget, 1)?;
        let result = if let Some(parent) = self.active.last().copied().flatten() {
            // A newly synthesized helper absent from this immutable snapshot
            // cannot become an unchecked assumption in its certificate pass.
            match self.type_id(ty) {
                Ok(Some(ty)) => Some(self.extend(parent, ty, budget)?),
                _ => None,
            }
        } else {
            None
        };
        self.active.push(result);
        Ok(())
    }
    pub(super) fn leave(&mut self) {
        assert!(self.active.len() > 1, "source certificate lexical scope");
        self.active.pop();
    }
    fn app_rule(&mut self, ft: TermId, at: TermId, budget: &mut usize) -> Result<Option<TermId>> {
        self.app_rule_admitted(ft, at, budget, None)
    }
    fn app_rule_admitted(
        &mut self,
        ft: TermId,
        at: TermId,
        budget: &mut usize,
        admission: Option<RuleAdmission>,
    ) -> Result<Option<TermId>> {
        if let Some(ty) = self.apps.get(&(ft, at)) {
            return Ok(*ty);
        }
        let ft_term = self.copy_admitted(ft, budget, admission)?;
        let normalized;
        let exposed = if matches!(&ft_term, Term::Pi { .. }) {
            &ft_term
        } else {
            normalized = normal(&self.ctx, &ft_term)?;
            &normalized
        };
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = exposed
        else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "source application is not a function",
            ));
        };
        if self
            .closed_id_admitted(param_type, budget, admission)?
            .is_none()
        {
            self.apps.insert((ft, at), None);
            return Ok(None);
        }
        // A dependent result needs its original argument value; the shallow
        // signature witness cannot certify that frontier.
        let Some(result) = self.closed_id_admitted(&body_type, budget, admission)? else {
            self.apps.insert((ft, at), None);
            return Ok(None);
        };
        let at_term = self.copy_admitted(at, budget, admission)?;
        Self::charge(budget, 5)?;
        let witness = Term::Lambda {
            param: "arena.rule.function".into(),
            param_type: Box::new(ft_term),
            body: Box::new(Term::Lambda {
                param: "arena.rule.argument".into(),
                param_type: Box::new(at_term),
                body: Box::new(apply(
                    Term::Var("arena.rule.function".into()),
                    Term::Var("arena.rule.argument".into()),
                )),
            }),
        };
        let inferred = self.both(&witness)?;
        let Term::Pi { body_type, .. } = inferred else {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "source application witness shape",
            ));
        };
        let Term::Pi { body_type, .. } = *body_type else {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "source application witness arguments",
            ));
        };
        let expected = self.copy_admitted(result, budget, admission)?;
        exact(&self.ctx, &body_type, &expected)?;
        self.apps.insert((ft, at), Some(result));
        Ok(Some(result))
    }
    fn lambda_rule(&mut self, dom: TermId, body: TermId, budget: &mut usize) -> Result<TermId> {
        self.lambda_rule_admitted(dom, body, budget, None)
    }
    fn lambda_rule_admitted(
        &mut self,
        dom: TermId,
        body: TermId,
        budget: &mut usize,
        admission: Option<RuleAdmission>,
    ) -> Result<TermId> {
        if let Some(ty) = self.lambdas.get(&(dom, body)) {
            return Ok(*ty);
        }
        let d = self.copy_admitted(dom, budget, admission)?;
        let b = self.copy_admitted(body, budget, admission)?;
        Self::charge(budget, 3)?;
        // The synthesized binder is named by the depth of the body type it scopes
        // over. A binder this rule synthesized anywhere inside that body type scopes
        // over a proper subterm, so its index is strictly smaller: opening a composed
        // telescope never shadows an enclosing synthesized binder, and the kernel
        // never renames (and re-traverses) the body once per level.
        let bound = format!("arena.rule.bound.{}", self.store.depth(body));
        let witness = Term::Lambda {
            param: "arena.rule.body".into(),
            param_type: Box::new(b),
            body: Box::new(Term::Lambda {
                param: bound,
                param_type: Box::new(d),
                body: Box::new(Term::Var("arena.rule.body".into())),
            }),
        };
        let ty = self.both(&witness)?;
        let Term::Pi { body_type, .. } = ty else {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "source lambda witness shape",
            ));
        };
        let Some(result) = self.closed_id_admitted(&body_type, budget, admission)? else {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "source lambda witness not closed",
            ));
        };
        self.lambdas.insert((dom, body), result);
        Ok(result)
    }
    fn closed_term(
        &self,
        importer: &Importer,
        plan: &dag::Plan,
        id: usize,
        budget: &mut usize,
    ) -> Result<Option<Term>> {
        if !plan.closed(id) || plan.depth(id) >= dag::CLOSED_CHUNK_DEPTH || plan.size(id) > 256 {
            return Ok(None);
        }
        Ok(Some(importer.term(
            &serde_json::json!(id),
            &mut Vec::new(),
            &self.params,
            budget,
        )?))
    }
    /// Return true only for a generically checked original source subgraph.
    pub(super) fn certify(
        &mut self,
        importer: &Importer,
        plan: &dag::Plan,
        root: usize,
        budget: &mut usize,
    ) -> Result<bool> {
        let Some(root_env) = self.active.last().copied().flatten() else {
            return Ok(false);
        };
        enum Frame {
            Visit(usize, EnvId),
            App(usize, EnvId),
            Lambda(usize, EnvId, TermId),
            Alias(usize, EnvId),
        }
        let mut work = vec![Frame::Visit(root, root_env)];
        let mut values = Vec::<Option<TermId>>::new();
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Visit(id, env) => {
                    Self::charge(budget, 1)?;
                    let key_env = if plan.closed(id) { 0 } else { env };
                    if let Some(ty) = self.source.get(&(id, key_env)) {
                        values.push(*ty);
                        continue;
                    }
                    if let Some(term) = self.closed_term(importer, plan, id, budget)? {
                        let ty = self.both(&term)?;
                        let ty = self.closed_id(&ty);
                        self.source.insert((id, key_env), ty);
                        values.push(ty);
                        continue;
                    }
                    let row = importer.exprs.get(&id).ok_or_else(|| {
                        declined(
                            DiagnosticCode::MalformedExport,
                            "unknown certificate source",
                        )
                    })?;
                    if let Some(bvar) = row.get("bvar") {
                        let ty = self.lookup(env, index(bvar)?, budget)?.ok_or_else(|| {
                            rejected(
                                DiagnosticCode::MalformedExport,
                                "free source certificate variable",
                            )
                        })?;
                        self.source.insert((id, key_env), Some(ty));
                        values.push(Some(ty));
                    } else if let Some(app) = row.get("app") {
                        work.push(Frame::App(id, key_env));
                        work.push(Frame::Visit(index(&app["arg"])?, env));
                        work.push(Frame::Visit(index(&app["fn"])?, env));
                    } else if let Some(lam) = row.get("lam") {
                        let Some(domain) =
                            self.closed_term(importer, plan, index(&lam["type"])?, budget)?
                        else {
                            self.source.insert((id, key_env), None);
                            values.push(None);
                            continue;
                        };
                        let Some(domain) = self.type_id(&domain)? else {
                            self.source.insert((id, key_env), None);
                            values.push(None);
                            continue;
                        };
                        let child_env = self.extend(env, domain, budget)?;
                        work.push(Frame::Lambda(id, key_env, domain));
                        work.push(Frame::Visit(index(&lam["body"])?, child_env));
                    } else if let Some(meta) = row.get("mdata") {
                        work.push(Frame::Alias(id, key_env));
                        work.push(Frame::Visit(index(&meta["expr"])?, env));
                    } else {
                        self.source.insert((id, key_env), None);
                        values.push(None);
                    }
                }
                Frame::App(id, env) => {
                    let arg = values.pop().expect("source argument certificate");
                    let fun = values.pop().expect("source function certificate");
                    let result = match (fun, arg) {
                        (Some(f), Some(a)) => self.app_rule(f, a, budget)?,
                        _ => None,
                    };
                    self.source.insert((id, env), result);
                    values.push(result);
                }
                Frame::Lambda(id, env, domain) => {
                    let result = values
                        .pop()
                        .expect("source body certificate")
                        .map(|body| self.lambda_rule(domain, body, budget))
                        .transpose()?;
                    self.source.insert((id, env), result);
                    values.push(result);
                }
                Frame::Alias(id, env) => {
                    let result = *values.last().expect("source metadata certificate");
                    self.source.insert((id, env), result);
                }
            }
        }
        debug_assert_eq!(values.len(), 1);
        Ok(values.pop().flatten().is_some())
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/beta/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/beta/environment_controls.rs"]
mod environment_controls;
