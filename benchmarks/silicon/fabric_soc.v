// fabric_soc — a flashable wrapper around config_fabric (the reconfigurable CGRA of Phase D). Present
// `cfg` + inputs, pulse `rst`, and after ONE settle clock `result`/`cycles` are valid and `done` is
// high. `cycles == 1`: a spatial decision settles in one clock, the metric the interpreter pays
// hundreds of fetch-decode-execute cycles for. Proven ≡ the VM under iverilog by `fabric_soc_lock`.
module fabric_soc (
  input        clk,
  input        rst,          // active-high synchronous reset / start
  input  [2:0] cfg,          // the config word: which datapath the array becomes
  input signed [63:0] in0,
  input signed [63:0] in1,
  input signed [63:0] in2,
  output reg signed [63:0] result,
  output reg [15:0] cycles,   // clocks from start to a valid result (the honest cycles/op)
  output reg        done
);
  wire signed [63:0] fab_out;
  config_fabric u_fab(.in0(in0), .in1(in1), .in2(in2), .cfg(cfg), .result(fab_out));
  reg running;
  always @(posedge clk) begin
   if (rst) begin
    result <= 64'sd0; cycles <= 16'd0; done <= 1'b0; running <= 1'b1;
   end else if (running) begin
    // the combinational array has settled; latch it. One clock = one decision.
    result <= fab_out; cycles <= 16'd1; done <= 1'b1; running <= 1'b0;
   end
  end
endmodule
