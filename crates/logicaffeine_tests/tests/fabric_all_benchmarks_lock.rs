//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ALL-BENCHMARKS-ON-THE-FABRIC RATCHET — the pit-of-success lock that forbids the whole-program fabric
//! from ever silently dropping a benchmark.
//!
//! Every chip-sized benchmark in `benchmarks/silicon/fpga/bench/*.lg` must lower to the whole-program fabric
//! (`program_calls_fabric` — the call-stack / block-walk engine with the BRAM state plane) and, run on the
//! authoritative evaluator, produce a result byte-identical to the VM. Today ALL 31 do. This lock pins that:
//!
//!   • SET-EQUALITY: the corpus IS the directory — every `.lg` present must be accounted for. A NEW benchmark
//!     that doesn't lower fails this lock until it is made to lower (grow the fabric) or added to `FRONTIER`
//!     with an honest reason. `FRONTIER` is a RATCHET — it may only shrink.
//!   • FLOOR: at least `BENCH_FLOOR` benchmarks run ≡ VM. Raise the floor when the number grows; never lower
//!     it to make a regression pass.
//!   • ≡ VM: each lowered benchmark's fabric result equals `vm_outcome` (int / float bit-pattern / rendered
//!     Text char-list — every value type the state plane carries).
//!
//! A regression that breaks a benchmark's lowering, or a change that drops one below the floor, goes RED.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted_heap, program_calls_fabric,
};
use logicaffeine_compile::compile::vm_outcome;

/// Benchmarks that legitimately do NOT yet lower to the whole-program fabric (with an honest reason). RATCHET:
/// this may only SHRINK — a benchmark leaves the frontier when the fabric grows to run it, never joins it to
/// dodge a break. EMPTY today: all 31 lower ≡ VM.
const FRONTIER: &[&str] = &[];

/// The floor — at least this many benchmarks must lower ≡ VM. RAISE when it grows; never lower.
const BENCH_FLOOR: usize = 31;

/// Wrap a benchmark's whole `Main` body into a zero-arg `__bench` so the whole-program engine runs the FULL
/// program (setup + loops + recursion + calls). The LAST `Show <e>` becomes the returned value (its compute
/// chain is never dead-code-eliminated); earlier Shows are pure display and dropped.
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

/// Does the fabric's returned bit-pattern render to the VM's output string? INT (decimal), FLOAT (f64 from
/// the bit pattern), or TEXT (a char-list `[len, byte0..]` at the returned heap pointer).
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
    let p = result_bits.max(0) as usize;
    if let Some(&len) = heap.get(p) {
        if len >= 0 && (p + 1 + len as usize) <= heap.len() {
            let bytes: Vec<u8> = (0..len as usize).map(|j| heap[p + 1 + j] as u8).collect();
            if let Ok(s) = String::from_utf8(bytes) {
                return s == vm;
            }
        }
    }
    false
}

fn bench_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("benchmarks/silicon/fpga/bench")
}

/// Lower one benchmark to the whole-program fabric and check it ≡ VM. Returns Ok(()) on match, Err(reason).
fn run_on_fabric(name: &str) -> Result<(), String> {
    let raw = std::fs::read_to_string(bench_dir().join(format!("{name}.lg")))
        .map_err(|e| format!("read: {e}"))?;
    let src = wrap_main(&raw).ok_or("no ## Main")?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cfg = program_calls_fabric(&src, "__bench");
        let (r, _cyc, heap) = eval_wprog_config_counted_heap(&cfg, &[]);
        (r, heap)
    }));
    match outcome {
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| e.downcast_ref::<&str>().copied())
                .unwrap_or("panic");
            Err(format!("refused: {}", &msg[..msg.len().min(80)]))
        }
        Ok((r, heap)) => {
            let vm = vm_outcome(&src).output;
            if matches_vm(r, &vm, &heap) {
                Ok(())
            } else {
                Err(format!("fabric result {r} != VM [{}]", vm.trim()))
            }
        }
    }
}

#[test]
fn every_benchmark_runs_on_the_whole_program_fabric_matching_the_vm() {
    std::panic::set_hook(Box::new(|_| {})); // silence the loud refuse panics; we classify them

    let mut names: Vec<String> = std::fs::read_dir(bench_dir())
        .expect("read bench dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".lg"))
        .map(|n| n.trim_end_matches(".lg").to_string())
        .collect();
    names.sort();

    let all: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let frontier: BTreeSet<&str> = FRONTIER.iter().copied().collect();

    // Set-equality guard: no stale FRONTIER entry (every excluded name must actually be a benchmark).
    for f in &frontier {
        assert!(
            all.contains(f),
            "FRONTIER lists `{f}` which is not a benchmark in {}",
            bench_dir().display()
        );
    }

    let mut lowered = 0usize;
    let mut broken: Vec<String> = Vec::new();
    for name in &names {
        if frontier.contains(name.as_str()) {
            continue; // an honestly-declared frontier benchmark — not required to lower yet
        }
        match run_on_fabric(name) {
            Ok(()) => lowered += 1,
            Err(reason) => broken.push(format!("{name}: {reason}")),
        }
    }

    assert!(
        broken.is_empty(),
        "these benchmarks must lower to the whole-program fabric ≡ VM (fix the fabric, or add to FRONTIER \
         with a reason — it may only shrink):\n  {}",
        broken.join("\n  ")
    );
    assert!(
        lowered >= BENCH_FLOOR,
        "only {lowered} benchmarks lowered ≡ VM — below the floor {BENCH_FLOOR}. Raise the fabric, don't \
         lower the floor."
    );

    println!("all-benchmarks-on-the-fabric: {lowered}/{} run on the whole-program fabric ≡ VM (frontier: {})", names.len(), FRONTIER.len());
}
