// term_slip — SLIP framing (RFC 1055) for the Fabric Terminal's link: a frame ends with __X_SLIP_END__, and an END or
// __X_SLIP_ESC__ inside a frame travels as ESC + __X_SLIP_ESC_END__ / __X_SLIP_ESC_ESC__.

// The receiver turns link bytes into frame bytes. `first` marks a frame's first byte (its kind); `frame_end`
// pulses at the END of a frame that had bytes and no damage; `frame_drop` pulses once when an ESC is followed by
// anything but ESC_END / ESC_ESC, and the rest of that frame is discarded up to its END.
module term_slip_rx (
  input            clk,
  input            rst,
  input      [7:0] in,
  input            in_valid,
  output reg [7:0] out,
  output reg       out_valid,
  output reg       first,
  output reg       frame_end,
  output reg       frame_drop
);
  reg esc = 1'b0, inframe = 1'b0, damaged = 1'b0;

  always @(posedge clk) begin
    out_valid <= 1'b0;
    frame_end <= 1'b0;
    frame_drop <= 1'b0;
    if (rst) begin
      esc <= 1'b0;
      inframe <= 1'b0;
      damaged <= 1'b0;
    end else if (in_valid) begin
      if (in == __V_SLIP_END__) begin
        frame_end <= inframe && !damaged;
        esc <= 1'b0;
        inframe <= 1'b0;
        damaged <= 1'b0;
      end else if (damaged) begin
      end else if (esc) begin
        esc <= 1'b0;
        if (in == __V_SLIP_ESC_END__ || in == __V_SLIP_ESC_ESC__) begin
          out <= (in == __V_SLIP_ESC_END__) ? __V_SLIP_END__ : __V_SLIP_ESC__;
          out_valid <= 1'b1;
          first <= !inframe;
          inframe <= 1'b1;
        end else begin
          damaged <= 1'b1;
          frame_drop <= 1'b1;
        end
      end else if (in == __V_SLIP_ESC__) begin
        esc <= 1'b1;
      end else begin
        out <= in;
        out_valid <= 1'b1;
        first <= !inframe;
        inframe <= 1'b1;
      end
    end
  end
endmodule

// The transmitter takes a frame a byte at a time: the producer holds `in` / `in_last` with `in_valid` until
// `in_taken` pulses, and the byte goes out escaped, followed by END after the byte marked `in_last`. `tx_byte`
// with a `tx_start` pulse feeds a `term_uart_tx` whose `tx_ready` it waits on.
module term_slip_tx (
  input            clk,
  input            rst,
  input      [7:0] in,
  input            in_valid,
  input            in_last,
  output reg       in_taken,
  output reg [7:0] tx_byte,
  output reg       tx_start,
  input            tx_ready
);
  localparam IDLE = 2'd0, SETTLE = 2'd1, DRAIN = 2'd2;
  reg [1:0] state = IDLE;
  reg [7:0] second = 8'd0;
  reg       has_second = 1'b0, has_end = 1'b0;

  always @(posedge clk) begin
    in_taken <= 1'b0;
    tx_start <= 1'b0;
    if (rst) begin
      state <= IDLE;
      has_second <= 1'b0;
      has_end <= 1'b0;
    end else begin
      case (state)
        IDLE:
          if (in_valid && tx_ready && !in_taken) begin
            in_taken <= 1'b1;
            has_end <= in_last;
            if (in == __V_SLIP_END__ || in == __V_SLIP_ESC__) begin
              tx_byte <= __V_SLIP_ESC__;
              second <= (in == __V_SLIP_END__) ? __V_SLIP_ESC_END__ : __V_SLIP_ESC_ESC__;
              has_second <= 1'b1;
            end else begin
              tx_byte <= in;
              has_second <= 1'b0;
            end
            tx_start <= 1'b1;
            state <= SETTLE;
          end
        SETTLE: state <= DRAIN;
        DRAIN:
          if (tx_ready) begin
            if (has_second) begin
              tx_byte <= second;
              has_second <= 1'b0;
              tx_start <= 1'b1;
              state <= SETTLE;
            end else if (has_end) begin
              tx_byte <= __V_SLIP_END__;
              has_end <= 1'b0;
              tx_start <= 1'b1;
              state <= SETTLE;
            end else state <= IDLE;
          end
        default: state <= IDLE;
      endcase
    end
  end
endmodule
