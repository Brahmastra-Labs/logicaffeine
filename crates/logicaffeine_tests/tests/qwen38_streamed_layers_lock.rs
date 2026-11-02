//! Qwen3.8, LOWERED ONE LAYER AT A TIME — and chained, it computes what the whole forward computes.
//!
//! Lowering the forward as one composed program costs ~14 GB per real-width layer, so the whole 64-layer
//! model cannot be held at once. But a layer is a self-contained program: it reads one hidden-state
//! tensor, carries its own conv rings and recurrent memory, and writes one hidden-state tensor. So each
//! layer can be lowered, certified and emitted on its own, and dropped — memory constant in depth.
//!
//! That is only worth anything if splitting the model changes NOTHING. This file proves it the direct
//! way: every layer is lowered to its own spatial program and certified; the programs are EXECUTED IN A
//! CHAIN, each layer's output tensor handed to the next layer as its input; the head turns the last
//! hidden state into logits; and those logits are compared against the VM running the WHOLE, unsplit
//! forward — the token must agree exactly, the logits within the bf16 bound.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_streamed_layers_lock`
//!
//! ★ THE NEGATIVE CONTROL IS A TEST, NOT A ONE-OFF PROBE. `breaking_the_chain_breaks_the_answer` hands every
//! layer a zeroed hidden state; the layers still lower, certify and run, and the logits land at drift 1.0
//! against a 0.02 bound. Without it, the positive comparison could pass while the chain carried nothing.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_head_lg, gen_qwen38_layer_lg, gen_qwen38_lg, Qwen38Cfg,
};
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::model::author::layer_input;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// Fill every declared tensor from its NAME, so the whole forward and each standalone layer get the same
/// values for the same tensor.
fn with_fills(authored: &str) -> String {
    let mut src = String::new();
    for line in authored.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        let salt = var.bytes().map(u64::from).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(salt.wrapping_mul(40503));
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                ((h % 97) as f64 / 97.0 - 0.5) * 0.4
            ));
        }
    }
    src
}

fn inputs_from_fills(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if t.init.is_float() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..n)
                        .map(|i| t.init.element_f64(i).expect("float fill"))
                        .collect(),
                ),
            );
        } else {
            inputs.tensors.insert(
                t.id,
                Value::Int((0..n).map(|i| t.init.element(i).expect("fill")).collect()),
            );
        }
    }
    inputs
}

fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Float(f) => f.clone(),
        Value::Int(i) => i.iter().map(|x| *x as f64).collect(),
    }
}

/// Lower one standalone program, certify it, and execute it with `input` bound to `carried`.
fn lower_certify_run(
    src: &str,
    input: &str,
    carried: Option<&[f64]>,
    out: &str,
) -> (Vec<f64>, usize) {
    let target = RduChip::Sn50.facts().spatial_target();
    let mut p = compile_program_to_spatial(src, &target)
        .expect("a tensor program")
        .program
        .expect("the program lowers whole");
    p.certificate = None;
    certify_choreography(&p).unwrap_or_else(|e| panic!("the standalone program certifies: {e:?}"));
    let mut inputs = inputs_from_fills(&p);
    if let Some(v) = carried {
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == input)
            .unwrap_or_else(|| panic!("the program reads `{input}`"));
        assert_eq!(
            t.elems.rows * t.elems.cols,
            v.len(),
            "the carried hidden state has the input's width"
        );
        inputs.tensors.insert(t.id, Value::Float(v.to_vec()));
    }
    let run = eval_choreography(&p, &inputs).expect("the choreography evaluates");
    let t = p
        .tensors
        .iter()
        .find(|t| t.name == out)
        .unwrap_or_else(|| panic!("the program writes `{out}`"));
    (
        floats(run.tensors.get(&t.id).expect("the output is written")),
        p.sections.len(),
    )
}

