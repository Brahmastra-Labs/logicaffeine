// tb_hft_uart — drive a full LOGOS program INTO nexys_soc_hft over simulated UART exactly as the FT2232
// (and scripts/nexys-host.py --words16) would, using the 2-byte length field, run it, and print the Show
// values + on-chip run cycles. Proves the whole tick-to-trade pipeline (>255 words) loads and executes
// over the wire — the real "python over the port" path, not just a direct-imem load.
//
//   iverilog -g2012 -s tb -o /tmp/tb_hft_uart.vvp nexys_soc_hft.v soft_core.v alu.v shared_divider.v \
//            crdt_merge.v bigint_alu.v bigint_div.v fpu.v tb_hft_uart.v
//   vvp /tmp/tb_hft_uart.vvp +HEX=/tmp/hft_pipeline.hex +N=331
//     -> RESULT: <shows...>  CYCLES: <n>
`timescale 1ns/1ps
module tb;
  localparam integer CPB = 8;
  reg clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx;
  wire [15:0] led;

  nexys_soc_hft #(.CLKS_PER_BIT(CPB)) dut (
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

  reg  [31:0] mem [0:1023];
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
    for (j = 0; j < 1024; j = j + 1) mem[j] = 32'h2B000000;
    $readmemh(hexfile, mem);

    rst = 1'b1; repeat (4) @(posedge clk);
    rst = 1'b0; repeat (4) @(posedge clk);

    send_byte(nwords[15:8]);                                   // length hi (2-byte big-endian)
    send_byte(nwords[7:0]);                                    // length lo
    for (j = 0; j < nwords; j = j + 1) begin
      send_byte(mem[j][31:24]); send_byte(mem[j][23:16]);
      send_byte(mem[j][15:8]);  send_byte(mem[j][7:0]);
    end

    recv_byte(got);
    if (got !== 8'hA5) begin $display("FAIL: bad sync %02h", got); $finish; end
    recv_byte(rcount[7:0]);
    for (j = 0; j < rcount; j = j + 1) begin
      recv_byte(got);                                          // tag (0=int) — the pipeline shows ints only
      recv_byte(rv[j][31:24]); recv_byte(rv[j][23:16]);
      recv_byte(rv[j][15:8]);  recv_byte(rv[j][7:0]);
    end
    recv_byte(rcycles[31:24]); recv_byte(rcycles[23:16]);
    recv_byte(rcycles[15:8]);  recv_byte(rcycles[7:0]);
    recv_byte(got);
    if (got !== 8'h5A) begin $display("FAIL: bad end %02h", got); $finish; end

    $write("RESULT:");
    for (j = 0; j < rcount; j = j + 1) $write(" %0d", rv[j]);
    $display("  CYCLES: %0d", rcycles);
    $finish;
  end

  initial begin #500000000; $display("FAIL: timeout"); $finish; end
endmodule
