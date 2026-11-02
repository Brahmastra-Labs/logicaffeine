// float_soc — the sequenced float datapath (binding fpu.v) under a cycle counter (X3b iverilog surface).
// `cycles` is the FPU pipeline latency (fp_mul/fp_addsub/fp_sqrt in sequence); `result` is the f64
// bit-pattern, valid when `done` rises.
module float_soc(
  input               clk,
  input               rst,
  input  [63:0]       in0,
  input  [63:0]       in1,
  output [63:0]       result,
  output [15:0]       cycles,
  output              done
);
  wire [63:0] dp_out; wire dp_done;
  reg [15:0] cyc; reg counting;
  float_fhypf u_dp(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .result(dp_out), .done(dp_done));
  always @(posedge clk) begin
   if (rst) begin cyc <= 16'd0; counting <= 1'b1; end
   else if (counting) begin if (dp_done) counting <= 1'b0; else cyc <= cyc + 16'd1; end
  end
  assign result = dp_out;
  assign cycles = cyc;
  assign done = dp_done;
endmodule
