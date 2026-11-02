//! Emit a representative FUSED elementwise chain to a `.cu` file and print the VM-expected output, so the
//! same emitted kernel can be built + run on any chip (e.g. the H100) and checked ≡ VM. This proves the
//! general elementwise-fusion pass works on real hardware beyond the local dev GPU.
//!
//!   cargo run -p logicaffeine-compile --example emit_fuse_h100 -- /tmp/fuse_h100.cu

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/fuse_h100.cu".to_string());
    // A 5-op linear chain over 1×8 Int tensors: G = ((((A+B)*C)+D)*A)+B — five elementwise ops that fuse
    // into ONE register-carried kernel (four intermediates never touch global memory).
    let src = "## Main\n\
        Let A be a 1 by 8 tensor of Int.\n\
        Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
        Set item 5 of A to 5.\nSet item 6 of A to 6.\nSet item 7 of A to 7.\nSet item 8 of A to 8.\n\
        Let B be a 1 by 8 tensor of Int.\n\
        Set item 1 of B to 2.\nSet item 2 of B to 2.\nSet item 3 of B to 2.\nSet item 4 of B to 2.\n\
        Set item 5 of B to 3.\nSet item 6 of B to 3.\nSet item 7 of B to 3.\nSet item 8 of B to 3.\n\
        Let C be a 1 by 8 tensor of Int.\n\
        Set item 1 of C to 1.\nSet item 2 of C to 2.\nSet item 3 of C to 1.\nSet item 4 of C to 2.\n\
        Set item 5 of C to 1.\nSet item 6 of C to 2.\nSet item 7 of C to 1.\nSet item 8 of C to 2.\n\
        Let D be a 1 by 8 tensor of Int.\n\
        Set item 1 of D to 10.\nSet item 2 of D to 20.\nSet item 3 of D to 30.\nSet item 4 of D to 40.\n\
        Set item 5 of D to 50.\nSet item 6 of D to 60.\nSet item 7 of D to 70.\nSet item 8 of D to 80.\n\
        Let E be A plus B.\n\
        Let F be E hadamard C.\n\
        Let G be F plus D.\n\
        Let H be G hadamard A.\n\
        Let I be H plus B.\n\
        Show item 1 of I.\nShow item 2 of I.\nShow item 3 of I.\nShow item 4 of I.\n\
        Show item 5 of I.\nShow item 6 of I.\nShow item 7 of I.\nShow item 8 of I.\n";
    let cu = compile_to_cuda_forward(src).expect("fused chain must lower to CUDA");
    std::fs::write(&out, &cu).expect("write cu");
    eprintln!(
        "wrote {out} — _fused: {}, _elem: {}",
        cu.matches("_fused").count(),
        cu.matches("_elem").count()
    );

    // A SwiGLU-shaped chain over 1×4 Real tensors: Z = silu(A+B) * C — the +/silu/* chain must fuse into ONE
    // kernel (silu applied in-register, no separate silu kernel). Emitted to a sibling file.
    let silu_src = "## Main\n\
        Let A be a 1 by 4 tensor of Real.\n\
        Set item 1 of A to -1.0.\nSet item 2 of A to 0.5.\nSet item 3 of A to 2.0.\nSet item 4 of A to -0.5.\n\
        Let B be a 1 by 4 tensor of Real.\n\
        Set item 1 of B to 0.5.\nSet item 2 of B to 0.5.\nSet item 3 of B to 0.5.\nSet item 4 of B to 0.5.\n\
        Let C be a 1 by 4 tensor of Real.\n\
        Set item 1 of C to 1.0.\nSet item 2 of C to 2.0.\nSet item 3 of C to 3.0.\nSet item 4 of C to 4.0.\n\
        Let X be A plus B.\n\
        Let Y be silu of X.\n\
        Let Z be Y hadamard C.\n\
        Show item 1 of Z.\nShow item 2 of Z.\nShow item 3 of Z.\nShow item 4 of Z.\n";
    let silu_out = format!("{out}.silu.cu");
    let silu_cu = compile_to_cuda_forward(silu_src).expect("swiglu chain must lower");
    std::fs::write(&silu_out, &silu_cu).expect("write silu cu");
    eprintln!(
        "wrote {silu_out} — _fused: {}, _silu: {}, _elem: {}",
        silu_cu.matches("_fused").count(),
        silu_cu.matches("_silu").count(),
        silu_cu.matches("_elem").count()
    );

    // stdout: INT-chain golden, then a blank line, then SwiGLU golden — the on-chip runs must match these.
    print!("{}", vm_outcome(src).output);
    println!();
    print!("{}", vm_outcome(silu_src).output);
}
