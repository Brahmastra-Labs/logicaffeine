//! Intake: the strict front door of a Lean → LOGOS result.
//!
//! The reader (`super`) is deliberately tolerant — it is also the Lean Kernel Arena benchmark
//! checker, and reads any `3.1.0` export. Intake is the door a *result* goes through, and it holds an
//! export to an explicit compatibility rule before any declaration is translated:
//!
//! * the header names the pinned profile ([`STAGE_A`]: Lean 4.14.0, lean4export 3.1.0, format
//!   3.1.0) — an unknown format version is [`UnsupportedExportVersion`](DiagnosticCode::UnsupportedExportVersion),
//!   any other producer is [`UnsupportedToolchain`](DiagnosticCode::UnsupportedToolchain);
//! * every record has exactly the fields format 3.1.0 defines for its kind — an unknown field is
//!   [`UnknownField`](DiagnosticCode::UnknownField), a missing or mistyped one
//!   [`MalformedExport`](DiagnosticCode::MalformedExport), an unknown kind
//!   [`UnknownRecordKind`](DiagnosticCode::UnknownRecordKind), a metavariable or free variable
//!   [`UnresolvedMetavariable`](DiagnosticCode::UnresolvedMetavariable);
//! * every referenced constant is declared ([`MissingDeclaration`](DiagnosticCode::MissingDeclaration))
//!   and none is declared twice ([`DuplicateDeclaration`](DiagnosticCode::DuplicateDeclaration)).
//!
//! It also answers the question retention, emission and packaging share: which declarations does a
//! target depend on ([`Intake::closure`]), in dependency order.

use super::{canonical_name, declined, rejected, DiagnosticCode, Result};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// The toolchain and assumption profile a result is produced under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub lean_version: &'static str,
    pub format_version: &'static str,
    pub exporter_name: &'static str,
    pub exporter_version: &'static str,
    /// The assumptions a verified result may depend on, by Lean name.
    pub allowed_axioms: &'static [&'static str],
}

/// Stage A: Lean 4.14.0, lean4export 3.1.0 (revision [`EXPORTER_REVISION`]), export format 3.1.0,
/// Lean core only, and Lean's three standard axioms.
pub const STAGE_A: Profile = Profile {
    lean_version: "4.14.0",
    format_version: "3.1.0",
    exporter_name: "lean4export",
    exporter_version: "3.1.0",
    allowed_axioms: &["propext", "Quot.sound", "Classical.choice"],
};

/// The elan toolchain name of the pinned Lean.
pub const TOOLCHAIN: &str = "leanprover/lean4:v4.14.0";
/// The pinned lean4export revision.
pub const EXPORTER_REVISION: &str = "20c4a64aa1b76f7c79a6db15d089084359f8c3dc";
/// SHA-256 of the pinned exporter's sources at [`EXPORTER_REVISION`]. The exporter is identified by
/// its content, never by the name of the directory holding it.
pub const EXPORTER_SOURCES: [(&str, &str); 2] = [
    (
        "Main.lean",
        "bd8e350903f1382b25fe767f9086dd9882000f01085f7046ea06545e7848842a",
    ),
    (
        "Export.lean",
        "16fa1fec370500f4ca03f042fc3e65cac27839b640b32c5a921e085206c7031b",
    ),
];
/// The exporter flags every result is produced with: metadata and unsafe declarations are exported
/// so that the reader, not the exporter, decides what they mean.
pub const EXPORTER_FLAGS: [&str; 2] = ["--export-mdata", "--export-unsafe"];

/// The export's first record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub lean_version: String,
    pub lean_githash: String,
    pub exporter_name: String,
    pub exporter_version: String,
    pub format_version: String,
}

/// What a declaration record declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationKind {
    Axiom,
    Definition,
    Theorem,
    Opaque,
    Quotient,
    Inductive,
    Constructor,
    Recursor,
}

impl DeclarationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeclarationKind::Axiom => "axiom",
            DeclarationKind::Definition => "def",
            DeclarationKind::Theorem => "thm",
            DeclarationKind::Opaque => "opaque",
            DeclarationKind::Quotient => "quot",
            DeclarationKind::Inductive => "inductive",
            DeclarationKind::Constructor => "ctor",
            DeclarationKind::Recursor => "rec",
        }
    }
}

/// One declared constant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The kernel identity the reader registers it under.
    pub kernel_name: String,
    /// The dotted Lean name.
    pub lean_name: String,
    pub kind: DeclarationKind,
    /// The 1-based export line of its record.
    pub line: usize,
    /// Its record: the members of one inductive block share one.
    record: usize,
}

