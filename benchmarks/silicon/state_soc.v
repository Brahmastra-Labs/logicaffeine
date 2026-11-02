// state_soc — the sequenced state-plane datapath under a cycle counter (Stage X2c iverilog surface).
// On `rst` the datapath restarts; the counter runs until `done`, so `cycles` is the HONEST multi-cycle
// state-plane latency (a heap program is NOT a 1-clock decision). `result` is valid when `done` rises.
module state_soc(
  input               clk,
  input               rst,
  input  signed [63:0] in0,
  input  signed [63:0] in1,
  input  signed [63:0] in2,
  output signed [63:0] result,
  output [15:0]       cycles,
  output              done
);
  wire signed [63:0] dp_out;
  wire dp_done;
  reg [15:0] cyc; reg counting;
  state_grade u_dp(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .in2(in2), .result(dp_out), .done(dp_done));
  always @(posedge clk) begin
   if (rst) begin cyc <= 16'd0; counting <= 1'b1; end
   else if (counting) begin if (dp_done) counting <= 1'b0; else cyc <= cyc + 16'd1; end
  end
  assign result = dp_out;
  assign cycles = cyc;
  assign done = dp_done;
endmodule
