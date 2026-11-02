// tb_coproc_soc — drive coproc_soc, decode its UART TX stream, and check the four result
// registers against the software model (run_core). PASS iff every streamed byte matches.
`timescale 1ns/1ps
module tb_coproc_soc;
  localparam CPB  = 8;    // CLKS_PER_BIT for fast simulation
  localparam CLKP = 10;   // clock period (ns)

  reg  clk = 0, rst = 1;
  wire uart_tx;
  wire [7:0] led;
  coproc_soc #(.CLKS_PER_BIT(CPB)) dut(.clk(clk), .rst(rst), .uart_tx(uart_tx), .led(led));
  always #(CLKP/2) clk = ~clk;

  reg [255:0] exp_r [2:5];
  reg [7:0]   got [0:128];
  reg [255:0] tmp;
  reg [7:0]   exp_byte, rb;
  integer i, g, p, errors;

  // Receive one 8N1 byte (LSB first) from the TX line by mid-bit sampling.
  task uart_rx_byte(output [7:0] data);
    integer k;
    begin
      @(negedge uart_tx);                       // start-bit edge
      #(CPB * CLKP + (CPB * CLKP) / 2);         // advance to the middle of data bit 0
      for (k = 0; k < 8; k = k + 1) begin
        data[k] = uart_tx;
        #(CPB * CLKP);
      end
    end
  endtask

  initial begin
    // the four retired result registers, {limb3,limb2,limb1,limb0}, from run_core(COPROC_A,B):
    exp_r[2] = 256'h0000000000000004_0000000000000003_1111111111111113_0000000000000000; // BADD
    exp_r[3] = 256'h0000000000000002_0000000000000001_1111111111111110_fffffffffffffffe; // BSUB
    exp_r[4] = 256'h0000000000000000_0000000000000000_0000000000000000_ffffffffffffffff; // BMUL
    exp_r[5] = 256'h0000000000000000_0000000000000000_1111111111111111_ffffffffffffffff; // CMERGE
    #(3 * CLKP) rst = 0;
    errors = 0;
    for (i = 0; i <= 128; i = i + 1) begin
      uart_rx_byte(rb);
      got[i] = rb;
    end

    if (got[0] !== 8'hA5) begin errors = errors + 1; $display("FAIL sync byte: %h", got[0]); end
    for (i = 0; i < 128; i = i + 1) begin
      g = i / 32; p = i % 32;
      tmp = exp_r[2 + g];
      exp_byte = tmp[(31 - p) * 8 +: 8];
      if (got[1 + i] !== exp_byte) begin
        errors = errors + 1;
        $display("FAIL byte %0d (R%0d MSB-pos %0d): got %h exp %h", i, 2 + g, p, got[1 + i], exp_byte);
      end
    end

    if (errors == 0)
      $display("PASS: coproc_soc streamed R2..R5 over UART, matching run_core (sync + 128 bytes)");
    else
      $display("FAIL: %0d UART byte mismatches", errors);
    $finish;
  end
endmodule
