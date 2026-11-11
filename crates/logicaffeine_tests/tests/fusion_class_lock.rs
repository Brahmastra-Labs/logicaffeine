//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY FUSION HAS A NAME AND A HOME BEFORE ITS RULE EXISTS.
//!
//! `tensor_graph::symbols` registers every class the book names (benchmarks/FUSIONS_WE_NEED.md, IR-3): the ledger's
//! classes (`benchmarks/fusion_parity.json`), the classes the book's fusion map adds (section 3.4), and the classes only
//! this compiler has (section 5.3). Each says what it is — fused, primitive, placement, numerics or schedule — and,
//! where a sugar writes it today, carries an exemplar program holding its pattern.
//!
//!   cargo nextest run -p logicaffeine-tests --test fusion_class_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::symbols::{ClassId, Exemplar, Kind, Origin, CLASSES};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger_classes() -> BTreeSet<String> {
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root().join("benchmarks/fusion_parity.json")).unwrap()).unwrap();
    doc["classes"].as_array().expect("the ledger's classes").iter().map(|c| c["id"].as_str().unwrap().to_string()).collect()
}

/// The fused symbols the book's section 3.4 names, by their bold ids.
fn book_symbols() -> BTreeSet<String> {
    let book = std::fs::read_to_string(root().join("benchmarks/FUSIONS_WE_NEED.md")).unwrap();
    let start = book.find("### 3.4 Every fusion on every chip").expect("section 3.4");
    let end = book[start..].find("### 3.5").map_or(book.len(), |e| start + e);
    book[start..end]
        .lines()
        .filter_map(|l| l.strip_prefix("- [ ] **`").or_else(|| l.strip_prefix("- [x] **`")))
        .filter_map(|l| l.split_once('`').map(|(id, _)| id.to_string()))
        .collect()
}

#[test]
fn every_ledger_class_is_a_registered_symbol() {
    let registered: BTreeSet<&str> = CLASSES.iter().map(|c| c.id).collect();
    let ledger = ledger_classes();
    for id in &ledger {
        assert!(registered.contains(id.as_str()), "the ledger's class `{id}` has no registered symbol");
    }
    for c in CLASSES {
        assert_eq!(c.origin == Origin::Ledger, ledger.contains(c.id), "`{}`: registered as {:?}, and the ledger {} it", c.id, c.origin, if ledger.contains(c.id) { "tracks" } else { "does not track" });
    }
    let book = book_symbols();
    assert!(book.len() >= 80, "the book's fusion map names its symbols: {}", book.len());
    for id in &book {
        assert!(registered.contains(id.as_str()), "the book's symbol `{id}` has no registered class");
    }
    for c in CLASSES.iter().filter(|c| c.origin != Origin::Ours) {
        assert!(book.contains(c.id), "`{}` is registered from the ledger or the book, and the book's map does not name it", c.id);
    }
    let mut seen = BTreeSet::new();
    for c in CLASSES {
        assert!(seen.insert(c.id), "`{}` is registered twice", c.id);
    }
}

#[test]
fn every_registered_symbol_names_its_kind() {
    for c in CLASSES {
        let named = match c.kind {
            Kind::Fused | Kind::Primitive | Kind::Placement | Kind::Numerics | Kind::Schedule => true,
        };
        assert!(named, "{}", c.id);
        if c.kind == Kind::Schedule {
            assert!(matches!(c.exemplar, Exemplar::None(_)), "`{}`: a schedule class has no exemplar rule", c.id);
        }
        if let Exemplar::None(why) = c.exemplar {
            assert!(why.split_whitespace().count() >= 4, "`{}`: a class without an exemplar says why: `{why}`", c.id);
        }
    }
    for id in ["strided_read", "row_write", "join_in_place", "strided_values"] {
        assert_eq!(CLASSES.iter().find(|c| c.id == id).map(|c| c.kind), Some(Kind::Placement), "{id} is a layout on an edge");
    }
    for id in ["cuda_graphs", "persistent_layer_megakernel"] {
        assert_eq!(CLASSES.iter().find(|c| c.id == id).map(|c| c.kind), Some(Kind::Schedule), "{id} is how a chip runs a step");
    }
}

#[test]
fn no_symbol_id_is_both_ledger_and_ours() {
    let ledger = ledger_classes();
    for c in CLASSES.iter().filter(|c| c.origin == Origin::Ours) {
        assert!(!ledger.contains(c.id), "`{}` is registered as ours, and the ledger tracks it from another stack", c.id);
    }
    for id in ["strided_read", "join_in_place", "strided_values", "embed_scale", "gdn_gating", "conv_silu_l2norm", "gdn_output_quant", "gemv_quant"] {
        assert_eq!(CLASSES.iter().find(|c| c.id == id).map(|c| c.origin), Some(Origin::Ours), "the book's section 5.3 names `{id}` ours");
    }
}

/// Whether `pattern` is a subsequence of `kinds`.
fn contains(kinds: &[&str], pattern: &[&str]) -> bool {
    let mut it = kinds.iter();
    pattern.iter().all(|p| it.any(|k| k == p))
}

#[test]
fn every_exemplar_lifts_and_contains_its_pattern() {
    let mut with = 0;
    for c in CLASSES {
        let Exemplar::Program { source, pattern } = c.exemplar else { continue };
        with += 1;
        let rp = recognize_stages(source).unwrap_or_else(|e| panic!("`{}`: its exemplar does not parse: {e:?}", c.id));
        let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("`{}`: its exemplar does not lift: {e}", c.id));
        let kinds: Vec<&str> = g.ops.iter().map(|o| o.kind()).collect();
        assert!(!pattern.is_empty(), "`{}`: an exemplar names its pattern", c.id);
        assert!(contains(&kinds, pattern), "`{}`: its exemplar lifts as {kinds:?}, which does not hold {pattern:?}", c.id);
    }
    assert!(with >= 40, "the classes with a sugar today carry exemplars: {with}");
    assert!(contains(&["A", "B", "C"], &["A", "C"]) && !contains(&["A", "B"], &["B", "A"]), "the pattern check reads order");
}

// ── The shared rules ───────────────────────────────────────────────────────────────────────────────

#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/fusion_corpus.rs"]
mod fusion_corpus;

use fusion_corpus::{inputs, io, lifted, value, witnesses, HOLD_BACKS};

use std::collections::BTreeMap;

use logicaffeine_compile::model::program::{self, Cut};
use logicaffeine_compile::tensor_graph::eval::eval;
use logicaffeine_compile::tensor_graph::fuse::{gemv, leaves, moe, optimize, realizes, rule_names, Weights, ADVANCE};
use logicaffeine_compile::tensor_graph::graph::{Op, QuantFormat};
use logicaffeine_compile::tensor_graph::shim::{Shim, Support};
use logicaffeine_compile::tensor_graph::graph::Role;
use logicaffeine_compile::tensor_graph::shim::ReferenceShim;

/// The values the optimized graph computes agree with the lifted graph's, for every tensor it keeps, and it keeps every
/// tensor the step carries or anything observes.
fn same_meaning(what: &str, lifted: &TensorGraph, optimized: &TensorGraph, fired: &[logicaffeine_compile::tensor_graph::fuse::Fired]) {
    let ins = inputs(lifted);
    let apart = eval(lifted, &ins).unwrap_or_else(|e| panic!("{what}: apart: {e}"));
    let fused = eval(optimized, &ins).unwrap_or_else(|e| panic!("{what}: fused: {e}"));
    for t in lifted.tensors.values().filter(|t| t.role == Role::State || t.observed) {
        assert!(fused.contains_key(&t.name), "{what}: `{}` is carried or observed, and the optimized graph does not keep it", t.name);
    }
    for (name, v) in &fused {
        if let Some(a) = apart.get(name) {
            let touching: Vec<String> = optimized
                .ops
                .iter()
                .filter(|o| o.writes().contains(&name.as_str()))
                .map(|o| format!("{} covering {:?}", o.kind(), o.covers))
                .collect();
            assert_eq!(a, v, "{what}: `{name}` apart and fused; written by {touching:?}; fired {fired:?}");
        }
    }
}

#[test]
fn every_rule_preserves_its_exemplars_meaning() {
    let mut fired: BTreeMap<&str, usize> = BTreeMap::new();
    for (what, g) in witnesses() {
        let o = optimize(&g, &ReferenceShim, &[]);
        for f in &o.fired {
            *fired.entry(f.rule).or_default() += 1;
        }
        same_meaning(&what, &g, &o.graph, &o.fired);
    }
    for name in rule_names().into_iter().chain([ADVANCE.name]) {
        assert!(fired.get(name).copied().unwrap_or(0) > 0, "`{name}` fires on none of its witnesses, so its meaning is unchecked: {fired:?}");
    }
}

#[test]
fn every_rule_can_be_held() {
    for (what, g) in witnesses() {
        let all = optimize(&g, &ReferenceShim, &[]);
        for name in rule_names() {
            let held = optimize(&g, &ReferenceShim, &[name]);
            assert_eq!(held.fired(name), 0, "{what}: `{name}` held fires");
            if all.fired(name) > 0 {
                same_meaning(&format!("{what} with `{name}` held"), &g, &held.graph, &held.fired);
            }
        }
        let none = optimize(&g, &ReferenceShim, &rule_names());
        assert!(none.fired.iter().all(|f| f.rule == ADVANCE.name), "{what}: every rule held, and {:?} fired", none.fired);
    }
}

/// Every fused class a program can write today, and every placement, is built from its exemplar by the shared table:
/// under the reference shim the optimized exemplar — lifted as a decode step, its matrices as checkpoint rows and as
/// values — holds an op that realizes the class, and means what the exemplar means.
#[test]
fn every_fused_class_with_an_exemplar_is_built_from_it() {
    let mut missing = Vec::new();
    for c in CLASSES.iter().filter(|c| matches!(c.kind, Kind::Fused | Kind::Placement)) {
        let Exemplar::Program { source, .. } = c.exemplar else { continue };
        for rows in [true, false] {
            let g = lifted(source, rows, Some(io(Cut::Decode { ctx: 1 })));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&format!("{} exemplar (rows {rows})", c.id), &g, &o.graph, &o.fired);
            if !o.graph.ops.iter().any(|op| realizes(op, &o.graph).contains(&c.class)) {
                let ops: Vec<String> = o.graph.ops.iter().map(|op| op.kind().to_string()).collect();
                missing.push(format!("{} (rows {rows}): {ops:?}", c.id));
            }
        }
    }
    assert!(missing.is_empty(), "{} exemplars the shared table does not build:\n{}", missing.len(), missing.join("\n"));
}

/// The classes whose pattern is a whole block of a model — no exemplar short of the block holds it — each with the tiny
/// architecture and the cut it is built on.
const BLOCKS: &[(ClassId, tiny_models::Tiny, &str)] = &[
    (ClassId::MoeDeferredFinalize, tiny_models::Tiny::Qwen3Moe, "decode"),
    (ClassId::MoeGatherInOperand, tiny_models::Tiny::Qwen3Moe, "prefill"),
    (ClassId::RouterSharedGateMerge, tiny_models::Tiny::DeepSeek2, "decode"),
    (ClassId::MlaUnabsorbInFold, tiny_models::Tiny::DeepSeek2, "decode"),
    (ClassId::MlaPrologue, tiny_models::Tiny::DeepSeek2, "decode"),
    (ClassId::GdnInputEpilogue, tiny_models::Tiny::Qwen35, "decode"),
    (ClassId::GdnPostConvPrep, tiny_models::Tiny::Qwen35, "prefill"),
    (ClassId::LastLayerRows, tiny_models::Tiny::Qwen3, "prefill"),
    (ClassId::QkNormRopeGateSplit, tiny_models::Tiny::Qwen35, "decode"),
    (ClassId::HorizontalNormRopeInsert, tiny_models::Tiny::Qwen3, "decode"),
];

/// The reference shim, but reading quantized, one word a value, every operand a product by checkpoint rows or a mixture
/// inside the op takes as its activation: the numerics a quantizing chip asks for, with everything linked whole.
struct Quantizing;

/// [`Quantizing`] in a given number of words.
struct QuantizingIn(QuantFormat);

impl Shim for QuantizingIn {
    fn name(&self) -> String {
        format!("reference-quantizing-{:?}", self.0)
    }
    fn supports(&self, c: ClassId) -> Support {
        Quantizing.supports(c)
    }
    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        Quantizing.fits(op, g)
    }
    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        Quantizing.activation(consumer, operand, g).map(|_| self.0)
    }
    fn numerics(&self) -> Option<logicaffeine_compile::tensor_graph::numerics::Contract> {
        ReferenceShim.numerics()
    }
}

impl Shim for Quantizing {
    fn name(&self) -> String {
        "reference-quantizing".to_string()
    }
    fn supports(&self, _: ClassId) -> Support {
        Support::Native
    }
    fn fits(&self, _: &Op, _: &TensorGraph) -> Result<(), String> {
        Ok(())
    }
    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        let t = &consumer.ins[operand];
        let projected = leaves(consumer.clone())
            .iter()
            .any(|l| l.ins.first() == Some(t) && (gemv(l, g).is_some_and(|p| matches!(p.weights, Weights::Rows(_))) || moe(l).is_some()));
        projected.then_some(QuantFormat::OneWord)
    }

    fn numerics(&self) -> Option<logicaffeine_compile::tensor_graph::numerics::Contract> {
        ReferenceShim.numerics()
    }
}

/// Every numerics fold means what its parts mean — a quantized form is its tensor's values — and every quantize a
/// producer can take in its epilogue is taken somewhere on the tiny cuts: a norm's, a sum-then-norm's, a gated pair's or
/// SwiGLU's, a delta rule's, an attention's, a product's.
#[test]
fn every_numerics_fold_means_what_its_parts_mean() {
    let mut realized: BTreeSet<ClassId> = BTreeSet::new();
    for &t in tiny_models::ALL {
        for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
            let ((source, _, _), _) = program::written(&t.spec(24), cut);
            let g = lifted(&source, true, Some(io(cut)));
            let o = optimize(&g, &Quantizing, &[]);
            same_meaning(&format!("{} {cut:?} quantized", t.name()), &g, &o.graph, &o.fired);
            for op in &o.graph.ops {
                realized.extend(realizes(op, &o.graph));
            }
        }
    }
    let folds = [ClassId::NormQuant, ClassId::AddNormQuant, ClassId::GluActMulQuant, ClassId::GdnOutputQuant, ClassId::AttnOutputQuant];
    let missing: Vec<ClassId> = folds.into_iter().filter(|c| !realized.contains(c)).collect();
    assert!(missing.is_empty(), "numerics folds the tiny cuts never take: {missing:?}; realized {realized:?}");
}

