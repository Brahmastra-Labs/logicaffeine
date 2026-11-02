//! fabric_cycles — the WHOLE-PROGRAM fabric cycle count for each benchmark, verified against the VM.
//!
//! Wraps each benchmark's whole `Main` into a `__bench` function so the call-stack / block-walk fabric engine
//! (`program_calls_fabric`) runs the FULL benchmark — array setup, loops, recursion, calls with real args.
//! `eval_wprog_config_counted` (the ONE authoritative evaluator, shared with the result path — no drifting
//! copy) returns the result AND the block-step count = fabric cycles. Every result is checked against the VM
//! (`vm_outcome` on the same source), so a real fabric-vs-VM divergence surfaces as MISMATCH — not hidden.
//!
//!   cargo run -p logicaffeine-compile --example fabric_cycles

use std::panic;

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted_heap, program_calls_fabric,
};
use logicaffeine_compile::compile::vm_outcome;

/// Wrap the benchmark's whole `Main` body into a zero-arg `__bench` so the whole-program engine runs the
/// FULL benchmark. The LAST `Show <e>.` becomes `Return <e>` (its compute chain is the verified value);
/// earlier Shows are pure display and dropped (their Let/loop compute stays).
fn wrap_main(src: &str) -> Option<String> {
    let idx = src.find("## Main")?;
    let (funcs, main) = src.split_at(idx);
    let body = main
        .trim_start_matches("## Main")
        .trim_start_matches(['\n', '\r']);
    let lines: Vec<&str> = body.lines().collect();
    let last_show = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("Show "));
    let mut out = String::from(funcs);
    out.push_str("## To __bench () -> Int:\n");
    let mut returned = false;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if t.is_empty() {
            continue;
        }
        if let Some(e) = t.strip_prefix("Show ") {
            if Some(i) == last_show {
                out.push_str(&format!("    Return {}\n", e.trim()));
                returned = true;
            }
        } else {
            out.push_str("    ");
            out.push_str(l.trim_end());
            out.push('\n');
        }
    }
    if !returned {
        out.push_str("    Return 0.\n");
    }
    out.push_str("## Main\nShow __bench().\n");
    Some(out)
}

/// Does the fabric's returned bit-pattern render to the VM's output string? Tries INT (decimal), FLOAT (f64
/// from the bit pattern), then TEXT (a char-list `[len, byte0..]` at the returned heap pointer — how the engine
/// returns a Text result).
fn matches_vm(result_bits: i64, vm_out: &str, heap: &[i64]) -> bool {
    let vm = vm_out.trim();
    if vm == result_bits.to_string() {
        return true;
    }
    if let Ok(vf) = vm.parse::<f64>() {
        let ff = f64::from_bits(result_bits as u64);
        if (ff - vf).abs() <= 1e-9 * vf.abs().max(1.0) || (ff.is_nan() && vf.is_nan()) {
            return true;
        }
    }
    // TEXT: render the char-list at `result_bits` and compare to the VM's string output.
    let p = result_bits.max(0) as usize;
    if let Some(&len) = heap.get(p) {
        if (0..=len.max(0)).all(|_| true) && len >= 0 && (p + 1 + len as usize) <= heap.len() {
            let bytes: Vec<u8> = (0..len as usize).map(|j| heap[p + 1 + j] as u8).collect();
            if let Ok(s) = String::from_utf8(bytes) {
                return s == vm;
            }
        }
    }
    false
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    let root = env!("CARGO_MANIFEST_DIR");
    let bench = format!("{root}/../../benchmarks/silicon/fpga/bench");
    let mut names: Vec<String> = std::fs::read_dir(&bench)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".lg"))
        .map(|n| n.trim_end_matches(".lg").to_string())
        .collect();
    names.sort();
    println!(
        "{:<15} {:>12} {:>16}  {}",
        "benchmark", "fab_cycles", "result", "vs VM"
    );
    let (mut ok, mut diverge, mut refused) = (0u32, 0u32, 0u32);
    for n in &names {
        let raw = std::fs::read_to_string(format!("{bench}/{n}.lg")).unwrap();
        let Some(src) = wrap_main(&raw) else {
            println!("{n:<15}  no-Main");
            continue;
        };
        let res = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let cfg = program_calls_fabric(&src, "__bench");
            let (r, cyc, heap) = eval_wprog_config_counted_heap(&cfg, &[]);
            let vm = vm_outcome(&src).output;
            (r, cyc, vm, heap)
        }));
        match res {
            Ok((r, cyc, vm, heap)) => {
                if matches_vm(r, &vm, &heap) {
                    println!("{n:<15} {cyc:>12} {r:>16}  MATCH");
                    ok += 1;
                } else {
                    println!("{n:<15} {cyc:>12} {r:>16}  DIVERGE (vm={})", vm.trim());
                    diverge += 1;
                }
            }
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("?");
                println!("{n:<15}  REFUSED: {}", &msg[..msg.len().min(66)]);
                refused += 1;
            }
        }
    }
    println!(
        "\n{ok} run ≡ VM, {diverge} DIVERGE (real fabric bug), {refused} refused (lane not wired)."
    );
}
