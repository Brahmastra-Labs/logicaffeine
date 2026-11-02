//! The reference translator: a deliberately small, unoptimized reading of a Lean export's declaration
//! TYPES into kernel terms, sharing no code with the production reader's conversion, sharing or
//! caching machinery. It is the independent oracle a result's statement is compared against
//! (competition rule R8): a faulty production translation and a faulty emitter cannot agree with it
//! by construction.
//!
//! # What it reads, and what it refuses
//!
//! One pass over the NDJSON records builds three tables — names, levels, expressions — keeping each
//! record as the JSON it arrived as. The requested declaration's type is then translated by a plain
//! post-order walk: `sort`, `bvar`, `const`, `app`, `lam`, `forallE`, `letE`, `mdata` (transparent),
//! `natVal` and `strVal`. Nothing is shared, cached, reduced or checked here; the kernel decides what the
//! resulting term means.
//!
//! A `strVal` string literal is read as the constructor application the export's own Lean release
//! defines it to be, from the header's `meta.lean.version` and nothing else: Lean 4.14.0, whose
//! `String` is the structure `String.mk (data : List Char)`, reads `"ab"` as
//! `String.mk [Char.ofNat 97, Char.ofNat 98]`; Lean 4.29.1, 4.30.0 and 4.33.0, whose `String` is UTF-8
//! bytes, read it as `String.ofList [Char.ofNat 97, Char.ofNat 98]`. Under any other release a
//! literal has no reference reading and is declined. Nothing about the export's `String` is looked
//! up or checked here; the kernel decides whether the reading is well-typed.
//!
//! `proj` is refused as unsupported: the production reader lowers it through typed machinery (a
//! projection needs its structure's constructor telescope), and reproducing that machinery would
//! make this oracle neither small nor independent. A statement that needs one is reported as having
//! no reference translation, which is an explicit gap and never a pass.
//!
//! # The identity contract
//!
//! Names are the export's hierarchical names written component by component — `/s:"T06_002"/s:"x"`
//! for a string component, `/n:3` for a numeric one — and universe parameters are spelled the same
//! way. The kernel's primitive identities are the one place a Lean name is not its own spelling:
//! `Nat`, `Nat.zero` and `Nat.succ` are the kernel's `Nat`, `Zero` and `Succ` (every other `Nat.x`
//! lives under `Nat/`), and Lean's four quotient primitives are the kernel's certified
//! `kernel.quot.*` declarations. That table is the translation's contract with the kernel, written
//! out here in full rather than imported, so a change to it on either side is a visible
//! disagreement.

use super::{declined, rejected, DiagnosticCode, Result};
use crate::{Literal, Term, Universe};
use serde_json::Value;
use std::collections::HashMap;

/// The largest term (in tree nodes) the reference translation will build. Exports share
/// subexpressions as a DAG; a tree unfolding is bounded so a hostile export cannot make the oracle
/// allocate without limit.
pub const REFERENCE_NODE_BUDGET: usize = 1_000_000;

/// Independent decimal workload ceiling. This bounds parsing effort, not the value of a Nat.
const REFERENCE_NATURAL_WORK_BUDGET: u64 = 1_000_000;

/// Admit bytes before examining them and decimal limb work before conversion. Return the
/// significant suffix without allocation; ASCII validation and zero-prefix discovery share
/// one pass. The caller carries `work` across every expanded literal occurrence.
fn admit_natural_literal<'a>(payload: &'a Value, work: &mut u64) -> Result<&'a str> {
    let digits = payload
        .as_str()
        .ok_or_else(|| malformed("natural literal"))?;
    *work = work
        .checked_add(u64::try_from(digits.len()).unwrap_or(u64::MAX))
        .filter(|sum| *sum <= REFERENCE_NATURAL_WORK_BUDGET)
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "reference natural literal workload",
            )
        })?;
    if digits.is_empty() {
        return Err(malformed("natural literal"));
    }
    let mut first = digits.len();
    for (index, byte) in digits.bytes().enumerate() {
        if !byte.is_ascii_digit() {
            return Err(malformed("natural literal"));
        }
        if first == digits.len() && byte != b'0' {
            first = index;
        }
    }
    let significant = &digits[first..];
    // Fewer than four bits per decimal digit: at most n/16+1 64-bit limbs.
    // Each digit's multiply-by-ten and add visit at most that many limbs.
    let n = significant.len() as u64;
    let conversion = n
        .checked_mul(n / 16 + 1)
        .and_then(|n| n.checked_mul(2))
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "reference natural literal workload",
            )
        })?;
    *work = work
        .checked_add(conversion)
        .filter(|sum| *sum <= REFERENCE_NATURAL_WORK_BUDGET)
        .ok_or_else(|| {
            declined(
                DiagnosticCode::ResourceLimit,
                "reference natural literal workload",
            )
        })?;
    Ok(significant)
}

/// Raw-source tree bounds, checked before allocating any kernel term or universe.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SourceMetrics {
    pub(super) depth: usize,
    pub(super) tree_nodes: u64,
    /// Cumulative decimal-byte and conversion work across all admitted source roots.
    pub(super) natural_work: u64,
    pub(super) has_projection: bool,
    pub(super) has_generated: bool,
}

impl Reference {
    /// Decode source metadata and indices without building terms. Shape reading itself does
    /// not authorize a block or its computational rules.
    pub(super) fn recursor_block_shape(
        &self,
        declaration: &str,
    ) -> Result<ReferenceInductiveBlock<u64>> {
        self.tables
            .read_recursor_block(declaration, Lookup::Mixed, &mut |root| Ok(root))
    }

    /// Independent family/constructor metadata by exact kernel identity. No source expression is translated,
    /// and no uncarried recursor declaration or rule is inspected.
    pub(super) fn family_shape(&self, declaration: &str) -> Result<ReferenceInductiveBlock<u64>> {
        let block = self.tables.containing_family_block(declaration)?;
        self.tables.read_block(block, false, &mut |root| Ok(root))
    }

    /// Exact translated root occurrences, including every rule RHS.
    pub(super) fn recursor_block_roots(&self, declaration: &str) -> Result<Vec<u64>> {
        Ok(self.recursor_block_shape(declaration)?.expression_roots())
    }

    /// Term-free admission of complete source blocks. Shared blocks are charged once, matching
    /// a canonical reader that caches each block once; every repeated root within a block counts.
    pub(super) fn preflight_recursor_blocks(
        &self,
        declarations: &[String],
        max_depth: usize,
        max_tree_nodes: u64,
    ) -> Result<SourceMetrics> {
        let mut total = SourceMetrics::default();
        self.preflight_recursor_blocks_into(
            declarations,
            max_depth,
            max_tree_nodes,
            &mut total,
            Lookup::Mixed,
        )?;
        Ok(total)
    }

    pub(super) fn preflight_recursor_blocks_kernel(
        &self,
        declarations: &[String],
        max_depth: usize,
        max_tree_nodes: u64,
    ) -> Result<SourceMetrics> {
        let mut total = SourceMetrics::default();
        self.preflight_recursor_blocks_into(
            declarations,
            max_depth,
            max_tree_nodes,
            &mut total,
            Lookup::Kernel,
        )?;
        Ok(total)
    }

