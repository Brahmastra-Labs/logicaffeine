//! AST ⇄ e-graph conversion and the statement-level Architect pass.
//!
//! Each expression gets its OWN e-graph (mutation between statements can
//! never leak equalities), seeded with the Oracle's per-site facts. Two
//! stability rules keep the seeding sound:
//!
//! - **Opaque uniqueness**: every unmodeled subexpression (calls, list
//!   literals, ranges, …) becomes a FRESH opaque leaf per occurrence —
//!   two identical calls never unify, so effects are never deduplicated.
//! - **Fact stability**: Oracle facts are seeded only on subtrees whose
//!   value cannot change WITHIN one evaluation of the enclosing
//!   expression — literals, variables (no `Set` can run mid-expression),
//!   and pure arithmetic over those. `Index`/`Length`/opaque subtrees can
//!   observe a collection mutated by a sibling call, and two occurrences
//!   share one e-class, so per-site facts must not be intersected there.

use std::collections::HashMap;

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimize::OracleFacts;

use super::extract::{best_tree, best_tree_filtered, ExtractTree};
use super::{rules, CompilerEGraph, CompilerENode, NodeId};

pub struct Converter<'a> {
    pub eg: CompilerEGraph,
    opaques: Vec<&'a Expr<'a>>,
    symbols: HashMap<u32, Symbol>,
    /// Length-range facts staged on Len nodes — applied only when the
    /// WHOLE converted expression is opaque-free (no call can mutate the
    /// collection mid-evaluation, so the statement-entry snapshot holds
    /// for every occurrence).
    pending_len_facts: Vec<(NodeId, (i64, i64))>,
    saw_opaque: bool,
    /// CURRENT version per symbol index (cross-statement runs): a `Set`
    /// or rebinding bumps its target, so equality never leaks across a
    /// write. Absent = version 0.
    versions: HashMap<u32, u32>,
    /// Symbols read AS COLLECTIONS so far (the collection child of
    /// Len/Index/Slice/Copy/Contains): any collection mutation bumps them
    /// ALL — aliasing between collections is invisible here, so contents
    /// equalities die wholesale while scalar equalities survive.
    coll_read_vars: std::collections::HashSet<u32>,
    /// Symbol indices whose VALUE facts (scalar value-kind, integer range,
    /// length range) must NOT be applied: variables mutated inside the
    /// enclosing loop. The Oracle records facts per OCCURRENCE, and a loop
    /// guard like `d * d <= i` carries `d`'s FIRST-ITERATION range — sound to
    /// read point-wise (v1 does), UNSOUND as a universal e-graph rewrite
    /// (`d * d → 4` collapsed primes). KIND facts (is-collection) stay; they
    /// are mutation-immune.
    suppressed: std::collections::HashSet<u32>,
}

impl<'a> Converter<'a> {
    pub fn new() -> Self {
        Converter {
            eg: CompilerEGraph::new(),
            opaques: Vec::new(),
            symbols: HashMap::new(),
            pending_len_facts: Vec::new(),
            saw_opaque: false,
            versions: HashMap::new(),
            coll_read_vars: std::collections::HashSet::new(),
            suppressed: std::collections::HashSet::new(),
        }
    }

    fn version_of(&self, ix: u32) -> u32 {
        self.versions.get(&ix).copied().unwrap_or(0)
    }

    /// Rebinding / mutation: later reads see a FRESH version.
    fn bump(&mut self, sym: Symbol) {
        *self.versions.entry(sym.index() as u32).or_insert(0) += 1;
    }

    /// A collection's CONTENTS changed somewhere: every symbol ever read
    /// as a collection gets a fresh version (aliases are indistinguishable
    /// at this level).
    fn bump_collection_reads(&mut self) {
        let vars: Vec<u32> = self.coll_read_vars.iter().copied().collect();
        for ix in vars {
            *self.versions.entry(ix).or_insert(0) += 1;
        }
    }

    fn note_collection_read(&mut self, collection: &Expr) {
        if let Expr::Identifier(sym) = collection {
            self.coll_read_vars.insert(sym.index() as u32);
        }
    }

