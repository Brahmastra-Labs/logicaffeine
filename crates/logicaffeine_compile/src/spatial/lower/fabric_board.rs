//! A fabric decode step as a board image: the step's RTL behind its host port, a serial line in front of the port,
//! and the autoregressive loop run on chip.
//!
//! The image serves a byte protocol over 8N1 serial, every multi-byte field little-endian:
//!
//! | Command | Arguments | Reply |
//! |---|---|---|
//! | `I` identify | — | `i`, `L O G S`, the memory read latency |
//! | `W` write a word | tensor (2), word (4), value (8) | `k` |
//! | `R` read a word | tensor (2), word (4) | `r`, value (8) |
//! | `S` one step | token (2), position (2) | `s`, next token (2), the step's cycles (4) |
//! | `G` generate | count (2) | the next token (2) per step, then `e` and the cycles of the whole loop (4) |
//!
//! A step writes the token and the position into the step's tensors, resets and starts the fabric, waits for it, and
//! reads the next token back; the cache the step writes stays in its memories. After a step the board holds the token
//! it produced and the position after it, so `G` continues from the last `S`: it runs the steps itself, each one's
//! token the one before it produced and its position the next, and its cycles are the chip's own decode time
//! ([`generate_cycles`]) — the serial line carries two bytes per token and never stalls it while its queue has room.
//! Tokens and positions are the values the step's tensors hold (the program's 1-based ones).

use super::SpatialLowerError;
use crate::spatial::ir::SpatialProgram;
use crate::spatial::lower::fabric::{host_word, VerilogArtifact};

/// Where a decode step's token, position and next token live: each a tensor and the word of its element `(0, 0)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeWords {
    pub tok: (usize, usize),
    pub pos: (usize, usize),
    pub next: (usize, usize),
}

impl DecodeWords {
    /// The words of the step's tensors named `tok`, `pos` and `next`.
    pub fn of(p: &SpatialProgram, tok: &str, pos: &str, next: &str) -> Result<DecodeWords, SpatialLowerError> {
        let word = |name: &str| -> Result<(usize, usize), SpatialLowerError> {
            let t = p
                .tensors
                .iter()
                .find(|t| t.name == name)
                .ok_or(SpatialLowerError::Unsupported("a decode step without its token, position or next-token tensor"))?;
            Ok((t.id.idx(), host_word(p, t, 0, 0)))
        };
        Ok(DecodeWords { tok: word(tok)?, pos: word(pos)?, next: word(next)? })
    }
}

/// The cycles `G` reports for steps that took `step_cycles` each: per step, the protocol's thirteen states around the
/// step — `G_LOOP`, the token and position writes, the reset, the start, the cycle the fabric's count ends on, the next
/// token's read and its `lat` cycles, `G_TOK`, and the four that queue the token's two bytes. The `G_LOOP` that finds no
/// step left reports the count as it stands before that cycle.
pub fn generate_cycles(step_cycles: &[u64], lat: u64) -> u64 {
    step_cycles.iter().map(|c| c + 13 + lat).sum()
}

/// A board image's core clock: the 100 MHz oscillator through a PLLE2, its VCO at `100 · mult / div` MHz and the core
/// at the VCO over `out`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreClock {
    pub mult: u32,
    pub div: u32,
    pub out: u32,
}

impl CoreClock {
    pub fn vco_mhz(self) -> f64 {
        100.0 * self.mult as f64 / self.div as f64
    }

    pub fn mhz(self) -> f64 {
        self.vco_mhz() / self.out as f64
    }

    /// Whether a PLLE2 of a speed grade -1 Artix-7 takes these settings (DS181): a multiplier of 2–64, an input divider
    /// of 1–56 and an output divider of 1–128, the phase detector at 19–450 MHz and the VCO at 800–1600 MHz.
    pub fn legal(self) -> bool {
        let pfd = 100.0 / self.div as f64;
        (2..=64).contains(&self.mult)
            && (1..=56).contains(&self.div)
            && (1..=128).contains(&self.out)
            && (19.0..=450.0).contains(&pfd)
            && (800.0..=1600.0).contains(&self.vco_mhz())
    }

    /// The fastest legal core clock whose period is at least `period_ns`; of clocks as fast, the one with the smallest
    /// input divider, then the slowest VCO.
    pub fn fastest_within(period_ns: f64) -> Option<CoreClock> {
        let mut best: Option<CoreClock> = None;
        for div in 1..=56 {
            for mult in 2..=64 {
                for out in 1..=128 {
                    let c = CoreClock { mult, div, out };
                    if !c.legal() || 1000.0 / c.mhz() < period_ns {
                        continue;
                    }
                    let better = match best {
                        None => true,
                        Some(b) => c.mhz() > b.mhz() + 1e-9 || ((c.mhz() - b.mhz()).abs() <= 1e-9 && (c.div, c.vco_mhz() as u64) < (b.div, b.vco_mhz() as u64)),
                    };
                    if better {
                        best = Some(c);
                    }
                }
            }
        }
        best
    }

    /// The core clock's own allowance a route does not model: the PLL's output jitter and the clock tree's skew.
    pub const UNCERTAINTY_NS: f64 = 0.5;

