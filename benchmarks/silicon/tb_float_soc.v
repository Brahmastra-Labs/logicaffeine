// tb_float_soc — the sequenced float lane (fpu.v) vs the f64 reference over the hypotenuse battery.
`timescale 1ns/1ps
module tb_float_soc;
  reg clk = 0, rst;
  reg [63:0] in0, in1;
  wire [63:0] result;
  wire [15:0] cycles;
  wire done;
  integer c;
  float_soc dut(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .result(result), .cycles(cycles), .done(done));
  always #5 clk = ~clk;
  task run(input [63:0] a, input [63:0] b);
  begin
   @(negedge clk); in0 = a; in1 = b; rst = 1;
   @(negedge clk); rst = 0;
   c = 0; while (!done && c < 5000) begin @(negedge clk); c = c + 1; end
   $display("%h %0d", result, cycles);
  end
  endtask
  initial begin
    run(64'h4008000000000000, 64'h4010000000000000);
    run(64'h4018000000000000, 64'h4020000000000000);
    run(64'h4014000000000000, 64'h4028000000000000);
    run(64'h4020000000000000, 64'h402e000000000000);
    run(64'h0000000000000000, 64'h401c000000000000);
    run(64'h4004000000000000, 64'h4018000000000000);
    run(64'h4022000000000000, 64'h4028000000000000);
   $finish;
  end
endmodule
