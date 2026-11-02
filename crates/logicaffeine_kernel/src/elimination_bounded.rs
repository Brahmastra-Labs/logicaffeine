//! Test-only bounded nested-inductive elimination proposal.
//! Existing declaration and recursor entry points are unchanged.
use super::{Elimination, NestedBlock, NestedDecl};
use crate::error::KernelError;
use crate::recursor::bounded::{
    Construction, GenerationResult, RecursorBudget, RecursorGenerationError,
};
use crate::term_store::{Node, TermId};
use crate::{Context, MutualInductive, Term};
use logicaffeine_base::Symbol;

#[path = "substitution_exact.rs"]
mod substitution_exact;

fn resource() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
fn invalid(message: &str) -> RecursorGenerationError {
    KernelError::CertificationError(format!("nested-elimination: {message}")).into()
}

struct Nest {
    params: Vec<(Symbol, TermId)>,
    sort: TermId,
    indexed_sort: TermId,
    constructors: Vec<(String, TermId)>,
}
struct Auxiliary {
    name: String,
    container: TermId,
    arguments: Vec<TermId>,
}
struct Member {
    name: String,
    sort: TermId,
    constructors: Vec<(String, TermId)>,
}

fn copy_name(d: &mut Construction<'_>, name: &str) -> GenerationResult<String> {
    d.budget.spend(name.len() as u64 + 1)?;
    d.budget.reserve(name.len() as u64 + 64)?;
    Ok(name.to_owned())
}
fn pi_parts(d: &Construction<'_>, id: TermId) -> Option<(Symbol, TermId, TermId)> {
    match d.store.node(id) {
        Node::Pi {
            param,
            param_type,
            body_type,
        } => Some((*param, *param_type, *body_type)),
        _ => None,
    }
}
fn final_sort(d: &mut Construction<'_>, mut id: TermId) -> GenerationResult<TermId> {
    while let Some((_, _, body)) = pi_parts(d, id) {
        d.budget.spend(1)?;
        id = body;
    }
    Ok(id)
}
/// Preserve admitted syntax while checking the universe meaning of both sort nodes.
fn same_sort(d: &mut Construction<'_>, a: TermId, b: TermId) -> GenerationResult<bool> {
    d.budget.spend(1)?;
    if !matches!(d.store.node(a), Node::Sort(_)) || !matches!(d.store.node(b), Node::Sort(_)) {
        return Ok(false);
    }
    if a == b {
        return Ok(true);
    }
    // Extraction and destruction of the owned input levels share the same stack envelope.
    // equiv_bounded reuses this already-sized stack for its temporary normalization trees.
    crate::term_store::grow_stack_for_depth(d.budget.max_depth, || {
        let a = d.extract(a)?;
        let b = d.extract(b)?;
        let (Term::Sort(a), Term::Sort(b)) = (&a, &b) else {
            unreachable!()
        };
        a.equiv_bounded(b, d.budget)
    })
}

fn close(
    d: &mut Construction<'_>,
    params: &[(Symbol, TermId)],
    mut body: TermId,
) -> GenerationResult<TermId> {
    for &(param, param_type) in params.iter().rev() {
        body = d.add(Node::Pi {
            param,
            param_type,
            body_type: body,
        })?;
    }
    Ok(body)
}
fn nesting(d: &mut Construction<'_>, decl: &NestedDecl, np: usize) -> GenerationResult<Nest> {
    d.budget.inventory(np)?;
    let mut params = Vec::new();
    let mut indexed_sort = d.intern(&decl.sort)?;
    for i in 0..np {
        d.budget.reserve(128)?;
        let name = format!("nested.parameter.{i}");
        let (old, domain, body) =
            pi_parts(d, indexed_sort).ok_or_else(|| invalid("short member parameter telescope"))?;
        let canonical = d.symbol(&name)?;
        let value = d.add(Node::Var(canonical))?;
        params.push((canonical, domain));
        indexed_sort = substitution_exact::substitute(d, body, old, value)?;
    }
    let sort = final_sort(d, indexed_sort)?;
    if !matches!(d.store.node(sort), Node::Sort(_)) {
        return Err(invalid("member telescope does not end in a sort"));
    }
    d.budget.inventory(decl.constructors.len())?;
    let mut constructors = Vec::new();
    for (name, source) in &decl.constructors {
        let mut ty = d.intern(source)?;
        for &(canonical, domain) in &params {
            let (old, actual, body) =
                pi_parts(d, ty).ok_or_else(|| invalid("short constructor parameter telescope"))?;
            if actual != domain {
                return Err(invalid("nested parameters are not uniform"));
            }
            let value = d.add(Node::Var(canonical))?;
            ty = substitution_exact::substitute(d, body, old, value)?;
        }
        constructors.push((copy_name(d, name)?, ty));
    }
    Ok(Nest {
        params,
        sort,
        indexed_sort,
        constructors,
    })
}

