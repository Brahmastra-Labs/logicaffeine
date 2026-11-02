//! wprog_rtl_lock — RECURSION ON SILICON: the whole-program engine lowered to synthesizable RTL runs a
//! recursive function bit-identically to the model (`eval_wprog_config`) — itself proven ≡ the VM — under
//! iverilog. This is the deployment step for the whole-program engine: the register file + block-walk
//! sequencer + terminator FSM + a FRAME STACK (saving the caller's register file on `Call`, restoring it on
//! `Return`) exist in hardware, so a program that calls itself to arbitrary depth runs on the chip, not just
//! in the interpreter. Where `fabric_sequencer_lock` proved the branchy control plane in RTL, this adds the
//! call stack — the last piece that makes the sequenced XPU genuinely WHOLE-PROGRAM on silicon.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_rtl_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_testbench, emit_wprog_verilog, eval_wprog_config, program_calls_fabric,
    serialize_program, WBlock, WCell, WProgConfig, WSrc, WTerm,
};
use logicaffeine_compile::compile::vm_outcome;

/// Build the RTL for `cfg`, run it under iverilog for each param vector, and assert every result matches the
/// `oracle` (== eval_wprog_config, itself cross-checked vs the VM by the callers). Returns false if iverilog
/// is unavailable (the test then no-ops). `name` keys the emitted `.v`/`.hex` files.
fn iverilog_matches(name: &str, cfg: &WProgConfig, inputs: &[i64], oracle: &[i64]) -> bool {
    if !common::require_tool("iverilog") {
        return false;
    }
    let dir = std::env::temp_dir().join("logos_wprog_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let rtl = emit_wprog_verilog(name, cfg);
    let tb = emit_wprog_testbench(name, cfg.n_params, inputs.len());
    let srcv = dir.join(format!("wprog_{name}.v"));
    let tbv = dir.join(format!("tb_wprog_{name}.v"));
    let sim = dir.join(format!("sim_wprog_{name}.vvp"));
    std::fs::write(&srcv, &rtl).expect("write rtl");
    std::fs::write(&tbv, &tb).expect("write tb");
    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&v| format!("{:016x}\n", v as u64))
        .collect();
    std::fs::write(dir.join(format!("wprog_{name}_in.hex")), pin).expect("write in.hex");
    std::fs::write(dir.join(format!("wprog_{name}_out.hex")), oout).expect("write out.hex");
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
        "iverilog failed on {name}:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the {name} RTL diverged from the model:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    true
}

#[test]
fn division_runs_on_the_whole_program_engine_rtl_matching_the_model() {
    // Func cells (a hardware divider): x/2 + x%3 lowers to DivS + ModS functional units on the fabric.
    let src = "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 6] = [4, 7, 10, 17, 20, 33];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "division model != VM at x={x}");
            m
        })
        .collect();
    if iverilog_matches("div", &cfg, &inputs, &oracle) {
        println!("DIVISION ON SILICON: x/2 + x%3 runs on the whole-program engine RTL (Func divider units) ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn collections_on_the_heap_run_on_the_whole_program_engine_rtl_matching_the_model() {
    // Heap cells (the state plane): NewList → HeapAlloc + HeapWrites, `Set item i` → HeapWrite, `item i` →
    // HeapRead — all threaded in order through the sequential per-cell engine + its heap BRAM.
    let src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 5] = [7, 20, 0, 5, 42];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "collection model != VM at x={x}");
            m
        })
        .collect();
    if iverilog_matches("heap", &cfg, &inputs, &oracle) {
        println!("COLLECTIONS ON SILICON: [10,x,30] with set-item/index on the heap state plane runs on the whole-program engine RTL ≡ eval_wprog_config ≡ VM");
    }
}

fn vm_f(src: &str, n: i64) -> i64 {
    let arg = if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({arg}).\n");
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error at n={n}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<i64>()
        .unwrap()
}

#[test]
fn recursion_runs_on_the_whole_program_engine_rtl_matching_the_model() {
    // f(n) = if n <= 0 { 0 } else { n + f(n-1) } = sum(0..=n) — a genuine self-recursion (a `Call` back into
    // the same function's entry block), so the RTL MUST use its frame stack: each level saves n and the
    // return slot, and the base case unwinds the whole chain.
    let src = "## To f (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + f(n - 1).\n";
    let cfg = program_calls_fabric(src, "f");
    let inputs: [i64; 6] = [0, 1, 3, 5, 8, 12];

    // The model reference — and cross-check it against the RUNNING VM so the oracle is anchored to truth.
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(m, vm_f(src, n), "eval_wprog_config != VM at n={n}");
            assert_eq!(
                m,
                (0..=n.max(0)).sum::<i64>(),
                "sum-down closed form at n={n}"
            );
            m
        })
        .collect();

    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_wprog_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let rtl = emit_wprog_verilog("rec", &cfg);
    let tb = emit_wprog_testbench("rec", 1, inputs.len());
    let srcv = dir.join("wprog_rec.v");
    let tbv = dir.join("tb_wprog_rec.v");
    let sim = dir.join("sim_wprog.vvp");
    std::fs::write(&srcv, &rtl).expect("write rtl");
    std::fs::write(&tbv, &tb).expect("write tb");

    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&v| format!("{:016x}\n", v as u64))
        .collect();
    std::fs::write(dir.join("wprog_rec_in.hex"), pin).expect("write in.hex");
    std::fs::write(dir.join("wprog_rec_out.hex"), oout).expect("write out.hex");

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
        "iverilog failed to compile the whole-program engine:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the whole-program engine RTL diverged from eval_wprog_config:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    println!(
        "WHOLE-PROGRAM ENGINE ON SILICON: the compiled program for f(n)=n+f(n-1) (register file + block-walk \
         FSM + branch + loop) runs on the whole-program engine RTL ≡ eval_wprog_config ≡ VM under iverilog — \
         for n in {inputs:?}"
    );
}

