// term_ps2 — a PS/2 port, both directions. The Nexys A7's USB host bridge presents a USB keyboard or mouse as a
// PS/2 device on two open-collector lines; this module reads its bytes and sends it bytes. Both lines are driven
// only low (`*_low` asks the top's tristate buffers to pull a line down) and read back through synchronizers.
//
// Device to host: 11 bits per byte — start (0), eight data bits LSB first, odd parity, stop (1) — the device
// drives the clock (PS/2 allows 10-16.7 kHz; the Nexys A7's bridge clocks a keyboard at 20-30 kHz) and the data is
// read at each falling clock edge. The clock is debounced by a window of 8 samples one microsecond apart, so a
// glitch shorter than 8 us is not an edge and any clock whose low phase lasts 8 us or more is read (60 kHz at an
// even duty). A frame that stalls mid-byte for more than RX_TIMEOUT_US is abandoned, and the next start bit begins
// a fresh one. A finished byte pulses `rx_valid` with `rx_status`: 0, __X_PS2_PARITY_ERROR__ for a parity error,
// __X_PS2_FRAMING_ERROR__ for a bad start or stop bit.
//
// Host to device: hold the clock low for INHIBIT_US, pull data low (the start bit) and release the clock; the
// device then clocks the byte in — the host puts each next bit on data after a falling edge: eight data bits,
// parity, then releases data for the stop bit — and the device answers by pulling data low at the eleventh
// falling edge. `tx_done` pulses at the end with `tx_ack` 1 if the device acknowledged; a device that never
// clocks within TX_TIMEOUT_US ends the send unacknowledged. Receiving pauses while a byte is sent.
//
// The line itself, for diagnosis: `line_falls` counts the clock's falling edges as they come (synchronized, not
// debounced) whenever the device pulls it low, `line_fell` pulses at each, and `line_min_low` is the shortest of
// those clock-low phases in microseconds (255 until one is shorter); `line_bytes` counts the bytes read whole and
// `line_errors` those read damaged (it stops at 255). A bridge that sends nothing leaves the edge count still; one
// that clocks faster than the debounce reads shows a low phase under 8 us and edges with no bytes.
module term_ps2 #(
  parameter CLK_HZ = 100_000_000,
  parameter INHIBIT_US = 120,
  parameter RX_TIMEOUT_US = 200,
  parameter TX_TIMEOUT_US = 20_000
) (
  input            clk,
  input            rst,
  input            ps2_clk_in,
  input            ps2_data_in,
  output reg       ps2_clk_low = 1'b0,
  output reg       ps2_data_low = 1'b0,
  output reg [7:0] rx_byte,
  output reg [7:0] rx_status,
  output reg       rx_valid,
  input      [7:0] tx_byte,
  input            tx_start,
  output           tx_busy,
  output reg       tx_done,
  output reg       tx_ack,
  output reg [15:0] line_falls = 16'd0,
  output reg [7:0]  line_min_low = 8'hFF,
  output reg        line_fell = 1'b0,
  output reg [15:0] line_bytes = 16'd0,
  output reg [7:0]  line_errors = 8'd0
);
  localparam TICK = CLK_HZ / 1_000_000;           // board clocks per microsecond
  localparam TW = $clog2(TICK + 1);
  localparam UW = $clog2(TX_TIMEOUT_US + 1);

  // One-microsecond sampling tick.
  reg [TW-1:0] tick_cnt = 0;
  wire tick = (tick_cnt == TICK - 1);
  always @(posedge clk) tick_cnt <= tick ? 0 : tick_cnt + 1'b1;

  // Synchronize both lines; debounce the clock over 8 samples.
  reg c1 = 1'b1, c2 = 1'b1, d1 = 1'b1, d2 = 1'b1;
  reg [7:0] cwin = 8'hFF;
  reg cfilt = 1'b1, cfilt_prev = 1'b1;
  always @(posedge clk) begin
    c1 <= ps2_clk_in;  c2 <= c1;
    d1 <= ps2_data_in; d2 <= d1;
    if (tick) begin
      cwin <= {cwin[6:0], c2};
      if (cwin == 8'hFF) cfilt <= 1'b1;
      else if (cwin == 8'h00) cfilt <= 1'b0;
    end
    cfilt_prev <= cfilt;
  end
  wire fall = cfilt_prev && !cfilt;

  // The line monitor: a low phase this port drives itself (the inhibit before a send) is neither counted nor timed.
  reg       c3 = 1'b1, own_low = 1'b0;
  reg [7:0] low_us = 8'd0;
  always @(posedge clk) begin
    c3 <= c2;
    line_fell <= 1'b0;
    if (rx_valid) begin
      if (rx_status == 8'd0) line_bytes <= line_bytes + 1'b1;
      else if (line_errors != 8'hFF) line_errors <= line_errors + 1'b1;
    end
    if (rst) begin
      line_falls <= 16'd0;
      line_min_low <= 8'hFF;
      line_bytes <= 16'd0;
      line_errors <= 8'd0;
      own_low <= 1'b0;
      low_us <= 8'd0;
    end else if (c3 && !c2) begin
      own_low <= ps2_clk_low;
      low_us <= 8'd0;
      if (!ps2_clk_low) begin
        line_falls <= line_falls + 1'b1;
        line_fell <= 1'b1;
      end
    end else if (!c3 && c2) begin
      if (!own_low && low_us < line_min_low) line_min_low <= low_us;
    end else if (!c2 && tick && low_us != 8'hFF) begin
      low_us <= low_us + 1'b1;
    end
  end

  // The direction state machine.
  localparam S_RX = 3'd0, S_INHIBIT = 3'd1, S_RTS = 3'd2, S_BITS = 3'd3, S_ACK = 3'd4, S_RELEASE = 3'd5;
  reg [2:0]    state = S_RX;
  reg [UW-1:0] us = 0;                              // microseconds in the current wait
  reg [3:0]    nbit = 4'd0;
  reg [10:0]   frame = 11'd0;
  reg [8:0]    out = 9'd0;                          // data bits then parity, sent LSB first
  assign tx_busy = (state != S_RX);

  always @(posedge clk) begin
    rx_valid <= 1'b0;
    tx_done <= 1'b0;
    if (rst) begin
      state <= S_RX;
      nbit <= 4'd0;
      us <= 0;
      ps2_clk_low <= 1'b0;
      ps2_data_low <= 1'b0;
    end else begin
      case (state)
        S_RX: begin
          if (tx_start) begin
            out <= {~^tx_byte, tx_byte};
            ps2_clk_low <= 1'b1;
            us <= 0;
            nbit <= 4'd0;
            state <= S_INHIBIT;
          end else if (fall) begin
            us <= 0;
            frame <= {d2, frame[10:1]};
            if (nbit == 4'd10) begin
              nbit <= 4'd0;
              rx_byte <= frame[9:2];
              rx_status <= (frame[1] != 1'b0 || d2 != 1'b1) ? __V_PS2_FRAMING_ERROR__ :
                           (^frame[10:2] != 1'b1)            ? __V_PS2_PARITY_ERROR__ : 8'd0;
              rx_valid <= 1'b1;
            end else nbit <= nbit + 1'b1;
          end else if (tick && nbit != 4'd0) begin
            if (us == RX_TIMEOUT_US) begin nbit <= 4'd0; us <= 0; end
            else us <= us + 1'b1;
          end
        end
        S_INHIBIT:
          if (tick) begin
            if (us == INHIBIT_US) begin
              ps2_data_low <= 1'b1;
              us <= 0;
              state <= S_RTS;
            end else us <= us + 1'b1;
          end
        S_RTS:
          if (tick) begin
            ps2_clk_low <= 1'b0;
            us <= 0;
            state <= S_BITS;
          end
        S_BITS:
          if (fall) begin
            us <= 0;
            if (nbit == 4'd9) begin
              ps2_data_low <= 1'b0;
              state <= S_ACK;
            end else begin
              ps2_data_low <= ~out[0];
              out <= {1'b1, out[8:1]};
              nbit <= nbit + 1'b1;
            end
          end else if (tick) begin
            if (us == TX_TIMEOUT_US) begin
              ps2_data_low <= 1'b0;
              tx_ack <= 1'b0;
              us <= 0;
              state <= S_RELEASE;
            end else us <= us + 1'b1;
          end
        S_ACK:
          if (fall) begin
            tx_ack <= !d2;
            us <= 0;
            state <= S_RELEASE;
          end else if (tick) begin
            if (us == TX_TIMEOUT_US) begin
              tx_ack <= 1'b0;
              us <= 0;
              state <= S_RELEASE;
            end else us <= us + 1'b1;
          end
        S_RELEASE:
          if (tick) begin
            if ((cfilt && d2) || us == TX_TIMEOUT_US) begin
              nbit <= 4'd0;
              tx_done <= 1'b1;
              state <= S_RX;
            end else us <= us + 1'b1;
          end
        default: state <= S_RX;
      endcase
    end
  end
endmodule