    fn preflight_recursor_blocks_into(
        &self,
        declarations: &[String],
        max_depth: usize,
        max_tree_nodes: u64,
        total: &mut SourceMetrics,
        lookup: Lookup,
    ) -> Result<()> {
        let mut seen = HashMap::<Vec<String>, ()>::new();
        for declaration in declarations {
            let shape = self
                .tables
                .read_recursor_block(declaration, lookup, &mut |root| Ok(root))?;
            let identity: Vec<_> = shape
                .recursors
                .iter()
                .map(|recursor| recursor.declaration.name.clone())
                .collect();
            if seen.insert(identity, ()).is_some() {
                continue;
            }
            total.has_generated = true;
            for root in shape.expression_roots() {
                self.tables
                    .preflight_root(root, max_depth, max_tree_nodes, total)?;
            }
        }
        Ok(())
    }

    /// Independently decode the containing block, using the default honest projection refusal.
    pub(super) fn recursor_block(
        &self,
        declaration: &str,
    ) -> Result<ReferenceInductiveBlock<Term>> {
        self.recursor_block_with(declaration, &mut NoProjection)
    }

    /// Admit ALL source roots before allocating the first translated expression or invoking a
    /// projection callback. Project callers additionally admit against their declared limits and
    /// reserve a worker for the canonical generator's full permitted output depth.
    pub(super) fn recursor_block_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<ReferenceInductiveBlock<Term>> {
        self.preflight_recursor_blocks(
            &[declaration.to_string()],
            REFERENCE_NODE_BUDGET,
            REFERENCE_NODE_BUDGET as u64,
        )?;
        self.tables
            .read_recursor_block(declaration, Lookup::Mixed, &mut |root| {
                self.tables.translate_with(&Value::from(root), projections)
            })
    }

    /// Exact kernel identity: certificate names cannot be mistaken for a different Lean spelling.
    pub(super) fn recursor_block_kernel_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<ReferenceInductiveBlock<Term>> {
        self.preflight_recursor_blocks_kernel(
            &[declaration.to_string()],
            REFERENCE_NODE_BUDGET,
            REFERENCE_NODE_BUDGET as u64,
        )?;
        self.tables
            .read_recursor_block(declaration, Lookup::Kernel, &mut |root| {
                self.tables.translate_with(&Value::from(root), projections)
            })
    }

    /// Resolve an existing source identity without translating its type.
    pub(super) fn source_name(&self, name: &str) -> Result<String> {
        let (record, _) = self.tables.find(name)?;
        self.tables.kernel_name(&record["name"])
    }

    /// Fields explicitly labelled Lean spelling never fall back to kernel aliases.
    pub(super) fn source_lean_name(&self, name: &str) -> Result<String> {
        let (record, _) = self.tables.find_lookup(name, Lookup::Lean)?;
        self.tables.kernel_name(&record["name"])
    }

    /// Resolve an exact kernel identity and its Lean spelling without lowering a type.
    pub(super) fn source_spelling(&self, name: &str) -> Result<(String, String)> {
        let (record, _) = self.tables.find_kernel(name)?;
        Ok((
            self.tables.kernel_name(&record["name"])?,
            self.tables.lean_name(&record["name"])?,
        ))
    }

    /// Exact one-root metrics for a pending decode named by kernel identity. Complete recursor-block admission remains
    /// the separate preflight contract used before choosing a worker or checking a certificate.
    pub(super) fn source_root_metrics(
        &self,
        name: &str,
        value: bool,
        max_depth: usize,
        max_tree_nodes: u64,
    ) -> Result<SourceMetrics> {
        let (record, _) = self.tables.find_kernel(name)?;
        let root = if value {
            &record["value"]
        } else {
            &record["type"]
        };
        let mut metrics = SourceMetrics::default();
        self.tables
            .preflight_root(id(root)?, max_depth, max_tree_nodes, &mut metrics)?;
        Ok(metrics)
    }

    /// Admit exactly the source types and transparent bodies a binding operation will read.
    /// `body` is false for theorem proofs: proving the same statement does not require their bytes.
    pub(super) fn preflight(
        &self,
        declarations: &[(String, bool)],
        max_depth: usize,
        max_tree_nodes: u64,
    ) -> Result<SourceMetrics> {
        self.preflight_lookup(declarations, max_depth, max_tree_nodes, Lookup::Mixed)
    }

    pub(super) fn preflight_kernel(
        &self,
        declarations: &[(String, bool)],
        max_depth: usize,
        max_tree_nodes: u64,
    ) -> Result<SourceMetrics> {
        self.preflight_lookup(declarations, max_depth, max_tree_nodes, Lookup::Kernel)
    }

    fn preflight_lookup(
        &self,
        declarations: &[(String, bool)],
        max_depth: usize,
        max_tree_nodes: u64,
        lookup: Lookup,
    ) -> Result<SourceMetrics> {
        let mut total = SourceMetrics::default();
        let mut recursors = Vec::new();
        for (name, body) in declarations {
            let (record, kind) = self.tables.find_lookup(name, lookup)?;
            if kind == ReferenceKind::Recursor {
                recursors.push(name.clone());
            }
            let mut roots = vec![&record["type"]];
            if *body {
                roots.push(&record["value"]);
            }
            for root in roots {
                self.tables
                    .preflight_root(id(root)?, max_depth, max_tree_nodes, &mut total)?;
            }
        }
        self.preflight_recursor_blocks_into(
            &recursors,
            max_depth,
            max_tree_nodes,
            &mut total,
            lookup,
        )?;
        Ok(total)
    }
}

impl Tables {
    /// Find a unique recursor declaration and its unique containing block. Lookup is by the
    /// same Lean/raw/kernel identity contract as `declaration`, never by stripping `.rec`.
    fn containing_recursor_block(&self, declaration: &str, lookup: Lookup) -> Result<&Value> {
        let (record, kind) = self.find_lookup(declaration, lookup)?;
        if kind != ReferenceKind::Recursor {
            return Err(malformed("requested declaration is not a recursor"));
        }
        let wanted = self.kernel_name(&record["name"])?;
        let mut found = None;
        for outer in &self.declarations {
            let Some(block) = outer.get("inductive") else {
                continue;
            };
            for recursor in block_array(block, "recs")? {
                if self.kernel_name(&recursor["name"])? == wanted {
                    if found.replace(block).is_some() {
                        return Err(malformed("recursor has more than one containing block"));
                    }
                }
            }
        }
        found.ok_or_else(|| malformed("recursor has no containing inductive block"))
    }

    fn containing_family_block(&self, declaration: &str) -> Result<&Value> {
        let (record, kind) = self.find_kernel(declaration)?;
        if kind != ReferenceKind::Inductive {
            return Err(malformed(
                "requested projection parent is not an inductive family",
            ));
        }
        let wanted = self.kernel_name(&record["name"])?;
        let mut found = None;
        for outer in &self.declarations {
            let Some(block) = outer.get("inductive") else {
                continue;
            };
            for member in block_array(block, "types")? {
                if self.kernel_name(&member["name"])? == wanted {
                    if found.replace(block).is_some() {
                        return Err(malformed("family has more than one containing block"));
                    }
                }
            }
        }
        found.ok_or_else(|| malformed("family has no containing inductive block"))
    }

    fn block_names(&self, value: &Value, field: &str) -> Result<Vec<String>> {
        block_array(value, field)?
            .iter()
            .map(|name| self.kernel_name(name))
            .collect()
    }