    /// True if `expr` references any suppressed (loop-mutated) variable. Such an
    /// expression's Oracle facts are a single-iteration snapshot, so they must
    /// not seed a universal rewrite. Recursive; cheap-exits when nothing is
    /// suppressed (the common, non-loop case).
    fn mentions_suppressed(&self, expr: &Expr) -> bool {
        if self.suppressed.is_empty() {
            return false;
        }
        match expr {
            Expr::Identifier(sym) => self.suppressed.contains(&(sym.index() as u32)),
            Expr::BinaryOp { left, right, .. } => {
                self.mentions_suppressed(left) || self.mentions_suppressed(right)
            }
            Expr::Not { operand } => self.mentions_suppressed(operand),
            Expr::Index { collection, index } => {
                self.mentions_suppressed(collection) || self.mentions_suppressed(index)
            }
            Expr::Length { collection } => self.mentions_suppressed(collection),
            Expr::Copy { expr } => self.mentions_suppressed(expr),
            Expr::Slice { collection, start, end } => {
                self.mentions_suppressed(collection)
                    || self.mentions_suppressed(start)
                    || self.mentions_suppressed(end)
            }
            Expr::Contains { collection, value } => {
                self.mentions_suppressed(collection) || self.mentions_suppressed(value)
            }
            _ => false,
        }
    }

    /// Stage opaque-free-gated facts now that conversion is complete.
    pub fn finish_facts(&mut self) {
        if !self.saw_opaque {
            for (id, (lo, hi)) in std::mem::take(&mut self.pending_len_facts) {
                self.eg.set_int_range(id, lo, hi);
            }
        }
    }

    fn opaque(&mut self, expr: &'a Expr<'a>) -> (NodeId, bool) {
        self.saw_opaque = true;
        let ix = self.opaques.len() as u32;
        self.opaques.push(expr);
        let id = self.eg.add(CompilerENode::Opaque(ix));
        // A list literal is opaque (effects in its items stay verbatim),
        // but its VALUE is unconditionally a list.
        if matches!(expr, Expr::List(_)) {
            self.eg.set_list(id);
        }
        (id, false)
    }

