// nexys_echo_soc — a UART RX+store diagnostic sharing the stream SoC's EXACT clocking (/4 BUFG core
// clock + power-on reset, CLKS_PER_BIT = 217) and load path. Receives 64 bytes over UART RX into
// imgmem, then streams them straight back as 3-byte records {0xA5, index, byte}, looping. If the
// echoed bytes == what the host sent, both RX byte-recovery AND the imgmem store are correct.
module nexys_echo_soc #(parameter CLKS_PER_BIT = 217) (
  input        clk,
  input        rst,
  input        uart_rx,
  output reg   uart_tx,
  output [15:0] led
);
  reg [1:0] dc = 2'd0;
  always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(dc[1]), .O(coreclk));
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 4'd1;
  wire rstb = ~por[3];

  localparam NPIX = 64;
  (* ram_style = "registers" *) reg [7:0] imgmem [0:NPIX-1];
  reg [6:0] rx_idx;
  reg       loaded;
  integer k;

  reg        rx_m, rx_s;
  reg [1:0]  rxst;
  localparam RX_IDLE = 2'd0, RX_START = 2'd1, RX_DATA = 2'd2, RX_STOP = 2'd3;
  reg [15:0] rx_cnt;
  reg [2:0]  rx_bi;
  reg [7:0]  rx_sh, rx_byte;
  reg        rx_done;

  localparam S_LOAD = 3'd0, S_START = 3'd1, S_DATA = 3'd2, S_STOP = 3'd3;
  reg [2:0]  state;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [1:0]  pos;
  reg [6:0]  evec;
  assign led = {rxst, rx_idx[3:0], loaded, evec[3:0], state, 2'd0};

  initial begin
    rx_idx = 7'd0; loaded = 1'b0; uart_tx = 1'b1;
    rxst = RX_IDLE; rx_cnt = 16'd0; rx_bi = 3'd0; rx_sh = 8'd0; rx_byte = 8'd0; rx_done = 1'b0;
    rx_m = 1'b1; rx_s = 1'b1;
    state = S_LOAD; clk_cnt = 16'd0; bit_idx = 3'd0; pos = 2'd0; evec = 7'd0; tx_byte = 8'd0;
    for (k = 0; k < NPIX; k = k + 1) imgmem[k] = 8'd0;
  end

  always @(posedge coreclk) begin
    rx_m <= uart_rx; rx_s <= rx_m;
    rx_done <= 1'b0;
    if (rst || rstb) begin rxst <= RX_IDLE; rx_cnt <= 16'd0; rx_bi <= 3'd0; end
    else case (rxst)
      RX_IDLE:  if (~rx_s) begin rx_cnt <= 16'd0; rxst <= RX_START; end
      RX_START: if (rx_cnt == (CLKS_PER_BIT >> 1)) begin
                  if (~rx_s) begin rx_cnt <= 16'd0; rx_bi <= 3'd0; rxst <= RX_DATA; end
                  else rxst <= RX_IDLE;
                end else rx_cnt <= rx_cnt + 16'd1;
      RX_DATA:  if (rx_cnt == CLKS_PER_BIT - 1) begin
                  rx_cnt <= 16'd0; rx_sh <= {rx_s, rx_sh[7:1]};
                  if (rx_bi == 3'd7) rxst <= RX_STOP; else rx_bi <= rx_bi + 3'd1;
                end else rx_cnt <= rx_cnt + 16'd1;
      RX_STOP:  if (rx_cnt == CLKS_PER_BIT - 1) begin
                  rx_cnt <= 16'd0; rx_byte <= rx_sh; rx_done <= 1'b1; rxst <= RX_IDLE;
                end else rx_cnt <= rx_cnt + 16'd1;
    endcase
  end

  wire [7:0] emit_val = imgmem[evec[5:0]];

  always @(posedge coreclk) begin
   if (rst || rstb) begin
    rx_idx<=7'd0; loaded<=1'b0; uart_tx<=1'b1;
    state<=S_LOAD; clk_cnt<=16'd0; bit_idx<=3'd0; pos<=2'd0; evec<=7'd0; tx_byte<=8'd0;
   end else if (!loaded) begin
    uart_tx <= 1'b1;
    if (rx_done) begin
     imgmem[rx_idx[5:0]] <= rx_byte;
     rx_idx <= rx_idx + 7'd1;
     if (rx_idx == NPIX[6:0] - 7'd1) begin loaded<=1'b1; state<=S_LOAD; pos<=2'd0; evec<=7'd0; end
    end
   end else begin
    case (state)
     S_LOAD: begin
      case (pos)
       2'd0: tx_byte<=8'hA5;
       2'd1: tx_byte<={1'd0, evec};
       default: tx_byte<=emit_val;
      endcase
      clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;
     end
     S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end
     S_DATA: begin uart_tx<=tx_byte[bit_idx];
      if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
     end
     S_STOP: begin uart_tx<=1'b1;
      if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;
       if (pos==2'd2) begin
        pos<=2'd0;
        if (evec==NPIX-1) evec<=7'd0; else evec<=evec+7'd1;
        state<=S_LOAD;
       end else begin pos<=pos+2'd1; state<=S_LOAD; end
      end else clk_cnt<=clk_cnt+16'd1;
     end
     default: state<=S_LOAD;
    endcase
   end
  end
endmodule
