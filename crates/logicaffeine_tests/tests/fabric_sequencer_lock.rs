//! fabric_sequencer_lock — Stage 1 of the self-synthesizing bytecode XPU: the AUTONOMOUS block-walk
//! SEQUENCER as synthesizable RTL, proven ≡ the VM under iverilog.
//!
//! `fabric_whole_program_lock`'s Stage X4 proved that a branchy whole program runs on ONE reconfigurable
//! grid IN SIMULATION (`run_program_cfg_on_grid`: a register file + a place-per-block walk of the
//! Jump/Branch/Return graph). But that walk lived only in Rust — the SILICON control plane did not exist.
//! This lock builds it: `emit_sequencer_verilog` emits the RTL of exactly that loop —
//!
//!   • a REGISTER FILE that survives block reconfiguration,
//!   • per-block PLACED grids (`route_loop_body`, the block's live-in registers as inputs, its register
//!     writes + terminator operand as outputs),
//!   • a block PROGRAM-COUNTER + a terminator FSM (Jump → next block, Branch → predicate picks the
//!     successor, Return → capture + halt),
//!   • a CYCLE COUNTER,
//!
//! — a hardwired micro-sequencer that walks the whole function ITSELF, one block per clock, with no host
//! help. It is proven bit-identical to `run_program_cfg_on_grid` (≡ `eval_program_cfg_fabric` ≡ the VM,
//! already locked by `fabric_whole_program_lock` + `fabric_cfg_lock`) under a real gate-level simulator,
//! over a battery of inputs per program — checking BOTH the returned value AND the exact number of block
//! steps (the sequencer must walk the SAME trajectory as the model, not merely reach the same answer).
//!
//! The reference is `eval_sequencer`, which IS `run_program_cfg_on_grid` with a step counter (same
//! route-per-block + `eval_grid_all` path), so the chain is RTL ≡ eval_sequencer ≡ run_program_cfg_on_grid
//! ≡ eval_program_cfg_fabric ≡ VM. Skipped gracefully where iverilog is absent (the Rust-level equivalence
//! still runs).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(fabric_sequencer_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_grid::run_program_cfg_on_grid;
use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_program_cfg_fabric, program_cfg_fabric};
use logicaffeine_compile::codegen_sva::sequencer_rtl::{
    emit_sequencer_testbench, emit_sequencer_verilog, eval_sequencer,
};
use logicaffeine_compile::compile::vm_outcome;

/// One branchy whole-program datapath: a LOGOS function, its callee name, and an input battery. Each is a
/// genuine MULTI-block CFG (branches / early returns), acyclic (no loops — that is the loop fabric's job).
struct SeqFn {
    src: &'static str,
    name: &'static str,
    args: &'static [&'static [i64]],
}

const CORPUS: &[SeqFn] = &[
    // Three-way clamp: four blocks, two Branches, three return paths — returns inputs/constants.
    SeqFn {
        name: "clamp",
        src: "## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    If x is less than lo:\n        Return lo.\n    If x is greater than hi:\n        Return hi.\n    Return x.\n",
        args: &[&[5, 0, 10], &[-3, 0, 10], &[20, 0, 10], &[0, 0, 10], &[10, 0, 10], &[7, -5, 5], &[-100, -5, 5]],
    },
    // Three-way sign: nested branches, three returns of constants.
    SeqFn {
        name: "sign",
        src: "## To sign (n: Int) -> Int:\n    If n is less than 0:\n        Return 0 - 1.\n    If n is equal to 0:\n        Return 0.\n    Return 1.\n",
        args: &[&[-7], &[-1], &[0], &[1], &[42], &[100], &[-100]],
    },
    // A step-grade ladder: three comparisons on input+constant, four returns — a deeper block chain.
    SeqFn {
        name: "grade",
        src: "## To grade (s: Int) -> Int:\n    If s is less than 60:\n        Return 0.\n    If s is less than 70:\n        Return 1.\n    If s is less than 90:\n        Return 2.\n    Return 3.\n",
        args: &[&[0], &[59], &[60], &[69], &[70], &[89], &[90], &[100]],
    },
    // The REGISTER-FILE stress: a value COMPUTED in the entry block (y = x*x) is live across a branch and
    // READ in BOTH successor blocks. This is the one the control plane exists for — a register written
    // under one configuration and consumed under another must survive the reconfiguration. (The branch is
    // on the input `x`, not on `y`: `route_loop_body` places a mux tile only when its comparison operands
    // are inputs/constants — routing a computed predicate onto the array is the placer's later work.)
    SeqFn {
        name: "sqbucket",
        src: "## To sqbucket (x: Int) -> Int:\n    Let y be x * x.\n    If x is greater than 5:\n        Return y - 100.\n    Return y + x.\n",
        args: &[&[0], &[1], &[5], &[10], &[11], &[-3], &[-12], &[7]],
    },
];

