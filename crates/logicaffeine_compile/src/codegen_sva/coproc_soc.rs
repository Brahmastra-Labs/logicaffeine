//! coproc_soc — the ULX3S-flashable SoC wrapper around `coproc_core`.
//!
//! `coproc_core` issues BADD/BSUB/BMUL/CMERGE to the synthesized coprocessor blocks and retires
//! to a 256-bit register file, but its `rout[255:0]` port is 262 pins — impossible on a board.
//! This wrapper makes it real hardware: hold the core in reset, run it to halt, then stream the
//! four RESULT registers (R2..R5) out over a single UART TX line, MSB byte first, each frame
//! preceded by a 0xA5 sync byte. A host (`scripts/coproc-host.py`) reads the 1 + 128 bytes and
//! checks them against the software model (`run_core`) — the on-silicon proof of a CPU-issued
//! coprocessor op.
//!
//! The RTL is FIXED logic (a UART-TX sequencer FSM); only the readout shape depends on the core
//! (four 256-bit result registers). The testbench's expected registers are DERIVED from
//! `run_core`, so a change to the coprocessor semantics regenerates the golden in lockstep — no
//! hand-typed hex. The wrapper .v, its testbench, and the board constraints are drift-locked; the
//! iverilog PASS + the routed `coproc_soc.bit` are recorded in `benchmarks/silicon/reports.json`.

use super::coproc_core::{run_core, COPROC_A, COPROC_B, COPROC_PROGRAM};

/// The result registers streamed out, in order: R2=BADD, R3=BSUB, R4=BMUL, R5=CMERGE.
pub const RESULT_REGS: [u8; 4] = [2, 3, 4, 5];

/// Emit the flashable SoC wrapper: `coproc_core` + a 7-state UART-TX readout FSM. External pins
/// (`clk`, `rst`, `uart_tx`, `led[7:0]`) fit the ULX3S; `CLKS_PER_BIT` = clk_hz / baud (217 for
/// 25 MHz / 115200). The RTL is operand-independent — the core carries the operands internally.
pub fn emit_coproc_soc_verilog() -> String {
    r#"// coproc_soc — a ULX3S-flashable SoC wrapper around coproc_core.
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
"#
    .to_string()
}

