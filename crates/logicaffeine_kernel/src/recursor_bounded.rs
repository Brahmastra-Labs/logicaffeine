//! Bounded ordinary/mutual recursor construction. Kernel checking retains its own limits.
//! No generated Term tree is materialized until DAG depth and unfolded size are admitted.
use crate::error::KernelError;
use crate::term_store::{Node, TermId, TermStore};
use crate::{Context, Term, Universe};

#[derive(Debug)]
pub enum RecursorGenerationError {
    ResourceLimit,
    Kernel(KernelError),
}
impl From<KernelError> for RecursorGenerationError {
    fn from(error: KernelError) -> Self {
        Self::Kernel(error)
    }
}
pub type GenerationResult<T> = Result<T, RecursorGenerationError>;
use logicaffeine_base::Symbol;

/// Caller-owned shared limits. Reuse one budget across every sibling/block in an operation.
/// Allocation is counted in conservative 64-byte credits, as in bounded store substitution.
#[derive(Debug)]
pub struct RecursorBudget {
    pub max_depth: usize,
    pub max_tree_nodes: u64,
    pub max_store_nodes: usize,
    pub work: u64,
    pub allocation: u64,
    pub materialized_nodes: u64,
}
fn limit() -> RecursorGenerationError {
    RecursorGenerationError::ResourceLimit
}
fn invalid(message: &str) -> RecursorGenerationError {
    RecursorGenerationError::Kernel(KernelError::CertificationError(message.into()))
}
impl RecursorBudget {
    pub(crate) fn spend(&mut self, n: u64) -> GenerationResult<()> {
        self.work = self.work.checked_sub(n).ok_or_else(limit)?;
        Ok(())
    }
    pub(crate) fn reserve(&mut self, bytes: u64) -> GenerationResult<()> {
        let credits = bytes.checked_add(63).ok_or_else(limit)? / 64;
        self.allocation = self.allocation.checked_sub(credits).ok_or_else(limit)?;
        Ok(())
    }
    pub(crate) fn inventory(&mut self, n: usize) -> GenerationResult<()> {
        if n > self.max_store_nodes {
            return Err(limit());
        }
        self.spend(n as u64)?;
        self.reserve((n as u64).saturating_mul(128))
    }
}
pub(crate) struct Construction<'a> {
    pub(crate) store: TermStore,
    pub(crate) budget: &'a mut RecursorBudget,
    payload_envelope: u64,
    input_envelope: u64,
}
type Dag<'a> = Construction<'a>;
impl<'a> Construction<'a> {
    pub(crate) fn new(budget: &'a mut RecursorBudget) -> GenerationResult<Self> {
        if budget.max_depth > crate::type_checker::MAX_TERM_DEPTH
            || budget.max_store_nodes > u32::MAX as usize
        {
            return Err(limit());
        }
        budget.reserve(4096)?;
        Ok(Self {
            store: TermStore::new(),
            budget,
            payload_envelope: 0,
            input_envelope: 0,
        })
    }
    pub(crate) fn admit(&self, id: TermId) -> GenerationResult<()> {
        if self.store.depth(id) as usize > self.budget.max_depth
            || self.store.tree_size(id) > self.budget.max_tree_nodes
        {
            return Err(limit());
        }
        Ok(())
    }
    /// Admit a borrowed universe before the existing recursive interner copies it.
    pub(crate) fn level(
        &mut self,
        level: &Universe,
    ) -> GenerationResult<crate::term_store::LevelId> {
        self.budget.inventory(1)?;
        let mut todo = vec![(level, 1usize)];
        let mut count = 0u64;
        let mut bytes = 0u64;
        while let Some((level, depth)) = todo.pop() {
            self.budget.spend(1)?;
            count = count.checked_add(1).ok_or_else(limit)?;
            if depth > self.budget.max_depth
                || count > self.budget.max_tree_nodes
                || count > self.budget.max_store_nodes as u64
            {
                return Err(limit());
            }
            match level {
                Universe::Var(name) => {
                    self.budget.spend(name.len() as u64)?;
                    bytes = bytes.saturating_add(name.len() as u64);
                }
                Universe::Succ(a) => {
                    self.budget.inventory(1)?;
                    todo.push((a, depth + 1));
                }
                Universe::Max(a, b) | Universe::IMax(a, b) => {
                    self.budget.inventory(2)?;
                    todo.push((a, depth + 1));
                    todo.push((b, depth + 1));
                }
                _ => {}
            }
        }
        // A per-call size limit alone cannot protect LevelId's u32 inventory.
        // Each source node can introduce at most one distinct level; admit before interning.
        let additional = usize::try_from(count).map_err(|_| limit())?;
        if self
            .store
            .level_count()
            .checked_add(additional)
            .is_none_or(|n| n > self.budget.max_store_nodes)
        {
            return Err(limit());
        }
        self.budget.spend(count.saturating_add(bytes))?;
        self.budget.reserve(
            count
                .saturating_mul(256)
                .saturating_add(bytes.saturating_mul(4)),
        )?;
        self.payload_envelope = self
            .payload_envelope
            .max(bytes.saturating_add(count.saturating_mul(64)));
        self.input_envelope = self
            .input_envelope
            .saturating_add(bytes)
            .saturating_add(count);
        Ok(self.store.level_of(level))
    }

