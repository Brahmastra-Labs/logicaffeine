//! reconfig_fabric_lock — Stage 4 of the self-synthesizing bytecode XPU: the placer in silicon as a
//! RECONFIGURABLE fabric, proven ≡ the VM under iverilog. ONE fixed universal tile array runs a DIVERSE
//! corpus of straight-line programs by loading each program's CERTIFIED placement (`route_loop_body`, proven
//! semantics-preserving by `verified_placer_lock`) as runtime config — no per-program hardwired grid. This is
//! the reconfigurable array the on-chip placer targets: the config IS the specialized program, and the same
//! silicon runs any of them.
//!
//! For each program: compile its residual, place it with `route_loop_body`, pad to the array size, serialize
//! the placement into the fabric's config, and check the fabric (loaded with that config) computes exactly
//! what the VM computes over a battery of inputs. Every program on ONE array ⇒ the fabric is universal +
//! reconfigurable, and the certified placement is what specializes it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(reconfig_fabric_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_grid::{eval_grid, pad_grid, route_loop_body, Grid};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_fabric, residual_from_source, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::reconfig_fabric_rtl::{
    emit_reconfig_fabric_verilog, emit_reconfig_testbench, serialize_grid_config,
};
use logicaffeine_compile::compile::vm_outcome;

/// One straight-line program: source, callee, and a battery of argument vectors (each vector's length is the
/// function's arity).
struct Prog {
    src: &'static str,
    name: &'static str,
    args: &'static [&'static [i64]],
}

const CORPUS: &[Prog] = &[
    Prog {
        name: "poly",
        src: "## To poly (x: Int) -> Int:\n    Return 3 * x + 7.\n",
        args: &[&[0], &[3], &[-5], &[12], &[-100], &[7]],
    },
    Prog {
        name: "lin",
        src: "## To lin (x: Int) -> Int:\n    Return x + x + x + 5.\n",
        args: &[&[0], &[1], &[-2], &[10], &[-50], &[123]],
    },
    Prog {
        name: "quad",
        src: "## To quad (x: Int, y: Int) -> Int:\n    Return x * x + y.\n",
        args: &[&[0, 0], &[3, 1], &[-4, 5], &[7, -2], &[10, 100], &[-6, -6]],
    },
    Prog {
        name: "mix",
        src: "## To mix (a: Int, b: Int) -> Int:\n    Return a * b - a + 2.\n",
        args: &[&[0, 0], &[3, 4], &[-2, 5], &[6, -3], &[10, 10], &[-7, 2]],
    },
];

const N_VECTORS: usize = 6;

fn vm_call(src: &str, name: &str, args: &[i64]) -> i64 {
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let arglist = args.iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
    let prog = format!("{src}## Main\n    Show {name}({arglist}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {name}{args:?}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int output")
}

/// Build a program's placed grid (its residual → LoopCell body → `route_loop_body` single-`Val` output).
fn placed_grid(src: &str, name: &str) -> (Grid, usize) {
    let (ops, result, n_in) = residual_from_source(src, name);
    let cells: Vec<LoopCell> = ops
        .iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect();
    let g = route_loop_body(&cells, n_in, &[Src::Val(result)]);
    // Sanity: the placement computes the datapath (the property Stage 3 certified for all such bodies).
    for &x in &[0i64, 1, -2, 7] {
        let ins: Vec<i64> = (0..n_in).map(|k| x + k as i64).collect();
        assert_eq!(
            eval_grid(&g, &ins),
            eval_fabric(&ops, &ins, result),
            "{name}: placed grid != datapath"
        );
    }
    (g, n_in)
}

