//! sol_solution_lock — the SOL-ExecBench submission artifact (`solution.json` + torch-binding CUDA kernel).
//!
//! `codegen_cuda::solbench` turns a recognized fused-row op into the exact artifact NVIDIA's SOL-ExecBench
//! evaluates (`sol-execbench <dir> --solution solution.json`): a manifest with `languages:["cuda_cpp"]`, an
//! `entry_point` `"kernel.cu::run"`, destination-passing-style, and a `void run(torch::Tensor inputs…,
//! torch::Tensor output)` binding that writes the pre-allocated output in place. This lock pins the artifact
//! to that schema and proves the DPS binding arity/shape tracks the recognized op — so a malformed or
//! non-conforming solution fails RED. (The kernel MATH is the same one GPU-verified ≡CPU in cuda_fused_row_lock;
//! the actual B200 build+score is a gated follow-up.)
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(sol_solution_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::{emit_fused_row_solution, CudaChip, CudaKnobs};
use logicaffeine_compile::compile::compile_to_sol_solution;
use logicaffeine_compile::intern::Symbol;
use logicaffeine_compile::tensor_recognize::{Epsilon, FusedRowSpec, RowReduceKind};

/// A LOGOS fused-add-RMSNorm program (the FIB-001 shape) — English straight to a submittable kernel.
const FUSED_ADD_LOGOS: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let R be a 2 by 3 tensor of Real.
Set item 1 of R to 0.5.
Set item 2 of R to 0.5.
Set item 3 of R to 0.5.
Set item 4 of R to 1.0.
Set item 5 of R to 1.0.
Set item 6 of R to 1.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X plus R with weight W.
Show round(item 1 of Y * 1000000).
"#;

fn base(rows: usize, cols: usize) -> FusedRowSpec {
    FusedRowSpec {
        rows,
        cols,
        reduce: RowReduceKind::RmsNorm,
        x: Symbol::EMPTY,
        y: Symbol::EMPTY,
        weight: None,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps: Epsilon::DEFAULT,
    }
}
fn fused_add() -> FusedRowSpec {
    FusedRowSpec {
        weight: Some(Symbol::EMPTY),
        residual: Some(Symbol::EMPTY),
        ..base(4, 2048)
    }
}
fn layernorm() -> FusedRowSpec {
    FusedRowSpec {
        reduce: RowReduceKind::LayerNorm,
        weight: Some(Symbol::EMPTY),
        bias: Some(Symbol::EMPTY),
        ..base(4, 2048)
    }
}

/// Undo solbench's JSON string-escaping (the inverse of its `json_escape`), so we can prove the embedded
/// source round-trips to exactly the emitted kernel.
fn json_unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn the_solution_manifest_conforms_to_the_sol_execbench_schema() {
    let sol = emit_fused_row_solution(
        &fused_add(),
        &CudaKnobs::default(),
        "001_fused_add_rmsnorm_h2048",
        &["hidden_states", "residual", "weight"],
        "output",
    );
    let j = &sol.solution_json;
    assert!(
        j.contains(r#""definition": "001_fused_add_rmsnorm_h2048""#),
        "names the problem: {j}"
    );
    assert!(
        j.contains(r#""languages": ["cuda_cpp"]"#),
        "cuda_cpp language"
    );
    assert!(
        j.contains(r#""entry_point": "kernel.cu::run""#),
        "file::fn entry point"
    );
    assert!(
        j.contains(r#""destination_passing_style": true"#),
        "DPS (output written in place)"
    );
    assert!(
        j.contains(r#""target_hardware": ["B200", "LOCAL""#),
        "targets the B200 (and local)"
    );
    assert!(
        j.contains(r#""path": "kernel.cu""#),
        "sources include the entry-point file"
    );
    // Balanced braces (a cheap well-formedness check without pulling in a JSON dep).
    assert_eq!(
        j.matches('{').count(),
        j.matches('}').count(),
        "JSON braces balance"
    );
    // The embedded source round-trips to exactly the emitted kernel.
    let start = j.find(r#""content": ""#).expect("has content") + r#""content": ""#.len();
    let end = j[start..].rfind(r#"" }"#).expect("content close") + start;
    assert_eq!(
        json_unescape(&j[start..end]),
        sol.kernel_cu,
        "embedded source == the kernel, escaped losslessly"
    );
}

#[test]
fn the_run_binding_is_destination_passing_style_with_the_right_inputs() {
    let sol = emit_fused_row_solution(
        &fused_add(),
        &CudaKnobs::default(),
        "001_fused_add_rmsnorm_h2048",
        &["hidden_states", "residual", "weight"],
        "output",
    );
    let cu = &sol.kernel_cu;
    assert!(cu.contains("#include <torch/extension.h>"), "torch binding");
    // Inputs in Definition order, then the output — all torch::Tensor.
    assert!(
        cu.contains("void run(torch::Tensor hidden_states, torch::Tensor residual, torch::Tensor weight, torch::Tensor output)"),
        "DPS signature (inputs then output): {cu}"
    );
    // Runtime-shaped (rows/cols are NOT baked #defines) and writes the output back in the output's dtype.
    assert!(
        cu.contains("int cols = x_.size(x_.dim()-1)"),
        "cols is a runtime arg"
    );
    assert!(
        cu.contains("output.copy_(y_.to(output.dtype())"),
        "writes the pre-allocated output in place"
    );
    // Fuses the residual add and the weight — the FIB-001 math.
    assert!(
        cu.contains("(xr[i]+rr[i])") && cu.contains("*scale*w[i]"),
        "rmsnorm(X+R)*W"
    );
    // Up-casts the (bf16) inputs to f32 for the reduction (the reference computes in f32).
    assert!(
        cu.contains(".to(torch::kFloat32).contiguous()"),
        "up-casts inputs to f32"
    );
}

#[test]
fn the_binding_arity_tracks_the_recognized_op() {
    // Plain weighted RMSNorm: x, weight, output (no residual).
    let w = emit_fused_row_solution(
        &FusedRowSpec {
            weight: Some(Symbol::EMPTY),
            ..base(4, 8)
        },
        &CudaKnobs::default(),
        "rmsnorm_x",
        &["x", "weight"],
        "output",
    );
    assert!(
        w.kernel_cu
            .contains("void run(torch::Tensor x, torch::Tensor weight, torch::Tensor output)"),
        "weighted: {}",
        w.kernel_cu
    );
    assert!(!w.kernel_cu.contains("residual"), "no residual input");

    // LayerNorm: x, weight, bias, output — and the scale+shift math.
    let ln = emit_fused_row_solution(
        &layernorm(),
        &CudaKnobs::default(),
        "layernorm_x",
        &["x", "weight", "bias"],
        "output",
    );
    assert!(ln.kernel_cu.contains("void run(torch::Tensor x, torch::Tensor weight, torch::Tensor bias, torch::Tensor output)"), "layernorm arity: {}", ln.kernel_cu);
    assert!(
        ln.kernel_cu.contains("*scale*w[i]+b[i]"),
        "layernorm scale + shift"
    );
}

#[test]
fn a_logos_program_compiles_end_to_end_into_a_submittable_solution() {
    // The whole point: English LOGOS -> the recognized op -> a valid SOL-ExecBench artifact, no hand-writing.
    let sol = compile_to_sol_solution(
        FUSED_ADD_LOGOS,
        "001_fused_add_rmsnorm_h2048",
        &["hidden_states", "residual", "weight"],
        "output",
    )
    .expect("LOGOS fused-add-rmsnorm must compile into a solution");
    assert!(sol.solution_json.contains(r#""languages": ["cuda_cpp"]"#));
    assert!(sol
        .solution_json
        .contains(r#""entry_point": "kernel.cu::run""#));
    assert!(
        sol.kernel_cu.contains("void run(torch::Tensor hidden_states, torch::Tensor residual, torch::Tensor weight, torch::Tensor output)"),
        "DPS binding from the recognized op: {}", sol.kernel_cu
    );
    assert!(
        sol.kernel_cu.contains("(xr[i]+rr[i])") && sol.kernel_cu.contains("*scale*w[i]"),
        "fused-add math"
    );

    // A non-fused-row program is a typed refusal, not a bogus solution.
    assert!(compile_to_sol_solution("## Main\n    Show 6 * 7.\n", "d", &["x"], "output").is_err());
}

#[test]
fn the_target_arch_flag_comes_from_the_chip_knob() {
    let local = emit_fused_row_solution(
        &fused_add(),
        &CudaKnobs::default(),
        "d",
        &["x", "residual", "weight"],
        "output",
    );
    let cloud = emit_fused_row_solution(
        &fused_add(),
        &CudaKnobs {
            chip: CudaChip::b200(),
            ..Default::default()
        },
        "d",
        &["x", "residual", "weight"],
        "output",
    );
    assert!(
        local.solution_json.contains("-arch=sm_86"),
        "local build targets the 3060"
    );
    assert!(
        cloud.solution_json.contains("-arch=sm_100a"),
        "cloud build targets the B200"
    );
    assert_ne!(local.solution_json, cloud.solution_json);
}
