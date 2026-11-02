// tb_fpu_seq — drive a sequential FPU op (add/sub/mul/div/sqrt) via its start/done handshake with
// Rust-generated f64 vectors and assert BIT-EXACT reproduction of the Rust result. Selected via plusargs:
//   +OP=add|sub|mul|div|sqrt   +HEX=<path to fpu_<op>.hex>
// Line format: a_bits b_bits expected_bits (sqrt ignores b). NaN results accept any quiet-NaN pattern.
// add/sub/mul/div/sqrt are all multi-cycle now (a small shifter-staging FSM), so every arithmetic op is
// driven through the same start/done protocol; only fp_cmp (lt/eq) stays combinational (tb_fpu_comb.v).
`timescale 1ns/1ps
module tb_fpu_seq;
  reg         clk = 0, rst = 1;
  reg         start_as = 0, start_mul = 0, start_div = 0, start_sqrt = 0;
  reg         op_sub = 0;              // fp_addsub select (0 add, 1 sub)
  reg  [63:0] a, b, exp;
  wire [63:0] y_as, y_mul, y_div, y_sqrt;
  wire        done_as, done_mul, done_div, done_sqrt;

  fp_addsub u_as (.clk(clk), .rst(rst), .start(start_as),  .a(a), .b_in(b), .sub(op_sub), .y(y_as),   .done(done_as));
  fp_mul    u_mul(.clk(clk), .rst(rst), .start(start_mul), .a(a), .b(b),                  .y(y_mul),  .done(done_mul));
  fp_div    u_div(.clk(clk), .rst(rst), .start(start_div), .a(a), .b(b),                  .y(y_div),  .done(done_div));
  fp_sqrt   u_sqrt(.clk(clk), .rst(rst), .start(start_sqrt), .a(a),                       .y(y_sqrt), .done(done_sqrt));

  always #5 clk = ~clk;

  integer errors, total, fd, code, sel;
  reg [8*16-1:0] opname;
  reg [8*256-1:0] hexpath;
  reg [63:0] va, vb, vexp, got;
  reg ok;

  function is_nan(input [63:0] v);
    is_nan = (v[62:52] == 11'h7FF) && (v[51:0] != 52'd0);
  endfunction

  task run_addsub(input [63:0] ta, input [63:0] tb_, input tsub);
    begin
      a = ta; b = tb_; op_sub = tsub;
      @(negedge clk); start_as = 1'b1;
      @(negedge clk); start_as = 1'b0;
      while (!done_as) @(negedge clk);
      got = y_as;
      @(negedge clk);
    end
  endtask

  task run_mul(input [63:0] ta, input [63:0] tb_);
    begin
      a = ta; b = tb_;
      @(negedge clk); start_mul = 1'b1;
      @(negedge clk); start_mul = 1'b0;
      while (!done_mul) @(negedge clk);
      got = y_mul;
      @(negedge clk);
    end
  endtask

  task run_div(input [63:0] ta, input [63:0] tb_);
    begin
      a = ta; b = tb_;
      @(negedge clk); start_div = 1'b1;
      @(negedge clk); start_div = 1'b0;
      while (!done_div) @(negedge clk);
      got = y_div;
      @(negedge clk);   // let done settle back before next op
    end
  endtask

  task run_sqrt(input [63:0] ta);
    begin
      a = ta; b = 64'd0;
      @(negedge clk); start_sqrt = 1'b1;
      @(negedge clk); start_sqrt = 1'b0;
      while (!done_sqrt) @(negedge clk);
      got = y_sqrt;
      @(negedge clk);
    end
  endtask

  initial begin
    errors = 0; total = 0;
    if (!$value$plusargs("OP=%s", opname))  begin $display("NO-OP-ARG"); $finish; end
    if (!$value$plusargs("HEX=%s", hexpath)) begin $display("NO-HEX-ARG"); $finish; end
    if      (opname == "add")  sel = 0;
    else if (opname == "sub")  sel = 1;
    else if (opname == "mul")  sel = 2;
    else if (opname == "div")  sel = 3;
    else if (opname == "sqrt") sel = 4;
    else begin $display("BAD-OP %0s", opname); $finish; end

    #12 rst = 0;
    @(negedge clk);

    fd = $fopen(hexpath, "r");
    if (fd == 0) begin $display("CANT-OPEN %0s", hexpath); $finish; end

    while (!$feof(fd)) begin
      code = $fscanf(fd, "%h %h %h\n", va, vb, vexp);
      if (code == 3) begin
        exp = vexp;
        case (sel)
          0: run_addsub(va, vb, 1'b0);
          1: run_addsub(va, vb, 1'b1);
          2: run_mul(va, vb);
          3: run_div(va, vb);
          4: run_sqrt(va);
        endcase
        total = total + 1;
        ok = 1'b0;
        if (got === exp) ok = 1'b1;
        else if (is_nan(exp) && is_nan(got)) ok = 1'b1;
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
