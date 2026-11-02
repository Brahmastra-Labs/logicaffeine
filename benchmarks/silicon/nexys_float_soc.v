// nexys_float_soc — the sequenced FLOAT LANE on a Nexys A7-100T (Stage X3b). A baked-in float ROM
// drives float_fhypf (an fpu.v-bound micro-sequencer) over the hypotenuse battery; each f64 result
// streams out the UART TX as a 12-byte record {0xA5, vec, result[63:0] MSB-first, cycles[15:0]}.
// cycles = the FPU pipeline latency (fp_mul/fp_addsub/fp_sqrt in sequence).
// The multi-cycle FPU has deep combinational shifter stages (nextpnr Fmax ~13 MHz), so the 100 MHz
// board clock is DIVIDED by 16 to a ~6.25 MHz core clock the FPU meets timing on — running it at the
// full 100 MHz latches wrong intermediates (proven on silicon). 8N1 115200 at 6.25 MHz (CLKS_PER_BIT=54).
module nexys_float_soc #(parameter CLKS_PER_BIT = 54) (
  input        clk,
  input        rst,
  output reg   uart_tx,
  output [15:0] led
);
  // Divide the 100 MHz board clock to ~6.25 MHz so the multi-cycle FPU meets timing on real silicon.
  reg [3:0] clkdiv = 4'd0;
  always @(posedge clk) clkdiv <= clkdiv + 4'd1;
  wire coreclk = clkdiv[3];
  localparam NVEC = 7;
  reg [5:0] vec;
  reg [63:0] v_in0, v_in1;
  always @(*) begin
   v_in0 = 64'd0; v_in1 = 64'd0;
   case (vec)
      0: begin v_in0 = 64'h4008000000000000; v_in1 = 64'h4010000000000000; end
      1: begin v_in0 = 64'h4018000000000000; v_in1 = 64'h4020000000000000; end
      2: begin v_in0 = 64'h4014000000000000; v_in1 = 64'h4028000000000000; end
      3: begin v_in0 = 64'h4020000000000000; v_in1 = 64'h402e000000000000; end
      4: begin v_in0 = 64'h0000000000000000; v_in1 = 64'h401c000000000000; end
      5: begin v_in0 = 64'h4004000000000000; v_in1 = 64'h4018000000000000; end
      6: begin v_in0 = 64'h4022000000000000; v_in1 = 64'h4028000000000000; end
    default: ;
   endcase
  end
  reg dp_rst; wire dp_done; wire [63:0] dp_out;
  float_fhypf u_dp(.clk(coreclk), .rst(dp_rst), .in0(v_in0), .in1(v_in1), .result(dp_out), .done(dp_done));
  reg [63:0] cap_out; reg [15:0] cap_cyc;
  localparam S_RESET=4'd0, S_DRST=4'd1, S_DWAIT=4'd2, S_CAP=4'd3, S_LOAD=4'd4, S_START=4'd5, S_DATA=4'd6, S_STOP=4'd7, S_DONE=4'd8;
  reg [3:0]  state;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [3:0]  pos;
  reg [3:0]  rst_cnt;
  reg [3:0]  dprst_cnt;
  assign led = {state, vec[4:0], 7'd0};
  wire [7:0] res_byte = cap_out[(9 - pos) * 8 +: 8];
  always @(posedge coreclk) begin
   if (rst) begin
    state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0;
    tx_byte<=8'd0; dp_rst<=1'b1; dprst_cnt<=4'd0; cap_out<=64'd0; cap_cyc<=16'd0;
   end else case (state)
    S_RESET: if (rst_cnt==4'd8) begin state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0; end else rst_cnt<=rst_cnt+4'd1;
    S_DRST: begin dp_rst<=1'b1; if (dprst_cnt==4'd3) begin state<=S_DWAIT; dp_rst<=1'b0; cap_cyc<=16'd0; end else dprst_cnt<=dprst_cnt+4'd1; end
    S_DWAIT: begin dp_rst<=1'b0; if (dp_done) begin cap_out<=dp_out; state<=S_CAP; end else cap_cyc<=cap_cyc+16'd1; end
    S_CAP: begin pos<=4'd0; state<=S_LOAD; end
    S_LOAD: begin
     case (pos)
      4'd0: tx_byte<=8'hA5;
      4'd1: tx_byte<={2'd0, vec};
      4'd10: tx_byte<=cap_cyc[15:8];
      4'd11: tx_byte<=cap_cyc[7:0];
      default: tx_byte<=res_byte;
     endcase
     clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;
    end
    S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end
    S_DATA: begin uart_tx<=tx_byte[bit_idx];
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
    end
    S_STOP: begin uart_tx<=1'b1;
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;
      if (pos==4'd11) begin
       pos<=4'd0;
       if (vec==NVEC-1) vec<=6'd0; else vec<=vec+6'd1;
       state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0;
      end else begin pos<=pos+4'd1; state<=S_LOAD; end
     end else clk_cnt<=clk_cnt+16'd1;
    end
    S_DONE: uart_tx<=1'b1;
    default: state<=S_DONE;
   endcase
  end
endmodule
