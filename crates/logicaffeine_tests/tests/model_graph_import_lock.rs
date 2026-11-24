//! model_graph_import_lock — INTAKE-8 of `benchmarks/PREPARE_FOR_SAMBA.md`: a network `torch.export` exports, brought
//! up from its graph alone through the one writer of models.
//!
//! `model::graph::Graph::import` reads a `.pt2` archive's ATen graph into the ops the author writes — a linear layer with
//! or without its bias, GELU exact (`erf`) or tanh, ReLU, SiLU, sigmoid, a sum with its `alpha` and a product — over one
//! input to one output, every parameter read from the archive's raw float32 weights; an ATen op it does not map is
//! refused by its name, as is every shape of graph it does not take. `model::author::graph` writes the forward as the
//! sentences the language has for each op (exact GELU through `erf of`); `largo emit rdu --graph` lowers that program
//! whole on the SN50 and writes its certified bundle, whose host (`spatial::drop::run_graph_bundle`) rechecks the
//! certificate, binds the input and every parameter by the manifest and runs the spatial evaluator.
//!
//! Each run is held to the graph's `f64` forward (`Graph::forward`): the fp32 twin within fp32's rounding, the SN50's
//! bf16 datapath within the bound every Samba exam row holds a bf16 step to, and the bundle bit for bit to the program
//! lowered and run in place. With PyTorch present (`LOGOS_PYTHON`), torch's own exports are read and the forward is
//! held to torch's own float32 output; without it, the archives written here hold every row but those.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test model_graph_import_lock`

#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;

use logicaffeine_compile::model::author;
use logicaffeine_compile::model::graph::{Graph, GraphError, GraphOp, GraphWeights, Unary};
use logicaffeine_compile::model::pt2::Pt2;
use logicaffeine_compile::spatial::drop::{run_graph_bundle, run_graph_on};
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ── An archive as `torch.export.save` writes one ────────────────────────────────────────────────────────

/// A zip of `files` under the root folder `net/`, every entry stored.
fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let name = format!("net/{name}");
        let header = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&header.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn ints(sizes: &[usize]) -> Value {
    Value::Array(sizes.iter().map(|s| json!({ "as_int": s })).collect())
}

fn contiguous(sizes: &[usize]) -> Vec<usize> {
    (0..sizes.len()).map(|i| sizes[i + 1..].iter().product()).collect()
}

fn meta(sizes: &[usize]) -> Value {
    json!({ "dtype": 7, "sizes": ints(sizes), "strides": ints(&contiguous(sizes)), "storage_offset": { "as_int": 0 }, "layout": 7 })
}

fn tensor(name: &str) -> Value {
    json!({ "as_tensor": { "name": name } })
}

/// A graph as `torch.export` serializes one: its nodes, its input `x` of `input` sizes, its parameters (placeholder,
/// module name, sizes, values) and its output.
struct Exported {
    input: Vec<usize>,
    params: Vec<(&'static str, &'static str, Vec<usize>, Vec<f32>)>,
    nodes: Vec<(String, Vec<(&'static str, Value)>, &'static str, Vec<usize>)>,
    output: &'static str,
}

impl Exported {
    fn node(mut self, target: &str, inputs: Vec<(&'static str, Value)>, out: &'static str, sizes: &[usize]) -> Self {
        self.nodes.push((format!("torch.ops.aten.{target}"), inputs, out, sizes.to_vec()));
        self
    }

    fn model(&self) -> Value {
        let mut values = serde_json::Map::new();
        values.insert("x".into(), meta(&self.input));
        for (placeholder, _, sizes, _) in &self.params {
            values.insert((*placeholder).into(), meta(sizes));
        }
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|(target, inputs, out, sizes)| {
                values.insert((*out).into(), meta(sizes));
                json!({
                    "target": target,
                    "inputs": inputs.iter().map(|(name, arg)| json!({ "name": name, "arg": arg, "kind": 1 })).collect::<Vec<_>>(),
                    "outputs": [tensor(out)],
                })
            })
            .collect();
        let mut specs: Vec<Value> = self
            .params
            .iter()
            .map(|(placeholder, name, _, _)| json!({ "parameter": { "arg": { "name": placeholder }, "parameter_name": name } }))
            .collect();
        specs.push(json!({ "user_input": { "arg": tensor("x") } }));
        json!({ "graph_module": {
            "graph": { "nodes": nodes, "tensor_values": values },
            "signature": { "input_specs": specs, "output_specs": [{ "user_output": { "arg": tensor(self.output) } }] },
        } })
    }