    /// Returns (class, stable). `stable` = the subtree's value cannot
    /// change within one evaluation of the enclosing expression.
    pub fn to_node(&mut self, expr: &'a Expr<'a>, facts: Option<&OracleFacts>) -> (NodeId, bool) {
        let (id, stable) = match expr {
            Expr::Literal(Literal::Number(n)) => (self.eg.add(CompilerENode::Int(*n)), true),
            Expr::Literal(Literal::Boolean(b)) => (self.eg.add(CompilerENode::Bool(*b)), true),
            Expr::Literal(Literal::Float(f)) => {
                (self.eg.add(CompilerENode::Float(f.to_bits())), true)
            }
            Expr::Identifier(sym) => {
                let ix = sym.index() as u32;
                self.symbols.insert(ix, *sym);
                let v = self.version_of(ix);
                let id = self.eg.add(CompilerENode::Var(ix, v));
                // KIND facts are mutation-immune (contents change, kinds
                // never do) — seed them unconditionally.
                if let Some(f) = facts {
                    if f.expr_is_tracked_collection(expr) {
                        self.eg.set_collection(id);
                    }
                }
                (id, true)
            }
            // ExactDivide (exact Rational division) has no integer e-node model and must
            // NEVER be floor-folded by the e-graph — treat the whole division as an opaque
            // leaf so the optimizer leaves it verbatim for the runtime.
            Expr::BinaryOp { op: BinaryOpKind::ExactDivide, .. } => self.opaque(expr),
            Expr::BinaryOp { op, left, right } => {
                let (l, ls) = self.to_node(left, facts);
                let (r, rs) = self.to_node(right, facts);
                let node = match op {
                    BinaryOpKind::Add => CompilerENode::Add(l, r),
                    BinaryOpKind::Subtract => CompilerENode::Sub(l, r),
                    BinaryOpKind::Multiply => CompilerENode::Mul(l, r),
                    BinaryOpKind::Divide => CompilerENode::Div(l, r),
                    BinaryOpKind::ExactDivide => unreachable!("ExactDivide handled by the opaque arm above"),
                    BinaryOpKind::Modulo => CompilerENode::Mod(l, r),
                    BinaryOpKind::Eq => CompilerENode::Eq(l, r),
                    BinaryOpKind::NotEq => CompilerENode::Ne(l, r),
                    BinaryOpKind::Lt => CompilerENode::Lt(l, r),
                    BinaryOpKind::Gt => CompilerENode::Gt(l, r),
                    BinaryOpKind::LtEq => CompilerENode::Le(l, r),
                    BinaryOpKind::GtEq => CompilerENode::Ge(l, r),
                    BinaryOpKind::And => CompilerENode::And(l, r),
                    BinaryOpKind::Or => CompilerENode::Or(l, r),
                    BinaryOpKind::Concat => CompilerENode::Concat(l, r),
                    BinaryOpKind::BitXor => CompilerENode::BitXor(l, r),
                    BinaryOpKind::Shl => CompilerENode::Shl(l, r),
                    BinaryOpKind::Shr => CompilerENode::Shr(l, r),
                };
                (self.eg.add(node), ls && rs)
            }
            Expr::Not { operand } => {
                let (o, os) = self.to_node(operand, facts);
                (self.eg.add(CompilerENode::Not(o)), os)
            }
            Expr::Index { collection, index } => {
                self.note_collection_read(collection);
                let (c, _) = self.to_node(collection, facts);
                let (i, _) = self.to_node(index, facts);
                // Collection contents can change under a sibling call.
                (self.eg.add(CompilerENode::Index(c, i)), false)
            }
            Expr::Length { collection } => {
                self.note_collection_read(collection);
                let (c, _) = self.to_node(collection, facts);
                let id = self.eg.add(CompilerENode::Len(c));
                // Length-range facts hold at statement entry; they apply
                // only if NO opaque (call) can mutate mid-evaluation —
                // staged here, applied by finish_facts().
                if let Some(f) = facts {
                    if !self.mentions_suppressed(collection) {
                        if let Some((lo, hi)) = f.expr_len_range(collection) {
                            self.pending_len_facts.push((id, (lo, hi)));
                        }
                    }
                }
                (id, false)
            }
            Expr::Copy { expr: inner } => {
                self.note_collection_read(inner);
                let (c, _) = self.to_node(inner, facts);
                let id = self.eg.add(CompilerENode::Copy(c));
                // Copy preserves its operand's kind.
                if self.eg.proven_list(c) {
                    self.eg.set_list(id);
                } else if self.eg.proven_collection(c) {
                    self.eg.set_collection(id);
                }
                (id, false)
            }
            Expr::Slice { collection, start, end } => {
                self.note_collection_read(collection);
                let (c, _) = self.to_node(collection, facts);
                let (s, _) = self.to_node(start, facts);
                let (e, _) = self.to_node(end, facts);
                (self.eg.add(CompilerENode::Slice(c, s, e)), false)
            }
            Expr::Contains { collection, value } => {
                self.note_collection_read(collection);
                let (c, _) = self.to_node(collection, facts);
                let (v, _) = self.to_node(value, facts);
                (self.eg.add(CompilerENode::Contains(c, v)), false)
            }
            other => self.opaque(other),
        };
        if stable {
            if let Some(f) = facts {
                if !self.mentions_suppressed(expr) {
                    if let Some(k) = f.expr_scalar(expr) {
                        self.eg.set_scalar(id, k);
                    }
                    if let Some((lo, hi)) = f.expr_int_range(expr) {
                        self.eg.set_int_range(id, lo, hi);
                    }
                }
            }
        }
        (id, stable)
    }