    fn block_declaration<E>(
        &self,
        record: &Value,
        expression: &mut impl FnMut(u64) -> Result<E>,
    ) -> Result<ReferenceBlockDeclaration<E>> {
        let level_params = block_array(record, "levelParams")?
            .iter()
            .map(|level| self.raw_name(level))
            .collect::<Result<Vec<_>>>()?;
        let mut seen = std::collections::HashSet::new();
        if level_params.iter().any(|level| !seen.insert(level)) {
            return Err(malformed("duplicate universe parameter in source block"));
        }
        Ok(ReferenceBlockDeclaration {
            name: self.kernel_name(&record["name"])?,
            lean_name: self.lean_name(&record["name"])?,
            level_params,
            ty: expression(id(&record["type"])?)?,
            is_unsafe: block_bool(record, "isUnsafe")?,
        })
    }

    fn read_recursor_block<E>(
        &self,
        declaration: &str,
        lookup: Lookup,
        expression: &mut impl FnMut(u64) -> Result<E>,
    ) -> Result<ReferenceInductiveBlock<E>> {
        let block = self.containing_recursor_block(declaration, lookup)?;
        self.read_block(block, true, expression)
    }

    fn read_block<E>(
        &self,
        block: &Value,
        include_recursors: bool,
        expression: &mut impl FnMut(u64) -> Result<E>,
    ) -> Result<ReferenceInductiveBlock<E>> {
        block_fields(block, &["types", "ctors", "recs"])?;
        let types = block_array(block, "types")?;
        if types.is_empty() {
            return Err(malformed("empty source inductive block"));
        }
        let mut members = Vec::new();
        for member in types {
            block_fields(
                member,
                &[
                    "name",
                    "levelParams",
                    "type",
                    "numParams",
                    "numIndices",
                    "numNested",
                    "isUnsafe",
                    "isRec",
                    "isReflexive",
                    "ctors",
                    "all",
                ],
            )?;
            members.push(ReferenceInductiveMember {
                declaration: self.block_declaration(member, expression)?,
                num_params: block_count(member, "numParams")?,
                num_indices: block_count(member, "numIndices")?,
                num_nested: block_count(member, "numNested")?,
                is_recursive: block_bool(member, "isRec")?,
                is_reflexive: block_bool(member, "isReflexive")?,
                constructors: self.block_names(member, "ctors")?,
                all: self.block_names(member, "all")?,
            });
        }
        let mut constructors = Vec::new();
        for constructor in block_array(block, "ctors")? {
            block_fields(
                constructor,
                &[
                    "name",
                    "levelParams",
                    "type",
                    "numParams",
                    "numFields",
                    "isUnsafe",
                    "induct",
                    "cidx",
                ],
            )?;
            constructors.push(ReferenceConstructor {
                declaration: self.block_declaration(constructor, expression)?,
                parent: self.kernel_name(&constructor["induct"])?,
                index: block_count(constructor, "cidx")?,
                num_params: block_count(constructor, "numParams")?,
                num_fields: block_count(constructor, "numFields")?,
            });
        }
        let mut recursors = Vec::new();
        for recursor in if include_recursors {
            block_array(block, "recs")?
        } else {
            &[]
        } {
            block_fields(
                recursor,
                &[
                    "name",
                    "levelParams",
                    "type",
                    "numParams",
                    "numIndices",
                    "numMotives",
                    "numMinors",
                    "isUnsafe",
                    "all",
                    "k",
                    "rules",
                ],
            )?;
            let declaration = self.block_declaration(recursor, expression)?;
            let mut rules = Vec::new();
            for rule in block_array(recursor, "rules")? {
                block_fields(rule, &["ctor", "nfields", "rhs"])?;
                rules.push(ReferenceRecursorRule {
                    constructor: self.kernel_name(&rule["ctor"])?,
                    num_fields: block_count(rule, "nfields")?,
                    rhs: expression(id(&rule["rhs"])?)?,
                });
            }
            recursors.push(ReferenceRecursor {
                declaration,
                num_params: block_count(recursor, "numParams")?,
                num_indices: block_count(recursor, "numIndices")?,
                num_motives: block_count(recursor, "numMotives")?,
                num_minors: block_count(recursor, "numMinors")?,
                all: self.block_names(recursor, "all")?,
                is_k: block_bool(recursor, "k")?,
                rules,
            });
        }
        let mut names = std::collections::HashSet::new();
        for name in members
            .iter()
            .map(|member| &member.declaration.name)
            .chain(constructors.iter().map(|ctor| &ctor.declaration.name))
            .chain(recursors.iter().map(|recursor| &recursor.declaration.name))
        {
            if !names.insert(name) {
                return Err(malformed(
                    "duplicate declaration identity in source inductive block",
                ));
            }
            // Check the complete export for duplicates of this exact kernel identity.
            self.find_kernel(name)?;
        }
        Ok(ReferenceInductiveBlock {
            members,
            constructors,
            recursors,
        })
    }

    fn preflight_root(
        &self,
        root: u64,
        max_depth: usize,
        max_tree_nodes: u64,
        total: &mut SourceMetrics,
    ) -> Result<()> {
        // Walk each occurrence, not just distinct DAG nodes: the lowerer allocates a tree.
        // Depth/size are bounded during traversal, so cyclic input also cannot loop unboundedly.
        enum SourceNode {
            Expression(u64),
            Level(u64, usize),
        }
        let mut pending = vec![(SourceNode::Expression(root), 1usize)];
        let mut root_nodes = 0u64;
        while let Some((node, depth)) = pending.pop() {
            if depth > max_depth {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "source expression depth exceeds the environment limit",
                ));
            }
            total.depth = total.depth.max(depth);
            let mut cost = 1u64;
            match node {
                SourceNode::Level(index, nesting) => {
                    // Match the existing reference level reader's nesting limit.
                    if nesting > 512 {
                        return Err(declined(
                            DiagnosticCode::ResourceLimit,
                            "universe level nesting",
                        ));
                    }
                    if index != 0 {
                        let row = self
                            .levels
                            .get(&index)
                            .ok_or_else(|| malformed("unknown source level index"))?;
                        if let Some(inner) = row.get("succ") {
                            pending.push((
                                SourceNode::Level(id(inner)?, nesting + 1),
                                depth.saturating_add(1),
                            ));
                        } else if let Some(pair) = row.get("max").or_else(|| row.get("imax")) {
                            let pair = pair
                                .as_array()
                                .filter(|pair| pair.len() == 2)
                                .ok_or_else(|| malformed("level pair"))?;
                            for child in pair {
                                pending.push((
                                    SourceNode::Level(id(child)?, nesting + 1),
                                    depth.saturating_add(1),
                                ));
                            }
                        } else if row.get("param").is_none() {
                            return Err(malformed("unknown source level record"));
                        }
                    }
                }
                SourceNode::Expression(index) => {
                    let row = self
                        .exprs
                        .get(&index)
                        .ok_or_else(|| malformed("unknown source expression index"))?;
                    if let Some(natural) = row.get("natVal") {
                        admit_natural_literal(natural, &mut total.natural_work)?;
                    }
                    if let Some(level) = row.get("sort") {
                        pending.push((SourceNode::Level(id(level)?, 0), depth.saturating_add(1)));
                    } else if let Some(constant) = row.get("const") {
                        for level in constant["us"]
                            .as_array()
                            .ok_or_else(|| malformed("constant levels"))?
                        {
                            pending
                                .push((SourceNode::Level(id(level)?, 0), depth.saturating_add(1)));
                        }
                    } else if row.get("strVal").is_some() {
                        // Count this reference reader's own expansion independently: each scalar
                        // adds eight term nodes and one list-constant universe; the empty packed
                        // list has five term nodes and one universe. No production lowering runs.
                        let scalars = row["strVal"]
                            .as_str()
                            .ok_or_else(|| malformed("string literal"))?
                            .chars()
                            .count();
                        cost = u64::try_from(scalars)
                            .unwrap_or(u64::MAX)
                            .saturating_mul(9)
                            .saturating_add(6);
                        let literal_depth = if scalars == 0 {
                            4
                        } else {
                            scalars.saturating_add(5)
                        };
                        let expanded_depth = depth.saturating_add(literal_depth.saturating_sub(1));
                        if expanded_depth > max_depth {
                            return Err(declined(
                                DiagnosticCode::ResourceLimit,
                                "source string expansion depth exceeds the environment limit",
                            ));
                        }
                        total.depth = total.depth.max(expanded_depth);
                    }
                    let children: Vec<&Value> = if let Some(app) = row.get("app") {
                        vec![&app["fn"], &app["arg"]]
                    } else if let Some(metadata) = row.get("mdata") {
                        vec![&metadata["expr"]]
                    } else if let Some(projection) = row.get("proj") {
                        total.has_projection = true;
                        vec![&projection["struct"]]
                    } else if let Some(binding) = row.get("letE") {
                        vec![&binding["type"], &binding["value"], &binding["body"]]
                    } else if let Some(binding) = row.get("lam").or_else(|| row.get("forallE")) {
                        vec![&binding["type"], &binding["body"]]
                    } else {
                        Vec::new()
                    };
                    for child in children {
                        pending.push((SourceNode::Expression(id(child)?), depth.saturating_add(1)));
                    }
                }
            }
            root_nodes = root_nodes.checked_add(cost).ok_or_else(|| {
                declined(DiagnosticCode::ResourceLimit, "source tree size overflow")
            })?;
            total.tree_nodes = total.tree_nodes.checked_add(cost).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "source aggregate tree size overflow",
                )
            })?;
            if root_nodes > REFERENCE_NODE_BUDGET as u64 || total.tree_nodes > max_tree_nodes {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "source expanded tree nodes exceed the reference or environment limit",
                ));
            }
        }
        Ok(())
    }
}

