//! Every chip says what it allows through one trait (benchmarks/FUSIONS_WE_NEED.md, OPT-2; section 5.6).
//!
//! A shim answers every registered class by name — linked whole by a primitive of the chip's own, run as its parts, or
//! refused with the reason — and judges each op's shape the way the chip's link would build it. The shared optimizer
//! commits a fusion only where the chip answers native, so a chip never receives a fused op it cannot run; the
//! resolver (`tensor_graph::link`) then takes every op of a graph to the chip's link, or names every op it cannot.
//!
//!   cargo nextest run -p logicaffeine-tests --test shim_lock

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;
#[path = "common/lane_cuts.rs"]
mod lane_cuts;
#[path = "common/lane_corpus.rs"]
mod lane_corpus;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_cpu::shim::{CpuActivations, CpuCache, CpuIsa, CpuShim};
use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_link::Link;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::compile::compile_to_cuda_forward;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::fuse::{classes_built, optimize, ChipRule, Outcome, Rewriter};
use logicaffeine_compile::tensor_graph::graph::{Access, Dst, Fused, Op, Storage, Strided, Sym, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{allows, Allow, Position, ReferenceShim, Shim, Support};
use logicaffeine_compile::tensor_graph::symbols::ClassId;

use lane_corpus::every_lane_cut;

/// Every shim the compiler declares, under every configuration that changes an answer.
fn shims() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for fuse_quant in [true, false] {
            for batch in [false, true] {
                let knobs = CudaKnobs { activations, fuse_quant, ..CudaKnobs::default() };
                out.push(Box::new(LaneShim { knobs, batch, held: Vec::new() }));
            }
        }
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        out.push(Box::new(SpatialShim { target: Box::leak(Box::new(target)) }));
    }
    for tiny in [tiny_models::Tiny::Qwen35, tiny_models::Tiny::Qwen3] {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny.spec(24))), numerics: EngineNumerics::precise() }));
    }
    for isa in CpuIsa::ALL {
        for activations in [CpuActivations::Values, CpuActivations::OneWord] {
            for cache in CpuCache::ALL {
                for fuse in [true, false] {
                    out.push(Box::new(CpuShim { isa, activations, cache, fuse }));
                }
            }
        }
    }
    out.push(Box::new(ReferenceShim));
    out
}

/// The shims that answer by a table of their own, by type, with the file that declares each: every `impl Shim for`
/// in the compiler must be one of them, or the reference shim.
const SHIMS: &[(&str, &str)] = &[
    ("LaneShim", "codegen_cuda/lane_shim.rs"),
    ("ForwardShim", "codegen_cuda/forward_shim.rs"),
    ("SpatialShim", "spatial/shim.rs"),
    ("EngineShim", "spatial/decode_engine/shim.rs"),
    ("CpuShim", "codegen_cpu/shim.rs"),
];

/// The reference shim answers no class by name: everything is linked whole by its definition.
const REFERENCE: (&str, &str) = ("ReferenceShim", "tensor_graph/shim.rs");

/// A chip as if it linked some classes answers the rest as that chip does: it adds no table of its own.
const LINKING: (&str, &str) = ("Linking", "tensor_graph/shim.rs");

fn compiler_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src")
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The body of the first `fn supports` after `impl Shim for <ty>` in `text`, through its closing brace.
fn supports_body<'t>(text: &'t str, ty: &str) -> Option<&'t str> {
    let head = format!("impl Shim for {ty}");
    let at = text.match_indices(&head).map(|(k, _)| k).find(|&k| matches!(text[k + head.len()..].chars().next(), Some(' ' | '<')))?;
    let start = at + text[at..].find("fn supports(")?;
    let open = start + text[start..].find('{')?;
    let mut depth = 0usize;
    for (k, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..open + k + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The arms of a match that answer every class not named before them: a wildcard, or a bare binding.
fn catch_all_arms(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| {
            let Some(pattern) = l.split("=>").next().filter(|_| l.contains("=>")) else { return false };
            let p = pattern.trim();
            p == "_" || (!p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()))
        })
        .map(String::from)
        .collect()
}

