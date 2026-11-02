//! The layer tour: each English statement of one layer of a model's one program, and the launch the CUDA lane's
//! decode step computes it in, with the fusions that made that launch.
//!
//! The statements are the layer's own cut of the program (`Cut::DecodeLayer`); each names the tensor it writes, and
//! the decode step's graph names the op that writes it. The launch is the one whose covered ops hold that op.

use std::collections::BTreeMap;

use logicaffeine_compile::codegen_cuda::lane::plan::kind;
use logicaffeine_compile::codegen_cuda::lane::Lane;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::spec::ModelSpec;

/// One statement's stop on the CUDA lane.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub statement: String,
    pub writes: String,
    pub launch: usize,
    pub kind: &'static str,
    pub rules: Vec<String>,
    pub with: Vec<String>,
}

/// Whether a statement declares rather than computes: a tensor declaration (`Let W be a R by C tensor of …`, a weight,
/// a state or the cut's input) or one of the cut's placeholder scalars (`Let pos be 1.`).
pub fn declares(statement: &str) -> bool {
    let Some(rest) = statement.strip_prefix("Let ") else { return false };
    let Some((_, value)) = rest.split_once(" be ") else { return false };
    let value = value.trim_end_matches('.');
    value.contains(" tensor of ") || value.parse::<i64>().is_ok()
}

/// The tensor a statement writes: `Let X be …` writes X; `Set row … of T to …` writes T.
fn writes(statement: &str) -> Option<String> {
    if let Some(rest) = statement.strip_prefix("Let ") {
        return rest.split(" be ").next().map(|s| s.trim().to_string());
    }
    if let Some(rest) = statement.strip_prefix("Set row ") {
        let of = rest.split(" of ").nth(1)?;
        return of.split(" to ").next().map(|s| s.trim().to_string());
    }
    None
}

/// The tour of layer `layer`: one stop for every statement of its cut, in the cut's order.
pub fn tour(spec: &ModelSpec, lane: &Lane, layer: usize, ctx: usize) -> Result<Vec<Stop>, String> {
    let ((text, _, _), _) = written(spec, Cut::DecodeLayer { layer, ctx });
    let form = lane.decode();
    let graph = &form.graph;
    let s = &form.schedule;
    let mut by_out: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, op) in graph.ops.iter().enumerate() {
        by_out.entry(op.out.as_str()).or_default().push(i);
    }
    let mut stops = Vec::new();
    for statement in text
        .lines()
        .map(str::trim)
        .filter(|l| (l.starts_with("Let ") || l.starts_with("Set row ")) && !declares(l))
    {
        let out = writes(statement).ok_or_else(|| format!("`{statement}`: names no tensor it writes"))?;
        let ops = by_out.get(out.as_str()).ok_or_else(|| format!("`{statement}`: no op of the decode step writes {out}"))?;
        let op = *ops.last().unwrap();
        let launches: Vec<usize> = (0..s.ops.len()).filter(|&l| s.ops[l].contains(&op)).collect();
        let [launch] = launches.as_slice() else {
            return Err(format!("`{statement}`: its op {op} rides {} launches ({launches:?})", launches.len()));
        };
        let mut rules: Vec<String> =
            s.fusions.iter().filter(|f| f.ops.contains(&op)).map(|f| f.rule.to_string()).collect();
        rules.sort();
        rules.dedup();
        let mut with: Vec<String> = s.ops[*launch]
            .iter()
            .filter(|&&o| o != op)
            .map(|&o| graph.ops[o].out.clone())
            .collect();
        with.sort();
        with.dedup();
        stops.push(Stop { statement: statement.to_string(), writes: out, launch: *launch, kind: kind(&s.launches[*launch]), rules, with });
    }
    Ok(stops)
}

/// How many statements of layer `layer`'s cut declare a weight, a state, an input or a placeholder.
pub fn declarations(spec: &ModelSpec, layer: usize, ctx: usize) -> usize {
    let ((text, _, _), _) = written(spec, Cut::DecodeLayer { layer, ctx });
    text.lines().map(str::trim).filter(|l| declares(l)).count()
}

/// The tour as the slide's table.
pub fn to_markdown(title: &str, declared: usize, stops: &[Stop]) -> String {
    let mut out = format!(
        "## {title}\n\n{} statements compute; {declared} declare its weights, state and inputs.\n\n\
         | LOGOS | CUDA launch | fused by | in the same launch |\n|---|---|---|---|\n",
        stops.len()
    );
    for s in stops {
        out.push_str(&format!(
            "| `{}` | #{} `{}` | {} | {} |\n",
            s.statement.replace('|', "\\|"),
            s.launch,
            s.kind,
            if s.rules.is_empty() { "—".to_string() } else { s.rules.join(", ") },
            if s.with.is_empty() { "—".to_string() } else { s.with.join(", ") },
        ));
    }
    out
}
