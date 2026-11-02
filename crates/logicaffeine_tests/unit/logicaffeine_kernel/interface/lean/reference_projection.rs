//! Typed projection lowering over an admitted DAG. Source syntax remains independently decoded.
//! Construction/substitution stays in TermStore; only pre-admitted trees cross kernel boundaries.
//! Dependent earlier-field projections use heap continuations, never recursive native calls.
use super::reference::{ReferenceBinding, ReferenceProjection};
use super::{declined, rejected, DiagnosticCode, Result};
use crate::term_store::{Node, TermId, TermStore};
use crate::{Context, Term, Universe};
use logicaffeine_base::Symbol;
use std::collections::{HashMap, HashSet};

pub(super) struct ProjectionReader<'a> {
    pub context: &'a Context,
    pub max_depth: usize,
}
fn resource(message: &str) -> super::Verdict {
    declined(DiagnosticCode::ResourceLimit, message)
}
fn invalid(message: &str) -> super::Verdict {
    rejected(DiagnosticCode::InvalidProof, message)
}

struct Dag<'a> {
    store: TermStore,
    allocation: &'a mut usize,
    work: u64,
    substitution_allocation: u64,
    node_limit: usize,
    depth_limit: usize,
    fresh: u64,
}

impl Dag<'_> {
    fn spend(&mut self, amount: u64) -> Result<()> {
        self.work = self
            .work
            .checked_sub(amount)
            .ok_or_else(|| resource("reference projection DAG work limit"))?;
        Ok(())
    }
    fn admit(&self, root: TermId) -> Result<()> {
        if self.store.depth(root) as usize > self.depth_limit {
            return Err(resource("reference projection generated depth limit"));
        }
        if self.store.tree_size(root) > *self.allocation as u64 {
            return Err(resource("reference projection generated tree limit"));
        }
        Ok(())
    }
    /// Iteratively inspect an existing tree before intern can copy it. Universe nodes count
    /// toward work/admission too. Literal/name payload bytes remain under source/package byte limits.
    fn intern(&mut self, term: &Term) -> Result<TermId> {
        enum Item<'a> {
            Term(&'a Term, usize),
            Level(&'a Universe, usize),
        }
        let mut pending = vec![Item::Term(term, 1)];
        let mut count = 0usize;
        while let Some(item) = pending.pop() {
            self.spend(1)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| resource("reference projection input size overflow"))?;
            if count > self.node_limit {
                return Err(resource("reference projection input node limit"));
            }
            match item {
                Item::Level(level, depth) => {
                    if depth > self.depth_limit {
                        return Err(resource("reference projection universe depth limit"));
                    }
                    match level {
                        Universe::Succ(a) => pending.push(Item::Level(a, depth + 1)),
                        Universe::Max(a, b) | Universe::IMax(a, b) => {
                            pending.push(Item::Level(a, depth + 1));
                            pending.push(Item::Level(b, depth + 1));
                        }
                        Universe::Prop | Universe::SProp | Universe::Type(_) | Universe::Var(_) => {
                        }
                    }
                }
                Item::Term(value, depth) => {
                    if depth > self.depth_limit {
                        return Err(resource("reference projection input depth limit"));
                    }
                    match value {
                        Term::Sort(level) => pending.push(Item::Level(level, 1)),
                        Term::Const { levels, .. } => {
                            if levels.len() > self.node_limit.saturating_sub(count) {
                                return Err(resource(
                                    "reference projection universe inventory limit",
                                ));
                            }
                            pending.extend(levels.iter().map(|level| Item::Level(level, 1)));
                        }
                        Term::Pi {
                            param_type,
                            body_type,
                            ..
                        } => {
                            pending.push(Item::Term(param_type, depth + 1));
                            pending.push(Item::Term(body_type, depth + 1));
                        }
                        Term::Lambda {
                            param_type, body, ..
                        } => {
                            pending.push(Item::Term(param_type, depth + 1));
                            pending.push(Item::Term(body, depth + 1));
                        }
                        Term::App(a, b) => {
                            pending.push(Item::Term(a, depth + 1));
                            pending.push(Item::Term(b, depth + 1));
                        }
                        Term::Match {
                            discriminant,
                            motive,
                            cases,
                        } => {
                            if cases.len() > self.node_limit.saturating_sub(count) {
                                return Err(resource("reference projection case inventory limit"));
                            }
                            pending.push(Item::Term(discriminant, depth + 1));
                            pending.push(Item::Term(motive, depth + 1));
                            pending.extend(cases.iter().map(|case| Item::Term(case, depth + 1)));
                        }
                        Term::Fix { body, .. } => pending.push(Item::Term(body, depth + 1)),
                        Term::MutualFix { defs, .. } => {
                            if defs.len() > self.node_limit.saturating_sub(count) {
                                return Err(resource(
                                    "reference projection mutual inventory limit",
                                ));
                            }
                            pending
                                .extend(defs.iter().map(|(_, body)| Item::Term(body, depth + 1)));
                        }
                        Term::Let {
                            ty, value, body, ..
                        } => {
                            pending.push(Item::Term(ty, depth + 1));
                            pending.push(Item::Term(value, depth + 1));
                            pending.push(Item::Term(body, depth + 1));
                        }
                        Term::Annotated { term, .. } => pending.push(Item::Term(term, depth + 1)),
                        Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
                    }
                }
            }
            if pending.len() > self.node_limit.saturating_sub(count) {
                return Err(resource("reference projection traversal inventory limit"));
            }
        }
        // At most one new store node per input term node. Reserve that upper bound first.
        if self
            .store
            .len()
            .checked_add(count)
            .is_none_or(|size| size > self.node_limit)
        {
            return Err(resource("reference projection distinct DAG node limit"));
        }
        self.spend((count as u64).saturating_mul(count as u64))?;
        let root = self.store.intern(term);
        self.admit(root)?;
        Ok(root)
    }
    fn add(&mut self, node: Node) -> Result<TermId> {
        if self.store.len() >= self.node_limit {
            return Err(resource("reference projection distinct DAG node limit"));
        }
        let mut depth = 1usize;
        let mut tree = 1u64;
        node.for_each_child(&mut |child| {
            depth = depth.max(self.store.depth(child) as usize + 1);
            tree = tree.saturating_add(self.store.tree_size(child));
        });
        if depth > self.depth_limit || tree > *self.allocation as u64 {
            return Err(resource(
                "reference projection construction exceeds admitted output",
            ));
        }
        // compute_info combines child name sets; a tree-size square is a conservative charge.
        self.spend(tree.saturating_mul(tree))?;
        Ok(self.store.add(node))
    }
    fn var(&mut self, name: &str) -> Result<TermId> {
        let name = self.store.symbol(name);
        self.add(Node::Var(name))
    }
    fn app(&mut self, f: TermId, a: TermId) -> Result<TermId> {
        self.add(Node::App(f, a))
    }
    fn lambda(&mut self, name: &str, ty: TermId, body: TermId) -> Result<TermId> {
        let param = self.store.symbol(name);
        self.add(Node::Lambda {
            param,
            param_type: ty,
            body,
        })
    }
    fn constant_like(&mut self, name: &str, model: TermId) -> Result<TermId> {
        let symbol = self.store.symbol(name);
        let node = match self.store.node(model) {
            Node::Const { levels, .. } => Node::Const {
                name: symbol,
                levels: levels.clone(),
            },
            Node::Global(_) => Node::Global(symbol),
            _ => return Err(invalid("reference family has no constant head")),
        };
        self.add(node)
    }
    fn instantiate(&mut self, root: TermId, pairs: &[(Symbol, TermId)]) -> Result<TermId> {
        let result = self
            .store
            .instantiate_bounded(
                root,
                pairs,
                &mut self.work,
                self.node_limit,
                &mut self.substitution_allocation,
            )
            .map_err(|_| resource("reference projection bounded substitution limit"))?;
        self.admit(result)?;
        Ok(result)
    }
    fn substitute(&mut self, root: TermId, name: &str, value: TermId) -> Result<TermId> {
        let symbol = self.store.symbol(name);
        self.instantiate(root, &[(symbol, value)])
    }
    fn extract(&mut self, root: TermId) -> Result<Term> {
        self.admit(root)?;
        let count = usize::try_from(self.store.tree_size(root))
            .map_err(|_| resource("reference projection tree size overflow"))?;
        *self.allocation = self
            .allocation
            .checked_sub(count)
            .ok_or_else(|| resource("reference projection tree allocation limit"))?;
        self.spend(count as u64)?;
        self.store
            .extract_bounded(root, count as u64)
            .map_err(|_| resource("reference projection extraction limit"))
    }
    fn infer(&mut self, ctx: &Context, root: TermId) -> Result<TermId> {
        let term = self.extract(root)?;
        let ty = crate::infer_type(ctx, &term).map_err(super::kernel)?;
        self.intern(&ty)
    }
    fn sort(&mut self, ctx: &Context, root: TermId) -> Result<Universe> {
        let term = self.extract(root)?;
        crate::type_checker::infer_sort(ctx, &term).map_err(super::kernel)
    }
    fn extend(&mut self, ctx: &Context, name: &str, ty: TermId) -> Result<Context> {
        Ok(ctx.extend(name, self.extract(ty)?))
    }
    fn fresh(&mut self, ctx: &Context, kind: &str) -> Result<String> {
        loop {
            self.spend(1)?;
            self.fresh = self
                .fresh
                .checked_add(1)
                .ok_or_else(|| resource("reference projection fresh-name limit"))?;
            let name = format!("reference.projection.{kind}.{}", self.fresh);
            if self.store.lookup_symbol(&name).is_none() && !ctx.local_name_in_use(&name) {
                return Ok(name);
            }
        }
    }
    fn head(&mut self, ctx: &Context, root: TermId, function: bool) -> Result<TermId> {
        if function && matches!(self.store.node(root), Node::Pi { .. } | Node::Sort(_)) {
            return Ok(root);
        }
        if ctx.simp_rules().is_empty() {
            let mut head = root;
            while let Node::App(f, _) = self.store.node(head) {
                let next = *f;
                self.spend(1)?;
                head = next;
            }
            if let Node::Global(name) | Node::Const { name, .. } = self.store.node(head) {
                if ctx.is_inductive(self.store.name(*name)) {
                    return Ok(root);
                }
            }
        }
        let term = self.extract(root)?;
        let exposed = if function {
            crate::type_checker::exposed_function_type(ctx, term)
        } else {
            crate::type_checker::exposed_inductive_type(ctx, term)
        };
        let exposed =
            match exposed {
                Ok(value) => value,
                Err(value) => match if function {
                    crate::type_checker::exposed_inductive_type(ctx, value)
                } else {
                    Err(value)
                } {
                    Ok(family) => family,
                    Err(value) => crate::reduction::normalize_checked(ctx, &value).map_err(
                        |(_, reason)| match reason {
                            crate::Unreduced::Cyclic => declined(
                                DiagnosticCode::UnsupportedFeature,
                                "reference projection type reduction is cyclic",
                            ),
                            crate::Unreduced::BudgetSpent | crate::Unreduced::TooDeep => {
                                resource("reference projection type reduction did not finish")
                            }
                        },
                    )?,
                },
            };
        self.intern(&exposed)
    }
    fn peel(&mut self, ctx: &Context, root: TermId, name: &str) -> Result<(TermId, TermId)> {
        let root = self.head(ctx, root, true)?;
        let (old, domain, body) = match self.store.node(root) {
            Node::Pi {
                param,
                param_type,
                body_type,
            } => (*param, *param_type, *body_type),
            _ => {
                return Err(invalid(
                    "reference projection expected a constructor telescope",
                ))
            }
        };
        let variable = self.var(name)?;
        Ok((domain, self.instantiate(body, &[(old, variable)])?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    scope: u64,
    parent: String,
    index: usize,
    value: TermId,
}
struct Request {
    key: Key,
    context: Context,
}
struct Frame {
    key: Key,
    context: Context,
    motive_context: Context,
    motive_scope: u64,
    telescope: Vec<(String, TermId)>,
    indices: Vec<(String, TermId)>,
    generic: TermId,
    major_name: String,
    major: TermId,
    result_type: TermId,
    next: usize,
    potentially_prop: bool,
}
enum Task {
    Enter(Request),
    Advance(Frame),
    Attach { frame: Frame, name: String },
}

fn prepare(dag: &mut Dag<'_>, request: Request, scope: &mut u64) -> Result<Frame> {
    let Request { key, context } = request;
    let inferred = dag.infer(&context, key.value)?;
    let ty = dag.head(&context, inferred, false)?;
    let mut arguments = Vec::new();
    let mut head = ty;
    while let Node::App(fun, arg) = dag.store.node(head) {
        let (fun, arg) = (*fun, *arg);
        dag.spend(1)?;
        arguments.push(arg);
        head = fun;
    }
    arguments.reverse();
    let parent = match dag.store.node(head) {
        Node::Global(name) | Node::Const { name, .. } => dag.store.name(*name),
        _ => return Err(invalid("reference projection value has no family head")),
    };
    if parent != key.parent || !context.is_inductive(&key.parent) {
        return Err(invalid("reference projection parent differs from source"));
    }
    let ctors = context.get_constructors(&key.parent);
    if ctors.len() != 1 {
        return Err(invalid("reference projection requires one constructor"));
    }
    let parameters = context.inductive_num_params(&key.parent);
    if arguments.len() < parameters {
        return Err(invalid("reference projection lacks parameters"));
    }
    let ctor = dag.constant_like(ctors[0].0, head)?;
    let mut fields = dag.infer(&context, ctor)?;
    let mut arity = dag.infer(&context, head)?;
    for argument in &arguments[..parameters] {
        let name = dag.fresh(&context, "parameter")?;
        let (_, rest) = dag.peel(&context, arity, &name)?;
        arity = dag.substitute(rest, &name, *argument)?;
        let (_, rest) = dag.peel(&context, fields, &name)?;
        fields = dag.substitute(rest, &name, *argument)?;
    }
    let mut motive_context = context.clone();
    let mut indices = Vec::new();
    for _ in parameters..arguments.len() {
        let name = dag.fresh(&motive_context, "index")?;
        let (domain, rest) = dag.peel(&motive_context, arity, &name)?;
        motive_context = dag.extend(&motive_context, &name, domain)?;
        indices.push((name, domain));
        arity = rest;
    }
    arity = dag.head(&motive_context, arity, true)?;
    if !matches!(dag.store.node(arity), Node::Sort(_)) {
        return Err(invalid("reference projection family is not fully applied"));
    }
    let mut generic = head;
    for argument in &arguments[..parameters] {
        generic = dag.app(generic, *argument)?;
    }
    for (name, _) in &indices {
        let variable = dag.var(name)?;
        generic = dag.app(generic, variable)?;
    }
    let potentially_prop = !Universe::Type(0).is_subtype_of(&dag.sort(&context, ty)?);
    let mut field_context = context.clone();
    let mut telescope = Vec::new();
    loop {
        fields = dag.head(&field_context, fields, true)?;
        if !matches!(dag.store.node(fields), Node::Pi { .. }) {
            break;
        }
        let name = dag.fresh(&field_context, "field")?;
        let (domain, rest) = dag.peel(&field_context, fields, &name)?;
        if potentially_prop
            && telescope.len() < key.index
            && dag.store.occurs_free(rest, &name)
            && !dag.sort(&field_context, domain)?.equiv(&Universe::Prop)
        {
            return Err(invalid(
                "reference dependent projection traverses data in a proof",
            ));
        }
        field_context = dag.extend(&field_context, &name, domain)?;
        telescope.push((name, domain));
        fields = rest;
    }
    let result_type = telescope
        .get(key.index)
        .ok_or_else(|| invalid("reference projection field ordinal outside constructor"))?
        .1;
    let major_name = dag.fresh(&motive_context, "major")?;
    let major = dag.var(&major_name)?;
    motive_context = dag.extend(&motive_context, &major_name, generic)?;
    *scope = scope
        .checked_add(1)
        .ok_or_else(|| resource("reference projection scope limit"))?;
    let next = key.index;
    Ok(Frame {
        key,
        context,
        motive_context,
        motive_scope: *scope,
        telescope,
        indices,
        generic,
        major_name,
        major,
        result_type,
        next,
        potentially_prop,
    })
}

fn finish(dag: &mut Dag<'_>, frame: Frame) -> Result<TermId> {
    if frame.potentially_prop
        && !dag
            .sort(&frame.motive_context, frame.result_type)?
            .equiv(&Universe::Prop)
    {
        return Err(invalid(
            "reference projection cannot extract data from possibly propositional family",
        ));
    }
    let chosen = &frame.telescope[frame.key.index].0;
    let mut branch = dag.var(chosen)?;
    for (name, ty) in frame.telescope.into_iter().rev() {
        branch = dag.lambda(&name, ty, branch)?;
    }
    let mut motive = dag.lambda(&frame.major_name, frame.generic, frame.result_type)?;
    for (name, ty) in frame.indices.into_iter().rev() {
        motive = dag.lambda(&name, ty, motive)?;
    }
    let root = dag.add(Node::Match {
        discriminant: frame.key.value,
        motive,
        cases: vec![branch].into_boxed_slice(),
    })?;
    dag.infer(&frame.context, root)?;
    Ok(root)
}

fn lower(dag: &mut Dag<'_>, root: Request) -> Result<TermId> {
    let mut pending = vec![Task::Enter(root)];
    let mut output = Vec::new();
    let mut cache = HashMap::new();
    let mut scope = 0;
    while let Some(task) = pending.pop() {
        dag.spend(1)?;
        match task {
            Task::Enter(request) => {
                if let Some(&root) = cache.get(&request.key) {
                    output.push(root);
                    continue;
                }
                pending.push(Task::Advance(prepare(dag, request, &mut scope)?));
            }
            Task::Advance(mut frame) => {
                let mut dependency = None;
                while frame.next > 0 {
                    frame.next -= 1;
                    dag.spend(1)?;
                    let name = &frame.telescope[frame.next].0;
                    if dag.store.occurs_free(frame.result_type, name) {
                        dependency = Some(name.clone());
                        break;
                    }
                }
                if let Some(name) = dependency {
                    let request = Request {
                        key: Key {
                            scope: frame.motive_scope,
                            parent: frame.key.parent.clone(),
                            index: frame.next,
                            value: frame.major,
                        },
                        context: frame.motive_context.clone(),
                    };
                    pending.push(Task::Attach { frame, name });
                    pending.push(Task::Enter(request));
                } else {
                    let key = frame.key.clone();
                    let root = finish(dag, frame)?;
                    cache.insert(key, root);
                    output.push(root);
                }
            }
            Task::Attach { mut frame, name } => {
                let root = output
                    .pop()
                    .ok_or_else(|| invalid("reference projection worklist lost a dependency"))?;
                frame.result_type = dag.substitute(frame.result_type, &name, root)?;
                pending.push(Task::Advance(frame));
            }
        }
    }
    if output.len() != 1 {
        return Err(invalid("reference projection worklist result inventory"));
    }
    Ok(output.pop().unwrap())
}

impl ReferenceProjection for ProjectionReader<'_> {
    fn generated_depth_limit(&self) -> Option<usize> {
        Some(self.max_depth.min(crate::type_checker::MAX_TERM_DEPTH))
    }

    fn project(
        &mut self,
        parent: &str,
        index: usize,
        discriminant: Term,
        scope: &[ReferenceBinding<'_>],
        budget: &mut usize,
    ) -> Result<Term> {
        let depth = self.max_depth.min(crate::type_checker::MAX_TERM_DEPTH);
        if depth == 0 {
            return Err(resource("reference projection depth allowance is empty"));
        }
        let node_limit = (*budget).min(u32::MAX as usize);
        // Separate work from cumulative tree allocation. The structural upper bound covers
        // quadratic name-set/substitution metadata over admitted nodes and nesting depth.
        let work = (node_limit as u64)
            .saturating_mul(node_limit as u64)
            .saturating_mul(depth as u64);
        let context = self.context;
        // The result's depth can exceed the syntactic projection depth. All tree boundaries,
        // kernel calls and tree drops execute on the permitted generated-depth stack.
        crate::term_store::grow_stack_for_depth(depth, move || {
            let mut dag = Dag {
                store: TermStore::new(),
                allocation: budget,
                work,
                substitution_allocation: (node_limit as u64).saturating_mul(64).min(1_000_000),
                node_limit,
                depth_limit: depth,
                fresh: 0,
            };
            let mut context = context.clone();
            let mut aliases = Vec::new();
            let mut names = HashSet::new();
            for binding in scope {
                if !names.insert(binding.name) || context.local_name_in_use(binding.name) {
                    return Err(invalid("reference binder collides with supplied local"));
                }
                let ty = dag.intern(binding.ty)?;
                let ty = dag.instantiate(ty, &aliases)?;
                if let Some(value) = binding.value {
                    let value = dag.intern(value)?;
                    let value = dag.instantiate(value, &aliases)?;
                    let actual = dag.infer(&context, value)?;
                    let actual = dag.extract(actual)?;
                    let expected = dag.extract(ty)?;
                    let (equal, stopped) = crate::reduction::observe_resource_stops(|| {
                        crate::type_checker::def_eq(&context, &actual, &expected)
                    });
                    if !equal {
                        return Err(if stopped.is_some() {
                            resource("reference let type comparison did not finish")
                        } else {
                            invalid("reference let value has another source type")
                        });
                    }
                    let name = dag.store.symbol(binding.name);
                    aliases.push((name, value));
                } else {
                    dag.sort(&context, ty)?;
                    context = dag.extend(&context, binding.name, ty)?;
                }
            }
            let value = dag.intern(&discriminant)?;
            let value = dag.instantiate(value, &aliases)?;
            let root = lower(
                &mut dag,
                Request {
                    key: Key {
                        scope: 0,
                        parent: parent.to_owned(),
                        index,
                        value,
                    },
                    context,
                },
            )?;
            dag.extract(root)
        })
    }
}

#[cfg(test)]
mod controls {
    use super::*;

    #[test]
    fn a_new_node_is_refused_before_an_oversized_tree_is_constructed() {
        let mut allowance = 2;
        let mut dag = Dag {
            store: TermStore::new(),
            allocation: &mut allowance,
            work: 10000,
            substitution_allocation: 100_000,
            node_limit: 100,
            depth_limit: 32,
            fresh: 0,
        };
        let a = dag.var("a").unwrap();
        let before = dag.store.len();
        assert!(dag.app(a, a).is_err());
        assert_eq!(dag.store.len(), before);
    }

    #[test]
    fn bounded_substitution_keeps_a_replacement_free_under_a_colliding_binder() {
        let mut store = TermStore::new();
        let ty = store.mk_sort(&Universe::Prop);
        let x = store.mk_var("x");
        let y = store.mk_var("y");
        let body = store.mk_lambda("x", ty, y);
        let symbol = store.symbol("y");
        let mut work = 1_000_000;
        let result = store
            .instantiate_bounded(body, &[(symbol, x)], &mut work, 100, &mut 100_000)
            .unwrap();
        assert!(store.occurs_free(result, "x"));
        assert!(!store.occurs_free(result, "y"));
        let Node::Lambda { param, body, .. } = store.node(result) else {
            panic!("lambda preserved")
        };
        assert_ne!(store.name(*param), "x");
        assert_eq!(*body, x);
    }

    #[test]
    fn substitution_work_exhaustion_does_not_allocate_a_partial_result() {
        let mut store = TermStore::new();
        let x = store.mk_var("x");
        let y = store.mk_var("y");
        let symbol = store.symbol("x");
        let before = store.len();
        let mut work = 0;
        assert!(store
            .instantiate_bounded(x, &[(symbol, y)], &mut work, 100, &mut 100_000)
            .is_err());
        assert_eq!(store.len(), before);
    }

    #[test]
    fn duplicated_substitution_is_admitted_before_any_tree_extraction() {
        let mut allowance = 8;
        let mut dag = Dag {
            store: TermStore::new(),
            allocation: &mut allowance,
            work: 1_000_000,
            substitution_allocation: 100_000,
            node_limit: 100,
            depth_limit: 32,
            fresh: 0,
        };
        let x = dag.var("x").unwrap();
        let pair = dag.app(x, x).unwrap();
        let v = dag.var("v").unwrap();
        let twice = dag.app(v, v).unwrap();
        let four = dag.app(twice, twice).unwrap();
        let before = *dag.allocation;
        assert!(dag.substitute(pair, "x", four).is_err());
        assert_eq!(*dag.allocation, before, "no tree was extracted");
        assert!(dag.store.len() <= dag.node_limit);
    }
}

#[cfg(test)]
#[path = "reference_projection_shared/semantics.rs"]
mod semantics;