    pub fn tree_to_expr(
        &self,
        t: &ExtractTree,
        arena: &'a Arena<Expr<'a>>,
    ) -> &'a Expr<'a> {
        let bin = |op: BinaryOpKind, s: &Self| -> &'a Expr<'a> {
            arena.alloc(Expr::BinaryOp {
                op,
                left: s.tree_to_expr(&t.children[0], arena),
                right: s.tree_to_expr(&t.children[1], arena),
            })
        };
        match t.node {
            CompilerENode::Int(n) => arena.alloc(Expr::Literal(Literal::Number(n))),
            CompilerENode::Bool(b) => arena.alloc(Expr::Literal(Literal::Boolean(b))),
            CompilerENode::Float(bits) => {
                arena.alloc(Expr::Literal(Literal::Float(f64::from_bits(bits))))
            }
            CompilerENode::Var(ix, _) => arena.alloc(Expr::Identifier(self.symbols[&ix])),
            CompilerENode::Opaque(ix) => self.opaques[ix as usize],
            CompilerENode::Not(_) => arena.alloc(Expr::Not {
                operand: self.tree_to_expr(&t.children[0], arena),
            }),
            CompilerENode::Len(_) => arena.alloc(Expr::Length {
                collection: self.tree_to_expr(&t.children[0], arena),
            }),
            CompilerENode::Index(..) => arena.alloc(Expr::Index {
                collection: self.tree_to_expr(&t.children[0], arena),
                index: self.tree_to_expr(&t.children[1], arena),
            }),
            CompilerENode::Copy(_) => arena.alloc(Expr::Copy {
                expr: self.tree_to_expr(&t.children[0], arena),
            }),
            CompilerENode::Slice(..) => arena.alloc(Expr::Slice {
                collection: self.tree_to_expr(&t.children[0], arena),
                start: self.tree_to_expr(&t.children[1], arena),
                end: self.tree_to_expr(&t.children[2], arena),
            }),
            CompilerENode::Contains(..) => arena.alloc(Expr::Contains {
                collection: self.tree_to_expr(&t.children[0], arena),
                value: self.tree_to_expr(&t.children[1], arena),
            }),
            CompilerENode::Add(..) => bin(BinaryOpKind::Add, self),
            CompilerENode::Sub(..) => bin(BinaryOpKind::Subtract, self),
            CompilerENode::Mul(..) => bin(BinaryOpKind::Multiply, self),
            CompilerENode::Div(..) => bin(BinaryOpKind::Divide, self),
            CompilerENode::Mod(..) => bin(BinaryOpKind::Modulo, self),
            CompilerENode::Shl(..) => bin(BinaryOpKind::Shl, self),
            CompilerENode::Shr(..) => bin(BinaryOpKind::Shr, self),
            CompilerENode::BitXor(..) => bin(BinaryOpKind::BitXor, self),
            CompilerENode::And(..) => bin(BinaryOpKind::And, self),
            CompilerENode::Or(..) => bin(BinaryOpKind::Or, self),
            CompilerENode::Eq(..) => bin(BinaryOpKind::Eq, self),
            CompilerENode::Ne(..) => bin(BinaryOpKind::NotEq, self),
            CompilerENode::Lt(..) => bin(BinaryOpKind::Lt, self),
            CompilerENode::Le(..) => bin(BinaryOpKind::LtEq, self),
            CompilerENode::Gt(..) => bin(BinaryOpKind::Gt, self),
            CompilerENode::Ge(..) => bin(BinaryOpKind::GtEq, self),
            CompilerENode::Concat(..) => bin(BinaryOpKind::Concat, self),
        }
    }
}

/// Structural equality over the MODELED fragment (bit-exact floats);
/// anything else compares by pointer (opaques re-emit the original).
fn expr_struct_eq(a: &Expr, b: &Expr) -> bool {
    if std::ptr::eq(a, b) {
        return true;
    }
    match (a, b) {
        (Expr::Literal(Literal::Number(x)), Expr::Literal(Literal::Number(y))) => x == y,
        (Expr::Literal(Literal::Boolean(x)), Expr::Literal(Literal::Boolean(y))) => x == y,
        (Expr::Literal(Literal::Float(x)), Expr::Literal(Literal::Float(y))) => {
            x.to_bits() == y.to_bits()
        }
        (Expr::Identifier(x), Expr::Identifier(y)) => x == y,
        (
            Expr::BinaryOp { op: oa, left: la, right: ra },
            Expr::BinaryOp { op: ob, left: lb, right: rb },
        ) => oa == ob && expr_struct_eq(la, lb) && expr_struct_eq(ra, rb),
        (Expr::Not { operand: x }, Expr::Not { operand: y }) => expr_struct_eq(x, y),
        (Expr::Length { collection: x }, Expr::Length { collection: y }) => expr_struct_eq(x, y),
        (
            Expr::Index { collection: ca, index: ia },
            Expr::Index { collection: cb, index: ib },
        ) => expr_struct_eq(ca, cb) && expr_struct_eq(ia, ib),
        (Expr::Copy { expr: a }, Expr::Copy { expr: b }) => expr_struct_eq(a, b),
        (
            Expr::Slice { collection: ca, start: sa, end: ea },
            Expr::Slice { collection: cb, start: sb, end: eb },
        ) => expr_struct_eq(ca, cb) && expr_struct_eq(sa, sb) && expr_struct_eq(ea, eb),
        (
            Expr::Contains { collection: ca, value: va },
            Expr::Contains { collection: cb, value: vb },
        ) => expr_struct_eq(ca, cb) && expr_struct_eq(va, vb),
        _ => false,
    }
}

