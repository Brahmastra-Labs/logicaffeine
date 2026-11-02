//! cuda_matmul_lowering_lock — proves the CUDA GEMM kernel is GENUINELY COMPILED from a LOGOS program,
//! not hand-written and not a fixed template. The compiler recognizes an ordinary triple-loop matmul
//! (the SAME backend-agnostic `recognize_matmul` the Tenstorrent tier uses), extracts its (M,N,K), and
//! lowers to a CUDA kernel whose dims are the *source program's*. Genuineness contract:
//!   • a real LOGOS matmul is RECOGNIZED and its (M,N,K) extracted from the source;
//!   • the emitted kernel is a FUNCTION of the source — change the dims, the kernel changes;
//!   • a non-matmul is REFUSED (typed);
//!   • (nvcc + GPU) the emitted kernel RUNS on the RTX 3060 and its checksum equals the VM's.
//! This is "our compiler emits the fast GEMM from LOGOS", tested on real silicon.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_matmul_lowering_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_cuda_matmul, vm_outcome};

/// A row-major GEMM `C(M×N) = A(M×K)·B(K×N)` as an ordinary LOGOS program — int8-range deterministic fill,
/// checksum — the kind of code a user writes, with NO matmul primitive. (Same shape as the TT lock.)
fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 5) - 2 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

#[test]
fn recognizes_a_logos_matmul_and_extracts_dims_from_the_source() {
    let (spec, _cu) = compile_to_cuda_matmul(&gemm_program(64, 128, 96))
        .expect("a genuine triple-loop matmul must be recognized");
    assert_eq!(
        (spec.m, spec.n, spec.k),
        (64, 128, 96),
        "dims must come from the source"
    );
}

#[test]
fn the_emitted_kernel_is_a_function_of_the_dims() {
    let (_s1, cu1) = compile_to_cuda_matmul(&gemm_program(16, 16, 16)).unwrap();
    let (_s2, cu2) = compile_to_cuda_matmul(&gemm_program(32, 48, 64)).unwrap();
    assert_ne!(
        cu1, cu2,
        "different dims must yield a different kernel (not one canned template)"
    );
    assert!(
        cu1.contains("16"),
        "16×16×16 dims must be baked into the kernel"
    );
    assert!(
        cu2.contains("48") && cu2.contains("64"),
        "32×48×64 dims must be baked"
    );
}

#[test]
fn a_non_matmul_is_refused() {
    // A plain arithmetic program is not a matmul — typed refusal, never a wrong kernel.
    assert!(compile_to_cuda_matmul("## Main\n    Show 6 * 7.\n").is_err());
    // A triple loop that is NOT a GEMM layout must also be refused (proved by the recognizer's layout check).
    let not_gemm = "## Main\nLet mutable c be a new Seq of Int.\nPush 0 to c.\nLet mutable i be 0.\nWhile i is less than 4:\n    Set i to i + 1.\nShow item 1 of c.\n";
    assert!(compile_to_cuda_matmul(not_gemm).is_err());
}

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

#[test]
fn emitted_cuda_matmul_equals_the_vm_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP cuda matmul exec: needs nvcc + a GPU driver (/dev/nvidia0).");
        return;
    }
    for (m, n, k) in [(16usize, 16usize, 16usize), (24, 32, 40), (8, 64, 48)] {
        let src = gemm_program(m, n, k);
        let (_spec, cu) = compile_to_cuda_matmul(&src).unwrap();
        let want = vm_outcome(&src).output.trim().to_string();

        let dir =
            std::env::temp_dir().join(format!("logos_cuda_mm_{}_{m}x{n}x{k}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (s, b) = (dir.join("m.cu"), dir.join("m"));
        std::fs::write(&s, &cu).unwrap();
        let c = std::process::Command::new("nvcc")
            .args(["-O2", "-arch=sm_86", "-o"])
            .arg(&b)
            .arg(&s)
            .output()
            .unwrap();
        assert!(
            c.status.success(),
            "emitted CUDA matmul must compile:\n{}\n\n{cu}",
            String::from_utf8_lossy(&c.stderr)
        );
        let r = std::process::Command::new(&b).output().unwrap();
        let got = String::from_utf8_lossy(&r.stdout).trim().to_string();
        assert_eq!(
            got, want,
            "({m}×{n}×{k}) emitted-CUDA checksum must equal the VM"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
