//! wpe_backend_lock — the whole-program engine's MEMORY BACKEND is a first-class, selectable choice, and it is
//! CORRECT for every choice. The same program, streamed over the wire into the engine emitted with each
//! `MemBackend` (Registers / Bram / ConfigBram), runs byte-identical to `eval_wprog_config` (≡ the VM) under
//! iverilog — and each memory class gets the substrate the backend names (config + state in flip-flops, state
//! plane in block RAM, or config AND state in block RAM). No hard-wiring: the substrate is chosen, not baked.
//!
//! In simulation `ram_style` is a no-op hint, so all three backends are proven functionally ≡VM here; on
//! silicon, `Registers` and `Bram` (state-plane block RAM) are the proven flows, and `ConfigBram` (config image
//! in block RAM — the large-capacity path for the whole transformer) additionally needs the registered
//! config-read FSM to make yosys infer the config BRAM (the named next build; the BRAM budget already fits).
//!
//! Run: `cargo test -p logicaffeine-tests --test wpe_backend_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::{program_cfg_fabric, with_seq_reserve};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog_backend,
    emit_wprog_engine_soc_verilog_backend_debug, emit_wprog_engine_soc_verilog_backend_rdw,
    eval_wprog_config, eval_wprog_config_counted_heap_preloaded, serialize_program,
    wprog_config_image, wprog_config_image_unchecked, MemBackend, WBlock, WCell, WProgConfig, WSrc,
    WTerm, WPE_STREAM_CAPS, WPE_STREAM_HEAP, WPE_SYNTH_DEPTH, WPE_SYNTH_HEAP, WPE_SYNTH_MAXARG,
    WPE_SYNTH_MAXBLK, WPE_SYNTH_MAXCELL, WPE_SYNTH_MAXK, WPE_SYNTH_MAXLIV, WPE_SYNTH_MAXWR,
    WPE_SYNTH_NREG,
};
use logicaffeine_compile::compile::vm_outcome;

const CLKS_PER_BIT: usize = 8;
const CAPS: [usize; 10] = [
    WPE_SYNTH_NREG,
    WPE_SYNTH_MAXBLK,
    WPE_SYNTH_MAXCELL,
    WPE_SYNTH_MAXWR,
    WPE_SYNTH_MAXARG,
    WPE_SYNTH_MAXLIV,
    WPE_SYNTH_MAXK,
    WPE_SYNTH_HEAP,
    WPE_SYNTH_DEPTH,
    WPE_SYNTH_MAXCELL,
];

// A small whole-program with a loop + division + mod — exercises blocks, cells, the func unit, and the
// register file, so ≡VM is a real check of the interpret core under each backend.
const SRC: &str = "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 0.\n    While i is less than x:\n        Set s to s + (i / 2) + (i % 3).\n        Set i to i + 1.\n    Return s.\n";

fn vm_f(x: i64) -> i64 {
    let prog = format!("{SRC}## Main\n    Show f({x}).\n");
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error at x={x}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<i64>()
        .unwrap()
}

/// Emit the engine with `backend`, stream the program + each input over a simulated UART, and assert every
/// result matches `eval_wprog_config` (≡ VM) under iverilog. Returns the emitted RTL (for structural checks).
fn run_backend(tag: &str, backend: MemBackend, inputs: &[i64]) -> String {
    let cfg = serialize_program(&program_cfg_fabric(SRC, "f"));
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| {
            let m = eval_wprog_config(&cfg, &[x]);
            assert_eq!(m, vm_f(x), "engine model f({x}) != VM");
            m
        })
        .collect();
    let soc = emit_wprog_engine_soc_verilog_backend(tag, CAPS, backend);
    if !common::require_tool("iverilog") {
        return soc;
    }
    let dir = std::env::temp_dir().join("logos_wpe_backend");
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image(&cfg);
    let (mut all, mut lens) = (Vec::<u8>::new(), Vec::<u32>::new());
    for &x in inputs {
        let b = im.wire_bytes(&[x]);
        lens.push(b.len() as u32);
        all.extend_from_slice(&b);
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
    let tb = emit_wprog_engine_soc_testbench(tag, inputs.len(), CLKS_PER_BIT, all.len());
    let srcv = dir.join(format!("wpe_soc_{tag}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{tag}.v"));
    let sim = dir.join(format!("sim_{tag}.vvp"));
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "{tag}: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "{tag}: engine diverged from eval_wprog_config:\n{stdout}"
    );
    soc
}

/// The `ram_style` a given memory declaration got in the emitted RTL (block | registers). Whitespace is
/// collapsed first — the template column-aligns its declarations with padding spaces (`cell_kind      [0:`).
fn style_of(rtl: &str, mem: &str) -> String {
    let needle = format!(" {mem} [0:");
    let line = rtl
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|l| l.contains(&needle))
        .unwrap_or_else(|| panic!("no decl for `{mem}`"));
    if line.contains("ram_style = \"block\"") {
        "block".into()
    } else if line.contains("ram_style = \"registers\"") {
        "registers".into()
    } else {
        "none".into()
    }
}

/// A program that reads PRE-LOADED weights via `loaded()` (heap pointer 0) — the streamed-weights primitive
/// the whole transformer uses. `sumw(n) = Σ_{k=1..n} loaded()[k]`. With the blob `[len=4, 10, 20, 30, 40]`
/// pre-loaded, `sumw(4)=100`, `sumw(2)=30`.
const LOADED_SUM: &str = "## To native loaded () -> Seq of Int\n## To sumw (n: Int) -> Int:\n    Let s be 0.\n    Let i be 0.\n    While i is less than n:\n        Set s to s + item (i + 1) of loaded().\n        Set i to i + 1.\n    Return s.\n";

