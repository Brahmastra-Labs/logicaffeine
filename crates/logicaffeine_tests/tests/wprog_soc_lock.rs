//! wprog_soc_lock — the CONFIG-DRIVEN whole-program processor: ONE fixed generic engine (`wpe_*`) runs ANY
//! program by loading its image (the tables a real chip receives over the wire) into on-chip config memories
//! and interpreting it, proven bit-identical to `eval_wprog_config` (≡ the VM) under iverilog. This is the
//! self-synthesizing bytecode XPU: not a program-specialized netlist, but one processor that runs recursion
//! (frame stack), collections (heap), and division (functional units) — whatever image is loaded.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_soc_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_testbench, emit_wprog_engine_verilog, eval_wprog_config, serialize_program,
    wprog_config_image, WBlock, WCell, WProgConfig, WSrc, WTerm,
};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

fn vm_f(src: &str, x: i64) -> i64 {
    let arg = if x < 0 {
        format!("(0 - {})", -x)
    } else {
        x.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({arg}).\n");
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error at x={x}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<i64>()
        .unwrap()
}

/// f(n) = if n<=0 {0} else n + f(n-1) — a GENUINE recursive `Call` config (the same one the specialized RTL
/// lock uses), so the config-driven engine's frame stack is exercised: interpret Call → save/push, Return → pop.
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
            WBlock {
                livein: vec![],
                cells: vec![],
                writes: vec![],
                term: WTerm::Return(cst(0)),
            },
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

/// Build `cfg`'s image, load it into the generic engine, and check every input runs ≡ the oracle under iverilog.
fn run_config_driven(name: &str, cfg: &WProgConfig, inputs: &[i64], oracle: &[i64]) -> bool {
    if !common::require_tool("iverilog") {
        return false;
    }
    let dir = std::env::temp_dir().join("logos_wpe_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    // The program image: the config memories the engine loads (what a chip receives over the wire).
    let im = wprog_config_image(cfg);
    for (mem, hex) in im.hex_files() {
        std::fs::write(dir.join(format!("wpe_{name}_{mem}.hex")), hex).expect("write config mem");
    }
    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&x| format!("{:016x}\n", x as u64))
        .collect();
    std::fs::write(dir.join(format!("wpe_{name}_in.hex")), pin).expect("write in");
    std::fs::write(dir.join(format!("wpe_{name}_out.hex")), oout).expect("write out");

    let eng = emit_wprog_engine_verilog(name, cfg.n_params);
    let tb = emit_wprog_engine_testbench(name, cfg.n_params, inputs.len());
    let srcv = dir.join(format!("wpe_{name}.v"));
    let tbv = dir.join(format!("tb_wpe_{name}.v"));
    let sim = dir.join(format!("sim_wpe_{name}.vvp"));
    std::fs::write(&srcv, &eng).expect("write engine");
    std::fs::write(&tbv, &tb).expect("write tb");
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
        "iverilog failed on config-driven engine {name}:\n{}\n--- RTL ---\n{eng}",
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
        "the config-driven engine {name} diverged from eval_wprog_config:\n{stdout}"
    );
    true
}

#[test]
fn one_generic_engine_runs_recursion_by_loading_its_image() {
    let cfg = recursive_call_config();
    let inputs: [i64; 6] = [0, 1, 2, 4, 7, 10];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(m, (0..=n).sum::<i64>(), "recursion model at n={n}");
            m
        })
        .collect();
    if run_config_driven("rec", &cfg, &inputs, &oracle) {
        println!("CONFIG-DRIVEN RECURSION: the generic engine LOADS the image of f(n)=n+f(n-1) and runs it on its frame stack ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn one_generic_engine_runs_collections_by_loading_its_image() {
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
    if run_config_driven("heap", &cfg, &inputs, &oracle) {
        println!("CONFIG-DRIVEN COLLECTIONS: the generic engine LOADS [10,x,30]'s image and runs it on its heap state plane ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn one_generic_engine_runs_division_by_loading_its_image() {
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
    if run_config_driven("div", &cfg, &inputs, &oracle) {
        println!("CONFIG-DRIVEN DIVISION: the generic engine LOADS x/2+x%3's image and runs it on its Func units ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn one_generic_engine_runs_the_float_pi_series_by_loading_its_image() {
    // The `pi_leibniz` benchmark's alternating float series, scaled + floored to an Int oracle. Exercises the
    // engine's FLOAT LANE: `f64`-bit-pattern add/sub/mul/div through the behavioral-FPU `func_f`, the INT→FLOAT
    // promotion of the loop counter `k` (`2.0 * k`), and the float→int `floor` bridge — all over the block walk.
    let src = "## To f (n: Int) -> Int:\n    Let mutable sum be 0.0.\n    Let mutable sign be 1.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set sum to sum + sign / (2.0 * k + 1.0).\n        Set sign to 0.0 - sign.\n        Set k to k + 1.\n    Let scaled be sum * 4.0 * 1000000.0.\n    Return floor(scaled).\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 6] = [0, 1, 2, 5, 10, 50];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(m, vm_f(src, n), "float model != VM at n={n}");
            m
        })
        .collect();
    if run_config_driven("pif", &cfg, &inputs, &oracle) {
        println!("CONFIG-DRIVEN FLOAT (π): the generic engine LOADS the Leibniz-series image and runs it on its float lane ≡ eval_wprog_config ≡ VM (n=50 → {})", oracle[5]);
    }
}

#[test]
fn one_generic_engine_runs_the_float_mandelbrot_by_loading_its_image() {
    // The `mandelbrot` benchmark's core iteration with the genuine IEEE-754 escape test `zr² + zi² > 4.0` — a
    // FLOAT COMPARISON (routed to the float-compare Func units, not the integer bit-pattern ordering). Proves
    // the config-driven engine sequences a float-conditional loop correctly under iverilog.
    let src = "## To f (px: Int) -> Int:\n    Let cr be 0.01 * px - 2.0.\n    Let ci be 0.5.\n    Let mutable zr be 0.0.\n    Let mutable zi be 0.0.\n    Let mutable i be 0.\n    While i is less than 100:\n        Let zr2 be zr * zr - zi * zi + cr.\n        Let zi2 be 2.0 * zr * zi + ci.\n        Set zr to zr2.\n        Set zi to zi2.\n        If zr * zr + zi * zi is greater than 4.0:\n            Return i.\n        Set i to i + 1.\n    Return i.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 6] = [0, 50, 100, 150, 250, 399];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&px| {
            let m = eval_wprog_config(&cfg, &[px]);
            assert_eq!(m, vm_f(src, px), "mandelbrot model != VM at px={px}");
            m
        })
        .collect();
    if run_config_driven("mand", &cfg, &inputs, &oracle) {
        println!("CONFIG-DRIVEN FLOAT (Mandelbrot): the generic engine LOADS the escape-count image and runs its float-compare escape test ≡ eval_wprog_config ≡ VM");
    }
}
