// tb_nexys_soc_full — drive real bytecode INTO the FULL board SoC (nexys_soc_full, the FPU-bearing
// soft_core) over UART, run it, and check the Show stream that comes back over the TAGGED protocol.
// This is the end-to-end board round-trip in simulation for the FLOAT-Show extension: reset ->
// load-over-UART -> run -> read-back-over-UART, where a program interleaves an INT Show and a FLOAT Show
// and the host-visible byte stream carries a per-show <tag> + width so both round-trip in order.
//
// FPGA -> host framing (mirrors nexys_soc_full.v L_SEND, byte-for-byte the same as scripts/nexys-host.py):
//   0xA5, <M=nshow>, then per show <tag:1> (0=int,1=float,2=string) + 4 int bytes OR 8 float bytes OR
//   (for a string) 4 length bytes then len*4 codepoint bytes (all MSB-first), then 4 run-cycle bytes
//   (MSB-first), then 0x5A. Program D shows a string ("hi"); Program E interleaves an int Show and a string.
`timescale 1ns/1ps
module tb;
  localparam integer CPB = 8;               // CLKS_PER_BIT (small, for fast sim)

  reg clk = 0, rst = 1, uart_rx = 1;
  wire uart_tx;
  wire [15:0] led;
  integer errors = 0;

  nexys_soc_full #(.CLKS_PER_BIT(CPB)) dut (
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

  // ── the program under test + its captured results (tagged) ────────────────────────────
  reg  [7:0]  prog [0:255];
  integer     plen;                          // program length in BYTES (multiple of 4)
  reg  [1:0]  rtag [0:31];                   // per-show type tag (0=int, 1=float, 2=string)
  reg  [63:0] rraw [0:31];                   // int: value in low 32 | float: the f64 bits | string: char count
  reg  [7:0]  rsoff [0:31];                  // per-string record: START offset into rstr of its codepoints
  reg  [31:0] rstr [0:255];                  // all string codepoints, back-to-back across the records
  reg  [8:0]  rstr_wp;                       // write pointer into rstr (running across all string records)
  reg  [5:0]  rcount;                        // number of shows read back
  reg  [31:0] rcycles;                       // on-chip run cycle count read back
  integer     k, bp;
  reg  [7:0]  got, tagb;
  reg  [31:0] slen, sc;                      // decoded string length / one codepoint

  // Reset, stream the program in over UART, run it, and read back the tagged result frame.
  task run_program;
    integer j, ci;
    begin
      rst = 1'b1; repeat (4) @(posedge clk);
      rst = 1'b0; repeat (4) @(posedge clk);

      send_byte(plen/4);              // length in WORDS
      for (j = 0; j < plen; j = j + 1) send_byte(prog[j]);

      recv_byte(got); chk(got, 8'hA5, "sync");
      recv_byte(rcount[7:0]);
      rstr_wp = 9'd0;
      for (j = 0; j < rcount; j = j + 1) begin
        recv_byte(tagb); rtag[j] = tagb[1:0];
        rraw[j] = 64'd0;
        if (rtag[j] == 2'd2) begin                  // string: 4 length bytes, then len*4 codepoint bytes (MSB-first)
          slen = 32'd0;
          for (bp = 0; bp < 4; bp = bp + 1) begin recv_byte(got); slen = {slen[23:0], got}; end
          rraw[j] = {32'd0, slen};                  // record the char count
          rsoff[j] = rstr_wp[7:0];                  // where this string's codepoints start in rstr
          for (ci = 0; ci < slen; ci = ci + 1) begin
            sc = 32'd0;
            for (bp = 0; bp < 4; bp = bp + 1) begin recv_byte(got); sc = {sc[23:0], got}; end
            rstr[rstr_wp] = sc; rstr_wp = rstr_wp + 1'b1;
          end
        end else if (rtag[j] == 2'd1) begin         // float: 8 MSB-first bytes
          for (bp = 0; bp < 8; bp = bp + 1) begin
            recv_byte(got); rraw[j] = {rraw[j][55:0], got};
          end
        end else begin                              // int: 4 MSB-first bytes (into the low word)
          for (bp = 0; bp < 4; bp = bp + 1) begin
            recv_byte(got); rraw[j] = {rraw[j][55:0], got};
          end
        end
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
  task chk64(input [63:0] a, input [63:0] b, input [127:0] name);
    if (a !== b) begin errors=errors+1; $display("FAIL: %0s = %016h expected %016h", name, a, b); end
  endtask
  // Check that string record `idx` decoded to `exp` (a packed ASCII literal of `explen` chars): the char
  // count matches, and each reassembled codepoint (rstr[rsoff[idx]+k], MSB-first over the wire) equals the
  // k-th byte of `exp`. `exp` is right-justified in the 128-bit literal, so byte k counts from the top.
  task chk_str(input integer idx, input [127:0] exp, input integer explen);
    integer k; reg [7:0] want;
    begin
      if (rraw[idx][31:0] !== explen)
        begin errors=errors+1; $display("FAIL: string rec %0d len = %0d expected %0d", idx, rraw[idx][31:0], explen); end
      else for (k = 0; k < explen; k = k + 1) begin
        want = exp[8*(explen-1-k) +: 8];             // k-th character (MSB-first packing)
        if (rstr[rsoff[idx]+k] !== {24'd0, want})
          begin errors=errors+1;
            $display("FAIL: string rec %0d char %0d = %0h expected %0h", idx, k, rstr[rsoff[idx]+k], want); end
      end
    end
  endtask

  // word helper: prog[base..base+3] = [op][d][a][b]
  task w(input integer base, input [7:0] op, input [7:0] d, input [7:0] a, input [7:0] b);
    begin prog[base]=op; prog[base+1]=d; prog[base+2]=a; prog[base+3]=b; end
  endtask
  // raw 32-bit program word (for FConst's two immediate words), MSB-first bytes
  task wraw(input integer base, input [31:0] v);
    begin prog[base]=v[31:24]; prog[base+1]=v[23:16]; prog[base+2]=v[15:8]; prog[base+3]=v[7:0]; end
  endtask

  initial begin
    // ── Program A: INT sanity on the full SoC (unchanged path) ── expect [7, 42]
    w(0,  8'h05, 8'h00, 8'h00, 8'h07);       // LoadConst r0, 7
    w(4,  8'h05, 8'h01, 8'h00, 8'h2A);       // LoadConst r1, 42
    w(8,  8'h59, 8'h00, 8'h00, 8'h00);       // Show r0
    w(12, 8'h59, 8'h00, 8'h01, 8'h00);       // Show r1
    w(16, 8'h2B, 8'h00, 8'h00, 8'h00);       // Halt
    plen = 20; run_program;
    chk(rcount, 8'd2, "progA count");
    if (rtag[0] !== 2'd0 || rtag[1] !== 2'd0) begin errors=errors+1; $display("FAIL: progA tags not int"); end
    chkv(rraw[0][31:0], 32'd7,  "progA show0");
    chkv(rraw[1][31:0], 32'd42, "progA show1");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: progA cycles==0"); end
    $display("progA cycles = %0d (two int Shows)", rcycles);

    // ── Program B: a single FLOAT Show — pi_leibniz's f64 bits 0x40091FEF0A9265EB round-trip ──
    // FConst f0 = bits (op 201=0xC9): word0=[C9][dst][a][b], word1=low32, word2=high32; then ShowF f0.
    w(0,   8'hC9, 8'h00, 8'h00, 8'h00);      // FConst f0
    wraw(4,  32'h0A9265EB);                  //   low  32 bits
    wraw(8,  32'h40091FEF);                  //   high 32 bits
    w(12,  8'hD4, 8'h00, 8'h00, 8'h00);      // ShowF f0  (op 212 = 0xD4)
    w(16,  8'h2B, 8'h00, 8'h00, 8'h00);      // Halt
    plen = 20; run_program;
    chk(rcount, 8'd1, "progB count");
    if (rtag[0] !== 2'd1) begin errors=errors+1; $display("FAIL: progB tag not float"); end
    chk64(rraw[0], 64'h40091FEF0A9265EB, "progB float bits");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: progB cycles==0"); end
    $display("progB cycles = %0d (float Show, bits=%016h)", rcycles, rraw[0]);

    // ── Program C: INTERLEAVED — Show int 3, THEN ShowF pi — must come back in ORDER, tagged. ──
    w(0,   8'h05, 8'h00, 8'h00, 8'h03);      // LoadConst r0, 3
    w(4,   8'h59, 8'h00, 8'h00, 8'h00);      // Show r0            (int, first)
    w(8,   8'hC9, 8'h01, 8'h00, 8'h00);      // FConst f1
    wraw(12, 32'h0A9265EB);                  //   low  32 bits
    wraw(16, 32'h40091FEF);                  //   high 32 bits
    w(20,  8'hD4, 8'h00, 8'h01, 8'h00);      // ShowF f1           (float, second)
    w(24,  8'h2B, 8'h00, 8'h00, 8'h00);      // Halt
    plen = 28; run_program;
    chk(rcount, 8'd2, "progC count");
    if (rtag[0] !== 2'd0) begin errors=errors+1; $display("FAIL: progC show0 not int"); end
    if (rtag[1] !== 2'd1) begin errors=errors+1; $display("FAIL: progC show1 not float"); end
    chkv(rraw[0][31:0], 32'd3, "progC int show");
    chk64(rraw[1], 64'h40091FEF0A9265EB, "progC float show");
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: progC cycles==0"); end
    $display("progC cycles = %0d (int 3 THEN float pi, in order)", rcycles);

    // ── Program D: a single STRING Show — `Show "hi"` (tag 2), the codepoints 'h','i' round-trip. ──
    // Emitted by `target/debug/examples/lg2hex --full` for `## Main\nShow "hi".`:
    //   TextConst r0 (op 216=0xD8), imm = char-pool offset 4; ShowText r0 (op 217=0xD9); Halt; then 'h','i'.
    w(0,   8'hD8, 8'h00, 8'h00, 8'h02);      // TextConst r0  (dst=0, len=2 in the low byte)
    wraw(4,  32'h00000004);                  //   char-pool offset = imem word 4
    w(8,   8'hD9, 8'h00, 8'h00, 8'h00);      // ShowText r0  (op 217 = 0xD9)
    w(12,  8'h2B, 8'h00, 8'h00, 8'h00);      // Halt
    wraw(16, 32'h00000068);                  //   'h'
    wraw(20, 32'h00000069);                  //   'i'
    plen = 24; run_program;
    chk(rcount, 8'd1, "progD count");
    if (rtag[0] !== 2'd2) begin errors=errors+1; $display("FAIL: progD tag not string (got %0d)", rtag[0]); end
    chk_str(0, "hi", 2);
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: progD cycles==0"); end
    $display("progD cycles = %0d (string Show \"hi\")", rcycles);

    // ── Program E: INTERLEAVED — Show int 42, THEN Show "hi" — string tag-2 record after an int record. ──
    w(0,   8'h05, 8'h00, 8'h00, 8'h2A);      // LoadConst r0, 42
    w(4,   8'h59, 8'h00, 8'h00, 8'h00);      // Show r0            (int, first)
    w(8,   8'hD8, 8'h01, 8'h00, 8'h02);      // TextConst r1  (dst=1, len=2)
    wraw(12, 32'h00000018);                  //   char-pool offset = imem word 24
    w(16,  8'hD9, 8'h00, 8'h01, 8'h00);      // ShowText r1        (string, second)
    w(20,  8'h2B, 8'h00, 8'h00, 8'h00);      // Halt
    // words 22,23 unused (padding to reach the char pool at word 24)
    w(88,  8'h00, 8'h00, 8'h00, 8'h68);      // imem[22] (unused word, harmless)
    w(92,  8'h00, 8'h00, 8'h00, 8'h69);      // imem[23] (unused word, harmless)
    wraw(96, 32'h00000068);                  // imem[24] = 'h'
    wraw(100, 32'h00000069);                 // imem[25] = 'i'
    plen = 104; run_program;
    chk(rcount, 8'd2, "progE count");
    if (rtag[0] !== 2'd0) begin errors=errors+1; $display("FAIL: progE show0 not int (got %0d)", rtag[0]); end
    if (rtag[1] !== 2'd2) begin errors=errors+1; $display("FAIL: progE show1 not string (got %0d)", rtag[1]); end
    chkv(rraw[0][31:0], 32'd42, "progE int show");
    chk_str(1, "hi", 2);
    if (rcycles == 0) begin errors=errors+1; $display("FAIL: progE cycles==0"); end
    $display("progE cycles = %0d (int 42 THEN string \"hi\", in order)", rcycles);

    // ── Program F: an EMPTY string Show — `Show ""` (tag 2, char count 0, no codepoints follow). ──
    // lg2hex --full emits: TextConst r0 (len 0), ShowText r0, Halt (no char pool needed).
    w(0,   8'hD8, 8'h00, 8'h00, 8'h00);      // TextConst r0  (len 0)
    wraw(4,  32'h00000004);                  //   char-pool offset (unused for an empty string)
    w(8,   8'hD9, 8'h00, 8'h00, 8'h00);      // ShowText r0
    w(12,  8'h2B, 8'h00, 8'h00, 8'h00);      // Halt
    plen = 16; run_program;
    chk(rcount, 8'd1, "progF count");
    if (rtag[0] !== 2'd2) begin errors=errors+1; $display("FAIL: progF tag not string (got %0d)", rtag[0]); end
    chk_str(0, "", 0);
    $display("progF cycles = %0d (empty string Show)", rcycles);

    // ── Program G: 7, "abc", 9 — a string BETWEEN two int Shows; the sender must return to a tag byte after
    //    a string record. lg2hex --full for `Show 7. Show "abc". Show 9.` (11 words). ──
    wraw(0,  32'h05000007);                  // LoadConst r0, 7
    wraw(4,  32'h59000000);                  // Show r0
    wraw(8,  32'hD8010003);                  // TextConst r1  (len 3)
    wraw(12, 32'h00000008);                  //   char-pool offset = imem word 8
    wraw(16, 32'hD9000100);                  // ShowText r1
    wraw(20, 32'h05020009);                  // LoadConst r2, 9
    wraw(24, 32'h59000200);                  // Show r2
    wraw(28, 32'h2B000000);                  // Halt
    wraw(32, 32'h00000061);                  //   'a'
    wraw(36, 32'h00000062);                  //   'b'
    wraw(40, 32'h00000063);                  //   'c'
    plen = 44; run_program;
    chk(rcount, 8'd3, "progG count");
    if (rtag[0] !== 2'd0) begin errors=errors+1; $display("FAIL: progG show0 not int (got %0d)", rtag[0]); end
    if (rtag[1] !== 2'd2) begin errors=errors+1; $display("FAIL: progG show1 not string (got %0d)", rtag[1]); end
    if (rtag[2] !== 2'd0) begin errors=errors+1; $display("FAIL: progG show2 not int (got %0d)", rtag[2]); end
    chkv(rraw[0][31:0], 32'd7, "progG int show0");
    chk_str(1, "abc", 3);
    chkv(rraw[2][31:0], 32'd9, "progG int show2");
    $display("progG cycles = %0d (int 7, string \"abc\", int 9 — in order)", rcycles);

    if (errors == 0)
      $display("tb_nexys_soc_full: PASS - int + float + STRING Shows round-trip over the tagged UART protocol (interleaved, in order); float bits == 40091FEF0A9265EB; string \"hi\" reassembled from tag-2 codepoints");
    else
      $display("tb_nexys_soc_full: FAIL - %0d error(s)", errors);
    $finish;
  end

  // Watchdog so a wiring bug cannot hang the run.
  initial begin
    #20000000;
    $display("tb_nexys_soc_full: FAIL - timeout (no complete readback)");
    $finish;
  end
endmodule
