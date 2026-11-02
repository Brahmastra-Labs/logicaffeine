// nexys_echo — UART loopback bring-up probe using the SAME RX+TX FSMs and /4 (25 MHz) clock as
// nexys_soc_scalar. It receives a byte on uart_rx and transmits it back on uart_tx. If the host sees its
// byte echoed, the RX path (two-flop synchronizer + mid-bit sampling) works on silicon at 115200 baud;
// if not, RX is the reason the scalar SoC never receives its program.
module nexys_echo (
  input clk, input rst, input uart_rx,
  output reg uart_tx = 1'b1,
  output [15:0] led
);
  reg [1:0] dc = 2'd0;
  always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk;
  BUFG b (.I(dc[1]), .O(coreclk));

  localparam integer CPB = 217;
  localparam integer CW  = 9;
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 4'd1;
  wire rstb = ~por[3];

  localparam R_IDLE=2'd0, R_START=2'd1, R_DATA=2'd2, R_STOP=2'd3;
  reg [1:0] rs; reg [CW-1:0] rc; reg [2:0] rbit; reg [7:0] rsh; reg rvalid; reg [7:0] rdata; reg rd1, rd2;
  localparam T_IDLE=2'd0, T_START=2'd1, T_DATA=2'd2, T_STOP=2'd3;
  reg [1:0] ts; reg [CW-1:0] tc; reg [2:0] tbit; reg [7:0] tsh; reg tstart; reg [7:0] tdata;
  wire tbusy = (ts != T_IDLE) || tstart;
  reg [7:0] last;
  assign led = {last, 8'd0};

  always @(posedge coreclk) begin
    rvalid <= 1'b0; tstart <= 1'b0; rd1 <= uart_rx; rd2 <= rd1;
    if (rstb) begin rs <= R_IDLE; ts <= T_IDLE; uart_tx <= 1'b1; rc <= 0; rbit <= 0; tc <= 0; tbit <= 0; end
    else begin
      case (rs)
        R_IDLE:  if (~rd2) begin rs <= R_START; rc <= 0; end
        R_START: if (rc == (CPB-1)/2) begin if (~rd2) begin rc <= 0; rbit <= 0; rs <= R_DATA; end else rs <= R_IDLE; end else rc <= rc + 1'b1;
        R_DATA:  if (rc == CPB-1) begin rc <= 0; rsh <= {rd2, rsh[7:1]}; if (rbit == 3'd7) rs <= R_STOP; else rbit <= rbit + 1'b1; end else rc <= rc + 1'b1;
        R_STOP:  if (rc == CPB-1) begin rc <= 0; rvalid <= 1'b1; rdata <= rsh; last <= rsh; rs <= R_IDLE; end else rc <= rc + 1'b1;
      endcase
      case (ts)
        T_IDLE:  if (tstart) begin tsh <= tdata; ts <= T_START; tc <= 0; uart_tx <= 1'b0; end
        T_START: if (tc == CPB-1) begin tc <= 0; tbit <= 0; ts <= T_DATA; uart_tx <= tsh[0]; end else tc <= tc + 1'b1;
        T_DATA:  if (tc == CPB-1) begin tc <= 0; if (tbit == 3'd7) begin ts <= T_STOP; uart_tx <= 1'b1; end else begin tbit <= tbit + 1'b1; uart_tx <= tsh[tbit+1]; end end else tc <= tc + 1'b1;
        T_STOP:  if (tc == CPB-1) begin tc <= 0; ts <= T_IDLE; uart_tx <= 1'b1; end else tc <= tc + 1'b1;
      endcase
      if (rvalid && !tbusy) begin tdata <= rdata; tstart <= 1'b1; end
    end
  end
endmodule
