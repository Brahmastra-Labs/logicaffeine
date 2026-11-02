//! Show the decode step a checkpoint's model runs as the shared front end lifts it: the identity of the model's one
//! program the step is a cut of, every op of its step graph (`codegen_cuda::lane::graph::StepGraph`) with its operands
//! and its result, and the step's tensors by role — the graph the CUDA lane compiles, and the one the D5005 engine's
//! program is read off.
//!
//!     d5005_step_graph <model.gguf> [--ctx <positions>] [--ops <n>] [--source <file>]
//!
//! `--ctx` is the context the step's caches hold (64 unless given); `--ops` prints only the first `n` ops; `--source`
//! writes the LOGOS text of the step to `file`.

use std::path::Path;
use std::process::exit;

use logicaffeine_compile::codegen_cuda::lane::graph::{Role, StepGraph};
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_step_graph: {why}");
    exit(1)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: d5005_step_graph <model.gguf> [--ctx <positions>] [--ops <n>] [--source <file>]");
        exit(2)
    };
    let (mut ctx, mut ops, mut out) = (64usize, usize::MAX, None);
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().unwrap_or_else(|| fail(format!("{flag} takes a value")));
        let number = || value.parse::<usize>().unwrap_or_else(|_| fail(format!("{flag}: not a number: {value}")));
        match flag.as_str() {
            "--ctx" => ctx = number(),
            "--ops" => ops = number(),
            "--source" => out = Some(value.clone()),
            _ => fail(format!("unknown flag {flag}")),
        }
    }
    let g = Gguf::open(Path::new(path)).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| fail(format!("{path}: {e:?}")));
    author::check(&spec).unwrap_or_else(|e| fail(format!("the language cannot write this model yet: {e:?}")));
    let ((source, _, _), identity) = written(&spec, Cut::Decode { ctx });
    if let Some(file) = &out {
        std::fs::write(file, &source).unwrap_or_else(|e| fail(format!("{file}: {e}")));
    }
    let rp = recognize_stages(&source).unwrap_or_else(|e| fail(format!("the authored step: {e:?}")));
    let graph = StepGraph::build(&rp).unwrap_or_else(|e| fail(e));
    println!("{}", identity.line());
    println!("{path}: {} layers, {} ops, {} tensors", spec.layers.len(), graph.ops.len(), graph.tensors.len());
    for role in [Role::Param, Role::State, Role::Constant, Role::Activation] {
        println!("  {role:?}: {}", graph.of_role(role).count());
    }
    for (i, op) in graph.ops.iter().enumerate().take(ops) {
        let shape = |name: &str| graph.tensors.get(name).map_or("?".into(), |t| format!("{}×{}", t.rows, t.cols));
        let ins: Vec<String> = op.ins.iter().map(|x| format!("{x} {}", shape(x))).collect();
        println!("  {i:5} {:12} {} -> {} {}", op.spec.kind(), ins.join(", "), op.out, shape(&op.out));
    }
}
