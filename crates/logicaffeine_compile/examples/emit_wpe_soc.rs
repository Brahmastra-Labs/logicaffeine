//! emit_wpe_soc — LOGOS source → the bytes the whole-program FABRIC ENGINE loads over the wire (and, once, the
//! engine's synthesizable RTL). This is the compiler half of the FPGA demo: `scripts/logos-chip.py` calls it to
//! turn a `.lg` into a `.wire` and the VM golden the chip's answer is checked against.
//!
//! The engine is UNIVERSAL: one netlist runs every fitting program, so `--engine` is written once and reused;
//! only the streamed `.wire` (the program's flat image + its params) changes per program.
//!
//!   cargo run -q -p logicaffeine-compile --example emit_wpe_soc -- \
//!       --program prog.lg --fn f --args 5 --wire prog.wire --profile stream
//!
//!   --main                        run the program's `## Main` (its `Show` output is the result; the golden is the
//!                                 VM's output, one line per Show, printed as `golden_json=[...]`)
//!   --caps NREG=256,DEPTH=32,…    the caps of the image actually FLASHED (the host reads them from the
//!                                 `<bitstream>.build.txt` the build writes); every dimension is checked, and
//!                                 DEPTH/HEAP are MEASURED by running the program on the model — the two the
//!                                 static profile check is blind to
//!   --profile hw|stream|generic   which DEPLOYED engine the program must fit: `hw` = the XC7A100T flip-flop
//!                                 engine (default), `stream` = the XC7A200T block-RAM engine. A `.wire` carries
//!                                 no record of the profile it was built for, and an over-cap image loads fine
//!                                 and returns a plausible wrong number — so this fit check is the only guard.
//!   --engine <path>               emit the engine RTL (`--synth` / `--configbram` pick the big variants)
//!   --demo recursion              a hand-built recursive config instead of a `.lg`
//!
//! On success stdout carries one `key=value` line per fact a host needs: `golden=` (the VM's answer for
//! `f(args)`), `lifter=` (which lowering produced the image), `profile=`, `bytes=`, and `map_json=` — what each
//! block and cell of the image does (`engine_build::fabric_map`), for the Fabric Terminal's panel labels.

use std::collections::HashMap;

use logicaffeine_compile::codegen_sva::engine_build::{
    fabric_map, fits_caps, lower, lower_main, main_register_names, Lifter, Profile,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_backend, emit_wprog_engine_soc_verilog_hw, eval_wprog_config,
    wprog_config_image_unchecked, MemBackend, WBlock, WCell, WProgConfig, WSrc, WTerm, WPE_STREAM_CAPS,
};

