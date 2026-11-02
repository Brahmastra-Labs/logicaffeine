// nexys_video_txprobe — Board-A UART TX diagnostic. Continuously transmits "A200T\r\n" out uart_tx at
// 115200 (8N1) off the /4 (25 MHz) core clock, independent of RX. Reading the host UART then answers three
// questions at once: WHICH FT2232 channel the FPGA UART is wired to (whichever ttyUSB shows bytes), whether
// the TX pin/direction is right (clean "A200T" vs nothing), and whether the baud is right (clean vs garbage).
// led[0] heartbeat = alive; led[7:1] = low bits of the byte index being sent.
module nexys_video_txprobe (
  input clk,
  output reg uart_tx = 1'b1,
  output [7:0] led
);
  reg [1:0] dc = 2'd0;
  always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk;
  BUFG b (.I(dc[1]), .O(coreclk));

  localparam integer CPB = 217;   // 25 MHz / 115200
  reg [24:0] hb = 25'd0;
  always @(posedge coreclk) hb <= hb + 1'b1;

  // message ROM
  localparam integer N = 7;
  reg [7:0] msg [0:N-1];
  initial begin
    msg[0]="A"; msg[1]="2"; msg[2]="0"; msg[3]="0"; msg[4]="T"; msg[5]=8'h0d; msg[6]=8'h0a;
  end

  localparam T_IDLE=2'd0, T_START=2'd1, T_DATA=2'd2, T_STOP=2'd3;
  reg [1:0] ts = T_IDLE; reg [8:0] tc = 0; reg [2:0] tbit = 0; reg [7:0] tsh;
  reg [2:0] idx = 0;
  reg [13:0] gap = 0;             // inter-byte gap so the stream is continuous but paced

  always @(posedge coreclk) begin
    case (ts)
      T_IDLE: begin
        if (gap == 14'd8000) begin gap <= 0; tsh <= msg[idx]; ts <= T_START; tc <= 0; uart_tx <= 1'b0;
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
