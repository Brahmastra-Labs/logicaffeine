// tb_nexys_soc — drive real bytecode programs INTO the board SoC over UART, let the full
// 113-op soft-core run each, and check the Show values that come back over UART. This is the
// end-to-end board round-trip in simulation: reset -> load-over-UART -> run -> read-back-over-UART,
// exercised by three distinct programs (constants, arithmetic through the ALU, and a multi-Show
// stream that fills the capture FIFO).
`timescale 1ns/1ps
module tb;
  localparam integer CPB = 8;               // CLKS_PER_BIT (small, for fast sim)

  reg clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx;
  wire [15:0] led;
  integer errors = 0;

  nexys_soc #(.CLKS_PER_BIT(CPB)) dut (
    .clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led)
  );

  always #5 clk = ~clk;                      // 100 MHz

  // ── UART bit-banging (8N1, LSB-first) ────────────────────────────────────────────────
  task send_byte(input [7:0] b);
    integer i;
    begin
      uart_rx = 1'b0;                                  // start bit
      repeat (CPB) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin
        uart_rx = b[i]; repeat (CPB) @(posedge clk);   // LSB first
      end
      uart_rx = 1'b1; repeat (CPB) @(posedge clk);     // stop bit
    end
  endtask

  task recv_byte(output [7:0] b);
    integer i;
    begin
      @(negedge uart_tx);                              // start bit edge
      repeat (CPB + CPB/2) @(posedge clk);             // to the middle of bit 0
      for (i = 0; i < 8; i = i + 1) begin
        b[i] = uart_tx; repeat (CPB) @(posedge clk);   // LSB first
      end
      @(posedge clk);                                  // settle into the stop bit
    end
  endtask

  // ── the program under test + its captured results (shared with run_program) ──────────
  reg  [7:0]  prog [0:255];
  integer     plen;                          // program length in BYTES (multiple of 4)
  reg  [31:0] rv   [0:31];                   // Show values read back
  reg  [5:0]  rcount;                        // number read back
  reg  [31:0] rcycles;                       // on-chip run cycle count read back
  integer     k;
  reg  [7:0]  got;

  // Reset, stream the program in over UART, run it, and read back the result frame.
  task run_program;
    integer j;
    begin
      rst = 1'b1; repeat (4) @(posedge clk);
      rst = 1'b0; repeat (4) @(posedge clk);

      send_byte(plen/4);              // length in WORDS
      for (j = 0; j < plen; j = j + 1) send_byte(prog[j]);

      recv_byte(got); chk(got, 8'hA5, "sync");
      recv_byte(rcount[7:0]);
      for (j = 0; j < rcount; j = j + 1) begin
        recv_byte(rv[j][31:24]); recv_byte(rv[j][23:16]);
        recv_byte(rv[j][15:8]);  recv_byte(rv[j][7:0]);
      end
      recv_byte(rcycles[31:24]); recv_byte(rcycles[23:16]);   // on-chip run-cycle count, MSB-first
      recv_byte(rcycles[15:8]);  recv_byte(rcycles[7:0]);
      recv_byte(got); chk(got, 8'h5A, "end");
    end
  endtask

  task chk(input [7:0] a, input [7:0] b, input [127:0] name);
    if (a !== b) begin errors=errors+1; $display("FAIL: %0s = %02h expected %02h", name, a, b); end
  endtask
  task chkv(input [31:0] a, input [31:0] b, input [127:0] name);
    if (a !== b) begin errors=errors+1; $display("FAIL: %0s = %0d expected %0d", name, a, b); end
  endtask

  // word helper: prog[base..base+3] = [op][d][a][b]
  task w(input integer base, input [7:0] op, input [7:0] d, input [7:0] a, input [7:0] b);
    begin prog[base]=op; prog[base+1]=d; prog[base+2]=a; prog[base+3]=b; end
  endtask

  initial begin
    // ── Program 1: two constants shown ── expect [7, 42]
    w(0,  8'h05, 8'h00, 8'h00, 8'h07);       // LoadConst r0, 7
    w(4,  8'h05, 8'h01, 8'h00, 8'h2A);       // LoadConst r1, 42
    w(8,  8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(12, 8'h59, 8'h00, 8'h01, 8'h00);       // Show r1
    w(16, 8'h2B, 8'h00, 8'h00, 8'h00);       // Halt
    plen = 20; run_program;
    chk(rcount, 8'd2, "prog1 count");
    chkv(rv[0], 32'd7,  "prog1 show0");
    chkv(rv[1], 32'd42, "prog1 show1");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: prog1 cycles==0"); end
    $display("prog1 cycles = %0d", rcycles);

    // ── Program 2: arithmetic through the real ALU ── 10 + 32 = 42, expect [42]
    w(0,  8'h05, 8'h00, 8'h00, 8'h0A);       // LoadConst r0, 10
    w(4,  8'h05, 8'h01, 8'h00, 8'h20);       // LoadConst r1, 32
    w(8,  8'h08, 8'h02, 8'h00, 8'h01);       // Add r2 = r0 + r1
    w(12, 8'h59, 8'h00, 8'h02, 8'h00);       // Show r2
    w(16, 8'h2B, 8'h00, 8'h00, 8'h00);       // Halt
    plen = 20; run_program;
    chk(rcount, 8'd1, "prog2 count");
    chkv(rv[0], 32'd42, "prog2 add");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: prog2 cycles==0"); end
    $display("prog2 cycles = %0d", rcycles);

    // ── Program 3: three Shows through the capture FIFO ── expect [1, 2, 3]
    w(0,  8'h05, 8'h00, 8'h00, 8'h01);       // LoadConst r0, 1
    w(4,  8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(8,  8'h05, 8'h00, 8'h00, 8'h02);       // LoadConst r0, 2
    w(12, 8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(16, 8'h05, 8'h00, 8'h00, 8'h03);       // LoadConst r0, 3
    w(20, 8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(24, 8'h2B, 8'h00, 8'h00, 8'h00);       // Halt
    plen = 28; run_program;
    chk(rcount, 8'd3, "prog3 count");
    chkv(rv[0], 32'd1, "prog3 show0");
    chkv(rv[1], 32'd2, "prog3 show1");
    chkv(rv[2], 32'd3, "prog3 show2");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: prog3 cycles==0"); end
    $display("prog3 cycles = %0d", rcycles);

    // ── Program 4: sum 1..5 = 15 — a REAL loop (LtEq / JumpIfFalse / Add / Jump), byte-identical to
    //    differential (1) in rtl.rs which is proven run_soft_core ≡ VM for `While i is at most 5`. ── [15]
    w(0,  8'h05, 8'h00, 8'h00, 8'h00);       // LoadConst r0, 0   (sum)
    w(4,  8'h05, 8'h01, 8'h00, 8'h01);       // LoadConst r1, 1   (i)
    w(8,  8'h05, 8'h02, 8'h00, 8'h05);       // LoadConst r2, 5   (n)
    w(12, 8'h05, 8'h03, 8'h00, 8'h01);       // LoadConst r3, 1   (step)
    w(16, 8'h1B, 8'h04, 8'h01, 8'h02);       // LtEq r4 = (i <= n)
    w(20, 8'h23, 8'h09, 8'h04, 8'h00);       // JumpIfFalse r4 -> word 9 (Show)
    w(24, 8'h08, 8'h00, 8'h00, 8'h01);       // Add r0 = sum + i
    w(28, 8'h08, 8'h01, 8'h01, 8'h03);       // Add r1 = i + 1
    w(32, 8'h22, 8'h04, 8'h00, 8'h00);       // Jump -> word 4 (loop test)
    w(36, 8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(40, 8'h2B, 8'h00, 8'h00, 8'h00);       // Halt
    plen = 44; run_program;
    chk(rcount, 8'd1, "prog4 count");
    chkv(rv[0], 32'd15, "prog4 sum1..5");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: prog4 cycles==0"); end
    $display("prog4 cycles = %0d (sum 1..5 = 15, a real loop on the board SoC)", rcycles);

    if (errors == 0)
      $display("tb_nexys_soc: PASS - 4 programs loaded over UART, run on the full core, results read back correct (incl. a real loop + on-chip cycle counts)");
    else
      $display("tb_nexys_soc: FAIL - %0d error(s)", errors);
    $finish;
  end

  // Watchdog so a wiring bug cannot hang the run.
  initial begin
    #20000000;
    $display("tb_nexys_soc: FAIL - timeout (no complete readback)");
    $finish;
  end
endmodule
