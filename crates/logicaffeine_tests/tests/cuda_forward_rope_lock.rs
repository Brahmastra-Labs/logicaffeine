//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RoPE — the rotary position embedding (`rope of Q with cos RC and sin RS`) authored in LOGOS lowers to a
//! parallel CUDA kernel and matches the VM. The last op the exact Qwen3 attention needs (`Q·Kᵀ` after RoPE).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// RoPE-NEOX over `H=2` heads of `hd=4` (`half=2`) rotates each head's `(i, i+2)` pairs by `(cos, sin)`;
/// authored in LOGOS, it composes to a parallel `k_rope` kernel and matches the VM on the GPU.
#[test]
fn rope_lowers_to_a_parallel_kernel_and_matches_the_vm() {
    let src = "## Main\n\
        Let Q be a 1 by 8 tensor of Real.\n\
        Set item 1 of Q to 0.1.\nSet item 2 of Q to 0.2.\nSet item 3 of Q to 0.3.\nSet item 4 of Q to 0.4.\n\
        Set item 5 of Q to 0.5.\nSet item 6 of Q to 0.6.\nSet item 7 of Q to 0.7.\nSet item 8 of Q to 0.8.\n\
        Let RC be a 1 by 2 tensor of Real.\n\
        Set item 1 of RC to 0.6.\nSet item 2 of RC to 0.8.\n\
        Let RS be a 1 by 2 tensor of Real.\n\
        Set item 1 of RS to 0.8.\nSet item 2 of RS to 0.6.\n\
        Let Qr be rope of Q with cos RC and sin RS.\n\
        Show item 1 of Qr.\nShow item 2 of Qr.\nShow item 3 of Qr.\nShow item 4 of Qr.\n\
        Show item 5 of Qr.\nShow item 6 of Qr.\nShow item 7 of Qr.\nShow item 8 of Qr.\n";
    let cu = compile_to_cuda_forward(src).expect("rope must lower to composed CUDA");
    assert!(
        cu.contains("k0_rope"),
        "rope must lower to a parallel kernel:\n{cu}"
    );

    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP rope GPU run: needs nvcc + a live GPU. (Emission asserted above.)");
        return;
    }
    let vm: Vec<f64> = {
        let o = vm_outcome(src);
        assert!(o.error.is_none(), "VM error: {:?}", o.error);
        o.output
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect()
    };
    assert_eq!(vm.len(), 8, "VM must produce 8 rotated entries, got {vm:?}");

    let dir = std::env::temp_dir().join(format!("logos_rope_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (cu_path, bin) = (dir.join("r.cu"), dir.join("r"));
    std::fs::write(&cu_path, &cu).unwrap();
    let comp = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&cu_path)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        comp.status.success(),
        "emitted rope must compile:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin).output().expect("run");
    assert!(
        run.status.success(),
        "run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let got: Vec<f32> = String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();

    assert_eq!(got.len(), 8);
    for (o, (g, r)) in got.iter().zip(&vm).enumerate() {
        assert!(
            (*g as f64 - r).abs() < 1e-5,
            "rope out {o}: GPU {g} vs VM {r}"
        );
    }
    println!("RoPE-NEOX authored in LOGOS → parallel CUDA kernel ≡ VM {vm:?}.");
}
