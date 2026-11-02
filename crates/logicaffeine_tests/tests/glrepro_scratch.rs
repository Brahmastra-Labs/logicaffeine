//! glrepro_scratch — SCRATCH harness (not a lock). Emits the current ConfigBram engine RTL + small custom
//! programs' wire bytes + oracle to /tmp/glrepro/step2 so an external yosys+iverilog gate-level flow can hunt
//! the residual FPGA synth-vs-sim divergence. Run explicitly:
//!   cargo test -p logicaffeine-tests --test glrepro_scratch -- --nocapture emit_
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog_backend,
    eval_wprog_config_counted_heap_preloaded, wprog_config_image, MemBackend, WBlock, WCell,
    WProgConfig, WSrc, WTerm, WPE_STREAM_CAPS,
};

const CLKS: usize = 8;

fn k(c: i64) -> WSrc {
    WSrc {
        mode: 1,
        reference: 0,
        konst: c,
    }
}
fn inp(r: u16) -> WSrc {
    WSrc {
        mode: 0,
        reference: r,
        konst: 0,
    }
}
fn val(j: u16) -> WSrc {
    WSrc {
        mode: 2,
        reference: j,
        konst: 0,
    }
}
fn nop() -> WSrc {
    k(0)
}

fn arith(code: u8, a: WSrc, b: WSrc) -> WCell {
    WCell {
        kind: 0,
        code,
        ops: [a, b, nop(), nop()],
    }
}
fn heap_write(idx: WSrc, v: WSrc) -> WCell {
    WCell {
        kind: 4,
        code: 0,
        ops: [idx, v, nop(), nop()],
    }
}
fn heap_read(idx: WSrc) -> WCell {
    WCell {
        kind: 3,
        code: 0,
        ops: [idx, nop(), nop(), nop()],
    }
}
fn func(code: u8, a: WSrc, b: WSrc) -> WCell {
    WCell {
        kind: 2,
        code,
        ops: [a, b, nop(), nop()],
    }
}

/// The shared step2 dir. ALL small programs stream into the SAME `wpe_soc_cb` netlist (the engine is universal:
/// the program is streamed over UART), so we synth that ONE netlist once and reuse it for every program.
const S2: &str = "/tmp/glrepro/step2";

