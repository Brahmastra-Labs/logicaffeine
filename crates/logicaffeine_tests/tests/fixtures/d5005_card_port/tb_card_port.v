// The card port between a scripted engine and four memory channels on four unrelated clocks, each the shell's own
// channel path (logos_d5005_channel) in front of a memory with random waits, preloaded from channel<b>.hex. The engine
// side runs ops.hex in order — a read held to the line expect.hex gives it, a write under its mask held until the port
// is ready — with random gaps between them; then no channel holds a write's done token the port left, and each
// channel's memory is written to out<b>.hex.
//   ops.hex:    {kind[7:0] (0 a read, 1 a write), line[31:0], mask[63:0], data[511:0]}
//   expect.hex: a read's line, in the order of the reads
`timescale 1ps/1ps
module tb;
  parameter AW = 27, DWM = 512, ECC = 0, LW = 32, RAW = 6, WAW = 4, LINES_LOG2 = 14, DUMP = 1, NOPS = 1, NREADS = 1,
            SPACE = 0, BASE = 0, GRANULE_LOG2 = 3, SEED = 1, GAP = 30;
  localparam NB = 4, DL = 512, DLW = AW + LW + 1;
  reg uclk = 0, rst_n = 0;
  // A channel clock a register, each its own process: Verilator takes no edge of one bit of a vector.
  reg mem_clk0 = 0, mem_clk1 = 0, mem_clk2 = 0, mem_clk3 = 0;
  wire [NB-1:0] mem_clk = {mem_clk3, mem_clk2, mem_clk1, mem_clk0};
  always #1111 uclk = ~uclk;
  always #1667 mem_clk0 = ~mem_clk0;
  initial begin #431 forever #1671 mem_clk1 = ~mem_clk1; end
  initial begin #977 forever #1663 mem_clk2 = ~mem_clk2; end
  initial begin #1301 forever #1669 mem_clk3 = ~mem_clk3; end

  reg  [615:0]    ops [0:NOPS-1];
  reg  [DL-1:0]   expected [0:NREADS-1];
  reg             mem_rd = 0, mem_wr = 0, clear = 0;
  reg  [31:0]     mem_rd_line = 0, mem_wr_line = 0;
  reg  [DL-1:0]   mem_wr_data = 0;
  reg  [DL/8-1:0] mem_wr_mask = 0;
  wire            mem_rd_valid, mem_wr_ready, fault;
  wire [DL-1:0]   mem_rd_data;
  wire [NB-1:0]   md_valid, md_ready, mr_valid, mr_ready, mw_valid, mw_ready, mdone_valid, mdone_ready;
  wire [NB*DLW-1:0] md_data;
  wire [NB*DL-1:0]  mr_data, mw_data;
  wire [NB*16-1:0]  mr_ecc;
  logos_card_port #(.NB(NB), .AW(AW), .LW(LW), .DL(DL)) port(.clk(uclk), .ce(1'b1), .rst_n(rst_n), .space(SPACE[31:0]),
    .base(BASE[AW-1:0]), .granule_log2(GRANULE_LOG2[4:0]), .clear(clear), .fault(fault), .mem_rd(mem_rd), .mem_rd_line(mem_rd_line),
    .mem_rd_valid(mem_rd_valid), .mem_rd_data(mem_rd_data), .mem_wr(mem_wr), .mem_wr_line(mem_wr_line),
    .mem_wr_data(mem_wr_data), .mem_wr_mask(mem_wr_mask), .mem_wr_ready(mem_wr_ready), .md_valid(md_valid),
    .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready), .mr_data(mr_data),
    .mr_ecc(mr_ecc), .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data(mw_data), .mdone_valid(mdone_valid),
    .mdone_ready(mdone_ready));

  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : ch
    wire [AW-1:0]    address;
    wire             read, write, waitrequest, readdatavalid;
    wire [6:0]       burstcount;
    wire [DWM-1:0]   writedata, readdata;
    wire [DWM/8-1:0] byteenable;
    logos_d5005_channel #(.AW(AW), .DWM(DWM), .ECC(ECC), .MBW(7), .LW(LW), .MAXB(16), .RAW(RAW), .WAW(WAW)) channel(
      .uclk(uclk), .uce(1'b1), .uclk_rst_n(rst_n), .mem_clk(mem_clk[b]), .mem_rst_n(rst_n),
      .md_valid(md_valid[b]), .md_ready(md_ready[b]), .md_data(md_data[b*DLW +: DLW]),
      .mr_valid(mr_valid[b]), .mr_ready(mr_ready[b]), .mr_data(mr_data[b*DL +: DL]), .mr_ecc(mr_ecc[b*16 +: 16]),
      .mw_valid(mw_valid[b]), .mw_ready(mw_ready[b]), .mw_data(mw_data[b*DL +: DL]), .mw_flip(21'h0),
      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]), .mem_address(address), .mem_read(read),
      .mem_write(write), .mem_burstcount(burstcount), .mem_writedata(writedata), .mem_byteenable(byteenable),
      .mem_waitrequest(waitrequest), .mem_readdata(readdata), .mem_readdatavalid(readdatavalid));
    logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(7), .LINES_LOG2(LINES_LOG2), .SEED(SEED * 10 + b)) memory(
      .clk(mem_clk[b]), .rst_n(rst_n), .address(address), .read(read), .write(write), .burstcount(burstcount),
      .writedata(writedata), .byteenable(byteenable), .waitrequest(waitrequest), .readdata(readdata),
      .readdatavalid(readdatavalid));
  end endgenerate

  integer seed = SEED, i, reads = 0, errors = 0;
  reg [615:0] op;
  initial begin
    $readmemh("ops.hex", ops);
    $readmemh("expect.hex", expected);
    #1;
    $readmemh("channel0.hex", ch[0].memory.mem, 0, DUMP - 1);
    $readmemh("channel1.hex", ch[1].memory.mem, 0, DUMP - 1);
    $readmemh("channel2.hex", ch[2].memory.mem, 0, DUMP - 1);
    $readmemh("channel3.hex", ch[3].memory.mem, 0, DUMP - 1);
    #20000 rst_n = 1;
    repeat (10) @(negedge uclk);
    for (i = 0; i < NOPS; i = i + 1) begin
      op = ops[i];
      if (op[615:608] == 8'd0) begin
        mem_rd = 1'b1;
        mem_rd_line = op[607:576];
        @(negedge uclk);
        mem_rd = 1'b0;
        while (!mem_rd_valid) @(negedge uclk);
        if (mem_rd_data !== expected[reads]) begin
          errors = errors + 1;
          if (errors < 8) $display("FAIL op %0d reads line %0d wrong", i, op[607:576]);
        end
        reads = reads + 1;
      end else begin
        mem_wr = 1'b1;
        mem_wr_line = op[607:576];
        mem_wr_mask = op[575:512];
        mem_wr_data = op[511:0];
        @(negedge uclk);
        while (!mem_wr_ready) @(negedge uclk);
        mem_wr = 1'b0;
      end
      repeat ((($random(seed) % GAP) + GAP) % GAP) @(negedge uclk);
    end
    repeat (50) @(negedge uclk);
    if (mdone_valid != 0) begin
      errors = errors + 1;
      $display("FAIL a write's done token is left on channels %b", mdone_valid);
    end
    $writememh("out0.hex", ch[0].memory.mem, 0, DUMP - 1);
    $writememh("out1.hex", ch[1].memory.mem, 0, DUMP - 1);
    $writememh("out2.hex", ch[2].memory.mem, 0, DUMP - 1);
    $writememh("out3.hex", ch[3].memory.mem, 0, DUMP - 1);
    $display("RESULT ops=%0d reads=%0d errors=%0d fault=%0d", NOPS, reads, errors, fault);
    $finish;
  end
  initial begin #20000000000; $display("RESULT timeout ops done %0d", i); $finish; end
endmodule
