// hart_bringup_top — the board's computer alone on the Nexys A7-100T, for bring-up: the hart (hart_soc) over the DDR2
// (hart_ddr2 + hart_ddr2_phy), booting from the configuration flash it reads through STARTUPE2, with its log on the
// USB UART — transmit only; the image cannot be sent anything. The terminal is not here: the link's bytes from the
// hart are drained and none arrive.
//
// Clocks from one PLL off the 100 MHz oscillator: CK 125 MHz (and a quarter CK later for DQ), the hart's 62.5 MHz,
// and 200 MHz for the input delays' reference. LEDs: 15 the PLL locked, 14 the DDR2 calibrated, 13 its calibration
// failed, 12 the hart stopped; 11..0 the hart's own LED register.
`include "soc_defines.vh"
module hart_bringup_top #(
  parameter [63:0] RTL_MD5_LO = 64'd0,
  parameter [63:0] RTL_MD5_HI = 64'd0,
  parameter        ROM_INIT = "boot_rom.hex"
) (
  input         clk,
  output        uart_tx,
  output [15:0] led,
  input  [15:0] sw,
  input         btnc,
  input         btnu,
  input         btnl,
  input         btnr,
  input         btnd,
  output        qspi_cs_n,
  output        qspi_dq0,
  input         qspi_dq1,
  output        qspi_dq2,
  output        qspi_dq3,
  output        ddr2_ck_p,
  output        ddr2_ck_n,
  output        ddr2_cke,
  output        ddr2_cs_n,
  output        ddr2_ras_n,
  output        ddr2_cas_n,
  output        ddr2_we_n,
  output [2:0]  ddr2_ba,
  output [12:0] ddr2_addr,
  output        ddr2_odt,
  output [1:0]  ddr2_dm,
  inout  [15:0] ddr2_dq,
  inout  [1:0]  ddr2_dqs_p,
  inout  [1:0]  ddr2_dqs_n
);
  // ── clocks and reset ───────────────────────────────────────────────────────────────────────────────────────
  wire fb, c125_raw, c125q_raw, c62_raw, c200_raw, locked;
  wire c125, c125q, c62, c200;
  PLLE2_BASE #(
    .CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT(10), .DIVCLK_DIVIDE(1),
    .CLKOUT0_DIVIDE(8), .CLKOUT1_DIVIDE(8), .CLKOUT1_PHASE(90.0), .CLKOUT2_DIVIDE(16), .CLKOUT3_DIVIDE(5)
  ) pll (
    .CLKIN1(clk), .CLKFBIN(fb), .CLKFBOUT(fb), .CLKOUT0(c125_raw), .CLKOUT1(c125q_raw), .CLKOUT2(c62_raw),
    .CLKOUT3(c200_raw), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0));
  BUFG b125 (.I(c125_raw), .O(c125));
  BUFG b125q (.I(c125q_raw), .O(c125q));
  BUFG b62 (.I(c62_raw), .O(c62));
  BUFG b200 (.I(c200_raw), .O(c200));

  // The hart's domain leaves reset once the PLL and the input delays are steady.
  wire idelay_ready;
  reg [1:0] lock_sync = 2'b00, ready_sync = 2'b00;
  reg [7:0] por = 8'd0;
  always @(posedge c62) begin
    lock_sync <= {lock_sync[0], locked};
    ready_sync <= {ready_sync[0], idelay_ready};
    if (!lock_sync[1] || !ready_sync[1]) por <= 8'd0;
    else if (!por[7]) por <= por + 8'd1;
  end
  wire rst = !por[7];
  reg phy_rst = 1'b1;
  always @(posedge c62) phy_rst <= !lock_sync[1];

  // ── the DDR2 ───────────────────────────────────────────────────────────────────────────────────────────────
  wire        mem_req, mem_we, mem_gnt, mem_wvalid, mem_wready, mem_rvalid;
  wire [63:0] mem_addr, mem_wdata, mem_rdata, ddr_status, ddr_cal;
  wire        cke, cs_n, ras_n, cas_n, we_n, dq_oe, dqs_oe, dqs_toggle, dly_ld;
  wire [2:0]  ba;
  wire [12:0] a;
  wire [63:0] dq_out, rd_frame;
  wire [9:0]  dly_tap;
  hart_ddr2 ddr2 (
    .clk(c62), .rst(rst), .req(mem_req), .we(mem_we), .addr(mem_addr), .gnt(mem_gnt), .wdata(mem_wdata),
    .wvalid(mem_wvalid), .wready(mem_wready), .rvalid(mem_rvalid), .rdata(mem_rdata), .status(ddr_status),
    .cal(ddr_cal), .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a),
    .dq_oe(dq_oe), .dq_out(dq_out), .dqs_oe(dqs_oe), .dqs_toggle(dqs_toggle), .rd_frame(rd_frame),
    .dly_ld(dly_ld), .dly_tap(dly_tap));
  hart_ddr2_phy phy (
    .c125(c125), .c125q(c125q), .c62(c62), .c200(c200), .rst(phy_rst), .idelay_ready(idelay_ready),
    .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a), .dq_oe(dq_oe),
    .dq_out(dq_out), .dqs_oe(dqs_oe), .dqs_toggle(dqs_toggle), .rd_frame(rd_frame), .dly_ld(dly_ld),
    .dly_tap(dly_tap), .ddr2_ck_p(ddr2_ck_p), .ddr2_ck_n(ddr2_ck_n), .ddr2_cke(ddr2_cke), .ddr2_cs_n(ddr2_cs_n),
    .ddr2_ras_n(ddr2_ras_n), .ddr2_cas_n(ddr2_cas_n), .ddr2_we_n(ddr2_we_n), .ddr2_ba(ddr2_ba),
    .ddr2_addr(ddr2_addr), .ddr2_odt(ddr2_odt), .ddr2_dm(ddr2_dm), .ddr2_dq(ddr2_dq), .ddr2_dqs_p(ddr2_dqs_p),
    .ddr2_dqs_n(ddr2_dqs_n));

  // ── the buttons, steady for 10 ms before they count ────────────────────────────────────────────────────────
  wire [4:0] raw = {btnd, btnr, btnl, btnu, btnc};
  reg  [4:0] b1 = 5'd0, b2 = 5'd0, buttons = 5'd0;
  reg  [19:0] steady = 20'd0;
  always @(posedge c62) begin
    b1 <= raw;
    b2 <= b1;
    if (b2 != buttons) begin
      steady <= steady + 20'd1;
      if (steady == 20'd625_000) begin
        buttons <= b2;
        steady <= 20'd0;
      end
    end else steady <= 20'd0;
  end

  // ── the computer ───────────────────────────────────────────────────────────────────────────────────────────
  wire [15:0] soc_leds;
  wire [7:0]  log_byte, link_out_byte;
  wire        log_valid, log_ready, link_out_valid, link_in_ready, qspi_sck, exited;
  wire [63:0] exit_status;
  hart_soc #(.RTL_MD5_LO(RTL_MD5_LO), .RTL_MD5_HI(RTL_MD5_HI), .ROM_INIT(ROM_INIT)) soc (
    .clk(c62), .rst(rst),
    .mem_req(mem_req), .mem_we(mem_we), .mem_addr(mem_addr), .mem_gnt(mem_gnt), .mem_wdata(mem_wdata),
    .mem_wvalid(mem_wvalid), .mem_wready(mem_wready), .mem_rvalid(mem_rvalid), .mem_rdata(mem_rdata),
    .ddr_status(ddr_status), .ddr_cal(ddr_cal),
    .switches(sw), .buttons(buttons), .leds(soc_leds),
    .log_byte(log_byte), .log_valid(log_valid), .log_ready(log_ready),
    .link_out_byte(link_out_byte), .link_out_valid(link_out_valid), .link_out_ready(1'b1),
    .link_in_byte(8'd0), .link_in_valid(1'b0), .link_in_ready(link_in_ready),
    .qspi_sck(qspi_sck), .qspi_cs_n(qspi_cs_n), .qspi_mosi(qspi_dq0), .qspi_miso(qspi_dq1),
    .exited(exited), .exit_status(exit_status));
  hart_uart_tx #(.CLKS_PER_BIT(`SOC_HART_HZ / 115200)) log (
    .clk(c62), .rst(rst), .byte_in(log_byte), .valid(log_valid), .ready(log_ready), .tx(uart_tx));

  // The flash's clock pin is the configuration clock, the design's once STARTUPE2 hands it over; write protect and
  // hold stay high.
  STARTUPE2 #(.PROG_USR("FALSE"), .SIM_CCLK_FREQ(0.0)) startup (
    .CLK(1'b0), .GSR(1'b0), .GTS(1'b0), .KEYCLEARB(1'b1), .PACK(1'b0),
    .USRCCLKO(qspi_sck), .USRCCLKTS(1'b0), .USRDONEO(1'b1), .USRDONETS(1'b1));
  assign qspi_dq2 = 1'b1;
  assign qspi_dq3 = 1'b1;

  assign led = {locked, ddr_status[0], ddr_status[1], exited, soc_leds[11:0]};
endmodule
