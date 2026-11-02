//! emit_qor_harness — materialise one candidate of the comparative-synthesis bench into a run
//! directory: its RTL files, the `qorbist_<candidate>` harness, a testbench, and `candidate.json`
//! (the candidate, its file list, and the golden signature of its first N results).
//!
//!     cargo run -p logicaffeine-compile --example emit_qor_harness -- <out-dir> <kernel> <candidate> [shaped|edge] [ops] [--trace] [--as <module>]
//!
//!     cargo run -p logicaffeine-compile --example emit_qor_harness -- --list
//!
//! `--list` prints every LOGOS candidate as `<kernel> <candidate>`, one per line.
//! `--trace` also writes `tb_qor_trace.v`, which prints every folded result, and `golden.txt`, the golden
//! results in the same form, so the first operation a candidate gets wrong can be found. `--as <module>` names the
//! harness module (and its file) `<module>` instead of `qorbist_<candidate>` — the harness hardened as a block.
//!
//! `<candidate>` is `logos` for the kernel's default LOGOS candidate, the name of one of its LOGOS
//! candidates (`logos_candidates`), or the name of a third-party
//! candidate in `benchmarks/silicon/qor/candidates.json`, whose files must still hash to their
//! recorded SHA3-256.

use logicaffeine_compile::codegen_sva::qor_harness::{
    check_ports, emit_harness_as, emit_testbench_as, emit_trace_testbench_as, golden, harness_module, kernel,
    logos_candidate, logos_candidates, manifest_candidate, signature, stimulus, trace_lines, Mode,
};
use std::path::{Path, PathBuf};

/// The run's whole budget per result, and the cycles a candidate may go without folding a result:
/// both far above the slowest candidate of the bench (a few thousand cycles per result).
const MAX_CYCLES_PER_OP: u64 = 20_000;
const MAX_IDLE: u64 = 100_000;

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.first().map(String::as_str) == Some("--list") {
        for k in logicaffeine_compile::codegen_sva::qor_harness::kernels() {
            for (c, _) in logos_candidates(k.id).unwrap_or_default() {
                println!("{} {}", k.id, c.name);
            }
        }
        return;
    }
    let trace = all.iter().any(|a| a == "--trace");
    let named = all.iter().position(|a| a == "--as").map(|i| {
        all.get(i + 1).cloned().unwrap_or_else(|| fail("--as names the harness module"))
    });
    let mut args: Vec<String> = Vec::new();
    let mut skip = false;
    for a in all {
        if skip {
            skip = false;
        } else if a == "--as" {
            skip = true;
        } else if a != "--trace" {
            args.push(a);
        }
    }
    if args.len() < 3 {
        eprintln!("usage: emit_qor_harness <out-dir> <kernel> <candidate|logos> [shaped|edge] [ops] [--trace] [--as <module>]");
        std::process::exit(2);
    }
    let out = PathBuf::from(&args[0]);
    let k = kernel(&args[1]).unwrap_or_else(|| fail(&format!("{}: no such kernel", args[1])));
    let mode = match args.get(3).map(String::as_str).unwrap_or("shaped") {
        "shaped" => Mode::Shaped,
        "edge" => Mode::Edge,
        other => fail(&format!("{other}: the mode is shaped or edge")),
    };
    let ops: usize = args.get(4).map(|s| s.parse().unwrap_or_else(|_| fail("ops is a count"))).unwrap_or(1000);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let own = logos_candidates(k.id).unwrap_or_default();
    let is_logos = args[2] == "logos" || own.iter().any(|(c, _)| c.name == args[2]);
    let (cand, files) = if args[2] == "logos" {
        logos_candidate(k.id)
    } else if let Some(found) = own.into_iter().find(|(c, _)| c.name == args[2]) {
        Ok(found)
    } else {
        manifest_candidate(&root, &args[2])
    }
    .unwrap_or_else(|e| fail(&e.to_string()));
    if cand.kernel != k.id {
        fail(&format!("{} is a candidate of {}, not {}", cand.name, cand.kernel, k.id));
    }
    check_ports(&k, &cand).unwrap_or_else(|e| fail(&e.to_string()));

    std::fs::create_dir_all(&out).unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    let mut written = Vec::new();
    for (name, text) in &files {
        std::fs::write(out.join(name), text).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
        written.push(name.clone());
    }
    let module = named.unwrap_or_else(|| harness_module(&cand));
    let harness = format!("{module}.v");
    std::fs::write(out.join(&harness), emit_harness_as(&k, &cand, mode, &module)).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::write(out.join("tb_qor.v"), emit_testbench_as(&module, ops, MAX_CYCLES_PER_OP * ops as u64 + 100_000, MAX_IDLE))
        .unwrap_or_else(|e| fail(&e.to_string()));

    let stim = stimulus(&k, mode, ops);
    let results = golden(&k, &stim).unwrap_or_else(|e| fail(&e.to_string()));
    if trace {
        std::fs::write(out.join("tb_qor_trace.v"), emit_trace_testbench_as(&k, &module, ops, MAX_CYCLES_PER_OP * ops as u64 + 100_000, MAX_IDLE))
            .unwrap_or_else(|e| fail(&e.to_string()));
        std::fs::write(out.join("golden.txt"), trace_lines(&k, &results)).unwrap_or_else(|e| fail(&e.to_string()));
        let operands: String = stim
            .iter()
            .enumerate()
            .map(|(op, w)| {
                let ports: Vec<String> = w.iter().map(|p| p.iter().rev().map(|c| format!("{c:016x}")).collect::<String>()).collect();
                format!("{op} {}\n", ports.join(" "))
            })
            .collect();
        std::fs::write(out.join("operands.txt"), operands).unwrap_or_else(|e| fail(&e.to_string()));
    }
    let record = serde_json::json!({
        "kernel": k.id,
        "kernel_name": k.name,
        "source_kind": if is_logos { k.source_kind.label() } else { "third-party" },
        "candidate": cand,
        "protocol": cand.protocol.label(),
        "files": written,
        "harness": harness,
        "harness_top": module,
        "testbench": "tb_qor.v",
        "mode": format!("{mode:?}").to_lowercase(),
        "ops": ops,
        "golden_signature": format!("{:016x}", signature(&k, &results)),
    });
    std::fs::write(out.join("candidate.json"), serde_json::to_string_pretty(&record).unwrap() + "\n")
        .unwrap_or_else(|e| fail(&e.to_string()));
    println!("{}", out.join("candidate.json").display());
}

fn fail(msg: &str) -> ! {
    eprintln!("emit_qor_harness: {msg}");
    std::process::exit(1);
}
