// term_uart — the Fabric Terminal's byte UART: 8 data bits, no parity, one stop bit, CLKS_PER_BIT board clocks
// per bit. The receiver samples the middle of each bit after a two-flop synchronizer; a start bit that is gone at
// its middle was a glitch and is ignored, and a stop bit read low is reported as a framing error.

module term_uart_rx #(parameter CLKS_PER_BIT = 100) (
  input            clk,
  input            rst,
  input            rx,
  output reg [7:0] data,
  output reg       valid,
  output reg       framing_error
);
  localparam CW = $clog2(CLKS_PER_BIT + 1);
  localparam IDLE = 2'd0, START = 2'd1, DATA = 2'd2, STOP = 2'd3;
  reg          rx1 = 1'b1, rx2 = 1'b1;
  reg [1:0]    state = IDLE;
  reg [CW-1:0] cnt = 0;
  reg [2:0]    bitn = 3'd0;
  reg [7:0]    shift = 8'd0;

  always @(posedge clk) begin
    rx1 <= rx;
    rx2 <= rx1;
    valid <= 1'b0;
    framing_error <= 1'b0;
    if (rst) begin
      state <= IDLE;
      cnt <= 0;
    end else begin
      case (state)
        IDLE: if (!rx2) begin state <= START; cnt <= 0; end
        START:
          if (cnt == (CLKS_PER_BIT - 1) / 2) begin
            cnt <= 0;
            if (!rx2) begin state <= DATA; bitn <= 3'd0; end
            else state <= IDLE;
          end else cnt <= cnt + 1'b1;
        DATA:
          if (cnt == CLKS_PER_BIT - 1) begin
            cnt <= 0;
            shift <= {rx2, shift[7:1]};
            if (bitn == 3'd7) state <= STOP;
            else bitn <= bitn + 1'b1;
          end else cnt <= cnt + 1'b1;
        STOP:
          if (cnt == CLKS_PER_BIT - 1) begin
            cnt <= 0;
            state <= IDLE;
            if (rx2) begin data <= shift; valid <= 1'b1; end
            else framing_error <= 1'b1;
          end else cnt <= cnt + 1'b1;
      endcase
    end
  end
endmodule

// The transmitter takes `data` on a `start` pulse while `ready`, and drives the line idle-high between bytes.
module term_uart_tx #(parameter CLKS_PER_BIT = 100) (
  input        clk,
  input        rst,
  input  [7:0] data,
  input        start,
  output       ready,
  output reg   tx = 1'b1
);
  localparam CW = $clog2(CLKS_PER_BIT + 1);
  reg [9:0]    frame = 10'h3FF;
  reg [3:0]    left = 4'd0;
  reg [CW-1:0] cnt = 0;
  assign ready = (left == 4'd0);

  always @(posedge clk) begin
    if (rst) begin
      tx <= 1'b1;
      left <= 4'd0;
      cnt <= 0;
    end else if (left == 4'd0) begin
      tx <= 1'b1;
      if (start) begin
        frame <= {1'b1, data, 1'b0};
        left <= 4'd10;
        cnt <= 0;
      end
    end else begin
      tx <= frame[0];
      if (cnt == CLKS_PER_BIT - 1) begin
        cnt <= 0;
        frame <= {1'b1, frame[9:1]};
        left <= left - 1'b1;
      end else cnt <= cnt + 1'b1;
    end
  end
endmodule
