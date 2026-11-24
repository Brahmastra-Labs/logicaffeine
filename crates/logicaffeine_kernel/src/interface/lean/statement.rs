//! Statements: the proposition a Lean declaration states, as its own checked definition.
//!
//! A Lean `theorem T : τ := p` states `τ`; a Lean `def T : Prop := v` states `v`. A statement
//! package (the `/2` contract) carries that proposition — never a proof of it — as a fresh,
//! kernel-checked definition `T.logos_statement : Prop`, translated, independently rechecked,
//! emitted as LOGOS and replayable from its bytes alone, exactly as a proof package is. Its proof
//! status is `not_claimed`: translating an open conjecture does not prove it.
//!
//! The artifact is *derived* from the original export by [`derive`], deterministically, so a replay
//! re-derives it from the carried original and must obtain the same bytes:
//!
//! * the **statement expression** is `T`'s type for a theorem ([`Origin::TheoremType`]) and `T`'s
//!   value for a safe definition whose type is exactly `Prop` ([`Origin::DefinitionValue`]); any
//!   other declaration states no proposition and is refused;
//! * the **selected closure** is every declaration that expression reaches — through constants,
//!   projections and the literal interfaces a `natVal` (`Nat`) or `strVal` (the release's `String`
//!   packing) implies — each with its own complete type, value and inductive block, and the four
//!   quotient primitives together (Lean declares them as one). `T`'s proof is
//!   not seeded: it is retained only if the selected closure itself reaches it. A theorem the
//!   statement's definitions depend on keeps its proof, so an unfinished (`sorry`) dependency is
//!   checked and disclosed by the assumption policy, never dropped;
//! * every **answer hole** of the statement expression — formal-conjectures' `answer(sorry)`, an
//!   `mdata` `answer := true` around a closed `sorryAx` — becomes a parameter of the artifact: the
//!   statement is then an **open template** `fun (answer : α) => τ[answer]` of type `α → Prop`. No
//!   answer is ever chosen, and the hole's `sorryAx` is not part of the artifact;
//! * the **derived export** is the original's header, every name and level record, exactly the
//!   original expression and declaration records of the selected closure (each byte for byte), the
//!   records a template needs, and the artifact's own `def` record, last.
//!
//! The relation between the original and the artifact ([`StatementRelation`]) is part of the package
//! manifest, and recomputed — never trusted — at replay.

use super::intake::{self, DeclarationKind};
use super::{declined, rejected, DiagnosticCode, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The schema of a statement package.
pub const PACKAGE_SCHEMA: &str = "logos-lean-statement-package/1";
/// The schema of a statement import's result document.
pub const RESULT_SCHEMA: &str = "logos-lean-result/2";
/// The schema of a statement package's replay receipt.
pub const REPLAY_SCHEMA: &str = "logos-lean-replay/2";
/// Where a statement package carries the original export its artifact is derived from.
pub const ORIGINAL: &str = "source/original.ndjson";
/// The last name component of every artifact: `T.logos_statement`.
pub const ARTIFACT_COMPONENT: &str = "logos_statement";
/// The information-change category disclosing that the artifact was constructed by LOGOS.
pub const INFORMATION_CATEGORY: &str = "STATEMENT_ARTIFACT";
/// The information-change category of an answer hole turned into a template parameter.
pub const ANSWER_CATEGORY: &str = "ANSWER_TEMPLATE";

/// Which expression of the original declaration the artifact's value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A theorem's type.
    TheoremType,
    /// A `Prop`-typed definition's value.
    DefinitionValue,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::TheoremType => "theorem_type",
            Origin::DefinitionValue => "definition_value",
        }
    }
}

/// An answer hole of the statement, abstracted as a parameter of the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerHole {
    /// The original export's id of the `mdata` expression marking the hole.
    pub expression: u64,
    /// The original export's id of the hole's type.
    pub type_expression: u64,
    /// The artifact's binder name for it.
    pub parameter: String,
}

/// The original declaration and the artifact derived from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatementRelation {
    pub origin: Origin,
    /// The original declaration's Lean name.
    pub original: String,
    /// `thm` or `def`.
    pub original_kind: String,
    /// Its universe parameters, in order — the artifact's, exactly.
    pub level_params: Vec<String>,
    /// The artifact's Lean name.
    pub artifact: String,
    /// The answer holes the artifact takes as parameters, outermost first. Empty: a closed
    /// proposition.
    pub answer_holes: Vec<AnswerHole>,
    /// Declarations of the original export the selected closure retains.
    pub retained_declarations: usize,
}

