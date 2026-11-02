//! The hart's DDR2 controller, `hart_ddr2`, against the part. Under iverilog it powers the part up, calibrates its read
//! capture and serves line traffic, all against `support/ddr2_device.rs` — the MT47H64M16 at the controller's frame
//! interface, which refuses every command its data sheet forbids and returns reads through a delay-tap eye on a
//! simulated board. On every board whose lanes it can align, the controller must calibrate and then return every line
//! exactly as last written; on a board with no eye, or lanes too far apart, it must say calibration failed and grant
//! nothing.
//!
//! The schedule `hart::ddr2` derives is also checked here against the part's own numbers, cycle by cycle.
//!
//! Tool-gated: `iverilog`.

mod common;
#[path = "support/ddr2_device.rs"]
mod ddr2_device;

use ddr2_device::Board;
use logicaffeine_compile::codegen_sva::hart::ddr2::{self, cks, frames, CK_PS, FRAME_PS, MT47H64M16};
use logicaffeine_compile::codegen_sva::hart::rtl;

/// The schedule the controller runs, replayed in CK cycles against every constraint the part states: a read line and
/// a write line back to back in one bank, ACTIVATE by ACTIVATE, burst by burst.
#[test]
fn the_schedule_keeps_every_timing_of_the_part() {
    let p = MT47H64M16;
    let s = ddr2::schedule(&p);
    let ck = |frame: u64| 2 * frame;
    let wl = p.cl - 1;
    // ACTIVATE → first column command.
    assert!(ck(s.rcd) * CK_PS >= p.trcd, "tRCD");
    // Column commands a burst apart.
    assert_eq!(ck(s.ccd), ddr2::BURST_BEATS / 2, "bursts are not back to back");
    let last = s.cas(ddr2::BURSTS_PER_LINE - 1);
    // A read line: auto-precharge after the last READ, then tRP, before the next ACTIVATE (at read_done + 1, the
    // earliest the controller can start another line).
    let rd_pre = (ck(last) + ddr2::BURST_BEATS / 2 - 2 + cks(p.trtp).max(2)).max(cks(p.tras));
    assert!(ck(s.read_done + 1) >= rd_pre + cks(p.trp), "a read line's bank is not idle when the next line begins");
    assert!(ck(s.read_done + 1) * CK_PS >= p.trc, "tRC after a read line");
    // A write line: precharge WL + BL/2 + WR after the last WRITE.
    let wr_pre = (ck(last) + wl + ddr2::BURST_BEATS / 2 + cks(p.twr)).max(cks(p.tras));
    assert!(ck(s.write_done + 1) >= wr_pre + cks(p.trp), "a write line's bank is not idle when the next line begins");
    // A read after a write line waits tWTR after the write data.
    let wdata_end = ck(last) + wl + ddr2::BURST_BEATS / 2;
    assert!(ck(s.write_done + 1 + s.rcd) >= wdata_end + cks(p.twtr), "tWTR");
    // Write data lands in whole frames, two after the WRITE.
    assert_eq!(wl % 2, 0, "the write latency is not whole frames");
    assert_eq!(wl / 2, 2, "write data is not two frames after its WRITE");
    // Refresh: REFRESH plus tRFC fit, and refreshes stay within tREFI even when a write line delays one.
    assert!(s.rfc * FRAME_PS >= p.trfc, "tRFC");
    assert!((s.refi + s.write_done + 2 + 1) * FRAME_PS <= p.trefi, "a delayed refresh misses tREFI");
    assert!(s.power_up * FRAME_PS >= p.power_up);
    assert_eq!(frames(p.trfc), s.rfc);
}

/// The calibration line identifies itself: each lane's first beat, or its complement, appears nowhere else in the
/// lane, so the scan can lock on at one place only.
#[test]
fn the_calibration_line_has_one_place_to_be_found() {
    let words = ddr2::calibration_line();
    for lane in 0..2 {
        let beats: Vec<u8> = (0..32).map(|b| (words[b / 4] >> (16 * (b % 4) + 8 * lane)) as u8).collect();
        for (i, &x) in beats.iter().enumerate() {
            for (k, &y) in beats.iter().enumerate() {
                if i != k {
                    assert!(x != y && x != !y, "lane {lane}: beats {i} and {k} are {x:#04x} and {y:#04x}");
                }
            }
        }
    }
}

