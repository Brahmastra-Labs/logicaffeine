// tb_bigint_alu — HW 256-bit add/sub/compare vs the software BigInt limb oracle.
`timescale 1ns/1ps
module tb_bigint_alu;
  reg  [255:0] a, b;
  reg  [255:0] oa   [0:63];
  reg  [255:0] ob   [0:63];
  reg  [256:0] osum [0:63];   // {cout, sum}
  reg  [256:0] osub [0:63];   // {borrow, diff}
  reg  [1:0]   ocmp [0:63];   // {eq, lt}
  wire [255:0] sum, diff;
  wire         cout, borrow, eq, lt;
  integer i, errors = 0;
  bigint_alu dut(.a(a), .b(b), .sum(sum), .cout(cout), .diff(diff),
                 .borrow(borrow), .eq(eq), .lt(lt));
  initial begin
    $readmemh("bigint_a.hex", oa);
    $readmemh("bigint_b.hex", ob);
    $readmemh("bigint_sum.hex", osum);
    $readmemh("bigint_sub.hex", osub);
    $readmemh("bigint_cmp.hex", ocmp);
    for (i = 0; i < 64; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      if ({cout, sum}    !== osum[i]) begin errors = errors + 1; $display("FAIL add %0d", i); end
      if ({borrow, diff} !== osub[i]) begin errors = errors + 1; $display("FAIL sub %0d", i); end
      if ({eq, lt}       !== ocmp[i]) begin errors = errors + 1; $display("FAIL cmp %0d", i); end
    end
    if (errors == 0) $display("PASS: 64 256-bit BigInt add/sub/compare match the software oracle");
    else $display("FAIL: %0d BigInt ALU mismatches", errors);
    $finish;
  end
endmodule