/// The logits of the chained, layer-by-layer forward.
fn streamed_logits(cfg: &Qwen38Cfg, carry: impl Fn(usize, &[f64]) -> Vec<f64>) -> Vec<f64> {
    let mut hidden: Option<Vec<f64>> = None;
    for l in 0..cfg.layers {
        let (src, _, _) = gen_qwen38_layer_lg(cfg, l, "/w");
        let input = layer_input(l);
        let bound = hidden.as_deref().map(|h| carry(l, h));
        let (out, _) = lower_certify_run(
            &with_fills(&src),
            &input,
            bound.as_deref(),
            &format!("xo{l}"),
        );
        hidden = Some(out);
    }
    let (head, _, _) = gen_qwen38_head_lg(cfg, "/w");
    let h = hidden.expect("at least one layer");
    lower_certify_run(
        &with_fills(&head),
        &layer_input(cfg.layers),
        Some(&carry(cfg.layers, &h)),
        "logits",
    )
    .0
}

fn vm_logits(cfg: &Qwen38Cfg) -> Vec<i64> {
    let (authored, _, _) = gen_qwen38_lg(cfg, "/w");
    let mut src = with_fills(&authored);
    for i in 1..=cfg.vocab {
        src.push_str(&format!("Show round(item {i} of logits * 1000000).\n"));
    }
    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the whole forward runs on the VM: {:?}",
        vm.error
    );
    vm.output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer logit"))
        .collect()
}

fn argmax(v: &[i64]) -> usize {
    v.iter()
        .enumerate()
        .max_by_key(|(_, x)| **x)
        .map(|(i, _)| i)
        .unwrap()
}

fn worst_drift(ours: &[i64], vm: &[i64]) -> f64 {
    let scale = vm.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    ours.iter()
        .zip(vm)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max)
}

const BF16_WHOLE_FORWARD_REL: f64 = 0.02;

/// ── EACH LAYER STANDS ALONE. Every layer of the forward lowers as its own complete program and certifies.
#[test]
fn every_layer_lowers_and_certifies_on_its_own() {
    let cfg = Qwen38Cfg::tiny();
    for l in 0..cfg.layers {
        let (src, _, _) = gen_qwen38_layer_lg(&cfg, l, "/w");
        let kind = if cfg.is_attn(l) {
            "attention"
        } else {
            "gated deltanet"
        };
        let (_, sections) = lower_certify_run(
            &with_fills(&src),
            &layer_input(l),
            None,
            &format!("xo{l}"),
        );
        println!("layer {l} ({kind}): {sections} sections, certified");
        assert!(sections > 1, "layer {l} composes more than one section");
    }
}

/// ── CHAINED, THE LAYERS ARE THE MODEL. The streamed forward picks the whole forward's token, within the
/// same bf16 bound the composed forward is held to.
#[test]
fn the_streamed_forward_picks_the_token_the_whole_forward_picks() {
    let cfg = Qwen38Cfg::tiny();
    let want = vm_logits(&cfg);
    let got: Vec<i64> = streamed_logits(&cfg, |_, h| h.to_vec())
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    assert_eq!(got.len(), want.len());
    assert_eq!(
        argmax(&got),
        argmax(&want),
        "the streamed forward picks a different token\n ours {got:?}\n vm   {want:?}"
    );
    let worst = worst_drift(&got, &want);
    println!(
        "streamed {} layers: argmax agrees, worst drift {worst:.5}",
        cfg.layers
    );
    assert!(
        worst <= BF16_WHOLE_FORWARD_REL,
        "the streamed forward drifts {worst:.5} from the VM, past {BF16_WHOLE_FORWARD_REL}"
    );
}

/// ── AND THE CHAIN IS LOAD-BEARING. Hand every layer a zeroed hidden state instead of the previous layer's
/// output: the layers still lower, certify and run, but the result must no longer be the model's. If this
/// passed, the comparison above would not be testing the chain at all.
#[test]
fn breaking_the_chain_breaks_the_answer() {
    let cfg = Qwen38Cfg::tiny();
    let want = vm_logits(&cfg);
    let got: Vec<i64> = streamed_logits(&cfg, |_, h| vec![0.0; h.len()])
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    let worst = worst_drift(&got, &want);
    println!("chain cut: worst drift {worst:.5}");
    assert!(
        worst > BF16_WHOLE_FORWARD_REL,
        "a forward that ignores its own hidden state must not match the model (drift {worst:.5})"
    );
}

