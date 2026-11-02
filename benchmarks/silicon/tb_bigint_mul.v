// tb_bigint_mul — HW 64x64 limb multiplier vs the software (a as u128)*(b as u128).
`timescale 1ns/1ps
module tb_bigint_mul;
  reg  [63:0]  a, b;
  reg  [63:0]  oa [0:63];
  reg  [63:0]  ob [0:63];
  reg  [127:0] op [0:63];
  wire [127:0] prod;
  integer i, errors = 0;
  bigint_mul dut(.a(a), .b(b), .prod(prod));
  initial begin
    $readmemh("bigint_mul_a.hex", oa);
    $readmemh("bigint_mul_b.hex", ob);
    $readmemh("bigint_mul_p.hex", op);
    for (i = 0; i < 64; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      if (prod !== op[i]) begin errors = errors + 1; $display("FAIL mul %0d", i); end
    end
    if (errors == 0) $display("PASS: 64 64x64 limb products match the software oracle");
    else $display("FAIL: %0d BigInt mul mismatches", errors);
    $finish;
  end
endmodule
