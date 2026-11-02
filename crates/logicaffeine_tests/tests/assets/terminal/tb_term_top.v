// tb_term_top — a Fabric Terminal top driven like the board: a host on its UART at the link's baud rate, a PS/2
// device on its open-collector lines, and a capture of whole VGA frames. The test writes `script.txt`, one step a
// line, and reads back what the board sent and what it drew:
//
//   B hh   the host sends byte hh (the script carries SLIP-encoded frames byte by byte)
//   W n    wait n microseconds
//   K hh   the PS/2 device sends byte hh
//   A      the PS/2 device takes one host-to-device byte and acknowledges it (logs `DEV hh parity_ok`)
//   F      capture the next whole frame into frame<k>.hex (640 x 480 lines of 0xRGB) and log `FRAME k flash f`
//   R      log the 7-segment value `RESULT hhhhhhhh`
//   T      wait for the board's next ENGINE_TAKEN report
//   E      wait until the engine runs a program (a top with an engine, `TERM_ENGINE` defined)
//
// Every byte the board sends is logged as `RX hh`, in order; the log ends with `DONE`. With `GATE_LEVEL` the top is
// a synthesized netlist: its parameters were set before synthesis and its internal nets are gone, so the flash
// phase a frame was drawn in is logged as `?`.
//
// With `TERM_ENGINE`, the activity panel is held to the engine's own clock: every engine clock a program runs is
// counted here by block and by cell, straight from the engine's registers, and logged at the end as
// `REF_BLOCKS c0 c1 …`, `REF_CELLS c0 c1 …` and `REF_OUTSIDE n` (clocks at a block or cell the panel has no place
// for). Every snapshot the panel takes is logged as `SNAP n running pc cell sp hp cycles`, with the counts it turns
// into levels (`BLOCKS n …`, `CELLS n …`), and once they are drawn-ready, `LEVELS n b0 … | c0 …` and `DIGITS n hh…`.
`timescale 1ns/1ps
module tb_term_top;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  localparam BIT_NS = __N_CLKS_PER_BIT__ * 10;

  reg  uart_in = 1'b1;
  wire uart_out;
  wire ps2_clk_w, ps2_data_w;
  pullup (ps2_clk_w);
  pullup (ps2_data_w);
  reg dev_clk_low = 1'b0, dev_data_low = 1'b0;
  assign ps2_clk_w  = dev_clk_low  ? 1'b0 : 1'bz;
  assign ps2_data_w = dev_data_low ? 1'b0 : 1'bz;
  wire [3:0] r, g, b;
  wire hs, vs;
  wire [6:0] seg;
  wire dp;
  wire [7:0] an;
  wire [15:0] led;

`ifndef GATE_LEVEL
  // The flash phase turns every frame here, so consecutive frames show both phases.
  defparam dut.vga.FLASH_BIT = 0;
`endif
  __TOP__ dut (
    .clk(clk), .uart_rx(uart_in), .uart_tx(uart_out), .ps2_clk(ps2_clk_w), .ps2_data(ps2_data_w),
    .vga_r(r), .vga_g(g), .vga_b(b), .vga_hs(hs), .vga_vs(vs), .seg(seg), .dp(dp), .an(an), .led(led));

  integer log;
  initial log = $fopen("board.txt", "w");

  // The host's receiver: every byte the board sends, logged.
  integer i;
  reg [7:0] rxb;
  reg       after_end = 1'b1;
  integer   taken = 0, taken_seen = 0;
  initial begin
    forever begin
      @(negedge uart_out);
      #(BIT_NS / 2);
      for (i = 0; i < 8; i = i + 1) begin
        #(BIT_NS);
        rxb[i] = uart_out;
      end
      #(BIT_NS);
      $fdisplay(log, "RX %02h", rxb);
      if (after_end && rxb == __V_B_ENGINE_TAKEN__) taken = taken + 1;
      after_end = (rxb == __V_SLIP_END__);
    end
  end

  task host_byte(input [7:0] v);
    integer k;
    begin
      uart_in = 1'b0; #(BIT_NS);
      for (k = 0; k < 8; k = k + 1) begin uart_in = v[k]; #(BIT_NS); end
      uart_in = 1'b1; #(BIT_NS);
    end
  endtask

  task dev_bit(input bv);
    begin
      #20000; dev_data_low = !bv;
      #20000; dev_clk_low = 1'b1;
      #40000; dev_clk_low = 1'b0;
    end
  endtask
  task dev_send(input [7:0] v);
    integer k;
    begin
      dev_bit(1'b0);
      for (k = 0; k < 8; k = k + 1) dev_bit(v[k]);
      dev_bit(~^v);
      dev_bit(1'b1);
      dev_data_low = 1'b0;
      #100000;
    end
  endtask
  reg [10:0] dev_rx;
  task dev_take;
    integer k;
    begin
      wait (ps2_clk_w == 1'b0);
      wait (ps2_clk_w == 1'b1);
      for (k = 0; k < 10; k = k + 1) begin
        #40000; dev_clk_low = 1'b1;
        #40000; dev_clk_low = 1'b0;
        #1000;  dev_rx[k] = ps2_data_w;
      end
      #20000; dev_data_low = 1'b1;
      #20000; dev_clk_low = 1'b1;
      #40000; dev_clk_low = 1'b0;
      #20000; dev_data_low = 1'b0;
      $fdisplay(log, "DEV %02h %0d", dev_rx[7:0], ((^dev_rx[8:0]) == 1'b1) && dev_rx[9]);
    end
  endtask

  // Frame capture: the colour registered at a pixel enable belongs to the position the beam had at that enable.
  reg [9:0] cx = 10'd0, cy = 10'd0;
  reg       sampled = 1'b0;
  always @(posedge clk) begin
    sampled <= dut.vga.pix;
    if (dut.vga.pix) begin cx <= dut.vga.px; cy <= dut.vga.py; end
  end
  reg     capturing = 1'b0, armed = 1'b0;
  integer frame_fd, nframe = 0;
  always @(posedge clk) if (sampled) begin
    if (armed && cx == 10'd0 && cy == 10'd0) begin
      armed <= 1'b0;
      capturing <= 1'b1;
      frame_fd = $fopen($sformatf("frame%0d.hex", nframe), "w");
`ifdef GATE_LEVEL
      $fdisplay(log, "FRAME %0d flash ?", nframe);
