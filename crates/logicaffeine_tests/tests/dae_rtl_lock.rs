//! dae_rtl_lock — the decoupled AGU/exec datapath's EMITTED Verilog runs ≡ eval_dae under iverilog.
//!
//! `dae_lock` proves the decoupling semantics-preserving + depth-shrinking at the model level. This closes
//! model→silicon: `emit_dae_verilog` emits the AGU lane + exec lane (with loads against a heap) as a
//! combinational module, and iverilog confirms it computes exactly what `eval_dae` (≡ the coupled body ≡ the
//! VM) does — for streaming reads with computed addresses (the canonical DAE case). Skipped gracefully where
//! iverilog is absent.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(dae_rtl_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::dae::{
    decouple_access, emit_dae_stateful_verilog, emit_dae_testbench, emit_dae_verilog, eval_dae,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_state_plane_testbench, AluOp, LoopCell, Src,
};

const DEPTH: usize = 256;

struct Body {
    name: &'static str,
    n_in: usize,
    cells: Vec<LoopCell>,
    result: Src,
}

fn arith(op: AluOp, a: Src, b: Src) -> LoopCell {
    LoopCell::Arith { op, a, b }
}

/// Load-only bodies: streaming reads with computed addresses (`addr_heavy`) and a pointer chase (`ptr_chase`).
fn corpus() -> Vec<Body> {
    use AluOp::*;
    use Src::{In, Val};
    vec![
        Body {
            name: "addr_heavy",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)),            // v0 = p + i     (AGU)
                arith(Mul, Val(0), In(2)),           // v1 = (p+i)*k   (AGU, deep address)
                LoopCell::HeapRead { addr: Val(1) }, // v2 = heap[v1]
                arith(Add, Val(2), In(1)),           // v3 = load + i  (exec)
            ],
            result: Val(3),
        },
        Body {
            name: "ptr_chase",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)),            // v0 = p + i     (AGU)
                LoopCell::HeapRead { addr: Val(0) }, // v1 = heap[v0]
                arith(Add, Val(1), In(2)),           // v2 = load + j  (tainted address, stays exec)
                LoopCell::HeapRead { addr: Val(2) }, // v3 = heap[v2]  (0 when out of range)
            ],
            result: Val(3),
        },
    ]
}

#[test]
fn the_emitted_decoupled_datapath_matches_eval_dae_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_dae_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // A deterministic read-only heap: heap[a] = 3a + 5 (distinct, non-negative, some out of index range).
    let seed: Vec<i64> = (0..DEPTH).map(|a| 3 * a as i64 + 5).collect();
    let heaphex: String = seed
        .iter()
        .map(|v| format!("{:016x}\n", *v as u64))
        .collect();

    for b in corpus() {
        let sched = decouple_access(&b.cells, b.result, b.n_in);
        assert!(
            sched.agu_len >= 1,
            "{}: the RTL corpus must decouple an address lane",
            b.name
        );

        // Inputs kept small so the primary addresses land in range; the pointer-chase second load exercises the
        // out-of-range guard (reads 0 in both the model and the RTL).
        let mut vectors: Vec<[i64; 3]> = Vec::new();
        for p in 0..4 {
            for i in 0..4 {
                for k in 0..3 {
                    vectors.push([p, i, k]);
                }
            }
        }

        let (mut inhex, mut outhex) = (String::new(), String::new());
        for v in &vectors {
            let inp = &v[..b.n_in];
            for a in inp {
                inhex.push_str(&format!("{:016x}\n", *a as u64));
            }
            let mut heap = seed.clone();
            let got = eval_dae(&sched, inp, &mut heap);
            outhex.push_str(&format!("{:016x}\n", got as u64));
        }

        let rtl = emit_dae_verilog(b.name, &sched, DEPTH);
        let tb = emit_dae_testbench(b.name, b.n_in, vectors.len());
        let srcv = dir.join(format!("dae_{}.v", b.name));
        let tbv = dir.join(format!("tb_dae_{}.v", b.name));
        let sim = dir.join(format!("sim_dae_{}.vvp", b.name));
        std::fs::write(&srcv, &rtl).unwrap();
        std::fs::write(&tbv, &tb).unwrap();
        std::fs::write(dir.join(format!("dae_{}_in.hex", b.name)), &inhex).unwrap();
        std::fs::write(dir.join(format!("dae_{}_out.hex", b.name)), &outhex).unwrap();
        std::fs::write(dir.join(format!("dae_{}_heap.hex", b.name)), &heaphex).unwrap();

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
            "iverilog failed to compile dae_{}: {}\n--- RTL ---\n{rtl}",
            b.name,
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
            "dae_{} RTL != eval_dae under iverilog: {stdout}\n--- RTL ---\n{rtl}",
            b.name
        );
    }

    println!("DAE RTL: the emitted AGU/exec decoupled datapath ≡ eval_dae (≡ the coupled body ≡ VM) under iverilog");
}