    fn config(&self) -> Value {
        let mut config = serde_json::Map::new();
        for (k, (_, name, sizes, _)) in self.params.iter().enumerate() {
            config.insert((*name).into(), json!({ "path_name": format!("weight_{k}"), "is_param": true, "use_pickle": false, "tensor_meta": meta(sizes) }));
        }
        json!({ "config": config })
    }

    /// The archive, its model and configuration passed through `edit` first.
    fn write_edited(&self, dir: &Path, edit: impl Fn(&mut Value, &mut Value)) -> PathBuf {
        let (mut model, mut config) = (self.model(), self.config());
        edit(&mut model, &mut config);
        let mut files = vec![
            ("models/model.json".to_string(), serde_json::to_vec(&model).unwrap()),
            ("data/weights/model_weights_config.json".to_string(), serde_json::to_vec(&config).unwrap()),
            ("archive_format".to_string(), b"pt2".to_vec()),
        ];
        for (k, (_, _, _, values)) in self.params.iter().enumerate() {
            files.push((format!("data/weights/weight_{k}"), values.iter().flat_map(|v| v.to_le_bytes()).collect()));
        }
        let path = dir.join("net.pt2");
        std::fs::write(&path, zip(&files)).unwrap();
        path
    }

    fn write(&self, dir: &Path) -> PathBuf {
        self.write_edited(dir, |_, _| {})
    }
}

/// `count` values of a counting pattern scaled into `[-scale, scale]`, distinct and irrational-looking.
fn values(count: usize, seed: usize, scale: f32) -> Vec<f32> {
    (0..count).map(|i| ((((i * 37 + seed * 11) % 97) as f32) / 48.0 - 1.0) * scale).collect()
}

/// A network of every mapped op but the tanh GELU, SiLU and sigmoid: `x` (2 × 4) through a linear layer with its bias,
/// ReLU, a linear layer without one, the exact GELU, a product and a sum with `alpha` 0.5.
fn network() -> Exported {
    Exported {
        input: vec![2, 4],
        params: vec![
            ("p_a_weight", "a.weight", vec![6, 4], values(24, 1, 0.9)),
            ("p_a_bias", "a.bias", vec![6], values(6, 2, 0.3)),
            ("p_b_weight", "b.weight", vec![3, 6], values(18, 3, 0.8)),
        ],
        nodes: Vec::new(),
        output: "add",
    }
    .node("linear.default", vec![("input", tensor("x")), ("weight", tensor("p_a_weight")), ("bias", tensor("p_a_bias"))], "linear", &[2, 6])
    .node("relu.default", vec![("self", tensor("linear"))], "relu", &[2, 6])
    .node("linear.default", vec![("input", tensor("relu")), ("weight", tensor("p_b_weight"))], "linear_1", &[2, 3])
    .node("gelu.default", vec![("self", tensor("linear_1"))], "gelu", &[2, 3])
    .node("mul.Tensor", vec![("self", tensor("gelu")), ("other", tensor("linear_1"))], "mul", &[2, 3])
    .node("add.Tensor", vec![("self", tensor("mul")), ("other", tensor("gelu")), ("alpha", json!({ "as_float": 0.5 }))], "add", &[2, 3])
}

/// The network's input: both rows keep three of their six ReLUs, and the GELU's `erf` reads both signs past 1.
const X: [f64; 8] = [0.25, -1.5, 0.75, 2.0, -1.75, 0.5, 1.25, -0.375];

/// The SN50 with fp32 in place of bf16 in every datapath: every unit exact to fp32's rounding.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.remove(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.remove(&Precision::Bf16);
    t.precisions.insert(Precision::Fp32);
    t
}

fn worst(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "the outputs are as long as each other");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}

fn largest(v: &[f64]) -> f64 {
    v.iter().fold(0.0f64, |m, x| m.max(x.abs()))
}

/// The bf16 SN50 datapath's bound on a step against the `f64` reference, every Samba exam row's: 2% of the largest
/// reference value, grown with the square root of the widest dot product past 64.
fn bf16_bound(want: &[f64], widest: usize) -> f64 {
    0.02 * (widest as f64 / 64.0).sqrt().max(1.0) * largest(want)
}

fn forward(graph: &Graph, pt2: &Pt2, x: &[f64]) -> Vec<f64> {
    graph.forward(&mut GraphWeights { graph, pt2 }, x).expect("the f64 forward")
}