/// The power-up sequence is the JEDEC order the device model holds the controller to, and its waits fit the ROM.
#[test]
fn the_power_up_sequence_is_the_jedec_order() {
    use logicaffeine_compile::codegen_sva::ddr2_phy::{InitStep, ModeReg};
    let seq = ddr2::init_sequence(&MT47H64M16);
    let commands: Vec<String> = seq
        .iter()
        .filter_map(|s| match s {
            InitStep::PrechargeAll => Some("PREA".to_string()),
            InitStep::Refresh => Some("REF".to_string()),
            InitStep::LoadMode { reg, a } => Some(format!("{reg:?}={a:#x}")),
            _ => None,
        })
        .collect();
    let mr_reset = ddr2::mr(&MT47H64M16, true);
    let mr_run = ddr2::mr(&MT47H64M16, false);
    assert_eq!(
        commands,
        [
            "PREA".to_string(),
            format!("{:?}=0x0", ModeReg::Emr2),
            format!("{:?}=0x0", ModeReg::Emr3),
            format!("{:?}=0x0", ModeReg::Emr1),
            format!("{:?}={mr_reset:#x}", ModeReg::Mr),
            "PREA".to_string(),
            "REF".to_string(),
            "REF".to_string(),
            format!("{:?}={mr_run:#x}", ModeReg::Mr),
            format!("{:?}={:#x}", ModeReg::Emr1, 0b111 << 7),
            format!("{:?}=0x0", ModeReg::Emr1),
        ]
    );
    assert_eq!(mr_run & 0b111, 0b011, "BL8");
    assert_eq!((mr_run >> 4) & 0b111, MT47H64M16.cl as u16, "CAS latency");
    assert_eq!(mr_reset, mr_run | 1 << 8, "the first MR resets the DLL, the second does not");
    for step in &seq {
        let _ = ddr2::init_word(step);
    }
}