/// A lowered layer with everything that NAMES its weights erased: tensor names and fills, and the stage and
/// buffer labels that quote those names (`s5.in_Csq0@0,0`). Nothing structural refers to a label — edges,
/// schedules and placements all refer by id — so what is left is the dataflow: stages, buffers, placement,
/// schedule, sections, access maps. That is what a chip runs.
///
/// MEASURED when this landed: two Gated DeltaNet layers differ in exactly 133 leaves, and all 133 are
/// `.name` fields (50 tensor, 49 stage, 33 buffer, 1 program). Not one structural field differs.
fn dataflow_of(src: &str) -> serde_json::Value {
    let target = RduChip::Sn50.facts().spatial_target();
    let mut p = compile_program_to_spatial(src, &target)
        .expect("a tensor program")
        .program
        .expect("lowers whole");
    p.certificate = None;
    for t in &mut p.tensors {
        t.name.clear();
        t.init = logicaffeine_compile::spatial::ir::TensorInit::Zero;
    }
    for b in &mut p.buffers {
        b.name.clear();
    }
    for st in &mut p.stages {
        st.name.clear();
    }
    for sec in &mut p.sections {
        sec.name.clear();
    }
    let mut v = serde_json::to_value(&p).expect("serializes");
    v["name"] = serde_json::Value::Null;
    v
}

/// ── LAYERS OF ONE KIND ARE ONE PROGRAM. Every Gated DeltaNet layer lowers to the same spatial program, and
/// every attention layer to another, differing ONLY in which weight tensors they read. So the forward is two
/// dataflows applied layer after layer — the basis for lowering each kind once and binding weights per layer,
/// stated as a fact about the compiler's output rather than assumed from the source looking regular.
#[test]
fn layers_of_the_same_kind_lower_to_the_same_program() {
    let cfg = Qwen38Cfg {
        layers: 8,
        ..Qwen38Cfg::tiny()
    };
    let (mut gdn, mut attn): (Option<serde_json::Value>, Option<serde_json::Value>) = (None, None);
    let (mut n_gdn, mut n_attn) = (0, 0);
    for l in 0..cfg.layers {
        let (src, _, _) = gen_qwen38_layer_lg(&cfg, l, "/w");
        let df = dataflow_of(&with_fills(&src));
        let (slot, n) = if cfg.is_attn(l) {
            (&mut attn, &mut n_attn)
        } else {
            (&mut gdn, &mut n_gdn)
        };
        match slot {
            None => *slot = Some(df),
            Some(first) => assert_eq!(
                &df, first,
                "layer {l} lowers to a different dataflow than the first layer of its kind"
            ),
        }
        *n += 1;
    }
    assert!(
        n_gdn > 1 && n_attn > 1,
        "the check needs at least two layers of each kind ({n_gdn} gdn, {n_attn} attention)"
    );
    assert_ne!(gdn, attn, "the two layer kinds are different programs");
    println!(
        "{n_gdn} gated-deltanet layers -> 1 dataflow, {n_attn} attention layers -> 1 dataflow"
    );
}