/// Independently decoded source syntax for a whole inductive block. `E` is a raw expression
/// index during admission, and a kernel term after reference translation. No field is a claim
/// that the block or its computation rules have been checked by either kernel.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceInductiveBlock<E> {
    pub members: Vec<ReferenceInductiveMember<E>>,
    pub constructors: Vec<ReferenceConstructor<E>>,
    pub recursors: Vec<ReferenceRecursor<E>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceBlockDeclaration<E> {
    pub name: String,
    pub lean_name: String,
    pub level_params: Vec<String>,
    pub ty: E,
    pub is_unsafe: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceInductiveMember<E> {
    pub declaration: ReferenceBlockDeclaration<E>,
    pub num_params: usize,
    pub num_indices: usize,
    pub num_nested: usize,
    pub is_recursive: bool,
    pub is_reflexive: bool,
    pub constructors: Vec<String>,
    pub all: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceConstructor<E> {
    pub declaration: ReferenceBlockDeclaration<E>,
    pub parent: String,
    pub index: usize,
    pub num_params: usize,
    pub num_fields: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceRecursor<E> {
    pub declaration: ReferenceBlockDeclaration<E>,
    pub num_params: usize,
    pub num_indices: usize,
    pub num_motives: usize,
    pub num_minors: usize,
    pub all: Vec<String>,
    pub is_k: bool,
    pub rules: Vec<ReferenceRecursorRule<E>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReferenceRecursorRule<E> {
    pub constructor: String,
    pub num_fields: usize,
    pub rhs: E,
}

impl ReferenceInductiveBlock<u64> {
    /// Every expression translated by `recursor_block`, in translation order. Repeated roots
    /// remain repeated: each occurrence contributes to the later tree-allocation budget.
    pub(super) fn expression_roots(&self) -> Vec<u64> {
        let mut roots = Vec::new();
        roots.extend(self.members.iter().map(|member| member.declaration.ty));
        roots.extend(self.constructors.iter().map(|ctor| ctor.declaration.ty));
        for recursor in &self.recursors {
            roots.push(recursor.declaration.ty);
            roots.extend(recursor.rules.iter().map(|rule| rule.rhs));
        }
        roots
    }
}

/// What a Lean declaration record is, as far as the statement binding needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Definition,
    Theorem,
    Opaque,
    Axiom,
    Inductive,
    Constructor,
    Recursor,
    Quotient,
}

impl ReferenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReferenceKind::Definition => "def",
            ReferenceKind::Theorem => "thm",
            ReferenceKind::Opaque => "opaque",
            ReferenceKind::Axiom => "axiom",
            ReferenceKind::Inductive => "inductive",
            ReferenceKind::Constructor => "ctor",
            ReferenceKind::Recursor => "rec",
            ReferenceKind::Quotient => "quot",
        }
    }
}

/// One declaration as the reference translator reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceDeclaration {
    /// The kernel spelling of the declaration's name.
    pub name: String,
    /// The Lean spelling (components joined by `.`).
    pub lean_name: String,
    pub kind: ReferenceKind,
    /// Universe parameters, in declaration order, in the kernel spelling.
    pub level_params: Vec<String>,
    pub ty: Term,
}

/// The type of `declaration` as the reference translator reads it from `export`.
///
/// `declaration` is either the Lean name (`T06_002.transport`) or its kernel spelling
/// (`/s:"T06_002"/s:"transport"`).
pub fn reference_type(export: &str, declaration: &str) -> Result<Term> {
    reference_declaration(export, declaration).map(|d| d.ty)
}

/// The whole reference reading of one declaration: kernel name, kind, universe parameters and type.
pub fn reference_declaration(export: &str, declaration: &str) -> Result<ReferenceDeclaration> {
    Reference::read(export)?.declaration(declaration)
}

/// A binder surrounding a source projection. Syntax is borrowed from the independent
/// decoder's current output stack; entries are ordered from outermost to innermost.
pub(super) struct ReferenceBinding<'a> {
    pub name: &'a str,
    pub ty: &'a Term,
    pub value: Option<&'a Term>,
}

/// Typed interpretation of a source projection. The source decoder still resolves the
/// parent identity, field ordinal, discriminant syntax and lexical scope independently.
/// Implementations must account for their generated syntax against `budget`.
pub(super) trait ReferenceProjection {
    /// Maximum generated term depth, including source wrappers around callback output.
    /// None preserves the pure reader's source-only iterative admission contract.
    fn generated_depth_limit(&self) -> Option<usize> {
        None
    }

    fn project(
        &mut self,
        parent: &str,
        index: usize,
        discriminant: Term,
        scope: &[ReferenceBinding<'_>],
        budget: &mut usize,
    ) -> Result<Term>;
}

/// Cached output shape: term nodes only; universe trees retain their separate admission.
/// Source wrappers combine this metadata in constant time and never rescan their children.
#[derive(Clone, Copy)]
struct OutputShape {
    depth: usize,
    nodes: usize,
    generated: bool,
}

impl OutputShape {
    const LEAF: Self = Self {
        depth: 1,
        nodes: 1,
        generated: false,
    };

