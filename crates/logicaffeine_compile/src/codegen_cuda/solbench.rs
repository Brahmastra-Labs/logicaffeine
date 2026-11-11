//! SOL-ExecBench harness bridge — turn a recognized LOGOS fused-row op into a submittable **`solution.json`**
//! plus a torch-binding CUDA kernel that conforms to the benchmark's ABI.
//!
//! The benchmark evaluates `sol-execbench <problem_dir> --solution solution.json`. A CUDA/C++ solution is a
//! JSON manifest referencing source files, with an `entry_point` `"file::fn"`. The entry point is a
//! **Destination-Passing-Style** binding — `void run(torch::Tensor inputs…, torch::Tensor output)` — that
//! takes the inputs (in Definition order) by value and writes the pre-allocated output in place. The harness
//! materializes the reference `run()` (PyTorch) and compares within `(atol, rtol, matched_ratio)` on a B200.
//!
//! Our emitters produce a *self-contained checksum* program for the ≡-oracle GPU locks; this bridge wraps the
//! SAME kernel math in the harness ABI instead: a runtime-shaped kernel (rows/cols are kernel args, not baked)
//! and a `run` binding that up-casts the (bf16/…) inputs to f32, launches, and writes the output back.

use crate::codegen_cuda::knobs::CudaKnobs;
use crate::tensor_recognize::{FusedRowSpec, RowReduceKind};

/// A generated SOL-ExecBench solution: the manifest and the CUDA source it references.
#[derive(Debug, Clone)]
pub struct SolSolution {
    /// The `solution.json` manifest (the `--solution` artifact).
    pub solution_json: String,
    /// The CUDA source referenced by `sources[0].content` (kernel + `run` binding).
    pub kernel_cu: String,
    /// The `entry_point` — `"kernel.cu::run"`.
    pub entry_point: String,
}

