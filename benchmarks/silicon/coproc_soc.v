// coproc_soc — a ULX3S-flashable SoC wrapper around coproc_core.
//
// coproc_core has a 262-pin interface (rout[255:0]) that cannot map to a board. This wrapper
// makes it real hardware: it runs coproc_core (which ISSUES BADD/BSUB/BMUL/CMERGE to the
// synthesized coprocessor blocks and retires to its register file), then streams the four
// RESULT registers (R2=BADD, R3=BSUB, R4=BMUL, R5=CMERGE) out over a UART TX line, MSB byte
// first, each frame preceded by a 0xA5 sync byte. A host reads the 4x32 = 128 bytes and checks
// them against the software model (run_core) — the on-silicon proof of a CPU-issued coproc op.
//
// UART: 8N1, one stop bit, LSB-first data. CLKS_PER_BIT = clk_hz / baud (217 for 25 MHz/115200).
// External pins fit the ULX3S: clk, rst, uart_tx, led. (Loadable programs over UART RX = later.)
module coproc_soc #(parameter CLKS_PER_BIT = 217) (
  input        clk,
  input        rst,        // active-high synchronous reset
  output reg   uart_tx,
  output [7:0] led
);
  // ---- the coprocessor-issuing core ----
  reg         core_rst;
  reg  [2:0]  rsel;
  wire [255:0] rout;
  wire         halted;
  coproc_core u_core(.clk(clk), .rst(core_rst), .rsel(rsel), .rout(rout), .halted(halted));

  // ---- readout + UART-TX sequencer ----
  localparam S_RESET = 3'd0,  // hold the core in reset for a few cycles
             S_RUN   = 3'd1,  // release, wait for halted
             S_LOAD  = 3'd2,  // latch the next byte to send
             S_START = 3'd3,  // UART start bit
             S_DATA  = 3'd4,  // 8 data bits
             S_STOP  = 3'd5,  // UART stop bit
             S_DONE  = 3'd6;
  reg [2:0]  state;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [7:0]  byte_idx;      // 0..128 : one 0xA5 sync + 4 regs x 32 bytes = 129 bytes total
  reg [3:0]  rst_cnt;

  // byte_idx layout: 0 = sync (0xA5); then group g = (byte_idx-1)/32 selects reg 2+g,
  // position p = (byte_idx-1)%32 selects the MSB-first byte of that 256-bit register.
  wire        is_sync = (byte_idx == 8'd0);
  wire [6:0]  n       = byte_idx - 8'd1;            // 0..127 once past the sync byte
  wire [1:0]  grp     = n[6:5];                     // 0..3  -> reg 2..5
  wire [4:0]  pos     = n[4:0];                     // 0..31 -> MSB-first byte index
  wire [7:0]  reg_byte = rout[(31 - pos) * 8 +: 8]; // pos 0 = most-significant byte

  assign led = {halted, byte_idx[6:0]};            // crude progress on the LEDs

  always @(posedge clk) begin
    if (rst) begin
      state <= S_RESET; core_rst <= 1'b1; uart_tx <= 1'b1; clk_cnt <= 16'd0;
      bit_idx <= 3'd0; byte_idx <= 8'd0; rsel <= 3'd2; rst_cnt <= 4'd0; tx_byte <= 8'd0;
    end else begin
      case (state)
        S_RESET: begin
          core_rst <= 1'b1; uart_tx <= 1'b1;
          if (rst_cnt == 4'd8) begin core_rst <= 1'b0; state <= S_RUN; end
          else rst_cnt <= rst_cnt + 4'd1;
        end
        S_RUN: begin
          core_rst <= 1'b0;
          rsel <= 3'd2 + grp;                 // present the register for the first data byte
          if (halted) state <= S_LOAD;
        end
        S_LOAD: begin
          rsel    <= 3'd2 + grp;
          tx_byte <= is_sync ? 8'hA5 : reg_byte;
          clk_cnt <= 16'd0; bit_idx <= 3'd0;
          state   <= S_START;
        end
        S_START: begin
          uart_tx <= 1'b0;                    // start bit
          if (clk_cnt == CLKS_PER_BIT - 1) begin clk_cnt <= 16'd0; state <= S_DATA; end
          else clk_cnt <= clk_cnt + 16'd1;
        end
        S_DATA: begin
          uart_tx <= tx_byte[bit_idx];        // LSB-first
          if (clk_cnt == CLKS_PER_BIT - 1) begin
            clk_cnt <= 16'd0;
            if (bit_idx == 3'd7) state <= S_STOP;
            else bit_idx <= bit_idx + 3'd1;
          end else clk_cnt <= clk_cnt + 16'd1;
        end
        S_STOP: begin
          uart_tx <= 1'b1;                     // stop bit
          if (clk_cnt == CLKS_PER_BIT - 1) begin
            clk_cnt <= 16'd0;
            if (byte_idx == 8'd128) state <= S_DONE;   // sync + 128 data bytes sent
            else begin byte_idx <= byte_idx + 8'd1; state <= S_LOAD; end
          end else clk_cnt <= clk_cnt + 16'd1;
        end
        S_DONE: begin uart_tx <= 1'b1; end
        default: state <= S_DONE;
      endcase
    end
  end
endmodule
