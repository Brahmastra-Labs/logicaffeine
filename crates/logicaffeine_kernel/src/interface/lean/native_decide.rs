//! `native_decide`, re-proved by the LOGOS kernel's own evaluation.
//!
//! # Lean's proof
//!
//! Lean 4.14.0 closes a closed decidable goal `p` with `native_decide` in two declarations
//! (`Lean.Elab.Tactic.evalNativeDecide`):
//!
//! ```text
//! def     T._nativeDecide_1 : Bool := Decidable.decide p inst
//! theorem T : p := of_decide_eq_true p inst
//!                     (Lean.ofReduceBool T._nativeDecide_1 Bool.true (Eq.refl.{1} Bool Bool.true))
//! ```
//!
//! `Lean.ofReduceBool d b h : d = b` is an axiom, and Lean's kernel accepts `Eq.refl b` as
//! `h : Lean.reduceBool d = b` only by running the compiled `d`: the proof rests on the compiler,
//! and `#print axioms` names `Lean.ofReduceBool`.
//!
//! # The re-derivation
//!
//! The step Lean hands to its compiler is a closed decision, and the LOGOS kernel decides it
//! itself. The leaf is read as
//!
//! ```text
//! of_decide_eq_true p inst (Eq.refl.{1} Bool Bool.true)
//! ```
//!
//! which checks exactly when the kernel converts `Bool.true` with `Decidable.decide p inst`: its own
//! reduction and certified accelerations, in the primary kernel and in the independent re-checker.
//! The statement is untouched; the justification of one step changes from Lean's compiled
//! evaluation to LOGOS's kernel evaluation, and the result discloses it. A decision the kernel
//! evaluates to `Bool.false` is a conversion mismatch, and the claim is refused; one it cannot
//! finish within its budget is a resource limit. Neither is ever trusted.
//!
//! # What is a leaf
//!
//! Structure, never a name alone. An export expression is a leaf when it is
//! `of_decide_eq_true p inst (Lean.ofReduceBool d Bool.true (Eq.refl.{1} Bool Bool.true))` and the
//! export declares
//!
//! * `Lean.ofReduceBool` as an axiom stating `∀ a b : Bool, Lean.reduceBool a = b → a = b`, over an
//!   opaque `Lean.reduceBool : Bool → Bool`;
//! * `of_decide_eq_true` as a theorem stating `∀ (p : Prop) (inst : Decidable p),
//!   Decidable.decide p inst = true → p`, with `Decidable.decide : ∀ p : Prop, Decidable p → Bool`;
//! * `d` as a definition without universe parameters, of type `Bool`, whose value is
//!   `Decidable.decide p inst` for the same `p` and `inst`, expression for expression (binder names
//!   and binder annotations aside);
//! * `Bool` as an inductive whose constructors are `Bool.false` and `Bool.true`, and `Eq` as an
//!   inductive whose one constructor is `Eq.refl`.
//!
//! Every other mention of `Lean.ofReduceBool` is read as exported and rests on the axiom; the
//! assumption policy refuses it.
//!
//! # Where it applies
//!
//! The LOGOS import surface ([`import_with`](super::import_with), hence the pipeline and the
//! package) reads every leaf of an export this way and lists each re-derivation in its report. The
//! Lean Kernel Arena protocol ([`check_str`](super::check_str), [`run_cli`](super::run_cli)) reads the
//! export exactly as exported. A package discloses the re-derived leaves of its target's closure as
//! [`INFORMATION_CATEGORY`] information changes, and a replay recomputes that disclosure from the
//! carried export and refuses a package whose manifest does not state it exactly.

use super::intake::{self, Component};
use super::{DiagnosticCode, Verdict};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The trust escape a re-derived leaf no longer rests on, as Lean's `#print axioms` names it.
pub const ELIMINATED_AXIOM: &str = "Lean.ofReduceBool";

/// The information-change category a re-derivation is disclosed under.
pub const INFORMATION_CATEGORY: &str = "NATIVE_DECIDE_REDERIVED";

/// How a disclosed step was re-derived.
pub const METHOD: &str = "kernel_evaluation";

