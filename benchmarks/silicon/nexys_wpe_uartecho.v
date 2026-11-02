// nexys_wpe_uartecho — bring-up diagnostic #2: the over-the-wire engine's EXACT UART RX + TX FSMs (lifted
// verbatim from wpe_soc_template.v), wired as a byte echo. It proves the FRAMED UART (start-bit detect, 8N1
// sampling at CLKS_PER_BIT=81, transmit) works at the 781.25 kHz core clock — i.e. the engine's rx_valid
// strobe fires with the correct byte and the TX path emits it. If the streamed bytes come back framed, the
// engine's UART front-end is sound and the no-reply is in the LOADER / INTERPRET / result-frame logic.
module nexys_wpe_uartecho #(parameter CLKS_PER_BIT = 81) (
  input         clk,
  input         rst,
  input         uart_rx,
  output reg    uart_tx,
  output [15:0] led
);
  reg [6:0] divcnt = 7'd0;
  always @(posedge clk) divcnt <= divcnt + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(divcnt[6]), .O(coreclk));
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 1'b1;
  wire rstc = rst | ~por[3];

  localparam CW = 20;
  localparam R_IDLE=2'd0, R_START=2'd1, R_DATA=2'd2, R_STOP=2'd3;
  reg [1:0] r_state; reg [CW-1:0] r_cnt; reg [2:0] r_bit; reg [7:0] r_shift;
  reg rx_valid; reg [7:0] rx_data; reg rx_d1, rx_d2;
  localparam T_IDLE=2'd0, T_START=2'd1, T_DATA=2'd2, T_STOP=2'd3;
  reg [1:0] t_state; reg [CW-1:0] t_cnt; reg [2:0] t_bit; reg [7:0] t_shift;
  reg tx_start; reg [7:0] tx_data;
  wire tx_busy = (t_state != T_IDLE) || tx_start;

  assign led = {14'd0, tx_busy, por[3]};

  always @(posedge coreclk) begin
    rx_valid <= 1'b0; tx_start <= 1'b0;
    rx_d1 <= uart_rx; rx_d2 <= rx_d1;
    if (rstc) begin
      r_state<=R_IDLE; t_state<=T_IDLE; uart_tx<=1'b1; r_cnt<=0; r_bit<=3'd0; t_cnt<=0; t_bit<=3'd0;
    end else begin
      case (r_state)
        R_IDLE:  if (~rx_d2) begin r_state<=R_START; r_cnt<=0; end
        R_START: if (r_cnt==(CLKS_PER_BIT-1)/2) begin
                   if (~rx_d2) begin r_cnt<=0; r_bit<=3'd0; r_state<=R_DATA; end else r_state<=R_IDLE;
                 end else r_cnt<=r_cnt+1'b1;
        R_DATA:  if (r_cnt==CLKS_PER_BIT-1) begin
                   r_cnt<=0; r_shift<={rx_d2, r_shift[7:1]};
                   if (r_bit==3'd7) r_state<=R_STOP; else r_bit<=r_bit+1'b1;
                 end else r_cnt<=r_cnt+1'b1;
        R_STOP:  if (r_cnt==CLKS_PER_BIT-1) begin r_cnt<=0; rx_valid<=1'b1; rx_data<=r_shift; r_state<=R_IDLE; end
                 else r_cnt<=r_cnt+1'b1;
      endcase
      case (t_state)
        T_IDLE:  if (tx_start) begin t_shift<=tx_data; t_state<=T_START; t_cnt<=0; uart_tx<=1'b0; end
        T_START: if (t_cnt==CLKS_PER_BIT-1) begin t_cnt<=0; t_bit<=3'd0; t_state<=T_DATA; uart_tx<=t_shift[0]; end
                 else t_cnt<=t_cnt+1'b1;
        T_DATA:  if (t_cnt==CLKS_PER_BIT-1) begin t_cnt<=0;
                   if (t_bit==3'd7) begin t_state<=T_STOP; uart_tx<=1'b1; end
                   else begin t_bit<=t_bit+1'b1; uart_tx<=t_shift[t_bit+1]; end
                 end else t_cnt<=t_cnt+1'b1;
        T_STOP:  if (t_cnt==CLKS_PER_BIT-1) begin t_cnt<=0; t_state<=T_IDLE; uart_tx<=1'b1; end
                 else t_cnt<=t_cnt+1'b1;
      endcase
      // echo each received byte
      if (rx_valid && !tx_busy) begin tx_data <= rx_data; tx_start <= 1'b1; end
    end
  end
endmodule