fn a_sentence(why: &str) -> bool {
    why.split_whitespace().count() >= 4
}

/// Every shim answers every registered class by name, by a match with no catch-all arm — so a class added to the
/// registry fails the build until each shim answers it — and every refusal says why. A shim declared anywhere in the
/// compiler is one this lock knows, and the classes the shared table builds are ones some chip links whole.
#[test]
fn every_shim_answers_every_class() {
    let mut files = Vec::new();
    rust_sources(&compiler_src(), &mut files);
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for (k, _) in text.match_indices("impl Shim for ") {
            let ty: String = text[k + "impl Shim for ".len()..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            declared.insert(ty);
        }
    }
    let known: BTreeSet<String> = SHIMS.iter().chain([&REFERENCE, &LINKING]).map(|(t, _)| t.to_string()).collect();
    assert_eq!(declared, known, "every shim in the compiler answers in this lock, and every one this lock names exists");
    for (ty, file) in SHIMS {
        let text = std::fs::read_to_string(compiler_src().join(file)).unwrap();
        let body = supports_body(&text, ty).unwrap_or_else(|| panic!("{ty} answers by `fn supports` in {file}"));
        assert!(body.contains("match class {"), "{ty} answers by a match over the class");
        let arms = catch_all_arms(body);
        assert!(arms.is_empty(), "{ty} answers every class by name, never by a catch-all arm: {arms:?}");
    }
    let text = std::fs::read_to_string(compiler_src().join(REFERENCE.1)).unwrap();
    let body = supports_body(&text, REFERENCE.0).expect("the reference shim's answer");
    assert!(body.contains("Support::Native") && !body.contains("Refuse"), "the reference shim links everything whole");
    let body = supports_body(&text, LINKING.0).expect("the linking shim's answer");
    assert!(body.contains("self.base.supports(class)") && !body.contains("Refuse"), "a chip as if it linked some classes answers the rest as the chip does");

    for shim in shims() {
        for &class in ClassId::ALL {
            if let Support::Refuse(why) = shim.supports(class) {
                assert!(a_sentence(why), "{} refuses `{class}` without saying why: `{why}`", shim.name());
            }
        }
    }
    let built: BTreeSet<ClassId> = classes_built().values().flat_map(|c| c.iter().copied()).collect();
    let book = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/FUSIONS_WE_NEED.md")).unwrap();
    for class in &built {
        let linked = shims().iter().any(|s| s.name() != "reference" && s.supports(*class) == Support::Native);
        match AWAITING_A_LINK.iter().find(|(c, _)| c == class) {
            Some((_, ticket)) => {
                assert!(!linked, "`{class}` is linked whole by a chip now: take it off AWAITING_A_LINK");
                assert!(book.contains(&format!("#### {ticket} ")), "`{class}` waits on {ticket}, a ticket of the book");
            }
            None => assert!(linked, "the shared table builds `{class}`, which no chip links whole"),
        }
    }
    for (class, _) in AWAITING_A_LINK {
        assert!(built.contains(class), "`{class}` awaits a link, but the shared table does not build it");
    }
}

/// The classes the shared table builds — each defined by its parts and checked by the evaluator on its exemplar — that
/// no chip links whole yet, each with the book's ticket that links it on CUDA. A class the shared table newly builds
/// joins with that ticket; a chip that links one takes it off.
const AWAITING_A_LINK: &[(ClassId, &str)] = &[
    (ClassId::LastLayerRows, "PARITY-LAST-LAYER-ROWS"),
];

/// Every tiny architecture's cuts, compiled by the lane under every numerics, each with the lane shim it compiles
/// under.
fn lane_corpus() -> Vec<(String, TensorGraph, LaneShim)> {
    every_lane_cut().into_iter().map(|c| (c.what, c.g, c.shim)).collect()
}

