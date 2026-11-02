//! block_engine_lock — Stage 5 of the self-synthesizing bytecode XPU: the on-chip EXECUTE engine for a
//! decoded block, proven ≡ the VM under iverilog. ONE universal reconfigurable datapath runs a diverse
//! corpus of straight-line programs by loading each program's DECODED OP LIST as config — no host placement
//! step, no per-program hardwired grid. This is the unit the LGB1 decoder feeds: the ops ARE the config, so
//! `bytecode → ops → this engine → result` runs entirely on-chip.
//!
//! For each program: compile its residual to an op list, serialize the list into the engine's config, and
//! check the engine (loaded with that config) computes exactly what the VM computes over a battery of inputs.
//! Every program on ONE engine ⇒ it is a universal on-chip execute unit for decoded blocks.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(block_engine_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::block_engine_rtl::{
    emit_block_engine_testbench, emit_block_engine_verilog, serialize_block,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_fabric, residual_from_source, FabricOp};
use logicaffeine_compile::compile::vm_outcome;

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
    // A deeper residual — five ops, three routed `Val` operands — exercising the prior-cell operand muxes.
    Prog {
        name: "horner",
        src: "## To horner (x: Int) -> Int:\n    Return ((2 * x + 3) * x + 4) * x + 5.\n",
        args: &[&[0], &[1], &[2], &[-1], &[-3], &[10]],
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

#[test]
fn the_block_engine_runs_decoded_ops_matching_the_vm() {
    if !common::require_tool("iverilog") {
        return;
    }

    // Compile each program's residual; size the ONE engine to fit the largest (cells + inputs).
    let residuals: Vec<(Vec<FabricOp>, usize, usize)> = CORPUS
        .iter()
        .map(|p| residual_from_source(p.src, p.name))
        .collect();
    let k = residuals.iter().map(|(ops, _, _)| ops.len()).max().unwrap();
    let n_in = residuals.iter().map(|(_, _, n)| *n).max().unwrap();
    assert!(
        k >= 2 && n_in >= 1,
        "a genuine multi-cell engine with real inputs"
    );

    let dir = std::env::temp_dir().join("logos_bexec_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // The engine + testbench are FIXED RTL — compile once; re-run per program with its config + vectors.
    let rtl = emit_block_engine_verilog("engine", k, n_in);
    let tb = emit_block_engine_testbench("engine", n_in, N_VECTORS);
    let srcv = dir.join("bexec_engine.v");
    let tbv = dir.join("tb_bexec_engine.v");
    let sim = dir.join("sim_bexec.vvp");
    std::fs::write(&srcv, &rtl).expect("write bexec rtl");
    std::fs::write(&tbv, &tb).expect("write bexec tb");
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
        "iverilog failed to compile the block engine: {}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );

    for (p, (ops, result, actual_n_in)) in CORPUS.iter().zip(residuals.iter()) {
        assert_eq!(
            p.args.len(),
            N_VECTORS,
            "{}: needs exactly {N_VECTORS} vectors",
            p.name
        );
        let cfg = serialize_block(ops, *result, k);

        let ctrl_hex: String = cfg.ctrl.iter().map(|w| format!("{w:08x}\n")).collect();
        let ac_hex: String = cfg
            .ac
            .iter()
            .map(|v| format!("{:016x}\n", *v as u64))
            .collect();
        let bc_hex: String = cfg
            .bc
            .iter()
            .map(|v| format!("{:016x}\n", *v as u64))
            .collect();
        let res_hex = format!("{:016x}\n", cfg.result as u64);
        std::fs::write(dir.join("bexec_engine_ctrl.hex"), ctrl_hex).unwrap();
        std::fs::write(dir.join("bexec_engine_ac.hex"), ac_hex).unwrap();
        std::fs::write(dir.join("bexec_engine_bc.hex"), bc_hex).unwrap();
        std::fs::write(dir.join("bexec_engine_res.hex"), res_hex).unwrap();

        let mut pin = String::new();
        let mut eout = String::new();
        for args in p.args {
            assert_eq!(
                args.len(),
                *actual_n_in,
                "{}: vector arity mismatch",
                p.name
            );
            for j in 0..n_in {
                let v = if j < args.len() { args[j] } else { 0 };
                pin.push_str(&format!("{:016x}\n", v as u64));
            }
            // The engine reproduces eval_fabric; assert that reference matches the VM before checking RTL.
            let ins: Vec<i64> = (0..*actual_n_in).map(|j| args[j]).collect();
            let ref_val = eval_fabric(ops, &ins, *result);
            let vm = vm_call(p.src, p.name, args);
            assert_eq!(
                ref_val, vm,
                "{}: eval_fabric {ref_val} != VM {vm} on {args:?}",
                p.name
            );
            eout.push_str(&format!("{:016x}\n", vm as u64));
        }
        std::fs::write(dir.join("bexec_engine_pin.hex"), pin).unwrap();
        std::fs::write(dir.join("bexec_engine_eout.hex"), eout).unwrap();

        let run = std::process::Command::new("vvp")
            .arg(&sim)
            .current_dir(&dir)
            .output()
            .expect("run vvp");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            stdout.contains("PASS:"),
            "block engine != VM for `{}`: {stdout}\n--- config ---\n{:?}",
            p.name,
            cfg.ctrl
        );
    }

    println!(
        "Stage 5: {} straight-line programs run on ONE universal {k}-cell on-chip execute engine by loading each program's DECODED OP LIST as config — all ≡ the VM under iverilog (bytecode ops → result, no host placement)",
        CORPUS.len()
    );
}