/// The numerics fold is a step a compile can hold like any rule: held, no producer takes the quantize of its result into
/// its epilogue — every quantize stands alone, each decision reported held — and the cut means what it meant.
#[test]
fn the_numerics_fold_can_be_held() {
    use logicaffeine_compile::tensor_graph::fuse::{steps, Outcome};
    assert!(steps(&Quantizing).contains(&"fused_quant"), "the fold is among the steps a compile holds");
    let (mut folded, mut held_back) = (0, 0);
    for &t in tiny_models::ALL {
        let cut = Cut::Decode { ctx: 24 };
        let ((source, _, _), _) = program::written(&t.spec(24), cut);
        let g = lifted(&source, true, Some(io(cut)));
        folded += optimize(&g, &Quantizing, &[]).fired("fused_quant");
        let o = optimize(&g, &Quantizing, &["fused_quant"]);
        assert_eq!(o.fired("fused_quant"), 0, "{}: the fold held fires", t.name());
        let quantizes = o.graph.ops.iter().filter(|op| matches!(op.sym, logicaffeine_compile::tensor_graph::graph::Sym::Quantize(_))).count();
        assert!(quantizes > 0, "{}: held, every quantize stands alone", t.name());
        held_back += o.report.decisions.iter().filter(|d| d.rule == "fused_quant" && d.outcome == Outcome::Held).count();
        same_meaning(&format!("{} with the fold held", t.name()), &g, &o.graph, &o.fired);
    }
    assert!(folded > 0, "the fold fires on the tiny cuts unheld");
    assert_eq!(held_back, folded, "every fold the compile holds is reported held");
}

/// What the host reads across steps stays whole: a run of elementwise ops never carries a step's logits (a result the
/// caller reads) or its next token (the state the next step starts from) in its registers, and its twin, the same run
/// through a name the host does not read, does.
#[test]
fn the_rules_keep_what_the_host_reads_across_steps() {
    let io = io(Cut::Decode { ctx: 1 });
    let run = |name: &str, elem: &str, n: usize| {
        let t = |x: &str| format!("Let {x} be a 1 by {n} tensor of {elem}.\n");
        format!("## Main\n{}{}{}Let {name} be A plus B.\nLet Z be {name} hadamard C.\n", t("A"), t("B"), t("C"))
    };
    for (what, carried, elem, n) in [("the logits, a result", "logits", "Real", 8), ("the next token, the state the next step starts from", "next", "Int", 1)] {
        for rows in [true, false] {
            let g = lifted(&run(carried, elem, n), rows, Some(io.clone()));
            let o = optimize(&g, &ReferenceShim, &[]);
            assert_eq!(o.fired("elementwise_chains"), 0, "a run carries {what} in its registers: {:?}", o.fired);
            same_meaning(&format!("a run through {what}"), &g, &o.graph, &o.fired);
            let t = lifted(&run("L", elem, n), rows, Some(io.clone()));
            let ot = optimize(&t, &ReferenceShim, &[]);
            assert!(ot.fired("elementwise_chains") > 0, "the twin of {what}: the run through `L` is carried: {:?}", ot.fired);
            same_meaning(&format!("the twin of {what}"), &t, &ot.graph, &ot.fired);
        }
    }
}

/// A product reads a quantized form of a row it reads in place only where the row starts on a whole block of its table:
/// a row at a whole block is quantized in its table, and the same row read at an offset inside a block is quantized as
/// its own tensor — the table's blocks are not the row's.
#[test]
fn a_view_is_quantized_in_place_only_at_whole_blocks() {
    use logicaffeine_compile::tensor_graph::graph::Sym;
    let src = "## Main\nLet A be a 4 by 64 tensor of Real.\nLet B be a 4 by 64 tensor of Real.\nLet M be a 64 by 32 tensor of Real.\nLet T be A hadamard B.\nLet R be row 2 of T.\nLet Y be R matmul M.\n";
    let quantized = |o: &TensorGraph| -> Vec<String> {
        o.ops.iter().flat_map(|op| leaves(op.clone())).filter(|l| matches!(l.sym, Sym::Quantize(_))).map(|l| l.ins[0].clone()).collect()
    };
    let whole = lifted(src, true, None);
    let o = optimize(&whole, &Quantizing, &[]);
    assert!(o.graph.ops.iter().any(|op| matches!(op.sym, Sym::View { offset: 64 })), "row 2 of a computed table is read in place: {:?}", o.graph.ops.iter().map(|op| op.kind()).collect::<Vec<_>>());
    assert_eq!(quantized(&o.graph), ["T"], "a row at a whole block is quantized in its table");
    same_meaning("a row at a whole block", &whole, &o.graph, &o.fired);
    // The same row read 16 values in: a view the front end never writes, made here.
    let mut inside = whole.clone();
    let at = inside.ops.iter().position(|op| op.out == "R").expect("the row");
    inside.ops[at] = Op { sym: Sym::View { offset: 80 }, ..inside.ops[at].clone() };
    let o = optimize(&inside, &Quantizing, &[]);
    assert_eq!(quantized(&o.graph), ["R"], "a row inside a block is quantized as its own tensor");
    same_meaning("a row inside a block", &inside, &o.graph, &o.fired);
}

// ── The shared side of the parity fusions (benchmarks/FUSIONS_WE_NEED.md, section 12.2) ────────────────

/// The op of `g` that realizes `class` and covers a lifted op of each kind in `kinds`, each counted as often as named.
fn folding<'a>(g: &'a TensorGraph, class: ClassId, kinds: &[&str]) -> Option<&'a Op> {
    g.ops.iter().find(|op| {
        let parts: Vec<String> = leaves((*op).clone()).iter().map(|l| l.kind().to_string()).collect();
        realizes(op, g).contains(&class) && kinds.iter().all(|k| parts.iter().filter(|p| p == k).count() >= kinds.iter().filter(|x| x == &k).count())
    })
}

/// A projection whose result is one summand of a residual add and the norm of the sum folds them into its epilogue
/// (PARITY-GEMV-NORM-EPILOGUE): the projection, the add, the written sum and the next norm as one op, meaning what
/// they meant, at rows of 1024 to 5120; a sum another op reads later is written on; Gemma's post-norm before the add
/// folds too, both norms in the one op; the last layer's next norm is the final norm; and under a quantizing chip the
/// norm's quantize joins the same epilogue.
#[test]
fn a_projection_feeding_a_residual_and_a_norm_folds_them() {
    let decl = |t: &str, r: usize, c: usize| format!("Let {t} be a {r} by {c} tensor of Real.\n");
    let base = |d: usize| format!("## Main\n{}{}{}{}{}Let P be X matmul W.\n", decl("X", 1, 64), decl("W", 64, d), decl("R", 1, d), decl("N", 1, d), decl("M", 1, d));
    let fold = |src: &str, what: &str, kinds: &[&str]| -> (TensorGraph, TensorGraph) {
        let g = lifted(src, true, None);
        let o = optimize(&g, &ReferenceShim, &[]);
        same_meaning(what, &g, &o.graph, &o.fired);
        assert!(
            folding(&o.graph, ClassId::GemvNormEpilogue, kinds).is_some(),
            "{what}: no op folds {kinds:?} into the projection: {:?}",
            o.graph.ops.iter().map(|op| op.kind()).collect::<Vec<_>>()
        );
        (g, o.graph)
    };
    for d in [1024, 2048, 4096, 5120] {
        fold(&format!("{}Let S be R plus P.\nLet Y be rmsnorm of S with weight N.\n", base(d)), &format!("rows of {d}"), &["Matmul", "Elementwise", "FusedRow"]);
    }
    let (_, o) = fold(&format!("{}Let S be R plus P.\nLet Y be rmsnorm of S with weight N.\nLet Z be S hadamard R.\n", base(64)), "a sum read later", &["Matmul", "Elementwise", "FusedRow"]);
    let op = folding(&o, ClassId::GemvNormEpilogue, &["Matmul"]).expect("the fold");
    assert!(op.keeps.iter().any(|t| t == "S"), "the written sum outlives the fold for its later reader: keeps {:?}", op.keeps);
    fold(
        &format!("{}Let Q be rmsnorm of P with weight M.\nLet S be R plus Q.\nLet Y be rmsnorm of S with weight N.\n", base(64)),
        "Gemma's post-norm before the add",
        &["Matmul", "FusedRow", "FusedRow"],
    );
    // The last layer's next norm is the final norm: every layer's output and down projections fold, the last one's into
    // the step's final norm.
    for &t in [tiny_models::Tiny::Qwen3, tiny_models::Tiny::Llama, tiny_models::Tiny::Gemma3].iter() {
        let spec = t.spec(24);
        let ((source, _, _), _) = program::written(&spec, Cut::Decode { ctx: 24 });
        let g = lifted(&source, true, Some(io(Cut::Decode { ctx: 24 })));
        let o = optimize(&g, &ReferenceShim, &[]);
        let folds = o.graph.ops.iter().filter(|op| realizes(op, &o.graph).contains(&ClassId::GemvNormEpilogue)).count();
        assert_eq!(folds, 2 * spec.layers.len(), "{}: an output and a down projection a layer fold, the last into the final norm", t.name());
        same_meaning(&format!("{} decode", t.name()), &g, &o.graph, &o.fired);
    }
    // Under a quantizing chip, in one word or two, the norm's result, read by an int8 product, is quantized in the
    // projection's epilogue.
    let src = format!("{}Let S be R plus P.\nLet Y be rmsnorm of S with weight N.\n{}Let Z be Y matmul V.\n", base(64), decl("V", 64, 32));
    for format in [QuantFormat::OneWord, QuantFormat::TwoWords] {
        let g = lifted(&src, true, None);
        let o = optimize(&g, &QuantizingIn(format), &[]);
        same_meaning(&format!("the quantize in the fold, {format:?}"), &g, &o.graph, &o.fired);
        let op = folding(&o.graph, ClassId::GemvNormEpilogue, &["Matmul", "FusedRow", "Quantize"]);
        assert!(op.is_some(), "{format:?}: the norm's quantize joins the projection's epilogue: {:?}", o.graph.ops.iter().map(|op| op.kind()).collect::<Vec<_>>());
    }
}

/// `src` lifted, its matrices as values, and optimized under the reference shim, meaning what it meant.
fn reference(src: &str, what: &str) -> (TensorGraph, TensorGraph) {
    let g = lifted(src, false, None);
    let o = optimize(&g, &ReferenceShim, &[]);
    same_meaning(what, &g, &o.graph, &o.fired);
    (g, o.graph)
}

fn kinds(g: &TensorGraph) -> Vec<String> {
    g.ops.iter().map(|op| op.kind().to_string()).collect()
}

/// An RMS norm followed by a scale folds the scale into the norm (PARITY-NORM-SCALE), meaning what the two meant: a
/// weighted norm, a scale of 1 (whose fold computes the norm itself, bit for bit), a negative scale, and a norm with no
/// weight.
#[test]
fn a_norm_feeding_a_scale_fuses() {
    use logicaffeine_compile::tensor_graph::eval::Data;
    let decl = "## Main\nLet X be a 1 by 64 tensor of Real.\nLet W be a 1 by 64 tensor of Real.\n";
    for (what, norm, scale) in [
        ("a weighted norm", "rmsnorm of X with weight W", "0.5"),
        ("a scale of 1", "rmsnorm of X with weight W", "1.0"),
        ("a negative scale", "rmsnorm of X with weight W", "-0.25"),
        ("a norm with no weight", "rmsnorm of X", "0.5"),
    ] {
        let src = format!("{decl}Let N be {norm}.\nLet Y be N scaled by {scale}.\n");
        let (g, o) = reference(&src, what);
        assert!(folding(&o, ClassId::NormScale, &["FusedRow", "Scale"]).is_some(), "{what}: the scale is not in the norm: {:?}", kinds(&o));
        if scale == "1.0" {
            let ins = inputs(&g);
            let (apart, fused) = (eval(&g, &ins).unwrap(), eval(&o, &ins).unwrap());
            let bits = |d: &Data| match d {
                Data::Real(v) => v.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                Data::Int(_) => panic!("a norm through integers"),
            };
            assert_eq!(bits(&fused["Y"]), bits(&apart["N"]), "a scale of 1 folds to the norm itself");
        }
    }
}

/// A product followed by a scale folds it into the product's epilogue (PARITY-GEMM-SCALE), on a step's one row and a
/// chunk's rows: a scalar scale; each column its own factor, a constant row; each element its own, a constant of the
/// product's shape (a row's own factor with it); a bias before the scale, and one after it.
#[test]
fn a_gemm_feeding_a_scale_fuses() {
    let decl = |m: usize| format!("## Main\nLet X be a {m} by 64 tensor of Real.\nLet W be a 64 by 32 tensor of Real.\nLet C be a 1 by 32 tensor of Real.\nLet F be a {m} by 32 tensor of Real.\nLet B be a 1 by 32 tensor of Real.\nLet P be X matmul W.\n");
    for m in [1, 4] {
        for (what, tail, parts) in [
            ("a scalar scale", "Let Y be P scaled by 0.125.\n", &["Matmul", "Scale"][..]),
            ("a column's own factor", "Let Y be P hadamard C.\n", &["Matmul", "Elementwise"][..]),
            ("each element its own factor", "Let Y be P hadamard F.\n", &["Matmul", "Elementwise"][..]),
            ("a bias before the scale", "Let Q be P plus B.\nLet Y be Q scaled by 0.125.\n", &["Matmul", "Elementwise", "Scale"][..]),
            ("a bias after the scale", "Let Q be P scaled by 0.125.\nLet Y be Q plus B.\n", &["Matmul", "Scale", "Elementwise"][..]),
        ] {
            let what = format!("{what}, {m} row(s)");
            let (_, o) = reference(&format!("{}{tail}", decl(m)), &what);
            assert!(folding(&o, ClassId::GemmScale, parts).is_some(), "{what}: not in the product's epilogue: {:?}", kinds(&o));
        }
    }
}