const OF_DECIDE_EQ_TRUE: &[&str] = &["of_decide_eq_true"];
const OF_REDUCE_BOOL: &[&str] = &["Lean", "ofReduceBool"];
const REDUCE_BOOL: &[&str] = &["Lean", "reduceBool"];
const DECIDE: &[&str] = &["Decidable", "decide"];
const DECIDABLE: &[&str] = &["Decidable"];
const EQ: &[&str] = &["Eq"];
const EQ_REFL: &[&str] = &["Eq", "refl"];
const BOOL: &[&str] = &["Bool"];
const BOOL_FALSE: &[&str] = &["Bool", "false"];
const BOOL_TRUE: &[&str] = &["Bool", "true"];

/// One recognized leaf: Lean's appeal to its compiler, and the pieces its re-derivation keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeDecision {
    /// The leaf, `of_decide_eq_true p inst (Lean.ofReduceBool d Bool.true h)`.
    pub leaf: usize,
    /// `of_decide_eq_true p inst`, which the re-derived leaf keeps.
    pub premise: usize,
    /// Lean's appeal, `Lean.ofReduceBool d Bool.true h`, which the re-derived leaf drops.
    pub appeal: usize,
    /// `h = Eq.refl.{1} Bool Bool.true`, the proof of `Decidable.decide p inst = Bool.true` the
    /// re-derived leaf supplies directly.
    pub evaluation: usize,
    /// The proposition `p`.
    pub proposition: usize,
    /// Its decision procedure `inst : Decidable p`.
    pub instance: usize,
    /// The constant Lean compiled and evaluated, as Lean writes its name.
    pub decision: String,
}

impl NativeDecision {
    pub fn to_json(&self) -> Value {
        json!({
            "leaf_expression": self.leaf,
            "appeal_expression": self.appeal,
            "evaluation_expression": self.evaluation,
            "proposition_expression": self.proposition,
            "instance_expression": self.instance,
            "decision": self.decision,
        })
    }
}

/// One declaration whose type or value holds re-derived leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rederivation {
    /// The kernel identity of the declaration.
    pub declaration: String,
    /// Its Lean name, as Lean writes it.
    pub lean: String,
    /// Its leaves, in export order.
    pub decisions: Vec<NativeDecision>,
}

impl Rederivation {
    /// The disclosure, stated plainly: what Lean's proof rested on and what LOGOS did instead.
    pub fn detail(&self) -> String {
        let decisions: Vec<&str> = self.decisions.iter().map(|d| d.decision.as_str()).collect();
        let steps = if decisions.len() == 1 {
            "one step".to_string()
        } else {
            format!("{} steps", decisions.len())
        };
        format!(
            "{}: Lean proved {steps} by native_decide, resting on {ELIMINATED_AXIOM} (Lean's compiled \
             evaluation of {}). LOGOS re-derived each step by kernel evaluation: Decidable.decide reduces \
             to Bool.true in the primary kernel and in the independent re-checker. The proof does not rest \
             on {ELIMINATED_AXIOM}; the statement is unchanged.",
            self.lean,
            decisions.join(", "),
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "declaration": self.declaration,
            "lean": self.lean,
            "eliminated": ELIMINATED_AXIOM,
            "method": METHOD,
            "decisions": self.decisions.iter().map(NativeDecision::to_json).collect::<Vec<_>>(),
        })
    }
}

/// What a target's closure rests on once its leaves are re-derived.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elimination {
    /// The declarations of the closure that hold re-derived leaves, in export order.
    pub rederivations: Vec<Rederivation>,
    /// Whether the re-derived closure still mentions `Lean.ofReduceBool` outside every leaf.
    pub escape_remains: bool,
}

impl Elimination {
    /// Whether the re-derived proof no longer rests on [`ELIMINATED_AXIOM`] although Lean's did.
    pub fn eliminates(&self) -> bool {
        !self.rederivations.is_empty() && !self.escape_remains
    }

    /// Lean's `#print axioms` (Lean names) with the eliminated escape removed: the axioms the
    /// re-derived proof must rest on. Lean's report must name the escape it says was eliminated.
    pub fn lean_after_rederivation(
        &self,
        lean: &[String],
    ) -> std::result::Result<Vec<String>, String> {
        if !self.eliminates() {
            return Ok(lean.to_vec());
        }
        if !lean.iter().any(|a| a == ELIMINATED_AXIOM) {
            return Err(format!(
                "the export's proof rests on {ELIMINATED_AXIOM} through native decisions ({}), but Lean's #print \
                 axioms reports {lean:?}",
                self.leaves().join(", ")
            ));
        }
        Ok(lean
            .iter()
            .filter(|a| *a != ELIMINATED_AXIOM)
            .cloned()
            .collect())
    }

