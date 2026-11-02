//! wpe_postsynth_silicon_lock — the POST-SYNTHESIS gate-level QA lock that would have caught the two yosys-0.33
//! `synth_xilinx` BRAM mis-synthesis bugs BEFORE they reached physical silicon (the whole-transformer engine
//! returned garbage on an XC7A200T while every behavioral ≡VM lock stayed green).
//!
//! WHY THIS TEST EXISTS. Every other engine lock (`wpe_backend_lock`, `wprog_engine_soc_lock`) runs iverilog on
//! the ENGINE RTL — where the memories are behavioral `reg` arrays — so it validates RTL BEHAVIOR and never the
//! SYNTHESIZED NETLIST. Both silicon bugs live entirely in yosys's BRAM mapping, invisible to RTL sim:
//!   1. HEAP DEPTH-CASCADE. A heap deeper than a 4096-word BRAM tile makes yosys drop the top address bits
//!      (`heap_ra[14:12]`) so any access >= 4096 aliases into tile 0. FIX = `WPE_STREAM_HEAP = 32768` (2^15 = one
//!      clean RAMB36 depth-tile at ×1, addressed by the whole 15-bit `heap_ra`, no yosys-level depth-cascade).
//!      A FURTHER, prjxray/openXC7-LEVEL defect (BELOW the netlist — the gate-level sim here stays green) mis-
//!      encodes that RAMB36 ×1 32768-deep tile's INTERNAL two-16K-half cascade on the XC7A200T, so the heap is
//!      now DEPTH-BANKED in RTL into two 16384-deep banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address bit
//!      14) → a 14-bit-indexed tile with NO 32K depth-cascade, a better-fuzzed primitive.
//!   2. SDP-72 PARITY MIS-WIRE. A 64-bit-wide array mapping to one RAMB36E1 in SDP-72 mode has its parity-lane
//!      write (DIPBDIP) mis-wired, so the 8 parity bits are lost and any value >= 256 reads back corrupted. FIX =
//!      splitting `file`/`v`/`heap`/`sfile` into two 32-bit halves (`*_lo`/`*_hi`), so each maps TDP-W36 — never
//!      SDP-72 — and no parity lane is used.
//!
//! HOW IT CATCHES THEM. This lock runs the REAL yosys flow (`synth_xilinx -family xc7`), then GATE-LEVEL
//! simulates the SYNTHESIZED NETLIST against a FAITHFUL behavioral RAMB36E1/RAMB18E1 model
//! (`assets/ramb_behav.v`: every TDP width and SDP-72/36 over the device's own data and parity layout — the parity
//! lanes are what make bug #2 observable — powering up as the device does, and stopping the sim on any configuration
//! it does not implement) plus yosys's own generic sim cells (with the two RAMB modules stripped, so our
//! faithful model provides them), and asserts the gate-level result == `eval_wprog_config` (≡ the VM). If yosys
//! mis-maps a heap tile or a parity lane, the netlist diverges from the VM and the lock goes RED — exactly the
//! signal that never fired before silicon.
//!
//! TEETH. `synth_and_gatelevel_result` takes an explicit heap-depth override, so
//! `postsynth_reverting_the_heap_fix_diverges_from_the_vm` re-synthesizes the SAME program at the buggy
//! non-power-of-two heap depth (20480) and asserts the gate-level result DIVERGES from the VM — proving the lock
//! actually catches the reverted fix, not just that today's tree happens to be green.
//!
//! This is SLOW and heavy: each case runs a full `synth_xilinx` (~minutes) then a gate-level sim of a
//! ~90-RAMB/tens-of-thousands-of-LUT netlist (~many minutes). It is gated on `yosys` + `iverilog` being present
//! and skips gracefully (with an eprintln!) when either is missing — it never silently passes. Run it
//! single-threaded so the heavy sims never overlap:
//!
//!   cargo test -p logicaffeine-tests --test wpe_postsynth_silicon_lock -- --nocapture --test-threads=1

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_backend, eval_wprog_config,
    eval_wprog_config_counted_heap_preloaded, eval_wprog_config_run, serialize_program,
    wprog_config_image_unchecked, MemBackend, WPE_STREAM_CAPS, WPE_STREAM_HEAP,
};

/// The faithful behavioral RAMB36E1/RAMB18E1 (TDP ×1/2/4/9/18/36 + SDP-72/36, READ_FIRST, data and parity laid out
/// as the device lays them). Committed as a reviewable repo asset — its SDP-72 parity-write model is exactly what
/// makes bug #2 observable at gate level. Embedded here so the test is self-contained (no dependency on any /tmp
/// scratch).
const RAMB_BEHAV_V: &str = include_str!("assets/ramb_behav.v");

/// The gate-level sim streams the wire at `CLKS_PER_BIT = 2` — the fewest clocks per UART bit the loader FSM
/// tolerates, so the (slow) netlist sim spends as few gate-evaluations as possible per streamed byte.
const CLKS_PER_BIT: usize = 2;

/// MEASURED on this box (2026-09-30): the slowest core class a gate-level sim runs on, a 4.4 GHz efficiency core,
/// simulates the engine netlist at 14 cycles/s (heaptile on CPU 31: 34,000 cycles in 2,396 s), and synthesis plus the
/// netlist compile take 572 s there (yosys 212 s, iverilog 360 s). A test's worst-case wall time is each of its sims'
/// cycle budget at that rate plus that overhead, and the `^postsynth_` cap in `.config/nextest.toml` must cover it.
const SLOWEST_GATE_CYCLES_PER_S: u64 = 14;
const SYNTH_AND_COMPILE_S: u64 = 600;

/// Every gate-level sim builds the RAMB model with `RAMB_POWERUP_RANDOM` at this seed. An ASIC's SRAM powers up as
/// garbage, so every row and output latch starts at a defined pseudo-random word, and a result that a dropped or
/// misplaced write would leave at the power-up value cannot match the VM by coincidence.
const RAMB_POWERUP_SEED: u32 = 20260930;

/// The iverilog defines every gate-level sim is built with.
fn gatelevel_defines() -> Vec<String> {
    vec![format!("RAMB_POWERUP_RANDOM={RAMB_POWERUP_SEED}")]
}

/// This test process's worst-case wall time so far (nextest runs each test in its own process).
static WORST_CASE_WALL_S: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The `^postsynth_` wall cap of the `full` profile the silicon runner uses: `period` × `terminate-after`.
fn postsynth_wall_cap_s() -> u64 {
    let toml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/nextest.toml"),
    )
    .expect("read .config/nextest.toml");
    let block = toml
        .split("[[")
        .find(|b| {
            b.starts_with("profile.full.overrides]]")
                && b.contains("filter = 'binary(wpe_postsynth_silicon_lock) & test(/^postsynth_/)'")
        })
        .expect("the full profile has a ^postsynth_ override");
    let line = block
        .lines()
        .find(|l| l.trim_start().starts_with("slow-timeout"))
        .expect("the override sets slow-timeout");
    let num = |key: &str| -> u64 {
        let rest = &line[line
            .find(key)
            .unwrap_or_else(|| panic!("no {key} in {line}"))
            + key.len()..];
        rest.trim_start_matches(|c: char| c == ' ' || c == '=' || c == '"')
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or_else(|_| panic!("unreadable {key} in {line}"))
    };
    num("period") * num("terminate-after")
}

/// The cycles a gate-level sim is given: the most a correct run takes, with a quarter's margin.
fn cycle_budget(expected: u64) -> u64 {
    expected + expected / 4 + 2_000
}

#[path = "common/mod.rs"]
mod common;

/// The private build dir this lock owns. Defaults to a PID-unique path so two concurrent test processes never
/// clobber each other's per-tag netlists/sims (env propagation through wrappers is unreliable); `WPE_POSTSYNTH_DIR`
/// still overrides it when a fixed path is wanted. Kept off `/tmp/psim`, which another tool uses.
fn build_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("WPE_POSTSYNTH_DIR")
            .unwrap_or_else(|_| format!("/tmp/psynthlock_{}", std::process::id())),
    )
}

/// yosys's generic Xilinx sim cells (LUTs, FDRE, carry, muxes, …) with the two RAMB modules (`RAMB18E1`,
/// `RAMB36E1`) STRIPPED — so our faithful `ramb_behav.v` provides them instead (no duplicate-module clash). The
/// two RAMB modules are the LAST two in yosys's `cells_sim.v` (contiguous, at the tail), so keeping everything
/// before `module RAMB18E1` is the whole strip. Returns None (graceful skip) if the yosys datadir isn't found.
fn cells_sim_noramb() -> Option<String> {
    // The canonical distro path; kept simple + explicit (this test is Linux/CI-only, gated on yosys anyway).
    for p in [
        "/usr/share/yosys/xilinx/cells_sim.v",
        "/usr/local/share/yosys/xilinx/cells_sim.v",
    ] {
        if let Ok(full) = std::fs::read_to_string(p) {
            let mut out = String::with_capacity(full.len());
            for line in full.lines() {
                if line.starts_with("module RAMB18E1 (") {
                    // Everything from the first RAMB module onward (RAMB18E1 then RAMB36E1) is dropped.
                    return Some(out);
                }
                out.push_str(line);
                out.push('\n');
            }
            // No RAMB module found (unexpected shape) — treat as unusable so we skip rather than misbehave.
            eprintln!("cells_sim.v at {p} has no `module RAMB18E1 (` — cannot strip the RAMB models; skipping.");
            return None;
        }
    }
    None
}

/// Synthesize the ConfigBram engine for `heap_depth`, then GATE-LEVEL simulate the synthesized NETLIST (over the
/// faithful RAMB model + yosys generic cells) with the wire stream for `input` + `heap_init` bit-banged into the
/// UART at `CLKS_PER_BIT`, and return the decoded 64-bit result. `heap_depth` is threaded into the engine's
/// `WPE_STREAM_CAPS[7]` (the HEAP cap) so a caller can synthesize the FIXED depth (32768, a clean 2^15 tile) or
/// the BUGGY depth (20480, a non-power-of-two that triggers yosys's ×9/4096 depth-cascade mis-map).
///
/// Returns `None` (with a skip eprintln!) if `yosys` / `iverilog` / the yosys sim cells are unavailable — the
/// same graceful-skip contract the behavioral locks use; it never silently reports a pass.
fn synth_and_gatelevel_result(
    tag: &str,
    src: &str,
    fn_name: &str,
    input: i64,
    heap_init: &[i64],
    heap_depth: usize,
) -> Option<i64> {
    synth_edit_and_gatelevel_result(tag, src, fn_name, input, heap_init, heap_depth, |netlist| {
        netlist
    })
}

