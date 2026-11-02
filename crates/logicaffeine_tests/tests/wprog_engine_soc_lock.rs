//! wprog_engine_soc_lock — the OVER-THE-WIRE whole-program engine. One universal netlist (`wpe_soc_*`) RECEIVES
//! a program's flat image + params over a UART, fills its on-chip config memories (the loader FSM SYNTHESIZES the
//! program it is handed), runs the interpret st-machine, and streams the result back — proven bit-identical to
//! `eval_wprog_config` (≡ the VM) under iverilog by a testbench that BIT-BANGS the wire. This is the difference
//! between "the fabric runs a baked-in program" and "send ANY program over the wire and it runs": the config is
//! streamed, not `$readmemh`-loaded. Integer functional units (the synthesizable engine); the float lane is the
//! FPU hardware follow-on.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_engine_soc_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog,
    emit_wprog_engine_soc_verilog_hw, emit_wprog_engine_soc_verilog_synth, eval_wprog_config,
    serialize_program, wprog_config_fits_hw_engine, wprog_config_fits_synth_engine,
    wprog_config_image, WBlock, WCell, WProgConfig, WSrc, WTerm,
};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

const CLKS_PER_BIT: usize = 8; // fast sim; the real board uses 868 (115200 @ 100 MHz)

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

/// f(n) = if n<=0 {0} else n + f(n-1) — a genuine recursive `Call` config (exercises the engine's frame stack
/// over the wire: interpret Call → save/push, Return → pop).
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

fn run_over_the_wire(name: &str, cfg: &WProgConfig, inputs: &[i64], oracle: &[i64]) -> bool {
    run_over_the_wire_impl(name, cfg, inputs, oracle, "behavioral")
}

/// Same, but drive the SYNTHESIZABLE netlist (`emit_wprog_engine_soc_verilog_synth` — small caps, mul-only
/// func_f with the sequential div/mod/floordiv/pow datapath, ram_style=registers) — the EXACT RTL that becomes
/// the Nexys A7 bitstream. Proving this variant ≡VM under iverilog means the hardware runs the identical program
/// the model does.
fn run_over_the_wire_synth(name: &str, cfg: &WProgConfig, inputs: &[i64], oracle: &[i64]) -> bool {
    wprog_config_fits_synth_engine(cfg).expect("config must fit the synth engine's cap profile");
    run_over_the_wire_impl(name, cfg, inputs, oracle, "synth")
}

/// Same, but drive the DEPLOYED hold-safe HARDWARE netlist (`emit_wprog_engine_soc_verilog_hw` — the `WPE_HW_*`
/// cap profile the real Nexys A7 bitstream uses). Proving this variant ≡VM under iverilog means the exact RTL
/// that runs on silicon runs the identical program the model does.
fn run_over_the_wire_hw(name: &str, cfg: &WProgConfig, inputs: &[i64], oracle: &[i64]) -> bool {
    wprog_config_fits_hw_engine(cfg).expect("config must fit the hardware engine's cap profile");
    run_over_the_wire_impl(name, cfg, inputs, oracle, "hw")
}

/// Stream every input's image+params over the (simulated) wire into the universal SoC and check each returned
/// frame == the oracle under iverilog. `oracle[i]` is `eval_wprog_config(cfg, [inputs[i]])`.
fn run_over_the_wire_impl(
    name: &str,
    cfg: &WProgConfig,
    inputs: &[i64],
    oracle: &[i64],
    kind: &str,
) -> bool {
    if !common::require_tool("iverilog") {
        return false;
    }
    let dir = std::env::temp_dir().join("logos_wpe_soc_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Build each vector's wire byte stream (full image + that vector's params + GO), concatenated.
    let im = wprog_config_image(cfg);
    let mut all_bytes: Vec<u8> = Vec::new();
    let mut lens: Vec<u32> = Vec::new();
    for &x in inputs {
        let bytes = im.wire_bytes(&[x]);
        lens.push(bytes.len() as u32);
        all_bytes.extend_from_slice(&bytes);
    }
    let wire_hex: String = all_bytes.iter().map(|b| format!("{b:02x}\n")).collect();
    let lens_hex: String = lens.iter().map(|n| format!("{n:08x}\n")).collect();
    let oracle_hex: String = oracle
        .iter()
        .map(|&x| format!("{:016x}\n", x as u64))
        .collect();
    std::fs::write(dir.join(format!("wire_{name}.hex")), wire_hex).expect("write wire");
    std::fs::write(dir.join(format!("lens_{name}.hex")), lens_hex).expect("write lens");
    std::fs::write(dir.join(format!("oracle_{name}.hex")), oracle_hex).expect("write oracle");

    let soc = match kind {
        "synth" => emit_wprog_engine_soc_verilog_synth(name),
        "hw" => emit_wprog_engine_soc_verilog_hw(name),
        _ => emit_wprog_engine_soc_verilog(name),
    };
    let tb = emit_wprog_engine_soc_testbench(name, inputs.len(), CLKS_PER_BIT, all_bytes.len());
    let srcv = dir.join(format!("wpe_soc_{name}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{name}.v"));
    let sim = dir.join(format!("sim_wpe_soc_{name}.vvp"));
    std::fs::write(&srcv, &soc).expect("write soc");
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
        "iverilog failed on over-the-wire SoC {name}:\n{}\n--- RTL ---\n{soc}",
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
        "the over-the-wire SoC {name} diverged from eval_wprog_config:\n{stdout}"
    );
    true
}

