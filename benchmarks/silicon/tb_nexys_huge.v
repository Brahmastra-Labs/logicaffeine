`timescale 1ns/1ps
// tb_nexys_huge — proves the IMEM→BRAM capacity gain (Phase A): a program LARGER than the old 2047-word
// ceiling loads into the 4096-word block-RAM imem and executes correctly. The program is 3004 words:
//   LoadConst r2,1 ; LoadConst r1,0 ; 3000× (RevAdd r1 += r2) ; Show r1 ; Halt   → Show = 3000.
// Streamed via the 2-byte-length escape (0xFF, len_hi, len_lo). This is the capacity a combinational
// LUT-RAM imem could never hold (it capped at ~256 words); the sync-read BRAM core runs it bit-exact.
module tb;
  localparam CPB = 4;                          // clocks-per-UART-bit (fast, for sim)
  localparam N   = 3000;                        // increment count → the Show value
  localparam NW  = N + 4;                       // total program words (2 setup + N + Show + Halt) = 3004
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

  reg  [31:0] words [0:4095];
  integer     j, bp;
  reg  [7:0]  got, tagb;
  reg  [5:0]  rc;
  reg  [31:0] val, rcyc;

  initial begin
    // Build the increment program: r2=1, r1=0, then N× (r1 += r2), Show r1, Halt.
    words[0] = 32'h05020001;                     // LoadConst r2, 1     [op=5][dst=2][imm16=1]
    words[1] = 32'h05010000;                     // LoadConst r1, 0     [op=5][dst=1][imm16=0]
    for (j = 0; j < N; j = j + 1)
      words[2 + j] = 32'h00010200;               // RevAdd r1 += r2     [op=0][dst=1][ra=2][rb=0]
    words[N + 2] = 32'h59000100;                 // Show r1             [op=89][dst=0][ra=1][rb=0]
    words[N + 3] = 32'h2B000000;                 // Halt                [op=43]

    rst = 1'b1; repeat (4) @(posedge clk);
    rst = 1'b0; repeat (4) @(posedge clk);

    // 2-BYTE ESCAPE: 0xFF then the 16-bit word count MSB-first (NW = 3004 = 0x0BBC > 2047).
    send_byte(8'hFF);
    send_byte(NW[15:8]);
    send_byte(NW[7:0]);
    for (j = 0; j < NW; j = j + 1) begin
      send_byte(words[j][31:24]); send_byte(words[j][23:16]);
      send_byte(words[j][15:8]);  send_byte(words[j][7:0]);
    end

    // Readback frame: 0xA5 sync, count(=1), tag+4 value bytes, 4 cycle bytes, 0x5A.
    recv_byte(got); if (got !== 8'hA5) begin errors = errors + 1; $display("FAIL: sync=%02h", got); end
    recv_byte(rc);
    if (rc !== 6'd1) begin errors = errors + 1; $display("FAIL: rcount=%0d expected 1", rc); end
    recv_byte(tagb);
    val = 32'd0;
    for (bp = 0; bp < 4; bp = bp + 1) begin recv_byte(got); val = {val[23:0], got}; end
    recv_byte(rcyc[31:24]); recv_byte(rcyc[23:16]); recv_byte(rcyc[15:8]); recv_byte(rcyc[7:0]);
    recv_byte(got); if (got !== 8'h5A) begin errors = errors + 1; $display("FAIL: end=%02h", got); end

    if (val === N && errors == 0)
      $display("tb_nexys_huge: PASS - a %0d-word program (>2047, the old ceiling) ran on the 4096-word BRAM imem; Show=%0d, on-chip cycles=%0d",
               NW, val, rcyc);
    else
      $display("tb_nexys_huge: FAIL - Show=%0d expected %0d, nwords=%0d, errors=%0d", val, N, NW, errors);
    $finish;
  end
endmodule