/// Emit the self-checking testbench. The expected result registers are DERIVED from `run_core`
/// (not hand-typed): the tb decodes the UART TX stream, reconstructs the sync byte + 128 payload
/// bytes, and checks each MSB-first byte against the model's retired R2..R5. PASS iff every byte
/// matches — the software≡RTL proof over the serial framing.
pub fn emit_coproc_soc_testbench() -> String {
    let regs = run_core(&COPROC_A, &COPROC_B, COPROC_PROGRAM);
    let tags = ["BADD", "BSUB", "BMUL", "CMERGE"];
    let mut exp = String::new();
    for (i, &r) in RESULT_REGS.iter().enumerate() {
        exp.push_str(&format!(
            "    exp_r[{r}] = {}; // {}\n",
            hex256(&regs[r as usize]),
            tags[i],
        ));
    }
    format!(
        r#"// tb_coproc_soc — drive coproc_soc, decode its UART TX stream, and check the four result
// registers against the software model (run_core). PASS iff every streamed byte matches.
`timescale 1ns/1ps
module tb_coproc_soc;
  localparam CPB  = 8;    // CLKS_PER_BIT for fast simulation
  localparam CLKP = 10;   // clock period (ns)

  reg  clk = 0, rst = 1;
  wire uart_tx;
  wire [7:0] led;
  coproc_soc #(.CLKS_PER_BIT(CPB)) dut(.clk(clk), .rst(rst), .uart_tx(uart_tx), .led(led));
  always #(CLKP/2) clk = ~clk;

  reg [255:0] exp_r [2:5];
  reg [7:0]   got [0:128];
  reg [255:0] tmp;
  reg [7:0]   exp_byte, rb;
  integer i, g, p, errors;

  // Receive one 8N1 byte (LSB first) from the TX line by mid-bit sampling.
  task uart_rx_byte(output [7:0] data);
    integer k;
    begin
      @(negedge uart_tx);                       // start-bit edge
      #(CPB * CLKP + (CPB * CLKP) / 2);         // advance to the middle of data bit 0
      for (k = 0; k < 8; k = k + 1) begin
        data[k] = uart_tx;
        #(CPB * CLKP);
      end
    end
  endtask

  initial begin
    // the four retired result registers, {{limb3,limb2,limb1,limb0}}, from run_core(COPROC_A,B):
{exp}
    #(3 * CLKP) rst = 0;
    errors = 0;
    for (i = 0; i <= 128; i = i + 1) begin
      uart_rx_byte(rb);
      got[i] = rb;
    end

    if (got[0] !== 8'hA5) begin errors = errors + 1; $display("FAIL sync byte: %h", got[0]); end
    for (i = 0; i < 128; i = i + 1) begin
      g = i / 32; p = i % 32;
      tmp = exp_r[2 + g];
      exp_byte = tmp[(31 - p) * 8 +: 8];
      if (got[1 + i] !== exp_byte) begin
        errors = errors + 1;
        $display("FAIL byte %0d (R%0d MSB-pos %0d): got %h exp %h", i, 2 + g, p, got[1 + i], exp_byte);
      end
    end

    if (errors == 0)
      $display("PASS: coproc_soc streamed R2..R5 over UART, matching run_core (sync + 128 bytes)");
    else
      $display("FAIL: %0d UART byte mismatches", errors);
    $finish;
  end
endmodule
"#,
        exp = exp.trim_end(),
    )
}

/// Emit the ULX3S (LFE5U-85F caBGA381) board constraints: 25 MHz clock (G2), reset = FIRE1 (R1),
/// UART TX → FTDI RXD (L4), 8 LEDs. Sites from the board's `ulx3s_v20.lpf`.
pub fn emit_ulx3s_coproc_lpf() -> String {
    let leds = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];
    let mut locate = String::new();
    let mut iobuf = String::new();
    for (i, site) in leds.iter().enumerate() {
        locate.push_str(&format!("LOCATE COMP \"led[{i}]\" SITE \"{site}\";\n"));
        iobuf.push_str(&format!(
            "IOBUF PORT \"led[{i}]\" PULLMODE=NONE IO_TYPE=LVCMOS33 DRIVE=4;\n"
        ));
    }
    format!(
        r#"# ULX3S (LFE5U-85F caBGA381) constraints for coproc_soc — the coprocessor-issuing core with a
# UART result-readout, flashable and verifiable on real hardware. Sites from the board's
# tools/prjtrellis/examples/ulx3s/ulx3s_v20.lpf.
SYSCONFIG CONFIG_IOVOLTAGE=3.3 COMPRESS_CONFIG=ON MCCLK_FREQ=62 MASTER_SPI_PORT=DISABLE SLAVE_SPI_PORT=DISABLE SLAVE_PARALLEL_PORT=DISABLE;

# 25 MHz oscillator
LOCATE COMP "clk" SITE "G2";
IOBUF PORT "clk" PULLMODE=NONE IO_TYPE=LVCMOS33;
FREQUENCY PORT "clk" 25 MHZ;

# reset = FIRE1 button (active-high: press to reset + re-stream)
LOCATE COMP "rst" SITE "R1";
IOBUF PORT "rst" PULLMODE=DOWN IO_TYPE=LVCMOS33;

# UART TX -> FTDI RXD (the FPGA transmits; read it on the host's /dev/ttyUSB* at 115200)
LOCATE COMP "uart_tx" SITE "L4";
IOBUF PORT "uart_tx" PULLMODE=UP IO_TYPE=LVCMOS33 DRIVE=4;

# 8 LEDs — crude progress / halted indicator
{locate}{iobuf}"#,
        locate = locate,
        iobuf = iobuf,
    )
}

fn hex256(x: &[u64; 4]) -> String {
    format!(
        "256'h{:016x}_{:016x}_{:016x}_{:016x}",
        x[3], x[2], x[1], x[0]
    )
}

/// The 129-byte serial frame the SoC streams: a 0xA5 sync byte, then R2..R5 (256-bit each),
/// most-significant byte first. This is exactly what `scripts/coproc-host.py` reconstructs and
/// what `tb_coproc_soc.v` samples off the wire — the single definition of the readout order.
pub fn expected_stream(regs: &[[u64; 4]; 8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 128);
    out.push(0xA5);
    for &r in &RESULT_REGS {
        let reg = regs[r as usize];
        for pos in 0..32u32 {
            // MSB byte first: pos 0 is the most-significant byte of the 256-bit register.
            let byte_from_top = pos;
            let bit_hi = 255 - byte_from_top * 8;
            let limb = (bit_hi / 64) as usize;
            let within = bit_hi % 64; // top bit of this byte inside its limb
            let byte = (reg[limb] >> (within - 7)) as u8;
            out.push(byte);
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/coproc_soc/tests.rs"]
mod tests;
