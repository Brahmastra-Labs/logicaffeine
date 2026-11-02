//! Emit a Tensix bundle from the spatial IR — the compiler's own choice or one named calibration candidate.
//!
//!   cargo run -p logicaffeine-compile --features codegen --example emit_spatial_tensix -- \
//!       <dir> M N K [wormhole|blackhole] [--fidelity LoFi|HiFi2|HiFi3|HiFi4] [--grid XxY] [--l1acc 0|1] [--packing loads|split] [--policy exact|relaxed] [--candidate best|streamed|fused|unfused]
//!
//! Writes `host.cpp`, `kernels/*.cpp`, `CMakeLists.txt`, `logos_oracle.c` and `spatial.json` (the IR's plan
//! summary + census) into `<dir>`, and prints the `LOGOS_MM_CFG` line the host will print.

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::tensix::{bundle_dir_files, TensixBackend};
use logicaffeine_compile::spatial::lower::Census;
use logicaffeine_compile::spatial::pipeline::{calibration_candidates_matching, CandidateKind};
use logicaffeine_compile::spatial::target::SpatialTarget;

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
    Push (p % 11) - 5 to b.
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

/// The P4 workloads, at a size worth measuring. The fills are explicit element sets — the same shape
/// the front-end locks use — so the recognizer sees a literal tensor and the emitted host golden is
/// computed from exactly those values rather than from a pattern the two sides could read differently.
fn rmsnorm_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += &format!("Let W be a 1 by {cols} tensor of Real.\n");
    for i in 1..=cols {
        s += &format!("Set item {i} of W to {:.1}.\n", 0.5 + (i % 3) as f64 * 0.5);
    }
    s += "Let Y be rmsnorm of X with weight W.\nShow round(item 1 of Y * 1000000).\n";
    s
}

/// LayerNorm — the same per-row shape as RMSNorm plus a mean subtraction and a bias shift, which is what
/// distinguishes the two in `match_layernorm_push`. Weight and bias are per-COLUMN, so both are `1 by cols`.
fn layernorm_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += &format!("Let W be a 1 by {cols} tensor of Real.\n");
    for i in 1..=cols {
        s += &format!("Set item {i} of W to {:.1}.\n", 0.5 + (i % 3) as f64 * 0.5);
    }
    s += &format!("Let B be a 1 by {cols} tensor of Real.\n");
    // ★ TWO DECIMALS, NOT ONE. The bias steps by 0.25, and `{:.1}` writes 0.25 as "0.2" — so at one
    // decimal this program does not contain the values this line appears to say, and the device and
    // the host golden agree with each other on the quantized ones while any reader (or independent
    // model) computes the unquantized ones. That cost a real investigation: the p4 reference model
    // made the checksum 26 and the device made it 24, and neither was wrong about its own inputs.
    for i in 1..=cols {
        s += &format!("Set item {i} of B to {:.2}.\n", (i % 5) as f64 * 0.25 - 0.5);
    }
    s += "Let Y be layernorm of X with weight W and bias B.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn swiglu_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet G be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of G to {:.1}.\n", (i % 5) as f64 - 2.0);
    }
    s += &format!("Let U be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of U to {:.1}.\n", (i % 3) as f64 + 1.0);
    }
    s += "Let Y be swiglu of G and U.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn softmax_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += "Let Y be softmax of X.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn elementwise_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet A be a {rows} by {cols} tensor of Int.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of A to {}.\n", (i % 7) as i64 - 3);
    }
    s += &format!("Let B be a {rows} by {cols} tensor of Int.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of B to {}.\n", (i % 11) as i64 - 5);
    }
    s += "Let C be A plus B.\nShow C.\n";
    s
}

