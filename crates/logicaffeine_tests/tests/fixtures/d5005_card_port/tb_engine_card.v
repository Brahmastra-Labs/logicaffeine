// The decode engine with its memory on the card: the engine stepping in a user clock DIVIDE times its clock, which the
// card's divider makes, its port through the card port onto four memory channels on four unrelated clocks, each the
// shell's own channel path in front of a memory with random waits, preloaded from channel<b>.hex. The host's part as
// the engine's tables and store take it (jobs.hex, rows.hex, slots.hex, tens.hex, ropes.hex, blocks.hex, store.hex),
// then the step from word 0 to JOBS; afterwards the store to store.out, each beat read at the rising edge after its
// address, as the card's core reads it, and each channel's memory to out<b>.hex, and the fault, the card port's fault
// and the cycles on stdout.
`timescale 1ps/1ps
module tb;
  parameter ROWS = 1, JOBS = 1, SLOTS = 1, TENSORS = 1, ROPES = 1, LAYERS = 1, STORE = 1, LINES_LOG2 = 14, DUMP = 1,
            SPACE = 0, BASE = 0, GRANULE_LOG2 = 9, F16 = 0, INT8 = 0, TWO = 0, PERBLOCK = 0, DWM = 512, ECC = 0, DIVIDE = 6;
  localparam NB = 4, AW = 27, LW = 32, DL = 512, DLW = AW + LW + 1;
  reg fclk = 0, frst_n = 0, rst = 1;
  wire uclk, uclk_rst_n, fedge, fmid;
  logos_compute_clock #(.DIVIDE(DIVIDE)) cc(.clk(fclk), .rst_n(frst_n), .cclk(uclk), .cclk_rst_n(uclk_rst_n), .fedge(fedge),
    .fmid(fmid));
  initial #17 frst_n = 1;
  // A channel clock a register, each its own process: Verilator takes no edge of one bit of a vector.
  reg mem_clk0 = 0, mem_clk1 = 0, mem_clk2 = 0, mem_clk3 = 0;
  wire [NB-1:0] mem_clk = {mem_clk3, mem_clk2, mem_clk1, mem_clk0};
  always #185 fclk = ~fclk;
  always #1667 mem_clk0 = ~mem_clk0;
  initial begin #431 forever #1671 mem_clk1 = ~mem_clk1; end
  initial begin #977 forever #1663 mem_clk2 = ~mem_clk2; end
  initial begin #1301 forever #1669 mem_clk3 = ~mem_clk3; end
  wire rst_n = !rst;

  reg          start = 0, host_wr = 0;
  reg  [2:0]   host_table = 0;
  reg  [31:0]  host_addr = 0, host_raddr = 0;
  reg  [511:0] host_wdata = 0;
  wire [511:0] host_rdata;
  wire         busy, fault, port_fault;
  wire [31:0]  pc;
  wire         e_rd, e_rd_valid, e_wr, e_wr_ready;
  wire [31:0]  e_rd_line, e_wr_line;
  wire [511:0] e_rd_data, e_wr_data;
  wire [63:0]  e_wr_mask;
  logos_engine u(.clk(uclk), .rst(rst), .start(start), .job_at(32'd0), .job_end(JOBS[31:0]), .rows(ROWS[15:0]),
    .state_f16(F16[0]), .cache_int8(INT8[0]), .two_words(TWO[0]), .per_block(PERBLOCK[0]), .busy(busy), .fault(fault),
    .pc(pc), .host_wr(host_wr), .host_table(host_table), .host_addr(host_addr), .host_wdata(host_wdata),
    .host_raddr(host_raddr), .host_rdata(host_rdata), .mem_rd(e_rd), .mem_rd_line(e_rd_line),
    .mem_rd_valid(e_rd_valid), .mem_rd_data(e_rd_data), .mem_wr(e_wr), .mem_wr_line(e_wr_line),
    .mem_wr_data(e_wr_data), .mem_wr_mask(e_wr_mask), .mem_wr_ready(e_wr_ready), .fclk(fclk), .frst(!frst_n),
    .fedge(fedge), .fmid(fmid));

  wire [NB-1:0]     md_valid, md_ready, mr_valid, mr_ready, mw_valid, mw_ready, mdone_valid, mdone_ready;
  wire [NB*DLW-1:0] md_data;
  wire [NB*DL-1:0]  mr_data, mw_data;
  wire [NB*16-1:0]  mr_ecc;
  logos_card_port #(.NB(NB), .AW(AW), .LW(LW), .DL(DL)) port(.clk(uclk), .ce(1'b1), .rst_n(rst_n), .space(SPACE[31:0]),
    .base(BASE[AW-1:0]), .granule_log2(GRANULE_LOG2[4:0]), .clear(1'b0), .fault(port_fault), .mem_rd(e_rd),
    .mem_rd_line(e_rd_line), .mem_rd_valid(e_rd_valid), .mem_rd_data(e_rd_data), .mem_wr(e_wr),
    .mem_wr_line(e_wr_line), .mem_wr_data(e_wr_data), .mem_wr_mask(e_wr_mask), .mem_wr_ready(e_wr_ready),
    .md_valid(md_valid), .md_ready(md_ready), .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready),
    .mr_data(mr_data), .mr_ecc(mr_ecc), .mw_valid(mw_valid), .mw_ready(mw_ready), .mw_data(mw_data),
    .mdone_valid(mdone_valid), .mdone_ready(mdone_ready));

  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : ch
    wire [AW-1:0]    address;
    wire             read, write, waitrequest, readdatavalid;
    wire [6:0]       burstcount;
    wire [DWM-1:0]   writedata, readdata;
    wire [DWM/8-1:0] byteenable;
    logos_d5005_channel #(.AW(AW), .DWM(DWM), .ECC(ECC), .MBW(7), .LW(LW), .MAXB(16), .RAW(6), .WAW(4)) channel(
      .uclk(uclk), .uce(1'b1), .uclk_rst_n(rst_n), .mem_clk(mem_clk[b]), .mem_rst_n(rst_n),
      .md_valid(md_valid[b]), .md_ready(md_ready[b]), .md_data(md_data[b*DLW +: DLW]),
      .mr_valid(mr_valid[b]), .mr_ready(mr_ready[b]), .mr_data(mr_data[b*DL +: DL]), .mr_ecc(mr_ecc[b*16 +: 16]),
      .mw_valid(mw_valid[b]), .mw_ready(mw_ready[b]), .mw_data(mw_data[b*DL +: DL]), .mw_flip(21'h0),
      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]), .mem_address(address), .mem_read(read),
      .mem_write(write), .mem_burstcount(burstcount), .mem_writedata(writedata), .mem_byteenable(byteenable),
      .mem_waitrequest(waitrequest), .mem_readdata(readdata), .mem_readdatavalid(readdatavalid));
    logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(7), .LINES_LOG2(LINES_LOG2), .SEED(50 + b)) memory(
      .clk(mem_clk[b]), .rst_n(rst_n), .address(address), .read(read), .write(write), .burstcount(burstcount),
      .writedata(writedata), .byteenable(byteenable), .waitrequest(waitrequest), .readdata(readdata),
      .readdatavalid(readdatavalid));
  end endgenerate

  reg [63:0]  t_jobs   [0:JOBS-1];
  reg [127:0] t_rows   [0:ROWS-1];
  reg [63:0]  t_slots  [0:SLOTS-1];
  reg [127:0] t_tens   [0:TENSORS-1];
  reg [63:0]  t_ropes  [0:ROPES-1];
  reg [511:0] t_blocks [0:LAYERS-1];
  reg [511:0] t_store  [0:STORE-1];
  task put(input [2:0] t, input [31:0] a, input [511:0] d);
    begin
      @(negedge uclk);
      host_wr = 1'b1; host_table = t; host_addr = a; host_wdata = d;
      @(negedge uclk);
      host_wr = 1'b0;
    end
  endtask
  integer i, fo, cycles;
  initial begin
    $readmemh("jobs.hex", t_jobs);
    $readmemh("rows.hex", t_rows);
    $readmemh("slots.hex", t_slots);
    $readmemh("tens.hex", t_tens);
    $readmemh("ropes.hex", t_ropes);
    $readmemh("blocks.hex", t_blocks);
    $readmemh("store.hex", t_store);
    #1;
    $readmemh("channel0.hex", ch[0].memory.mem, 0, DUMP - 1);
    $readmemh("channel1.hex", ch[1].memory.mem, 0, DUMP - 1);
    $readmemh("channel2.hex", ch[2].memory.mem, 0, DUMP - 1);
    $readmemh("channel3.hex", ch[3].memory.mem, 0, DUMP - 1);
    #20000;
    @(negedge uclk); rst = 1'b0;
    for (i = 0; i < JOBS; i = i + 1) put(3'd0, i, {448'd0, t_jobs[i]});
    for (i = 0; i < ROWS; i = i + 1) put(3'd1, i, {384'd0, t_rows[i]});
    for (i = 0; i < SLOTS; i = i + 1) put(3'd2, i, {448'd0, t_slots[i]});
    for (i = 0; i < TENSORS; i = i + 1) put(3'd3, i, {384'd0, t_tens[i]});
    for (i = 0; i < ROPES; i = i + 1) put(3'd4, i, {448'd0, t_ropes[i]});
    for (i = 0; i < LAYERS; i = i + 1) put(3'd5, i, t_blocks[i]);
    for (i = 0; i < STORE; i = i + 1) put(3'd6, i, t_store[i]);
    // The start at a rising edge, as the shell's core gives it: the engine takes it at the next and its busy shows by
    // the one after.
    @(posedge uclk); start <= 1'b1;
    @(posedge uclk); start <= 1'b0;
    @(posedge uclk);
    cycles = 0;
    while (busy && cycles < 400000000) begin @(negedge uclk); cycles = cycles + 1; end
    repeat (50) @(negedge uclk);
    if (fault) $display("FAULT at word %0d", pc);
    if (port_fault) $display("PORT FAULT");
    if (busy) $display("TIMEOUT");
    $display("CYCLES %0d", cycles);
    fo = $fopen("store.out", "w");
    for (i = 0; i < STORE; i = i + 1) begin
      host_raddr = i;
      @(posedge uclk);
      #1;
      $fwrite(fo, "%h\n", host_rdata);
    end
    $fclose(fo);
    $writememh("out0.hex", ch[0].memory.mem, 0, DUMP - 1);
    $writememh("out1.hex", ch[1].memory.mem, 0, DUMP - 1);
    $writememh("out2.hex", ch[2].memory.mem, 0, DUMP - 1);
    $writememh("out3.hex", ch[3].memory.mem, 0, DUMP - 1);
    $finish;
  end
endmodule
