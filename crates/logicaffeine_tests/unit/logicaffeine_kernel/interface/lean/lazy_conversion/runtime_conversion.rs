//! Source/native closure evaluation with independently replayed compositional rules.
//!
//! Original typing belongs to TypedSource. Native links are created only by the
//! both-kernel bridge; replay checks how the original source reaches those links.
//! No type-only value merging and no expanded captured-source readback occurs.
use super::native_conversion::{self, NativeCertificates};
use super::*;
use crate::term_store::{Node, TermId, TermStore};
use logicaffeine_base::Symbol;
use std::rc::Rc;

fn context_error(error: Verdict, context: String) -> Verdict {
    error.located(context)
}

type ValueId = usize;
type EnvId = usize;
type ProofId = usize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Syntax {
    Source(usize),
    Native(TermId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Captured {
    node: Syntax,
    env: EnvId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RuntimeValue {
    Ground(TermId),
    Lambda(Captured),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Goal {
    Eval(Captured),
    Call(usize),
    UnaryCall {
        function: ValueId,
        argument: ValueId,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Call {
    function: ValueId,
    arguments: Vec<ValueId>,
}
#[derive(Clone, Copy)]
struct RawBinding {
    parent: EnvId,
    value: ValueId,
    name: Option<Symbol>,
}

// A unary source callback needs no owned argument vector, call-table entry or
// separate interning record. Its exact fixed-size Goal is already the key in
// done/active. Replay sees the same ordered Call through this borrowed view.
#[derive(Clone, Copy)]
enum CallArguments<'a> {
    Borrowed(&'a [ValueId]),
    Unary([ValueId; 1]),
    Evidence(&'a [OperandEvidence]),
}
impl<'a> CallArguments<'a> {
    fn len(&self) -> usize {
        match self {
            Self::Borrowed(v) => v.len(),
            Self::Unary(_) => 1,
            Self::Evidence(v) => v.len(),
        }
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, index: usize) -> Option<&ValueId> {
        match self {
            Self::Borrowed(v) => v.get(index),
            Self::Unary(v) => v.get(index),
            Self::Evidence(v) => v.get(index).map(OperandEvidence::value_ref),
        }
    }
    fn first(&self) -> Option<&ValueId> {
        self.get(0)
    }
    fn iter(&self) -> impl DoubleEndedIterator<Item = &ValueId> + ExactSizeIterator {
        (0..self.len()).map(|i| &self[i])
    }
    fn suffix(self, start: usize) -> Self {
        match self {
            Self::Borrowed(v) => Self::Borrowed(&v[start..]),
            Self::Evidence(v) => Self::Evidence(&v[start..]),
            Self::Unary(v) => {
                assert!(start <= 1);
                if start == 0 {
                    Self::Unary(v)
                } else {
                    Self::Borrowed(&[])
                }
            }
        }
    }
}
impl std::ops::Index<usize> for CallArguments<'_> {
    type Output = ValueId;
    fn index(&self, index: usize) -> &ValueId {
        self.get(index).expect("admitted call argument index")
    }
}
#[derive(Clone, Copy)]
struct CallView<'a> {
    function: ValueId,
    arguments: CallArguments<'a>,
}
#[derive(Clone, Copy)]
struct Jump {
    end: EnvId,
    extra: usize,
}
struct BindingIndex {
    run: usize,
    jumps: Rc<Vec<Jump>>,
}
struct Environments {
    raw: Vec<Option<RawBinding>>,
    index: Vec<Option<BindingIndex>>,
    ids: HashMap<(EnvId, ValueId, Option<Symbol>), EnvId>,
}
impl Environments {
    fn new() -> Self {
        Self {
            raw: vec![None],
            index: vec![None],
            ids: HashMap::new(),
        }
    }
    // This is storage metadata, not conversion evidence. Replay rebuilds it
    // from the raw parent/value edges instead of trusting producer jump tables.
    fn append(&mut self, b: RawBinding, budget: &mut usize) -> Result<EnvId> {
        if b.parent >= self.raw.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime forward environment",
            ));
        }
        spend(budget, 1)?;
        let same = self.raw[b.parent].is_some_and(|p| p.name == b.name && p.value == b.value);
        let index = if same {
            let parent = self.index[b.parent].as_ref().unwrap();
            BindingIndex {
                run: parent.run.checked_add(1).ok_or_else(|| {
                    declined(
                        DiagnosticCode::ResourceLimit,
                        "runtime binding length overflow",
                    )
                })?,
                jumps: Rc::clone(&parent.jumps),
            }
        } else {
            let cap = self.index[b.parent]
                .as_ref()
                .map_or(1, |p| p.jumps.len() + 1);
            spend(budget, cap + 1)?;
            let mut jumps = Vec::with_capacity(cap);
            jumps.push(Jump {
                end: b.parent,
                extra: 0,
            });
            for level in 0..cap - 1 {
                let previous = jumps[level];
                let Some(mid) = self.index[previous.end].as_ref() else {
                    break;
                };
                let Some(next) = mid.jumps.get(level) else {
                    break;
                };
                let extra = previous
                    .extra
                    .checked_add(mid.run)
                    .and_then(|n| n.checked_add(next.extra))
                    .ok_or_else(|| {
                        declined(
                            DiagnosticCode::ResourceLimit,
                            "runtime jump distance overflow",
                        )
                    })?;
                jumps.push(Jump {
                    end: next.end,
                    extra,
                });
            }
            BindingIndex {
                run: 1,
                jumps: Rc::new(jumps),
            }
        };
        let id = self.raw.len();
        self.raw.push(Some(b));
        self.index.push(Some(index));
        Ok(id)
    }
    fn extend(
        &mut self,
        parent: EnvId,
        value: ValueId,
        name: Option<Symbol>,
        budget: &mut usize,
    ) -> Result<EnvId> {
        spend(budget, 1)?;
        if let Some(id) = self.ids.get(&(parent, value, name)) {
            return Ok(*id);
        }
        let id = self.append(
            RawBinding {
                parent,
                value,
                name,
            },
            budget,
        )?;
        self.ids.insert((parent, value, name), id);
        Ok(id)
    }
    fn source(&self, mut env: EnvId, mut offset: usize, budget: &mut usize) -> Result<ValueId> {
        spend(budget, 1)?;
        let original = self
            .index
            .get(env)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "unbound runtime source variable",
                )
            })?;
        if offset < original.run {
            return Ok(self.raw[env].unwrap().value);
        }
        for level in (0..original.jumps.len()).rev() {
            spend(budget, 1)?;
            let Some(current) = self.index.get(env).and_then(Option::as_ref) else {
                break;
            };
            let Some(jump) = current.jumps.get(level) else {
                continue;
            };
            let distance = current.run.checked_add(jump.extra).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime lookup distance overflow",
                )
            })?;
            if offset >= distance {
                offset -= distance;
                env = jump.end;
                if let Some(next) = self.index.get(env).and_then(Option::as_ref) {
                    if offset < next.run {
                        return Ok(self.raw[env].unwrap().value);
                    }
                }
            }
        }
        Err(rejected(
            DiagnosticCode::InvalidProof,
            "unbound runtime source variable",
        ))
    }
    fn named(&self, mut env: EnvId, name: Symbol, budget: &mut usize) -> Result<ValueId> {
        while env != 0 {
            spend(budget, 1)?;
            let b = self.raw.get(env).copied().flatten().ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime native environment index",
                )
            })?;
            if b.name == Some(name) {
                return Ok(b.value);
            }
            env = b.parent;
        }
        Err(rejected(
            DiagnosticCode::InvalidProof,
            "unbound runtime native variable",
        ))
    }
    fn verifies(&self, env: EnvId, expected: RawBinding) -> bool {
        self.raw.get(env).copied().flatten().is_some_and(|b| {
            b.parent == expected.parent && b.value == expected.value && b.name == expected.name
        })
    }
}