    /// The information changes a package states, one per declaration holding leaves.
    pub fn information_changes(&self) -> Vec<(String, String)> {
        if !self.eliminates() {
            return Vec::new();
        }
        self.rederivations
            .iter()
            .map(|r| (INFORMATION_CATEGORY.to_string(), r.detail()))
            .collect()
    }

    /// Whether `stated` (category, detail pairs) carries exactly this disclosure: every change it
    /// must, and no change of the category it must not.
    pub fn disclosed_by<'a>(
        &self,
        stated: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> std::result::Result<(), String> {
        let stated: Vec<(String, String)> = stated
            .into_iter()
            .filter(|(category, _)| *category == INFORMATION_CATEGORY)
            .map(|(category, detail)| (category.to_string(), detail.to_string()))
            .collect();
        let expected = self.information_changes();
        if stated == expected {
            return Ok(());
        }
        Err(format!(
            "the manifest states {} {INFORMATION_CATEGORY} change(s), but the export's proof re-derives {}",
            stated.len(),
            if expected.is_empty() { "none".to_string() } else { self.leaves().join(", ") }
        ))
    }

    fn leaves(&self) -> Vec<String> {
        self.rederivations
            .iter()
            .flat_map(|r| r.decisions.iter().map(|d| d.decision.clone()))
            .collect()
    }
}

/// The native decisions of one export.
pub struct NativeDecisions {
    tables: Option<Tables>,
    decisions: BTreeMap<usize, NativeDecision>,
    /// Expressions whose subterm, as LOGOS reads it, holds a leaf.
    holds_leaf: HashSet<usize>,
}

impl NativeDecisions {
    /// Recognize every leaf of `export`. An export that never names `Lean.ofReduceBool` costs one
    /// substring scan. Nothing is refused here: an export the tables cannot read has no leaves, and
    /// the importer reports what is wrong with it.
    pub fn read(export: &str) -> NativeDecisions {
        let none = NativeDecisions {
            tables: None,
            decisions: BTreeMap::new(),
            holds_leaf: HashSet::new(),
        };
        if !export.contains("\"ofReduceBool\"") {
            return none;
        }
        let Some(tables) = Tables::read(export) else {
            return none;
        };
        let decisions: BTreeMap<usize, NativeDecision> = if tables.canonical_environment() {
            tables
                .order
                .iter()
                .filter_map(|&e| tables.leaf(e).map(|d| (e, d)))
                .collect()
        } else {
            BTreeMap::new()
        };
        let mut holds_leaf = HashSet::new();
        for &e in &tables.order {
            if decisions.contains_key(&e)
                || tables
                    .children(e, &decisions)
                    .iter()
                    .any(|c| holds_leaf.contains(c))
            {
                holds_leaf.insert(e);
            }
        }
        NativeDecisions {
            tables: Some(tables),
            decisions,
            holds_leaf,
        }
    }

