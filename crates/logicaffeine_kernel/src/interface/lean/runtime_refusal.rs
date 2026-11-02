//! Refusal-only checking of acyclic unsafe definitions under local assumptions.
//! No quarantined declaration is registered, and this module returns no kernel environment.

use super::*;
use crate::recursor::bounded::{
    instantiate_universes_bounded, Construction, RecursorBudget, RecursorGenerationError,
};
use crate::term_store::{LevelId, Node, TermId};
use logicaffeine_base::Symbol;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const NODE_BUDGET: usize = 100_000;
const DEPTH_BUDGET: usize = 256;
const ESCAPES: [&str; 3] = ["sorryAx", "lcProof", "Lean.trustCompiler"];

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckScope {
    QuarantinedConditionalJudgment,
}

#[derive(Debug, Serialize)]
struct CheckedInstance {
    declaration: String,
    universe_arguments: Vec<String>,
    local_variable: String,
    declared_type: String,
    assumption: bool,
}

/// Evidence about a conditional judgment, not a proof of the unsafe target.
#[derive(Debug, Serialize)]
pub(super) struct ConditionalAudit {
    schema: &'static str,
    original_target: String,
    scope: CheckScope,
    original_target_proved: bool,
    original_target_importable: bool,
    unsafe_globals_registered: bool,
    universe_parameters: Vec<String>,
    conditional_type: String,
    instances: Vec<CheckedInstance>,
    safe_environment: Value,
    /// Axiom leaves of the original source graph, with Lean CollectAxioms leaf semantics.
    /// The explicit `instances` separately disclose every conditional local assumption.
    pub(super) assumptions: Vec<String>,
    #[serde(skip)]
    pub(super) taint: Vec<taint::TaintPath>,
}

impl ConditionalAudit {
    pub(super) fn to_json(&self) -> Value {
        // TaintPath's public representation is rendered explicitly; it is diagnostic evidence.
        let mut value = serde_json::to_value(self).expect("conditional audit is JSON");
        value["taint"] = serde_json::json!(self
            .taint
            .iter()
            .map(|path| { serde_json::json!({"assumption": path.assumption, "path": path.path}) })
            .collect::<Vec<_>>());
        value
    }
}

#[derive(Clone)]
struct Source {
    kind: intake::DeclarationKind,
    record: Value,
    quarantined: bool,
}

/// None leaves the ordinary importer in charge. Some is always a refusal-only conditional audit.
pub(super) fn check(
    export: &str,
    target: &str,
    read: &intake::Intake,
) -> Result<Option<ConditionalAudit>> {
    // Explicit diagnostic-only generation policy, separate from source export byte limits.
    let mut budget = RecursorBudget {
        max_depth: DEPTH_BUDGET,
        max_tree_nodes: NODE_BUDGET as u64,
        max_store_nodes: NODE_BUDGET,
        work: 1_000_000_000,
        allocation: 1_000_000,
        materialized_nodes: 1_000_000,
    };
    crate::term_store::grow_stack_for_depth(DEPTH_BUDGET, || {
        check_with_budget(export, target, read, &mut budget)
    })
}