/// `synth_and_gatelevel_result` with `edit` applied to the synthesized netlist before it is simulated: the way a test
/// puts a synthesis defect into an otherwise correct netlist.
fn synth_edit_and_gatelevel_result(
    tag: &str,
    src: &str,
    fn_name: &str,
    input: i64,
    heap_init: &[i64],
    heap_depth: usize,
    edit: impl FnOnce(String) -> String,
) -> Option<i64> {
    if !common::require_tool("yosys") {
        return None;
    }
    if !common::require_tool("iverilog") {
        return None;
    }
    let noramb = cells_sim_noramb()?;

    // Build the config image at the requested heap depth (threaded through WPE_STREAM_CAPS[7]).
    let mut caps = WPE_STREAM_CAPS;
    caps[7] = heap_depth;
    let cfg = serialize_program(&program_cfg_fabric(src, fn_name));
    // UNCHECKED (like `emit_transformer_wire`): the checked builder caps at NREG=64, but the engine is emitted at
    // WPE_STREAM_CAPS (NREG=128) — so the checked cap silently confined every probe to reg[0..64], never touching
    // reg[64..124] where the transformer operates. Unchecked lets a probe reach the register indices the whole net
    // actually uses (the engine's own fits-check on the RTL is what bounds it).
    let im = wprog_config_image_unchecked(&cfg);
    let wire = im.wire_bytes_preloaded(&[input], heap_init);
    // The gate-level testbench (`gatelevel_tb`). A stalled netlist never sends its frame, so the sim runs to a CYCLE
    // budget built from the cycles a correct run takes at most: reset and the upload, the register-file seed, the
    // serial heap clear, the program (bounded by the engine model) and the result frame.
    let n_wire = wire.len();
    let per_byte = 1 + 10 * CLKS_PER_BIT as u64;
    let run_bound = eval_wprog_config_run(&cfg, &[input], heap_init)
        .work
        .rtl_cycle_bound(caps[0], caps[4]);
    let expected = 6
        + n_wire as u64 * per_byte
        + caps[0] as u64
        + 64
        + (heap_depth - heap_init.len().min(heap_depth)) as u64
        + run_bound
        + 14 * (per_byte + 2);
    let sim_s = cycle_budget(expected) / SLOWEST_GATE_CYCLES_PER_S + SYNTH_AND_COMPILE_S;
    let wall = WORST_CASE_WALL_S.fetch_add(sim_s, std::sync::atomic::Ordering::SeqCst) + sim_s;
    let cap = postsynth_wall_cap_s();
    assert!(
        wall <= cap,
        "{tag}: this test's gate-level sims can take {wall} s on the slowest core ({SLOWEST_GATE_CYCLES_PER_S} \
         cycles/s, {SYNTH_AND_COMPILE_S} s synth+compile per sim), past the ^postsynth_ cap of {cap} s — raise the \
         slow-timeout in .config/nextest.toml so a correct run cannot be cut off"
    );

    let dir = build_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let module = format!("wpe_soc_{tag}");
    let engine_v = dir.join(format!("engine_{tag}.v"));
    let netlist_v = dir.join(format!("netlist_{tag}.v"));
    let ramb_v = dir.join("ramb_behav.v");
    let cells_v = dir.join("cells_sim_noramb.v");
    let wire_hex = dir.join(format!("wire_{tag}.hex"));
    let ys = dir.join(format!("synth_{tag}.ys"));
    let synth_log = dir.join(format!("synth_{tag}.log"));
    let tb_v = dir.join(format!("tb_{tag}.v"));
    let sim = dir.join(format!("sim_{tag}.vvp"));

    // Emit the NON-DEBUG ConfigBram engine (the exact netlist a route+flash uses — no readback port to force
    // BRAM replication). `emit_..._backend` bakes CFG_BRAM=1; the yosys `chparam` sets WITH_FPU 0 + CLKS_PER_BIT 2.
    let soc = emit_wprog_engine_soc_verilog_backend(tag, caps, MemBackend::ConfigBram);
    std::fs::write(&engine_v, &soc).unwrap();
    std::fs::write(&ramb_v, RAMB_BEHAV_V).unwrap();
    std::fs::write(&cells_v, &noramb).unwrap();
    std::fs::write(
        &wire_hex,
        wire.iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();

    // The yosys flow: synth_xilinx to a 7-series netlist, WITH_FPU=0 + CFG_BRAM=1 + CLKS_PER_BIT=2 (fast bit-bang),
    // no DSP / no wide-LUT / flattened — the same flow that reached silicon. `write_verilog -noattr` for iverilog.
    let flow = format!(
        "read_verilog -sv {engine}\n\
         chparam -set WITH_FPU 0 {module}\n\
         chparam -set CFG_BRAM 1 {module}\n\
         chparam -set CLKS_PER_BIT {clks} {module}\n\
         hierarchy -top {module}\n\
         synth_xilinx -family xc7 -nodsp -nowidelut -flatten -top {module}\n\
         write_verilog -noattr {netlist}\n",
        engine = engine_v.display(),
        module = module,
        clks = CLKS_PER_BIT,
        netlist = netlist_v.display(),
    );
    std::fs::write(&ys, &flow).unwrap();

    eprintln!(
        "{tag}: synthesizing (heap_depth={heap_depth}, {} wire bytes) — this is the slow step…",
        wire.len()
    );
    let synth = std::process::Command::new("yosys")
        .arg("-s")
        .arg(&ys)
        .current_dir(&dir)
        .output()
        .expect("yosys");
    std::fs::write(&synth_log, &synth.stdout).ok();
    assert!(
        synth.status.success(),
        "{tag}: yosys synth failed:\n{}\n{}",
        String::from_utf8_lossy(&synth.stdout),
        String::from_utf8_lossy(&synth.stderr)
    );
    assert!(
        netlist_v.exists(),
        "{tag}: yosys produced no netlist at {}",
        netlist_v.display()
    );

    let netlist = edit(std::fs::read_to_string(&netlist_v).unwrap());
    std::fs::write(&netlist_v, &netlist).unwrap();
    let wire_name = wire_hex.file_name().unwrap().to_string_lossy().to_string();
    let tb = gatelevel_tb(
        &module,
        n_wire,
        &wire_name,
        expected,
        &kept_state_registers(&netlist),
    );
    std::fs::write(&tb_v, &tb).unwrap();

    // Compile the SYNTHESIZED NETLIST + the faithful RAMB model + yosys generic cells + the testbench.
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .args(gatelevel_defines().iter().map(|d| format!("-D{d}")))
        .arg("-o")
        .arg(&sim)
        .arg(&netlist_v)
        .arg(&ramb_v)
        .arg(&cells_v)
        .arg(&tb_v)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "{tag}: gate-level iverilog build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    eprintln!("{tag}: gate-level simulating the synthesized netlist (slow)…");
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout
            .lines()
            .any(|l| l == format!("POSTSYNTH_POWERUP random {RAMB_POWERUP_SEED}")),
        "{tag}: the gate-level sim was not built with the RAMB model's random power-up:\n{stdout}"
    );
    let line = stdout
        .lines()
        .find(|l| l.contains("POSTSYNTH_RESULT"))
        .unwrap_or_else(|| {
            panic!("{tag}: no POSTSYNTH_RESULT line in gate-level sim output:\n{stdout}")
        });
    assert!(
        !line.contains("XSTATE"),
        "{tag}: a state register of the gate-level netlist went X — {line}\n{stdout}"
    );
    assert!(
        !line.contains("TIMEOUT"),
        "{tag}: gate-level sim TIMED OUT before the result frame:\n{stdout}"
    );
    assert!(
        line.contains("sync=a5") && line.contains("end=5a"),
        "{tag}: malformed result frame: {line}"
    );
    let result: i64 = line
        .split("result=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("{tag}: cannot parse result from: {line}"));
    eprintln!("{tag}: gate-level {line}");
    Some(result)
}

/// The state registers of the engine's FSM that the synthesized netlist kept by name.
fn kept_state_registers(netlist: &str) -> Vec<&'static str> {
    ["ph", "st", "pc", "hclr", "hp"]
        .into_iter()
        .filter(|name| {
            let tail = format!(" {name};");
            netlist
                .lines()
                .any(|l| l.trim_start().starts_with("wire ") && l.trim_end().ends_with(&tail))
        })
        .collect()
}

/// The gate-level testbench for `module`: bit-bang the `n_wire`-byte image in `wire_name` into uart_rx at
/// CLKS_PER_BIT, decode the RESULT frame and print it as `POSTSYNTH_RESULT sync=… end=… result=… cycles=…`. Two early
/// stops print the `kept` state registers, so a failure names where the netlist stood: `XSTATE`, the cycle a kept
/// register that has held a value goes X (an FSM in X never recovers, so this is a hang's first cycle), and
/// `TIMEOUT`, the cycle budget — `expected`, the cycles a correct run takes at most, with a quarter's margin. It first
/// prints `POSTSYNTH_POWERUP`, the RAMB power-up it was built with.
fn gatelevel_tb(
    module: &str,
    n_wire: usize,
    wire_name: &str,
    expected: u64,
    kept: &[&str],
) -> String {
    let budget = cycle_budget(expected);
    let where_fmt: String = kept.iter().map(|name| format!(" {name}=%0d")).collect();
    let where_args: String = kept.iter().map(|name| format!(", dut.{name}")).collect();
    let x_regs: String = kept
        .iter()
        .map(|name| format!("  reg held_{name} = 1'b0;\n"))
        .collect();
    let x_checks: String = kept
        .iter()
        .map(|name| {
            format!(
                "    if (^dut.{name} !== 1'bx) held_{name} <= 1'b1;\n    else if (held_{name}) begin \
                 $display(\"POSTSYNTH_RESULT XSTATE {name} went X at cycle %0d:{where_fmt}\", tb_cyc{where_args}); \
                 $fflush; $finish; end\n"
            )
        })
        .collect();
    format!(
        r#"`timescale 1ns/1ps
module tb;
  localparam CLKS = {CLKS_PER_BIT};
  localparam MAXB = {n_wire};
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  {module} dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;
  reg [7:0] wirebytes [0:MAXB-1];
  integer k, fk, bi;
  reg [7:0] rb, sync, endb;
  reg [63:0] result; reg [31:0] cyc;
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
`ifdef RAMB_POWERUP_RANDOM
    $display("POSTSYNTH_POWERUP random %0d", `RAMB_POWERUP_RANDOM);
`else
    $display("POSTSYNTH_POWERUP init");
`endif
    $readmemh("{wire_name}", wirebytes);
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (bi=0; bi<MAXB; bi=bi+1) send_byte(wirebytes[bi]);
    recv_byte(sync);
    result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {{result[55:0], rb}}; end
    cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {{cyc[23:0], rb}}; end
    recv_byte(endb);
    $display("POSTSYNTH_RESULT sync=%02x end=%02x result=%0d cycles=%0d", sync, endb, $signed(result), cyc);
    $fflush;
    $finish;
  end
  // The early stops. `vvp` buffers stdout when it is a pipe (which it is under Command::output()), so each line is
  // invisible until $finish/$fflush — the `$fflush` calls guarantee the loud POSTSYNTH_RESULT line reaches the Rust
  // scraper even if the run is externally killed after printing.
  reg [63:0] tb_cyc = 0;
{x_regs}  always @(posedge clk) begin
    tb_cyc <= tb_cyc + 1;
{x_checks}  end
  initial begin
    repeat ({budget}) @(posedge clk);
    $display("POSTSYNTH_RESULT TIMEOUT after {budget} cycles (a correct run takes at most {expected}):{where_fmt}"
      {where_args});
    $fflush;
    $finish;
  end
endmodule
"#
    )
}