/// Emit a SOL-ExecBench solution for a recognized fused-row op (RMSNorm plain/weighted/fused-add, or LayerNorm).
/// `definition` is the problem's Definition name; `inputs` the input tensor names in Definition order; `output`
/// the output name. The emitted `run` binding matches that order (DPS).
pub fn emit_fused_row_solution(
    spec: &FusedRowSpec,
    knobs: &CudaKnobs,
    definition: &str,
    inputs: &[&str],
    output: &str,
) -> SolSolution {
    assert_eq!(spec.weight_rows, 1, "the SOL-ExecBench kernel reads one weight row; `compile_to_sol_solution` refuses a norm in lanes");
    let chip = knobs.chip;
    let layernorm = matches!(spec.reduce, RowReduceKind::LayerNorm);
    let residual = !layernorm && spec.residual.is_some();
    let weighted = layernorm || spec.weight.is_some();
    let bias = layernorm;
    let eps = format!("{:e}f", spec.eps.get() as f32);

    // Kernel: one block per row, runtime `cols`, shared-memory tree reduction over a fixed 256-wide block.
    let kernel_math = if layernorm {
        // two-pass mean/variance, then (x-mean)*scale*w + b
        r#"    float s = 0.0f;
    for(int i=threadIdx.x;i<cols;i+=blockDim.x) s += xr[i];
    sh[threadIdx.x]=s; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
    float mean = sh[0]/(float)cols; __syncthreads();
    float v = 0.0f;
    for(int i=threadIdx.x;i<cols;i+=blockDim.x){ float d=xr[i]-mean; v+=d*d; }
    sh[threadIdx.x]=v; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
    float scale = rsqrtf(sh[0]/(float)cols + EPS);
    for(int i=threadIdx.x;i<cols;i+=blockDim.x) yr[i] = (xr[i]-mean)*scale*w[i]+b[i];"#
            .replace("EPS", &eps)
    } else {
        let elem = if residual { "(xr[i]+rr[i])" } else { "xr[i]" };
        let wmul = if weighted { "*w[i]" } else { "" };
        format!(
            r#"    float s = 0.0f;
    for(int i=threadIdx.x;i<cols;i+=blockDim.x){{ float e={elem}; s+=e*e; }}
    sh[threadIdx.x]=s; __syncthreads();
    for(int k=blockDim.x/2;k>0;k>>=1){{ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }}
    float scale = rsqrtf(sh[0]/(float)cols + {eps});
    for(int i=threadIdx.x;i<cols;i+=blockDim.x){{ float e={elem}; yr[i] = e*scale{wmul}; }}"#,
            elem = elem,
            wmul = wmul,
            eps = eps,
        )
    };

    // Kernel signature + row-pointer prologue for whichever inputs the op consumes.
    let kparams = {
        let mut p = String::from("const float* x");
        if residual {
            p.push_str(", const float* res");
        }
        if weighted {
            p.push_str(", const float* w");
        }
        if bias {
            p.push_str(", const float* b");
        }
        p.push_str(", float* y, int rows, int cols");
        p
    };
    let res_row = if residual {
        "    const float* rr = res + (long)row*cols;\n"
    } else {
        ""
    };

    // `run` binding: convert each Definition input to f32/contiguous, launch, write output back in its dtype.
    // Inputs are (in Definition order) x, [residual], [weight], [bias].
    let mut run_params = Vec::new();
    let mut conversions = String::new();
    let mut kernel_args = Vec::new();
    let mut it = inputs.iter();
    let xn = it.next().copied().unwrap_or("x");
    run_params.push(format!("torch::Tensor {xn}"));
    conversions.push_str(&format!(
        "    auto x_ = {xn}.to(torch::kFloat32).contiguous();\n"
    ));
    kernel_args.push("x_.data_ptr<float>()".to_string());
    if residual {
        let rn = it.next().copied().unwrap_or("residual");
        run_params.push(format!("torch::Tensor {rn}"));
        conversions.push_str(&format!(
            "    auto res_ = {rn}.to(torch::kFloat32).contiguous();\n"
        ));
        kernel_args.push("res_.data_ptr<float>()".to_string());
    }
    if weighted {
        let wn = it.next().copied().unwrap_or("weight");
        run_params.push(format!("torch::Tensor {wn}"));
        conversions.push_str(&format!(
            "    auto w_ = {wn}.to(torch::kFloat32).contiguous();\n"
        ));
        kernel_args.push("w_.data_ptr<float>()".to_string());
    }
    if bias {
        let bn = it.next().copied().unwrap_or("bias");
        run_params.push(format!("torch::Tensor {bn}"));
        conversions.push_str(&format!(
            "    auto b_ = {bn}.to(torch::kFloat32).contiguous();\n"
        ));
        kernel_args.push("b_.data_ptr<float>()".to_string());
    }
    run_params.push(format!("torch::Tensor {output}"));
    kernel_args.push("y_.data_ptr<float>()".to_string());
    kernel_args.push("rows".to_string());
    kernel_args.push("cols".to_string());

    let kernel_cu = format!(
        r#"// Auto-generated by LOGOS codegen_cuda::solbench — a recognized LOGOS {op} as a SOL-ExecBench solution.
// Target: {chip_name} (sm_{arch}). Destination-passing-style torch binding; runtime rows/cols.
#include <torch/extension.h>
#include <cuda_runtime.h>
#include <cmath>

__global__ void fused_row({kparams}){{
    int row = blockIdx.x; if(row>=rows) return;
    const float* xr = x + (long)row*cols;
{res_row}    float* yr = y + (long)row*cols;
    __shared__ float sh[256];
{kernel_math}
}}

void run({run_params}){{
{conversions}    int rows = x_.size(0);
    int cols = x_.size(x_.dim()-1);
    auto y_ = torch::empty_like(x_);
    fused_row<<<rows, 256>>>({kernel_args});
    cudaDeviceSynchronize();
    {output}.copy_(y_.to({output}.dtype()).view_as({output}));
}}
"#,
        op = spec.reduce.name(),
        chip_name = chip.name,
        arch = chip.sm_arch,
        kparams = kparams,
        res_row = res_row,
        kernel_math = kernel_math,
        run_params = run_params.join(", "),
        conversions = conversions,
        kernel_args = kernel_args.join(", "),
        output = output,
    );

    let entry_point = "kernel.cu::run".to_string();
    // JSON: hand-built (small, fixed shape) so we don't pull a serde dep into the emitter.
    let escaped = json_escape(&kernel_cu);
    let arch_flag = knobs.chip.nvcc_arch_flag();
    let solution_json = format!(
        r#"{{
  "name": "logos_{op}_{def_slug}",
  "definition": "{definition}",
  "author": "logicaffeine",
  "description": "Compiler-generated (LOGOS -> codegen_cuda) fused-row kernel.",
  "spec": {{
    "languages": ["cuda_cpp"],
    "target_hardware": ["B200", "LOCAL"],
    "entry_point": "{entry_point}",
    "dependencies": ["torch"],
    "destination_passing_style": true,
    "compile_options": {{ "cuda_cflags": ["-O3", "--use_fast_math", "{arch_flag}"] }}
  }},
  "sources": [
    {{ "path": "kernel.cu", "content": "{escaped}" }}
  ]
}}
"#,
        op = spec.reduce.name(),
        def_slug = definition.replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
        definition = definition,
        entry_point = entry_point,
        arch_flag = arch_flag,
        escaped = escaped,
    );

    SolSolution {
        solution_json,
        kernel_cu,
        entry_point,
    }
}

/// Minimal JSON string escaping for embedding the CUDA source in `sources[].content`.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}
