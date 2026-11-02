// term_link — the Fabric Terminal's link controller: it executes the host's frames against the screen, the PS/2
// port and the fabric engine, and frames everything the board reports back.
//
// Host frames arrive a byte at a time from `term_slip_rx` into a 16-entry queue, so a long FILL never loses the
// bytes behind it. PUT writes each cell as its second byte arrives; FILL, CURSOR, POINTER, WINDOW, RESULT,
// PS2_SEND, READ, PING and ENGINE_RESET act at their frame's END, so a damaged frame does nothing. PUT, FILL and READ
// address the screen's rows and the small plane's (rows __N_SMALL_FIRST_ROW__ on, columns below __N_SMALL_COLS__);
// a cell anywhere else is not written. ENGINE bytes go into a
// 256-byte queue the engine's loader drains through an internal UART; when the frame's bytes are all inside the
// engine, ENGINE_TAKEN reports their count.
//
// Board frames, one at a time through `term_slip_tx`, most urgent first: engine bytes (up to 16 a frame), PS/2
// bytes, PS2_SENT, ENGINE_TAKEN, PONG, ACTIVITY, TOTALS, then CELLS, read from the text buffer between the host's
// writes.
// PONG carries the PS/2 line monitor's counts (`term_ps2`), taken together as the frame starts so they agree.
// ACTIVITY carries the activity panel's last frame (`term_activity`): its snapshot, taken as the frame starts, then
// each block's and cell's engine clocks read one by one (`act_idx`), then the frame number again, read last.
// TOTALS carries the run's cycles as the snapshot holds them, then the run's totals the same way, each block's then
// each cell's, four bytes each.
module term_link (
  input             clk,
  input             rst,
  input      [7:0]  f_byte,
  input             f_valid,
  input             f_first,
  input             f_end,
  input             f_drop,
  output reg [7:0]  o_byte,
  output reg        o_valid,
  output reg        o_last,
  input             o_taken,
  output            tb_we,
  output     [12:0] tb_addr,
  output     [15:0] tb_wdata,
  input      [15:0] tb_rdata,
  output reg [6:0]  win_row = 7'd0,
  output reg [6:0]  win_col = 7'd0,
  output reg [6:0]  win_rows = 7'd0,
  output reg [6:0]  win_cols = 7'd0,
  output reg [6:0]  cursor_row = 7'd0,
  output reg [6:0]  cursor_col = 7'd0,
  output reg        cursor_on = 1'b0,
  output reg [6:0]  pointer_row = 7'd0,
  output reg [6:0]  pointer_col = 7'd0,
  output reg        pointer_on = 1'b0,
  output reg [31:0] result = 32'd0,
  output reg [7:0]  ps2_tx_byte = 8'd0,
  output reg        ps2_tx_start = 1'b0,
  input             ps2_tx_busy,
  input             ps2_tx_done,
  input             ps2_tx_ack,
  input      [7:0]  ps2_rx_byte,
  input      [7:0]  ps2_rx_status,
  input             ps2_rx_valid,
  output reg        engine_reset = 1'b0,
  output reg [7:0]  eng_tx_byte = 8'd0,
  output reg        eng_tx_start = 1'b0,
  input             eng_tx_ready,
  input      [7:0]  eng_rx_byte,
  input             eng_rx_valid,
  input      [7:0]  features,
  input      [15:0] ps2_line_falls,
  input      [7:0]  ps2_line_min_low,
  input      [15:0] ps2_line_bytes,
  input      [7:0]  ps2_line_errors,
  input      [15:0] act_frame,
  input             act_running,
  input      [15:0] act_pc,
  input      [15:0] act_cell,
  input      [15:0] act_sp,
  input      [15:0] act_hp,
  input      [31:0] act_cycles,
  output     [5:0]  act_idx,
  input      [13:0] act_count,
  input      [31:0] act_total
);
  localparam COLS = __N_COLS__;

  // ── the host-frame queue: {drop, end, first, valid-byte, byte} ──────────────────────────────────────────
  // An END or a drop is an entry of its own, after the frame's last byte.
  (* ram_style = "registers" *) reg [11:0] q [0:15];
  reg [3:0]  q_w = 4'd0, q_r = 4'd0;
  reg [4:0]  q_n = 5'd0;
  wire       q_push = f_valid | f_end | f_drop;
  wire [11:0] q_in = {f_drop, f_end, f_first, f_valid, f_byte};
  reg        q_pop;
  wire [11:0] q_head = q[q_r];
  always @(posedge clk) begin
    if (rst) begin
      q_w <= 4'd0;
      q_r <= 4'd0;
      q_n <= 5'd0;
    end else begin
      if (q_push && q_n != 5'd16) begin q[q_w] <= q_in; q_w <= q_w + 1'b1; end
      if (q_pop) q_r <= q_r + 1'b1;
      q_n <= q_n + (q_push && q_n != 5'd16) - q_pop;
    end
  end

  // ── the engine queue (256 bytes) and its drain into the engine's UART ────────────────────────────────────
  (* ram_style = "block" *) reg [7:0] eq [0:255];
  reg [7:0] eq_w = 8'd0, eq_r = 8'd0;
  reg [8:0] eq_n = 9'd0;
  reg       eq_push = 1'b0;
  reg [7:0] eq_in = 8'd0;
  reg [7:0] eq_q = 8'd0;
  reg [1:0] drain = 2'd0;
  wire      eq_pop = (drain == 2'd0) && eq_n != 9'd0 && eng_tx_ready && !eng_tx_start;
  always @(posedge clk) begin
    if (eq_push) eq[eq_w] <= eq_in;
    eq_q <= eq[eq_r];
  end
  always @(posedge clk) begin
    eng_tx_start <= 1'b0;
    if (rst || engine_reset) begin
      eq_w <= 8'd0;
      eq_r <= 8'd0;
      eq_n <= 9'd0;
      drain <= 2'd0;
    end else begin
      if (eq_push) eq_w <= eq_w + 1'b1;
      case (drain)
        2'd0: if (eq_pop) drain <= 2'd1;                       // eq_r is presented; the read lands next clock
        2'd1: drain <= 2'd2;
        2'd2: begin eng_tx_byte <= eq_q; eng_tx_start <= 1'b1; eq_r <= eq_r + 1'b1; drain <= 2'd3; end
        2'd3: drain <= 2'd0;                                   // the UART drops ready on the next clock
      endcase
      eq_n <= eq_n + eq_push - (drain == 2'd2);
    end
  end

  // ── the board-report sources ──────────────────────────────────────────────────────────────────────────────
  (* ram_style = "registers" *) reg [7:0] er [0:15];    // engine bytes waiting to be framed
  reg [3:0]  er_w = 4'd0, er_r = 4'd0;
  reg [4:0]  er_n = 5'd0;
  reg        er_pop = 1'b0;
  (* ram_style = "registers" *) reg [15:0] pr [0:7];    // PS/2 {status, byte} waiting to be framed
  reg [2:0]  pr_w = 3'd0, pr_r = 3'd0;
  reg [3:0]  pr_n = 4'd0;
  reg        pr_pop = 1'b0;
  always @(posedge clk) begin
    if (rst) begin
      er_w <= 4'd0; er_r <= 4'd0; er_n <= 5'd0;
      pr_w <= 3'd0; pr_r <= 3'd0; pr_n <= 4'd0;
    end else begin
      if (eng_rx_valid && er_n != 5'd16) begin er[er_w] <= eng_rx_byte; er_w <= er_w + 1'b1; end
      if (er_pop) er_r <= er_r + 1'b1;
      er_n <= er_n + (eng_rx_valid && er_n != 5'd16) - er_pop;
      if (ps2_rx_valid && pr_n != 4'd8) begin pr[pr_w] <= {ps2_rx_status, ps2_rx_byte}; pr_w <= pr_w + 1'b1; end
      if (pr_pop) pr_r <= pr_r + 1'b1;
      pr_n <= pr_n + (ps2_rx_valid && pr_n != 4'd8) - pr_pop;
    end
  end

  reg        ev_sent = 1'b0, ev_taken = 1'b0, ev_pong = 1'b0, ev_cells = 1'b0, ev_act = 1'b0, ev_tot = 1'b0;
  reg [7:0]  sent_status = 8'd0, taken_count = 8'd0;
  reg [6:0]  cr_row = 7'd0, cr_col = 7'd0;   // the READ the command machine accepted
  reg [7:0]  cr_count = 8'd0;
  reg [6:0]  rd_row = 7'd0, rd_col = 7'd0;   // the writer's position in it
  reg [7:0]  rd_left = 8'd0;
  reg        set_sent, set_taken, set_pong, set_cells, set_act, set_tot;
  reg        clr_sent, clr_taken, clr_pong, clr_cells, clr_act, clr_tot;

  // ── the command machine ──────────────────────────────────────────────────────────────────────────────────
  localparam C_KIND = 3'd0, C_ARGS = 3'd1, C_CELLS = 3'd2, C_ENGINE = 3'd3, C_SKIP = 3'd4, C_FILL = 3'd5;
  reg [2:0]  cs = C_KIND;
  reg [7:0]  kind = 8'd0;
  (* ram_style = "registers" *) reg [7:0] a [0:5];
  reg [2:0]  na = 3'd0;
  reg [7:0]  hi = 8'd0;
  reg        have_hi = 1'b0;
  reg [6:0]  wr_row = 7'd0, wr_col = 7'd0;
  reg        cmd_we = 1'b0;
  reg [12:0] cmd_addr = 13'd0;
  // A cell the buffer holds: one of the screen's, or one of the small plane's.
  function holds;
    input [6:0] row, col;
    holds = (row < __N_ROWS__ && col < COLS) ||
            (row >= __N_SMALL_FIRST_ROW__ && row < __N_SMALL_FIRST_ROW__ + __N_SMALL_ROWS__ && col < __N_SMALL_COLS__);
  endfunction
  reg [15:0] cmd_wdata = 16'd0;
  reg [7:0]  frame_bytes = 8'd0;
  reg        taken_wait = 1'b0;
  reg [6:0]  fill_r0 = 7'd0, fill_c0 = 7'd0, fill_r = 7'd0, fill_c = 7'd0, fill_h = 7'd0, fill_w = 7'd0;
  reg [15:0] fill_cell = 16'd0;

  wire       h_valid = q_head[8];
  wire       h_first = q_head[9];
  wire       h_end   = q_head[10];
  wire       h_drop  = q_head[11];
  wire [7:0] h_byte  = q_head[7:0];
  wire       busy = (cs == C_FILL);

  function [2:0] args_of;
    input [7:0] k;
    case (k)
      __V_H_PUT__:      args_of = 3'd2;
      __V_H_FILL__:     args_of = 3'd6;
      __V_H_CURSOR__:   args_of = 3'd3;
      __V_H_POINTER__:  args_of = 3'd3;
      __V_H_WINDOW__:   args_of = 3'd4;
      __V_H_RESULT__:   args_of = 3'd4;
      __V_H_PS2_SEND__: args_of = 3'd1;
      __V_H_READ__:     args_of = 3'd3;
      default:          args_of = 3'd0;
    endcase
  endfunction

  always @(posedge clk) begin
    q_pop <= 1'b0;
    cmd_we <= 1'b0;
    eq_push <= 1'b0;
    ps2_tx_start <= 1'b0;
    engine_reset <= 1'b0;
    set_sent <= 1'b0; set_taken <= 1'b0; set_pong <= 1'b0; set_cells <= 1'b0; set_act <= 1'b0; set_tot <= 1'b0;
    if (rst) begin
      cs <= C_KIND;
      taken_wait <= 1'b0;
      cursor_on <= 1'b0;
      pointer_on <= 1'b0;
    end else if (busy) begin
      // FILL: one cell per clock, row by row; cells off the screen are not written.
      cmd_we <= holds(fill_r, fill_c);
      cmd_addr <= {fill_r[5:0], fill_c};
      cmd_wdata <= fill_cell;
      if (fill_c == fill_c0 + fill_w - 1'b1) begin
        fill_c <= fill_c0;
        if (fill_r == fill_r0 + fill_h - 1'b1) cs <= C_KIND;
        else fill_r <= fill_r + 1'b1;
      end else fill_c <= fill_c + 1'b1;
    end else if (q_n != 5'd0 && !q_pop) begin
      q_pop <= 1'b1;
      if (h_drop) begin
        cs <= C_KIND;
      end else if (h_end) begin
        if (cs == C_ARGS && na == args_of(kind)) begin
          case (kind)
            __V_H_FILL__:
              if (a[2] != 8'd0 && a[3] != 8'd0) begin
                fill_r0 <= a[0][6:0]; fill_r <= a[0][6:0];
                fill_c0 <= a[1][6:0]; fill_c <= a[1][6:0];
                fill_h <= a[2][6:0];  fill_w <= a[3][6:0];
                fill_cell <= {a[4], a[5]};
                cs <= C_FILL;
              end
            __V_H_CURSOR__:   begin cursor_row <= a[0][6:0]; cursor_col <= a[1][6:0]; cursor_on <= a[2] != 8'd0; end
            __V_H_POINTER__:  begin pointer_row <= a[0][6:0]; pointer_col <= a[1][6:0]; pointer_on <= a[2] != 8'd0; end
            __V_H_WINDOW__:   begin win_row <= a[0][6:0]; win_col <= a[1][6:0]; win_rows <= a[2][6:0]; win_cols <= a[3][6:0]; end
            __V_H_RESULT__:   result <= {a[0], a[1], a[2], a[3]};
            __V_H_PS2_SEND__: if (!ps2_tx_busy) begin ps2_tx_byte <= a[0]; ps2_tx_start <= 1'b1; end
            __V_H_READ__:     begin cr_row <= a[0][6:0]; cr_col <= a[1][6:0]; cr_count <= a[2]; set_cells <= a[2] != 8'd0; end
            __V_H_PING__:     set_pong <= 1'b1;
            __V_H_ACTIVITY__: set_act <= 1'b1;
            __V_H_TOTALS__:   set_tot <= 1'b1;
            __V_H_ENGINE_RESET__: begin engine_reset <= 1'b1; taken_wait <= 1'b0; end
            default: ;
          endcase
        end else if (cs == C_ENGINE) begin
          taken_count <= frame_bytes;
          taken_wait <= 1'b1;
        end
        if (!(cs == C_ARGS && na == args_of(kind) && kind == __V_H_FILL__ && a[2] != 8'd0 && a[3] != 8'd0))
          cs <= C_KIND;
      end else if (h_valid) begin
        if (h_first) begin
          kind <= h_byte;
          na <= 3'd0;
          have_hi <= 1'b0;
          frame_bytes <= 8'd0;
          cs <= (h_byte == __V_H_ENGINE__) ? C_ENGINE : C_ARGS;
        end else case (cs)
          C_ARGS:
            if (na != args_of(kind)) begin
              a[na] <= h_byte;
              na <= na + 1'b1;
              if (kind == __V_H_PUT__ && na == 3'd1) begin
                wr_row <= a[0][6:0];
                wr_col <= h_byte[6:0];
                cs <= C_CELLS;
              end
            end else cs <= C_SKIP;
          C_CELLS:
            if (!have_hi) begin
              hi <= h_byte;
              have_hi <= 1'b1;
            end else begin
              have_hi <= 1'b0;
              cmd_we <= holds(wr_row, wr_col);
              cmd_addr <= {wr_row[5:0], wr_col};
              cmd_wdata <= {hi, h_byte};
              if (wr_col == COLS - 1) begin wr_col <= 7'd0; wr_row <= wr_row + 1'b1; end
              else wr_col <= wr_col + 1'b1;
            end
          C_ENGINE:
            if (eq_n + eq_push != 9'd256) begin
              eq_in <= h_byte;
              eq_push <= 1'b1;
              frame_bytes <= frame_bytes + 1'b1;
            end
          default: ;
        endcase
      end
    end
    if (taken_wait && eq_n == 9'd0 && drain == 2'd0 && eng_tx_ready && !eq_push && !eng_tx_start) begin
      taken_wait <= 1'b0;
      set_taken <= 1'b1;
    end
    if (ps2_tx_done) begin
      sent_status <= ps2_tx_ack ? 8'd0 : __V_PS2_NO_ACK__;
      set_sent <= 1'b1;
    end
  end

  // ── the text buffer's link port: the command machine writes, the CELLS report reads between writes ───────
  reg  [12:0] rd_addr = 13'd0;
  reg         rd_req = 1'b0;
  wire        rd_grant = rd_req && !cmd_we;
  reg         rd_got = 1'b0;
  assign tb_we    = cmd_we;
  assign tb_addr  = cmd_we ? cmd_addr : rd_addr;
  assign tb_wdata = cmd_wdata;

  // ── the report writer ─────────────────────────────────────────────────────────────────────────────────────
  localparam W_IDLE = 4'd0, W_KIND = 4'd1, W_ENG = 4'd2, W_PS2B = 4'd3, W_PS2S = 4'd4, W_ONE = 4'd5,
             W_PONG = 4'd6, W_CROW = 4'd7, W_CCOL = 4'd8, W_CREQ = 4'd9, W_CWAIT = 4'd10, W_CHI = 4'd11,
             W_CLO = 4'd12, W_ACT = 4'd13, W_TOT = 4'd14;
  localparam ACT_COUNTS = __N_MAXBLK__ + __N_MAXCELL__;         // one count each, two bytes
  localparam ACT_HEAD = 15;                                      // frame, running, block, cell, depth, heap, cycles
  localparam ACT_LAST = ACT_HEAD + 2 * ACT_COUNTS + 1;          // the frame number again, its two bytes
  localparam TOT_HEAD = 4;                                       // the run's cycles
  localparam TOT_LAST = TOT_HEAD + 4 * ACT_COUNTS - 1;           // one total each, four bytes
  reg [3:0]  ws = W_IDLE, wnext = W_IDLE;
  reg [4:0]  w_left = 5'd0;
  reg [15:0] w_ps2 = 16'd0;
  reg [7:0]  w_one = 8'd0;
  reg [3:0]  w_pong = 4'd0;
  reg [47:0] w_line = 48'd0;
  reg [15:0] w_cell = 16'd0;
  reg [7:0]  w_ab = 8'd0;                                        // the ACTIVITY or TOTALS byte to load next
  reg        w_tot = 1'b0;                                       // TOTALS: set as its kind byte goes out
  reg [119:0] w_head = 120'd0;
  wire [7:0] w_ak = w_tot ? ((w_ab - TOT_HEAD) >> 2) : ((w_ab - ACT_HEAD) >> 1);
  assign act_idx = w_ak[5:0];

  always @(posedge clk) begin
    er_pop <= 1'b0;
    pr_pop <= 1'b0;
    clr_sent <= 1'b0; clr_taken <= 1'b0; clr_pong <= 1'b0; clr_cells <= 1'b0; clr_act <= 1'b0; clr_tot <= 1'b0;
    rd_got <= rd_grant;
    if (rst) begin
      ws <= W_IDLE;
      o_valid <= 1'b0;
      rd_req <= 1'b0;
    end else if (o_valid) begin
      if (o_taken) begin
        o_valid <= 1'b0;
        ws <= wnext;
      end
    end else begin
      case (ws)
        W_IDLE:
          if (er_n != 5'd0) begin
            o_byte <= __V_B_ENGINE__; o_last <= 1'b0; o_valid <= 1'b1;
            w_left <= er_n; wnext <= W_ENG;
          end else if (pr_n != 4'd0) begin
            o_byte <= __V_B_PS2__; o_last <= 1'b0; o_valid <= 1'b1;
            w_ps2 <= pr[pr_r]; pr_pop <= 1'b1; wnext <= W_PS2B;
          end else if (ev_sent) begin
            o_byte <= __V_B_PS2_SENT__; o_last <= 1'b0; o_valid <= 1'b1;
            w_one <= sent_status; clr_sent <= 1'b1; wnext <= W_ONE;
          end else if (ev_taken) begin
            o_byte <= __V_B_ENGINE_TAKEN__; o_last <= 1'b0; o_valid <= 1'b1;
            w_one <= taken_count; clr_taken <= 1'b1; wnext <= W_ONE;
          end else if (ev_pong) begin
            o_byte <= __V_B_PONG__; o_last <= 1'b0; o_valid <= 1'b1;
            w_pong <= 4'd0; clr_pong <= 1'b1; wnext <= W_PONG;
            w_line <= {ps2_line_falls, ps2_line_min_low, ps2_line_bytes, ps2_line_errors};
          end else if (ev_act) begin
            o_byte <= __V_B_ACTIVITY__; o_last <= 1'b0; o_valid <= 1'b1;
            w_ab <= 8'd0; w_tot <= 1'b0; clr_act <= 1'b1; wnext <= W_ACT;
            w_head <= {act_frame, 7'd0, act_running, act_pc, act_cell, act_sp, act_hp, act_cycles};
          end else if (ev_tot) begin
            o_byte <= __V_B_TOTALS__; o_last <= 1'b0; o_valid <= 1'b1;
            w_ab <= 8'd0; w_tot <= 1'b1; clr_tot <= 1'b1; wnext <= W_TOT;
            w_head <= {88'd0, act_cycles};
          end else if (ev_cells) begin
            o_byte <= __V_B_CELLS__; o_last <= 1'b0; o_valid <= 1'b1;
            rd_row <= cr_row; rd_col <= cr_col; rd_left <= cr_count;
            clr_cells <= 1'b1; wnext <= W_CROW;
          end
        W_ENG: begin
          o_byte <= er[er_r]; er_pop <= 1'b1;
          o_last <= (w_left == 5'd1); o_valid <= 1'b1;
          w_left <= w_left - 1'b1;
          wnext <= (w_left == 5'd1) ? W_IDLE : W_ENG;
        end
        W_PS2B: begin o_byte <= w_ps2[7:0]; o_last <= 1'b0; o_valid <= 1'b1; wnext <= W_PS2S; end
        W_PS2S: begin o_byte <= w_ps2[15:8]; o_last <= 1'b1; o_valid <= 1'b1; wnext <= W_IDLE; end
        W_ONE:  begin o_byte <= w_one; o_last <= 1'b1; o_valid <= 1'b1; wnext <= W_IDLE; end
        W_PONG: begin
          case (w_pong)
            4'd0:  o_byte <= __V_VERSION__;
            4'd1:  o_byte <= __N_COLS__;
            4'd2:  o_byte <= __N_ROWS__;
            4'd3:  o_byte <= __V_PANEL_ROWS__;
            4'd4:  o_byte <= features;
            4'd5:  o_byte <= w_line[47:40];
            4'd6:  o_byte <= w_line[39:32];
            4'd7:  o_byte <= w_line[31:24];
            4'd8:  o_byte <= w_line[23:16];
            4'd9:  o_byte <= w_line[15:8];
            default: o_byte <= w_line[7:0];
          endcase
          o_last <= (w_pong == 4'd10); o_valid <= 1'b1;
          w_pong <= w_pong + 1'b1;
          wnext <= (w_pong == 4'd10) ? W_IDLE : W_PONG;
        end
        W_ACT: begin
          if (w_ab < ACT_HEAD) o_byte <= w_head[(ACT_HEAD - 1 - w_ab) * 8 +: 8];
          else if (w_ab < ACT_HEAD + 2 * ACT_COUNTS) o_byte <= w_ab[0] ? {2'b00, act_count[13:8]} : act_count[7:0];
          else o_byte <= (w_ab == ACT_LAST) ? act_frame[7:0] : act_frame[15:8];
          o_last <= (w_ab == ACT_LAST); o_valid <= 1'b1;
          w_ab <= w_ab + 1'b1;
          wnext <= (w_ab == ACT_LAST) ? W_IDLE : W_ACT;
        end
        W_TOT: begin
          if (w_ab < TOT_HEAD) o_byte <= w_head[(TOT_HEAD - 1 - w_ab) * 8 +: 8];
          else case (w_ab[1:0])
            2'd0: o_byte <= act_total[31:24];
            2'd1: o_byte <= act_total[23:16];
            2'd2: o_byte <= act_total[15:8];
            default: o_byte <= act_total[7:0];
          endcase
          o_last <= (w_ab == TOT_LAST); o_valid <= 1'b1;
          w_ab <= w_ab + 1'b1;
          wnext <= (w_ab == TOT_LAST) ? W_IDLE : W_TOT;
        end
        W_CROW: begin o_byte <= {1'b0, rd_row}; o_last <= 1'b0; o_valid <= 1'b1; wnext <= W_CCOL; end
        W_CCOL: begin o_byte <= {1'b0, rd_col}; o_last <= 1'b0; o_valid <= 1'b1; wnext <= W_CREQ; end
        W_CREQ: begin rd_addr <= {rd_row[5:0], rd_col}; rd_req <= 1'b1; ws <= W_CWAIT; end
        W_CWAIT:
          if (rd_got) begin
            rd_req <= 1'b0;
            w_cell <= tb_rdata;
            ws <= W_CHI;
          end
        W_CHI: begin o_byte <= w_cell[15:8]; o_last <= 1'b0; o_valid <= 1'b1; wnext <= W_CLO; end
        W_CLO: begin
          o_byte <= w_cell[7:0]; o_last <= (rd_left == 8'd1); o_valid <= 1'b1;
          rd_left <= rd_left - 1'b1;
          if (rd_col == COLS - 1) begin rd_col <= 7'd0; rd_row <= rd_row + 1'b1; end
          else rd_col <= rd_col + 1'b1;
          wnext <= (rd_left == 8'd1) ? W_IDLE : W_CREQ;
        end
        default: ws <= W_IDLE;
      endcase
    end
    if (rd_grant) rd_req <= 1'b0;
  end

  // ── pending-report flags: set by the command side, cleared by the writer ─────────────────────────────────
  always @(posedge clk) begin
    if (rst) begin
      ev_sent <= 1'b0; ev_taken <= 1'b0; ev_pong <= 1'b0; ev_cells <= 1'b0; ev_act <= 1'b0; ev_tot <= 1'b0;
    end else begin
      if (set_sent) ev_sent <= 1'b1; else if (clr_sent) ev_sent <= 1'b0;
      if (set_taken) ev_taken <= 1'b1; else if (clr_taken) ev_taken <= 1'b0;
      if (set_pong) ev_pong <= 1'b1; else if (clr_pong) ev_pong <= 1'b0;
      if (set_act) ev_act <= 1'b1; else if (clr_act) ev_act <= 1'b0;
      if (set_tot) ev_tot <= 1'b1; else if (clr_tot) ev_tot <= 1'b0;
      if (set_cells) ev_cells <= 1'b1; else if (clr_cells) ev_cells <= 1'b0;
    end
  end
endmodule