    /// The fastest legal core clock a route closes under the clock contract: its period at least the routed path
    /// (`1000 / fmax_mhz`), the residual the router bounds but does not time (`residual_ns`) and
    /// [`Self::UNCERTAINTY_NS`].
    pub fn for_route(fmax_mhz: f64, residual_ns: f64) -> Option<CoreClock> {
        CoreClock::fastest_within(1000.0 / fmax_mhz + residual_ns + CoreClock::UNCERTAINTY_NS)
    }
}

/// The serial line's bit time in core clocks at 115200 baud.
pub fn clks_per_bit(clock: CoreClock) -> u32 {
    (clock.mhz() * 1e6 / 115_200.0).round() as u32
}

/// An 8N1 receiver and transmitter: the receiver samples each bit at its middle and raises `valid` for one cycle with
/// a byte whose stop bit held; the transmitter sends `data` when `go` is raised while it is idle.
pub fn uart_verilog() -> String {
    String::from(
        "module uart_rx #(parameter CLKS_PER_BIT = 868) (input clk, input rst, input rx, output reg valid, output reg [7:0] data);
  reg [1:0] sync = 2'b11;
  always @(posedge clk) sync <= {sync[0], rx};
  wire line = sync[1];
  reg [1:0] st; reg [15:0] cnt; reg [2:0] bit_i; reg [7:0] sh;
  always @(posedge clk) begin
    valid <= 1'b0;
    if (rst) begin st <= 2'd0; cnt <= 16'd0; bit_i <= 3'd0; end
    else case (st)
      2'd0: if (!line) begin st <= 2'd1; cnt <= 16'd0; end
      2'd1: if (cnt == CLKS_PER_BIT / 2 - 1) begin cnt <= 16'd0; bit_i <= 3'd0; st <= line ? 2'd0 : 2'd2; end else cnt <= cnt + 16'd1;
      2'd2: if (cnt == CLKS_PER_BIT - 1) begin cnt <= 16'd0; sh <= {line, sh[7:1]}; if (bit_i == 3'd7) st <= 2'd3; else bit_i <= bit_i + 3'd1; end else cnt <= cnt + 16'd1;
      2'd3: if (cnt == CLKS_PER_BIT - 1) begin cnt <= 16'd0; st <= 2'd0; if (line) begin valid <= 1'b1; data <= sh; end end else cnt <= cnt + 16'd1;
    endcase
  end
endmodule
module uart_tx #(parameter CLKS_PER_BIT = 868) (input clk, input rst, input go, input [7:0] data, output reg tx, output busy);
  reg [1:0] st; reg [15:0] cnt; reg [2:0] bit_i; reg [7:0] sh;
  assign busy = st != 2'd0;
  always @(posedge clk) begin
    if (rst) begin st <= 2'd0; tx <= 1'b1; cnt <= 16'd0; bit_i <= 3'd0; end
    else case (st)
      2'd0: begin tx <= 1'b1; if (go) begin sh <= data; st <= 2'd1; cnt <= 16'd0; end end
      2'd1: begin tx <= 1'b0; if (cnt == CLKS_PER_BIT - 1) begin cnt <= 16'd0; bit_i <= 3'd0; st <= 2'd2; end else cnt <= cnt + 16'd1; end
      2'd2: begin tx <= sh[bit_i]; if (cnt == CLKS_PER_BIT - 1) begin cnt <= 16'd0; if (bit_i == 3'd7) st <= 2'd3; else bit_i <= bit_i + 3'd1; end else cnt <= cnt + 16'd1; end
      2'd3: begin tx <= 1'b1; if (cnt == CLKS_PER_BIT - 1) begin cnt <= 16'd0; st <= 2'd0; end else cnt <= cnt + 16'd1; end
    endcase
  end
endmodule
",
    )
}

/// A board's serial pair and the 128-byte reply queue that drains into its transmitter: what `decode_board` and
/// `chip_board` share. The protocol pushes a reply byte with `q_push`/`q_byte` while `q_room`.
const REPLY_QUEUE: &str = "  // The serial line and the reply queue it drains.
  wire rx_v; wire [7:0] rx_b; wire tx_busy; reg tx_go; reg [7:0] tx_byte;
  uart_rx #(CLKS_PER_BIT) urx(.clk(clk), .rst(rst), .rx(uart_rx), .valid(rx_v), .data(rx_b));
  uart_tx #(CLKS_PER_BIT) utx(.clk(clk), .rst(rst), .go(tx_go), .data(tx_byte), .tx(uart_tx), .busy(tx_busy));
  (* ram_style = \"registers\" *) reg [7:0] txq [0:127];
  reg [6:0] tq_w, tq_r; reg [7:0] tq_n; reg q_push; reg [7:0] q_byte;
  wire q_room = tq_n < 8'd127;
  wire q_pop = !tx_busy && !tx_go && tq_n != 8'd0;
  always @(posedge clk) begin
    tx_go <= 1'b0;
    if (rst) begin tq_w <= 7'd0; tq_r <= 7'd0; tq_n <= 8'd0; end
    else begin
      if (q_push) begin txq[tq_w] <= q_byte; tq_w <= tq_w + 7'd1; end
      if (q_pop) begin tx_go <= 1'b1; tx_byte <= txq[tq_r]; tq_r <= tq_r + 7'd1; end
      tq_n <= tq_n + (q_push ? 8'd1 : 8'd0) - (q_pop ? 8'd1 : 8'd0);
    end
  end
";

/// `decode_board`: the step (`spatial_top`), its host port driven by the protocol, the serial pair and a 128-byte reply
/// queue. `lat` is the step's memory read latency, the cycles a host read waits for its word.
pub fn decode_board_verilog(w: DecodeWords, lat: u64) -> String {
    let mut s = uart_verilog();
    s.push_str(&format!(
        "module decode_board #(parameter CLKS_PER_BIT = 868) (input clk, input rst, input uart_rx, output uart_tx, output [7:0] led);
  localparam LAT = {lat};
  localparam [15:0] TOK_T = 16'd{tok_t}, POS_T = 16'd{pos_t}, NEXT_T = 16'd{next_t};
  localparam [31:0] TOK_W = 32'd{tok_w}, POS_W = 32'd{pos_w}, NEXT_W = 32'd{next_w};
  // The step.
  reg f_rst, f_start; wire f_done; wire [31:0] f_cycles;
  reg h_we, h_re; reg [15:0] h_tensor; reg [31:0] h_addr; reg [63:0] h_wdata; wire [63:0] h_rdata;
  spatial_top fab(.clk(clk), .rst(rst | f_rst), .start(f_start), .done(f_done), .cycles(f_cycles), .host_we(h_we), .host_re(h_re),
    .host_site(16'd0), .host_tensor(h_tensor), .host_addr(h_addr), .host_wdata(h_wdata), .host_rdata(h_rdata));
",
        tok_t = w.tok.0,
        tok_w = w.tok.1,
        pos_t = w.pos.0,
        pos_w = w.pos.1,
        next_t = w.next.0,
        next_w = w.next.1,
    ));
    s.push_str(REPLY_QUEUE);
    s.push_str(&format!(
        "  // The protocol.
  localparam C_IDLE = 5'd0, C_ARGS = 5'd1, C_EXEC = 5'd2, C_REPLY = 5'd3, C_WROTE = 5'd4, C_READW = 5'd5,
             ST_WTOK = 5'd6, ST_WPOS = 5'd7, ST_RST = 5'd8, ST_START = 5'd9, ST_WAIT = 5'd10, ST_READ = 5'd11, ST_READW = 5'd12,
             S_DONE = 5'd13, G_LOOP = 5'd14, G_TOK = 5'd15;
  reg [4:0] st, ret, rret; reg [7:0] cmd; reg [3:0] argi, argc; reg [7:0] args [0:13];
  reg [7:0] rb [0:8]; reg [3:0] ri, rlen; reg [3:0] lcnt;
  reg [15:0] tok_r, pos_r, next_r, gen_n; reg [31:0] step_cycles, g_cycles; reg g_on;
  wire [15:0] arg16_0 = {{args[1], args[0]}}, arg16_2 = {{args[3], args[2]}};
  wire [31:0] arg32_2 = {{args[5], args[4], args[3], args[2]}};
  wire [63:0] arg64_6 = {{args[13], args[12], args[11], args[10], args[9], args[8], args[7], args[6]}};
  function [3:0] nargs(input [7:0] c);
    case (c) 8'h57: nargs = 4'd14; 8'h52: nargs = 4'd6; 8'h53: nargs = 4'd4; 8'h47: nargs = 4'd2; default: nargs = 4'd0; endcase
  endfunction
  assign led = {{g_on, st[4:0], tq_n != 8'd0, f_done}};
  always @(posedge clk) begin
    q_push <= 1'b0; h_we <= 1'b0; f_rst <= 1'b0; f_start <= 1'b0;
    if (g_on) g_cycles <= g_cycles + 32'd1;
    if (rst) begin st <= C_IDLE; h_re <= 1'b0; g_on <= 1'b0; tok_r <= 16'd0; pos_r <= 16'd0; end
    else case (st)
      C_IDLE: if (rx_v) begin
        cmd <= rx_b; argi <= 4'd0; argc <= nargs(rx_b);
        st <= (nargs(rx_b) == 4'd0) ? C_EXEC : C_ARGS;
      end
      C_ARGS: if (rx_v) begin
        args[argi] <= rx_b; argi <= argi + 4'd1;
        if (argi + 4'd1 == argc) st <= C_EXEC;
      end
      C_EXEC: case (cmd)
        8'h49: begin rb[0] <= 8'h69; rb[1] <= 8'h4c; rb[2] <= 8'h4f; rb[3] <= 8'h47; rb[4] <= 8'h53; rb[5] <= LAT;
                     rlen <= 4'd6; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
        8'h57: begin h_we <= 1'b1; h_tensor <= arg16_0; h_addr <= arg32_2; h_wdata <= arg64_6; st <= C_WROTE; end
        8'h52: begin h_re <= 1'b1; h_tensor <= arg16_0; h_addr <= arg32_2; lcnt <= LAT; st <= C_READW; end
        8'h53: begin tok_r <= arg16_0; pos_r <= arg16_2; ret <= S_DONE; st <= ST_WTOK; end
        8'h47: begin gen_n <= arg16_0; g_cycles <= 32'd0; g_on <= 1'b1; st <= G_LOOP; end
        default: begin rb[0] <= 8'h3f; rlen <= 4'd1; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
      endcase
      C_REPLY: if (ri == rlen) st <= rret;
               else if (q_room && !q_push) begin q_push <= 1'b1; q_byte <= rb[ri]; ri <= ri + 4'd1; end
      C_WROTE: begin rb[0] <= 8'h6b; rlen <= 4'd1; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
      C_READW: if (lcnt == 4'd0) begin
        h_re <= 1'b0; rb[0] <= 8'h72;
        rb[1] <= h_rdata[7:0]; rb[2] <= h_rdata[15:8]; rb[3] <= h_rdata[23:16]; rb[4] <= h_rdata[31:24];
        rb[5] <= h_rdata[39:32]; rb[6] <= h_rdata[47:40]; rb[7] <= h_rdata[55:48]; rb[8] <= h_rdata[63:56];
        rlen <= 4'd9; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end else lcnt <= lcnt - 4'd1;
      // One step: the token and the position in, a reset, a start, the run, the next token out.
      ST_WTOK: begin h_we <= 1'b1; h_tensor <= TOK_T; h_addr <= TOK_W; h_wdata <= {{48'd0, tok_r}}; st <= ST_WPOS; end
      ST_WPOS: begin h_we <= 1'b1; h_tensor <= POS_T; h_addr <= POS_W; h_wdata <= {{48'd0, pos_r}}; st <= ST_RST; end
      ST_RST: begin f_rst <= 1'b1; st <= ST_START; end
      ST_START: begin f_start <= 1'b1; st <= ST_WAIT; end
      ST_WAIT: if (f_done && !f_start) begin step_cycles <= f_cycles; st <= ST_READ; end
      ST_READ: begin h_re <= 1'b1; h_tensor <= NEXT_T; h_addr <= NEXT_W; lcnt <= LAT; st <= ST_READW; end
      ST_READW: if (lcnt == 4'd0) begin h_re <= 1'b0; next_r <= h_rdata[15:0]; st <= ret; end else lcnt <= lcnt - 4'd1;
      S_DONE: begin
        tok_r <= next_r; pos_r <= pos_r + 16'd1;
        rb[0] <= 8'h73; rb[1] <= next_r[7:0]; rb[2] <= next_r[15:8];
        rb[3] <= step_cycles[7:0]; rb[4] <= step_cycles[15:8]; rb[5] <= step_cycles[23:16]; rb[6] <= step_cycles[31:24];
        rlen <= 4'd7; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end
      // Generation: each step's token is the one before it produced, its position the next.
      G_LOOP: if (gen_n == 16'd0) begin
        g_on <= 1'b0; rb[0] <= 8'h65;
        rb[1] <= g_cycles[7:0]; rb[2] <= g_cycles[15:8]; rb[3] <= g_cycles[23:16]; rb[4] <= g_cycles[31:24];
        rlen <= 4'd5; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end else begin ret <= G_TOK; st <= ST_WTOK; end
      G_TOK: begin
        rb[0] <= next_r[7:0]; rb[1] <= next_r[15:8]; rlen <= 4'd2; ri <= 4'd0;
        tok_r <= next_r; pos_r <= pos_r + 16'd1; gen_n <= gen_n - 16'd1; rret <= G_LOOP; st <= C_REPLY;
      end
      default: st <= C_IDLE;
    endcase
  end
endmodule
"
    ));
    s
}

/// `chip_board`: one chip's program of a program split over chips ([`compile_program_to_chips`]), its host port and its
/// halts driven over the serial pair. Beside `I`, `W` and `R` as `decode_board` serves them:
///
/// | Command | Arguments | Reply |
/// |---|---|---|
/// | `X` run | — | `x`, the link it halted at (2), or `0xFFFF` at its end, then the cycles it has run (4) |
///
/// `X` starts a step — a reset and a start — when none is running, and otherwise continues the halted fabric past its
/// halt; either way it replies once the fabric halts again or finishes. The cycles pause while the fabric is halted, so
/// each reply counts the cycles the program has run ([`fabric_cycle_segments`](super::fabric::fabric_cycle_segments)).
/// A halted fabric's memories are still, so the host reads what the chip sends and writes what it receives with `R` and
/// `W` between runs.
///
/// [`compile_program_to_chips`]: crate::spatial::pipeline::compile_program_to_chips
pub fn chip_board_verilog(lat: u64) -> String {
    let mut s = uart_verilog();
    s.push_str(&format!(
        "module chip_board #(parameter CLKS_PER_BIT = 868) (input clk, input rst, input uart_rx, output uart_tx, output [7:0] led);
  localparam LAT = {lat};
  // The chip's program.
  reg f_rst, f_start, f_go; wire f_done, f_halted; wire [15:0] f_halt_at; wire [31:0] f_cycles;
  reg h_we, h_re; reg [15:0] h_tensor; reg [31:0] h_addr; reg [63:0] h_wdata; wire [63:0] h_rdata;
  spatial_top fab(.clk(clk), .rst(rst | f_rst), .start(f_start), .done(f_done), .cycles(f_cycles), .host_we(h_we), .host_re(h_re),
    .host_site(16'd0), .host_tensor(h_tensor), .host_addr(h_addr), .host_wdata(h_wdata), .host_rdata(h_rdata),
    .link_go(f_go), .halted(f_halted), .halt_at(f_halt_at));
"
    ));
    s.push_str(REPLY_QUEUE);
    s.push_str(
        "  // The protocol.
  localparam C_IDLE = 4'd0, C_ARGS = 4'd1, C_EXEC = 4'd2, C_REPLY = 4'd3, C_WROTE = 4'd4, C_READW = 4'd5,
             X_START = 4'd6, X_LEAVE = 4'd7, X_WAIT = 4'd8;
  reg [3:0] st, rret; reg [7:0] cmd; reg [3:0] argi, argc; reg [7:0] args [0:13];
  reg [7:0] rb [0:8]; reg [3:0] ri, rlen; reg [3:0] lcnt; reg x_on;
  wire [15:0] arg16_0 = {args[1], args[0]};
  wire [31:0] arg32_2 = {args[5], args[4], args[3], args[2]};
  wire [63:0] arg64_6 = {args[13], args[12], args[11], args[10], args[9], args[8], args[7], args[6]};
  function [3:0] nargs(input [7:0] c);
    case (c) 8'h57: nargs = 4'd14; 8'h52: nargs = 4'd6; default: nargs = 4'd0; endcase
  endfunction
  assign led = {x_on, f_halted, st[3:0], tq_n != 8'd0, f_done};
  always @(posedge clk) begin
    q_push <= 1'b0; h_we <= 1'b0; f_rst <= 1'b0; f_start <= 1'b0; f_go <= 1'b0;
    if (rst) begin st <= C_IDLE; h_re <= 1'b0; x_on <= 1'b0; end
    else case (st)
      C_IDLE: if (rx_v) begin
        cmd <= rx_b; argi <= 4'd0; argc <= nargs(rx_b);
        st <= (nargs(rx_b) == 4'd0) ? C_EXEC : C_ARGS;
      end
      C_ARGS: if (rx_v) begin
        args[argi] <= rx_b; argi <= argi + 4'd1;
        if (argi + 4'd1 == argc) st <= C_EXEC;
      end
      C_EXEC: case (cmd)
        8'h49: begin rb[0] <= 8'h69; rb[1] <= 8'h4c; rb[2] <= 8'h4f; rb[3] <= 8'h47; rb[4] <= 8'h53; rb[5] <= LAT;
                     rlen <= 4'd6; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
        8'h57: begin h_we <= 1'b1; h_tensor <= arg16_0; h_addr <= arg32_2; h_wdata <= arg64_6; st <= C_WROTE; end
        8'h52: begin h_re <= 1'b1; h_tensor <= arg16_0; h_addr <= arg32_2; lcnt <= LAT; st <= C_READW; end
        // A step starts with a reset and a start; a halted step continues past its halt and leaves it.
        8'h58: if (!x_on) begin f_rst <= 1'b1; st <= X_START; end else begin f_go <= 1'b1; st <= X_LEAVE; end
        default: begin rb[0] <= 8'h3f; rlen <= 4'd1; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
      endcase
      C_REPLY: if (ri == rlen) st <= rret;
               else if (q_room && !q_push) begin q_push <= 1'b1; q_byte <= rb[ri]; ri <= ri + 4'd1; end
      C_WROTE: begin rb[0] <= 8'h6b; rlen <= 4'd1; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY; end
      C_READW: if (lcnt == 4'd0) begin
        h_re <= 1'b0; rb[0] <= 8'h72;
        rb[1] <= h_rdata[7:0]; rb[2] <= h_rdata[15:8]; rb[3] <= h_rdata[23:16]; rb[4] <= h_rdata[31:24];
        rb[5] <= h_rdata[39:32]; rb[6] <= h_rdata[47:40]; rb[7] <= h_rdata[55:48]; rb[8] <= h_rdata[63:56];
        rlen <= 4'd9; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end else lcnt <= lcnt - 4'd1;
      X_START: begin f_start <= 1'b1; x_on <= 1'b1; st <= X_WAIT; end
      X_LEAVE: if (!f_halted) st <= X_WAIT;
      // Halted again, or finished: the link (0xFFFF at the end) and the cycles run.
      X_WAIT: if (f_done && !f_start) begin
        x_on <= 1'b0; rb[0] <= 8'h78; rb[1] <= 8'hff; rb[2] <= 8'hff;
        rb[3] <= f_cycles[7:0]; rb[4] <= f_cycles[15:8]; rb[5] <= f_cycles[23:16]; rb[6] <= f_cycles[31:24];
        rlen <= 4'd7; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end else if (f_halted) begin
        rb[0] <= 8'h78; rb[1] <= f_halt_at[7:0]; rb[2] <= f_halt_at[15:8];
        rb[3] <= f_cycles[7:0]; rb[4] <= f_cycles[15:8]; rb[5] <= f_cycles[23:16]; rb[6] <= f_cycles[31:24];
        rlen <= 4'd7; ri <= 4'd0; rret <= C_IDLE; st <= C_REPLY;
      end
      default: st <= C_IDLE;
    endcase
  end
endmodule
",
    );
    s
}

/// What a host needs of one chip's program of a program split over chips, as its ledger states it: the twin's count at
/// each halt and at the end ([`fabric_cycle_segments`](super::fabric::fabric_cycle_segments)), each link — the section it
/// follows, whether the fabric halts there, and each tensor it sends and receives with its id and every element's host
/// word, row-major — the words of the step's inputs (`tok`, `pos`) the chip holds, and of its result (`next`) if it holds
/// it. `None` where the fabric does not lower `p`.
pub fn chip_record(p: &SpatialProgram, target: &crate::spatial::target::SpatialTarget) -> Option<serde_json::Value> {
    let segments = super::fabric::fabric_cycle_segments(p, target)?;
    let words = |name: &str| -> Option<serde_json::Value> {
        let t = p.tensors.iter().find(|t| t.name == name)?;
        let w: Vec<usize> = (0..t.elems.rows * t.elems.cols).map(|i| host_word(p, t, i / t.elems.cols, i % t.elems.cols)).collect();
        Some(serde_json::json!({ "name": name, "tensor": t.id.idx(), "words": w }))
    };
    let named = |ids: &[crate::spatial::ir::TensorId]| -> Vec<serde_json::Value> {
        ids.iter().filter_map(|t| words(&p.tensor(*t).name)).collect()
    };
    let links: Vec<serde_json::Value> = p
        .links
        .iter()
        .map(|l| {
            serde_json::json!({
                "after": l.after,
                "halts": l.after.is_some_and(|k| k + 1 < p.sections.len()),
                "sends": named(&l.sends),
                "receives": named(&l.receives),
            })
        })
        .collect();
    let inputs: Vec<serde_json::Value> = ["tok", "pos"].iter().filter_map(|n| words(n)).collect();
    Some(serde_json::json!({
        "segments": segments,
        "links": links,
        "inputs": inputs,
        "next": words("next").unwrap_or(serde_json::Value::Null),
    }))
}

/// The relay as a split ledger states it: `{"run": chip, "to": link}` and `{"copy": tensor, "from": chip, "to": chip}`.
pub fn relay_record(relay: &[crate::spatial::tensor_parallel::RelayStep]) -> serde_json::Value {
    use crate::spatial::tensor_parallel::RelayStep;
    serde_json::Value::Array(
        relay
            .iter()
            .map(|s| match s {
                RelayStep::Run { chip, to } => serde_json::json!({ "run": chip, "to": to }),
                RelayStep::Copy { tensor, from, to } => serde_json::json!({ "copy": tensor, "from": from, "to": to }),
            })
            .collect(),
    )
}

/// The protocol a board image serves over its byte channel: the decode step's (`decode_board`: `I`, `W`, `R`, `S`, `G`)
/// or a chip's of a program split over chips (`chip_board`: `I`, `W`, `R`, `X`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardProtocol {
    Decode(DecodeWords),
    Chip,
}

impl BoardProtocol {
    /// The protocol's module, and the name of its file in an image.
    pub fn module(self) -> &'static str {
        match self {
            BoardProtocol::Decode(_) => "decode_board",
            BoardProtocol::Chip => "chip_board",
        }
    }

    /// The protocol's module, its host reads `lat` cycles long.
    pub fn verilog(self, lat: u64) -> String {
        match self {
            BoardProtocol::Decode(w) => decode_board_verilog(w, lat),
            BoardProtocol::Chip => chip_board_verilog(lat),
        }
    }

    /// What an image serving it is, as its constraints say.
    fn what(self) -> &'static str {
        match self {
            BoardProtocol::Decode(_) => "the decode step's board image",
            BoardProtocol::Chip => "a chip's board image of a program split over chips",
        }
    }
}

/// A Nexys board the image is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NexysBoard {
    /// The Nexys A7-100T (XC7A100T-1CSG324C): its byte channel is the FT2232's UART bridge.
    A7,
    /// The Nexys Video (XC7A200T-1SBG484C): its byte channel is the FT2232's FT245 FIFO, which the FT245 bridge
    /// ([`FT245_UART_BRIDGE_V`]) turns into the serial line `decode_board` speaks.
    Video,
}

/// The FT245 bridge a Nexys Video top instantiates: the board's FT245 FIFO to a serial pair.
/// `benchmarks/silicon/ft245_uart_bridge.v` is its golden twin, for the board flows that read it there.
pub const FT245_UART_BRIDGE_V: &str = include_str!("../../codegen_sva/ft245_uart_bridge.v");

/// The serial line between the FT245 bridge and `decode_board` on the Nexys Video, in core clocks per bit: on chip, so
/// short; the host's side runs at the FIFO's own rate.
const BRIDGE_CPB: u32 = 8;

impl NexysBoard {
    /// The part, as the open flow names it.
    pub fn part(self) -> &'static str {
        match self {
            NexysBoard::A7 => "xc7a100tcsg324-1",
            NexysBoard::Video => "xc7a200tsbg484-1",
        }
    }

    /// The hand-written Verilog the top instantiates beside the step and its protocol, each by its file name and text.
    pub fn extra_sources(self) -> &'static [(&'static str, &'static str)] {
        match self {
            NexysBoard::A7 => &[],
            NexysBoard::Video => &[("ft245_uart_bridge.v", FT245_UART_BRIDGE_V)],
        }
    }

    /// The board's top `top`: the 100 MHz oscillator through a PLL onto a global clock buffer for the core at `clock`,
    /// a reset held until the PLL locks and eight cycles after, `protocol`'s module on the board's byte channel, and the
    /// eight status LEDs.
    pub fn top_verilog(self, top: &str, clock: CoreClock, protocol: BoardProtocol) -> String {
        let module = protocol.module();
        let core = format!(
            "  wire clkfb, core_raw, locked, coreclk;\n\
             \x20 PLLE2_BASE #(.CLKIN1_PERIOD(10.0), .CLKFBOUT_MULT({}), .DIVCLK_DIVIDE({}), .CLKOUT0_DIVIDE({})) pll (\n\
             \x20   .CLKIN1(clk), .CLKFBIN(clkfb), .CLKFBOUT(clkfb), .CLKOUT0(core_raw), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0));\n\
             \x20 BUFG bufg_core (.I(core_raw), .O(coreclk));\n",
            clock.mult, clock.div, clock.out
        );
        let reset = "  reg [3:0] por = 4'd0;\n  always @(posedge coreclk) if (!locked) por <= 4'd0; else if (!por[3]) por <= por + 4'd1;\n";
        match self {
            NexysBoard::A7 => format!(
                "module {top} (input clk, input uart_rx, output uart_tx, output [7:0] led);\n{core}{reset}\
                 \x20 {module} #(.CLKS_PER_BIT({cpb})) board(.clk(coreclk), .rst(!por[3]), .uart_rx(uart_rx), .uart_tx(uart_tx), .led(led));\n\
                 endmodule\n",
                cpb = clks_per_bit(clock)
            ),
            NexysBoard::Video => format!(
                "module {top} (input clk, inout [7:0] prog_d, input prog_rxen, input prog_txen, output prog_rdn, output prog_wrn,\n\
                 \x20 output prog_oen, output prog_siwun, output prog_spien, output [7:0] led);\n{core}{reset}\
                 \x20 wire to_board, from_board;\n\
                 \x20 ft245_uart_bridge #(.CLKS_PER_BIT({BRIDGE_CPB}), .W(4)) bridge(.clk(coreclk), .rst(!por[3]), .eng_rx(to_board), .eng_tx(from_board),\n\
                 \x20   .prog_d(prog_d), .prog_rxen(prog_rxen), .prog_txen(prog_txen), .prog_rdn(prog_rdn), .prog_wrn(prog_wrn), .prog_oen(prog_oen),\n\
                 \x20   .prog_siwun(prog_siwun), .prog_spien(prog_spien));\n\
                 \x20 {module} #(.CLKS_PER_BIT({BRIDGE_CPB})) board(.clk(coreclk), .rst(!por[3]), .uart_rx(to_board), .uart_tx(from_board), .led(led));\n\
                 endmodule\n"
            ),
        }
    }

    /// The constraints for `top`: the oscillator, the byte channel's pins and the LEDs, from Digilent's master XDC
    /// files.
    pub fn xdc(self, top: &str, protocol: BoardProtocol) -> String {
        let pin = |p: &str, port: &str, io: &str| format!("set_property -dict {{ PACKAGE_PIN {p} IOSTANDARD {io} }} [get_ports {{ {port} }}]\n");
        let mut s = format!("# {top}: {} on the {self:?} ({}).\n", protocol.what(), self.part());
        let (clk, leds, io_led): (&str, [&str; 8], &str) = match self {
            NexysBoard::A7 => ("E3", ["H17", "K15", "J13", "N14", "R18", "V17", "U17", "U16"], "LVCMOS33"),
            NexysBoard::Video => ("R4", ["T14", "T15", "T16", "U16", "V15", "W16", "W15", "Y13"], "LVCMOS25"),
        };
        s.push_str(&pin(clk, "clk", "LVCMOS33"));
        s.push_str("create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]\n");
        match self {
            NexysBoard::A7 => {
                s.push_str(&pin("C4", "uart_rx", "LVCMOS33"));
                s.push_str(&pin("D4", "uart_tx", "LVCMOS33"));
            }
            NexysBoard::Video => {
                for (i, p) in ["U20", "P14", "P15", "U17", "R17", "P16", "R18", "N14"].iter().enumerate() {
                    s.push_str(&pin(p, &format!("prog_d[{i}]"), "LVCMOS33"));
                }
                for (p, port) in [("N17", "prog_rxen"), ("Y19", "prog_txen"), ("P19", "prog_rdn"), ("R19", "prog_wrn"), ("V17", "prog_oen"), ("P17", "prog_siwun"), ("R14", "prog_spien")] {
                    s.push_str(&pin(p, port, "LVCMOS33"));
                }
            }
        }
        for (i, p) in leds.iter().enumerate() {
            s.push_str(&pin(p, &format!("led[{i}]"), io_led));
        }
        s
    }
}

/// Every file a board build reads, by name: the program's RTL and the memory images its `$readmemh` loads, `protocol`'s
/// module with the serial pair, the board's top — named `top`, its core at `clock` — its constraints, and the
/// hand-written Verilog the top instantiates ([`NexysBoard::extra_sources`]).
pub fn board_image(
    art: &VerilogArtifact,
    images: &[(String, String)],
    protocol: BoardProtocol,
    lat: u64,
    board: NexysBoard,
    top: &str,
    clock: CoreClock,
) -> Vec<(String, String)> {
    let mut files = vec![
        ("spatial_top.v".to_string(), art.rtl.clone()),
        (format!("{}.v", protocol.module()), protocol.verilog(lat)),
        (format!("{top}.v"), board.top_verilog(top, clock, protocol)),
        (format!("{top}.xdc"), board.xdc(top, protocol)),
    ];
    files.extend(art.hex.iter().cloned());
    // The program's own fills overwrite the artifact's placeholder images of the same name.
    for (name, body) in images {
        match files.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = body.clone(),
            None => files.push((name.clone(), body.clone())),
        }
    }
    files.extend(board.extra_sources().iter().map(|(name, text)| (name.to_string(), text.to_string())));
    files
}

/// What a board's memories hold when it is configured: each tensor's fill, and zero for one supplied at run time
/// ([`TensorInit::External`], which the host or the link writes before the program reads it).
fn configured_memories(p: &SpatialProgram) -> Result<crate::spatial::eval::Inputs, BoardImageError> {
    use crate::spatial::eval::{Inputs, Value};
    use crate::spatial::ir::{TensorInit, TensorRole};
    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let n = t.elems.rows * t.elems.cols;
        let value = if t.init == TensorInit::External {
            Some(Value::Int(vec![0; n]))
        } else if t.init.is_float() {
            (0..n).map(|i| t.init.element_f64(i)).collect::<Option<Vec<f64>>>().map(Value::Float)
        } else {
            (0..n).map(|i| t.init.element(i)).collect::<Option<Vec<i64>>>().map(Value::Int)
        };
        let value = value.ok_or_else(|| BoardImageError::Fill(format!("`{}` has no deterministic fill", t.name)))?;
        inputs.tensors.insert(t.id, value);
    }
    Ok(inputs)
}

/// Why a program has no board image.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardImageError {
    /// The fabric does not lower it.
    Lower(SpatialLowerError),
    /// A tensor it reads has no deterministic fill to load its memory with, and is not supplied at run time.
    Fill(String),
    /// The target does not say how many cycles its memories take to read.
    ReadLatency,
}

/// What a board image runs: a whole decode step, which the host steps and the chip loops ([`BoardProtocol::Decode`]),
/// or one chip's program of a split, which the host runs from halt to halt ([`BoardProtocol::Chip`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRole {
    DecodeStep,
    Chip,
}

/// The board image of `p`, a fabric program for `target`: lowered, its memories loaded with its fills, on `board` with
/// the top named `top` and its core at `clock`, speaking `role`'s protocol (a decode step's through its `tok`, `pos` and
/// `next` words). Every file synthesis reads, so its [`image_digest`] is the image's identity.
pub fn program_board_image(
    p: &SpatialProgram,
    target: &crate::spatial::target::SpatialTarget,
    board: NexysBoard,
    role: ImageRole,
    top: &str,
    clock: CoreClock,
) -> Result<Vec<(String, String)>, BoardImageError> {
    use crate::spatial::lower::SpatialBackend;
    let inputs = configured_memories(p)?;
    let art = match crate::spatial::lower::fabric::FabricBackend::new(target.clone()).lower(p) {
        Ok(crate::spatial::lower::SpatialArtifact::Verilog(v)) => v,
        Ok(_) => unreachable!("the fabric backend lowers to Verilog"),
        Err(e) => return Err(BoardImageError::Lower(e)),
    };
    let lat = target.site.local_mem_read_latency.ok_or(BoardImageError::ReadLatency)? as u64;
    let protocol = match role {
        ImageRole::DecodeStep => BoardProtocol::Decode(DecodeWords::of(p, "tok", "pos", "next").map_err(BoardImageError::Lower)?),
        ImageRole::Chip => BoardProtocol::Chip,
    };
    let images = crate::spatial::lower::fabric::input_hex_files(p, &inputs);
    Ok(board_image(&art, &images, protocol, lat, board, top, clock))
}

/// The identity of a board build: the sha256, in hex, of every file synthesis reads — in name order, each its name, a
/// zero byte, its text and a zero byte — with every Verilog file's simulation-only regions (`synthesis translate_off`
/// through `synthesis translate_on`) taken out, so a check only a simulation runs moves no bitstream's identity.
pub fn image_digest(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut msg = Vec::new();
    for (name, body) in sorted {
        msg.extend_from_slice(name.as_bytes());
        msg.push(0);
        let text = if name.ends_with(".v") { synthesized(body) } else { body.clone() };
        msg.extend_from_slice(text.as_bytes());
        msg.push(0);
    }
    crate::crypto_accel::sha256(&msg).iter().map(|b| format!("{b:02x}")).collect()
}

/// `verilog` as synthesis reads it: without its simulation-only regions.
pub fn synthesized(verilog: &str) -> String {
    let mut out = String::new();
    let mut simulation_only = false;
    for line in verilog.lines() {
        if line.contains("synthesis translate_off") {
            simulation_only = true;
        } else if line.contains("synthesis translate_on") {
            simulation_only = false;
        } else if !simulation_only {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
