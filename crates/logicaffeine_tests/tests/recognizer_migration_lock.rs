//! recognizer_migration_lock — the D9 golden: the CUDA forward's stage walk is a FUNCTION OF THE PROGRAM, and
//! moving its recognizers to their shared home (`tensor_recognize::qwen`), and then the forward onto the one IR
//! (MIGRATE-2), must not move it by one byte.
//!
//! The forward once walked a program's top-level statements with recognizers of its own, lifting each into a stage.
//! Before those recognizers were shared with the spatial front end, this lock pinned the walk over a fixed corpus —
//! the tiny Qwen3.8 forward, the tiny Qwen3-30B forward, the hand-authored tiny demo, and every probe program — as a
//! checked-in golden (`fixtures/cuda_stages_golden.txt`, one projected line per stage: variant, operand names,
//! dims), generated against the code as it stood before the move. Since MIGRATE-2 the forward lowers the one IR's lift
//! of the shared front end's walk, and the golden is that walk seen through the forward's lens: the stages the forward
//! has kernels for, in the forward's projection. It is regenerated only under `LOGOS_UPDATE_FIXTURES=1`; otherwise a
//! difference is a failure that prints the first divergent line.
//!
//! The golden pins the walk AS IT WAS, warts included — that is the point of a migration golden. Two of them are worth
//! knowing: the forward's `Silu` stage read its counted loop's `<=` bound as `<` (its `n` is one more than the element
//! count), and the whole `x / (1 + e^…)` skeleton claimed it, so the gelu probe's block appears as a `Silu` stage. The
//! lens ([`lens`]) states both as the exact relation between the shared specs and the golden's lines.
//!
//! ★ MUTATION PROBE: drop the lens's `+ 1` on a unary map's count → every `Silu` line of the golden moves and
//! `the_cuda_stage_sequence_is_unchanged_by_the_move` fails naming the line.
//! ★ MUTATION PROBE: remove `attention` from the spatial walk's `RECOGNIZER_ORDER` → every attention layer leaves the
//! walk and the golden moves.
//!
//!   cargo nextest run -p logicaffeine-tests --test recognizer_migration_lock
#![cfg(feature = "codegen")]

use std::path::PathBuf;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use logicaffeine_compile::codegen_cuda::qwen_emit::{forward_graph, gen_qwen30b_lg, ForwardError, Q8MatKind, Qwen30bCfg, StageRefusal};
use logicaffeine_compile::intern::{Interner, Symbol};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages as spatial_stages, TensorOpSpec};
use logicaffeine_compile::tensor_graph::fuse::leaves;
use logicaffeine_compile::tensor_recognize::qwen::SiluSpec;
use logicaffeine_compile::ui_bridge::with_parsed_program;

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;
use tensor_probes::OP_PROBES;

