// tb_term_activity — the activity panel on the screen, its probes driven by the test instead of an engine. The test
// writes `plan.txt`, one step a line:
//
//   E                               at the next frame's start, snapshot an idle engine (the counters start over)
//   S pc cell n                     n samples of a running engine at block `pc`, cell `cell`, one a clock
//   I pc cell n                     n samples of an engine that is not running, its probes at `pc` and `cell`
//   R                               hold the engine's reset for a while (`engine_held`), as the top does on a reset
//   C running pc cell sp hp cycles  hold these probes, take the next frame's snapshot of them, and capture that
//                                   frame into frame<k>.hex (640 x 480 lines of 0xRGB), logging `FRAME k`
//   G n                             samples from here on `n` board clocks apart (1 at the start; the board's are an
//                                   engine clock apart)
//   P i v                           set run total `i` (blocks first, then cells) to `v`
//   T i                             read run total `i` back through `act_idx`, logging `TOTAL i v`
//
// Samples are never closer to a snapshot than the sweep that follows it, as on the board, where they are an
// engine clock apart. The log (`board.txt`) ends with `DONE`.
`timescale 1ns/1ps
module tb_term_activity;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg rst = 1'b1;

  wire [12:0] vga_addr;
  wire [15:0] vga_data, a_rdata;
  term_textbuf textbuf (
    .clk(clk), .a_we(1'b0), .a_addr(13'd0), .a_wdata(16'd0), .a_rdata(a_rdata), .b_addr(vga_addr), .b_rdata(vga_data));

  reg         running = 1'b0, sample = 1'b0, engine_held = 1'b0;
  reg  [15:0] pc = 16'd0, cell_ix = 16'd0, sp = 16'd0, hp = 16'd0;
  reg  [31:0] cycles = 32'd0;
  reg  [5:0]  act_idx = 6'd0;
  wire [31:0] act_total;
  wire [9:0]  px, py;
  wire [1:0]  phase;
  wire        pix, frame_start, ov_on;
  wire [11:0] ov_rgb;
  wire [3:0]  r, g, b;
  wire        hs, vs;
  term_vga vga (
    .clk(clk), .rst(rst), .tb_addr(vga_addr), .tb_data(vga_data),
    .win_row(7'd0), .win_col(7'd0), .win_rows(7'd0), .win_cols(7'd0),
    .cursor_row(7'd0), .cursor_col(7'd0), .cursor_on(1'b0), .pointer_row(7'd0), .pointer_col(7'd0), .pointer_on(1'b0),
    .ov_on(ov_on), .ov_rgb(ov_rgb), .px(px), .py(py), .pix(pix), .phase(phase), .frame_start(frame_start),
    .r(r), .g(g), .b(b), .hs(hs), .vs(vs));
  term_activity activity (
    .clk(clk), .rst(rst), .sample(sample), .engine_held(engine_held), .running(running), .pc(pc), .cell_ix(cell_ix), .sp(sp), .hp(hp),
    .cycles(cycles), .px(px), .py(py), .phase(phase), .frame_start(frame_start), .ov_on(ov_on), .ov_rgb(ov_rgb),
    .act_idx(act_idx), .act_total(act_total));

  integer log;
  initial log = $fopen("board.txt", "w");

  // Frame capture: the colour registered at a pixel enable belongs to the position the beam had at that enable.
  reg [9:0] cx = 10'd0, cy = 10'd0;
  reg       sampled = 1'b0;
  always @(posedge clk) begin
    sampled <= pix;
    if (pix) begin cx <= px; cy <= py; end
  end
  reg     capturing = 1'b0, armed = 1'b0;
  integer frame_fd, nframe = 0;
  always @(posedge clk) if (sampled) begin
    if (armed && cx == 10'd0 && cy == 10'd0) begin
      armed <= 1'b0;
      capturing <= 1'b1;
      frame_fd = $fopen($sformatf("frame%0d.hex", nframe), "w");
      $fdisplay(log, "FRAME %0d", nframe);
    end
    if ((capturing || (armed && cx == 10'd0 && cy == 10'd0)) && cx < 10'd640 && cy < 10'd480)
      $fdisplay(frame_fd, "%03h", {r, g, b});
    if (capturing && cx == 10'd639 && cy == 10'd479) begin
      capturing <= 1'b0;
      $fclose(frame_fd);
      nframe = nframe + 1;
    end
  end

  // The snapshot at the next frame's start: one sample just after it, then a sweep's worth of quiet.
  task snapshot;
    begin
      @(posedge clk); while (!frame_start) @(posedge clk);
      @(posedge clk); sample <= 1'b1;
      @(posedge clk); sample <= 1'b0;
      repeat (__N_MAXBLK__ + __N_MAXCELL__ + 8) @(posedge clk);
    end
  endtask

  integer gap = 1;
  task samples(input [31:0] n);
    integer k, w;
    begin
      for (k = 0; k < n; k = k + 1) begin
        @(posedge clk); sample <= 1'b1;
        for (w = 1; w < gap; w = w + 1) begin @(posedge clk); sample <= 1'b0; end
      end
      @(posedge clk); sample <= 1'b0;
    end
  endtask

  integer fd, n;
  reg [8*4:1] op;
  reg [31:0] a0, a1, a2, a3, a4, a5;
  initial begin
    fd = $fopen("plan.txt", "r");
    repeat (8) @(posedge clk);
    rst <= 1'b0;
    while (!$feof(fd)) begin
      n = $fscanf(fd, "%s", op);
      if (n == 1) begin
        if (op == "E") begin running <= 1'b0; snapshot; end
        else if (op == "S") begin
          n = $fscanf(fd, "%d %d %d", a0, a1, a2);
          running <= 1'b1; pc <= a0[15:0]; cell_ix <= a1[15:0];
          samples(a2);
        end
        else if (op == "I") begin
          n = $fscanf(fd, "%d %d %d", a0, a1, a2);
          running <= 1'b0; pc <= a0[15:0]; cell_ix <= a1[15:0];
          samples(a2);
        end
        else if (op == "R") begin
          running <= 1'b0; engine_held <= 1'b1;
          repeat (4 * __N_ENGINE_CLOCK_DIVIDER__) @(posedge clk);
          engine_held <= 1'b0;
        end
        else if (op == "G") begin
          n = $fscanf(fd, "%d", a0);
          gap = a0;
        end
        else if (op == "P") begin
          n = $fscanf(fd, "%d %d", a0, a1);
          repeat (4) @(posedge clk);
          if (a0 < __N_MAXBLK__) activity.btotal[a0] = a1; else activity.ctotal[a0 - __N_MAXBLK__] = a1;
        end
        else if (op == "T") begin
          n = $fscanf(fd, "%d", a0);
          repeat (4) @(posedge clk);
          act_idx <= a0[5:0];
          repeat (4) @(posedge clk);
          $fdisplay(log, "TOTAL %0d %0d", a0, act_total);
        end
        else if (op == "C") begin
          n = $fscanf(fd, "%d %d %d %d %d %d", a0, a1, a2, a3, a4, a5);
          running <= a0[0]; pc <= a1[15:0]; cell_ix <= a2[15:0]; sp <= a3[15:0]; hp <= a4[15:0]; cycles <= a5;
          armed = 1'b1;
          snapshot;
          wait (capturing); wait (!capturing);
        end
      end
    end
    $fdisplay(log, "DONE");
    $fclose(log);
    $finish;
  end
endmodule