/// The archive's graph brought up on every target: the fp32 twin within fp32's rounding, the SN50 in place within its
/// bf16 bound, and the bundle `largo emit rdu --graph` writes bit for bit the SN50 in place. The SN50's output.
fn brings_up(dir: &Path, archive: &Path, x: &[f64], want: &[f64], widest: usize, what: &str) -> Vec<f64> {
    let (graph, pt2) = Graph::open(archive).unwrap_or_else(|e| panic!("{what}: the archive imports: {e}"));
    let twin = run_graph_on(&graph, &pt2, &fp32_twin(), x).unwrap_or_else(|e| panic!("{what}: the fp32 twin runs it: {e}"));
    let fp32 = 1e-6 * (widest as f64).sqrt() * largest(want).max(1.0);
    assert!(worst(&twin, want) <= fp32, "{what}: the fp32 twin is {:e} from the f64 forward, past {fp32:e}", worst(&twin, want));
    let sn50 = run_graph_on(&graph, &pt2, &SpatialTarget::sn50_rdu(), x).unwrap_or_else(|e| panic!("{what}: the SN50 runs it: {e}"));
    let bound = bf16_bound(want, widest);
    assert!(worst(&sn50, want) <= bound, "{what}: the SN50 is {:e} from the f64 forward, past its datapath's {bound:e}", worst(&sn50, want));
    let bundle = dir.join("bundle");
    let out = cli::largo_in(dir, &["emit", "rdu", "--chip", "sn50", "--graph", archive.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{what}: the graph lowers:\n{}", cli::stderr(&out));
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(bundle.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "logos.rdu.graph_manifest", "{what}");
    assert_eq!(manifest["chip"], "sn50", "{what}");
    assert_eq!(manifest["forward"]["rechecked"], true, "{what}: the bundle's certificate was rechecked");
    let authored = author::graph(&graph).expect("the author writes it");
    assert_eq!(std::fs::read_to_string(bundle.join("program.lg")).unwrap(), authored.text, "{what}: program.lg is the author's");
    for file in ["neutral.json", "placement.txt", "certificate.json", "report.json"] {
        assert!(bundle.join("forward").join(file).is_file(), "{what}: forward/{file}");
    }
    let hosted = run_graph_bundle(&bundle, archive, x).unwrap_or_else(|e| panic!("{what}: the bundle's host runs it: {e}"));
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&hosted), bits(&sn50), "{what}: the bundle computes what the SN50 computes in place, bit for bit");
    sn50
}

// ── The archive written here ────────────────────────────────────────────────────────────────────────────

#[test]
fn every_op_is_read_as_the_op_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let (graph, _) = Graph::open(&network().write(dir.path())).expect("the archive imports");
    assert_eq!(graph.input, "x");
    assert_eq!(graph.input_sizes, vec![2, 4]);
    assert_eq!(graph.output, "add");
    let names: Vec<(&str, &str, Vec<usize>)> =
        graph.params.iter().map(|(k, p)| (k.as_str(), p.name.as_str(), p.sizes.clone())).collect();
    assert_eq!(names, vec![("p_a_bias", "a.bias", vec![6]), ("p_a_weight", "a.weight", vec![6, 4]), ("p_b_weight", "b.weight", vec![3, 6])]);
    let s = |v: &str| v.to_string();
    assert_eq!(
        graph.ops,
        vec![
            GraphOp::Linear { out: s("linear"), input: s("x"), weight: s("p_a_weight"), bias: Some(s("p_a_bias")), in_features: 4, out_features: 6 },
            GraphOp::Unary { out: s("relu"), input: s("linear"), op: Unary::Relu },
            GraphOp::Linear { out: s("linear_1"), input: s("relu"), weight: s("p_b_weight"), bias: None, in_features: 6, out_features: 3 },
            GraphOp::Unary { out: s("gelu"), input: s("linear_1"), op: Unary::GeluExact },
            GraphOp::Mul { out: s("mul"), a: s("gelu"), b: s("linear_1") },
            GraphOp::Add { out: s("add"), a: s("mul"), b: s("gelu"), alpha: 0.5 },
        ]
    );
}

