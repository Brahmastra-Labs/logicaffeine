// ft245_uart_bridge — adapts the FT2232H ASYNC FIFO (Nexys Video DPTI channel) to a plain serial UART port,
// so the UNMODIFIED wpe_soc_fpga engine (which speaks uart_rx/uart_tx at CLKS_PER_BIT) runs on Board A with
// the host streaming bytes over /dev/ttyUSB exactly as on Board B. Host byte -> FTDI FIFO -> we FT245-read it
// -> serialize onto eng_rx (feeds the engine's UART RX). Engine's uart_tx -> we deserialize -> FT245-write to
// the FTDI -> host reads it. The internal serial link runs at CLKS_PER_BIT (same clock as the engine); the
// FTDI's 4 KB FIFO buffers the fast host side against the slow internal baud.
module ft245_uart_bridge #(parameter CLKS_PER_BIT = 81, parameter W = 4) (
  input  clk, input rst,
  output reg eng_rx = 1'b1,     // serial into the engine's uart_rx
  input      eng_tx,            // serial from the engine's uart_tx
  inout [7:0] prog_d,
  input  prog_rxen,             // RXF# (low = FIFO has a byte for us)
  input  prog_txen,             // TXE# (low = FIFO has room for us)
  output reg prog_rdn = 1'b1,
  output reg prog_wrn = 1'b1,
  output reg prog_oen = 1'b1,
  output prog_siwun,
  output prog_spien
);
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;     // DPTI (parallel FIFO) select

  reg rxf1,rxf2,txe1,txe2;
  always @(posedge clk) begin rxf1<=prog_rxen; rxf2<=rxf1; txe1<=prog_txen; txe2<=txe1; end
  wire rx_avail = ~rxf2;
  wire tx_space = ~txe2;

  // ---- feeder UART: serialize a byte onto eng_rx (into the engine) ----
  localparam F_IDLE=2'd0,F_START=2'd1,F_DATA=2'd2,F_STOP=2'd3;
  reg [1:0] fs = F_IDLE; reg [8:0] fc; reg [2:0] fbit; reg [7:0] fsh;
  reg feed_start; reg [7:0] feed_data;
  wire feed_busy = (fs != F_IDLE) || feed_start;
  always @(posedge clk) begin
    if (rst) begin fs<=F_IDLE; eng_rx<=1'b1; fc<=0; fbit<=0; end
    else case (fs)
      F_IDLE:  begin eng_rx<=1'b1; if (feed_start) begin fsh<=feed_data; fs<=F_START; fc<=0; eng_rx<=1'b0; end end
      F_START: if (fc==CLKS_PER_BIT-1) begin fc<=0; fbit<=0; fs<=F_DATA; eng_rx<=fsh[0]; end else fc<=fc+1'b1;
      F_DATA:  if (fc==CLKS_PER_BIT-1) begin fc<=0; if (fbit==3'd7) begin fs<=F_STOP; eng_rx<=1'b1; end else begin fbit<=fbit+1'b1; eng_rx<=fsh[fbit+1]; end end else fc<=fc+1'b1;
      F_STOP:  if (fc==CLKS_PER_BIT-1) begin fc<=0; fs<=F_IDLE; eng_rx<=1'b1; end else fc<=fc+1'b1;
    endcase
  end

  // ---- reader UART: deserialize eng_tx (from the engine) into bytes ----
  localparam G_IDLE=2'd0,G_START=2'd1,G_DATA=2'd2,G_STOP=2'd3;
  reg [1:0] gs = G_IDLE; reg [8:0] gc; reg [2:0] gbit; reg [7:0] gsh; reg gd1,gd2;
  reg read_valid; reg [7:0] read_data;
  always @(posedge clk) begin
    gd1<=eng_tx; gd2<=gd1; read_valid<=1'b0;
    if (rst) begin gs<=G_IDLE; gc<=0; gbit<=0; end
    else case (gs)
      G_IDLE:  if (~gd2) begin gs<=G_START; gc<=0; end
      G_START: if (gc==(CLKS_PER_BIT-1)/2) begin if (~gd2) begin gc<=0; gbit<=0; gs<=G_DATA; end else gs<=G_IDLE; end else gc<=gc+1'b1;
      G_DATA:  if (gc==CLKS_PER_BIT-1) begin gc<=0; gsh<={gd2,gsh[7:1]}; if (gbit==3'd7) gs<=G_STOP; else gbit<=gbit+1'b1; end else gc<=gc+1'b1;
      G_STOP:  if (gc==CLKS_PER_BIT-1) begin gc<=0; read_valid<=1'b1; read_data<=gsh; gs<=G_IDLE; end else gc<=gc+1'b1;
    endcase
  end

  // ---- FT245 async-FIFO master: read host->engine, write engine->host ----
  localparam A_IDLE=3'd0,A_RD_OE=3'd1,A_RD_LOW=3'd2,A_RD_CAP=3'd3,A_RD_HI=3'd4,A_WR_LOW=3'd5,A_WR_HI=3'd6;
  reg [2:0] ast = A_IDLE; reg [3:0] acnt;
  reg drive; reg [7:0] dbus;
  assign prog_d = drive ? dbus : 8'bzzzzzzzz;
  reg wpend; reg [7:0] wdata;
  always @(posedge clk) begin
    feed_start <= 1'b0;
    if (rst) begin ast<=A_IDLE; prog_rdn<=1; prog_wrn<=1; prog_oen<=1; drive<=0; acnt<=0; wpend<=0; end
    else begin
      if (read_valid) begin wpend<=1'b1; wdata<=read_data; end   // queue an engine byte for the host (write wins)
      case (ast)
        A_IDLE: begin
          prog_rdn<=1; prog_wrn<=1; prog_oen<=1; drive<=0; acnt<=0;
          if (wpend && tx_space) begin dbus<=wdata; drive<=1'b1; ast<=A_WR_LOW; end
          else if (rx_avail && !feed_busy) begin prog_oen<=1'b0; ast<=A_RD_OE; end
        end
        A_RD_OE:  begin prog_oen<=1'b0; if (acnt==W) begin acnt<=0; prog_rdn<=1'b0; ast<=A_RD_LOW; end else acnt<=acnt+1'b1; end
        A_RD_LOW: begin if (acnt==W) begin acnt<=0; ast<=A_RD_CAP; end else acnt<=acnt+1'b1; end
        A_RD_CAP: begin feed_data<=prog_d; feed_start<=1'b1; prog_rdn<=1'b1; prog_oen<=1'b1; acnt<=0; ast<=A_RD_HI; end
        A_RD_HI:  begin if (acnt==W) begin acnt<=0; ast<=A_IDLE; end else acnt<=acnt+1'b1; end
        A_WR_LOW: begin prog_wrn<=1'b0; if (acnt==W) begin acnt<=0; ast<=A_WR_HI; end else acnt<=acnt+1'b1; end
        A_WR_HI:  begin prog_wrn<=1'b1; if (acnt==W) begin acnt<=0; drive<=1'b0; if (!read_valid) wpend<=1'b0; ast<=A_IDLE; end else acnt<=acnt+1'b1; end
        default:  ast<=A_IDLE;
      endcase
    end
  end
endmodule
