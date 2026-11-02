//! qor_dram_lock — the K7 traffic gate. One stream of 64-bit words through each DDR2 controller of the comparative
//! bench into a model of the Nexys A7's part at DDR2-400, the bus held to JEDEC DDR2 (codegen_sva::ddr2_traffic,
//! codegen_sva::ddr2_jedec): every word read back is the word written, every request reaches the bus, and a
//! controller that breaks a timing rule is refused however correct its data. LOGOS's DFI controller also carries
//! random traffic — any mix of reads and writes, bank conflicts and row hits, held-back write data and byte masks, and
//! stretches long enough that postponed refresh must be repaid — through a model of the PHY, every command held to
//! JEDEC and every word read back the one last written. Each simulation needs Icarus Verilog; those on LiteDRAM's
//! PHY model also the LiteX environment and yosys; each skips loud without them.

mod common;

use logicaffeine_compile::codegen_sva::ddr2_dfi::emit_ddr2_dfi_controller_verilog;
use logicaffeine_compile::codegen_sva::ddr2_jedec::{check_bus, Ddr2Timing};
use logicaffeine_compile::codegen_sva::ddr2_phy::emit_ddr2_controller_verilog;
use logicaffeine_compile::codegen_sva::ddr2_traffic::{
    emit_ddr2_ctrl_traffic_testbench, emit_ddr2_device_verilog, emit_ddr2_dfi_ctrl_traffic_testbench,
    emit_litedram_traffic_testbench, judge, k7_controller, k7_ddr2_ctrl_verilog, k7_dfi_ctrl_verilog, k7_timing, read_log,
    Judgement, Pattern, Traffic, K7_GRADE,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fresh(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("qor_dram_lock").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Compile `sources` in `dir` with iverilog under top `tb_qor_dram` (or `top`) and run it there; the simulator's
/// output.
fn simulate(dir: &Path, top: &str, sources: &[&str]) -> String {
    let build = Command::new("iverilog")
        .current_dir(dir)
        .args(["-g2005", "-s", top, "-o", "sim.vvp"])
        .args(sources)
        .output()
        .expect("run iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir).args(["-n", "sim.vvp"]).output().expect("run vvp");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// `ddr2_ctrl` emitted from `rtl` carrying `traffic`, judged at `t`.
fn ddr2_ctrl_run(name: &str, rtl: &str, t: &Ddr2Timing, traffic: Traffic) -> (String, Result<Judgement, String>) {
    let dir = fresh(name);
    std::fs::write(dir.join("words.hex"), traffic.rom()).unwrap();
    std::fs::write(dir.join("ddr2_ctrl.v"), rtl).unwrap();
    std::fs::write(dir.join("ddr2_device.v"), emit_ddr2_device_verilog(t)).unwrap();
    std::fs::write(dir.join("tb_qor_dram.v"), emit_ddr2_ctrl_traffic_testbench(t, &traffic)).unwrap();
    let out = simulate(&dir, "tb_qor_dram", &["tb_qor_dram.v", "ddr2_ctrl.v", "ddr2_device.v"]);
    let log = read_log(&std::fs::read_to_string(dir.join("dram.log")).expect("the bench writes dram.log")).expect("the log reads");
    let iface = k7_controller("k7_logos_ddr2_ctrl").unwrap().iface;
    (out, judge(t, &traffic, iface, &log))
}

#[test]
fn ddr2_ctrl_carries_every_stream_under_jedec() {
    if !common::require_tool("iverilog") {
        return;
    }
    let t = k7_timing(8);
    for pattern in [Pattern::Sequential, Pattern::Scattered, Pattern::Probe] {
        let traffic = Traffic { pattern, words: 256 };
        let (out, j) = ddr2_ctrl_run(&format!("ddr2_ctrl_{}", pattern.label()), &k7_ddr2_ctrl_verilog(), &t, traffic);
        assert!(out.contains("PASS"), "{out}");
        let j = j.unwrap_or_else(|e| panic!("{}: {e}", pattern.label()));
        assert_eq!(j.requests, 8 * 256, "four 16-bit requests a word, each way");
        assert_eq!(j.bus.activates, u64::from(j.requests), "closed page: every request opens its row");
        assert_eq!(j.bus.precharges, u64::from(j.requests));
        assert_eq!(j.row_hits, 0);
        // A request is taken only when the controller is idle with no refresh pending, so from acceptance on its
        // transaction runs the same every time: ACT the clock after, the column command tRCD later.
        for (what, l) in [("write", j.write_latency), ("read", j.read_latency)] {
            assert_eq!((l.min, l.max), (1 + t.trcd, 1 + t.trcd), "{}: {what} latency", pattern.label());
        }
        assert!(j.bus.refreshes + 8 >= j.bus_span / t.trefi, "{} refreshes over {} clocks", j.bus.refreshes, j.bus_span);
        assert!(j.read_service.min > j.read_latency.min + t.cl, "a word is four reads, each a whole transaction");
    }
}

/// The gate's teeth: a controller whose data is right but whose bus is not is refused, named by the rule it broke.
#[test]
fn a_controller_that_breaks_a_bus_rule_is_refused_whatever_its_data() {
    if !common::require_tool("iverilog") {
        return;
    }
    let t = k7_timing(8);
    let traffic = Traffic { pattern: Pattern::Sequential, words: 16 };
    for (name, table, rule) in [
        ("no_write_recovery", logicaffeine_compile::codegen_sva::dram::DramTimings { twr: 0, ..t.dram_timings() }, "WRITE to PRECHARGE"),
        ("short_trcd", logicaffeine_compile::codegen_sva::dram::DramTimings { trcd: t.trcd - 1, ..t.dram_timings() }, "tRCD"),
        ("short_tras", logicaffeine_compile::codegen_sva::dram::DramTimings { tras: 0, ..t.dram_timings() }, "tRAS"),
    ] {
        let (out, j) = ddr2_ctrl_run(name, &emit_ddr2_controller_verilog(&table), &t, traffic);
        assert!(out.contains("PASS"), "{name}: the device model does not judge timing, so its data still checks: {out}");
        let e = j.err().unwrap_or_else(|| panic!("{name}: a controller breaking {rule} was accepted"));
        assert!(e.contains(rule), "{name}: {e}");
    }
}

/// `ddr2_device` stores a burst in the sequential order that wraps inside its aligned block, leaves a masked beat
/// alone, returns a burst CL clocks after its READ, and refuses a row past the ones it stores.
#[test]
fn the_device_wraps_bursts_and_refuses_rows_it_does_not_store() {
    if !common::require_tool("iverilog") {
        return;
    }
    let t = k7_timing(8);
    let dir = fresh("device");
    std::fs::write(dir.join("ddr2_device.v"), emit_ddr2_device_verilog(&t)).unwrap();
    let tb = r#"`timescale 1ns/1ps
module tb_device;
  reg clk = 0; always #2.5 clk = ~clk;
  reg rst = 1;
  reg cs_n = 1, ras_n = 1, cas_n = 1, we_n = 1; reg [2:0] ba = 0; reg [12:0] a = 0;
  reg [127:0] wr_burst = 0; reg [7:0] wr_mask = 8'hff;
  wire [127:0] rd_burst; wire rd_valid, model_error;
  integer i, clocks;
  ddr2_device #(.ROWB(1)) dev(.clk(clk), .rst(rst), .cke(1'b1), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n),
    .ba(ba), .a(a), .wr_burst(wr_burst), .wr_mask(wr_mask), .rd_burst(rd_burst), .rd_valid(rd_valid), .model_error(model_error));
  task cmd(input [2:0] p, input [2:0] b, input [12:0] ad); begin
    @(negedge clk); cs_n = 0; {ras_n, cas_n, we_n} = p; ba = b; a = ad;
    @(negedge clk); cs_n = 1; {ras_n, cas_n, we_n} = 3'b111;
  end endtask
  initial begin
    repeat (3) @(negedge clk); rst = 0;
    cmd(3'b011, 0, 1);
    for (i = 0; i < 8; i = i + 1) wr_burst[16 * i +: 16] = 16'h1000 + i;
    wr_mask = 8'b0000_0100;
    cmd(3'b100, 0, 5);
    wr_mask = 8'hff;
    cmd(3'b101, 0, 0);
    clocks = 0;
    while (!rd_valid) begin @(negedge clk); clocks = clocks + 1; end
    $display("LATENCY %0d", clocks);
    for (i = 0; i < 8; i = i + 1) $display("BEAT %0d %h", i, rd_burst[16 * i +: 16]);
    $display("ERROR %0d", model_error);
    cmd(3'b011, 1, 2);
    @(posedge clk); @(posedge clk);
    $display("ERROR %0d", model_error);
    $finish;
  end
endmodule
"#;
    std::fs::write(dir.join("tb_device.v"), tb).unwrap();
    let out = simulate(&dir, "tb_device", &["tb_device.v", "ddr2_device.v"]);
    // Beats 0..7 of the WRITE at column 5 land on columns 5, 6, 7, 0, 1, 2, 3, 4; beat 2 (column 7) is masked.
    let want = ["1003", "1004", "1005", "1006", "1007", "1000", "1001", "xxxx"];
    for (i, w) in want.iter().enumerate() {
        assert!(out.contains(&format!("BEAT {i} {w}")), "beat {i} should be {w}:\n{out}");
    }
    let latency = out.lines().find_map(|l| l.strip_prefix("LATENCY ")).expect("a read came back");
    assert_eq!(latency.trim().parse::<u64>().unwrap(), t.cl, "the burst returns CL clocks after its READ:\n{out}");
    let errors: Vec<&str> = out.lines().filter(|l| l.starts_with("ERROR")).collect();
    assert_eq!(errors, ["ERROR 0", "ERROR 1"], "row 2 is past the one row bit stored:\n{out}");
}

/// LiteDRAM's controller as the bench synthesises it, built from the datasheet-complete module, wired to LiteDRAM's
/// own SDRAMPHYModel: every stream
/// legal, every word back, and the sequential stream almost all row hits.
#[test]
fn litedram_carries_every_stream_under_jedec() {
    if !common::require_tool("iverilog") || !common::require_tool("yosys") || !common::require_tool("litex") {
        return;
    }
    let c = k7_controller("k7_litedram_ctrl").unwrap();
    let t = k7_timing(c.bl);
    let python = root().join("tools/qor/litex-venv/bin/python");
    let generator = root().join("benchmarks/silicon/qor/kernels/K7/litedram_controller.py");
    for pattern in [Pattern::Sequential, Pattern::Scattered, Pattern::Probe] {
        let traffic = Traffic { pattern, words: 256 };
        let dir = fresh(&format!("litedram_{}", pattern.label()));
        let row_bits = traffic.row_bits().to_string();
        for (top, extra) in [("litedram_ctrl", vec![]), ("litedram_phy_model", vec!["--phy-model", "--row-bits", row_bits.as_str()])] {
            let rtl = dir.join(format!("{top}_rtl.v"));
            let g = Command::new(&python).arg(&generator).args(&extra).arg(&rtl).output().expect("run the generator");
            assert!(g.status.success(), "{top}: {}", String::from_utf8_lossy(&g.stderr));
            let y = Command::new("yosys")
                .args(["-q", "-p"])
                .arg(format!(
                    "read_verilog {}; hierarchy -top {top}; proc; check -assert; opt_clean; write_verilog -noattr {}",
                    rtl.display(),
                    dir.join(format!("{top}.v")).display()
                ))
                .output()
                .expect("run yosys");
            assert!(y.status.success(), "{top}: {}", String::from_utf8_lossy(&y.stderr));
        }
        std::fs::write(dir.join("words.hex"), traffic.rom()).unwrap();
        std::fs::write(dir.join("tb_qor_dram.v"), emit_litedram_traffic_testbench(&t, &traffic)).unwrap();
        let out = simulate(&dir, "tb_qor_dram", &["tb_qor_dram.v", "litedram_ctrl.v", "litedram_phy_model.v"]);
        assert!(out.contains("PASS"), "{out}");
        let log = read_log(&std::fs::read_to_string(dir.join("dram.log")).unwrap()).unwrap();
        let j = judge(&t, &traffic, c.iface, &log).unwrap_or_else(|e| panic!("{}: {e}", pattern.label()));
        assert_eq!(j.requests, 2 * 256, "one request a word, each way");
        let cas = j.bus.reads + j.bus.writes;
        assert_eq!(cas, 2 * 256, "one BL4 burst a word");
        if pattern == Pattern::Sequential {
            assert!(j.row_hits * 10 >= cas * 9, "{} row hits of {cas}", j.row_hits);
        }
    }
}

/// LiteDRAM's SDRAMPHYModel for `traffic`, generated by the bench's generator and elaborated by yosys `proc` in `dir`.
fn litedram_phy_model(dir: &Path, traffic: &Traffic) {
    let python = root().join("tools/qor/litex-venv/bin/python");
    let generator = root().join("benchmarks/silicon/qor/kernels/K7/litedram_controller.py");
    let rtl = dir.join("litedram_phy_model_rtl.v");
    let g = Command::new(&python)
        .arg(&generator)
        .args(["--phy-model", "--row-bits", &traffic.row_bits().to_string()])
        .arg(&rtl)
        .output()
        .expect("run the generator");
    assert!(g.status.success(), "{}", String::from_utf8_lossy(&g.stderr));
    let y = Command::new("yosys")
        .args(["-q", "-p"])
        .arg(format!(
            "read_verilog {}; hierarchy -top litedram_phy_model; proc; check -assert; opt_clean; write_verilog -noattr {}",
            rtl.display(),
            dir.join("litedram_phy_model.v").display()
        ))
        .output()
        .expect("run yosys");
    assert!(y.status.success(), "{}", String::from_utf8_lossy(&y.stderr));
}

/// LOGOS's DFI controller in LiteDRAM's place — the same native port, the same SDRAMPHYModel, the same testbench —
/// carries every stream under JEDEC: one BL4 burst a word, no explicit precharge (a row it will not read again closes
/// with its last column command), the sequential stream all but its first opens row hits within a few clocks of its
/// payload, the scattered stream within three times its payload, and a read that finds it idle back on the bus within
/// its ACT, column command, CAS latency and burst and the clocks its registers add.
#[test]
fn the_dfi_controller_carries_every_stream_on_litedrams_phy_model() {
    if !common::require_tool("iverilog") || !common::require_tool("yosys") || !common::require_tool("litex") {
        return;
    }
    let c = k7_controller("k7_logos_dfi_ctrl").unwrap();
    assert_eq!((c.bl, c.iface.parts, c.iface.cols_per_part), (4, 1, 4), "a word a request, one BL4 burst");
    let t = k7_timing(c.bl);
    for pattern in [Pattern::Sequential, Pattern::Scattered, Pattern::Probe] {
        let traffic = Traffic { pattern, words: 256 };
        let dir = fresh(&format!("dfi_{}", pattern.label()));
        litedram_phy_model(&dir, &traffic);
        std::fs::write(dir.join("words.hex"), traffic.rom()).unwrap();
        std::fs::write(dir.join("ddr2_dfi_ctrl.v"), k7_dfi_ctrl_verilog()).unwrap();
        std::fs::write(dir.join("tb_qor_dram.v"), emit_ddr2_dfi_ctrl_traffic_testbench(&t, &traffic)).unwrap();
        let out = simulate(&dir, "tb_qor_dram", &["tb_qor_dram.v", "ddr2_dfi_ctrl.v", "litedram_phy_model.v"]);
        assert!(out.contains("PASS: 256 words through ddr2_dfi_ctrl"), "{out}");
        let log = read_log(&std::fs::read_to_string(dir.join("dram.log")).unwrap()).unwrap();
        let j = judge(&t, &traffic, c.iface, &log).unwrap_or_else(|e| panic!("{}: {e}", pattern.label()));
        let label = pattern.label();
        assert_eq!(j.requests, 2 * 256, "{label}: one request a word, each way");
        assert_eq!(j.bus.reads + j.bus.writes, 2 * 256, "{label}: one BL4 burst a word");
        assert!(j.bus.precharges <= j.bus.refreshes, "{label}: a precharge only before a refresh; a row closes with its last read or write");
        match pattern {
            Pattern::Sequential => {
                assert!(j.bus.activates <= 2 + j.bus.refreshes, "{label}: the stream's one row opens once a pass, and after a refresh");
                assert!(j.stream_cycles <= j.payload_cycles + 48, "{label}: {} clocks for a payload of {}", j.stream_cycles, j.payload_cycles);
            }
            Pattern::Scattered => assert!(j.stream_cycles <= 3 * j.payload_cycles, "{label}: {} clocks for {}", j.stream_cycles, j.payload_cycles),
            Pattern::Probe => {
                // the request's register and the ACT's (two system clocks), ACT on phase 1, then tRCD, CL and the burst
                let idle = 2 * 2 + 1 + t.trcd + t.cl + t.bl / 2;
                assert!(j.read_bus.p50 <= idle, "{label}: a read on an idle controller, {} clocks", j.read_bus.p50);
            }
        }
    }
}

/// The DFI part of `ddr2_dfi_ctrl`'s testbenches: the x16 part behind an ideal PHY at 1:2, as SDRAMPHYModel models it —
/// write data taken with its WRITE, read data on both phases `RL` system clocks after its READ through registers, phase
/// 1's command after phase 0's — storing 64-bit words for `ROWB` row bits. Out of reset, a command to a bank in the
/// wrong state (ACT to an open bank, a column command to a closed one, REFRESH with one open), a row past those
/// stored, or an unknown command pin raises `err`; timing is the JEDEC checker's.
const DFI_MODEL: &str = r#"module dfi_model #(parameter ROWB = 2, RL = 8) (
  input clk, input rst,
  input [12:0] p0_a, input [2:0] p0_ba, input p0_cs_n, p0_ras_n, p0_cas_n, p0_we_n, input [31:0] p0_wd, input [3:0] p0_wm,
  input [12:0] p1_a, input [2:0] p1_ba, input p1_cs_n, p1_ras_n, p1_cas_n, p1_we_n, input [31:0] p1_wd, input [3:0] p1_wm,
  output [31:0] p0_rd, output [31:0] p1_rd, output rv, output reg err);
  reg [63:0] mem [0:(8 << (ROWB + 8)) - 1];
  reg openb [0:7]; reg [12:0] orow [0:7];
  reg [63:0] pd [1:RL]; reg pv [1:RL];
  reg rd_now; reg [63:0] rd_word;
  integer k;
  initial begin err = 0; for (k = 0; k < 8; k = k + 1) openb[k] = 0; for (k = 1; k <= RL; k = k + 1) pv[k] = 0;
    for (k = 0; k < (8 << (ROWB + 8)); k = k + 1) mem[k] = 64'd0; end
  assign rv = pv[RL]; assign p0_rd = pd[RL][31:0]; assign p1_rd = pd[RL][63:32];
  task phase(input cs, input ras, input cas, input we, input [2:0] ba, input [12:0] a, input [63:0] wd, input [7:0] wm);
    reg [ROWB+10:0] w; integer by;
    begin
      if ({cs, ras, cas, we} === 4'bxxxx || ^{cs, ras, cas, we} === 1'bx) begin err = 1; $display("FAIL: an unknown command pin"); end
      else if (!cs) case ({ras, cas, we})
        3'b011: begin
          if (openb[ba]) begin err = 1; $display("FAIL: ACT to open bank %0d", ba); end
          if (a >= (13'd1 << ROWB)) begin err = 1; $display("FAIL: ACT to row %0d past the stored", a); end
          openb[ba] = 1; orow[ba] = a;
        end
        3'b010: if (a[10]) for (k = 0; k < 8; k = k + 1) openb[k] = 0; else openb[ba] = 0;
        3'b001: for (k = 0; k < 8; k = k + 1) if (openb[k]) begin err = 1; $display("FAIL: REFRESH with bank %0d open", k); end
        3'b100, 3'b101: begin
          if (!openb[ba]) begin err = 1; $display("FAIL: column command to closed bank %0d", ba); end
          w = {ba, orow[ba][ROWB-1:0], a[9:2]};
          if (!we) begin for (by = 0; by < 8; by = by + 1) if (!wm[by]) mem[w][8 * by +: 8] = wd[8 * by +: 8]; end
          else begin rd_now = 1; rd_word = mem[w]; end
          if (a[10]) openb[ba] = 0;
        end
        default: ;
      endcase
    end
  endtask
  always @(posedge clk) begin
    rd_now = 0; rd_word = 64'd0;
    if (!rst) begin
      phase(p0_cs_n, p0_ras_n, p0_cas_n, p0_we_n, p0_ba, p0_a, {p1_wd, p0_wd}, {p1_wm, p0_wm});
      phase(p1_cs_n, p1_ras_n, p1_cas_n, p1_we_n, p1_ba, p1_a, {p1_wd, p0_wd}, {p1_wm, p0_wm});
    end
    pv[1] <= rd_now; pd[1] <= rd_word;
    for (k = RL; k > 1; k = k - 1) begin pv[k] <= pv[k - 1]; pd[k] <= pd[k - 1]; end
  end
endmodule
"#;

/// A random native-port master for `ddr2_dfi_ctrl`: `%N%` requests drawn from seed `%SEED%`, each the word after the
/// last (a row run) or a random word of a 256-word region (eight banks, four rows, eight columns: conflicts and hits),
/// a write half the time with a random byte mask; requests held back `%GAP%`% of clocks outside requests `%SAT0%` to
/// `%SAT1%` (a saturated stretch), write data `%WGAP%`%. Valid is held until accepted. A shadow of the region gives
/// every read its expected word in request order; the bus goes to `dram.log` in DRAM clocks.
const RANDOM_TB: &str = r#"`timescale 1ns/1ps
module tb_rand;
  localparam integer N = %N%, GAP = %GAP%, WGAP = %WGAP%, SAT0 = %SAT0%, SAT1 = %SAT1%;
  localparam [63:0] SEED = 64'd%SEED%;
  reg clk = 1'b0; always #5 clk = ~clk;
  reg rst = 1'b1;
  function [63:0] mix(input [63:0] x);
    reg [63:0] z;
    begin z = x + 64'h9E3779B97F4A7C15; z = (z ^ (z >> 30)) * 64'hBF58476D1CE4E5B9; z = (z ^ (z >> 27)) * 64'h94D049BB133111EB; mix = z ^ (z >> 31); end
  endfunction
  function [63:0] wdata_of(input [63:0] n); wdata_of = mix(SEED ^ 64'hD1B54A32D192ED03 ^ n); endfunction
  function [7:0] be_of(input [63:0] d); be_of = d[63] ? 8'hff : d[7:0] | 8'h01; endfunction
  reg [63:0] s, k;
  reg [63:0] shadow [0:255];
  reg [63:0] expect [0:N-1];
  integer r, wsent, writes, reads, got, errors, f, i, r_n, wsent_n;
  reg pending, wpending, pending_n, wpending_n;
  reg [7:0] last;
  wire [63:0] h = mix(SEED ^ (r * 64'd7919));
  wire [7:0] word = h[1] && r > 0 ? last + 8'd1 : h[17:10];
  wire we = h[2];
  wire [23:0] addr = {11'd0, word[7:6], word[5:3], 5'd0, word[2:0]};
  wire [63:0] wdat = wdata_of(wsent);
  wire [7:0] wbe = be_of(wdat);
  wire cmd_valid = pending, wdata_valid = wpending;
  wire cmd_ready, wdata_ready, rdata_valid;
  wire [63:0] rdata;
  wire [63:0] wfill = wdata_of(writes);
  wire [12:0] p0_a, p1_a; wire [2:0] p0_ba, p1_ba;
  wire p0_cs_n, p0_ras_n, p0_cas_n, p0_we_n, p1_cs_n, p1_ras_n, p1_cas_n, p1_we_n, rv, err;
  wire [31:0] p0_wd, p1_wd, p0_rd, p1_rd; wire [3:0] p0_wm, p1_wm;
  ddr2_dfi_ctrl ctrl(.sys_clk(clk), .sys_rst(rst),
    .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_payload_we(we), .cmd_payload_addr(addr),
    .wdata_valid(wdata_valid), .wdata_ready(wdata_ready), .wdata_payload_data(wdat), .wdata_payload_we(wbe),
    .rdata_valid(rdata_valid), .rdata_payload_data(rdata),
    .dfi_p0_address(p0_a), .dfi_p0_bank(p0_ba), .dfi_p0_cs_n(p0_cs_n), .dfi_p0_ras_n(p0_ras_n), .dfi_p0_cas_n(p0_cas_n),
    .dfi_p0_we_n(p0_we_n), .dfi_p0_wrdata(p0_wd), .dfi_p0_wrdata_mask(p0_wm), .dfi_p0_rddata(p0_rd), .dfi_p0_rddata_valid(rv),
    .dfi_p1_address(p1_a), .dfi_p1_bank(p1_ba), .dfi_p1_cs_n(p1_cs_n), .dfi_p1_ras_n(p1_ras_n), .dfi_p1_cas_n(p1_cas_n),
    .dfi_p1_we_n(p1_we_n), .dfi_p1_wrdata(p1_wd), .dfi_p1_wrdata_mask(p1_wm), .dfi_p1_rddata(p1_rd), .dfi_p1_rddata_valid(rv));
  dfi_model #(.ROWB(2)) part(.clk(clk), .rst(rst),
    .p0_a(p0_a), .p0_ba(p0_ba), .p0_cs_n(p0_cs_n), .p0_ras_n(p0_ras_n), .p0_cas_n(p0_cas_n), .p0_we_n(p0_we_n), .p0_wd(p0_wd), .p0_wm(p0_wm),
    .p1_a(p1_a), .p1_ba(p1_ba), .p1_cs_n(p1_cs_n), .p1_ras_n(p1_ras_n), .p1_cas_n(p1_cas_n), .p1_we_n(p1_we_n), .p1_wd(p1_wd), .p1_wm(p1_wm),
    .p0_rd(p0_rd), .p1_rd(p1_rd), .rv(rv), .err(err));
  // what the controller samples changes only with the clock's nonblocking updates
  always @(posedge clk) begin
    if (rst) begin
      k <= 0; r <= 0; wsent <= 0; pending <= 1'b0; wpending <= 1'b0; last <= 8'd0;
      writes = 0; reads = 0; got = 0; s = SEED;
    end else begin
      k <= k + 1;
      if (!p0_cs_n && !(p0_ras_n && p0_cas_n && p0_we_n))
        $fwrite(f, "C %0d %0d %0d %0d %0d %0d %0d\n", 2 * k, p0_cs_n, p0_ras_n, p0_cas_n, p0_we_n, p0_ba, p0_a);
      if (!p1_cs_n && !(p1_ras_n && p1_cas_n && p1_we_n))
        $fwrite(f, "C %0d %0d %0d %0d %0d %0d %0d\n", 2 * k + 1, p1_cs_n, p1_ras_n, p1_cas_n, p1_we_n, p1_ba, p1_a);
      if (rdata_valid) begin
        if (got >= reads || rdata !== expect[got]) begin
          errors = errors + 1;
          if (errors < 5) $display("FAIL: read %0d returned %h, expected %h", got, rdata, expect[got]);
        end
        got = got + 1;
      end
      r_n = r; wsent_n = wsent; pending_n = pending; wpending_n = wpending;
      if (wdata_valid && wdata_ready) begin wsent_n = wsent + 1; wpending_n = 1'b0; end
      if (cmd_valid && cmd_ready) begin
        if (we) begin
          for (i = 0; i < 8; i = i + 1) if (be_of(wfill) >> i & 1) shadow[word][8 * i +: 8] = wfill[8 * i +: 8];
          writes = writes + 1;
        end else begin expect[reads] = shadow[word]; reads = reads + 1; end
        last <= word; r_n = r + 1; pending_n = 1'b0;
      end
      s = s ^ (s << 13); s = s ^ (s >> 7); s = s ^ (s << 17);
      if (!pending_n && r_n < N && (r_n >= SAT0 && r_n < SAT1 || s[15:0] % 100 >= GAP)) pending_n = 1'b1;
      if (!wpending_n && wsent_n < writes && s[31:16] % 100 >= WGAP) wpending_n = 1'b1;
      r <= r_n; wsent <= wsent_n; pending <= pending_n; wpending <= wpending_n;
    end
  end
  initial begin
    f = $fopen("dram.log", "w");
    errors = 0;
    for (i = 0; i < 256; i = i + 1) shadow[i] = 64'd0;
    repeat (4) @(posedge clk);
    rst <= 1'b0;
    wait (r == N && got == reads || k > 64'd400000);
    repeat (64) @(posedge clk);
    if (r != N || got != reads) begin errors = errors + 1; $display("FAIL: %0d of %0d requests taken, %0d of %0d reads back", r, N, got, reads); end
    if (err) errors = errors + 1;
    $fwrite(f, "END %0d %0d\n", 2 * k, errors);
    $fclose(f);
    $display("RESULT %0d requests %0d reads %0d errors", r, reads, errors);
    $finish;
  end
endmodule
"#;

/// Run `ddr2_dfi_ctrl` for `t` under the random master; the simulator's output and the log's bus.
fn dfi_random_run(name: &str, t: &Ddr2Timing, fields: &[(&str, u64)]) -> (String, logicaffeine_compile::codegen_sva::ddr2_traffic::Log) {
    let dir = fresh(name);
    let mut tb = RANDOM_TB.to_string();
    for (k, v) in fields {
        tb = tb.replace(&format!("%{k}%"), &v.to_string());
    }
    for (k, _) in fields {
        assert!(!tb.contains(&format!("%{k}%")), "%{k}% unfilled");
    }
    std::fs::write(dir.join("tb_rand.v"), tb).unwrap();
    std::fs::write(dir.join("dfi_model.v"), DFI_MODEL).unwrap();
    std::fs::write(dir.join("ddr2_dfi_ctrl.v"), emit_ddr2_dfi_controller_verilog(t)).unwrap();
    let out = simulate(&dir, "tb_rand", &["tb_rand.v", "dfi_model.v", "ddr2_dfi_ctrl.v"]);
    let log = read_log(&std::fs::read_to_string(dir.join("dram.log")).expect("the bench writes dram.log")).expect("the log reads");
    (out, log)
}

/// Any traffic, at two operating points: every command `ddr2_dfi_ctrl` puts on the bus is legal JEDEC DDR2 (the
/// checker over the whole trace, refresh debt included), every read returns the word last written there through its
/// byte mask, and every request is served — under idle gaps and held-back write data, and through saturated
/// stretches long enough that the postponed refreshes must be repaid while requests still wait.
#[test]
fn the_dfi_controller_keeps_jedec_and_every_word_under_random_traffic() {
    if !common::require_tool("iverilog") {
        return;
    }
    let ddr2_533 = K7_GRADE.at(3750, 4).unwrap();
    for (name, t, n, gap, wgap, sat, seed) in [
        ("idle_gaps", k7_timing(4), 4000, 40, 20, (0, 0), 1u64),
        ("held_data", k7_timing(4), 4000, 10, 70, (0, 0), 2),
        ("saturated", k7_timing(4), 12000, 30, 0, (1000, 11000), 3),
        ("ddr2_533", ddr2_533, 8000, 15, 15, (2000, 7000), 4),
    ] {
        let fields = [("N", n), ("GAP", gap), ("WGAP", wgap), ("SAT0", sat.0), ("SAT1", sat.1), ("SEED", seed)];
        let (out, log) = dfi_random_run(&format!("dfi_random_{name}"), &t, &fields);
        assert!(out.contains(&format!("RESULT {n} requests")) && out.contains(" 0 errors"), "{name}: {out}");
        let facts = check_bus(&t, &log.bus).unwrap_or_else(|v| panic!("{name}: {v}"));
        assert_eq!(facts.reads + facts.writes, n, "{name}: one column command a request");
        assert!(facts.activates > n / 8 && facts.refreshes > 0, "{name}: conflicts opened rows and refresh ran");
        use logicaffeine_compile::codegen_sva::ddr2_phy::Ddr2Cmd;
        for c in log.bus.iter().filter(|c| c.cmd == Ddr2Cmd::Precharge) {
            assert!(c.a & 1 << 10 != 0, "{name}: a PRECHARGE of one bank at cycle {}: no request finds its bank open on another row", c.cycle);
        }
        assert!(facts.precharges <= facts.refreshes, "{name}: a precharge-all only before a refresh");
        if sat.1 > sat.0 {
            let (first, last) = (log.bus.iter().find(|c| c.cmd == logicaffeine_compile::codegen_sva::ddr2_phy::Ddr2Cmd::Read || c.cmd == logicaffeine_compile::codegen_sva::ddr2_phy::Ddr2Cmd::Write).unwrap().cycle, facts.last_cycle);
            assert!((last - first) / t.trefi > 9, "{name}: the run outlasts nine refresh intervals");
        }
    }
}
