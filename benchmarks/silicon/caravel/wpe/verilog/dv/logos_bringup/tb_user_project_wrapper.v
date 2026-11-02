// SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.
// SPDX-License-Identifier: MIT
// tb_wpe_soc_caravel — the OVER-THE-WIRE HOST for the wpe_soc engine. Reads the concatenated wire bytes
// (wire_caravel.hex, one byte/line), per-vector byte lengths (lens_caravel.hex), and the eval_wprog_config
// oracle (oracle_caravel.hex); for each vector it bit-bangs that vector's bytes onto uart_rx, decodes the
// returned frame (0xA5 sync + result[63:0] MSB-first + cycles[31:0] + 0x5A), and checks result == oracle.
// SHOW records (0xA7 tag payload) that arrive before the frame are printed as `SHOW`/`SHOWT`/`SHOWB` lines.
// Proves the WHOLE wire -> on-chip synthesize -> run -> wire path ≡ the VM under iverilog.
`timescale 1ns/1ps
module tb_wpe_soc_caravel;
  localparam CLKS = 413;
  localparam NVEC = 3;
  localparam MAXB = 2739;
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
  wire psram_ce_n, psram_sclk; wire [3:0] psram_sio_out, psram_sio_oe, psram_sio;
  wire [37:0] io_out, io_oeb; wire [37:0] io_in;
  genvar hk;
  generate for (hk = 0; hk < 38; hk = hk + 1) begin : idle_io
    if (hk != 8 && hk != 12 && hk != 13 && hk != 14 && hk != 15) begin : quiet
      assign io_in[hk] = 1'b0;
    end
  end endgenerate
  assign io_in[8] = uart_rx;
  assign #(5.550) io_in[12] = psram_sio[0];
  assign #(5.550) io_in[13] = psram_sio[1];
  assign #(5.550) io_in[14] = psram_sio[2];
  assign #(5.550) io_in[15] = psram_sio[3];
  assign uart_tx = io_out[9];
  assign #(10.120) psram_ce_n = io_out[10];
  assign #(10.120) psram_sclk = io_out[11];
  assign #(10.120) psram_sio_out[0] = io_out[12]; assign #(10.320) psram_sio_oe[0] = ~io_oeb[12];
  assign #(10.120) psram_sio_out[1] = io_out[13]; assign #(10.320) psram_sio_oe[1] = ~io_oeb[13];
  assign #(10.120) psram_sio_out[2] = io_out[14]; assign #(10.320) psram_sio_oe[2] = ~io_oeb[14];
  assign #(10.120) psram_sio_out[3] = io_out[15]; assign #(10.320) psram_sio_oe[3] = ~io_oeb[15];
  wire wbs_ack_o; wire [31:0] wbs_dat_o; wire [127:0] la_data_out; wire [2:0] user_irq;
  // The management SoC's clock, at the period Caravel's chip signoff times it at; the engine's is clk.
  reg wb_clk = 0;
  always #12.5 wb_clk = ~wb_clk;
  user_project_wrapper dut (
    .wb_clk_i(wb_clk), .wb_rst_i(rst), .wbs_stb_i(1'b0), .wbs_cyc_i(1'b0), .wbs_we_i(1'b0), .wbs_sel_i(4'd0),
    .wbs_dat_i(32'd0), .wbs_adr_i(32'd0), .wbs_ack_o(wbs_ack_o), .wbs_dat_o(wbs_dat_o),
    .la_data_in(128'd0), .la_data_out(la_data_out), .la_oenb({128{1'b1}}),
    .io_in(io_in), .io_out(io_out), .io_oeb(io_oeb), .user_clock2(clk), .user_irq(user_irq));
  // After reset every output pin drives and every other non-SIO pin is an input.
  always @(posedge clk) if (!rst && (io_oeb[9] || io_oeb[10] || io_oeb[11] || ~&{io_oeb[0], io_oeb[1], io_oeb[2], io_oeb[3], io_oeb[4], io_oeb[5], io_oeb[6], io_oeb[7], io_oeb[8], io_oeb[16], io_oeb[17], io_oeb[18], io_oeb[19], io_oeb[20], io_oeb[21], io_oeb[22], io_oeb[23], io_oeb[24], io_oeb[25], io_oeb[26], io_oeb[27], io_oeb[28], io_oeb[29], io_oeb[30], io_oeb[31], io_oeb[32], io_oeb[33], io_oeb[34], io_oeb[35], io_oeb[36], io_oeb[37]})) begin
    $display("PIN-DIRECTION io_oeb=%b", io_oeb);
    $display("FAIL: a Caravel pin is driven the wrong way");
    $finish;
  end
  always #10.5 clk = ~clk;
  // The board: the PSRAM's four SIO pads, driven by whichever side enables them, and the part itself.
  genvar pad;
  generate for (pad = 0; pad < 4; pad = pad + 1) begin : sio_pads
    assign psram_sio[pad] = psram_sio_oe[pad] ? psram_sio_out[pad] : 1'bz;
  end endgenerate
  psram_model part(.ce_n(psram_ce_n), .sclk(psram_sclk), .sio(psram_sio));
  // Every received byte must be taken before the next one completes: the receiver holds one byte while the
  // engine is stalled on the part, so a byte is lost exactly when another completes during that stall.
  // `part_stall` counts the stalled cycles, which the result frame's run-cycle count excludes.
  integer rx_done = 0, rx_taken = 0, part_stall = 0;
  always @(posedge clk) if (!rst) begin
    if (dut.mprj.engine.mstall) part_stall = part_stall + 1;
    if (dut.mprj.engine.rx_valid && !dut.mprj.engine.mstall) rx_taken = rx_taken + 1;
    if (dut.mprj.engine.r_state == 2'd3 && dut.mprj.engine.r_cnt == CLKS - 1) begin
      if (rx_done != rx_taken) begin
        $display("UART-LOST byte %0d", rx_taken);
        $display("FAIL: a received byte was overwritten before the engine took it");
        $finish;
      end
      rx_done = rx_done + 1;
    end
  end

  reg [7:0]  wirebytes [0:MAXB-1];
  reg [31:0] lens   [0:NVEC-1];
  reg [63:0] oracle [0:NVEC-1];
  integer k, fk, vi, bi, cursor, errors;   // k is used INSIDE send_byte/recv_byte; fk for the frame-decode loops
  reg [7:0]  rb, sync, endb;
  reg [7:0]  tag; reg [31:0] slen; reg [63:0] sval; integer sj;   // SHOW record decode
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
    $readmemh("wire_caravel.hex",   wirebytes);
    $readmemh("lens_caravel.hex",   lens);
    $readmemh("oracle_caravel.hex", oracle);
    errors = 0; cursor = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (vi=0; vi<NVEC; vi=vi+1) begin
      for (bi=0; bi<lens[vi]; bi=bi+1) begin send_byte(wirebytes[cursor]); cursor = cursor + 1; end
      recv_byte(sync);
      // SHOW records (0xA7, tag, payload) stream BEFORE the result frame; print each for the harness to check.
      while (sync == 8'hA7) begin
        recv_byte(tag);
        if (tag == 8'd2) begin
          slen = 32'd0; for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); slen = {slen[23:0], rb}; end
          $display("SHOWT len=%0d", slen);
          for (sj=0; sj<slen; sj=sj+1) begin recv_byte(rb); $display("SHOWB %02x", rb); end
        end else begin
          sval = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); sval = {sval[55:0], rb}; end
          $display("SHOW t=%0d v=%016x", tag, sval);
        end
        recv_byte(sync);
      end
      result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {result[55:0], rb}; end
      cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {cyc[23:0], rb}; end
      recv_byte(endb);
      $display("FRAME result=%016x cycles=%0d end=%02x", result, cyc, endb);
      if (sync!==8'hA5 || endb!==8'h5A || result!==oracle[vi]) begin
        errors = errors + 1;
        $display("FAIL vec %0d: sync=%02x result=%0d exp=%0d end=%02x", vi, sync, result, oracle[vi], endb);
      end
    end
    if (errors==0) $display("PASS: %0d over-the-wire runs match eval_wprog_config", NVEC);
    else $display("FAIL: %0d over-the-wire mismatches", errors);
    $display("PSRAM-VIOLATIONS %0d", part.violations);
    $display("PSRAM-STALL-CYCLES %0d", part_stall);
    $finish;
  end

  initial begin #20000000000 $display("FAIL: timeout"); $finish; end
endmodule
// psram_model — a pin-level model of the APS6404L-3SQN (AP Memory APS6404L-3SQN QSPI PSRAM datasheet, Rev. 2.7 (2024-01-05): §8 power-up initialization, §9.1-9.5 address space, page size and command table, §16.7 Table 5 READ/WRITE timing (3.3 V package, standard grade)).
// Emitted by `cargo run -p logicaffeine-compile --example emit_psram_edge` (codegen_sva::psram).
`timescale 1ns/1ps
module psram_model (input ce_n, input sclk, inout [3:0] sio);
  localparam real T_PU = 150000.000, T_CEM = 8000.000, T_CPH = 18.000, T_RST = 50.000, T_CLK = 9.170, T_ACLK = 5.500;
  reg [7:0] mem [0:8388607];
  reg qpi = 1'b0, armed = 1'b0, ready = 1'b0, wrapped = 1'b0, drive = 1'b0;
  reg [3:0] dout = 4'b0000;
  assign sio = drive ? dout : 4'bzzzz;
  integer violations = 0, nedge = 0;
  reg [7:0] cmd, wbyte;
  reg [23:0] addr;
  realtime t_fall = 0.0, t_rise = -1.0e9, t_rst_end = -1.0e9, t_last = 0.0;

  // An SIO bit nobody drives resolves to z; a driven unknown value is x, and only the first is a fault.
  function floating(input [3:0] v);
    floating = (v[0] === 1'bz) || (v[1] === 1'bz) || (v[2] === 1'bz) || (v[3] === 1'bz);
  endfunction
  task violation(input [8*8-1:0] rule);
    begin $display("PSRAM-VIOLATION %0s %0t", rule, $realtime); violations = violations + 1; end
  endtask

  // The byte `k` bytes into the burst: a burst wraps to the start of its page.
  function [22:0] at(input [23:0] a, input integer k);
    reg [10:0] off;
    begin off = a[9:0] + k; at = {a[22:10], off[9:0]}; end
  endfunction
  task note_wrap(input [23:0] a, input integer k);
    begin if (!wrapped && a[9:0] + k >= 1024) begin wrapped = 1'b1; violation("wrap"); end end
  endtask

  always @(negedge ce_n) begin
    if ($realtime < T_PU) violation("tPU");
    if ($realtime - t_rise < T_CPH) violation("tCPH");
    if ($realtime - t_rst_end < T_RST) violation("tRST");
    t_rst_end = -1.0e9;
    t_fall = $realtime; nedge = 0; cmd = 8'h00; addr = 24'h0; wrapped = 1'b0;
  end

  always @(posedge ce_n) begin
    if ($realtime - t_fall > T_CEM) violation("tCEM");
    t_rise = $realtime; drive = 1'b0;
    if (!qpi && nedge == 8) begin
      if (cmd == 8'h99 && armed) begin ready = 1'b1; qpi = 1'b0; t_rst_end = $realtime; end
      else if (cmd == 8'h35 && ready) qpi = 1'b1;
      armed = (cmd == 8'h66);
    end else if (qpi && nedge == 2) begin
      if (cmd == 8'h99 && armed) begin ready = 1'b1; qpi = 1'b0; t_rst_end = $realtime; end
      armed = (cmd == 8'h66);
    end else armed = 1'b0;
  end

  always @(posedge sclk) begin
    if ($realtime < T_PU) violation("tPU");
    if (!ce_n) begin
      if (nedge > 0 && $realtime - t_last < T_CLK) violation("tCLK");
      t_last = $realtime;
      if (!qpi && nedge < 8) begin
        if (sio[0] === 1'bz) violation("float");
        cmd = {cmd[6:0], sio[0]};
        if (nedge == 7 && !(cmd == 8'h66 || cmd == 8'h99 || (cmd == 8'h35 && ready)))
          violation(ready ? "cmd" : "init");
      end else if (qpi && nedge < 2) begin
        if (floating(sio)) violation("float");
        cmd = {cmd[3:0], sio};
        if (nedge == 1 && cmd != 8'heb && cmd != 8'h38 && cmd != 8'h02 && cmd != 8'h66 && cmd != 8'h99)
          violation("cmd");
      end else if (qpi) begin
        if (nedge < 8) begin
          if (floating(sio)) violation("float");
          addr = {addr[19:0], sio};
        end else if (cmd == 8'h38 || cmd == 8'h02) begin
          if (floating(sio)) violation("float");
          if ((nedge - 8) % 2 == 0) wbyte[7:4] = sio;
          else begin
            wbyte[3:0] = sio;
            note_wrap(addr, (nedge - 8) / 2);
            mem[at(addr, (nedge - 8) / 2)] = wbyte;
          end
        end else if (drive && sio !== dout) violation("contend");
      end
      nedge = nedge + 1;
    end
  end

  // Read data: nibble n follows the falling edge after rising edge 14 + n, valid tACLK later.
  always @(negedge sclk) begin
    if (!ce_n && qpi && cmd == 8'heb && nedge >= 14) begin : drive_nibble
      reg [7:0] b; integer n;
      n = nedge - 14;
      note_wrap(addr, n / 2);
      b = mem[at(addr, n / 2)];
      #(T_ACLK);
      if (!ce_n) begin dout = (n % 2 == 0) ? b[7:4] : b[3:0]; drive = 1'b1; end
    end
  end
endmodule