    fn wrapper(children: &[Self], limit: Option<usize>) -> Result<Self> {
        let mut result = Self::LEAF;
        for child in children {
            result.depth = result.depth.max(child.depth.checked_add(1).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "reference output depth overflow",
                )
            })?);
            result.nodes = result.nodes.checked_add(child.nodes).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "reference output size overflow",
                )
            })?;
            result.generated |= child.generated;
        }
        result.admit(limit)?;
        Ok(result)
    }

    fn admit(self, limit: Option<usize>) -> Result<()> {
        if self.nodes > REFERENCE_NODE_BUDGET {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "reference output tree node limit",
            ));
        }
        if self.generated && limit.is_some_and(|limit| self.depth > limit) {
            return Err(declined(
                DiagnosticCode::ResourceLimit,
                "reference generated output depth limit",
            ));
        }
        Ok(())
    }

    /// The callback owns its allocation accounting. This independent, single iterative pass
    /// measures its actual returned tree without charging those materialized nodes again.
    fn callback(term: &Term, limit: Option<usize>) -> Result<Self> {
        let mut shape = Self {
            depth: 0,
            nodes: 0,
            generated: true,
        };
        let mut pending = vec![(term, 1usize)];
        while let Some((term, depth)) = pending.pop() {
            shape.nodes += 1;
            shape.depth = shape.depth.max(depth);
            shape.admit(limit)?;
            let next = depth.checked_add(1).ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "reference output depth overflow",
                )
            })?;
            // Reserve traversal inventory before extending any variable-length child list.
            let child_count = match term {
                Term::Pi { .. } | Term::Lambda { .. } | Term::App(..) => Some(2),
                Term::Let { .. } => Some(3),
                Term::Match { cases, .. } => cases.len().checked_add(2),
                Term::MutualFix { defs, .. } => Some(defs.len()),
                Term::Fix { .. } | Term::Annotated { .. } => Some(1),
                _ => Some(0),
            };
            let child_count = child_count.ok_or_else(|| {
                declined(
                    DiagnosticCode::ResourceLimit,
                    "reference output child inventory overflow",
                )
            })?;
            if child_count
                > REFERENCE_NODE_BUDGET
                    .saturating_sub(shape.nodes)
                    .saturating_sub(pending.len())
            {
                return Err(declined(
                    DiagnosticCode::ResourceLimit,
                    "reference output child inventory limit",
                ));
            }
            match term {
                Term::Pi {
                    param_type,
                    body_type,
                    ..
                } => {
                    pending.push((param_type, next));
                    pending.push((body_type, next));
                }
                Term::Lambda {
                    param_type, body, ..
                } => {
                    pending.push((param_type, next));
                    pending.push((body, next));
                }
                Term::App(fun, arg) => {
                    pending.push((fun, next));
                    pending.push((arg, next));
                }
                Term::Let {
                    ty, value, body, ..
                } => {
                    pending.push((ty, next));
                    pending.push((value, next));
                    pending.push((body, next));
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    pending.push((discriminant, next));
                    pending.push((motive, next));
                    pending.extend(cases.iter().map(|term| (term, next)));
                }
                Term::Fix { body, .. } => pending.push((body, next)),
                Term::MutualFix { defs, .. } => {
                    pending.extend(defs.iter().map(|(_, term)| (term, next)))
                }
                Term::Annotated { term, .. } => pending.push((term, next)),
                Term::Sort(_)
                | Term::Const { .. }
                | Term::Var(_)
                | Term::Global(_)
                | Term::Lit(_)
                | Term::Hole => {}
            }
        }
        Ok(shape)
    }
}

struct OutputTerm {
    term: Term,
    shape: OutputShape,
}
impl OutputTerm {
    fn leaf(term: Term) -> Self {
        Self {
            term,
            shape: OutputShape::LEAF,
        }
    }
}

struct NoProjection;
impl ReferenceProjection for NoProjection {
    fn project(
        &mut self,
        _: &str,
        _: usize,
        _: Term,
        _: &[ReferenceBinding<'_>],
        _: &mut usize,
    ) -> Result<Term> {
        Err(unsupported(
            "the reference translator needs a typed projection reader",
        ))
    }
}

/// An export read once, for reading many declarations from it.
pub struct Reference {
    tables: Tables,
}

impl Reference {
    pub fn read(export: &str) -> Result<Reference> {
        Ok(Reference {
            tables: Tables::read(export)?,
        })
    }

    /// One declaration, by its Lean or kernel spelling.
    pub fn declaration(&self, declaration: &str) -> Result<ReferenceDeclaration> {
        self.declaration_with(declaration, &mut NoProjection)
    }

    pub(super) fn declaration_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<ReferenceDeclaration> {
        self.declaration_lookup_with(declaration, projections, Lookup::Mixed)
    }

    pub(super) fn declaration_kernel(&self, declaration: &str) -> Result<ReferenceDeclaration> {
        self.declaration_kernel_with(declaration, &mut NoProjection)
    }

    pub(super) fn declaration_kernel_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<ReferenceDeclaration> {
        self.declaration_lookup_with(declaration, projections, Lookup::Kernel)
    }

    fn declaration_lookup_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
        lookup: Lookup,
    ) -> Result<ReferenceDeclaration> {
        let tables = &self.tables;
        let (record, kind) = tables.find_lookup(declaration, lookup)?;
        let name = tables.kernel_name(&record["name"])?;
        let lean_name = tables.lean_name(&record["name"])?;
        let level_params = match record.get("levelParams") {
            Some(Value::Array(params)) => params
                .iter()
                .map(|p| tables.raw_name(p))
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(malformed("declaration has no universe parameter list")),
        };
        let ty = tables.translate_with(&record["type"], projections)?;
        Ok(ReferenceDeclaration {
            name,
            lean_name,
            kind,
            level_params,
            ty,
        })
    }

    /// Decode a declaration's source body without admitting it to a kernel context. Callers must
    /// check the returned syntax under the intended dependency environment before trusting it.
    pub(super) fn value(&self, declaration: &str) -> Result<Term> {
        self.value_with(declaration, &mut NoProjection)
    }

    pub(super) fn value_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<Term> {
        self.value_lookup_with(declaration, projections, Lookup::Mixed)
    }

    pub(super) fn value_kernel(&self, declaration: &str) -> Result<Term> {
        self.value_kernel_with(declaration, &mut NoProjection)
    }

    pub(super) fn value_kernel_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
    ) -> Result<Term> {
        self.value_lookup_with(declaration, projections, Lookup::Kernel)
    }

    fn value_lookup_with(
        &self,
        declaration: &str,
        projections: &mut impl ReferenceProjection,
        lookup: Lookup,
    ) -> Result<Term> {
        let (record, kind) = self.tables.find_lookup(declaration, lookup)?;
        if !matches!(
            kind,
            ReferenceKind::Definition | ReferenceKind::Theorem | ReferenceKind::Opaque
        ) {
            return Err(unsupported("the source declaration has no body"));
        }
        self.tables.translate_with(&record["value"], projections)
    }

    /// Every declared name, in the kernel spelling, with its kind — without translating any type.
    pub fn names(&self) -> Result<Vec<(String, ReferenceKind)>> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for (record, kind) in self.tables.candidates() {
            let name = self.tables.kernel_name(&record["name"])?;
            if !seen.insert(name.clone()) {
                return Err(malformed(format!(
                    "the export declares kernel identity {name} more than once"
                )));
            }
            names.push((name, kind));
        }
        Ok(names)
    }
}

