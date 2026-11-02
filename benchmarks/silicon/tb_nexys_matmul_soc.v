`timescale 1ns/1ps
// A sim-only stub for the Xilinx BUFG global-buffer primitive (yosys synth_xilinx uses the real one).
module BUFG(input I, output O); assign O = I; endmodule
// tb_nexys_matmul_soc — stream TWO int8 vectors INTERLEAVED into the matmul SoC over a (fast) UART, let the
// LOGOS-emitted int8 DSP dot-product array compute a·b, and check the captured result + cycle count.
// CLKS_PER_BIT is shrunk to 8 so the sim is quick; coreclk = clk/4 = 25 MHz → 40 ns; one UART bit = 320 ns.
module tb;
  reg clk = 0, rst; reg uart_rx; wire uart_tx; wire [15:0] led;
  wire [6:0] seg; wire dp; wire [7:0] an;
  localparam CPB = 8;
  localparam integer BITT = CPB * 40; // ns per UART bit at coreclk=25MHz
  nexys_matmul_soc #(.CLKS_PER_BIT(CPB), .REFRESH_BITS(2)) dut (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led),
    .seg(seg), .dp(dp), .an(an));
  always #5 clk = ~clk; // 100 MHz board clock

  // decode a 7-seg cathode pattern back to a character (to read the physical display in sim)
  function [7:0] dc; input [6:0] s;
    case (s)
      7'b1000000: dc="0"; 7'b1111001: dc="1"; 7'b0100100: dc="2"; 7'b0110000: dc="3"; 7'b0011001: dc="4";
      7'b0010010: dc="5"; 7'b0000010: dc="6"; 7'b1111000: dc="7"; 7'b0000000: dc="8"; 7'b0010000: dc="9";
      7'b0111111: dc="-"; 7'b1111111: dc=" "; default: dc="?";
    endcase
  endfunction

  task send_byte(input [7:0] b);
    integer i;
    begin
      uart_rx = 1'b0; #(BITT);                 // start bit
      for (i = 0; i < 8; i = i + 1) begin uart_rx = b[i]; #(BITT); end // LSB first
      uart_rx = 1'b1; #(BITT);                 // stop bit
      #(BITT);                                  // inter-byte idle
    end
  endtask

  function signed [63:0] sx(input [7:0] b); sx = $signed({{56{b[7]}}, b}); endfunction

  integer k, c, P; reg signed [63:0] exp; reg [7:0] xs [0:15]; integer L; integer pass;

  // send one interleaved a·b request, wait for the result latch, check it — WITHOUT resetting the board
  // (proves back-to-back requests on a single flash).
  task do_dot(input [7:0] a0,a1,a2,a3, input [7:0] b0,b1,b2,b3, input integer np, input signed [63:0] want);
    begin
      xs[0]=a0; xs[1]=b0; xs[2]=a1; xs[3]=b1; xs[4]=a2; xs[5]=b2; xs[6]=a3; xs[7]=b3;
      L = 2*np;
      send_byte(L[7:0]);
      for (k=0;k<L;k=k+1) send_byte(xs[k]);
      c = 0;
      while (dut.cs != 3'd3 && c < 200000) begin @(posedge clk); c = c + 1; end   // wait until S_EMIT captured
      while (dut.cs == 3'd3 && c < 400000) begin @(posedge clk); c = c + 1; end   // wait until it returns to idle
      $display("RESULT: %0d CYCLES=%0d EXPECT=%0d", $signed(dut.result_lat), dut.cyc_lat, want);
      if ($signed(dut.result_lat) === want && dut.cyc_lat == L)
        $display("  OK a.b = %0d in %0d cycles", want, dut.cyc_lat);
      else begin $display("  FAIL got %0d cyc %0d", $signed(dut.result_lat), dut.cyc_lat); pass = 0; end
    end
  endtask

  initial begin
    pass = 1;
    uart_rx = 1'b1; rst = 1'b1; #200; rst = 1'b0; #2000;
    // THREE back-to-back requests on ONE reset — the loop-to-idle FSM in action.
    do_dot(8'sd3, -8'sd4, 8'sd5, 8'sd0,  8'sd6, 8'sd7, -8'sd8, 8'sd0,  3, -64'sd50);
    do_dot(8'sd10,-8'sd7, 8'sd25,-8'sd3, 8'sd100,-8'sd128,8'sd127,-8'sd1, 4, 64'sd5074);
    do_dot(8'sd127,8'sd127,8'sd0,8'sd0,  8'sd127,8'sd127,8'sd0,8'sd0,  2, 64'sd32258);

    // ── the 7-SEG DISPLAY must light up with the last result (32258), read straight off the cathodes ──
    begin : disp_check
      integer d, t, lit; reg [7:0] ch [0:7]; reg [8*12:1] s; integer kk;
      repeat (120) @(posedge clk);   // let the display's sequential double-dabble converge on the last result
      lit = 0;
      for (d = 0; d < 8; d = d + 1) begin
        t = 0; while (an !== ~(8'd1 << d) && t < 200000) begin @(posedge clk); t = t + 1; end
        ch[d] = dc(seg);
        if (seg !== 7'b1111111) lit = lit + 1;
      end
      s = 0; kk = 0;
      for (d = 7; d >= 0; d = d - 1) if (!(kk==0 && ch[d]==" ")) begin s = (s<<8)|ch[d]; kk = kk+1; end
      $display("DISPLAY shows \"%0s\" (%0d digits lit)  want \"32258\"", s, lit);
      if (s !== "32258") begin $display("  FAIL: display != result"); pass = 0; end
      if (lit == 0)      begin $display("  FAIL: display DARK");      pass = 0; end
    end

    if (pass) $display("PASS: matmul SoC — 3 back-to-back a.b over UART + the 7-seg lights up with the result");
    else      $display("FAIL: matmul SoC");
    $finish;
  end
endmodule
