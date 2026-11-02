//! Checked parametric sharing for source expression DAGs.
use super::*;
use crate::term_store::{TermId, TermStore};
use std::collections::BTreeSet;

// Construction target for conservative helper cuts, not a raised kernel limit.
pub(super) const CLOSED_CHUNK_DEPTH: usize = 128;

pub(super) struct Plan {
    uses: HashMap<usize, usize>,
    sizes: HashMap<usize, usize>,
    depths: HashMap<usize, usize>,
    free: HashMap<usize, Vec<u64>>,
    /// The free source indices that occur more than once in a node's expanded tree. A DAG child
    /// reached twice doubles every occurrence below it, so this is `free` counted to two.
    repeated: HashMap<usize, Vec<u64>>,
    /// Whether a node's expanded tree holds a shared node, which the sharing path lifts into a
    /// helper that captures the locals it mentions.
    lifts: HashMap<usize, bool>,
    root: usize,
}
impl Plan {
    pub(super) fn new(importer: &Importer, root: usize, bound: usize) -> Result<Self> {
        let mut order = Vec::new();
        let mut states = HashMap::new();
        let mut uses = HashMap::from([(root, 1usize)]);
        let mut work = vec![(root, false)];
        while let Some((id, finish)) = work.pop() {
            if finish {
                states.insert(id, 2);
                order.push(id);
                continue;
            }
            match states.get(&id) {
                Some(2) => continue,
                Some(1) => {
                    return Err(rejected(
                        DiagnosticCode::MalformedExport,
                        "cyclic source expression DAG",
                    ))
                }
                _ => {}
            }
            let row = importer.exprs.get(&id).ok_or_else(|| {
                declined(DiagnosticCode::MalformedExport, "unknown expression index")
            })?;
            states.insert(id, 1);
            work.push((id, true));
            for child in record_children(row).into_iter().rev() {
                let child = index(child)?;
                *uses.entry(child).or_insert(0) += 1;
                work.push((child, false));
            }
        }
        let mut sizes = HashMap::new();
        let mut depths = HashMap::new();
        for id in &order {
            if importer.exprs[id].get("strVal").is_some() {
                let (size, depth) = strings::layout(&importer.exprs[id])?;
                sizes.insert(*id, size);
                depths.insert(*id, depth);
                continue;
            }
            let size = record_children(&importer.exprs[id])
                .into_iter()
                .try_fold(1usize, |size, c| {
                    Ok::<_, Verdict>(size.saturating_add(sizes[&index(c)?]))
                })?;
            sizes.insert(*id, size);
            let depth = record_children(&importer.exprs[id])
                .into_iter()
                .map(|c| index(c).map(|id| depths[&id]))
                .collect::<Result<Vec<usize>>>()?
                .into_iter()
                .max()
                .unwrap_or(0)
                + 1;
            depths.insert(*id, depth);
        }
        let mut free = HashMap::<usize, Vec<u64>>::new();
        let mut repeated = HashMap::<usize, Vec<u64>>::new();
        let mut lifts = HashMap::<usize, bool>::new();
        // A source binder path cannot contain more distinct source nodes than
        // the reachable acyclic graph. Reject impossible indices before sizing.
        let maximum_index = order.len().saturating_add(bound);
        for id in order {
            let row = &importer.exprs[&id];
            let mut bits = Vec::new();
            let mut twice = Vec::new();
            if let Some(var) = row.get("bvar") {
                let var = index(var)?;
                if var >= maximum_index {
                    return Err(rejected(
                        DiagnosticCode::MalformedExport,
                        "unbound source de Bruijn variable",
                    ));
                }
                bits.resize(var / 64 + 1, 0);
                bits[var / 64] |= 1u64 << (var % 64);
            } else if let Some((kind, binder)) = ["lam", "forallE", "letE"]
                .into_iter()
                .find_map(|kind| row.get(kind).map(|v| (kind, v)))
            {
                let domain = index(&binder["type"])?;
                accumulate(&mut bits, &mut twice, &free[&domain], &repeated[&domain]);
                if kind == "letE" {
                    let value = index(&binder["value"])?;
                    accumulate(&mut bits, &mut twice, &free[&value], &repeated[&value]);
                }
                let body = index(&binder["body"])?;
                accumulate(
                    &mut bits,
                    &mut twice,
                    &unshift(&free[&body]),
                    &unshift(&repeated[&body]),
                );
            } else {
                for child in record_children(row) {
                    let child = index(child)?;
                    accumulate(&mut bits, &mut twice, &free[&child], &repeated[&child]);
                }
            }
            while bits.last() == Some(&0) {
                bits.pop();
            }
            while twice.last() == Some(&0) {
                twice.pop();
            }
            let shared = uses.get(&id).copied().unwrap_or(0) > 1 && sizes[&id] >= 64;
            let lifted = shared
                || record_children(row)
                    .into_iter()
                    .map(|child| index(child).map(|child| lifts[&child]))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .any(|lifted| lifted);
            free.insert(id, bits);
            repeated.insert(id, twice);
            lifts.insert(id, lifted);
        }
        Ok(Self {
            uses,
            sizes,
            depths,
            free,
            repeated,
            lifts,
            root,
        })
    }
    /// How often a let binds: the occurrences of the variable a source `letE` introduces, counted in
    /// its body's expanded tree and saturated at [`Occurrences::Many`].
    pub(super) fn let_occurrences(&self, body: usize) -> crate::term_store::Occurrences {
        use crate::term_store::Occurrences;
        let bit = |bits: &[u64]| bits.first().is_some_and(|word| word & 1 != 0);
        if bit(&self.repeated[&body]) {
            Occurrences::Many
        } else if bit(&self.free[&body]) {
            Occurrences::Once
        } else {
            Occurrences::Zero
        }
    }
    pub(super) fn shared(&self, id: usize) -> bool {
        self.uses.get(&id).copied().unwrap_or(0) > 1 && self.sizes[&id] >= 64
    }
    pub(super) fn closed(&self, id: usize) -> bool {
        self.free[&id].is_empty()
    }
    pub(super) fn depth(&self, id: usize) -> usize {
        self.depths[&id]
    }
    pub(super) fn size(&self, id: usize) -> usize {
        self.sizes[&id]
    }
    fn free_indices(&self, id: usize) -> impl Iterator<Item = usize> + '_ {
        self.free[&id].iter().enumerate().flat_map(|(word, bits)| {
            (0..64).filter_map(move |bit| ((bits >> bit) & 1 != 0).then_some(word * 64 + bit))
        })
    }
}
fn union(target: &mut Vec<u64>, source: &[u64]) {
    target.resize(target.len().max(source.len()), 0);
    for (a, b) in target.iter_mut().zip(source) {
        *a |= b;
    }
}
/// Add one child's occurrence sets (`once`: free, `many`: repeated) into a node's, saturating at
/// two: an index already seen, or seen twice in the child, is repeated.
fn accumulate(seen: &mut Vec<u64>, twice: &mut Vec<u64>, once: &[u64], many: &[u64]) {
    let overlap = seen
        .iter()
        .zip(once)
        .map(|(a, b)| a & b)
        .collect::<Vec<_>>();
    union(twice, &overlap);
    union(twice, many);
    union(seen, once);
}
/// A binder body's indices as seen outside the binder: index 0 is the binder's own variable.
fn unshift(bits: &[u64]) -> Vec<u64> {
    (0..bits.len())
        .map(|i| (bits[i] >> 1) | bits.get(i + 1).copied().unwrap_or(0).wrapping_shl(63))
        .collect()
}
#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct Key {
    source: usize,
    telescope: TermId,
    free_slots: Vec<usize>,
}
pub(super) struct Capture {
    pub(super) key: Key,
    pub(super) bindings: Vec<(String, Term, Option<Term>)>,
    binding_nodes: usize,
}
/// What a shared-term materialization charge paid for. The ledger of every
/// category is reported with an exhausted allowance, so a resource decline says
/// where the declaration's allowance went instead of only where it ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Materialized {
    CaptureTelescope,
    CachedApplication,
    HelperAbstraction,
    ClosedLetWitness,
    CheckedValueCopy,
    ClosedChunk,
}
impl Materialized {
    const ALL: [Materialized; 6] = [
        Materialized::CaptureTelescope,
        Materialized::CachedApplication,
        Materialized::HelperAbstraction,
        Materialized::ClosedLetWitness,
        Materialized::CheckedValueCopy,
        Materialized::ClosedChunk,
    ];
    fn name(self) -> &'static str {
        match self {
            Materialized::CaptureTelescope => "capture_telescope",
            Materialized::CachedApplication => "cached_application",
            Materialized::HelperAbstraction => "helper_abstraction",
            Materialized::ClosedLetWitness => "closed_let_witness",
            Materialized::CheckedValueCopy => "checked_value_copy",
            Materialized::ClosedChunk => "closed_chunk",
        }
    }
}
pub(super) struct Sharing {
    pub(super) plan: Plan,
    pub(super) beta: Option<beta::Certificates>,
    store: TermStore,
    checked: HashMap<Key, Term>,
    levels: Vec<String>,
    ledger: [std::cell::Cell<usize>; 6],
}
impl Sharing {
    pub(super) fn new(plan: Plan, params: &HashSet<String>) -> Self {
        let mut levels = params.iter().cloned().collect::<Vec<_>>();
        levels.sort();
        Self {
            plan,
            beta: None,
            store: TermStore::new(),
            checked: HashMap::new(),
            levels,
            ledger: Default::default(),
        }
    }
    #[cfg(test)]
    pub(super) fn spent(&self, category: Materialized) -> usize {
        self.ledger[category as usize].get()
    }
    /// Whether the source let whose body is `body` is published as a let, its uses kept as
    /// references. The let is the sharing unless something else shares better:
    /// - a variable used more than once has its checked closed value shared through δ instead — a
    ///   checked helper when the value is wide — so the kernel never ζ-duplicates it;
    /// - a body holding a shared node lowers the let: the node is lifted into a helper whose
    ///   captured telescope copies every local binding it mentions, and a lowered closed let —
    ///   with the lets its value closes in turn — reaches that telescope as a checked value or
    ///   helper, never as a wide binding copied into each capture;
    /// - a declaration too deep for the kernel's depth door as written lowers its closed lets so
    ///   the chunks around their uses close and are cut into shallow checked helpers. No lowering
    ///   or cut adds more than a chunk to any path, which is the margin.
    pub(super) fn retains_let(&self, body: usize) -> bool {
        self.plan
            .depth(self.plan.root)
            .saturating_add(CLOSED_CHUNK_DEPTH)
            <= crate::type_checker::MAX_TERM_DEPTH
            && self.plan.let_occurrences(body) != crate::term_store::Occurrences::Many
            && !self.plan.lifts[&body]
    }
    /// Charge `nodes` for `category`, or decline with the whole ledger. A refused
    /// charge neither consumes nor refunds the caller's allowance.
    fn charge(&self, budget: &mut usize, nodes: usize, category: Materialized) -> Result<()> {
        match budget.checked_sub(nodes) {
            Some(remaining) => {
                *budget = remaining;
                let entry = &self.ledger[category as usize];
                entry.set(entry.get().saturating_add(nodes));
                Ok(())
            }
            None => {
                let ledger = Materialized::ALL.map(|c| self.ledger[c as usize].get());
                let spent = Materialized::ALL
                    .iter()
                    .zip(ledger)
                    .map(|(c, n)| format!("{}={n}", c.name()))
                    .collect::<Vec<_>>()
                    .join(" ");
                Err(declined(DiagnosticCode::ResourceLimit, format!(
                    "shared term materialization budget: requested {nodes} for {}, remaining {budget}; spent {spent}",
                    category.name())))
            }
        }
    }
    #[cfg(test)]
    fn capture(&mut self, id: usize, bound: &[(String, Term, Option<Term>)]) -> Result<Capture> {
        {
            let mut budget = usize::MAX;
            self.capture_budgeted(id, bound, &mut budget)
        }
    }
    pub(super) fn capture_budgeted(
        &mut self,
        id: usize,
        bound: &[(String, Term, Option<Term>)],
        budget: &mut usize,
    ) -> Result<Capture> {
        let names = bound
            .iter()
            .enumerate()
            .map(|(i, (n, _, _))| (n.as_str(), i))
            .collect::<HashMap<_, _>>();
        let mut selected = BTreeSet::new();
        let mut work = Vec::new();
        for index in self.plan.free_indices(id) {
            let position = bound.len().checked_sub(index + 1).ok_or_else(|| {
                rejected(
                    DiagnosticCode::MalformedExport,
                    "unbound source de Bruijn variable",
                )
            })?;
            if selected.insert(position) {
                work.push(position);
            }
        }
        while let Some(position) = work.pop() {
            let (_, ty, value) = &bound[position];
            for term in std::iter::once(ty).chain(value.iter()) {
                let term = self.store.intern(term);
                for name in self.store.free_vars(term) {
                    let earlier = *names.get(name).ok_or_else(|| {
                        rejected(
                            DiagnosticCode::MalformedExport,
                            "unbound variable in captured local telescope",
                        )
                    })?;
                    if earlier >= position {
                        return Err(rejected(
                            DiagnosticCode::MalformedExport,
                            "cyclic captured local telescope",
                        ));
                    }
                    if selected.insert(earlier) {
                        work.push(earlier);
                    }
                }
            }
        }
        let positions = selected.into_iter().collect::<Vec<_>>();
        let free_slots = self
            .plan
            .free_indices(id)
            .map(|index| {
                positions
                    .binary_search(&(bound.len() - index - 1))
                    .expect("selected free source variable")
            })
            .collect();
        let mut binding_nodes = 0usize;
        for position in &positions {
            let (_, ty, value) = &bound[*position];
            for term in std::iter::once(ty).chain(value.iter()) {
                let id = self.store.intern(term);
                binding_nodes = binding_nodes.saturating_add(
                    usize::try_from(self.store.tree_size(id)).unwrap_or(usize::MAX),
                );
            }
        }
        // Capture storage and alpha-key telescope each own a copy of every
        // annotation/value. Charge before either copy, including wrapper nodes.
        self.charge(
            budget,
            binding_nodes
                .saturating_mul(2)
                .saturating_add(positions.len())
                .saturating_add(1),
            Materialized::CaptureTelescope,
        )?;
        let bindings = positions
            .into_iter()
            .map(|i| bound[i].clone())
            .collect::<Vec<_>>();
        let mut telescope = Term::Hole;
        for (name, ty, value) in bindings.iter().rev() {
            telescope = if let Some(value) = value {
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(ty.clone()),
                    value: Box::new(value.clone()),
                    body: Box::new(telescope),
                }
            } else {
                Term::Pi {
                    param: name.clone(),
                    param_type: Box::new(ty.clone()),
                    body_type: Box::new(telescope),
                }
            };
        }
        let telescope = self.store.intern(&telescope);
        let telescope = self.store.alpha_id(telescope);
        Ok(Capture {
            key: Key {
                source: id,
                telescope,
                free_slots,
            },
            bindings,
            binding_nodes,
        })
    }
    fn apply_capture(head: Term, capture: &Capture) -> Term {
        capture
            .bindings
            .iter()
            .filter(|(_, _, v)| v.is_none())
            .fold(head, |f, (n, _, _)| apply(f, Term::Var(n.clone())))
    }
    pub(super) fn cached(&self, capture: &Capture, budget: &mut usize) -> Result<Option<Term>> {
        let Some(head) = self.checked.get(&capture.key) else {
            return Ok(None);
        };
        self.charge(
            budget,
            application_nodes(capture),
            Materialized::CachedApplication,
        )?;
        Ok(Some(Self::apply_capture(head.clone(), capture)))
    }
    pub(super) fn finish(
        &mut self,
        ctx: &mut Context,
        report: &mut ImportReport,
        capture: Capture,
        mut body: Term,
        budget: &mut usize,
    ) -> Result<Term> {
        // Closed helper abstraction plus its first application; typed bodies
        // themselves were charged by the ordinary source decoder.
        self.charge(
            budget,
            capture
                .binding_nodes
                .saturating_add(capture.bindings.len())
                .saturating_add(application_nodes(&capture))
                .saturating_add(1),
            Materialized::HelperAbstraction,
        )?;
        for (name, ty, value) in capture.bindings.iter().rev() {
            body = if let Some(value) = value {
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(ty.clone()),
                    value: Box::new(value.clone()),
                    body: Box::new(body),
                }
            } else {
                Term::Lambda {
                    param: name.clone(),
                    param_type: Box::new(ty.clone()),
                    body: Box::new(body),
                }
            };
        }
        let captured_parameters = capture
            .bindings
            .iter()
            .filter(|(_, _, v)| v.is_none())
            .count();
        let head =
            self.register_checked(ctx, report, capture.key.source, captured_parameters, body)?;
        self.checked.insert(capture.key.clone(), head.clone());
        Ok(Self::apply_capture(head, &capture))
    }

    fn closed(&mut self, term: &Term) -> bool {
        let id = self.store.intern(term);
        self.store.free_vars(id).is_empty()
            && !self
                .store
                .shapes(id)
                .contains(crate::term_store::NodeShape::Hole)
    }

    fn nodes(&mut self, term: &Term) -> usize {
        let id = self.store.intern(term);
        usize::try_from(self.store.tree_size(id)).unwrap_or(usize::MAX)
    }

    /// Only this successful gate authorizes source-let substitution. Ordinary
    /// binding values are never implicitly treated as checked certificates.
    pub(super) fn check_closed_let(
        &mut self,
        ctx: &Context,
        ty: &Term,
        value: &Term,
        budget: &mut usize,
    ) -> Result<bool> {
        if !self.closed(ty) || !self.closed(value) {
            return Ok(false);
        }
        let sort = lean_type(ctx, ty)?;
        if !matches!(normal(ctx, &sort)?, Term::Sort(_)) {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "closed let annotation is not a type",
            ));
        }
        let nodes = self
            .nodes(ty)
            .saturating_add(self.nodes(value))
            .saturating_add(2);
        self.charge(budget, nodes, Materialized::ClosedLetWitness)?;
        let witness = Term::Let {
            name: "arena.checked.let".into(),
            ty: Box::new(ty.clone()),
            value: Box::new(value.clone()),
            body: Box::new(Term::Var("arena.checked.let".into())),
        };
        let inferred = lean_type(ctx, &witness)?;
        let independent = crate::recheck(ctx, &witness)
            .map_err(|e| independent_verdict(&e, "independent closed-let check"))?;
        exact(ctx, &inferred, ty)?;
        exact(ctx, &independent, ty)?;
        Ok(true)
    }

    pub(super) fn copy_checked_value(&mut self, value: &Term, budget: &mut usize) -> Result<Term> {
        let nodes = self.nodes(value);
        self.charge(budget, nodes, Materialized::CheckedValueCopy)?;
        Ok(value.clone())
    }

    /// Keep a wide checked let value as a transparent definition before its
    /// uses or captured telescope can duplicate it. The ordinary helper gate
    /// independently checks its full body and retains all dependencies.
    pub(super) fn share_closed_value(
        &mut self,
        ctx: &mut Context,
        report: &mut ImportReport,
        source: usize,
        value: Term,
        budget: &mut usize,
    ) -> Result<Term> {
        if self.nodes(&value) < 64 {
            return Ok(value);
        }
        self.cut_closed(ctx, report, source, value, budget)
            .map(|(term, _)| term)
    }

    /// Closed chunks may depend on substituted let values. They deliberately
    /// do not enter the source/telescope template cache: source ID alone would
    /// not identify their meaning under different substitution environments.
    pub(super) fn cut_closed(
        &mut self,
        ctx: &mut Context,
        report: &mut ImportReport,
        source: usize,
        body: Term,
        budget: &mut usize,
    ) -> Result<(Term, bool)> {
        if !self.closed(&body) {
            return Ok((body, false));
        }
        self.charge(budget, 2, Materialized::ClosedChunk)?; // registered helper reference and returned occurrence
        Ok((self.register_checked(ctx, report, source, 0, body)?, true))
    }

    fn register_checked(
        &self,
        ctx: &mut Context,
        report: &mut ImportReport,
        source: usize,
        captured_parameters: usize,
        body: Term,
    ) -> Result<Term> {
        let ty = lean_type(ctx, &body)?;
        let independently_inferred = crate::recheck(ctx, &body)
            .map_err(|e| independent_verdict(&e, "independent shared-template check"))?;
        exact(ctx, &ty, &independently_inferred).map_err(|_| {
            declined(
                DiagnosticCode::RecheckRefused,
                "independent shared-template type disagreement",
            )
        })?;
        let mut serial = report.internal_helpers.len();
        let name = loop {
            // Source names are structurally encoded with a leading slash.
            // This namespace is inaccessible through source constant records.
            let name = format!("arena.internal.dag.{serial}");
            // get_global deliberately excludes universe-polymorphic definitions.
            // Reserve across every checked declaration registry and pending name,
            // including helpers committed by earlier source declarations.
            if ctx.get_global(&name).is_none()
                && ctx.poly_declaration(&name).is_none()
                && !ctx
                    .pending_obligations()
                    .iter()
                    .any(|(pending, _)| *pending == name)
            {
                break name;
            }
            serial += 1;
        };
        if self.levels.is_empty() {
            ctx.declare(&name, ty, body).map_err(kernel)?;
        } else {
            ctx.declare_universe_poly(&name, self.levels.clone(), ty, body)
                .map_err(kernel)?;
        }
        if ctx
            .pending_obligations()
            .iter()
            .any(|(pending, _)| *pending == name)
        {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "shared helper declaration retained a pending obligation",
            ));
        }
        let head = if self.levels.is_empty() {
            Term::Global(name.clone())
        } else {
            Term::Const {
                name: name.clone(),
                levels: self.levels.iter().cloned().map(Universe::Var).collect(),
            }
        };
        report.internal_helpers.push(InternalHelper {
            name,
            source_expression: source,
            level_parameters: self.levels.clone(),
            captured_parameters,
        });
        Ok(head)
    }
}

fn application_nodes(capture: &Capture) -> usize {
    capture
        .bindings
        .iter()
        .filter(|(_, _, value)| value.is_none())
        .count()
        .saturating_mul(2)
        .saturating_add(1)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/dag/tests.rs"]
mod tests;
