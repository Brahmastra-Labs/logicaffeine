//! A model imported from a PyTorch export — the spec kind `model::author` writes for a `torch.export` graph no spec arm
//! describes (Tristen, 2026-11-09: an imported graph enters as a spec kind the author writes, so one writer of models
//! still holds).
//!
//! [`Graph::import`] reads the archive's ATen graph (`models/model.json`) into ops the author writes — a linear layer
//! with or without its bias, GELU (exact or tanh), ReLU, SiLU, sigmoid, a sum (with its `alpha`) and a product — over
//! one input and to one output, every parameter read from the archive's weights as it is stored. An ATen op the
//! importer does not map is refused by its name (`aten.cumsum.default`), never written as something else.

use super::pt2::Pt2;
use super::quant::GgmlType;
use super::reference::WeightSource;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::Path;

/// A unary op the author writes elementwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    /// `x·Φ(x)`, the normal distribution's own: `0.5·x·(1 + erf(x/√2))` — `aten.gelu` with no `approximate`.
    GeluExact,
    /// `0.5·x·(1 + tanh(√(2/π)·(x + 0.044715·x³)))` — `aten.gelu` with `approximate="tanh"`.
    GeluTanh,
    Relu,
    Silu,
    Sigmoid,
}

/// One op of an imported graph, each naming the tensors it reads and the one it writes.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphOp {
    /// `out = input · weightᵀ + bias` — `aten.linear`, its weight `[out_features, in_features]` as PyTorch stores it.
    Linear { out: String, input: String, weight: String, bias: Option<String>, in_features: usize, out_features: usize },
    Unary { out: String, input: String, op: Unary },
    /// `out = a + alpha·b` — `aten.add.Tensor`, over operands of one shape.
    Add { out: String, a: String, b: String, alpha: f64 },
    /// `out = a ⊙ b` — `aten.mul.Tensor`, over operands of one shape.
    Mul { out: String, a: String, b: String },
}

/// A parameter of the graph: its name in the module (`a.weight`), the archive file holding it, and its sizes.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub file: String,
    pub sizes: Vec<usize>,
}

/// An exported graph read: its one input and its shape, its parameters by the placeholder that names each, its ops in
/// order, and its one output.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub input: String,
    pub input_sizes: Vec<usize>,
    pub params: BTreeMap<String, Param>,
    pub ops: Vec<GraphOp>,
    pub output: String,
    /// Every tensor's sizes, by the name the graph gives it.
    pub sizes: BTreeMap<String, Vec<usize>>,
}

/// Why an archive does not import.
#[derive(Debug)]
pub enum GraphError {
    Io(io::Error),
    /// An ATen op the importer does not map, by its name.
    Unmapped(String),
    /// What the importer reads but does not take, said in words.
    Unsupported(String),
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::Io(e) => write!(f, "{e}"),
            GraphError::Unmapped(op) => write!(f, "the exported graph holds `{op}`, an ATen op the importer does not map"),
            GraphError::Unsupported(what) => write!(f, "the exported graph {what}, which the importer does not take"),
        }
    }
}

impl std::error::Error for GraphError {}

impl From<io::Error> for GraphError {
    fn from(e: io::Error) -> GraphError {
        GraphError::Io(e)
    }
}

/// PyTorch's serialized dtype for `float32` (`ScalarType.FLOAT`).
const FLOAT32: u64 = 7;

/// The ATen ops the importer maps, by the name the graph gives each.
const MAPPED: &[&str] = &[
    "aten.linear.default",
    "aten.gelu.default",
    "aten.relu.default",
    "aten.silu.default",
    "aten.sigmoid.default",
    "aten.add.Tensor",
    "aten.mul.Tensor",
];

fn ints(v: &Value) -> Option<Vec<usize>> {
    v.as_array()?.iter().map(|s| s["as_int"].as_u64().map(|n| n as usize)).collect()
}

/// A node's argument named `name`.
fn arg<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node["inputs"].as_array()?.iter().find(|a| a["name"] == name).map(|a| &a["arg"])
}

