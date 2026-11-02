// nexys_heartbeat_div — like nexys_heartbeat but clocked by the SAME /4 BUFG divider the scalar SoC uses
// (coreclk = clk/4 = 25 MHz), transmitting 0x55 at 115200 baud (CLKS_PER_BIT = 25e6/115200 = 217). If the
// host sees 0x55, the divided-clock scheme itself works on silicon; if it goes silent while the plain
// 100 MHz heartbeat streams fine, the BUFG-from-divided-register clock is the culprit.
module nexys_heartbeat_div (
  input         clk, input rst, input uart_rx,
  output reg    uart_tx = 1'b1,
  output [15:0] led
);
  reg [1:0] divcnt = 2'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[1]), .O(coreclk));

  localparam integer CPB = 217;
  reg [15:0] cnt = 16'd0;
  reg [3:0]  idx = 4'd0;
  localparam [7:0] DATA = 8'h55;
  always @(posedge coreclk) begin
    if (cnt == CPB-1) begin
      cnt <= 16'd0;
      case (idx)
        4'd0:    uart_tx <= 1'b0;
        4'd9:    uart_tx <= 1'b1;
        default: uart_tx <= DATA[idx-4'd1];
      endcase
      idx <= (idx == 4'd9) ? 4'd0 : idx + 4'd1;
    end else cnt <= cnt + 16'd1;
  end
  assign led = 16'h5555;
endmodule
