// tb_state_soc — the sequenced state plane vs the RUNNING VM over the list-sum battery.
`timescale 1ns/1ps
module tb_state_soc;
  reg clk = 0, rst;
  reg signed [63:0] in0, in1, in2;
  wire signed [63:0] result;
  wire [15:0] cycles;
  wire done;
  integer c;
  state_soc dut(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .in2(in2), .result(result), .cycles(cycles), .done(done));
  always #5 clk = ~clk;
  task run(input signed [63:0] a0, input signed [63:0] a1, input signed [63:0] a2);
  begin
   @(negedge clk); in0 = a0; in1 = a1; in2 = a2; rst = 1;
   @(negedge clk); rst = 0;
   c = 0; while (!done && c < 200) begin @(negedge clk); c = c + 1; end
   $display("%0d %0d", result, cycles);
  end
  endtask
  initial begin
    run(64'sd10, 64'sd20, 64'sd30);
    run(64'sd1, 64'sd2, 64'sd3);
    run(64'sd40, 64'sd40, 64'sd40);
    run(-64'sd40, -64'sd40, -64'sd40);
    run(64'sd33, 64'sd33, 64'sd33);
    run(64'sd0, 64'sd0, 64'sd0);
    run(64'sd70, 64'sd20, 64'sd5);
   $finish;
  end
endmodule