/// The kernel spelling of a Lean name given as its hierarchical components.
///
/// Every component is a string component; a numeric Lean component cannot be told apart from a
/// digit string by its dotted spelling, so callers holding an export resolve such names through
/// [`reference_declaration`], which matches against the export's own name table.
pub fn kernel_name_of_components(components: &[&str]) -> String {
    let mut raw = String::new();
    for component in components {
        raw.push_str("/s:");
        raw.push_str(&serde_json::to_string(component).expect("a string serializes"));
    }
    primitive_identity(&raw)
}

// This decoder owns its schema checks; it does not call the production reader's validators.
fn block_fields(value: &Value, expected: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| malformed("expected a source block record"))?;
    if object.len() != expected.len() || expected.iter().any(|field| !object.contains_key(*field)) {
        return Err(malformed(
            "source block record has missing or unexpected fields",
        ));
    }
    Ok(())
}

fn block_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value]> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| malformed(format!("source block {field} must be an array")))
}

fn block_bool(value: &Value, field: &str) -> Result<bool> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed(format!("source block {field} must be a Boolean")))
}

fn block_count(value: &Value, field: &str) -> Result<usize> {
    let count = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed(format!("source block {field} must be an unsigned integer")))?;
    usize::try_from(count)
        .map_err(|_| malformed(format!("source block {field} exceeds platform size")))
}

fn malformed(message: impl Into<String>) -> super::Verdict {
    declined(DiagnosticCode::MalformedExport, message)
}

fn unsupported(message: impl Into<String>) -> super::Verdict {
    declined(DiagnosticCode::UnsupportedFeature, message)
}

/// The kernel's primitive identities, applied to a raw hierarchical spelling.
fn primitive_identity(raw: &str) -> String {
    const NAT: &str = "/s:\"Nat\"";
    match raw {
        "/s:\"Quot\"" => "kernel.quot.type".into(),
        "/s:\"Quot\"/s:\"mk\"" => "kernel.quot.mk".into(),
        "/s:\"Quot\"/s:\"lift\"" => "kernel.quot.lift".into(),
        "/s:\"Quot\"/s:\"ind\"" => "kernel.quot.ind".into(),
        NAT => "Nat".into(),
        "/s:\"Nat\"/s:\"zero\"" => "Zero".into(),
        "/s:\"Nat\"/s:\"succ\"" => "Succ".into(),
        _ => match raw.strip_prefix("/s:\"Nat\"/") {
            Some(rest) => format!("Nat/{rest}"),
            None => raw.to_string(),
        },
    }
}

/// A name as its components: `(raw spelling, Lean spelling)`.
#[derive(Clone)]
struct Name {
    raw: String,
    lean: String,
}

/// External mixed-name convenience APIs keep their existing ambiguity rejection.
/// Internal callers already hold canonical kernel identities and must use only that namespace.
#[derive(Clone, Copy)]
enum Lookup {
    Mixed,
    Kernel,
    Lean,
}

struct Tables {
    names: HashMap<u64, Name>,
    levels: HashMap<u64, Value>,
    exprs: HashMap<u64, Value>,
    declarations: Vec<Value>,
    /// The header's `meta.lean.version`, which alone decides how a string literal reads.
    release: Option<String>,
}

/// The constant a Lean release applies to a string literal's `List Char`, as each release's own
/// `Init.Prelude` defines `String`. `None`: no reading is pinned for that release.
fn string_literal_packing(release: Option<&str>) -> Option<&'static str> {
    match release? {
        "4.14.0" => Some("/s:\"String\"/s:\"mk\""),
        "4.29.1" | "4.30.0" | "4.33.0" => Some("/s:\"String\"/s:\"ofList\""),
        _ => None,
    }
}

fn id(v: &Value) -> Result<u64> {
    v.as_u64().ok_or_else(|| malformed("expected an index"))
}

impl Tables {
    fn read(export: &str) -> Result<Tables> {
        let mut lines = export.lines();
        let header: Value =
            serde_json::from_str(lines.next().ok_or_else(|| malformed("empty export"))?)
                .map_err(|e| malformed(format!("export header: {e}")))?;
        if header["meta"]["format"]["version"] != "3.1.0" {
            return Err(declined(
                DiagnosticCode::UnsupportedExportVersion,
                "reference translation reads format 3.1.0 only",
            ));
        }
        let mut tables = Tables {
            names: HashMap::from([(
                0,
                Name {
                    raw: String::new(),
                    lean: String::new(),
                },
            )]),
            levels: HashMap::new(),
            exprs: HashMap::new(),
            declarations: Vec::new(),
            release: header["meta"]["lean"]["version"]
                .as_str()
                .map(str::to_owned),
        };
        for (at, line) in lines.enumerate() {
            let record: Value = serde_json::from_str(line)
                .map_err(|e| malformed(format!("line {}: {e}", at + 2)))?;
            if let Some(index) = record.get("in") {
                let index = id(index)?;
                let (tag, part) = if let Some(x) = record.get("str") {
                    let s = x["str"]
                        .as_str()
                        .ok_or_else(|| malformed("string name component"))?;
                    (
                        "s",
                        (
                            serde_json::to_string(s).expect("a string serializes"),
                            s.to_string(),
                        ),
                    )
                } else if let Some(x) = record.get("num") {
                    let n = id(&x["i"])?;
                    ("n", (n.to_string(), n.to_string()))
                } else {
                    return Err(malformed("unknown name record"));
                };
                let pre_record = record
                    .get("str")
                    .or_else(|| record.get("num"))
                    .expect("tagged above");
                let prefix = tables
                    .names
                    .get(&id(&pre_record["pre"])?)
                    .ok_or_else(|| malformed("name prefix is not an earlier name"))?
                    .clone();
                let lean = if prefix.lean.is_empty() {
                    part.1
                } else {
                    format!("{}.{}", prefix.lean, part.1)
                };
                let name = Name {
                    raw: format!("{}/{tag}:{}", prefix.raw, part.0),
                    lean,
                };
                if tables.names.insert(index, name).is_some() {
                    return Err(malformed("duplicate name index"));
                }
            } else if let Some(index) = record.get("il") {
                if tables.levels.insert(id(index)?, record).is_some() {
                    return Err(malformed("duplicate level index"));
                }
            } else if let Some(index) = record.get("ie") {
                if tables.exprs.insert(id(index)?, record).is_some() {
                    return Err(malformed("duplicate expression index"));
                }
            } else {
                tables.declarations.push(record);
            }
        }
        Ok(tables)
    }

    fn name_entry(&self, v: &Value) -> Result<&Name> {
        self.names
            .get(&id(v)?)
            .ok_or_else(|| malformed("unknown name index"))
    }

    fn raw_name(&self, v: &Value) -> Result<String> {
        Ok(self.name_entry(v)?.raw.clone())
    }

    fn kernel_name(&self, v: &Value) -> Result<String> {
        Ok(primitive_identity(&self.name_entry(v)?.raw))
    }

    fn lean_name(&self, v: &Value) -> Result<String> {
        Ok(self.name_entry(v)?.lean.clone())
    }