/// f(n) = if n<=0 {0} else n + f(n-1) — a genuine recursive `Call` config, so the streamed program exercises
/// the engine's FRAME STACK on real silicon (interpret Call → save/push, Return → pop). The `--demo recursion`
/// program; no `.lg` because the single-function CFG lifter does not emit `Call` (that is program_calls_fabric).
fn recursive_sumdown_config() -> WProgConfig {
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

fn main() {
    let mut engine: Option<String> = None;
    let mut program: Option<String> = None;
    let mut demo: Option<String> = None;
    let mut func = "f".to_string();
    let mut args_csv = String::new();
    let mut wire: Option<String> = None;
    let mut profile = "hw".to_string();
    let mut caps: Option<String> = None; // --caps: the flashed image's caps, every dimension checked (DEPTH/HEAP measured)
    let mut main = false; // --main: run the program's `## Main`; its Show output is the result
    let mut synth = false; // --synth: emit the big BRAM (WPE_SYNTH) engine instead of the deployed hw one
    let mut configbram = false; // --configbram: emit the ConfigBram engine (WPE_STREAM_CAPS) — the 200T target
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--engine" => engine = it.next(),
            "--program" => program = it.next(),
            "--demo" => demo = it.next(),
            "--fn" => func = it.next().unwrap_or(func),
            "--args" => args_csv = it.next().unwrap_or_default(),
            "--wire" => wire = it.next(),
            "--profile" => profile = it.next().unwrap_or(profile),
            "--main" => main = true,
            "--caps" => caps = it.next(),
            "--synth" => synth = true,
            "--configbram" => configbram = true,
            other => {
                eprintln!("emit_wpe_soc: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(engine_profile) = Profile::from_name(&profile) else {
        eprintln!("emit_wpe_soc: unknown --profile {profile} (known: hw, stream, generic)");
        std::process::exit(2);
    };

    if let Some(path) = &engine {
        // The module name is derived from nexys_wpe_soc.v's instantiation (`wpe_soc_fpga`). `--synth` emits the
        // big BRAM engine (HEAP=64/DEPTH=8, heap/sfile in block RAM); default is the deployed hw (all-flip-flop).
        let rtl = if configbram {
            emit_wprog_engine_soc_verilog_backend("fpga", WPE_STREAM_CAPS, MemBackend::ConfigBram)
        } else if synth {
            logicaffeine_compile::codegen_sva::wprog_engine_rtl::emit_wprog_engine_soc_verilog_synth(
                "fpga",
            )
        } else {
            emit_wprog_engine_soc_verilog_hw("fpga")
        };
        std::fs::write(path, &rtl).unwrap_or_else(|e| {
            eprintln!("emit_wpe_soc: cannot write {path}: {e}");
            std::process::exit(1);
        });
        eprintln!(
            "emit_wpe_soc: wrote engine RTL -> {path} ({} bytes)",
            rtl.len()
        );
    }

    let Some(wire_path) = &wire else { return };
    if program.is_none() && demo.is_none() {
        return;
    }
    let params: Vec<i64> = if args_csv.trim().is_empty() {
        vec![]
    } else {
        args_csv
            .split(',')
            .map(|s| {
                s.trim().parse::<i64>().unwrap_or_else(|_| {
                    eprintln!("emit_wpe_soc: bad --args value {s:?}");
                    std::process::exit(2);
                })
            })
            .collect()
    };
    let label = demo.clone().or_else(|| program.clone()).unwrap_or_default();
    // A Main program's golden is its OUTPUT (lines), not a return value; `golden_lines` carries it to the end.
    let mut golden_lines: Option<Vec<String>> = None;
    let (cfg, lifter, golden, names) = if let Some(kind) = &demo {
        let cfg = match kind.as_str() {
            "recursion" => recursive_sumdown_config(),
            other => {
                eprintln!("emit_wpe_soc: unknown --demo {other} (known: recursion)");
                std::process::exit(2);
            }
        };
        // A hand-built config has no LOGOS source, so its golden is the engine model's own answer.
        let golden = eval_wprog_config(&cfg, &params);
        (cfg, "demo", golden, HashMap::new())
    } else {
        let prog = program.as_ref().unwrap();
        let src = std::fs::read_to_string(prog).unwrap_or_else(|e| {
            eprintln!("emit_wpe_soc: cannot read {prog}: {e}");
            std::process::exit(1);
        });
        if main {
            if !params.is_empty() {
                eprintln!("emit_wpe_soc: --main takes no --args (Main has no parameters)");
                std::process::exit(2);
            }
            let (cfg, lines) = lower_main(&src).unwrap_or_else(|e| {
                eprintln!("emit_wpe_soc: {prog}: {e}");
                std::process::exit(1);
            });
            golden_lines = Some(lines);
            (cfg, Lifter::Main.name(), 0, main_register_names(&src))
        } else {
            let (cfg, lifter, golden) = lower(&src, &func, &params).unwrap_or_else(|e| {
                eprintln!("emit_wpe_soc: {prog}: {e}");
                std::process::exit(1);
            });
            (cfg, lifter.name(), golden, HashMap::new())
        }
    };
    if let Err(e) = engine_profile.fits(&cfg) {
        eprintln!("emit_wpe_soc: {label} does not fit the {profile} engine: {e}");
        std::process::exit(1);
    }
    if let Some(c) = &caps {
        if let Err(e) = fits_caps(&cfg, &params, c) {
            eprintln!("emit_wpe_soc: {label} does not fit the flashed image: {e}");
            std::process::exit(1);
        }
    }
    let bytes = wprog_config_image_unchecked(&cfg).wire_bytes(&params);
    std::fs::write(wire_path, &bytes).unwrap_or_else(|e| {
        eprintln!("emit_wpe_soc: cannot write {wire_path}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "emit_wpe_soc: wrote {} wire bytes for {} -> {wire_path}",
        bytes.len(),
        if main {
            "Main".to_string()
        } else {
            format!("{func}({args_csv})")
        }
    );
    match &golden_lines {
        Some(lines) => {
            println!(
                "golden_json={}",
                serde_json::to_string(lines).expect("json")
            );
            println!("shows={}", lines.len());
        }
        None => println!("golden={golden}"),
    }
    println!("lifter={lifter}");
    println!("profile={profile}");
    println!("bytes={}", bytes.len());
    println!("map_json={}", serde_json::to_string(&fabric_map(&cfg, &names)).expect("json"));
}
