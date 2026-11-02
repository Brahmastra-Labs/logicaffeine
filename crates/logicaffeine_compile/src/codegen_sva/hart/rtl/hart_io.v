// hart_io — the board's device registers, at the addresses `hart::soc` defines (soc_defines.vh) and with the behaviour
// its register table documents: identity, clock, the bitstream's fingerprint, switches, buttons and LEDs, exit and
// restart; the log UART and the terminal link as byte queues; the flash's streaming port; the DDR2's calibration.
//
// An access is taken in one cycle and answered the next, exactly once: a read that takes a byte or a word takes it as
// the access is taken. A store into a full queue, and a read of the flash's stream before its next word has arrived,
// wait. An address no register holds reads as zero and ignores writes.
`include "soc_defines.vh"
module hart_io #(
  parameter [63:0] RTL_MD5_LO = 64'd0,
  parameter [63:0] RTL_MD5_HI = 64'd0,
  parameter        QSPI_HALF = 2
) (
  input             clk,
  input             rst,
  input             req,
  input      [63:0] addr,
  input             we,
  input      [63:0] wdata,
  output            ready,
  output reg [63:0] rdata,
  input      [15:0] switches,
  input      [4:0]  buttons,
  output reg [15:0] leds,
  input      [63:0] ddr_status,
  input      [63:0] ddr_cal,
  // the log UART's bytes, and the terminal link's bytes each way
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
  output reg        exited,
  output reg [63:0] exit_status,
  output reg        restart
);
  reg [63:0] cycles;
  reg        ack;
  reg        stream_on;
  assign ready = ack;

  wire log_empty, log_full, lo_empty, lo_full, li_empty, li_full;
  wire [7:0] li_head;
  wire       go;
  wire is_uart_tx = addr == `SOC_UART_TX;
  wire is_link_tx = addr == `SOC_LINK_TX;
  wire is_link_rx = addr == `SOC_LINK_RX;
  wire is_qdata   = addr == `SOC_QSPI_DATA;
  wire q_valid;
  wire [63:0] q_word;

  hart_fifo #(.WIDTH(8), .DEPTH_BITS(4)) log_q (
    .clk(clk), .rst(rst), .clear(1'b0), .push(go && we && is_uart_tx), .din(wdata[7:0]),
    .pop(log_ready), .head(log_byte), .empty(log_empty), .full(log_full));
  assign log_valid = !log_empty;
  hart_fifo #(.WIDTH(8), .DEPTH_BITS(4)) link_out_q (
    .clk(clk), .rst(rst), .clear(1'b0), .push(go && we && is_link_tx), .din(wdata[7:0]),
    .pop(link_out_ready), .head(link_out_byte), .empty(lo_empty), .full(lo_full));
  assign link_out_valid = !lo_empty;
  hart_fifo #(.WIDTH(8), .DEPTH_BITS(4)) link_in_q (
    .clk(clk), .rst(rst), .clear(1'b0), .push(link_in_valid), .din(link_in_byte),
    .pop(go && !we && is_link_rx), .head(li_head), .empty(li_empty), .full(li_full));
  // A byte is taken only when the queue stores it: in reset the queue holds its pointers, so it takes none.
  assign link_in_ready = !rst && !li_full;

  hart_qspi #(.HALF(QSPI_HALF)) qspi (
    .clk(clk), .rst(rst),
    .start(go && we && addr == `SOC_QSPI_START), .start_addr(wdata[23:0]),
    .stop(go && we && addr == `SOC_QSPI_STOP),
    .word_valid(q_valid), .word(q_word), .word_pop(go && !we && is_qdata && stream_on),
    .sck(qspi_sck), .cs_n(qspi_cs_n), .mosi(qspi_mosi), .miso(qspi_miso));

  wire waits = we ? ((is_uart_tx && log_full) || (is_link_tx && lo_full)) : (is_qdata && stream_on && !q_valid);
  assign go = req && !ack && !waits && !exited;

  reg [63:0] value;
  always @(*) begin
    case (addr)
      `SOC_SYS_ID:          value = `SOC_SOC_ID;
      `SOC_SYS_CYCLES:      value = cycles;
      `SOC_SYS_FLAGS:       value = {62'd0, ddr_status[1:0]};
      `SOC_SYS_RTL_MD5_LO:  value = RTL_MD5_LO;
      `SOC_SYS_RTL_MD5_HI:  value = RTL_MD5_HI;
      `SOC_SYS_SWITCHES:    value = {48'd0, switches};
      `SOC_UART_STATUS:     value = {63'd0, log_full};
      `SOC_LINK_RX:         value = li_empty ? 64'd0 : {55'd0, 1'b1, li_head};
      `SOC_LINK_STATUS:     value = {62'd0, !li_empty, lo_full};
      `SOC_BUTTONS_STATE:   value = {59'd0, buttons};
      `SOC_BUTTONS_LEDS:    value = {48'd0, leds};
      `SOC_QSPI_DATA:       value = stream_on ? q_word : {64{1'b1}};
      `SOC_DDR_STATUS:      value = ddr_status;
      `SOC_DDR_CAL:         value = ddr_cal;
      default:              value = 64'd0;
    endcase
  end

  always @(posedge clk) begin
    if (rst) begin
      cycles <= 64'd0;
      ack <= 1'b0;
      stream_on <= 1'b0;
      leds <= 16'd0;
      exited <= 1'b0;
      exit_status <= 64'd0;
      restart <= 1'b0;
    end else begin
      cycles <= cycles + 64'd1;
      ack <= go;
      restart <= 1'b0;
      if (go) begin
        rdata <= value;
        if (we) begin
          case (addr)
            `SOC_SYS_RESET:     restart <= wdata == `SOC_SOC_RESET_KEY;
            `SOC_SYS_EXIT:      begin exited <= 1'b1; exit_status <= wdata; end
            `SOC_BUTTONS_LEDS:  leds <= wdata[15:0];
            `SOC_QSPI_START:    stream_on <= 1'b1;
            `SOC_QSPI_STOP:     stream_on <= 1'b0;
            default: ;
          endcase
        end
      end
    end
  end
endmodule