/// A structurally faithful Qwen3-30B-A3B miniature: two MoE layers at toy widths, every op of the real
/// block (`Qwen30bCfg` ships only the real shape).
fn qwen30b_tiny() -> Qwen30bCfg {
    Qwen30bCfg {
        d: 8,
        dff: 4,
        ne: 4,
        k: 2,
        layers: 2,
        h: 2,
        kv: 1,
        hd: 4,
        vocab: 16,
        rope_theta: 1.0e6,
        rms_eps: 1e-6,
        attn_split: 8,
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_path() -> PathBuf {
    manifest_dir().join("tests/fixtures/cuda_stages_golden.txt")
}

fn demo_program() -> String {
    let path = manifest_dir().join("../../benchmarks/silicon/qwen_tiny_demo.lg");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The corpus the golden covers, in a fixed order: the two authored forwards at their tiny configs, the
/// hand-authored demo, then every probe program (pending sugars included — a probe the parser refuses is
/// pinned as `!parse`, so the day its sugar lands the golden moves visibly).
fn corpus() -> Vec<(String, String)> {
    let mut v = vec![
        (
            "qwen38_tiny".to_string(),
            gen_qwen38_lg(&Qwen38Cfg::tiny(), "/w").0,
        ),
        ("qwen_tiny_demo.lg".to_string(), demo_program()),
        (
            "qwen30b_tiny".to_string(),
            gen_qwen30b_lg(&qwen30b_tiny(), "/w").0,
        ),
    ];
    for p in OP_PROBES {
        v.push((format!("probe:{}", p.op), p.program.to_string()));
    }
    v
}

/// One lifted op through the forward's lens: its projection — the stage variant, its operand NAMES (never symbol ids),
/// its dims — or `None` for a kind the forward has no stage for (sigmoid, soft cap, decay, causal conv, gated delta,
/// l2norm, swiglu, full attention, row gather, rows gather, argmax, set row, and the integer requant, clamp, ReLU and
/// LUT attention). The forward's two conventions are stated here and nowhere else in the test: a unary map's `n` is its
/// `<=` bound read as `<` (one past the count), and the gelu block — the same `x / (1 + e^…)` skeleton — is a `Silu`
/// stage to it.
fn lens(op: &TensorOpSpec, n: &dyn Fn(Symbol) -> String) -> Option<String> {
    let opt = |s: Option<Symbol>| s.map(n).unwrap_or_else(|| "-".to_string());
    Some(match op {
        TensorOpSpec::Matmul(m) => format!("Matmul out={} a={} b={} m={} k={} n={}", n(m.c), n(m.a), n(m.b), m.m, m.k, m.n),
        TensorOpSpec::Elementwise(e) => format!("Elementwise out={} a={} b={} op={:?} n={}", n(e.c), n(e.a), n(e.b), e.op, e.n),
        TensorOpSpec::FusedRow(r) => format!(
            "FusedRow out={} x={} reduce={} rows={} cols={} weight={} residual={} bias={}",
            n(r.y),
            n(r.x),
            r.reduce.name(),
            r.rows,
            r.cols,
            opt(r.weight),
            opt(r.residual),
            opt(r.bias)
        ),
        TensorOpSpec::Transpose(t) => format!("Transpose out={} x={} rows={} cols={}", n(t.out), n(t.x), t.rows, t.cols),
        TensorOpSpec::Silu(u) => format!("Silu out={} x={} n={}", n(u.out), n(u.x), u.n + 1),
        TensorOpSpec::Gelu(g) => format!("Silu out={} x={} n={}", n(g.out), n(g.x), g.n + 1),
        TensorOpSpec::Rope(r) => format!("Rope out={} q={} rc={} rs={} h={} half={}", n(r.out), n(r.q), n(r.rc), n(r.rs), r.h, r.half),
        TensorOpSpec::Moe(m) => format!(
            "Moe out={} x={} wr={} wg={} wu={} wd={} d={} dff={} ne={} k={}",
            n(m.out),
            n(m.x),
            n(m.wr),
            n(m.wg),
            n(m.wu),
            n(m.wd),
            m.d,
            m.dff,
            m.ne,
            m.k
        ),
        TensorOpSpec::Attention(a) => format!(
            "Attention out={} q={} kc={} vc={} h={} kv={} hd={} l={}",
            n(a.out),
            n(a.q),
            n(a.kc),
            n(a.vc),
            a.h,
            a.kv,
            a.hd,
            a.l
        ),
        TensorOpSpec::Sigmoid(_)
        | TensorOpSpec::Softcap(_)
        | TensorOpSpec::Scale(_)
        | TensorOpSpec::HeadSlice(_)
        | TensorOpSpec::Join(_)
        | TensorOpSpec::HeadMatmul(_)
        | TensorOpSpec::Decay(_)
        | TensorOpSpec::L2Norm(_)
        | TensorOpSpec::CausalConv(_)
        | TensorOpSpec::GatedDelta(_)
        | TensorOpSpec::SwiGlu(_)
        | TensorOpSpec::FullAttention(_)
        | TensorOpSpec::RowGather(_)
        | TensorOpSpec::RowsGather(_)
        | TensorOpSpec::ArgMax(_)
        | TensorOpSpec::SetRow(_)
        | TensorOpSpec::IntRequant(_)
        | TensorOpSpec::IntClamp(_)
        | TensorOpSpec::Relu(_)
        | TensorOpSpec::LutAttention(_) => return None,
    })
}

/// The shared front end's walk of one program through the forward's lens. `Err` when the program does not parse.
fn cuda_walk(source: &str) -> Result<Vec<String>, String> {
    with_parsed_program(source, |parsed, _| parsed.map(|_| ()))?;
    let rp = spatial_stages(source).unwrap_or_else(|e| panic!("the front end refused a program that parses: {e:?}"));
    Ok(rp.stages.iter().filter_map(|s| lens(&s.op, &|sym| rp.name(sym).to_string())).collect())
}

fn render(corpus: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, source) in corpus {
        match cuda_walk(source) {
            Ok(lines) => {
                out.push_str(&format!("## {name} ({} stages)\n", lines.len()));
                for l in lines {
                    out.push_str(&l);
                    out.push('\n');
                }
            }
            Err(_) => out.push_str(&format!("## {name}\n!parse\n")),
        }
        out.push('\n');
    }
    out
}

fn update_requested() -> bool {
    std::env::var("LOGOS_UPDATE_FIXTURES")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[test]
fn the_cuda_stage_sequence_is_unchanged_by_the_move() {
    let got = render(&corpus());
    let path = fixture_path();
    if update_requested() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("no golden at {} ({e}) — generate it ONCE, against the pre-move code, with LOGOS_UPDATE_FIXTURES=1", path.display())
    });
    if got != want {
        let (gl, wl): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
        let at = gl
            .iter()
            .zip(wl.iter())
            .position(|(g, w)| g != w)
            .unwrap_or(gl.len().min(wl.len()));
        let section = wl[..at.min(wl.len())]
            .iter()
            .rev()
            .find(|l| l.starts_with("## "))
            .unwrap_or(&"(start)");
        panic!(
            "the CUDA stage walk moved (line {} of the golden, in {section}):\n  golden: {}\n  now:    {}\n\
             Every line of fixtures/cuda_stages_golden.txt is the CUDA lane's behaviour before the move; a change here is a \
             behaviour change of the orchestrator, not a refactor. If it is intended, regenerate with LOGOS_UPDATE_FIXTURES=1 and say why.",
            at + 1,
            wl.get(at).unwrap_or(&"<end>"),
            gl.get(at).unwrap_or(&"<end>"),
        );
    }
}

/// TEETH FOR THE GOLDEN ITSELF: it names every corpus program, the authored forwards are not empty walks,
/// and the projection carries the dims (two stages that differ only in a count project differently) — so
/// an empty or dims-blind fixture cannot pass as a pin.
#[test]
fn the_golden_pins_every_corpus_program_and_carries_the_dims() {
    let corpus = corpus();
    let text = render(&corpus);
    for (name, _) in &corpus {
        assert!(
            text.contains(&format!("## {name}")),
            "the golden must name `{name}`"
        );
    }
    let stages_of = |name: &str| -> usize {
        let head = format!("## {name} (");
        let line = text
            .lines()
            .find(|l| l.starts_with(&head))
            .unwrap_or_else(|| panic!("`{name}` has no stage header — did it stop parsing?"));
        line[head.len()..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(
        stages_of("qwen38_tiny") >= 4 * 10,
        "the tiny Qwen3.8 walk lifts dozens of stages, got {}",
        stages_of("qwen38_tiny")
    );
    assert!(
        stages_of("qwen30b_tiny") >= 2 * 8,
        "the tiny Qwen3-30B walk lifts every layer's stages, got {}",
        stages_of("qwen30b_tiny")
    );
    assert_eq!(
        stages_of("probe:moe"),
        1,
        "the moe probe is exactly one Moe stage"
    );

    let mut interner = Interner::new();
    let (x, y) = (interner.intern("x"), interner.intern("y"));
    let name = |s: Symbol| interner.resolve(s).to_string();
    let four = lens(&TensorOpSpec::Silu(SiluSpec { x, out: y, n: 4 }), &name).unwrap();
    let five = lens(&TensorOpSpec::Silu(SiluSpec { x, out: y, n: 5 }), &name).unwrap();
    assert_ne!(
        four, five,
        "the projection must carry the count, or the golden is blind to it"
    );
    assert!(
        four.contains("out=y") && four.contains("x=x"),
        "the projection names operands by name: {four}"
    );
}

/// The forward lowers what the golden shows: for every corpus program it compiles, the lifted ops of its graph — the
/// shared table's fused ops opened into their members, a quantize aside — are each a stage the forward has a kernel
/// for, and through the lens they ARE the golden's lines for that program, in order. Of the programs the golden shows
/// lines for, the forward refuses exactly three, each by the kinds it has no kernel for: the tiny Qwen3.8 for its
/// Gated-DeltaNet family, its swiglu and its partial rotary; the gelu probe, whose line is the walk's `Silu` wart; and
/// the partial-rotary probe.
#[test]
fn the_forward_lifts_what_the_golden_shows() {
    let mut refused: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    let mut lowered = BTreeSet::new();
    for (name, source) in corpus() {
        let Ok(walked) = cuda_walk(&source) else {
            continue;
        };
        if walked.is_empty() {
            continue;
        }
        let graph = with_parsed_program(&source, |parsed, interner| {
            let (stmts, _, _) = parsed.expect("a program the walk read parses");
            forward_graph(stmts, interner, &HashMap::new(), &HashMap::new(), Q8MatKind::Dequant).map(|graph| {
                let ops: Vec<_> = graph.ops.iter().flat_map(|op| leaves(op.clone())).filter_map(|op| op.prim().cloned()).collect();
                ops.iter()
                    .map(|op| {
                        lens(op, &|sym| interner.resolve(sym).to_string())
                            .unwrap_or_else(|| panic!("{name}: the forward lowers a `{}` it has no stage for", op.kind()))
                    })
                    .collect::<Vec<_>>()
            })
        });
        match graph {
            Ok(lifted) => {
                assert_eq!(lifted, walked, "{name}: the forward's lifted ops through the lens are the golden's lines");
                lowered.insert(name);
            }
            Err(ForwardError::Refused(refusals)) => {
                let kinds = refusals
                    .iter()
                    .map(|r| match r {
                        StageRefusal::NoKernel { kind, .. } => *kind,
                        other => panic!("{name}: the walk lifts every statement the golden shows: {other}"),
                    })
                    .collect();
                refused.insert(name, kinds);
            }
            Err(other) => panic!("{name}: the forward neither lowers nor refuses by kind: {other:?}"),
        }
    }
    let kinds = |ks: &[&'static str]| ks.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        refused,
        BTreeMap::from([
            ("probe:gelu".to_string(), kinds(&["Gelu"])),
            ("probe:rope_partial".to_string(), kinds(&["Rope"])),
            (
                "qwen38_tiny".to_string(),
                kinds(&["CausalConv", "Decay", "GatedDelta", "L2Norm", "Rope", "Sigmoid", "SwiGlu"])
            ),
        ]),
        "the programs the forward refuses, and the kinds it refuses each by"
    );
    assert!(lowered.contains("qwen30b_tiny"), "the authored Qwen3-30B forward lowers: {lowered:?}");
}