/// Decoupled schedules that MUTATE the heap (alloc + writes), run on the sequenced state-plane realization.
fn write_corpus() -> Vec<Body> {
    use AluOp::*;
    use Src::{Const as C, In, Val};
    vec![
        // Allocate 3 words, write both inputs at computed offsets, read them back, sum.
        Body {
            name: "alloc_write_read",
            n_in: 2,
            cells: vec![
                LoopCell::HeapAlloc { size: C(3) }, // v0 = ptr
                LoopCell::HeapWrite {
                    addr: Val(0),
                    val: In(0),
                }, // heap[ptr] = a
                arith(Add, Val(0), C(1)),           // v2 = ptr+1
                LoopCell::HeapWrite {
                    addr: Val(2),
                    val: In(1),
                }, // heap[ptr+1] = b
                LoopCell::HeapRead { addr: Val(0) }, // v4 = a
                LoopCell::HeapRead { addr: Val(2) }, // v5 = b
                arith(Add, Val(4), Val(5)),         // v6 = a+b
            ],
            result: Val(6),
        },
        // A write to a PURE-INPUT address (base+i) — the write address decouples into the AGU lane.
        Body {
            name: "decoupled_write_addr",
            n_in: 3,
            cells: vec![
                arith(Add, In(0), In(1)), // v0 = base+i (AGU-hoistable write address)
                LoopCell::HeapWrite {
                    addr: Val(0),
                    val: In(2),
                }, // heap[base+i] = val
                LoopCell::HeapRead { addr: Val(0) }, // read it back
            ],
            result: Val(2),
        },
    ]
}

#[test]
fn the_emitted_dae_write_alloc_datapath_matches_eval_dae_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_dae_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    for b in write_corpus() {
        let sched = decouple_access(&b.cells, b.result, b.n_in);

        // Small non-negative inputs so writes land in the modeled heap window.
        let mut vectors: Vec<[i64; 3]> = Vec::new();
        for x in 0..5 {
            for y in 0..5 {
                for z in 0..4 {
                    vectors.push([x, y, z]);
                }
            }
        }

        let (mut inhex, mut outhex) = (String::new(), String::new());
        for v in &vectors {
            let inp = &v[..b.n_in];
            for a in inp {
                inhex.push_str(&format!("{:016x}\n", *a as u64));
            }
            // The DAE model runs on a FRESH heap (the sequencer starts empty + allocs its own).
            let mut heap: Vec<i64> = Vec::new();
            outhex.push_str(&format!(
                "{:016x}\n",
                eval_dae(&sched, inp, &mut heap) as u64
            ));
        }

        let rtl = emit_dae_stateful_verilog(b.name, &sched, DEPTH);
        let tb =
            emit_state_plane_testbench(b.name, b.n_in, vectors.len(), sched.layout.len() * 4 + 24);
        let srcv = dir.join(format!("state_{}.v", b.name));
        let tbv = dir.join(format!("tb_state_{}.v", b.name));
        let sim = dir.join(format!("sim_state_{}.vvp", b.name));
        std::fs::write(&srcv, &rtl).unwrap();
        std::fs::write(&tbv, &tb).unwrap();
        std::fs::write(dir.join(format!("state_{}_in.hex", b.name)), &inhex).unwrap();
        std::fs::write(dir.join(format!("state_{}_out.hex", b.name)), &outhex).unwrap();

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
            "iverilog failed to compile state_{}: {}\n--- RTL ---\n{rtl}",
            b.name,
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
            "state_{} (DAE write/alloc) RTL != eval_dae under iverilog: {stdout}\n--- RTL ---\n{rtl}",
            b.name
        );
    }
    println!("DAE RTL: the decoupled write/alloc schedule ≡ eval_dae on the sequenced state-plane under iverilog");
}
