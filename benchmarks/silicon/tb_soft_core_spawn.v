`timescale 1ns/1ps
// Wave 4 concurrency: producer/consumer. Spawn runs produce (sends 1,2 into the channel) synchronously,
// then Main receives 1,2 (== rtl.rs program 29).
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown [0:3]; integer nsh = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown[nsh] = out; nsh = nsh + 1; end
  initial begin
    #1;
    dut.imem[0]  = 32'h5C000000;  // ChanNew jobs(r0)               (op 92=0x5C)
    dut.imem[1]  = 32'h62070001;  // Spawn produce(entry=7,args=r0,count=1) (op 98=0x62)
    dut.imem[2]  = 32'h5E010000;  // ChanRecv first(r1) from jobs
    dut.imem[3]  = 32'h59000100;  // Show r1 (=1)
    dut.imem[4]  = 32'h5E020000;  // ChanRecv second(r2) from jobs
    dut.imem[5]  = 32'h59000200;  // Show r2 (=2)
    dut.imem[6]  = 32'h2B000000;  // Halt
    dut.imem[7]  = 32'h05030001;  // [produce] r3 = 1
    dut.imem[8]  = 32'h5D000300;  // ChanSend jobs, r3 (=1)
    dut.imem[9]  = 32'h05030002;  // r3 = 2
    dut.imem[10] = 32'h5D000300;  // ChanSend jobs, r3 (=2)
    dut.imem[11] = 32'h2A000000;  // ReturnNothing                  (op 42=0x2A)
    #3 rst = 0; wait (halted); #10;
    if (nsh == 2 && shown[0] == 32'd1 && shown[1] == 32'd2)
      $display("PASS: soft-core producer/consumer (Spawn -> recv %0d,%0d)", shown[0], shown[1]);
    else $display("FAIL: nsh=%0d shown=%0d,%0d (expected 1,2)", nsh, shown[0], shown[1]);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
