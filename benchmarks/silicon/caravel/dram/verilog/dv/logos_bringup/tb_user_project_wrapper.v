// SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
// SPDX-License-Identifier: MIT
// tb_user_project_wrapper — bring-up verification of the CARAVEL TOP CELL we would fabricate.
//
// WHY THIS EXISTS, and why the existing tb_dram_executor_wb is not enough. That testbench drives
// `dram_executor_wb` — an INNER module. What gets manufactured is `user_project_wrapper`, and
// between the two sit `dram_user_project` (the full-interface macro) and the wrapper's own 1:1 pin
// mapping. A mis-wired pin there is invisible to an inner-module test, passes LVS (the netlist and
// the layout agree with each other — on the same mistake), and is only discovered on silicon.
//
// It also covers what nothing covered:
//   * the Wishbone READBACK registers 0x0c / 0x10. The only post-fab way to read this chip is the
//     management SoC polling those two addresses, and no test had ever read them.
//   * a NON-ZERO delta. The executor is a COUNTDOWN player — waiting the programmed number of
//     cycles before issuing is its entire purpose — and every prior test used delta = 0, which is
//     the one value that exercises no counting at all.
//   * the GPIO contract: the macro ties `io_oeb` high and drives `io_out` low, and `user_defines.v`
//     configures all 38 pads as inputs to match. If the RTL ever starts driving a pad while that
//     file still says input, the pad fights the pad driver on real silicon.
//
// Command word (RTL dram_executor.v:34-37, encoder codegen_sva/dram.rs:783 — the SAME layout):
//     [63:40] delta   [39:37] op   [36:32] bank   [31:0] row
`timescale 1ns/1ps
`define MPRJ_IO_PADS 38