/// An embedding row gathered and normed in one op (PARITY-EMBED-NORM), the raw row written on for the residual: a step's
/// token, a chunk's rows with the same token twice, a table in a block format, a tied table the head reads too, and a
/// model with no embedding scale beside one with Gemma's.
#[test]
fn a_gather_feeding_a_norm_fuses_into_one_op() {
    let table = "## Main\nLet E be a 16 by 64 tensor of Real.\nLet W be a 1 by 64 tensor of Real.\nLet H be a 64 by 16 tensor of Real.\nLet Z be a 1 by 64 tensor of Real.\n";
    // The norm feeds the first projection; the raw row is the residual a later add reads.
    let tail = "Let N be rmsnorm of X with weight W.\nLet L be N matmul H.\nLet R be X plus Z.\n";
    let step = format!("{table}Let tok be 3.\nLet X be row tok of E.\n{tail}");
    let chunk = format!("{table}Let T be a 1 by 4 tensor of Int.\nSet item 1 of T to 3.\nSet item 2 of T to 7.\nSet item 3 of T to 3.\nSet item 4 of T to 1.\nLet X be rows T of E.\nLet N be rmsnorm of X with weight W.\nLet L be N matmul H.\nLet Y be a 4 by 64 tensor of Real.\nLet R be X plus Y.\n");
    let scaled = format!("{table}Let tok be 3.\nLet G be row tok of E.\nLet X be G scaled by 8.0.\n{tail}");
    let tied = format!("{table}Let tok be 3.\nLet X be row tok of E.\n{tail}Let O be a 64 by 1 tensor of Real.\nLet K be E matmul O.\n");
    for (what, src) in [("a step's token", &step), ("a chunk's rows, one token twice", &chunk), ("Gemma's embedding scale", &scaled), ("a tied table", &tied)] {
        for block in [true, false] {
            let what = format!("{what} (table in Q8_0: {block})");
            let rp = recognize_stages(src).unwrap_or_else(|e| panic!("{e:?}"));
            let storage = |t: &str| if block && t == "E" { Storage::Rows { format: logicaffeine_compile::model::quant::GgmlType::Q8_0, transposed: false } } else { Storage::Values };
            let g = TensorGraph::lift(&rp, None, None, &storage).unwrap_or_else(|e| panic!("{what}: {e}"));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let op = o.graph.ops.iter().find(|op| realizes(op, &o.graph).contains(&ClassId::EmbedNorm)).unwrap_or_else(|| panic!("{what}: the gather and the norm are apart: {:?}", kinds(&o.graph)));
            let parts: Vec<String> = leaves(op.clone()).iter().map(|l| l.kind().to_string()).collect();
            assert!(parts.iter().any(|k| k.contains("Gather")) && parts.iter().any(|k| k == "FusedRow"), "{what}: the fold holds the gather and the norm: {parts:?}");
            assert!(op.keeps.iter().any(|t| t == "X"), "{what}: the raw row is written on for the residual: keeps {:?}", op.keeps);
        }
    }
}

/// A norm whose whole result is one summand of an add folds the add into its row op (PARITY-NORM-THEN-ADD), meaning what
/// the two meant: the add's operands in either order, a norm with no weight, the sum written back only when a later op
/// reads it, a post-norm another op reads kept, and the next norm after the sum folded in too, quantizing under a
/// quantizing chip.
#[test]
fn a_norm_feeding_an_add_fuses_into_one_row_op() {
    let decl = "## Main\nLet P be a 1 by 64 tensor of Real.\nLet R be a 1 by 64 tensor of Real.\nLet M be a 1 by 64 tensor of Real.\nLet V be a 1 by 64 tensor of Real.\n";
    for (what, body) in [
        ("the residual first", "Let Q be rmsnorm of P with weight M.\nLet S be R plus Q.\n"),
        ("the residual second", "Let Q be rmsnorm of P with weight M.\nLet S be Q plus R.\n"),
        ("a norm with no weight", "Let Q be rmsnorm of P.\nLet S be R plus Q.\n"),
        ("a sum a later op reads", "Let Q be rmsnorm of P with weight M.\nLet S be R plus Q.\nLet Z be S hadamard R.\n"),
    ] {
        let (_, o) = reference(&format!("{decl}{body}"), what);
        assert!(folding(&o, ClassId::NormThenAdd, &["FusedRow", "Elementwise"]).is_some(), "{what}: the norm and its add are apart: {:?}", kinds(&o));
    }
    let (_, o) = reference(&format!("{decl}Let Q be rmsnorm of P with weight M.\nLet S be R plus Q.\nLet Z be Q hadamard R.\n"), "a post-norm another op reads after the add");
    let op = folding(&o, ClassId::NormThenAdd, &["FusedRow", "Elementwise"]).unwrap_or_else(|| panic!("a post-norm read again after the add folds: {:?}", kinds(&o)));
    assert!(op.keeps.iter().any(|t| t == "Q"), "the post-norm is written on for its later reader: keeps {:?}", op.keeps);
    let (_, o) = reference(&format!("{decl}Let Q be rmsnorm of P with weight M.\nLet Z be Q hadamard R.\nLet S be R plus Q.\n"), "a post-norm read before the add");
    assert!(folding(&o, ClassId::NormThenAdd, &["FusedRow", "Elementwise"]).is_none(), "a post-norm another op reads before the add is not moved past it: {:?}", kinds(&o));
    // The next norm after the sum joins the fold; the sum it norms is written back only where a later op reads it.
    let next = format!("{decl}Let Q be rmsnorm of P with weight M.\nLet S be R plus Q.\nLet Y be rmsnorm of S with weight V.\nLet U be a 64 by 32 tensor of Real.\nLet Z be Y matmul U.\n");
    for (read, src) in [(false, next.clone()), (true, format!("{next}Let K be S hadamard R.\n"))] {
        let (_, o) = reference(&src, &format!("the next norm after the sum, the sum read later: {read}"));
        let op = folding(&o, ClassId::NormThenAdd, &["FusedRow", "FusedRow"]).unwrap_or_else(|| panic!("the next norm joins the fold: {:?}", kinds(&o)));
        assert_eq!(op.keeps.iter().any(|t| t == "S"), read, "the sum is written back exactly when a later op reads it: keeps {:?}", op.keeps);
    }
    for format in [QuantFormat::OneWord, QuantFormat::TwoWords] {
        let g = lifted(&next, true, None);
        let o = optimize(&g, &QuantizingIn(format), &[]);
        same_meaning(&format!("the next norm quantizing, {format:?}"), &g, &o.graph, &o.fired);
        assert!(folding(&o.graph, ClassId::NormThenAdd, &["FusedRow", "FusedRow", "Quantize"]).is_some(), "{format:?}: the next norm's quantize joins the fold: {:?}", kinds(&o.graph));
    }
}

/// The projection a rotation's input comes from: back through the per-head ops between them — a per-head norm, a bias
/// added to its every row, a slice of each head — to the product, each read by nothing but the next.
fn projection_of<'a>(g: &'a TensorGraph, rope: &Op) -> Option<&'a Op> {
    let mut x = rope.ins[0].clone();
    loop {
        let producer = g.ops.iter().find(|op| op.out == x)?;
        if producer.kind() == "Matmul" {
            return Some(producer);
        }
        let through = matches!(producer.kind(), "FusedRow" | "HeadSlice" | "Elementwise") && g.ops.iter().filter(|op| op.ins.contains(&x)).count() == 1;
        if !through {
            return None;
        }
        x = producer.ins[0].clone();
    }
}

/// A decode layer's query, key and value come from one op that applies each head's norm, rotation and cache write in
/// its epilogue (PARITY-QKV-HEAD-EPILOGUE: Gemma's five launches and Llama's five become one): on every tiny
/// architecture whose attention rotates its queries and keys, and reshaped to 4:1, 8:1, 3:1 and 4:1 query to key heads
/// 256, 128, 64 and 64 wide — Qwen3.5's partial rotation of a query read beside its gate, gpt-oss's biased projections,
/// keys written into the cache — every rotation is computed by the op that computes its projection, and that op holds
/// the layer's value projection too.
#[test]
fn a_projection_feeding_per_head_norms_and_rotations_folds_them_into_its_epilogue() {
    let reshaped = |t: tiny_models::Tiny, heads: usize, kv_heads: usize, hd: usize| {
        let mut spec = t.spec(24);
        for layer in &mut spec.layers {
            if let logicaffeine_compile::model::spec::Mixer::Attention(a) = &mut layer.mixer {
                (a.heads, a.kv_heads, a.head_dim, a.value_dim, a.scale) = (heads, kv_heads, hd, hd, 1.0 / (hd as f64).sqrt());
                if let Some(r) = &mut a.rope {
                    r.dims = hd;
                }
            }
        }
        (format!("{} with {heads} query heads over {kv_heads}, {hd} wide", t.name()), spec)
    };
    let mut specs: Vec<(String, logicaffeine_compile::model::spec::ModelSpec)> = tiny_models::ALL
        .iter()
        .filter(|t| !t.name().starts_with("deepseek2"))
        .map(|&t| (t.name().to_string(), t.spec(24)))
        .collect();
    specs.extend([
        reshaped(tiny_models::Tiny::Qwen3, 4, 1, 256),
        reshaped(tiny_models::Tiny::Llama, 8, 1, 128),
        reshaped(tiny_models::Tiny::Gemma3, 3, 1, 64),
        reshaped(tiny_models::Tiny::Qwen3, 8, 2, 64),
    ]);
    for (what, spec) in specs {
        let ((source, _, _), _) = program::written(&spec, Cut::Decode { ctx: 24 });
        let g = lifted(&source, true, Some(io(Cut::Decode { ctx: 24 })));
        let o = optimize(&g, &ReferenceShim, &[]);
        same_meaning(&format!("{what} decode"), &g, &o.graph, &o.fired);
        let holder = |t: &str| o.graph.ops.iter().find(|op| leaves((*op).clone()).iter().any(|l| l.out == t));
        let mut folded = 0;
        for rope in g.ops.iter().filter(|op| op.kind() == "Rope") {
            let Some(projection) = projection_of(&g, rope) else { continue };
            let op = holder(&rope.out).unwrap_or_else(|| panic!("{what}: no op computes `{}`", rope.out));
            let held: Vec<String> = leaves(op.clone()).iter().map(|l| l.out.clone()).collect();
            assert!(
                held.contains(&projection.out) && realizes(op, &o.graph).contains(&ClassId::QkvHeadEpilogue),
                "{what}: the rotation `{}` is not in its projection `{}`'s epilogue: it is computed by a `{}` holding {held:?}",
                rope.out,
                projection.out,
                op.kind()
            );
            let values = g.ops.iter().filter(|p| p.kind() == "Matmul" && p.ins[0] == projection.ins[0] && !held.contains(&p.out)).count();
            assert_eq!(values, 0, "{what}: a projection of `{}` beside `{}` is a launch of its own", projection.ins[0], projection.out);
            assert!(realizes(op, &o.graph).contains(&ClassId::MergedQkv), "{what}: `{}`'s query, key and value are not one op", projection.ins[0]);
            folded += 1;
        }
        let rotating = spec.layers.iter().filter(|l| matches!(&l.mixer, logicaffeine_compile::model::spec::Mixer::Attention(a) if a.rope.is_some())).count();
        assert!(folded >= 2 * rotating, "{what}: {folded} rotations folded, where {rotating} rotating layers turn a query and a key each");
    }
}

/// A merged projection is one product over one activation: every product it holds, at every depth — its members' and
/// their epilogues' — reads the one activation at the one depth, on every witness and every tiny cut.
#[test]
fn a_merged_projection_reads_one_activation() {
    let mut merged = 0;
    for (what, g) in witnesses() {
        let o = optimize(&g, &ReferenceShim, &[]);
        for op in o.graph.ops.iter().flat_map(|op| {
            fn nested(op: &Op) -> Vec<&Op> {
                match &op.sym {
                    logicaffeine_compile::tensor_graph::graph::Sym::Fused(f) => std::iter::once(op).chain(f.parts.iter().flat_map(nested)).collect(),
                    _ => Vec::new(),
                }
            }
            nested(op)
        }) {
            if !matches!(&op.sym, logicaffeine_compile::tensor_graph::graph::Sym::Fused(f) if f.class == ClassId::MergedQkv) {
                continue;
            }
            merged += 1;
            let products: Vec<(String, usize, usize)> = leaves(op.clone()).iter().filter_map(|l| gemv(l, &o.graph)).map(|p| (p.x, p.k, p.m)).collect();
            assert!(
                products.windows(2).all(|w| w[0] == w[1]) && products.first().map(|p| &p.0) == op.ins.first(),
                "{what}: a merged projection reads more than one activation: {products:?}"
            );
        }
    }
    assert!(merged > 0, "no witness merges its projections");
}

/// A gated delta layer of `rows` rows, `nk` key heads and `nv` value heads `w` wide: its query's, key's and values'
/// convolutions with their SiLU and L2 norms, its two gates and the rule — read from projections of one activation `X`
/// where `projected`, from inputs of their own otherwise — and, with `out`, the output projection that reads its result.
fn delta_layer(nk: usize, nv: usize, w: usize, rows: usize, projected: bool, out: bool) -> String {
    let decl = |t: &str, r: usize, c: usize| format!("Let {t} be a {r} by {c} tensor of Real.\n");
    let (qk, vw) = (nk * w, nv * w);
    let made = [("XQ", qk), ("XK", qk), ("XV", vw), ("Z", vw), ("BR", nv), ("AR", nv)];
    let mut s = String::from("## Main\n");
    if projected {
        s += &decl("X", rows, 64);
        for (t, c) in made {
            s += &decl(&format!("P{t}"), 64, c);
        }
    } else {
        for (t, c) in made {
            s += &decl(t, rows, c);
        }
    }
    for (t, r, c) in [("WQ", 4, qk), ("WK", 4, qk), ("WV", 4, vw), ("CQ", 4, qk), ("CK", 4, qk), ("CV", 4, vw), ("S", vw, w), ("G", 1, w), ("DT", 1, nv), ("AL", 1, nv)] {
        s += &decl(t, r, c);
    }
    let project = |t: &str| if projected { format!("Let {t} be X matmul P{t}.\n") } else { String::new() };
    let heads = rows * nk;
    s += &format!(
        "{}{}{}{}Let YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\n\
         Let YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\n\
         Let Q be l2norm of SQ across {heads} heads.\nLet K be l2norm of SK across {heads} heads.\n{}Let BT be sigmoid of BR.\n\
         {}Let DY be decay of AR with bias DT rate AL.\n\
         Let O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with {nv} heads {nk} groups.\n",
        project("XQ"),
        project("XK"),
        project("XV"),
        project("Z"),
        project("BR"),
        project("AR"),
    );
    if out {
        s += &format!("{}Let Y be O matmul WO.\n", decl("WO", vw, 64));
    }
    s
}