fn check_with_budget(
    export: &str,
    target: &str,
    read: &intake::Intake,
    budget: &mut RecursorBudget,
) -> Result<Option<ConditionalAudit>> {
    let declaration = read
        .declaration(target)
        .ok_or_else(|| rejected(DiagnosticCode::MissingDeclaration, "missing runtime target"))?;
    if declaration.kind != intake::DeclarationKind::Definition {
        return Ok(None);
    }
    let target_row: Value =
        serde_json::from_str(export.lines().nth(declaration.line - 1).ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                "missing runtime target record",
            )
        })?)
        .map_err(|error| declined(DiagnosticCode::MalformedExport, error.to_string()))?;
    let target_record = &target_row["def"];
    if target_record["safety"] != "unsafe" && target_record["isUnsafe"] != true {
        return Ok(None);
    }
    // Admit duplicate JSON/source bookkeeping before parsing; the ordinary importer
    // and Reference reader retain their separate source/checking limits.
    spend(budget, export.len() as u64)?;
    reserve(budget, (export.len() as u64).saturating_mul(32))?;
    let rows: Vec<Value> = export
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| declined(DiagnosticCode::MalformedExport, error.to_string()))
        })
        .collect::<Result<_>>()?;

    // This importer is confined to this diagnostic operation. Its normal guards remain enabled.
    // Only already checked safe declarations enter its context; runtime records stay quarantined.
    let mut importer = Importer::new(true);
    importer.lean_version = rows[0]["meta"]["lean"]["version"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    for (line, row) in rows.iter().enumerate().skip(1) {
        importer.record(row.clone(), line + 1)?;
    }
    let mut sources = BTreeMap::new();
    for declaration in read.declarations() {
        let row = &rows[declaration.line - 1];
        let record = match declaration.kind {
            intake::DeclarationKind::Inductive
            | intake::DeclarationKind::Constructor
            | intake::DeclarationKind::Recursor => {
                let field = match declaration.kind {
                    intake::DeclarationKind::Inductive => "types",
                    intake::DeclarationKind::Constructor => "ctors",
                    _ => "recs",
                };
                array(&row["inductive"][field])?
                    .iter()
                    .find(|record| {
                        importer.name(&record["name"]).ok().as_deref()
                            == Some(declaration.kernel_name.as_str())
                    })
                    .ok_or_else(|| {
                        declined(
                            DiagnosticCode::MalformedExport,
                            "missing inductive declaration record",
                        )
                    })?
                    .clone()
            }
            _ => row[declaration.kind.as_str()].clone(),
        };
        sources.insert(
            declaration.kernel_name.clone(),
            Source {
                kind: declaration.kind,
                quarantined: importer
                    .runtime_names
                    .contains_key(&declaration.kernel_name),
                record,
            },
        );
    }
    let paths = source_assumptions(&importer, &sources, target, budget)?;
    if !paths
        .iter()
        .any(|path| ESCAPES.contains(&path.assumption.as_str()))
    {
        return Ok(None);
    }
    for name in importer.runtime_names.keys() {
        if importer.ctx.get_global(name).is_some() || importer.ctx.poly_declaration(name).is_some()
        {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "a runtime declaration escaped quarantine",
            ));
        }
    }
    let reference = reference::Reference::read(export)?;
    // Use Reference's own raw universe-name telescope, not importer kernel spelling.
    // The current Reference already owns value(); no accessor or source overwrite is needed.
    inventory(budget, 1)?;
    let target_name = copied(budget, target)?;
    let target_metrics = reference.preflight_kernel(
        &[(target_name, true)],
        budget.max_depth,
        budget.max_tree_nodes,
    )?;
    let target_decode_work = target_metrics
        .tree_nodes
        .saturating_mul(export.len() as u64 + 1);
    spend(budget, target_decode_work)?;
    reserve(
        budget,
        target_decode_work
            .saturating_mul(4)
            .saturating_add(target_metrics.tree_nodes.saturating_mul(256)),
    )?;
    budget.materialized_nodes = budget
        .materialized_nodes
        .checked_sub(target_metrics.tree_nodes)
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "conditional target source allowance",
            )
        })?;
    let target_declaration = reference.declaration_kernel(target)?;
    let target_parameters = target_declaration.level_params;
    inventory(budget, target_parameters.len())?;
    let mut levels = Vec::new();
    for name in &target_parameters {
        levels.push(Universe::Var(copied(budget, name)?));
    }
    let d = Construction::new(budget).map_err(generation)?;
    let mut builder = ConditionalBuilder {
        reference: &reference,
        sources: &sources,
        instances: BTreeMap::new(),
        active: BTreeSet::new(),
        bindings: Vec::new(),
        d,
        source_nodes: (NODE_BUDGET as u64).saturating_sub(target_metrics.tree_nodes),
        source_payload_envelope: export.len() as u64,
        serial: 0,
    };
    let (body, ty) = builder.instance(target, &levels, 0)?;
    let (body, ty) = builder.close(body, ty)?;
    let conditional_body = builder.d.extract(body).map_err(generation)?;
    let conditional_type = builder.d.extract(ty).map_err(generation)?;
    let main_sort = lean_type(&importer.ctx, &conditional_type)?;
    sort(&importer.ctx, &conditional_type)?;
    let main_type = lean_type(&importer.ctx, &conditional_body)?;
    exact(&importer.ctx, &main_type, &conditional_type)?;
    let independent_sort = crate::recheck(&importer.ctx, &conditional_type)
        .map_err(|error| independent_verdict(&error, "conditional runtime statement"))?;
    independent_comparison(
        exact(&importer.ctx, &main_sort, &independent_sort),
        "conditional runtime statement type",
    )?;
    let independent_type = crate::recheck(&importer.ctx, &conditional_body)
        .map_err(|error| independent_verdict(&error, "conditional runtime body"))?;
    independent_comparison(
        exact(&importer.ctx, &independent_type, &conditional_type),
        "conditional runtime body type",
    )?;
    inventory(builder.d.budget, builder.bindings.len())?;
    let mut instances = Vec::new();
    for binding in &builder.bindings {
        let instance = builder.d.extract(binding.instance).map_err(generation)?;
        let Term::Const { levels, .. } = instance else {
            unreachable!()
        };
        inventory(builder.d.budget, levels.len())?;
        let mut universe_arguments = Vec::new();
        for level in levels {
            universe_arguments.push(rendered(&level, builder.d.budget)?);
        }
        let ty = builder.d.extract(binding.ty).map_err(generation)?;
        let declaration = copied(builder.d.budget, &binding.source)?;
        let declaration = intake::lean_name(&declaration);
        let local = builder.d.store.name(binding.local);
        let local_variable = copied(builder.d.budget, local)?;
        instances.push(CheckedInstance {
            declaration,
            universe_arguments,
            local_variable,
            declared_type: rendered(&ty, builder.d.budget)?,
            assumption: binding.body.is_none(),
        });
    }
    let conditional_text = rendered(&conditional_type, builder.d.budget)?;
    Ok(Some(ConditionalAudit {
        schema: "logos-lean-runtime-refusal-audit/1",
        original_target: intake::lean_name(target),
        scope: CheckScope::QuarantinedConditionalJudgment,
        original_target_proved: false,
        original_target_importable: false,
        unsafe_globals_registered: false,
        universe_parameters: target_parameters,
        conditional_type: conditional_text,
        instances,
        safe_environment: importer.report.to_json(),
        assumptions: paths.iter().map(|path| path.assumption.clone()).collect(),
        taint: paths,
    }))
}

