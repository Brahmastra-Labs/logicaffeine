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
    dut.imem[0] = 32'hC8000000;
    dut.imem[1] = 32'h000186A0;
    dut.imem[2] = 32'h59000000;
    dut.imem[3] = 32'hC8010000;
    dut.imem[4] = 32'h3B9ACA07;
    dut.imem[5] = 32'h59000100;
    dut.imem[6] = 32'hC8030000;
    dut.imem[7] = 32'h00009C40;
    dut.imem[8] = 32'h05040032;
    dut.imem[9] = 32'h0B020304;
    dut.imem[10] = 32'h59000200;
    dut.imem[11] = 32'h2B000000;
    #3 rst = 0;
    wait (halted);
    #10;
    if (n == 3 && seen[0] == 32'h000186A0 && seen[1] == 32'h3B9ACA07 && seen[2] == 32'h001E8480)
      $display("PASS: full_wideconst");
    else begin
      $display("FAIL: full_wideconst (got n=%0d)", n);
      $display("  seen[0] = %0d", $signed(seen[0]));
      $display("  seen[1] = %0d", $signed(seen[1]));
      $display("  seen[2] = %0d", $signed(seen[2]));
    end
    $finish;
  end
  initial begin #1000000000 $display("FAIL: full_wideconst timeout"); $finish; end
endmodule
