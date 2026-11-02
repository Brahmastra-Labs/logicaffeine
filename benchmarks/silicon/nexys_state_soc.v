// nexys_state_soc — the sequenced STATE PLANE on a Nexys A7-100T (Stage X2c). A baked-in vector ROM
// drives state_grade (a LUTRAM-heap micro-sequencer) over the list-sum battery; each result streams
// out the UART TX as a 12-byte record {0xA5, vec, result[63:0] MSB-first, cycles[15:0]}. cycles is
// the HONEST multi-cycle state-plane latency (not the combinational 1). 100 MHz; 8N1 115200.
module nexys_state_soc #(parameter CLKS_PER_BIT = 868) (
  input        clk,
  input        rst,
  output reg   uart_tx,
  output [15:0] led
);
  localparam NVEC = 7;
  reg [5:0] vec;
  reg signed [63:0] v_in0, v_in1, v_in2;
  always @(*) begin
   v_in0 = 64'sd0; v_in1 = 64'sd0; v_in2 = 64'sd0;
   case (vec)
      0: begin v_in0 = 64'sd10; v_in1 = 64'sd20; v_in2 = 64'sd30; end
      1: begin v_in0 = 64'sd1; v_in1 = 64'sd2; v_in2 = 64'sd3; end
      2: begin v_in0 = 64'sd40; v_in1 = 64'sd40; v_in2 = 64'sd40; end
      3: begin v_in0 = -64'sd40; v_in1 = -64'sd40; v_in2 = -64'sd40; end
      4: begin v_in0 = 64'sd33; v_in1 = 64'sd33; v_in2 = 64'sd33; end
      5: begin v_in0 = 64'sd0; v_in1 = 64'sd0; v_in2 = 64'sd0; end
      6: begin v_in0 = 64'sd70; v_in1 = 64'sd20; v_in2 = 64'sd5; end
    default: ;
   endcase
  end
  // the sequenced datapath + a run controller.
  reg dp_rst; wire dp_done; wire signed [63:0] dp_out;
  state_grade u_dp(.clk(clk), .rst(dp_rst), .in0(v_in0), .in1(v_in1), .in2(v_in2), .result(dp_out), .done(dp_done));
  reg signed [63:0] cap_out; reg [15:0] cap_cyc;
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
  always @(posedge clk) begin
   if (rst) begin
    state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0;
    tx_byte<=8'd0; dp_rst<=1'b1; dprst_cnt<=4'd0; cap_out<=64'sd0; cap_cyc<=16'd0;
   end else case (state)
    S_RESET: if (rst_cnt==4'd8) begin state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0; end else rst_cnt<=rst_cnt+4'd1;
    // pulse the datapath reset so it latches inputs + zeroes its heap, then run.
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
