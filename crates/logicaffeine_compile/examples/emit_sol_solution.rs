//! Emit a compiler-generated SOL-ExecBench solution to disk — English LOGOS → `kernel.cu` + `solution.json`.
//!
//!   cargo run -p logicaffeine-compile --example emit_sol_solution -- <op> <out_dir>
//!
//! `<op>` ∈ { fused_add_rmsnorm, rmsnorm_weighted, layernorm }. The emitted `kernel.cu` (a runtime-shaped,
//! torch-binding CUDA kernel in the benchmark's DPS ABI) is what `scripts/accel/sol_local_run.py` builds + validates
//! against a PyTorch reference on the RTX 3060.

use logicaffeine_compile::compile::compile_to_sol_solution;

fn program(op: &str) -> (&'static str, &'static str, Vec<&'static str>) {
    // (logos_source, definition_name, input_names in Definition order)
    match op {
        "fused_add_rmsnorm" => (
            "## Main\nLet X be a 2 by 3 tensor of Real.\nSet item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\nSet item 4 of X to 4.0.\nSet item 5 of X to 5.0.\nSet item 6 of X to 6.0.\nLet R be a 2 by 3 tensor of Real.\nSet item 1 of R to 0.5.\nSet item 2 of R to 0.5.\nSet item 3 of R to 0.5.\nSet item 4 of R to 1.0.\nSet item 5 of R to 1.0.\nSet item 6 of R to 1.0.\nLet W be a 1 by 3 tensor of Real.\nSet item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 2.0.\nLet Y be rmsnorm of X plus R with weight W.\nShow round(item 1 of Y * 1000000).\n",
            "001_fused_add_rmsnorm_h2048",
            vec!["hidden_states", "residual", "weight"],
        ),
        "rmsnorm_weighted" => (
            "## Main\nLet X be a 2 by 3 tensor of Real.\nSet item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\nSet item 4 of X to 4.0.\nSet item 5 of X to 5.0.\nSet item 6 of X to 6.0.\nLet W be a 1 by 3 tensor of Real.\nSet item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 2.0.\nLet Y be rmsnorm of X with weight W.\nShow round(item 1 of Y * 1000000).\n",
            "021_rmsnorm_h2048",
            vec!["hidden_states", "weight"],
        ),
        "layernorm" => (
            "## Main\nLet X be a 2 by 3 tensor of Real.\nSet item 1 of X to 1.0.\nSet item 2 of X to 2.0.\nSet item 3 of X to 3.0.\nSet item 4 of X to 4.0.\nSet item 5 of X to 5.0.\nSet item 6 of X to 6.0.\nLet W be a 1 by 3 tensor of Real.\nSet item 1 of W to 0.5.\nSet item 2 of W to 1.0.\nSet item 3 of W to 2.0.\nLet B be a 1 by 3 tensor of Real.\nSet item 1 of B to 0.1.\nSet item 2 of B to 0.2.\nSet item 3 of B to 0.3.\nLet Y be layernorm of X with weight W and bias B.\nShow round(item 1 of Y * 1000000).\n",
            "layernorm_h2048",
            vec!["hidden_states", "weight", "bias"],
        ),
        other => panic!("unknown op `{other}` (expected fused_add_rmsnorm | rmsnorm_weighted | layernorm)"),
    }
}

fn main() {
    let op = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fused_add_rmsnorm".to_string());
    let out_dir = std::env::args()
        .nth(2)
        .unwrap_or_else(|| format!("target/sol-execbench/{op}"));
    let (src, definition, inputs) = program(&op);

    let sol = compile_to_sol_solution(src, definition, &inputs, "output")
        .unwrap_or_else(|e| panic!("LOGOS {op} must compile into a SOL-ExecBench solution: {e}"));

    std::fs::create_dir_all(&out_dir).expect("create out dir");
    std::fs::write(format!("{out_dir}/kernel.cu"), &sol.kernel_cu).expect("write kernel.cu");
    std::fs::write(format!("{out_dir}/solution.json"), &sol.solution_json)
        .expect("write solution.json");
    println!(
        "wrote {out_dir}/kernel.cu + solution.json for {op} (entry_point {})",
        sol.entry_point
    );
}