    /// First-occurrence universe telescope of the actual generated DAG, with bounded traversal
    /// and allocation. Matches term::free_level_vars' left-to-right term/level order.
    pub(crate) fn free_level_vars(&mut self, root: TermId) -> GenerationResult<Vec<String>> {
        use crate::term_store::{LevelId, LevelView};
        enum Item {
            Term(TermId),
            Level(LevelId),
        }
        self.admit(root)?;
        self.budget.inventory(1)?;
        let mut todo = vec![Item::Term(root)];
        let mut out: Vec<String> = Vec::new();
        while let Some(item) = todo.pop() {
            self.budget.spend(1)?;
            match item {
                Item::Term(id) => {
                    let node = self.store.node(id);
                    match node {
                        Node::Sort(level) => {
                            self.budget.inventory(1)?;
                            todo.push(Item::Level(*level));
                        }
                        Node::Const { levels, .. } => {
                            self.budget.inventory(levels.len())?;
                            todo.extend(levels.iter().rev().map(|level| Item::Level(*level)));
                        }
                        _ => {
                            self.budget.inventory(node.child_count())?;
                            let start = todo.len();
                            node.for_each_child(&mut |child| todo.push(Item::Term(child)));
                            todo[start..].reverse();
                        }
                    }
                }
                Item::Level(id) => match self.store.level_view(id) {
                    LevelView::Var(name) => {
                        self.budget
                            .spend((name.len() as u64 + 1).saturating_mul(out.len() as u64 + 1))?;
                        if !out.iter().any(|present| present == name) {
                            self.budget.inventory(1)?;
                            self.budget.reserve(name.len() as u64 + 64)?;
                            out.push(name.to_string());
                        }
                    }
                    LevelView::Succ(child) => {
                        self.budget.inventory(1)?;
                        todo.push(Item::Level(child));
                    }
                    LevelView::Max(a, b) | LevelView::IMax(a, b) => {
                        self.budget.inventory(2)?;
                        todo.push(Item::Level(b));
                        todo.push(Item::Level(a));
                    }
                    _ => {}
                },
            }
        }
        Ok(out)
    }

    pub(crate) fn symbol(&mut self, name: &str) -> GenerationResult<Symbol> {
        self.budget.spend(name.len() as u64 + 1)?;
        self.budget
            .reserve((name.len() as u64).saturating_mul(2).saturating_add(128))?;
        self.payload_envelope = self.payload_envelope.max(name.len() as u64 + 64);
        Ok(self.store.symbol(name))
    }
    pub(crate) fn add(&mut self, node: Node) -> GenerationResult<TermId> {
        self.budget.spend(node.child_count() as u64 + 1)?;
        let mut depth = 1usize;
        let mut nodes = 1u64;
        node.for_each_child(&mut |c| {
            depth = depth.max(self.store.depth(c) as usize + 1);
            nodes = nodes.saturating_add(self.store.tree_size(c));
        });
        if depth > self.budget.max_depth || nodes > self.budget.max_tree_nodes {
            return Err(limit());
        }
        self.store
            .add_bounded(
                node,
                &mut self.budget.work,
                self.budget.max_store_nodes,
                &mut self.budget.allocation,
            )
            .map_err(|_| limit())
    }
    pub(crate) fn generated_name(
        &mut self,
        prefix: &str,
        index: usize,
        second: Option<usize>,
    ) -> GenerationResult<String> {
        // Two decimal usize values, a separator and prefix; reserve before formatting.
        let bytes = prefix.len() as u64 + 48;
        self.budget.spend(bytes)?;
        self.budget.reserve(bytes.saturating_mul(2) + 64)?;
        let name = match second {
            Some(second) => format!("{prefix}{index}_{second}"),
            None => format!("{prefix}{index}"),
        };
        self.payload_envelope = self.payload_envelope.max(name.len() as u64 + 64);
        Ok(name)
    }

