//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ONE IR BELOW THE FRONT END, AND ONE TABLE OF FUSIONS OVER IT.
//!
//! The book's architecture (benchmarks/FUSIONS_WE_NEED.md, sections 5 and 7, INV-ONE-IR and INV-NO-BACKEND-FUSES):
//! a model's one LOGOS program is lifted by one front end into one IR (`tensor_graph`), its fusions are decided once by
//! one optimizer over that IR, and each chip keeps only what is genuinely its own. The older forward (MIGRATE-2), the
//! spatial pipeline (MIGRATE-3) and the D5005 engine (MIGRATE-4) read the shared table's decisions; the CUDA lane's
//! launch rewriter still derives its own until MIGRATE-1, and the spatial passes and the engine's commands keep what is
//! their chip's. No new fusion may land in any of them.
//!
//! `no_fusion_rule_is_added_outside_the_tensor_graph` is that ratchet. It reads each representation's rewrites by the
//! shape that representation gives them, and holds each set to an allowlist that only shrinks:
//!   • the lane: every function that rewrites the launch list (a `&mut Slots` or `&mut Vec<Launch>` parameter), and
//!     every rule name a fusion is recorded under;
//!   • every `fn fuse*` and `fn merge*` under `codegen_cuda/` and `spatial/`;
//!   • the engine: every command `lower_graph` writes, each a block the engine merges its ops into.
//! A new entry in any of them is a fusion added outside the IR, and fails. An allowlisted entry that is gone is a
//! row to delete: the list shrinks as each representation moves (MIGRATE-1 to MIGRATE-4).
//!
//!   cargo nextest run -p logicaffeine-tests --test one_ir_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(src().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Every `.rs` file under `rel`, by its path from the compiler's `src/`.
fn rust_files(rel: &str) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(&src().join(rel), &src(), &mut out);
    out
}

/// The identifier that starts at byte `at` of `text`.
fn ident_at(text: &str, at: usize) -> &str {
    let rest = &text[at..];
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
    &rest[..end]
}

/// The names of the functions in `text` whose signature (up to its body's brace) contains `needle`.
fn fns_whose_signature_has(text: &str, needle: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("fn ") {
        let at = from + i;
        from = at + 3;
        if at > 0 && text.as_bytes()[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        let name = ident_at(text, at + 3);
        if name.is_empty() {
            continue;
        }
        let sig_end = text[at..].find('{').map_or(text.len(), |e| at + e);
        if text[at..sig_end].contains(needle) {
            out.push(name.to_string());
        }
    }
    out
}

/// Every string literal in `text` that directly follows one of `markers`, up to its closing quote.
fn literals_after(text: &str, markers: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for m in markers {
        let mut from = 0;
        while let Some(i) = text[from..].find(m) {
            let at = from + i + m.len();
            from = at;
            if let Some(end) = text[at..].find('"') {
                out.push(text[at..at + end].to_string());
            }
        }
    }
    out
}

/// One representation's rewrites: what they are, where they were read from, and their names.
struct Census {
    what: &'static str,
    found: BTreeSet<String>,
}

/// The lane's launch rewriters, by function, and the rule names its fusions are recorded under.
fn lane() -> Vec<Census> {
    let mut rewriters = BTreeSet::new();
    let mut rules = BTreeSet::new();
    for file in rust_files("codegen_cuda/lane") {
        let text = read(&file);
        for needle in ["&mut Slots", "&mut Vec<Launch>"] {
            rewriters.extend(fns_whose_signature_has(&text, needle).into_iter().map(|f| format!("{file}::{f}")));
        }
        rules.extend(literals_after(&text, &["rule: \""]));
        // A fold recorded by the rewriter's own `absorb` / `absorb_all`: its rule is the literal before `, fusions`.
        let mut from = 0;
        while let Some(i) = text[from..].find("\", fusions") {
            let end = from + i;
            from = end + 1;
            let start = text[..end].rfind('"').expect("the rule literal opens") + 1;
            rules.insert(text[start..end].to_string());
        }
        // The rule tables' rows: `("name", function)`.
        for name in literals_after(&text, &["(\""]) {
            if text.contains(&format!("(\"{name}\", {name})")) {
                rules.insert(name);
            }
        }
    }
    vec![
        Census { what: "the lane's launch rewriters (codegen_cuda/lane)", found: rewriters },
        Census { what: "the lane's recorded fusion rules", found: rules },
    ]
}

/// Every `fn fuse*` and `fn merge*` under `codegen_cuda/` and `spatial/`.
fn fuse_and_merge() -> Census {
    let mut found = BTreeSet::new();
    for dir in ["codegen_cuda", "spatial"] {
        for file in rust_files(dir) {
            let text = read(&file);
            for prefix in ["fn fuse", "fn merge"] {
                let mut from = 0;
                while let Some(i) = text[from..].find(prefix) {
                    let at = from + i + 3;
                    from = at;
                    found.insert(format!("{file}::{}", ident_at(&text, at)));
                }
            }
        }
    }
    Census { what: "the fuse and merge functions under codegen_cuda/ and spatial/", found }
}

/// Every command the engine's block reader writes.
fn engine() -> Census {
    let text = read("spatial/decode_engine/lower_graph.rs");
    let mut found = BTreeSet::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("Command::") {
        let at = from + i + "Command::".len();
        from = at;
        found.insert(ident_at(&text, at).to_string());
    }
    Census { what: "the commands the D5005 engine's block reader writes (spatial/decode_engine/lower_graph.rs)", found }
}

/// Today's rewrites outside the IR. Each row leaves as its representation moves onto the shared table; none is ever
/// added.
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "the lane's launch rewriters (codegen_cuda/lane)",
        &["codegen_cuda/lane/plan.rs::rows_a_warp", "codegen_cuda/lane/plan.rs::sequences", "codegen_cuda/lane/plan.rs::shared_rows"],
    ),
    ("the lane's recorded fusion rules", &["rows_a_warp", "sequences"]),
    (
        "the fuse and merge functions under codegen_cuda/ and spatial/",
        &[
            "codegen_cuda/lane/plan.rs::merge_argmaxes",
            "codegen_cuda/lane/plan.rs::merge_attentions",
            "codegen_cuda/lane/plan.rs::merge_convs",
            "codegen_cuda/lane/plan.rs::merge_deltas",
            "codegen_cuda/lane/plan.rs::merge_gates",
            "codegen_cuda/lane/plan.rs::merge_joins",
            "codegen_cuda/lane/plan.rs::merge_rotations",
            "codegen_cuda/lane/plan.rs::merge_row_sets",
            "spatial/lower/cuda.rs::merge_programs",
            "spatial/passes/from_qwen_ops.rs::merge_fans",
            "spatial/passes/fuse_generic.rs::fuse_generic",
            "spatial/passes/fuse_generic.rs::fuse_once",
            "spatial/passes/fuse_readers.rs::fuse_readers",
            "spatial/passes/fuse_readers.rs::fuse_readers_forced",
            "spatial/passes/synthesize_handshakes.rs::fuse_adjacent_loops",
        ],
    ),
    (
        "the commands the D5005 engine's block reader writes (spatial/decode_engine/lower_graph.rs)",
        &[
            "Add", "Advance", "Attend", "CacheAppend", "Conv", "Delta", "Embed", "GatedNorm", "Gates", "Hadamard", "L2Heads", "Matvec",
            "Norm", "RmsHeads", "Rotate", "SigmoidGate", "Split", "SwiGlu",
        ],
    ),
];

