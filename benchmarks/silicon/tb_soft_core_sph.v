`timescale 1ns/1ps
// SpawnHandle + TaskAbort: Launch produce (sends 5) with handle h; Stop h (no-op); receive 5.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h5C000000;  // ChanNew jobs(r0)                (op 92=0x5C)
    dut.imem[1] = 32'h63010600;  // SpawnHandle handle=r1, entry=6  (op 99=0x63)
    dut.imem[2] = 32'h5E020000;  // ChanRecv x=r2 from jobs         (op 94=0x5E)
    dut.imem[3] = 32'h65010000;  // TaskAbort (Stop h)              (op 101=0x65)
    dut.imem[4] = 32'h59000200;  // Show r2
    dut.imem[5] = 32'h2B000000;  // Halt
    dut.imem[6] = 32'h05030005;  // [produce] r3=5
    dut.imem[7] = 32'h5D000300;  // ChanSend jobs, r3               (op 93=0x5D)
    dut.imem[8] = 32'h2A000000;  // ReturnNothing                   (op 42=0x2A)
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd5) $display("PASS: soft-core SpawnHandle+TaskAbort (recv = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 5)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