/// One declaration record's dependency roots.
struct Record {
    roots: Vec<usize>,
    members: Vec<usize>,
}

/// An export that passed intake.
pub struct Intake {
    header: Header,
    declarations: Vec<Declaration>,
    records: Vec<Record>,
    by_kernel_name: HashMap<String, usize>,
    /// Expression id → (children, referenced constant's name index).
    expressions: HashMap<usize, (Vec<usize>, Option<usize>)>,
    /// Name index → declaration index.
    declared: HashMap<usize, usize>,
}

impl Intake {
    pub fn header(&self) -> &Header {
        &self.header
    }
    /// Every declared constant, in export order — which lean4export makes dependency order.
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
    /// The declaration registered under `kernel_name`.
    pub fn declaration(&self, kernel_name: &str) -> Option<&Declaration> {
        self.by_kernel_name
            .get(kernel_name)
            .map(|&i| &self.declarations[i])
    }
    /// The dependency closure of `kernel_name`: the declaration itself, every constant its type and
    /// value mention, transitively, and every member of each inductive block reached — in export
    /// order, so the target comes after everything it depends on.
    pub fn closure(&self, kernel_name: &str) -> Result<Vec<String>> {
        let &start = self.by_kernel_name.get(kernel_name).ok_or_else(|| {
            rejected(
                DiagnosticCode::MissingDeclaration,
                format!("the export does not declare {}", lean_name(kernel_name)),
            )
            .in_declaration(lean_name(kernel_name))
        })?;
        let mut records_seen = HashSet::new();
        let mut expressions_seen = HashSet::new();
        let mut in_closure = vec![false; self.declarations.len()];
        let mut pending_records = vec![self.declarations[start].record];
        let mut pending_expressions = Vec::new();
        while let Some(record) = pending_records.pop() {
            if !records_seen.insert(record) {
                continue;
            }
            for &member in &self.records[record].members {
                in_closure[member] = true;
            }
            pending_expressions.extend(self.records[record].roots.iter().copied());
            while let Some(expression) = pending_expressions.pop() {
                if !expressions_seen.insert(expression) {
                    continue;
                }
                let Some((children, constant)) = self.expressions.get(&expression) else {
                    continue;
                };
                pending_expressions.extend(children.iter().copied());
                if let Some(declaration) = constant.and_then(|name| self.declared.get(&name)) {
                    pending_records.push(self.declarations[*declaration].record);
                }
            }
        }
        Ok(self
            .declarations
            .iter()
            .zip(in_closure)
            .filter(|(_, member)| *member)
            .map(|(d, _)| d.kernel_name.clone())
            .collect())
    }
}

/// Read and validate an export under `profile`.
pub fn read(export: &str, profile: &Profile) -> Result<Intake> {
    let mut lines = export.lines();
    let first = lines.next().ok_or_else(|| {
        declined(DiagnosticCode::MalformedExport, "the export is empty").at_line(1)
    })?;
    let header = read_header(first, profile).map_err(|v| v.at_line(1))?;
    let mut reader = Reader::default();
    reader.names.insert(0, String::new());
    reader.levels.insert(0);
    for (offset, text) in lines.enumerate() {
        let line = offset + 2;
        if text.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(text).map_err(|e| {
            declined(DiagnosticCode::MalformedExport, format!("line {line}: {e}")).at_line(line)
        })?;
        reader.record(&record, line).map_err(|v| v.at_line(line))?;
    }
    reader.finish(header)
}

