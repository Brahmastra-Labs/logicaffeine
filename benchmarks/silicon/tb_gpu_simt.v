// tb_gpu_simt — the SIMT engine's N-lane map (+ warp-reduce) vs the RUNNING VM's output.
`timescale 1ns/1ps
module tb_gpu_simt;
  reg clk = 0; reg rst, load, go; reg [63:0] din; reg [7:0] raddr;
  wire [63:0] dout; wire done;
  reg  [63:0] oin  [0:311];
  reg  [63:0] oout [0:119];
  integer i, s, r, errors = 0;
  gpu_simt dut(.clk(clk), .rst(rst), .load(load), .din(din), .go(go),
                 .raddr(raddr), .dout(dout), .done(done));
  always #5 clk = ~clk;
  initial begin
    $readmemh("gpu_simt_in.hex", oin);
    $readmemh("gpu_simt_out.hex", oout);
    rst = 1; load = 0; go = 0; din = 0; raddr = 0;
    for (i = 0; i < 24; i = i + 1) begin
      rst = 1; @(posedge clk); #1 rst = 0;
      for (s = 0; s < 13; s = s + 1) begin
        din = oin[i*13+s]; load = 1; @(posedge clk); #1;
      end
      load = 0; go = 1; @(posedge clk); #1 go = 0; repeat (65) @(posedge clk); 
      for (r = 0; r < 5; r = r + 1) begin
        raddr = r; #1;
        if (dout !== oout[i*5+r]) begin errors = errors + 1; $display("FAIL vec %0d read %0d", i, r); end
      end
    end
    if (errors == 0) $display("PASS: 24 SIMT evaluations match the VM (N-lane map, no warp scheduler)");
    else $display("FAIL: %0d SIMT mismatches", errors);
    $finish;
  end
endmodule