#[test]
fn the_reconfigurable_fabric_runs_the_corpus_matching_the_vm() {
    if !common::require_tool("iverilog") {
        return;
    }

    // Size the ONE universal array to fit every program's placement, then pad each to that shape.
    let grids: Vec<(Grid, usize)> = CORPUS.iter().map(|p| placed_grid(p.src, p.name)).collect();
    let rows = grids.iter().map(|(g, _)| g.rows).max().unwrap();
    let cols = grids.iter().map(|(g, _)| g.cols).max().unwrap();
    let n_in = grids.iter().map(|(_, n)| *n).max().unwrap();
    assert!(
        rows * cols > 4 && n_in >= 1,
        "a genuine 2-D array with real inputs"
    );

    let dir = std::env::temp_dir().join("logos_reconfig_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // The fabric + testbench are FIXED RTL — compile once; re-run per program with its config + vectors.
    let rtl = emit_reconfig_fabric_verilog("engine", rows, cols, n_in);
    let tb = emit_reconfig_testbench("engine", n_in, N_VECTORS);
    let srcv = dir.join("reconfig_engine.v");
    let tbv = dir.join("tb_reconfig_engine.v");
    let sim = dir.join("sim_reconfig.vvp");
    std::fs::write(&srcv, &rtl).expect("write reconfig rtl");
    std::fs::write(&tbv, &tb).expect("write reconfig tb");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the reconfig fabric: {}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );

    for (p, (g, actual_n_in)) in CORPUS.iter().zip(grids.iter()) {
        assert_eq!(
            p.args.len(),
            N_VECTORS,
            "{}: needs exactly {N_VECTORS} vectors",
            p.name
        );
        let padded = pad_grid(g, rows, cols);
        // Padding is semantics-preserving (a safety check on the array-fit step).
        for &x in &[0i64, 1, -3, 9] {
            let ins: Vec<i64> = (0..*actual_n_in).map(|k| x + k as i64).collect();
            assert_eq!(
                eval_grid(&padded, &ins),
                eval_grid(g, &ins),
                "{}: pad changed semantics",
                p.name
            );
        }
        let cfg = serialize_grid_config(&padded);

        // The fabric's config for this program.
        let ctrl_hex: String = cfg.ctrl.iter().map(|w| format!("{w:08x}\n")).collect();
        let sac_hex: String = cfg
            .sac
            .iter()
            .map(|v| format!("{:016x}\n", *v as u64))
            .collect();
        let sbc_hex: String = cfg
            .sbc
            .iter()
            .map(|v| format!("{:016x}\n", *v as u64))
            .collect();
        let out_hex = format!(
            "{:016x}\n{:016x}\n",
            cfg.out_sel as u64, cfg.out_chan as u64
        );
        std::fs::write(dir.join("reconfig_engine_ctrl.hex"), ctrl_hex).unwrap();
        std::fs::write(dir.join("reconfig_engine_sac.hex"), sac_hex).unwrap();
        std::fs::write(dir.join("reconfig_engine_sbc.hex"), sbc_hex).unwrap();
        std::fs::write(dir.join("reconfig_engine_out.hex"), out_hex).unwrap();

        // The input vectors (padded to the array's input arity) + the VM oracle.
        let mut pin = String::new();
        let mut eout = String::new();
        for args in p.args {
            assert_eq!(
                args.len(),
                *actual_n_in,
                "{}: vector arity mismatch",
                p.name
            );
            for k in 0..n_in {
                let v = if k < args.len() { args[k] } else { 0 };
                pin.push_str(&format!("{:016x}\n", v as u64));
            }
            let vm = vm_call(p.src, p.name, args);
            eout.push_str(&format!("{:016x}\n", vm as u64));
        }
        std::fs::write(dir.join("reconfig_engine_pin.hex"), pin).unwrap();
        std::fs::write(dir.join("reconfig_engine_eout.hex"), eout).unwrap();

        let run = std::process::Command::new("vvp")
            .arg(&sim)
            .current_dir(&dir)
            .output()
            .expect("run vvp");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            stdout.contains("PASS:"),
            "reconfig fabric != VM for `{}`: {stdout}\n--- config ctrl ---\n{:?}",
            p.name,
            cfg.ctrl
        );
    }

    println!(
        "Stage 4: {} straight-line programs run on ONE universal {rows}x{cols} reconfigurable fabric by loading each program's CERTIFIED placement as config — all ≡ the VM under iverilog (the placer in silicon)",
        CORPUS.len()
    );
}