/// Every tensor probe the front end lifts, as a graph of values.
fn probe_corpus() -> Vec<(String, String, TensorGraph)> {
    tensor_probes::OP_PROBES
        .iter()
        .filter_map(|p| {
            let rp = recognize_stages(p.program).ok()?;
            let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).ok()?;
            Some((p.op.to_string(), p.program.to_string(), g))
        })
        .collect()
}

/// What the optimizer hands a chip it can run: every graph a chip resolves lifted still resolves once the shared table
/// has run under that chip's shim, and every fused op the table built is one the chip links whole.
#[test]
fn every_native_answer_links() {
    let mut checked = 0usize;
    let mut check = |what: &str, g: &TensorGraph, shim: &dyn Shim| {
        if resolve(g, shim).is_err() {
            return;
        }
        let o = optimize(g, shim, &[]);
        if let Err(unresolved) = resolve(&o.graph, shim) {
            let lines: Vec<String> = unresolved.iter().map(|u| u.to_string()).collect();
            panic!("{what} under {}: the optimized graph leaves ops the chip cannot run:\n{}", shim.name(), lines.join("\n"));
        }
        for op in &o.graph.ops {
            if let Sym::Fused(f) = &op.sym {
                assert_eq!(allows(shim, op, &o.graph), Allow::Native, "{what} under {}: the table built a `{}` the chip does not link whole", shim.name(), f.class);
            }
        }
        checked += 1;
    };
    for (what, g, shim) in lane_corpus() {
        check(&what, &g, &shim);
    }
    for (op, _, g) in probe_corpus() {
        for shim in shims() {
            check(&format!("probe {op}"), &g, shim.as_ref());
        }
    }
    assert!(checked >= 9 * 3 * 4, "the lock judges every lane cut and the probes the forward runs: {checked}");
}

/// Whether every member a fused op holds, at every depth, is a lifted op or a class the lane links whole: an op the
/// shared table could hand the lane.
fn lane_shaped(op: &Op, shim: &LaneShim) -> bool {
    match &op.sym {
        Sym::Fused(f) => f.parts.iter().all(|p| !matches!(&p.sym, Sym::Fused(inner) if shim.supports(inner.class) != Support::Native) && lane_shaped(p, shim)),
        _ => true,
    }
}

/// A fused op and every fused op it holds, at every depth, each with its depth.
fn nested(op: &Op, depth: usize) -> Vec<(&Op, usize)> {
    match &op.sym {
        Sym::Fused(f) => std::iter::once((op, depth)).chain(f.parts.iter().flat_map(|p| nested(p, depth + 1))).collect(),
        _ => Vec::new(),
    }
}