#[derive(Clone, Copy)]
enum LinkKind {
    Expose,
    Ground { partial: bool },
}
// Producer proposals record the exact endpoints checked during search. Replay
// does not trust these fields: it independently certifies every used link once
// against the immutable original Context before accepting that rule.
struct NativeLink {
    kind: LinkKind,
    from: TermId,
    to: TermId,
}
struct TemplateRecord {
    body: TermId,
    checked: runtime_template::CheckedTemplate,
    grounds: Vec<TermId>,
}
#[derive(Clone, Copy)]
enum OperandEvidence {
    Lambda(ValueId),
    Atom(ValueId),
    Variable(ValueId),
    Proof { proof: ProofId, value: ValueId },
}
impl OperandEvidence {
    fn value(self) -> ValueId {
        *self.value_ref()
    }
    fn value_ref(&self) -> &ValueId {
        match self {
            Self::Lambda(v) | Self::Atom(v) | Self::Variable(v) | Self::Proof { value: v, .. } => v,
        }
    }
}
#[derive(Clone)]
enum TemplateOrigin {
    ClosedLambda,
    Exposed {
        link: usize,
    },
    Source {
        call: usize,
        head: OperandEvidence,
        arguments: Vec<OperandEvidence>,
        expose: Option<usize>,
    },
    // The primary judgment is the source lambda Call. Its exact beta body is
    // the source application, so no separate Eval(body) record is required.
    SourceBeta {
        source: Captured,
        head: OperandEvidence,
        arguments: Vec<OperandEvidence>,
        expose: Option<usize>,
    },
}
struct TemplateInvocation {
    goal: Goal,
    template: usize,
    origin: TemplateOrigin,
    instructions: Vec<ProofId>,
}
struct Pool<'a, 'source, 'input> {
    source: &'a TypedSource<'source, 'input>,
    bridge: NativeCertificates<'a>,
    store: TermStore,
    costs: HashMap<TermId, usize>,
    atoms: HashMap<usize, TermId>,
    exposes: HashMap<TermId, usize>,
    grounds: HashMap<TermId, usize>,
    links: Vec<NativeLink>,
    templates: Vec<TemplateRecord>,
    template_ids: HashMap<TermId, Option<usize>>,
}
impl<'a, 'source, 'input> Pool<'a, 'source, 'input> {
    fn new(source: &'a TypedSource<'source, 'input>, budget: &mut usize) -> Result<Self> {
        Ok(Self {
            source,
            bridge: NativeCertificates::new(&source.importer.ctx, budget)?,
            store: TermStore::new(),
            costs: HashMap::new(),
            atoms: HashMap::new(),
            exposes: HashMap::new(),
            grounds: HashMap::new(),
            links: Vec::new(),
            templates: Vec::new(),
            template_ids: HashMap::new(),
        })
    }
    fn intern(&mut self, term: &Term, budget: &mut usize) -> Result<TermId> {
        let cost = native_conversion::admit(term, None, budget)?;
        // Every float path is outside this initial fragment: hash-consing a
        // literal must not silently turn NaN or signed-zero identity into a
        // conversion claim. Other literal families use their exact native link.
        let mut pending = vec![term];
        while let Some(t) = pending.pop() {
            spend(budget, 1)?;
            match t {
                Term::Lit(crate::Literal::Float(_)) => {
                    return Err(declined(
                        DiagnosticCode::UnsupportedFeature,
                        "runtime float fragment unavailable",
                    ))
                }
                Term::App(a, b) => {
                    pending.push(a);
                    pending.push(b);
                }
                Term::Pi {
                    param_type,
                    body_type,
                    ..
                }
                | Term::Lambda {
                    param_type,
                    body: body_type,
                    ..
                } => {
                    pending.push(param_type);
                    pending.push(body_type);
                }
                Term::Let {
                    ty, value, body, ..
                } => {
                    pending.push(ty);
                    pending.push(value);
                    pending.push(body);
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    pending.push(discriminant);
                    pending.push(motive);
                    pending.extend(cases);
                }
                Term::Fix { body, .. } => pending.push(body),
                _ => {}
            }
        }
        spend(budget, cost)?; // native store ownership before intern clones payload
        let root = self.store.intern(term);
        let mut work = vec![root];
        let mut seen = HashSet::new();
        while let Some(id) = work.pop() {
            spend(budget, 1)?;
            if !seen.insert(id) {
                continue;
            }
            // Root cost is a conservative bound for every original subterm.
            self.costs
                .entry(id)
                .and_modify(|n| *n = (*n).min(cost))
                .or_insert(cost);
            self.store
                .node(id)
                .for_each_child(&mut |child| work.push(child));
        }
        Ok(root)
    }
    fn handle(&self, id: TermId) -> Result<()> {
        if id.index() >= self.store.len() || !self.costs.contains_key(&id) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime foreign or unadmitted native handle",
            ));
        }
        Ok(())
    }
    fn copy(&self, id: TermId, budget: &mut usize) -> Result<Term> {
        self.handle(id)?;
        if self.store.tree_size(id) > 256 || self.store.depth(id) >= 128 {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "runtime native materialization bound",
            ));
        }
        let cost = self.copy_cost(id, budget)?;
        spend(budget, cost)?;
        self.store.extract_bounded(id, 256).map_err(|_| {
            declined(
                DiagnosticCode::ResourceLimit,
                "runtime native extraction bound",
            )
        })
    }
    /// Recompute expanded ownership from borrowed store data. Producer side
    /// tables are hints, never authority to discount a clone's payload/work.
    fn copy_cost(&self, root: TermId, budget: &mut usize) -> Result<usize> {
        use crate::term_store::{LevelId, LevelView};
        enum Frame {
            Term(TermId),
            Level(LevelId, usize),
        }
        fn payload(bytes: usize, copied: &mut usize, budget: &mut usize) -> Result<()> {
            if bytes > 4096 {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime native copy payload bound",
                ));
            }
            spend(budget, bytes)?;
            *copied = copied.checked_add(bytes).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime native copy cost overflow",
                )
            })?;
            Ok(())
        }
        self.handle(root)?;
        if !self.store.float_free(root) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "runtime float fragment unavailable",
            ));
        }
        spend(budget, 1)?;
        let mut work = vec![Frame::Term(root)];
        let mut copied = 0usize;
        let mut nodes = 0usize;
        let mut levels = 0usize;
        while let Some(frame) = work.pop() {
            copied = copied.checked_add(1).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime native copy cost overflow",
                )
            })?;
            match frame {
                Frame::Term(id) => {
                    nodes += 1;
                    if nodes > 256 {
                        return Err(declined(
                            DiagnosticCode::ResourceLimit,
                            "runtime native copy term bound",
                        ));
                    }
                    let node = self.store.node(id);
                    match node {
                        Node::Var(name) | Node::Global(name) => {
                            payload(self.store.name(*name).len(), &mut copied, budget)?
                        }
                        Node::Pi { param, .. } | Node::Lambda { param, .. } => {
                            payload(self.store.name(*param).len(), &mut copied, budget)?
                        }
                        Node::Let { name, .. } | Node::Fix { name, .. } => {
                            payload(self.store.name(*name).len(), &mut copied, budget)?
                        }
                        Node::Const {
                            name,
                            levels: arguments,
                        } => {
                            payload(self.store.name(*name).len(), &mut copied, budget)?;
                            if arguments.len() > 256 {
                                return Err(declined(
                                    DiagnosticCode::ResourceLimit,
                                    "runtime native copy universe arity",
                                ));
                            }
                            spend(budget, arguments.len())?;
                            work.extend(arguments.iter().map(|level| Frame::Level(*level, 1)));
                        }
                        Node::Sort(level) => {
                            spend(budget, 1)?;
                            work.push(Frame::Level(*level, 1));
                        }
                        Node::Lit(literal) => payload(
                            self.store.literal_payload_bytes(*literal),
                            &mut copied,
                            budget,
                        )?,
                        Node::Hole | Node::Bound(_) | Node::MutualFix { .. } => {
                            return Err(declined(
                                DiagnosticCode::UnsupportedFeature,
                                "runtime native copy syntax unavailable",
                            ))
                        }
                        Node::App(..) | Node::Match { .. } => {}
                    }
                    spend(budget, node.child_count())?;
                    node.for_each_child(&mut |child| work.push(Frame::Term(child)));
                }
                Frame::Level(id, depth) => {
                    levels += 1;
                    if levels > 256 || depth >= 128 {
                        return Err(declined(
                            DiagnosticCode::ResourceLimit,
                            "runtime native copy universe bound",
                        ));
                    }
                    match self.store.level_view(id) {
                        LevelView::Var(name) => {
                            if name.starts_with('?') {
                                return Err(declined(
                                    DiagnosticCode::UnsupportedFeature,
                                    "runtime unresolved native universe",
                                ));
                            }
                            payload(name.len(), &mut copied, budget)?;
                        }
                        LevelView::Succ(a) => {
                            spend(budget, 1)?;
                            work.push(Frame::Level(a, depth + 1));
                        }
                        LevelView::Max(a, b) | LevelView::IMax(a, b) => {
                            spend(budget, 2)?;
                            work.push(Frame::Level(a, depth + 1));
                            work.push(Frame::Level(b, depth + 1));
                        }
                        LevelView::SProp | LevelView::Prop | LevelView::Type(_) => {}
                    }
                }
            }
        }
        Ok(copied)
    }
    fn atom(&mut self, source: usize, budget: &mut usize) -> Result<TermId> {
        if let Some(id) = self.atoms.get(&source) {
            return Ok(*id);
        }
        let term = self
            .source
            .typing
            .closed_readback(source, budget)
            .map_err(|e| context_error(e, format!("runtime source atom {source}")))?;
        let id = self
            .intern(&term, budget)
            .map_err(|e| context_error(e, format!("runtime source atom admission {source}")))?;
        self.atoms.insert(source, id);
        Ok(id)
    }
    fn app(&mut self, function: TermId, argument: TermId, budget: &mut usize) -> Result<TermId> {
        let size = self
            .store
            .tree_size(function)
            .checked_add(self.store.tree_size(argument))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime native application size",
                )
            })?;
        if size > 256 || self.store.depth(function).max(self.store.depth(argument)) + 1 >= 128 {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "runtime native application bound",
            ));
        }
        let cost = self.costs[&function]
            .checked_add(self.costs[&argument])
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime native copy allowance overflow",
                )
            })?;
        spend(budget, 1)?;
        let id = self.store.mk_app(function, argument);
        self.costs
            .entry(id)
            .and_modify(|n| *n = (*n).min(cost))
            .or_insert(cost);
        Ok(id)
    }
    fn expose(&mut self, id: TermId, budget: &mut usize) -> Result<usize> {
        if let Some(link) = self.exposes.get(&id) {
            return Ok(*link);
        }
        let instance = self.copy(id, budget)?;
        let body = self.bridge.expose(&instance, budget).map_err(|e| match e {
            Verdict::Reject(s) => declined(
                DiagnosticCode::UnsupportedFeature,
                format!("runtime native exposure unavailable for {instance:?}: {s}"),
            ),
            other => context_error(other, format!("runtime expose {instance:?}")),
        })?;
        let to = self.intern(&body, budget).map_err(|e| {
            context_error(
                e,
                format!("runtime exposed body admission for {instance:?}"),
            )
        })?;
        spend(budget, 1)?;
        let link = self.links.len();
        self.links.push(NativeLink {
            kind: LinkKind::Expose,
            from: id,
            to,
        });
        self.exposes.insert(id, link);
        Ok(link)
    }
    fn ground(&mut self, id: TermId, allow_partial: bool, budget: &mut usize) -> Result<usize> {
        if let Some(&link) = self.grounds.get(&id) {
            if !allow_partial && matches!(self.links[link].kind, LinkKind::Ground { partial: true })
            {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "runtime root remains function-valued",
                ));
            }
            return Ok(link);
        }
        let term = self.copy(id, budget)?;
        // Keep a well-typed partial application as compact native syntax.
        // Normalizing underneath it can enter a recursor before later arguments
        // arrive. Its identity link is checked rather than inferred from a name.
        spend(budget, 1)?;
        let ty = lean_type(&self.source.importer.ctx, &term)
            .map_err(|e| context_error(e, format!("runtime ground type {term:?}")))?;
        let partial = matches!(normal(&self.source.importer.ctx, &ty)?, Term::Pi { .. });
        if partial && !allow_partial {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "runtime root remains function-valued",
            ));
        }
        let to = if partial {
            self.bridge.conversion(&term, &term, &ty, budget)?;
            id
        } else {
            match self.bridge.normalize_ground(&term, budget) {
                Ok(answer) => self.intern(&answer, budget).map_err(|e| {
                    context_error(e, format!("runtime ground result admission for {term:?}"))
                })?,
                Err(error @ Verdict::Decline(_)) => {
                    // A closed record can contain large function bodies. Keep its
                    // exact syntax for a later complete application when eager
                    // normalization exceeds admission, with a checked identity
                    // conversion and no refund of the failed normalization work.
                    if self.bridge.conversion(&term, &term, &ty, budget).is_err() {
                        return Err(context_error(
                            error,
                            format!("runtime native ground call {term:?}"),
                        ));
                    }
                    id
                }
                Err(error) => {
                    return Err(context_error(
                        error,
                        format!("runtime native ground call {term:?}"),
                    ))
                }
            }
        };
        spend(budget, 1)?;
        let link = self.links.len();
        self.links.push(NativeLink {
            kind: LinkKind::Ground { partial },
            from: id,
            to,
        });
        self.grounds.insert(id, link);
        Ok(link)
    }
    fn template(&mut self, body: TermId, budget: &mut usize) -> Result<Option<usize>> {
        spend(budget, 1)?;
        if let Some(cached) = self.template_ids.get(&body) {
            return Ok(*cached);
        }
        self.handle(body)?;
        if !self.store.free_names_are_empty(body)
            || !matches!(self.store.node(body), Node::Lambda { .. })
        {
            return Ok(None);
        }
        let original = self.copy(body, budget)?;
        let candidate = (|| -> Result<Option<TemplateRecord>> {
            let Some(checked) =
                runtime_template::compile(&self.source.importer.ctx, &original, budget)?
            else {
                return Ok(None);
            };
            spend(budget, checked.program().grounds.len())?;
            let mut grounds = Vec::with_capacity(checked.program().grounds.len());
            for ground in &checked.program().grounds {
                grounds.push(self.intern(ground, budget)?);
            }
            Ok(Some(TemplateRecord {
                body,
                checked,
                grounds,
            }))
        })();
        let record = match candidate {
            Ok(record) => record,
            // Template construction is optional. A candidate failure proves
            // nothing against the already typed original; keep spent work.
            Err(error) => {
                if *budget == 0 {
                    return Err(error);
                }
                None
            }
        };
        spend(budget, 1)?;
        let result = record.map(|record| {
            let id = self.templates.len();
            self.templates.push(record);
            id
        });
        self.template_ids.insert(body, result);
        Ok(result)
    }
    fn captured(&self, node: Syntax, env: EnvId) -> Captured {
        let closed = match node {
            Syntax::Source(id) => self.source.importer.exprs.get(&id).is_some_and(|r| {
                r.get("sort").is_some() || r.get("const").is_some() || r.get("natVal").is_some()
            }),
            Syntax::Native(id) => self.store.free_names_are_empty(id),
        };
        Captured {
            node,
            env: if closed { 0 } else { env },
        }
    }
    fn pair(&self, node: Syntax) -> Result<Option<(Syntax, Syntax)>> {
        Ok(match node {
            Syntax::Source(id) => self
                .source
                .row(id)?
                .get("app")
                .map(|a| {
                    Ok::<_, Verdict>((
                        Syntax::Source(index(&a["fn"])?),
                        Syntax::Source(index(&a["arg"])?),
                    ))
                })
                .transpose()?,
            Syntax::Native(id) => match self.store.node(id) {
                Node::App(f, a) => Some((Syntax::Native(*f), Syntax::Native(*a))),
                _ => None,
            },
        })
    }
    fn binder(&self, captured: Captured) -> Result<Option<(Option<Symbol>, Syntax)>> {
        Ok(match captured.node {
            Syntax::Source(id) => self
                .source
                .row(id)?
                .get("lam")
                .map(|l| index(&l["body"]).map(|body| (None, Syntax::Source(body))))
                .transpose()?,
            Syntax::Native(id) => match self.store.node(id) {
                Node::Lambda { param, body, .. } => Some((Some(*param), Syntax::Native(*body))),
                _ => None,
            },
        })
    }
    fn binding(&self, captured: Captured) -> Result<Option<(Option<Symbol>, Syntax, Syntax)>> {
        Ok(match captured.node {
            Syntax::Source(id) => self
                .source
                .row(id)?
                .get("letE")
                .map(|l| {
                    Ok::<_, Verdict>((
                        None,
                        Syntax::Source(index(&l["value"])?),
                        Syntax::Source(index(&l["body"])?),
                    ))
                })
                .transpose()?,
            Syntax::Native(id) => match self.store.node(id) {
                Node::Let {
                    name, value, body, ..
                } => Some((Some(*name), Syntax::Native(*value), Syntax::Native(*body))),
                _ => None,
            },
        })
    }
    fn variable(
        &self,
        captured: Captured,
        envs: &Environments,
        budget: &mut usize,
    ) -> Result<Option<ValueId>> {
        match captured.node {
            Syntax::Source(id) => self
                .source
                .row(id)?
                .get("bvar")
                .map(|n| envs.source(captured.env, index(n)?, budget))
                .transpose(),
            Syntax::Native(id) => match self.store.node(id) {
                Node::Var(name) => envs.named(captured.env, *name, budget).map(Some),
                _ => Ok(None),
            },
        }
    }
    fn metadata(&self, captured: Captured) -> Result<Option<Syntax>> {
        match captured.node {
            Syntax::Source(id) => self
                .source
                .row(id)?
                .get("mdata")
                .map(|m| index(&m["expr"]).map(Syntax::Source))
                .transpose(),
            _ => Ok(None),
        }
    }
}