/// ★ HEAP PRE-LOAD ≡ MODEL — the streamed-weights primitive on the ACTUAL engine RTL. The weight blob is
/// streamed over the wire (`wire_bytes_preloaded`) into the engine's block-RAM heap, `loaded()` reads it, the
/// bump allocator is seeded past it, and `P_CLR` zeroes only the runtime region so the weights survive. This
/// is the RTL proof that the whole streamed transformer's weight-delivery path is correct (the full 158-block
/// net is too slow to bit-bang over a simulated UART; this small `loaded()`-reader exercises every loader
/// state — P_HN count, P_HD data, INIT `hp<-hpre`, P_CLR from `hpre` — that the transformer relies on).
#[test]
fn heap_preload_streams_weights_into_the_engine_and_runs_byte_identical_to_the_model() {
    let heap_init: Vec<i64> = vec![4, 10, 20, 30, 40]; // loaded()[0]=len=4, then 4 weights
    let cfg = serialize_program(&program_cfg_fabric(LOADED_SUM, "sumw"));

    // The pre-loaded model (≡ the VM's `loaded()` semantics): sum the streamed weights.
    assert_eq!(
        eval_wprog_config_counted_heap_preloaded(&cfg, &[4], &heap_init).0,
        100,
        "pre-loaded model must sum all four streamed weights (10+20+30+40)"
    );
    assert_eq!(
        eval_wprog_config_counted_heap_preloaded(&cfg, &[2], &heap_init).0,
        30,
        "pre-loaded model must sum the first two streamed weights (10+20)"
    );

    let inputs = [4i64, 2, 3];
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &heap_init).0)
        .collect();

    // Heap in BLOCK RAM — the pre-load must land in BRAM and survive the per-run P_CLR. This is the transformer's
    // backend.
    let soc = emit_wprog_engine_soc_verilog_backend("hpre", CAPS, MemBackend::Bram);
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_wpe_hpre");
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image(&cfg);
    let (mut all, mut lens) = (Vec::<u8>::new(), Vec::<u32>::new());
    for &n in &inputs {
        let b = im.wire_bytes_preloaded(&[n], &heap_init);
        lens.push(b.len() as u32);
        all.extend_from_slice(&b);
    }
    std::fs::write(
        dir.join("wire_hpre.hex"),
        all.iter().map(|b| format!("{b:02x}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join("lens_hpre.hex"),
        lens.iter()
            .map(|n| format!("{n:08x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join("oracle_hpre.hex"),
        oracle
            .iter()
            .map(|&x| format!("{:016x}\n", x as u64))
            .collect::<String>(),
    )
    .unwrap();
    let tb = emit_wprog_engine_soc_testbench("hpre", inputs.len(), CLKS_PER_BIT, all.len());
    let srcv = dir.join("wpe_soc_hpre.v");
    let tbv = dir.join("tb_wpe_soc_hpre.v");
    let sim = dir.join("sim_hpre.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "hpre: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "hpre: engine diverged from the pre-loaded model:\n{stdout}"
    );
    println!("HEAP PRE-LOAD ≡ MODEL: streamed weights land in the engine's block-RAM heap, loaded() reads them, the bump allocator starts past them, and the run matches the pre-loaded model byte-identically over the wire.");
}

/// The transformer-shaped combination that CFG_BRAM=1 first meets on the whole net and that NO existing test
/// exercises: pre-loaded weights read via `loaded()`, MANY live registers carried across a loop, several
/// within-block writes per iteration (some reading a register written EARLIER in the same block's write phase),
/// a branch inside the loop body, and reads of just-written cell values `v[op]`. Streamed with
/// `wire_bytes_preloaded` under `MemBackend::ConfigBram` (CFG_BRAM=1) into the ACTUAL engine RTL, checked
/// byte-for-byte vs `eval_wprog_config_counted_heap_preloaded` (≡ the VM) under iverilog. This is the ≡VM proof
/// for the on-chip wrong-answer bug: the model returns 80113 for the transformer, the chip returned 3.
const XFMR_SHAPED: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let xs be a new Seq of Int.
    Let mutable a be 0.
    Let mutable b be 1.
    Let mutable c be 7.
    Let mutable i be 0.
    While i is less than n:
        Let w be item (i + 1) of loaded().
        Set a to a + w.
        Set b to (b * 2) + w.
        Set c to (c + a) + b.
        Push (a + b) to xs.
        Push c to xs.
        Set i to i + 1.
    Let mutable s be 0.
    Let mutable k be 0.
    While k is less than (length of xs):
        Set s to s + (item (k + 1) of xs).
        Set k to k + 1.
    Return ((a + b) + c) + s.
";

/// ★ CFG_BRAM=1 + PRE-LOADED WEIGHTS + MANY REGISTERS + WRITES + BRANCH ≡VM — the transformer-shaped
/// combination the whole net first meets (Push-built collection + `loaded()` reads + several live registers
/// across a loop + a branch, at the STREAM caps + `with_seq_reserve(8)` the on-chip engine actually uses). This
/// is a behavioural ≡VM lock on the CFG_BRAM=1 operand/write/heap paths the transformer drives — the fast guard
/// that a future change to those paths stays byte-identical to `eval_wprog_config_counted_heap_preloaded` ≡ VM.
#[test]
fn config_bram_preloaded_weights_many_registers_runs_byte_identical_to_the_vm() {
    let heap_init: Vec<i64> = vec![4, 10, 30, 5, 40]; // loaded()[0]=len=4, then 4 weights
                                                      // Right-size the per-collection reserve (8, not the 2048 default) exactly as the streamed transformer does,
                                                      // so the Push-built `xs` heap stays tiny — the config lowering AND the engine deep-copy honour it, so the
                                                      // ≡VM run is self-consistent — and use the STREAM caps the transformer's on-chip engine actually uses.
    let (cfg, oracle, inputs) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(XFMR_SHAPED, "acc"));
        let inputs = [2i64, 4, 1, 0];
        let oracle: Vec<i64> = inputs
            .iter()
            .map(|&n| eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &heap_init).0)
            .collect();
        (cfg, oracle, inputs)
    });

    let soc =
        emit_wprog_engine_soc_verilog_backend("xfmr", WPE_STREAM_CAPS, MemBackend::ConfigBram);
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_wpe_xfmr");
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image(&cfg);
    let (mut all, mut lens) = (Vec::<u8>::new(), Vec::<u32>::new());
    for &n in &inputs {
        let b = im.wire_bytes_preloaded(&[n], &heap_init);
        lens.push(b.len() as u32);
        all.extend_from_slice(&b);
    }
    std::fs::write(
        dir.join("wire_xfmr.hex"),
        all.iter().map(|b| format!("{b:02x}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join("lens_xfmr.hex"),
        lens.iter()
            .map(|n| format!("{n:08x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join("oracle_xfmr.hex"),
        oracle
            .iter()
            .map(|&x| format!("{:016x}\n", x as u64))
            .collect::<String>(),
    )
    .unwrap();
    let tb = emit_wprog_engine_soc_testbench("xfmr", inputs.len(), CLKS_PER_BIT, all.len());
    let srcv = dir.join("wpe_soc_xfmr.v");
    let tbv = dir.join("tb_wpe_soc_xfmr.v");
    let sim = dir.join("sim_xfmr.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "xfmr: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "xfmr: engine diverged from the pre-loaded model (the on-chip bug):\n{stdout}"
    );
    println!("CFG_BRAM PRE-LOAD + MANY-REGISTER LOOP ≡VM: the transformer-shaped combination runs byte-identical to eval_wprog_config_counted_heap_preloaded ≡ VM over the wire.");
}

/// ★ ON-CHIP READBACK ECHOES THE LOADED IMAGE — the synthesis-bug localizer, proven in sim. The
/// synthesized 200T returns a near-zero answer (3) where the model returns 80113, and the hypothesis is that
/// yosys mis-maps one of the wide 64-bit config/heap BRAMs (the synth log shows 64→32 data-port resizing on
/// `cell_*`/`heap` + `file` address resizing). To LOCALIZE which memory is wrong on silicon, the engine gains a
/// compile-time `DEBUG_READBACK` mode: at the load→run boundary it streams a DEBUG FRAME of a fixed diagnostic
/// slice of each major memory back over the SAME UART tx BEFORE interpreting, so the host can compare the
/// echoed values against the KNOWN config image / heap blob byte-for-byte.
///
/// This test proves the readback logic itself is CORRECT in sim: it reuses the transformer-shaped harness
/// (`XFMR_SHAPED`, 8 blocks / 35 cells so every echoed `[0..8)` slot is loaded + deterministic), emits the
/// engine with `DEBUG_READBACK=1`, streams config + an 8-word weight blob over the bit-banged UART, then decodes
/// the DEBUG FRAME and asserts every echoed `cell_kind`/`cell_op0`/`cell_op1`/`blk_term_kind`/`heap[0..8)` word
/// equals the known image/heap value. With the readback proven ≡ image in sim, ANY mismatch on real silicon is
/// the synthesis bug — not the readback — so it points straight at the mis-mapped BRAM.
///
/// DEBUG FRAME layout (the host parser): sync `0xD8`, then for each memory in order
/// {`cell_kind`, `cell_op0`, `cell_op1`, `blk_term_kind`, `heap`} entries `[0..8)`, each a 64-bit BIG-ENDIAN
/// word (8 bytes MSB-first, the result-frame byte order; the 32-bit config words zero-extend), then end `0x8D`.
#[test]
fn config_bram_debug_readback_echoes_the_loaded_image() {
    // A blob with ≥8 words so heap[0..8) are all loaded weights (they survive P_CLR — it zeroes only the runtime
    // region past `hpre`), and distinct values so a mismatch is unambiguous.
    let heap_init: Vec<i64> = vec![8, 11, 22, 33, 44, 55, 66, 77, 88, 99];
    let (cfg, oracle_head, inputs) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(XFMR_SHAPED, "acc"));
        let inputs = [2i64];
        // The run must still terminate + return the model result (the readback runs BEFORE the interpret).
        let oracle: Vec<i64> = inputs
            .iter()
            .map(|&n| eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &heap_init).0)
            .collect();
        (cfg, oracle, inputs)
    });

    // The KNOWN image slices the readback must echo (pad to 8 with 0 — an unloaded config slot / cleared heap
    // word reads 0; here every slot is loaded, but the pad keeps the oracle total).
    let im = wprog_config_image(&cfg);
    let slice8 = |v: &[u32]| -> Vec<i64> {
        (0..8)
            .map(|i| v.get(i).copied().unwrap_or(0) as i64)
            .collect()
    };
    let heap8: Vec<i64> = (0..8)
        .map(|i| heap_init.get(i).copied().unwrap_or(0))
        .collect();
    // The order the DEBUG FRAME walks: cell_kind, cell_op0, cell_op1, blk_term_kind, heap.
    let expected: Vec<(&str, Vec<i64>)> = vec![
        ("cell_kind", slice8(&im.cell_kind)),
        ("cell_op0", slice8(&im.cell_op0)),
        ("cell_op1", slice8(&im.cell_op1)),
        ("blk_term_kind", slice8(&im.blk_term_kind)),
        ("heap", heap8),
    ];

    let soc = emit_wprog_engine_soc_verilog_backend_debug(
        "dbgrb",
        WPE_STREAM_CAPS,
        MemBackend::ConfigBram,
        true,
    );
    // Byte-identity guard: emitting WITHOUT debug must not contain any readback text (the gating is byte-perfect).
    let nodbg =
        emit_wprog_engine_soc_verilog_backend("dbgrb0", WPE_STREAM_CAPS, MemBackend::ConfigBram);
    assert!(
        !nodbg.contains("DEBUG_READBACK") && !nodbg.contains("P_DBG") && !nodbg.contains("dbg_"),
        "DEBUG_READBACK=0 RTL must be byte-identical to today (no readback text leaked)"
    );
    assert!(
        soc.contains("parameter DEBUG_READBACK = 1") && soc.contains("P_DBG"),
        "DEBUG_READBACK=1 RTL must carry the readback parameter + FSM"
    );

    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_wpe_dbgrb");
    std::fs::create_dir_all(&dir).unwrap();
    let wire = im.wire_bytes_preloaded(&[inputs[0]], &heap_init);
    std::fs::write(
        dir.join("wire_dbgrb.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();

    // A bit-bang-UART host: stream the whole config+weights over uart_rx (the SAME send_byte the shared host
    // uses), FIRST decode the DEBUG FRAME (0xD8 + 5*8 words MSB-first + 0x8D), check every word against the
    // known image, THEN decode the result frame (0xA5 + result + cycles + 0x5A) so the run still completed.
    let n_wire = wire.len();
    let model = oracle_head[0];
    let dbg_words: Vec<i64> = expected
        .iter()
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    let n_dbg = dbg_words.len(); // 5 * 8 = 40
    let dbg_hex: String = dbg_words
        .iter()
        .map(|w| format!("{:016x}\n", *w as u64))
        .collect();
    std::fs::write(dir.join("dbg_expected.hex"), &dbg_hex).unwrap();

    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_dbgrb;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  localparam NDBG = {n_dbg};
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  // WITH_FPU(0) + CFG_BRAM(1) + DEBUG_READBACK(1) — the on-chip synth parameters plus the readback probe.
  wpe_soc_dbgrb #(.CLKS_PER_BIT(CLKS), .WITH_FPU(0), .CFG_BRAM(1), .DEBUG_READBACK(1))
    dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;

  reg [7:0] wirebytes [0:MAXB-1];
  reg signed [63:0] dbg_exp [0:NDBG-1];
  integer k, fk, bi, errs;
  reg [7:0] rb, sync, endb;
  reg [63:0] word, result; reg [31:0] cyc; integer got;

  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask
  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask

  initial begin
    $readmemh("wire_dbgrb.hex", wirebytes);
    $readmemh("dbg_expected.hex", dbg_exp);
    errs = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);

    // ── DEBUG FRAME first: 0xD8, then NDBG 64-bit big-endian words, then 0x8D ──
    recv_byte(sync);
    if (sync !== 8'hD8) begin $display("FAIL: debug sync=%02x exp d8", sync); errs = errs + 1; end
    for (fk=0; fk<NDBG; fk=fk+1) begin
      word = 64'd0;
      for (bi=0; bi<8; bi=bi+1) begin recv_byte(rb); word = {{word[55:0], rb}}; end
      if (word !== dbg_exp[fk]) begin
        $display("FAIL: debug word %0d got %0d exp %0d", fk, $signed(word), $signed(dbg_exp[fk]));
        errs = errs + 1;
      end
    end
    recv_byte(endb);
    if (endb !== 8'h8D) begin $display("FAIL: debug end=%02x exp 8d", endb); errs = errs + 1; end

    // ── RESULT FRAME: 0xA5 + result[63:0] + cycles[31:0] + 0x5A — proves the run still completed after readback ──
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync!==8'hA5 || endb!==8'h5A || result!=64'd{model}) begin
      $display("FAIL: result frame got %0d (sync=%02x end=%02x) exp %0d", got, sync, endb, {model});
      errs = errs + 1;
    end

    if (errs == 0)
      $display("PASS: debug readback echoed the loaded image (%0d words) + the run returned %0d in %0d cycles", NDBG, got, cyc);
    else
      $display("FAIL: %0d debug-readback mismatches", errs);
    $finish;
  end
  initial begin #40000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );

    let srcv = dir.join("wpe_soc_dbgrb.v");
    let tbv = dir.join("tb_dbgrb.v");
    let sim = dir.join("sim_dbgrb.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "dbgrb: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "dbgrb: the debug readback did not echo the loaded image:\n{stdout}"
    );
    println!("ON-CHIP READBACK ≡ IMAGE (sim): the DEBUG FRAME echoed cell_kind/op0/op1/blk_term_kind/heap[0..8) byte-for-byte against the known config image + weight blob, then the run returned the model result — so a silicon mismatch localizes the synthesis bug. {}", stdout.trim());
}

/// ★★★ THE WHOLE STREAMED TRANSFORMER, CFG_BRAM=1, DIRECT-LOADED ≡VM — the make-or-break behavioural proof. The
/// full 158-block streamed transformer is too slow to bit-bang over a simulated UART, so this testbench
/// DIRECT-LOADS the flat config image + the pre-loaded weight blob straight into the engine's config/heap
/// memories (via `$readmemh` into the DUT's arrays), forces the loader into `P_INIT`, and runs the SAME
/// CFG_BRAM=1 interpret FSM the on-chip bitstream runs — with `WITH_FPU(0)` + `CFG_BRAM(1)` (the EXACT on-chip
/// `chparam` synth parameters) — then reads back the 64-bit result over the TX frame. The model
/// (`eval_wprog_config_counted_heap_preloaded`) generates "mat" = 80113. This proves the CFG_BRAM=1 engine RTL
/// is ≡VM on the whole transformer at the on-chip parameters — so a wrong on-chip answer is a synthesis/route
/// artifact, not an RTL-logic bug (the fix that hardens the `v`/`file` BRAM mapping lives in the template's
/// `resolve`/`resolve0`, gating their dead combinational reads behind `!CFG_BRAM`). python3-gated.
#[test]
fn whole_streamed_transformer_runs_byte_identical_to_the_model_under_config_bram() {
    fn gen(flags: &[&str]) -> Option<String> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let out = std::process::Command::new("python3")
            .current_dir(&root)
            .arg("scripts/gen_tiny_transformer_lg.py")
            .args(flags)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    if !common::require_tool("python3") {
        return;
    }
    if !common::require_tool("iverilog") {
        return;
    }
    let src = gen(&["--fabric", "--stream"]).expect("streamed transformer program");
    let blob: Vec<i64> = {
        let s = gen(&["--blob"]).expect("weight blob");
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i64>().expect("blob int"))
            .collect()
    };

    // The engine model IS the golden: the transformer must generate "mat" (80113) before we prove the RTL.
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        let (m, _steps, _heap) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (cfg, m)
    });
    assert_eq!(
        model, 80113,
        "the engine model must generate \"mat\" (80113) before we flash the RTL"
    );

    let im = wprog_config_image_unchecked(&cfg);
    let soc =
        emit_wprog_engine_soc_verilog_backend("wxfmr", WPE_STREAM_CAPS, MemBackend::ConfigBram);

    let dir = std::env::temp_dir().join("logos_wpe_wxfmr");
    std::fs::create_dir_all(&dir).unwrap();

    // Dump every flat config array + the weight blob as `$readmemh` files (one hex word per line).
    let dump_u32 = |name: &str, v: &[u32]| {
        std::fs::write(
            dir.join(format!("{name}.hex")),
            v.iter().map(|x| format!("{x:08x}\n")).collect::<String>(),
        )
        .unwrap();
    };
    let dump_i64 = |name: &str, v: &[i64]| {
        std::fs::write(
            dir.join(format!("{name}.hex")),
            v.iter()
                .map(|x| format!("{:016x}\n", *x as u64))
                .collect::<String>(),
        )
        .unwrap();
    };
    dump_u32("blk_livein_ptr", &im.blk_livein_ptr);
    dump_u32("blk_n_livein", &im.blk_n_livein);
    dump_u32("blk_cell_ptr", &im.blk_cell_ptr);
    dump_u32("blk_n_cells", &im.blk_n_cells);
    dump_u32("blk_write_ptr", &im.blk_write_ptr);
    dump_u32("blk_n_writes", &im.blk_n_writes);
    dump_u32("blk_term_kind", &im.blk_term_kind);
    dump_u32("blk_t0", &im.blk_t0);
    dump_u32("blk_t1", &im.blk_t1);
    dump_u32("blk_top", &im.blk_top);
    dump_u32("blk_cdst", &im.blk_cdst);
    dump_u32("blk_cargc", &im.blk_cargc);
    dump_u32("blk_cargp", &im.blk_cargp);
    dump_u32("cell_kind", &im.cell_kind);
    dump_u32("cell_code", &im.cell_code);
    dump_u32("cell_op0", &im.cell_op0);
    dump_u32("cell_op1", &im.cell_op1);
    dump_u32("cell_op2", &im.cell_op2);
    dump_u32("cell_op3", &im.cell_op3);
    dump_u32("wr_reg", &im.wr_reg);
    dump_u32("wr_op", &im.wr_op);
    dump_u32("arg_op", &im.arg_op);
    dump_u32("liv_reg", &im.liv_reg);
    dump_i64("konst", &im.konst);
    dump_i64("heapblob", &blob);

    // A direct-load testbench: force the flat config image + weight blob straight into the DUT's memories, seed
    // `hpre`, then drive the loader FSM into P_INIT (so INIT→P_FINIT→P_CLR→P_RUN→P_SEND run the SAME CFG_BRAM=1
    // interpret FSM the on-chip bitstream does), and decode the TX result frame. No UART streaming (too slow).
    let n_blocks = im.n_blocks;
    let n_cells = im.cell_kind.len();
    let n_wr = im.wr_reg.len();
    let n_liv = im.liv_reg.len();
    let n_arg = im.arg_op.len().max(1);
    let n_k = im.konst.len().max(1);
    let n_blob = blob.len();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_wxfmr;
  reg clk=0, rst=1;
  wire tx;
  // WITH_FPU(0) + CFG_BRAM(1) — EXACTLY the on-chip synth parameters (ser-build-200t.sh does `chparam -set
  // WITH_FPU 0`), so this behavioral run matches the flashed bitstream's RTL configuration byte-for-byte.
  wpe_soc_wxfmr #(.CLKS_PER_BIT(8), .WITH_FPU(0), .CFG_BRAM(1)) dut(.clk(clk), .rst(rst), .uart_rx(1'b1), .uart_tx(tx));
  always #5 clk = ~clk;
  integer i;
  reg [31:0] t_blk_livein_ptr[0:{n_blocks}-1]; reg [31:0] t_blk_n_livein[0:{n_blocks}-1];
  reg [31:0] t_blk_cell_ptr[0:{n_blocks}-1];   reg [31:0] t_blk_n_cells[0:{n_blocks}-1];
  reg [31:0] t_blk_write_ptr[0:{n_blocks}-1];  reg [31:0] t_blk_n_writes[0:{n_blocks}-1];
  reg [31:0] t_blk_term_kind[0:{n_blocks}-1];  reg [31:0] t_blk_t0[0:{n_blocks}-1];
  reg [31:0] t_blk_t1[0:{n_blocks}-1];         reg [31:0] t_blk_top[0:{n_blocks}-1];
  reg [31:0] t_blk_cdst[0:{n_blocks}-1];       reg [31:0] t_blk_cargc[0:{n_blocks}-1];
  reg [31:0] t_blk_cargp[0:{n_blocks}-1];
  reg [31:0] t_cell_kind[0:{n_cells}-1]; reg [31:0] t_cell_code[0:{n_cells}-1];
  reg [31:0] t_cell_op0[0:{n_cells}-1];  reg [31:0] t_cell_op1[0:{n_cells}-1];
  reg [31:0] t_cell_op2[0:{n_cells}-1];  reg [31:0] t_cell_op3[0:{n_cells}-1];
  reg [31:0] t_wr_reg[0:{n_wr}-1]; reg [31:0] t_wr_op[0:{n_wr}-1];
  reg [31:0] t_arg_op[0:{n_arg}-1]; reg [31:0] t_liv_reg[0:{n_liv}-1];
  reg signed [63:0] t_konst[0:{n_k}-1]; reg signed [63:0] t_heapblob[0:{n_blob}-1];
  reg [63:0] result; integer got;

  task recv_byte(output [7:0] b); integer k;
    begin
      @(negedge tx);
      repeat (8 + 4) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = tx; if (k<7) repeat (8) @(posedge clk); end
    end
  endtask
  reg [7:0] sync, endb, rb; integer fk; reg [31:0] cyc;

  initial begin
    $readmemh("blk_livein_ptr.hex", t_blk_livein_ptr); $readmemh("blk_n_livein.hex", t_blk_n_livein);
    $readmemh("blk_cell_ptr.hex", t_blk_cell_ptr);     $readmemh("blk_n_cells.hex", t_blk_n_cells);
    $readmemh("blk_write_ptr.hex", t_blk_write_ptr);   $readmemh("blk_n_writes.hex", t_blk_n_writes);
    $readmemh("blk_term_kind.hex", t_blk_term_kind);   $readmemh("blk_t0.hex", t_blk_t0);
    $readmemh("blk_t1.hex", t_blk_t1);                 $readmemh("blk_top.hex", t_blk_top);
    $readmemh("blk_cdst.hex", t_blk_cdst);             $readmemh("blk_cargc.hex", t_blk_cargc);
    $readmemh("blk_cargp.hex", t_blk_cargp);
    $readmemh("cell_kind.hex", t_cell_kind); $readmemh("cell_code.hex", t_cell_code);
    $readmemh("cell_op0.hex", t_cell_op0);   $readmemh("cell_op1.hex", t_cell_op1);
    $readmemh("cell_op2.hex", t_cell_op2);   $readmemh("cell_op3.hex", t_cell_op3);
    $readmemh("wr_reg.hex", t_wr_reg);       $readmemh("wr_op.hex", t_wr_op);
    $readmemh("arg_op.hex", t_arg_op);       $readmemh("liv_reg.hex", t_liv_reg);
    $readmemh("konst.hex", t_konst);         $readmemh("heapblob.hex", t_heapblob);

    // Hold reset a few cycles, then STOP the loader FSM at P_CNT and hijack it: force the config image in.
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (i=0;i<{n_blocks};i=i+1) begin
      dut.blk_livein_ptr[i]=t_blk_livein_ptr[i]; dut.blk_n_livein[i]=t_blk_n_livein[i];
      dut.blk_cell_ptr[i]=t_blk_cell_ptr[i];     dut.blk_n_cells[i]=t_blk_n_cells[i];
      dut.blk_write_ptr[i]=t_blk_write_ptr[i];   dut.blk_n_writes[i]=t_blk_n_writes[i];
      dut.blk_term_kind[i]=t_blk_term_kind[i];   dut.blk_t0[i]=t_blk_t0[i];
      dut.blk_t1[i]=t_blk_t1[i];                 dut.blk_top[i]=t_blk_top[i];
      dut.blk_cdst[i]=t_blk_cdst[i];             dut.blk_cargc[i]=t_blk_cargc[i];
      dut.blk_cargp[i]=t_blk_cargp[i];
    end
    for (i=0;i<{n_cells};i=i+1) begin
      dut.cell_kind[i]=t_cell_kind[i]; dut.cell_code[i]=t_cell_code[i];
      dut.cell_op0[i]=t_cell_op0[i];   dut.cell_op1[i]=t_cell_op1[i];
      dut.cell_op2[i]=t_cell_op2[i];   dut.cell_op3[i]=t_cell_op3[i];
    end
    for (i=0;i<{n_wr};i=i+1) begin dut.wr_reg[i]=t_wr_reg[i]; dut.wr_op[i]=t_wr_op[i]; end
    for (i=0;i<{n_liv};i=i+1) dut.liv_reg[i]=t_liv_reg[i];
    for (i=0;i<{n_arg};i=i+1) dut.arg_op[i]=t_arg_op[i];
    for (i=0;i<{n_k};i=i+1) dut.konst[i]=t_konst[i];
    for (i=0;i<{n_blob};i=i+1) begin if (i[14]) begin dut.heap_b1_lo[i[13:0]]=t_heapblob[i][31:0]; dut.heap_b1_hi[i[13:0]]=t_heapblob[i][63:32]; end else begin dut.heap_b0_lo[i[13:0]]=t_heapblob[i][31:0]; dut.heap_b0_hi[i[13:0]]=t_heapblob[i][63:32]; end end
    // No params (generate() takes none); seed hpre so INIT sets hp past the weights, P_CLR zeroes only runtime.
    dut.np = 16'd0; dut.hpre = 16'd{n_blob};
    // Drive the loader straight into P_INIT (== the '0x5A' go byte in P_GO). Everything below is the same FSM
    // the bitstream runs: P_INIT -> P_FINIT (serial file seed) -> P_CLR -> P_RUN (CFG_BRAM interpret) -> P_SEND.
    @(negedge clk); dut.ph = 4'd5; // P_INIT

    // Decode the returned frame (0xA5 + result[63:0] MSB-first + cycles[31:0] + 0x5A).
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync===8'hA5 && endb===8'h5A && result==64'd{model})
      $display("PASS: whole transformer generated %0d (mat) on the CFG_BRAM engine in %0d cycles", got, cyc);
    else
      $display("FAIL: got %0d (sync=%02x end=%02x) exp %0d", got, sync, endb, {model});
    $finish;
  end
  initial begin #200000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );

    let srcv = dir.join("wpe_soc_wxfmr.v");
    let tbv = dir.join("tb_wxfmr.v");
    let sim = dir.join("sim_wxfmr.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "wxfmr: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("PASS:"), "wxfmr: the CFG_BRAM engine diverged from the model on the whole transformer (the on-chip bug):\n{stdout}");
    println!("WHOLE TRANSFORMER ≡ MODEL under CFG_BRAM=1: {stdout}");
}