fn source_assumptions(
    importer: &Importer,
    sources: &BTreeMap<String, Source>,
    target: &str,
    accounting: &mut RecursorBudget,
) -> Result<Vec<taint::TaintPath>> {
    let mut parent = BTreeMap::<String, String>::new();
    let mut seen = BTreeSet::from([target.to_string()]);
    let mut queue = VecDeque::from([target.to_string()]);
    let mut assumptions = BTreeSet::new();
    let mut budget = NODE_BUDGET;
    while let Some(name) = queue.pop_front() {
        inventory(accounting, 1)?;
        spend(accounting, name.len() as u64 + 1)?;
        let source = sources.get(&name).ok_or_else(|| {
            rejected(
                DiagnosticCode::MissingDeclaration,
                format!("missing source declaration {name}"),
            )
        })?;
        if source.kind == intake::DeclarationKind::Axiom {
            assumptions.insert(name);
            continue;
        }
        if source.kind == intake::DeclarationKind::Quotient {
            continue;
        }
        let mut roots = vec![index(&source.record["type"])?];
        if matches!(
            source.kind,
            intake::DeclarationKind::Definition
                | intake::DeclarationKind::Theorem
                | intake::DeclarationKind::Opaque
        ) {
            roots.push(index(&source.record["value"])?);
        }
        let mut references = BTreeSet::new();
        if source.kind == intake::DeclarationKind::Inductive {
            for constructor in array(&source.record["ctors"])? {
                references.insert(importer.name(constructor)?);
            }
        }
        let mut expressions = HashSet::new();
        while let Some(id) = roots.pop() {
            inventory(accounting, 1)?;
            if !expressions.insert(id) {
                continue;
            }
            budget = budget.checked_sub(1).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "runtime source dependency budget",
                )
            })?;
            let row = importer.exprs.get(&id).ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "missing dependency expression",
                )
            })?;
            if let Some(constant) = row.get("const") {
                references.insert(importer.name(&constant["name"])?);
            }
            for child in record_children(row) {
                inventory(accounting, 1)?;
                roots.push(index(child)?);
            }
        }
        let mut references: Vec<String> = references.into_iter().collect();
        references.sort_by_key(|name| intake::lean_name(name));
        for next in references {
            inventory(accounting, 3)?;
            reserve(
                accounting,
                (next.len() as u64)
                    .saturating_mul(3)
                    .saturating_add(name.len() as u64),
            )?;
            if seen.insert(next.clone()) {
                parent.insert(next.clone(), name.clone());
                queue.push_back(next);
            }
        }
    }
    let mut paths = Vec::new();
    for assumption in assumptions {
        let mut path = vec![assumption.clone()];
        while path.last().unwrap() != target {
            inventory(accounting, 1)?;
            let previous = parent.get(path.last().unwrap()).ok_or_else(|| {
                declined(
                    DiagnosticCode::InternalInconsistency,
                    "missing runtime taint predecessor",
                )
            })?;
            path.push(copied(accounting, previous)?);
        }
        path.reverse();
        paths.push(taint::TaintPath {
            assumption: intake::lean_name(&assumption),
            path: path.iter().map(|name| intake::lean_name(name)).collect(),
        });
    }
    paths.sort_by(|a, b| a.assumption.cmp(&b.assumption));
    Ok(paths)
}