/// The testbench: the controller, the part, and a requester that first writes every one of its lines (spread over
/// every bank, sixteen rows) and then reads and rewrites them at random, comparing every read with what it last wrote;
/// the line port's write words arrive with random gaps.
fn testbench(board: &Board, seed: u32, ops: u32) -> String {
    format!(
        r#"
`timescale 1ns/1ps
module tb;
  reg clk = 1'b0, rst = 1'b1;
  always #8 clk = ~clk;
  integer seed = {seed};
  reg req = 1'b0, we = 1'b0, wvalid = 1'b0;
  reg [63:0] addr = 64'd0, wdata = 64'd0;
  integer drift = 0;
  wire gnt, wready, rvalid;
  wire [63:0] rdata, status, cal;
  wire cke, cs_n, ras_n, cas_n, we_n, dq_oe, dqs_oe, dqs_toggle, dly_ld, error;
  wire [2:0] ba;
  wire [12:0] a;
  wire [63:0] dq_out, rd_frame;
  wire [9:0] dly_tap;
  wire [31:0] refreshes, lines_read, lines_written;
  hart_ddr2 ctrl (
    .clk(clk), .rst(rst), .req(req), .we(we), .addr(addr), .gnt(gnt), .wdata(wdata), .wvalid(wvalid),
    .wready(wready), .rvalid(rvalid), .rdata(rdata), .status(status), .cal(cal),
    .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a), .dq_oe(dq_oe),
    .dq_out(dq_out), .dqs_oe(dqs_oe), .dqs_toggle(dqs_toggle), .rd_frame(rd_frame), .dly_ld(dly_ld),
    .dly_tap(dly_tap));
  ddr2_device #(.SKEW0({skew0}), .SKEW1({skew1}), .PHASE0({phase0}), .PHASE1({phase1}), .INV0({inv0}),
                .INV1({inv1}), .BLIND({blind}), .SEED({seed})) dev (
    .clk(clk), .rst(rst), .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a),
    .dq_oe(dq_oe), .dq_out(dq_out), .dqs_oe(dqs_oe), .dqs_toggle(dqs_toggle), .rd_frame(rd_frame),
    .dly_ld(dly_ld), .dly_tap(dly_tap), .drift(drift), .error(error), .refreshes(refreshes),
    .lines_read(lines_read), .lines_written(lines_written));

  localparam SLOTS = 48, OPS = {ops};
  reg [63:0] slot_addr [0:SLOTS - 1];
  reg [63:0] ref_line [0:SLOTS * 8 - 1];
  reg [63:0] line [0:7];
  integer n, k, slot, reads, writes, bad, first, granted;
  reg is_write;

  task line_op(input do_write, input integer sl);
    begin
      @(posedge clk);
      req <= 1'b1; we <= do_write; addr <= slot_addr[sl];
      @(posedge clk);
      while (!gnt) @(posedge clk);
      req <= 1'b0;
      if (do_write) begin
        for (k = 0; k < 8; k = k + 1) line[k] = {{$random(seed), $random(seed)}};
        k = 0;
        while (k < 8) begin
          wvalid <= ($random(seed) & 3) != 0;
          wdata <= line[k];
          @(posedge clk);
          if (wvalid && wready) k = k + 1;
        end
        wvalid <= 1'b0;
        for (k = 0; k < 8; k = k + 1) ref_line[sl * 8 + k] = line[k];
        writes = writes + 1;
      end else begin
        k = 0;
        while (k < 8) begin
          @(posedge clk);
          if (rvalid) begin
            if (rdata !== ref_line[sl * 8 + k]) begin
              if (bad < 5) $display("FAIL: line %0d (%h) word %0d: read %h, wrote %h", sl, slot_addr[sl], k, rdata,
                                    ref_line[sl * 8 + k]);
              bad = bad + 1;
            end
            k = k + 1;
          end
        end
        reads = reads + 1;
      end
    end
  endtask

  initial begin
    reads = 0; writes = 0; bad = 0; granted = 0;
    // Distinct lines in rows 0..15 of every bank: 64-byte aligned offsets below 256 KB, a stride apart from a random
    // start (an odd stride visits every line once).
    first = {{$random(seed)}} % 4096;
    for (n = 0; n < SLOTS; n = n + 1) slot_addr[n] = 64'h80000000 | (((first + n * 1237) % 4096) << 6);
    repeat (4) @(posedge clk);
    rst <= 1'b0;
    while (!status[0] && !status[1]) @(posedge clk);
    if (status[1]) begin
      // A controller that failed must not take a request.
      req <= 1'b1; we <= 1'b0; addr <= slot_addr[0];
      repeat (200) begin
        @(posedge clk);
        if (gnt) granted = granted + 1;
      end
      $display("CALIBRATION FAILED status=%h cal=%h granted=%0d error=%0d", status, cal, granted, error);
      $finish;
    end
    $display("CALIBRATED status=%h cal=%h", status, cal);
    // The board warms up: its eye moves.
    drift = {drift};
    for (n = 0; n < SLOTS; n = n + 1) line_op(1'b1, n);
    for (n = 0; n < OPS; n = n + 1) begin
      is_write = ($random(seed) & 3) == 0;
      slot = {{$random(seed)}} % SLOTS;
      line_op(is_write, slot);
    end
    // Idle long enough to need refreshes on its own.
    repeat (2000) @(posedge clk);
    $display("DONE reads=%0d writes=%0d bad=%0d error=%0d refreshes=%0d lines_read=%0d lines_written=%0d",
             reads, writes, bad, error, refreshes, lines_read, lines_written);
    $finish;
  end
  initial begin
    #20000000 $display("TIMEOUT status=%h", status);
    $finish;
  end
endmodule
"#,
        skew0 = board.skew[0],
        skew1 = board.skew[1],
        phase0 = board.phase[0],
        phase1 = board.phase[1],
        inv0 = board.invert[0] as u8,
        inv1 = board.invert[1] as u8,
        blind = board.blind as u8,
        drift = board.drift,
    )
}

/// Compile and run the controller on `board`; the testbench's report.
fn run(board: &Board, seed: u32, ops: u32, label: &str) -> String {
    let dir = std::env::temp_dir().join(format!("hart_ddr2_lock_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in rtl::ddr2_files() {
        std::fs::write(dir.join(name), text).unwrap();
    }
    std::fs::write(dir.join("device.v"), ddr2_device::model(&MT47H64M16)).unwrap();
    std::fs::write(dir.join("tb.v"), testbench(board, seed, ops)).unwrap();
    let sim = dir.join("sim");
    let out = std::process::Command::new("iverilog")
        .args(["-g2012", "-I"])
        .arg(&dir)
        .arg("-o")
        .arg(&sim)
        .arg(dir.join("tb.v"))
        .arg(dir.join("device.v"))
        .arg(dir.join("hart_ddr2.v"))
        .output()
        .expect("run iverilog");
    assert!(out.status.success(), "{label}: does not compile:\n{}", String::from_utf8_lossy(&out.stderr));
    let out = std::process::Command::new("vvp").arg(&sim).output().expect("run vvp");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{label}: vvp failed:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn field(report: &str, key: &str) -> u64 {
    let at = report.find(&format!("{key}=")).unwrap_or_else(|| panic!("no {key} in:\n{report}")) + key.len() + 1;
    report[at..].split(|c: char| !c.is_ascii_hexdigit()).next().unwrap().parse().unwrap()
}

/// Boards the controller must calibrate on: lanes skewed apart, eyes anywhere against the taps (two with a beat edge
/// inside the tap range, so two runs compete), lanes inverted, and every eye drifting five taps one way or the other
/// once calibrated — which only a tap from the middle of the eye survives.
fn good_boards() -> Vec<Board> {
    vec![
        Board { skew: [0, 0], phase: [0, 0], invert: [false, false], drift: -5, blind: false },
        Board { skew: [3, 5], phase: [20, 35], invert: [false, false], drift: 5, blind: false },
        Board { skew: [6, 2], phase: [44, 9], invert: [true, false], drift: -5, blind: false },
        Board { skew: [9, 9], phase: [30, 30], invert: [true, true], drift: 5, blind: false },
        Board { skew: [1, 6], phase: [50, 0], invert: [false, true], drift: 5, blind: false },
    ]
}

#[test]
fn the_controller_calibrates_and_holds_every_line_on_every_board() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (n, board) in good_boards().iter().enumerate() {
        let report = run(board, 0xd2_0001 + n as u32, 400, &format!("good{n}"));
        let label = format!("board {board:?}");
        assert!(report.contains("CALIBRATED"), "{label}: did not calibrate:\n{report}");
        assert!(!report.contains("DDR2 MODEL"), "{label}: the part refused a command:\n{report}");
        assert!(report.contains("DONE"), "{label}: the traffic did not finish:\n{report}");
        assert_eq!(field(&report, "bad"), 0, "{label}: reads differ from what was written:\n{report}");
        assert_eq!(field(&report, "error"), 0, "{label}:\n{report}");
        let (reads, writes) = (field(&report, "reads"), field(&report, "writes"));
        assert_eq!(reads + writes, 48 + 400, "{label}:\n{report}");
        // The part saw every line the requester asked for, plus calibration's write, 32 raw reads and its check.
        assert_eq!(field(&report, "lines_written"), writes + 1, "{label}:\n{report}");
        assert_eq!(field(&report, "lines_read"), reads + 32 + 1, "{label}:\n{report}");
        assert!(field(&report, "refreshes") > 10, "{label}: the part was barely refreshed:\n{report}");
    }
}

#[test]
fn a_board_without_an_eye_fails_calibration_and_is_never_granted() {
    if !common::require_tool("iverilog") {
        return;
    }
    let blind = Board { skew: [2, 2], phase: [10, 10], invert: [false, false], drift: 0, blind: true };
    let report = run(&blind, 0xb11d, 10, "blind");
    assert!(report.contains("CALIBRATION FAILED"), "a blind board calibrated:\n{report}");
    assert!(report.contains("granted=0"), "{report}");
    assert!(!report.contains("DDR2 MODEL"), "the part refused a command:\n{report}");
}

#[test]
fn lanes_too_far_apart_fail_calibration() {
    if !common::require_tool("iverilog") {
        return;
    }
    // Lane 1's first beat lands fourteen beats after lane 0's: past what a lane's sixteen-beat history can hold beside
    // the other.
    let apart = Board { skew: [0, 14], phase: [10, 10], invert: [false, false], drift: 0, blind: false };
    let report = run(&apart, 0xa9a7, 10, "apart");
    assert!(report.contains("CALIBRATION FAILED"), "lanes fourteen beats apart calibrated:\n{report}");
}