#[test]
fn the_program_is_the_sentences_the_language_has_for_each_op() {
    let dir = tempfile::tempdir().unwrap();
    let (graph, _) = Graph::open(&network().write(dir.path())).expect("the archive imports");
    let p = author::graph(&graph).expect("the author writes it");
    assert_eq!(
        p.text,
        "## Main\n\
         Let x be a 2 by 4 tensor of Real.\n\
         Let W1 be a 4 by 6 tensor of Real.\n\
         Let B1 be a 1 by 6 tensor of Real.\n\
         Let h1r be x matmul W1.\n\
         Let h1 be h1r plus B1.\n\
         Let h2 be relu of h1.\n\
         Let W2 be a 6 by 3 tensor of Real.\n\
         Let h3 be h2 matmul W2.\n\
         Let h4s be h3 scaled by 0.7071067811865476.\n\
         Let h4e be erf of h4s.\n\
         Let h4p be h3 hadamard h4e.\n\
         Let h4q be h3 plus h4p.\n\
         Let h4 be h4q scaled by 0.5.\n\
         Let h5 be h4 hadamard h3.\n\
         Let h6b be h4 scaled by 0.5.\n\
         Let h6 be h5 plus h6b.\n"
    );
    let bound: Vec<(&str, &str, &str)> = p
        .params
        .iter()
        .map(|(v, b)| (v.as_str(), b.tensor.as_str(), if b.view == logicaffeine_compile::model::bind::View::Transposed { "transposed" } else { "same" }))
        .collect();
    assert_eq!(bound, vec![("W1", "a.weight", "transposed"), ("B1", "a.bias", "same"), ("W2", "b.weight", "transposed")]);
    assert_eq!((p.input.var.as_str(), p.input.rows, p.input.cols), ("x", 2, 4));
    assert_eq!((p.output.var.as_str(), p.output.graph.as_str(), p.output.rows, p.output.cols), ("h6", "add", 2, 3));
}

#[test]
fn the_forward_is_the_networks_arithmetic() {
    let dir = tempfile::tempdir().unwrap();
    let net = network();
    let (graph, pt2) = Graph::open(&net.write(dir.path())).expect("the archive imports");
    let got = forward(&graph, &pt2, &X);
    // The same network written out by hand, from the stored values: y = x·Wᵀ + b in the order of the inputs.
    let w = |k: usize| net.params[k].3.iter().map(|v| f64::from(*v)).collect::<Vec<_>>();
    let (wa, ba, wb) = (w(0), w(1), w(2));
    let linear = |x: &[f64], w: &[f64], b: Option<&[f64]>, i: usize, o: usize| -> Vec<f64> {
        (0..x.len() / i)
            .flat_map(|r| (0..o).map(move |c| (0..i).fold(0.0, |s, k| s + x[r * i + k] * w[c * i + k]) + b.map_or(0.0, |b| b[c])))
            .collect()
    };
    let h = linear(&X, &wa, Some(&ba), 4, 6).into_iter().map(|v| v.max(0.0)).collect::<Vec<_>>();
    let l1 = linear(&h, &wb, None, 6, 3);
    let gelu: Vec<f64> = l1.iter().map(|v| 0.5 * v * (1.0 + erf_reference(v / std::f64::consts::SQRT_2))).collect();
    let want: Vec<f64> = (0..6).map(|i| gelu[i] * l1[i] + 0.5 * gelu[i]).collect();
    assert!(worst(&got, &want) <= 1e-15 * largest(&want).max(1.0), "the forward is the network's: {got:?} vs {want:?}");
}

/// The error function by its Maclaurin series: to double precision where `|x|` stays below about 1.5, as every argument
/// of this network's GELU does (past it the alternating terms cancel).
fn erf_reference(x: f64) -> f64 {
    let (mut term, mut sum, mut n) = (x, x, 0.0);
    while term.abs() > 1e-18 * sum.abs() {
        n += 1.0;
        term *= -x * x / n;
        sum += term / (2.0 * n + 1.0);
    }
    sum * std::f64::consts::FRAC_2_SQRT_PI
}

#[test]
fn the_network_brings_up_on_the_sn50() {
    let dir = tempfile::tempdir().unwrap();
    let archive = network().write(dir.path());
    let (graph, pt2) = Graph::open(&archive).unwrap();
    let want = forward(&graph, &pt2, &X);
    brings_up(dir.path(), &archive, &X, &want, 6, "the network");
}

