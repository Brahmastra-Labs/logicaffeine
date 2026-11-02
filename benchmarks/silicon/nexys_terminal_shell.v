// nexys_terminal_shell — the Fabric Terminal on the Nexys A7-100T: a LOGOS editor on the VGA screen, a USB keyboard or mouse
// on the board's USB host port (which the board presents as PS/2), and the fabric engine that runs each program,
// all behind one UART to a host that compiles. The host draws text with PUT / FILL / CURSOR frames and reads the
// keyboard from PS2 reports; it streams a compiled image to the engine in ENGINE frames and gets the engine's
// Show records and result frame back in ENGINE reports. The 7-segment display shows the RESULT the host sets.
//
// Clocks: everything but the engine runs on the 100 MHz board clock, the screen on a 25 MHz pixel enable. The
// engine runs on clk / 128 (its hold-safe clock) on a global buffer, and its loader is fed through an internal UART at
// 8 engine clocks a bit: nothing crosses between the two clock domains but that serial line each way and the engine's
// reset.
module nexys_terminal_shell #(
  // The engine's IEEE-754 unit, as a top-level knob: a build sets it with `chparam` and its recipe records it.
  parameter WITH_FPU = 1
) (
  input        clk,
  input        uart_rx,
  output       uart_tx,
  inout        ps2_clk,
  inout        ps2_data,
  output [3:0] vga_r,
  output [3:0] vga_g,
  output [3:0] vga_b,
  output       vga_hs,
  output       vga_vs,
  output [6:0] seg,
  output       dp,
  output [7:0] an,
  output [15:0] led
);
  reg [4:0] por = 5'd0;
  always @(posedge clk) if (!por[4]) por <= por + 1'b1;
  wire rst = !por[4];

  // ── the link ────────────────────────────────────────────────────────────────────────────────────────────────
  wire [7:0] rx_byte;
  wire       rx_valid, rx_framing;
  term_uart_rx #(.CLKS_PER_BIT(100)) link_rx (
    .clk(clk), .rst(rst), .rx(uart_rx), .data(rx_byte), .valid(rx_valid), .framing_error(rx_framing));
  wire [7:0] f_byte;
  wire       f_valid, f_first, f_end, f_drop;
  term_slip_rx slip_rx (
    .clk(clk), .rst(rst), .in(rx_byte), .in_valid(rx_valid),
    .out(f_byte), .out_valid(f_valid), .first(f_first), .frame_end(f_end), .frame_drop(f_drop));
  wire [7:0] o_byte, tx_byte;
  wire       o_valid, o_last, o_taken, tx_start, tx_ready;
  term_slip_tx slip_tx (
    .clk(clk), .rst(rst), .in(o_byte), .in_valid(o_valid), .in_last(o_last), .in_taken(o_taken),
    .tx_byte(tx_byte), .tx_start(tx_start), .tx_ready(tx_ready));
  term_uart_tx #(.CLKS_PER_BIT(100)) link_tx (
    .clk(clk), .rst(rst), .data(tx_byte), .start(tx_start), .ready(tx_ready), .tx(uart_tx));

  // ── PS/2: open-collector lines through tristate buffers ─────────────────────────────────────────────────────
  wire ps2_clk_low, ps2_data_low, ps2_clk_in, ps2_data_in;
  IOBUF ps2_clk_buf  (.IO(ps2_clk),  .I(1'b0), .T(!ps2_clk_low),  .O(ps2_clk_in));
  IOBUF ps2_data_buf (.IO(ps2_data), .I(1'b0), .T(!ps2_data_low), .O(ps2_data_in));
  wire [7:0] ps2_rx_byte, ps2_rx_status, ps2_tx_byte;
  wire       ps2_rx_valid, ps2_tx_start, ps2_tx_busy, ps2_tx_done, ps2_tx_ack, ps2_line_fell;
  wire [15:0] ps2_line_falls, ps2_line_bytes;
  wire [7:0]  ps2_line_min_low, ps2_line_errors;
  term_ps2 ps2 (
    .clk(clk), .rst(rst), .ps2_clk_in(ps2_clk_in), .ps2_data_in(ps2_data_in),
    .ps2_clk_low(ps2_clk_low), .ps2_data_low(ps2_data_low),
    .rx_byte(ps2_rx_byte), .rx_status(ps2_rx_status), .rx_valid(ps2_rx_valid),
    .tx_byte(ps2_tx_byte), .tx_start(ps2_tx_start), .tx_busy(ps2_tx_busy), .tx_done(ps2_tx_done), .tx_ack(ps2_tx_ack),
    .line_falls(ps2_line_falls), .line_min_low(ps2_line_min_low), .line_fell(ps2_line_fell),
    .line_bytes(ps2_line_bytes), .line_errors(ps2_line_errors));

  // ── the screen ──────────────────────────────────────────────────────────────────────────────────────────────
  wire        tb_we;
  wire [12:0] tb_addr, vga_addr;
  wire [6:0]  win_row, win_col, win_rows, win_cols;
  wire [15:0] tb_wdata, tb_rdata, vga_data;
  term_textbuf textbuf (
    .clk(clk), .a_we(tb_we), .a_addr(tb_addr), .a_wdata(tb_wdata), .a_rdata(tb_rdata),
    .b_addr(vga_addr), .b_rdata(vga_data));
  wire [6:0]  cursor_row, cursor_col, pointer_row, pointer_col;
  wire        cursor_on, pointer_on;
  wire [9:0]  px, py;
  wire        pix, frame_start;
  wire [1:0]  phase;
  wire        ov_on;
  wire [11:0] ov_rgb;
  term_vga vga (
    .clk(clk), .rst(rst), .tb_addr(vga_addr), .tb_data(vga_data),
    .win_row(win_row), .win_col(win_col), .win_rows(win_rows), .win_cols(win_cols),
    .cursor_row(cursor_row), .cursor_col(cursor_col), .cursor_on(cursor_on),
    .pointer_row(pointer_row), .pointer_col(pointer_col), .pointer_on(pointer_on),
    .ov_on(ov_on), .ov_rgb(ov_rgb), .px(px), .py(py), .pix(pix), .phase(phase), .frame_start(frame_start),
    .r(vga_r), .g(vga_g), .b(vga_b), .hs(vga_hs), .vs(vga_vs));

  // ── the link controller ─────────────────────────────────────────────────────────────────────────────────────
  wire [31:0] result;
  wire [7:0]  features;
  wire        engine_reset, eng_tx_start, eng_tx_ready, eng_rx_valid;
  wire [15:0] act_frame, act_pc, act_cell, act_sp, act_hp;
  wire [31:0] act_cycles;
  wire        act_running;
  wire [5:0]  act_idx;
  wire [13:0] act_count;
  wire [31:0] act_total;
  wire [7:0]  eng_tx_byte, eng_rx_byte;
  term_link link (
    .clk(clk), .rst(rst),
    .f_byte(f_byte), .f_valid(f_valid), .f_first(f_first), .f_end(f_end), .f_drop(f_drop),
    .o_byte(o_byte), .o_valid(o_valid), .o_last(o_last), .o_taken(o_taken),
    .tb_we(tb_we), .tb_addr(tb_addr), .tb_wdata(tb_wdata), .tb_rdata(tb_rdata),
    .win_row(win_row), .win_col(win_col), .win_rows(win_rows), .win_cols(win_cols),
    .cursor_row(cursor_row), .cursor_col(cursor_col), .cursor_on(cursor_on),
    .pointer_row(pointer_row), .pointer_col(pointer_col), .pointer_on(pointer_on),
    .result(result),
    .ps2_tx_byte(ps2_tx_byte), .ps2_tx_start(ps2_tx_start), .ps2_tx_busy(ps2_tx_busy),
    .ps2_tx_done(ps2_tx_done), .ps2_tx_ack(ps2_tx_ack),
    .ps2_rx_byte(ps2_rx_byte), .ps2_rx_status(ps2_rx_status), .ps2_rx_valid(ps2_rx_valid),
    .engine_reset(engine_reset), .eng_tx_byte(eng_tx_byte), .eng_tx_start(eng_tx_start),
    .eng_tx_ready(eng_tx_ready), .eng_rx_byte(eng_rx_byte), .eng_rx_valid(eng_rx_valid), .features(features),
    .ps2_line_falls(ps2_line_falls), .ps2_line_min_low(ps2_line_min_low), .ps2_line_bytes(ps2_line_bytes),
    .ps2_line_errors(ps2_line_errors),
    .act_frame(act_frame), .act_running(act_running), .act_pc(act_pc), .act_cell(act_cell), .act_sp(act_sp),
    .act_hp(act_hp), .act_cycles(act_cycles), .act_idx(act_idx), .act_count(act_count), .act_total(act_total));

  // ── the 7-segment display and the LEDs ──────────────────────────────────────────────────────────────────────
  // LED 15: alive; 14: toggles per host frame; 13: a host frame arrived damaged; 12: a PS/2 byte arrived damaged;
  // 11: a PS/2 send in flight; 10: the PS/2 clock moving (lit 84 ms after each edge the device drives — typing on a
  // keyboard the board's USB bridge serves flickers it); 9: toggles per PS/2 byte read whole; 8: the engine
  // running; 7..0: the engine's block, one-hot (no engine: the last PS/2 byte).
  sevenseg_dec #(.REFRESH_BITS(17)) display (.clk(clk), .rst(rst), .val(result), .seg(seg), .dp(dp), .an(an));
  reg [7:0] last_ps2 = 8'd0;
  reg       ps2_damaged = 1'b0, link_blink = 1'b0, link_damaged = 1'b0, ps2_blink = 1'b0;
  reg [22:0] ps2_moving = 23'd0;
  always @(posedge clk) begin
    if (ps2_rx_valid) begin last_ps2 <= ps2_rx_byte; ps2_damaged <= ps2_rx_status != 8'd0; end
    if (ps2_rx_valid && ps2_rx_status == 8'd0) ps2_blink <= !ps2_blink;
    if (ps2_line_fell) ps2_moving <= {23{1'b1}};
    else if (ps2_moving != 23'd0) ps2_moving <= ps2_moving - 1'b1;
    if (f_end) link_blink <= !link_blink;
    if (f_drop || rx_framing) link_damaged <= 1'b1;
  end
  wire [8:0] eng_led;
  assign led = {!rst, link_blink, link_damaged, ps2_damaged, ps2_tx_busy, ps2_moving != 23'd0, ps2_blink, eng_led};

  // No engine in this image: ENGINE bytes are taken at once and nothing comes back.
  assign features = 8'h02;
  assign eng_tx_ready = 1'b1;
  assign eng_rx_valid = 1'b0;
  assign eng_rx_byte = 8'd0;
  assign ov_on = 1'b0;
  assign ov_rgb = 12'h000;
  assign eng_led = {1'b0, last_ps2};
  assign {act_frame, act_pc, act_cell, act_sp, act_hp, act_cycles, act_running, act_count, act_total} = 0;
endmodule