/// A derived statement export and its relation to the original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Derived {
    pub export: String,
    pub relation: StatementRelation,
}

fn unsupported(message: impl Into<String>) -> super::Verdict {
    declined(DiagnosticCode::UnsupportedFeature, message)
}

fn malformed(message: impl Into<String>) -> super::Verdict {
    declined(DiagnosticCode::MalformedExport, message)
}

fn natural(value: &Value) -> Result<u64> {
    value.as_u64().ok_or_else(|| malformed("expected an index"))
}

/// The children of an expression record, in the one order every walk here uses.
fn children(row: &Value) -> Vec<(u64, u32)> {
    let field = |v: &Value, k: &str| v[k].as_u64();
    let mut out = Vec::new();
    if let Some(x) = row.get("app") {
        out.extend(field(x, "fn").map(|i| (i, 0)));
        out.extend(field(x, "arg").map(|i| (i, 0)));
    } else if let Some(x) = row.get("lam").or_else(|| row.get("forallE")) {
        out.extend(field(x, "type").map(|i| (i, 0)));
        out.extend(field(x, "body").map(|i| (i, 1)));
    } else if let Some(x) = row.get("letE") {
        out.extend(field(x, "type").map(|i| (i, 0)));
        out.extend(field(x, "value").map(|i| (i, 0)));
        out.extend(field(x, "body").map(|i| (i, 1)));
    } else if let Some(x) = row.get("mdata") {
        out.extend(field(x, "expr").map(|i| (i, 0)));
    } else if let Some(x) = row.get("proj") {
        out.extend(field(x, "struct").map(|i| (i, 0)));
    }
    out
}

/// Retain the declaration record at `record` — with the whole quotient declaration when it is one
/// of its primitives — and queue its expression roots.
fn retain(rows: &Rows, record: usize, retained: &mut HashSet<usize>, pending: &mut Vec<u64>) {
    let unit: Vec<usize> = if rows.quotient.contains(&record) {
        rows.quotient.clone()
    } else {
        vec![record]
    };
    for at in unit {
        if retained.insert(at) {
            pending.extend(declaration_roots(&rows.declarations[&at]));
        }
    }
}

/// What one export line holds.
#[derive(Clone, Copy)]
enum LineKind {
    Blank,
    Shared,
    Expression(u64),
    Declaration,
}

/// The export, record by record, with the indices a derivation needs.
struct Rows<'a> {
    lines: Vec<&'a str>,
    kinds: Vec<LineKind>,
    /// Expression id → (line index, record).
    exprs: HashMap<u64, (usize, Value)>,
    /// Name index → its Lean components.
    names: HashMap<u64, Vec<intake::Component>>,
    /// Lean components → name index.
    by_components: HashMap<Vec<intake::Component>, u64>,
    /// Name index → the declaration record (line index) declaring it.
    declarer: HashMap<u64, usize>,
    /// Declaration line index → its record.
    declarations: BTreeMap<usize, Value>,
    /// The lines of the quotient primitives (`quot` records): Lean adds `Quot`, `Quot.mk`,
    /// `Quot.lift` and `Quot.ind` as one declaration, so a closure retains them together.
    quotient: Vec<usize>,
    max_expr: u64,
    max_name: u64,
}

