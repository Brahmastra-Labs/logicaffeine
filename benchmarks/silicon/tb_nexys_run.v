// tb_nexys_run — the SIMULATION TWIN of scripts/nexys-host.py. Loads a real .hex bytecode program
// (the same file the host streams to the board), drives it INTO nexys_soc over simulated UART exactly
// as the FT2232 would, runs it on the full 113-op soft-core, and prints the Show values + on-chip run
// cycles that come back. Lets us prove any program on the actual RTL before burning a bitstream.
//
//   iverilog -g2012 -s tb -o /tmp/run.vvp nexys_soc.v soft_core.v alu.v shared_divider.v \
//            crdt_merge.v bigint_alu.v bigint_div.v tb_nexys_run.v
//   vvp /tmp/run.vvp +HEX=programs/sum_loop.hex +N=11
//     -> RESULT: 15  CYCLES: 34
`timescale 1ns/1ps
module tb;
  localparam integer CPB = 8;                 // CLKS_PER_BIT (small, for fast sim)
  reg clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx;
  wire [15:0] led;

`ifndef SOC_MODULE
`define SOC_MODULE nexys_soc
`endif
  `SOC_MODULE #(.CLKS_PER_BIT(CPB)) dut (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );
  always #5 clk = ~clk;                        // 100 MHz

  task send_byte(input [7:0] b);
    integer i;
    begin
      uart_rx = 1'b0; repeat (CPB) @(posedge clk);              // start bit
      for (i = 0; i < 8; i = i + 1) begin
        uart_rx = b[i]; repeat (CPB) @(posedge clk);            // LSB first
      end
      uart_rx = 1'b1; repeat (CPB) @(posedge clk);              // stop bit
    end
  endtask

  task recv_byte(output [7:0] b);
    integer i;
    begin
      @(negedge uart_tx);                                       // start-bit edge
      repeat (CPB + CPB/2) @(posedge clk);                      // to the middle of bit 0
      for (i = 0; i < 8; i = i + 1) begin
        b[i] = uart_tx; repeat (CPB) @(posedge clk);            // LSB first
      end
      @(posedge clk);                                           // settle into the stop bit
    end
  endtask

  reg  [31:0] mem [0:63];                       // program words ($readmemh honours // comments)
  integer     nwords;
  reg  [1023:0] hexfile;
  reg  [31:0] rv [0:31];
  reg  [5:0]  rcount;
  reg  [31:0] rcycles;
  reg  [7:0]  got;
  integer     j;

  initial begin
    if (!$value$plusargs("HEX=%s", hexfile)) begin $display("FAIL: need +HEX=<file>"); $finish; end
    if (!$value$plusargs("N=%d", nwords))    begin $display("FAIL: need +N=<nwords>"); $finish; end
    $readmemh(hexfile, mem);

    rst = 1'b1; repeat (4) @(posedge clk);
    rst = 1'b0; repeat (4) @(posedge clk);

    send_byte(nwords[7:0]);                                     // length in words
    for (j = 0; j < nwords; j = j + 1) begin                   // MSB-first per word
      send_byte(mem[j][31:24]); send_byte(mem[j][23:16]);
      send_byte(mem[j][15:8]);  send_byte(mem[j][7:0]);
    end

    recv_byte(got);                                            // 0xA5 sync
    if (got !== 8'hA5) begin $display("FAIL: bad sync %02h", got); $finish; end
    recv_byte(rcount[7:0]);
    for (j = 0; j < rcount; j = j + 1) begin
      recv_byte(rv[j][31:24]); recv_byte(rv[j][23:16]);
      recv_byte(rv[j][15:8]);  recv_byte(rv[j][7:0]);
    end
    recv_byte(rcycles[31:24]); recv_byte(rcycles[23:16]);
    recv_byte(rcycles[15:8]);  recv_byte(rcycles[7:0]);
    recv_byte(got);                                            // 0x5A end
    if (got !== 8'h5A) begin $display("FAIL: bad end %02h", got); $finish; end

    $write("RESULT:");
    for (j = 0; j < rcount; j = j + 1) $write(" %0d", rv[j]);
    $display("  CYCLES: %0d", rcycles);
    $finish;
  end

  initial begin #20000000; $display("FAIL: timeout"); $finish; end   // watchdog
endmodule