fn censuses() -> Vec<Census> {
    let mut all = lane();
    all.push(fuse_and_merge());
    all.push(engine());
    all
}

#[test]
fn no_fusion_rule_is_added_outside_the_tensor_graph() {
    let all = censuses();
    let named: BTreeSet<&str> = ALLOWED.iter().map(|(w, _)| *w).collect();
    let read: BTreeSet<&str> = all.iter().map(|c| c.what).collect();
    assert_eq!(named, read, "every census has an allowlist, and every allowlist a census");
    let mut failures = Vec::new();
    for c in &all {
        let allowed: BTreeSet<String> = ALLOWED.iter().find(|(w, _)| *w == c.what).unwrap().1.iter().map(|s| s.to_string()).collect();
        assert!(!c.found.is_empty(), "{}: the census read nothing, so it would pass anything", c.what);
        for new in c.found.difference(&allowed) {
            failures.push(format!(
                "{}: `{new}` is a fusion added outside the IR. A fusion is a rule in tensor_graph::fuse, decided once for every chip (book 5.4); \
                 a chip's own schedule goes through its shim (5.8).",
                c.what
            ));
        }
        for gone in allowed.difference(&c.found) {
            failures.push(format!("{}: `{gone}` is gone from the source; delete its allowlist row so the list only shrinks.", c.what));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The lane derives no fusion (MIGRATE-1): its plan optimizes the step under the lane's shim and links what the shared
/// table leaves. What it still rewrites is its schedule — four rows a warp for a wide product, the shared-row attention,
/// a batch's sequences as one launch — and the only firings it records itself are the schedule's.
#[test]
fn the_lane_derives_no_fusion() {
    let census = lane();
    let found = |what: &str| census.iter().find(|c| c.what == what).map(|c| c.found.clone()).expect("a census of the lane");
    let schedule: BTreeSet<String> = ["rows_a_warp", "shared_rows", "sequences"].iter().map(|f| format!("codegen_cuda/lane/plan.rs::{f}")).collect();
    let rewriters = found("the lane's launch rewriters (codegen_cuda/lane)");
    assert_eq!(rewriters, schedule, "the lane rewrites its launches only to schedule them: {:?}", rewriters.difference(&schedule).collect::<Vec<_>>());
    let recorded: BTreeSet<String> = ["rows_a_warp", "sequences"].iter().map(|r| r.to_string()).collect();
    let rules = found("the lane's recorded fusion rules");
    assert_eq!(rules, recorded, "the lane records only its schedule's steps: {:?}", rules.difference(&recorded).collect::<Vec<_>>());
}

/// The census reads each kind of rewrite it claims to read: a launch rewriter, a recorded rule, a fuse or merge function
/// and an engine command, each written as the source writes them.
#[test]
fn the_census_sees_every_shape_of_rewrite() {
    let text = "fn quiet(a: usize) -> usize {\n}\nfn folds(s: &mut Slots, fusions: &mut Vec<Fusion>) {\n    s.absorb(j, i, \"made_up\", fusions);\n    fusions.push(Fusion { rule: \"also_made_up\", ops });\n}\n";
    assert_eq!(fns_whose_signature_has(text, "&mut Slots"), ["folds"]);
    assert_eq!(literals_after(text, &["rule: \""]), ["also_made_up"]);
    let lane_rule = |t: &str| {
        let end = t.find("\", fusions").unwrap();
        t[t[..end].rfind('"').unwrap() + 1..end].to_string()
    };
    assert_eq!(lane_rule(text), "made_up");
    assert_eq!(ident_at("Command::Matvec { tensor }", "Command::".len()), "Matvec");
}

/// Every `.rs` file of the compiler outside the IR and the front end's recognizers, by its path from `src/`.
fn backend_files() -> Vec<String> {
    rust_files(".")
        .into_iter()
        .map(|f| f.trim_start_matches("./").to_string())
        .filter(|f| !f.starts_with("tensor_graph/") && !f.starts_with("tensor_recognize/"))
        .collect()
}

/// The front end's spec types: the payload each op of the IR carries.
fn spec_types() -> BTreeSet<String> {
    let front = read("tensor_graph/front.rs");
    let at = front.find("pub enum TensorOpSpec {").expect("the IR's op enum");
    let body = &front[at..at + front[at..].find("\n}").expect("its end")];
    body.lines()
        .filter_map(|l| {
            let l = l.trim();
            let payload = &l[l.find('(')? + 1..l.rfind(')')?];
            Some(payload.rsplit("::").next()?.to_string())
        })
        .collect()
}

/// The enums declared in `text` whose variants carry one of `specs`, by name.
fn op_enums_in(text: &str, specs: &BTreeSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        let decl = line.trim_start();
        let decl = decl.strip_prefix("pub ").or_else(|| decl.strip_prefix("pub(crate) ")).or_else(|| decl.strip_prefix("pub(super) ")).unwrap_or(decl);
        let Some(rest) = decl.strip_prefix("enum ") else { continue };
        let name = ident_at(rest, 0);
        let Some(open) = text[at..].find('{').map(|o| at + o) else { continue };
        let mut depth = 0;
        let close = text[open..]
            .char_indices()
            .find(|&(_, c)| {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                depth == 0
            })
            .map_or(text.len(), |(k, _)| open + k);
        if text[open..close].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| specs.contains(w)) {
            out.push(name.to_string());
        }
    }
    out
}

/// The backend files that call a walk of the source — the front end's, or a recognizer of a backend's own — by path.
fn walk_callers() -> BTreeSet<String> {
    backend_files()
        .into_iter()
        .filter(|f| {
            read(f).lines().any(|l| {
                let l = l.trim_start();
                !l.starts_with("//") && l.contains("recognize_stages(") && !l.contains("fn recognize_stages")
            })
        })
        .collect()
}

/// INV-ONE-IR (book 5.1): below the front end there is one op type, the IR's. A backend that keeps an enum of its own
/// whose variants carry the front end's specs has lifted the program a second time; the ones that remain are listed
/// with the ticket that deletes each, and the list only shrinks.
#[test]
fn no_backend_keeps_a_private_op_enum() {
    const AWAITING_ITS_MOVE: &[(&str, &str)] = &[];
    let specs = spec_types();
    assert!(specs.len() >= 20 && specs.contains("MatmulSpec"), "the census reads the IR's spec types: {specs:?}");
    let found: BTreeSet<String> =
        backend_files().iter().flat_map(|f| op_enums_in(&read(f), &specs).into_iter().map(move |e| format!("{f}::{e}"))).collect();
    let allowed: BTreeSet<String> = AWAITING_ITS_MOVE.iter().map(|(e, _)| e.to_string()).collect();
    let added: Vec<&String> = found.difference(&allowed).collect();
    let gone: Vec<&String> = allowed.difference(&found).collect();
    assert!(
        added.is_empty(),
        "a backend keeps its own op enum over the front end's specs: {added:?}. A backend lowers the IR's ops \
         (tensor_graph::graph::Op) through its link."
    );
    assert!(gone.is_empty(), "{gone:?} no longer exist: delete their rows so the list only shrinks");
}

/// INV-ONE-IR: a backend reaches a source's ops through the IR (`TensorGraph::lift_source`), never by walking the source
/// itself. The callers left are listed with the ticket that moves each, and the list only shrinks.
#[test]
fn every_backend_lowers_the_tensor_graph() {
    const AWAITING_ITS_MOVE: &[(&str, &str)] = &[
        ("codegen_cuda/lane/mod.rs", "MIGRATE-1"),
    ];
    let found = walk_callers();
    let allowed: BTreeSet<String> = AWAITING_ITS_MOVE.iter().map(|(f, _)| f.to_string()).collect();
    let added: Vec<&String> = found.difference(&allowed).collect();
    let gone: Vec<&String> = allowed.difference(&found).collect();
    assert!(added.is_empty(), "these backends walk the source themselves: {added:?}. Lift it with TensorGraph::lift_source and lower the graph's ops.");
    assert!(gone.is_empty(), "{gone:?} no longer walk the source: delete their rows so the list only shrinks");
}

/// The op-enum census sees an enum that carries a spec, in either form a variant can carry one, and nothing else: not an
/// enum of the chip's own, and not a comment that names a spec.
#[test]
fn the_op_enum_census_sees_an_enum_that_carries_a_spec() {
    let specs: BTreeSet<String> = ["MatmulSpec", "RopeSpec"].iter().map(|s| s.to_string()).collect();
    let text = "pub enum Mine {\n    A { spec: qwen::RopeSpec, names: Names },\n    B(MatmulSpec),\n}\n\
                enum Launch {\n    Gemv { rows: usize },\n}\n// enum Comment { X(MatmulSpec) }\n\
                pub(crate) enum Nested {\n    A { inner: Box<MatmulSpec> },\n}\n";
    assert_eq!(op_enums_in(text, &specs), ["Mine", "Nested"]);
    assert!(walk_callers().iter().all(|f| !f.starts_with("tensor_graph/")), "the IR's own walk is not a backend's");
}

/// The types declared in `text` whose body names the front end's op or lifted statement, by name.
fn op_carriers_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        let decl = line.trim_start();
        let decl = decl.strip_prefix("pub ").or_else(|| decl.strip_prefix("pub(crate) ")).or_else(|| decl.strip_prefix("pub(super) ")).unwrap_or(decl);
        let Some(rest) = decl.strip_prefix("struct ").or_else(|| decl.strip_prefix("enum ")) else { continue };
        let name = ident_at(rest, 0);
        let Some(open) = text[at..].find(['{', ';']).map(|o| at + o).filter(|&o| text.as_bytes()[o] == b'{') else { continue };
        let mut depth = 0;
        let close = text[open..]
            .char_indices()
            .find(|&(_, c)| {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                depth == 0
            })
            .map_or(text.len(), |(k, _)| open + k);
        if text[open..close].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == "TensorOpSpec" || w == "RecognizedStage") {
            out.push(name.to_string());
        }
    }
    out
}

