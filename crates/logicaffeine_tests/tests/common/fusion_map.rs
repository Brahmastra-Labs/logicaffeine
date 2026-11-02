//! The fusion map: which fused symbols of the book's section 3.4 each chip's lowering of a model fires, read from
//! the lowerings themselves.
//!
//! - The CUDA lane: every fusion its decode step's schedule records (`Schedule::fusions`), each rule read as the
//!   symbol it builds. A quantize folded into its producer is the producer's `*_quant` symbol.
//! - The D5005 engine: the merges `lower_graph` makes, read off its program's commands.
//!
//! Each fired rule cites the census tests that hold it: the `lane_compile_lock` tests whose bodies name it.

use std::collections::BTreeMap;
use std::path::Path;

use logicaffeine_compile::codegen_cuda::lane::plan::{kind, Launch, Schedule};
use logicaffeine_compile::codegen_cuda::lane::Lane;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::decode_engine::capability::{refusals, summarize};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::program::{lower, Command, Slot};

/// What a lane rule builds: a symbol of section 3.4, a quantize folded into its producer, or a schedule property.
#[derive(Clone, Copy, Debug)]
pub enum Meaning {
    Symbol(&'static str),
    FoldedQuant,
    Schedule(&'static str),
}

/// Every rule the CUDA lane records, and what it builds (section 5.9 of the book).
pub const LANE_RULES: &[(&str, Meaning)] = &[
    ("add_norm", Meaning::Symbol("add_norm")),
    ("sandwich_norm", Meaning::Symbol("norm_then_add")),
    ("gemv_bias", Meaning::Symbol("gemm_bias")),
    ("gemv_norm_epilogue", Meaning::Symbol("gemv_norm_epilogue")),
    ("head_norm_rope", Meaning::Symbol("qk_norm_rope")),
    ("slice_read", Meaning::Symbol("strided_read")),
    ("row_write", Meaning::Symbol("row_write")),
    ("qkv_head_epilogue", Meaning::Symbol("qkv_head_epilogue")),
    ("join_in_place", Meaning::Symbol("join_in_place")),
    ("conv_silu", Meaning::Symbol("conv_bias_silu")),
    ("conv_silu_l2", Meaning::Symbol("conv_bias_silu")),
    ("conv_parts", Meaning::Symbol("conv_parts")),
    ("gates", Meaning::Symbol("gdn_gating")),
    ("gate_projections", Meaning::Symbol("gdn_gate_projections")),
    ("act_gate", Meaning::Symbol("glu_act_mul")),
    ("attn_output_gate", Meaning::Symbol("attn_output_gate")),
    ("glu_pair", Meaning::Symbol("gemv_gate_up_glu")),
    ("shared_expert", Meaning::Symbol("shared_expert_fusion")),
    ("moe_rows", Meaning::Symbol("moe_token_sort")),
    ("moe_deferred_finalize", Meaning::Symbol("moe_deferred_finalize")),
    ("gather_scale", Meaning::Symbol("embed_scale")),
    ("embed_norm", Meaning::Symbol("embed_norm")),
    ("strided_values", Meaning::Symbol("strided_values")),
    ("rope_parts", Meaning::Symbol("rope_qk_one_launch")),
    ("gemv_parts", Meaning::Symbol("merged_qkv")),
    ("argmax_advance", Meaning::Symbol("fused_sampling")),
    ("fused_quant", Meaning::FoldedQuant),
    ("rows_a_warp", Meaning::Schedule("rows a warp")),
    ("sequences", Meaning::Schedule("a batch's per-sequence launches as one")),
];

/// The symbol a lane rule builds in `schedule`, or why it builds none. A folded quantize is named by the launch it
/// folded into.
pub fn lane_symbol(rule: &str, ops: &[usize], schedule: &Schedule) -> Result<Option<&'static str>, String> {
    let meaning = LANE_RULES
        .iter()
        .find(|(r, _)| *r == rule)
        .map(|(_, m)| *m)
        .ok_or_else(|| format!("the lane rule `{rule}` has no meaning in LANE_RULES"))?;
    match meaning {
        Meaning::Symbol(s) => Ok(Some(s)),
        Meaning::Schedule(_) => Ok(None),
        Meaning::FoldedQuant => {
            let launch = schedule
                .ops
                .iter()
                .position(|covered| ops.iter().any(|o| covered.contains(o)))
                .ok_or_else(|| format!("a folded quantize over ops {ops:?} rides no launch"))?;
            // A fold quantizes in the part that writes its result.
            let host = match &schedule.launches[launch] {
                Launch::Binary { gate: Some(_), .. } => "gated_binary",
                Launch::Fold { then, .. } => kind(then),
                l => kind(l),
            };
            let with_add = schedule
                .fusions
                .iter()
                .any(|f| matches!(f.rule, "add_norm" | "sandwich_norm") && f.ops.iter().any(|o| schedule.ops[launch].contains(o)));
            match host {
                "rmsnorm" if with_add => Ok(Some("add_norm_quant")),
                "rmsnorm" => Ok(Some("norm_quant")),
                "attention" => Ok(Some("attn_output_quant")),
                "swiglu" | "unary" | "gemv_glu" | "gated_binary" => Ok(Some("glu_act_mul_quant")),
                "delta" => Ok(Some("gated_norm")),
                "set_row" => Ok(Some("row_write_quant")),
                other => Err(format!("a quantize folded into a `{other}` launch has no symbol")),
            }
        }
    }
}

/// One chip's cell for one model: the symbols it fired with their counts, the rules behind them, or why it refused.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub identity: String,
    pub symbols: BTreeMap<String, usize>,
    pub rules: BTreeMap<String, usize>,
    pub refused: Option<String>,
}

