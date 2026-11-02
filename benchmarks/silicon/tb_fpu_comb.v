// tb_fpu_comb — drive a combinational FPU comparison (lt/eq) with Rust-generated f64 vectors and assert
// BIT-EXACT reproduction of the Rust result. Only fp_cmp is combinational now (add/sub/mul are the
// multi-cycle FSMs in tb_fpu_seq.v). The op and vector file are selected via plusargs:
//   +OP=lt|eq   +HEX=<path to fpu_<op>.hex>
// Each line of the hex is: a_bits b_bits expected_bits (all 16 hex digits = 64 bits).
`timescale 1ns/1ps
module tb_fpu_comb;
  reg  [63:0] a, b, exp;
  wire        lt, eq;

  fp_cmp    u_cmp   (.a(a), .b(b), .lt(lt), .eq(eq));

  integer    errors, total, i, fd, code;
  reg [8*16-1:0] opname;
  reg [8*256-1:0] hexpath;
  integer sel;               // 3 lt, 4 eq

  reg [63:0] got;
  reg ok;

  // We read the hex file line-by-line via $fscanf (three hex tokens per line).
  reg [63:0] va, vb, vexp;

  initial begin
    errors = 0; total = 0;
    if (!$value$plusargs("OP=%s", opname)) begin $display("NO-OP-ARG"); $finish; end
    if (!$value$plusargs("HEX=%s", hexpath)) begin $display("NO-HEX-ARG"); $finish; end

    // map op name -> selector
    if      (opname == "lt")  sel = 3;
    else if (opname == "eq")  sel = 4;
    else begin $display("BAD-OP %0s", opname); $finish; end

    fd = $fopen(hexpath, "r");
    if (fd == 0) begin $display("CANT-OPEN %0s", hexpath); $finish; end

    while (!$feof(fd)) begin
      code = $fscanf(fd, "%h %h %h\n", va, vb, vexp);
      if (code == 3) begin
        a = va; b = vb; exp = vexp;
        #1;   // let combinational logic settle
        case (sel)
          3:    got = {63'd0, lt};
          4:    got = {63'd0, eq};
        endcase
        total = total + 1;
        ok = 1'b0;
        // comparison: exact 1-bit
        if (got[0] === exp[0]) ok = 1'b1;
        if (!ok) begin
          errors = errors + 1;
          if (errors <= 20)
            $display("FAIL[%0s] a=%h b=%h got=%h exp=%h", opname, va, vb, got, vexp);
        end
      end
    end
    $fclose(fd);
    if (errors == 0) $display("PASS: %0s bit-exact over %0d vectors", opname, total);
    else             $display("FAILSUM: %0s %0d/%0d mismatched", opname, errors, total);
    $finish;
  end
endmodule