impl<'a> Rows<'a> {
    fn read(export: &'a str) -> Result<Rows<'a>> {
        let lines: Vec<&str> = export.lines().collect();
        let mut rows = Rows {
            lines: Vec::new(),
            kinds: vec![LineKind::Shared; lines.len()],
            exprs: HashMap::new(),
            names: HashMap::from([(0, Vec::new())]),
            by_components: HashMap::new(),
            declarer: HashMap::new(),
            declarations: BTreeMap::new(),
            quotient: Vec::new(),
            max_expr: 0,
            max_name: 0,
        };
        for (at, text) in lines.iter().enumerate().skip(1) {
            if text.trim().is_empty() {
                rows.kinds[at] = LineKind::Blank;
                continue;
            }
            let row: Value = serde_json::from_str(text)
                .map_err(|e| malformed(format!("line {}: {e}", at + 1)))?;
            if let Some(index) = row.get("in") {
                let index = natural(index)?;
                rows.max_name = rows.max_name.max(index);
                let (pre, component) = if let Some(s) = row.get("str") {
                    let text = s["str"].as_str().ok_or_else(|| malformed("name string"))?;
                    (
                        natural(&s["pre"])?,
                        intake::Component::Str(text.to_string()),
                    )
                } else if let Some(n) = row.get("num") {
                    (
                        natural(&n["pre"])?,
                        intake::Component::Num(natural(&n["i"])?),
                    )
                } else {
                    return Err(malformed("a name record that is neither str nor num"));
                };
                let mut components = rows
                    .names
                    .get(&pre)
                    .cloned()
                    .ok_or_else(|| malformed("a name's prefix is not an earlier name"))?;
                components.push(component);
                rows.by_components.insert(components.clone(), index);
                rows.names.insert(index, components);
            } else if let Some(index) = row.get("ie") {
                let index = natural(index)?;
                rows.max_expr = rows.max_expr.max(index);
                rows.exprs.insert(index, (at, row));
                rows.kinds[at] = LineKind::Expression(index);
            } else if row.get("il").is_none() {
                for name in declared_names(&row) {
                    rows.declarer.insert(name, at);
                }
                if row.get("quot").is_some() {
                    rows.quotient.push(at);
                }
                rows.declarations.insert(at, row);
                rows.kinds[at] = LineKind::Declaration;
            }
        }
        rows.lines = lines;
        Ok(rows)
    }

    fn expr(&self, id: u64) -> Result<&Value> {
        self.exprs
            .get(&id)
            .map(|(_, row)| row)
            .ok_or_else(|| malformed(format!("expression {id} is not in the export")))
    }

    /// The name index whose Lean components are `components`.
    fn name_index(&self, components: &[intake::Component]) -> Option<u64> {
        self.by_components.get(components).copied()
    }
}

/// The name indices a declaration record declares: every member of an inductive block.
fn declared_names(row: &Value) -> Vec<u64> {
    let Some((kind, body)) = row.as_object().and_then(|o| o.iter().next()) else {
        return Vec::new();
    };
    if kind == "inductive" {
        ["types", "ctors", "recs"]
            .iter()
            .flat_map(|field| body[*field].as_array().cloned().unwrap_or_default())
            .filter_map(|member| member["name"].as_u64())
            .collect()
    } else {
        body["name"].as_u64().into_iter().collect()
    }
}

/// Every expression root a declaration record's checking reads.
fn declaration_roots(row: &Value) -> Vec<u64> {
    let Some((kind, body)) = row.as_object().and_then(|o| o.iter().next()) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if kind == "inductive" {
        for field in ["types", "ctors", "recs"] {
            for member in body[field].as_array().into_iter().flatten() {
                roots.extend(member["type"].as_u64());
                for rule in member["rules"].as_array().into_iter().flatten() {
                    roots.extend(rule["rhs"].as_u64());
                }
            }
        }
    } else {
        roots.extend(body["type"].as_u64());
        roots.extend(body["value"].as_u64());
    }
    roots
}

/// The kernel name of `Nat` and of every constant a string literal of `release` is read through.
fn literal_dependencies(literal: &str, release: &str) -> Vec<Vec<intake::Component>> {
    let path = |parts: &[&str]| -> Vec<intake::Component> {
        parts
            .iter()
            .map(|p| intake::Component::Str(p.to_string()))
            .collect()
    };
    match literal {
        "natVal" => vec![path(&["Nat"])],
        _ => {
            let packing = super::profile::release(release)
                .map(|r| r.string_literals)
                .unwrap_or(super::profile::StringLiterals::Utf8Bytes);
            let packed = match packing {
                super::profile::StringLiterals::ListOfChars => path(&["String", "mk"]),
                super::profile::StringLiterals::Utf8Bytes => path(&["String", "ofList"]),
            };
            vec![
                path(&["Char"]),
                path(&["Char", "ofNat"]),
                path(&["String"]),
                path(&["List"]),
                path(&["List", "nil"]),
                path(&["List", "cons"]),
                path(&["Nat"]),
                packed,
            ]
        }
    }
}

/// The holes a statement expression carries, in first-occurrence order of a pre-order walk, each
/// with the expression of its type; the hole is not descended into.
fn answer_holes(rows: &Rows, root: u64) -> Result<Vec<(u64, u64)>> {
    let mut holes = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let row = rows.expr(id)?;
        if let Some(metadata) = row.get("mdata") {
            if metadata["data"].get("answer").is_some() {
                if metadata["data"]["answer"] != json!("Lean.DataValue.ofBool true")
                    || metadata["data"].as_object().map_or(0, |d| d.len()) != 1
                {
                    return Err(unsupported(format!(
                        "expression {id} marks an answer with {}, which is not `answer := true`",
                        metadata["data"]
                    )));
                }
                let hole_type = hole_type(rows, natural(&metadata["expr"])?)
                    .map_err(|why| unsupported(format!("answer hole at expression {id}: {why}")))?;
                holes.push((id, hole_type));
                continue;
            }
        }
        // Pre-order: push children in reverse so the first child is visited first.
        for (child, _) in children(row).into_iter().rev() {
            stack.push(child);
        }
    }
    Ok(holes)
}