#[test]
fn wire_stream_reproduces_the_config_memories_exactly() {
    // The host's `wire_bytes` and the chip's loader are two ends of one protocol — the bytes MUST reproduce the
    // image the engine loads via `$readmemh`. Check the stream is self-consistent: the length-prefixed blocks
    // decode back to the identical field vectors, so a chip walking the wire builds the identical config.
    let cfg = serialize_program(&program_cfg_fabric(
        "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n",
        "f",
    ));
    let im = wprog_config_image(&cfg);
    let bytes = im.wire_bytes(&[7]);
    // Decode the 23 u32 mems then konst then params then GO, exactly as the loader FSM does.
    let mut p = 0usize;
    let rd_u16 = |b: &[u8], p: &mut usize| {
        let v = ((b[*p] as u16) << 8) | b[*p + 1] as u16;
        *p += 2;
        v
    };
    let rd_u32 = |b: &[u8], p: &mut usize| {
        let v = u32::from_be_bytes([b[*p], b[*p + 1], b[*p + 2], b[*p + 3]]);
        *p += 4;
        v
    };
    let expect32: [&Vec<u32>; 23] = [
        &im.blk_livein_ptr,
        &im.blk_n_livein,
        &im.blk_cell_ptr,
        &im.blk_n_cells,
        &im.blk_write_ptr,
        &im.blk_n_writes,
        &im.blk_term_kind,
        &im.blk_t0,
        &im.blk_t1,
        &im.blk_top,
        &im.blk_cdst,
        &im.blk_cargc,
        &im.blk_cargp,
        &im.cell_kind,
        &im.cell_code,
        &im.cell_op0,
        &im.cell_op1,
        &im.cell_op2,
        &im.cell_op3,
        &im.wr_reg,
        &im.wr_op,
        &im.arg_op,
        &im.liv_reg,
    ];
    for exp in expect32 {
        let n = rd_u16(&bytes, &mut p) as usize;
        assert_eq!(n, exp.len(), "wire count mismatch");
        for &e in exp.iter() {
            assert_eq!(rd_u32(&bytes, &mut p), e, "wire word mismatch");
        }
    }
    let nk = rd_u16(&bytes, &mut p) as usize;
    assert_eq!(nk, im.konst.len());
    for &k in &im.konst {
        let v = u64::from_be_bytes(bytes[p..p + 8].try_into().unwrap()) as i64;
        p += 8;
        assert_eq!(v, k, "wire konst mismatch");
    }
    let np = rd_u16(&bytes, &mut p) as usize;
    assert_eq!(np, 1);
    let param = u64::from_be_bytes(bytes[p..p + 8].try_into().unwrap()) as i64;
    p += 8;
    assert_eq!(param, 7);
    assert_eq!(bytes[p], 0x5A, "wire must end with the GO marker");
    assert_eq!(p + 1, bytes.len(), "no trailing bytes");
}

