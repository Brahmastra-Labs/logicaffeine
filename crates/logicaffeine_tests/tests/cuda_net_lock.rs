//! ★ THE C / CUDA OFFLINE NETWORKING (loopback transport) — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! The networking ops run the deterministic OFFLINE model on a single device thread — exactly as the
//! RISC-V AOT tier does under qemu: `Connect`/`Listen`/`Sync` are no-ops, a peer handle IS its address
//! string (`MakePeer` = identity), `Send`/`Stream` enqueue a 64-bit message (a scalar, or a `Seq` pointer
//! for a stream — the same object round-trips on one node) into a per-address FIFO, and `Await` dequeues it.
//! So `Send X to peer; Await from peer into y` round-trips deterministically. This lock proves the AOT-C
//! backend lowers all seven Net ops FAITHFULLY (codegen_c had ZERO Net support — all seven were
//! `/* unsupported stmt */` no-ops) AND that the device-lifted `C_NET_RUNTIME` (strcmp/strdup shimmed) runs
//! ≡VM on the real GPU.
//!
//! Discipline (mirrors `c_temporal_lock`):
//! - The oracle pair (`tw_outcome` + `vm_outcome_net`) AGREES on every corpus program.
//! - Each program compiles to C and runs byte-for-byte ≡ that oracle under gcc; and (gated) its `.cu`
//!   compiles under nvcc and runs ≡VM on `/dev/nvidia0`.
//! - The emitted C literally calls `logos_net_send`/`logos_net_await` — anti-floor-gaming.
//!
//! ⚠️ RED tests are the spec: if a program fails ≡VM, fix `codegen_c`, NEVER weaken this file. The C
//! execution half is gated on `gcc`; the GPU half on `nvcc`+`/dev/nvidia0`. Both SKIP when absent.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda, tw_outcome, vm_outcome_net};

/// Scalar loopback: send an Int to a self-peer, await it back — round-trips to `7`. Exercises
/// Connect/Listen/MakePeer/Send/Await.
const SCALAR: &str = "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    \
Let peer be a PeerAgent at \"me\".\n    Send 7 to peer.\n    Await response from peer into got.\n    Show got.\n";

/// Stream loopback: stream a Seq to a self-peer, await the batch back, read `item 2` — round-trips to `2`.
/// Exercises Stream + stream-Await (the Seq pointer round-trips on one node).
const STREAM: &str = "## Main\n    Connect to \"ws://127.0.0.1:9\".\n    Listen on \"me\".\n    \
Let peer be a PeerAgent at \"me\".\n    Let vals be [1, 2, 3].\n    Stream vals to peer.\n    \
Await stream from peer into b.\n    Show item 2 of b.\n";

const CORPUS: &[(&str, &str)] = &[("scalar", SCALAR), ("stream", STREAM)];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir =
        std::env::temp_dir().join(format!("logos_cnet_{}_{}_{}", std::process::id(), label, n));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "gcc failed for {label}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

fn cu_output(cu: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_cudanet_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-fmad=false", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "nvcc failed for {label}:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !r.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&r.stdout).trim().to_string())
}

#[test]
fn the_net_oracles_agree() {
    for (label, src) in CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome_net(src);
        assert_eq!(
            (tw.output.trim(), tw.error.is_some()),
            (vm.output.trim(), vm.error.is_some()),
            "`{label}`: the tree-walker and offline-VM oracles must agree",
        );
    }
    assert_eq!(vm_outcome_net(SCALAR).output.trim(), "7");
    assert_eq!(vm_outcome_net(STREAM).output.trim(), "2");
}

#[test]
fn emitted_c_lowers_the_transport() {
    // Anti-floor-gaming: the Net statements really lower to the transport, not a silent no-op.
    for (label, src) in CORPUS {
        let c = compile_to_c(src).expect("compile");
        assert!(
            c.contains("logos_net_send("),
            "`{label}`: Send/Stream must lower to logos_net_send"
        );
        assert!(
            c.contains("logos_net_await("),
            "`{label}`: Await must lower to logos_net_await"
        );
        assert!(
            !c.contains("/* unsupported stmt */"),
            "`{label}`: no statement should be unsupported"
        );
    }
}

#[test]
fn c_backend_matches_the_oracle() {
    if !have_gcc() {
        eprintln!("skip: gcc absent");
        return;
    }
    for (label, src) in CORPUS {
        let want = vm_outcome_net(src).output.trim().to_string();
        let got = c_output(src, label).unwrap_or_else(|| panic!("`{label}`: C must compile + run"));
        assert_eq!(
            got, want,
            "`{label}`: the C backend must match the offline-VM oracle byte-for-byte"
        );
    }
}

#[test]
fn net_runs_on_the_gpu_equiv_vm() {
    // The CUDA codegen must SUCCEED regardless of a GPU being present.
    let arts: Vec<_> = CORPUS
        .iter()
        .map(|(label, src)| {
            (
                *label,
                *src,
                compile_to_cuda(src)
                    .unwrap_or_else(|e| panic!("`{label}` must lower to CUDA: {e:?}")),
            )
        })
        .collect();

    if !have_nvcc_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0 — cannot run the net kernels on the GPU");
        return;
    }
    for (label, src, art) in &arts {
        let want = vm_outcome_net(src).output.trim().to_string();
        let got = cu_output(&art.kernel_cu, label)
            .unwrap_or_else(|| panic!("`{label}`: .cu must nvcc-compile + run"));
        assert_eq!(
            &got, &want,
            "`{label}`: the CUDA kernel must run ≡VM on the GPU"
        );
    }
}
