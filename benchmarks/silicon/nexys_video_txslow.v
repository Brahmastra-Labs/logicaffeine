// nexys_video_txslow — localizer. Same continuous UART TX as nexys_video_txprobe, but paced at ~2 bytes/sec
// so the visible state proves the FSM independently of the UART pin: led[7:5] = message byte index (counts
// 0..6, visibly stepping ~2/sec as each byte is sent), led[0] = heartbeat. If those LEDs step but the host
// UART still sees nothing, the FSM + transmit logic are correct and the fault is isolated to the bank-14
// uart_tx pin/open-flow output. Byte still goes out at 115200 so the host can read it if the pin works.
module nexys_video_txslow (
  input clk,
  output reg uart_tx = 1'b1,
  output [7:0] led
);
  reg [1:0] dc = 2'd0;
  always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk;
  BUFG b (.I(dc[1]), .O(coreclk));

  localparam integer CPB = 217;
  reg [24:0] hb = 25'd0;
  always @(posedge coreclk) hb <= hb + 1'b1;

  localparam integer N = 7;
  reg [7:0] msg [0:N-1];
  initial begin
    msg[0]="A"; msg[1]="2"; msg[2]="0"; msg[3]="0"; msg[4]="T"; msg[5]=8'h0d; msg[6]=8'h0a;
  end

  localparam T_IDLE=2'd0, T_START=2'd1, T_DATA=2'd2, T_STOP=2'd3;
  reg [1:0] ts = T_IDLE; reg [8:0] tc = 0; reg [2:0] tbit = 0; reg [7:0] tsh;
  reg [2:0] idx = 0;
  reg [23:0] gap = 0;              // ~0.5 s between bytes at 25 MHz

  always @(posedge coreclk) begin
    case (ts)
      T_IDLE: begin
        if (gap == 24'd12500000) begin gap <= 0; tsh <= msg[idx]; ts <= T_START; tc <= 0; uart_tx <= 1'b0;
                                        idx <= (idx == N-1) ? 3'd0 : idx + 1'b1; end
        else gap <= gap + 1'b1;
      end
      T_START: if (tc == CPB-1) begin tc <= 0; tbit <= 0; ts <= T_DATA; uart_tx <= tsh[0]; end else tc <= tc + 1'b1;
      T_DATA:  if (tc == CPB-1) begin tc <= 0; if (tbit == 3'd7) begin ts <= T_STOP; uart_tx <= 1'b1; end else begin tbit <= tbit + 1'b1; uart_tx <= tsh[tbit+1]; end end else tc <= tc + 1'b1;
      T_STOP:  if (tc == CPB-1) begin tc <= 0; ts <= T_IDLE; uart_tx <= 1'b1; end else tc <= tc + 1'b1;
    endcase
  end

  assign led = {idx, 4'd0, hb[24]};
endmodule