/// The tensor a node's argument `name` names.
fn tensor_arg(node: &Value, name: &str) -> Option<String> {
    arg(node, name)?.get("as_tensor")?.get("name")?.as_str().map(String::from)
}

impl Graph {
    /// The graph the archive at `path` exports.
    pub fn open(path: &Path) -> Result<(Graph, Pt2), GraphError> {
        let pt2 = Pt2::open(path)?;
        let graph = Graph::import(&pt2)?;
        Ok((graph, pt2))
    }

    /// `pt2`'s exported program read into ops.
    pub fn import(pt2: &Pt2) -> Result<Graph, GraphError> {
        let model = pt2.json("models/model.json")?;
        let config = pt2.json("data/weights/model_weights_config.json")?;
        let g = &model["graph_module"]["graph"];
        let sig = &model["graph_module"]["signature"];
        let unsupported = |what: String| GraphError::Unsupported(what);
        let mut sizes = BTreeMap::new();
        for (name, meta) in g["tensor_values"].as_object().into_iter().flatten() {
            if meta["dtype"].as_u64() != Some(FLOAT32) {
                return Err(unsupported(format!("holds `{name}` of dtype {}, not float32", meta["dtype"])));
            }
            let s = ints(&meta["sizes"]).ok_or_else(|| unsupported(format!("gives `{name}` sizes that are not integers")))?;
            sizes.insert(name.clone(), s);
        }
        let (mut params, mut inputs) = (BTreeMap::new(), Vec::new());
        for spec in sig["input_specs"].as_array().into_iter().flatten() {
            if let Some(p) = spec.get("parameter") {
                let (placeholder, name) = (p["arg"]["name"].as_str(), p["parameter_name"].as_str());
                let (Some(placeholder), Some(name)) = (placeholder, name) else {
                    return Err(unsupported("names a parameter without its placeholder".into()));
                };
                let entry = &config["config"][name];
                let meta = &entry["tensor_meta"];
                if entry["use_pickle"].as_bool() != Some(false) || meta["dtype"].as_u64() != Some(FLOAT32) {
                    return Err(unsupported(format!("stores `{name}` other than as raw float32")));
                }
                let s = ints(&meta["sizes"]).ok_or_else(|| unsupported(format!("gives `{name}` no sizes")))?;
                let strides = ints(&meta["strides"]).ok_or_else(|| unsupported(format!("gives `{name}` no strides")))?;
                let contiguous: Vec<usize> = (0..s.len()).map(|i| s[i + 1..].iter().product()).collect();
                if strides != contiguous || meta["storage_offset"]["as_int"].as_u64().unwrap_or(0) != 0 {
                    return Err(unsupported(format!("stores `{name}` other than contiguously")));
                }
                let file = format!("data/weights/{}", entry["path_name"].as_str().unwrap_or_default());
                params.insert(placeholder.to_string(), Param { name: name.to_string(), file, sizes: s });
            } else if let Some(u) = spec.get("user_input") {
                let name = u["arg"]["as_tensor"]["name"].as_str().ok_or_else(|| unsupported("takes an input that is not a tensor".into()))?;
                inputs.push(name.to_string());
            } else {
                let kind = spec.as_object().and_then(|o| o.keys().next().cloned()).unwrap_or_default();
                return Err(unsupported(format!("takes an input of kind `{kind}` (a buffer or a constant)")));
            }
        }
        let [input] = inputs.as_slice() else {
            return Err(unsupported(format!("takes {} inputs where the importer takes one", inputs.len())));
        };
        let outputs: Vec<String> = sig["output_specs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["user_output"]["arg"]["as_tensor"]["name"].as_str().map(String::from))
            .collect();
        let [output] = outputs.as_slice() else {
            return Err(unsupported(format!("gives {} outputs where the importer takes one", outputs.len())));
        };
        let input_sizes = sizes.get(input).cloned().ok_or_else(|| unsupported(format!("gives its input `{input}` no sizes")))?;
        let size = |t: &str| sizes.get(t).cloned().unwrap_or_default();
        let mut ops = Vec::new();
        for node in g["nodes"].as_array().into_iter().flatten() {
            let target = node["target"].as_str().unwrap_or_default();
            let op = target.strip_prefix("torch.ops.").unwrap_or(target);
            if !MAPPED.contains(&op) {
                return Err(GraphError::Unmapped(op.to_string()));
            }
            let out = node["outputs"][0]["as_tensor"]["name"].as_str().map(String::from);
            let need = |name: &str| tensor_arg(node, name).ok_or_else(|| unsupported(format!("gives `{op}` no tensor `{name}`")));
            let Some(out) = out else {
                return Err(unsupported(format!("gives `{op}` an output that is not one tensor")));
            };
            let same = |a: &str, b: &str| -> Result<(), GraphError> {
                if size(a) != size(b) {
                    return Err(unsupported(format!("applies `{op}` to `{a}` {:?} and `{b}` {:?}, shapes it would broadcast", size(a), size(b))));
                }
                Ok(())
            };
            ops.push(match op {
                "aten.linear.default" => {
                    let (input, weight) = (need("input")?, need("weight")?);
                    let bias = tensor_arg(node, "bias");
                    let w = params.get(&weight).ok_or_else(|| unsupported(format!("multiplies by `{weight}`, which is not a parameter")))?;
                    let [out_features, in_features] = w.sizes[..] else {
                        return Err(unsupported(format!("multiplies by `{weight}` of sizes {:?}, not a matrix", w.sizes)));
                    };
                    if let Some(b) = &bias {
                        if params.get(b).map(|p| p.sizes.clone()) != Some(vec![out_features]) {
                            return Err(unsupported(format!("adds `{b}` as `{weight}`'s bias, not a parameter of its {out_features} outputs")));
                        }
                    }
                    GraphOp::Linear { out, input, weight, bias, in_features, out_features }
                }
                "aten.gelu.default" => {
                    let approximate = arg(node, "approximate").and_then(|a| a["as_string"].as_str()).unwrap_or("none");
                    let op = match approximate {
                        "none" => Unary::GeluExact,
                        "tanh" => Unary::GeluTanh,
                        other => return Err(unsupported(format!("approximates GELU as `{other}`"))),
                    };
                    GraphOp::Unary { out, input: need("self")?, op }
                }
                "aten.relu.default" => GraphOp::Unary { out, input: need("self")?, op: Unary::Relu },
                "aten.silu.default" => GraphOp::Unary { out, input: need("self")?, op: Unary::Silu },
                "aten.sigmoid.default" => GraphOp::Unary { out, input: need("self")?, op: Unary::Sigmoid },
                "aten.add.Tensor" => {
                    let (a, b) = (need("self")?, need("other")?);
                    same(&a, &b)?;
                    let alpha = arg(node, "alpha").and_then(|v| v["as_float"].as_f64().or_else(|| v["as_int"].as_i64().map(|i| i as f64))).unwrap_or(1.0);
                    GraphOp::Add { out, a, b, alpha }
                }
                "aten.mul.Tensor" => {
                    let (a, b) = (need("self")?, need("other")?);
                    same(&a, &b)?;
                    GraphOp::Mul { out, a, b }
                }
                other => return Err(GraphError::Unmapped(other.to_string())),
            });
        }
        Ok(Graph { input: input.clone(), input_sizes, params, ops, output: output.clone(), sizes })
    }
}

/// An exported graph's parameters as weights a host reads: each by its name in the module, float32 rows as the archive
/// stores them, its sizes reversed into the fastest-first order a checkpoint's dims take.
pub struct GraphWeights<'a> {
    pub graph: &'a Graph,
    pub pt2: &'a Pt2,
}