/// `g` and its optimized `o` run `steps` times from a zero `carried` state, each step from the state the last one left
/// and each with `fresh` inputs of its own, agreeing on every value at every step: the memory `S` the run leaves.
fn carried_run(g: &TensorGraph, o: &TensorGraph, carried: &[&str], fresh: &[&str], steps: usize) -> Vec<f64> {
    use logicaffeine_compile::tensor_graph::eval::Data;
    let mut ins = inputs(g);
    for &t in carried {
        ins.insert(t.to_string(), Data::Real(vec![0.0; g.tensors[t].len()]));
    }
    let (mut apart, mut fused) = (ins.clone(), ins);
    for step in 0..steps {
        for &t in fresh {
            let v = Data::Real((0..g.tensors[t].len()).map(|k| 4.0 * value(&format!("{t}{step}"), k)).collect());
            apart.insert(t.to_string(), v.clone());
            fused.insert(t.to_string(), v);
        }
        let a = eval(g, &apart).unwrap_or_else(|e| panic!("step {step} apart: {e}"));
        let f = eval(o, &fused).unwrap_or_else(|e| panic!("step {step} fused: {e}"));
        for (name, v) in &f {
            if let Some(x) = a.get(name) {
                assert_eq!(x, v, "step {step}: `{name}` apart and fused");
            }
        }
        for &t in carried {
            apart.insert(t.to_string(), a[t].clone());
            fused.insert(t.to_string(), f[t].clone());
        }
    }
    let Data::Real(memory) = &apart["S"] else { panic!("the memory is real") };
    assert!(memory.iter().all(|x| x.is_finite()) && memory.iter().any(|x| *x != 0.0), "the run leaves a memory: {:?}", &memory[..8]);
    memory.clone()
}

/// A one-row gated delta rule's whole step between its projections is one op (PARITY-GDN-DECODE-WHOLE): its query's,
/// key's and values' convolutions with their SiLU and L2 norms, its two gates, the update of its state and its gated
/// norm, meaning what they mean — with as many value heads as key heads and more, at state widths of 32 to 256; from a
/// zero state and over a run of steps, each from the memory and rings the last one left; with its result's quantize in
/// the op's epilogue in one word and in two. On Qwen3.5's decode, its input epilogue held, every layer's step is that
/// op, and on its batch, each sequence's step one row, every sequence's; a prefill chunk's rule, many rows at once, is
/// not.
#[test]
fn a_delta_step_with_its_conv_gates_norms_and_output_fuses_into_one_op() {
    let whole = ["CausalConv", "CausalConv", "CausalConv", "Silu", "Silu", "Silu", "L2Norm", "L2Norm", "Sigmoid", "Decay", "GatedDelta"];
    // Between the projections, the step is the one op.
    let one_op = |o: &TensorGraph, what: &str| {
        assert!(folding(o, ClassId::GdnDecodeWhole, &whole).is_some(), "{what}: no op is the whole step: {:?}", kinds(o));
        let apart: Vec<String> = o.ops.iter().filter(|op| !realizes(op, o).contains(&ClassId::GdnDecodeWhole) && gemv(op, o).is_none()).map(|op| op.kind().to_string()).collect();
        assert!(apart.is_empty(), "{what}: the step runs beside {apart:?}");
    };
    for (nk, nv, w) in [(2, 2, 64), (2, 4, 32), (1, 4, 64), (2, 4, 128), (1, 2, 256), (2, 6, 64)] {
        let what = format!("{nk} key heads, {nv} value heads, {w} wide");
        for out in [false, true] {
            let (_, o) = reference(&delta_layer(nk, nv, w, 1, false, out), &what);
            one_op(&o, &what);
        }
    }
    // A run of steps from a zero state, each from the memory and the rings the last one left, its own inputs each step:
    // the op and its parts agree on every value at every step.
    let g = lifted(&delta_layer(2, 4, 64, 1, false, false), false, None);
    let o = optimize(&g, &ReferenceShim, &[]);
    one_op(&o.graph, "the run");
    carried_run(&g, &o.graph, &["S", "CQ", "CK", "CV"], &["XQ", "XK", "XV", "Z", "BR", "AR"], 64);
    // Under a quantizing chip, in one word or two, the result the output projection reads is quantized in the step's
    // epilogue.
    let src = delta_layer(2, 4, 32, 1, false, true);
    for format in [QuantFormat::OneWord, QuantFormat::TwoWords] {
        let g = lifted(&src, true, None);
        let o = optimize(&g, &QuantizingIn(format), &[]);
        same_meaning(&format!("the step's quantize, {format:?}"), &g, &o.graph, &o.fired);
        let quantized: Vec<&str> = whole.iter().copied().chain(["Quantize"]).collect();
        let op = folding(&o.graph, ClassId::GdnDecodeWhole, &quantized).unwrap_or_else(|| panic!("{format:?}: the quantize is not in the step's epilogue: {:?}", kinds(&o.graph)));
        assert!(realizes(op, &o.graph).contains(&ClassId::GdnOutputQuant), "{format:?}: the step's epilogue quantize is a `gdn_output_quant`");
    }
    // Qwen3.5's decode with its input epilogue held, and its batch, whose products read every sequence's row: every
    // layer's step, each sequence's, is one op. A prefill chunk's rule runs many rows, and its step is not.
    let t = tiny_models::Tiny::Qwen35;
    for (cut, held) in [(Cut::Decode { ctx: 24 }, Some("gdn_input_epilogue")), (Cut::Batch { rows: 2, ctx: 24 }, None), (Cut::Prefill { rows: 4, ctx: 24 }, Some("gdn_post_conv_prep"))] {
        let ((source, _, _), _) = program::written(&t.spec(24), cut);
        for rows in [true, false] {
            let what = format!("{} {cut:?} (rows {rows}), {held:?} held", t.name());
            let g = lifted(&source, rows, Some(io(cut)));
            let o = optimize(&g, &ReferenceShim, held.as_slice());
            same_meaning(&what, &g, &o.graph, &o.fired);
            let deltas = g.ops.iter().filter(|op| op.kind() == "GatedDelta").count();
            let steps = o
                .graph
                .ops
                .iter()
                .filter(|op| {
                    let parts: Vec<String> = leaves((*op).clone()).iter().map(|l| l.kind().to_string()).collect();
                    realizes(op, &o.graph).contains(&ClassId::GdnDecodeWhole) && whole.iter().all(|k| parts.iter().any(|p| p == k))
                })
                .count();
            assert!(deltas > 0, "{what}: no gated delta layer");
            let expected = if matches!(cut, Cut::Prefill { .. }) { 0 } else { deltas };
            assert_eq!(steps, expected, "{what}: {steps} whole steps of {deltas} delta rules: {:?}", o.fired);
        }
    }
}

/// A one-row gated delta layer's every input projection — query, key, values, output gate and both gates — is one op
/// over its one activation with the convolutions, SiLU, L2 norms and gates in its epilogue (PARITY-GDN-INPUT-EPILOGUE),
/// meaning what they mean: with as many value heads as key heads and more (Qwen3.8's three to one), its gates projected
/// in f32 beside int8 projections (Qwen3.8's alpha and beta), from a zero history over a run of steps carrying the rings
/// and the memory, and on Qwen3.5's decode as it is and reshaped to Qwen3.8's heads and gates, every layer. A prefill
/// chunk's convolutions and gates are one op past its projections, the rings and the memory carried from chunk to chunk.
#[test]
fn a_projection_feeding_a_causal_conv_and_gates_folds_them_per_row() {
    use logicaffeine_compile::model::quant::GgmlType;
    let epilogue = ["Matmul", "Matmul", "Matmul", "Matmul", "Matmul", "Matmul", "CausalConv", "CausalConv", "CausalConv", "Silu", "Silu", "Silu", "L2Norm", "L2Norm", "Sigmoid", "Decay"];
    // Every input of each rule made by one op, whose six products read one activation — and, for a layer `alone`,
    // beside them only the rule and the product of its result.
    let folded = |g: &TensorGraph, o: &TensorGraph, what: &str, alone: bool| {
        let deltas: Vec<&Op> = g.ops.iter().filter(|op| op.kind() == "GatedDelta").collect();
        assert!(!deltas.is_empty(), "{what}: no gated delta rule");
        for delta in deltas {
            let op = o
                .ops
                .iter()
                .find(|op| leaves((*op).clone()).iter().any(|l| l.out == delta.ins[0]))
                .unwrap_or_else(|| panic!("{what}: no op computes `{}`", delta.ins[0]));
            let held: Vec<String> = leaves(op.clone()).iter().map(|l| l.out.clone()).collect();
            let parts: Vec<String> = leaves(op.clone()).iter().map(|l| l.kind().to_string()).collect();
            assert!(
                realizes(op, o).contains(&ClassId::GdnInputEpilogue) && epilogue.iter().all(|k| parts.iter().filter(|p| p == k).count() >= epilogue.iter().filter(|x| x == &k).count()),
                "{what}: `{}`'s inputs are made by a `{}` of {parts:?}",
                delta.out,
                op.kind()
            );
            for t in delta.ins[0..6].iter() {
                assert!(held.contains(t), "{what}: `{t}`, read by `{}`, is not made in the epilogue", delta.out);
            }
            let products: BTreeSet<String> = leaves(op.clone()).iter().filter_map(|l| gemv(l, o)).map(|p| p.x).collect();
            assert_eq!(products.len(), 1, "{what}: the epilogue's products read {products:?}");
        }
        if alone {
            let apart: Vec<String> = o
                .ops
                .iter()
                .filter(|op| !realizes(op, o).contains(&ClassId::GdnInputEpilogue) && !matches!(op.kind(), "GatedDelta" | "Matmul"))
                .map(|op| op.kind().to_string())
                .collect();
            assert!(apart.is_empty(), "{what}: the inputs run beside {apart:?}");
        }
    };
    let q8 = Storage::Rows { format: GgmlType::Q8_0, transposed: true };
    for (nk, nv, w) in [(2, 2, 64), (2, 4, 32), (2, 6, 32), (1, 4, 128)] {
        for out in [false, true] {
            for rows in [true, false] {
                let what = format!("{nk} key heads, {nv} value heads, {w} wide (rows {rows}, out {out})");
                let g = lifted(&delta_layer(nk, nv, w, 1, true, out), rows, None);
                let o = optimize(&g, &ReferenceShim, &[]);
                same_meaning(&what, &g, &o.graph, &o.fired);
                folded(&g, &o.graph, &what, true);
            }
        }
    }
    // Qwen3.8's gates project in f32 beside the int8 projections.
    let src = delta_layer(2, 6, 32, 1, true, true);
    let rp = recognize_stages(&src).unwrap_or_else(|e| panic!("{e:?}"));
    let g = TensorGraph::lift(&rp, None, None, &|t| match t {
        "PBR" | "PAR" => Storage::Values,
        t if t.starts_with('P') || t == "WO" => q8.clone(),
        _ => Storage::Values,
    })
    .unwrap();
    let o = optimize(&g, &ReferenceShim, &[]);
    same_meaning("f32 gates", &g, &o.graph, &o.fired);
    folded(&g, &o.graph, "f32 gates", true);
    // From a zero history, the rings and the memory carried step to step.
    let g = lifted(&delta_layer(2, 4, 32, 1, true, false), true, None);
    let o = optimize(&g, &ReferenceShim, &[]);
    folded(&g, &o.graph, "a run from a zero history", true);
    carried_run(&g, &o.graph, &["S", "CQ", "CK", "CV"], &["X"], 32);
    // Qwen3.5's decode, and reshaped to Qwen3.8's three value heads a key head with its gates in f32.
    let mut wide = tiny_models::Tiny::Qwen35.spec(24);
    for layer in &mut wide.layers {
        if let logicaffeine_compile::model::spec::Mixer::GatedDelta(d) = &mut layer.mixer {
            (d.k_heads, d.v_heads) = (2, 6);
        }
    }
    let cut = Cut::Decode { ctx: 24 };
    for (what, spec, f32_gates) in [("Qwen3.5 decode", tiny_models::Tiny::Qwen35.spec(24), false), ("Qwen3.5 decode at Qwen3.8's heads and gates", wide, true)] {
        let ((source, _, _), _) = program::written(&spec, cut);
        let rp = recognize_stages(&source).unwrap_or_else(|e| panic!("{e:?}"));
        let plain = TensorGraph::lift(&rp, None, Some(io(cut)), &|_| Storage::Values).unwrap();
        let m = fusion_corpus::matrices(&plain);
        let g = TensorGraph::lift(&rp, None, Some(io(cut)), &|t| {
            let gate = t.starts_with("Wb") || t.starts_with("Wa");
            if m.contains(t) && !(f32_gates && gate) { q8.clone() } else { Storage::Values }
        })
        .unwrap();
        let o = optimize(&g, &ReferenceShim, &[]);
        same_meaning(what, &g, &o.graph, &o.fired);
        folded(&g, &o.graph, what, false);
    }
    // A prefill chunk: the convolutions and gates are one op past the projections, the rings and the memory carried
    // from chunk to chunk.
    let g = lifted(&delta_layer(2, 4, 32, 4, true, false), true, None);
    let o = optimize(&g, &ReferenceShim, &[]);
    assert_eq!(o.fired("gdn_input_epilogue"), 0, "a chunk's projections are products of many rows: {:?}", o.fired);
    assert!(
        folding(&o.graph, ClassId::GdnPostConvPrep, &["CausalConv", "CausalConv", "CausalConv", "Sigmoid", "Decay"]).is_some(),
        "a chunk's convolutions and gates are not one op: {:?}",
        kinds(&o.graph)
    );
    carried_run(&g, &o.graph, &["S", "CQ", "CK", "CV"], &["X"], 3);
}