/// The header, held to the profile: the format version first (a newer format may legitimately add
/// fields), then the header's own fields, then the producing toolchain.
pub fn read_header(first: &str, profile: &Profile) -> Result<Header> {
    let value: Value = serde_json::from_str(first).map_err(|e| {
        declined(
            DiagnosticCode::MalformedExport,
            format!("the export header is not JSON: {e}"),
        )
    })?;
    if let Some(format) = value
        .pointer("/meta/format/version")
        .and_then(Value::as_str)
    {
        if format != profile.format_version {
            return Err(declined(
                DiagnosticCode::UnsupportedExportVersion,
                format!(
                    "export format {format} is not the supported format {}",
                    profile.format_version
                ),
            ));
        }
    }
    let root = object(&value, "the export header")?;
    fields(root, "the export header", &["meta"], &[])?;
    let meta = object(&root["meta"], "meta")?;
    fields(meta, "meta", &["exporter", "lean", "format"], &[])?;
    let exporter = object(&meta["exporter"], "meta.exporter")?;
    fields(exporter, "meta.exporter", &["name", "version"], &[])?;
    let lean = object(&meta["lean"], "meta.lean")?;
    fields(lean, "meta.lean", &["githash", "version"], &[])?;
    let format = object(&meta["format"], "meta.format")?;
    fields(format, "meta.format", &["version"], &[])?;
    let header = Header {
        lean_version: string(&lean["version"], "meta.lean.version")?,
        lean_githash: string(&lean["githash"], "meta.lean.githash")?,
        exporter_name: string(&exporter["name"], "meta.exporter.name")?,
        exporter_version: string(&exporter["version"], "meta.exporter.version")?,
        format_version: string(&format["version"], "meta.format.version")?,
    };
    if header.format_version != profile.format_version {
        return Err(declined(
            DiagnosticCode::UnsupportedExportVersion,
            format!(
                "export format {} is not the supported format {}",
                header.format_version, profile.format_version
            ),
        ));
    }
    if header.lean_version != profile.lean_version
        || header.exporter_name != profile.exporter_name
        || header.exporter_version != profile.exporter_version
    {
        return Err(declined(
            DiagnosticCode::UnsupportedToolchain,
            format!(
                "the export was produced by Lean {} with {} {}; the supported profile is Lean {} with {} {}",
                header.lean_version,
                header.exporter_name,
                header.exporter_version,
                profile.lean_version,
                profile.exporter_name,
                profile.exporter_version
            ),
        ));
    }
    Ok(header)
}

fn object<'v>(value: &'v Value, what: &str) -> Result<&'v Map<String, Value>> {
    value.as_object().ok_or_else(|| {
        declined(
            DiagnosticCode::MalformedExport,
            format!("{what} is not a JSON object"),
        )
    })
}

fn string(value: &Value, what: &str) -> Result<String> {
    value.as_str().map(str::to_string).ok_or_else(|| {
        declined(
            DiagnosticCode::MalformedExport,
            format!("{what} is not a string"),
        )
    })
}

/// `object` has every field in `required`, possibly some in `optional`, and nothing else.
fn fields(
    object: &Map<String, Value>,
    what: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<()> {
    if let Some(unknown) = object
        .keys()
        .find(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return Err(declined(
            DiagnosticCode::UnknownField,
            format!("{what} has the field {unknown:?}, which export format 3.1.0 does not define"),
        ));
    }
    if let Some(missing) = required.iter().find(|k| !object.contains_key(**k)) {
        return Err(declined(
            DiagnosticCode::MalformedExport,
            format!("{what} lacks its mandatory field {missing:?}"),
        ));
    }
    Ok(())
}

fn natural(value: &Value, what: &str) -> Result<usize> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                format!("{what} is not a natural number"),
            )
        })
}

fn boolean(value: &Value, what: &str) -> Result<()> {
    value.as_bool().map(|_| ()).ok_or_else(|| {
        declined(
            DiagnosticCode::MalformedExport,
            format!("{what} is not a boolean"),
        )
    })
}

fn naturals(value: &Value, what: &str) -> Result<Vec<usize>> {
    value
        .as_array()
        .ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                format!("{what} is not an array"),
            )
        })?
        .iter()
        .map(|v| natural(v, what))
        .collect()
}

#[derive(Default)]
struct Reader {
    names: HashMap<usize, String>,
    levels: HashSet<usize>,
    expressions: HashMap<usize, (Vec<usize>, Option<usize>)>,
    /// The first line at which each constant name index is referenced by an expression.
    referenced: HashMap<usize, usize>,
    declarations: Vec<Declaration>,
    records: Vec<Record>,
    by_kernel_name: HashMap<String, usize>,
    declared: HashMap<usize, usize>,
}

