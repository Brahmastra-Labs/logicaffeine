// term_activity — the Fabric Terminal's activity panel: the fabric engine's own state, drawn by the board every
// frame over the bottom of the screen, from the engine's probes alone.
//
// The probes live in the engine's clock domain; they are read only on `sample`, a board-clock strobe once per engine
// clock at a fixed phase after the engine's edge, when every engine register has settled. Each sample, while a
// program runs, adds one to the counter of the block the engine is in and of the cell it is walking. A frame's start
// asks for a snapshot, taken at the next sample (that sample still counts toward the frame that ends):
// - the block, cell, frame depth, heap frontier and run-cycle count are latched — the running program's, or the
//   last one's once it ends, and zeros until a program has run since the engine was last reset (`engine_held` is
//   high for the reset's whole hold), since the engine's reset leaves its walk registers as they were;
// - the counters become heat levels — the bit length of the count, so a level is a power of two of engine cycles —
//   and clear, one per board clock, done before the sample after;
// - the cycle count turns into ten decimal digits (sequential double-dabble).
// So every engine clock a program runs is counted once, in one frame, and a counter holds a frame's worth of
// samples without saturating. The frame's exact counts stay readable until the next frame's (`act_idx` →
// `act_count` two clocks later: blocks first, then cells), beside the snapshot (`act_*`); `act_frame` steps once a
// frame's counts are all in, so a reader that sees it unchanged across a read read one frame.
// Each sample while a program runs also adds one to the run total of its block and of its cell (`act_idx` →
// `act_total`, two clocks later): read, then written back one more, a clock apart, each total holding at its
// largest rather than wrapping; the totals clear while the engine is held in reset, so once a run ends they are its
// engine clocks by block and cell, every one. `act_idx` is registered where it arrives, so the reader's index logic
// and the way across the chip to these counters are a path of their own. Drawing:
//   row 26, from column 62: the cycle count, leading zeros dark;
//   row 28, from column 8: 8 block tiles, 6 columns each, heat-coloured, the current block outlined;
//            from column 58: the frame stack, one segment per frame (8); from column 69: the heap, a pixel a word (64);
//   row 29, from column 8: 32 cell squares two columns apart, heat-coloured, the current cell outlined.
// The pixel is resolved over the four board clocks a pixel lasts (`phase` 0..2), so `ov_on`/`ov_rgb` are
// registered and steady when the screen takes them at the pixel's last clock. No divide, multiply or shift sits on
// the pixel path: the tile under the beam and the pixel within it are counters that step with the beam, and which
// digits are leading zeros is worked out once per frame.
module term_activity (
  input             clk,
  input             rst,
  input             sample,
  input             engine_held,
  input             running,
  input      [15:0] pc,
  input      [15:0] cell_ix,
  input      [15:0] sp,
  input      [15:0] hp,
  input      [31:0] cycles,
  input      [9:0]  px,
  input      [9:0]  py,
  input      [1:0]  phase,
  input             frame_start,
  output reg        ov_on = 1'b0,
  output reg [11:0] ov_rgb = 12'h000,
  output reg [15:0] act_frame = 16'd0,
  output            act_running,
  output     [15:0] act_pc,
  output     [15:0] act_cell,
  output     [15:0] act_sp,
  output     [15:0] act_hp,
  output     [31:0] act_cycles,
  input      [5:0]  act_idx,
  output reg [13:0] act_count = 14'd0,
  output reg [31:0] act_total = 32'd0
);
  localparam NB = 8, NC = 32, DEPTH = 8, HEAP = 64;
  localparam BW = $clog2(NB), CW = $clog2(NC);
  localparam DIV = 128;
  localparam TILE = 6 * 8;
  localparam FRAME_CLOCKS = (640 + 16 + 96 + 48) *
                            (480 + 10 + 2 + 33) * 4;

  generate if (NB + NC + 3 >= DIV) begin : sweep_must_end_before_the_next_sample
    term_activity_sweep_outlasts_an_engine_clock refuse ();
  end endgenerate
  generate if (FRAME_CLOCKS / DIV + 1 > 14'h3FFF) begin : a_frame_of_samples_must_fit_a_counter
    term_activity_counter_narrower_than_a_frame refuse ();
  end endgenerate

  // ── counting, and the frame's snapshot ───────────────────────────────────────────────────────────────────
  // Stage 0, on `sample`: the probes into board-clock registers, so nothing after it reads the engine's domain.
  // Stage 1: every counter has its own incrementer, enabled when the registered block or cell is its own — no
  // shared multiplexer or adder on the count. The sweep selects one counter a clock and clears it, and the clock
  // after turns the selected count into its level.
  (* ram_style = "registers" *) reg [13:0] bhits [0:NB-1];
  (* ram_style = "registers" *) reg [13:0] chits [0:NC-1];
  (* ram_style = "registers" *) reg [3:0]  blevel [0:NB-1];
  (* ram_style = "registers" *) reg [3:0]  clevel [0:NC-1];
  (* ram_style = "registers" *) reg [13:0] bcount [0:NB-1];
  (* ram_style = "registers" *) reg [13:0] ccount [0:NC-1];
  (* ram_style = "registers" *) reg [31:0] btotal [0:NB-1];
  (* ram_style = "registers" *) reg [31:0] ctotal [0:NC-1];
  reg [31:0] cur_cycles = 32'd0;
  reg        s_valid = 1'b0, s_run = 1'b0;
  reg [15:0] s_pc = 16'd0, s_cell = 16'd0, s_sp = 16'd0, s_hp = 16'd0;
  reg [31:0] s_cycles = 32'd0;
  reg [15:0] cur_pc = 16'd0, cur_cell = 16'd0, cur_sp = 16'd0, cur_hp = 16'd0;
  reg        cur_running = 1'b0;
  reg        snap = 1'b0;                       // a frame started: snapshot at the next sample
  reg        ran = 1'b0;                        // a program has run since the engine's last reset
  reg [6:0]  sweep = 7'd0;                      // 1..NB+NC: the counter selected this clock
  reg [6:0]  lvl_at = 7'd0;                     // 1..NB+NC: the counter whose count is in `sel`
  reg [13:0] sel = 14'd0;
  reg [5:0]  read_at = 6'd0;                    // `act_idx`, registered
  reg        t_block = 1'b0, t_cell = 1'b0;     // a sample's totals to write back, read the clock before
  reg [15:0] t_pc = 16'd0, t_ci = 16'd0;
  reg [31:0] t_bold = 32'd0, t_cold = 32'd0;
  reg [31:0] dd_bin = 32'd0;
  reg [39:0] dd_bcd = 40'd0, digits = 40'd0;
  reg [5:0]  dd_n = 6'd0;
  reg [9:0]  dark = 10'd0;                      // digit i (0 the units) is a leading zero

  function [3:0] level;                         // the bit length of a count
    input [13:0] n;
    integer i;
    begin
      level = 4'd0;
      for (i = 0; i < 14; i = i + 1) if (n[i]) level = i + 1;
    end
  endfunction

  function [39:0] dd_step;                      // one double-dabble step: add 3 to every digit of 5 or more
    input [39:0] b;
    integer i;
    begin
      dd_step = b;
      for (i = 0; i < 10; i = i + 1)
        if (dd_step[i*4 +: 4] >= 4'd5) dd_step[i*4 +: 4] = dd_step[i*4 +: 4] + 4'd3;
    end
  endfunction

  function [9:0] leading_zeros;                 // digit i is dark when it and every digit above it are 0
    input [39:0] d;
    integer i;
    reg seen;
    begin
      seen = 1'b0;
      for (i = 9; i >= 1; i = i - 1) begin
        seen = seen | (d[i*4 +: 4] != 4'd0);
        leading_zeros[i] = !seen;
      end
      leading_zeros[0] = 1'b0;
    end
  endfunction

  integer k, z;
  initial begin
    for (z = 0; z < NB; z = z + 1) begin bhits[z] = 14'd0; blevel[z] = 4'd0; bcount[z] = 14'd0; end
    for (z = 0; z < NC; z = z + 1) begin chits[z] = 14'd0; clevel[z] = 4'd0; ccount[z] = 14'd0; end
    for (z = 0; z < NB; z = z + 1) btotal[z] = 32'd0;
    for (z = 0; z < NC; z = z + 1) ctotal[z] = 32'd0;
  end

  always @(posedge clk) begin
    s_valid <= sample && !rst;
    if (sample) begin
      s_run <= running; s_pc <= pc; s_cell <= cell_ix; s_sp <= sp; s_hp <= hp; s_cycles <= cycles;
    end
    for (k = 0; k < NB; k = k + 1)
      if (sweep == k + 1) bhits[k] <= 14'd0;
      else if (s_valid && s_run && s_pc == k) bhits[k] <= bhits[k] + 1'b1;
    for (k = 0; k < NC; k = k + 1)
      if (sweep == NB + k + 1) chits[k] <= 14'd0;
      else if (s_valid && s_run && s_cell == k) chits[k] <= chits[k] + 1'b1;
    if (lvl_at != 7'd0) begin
      if (lvl_at <= NB) begin blevel[lvl_at - 1] <= level(sel); bcount[lvl_at - 1] <= sel; end
      else begin clevel[lvl_at - 1 - NB] <= level(sel); ccount[lvl_at - 1 - NB] <= sel; end
    end
    t_block <= s_valid && s_run && s_pc < NB;
    t_cell <= s_valid && s_run && s_cell < NC;
    t_pc <= s_pc; t_ci <= s_cell;
    t_bold <= btotal[s_pc[BW-1:0]];
    t_cold <= ctotal[s_cell[CW-1:0]];
    if (engine_held) begin
      for (k = 0; k < NB; k = k + 1) btotal[k] <= 32'd0;
      for (k = 0; k < NC; k = k + 1) ctotal[k] <= 32'd0;
    end else begin
      if (t_block && t_bold != 32'hFFFF_FFFF) btotal[t_pc] <= t_bold + 1'b1;
      if (t_cell && t_cold != 32'hFFFF_FFFF) ctotal[t_ci] <= t_cold + 1'b1;
    end
    read_at <= act_idx;
    act_count <= (read_at < NB) ? bcount[read_at] : ccount[read_at - NB];
    act_total <= (read_at < NB) ? btotal[read_at] : ctotal[read_at - NB];
  end

  assign act_running = cur_running;
  assign act_pc = cur_pc;
  assign act_cell = cur_cell;
  assign act_sp = cur_sp;
  assign act_hp = cur_hp;
  assign act_cycles = cur_cycles;

  wire [39:0] dd_adj = dd_step(dd_bcd);
  always @(posedge clk) begin
    if (rst) begin
      snap <= 1'b0;
      ran <= 1'b0;
      sweep <= 7'd0;
      lvl_at <= 7'd0;
      dd_n <= 6'd0;
    end else begin
      if (frame_start) snap <= 1'b1;
      if (engine_held) ran <= 1'b0;
      else if (s_valid && s_run) ran <= 1'b1;
      if (s_valid && snap) begin
        snap <= 1'b0;
        sweep <= 7'd1;
        if ((ran || s_run) && !engine_held) begin
          cur_pc <= s_pc; cur_cell <= s_cell; cur_sp <= s_sp; cur_hp <= s_hp; cur_running <= s_run; dd_bin <= s_cycles;
          cur_cycles <= s_cycles;
        end else begin
          cur_pc <= 16'd0; cur_cell <= 16'd0; cur_sp <= 16'd0; cur_hp <= 16'd0; cur_running <= 1'b0; dd_bin <= 32'd0;
          cur_cycles <= 32'd0;
        end
        dd_bcd <= 40'd0; dd_n <= 6'd32;
      end
      if (sweep != 7'd0) begin
        sel <= (sweep <= NB) ? bhits[sweep - 1] : chits[sweep - 1 - NB];
        sweep <= (sweep == NB + NC) ? 7'd0 : sweep + 1'b1;
      end
      lvl_at <= sweep;
      if (lvl_at == NB + NC) act_frame <= act_frame + 1'b1;
      if (dd_n != 6'd0) begin
        dd_bcd <= {dd_adj[38:0], dd_bin[31]};
        dd_bin <= {dd_bin[30:0], 1'b0};
        dd_n <= dd_n - 1'b1;
        if (dd_n == 6'd1) digits <= {dd_adj[38:0], dd_bin[31]};
      end
      dark <= leading_zeros(digits);
    end
  end

  // ── the digit glyphs, from the font ──────────────────────────────────────────────────────────────────────
  function [7:0] digit_bits;
    input [7:0] key;                            // {digit, glyph row}
    case (key)
      8'h00: digit_bits = 8'h00;
      8'h01: digit_bits = 8'h00;
      8'h02: digit_bits = 8'h38;
      8'h03: digit_bits = 8'h6C;
      8'h04: digit_bits = 8'hC6;
      8'h05: digit_bits = 8'hC6;
      8'h06: digit_bits = 8'hD6;
      8'h07: digit_bits = 8'hD6;
      8'h08: digit_bits = 8'hC6;
      8'h09: digit_bits = 8'hC6;
      8'h0A: digit_bits = 8'h6C;
      8'h0B: digit_bits = 8'h38;
      8'h0C: digit_bits = 8'h00;
      8'h0D: digit_bits = 8'h00;
      8'h0E: digit_bits = 8'h00;
      8'h0F: digit_bits = 8'h00;
      8'h10: digit_bits = 8'h00;
      8'h11: digit_bits = 8'h00;
      8'h12: digit_bits = 8'h18;
      8'h13: digit_bits = 8'h38;
      8'h14: digit_bits = 8'h78;
      8'h15: digit_bits = 8'h18;
      8'h16: digit_bits = 8'h18;
      8'h17: digit_bits = 8'h18;
      8'h18: digit_bits = 8'h18;
      8'h19: digit_bits = 8'h18;
      8'h1A: digit_bits = 8'h18;
      8'h1B: digit_bits = 8'h7E;
      8'h1C: digit_bits = 8'h00;
      8'h1D: digit_bits = 8'h00;
      8'h1E: digit_bits = 8'h00;
      8'h1F: digit_bits = 8'h00;
      8'h20: digit_bits = 8'h00;
      8'h21: digit_bits = 8'h00;
      8'h22: digit_bits = 8'h7C;
      8'h23: digit_bits = 8'hC6;
      8'h24: digit_bits = 8'h06;
      8'h25: digit_bits = 8'h0C;
      8'h26: digit_bits = 8'h18;
      8'h27: digit_bits = 8'h30;
      8'h28: digit_bits = 8'h60;
      8'h29: digit_bits = 8'hC0;
      8'h2A: digit_bits = 8'hC6;
      8'h2B: digit_bits = 8'hFE;
      8'h2C: digit_bits = 8'h00;
      8'h2D: digit_bits = 8'h00;
      8'h2E: digit_bits = 8'h00;
      8'h2F: digit_bits = 8'h00;
      8'h30: digit_bits = 8'h00;
      8'h31: digit_bits = 8'h00;
      8'h32: digit_bits = 8'h7C;
      8'h33: digit_bits = 8'hC6;
      8'h34: digit_bits = 8'h06;
      8'h35: digit_bits = 8'h06;
      8'h36: digit_bits = 8'h3C;
      8'h37: digit_bits = 8'h06;
      8'h38: digit_bits = 8'h06;
      8'h39: digit_bits = 8'h06;
      8'h3A: digit_bits = 8'hC6;
      8'h3B: digit_bits = 8'h7C;
      8'h3C: digit_bits = 8'h00;
      8'h3D: digit_bits = 8'h00;
      8'h3E: digit_bits = 8'h00;
      8'h3F: digit_bits = 8'h00;
      8'h40: digit_bits = 8'h00;
      8'h41: digit_bits = 8'h00;
      8'h42: digit_bits = 8'h0C;
      8'h43: digit_bits = 8'h1C;
      8'h44: digit_bits = 8'h3C;
      8'h45: digit_bits = 8'h6C;
      8'h46: digit_bits = 8'hCC;
      8'h47: digit_bits = 8'hFE;
      8'h48: digit_bits = 8'h0C;
      8'h49: digit_bits = 8'h0C;
      8'h4A: digit_bits = 8'h0C;
      8'h4B: digit_bits = 8'h1E;
      8'h4C: digit_bits = 8'h00;
      8'h4D: digit_bits = 8'h00;
      8'h4E: digit_bits = 8'h00;
      8'h4F: digit_bits = 8'h00;
      8'h50: digit_bits = 8'h00;
      8'h51: digit_bits = 8'h00;
      8'h52: digit_bits = 8'hFE;
      8'h53: digit_bits = 8'hC0;
      8'h54: digit_bits = 8'hC0;
      8'h55: digit_bits = 8'hC0;
      8'h56: digit_bits = 8'hFC;
      8'h57: digit_bits = 8'h06;
      8'h58: digit_bits = 8'h06;
      8'h59: digit_bits = 8'h06;
      8'h5A: digit_bits = 8'hC6;
      8'h5B: digit_bits = 8'h7C;
      8'h5C: digit_bits = 8'h00;
      8'h5D: digit_bits = 8'h00;
      8'h5E: digit_bits = 8'h00;
      8'h5F: digit_bits = 8'h00;
      8'h60: digit_bits = 8'h00;
      8'h61: digit_bits = 8'h00;
      8'h62: digit_bits = 8'h38;
      8'h63: digit_bits = 8'h60;
      8'h64: digit_bits = 8'hC0;
      8'h65: digit_bits = 8'hC0;
      8'h66: digit_bits = 8'hFC;
      8'h67: digit_bits = 8'hC6;
      8'h68: digit_bits = 8'hC6;
      8'h69: digit_bits = 8'hC6;
      8'h6A: digit_bits = 8'hC6;
      8'h6B: digit_bits = 8'h7C;
      8'h6C: digit_bits = 8'h00;
      8'h6D: digit_bits = 8'h00;
      8'h6E: digit_bits = 8'h00;
      8'h6F: digit_bits = 8'h00;
      8'h70: digit_bits = 8'h00;
      8'h71: digit_bits = 8'h00;
      8'h72: digit_bits = 8'hFE;
      8'h73: digit_bits = 8'hC6;
      8'h74: digit_bits = 8'h06;
      8'h75: digit_bits = 8'h06;
      8'h76: digit_bits = 8'h0C;
      8'h77: digit_bits = 8'h18;
      8'h78: digit_bits = 8'h30;
      8'h79: digit_bits = 8'h30;
      8'h7A: digit_bits = 8'h30;
      8'h7B: digit_bits = 8'h30;
      8'h7C: digit_bits = 8'h00;
      8'h7D: digit_bits = 8'h00;
      8'h7E: digit_bits = 8'h00;
      8'h7F: digit_bits = 8'h00;
      8'h80: digit_bits = 8'h00;
      8'h81: digit_bits = 8'h00;
      8'h82: digit_bits = 8'h7C;
      8'h83: digit_bits = 8'hC6;
      8'h84: digit_bits = 8'hC6;
      8'h85: digit_bits = 8'hC6;
      8'h86: digit_bits = 8'h7C;
      8'h87: digit_bits = 8'hC6;
      8'h88: digit_bits = 8'hC6;
      8'h89: digit_bits = 8'hC6;
      8'h8A: digit_bits = 8'hC6;
      8'h8B: digit_bits = 8'h7C;
      8'h8C: digit_bits = 8'h00;
      8'h8D: digit_bits = 8'h00;
      8'h8E: digit_bits = 8'h00;
      8'h8F: digit_bits = 8'h00;
      8'h90: digit_bits = 8'h00;
      8'h91: digit_bits = 8'h00;
      8'h92: digit_bits = 8'h7C;
      8'h93: digit_bits = 8'hC6;
      8'h94: digit_bits = 8'hC6;
      8'h95: digit_bits = 8'hC6;
      8'h96: digit_bits = 8'h7E;
      8'h97: digit_bits = 8'h06;
      8'h98: digit_bits = 8'h06;
      8'h99: digit_bits = 8'h06;
      8'h9A: digit_bits = 8'h0C;
      8'h9B: digit_bits = 8'h78;
      8'h9C: digit_bits = 8'h00;
      8'h9D: digit_bits = 8'h00;
      8'h9E: digit_bits = 8'h00;
      8'h9F: digit_bits = 8'h00;
      default: digit_bits = 8'h00;
    endcase
  endfunction

  function [11:0] heat_rgb;
    input [3:0] level_in;
    case (level_in)
      4'd0: heat_rgb = 12'h112;
      4'd1: heat_rgb = 12'h124;
      4'd2: heat_rgb = 12'h136;
      4'd3: heat_rgb = 12'h148;
      4'd4: heat_rgb = 12'h16A;
      4'd5: heat_rgb = 12'h18B;
      4'd6: heat_rgb = 12'h1AB;
      4'd7: heat_rgb = 12'h2C9;
      4'd8: heat_rgb = 12'h4D6;
      4'd9: heat_rgb = 12'h7D4;
      4'd10: heat_rgb = 12'hAD3;
      4'd11: heat_rgb = 12'hCC2;
      4'd12: heat_rgb = 12'hEA1;
      4'd13: heat_rgb = 12'hF81;
      4'd14: heat_rgb = 12'hF51;
      4'd15: heat_rgb = 12'hF22;
      default: heat_rgb = 12'h000;
    endcase
  endfunction

  // ── drawing, resolved over a pixel's clocks ──────────────────────────────────────────────────────────────
  localparam R_NONE = 3'd0, R_DIGIT = 3'd1, R_BLOCK = 3'd2, R_STACK = 3'd3, R_HEAP = 3'd4, R_CELL = 3'd5;
  localparam Y_TITLE = 26 * 16, Y_BLOCK = 28 * 16, Y_CELL = 29 * 16;
  localparam X_DIGIT = 62 * 8, X_BLOCK = 8 * 8, X_STACK = 58 * 8,
             X_HEAP = 69 * 8, X_CELL = 8 * 8;

  reg [2:0]  region = R_NONE;
  reg [9:0]  dx = 10'd0;
  reg [3:0]  gy = 4'd0;
  reg [5:0]  idx = 6'd0;
  reg [5:0]  sub = 6'd0;
  reg [3:0]  lv = 4'd0;
  reg        cur = 1'b0, rim = 1'b0, inner = 1'b0, lit = 1'b0;
  reg [3:0]  dig = 4'd0;
  reg        lead = 1'b0;

  // The block tile under the beam and the pixel within it: zero at the tiles' first pixel, stepped once a pixel
  // (at its last clock, `px` moving to px + 1).
  reg [5:0] btile = 6'd0, bsub = 6'd0;
  always @(posedge clk) if (phase == 2'd3) begin
    if (px + 1'b1 == X_BLOCK) begin btile <= 6'd0; bsub <= 6'd0; end
    else if (bsub == TILE - 1) begin btile <= btile + 1'b1; bsub <= 6'd0; end
    else bsub <= bsub + 1'b1;
  end

  wire [9:0] cdx = px - X_CELL;
  wire [7:0] dbits = digit_bits({dig, gy});

  always @(posedge clk) begin
    case (phase)
      2'd0: begin
        gy <= py[3:0];
        if (py >= Y_TITLE && py < Y_TITLE + 16 && px >= X_DIGIT && px < X_DIGIT + 80) begin
          region <= R_DIGIT; dx <= px - X_DIGIT;
        end else if (py >= Y_BLOCK && py < Y_BLOCK + 16 && px >= X_BLOCK && px < X_BLOCK + NB * TILE) begin
          region <= R_BLOCK; idx <= btile; sub <= bsub;
        end else if (py >= Y_BLOCK && py < Y_BLOCK + 16 && px >= X_STACK && px < X_STACK + DEPTH * 8) begin
          region <= R_STACK; dx <= px - X_STACK;
        end else if (py >= Y_BLOCK && py < Y_BLOCK + 16 && px >= X_HEAP && px < X_HEAP + HEAP) begin
          region <= R_HEAP; dx <= px - X_HEAP;
        end else if (py >= Y_CELL && py < Y_CELL + 16 && px >= X_CELL && px < X_CELL + NC * 16) begin
          region <= R_CELL; idx <= cdx[9:4]; sub <= {2'b00, cdx[3:0]};
        end else region <= R_NONE;
      end
      2'd1: begin
        case (region)
          R_DIGIT: begin
            dig <= digits[(4'd9 - dx[6:3]) * 4 +: 4];
            lead <= dark[4'd9 - dx[6:3]];
          end
          R_BLOCK: begin
            lv <= blevel[idx];
            cur <= cur_running && cur_pc == idx;
            inner <= sub >= 2 && sub < TILE - 2 && gy >= 2 && gy < 14;
            rim <= (sub == 2 || sub == TILE - 3 || gy == 2 || gy == 13);
          end
          R_CELL: begin
            lv <= clevel[idx];
            cur <= cur_running && cur_cell == idx;
            inner <= sub >= 2 && sub < 14 && gy >= 2 && gy < 14;
            rim <= (sub == 2 || sub == 13 || gy == 2 || gy == 13);
          end
          R_STACK: begin
            lit <= dx[9:3] < cur_sp;
            inner <= dx[2:0] >= 1 && dx[2:0] < 7 && gy >= 3 && gy < 13;
          end
          R_HEAP: begin
            lit <= dx < cur_hp;
            inner <= gy >= 4 && gy < 12;
          end
          default: ;
        endcase
      end
      2'd2: begin
        ov_on <= (region != R_NONE);
        case (region)
          R_DIGIT: ov_rgb <= (!lead && dbits[3'd7 - dx[2:0]]) ? 12'hFFF : 12'h000;
          R_BLOCK, R_CELL: ov_rgb <= !inner ? 12'h000 : (cur && rim) ? 12'hFFF : heat_rgb(lv);
          R_STACK: ov_rgb <= !inner ? 12'h000 : lit ? 12'h4D6 : 12'h112;
          R_HEAP:  ov_rgb <= !inner ? 12'h000 : lit ? 12'h1AB : 12'h112;
          default: ov_rgb <= 12'h000;
        endcase
      end
      default: ;
    endcase
  end
endmodule