/// Does the expression contain anything the e-graph can act on?
fn worth_modeling(expr: &Expr) -> bool {
    match expr {
        Expr::BinaryOp { .. } | Expr::Not { .. } => true,
        // A bare `length of xs` / `item i of xs` has nothing to improve,
        // but the same reads over a COPY or SLICE are fusion's home turf.
        Expr::Index { collection, index } => {
            fusable(collection) || worth_modeling(collection) || worth_modeling(index)
        }
        Expr::Length { collection } => fusable(collection) || worth_modeling(collection),
        Expr::Copy { expr } => fusable(expr) || worth_modeling(expr),
        Expr::Slice { collection, start, end } => {
            worth_modeling(collection) || worth_modeling(start) || worth_modeling(end)
        }
        Expr::Contains { collection, value } => {
            worth_modeling(collection) || worth_modeling(value)
        }
        _ => false,
    }
}

/// Operands whose shape the fusion algebra can act on directly.
fn fusable(expr: &Expr) -> bool {
    matches!(expr, Expr::Copy { .. } | Expr::Slice { .. })
}

/// Saturate ONE expression against the rule set and extract the cheapest
/// equivalent. Returns the original pointer when nothing improved.
pub fn simplify_expr<'a>(
    expr: &'a Expr<'a>,
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    simplify_expr_in(expr, facts, rule_set, arena, &std::collections::HashSet::new())
}

/// As [`simplify_expr`], but suppresses value facts for the `suppressed`
/// (loop-mutated) symbols — their Oracle per-occurrence range is iteration-
/// specific and may not seed a universal rewrite.
fn simplify_expr_in<'a>(
    expr: &'a Expr<'a>,
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    arena: &'a Arena<Expr<'a>>,
    suppressed: &std::collections::HashSet<u32>,
) -> &'a Expr<'a> {
    if !worth_modeling(expr) {
        return expr;
    }
    let mut cv = Converter::new();
    cv.suppressed = suppressed.clone();
    let (root, _) = cv.to_node(expr, Some(facts));
    cv.finish_facts();
    cv.eg.saturate(rule_set);
    let tree = best_tree(&mut cv.eg, root);
    let rebuilt = cv.tree_to_expr(&tree, arena);
    if expr_struct_eq(rebuilt, expr) {
        expr
    } else {
        rebuilt
    }
}

/// The Architect statement pass: STRAIGHT-LINE RUNS of simple statements
/// share one e-graph (a Let merges its variable with its defining
/// expression's class, so later statements extract against everything
/// proven so far — the GVN replacement); everything else falls back to
/// per-expression graphs and ENDS the run. Versioning makes mutation
/// kills structural.
pub fn egraph_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    let facts = crate::optimize::oracle_analyze_with(&stmts, interner);
    let rule_set = rules::all();
    walk_stmts(stmts, &facts, &rule_set, expr_arena, stmt_arena, &std::collections::HashSet::new())
}

/// Statements a cross-statement run can model EXACTLY (effects and kills
/// understood). Anything else flushes the run.
fn run_modeled(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::Let { .. }
            | Stmt::Set { .. }
            | Stmt::Show { .. }
            | Stmt::RuntimeAssert { .. }
            | Stmt::Return { .. }
            | Stmt::Push { .. }
            | Stmt::SetIndex { .. }
            | Stmt::Call { .. }
    )
}

/// One extraction site inside a run: the converted root, the VERSION
/// SNAPSHOT it may reference, the opaque indices it owns, and the
/// original expression (the fallback and the no-change comparator).
struct SiteRec<'a> {
    root: NodeId,
    ctx: HashMap<u32, u32>,
    opq: (usize, usize),
    original: &'a Expr<'a>,
}

fn walk_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    suppressed: &std::collections::HashSet<u32>,
) -> Vec<Stmt<'a>> {
    let mut out: Vec<Stmt<'a>> = Vec::new();
    let mut run: Vec<Stmt<'a>> = Vec::new();
    for s in stmts {
        if run_modeled(&s) {
            run.push(s);
        } else {
            flush_run(&mut run, &mut out, facts, rule_set, expr_arena, suppressed);
            out.push(walk_stmt(s, facts, rule_set, expr_arena, stmt_arena, suppressed));
        }
    }
    flush_run(&mut run, &mut out, facts, rule_set, expr_arena, suppressed);
    out
}