    /// Every declaration-shaped record with its kind: top-level declarations, and the types,
    /// constructors and recursors inside inductive blocks.
    fn candidates(&self) -> Vec<(&Value, ReferenceKind)> {
        let mut out = Vec::new();
        for record in &self.declarations {
            let Some(object) = record.as_object() else {
                continue;
            };
            for (tag, body) in object {
                match tag.as_str() {
                    "def" => out.push((body, ReferenceKind::Definition)),
                    "thm" => out.push((body, ReferenceKind::Theorem)),
                    "opaque" => out.push((body, ReferenceKind::Opaque)),
                    "axiom" => out.push((body, ReferenceKind::Axiom)),
                    "quot" => out.push((body, ReferenceKind::Quotient)),
                    "inductive" => {
                        for (field, kind) in [
                            ("types", ReferenceKind::Inductive),
                            ("ctors", ReferenceKind::Constructor),
                            ("recs", ReferenceKind::Recursor),
                        ] {
                            if let Some(Value::Array(items)) = body.get(field) {
                                out.extend(items.iter().map(|item| (item, kind)));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    fn find(&self, declaration: &str) -> Result<(&Value, ReferenceKind)> {
        self.find_lookup(declaration, Lookup::Mixed)
    }

    fn find_kernel(&self, declaration: &str) -> Result<(&Value, ReferenceKind)> {
        self.find_lookup(declaration, Lookup::Kernel)
    }

    fn find_lookup(&self, declaration: &str, lookup: Lookup) -> Result<(&Value, ReferenceKind)> {
        let mut found = Vec::new();
        for (record, kind) in self.candidates() {
            let name = self.name_entry(&record["name"])?;
            let matches = match lookup {
                Lookup::Mixed => {
                    name.lean == declaration
                        || name.raw == declaration
                        || primitive_identity(&name.raw) == declaration
                }
                Lookup::Kernel => primitive_identity(&name.raw) == declaration,
                Lookup::Lean => name.lean == declaration,
            };
            if matches {
                found.push((record, kind));
            }
        }
        match found.len() {
            0 => Err(rejected(
                DiagnosticCode::InvalidProof,
                format!("the export declares no {declaration}"),
            )),
            1 => Ok(found.remove(0)),
            _ => Err(rejected(
                DiagnosticCode::MalformedExport,
                format!("the export declares {declaration} more than once"),
            )),
        }
    }

    fn level(&self, v: &Value) -> Result<Universe> {
        let index = id(v)?;
        if index == 0 {
            return Ok(Universe::Prop);
        }
        enum Task {
            Enter(u64, usize),
            Succ,
            Max,
            IMax,
        }
        let mut work = vec![Task::Enter(index, 0)];
        let mut out: Vec<Universe> = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Task::Enter(i, depth) => {
                    if depth > 512 {
                        return Err(declined(
                            DiagnosticCode::ResourceLimit,
                            "universe level nesting",
                        ));
                    }
                    if i == 0 {
                        out.push(Universe::Prop);
                        continue;
                    }
                    let record = self
                        .levels
                        .get(&i)
                        .ok_or_else(|| malformed("unknown level index"))?;
                    if let Some(inner) = record.get("succ") {
                        work.push(Task::Succ);
                        work.push(Task::Enter(id(inner)?, depth + 1));
                    } else if let Some(param) = record.get("param") {
                        out.push(Universe::Var(self.raw_name(param)?));
                    } else if let Some(pair) = record.get("max").or_else(|| record.get("imax")) {
                        let pair = pair
                            .as_array()
                            .filter(|p| p.len() == 2)
                            .ok_or_else(|| malformed("level pair"))?;
                        work.push(if record.get("max").is_some() {
                            Task::Max
                        } else {
                            Task::IMax
                        });
                        work.push(Task::Enter(id(&pair[1])?, depth + 1));
                        work.push(Task::Enter(id(&pair[0])?, depth + 1));
                    } else {
                        return Err(malformed("unknown level record"));
                    }
                }
                Task::Succ => {
                    let u = out.pop().expect("successor operand");
                    out.push(u.succ());
                }
                Task::Max | Task::IMax => {
                    let b = out.pop().expect("right operand");
                    let a = out.pop().expect("left operand");
                    out.push(if matches!(task, Task::Max) {
                        Universe::Max(Box::new(a), Box::new(b))
                    } else {
                        Universe::IMax(Box::new(a), Box::new(b))
                    });
                }
            }
        }
        Ok(out.pop().expect("one level"))
    }

    /// Translate one expression, post-order, with an explicit stack. Bound variables are named by
    /// binder depth (`ref.<depth>`), so a de Bruijn index resolves to exactly one binder.
    fn translate(&self, root: &Value) -> Result<Term> {
        self.translate_with(root, &mut NoProjection)
    }

    fn translate_with(
        &self,
        root: &Value,
        projections: &mut impl ReferenceProjection,
    ) -> Result<Term> {
        // Direct decoding preserves the existing iterative million-node envelope. Package
        // callers first admit these roots against their stricter declared environment limits.
        // Do this before *any* source allocation or callback, including on the `_with` APIs.
        self.preflight_root(
            id(root)?,
            REFERENCE_NODE_BUDGET,
            REFERENCE_NODE_BUDGET as u64,
            &mut SourceMetrics::default(),
        )?;
        enum Task<'a> {
            Enter(u64),
            App,
            Projection { parent: String, index: usize },
            Binder { kind: &'a str, record: &'a Value },
            Close { kind: &'a str, name: String },
        }
        let mut work = vec![Task::Enter(id(root)?)];
        let mut out: Vec<OutputTerm> = Vec::new();
        let generated_depth_limit = projections.generated_depth_limit();
        let mut depth: usize = 0;
        // Domains and let values stay below the body on `out` until Close. Indices avoid
        // copying a whole telescope every time a nested binder is entered.
        let mut scope: Vec<(String, usize, Option<usize>)> = Vec::new();
        let mut budget = REFERENCE_NODE_BUDGET;
        let mut natural_work = 0u64;
        while let Some(task) = work.pop() {
            match task {
                Task::Enter(i) => {
                    budget = budget.checked_sub(1).ok_or_else(|| {
                        declined(
                            DiagnosticCode::ResourceLimit,
                            "reference translation node budget",
                        )
                    })?;
                    let record = self
                        .exprs
                        .get(&i)
                        .ok_or_else(|| malformed("unknown expression index"))?;
                    if let Some(u) = record.get("sort") {
                        out.push(OutputTerm::leaf(Term::Sort(self.level(u)?)));
                    } else if let Some(k) = record.get("bvar") {
                        let k = usize::try_from(id(k)?).map_err(|_| malformed("bound index"))?;
                        let at = depth.checked_sub(k + 1).ok_or_else(|| {
                            rejected(DiagnosticCode::MalformedExport, "loose bound variable")
                        })?;
                        out.push(OutputTerm::leaf(Term::Var(format!("ref.{at}"))));
                    } else if let Some(c) = record.get("const") {
                        let name = self.kernel_name(&c["name"])?;
                        let levels = c["us"]
                            .as_array()
                            .ok_or_else(|| malformed("constant levels"))?
                            .iter()
                            .map(|u| self.level(u))
                            .collect::<Result<Vec<_>>>()?;
                        out.push(OutputTerm::leaf(if levels.is_empty() {
                            Term::Global(name)
                        } else {
                            Term::Const { name, levels }
                        }));
                    } else if let Some(a) = record.get("app") {
                        work.push(Task::App);
                        work.push(Task::Enter(id(&a["arg"])?));
                        work.push(Task::Enter(id(&a["fn"])?));
                    } else if let Some(m) = record.get("mdata") {
                        work.push(Task::Enter(id(&m["expr"])?));
                    } else if let Some((kind, b)) = ["forallE", "lam", "letE"]
                        .iter()
                        .find_map(|k| record.get(*k).map(|b| (*k, b)))
                    {
                        // The domain (and a let's value) live outside the binder; the body inside.
                        work.push(Task::Binder { kind, record: b });
                        if kind == "letE" {
                            work.push(Task::Enter(id(&b["value"])?));
                        }
                        work.push(Task::Enter(id(&b["type"])?));
                    } else if let Some(n) = record.get("natVal") {
                        let significant = admit_natural_literal(n, &mut natural_work)?;
                        let value = if significant.is_empty() {
                            crate::BigInt::zero()
                        } else {
                            crate::BigInt::parse_decimal(significant)
                                .ok_or_else(|| malformed("natural literal"))?
                        };
                        out.push(OutputTerm::leaf(Term::Lit(Literal::Nat(value))));
                    } else if let Some(proj) = record.get("proj") {
                        let parent = self.kernel_name(&proj["typeName"])?;
                        let index = usize::try_from(id(&proj["idx"])?)
                            .map_err(|_| malformed("projection index"))?;
                        work.push(Task::Projection { parent, index });
                        work.push(Task::Enter(id(&proj["struct"])?));
                    } else if let Some(text) = record.get("strVal") {
                        let text = text.as_str().ok_or_else(|| malformed("string literal"))?;
                        let packing = string_literal_packing(self.release.as_deref()).ok_or_else(|| declined(
                            DiagnosticCode::UnsupportedExportVersion,
                            format!("the reference translator has no string-literal reading for Lean release {:?}", self.release),
                        ))?;
                        // `packing (List.cons Char (Char.ofNat c₁) (… (List.nil Char)))`: four nodes per
                        // scalar value plus the nil, the element type and the packing, spent up front.
                        let scalars: Vec<char> = text.chars().collect();
                        // The source preflight already admits this complete expansion. Record
                        // its term-only shape once; do not scan the growing list after each cons.
                        let shape = OutputShape {
                            depth: if scalars.is_empty() {
                                3
                            } else {
                                scalars.len() + 4
                            },
                            nodes: scalars
                                .len()
                                .checked_mul(8)
                                .and_then(|n| n.checked_add(5))
                                .ok_or_else(|| {
                                    declined(
                                        DiagnosticCode::ResourceLimit,
                                        "reference string output size overflow",
                                    )
                                })?,
                            generated: false,
                        };
                        shape.admit(generated_depth_limit)?;
                        budget = scalars
                            .len()
                            .checked_mul(4)
                            .and_then(|n| n.checked_add(3))
                            .and_then(|n| budget.checked_sub(n))
                            .ok_or_else(|| {
                                declined(
                                    DiagnosticCode::ResourceLimit,
                                    "reference translation node budget",
                                )
                            })?;
                        let character = || Term::Global("/s:\"Char\"".into());
                        let list_constant = |component: &str| Term::Const {
                            name: format!("/s:\"List\"/s:\"{component}\""),
                            levels: vec![Universe::Prop],
                        };
                        let mut list =
                            Term::App(Box::new(list_constant("nil")), Box::new(character()));
                        for scalar in scalars.into_iter().rev() {
                            let code_point = Term::App(
                                Box::new(Term::Global("/s:\"Char\"/s:\"ofNat\"".into())),
                                Box::new(Term::Lit(Literal::Nat(crate::BigInt::from_u64(
                                    u64::from(scalar),
                                )))),
                            );
                            let cons =
                                Term::App(Box::new(list_constant("cons")), Box::new(character()));
                            list = Term::App(
                                Box::new(Term::App(Box::new(cons), Box::new(code_point))),
                                Box::new(list),
                            );
                        }
                        out.push(OutputTerm {
                            term: Term::App(Box::new(Term::Global(packing.into())), Box::new(list)),
                            shape,
                        });
                    } else {
                        return Err(malformed("unknown expression record"));
                    }
                }
                Task::App => {
                    let arg = out.pop().expect("argument");
                    let fun = out.pop().expect("function");
                    let shape =
                        OutputShape::wrapper(&[fun.shape, arg.shape], generated_depth_limit)?;
                    out.push(OutputTerm {
                        term: Term::App(Box::new(fun.term), Box::new(arg.term)),
                        shape,
                    });
                }
                Task::Projection { parent, index } => {
                    let discriminant = out.pop().expect("projection discriminant");
                    let bindings: Vec<_> = scope
                        .iter()
                        .map(|(name, ty, value)| ReferenceBinding {
                            name,
                            ty: &out[*ty].term,
                            value: value.map(|at| &out[at].term),
                        })
                        .collect();
                    let projected = projections.project(
                        &parent,
                        index,
                        discriminant.term,
                        &bindings,
                        &mut budget,
                    )?;
                    let shape = OutputShape::callback(&projected, generated_depth_limit)?;
                    out.push(OutputTerm {
                        term: projected,
                        shape,
                    });
                }
                Task::Binder { kind, record } => {
                    let name = format!("ref.{depth}");
                    let (ty, value) = if kind == "letE" {
                        (out.len() - 2, Some(out.len() - 1))
                    } else {
                        (out.len() - 1, None)
                    };
                    scope.push((name.clone(), ty, value));
                    depth += 1;
                    work.push(Task::Close { kind, name });
                    work.push(Task::Enter(id(&record["body"])?));
                }
                Task::Close { kind, name } => {
                    scope.pop().expect("active source binder");
                    depth -= 1;
                    let body = out.pop().expect("binder body");
                    let output = match kind {
                        "letE" => {
                            let value = out.pop().expect("let value");
                            let ty = out.pop().expect("let type");
                            let shape = OutputShape::wrapper(
                                &[ty.shape, value.shape, body.shape],
                                generated_depth_limit,
                            )?;
                            OutputTerm {
                                term: Term::Let {
                                    name,
                                    ty: Box::new(ty.term),
                                    value: Box::new(value.term),
                                    body: Box::new(body.term),
                                },
                                shape,
                            }
                        }
                        "lam" => {
                            let ty = out.pop().expect("binder domain");
                            let shape = OutputShape::wrapper(
                                &[ty.shape, body.shape],
                                generated_depth_limit,
                            )?;
                            OutputTerm {
                                term: Term::Lambda {
                                    param: name,
                                    param_type: Box::new(ty.term),
                                    body: Box::new(body.term),
                                },
                                shape,
                            }
                        }
                        _ => {
                            let ty = out.pop().expect("binder domain");
                            let shape = OutputShape::wrapper(
                                &[ty.shape, body.shape],
                                generated_depth_limit,
                            )?;
                            OutputTerm {
                                term: Term::Pi {
                                    param: name,
                                    param_type: Box::new(ty.term),
                                    body_type: Box::new(body.term),
                                },
                                shape,
                            }
                        }
                    };
                    out.push(output);
                }
            }
        }
        Ok(out.pop().expect("one term").term)
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference/projection_scope_controls.rs"]
mod projection_scope_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference/block_controls.rs"]
mod block_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference/wrapper_controls.rs"]
mod wrapper_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference/natural_work_controls.rs"]
mod natural_work_controls;

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/interface/lean/reference/identity_controls.rs"]
mod identity_controls;