// ── The three bug-triggering programs, each MINIMAL (gate-level sim is slow) but exercising a distinct mis-map. ──

/// (a) HEAP BEYOND ONE 4096-WORD TILE. Default `SEQ_RESERVE` (2048) means the third collection's `Push` lands at
/// `heap[4120]`, whose depth-cascade alias is physical row `4120 & 4095 = 24`. `heap[24]` is NOT pre-loaded, so
/// the VM (and a correctly-mapped 32768-tile netlist) reads 0; a netlist that drops the top heap-address bits
/// aliases the `Push` into row 24 and reads back 999. Reading `item 24 of loaded()` (= `heap[24]`) is therefore
/// the clean discriminator with only a ~21-word heap preload (a few-hundred-byte stream).
const HEAP_BEYOND_TILE_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let a be a new Seq of Int.
    Let b be a new Seq of Int.
    Let c be a new Seq of Int.
    Push 999 to c.
    Return item 24 of loaded().
";

/// (c) CONTROL FLOW: a branch + a loop over a few blocks returning a computed value. `g(x)` loops `x` times,
/// adding `i` when `i` is even and `2*i` when odd — a data-dependent branch inside a loop, so the block-walk,
/// the decision terminator, and the register carry all ride the synthesized config/state BRAMs.
const CONTROL_FLOW_SRC: &str = "## To g (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 0.\n    While i is less than x:\n        If (i % 2) is equal to 0:\n            Set s to s + i.\n        Otherwise:\n            Set s to s + (2 * i).\n        Set i to i + 1.\n    Return s.\n";

/// The VM oracle for a heap-preloaded program (≡ `eval_wprog_config_counted_heap_preloaded`).
fn vm_preloaded(src: &str, fn_name: &str, input: i64, heap_init: &[i64]) -> i64 {
    let cfg = serialize_program(&program_cfg_fabric(src, fn_name));
    eval_wprog_config_counted_heap_preloaded(&cfg, &[input], heap_init).0
}

// ── PHASE 1 LOCALIZATION PROBES for the THIRD silicon mis-map (transformer returns 130913 vs 80113 on the
//    XC7A200T, cycle count matching to 0.016% ⇒ control flow correct, a residual DATA error). The two passing
//    gold-standard tests above prove ADDRESSING (heap tile) and a ~±600 register round-trip (SDP-72 parity), but
//    NEITHER verifies DATA INTEGRITY across ALL 64 bit-planes of the split state memories, nor the int8-multiply
//    / max-compare data paths. These probes each drive a DISTINCT construct with values that light up every
//    bit-plane and both signs, comparing the gate-level netlist result to `eval_wprog_config` (the VM). The
//    SMALLEST one that diverges reproduces the third bug in the faithful predictor. ──

/// (a) HEAP bit-plane DATA integrity. Preload the heap with values whose set bits sweep the WHOLE 64-bit range
/// (2^0..2^62, negatives, INT_MIN, alternating 0x55/0xAA masks) and XOR them all back through `item (i+1) of
/// loaded()` (= `heap[i+1]`) into a checksum. The existing `postsynth_heap_beyond_one_tile` test only ever read
/// back an UNSTORED slot (value 0), so it proved the heap ADDRESS decode but never that a stored 64-bit value
/// survives the depth-banked `heap_b*_lo`/`heap_b*_hi` split round-trip at EVERY bit. A single mis-wired heap bit-plane (a ×1 tile
/// wired to the wrong DI/DO net) corrupts exactly the values with that bit set — invisible to a read-back-zero
/// probe, visible here as a checksum mismatch.
const HEAP_XOR_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let mutable s be 0.
    Let mutable i be 0.
    While i is less than n:
        Set s to s xor (item (i + 1) of loaded()).
        Set i to i + 1.
    Return s.
";

/// The bit-plane-sweeping heap payload for (a): every value below has a DISTINCT set-bit signature so the XOR
/// checksum is sensitive to a corruption of ANY single bit-plane on ANY word.
fn heap_xor_payload() -> Vec<i64> {
    vec![
        1,
        2,
        4,
        8,
        16,
        1024,
        65536,
        0x0010_0000,
        0x0100_0000,
        0x4000_0000,
        1i64 << 32,
        1i64 << 40,
        1i64 << 48,
        1i64 << 62,
        -12345,
        0x000A_5A5A,
        0x0005_A5A5,
        -1,
        i64::MIN,
        0x5555_5555_5555_5555u64 as i64,
        0x00AA_AA00_00AA_AA00u64 as i64,
    ]
}

/// (b) INT8 MULTIPLY-accumulate (the matmul core). Preloaded pairs stress the signed 64-bit multiply the DSP
/// `func_f` maps (127*127, -128*127, 100*-99, …); each product is accumulated. A combinational-multiplier
/// mis-synth (or a sign-extension slip) shows up as a wrong accumulated sum.
const MUL_ACC_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let mutable s be 0.
    Let mutable i be 0.
    While i is less than n:
        Let a be item ((i * 2) + 1) of loaded().
        Let b be item ((i * 2) + 2) of loaded().
        Set s to s + (a * b).
        Set i to i + 1.
    Return s.
";

/// (e) SEQUENTIAL DIVIDER (`st==6`, the softmax-normalization path). `//` (floordiv) and `%` (mod) STALL the cell
/// walk into a 63-cycle magnitude restoring divider (`|a|/|x|`, re-signed) — the exact multi-cycle unit that can
/// mis-synthesize while the combinational multiply stays correct. Stresses both signs and magnitudes.
const DIV_ACC_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let mutable s be 0.
    Let mutable i be 0.
    While i is less than n:
        Let a be item ((i * 2) + 1) of loaded().
        Let b be item ((i * 2) + 2) of loaded().
        Set s to s + (a // b).
        Set s to s + (a % b).
        Set i to i + 1.
    Return s.
";

/// (d) MAX / argmax (the softmax argmax path). Reads preloaded values that cross the bit-16 and bit-24
/// boundaries, tracking the running max AND its index, then returns `best*1000 + argmax` so BOTH the compared
/// value and the index selection are checked. A `>` compare that mis-synthesizes a high bit-plane picks the
/// wrong element — exactly the argmax token error the silicon shows.
const MAX_SRC: &str = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let mutable best be item 1 of loaded().
    Let mutable bi be 0.
    Let mutable i be 1.
    While i is less than n:
        Let w be item (i + 1) of loaded().
        If w is greater than best:
            Set best to w.
            Set bi to i.
        Set i to i + 1.
    Return (best * 1000) + bi.
";

/// (a) HEAP DATA-INTEGRITY gate-level lock. XOR-checksums a bit-plane-sweeping heap payload back through the
/// synthesized depth-banked `heap_b*_lo`/`heap_b*_hi` split BRAM and asserts the netlist checksum == the VM's.
/// This is the probe the existing heap test could not be: it round-trips REAL 64-bit data (not a read-back-zero),
/// so a mis-wired heap bit-plane diverges here.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_heap_bitplane_data_integrity() {
    let payload = heap_xor_payload();
    let n = payload.len() as i64;
    let mut heap_init = vec![n];
    heap_init.extend_from_slice(&payload);
    let vm = vm_preloaded(HEAP_XOR_SRC, "acc", n, &heap_init);
    let expect = payload.iter().fold(0i64, |a, v| a ^ v);
    assert_eq!(vm, expect, "VM XOR checksum sanity");

    match synth_and_gatelevel_result(
        "heapbits",
        HEAP_XOR_SRC,
        "acc",
        n,
        &heap_init,
        WPE_STREAM_HEAP,
    ) {
        None => {
            eprintln!("SKIPPED postsynth_heap_bitplane_data_integrity (yosys/iverilog absent).")
        }
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH heap bit-plane DATA mis-wire: the gate-level netlist XOR-checksum is {gate:#018x} but \
                 the VM's is {vm:#018x} (diff {:#018x}) — a heap value bit-plane is mis-wired through the \
                 depth-banked heap_b*_lo/heap_b*_hi split BRAM (the THIRD silicon bug candidate)",
                (gate ^ vm)
            );
            println!("POSTSYNTH HEAP BIT-PLANE ≡VM: XOR checksum {gate:#018x} == VM {vm:#018x}.");
        }
    }
}

/// (b) INT8 MULTIPLY-accumulate gate-level lock. Asserts the synthesized DSP-mapped signed multiply + heap
/// operand fetch produce the VM's accumulated dot-product for boundary int8 pairs.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_int8_multiply_accumulate() {
    let pairs: [(i64, i64); 6] = [
        (127, 127),
        (-128, 127),
        (100, -99),
        (-50, -50),
        (0, 127),
        (63, -64),
    ];
    let mut flat: Vec<i64> = Vec::new();
    for (a, b) in pairs {
        flat.push(a);
        flat.push(b);
    }
    let mut heap_init = vec![flat.len() as i64];
    heap_init.extend_from_slice(&flat);
    let n = pairs.len() as i64;
    let vm = vm_preloaded(MUL_ACC_SRC, "acc", n, &heap_init);
    let expect = pairs.iter().fold(0i64, |s, (a, b)| s + a * b);
    assert_eq!(vm, expect, "VM mul-acc sanity");

    match synth_and_gatelevel_result("mulacc", MUL_ACC_SRC, "acc", n, &heap_init, WPE_STREAM_HEAP) {
        None => eprintln!("SKIPPED postsynth_int8_multiply_accumulate (yosys/iverilog absent)."),
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH int8-multiply mis-synth: the gate-level netlist dot-product is {gate} but the VM's is \
                 {vm} — the signed multiply / accumulate data path diverged on the synthesized netlist"
            );
            println!("POSTSYNTH INT8 MUL-ACC ≡VM: dot-product {gate} == VM {vm}.");
        }
    }
}