/// Call `name(args)` on the VM and return its integer result — the ground-truth oracle.
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

/// The reference `eval_sequencer` — the sequencer's own trajectory model — must agree with the already-VM-
/// locked oracle chain (`run_program_cfg_on_grid` ≡ `eval_program_cfg_fabric` ≡ VM) on both the RESULT and,
/// implicitly, the walk. This pins the RTL's software reference before it is ever emitted.
#[test]
fn the_sequencer_reference_equals_the_cfg_model_equals_the_vm() {
    for f in CORPUS {
        let pf = program_cfg_fabric(f.src, f.name);
        assert!(
            pf.blocks.len() >= 2,
            "{}: must be a genuine MULTI-block CFG, got {}",
            f.name,
            pf.blocks.len()
        );
        for args in f.args {
            let (seq, steps) = eval_sequencer(&pf, args);
            let grid = run_program_cfg_on_grid(&pf, args);
            let model = eval_program_cfg_fabric(&pf, args);
            let vm = vm_call(f.src, f.name, args);
            assert_eq!(
                seq, grid,
                "{}: eval_sequencer {seq} != grid walk {grid} on {args:?}",
                f.name
            );
            assert_eq!(
                seq, model,
                "{}: eval_sequencer {seq} != CFG model {model} on {args:?}",
                f.name
            );
            assert_eq!(
                seq, vm,
                "{}: eval_sequencer {seq} != VM {vm} on {args:?}",
                f.name
            );
            assert!(
                steps >= 1,
                "{}: a run must take at least one block step on {args:?}",
                f.name
            );
        }
    }
    println!(
        "Stage 1: the sequencer reference ({} branchy programs) ≡ the place-per-block grid walk ≡ the CFG model ≡ the VM",
        CORPUS.len()
    );
}

/// The synthesizable SEQUENCER RTL computes bit-identically to the VM under iverilog — both the returned
/// value AND the exact block-step count — for every input in the battery, for every branchy program.
#[test]
fn the_sequencer_rtl_matches_the_vm_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_sequencer_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    for f in CORPUS {
        let pf = program_cfg_fabric(f.src, f.name);
        let n_params = pf.n_params;
        let rtl = emit_sequencer_verilog(f.name, &pf);
        let tb = emit_sequencer_testbench(f.name, n_params, f.args.len());

        // The VM oracle + the driven inputs + the expected block-step count, all as 64-bit hex.
        let mut inhex = String::new();
        let mut outhex = String::new();
        let mut cychex = String::new();
        for args in f.args {
            assert_eq!(
                args.len(),
                n_params,
                "{}: vector arity {} != param count {n_params}",
                f.name,
                args.len()
            );
            for a in *args {
                inhex.push_str(&format!("{:016x}\n", *a as u64));
            }
            let (seq, steps) = eval_sequencer(&pf, args);
            let vm = vm_call(f.src, f.name, args);
            assert_eq!(
                seq, vm,
                "{}: reference {seq} != VM {vm} on {args:?} (oracle mismatch)",
                f.name
            );
            outhex.push_str(&format!("{:016x}\n", vm as u64));
            cychex.push_str(&format!("{:016x}\n", steps));
        }

        let srcv = dir.join(format!("seq_{}.v", f.name));
        let tbv = dir.join(format!("tb_seq_{}.v", f.name));
        let sim = dir.join(format!("sim_seq_{}.vvp", f.name));
        std::fs::write(&srcv, &rtl).expect("write sequencer rtl");
        std::fs::write(&tbv, &tb).expect("write sequencer tb");
        std::fs::write(dir.join(format!("seq_{}_in.hex", f.name)), &inhex).expect("write in hex");
        std::fs::write(dir.join(format!("seq_{}_out.hex", f.name)), &outhex)
            .expect("write out hex");
        std::fs::write(dir.join(format!("seq_{}_cyc.hex", f.name)), &cychex)
            .expect("write cyc hex");

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
            "iverilog failed to compile seq_{}: {}\n--- RTL ---\n{rtl}",
            f.name,
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
            "seq_{} RTL != VM under iverilog: {stdout}\n--- RTL ---\n{rtl}",
            f.name
        );
    }

    println!(
        "Stage 1: {} branchy whole programs run on the AUTONOMOUS on-chip sequencer (register file + block-PC + terminator FSM + cycle counter) — result AND cycle count ≡ the VM under iverilog",
        CORPUS.len()
    );
}