/// Convert a run's expression sites into ONE shared graph (with version
/// bookkeeping per statement), saturate once, then rebuild every
/// statement from its site's filtered extraction.
fn flush_run<'a>(
    run: &mut Vec<Stmt<'a>>,
    out: &mut Vec<Stmt<'a>>,
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    expr_arena: &'a Arena<Expr<'a>>,
    suppressed: &std::collections::HashSet<u32>,
) {
    if run.is_empty() {
        return;
    }
    let stmts = std::mem::take(run);
    let mut cv = Converter::new();
    cv.suppressed = suppressed.clone();
    let mut sites: Vec<SiteRec<'a>> = Vec::new();
    // Per statement: the site indices feeding its rebuild, in field order.
    let mut plans: Vec<Vec<usize>> = Vec::with_capacity(stmts.len());

    let mut convert_site = |cv: &mut Converter<'a>,
                            sites: &mut Vec<SiteRec<'a>>,
                            e: &'a Expr<'a>|
     -> usize {
        let opq_start = cv.opaques.len();
        let (root, _) = cv.to_node(e, Some(facts));
        let opq_end = cv.opaques.len();
        sites.push(SiteRec {
            root,
            ctx: cv.versions.clone(),
            opq: (opq_start, opq_end),
            original: e,
        });
        sites.len() - 1
    };

    for s in &stmts {
        let mut plan: Vec<usize> = Vec::new();
        match s {
            Stmt::Let { var, value, .. } => {
                let site = convert_site(&mut cv, &mut sites, value);
                plan.push(site);
                let had_opaques = sites[site].opq.0 != sites[site].opq.1;
                cv.bump(*var);
                if !had_opaques {
                    let ix = var.index() as u32;
                    cv.symbols.insert(ix, *var);
                    let v = cv.version_of(ix);
                    let vnode = cv.eg.add(CompilerENode::Var(ix, v));
                    let root = sites[site].root;
                    cv.eg.union(vnode, root);
                } else {
                    // The binding's value went through a call — its
                    // collection arguments may have been resized.
                    cv.bump_collection_reads();
                }
            }
            Stmt::Set { target, value } => {
                let site = convert_site(&mut cv, &mut sites, value);
                plan.push(site);
                let had_opaques = sites[site].opq.0 != sites[site].opq.1;
                cv.bump(*target);
                if !had_opaques {
                    let ix = target.index() as u32;
                    cv.symbols.insert(ix, *target);
                    let v = cv.version_of(ix);
                    let vnode = cv.eg.add(CompilerENode::Var(ix, v));
                    let root = sites[site].root;
                    cv.eg.union(vnode, root);
                } else {
                    cv.bump_collection_reads();
                }
            }
            Stmt::Show { object, .. } => {
                plan.push(convert_site(&mut cv, &mut sites, object));
                // Show stringifies arbitrary values through an opaque-free
                // read — no kills.
            }
            Stmt::RuntimeAssert { condition, .. } => {
                plan.push(convert_site(&mut cv, &mut sites, condition));
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    plan.push(convert_site(&mut cv, &mut sites, v));
                }
            }
            Stmt::Push { value, collection } => {
                plan.push(convert_site(&mut cv, &mut sites, value));
                // Contents change: every collection read goes stale.
                if let Expr::Identifier(sym) = collection {
                    cv.bump(*sym);
                }
                cv.bump_collection_reads();
            }
            Stmt::SetIndex { collection, index, value } => {
                plan.push(convert_site(&mut cv, &mut sites, index));
                plan.push(convert_site(&mut cv, &mut sites, value));
                if let Expr::Identifier(sym) = collection {
                    cv.bump(*sym);
                }
                cv.bump_collection_reads();
            }
            Stmt::Call { args, .. } => {
                for a in args {
                    plan.push(convert_site(&mut cv, &mut sites, a));
                }
                // The callee may resize any collection it can reach.
                cv.bump_collection_reads();
            }
            _ => unreachable!("run_modeled admitted an unhandled statement"),
        }
        plans.push(plan);
    }

    cv.finish_facts();
    cv.eg.saturate(rule_set);

    let mut extract = |cv: &mut Converter<'a>, site: usize| -> &'a Expr<'a> {
        let rec = &sites[site];
        if !worth_modeling(rec.original) {
            return rec.original;
        }
        let ctx = rec.ctx.clone();
        let (lo, hi) = rec.opq;
        let admissible = move |n: &CompilerENode| match *n {
            CompilerENode::Var(ix, v) => ctx.get(&ix).copied().unwrap_or(0) == v,
            CompilerENode::Opaque(j) => (lo..hi).contains(&(j as usize)),
            _ => true,
        };
        match best_tree_filtered(&mut cv.eg, rec.root, &admissible) {
            Some(tree) => {
                let rebuilt = cv.tree_to_expr(&tree, expr_arena);
                if expr_struct_eq(rebuilt, rec.original) {
                    rec.original
                } else {
                    rebuilt
                }
            }
            None => rec.original,
        }
    };

    for (s, plan) in stmts.into_iter().zip(plans) {
        let rebuilt = match s {
            Stmt::Let { var, ty, value: _, mutable } => Stmt::Let {
                var,
                ty,
                value: extract(&mut cv, plan[0]),
                mutable,
            },
            Stmt::Set { target, value: _ } => {
                Stmt::Set { target, value: extract(&mut cv, plan[0]) }
            }
            Stmt::Show { object: _, recipient } => {
                Stmt::Show { object: extract(&mut cv, plan[0]), recipient }
            }
            Stmt::RuntimeAssert { condition: _ , hard } => {
                Stmt::RuntimeAssert { condition: extract(&mut cv, plan[0]) , hard }
            }
            Stmt::Return { value } => Stmt::Return {
                value: value.map(|_| extract(&mut cv, plan[0])),
            },
            Stmt::Push { value: _, collection } => {
                Stmt::Push { value: extract(&mut cv, plan[0]), collection }
            }
            Stmt::SetIndex { collection, index: _, value: _ } => Stmt::SetIndex {
                collection,
                index: extract(&mut cv, plan[0]),
                value: extract(&mut cv, plan[1]),
            },
            Stmt::Call { function, args } => Stmt::Call {
                function,
                args: (0..args.len()).map(|k| extract(&mut cv, plan[k])).collect(),
            },
            other => other,
        };
        out.push(rebuilt);
    }
}

