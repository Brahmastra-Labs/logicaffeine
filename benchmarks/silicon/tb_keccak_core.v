// tb_keccak_core — the RTL Keccak-f[1600] vs the native oracle (keccak_vec.hex).
`timescale 1ns/1ps
module tb_keccak_core;
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, start = 0;
  reg [63:0] mem [0:49];
  reg [1599:0] s_in, expect_out;
  wire done; wire [1599:0] s_out;
  integer i;
  reg seen_done = 0;
  keccak_core dut(.clk(clk), .rst(rst), .start(start), .s_in(s_in), .done(done), .s_out(s_out));
  always @(posedge clk) if (done) seen_done <= 1'b1;
  initial begin
    $readmemh("keccak_vec.hex", mem);
    for (i = 0; i < 25; i = i + 1) s_in[64*i +: 64] = mem[i];
    for (i = 0; i < 25; i = i + 1) expect_out[64*i +: 64] = mem[25 + i];
    rst = 1;
    repeat (3) @(posedge clk);
    #1 rst = 0;                    // deassert OFF the edge — no reset/sample race
    @(posedge clk); #1 start = 1;
    @(posedge clk); #1 start = 0;
    repeat (40) @(posedge clk);   // bounded — 24-round permutation + margin, never hangs
    if (!seen_done)
      $display("FAIL: Keccak core never asserted done");
    else if (s_out === expect_out)
      $display("PASS: Keccak-f[1600] RTL core matches the native oracle bit-exact");
    else
      $display("FAIL: Keccak mismatch");
    $finish;
  end
endmodule