/// Emit one program's wire bytes + lengths + oracle under a per-program basename into the shared step2 dir, and
/// a per-program testbench that instantiates the shared `wpe_soc_cb` module but reads THIS program's hex files.
/// Prints the oracle so the external gate-level run can be checked by eye too.
fn emit(tag: &str, cfg: &WProgConfig, inputs: &[i64], heap_init: &[i64]) {
    let dir = std::path::PathBuf::from(S2);
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image(cfg);
    let (mut all, mut lens) = (Vec::<u8>::new(), Vec::<u32>::new());
    let mut oracle = Vec::<i64>::new();
    for &x in inputs {
        let b = im.wire_bytes_preloaded(&[x], heap_init);
        lens.push(b.len() as u32);
        all.extend_from_slice(&b);
        let o = eval_wprog_config_counted_heap_preloaded(cfg, &[x], heap_init).0;
        oracle.push(o);
    }
    std::fs::write(
        dir.join(format!("wire_{tag}.hex")),
        all.iter().map(|b| format!("{b:02x}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("lens_{tag}.hex")),
        lens.iter()
            .map(|n| format!("{n:08x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{tag}.hex")),
        oracle
            .iter()
            .map(|&x| format!("{:016x}\n", x as u64))
            .collect::<String>(),
    )
    .unwrap();
    // A tb that instantiates the shared cb netlist but reads THIS program's hex (template uses __NAME__ for both
    // the module name and the hex basenames, so emit with __NAME__ = "cb" then rewrite the hex basenames to tag).
    let tb = emit_wprog_engine_soc_testbench("cb", inputs.len(), CLKS, all.len())
        .replace("wire_cb.hex", &format!("wire_{tag}.hex"))
        .replace("lens_cb.hex", &format!("lens_{tag}.hex"))
        .replace("oracle_cb.hex", &format!("oracle_{tag}.hex"))
        .replace(
            "module tb_wpe_soc_cb;",
            &format!("module tb_wpe_soc_{tag};"),
        );
    std::fs::write(dir.join(format!("tb_{tag}.v")), &tb).unwrap();
    println!("EMIT {tag}: inputs={inputs:?} heap_init={heap_init:?} oracle={oracle:?}");
}

/// STEP 1: dump the current ConfigBram RTL alone so yosys can synth it and we can grep the netlist for
/// konst/wbuf_val/param_buf mapping + any residual wide RAMB.
#[test]
fn emit_configbram_engine_rtl() {
    let soc = emit_wprog_engine_soc_verilog_backend("cb", WPE_STREAM_CAPS, MemBackend::ConfigBram);
    std::fs::create_dir_all("/tmp/glrepro/step1").unwrap();
    std::fs::create_dir_all(S2).unwrap();
    std::fs::write("/tmp/glrepro/step1/wpe_soc_cb.v", &soc).unwrap();
    std::fs::write(format!("{S2}/wpe_soc_cb.v"), &soc).unwrap();
    // Print the ram_style verdict the emitter chose (sim no-op, but it drives yosys inference).
    for mem in [
        "konst",
        "wbuf_val",
        "param_buf",
        "wbuf_tgt",
        "file_lo",
        "file_hi",
        "v_lo",
        "v_hi",
        "heap_b0_lo",
        "heap_b0_hi",
        "heap_b1_lo",
        "heap_b1_hi",
        "sfile_lo",
        "sfile_hi",
        "liv_reg",
        "arg_op",
    ] {
        let needle = format!(" {mem} ");
        let line = soc
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .find(|l| l.contains(&needle) && l.contains("[0:"));
        let style = match line {
            Some(l) if l.contains("\"block\"") => "block",
            Some(l) if l.contains("\"registers\"") => "registers",
            Some(_) => "NONE",
            None => "??",
        };
        println!("RAMSTYLE {mem} = {style}");
    }
    println!("WROTE /tmp/glrepro/step1/wpe_soc_cb.v");
}

/// A: negative-heavy multiply chain (int8 MAC surface): result = a*b + c*d, mixing signs. Single block, Return.
/// Exercises the -nodsp LUT multiply on negative operands.
#[test]
fn emit_neg_mul_mac() {
    // Cells: v0 = in0 * in1 ; v1 = in2 * in3 ; v2 = v0 + v1 ; Return v2
    // Use params 0..3, but the wire only streams ONE param (the input x). So encode operands as konsts to sweep
    // sign combos deterministically, and let the input pick which combo via a Select — simpler: bake several
    // programs, each with fixed konst operands, input unused. We sweep via multiple tags below instead.
    let combos: [(i64, i64, i64, i64); 6] = [
        (5, 7, 3, 2),
        (-5, 7, 3, 2),
        (5, -7, -3, 2),
        (-5, -7, -3, -2),
        (-127, 127, -128, 100),
        (1_000_000, -1_000_000, -3, 7),
    ];
    for (i, (a, b, c, d)) in combos.iter().enumerate() {
        let blk = WBlock {
            livein: vec![0],
            cells: vec![
                arith(2, k(*a), k(*b)),   // v0 = a*b
                arith(2, k(*c), k(*d)),   // v1 = c*d
                arith(0, val(0), val(1)), // v2 = v0 + v1
            ],
            writes: vec![],
            term: WTerm::Return(val(2)),
        };
        let cfg = WProgConfig {
            n_params: 1,
            blocks: vec![blk],
        };
        emit(&format!("negmul{i}"), &cfg, &[0], &[]);
    }
}

/// B: signed division / mod / floordiv with negative operands — the sequential restoring divider (st6).
#[test]
fn emit_signed_div() {
    // codes into func cell kind=2: 0 Div, 1 Mod, 2 FloorDiv.  Sweep sign combos.
    let combos: [(i64, i64); 8] = [
        (7, 2),
        (-7, 2),
        (7, -2),
        (-7, -2),
        (100, 7),
        (-100, 7),
        (i64::MIN + 5, -3),
        (123456789, -777),
    ];
    for (i, (a, b)) in combos.iter().enumerate() {
        let blk = WBlock {
            livein: vec![0],
            cells: vec![
                func(0, k(*a), k(*b)),         // v0 = a / b
                func(1, k(*a), k(*b)),         // v1 = a % b
                func(2, k(*a), k(*b)),         // v2 = a floordiv b
                arith(0, val(0), arith_tmp()), // placeholder, replaced below
            ],
            writes: vec![],
            term: WTerm::Return(val(0)),
        };
        // We actually want three separate returns; simpler: emit three tags per combo.
        let _ = blk;
        for (which, code) in [("div", 0u8), ("mod", 1u8), ("fdiv", 2u8)] {
            let b1 = WBlock {
                livein: vec![0],
                cells: vec![func(code, k(*a), k(*b))],
                writes: vec![],
                term: WTerm::Return(val(0)),
            };
            let cfg = WProgConfig {
                n_params: 1,
                blocks: vec![b1],
            };
            emit(&format!("{which}{i}"), &cfg, &[0], &[]);
        }
    }
}
fn arith_tmp() -> WSrc {
    k(0)
}

/// C: heap write-then-read of NEGATIVE and large 64-bit values (the heap _lo/_hi split path + the write-buffer).
#[test]
fn emit_heap_negwords() {
    // Write several distinct words (incl negatives / high-word-0xFFFFFFFF) to the heap, then read them back and
    // sum — exercising the depth-banked heap (heap_b*_lo/hi) round-trip on negative and >32-bit magnitudes.
    let words: [i64; 6] = [
        0x0123_4567_89AB_CDEF,
        -1,
        -256,
        0x0000_0001_0000_0000, // high word 1, low word 0
        i64::MIN + 1,
        0xFFFF_FFFF_0000_0001u64 as i64,
    ];
    let mut cells = Vec::new();
    // write words[i] to heap[i]
    for (i, w) in words.iter().enumerate() {
        cells.push(heap_write(k(i as i64), k(*w)));
    }
    // read them back into v[6..12]
    let base = cells.len();
    for i in 0..words.len() {
        cells.push(heap_read(k(i as i64)));
    }
    // sum v[base..base+6] pairwise: acc chain
    let mut acc = val((base) as u16);
    let mut sum_cells = Vec::new();
    for i in 1..words.len() {
        let next = val((base + i) as u16);
        let dst = (cells.len() + sum_cells.len()) as u16;
        sum_cells.push(arith(0, acc, next));
        acc = val(dst);
    }
    let ret = acc;
    cells.extend(sum_cells);
    let blk = WBlock {
        livein: vec![0],
        cells,
        writes: vec![],
        term: WTerm::Return(ret),
    };
    let cfg = WProgConfig {
        n_params: 1,
        blocks: vec![blk],
    };
    emit("heapneg", &cfg, &[0], &[]);
}

/// D: register write phase with negative / high-word values carried across a write (the wbuf read-before-write
/// path under ConfigBram). Two blocks: block0 writes several registers from konsts (incl negatives), jumps to
/// block1 which returns their sum via live-ins.
#[test]
fn emit_reg_writes_neg() {
    let vals: [i64; 5] = [-1, -256, 0x0000_0001_0000_0000, i64::MIN + 7, 42];
    let mut writes = Vec::new();
    for (r, v) in vals.iter().enumerate() {
        writes.push(((r + 1) as u16, k(*v))); // registers 1..=5
    }
    let blk0 = WBlock {
        livein: vec![0],
        cells: vec![],
        writes,
        term: WTerm::Jump(1),
    };
    // block1: sum registers 1..=5 (live-ins), return.
    let li: Vec<u16> = (1..=5).collect();
    let mut cells = Vec::new();
    let mut acc = inp(0); // = register 1 -> livein index 0
    for i in 1..vals.len() {
        let dst = cells.len() as u16;
        cells.push(arith(0, acc, inp(i as u16)));
        acc = val(dst);
    }
    let ret = acc;
    let blk1 = WBlock {
        livein: li,
        cells,
        writes: vec![],
        term: WTerm::Return(ret),
    };
    let cfg = WProgConfig {
        n_params: 1,
        blocks: vec![blk0, blk1],
    };
    emit("regwrneg", &cfg, &[0], &[]);
}