`else
      $fdisplay(log, "FRAME %0d flash %0d", nframe, dut.vga.flash);
`endif
    end
    if ((capturing || (armed && cx == 10'd0 && cy == 10'd0)) && cx < 10'd640 && cy < 10'd480)
      $fdisplay(frame_fd, "%03h", {r, g, b});
    if (capturing && cx == 10'd639 && cy == 10'd479) begin
      capturing <= 1'b0;
      $fclose(frame_fd);
      nframe = nframe + 1;
    end
  end

`ifdef TERM_ENGINE
  localparam NB = __N_MAXBLK__, NC = __N_MAXCELL__;
  integer ref_b [0:NB-1];
  integer ref_c [0:NC-1];
  integer q, ref_outside = 0, nsnap = 0;
  initial begin
    for (q = 0; q < NB; q = q + 1) ref_b[q] = 0;
    for (q = 0; q < NC; q = q + 1) ref_c[q] = 0;
  end
  wire [15:0] eng_cell = dut.engine.c_cellptr[15:0] + dut.engine.ci;
  always @(posedge dut.coreclk) if (dut.engine.ph == dut.engine.P_RUN) begin
    if (dut.engine.pc < NB) ref_b[dut.engine.pc] = ref_b[dut.engine.pc] + 1; else ref_outside = ref_outside + 1;
    if (eng_cell < NC) ref_c[eng_cell] = ref_c[eng_cell] + 1; else ref_outside = ref_outside + 1;
  end
  reg drawn_ready = 1'b0;
  always @(posedge clk) begin
    if (dut.activity.sweep == 7'd1) begin
      $fwrite(log, "SNAP %0d %0d %0d %0d %0d %0d %0d\n", nsnap, dut.activity.cur_running, dut.activity.cur_pc,
              dut.activity.cur_cell, dut.activity.cur_sp, dut.activity.cur_hp, dut.activity.dd_bin);
      $fwrite(log, "BLOCKS %0d", nsnap);
      for (q = 0; q < NB; q = q + 1) $fwrite(log, " %0d", dut.activity.bhits[q]);
      $fwrite(log, "\nCELLS %0d", nsnap);
      for (q = 0; q < NC; q = q + 1) $fwrite(log, " %0d", dut.activity.chits[q]);
      $fwrite(log, "\n");
      drawn_ready <= 1'b1;
    end else if (drawn_ready && dut.activity.sweep == 7'd0 && dut.activity.lvl_at == 7'd0 && dut.activity.dd_n == 6'd0) begin
      $fwrite(log, "LEVELS %0d", nsnap);
      for (q = 0; q < NB; q = q + 1) $fwrite(log, " %0d", dut.activity.blevel[q]);
      $fwrite(log, " |");
      for (q = 0; q < NC; q = q + 1) $fwrite(log, " %0d", dut.activity.clevel[q]);
      $fwrite(log, "\nDIGITS %0d %h\n", nsnap, dut.activity.digits);
      drawn_ready <= 1'b0;
      nsnap = nsnap + 1;
    end
  end
`endif

  integer fd, n;
  reg [8*4:1] op;
  reg [31:0] arg;
  initial begin
    fd = $fopen("script.txt", "r");
    #2000;
    while (!$feof(fd)) begin
      n = $fscanf(fd, "%s", op);
      if (n == 1) begin
        if (op == "B") begin n = $fscanf(fd, "%h", arg); host_byte(arg[7:0]); end
        else if (op == "W") begin n = $fscanf(fd, "%d", arg); #(arg * 1000); end
        else if (op == "K") begin n = $fscanf(fd, "%h", arg); dev_send(arg[7:0]); end
        else if (op == "A") dev_take;
        else if (op == "F") begin armed = 1'b1; wait (capturing); wait (!capturing); end
        else if (op == "R") $fdisplay(log, "RESULT %08h", dut.link.result);
        else if (op == "T") begin wait (taken > taken_seen); taken_seen = taken_seen + 1; end
`ifdef TERM_ENGINE
        else if (op == "E") wait (dut.probe_running);
`endif
      end
    end
    #(BIT_NS * 200);
`ifdef TERM_ENGINE
    $fwrite(log, "REF_BLOCKS");
    for (q = 0; q < NB; q = q + 1) $fwrite(log, " %0d", ref_b[q]);
    $fwrite(log, "\nREF_CELLS");
    for (q = 0; q < NC; q = q + 1) $fwrite(log, " %0d", ref_c[q]);
    $fwrite(log, "\nREF_OUTSIDE %0d\n", ref_outside);
`endif
    $fdisplay(log, "DONE");
    $fclose(log);
    $finish;
  end
endmodule
