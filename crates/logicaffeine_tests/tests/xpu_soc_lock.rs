//! xpu_soc_lock — Stage 6 of the self-synthesizing bytecode XPU: the north star. A program's bytecode is
//! streamed OVER THE WIRE into the SoC byte-by-byte, parsed on-chip, and RUN on the reconfigurable execute
//! engine — the SoC's `result` must equal the VM's, for a corpus of programs across a battery of inputs. No
//! host help beyond the bytes on the wire: the chip receives the bytecode and computes the answer.
//!
//! Each case serializes a program's decoded block + one input vector into the wire byte frame
//! (`block_stream_bytes`), the testbench clocks the frame in, and the SoC assembles + runs it. Every frame
//! ≡ the VM ⇒ "send the bytecode down the wire, the chip runs it" holds in simulation.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(xpu_soc_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::block_engine_rtl::serialize_block;
use logicaffeine_compile::codegen_sva::fabric_rtl::{residual_from_source, FabricOp};
use logicaffeine_compile::codegen_sva::xpu_soc_rtl::{
    block_stream_bytes, emit_xpu_soc_testbench, emit_xpu_soc_verilog,
};
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
    Prog {
        name: "horner",
        src: "## To horner (x: Int) -> Int:\n    Return ((2 * x + 3) * x + 4) * x + 5.\n",
        args: &[&[0], &[1], &[2], &[-1], &[-3], &[10]],
    },
];

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
fn bytecode_streamed_over_the_wire_runs_on_the_xpu_soc_matching_the_vm() {
    if !common::require_tool("iverilog") {
        return;
    }

    let residuals: Vec<(Vec<FabricOp>, usize, usize)> = CORPUS
        .iter()
        .map(|p| residual_from_source(p.src, p.name))
        .collect();
    let k = residuals.iter().map(|(ops, _, _)| ops.len()).max().unwrap();
    let n_in = residuals.iter().map(|(_, _, n)| *n).max().unwrap();
    let frame_len = k * 20 + 2 + n_in * 8;
    let n_cases: usize = CORPUS.iter().map(|p| p.args.len()).sum();

    let dir = std::env::temp_dir().join("logos_xpu_soc_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let rtl = emit_xpu_soc_verilog("core", k, n_in);
    let tb = emit_xpu_soc_testbench("core", n_cases, frame_len);
    let srcv = dir.join("xpu_core.v");
    let tbv = dir.join("tb_xpu_core.v");
    let sim = dir.join("sim_xpu.vvp");
    std::fs::write(&srcv, &rtl).expect("write soc rtl");
    std::fs::write(&tbv, &tb).expect("write soc tb");
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
        "iverilog failed to compile the XPU SoC: {}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );

    // Build every case's wire frame (config + one input vector) + the VM oracle, concatenated.
    let mut frames = String::new();
    let mut eout = String::new();
    for (p, (ops, result, actual_n_in)) in CORPUS.iter().zip(residuals.iter()) {
        let cfg = serialize_block(ops, *result, k);
        for args in p.args {
            assert_eq!(
                args.len(),
                *actual_n_in,
                "{}: vector arity mismatch",
                p.name
            );
            let bytes = block_stream_bytes(&cfg, args, k, n_in);
            assert_eq!(bytes.len(), frame_len, "{}: frame length mismatch", p.name);
            for b in bytes {
                frames.push_str(&format!("{b:02x}\n"));
            }
            eout.push_str(&format!("{:016x}\n", vm_call(p.src, p.name, args) as u64));
        }
    }
    std::fs::write(dir.join("xpu_core_frames.hex"), frames).unwrap();
    std::fs::write(dir.join("xpu_core_eout.hex"), eout).unwrap();

    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "XPU SoC != VM streaming bytecode: {stdout}\n--- RTL ---\n{rtl}"
    );

    println!(
        "Stage 6: {n_cases} bytecode frames ({} programs) streamed byte-by-byte over the wire into the XPU SoC — parsed on-chip + run on the reconfigurable engine — all ≡ the VM under iverilog. THE NORTH STAR: send the bytecode down the wire, the chip runs it.",
        CORPUS.len()
    );
}