/// The type of an `answer(sorry)`: `sorryAx α b` (α), or Lean's tagged `sorryAx (Name → α) b tag`
/// (α), with α closed.
fn hole_type(rows: &Rows, inner: u64) -> std::result::Result<u64, String> {
    let mut args = Vec::new();
    let mut head = inner;
    loop {
        let row = rows
            .expr(head)
            .map_err(|v| v.reason().message().to_string())?;
        match row.get("app") {
            Some(app) => {
                args.push(app["arg"].as_u64().ok_or("malformed application")?);
                head = app["fn"].as_u64().ok_or("malformed application")?;
            }
            None => break,
        }
    }
    args.reverse();
    let head_row = rows
        .expr(head)
        .map_err(|v| v.reason().message().to_string())?;
    let name = head_row["const"]["name"]
        .as_u64()
        .and_then(|n| rows.names.get(&n))
        .ok_or("the hole is not an application of sorryAx")?;
    if name.as_slice() != [intake::Component::Str("sorryAx".into())] {
        return Err(format!(
            "the hole is an application of {}, not sorryAx",
            intake::print_lean_name(name)
        ));
    }
    let alpha = match args.as_slice() {
        [alpha, _] => *alpha,
        [tagged, _, _] => {
            let pi = rows
                .expr(*tagged)
                .map_err(|v| v.reason().message().to_string())?;
            let body = pi["forallE"]["body"]
                .as_u64()
                .ok_or("a tagged sorry whose type is not a function of its tag")?;
            if loose_range(rows, body, &mut HashMap::new())? > 0 {
                return Err(
                    "the answer's type depends on the sorry's tag or a bound variable".into(),
                );
            }
            body
        }
        _ => return Err(format!("sorryAx applied to {} arguments", args.len())),
    };
    if loose_range(rows, alpha, &mut HashMap::new())? > 0 {
        return Err("the answer's type depends on a bound variable".into());
    }
    if loose_range(rows, inner, &mut HashMap::new())? > 0 {
        return Err("the hole depends on a bound variable".into());
    }
    Ok(alpha)
}

/// How many enclosing binders an expression's loose bound variables need (0: closed).
fn loose_range(
    rows: &Rows,
    id: u64,
    memo: &mut HashMap<u64, u64>,
) -> std::result::Result<u64, String> {
    if let Some(&range) = memo.get(&id) {
        return Ok(range);
    }
    let mut order = vec![(id, false)];
    while let Some((at, done)) = order.pop() {
        if memo.contains_key(&at) {
            continue;
        }
        let row = rows
            .expr(at)
            .map_err(|v| v.reason().message().to_string())?;
        let kids = children(row);
        if !done {
            order.push((at, true));
            for (child, _) in &kids {
                if !memo.contains_key(child) {
                    order.push((*child, false));
                }
            }
            continue;
        }
        let mut range = row.get("bvar").and_then(Value::as_u64).map_or(0, |b| b + 1);
        for (child, binders) in kids {
            range = range.max(memo[&child].saturating_sub(binders as u64));
        }
        memo.insert(at, range);
    }
    Ok(memo[&id])
}