/// A one-row gated delta layer's input epilogue takes its gates' products whether or not they fused with their gates
/// first (PARITY-GDN-INPUT-EPILOGUE): with `gate_projections` held — a chip that runs a gate's projection only inside
/// the epilogue, as the CUDA lane runs Q8_0 gates — the op that makes the rule's inputs still holds all six products
/// of its one activation, the write strength's and the forget gate's beside the query's, key's, values' and output
/// gate's, nothing of the layer but the rule and the product of its result running beside it, and means what its parts
/// mean, through a run of steps from a zero history.
#[test]
fn a_gated_delta_layers_input_epilogue_takes_its_gates_products_where_they_stay_apart() {
    let products = ["PXQ", "PXK", "PXV", "PZ", "PBR", "PAR"];
    for (nk, nv, w) in [(2, 2, 64), (2, 6, 32), (1, 4, 128)] {
        for rows in [true, false] {
            let what = format!("{nk} key heads, {nv} value heads, {w} wide (rows {rows})");
            let g = lifted(&delta_layer(nk, nv, w, 1, true, true), rows, None);
            let o = optimize(&g, &ReferenceShim, &["gate_projections"]);
            assert_eq!(o.fired("gate_projections"), 0, "{what}: held");
            assert_eq!(o.fired("gdn_input_epilogue"), 1, "{what}: {:?}", o.fired);
            assert_eq!(o.fired("merged_projections"), 0, "{what}: no product is left to merge beside the epilogue: {:?}", o.fired);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let epilogue = o
                .graph
                .ops
                .iter()
                .find(|op| op.kind() == ClassId::GdnInputEpilogue.id())
                .unwrap_or_else(|| panic!("{what}: no input epilogue makes the rule's inputs: {:?}", kinds(&o.graph)));
            let made: Vec<Op> = leaves(epilogue.clone());
            let read: BTreeSet<String> = made.iter().filter_map(|l| gemv(l, &o.graph)).map(|p| p.x).collect();
            let by: BTreeSet<String> = made.iter().filter_map(|l| gemv(l, &o.graph)).map(|p| p.w).collect();
            assert_eq!(by, products.iter().map(|p| p.to_string()).collect(), "{what}: the epilogue's products");
            assert_eq!(read, BTreeSet::from(["X".to_string()]), "{what}: one activation");
            for t in ["BR", "AR", "BT", "DY"] {
                assert!(made.iter().any(|l| l.out == t), "{what}: `{t}` is made in the epilogue");
            }
            let apart: Vec<&str> = o.graph.ops.iter().filter(|op| !std::ptr::eq(*op, epilogue)).map(|op| op.kind()).filter(|k| !matches!(*k, "GatedDelta" | "Matmul")).collect();
            assert!(apart.is_empty(), "{what}: the inputs run beside {apart:?}");
            assert_eq!(o.graph.ops.iter().filter(|op| op.kind() == "Matmul").count(), 1, "{what}: only the product of the rule's result apart");
        }
    }
    let g = lifted(&delta_layer(2, 4, 32, 1, true, false), true, None);
    let o = optimize(&g, &ReferenceShim, &["gate_projections"]);
    carried_run(&g, &o.graph, &["S", "CQ", "CK", "CV"], &["X"], 32);
    // Gates projected from a second activation stay out of the epilogue, which still takes the four of its own.
    let second = delta_layer(2, 4, 32, 1, true, false)
        .replacen("Let BR be X matmul PBR.", "Let BR be X2 matmul PBR.", 1)
        .replacen("Let AR be X matmul PAR.", "Let AR be X2 matmul PAR.", 1)
        .replacen("## Main\n", "## Main\nLet X2 be a 1 by 64 tensor of Real.\n", 1);
    for rows in [true, false] {
        let what = format!("gates from a second activation (rows {rows})");
        let g = lifted(&second, rows, None);
        let o = optimize(&g, &ReferenceShim, &["gate_projections"]);
        assert_eq!(o.fired("gdn_input_epilogue"), 1, "{what}: {:?}", o.fired);
        same_meaning(&what, &g, &o.graph, &o.fired);
        let epilogue = o.graph.ops.iter().find(|op| op.kind() == ClassId::GdnInputEpilogue.id()).unwrap_or_else(|| panic!("{what}: no input epilogue: {:?}", kinds(&o.graph)));
        let by: BTreeSet<String> = leaves(epilogue.clone()).iter().filter_map(|l| gemv(l, &o.graph)).map(|p| p.w).collect();
        assert_eq!(by, ["PXQ", "PXK", "PXV", "PZ"].iter().map(|p| p.to_string()).collect(), "{what}: the epilogue's products");
        for t in ["BR", "AR"] {
            assert!(!leaves(epilogue.clone()).iter().any(|l| l.out == t), "{what}: `{t}` is projected outside the epilogue");
            assert!(o.graph.ops.iter().any(|op| leaves(op.clone()).iter().any(|l| l.out == t && gemv(l, &o.graph).is_some_and(|p| p.x == "X2"))), "{what}: `{t}` is a product of the second activation");
        }
    }
}

/// A chip whose own rule hands the query's and key's L2 norms of a one-row gated delta layer's input epilogue to the
/// delta rule reading them (`Rewriter::hand_over`, as the CUDA lane's `delta_qk_norm` does).
struct HandingNorms;

impl HandingNorms {
    fn hand(r: &mut logicaffeine_compile::tensor_graph::fuse::Rewriter) {
        for j in 0..r.s.len() {
            let Some(op) = r.s.slot(j) else { continue };
            if op.kind() != "GatedDelta" {
                continue;
            }
            let (q, k) = (op.ins[0].clone(), op.ins[1].clone());
            let Some(e) = r.s.maker(&q) else { continue };
            if r.s.maker(&k) == Some(e) && r.s.get(e).kind() == ClassId::GdnInputEpilogue.id() {
                r.hand_over("hand_norms", ClassId::DeltaQkNorm, e, &[q, k], j);
            }
        }
    }
}

impl Shim for HandingNorms {
    fn name(&self) -> String {
        "reference-handing-norms".to_string()
    }
    fn supports(&self, _: ClassId) -> Support {
        Support::Native
    }
    fn fits(&self, _: &Op, _: &TensorGraph) -> Result<(), String> {
        Ok(())
    }
    fn rules(&self) -> Vec<(logicaffeine_compile::tensor_graph::shim::Position, logicaffeine_compile::tensor_graph::fuse::ChipRule<'_>)> {
        use logicaffeine_compile::tensor_graph::fuse::ChipRule;
        use logicaffeine_compile::tensor_graph::shim::Position;
        vec![(Position::After("gdn_input_epilogue"), ChipRule { name: "hand_norms", builds: &[ClassId::DeltaQkNorm], apply: Box::new(Self::hand) })]
    }
    fn numerics(&self) -> Option<logicaffeine_compile::tensor_graph::numerics::Contract> {
        ReferenceShim.numerics()
    }
}

/// Every fused op of `o` at every depth computes its result in one of its parts — into a buffer of that name, where it
/// lands in one of its own — covers exactly the lifted ops its parts cover, and keeps every result of its parts that
/// anything outside it reads or the host does; and the top-level ops cover each lifted op at most once.
fn folds_are_whole(what: &str, o: &TensorGraph) {
    use logicaffeine_compile::tensor_graph::graph::Sym;
    fn every<'a>(op: &'a Op, out: &mut Vec<&'a Op>) {
        if let Sym::Fused(f) = &op.sym {
            out.push(op);
            for p in &f.parts {
                every(p, out);
            }
        }
    }
    let mut folds = Vec::new();
    for op in &o.ops {
        every(op, &mut folds);
    }
    let lifted: Vec<Op> = o.ops.iter().flat_map(|op| leaves(op.clone())).collect();
    let observed = |t: &str| o.tensors.get(t).is_some_and(|x| x.observed) || o.io.as_ref().is_some_and(|io| io.names().contains(&t));
    for fold in folds {
        let inside = leaves(fold.clone());
        let mut covers: Vec<usize> = inside.iter().flat_map(|l| l.covers.clone()).collect();
        covers.sort_unstable();
        assert_eq!(fold.covers, covers, "{what}: a `{}` covers what its parts cover", fold.kind());
        assert!(inside.iter().any(|l| l.out == fold.out), "{what}: a `{}`'s result `{}` is computed by a part of it", fold.kind(), fold.out);
        if fold.dst.own().is_some() {
            assert_eq!(fold.dst.buf, fold.out, "{what}: a `{}` lands its result in a buffer of its own", fold.kind());
        }
        let made: BTreeSet<&str> = inside.iter().map(|l| l.out.as_str()).collect();
        for l in &inside {
            let read_outside = lifted.iter().any(|x| !made.contains(x.out.as_str()) && x.ins.contains(&l.out));
            if l.out != fold.out && (read_outside || observed(&l.out)) {
                assert!(fold.keeps.contains(&l.out), "{what}: a `{}` keeps `{}`, which is read outside it", fold.kind(), l.out);
            }
        }
    }
    let mut seen = BTreeSet::new();
    for c in o.ops.iter().flat_map(|op| op.covers.clone()) {
        assert!(seen.insert(c), "{what}: lifted op {c} is covered twice");
    }
}

/// A chip's own rule hands parts of a fold to the op that reads them (`Rewriter::hand_over`): on one-row gated delta
/// layers of as many value heads as key heads and more, at heads of 32 to 128, the query's and key's L2 norms handed
/// from the input epilogue to the delta rule leave the epilogue's convolutions writing the heads unnormed — each fold
/// of a convolution, its SiLU and its norm the convolution and its SiLU alone, no norm left in the epilogue — and the
/// rule one `delta_qk_norm` op over the two norms and itself, reading the SiLU's results; every fold whole
/// (`folds_are_whole`); the graph meaning what the lifted graph means, and through a run of steps from a zero history.
/// Held, the rule fires nowhere, is reported held where it would have fired, and the graph is the reference's.
#[test]
fn a_chips_rule_hands_a_folds_parts_to_the_op_reading_them() {
    use logicaffeine_compile::tensor_graph::fuse::Outcome;
    for (nk, nv, w) in [(2, 2, 64), (2, 4, 32), (2, 6, 32), (1, 4, 128)] {
        for out in [false, true] {
            for rows in [true, false] {
                let what = format!("{nk} key heads, {nv} value heads, {w} wide (rows {rows}, out {out})");
                let g = lifted(&delta_layer(nk, nv, w, 1, true, out), rows, None);
                let o = optimize(&g, &HandingNorms, &[]);
                assert_eq!(o.fired("gdn_input_epilogue"), 1, "{what}: {:?}", o.fired);
                assert_eq!(o.fired("hand_norms"), 1, "{what}: {:?}", o.fired);
                same_meaning(&what, &g, &o.graph, &o.fired);
                folds_are_whole(&what, &o.graph);
                let epilogue = o.graph.ops.iter().find(|op| op.kind() == ClassId::GdnInputEpilogue.id()).unwrap_or_else(|| panic!("{what}: {:?}", kinds(&o.graph)));
                assert!(!leaves(epilogue.clone()).iter().any(|l| l.kind() == "L2Norm"), "{what}: the epilogue keeps no norm");
                let classes = realizes(epilogue, &o.graph);
                assert!(classes.contains(&ClassId::ConvBiasSilu) && !classes.contains(&ClassId::ConvSiluL2norm), "{what}: its convolutions end at their SiLU: {classes:?}");
                for t in ["SQ", "SK"] {
                    assert!(epilogue.keeps.contains(&t.to_string()) || epilogue.out == t, "{what}: the epilogue keeps `{t}` for the rule");
                }
                let step = o.graph.ops.iter().find(|op| op.kind() == ClassId::DeltaQkNorm.id()).unwrap_or_else(|| panic!("{what}: {:?}", kinds(&o.graph)));
                let parts: Vec<String> = leaves(step.clone()).iter().map(|l| l.kind().to_string()).collect();
                assert_eq!(parts, ["L2Norm", "L2Norm", "GatedDelta"], "{what}: the rule over the two norms it was handed");
                assert_eq!(step.ins[..2], ["SQ".to_string(), "SK".to_string()], "{what}: the rule reads the SiLU's results");
                assert_eq!(step.out, "O", "{what}: the rule's result is the op's");
                let held = optimize(&g, &HandingNorms, &["hand_norms"]);
                assert_eq!(held.fired("hand_norms"), 0, "{what}: held");
                assert!(!held.report.of("hand_norms").is_empty() && held.report.of("hand_norms").iter().all(|d| d.outcome == Outcome::Held), "{what}: reported held");
                let reference = optimize(&g, &ReferenceShim, &[]);
                assert_eq!(format!("{:?}", held.graph.ops), format!("{:?}", reference.graph.ops), "{what}: held, the graph is the reference's");
            }
        }
    }
    let g = lifted(&delta_layer(2, 4, 32, 1, true, false), true, None);
    let o = optimize(&g, &HandingNorms, &[]);
    assert_eq!(o.fired("hand_norms"), 1, "the run's rule hands its norms");
    carried_run(&g, &o.graph, &["S", "CQ", "CK", "CV"], &["X"], 32);
}