/// The value `with_fills` gives element `i` of the tensor named `var` — computed from the name alone, so a
/// layer's weights can be produced for ANY layer without generating that layer's program.
fn fill_value(var: &str, i: usize) -> f64 {
    let salt = var.bytes().map(u64::from).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// ── THE WHOLE FORWARD FROM TWO LAYER PROGRAMS AND A MANIFEST. Only the first layer of each kind is lowered.
/// Every layer then runs one of those two programs, with that layer's weights bound by the manifest's name
/// map and the previous layer's output as its input. The logits must be the whole, unsplit forward's.
///
/// A binding the manifest got wrong — a weight renamed to the wrong layer, a missed carried-state tensor, the
/// wrong hidden input — computes a different model, and this comparison is where it would show.
///
/// ★ MUTATION PROBE (run when this landed): make `rebind` return every per-layer name unchanged, so each
/// layer runs with its kind's REPRESENTATIVE weights instead of its own — this test reds.
#[test]
fn the_forward_runs_from_one_program_per_layer_kind_and_a_manifest() {
    use logicaffeine_compile::spatial::streamed::lower_forward_streamed;
    let cfg = Qwen38Cfg {
        layers: 8,
        ..Qwen38Cfg::tiny()
    };
    let target = RduChip::Sn50.facts().spatial_target();
    let model =
        lower_forward_streamed(&cfg.spec(), Storage::AllQ8, &target, with_fills).expect("the streamed model lowers");
    assert_eq!(
        model.kinds.len(),
        2,
        "two layer kinds: {:?}",
        model.kinds.iter().map(|k| &k.name).collect::<Vec<_>>()
    );
    assert_eq!(model.layers.len(), cfg.layers, "every layer is bound");
    for k in &model.kinds {
        assert!(
            k.program
                .certificate
                .as_ref()
                .is_some_and(|c| c.recheck(&k.program)),
            "the {} program carries a certificate that rechecks",
            k.name
        );
    }

    let run = |p: &SpatialProgram,
               bind: &dyn Fn(&str) -> String,
               carried: &[(String, Vec<f64>)],
               out: &str|
     -> Vec<f64> {
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let bound = bind(&t.name);
            let n = t.elems.rows * t.elems.cols;
            let v = match carried.iter().find(|(name, _)| *name == bound) {
                Some((_, v)) => v.clone(),
                None => (0..n).map(|i| fill_value(&bound, i)).collect(),
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        let o = p
            .tensors
            .iter()
            .find(|t| bind(&t.name) == out)
            .unwrap_or_else(|| panic!("writes `{out}`"));
        floats(r.tensors.get(&o.id).expect("the output is written"))
    };

    // The first layer's hidden state is the whole forward's `x`, filled from its name like every weight.
    let mut hidden: (String, Vec<f64>) = (
        "x".to_string(),
        (0..cfg.d).map(|i| fill_value("x", i)).collect(),
    );
    for b in &model.layers {
        let kind = &model.kinds[b.kind];
        let map = b.tensors.clone();
        let bind = move |name: &str| {
            map.iter()
                .find(|(from, _)| from == name)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| name.to_string())
        };
        let out = format!("xo{}", b.layer);
        let v = run(&kind.program, &bind, std::slice::from_ref(&hidden), &out);
        hidden = (out, v);
    }
    let logits = run(
        &model.head,
        &|n: &str| n.to_string(),
        std::slice::from_ref(&hidden),
        "logits",
    );

    let want = vm_logits(&cfg);
    let got: Vec<i64> = logits
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    assert_eq!(got.len(), want.len());
    assert_eq!(
        argmax(&got),
        argmax(&want),
        "the manifest-driven forward picks a different token\n ours {got:?}\n vm   {want:?}"
    );
    let worst = worst_drift(&got, &want);
    println!(
        "{} layers from {} lowered layer programs + head: argmax agrees, worst drift {worst:.5}",
        cfg.layers,
        model.kinds.len()
    );
    assert!(
        worst <= BF16_WHOLE_FORWARD_REL,
        "drift {worst:.5} past {BF16_WHOLE_FORWARD_REL}"
    );
}

/// ── THE MANIFEST NAMES A LAYER'S TENSORS THE WAY THAT LAYER NAMES THEM. Lower layer `l` directly and
/// compare its tensor names with the names the manifest binds for `l` — including the intermediates the
/// blocked lowerings derive from a layer's names (`xn5__xsq`), which a suffix-only rename leaves pointing at
/// the representative layer. The execution test cannot see this (intermediates are computed, not bound); a
/// consumer binding by name would.
#[test]
fn the_manifest_names_every_tensor_as_its_own_layer_does() {
    use logicaffeine_compile::spatial::streamed::lower_forward_streamed;
    let cfg = Qwen38Cfg {
        layers: 8,
        ..Qwen38Cfg::tiny()
    };
    let target = RduChip::Sn50.facts().spatial_target();
    let model = lower_forward_streamed(&cfg.spec(), Storage::AllQ8, &target, with_fills).expect("lowers");
    for b in model
        .layers
        .iter()
        .filter(|b| b.layer != model.kinds[b.kind].representative)
    {
        let (src, _, _) = gen_qwen38_layer_lg(&cfg, b.layer, "/w");
        let direct = compile_program_to_spatial(&with_fills(&src), &target)
            .unwrap()
            .program
            .unwrap();
        let mut want: Vec<String> = direct.tensors.iter().map(|t| t.name.clone()).collect();
        let mut got: Vec<String> = b.tensors.iter().map(|(_, to)| to.clone()).collect();
        want.sort();
        got.sort();
        assert_eq!(
            got, want,
            "layer {}: the manifest's names differ from the names layer {} gives its own tensors",
            b.layer, b.layer
        );
    }
}