/// (e) SEQUENTIAL DIVIDER gate-level lock. Asserts the synthesized 63-cycle restoring divider (`st==6`, used for
/// softmax normalization) produces the VM's floordiv/mod accumulation for both-sign, varied-magnitude operands.
/// The multi-cycle divider FSM is the prime residual-bug candidate after heap/multiply/argmax proved ≡VM.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_sequential_divider_matches_vm() {
    let pairs: [(i64, i64); 8] = [
        (1000, 7),
        (-1000, 7),
        (1000, -7),
        (-1000, -7),
        (123456, 321),
        (5, 1000),
        (-5, 1000),
        (777, 1),
    ];
    let mut flat: Vec<i64> = Vec::new();
    for (a, b) in pairs {
        flat.push(a);
        flat.push(b);
    }
    let mut heap_init = vec![flat.len() as i64];
    heap_init.extend_from_slice(&flat);
    let n = pairs.len() as i64;
    let vm = vm_preloaded(DIV_ACC_SRC, "acc", n, &heap_init);

    match synth_and_gatelevel_result("divacc", DIV_ACC_SRC, "acc", n, &heap_init, WPE_STREAM_HEAP) {
        None => {
            eprintln!("SKIPPED postsynth_sequential_divider_matches_vm (yosys/iverilog absent).")
        }
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH sequential-divider mis-synth: the gate-level netlist floordiv/mod accumulation is \
                 {gate} but the VM's is {vm} — the 63-cycle restoring divider (st==6) diverged on the synthesized \
                 netlist (the softmax-normalization path — the residual third-bug candidate)"
            );
            println!("POSTSYNTH SEQ DIVIDER ≡VM: floordiv/mod acc {gate} == VM {vm}.");
        }
    }
}

/// (f) SOFTMAX-LIKE COMBINED-PATTERN gate-level lock. Every core op proved ≡VM in ISOLATION, but the transformer
/// diverges — so the residual is a COMBINATION no single-op probe exercises together: the softmax/argmax path,
/// which in one loop builds heap values, reads them by a COMPUTED index, multiplies, takes a mod, DIVIDES to
/// normalize, and runs a running-max argmax. If the netlist diverges only when these interleave (a read-during-
/// write, a divider-into-compare hazard, an operand-resolve chain), this reproduces it where the isolated probes
/// could not. Small loop → few registers → fits.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_softmax_like_pattern_matches_vm() {
    // 24 preloaded "scores"; the loop mimics exp-LUT-index → normalize(÷) → argmax over them.
    let scores: [i64; 24] = [
        5, -8, 130, 12, -400, 77, 256, -1, 900, 43, -900, 64, 1000, 7, -50, 511, 300, -300, 65,
        128, 999, -777, 250, 33,
    ];
    let mut heap_init = vec![scores.len() as i64];
    heap_init.extend_from_slice(&scores);
    let n = scores.len() as i64;
    let src = "\
## To native loaded () -> Seq of Int
## To acc (n: Int) -> Int:
    Let mutable best be -999999999.
    Let mutable bidx be 0.
    Let mutable sum be 0.
    Let mutable i be 0.
    While i is less than n:
        Let raw be item (i + 1) of loaded().
        Let e be (raw * raw) % 100003.
        Let w be (e * 1000) // (i + 3).
        Set sum to sum + w.
        If w is greater than best:
            Set best to w.
            Set bidx to i.
        Set i to i + 1.
    Return (((best * 1000) + bidx) * 1000) + (sum % 1000).
";
    let vm = vm_preloaded(src, "acc", n, &heap_init);

    match synth_and_gatelevel_result("softmax", src, "acc", n, &heap_init, WPE_STREAM_HEAP) {
        None => {
            eprintln!("SKIPPED postsynth_softmax_like_pattern_matches_vm (yosys/iverilog absent).")
        }
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH softmax-pattern mis-synth: the gate-level netlist computed {gate} but the VM {vm} — the \
                 interleaved heap-read / multiply / mod / divide / argmax path diverged on the synthesized netlist \
                 (the residual third-bug candidate — a COMBINATION the isolated probes each passed)"
            );
            println!("POSTSYNTH SOFTMAX-LIKE ≡VM: {gate} == VM {vm} (combined heap/mul/mod/div/argmax path).");
        }
    }
}

/// (g) HIGH REGISTER-INDEX gate-level lock — reachable ONLY now that the harness uses the UNCHECKED image builder
/// (the checked one capped at NREG=64). Holds ~90 preloaded values simultaneously live (register file indices up
/// to ~89, i.e. well past 64) then sums them incrementally. Every prior probe was confined to reg[0..64]; the
/// transformer uses ~124 registers, so a register-file address-decode mis-wire at index > 64 would corrupt the
/// whole net but nothing the earlier probes reached. This is the first probe into that regime.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_high_register_index_matches_vm() {
    let n: usize = 90;
    let mut heap_init = vec![n as i64];
    for k in 0..n {
        heap_init.push(((k as i64).wrapping_mul(2_654_435_761) % 1_000_003) - 500_000);
    }
    let mut body =
        String::from("## To native loaded () -> Seq of Int\n## To acc (unused: Int) -> Int:\n");
    for k in 0..n {
        body.push_str(&format!("    Let a{k} be item {} of loaded().\n", k + 1));
    }
    body.push_str("    Let mutable s be 0.\n");
    for k in 0..n {
        body.push_str(&format!("    Set s to s + a{k}.\n"));
    }
    body.push_str("    Return s.\n");
    let vm = vm_preloaded(&body, "acc", 0, &heap_init);

    match synth_and_gatelevel_result("highreg", &body, "acc", 0, &heap_init, WPE_STREAM_HEAP) {
        None => {
            eprintln!("SKIPPED postsynth_high_register_index_matches_vm (yosys/iverilog absent).")
        }
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH high-register-index mis-map: the gate-level netlist summed {gate} but the VM {vm} — the \
                 register-file address decode mis-wires past index 64 (the transformer uses ~124 registers), a \
                 regime NO earlier probe (all capped at reg 64) could reach"
            );
            println!("POSTSYNTH HIGH-REGISTER-INDEX ≡VM: sum {gate} == VM {vm} (~90 live registers, reg[64+] hit).");
        }
    }
}

/// (h) HEAP READ-MODIFY-WRITE COLLISION reproducer — the leading residual-bug candidate. The transformer's hot
/// loops do `Set item X of arr to (item X of arr) + delta` (num[X] += …, stream[X] += …) — a read-modify-write of
/// the SAME heap element, so the heap BRAM's read port and write port target the same address with overlapping
/// pipeline timing. A cross-port collision (UNDEFINED on a Xilinx RAMB) makes the read return STALE data on silicon
/// while the behavioral model returns the committed value — dropping increments. This does `xs[1] += 7` in a tight
/// loop: on a colliding netlist the sum is LESS than the VM's 7·n; the fix is explicit write-forwarding in
/// wpe_soc_template.v (bypass the RAMB read with the in-flight write data when the addresses match). RED here
/// reproduces the third bug in the faithful predictor; GREEN after the forwarding fix proves it before we route.
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_heap_read_modify_write_collision_matches_vm() {
    let n = 48i64;
    let empty: Vec<i64> = Vec::new();
    let src = "\
## To acc (n: Int) -> Int:
    Let mutable xs be [0, 0].
    Let mutable i be 0.
    While i is less than n:
        Set item 1 of xs to (item 1 of xs) + 7.
        Set i to i + 1.
    Return item 1 of xs.
";
    let vm = vm_preloaded(src, "acc", n, &empty);
    assert_eq!(vm, 7 * n, "VM sanity: xs[1] += 7 done n times = 7n");

    match synth_and_gatelevel_result("rmw", src, "acc", n, &empty, WPE_STREAM_HEAP) {
        None => eprintln!("SKIPPED postsynth_heap_read_modify_write_collision_matches_vm (yosys/iverilog absent)."),
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH heap read-modify-write COLLISION: the gate-level netlist computed {gate} but the VM {vm} \
                 (= 7·{n}) — the heap BRAM read port read STALE data while the write port wrote the same element (a \
                 cross-port collision), dropping increments. FIX = explicit write-forwarding in wpe_soc_template.v"
            );
            println!("POSTSYNTH READ-MODIFY-WRITE ≡VM: {gate} == VM {vm} (no heap collision — forwarding correct).");
        }
    }
}