/// A hand-built config with a GENUINE `WTerm::Call` — f(n) = if n<=0 {0} else n + f(n-1) as real recursion (a
/// call back into block 0), so the RTL's FRAME STACK is actually exercised: each level pushes the caller's
/// register file, the base case begins the unwind, and every `Return` pops a frame and writes the return value
/// into the caller's `dst` register. This is the piece that makes the sequenced XPU whole-program on silicon.
fn recursive_call_config() -> WProgConfig {
    let in_ = |k: u16| WSrc {
        mode: 0,
        reference: k,
        konst: 0,
    };
    let cst = |v: i64| WSrc {
        mode: 1,
        reference: 0,
        konst: v,
    };
    let val = |j: u16| WSrc {
        mode: 2,
        reference: j,
        konst: 0,
    };
    WProgConfig {
        n_params: 1,
        blocks: vec![
            // blk 0 (entry): cell0 = (n <= 0) ? 1 : 0 ; branch cond TRUE → base(1), FALSE → recurse(2).
            WBlock {
                livein: vec![0],
                cells: vec![WCell {
                    kind: 1,
                    code: 2,
                    ops: [in_(0), cst(0), cst(1), cst(0)],
                }],
                writes: vec![],
                term: WTerm::Branch {
                    cond: val(0),
                    then_blk: 1,
                    else_blk: 2,
                },
            },
            // blk 1 (base): return 0.
            WBlock {
                livein: vec![],
                cells: vec![],
                writes: vec![],
                term: WTerm::Return(cst(0)),
            },
            // blk 2 (recurse): cell0 = n - 1 ; CALL f(n-1) → reg 1, resume at blk 3.
            WBlock {
                livein: vec![0],
                cells: vec![WCell {
                    kind: 0,
                    code: 1,
                    ops: [in_(0), cst(1), cst(0), cst(0)],
                }],
                writes: vec![],
                term: WTerm::Call {
                    callee_block: 0,
                    args: vec![val(0)],
                    dst_reg: 1,
                    ret_block: 3,
                },
            },
            // blk 3 (continuation): cell0 = n + retval ; return it (n is the restored live-in, reg 1 the retval).
            WBlock {
                livein: vec![0, 1],
                cells: vec![WCell {
                    kind: 0,
                    code: 0,
                    ops: [in_(0), in_(1), cst(0), cst(0)],
                }],
                writes: vec![],
                term: WTerm::Return(val(0)),
            },
        ],
    }
}

#[test]
fn a_real_call_stack_drives_the_frame_stack_rtl_matching_the_model() {
    let cfg = recursive_call_config();
    let inputs: [i64; 6] = [0, 1, 2, 4, 7, 10];
    // Sanity: the model computes sum(0..=n) via the frame stack, and each `Return` pops correctly.
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(m, (0..=n).sum::<i64>(), "call-stack model at n={n}");
            m
        })
        .collect();

    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_wprog_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let rtl = emit_wprog_verilog("call", &cfg);
    let tb = emit_wprog_testbench("call", 1, inputs.len());
    let srcv = dir.join("wprog_call.v");
    let tbv = dir.join("tb_wprog_call.v");
    let sim = dir.join("sim_wprog_call.vvp");
    std::fs::write(&srcv, &rtl).expect("write rtl");
    std::fs::write(&tbv, &tb).expect("write tb");
    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&v| format!("{:016x}\n", v as u64))
        .collect();
    std::fs::write(dir.join("wprog_call_in.hex"), pin).expect("write in.hex");
    std::fs::write(dir.join("wprog_call_out.hex"), oout).expect("write out.hex");

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
        "iverilog failed on the frame-stack engine:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the FRAME STACK RTL diverged from the model:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    println!(
        "RECURSION ON SILICON (real call stack): f(n)=n+f(n-1) with a genuine WTerm::Call runs on the \
         whole-program engine RTL — the FRAME STACK pushes/pops per level ≡ eval_wprog_config — for n in {inputs:?}"
    );
}