/// OPT-2: the lane's shim is its link. Over every lane cut, fused by the whole shared table so that every class it can
/// build is built, every fused op the lane could be handed — a fold's members at every depth as much as the fold — is
/// `Native` under the lane's shim exactly when the lane's link builds it whole: a class the shim answers `Native` for has
/// a launch, and every op of it the kernels run (`fits`) links; a class the shim decomposes or refuses has no launch.
/// Nothing built escapes: every class a cut builds is judged on it, the only ones passed over being ops that hold a
/// member the lane does not link whole, which the lane is never handed as they stand — and a fold's members are judged
/// where the fold is not handed whole (a class judged only inside a fold is met).
#[test]
fn the_lane_shim_is_its_link() {
    let mut native: BTreeSet<ClassId> = BTreeSet::new();
    let mut apart: BTreeSet<ClassId> = BTreeSet::new();
    let (mut at_top, mut inside): (BTreeSet<ClassId>, BTreeSet<ClassId>) = (BTreeSet::new(), BTreeSet::new());
    for c in every_lane_cut() {
        let all = optimize(&c.g, &ReferenceShim, &[]).graph;
        let link = Link { g: &all, constants: &c.constants, shim: &c.shim, io: &c.io };
        let (mut built, mut judged): (BTreeSet<ClassId>, BTreeSet<ClassId>) = (BTreeSet::new(), BTreeSet::new());
        for (op, depth) in all.ops.iter().flat_map(|o| nested(o, 0)) {
            let Sym::Fused(f) = &op.sym else { continue };
            built.insert(f.class);
            if !lane_shaped(op, &c.shim) {
                continue;
            }
            judged.insert(f.class);
            if depth == 0 { at_top.insert(f.class) } else { inside.insert(f.class) };
            let allowed = allows(&c.shim, op, &all) == Allow::Native;
            let linked = link.whole(op);
            assert_eq!(
                allowed,
                linked.is_ok(),
                "{}: the lane's shim answers {} for a `{}`{}, and its link {} it: {:?}",
                c.what,
                if allowed { "Native" } else { "other than Native" },
                f.class,
                if depth == 0 { String::new() } else { format!(" held {depth} deep") },
                if linked.is_ok() { "builds" } else { "does not build" },
                linked.err()
            );
            if allowed {
                native.insert(f.class);
            } else {
                apart.insert(f.class);
            }
        }
        // A class built and never judged is one whose every op holds a member the lane does not link whole.
        for class in built.difference(&judged) {
            for (op, _) in all.ops.iter().flat_map(|o| nested(o, 0)) {
                let Sym::Fused(f) = &op.sym else { continue };
                if f.class == *class {
                    assert!(
                        f.parts.iter().any(|p| matches!(&p.sym, Sym::Fused(inner) if c.shim.supports(inner.class) != Support::Native)),
                        "{}: a `{class}` the lane could be handed is never judged",
                        c.what
                    );
                }
            }
        }
    }
    assert!(native.len() >= 10 && apart.len() >= 10, "both answers are met over the lane cuts: native {native:?}, apart {apart:?}");
    let only_inside: BTreeSet<&ClassId> = inside.difference(&at_top).collect();
    assert!(!only_inside.is_empty(), "no class is judged only inside a fold, so the folds' members are not what the judgement reaches: inside {inside:?}");
}

/// The forward's shim is its own compile: a probe the older forward compiles resolves under its shim, and a probe it
/// refuses is refused by the shim with the forward's own words.
#[test]
fn the_forward_shim_is_its_compile() {
    let mut both = (0usize, 0usize);
    for (op, src, g) in probe_corpus() {
        let compiled = compile_to_cuda_forward(&src);
        let resolved = resolve(&g, &ForwardShim { kind: Q8MatKind::Dequant });
        match (&compiled, &resolved) {
            (Ok(_), Ok(_)) => both.0 += 1,
            (Err(e), Err(unresolved)) => {
                assert!(unresolved.iter().all(|u| e.contains(&u.why)), "probe {op}: the shim refuses in the forward's words: {e}\n{unresolved:?}");
                both.1 += 1;
            }
            _ => panic!("probe {op}: the forward {} it, the shim {} it", if compiled.is_ok() { "compiles" } else { "refuses" }, if resolved.is_ok() { "resolves" } else { "refuses" }),
        }
    }
    assert!(both.0 >= 5 && both.1 >= 5, "the probes hold both answers: {both:?}");
}

/// Every refusal names its reason: a class refused, an op refused, a part refused inside a class run as its parts.
#[test]
fn every_refusal_names_its_reason() {
    let mut refusals = 0usize;
    for (op, _, g) in probe_corpus() {
        for shim in shims() {
            if let Err(unresolved) = resolve(&g, shim.as_ref()) {
                for u in unresolved {
                    assert!(a_sentence(&u.why), "probe {op} under {}: op {} ({}) is refused without a reason: `{}`", shim.name(), u.op, u.kind, u.why);
                    assert_eq!(u.kind, g.ops[u.op].kind(), "probe {op}: a refusal names the op it refuses");
                    refusals += 1;
                }
            }
        }
    }
    assert!(refusals > 0, "the probes meet refusals");
}