module tb_user_project_wrapper;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1;
  reg stb = 0, cyc = 0, we = 0;
  reg [3:0]  sel = 4'hf;
  reg [31:0] adr = 0, wdat = 0;
  wire        ack;
  wire [31:0] rdat;
  wire [127:0] la_out;
  wire [`MPRJ_IO_PADS-1:0] io_out, io_oeb;
  integer errors = 0;

  user_project_wrapper dut (
    .wb_clk_i(clk), .wb_rst_i(rst),
    .wbs_stb_i(stb), .wbs_cyc_i(cyc), .wbs_we_i(we),
    .wbs_sel_i(sel), .wbs_dat_i(wdat), .wbs_adr_i(adr),
    .wbs_ack_o(ack), .wbs_dat_o(rdat),
    .la_data_in(128'd0), .la_data_out(la_out), .la_oenb({128{1'b1}}),
    .io_in({`MPRJ_IO_PADS{1'b0}}), .io_out(io_out), .io_oeb(io_oeb),
    .analog_io(), .user_clock2(1'b0), .user_irq()
  );

  task wb_write(input [31:0] a, input [31:0] d);
    begin
      @(posedge clk); adr <= a; wdat <= d; we <= 1; stb <= 1; cyc <= 1;
      @(posedge clk); while (!ack) @(posedge clk);
      stb <= 0; cyc <= 0; we <= 0; @(posedge clk);
    end
  endtask

  // A READ is the post-fab observation path, so it is exercised exactly as firmware would.
  task wb_read(input [31:0] a, output [31:0] d);
    begin
      @(posedge clk); adr <= a; we <= 0; stb <= 1; cyc <= 1;
      @(posedge clk); while (!ack) @(posedge clk);
      d = rdat;
      stb <= 0; cyc <= 0; @(posedge clk);
    end
  endtask

  task check(input [63:0] got, input [63:0] want, input [511:0] what);
    begin
      if (got !== want) begin
        errors = errors + 1;
        $display("FAIL: %0s = %0d, expected %0d", what, got, want);
      end
    end
  endtask

  // Issue one command and verify it through BOTH observation paths.
  task run_cmd(input [23:0] delta, input [2:0] op, input [4:0] bank, input [31:0] row);
    reg [31:0] st, rw;
    reg [7:0]  before_n;
    integer waited;
    begin
      // NO reset between commands — that was the workaround the sticky flag forced. Sample the
      // counter first; the command is complete when it has advanced.
      wb_read(32'h0c, st); before_n = st[15:8];
      wb_write(32'h00, row);
      wb_write(32'h04, {delta, op, bank});      // cmd_word[63:32]
      wb_write(32'h08, 32'h1);                  // launch
      // POLL THE ISSUE COUNTER — exactly the handshake post-fab firmware uses. No reset, no
      // fixed delay, no sticky flag: wait until the counter this command will tick actually
      // ticks. This is the assertion that the defect is fixed; against the old sticky-flag RTL
      // the loop below would fall straight through and the checks would read stale fields.
      waited = 0;
      while (la_out[47:40] === before_n && waited < (delta + 400)) begin
        @(posedge clk); waited = waited + 1;
      end
      check({56'd0, la_out[47:40]}, {56'd0, (before_n + 8'd1)}, "issue_count advanced by exactly 1");
      repeat (2) @(posedge clk);

      check({63'd0, la_out[63]}, 64'd1, "issued (la)");
      check({59'd0, la_out[39:35]}, {59'd0, bank}, "bank (la)");
      check({61'd0, la_out[34:32]}, {61'd0, op},   "op (la)");
      check({32'd0, la_out[31:0]},  {32'd0, row},  "row (la)");

      // The Wishbone readback path — the ONLY way firmware sees this chip post-fab.
      wb_read(32'h0c, st);
      wb_read(32'h10, rw);
      check({63'd0, st[31]},      64'd1,           "issued (wb 0x0c)");
      check({56'd0, st[15:8]},    {56'd0, (before_n + 8'd1)}, "issue_count (wb 0x0c)");
      check({59'd0, st[7:3]},     {59'd0, bank},   "bank (wb 0x0c)");
      check({61'd0, st[2:0]},     {61'd0, op},     "op (wb 0x0c)");
      check({32'd0, rw},          {32'd0, row},    "row (wb 0x10)");
    end
  endtask

  initial begin
    repeat (3) @(posedge clk); rst = 0; @(posedge clk);

    // The GPIO contract the fabricated pad ring depends on.
    check({26'd0, io_oeb}, {26'd0, {`MPRJ_IO_PADS{1'b1}}}, "io_oeb all-inputs");
    check({26'd0, io_out}, 64'd0,                          "io_out driven low");

    run_cmd(24'd0,  3'd1, 5'd3,  32'd7);          // the legacy delta-0 case
    run_cmd(24'd17, 3'd5, 5'd21, 32'hDEAD_BEEF);  // a real countdown, wide row
    run_cmd(24'd3,  3'd7, 5'd31, 32'hFFFF_FFFF);  // max op / max bank / all-ones row

    if (errors == 0)
      $display("PASS: user_project_wrapper bring-up — 3 commands via Wishbone, verified on both la_data_out and the 0x0c/0x10 readback, GPIO contract held");
    else
      $display("FAIL: %0d wrapper bring-up error(s)", errors);
    $finish;
  end
endmodule

// ── DEFECT FOUND BY THIS TESTBENCH, AND FIXED, 2026-09-21 ──────────────────────────────────
// First run of this file found that `issued_c` was a STICKY flag — set on issue, cleared only by
// reset — so firmware polling the status register read the PREVIOUS command's `issued = 1` and
// could not tell its own command had gone out. Two identical back-to-back commands were
// indistinguishable in every visible field. Since the management SoC reading these registers is
// the user area's ONLY post-fab observation path, that made command sequencing unreliable on
// real silicon, and this testbench could only work around it by resetting before each command.
//
// FIXED in dram_executor_wb by adding `issue_count`, a monotonic 8-bit counter ticked once per
// issued command and exposed at 0x0c[15:8] and la_data_out[47:40] — both previously zero, so
// every pre-existing field keeps its bit position. The workaround is gone from this file: each
// command now samples the counter, launches, and polls until it advances by exactly one. Against
// the old RTL that poll falls through immediately and the checks read stale fields, so this
// testbench now FAILS on the defect it found.
// ────────────────────────────────────────────────────────────────────────────────────────────