#[derive(Clone)]
enum Rule {
    Atom(TermId),
    Lambda,
    Variable,
    Metadata(ProofId),
    Let {
        value: ProofId,
        body: ProofId,
    },
    Spine {
        head: ProofId,
        arguments: Vec<ProofId>,
        call: ProofId,
    },
    Beta {
        body: ProofId,
        tail: Option<ProofId>,
    },
    Expose {
        link: usize,
        body: ProofId,
        call: ProofId,
    },
    Ground(usize),
    SourceGround {
        source: Captured,
        head: OperandEvidence,
        arguments: Vec<OperandEvidence>,
        call: usize,
        link: usize,
    },
    SourceApply {
        source: Captured,
        head: OperandEvidence,
        arguments: Vec<OperandEvidence>,
        call: Goal,
        premise: ProofId,
    },
    Reassociate(ProofId),
    Template {
        template: usize,
        origin: TemplateOrigin,
        instructions: Vec<ProofId>,
        tail: Option<ProofId>,
    },
}
struct Proof {
    goal: Goal,
    value: ValueId,
    rule: Rule,
}
struct Search<'a, 'source, 'input> {
    pool: Pool<'a, 'source, 'input>,
    envs: Environments,
    values: Vec<RuntimeValue>,
    value_ids: HashMap<RuntimeValue, ValueId>,
    calls: Vec<Call>,
    call_ids: HashMap<Call, usize>,
    proofs: Vec<Proof>,
    done: HashMap<Goal, ProofId>,
    active: HashSet<Goal>,
}
enum Frame {
    Visit(Goal),
    Metadata {
        goal: Goal,
        child: Goal,
    },
    LetValue {
        goal: Goal,
        value: Goal,
        body: Syntax,
        name: Option<Symbol>,
        env: EnvId,
    },
    LetBody {
        goal: Goal,
        value: ProofId,
        body: Goal,
    },
    Spine {
        goal: Goal,
        head: Goal,
        arguments: Vec<Goal>,
    },
    SpineCall {
        goal: Goal,
        head: ProofId,
        arguments: Vec<ProofId>,
        call: Goal,
    },
    SourceSpine {
        goal: Goal,
        source: Captured,
        head: Goal,
        arguments: Vec<Goal>,
    },
    SourceCall {
        goal: Goal,
        source: Captured,
        head: OperandEvidence,
        arguments: Vec<OperandEvidence>,
        call: Goal,
    },
    BetaBody {
        goal: Goal,
        body: Goal,
        remaining: Vec<ValueId>,
    },
    BetaTail {
        goal: Goal,
        body: ProofId,
        tail: Goal,
    },
    ExposeBody {
        goal: Goal,
        link: usize,
        body: Goal,
        arguments: Vec<ValueId>,
    },
    ExposeCall {
        goal: Goal,
        link: usize,
        body: ProofId,
        call: Goal,
    },
    Reassociate {
        goal: Goal,
        call: Goal,
    },
    TemplateStep(TemplateInvocation),
    TemplateResume {
        invocation: TemplateInvocation,
        child: Goal,
    },
    TemplateTail {
        invocation: TemplateInvocation,
        tail: Goal,
    },
}
impl<'a, 'source, 'input> Search<'a, 'source, 'input> {
    fn new(source: &'a TypedSource<'source, 'input>, budget: &mut usize) -> Result<Self> {
        Ok(Self {
            pool: Pool::new(source, budget)?,
            envs: Environments::new(),
            values: Vec::new(),
            value_ids: HashMap::new(),
            calls: Vec::new(),
            call_ids: HashMap::new(),
            proofs: Vec::new(),
            done: HashMap::new(),
            active: HashSet::new(),
        })
    }
    fn value(&mut self, value: RuntimeValue, budget: &mut usize) -> Result<ValueId> {
        if let Some(id) = self.value_ids.get(&value) {
            return Ok(*id);
        }
        spend(budget, 1)?;
        let id = self.values.len();
        self.values.push(value);
        self.value_ids.insert(value, id);
        Ok(id)
    }
    fn call(
        &mut self,
        function: ValueId,
        arguments: Vec<ValueId>,
        budget: &mut usize,
    ) -> Result<Goal> {
        if arguments.len() > 256 {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "runtime application spine bound",
            ));
        }
        if arguments.len() == 1
            && matches!(
                self.values.get(function),
                Some(RuntimeValue::Lambda(Captured {
                    node: Syntax::Source(_),
                    ..
                }))
            )
        {
            spend(budget, 1)?;
            return Ok(Goal::UnaryCall {
                function,
                argument: arguments[0],
            });
        }
        spend(budget, arguments.len() + 1)?;
        let call = Call {
            function,
            arguments,
        };
        if let Some(id) = self.call_ids.get(&call) {
            return Ok(Goal::Call(*id));
        }
        spend(budget, call.arguments.len() + 1)?;
        let id = self.calls.len();
        self.calls.push(call.clone());
        self.call_ids.insert(call, id);
        Ok(Goal::Call(id))
    }
    fn call_view(&self, goal: Goal) -> Result<CallView<'_>> {
        match goal {
            Goal::Call(id) => self
                .calls
                .get(id)
                .map(|c| CallView {
                    function: c.function,
                    arguments: CallArguments::Borrowed(&c.arguments),
                })
                .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "runtime call index")),
            Goal::UnaryCall { function, argument } => Ok(CallView {
                function,
                arguments: CallArguments::Unary([argument]),
            }),
            Goal::Eval(_) => Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime call premise shape",
            )),
        }
    }
    fn emit(&mut self, goal: Goal, value: ValueId, rule: Rule, budget: &mut usize) -> Result<()> {
        let projection = if matches!(goal, Goal::Eval(_)) {
            match &rule {
                Rule::Template {
                    origin: TemplateOrigin::Source { call, .. },
                    ..
                }
                | Rule::SourceGround { call, .. } => Some(Goal::Call(*call)),
                Rule::SourceApply { call, .. } => Some(*call),
                _ => None,
            }
        } else {
            None
        };
        // A SourceBeta proof has only the enclosing source Call judgment.
        // Its inner native arguments borrow original OperandEvidence and have
        // no independent stored Call, projection, or active-table entry.
        spend(budget, 1)?;
        let id = self.proofs.len();
        self.proofs.push(Proof { goal, value, rule });
        self.done.insert(goal, id);
        self.active.remove(&goal);
        if let Some(call) = projection {
            spend(budget, 1)?;
            self.done.insert(call, id);
            self.active.remove(&call);
        }
        Ok(())
    }
    fn push(work: &mut Vec<Frame>, frame: Frame, budget: &mut usize) -> Result<()> {
        spend(budget, 1)?;
        work.push(frame);
        Ok(())
    }
    fn invocation_view<'s>(&'s self, invocation: &'s TemplateInvocation) -> Result<CallView<'s>> {
        match &invocation.origin {
            TemplateOrigin::SourceBeta {
                head, arguments, ..
            } => Ok(CallView {
                function: head.value(),
                arguments: CallArguments::Evidence(arguments),
            }),
            TemplateOrigin::Source { call, .. } => self.call_view(Goal::Call(*call)),
            _ => self.call_view(invocation.goal),
        }
    }
    fn immediate_source(&self, c: Captured, budget: &mut usize) -> Result<bool> {
        spend(budget, 1)?;
        let Syntax::Source(id) = c.node else {
            return Ok(false);
        };
        let row = self.pool.source.row(id)?;
        Ok(row.get("lam").is_some()
            || row.get("bvar").is_some()
            || row.get("const").is_some()
            || row.get("sort").is_some()
            || row.get("natVal").is_some())
    }
    fn source_operand(&mut self, c: Captured, budget: &mut usize) -> Result<OperandEvidence> {
        spend(budget, 1)?;
        let Syntax::Source(id) = c.node else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime immediate operand is not source syntax",
            ));
        };
        let row = self.pool.source.row(id)?;
        if row.get("lam").is_some() {
            self.pool.binder(c)?.ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime immediate lambda syntax",
                )
            })?;
            return self
                .value(RuntimeValue::Lambda(c), budget)
                .map(OperandEvidence::Lambda);
        }
        if row.get("bvar").is_some() {
            return self
                .pool
                .variable(c, &self.envs, budget)?
                .map(OperandEvidence::Variable)
                .ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime immediate variable syntax",
                    )
                });
        }
        if row.get("const").is_some() || row.get("sort").is_some() || row.get("natVal").is_some() {
            let native = self.pool.atom(id, budget)?;
            return self
                .value(RuntimeValue::Ground(native), budget)
                .map(OperandEvidence::Atom);
        }
        let proof = *self.done.get(&Goal::Eval(c)).ok_or_else(|| {
            rejected(
                DiagnosticCode::InvalidProof,
                "runtime source operand is not evaluated",
            )
        })?;
        Ok(OperandEvidence::Proof {
            proof,
            value: self.proofs[proof].value,
        })
    }
    fn available_source(&self, c: Captured, budget: &mut usize) -> Result<bool> {
        Ok(self.done.contains_key(&Goal::Eval(c)) || self.immediate_source(c, budget)?)
    }
    fn schedule_source(
        &mut self,
        work: &mut Vec<Frame>,
        goal: Goal,
        c: Captured,
        budget: &mut usize,
    ) -> Result<()> {
        let mut head = c.node;
        let mut arguments = Vec::new();
        while let Some((f, a)) = self.pool.pair(head)? {
            spend(budget, 1)?;
            if arguments.len() >= 256 {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime source spine bound",
                ));
            }
            arguments.push(Goal::Eval(self.pool.captured(a, c.env)));
            head = f;
        }
        arguments.reverse();
        let head = Goal::Eval(self.pool.captured(head, c.env));
        if self.source_template(work, goal, c, head, &arguments, budget)? {
            return Ok(());
        }
        // Only non-immediate operands need evaluator records. Retain one source
        // spine vector, not a cloned argument/type/capture telescope.
        let mut pending = Vec::new();
        for operand in std::iter::once(&head).chain(arguments.iter()) {
            let Goal::Eval(c) = *operand else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime source operand goal",
                ));
            };
            if !self.available_source(c, budget)? {
                spend(budget, 1)?;
                pending.push(*operand);
            }
        }
        Self::push(
            work,
            Frame::SourceSpine {
                goal,
                source: c,
                head,
                arguments,
            },
            budget,
        )?;
        for operand in pending.into_iter().rev() {
            Self::push(work, Frame::Visit(operand), budget)?;
        }
        Ok(())
    }
    fn source_template(
        &mut self,
        work: &mut Vec<Frame>,
        goal: Goal,
        c: Captured,
        head: Goal,
        arguments: &[Goal],
        budget: &mut usize,
    ) -> Result<bool> {
        if !matches!(c.node, Syntax::Source(_)) {
            return Ok(false);
        }
        // Eval already gathered and bounded this exact application spine.
        // Borrow its operands so a conservative fallback reuses the same
        // vector rather than allocating and traversing the source twice.
        if arguments.is_empty() {
            return Ok(false);
        }
        let Goal::Eval(head) = head else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused source head subject",
            ));
        };
        if !self.available_source(head, budget)? {
            return Ok(false);
        }
        for arg in arguments {
            let Goal::Eval(arg) = arg else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime fused source operand subject",
                ));
            };
            if !self.available_source(*arg, budget)? {
                return Ok(false);
            }
        }
        let head = self.source_operand(head, budget)?;
        spend(budget, arguments.len())?;
        let mut evidence = Vec::with_capacity(arguments.len());
        for arg in arguments {
            let Goal::Eval(arg) = arg else {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime fused source operand subject",
                ));
            };
            let operand = self.source_operand(*arg, budget)?;
            evidence.push(operand);
        }
        // A source beta body already retains the exact ordered operand
        // evidence. A successful native template borrows it directly, without
        // constructing a duplicate ValueId vector or inner Call/interner key.
        if !matches!(goal, Goal::Eval(_)) {
            let candidate = match self.values[head.value()] {
                RuntimeValue::Lambda(Captured {
                    node: Syntax::Native(body),
                    env: 0,
                }) => Some((body, None)),
                RuntimeValue::Ground(function)
                    if evidence
                        .iter()
                        .any(|e| !matches!(self.values[e.value()], RuntimeValue::Ground(_)))
                        && matches!(
                            self.pool.store.node(function),
                            Node::Global(_) | Node::Const { .. }
                        ) =>
                {
                    let link = self.pool.expose(function, budget)?;
                    Some((self.pool.links[link].to, Some(link)))
                }
                _ => None,
            };
            if let Some((body, expose)) = candidate {
                if let Some(template) = self.pool.template(body, budget)? {
                    let program = self.pool.templates[template].checked.program();
                    if evidence.len() >= program.parameters.len() {
                        let count = program.calls.len();
                        spend(budget, count)?;
                        let origin = TemplateOrigin::SourceBeta {
                            source: c,
                            head,
                            arguments: evidence,
                            expose,
                        };
                        let invocation = TemplateInvocation {
                            goal,
                            template,
                            origin,
                            instructions: Vec::with_capacity(count),
                        };
                        Self::push(work, Frame::TemplateStep(invocation), budget)?;
                        return Ok(true);
                    }
                }
            }
        }
        spend(budget, evidence.len())?;
        let values = evidence.iter().map(|operand| operand.value()).collect();
        let call = self.call(head.value(), values, budget)?;
        if matches!(call, Goal::UnaryCall { .. }) {
            if let Some(&premise) = self.done.get(&call) {
                return self
                    .emit(
                        goal,
                        self.proofs[premise].value,
                        Rule::SourceApply {
                            source: c,
                            head,
                            arguments: evidence,
                            call,
                            premise,
                        },
                        budget,
                    )
                    .map(|_| true);
            }
            Self::push(
                work,
                Frame::SourceCall {
                    goal,
                    source: c,
                    head,
                    arguments: evidence,
                    call,
                },
                budget,
            )?;
            Self::push(work, Frame::Visit(call), budget)?;
            return Ok(true);
        }
        let Goal::Call(call_id) = call else {
            unreachable!()
        };
        if let Some(&premise) = self.done.get(&call) {
            return self
                .emit(
                    goal,
                    self.proofs[premise].value,
                    Rule::SourceApply {
                        source: c,
                        head,
                        arguments: evidence,
                        call,
                        premise,
                    },
                    budget,
                )
                .map(|_| true);
        }
        let (body, expose) = match self.values[head.value()] {
            RuntimeValue::Lambda(Captured {
                node: Syntax::Native(body),
                env: 0,
            }) => (Some(body), None),
            RuntimeValue::Ground(function) => {
                if self.calls[call_id]
                    .arguments
                    .iter()
                    .all(|id| matches!(self.values[*id], RuntimeValue::Ground(_)))
                {
                    let mut input = function;
                    for at in 0..self.calls[call_id].arguments.len() {
                        let RuntimeValue::Ground(arg) =
                            self.values[self.calls[call_id].arguments[at]]
                        else {
                            unreachable!()
                        };
                        input = self.pool.app(input, arg, budget)?;
                    }
                    let link = self.pool.ground(input, true, budget)?;
                    let value =
                        self.value(RuntimeValue::Ground(self.pool.links[link].to), budget)?;
                    return self
                        .emit(
                            goal,
                            value,
                            Rule::SourceGround {
                                source: c,
                                head,
                                arguments: evidence,
                                call: call_id,
                                link,
                            },
                            budget,
                        )
                        .map(|_| true);
                }
                if matches!(
                    self.pool.store.node(function),
                    Node::Global(_) | Node::Const { .. }
                ) {
                    let link = self.pool.expose(function, budget)?;
                    (Some(self.pool.links[link].to), Some(link))
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };
        if let Some(body) = body {
            if let Some(template) = self.pool.template(body, budget)? {
                let program = self.pool.templates[template].checked.program();
                if self.calls[call_id].arguments.len() >= program.parameters.len() {
                    let instructions = program.calls.len();
                    if matches!(goal, Goal::Eval(_)) && !self.active.insert(call) {
                        return Err(declined(
                            DiagnosticCode::UnsupportedFeature,
                            "cyclic fused runtime call",
                        ));
                    }
                    spend(budget, instructions)?;
                    let origin = if matches!(goal, Goal::Eval(_)) {
                        TemplateOrigin::Source {
                            call: call_id,
                            head,
                            arguments: evidence,
                            expose,
                        }
                    } else {
                        TemplateOrigin::SourceBeta {
                            source: c,
                            head,
                            arguments: evidence,
                            expose,
                        }
                    };
                    let invocation = TemplateInvocation {
                        goal,
                        template,
                        origin,
                        instructions: Vec::with_capacity(instructions),
                    };
                    Self::push(work, Frame::TemplateStep(invocation), budget)?;
                    return Ok(true);
                }
            }
        }
        // General source call: exact inline/earlier-proof operands plus a
        // previously replayed Call premise. No erased source child is omitted
        // from the original typing certificate.
        Self::push(
            work,
            Frame::SourceCall {
                goal,
                source: c,
                head,
                arguments: evidence,
                call,
            },
            budget,
        )?;
        Self::push(work, Frame::Visit(call), budget)?;
        Ok(true)
    }
    fn start_template(
        &mut self,
        work: &mut Vec<Frame>,
        goal: Goal,
        body: TermId,
        origin: TemplateOrigin,
        budget: &mut usize,
    ) -> Result<bool> {
        let Some(template) = self.pool.template(body, budget)? else {
            return Ok(false);
        };
        let Goal::Call(call) = goal else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime template needs a call",
            ));
        };
        let program = self.pool.templates[template].checked.program();
        if self.calls[call].arguments.len() < program.parameters.len() {
            return Ok(false);
        }
        spend(budget, program.calls.len())?;
        let invocation = TemplateInvocation {
            goal,
            template,
            origin,
            instructions: Vec::with_capacity(program.calls.len()),
        };
        Self::push(work, Frame::TemplateStep(invocation), budget)?;
        Ok(true)
    }
    fn template_value(
        &mut self,
        invocation: &TemplateInvocation,
        reference: runtime_template::TemplateRef,
        budget: &mut usize,
    ) -> Result<ValueId> {
        use runtime_template::TemplateRef;
        spend(budget, 1)?;
        match reference {
            TemplateRef::Argument(slot) => self
                .invocation_view(invocation)?
                .arguments
                .get(slot)
                .copied()
                .ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime template argument index",
                    )
                }),
            TemplateRef::Ground(slot) => {
                let id = *self.pool.templates[invocation.template]
                    .grounds
                    .get(slot)
                    .ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime template ground index",
                        )
                    })?;
                self.value(RuntimeValue::Ground(id), budget)
            }
            TemplateRef::Call(slot) => {
                let proof = *invocation.instructions.get(slot).ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime template forward instruction",
                    )
                })?;
                Ok(self.proofs[proof].value)
            }
        }
    }
    fn template_step(
        &mut self,
        mut invocation: TemplateInvocation,
        work: &mut Vec<Frame>,
        budget: &mut usize,
    ) -> Result<()> {
        loop {
            spend(budget, 1)?;
            let next = invocation.instructions.len();
            let program = self.pool.templates[invocation.template].checked.program();
            if next == program.calls.len() {
                let result = program.result;
                let arity = program.parameters.len();
                let value = self.template_value(&invocation, result, budget)?;
                let view = self.invocation_view(&invocation)?;
                let suffix = view.arguments.suffix(arity);
                if suffix.is_empty() {
                    return self.emit(
                        invocation.goal,
                        value,
                        Rule::Template {
                            template: invocation.template,
                            origin: invocation.origin,
                            instructions: invocation.instructions,
                            tail: None,
                        },
                        budget,
                    );
                }
                spend(budget, suffix.len())?;
                let arguments = suffix.iter().copied().collect();
                let tail = self.call(value, arguments, budget)?;
                if let Some(&proof) = self.done.get(&tail) {
                    return self.emit(
                        invocation.goal,
                        self.proofs[proof].value,
                        Rule::Template {
                            template: invocation.template,
                            origin: invocation.origin,
                            instructions: invocation.instructions,
                            tail: Some(proof),
                        },
                        budget,
                    );
                }
                Self::push(work, Frame::TemplateTail { invocation, tail }, budget)?;
                return Self::push(work, Frame::Visit(tail), budget);
            }
            let instruction = &program.calls[next];
            let function_ref = instruction.function;
            let count = instruction.arguments.len();
            let function = self.template_value(&invocation, function_ref, budget)?;
            spend(budget, count)?;
            let mut arguments = Vec::with_capacity(count);
            for i in 0..count {
                let reference = self.pool.templates[invocation.template]
                    .checked
                    .program()
                    .calls[next]
                    .arguments[i];
                arguments.push(self.template_value(&invocation, reference, budget)?);
            }
            let child = self.call(function, arguments, budget)?;
            if let Some(&proof) = self.done.get(&child) {
                spend(budget, 1)?;
                invocation.instructions.push(proof);
            } else {
                Self::push(work, Frame::TemplateResume { invocation, child }, budget)?;
                return Self::push(work, Frame::Visit(child), budget);
            }
        }
    }
    fn eval(&mut self, root: Goal, budget: &mut usize) -> Result<ProofId> {
        let mut work = Vec::new();
        Self::push(&mut work, Frame::Visit(root), budget)?;
        while let Some(frame) = work.pop() {
            spend(budget, 1)?;
            match frame {
                Frame::Visit(goal) => {
                    if self.done.contains_key(&goal) {
                        continue;
                    }
                    if !self.active.insert(goal) {
                        return Err(declined(
                            DiagnosticCode::UnsupportedFeature,
                            "cyclic runtime demand",
                        ));
                    }
                    match goal {
                        Goal::Eval(c) => {
                            if let Some(value) = self.pool.variable(c, &self.envs, budget)? {
                                self.emit(goal, value, Rule::Variable, budget)?;
                            } else if self.pool.binder(c)?.is_some() {
                                let value = self.value(RuntimeValue::Lambda(c), budget)?;
                                self.emit(goal, value, Rule::Lambda, budget)?;
                            } else if let Some(node) = self.pool.metadata(c)? {
                                let child = Goal::Eval(self.pool.captured(node, c.env));
                                Self::push(&mut work, Frame::Metadata { goal, child }, budget)?;
                                Self::push(&mut work, Frame::Visit(child), budget)?;
                            } else if let Some((name, value, body)) = self.pool.binding(c)? {
                                let value = Goal::Eval(self.pool.captured(value, c.env));
                                Self::push(
                                    &mut work,
                                    Frame::LetValue {
                                        goal,
                                        value,
                                        body,
                                        name,
                                        env: c.env,
                                    },
                                    budget,
                                )?;
                                Self::push(&mut work, Frame::Visit(value), budget)?;
                            } else if self.pool.pair(c.node)?.is_some() {
                                if matches!(c.node, Syntax::Source(_)) {
                                    self.schedule_source(&mut work, goal, c, budget)?;
                                    continue;
                                }
                                let mut head = c.node;
                                let mut arguments = Vec::new();
                                while let Some((f, a)) = self.pool.pair(head)? {
                                    spend(budget, 1)?;
                                    if arguments.len() >= 256 {
                                        return Err(declined(
                                            DiagnosticCode::ResourceLimit,
                                            "runtime source spine bound",
                                        ));
                                    }
                                    arguments.push(Goal::Eval(self.pool.captured(a, c.env)));
                                    head = f;
                                }
                                arguments.reverse();
                                let head = Goal::Eval(self.pool.captured(head, c.env));
                                if self
                                    .source_template(&mut work, goal, c, head, &arguments, budget)?
                                {
                                    continue;
                                }
                                spend(budget, arguments.len())?;
                                Self::push(
                                    &mut work,
                                    Frame::Spine {
                                        goal,
                                        head,
                                        arguments: arguments.clone(),
                                    },
                                    budget,
                                )?;
                                for arg in arguments.into_iter().rev() {
                                    Self::push(&mut work, Frame::Visit(arg), budget)?;
                                }
                                Self::push(&mut work, Frame::Visit(head), budget)?;
                            } else {
                                let id = match c.node {
                                    Syntax::Source(id) => self.pool.atom(id, budget)?,
                                    Syntax::Native(id)
                                        if self.pool.store.free_names_are_empty(id) =>
                                    {
                                        id
                                    }
                                    _ => {
                                        return Err(declined(
                                            DiagnosticCode::UnsupportedFeature,
                                            "runtime open native rule unavailable",
                                        ))
                                    }
                                };
                                let value = self.value(RuntimeValue::Ground(id), budget)?;
                                self.emit(goal, value, Rule::Atom(id), budget)?;
                            }
                        }
                        Goal::UnaryCall { function, argument } => {
                            let RuntimeValue::Lambda(c) = self.values[function] else {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime unary callback is not a source lambda",
                                ));
                            };
                            let (name, node) = self.pool.binder(c)?.ok_or_else(|| {
                                rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime unary callback lambda syntax",
                                )
                            })?;
                            let env = self.envs.extend(c.env, argument, name, budget)?;
                            let captured = self.pool.captured(node, env);
                            if matches!(captured.node, Syntax::Source(_))
                                && self.pool.pair(captured.node)?.is_some()
                            {
                                self.schedule_source(&mut work, goal, captured, budget)?;
                                continue;
                            }
                            let body = Goal::Eval(captured);
                            Self::push(
                                &mut work,
                                Frame::BetaBody {
                                    goal,
                                    body,
                                    remaining: Vec::new(),
                                },
                                budget,
                            )?;
                            Self::push(&mut work, Frame::Visit(body), budget)?;
                        }
                        Goal::Call(id) => {
                            let call = &self.calls[id];
                            spend(budget, call.arguments.len())?;
                            let arguments = call.arguments.clone();
                            let function = self.values[call.function];
                            if arguments.is_empty() {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "empty runtime call",
                                ));
                            }
                            match function {
                                RuntimeValue::Lambda(c) => {
                                    if let Captured {
                                        node: Syntax::Native(body),
                                        env: 0,
                                    } = c
                                    {
                                        if self.start_template(
                                            &mut work,
                                            goal,
                                            body,
                                            TemplateOrigin::ClosedLambda,
                                            budget,
                                        )? {
                                            continue;
                                        }
                                    }
                                    let (name, node) = self.pool.binder(c)?.ok_or_else(|| {
                                        rejected(
                                            DiagnosticCode::InvalidProof,
                                            "runtime non-lambda value",
                                        )
                                    })?;
                                    let env =
                                        self.envs.extend(c.env, arguments[0], name, budget)?;
                                    let captured = self.pool.captured(node, env);
                                    if arguments.len() == 1
                                        && matches!(captured.node, Syntax::Source(_))
                                        && self.pool.pair(captured.node)?.is_some()
                                    {
                                        self.schedule_source(&mut work, goal, captured, budget)?;
                                        continue;
                                    }
                                    let body = Goal::Eval(captured);
                                    spend(budget, arguments.len() - 1)?;
                                    let remaining = arguments[1..].to_vec();
                                    Self::push(
                                        &mut work,
                                        Frame::BetaBody {
                                            goal,
                                            body,
                                            remaining,
                                        },
                                        budget,
                                    )?;
                                    Self::push(&mut work, Frame::Visit(body), budget)?;
                                }
                                RuntimeValue::Ground(head) => {
                                    if arguments.iter().all(|id| {
                                        matches!(self.values[*id], RuntimeValue::Ground(_))
                                    }) {
                                        let mut input = head;
                                        for arg in arguments {
                                            let RuntimeValue::Ground(arg) = self.values[arg] else {
                                                unreachable!()
                                            };
                                            input = self.pool.app(input, arg, budget)?;
                                        }
                                        let link = self.pool.ground(input, true, budget)?;
                                        let value = self.value(
                                            RuntimeValue::Ground(self.pool.links[link].to),
                                            budget,
                                        )?;
                                        self.emit(goal, value, Rule::Ground(link), budget)?;
                                    } else if matches!(self.pool.store.node(head), Node::App(..)) {
                                        let mut base = head;
                                        let mut prefix = Vec::new();
                                        while let Node::App(f, a) = self.pool.store.node(base) {
                                            spend(budget, 1)?;
                                            prefix.push(*a);
                                            base = *f;
                                        }
                                        prefix.reverse();
                                        let mut all = Vec::new();
                                        for arg in prefix {
                                            spend(budget, 1)?;
                                            all.push(
                                                self.value(RuntimeValue::Ground(arg), budget)?,
                                            );
                                        }
                                        spend(budget, arguments.len())?;
                                        all.extend(arguments);
                                        let function =
                                            self.value(RuntimeValue::Ground(base), budget)?;
                                        let call = self.call(function, all, budget)?;
                                        Self::push(
                                            &mut work,
                                            Frame::Reassociate { goal, call },
                                            budget,
                                        )?;
                                        Self::push(&mut work, Frame::Visit(call), budget)?;
                                    } else {
                                        let link = self.pool.expose(head, budget)?;
                                        let native_body = self.pool.links[link].to;
                                        if self.start_template(
                                            &mut work,
                                            goal,
                                            native_body,
                                            TemplateOrigin::Exposed { link },
                                            budget,
                                        )? {
                                            continue;
                                        }
                                        let body =
                                            Goal::Eval(self.pool.captured(
                                                Syntax::Native(self.pool.links[link].to),
                                                0,
                                            ));
                                        Self::push(
                                            &mut work,
                                            Frame::ExposeBody {
                                                goal,
                                                link,
                                                body,
                                                arguments,
                                            },
                                            budget,
                                        )?;
                                        Self::push(&mut work, Frame::Visit(body), budget)?;
                                    }
                                }
                            }
                        }
                    }
                }
                Frame::Metadata { goal, child } => {
                    let p = self.done[&child];
                    self.emit(goal, self.proofs[p].value, Rule::Metadata(p), budget)?;
                }
                Frame::LetValue {
                    goal,
                    value,
                    body,
                    name,
                    env,
                } => {
                    let p = self.done[&value];
                    let env = self.envs.extend(env, self.proofs[p].value, name, budget)?;
                    let body = Goal::Eval(self.pool.captured(body, env));
                    Self::push(
                        &mut work,
                        Frame::LetBody {
                            goal,
                            value: p,
                            body,
                        },
                        budget,
                    )?;
                    Self::push(&mut work, Frame::Visit(body), budget)?;
                }
                Frame::LetBody { goal, value, body } => {
                    let p = self.done[&body];
                    self.emit(
                        goal,
                        self.proofs[p].value,
                        Rule::Let { value, body: p },
                        budget,
                    )?;
                }
                Frame::SourceSpine {
                    goal,
                    source,
                    head,
                    arguments,
                } => {
                    if !self.source_template(&mut work, goal, source, head, &arguments, budget)? {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime pending source operands were not completed",
                        ));
                    }
                }
                Frame::SourceCall {
                    goal,
                    source,
                    head,
                    arguments,
                    call,
                } => {
                    let premise = self.done[&call];
                    self.emit(
                        goal,
                        self.proofs[premise].value,
                        Rule::SourceApply {
                            source,
                            head,
                            arguments,
                            call,
                            premise,
                        },
                        budget,
                    )?;
                }
                Frame::Spine {
                    goal,
                    head,
                    arguments,
                } => {
                    spend(budget, arguments.len() * 2)?;
                    let head = self.done[&head];
                    let args: Vec<_> = arguments.iter().map(|g| self.done[g]).collect();
                    let vals = args.iter().map(|p| self.proofs[*p].value).collect();
                    let call = self.call(self.proofs[head].value, vals, budget)?;
                    Self::push(
                        &mut work,
                        Frame::SpineCall {
                            goal,
                            head,
                            arguments: args,
                            call,
                        },
                        budget,
                    )?;
                    Self::push(&mut work, Frame::Visit(call), budget)?;
                }
                Frame::SpineCall {
                    goal,
                    head,
                    arguments,
                    call,
                } => {
                    let p = self.done[&call];
                    self.emit(
                        goal,
                        self.proofs[p].value,
                        Rule::Spine {
                            head,
                            arguments,
                            call: p,
                        },
                        budget,
                    )?;
                }
                Frame::BetaBody {
                    goal,
                    body,
                    remaining,
                } => {
                    let p = self.done[&body];
                    if remaining.is_empty() {
                        self.emit(
                            goal,
                            self.proofs[p].value,
                            Rule::Beta {
                                body: p,
                                tail: None,
                            },
                            budget,
                        )?;
                    } else {
                        let tail = self.call(self.proofs[p].value, remaining, budget)?;
                        Self::push(
                            &mut work,
                            Frame::BetaTail {
                                goal,
                                body: p,
                                tail,
                            },
                            budget,
                        )?;
                        Self::push(&mut work, Frame::Visit(tail), budget)?;
                    }
                }
                Frame::BetaTail { goal, body, tail } => {
                    let p = self.done[&tail];
                    self.emit(
                        goal,
                        self.proofs[p].value,
                        Rule::Beta {
                            body,
                            tail: Some(p),
                        },
                        budget,
                    )?;
                }
                Frame::ExposeBody {
                    goal,
                    link,
                    body,
                    arguments,
                } => {
                    let p = self.done[&body];
                    let call = self.call(self.proofs[p].value, arguments, budget)?;
                    Self::push(
                        &mut work,
                        Frame::ExposeCall {
                            goal,
                            link,
                            body: p,
                            call,
                        },
                        budget,
                    )?;
                    Self::push(&mut work, Frame::Visit(call), budget)?;
                }
                Frame::ExposeCall {
                    goal,
                    link,
                    body,
                    call,
                } => {
                    let p = self.done[&call];
                    self.emit(
                        goal,
                        self.proofs[p].value,
                        Rule::Expose {
                            link,
                            body,
                            call: p,
                        },
                        budget,
                    )?;
                }
                Frame::Reassociate { goal, call } => {
                    let p = self.done[&call];
                    self.emit(goal, self.proofs[p].value, Rule::Reassociate(p), budget)?;
                }
                Frame::TemplateStep(invocation) => {
                    self.template_step(invocation, &mut work, budget)?
                }
                Frame::TemplateResume {
                    mut invocation,
                    child,
                } => {
                    spend(budget, 1)?;
                    invocation.instructions.push(self.done[&child]);
                    self.template_step(invocation, &mut work, budget)?;
                }
                Frame::TemplateTail { invocation, tail } => {
                    let proof = self.done[&tail];
                    self.emit(
                        invocation.goal,
                        self.proofs[proof].value,
                        Rule::Template {
                            template: invocation.template,
                            origin: invocation.origin,
                            instructions: invocation.instructions,
                            tail: Some(proof),
                        },
                        budget,
                    )?;
                }
            }
        }
        Ok(self.done[&root])
    }
    fn finish(&mut self, id: TermId, budget: &mut usize) -> Result<Term> {
        // The producer may have populated a cache entry that no replayed rule
        // used. Independently check that final proposal as well as its exact
        // starting endpoint; cache presence never grants conversion authority.
        if let Some(cached) = self.pool.grounds.get(&id) {
            let link = self.pool.links.get(*cached).ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime final cached link index",
                )
            })?;
            if link.from != id || !matches!(link.kind, LinkKind::Ground { .. }) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime final cached link changed its subject or kind",
                ));
            }
            if matches!(link.kind, LinkKind::Ground { partial: true }) {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "runtime root remains function-valued",
                ));
            }
        }
        let id_link = self.pool.ground(id, false, budget)?;
        let mut bridge = NativeCertificates::new(&self.pool.source.importer.ctx, budget)?;
        self.replay_link(id_link, &mut bridge, &mut HashSet::new(), budget)?;
        let link = self.pool.links.get(id_link).ok_or_else(|| {
            rejected(
                DiagnosticCode::InvalidProof,
                "runtime final native link index",
            )
        })?;
        if link.from != id || !matches!(link.kind, LinkKind::Ground { partial: false }) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime final link changed its source endpoint",
            ));
        }
        self.pool.copy(link.to, budget)
    }
    fn replay_link(
        &self,
        id: usize,
        bridge: &mut NativeCertificates<'_>,
        checked: &mut HashSet<usize>,
        budget: &mut usize,
    ) -> Result<()> {
        spend(budget, 1)?;
        if checked.contains(&id) {
            return Ok(());
        }
        let link =
            self.pool.links.get(id).ok_or_else(|| {
                rejected(DiagnosticCode::InvalidProof, "runtime native link index")
            })?;
        let from = self.pool.copy(link.from, budget)?;
        let to = self.pool.copy(link.to, budget)?;
        match link.kind {
            LinkKind::Expose => {
                let original = bridge.expose(&from, budget)?;
                if original != to {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime exposure changed the checked definition body",
                    ));
                }
            }
            LinkKind::Ground { partial } => {
                spend(budget, 1)?;
                let ty = lean_type(&self.pool.source.importer.ctx, &from)?;
                if matches!(
                    normal(&self.pool.source.importer.ctx, &ty)?,
                    Term::Pi { .. }
                ) != partial
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime native link changed its function-valued classification",
                    ));
                }
                bridge.conversion(&from, &to, &ty, budget)?;
            }
        }
        spend(budget, 1)?;
        checked.insert(id);
        Ok(())
    }
    fn replay_operand(
        &self,
        c: Captured,
        evidence: OperandEvidence,
        environments: &Environments,
        atoms: &mut HashMap<usize, TermId>,
        at: usize,
        budget: &mut usize,
    ) -> Result<ValueId> {
        spend(budget, 1)?;
        let Syntax::Source(source) = c.node else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime inline operand is not source syntax",
            ));
        };
        let actual = self.values.get(evidence.value()).copied().ok_or_else(|| {
            rejected(
                DiagnosticCode::InvalidProof,
                "runtime inline operand value index",
            )
        })?;
        let row = self.pool.source.row(source)?;
        match evidence {
            OperandEvidence::Proof { proof, value } => {
                if proof >= at {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime source operand proof is forward or cyclic",
                    ));
                }
                let original = self.proofs.get(proof).ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime source operand proof index",
                    )
                })?;
                if original.goal != Goal::Eval(c) || original.value != value {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime source operand proof changed its source or environment",
                    ));
                }
            }
            OperandEvidence::Lambda(_) => {
                if row.get("lam").is_none()
                    || self.pool.binder(c)?.is_none()
                    || actual != RuntimeValue::Lambda(c)
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime inline lambda changed its original syntax or captured values",
                    ));
                }
            }
            OperandEvidence::Variable(id) => {
                if row.get("bvar").is_none()
                    || self.pool.variable(c, environments, budget)? != Some(id)
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime inline variable changed its scope lookup",
                    ));
                }
            }
            OperandEvidence::Atom(_) => {
                if !(row.get("const").is_some()
                    || row.get("sort").is_some()
                    || row.get("natVal").is_some())
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime inline atom is not original atomic syntax",
                    ));
                }
                let RuntimeValue::Ground(native) = actual else {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime inline atom endpoint is not native",
                    ));
                };
                if let Some(expected) = atoms.get(&source) {
                    if *expected != native {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime inline source atom endpoint changed",
                        ));
                    }
                } else {
                    let original = self.pool.source.typing.closed_readback(source, budget)?;
                    if original != self.pool.copy(native, budget)? {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime inline source atom changed its original value",
                        ));
                    }
                    spend(budget, 1)?;
                    atoms.insert(source, native);
                }
            }
        }
        Ok(evidence.value())
    }
    fn replay_source_subject(
        &self,
        goal: Goal,
        source: Captured,
        environments: &Environments,
        budget: &mut usize,
    ) -> Result<()> {
        spend(budget, 1)?;
        if !matches!(source.node, Syntax::Source(_)) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused body is not source syntax",
            ));
        }
        match goal {
            Goal::Eval(original) if original == source => Ok(()),
            Goal::Call(_) | Goal::UnaryCall { .. } => {
                let call = self.call_view(goal)?;
                if call.arguments.len() != 1 {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime fused beta changes saturation",
                    ));
                }
                let RuntimeValue::Lambda(lambda) =
                    *self.values.get(call.function).ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime fused beta function index",
                        )
                    })?
                else {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime fused beta needs a source lambda",
                    ));
                };
                let (name, node) = self.pool.binder(lambda)?.ok_or_else(|| {
                    rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime fused beta lambda syntax",
                    )
                })?;
                let closed = self.pool.captured(node, 1).env == 0;
                if !closed
                    && !environments.verifies(
                        source.env,
                        RawBinding {
                            parent: lambda.env,
                            value: call.arguments[0],
                            name,
                        },
                    )
                {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime fused beta changed its argument or capture",
                    ));
                }
                if self.pool.captured(node, source.env) != source {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime fused beta changed the original body",
                    ));
                }
                Ok(())
            }
            _ => Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused source subject changed",
            )),
        }
    }
    fn replay_source_application(
        &self,
        goal: Goal,
        head: OperandEvidence,
        arguments: &[OperandEvidence],
        call: &CallView<'_>,
        environments: &Environments,
        atoms: &mut HashMap<usize, TermId>,
        at: usize,
        budget: &mut usize,
    ) -> Result<()> {
        let Goal::Eval(c) = goal else {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused source certificate has no source judgment",
            ));
        };
        if !matches!(c.node, Syntax::Source(_)) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused application is not source syntax",
            ));
        }
        let mut node = c.node;
        let mut expected = Vec::new();
        while let Some((f, a)) = self.pool.pair(node)? {
            spend(budget, 1)?;
            if expected.len() >= 256 {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime replay fused spine bound",
                ));
            }
            expected.push(self.pool.captured(a, c.env));
            node = f;
        }
        if expected.is_empty()
            || expected.len() != arguments.len()
            || arguments.len() != call.arguments.len()
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused source application arity changed",
            ));
        }
        if self.replay_operand(
            self.pool.captured(node, c.env),
            head,
            environments,
            atoms,
            at,
            budget,
        )? != call.function
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime fused source application head changed",
            ));
        }
        for ((source, evidence), actual) in expected
            .iter()
            .rev()
            .zip(arguments)
            .zip(call.arguments.iter())
        {
            spend(budget, 1)?;
            if self.replay_operand(*source, *evidence, environments, atoms, at, budget)? != *actual
            {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime fused source application operand changed",
                ));
            }
        }
        Ok(())
    }
    fn replay_template(
        &self,
        id: usize,
        body: TermId,
        checked: &mut HashSet<(usize, TermId)>,
        budget: &mut usize,
    ) -> Result<()> {
        spend(budget, 1)?;
        let record = self.pool.templates.get(id).ok_or_else(|| {
            rejected(
                DiagnosticCode::InvalidProof,
                "runtime template record index",
            )
        })?;
        if record.body != body {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime template original body changed",
            ));
        }
        self.pool.handle(body)?;
        if !self.pool.store.free_names_are_empty(body)
            || !matches!(self.pool.store.node(body), Node::Lambda { .. })
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime template original is not a closed lambda",
            ));
        }
        if checked.contains(&(id, body)) {
            return Ok(());
        }
        let original = self.pool.copy(body, budget)?;
        let program = record.checked.program();
        runtime_template::verify(&self.pool.source.importer.ctx, &original, program, budget)?;
        if program.grounds.len() != record.grounds.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime template ground inventory changed",
            ));
        }
        for (original, native) in program.grounds.iter().zip(&record.grounds) {
            spend(budget, 1)?;
            if *original != self.pool.copy(*native, budget)? {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime template ground syntax changed",
                ));
            }
        }
        spend(budget, 1)?;
        checked.insert((id, body));
        Ok(())
    }
    /// Independent rule replay. It never consults the producer's done/active
    /// maps, call-interning cache, value interner, or environment jump tables.
    /// Source atoms are reconstructed from the original typed source. Native
    /// links are untrusted proposals, independently certified once on replay;
    /// this checker also verifies their exact relation to each source judgment.
    fn replay(&self, root: Goal, root_proof: ProofId, budget: &mut usize) -> Result<RuntimeValue> {
        fn premise<'p>(proofs: &'p [Proof], at: usize, id: usize) -> Result<&'p Proof> {
            if id >= at {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime premise is cyclic or forward",
                ));
            }
            proofs
                .get(id)
                .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "runtime missing premise"))
        }
        fn require_goal(proof: &Proof, expected: Goal) -> Result<()> {
            if proof.goal != expected {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime premise describes a different source",
                ));
            }
            Ok(())
        }
        let value = |id: ValueId| {
            self.values
                .get(id)
                .copied()
                .ok_or_else(|| rejected(DiagnosticCode::InvalidProof, "runtime value index"))
        };
        let get_call = |goal: Goal| -> Result<CallView<'_>> { self.call_view(goal) };
        // A previously replayed fused source rule proves its explicit Call
        // projection as well as its primary Eval judgment. No other rule can
        // substitute an arbitrary stored call for its source goal.
        let proof_call = |proof: &Proof| -> Result<CallView<'_>> {
            match (&proof.goal, &proof.rule) {
                (Goal::Call(_) | Goal::UnaryCall { .. }, _) => get_call(proof.goal),
                (
                    Goal::Eval(_),
                    Rule::Template {
                        origin: TemplateOrigin::Source { call, .. },
                        ..
                    },
                )
                | (Goal::Eval(_), Rule::SourceGround { call, .. }) => get_call(Goal::Call(*call)),
                (Goal::Eval(_), Rule::SourceApply { call, .. }) => get_call(*call),
                _ => Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime proof has no checked call projection",
                )),
            }
        };
        let check_call =
            |proof: &Proof, function: ValueId, args: CallArguments<'_>| -> Result<()> {
                let call = proof_call(proof)?;
                if call.function != function || !call.arguments.iter().eq(args.iter()) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime call value/argument mismatch",
                    ));
                }
                Ok(())
            };
        if root_proof.checked_add(1) != Some(self.proofs.len())
            || self.proofs.get(root_proof).is_none_or(|p| p.goal != root)
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime trace concludes a different root",
            ));
        }
        let validate_capture = |capture: Captured| -> Result<()> {
            if capture.env >= self.envs.raw.len() {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime capture environment index",
                ));
            }
            match capture.node {
                Syntax::Native(id) => self.pool.handle(id),
                Syntax::Source(id) => self.pool.source.row(id).map(|_| ()),
            }
        };
        // Validate all raw handles before any store indexing. Numeric handles
        // have meaning only in this pool; replay then anchors their actual
        // syntax to original source atoms or independently checked native links.
        for v in &self.values {
            spend(budget, 1)?;
            match v {
                RuntimeValue::Ground(id) => self.pool.handle(*id)?,
                RuntimeValue::Lambda(c) => validate_capture(*c)?,
            }
        }
        for p in &self.proofs {
            spend(budget, 1)?;
            match p.goal {
                Goal::Eval(c) => validate_capture(c)?,
                Goal::UnaryCall { function, argument } => {
                    value(function)?;
                    spend(budget, 1)?;
                    value(argument)?;
                }
                Goal::Call(_) => {}
            }
            if let Rule::Atom(id) = p.rule {
                self.pool.handle(id)?;
            }
            match &p.rule {
                Rule::SourceGround { source, .. }
                | Rule::SourceApply { source, .. }
                | Rule::Template {
                    origin: TemplateOrigin::SourceBeta { source, .. },
                    ..
                } => validate_capture(*source)?,
                _ => {}
            }
        }
        for call in &self.calls {
            spend(budget, 1)?;
            if call.arguments.len() > 256 {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime replay application spine bound",
                ));
            }
            value(call.function)?;
            for argument in &call.arguments {
                spend(budget, 1)?;
                value(*argument)?;
            }
        }
        let mut replay_bridge = NativeCertificates::new(&self.pool.source.importer.ctx, budget)?;
        let mut checked_atoms = HashMap::<usize, TermId>::new();
        let mut checked_links = HashSet::new();
        let mut checked_templates = HashSet::new();
        let mut environments = Environments::new();
        for (id, raw) in self.envs.raw.iter().enumerate().skip(1) {
            spend(budget, 1)?;
            let b = raw.ok_or_else(|| {
                rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime missing environment record",
                )
            })?;
            if b.parent >= id || b.value >= self.values.len() {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime forged environment edge",
                ));
            }
            if let RuntimeValue::Lambda(capture) = value(b.value)? {
                if capture.env >= id {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime cyclic captured value environment",
                    ));
                }
            }
            environments.append(b, budget)?;
        }
        for (at, proof) in self.proofs.iter().enumerate() {
            spend(budget, 1)?;
            let claimed = value(proof.value)?;
            let actual = match &proof.rule {
                Rule::Atom(id) => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime atom rule subject",
                        ));
                    };
                    match c.node {
                        Syntax::Source(source) => {
                            if let Some(checked) = checked_atoms.get(&source) {
                                if checked != id {
                                    return Err(rejected(
                                        DiagnosticCode::InvalidProof,
                                        "runtime source atom changed its endpoint",
                                    ));
                                }
                            } else {
                                let original =
                                    self.pool.source.typing.closed_readback(source, budget)?;
                                let proposed = self.pool.copy(*id, budget)?;
                                if original != proposed {
                                    return Err(rejected(
                                        DiagnosticCode::InvalidProof,
                                        "runtime atom is not original closed source",
                                    ));
                                }
                                spend(budget, 1)?;
                                checked_atoms.insert(source, *id);
                            }
                        }
                        Syntax::Native(native) => {
                            if native != *id
                                || !self.pool.costs.contains_key(id)
                                || !self.pool.store.free_names_are_empty(*id)
                                || matches!(
                                    self.pool.store.node(*id),
                                    Node::Lambda { .. }
                                        | Node::Var(_)
                                        | Node::App(..)
                                        | Node::Let { .. }
                                )
                            {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime atom is not admitted closed native syntax",
                                ));
                            }
                        }
                    }
                    RuntimeValue::Ground(*id)
                }
                Rule::Lambda => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime lambda rule subject",
                        ));
                    };
                    if self.pool.binder(c)?.is_none() {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime lambda syntax mismatch",
                        ));
                    }
                    RuntimeValue::Lambda(c)
                }
                Rule::Variable => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime variable rule subject",
                        ));
                    };
                    let id = self
                        .pool
                        .variable(c, &environments, budget)?
                        .ok_or_else(|| {
                            rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime variable syntax mismatch",
                            )
                        })?;
                    value(id)?
                }
                Rule::Metadata(child) => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime metadata subject",
                        ));
                    };
                    let node = self.pool.metadata(c)?.ok_or_else(|| {
                        rejected(DiagnosticCode::InvalidProof, "runtime metadata syntax")
                    })?;
                    let p = premise(&self.proofs, at, *child)?;
                    require_goal(p, Goal::Eval(self.pool.captured(node, c.env)))?;
                    value(p.value)?
                }
                Rule::Let {
                    value: vp,
                    body: bp,
                } => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime let subject",
                        ));
                    };
                    let (name, node, body) = self.pool.binding(c)?.ok_or_else(|| {
                        rejected(DiagnosticCode::InvalidProof, "runtime let syntax")
                    })?;
                    let v = premise(&self.proofs, at, *vp)?;
                    let b = premise(&self.proofs, at, *bp)?;
                    require_goal(v, Goal::Eval(self.pool.captured(node, c.env)))?;
                    let Goal::Eval(next) = b.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime let body subject",
                        ));
                    };
                    let closed = self.pool.captured(body, 1).env == 0;
                    if !closed
                        && !environments.verifies(
                            next.env,
                            RawBinding {
                                parent: c.env,
                                value: v.value,
                                name,
                            },
                        )
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime let changed its captured value",
                        ));
                    }
                    require_goal(b, Goal::Eval(self.pool.captured(body, next.env)))?;
                    value(b.value)?
                }
                Rule::Spine {
                    head,
                    arguments,
                    call,
                } => {
                    let Goal::Eval(c) = proof.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime spine subject",
                        ));
                    };
                    let mut node = c.node;
                    let mut expected = Vec::new();
                    while let Some((f, a)) = self.pool.pair(node)? {
                        spend(budget, 1)?;
                        if expected.len() >= 256 {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "runtime replay spine bound",
                            ));
                        }
                        expected.push(self.pool.captured(a, c.env));
                        node = f;
                    }
                    if expected.is_empty() || expected.len() != arguments.len() {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime spine arity mismatch",
                        ));
                    }
                    let h = premise(&self.proofs, at, *head)?;
                    require_goal(h, Goal::Eval(self.pool.captured(node, c.env)))?;
                    let cp = premise(&self.proofs, at, *call)?;
                    let actual_call = proof_call(cp)?;
                    if actual_call.function != h.value
                        || actual_call.arguments.len() != arguments.len()
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime spine call mismatch",
                        ));
                    }
                    for ((expected, arg), actual) in expected
                        .iter()
                        .rev()
                        .zip(arguments)
                        .zip(actual_call.arguments.iter())
                    {
                        spend(budget, 1)?;
                        let p = premise(&self.proofs, at, *arg)?;
                        require_goal(p, Goal::Eval(*expected))?;
                        if p.value != *actual {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime spine changed an argument endpoint",
                            ));
                        }
                    }
                    value(cp.value)?
                }
                Rule::Beta { body, tail } => {
                    let call = proof_call(proof)?;
                    let RuntimeValue::Lambda(lambda) = value(call.function)? else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime beta function is not a lambda",
                        ));
                    };
                    let Some(&argument) = call.arguments.first() else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime beta has no argument",
                        ));
                    };
                    let (name, node) = self.pool.binder(lambda)?.ok_or_else(|| {
                        rejected(DiagnosticCode::InvalidProof, "runtime beta lambda syntax")
                    })?;
                    let b = premise(&self.proofs, at, *body)?;
                    let Goal::Eval(next) = b.goal else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime beta body subject",
                        ));
                    };
                    let closed = self.pool.captured(node, 1).env == 0;
                    if !closed
                        && !environments.verifies(
                            next.env,
                            RawBinding {
                                parent: lambda.env,
                                value: argument,
                                name,
                            },
                        )
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime beta discarded or changed its capture",
                        ));
                    }
                    require_goal(b, Goal::Eval(self.pool.captured(node, next.env)))?;
                    match tail {
                        None if call.arguments.len() == 1 => value(b.value)?,
                        Some(t) => {
                            let p = premise(&self.proofs, at, *t)?;
                            check_call(p, b.value, call.arguments.suffix(1))?;
                            value(p.value)?
                        }
                        _ => {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime beta dropped remaining arguments",
                            ))
                        }
                    }
                }
                Rule::Expose {
                    link,
                    body,
                    call: cp,
                } => {
                    let original = proof_call(proof)?;
                    let RuntimeValue::Ground(head) = value(original.function)? else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime delta head is not native",
                        ));
                    };
                    self.replay_link(*link, &mut replay_bridge, &mut checked_links, budget)?;
                    let link = self.pool.links.get(*link).ok_or_else(|| {
                        rejected(DiagnosticCode::InvalidProof, "runtime native link index")
                    })?;
                    if !matches!(link.kind, LinkKind::Expose) || link.from != head {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime delta certificate belongs to another constant",
                        ));
                    }
                    let b = premise(&self.proofs, at, *body)?;
                    require_goal(
                        b,
                        Goal::Eval(self.pool.captured(Syntax::Native(link.to), 0)),
                    )?;
                    let p = premise(&self.proofs, at, *cp)?;
                    check_call(p, b.value, original.arguments)?;
                    value(p.value)?
                }
                Rule::SourceApply {
                    source,
                    head,
                    arguments,
                    call,
                    premise: child,
                } => {
                    self.replay_source_subject(proof.goal, *source, &environments, budget)?;
                    let original = get_call(*call)?;
                    self.replay_source_application(
                        Goal::Eval(*source),
                        *head,
                        arguments,
                        &original,
                        &environments,
                        &mut checked_atoms,
                        at,
                        budget,
                    )?;
                    let child = premise(&self.proofs, at, *child)?;
                    check_call(child, original.function, original.arguments)?;
                    value(child.value)?
                }
                Rule::SourceGround {
                    source,
                    head,
                    arguments,
                    call,
                    link,
                } => {
                    self.replay_source_subject(proof.goal, *source, &environments, budget)?;
                    let original = get_call(Goal::Call(*call))?;
                    self.replay_source_application(
                        Goal::Eval(*source),
                        *head,
                        arguments,
                        &original,
                        &environments,
                        &mut checked_atoms,
                        at,
                        budget,
                    )?;
                    let RuntimeValue::Ground(head) = value(original.function)? else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime fused native head",
                        ));
                    };
                    self.replay_link(*link, &mut replay_bridge, &mut checked_links, budget)?;
                    let link = self.pool.links.get(*link).ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime fused native link index",
                        )
                    })?;
                    if !matches!(link.kind, LinkKind::Ground { .. }) {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime fused native link kind",
                        ));
                    }
                    let mut node = link.from;
                    for argument in original.arguments.iter().rev() {
                        spend(budget, 1)?;
                        let RuntimeValue::Ground(expected) = value(*argument)? else {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime fused ground call captured lambda",
                            ));
                        };
                        let Node::App(f, a) = self.pool.store.node(node) else {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime fused native application shape",
                            ));
                        };
                        if *a != expected {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime fused native argument changed",
                            ));
                        }
                        node = *f;
                    }
                    if node != head {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime fused native head changed",
                        ));
                    }
                    RuntimeValue::Ground(link.to)
                }
                Rule::Ground(link) => {
                    let call = proof_call(proof)?;
                    let RuntimeValue::Ground(head) = value(call.function)? else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime native ground head",
                        ));
                    };
                    self.replay_link(*link, &mut replay_bridge, &mut checked_links, budget)?;
                    let link = self.pool.links.get(*link).ok_or_else(|| {
                        rejected(DiagnosticCode::InvalidProof, "runtime ground link index")
                    })?;
                    if !matches!(link.kind, LinkKind::Ground { .. }) {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime wrong native link kind",
                        ));
                    }
                    let mut node = link.from;
                    for arg in call.arguments.iter().rev() {
                        spend(budget, 1)?;
                        let RuntimeValue::Ground(expected) = value(*arg)? else {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime native call smuggles a captured lambda",
                            ));
                        };
                        let Node::App(f, a) = self.pool.store.node(node) else {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime native call shape",
                            ));
                        };
                        if *a != expected {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime native certificate changed an operand",
                            ));
                        }
                        node = *f;
                    }
                    if node != head {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime native certificate changed its head",
                        ));
                    }
                    RuntimeValue::Ground(link.to)
                }
                Rule::Template {
                    template,
                    origin,
                    instructions,
                    tail,
                } => {
                    use runtime_template::TemplateRef;
                    let original = if let TemplateOrigin::SourceBeta {
                        head, arguments, ..
                    } = origin
                    {
                        CallView {
                            function: head.value(),
                            arguments: CallArguments::Evidence(arguments),
                        }
                    } else {
                        proof_call(proof)?
                    };
                    let record = self.pool.templates.get(*template).ok_or_else(|| {
                        rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime template record index",
                        )
                    })?;
                    match origin {
                        TemplateOrigin::Source {
                            head, arguments, ..
                        } => {
                            self.replay_source_application(
                                proof.goal,
                                *head,
                                arguments,
                                &original,
                                &environments,
                                &mut checked_atoms,
                                at,
                                budget,
                            )?;
                        }
                        TemplateOrigin::SourceBeta {
                            source,
                            head,
                            arguments,
                            ..
                        } => {
                            if !matches!(proof.goal, Goal::Call(_) | Goal::UnaryCall { .. }) {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime beta-template has no lambda call",
                                ));
                            }
                            self.replay_source_subject(proof.goal, *source, &environments, budget)?;
                            self.replay_source_application(
                                Goal::Eval(*source),
                                *head,
                                arguments,
                                &original,
                                &environments,
                                &mut checked_atoms,
                                at,
                                budget,
                            )?;
                        }
                        _ => {}
                    }
                    let body = match origin {
                        TemplateOrigin::ClosedLambda
                        | TemplateOrigin::Source { expose: None, .. }
                        | TemplateOrigin::SourceBeta { expose: None, .. } => {
                            let RuntimeValue::Lambda(Captured {
                                node: Syntax::Native(body),
                                env: 0,
                            }) = value(original.function)?
                            else {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template closed-lambda origin changed",
                                ));
                            };
                            body
                        }
                        TemplateOrigin::Exposed { link }
                        | TemplateOrigin::Source {
                            expose: Some(link), ..
                        }
                        | TemplateOrigin::SourceBeta {
                            expose: Some(link), ..
                        } => {
                            let RuntimeValue::Ground(head) = value(original.function)? else {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template exposure head changed",
                                ));
                            };
                            self.replay_link(
                                *link,
                                &mut replay_bridge,
                                &mut checked_links,
                                budget,
                            )?;
                            let link = self.pool.links.get(*link).ok_or_else(|| {
                                rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template exposure index",
                                )
                            })?;
                            if !matches!(link.kind, LinkKind::Expose) || link.from != head {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template exposure belongs to another head",
                                ));
                            }
                            link.to
                        }
                    };
                    self.replay_template(*template, body, &mut checked_templates, budget)?;
                    let program = record.checked.program();
                    let arity = program.parameters.len();
                    if original.arguments.len() < arity || instructions.len() != program.calls.len()
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime template argument or instruction count changed",
                        ));
                    }
                    // Exact dynamic values and closed native syntax occupy
                    // separate cases; no type-based or producer-interner lookup.
                    enum Expected {
                        Value(ValueId),
                        Ground(TermId),
                    }
                    let reference = |r: TemplateRef, limit: usize| -> Result<Expected> {
                        Ok(match r {
                            TemplateRef::Argument(slot) => Expected::Value(
                                *original
                                    .arguments
                                    .get(slot)
                                    .filter(|_| slot < arity)
                                    .ok_or_else(|| {
                                        rejected(
                                            DiagnosticCode::InvalidProof,
                                            "runtime template formal index",
                                        )
                                    })?,
                            ),
                            TemplateRef::Ground(slot) => {
                                Expected::Ground(*record.grounds.get(slot).ok_or_else(|| {
                                    rejected(
                                        DiagnosticCode::InvalidProof,
                                        "runtime template ground index",
                                    )
                                })?)
                            }
                            TemplateRef::Call(slot) => {
                                if slot >= limit {
                                    return Err(rejected(
                                        DiagnosticCode::InvalidProof,
                                        "runtime template instruction is forward or cyclic",
                                    ));
                                }
                                let id = *instructions.get(slot).ok_or_else(|| {
                                    rejected(
                                        DiagnosticCode::InvalidProof,
                                        "runtime template missing instruction proof",
                                    )
                                })?;
                                Expected::Value(premise(&self.proofs, at, id)?.value)
                            }
                        })
                    };
                    let check_value = |expected: Expected, actual: ValueId| -> Result<()> {
                        let equal = match expected {
                            Expected::Value(id) => id == actual,
                            Expected::Ground(id) => value(actual)? == RuntimeValue::Ground(id),
                        };
                        if !equal {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime template changed a dynamic operand",
                            ));
                        }
                        Ok(())
                    };
                    for (slot, (instruction, id)) in
                        program.calls.iter().zip(instructions).enumerate()
                    {
                        spend(budget, 1)?;
                        let child = premise(&self.proofs, at, *id)?;
                        let call = proof_call(child)?;
                        if call.arguments.len() != instruction.arguments.len() {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime template call arity changed",
                            ));
                        }
                        check_value(reference(instruction.function, slot)?, call.function)?;
                        for (reference_arg, actual) in
                            instruction.arguments.iter().zip(call.arguments.iter())
                        {
                            spend(budget, 1)?;
                            check_value(reference(*reference_arg, slot)?, *actual)?;
                        }
                    }
                    let result = reference(program.result, instructions.len())?;
                    let suffix = original.arguments.suffix(arity);
                    match tail {
                        Some(id) => {
                            if suffix.is_empty() {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template invented trailing arguments",
                                ));
                            }
                            let child = premise(&self.proofs, at, *id)?;
                            let call = proof_call(child)?;
                            check_value(result, call.function)?;
                            spend(budget, suffix.len())?;
                            if !call.arguments.iter().eq(suffix.iter()) {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template changed trailing arguments",
                                ));
                            }
                            value(child.value)?
                        }
                        None => {
                            if !suffix.is_empty() {
                                return Err(rejected(
                                    DiagnosticCode::InvalidProof,
                                    "runtime template dropped trailing arguments",
                                ));
                            }
                            match result {
                                Expected::Value(id) => value(id)?,
                                Expected::Ground(id) => RuntimeValue::Ground(id),
                            }
                        }
                    }
                }
                Rule::Reassociate(child) => {
                    let original = proof_call(proof)?;
                    let RuntimeValue::Ground(mut head) = value(original.function)? else {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime reassociation head",
                        ));
                    };
                    let mut prefix = Vec::new();
                    while let Node::App(f, a) = self.pool.store.node(head) {
                        spend(budget, 1)?;
                        prefix.push(*a);
                        head = *f;
                    }
                    let p = premise(&self.proofs, at, *child)?;
                    let call = proof_call(p)?;
                    if prefix.is_empty()
                        || value(call.function)? != RuntimeValue::Ground(head)
                        || call.arguments.len() != prefix.len() + original.arguments.len()
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime reassociation arity/head",
                        ));
                    }
                    for (expected, actual) in prefix.iter().rev().zip(call.arguments.iter()) {
                        spend(budget, 1)?;
                        if value(*actual)? != RuntimeValue::Ground(*expected) {
                            return Err(rejected(
                                DiagnosticCode::InvalidProof,
                                "runtime reassociation operand",
                            ));
                        }
                    }
                    if !call
                        .arguments
                        .suffix(prefix.len())
                        .iter()
                        .eq(original.arguments.iter())
                    {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "runtime reassociation tail",
                        ));
                    }
                    value(p.value)?
                }
            };
            if claimed != actual {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "runtime forged conversion endpoint",
                ));
            }
        }
        value(self.proofs[root_proof].value)
    }
}

