`timescale 1ns/1ps
// AUTO-GENERATED from the run_soft_core ≡VM differential (codegen_sva/rtl.rs). Do not edit by hand.
// Regenerate: LOGOS_EMIT_SC_TB=1 cargo test -p logicaffeine-compile --lib the_soft_core_runs_logos_programs_identically_to_the_vm
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  integer n = 0; reg [31:0] seen [0:31];
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h33000000;
    dut.imem[1] = 32'h05010001;
    dut.imem[2] = 32'h37000100;
    dut.imem[3] = 32'h05020002;
    dut.imem[4] = 32'h37000200;
    dut.imem[5] = 32'h33030000;
    dut.imem[6] = 32'h05040002;
    dut.imem[7] = 32'h37030400;
    dut.imem[8] = 32'h05050003;
    dut.imem[9] = 32'h37030500;
    dut.imem[10] = 32'h44060003;
    dut.imem[11] = 32'h3E070600;
    dut.imem[12] = 32'h59000700;
    dut.imem[13] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 1 && seen[0] == 32'h00000003)
      $display("PASS: union");
    else begin
      $display("FAIL: union (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: union timeout"); $finish; end
endmodule