impl Reader {
    fn record(&mut self, record: &Value, line: usize) -> Result<()> {
        let map = record.as_object().ok_or_else(|| {
            declined(
                DiagnosticCode::MalformedExport,
                "the record is not a JSON object",
            )
        })?;
        if map.contains_key("in") {
            return self.name(map);
        }
        if map.contains_key("il") {
            return self.level(map);
        }
        if map.contains_key("ie") {
            return self.expression(map, line);
        }
        if map.len() != 1 {
            let kinds: Vec<&String> = map.keys().collect();
            return Err(declined(
                DiagnosticCode::UnknownRecordKind,
                format!("a record with fields {kinds:?} is no record kind of export format 3.1.0"),
            ));
        }
        let (kind, body) = map.iter().next().expect("one field");
        let body = object(body, kind)?;
        match kind.as_str() {
            "axiom" => {
                fields(
                    body,
                    "axiom",
                    &["name", "levelParams", "type", "isUnsafe"],
                    &[],
                )?;
                boolean(&body["isUnsafe"], "axiom.isUnsafe")?;
                self.simple(body, DeclarationKind::Axiom, line, &["type"])
            }
            "def" => {
                fields(
                    body,
                    "def",
                    &[
                        "name",
                        "levelParams",
                        "type",
                        "value",
                        "hints",
                        "safety",
                        "all",
                    ],
                    &[],
                )?;
                naturals(&body["all"], "def.all")?;
                match &body["safety"] {
                    Value::String(s) if ["safe", "unsafe", "partial"].contains(&s.as_str()) => {}
                    other => {
                        return Err(declined(
                            DiagnosticCode::MalformedExport,
                            format!("def.safety {other} is not safe, unsafe or partial"),
                        ))
                    }
                }
                self.simple(body, DeclarationKind::Definition, line, &["type", "value"])
            }
            "opaque" => {
                fields(
                    body,
                    "opaque",
                    &["name", "levelParams", "type", "value", "isUnsafe", "all"],
                    &[],
                )?;
                boolean(&body["isUnsafe"], "opaque.isUnsafe")?;
                naturals(&body["all"], "opaque.all")?;
                self.simple(body, DeclarationKind::Opaque, line, &["type", "value"])
            }
            "thm" => {
                fields(
                    body,
                    "thm",
                    &["name", "levelParams", "type", "value", "all"],
                    &[],
                )?;
                naturals(&body["all"], "thm.all")?;
                self.simple(body, DeclarationKind::Theorem, line, &["type", "value"])
            }
            "quot" => {
                fields(body, "quot", &["name", "levelParams", "type", "kind"], &[])?;
                match body["kind"].as_str() {
                    Some("type" | "ctor" | "lift" | "ind") => {}
                    _ => {
                        return Err(declined(
                            DiagnosticCode::UnknownRecordKind,
                            format!(
                                "quot.kind {} is no quotient kind of export format 3.1.0",
                                body["kind"]
                            ),
                        ))
                    }
                }
                self.simple(body, DeclarationKind::Quotient, line, &["type"])
            }
            "inductive" => self.inductive(body, line),
            other => Err(declined(
                DiagnosticCode::UnknownRecordKind,
                format!("{other:?} is no declaration kind of export format 3.1.0"),
            )),
        }
    }