/// The types declared outside the IR that hold the front end's ops, as `file::Type`.
fn op_carriers() -> BTreeSet<String> {
    backend_files().into_iter().flat_map(|f| op_carriers_in(&read(&f)).into_iter().map(move |t| format!("{f}::{t}"))).collect()
}

/// IR-1: the graph of the front end's ops has one home, the IR. A type outside it that holds those ops is a second
/// graph; the ones left are listed with the ticket that retires each, and the list only shrinks.
#[test]
fn the_tensor_graph_lives_in_tensor_graph() {
    const AWAITING_ITS_MOVE: &[(&str, &str)] =
        &[("codegen_cuda/lane/graph.rs::Op", "MIGRATE-1's window: the lane's StepGraph becomes the IR's TensorGraph (IR-1's move)")];
    let found = op_carriers();
    let allowed: BTreeSet<String> = AWAITING_ITS_MOVE.iter().map(|(t, _)| t.to_string()).collect();
    let added: Vec<&String> = found.difference(&allowed).collect();
    let gone: Vec<&String> = allowed.difference(&found).collect();
    assert!(added.is_empty(), "these types hold the front end's ops outside the IR: {added:?}. A backend holds tensor_graph::graph::TensorGraph.");
    assert!(gone.is_empty(), "{gone:?} no longer hold the front end's ops: delete their rows so the list only shrinks");
    let witness = "pub struct Graph {\n    ops: Vec<Op>,\n}\npub(crate) struct Op {\n    spec: TensorOpSpec,\n}\nstruct Unit;\nenum Lifted {\n    One(RecognizedStage),\n}\n// struct Note { spec: TensorOpSpec }\n";
    assert_eq!(op_carriers_in(witness), ["Op", "Lifted"], "the census sees a type holding the front end's op or lifted statement, and nothing else");
}