/// A lifted op: a per-head product reading a part of its input at a place no 32-value block starts.
fn misaligned_head_product(g: &TensorGraph) -> Op {
    let op = g.ops.iter().find(|o| matches!(o.prim().map(|p| p.kind()), Some("HeadMatmul"))).expect("a per-head product").clone();
    Op { access: vec![Access::Strided(Strided { pitch: 64, offset: 8, width: 16 }), Access::Whole], ..op }
}

/// A class a chip runs as its parts is refused whole when a part is, naming the class, the part and the part's reason.
#[test]
fn a_decomposed_class_whose_part_is_refused_is_refused_whole_by_name() {
    let corpus = lane_corpus();
    let (what, g, shim) = corpus.iter().find(|(_, g, _)| g.ops.iter().any(|o| o.prim().is_some_and(|p| p.kind() == "HeadMatmul"))).expect("a cut with a per-head product");
    assert_eq!(shim.supports(ClassId::DualNorm), Support::Decompose, "the lane runs two norms of one source apart");
    let part = misaligned_head_product(g);
    let Allow::Refuse(part_why) = allows(shim, &part, g) else { panic!("{what}: the lane refuses a per-head product off the blocks") };
    let fused = Op { sym: Sym::Fused(Fused { class: ClassId::DualNorm, parts: vec![part.clone()] }), ..part.clone() };
    match allows(shim, &fused, g) {
        Allow::Refuse(why) => {
            assert!(why.contains("dual_norm") && why.contains("HeadMatmul") && why.contains(&part_why), "{what}: the refusal names the class, the part and its reason: {why}");
        }
        other => panic!("{what}: a class run as parts with a refused part answers {other:?}"),
    }
    let whole = Op { sym: Sym::Fused(Fused { class: ClassId::Mla, parts: vec![part] }), dst: Dst::to(&fused.out), ..fused };
    assert!(matches!(allows(shim, &whole, g), Allow::Refuse(_)), "{what}: a class linked whole is judged by its shape, and a part off the blocks fails it");
}

/// A chip's own rules link into the table where their positions say — before a shared rule, after one, or after
/// them all — and each can be held: held, it fires nowhere and is reported held where it would have fired.
#[test]
fn a_chips_own_rules_run_where_they_are_linked_and_can_be_held() {
    struct Probe;
    impl Shim for Probe {
        fn name(&self) -> String {
            "probe".to_string()
        }
        fn supports(&self, _: ClassId) -> Support {
            Support::Native
        }
        fn fits(&self, _: &Op, _: &TensorGraph) -> Result<(), String> {
            Ok(())
        }
        fn rules(&self) -> Vec<(Position, ChipRule<'_>)> {
            let at = |name: &'static str| -> Box<dyn Fn(&mut Rewriter) + '_> {
                Box::new(move |r: &mut Rewriter| {
                    let first = r.s.live().map(|(k, _)| k).next();
                    if let Some(k) = first {
                        r.record(name, ClassId::Mla, k);
                    }
                })
            };
            vec![
                (Position::Before("add_norm"), ChipRule { name: "before_add_norm", builds: &[ClassId::Mla], apply: at("before_add_norm") }),
                (Position::After("gemv_bias"), ChipRule { name: "after_gemv_bias", builds: &[ClassId::Mla], apply: at("after_gemv_bias") }),
                (Position::End, ChipRule { name: "at_the_end", builds: &[ClassId::Mla], apply: at("at_the_end") }),
            ]
        }

        fn numerics(&self) -> Option<logicaffeine_compile::tensor_graph::numerics::Contract> {
            ReferenceShim.numerics()
        }
    }
    let names = ["before_add_norm", "after_gemv_bias", "at_the_end"];
    let (_, _, g) = probe_corpus().into_iter().next().expect("a probe");
    let o = optimize(&g, &Probe, &[]);
    let ran: Vec<&str> = o.fired.iter().map(|f| f.rule).filter(|r| names.contains(r)).collect();
    assert_eq!(ran, names, "each runs where it is linked, in the table's order");
    let order: Vec<&str> = o.report.decisions.iter().map(|d| d.rule).filter(|r| names.contains(r) || *r == "add_norm" || *r == "gemv_bias").collect();
    let place = |r: &str| order.iter().position(|x| *x == r);
    if let (Some(a), Some(b)) = (place("before_add_norm"), place("add_norm")) {
        assert!(a < b, "`before_add_norm` decides before add_norm");
    }
    for held in names {
        let h = optimize(&g, &Probe, &[held]);
        assert_eq!(h.fired(held), 0, "`{held}` held fires nowhere");
        assert!(h.report.of(held).iter().all(|d| d.outcome == Outcome::Held) && !h.report.of(held).is_empty(), "`{held}` held is reported held");
        let others: Vec<&str> = h.fired.iter().map(|f| f.rule).filter(|r| names.contains(r)).collect();
        assert_eq!(others, names.iter().copied().filter(|n| *n != held).collect::<Vec<_>>(), "holding `{held}` holds nothing else");
    }
}