#[test]
fn a_bundle_whose_program_changed_is_refused_by_its_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let archive = network().write(dir.path());
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(dir.path(), &["emit", "rdu", "--graph", archive.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", cli::stderr(&out));
    let neutral = bundle.join("forward/neutral.json");
    let mut program: Value = serde_json::from_str(&std::fs::read_to_string(&neutral).unwrap()).unwrap();
    let stages = program.pointer_mut("/program/stages").and_then(Value::as_array_mut).expect("the program's stages");
    let last = stages.pop().expect("a stage");
    stages.insert(0, last);
    std::fs::write(&neutral, serde_json::to_string(&program).unwrap()).unwrap();
    let e = run_graph_bundle(&bundle, &archive, &X).expect_err("a changed program is refused").to_string();
    assert!(e.contains("forward/neutral.json") && (e.contains("not the one certified") || e.contains("cannot be made again") || e.contains("does not parse")), "{e}");
}

#[test]
fn what_the_importer_does_not_take_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let refused = |edit: &dyn Fn(&mut Value, &mut Value), what: &str, names: &[&str]| {
        let archive = network().write_edited(dir.path(), edit);
        let e = Graph::open(&archive).map(|_| ()).expect_err(what).to_string();
        for n in names {
            assert!(e.contains(n), "{what}: the refusal names `{n}`: {e}");
        }
        let bundle = dir.path().join("refused");
        let out = cli::largo_in(dir.path(), &["emit", "rdu", "--graph", archive.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
        assert_ne!(out.status.code(), Some(0), "{what}: largo refuses it");
        for n in names {
            assert!(cli::stderr(&out).contains(n), "{what}: largo names `{n}`:\n{}", cli::stderr(&out));
        }
    };
    let node = |m: &mut Value, at: usize| m.pointer_mut(&format!("/graph_module/graph/nodes/{at}")).unwrap().clone();
    refused(&|m, _| m["graph_module"]["graph"]["nodes"][1]["target"] = json!("torch.ops.aten.cumsum.default"), "an unmapped op", &["`aten.cumsum.default`", "does not map"]);
    refused(&|m, _| m["graph_module"]["graph"]["nodes"][3]["inputs"] = json!([{ "name": "self", "arg": tensor("linear_1") }, { "name": "approximate", "arg": { "as_string": "erfc" } }]), "an unknown GELU", &["approximates GELU as `erfc`"]);
    refused(&|m, _| {
        let specs = m["graph_module"]["signature"]["input_specs"].as_array_mut().unwrap();
        specs.push(json!({ "user_input": { "arg": tensor("relu") } }));
    }, "two inputs", &["takes 2 inputs"]);
    refused(&|m, _| {
        let specs = m["graph_module"]["signature"]["input_specs"].as_array_mut().unwrap();
        specs.insert(0, json!({ "buffer": { "arg": { "name": "b_running" }, "buffer_name": "running", "persistent": true } }));
    }, "a buffer", &["`buffer`"]);
    refused(&|m, _| {
        let specs = m["graph_module"]["signature"]["output_specs"].as_array_mut().unwrap();
        specs.push(json!({ "user_output": { "arg": tensor("gelu") } }));
    }, "two outputs", &["gives 2 outputs"]);
    refused(&|m, _| m["graph_module"]["graph"]["tensor_values"]["relu"]["dtype"] = json!(5), "a float16 tensor", &["`relu`", "not float32"]);
    refused(&|_, c| c["config"]["a.weight"]["tensor_meta"]["strides"] = ints(&[1, 6]), "a transposed weight", &["`a.weight`", "contiguously"]);
    refused(&|_, c| c["config"]["b.weight"]["use_pickle"] = json!(true), "a pickled weight", &["`b.weight`", "raw float32"]);
    refused(&|m, _| m["graph_module"]["graph"]["tensor_values"]["mul"]["sizes"] = ints(&[1, 3]), "a broadcast sum", &["`aten.add.Tensor`", "broadcast"]);
    refused(&|m, _| {
        let mut n = node(m, 0);
        n["inputs"][1]["arg"] = tensor("relu");
        m["graph_module"]["graph"]["nodes"][0] = n;
    }, "a weight that is no parameter", &["`relu`", "not a parameter"]);
    refused(&|m, _| m["graph_module"]["graph"]["nodes"][0]["inputs"][2]["arg"] = tensor("p_b_weight"), "a bias of the wrong width", &["`p_b_weight`", "bias"]);

    let not_zip = dir.path().join("not.pt2");
    std::fs::write(&not_zip, b"an archive in name only, long enough that its tail is searched for a directory").unwrap();
    let e = Graph::open(&not_zip).map(|_| ()).expect_err("not a zip").to_string();
    assert!(e.contains("not a zip archive"), "{e}");
}

#[test]
fn a_graph_the_author_cannot_write_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let net = Exported { input: vec![2, 4], params: vec![("p_a_weight", "a.weight", vec![6, 4], values(24, 1, 0.9))], nodes: Vec::new(), output: "x" }
        .node("relu.default", vec![("self", tensor("x"))], "relu", &[2, 4]);
    let (graph, _) = Graph::open(&net.write(dir.path())).expect("it imports");
    let e = author::graph(&graph).expect_err("an output no op computes").to_string();
    assert!(e.contains("whose output no op computes"), "{e}");
}

// ── PyTorch's own exports ───────────────────────────────────────────────────────────────────────────────

/// Export `module` (Python defining `Net`) on `torch.randn(shape)` seeded 0 into `dir/net.pt2`, and torch's own
/// float32 output on the input, which is written beside it. The input and the output, row by row.
fn torch_export(dir: &Path, module: &str, shape: &[usize]) -> (PathBuf, Vec<f64>, Vec<f64>) {
    let archive = dir.join("net.pt2");
    let io = dir.join("io.json");
    let shape = shape.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ");
    let script = format!(
        "import json, torch\n{module}\ntorch.manual_seed(0)\nnet = Net()\nx = torch.randn({shape})\n\
         ep = torch.export.export(net, (x,))\ntorch.export.save(ep, r'{}')\n\
         y = net(x)\njson.dump({{'x': x.flatten().tolist(), 'y': y.flatten().tolist()}}, open(r'{}', 'w'))\n",
        archive.display(),
        io.display()
    );
    let made = std::process::Command::new(common::python_for_tools()).args(["-c", &script]).output().expect("python");
    assert!(made.status.success(), "the export: {}", String::from_utf8_lossy(&made.stderr));
    let io: Value = serde_json::from_str(&std::fs::read_to_string(io).unwrap()).unwrap();
    let floats = |k: &str| io[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    (archive, floats("x"), floats("y"))
}

#[test]
fn every_op_torch_exports_is_read_and_computed_as_torch_computes_it() {
    if !common::require_tool("torch") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let module = "class Net(torch.nn.Module):\n    def __init__(s):\n        super().__init__()\n        s.a = torch.nn.Linear(8, 12)\n        s.b = torch.nn.Linear(12, 12, bias=False)\n    def forward(s, x):\n        h = torch.nn.functional.gelu(s.a(x))\n        k = torch.nn.functional.gelu(s.b(h), approximate='tanh')\n        q = torch.sigmoid(torch.nn.functional.silu(torch.relu(k)))\n        return torch.mul(torch.add(h, q, alpha=0.5), h)";
    let (archive, x, torch) = torch_export(dir.path(), module, &[2, 8]);
    let (graph, pt2) = Graph::open(&archive).expect("torch's archive imports");
    let kinds: Vec<String> = graph
        .ops
        .iter()
        .map(|op| match op {
            GraphOp::Linear { bias, .. } => format!("linear{}", if bias.is_some() { "+bias" } else { "" }),
            GraphOp::Unary { op, .. } => format!("{op:?}"),
            GraphOp::Add { alpha, .. } => format!("add {alpha}"),
            GraphOp::Mul { .. } => "mul".into(),
        })
        .collect();
    assert_eq!(kinds, ["linear+bias", "GeluExact", "linear", "GeluTanh", "Relu", "Silu", "Sigmoid", "add 0.5", "mul"]);
    let want = forward(&graph, &pt2, &x);
    let fp32 = 1e-5 * largest(&want).max(1.0);
    assert!(worst(&want, &torch) <= fp32, "the f64 forward is {:e} from torch's float32 output, past {fp32:e}", worst(&want, &torch));
    brings_up(dir.path(), &archive, &x, &want, 12, "torch's every-op network");
}

#[test]
fn an_unmapped_aten_op_is_refused_by_name() {
    if !common::require_tool("torch") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let module = "class Net(torch.nn.Module):\n    def __init__(s):\n        super().__init__()\n        s.a = torch.nn.Linear(8, 8)\n    def forward(s, x):\n        return torch.cumsum(s.a(x), dim=1)";
    let (archive, _, _) = torch_export(dir.path(), module, &[1, 8]);
    match Graph::open(&archive) {
        Err(GraphError::Unmapped(op)) => assert_eq!(op, "aten.cumsum.default"),
        other => panic!("cumsum is refused by its name, not {other:?}"),
    }
    let bundle = dir.path().join("bundle");
    let out = cli::largo_in(dir.path(), &["emit", "rdu", "--graph", archive.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
    assert!(cli::stderr(&out).contains("`aten.cumsum.default`"), "{}", cli::stderr(&out));
}