/// IR-1: the front end's walk is defined in `tensor_graph::front` and nowhere else; a recognizer of a backend's own is
/// listed with the ticket that retires it, and the list only shrinks.
#[test]
fn the_front_end_lives_in_tensor_graph() {
    const AWAITING_ITS_MOVE: &[(&str, &str)] = &[];
    assert!(read("tensor_graph/front.rs").contains("pub fn recognize_stages(source: &str)"), "the front end's walk is in tensor_graph::front");
    let found: BTreeSet<String> = backend_files().into_iter().filter(|f| read(f).lines().any(|l| !l.trim_start().starts_with("//") && l.contains("fn recognize_stages"))).collect();
    let allowed: BTreeSet<String> = AWAITING_ITS_MOVE.iter().map(|(f, _)| f.to_string()).collect();
    let added: Vec<&String> = found.difference(&allowed).collect();
    let gone: Vec<&String> = allowed.difference(&found).collect();
    assert!(added.is_empty(), "these files define a walk of their own: {added:?}. The front end is tensor_graph::front's.");
    assert!(gone.is_empty(), "{gone:?} no longer define a walk: delete their rows so the list only shrinks");
}

/// Every tiny architecture's decode step, prefill chunk of 4 rows and batch of 2 sequences, each as the IR lifts the cut
/// (`TensorGraph::of_cut`), by name, with the cut and the spec.
#[cfg(feature = "codegen")]
fn tiny_cut_graphs() -> Vec<(String, logicaffeine_compile::model::program::Cut, logicaffeine_compile::model::spec::ModelSpec, logicaffeine_compile::tensor_graph::graph::TensorGraph)> {
    use logicaffeine_compile::model::program::Cut;
    use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
    #[path = "common/tiny_models.rs"]
    mod tiny_models;
    let mut out = Vec::new();
    for t in tiny_models::ALL {
        let spec = t.spec(24);
        for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
            let g = TensorGraph::of_cut(&spec, cut, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{} {cut}: {e}", t.name()));
            out.push((format!("{} {cut}", t.name()), cut, spec.clone(), g));
        }
    }
    out
}

/// IR-1: every tensor carries its facts. Its element kind is the front end's; only an integer is ranged, and every
/// integer of a tiny cut is an index ranged within what it indexes — a token or a next token within the vocabulary, a
/// position within the context, counted from one; a constant lives where the lift's storage says and every other tensor
/// is computed. An integer whose interval nothing proves is unranged rather than given one, and a tensor a `Show`
/// reads is observed.
#[cfg(feature = "codegen")]
#[test]
fn every_tensor_carries_its_facts() {
    use logicaffeine_compile::model::program::Cut;
    use logicaffeine_compile::tensor_graph::graph::{Elem, Role, Storage, TensorGraph};
    let mut checked = 0usize;
    for (what, cut, spec, g) in tiny_cut_graphs() {
        let ctx = match cut {
            Cut::Decode { ctx } | Cut::Prefill { ctx, .. } | Cut::Batch { ctx, .. } => ctx as i64,
            _ => unreachable!("the tiny cuts are steps"),
        };
        let vocab = spec.vocab as i64;
        for t in g.tensors.values() {
            checked += 1;
            let storage = if t.role == Role::Constant { Storage::Values } else { Storage::Computed };
            assert_eq!(t.storage, storage, "{what}: `{}` ({:?}) lives where the lift says", t.name, t.role);
            match t.elem {
                Elem::Float => assert_eq!(t.range, None, "{what}: `{}` holds floats and is ranged", t.name),
                Elem::Int => {
                    let bound = if t.name.starts_with("pos") { ctx } else { vocab };
                    assert!(
                        t.range.is_some_and(|(lo, hi)| 1 <= lo && lo <= hi && hi <= bound),
                        "{what}: the index `{}` is ranged within [1, {bound}], not {:?}",
                        t.name,
                        t.range
                    );
                }
            }
            if t.role == Role::Param {
                assert_eq!(t.elem, Elem::Int, "{what}: the index `{}` is an integer", t.name);
            }
        }
        for name in g.tensors.keys().filter(|n| n.starts_with("next")) {
            assert_eq!(g.tensors[name].elem, Elem::Int, "{what}: the next token `{name}` is an index");
        }
        assert!(g.tensors.values().any(|t| t.role == Role::Constant && t.elem == Elem::Float), "{what}: the weights hold floats");
    }
    assert!(checked > 1000, "the tiny cuts' tensors are read: {checked}");
    let lut = |first: i64, shown: bool| {
        let mut s = String::from(
            "## Main\nLet Q be a 1 by 2 tensor of Int.\nSet item 1 of Q to 1.\nSet item 2 of Q to 2.\n\
             Let K be a 2 by 2 tensor of Int.\nSet item 1 of K to 3.\n\
             Let V be a 2 by 2 tensor of Int.\nSet item 1 of V to -9.\nSet item 4 of V to 11.\n",
        );
        s += &format!("Let E be a 1 by 2 tensor of Int.\nSet item 1 of E to {first}.\nSet item 2 of E to 5.\n");
        s += "Let C be lut attention of Q over K and V with 1 heads 1 groups with table E and scale 1 shifted right by 0.\n";
        if shown {
            s += "Show item 1 of C.\n";
        }
        TensorGraph::lift_source(&s, &|_| Storage::Values).expect("the attention walks").graph
    };
    let held = lut(7, false);
    assert_eq!((held.tensors["C"].elem, held.tensors["C"].range), (Elem::Int, Some((-9, 11))), "a positive table: the context lies among the values");
    assert!(!held.tensors["C"].observed, "nothing but the op reads C");
    let unproven = lut(0, true);
    assert_eq!((unproven.tensors["C"].elem, unproven.tensors["C"].range), (Elem::Int, None), "a zero weight proves nothing, so C is unranged");
    assert!(unproven.tensors["C"].observed, "a Show reads C");
}

