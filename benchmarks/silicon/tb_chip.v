`timescale 1ns/1ps
// Chip-config RTL oracle: soft_core at the EXACT bitstream parameters the Nexys A7 runs
// (FRAMES=8, REGS=32, IMEM_WORDS=128, HEAP=2048). Loads a hex program, runs to halt, prints
// "RESULT: <shows>". Because it models the real heap/imem/frame sizes, it catches resource
// overflow (a benchmark input too big for the chip) — the software `run_soft_core` oracle can't.
//   iverilog -DNO_BIGINT -DHEXFILE="\"prog.hex\"" -DNWORDS=<n> soft_core.v alu.v shared_divider.v crdt_merge.v tb_chip.v
module tb;
  reg clk=0, rst=1; wire [31:0] out; wire ov, halted;
  soft_core #(.FRAMES(8), .REGS(32), .IMEM_WORDS(128), .HEAP(2048))
    dut(.clk(clk), .rst(rst), .out(out), .out_valid(ov), .halted(halted));
  integer n=0, i; reg [31:0] seen[0:63]; reg [31:0] mem[0:127];
  always #5 clk=~clk;
  always @(posedge clk) if (ov) begin seen[n]=out; n=n+1; end
  initial begin
    $readmemh(`HEXFILE, mem);
    #1; for (i=0;i<`NWORDS;i=i+1) dut.imem[i]=mem[i];
    #3 rst=0; wait(halted); #10;
    $write("RESULT:"); for (i=0;i<n;i=i+1) $write(" %0d", $signed(seen[i])); $display("");
    $finish;
  end
  initial begin #200000000 $display("RESULT: TIMEOUT"); $finish; end
endmodule