    /// Whether the export has no leaf.
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }

    /// Every leaf, in expression order.
    pub fn decisions(&self) -> impl Iterator<Item = &NativeDecision> {
        self.decisions.values()
    }

    /// The expression record LOGOS reads in place of `record`: a leaf's premise applied to Lean's
    /// evaluation step itself, `of_decide_eq_true p inst (Eq.refl.{1} Bool Bool.true)`. Every other
    /// record is returned as it is.
    pub fn rederive(&self, record: Value) -> Value {
        if self.decisions.is_empty() {
            return record;
        }
        let Some(decision) = record
            .get("ie")
            .and_then(Value::as_u64)
            .and_then(|e| self.decisions.get(&(e as usize)))
        else {
            return record;
        };
        json!({ "ie": decision.leaf, "app": { "fn": decision.premise, "arg": decision.evaluation } })
    }

    /// Every declaration whose type or value holds a leaf, in export order.
    pub fn rederivations(&self) -> Vec<Rederivation> {
        let Some(tables) = &self.tables else {
            return Vec::new();
        };
        tables
            .declared
            .iter()
            .filter_map(|&name| self.rederivation(tables, name))
            .collect()
    }

    /// The closure of `target` (a kernel name) as Lean's `#print axioms` walks it — a definition,
    /// theorem or opaque through its type and value, an inductive through its type and constructors,
    /// a constructor or recursor through its type, an axiom not at all — with every leaf read as
    /// its re-derivation.
    pub fn closure_of(&self, target: &str) -> Elimination {
        let Some(tables) = &self.tables else {
            return Elimination::default();
        };
        let Some(&start) = tables.by_kernel_name.get(target) else {
            return Elimination::default();
        };
        let mut elimination = Elimination::default();
        let mut constants = vec![start];
        let mut constants_seen = HashSet::new();
        let mut expressions_seen = HashSet::new();
        let mut rederived = BTreeSet::new();
        while let Some(name) = constants.pop() {
            if !constants_seen.insert(name) {
                continue;
            }
            let Some(constant) = tables.constants.get(&name) else {
                continue;
            };
            let roots: Vec<usize> = match &constant.kind {
                Kind::Axiom | Kind::Quotient => Vec::new(),
                Kind::Definition | Kind::Theorem | Kind::Opaque => {
                    if self.rederivation(tables, name).is_some() {
                        rederived.insert(name);
                    }
                    std::iter::once(constant.ty).chain(constant.value).collect()
                }
                Kind::Inductive { constructors } => {
                    constants.extend(constructors.iter().copied());
                    vec![constant.ty]
                }
                Kind::Constructor { .. } | Kind::Recursor => vec![constant.ty],
            };
            let mut work = roots;
            while let Some(e) = work.pop() {
                if !expressions_seen.insert(e) {
                    continue;
                }
                if let Some((named, _)) = tables.constant(e) {
                    if tables.name_is(named, OF_REDUCE_BOOL) {
                        elimination.escape_remains = true;
                    }
                    constants.push(named);
                }
                work.extend(tables.children(e, &self.decisions));
            }
        }
        elimination.rederivations = tables
            .declared
            .iter()
            .filter(|name| rederived.contains(name))
            .filter_map(|&name| self.rederivation(tables, name))
            .collect();
        elimination
    }

    /// `verdict`, saying which of the failing declaration's steps were re-derived: a native decision
    /// the kernel refutes or cannot finish is named, never left to look like an unrelated mismatch.
    pub fn explain(&self, verdict: Verdict, declaration: Option<usize>) -> Verdict {
        let Some(tables) = &self.tables else {
            return verdict;
        };
        let Some(rederivation) = declaration.and_then(|name| self.rederivation(tables, name))
        else {
            return verdict;
        };
        if !matches!(
            verdict.reason().code(),
            DiagnosticCode::InvalidProof | DiagnosticCode::ResourceLimit
        ) {
            return verdict;
        }
        let decisions: Vec<&str> = rederivation
            .decisions
            .iter()
            .map(|d| d.decision.as_str())
            .collect();
        verdict.located(format!(
            "{} re-derives its native decision(s) {} by kernel evaluation instead of {ELIMINATED_AXIOM}, and \
             the kernel must reduce each Decidable.decide to Bool.true",
            rederivation.lean,
            decisions.join(", ")
        ))
    }

    fn rederivation(&self, tables: &Tables, name: usize) -> Option<Rederivation> {
        let constant = tables.constants.get(&name)?;
        if !matches!(
            constant.kind,
            Kind::Definition | Kind::Theorem | Kind::Opaque
        ) {
            return None;
        }
        let mut leaves = BTreeSet::new();
        let mut work: Vec<usize> = std::iter::once(constant.ty).chain(constant.value).collect();
        let mut seen = HashSet::new();
        while let Some(e) = work.pop() {
            if !self.holds_leaf.contains(&e) || !seen.insert(e) {
                continue;
            }
            if self.decisions.contains_key(&e) {
                leaves.insert(e);
            }
            work.extend(tables.children(e, &self.decisions));
        }
        if leaves.is_empty() {
            return None;
        }
        Some(Rederivation {
            declaration: tables.kernel_name(name),
            lean: intake::print_lean_name(&tables.names[&name]),
            decisions: leaves
                .into_iter()
                .map(|e| self.decisions[&e].clone())
                .collect(),
        })
    }
}

