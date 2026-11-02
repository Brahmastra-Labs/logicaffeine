// nexys_fabric_soc — the Phase-D reconfigurable fabric on a Nexys A7-100T, measuring its own settle. A
// baked-in vector ROM steps the ONE config_fabric array through every corpus datapath. For each vector a
// settle sweep primes the launch register, launches the vector, and captures the array's output L clocks
// later, for L from LMAX-1 down to 1, under two primes (the inputs inverted; the next vector), REPS times.
// The settled value is the capture at 2*LMAX, and it counts only when the capture at LMAX already equals it.
// Each record streams out the UART TX as {0xA5, cfg, result[63:0] MSB-first, cycles[15:0]}: cycles is
// the smallest L from which every capture equals the settled result, the worst over the repetitions, and
// 0 when the result did not settle inside the window. 100 MHz board clock; 8N1 115200 (CLKS_PER_BIT=868).
// Host: scripts/fabric-host.py checks each result against the VM.
module nexys_fabric_soc #(parameter CLKS_PER_BIT = 868, parameter LMAX = 255, parameter REPS = 8) (
  input        clk,
  input        rst,
  output reg   uart_tx,
  output [15:0] led
);
  localparam NVEC = 14;
  // vector ROM: {cfg, in0, in1, in2} of vector i.
  function [194:0] rom(input [5:0] i);
   case (i)
      0: rom = {3'd0, 64'h0000000000000000, 64'h0000000000000000, 64'h0000000000000000};
      1: rom = {3'd0, 64'h0000000000000003, 64'h0000000000000000, 64'h0000000000000000};
      2: rom = {3'd0, 64'hfffffffffffffffb, 64'h0000000000000000, 64'h0000000000000000};
      3: rom = {3'd1, 64'h0000000000000005, 64'h0000000000000000, 64'h000000000000000a};
      4: rom = {3'd1, 64'hfffffffffffffffd, 64'h0000000000000000, 64'h000000000000000a};
      5: rom = {3'd1, 64'h0000000000000014, 64'h0000000000000000, 64'h000000000000000a};
      6: rom = {3'd2, 64'hfffffffffffffff9, 64'h0000000000000000, 64'h0000000000000000};
      7: rom = {3'd2, 64'h0000000000000000, 64'h0000000000000000, 64'h0000000000000000};
      8: rom = {3'd2, 64'h0000000000000009, 64'h0000000000000000, 64'h0000000000000000};
      9: rom = {3'd3, 64'h0000000000000003, 64'h0000000000000008, 64'h0000000000000000};
      10: rom = {3'd3, 64'h0000000000000008, 64'h0000000000000003, 64'h0000000000000000};
      11: rom = {3'd4, 64'h000000000000000a, 64'h0000000000000003, 64'h0000000000000000};
      12: rom = {3'd4, 64'h0000000000000064, 64'h0000000000000007, 64'h0000000000000000};
      13: rom = {3'd4, 64'hffffffffffffffec, 64'h0000000000000004, 64'h0000000000000000};
    default: rom = 0;
   endcase
  endfunction
  reg  [5:0] vec;
  wire [5:0] nxt_vec = (vec == NVEC - 1) ? 6'd0 : vec + 6'd1;
  wire [194:0] tgt = rom(vec);
  wire [194:0] nxt = rom(nxt_vec);
  // the launch register: the array computes from these.
  reg [2:0] l_cfg; reg [63:0] l_in0, l_in1, l_in2;
  wire [63:0] fab_out;
  config_fabric u_fab(.in0(l_in0), .in1(l_in1), .in2(l_in2), .cfg(l_cfg), .result(fab_out));
  localparam M_PRIME=4'd0, M_SETTLE=4'd1, M_LAUNCH=4'd2, M_WAIT=4'd3, M_CHECK=4'd4,
              M_LOAD=4'd5, M_START=4'd6, M_DATA=4'd7, M_STOP=4'd8;
  reg [3:0]  m;
  reg [15:0] cnt;      // clocks since the launch register was last loaded
  reg [15:0] lat;      // the capture latency under test
  reg [15:0] settle;   // the worst settle over the repetitions so far
  reg [15:0] rep;
  reg        scan;     // 0: the reference captures; 1: the descending scan
  reg        prime_b;  // 0: the inverted inputs prime the launch; 1: the next vector does
  reg        stable;   // every reference agreed: the result settled inside the window
  reg        have_ref; // result holds the first repetition's settled value
  reg [63:0] cap;      // the capture at LMAX (reference)
  reg [63:0] cap2;     // the capture at 2*LMAX (reference) or at lat (scan)
  reg [63:0] ref_v;    // this repetition's settled value
  reg [63:0] result;   // the settled value the record streams
  wire [15:0] want = scan ? lat : 16'd2 * LMAX;
  wire [15:0] miss = lat + 16'd1;
  wire [15:0] cycles_out = stable ? settle : 16'd0;
  wire        last_rep = (rep == REPS - 1);
  // UART TX byte sequencer: 12 bytes a record.
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [3:0]  pos;
  assign led = {m, vec, settle[5:0]};
  wire [7:0] res_byte = result[(9 - pos) * 8 +: 8];
  always @(posedge clk) begin
   if (rst) begin
    m<=M_PRIME; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; pos<=4'd0; tx_byte<=8'd0; vec<=6'd0;
    cnt<=16'd0; lat<=16'd0; settle<=16'd0; rep<=16'd0; scan<=1'b0; prime_b<=1'b0; stable<=1'b1; have_ref<=1'b0;
    l_cfg<=3'd0; l_in0<=64'd0; l_in1<=64'd0; l_in2<=64'd0; cap<=64'd0; cap2<=64'd0; ref_v<=64'd0; result<=64'd0;
   end else case (m)
    M_PRIME: begin
     if (!prime_b) begin
      l_cfg<=tgt[194:192]; l_in0<=~tgt[191:128]; l_in1<=~tgt[127:64]; l_in2<=~tgt[63:0];
     end else begin
      l_cfg<=nxt[194:192]; l_in0<=nxt[191:128]; l_in1<=nxt[127:64]; l_in2<=nxt[63:0];
     end
     cnt<=16'd1; m<=M_SETTLE;
    end
    M_SETTLE: if (cnt == 16'd2 * LMAX) m<=M_LAUNCH; else cnt<=cnt+16'd1;
    M_LAUNCH: begin
     l_cfg<=tgt[194:192]; l_in0<=tgt[191:128]; l_in1<=tgt[127:64]; l_in2<=tgt[63:0];
     cnt<=16'd1; m<=M_WAIT;
    end
    M_WAIT: begin
     if (!scan && cnt == LMAX) cap<=fab_out;
     if (cnt == want) begin cap2<=fab_out; m<=M_CHECK; end else cnt<=cnt+16'd1;
    end
    M_CHECK: begin
     if (!scan) begin
      if (cap != cap2) stable<=1'b0;
      if (!prime_b) begin ref_v<=cap2; prime_b<=1'b1; m<=M_PRIME; end
      else begin
       if (cap2 != ref_v) stable<=1'b0;
       if (!have_ref) begin result<=ref_v; have_ref<=1'b1; end else if (ref_v != result) stable<=1'b0;
       scan<=1'b1; prime_b<=1'b0; lat<=LMAX - 16'd1; m<=M_PRIME;
      end
     end else if (cap2 != ref_v || (prime_b && lat == 16'd1)) begin
      if (cap2 != ref_v) begin if (miss > settle) settle<=miss; end
      else if (settle == 16'd0) settle<=16'd1;
      if (last_rep) begin pos<=4'd0; m<=M_LOAD; end
      else begin rep<=rep+16'd1; scan<=1'b0; prime_b<=1'b0; m<=M_PRIME; end
     end else if (!prime_b) begin prime_b<=1'b1; m<=M_PRIME; end
     else begin prime_b<=1'b0; lat<=lat-16'd1; m<=M_PRIME; end
    end
    M_LOAD: begin
     case (pos)
      4'd0: tx_byte<=8'hA5;
      4'd1: tx_byte<={5'd0, tgt[194:192]};
      4'd10: tx_byte<=cycles_out[15:8];
      4'd11: tx_byte<=cycles_out[7:0];
      default: tx_byte<=res_byte;
     endcase
     clk_cnt<=16'd0; bit_idx<=3'd0; m<=M_START;
    end
    M_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; m<=M_DATA; end else clk_cnt<=clk_cnt+16'd1; end
    M_DATA: begin uart_tx<=tx_byte[bit_idx];
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) m<=M_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
    end
    M_STOP: begin uart_tx<=1'b1;
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;
      if (pos==4'd11) begin
       // the next vector, measured afresh; the stream loops so a host can lock onto any lap.
       vec<=nxt_vec; rep<=16'd0; settle<=16'd0; stable<=1'b1; have_ref<=1'b0; scan<=1'b0; prime_b<=1'b0; m<=M_PRIME;
      end else begin pos<=pos+4'd1; m<=M_LOAD; end
     end else clk_cnt<=clk_cnt+16'd1;
    end
    default: m<=M_PRIME;
   endcase
  end
endmodule