/// IR-1: what a step writes in place is explicit in the IR — the tables its ops update as state, and every table a
/// destination lands rows or units in — and the state among them is a fact of the program, not of its fusions: the tables
/// the front end's ops update in place, before the shared optimizer and after it, with every fused member's writes
/// counted. Fusing adds only the activations a placement assembles in place, each part written where it lands. A table an
/// op assembles a row at a time is that op's result, computed whole.
#[cfg(feature = "codegen")]
#[test]
fn state_writes_are_explicit() {
    use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
    use logicaffeine_compile::tensor_graph::fuse::optimize;
    use logicaffeine_compile::tensor_graph::graph::{Op, Role, Sym};
    use logicaffeine_compile::tensor_graph::shim::ReferenceShim;
    use logicaffeine_compile::model::program;
    fn leaves<'a>(op: &'a Op, out: &mut Vec<&'a Op>) {
        match &op.sym {
            Sym::Fused(f) => f.parts.iter().for_each(|p| leaves(p, out)),
            _ => out.push(op),
        }
    }
    let written = |ops: &[Op]| -> BTreeSet<String> {
        let mut all = Vec::new();
        ops.iter().for_each(|o| leaves(o, &mut all));
        all.iter()
            .flat_map(|o| o.state_inputs().into_iter().map(|k| o.ins[k].clone()).chain((o.dst.row.is_some() || o.dst.unit.is_some()).then(|| o.dst.buf.clone())))
            .collect()
    };
    for (what, cut, spec, g) in tiny_cut_graphs() {
        let ((text, _, _), _) = program::written(&spec, cut);
        let rp = recognize_stages(&text).expect("the cut walks");
        let walked: BTreeSet<String> = rp.stages.iter().flat_map(|st| st.op.state_inputs().into_iter().map(|k| st.ins[k].clone())).collect();
        assert_eq!(g.writes_in_place(), walked, "{what}: the lifted graph writes in place what the front end finds as state");
        assert_eq!(written(&g.ops), walked, "{what}: each op's in-place writes are its state inputs and its row and unit destinations");
        let fused = optimize(&g, &ReferenceShim, &[]).graph;
        assert!(fused.ops.len() < g.ops.len(), "{what}: the shared table fuses something");
        let state = |set: BTreeSet<String>| -> BTreeSet<String> {
            let (state, assembled): (BTreeSet<String>, BTreeSet<String>) = set.into_iter().partition(|t| g.tensors[t].role != Role::Activation);
            assert!(assembled.iter().all(|t| !walked.contains(t)), "{what}: an activation assembled in place is no state");
            state
        };
        assert_eq!(state(written(&fused.ops)), walked, "{what}: the fused members write in place the state the program does");
        assert_eq!(state(fused.writes_in_place()), walked, "{what}: fusing moves no in-place write of state out of sight");
        assert_eq!(fused.caches(), g.caches(), "{what}: fusing hides no attention's cache");
    }
}