/// A universe level, as far as the recognizer compares levels.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Level {
    Zero,
    Succ(Box<Level>),
    Max(Box<Level>, Box<Level>),
    IMax(Box<Level>, Box<Level>),
    Param(Vec<Component>),
}

fn one() -> Level {
    Level::Succ(Box::new(Level::Zero))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Definition,
    Theorem,
    Opaque,
    Axiom,
    Quotient,
    Inductive { constructors: Vec<usize> },
    Constructor { inductive: usize },
    Recursor,
}

/// A declared constant: its kind, universe parameter count, type and value.
struct Constant {
    kind: Kind,
    level_params: usize,
    ty: usize,
    value: Option<usize>,
}

/// A structural pattern over export expressions, binder names and annotations aside.
enum Shape {
    Prop,
    Constant(&'static [&'static str], Vec<Level>),
    Bound(u64),
    Apply(Box<Shape>, Box<Shape>),
    Forall(Box<Shape>, Box<Shape>),
}

fn c(name: &'static [&'static str]) -> Shape {
    Shape::Constant(name, Vec::new())
}
fn at(name: &'static [&'static str], levels: Vec<Level>) -> Shape {
    Shape::Constant(name, levels)
}
fn b(index: u64) -> Shape {
    Shape::Bound(index)
}
fn ap(f: Shape, args: impl IntoIterator<Item = Shape>) -> Shape {
    args.into_iter()
        .fold(f, |f, a| Shape::Apply(Box::new(f), Box::new(a)))
}
fn pi(domain: Shape, body: Shape) -> Shape {
    Shape::Forall(Box::new(domain), Box::new(body))
}
fn eq_bool(lhs: Shape, rhs: Shape) -> Shape {
    ap(at(EQ, vec![one()]), [c(BOOL), lhs, rhs])
}

/// The export, read once: names, levels, expressions and declared constants.
struct Tables {
    names: HashMap<usize, Vec<Component>>,
    levels: HashMap<usize, Level>,
    exprs: HashMap<usize, Value>,
    /// Expression indices in export order: every child before its parents.
    order: Vec<usize>,
    constants: HashMap<usize, Constant>,
    /// Declared name indices in export order.
    declared: Vec<usize>,
    by_kernel_name: HashMap<String, usize>,
}

impl Tables {
    fn read(export: &str) -> Option<Tables> {
        let mut tables = Tables {
            names: HashMap::from([(0, Vec::new())]),
            levels: HashMap::from([(0, Level::Zero)]),
            exprs: HashMap::new(),
            order: Vec::new(),
            constants: HashMap::new(),
            declared: Vec::new(),
            by_kernel_name: HashMap::new(),
        };
        for line in export.lines().skip(1) {
            let record: Value = serde_json::from_str(line).ok()?;
            if let Some(i) = record.get("in") {
                let i = index(i)?;
                let (part, pre) = if let Some(x) = record.get("str") {
                    (
                        Component::Str(x["str"].as_str()?.to_string()),
                        index(&x["pre"])?,
                    )
                } else {
                    let x = record.get("num")?;
                    (Component::Num(x["i"].as_u64()?), index(&x["pre"])?)
                };
                let mut name = tables.names.get(&pre)?.clone();
                name.push(part);
                tables.names.insert(i, name);
            } else if let Some(i) = record.get("il") {
                let i = index(i)?;
                let level = if let Some(x) = record.get("succ") {
                    Level::Succ(Box::new(tables.levels.get(&index(x)?)?.clone()))
                } else if let Some(x) = record.get("param") {
                    Level::Param(tables.names.get(&index(x)?)?.clone())
                } else if let Some(pair) = record.get("max").or_else(|| record.get("imax")) {
                    let pair = pair.as_array()?;
                    let lhs = Box::new(tables.levels.get(&index(pair.first()?)?)?.clone());
                    let rhs = Box::new(tables.levels.get(&index(pair.get(1)?)?)?.clone());
                    if record.get("max").is_some() {
                        Level::Max(lhs, rhs)
                    } else {
                        Level::IMax(lhs, rhs)
                    }
                } else {
                    return None;
                };
                tables.levels.insert(i, level);
            } else if let Some(i) = record.get("ie") {
                let i = index(i)?;
                tables.order.push(i);
                tables.exprs.insert(i, record);
            } else {
                tables.declaration(&record)?;
            }
        }
        Some(tables)
    }

