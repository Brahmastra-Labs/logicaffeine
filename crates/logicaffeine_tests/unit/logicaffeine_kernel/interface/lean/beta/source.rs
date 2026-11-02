//! Plan-free original-source typing for the experimental conversion prerequisite.
//!
//! This wrapper borrows one immutable importer. Numeric source IDs, its cloned
//! Context, and its universe parameter scope therefore cannot change underneath
//! either the source or closed-domain certificate caches. It stores typing
//! evidence, never equality of captured values.
use super::*;

const SMALL_SOURCE_NODES: usize = 256;
const SOURCE_DEPTH: usize = 128;
const LEVEL_NODES: usize = 256;
const PAYLOAD_BYTES: usize = 4096;
const TYPE_DEPTH: usize = 256;

pub(in super::super) struct SourceCertificates<'source, 'input> {
    importer: &'source Importer<'input>,
    certificates: Certificates,
    domains: HashMap<usize, TermId>,
    roots: HashMap<usize, TermId>,
}

fn charge(budget: &mut usize, work: usize) -> Result<()> {
    Certificates::charge(budget, work)
}
fn push<T>(stack: &mut Vec<T>, value: T, budget: &mut usize) -> Result<()> {
    // One frame pays for allocation and its bounded constant processing work.
    // Popping this frame is deliberately not charged a second time.
    charge(budget, 1)?;
    stack.push(value);
    Ok(())
}
fn text_payload(text: &str, budget: &mut usize) -> Result<()> {
    if text.len() > PAYLOAD_BYTES {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "source certificate text payload bound",
        ));
    }
    charge(budget, text.len())
}
fn level_payload(
    root: &Universe,
    params: Option<&HashSet<String>>,
    budget: &mut usize,
) -> Result<()> {
    let mut work = Vec::new();
    push(&mut work, (root, 1usize), budget)?;
    let mut nodes = 0;
    while let Some((level, depth)) = work.pop() {
        nodes += 1;
        if nodes > LEVEL_NODES || depth >= SOURCE_DEPTH {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "source certificate universe payload bound",
            ));
        }
        match level {
            Universe::Var(name) => {
                text_payload(name, budget)?;
                if params.is_some_and(|params| !params.contains(name)) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "unbound universe parameter",
                    ));
                }
            }
            Universe::Succ(a) => push(&mut work, (a.as_ref(), depth + 1), budget)?,
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                push(&mut work, (b.as_ref(), depth + 1), budget)?;
                push(&mut work, (a.as_ref(), depth + 1), budget)?;
            }
            Universe::SProp | Universe::Prop | Universe::Type(_) => {}
        }
    }
    Ok(())
}

/// Admit types before interning them. Kernel inference/normalization keeps its
/// own limits; this is the wrapper's borrowed admission for newly owned output.
fn admitted_type_nodes(term: &Term, budget: &mut usize) -> Result<usize> {
    let mut work = Vec::new();
    push(&mut work, (term, 1usize), budget)?;
    let mut nodes = 0;
    while let Some((term, depth)) = work.pop() {
        nodes += 1;
        if depth > TYPE_DEPTH {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "source certificate inferred type depth",
            ));
        }
        match term {
            Term::Sort(level) => level_payload(level, None, budget)?,
            Term::Var(name) | Term::Global(name) => text_payload(name, budget)?,
            Term::Const { name, levels } => {
                text_payload(name, budget)?;
                if levels.len() > LEVEL_NODES {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "source certificate inferred universe arity",
                    ));
                }
                charge(budget, levels.len())?;
                for level in levels {
                    level_payload(level, None, budget)?;
                }
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            }
            | Term::Lambda {
                param,
                param_type,
                body: body_type,
            } => {
                text_payload(param, budget)?;
                push(&mut work, (body_type.as_ref(), depth + 1), budget)?;
                push(&mut work, (param_type.as_ref(), depth + 1), budget)?;
            }
            Term::App(f, a) => {
                push(&mut work, (a.as_ref(), depth + 1), budget)?;
                push(&mut work, (f.as_ref(), depth + 1), budget)?;
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                text_payload(name, budget)?;
                push(&mut work, (body.as_ref(), depth + 1), budget)?;
                push(&mut work, (value.as_ref(), depth + 1), budget)?;
                push(&mut work, (ty.as_ref(), depth + 1), budget)?;
            }
            // A refusal is conservative: no new typing rule is invented for
            // computed type syntax outside this small certificate fragment.
            _ => {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "source certificate inferred type outside bounded fragment",
                ))
            }
        }
    }
    Ok(nodes)
}