fn usage() -> ! {
    eprintln!("usage: emit_spatial_tensix <dir> M N K [wormhole|blackhole] [--fidelity LoFi|HiFi2|HiFi3|HiFi4] [--grid XxY] [--l1acc 0|1] [--policy exact|relaxed] [--candidate best|streamed|fused|unfused]");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        usage();
    }
    let dir = std::path::PathBuf::from(&args[0]);
    let dims: Vec<usize> = args[1..4]
        .iter()
        .map(|a| a.parse().unwrap_or_else(|_| usage()))
        .collect();
    let (m, n, k) = (dims[0], dims[1], dims[2]);
    let mut chip = "wormhole".to_string();
    let mut grid: Option<(usize, usize)> = None;
    let mut l1acc: Option<bool> = None;
    let mut depth: Option<usize> = None;
    let mut packing: Option<logicaffeine_compile::spatial::ir::EnginePacking> = None;
    let mut per_site: Option<(usize, usize)> = None;
    let mut blk: Option<logicaffeine_compile::spatial::ir::BlockShape> = None;
    let mut policy = NumericsPolicy::Exact;
    // The fidelity to DECLARE the matrix engine at. A Tensix runs bf16 matmul at any of these; the descriptor
    // pinned HiFi4, which meant we could not emit at ttnn's own bf16 default (HiFi2) and so had nothing of
    // ours in the one knob group where a like-for-like head-to-head matters.
    let mut fidelity: Option<logicaffeine_compile::spatial::target::Fidelity> = None;
    let mut candidate = CandidateKind::Best;
    let mut workload = "gemm".to_string();
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "wormhole" | "blackhole" => chip = args[i].clone(),
            "--fidelity" => {
                // One parser for this identifier, wherever it arrives from. It is the same token the
                // harness sets as `LOGOS_TT_FIDELITY`, the same one the emitted program prints on its
                // `LOGOS_MM_PASS` line, and the same one the ledger keys its rows on — three places that
                // must never disagree about what "HiFi3" means, so they read it through one function.
                use logicaffeine_compile::spatial::target::Fidelity;
                i += 1;
                fidelity = Some(
                    match args.get(i).map(|s| s.as_str()).and_then(Fidelity::parse) {
                        Some(Fidelity::Exact) | None => usage(),
                        Some(f) => f,
                    },
                );
            }
            "--per-core" => {
                i += 1;
                let Some(spec) = args.get(i) else { usage() };
                let g: Vec<&str> = spec.split('x').collect();
                if g.len() != 2 {
                    usage();
                }
                per_site = Some((
                    g[0].parse().unwrap_or_else(|_| usage()),
                    g[1].parse().unwrap_or_else(|_| usage()),
                ));
            }
            "--block" => {
                i += 1;
                let Some(spec) = args.get(i) else { usage() };
                let g: Vec<&str> = spec.split('x').collect();
                if g.len() != 3 {
                    usage();
                }
                blk = Some(logicaffeine_compile::spatial::ir::BlockShape {
                    osh: g[0].parse().unwrap_or_else(|_| usage()),
                    osw: g[1].parse().unwrap_or_else(|_| usage()),
                    ibw: g[2].parse().unwrap_or_else(|_| usage()),
                });
            }
            "--packing" => {
                i += 1;
                packing = match args.get(i).map(String::as_str) {
                    Some("loads") => {
                        Some(logicaffeine_compile::spatial::ir::EnginePacking::LoadsTogether)
                    }
                    Some("split") => {
                        Some(logicaffeine_compile::spatial::ir::EnginePacking::OperandWithStore)
                    }
                    _ => usage(),
                };
            }
            "--l1acc" => {
                i += 1;
                l1acc = match args.get(i).map(String::as_str) {
                    Some("0") => Some(false),
                    Some("1") => Some(true),
                    _ => usage(),
                };
            }
            "--depth" => {
                i += 1;
                depth = Some(
                    args.get(i)
                        .and_then(|d| d.parse().ok())
                        .unwrap_or_else(|| usage()),
                );
            }
            "--grid" => {
                i += 1;
                let g = args
                    .get(i)
                    .and_then(|g| g.split_once('x'))
                    .unwrap_or_else(|| usage());
                grid = Some((
                    g.0.parse().unwrap_or_else(|_| usage()),
                    g.1.parse().unwrap_or_else(|_| usage()),
                ));
            }
            "--policy" => {
                i += 1;
                policy = match args.get(i).map(String::as_str) {
                    Some("exact") => NumericsPolicy::Exact,
                    Some("relaxed") => NumericsPolicy::AllowSpillRounding,
                    _ => usage(),
                };
            }
            "--workload" => {
                i += 1;
                workload = match args.get(i).map(String::as_str) {
                    Some(
                        w @ ("gemm" | "rmsnorm" | "layernorm" | "swiglu" | "softmax"
                        | "elementwise"),
                    ) => w.to_string(),
                    _ => usage(),
                };
            }
            "--candidate" => {
                i += 1;
                candidate = match args.get(i).map(String::as_str) {
                    Some("best") => CandidateKind::Best,
                    Some("streamed") => CandidateKind::Streamed,
                    Some("fused") => CandidateKind::Fused,
                    Some("unfused") => CandidateKind::Unfused,
                    _ => usage(),
                };
            }
            _ => usage(),
        }
        i += 1;
    }
    let target = match chip.as_str() {
        "wormhole" => SpatialTarget::wormhole(),
        "blackhole" => SpatialTarget::blackhole(),
        _ => usage(),
    };
    let target = match fidelity {
        Some(f) => target.at_fidelity(f),
        None => target,
    };
    let target = match grid {
        Some((x, y)) => target.with_sites(x, y),
        None => target,
    };
    if workload != "gemm" {
        // ★ NOT A CALIBRATION SWEEP. `calibration_candidates` exists to compare GEMM TILINGS; a row op
        // or an elementwise map has one lowering, so asking for `streamed`/`fused`/`unfused` here would
        // be asking which of one thing is fastest. It takes the pipeline's own verdict instead.
        let src = match workload.as_str() {
            "rmsnorm" => rmsnorm_program(m, n),
            "layernorm" => layernorm_program(m, n),
            "swiglu" => swiglu_program(m, n),
            "softmax" => softmax_program(m, n),
            _ => elementwise_program(m, n),
        };
        let program = logicaffeine_compile::spatial::pipeline::compile_to_spatial(&src, &target)
            .unwrap_or_else(|e| {
                eprintln!("the spatial pipeline refused {workload} {m}x{n} on {chip}: {e:?}");
                std::process::exit(1)
            });
        let art = TensixBackend::new(target.clone())
            .lower_tensix(&program)
            .unwrap_or_else(|e| {
                eprintln!("the Tensix lowering refused {workload} {m}x{n}: {e:?}");
                std::process::exit(1)
            });
        std::fs::create_dir_all(dir.join("kernels")).expect("bundle dir");
        if let Ok(rd) = std::fs::read_dir(dir.join("kernels")) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        for (rel, body) in bundle_dir_files(&art) {
            std::fs::write(dir.join(&rel), body).unwrap_or_else(|e| panic!("write {rel}: {e}"));
        }
        let summary = program.summary();
        let meta = serde_json::json!({
            "workload": workload,
            "shape": [m, n],
            "chip": chip,
            "mesh": [target.sites.x, target.sites.y],
            "summary": summary,
            "census": Census::of(&program),
            "cfg_line": art.cfg_line,
            "kernels": art.kernels.iter().map(|k| k.name.clone()).collect::<Vec<_>>(),
        });
        std::fs::write(
            dir.join("spatial.json"),
            serde_json::to_string_pretty(&meta).unwrap(),
        )
        .expect("spatial.json");
        println!("{}", art.cfg_line);
        println!(
            "wrote {} kernels + host to {} ({workload} {m}x{n})",
            art.kernels.len(),
            dir.display()
        );
        return;
    }
    let src = gemm_program(m, n, k);
    let want = logicaffeine_compile::spatial::pipeline::PlanFilter {
        pack_accumulate: l1acc,
        engines: packing,
        per_site,
        block: blk,
    };
    let candidates =
        calibration_candidates_matching(&src, &target, policy, want).unwrap_or_else(|e| {
            eprintln!("spatial pipeline refused {m}x{n}x{k} on {chip}: {e:?}");
            std::process::exit(1)
        });
    let Some((_, program)) = candidates.into_iter().find(|(kind, _)| *kind == candidate) else {
        eprintln!(
            "no `{}` candidate exists for {m}x{n}x{k} on {chip}",
            candidate.label()
        );
        std::process::exit(1)
    };
    // A requested knob the emitted plan does not run is a refusal, never a mislabelled program.
    if let Some(want) = l1acc {
        let got = program
            .work
            .plan
            .map(|pl| pl.pack_accumulate)
            .unwrap_or(false);
        if got != want {
            eprintln!(
                "the `{}` plan for {m}x{n}x{k} packs {} — it cannot print l1acc={}",
                candidate.label(),
                if got { "in place" } else { "by replacement" },
                want as u8
            );
            std::process::exit(1)
        }
    }
    let program = match depth {
        Some(d) => logicaffeine_compile::spatial::passes::size_buffers::resize_operand_fifos(
            &program, &target, d,
        )
        .unwrap_or_else(|e| {
            eprintln!("depth {d} does not fit the site: {e:?}");
            std::process::exit(1)
        }),
        None => program,
    };
    let art = TensixBackend::new(target.clone())
        .lower_tensix(&program)
        .unwrap_or_else(|e| {
            eprintln!("the Tensix lowering refused the program: {e:?}");
            std::process::exit(1)
        });
    std::fs::create_dir_all(dir.join("kernels")).expect("bundle dir");
    // Leave no stale kernel behind: the bundle is exactly the artifact.
    if let Ok(rd) = std::fs::read_dir(dir.join("kernels")) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    for (rel, body) in bundle_dir_files(&art) {
        std::fs::write(dir.join(&rel), body).unwrap_or_else(|e| panic!("write {rel}: {e}"));
    }
    let summary = program.summary();
    let meta = serde_json::json!({
        "shape": [m, n, k],
        "chip": chip,
        "mesh": [target.sites.x, target.sites.y],
        "policy": match policy { NumericsPolicy::Exact => "exact", NumericsPolicy::AllowSpillRounding => "relaxed" },
        "candidate": candidate.label(),
        "arrangement": summary.arrangement_label(),
        "summary": summary,
        "census": Census::of(&program),
        "cfg_line": art.cfg_line,
        "kernels": art.kernels.iter().map(|k| k.name.clone()).collect::<Vec<_>>(),
    });
    std::fs::write(
        dir.join("spatial.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .expect("spatial.json");
    println!("{}", art.cfg_line);
    println!(
        "wrote {} kernels + host to {}",
        art.kernels.len(),
        dir.display()
    );
}
