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
