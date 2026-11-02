// hart_cpu — the hart with its caches: the core, the instruction and data caches over one line port to memory, and
// the maintenance they share. Accesses to the RAM go through the data cache; every other data access leaves on the
// local port, which the SoC answers from the boot ROM, the scratch RAM and the devices (the port's pre-read edge is
// the core's, so a block RAM behind it reads as the cache does). Instruction fetches outside the RAM read the ROM
// through its fetch port.
//
// fence.i asks the data cache to write every dirty line back, then the instruction cache to forget every line, and
// finishes when both have. `restart` holds the hart, does the same, and lets the hart go from its reset vector with
// memory as the program left it; `stop` holds the hart for good. The device whose store asks for either answers it
// in one cycle and the core lets nothing younger than a device access leave E, so the hart does nothing after it.
module hart_cpu #(
  parameter [63:0] ROM_BASE = 64'h0, parameter [63:0] ROM_SIZE = 64'h0,
  parameter [63:0] SRAM_BASE = 64'h0, parameter [63:0] SRAM_SIZE = 64'h0,
  parameter [63:0] IO_BASE = 64'h0, parameter [63:0] IO_SIZE = 64'h0,
  parameter [63:0] RAM_BASE = 64'h8000_0000, parameter [63:0] RAM_SIZE = 64'h800_0000,
  parameter        CACHE_INDEX_BITS = 8
) (
  input             clk,
  input             rst,
  input      [63:0] reset_pc,
  input             restart,
  input             stop,
  // the local port: every data access outside the RAM
  output            pre_valid,
  output     [63:0] pre_addr,
  output            lp_req,
  output     [63:0] lp_addr,
  output            lp_we,
  output     [7:0]  lp_be,
  output     [63:0] lp_wdata,
  input             lp_ready,
  input      [63:0] lp_rdata,
  // the boot ROM's fetch port
  output            rom_en,
  output     [63:0] rom_index,
  input      [63:0] rom_word,
  // the line port to memory
  output            mem_req,
  output            mem_we,
  output     [63:0] mem_addr,
  input             mem_gnt,
  output     [63:0] mem_wdata,
  output            mem_wvalid,
  input             mem_wready,
  input             mem_rvalid,
  input      [63:0] mem_rdata
);
  localparam RAM_BITS = $clog2(RAM_SIZE);

  wire        hold;
  wire        if_req, if_ready;
  wire [63:0] if_addr, if_rdata;
  wire        dm_req, dm_we, dm_ready, cm_req;
  wire [63:0] dm_addr, dm_wdata, dm_rdata;
  wire [7:0]  dm_be;
  reg         cm_done;
  hart_core #(
    .ROM_BASE(ROM_BASE), .ROM_SIZE(ROM_SIZE), .SRAM_BASE(SRAM_BASE), .SRAM_SIZE(SRAM_SIZE),
    .IO_BASE(IO_BASE), .IO_SIZE(IO_SIZE), .RAM_BASE(RAM_BASE), .RAM_SIZE(RAM_SIZE)
  ) core (
    .clk(clk), .rst(rst || hold), .reset_pc(reset_pc),
    .if_req(if_req), .if_addr(if_addr), .if_ready(if_ready), .if_rdata(if_rdata),
    .dm_pre_valid(pre_valid), .dm_pre_addr(pre_addr), .dm_req(dm_req), .dm_addr(dm_addr), .dm_we(dm_we),
    .dm_be(dm_be), .dm_wdata(dm_wdata), .dm_ready(dm_ready), .dm_rdata(dm_rdata), .cm_req(cm_req), .cm_done(cm_done));

  wire at_ram = dm_addr >= RAM_BASE && dm_addr < RAM_BASE + RAM_SIZE;
  assign lp_req = dm_req && !at_ram;
  assign lp_addr = dm_addr;
  assign lp_we = dm_we;
  assign lp_be = dm_be;
  assign lp_wdata = dm_wdata;

  wire        ic_fill_req, ic_fill_gnt, ic_fill_valid, ic_invalidate, ic_invalidated;
  wire [63:0] ic_fill_addr, ic_fill_data;
  hart_icache #(
    .RAM_BASE(RAM_BASE), .RAM_BITS(RAM_BITS), .INDEX_BITS(CACHE_INDEX_BITS), .ROM_BASE(ROM_BASE), .ROM_SIZE(ROM_SIZE)
  ) icache (
    .clk(clk), .rst(rst), .if_req(if_req), .if_addr(if_addr), .if_ready(if_ready), .if_rdata(if_rdata),
    .rom_en(rom_en), .rom_index(rom_index), .rom_word(rom_word),
    .fill_req(ic_fill_req), .fill_addr(ic_fill_addr), .fill_gnt(ic_fill_gnt), .fill_valid(ic_fill_valid),
    .fill_data(ic_fill_data), .invalidate(ic_invalidate), .invalidated(ic_invalidated));

  wire        dc_ready, dc_mem_req, dc_mem_we, dc_mem_gnt, dc_mem_wvalid, dc_mem_wready, dc_mem_rvalid;
  wire        dc_flush, dc_flushed;
  wire [63:0] dc_rdata, dc_mem_addr, dc_mem_wdata, dc_mem_rdata;
  hart_dcache #(.RAM_BASE(RAM_BASE), .RAM_BITS(RAM_BITS), .INDEX_BITS(CACHE_INDEX_BITS)) dcache (
    .clk(clk), .rst(rst), .pre_valid(pre_valid), .pre_addr(pre_addr),
    .req(dm_req && at_ram), .addr(dm_addr), .we(dm_we), .be(dm_be), .wdata(dm_wdata),
    .ready(dc_ready), .rdata(dc_rdata),
    .mem_req(dc_mem_req), .mem_we(dc_mem_we), .mem_addr(dc_mem_addr), .mem_gnt(dc_mem_gnt),
    .mem_wdata(dc_mem_wdata), .mem_wvalid(dc_mem_wvalid), .mem_wready(dc_mem_wready),
    .mem_rvalid(dc_mem_rvalid), .mem_rdata(dc_mem_rdata), .flush(dc_flush), .flushed(dc_flushed));

  assign dm_ready = at_ram ? dc_ready : lp_ready;
  assign dm_rdata = at_ram ? dc_rdata : lp_rdata;

  hart_mem lines (
    .clk(clk), .rst(rst),
    .i_req(ic_fill_req), .i_addr(ic_fill_addr), .i_gnt(ic_fill_gnt), .i_valid(ic_fill_valid), .i_data(ic_fill_data),
    .d_req(dc_mem_req), .d_we(dc_mem_we), .d_addr(dc_mem_addr), .d_gnt(dc_mem_gnt), .d_wdata(dc_mem_wdata),
    .d_wvalid(dc_mem_wvalid), .d_wready(dc_mem_wready), .d_rvalid(dc_mem_rvalid), .d_rdata(dc_mem_rdata),
    .m_req(mem_req), .m_we(mem_we), .m_addr(mem_addr), .m_gnt(mem_gnt), .m_wdata(mem_wdata),
    .m_wvalid(mem_wvalid), .m_wready(mem_wready), .m_rvalid(mem_rvalid), .m_rdata(mem_rdata));

  // ── maintenance: fence.i and restart ───────────────────────────────────────────────────────────────────────
  localparam [1:0] K_IDLE = 2'd0, K_FLUSH = 2'd1, K_FORGET = 2'd2;
  reg [1:0] keep;
  reg       restarting;
  reg       stopped;
  assign dc_flush = keep == K_FLUSH;
  assign ic_invalidate = keep == K_FORGET;
  assign hold = restarting || stopped;
  always @(posedge clk) begin
    if (rst) begin
      keep <= K_IDLE;
      restarting <= 1'b0;
      stopped <= 1'b0;
      cm_done <= 1'b0;
    end else begin
      cm_done <= 1'b0;
      if (stop) stopped <= 1'b1;
      case (keep)
        K_IDLE: begin
          if (restart) begin
            restarting <= 1'b1;
            keep <= K_FLUSH;
          end else if (cm_req) keep <= K_FLUSH;
        end
        K_FLUSH: if (dc_flushed) keep <= K_FORGET;
        default: if (ic_invalidated) begin
          keep <= K_IDLE;
          if (restarting) restarting <= 1'b0;
          else cm_done <= 1'b1;
        end
      endcase
    end
  end

`ifdef HART_TRACE
  // A restart begins the commit trace's numbering again; it is marked once the hart is held, after the restarting
  // store's own records.
  reg restart_mark;
  always @(posedge clk) begin
    restart_mark <= !rst && restart && keep == K_IDLE;
    if (restart_mark) $display("R");
  end
`endif
endmodule