fn walk_block<'a>(
    block: &'a [Stmt<'a>],
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    suppressed: &std::collections::HashSet<u32>,
) -> &'a [Stmt<'a>] {
    let walked = walk_stmts(
        block.to_vec(),
        facts,
        rule_set,
        expr_arena,
        stmt_arena,
        suppressed,
    );
    stmt_arena.alloc_slice(walked)
}

/// Accumulate the symbol indices ASSIGNED or BOUND anywhere in `stmts`
/// (recursively): `Set`/`Let` targets, `Pop`/`Read` bindings, and the
/// collections that `Push`/`Pop`/`Add`/`Remove`/`SetIndex`/`SetField` mutate.
/// A variable in this set is loop-variant, so inside the loop its Oracle
/// per-occurrence facts must not drive a universal rewrite.
fn collect_mutated(stmts: &[Stmt], out: &mut std::collections::HashSet<u32>) {
    for s in stmts {
        match s {
            Stmt::Set { target, .. } => {
                out.insert(target.index() as u32);
            }
            Stmt::Let { var, .. } => {
                out.insert(var.index() as u32);
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_mutated(then_block, out);
                if let Some(e) = else_block {
                    collect_mutated(e, out);
                }
            }
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Zone { body, .. } => collect_mutated(body, out),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => collect_mutated(tasks, out),
            Stmt::Inspect { arms, .. } => {
                for a in arms {
                    collect_mutated(a.body, out);
                }
            }
            Stmt::Push { collection, .. }
            | Stmt::Add { collection, .. }
            | Stmt::Remove { collection, .. }
            | Stmt::SetIndex { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    out.insert(sym.index() as u32);
                }
            }
            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(sym) = collection {
                    out.insert(sym.index() as u32);
                }
                if let Some(v) = into {
                    out.insert(v.index() as u32);
                }
            }
            Stmt::SetField { object, .. } => {
                if let Expr::Identifier(sym) = object {
                    out.insert(sym.index() as u32);
                }
            }
            Stmt::ReadFrom { var, .. } => {
                out.insert(var.index() as u32);
            }
            _ => {}
        }
    }
}

