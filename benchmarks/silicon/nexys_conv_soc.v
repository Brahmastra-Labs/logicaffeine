// nexys_conv_soc — a fixed-point 3x3 GAUSSIAN-BLUR CONVOLUTION on a Nexys A7-100T (Artix-7).
// A baked-in vector ROM steps the combinational `fabric_gaussblur3` stencil (kernel [1,2,1;2,4,2;1,2,1]
// >> 4) through 8 real 8-bit pixel windows; each blurred output pixel streams out the UART TX as a
// 12-byte record {0xA5, vec, result[63:0] MSB-first, cycles[15:0]}. cycles==1: the spatial stencil
// settles in one clock. 100 MHz board clock; 8N1 115200 (CLKS_PER_BIT=868). Host: conv-host.py, which
// checks each blurred pixel == the VM's conv on the same window. The supercompiler's CV kernel on silicon.
module nexys_conv_soc #(parameter CLKS_PER_BIT = 868) (
  input        clk,
  input        rst,
  output reg   uart_tx,
  output [15:0] led
);
  localparam NVEC = 8;
  reg [5:0] vec;
  reg signed [63:0] p0, p1, p2, p3, p4, p5, p6, p7, p8;
  always @(*) begin
   p0=0; p1=0; p2=0; p3=0; p4=0; p5=0; p6=0; p7=0; p8=0;
   case (vec)
      0: begin p0=0;   p1=0;   p2=0;   p3=0;   p4=0;   p5=0;   p6=0;   p7=0;   p8=0;   end
      1: begin p0=255; p1=255; p2=255; p3=255; p4=255; p5=255; p6=255; p7=255; p8=255; end
      2: begin p0=0;   p1=0;   p2=0;   p3=128; p4=128; p5=128; p6=255; p7=255; p8=255; end // horizontal edge
      3: begin p0=10;  p1=20;  p2=30;  p3=40;  p4=50;  p5=60;  p6=70;  p7=80;  p8=90;  end // ramp
      4: begin p0=200; p1=10;  p2=200; p3=10;  p4=240; p5=10;  p6=200; p7=10;  p8=200; end // checker
      5: begin p0=100; p1=100; p2=100; p3=100; p4=200; p5=100; p6=100; p7=100; p8=100; end // bright center
      6: begin p0=17;  p1=240; p2=3;   p3=199; p4=88;  p5=46;  p6=130; p7=7;   p8=251; end
      7: begin p0=64;  p1=64;  p2=64;  p3=64;  p4=64;  p5=64;  p6=64;  p7=64;  p8=64;  end
    default: ;
   endcase
  end
  wire signed [63:0] fab_out;
  fabric_gaussblur3 u_fab(.in0(p0), .in1(p1), .in2(p2), .in3(p3), .in4(p4),
                          .in5(p5), .in6(p6), .in7(p7), .in8(p8), .result(fab_out));
  localparam [15:0] CYCLES = 16'd1;
  localparam S_RESET=3'd0, S_LOAD=3'd1, S_START=3'd2, S_DATA=3'd3, S_STOP=3'd4, S_DONE=3'd5;
  reg [2:0]  state;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [3:0]  pos;          // 0..11 byte within the current vector record
  reg [3:0]  rst_cnt;
  assign led = {state, vec[4:0], 8'd0};
  // MSB-first result byte for pos 2..9 (pos 2 = result[63:56]).
  wire [7:0] res_byte = fab_out[(9 - pos) * 8 +: 8];
  always @(posedge clk) begin
   if (rst) begin
    state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0; tx_byte<=8'd0;
   end else case (state)
    S_RESET: if (rst_cnt==4'd8) state<=S_LOAD; else rst_cnt<=rst_cnt+4'd1;
    S_LOAD: begin
     case (pos)
      4'd0: tx_byte<=8'hA5;
      4'd1: tx_byte<={2'd0, vec};
      4'd10: tx_byte<=CYCLES[15:8];
      4'd11: tx_byte<=CYCLES[7:0];
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
       if (vec==NVEC-1) vec<=6'd0; else vec<=vec+6'd1; state<=S_LOAD;
      end else begin pos<=pos+4'd1; state<=S_LOAD; end
     end else clk_cnt<=clk_cnt+16'd1;
    end
    S_DONE: uart_tx<=1'b1;
    default: state<=S_DONE;
   endcase
  end
endmodule
