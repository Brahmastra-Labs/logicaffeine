// tb_sevenseg — proves the 7-segment driver: the hex nibble->segment table for all 16 values, the
// one-hot-low anode cycling across all 8 digits, and blank-on-reset. REFRESH_BITS is shrunk so the mux
// cycles in a few clocks.
`timescale 1ns/1ps
module tb_sevenseg;
  reg clk = 0, rst, blank;
  reg [31:0] val;
  wire [6:0] seg;
  wire dp;
  wire [7:0] an;
  integer errors = 0, d, k;
  reg [6:0] expect_seg [0:15];
  reg [3:0] nib;
  reg [7:0] seen_an;

  sevenseg #(.REFRESH_BITS(2)) dut (.clk(clk), .rst(rst), .val(val), .blank(blank),
                                    .seg(seg), .dp(dp), .an(an));
  always #5 clk = ~clk;

  initial begin
    // the golden active-low table {g,f,e,d,c,b,a}
    expect_seg[0]=7'b1000000; expect_seg[1]=7'b1111001; expect_seg[2]=7'b0100100; expect_seg[3]=7'b0110000;
    expect_seg[4]=7'b0011001; expect_seg[5]=7'b0010010; expect_seg[6]=7'b0000010; expect_seg[7]=7'b1111000;
    expect_seg[8]=7'b0000000; expect_seg[9]=7'b0010000; expect_seg[10]=7'b0001000; expect_seg[11]=7'b0000011;
    expect_seg[12]=7'b1000110; expect_seg[13]=7'b0100001; expect_seg[14]=7'b0000110; expect_seg[15]=7'b0001110;

    // reset -> blank (all dark)
    rst = 1; blank = 0; val = 32'h0; @(negedge clk); @(negedge clk);
    if (seg !== 7'b1111111 || an !== 8'hFF) begin errors=errors+1; $display("FAIL: reset not blank seg=%b an=%b", seg, an); end
    rst = 0;

    // val = 0xFEDCBA98: digit d shows nibble d (0x98BA... low nibble first). Walk >=8 full digit dwells,
    // check that each active digit (anode low) shows the correct nibble's segments, and that all 8 anodes
    // are exercised one-hot-low.
    val = 32'hFEDCBA98;   // nibbles: d0=8,d1=9,d2=A,d3=B,d4=C,d5=D,d6=E,d7=F
    seen_an = 8'h00;
    for (k = 0; k < 400; k = k + 1) begin
      @(negedge clk);
      if (an !== 8'hFF) begin
        // find the one active (low) anode
        d = -1;
        if (an == 8'b11111110) d=0; else if (an == 8'b11111101) d=1;
        else if (an == 8'b11111011) d=2; else if (an == 8'b11110111) d=3;
        else if (an == 8'b11101111) d=4; else if (an == 8'b11011111) d=5;
        else if (an == 8'b10111111) d=6; else if (an == 8'b01111111) d=7;
        if (d < 0) begin errors=errors+1; $display("FAIL: anode not one-hot-low: %b", an); end
        else begin
          seen_an[d] = 1'b1;
          nib = val[4*d +: 4];
          if (seg !== expect_seg[nib]) begin
            errors=errors+1;
            $display("FAIL: digit %0d nib %h seg=%b expected %b", d, nib, seg, expect_seg[nib]);
          end
        end
      end
    end
    for (d = 0; d < 8; d = d + 1)
      if (!seen_an[d]) begin errors=errors+1; $display("FAIL: anode %0d never activated", d); end

    // independently sweep every nibble value 0..15 through digit 0
    for (k = 0; k < 16; k = k + 1) begin
      val = k; // low nibble = k
      // wait until digit 0 is active
      @(negedge clk);
      while (an !== 8'b11111110) @(negedge clk);
      if (seg !== expect_seg[k]) begin errors=errors+1; $display("FAIL: nib %h seg=%b expected %b", k, seg, expect_seg[k]); end
    end

    // blank forces dark even with a value present
    val = 32'h12345678; blank = 1; @(negedge clk); @(negedge clk);
    if (seg !== 7'b1111111 || an !== 8'hFF) begin errors=errors+1; $display("FAIL: blank not dark seg=%b an=%b", seg, an); end

    if (errors == 0) $display("PASS: sevenseg — 16 hex patterns, 8-digit one-hot anode cycling, blank-on-reset");
    else $display("FAIL: %0d sevenseg errors", errors);
    $finish;
  end
endmodule