// ── The book's fusion map is the shims' answers (ONE-PROGRAM-FUSION-MAP-LOCK) ─────────────────────────────

/// One row of the book's section 3.4: the class, whether its box is ticked, and each chip family's line — ticked or
/// not, and what it says.
struct MapRow {
    id: String,
    ticked: bool,
    families: Vec<(String, bool, String)>,
}

/// A checklist item's box and the text after it.
fn tick(rest: &str) -> Option<(bool, &str)> {
    rest.strip_prefix("[x] ").map(|r| (true, r)).or_else(|| rest.strip_prefix("[ ] ").map(|r| (false, r)))
}

/// The book's section 3.4, row by row.
fn fusion_map() -> Vec<MapRow> {
    let book = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/FUSIONS_WE_NEED.md")).expect("the book");
    let start = book.find("### 3.4 Every fusion on every chip").expect("section 3.4");
    let end = book[start..].find("### 3.5").map_or(book.len(), |e| start + e);
    let mut rows: Vec<MapRow> = Vec::new();
    for line in book[start..end].lines() {
        if let Some((ticked, rest)) = line.strip_prefix("- ").and_then(tick) {
            let id = rest.strip_prefix("**`").and_then(|r| r.split_once('`')).map(|(id, _)| id.to_string());
            rows.push(MapRow { id: id.unwrap_or_else(|| panic!("a row of section 3.4 names its class: {line}")), ticked, families: Vec::new() });
        } else if let Some((ticked, rest)) = line.strip_prefix("  - ").and_then(tick) {
            let (family, said) = rest.split_once(": ").unwrap_or_else(|| panic!("a family line names its family: {line}"));
            rows.last_mut().expect("a family line follows its row").families.push((family.to_string(), ticked, said.to_string()));
        }
    }
    rows
}

/// The chip families the map names, in its order, each with the shims that answer for it by name: the CUDA lane at its
/// default knobs, every spatial target, the D5005 engine. A family no shim answers for yet has none, and its boxes stay
/// open.
fn families() -> Vec<(&'static str, Vec<(&'static str, Box<dyn Shim>)>)> {
    fn spatial(t: SpatialTarget) -> Box<dyn Shim> {
        Box::new(SpatialShim { target: Box::leak(Box::new(t)) })
    }
    vec![
        ("CUDA", vec![("the lane", Box::new(LaneShim { knobs: CudaKnobs::default(), batch: false, held: Vec::new() }) as Box<dyn Shim>)]),
        ("CPU", Vec::new()),
        ("AMD, Intel", Vec::new()),
        (
            "Spatial chips (CUDA persistent, Tensix, RDU, fabric)",
            vec![
                ("CUDA persistent", spatial(SpatialTarget::rtx3060())),
                ("Tensix", spatial(SpatialTarget::wormhole())),
                ("RDU", spatial(SpatialTarget::sn50_rdu())),
                ("fabric", spatial(SpatialTarget::sky_fabric(64, 64))),
            ],
        ),
        ("D5005", vec![("the engine", Box::new(EngineShim { spec: Box::leak(Box::new(tiny_models::Tiny::Qwen35.spec(24))), numerics: EngineNumerics::precise() }) as Box<dyn Shim>)]),
    ]
}

