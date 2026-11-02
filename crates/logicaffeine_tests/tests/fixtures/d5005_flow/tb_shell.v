// The shell end to end: four memory channels on four unrelated clocks, the host channel and the core on two more;
// every register, every command and every path between the clocks, read back. The core reports a running command's
// progress every 2^HB of its cycles, so every command here runs with progress records among its own. A shell whose
// compute side steps at every DIVIDE-th user cycle runs on a user clock DIVIDE times as fast, its compute side at the
// undivided shell's rate.
`timescale 1ps/1ps
module tb;
  parameter NB = 4, AW = 27, DWM = 576, MBW = 7, HAW = 51, DWH = 512, HBW = 7, MAW = 15, HB = 6, DIVIDE = 1;
  parameter [63:0] ID_L = 64'h0123456789abcdef, ID_H = 64'hfedcba9876543210;
  reg pclk = 0, uclk = 0, rst_n = 0;
  reg [NB-1:0] mem_clk = 0;
  always #2000 pclk = ~pclk;
  always #(1111 / DIVIDE) uclk = ~uclk;
  always #1667 mem_clk[0] = ~mem_clk[0];
  initial begin #431 forever #1671 mem_clk[1] = ~mem_clk[1]; end
  initial begin #977 forever #1663 mem_clk[2] = ~mem_clk[2]; end
  initial begin #1301 forever #1669 mem_clk[3] = ~mem_clk[3]; end

  reg  [MAW-1:0] mmio_address = 0; reg mmio_read = 0, mmio_write = 0; reg [63:0] mmio_writedata = 0; reg [7:0] mmio_byteenable = 8'hff;
  wire mmio_waitrequest, mmio_readdatavalid; wire [63:0] mmio_readdata;
  wire [HAW-1:0] host_rd_address, host_wr_address; wire host_rd_read, host_wr_write; wire [HBW-1:0] host_rd_burstcount, host_wr_burstcount;
  wire host_rd_waitrequest, host_rd_readdatavalid, host_wr_waitrequest, host_wr_writeresponsevalid;
  wire [DWH-1:0] host_rd_readdata, host_wr_writedata; wire [DWH/8-1:0] host_wr_byteenable;
  wire [NB*AW-1:0] mem_address; wire [NB-1:0] mem_read, mem_write, mem_waitrequest, mem_readdatavalid;
  wire [NB*MBW-1:0] mem_burstcount; wire [NB*DWM-1:0] mem_writedata, mem_readdata; wire [NB*DWM/8-1:0] mem_byteenable;

  logos_d5005_shell #(.NB(NB), .AW(AW), .DWM(DWM), .MBW(MBW), .HAW(HAW), .DWH(DWH), .HBW(HBW), .MAW(MAW),
                      .AFU_ID_L(ID_L), .AFU_ID_H(ID_H), .BUILD(64'h5eed), .HB(HB)) dut(
    .pclk(pclk), .pclk_rst_n(rst_n), .uclk(uclk), .uclk_rst_n(rst_n),
    .mmio_address(mmio_address), .mmio_read(mmio_read), .mmio_write(mmio_write), .mmio_writedata(mmio_writedata),
    .mmio_byteenable(mmio_byteenable), .mmio_waitrequest(mmio_waitrequest), .mmio_readdata(mmio_readdata),
    .mmio_readdatavalid(mmio_readdatavalid),
    .host_rd_address(host_rd_address), .host_rd_read(host_rd_read), .host_rd_burstcount(host_rd_burstcount),
    .host_rd_waitrequest(host_rd_waitrequest), .host_rd_readdata(host_rd_readdata), .host_rd_readdatavalid(host_rd_readdatavalid),
    .host_wr_address(host_wr_address), .host_wr_write(host_wr_write), .host_wr_burstcount(host_wr_burstcount),
    .host_wr_writedata(host_wr_writedata), .host_wr_byteenable(host_wr_byteenable), .host_wr_waitrequest(host_wr_waitrequest),
    .host_wr_writeresponsevalid(host_wr_writeresponsevalid),
    .mem_clk(mem_clk), .mem_rst_n({NB{rst_n}}), .mem_address(mem_address), .mem_read(mem_read), .mem_write(mem_write),
    .mem_burstcount(mem_burstcount), .mem_writedata(mem_writedata), .mem_byteenable(mem_byteenable),
    .mem_waitrequest(mem_waitrequest), .mem_readdata(mem_readdata), .mem_readdatavalid(mem_readdatavalid));

  // `hold` keeps every channel's memory from taking a command, as a channel the controller never serves.
  reg hold = 0;
  wire [NB-1:0] model_wait;
  assign mem_waitrequest = model_wait | {NB{hold}};
  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : m
    logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(MBW), .SEED(100 + b)) model(.clk(mem_clk[b]), .rst_n(rst_n),
      .address(mem_address[b*AW +: AW]), .read(mem_read[b] && !hold), .write(mem_write[b] && !hold),
      .burstcount(mem_burstcount[b*MBW +: MBW]), .writedata(mem_writedata[b*DWM +: DWM]),
      .byteenable(mem_byteenable[b*DWM/8 +: DWM/8]), .waitrequest(model_wait[b]), .readdata(mem_readdata[b*DWM +: DWM]),
      .readdatavalid(mem_readdatavalid[b]));
  end endgenerate
  logos_host_memory_model #(.AW(HAW), .DW(DWH), .BW(HBW)) host(.clk(pclk), .rst_n(rst_n),
    .rd_address(host_rd_address), .rd_read(host_rd_read), .rd_burstcount(host_rd_burstcount), .rd_waitrequest(host_rd_waitrequest),
    .rd_readdata(host_rd_readdata), .rd_readdatavalid(host_rd_readdatavalid), .wr_address(host_wr_address), .wr_write(host_wr_write),
    .wr_burstcount(host_wr_burstcount), .wr_writedata(host_wr_writedata), .wr_byteenable(host_wr_byteenable),
    .wr_waitrequest(host_wr_waitrequest), .wr_writeresponsevalid(host_wr_writeresponsevalid));

  localparam DFH = 0, ID_LO = 1, ID_HI = 2, SCRATCH = 5, BUILD_ID = 6, STATUS = 8, RESULT0 = 9, RESULT1 = 10,
             STATUS_SEQ = 11, CMD_SPACE = 12, ERRORS = 13, DROPPED = 14, ARG0 = 16, ARG1 = 17, ARG2 = 18, DOORBELL = 19;
  localparam [7:0] RUNNING = 8'h01;
  integer errors = 0;
  reg [63:0] v, seq0;

  task wr(input [MAW-1:0] a, input [63:0] d);
    begin
      @(posedge pclk); mmio_address <= a; mmio_writedata <= d; mmio_write <= 1;
      @(posedge pclk); mmio_write <= 0;
    end
  endtask
  task rd(input [MAW-1:0] a, output [63:0] d);
    begin
      @(posedge pclk); mmio_address <= a; mmio_read <= 1;
      @(posedge pclk); mmio_read <= 0;
      while (!mmio_readdatavalid) @(posedge pclk);
      d = mmio_readdata;
    end
  endtask
  task reg_is(input [MAW-1:0] a, input [63:0] want, input [8*40-1:0] what);
    reg [63:0] got;
    begin
      rd(a, got);
      if (got !== want) begin errors = errors + 1; $display("FAIL %0s: register %0d reads %h, want %h", what, a, got, want); end
    end
  endtask
  // Ring a command and wait for its status record.
  task run(input [7:0] op, input [55:0] tag, input [63:0] a0, input [63:0] a1, input [63:0] a2);
    reg [63:0] s;
    integer guard;
    begin
      rd(STATUS_SEQ, seq0);
      rd(STATUS, s);
      wr(ARG0, a0); wr(ARG1, a1); wr(ARG2, a2); wr(DOORBELL, {tag, op});
      guard = 0;
      // The record of this command carries the next sequence number, or its tag when the mirror was empty.
      while (1) begin
        rd(STATUS, s);
        if (s[63:8] == tag) begin rd(STATUS_SEQ, v); if (v != seq0) disable run; end
        guard = guard + 1;
        if (guard > 200000) begin errors = errors + 1; $display("FAIL op %0d tag %0h never completed", op, tag); disable run; end
      end
    end
  endtask

  function [DWM-1:0] pattern(input [31:0] seed, input [31:0] line);
    integer k; reg [31:0] lane;
    begin
      for (k = 0; k < DWM / 64; k = k + 1) begin lane = k; pattern[64*k +: 64] = {seed ^ line ^ (lane * 32'h9e3779b9), ~line + lane * 32'h85ebca6b}; end
    end
  endfunction

  integer i, k, ln, beats;
  reg [63:0] s, p, c, c2, last;
  initial begin
    #50000 rst_n = 1;
    repeat (20) @(posedge pclk);
    reg_is(DFH, {4'h1, 8'h0, 4'h0, 7'h0, 1'b1, 24'h0, 4'h1, 12'h0}, "feature header");
    reg_is(ID_LO, ID_L, "AFU ID low");
    reg_is(ID_HI, ID_H, "AFU ID high");
    reg_is(BUILD_ID, 64'h5eed, "build");
    wr(SCRATCH, 64'hdeadbeefcafef00d);
    reg_is(SCRATCH, 64'hdeadbeefcafef00d, "scratch");
    reg_is(CMD_SPACE, 64'd16, "command space");

    run(8'd0, 56'h1, 0, 0, 0);
    reg_is(STATUS, {56'h1, 8'h00}, "nop");

    run(8'd1, 56'h2, 64'hf, 64'd64, {32'h1234, 32'd200});
    reg_is(STATUS, {56'h2, 8'h00}, "fill");
    for (i = 0; i < NB; i = i + 1) for (ln = 64; ln < 264; ln = ln + 1) begin
      case (i)
        0: v = (m[0].model.mem[ln] !== pattern(32'h1234, ln));
        1: v = (m[1].model.mem[ln] !== pattern(32'h1234, ln));
        2: v = (m[2].model.mem[ln] !== pattern(32'h1234, ln));
        3: v = (m[3].model.mem[ln] !== pattern(32'h1234, ln));
      endcase
      if (v[0]) begin errors = errors + 1; if (errors < 4) $display("FAIL fill: channel %0d line %0d", i, ln); end
    end

    run(8'd2, 56'h3, 64'hf, 64'd64, {32'h1234, 32'd200});
    reg_is(STATUS, {56'h3, 8'h00}, "check");
    reg_is(RESULT0, 64'd0, "check finds nothing wrong");
    run(8'd2, 56'h4, 64'hf, 64'd64, {32'h1235, 32'd200});
    reg_is(RESULT0, 64'd800, "a wrong seed differs on every line of every channel");
    m[2].model.mem[100][300] = ~m[2].model.mem[100][300];
    run(8'd2, 56'h5, 64'hf, 64'd64, {32'h1234, 32'd200});
    reg_is(RESULT0, 64'd1, "one flipped bit in channel 2");

    for (ln = 0; ln < 4096; ln = ln + 1) host.mem[ln] = {16{ln[31:0] ^ 32'ha5a5_0000}};
    run(8'd3, 56'h6, 64'd10, {8'd1, 29'd0, 27'd1000}, {32'd0, 32'd130});
    reg_is(STATUS, {56'h6, 8'h00}, "host to memory");
    for (ln = 0; ln < 130; ln = ln + 1)
      if (m[1].model.mem[1000 + ln] !== {64'h0, host.mem[10 + ln]}) begin errors = errors + 1; if (errors < 8) $display("FAIL host->mem line %0d", ln); end

    run(8'd4, 56'h7, 64'd500, {8'd3, 29'd0, 27'd64}, {32'd0, 32'd150});
    reg_is(STATUS, {56'h7, 8'h00}, "memory to host");
    for (ln = 0; ln < 150; ln = ln + 1)
      if (host.mem[500 + ln] !== m[3].model.mem[64 + ln][DWH-1:0]) begin errors = errors + 1; if (errors < 12) $display("FAIL mem->host line %0d", ln); end

    run(8'd9, 56'h8, 0, 0, 0);
    reg_is(STATUS, {56'h8, 8'h80}, "an unknown op is refused");

    // Ring past the command FIFO while a long fill runs: what the FIFO cannot take is dropped, counted and flagged,
    // and every doorbell it took completes.
    wr(ARG0, 64'h1); wr(ARG1, 64'd0); wr(ARG2, {32'h77, 32'd3000});
    for (k = 0; k < 20; k = k + 1) wr(DOORBELL, {56'h100 + k, 8'd1});
    reg_is(ERRORS, 64'h1, "a doorbell past a full command FIFO is flagged");
    rd(DROPPED, seq0);
    if (seq0 < 1 || seq0 > 4) begin errors = errors + 1; $display("FAIL %0d doorbells dropped: the FIFO holds 16 beside the one running", seq0); end
    wr(ERRORS, 64'h1);
    reg_is(ERRORS, 64'h0, "errors clear on write");
    k = 0;
    while (k < 400000) begin rd(STATUS_SEQ, v); if (v + seq0 >= 64'd8 + 64'd20) k = 400000; else k = k + 1; end
    rd(STATUS_SEQ, v);
    if (v + seq0 != 64'd28) begin errors = errors + 1; $display("FAIL %0d completed and %0d dropped of 20", v - 8, seq0); end
    reg_is(STATUS, {56'h113 - seq0[55:0], 8'h00}, "the last doorbell taken completes last");

    // A long fill reports its progress while its lines move: records {the commands completed before it, {its tag,
    // RUNNING}, its progress, its cycles so far}, the progress growing record to record, then its own record.
    rd(STATUS_SEQ, seq0);
    wr(ARG0, 64'h1); wr(ARG1, 64'd0); wr(ARG2, {32'h99, 32'd4096});
    wr(DOORBELL, {56'h200, 8'd1});
    beats = 0; last = 0; k = 0;
    while (k == 0) begin
      rd(STATUS, s); rd(RESULT0, p); rd(RESULT1, c); rd(STATUS_SEQ, v);
      if (v != seq0) k = 1;
      else if (s == {56'h200, RUNNING}) begin
        if (p < last) begin errors = errors + 1; $display("FAIL the fill's progress went from %0d back to %0d", last, p); end
        if (p != last) beats = beats + 1;
        if (c == 0) begin errors = errors + 1; $display("FAIL a progress record without the cycles so far"); end
        last = p;
      end else if (s[7:0] == RUNNING) begin errors = errors + 1; $display("FAIL a progress record tagged %0h", s[63:8]); end
    end
    if (beats < 8) begin errors = errors + 1; $display("FAIL a 4096-line fill reported its progress %0d times", beats); end
    reg_is(STATUS, {56'h200, 8'h00}, "the long fill completes");
    reg_is(STATUS_SEQ, seq0 + 64'd1, "progress records leave the count of completed commands alone");

    // A fill whose channels never take a line reports no progress once its write FIFO is full, then completes once
    // they take its lines again.
    hold = 1;
    rd(STATUS_SEQ, seq0);
    wr(ARG0, 64'h1); wr(ARG1, 64'd0); wr(ARG2, {32'h98, 32'd256});
    wr(DOORBELL, {56'h201, 8'd1});
    repeat ((8 << HB) * DIVIDE) @(posedge uclk);
    rd(STATUS, s); rd(RESULT0, last); rd(RESULT1, c);
    beats = 0;
    for (k = 0; k < 64; k = k + 1) begin
      repeat ((1 << HB) * DIVIDE) @(posedge uclk);
      rd(STATUS, v); rd(RESULT0, p); rd(RESULT1, c2);
      if (v != s || p != last || c2 != c) beats = beats + 1;
    end
    if (beats != 0) begin errors = errors + 1; $display("FAIL a held fill reported progress %0d times", beats); end
    reg_is(STATUS_SEQ, seq0, "a held fill has not completed");
    hold = 0;
    k = 0;
    while (k < 400000) begin rd(STATUS_SEQ, v); if (v != seq0) k = 400000; else k = k + 1; end
    reg_is(STATUS, {56'h201, 8'h00}, "the held fill completes once its channels take lines again");

    $display("RESULT errors=%0d", errors);
    $finish;
  end
  initial begin #20000000000; $display("RESULT timeout errors=%0d", errors); $finish; end
endmodule