    fn declaration(&mut self, record: &Value) -> Option<()> {
        let (tag, body) = record.as_object()?.iter().next()?;
        let simple = |kind: Kind, body: &Value| -> Option<(usize, Constant)> {
            let name = index(&body["name"])?;
            let value = if matches!(kind, Kind::Axiom | Kind::Quotient) {
                None
            } else {
                Some(index(&body["value"])?)
            };
            let level_params = body["levelParams"].as_array()?.len();
            Some((
                name,
                Constant {
                    kind,
                    level_params,
                    ty: index(&body["type"])?,
                    value,
                },
            ))
        };
        let mut declared = Vec::new();
        match tag.as_str() {
            "def" => declared.push(simple(Kind::Definition, body)?),
            "thm" => declared.push(simple(Kind::Theorem, body)?),
            "opaque" => declared.push(simple(Kind::Opaque, body)?),
            "axiom" => declared.push(simple(Kind::Axiom, body)?),
            "quot" => declared.push(simple(Kind::Quotient, body)?),
            "inductive" => {
                for ty in body["types"].as_array()? {
                    let constructors = ty["ctors"]
                        .as_array()?
                        .iter()
                        .map(index)
                        .collect::<Option<Vec<_>>>()?;
                    declared.push(simple_typed(Kind::Inductive { constructors }, ty)?);
                }
                for ctor in body["ctors"].as_array()? {
                    declared.push(simple_typed(
                        Kind::Constructor {
                            inductive: index(&ctor["induct"])?,
                        },
                        ctor,
                    )?);
                }
                for rec in body["recs"].as_array()? {
                    declared.push(simple_typed(Kind::Recursor, rec)?);
                }
            }
            _ => return None,
        }
        for (name, constant) in declared {
            let kernel = self.kernel_name(name);
            self.by_kernel_name.insert(kernel, name);
            self.declared.push(name);
            self.constants.insert(name, constant);
        }
        Some(())
    }

    fn kernel_name(&self, name: usize) -> String {
        super::canonical_name(&intake::encode(&self.names[&name]))
    }

    fn name_is(&self, name: usize, expected: &[&str]) -> bool {
        self.names.get(&name).is_some_and(|components| {
            components.len() == expected.len()
                && components
                    .iter()
                    .zip(expected)
                    .all(|(c, e)| matches!(c, Component::Str(s) if s == e))
        })
    }

    fn named(&self, expected: &[&str]) -> Option<(usize, &Constant)> {
        self.declared
            .iter()
            .filter(|&&name| self.name_is(name, expected))
            .map(|&name| (name, &self.constants[&name]))
            .next()
    }

    fn app(&self, e: usize) -> Option<(usize, usize)> {
        let x = self.exprs.get(&e)?.get("app")?;
        Some((index(&x["fn"])?, index(&x["arg"])?))
    }