fn said(s: Support) -> &'static str {
    match s {
        Support::Native => "native",
        Support::Decompose => "parts",
        Support::Refuse(_) => "refused",
    }
}

/// Section 3.4 says what the code does: a family's box is ticked exactly when every shim answering for it links the class
/// whole, its line names each shim's answer where they differ, and the CUDA line names the lane's answer with
/// `fuse_quant` off where that differs; a row's box is ticked exactly when every family's is.
#[test]
fn the_books_fusion_map_is_every_shims_answers() {
    let families = families();
    let off = LaneShim { knobs: CudaKnobs { fuse_quant: false, ..CudaKnobs::default() }, batch: false, held: Vec::new() };
    let mut problems = Vec::new();
    for row in fusion_map() {
        let Some(class) = logicaffeine_compile::tensor_graph::symbols::CLASSES.iter().find(|c| c.id == row.id).map(|c| c.class) else {
            problems.push(format!("`{}` is no registered class", row.id));
            continue;
        };
        let named: Vec<&str> = row.families.iter().map(|(f, _, _)| f.as_str()).collect();
        let expected: Vec<&str> = families.iter().map(|(f, _)| *f).collect();
        if named != expected {
            problems.push(format!("`{}` names the families {named:?}, not {expected:?}", row.id));
            continue;
        }
        for ((family, ticked, line), (_, shims)) in row.families.iter().zip(&families) {
            let answers: Vec<(&str, Support)> = shims.iter().map(|(name, s)| (*name, s.supports(class))).collect();
            let native = !answers.is_empty() && answers.iter().all(|(_, a)| *a == Support::Native);
            if *ticked != native {
                let each: Vec<String> = answers.iter().map(|(n, a)| format!("{n}: {}", said(*a))).collect();
                problems.push(format!("`{}` / {family}: the box is {} and the shims answer {each:?}", row.id, if *ticked { "ticked" } else { "open" }));
            }
            let distinct: BTreeSet<&str> = answers.iter().map(|(_, a)| said(*a)).collect();
            if distinct.len() > 1 {
                for (name, a) in &answers {
                    if !line.contains(&format!("{name}: {}", said(*a))) {
                        problems.push(format!("`{}` / {family}: the shims differ, and the line does not say `{name}: {}`", row.id, said(*a)));
                    }
                }
            }
            if *family == "CUDA" && off.supports(class) != shims[0].1.supports(class) && !line.contains(&format!("{} with `fuse_quant` off", said(off.supports(class)))) {
                problems.push(format!("`{}` / CUDA: the lane answers {} with `fuse_quant` off, and the line does not say so", row.id, said(off.supports(class))));
            }
        }
        let all = row.families.iter().all(|(_, t, _)| *t);
        if row.ticked != all {
            problems.push(format!("`{}`: the row's box is {} and its families' are {}", row.id, if row.ticked { "ticked" } else { "open" }, if all { "all ticked" } else { "not all ticked" }));
        }
    }
    assert!(problems.is_empty(), "section 3.4 says what the shims do not ({}):\n{}", problems.len(), problems.join("\n"));
}

/// Every class the ledger tracks has a row in the map, once.
#[test]
fn every_class_in_the_ledger_has_a_row_in_the_map() {
    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/fusion_parity.json")).expect("the ledger"))
            .expect("the ledger's JSON");
    let ids: Vec<String> = fusion_map().into_iter().map(|r| r.id).collect();
    let rows: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(rows.len(), ids.len(), "a class has one row");
    let missing: Vec<&str> = ledger["classes"]
        .as_array()
        .expect("the ledger's classes")
        .iter()
        .map(|c| c["id"].as_str().expect("an id"))
        .filter(|id| !rows.contains(id))
        .collect();
    assert!(missing.is_empty(), "the ledger's classes with no row in section 3.4: {missing:?}");
}
