`timescale 1ns/1ps
module tb_fabric_soc;
  reg clk = 0, rst = 0;
  reg [2:0] cfg;
  reg signed [63:0] in0;
  reg signed [63:0] in1;
  reg signed [63:0] in2;
  wire signed [63:0] result;
  wire [15:0] cycles;
  wire done;
  fabric_soc dut(.clk(clk), .rst(rst), .cfg(cfg), .in0(in0), .in1(in1), .in2(in2), .result(result), .cycles(cycles), .done(done));
  always #5 clk = ~clk;
  task run(input [2:0] c, input signed [63:0] a0, input signed [63:0] a1, input signed [63:0] a2);
  begin
   @(negedge clk); cfg=c; in0=0; in1=0; in2=0;  in0=a0; in1=a1; in2=a2; rst=1;
   @(negedge clk); rst=0;
   wait (done);
   @(negedge clk);
   $display("%0d %0d %0d", c, $signed(result), cycles);
  end
  endtask
  initial begin
  run(3'd0, 0, 0, 0);
  run(3'd0, 3, 0, 0);
  run(3'd0, -5, 0, 0);
  run(3'd1, 5, 0, 10);
  run(3'd1, -3, 0, 10);
  run(3'd1, 20, 0, 10);
  run(3'd2, -7, 0, 0);
  run(3'd2, 0, 0, 0);
  run(3'd2, 9, 0, 0);
  run(3'd3, 3, 8, 0);
  run(3'd3, 8, 3, 0);
  run(3'd4, 10, 3, 0);
  run(3'd4, 100, 7, 0);
  run(3'd4, -20, 4, 0);
   $finish;
  end
endmodule
