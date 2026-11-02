`timescale 1ns/1ps
// FULL-config RTL oracle: soft_core at the wide FPGA parameters (FRAMES=64, REGS=256, IMEM_WORDS=512,
// HEAP=2048) — the config the register-heavy benchmarks (matrix_mult needs REGS>=91, knapsack likewise)
// require. Loads a hex program, runs to halt, prints "RESULT: <shows>". Mirrors tb_chip.v but at REGS=256
// with a wider imem-load buffer and a larger sim-time budget for the sync-read-BRAM core's multi-cycle
// operand bubbles + sequential Call/Return.
//   iverilog -g2012 -DNO_BIGINT -DHEXFILE="\"prog.hex\"" -DNWORDS=<n> soft_core.v alu.v shared_divider.v crdt_merge.v tb_chip256.v
module tb;
  reg clk=0, rst=1; wire [31:0] out; wire [63:0] outf; wire ov, ofv, halted;
  soft_core #(.FRAMES(64), .REGS(256), .IMEM_WORDS(512), .HEAP(2048))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .outf(outf), .outf_valid(ofv), .halted(halted));
  // A show goes into one 64-bit stream (int zero-extended, float = raw f64 bits) with a per-show float flag,
  // so a program mixing int and float Shows preserves order. Integer programs (the default) print `%0d`; a
  // FLOAT_SHOW build prints every float show as its 64-bit hex payload (the host maps bits -> decimal, the
  // same way `emit_fpu_soft_core_tb` compares — Verilog cannot reproduce f64 decimal formatting).
  integer n=0, i; reg [63:0] seen[0:63]; reg isf[0:63]; reg [31:0] mem[0:511];
  always #5 clk=~clk;
  always @(posedge clk) begin
    if (ov)  begin seen[n]={32'd0,out}; isf[n]=1'b0; n=n+1; end
    if (ofv) begin seen[n]=outf;        isf[n]=1'b1; n=n+1; end
  end
  initial begin
    $readmemh(`HEXFILE, mem);
    #1; for (i=0;i<`NWORDS;i=i+1) dut.imem[i]=mem[i];
    #3 rst=0; wait(halted); #10;
    $write("RESULT:");
    for (i=0;i<n;i=i+1) begin
`ifdef FLOAT_SHOW
      if (isf[i]) $write(" f%016h", seen[i]); else $write(" %0d", $signed(seen[i][31:0]));
`else
      $write(" %0d", $signed(seen[i][31:0]));
`endif
    end
    $display("");
    $finish;
  end
  initial begin #2000000000 $display("RESULT: TIMEOUT"); $finish; end
endmodule