struct Binding {
    source: String,
    instance: TermId,
    local: Symbol,
    ty: TermId,
    body: Option<TermId>,
}
struct ConditionalBuilder<'a, 'b> {
    reference: &'a reference::Reference,
    sources: &'a BTreeMap<String, Source>,
    instances: BTreeMap<(String, Vec<LevelId>), (Symbol, TermId)>,
    active: BTreeSet<(String, Vec<LevelId>)>,
    bindings: Vec<Binding>,
    d: Construction<'b>,
    source_nodes: u64,
    source_payload_envelope: u64,
    serial: usize,
}
fn generation(error: RecursorGenerationError) -> Verdict {
    match error {
        RecursorGenerationError::ResourceLimit => declined(
            DiagnosticCode::ResourceLimit,
            "conditional runtime construction allowance",
        ),
        RecursorGenerationError::Kernel(error) => kernel(error),
    }
}
fn spend(b: &mut RecursorBudget, n: u64) -> Result<()> {
    b.spend(n).map_err(generation)
}
fn reserve(b: &mut RecursorBudget, n: u64) -> Result<()> {
    b.reserve(n).map_err(generation)
}
fn inventory(b: &mut RecursorBudget, n: usize) -> Result<()> {
    b.inventory(n).map_err(generation)
}
fn copied(b: &mut RecursorBudget, s: &str) -> Result<String> {
    spend(b, s.len() as u64 + 1)?;
    reserve(b, s.len() as u64 + 64)?;
    Ok(s.into())
}