    fn constant(&self, e: usize) -> Option<(usize, Vec<Level>)> {
        let x = self.exprs.get(&e)?.get("const")?;
        let levels = x["us"]
            .as_array()?
            .iter()
            .map(|u| self.levels.get(&index(u)?).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some((index(&x["name"])?, levels))
    }

    fn is_constant(&self, e: usize, name: &[&str], levels: &[Level]) -> bool {
        self.constant(e)
            .is_some_and(|(n, us)| self.name_is(n, name) && us == levels)
    }

    /// The children of `e` as LOGOS reads it: a leaf's are its premise and its evaluation step.
    fn children(&self, e: usize, leaves: &BTreeMap<usize, NativeDecision>) -> Vec<usize> {
        if let Some(leaf) = leaves.get(&e) {
            return vec![leaf.premise, leaf.evaluation];
        }
        let Some(record) = self.exprs.get(&e) else {
            return Vec::new();
        };
        let fields: &[(&str, &[&str])] = &[
            ("app", &["fn", "arg"]),
            ("mdata", &["expr"]),
            ("proj", &["struct"]),
            ("letE", &["type", "value", "body"]),
            ("lam", &["type", "body"]),
            ("forallE", &["type", "body"]),
        ];
        for (tag, children) in fields {
            if let Some(x) = record.get(*tag) {
                return children.iter().filter_map(|c| index(&x[*c])).collect();
            }
        }
        Vec::new()
    }

    /// Whether `e` has exactly the structure `shape`.
    fn fits(&self, e: usize, shape: &Shape) -> bool {
        let Some(record) = self.exprs.get(&e) else {
            return false;
        };
        match shape {
            Shape::Prop => record
                .get("sort")
                .and_then(index)
                .is_some_and(|l| self.levels.get(&l) == Some(&Level::Zero)),
            Shape::Constant(name, levels) => self.is_constant(e, name, levels),
            Shape::Bound(i) => record.get("bvar").and_then(Value::as_u64) == Some(*i),
            Shape::Apply(f, a) => self
                .app(e)
                .is_some_and(|(ef, ea)| self.fits(ef, f) && self.fits(ea, a)),
            Shape::Forall(domain, body) => record.get("forallE").is_some_and(|x| {
                index(&x["type"]).is_some_and(|t| self.fits(t, domain))
                    && index(&x["body"]).is_some_and(|t| self.fits(t, body))
            }),
        }
    }

    fn declares(&self, name: &'static [&'static str], kind: fn(&Kind) -> bool, ty: Shape) -> bool {
        self.named(name).is_some_and(|(_, constant)| {
            kind(&constant.kind) && constant.level_params == 0 && self.fits(constant.ty, &ty)
        })
    }

    /// The declarations a leaf's meaning depends on, in their canonical shapes.
    fn canonical_environment(&self) -> bool {
        let of_reduce_bool = pi(
            c(BOOL),
            pi(
                c(BOOL),
                pi(
                    eq_bool(ap(c(REDUCE_BOOL), [b(1)]), b(0)),
                    eq_bool(b(2), b(1)),
                ),
            ),
        );
        let of_decide_eq_true = pi(
            Shape::Prop,
            pi(
                ap(c(DECIDABLE), [b(0)]),
                pi(eq_bool(ap(c(DECIDE), [b(1), b(0)]), c(BOOL_TRUE)), b(2)),
            ),
        );
        let decide = pi(Shape::Prop, pi(ap(c(DECIDABLE), [b(0)]), c(BOOL)));
        self.declares(OF_REDUCE_BOOL, |k| *k == Kind::Axiom, of_reduce_bool)
            && self.declares(REDUCE_BOOL, |k| *k == Kind::Opaque, pi(c(BOOL), c(BOOL)))
            && self.declares(
                OF_DECIDE_EQ_TRUE,
                |k| *k == Kind::Theorem,
                of_decide_eq_true,
            )
            && self.declares(DECIDE, |k| *k == Kind::Definition, decide)
            && self.inductive_with_constructors(BOOL, &[BOOL_FALSE, BOOL_TRUE])
            && self.inductive_with_constructors(EQ, &[EQ_REFL])
    }