/// Derive the statement export of `target` (a dotted Lean name) from `original`.
pub fn derive(original: &str, target: &str) -> Result<Derived> {
    let read = intake::read_admitted(original)?;
    let kernel = intake::kernel_name(target)?;
    let declaration = read.declaration(&kernel).ok_or_else(|| {
        rejected(
            DiagnosticCode::MissingDeclaration,
            format!("the export does not declare {target}"),
        )
        .in_declaration(target)
    })?;
    let rows = Rows::read(original)?;
    let record = rows
        .declarations
        .get(&(declaration.line - 1))
        .ok_or_else(|| malformed(format!("{target}'s record is not at its line")))?;
    let (origin, body) = match declaration.kind {
        DeclarationKind::Theorem => (Origin::TheoremType, &record["thm"]),
        DeclarationKind::Definition => (Origin::DefinitionValue, &record["def"]),
        other => {
            return Err(unsupported(format!(
            "{target} is a Lean {}: a statement is a theorem's type or a Prop definition's value",
            other.as_str()
        ))
            .in_declaration(target))
        }
    };
    if origin == Origin::DefinitionValue {
        if body["safety"] != json!("safe") {
            return Err(unsupported(format!(
                "{target} is a {} definition: only a safe definition states a proposition",
                body["safety"].as_str().unwrap_or("non-safe")
            ))
            .in_declaration(target));
        }
        let ty = rows.expr(natural(&body["type"])?)?;
        if ty.get("sort") != Some(&json!(0)) {
            return Err(unsupported(format!(
                "{target}'s type is not Prop: a definition states a proposition only when its value is one"
            ))
            .in_declaration(target));
        }
    }
    let root = natural(match origin {
        Origin::TheoremType => &body["type"],
        Origin::DefinitionValue => &body["value"],
    })?;
    let original_name = natural(&body["name"])?;
    let level_params: Vec<u64> = body["levelParams"]
        .as_array()
        .ok_or_else(|| malformed("levelParams"))?
        .iter()
        .map(natural)
        .collect::<Result<_>>()?;
    let level_names: Vec<String> = level_params
        .iter()
        .map(|l| {
            rows.names
                .get(l)
                .map(|c| intake::print_lean_name(c))
                .ok_or_else(|| malformed("a universe parameter's name is not a name"))
        })
        .collect::<Result<_>>()?;

    // The artifact's name: `T.logos_statement`, which the export must not already declare.
    let mut artifact_components = rows
        .names
        .get(&original_name)
        .cloned()
        .ok_or_else(|| malformed("the target's name is not a name"))?;
    artifact_components.push(intake::Component::Str(ARTIFACT_COMPONENT.into()));
    let artifact = intake::print_lean_name(&artifact_components);
    if let Some(existing) = rows.name_index(&artifact_components) {
        if rows.declarer.contains_key(&existing) {
            return Err(unsupported(format!(
                "the export already declares {artifact}, the name the statement of {target} is given"
            ))
            .in_declaration(target));
        }
    }

    let holes = answer_holes(&rows, root)?;

    // Every record the derived export appends, with fresh indices past the original's.
    let mut appended: Vec<String> = Vec::new();
    let mut next_name = rows.max_name + 1;
    let mut next_expr = rows.max_expr + 1;
    let mut name_row = |pre: u64, text: &str, appended: &mut Vec<String>| {
        let index = next_name;
        next_name += 1;
        appended.push(json!({"str": {"pre": pre, "str": text}, "in": index}).to_string());
        index
    };
    let artifact_name = name_row(original_name, ARTIFACT_COMPONENT, &mut appended);
    let parameters: Vec<String> = match holes.len() {
        0 => Vec::new(),
        1 => vec!["answer".to_string()],
        n => (1..=n).map(|i| format!("answer_{i}")).collect(),
    };
    let parameter_names: Vec<u64> = parameters
        .iter()
        .map(|p| name_row(0, p, &mut appended))
        .collect();
    let mut expr_row = |body: Value, appended: &mut Vec<String>| {
        let id = next_expr;
        next_expr += 1;
        let mut row = serde_json::Map::new();
        row.insert("ie".into(), json!(id));
        for (k, v) in body.as_object().expect("an expression body").clone() {
            row.insert(k, v);
        }
        appended.push(Value::Object(row).to_string());
        id
    };

    // The value: the statement expression, each hole replaced by its parameter.
    let hole_index: HashMap<u64, usize> = holes
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let value = if holes.is_empty() {
        root
    } else {
        let mut holds_hole: HashMap<u64, bool> = HashMap::new();
        let mut memo: HashMap<(u64, u64), u64> = HashMap::new();
        let n = holes.len() as u64;
        // Post-order over (expression, depth), children before parents.
        let mut work = vec![(root, 0u64, false)];
        while let Some((id, depth, done)) = work.pop() {
            if memo.contains_key(&(id, depth)) {
                continue;
            }
            if let Some(&i) = hole_index.get(&id) {
                let bvar = expr_row(json!({"bvar": depth + (n - 1 - i as u64)}), &mut appended);
                memo.insert((id, depth), bvar);
                holds_hole.insert(id, true);
                continue;
            }
            let row = rows.expr(id)?;
            let kids = children(row);
            if !done {
                work.push((id, depth, true));
                for (child, binders) in kids.iter().rev() {
                    work.push((*child, depth + *binders as u64, false));
                }
                continue;
            }
            let rewritten: Vec<u64> = kids
                .iter()
                .map(|(child, binders)| memo[&(*child, depth + *binders as u64)])
                .collect();
            if kids.iter().zip(&rewritten).all(|((c, _), r)| c == r) {
                memo.insert((id, depth), id);
                continue;
            }
            let mut body = row.clone();
            let object = body.as_object_mut().expect("an expression record");
            object.remove("ie");
            let (kind, fields) = object.iter_mut().next().expect("one expression kind");
            let keys: &[&str] = match kind.as_str() {
                "app" => &["fn", "arg"],
                "lam" | "forallE" => &["type", "body"],
                "letE" => &["type", "value", "body"],
                "mdata" => &["expr"],
                "proj" => &["struct"],
                _ => &[],
            };
            for (key, new) in keys.iter().zip(&rewritten) {
                fields[*key] = json!(new);
            }
            let new = expr_row(body, &mut appended);
            memo.insert((id, depth), new);
        }
        let mut value = memo[&(root, 0)];
        for (i, (_, hole_ty)) in holes.iter().enumerate().rev() {
            value = expr_row(
                json!({"lam": {"name": parameter_names[i], "type": hole_ty, "body": value, "binderInfo": "default"}}),
                &mut appended,
            );
        }
        value
    };
    let prop = rows
        .exprs
        .iter()
        .filter(|(_, (_, row))| {
            row.get("sort") == Some(&json!(0)) && row.as_object().map_or(0, |o| o.len()) == 2
        })
        .map(|(id, _)| *id)
        .min();
    let prop = match prop {
        Some(prop) => prop,
        None => expr_row(json!({"sort": 0}), &mut appended),
    };
    let mut ty = prop;
    for (i, (_, hole_ty)) in holes.iter().enumerate().rev() {
        ty = expr_row(
            json!({"forallE": {"name": parameter_names[i], "type": hole_ty, "body": ty, "binderInfo": "default"}}),
            &mut appended,
        );
    }
    appended.push(
        json!({"def": {
            "name": artifact_name,
            "levelParams": level_params,
            "type": ty,
            "value": value,
            "hints": "abbrev",
            "safety": "safe",
            "all": [artifact_name],
        }})
        .to_string(),
    );

    // The selected closure: every declaration the artifact's type and value reach, each with its
    // complete record, and the literal interfaces its literals imply. The artifact reaches the
    // original records its own rows name: the statement expression itself, or — for a template —
    // the subterms left unchanged around the holes, and each hole's type.
    let mut pending: Vec<u64> = Vec::new();
    for text in &appended {
        let row: Value = serde_json::from_str(text).expect("an appended record is JSON");
        if row.get("ie").is_some() {
            pending.extend(
                children(&row)
                    .into_iter()
                    .map(|(c, _)| c)
                    .filter(|c| *c <= rows.max_expr),
            );
        }
    }
    if holes.is_empty() {
        pending.push(root);
    }
    if prop <= rows.max_expr {
        pending.push(prop);
    }
    let release = read.header().lean_version.clone();
    let mut retained_records: HashSet<usize> = HashSet::new();
    let mut retained_exprs: HashSet<u64> = HashSet::new();
    loop {
        let mut implied: std::collections::BTreeSet<Vec<intake::Component>> =
            std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            let Some((_, row)) = rows.exprs.get(&id) else {
                return Err(malformed(format!("expression {id} is not in the export")));
            };
            if !retained_exprs.insert(id) {
                continue;
            }
            let referenced = row
                .get("const")
                .and_then(|c| c["name"].as_u64())
                .or_else(|| row.get("proj").and_then(|p| p["typeName"].as_u64()));
            if let Some(name) = referenced {
                if let Some(&record) = rows.declarer.get(&name) {
                    retain(&rows, record, &mut retained_records, &mut pending);
                }
            }
            for literal in ["natVal", "strVal"] {
                if row.get(literal).is_some() {
                    implied.extend(literal_dependencies(literal, &release));
                }
            }
            pending.extend(children(row).into_iter().map(|(c, _)| c));
        }
        for components in implied {
            let Some(name) = rows.name_index(&components) else {
                return Err(rejected(
                    DiagnosticCode::MissingDeclaration,
                    format!(
                        "the statement's literals require {}, which the export does not declare",
                        intake::print_lean_name(&components)
                    ),
                ));
            };
            if let Some(&record) = rows.declarer.get(&name) {
                retain(&rows, record, &mut retained_records, &mut pending);
            }
        }
        if pending.is_empty() {
            break;
        }
    }

    let mut export = String::with_capacity(original.len());
    export.push_str(rows.lines[0]);
    export.push('\n');
    for (at, text) in rows.lines.iter().enumerate().skip(1) {
        let keep = match rows.kinds[at] {
            LineKind::Blank => false,
            LineKind::Shared => true,
            LineKind::Expression(id) => retained_exprs.contains(&id),
            LineKind::Declaration => retained_records.contains(&at),
        };
        if keep {
            export.push_str(text);
            export.push('\n');
        }
    }
    for row in &appended {
        export.push_str(row);
        export.push('\n');
    }
    Ok(Derived {
        export,
        relation: StatementRelation {
            origin,
            original: target.to_string(),
            original_kind: match origin {
                Origin::TheoremType => "thm".into(),
                Origin::DefinitionValue => "def".into(),
            },
            level_params: level_names,
            artifact,
            answer_holes: holes
                .iter()
                .zip(&parameters)
                .map(|((expression, ty), parameter)| AnswerHole {
                    expression: *expression,
                    type_expression: *ty,
                    parameter: parameter.clone(),
                })
                .collect(),
            retained_declarations: retained_records.len(),
        },
    })
}

