// hart_uart_tx — the log UART's transmitter: 8 data bits, no parity, one stop bit, CLKS_PER_BIT clocks a bit. It
// takes a byte from the stream whenever it is idle (`ready`), and holds the line high between bytes.
module hart_uart_tx #(
  parameter CLKS_PER_BIT = 542
) (
  input        clk,
  input        rst,
  input  [7:0] byte_in,
  input        valid,
  output       ready,
  output reg   tx
);
  localparam CW = $clog2(CLKS_PER_BIT + 1);
  reg [9:0]    frame;
  reg [3:0]    left;
  reg [CW-1:0] count;
  assign ready = left == 4'd0;
  always @(posedge clk) begin
    if (rst) begin
      tx <= 1'b1;
      left <= 4'd0;
      count <= 0;
    end else if (left == 4'd0) begin
      tx <= 1'b1;
      if (valid) begin
        frame <= {1'b1, byte_in, 1'b0};
        left <= 4'd10;
        count <= 0;
      end
    end else begin
      tx <= frame[0];
      if (count == CLKS_PER_BIT - 1) begin
        count <= 0;
        frame <= {1'b1, frame[9:1]};
        left <= left - 4'd1;
      end else count <= count + 1'b1;
    end
  end
endmodule
