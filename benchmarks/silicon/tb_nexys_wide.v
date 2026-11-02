`timescale 1ns/1ps
// tb_nexys_wide — proves the 2-byte-length ESCAPE loader (P0d): stream a >255-word program into the full
// SoC over UART using the `0xFF, len_hi, len_lo` escape, run it, and check the Show readback == the VM.
// The legacy 1-byte path (programs <=254 words) is regression-covered by tb_nexys_soc_full.v.
module tb;
  localparam CPB = 4;                          // clocks-per-UART-bit (fast, for sim)
  reg  clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx; wire [15:0] led;
  integer errors = 0;

  nexys_soc_full #(.CLKS_PER_BIT(CPB)) dut (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
  always #5 clk = ~clk;

  task send_byte(input [7:0] b);
    integer i;
    begin
      uart_rx = 1'b0; repeat (CPB) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin uart_rx = b[i]; repeat (CPB) @(posedge clk); end
      uart_rx = 1'b1; repeat (CPB) @(posedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    integer i;
    begin
      @(negedge uart_tx);
      repeat (CPB + CPB/2) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin b[i] = uart_tx; repeat (CPB) @(posedge clk); end
      @(posedge clk);
    end
  endtask

  reg  [31:0] words [0:1023];
  integer     nwords, j, bp;
  reg  [7:0]  got, tagb;
  reg  [5:0]  rc;
  reg  [31:0] val, rcyc;

  initial begin
    for (j = 0; j < 1024; j = j + 1) words[j] = 32'hxxxxxxxx;
    $readmemh("/tmp/fpga/e.hex", words);
    nwords = 0;
    for (j = 0; j < 1024; j = j + 1) if ((^words[j]) !== 1'bx) nwords = j + 1;

    rst = 1'b1; repeat (4) @(posedge clk);
    rst = 1'b0; repeat (4) @(posedge clk);

    // 2-BYTE ESCAPE: 0xFF then the 16-bit word count MSB-first.
    send_byte(8'hFF);
    send_byte(nwords[15:8]);
    send_byte(nwords[7:0]);
    for (j = 0; j < nwords; j = j + 1) begin
      send_byte(words[j][31:24]); send_byte(words[j][23:16]);
      send_byte(words[j][15:8]);  send_byte(words[j][7:0]);
    end

    // Readback frame: 0xA5 sync, count, then per-show [tag + value], then 4 cycle bytes, then 0x5A.
    recv_byte(got); if (got !== 8'hA5) begin errors = errors + 1; $display("FAIL: sync=%02h", got); end
    recv_byte(rc);
    if (rc !== 6'd1) begin errors = errors + 1; $display("FAIL: rcount=%0d expected 1", rc); end
    recv_byte(tagb);
    val = 32'd0;
    for (bp = 0; bp < 4; bp = bp + 1) begin recv_byte(got); val = {val[23:0], got}; end
    recv_byte(rcyc[31:24]); recv_byte(rcyc[23:16]); recv_byte(rcyc[15:8]); recv_byte(rcyc[7:0]);
    recv_byte(got); if (got !== 8'h5A) begin errors = errors + 1; $display("FAIL: end=%02h", got); end

    if (val === 32'd8001 && errors == 0)
      $display("tb_nexys_wide: PASS - a %0d-word program (>255) loaded via the 2-byte escape; Show=%0d, on-chip cycles=%0d",
               nwords, val, rcyc);
    else
      $display("tb_nexys_wide: FAIL - Show=%0d expected 8001, nwords=%0d, errors=%0d", val, nwords, errors);
    $finish;
  end
endmodule