/// A gated attention's output, read only by its product with its gate's activation, is gated in the attention's store
/// (`attn_output_gate`, K-13), meaning what they mean bit for bit: on Qwen3.5's gated attention, on a decode step, a
/// prefill chunk and a batch, every attention whose output a gate multiplies.
#[test]
fn a_gated_attentions_output_gate_folds_into_its_attention() {
    let spec = tiny_models::Tiny::Qwen35.spec(24);
    for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
        let ((source, _, _), _) = program::written(&spec, cut);
        for rows in [true, false] {
            let what = format!("Qwen3.5 {cut:?} (rows {rows})");
            let g = lifted(&source, rows, Some(io(cut)));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let attentions: Vec<&Op> = g.ops.iter().filter(|op| op.kind() == "Attention").collect();
            assert!(!attentions.is_empty(), "{what}: no attention");
            for a in attentions {
                let op = o
                    .graph
                    .ops
                    .iter()
                    .find(|op| leaves((*op).clone()).iter().any(|l| l.kind() == "Attention" && l.out == a.out))
                    .unwrap_or_else(|| panic!("{what}: no op computes the attention `{}`", a.out));
                let parts = leaves(op.clone());
                assert!(
                    realizes(op, &o.graph).contains(&ClassId::AttnOutputGate) && parts.iter().any(|l| l.kind() == "Sigmoid"),
                    "{what}: the attention `{}` is gated by a `{}` of {:?}",
                    a.out,
                    op.kind(),
                    parts.iter().map(|l| l.kind()).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// A mixture's combined result, one summand of the residual add whose sum the next norm reads, is combined inside that
/// add and norm (PARITY-MOE-DEFERRED-FINALIZE), meaning what they mean bit for bit, its experts summed in their order: on
/// every tiny architecture with a mixture — Qwen3-MoE's renormalized top-k and the same un-renormalized, gpt-oss's biased
/// experts, DeepSeek-V2's shared expert — on a decode step, a prefill chunk and a batch, every mixture, a step's last
/// layer's into the final norm. A chunk's last layer feeds only its last row, which the final norm reads
/// (PARITY-LAST-LAYER-ROWS): no norm reads that layer's sum.
#[test]
fn an_expert_combine_feeding_an_add_and_norm_fuses_into_it() {
    let mut plain = tiny_models::Tiny::Qwen3Moe.spec(24);
    for layer in &mut plain.layers {
        if let logicaffeine_compile::model::spec::Ffn::Moe(m) = &mut layer.ffn {
            m.norm_top_k = false;
        }
    }
    let specs = [
        ("Qwen3-MoE", tiny_models::Tiny::Qwen3Moe.spec(24)),
        ("Qwen3-MoE, its weights not renormalized", plain),
        ("gpt-oss", tiny_models::Tiny::GptOss.spec(24)),
        ("DeepSeek-V2", tiny_models::Tiny::DeepSeek2.spec(24)),
    ];
    for (name, spec) in specs {
        for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
            let ((source, _, _), _) = program::written(&spec, cut);
            for rows in [true, false] {
                let what = format!("{name} {cut:?} (rows {rows})");
                let g = lifted(&source, rows, Some(io(cut)));
                let o = optimize(&g, &ReferenceShim, &[]);
                same_meaning(&what, &g, &o.graph, &o.fired);
                let mixtures: Vec<&Op> = g.ops.iter().filter(|op| op.kind() == "Moe").collect();
                assert!(!mixtures.is_empty(), "{what}: no mixture");
                // The last layer's mixtures: those routed by its router, one a row in a chunk.
                let last_router = mixtures.last().expect("a mixture").ins[1].clone();
                let chunk = matches!(cut, Cut::Prefill { .. });
                for m in mixtures.iter().filter(|m| !(chunk && m.ins[1] == last_router)) {
                    let op = o
                        .graph
                        .ops
                        .iter()
                        .find(|op| leaves((*op).clone()).iter().any(|l| l.kind() == "Moe" && l.out == m.out))
                        .unwrap_or_else(|| panic!("{what}: no op computes the mixture `{}`", m.out));
                    let parts = leaves(op.clone());
                    assert!(
                        realizes(op, &o.graph).contains(&ClassId::MoeDeferredFinalize) && parts.iter().any(|l| l.kind() == "FusedRow"),
                        "{what}: the mixture `{}` is combined by a `{}` of {:?}",
                        m.out,
                        op.kind(),
                        parts.iter().map(|l| l.kind()).collect::<Vec<_>>()
                    );
                    if m.ins[1] == last_router {
                        assert!(
                            parts.iter().any(|l| l.kind() == "FusedRow" && l.ins.iter().any(|t| t == "on")),
                            "{what}: the last layer's mixture is not combined in the final norm"
                        );
                    }
                }
            }
        }
    }
}

/// An all-reduce of products' partials feeding an add and norm folds into the norm (PARITY-ALLREDUCE-NORM), meaning
/// what it meant bit for bit, its partials summed in the order the program adds them: the left fold of `plus` over two,
/// three, four and eight ranks' products into the add and norm its sum feeds, and into a post-norm that takes the sum
/// alone — every add of the fold and that norm one op, the residual's add and norm after it an op of its own; the norm
/// reads every partial where its product wrote it. A sum the program adds as a tree, or one of whose summands no product
/// writes, stays apart. Qwen3 and Llama split over two and four chips fold every layer's two all-reduces, the last into
/// the final norm; Gemma 3, whose sublayers post-norm their outputs, folds each into its post-norm.
#[test]
fn an_all_reduce_feeding_an_add_and_norm_fuses() {
    use logicaffeine_compile::tensor_graph::fuse::allreduce_partials;
    let decl = |t: &str, r: usize, c: usize| format!("Let {t} be a {r} by {c} tensor of Real.\n");
    let ranks = |tp: usize| -> (String, Vec<String>) {
        let mut src = format!("## Main\n{}{}{}", decl("R", 1, 32), decl("N", 1, 32), decl("M", 1, 32));
        let mut partials = Vec::new();
        for k in 0..tp {
            src += &format!("{}{}Let P{k} be X{k} matmul W{k}.\n", decl(&format!("X{k}"), 1, 16), decl(&format!("W{k}"), 16, 32));
            partials.push(format!("P{k}"));
        }
        let mut acc = "P0".to_string();
        for k in 1..tp {
            let s = if k + 1 == tp { "S".to_string() } else { format!("S{k}") };
            src += &format!("Let {s} be {acc} plus P{k}.\n");
            acc = s;
        }
        (src, partials)
    };
    for tp in [2, 3, 4, 8] {
        let (body, partials) = ranks(tp);
        for (form, tail, adds) in [
            ("an add and norm", "Let T be R plus S.\nLet Y be rmsnorm of T with weight N.\n", tp),
            ("a post-norm", "Let Q be rmsnorm of S with weight M.\nLet T be R plus Q.\nLet Y be rmsnorm of T with weight N.\n", tp - 1),
        ] {
            let what = format!("{tp} ranks into {form}");
            let (_, o) = reference(&format!("{body}{tail}"), &what);
            let mut parts = vec!["Elementwise"; adds];
            parts.push("FusedRow");
            let op = folding(&o, ClassId::AllreduceNorm, &parts).unwrap_or_else(|| panic!("{what}: the all-reduce is not in the norm: {:?}", kinds(&o)));
            if adds < tp {
                assert!(
                    o.ops.iter().any(|w| realizes(w, &o).contains(&ClassId::AddNorm) && w.ins.iter().any(|t| t == "Q")),
                    "{what}: the residual's add and norm is an op of its own, reading the post-norm: {:?}",
                    kinds(&o)
                );
            }
            let (folded, sum) = allreduce_partials(op).unwrap_or_else(|| panic!("{what}: the fold names no partials: {op:?}"));
            assert_eq!(folded, partials, "{what}: every rank's partial, in the order the program adds them");
            assert_eq!(sum, "S", "{what}: the fold ends at the all-reduce's sum");
            for p in &partials {
                assert!(o.ops.iter().any(|w| w.kind() == "Matmul" && w.out == *p), "{what}: `{p}` is still its rank's product");
            }
        }
    }
    let (body, _) = ranks(2);
    let tree = format!(
        "## Main\n{}{}{}{}Let P0 be X0 matmul W0.\nLet P1 be X1 matmul W1.\nLet P2 be X2 matmul W2.\nLet P3 be X3 matmul W3.\n\
         Let A be P0 plus P1.\nLet B be P2 plus P3.\nLet S be A plus B.\nLet T be R plus S.\nLet Y be rmsnorm of T with weight N.\n",
        decl("R", 1, 32),
        decl("N", 1, 32),
        (0..4).map(|k| decl(&format!("X{k}"), 1, 16)).collect::<String>(),
        (0..4).map(|k| decl(&format!("W{k}"), 16, 32)).collect::<String>(),
    );
    let unproduced = format!("{}{}Let U be S plus V.\nLet T be R plus U.\nLet Y be rmsnorm of T with weight N.\n", body, decl("V", 1, 32));
    for (what, src) in [("a sum added as a tree", tree), ("a summand no product writes", unproduced)] {
        let (_, o) = reference(&src, what);
        assert!(folding(&o, ClassId::AllreduceNorm, &[]).is_none(), "{what}: folded as an all-reduce: {:?}", kinds(&o));
    }
    for t in [tiny_models::Tiny::Qwen3, tiny_models::Tiny::Llama] {
        let spec = t.wide(24);
        for tp in [2, 4] {
            let what = format!("{} wide over {tp}", t.name());
            let ((source, _, _, _), _) = program::written_split(&spec, Cut::Decode { ctx: 24 }, "", tp).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            let g = lifted(&source, true, Some(io(Cut::Decode { ctx: 24 })));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let folds: Vec<&Op> = o.graph.ops.iter().filter(|op| realizes(op, &o.graph).contains(&ClassId::AllreduceNorm)).collect();
            assert_eq!(folds.len(), 2 * spec.layers.len(), "{what}: the attention's and the FFN's all-reduce of every layer fold");
            for op in &folds {
                let (partials, _) = allreduce_partials(op).unwrap_or_else(|| panic!("{what}: a fold names no partials"));
                assert_eq!(partials.len(), tp, "{what}: every rank's partial: {partials:?}");
            }
            let last = folds.last().expect("a fold");
            assert!(leaves((*last).clone()).iter().any(|l| l.kind() == "FusedRow" && l.ins.iter().any(|t| t == "on")), "{what}: the last all-reduce folds into the final norm");
        }
    }
    let spec = tiny_models::Tiny::Gemma3.wide(24);
    for tp in [2, 4] {
        let what = format!("gemma3 wide over {tp}");
        let ((source, _, _, _), _) = program::written_split(&spec, Cut::Decode { ctx: 24 }, "", tp).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let g = lifted(&source, true, Some(io(Cut::Decode { ctx: 24 })));
        let o = optimize(&g, &ReferenceShim, &[]);
        same_meaning(&what, &g, &o.graph, &o.fired);
        let folds: Vec<&Op> = o.graph.ops.iter().filter(|op| realizes(op, &o.graph).contains(&ClassId::AllreduceNorm)).collect();
        assert_eq!(folds.len(), 2 * spec.layers.len(), "{what}: every sublayer's all-reduce folds into its post-norm");
        for op in &folds {
            let (partials, _) = allreduce_partials(op).unwrap_or_else(|| panic!("{what}: a fold names no partials"));
            assert_eq!(partials.len(), tp, "{what}: every rank's partial: {partials:?}");
            assert!(!realizes(op, &o.graph).contains(&ClassId::AddNorm), "{what}: a post-norm takes the sum alone, the residual's add apart");
        }
    }
}

/// A norm of a whole row joined from parts, whose result slices alone read, each taking one part back, norms each part
/// where it lives by the whole row's sum of squares (PARITY-ALLREDUCE-NORM, `allreduce_sumsq_norm`), meaning what the
/// program means: over two, three, four and eight parts of unequal widths, joined as the left fold a tensor-parallel
/// step writes and as a tree, the joins, the norm and the slices are one op that reads the parts themselves — never the
/// joined row — and writes each slice; the parts are the joined operands in order, each slice taking its own. A join
/// another op reads, a slice across two parts, a norm a statement shows or another op reads, and a norm head by head
/// stay apart. A step whose QK norm spans every head (MiniMax-M2's), split over two and four chips, norms each layer's
/// queries and keys so, every chip's raw projection a part and its normed heads the slice it takes back — in the decode
/// step and in a batch.
#[test]
fn a_norms_all_reduce_carries_only_its_sum_of_squares() {
    use logicaffeine_compile::model::spec::Mixer;
    use logicaffeine_compile::tensor_graph::fuse::parts_norm;
    let decl = |t: &str, r: usize, c: usize| format!("Let {t} be a {r} by {c} tensor of Real.\n");
    // Each part `P{k}` a product of `X` by its own columns; the row they join normed whole; each part taken back as `S{k}`
    // and shown. `join` writes the joins of parts `lo..hi` and names the row they make.
    fn join(src: &mut String, lo: usize, hi: usize, tree: bool, names: &mut usize) -> String {
        if hi - lo == 1 {
            return format!("P{lo}");
        }
        let (a, b) = if tree {
            let mid = lo + (hi - lo) / 2;
            (join(src, lo, mid, tree, names), join(src, mid, hi, tree, names))
        } else {
            (join(src, lo, hi - 1, tree, names), format!("P{}", hi - 1))
        };
        *names += 1;
        let out = format!("J{names}");
        src.push_str(&format!("Let {out} be {a} joined with {b}.\n"));
        out
    }
    let taken = |at: usize, w: usize, total: usize| match (at, at + w == total) {
        (0, _) => format!("the first {w}"),
        (_, true) => format!("the last {w}"),
        _ => format!("the {w} after the first {at}"),
    };
    let program = |widths: &[usize], tree: bool, extra: &str, slices: &[(usize, usize)]| -> (String, String) {
        let total: usize = widths.iter().sum();
        let mut src = format!("## Main\n{}{}", decl("X", 1, 16), decl("N", 1, total));
        for (k, w) in widths.iter().enumerate() {
            src += &format!("{}Let P{k} be X matmul W{k}.\n", decl(&format!("W{k}"), 16, *w));
        }
        let row = join(&mut src, 0, widths.len(), tree, &mut 0);
        src += &format!("Let Y be rmsnorm of {row} with weight N.\n");
        for (k, &(at, w)) in slices.iter().enumerate() {
            src += &format!("Let S{k} be {} of every {total} of Y.\nShow item 1 of S{k}.\n", taken(at, w, total));
        }
        src += extra;
        (src, row)
    };
    let own = |widths: &[usize]| -> Vec<(usize, usize)> { widths.iter().scan(0, |at, &w| Some((std::mem::replace(at, *at + w), w))).collect() };
    for widths in [&[8, 24][..], &[16, 8, 40], &[8, 16, 8, 32], &[8, 8, 16, 8, 8, 24, 8, 8]] {
        for tree in [false, true] {
            let what = format!("{} parts {widths:?}{}", widths.len(), if tree { " joined as a tree" } else { "" });
            let (src, _) = program(widths, tree, "", &own(widths));
            let (_, o) = reference(&src, &what);
            let mut pattern = vec!["Join"; widths.len() - 1];
            pattern.push("FusedRow");
            pattern.extend(vec!["HeadSlice"; widths.len()]);
            let op = folding(&o, ClassId::AllreduceSumsqNorm, &pattern).unwrap_or_else(|| panic!("{what}: the norm is not by parts: {:?}", kinds(&o)));
            let (parts, n, slices) = parts_norm(op).unwrap_or_else(|| panic!("{what}: the op names no parts: {op:?}"));
            let named: Vec<(String, usize)> = widths.iter().enumerate().map(|(k, &w)| (format!("P{k}"), w)).collect();
            assert_eq!(parts, named, "{what}: every part, in the order the joins put them side by side");
            assert_eq!((n.out.as_str(), n.cols), ("Y", widths.iter().sum()), "{what}: the norm of the whole row");
            let took: Vec<(usize, String)> = slices.iter().map(|(k, s, _)| (*k, s.clone())).collect();
            assert_eq!(took, (0..widths.len()).map(|k| (k, format!("S{k}"))).collect::<Vec<_>>(), "{what}: each slice takes its own part back");
            let mut reads: Vec<String> = named.iter().map(|(t, _)| t.clone()).chain(["N".to_string()]).collect();
            let mut ins = op.ins.clone();
            reads.sort();
            ins.sort();
            assert_eq!(ins, reads, "{what}: the op reads the parts and the weight, never a joined row");
            assert_eq!(op.out, format!("S{}", widths.len() - 1), "{what}: the op ends at the last slice");
            assert_eq!(op.keeps, (0..widths.len() - 1).map(|k| format!("S{k}")).collect::<Vec<_>>(), "{what}: every slice is written");
            for (t, _) in &named {
                assert!(
                    o.ops.iter().any(|w| !std::ptr::eq(w, op) && (w.out == *t || w.keeps.contains(t))),
                    "{what}: `{t}` is written by a product apart from the norm, alone or merged with the others of one input"
                );
            }
        }
    }
    let widths = [8, 16, 8];
    for (what, slices, extra) in [
        ("a join another op reads", own(&widths), "Let Z be J1 scaled by 2.0.\n".to_string()),
        ("the joined row another op reads", own(&widths), "Let Z be J2 scaled by 2.0.\n".to_string()),
        ("a slice across two parts", vec![(0, 8), (4, 16), (24, 8)], String::new()),
        ("a norm a statement shows", own(&widths), "Show item 1 of Y.\n".to_string()),
        ("a norm another op reads", own(&widths), "Let Z be Y scaled by 2.0.\n".to_string()),
    ] {
        let (src, _) = program(&widths, false, &extra, &slices);
        let (_, o) = reference(&src, what);
        assert!(folding(&o, ClassId::AllreduceSumsqNorm, &[]).is_none(), "{what}: normed by parts: {:?}", kinds(&o));
    }
    let heads = format!(
        "## Main\n{}{}{}{}Let P0 be X matmul W0.\nLet P1 be X matmul W1.\nLet J be P0 joined with P1.\nLet Y be rmsnorm of J with weight N across 4 heads.\n\
         Let S0 be the first 16 of every 32 of Y.\nLet S1 be the last 16 of every 32 of Y.\nShow item 1 of S0.\nShow item 1 of S1.\n",
        decl("X", 1, 16),
        decl("N", 1, 8),
        decl("W0", 16, 16),
        decl("W1", 16, 16),
    );
    let (_, o) = reference(&heads, "a norm head by head");
    assert!(folding(&o, ClassId::AllreduceSumsqNorm, &[]).is_none(), "a norm head by head: normed by parts: {:?}", kinds(&o));
    // MiniMax-M2's attention: the QK norm across every head, on Qwen3's layout; four KV heads for four chips.
    let whole = |mut spec: logicaffeine_compile::model::spec::ModelSpec, heads: Option<(usize, usize)>| {
        for layer in &mut spec.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                a.qk_norm_whole = true;
                if let Some((h, kv)) = heads {
                    (a.heads, a.kv_heads) = (h, kv);
                }
            }
        }
        spec
    };
    for (spec, tp) in [(whole(tiny_models::Tiny::Qwen3.wide(24), None), 2), (whole(tiny_models::Tiny::Qwen3.wide(24), Some((8, 4))), 4)] {
        for (cut, pre) in [(Cut::Decode { ctx: 24 }, ""), (Cut::Batch { rows: 2, ctx: 24 }, "b")] {
            let what = format!("a whole QK norm over {tp} chips, {cut:?}");
            let ((source, _, _, _), _) = program::written_split(&spec, cut, "", tp).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            let g = lifted(&source, true, Some(io(cut)));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let norms: Vec<&Op> = o.graph.ops.iter().filter(|op| realizes(op, &o.graph).contains(&ClassId::AllreduceSumsqNorm)).collect();
            assert_eq!(norms.len(), 2 * spec.layers.len(), "{what}: every layer's queries and keys are normed by their chips' parts");
            for (i, op) in norms.iter().enumerate() {
                let (l, side) = (i / 2, if i % 2 == 0 { ("qr", "qm") } else { ("kr", "km") });
                let (parts, _, slices) = parts_norm(op).unwrap_or_else(|| panic!("{what}: a norm names no parts"));
                let raw: Vec<String> = parts.iter().map(|(t, _)| t.clone()).collect();
                assert_eq!(raw, (0..tp).map(|c| format!("{pre}{}{l}_c{c}", side.0)).collect::<Vec<_>>(), "{what}: every chip's projection a part, in chip order");
                let took: Vec<(usize, String)> = slices.iter().map(|(k, s, _)| (*k, s.clone())).collect();
                assert_eq!(took, (0..tp).map(|c| (c, format!("{pre}{}{l}_c{c}", side.1))).collect::<Vec<_>>(), "{what}: each chip takes its own heads back");
            }
        }
    }
    let refused = program::written_split(&whole(tiny_models::Tiny::Qwen3.wide(24), None), Cut::Decode { ctx: 24 }, "", 4).map(|_| ()).unwrap_err();
    assert!(format!("{refused:?}").contains("a KV head on more than one chip"), "a whole QK norm over more chips than KV heads is refused by name: {refused:?}");
}

/// A cache row kept in each scheme of `kv_cache_quant` (PARITY-KV-CACHE-QUANT) reads back within the bound the scheme
/// declares, its block the row as stored: binary16, bfloat16 and fp8 e4m3 each value to its nearest; int8 codes under the
/// row's one binary16 scale; the same of the row turned by the Walsh–Hadamard transform, turned back — over smooth rows,
/// a row one channel of which is a hundred times the rest, zeros, one repeated value, tiny values, a float format's
/// extremes, and a row no power of two long, which the turned scheme refuses. The turn keeps the outlier row closer than
/// the plain codes do, the reason it turns. The D5005 engine's cache rows, in the card's own arithmetic
/// (`decode_engine::numerics`), lie within the same bounds: its int8 and Hadamard caches link the shared schemes.
#[test]
fn a_quantized_cache_row_reads_back_what_its_scheme_keeps() {
    use logicaffeine_compile::model::quant::f16_to_f32;
    use logicaffeine_compile::spatial::decode_engine::numerics::{self as card, FpUnit};
    use logicaffeine_compile::tensor_graph::numerics::Keep;
    let seeded = |n: usize, salt: u64| -> Vec<f64> {
        (0..n)
            .map(|i| {
                let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
                ((h >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
            })
            .collect()
    };
    let mut outlier = seeded(64, 3);
    outlier[17] = 100.0;
    let rows: Vec<(String, Vec<f64>)> = vec![
        ("a smooth row of 64".into(), seeded(64, 1)),
        ("a smooth row of 128".into(), seeded(128, 2)),
        ("a row one channel of which is a hundred times the rest".into(), outlier.clone()),
        ("a row of zeros".into(), vec![0.0; 64]),
        ("a row of one value".into(), vec![0.375; 32]),
        ("a row of tiny values".into(), seeded(64, 5).iter().map(|v| v * 1e-6).collect()),
        ("a row of 96, no power of two".into(), seeded(96, 4)),
    ];
    let schemes = [Keep::F16, Keep::Bf16, Keep::E4m3, Keep::Int8Rows, Keep::Int8RowsRotated];
    let within = |keep: Keep, what: &str, row: &[f64], kept: &[f64]| {
        let (bound, block) = (keep.bound(), keep.row_block(row));
        for (i, (&v, &k)) in row.iter().zip(kept).enumerate() {
            assert!(bound.holds(v, k, block), "{keep:?}, {what}: value {i} is {v}, kept as {k}, past the bound of a block of {block}");
        }
    };
    for keep in schemes {
        let mut cases = rows.clone();
        if keep.range().is_finite() {
            let top = keep.range();
            cases.push(("the format's largest values".into(), seeded(32, 6).iter().map(|v| v * top).collect()));
        }
        for (what, row) in &cases {
            match keep.keep_row(row) {
                Some(kept) => within(keep, what, row, &kept),
                None => assert!(keep == Keep::Int8RowsRotated && !row.len().is_power_of_two(), "{keep:?}, {what}: only a turn of a row no power of two long is refused"),
            }
        }
    }
    let worst = |keep: Keep, row: &[f64]| keep.keep_row(row).unwrap().iter().zip(row).map(|(k, v)| (k - v).abs()).fold(0.0, f64::max);
    // Rows of uniform values from splitmix64, each with one channel a hundred times the rest: no row aligned with the
    // transform's own basis, as a seeded hash's can be.
    let uniform = |n: usize, seed: u64| -> Vec<f64> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = s;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                ((z >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
            })
            .collect()
    };
    let ratios: Vec<f64> = (1..=32)
        .map(|seed| {
            let mut row = uniform(64, seed);
            row[17] = 100.0;
            worst(Keep::Int8RowsRotated, &row) / worst(Keep::Int8Rows, &row)
        })
        .collect();
    let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
    assert!(
        mean < 1.0 / 3.0,
        "the turn spreads an outlier across its row, its one scale fitting every value: over 32 rows the turned row's worst error is {mean} of the plain row's ({ratios:?})"
    );
    // The engine's rows, as the card computes them: the codes under its scale, and those of the turned row turned back.
    for (what, row) in &rows {
        let x: Vec<f32> = row.iter().map(|&v| v as f32).collect();
        let read = |x: &[f32]| -> Vec<f32> {
            let (s, codes) = card::cache_row(FpUnit::Soft, x);
            codes.iter().map(|&c| f32::from(c) * f16_to_f32(s)).collect()
        };
        let plain: Vec<f64> = read(&x).into_iter().map(f64::from).collect();
        within(Keep::Int8Rows, &format!("the engine's {what}"), row, &plain);
        if x.len().is_power_of_two() {
            let mut turned = x.clone();
            card::hadamard(&mut turned);
            let mut back = read(&turned);
            card::hadamard(&mut back);
            let inv = 1.0 / x.len() as f32;
            let rotated: Vec<f64> = back.into_iter().map(|v| f64::from(v * inv)).collect();
            within(Keep::Int8RowsRotated, &format!("the engine's turned {what}"), row, &rotated);
        }
    }
}

/// A soft cap whose values an arg-max alone reads folds into the arg-max's read (PARITY-SOFTCAP, `cap_argmax`): over a
/// row of 8 and a row of the tiny vocabulary, the fold reads the cap's input, writes the capped row nowhere, and picks
/// what the arg-max of the raw row picks; a cap a statement shows folds too, the capped row kept; a cap a second op
/// reads stays apart. Gemma 2's decode step, chunk and batch fold every sequence's final cap into its arg-max, keeping
/// the capped logits exactly where the host reads them.
#[test]
fn a_cap_before_an_argmax_is_dropped_and_the_argmax_is_unchanged() {
    use logicaffeine_compile::tensor_graph::graph::{StepIo, Sym};
    /// Every fused op of `op`, `op` too, at any depth.
    fn fused(op: &Op, out: &mut Vec<Op>) {
        if let Sym::Fused(f) = &op.sym {
            out.push(op.clone());
            f.parts.iter().for_each(|p| fused(p, out));
        }
    }
    for n in [8, tiny_models::VOCAB] {
        let what = format!("a row of {n}");
        let decl = format!("## Main\nLet X be a 1 by {n} tensor of Real.\n");
        let (g, o) = reference(&format!("{decl}Let Y be X capped at 30.0.\nLet next be argmax of Y.\n"), &what);
        let op = folding(&o, ClassId::CapArgmax, &["Softcap", "ArgMax"]).unwrap_or_else(|| panic!("{what}: the cap is not in the arg-max's read: {:?}", kinds(&o)));
        assert_eq!(op.ins, ["X"], "{what}: the arg-max reads the cap's input");
        assert!(op.keeps.is_empty(), "{what}: the capped row is written nowhere: keeps {:?}", op.keeps);
        let raw = lifted(&format!("{decl}Let next be argmax of X.\n"), false, None);
        let ins = inputs(&g);
        assert_eq!(eval(&o, &ins).unwrap()["next"], eval(&raw, &ins).unwrap()["next"], "{what}: the fold picks what the raw row's arg-max picks");
    }
    let decl = "## Main\nLet X be a 1 by 8 tensor of Real.\nLet Y be X capped at 30.0.\nLet next be argmax of Y.\n";
    let (_, o) = reference(&format!("{decl}Show item 1 of Y.\n"), "a cap a statement shows");
    let op = folding(&o, ClassId::CapArgmax, &["Softcap", "ArgMax"]).unwrap_or_else(|| panic!("a cap a statement shows: not folded: {:?}", kinds(&o)));
    assert_eq!(op.keeps, ["Y"], "a cap a statement shows: the capped row kept");
    let (_, o) = reference(&format!("{decl}Let Z be Y scaled by 2.0.\n"), "a cap a second op reads");
    assert!(folding(&o, ClassId::CapArgmax, &[]).is_none(), "a cap a second op reads: folded into the arg-max: {:?}", kinds(&o));
    let spec = tiny_models::Tiny::Gemma2.spec(24);
    for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
        let ((source, _, _), _) = program::written(&spec, cut);
        for logits_read in [true, false] {
            let what = format!("gemma2 {cut:?}, logits {}", if logits_read { "read" } else { "unread" });
            let step_io = StepIo { logits_read, ..io(cut) };
            let g = lifted(&source, true, Some(step_io.clone()));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&what, &g, &o.graph, &o.fired);
            let logits: Vec<String> = std::iter::once(step_io.logits.clone()).chain(step_io.more.iter().map(|q| q.logits.clone())).collect();
            let mut all = Vec::new();
            o.graph.ops.iter().for_each(|op| fused(op, &mut all));
            let mut folds = 0;
            for part in all.iter().filter(|p| matches!(&p.sym, Sym::Fused(f) if f.class == ClassId::CapArgmax)) {
                folds += 1;
                let kept = part.keeps.iter().any(|k| logits.contains(k));
                assert_eq!(kept, logits_read, "{what}: the capped logits kept exactly where the host reads them: keeps {:?}", part.keeps);
            }
            assert_eq!(folds, logits.len(), "{what}: each sequence's final cap folds into its arg-max");
        }
    }
}

