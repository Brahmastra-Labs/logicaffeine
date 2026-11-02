// hart_ddr2_loopback_top — a board probe of the DDR2 PHY alone. The DDR2 is held idle (CKE low: its outputs off),
// and the PHY drives DQ with a pattern whose every beat is distinct — beat b of frame k carries c = (4k + b) mod 256
// on lane 0 and 3c mod 256 on lane 1 — and reads its own pads back through the input delays and deserializers.
// Thirty-two frames of what it reads, after eight frames of the pattern settling, go out on the USB UART (115200
// baud) as
//   0xA5, the first frame number captured (2 bytes), then each frame's 64 bits little-endian, then 0x5A,
// once a second (scripts/hw/ddr2-loopback.py reads them). From them: the order the deserializer hands a frame's beats over (FIRST_BEAT_ON_Q4), the frames the
// round trip takes, and whether either lane reads inverted. The input delays are loaded with the switches' low five
// bits, so the probe also shows where along the taps the pads' own eye sits.
module hart_ddr2_loopback_top #(
  parameter CAPTURE_QUARTER = 1,
  parameter FIRST_BEAT_ON_Q4 = 1
) (
  input         clk,
  output        uart_tx,
  output [15:0] led,
  input  [15:0] sw,
  output        ddr2_ck_p,
  output        ddr2_ck_n,
  output        ddr2_cke,
  output        ddr2_cs_n,
  output        ddr2_ras_n,
  output        ddr2_cas_n,
  output        ddr2_we_n,
  output [2:0]  ddr2_ba,
  output [12:0] ddr2_addr,
  output        ddr2_odt,
  output [1:0]  ddr2_dm,
  inout  [15:0] ddr2_dq,
  inout  [1:0]  ddr2_dqs_p,
  inout  [1:0]  ddr2_dqs_n
);
  wire fb, c125_raw, c125q_raw, c62_raw, c200_raw, locked;
  wire c125, c125q, c62, c200;
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(8), .CLKOUT1_PHASE(90.0), .CLKOUT2_DIVIDE(16), .CLKOUT3_DIVIDE(5)
  ) pll (
    .CLKIN1(clk), .CLKFBIN(fb), .CLKFBOUT(fb), .CLKOUT0(c125_raw), .CLKOUT1(c125q_raw), .CLKOUT2(c62_raw),
    .CLKOUT3(c200_raw), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0));
  BUFG b125 (.I(c125_raw), .O(c125));
  BUFG b125q (.I(c125q_raw), .O(c125q));
  BUFG b62 (.I(c62_raw), .O(c62));
  BUFG b200 (.I(c200_raw), .O(c200));

  wire idelay_ready;
  reg [1:0] lock_sync = 2'b00;
  reg [7:0] por = 8'd0;
  always @(posedge c62) begin
    lock_sync <= {lock_sync[0], locked};
    if (!lock_sync[1] || !idelay_ready) por <= 8'd0;
    else if (!por[7]) por <= por + 8'd1;
  end
  wire rst = !por[7];

  // The pattern, and the delay taps from the switches (loaded whenever they change).
  reg  [15:0] k = 16'd0;
  wire [7:0]  c0 = k[5:0] * 8'd4, c1 = c0 + 8'd1, c2 = c0 + 8'd2, c3 = c0 + 8'd3;
  wire [63:0] pattern = {c3 * 8'd3, c3, c2 * 8'd3, c2, c1 * 8'd3, c1, c0 * 8'd3, c0};
  reg  [4:0]  taps = 5'd0;
  reg         dly_ld = 1'b0;
  always @(posedge c62) begin
    k <= k + 16'd1;
    dly_ld <= taps != sw[4:0] || rst;
    taps <= sw[4:0];
  end

  wire [63:0] rd_frame;
  hart_ddr2_phy #(.CAPTURE_QUARTER(CAPTURE_QUARTER), .FIRST_BEAT_ON_Q4(FIRST_BEAT_ON_Q4)) phy (
    .c125(c125), .c125q(c125q), .c62(c62), .c200(c200), .rst(!lock_sync[1]), .idelay_ready(idelay_ready),
    .cke(1'b0), .cs_n(1'b1), .ras_n(1'b1), .cas_n(1'b1), .we_n(1'b1), .ba(3'd0), .a(13'd0),
    .dq_oe(!rst), .dq_out(pattern), .dqs_oe(1'b0), .dqs_toggle(1'b0), .rd_frame(rd_frame),
    .dly_ld(dly_ld), .dly_tap({taps, taps}),
    .ddr2_ck_p(ddr2_ck_p), .ddr2_ck_n(ddr2_ck_n), .ddr2_cke(ddr2_cke), .ddr2_cs_n(ddr2_cs_n),
    .ddr2_ras_n(ddr2_ras_n), .ddr2_cas_n(ddr2_cas_n), .ddr2_we_n(ddr2_we_n), .ddr2_ba(ddr2_ba),
    .ddr2_addr(ddr2_addr), .ddr2_odt(ddr2_odt), .ddr2_dm(ddr2_dm), .ddr2_dq(ddr2_dq), .ddr2_dqs_p(ddr2_dqs_p),
    .ddr2_dqs_n(ddr2_dqs_n));

  // Capture thirty-two frames once a second, then send them.
  (* ram_style = "registers" *) reg [63:0] frames [0:31];
  reg [15:0] first;
  reg [5:0]  n = 6'd0;
  reg        capturing = 1'b0, sending = 1'b0;
  reg [25:0] second = 26'd0;
  reg [8:0]  pos = 9'd0;            // the byte being sent: 0 header, 1-2 first, 3..258 frames, 259 trailer
  wire       tx_ready;
  reg        tx_valid = 1'b0;
  reg [7:0]  tx_byte;
  wire [63:0] f = frames[(pos - 9'd3) >> 3];
  always @(posedge c62) begin
    tx_valid <= 1'b0;
    if (rst) begin
      second <= 26'd0;
      capturing <= 1'b0;
      sending <= 1'b0;
    end else begin
      second <= (second == 26'd62_499_999) ? 26'd0 : second + 26'd1;
      if (second == 26'd8 && !sending) begin
        capturing <= 1'b1;
        n <= 6'd0;
        first <= k;
      end
      if (capturing) begin
        frames[n[4:0]] <= rd_frame;
        n <= n + 6'd1;
        if (n == 6'd31) begin
          capturing <= 1'b0;
          sending <= 1'b1;
          pos <= 9'd0;
        end
      end
      if (sending && tx_ready && !tx_valid) begin
        tx_valid <= 1'b1;
        tx_byte <= (pos == 9'd0) ? 8'hA5 : (pos == 9'd1) ? first[7:0] : (pos == 9'd2) ? first[15:8]
                 : (pos == 9'd259) ? 8'h5A : f >> {(pos - 9'd3) & 9'd7, 3'b000};
        pos <= pos + 9'd1;
        if (pos == 9'd259) sending <= 1'b0;
      end
    end
  end
  hart_uart_tx #(.CLKS_PER_BIT(542)) log (
    .clk(c62), .rst(rst), .byte_in(tx_byte), .valid(tx_valid), .ready(tx_ready), .tx(uart_tx));

  assign led = {locked, idelay_ready, sending, capturing, 7'd0, taps};
endmodule