    pub(crate) fn var(&mut self, name: &str) -> GenerationResult<TermId> {
        let n = self.symbol(name)?;
        self.add(Node::Var(n))
    }
    pub(crate) fn app(&mut self, f: TermId, a: TermId) -> GenerationResult<TermId> {
        self.add(Node::App(f, a))
    }
    pub(crate) fn pi(&mut self, name: &str, ty: TermId, body: TermId) -> GenerationResult<TermId> {
        let param = self.symbol(name)?;
        self.add(Node::Pi {
            param,
            param_type: ty,
            body_type: body,
        })
    }
    pub(crate) fn lam(&mut self, name: &str, ty: TermId, body: TermId) -> GenerationResult<TermId> {
        let param = self.symbol(name)?;
        self.add(Node::Lambda {
            param,
            param_type: ty,
            body,
        })
    }
    pub(crate) fn rename(
        &mut self,
        body: TermId,
        old: &str,
        new: &str,
    ) -> GenerationResult<TermId> {
        let value = self.var(new)?;
        let old = self.symbol(old)?;
        self.instantiate(body, &[(old, value)])
    }
    pub(crate) fn instantiate(
        &mut self,
        body: TermId,
        pairs: &[(Symbol, TermId)],
    ) -> GenerationResult<TermId> {
        let out = self
            .store
            .instantiate_bounded(
                body,
                pairs,
                &mut self.budget.work,
                self.budget.max_store_nodes,
                &mut self.budget.allocation,
            )
            .map_err(|_| limit())?;
        // Store capture avoidance may append a fresh suffix to a retained binder name.
        // Its own allocation is charged by instantiate_bounded; extraction must cover it too.
        self.payload_envelope = self.payload_envelope.saturating_add(32);
        self.admit(out)?;
        Ok(out)
    }
    pub(crate) fn rewrite(
        &mut self,
        mut term: TermId,
        tele: &[(String, TermId)],
        names: &[String],
    ) -> GenerationResult<TermId> {
        for ((old, _), new) in tele.iter().zip(names) {
            if old != "_" {
                term = self.rename(term, old, new)?;
            }
        }
        Ok(term)
    }
    pub(crate) fn peel(
        &mut self,
        mut term: TermId,
    ) -> GenerationResult<(Vec<(String, TermId)>, TermId)> {
        let mut out = Vec::new();
        while let Node::Pi {
            param,
            param_type,
            body_type,
        } = self.store.node(term)
        {
            let (param, ty, body) = (*param, *param_type, *body_type);
            self.budget.inventory(1)?;
            let name = self.store.name(param);
            self.budget.reserve(name.len() as u64 + 32)?;
            out.push((name.to_string(), ty));
            term = body;
        }
        Ok((out, term))
    }
    pub(crate) fn spine(&mut self, mut term: TermId) -> GenerationResult<(TermId, Vec<TermId>)> {
        let mut args = Vec::new();
        while let Node::App(f, a) = self.store.node(term) {
            let (f, a) = (*f, *a);
            self.budget.inventory(1)?;
            args.push(a);
            term = f;
        }
        args.reverse();
        Ok((term, args))
    }
    pub(crate) fn head_name(&self, id: TermId) -> Option<&str> {
        match self.store.node(id) {
            Node::Global(n) | Node::Const { name: n, .. } => Some(self.store.name(*n)),
            _ => None,
        }
    }
    pub(crate) fn extract(&mut self, id: TermId) -> GenerationResult<Term> {
        self.admit(id)?;
        let nodes = self.store.tree_size(id);
        self.budget.materialized_nodes = self
            .budget
            .materialized_nodes
            .checked_sub(nodes)
            .ok_or_else(limit)?;
        self.budget
            .spend(nodes.saturating_mul(self.payload_envelope.saturating_add(1)))?;
        self.budget
            .reserve(nodes.saturating_mul(self.payload_envelope.saturating_add(256)))?;
        self.store.extract_bounded(id, nodes).map_err(|_| limit())
    }
    /// Admit source/kernel-returned trees before intern copies them. Scratch pushes are charged.
    pub(crate) fn intern(&mut self, term: &Term) -> GenerationResult<TermId> {
        // `U` carries the slot of the Sort/Const node whose extraction copies that universe.
        enum Item<'a> {
            T(&'a Term, usize),
            U(&'a Universe, usize, usize),
        }
        self.budget.inventory(1)?;
        let mut todo = vec![Item::T(term, 1)];
        let mut count = 0u64;
        let mut bytes = 0u64;
        let mut universe_count = 0usize;
        let mut metadata_sets = 0u64;
        // The payload one extracted node copies: its own name or literal (`widest`), or for a
        // Sort/Const its name plus every universe it carries (`carried`). 64 per node or level.
        let mut widest = 0u64;
        let mut carried = Vec::<u64>::new();
        macro_rules! push {
            ($v:expr) => {{
                self.budget.inventory(1)?;
                todo.push($v);
            }};
        }
        while let Some(item) = todo.pop() {
            self.budget.spend(1)?;
            count = count.checked_add(1).ok_or_else(limit)?;
            if count > self.budget.max_tree_nodes {
                return Err(limit());
            }
            match item {
                Item::U(u, d, slot) => {
                    universe_count = universe_count.checked_add(1).ok_or_else(limit)?;
                    if d > self.budget.max_depth {
                        return Err(limit());
                    }
                    carried[slot] = carried[slot].saturating_add(64);
                    match u {
                        Universe::Var(n) => {
                            bytes = bytes.saturating_add(n.len() as u64);
                            carried[slot] = carried[slot].saturating_add(n.len() as u64);
                        }
                        Universe::Succ(a) => push!(Item::U(a, d + 1, slot)),
                        Universe::Max(a, b) | Universe::IMax(a, b) => {
                            push!(Item::U(a, d + 1, slot));
                            push!(Item::U(b, d + 1, slot));
                        }
                        _ => {}
                    }
                }
                Item::T(t, d) => {
                    metadata_sets = metadata_sets.saturating_add(raw_metadata_set_growth(t));
                    if d > self.budget.max_depth {
                        return Err(limit());
                    }
                    let own = match t {
                        Term::Var(n) | Term::Global(n) => {
                            bytes = bytes.saturating_add(n.len() as u64);
                            n.len() as u64
                        }
                        Term::Sort(u) => {
                            let slot = carried.len();
                            carried.push(64);
                            push!(Item::U(u, 1, slot));
                            0
                        }
                        Term::Const { name, levels } => {
                            bytes = bytes.saturating_add(name.len() as u64);
                            let slot = carried.len();
                            carried.push(name.len() as u64 + 64);
                            for u in levels {
                                push!(Item::U(u, 1, slot));
                            }
                            0
                        }
                        Term::Pi {
                            param,
                            param_type,
                            body_type,
                        } => {
                            bytes = bytes.saturating_add(param.len() as u64);
                            push!(Item::T(param_type, d + 1));
                            push!(Item::T(body_type, d + 1));
                            param.len() as u64
                        }
                        Term::Lambda {
                            param,
                            param_type,
                            body,
                        } => {
                            bytes = bytes.saturating_add(param.len() as u64);
                            push!(Item::T(param_type, d + 1));
                            push!(Item::T(body, d + 1));
                            param.len() as u64
                        }
                        Term::App(f, a) => {
                            push!(Item::T(f, d + 1));
                            push!(Item::T(a, d + 1));
                            0
                        }
                        Term::Let {
                            name,
                            ty,
                            value,
                            body,
                        } => {
                            bytes = bytes.saturating_add(name.len() as u64);
                            push!(Item::T(ty, d + 1));
                            push!(Item::T(value, d + 1));
                            push!(Item::T(body, d + 1));
                            name.len() as u64
                        }
                        Term::Match {
                            discriminant,
                            motive,
                            cases,
                        } => {
                            push!(Item::T(discriminant, d + 1));
                            push!(Item::T(motive, d + 1));
                            for c in cases {
                                push!(Item::T(c, d + 1));
                            }
                            0
                        }
                        Term::Fix { name, body } => {
                            bytes = bytes.saturating_add(name.len() as u64);
                            push!(Item::T(body, d + 1));
                            name.len() as u64
                        }
                        Term::MutualFix { defs, .. } => {
                            let mut names = 0u64;
                            for (n, b) in defs {
                                bytes = bytes.saturating_add(n.len() as u64);
                                names = names.saturating_add(n.len() as u64);
                                push!(Item::T(b, d + 1));
                            }
                            names
                        }
                        Term::Lit(literal) => {
                            let payload = match literal {
                                crate::Literal::Text(s) => s.len() as u64,
                                crate::Literal::BigInt(n) | crate::Literal::Nat(n) => {
                                    n.clone_payload_bytes() as u64
                                }
                                _ => 0,
                            };
                            bytes = bytes.saturating_add(payload);
                            payload
                        }
                        Term::Hole => 0,
                        Term::Annotated { term, .. } => {
                            push!(Item::T(term, d + 1));
                            0
                        }
                    };
                    widest = widest.max(own.saturating_add(64));
                }
            }
        }
        let count_usize = usize::try_from(count).map_err(|_| limit())?;
        if self
            .store
            .len()
            .checked_add(count_usize)
            .is_none_or(|n| n > self.budget.max_store_nodes)
        {
            return Err(limit());
        }
        if self
            .store
            .level_count()
            .checked_add(universe_count)
            .is_none_or(|n| n > self.budget.max_store_nodes)
        {
            return Err(limit());
        }
        // Ordinary intern uses compute_info for each new raw term node. Its metadata
        // IDs are a separate dense inventory from term/level IDs and allocation credits.
        self.store
            .admit_name_set_growth(metadata_sets)
            .map_err(|_| limit())?;
        self.budget
            .spend(count.saturating_mul(count).saturating_add(bytes))?;
        self.budget.reserve(
            count
                .saturating_mul(count)
                .saturating_mul(128)
                .saturating_add(bytes.saturating_mul(4)),
        )?;
        self.payload_envelope = self
            .payload_envelope
            .max(carried.iter().copied().fold(widest, u64::max));
        self.input_envelope = self
            .input_envelope
            .saturating_add(bytes)
            .saturating_add(count);
        let out = self.store.intern(term);
        self.admit(out)?;
        Ok(out)
    }
    pub(crate) fn declaration(&mut self, ctx: &Context, name: &str) -> GenerationResult<TermId> {
        // This small constructor only copies a declaration's already-admitted level names.
        if let Some((levels, _)) = ctx.poly_declaration(name) {
            self.budget.inventory(levels.len())?;
            for l in levels {
                self.budget.reserve(l.len() as u64 + 64)?;
            }
        }
        self.budget.reserve(name.len() as u64 + 64)?;
        self.intern(&super::declaration_head(ctx, name))
    }
    pub(crate) fn infer(&mut self, ctx: &Context, id: TermId) -> GenerationResult<TermId> {
        let term = self.extract(id)?;
        let ty = crate::infer_type(ctx, &term)?;
        self.intern(&ty)
    }
}

macro_rules! generated_var {
    ($dag:expr,$prefix:expr,$index:expr) => {{
        let name = $dag.generated_name($prefix, $index, None)?;
        $dag.var(&name)?
    }};
}
struct Occ {
    member: usize,
    tele: Vec<(String, TermId)>,
    indices: Vec<TermId>,
}
struct Analysis {
    types: Vec<TermId>,
    recursive: Vec<Option<Occ>>,
    indices: Vec<TermId>,
}
fn analyze(
    d: &mut Dag<'_>,
    ctx: &Context,
    ctype: TermId,
    owner: &str,
    block: &[String],
    np: usize,
    params: &[String],
) -> GenerationResult<Analysis> {
    let (tele, result) = d.peel(ctype)?;
    if tele.len() < np {
        return Err(invalid("short constructor parameter telescope"));
    }
    let (head, result_args) = d.spine(result)?;
    if d.head_name(head) != Some(owner) {
        return Err(invalid("constructor result owner mismatch"));
    }
    let (tp, vp) = tele.split_at(np);
    d.budget.inventory(vp.len())?;
    let names: Vec<_> = (0..vp.len())
        .map(|i| d.generated_name("a", i, None))
        .collect::<GenerationResult<_>>()?;
    let mut types = Vec::new();
    let mut recursive = Vec::new();
    for (j, (_, ty)) in vp.iter().enumerate() {
        d.budget.inventory(2)?;
        let t = d.rewrite(*ty, tp, params)?;
        let t = d.rewrite(t, &vp[..j], &names[..j])?;
        let tree = d.extract(t)?;
        d.budget.inventory(block.len())?;
        let refs: Vec<_> = block.iter().map(String::as_str).collect();
        let occ = if crate::positivity::mentions_inductive_block(ctx, &refs, &tree) {
            // Existing kernel normalization owns its own allocations and reduction limits.
            let reduced =
                crate::normalize_checked(ctx, &tree).map_err(|(_, reason)| match reason {
                    crate::Unreduced::BudgetSpent | crate::Unreduced::TooDeep => limit(),
                    crate::Unreduced::Cyclic => invalid("recursive-field normalization is cyclic"),
                })?;
            let reduced = d.intern(&reduced)?;
            let (ft, end) = d.peel(reduced)?;
            let (h, args) = d.spine(end)?;
            d.head_name(h)
                .and_then(|n| block.iter().position(|b| b == n))
                .map(|member| Occ {
                    member,
                    tele: ft,
                    indices: args.into_iter().skip(np).collect(),
                })
        } else {
            None
        };
        types.push(t);
        recursive.push(occ);
    }
    let mut indices = Vec::new();
    for arg in result_args.into_iter().skip(np) {
        d.budget.inventory(1)?;
        let t = d.rewrite(arg, tp, params)?;
        indices.push(d.rewrite(t, vp, &names)?);
    }
    Ok(Analysis {
        types,
        recursive,
        indices,
    })
}
fn minor(
    d: &mut Dag<'_>,
    ctx: &Context,
    owner: &str,
    motives: &[String],
    ctor: &str,
    params: &[String],
    a: &Analysis,
) -> GenerationResult<TermId> {
    let mut c = d.declaration(ctx, ctor)?;
    for p in params {
        let v = d.var(p)?;
        c = d.app(c, v)?;
    }
    for j in 0..a.types.len() {
        let v = generated_var!(d, "a", j);
        c = d.app(c, v)?;
    }
    let mut body = d.var(owner)?;
    for i in &a.indices {
        body = d.app(body, *i)?;
    }
    body = d.app(body, c)?;
    for j in (0..a.types.len()).rev() {
        if let Some(o) = &a.recursive[j] {
            let mut ih = d.var(&motives[o.member])?;
            for i in &o.indices {
                ih = d.app(ih, *i)?;
            }
            let mut value = generated_var!(d, "a", j);
            for (n, _) in &o.tele {
                let v = d.var(n)?;
                value = d.app(value, v)?;
            }
            ih = d.app(ih, value)?;
            for (n, t) in o.tele.iter().rev() {
                ih = d.pi(n, *t, ih)?;
            }
            let name = d.generated_name("ih", j, None)?;
            body = d.pi(&name, ih, body)?;
        }
    }
    for j in (0..a.types.len()).rev() {
        let name = d.generated_name("a", j, None)?;
        body = d.pi(&name, a.types[j], body)?;
    }
    Ok(body)
}
fn case(d: &mut Dag<'_>, a: &Analysis, minor: &str, recs: &[String]) -> GenerationResult<TermId> {
    let mut body = d.var(minor)?;
    for j in 0..a.types.len() {
        let v = generated_var!(d, "a", j);
        body = d.app(body, v)?;
    }
    for j in 0..a.types.len() {
        if let Some(o) = &a.recursive[j] {
            let mut call = d.var(&recs[o.member])?;
            for i in &o.indices {
                call = d.app(call, *i)?;
            }
            let mut value = generated_var!(d, "a", j);
            for (n, _) in &o.tele {
                let v = d.var(n)?;
                value = d.app(value, v)?;
            }
            call = d.app(call, value)?;
            for (n, t) in o.tele.iter().rev() {
                call = d.lam(n, *t, call)?;
            }
            body = d.app(body, call)?;
        }
    }
    for j in (0..a.types.len()).rev() {
        let name = d.generated_name("a", j, None)?;
        body = d.lam(&name, a.types[j], body)?;
    }
    Ok(body)
}
struct Member {
    indices: Vec<String>,
    types: Vec<TermId>,
    carrier: TermId,
    motive: TermId,
    cases: Vec<TermId>,
    recursive: bool,
}

/// Draft construction path; not exported/wired until the admission audit below is complete.
/// Existing unbounded APIs remain unchanged. Caller must reserve the configured kernel stack.
pub fn derive_recursor_universe_poly_bounded(
    ctx: &Context,
    ind: &str,
    budget: &mut RecursorBudget,
) -> GenerationResult<(Vec<String>, Term, Term)> {
    if budget.max_depth > crate::type_checker::MAX_TERM_DEPTH {
        return Err(limit());
    }
    let mut d = Construction::new(budget)?;
    let mutual = ctx.mutual_block_of(ind).is_some();
    let block = match ctx.mutual_block_of(ind) {
        Some(b) => {
            d.budget.inventory(b.len())?;
            for name in b {
                d.budget.reserve(name.len() as u64 + 32)?;
            }
            b.to_vec()
        }
        None => {
            d.budget.inventory(1)?;
            d.budget.reserve(ind.len() as u64 + 32)?;
            vec![ind.to_string()]
        }
    };
    if !ctx.is_inductive(ind) {
        return Err(KernelError::UnboundVariable(ind.to_string()).into());
    }
    let bi = block
        .iter()
        .position(|n| n == ind)
        .ok_or_else(|| invalid("recursor is absent from its block"))?;
    let m = block.len();
    // Admit all borrowed declarations before the fresh-level routine scans/copies names.
    for name in &block {
        if let Some((parameters, _)) = ctx.poly_declaration(name) {
            d.budget.inventory(parameters.len())?;
            for parameter in parameters {
                d.budget.reserve(parameter.len() as u64 + 64)?;
                d.input_envelope = d.input_envelope.saturating_add(parameter.len() as u64 + 1);
            }
        }
        let ty = ctx
            .get_global(name)
            .ok_or_else(|| invalid("missing inductive declaration"))?;
        d.intern(ty)?;
        for (_, ty) in ctx.get_constructors(name) {
            d.intern(ty)?;
        }
    }
    d.budget.inventory(m)?;
    let fresh_work = d.input_envelope.saturating_add(m as u64).saturating_add(1);
    d.budget.spend(fresh_work.saturating_mul(fresh_work))?;
    d.budget.reserve(fresh_work.saturating_mul(256))?;
    let levels = super::fresh_motive_levels(ctx, ind, m);
    let arity = d.intern(
        ctx.get_global(&block[0])
            .ok_or_else(|| invalid("missing block arity"))?,
    )?;
    let (first, _) = d.peel(arity)?;
    let np = ctx.inductive_num_params(&block[0]).min(first.len());
    d.budget.inventory(np)?;
    let params: Vec<_> = (0..np)
        .map(|i| d.generated_name("A", i, None))
        .collect::<GenerationResult<_>>()?;
    let mut param_types = Vec::new();
    for i in 0..np {
        d.budget.inventory(1)?;
        param_types.push(d.rewrite(first[i].1, &first[..i], &params)?);
    }
    d.budget.inventory(m.saturating_mul(3))?;
    let motives: Vec<_> = (0..m)
        .map(|i| {
            if mutual {
                d.generated_name("P", i, None)
            } else {
                d.budget.reserve(64)?;
                Ok("P".into())
            }
        })
        .collect::<GenerationResult<_>>()?;
    let recs: Vec<_> = (0..m)
        .map(|i| {
            if mutual {
                d.generated_name("rec", i, None)
            } else {
                d.budget.reserve(64)?;
                Ok("rec".into())
            }
        })
        .collect::<GenerationResult<_>>()?;
    let mut infos = Vec::new();
    let mut minors = Vec::new();
    for (k, name) in block.iter().enumerate() {
        let arity = d.intern(
            ctx.get_global(name)
                .ok_or_else(|| invalid("missing member arity"))?,
        )?;
        let (tele, sort) = d.peel(arity)?;
        if ctx.inductive_num_params(name).min(tele.len()) != np {
            return Err(invalid("mutual parameter count mismatch"));
        }
        let it = &tele[np..];
        d.budget.inventory(it.len())?;
        let indices: Vec<_> = (0..it.len())
            .map(|j| {
                if mutual {
                    d.generated_name("Idx", k, Some(j))
                } else {
                    d.generated_name("Idx", j, None)
                }
            })
            .collect::<GenerationResult<_>>()?;
        let mut types = Vec::new();
        for j in 0..it.len() {
            d.budget.inventory(1)?;
            let t = d.rewrite(it[j].1, &first[..np], &params)?;
            types.push(d.rewrite(t, &it[..j], &indices)?);
        }
        let mut carrier = d.declaration(ctx, name)?;
        for p in &params {
            let v = d.var(p)?;
            carrier = d.app(carrier, v)?;
        }
        for n in &indices {
            let v = d.var(n)?;
            carrier = d.app(carrier, v)?;
        }
        let sort = d.extract(sort)?;
        let target = super::motive_target_at(ctx, name, &sort, levels.get(k));
        let target = d.intern(&target)?;
        let mut motive = d.pi("_", carrier, target)?;
        for j in (0..indices.len()).rev() {
            motive = d.pi(&indices[j], types[j], motive)?;
        }
        let mut cases = Vec::new();
        let mut recursive = false;
        for (cname, cty) in ctx.get_constructors(name) {
            d.budget.inventory(2)?;
            let cty = d.intern(cty)?;
            let a = analyze(&mut d, ctx, cty, name, &block, np, &params)?;
            recursive |= a.recursive.iter().any(Option::is_some);
            let fi = d.generated_name("f", minors.len(), None)?;
            let mt = minor(&mut d, ctx, &motives[k], &motives, cname, &params, &a)?;
            let ct = case(&mut d, &a, &fi, &recs)?;
            minors.push((fi, mt));
            cases.push(ct);
        }
        d.budget.inventory(1)?;
        infos.push(Member {
            indices,
            types,
            carrier,
            motive,
            cases,
            recursive,
        });
    }
    let mut defs = Vec::new();
    for (k, info) in infos.iter().enumerate() {
        d.budget.inventory(info.indices.len())?;
        let names: Vec<_> = (0..info.indices.len())
            .map(|j| {
                if mutual {
                    d.generated_name("mj", k, Some(j))
                } else {
                    d.generated_name("mj", j, None)
                }
            })
            .collect::<GenerationResult<_>>()?;
        let mut result = d.var(&motives[k])?;
        for n in &names {
            let v = d.var(n)?;
            result = d.app(result, v)?;
        }
        let x = d.var("x")?;
        result = d.app(result, x)?;
        let mut carrier = d.declaration(ctx, &block[k])?;
        for p in &params {
            let v = d.var(p)?;
            carrier = d.app(carrier, v)?;
        }
        for n in &names {
            let v = d.var(n)?;
            carrier = d.app(carrier, v)?;
        }
        let mut mf = d.lam("x", carrier, result)?;
        for j in (0..names.len()).rev() {
            let mut t = info.types[j];
            for i in 0..j {
                t = d.rename(t, &info.indices[i], &names[i])?;
            }
            mf = d.lam(&names[j], t, mf)?;
        }
        d.budget.inventory(info.cases.len())?;
        let match_term = d.add(Node::Match {
            discriminant: x,
            motive: mf,
            cases: info.cases.clone().into_boxed_slice(),
        })?;
        let mut body = d.lam("x", info.carrier, match_term)?;
        for j in (0..info.indices.len()).rev() {
            body = d.lam(&info.indices[j], info.types[j], body)?;
        }
        d.budget.inventory(1)?;
        defs.push((d.symbol(&recs[k])?, body));
    }
    let mut term = if mutual {
        d.add(Node::MutualFix {
            defs: defs.into_boxed_slice(),
            index: bi,
        })?
    } else {
        let (name, body) = defs[0];
        if infos[0].recursive {
            d.add(Node::Fix { name, body })?
        } else {
            body
        }
    };
    for (n, t) in minors.iter().rev() {
        term = d.lam(n, *t, term)?;
    }
    for k in (0..m).rev() {
        term = d.lam(&motives[k], infos[k].motive, term)?;
    }
    for i in (0..np).rev() {
        term = d.lam(&params[i], param_types[i], term)?;
    }
    let ty = d.infer(ctx, term)?;
    let used = d.free_level_vars(ty)?;
    let own = if let Some((parameters, _)) = ctx.poly_declaration(ind) {
        d.budget.inventory(parameters.len())?;
        for parameter in parameters {
            d.budget.reserve(parameter.len() as u64 + 64)?;
        }
        parameters.to_vec()
    } else if let Some(arity) = ctx.get_global(ind) {
        let arity = d.intern(arity)?;
        d.free_level_vars(arity)?
    } else {
        Vec::new()
    };
    let mut retained = Vec::new();
    for level in levels {
        d.budget
            .spend((level.len() as u64 + 1).saturating_mul(used.len() as u64 + 1))?;
        if used.contains(&level) {
            d.budget.inventory(1)?;
            retained.push(level);
        }
    }
    for level in own.into_iter().chain(used) {
        d.budget
            .spend((level.len() as u64 + 1).saturating_mul(retained.len() as u64 + 1))?;
        if !retained.contains(&level) {
            d.budget.inventory(1)?;
            retained.push(level);
        }
    }
    let levels = retained;
    let ty = d.extract(ty)?;
    let term = d.extract(term)?;
    Ok((levels, ty, term))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recursor/bounded/controls.rs"]
mod controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recursor/bounded/accounting_controls.rs"]
mod accounting_controls;

#[path = "recursor_bounded_universes.rs"]
mod universes;
pub(crate) use universes::instantiate_universes_bounded;

// No traversal or allocation: upper bound compute_info's new sets for this one raw node.
// Each child has at most three unions (free/binder/constant); each scoped child may
// remove each binder; then this node inserts its binders or its own variable/global.
fn raw_metadata_set_growth(term: &Term) -> u64 {
    let (children, bound, scoped, own): (u64, u64, u64, u64) = match term {
        Term::Var(_) | Term::Global(_) | Term::Const { .. } => (0, 0, 0, 1),
        Term::Pi { .. } | Term::Lambda { .. } => (2, 1, 1, 0),
        Term::App(_, _) => (2, 0, 0, 0),
        Term::Let { .. } => (3, 1, 1, 0),
        Term::Match { cases, .. } => ((cases.len() as u64).saturating_add(2), 0, 0, 0),
        Term::Fix { .. } => (1, 1, 1, 0),
        Term::MutualFix { defs, .. } => {
            let n = defs.len() as u64;
            (n, n, n, 0)
        }
        Term::Sort(_) | Term::Lit(_) | Term::Hole => (0, 0, 0, 0),
        Term::Annotated { .. } => (1, 0, 0, 0),
    };
    children
        .saturating_mul(3)
        .saturating_add(bound.saturating_mul(scoped))
        .saturating_add(bound)
        .saturating_add(own)
}
#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recursor/bounded/raw_intern_accounting_controls.rs"]
mod raw_intern_accounting_controls;
