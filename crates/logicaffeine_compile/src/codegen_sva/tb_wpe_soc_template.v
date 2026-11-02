// tb_wpe_soc___NAME__ — the OVER-THE-WIRE HOST for the wpe_soc engine. Reads the concatenated wire bytes
// (wire___NAME__.hex, one byte/line), per-vector byte lengths (lens___NAME__.hex), and the eval_wprog_config
// oracle (oracle___NAME__.hex); for each vector it bit-bangs that vector's bytes onto uart_rx, decodes the
// returned frame (__X_FRAME_SYNC__ sync + result[63:0] MSB-first + cycles[31:0] + __X_FRAME_END__), and checks result == oracle.
// SHOW records (__X_REC_SHOW__ tag payload) that arrive before the frame are printed as `SHOW`/`SHOWT`/`SHOWB` lines.
// Proves the WHOLE wire -> on-chip synthesize -> run -> wire path ≡ the VM under iverilog.
`timescale 1ns/1ps
module tb_wpe_soc___NAME__;
  localparam CLKS = __CLKS__;
  localparam NVEC = __NVEC__;
  localparam MAXB = __MAXBYTES__;
  reg clk=0, rst=1, uart_rx=1;
  wire uart_tx;
//__ONCHIP_ALL_BEGIN__
  wpe_soc___NAME__ #(.CLKS_PER_BIT(CLKS)) dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx));
  always #5 clk = ~clk;
//__ONCHIP_ALL_END__
//__OFFCHIP_ANY_BEGIN__
  wire psram_ce_n, psram_sclk; wire [3:0] psram_sio_out, psram_sio_oe, psram_sio;
  wpe_soc___NAME__ #(.CLKS_PER_BIT(CLKS)) dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx),
    .psram_ce_n(psram_ce_n), .psram_sclk(psram_sclk), .psram_sio_out(psram_sio_out), .psram_sio_oe(psram_sio_oe),
    .psram_sio_in(psram_sio));
  always #__HALF_NS__ clk = ~clk;
  // The board: the PSRAM's four SIO pads, driven by whichever side enables them, and the part itself.
  genvar pad;
  generate for (pad = 0; pad < 4; pad = pad + 1) begin : sio_pads
    assign psram_sio[pad] = psram_sio_oe[pad] ? psram_sio_out[pad] : 1'bz;
  end endgenerate
  psram_model part(.ce_n(psram_ce_n), .sclk(psram_sclk), .sio(psram_sio));
  // Every received byte must be taken before the next one completes: the receiver holds one byte while the
  // engine is stalled on the part, so a byte is lost exactly when another completes during that stall.
  // `part_stall` counts the stalled cycles, which the result frame's run-cycle count excludes.
  integer rx_done = 0, rx_taken = 0, part_stall = 0;
  always @(posedge clk) if (!rst) begin
    if (dut.mstall) part_stall = part_stall + 1;
    if (dut.rx_valid && !dut.mstall) rx_taken = rx_taken + 1;
    if (dut.r_state == 2'd3 && dut.r_cnt == CLKS - 1) begin
      if (rx_done != rx_taken) begin
        $display("UART-LOST byte %0d", rx_taken);
        $display("FAIL: a received byte was overwritten before the engine took it");
        $finish;
      end
      rx_done = rx_done + 1;
    end
  end
//__OFFCHIP_ANY_END__

  reg [7:0]  wirebytes [0:MAXB-1];
  reg [31:0] lens   [0:NVEC-1];
  reg [63:0] oracle [0:NVEC-1];
  integer k, fk, vi, bi, cursor, errors;   // k is used INSIDE send_byte/recv_byte; fk for the frame-decode loops
  reg [7:0]  rb, sync, endb;
  reg [7:0]  tag; reg [31:0] slen; reg [63:0] sval; integer sj;   // SHOW record decode
  reg [63:0] result; reg [31:0] cyc;

  task send_byte(input [7:0] b);
    begin
      @(negedge clk); uart_rx <= 1'b0;
      for (k=0;k<CLKS;k=k+1) @(negedge clk);
      for (k=0;k<8;k=k+1) begin uart_rx <= b[k]; repeat (CLKS) @(negedge clk); end
      uart_rx <= 1'b1; repeat (CLKS) @(negedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    begin
      @(negedge uart_tx);
      repeat (CLKS + CLKS/2) @(posedge clk);
      for (k=0;k<8;k=k+1) begin b[k] = uart_tx; if (k<7) repeat (CLKS) @(posedge clk); end
    end
  endtask

  initial begin
    $readmemh("wire___NAME__.hex",   wirebytes);
    $readmemh("lens___NAME__.hex",   lens);
    $readmemh("oracle___NAME__.hex", oracle);
    errors = 0; cursor = 0;
    repeat (4) @(negedge clk); rst <= 0; repeat (2) @(negedge clk);
    for (vi=0; vi<NVEC; vi=vi+1) begin
      for (bi=0; bi<lens[vi]; bi=bi+1) begin send_byte(wirebytes[cursor]); cursor = cursor + 1; end
      recv_byte(sync);
      // SHOW records (__X_REC_SHOW__, tag, payload) stream BEFORE the result frame; print each for the harness to check.
      while (sync == __V_REC_SHOW__) begin
        recv_byte(tag);
        if (tag == 8'd2) begin
          slen = 32'd0; for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); slen = {slen[23:0], rb}; end
          $display("SHOWT len=%0d", slen);
          for (sj=0; sj<slen; sj=sj+1) begin recv_byte(rb); $display("SHOWB %02x", rb); end
        end else begin
          sval = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); sval = {sval[55:0], rb}; end
          $display("SHOW t=%0d v=%016x", tag, sval);
        end
        recv_byte(sync);
      end
      result = 64'd0; for (fk=0;fk<8;fk=fk+1) begin recv_byte(rb); result = {result[55:0], rb}; end
      cyc = 32'd0;    for (fk=0;fk<4;fk=fk+1) begin recv_byte(rb); cyc = {cyc[23:0], rb}; end
      recv_byte(endb);
      $display("FRAME result=%016x cycles=%0d end=%02x", result, cyc, endb);
      if (sync!==__V_FRAME_SYNC__ || endb!==__V_FRAME_END__ || result!==oracle[vi]) begin
        errors = errors + 1;
        $display("FAIL vec %0d: sync=%02x result=%0d exp=%0d end=%02x", vi, sync, result, oracle[vi], endb);
      end
    end
    if (errors==0) $display("PASS: %0d over-the-wire runs match eval_wprog_config", NVEC);
    else $display("FAIL: %0d over-the-wire mismatches", errors);
//__OFFCHIP_ANY_BEGIN__
    $display("PSRAM-VIOLATIONS %0d", part.violations);
    $display("PSRAM-STALL-CYCLES %0d", part_stall);
//__OFFCHIP_ANY_END__
    $finish;
  end

  initial begin #20000000000 $display("FAIL: timeout"); $finish; end
endmodule
