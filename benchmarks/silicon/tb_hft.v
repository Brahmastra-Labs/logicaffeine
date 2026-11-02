`timescale 1ns/1ps
// tb_hft — run a full LOGOS tick-to-trade program on the standalone full soft-core, collecting every
// Show value and the run-cycle count. Loads the .hex straight into imem ($readmemh), bypassing the SoC's
// 1-byte UART length field (the pipeline is >255 words), so the cycle-exact determinism proof does not
// depend on the loader protocol. Same core the bitstream routes; the load path does not affect run cycles.
//
//   iverilog -g2012 -s tb -o /tmp/tb_hft.vvp soft_core.v alu.v shared_divider.v crdt_merge.v \
//            bigint_alu.v bigint_div.v fpu.v tb_hft.v
//   vvp /tmp/tb_hft.vvp +HEX=/tmp/hft_pipeline.hex
//     -> RESULT: <shows...>  CYCLES: <n>
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out;
  wire out_valid, halted;
  soft_core #(.MAXELT(64)) dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));

  reg [31:0] shows [0:255];
  reg [31:0] shcyc [0:255];                              // cycle at which each Show fired (for per-msg latency)
  integer    nsh = 0;
  reg [31:0] cyc = 0;
  reg [1023:0] hexfile;
  integer k;

  always #5 clk = ~clk;                                  // 100 MHz

  always @(posedge clk) begin
    if (!rst && !halted) cyc <= cyc + 1;                 // run-cycle counter (run-start .. halt)
    if (out_valid) begin shows[nsh] = out; shcyc[nsh] = cyc; nsh = nsh + 1; end
  end

  initial begin
    if (!$value$plusargs("HEX=%s", hexfile)) begin $display("FAIL: need +HEX=<file>"); $finish; end
    for (k = 0; k < 1024; k = k + 1) dut.imem[k] = 32'h2B000000;   // Halt-fill (op 0x2B) so a runaway PC stops
    $readmemh(hexfile, dut.imem);
    #3 rst = 0;
    wait (halted);
    #10;
    $write("RESULT:");
    for (k = 0; k < nsh; k = k + 1) $write(" %0d", shows[k]);
    $display("  CYCLES: %0d", cyc);
    $write("SHOWCYC:");                                  // cycle each Show fired (3 per message)
    for (k = 0; k < nsh; k = k + 1) $write(" %0d", shcyc[k]);
    $display("");
    $finish;
  end

  initial begin #200000000 $display("FAIL: timeout"); $finish; end   // watchdog
endmodule
