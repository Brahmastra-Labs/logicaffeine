//! Test-only typed projection provider using the shared generator admission layer.
//! Keep one reader/budget alive across source binding; no per-projection meter resets.
use super::reference::{ReferenceBinding, ReferenceProjection};
use super::{declined, rejected, DiagnosticCode, Result};
use crate::recursor::bounded::{Construction, RecursorBudget, RecursorGenerationError};
use crate::term_store::{Node, TermId};
use crate::{Context, Term, Universe};
use logicaffeine_base::Symbol;
use std::collections::{HashMap, HashSet};

pub(super) struct SharedProjectionReader<'a, 'b> {
    pub context: &'a Context,
    pub max_depth: usize,
    pub budget: &'b mut RecursorBudget,
}
fn resource(message: &str) -> super::Verdict {
    declined(DiagnosticCode::ResourceLimit, message)
}
fn invalid(message: &str) -> super::Verdict {
    rejected(DiagnosticCode::InvalidProof, message)
}
fn generation(error: RecursorGenerationError) -> super::Verdict {
    match error {
        RecursorGenerationError::ResourceLimit => {
            resource("reference projection shared construction limit")
        }
        RecursorGenerationError::Kernel(error) => super::kernel(error),
    }
}
struct Dag<'a> {
    d: Construction<'a>,
    allocation: &'a mut usize,
    fresh: u64,
}
impl Dag<'_> {
    fn spend(&mut self, amount: u64) -> Result<()> {
        self.d.budget.spend(amount).map_err(generation)
    }
    fn inventory(&mut self, count: usize) -> Result<()> {
        self.d.budget.inventory(count).map_err(generation)
    }
    fn reserve(&mut self, bytes: u64) -> Result<()> {
        self.d.budget.reserve(bytes).map_err(generation)
    }
    fn copy_name(&mut self, name: &str) -> Result<String> {
        self.spend(name.len() as u64 + 1)?;
        self.reserve(name.len() as u64 + 64)?;
        Ok(name.to_owned())
    }
    fn admit_context_clone(&mut self, ctx: &Context) -> Result<()> {
        self.reserve((std::mem::size_of::<Context>() as u64).saturating_add(256))?;
        self.inventory(ctx.local_binding_capacity())?;
        for name in ctx.local_binding_names() {
            self.spend(name.len() as u64 + 1)?;
            self.reserve(name.len() as u64 + 64)?;
        }
        Ok(())
    }
    fn clone_context(&mut self, ctx: &Context) -> Result<Context> {
        self.admit_context_clone(ctx)?;
        Ok(ctx.clone())
    }
    fn symbol(&mut self, name: &str) -> Result<Symbol> {
        self.d.symbol(name).map_err(generation)
    }
    fn admit(&self, root: TermId) -> Result<()> {
        self.d.admit(root).map_err(generation)?;
        if self.d.store.tree_size(root) > *self.allocation as u64 {
            return Err(resource("reference projection source tree allowance"));
        }
        Ok(())
    }
    fn intern(&mut self, term: &Term) -> Result<TermId> {
        let id = self.d.intern(term).map_err(generation)?;
        self.admit(id)?;
        Ok(id)
    }
    fn add(&mut self, node: Node) -> Result<TermId> {
        let mut nodes = 1u64;
        node.for_each_child(&mut |child| {
            nodes = nodes.saturating_add(self.d.store.tree_size(child))
        });
        if nodes > *self.allocation as u64 {
            return Err(resource(
                "reference projection source construction allowance",
            ));
        }
        self.d.add(node).map_err(generation)
    }
    fn var(&mut self, name: &str) -> Result<TermId> {
        let name = self.symbol(name)?;
        self.add(Node::Var(name))
    }
    fn app(&mut self, f: TermId, a: TermId) -> Result<TermId> {
        self.add(Node::App(f, a))
    }
    fn lambda(&mut self, name: &str, ty: TermId, body: TermId) -> Result<TermId> {
        let param = self.symbol(name)?;
        self.add(Node::Lambda {
            param,
            param_type: ty,
            body,
        })
    }
    fn constant_like(&mut self, name: &str, model: TermId) -> Result<TermId> {
        let symbol = self.symbol(name)?;
        let levels = match self.d.store.node(model) {
            Node::Const { levels, .. } => Some(levels.len()),
            Node::Global(_) => None,
            _ => return Err(invalid("reference family has no constant head")),
        };
        if let Some(count) = levels {
            self.inventory(count)?;
        }
        let node = match self.d.store.node(model) {
            Node::Const { levels, .. } => Node::Const {
                name: symbol,
                levels: levels.clone(),
            },
            _ => Node::Global(symbol),
        };
        self.add(node)
    }
    fn instantiate(&mut self, root: TermId, pairs: &[(Symbol, TermId)]) -> Result<TermId> {
        let id = self.d.instantiate(root, pairs).map_err(generation)?;
        self.admit(id)?;
        Ok(id)
    }
    fn substitute(&mut self, root: TermId, name: &str, value: TermId) -> Result<TermId> {
        let name = self.symbol(name)?;
        self.instantiate(root, &[(name, value)])
    }
    fn extract(&mut self, root: TermId) -> Result<Term> {
        self.admit(root)?;
        let nodes = usize::try_from(self.d.store.tree_size(root))
            .map_err(|_| resource("reference projection tree overflow"))?;
        *self.allocation = self
            .allocation
            .checked_sub(nodes)
            .ok_or_else(|| resource("reference projection source tree allocation"))?;
        self.d.extract(root).map_err(generation)
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
        self.admit_context_clone(ctx)?;
        self.reserve(name.len() as u64 + 128)?;
        Ok(ctx.extend(name, self.extract(ty)?))
    }
    fn fresh(&mut self, ctx: &Context, kind: &str) -> Result<String> {
        loop {
            self.spend(kind.len() as u64 + 64)?;
            self.reserve(kind.len() as u64 + 128)?;
            self.fresh = self
                .fresh
                .checked_add(1)
                .ok_or_else(|| resource("reference projection fresh-name limit"))?;
            let name = format!("reference.projection.{kind}.{}", self.fresh);
            if self.d.store.lookup_symbol(&name).is_none() && !ctx.local_name_in_use(&name) {
                return Ok(name);
            }
        }
    }
    fn head(&mut self, ctx: &Context, root: TermId, function: bool) -> Result<TermId> {
        if function && matches!(self.d.store.node(root), Node::Pi { .. } | Node::Sort(_)) {
            return Ok(root);
        }
        if ctx.simp_rules().is_empty() {
            let mut head = root;
            while let Node::App(f, _) = self.d.store.node(head) {
                let next = *f;
                self.spend(1)?;
                head = next;
            }
            if let Node::Global(name) | Node::Const { name, .. } = self.d.store.node(head) {
                if ctx.is_inductive(self.d.store.name(*name)) {
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
        let (old, domain, body) = match self.d.store.node(root) {
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
    while let Node::App(fun, arg) = dag.d.store.node(head) {
        let (fun, arg) = (*fun, *arg);
        dag.inventory(1)?;
        arguments.push(arg);
        head = fun;
    }
    arguments.reverse();
    let parent_symbol = match dag.d.store.node(head) {
        Node::Global(name) | Node::Const { name, .. } => *name,
        _ => return Err(invalid("reference projection value has no family head")),
    };
    // Parent equality plus the four metadata map lookups below. Each lookup can
    // hash and compare the complete key; charge these independently of key copies.
    let parent_length = dag.d.store.name(parent_symbol).len() as u64;
    dag.spend(
        parent_length
            .saturating_add(key.parent.len() as u64)
            .saturating_add(1),
    )?;
    dag.spend((key.parent.len() as u64 + 1).saturating_mul(8))?;
    let parent = dag.d.store.name(parent_symbol);
    if parent != key.parent || !context.is_inductive(&key.parent) {
        return Err(invalid("reference projection parent differs from source"));
    }
    dag.spend(context.constructor_inventory_upper_bound(&key.parent) as u64)?;
    let mut headers = context.constructor_headers(&key.parent);
    let ctor_header = headers
        .next()
        .ok_or_else(|| invalid("reference projection requires one constructor"))?;
    if headers.next().is_some() {
        return Err(invalid("reference projection requires one constructor"));
    }
    let parameters = context.inductive_num_params(&key.parent);
    if arguments.len() < parameters {
        return Err(invalid("reference projection lacks parameters"));
    }
    let ctor = dag.constant_like(ctor_header.0, head)?;
    drop(headers);
    let mut fields = dag.infer(&context, ctor)?;
    let mut arity = dag.infer(&context, head)?;
    for argument in &arguments[..parameters] {
        let name = dag.fresh(&context, "parameter")?;
        let (_, rest) = dag.peel(&context, arity, &name)?;
        arity = dag.substitute(rest, &name, *argument)?;
        let (_, rest) = dag.peel(&context, fields, &name)?;
        fields = dag.substitute(rest, &name, *argument)?;
    }
    let mut motive_context = dag.clone_context(&context)?;
    let mut indices = Vec::new();
    for _ in parameters..arguments.len() {
        let name = dag.fresh(&motive_context, "index")?;
        let (domain, rest) = dag.peel(&motive_context, arity, &name)?;
        motive_context = dag.extend(&motive_context, &name, domain)?;
        dag.inventory(1)?;
        indices.push((name, domain));
        arity = rest;
    }
    arity = dag.head(&motive_context, arity, true)?;
    if !matches!(dag.d.store.node(arity), Node::Sort(_)) {
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
    let mut field_context = dag.clone_context(&context)?;
    let mut telescope = Vec::new();
    loop {
        fields = dag.head(&field_context, fields, true)?;
        if !matches!(dag.d.store.node(fields), Node::Pi { .. }) {
            break;
        }
        let name = dag.fresh(&field_context, "field")?;
        let (domain, rest) = dag.peel(&field_context, fields, &name)?;
        if potentially_prop
            && telescope.len() < key.index
            && dag.d.store.occurs_free(rest, &name)
            && !dag.sort(&field_context, domain)?.equiv(&Universe::Prop)
        {
            return Err(invalid(
                "reference dependent projection traverses data in a proof",
            ));
        }
        field_context = dag.extend(&field_context, &name, domain)?;
        dag.inventory(1)?;
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
    dag.inventory(1)?;
    let root = dag.add(Node::Match {
        discriminant: frame.key.value,
        motive,
        cases: vec![branch].into_boxed_slice(),
    })?;
    dag.infer(&frame.context, root)?;
    Ok(root)
}

fn lower(dag: &mut Dag<'_>, root: Request) -> Result<TermId> {
    dag.inventory(1)?;
    let mut pending = vec![Task::Enter(root)];
    let mut output = Vec::new();
    let mut cache = HashMap::new();
    let mut cache_name_bytes = 0u64;
    let mut scope = 0;
    while let Some(task) = pending.pop() {
        dag.spend(1)?;
        match task {
            Task::Enter(request) => {
                // Query hashing plus a conservative comparison pass over every
                // resident key, even if hash collisions defeat ordinary O(1) lookup.
                dag.spend(
                    (request.key.parent.len() as u64 + 32)
                        .saturating_mul(cache.len() as u64 + 1)
                        .saturating_add(cache_name_bytes),
                )?;
                if let Some(&root) = cache.get(&request.key) {
                    dag.inventory(1)?;
                    output.push(root);
                    continue;
                }
                dag.inventory(1)?;
                pending.push(Task::Advance(prepare(dag, request, &mut scope)?));
            }
            Task::Advance(mut frame) => {
                let mut dependency = None;
                while frame.next > 0 {
                    frame.next -= 1;
                    dag.spend(1)?;
                    let name = &frame.telescope[frame.next].0;
                    if dag.d.store.occurs_free(frame.result_type, name) {
                        dependency = Some(dag.copy_name(name)?);
                        break;
                    }
                }
                if let Some(name) = dependency {
                    dag.inventory(3)?;
                    let request = Request {
                        key: Key {
                            scope: frame.motive_scope,
                            parent: dag.copy_name(&frame.key.parent)?,
                            index: frame.next,
                            value: frame.major,
                        },
                        context: dag.clone_context(&frame.motive_context)?,
                    };
                    pending.push(Task::Attach { frame, name });
                    pending.push(Task::Enter(request));
                } else {
                    dag.inventory(2)?;
                    let key = Key {
                        scope: frame.key.scope,
                        parent: dag.copy_name(&frame.key.parent)?,
                        index: frame.key.index,
                        value: frame.key.value,
                    };
                    let root = finish(dag, frame)?;
                    // Include possible growth rehash of all retained parent names.
                    dag.spend(
                        (key.parent.len() as u64 + 32)
                            .saturating_mul(cache.len() as u64 + 1)
                            .saturating_add(cache_name_bytes.saturating_mul(2)),
                    )?;
                    cache_name_bytes = cache_name_bytes.saturating_add(key.parent.len() as u64);
                    cache.insert(key, root);
                    output.push(root);
                }
            }
            Task::Attach { mut frame, name } => {
                let root = output
                    .pop()
                    .ok_or_else(|| invalid("reference projection worklist lost a dependency"))?;
                frame.result_type = dag.substitute(frame.result_type, &name, root)?;
                dag.inventory(1)?;
                pending.push(Task::Advance(frame));
            }
        }
    }
    if output.len() != 1 {
        return Err(invalid("reference projection worklist result inventory"));
    }
    Ok(output.pop().unwrap())
}

impl ReferenceProjection for SharedProjectionReader<'_, '_> {
    fn generated_depth_limit(&self) -> Option<usize> {
        Some(
            self.max_depth
                .min(self.budget.max_depth)
                .min(crate::type_checker::MAX_TERM_DEPTH),
        )
    }
    fn project(
        &mut self,
        parent: &str,
        index: usize,
        discriminant: Term,
        scope: &[ReferenceBinding<'_>],
        budget: &mut usize,
    ) -> Result<Term> {
        let depth = self.generated_depth_limit().unwrap();
        if depth == 0 {
            return Err(resource("reference projection depth allowance is empty"));
        }
        // Caps may tighten for this decode call, but cumulative counters are the caller's
        // remaining balances. Copy them back even on refusal; no operation replenishes them.
        let mut operation = RecursorBudget {
            max_depth: depth,
            max_tree_nodes: self.budget.max_tree_nodes.min(*budget as u64),
            max_store_nodes: self
                .budget
                .max_store_nodes
                .min(*budget)
                .min(u32::MAX as usize),
            work: self.budget.work,
            allocation: self.budget.allocation,
            materialized_nodes: self.budget.materialized_nodes,
        };
        let context = self.context;
        let operation_ref = &mut operation;
        let result = crate::term_store::grow_stack_for_depth(depth, move || {
            // Own the caller's tree on this stack so its destruction is covered too.
            let discriminant = discriminant;
            let d = Construction::new(operation_ref).map_err(generation)?;
            let mut dag = Dag {
                d,
                allocation: budget,
                fresh: 0,
            };
            let mut context = dag.clone_context(context)?;
            let mut aliases = Vec::new();
            let mut names = HashSet::new();
            for binding in scope {
                dag.inventory(2)?;
                dag.spend(binding.name.len() as u64 + 1)?;
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
                    let name = dag.symbol(binding.name)?;
                    aliases.push((name, value));
                } else {
                    dag.sort(&context, ty)?;
                    context = dag.extend(&context, binding.name, ty)?;
                }
            }
            let value = dag.intern(&discriminant)?;
            let value = dag.instantiate(value, &aliases)?;
            let parent = dag.copy_name(parent)?;
            let root = lower(
                &mut dag,
                Request {
                    key: Key {
                        scope: 0,
                        parent,
                        index,
                        value,
                    },
                    context,
                },
            )?;
            dag.extract(root)
        });
        self.budget.work = operation.work;
        self.budget.allocation = operation.allocation;
        self.budget.materialized_nodes = operation.materialized_nodes;
        result
    }
}

// Compatibility harness for the byte-identical existing semantic control file.
// Its reader literals have only context/max_depth fields. The test thread owns
// one cumulative budget, so even this harness does not reset on project calls.
#[cfg(test)]
pub(super) struct ProjectionReader<'a> {
    pub context: &'a Context,
    pub max_depth: usize,
}
#[cfg(test)]
thread_local! {
    static SEMANTICS_BUDGET:std::cell::RefCell<RecursorBudget>=std::cell::RefCell::new(RecursorBudget{
        max_depth:crate::type_checker::MAX_TERM_DEPTH,max_tree_nodes:20_000_000,
        max_store_nodes:20_000_000,work:1_000_000_000_000_000,
        allocation:1_000_000_000_000,materialized_nodes:200_000_000,
    });
}
#[cfg(test)]
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
        SEMANTICS_BUDGET.with(|shared| {
            let mut shared = shared.borrow_mut();
            SharedProjectionReader {
                context: self.context,
                max_depth: self.max_depth,
                budget: &mut shared,
            }
            .project(parent, index, discriminant, scope, budget)
        })
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference_projection_shared/controls.rs"]
mod controls;
#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference_projection_shared/semantics.rs"]
mod semantics;
