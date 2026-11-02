// psram_edge — the off-chip memory edge for the APS6404L-3SQN QSPI PSRAM (codegen_sva::psram).
// Power-up: tPU with CLK low and CE# high, Reset-Enable 0x66 + Reset 0x99 in SPI, tRST,
// then 0x35 into QPI. A request is one QPI burst: 0xeb (wait 6) to read, 0x38 to write,
// a 24-bit byte address, 16 nibbles per 64-bit word (byte 0 first, high nibble first). Outputs
// change at SCLK's falling edge; the part samples at its rising edge, and read data is taken at
// the rising edge. Emitted by `cargo run -p logicaffeine-compile --example emit_psram_edge`.
module psram_edge #(
  parameter integer H = 1,
  parameter integer POWERUP = 7500,
  parameter integer CPH = 1,
  parameter integer RSTGAP = 3
) (
  input clk, input rst,
  input req_valid, output req_ready, input req_we, input [19:0] req_addr, input [7:0] req_len,
  input [63:0] wdata, output reg wdata_take,
  output reg [63:0] rdata, output reg rdata_valid,
  output reg ce_n, output reg sclk, output reg [3:0] sio_out, output reg [3:0] sio_oe, input [3:0] sio_in
);
  localparam S_PU = 2'd0, S_OP = 2'd1, S_GAP = 2'd2, S_IDLE = 2'd3;
  localparam O_SPI = 2'd0, O_READ = 2'd1, O_WRITE = 2'd2;
  reg [1:0] state, op, init;
  reg [7:0] spi_byte;
  reg [23:0] baddr;
  reg [11:0] ei, nei;
  reg [7:0] phase;
  reg [31:0] count;
  reg [63:0] wbuf, rbuf;
  assign req_ready = (state == S_IDLE);

  // The nibble to present on edge `e` of the current operation, and whether to drive it.
  function [4:0] nibble(input [1:0] o, input [11:0] e, input [7:0] sb, input [23:0] a, input [63:0] w);
    reg [11:0] d; reg [3:0] j;
    begin
      if (o == O_SPI) nibble = {1'b1, 3'b000, sb[3'd7 - e[2:0]]};
      else if (e == 12'd0) nibble = {1'b1, (o == O_READ) ? 4'he : 4'h3};
      else if (e == 12'd1) nibble = {1'b1, (o == O_READ) ? 4'hb : 4'h8};
      else if (e < 12'd8) nibble = {1'b1, a[5'd23 - 5'd4 * (e[4:0] - 5'd2) -: 4]};
      else if (o == O_READ) nibble = 5'b0_0000;
      else begin
        d = e - 12'd8; j = d[3:0];
        nibble = {1'b1, w[8 * j[3:1] + 7 - 4 * j[0] -: 4]};
      end
    end
  endfunction

  // Begin operation `o`: CE# falls with the first edge's output.
  task start(input [1:0] o, input [7:0] sb);
    reg [4:0] n;
    begin
      op <= o; spi_byte <= sb; ei <= 12'd0; phase <= 8'd0; state <= S_OP;
      ce_n <= 1'b0; sclk <= 1'b0;
      nei <= (o == O_SPI) ? 12'd8 : (o == O_READ) ? 12'd14 + {req_len, 4'b0000} : 12'd8 + {req_len, 4'b0000};
      n = nibble(o, 12'd0, sb, (o == O_SPI) ? 24'd0 : {1'b0, req_addr, 3'b000}, wdata);
      sio_oe <= (o == O_SPI) ? 4'b0001 : {4{n[4]}}; sio_out <= n[3:0];
    end
  endtask

  reg [4:0] nx;
  reg [11:0] dn;
  reg [11:0] ds;
  always @(posedge clk) begin
    wdata_take <= 1'b0; rdata_valid <= 1'b0;
    if (rst) begin
      state <= S_PU; count <= 32'd0; init <= 2'd0;
      ce_n <= 1'b1; sclk <= 1'b0; sio_oe <= 4'b1111; sio_out <= 4'b0000;
    end else case (state)
      S_PU: if (count >= POWERUP - 1) start(O_SPI, 8'h66); else count <= count + 32'd1;
      S_IDLE: if (req_valid) begin
        baddr <= {1'b0, req_addr, 3'b000};
        start(req_we ? O_WRITE : O_READ, 8'h00);
      end
      S_GAP: if (count <= 32'd1) begin
        case (init)
          2'd1: start(O_SPI, 8'h99);
          2'd2: start(O_SPI, 8'h35);
          default: state <= S_IDLE;
        endcase
      end else count <= count - 32'd1;
      S_OP: begin
        if (phase == H - 1) begin
          sclk <= 1'b1;
          if (op == O_READ && ei >= 12'd14) begin
            ds = ei - 12'd14;
            if (ds[3:0] == 4'd15) begin
              rdata <= {rbuf[63:60], sio_in, rbuf[55:0]}; rdata_valid <= 1'b1;
            end else rbuf[8 * ds[3:1] + 7 - 4 * ds[0] -: 4] <= sio_in;
          end
        end
        if (phase == 2 * H - 1) begin
          sclk <= 1'b0; phase <= 8'd0;
          if (ei == nei - 12'd1) begin
            ce_n <= 1'b1; sio_oe <= 4'b0000; state <= S_GAP;
            if (op == O_SPI) begin
              init <= init + 2'd1;
              count <= (init == 2'd1) ? RSTGAP : CPH;
            end else count <= CPH;
          end else begin
            ei <= ei + 12'd1;
            dn = ei + 12'd1 - 12'd8;
            if (op == O_WRITE && ei + 12'd1 >= 12'd8 && dn[3:0] == 4'd0) begin
              wbuf <= wdata; wdata_take <= 1'b1;
              nx = nibble(op, ei + 12'd1, spi_byte, baddr, wdata);
            end else nx = nibble(op, ei + 12'd1, spi_byte, baddr, wbuf);
            sio_oe <= (op == O_SPI) ? 4'b0001 : {4{nx[4]}}; sio_out <= nx[3:0];
          end
        end else phase <= phase + 8'd1;
      end
    endcase
  end
endmodule
