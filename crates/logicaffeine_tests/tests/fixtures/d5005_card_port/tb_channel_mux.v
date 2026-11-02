// P fast ports through the channel mux beside the card port, on four memory channels on four unrelated clocks, each a
// hybrid engine's channel path (logos_d5005_channel with its fast port's reads and writes) in front of a memory with
// random waits, preloaded from channel<b>.hex. The card port runs ops.hex in order in the engine clock, as the card
// port's own testbench does; fast port p runs fast<p>.hex in the user clock, unrelated to it, its reads and writes taken
// back to back with random gaps, its reads checked in the order they return against fast_expect<p>.hex, and a fence
// waiting until every write of its is reported written and every read of its has returned; every port at once, all
// of them through logos_channel_mux onto the channels' one fast side. Then no channel holds a write's done token any
// port left, and each channel's memory is written to out<b>.hex.
//   ops.hex:            {kind[7:0] (0 a read, 1 a write), line[31:0], mask[63:0], data[511:0]}
//   fast<p>.hex:        {kind[7:0] (0 a read, 1 a write, 2 a fence), line[31:0], data[511:0]}
//   fast_expect<p>.hex: a fast read's line, in the order of port p's reads
//   counts.hex:         port p's ops and reads, {ops[31:0], reads[31:0]}, a line a port (NFAST and NFREADS the most)
`timescale 1ps/1ps
module tb;
  parameter AW = 27, DWM = 512, ECC = 0, LW = 32, RAW = 6, WAW = 4, LINES_LOG2 = 14, DUMP = 1, NOPS = 1, NREADS = 1,
            P = 2, NFAST = 1, NFREADS = 1, SPACE = 0, BASE = 0, GRANULE_LOG2 = 3, SEED = 1, GAP = 30, FGAP = 4, MAXL = 32;
  localparam NB = 4, DL = 512, DLW = AW + LW + 1;
  reg uclk = 0, fclk = 0, rst_n = 0;
  reg mem_clk0 = 0, mem_clk1 = 0, mem_clk2 = 0, mem_clk3 = 0;
  wire [NB-1:0] mem_clk = {mem_clk3, mem_clk2, mem_clk1, mem_clk0};
  always #1111 uclk = ~uclk;
  initial begin #257 forever #947 fclk = ~fclk; end
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

  // Every fast port's channel side, port p's of channel b at p·NB + b, and the channels' one fast side after the mux.
  wire [P*NB-1:0]     pd_valid, pd_ready, pr_valid, pr_ready, pw_valid, pw_ready, pdone_valid, pdone_ready;
  wire [P*NB*DLW-1:0] pd_data;
  wire [P*NB*DL-1:0]  pw_data;
  wire [NB-1:0]   fd_valid, fd_ready, fr_valid, fr_ready, fw_valid, fw_ready, fdone_valid, fdone_ready;
  wire [NB*DLW-1:0] fd_data;
  wire [NB*DL-1:0]  fr_data, fw_data;
  reg frst = 1;
  logos_channel_mux #(.P(P), .NB(NB), .AW(AW), .LW(LW), .DL(DL)) mux(.clk(fclk), .rst(frst), .pd_valid(pd_valid),
    .pd_ready(pd_ready), .pd_data(pd_data), .pr_valid(pr_valid), .pr_ready(pr_ready), .pw_valid(pw_valid),
    .pw_ready(pw_ready), .pw_data(pw_data), .pdone_valid(pdone_valid), .pdone_ready(pdone_ready), .md_valid(fd_valid),
    .md_ready(fd_ready), .md_data(fd_data), .mr_valid(fr_valid), .mr_ready(fr_ready), .mw_valid(fw_valid),
    .mw_ready(fw_ready), .mw_data(fw_data), .mdone_valid(fdone_valid), .mdone_ready(fdone_ready));

  genvar b, p;
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
      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]),
      .fclk(fclk), .fclk_rst_n(rst_n), .fd_valid(fd_valid[b]), .fd_ready(fd_ready[b]), .fd_data(fd_data[b*DLW +: DLW]),
      .fr_valid(fr_valid[b]), .fr_ready(fr_ready[b]), .fr_data(fr_data[b*DL +: DL]), .fw_valid(fw_valid[b]),
      .fw_ready(fw_ready[b]), .fw_data(fw_data[b*DL +: DL]), .fdone_valid(fdone_valid[b]), .fdone_ready(fdone_ready[b]),
      .mem_address(address), .mem_read(read),
      .mem_write(write), .mem_burstcount(burstcount), .mem_writedata(writedata), .mem_byteenable(byteenable),
      .mem_waitrequest(waitrequest), .mem_readdata(readdata), .mem_readdatavalid(readdatavalid));
    logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(7), .LINES_LOG2(LINES_LOG2), .SEED(SEED * 10 + b)) memory(
      .clk(mem_clk[b]), .rst_n(rst_n), .address(address), .read(read), .write(write), .burstcount(burstcount),
      .writedata(writedata), .byteenable(byteenable), .waitrequest(waitrequest), .readdata(readdata),
      .readdatavalid(readdatavalid));
  end endgenerate

  integer fast_done = 0, fast_count = 0, fast_reads = 0, fast_errors = 0, fast_faults = 0;
  reg [63:0] counts [0:P-1];
  initial $readmemh("counts.hex", counts);
  generate for (p = 0; p < P; p = p + 1) begin : fp
    reg  [551:0]    fops [0:NFAST-1];
    reg  [DL-1:0]   fexpected [0:NFREADS-1];
    reg             rq_valid = 0, wq_valid = 0;
    reg  [31:0]     rq_line = 0, wq_line = 0;
    reg  [DL-1:0]   wq_data = 0;
    wire            rq_ready, rs_valid, wq_ready, writing, ffault;
    wire [DL-1:0]   rs_data;
    logos_fast_port #(.NB(NB), .AW(AW), .LW(LW), .DL(DL), .MAXL(MAXL)) fport(.clk(fclk), .rst(frst), .space(SPACE[31:0]),
      .base(BASE[AW-1:0]), .granule_log2(GRANULE_LOG2[4:0]), .clear(1'b0), .fault(ffault), .rq_valid(rq_valid),
      .rq_ready(rq_ready), .rq_line(rq_line), .rs_valid(rs_valid), .rs_data(rs_data), .wq_valid(wq_valid),
      .wq_ready(wq_ready), .wq_line(wq_line), .wq_data(wq_data), .writing(writing), .md_valid(pd_valid[p*NB +: NB]),
      .md_ready(pd_ready[p*NB +: NB]), .md_data(pd_data[p*NB*DLW +: NB*DLW]), .mr_valid(pr_valid[p*NB +: NB]),
      .mr_ready(pr_ready[p*NB +: NB]), .mr_data(fr_data), .mw_valid(pw_valid[p*NB +: NB]), .mw_ready(pw_ready[p*NB +: NB]),
      .mw_data(pw_data[p*NB*DL +: NB*DL]), .mdone_valid(pdone_valid[p*NB +: NB]), .mdone_ready(pdone_ready[p*NB +: NB]));
    // The port's reads in the order they return, each against the line it should be.
    integer freads = 0, fasked = 0, ferrors = 0, j, fseed = SEED * 7 + 3 + 101 * p;
    reg [551:0] fop;
    always @(posedge fclk) if (rs_valid) begin
      if (freads >= counts[p][31:0] || rs_data !== fexpected[freads]) begin
        ferrors = ferrors + 1;
        if (ferrors < 8) $display("FAIL port %0d's read %0d returns the wrong line", p, freads);
      end
      freads = freads + 1;
    end
    initial begin
      $readmemh($sformatf("fast%0d.hex", p), fops);
      $readmemh($sformatf("fast_expect%0d.hex", p), fexpected);
      #20000;
      repeat (4) @(posedge fclk);
      #1;
      repeat (12) @(posedge fclk);
      #1;
      for (j = 0; j < counts[p][63:32]; j = j + 1) begin
        fop = fops[j];
        if (fop[551:544] == 8'd2) begin
          rq_valid = 0; wq_valid = 0;
          @(posedge fclk);
          while (writing || freads != fasked) @(posedge fclk);
          #1;
        end else begin
          if (fop[551:544] == 8'd0) begin rq_valid = 1; wq_valid = 0; rq_line = fop[543:512]; end
          else begin wq_valid = 1; rq_valid = 0; wq_line = fop[543:512]; wq_data = fop[511:0]; end
          @(posedge fclk);
          while (!(rq_valid ? rq_ready : wq_ready)) @(posedge fclk);
          if (rq_valid) fasked = fasked + 1;
          #1;
          if ((($random(fseed) % FGAP) + FGAP) % FGAP == 0) begin
            rq_valid = 0; wq_valid = 0;
            repeat ((($random(fseed) % 9) + 9) % 9) @(posedge fclk);
            #1;
          end
        end
      end
      rq_valid = 0; wq_valid = 0;
      @(posedge fclk);
      while (writing || freads != fasked) @(posedge fclk);
      $display("PORT %0d fast=%0d freads=%0d ferrors=%0d ffault=%0d", p, j, freads, ferrors, ffault);
      fast_done = fast_done + 1;
      fast_count = fast_count + j;
      fast_reads = fast_reads + freads;
      fast_errors = fast_errors + ferrors;
      fast_faults = fast_faults + ffault;
    end
  end endgenerate
  initial begin
    #20000;
    repeat (4) @(posedge fclk);
    #1 frst = 0;
  end

  integer seed = SEED, i, reads = 0, errors = 0, card_done = 0;
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
    card_done = 1;
  end
  initial begin
    wait (card_done && fast_done == P);
    repeat (50) @(negedge uclk);
    if (mdone_valid != 0 || fdone_valid != 0) begin
      errors = errors + 1;
      $display("FAIL a write's done token is left on channels %b / %b", mdone_valid, fdone_valid);
    end
    $writememh("out0.hex", ch[0].memory.mem, 0, DUMP - 1);
    $writememh("out1.hex", ch[1].memory.mem, 0, DUMP - 1);
    $writememh("out2.hex", ch[2].memory.mem, 0, DUMP - 1);
    $writememh("out3.hex", ch[3].memory.mem, 0, DUMP - 1);
    $display("RESULT ops=%0d reads=%0d errors=%0d fault=%0d fast=%0d freads=%0d ferrors=%0d", NOPS, reads, errors, fault,
      fast_count, fast_reads, fast_errors);
    $finish;
  end
  initial begin #40000000000; $display("RESULT timeout ops done %0d, %0d ports done", i, fast_done); $finish; end
endmodule