/// (d) MAX / argmax gate-level lock. Asserts the synthesized `>` compare + running-max select pick the SAME
/// element the VM does for values crossing high bit-plane boundaries (the argmax token path).
#[test]
#[ignore = "PHASE-1 localization gate-level sim (~20-30min); run with --ignored --nocapture."]
fn postsynth_max_argmax_high_bits() {
    let vals: Vec<i64> = vec![
        100,
        0x0001_2345,
        0x00FF_FFFF,
        0x0080_0000,
        0x0000_FFFF,
        0x0100_0001,
        0x007F_FFFF,
    ];
    let mut heap_init = vec![vals.len() as i64];
    heap_init.extend_from_slice(&vals);
    let n = vals.len() as i64;
    let vm = vm_preloaded(MAX_SRC, "acc", n, &heap_init);
    let (bv, bi) =
        vals.iter().enumerate().fold(
            (vals[0], 0usize),
            |(bv, bi), (i, &v)| if v > bv { (v, i) } else { (bv, bi) },
        );
    assert_eq!(vm, bv * 1000 + bi as i64, "VM argmax sanity");

    match synth_and_gatelevel_result("maxarg", MAX_SRC, "acc", n, &heap_init, WPE_STREAM_HEAP) {
        None => eprintln!("SKIPPED postsynth_max_argmax_high_bits (yosys/iverilog absent)."),
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH max/argmax mis-synth: the gate-level netlist chose {gate} but the VM chose {vm} — the \
                 high-bit `>` compare / running-max select diverged on the synthesized netlist"
            );
            println!("POSTSYNTH MAX/ARGMAX ≡VM: {gate} == VM {vm}.");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────────────
// FAST CONSTRUCTION LOCKS — run in EVERY CI (no yosys/iverilog, milliseconds). They enforce the exact invariant
// each fix establishes, so a future change that reverts a fix goes RED instantly — long before synth or silicon.
// The slow `#[ignore]`d gate-level *result* sims below are the on-demand gold standard (they sim the real
// synthesized netlist ~30-60min each); these keep the pit-of-success closed on every push.
// ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// FAST LOCK for the RAMB MODEL itself (the harness's faithfulness). Every gate-level result sim above trusts
/// `assets/ramb_behav.v` to simulate the exact RAMB36E1 tiles yosys emits. A flat 2^15-deep heap maps to 64 RAMB36
/// each in ×1 (single-bit-plane) mode (the depth-banked heap's 16384-deep banks map ×2, held with every other width
/// by `the_ramb_model_round_trips_every_width`), and yosys ties the unused cascade address bit (bit 15) HIGH on the
/// tiles it places in the upper half — so a faithful model MUST (a) implement the ×1 read/write width and (b) index
/// the word with `ADDR[14:0]` (dropping the tied bit 15), never `ADDR[15:N]` (which runs the ×N word index off the
/// end of the 32768-bit data array and reads back X). A model missing EITHER makes every heap read return X — so the
/// gate-level sim never produces a valid result frame (the exact "no POSTSYNTH_RESULT line" the localization probes
/// hit), silently disarming the whole post-synth QA gate. This runs the model directly under iverilog (no yosys, ~1s):
/// write a bit to a HIGH heap address on a ×1 TDP RAMB36 with bit 15 tied high (exactly the heap bit-plane wiring),
/// read it back, and assert it is the written value — NOT X. If the model regresses to `[15:N]` or loses the ×1
/// branch, this goes RED instantly instead of a 30-min gate sim mysteriously yielding no result.
#[test]
fn ramb_model_round_trips_a_width1_heap_bit_plane_with_the_cascade_bit_tied_high() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::path::PathBuf::from("/tmp/psynthlock_rambmodel");
    std::fs::create_dir_all(&dir).unwrap();
    let ramb_v = dir.join("ramb_behav.v");
    let tb_v = dir.join("tb_ramb_w1.v");
    let sim = dir.join("ramb_w1.vvp");
    std::fs::write(&ramb_v, RAMB_BEHAV_V).unwrap();
    // A ×1 TDP RAMB36 wired EXACTLY like a heap bit-plane: port A write / port B read, address = {1'b1, idx15}
    // (the tied-high cascade bit yosys emits). Writes 1@20000 and 0@20001 (both > the 4096 single-tile boundary,
    // exercising the full 15-bit word address), reads them back, and prints them for the Rust side to check.
    let tb = r#"`timescale 1ns/1ps
module tb;
  reg clk=0; always #5 clk=~clk;
  reg wr; reg [14:0] wa, ra; reg dbit; wire [31:0] dob;
  RAMB36E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(0), .READ_WIDTH_B(1),
             .WRITE_WIDTH_A(1), .WRITE_WIDTH_B(0),
             .WRITE_MODE_A("READ_FIRST"), .WRITE_MODE_B("READ_FIRST")) dut (
    .CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(1'b1), .ENBWREN(1'b1),
    .REGCEAREGCE(1'b0), .REGCEB(1'b0), .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0),
    .RSTREGARSTREG(1'b0), .RSTREGB(1'b0),
    .ADDRARDADDR({1'b1, wa}), .ADDRBWRADDR({1'b1, ra}),
    .DIADI({31'b0, dbit}), .DIBDI(32'b0), .DIPADIP(4'b0), .DIPBDIP(4'b0),
    .DOADO(), .DOBDO(dob), .DOPADOP(), .DOPBDOP(), .WEA({4{wr}}), .WEBWE(8'b0));
  initial begin
    @(negedge clk); wr=1; wa=15'd20000; dbit=1'b1;
    @(negedge clk); wa=15'd20001; dbit=1'b0;
    @(negedge clk); wr=0; ra=15'd20000;
    @(posedge clk); #1; $display("RAMBW1 hi=%b", dob[0]);
    @(negedge clk); ra=15'd20001;
    @(posedge clk); #1; $display("RAMBW1 lo=%b", dob[0]);
    $finish;
  end
endmodule
"#;
    std::fs::write(&tb_v, tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&ramb_v)
        .arg(&tb_v)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "RAMB-model width-1 tb build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        out.contains("RAMBW1 hi=1"),
        "the faithful RAMB model read back `{}` for a WRITTEN width-1 heap bit-plane at a high address (expected \
         hi=1) — the model is missing the ×1 width branch or is indexing with the tied cascade bit `ADDR[15:N]` \
         instead of `ADDR[14:0]`, so every heap read returns X and the gate-level sim yields no valid result frame.\n\
         Full output:\n{out}",
        out.lines().find(|l| l.contains("RAMBW1 hi")).unwrap_or("<none>")
    );
    assert!(
        out.contains("RAMBW1 lo=0"),
        "the faithful RAMB model did not read back 0 for a width-1 heap bit-plane written 0 (output:\n{out})"
    );
}