#[test]
fn the_universal_engine_runs_recursion_streamed_over_the_wire() {
    let cfg = recursive_call_config();
    let inputs: [i64; 3] = [0, 2, 5];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(m, (0..=n).sum::<i64>(), "recursion model at n={n}");
            m
        })
        .collect();
    if run_over_the_wire("rec", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — RECURSION: streamed f(n)=n+f(n-1)'s image + params over the UART; the chip synthesized it and ran it on its frame stack ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_universal_engine_runs_a_loop_streamed_over_the_wire() {
    // A real LOGOS loop compiled to a multi-block CFG (Branch + Jump + register writes) — the branchy control
    // plane, streamed over the wire.
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [5, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "loop model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire("loop", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — LOOP: streamed sum(1..n)'s branchy image over the UART; the chip ran the block walk ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_universal_engine_runs_collections_streamed_over_the_wire() {
    let src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [7, 20, 5];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "collection model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire("heap", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — COLLECTIONS: streamed [10,x,30]'s image over the UART; the chip ran it on its heap state plane ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_universal_engine_runs_division_streamed_over_the_wire() {
    let src = "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [7, 17, 33];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "division model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire("div", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — DIVISION: streamed x/2+x%3's image over the UART; the chip ran it on its Func units ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_universal_engine_runs_floordiv_and_pow_over_the_wire() {
    // FloorDiv (func code 2, sign-adjusting) + Pow (func code 3, repeat-multiply) — the remaining integer func
    // units, both via the engine's SEQUENTIAL datapath. FloorDiv exercises the negative-operand sign fix.
    for (label, src, inputs) in [
        (
            "floordiv",
            "## To f (x: Int) -> Int:\n    Return x // 4.\n",
            [7i64, 33, -7],
        ),
        (
            "pow",
            "## To f (x: Int) -> Int:\n    Return x to the power of 3.\n",
            [3i64, 7, 10],
        ),
    ] {
        let cfg = serialize_program(&program_cfg_fabric(src, "f"));
        let oracle: Vec<i64> = inputs
            .iter()
            .map(|&x| {
                let m = eval_wprog_config(&cfg, &[x]);
                assert_eq!(m, vm_f(src, x), "{label} model != VM at x={x}");
                m
            })
            .collect();
        if run_over_the_wire(label, &cfg, &inputs, &oracle) {
            println!("OVER THE WIRE — {}: streamed over the UART; the chip ran it on its sequential Func datapath ≡ eval_wprog_config ≡ VM", label.to_uppercase());
        }
    }
}

#[test]
fn the_universal_engine_runs_a_loop_with_division_over_the_wire() {
    // COMPOSITION: a real While loop whose body divides by the DYNAMIC loop variable (acc += n/i). Control plane
    // (Branch/Jump/register writes) + the sequential divider, in one streamed program — the full arithmetic
    // surface composing over the wire, not a single op in isolation.
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + n / i.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [6, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "loop+div model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire("loopdiv", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — LOOP+DIVISION: streamed a loop dividing by the loop variable; the chip ran the block walk AND the sequential divider ≡ eval_wprog_config ≡ VM");
    }
}

// ── The SYNTHESIZABLE netlist (the exact RTL that becomes the Nexys A7 bitstream), proven ≡VM ──────────────
// These run the demo configs through `emit_wprog_engine_soc_verilog_synth` (small caps, ram_style=registers).
// The func units now include a SEQUENTIAL integer datapath (magnitude-restoring divide for Div/Mod/FloorDiv,
// repeat-multiply for Pow) — replacing the yosys-hostile combinational 64-bit divider — so the FULL integer
// arithmetic surface (not just DSP multiplies) runs on the FPGA-bound RTL over the wire. The float lane is the
// FPU hardware follow-on (`wprog_config_fits_synth_engine` still rejects float/popcount func codes).

#[test]
fn the_synth_netlist_runs_division_over_the_wire() {
    // Div (func code 0) + Mod (func code 1) on the FPGA-bound netlist — the sequential divider over the wire.
    let src = "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 4] = [7, 17, 33, 100];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "synth division model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_synth("sdiv", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — DIVISION: the FPGA-bound RTL ran x/2+x%3 streamed over the wire on its sequential divider ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_synth_netlist_runs_floordiv_and_pow_over_the_wire() {
    // FloorDiv (func code 2, sign-adjusting) + Pow (func code 3, repeat-multiply) on the FPGA-bound netlist.
    for (label, src, inputs) in [
        (
            "sfloordiv",
            "## To f (x: Int) -> Int:\n    Return x // 4.\n",
            [7i64, 33, -7],
        ),
        (
            "spow",
            "## To f (x: Int) -> Int:\n    Return x to the power of 3.\n",
            [3i64, 7, 10],
        ),
    ] {
        let cfg = serialize_program(&program_cfg_fabric(src, "f"));
        let oracle: Vec<i64> = inputs
            .iter()
            .map(|&x| {
                let m = eval_wprog_config(&cfg, &[x]);
                assert_eq!(m, vm_f(src, x), "synth {label} model != VM at x={x}");
                m
            })
            .collect();
        if run_over_the_wire_synth(label, &cfg, &inputs, &oracle) {
            println!("SYNTH NETLIST — {}: the FPGA-bound RTL ran it streamed over the wire on its sequential Func datapath ≡ eval_wprog_config ≡ VM", label.to_uppercase());
        }
    }
}

#[test]
fn the_synth_netlist_runs_a_loop_with_division_over_the_wire() {
    // COMPOSITION on the FPGA-bound netlist: a While loop dividing by the dynamic loop variable AND a mod, all
    // streamed over the wire — the control plane + the sequential divider running together on silicon-bound RTL.
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + (n % i) + i / 2.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [6, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "synth loop+mod+div model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_synth("sloopdiv", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — LOOP+MOD+DIVISION: the FPGA-bound RTL ran a loop with (n%i)+i/2 streamed over the wire on its block walk + sequential divider ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_synth_netlist_runs_recursion_over_the_wire() {
    let cfg = recursive_call_config();
    let inputs: [i64; 3] = [0, 3, 6]; // depth <= 7 fits the synth DEPTH=8 frame stack
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| eval_wprog_config(&cfg, &[n]))
        .collect();
    if run_over_the_wire_synth("srec", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — RECURSION: the FPGA-bound RTL ran f(n)=n+f(n-1) streamed over the wire ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_synth_netlist_runs_a_loop_over_the_wire() {
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [5, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "synth loop model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_synth("sloop", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — LOOP: the FPGA-bound RTL ran sum(1..n) streamed over the wire ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_synth_netlist_runs_collections_over_the_wire() {
    let src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [7, 20, 5];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "synth collection model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_synth("sheap", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — COLLECTIONS: the FPGA-bound RTL ran [10,x,30] streamed over the wire ≡ eval_wprog_config ≡ VM");
    }
}

// ── The DEPLOYED hold-safe HW netlist (the exact RTL of the real Nexys A7 bitstream), proven ≡VM ────────────
// These run the demo configs through `emit_wprog_engine_soc_verilog_hw` (the WPE_HW_* cap profile). Recursion
// depth is bounded to the profile's DEPTH=3 (n<=2). This is the netlist that runs loops/branches/writes,
// recursion, small collections, AND now the full integer arithmetic surface (sequential divide/mod/floordiv/
// pow) over the wire on real silicon.

#[test]
fn the_hw_netlist_runs_division_over_the_wire() {
    // The deployed bitstream RTL runs Div (code 0) + Mod (code 1) on its sequential divider, streamed over UART.
    let src = "## To f (x: Int) -> Int:\n    Return x / 2 + x % 3.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [7, 17, 33];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "hw division model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_hw("hwdiv", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — DIVISION: the deployed FPGA RTL ran x/2+x%3 over the wire on its sequential divider ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_hw_netlist_runs_a_loop_with_division_over_the_wire() {
    // COMPOSITION on real silicon RTL: a While loop dividing by the loop variable, streamed over the wire.
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + n / i.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [6, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "hw loop+div model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_hw("hwloopdiv", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — LOOP+DIVISION: the deployed FPGA RTL ran a loop dividing by the loop variable over the wire ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_hw_netlist_runs_a_loop_over_the_wire() {
    let src = "## To f (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Return acc.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [5, 10, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "hw loop model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_hw("hwloop", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — LOOP: the deployed FPGA RTL ran sum(1..n) streamed over the wire ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_hw_netlist_runs_recursion_over_the_wire() {
    let cfg = recursive_call_config();
    let inputs: [i64; 2] = [0, 1]; // depth <= 1 fits the hw DEPTH=2 frame stack
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| eval_wprog_config(&cfg, &[n]))
        .collect();
    if run_over_the_wire_hw("hwrec", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — RECURSION: the deployed FPGA RTL ran f(n)=n+f(n-1) on its frame stack ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_hw_netlist_runs_collections_over_the_wire() {
    let src = "## To f (x: Int) -> Int:\n    Let mutable xs be [10, x, 30].\n    Set item 1 of xs to 99.\n    Return item 1 of xs + item 2 of xs.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [7, 20, 5];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(src, x), "hw collection model != VM at x={x}");
            m
        })
        .collect();
    if run_over_the_wire_hw("hwheap", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — COLLECTIONS: the deployed FPGA RTL ran [10,x,30] on its heap state plane ≡ eval_wprog_config ≡ VM");
    }
}

// ── The FLOAT lane over the wire: the real sequential FPU (fpu.v) runs streamed float programs ≡VM ───────────
// The `pi_leibniz` alternating series returns a scaled+FLOORED Int, so `vm_f` (an Int parse) is a clean oracle
// while the program exercises the full float lane: FAdd/FSub/FMul/FDiv on the sequential IEEE-754 FPU, the
// INT→FLOAT promotion of the loop counter (`2.0 * k`), and the float→int `floor` bridge — all streamed over the
// UART and run on the engine's float datapath ≡ eval_wprog_config ≡ VM.
const PI_LEIBNIZ_SRC: &str = "## To f (n: Int) -> Int:\n    Let mutable sum be 0.0.\n    Let mutable sign be 1.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set sum to sum + sign / (2.0 * k + 1.0).\n        Set sign to 0.0 - sign.\n        Set k to k + 1.\n    Let scaled be sum * 4.0 * 1000000.0.\n    Return floor(scaled).\n";

// The `sqrt` benchmark slice: INT→FLOAT promotion (`n * 1.0`), a sequential `fp_sqrt`, a float multiply, and the
// float→int `floor` — returns a clean Int. And a NEGATIVE-floor slice that exercises `fp_div` + the FFloor sign
// path (floor of a negative non-integer rounds AWAY from zero).
const SQRT_SRC: &str = "## To f (n: Int) -> Int:\n    Let x be n * 1.0.\n    Let r be sqrt(x).\n    Return floor(r * 1000.0).\n";
const NEG_FLOOR_SRC: &str =
    "## To f (n: Int) -> Int:\n    Let x be n * 1.0.\n    Return floor(0.0 - x / 3.0).\n";
// The `mandelbrot` core with the genuine IEEE-754 escape test `zr² + zi² > 4.0` — a FLOAT COMPARISON. Its float
// temps push it past the synth cap profile's register budget, so it is proven on the behavioral engine (the
// compare logic — one `fp_cmp` — is identical across all three netlists).
const MANDEL_SRC: &str = "## To f (px: Int) -> Int:\n    Let cr be 0.01 * px - 2.0.\n    Let ci be 0.5.\n    Let mutable zr be 0.0.\n    Let mutable zi be 0.0.\n    Let mutable i be 0.\n    While i is less than 100:\n        Let zr2 be zr * zr - zi * zi + cr.\n        Let zi2 be 2.0 * zr * zi + ci.\n        Set zr to zr2.\n        Set zi to zi2.\n        If zr * zr + zi * zi is greater than 4.0:\n            Return i.\n        Set i to i + 1.\n    Return i.\n";

fn float_oracle(src: &str, cfg: &WProgConfig, inputs: &[i64]) -> Vec<i64> {
    inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(cfg, &[n]);
            assert_eq!(m, vm_f(src, n), "float model != VM at n={n}");
            m
        })
        .collect()
}

#[test]
fn the_universal_engine_runs_the_float_pi_series_over_the_wire() {
    let cfg = serialize_program(&program_cfg_fabric(PI_LEIBNIZ_SRC, "f"));
    let inputs: [i64; 5] = [0, 1, 2, 5, 10];
    let oracle = float_oracle(PI_LEIBNIZ_SRC, &cfg, &inputs);
    if run_over_the_wire("fpi", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — FLOAT π: streamed the Leibniz series over the UART; the chip ran it on its sequential IEEE-754 FPU ≡ eval_wprog_config ≡ VM (n=10 → {})", oracle[4]);
    }
}

#[test]
fn the_synth_netlist_runs_the_float_pi_series_over_the_wire() {
    let cfg = serialize_program(&program_cfg_fabric(PI_LEIBNIZ_SRC, "f"));
    let inputs: [i64; 4] = [0, 2, 5, 10];
    let oracle = float_oracle(PI_LEIBNIZ_SRC, &cfg, &inputs);
    if run_over_the_wire_synth("sfpi", &cfg, &inputs, &oracle) {
        println!("SYNTH NETLIST — FLOAT π: the FPGA-bound RTL ran the Leibniz series streamed over the wire on its sequential IEEE-754 FPU ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_hw_netlist_runs_the_float_pi_series_over_the_wire() {
    let cfg = serialize_program(&program_cfg_fabric(PI_LEIBNIZ_SRC, "f"));
    let inputs: [i64; 3] = [0, 2, 5];
    let oracle = float_oracle(PI_LEIBNIZ_SRC, &cfg, &inputs);
    if run_over_the_wire_hw("hwfpi", &cfg, &inputs, &oracle) {
        println!("HW NETLIST — FLOAT π: the deployed FPGA RTL ran the Leibniz series over the wire on its sequential IEEE-754 FPU ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_synth_and_hw_netlists_run_float_sqrt_over_the_wire() {
    // FSqrt (sequential fp_sqrt) + IntToF + float multiply + FFloor — on the FPGA-bound netlists.
    let cfg = serialize_program(&program_cfg_fabric(SQRT_SRC, "f"));
    let inputs: [i64; 4] = [0, 2, 10, 100];
    let oracle = float_oracle(SQRT_SRC, &cfg, &inputs);
    run_over_the_wire("fsqrt", &cfg, &inputs, &oracle);
    run_over_the_wire_synth("sfsqrt", &cfg, &inputs, &oracle);
    if run_over_the_wire_hw("hwfsqrt", &cfg, &inputs, &oracle) {
        println!("SYNTH+HW — FLOAT SQRT: floor(sqrt(n)*1000) ran over the wire on the sequential fp_sqrt ≡ eval_wprog_config ≡ VM (n=100 → {})", oracle[3]);
    }
}

#[test]
fn the_synth_and_hw_netlists_run_negative_floor_over_the_wire() {
    // fp_div + the FFloor NEGATIVE path (floor(-2.33)=-3) — the float→int sign edge, over the wire.
    let cfg = serialize_program(&program_cfg_fabric(NEG_FLOOR_SRC, "f"));
    let inputs: [i64; 4] = [0, 7, 9, 20];
    let oracle = float_oracle(NEG_FLOOR_SRC, &cfg, &inputs);
    run_over_the_wire("fnf", &cfg, &inputs, &oracle);
    run_over_the_wire_synth("sfnf", &cfg, &inputs, &oracle);
    if run_over_the_wire_hw("hwfnf", &cfg, &inputs, &oracle) {
        println!("SYNTH+HW — NEGATIVE FLOOR: floor(-n/3) ran over the wire ≡ eval_wprog_config ≡ VM (n=20 → {})", oracle[3]);
    }
}

#[test]
fn the_universal_engine_runs_the_float_mandelbrot_compare_over_the_wire() {
    // The IEEE-754 escape test `zr² + zi² > 4.0` — the float COMPARE lane (fp_cmp), streamed over the wire.
    let cfg = serialize_program(&program_cfg_fabric(MANDEL_SRC, "f"));
    let inputs: [i64; 3] = [0, 100, 250];
    let oracle = float_oracle(MANDEL_SRC, &cfg, &inputs);
    if run_over_the_wire("fmand", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — FLOAT COMPARE (mandelbrot): the escape test zr²+zi²>4.0 ran on fp_cmp over the wire ≡ eval_wprog_config ≡ VM");
    }
}

#[test]
fn the_universal_engine_runs_all_float_relations_over_the_wire() {
    // Every surface-reachable float relation in ONE program — FLt(14) `<`, FGt(15) `>`, FLe(16) `<=`, FGe(17)
    // `>=`, FEq(18) `==` — each folded into a result bit, exercised with a<b (n=3 → 1+4=5), a==b (n=5 →
    // 4+8+16=28), a>b (n=7 → 2+8=10). One fp_cmp, five distinct st==7 derivations, all streamed over the wire ≡VM.
    let src = "## To f (n: Int) -> Int:\n    Let a be n * 1.0.\n    Let b be 5.0.\n    Let mutable r be 0.\n    If a is less than b:\n        Set r to r + 1.\n    If a is greater than b:\n        Set r to r + 2.\n    If a is less than or equal to b:\n        Set r to r + 4.\n    If a is greater than or equal to b:\n        Set r to r + 8.\n    If a is equal to b:\n        Set r to r + 16.\n    Return r.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [3, 5, 7];
    let oracle = float_oracle(src, &cfg, &inputs);
    assert_eq!(
        oracle,
        vec![5, 28, 10],
        "the five float relations must fold to 5/28/10 for a<b / a==b / a>b"
    );
    if run_over_the_wire("frel", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — ALL FLOAT RELATIONS: </≤/==/≥/> all ran on the one fp_cmp over the wire ≡ eval_wprog_config ≡ VM (n=3/5/7 → 5/28/10)");
    }
}

#[test]
fn the_universal_engine_runs_a_float_seq_over_the_wire() {
    // A `Seq of Float`: the float PAYLOADS ride in the engine's i64 heap as f64 bit patterns (same HeapRead/
    // HeapWrite cells as an int seq — no separate float plane), while the FPU produces/consumes them. Builds,
    // mutates, and sums a float array, then floors. Its float temps exceed the synth cap profile's registers, so
    // it is proven on the behavioral engine (the heap + FPU mechanisms are identical across all three netlists).
    let src = "## To f (n: Int) -> Int:\n    Let mutable xs be [1.5, 2.5, 3.5].\n    Set item 1 of xs to n * 1.0.\n    Let mutable s be 0.0.\n    Let mutable i be 1.\n    While i is less than or equal to 3:\n        Set s to s + item i of xs.\n        Set i to i + 1.\n    Return floor(s * 100.0).\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let inputs: [i64; 3] = [0, 5, 10];
    let oracle = float_oracle(src, &cfg, &inputs);
    if run_over_the_wire("fseq", &cfg, &inputs, &oracle) {
        println!("OVER THE WIRE — FLOAT SEQ: a Seq of Float rode in the i64 heap as bit patterns while the FPU summed it ≡ eval_wprog_config ≡ VM (n=10 → {})", oracle[2]);
    }
}

// ── The synthesizability guard: the FPGA-bound netlist MUST map with NO combinational divider ────────────────
// The whole reason Div/Mod/FloorDiv/Pow run on a SEQUENTIAL datapath (template st==6) is that a combinational
// 64-bit `a/x` is precisely what the open synthesis flow cannot map in bounded time. This lock proves that with
// the divider programs proven ≡VM above (`the_synth_netlist_runs_division_*`), the FPGA-bound RTL still
// synthesizes and its netlist contains NO `$div`/`$mod`/`$divfloor`/`$modfloor`/`$pow` cell — so a combinational
// divider can never regress back onto the real bitstream while the ≡VM division tests stay green. Uses a coarse
// yosys pass (`proc; opt`) at which a combinational divide would already appear as a `$div` cell (fast, ~8s).

fn yosys_coarse_netlist(label: &str, rtl: &str) -> String {
    let dir = std::env::temp_dir().join("logos_wpe_synth_lock");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let v = dir.join(format!("wpe_soc_{label}.v"));
    std::fs::write(&v, rtl).expect("write rtl");
    let out = std::process::Command::new("yosys")
        .arg("-p")
        .arg(format!(
            "read_verilog -sv {}; hierarchy -top wpe_soc_probe; proc; opt; stat",
            v.display()
        ))
        .output()
        .expect("run yosys");
    assert!(
        out.status.success(),
        "yosys failed to synthesize the {label} engine:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_fpga_bound_engines_synthesize_with_no_combinational_divider() {
    if !common::require_tool("yosys") {
        return;
    }
    for (label, rtl) in [
        ("synth", emit_wprog_engine_soc_verilog_synth("probe")),
        ("hw", emit_wprog_engine_soc_verilog_hw("probe")),
    ] {
        // The FLOAT lane must be wired in — the sequential FPU units are instantiated in the engine (a future
        // change can't silently drop the float datapath while the float ≡VM tests above keep passing behaviorally).
        for unit in ["fp_addsub", "fp_mul", "fp_div", "fp_sqrt", "fp_cmp"] {
            assert!(
                rtl.contains(unit),
                "{label}: the emitted engine is missing the {unit} FPU unit"
            );
        }
        let log = yosys_coarse_netlist(label, &rtl);
        // The combinational INTEGER divider/pow — the cells the open flow can't map — must be ABSENT. The float
        // divider (fp_div) is a SEQUENTIAL digit-recurrence FSM, so it too maps with no `$div` cell: this asserts
        // BOTH the integer datapath (st==6) and the FPU stay off any combinational divider.
        for forbidden in ["$div", "$mod", "$divfloor", "$modfloor", "$pow"] {
            assert!(
                !log.contains(forbidden),
                "the {label} engine inferred a combinational {forbidden} cell — a divider regressed off its sequential datapath (integer st==6 or the fp_div FSM)"
            );
        }
        // Non-vacuity: yosys really elaborated the engine's datapath (a netlist with the DSP multiplies present),
        // so the absence of `$div` is a genuine synthesis result, not a parse/elaboration miss.
        assert!(
            log.contains("Number of cells:"),
            "{label}: yosys reported no netlist"
        );
        assert!(
            log.contains("$mul"),
            "{label}: expected the DSP multiplies in the netlist — did yosys elaborate the datapath?"
        );
        println!("SYNTHESIZABILITY GUARD — {}: yosys mapped the FPGA-bound engine (integer datapath + IEEE-754 FPU) with NO combinational divider (sequential datapaths only)", label.to_uppercase());
    }
}

/// A full `synth_xilinx` netlist (does the memory→BRAM mapping the coarse `proc;opt` pass skips) — for the
/// BRAM/FF budget guard below. Slower (~40s) but it is the only pass that reveals RAMB inference + the real FF
/// count. `-flatten` folds the FPU submodules into the top so one `stat` block has the whole design's counts.
fn yosys_synth_xilinx_stat(label: &str, rtl: &str, top: &str) -> String {
    let dir = std::env::temp_dir().join("logos_wpe_bram_lock");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let v = dir.join(format!("wpe_soc_{label}.v"));
    std::fs::write(&v, rtl).expect("write rtl");
    let out = std::process::Command::new("yosys")
        .arg("-p")
        // `chparam WITH_FPU 0`: measure the DEPLOYED config (nexys_wpe_soc instantiates the engine with
        // WITH_FPU=0 — the float lane is excluded on this board). The bare module defaults WITH_FPU=1, whose FPU
        // adds ~1.5k FF that never ships; the budget guard must reflect what actually holds on silicon.
        .arg(format!(
            "read_verilog -sv {}; chparam -set WITH_FPU 0 {top}; synth_xilinx -family xc7 -nodsp -flatten -top {top}; stat",
            v.display()
        ))
        .output()
        .expect("run yosys");
    assert!(
        out.status.success(),
        "yosys synth_xilinx failed for the {label} engine:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Count a primitive in a yosys `stat` table (`   CELLNAME   <n>`), taking the max over every stat block so a
/// flattened top's full count wins. Exact first-token match so `RAMB36E1` never counts `RAMB36E1_something`.
fn count_prim(stat: &str, cell: &str) -> usize {
    stat.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            match (it.next(), it.next()) {
                (Some(name), Some(n)) if name == cell => n.parse::<usize>().ok(),
                _ => None,
            }
        })
        .max()
        .unwrap_or(0)
}

/// PIT-OF-SUCCESS lock for the BRAM redesign: the big (`WPE_SYNTH_*` = HEAP=64/DEPTH=8) engine — the full
/// collection/deep-recursion corpus — must map `heap` + `sfile` to BLOCK RAM and stay UNDER the board's hold-safe
/// FF threshold. Before the redesign it was ~23k FF (heap/sfile in flip-flops) → past ~14.3k → SILENT on silicon.
/// This locks the invariant three ways: (1) the emitted RTL tags heap/sfile `ram_style="block"`; (2) yosys
/// actually infers ≥2 RAMB from them (a dropped tag or a combinational read that defeats inference → 0-1 RAMB →
/// RED); (3) the FF count stays under ~14k (a config memory silently landing in flip-flops, or heap/sfile falling
/// back, blows the budget → RED). So the redesign can never silently regress back to a hold-unsafe netlist.
#[test]
fn the_big_engine_fits_the_hold_safe_ff_budget_via_block_ram() {
    if !common::require_tool("yosys") {
        return;
    }
    let rtl = emit_wprog_engine_soc_verilog_synth("probe");
    // (1) The emitter must tag the state plane + frame stack as block RAM (not the default `registers`).
    // The heap is SPLIT into two 32-bit halves (`*_lo`/`*_hi`) so it never maps to a single 64-bit RAMB36 in
    // SDP-72 mode (whose parity-lane write yosys 0.33 mis-wires — reproduced on real Artix-7 silicon: negative
    // weights read back with their high word corrupted to a constant parity-spill mask) AND depth-banked into two
    // 16384-deep banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address bit 14) so it never maps to a RAMB36E1
    // in ×1-width, 32768-deep mode — the internal two-16K-half depth-cascade that prjxray/openXC7 mis-encodes on
    // the XC7A200T (correct at gate level, wrong on silicon). Every half of every bank must still be block RAM.
    for half in ["heap_b0_lo", "heap_b0_hi", "heap_b1_lo", "heap_b1_hi"] {
        assert!(
            rtl.contains(&format!("(* ram_style = \"block\" *) reg [31:0] {half}")),
            "the state plane heap bank `{half}` must be tagged block RAM in the emitted engine"
        );
    }
    // The recursion frame stack `sfile` is SPLIT into two 32-bit halves (`sfile_lo`/`sfile_hi`) so it never maps
    // to a single 64-bit RAMB36 in SDP-72 mode (whose parity-lane write yosys 0.33 mis-wires — gate-level
    // reproduced). Both halves must still be tagged block RAM (state plane).
    assert!(
        rtl.contains("(* ram_style = \"block\" *) reg [31:0] sfile_lo"),
        "the recursion frame stack `sfile_lo` half must be tagged block RAM in the emitted engine"
    );
    assert!(
        rtl.contains("(* ram_style = \"block\" *) reg [31:0] sfile_hi"),
        "the recursion frame stack `sfile_hi` half must be tagged block RAM in the emitted engine"
    );
    let stat = yosys_synth_xilinx_stat("bram_budget", &rtl, "wpe_soc_probe");
    assert!(
        stat.contains("Number of cells:"),
        "yosys reported no netlist for the BRAM budget guard"
    );
    let ramb = count_prim(&stat, "RAMB36E1") + count_prim(&stat, "RAMB18E1");
    let ff = count_prim(&stat, "FDRE") + count_prim(&stat, "FDSE") + count_prim(&stat, "FDCE");
    // (2) heap + sfile actually became block RAM — not flip-flops.
    assert!(
        ramb >= 2,
        "the big engine mapped only {ramb} block RAMs — heap/sfile regressed to flip-flops (a lost ram_style tag or a combinational read defeating RAMB inference)"
    );
    // (3) the FF count stays under the board's hold-safe silent threshold (~14.3k). ~11.8k with the BRAM redesign;
    // was ~23k before it. A regression that pushes state back into flip-flops fails HERE, loudly.
    assert!(
        ff > 4000 && ff < 14000,
        "the big engine synthesized to {ff} flip-flops — must stay in (4000, 14000): under the ~14.3k hold-safe threshold, and non-vacuously elaborated"
    );
    println!(
        "BRAM/FF BUDGET GUARD: the big engine (HEAP=64/DEPTH=8) → {ff} FF + {ramb} block RAM — heap+sfile off flip-flops, under the ~14k hold-safe threshold"
    );
}

/// A within-block SWAP (`Set b to …; Set a to temp` where temp = old b) must apply the block's register writes
/// against the BLOCK-ENTRY state (atomically, as `eval_wprog_config` does), not sequentially read-after-write —
/// else the second write reads the first's new value. This is the fib_iterative divergence (RTL returned
/// 2^(n-1) instead of fib(n)). Runs the config through the engine RTL under iverilog vs the eval oracle.
#[test]
fn within_block_swap_writes_read_the_block_entry_snapshot() {
    let src = "## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 0.\n    While i is less than n:\n        Let temp be b.\n        Set b to (a + b) % 1000000007.\n        Set a to temp.\n        Set i to i + 1.\n    Return a.\n";
    let cfg = serialize_program(&program_cfg_fabric(src, "f"));
    let ns = [5i64, 10, 15, 20];
    let oracle: Vec<i64> = ns.iter().map(|&n| eval_wprog_config(&cfg, &[n])).collect();
    // eval oracle is fib: [5,55,610,6765]
    assert_eq!(oracle, vec![5, 55, 610, 6765], "eval oracle must be fib(n)");
    run_over_the_wire_hw("swapfib", &cfg, &ns, &oracle);
}
