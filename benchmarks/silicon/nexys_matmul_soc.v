// nexys_matmul_soc — an int8 dot-product (matmul inner-kernel) accelerator SoC on the Nexys A7-100T. The
// compute core `cgra_dot` is AUTO-EMITTED FROM LOGOS (`## To dot (xs): Repeat for x in xs: … acc += a·b`) with
// the int8-native datapath, so each MAC is one DSP48E1. The host streams two int8 vectors INTERLEAVED
// [a0,b0,a1,b1,…] over UART; the SoC loads them into the core's on-chip operand buffer (MemRead), runs the DSP
// MAC (1 MAC/clock), and streams back a result record {0xA5, result[63:0] MSB-first, cycles[15:0]} = 11 bytes.
// A real vector·vector dot on the board: send the operands, the DSP array computes, the answer comes back
// byte-exact. It LOOPS — after each result it returns to idle, so you flash once and fire request after
// request ("our LOGOS code just runs on the chip").
//
// Protocol (host → board): [L][x_0]..[x_{L-1}]  — L = interleaved element count (2·pairs, one byte, ≤255),
// each x a signed int8. /4 BUFG core clock (25 MHz from the 100 MHz osc); 8N1 115200 (CLKS_PER_BIT = 217).
module nexys_matmul_soc #(parameter CLKS_PER_BIT = 217, parameter REFRESH_BITS = 17) (
  input        clk,
  input        rst,
  input        uart_rx,
  output reg   uart_tx,
  output [15:0] led,
  output [6:0] seg,          // 7-seg cathodes (active-low)
  output       dp,           // decimal point (off)
  output [7:0] an            // 7-seg anodes (active-low, one-hot)
);
  // ── core clock + power-on reset ──────────────────────────────────────────────────────────────────
  reg [1:0] dc = 2'd0;
  always @(posedge clk) dc <= dc + 1'b1;
  wire coreclk;
  BUFG bufg_core (.I(dc[1]), .O(coreclk));
  reg [3:0] por = 4'd0;
  always @(posedge coreclk) if (!por[3]) por <= por + 4'd1;
  wire rstb = ~por[3] | rst;

  // ── UART RX (8N1) → rx_byte + rx_done ────────────────────────────────────────────────────────────
  reg rx_m, rx_s;
  reg [1:0] rxst;
  localparam RX_IDLE = 2'd0, RX_START = 2'd1, RX_DATA = 2'd2, RX_STOP = 2'd3;
  reg [15:0] rx_cnt; reg [2:0] rx_bi; reg [7:0] rx_sh, rx_byte; reg rx_done;
  initial begin rx_m=1'b1; rx_s=1'b1; rxst=RX_IDLE; rx_cnt=0; rx_bi=0; rx_sh=0; rx_byte=0; rx_done=0; uart_tx=1'b1; end
  always @(posedge coreclk) begin
    rx_m <= uart_rx; rx_s <= rx_m; rx_done <= 1'b0;
    if (rstb) begin rxst<=RX_IDLE; rx_cnt<=0; rx_bi<=0; end
    else case (rxst)
      RX_IDLE:  if (~rx_s) begin rx_cnt<=0; rxst<=RX_START; end
      RX_START: if (rx_cnt==(CLKS_PER_BIT>>1)) begin
                  if (~rx_s) begin rx_cnt<=0; rx_bi<=0; rxst<=RX_DATA; end else rxst<=RX_IDLE;
                end else rx_cnt<=rx_cnt+16'd1;
      RX_DATA:  if (rx_cnt==CLKS_PER_BIT-1) begin
                  rx_cnt<=0; rx_sh<={rx_s, rx_sh[7:1]};
                  if (rx_bi==3'd7) rxst<=RX_STOP; else rx_bi<=rx_bi+3'd1;
                end else rx_cnt<=rx_cnt+16'd1;
      RX_STOP:  if (rx_cnt==CLKS_PER_BIT-1) begin rx_cnt<=0; rx_byte<=rx_sh; rx_done<=1'b1; rxst<=RX_IDLE; end
                else rx_cnt<=rx_cnt+16'd1;
    endcase
  end

  // ── the LOGOS-emitted int8 MAC array (operand buffer written by the RX loader) ───────────────────
  reg  [63:0] seq_len;
  reg         mem_we; reg [8:0] mem_waddr; reg signed [63:0] mem_wdata;
  reg         arr_rst;
  wire [63:0] arr_result; wire arr_done;
  cgra_dot u_mac (
    .clk(coreclk), .rst(arr_rst), .seq_len(seq_len),
    .mem_we(mem_we), .mem_waddr(mem_waddr), .mem_wdata(mem_wdata),
    .result(arr_result), .done(arr_done)
  );

  // ── show the result in SIGNED DECIMAL on the 8-digit 7-seg display (100 MHz refresh domain) ──────────
  // result_lat updates rarely (once per completed dot) and holds, so a plain sample across the clock domain
  // is safe; the display starts at 0 (a lit "0") so it is visibly ON the instant the board is flashed.
  reg [31:0] disp_val = 32'd0;
  always @(posedge clk) disp_val <= result_lat[31:0];
  sevenseg_dec #(.REFRESH_BITS(REFRESH_BITS)) u_disp (.clk(clk), .rst(rst), .val(disp_val), .seg(seg), .dp(dp), .an(an));

  // ── control FSM: LOAD_LEN → LOAD_OPS → RUN → EMIT ────────────────────────────────────────────────
  localparam S_LEN=3'd0, S_OPS=3'd1, S_RUN=3'd2, S_EMIT=3'd3, S_DONE=3'd4;
  reg [2:0]  cs;
  reg [8:0]  n_ops, load_i;
  reg [63:0] result_lat; reg [15:0] cyc_lat, run_cyc;
  reg [3:0]  txpos;               // 0=0xA5, 1..8=result bytes, 9..10=cycles
  reg [7:0]  tx_byte; reg [2:0] bit_idx; reg [15:0] clk_cnt; reg [1:0] txst;
  localparam TX_IDLE=2'd0, TX_START=2'd1, TX_DATA=2'd2, TX_STOP=2'd3;
  assign led = {cs, result_lat[4:0], run_cyc[7:0]};

  initial begin
    cs=S_LEN; n_ops=0; load_i=0; seq_len=0; mem_we=0; mem_waddr=0; mem_wdata=0;
    arr_rst=1'b1; result_lat=0; cyc_lat=0; run_cyc=0; txpos=0; tx_byte=0; bit_idx=0; clk_cnt=0; txst=TX_IDLE;
  end

  always @(posedge coreclk) begin
    mem_we <= 1'b0;
    if (rstb) begin
      cs<=S_LEN; n_ops<=0; load_i<=0; seq_len<=0; arr_rst<=1'b1; txpos<=0; txst<=TX_IDLE; uart_tx<=1'b1; run_cyc<=0;
    end else case (cs)
      S_LEN: if (rx_done) begin n_ops<={1'b0, rx_byte}; load_i<=0; arr_rst<=1'b1; cs<=(rx_byte==8'd0)?S_RUN:S_OPS; end
      S_OPS: if (rx_done) begin
               mem_we<=1'b1; mem_waddr<=load_i; mem_wdata<={{56{rx_byte[7]}}, rx_byte}; // sign-extend int8
               load_i<=load_i+9'd1;
               if (load_i==n_ops-9'd1) begin seq_len<={55'd0, n_ops}; cs<=S_RUN; arr_rst<=1'b1; end
             end
      S_RUN: begin
               // hold arr_rst one core cycle after the last write settles, then release to run + count cycles.
               arr_rst<=1'b0; run_cyc<=16'd0;
               if (!arr_rst) begin
                 if (arr_done) begin result_lat<=arr_result; cyc_lat<=run_cyc; txpos<=0; txst<=TX_IDLE; cs<=S_EMIT; end
                 else run_cyc<=run_cyc+16'd1;
               end
             end
      S_EMIT: begin
                case (txst)
                  TX_IDLE: begin
                    case (txpos)
                      4'd0: tx_byte<=8'hA5;
                      4'd1: tx_byte<=result_lat[63:56]; 4'd2: tx_byte<=result_lat[55:48];
                      4'd3: tx_byte<=result_lat[47:40]; 4'd4: tx_byte<=result_lat[39:32];
                      4'd5: tx_byte<=result_lat[31:24]; 4'd6: tx_byte<=result_lat[23:16];
                      4'd7: tx_byte<=result_lat[15:8];  4'd8: tx_byte<=result_lat[7:0];
                      4'd9: tx_byte<=cyc_lat[15:8];    default: tx_byte<=cyc_lat[7:0];
                    endcase
                    clk_cnt<=0; bit_idx<=0; txst<=TX_START;
                  end
                  TX_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=0; txst<=TX_DATA; end else clk_cnt<=clk_cnt+16'd1; end
                  TX_DATA:  begin uart_tx<=tx_byte[bit_idx];
                              if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=0; if (bit_idx==3'd7) txst<=TX_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
                            end
                  TX_STOP:  begin uart_tx<=1'b1;
                              if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=0;
                                // after the last byte, RETURN TO IDLE — ready for the next request with no
                                // re-flash. Hold the compute array in reset until the next vector arrives.
                                if (txpos==4'd10) begin cs<=S_LEN; arr_rst<=1'b1; load_i<=9'd0; end
                                else txpos<=txpos+4'd1;
                                txst<=TX_IDLE;
                              end else clk_cnt<=clk_cnt+16'd1;
                            end
                endcase
              end
      default: begin cs<=S_LEN; arr_rst<=1'b1; end // recover to idle, ready for a request
    endcase
  end
endmodule