pub(super) fn normalize(source: &TypedSource<'_, '_>, budget: &mut usize) -> Result<Term> {
    let mut search = Search::new(source, budget)?;
    let root = Goal::Eval(search.pool.captured(Syntax::Source(source.root), 0));
    let proof = search.eval(root, budget)?;
    let value = search.replay(root, proof, budget)?;
    let RuntimeValue::Ground(id) = value else {
        return Err(declined(
            DiagnosticCode::UnsupportedFeature,
            "runtime cannot quote a captured lambda endpoint",
        ));
    };
    // This final native link is itself checked by both kernels. The preceding
    // replay established the original source's conversion to this compact id.
    search.finish(id, budget)
}

#[cfg(test)]
#[path = "runtime_conversion/replay_controls.rs"]
mod replay_controls;

#[cfg(test)]
#[path = "runtime_conversion/finish_controls.rs"]
mod finish_controls;

#[path = "runtime_conversion/runtime_template.rs"]
mod runtime_template;

#[cfg(test)]
#[path = "runtime_conversion/adversarial_bounds_controls.rs"]
mod adversarial_bounds_controls;

#[cfg(test)]
#[path = "runtime_conversion/template_execution_controls.rs"]
mod template_execution_controls;

#[cfg(test)]
#[path = "runtime_conversion/template_replay_controls.rs"]
mod template_replay_controls;

#[cfg(test)]
#[path = "runtime_conversion/source_fusion_controls.rs"]
mod source_fusion_controls;

#[cfg(test)]
#[path = "runtime_conversion/source_fusion_replay_controls.rs"]
mod source_fusion_replay_controls;

#[cfg(test)]
#[path = "runtime_conversion/composition_controls.rs"]
mod composition_controls;

#[cfg(test)]
#[path = "runtime_conversion/composition_replay_controls.rs"]
mod composition_replay_controls;

#[cfg(test)]
#[path = "runtime_conversion/composition_same_node_controls.rs"]
mod composition_same_node_controls;

#[cfg(test)]
#[path = "runtime_conversion/inline_call_controls.rs"]
mod inline_call_controls;

#[cfg(test)]
#[path = "runtime_conversion/inline_call_replay_controls.rs"]
mod inline_call_replay_controls;

#[cfg(test)]
#[path = "runtime_conversion/source_beta_view_controls.rs"]
mod source_beta_view_controls;

#[cfg(test)]
#[path = "runtime_conversion/source_apply_replay_controls.rs"]
mod source_apply_replay_controls;