/// Pay for the expanded payload that extraction will clone, before extraction.
/// Repeated handles are deliberately revisited: extraction expands each use.
/// Kernel-internal inference and normalization retain their own resource limits.
fn admit_store_copy(store: &TermStore, root: TermId, budget: &mut usize) -> Result<()> {
    use crate::term_store::{LevelView, Node};
    if store.depth(root) as usize > TYPE_DEPTH || store.tree_size(root) > *budget as u64 {
        return Err(declined(
            DiagnosticCode::ResourceLimit,
            "source certificate copied type bound",
        ));
    }
    fn level(
        store: &TermStore,
        root: crate::term_store::LevelId,
        budget: &mut usize,
    ) -> Result<()> {
        let mut work = Vec::new();
        push(&mut work, (root, 1usize), budget)?;
        let mut nodes = 0usize;
        while let Some((id, depth)) = work.pop() {
            nodes += 1;
            if nodes > LEVEL_NODES || depth >= SOURCE_DEPTH {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "source certificate copied universe bound",
                ));
            }
            // Frame admission pays for inspection; this unit pays for the
            // corresponding universe node cloned by extract_level.
            charge(budget, 1)?;
            match store.level_view(id) {
                LevelView::Var(name) => text_payload(name, budget)?,
                LevelView::Succ(a) => push(&mut work, (a, depth + 1), budget)?,
                LevelView::Max(a, b) | LevelView::IMax(a, b) => {
                    push(&mut work, (b, depth + 1), budget)?;
                    push(&mut work, (a, depth + 1), budget)?;
                }
                LevelView::SProp | LevelView::Prop | LevelView::Type(_) => {}
            }
        }
        Ok(())
    }
    let mut work = Vec::new();
    push(&mut work, root, budget)?;
    while let Some(id) = work.pop() {
        match store.node(id) {
            Node::Sort(u) => level(store, *u, budget)?,
            Node::Var(name) | Node::Global(name) => text_payload(store.name(*name), budget)?,
            Node::Const { name, levels } => {
                text_payload(store.name(*name), budget)?;
                if levels.len() > LEVEL_NODES {
                    return Err(declined(
                        DiagnosticCode::ResourceLimit,
                        "source certificate copied universe arity",
                    ));
                }
                charge(budget, levels.len())?;
                for u in levels.iter() {
                    level(store, *u, budget)?;
                }
            }
            Node::Pi {
                param,
                param_type,
                body_type,
            }
            | Node::Lambda {
                param,
                param_type,
                body: body_type,
            } => {
                text_payload(store.name(*param), budget)?;
                push(&mut work, *body_type, budget)?;
                push(&mut work, *param_type, budget)?;
            }
            Node::App(f, a) => {
                push(&mut work, *a, budget)?;
                push(&mut work, *f, budget)?;
            }
            Node::Let {
                name,
                ty,
                value,
                body,
            } => {
                text_payload(store.name(*name), budget)?;
                push(&mut work, *body, budget)?;
                push(&mut work, *value, budget)?;
                push(&mut work, *ty, budget)?;
            }
            _ => {
                return Err(declined(
                    DiagnosticCode::UnsupportedFeature,
                    "source certificate copied type outside bounded fragment",
                ))
            }
        }
    }
    // Certificates::copy separately charges the expanded Term node count.
    Ok(())
}
fn admit_rule_intern(term: &Term, budget: &mut usize) -> Result<()> {
    let nodes = admitted_type_nodes(term, budget)?;
    charge(budget, nodes)
}
const SOURCE_RULE_ADMISSION: RuleAdmission = RuleAdmission {
    before_copy: admit_store_copy,
    before_intern: admit_rule_intern,
};