struct Eliminator<'a, 'b> {
    ctx: &'a Context,
    d: Construction<'b>,
    params: Vec<(Symbol, TermId)>,
    /// Aux references have precisely this universe instance, including an empty Global head.
    reference_model: TermId,
    members: Vec<String>,
    base: String,
    auxiliaries: Vec<Auxiliary>,
    auxiliary_limit: usize,
}

impl Eliminator<'_, '_> {
    fn spine(&mut self, mut id: TermId) -> GenerationResult<(TermId, Vec<TermId>)> {
        let mut args = Vec::new();
        while let Node::App(f, a) = self.d.store.node(id) {
            let (f, a) = (*f, *a);
            self.d.budget.inventory(1)?;
            args.push(a);
            id = f;
        }
        args.reverse();
        Ok((id, args))
    }
    fn name(&self, id: TermId) -> Option<&str> {
        match self.d.store.node(id) {
            Node::Global(n) | Node::Const { name: n, .. } => Some(self.d.store.name(*n)),
            _ => None,
        }
    }
    /// Traverse every variant, deliberately ignoring local-variable shadowing just like
    /// occurs_anywhere/mentions_var. replace itself traverses only Pi/Lambda/App/Let.
    fn mentions(&mut self, root: TermId, local: Option<Symbol>) -> GenerationResult<bool> {
        self.d.budget.inventory(1)?;
        let mut todo = vec![root];
        while let Some(id) = todo.pop() {
            self.d.budget.spend(1)?;
            let found = match self.d.store.node(id) {
                Node::Var(name) => local == Some(*name),
                Node::Global(name) | Node::Const { name, .. } if local.is_none() => {
                    self.d.budget.spend(self.members.len() as u64)?;
                    self.members.iter().any(|m| m == self.d.store.name(*name))
                }
                _ => false,
            };
            if found {
                return Ok(true);
            }
            self.d
                .budget
                .inventory(self.d.store.node(id).child_count())?;
            self.d
                .store
                .node(id)
                .for_each_child(&mut |child| todo.push(child));
        }
        Ok(false)
    }
    fn any_mentions(&mut self, ids: &[TermId], local: Option<Symbol>) -> GenerationResult<bool> {
        for &id in ids {
            if self.mentions(id, local)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn reference_like(&mut self, name: &str, model: TermId) -> GenerationResult<TermId> {
        let name = self.d.symbol(name)?;
        let node = match self.d.store.node(model) {
            Node::Global(_) => Node::Global(name),
            Node::Const { levels, .. } => {
                self.d.budget.inventory(levels.len())?;
                Node::Const {
                    name,
                    levels: levels.clone(),
                }
            }
            _ => return Err(invalid("container has no reference head")),
        };
        self.d.add(node)
    }
    /// No name, argument vector, members entry, or auxiliary constructor is allocated
    /// for the next distinct instance until the independent inventory admits it.
    fn auxiliary(&mut self, container: TermId, arguments: &[TermId]) -> GenerationResult<usize> {
        for (index, aux) in self.auxiliaries.iter().enumerate() {
            self.d.budget.spend(arguments.len() as u64 + 1)?;
            if aux.container == container && aux.arguments == arguments {
                return Ok(index);
            }
        }
        if self.auxiliaries.len() >= self.auxiliary_limit {
            return Err(resource());
        }
        self.d.budget.inventory(arguments.len().saturating_add(2))?;
        self.d.budget.reserve(self.base.len() as u64 + 128)?;
        let name = format!("{}$nested{}", self.base, self.auxiliaries.len() + 1);
        self.d.budget.spend(self.members.len() as u64 + 1)?;
        if self.ctx.get_global(&name).is_some() || self.members.contains(&name) {
            return Err(invalid("auxiliary name is already taken"));
        }
        let member = copy_name(&mut self.d, &name)?;
        self.members.push(member);
        self.auxiliaries.push(Auxiliary {
            name,
            container,
            arguments: arguments.to_vec(),
        });
        Ok(self.auxiliaries.len() - 1)
    }
    fn replace(&mut self, root: TermId) -> GenerationResult<TermId> {
        #[derive(Clone, Copy)]
        enum Shape {
            Pi(Symbol),
            Lambda(Symbol),
            App,
            Let(Symbol),
        }
        enum Op {
            Visit(TermId),
            Build(Shape),
            Bind(Symbol),
            Unbind,
        }
        self.d.budget.inventory(1)?;
        let mut todo = vec![Op::Visit(root)];
        let mut out = Vec::new();
        let mut locals = Vec::new();
        while let Some(op) = todo.pop() {
            self.d.budget.spend(1)?;
            match op {
                Op::Bind(name) => {
                    self.d.budget.inventory(1)?;
                    locals.push(name);
                }
                Op::Unbind => {
                    locals.pop().expect("matching binder continuation");
                }
                Op::Build(shape) => {
                    let body = out.pop().expect("rewritten child");
                    let node = match shape {
                        Shape::Pi(param) => Node::Pi {
                            param,
                            param_type: out.pop().unwrap(),
                            body_type: body,
                        },
                        Shape::Lambda(param) => Node::Lambda {
                            param,
                            param_type: out.pop().unwrap(),
                            body,
                        },
                        Shape::App => Node::App(out.pop().unwrap(), body),
                        Shape::Let(name) => {
                            let value = out.pop().unwrap();
                            Node::Let {
                                name,
                                ty: out.pop().unwrap(),
                                value,
                                body,
                            }
                        }
                    };
                    let id = self.d.add(node)?;
                    self.d.budget.inventory(1)?;
                    out.push(id);
                }
                Op::Visit(id) => {
                    let (head, args) = self.spine(id)?;
                    // Finish the immutable name borrow before scans mutate accounting.
                    let candidate = match self.d.store.node(head) {
                        Node::Global(symbol) | Node::Const { name: symbol, .. } => {
                            let name = self.d.store.name(*symbol);
                            self.d.budget.spend(self.members.len() as u64 + 1)?;
                            if !self.members.iter().any(|m| m == name)
                                && self.ctx.is_inductive(name)
                            {
                                Some((
                                    self.ctx.inductive_num_params(name),
                                    self.ctx.inductive_arity(name),
                                ))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some((p, arity)) = candidate {
                        if args.len() == arity
                            && p <= args.len()
                            && self.any_mentions(&args[..p], None)?
                        {
                            for &local in &locals {
                                if self.any_mentions(&args[..p], Some(local))? {
                                    return Err(KernelError::InvalidDeclaration("nested-elimination: a container parameter mentions a constructor binder".into()).into());
                                }
                            }
                            if self.any_mentions(&args[p..], None)? {
                                return Err(invalid("a container index mentions the block"));
                            }
                            let index = self.auxiliary(head, &args[..p])?;
                            // Charge before copying the auxiliary name for the mutable helper.
                            let name = copy_name(&mut self.d, &self.auxiliaries[index].name)?;
                            let mut rewritten = self.reference_like(&name, self.reference_model)?;
                            for i in 0..self.params.len() {
                                let v = self.d.add(Node::Var(self.params[i].0))?;
                                rewritten = self.d.app(rewritten, v)?;
                            }
                            for &arg in &args[p..] {
                                rewritten = self.d.app(rewritten, arg)?;
                            }
                            self.d.budget.inventory(1)?;
                            out.push(rewritten);
                            continue;
                        }
                    }
                    // Charge continuation storage before pushing. Reverse order yields the
                    // original left-to-right top-down discovery sequence without native recursion.
                    self.d.budget.inventory(7)?;
                    match self.d.store.node(id) {
                        Node::Pi {
                            param,
                            param_type,
                            body_type,
                        } => {
                            todo.extend([
                                Op::Build(Shape::Pi(*param)),
                                Op::Unbind,
                                Op::Visit(*body_type),
                                Op::Bind(*param),
                                Op::Visit(*param_type),
                            ]);
                        }
                        Node::Lambda {
                            param,
                            param_type,
                            body,
                        } => {
                            todo.extend([
                                Op::Build(Shape::Lambda(*param)),
                                Op::Unbind,
                                Op::Visit(*body),
                                Op::Bind(*param),
                                Op::Visit(*param_type),
                            ]);
                        }
                        Node::App(f, a) => {
                            todo.extend([Op::Build(Shape::App), Op::Visit(*a), Op::Visit(*f)]);
                        }
                        Node::Let {
                            name,
                            ty,
                            value,
                            body,
                        } => {
                            todo.extend([
                                Op::Build(Shape::Let(*name)),
                                Op::Unbind,
                                Op::Visit(*body),
                                Op::Bind(*name),
                                Op::Visit(*value),
                                Op::Visit(*ty),
                            ]);
                        }
                        _ => out.push(id),
                    }
                }
            }
        }
        Ok(out.pop().expect("one rewritten root"))
    }
}

/// `expected_recursors` is the independently decoded source inventory, including source
/// members. Sharing this budget across later generation/restoration is the caller's job.
/// Kernel inference is the existing checking boundary and retains its own resource limits.
pub(crate) fn eliminate_nested_bounded(
    ctx: &Context,
    block: &NestedBlock<'_>,
    expected_recursors: usize,
    budget: &mut RecursorBudget,
) -> GenerationResult<Elimination> {
    let first = block
        .members
        .first()
        .ok_or_else(|| invalid("an empty block"))?;
    let auxiliary_limit = expected_recursors
        .checked_sub(block.members.len())
        .ok_or_else(resource)?;
    let mut d = Construction::new(budget)?;
    d.budget.inventory(block.members.len())?;
    let mut nests = Vec::new();
    for member in block.members {
        nests.push(nesting(&mut d, member, block.num_params)?);
    }
    let sort = nests[0].sort;
    for nest in &nests {
        if nest.params != nests[0].params {
            return Err(invalid("members do not share uniform parameters"));
        }
        if !same_sort(&mut d, nest.sort, sort)? {
            return Err(invalid("members live in different sorts"));
        }
    }
    d.budget.inventory(nests[0].params.len())?;
    let params = nests[0].params.clone();
    d.budget.inventory(block.members.len())?;
    let mut members = Vec::new();
    for member in block.members {
        members.push(copy_name(&mut d, &member.name)?);
    }
    let base = copy_name(&mut d, &first.name)?;
    // reference_at only constructs a small reference from an admitted level-name inventory.
    d.budget.inventory(block.levels.len())?;
    d.budget.reserve(first.name.len() as u64 + 64)?;
    for level in block.levels {
        d.budget.reserve(level.len() as u64 + 64)?;
    }
    let reference_model = d.intern(&super::reference_at(&first.name, block.levels))?;
    let mut e = Eliminator {
        ctx,
        d,
        params,
        reference_model,
        members,
        base,
        auxiliaries: Vec::new(),
        auxiliary_limit,
    };
    let mut emitted = Vec::new();
    for (member, nest) in block.members.iter().zip(nests) {
        e.d.budget
            .inventory(nest.constructors.len().saturating_add(1))?;
        let mut constructors = Vec::new();
        for (name, ty) in nest.constructors {
            let rewritten = e.replace(ty)?;
            constructors.push((name, close(&mut e.d, &e.params, rewritten)?));
        }
        emitted.push(Member {
            name: copy_name(&mut e.d, &member.name)?,
            sort: close(&mut e.d, &e.params, nest.indexed_sort)?,
            constructors,
        });
    }
    let mut restoration = Vec::new();
    let mut next = 0;
    while next < e.auxiliaries.len() {
        let aux = &e.auxiliaries[next];
        let name = copy_name(&mut e.d, &aux.name)?;
        let container = aux.container;
        e.d.budget.inventory(aux.arguments.len())?;
        let arguments = aux.arguments.clone();
        let container_symbol = match e.d.store.node(container) {
            Node::Global(n) | Node::Const { name: n, .. } => *n,
            _ => return Err(invalid("container has no reference head")),
        };
        // Copying the name is charged before taking a mutable Construction borrow.
        let container_name = {
            let s = e.d.store.name(container_symbol);
            e.d.budget.reserve(s.len() as u64 + 64)?;
            s.to_owned()
        };
        let mut arity = e.d.infer(ctx, container)?;
        for &argument in &arguments {
            let (param, _, body) = pi_parts(&e.d, arity)
                .ok_or_else(|| invalid("container has fewer parameters than applied"))?;
            arity = substitution_exact::substitute(&mut e.d, body, param, argument)?;
        }
        let result_sort = final_sort(&mut e.d, arity)?;
        if !same_sort(&mut e.d, result_sort, sort)? {
            return Err(invalid("nested instance sort differs from block sort"));
        }
        // get_constructors itself allocates a borrowed inventory; reserve its known count first.
        e.d.budget
            .inventory(ctx.constructor_inventory_upper_bound(&container_name))?;
        let constructor_inventory = ctx.get_constructors(&container_name);
        let mut constructors = Vec::new();
        e.d.budget.inventory(
            constructor_inventory
                .len()
                .saturating_mul(2)
                .saturating_add(2),
        )?;
        for (constructor, _) in constructor_inventory {
            let instance = e.reference_like(constructor, container)?;
            let mut ty = e.d.infer(ctx, instance)?;
            for &argument in &arguments {
                let (param, _, body) = pi_parts(&e.d, ty)
                    .ok_or_else(|| invalid("constructor has fewer parameters than its family"))?;
                ty = substitution_exact::substitute(&mut e.d, body, param, argument)?;
            }
            let rewritten = e.replace(ty)?;
            e.d.budget
                .reserve(name.len() as u64 + constructor.len() as u64 + 64)?;
            let auxiliary_constructor = format!("{name}${constructor}");
            let mut image = instance;
            for &argument in &arguments {
                image = e.d.app(image, argument)?;
            }
            restoration.push((copy_name(&mut e.d, &auxiliary_constructor)?, image));
            constructors.push((
                auxiliary_constructor,
                close(&mut e.d, &e.params, rewritten)?,
            ));
        }
        let mut image = container;
        for &argument in &arguments {
            image = e.d.app(image, argument)?;
        }
        restoration.push((copy_name(&mut e.d, &name)?, image));
        emitted.push(Member {
            name,
            sort: close(&mut e.d, &e.params, arity)?,
            constructors,
        });
        next += 1;
    }
    if e.auxiliaries.is_empty() {
        return Err(invalid("no nested occurrence; register directly"));
    }
    if emitted.len() != expected_recursors {
        return Err(invalid(
            "source recursor inventory differs from elimination",
        ));
    }
    // All generated fields remain DAG ids until every auxiliary has been processed.
    // Each tree copy is admitted independently, against one cumulative materialization meter.
    e.d.budget.inventory(
        emitted
            .len()
            .saturating_add(restoration.len())
            .saturating_add(e.params.len()),
    )?;
    let mut output = Vec::new();
    for member in emitted {
        e.d.budget.inventory(member.constructors.len())?;
        let mut constructors = Vec::new();
        for (name, ty) in member.constructors {
            constructors.push((name, e.d.extract(ty)?));
        }
        output.push(MutualInductive {
            name: member.name,
            sort: e.d.extract(member.sort)?,
            num_params: block.num_params,
            constructors,
        });
    }
    let mut restored = Vec::new();
    for (name, image) in restoration {
        restored.push((name, e.d.extract(image)?));
    }
    let mut parameters = Vec::new();
    for (name, ty) in e.params {
        let name = {
            let s = e.d.store.name(name);
            e.d.budget.reserve(s.len() as u64 + 64)?;
            s.to_owned()
        };
        parameters.push((name, e.d.extract(ty)?));
    }
    Ok(Elimination {
        block: output,
        restoration: restored,
        parameters,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/bounded_elimination/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/inductive_compile/bounded_elimination/universe_controls.rs"]
mod universe_controls;