/// The op of `g` realizing `class` that computes `t`, at any depth of its parts.
fn computing<'a>(g: &'a TensorGraph, class: ClassId, t: &str) -> Option<&'a Op> {
    g.ops.iter().find(|op| realizes(op, g).contains(&class) && leaves((*op).clone()).iter().any(|l| l.out == t))
}

/// A connected region of pointwise ops of one length and element type is one op (PARITY-ELEMENTWISE-REGIONS), meaning
/// what its parts mean: a diamond; a diamond whose intermediate a later op reads, which the region writes out beside
/// its result; a bias row combined with every row of a block inside it; integer values as well as real ones, a region
/// ending where real values are made of integer ones; and two regions apart where a reduction stands between them, the
/// reduction in neither.
#[test]
fn a_pointwise_dag_fuses_into_one_region() {
    let decl = |t: &str, r: usize, c: usize, e: &str| format!("Let {t} be a {r} by {c} tensor of {e}.\n");
    let region_of = |src: &str, what: &str, members: &[&str]| -> Op {
        let (_, o) = reference(src, what);
        let op = computing(&o, ClassId::ElementwiseRegions, members[0]).unwrap_or_else(|| panic!("{what}: no region computes `{}`: {:?}", members[0], kinds(&o)));
        let held: Vec<String> = leaves(op.clone()).iter().map(|l| l.out.clone()).collect();
        for m in members {
            assert!(held.iter().any(|h| h == m), "{what}: `{m}` is not in the region of {held:?}");
        }
        op.clone()
    };
    let diamond = |e: &str, r: usize| {
        format!("## Main\n{}{}Let S be A plus B.\nLet P be A hadamard B.\nLet Y be S plus P.\n", decl("A", r, 8, e), decl("B", r, 8, e))
    };
    region_of(&diamond("Real", 1), "a diamond", &["Y", "S", "P"]);
    region_of(&diamond("Int", 1), "an integer diamond", &["Y", "S", "P"]);
    // A scale by a real factor makes real values of integer ones: the integer region ends before it.
    let typed = format!("{}Let W be Y scaled by 0.5.\nLet V be W plus W.\n", diamond("Int", 1));
    let op = region_of(&typed, "an integer diamond, then real values", &["Y", "S", "P"]);
    let held: Vec<String> = leaves(op.clone()).iter().map(|l| l.out.clone()).collect();
    assert!(!held.iter().any(|h| h == "W" || h == "V"), "a region holds values of one element type: {held:?}");
    let later = format!("{}Let Z be P scaled by 2.0.\n", diamond("Real", 1));
    let op = region_of(&later, "a diamond whose intermediate a later op reads", &["Y", "S", "P"]);
    assert!(op.keeps.iter().any(|k| k == "P"), "the region writes `P` out for its later reader: keeps {:?}", op.keeps);
    let biased = format!(
        "## Main\n{}{}{}Let S be A plus R.\nLet P be A hadamard B.\nLet Y be S plus P.\n",
        decl("A", 4, 8, "Real"),
        decl("B", 4, 8, "Real"),
        decl("R", 1, 8, "Real")
    );
    region_of(&biased, "a bias row inside a diamond over a block", &["Y", "S", "P"]);
    let split = format!(
        "## Main\n{}{}{}Let S be A plus B.\nLet P be A hadamard B.\nLet Y be S hadamard P.\nLet N be rmsnorm of Y with weight W.\n\
         Let C be N hadamard A.\nLet D be N hadamard B.\nLet E be C plus D.\n",
        decl("A", 1, 8, "Real"),
        decl("B", 1, 8, "Real"),
        decl("W", 1, 8, "Real")
    );
    let (_, o) = reference(&split, "two regions about a reduction");
    let first = computing(&o, ClassId::ElementwiseRegions, "Y").expect("the region before the reduction");
    let second = computing(&o, ClassId::ElementwiseRegions, "E").expect("the region after the reduction");
    assert!(!std::ptr::eq(first, second), "a reduction stands between the two regions");
    for op in [first, second] {
        assert!(leaves(op.clone()).iter().all(|l| l.kind() != "FusedRow"), "a region holds no reduction: {:?}", kinds(&o));
    }
}

