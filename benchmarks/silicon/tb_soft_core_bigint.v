// tb_soft_core_bigint — the full soft-core runs the 256-bit BigInt lane (BigIntWide, op 114),
// routing the wide register bank to the proven bigint_alu. Load W0=100, W1=200; Show (W0+W1)=300,
// (W1-W0)=100, (W0<W1)=1. The dst-byte high nibble is the operation (0=FromInt,1=Add,2=Sub,3=Cmp,4=Low).
`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  always #5 clk = ~clk;
  reg [31:0] seen [0:7];
  integer n = 0, errors = 0;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    dut.imem[0]=32'h05000064; dut.imem[1]=32'h72000000;   // r0=100; W0 = FromInt r0
    dut.imem[2]=32'h050100C8; dut.imem[3]=32'h72010100;   // r1=200; W1 = FromInt r1
    dut.imem[4]=32'h72120001; dut.imem[5]=32'h72430200; dut.imem[6]=32'h59000300; // W2=W0+W1; r3=low(W2); Show 300
    dut.imem[7]=32'h72240100; dut.imem[8]=32'h72430400; dut.imem[9]=32'h59000300; // W4=W1-W0; r3=low(W4); Show 100
    dut.imem[10]=32'h72330001; dut.imem[11]=32'h59000300; // r3=(W0<W1); Show 1
    dut.imem[12]=32'h72550100; dut.imem[13]=32'h72430500; dut.imem[14]=32'h59000300; // W5=W1/W0=2 (bigint_div); r3=low; Show 2
    dut.imem[15]=32'h72660001; dut.imem[16]=32'h72430600; dut.imem[17]=32'h59000300; // W6=W0*W1=20000 (bigint mul); r3=low; Show 20000
    dut.imem[18]=32'h2B000000;                            // Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (seen[0] !== 32'd300)   begin errors=errors+1; $display("FAIL add: %0d != 300", seen[0]); end
    if (seen[1] !== 32'd100)   begin errors=errors+1; $display("FAIL sub: %0d != 100", seen[1]); end
    if (seen[2] !== 32'd1)     begin errors=errors+1; $display("FAIL cmp: %0d != 1", seen[2]); end
    if (seen[3] !== 32'd2)     begin errors=errors+1; $display("FAIL div: %0d != 2", seen[3]); end
    if (seen[4] !== 32'd20000) begin errors=errors+1; $display("FAIL mul: %0d != 20000", seen[4]); end
    if (n !== 5)               begin errors=errors+1; $display("FAIL count: %0d != 5", n); end
    if (errors == 0) $display("PASS: soft-core BigInt lane (add=300 sub=100 cmp=1 div=2 mul=20000 via bigint_alu+div+mul)");
    else $display("FAIL: soft-core BigInt lane, %0d error(s)", errors);
    $finish;
  end
  initial begin #200000; $display("FAIL: soft-core BigInt lane timeout"); $finish; end
endmodule