/// The CUDA lane's cell: its decode step's fusions.
pub fn cuda_cell(lane: &Lane) -> Result<Cell, String> {
    let form = lane.decode();
    let s = &form.schedule;
    let mut cell = Cell { identity: form.identity.line(), ..Cell::default() };
    for f in &s.fusions {
        *cell.rules.entry(f.rule.to_string()).or_insert(0) += 1;
        if let Some(sym) = lane_symbol(f.rule, &f.ops, s)? {
            *cell.symbols.entry(sym.to_string()).or_insert(0) += 1;
        }
    }
    Ok(cell)
}

/// The D5005 engine's cell: the merges its program's commands carry, or the engine's refusal by name.
pub fn d5005_cell(spec: &ModelSpec, checkpoint: &Path, ctx: usize) -> Cell {
    let g = Gguf::open(checkpoint).unwrap_or_else(|e| panic!("{}: {e}", checkpoint.display()));
    let tensors: Vec<(String, GgmlType)> = g
        .tensors()
        .iter()
        .filter_map(|t| GgmlType::from_id(t.ggml_type).ok().map(|ty| (t.name.clone(), ty)))
        .collect();
    let gaps = refusals(spec, &tensors);
    if !gaps.is_empty() {
        return Cell { refused: Some(summarize(&gaps, spec.layers.len()).join("; ")), ..Cell::default() };
    }
    let lowered = match lower(spec, EngineNumerics::fastest(), ctx) {
        Ok(l) => l,
        Err(e) => return Cell { refused: Some(e.to_string()), ..Cell::default() },
    };
    let mut cell = Cell { identity: lowered.identity.line(), ..Cell::default() };
    for c in &lowered.program.commands {
        let (rule, symbol) = match c {
            Command::Matvec { to: Slot::Qkv, .. } => ("Matvec into Qkv", "merged_qkv"),
            Command::Conv { .. } => ("Conv", "conv_bias_silu"),
            Command::Gates { .. } => ("Gates", "gdn_gating"),
            Command::SigmoidGate { .. } => ("SigmoidGate", "glu_act_mul"),
            Command::CacheAppend { .. } => ("CacheAppend", "row_write"),
            Command::GatedNorm { .. } => ("GatedNorm", "gated_norm"),
            Command::Hadamard { .. } => ("Hadamard", "hadamard_rotation"),
            Command::Advance => ("Advance", "fused_sampling"),
            _ => continue,
        };
        *cell.rules.entry(rule.to_string()).or_insert(0) += 1;
        *cell.symbols.entry(symbol.to_string()).or_insert(0) += 1;
    }
    cell
}

/// The `lane_compile_lock` tests whose bodies name `rule` (a `fired` count, a census table, a held rule): the census
/// behind a CUDA cell. Only `#[test]` functions count; a helper that names a rule asserts nothing by itself.
pub fn lane_census(rule: &str) -> Vec<String> {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lane_compile_lock.rs"))
        .expect("lane_compile_lock.rs");
    let needle = format!("\"{rule}\"");
    let mut tests = Vec::new();
    let mut current: Option<String> = None;
    let mut attributed_test = false;
    for line in src.lines() {
        if line.starts_with("#[test]") {
            attributed_test = true;
        } else if let Some(rest) = line.strip_prefix("fn ") {
            current = if attributed_test { rest.split('(').next().map(str::to_string) } else { None };
            attributed_test = false;
        } else if !line.starts_with("#[") && !line.starts_with("///") && !line.is_empty() && !line.starts_with(' ') && line != "}" {
            current = None;
        }
        if line.contains(&needle) {
            if let Some(t) = &current {
                if !tests.contains(t) {
                    tests.push(t.clone());
                }
            }
        }
    }
    tests.into_iter().map(|t| format!("lane_compile_lock::{t}")).collect()
}

/// The map as JSON: per model, per chip, the cell; per fired lane rule, its census.
pub fn to_json(models: &BTreeMap<String, BTreeMap<&'static str, Cell>>) -> String {
    let mut root = serde_json::Map::new();
    root.insert(
        "doc".into(),
        "Which fused symbols of benchmarks/FUSIONS_WE_NEED.md section 3.4 each chip's lowering of each model fires, \
         generated by fusion_map_lock from the lowerings (CUDA: the decode step's Schedule::fusions; D5005: the \
         engine program's commands). Never edited by hand."
            .into(),
    );
    let mut census = serde_json::Map::new();
    let mut ms = serde_json::Map::new();
    for (model, chips) in models {
        let mut cs = serde_json::Map::new();
        for (chip, cell) in chips {
            let mut c = serde_json::Map::new();
            if let Some(why) = &cell.refused {
                c.insert("refused".into(), why.clone().into());
            } else {
                c.insert("identity".into(), cell.identity.clone().into());
                c.insert("symbols".into(), serde_json::to_value(&cell.symbols).unwrap());
                c.insert("rules".into(), serde_json::to_value(&cell.rules).unwrap());
                if *chip == "cuda" {
                    for rule in cell.rules.keys() {
                        census.entry(rule.clone()).or_insert_with(|| serde_json::to_value(lane_census(rule)).unwrap());
                    }
                }
            }
            cs.insert(chip.to_string(), serde_json::Value::Object(c));
        }
        ms.insert(model.clone(), serde_json::Value::Object(cs));
    }
    root.insert("models".into(), serde_json::Value::Object(ms));
    root.insert("lane_census".into(), serde_json::Value::Object(census));
    serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap() + "\n"
}