    fn name(&mut self, map: &Map<String, Value>) -> Result<()> {
        let index = natural(&map["in"], "in")?;
        let (part, body) = if map.contains_key("str") {
            fields(map, "a name record", &["in", "str"], &[])?;
            let body = object(&map["str"], "str")?;
            fields(body, "str", &["pre", "str"], &[])?;
            let text = string(&body["str"], "str.str")?;
            (
                format!(
                    "s:{}",
                    serde_json::to_string(&text).expect("a string serializes")
                ),
                body,
            )
        } else if map.contains_key("num") {
            fields(map, "a name record", &["in", "num"], &[])?;
            let body = object(&map["num"], "num")?;
            fields(body, "num", &["pre", "i"], &[])?;
            (format!("n:{}", natural(&body["i"], "num.i")?), body)
        } else {
            return Err(declined(
                DiagnosticCode::UnknownRecordKind,
                "a name record that is neither str nor num",
            ));
        };
        let prefix = self
            .names
            .get(&natural(&body["pre"], "pre")?)
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    "a name's prefix is not an earlier name",
                )
            })?
            .clone();
        if self
            .names
            .insert(index, format!("{prefix}/{part}"))
            .is_some()
        {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                format!("name index {index} is defined twice"),
            ));
        }
        Ok(())
    }

    fn level(&mut self, map: &Map<String, Value>) -> Result<()> {
        let index = natural(&map["il"], "il")?;
        let kind = ["succ", "max", "imax", "param"]
            .into_iter()
            .find(|k| map.contains_key(*k))
            .ok_or_else(|| {
                declined(
                    DiagnosticCode::UnknownRecordKind,
                    "a level record of no level kind of export format 3.1.0",
                )
            })?;
        fields(map, "a level record", &["il", kind], &[])?;
        let earlier = |this: &Self, v: &Value| -> Result<()> {
            let j = natural(v, kind)?;
            if this.levels.contains(&j) {
                Ok(())
            } else {
                Err(declined(
                    DiagnosticCode::MalformedExport,
                    format!("level {index} refers to level {j}, which is not an earlier level"),
                ))
            }
        };
        match kind {
            "succ" => earlier(self, &map["succ"])?,
            "param" => {
                let name = natural(&map["param"], "param")?;
                if !self.names.contains_key(&name) {
                    return Err(declined(
                        DiagnosticCode::MalformedExport,
                        "a universe parameter's name is not an earlier name",
                    ));
                }
            }
            _ => {
                let pair = map[kind]
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .ok_or_else(|| {
                        declined(
                            DiagnosticCode::MalformedExport,
                            format!("{kind} is not a pair of levels"),
                        )
                    })?;
                earlier(self, &pair[0])?;
                earlier(self, &pair[1])?;
            }
        }
        if !self.levels.insert(index) {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                format!("level index {index} is defined twice"),
            ));
        }
        Ok(())
    }

    fn expression(&mut self, map: &Map<String, Value>, line: usize) -> Result<()> {
        let index = natural(&map["ie"], "ie")?;
        if map.contains_key("mvar") || map.contains_key("fvar") {
            return Err(rejected(
                DiagnosticCode::UnresolvedMetavariable,
                format!("expression {index} is a metavariable or free variable: the declaration was never fully elaborated"),
            ));
        }
        const KINDS: [&str; 11] = [
            "bvar", "sort", "const", "app", "lam", "forallE", "letE", "proj", "natVal", "strVal",
            "mdata",
        ];
        let kind = KINDS
            .into_iter()
            .find(|k| map.contains_key(*k))
            .ok_or_else(|| {
                let fields: Vec<&String> = map.keys().collect();
                declined(DiagnosticCode::UnknownRecordKind, format!("expression {index} ({fields:?}) is no expression kind of export format 3.1.0"))
            })?;
        fields(map, "an expression record", &["ie", kind], &[])?;
        let body = &map[kind];
        let mut children = Vec::new();
        let mut constant = None;
        match kind {
            "bvar" => {
                natural(body, "bvar")?;
            }
            "sort" => self.level_ref(body)?,
            "natVal" | "strVal" => {
                string(body, kind)?;
            }
            "const" => {
                let c = object(body, "const")?;
                fields(c, "const", &["name", "us"], &[])?;
                let name = natural(&c["name"], "const.name")?;
                if !self.names.contains_key(&name) {
                    return Err(declined(
                        DiagnosticCode::MalformedExport,
                        "a constant's name is not an earlier name",
                    ));
                }
                for u in c["us"].as_array().ok_or_else(|| {
                    declined(DiagnosticCode::MalformedExport, "const.us is not an array")
                })? {
                    self.level_ref(u)?;
                }
                self.referenced.entry(name).or_insert(line);
                constant = Some(name);
            }
            "app" => {
                let a = object(body, "app")?;
                fields(a, "app", &["fn", "arg"], &[])?;
                children.push(natural(&a["fn"], "app.fn")?);
                children.push(natural(&a["arg"], "app.arg")?);
            }
            "lam" | "forallE" => {
                let b = object(body, kind)?;
                fields(b, kind, &["name", "type", "body", "binderInfo"], &[])?;
                self.name_ref(&b["name"])?;
                match b["binderInfo"].as_str() {
                    Some("default" | "implicit" | "strictImplicit" | "instImplicit") => {}
                    _ => {
                        return Err(declined(
                            DiagnosticCode::UnknownField,
                            format!(
                            "{kind}.binderInfo {} is no binder information of export format 3.1.0",
                            b["binderInfo"]
                        ),
                        ))
                    }
                }
                children.push(natural(&b["type"], kind)?);
                children.push(natural(&b["body"], kind)?);
            }
            "letE" => {
                let b = object(body, "letE")?;
                fields(b, "letE", &["name", "type", "value", "body", "nondep"], &[])?;
                self.name_ref(&b["name"])?;
                boolean(&b["nondep"], "letE.nondep")?;
                for part in ["type", "value", "body"] {
                    children.push(natural(&b[part], "letE")?);
                }
            }
            "proj" => {
                let p = object(body, "proj")?;
                fields(p, "proj", &["typeName", "idx", "struct"], &[])?;
                let structure = natural(&p["typeName"], "proj.typeName")?;
                if !self.names.contains_key(&structure) {
                    return Err(declined(
                        DiagnosticCode::MalformedExport,
                        "a projection's structure is not an earlier name",
                    ));
                }
                natural(&p["idx"], "proj.idx")?;
                self.referenced.entry(structure).or_insert(line);
                constant = Some(structure);
                children.push(natural(&p["struct"], "proj.struct")?);
            }
            "mdata" => {
                let m = object(body, "mdata")?;
                fields(m, "mdata", &["expr", "data"], &[])?;
                object(&m["data"], "mdata.data")?;
                children.push(natural(&m["expr"], "mdata.expr")?);
            }
            _ => unreachable!("every expression kind is matched"),
        }
        if let Some(child) = children.iter().find(|c| !self.expressions.contains_key(c)) {
            return Err(declined(DiagnosticCode::MalformedExport, format!("expression {index} refers to expression {child}, which is not an earlier expression")));
        }
        if self
            .expressions
            .insert(index, (children, constant))
            .is_some()
        {
            return Err(declined(
                DiagnosticCode::MalformedExport,
                format!("expression index {index} is defined twice"),
            ));
        }
        Ok(())
    }

    fn level_ref(&self, value: &Value) -> Result<()> {
        let level = natural(value, "level")?;
        if self.levels.contains(&level) {
            Ok(())
        } else {
            Err(declined(
                DiagnosticCode::MalformedExport,
                format!("level {level} is not an earlier level"),
            ))
        }
    }

    fn name_ref(&self, value: &Value) -> Result<usize> {
        let name = natural(value, "name")?;
        if self.names.contains_key(&name) {
            Ok(name)
        } else {
            Err(declined(
                DiagnosticCode::MalformedExport,
                format!("name {name} is not an earlier name"),
            ))
        }
    }

    fn expression_ref(&self, value: &Value, what: &str) -> Result<usize> {
        let expression = natural(value, what)?;
        if self.expressions.contains_key(&expression) {
            Ok(expression)
        } else {
            Err(declined(
                DiagnosticCode::MalformedExport,
                format!(
                    "{what} refers to expression {expression}, which is not an earlier expression"
                ),
            ))
        }
    }

    fn level_params(&self, value: &Value, what: &str) -> Result<()> {
        for name in naturals(value, what)? {
            if !self.names.contains_key(&name) {
                return Err(declined(
                    DiagnosticCode::MalformedExport,
                    format!("{what} names an unknown universe parameter"),
                ));
            }
        }
        Ok(())
    }

    /// A single-declaration record whose dependency roots are the expressions under `roots`.
    fn simple(
        &mut self,
        body: &Map<String, Value>,
        kind: DeclarationKind,
        line: usize,
        roots: &[&str],
    ) -> Result<()> {
        self.level_params(&body["levelParams"], "levelParams")?;
        let roots = roots
            .iter()
            .map(|field| self.expression_ref(&body[*field], field))
            .collect::<Result<Vec<_>>>()?;
        let name = self.name_ref(&body["name"])?;
        let record = self.records.len();
        self.records.push(Record {
            roots,
            members: Vec::new(),
        });
        self.declare(name, kind, line, record)
    }

    fn inductive(&mut self, body: &Map<String, Value>, line: usize) -> Result<()> {
        fields(body, "inductive", &["types", "ctors", "recs"], &[])?;
        let record = self.records.len();
        self.records.push(Record {
            roots: Vec::new(),
            members: Vec::new(),
        });
        let list = |field: &str| -> Result<Vec<Value>> {
            body[field].as_array().cloned().ok_or_else(|| {
                declined(
                    DiagnosticCode::MalformedExport,
                    format!("inductive.{field} is not an array"),
                )
            })
        };
        let (types, ctors, recs) = (list("types")?, list("ctors")?, list("recs")?);
        let mut roots = Vec::new();
        let mut members = Vec::new();
        for t in &types {
            let t = object(t, "an inductive type")?;
            fields(
                t,
                "an inductive type",
                &[
                    "name",
                    "levelParams",
                    "type",
                    "numParams",
                    "numIndices",
                    "all",
                    "ctors",
                    "numNested",
                    "isRec",
                    "isUnsafe",
                    "isReflexive",
                ],
                &[],
            )?;
            for n in ["numParams", "numIndices", "numNested"] {
                natural(&t[n], n)?;
            }
            for b in ["isRec", "isUnsafe", "isReflexive"] {
                boolean(&t[b], b)?;
            }
            naturals(&t["all"], "all")?;
            naturals(&t["ctors"], "ctors")?;
            self.level_params(&t["levelParams"], "levelParams")?;
            roots.push(self.expression_ref(&t["type"], "type")?);
            members.push((self.name_ref(&t["name"])?, DeclarationKind::Inductive));
        }
        for c in &ctors {
            let c = object(c, "a constructor")?;
            fields(
                c,
                "a constructor",
                &[
                    "name",
                    "levelParams",
                    "type",
                    "induct",
                    "cidx",
                    "numParams",
                    "numFields",
                    "isUnsafe",
                ],
                &[],
            )?;
            for n in ["induct", "cidx", "numParams", "numFields"] {
                natural(&c[n], n)?;
            }
            boolean(&c["isUnsafe"], "isUnsafe")?;
            self.level_params(&c["levelParams"], "levelParams")?;
            roots.push(self.expression_ref(&c["type"], "type")?);
            members.push((self.name_ref(&c["name"])?, DeclarationKind::Constructor));
        }
        for r in &recs {
            let r = object(r, "a recursor")?;
            fields(
                r,
                "a recursor",
                &[
                    "name",
                    "levelParams",
                    "type",
                    "all",
                    "numParams",
                    "numIndices",
                    "numMotives",
                    "numMinors",
                    "rules",
                    "k",
                    "isUnsafe",
                ],
                &[],
            )?;
            for n in ["numParams", "numIndices", "numMotives", "numMinors"] {
                natural(&r[n], n)?;
            }
            for b in ["k", "isUnsafe"] {
                boolean(&r[b], b)?;
            }
            naturals(&r["all"], "all")?;
            self.level_params(&r["levelParams"], "levelParams")?;
            roots.push(self.expression_ref(&r["type"], "type")?);
            for rule in r["rules"]
                .as_array()
                .ok_or_else(|| declined(DiagnosticCode::MalformedExport, "rules is not an array"))?
            {
                let rule = object(rule, "a recursor rule")?;
                fields(rule, "a recursor rule", &["ctor", "nfields", "rhs"], &[])?;
                self.name_ref(&rule["ctor"])?;
                natural(&rule["nfields"], "nfields")?;
                roots.push(self.expression_ref(&rule["rhs"], "rhs")?);
            }
            members.push((self.name_ref(&r["name"])?, DeclarationKind::Recursor));
        }
        self.records[record].roots = roots;
        for (name, kind) in members {
            self.declare(name, kind, line, record)?;
        }
        Ok(())
    }

    fn declare(
        &mut self,
        name: usize,
        kind: DeclarationKind,
        line: usize,
        record: usize,
    ) -> Result<()> {
        let encoded = &self.names[&name];
        let kernel_name = canonical_name(encoded);
        let lean = lean_name(&kernel_name);
        if let Some(&earlier) = self.by_kernel_name.get(&kernel_name) {
            return Err(rejected(
                DiagnosticCode::DuplicateDeclaration,
                format!(
                    "{lean} is declared again (first declared at line {})",
                    self.declarations[earlier].line
                ),
            )
            .in_declaration(lean));
        }
        let index = self.declarations.len();
        self.by_kernel_name.insert(kernel_name.clone(), index);
        self.declared.insert(name, index);
        self.records[record].members.push(index);
        self.declarations.push(Declaration {
            kernel_name,
            lean_name: lean,
            kind,
            line,
            record,
        });
        Ok(())
    }

    /// Every referenced constant must be declared somewhere in the export. The refusal names the
    /// first declaration (in export order) whose closure reaches the missing constant.
    fn finish(self, header: Header) -> Result<Intake> {
        let mut missing: Vec<(usize, usize)> = self
            .referenced
            .iter()
            .filter(|(name, _)| !self.declared.contains_key(name))
            .map(|(&name, &line)| (line, name))
            .collect();
        missing.sort();
        let intake = Intake {
            header,
            declarations: self.declarations,
            records: self.records,
            by_kernel_name: self.by_kernel_name,
            expressions: self.expressions,
            declared: self.declared,
        };
        let Some(&(first_line, name)) = missing.first() else {
            return Ok(intake);
        };
        let lean = lean_name(&canonical_name(&self.names[&name]));
        let missing_names: HashSet<usize> = missing.iter().map(|(_, n)| *n).collect();
        let referrer = intake.declarations.iter().find(|d| {
            let mut seen = HashSet::new();
            let mut work = intake.records[d.record].roots.clone();
            while let Some(e) = work.pop() {
                if !seen.insert(e) {
                    continue;
                }
                if let Some((children, constant)) = intake.expressions.get(&e) {
                    if constant.is_some_and(|c| missing_names.contains(&c)) {
                        return true;
                    }
                    work.extend(children.iter().copied());
                }
            }
            false
        });
        let refusal = rejected(
            DiagnosticCode::MissingDeclaration,
            format!("{lean} is referenced but never declared"),
        );
        Err(match referrer {
            Some(d) => refusal.at_line(d.line).in_declaration(d.lean_name.clone()),
            None => refusal.at_line(first_line),
        })
    }
}