/// ★ RDW COLLISION CONFIRM/REFUTE — the make-or-break diagnostic for the on-chip wrong-answer bug (silicon
/// returns 130913 where the behavioral model returns 80113). The leading hypothesis is a same-cycle cross-port
/// BRAM read-during-write collision: a registered read `*_rd <= mem[*_ra]` (port A) firing the cycle a write
/// `mem[wa] <= wd` (port B) hits `wa == *_ra`. On a real RAMB36 that read returns UNDEFINED; this behavioral sim
/// (and the gate-level model) idealize it as READ_FIRST (old data). This test runs the WHOLE transformer through
/// the SAME CFG_BRAM=1 interpret FSM the bitstream runs, but with the sim-only RDW detectors active
/// (`RDW_DETECT(1)`), which `$display` + count every same-cycle read-during-write coincidence on `v`/`heap`/
/// `file`/`sfile`. It asserts the transformer STILL generates 80113 (detectors are pure observers) and prints the
/// per-memory collision counts — CONFIRMING the hypothesis if any fire, REFUTING it if none do. python3-gated.
#[test]
fn whole_transformer_rdw_collision_detectors_report_cross_port_hazards() {
    fn gen(flags: &[&str]) -> Option<String> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let out = std::process::Command::new("python3")
            .current_dir(&root)
            .arg("scripts/gen_tiny_transformer_lg.py")
            .args(flags)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    if !common::require_tool("python3") {
        return;
    }
    if !common::require_tool("iverilog") {
        return;
    }
    let src = gen(&["--fabric", "--stream"]).expect("streamed transformer program");
    let blob: Vec<i64> = {
        let s = gen(&["--blob"]).expect("weight blob");
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i64>().expect("blob int"))
            .collect()
    };
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        let (m, _steps, _heap) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (cfg, m)
    });
    assert_eq!(
        model, 80113,
        "the engine model must generate \"mat\" (80113) before we probe the RTL"
    );

    let im = wprog_config_image_unchecked(&cfg);
    // The SAME engine RTL the direct-load lock uses, but emitted with the sim-only RDW detectors active.
    let soc =
        emit_wprog_engine_soc_verilog_backend_rdw("wrdw", WPE_STREAM_CAPS, MemBackend::ConfigBram);

    let dir = std::env::temp_dir().join("logos_wpe_wrdw");
    std::fs::create_dir_all(&dir).unwrap();

    let dump_u32 = |name: &str, v: &[u32]| {
        std::fs::write(
            dir.join(format!("{name}.hex")),
            v.iter().map(|x| format!("{x:08x}\n")).collect::<String>(),
        )
        .unwrap();
    };
    let dump_i64 = |name: &str, v: &[i64]| {
        std::fs::write(
            dir.join(format!("{name}.hex")),
            v.iter()
                .map(|x| format!("{:016x}\n", *x as u64))
                .collect::<String>(),
        )
        .unwrap();
    };
    dump_u32("blk_livein_ptr", &im.blk_livein_ptr);
    dump_u32("blk_n_livein", &im.blk_n_livein);
    dump_u32("blk_cell_ptr", &im.blk_cell_ptr);
    dump_u32("blk_n_cells", &im.blk_n_cells);
    dump_u32("blk_write_ptr", &im.blk_write_ptr);
    dump_u32("blk_n_writes", &im.blk_n_writes);
    dump_u32("blk_term_kind", &im.blk_term_kind);
    dump_u32("blk_t0", &im.blk_t0);
    dump_u32("blk_t1", &im.blk_t1);
    dump_u32("blk_top", &im.blk_top);
    dump_u32("blk_cdst", &im.blk_cdst);
    dump_u32("blk_cargc", &im.blk_cargc);
    dump_u32("blk_cargp", &im.blk_cargp);
    dump_u32("cell_kind", &im.cell_kind);
    dump_u32("cell_code", &im.cell_code);
    dump_u32("cell_op0", &im.cell_op0);
    dump_u32("cell_op1", &im.cell_op1);
    dump_u32("cell_op2", &im.cell_op2);
    dump_u32("cell_op3", &im.cell_op3);
    dump_u32("wr_reg", &im.wr_reg);
    dump_u32("wr_op", &im.wr_op);
    dump_u32("arg_op", &im.arg_op);
    dump_u32("liv_reg", &im.liv_reg);
    dump_i64("konst", &im.konst);
    dump_i64("heapblob", &blob);

    let n_blocks = im.n_blocks;
    let n_cells = im.cell_kind.len();
    let n_wr = im.wr_reg.len();
    let n_liv = im.liv_reg.len();
    let n_arg = im.arg_op.len().max(1);
    let n_k = im.konst.len().max(1);
    let n_blob = blob.len();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_wrdw;
  reg clk=0, rst=1;
  wire tx;
  // WITH_FPU(0) + CFG_BRAM(1) + RDW_DETECT(1) — the on-chip synth parameters plus the sim-only collision probe.
  wpe_soc_wrdw #(.CLKS_PER_BIT(8), .WITH_FPU(0), .CFG_BRAM(1), .RDW_DETECT(1)) dut(.clk(clk), .rst(rst), .uart_rx(1'b1), .uart_tx(tx));
  always #5 clk = ~clk;
  integer i;
  reg [31:0] t_blk_livein_ptr[0:{n_blocks}-1]; reg [31:0] t_blk_n_livein[0:{n_blocks}-1];
  reg [31:0] t_blk_cell_ptr[0:{n_blocks}-1];   reg [31:0] t_blk_n_cells[0:{n_blocks}-1];
  reg [31:0] t_blk_write_ptr[0:{n_blocks}-1];  reg [31:0] t_blk_n_writes[0:{n_blocks}-1];
  reg [31:0] t_blk_term_kind[0:{n_blocks}-1];  reg [31:0] t_blk_t0[0:{n_blocks}-1];
  reg [31:0] t_blk_t1[0:{n_blocks}-1];         reg [31:0] t_blk_top[0:{n_blocks}-1];
  reg [31:0] t_blk_cdst[0:{n_blocks}-1];       reg [31:0] t_blk_cargc[0:{n_blocks}-1];
  reg [31:0] t_blk_cargp[0:{n_blocks}-1];
  reg [31:0] t_cell_kind[0:{n_cells}-1]; reg [31:0] t_cell_code[0:{n_cells}-1];
  reg [31:0] t_cell_op0[0:{n_cells}-1];  reg [31:0] t_cell_op1[0:{n_cells}-1];
  reg [31:0] t_cell_op2[0:{n_cells}-1];  reg [31:0] t_cell_op3[0:{n_cells}-1];
  reg [31:0] t_wr_reg[0:{n_wr}-1]; reg [31:0] t_wr_op[0:{n_wr}-1];
  reg [31:0] t_arg_op[0:{n_arg}-1]; reg [31:0] t_liv_reg[0:{n_liv}-1];
  reg signed [63:0] t_konst[0:{n_k}-1]; reg signed [63:0] t_heapblob[0:{n_blob}-1];
  reg [63:0] result; integer got;

  task recv_byte(output [7:0] b); integer k;
    begin
      @(negedge tx);
      repeat (8 + 4) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = tx; if (k<7) repeat (8) @(posedge clk); end
    end
  endtask
  reg [7:0] sync, endb, rb; integer fk; reg [31:0] cyc;

  initial begin
    $readmemh("blk_livein_ptr.hex", t_blk_livein_ptr); $readmemh("blk_n_livein.hex", t_blk_n_livein);
    $readmemh("blk_cell_ptr.hex", t_blk_cell_ptr);     $readmemh("blk_n_cells.hex", t_blk_n_cells);
    $readmemh("blk_write_ptr.hex", t_blk_write_ptr);   $readmemh("blk_n_writes.hex", t_blk_n_writes);
    $readmemh("blk_term_kind.hex", t_blk_term_kind);   $readmemh("blk_t0.hex", t_blk_t0);
    $readmemh("blk_t1.hex", t_blk_t1);                 $readmemh("blk_top.hex", t_blk_top);
    $readmemh("blk_cdst.hex", t_blk_cdst);             $readmemh("blk_cargc.hex", t_blk_cargc);
    $readmemh("blk_cargp.hex", t_blk_cargp);
    $readmemh("cell_kind.hex", t_cell_kind); $readmemh("cell_code.hex", t_cell_code);
    $readmemh("cell_op0.hex", t_cell_op0);   $readmemh("cell_op1.hex", t_cell_op1);
    $readmemh("cell_op2.hex", t_cell_op2);   $readmemh("cell_op3.hex", t_cell_op3);
    $readmemh("wr_reg.hex", t_wr_reg);       $readmemh("wr_op.hex", t_wr_op);
    $readmemh("arg_op.hex", t_arg_op);       $readmemh("liv_reg.hex", t_liv_reg);
    $readmemh("konst.hex", t_konst);         $readmemh("heapblob.hex", t_heapblob);

    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (i=0;i<{n_blocks};i=i+1) begin
      dut.blk_livein_ptr[i]=t_blk_livein_ptr[i]; dut.blk_n_livein[i]=t_blk_n_livein[i];
      dut.blk_cell_ptr[i]=t_blk_cell_ptr[i];     dut.blk_n_cells[i]=t_blk_n_cells[i];
      dut.blk_write_ptr[i]=t_blk_write_ptr[i];   dut.blk_n_writes[i]=t_blk_n_writes[i];
      dut.blk_term_kind[i]=t_blk_term_kind[i];   dut.blk_t0[i]=t_blk_t0[i];
      dut.blk_t1[i]=t_blk_t1[i];                 dut.blk_top[i]=t_blk_top[i];
      dut.blk_cdst[i]=t_blk_cdst[i];             dut.blk_cargc[i]=t_blk_cargc[i];
      dut.blk_cargp[i]=t_blk_cargp[i];
    end
    for (i=0;i<{n_cells};i=i+1) begin
      dut.cell_kind[i]=t_cell_kind[i]; dut.cell_code[i]=t_cell_code[i];
      dut.cell_op0[i]=t_cell_op0[i];   dut.cell_op1[i]=t_cell_op1[i];
      dut.cell_op2[i]=t_cell_op2[i];   dut.cell_op3[i]=t_cell_op3[i];
    end
    for (i=0;i<{n_wr};i=i+1) begin dut.wr_reg[i]=t_wr_reg[i]; dut.wr_op[i]=t_wr_op[i]; end
    for (i=0;i<{n_liv};i=i+1) dut.liv_reg[i]=t_liv_reg[i];
    for (i=0;i<{n_arg};i=i+1) dut.arg_op[i]=t_arg_op[i];
    for (i=0;i<{n_k};i=i+1) dut.konst[i]=t_konst[i];
    for (i=0;i<{n_blob};i=i+1) begin if (i[14]) begin dut.heap_b1_lo[i[13:0]]=t_heapblob[i][31:0]; dut.heap_b1_hi[i[13:0]]=t_heapblob[i][63:32]; end else begin dut.heap_b0_lo[i[13:0]]=t_heapblob[i][31:0]; dut.heap_b0_hi[i[13:0]]=t_heapblob[i][63:32]; end end
    dut.np = 16'd0; dut.hpre = 16'd{n_blob};
    @(negedge clk); dut.ph = 4'd5; // P_INIT

    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    // Report the collision counters (read directly from the DUT after the run finished).
    $display("RDW-SUMMARY v=%0d heap=%0d file=%0d sfile=%0d",
             dut.rdw_v_cnt, dut.rdw_heap_cnt, dut.rdw_file_cnt, dut.rdw_sfile_cnt);
    // Report the CONSUMED-collision counters — the ONLY collisions whose corrupt read propagates into the answer.
    $display("RDW-CONSUMED-SUMMARY v=%0d heap=%0d file=%0d sfile=%0d",
             dut.rdw_v_consumed, dut.rdw_heap_consumed, dut.rdw_file_consumed, dut.rdw_sf_consumed);
    if (sync===8'hA5 && endb===8'h5A && result==64'd{model})
      $display("PASS: whole transformer generated %0d (mat) with RDW detectors active in %0d cycles", got, cyc);
    else
      $display("FAIL: got %0d (sync=%02x end=%02x) exp %0d", got, sync, endb, {model});
    $finish;
  end
  initial begin #200000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );

    let srcv = dir.join("wpe_soc_wrdw.v");
    let tbv = dir.join("tb_wrdw.v");
    let sim = dir.join("sim_wrdw.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "wrdw: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    // The detectors are pure observers — the transformer must STILL generate 80113.
    assert!(stdout.contains("PASS:"), "wrdw: the transformer diverged with RDW detectors active (detectors must not change the result):\n{stdout}");
    // Surface the collision summary + every individual collision line to the test log for the verdict.
    let summary = stdout
        .lines()
        .find(|l| l.contains("RDW-SUMMARY"))
        .unwrap_or("RDW-SUMMARY (missing)");
    let consumed = stdout
        .lines()
        .find(|l| l.contains("RDW-CONSUMED-SUMMARY"))
        .unwrap_or("RDW-CONSUMED-SUMMARY (missing)");
    // ★ THE VERDICT INVARIANT: raw cross-port read-during-write collisions DO occur on `v`/`heap`/`file` (the
    // registered-read/BRAM-write coincidences), but NOT ONE of them feeds a CONSUMED read — every corrupted
    // `*_rd` is stale/unconsumed, so no read-during-write corruption can propagate into the transformer's answer.
    // This is the refutation of the "consumed cross-port RDW read causes the wrong silicon answer" hypothesis,
    // locked so a future change that makes a corrupt read consumable goes RED loudly.
    assert!(
        consumed.contains("v=0 heap=0 file=0 sfile=0"),
        "wrdw: a read-during-write collision now feeds a CONSUMED read — a corrupt BRAM read can propagate into \
         the answer (the RDW hypothesis would no longer be refuted). Consumed summary: {consumed}\n{stdout}"
    );
    let n_collisions = stdout
        .lines()
        .filter(|l| l.contains("RDW-COLLISION"))
        .count();
    let n_consumed = stdout
        .lines()
        .filter(|l| l.starts_with("RDW-CONSUMED ") || l.trim_start().starts_with("RDW-CONSUMED "))
        .count();
    println!("RDW CONFIRM/REFUTE: {summary}  (raw RDW-COLLISION lines: {n_collisions})");
    println!("RDW CONSUMED (propagating): {consumed}  (RDW-CONSUMED lines: {n_consumed})");
    for l in stdout
        .lines()
        .filter(|l| l.contains("RDW-CONSUMED "))
        .take(40)
    {
        println!("  {l}");
    }
    for l in stdout
        .lines()
        .filter(|l| l.contains("RDW-COLLISION"))
        .take(10)
    {
        println!("  {l}");
    }
}

/// ★★★ THE WHOLE STREAMED TRANSFORMER, CFG_BRAM=1, STREAMED OVER THE BIT-BANGED UART ≡VM — the EXACT on-chip
/// path, and the one no other test covers. `whole_streamed_transformer_..._direct_load` proves the interpret FSM
/// is ≡VM but DIRECT-LOADS the config/weights via `$readmemh` (bypassing the loader); `config_bram_preloaded_..`
/// streams over the bit-banged UART loader but with a SMALL program. THIS test is their intersection: the FULL
/// 158-block streamed transformer + the 3136-word weight blob, streamed byte-by-byte over the bit-banged UART
/// (`wire_bytes_preloaded` → serial into `uart_rx` at CLKS_PER_BIT), through the SAME loader FSM the silicon runs
/// (P_CNT→…→P_GO→'H'→P_HN→P_HD [heap pre-load] →P_GO→'5A'→P_INIT→P_FINIT [serial file seed]→P_CLR→P_RUN→P_SEND),
/// at the EXACT on-chip synth parameters (`WITH_FPU(0)` + `CFG_BRAM(1)` — `ser-build-200t.sh`'s `chparam`s). The
/// model generates "mat" = 80113; the chip returned 3. This is the ≡VM reproducer for the on-chip wrong answer.
/// It instruments the load→run boundary (P_CLR→P_RUN): it dumps `heap[0..16]` and the seeded bump pointer `hp`
/// right when the run starts, so if the weights did NOT land, the readback shows exactly where. python3-gated,
/// iverilog-gated, and slow (bit-banging ~53KB + tens of millions of run cycles), so it carries a large deadline.
#[test]
fn full_transformer_over_uart_config_bram_reproduces_the_on_chip_run() {
    fn gen(flags: &[&str]) -> Option<String> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let out = std::process::Command::new("python3")
            .current_dir(&root)
            .arg("scripts/gen_tiny_transformer_lg.py")
            .args(flags)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    if !common::require_tool("python3") {
        return;
    }
    if !common::require_tool("iverilog") {
        return;
    }

    // Build the config + weight blob EXACTLY as the direct-load `wxfmr` test does — same program, same blob, same
    // WPE_STREAM_CAPS, same with_seq_reserve(8), same model golden.
    let src = gen(&["--fabric", "--stream"]).expect("streamed transformer program");
    let blob: Vec<i64> = {
        let s = gen(&["--blob"]).expect("weight blob");
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i64>().expect("blob int"))
            .collect()
    };
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        let (m, _steps, _heap) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (cfg, m)
    });
    assert_eq!(
        model, 80113,
        "the engine model must generate \"mat\" (80113) before we reproduce the on-chip run"
    );

    // The whole thing, streamed over the wire (params EMPTY — generate() takes none — then the 'H' heap segment).
    let im = wprog_config_image_unchecked(&cfg);
    let wire = im.wire_bytes_preloaded(&[], &blob);
    let soc =
        emit_wprog_engine_soc_verilog_backend("uxfmr", WPE_STREAM_CAPS, MemBackend::ConfigBram);

    let dir = std::env::temp_dir().join("logos_wpe_uxfmr");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wire_uxfmr.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    eprintln!(
        "over-the-UART transformer: streaming {} wire bytes ({} weight words), model golden = {model}",
        wire.len(),
        blob.len()
    );

    // A bit-banged-UART host testbench: instantiate the DUT at the EXACT on-chip synth parameters
    // (WITH_FPU(0), CFG_BRAM(1), CLKS_PER_BIT(8)), serialize the whole wire into uart_rx through the SAME
    // send_byte the shared host uses, let the loader FSM run naturally (no forcing), instrument the load→run
    // boundary, then decode the returned frame and compare to the model. Large $finish deadline: bit-banging
    // ~53KB at 8 clk/bit (~4.9M cycles) + tens of millions of run cycles.
    let n_wire = wire.len();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_uxfmr;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  // WITH_FPU(0) + CFG_BRAM(1) — the EXACT on-chip synth parameters (ser-build-200t.sh chparams), so this
  // behavioral run matches the flashed bitstream's RTL configuration, and drive it over the bit-banged UART.
  wpe_soc_uxfmr #(.CLKS_PER_BIT(CLKS), .WITH_FPU(0), .CFG_BRAM(1)) dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;

  reg [7:0] wirebytes [0:MAXB-1];
  integer k, fk, bi;
  reg [7:0] rb, sync, endb;
  reg [63:0] result; reg [31:0] cyc; integer got;
  reg dumped;

  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask

  // Instrument the LOAD -> RUN boundary: the cycle the loader hands off to the interpret FSM (P_CLR done ->
  // P_RUN, ph goes 4'd9 -> 4'd6). Dump the bump pointer, the pre-load count, and heap[0..16] + a few file[]
  // entries, so a weights-not-loaded failure shows EXACTLY what landed (or didn't).
  // P_CLR=4'd9, P_RUN=4'd6, P_FINIT=4'd12, P_HD=4'd11.
  initial dumped = 1'b0;
  always @(posedge clk) begin
    if (!dumped && dut.ph == 4'd6) begin
      dumped <= 1'b1;
      $display("LOAD->RUN: hp(bump)=%0d hpre(preload words)=%0d", dut.hp, dut.hpre);
      $display("  heap[0..15] = %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d",
        $signed({{dut.heap_b0_hi[0], dut.heap_b0_lo[0]}}), $signed({{dut.heap_b0_hi[1], dut.heap_b0_lo[1]}}), $signed({{dut.heap_b0_hi[2], dut.heap_b0_lo[2]}}), $signed({{dut.heap_b0_hi[3], dut.heap_b0_lo[3]}}),
        $signed({{dut.heap_b0_hi[4], dut.heap_b0_lo[4]}}), $signed({{dut.heap_b0_hi[5], dut.heap_b0_lo[5]}}), $signed({{dut.heap_b0_hi[6], dut.heap_b0_lo[6]}}), $signed({{dut.heap_b0_hi[7], dut.heap_b0_lo[7]}}),
        $signed({{dut.heap_b0_hi[8], dut.heap_b0_lo[8]}}), $signed({{dut.heap_b0_hi[9], dut.heap_b0_lo[9]}}), $signed({{dut.heap_b0_hi[10], dut.heap_b0_lo[10]}}), $signed({{dut.heap_b0_hi[11], dut.heap_b0_lo[11]}}),
        $signed({{dut.heap_b0_hi[12], dut.heap_b0_lo[12]}}), $signed({{dut.heap_b0_hi[13], dut.heap_b0_lo[13]}}), $signed({{dut.heap_b0_hi[14], dut.heap_b0_lo[14]}}), $signed({{dut.heap_b0_hi[15], dut.heap_b0_lo[15]}}));
      $display("  heap[tail 3135..3137] = %0d %0d %0d", $signed({{dut.heap_b0_hi[3135], dut.heap_b0_lo[3135]}}), $signed({{dut.heap_b0_hi[3136], dut.heap_b0_lo[3136]}}), $signed({{dut.heap_b0_hi[3137], dut.heap_b0_lo[3137]}}));
      $display("  file[0..3] = %0d %0d %0d %0d",
        {{dut.file_hi[0], dut.file_lo[0]}}, {{dut.file_hi[1], dut.file_lo[1]}},
        {{dut.file_hi[2], dut.file_lo[2]}}, {{dut.file_hi[3], dut.file_lo[3]}});
    end
  end

  initial begin
    $readmemh("wire_uxfmr.hex", wirebytes);
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync===8'hA5 && endb===8'h5A && result==64'd{model})
      $display("PASS: whole transformer streamed over UART generated %0d (mat) on the CFG_BRAM engine in %0d run cycles", got, cyc);
    else
      $display("FAIL: got %0d (sync=%02x end=%02x) exp %0d in %0d run cycles", got, sync, endb, {model}, cyc);
    $finish;
  end
  initial begin #400000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );

    let srcv = dir.join("wpe_soc_uxfmr.v");
    let tbv = dir.join("tb_uxfmr.v");
    let sim = dir.join("sim_uxfmr.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "uxfmr: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    // Print the readback + result unconditionally — this test is a diagnostic reproducer.
    eprintln!("=== uxfmr vvp stdout ===\n{stdout}\n=== uxfmr vvp stderr ===\n{stderr}");
    assert!(stdout.contains("PASS:"), "uxfmr: the CFG_BRAM engine diverged from the model when the whole transformer was STREAMED OVER THE UART (the on-chip path):\n{stdout}");
    println!("WHOLE TRANSFORMER OVER UART ≡ MODEL under CFG_BRAM=1: {stdout}");
}

/// ★★★ THE WHOLE STREAMED TRANSFORMER'S GOLDEN DEBUG FRAME — the silicon-diff reference. This captures the EXACT
/// on-wire DEBUG FRAME the `DEBUG_READBACK=1` engine emits at the load→run boundary for the FULL 158-block
/// streamed transformer, so the host `read-dbg-200t.py` can diff the golden 322 debug bytes byte-for-byte against
/// what the real 200T bitstream echoes and LOCALIZE which config/heap BRAM the synthesis mis-maps. It reuses the
/// EXACT harness of `full_transformer_over_uart_config_bram_reproduces_the_on_chip_run` (same
/// `gen(["--fabric","--stream"])` program + `gen(["--blob"])` weight blob + `with_seq_reserve(8)` + `WPE_STREAM_CAPS`
/// + `wire_bytes_preloaded(&[], &blob)`), but emits the engine via `emit_wprog_engine_soc_verilog_backend_debug`
/// (`DEBUG_READBACK=1`) at the on-chip params `WITH_FPU(0)` + `CFG_BRAM(1)` + `CLKS_PER_BIT(8)`. It streams the
/// whole config+weights over the bit-banged UART, receives the 322-byte DEBUG FRAME + the 14-byte result frame,
/// asserts the sync/end bytes (0xD8/0x8D) and result==80113 (the run still completes with readback on), then
/// WRITES the raw 322 debug-frame bytes to `/tmp/golden_dbg.bin` and a human-readable 5×8 decode to
/// `/tmp/golden_dbg.txt`. python3-gated, iverilog-gated, and slow (bit-banging ~53KB + the full run with readback),
/// so it carries a large deadline.
#[test]
fn full_transformer_debug_readback_captures_golden_frame() {
    fn gen(flags: &[&str]) -> Option<String> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let out = std::process::Command::new("python3")
            .current_dir(&root)
            .arg("scripts/gen_tiny_transformer_lg.py")
            .args(flags)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    if !common::require_tool("python3") {
        return;
    }
    if !common::require_tool("iverilog") {
        return;
    }

    // Build the config + weight blob EXACTLY as the over-the-UART reproducer does — same program, same blob, same
    // WPE_STREAM_CAPS, same with_seq_reserve(8), same model golden.
    let src = gen(&["--fabric", "--stream"]).expect("streamed transformer program");
    let blob: Vec<i64> = {
        let s = gen(&["--blob"]).expect("weight blob");
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i64>().expect("blob int"))
            .collect()
    };
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src, "generate"));
        let (m, _steps, _heap) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (cfg, m)
    });
    assert_eq!(
        model, 80113,
        "the engine model must generate \"mat\" (80113) before we capture the golden debug frame"
    );

    // The whole thing, streamed over the wire (params EMPTY — generate() takes none — then the 'H' heap segment).
    let im = wprog_config_image_unchecked(&cfg);
    let wire = im.wire_bytes_preloaded(&[], &blob);
    // Emit the engine with DEBUG_READBACK=1 (only defined for ConfigBram) — the SAME on-chip params the flashed
    // bitstream uses, plus the readback probe.
    let soc = emit_wprog_engine_soc_verilog_backend_debug(
        "gxfmr",
        WPE_STREAM_CAPS,
        MemBackend::ConfigBram,
        true,
    );
    assert!(
        soc.contains("parameter DEBUG_READBACK = 1") && soc.contains("P_DBG"),
        "DEBUG_READBACK=1 RTL must carry the readback parameter + FSM"
    );

    let dir = std::env::temp_dir().join("logos_wpe_gxfmr");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wire_gxfmr.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    eprintln!(
        "golden debug-frame capture: streaming {} wire bytes ({} weight words), model golden = {model}",
        wire.len(),
        blob.len()
    );

    // The raw 322-byte DEBUG FRAME is written to a stable, iverilog-friendly path in the sim dir, then copied to
    // /tmp/golden_dbg.bin by the Rust side (so the on-wire bytes are byte-exact — no re-encoding).
    let dbg_bin = dir.join("golden_dbg.bin");
    let dbg_bin_str = dbg_bin.to_string_lossy().replace('\\', "/");

    // A bit-banged-UART host testbench: instantiate the DUT at the EXACT on-chip synth parameters
    // (WITH_FPU(0), CFG_BRAM(1), DEBUG_READBACK(1), CLKS_PER_BIT(8)), serialize the whole wire into uart_rx through
    // the SAME send_byte the shared host uses, let the loader FSM run naturally, receive the 322-byte DEBUG FRAME
    // (0xD8 + 5*8*8 + 0x8D) followed by the 14-byte result frame (0xA5 + result[63:0] + cycles[31:0] + 0x5A). The
    // 322 debug-frame bytes are captured EXACTLY as they come off the wire and $fwrite-n raw to golden_dbg.bin.
    let n_wire = wire.len();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_gxfmr;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  localparam NDBGB = 322;   // 1 (0xD8) + 5*8*8 (words) + 1 (0x8D)
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  // WITH_FPU(0) + CFG_BRAM(1) + DEBUG_READBACK(1) — the EXACT on-chip synth parameters plus the readback probe.
  wpe_soc_gxfmr #(.CLKS_PER_BIT(CLKS), .WITH_FPU(0), .CFG_BRAM(1), .DEBUG_READBACK(1))
    dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;

  reg [7:0] wirebytes [0:MAXB-1];
  reg [7:0] dbgbytes  [0:NDBGB-1];
  integer k, fk, bi;
  reg [7:0] rb, sync, endb;
  reg [63:0] result; reg [31:0] cyc; integer got, errs, fd;

  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask

  initial begin
    $readmemh("wire_gxfmr.hex", wirebytes);
    errs = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);

    // ── DEBUG FRAME first: capture ALL 322 bytes EXACTLY as they arrive (0xD8, 320 word bytes, 0x8D). ──
    for (bi=0; bi<NDBGB; bi=bi+1) begin recv_byte(rb); dbgbytes[bi] = rb; end
    if (dbgbytes[0] !== 8'hD8) begin $display("FAIL: debug sync=%02x exp d8", dbgbytes[0]); errs = errs + 1; end
    if (dbgbytes[NDBGB-1] !== 8'h8D) begin $display("FAIL: debug end=%02x exp 8d", dbgbytes[NDBGB-1]); errs = errs + 1; end

    // Write the raw 322 debug-frame bytes to golden_dbg.bin (one byte each, exactly on-wire order).
    fd = $fopen("{dbg_bin_str}", "wb");
    if (fd == 0) begin $display("FAIL: cannot open golden_dbg.bin"); errs = errs + 1; end
    else begin
      for (bi=0; bi<NDBGB; bi=bi+1) $fwrite(fd, "%c", dbgbytes[bi]);
      $fclose(fd);
    end

    // ── RESULT FRAME: 0xA5 + result[63:0] + cycles[31:0] + 0x5A — proves the run still completed after readback. ──
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync!==8'hA5 || endb!==8'h5A || result!=64'd{model}) begin
      $display("FAIL: result frame got %0d (sync=%02x end=%02x) exp %0d in %0d run cycles", got, sync, endb, {model}, cyc);
      errs = errs + 1;
    end

    if (errs == 0)
      $display("PASS: golden debug frame captured (322 bytes, sync d8/end 8d) + the whole transformer returned %0d (mat) in %0d run cycles", got, cyc);
    else
      $display("FAIL: %0d golden-capture errors", errs);
    $finish;
  end
  initial begin #400000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );

    let srcv = dir.join("wpe_soc_gxfmr.v");
    let tbv = dir.join("tb_gxfmr.v");
    let sim = dir.join("sim_gxfmr.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "gxfmr: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    eprintln!("=== gxfmr vvp stdout ===\n{stdout}\n=== gxfmr vvp stderr ===\n{stderr}");
    assert!(
        stdout.contains("PASS:"),
        "gxfmr: the golden debug-frame capture failed:\n{stdout}"
    );

    // Read the raw 322 debug-frame bytes the testbench captured, copy them to the canonical /tmp/golden_dbg.bin,
    // and decode the human-readable 5×8 table to /tmp/golden_dbg.txt. The 5 memories, in the DEBUG FRAME order.
    let raw = std::fs::read(&dbg_bin).expect("golden_dbg.bin written by the testbench");
    assert_eq!(
        raw.len(),
        322,
        "the golden debug frame must be exactly 322 bytes on the wire, got {}",
        raw.len()
    );
    assert_eq!(
        raw[0], 0xD8,
        "golden debug frame must start with the 0xD8 sync byte"
    );
    assert_eq!(
        raw[321], 0x8D,
        "golden debug frame must end with the 0x8D end byte"
    );
    std::fs::write("/tmp/golden_dbg.bin", &raw).expect("write /tmp/golden_dbg.bin");

    // Decode: skip the sync byte, then 5 memories × 8 signed 64-bit big-endian words (8 bytes each), then the end.
    let mem_names = ["cell_kind", "cell_op0", "cell_op1", "blk_term_kind", "heap"];
    let mut off = 1usize;
    let mut txt = String::new();
    let mut memvals: Vec<(&str, Vec<i64>)> = Vec::new();
    for name in mem_names {
        let mut vals = Vec::with_capacity(8);
        for _ in 0..8 {
            let mut w = [0u8; 8];
            w.copy_from_slice(&raw[off..off + 8]);
            vals.push(i64::from_be_bytes(w));
            off += 8;
        }
        let line = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        txt.push_str(&format!("{name}: {line}\n"));
        memvals.push((name, vals));
    }
    assert_eq!(
        off, 321,
        "the 5×8 words must consume bytes 1..321, ended at {off}"
    );
    std::fs::write("/tmp/golden_dbg.txt", &txt).expect("write /tmp/golden_dbg.txt");

    println!("GOLDEN DEBUG FRAME CAPTURED: 322 bytes -> /tmp/golden_dbg.bin, decode -> /tmp/golden_dbg.txt");
    for (name, vals) in &memvals {
        println!("  {name}: {:?}", vals);
    }
    println!("{}", stdout.trim());
}

#[test]
fn every_memory_backend_runs_byte_identical_to_the_vm() {
    let inputs = [0i64, 1, 5, 9, 17];
    for (tag, backend) in [
        ("reg", MemBackend::Registers),
        ("bram", MemBackend::Bram),
        ("cbram", MemBackend::ConfigBram),
    ] {
        run_backend(tag, backend, &inputs);
    }
    println!("SELECTABLE BACKEND ≡VM: the same program runs byte-identical to the VM on all three memory backends (Registers / Bram / ConfigBram) over the wire.");
}

/// f(n) = if n<=0 {0} else n + f(n-1) — a genuine recursive `Call` config (exercises the engine's frame stack:
/// interpret Call → save/push the caller register file, Return → pop + restore). The `f(x)` loop program above
/// never calls, so this is the ONLY thing that drives the CFG_BRAM=1 frame-save / call-arg-gather BRAM read
/// paths (ST_FSD/ST_FSW read `file` through the registered port; ST_AGD/ST_AGW read `v` through it).
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

/// Stream `cfg` + each input over a simulated UART into the engine emitted with `backend`, and assert every
/// result matches `eval_wprog_config` (≡ VM) under iverilog. The config-driven sibling of [`run_backend`]
/// (which is fixed to the `f(x)` loop source) — used to drive a recursive `Call` config through ConfigBram.
fn run_cfg_backend(tag: &str, backend: MemBackend, cfg: &WProgConfig, inputs: &[i64]) {
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&x| eval_wprog_config(cfg, &[x]))
        .collect();
    let soc = emit_wprog_engine_soc_verilog_backend(tag, CAPS, backend);
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_wpe_backend");
    std::fs::create_dir_all(&dir).unwrap();
    let im = wprog_config_image(cfg);
    let (mut all, mut lens) = (Vec::<u8>::new(), Vec::<u32>::new());
    for &x in inputs {
        let b = im.wire_bytes(&[x]);
        lens.push(b.len() as u32);
        all.extend_from_slice(&b);
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
    let tb = emit_wprog_engine_soc_testbench(tag, inputs.len(), CLKS_PER_BIT, all.len());
    let srcv = dir.join(format!("wpe_soc_{tag}.v"));
    let tbv = dir.join(format!("tb_wpe_soc_{tag}.v"));
    let sim = dir.join(format!("sim_{tag}.vvp"));
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "{tag}: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "{tag}: engine diverged from eval_wprog_config:\n{stdout}"
    );
}

/// ★ CFG_BRAM=1 RECURSION ≡VM — the register file `file` and cell-value array `v` are now BLOCK RAM under
/// ConfigBram (the routing fix), read through the registered file_rd/v_rd ports. A recursive `Call` is the only
/// program that drives the frame-save (ST_FSD/ST_FSW: `sfile[..] <= file_rd`) and call-arg-gather
/// (ST_AGD/ST_AGW: `file[ai] <= v_rd`) BRAM-read paths — the loop program in `every_memory_backend_*` exercises
/// only operands/writes/terminator. This proves the whole frame stack over the wire is byte-identical to the VM
/// with `file`/`v` in block RAM, so the transformer's calls stay correct once the config image routes on the 200T.
#[test]
fn config_bram_runs_recursion_byte_identical_to_the_vm() {
    let cfg = recursive_call_config();
    let inputs: [i64; 4] = [0, 1, 3, 6]; // f(n)=Σ 1..n; depth ≤ 6 fits the synth DEPTH=8 frame stack
                                         // Sanity: the model's frame-stack semantics (matches the VM's recursion).
    assert_eq!(
        eval_wprog_config(&cfg, &[6]),
        21,
        "f(6) must be 1+2+3+4+5+6"
    );
    run_cfg_backend("cbram_rec", MemBackend::ConfigBram, &cfg, &inputs);
    println!("CONFIG-BRAM RECURSION ≡VM: f(n)=n+f(n-1) ran on the engine's frame stack with the register file AND cell-value array in BLOCK RAM (file_rd/v_rd registered reads), byte-identical to eval_wprog_config ≡ VM over the wire.");
}

/// The substrate is chosen per memory class from the backend — not hard-wired. A memory may become block RAM
/// ONLY if it has a registered read (yosys cannot map a combinational-read array to a synchronous RAMB): the
/// state plane (`heap`/`sfile`, registered via `heap_rd`/`sf_rd`) and, under ConfigBram, the registered-read
/// config memories `cell_*` (via the `ST_CF` cell-fetch wait-state), `wr_*` (via the `ST_WF` write-fetch
/// wait-state), and the 13 `blk_*` block-record arrays (via the `ST_BF` block-fetch wait-state). The remaining
/// config memories (`arg_op`/`liv_reg`/`konst`) are still read combinationally, so they stay flip-flops under
/// every backend — a block tag on them would produce an unsynthesizable netlist. Moving `cell_*`/`wr_*`/`blk_*`
/// off flip-flops is what shrinks the config read muxes enough for the streamed transformer to place+route on
/// the 200T under the open flow; a registered read for `arg_op` is the remaining follow-on.
#[test]
fn the_backend_selects_each_memory_substrate_not_hardwired() {
    let reg = emit_wprog_engine_soc_verilog_backend("s_reg", CAPS, MemBackend::Registers);
    let bram = emit_wprog_engine_soc_verilog_backend("s_bram", CAPS, MemBackend::Bram);
    let cbram = emit_wprog_engine_soc_verilog_backend("s_cbram", CAPS, MemBackend::ConfigBram);

    // The register file `file`, cell-value array `v`, and the recursion frame stack `sfile` are each SPLIT into
    // two 32-bit halves (`*_lo`/`*_hi`) so none maps to a single 64-bit RAMB36 in SDP-72 mode — whose parity-lane
    // write yosys 0.33 mis-wires (heap reproduced on real Artix-7 silicon, the rest at gate level). The heap is
    // ADDITIONALLY depth-banked into two 16384-deep banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address bit
    // 14) so it never maps to a RAMB36E1 in ×1-width, 32768-deep mode — the internal two-16K-half depth-cascade
    // prjxray/openXC7 mis-encodes on the XC7A200T. Each half of each bank must carry the SAME substrate the
    // un-split array had; asserting on every half is the substrate-tracking invariant AND the anti-regression guard.
    // Registers: everything in flip-flops.
    assert_eq!(style_of(&reg, "cell_kind"), "registers");
    for half in [
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
    ] {
        assert_eq!(
            style_of(&reg, half),
            "registers",
            "{half} must be flip-flops under Registers"
        );
    }
    // Bram: state plane → block RAM, config image → flip-flops. The register file `file` and cell-value `v` halves
    // stay flip-flops here — their block-RAM registered read exists ONLY under ConfigBram (Bram reads them
    // combinationally). The heap banks and frame-stack `sfile` halves are state plane → block RAM.
    for half in ["heap_b0_lo", "heap_b0_hi", "heap_b1_lo", "heap_b1_hi"] {
        assert_eq!(
            style_of(&bram, half),
            "block",
            "{half} must be block RAM under Bram (state plane)"
        );
    }
    assert_eq!(style_of(&bram, "sfile_lo"), "block");
    assert_eq!(style_of(&bram, "sfile_hi"), "block");
    assert_eq!(style_of(&bram, "cell_kind"), "registers");
    assert_eq!(style_of(&bram, "blk_cell_ptr"), "registers");
    for half in ["file_lo", "file_hi", "v_lo", "v_hi"] {
        assert_eq!(
            style_of(&bram, half),
            "registers",
            "{half} must be flip-flops under Bram (combinational reads there)"
        );
    }
    // ConfigBram: state plane AND every registered-read config memory (`cell_*`/`wr_*`/`blk_*`) → block RAM;
    // the still-combinational config memories stay flip-flops (a block tag on them would not map to RAMB).
    for half in ["heap_b0_lo", "heap_b0_hi", "heap_b1_lo", "heap_b1_hi"] {
        assert_eq!(
            style_of(&cbram, half),
            "block",
            "{half} must be block RAM under ConfigBram (state plane)"
        );
    }
    assert_eq!(style_of(&cbram, "sfile_lo"), "block");
    assert_eq!(style_of(&cbram, "sfile_hi"), "block");
    for cell in [
        "cell_kind",
        "cell_code",
        "cell_op0",
        "cell_op1",
        "cell_op2",
        "cell_op3",
    ] {
        assert_eq!(
            style_of(&cbram, cell),
            "block",
            "{cell} must be block RAM under ConfigBram (it has the registered ST_CF read)"
        );
    }
    for wr in ["wr_reg", "wr_op"] {
        assert_eq!(
            style_of(&cbram, wr),
            "block",
            "{wr} must be block RAM under ConfigBram (it has the registered ST_WF read)"
        );
    }
    for blk in [
        "blk_livein_ptr",
        "blk_n_livein",
        "blk_cell_ptr",
        "blk_n_cells",
        "blk_write_ptr",
        "blk_n_writes",
        "blk_term_kind",
        "blk_t0",
        "blk_t1",
        "blk_top",
        "blk_cdst",
        "blk_cargc",
        "blk_cargp",
    ] {
        assert_eq!(
            style_of(&cbram, blk),
            "block",
            "{blk} must be block RAM under ConfigBram (it has the registered ST_BF read)"
        );
    }
    assert_eq!(
        style_of(&cbram, "liv_reg"),
        "registers",
        "liv_reg deliberately stays flip-flops (keeps resolve unchanged, per the plan)"
    );
    // The register file `file` and per-block cell-value array `v` (both 32-bit halves) are the routing fix: block
    // RAM under ConfigBram (registered file_rd/v_rd reads via ST_OP/ST_WR/ST_TR/ST_FS/ST_AG fetch waits),
    // dissolving the 128:1×64 operand-mux hub the open router could not legalize. `file0` (the CFG_BRAM=0 write
    // snapshot, DEAD under ConfigBram, a single 64-bit array) always stays flip-flops.
    for half in ["file_lo", "file_hi"] {
        assert_eq!(
            style_of(&cbram, half),
            "block",
            "`{half}` must be block RAM under ConfigBram (registered file_rd read)"
        );
    }
    for half in ["v_lo", "v_hi"] {
        assert_eq!(
            style_of(&cbram, half),
            "block",
            "`{half}` must be block RAM under ConfigBram (registered v_rd read)"
        );
    }
    assert_eq!(
        style_of(&cbram, "file0"),
        "none",
        "`file0` (dead under ConfigBram) carries no ram_style tag — yosys prunes it"
    );
    println!("NO HARD-WIRING: each memory class gets a synthesizable substrate — block RAM only where a registered read exists (heap + sfile_lo/hi + cell_*/wr_*/blk_* + file_lo/hi/v_lo/hi under ConfigBram), flip-flops otherwise.");
}

/// POST-SYNTH gate-level reproduction harness (fallback for the 4.3M-clock full-transformer stream, which is too
/// slow to bit-bang at gate level): dump a SMALL ConfigBram program's wire stream + its known DEBUG-FRAME image
/// slices to `/tmp/psim_small/`, so a synthesized netlist (same WPE_STREAM_CAPS ⇒ identical RAMB tiling) can be
/// gate-level simulated over the small stream and its post-synth readback diffed against these golden slices.
/// The heap is pre-loaded with 520 words (> 512) so the write path spans BEYOND the first SDP-72 heap RAMB tile
/// (512 words fill tile 0), forcing the multi-RAMB heap tiling the transformer relies on — while heap[0..8) (the
/// DEBUG-frame window) reads distinct values so a first-tile read mis-map is unambiguous. Same `XFMR_SHAPED` +
/// `with_seq_reserve(8)` config as the golden capture, so cell_*/blk_* tile identically.
#[test]
fn emit_small_heap_tiling_debug_stream_for_postsynth_sim() {
    let mut heap_init: Vec<i64> = vec![520]; // loaded()[0] = length
    for k in 0..520i64 {
        heap_init.push(1000 + k); // 520 distinct weight words; heap[1..9) = 1001..1008 in the frame window
    }
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(XFMR_SHAPED, "acc"));
        let (m, _s, _h) = eval_wprog_config_counted_heap_preloaded(&cfg, &[2], &heap_init);
        (cfg, m)
    });

    let im = wprog_config_image(&cfg);
    let slice8 = |v: &[u32]| -> Vec<i64> {
        (0..8)
            .map(|i| v.get(i).copied().unwrap_or(0) as i64)
            .collect()
    };
    let heap8: Vec<i64> = (0..8)
        .map(|i| heap_init.get(i).copied().unwrap_or(0))
        .collect();
    let expected: Vec<(&str, Vec<i64>)> = vec![
        ("cell_kind", slice8(&im.cell_kind)),
        ("cell_op0", slice8(&im.cell_op0)),
        ("cell_op1", slice8(&im.cell_op1)),
        ("blk_term_kind", slice8(&im.blk_term_kind)),
        ("heap", heap8),
    ];

    let wire = im.wire_bytes_preloaded(&[2], &heap_init);
    let dir = std::path::PathBuf::from("/tmp/psim_small");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wire_small.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let mut golden = String::new();
    for (name, vals) in &expected {
        golden.push_str(&format!(
            "{name}: {}\n",
            vals.iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    std::fs::write(dir.join("golden_small.txt"), &golden).unwrap();
    std::fs::write(dir.join("nwire_small.txt"), format!("{}\n", wire.len())).unwrap();
    eprintln!(
        "SMALL heap-tiling stream: {} wire bytes ({} heap words > 512 ⇒ multi-RAMB tiling), model={model}\n{golden}",
        wire.len(), heap_init.len()
    );
}

/// PHASE-1 NON-DEBUG RESULT-frame reproduction: the same `XFMR_SHAPED`/`acc` program + the same 520-word heap
/// preload as `emit_small_heap_tiling_debug_stream_for_postsynth_sim`, but streamed into the NORMAL engine
/// (`DEBUG_READBACK=0`, `MemBackend::ConfigBram`) and checked on the RESULT FRAME (0xA5 + result[63:0] +
/// cycles[31:0] + 0x5A) rather than the confounded debug frame. `acc(2)` READS `heap[1]` and `heap[2]`
/// (`item (i+1) of loaded()`) across two loop iterations and returns a value computed from them, so a mis-map of
/// the engine's OWN compute read port (`heap_ra`) — the one the transformer rides, distinct from the debug read
/// port that induced BRAM replication — corrupts the RESULT. This emits (a) the non-debug ConfigBram RTL to
/// `/tmp/psim/wpe_soc_stream_nodbg.v`, (b) the small heap-read wire + RESULT golden to `/tmp/psim_small/`, and
/// runs the BEHAVIORAL result-frame sim to establish the ≡VM baseline the gate-level netlist is diffed against.
/// The gate-level run (over the synthesized netlist + `ramb_behav.v`) is driven by the /tmp/psim harness.
#[test]
fn emit_nodebug_heap_read_result_stream_for_postsynth_sim() {
    // Heap preload: 520 words (> 512 ⇒ multi-RAMB tiling), heap[1..]=1001.. so the frame window is distinct.
    let mut heap_init: Vec<i64> = vec![520];
    for k in 0..520i64 {
        heap_init.push(1000 + k);
    }
    let (cfg, model) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(XFMR_SHAPED, "acc"));
        let (m, _s, _h) = eval_wprog_config_counted_heap_preloaded(&cfg, &[2], &heap_init);
        (cfg, m)
    });

    // Emit the NORMAL (DEBUG_READBACK=0) ConfigBram engine — the exact netlist a route+flash uses.
    let soc = emit_wprog_engine_soc_verilog_backend(
        "stream_nodbg",
        WPE_STREAM_CAPS,
        MemBackend::ConfigBram,
    );
    assert!(
        !soc.contains("DEBUG_READBACK") && !soc.contains("P_DBG") && !soc.contains("dbg_"),
        "the non-debug engine must carry NO readback text (no extra debug read port to confound the mis-map)"
    );
    assert!(
        soc.contains("parameter CFG_BRAM = 1"),
        "ConfigBram engine must default CFG_BRAM=1"
    );

    let psim = std::path::PathBuf::from("/tmp/psim");
    std::fs::create_dir_all(&psim).unwrap();
    std::fs::write(psim.join("wpe_soc_stream_nodbg.v"), &soc).unwrap();

    let im = wprog_config_image(&cfg);
    let wire = im.wire_bytes_preloaded(&[2], &heap_init);
    let dir = std::path::PathBuf::from("/tmp/psim_small");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wire_result.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(dir.join("golden_result.txt"), format!("result: {model}\n")).unwrap();
    std::fs::write(dir.join("nwire_result.txt"), format!("{}\n", wire.len())).unwrap();
    eprintln!(
        "NON-DEBUG heap-read RESULT stream: {} wire bytes ({} heap words > 512 ⇒ multi-RAMB tiling), RESULT golden = {model}",
        wire.len(),
        heap_init.len()
    );

    if !common::require_tool("iverilog") {
        return;
    }

    // Behavioral RESULT-frame sim (the ≡VM baseline the gate-level netlist is diffed against): stream the wire,
    // receive 0xA5 + result[63:0] + cycles[31:0] + 0x5A, and assert result == the VM golden.
    let n_wire = wire.len();
    let simdir = std::env::temp_dir().join("logos_wpe_result_nodbg");
    std::fs::create_dir_all(&simdir).unwrap();
    std::fs::write(
        simdir.join("wire_result.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_result_nodbg;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  wpe_soc_stream_nodbg #(.CLKS_PER_BIT(CLKS), .WITH_FPU(0), .CFG_BRAM(1))
    dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;

  reg [7:0] wirebytes [0:MAXB-1];
  integer k, fk, bi, errs;
  reg [7:0] rb, sync, endb;
  reg [63:0] result; reg [31:0] cyc; integer got;

  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask
  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask

  initial begin
    $readmemh("wire_result.hex", wirebytes);
    errs = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync!==8'hA5 || endb!==8'h5A || result!=64'd{model}) begin
      $display("FAIL: result frame got %0d (sync=%02x end=%02x) exp %0d in %0d cycles", got, sync, endb, {model}, cyc);
      errs = errs + 1;
    end
    if (errs == 0) $display("PASS: non-debug engine returned %0d (heap-read acc) in %0d run cycles", got, cyc);
    else $display("FAIL: %0d result errors", errs);
    $finish;
  end
  initial begin #40000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );
    let srcv = simdir.join("wpe_soc_stream_nodbg.v");
    let tbv = simdir.join("tb_result_nodbg.v");
    let sim = simdir.join("sim_result_nodbg.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&simdir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "result_nodbg: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&simdir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "result_nodbg: behavioral RESULT sim diverged from VM:\n{stdout}"
    );
    println!("NON-DEBUG RESULT ≡VM (behavioral): {}", stdout.trim());
}

/// A MINIMAL program that provokes the yosys heap depth-cascade mis-map with a tiny stream. It allocates three
/// collections (default `SEQ_RESERVE=2048`), so the third's `Push` lands at `heap[4120]` — physical row 24 under
/// the buggy 4096-word-tile truncation (`4120 & 4095 = 24`). `heap[24]` is NOT preloaded, so the VM (and a
/// correctly-mapped netlist) reads 0; a netlist that drops the top heap-address bits aliases the `Push` into row
/// 24 and reads back `999`. Reading `item 24 of loaded()` (= `heap[24]`) therefore yields 0 on a faithful netlist
/// and 999 on the mis-mapped one — a clean divergence with only a ~21-word heap preload (a few-hundred-byte
/// stream, gate-level-simulatable in minutes, unlike a multi-thousand-word high-address preload).
const HIREAD_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let a be a new Seq of Int.
    Let b be a new Seq of Int.
    Let c be a new Seq of Int.
    Push 999 to c.
    Return item 24 of loaded().
";

/// PHASE-1 HEAP-COLLISION RESULT stream: the decisive non-confounded repro. The 520-word small stream stays under
/// 4096 (so it can NOT exercise the depth-cascade truncation); this one makes a `Push` land at `heap[4120]` and
/// reads its truncation-alias `heap[24]`, so a netlist that drops the top heap-address bits returns 999 while the
/// VM returns 0. It emits the non-debug ConfigBram RTL and the minimal wire + RESULT golden to `/tmp/psim_hi/`,
/// then runs the BEHAVIORAL result-frame sim to lock the ≡VM baseline. The gate-level run over the synthesized
/// netlist (buggy vs fixed) is driven by the /tmp/psim harness: buggy RESULT=999 (bug reproduced), fixed RESULT=0.
#[test]
fn emit_high_address_heap_read_result_stream_for_postsynth_sim() {
    // A small heap preload (21 words) — keeps the stream tiny; heap[24] (the collision-alias row) stays 0.
    let mut heap_init: Vec<i64> = vec![20];
    for k in 0..20i64 {
        heap_init.push(50 + k);
    }
    // DEFAULT SEQ_RESERVE (2048) so the third collection's Push lands at heap[4120] (row 24 under truncation).
    let (cfg, model) = {
        let cfg = serialize_program(&program_cfg_fabric(HIREAD_SRC, "acc"));
        let (m, _s, _h) = eval_wprog_config_counted_heap_preloaded(&cfg, &[0], &heap_init);
        (cfg, m)
    };
    assert_eq!(
        model, 0,
        "heap[24] is unpreloaded, so the VM/faithful-netlist result is 0 (buggy returns 999)"
    );

    let soc = emit_wprog_engine_soc_verilog_backend(
        "stream_nodbg",
        WPE_STREAM_CAPS,
        MemBackend::ConfigBram,
    );
    let im = wprog_config_image(&cfg);
    let wire = im.wire_bytes_preloaded(&[0], &heap_init);

    let dir = std::path::PathBuf::from("/tmp/psim_hi");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("wire_hi.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(dir.join("golden_hi.txt"), format!("result: {model}\n")).unwrap();
    std::fs::write(dir.join("nwire_hi.txt"), format!("{}\n", wire.len())).unwrap();
    std::fs::copy(
        dir.join("wire_hi.hex"),
        std::path::PathBuf::from("/tmp/psim/wire_hi.hex"),
    )
    .ok();
    eprintln!(
        "HEAP-COLLISION RESULT stream: {} wire bytes ({} heap words; Push lands at heap[4120], reads alias heap[24]), golden = {model}",
        wire.len(),
        heap_init.len()
    );

    if !common::require_tool("iverilog") {
        return;
    }
    let n_wire = wire.len();
    let simdir = std::env::temp_dir().join("logos_wpe_hi");
    std::fs::create_dir_all(&simdir).unwrap();
    std::fs::write(
        simdir.join("wire_hi.hex"),
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let tb = format!(
        r#"`timescale 1ns/1ps
module tb_hi;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  wpe_soc_stream_nodbg #(.CLKS_PER_BIT(CLKS), .WITH_FPU(0), .CFG_BRAM(1))
    dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;
  reg [7:0] wirebytes [0:MAXB-1];
  integer k, fk, bi, errs;
  reg [7:0] rb, sync, endb;
  reg [63:0] result; reg [31:0] cyc; integer got;
  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask
  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask
  initial begin
    $readmemh("wire_hi.hex", wirebytes);
    errs = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    got = result;
    if (sync!==8'hA5 || endb!==8'h5A || result!=64'd{model}) begin
      $display("FAIL: result frame got %0d (sync=%02x end=%02x) exp %0d in %0d cycles", got, sync, endb, {model}, cyc);
      errs = errs + 1;
    end
    if (errs == 0) $display("PASS: high-address engine returned %0d in %0d run cycles", got, cyc);
    else $display("FAIL: %0d result errors", errs);
    $finish;
  end
  initial begin #40000000000 $display("FAIL: timeout"); $finish; end
endmodule
"#
    );
    let srcv = simdir.join("wpe_soc_stream_nodbg.v");
    let tbv = simdir.join("tb_hi.v");
    let sim = simdir.join("sim_hi.vvp");
    std::fs::write(&srcv, &soc).unwrap();
    std::fs::write(&tbv, &tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&simdir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "hi: iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&simdir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "hi: behavioral RESULT sim diverged from VM:\n{stdout}"
    );
    println!("HIGH-ADDRESS RESULT ≡VM (behavioral): {}", stdout.trim());
}

/// PIT-OF-SUCCESS lock: the streamed engine's heap depth MUST stay single-RAMB-tile-safe on the open flow.
/// yosys 0.33 `synth_xilinx` mis-maps a block-RAM heap whose depth is NOT a clean single-tile fit: it picks a
/// wide RAMB tile (e.g. ×9 → 4096 deep) and then FAILS to depth-cascade, wiring every tile to the low 12 address
/// bits and dropping the top bits — so any heap read/write to an address >= the tile depth aliases into the first
/// tile and returns garbage on silicon (the whole-program transformer returned the wrong result on the XC7A200T).
/// A single RAMB36E1 at ×1 width is exactly 2^15 = 32768 words, addressed by the full 15-bit `heap_ra` with NO
/// cascade — so a heap that is a POWER OF TWO and <= 32768 maps to one correctly-addressed tile. This guard fails
/// LOUDLY the moment a growing model bumps `WPE_STREAM_HEAP` off that safe set, before it can silently mis-map on
/// the chip. (If the model ever needs > 32768 heap words, the real fix is depth-banking the heap in RTL — see the
/// `WPE_STREAM_HEAP` doc-comment — not just widening this bound.)
#[test]
fn stream_heap_depth_is_single_ramb_tile_safe() {
    assert!(
        WPE_STREAM_HEAP.is_power_of_two(),
        "WPE_STREAM_HEAP={WPE_STREAM_HEAP} must be a power of two so yosys maps it to a single, fully-addressed \
         RAMB depth-tile (a non-tile-fit depth triggers the ×9/4096 depth-cascade mis-map)"
    );
    assert!(
        WPE_STREAM_HEAP <= 32768,
        "WPE_STREAM_HEAP={WPE_STREAM_HEAP} exceeds one RAMB36E1 (2^15=32768 words at ×1) — it would need depth \
         cascading, which yosys 0.33 mis-maps; depth-bank the heap in RTL before growing past 32768"
    );
}