impl ConditionalBuilder<'_, '_> {
    fn preflight(&mut self, name: &str, body: bool) -> Result<()> {
        inventory(self.d.budget, 1)?;
        let name = copied(self.d.budget, name)?;
        let metrics = self.reference.preflight_kernel(
            &[(name, body)],
            self.d.budget.max_depth,
            self.source_nodes,
        )?;
        self.source_nodes = self
            .source_nodes
            .checked_sub(metrics.tree_nodes)
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "conditional source expansion allowance",
                )
            })?;
        // Each source occurrence can duplicate at most the admitted export's identifier/
        // literal payload. Charge this conservative envelope before the independent
        // Reference lowerer constructs trees, including repeated universe instances.
        let source_work = metrics
            .tree_nodes
            .saturating_mul(self.source_payload_envelope.saturating_add(1));
        spend(self.d.budget, source_work)?;
        reserve(
            self.d.budget,
            metrics
                .tree_nodes
                .saturating_mul(256)
                .saturating_add(source_work.saturating_mul(4)),
        )?;
        self.d.budget.materialized_nodes = self
            .d
            .budget
            .materialized_nodes
            .checked_sub(metrics.tree_nodes)
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "conditional source materialization allowance",
                )
            })?;
        Ok(())
    }
    fn instance(
        &mut self,
        name: &str,
        levels: &[Universe],
        depth: usize,
    ) -> Result<(TermId, TermId)> {
        spend(self.d.budget, 1)?;
        if depth > self.d.budget.max_depth {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "conditional dependency depth",
            ));
        }
        inventory(self.d.budget, levels.len().saturating_add(2))?;
        let mut level_ids = Vec::new();
        for level in levels {
            level_ids.push(self.d.level(level).map_err(generation)?);
        }
        let key = (copied(self.d.budget, name)?, level_ids);
        spend(
            self.d.budget,
            (name.len() as u64 + key.1.len() as u64 + 1)
                .saturating_mul(self.instances.len() as u64 + self.active.len() as u64 + 1),
        )?;
        if let Some(&(local, ty)) = self.instances.get(&key) {
            return Ok((self.d.add(Node::Var(local)).map_err(generation)?, ty));
        }
        inventory(self.d.budget, key.1.len().saturating_add(1))?;
        reserve(self.d.budget, name.len() as u64 + 64)?;
        if !self.active.insert(key.clone()) {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "recursive runtime dependency has no conditional acyclic check",
            ));
        }
        let source = self.sources.get(name).ok_or_else(|| {
            rejected(DiagnosticCode::MissingDeclaration, "missing runtime source")
        })?;
        if !source.quarantined
            || !matches!(
                source.kind,
                intake::DeclarationKind::Definition | intake::DeclarationKind::Axiom
            )
        {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "conditional audit requires an unsafe definition or unsafe local assumption",
            ));
        }
        if source.record["safety"] == "partial" {
            return Err(declined(
                DiagnosticCode::UnsupportedFeature,
                "partial runtime bodies are outside conditional refusal checking",
            ));
        }
        let body_required = source.kind != intake::DeclarationKind::Axiom;
        self.preflight(name, body_required)?;
        let declaration = self.reference.declaration_kernel(name)?;
        if declaration.level_params.len() != levels.len() {
            return Err(rejected(
                DiagnosticCode::InvalidProof,
                "runtime universe arity mismatch",
            ));
        }
        inventory(self.d.budget, declaration.level_params.len())?;
        let mut permitted = HashSet::new();
        for param in &declaration.level_params {
            if !permitted.insert(param.as_str()) {
                return Err(rejected(
                    DiagnosticCode::InvalidProof,
                    "duplicate runtime universe parameter",
                ));
            }
        }
        let original_body = if body_required {
            Some(self.reference.value_kernel(name)?)
        } else {
            None
        };
        for term in std::iter::once(&declaration.ty).chain(original_body.iter()) {
            let root = self.d.intern(term).map_err(generation)?;
            for level in self.d.free_level_vars(root).map_err(generation)? {
                spend(self.d.budget, level.len() as u64 + 1)?;
                if !permitted.contains(level.as_str()) {
                    return Err(rejected(
                        DiagnosticCode::InvalidProof,
                        "runtime body has an undeclared universe parameter",
                    ));
                }
            }
        }
        inventory(self.d.budget, levels.len())?;
        let mut substitution = HashMap::new();
        // Copy replacement universes through an admitted small Const instead of raw Clone.
        let symbol = self.d.symbol(name).map_err(generation)?;
        inventory(self.d.budget, key.1.len())?;
        let instance = self
            .d
            .add(Node::Const {
                name: symbol,
                levels: key.1.clone().into_boxed_slice(),
            })
            .map_err(generation)?;
        let Term::Const {
            levels: copied_levels,
            ..
        } = self.d.extract(instance).map_err(generation)?
        else {
            unreachable!()
        };
        for (param, level) in declaration.level_params.iter().zip(copied_levels) {
            substitution.insert(copied(self.d.budget, param)?, level);
        }
        let specialized =
            instantiate_universes_bounded(&declaration.ty, &substitution, self.d.budget)
                .map_err(generation)?;
        let root = self.d.intern(&specialized).map_err(generation)?;
        let ty = self.term(root, depth + 1)?;
        let body = if let Some(original) = original_body {
            let specialized =
                instantiate_universes_bounded(&original, &substitution, self.d.budget)
                    .map_err(generation)?;
            let root = self.d.intern(&specialized).map_err(generation)?;
            Some(self.term(root, depth + 1)?)
        } else {
            None
        };
        let local_name = self
            .d
            .generated_name("runtime.conditional.", self.serial, None)
            .map_err(generation)?;
        self.serial = self.serial.checked_add(1).ok_or_else(|| {
            declined(DiagnosticCode::ResourceLimit, "conditional local inventory")
        })?;
        let local = self.d.symbol(&local_name).map_err(generation)?;
        inventory(self.d.budget, 2)?;
        self.bindings.push(Binding {
            source: copied(self.d.budget, name)?,
            instance,
            local,
            ty,
            body,
        });
        self.active.remove(&key);
        self.instances.insert(key, (local, ty));
        Ok((self.d.add(Node::Var(local)).map_err(generation)?, ty))
    }
    fn term(&mut self, root: TermId, depth: usize) -> Result<TermId> {
        enum Op {
            Visit(TermId),
            Build(TermId, usize),
        }
        inventory(self.d.budget, 1)?;
        let mut todo = vec![Op::Visit(root)];
        let mut out = Vec::new();
        while let Some(op) = todo.pop() {
            spend(self.d.budget, 1)?;
            match op {
                Op::Visit(id) => {
                    if let Node::Global(name) | Node::Const { name, .. } = self.d.store.node(id) {
                        let name = self.d.store.name(*name);
                        spend(
                            self.d.budget,
                            (name.len() as u64 + 1).saturating_mul(self.sources.len() as u64 + 1),
                        )?;
                        if self
                            .sources
                            .get(name)
                            .is_some_and(|source| source.quarantined)
                        {
                            let term = self.d.extract(id).map_err(generation)?;
                            let (name, levels) = match term {
                                Term::Global(name) => (name, Vec::new()),
                                Term::Const { name, levels } => (name, levels),
                                _ => unreachable!(),
                            };
                            let (value, _) = self.instance(&name, &levels, depth + 1)?;
                            inventory(self.d.budget, 1)?;
                            out.push(value);
                            continue;
                        }
                    }
                    if matches!(
                        self.d.store.node(id),
                        Node::Fix { .. } | Node::MutualFix { .. }
                    ) {
                        return Err(declined(
                            DiagnosticCode::UnsupportedFeature,
                            "recursive runtime term is outside acyclic checking",
                        ));
                    }
                    if matches!(self.d.store.node(id), Node::Hole) {
                        return Err(rejected(
                            DiagnosticCode::InvalidProof,
                            "conditional runtime judgment contains a hole",
                        ));
                    }
                    let n = self.d.store.node(id).child_count();
                    inventory(self.d.budget, n.saturating_add(2))?;
                    if n == 0 {
                        out.push(id);
                        continue;
                    }
                    todo.push(Op::Build(id, n));
                    let start = todo.len();
                    self.d
                        .store
                        .node(id)
                        .for_each_child(&mut |child| todo.push(Op::Visit(child)));
                    todo[start..].reverse();
                }
                Op::Build(id, n) => {
                    inventory(self.d.budget, n.saturating_add(1))?;
                    let base = out.len() - n;
                    let node = self.d.store.node(id).rebuild_with(&out[base..]);
                    out.truncate(base);
                    out.push(self.d.add(node).map_err(generation)?);
                }
            }
        }
        Ok(out.pop().expect("one conditional term root"))
    }
    fn close(&mut self, mut body: TermId, mut ty: TermId) -> Result<(TermId, TermId)> {
        for binding in self.bindings.iter().rev() {
            let name = binding.local;
            let domain = binding.ty;
            match binding.body {
                Some(value) => {
                    body = self
                        .d
                        .add(Node::Let {
                            name,
                            ty: domain,
                            value,
                            body,
                        })
                        .map_err(generation)?;
                    ty = self
                        .d
                        .add(Node::Let {
                            name,
                            ty: domain,
                            value,
                            body: ty,
                        })
                        .map_err(generation)?;
                }
                None => {
                    body = self
                        .d
                        .add(Node::Lambda {
                            param: name,
                            param_type: domain,
                            body,
                        })
                        .map_err(generation)?;
                    ty = self
                        .d
                        .add(Node::Pi {
                            param: name,
                            param_type: domain,
                            body_type: ty,
                        })
                        .map_err(generation)?;
                }
            }
        }
        Ok((body, ty))
    }
}

/// Display has its own pre-allocation output guard, including recursive universes
/// and literal/name payloads. Admission does not format a term to find its size.
fn rendered<T: std::fmt::Display>(value: &T, budget: &mut RecursorBudget) -> Result<String> {
    struct Limited<'a> {
        text: String,
        budget: &'a mut RecursorBudget,
        failed: bool,
    }
    impl std::fmt::Write for Limited<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            if self.budget.spend(s.len() as u64 + 1).is_err()
                || self
                    .budget
                    .reserve((s.len() as u64).saturating_mul(2).saturating_add(64))
                    .is_err()
            {
                self.failed = true;
                return Err(std::fmt::Error);
            }
            self.text.push_str(s);
            Ok(())
        }
    }
    let mut writer = Limited {
        text: String::new(),
        budget,
        failed: false,
    };
    std::fmt::write(&mut writer, format_args!("{value}")).map_err(|_| {
        declined(
            DiagnosticCode::ResourceLimit,
            "conditional audit text allowance",
        )
    })?;
    Ok(writer.text)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/runtime_refusal/controls.rs"]
mod controls;