impl GraphWeights<'_> {
    fn param(&self, name: &str) -> io::Result<&Param> {
        self.graph
            .params
            .values()
            .find(|p| p.name == name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no parameter `{name}` in the exported graph")))
    }
}

impl WeightSource for GraphWeights<'_> {
    fn describe(&self, name: &str) -> io::Result<(GgmlType, Vec<u64>)> {
        let p = self.param(name)?;
        Ok((GgmlType::F32, p.sizes.iter().rev().map(|&s| s as u64).collect()))
    }

    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> io::Result<Vec<u8>> {
        let p = self.param(name)?;
        let row = 4 * p.sizes.last().copied().unwrap_or(1);
        self.pt2.read_range(&p.file, (first * row) as u64, count * row)
    }

    fn has(&self, name: &str) -> bool {
        self.param(name).is_ok()
    }
}

impl Graph {
    /// The graph's forward in `f64`, the oracle a lowering of it is held to: `x` its input row by row, every parameter
    /// read from `weights` as the archive stores it; its output row by row. A linear layer sums its products in the
    /// order of its inputs, an exact GELU reads the library's `erf`.
    pub fn forward(&self, weights: &mut dyn WeightSource, x: &[f64]) -> Result<Vec<f64>, GraphError> {
        let unsupported = |what: String| GraphError::Unsupported(what);
        let want: usize = self.input_sizes.iter().product();
        if x.len() != want {
            return Err(unsupported(format!("takes {want} input values, given {}", x.len())));
        }
        let mut held: BTreeMap<String, Vec<f64>> = BTreeMap::from([(self.input.clone(), x.to_vec())]);
        let mut param = |placeholder: &str, held: &mut BTreeMap<String, Vec<f64>>| -> Result<Vec<f64>, GraphError> {
            if let Some(v) = held.get(placeholder) {
                return Ok(v.clone());
            }
            let p = self.params.get(placeholder).ok_or_else(|| unsupported(format!("reads `{placeholder}` before any op writes it")))?;
            let rows: usize = p.sizes[..p.sizes.len().saturating_sub(1)].iter().product();
            let bytes = weights.row_bytes(&p.name, 0, rows.max(1))?;
            let v: Vec<f64> = bytes.chunks_exact(4).map(|b| f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))).collect();
            held.insert(placeholder.to_string(), v.clone());
            Ok(v)
        };
        let erf = crate::tensor_graph::eval::lib_erf;
        for op in &self.ops {
            let (out, v) = match op {
                GraphOp::Linear { out, input, weight, bias, in_features, out_features } => {
                    let (x, w) = (param(input, &mut held)?, param(weight, &mut held)?);
                    let b = bias.as_deref().map(|b| param(b, &mut held)).transpose()?;
                    let rows = x.len() / in_features;
                    let mut y = vec![0.0; rows * out_features];
                    for r in 0..rows {
                        for o in 0..*out_features {
                            let dot = (0..*in_features).fold(0.0, |acc, i| acc + x[r * in_features + i] * w[o * in_features + i]);
                            y[r * out_features + o] = dot + b.as_ref().map_or(0.0, |b| b[o]);
                        }
                    }
                    (out, y)
                }
                GraphOp::Unary { out, input, op } => {
                    let x = param(input, &mut held)?;
                    let f = |v: f64| match op {
                        Unary::GeluExact => 0.5 * v * (1.0 + erf(v * std::f64::consts::FRAC_1_SQRT_2)),
                        Unary::GeluTanh => 0.5 * v * (1.0 + (0.797_884_560_802_865_4 * (v + 0.044_715 * v * v * v)).tanh()),
                        Unary::Relu => v.max(0.0),
                        Unary::Silu => v / (1.0 + (-v).exp()),
                        Unary::Sigmoid => 1.0 / (1.0 + (-v).exp()),
                    };
                    (out, x.into_iter().map(f).collect())
                }
                GraphOp::Add { out, a, b, alpha } => {
                    let (a, b) = (param(a, &mut held)?, param(b, &mut held)?);
                    (out, a.iter().zip(&b).map(|(a, b)| a + alpha * b).collect())
                }
                GraphOp::Mul { out, a, b } => {
                    let (a, b) = (param(a, &mut held)?, param(b, &mut held)?);
                    (out, a.iter().zip(&b).map(|(a, b)| a * b).collect())
                }
            };
            held.insert(out.clone(), v);
        }
        held.remove(&self.output).ok_or_else(|| unsupported(format!("gives `{}` as its output, which no op computes", self.output)))
    }
}
