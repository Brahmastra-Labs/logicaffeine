`timescale 1ns/1ps
// tb_sevenseg_dec — drive the signed-decimal 7-seg display with several values, decode the segment patterns
// back to characters (reading each digit as its anode goes active), and check the rendered string. Proves
// the display LIGHTS UP with the correct number (matching what the laptop prints), incl. minus + zero.
module tb;
  reg clk = 0, rst; reg [31:0] val; wire [6:0] seg; wire dp; wire [7:0] an;
  sevenseg_dec #(.REFRESH_BITS(1)) dut (.clk(clk), .rst(rst), .val(val), .seg(seg), .dp(dp), .an(an));
  always #5 clk = ~clk;

  function [7:0] dc; input [6:0] s;
    case (s)
      7'b1000000: dc="0"; 7'b1111001: dc="1"; 7'b0100100: dc="2"; 7'b0110000: dc="3"; 7'b0011001: dc="4";
      7'b0010010: dc="5"; 7'b0000010: dc="6"; 7'b1111000: dc="7"; 7'b0000000: dc="8"; 7'b0010000: dc="9";
      7'b0111111: dc="-"; 7'b1111111: dc=" "; default: dc="?";
    endcase
  endfunction

  integer d, t, k, lit, pass;
  reg [7:0] ch [0:7];
  reg [8*12:1] s;

  task show(input [31:0] v, input [8*12:1] want);
    begin
      val = v; rst = 1'b1; repeat (4) @(posedge clk); rst = 1'b0;
      repeat (80) @(posedge clk);   // let the sequential double-dabble converge
      // scan all 8 digits: wait for each anode to go active, record the segment→char and whether it's lit.
      lit = 0;
      for (d = 0; d < 8; d = d + 1) begin
        t = 0; while (an !== ~(8'd1 << d) && t < 100000) begin @(posedge clk); t = t + 1; end
        ch[d] = dc(seg);
        if (seg !== 7'b1111111) lit = lit + 1;
      end
      // assemble MSB→LSB, dropping leading blanks
      s = 0; k = 0;
      for (d = 7; d >= 0; d = d - 1)
        if (!(k == 0 && ch[d] == " ")) begin s = (s << 8) | ch[d]; k = k + 1; end
      $write("  val=%0d -> \"", $signed(v));
      for (d = k-1; d >= 0; d = d - 1) $write("%s", (s >> (8*d)) & 8'hFF);
      $display("\"  (lit digits: %0d)  want \"%0s\"", lit, want);
      if (s !== want) begin $display("  FAIL: rendered != expected"); pass = 0; end
      if (lit == 0)   begin $display("  FAIL: display is DARK"); pass = 0; end
    end
  endtask

  initial begin
    pass = 1;
    show(32'd0,               "0");
    show(32'd50,              "50");
    show(-32'sd50,            "-50");
    show(32'd5074,            "5074");
    show(32'd32258,           "32258");
    show(-32'sd40,            "-40");
    show(32'd30000,           "30000");
    if (pass) $display("PASS: sevenseg_dec renders signed decimal, always lit");
    else      $display("FAIL: sevenseg_dec");
    $finish;
  end
endmodule