/// IR-1: a graph lifted from a cut of a model's program records the cut's identity, and it is the identity the CUDA
/// lane records for the same cut: one program, one name, in every representation.
#[cfg(feature = "codegen")]
#[test]
fn every_lifted_graph_records_its_cut() {
    use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
    use logicaffeine_compile::codegen_cuda::lane::{compile_batch_holding, FormKind, LaneOptions};
    use logicaffeine_compile::model::program::{self, Cut};
    use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
    #[path = "common/scratch.rs"]
    mod scratch;
    #[path = "common/tiny_models.rs"]
    mod tiny_models;
    let mut compared = 0usize;
    for (what, cut, spec, g) in tiny_cut_graphs() {
        let ((text, _, _), identity) = program::written(&spec, cut);
        assert_eq!(g.identity.as_ref(), Some(&identity), "{what}: the lifted graph records its cut");
        assert_eq!(identity, program::Identity::of(cut, &text), "{what}: the identity names the text the graph was lifted from");
    }
    for t in tiny_models::ALL {
        let spec = t.spec(24);
        let ckpt = scratch::dir("logos_one_ir_lock_identity").join(format!("{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        let lane = compile_batch_holding(&spec, &ckpt, &LaneOptions { ctx: 24, knobs: CudaKnobs::default(), prefill: 4 }, 2, &[])
            .unwrap_or_else(|e| panic!("{}: {e}", t.name()));
        for form in &lane.forms {
            let cut = match form.kind {
                FormKind::Decode => Cut::Decode { ctx: 24 },
                FormKind::Prefill => Cut::Prefill { rows: form.rows, ctx: 24 },
                FormKind::Batch => Cut::Batch { rows: form.rows, ctx: 24 },
            };
            let g = TensorGraph::of_cut(&spec, cut, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{} {cut}: {e}", t.name()));
            assert_eq!(g.identity.as_ref(), Some(&form.identity), "{} {cut}: the IR and the lane name one program", t.name());
            compared += 1;
        }
    }
    assert!(compared >= tiny_models::ALL.len() * 3, "every lane form is compared: {compared}");
}


/// The shared table, under the lane's shim, fires exactly the fusions the lane's own rule table fires — rule by rule,
/// over the same lifted ops, the activations quantized in the same producers' epilogues and the same products merged —
/// and leaves the same launches in the same order, each computing the same lifted ops: for every tiny architecture under
/// every numerics, with the epilogue quantize on and off, in the decode step, the prefill chunks and a batch.
/// MIGRATE-1 deletes the lane's table only once this holds.
#[cfg(feature = "codegen")]
#[test]
fn the_shared_table_fires_what_the_lane_fires() {
    use std::collections::BTreeMap;

    use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
    use logicaffeine_compile::codegen_cuda::lane::plan::Launch;
    use logicaffeine_compile::codegen_cuda::lane::{compile_batch_holding, FormKind, LaneOptions};
    use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
    use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::model::spec::ModelSpec;
    use logicaffeine_compile::codegen_cuda::lane::plan::gates_batch_rows;
    use logicaffeine_compile::tensor_graph::fuse::{classes_built, optimize, rule_names, ADVANCE};
    use logicaffeine_compile::tensor_graph::graph::Sym;
    use logicaffeine_compile::tensor_graph::shim::{Shim, Support};

    #[path = "common/lane_cuts.rs"]
    mod lane_cuts;
    #[path = "common/scratch.rs"]
    mod scratch;
    #[path = "common/tiny_models.rs"]
    mod tiny_models;
    use tiny_models::Tiny;

    let shared: BTreeSet<&str> = rule_names().into_iter().chain([ADVANCE.name, "fused_quant", "gemv_parts", "latent_expanded"]).collect();
    let set = |ops: &[usize]| ops.iter().copied().collect::<BTreeSet<usize>>().into_iter().collect::<Vec<usize>>();
    // Every tiny architecture in Q8_0 with chunks of 4 rows, within the GEMV's; the wide models with their matrices in
    // the block formats by turns and chunks of 32 rows, on the tensor cores; the medium models in Q4_K, whose wide
    // products the GEMV takes four rows a warp.
    enum Weights {
        Q8,
        Turns,
        Q4k,
    }
    let mut models: Vec<(String, ModelSpec, Weights, usize, usize)> =
        tiny_models::ALL.iter().map(|&t| (t.name().to_string(), t.spec(24), Weights::Q8, 24, 4)).collect();
    models.extend(
        [Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::Qwen35, Tiny::GptOss, Tiny::DeepSeek2].map(|t| (format!("{}-wide", t.name()), t.wide(64), Weights::Turns, 64, 32)),
    );
    models.extend([Tiny::Qwen3, Tiny::Qwen35, Tiny::DeepSeek2].map(|t| (format!("{}-medium", t.name()), t.medium(64), Weights::Q4k, 64, 4)));
    let mut differ = Vec::new();
    let mut compared = 0usize;
    let mut fired_anywhere: BTreeSet<String> = BTreeSet::new();
    for (name, spec, weights, ctx, prefill) in &models {
        let ckpt = scratch::dir("logos_one_ir_lock").join(format!("{name}.gguf"));
        // The matrices a block format takes: rows of whole super-blocks.
        let matrices: Vec<String> = tiny_models::tensors(spec)
            .into_iter()
            .filter(|(n, dims)| n.ends_with(".weight") && dims.len() >= 2 && dims[0] % 256 == 0)
            .map(|(n, _)| n)
            .filter(|n| !n.contains("norm") && !n.contains("attn_sinks") && !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
            .collect();
        match weights {
            Weights::Q8 => tiny_models::write_checkpoint(spec, &ckpt),
            Weights::Turns => tiny_models::write_checkpoint_as(spec, &ckpt, &|t| match matrices.iter().position(|m| m == t) {
                Some(k) => tiny_models::BLOCK_FORMATS[k % tiny_models::BLOCK_FORMATS.len()],
                None => GgmlType::F32,
            }),
            Weights::Q4k => tiny_models::write_checkpoint_as(spec, &ckpt, &|t| {
                if matrices.iter().any(|m| m == t) && !t.contains("ffn_gate_inp") {
                    GgmlType::Q4K
                } else {
                    GgmlType::F32
                }
            }),
        }
        let configurations =
            [(Q8MatKind::Dequant, true), (Q8MatKind::Int8Act, true), (Q8MatKind::Int8Act, false), (Q8MatKind::TwoPassInt8, true), (Q8MatKind::TwoPassInt8, false)];
        for (numerics, fuse_quant) in configurations {
            let knobs = CudaKnobs { activations: numerics, fuse_quant, ..CudaKnobs::default() };
            // A batch's launches over its sequences as one (`sequences`) are the lane's schedule, laid over its buffers
            // after the plan: held, the batch's launches are its plan's.
            let lane = compile_batch_holding(spec, &ckpt, &LaneOptions { ctx: *ctx, knobs, prefill: *prefill }, 2, &["sequences"])
                .unwrap_or_else(|e| panic!("{name} {numerics:?}: the lane: {e}"));
            for form in &lane.forms {
                let what = format!("{name} {numerics:?} fuse_quant={fuse_quant} {}", form.tag());
                let g = lane_cuts::lifted(form, &what);
                let shim = LaneShim { knobs, batch: form.kind == FormKind::Batch, held: Vec::new() };
                let o = optimize(&g, &shim, &[]);
                // The launches, in order: what each computes, and where the lane quantizes an activation by a launch of
                // its own, a quantize op. The lane runs a latent attention taken whole (`mla`) as three launches, its key
                // half and its value half right before the attention: the three as the one op the shared table takes.
                let mut lane_launches: Vec<(bool, Vec<usize>)> =
                    form.schedule.launches.iter().zip(&form.schedule.ops).map(|(l, ops)| (matches!(l, Launch::Quant { .. }), set(ops))).collect();
                let mut kinds: Vec<&Launch> = form.schedule.launches.iter().collect();
                let mut expanded: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
                for f in form.schedule.fusions.iter().filter(|f| f.rule == "latent_expanded") {
                    let covered = set(&f.ops);
                    let at = lane_launches
                        .iter()
                        .zip(&kinds)
                        .position(|((_, ops), l)| matches!(l, Launch::Attention { split: Some(_), .. }) && ops.iter().all(|o| covered.contains(o)))
                        .unwrap_or_else(|| panic!("{what}: the expanded attention's launch"));
                    let attend = lane_launches[at].1.clone();
                    assert!(
                        at >= 2 && kinds[at - 2..at].iter().all(|l| matches!(l, Launch::HeadGemv { shared: true, .. })),
                        "{what}: the halves run right before the expanded attention"
                    );
                    let whole = set(&[lane_launches[at - 2].1.clone(), lane_launches[at - 1].1.clone(), attend.clone()].concat());
                    lane_launches.splice(at - 2..=at, [(false, whole.clone())]);
                    kinds.drain(at - 2..at);
                    expanded.insert(covered.clone(), whole.clone());
                    expanded.insert(attend, whole);
                }
                // A batch's gated attentions (`attn_output_gate` over its sequences' rows, alone or with the quantize of
                // their product) run as each sequence's attention, gating its row, and its row's set, which `sequences`
                // then runs as one: those launches, in a row, as the one op the shared table takes.
                for op in o.graph.ops.iter().filter(|op| gates_batch_rows(op)) {
                    let covers = set(&op.covers);
                    let at: Vec<usize> =
                        lane_launches.iter().enumerate().filter(|(_, (_, ops))| !ops.is_empty() && ops.iter().all(|k| covers.contains(k))).map(|(i, _)| i).collect();
                    let joined = set(&at.iter().flat_map(|&i| lane_launches[i].1.clone()).collect::<Vec<usize>>());
                    assert!(
                        !at.is_empty() && at.windows(2).all(|w| w[1] == w[0] + 1) && joined == covers,
                        "{what}: a batch's gated attentions run as launches in a row computing their op: {at:?}"
                    );
                    let (first, last) = (at[0], at[at.len() - 1]);
                    lane_launches.splice(first..=last, [(false, covers)]);
                    kinds.drain(first..last);
                }
                // Each firing as the set of lifted ops it computes (the lane records some in their group's order). The
                // lane's tensor-core merge records no firing: its launches in parts that no `gemv_parts` built are its.
                let mut lane_fired: Vec<(String, Vec<usize>)> = form
                    .schedule
                    .fusions
                    .iter()
                    .filter(|f| shared.contains(f.rule))
                    .map(|f| (f.rule.to_string(), expanded.get(&set(&f.ops)).cloned().unwrap_or_else(|| set(&f.ops))))
                    .collect();
                let in_parts: Vec<Vec<usize>> = lane_fired.iter().filter(|(r, _)| r == "gemv_parts").map(|(_, ops)| ops.clone()).collect();
                lane_fired.extend(
                    form.schedule
                        .launches
                        .iter()
                        .zip(&form.schedule.ops)
                        .filter(|(l, ops)| matches!(l, Launch::GemmParts { .. }) && !in_parts.contains(&set(ops)))
                        .map(|(_, ops)| ("merged_projections".to_string(), set(ops))),
                );
                let mut shared_fired: Vec<(String, Vec<usize>)> = o.fired.iter().map(|f| (f.rule.to_string(), set(&f.covers))).collect();
                lane_fired.sort();
                shared_fired.sort();
                fired_anywhere.extend(shared_fired.iter().map(|(r, _)| r.clone()));
                compared += 1;
                if lane_fired != shared_fired {
                    let only_lane: Vec<_> = lane_fired.iter().filter(|f| !shared_fired.contains(f)).collect();
                    let only_shared: Vec<_> = shared_fired.iter().filter(|f| !lane_fired.contains(f)).collect();
                    differ.push(format!("{what}:\n    the lane alone: {only_lane:?}\n    the shared table alone: {only_shared:?}"));
                }
                let shared_launches: Vec<(bool, Vec<usize>)> =
                    o.graph.ops.iter().map(|op| (matches!(op.sym, Sym::Quantize(_)), set(&op.covers))).collect();
                if lane_launches != shared_launches {
                    let at = lane_launches.iter().zip(&shared_launches).position(|(a, b)| a != b).unwrap_or(lane_launches.len().min(shared_launches.len()));
                    let around = |v: &[(bool, Vec<usize>)]| v[at.saturating_sub(2)..(at + 3).min(v.len())].to_vec();
                    differ.push(format!(
                        "{what}: the launches part at {at} ({} lane, {} shared):\n    the lane: {:?}\n    the shared table: {:?}",
                        lane_launches.len(),
                        shared_launches.len(),
                        around(&lane_launches),
                        around(&shared_launches)
                    ));
                }
            }
        }
    }
    assert!(differ.is_empty(), "{} of {compared} forms differ:\n{}", differ.len(), differ.join("\n"));
    assert!(compared >= models.len() * 5 * 4, "the differential compares every form: {compared}");
    // The rules the lane links: those whose every class the lane runs whole under its default knobs.
    let lane = LaneShim { knobs: CudaKnobs::default(), batch: false, held: Vec::new() };
    let built = classes_built();
    let linked: Vec<&str> = shared
        .iter()
        .chain(&["merged_projections"])
        .filter(|r| built.get(*r).is_none_or(|classes| classes.iter().all(|c| lane.supports(*c) == Support::Native)))
        .copied()
        .collect();
    let unexercised: Vec<&str> = linked.into_iter().filter(|r| !fired_anywhere.contains(*r)).collect();
    assert!(unexercised.is_empty(), "the models exercise every rule the lane links; these never fired: {unexercised:?}");
}

/// The lane's link builds the launches the lane plans: every tiny, wide and medium architecture's every form, optimized
/// under the lane's shim and linked op by op, is `plan.rs`'s launch list exactly — but for what the lane's schedule sets
/// after the link (four rows a warp, the shared-row attention, the split counts). MIGRATE-1 swaps `plan.rs`'s own
/// lowering and rule table for the link only once this holds.
#[cfg(feature = "codegen")]
#[test]
fn the_lane_link_builds_the_lanes_launches() {
    use std::collections::BTreeMap;

    use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
    use logicaffeine_compile::codegen_cuda::lane::plan::{Fill, Launch};
    use logicaffeine_compile::codegen_cuda::lane::weights::Placement;
    use logicaffeine_compile::codegen_cuda::lane::{batch_io, compile_batch_holding, decode_io, prefill_io, FormKind, LaneOptions};
    use logicaffeine_compile::codegen_cuda::lane_link::link;
    use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
    use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::tensor_graph::fuse::optimize;

    #[path = "common/lane_cuts.rs"]
    mod lane_cuts;
    #[path = "common/scratch.rs"]
    mod scratch;
    #[path = "common/tiny_models.rs"]
    mod tiny_models;
    use tiny_models::Tiny;

    // What the lane's schedule sets after the link, set aside on both sides: a mixture's, inside the fold that finishes it
    // too.
    let unscheduled = |l: &Launch| {
        let mut l = l.clone();
        if let Some(Launch::Moe { per_warp, router_split, .. }) = l.mixture_mut() {
            (*per_warp, *router_split) = (1, 1);
        }
        match &mut l {
            Launch::Gemv { per_warp, .. } | Launch::GemvGlu { per_warp, .. } => *per_warp = 1,
            Launch::GemvF32 { split, .. } => *split = 1,
            Launch::GateProj { parts, .. } => *parts = 1,
            Launch::Attention { shared, .. } => *shared = None,
            _ => {}
        }
        l
    };
    let mut models: Vec<(String, logicaffeine_compile::model::spec::ModelSpec, bool, usize, usize)> =
        tiny_models::ALL.iter().map(|&t| (t.name().to_string(), t.spec(24), false, 24, 4)).collect();
    models.extend([Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::Qwen35, Tiny::GptOss, Tiny::DeepSeek2].map(|t| (format!("{}-wide", t.name()), t.wide(64), true, 64, 32)));
    models.extend([Tiny::Qwen3, Tiny::Qwen35, Tiny::DeepSeek2].map(|t| (format!("{}-medium", t.name()), t.medium(64), true, 64, 4)));
    let mut differ = Vec::new();
    let mut compared = 0usize;
    for (name, spec, blocks, ctx, prefill) in &models {
        let ckpt = scratch::dir("logos_one_ir_link").join(format!("{name}.gguf"));
        let matrices: Vec<String> = tiny_models::tensors(spec)
            .into_iter()
            .filter(|(n, dims)| n.ends_with(".weight") && dims.len() >= 2 && dims[0] % 256 == 0)
            .map(|(n, _)| n)
            .filter(|n| !n.contains("norm") && !n.contains("attn_sinks") && !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
            .collect();
        if *blocks {
            tiny_models::write_checkpoint_as(spec, &ckpt, &|t| match matrices.iter().position(|m| m == t) {
                Some(k) => tiny_models::BLOCK_FORMATS[k % tiny_models::BLOCK_FORMATS.len()],
                None => GgmlType::F32,
            });
        } else {
            tiny_models::write_checkpoint(spec, &ckpt);
        }
        for (numerics, fuse_quant) in [(Q8MatKind::Dequant, true), (Q8MatKind::Int8Act, true), (Q8MatKind::Int8Act, false), (Q8MatKind::TwoPassInt8, true)] {
            let knobs = CudaKnobs { activations: numerics, fuse_quant, ..CudaKnobs::default() };
            let lane = compile_batch_holding(spec, &ckpt, &LaneOptions { ctx: *ctx, knobs, prefill: *prefill }, 2, &["sequences"])
                .unwrap_or_else(|e| panic!("{name} {numerics:?}: the lane: {e}"));
            for form in &lane.forms {
                let what = format!("{name} {numerics:?} fuse_quant={fuse_quant} {}", form.tag());
                let g = lane_cuts::lifted(form, &what);
                let shim = LaneShim { knobs, batch: form.kind == FormKind::Batch, held: Vec::new() };
                let o = optimize(&g, &shim, &[]);
                let constants: BTreeMap<String, Placement> = form
                    .schedule
                    .buffers
                    .iter()
                    .filter_map(|(n, b)| match &b.fill {
                        Fill::Constant(p) => Some((n.clone(), p.clone())),
                        _ => None,
                    })
                    .collect();
                let io = match form.kind {
                    FormKind::Decode => decode_io(),
                    FormKind::Prefill => prefill_io(),
                    FormKind::Batch => batch_io(form.rows),
                };
                compared += 1;
                let linked = match link(&o.graph, &constants, &shim, &io) {
                    Ok(l) => l,
                    Err((k, why)) => {
                        differ.push(format!("{what}: op {k} ({}) does not link: {why}", o.graph.ops[k].kind()));
                        continue;
                    }
                };
                let ours: Vec<Launch> = linked.iter().map(|(l, _)| unscheduled(l)).collect();
                let theirs: Vec<Launch> = form.schedule.launches.iter().map(unscheduled).collect();
                let set = |ops: &[usize]| ops.iter().copied().collect::<std::collections::BTreeSet<usize>>();
                let covers_differ = linked.len() == form.schedule.ops.len() && linked.iter().zip(&form.schedule.ops).any(|((_, a), b)| set(a) != set(b));
                if covers_differ {
                    differ.push(format!("{what}: a launch computes other lifted ops than the lane's"));
                }
                if ours != theirs {
                    let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
                    differ.push(format!(
                        "{what}: launch {at} of {} linked, {} planned:\n    linked:  {:?}\n    planned: {:?}",
                        ours.len(),
                        theirs.len(),
                        ours.get(at),
                        theirs.get(at)
                    ));
                }
            }
        }
    }
    assert!(compared >= models.len() * 4 * 4, "every form is linked: {compared}");
    assert!(differ.is_empty(), "{} of {compared} forms link differently:\n{}", differ.len(), differ.join("\n"));
}