fn walk_stmt<'a>(
    stmt: Stmt<'a>,
    facts: &OracleFacts,
    rule_set: &[rules::Rewrite],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    suppressed: &std::collections::HashSet<u32>,
) -> Stmt<'a> {
    let se = |e: &'a Expr<'a>| simplify_expr_in(e, facts, rule_set, expr_arena, suppressed);
    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            Stmt::Let { var, ty, value: se(value), mutable }
        }
        Stmt::Set { target, value } => Stmt::Set { target, value: se(value) },
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond: se(cond),
            then_block: walk_block(then_block, facts, rule_set, expr_arena, stmt_arena, suppressed),
            else_block: else_block
                .map(|b| walk_block(b, facts, rule_set, expr_arena, stmt_arena, suppressed)),
        },
        Stmt::While { cond, body, decreasing } => {
            // Inside the loop, every variable the body mutates is loop-variant;
            // its Oracle per-occurrence facts (a single-iteration snapshot) must
            // not seed a universal rewrite — `cond` is re-evaluated each pass.
            let mut inner = suppressed.clone();
            collect_mutated(body, &mut inner);
            Stmt::While {
                cond: simplify_expr_in(cond, facts, rule_set, expr_arena, &inner),
                body: walk_block(body, facts, rule_set, expr_arena, stmt_arena, &inner),
                decreasing,
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            // `iterable` is evaluated ONCE in the outer context; the body runs
            // per element, so loop-mutated vars are suppressed within it.
            let mut inner = suppressed.clone();
            collect_mutated(body, &mut inner);
            Stmt::Repeat {
                pattern,
                iterable: se(iterable),
                body: walk_block(body, facts, rule_set, expr_arena, stmt_arena, &inner),
            }
        }
        Stmt::FunctionDef {
            name,
            params,
            generics,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } => Stmt::FunctionDef {
            name,
            params,
            generics,
            body: walk_block(body, facts, rule_set, expr_arena, stmt_arena, suppressed),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        },
        Stmt::Show { object, recipient } => Stmt::Show { object: se(object), recipient },
        Stmt::Return { value } => Stmt::Return { value: value.map(se) },
        Stmt::RuntimeAssert { condition, hard } => Stmt::RuntimeAssert { condition: se(condition) , hard },
        Stmt::Push { value, collection } => Stmt::Push { value: se(value), collection },
        Stmt::SetField { object, field, value } => {
            Stmt::SetField { object, field, value: se(value) }
        }
        Stmt::SetIndex { collection, index, value } => {
            Stmt::SetIndex { collection, index: se(index), value: se(value) }
        }
        Stmt::Call { function, args } => Stmt::Call {
            function,
            args: args.into_iter().map(se).collect(),
        },
        Stmt::Give { object, recipient } => {
            Stmt::Give { object: se(object), recipient: se(recipient) }
        }
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target: se(target),
            arms: arms
                .into_iter()
                .map(|arm| crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: walk_block(arm.body, facts, rule_set, expr_arena, stmt_arena, suppressed),
                })
                .collect(),
            has_otherwise,
        },
        Stmt::Pop { collection, into } => Stmt::Pop { collection: se(collection), into },
        Stmt::Add { value, collection } => {
            Stmt::Add { value: se(value), collection: se(collection) }
        }
        Stmt::Remove { value, collection } => {
            Stmt::Remove { value: se(value), collection: se(collection) }
        }
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: walk_block(body, facts, rule_set, expr_arena, stmt_arena, suppressed),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: walk_block(tasks, facts, rule_set, expr_arena, stmt_arena, suppressed),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: walk_block(tasks, facts, rule_set, expr_arena, stmt_arena, suppressed),
        },
        Stmt::WriteFile { content, path } => {
            Stmt::WriteFile { content: se(content), path: se(path) }
        }
        Stmt::SendMessage { message, destination, compression, cached, unchecked, layout, shared, computed } => {
            Stmt::SendMessage { message: se(message), destination: se(destination), compression, cached, unchecked, layout, shared, computed }
        }
        Stmt::IncreaseCrdt { object, field, amount } => {
            Stmt::IncreaseCrdt { object: se(object), field, amount: se(amount) }
        }
        Stmt::DecreaseCrdt { object, field, amount } => {
            Stmt::DecreaseCrdt { object: se(object), field, amount: se(amount) }
        }
        Stmt::Sleep { milliseconds } => Stmt::Sleep { milliseconds: se(milliseconds) },
        Stmt::MergeCrdt { source, target } => {
            Stmt::MergeCrdt { source: se(source), target: se(target) }
        }
        other => other,
    }
}