    fn inductive_with_constructors(
        &self,
        name: &'static [&'static str],
        constructors: &[&[&str]],
    ) -> bool {
        let Some((inductive, constant)) = self.named(name) else {
            return false;
        };
        let Kind::Inductive {
            constructors: declared,
        } = &constant.kind
        else {
            return false;
        };
        declared.len() == constructors.len()
            && declared.iter().zip(constructors).all(|(&ctor, expected)| {
                self.name_is(ctor, expected)
                    && self
                        .constants
                        .get(&ctor)
                        .is_some_and(|k| k.kind == Kind::Constructor { inductive })
            })
    }

    /// The leaf rooted at `e`, when `e` is one.
    fn leaf(&self, e: usize) -> Option<NativeDecision> {
        let (premise, appeal) = self.app(e)?;
        let (partial, instance) = self.app(premise)?;
        let (head, proposition) = self.app(partial)?;
        if !self.is_constant(head, OF_DECIDE_EQ_TRUE, &[]) {
            return None;
        }
        let (applied, evaluation) = self.app(appeal)?;
        let (appealed, answer) = self.app(applied)?;
        let (escape, decision) = self.app(appealed)?;
        if !self.is_constant(escape, OF_REDUCE_BOOL, &[])
            || !self.is_constant(answer, BOOL_TRUE, &[])
        {
            return None;
        }
        if !self.fits(
            evaluation,
            &ap(at(EQ_REFL, vec![one()]), [c(BOOL), c(BOOL_TRUE)]),
        ) {
            return None;
        }
        let (decision_name, levels) = self.constant(decision)?;
        let constant = self.constants.get(&decision_name)?;
        if !levels.is_empty()
            || constant.kind != Kind::Definition
            || constant.level_params != 0
            || !self.is_constant(constant.ty, BOOL, &[])
        {
            return None;
        }
        let (decided, decided_instance) = self.app(constant.value?)?;
        let (decide, decided_proposition) = self.app(decided)?;
        if !self.is_constant(decide, DECIDE, &[])
            || !self.same(decided_proposition, proposition)
            || !self.same(decided_instance, instance)
        {
            return None;
        }
        Some(NativeDecision {
            leaf: e,
            premise,
            appeal,
            evaluation,
            proposition,
            instance,
            decision: intake::print_lean_name(&self.names[&decision_name]),
        })
    }

    /// Structural equality of two expressions, binder names and binder annotations aside.
    fn same(&self, a: usize, b: usize) -> bool {
        let mut work = vec![(a, b)];
        let mut seen = HashSet::new();
        while let Some((a, b)) = work.pop() {
            if a == b || !seen.insert((a, b)) {
                continue;
            }
            if self.same_node(a, b, &mut work).is_none() {
                return false;
            }
        }
        true
    }

    /// Whether the nodes `a` and `b` agree, their children queued on `work` for comparison.
    fn same_node(&self, a: usize, b: usize, work: &mut Vec<(usize, usize)>) -> Option<()> {
        let (x, y) = (self.exprs.get(&a)?, self.exprs.get(&b)?);
        let mut pair = |field: &str, keys: &[&str]| -> Option<()> {
            let (x, y) = (x.get(field)?, y.get(field)?);
            for key in keys {
                work.push((index(&x[*key])?, index(&y[*key])?));
            }
            Some(())
        };
        if x.get("app").is_some() {
            pair("app", &["fn", "arg"])
        } else if x.get("lam").is_some() {
            pair("lam", &["type", "body"])
        } else if x.get("forallE").is_some() {
            pair("forallE", &["type", "body"])
        } else if let Some(xl) = x.get("letE") {
            (xl.get("nondep") == y.get("letE")?.get("nondep")).then_some(())?;
            pair("letE", &["type", "value", "body"])
        } else if let Some(xm) = x.get("mdata") {
            (xm["data"] == y.get("mdata")?["data"]).then_some(())?;
            pair("mdata", &["expr"])
        } else if let Some(xp) = x.get("proj") {
            let yp = y.get("proj")?;
            let names = (
                self.names.get(&index(&xp["typeName"])?),
                self.names.get(&index(&yp["typeName"])?),
            );
            (xp["idx"] == yp["idx"] && names.0.is_some() && names.0 == names.1).then_some(())?;
            pair("proj", &["struct"])
        } else if x.get("const").is_some() {
            let (xn, xl) = self.constant(a)?;
            let (yn, yl) = self.constant(b)?;
            (self.names.get(&xn).is_some()
                && self.names.get(&xn) == self.names.get(&yn)
                && xl == yl)
                .then_some(())
        } else if let Some(xs) = x.get("sort") {
            let (xs, ys) = (
                self.levels.get(&index(xs)?),
                self.levels.get(&index(y.get("sort")?)?),
            );
            (xs.is_some() && xs == ys).then_some(())
        } else {
            let tag = ["bvar", "natVal", "strVal"]
                .into_iter()
                .find(|k| x.get(*k).is_some())?;
            (y.get(tag).is_some() && x.get(tag) == y.get(tag)).then_some(())
        }
    }
}

fn simple_typed(kind: Kind, body: &Value) -> Option<(usize, Constant)> {
    let name = index(&body["name"])?;
    let level_params = body["levelParams"].as_array()?.len();
    Some((
        name,
        Constant {
            kind,
            level_params,
            ty: index(&body["type"])?,
            value: None,
        },
    ))
}

fn index(v: &Value) -> Option<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok())
}