/// A fused op writes every table and state any of its parts writes, however deep it lies: a member a rule built that
/// holds another member — a merged projection holding a query and key's norm, rotation and cache insert — writes the
/// cache its inner part lands a row in (`Op::writes`), so the dataflow facts every rule reads (`facts::writes`) see the
/// write. On every tiny architecture's decode step, prefill chunk and batch under the reference shim, every tensor a
/// leaf of a fused op lands rows or units in, or updates as state, is among the fused op's writes, and some fused op
/// holds another.
#[test]
fn a_fused_op_writes_every_table_and_state_its_parts_write() {
    let mut nested = 0usize;
    for &t in tiny_models::ALL {
        let spec = t.spec(24);
        for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
            let ((source, _, _), _) = program::written(&spec, cut);
            let g = lifted(&source, true, Some(io(cut)));
            let o = optimize(&g, &ReferenceShim, &[]);
            for op in &o.graph.ops {
                let logicaffeine_compile::tensor_graph::graph::Sym::Fused(f) = &op.sym else { continue };
                nested += f.parts.iter().filter(|p| matches!(p.sym, logicaffeine_compile::tensor_graph::graph::Sym::Fused(_))).count();
                let writes: BTreeSet<String> = op.writes().into_iter().map(String::from).collect();
                for leaf in leaves(op.clone()) {
                    let placed = (leaf.dst.row.is_some() || leaf.dst.unit.is_some()).then(|| leaf.dst.buf.clone());
                    for w in placed.into_iter().chain(leaf.state_inputs().into_iter().map(|k| leaf.ins[k].clone())) {
                        assert!(writes.contains(&w), "{} {cut:?}: `{}` holds a {} landing `{w}`, and writes only {writes:?}", t.name(), op.kind(), leaf.kind());
                    }
                }
            }
        }
    }
    assert!(nested > 0, "some fused op holds another");
}

/// A prefill chunk's last layer computes only the row its output reads (PARITY-LAST-LAYER-ROWS): from the attention on,
/// every op of the tail that only that row reads is in one op with the row, meaning what it meant, and no op of it
/// writes the carried state — the cache writes stay outside. On every tiny architecture whose last layer attends; none
/// where the last layer is recurrent (its memory takes every row in order), on a batch (every sequence's row an
/// output), or where every row's logits are read.
#[test]
fn a_row_that_feeds_no_output_and_no_state_is_removed() {
    use logicaffeine_compile::model::spec::Mixer;
    let chunk = Cut::Prefill { rows: 4, ctx: 24 };
    let pruned = |spec: &logicaffeine_compile::model::spec::ModelSpec, cut: Cut, what: &str| -> Vec<Op> {
        let ((source, _, _), _) = program::written(spec, cut);
        let g = lifted(&source, true, Some(io(cut)));
        let o = optimize(&g, &ReferenceShim, &[]);
        same_meaning(what, &g, &o.graph, &o.fired);
        let ops: Vec<Op> = o.graph.ops.iter().filter(|op| realizes(op, &o.graph).contains(&ClassId::LastLayerRows)).cloned().collect();
        for op in &ops {
            for leaf in leaves(op.clone()) {
                for t in leaf.writes() {
                    assert!(g.tensors.get(t).is_none_or(|x| !x.carried), "{what}: the output row's op writes the carried state `{t}`");
                }
            }
            assert!(leaves(op.clone()).iter().any(|l| l.kind() == "Attention"), "{what}: the output row's op starts at the attention");
        }
        ops
    };
    for &t in tiny_models::ALL {
        let spec = t.spec(24);
        let attends = matches!(spec.layers.last().map(|l| &l.mixer), Some(Mixer::Attention(_) | Mixer::Mla(_)));
        let ops = pruned(&spec, chunk, &format!("{} chunk", t.name()));
        assert_eq!(ops.len(), usize::from(attends), "{}: the last layer {} its output row", t.name(), if attends { "computes only" } else { "keeps every row of" });
        assert!(pruned(&spec, Cut::Batch { rows: 2, ctx: 24 }, &format!("{} batch", t.name())).is_empty(), "{}: a batch's every row is an output", t.name());
    }
    let mut recurrent = tiny_models::Tiny::Qwen35.spec(24);
    recurrent.layers.rotate_left(1);
    assert!(matches!(recurrent.layers.last().map(|l| &l.mixer), Some(Mixer::GatedDelta(_))), "the reshaped last layer is recurrent");
    assert!(pruned(&recurrent, chunk, "a recurrent last layer").is_empty(), "a recurrent last layer's memory takes every row");
    let every = "## Main\nLet X be a 4 by 8 tensor of Real.\nLet K be a 8 by 8 tensor of Real.\nLet V be a 8 by 8 tensor of Real.\n\
                 Let W be a 1 by 8 tensor of Real.\nLet H be a 8 by 16 tensor of Real.\nLet pos be 4.\n\
                 Let A be causal attention of X over K and V with 1 heads 1 groups from pos.\nLet S be X plus A.\n\
                 Let N be rmsnorm of S with weight W.\nLet logits be N matmul H.\n";
    let (_, o) = reference(every, "every row's logits read");
    assert!(!o.ops.iter().any(|op| realizes(op, &o).contains(&ClassId::LastLayerRows)), "every row's logits are read: no row is dropped");
}

/// A layer's query and key rotations — two row ops over one projection's heads, by one table at one position — are one
/// op (PARITY-ROPE-QK-ONE-LAUNCH), meaning what they mean, where nothing folds them into their projection first
/// (`qkv_head_epilogue` held): on every tiny architecture whose attention rotates, and reshaped to 4:1, 8:1 and 3:1
/// query to key heads 256, 128 and 64 wide; Qwen3.5's partial rotation in interleaved sections; the key's rotation
/// landing in its row of the cache.
#[test]
fn two_independent_row_ops_over_one_source_fuse_horizontally() {
    use logicaffeine_compile::model::spec::Mixer;
    let reshaped = |t: tiny_models::Tiny, heads: usize, kv_heads: usize, hd: usize| {
        let mut spec = t.spec(24);
        for layer in &mut spec.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                (a.heads, a.kv_heads, a.head_dim, a.value_dim, a.scale) = (heads, kv_heads, hd, hd, 1.0 / (hd as f64).sqrt());
                if let Some(r) = &mut a.rope {
                    r.dims = hd;
                }
            }
        }
        (format!("{} with {heads} query heads over {kv_heads}, {hd} wide", t.name()), spec)
    };
    let mut specs: Vec<(String, logicaffeine_compile::model::spec::ModelSpec)> = tiny_models::ALL
        .iter()
        .filter(|t| !t.name().starts_with("deepseek2"))
        .map(|&t| (t.name().to_string(), t.spec(24)))
        .collect();
    specs.extend([reshaped(tiny_models::Tiny::Qwen3, 4, 1, 256), reshaped(tiny_models::Tiny::Llama, 8, 1, 128), reshaped(tiny_models::Tiny::Gemma3, 3, 1, 64)]);
    let cut = Cut::Decode { ctx: 24 };
    for (what, spec) in specs {
        let ((source, _, _), _) = program::written(&spec, cut);
        let g = lifted(&source, true, Some(io(cut)));
        let o = optimize(&g, &ReferenceShim, &["qkv_head_epilogue"]);
        same_meaning(&what, &g, &o.graph, &o.fired);
        let ropes: Vec<&Op> = g.ops.iter().filter(|op| op.kind() == "Rope").collect();
        let rotating = spec.layers.iter().filter(|l| matches!(&l.mixer, Mixer::Attention(a) if a.rope.is_some())).count();
        assert_eq!(ropes.len(), 2 * rotating, "{what}: a query and a key rotation an attention layer");
        for rope in ropes {
            let op = computing(&o.graph, ClassId::RopeQkOneLaunch, &rope.out).unwrap_or_else(|| panic!("{what}: `{}` rotates apart: {:?}", rope.out, kinds(&o.graph)));
            let turned: Vec<Op> = leaves(op.clone()).into_iter().filter(|l| l.kind() == "Rope").collect();
            assert_eq!(turned.len(), 2, "{what}: one layer's query and key rotate in one op, no more: {:?}", turned.iter().map(|l| &l.out).collect::<Vec<_>>());
            assert!(turned.iter().any(|l| l.dst.row.is_some()), "{what}: the key's rotation lands in its row of the cache");
        }
    }
}

/// Every class whose pattern is a whole block is built on the tiny cut that holds it, under the reference shim, with its
/// matrices as checkpoint rows and as values — and the cut means what it meant.
#[test]
fn every_block_class_is_built_on_the_cut_that_holds_it() {
    let mut missing = Vec::new();
    for &(class, t, kind) in BLOCKS {
        let c = CLASSES.iter().find(|c| c.class == class).expect("a registered class");
        assert!(matches!(c.exemplar, Exemplar::None(_)), "`{}` has an exemplar of its own", c.id);
        let cut = match kind {
            "decode" => Cut::Decode { ctx: 24 },
            _ => Cut::Prefill { rows: 4, ctx: 24 },
        };
        let ((source, _, _), _) = program::written(&t.spec(24), cut);
        for rows in [true, false] {
            let g = lifted(&source, rows, Some(io(cut)));
            let o = optimize(&g, &ReferenceShim, &[]);
            same_meaning(&format!("{} on {} {kind} (rows {rows})", c.id, t.name()), &g, &o.graph, &o.fired);
            if !o.graph.ops.iter().any(|op| realizes(op, &o.graph).contains(&class)) {
                missing.push(format!("{} on {} {kind} (rows {rows})", c.id, t.name()));
            }
        }
    }
    assert!(missing.is_empty(), "{} block classes the shared table does not build:\n{}", missing.len(), missing.join("\n"));
}

/// Each rule's witness that its dataflow forbids it: a second reader of what it would fold, a write between, an observed
/// intermediate — where it must not fire — and its twin with that taken away, where it must.
#[test]
fn every_rule_holds_back_where_its_dataflow_forbids() {
    // Each case with its twin: the same program without what forbids the rule, which the rule fires on.
    let cases = HOLD_BACKS;
    for (rule, why, source, twin) in cases {
        for rows in [true, false] {
            let g = lifted(source, rows, None);
            let o = optimize(&g, &ReferenceShim, &[]);
            assert_eq!(o.fired(rule), 0, "`{rule}` fires on {why}: {:?}", o.fired);
            same_meaning(&format!("{rule}: {why}"), &g, &o.graph, &o.fired);
            let t = lifted(twin, rows, None);
            let ot = optimize(&t, &ReferenceShim, &[]);
            assert!(ot.fired(rule) > 0, "`{rule}` fires once nothing forbids it ({why} taken away): {:?}", ot.fired);
            same_meaning(&format!("{rule}: twin of {why}"), &t, &ot.graph, &ot.fired);
        }
    }
    let ruled: BTreeSet<&str> = rule_names().into_iter().collect();
    for (rule, _, _, _) in cases {
        assert!(ruled.contains(rule), "`{rule}` is a rule of the table");
    }
}