impl<'source, 'input> SourceCertificates<'source, 'input> {
    pub(in super::super) fn new(
        importer: &'source Importer<'input>,
        params: &HashSet<String>,
        budget: &mut usize,
    ) -> Result<Self> {
        // Certificates::new clones this set. Account for bucket capacity as
        // well as every string before that clone; the Context clone is shared.
        charge(budget, 1)?;
        charge(budget, params.capacity())?;
        for name in params {
            text_payload(name, budget)?;
        }
        let certificates = Certificates::new(importer.ctx.clone(), params, &[], budget)?;
        Ok(Self {
            importer,
            certificates,
            domains: HashMap::new(),
            roots: HashMap::new(),
        })
    }

    fn row(&self, id: usize) -> Result<&Value> {
        self.importer.exprs.get(&id).ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                "unknown source certificate index",
            )
        })
    }

    fn source_level(&self, id: &Value, budget: &mut usize) -> Result<()> {
        let id = index(id)?;
        let level = self.importer.levels.get(&id).ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                "unknown source certificate level",
            )
        })?;
        // The ordinary decoder also consults this set. Admit its traversal and
        // avoid indexing missing auxiliary metadata even in synthetic inputs.
        let variables = self.importer.level_vars.get(&id).ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                "missing source level parameter metadata",
            )
        })?;
        charge(budget, variables.len())?;
        for name in variables {
            text_payload(name, budget)?;
        }
        level_payload(level, Some(&self.certificates.params), budget)
    }

    fn atom_payload(&self, row: &Value, budget: &mut usize) -> Result<()> {
        if let Some(level) = row.get("sort") {
            return self.source_level(level, budget);
        }
        if let Some(constant) = row.get("const") {
            let name = self
                .importer
                .names
                .get(&index(&constant["name"])?)
                .ok_or_else(|| {
                    declined(
                        DiagnosticCode::MalformedExport,
                        "unknown source certificate name",
                    )
                })?;
            text_payload(name, budget)?;
            let levels = array(&constant["us"])?;
            if levels.len() > LEVEL_NODES {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "source certificate explicit universe arity bound",
                ));
            }
            charge(budget, levels.len())?;
            for level in levels {
                self.source_level(level, budget)?;
            }
            return Ok(());
        }
        if let Some(number) = row.get("natVal") {
            let digits = number.as_str().ok_or_else(|| {
                declined(DiagnosticCode::MalformedExport, "malformed natural literal")
            })?;
            // The existing parser still validates and charges parsing itself.
            return text_payload(digits, budget);
        }
        Err(declined(
            DiagnosticCode::UnsupportedFeature,
            "unsupported source certificate atom",
        ))
    }

    /// Local, expanded-tree admission, never an allocation-sized free-index
    /// bitmap. Both the annotation and initializer of every Let are traversed.
    fn admit_closed(
        &self,
        root: usize,
        limit: usize,
        outer_active: &HashSet<usize>,
        budget: &mut usize,
    ) -> Result<bool> {
        #[derive(Clone, Copy)]
        enum Kind {
            App,
            Binder,
            Let,
            Metadata,
        }
        enum Frame {
            Visit(usize, usize, usize),
            Finish(usize, Kind),
        }
        let mut work = Vec::new();
        let mut sizes = Vec::<usize>::new();
        let mut active = HashSet::new();
        let mut nodes = 0usize;
        push(&mut work, Frame::Visit(root, 1, 0), budget)?;
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Visit(id, depth, bound) => {
                    nodes += 1;
                    if nodes > limit || depth >= SOURCE_DEPTH {
                        return Ok(false);
                    }
                    // The caller's current node is the root of this decoding
                    // attempt; all other active ancestors remain forbidden.
                    if active.contains(&id)
                        || (outer_active.contains(&id) && !(id == root && depth == 1))
                    {
                        return Err(rejected(
                            DiagnosticCode::MalformedExport,
                            "cyclic original source certificate",
                        ));
                    }
                    let row = self.row(id)?;
                    active.insert(id);
                    if let Some(app) = row.get("app") {
                        push(&mut work, Frame::Finish(id, Kind::App), budget)?;
                        push(
                            &mut work,
                            Frame::Visit(index(&app["arg"])?, depth + 1, bound),
                            budget,
                        )?;
                        push(
                            &mut work,
                            Frame::Visit(index(&app["fn"])?, depth + 1, bound),
                            budget,
                        )?;
                    } else if let Some(meta) = row.get("mdata") {
                        push(&mut work, Frame::Finish(id, Kind::Metadata), budget)?;
                        push(
                            &mut work,
                            Frame::Visit(index(&meta["expr"])?, depth + 1, bound),
                            budget,
                        )?;
                    } else if let Some((is_let, binder)) =
                        [("forallE", false), ("lam", false), ("letE", true)]
                            .into_iter()
                            .find_map(|(name, is_let)| row.get(name).map(|b| (is_let, b)))
                    {
                        let body_bound = bound.checked_add(1).ok_or_else(|| {
                            declined(
                                DiagnosticCode::ResourceLimit,
                                "source certificate binder count overflow",
                            )
                        })?;
                        push(
                            &mut work,
                            Frame::Finish(id, if is_let { Kind::Let } else { Kind::Binder }),
                            budget,
                        )?;
                        push(
                            &mut work,
                            Frame::Visit(index(&binder["body"])?, depth + 1, body_bound),
                            budget,
                        )?;
                        if is_let {
                            push(
                                &mut work,
                                Frame::Visit(index(&binder["value"])?, depth + 1, bound),
                                budget,
                            )?;
                        }
                        push(
                            &mut work,
                            Frame::Visit(index(&binder["type"])?, depth + 1, bound),
                            budget,
                        )?;
                    } else {
                        if let Some(variable) = row.get("bvar") {
                            if index(variable)? >= bound {
                                return Ok(false);
                            }
                        } else {
                            self.atom_payload(row, budget)?;
                        }
                        active.remove(&id);
                        sizes.push(1);
                    }
                }
                Frame::Finish(id, kind) => {
                    let arity = match kind {
                        Kind::Metadata => 1,
                        Kind::Let => 3,
                        _ => 2,
                    };
                    let start = sizes.len().checked_sub(arity).ok_or_else(|| {
                        declined(
                            DiagnosticCode::UnsupportedFeature,
                            "source admission frame shape",
                        )
                    })?;
                    // term_core clones a binder's original domain into its local
                    // telescope; Let also copies its initializer. Pay before decoding.
                    if matches!(kind, Kind::Binder | Kind::Let) {
                        charge(budget, sizes[start])?;
                    }
                    if matches!(kind, Kind::Let) {
                        charge(budget, sizes[start + 1])?;
                    }
                    let size = sizes[start..].iter().try_fold(1usize, |sum, n| {
                        sum.checked_add(*n).ok_or_else(|| {
                            declined(
                                DiagnosticCode::ResourceLimit,
                                "source expanded size overflow",
                            )
                        })
                    })?;
                    sizes.truncate(start);
                    sizes.push(size);
                    active.remove(&id);
                }
            }
        }
        Ok(sizes.len() == 1)
    }

    fn decode_closed(
        &self,
        id: usize,
        limit: usize,
        active: &HashSet<usize>,
        budget: &mut usize,
    ) -> Result<Option<Term>> {
        if !self.admit_closed(id, limit, active, budget)? {
            return Ok(None);
        }
        // Existing decoding preserves all ordinary constant, universe, natural
        // literal, and binder rules. It charges its actual materialized visits.
        Ok(Some(self.importer.term(
            &serde_json::json!(id),
            &mut Vec::new(),
            &self.certificates.params,
            budget,
        )?))
    }

    fn inferred_id(&mut self, ty: &Term, budget: &mut usize) -> Result<Option<TermId>> {
        let nodes = admitted_type_nodes(ty, budget)?;
        charge(budget, nodes)?; // the subsequent TermStore interning traversal
        Ok(self.certificates.closed_id(ty))
    }

    fn domain(
        &mut self,
        id: usize,
        active: &HashSet<usize>,
        budget: &mut usize,
    ) -> Result<Option<TermId>> {
        if active.contains(&id) {
            return Err(rejected(
                DiagnosticCode::MalformedExport,
                "cyclic source lambda domain",
            ));
        }
        if let Some(id) = self.domains.get(&id) {
            return Ok(Some(*id));
        }
        let Some(domain) = self.decode_closed(id, SMALL_SOURCE_NODES, active, budget)? else {
            return Ok(None);
        };
        let nodes = admitted_type_nodes(&domain, budget)?;
        charge(budget, nodes)?;
        let checked = self.certificates.type_id(&domain)?;
        if let Some(ty) = checked {
            self.domains.insert(id, ty);
        }
        Ok(checked)
    }

    pub(in super::super) fn certify(&mut self, root: usize, budget: &mut usize) -> Result<bool> {
        enum Frame {
            Visit(usize, EnvId),
            App(usize, EnvId),
            Lambda(usize, EnvId, TermId),
            Metadata(usize, EnvId),
        }
        let mut work = Vec::new();
        let mut values = Vec::<Option<TermId>>::new();
        let mut active = HashSet::<usize>::new();
        push(&mut work, Frame::Visit(root, 0), budget)?;
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Visit(id, env) => {
                    if active.contains(&id) {
                        return Err(rejected(
                            DiagnosticCode::MalformedExport,
                            "cyclic original source certificate",
                        ));
                    }
                    let row = self.row(id)?;
                    let atom = row.get("sort").is_some()
                        || row.get("const").is_some()
                        || row.get("natVal").is_some();
                    let key_env = if atom { 0 } else { env };
                    if let Some(ty) = self.certificates.source.get(&(id, key_env)) {
                        values.push(*ty);
                        continue;
                    }
                    active.insert(id);
                    if let Some(app) = row.get("app") {
                        let function = index(&app["fn"])?;
                        let argument = index(&app["arg"])?;
                        push(&mut work, Frame::App(id, key_env), budget)?;
                        push(&mut work, Frame::Visit(argument, env), budget)?;
                        push(&mut work, Frame::Visit(function, env), budget)?;
                    } else if let Some(meta) = row.get("mdata") {
                        let child = index(&meta["expr"])?;
                        push(&mut work, Frame::Metadata(id, key_env), budget)?;
                        push(&mut work, Frame::Visit(child, env), budget)?;
                    } else if let Some(lambda) = row.get("lam") {
                        let domain_source = index(&lambda["type"])?;
                        let body = index(&lambda["body"])?;
                        if let Some(domain) = self.domain(domain_source, &active, budget)? {
                            let child_env = self.certificates.extend(env, domain, budget)?;
                            push(&mut work, Frame::Lambda(id, key_env, domain), budget)?;
                            push(&mut work, Frame::Visit(body, child_env), budget)?;
                        } else {
                            self.certificates.source.insert((id, key_env), None);
                            values.push(None);
                            active.remove(&id);
                        }
                    } else if let Some(variable) = row.get("bvar") {
                        let ty = self
                            .certificates
                            .lookup(env, index(variable)?, budget)?
                            .ok_or_else(|| {
                                rejected(
                                    DiagnosticCode::MalformedExport,
                                    "free source certificate variable",
                                )
                            })?;
                        self.certificates.source.insert((id, key_env), Some(ty));
                        values.push(Some(ty));
                        active.remove(&id);
                    } else {
                        // Closed atoms, and small original closed Lets/Pis, use
                        // the original decoder and BOTH existing kernels.
                        let result = if let Some(term) =
                            self.decode_closed(id, SMALL_SOURCE_NODES, &active, budget)?
                        {
                            let ty = self.certificates.both(&term)?;
                            self.inferred_id(&ty, budget)?
                        } else {
                            None
                        };
                        self.certificates.source.insert((id, key_env), result);
                        values.push(result);
                        active.remove(&id);
                    }
                }
                Frame::App(id, env) => {
                    let argument = values.pop().expect("source argument certificate");
                    let function = values.pop().expect("source function certificate");
                    let result = match (function, argument) {
                        (Some(f), Some(a)) => self.certificates.app_rule_admitted(
                            f,
                            a,
                            budget,
                            Some(SOURCE_RULE_ADMISSION),
                        )?,
                        _ => None,
                    };
                    self.certificates.source.insert((id, env), result);
                    values.push(result);
                    active.remove(&id);
                }
                Frame::Lambda(id, env, domain) => {
                    let result = values
                        .pop()
                        .expect("source lambda body certificate")
                        .map(|body| {
                            self.certificates.lambda_rule_admitted(
                                domain,
                                body,
                                budget,
                                Some(SOURCE_RULE_ADMISSION),
                            )
                        })
                        .transpose()?;
                    self.certificates.source.insert((id, env), result);
                    values.push(result);
                    active.remove(&id);
                }
                Frame::Metadata(id, env) => {
                    let result = *values.last().expect("source metadata certificate");
                    self.certificates.source.insert((id, env), result);
                    active.remove(&id);
                }
            }
        }
        if values.len() != 1 {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "source certificate conclusion shape",
            ));
        }
        let mut result = values.pop().flatten();
        if result.is_none() {
            // Preserve the previous small-original-tree fallback, including the
            // 257-node domain boundary. This checks the ORIGINAL whole term.
            let limit = *budget;
            if let Some(original) = self.decode_closed(root, limit, &HashSet::new(), budget)? {
                let ty = self.certificates.both(&original)?;
                result = self.inferred_id(&ty, budget)?;
            }
        }
        if let Some(ty) = result {
            charge(budget, 1)?;
            self.roots.insert(root, ty);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Bounded source syntax only; successful root typing and trace replay are
    /// separate prerequisites and this method is not an acceptance certificate.
    pub(in super::super) fn closed_readback(&self, id: usize, budget: &mut usize) -> Result<Term> {
        let limit = *budget;
        self.decode_closed(id, limit, &HashSet::new(), budget)?
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::UnsupportedFeature,
                    "source endpoint outside bounded closed readback",
                )
            })
    }

    /// Copy the actual certified root type, never a predicted type. Refuse a
    /// large/deep expansion before TermStore::extract allocates it.
    pub(in super::super) fn root_type(&self, root: usize, budget: &mut usize) -> Result<Term> {
        let ty = *self.roots.get(&root).ok_or_else(|| {
            declined(
                DiagnosticCode::UnsupportedFeature,
                "source root has not been certified",
            )
        })?;
        let nodes = self.certificates.store.tree_size(ty);
        if nodes > *budget as u64
            || nodes > SMALL_SOURCE_NODES as u64
            || self.certificates.store.depth(ty) as usize >= SOURCE_DEPTH
        {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "certified source root type exceeds bounded readback",
            ));
        }
        self.certificates
            .copy_admitted(ty, budget, Some(SOURCE_RULE_ADMISSION))
    }
}

#[cfg(test)]
#[path = "source/resource_controls.rs"]
mod resource_controls;
