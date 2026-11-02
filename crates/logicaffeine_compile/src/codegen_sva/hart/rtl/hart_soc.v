// hart_soc — the board's computer, without its board: the hart with its caches (hart_cpu) over a line port to the
// DDR2, the boot ROM and scratch RAM in block RAM, and the device registers, on the memory map `hart::soc` defines
// (soc_defines.vh). The board's top wires the line port to the DDR2 controller, the flash pins to the configuration
// flash, and the byte streams to the log UART and the terminal; a testbench wires them to models.
`include "soc_defines.vh"
module hart_soc #(
  parameter [63:0] RTL_MD5_LO = 64'd0,
  parameter [63:0] RTL_MD5_HI = 64'd0,
  parameter        ROM_INIT = "boot_rom.hex",
  parameter        QSPI_HALF = 2,
  parameter        CACHE_INDEX_BITS = 8
) (
  input             clk,
  input             rst,
  // the DDR2's line port
  output            mem_req,
  output            mem_we,
  output     [63:0] mem_addr,
  input             mem_gnt,
  output     [63:0] mem_wdata,
  output            mem_wvalid,
  input             mem_wready,
  input             mem_rvalid,
  input      [63:0] mem_rdata,
  input      [63:0] ddr_status,
  input      [63:0] ddr_cal,
  // the board
  input      [15:0] switches,
  input      [4:0]  buttons,
  output     [15:0] leds,
  output     [7:0]  log_byte,
  output            log_valid,
  input             log_ready,
  output     [7:0]  link_out_byte,
  output            link_out_valid,
  input             link_out_ready,
  input      [7:0]  link_in_byte,
  input             link_in_valid,
  output            link_in_ready,
  output            qspi_sck,
  output            qspi_cs_n,
  output            qspi_mosi,
  input             qspi_miso,
  output            exited,
  output     [63:0] exit_status
);
  wire        pre_valid, lp_req, lp_we, lp_ready, rom_f_en, restart;
  wire [63:0] pre_addr, lp_addr, lp_wdata, lp_rdata, rom_f_index, rom_f_word, rom_d_word, sram_rdata, io_rdata;
  wire [7:0]  lp_be;
  wire        io_ready;
  hart_cpu #(
    .ROM_BASE(`SOC_ROM_BASE), .ROM_SIZE(`SOC_ROM_SIZE), .SRAM_BASE(`SOC_SRAM_BASE), .SRAM_SIZE(`SOC_SRAM_SIZE),
    .IO_BASE(`SOC_IO_BASE), .IO_SIZE(`SOC_IO_SIZE), .RAM_BASE(`SOC_RAM_BASE), .RAM_SIZE(`SOC_RAM_SIZE),
    .CACHE_INDEX_BITS(CACHE_INDEX_BITS)
  ) cpu (
    .clk(clk), .rst(rst), .reset_pc(`SOC_RESET_PC), .restart(restart), .stop(exited),
    .pre_valid(pre_valid), .pre_addr(pre_addr), .lp_req(lp_req), .lp_addr(lp_addr), .lp_we(lp_we), .lp_be(lp_be),
    .lp_wdata(lp_wdata), .lp_ready(lp_ready), .lp_rdata(lp_rdata),
    .rom_en(rom_f_en), .rom_index(rom_f_index), .rom_word(rom_f_word),
    .mem_req(mem_req), .mem_we(mem_we), .mem_addr(mem_addr), .mem_gnt(mem_gnt), .mem_wdata(mem_wdata),
    .mem_wvalid(mem_wvalid), .mem_wready(mem_wready), .mem_rvalid(mem_rvalid), .mem_rdata(mem_rdata));

  // The local port's access is in exactly one of the ROM, the scratch RAM and the devices: E refused the rest.
  wire at_io  = lp_addr >= `SOC_IO_BASE && lp_addr < `SOC_IO_BASE + `SOC_IO_SIZE;
  wire at_rom = lp_addr >= `SOC_ROM_BASE && lp_addr < `SOC_ROM_BASE + `SOC_ROM_SIZE;
  wire [63:0] pre_rom = pre_addr - `SOC_ROM_BASE;
  wire [63:0] pre_sram = pre_addr - `SOC_SRAM_BASE;
  wire [63:0] lp_sram = lp_addr - `SOC_SRAM_BASE;

  hart_rom #(.WORDS(`SOC_ROM_SIZE / 8), .INIT(ROM_INIT)) rom (
    .clk(clk), .f_en(rom_f_en), .f_index(rom_f_index), .f_word(rom_f_word),
    .d_en(pre_valid), .d_index({3'd0, pre_rom[63:3]}), .d_word(rom_d_word));
  hart_sram #(.WORDS(`SOC_SRAM_SIZE / 8)) sram (
    .clk(clk), .rst(rst), .pre_valid(pre_valid), .pre_index({3'd0, pre_sram[63:3]}),
    .req(lp_req && !at_io && !at_rom), .index({3'd0, lp_sram[63:3]}), .we(lp_we), .be(lp_be),
    .wdata(lp_wdata), .rdata(sram_rdata));
  hart_io #(.RTL_MD5_LO(RTL_MD5_LO), .RTL_MD5_HI(RTL_MD5_HI), .QSPI_HALF(QSPI_HALF)) io (
    .clk(clk), .rst(rst), .req(lp_req && at_io), .addr(lp_addr), .we(lp_we), .wdata(lp_wdata),
    .ready(io_ready), .rdata(io_rdata), .switches(switches), .buttons(buttons), .leds(leds),
    .ddr_status(ddr_status), .ddr_cal(ddr_cal),
    .log_byte(log_byte), .log_valid(log_valid), .log_ready(log_ready),
    .link_out_byte(link_out_byte), .link_out_valid(link_out_valid), .link_out_ready(link_out_ready),
    .link_in_byte(link_in_byte), .link_in_valid(link_in_valid), .link_in_ready(link_in_ready),
    .qspi_sck(qspi_sck), .qspi_cs_n(qspi_cs_n), .qspi_mosi(qspi_mosi), .qspi_miso(qspi_miso),
    .exited(exited), .exit_status(exit_status), .restart(restart));

  assign lp_ready = at_io ? io_ready : 1'b1;
  assign lp_rdata = at_io ? io_rdata : at_rom ? rom_d_word : sram_rdata;
endmodule