impl std::fmt::Debug for Intake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Intake")
            .field("header", &self.header)
            .field("declarations", &self.declarations.len())
            .finish()
    }
}

// ============================================================================================
// LEAN NAMES ↔ KERNEL NAMES
// ============================================================================================

/// One component of a Lean name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Component {
    Str(String),
    Num(u64),
}

/// Parse a dotted Lean name as Lean prints it: components separated by `.`, a component that is not
/// an identifier written `«…»`, a component of digits numeric.
pub fn parse_lean_name(text: &str) -> Result<Vec<Component>> {
    let invalid = || {
        declined(
            DiagnosticCode::Usage,
            format!("{text:?} is not a Lean name"),
        )
    };
    let mut components = Vec::new();
    let mut rest = text;
    loop {
        let (component, after) = if let Some(quoted) = rest.strip_prefix('«') {
            let end = quoted.find('»').ok_or_else(invalid)?;
            (
                Component::Str(quoted[..end].to_string()),
                &quoted[end + '»'.len_utf8()..],
            )
        } else {
            let end = rest.find('.').unwrap_or(rest.len());
            let part = &rest[..end];
            if part.is_empty() {
                return Err(invalid());
            }
            let component = if part.bytes().all(|b| b.is_ascii_digit()) {
                Component::Num(part.parse().map_err(|_| invalid())?)
            } else {
                Component::Str(part.to_string())
            };
            (component, &rest[end..])
        };
        components.push(component);
        if after.is_empty() {
            return Ok(components);
        }
        rest = after.strip_prefix('.').ok_or_else(invalid)?;
        if rest.is_empty() {
            return Err(invalid());
        }
    }
}

