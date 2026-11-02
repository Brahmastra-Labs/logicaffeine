// tb_secded — SECDED(72,64) encode -> inject fault -> decode, vs the oracle.
`timescale 1ns/1ps
module tb_secded;
  reg [63:0] din, edout; reg [71:0] mask; reg ecorr, edbl;
  integer fd, r, errs = 0, n = 0;
  wire [71:0] cw; wire [63:0] dout; wire corrected, double_err;
  secded_enc enc(.din(din), .cw(cw));
  wire [71:0] cw_corrupt = cw ^ mask;
  secded_dec dec(.cw_in(cw_corrupt), .dout(dout), .corrected(corrected), .double_err(double_err));
  initial begin
    fd = $fopen("secded_vec.hex", "r");
    while (!$feof(fd)) begin
      r = $fscanf(fd, "%h %h %h %b %b\n", din, mask, edout, ecorr, edbl);
      if (r == 5) begin
        #1;
        if (double_err !== edbl) begin errs=errs+1; $display("FAIL row %0d: double_err %b!=%b", n, double_err, edbl); end
        else if (!edbl && dout !== edout) begin errs=errs+1; $display("FAIL row %0d: data %h!=%h", n, dout, edout); end
        n = n + 1;
      end
    end
    if (errs == 0) $display("PASS: %0d SECDED fault cases match the oracle (SEC + DED)", n);
    $finish;
  end
endmodule