/// The information changes a statement package discloses: the artifact's construction, and each
/// answer hole turned into a parameter.
pub fn information_changes(relation: &StatementRelation) -> Vec<(String, String)> {
    let mut changes = vec![(
        INFORMATION_CATEGORY.to_string(),
        format!(
            "{} is LOGOS's definition of the proposition {} states ({}), derived from the original export by \
             selecting that expression's closure; {}'s proof is not part of the package and no proof of the \
             proposition is claimed",
            relation.artifact,
            relation.original,
            relation.origin.as_str(),
            relation.original
        ),
    )];
    for hole in &relation.answer_holes {
        changes.push((
            ANSWER_CATEGORY.to_string(),
            format!(
                "expression {}: the answer hole answer(sorry) is the template parameter {} (its type: expression {}); \
                 no answer is chosen",
                hole.expression, hole.parameter, hole.type_expression
            ),
        ));
    }
    changes
}

/// A statement's identity independent of how an export numbers it: a SHA-256 over the artifact's
/// type and value and every declaration its derived closure holds, each read structurally — names by
/// their Lean spelling, universe parameters by position, binder names, binder annotations and
/// metadata ignored, a theorem by its statement alone (proof irrelevance), an opaque declaration by
/// its type. Two exports state the same proposition of `target`, over the same definitions, exactly
/// when their fingerprints agree: a proof of `target` from one export is a proof of the statement
/// of the other.
pub fn fingerprint(export: &str, target: &str) -> Result<String> {
    let derived = derive(export, target)?;
    let rows = Rows::read(&derived.export)?;
    let mut levels: HashMap<u64, Value> = HashMap::new();
    for text in rows.lines.iter().skip(1) {
        if text.contains("\"il\"") {
            if let Ok(row) = serde_json::from_str::<Value>(text) {
                if let Some(i) = row.get("il").and_then(Value::as_u64) {
                    levels.insert(i, row);
                }
            }
        }
    }
    let hasher = Fingerprinter {
        rows: &rows,
        levels: &levels,
    };
    let mut declarations: Vec<(String, String)> = Vec::new();
    for row in rows.declarations.values() {
        let (kind, body) = row
            .as_object()
            .and_then(|o| o.iter().next())
            .ok_or_else(|| malformed("a declaration record"))?;
        let names: Vec<u64> = if kind == "inductive" {
            body["types"][0]["levelParams"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect()
        } else {
            body["levelParams"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect()
        };
        let scope = |e: &Value| hasher.expr(natural(e)?, &names, &mut HashMap::new());
        let name = |v: &Value| -> Result<String> {
            Ok(intake::print_lean_name(
                rows.names
                    .get(&natural(v)?)
                    .ok_or_else(|| malformed("a declaration's name"))?,
            ))
        };
        let (key, digest) = match kind.as_str() {
            "axiom" | "quot" => (
                name(&body["name"])?,
                format!(
                    "{kind}:{}:{}:{}",
                    names.len(),
                    scope(&body["type"])?,
                    body["kind"]
                ),
            ),
            "thm" | "opaque" => (
                name(&body["name"])?,
                format!("{kind}:{}:{}", names.len(), scope(&body["type"])?),
            ),
            "def" => (
                name(&body["name"])?,
                format!(
                    "def:{}:{}:{}:{}",
                    names.len(),
                    scope(&body["type"])?,
                    scope(&body["value"])?,
                    body["safety"]
                ),
            ),
            "inductive" => {
                let mut parts = vec![format!("inductive:{}", names.len())];
                for t in body["types"].as_array().into_iter().flatten() {
                    parts.push(format!(
                        "type:{}:{}:{}:{}",
                        name(&t["name"])?,
                        t["numParams"],
                        t["numIndices"],
                        scope(&t["type"])?
                    ));
                }
                for c in body["ctors"].as_array().into_iter().flatten() {
                    parts.push(format!("ctor:{}:{}", name(&c["name"])?, scope(&c["type"])?));
                }
                (name(&body["types"][0]["name"])?, parts.join("|"))
            }
            other => return Err(malformed(format!("a {other} record"))),
        };
        declarations.push((key, super::package::sha256_hex(digest.as_bytes())));
    }
    declarations.sort();
    let mut preimage = format!(
        "logos-lean-statement-fingerprint/1\0{}\0",
        derived.relation.level_params.len()
    );
    for (name, digest) in &declarations {
        preimage.push_str(name);
        preimage.push('\0');
        preimage.push_str(digest);
        preimage.push('\n');
    }
    Ok(format!(
        "sha256:{}",
        super::package::sha256_hex(preimage.as_bytes())
    ))
}

struct Fingerprinter<'r, 'a> {
    rows: &'r Rows<'a>,
    levels: &'r HashMap<u64, Value>,
}

impl Fingerprinter<'_, '_> {
    fn level(&self, id: u64, params: &[u64]) -> Result<String> {
        if id == 0 {
            return Ok("0".into());
        }
        let row = self
            .levels
            .get(&id)
            .ok_or_else(|| malformed(format!("level {id}")))?;
        Ok(if let Some(l) = row.get("succ") {
            format!("s({})", self.level(natural(l)?, params)?)
        } else if let Some(pair) = row.get("max") {
            format!(
                "m({},{})",
                self.level(natural(&pair[0])?, params)?,
                self.level(natural(&pair[1])?, params)?
            )
        } else if let Some(pair) = row.get("imax") {
            format!(
                "i({},{})",
                self.level(natural(&pair[0])?, params)?,
                self.level(natural(&pair[1])?, params)?
            )
        } else if let Some(p) = row.get("param") {
            let p = natural(p)?;
            match params.iter().position(|q| *q == p) {
                Some(i) => format!("p{i}"),
                None => return Err(malformed("an undeclared universe parameter")),
            }
        } else {
            return Err(malformed("a level record"));
        })
    }

    fn expr(&self, root: u64, params: &[u64], memo: &mut HashMap<u64, String>) -> Result<String> {
        let mut work = vec![(root, false)];
        while let Some((id, done)) = work.pop() {
            if memo.contains_key(&id) {
                continue;
            }
            let row = self.rows.expr(id)?;
            let kids = children(row);
            if !done {
                work.push((id, true));
                for (child, _) in kids.iter().rev() {
                    work.push((*child, false));
                }
                continue;
            }
            let child = |i: usize| memo[&kids[i].0].clone();
            let text = if let Some(b) = row.get("bvar") {
                format!("b{b}")
            } else if let Some(s) = row.get("sort") {
                format!("S({})", self.level(natural(s)?, params)?)
            } else if let Some(c) = row.get("const") {
                let us = c["us"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|u| self.level(natural(u)?, params))
                    .collect::<Result<Vec<_>>>()?;
                let name = self
                    .rows
                    .names
                    .get(&natural(&c["name"])?)
                    .ok_or_else(|| malformed("a constant's name"))?;
                format!("C({};{})", intake::print_lean_name(name), us.join(","))
            } else if row.get("app").is_some() {
                format!("A({},{})", child(0), child(1))
            } else if row.get("lam").is_some() {
                format!("L({},{})", child(0), child(1))
            } else if row.get("forallE").is_some() {
                format!("P({},{})", child(0), child(1))
            } else if row.get("letE").is_some() {
                format!("D({},{},{})", child(0), child(1), child(2))
            } else if row.get("mdata").is_some() {
                child(0)
            } else if let Some(p) = row.get("proj") {
                let name = self
                    .rows
                    .names
                    .get(&natural(&p["typeName"])?)
                    .ok_or_else(|| malformed("a projection's structure"))?;
                format!(
                    "J({},{},{})",
                    intake::print_lean_name(name),
                    p["idx"],
                    child(0)
                )
            } else if let Some(n) = row.get("natVal") {
                format!("N({})", n.as_str().unwrap_or_default())
            } else if let Some(s) = row.get("strVal") {
                format!(
                    "T({})",
                    serde_json::to_string(s).expect("a string serializes")
                )
            } else {
                return Err(malformed(format!("expression {id}")));
            };
            // Keep nodes small: a subterm is named by the digest of its text.
            memo.insert(
                id,
                super::package::sha256_hex(text.as_bytes())[..32].to_string(),
            );
        }
        Ok(memo[&root].clone())
    }
}