/// The structural encoding of a Lean name, before the reader's reserved identities apply.
pub fn encode(components: &[Component]) -> String {
    components
        .iter()
        .map(|c| match c {
            Component::Str(s) => format!(
                "/s:{}",
                serde_json::to_string(s).expect("a string serializes")
            ),
            Component::Num(n) => format!("/n:{n}"),
        })
        .collect()
}

/// The kernel identity the reader registers the dotted Lean name `lean` under.
pub fn kernel_name(lean: &str) -> Result<String> {
    Ok(canonical_name(&encode(&parse_lean_name(lean)?)))
}

/// The components of a kernel identity, when it is the encoding of a Lean name.
pub fn components(kernel: &str) -> Option<Vec<Component>> {
    let encoded = uncanonical(kernel)?;
    let mut rest = encoded.as_str();
    let mut out = Vec::new();
    while !rest.is_empty() {
        rest = rest.strip_prefix('/')?;
        if let Some(s) = rest.strip_prefix("s:") {
            let mut stream = serde_json::Deserializer::from_str(s).into_iter::<String>();
            let text = stream.next()?.ok()?;
            let used = stream.byte_offset();
            out.push(Component::Str(text));
            rest = &s[used..];
        } else if let Some(n) = rest.strip_prefix("n:") {
            let end = n.find('/').unwrap_or(n.len());
            out.push(Component::Num(n[..end].parse().ok()?));
            rest = &n[end..];
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The dotted Lean name a kernel identity encodes, as Lean prints it; a kernel name that encodes no
/// Lean name is returned unchanged.
pub fn lean_name(kernel: &str) -> String {
    match components(kernel) {
        Some(components) => print_lean_name(&components),
        None => kernel.to_string(),
    }
}

/// Print components as Lean prints a name.
pub fn print_lean_name(components: &[Component]) -> String {
    components
        .iter()
        .map(|c| match c {
            Component::Num(n) => n.to_string(),
            Component::Str(s) if is_identifier(s) => s.clone(),
            Component::Str(s) => format!("«{s}»"),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_')
        && !s.bytes().all(|b| b.is_ascii_digit())
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '\'' | '!' | '?'))
}

/// Undo [`canonical_name`]'s reserved identities.
fn uncanonical(kernel: &str) -> Option<String> {
    use crate::quotient::Kind;
    let quotients = [
        (Kind::Type, "/s:\"Quot\""),
        (Kind::Constructor, "/s:\"Quot\"/s:\"mk\""),
        (Kind::Lift, "/s:\"Quot\"/s:\"lift\""),
        (Kind::Induction, "/s:\"Quot\"/s:\"ind\""),
    ];
    if let Some((_, encoded)) = quotients.iter().find(|(kind, _)| kind.name() == kernel) {
        return Some(encoded.to_string());
    }
    Some(match kernel {
        "Nat" => "/s:\"Nat\"".to_string(),
        "Zero" => "/s:\"Nat\"/s:\"zero\"".to_string(),
        "Succ" => "/s:\"Nat\"/s:\"succ\"".to_string(),
        _ => match kernel.strip_prefix("Nat/") {
            Some(suffix) => format!("/s:\"Nat\"/{suffix}"),
            None if kernel.starts_with('/') => kernel.to_string(),
            None => return None,
        },
    })
}