/// Write `sources` (name, text) to a scratch dir, compile the `.v` ones with iverilog under `defines` (the rest are
/// data the sim reads), run them, and return whether the build and the run succeeded with everything they printed;
/// `None` without iverilog.
fn iverilog_run(
    tag: &str,
    sources: &[(&str, &str)],
    defines: &[String],
) -> Option<(bool, bool, String)> {
    if !common::require_tool("iverilog") {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("psynthlock_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in sources {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let sim = dir.join(format!("{tag}.vvp"));
    let mut build = std::process::Command::new("iverilog");
    build.args(["-g2012", "-o"]).arg(&sim);
    for d in defines {
        build.arg(format!("-D{d}"));
    }
    let verilog = sources
        .iter()
        .filter(|(n, _)| n.ends_with(".v"))
        .map(|(n, _)| dir.join(n));
    let build = build
        .args(verilog)
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    let mut log = String::from_utf8_lossy(&build.stderr).to_string();
    if !build.status.success() {
        std::fs::remove_dir_all(&dir).ok();
        return Some((false, false, log));
    }
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    log.push_str(&String::from_utf8_lossy(&run.stdout));
    log.push_str(&String::from_utf8_lossy(&run.stderr));
    std::fs::remove_dir_all(&dir).ok();
    Some((true, run.status.success() && !log.contains("FATAL"), log))
}

/// A RAMB36E1/RAMB18E1 instance for the model locks: `prim`, its `params`, clocks on `clk`, both ports enabled, each
/// port's address/data/enables named `<inst>_<port signal>` so a testbench drives them directly.
fn ramb_inst(prim: &str, inst: &str, params: &str) -> String {
    let (aw, dw, pw, wea, web) = if prim == "RAMB36E1" {
        (16, 32, 4, 4, 8)
    } else {
        (14, 16, 2, 2, 4)
    };
    let (a, d, p, wa, wb) = (aw - 1, dw - 1, pw - 1, wea - 1, web - 1);
    format!(
        "reg [{a}:0] {inst}_aa = 0, {inst}_ab = 0; reg [{d}:0] {inst}_da = 0, {inst}_db = 0;\n\
         reg [{p}:0] {inst}_pa = 0, {inst}_pb = 0; reg [{wa}:0] {inst}_wa = 0; reg [{wb}:0] {inst}_wb = 0;\n\
         wire [{d}:0] {inst}_oa, {inst}_ob; wire [{p}:0] {inst}_qa, {inst}_qb;\n\
         {prim} #({params}) {inst} (.CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(1'b1), .ENBWREN(1'b1),\n\
         .REGCEAREGCE(1'b0), .REGCEB(1'b0), .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0),\n\
         .RSTREGARSTREG(1'b0), .RSTREGB(1'b0),\n\
         .ADDRARDADDR({inst}_aa), .ADDRBWRADDR({inst}_ab), .DIADI({inst}_da), .DIBDI({inst}_db),\n\
         .DIPADIP({inst}_pa), .DIPBDIP({inst}_pb), .WEA({inst}_wa), .WEBWE({inst}_wb),\n\
         .DOADO({inst}_oa), .DOBDO({inst}_ob), .DOPADOP({inst}_qa), .DOPBDOP({inst}_qb));\n"
    )
}

/// A testbench of `insts` (from `ramb_inst`) running `body` from time 0 on a 10 ns clock.
fn ramb_tb(insts: &[String], body: &str) -> String {
    let insts = insts.concat();
    format!(
        "`timescale 1ns/1ps\nmodule tb;\nreg clk = 0; always #5 clk = ~clk;\n{insts}\
         initial begin\n{body}$finish;\nend\nendmodule\n"
    )
}

const RF: &str = ".WRITE_MODE_A(\"READ_FIRST\"), .WRITE_MODE_B(\"READ_FIRST\")";

/// The RAMB model powers up as 7-series silicon does: the arrays at INIT_xx/INITP_xx and the output latches at
/// INIT_A/INIT_B, an x bit written as 0 (as the bitstream writes it), so a never-written row or a port not yet read
/// reads a value, never X. yosys writes every INIT_xx/INITP_xx of an uninitialized memory as all x, as instance `b`
/// carries them. `RAMB_POWERUP_RANDOM`, which every gate-level sim is built with (`gatelevel_defines`), fills every row
/// and latch with defined pseudo-random words, different per instance, so a result a dropped write would leave at the
/// power-up value cannot match the VM by coincidence.
#[test]
fn the_ramb_model_powers_up_as_the_silicon_does() {
    let tdp36 = format!(".RAM_MODE(\"TDP\"), .READ_WIDTH_B(36), .WRITE_WIDTH_A(36), {RF}");
    let insts = [
        ramb_inst(
            "RAMB36E1",
            "a",
            &format!("{tdp36}, .INIT_B(36'h00000x005), .INIT_00(256'hCAFEF00D)"),
        ),
        ramb_inst(
            "RAMB36E1",
            "b",
            &format!("{tdp36}, .INIT_00(256'hx), .INITP_00(256'hx)"),
        ),
        ramb_inst(
            "RAMB18E1",
            "c",
            &format!(".RAM_MODE(\"SDP\"), .READ_WIDTH_A(36), .WRITE_WIDTH_B(36), {RF}"),
        ),
    ];
    let body = "#1 $display(\"LATCH a=%h b=%h\", a_ob, b_ob);\n\
                a_ab = 0; b_ab = 7 << 5; c_aa = 9 << 5;\n\
                @(posedge clk); #1 $display(\"ROW a0=%h b7=%h/%h c9=%h%h\", a_ob, b_ob, b_qb, c_ob, c_oa);\n\
                a_ab = 7 << 5;\n\
                @(posedge clk); #1 $display(\"SAME a7=%h b7=%h\", a_ob, b_ob);\n";
    let tb = ramb_tb(&insts, body);
    let Some((built, ran, out)) = iverilog_run(
        "rambpowerup",
        &[("ramb_behav.v", RAMB_BEHAV_V), ("tb.v", &tb)],
        &[],
    ) else {
        return;
    };
    assert!(
        built && ran,
        "the power-up tb did not build and run:\n{out}"
    );
    assert!(
        out.contains("LATCH a=00000005 b=00000000"),
        "the latches power up at INIT_B, an x bit as 0:\n{out}"
    );
    assert!(
        out.contains("ROW a0=cafef00d b7=00000000/0 c9=00000000"),
        "rows power up at INIT_xx/INITP_xx, an x bit as 0, never X:\n{out}"
    );

    let (built, ran, out) = iverilog_run(
        "rambrandom",
        &[("ramb_behav.v", RAMB_BEHAV_V), ("tb.v", &tb)],
        &gatelevel_defines(),
    )
    .unwrap();
    assert!(
        built && ran,
        "the random power-up tb did not build and run:\n{out}"
    );
    let line = |tag: &str| {
        out.lines()
            .find(|l| l.starts_with(tag))
            .unwrap_or_else(|| panic!("no {tag} line:\n{out}"))
            .to_string()
    };
    let words = |l: &str| -> Vec<String> {
        l.split_whitespace()
            .skip(1)
            .map(|w| w.split('=').nth(1).unwrap_or("").replace('/', ""))
            .collect()
    };
    let row = line("ROW ");
    assert!(
        !row.contains('x') && !row.contains('X'),
        "a random power-up is defined in every bit: {row}"
    );
    assert!(
        words(&row).iter().all(|w| w.chars().any(|c| c != '0')),
        "a random power-up is not the zero power-up: {row}"
    );
    let same = words(&line("SAME "));
    assert_ne!(
        same[0], same[1],
        "each instance powers up differently, row 7 of `a` against row 7 of `b`"
    );
}

/// Every width the model accepts round-trips, laid out as the device lays it out: TDP ×1/2/4/9/18/36 written on port A
/// and read on port B at a high word address (the cascade bit tied high, as yosys ties it), a ×2 write read back as two
/// ×1 bits, a ×36 write read back as four ×9 bytes, and SDP-72/SDP-36 with their parity lanes. The gate-level heap is
/// ×2 TDP: a model without that width dropped every heap write and left the heap's read latch at its power-up value.
#[test]
fn the_ramb_model_round_trips_every_width() {
    let tdp = |write: u32, read: u32| {
        format!(".RAM_MODE(\"TDP\"), .WRITE_WIDTH_A({write}), .READ_WIDTH_B({read}), {RF}")
    };
    let sdp = |width: u32| {
        format!(".RAM_MODE(\"SDP\"), .READ_WIDTH_A({width}), .WRITE_WIDTH_B({width}), {RF}")
    };
    let mut insts = Vec::new();
    let mut body = String::new();
    for (w, word, value) in [
        (1u32, 30000u32, 1u64),
        (2, 16000, 2),
        (4, 8000, 0xA),
        (9, 4000, 0x1A5),
        (18, 2000, 0x3BEEF),
        (36, 1000, 0xFCAFEF00D),
    ] {
        let inst = format!("w{w}");
        insts.push(ramb_inst("RAMB36E1", &inst, &tdp(w, w)));
        let shift = [(1, 0), (2, 1), (4, 2), (9, 3), (18, 4), (36, 5)]
            .iter()
            .find(|(x, _)| *x == w)
            .unwrap()
            .1;
        let data_bits = match w {
            9 => 8,
            18 => 16,
            36 => 32,
            _ => w,
        };
        let (data, par) = (value & ((1u64 << data_bits) - 1), value >> data_bits);
        body.push_str(&format!(
            "@(negedge clk); {inst}_aa = 16'h8000 | ({word} << {shift}); {inst}_da = {data}; {inst}_pa = {par};\n\
             {inst}_wa = 4'hF;\n\
             @(negedge clk); {inst}_wa = 0; {inst}_ab = 16'h8000 | ({word} << {shift});\n\
             @(posedge clk); #1 $display(\"W{w} %0d %0d\", {inst}_ob, {inst}_qb);\n"
        ));
    }
    insts.push(ramb_inst("RAMB36E1", "m21", &tdp(2, 1)));
    body.push_str(
        "@(negedge clk); m21_aa = 16'h8000 | (100 << 1); m21_da = 2'b10; m21_wa = 4'hF;\n\
         @(negedge clk); m21_wa = 0; m21_ab = 16'h8000 | 200;\n\
         @(posedge clk); #1 $display(\"M21 lo=%0d\", m21_ob);\n\
         @(negedge clk); m21_ab = 16'h8000 | 201;\n\
         @(posedge clk); #1 $display(\"M21 hi=%0d\", m21_ob);\n",
    );
    insts.push(ramb_inst("RAMB36E1", "m369", &tdp(36, 9)));
    body.push_str(
        "@(negedge clk); m369_aa = 3 << 5; m369_da = 32'hAABBCCDD; m369_wa = 4'hF;\n\
         @(negedge clk); m369_wa = 0; m369_ab = 13 << 3;\n\
         @(posedge clk); #1 $display(\"M369 byte13=%h\", m369_ob);\n",
    );
    insts.push(ramb_inst("RAMB36E1", "s72", &sdp(72)));
    body.push_str(
        "@(negedge clk); s72_ab = 40 << 6; s72_da = 32'h11223344; s72_db = 32'h55667788;\n\
         s72_pa = 4'hA; s72_pb = 4'h5; s72_wb = 8'hFF;\n\
         @(negedge clk); s72_wb = 0; s72_aa = 40 << 6;\n\
         @(posedge clk); #1 $display(\"S72 %h%h %h%h\", s72_ob, s72_oa, s72_qb, s72_qa);\n",
    );
    insts.push(ramb_inst("RAMB18E1", "s36", &sdp(36)));
    body.push_str(
        "@(negedge clk); s36_ab = 300 << 5; s36_da = 16'h1234; s36_db = 16'h5678; s36_pa = 2'b10; s36_pb = 2'b01;\n\
         s36_wb = 4'hF;\n\
         @(negedge clk); s36_wb = 0; s36_aa = 300 << 5;\n\
         @(posedge clk); #1 $display(\"S36 %h%h %h%h\", s36_ob, s36_oa, s36_qb, s36_qa);\n",
    );
    let tb = ramb_tb(&insts, &body);
    let Some((built, ran, out)) = iverilog_run(
        "rambwidths",
        &[("ramb_behav.v", RAMB_BEHAV_V), ("tb.v", &tb)],
        &[],
    ) else {
        return;
    };
    assert!(built && ran, "the width tb did not build and run:\n{out}");
    for want in [
        "W1 1 0",
        "W2 2 0",
        "W4 10 0",
        "W9 165 1",
        "W18 48879 3",
        "W36 3405705229 15",
        "M21 lo=0",
        "M21 hi=1",
        "M369 byte13=000000cc",
        "S72 5566778811223344 5a",
        "S36 56781234 12",
    ] {
        assert!(
            out.lines().any(|l| l.trim() == want),
            "`{want}` did not round-trip:\n{out}"
        );
    }
}

/// A configuration the model does not implement stops the simulation at time 0, naming the cell and what it could not
/// model: a width it cannot simulate would otherwise drop writes and hold its read latch at the power-up value.
#[test]
fn the_ramb_model_refuses_what_it_does_not_implement() {
    for (params, what) in [
        (format!(".RAM_MODE(\"TDP\"), .READ_WIDTH_B(5), .WRITE_WIDTH_A(36), {RF}"), "TDP port width"),
        (format!(".RAM_MODE(\"TDP\"), .READ_WIDTH_B(36), .WRITE_WIDTH_A(36), .DOB_REG(1), {RF}"), "output register"),
        (
            format!(".RAM_MODE(\"TDP\"), .READ_WIDTH_B(1), .WRITE_WIDTH_A(1), .RAM_EXTENSION_A(\"UPPER\"), {RF}"),
            "RAM_EXTENSION",
        ),
        (
            ".RAM_MODE(\"TDP\"), .READ_WIDTH_A(36), .WRITE_WIDTH_A(36), .WRITE_MODE_A(\"WRITE_FIRST\")".to_string(),
            "WRITE_MODE_A",
        ),
        (format!(".RAM_MODE(\"SDP\"), .READ_WIDTH_A(36), .WRITE_WIDTH_B(72), {RF}"), "SDP port widths"),
    ] {
        let tb = ramb_tb(&[ramb_inst("RAMB36E1", "r", &params)], "    #20;\n");
        let Some((built, ran, out)) = iverilog_run("rambrefuse", &[("ramb_behav.v", RAMB_BEHAV_V), ("tb.v", &tb)], &[])
        else {
            return;
        };
        assert!(
            built && !ran && out.contains("unsupported") && out.contains(what) && out.contains("tb.r"),
            "`{params}` must stop the sim naming the cell and `{what}`:\n{out}"
        );
    }
}

/// The gate-level testbench stops a stalled netlist early and names where it stood: a kept state register that goes X
/// after holding a value stops the run on that cycle (`XSTATE`), and a netlist that never sends its frame stops at the
/// cycle budget (`TIMEOUT`), each printing the kept registers. Built as every gate-level sim is (`gatelevel_defines`),
/// it names the random RAMB power-up first. Run on stand-in DUTs, in seconds.
#[test]
fn the_gatelevel_testbench_stops_a_stalled_netlist_and_names_it() {
    let stub = |x_at: u32| {
        format!(
            r#"module stub(input clk, input rst, input uart_rx, output uart_tx);
  assign uart_tx = 1'b1;
  reg [3:0] ph; reg [6:0] st; reg [31:0] n;
  always @(posedge clk) begin
    if (rst) begin ph <= 4'd0; n <= 0; end
    else begin n <= n + 1; ph <= 4'd6; st <= (n == {x_at}) ? 7'bx : (n > {x_at} ? st : 7'd21); end
  end
endmodule
"#
        )
    };
    let hex = "00\n";
    let defines = gatelevel_defines();
    let tb = gatelevel_tb("stub", 1, "w.hex", 100, &["ph", "st"]);
    let Some((built, _, out)) = iverilog_run(
        "tbxstate",
        &[("stub.v", &stub(40)), ("tb.v", &tb), ("w.hex", hex)],
        &defines,
    ) else {
        return;
    };
    assert!(built, "the stub tb did not build:\n{out}");
    assert!(
        out.lines()
            .any(|l| l == format!("POSTSYNTH_POWERUP random {RAMB_POWERUP_SEED}")),
        "the testbench names the random RAMB power-up the gate-level sims are built with:\n{out}"
    );
    let line = out
        .lines()
        .find(|l| l.contains("POSTSYNTH_RESULT"))
        .unwrap_or_else(|| panic!("no result line:\n{out}"));
    let at: u64 = line
        .split("XSTATE st went X at cycle ")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .and_then(|c| c.trim().parse().ok())
        .unwrap_or_else(|| {
            panic!("an X on a held state register stops the run with XSTATE: {line}")
        });
    assert!(
        (40..=50).contains(&at),
        "the stop names the cycle the register went X (the stub's 40, after reset): {line}"
    );
    assert!(
        line.contains("ph=6"),
        "and prints the kept registers: {line}"
    );

    let (built, _, out) = iverilog_run(
        "tbtimeout",
        &[("stub.v", &stub(1_000_000)), ("tb.v", &tb), ("w.hex", hex)],
        &defines,
    )
    .unwrap();
    assert!(built, "the stub tb did not build:\n{out}");
    let line = out
        .lines()
        .find(|l| l.contains("POSTSYNTH_RESULT"))
        .unwrap_or_else(|| panic!("no result line:\n{out}"));
    assert_eq!(
        line.trim(),
        "POSTSYNTH_RESULT TIMEOUT after 2125 cycles (a correct run takes at most 100): ph=6 st=21",
        "a netlist that never answers stops at the budget, naming where it stands"
    );
}

/// FAST LOCK for bug #1 (yosys depth-cascade). The mis-map ONLY bites a heap whose depth is NOT a power of two:
/// yosys 0.33 maps it to a ×9 / 4096-deep RAMB tile and fails to depth-cascade, dropping the top address bits.
/// A power-of-two depth maps to clean tiles with no cascade. So the invariant that kills bug #1 is "WPE_STREAM_HEAP
/// is a power of two (and covers the transformer's working set)".
#[test]
fn heap_stream_depth_is_a_power_of_two_so_yosys_needs_no_depth_cascade() {
    assert!(
        WPE_STREAM_HEAP.is_power_of_two(),
        "WPE_STREAM_HEAP={WPE_STREAM_HEAP} must be a power of two — a non-power-of-two heap depth triggers the \
         yosys 0.33 depth-cascade mis-map (bug #1) that silently drops the top heap-address bits on silicon."
    );
    assert!(
        WPE_STREAM_HEAP >= 18_300,
        "WPE_STREAM_HEAP={WPE_STREAM_HEAP} must cover the streamed transformer's ~18,300-word heap working set."
    );
}

/// FAST LOCK for bug #2 (SDP-72 parity mis-wire). A 64-bit-wide state memory maps to one RAMB36 in SDP-72 mode,
/// whose parity-lane write yosys 0.33 mis-wires — so the 8 parity bits a 64-bit word needs are never stored and
/// any value >= 256 reads back corrupted on silicon. The fix splits `file`/`v`/`sfile` into 32-bit `*_lo`/`*_hi`
/// halves (each maps TDP-W36, never SDP-72). Asserts the emitted ConfigBram engine declares NO 64-bit-wide
/// `file`/`v`/`sfile` and DOES declare the split halves — instant; an un-split turns it RED before synth.
#[test]
fn wide_state_memories_are_lo_hi_split_so_none_maps_to_sdp72() {
    let soc = emit_wprog_engine_soc_verilog_backend("qa", WPE_STREAM_CAPS, MemBackend::ConfigBram);
    for mem in ["file", "v", "sfile"] {
        let unsplit = format!("[63:0] {mem} [");
        assert!(
            !soc.contains(&unsplit),
            "the ConfigBram engine declares a 64-bit-wide `{mem}` (`{unsplit}`) — it must be split into \
             `{mem}_lo`/`{mem}_hi` 32-bit halves so it never maps to a RAMB36 in SDP-72 mode, whose parity lane \
             yosys 0.33 mis-wires (bug #2: any register value >= 256 corrupts on silicon)."
        );
        assert!(
            soc.contains(&format!("{mem}_lo")) && soc.contains(&format!("{mem}_hi")),
            "the ConfigBram engine must declare `{mem}_lo` and `{mem}_hi` (the 32-bit split that dodges the \
             SDP-72 parity-lane mis-wire — bug #2's fix)."
        );
    }
}

/// (a) The heap depth-cascade lock. Bug #1's fix is `WPE_STREAM_HEAP = 32768`; this proves that at the FIXED
/// depth, a `Push` to `heap[4120]` and a read of `heap[24]` are DISTINCT rows, so the gate-level netlist reads
/// back the VM's value (0). Before the fix, both aliased into tile 0 and the netlist returned 999.
#[test]
#[ignore = "gold-standard gate-level result sim (~30-60min); run with --ignored. The instant lock \
            heap_stream_depth_is_a_power_of_two_so_yosys_needs_no_depth_cascade enforces bug #1's fix every CI."]
fn postsynth_heap_beyond_one_tile_reads_back_correct() {
    // The engine's compile-time heap fix must be the 2^15 single-tile depth this test proves correct.
    assert_eq!(
        WPE_STREAM_HEAP, 32768,
        "bug #1's fix is WPE_STREAM_HEAP=32768 (one clean RAMB36 depth-tile)"
    );

    let mut heap_init: Vec<i64> = vec![20];
    for k in 0..20i64 {
        heap_init.push(50 + k);
    }
    let vm = vm_preloaded(HEAP_BEYOND_TILE_SRC, "acc", 0, &heap_init);
    assert_eq!(
        vm, 0,
        "heap[24] is unpreloaded, so the VM reads 0 (a mis-mapped netlist reads the Push's 999)"
    );

    match synth_and_gatelevel_result(
        "heaptile",
        HEAP_BEYOND_TILE_SRC,
        "acc",
        0,
        &heap_init,
        WPE_STREAM_HEAP,
    ) {
        None => eprintln!(
            "SKIPPED postsynth_heap_beyond_one_tile_reads_back_correct (yosys/iverilog absent)."
        ),
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH heap-tile mis-map: the gate-level netlist read {gate} for heap[24] but the VM reads \
                 {vm} — a heap access beyond the 4096-word tile aliased into tile 0 (yosys depth-cascade bug #1)"
            );
            println!(
                "POSTSYNTH HEAP DEPTH-CASCADE ≡VM: heap[4120] Push + heap[24] read are distinct rows on the \
                 SYNTHESIZED netlist (result {gate} == VM {vm}) at WPE_STREAM_HEAP={WPE_STREAM_HEAP}."
            );
        }
    }
}

/// (b) The SDP-72 parity lock. Bug #2's fix splits `file`/`v`/`heap`/`sfile` into 32-bit `*_lo`/`*_hi` halves so
/// none maps to a single RAMB36 in SDP-72 mode (whose parity write yosys 0.33 mis-wires). This proves a register
/// value >= 256 — large positive AND large negative — survives the round-trip through the synthesized register-file
/// BRAM byte-for-byte. Under the un-split 64-bit array, the lost parity lane corrupted any value >= 256.
#[test]
#[ignore = "gold-standard gate-level result sim (~30-60min); run with --ignored. The instant lock \
            wide_state_memories_are_lo_hi_split_so_none_maps_to_sdp72 enforces bug #2's fix every CI."]
fn postsynth_register_value_past_255_reads_back_correct() {
    // Two inputs: a large positive (0 + 100*3 = 300) and a large negative (-600 + 100*3 = -300), so both the high
    // data bits and the sign extension cross the 8-bit byte-lane / parity boundary the SDP-72 mis-wire dropped.
    let empty: Vec<i64> = Vec::new();
    for (label, p, x, want) in [("pos", 0i64, 3i64, 300i64), ("neg", -600i64, 3i64, -300i64)] {
        // f takes (p, x); wire_bytes takes the first param, so bind x via a fixed one-param projection: we
        // instead fold x into the program by making the streamed input carry `p` and hard-coding the loop bound.
        // Both are covered by two dedicated one-param programs so the wire's single input is unambiguous.
        let src = format!(
            "## To f (p: Int) -> Int:\n    Let mutable s be p.\n    Let mutable i be 0.\n    While i is less than {x}:\n        Set s to s + 100.\n        Set i to i + 1.\n    Return s.\n"
        );
        let cfg = serialize_program(&program_cfg_fabric(&src, "f"));
        let vm = eval_wprog_config(&cfg, &[p]);
        assert_eq!(vm, want, "{label}: VM f({p}) over {x} iters must be {want}");
        assert!(
            want.unsigned_abs() >= 256,
            "{label}: the carried value must exceed 255 to hit the parity lane"
        );

        match synth_and_gatelevel_result(
            &format!("reg255_{label}"),
            &src,
            "f",
            p,
            &empty,
            WPE_STREAM_HEAP,
        ) {
            None => {
                eprintln!("SKIPPED postsynth_register_value_past_255_reads_back_correct ({label}; tools absent).");
                return;
            }
            Some(gate) => {
                assert_eq!(
                    gate, vm,
                    "POST-SYNTH SDP-72 parity mis-map: the gate-level netlist read {gate} for a register value \
                     past 255 but the VM reads {vm} ({label}) — the RAMB36 SDP-72 parity lane was dropped (bug #2)"
                );
                println!(
                    "POSTSYNTH SDP-72 PARITY ≡VM ({label}): register value {gate} (|{gate}| >= 256) survived the \
                     synthesized file_lo/file_hi round-trip == VM {vm}."
                );
            }
        }
    }
}

/// (c) The control-flow lock. A data-dependent branch inside a loop over several blocks, returning a computed
/// value — exercises the config-driven block-walk / decision terminator / register carry on the synthesized
/// config + state BRAMs. Proves the whole control path is ≡VM on the real netlist, not just in RTL sim.
#[test]
#[ignore = "gold-standard gate-level result sim (~30-60min); run with --ignored."]
fn postsynth_control_flow_matches_vm() {
    let empty: Vec<i64> = Vec::new();
    let x = 6i64;
    let cfg = serialize_program(&program_cfg_fabric(CONTROL_FLOW_SRC, "g"));
    let vm = eval_wprog_config(&cfg, &[x]);
    // g(6) = 0 + 2 + 2 + 6 + 4 + 10 = 24 (even i adds i, odd i adds 2i, over i=0..5).
    assert_eq!(vm, 24, "g(6) must be 0+2+2+6+4+10 = 24");

    match synth_and_gatelevel_result("ctrl", CONTROL_FLOW_SRC, "g", x, &empty, WPE_STREAM_HEAP) {
        None => eprintln!("SKIPPED postsynth_control_flow_matches_vm (yosys/iverilog absent)."),
        Some(gate) => {
            assert_eq!(
                gate, vm,
                "POST-SYNTH control-flow mis-map: the gate-level netlist computed {gate} but the VM computes \
                 {vm} — the branch/loop config path diverged on the synthesized netlist"
            );
            println!(
                "POSTSYNTH CONTROL FLOW ≡VM: the branch-in-loop program computed {gate} on the SYNTHESIZED \
                 netlist == VM {vm}."
            );
        }
    }
}

/// The heap depth the teeth lock synthesizes. Bank 0 is 12288 rows, which yosys tiles ×9 three 4096-row tiles deep,
/// each depth tile written through its own decoded strobe.
const DEPTH_TILED_HEAP: usize = 12288;

/// Bug #1's defect, put into a synthesized netlist: heap bank 0 loses its depth decode. Every tile's write strobe
/// becomes the OR of the per-depth strobes, so a write lands at its row in every depth tile and a read past a tile's
/// 4096 rows returns the latest write to any address on the same row. Panics (the PRECONDITION) unless bank 0 is
/// depth-tiled as the edit assumes: block-RAM tiles of 4096 ×9 rows read on `heap_ra[11:0]`, one strobe per tile, at
/// least two distinct strobes.
fn drop_heap_bank0_depth_decode(netlist: String) -> String {
    const READ_ADDR: &str = ".ADDRBWRADDR({ 1'h1, heap_ra[11:0], 3'h0 })";
    const WEA_OPEN: &str = ".WEA({ ";
    const WEA_CLOSE: &str = " })";
    let mut strobes: Vec<String> = Vec::new();
    let mut weas: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    let mut at = 0;
    while let Some(off) = netlist[at..].find(") \\heap_b0_") {
        let head = at + off;
        let body_start = head
            + netlist[head..]
                .find('\n')
                .expect("an instance header ends its line");
        let body_end = body_start
            + netlist[body_start..]
                .find("\n  );")
                .expect("an instance ends");
        let name = netlist[head + 2..body_start]
            .trim_end()
            .trim_end_matches('(')
            .trim_end();
        let prim = netlist[..head]
            .rsplit_once(" #(")
            .and_then(|(before, _)| before.split_whitespace().last())
            .unwrap_or("");
        assert!(
            prim == "RAMB36E1" || prim == "RAMB18E1",
            "PRECONDITION: heap bank 0 cell {name} is a {prim}, not a block-RAM tile"
        );
        let body = &netlist[body_start..body_end];
        assert!(
            body.contains(READ_ADDR),
            "PRECONDITION: {name} is not a 4096-row ×9 tile read on heap_ra[11:0]:{body}"
        );
        let open = body_start
            + body
                .find(WEA_OPEN)
                .unwrap_or_else(|| panic!("{name} has no WEA connection"))
            + WEA_OPEN.len();
        let close = open
            + netlist[open..]
                .find(WEA_CLOSE)
                .expect("the WEA concatenation closes");
        let nets: Vec<&str> = netlist[open..close].split(", ").collect();
        assert!(
            nets.iter().all(|n| *n == nets[0]),
            "PRECONDITION: {name} writes its byte lanes on different strobes: {nets:?}"
        );
        if !strobes.iter().any(|s| s == nets[0]) {
            strobes.push(nets[0].to_string());
        }
        weas.push((open - WEA_OPEN.len()..close + WEA_CLOSE.len(), nets.len()));
        at = body_end;
    }
    assert!(
        strobes.len() >= 2,
        "PRECONDITION: heap bank 0 is not tiled in depth: {} tile(s) on the write strobe(s) {strobes:?}",
        weas.len()
    );
    eprintln!(
        "dropping the depth decode of heap bank 0: {} tiles on strobes {strobes:?}",
        weas.len()
    );
    let mut out = netlist;
    for (range, width) in weas.into_iter().rev() {
        out.replace_range(range, &format!(".WEA({{{width}{{heap_b0_we_undecoded}}}})"));
    }
    let end = out.rfind("endmodule").expect("the netlist module ends");
    out.insert_str(
        end,
        &format!("  assign heap_b0_we_undecoded = {};\n", strobes.join(" | ")),
    );
    let module = out.find("module ").expect("the netlist has a module");
    let header_end = module + out[module..].find(";\n").expect("the module header ends") + 2;
    out.insert_str(header_end, "  wire heap_b0_we_undecoded;\n");
    out
}

/// The decode drop the teeth lock applies, on a netlist written as yosys writes one: it rewires the write strobe of
/// every heap bank 0 tile and nothing else, and it refuses every netlist whose bank 0 is not depth-tiled as it assumes,
/// so the teeth lock goes red on its precondition rather than simulating an edit that never happened.
#[test]
fn the_heap_decode_drop_rewires_every_bank0_strobe_and_refuses_an_untiled_bank() {
    let tile = |prim: &str, name: &str, read: &str, wea: &str| {
        format!(
            "  {prim} #(\n    .RAM_MODE(\"TDP\")\n  ) \\{name}  (\n    .ADDRBWRADDR({read}),\n    \
             .WEA({{ {wea} }}),\n    .WEBWE(8'h00)\n  );\n"
        )
    };
    let netlist = |tiles: &[String]| {
        format!(
            "module wpe_soc_t(clk, rst);\n  input clk;\n  input rst;\n  wire [2:0] _066_;\n{}endmodule\n",
            tiles.concat()
        )
    };
    const TILED: &str = "{ 1'h1, heap_ra[11:0], 3'h0 }";
    let strobe = |d: u32| vec![format!("_066_[{d}]"); 4].join(", ");
    let other = tile(
        "RAMB36E1",
        "file_lo.0.0",
        "{ 1'h1, file_ra[9:0], 5'h00 }",
        "_077_, _077_, _077_, _077_",
    );
    let good = netlist(&[
        tile("RAMB36E1", "heap_b0_lo.0.0", TILED, &strobe(0)),
        tile("RAMB36E1", "heap_b0_hi.0.4", TILED, &strobe(1)),
        other.clone(),
        tile("RAMB36E1", "heap_b0_hi.0.8", TILED, &strobe(2)),
    ]);
    let out = drop_heap_bank0_depth_decode(good.clone());
    assert_eq!(
        out.matches(".WEA({4{heap_b0_we_undecoded}})").count(),
        3,
        "every bank 0 tile writes on the undecoded strobe:\n{out}"
    );
    assert!(
        out.contains(&other),
        "a tile outside heap bank 0 is untouched:\n{out}"
    );
    assert!(
        out.starts_with("module wpe_soc_t(clk, rst);\n  wire heap_b0_we_undecoded;\n"),
        "the undecoded strobe is declared before any use:\n{out}"
    );
    assert!(
        out.ends_with(
            "  assign heap_b0_we_undecoded = _066_[0] | _066_[1] | _066_[2];\nendmodule\n"
        ),
        "the undecoded strobe is the OR of every depth strobe:\n{out}"
    );
    let restored = out
        .replace("  wire heap_b0_we_undecoded;\n", "")
        .replace(
            "  assign heap_b0_we_undecoded = _066_[0] | _066_[1] | _066_[2];\n",
            "",
        )
        .replacen(
            ".WEA({4{heap_b0_we_undecoded}})",
            &format!(".WEA({{ {} }})", strobe(0)),
            1,
        )
        .replacen(
            ".WEA({4{heap_b0_we_undecoded}})",
            &format!(".WEA({{ {} }})", strobe(1)),
            1,
        )
        .replacen(
            ".WEA({4{heap_b0_we_undecoded}})",
            &format!(".WEA({{ {} }})", strobe(2)),
            1,
        );
    assert_eq!(
        restored, good,
        "the edit changes the three strobes and adds the undecoded one, nothing else"
    );

    let refusal = |bad: String| -> String {
        let err = std::panic::catch_unwind(|| drop_heap_bank0_depth_decode(bad))
            .expect_err("the edit must refuse");
        err.downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    };
    for (why, bad, says) in [
        (
            "one depth tile",
            netlist(&[
                tile("RAMB36E1", "heap_b0_lo.0.0", TILED, &strobe(0)),
                tile("RAMB36E1", "heap_b0_hi.0.1", TILED, &strobe(0)),
            ]),
            "not tiled in depth",
        ),
        (
            "the shipping ×2 bank",
            netlist(&[tile(
                "RAMB36E1",
                "heap_b0_lo.0.0",
                "{ 1'h1, heap_ra[13:0], 1'h0 }",
                "_000091_, _000091_, _000091_, _000091_",
            )]),
            "not a 4096-row",
        ),
        (
            "a bank in flops",
            netlist(&[tile("FDRE", "heap_b0_lo_reg", TILED, &strobe(0))]),
            "not a block-RAM tile",
        ),
        ("no bank 0 tile", netlist(&[other.clone()]), "0 tile(s)"),
        (
            "lanes on two strobes",
            netlist(&[tile(
                "RAMB36E1",
                "heap_b0_lo.0.0",
                TILED,
                "_066_[0], _066_[0], _066_[1], _066_[1]",
            )]),
            "different strobes",
        ),
    ] {
        let msg = refusal(bad);
        assert!(
            msg.contains("PRECONDITION") && msg.contains(says),
            "{why}: the edit must refuse with `{says}`, said `{msg}`"
        );
    }
}

/// ★ TEETH — proves this lock CATCHES bug #1, heap accesses past a tile's rows aliasing into tile 0, and is not merely
/// green on today's tree. yosys 0.33 decodes a depth-tiled heap correctly at every depth (MEASURED 2026-09-30, flat and
/// banked, 5120 to 32768 words): the ×9 tiles take `heap_ra[11:0]` on their address pins, and the depth goes to
/// per-tile write strobes and the read mux, so no synthesis parameter reproduces the defect. The lock therefore
/// synthesizes the heap-beyond-tile program with bank 0 depth-tiled (`DEPTH_TILED_HEAP`), removes the depth decode
/// from the tiles' write strobes (`drop_heap_bank0_depth_decode`, which first checks the tiling is there), and asserts
/// the gate-level result diverges from the VM with exactly the aliased value: the `Push` of 999 to `heap[4120]`
/// lands on row 24 of every depth tile, so the read of `heap[24]` returns 999 instead of the VM's 0.
#[test]
#[ignore = "gold-standard RED-teeth gate-level sim (~30-60min); run with --ignored to re-prove the lock catches \
            heap aliasing."]
fn postsynth_reverting_the_heap_fix_diverges_from_the_vm() {
    let mut heap_init: Vec<i64> = vec![20];
    for k in 0..20i64 {
        heap_init.push(50 + k);
    }
    let vm = vm_preloaded(HEAP_BEYOND_TILE_SRC, "acc", 0, &heap_init);
    assert_eq!(
        vm, 0,
        "the VM reads heap[24] = 0 regardless of the synthesized heap depth"
    );

    match synth_edit_and_gatelevel_result(
        "heapbug",
        HEAP_BEYOND_TILE_SRC,
        "acc",
        0,
        &heap_init,
        DEPTH_TILED_HEAP,
        drop_heap_bank0_depth_decode,
    ) {
        None => eprintln!("SKIPPED postsynth_reverting_the_heap_fix_diverges_from_the_vm (yosys/iverilog absent)."),
        Some(gate) => {
            assert_ne!(
                gate, vm,
                "TEETH FAILURE: with heap bank 0's depth decode removed at heap depth {DEPTH_TILED_HEAP}, the \
                 gate-level netlist STILL matched the VM ({vm}), so this lock has no teeth. Expected the netlist to \
                 alias heap[4120] into row 24 and read back 999."
            );
            assert_eq!(
                gate, 999,
                "the divergence must be the alias: heap[24] reads the Push of 999 to heap[4120], on the same row of \
                 the next depth tile, and nothing else"
            );
            println!(
                "POSTSYNTH TEETH CONFIRMED: heap bank 0 without its depth decode makes the SYNTHESIZED netlist DIVERGE \
                 from the VM: heap[24] reads the aliased Push, {gate} != VM {vm} (bug #1, caught by this lock)."
            );
        }
    }
}
