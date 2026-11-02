// tb_nexys_tformer — end-to-end board round-trip for the DATA-segment weight loader on the real full-core
// RTL: reset -> load PROGRAM over UART -> stream a DATA SEGMENT (weights) into the heap over UART -> run ->
// read back the Show over UART, and check `disp_val` (the 7-seg feed) latched the last Show. The program is
// a compiled `loaded()` reader ($readmemh +HEX); the DATA blob is a `Seq of Int` at ptr 0 ($readmemh +DATA).
// This validates the NEW RTL (the DATA phase + core data-preload port + hp_init) on the actual gates; the
// whole transformer's correctness is locked separately on the soft-core model (tiny_transformer_lock).
`timescale 1ns/1ps
module tb;
  localparam integer CPB = 8;               // CLKS_PER_BIT (small, for fast sim)
  reg clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx;
  wire [15:0] led;
  wire [31:0] disp_val;
  integer errors = 0, i, j;

  nexys_soc_tformer #(.CLKS_PER_BIT(CPB)) dut (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led), .disp_val(disp_val)
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

  reg  [31:0] prog [0:1023];                 // program words ($readmemh +HEX)
  reg  [31:0] data [0:65535];                // DATA-segment words ($readmemh +DATA) — the weight blob
  integer     nprog, ndata, expect_show;
  reg  [7:0]  got;
  reg  [31:0] rv;
  reg  [5:0]  rcount;

  initial begin
    if (!$value$plusargs("N=%d", nprog))   nprog = 0;
    if (!$value$plusargs("ND=%d", ndata))  ndata = 0;
    if (!$value$plusargs("EXP=%d", expect_show)) expect_show = 0;
    $readmemh("tformer_prog.hex", prog);
    $readmemh("tformer_data.hex", data);

    repeat (20) @(posedge clk); rst = 0; repeat (20) @(posedge clk);

    // ── PROGRAM phase: 1-byte word count (nprog < 255) then nprog words, 4 bytes each MSB-first ──
    send_byte(nprog[7:0]);
    for (i = 0; i < nprog; i = i + 1) begin
      send_byte(prog[i][31:24]); send_byte(prog[i][23:16]); send_byte(prog[i][15:8]); send_byte(prog[i][7:0]);
    end
    // ── DATA phase: 2-byte word count (MSB-first) then ndata words, 4 bytes each MSB-first ──
    send_byte(ndata[15:8]); send_byte(ndata[7:0]);
    for (i = 0; i < ndata; i = i + 1) begin
      send_byte(data[i][31:24]); send_byte(data[i][23:16]); send_byte(data[i][15:8]); send_byte(data[i][7:0]);
    end

    // ── read back the reply frame: 0xA5, count, per-int-record (tag + 4 bytes), cycles, 0x5A ──
    recv_byte(got);
    if (got !== 8'hA5) begin errors=errors+1; $display("FAIL: sync = %02h expected A5", got); end
    recv_byte(got); rcount = got[5:0];
    rv = 0;
    for (j = 0; j < rcount; j = j + 1) begin
      recv_byte(got);                                          // tag (0 = int)
      recv_byte(rv[31:24]); recv_byte(rv[23:16]); recv_byte(rv[15:8]); recv_byte(rv[7:0]);
    end
    // cycles (4B) + end byte
    recv_byte(got); recv_byte(got); recv_byte(got); recv_byte(got);
    recv_byte(got);
    if (got !== 8'h5A) begin errors=errors+1; $display("FAIL: end = %02h expected 5A", got); end

    // the LAST int Show must equal the expected value, and disp_val must have latched it
    if (rv !== expect_show) begin errors=errors+1; $display("FAIL: show = %0d expected %0d", rv, expect_show); end
    if (disp_val !== expect_show) begin errors=errors+1; $display("FAIL: disp_val = %0d expected %0d", disp_val, expect_show); end

    if (errors == 0)
      $display("tb_nexys_tformer: PASS - program + %0d-word DATA segment loaded over UART, loaded() read the weights on the full core, show=%0d, disp_val=%0d", ndata, rv, disp_val);
    else
      $display("tb_nexys_tformer: FAIL - %0d error(s)", errors);
    $finish;
  end

  initial begin
    repeat (5_000_000) @(posedge clk);
    $display("tb_nexys_tformer: FAIL - timeout"); $finish;
  end
endmodule
