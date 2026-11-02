//! The engine's memories as the FPGA's block RAM holds them.
//!
//! `logos_ram_w<W> #(D, A)` is a simple dual-port memory of `D` words of `W` bits: its read port latches the address on
//! the clock's falling edge and holds the word until the next falling edge; its write port writes on the rising edge
//! under a byte mask. Between two rising edges a read sees the memory as the last rising edge left it, at an address
//! the registers that edge set, which is what a combinational read of a register array sees within the cycle. So a
//! unit's state machine keeps its reads where they are while the memory maps onto M20K blocks, the address given half a
//! cycle and the data half a cycle.
//!
//! `logos_bram_w<W> #(D, A)` is the same memory with the block RAM's registered read: the address taken on a rising edge
//! and its word out from that edge until the next, a read of the address the same edge writes returning the word before
//! the write. The streaming engine, clocked at the memory interface's rate, gives each read address a clock ahead.
//!
//! `logos_bram_re_w<W> #(D, A)` takes its read address only on a rising edge where `re` is high and holds the word it
//! read until the next such edge: the hybrid engine's token-row store, clocked by the fast clock, read at the engine
//! clock's falling edges while the engine owns it.
//!
//! Each width is its own module whose write names every byte lane in its own statement, the form the synthesis tools
//! map to a block RAM's byte enables. A registered or held memory (`logos_bram_w<W>`, `logos_bram_re_w<W>`) of 40 bits
//! or wider, more than one layer of 512 words deep in a number of layers that is no power of two, is stacks of powers of
//! two layers, its deepest its own and the rest `<name>_part`s beside it, so no stack is built past the memory's depth.
//!
//! Every memory asks for the block RAM its depth fills (`LOGOS_RAMSTYLE`, [`RAMSTYLE`]): MLABs to [`MLAB_WORDS`] words,
//! M20K blocks beyond.

use std::fmt::Write as _;

/// The deepest memory kept in MLABs: an MLAB's depth. A memory this shallow asking for nothing is built by Quartus Prime
/// Pro in an M20K block for every forty bits of its width, a sixteenth of the block's depth or less filled (hybrid9c2:
/// 5,142 blocks, 44% of the device's, on 2.2 Mbit, and a fit that failed routing with each column core spread over its
/// 214 blocks). From two words on an MLAB holds a wide memory in fewer ALMs than its flip-flops do (hybrid9d: a column
/// core's ring of two 5,280-bit beats in registers, 169k across the array, left synthesis at 88% of the device's ALMs);
/// Quartus keeps a memory too small for one in registers itself.
pub const MLAB_WORDS: usize = 32;

/// `LOGOS_RAMSTYLE(words)`: the `ramstyle` attribute of a memory of `words` words (a constant expression of the module's
/// parameters) read into a register — MLABs to [`MLAB_WORDS`] words, M20K blocks beyond — and
/// `LOGOS_RAMSTYLE_ASYNC(words)` the same for a memory read without a register after it (unregistered, or at a
/// registered address, so seeing a word the clock it is written), its MLABs asking for no read-during-write check:
/// Quartus Prime Pro builds an MLAB's read only through the MLAB's output register otherwise, and builds such a memory in
/// flip-flops (a probe's 32 x 1,056-bit buffer: 33,797 registers). An engine's build times an MLAB's read of a word in
/// the clock after its write (`logos_settings.tcl`, [`super::super::ofs`]), so the word read is the one written, as the
/// RTL reads it. Defined once in whichever text names it first, so every unit's text carries it.
pub const RAMSTYLE: &str = "`ifndef LOGOS_RAMSTYLE
// The block RAM a memory of `words` words read into a register asks for: MLABs to thirty-two words (an MLAB's depth),
// M20K blocks beyond.
`define LOGOS_RAMSTYLE(words) (* ramstyle = (words) <= 32 ? \"MLAB\" : \"M20K\" *)
// The same for a memory read without a register after it, its MLABs checking no read-during-write: the build times an
// MLAB's read in the clock after its word's write.
`define LOGOS_RAMSTYLE_ASYNC(words) (* ramstyle = (words) <= 32 ? \"MLAB, no_rw_check\" : \"M20K\" *)
`endif
";

/// The module name of the memory `w` bits wide.
pub fn ram_name(w: usize) -> String {
    format!("logos_ram_w{w}")
}

/// The module name of the memory `w` bits wide whose read is registered on the rising edge.
pub fn bram_name(w: usize) -> String {
    format!("logos_bram_w{w}")
}

/// The memory modules for the widths `widths` (each a whole number of bytes), each asking for the block RAM its depth fills.
pub fn ram_verilog(widths: &[usize]) -> String {
    memories(widths, ram_name, "the read address latched on the falling edge", Read::Falling)
}

/// The registered-read memory modules for the widths `widths` (each a whole number of bytes).
pub fn bram_verilog(widths: &[usize]) -> String {
    memories(widths, bram_name, "the read registered on the rising edge", Read::Rising)
}

/// The module name of the memory `w` bits wide whose read is registered on the rising edges `re` is high on.
pub fn bram_re_name(w: usize) -> String {
    format!("logos_bram_re_w{w}")
}

/// The memory modules for the widths `widths` whose read is registered on the rising edges `re` is high on, the word
/// held between them.
pub fn bram_re_verilog(widths: &[usize]) -> String {
    memories(widths, bram_re_name, "the read registered on the rising edges re is high on and held between them", Read::Held)
}

/// The module name of the memory `w` bits wide of an engine whose registers step on the rising edges `ce` is high on.
pub fn ram_ce_name(w: usize) -> String {
    format!("logos_ram_ce_w{w}")
}

/// `logos_ram_w<W>` for an engine whose registers step on the rising edges `ce` is high on: the read registered on the
/// edges `cr` is high on, the word held between them, the write on the edges `ce` is high on. With `ce` high the user
/// clock's cycle that ends with an engine step and `cr` high the cycle after the engine cycle's middle, a read sees
/// the memory as the step before left it, at an address the registers that step set, as the falling-edge read does.
pub fn ram_ce_verilog(widths: &[usize]) -> String {
    memories(widths, ram_ce_name, "the read registered on the rising edges cr is high on and held between them", Read::Enabled)
}

/// Clocks the pipelined token-row store (`logos_store_w512`, [`store_verilog`]) takes from a read's address to its word:
/// the ports registered, the layer read, the layers' words gathered a group at a time, and the groups' into the word.
pub const STORE_READ_LATENCY: usize = 4;

/// Layers of a pipelined store whose words one register of its first gathering takes.
pub const STORE_GROUP: usize = 8;

/// `logos_store_w512 #(D, A)`: the token-row store's bank as [`bram_re_verilog`]'s `logos_bram_re_w512` holds it — its read
/// taken on the edges `re` is high on and held between them, a write under a byte mask — its word out
/// [`STORE_READ_LATENCY`] clocks after its address instead of one. Its words are layers of 512, each a block RAM of its
/// own read every clock (`logos_bram_w512_part`), so the block takes its read register whole and reads the word before a
/// write of its address in the same clock, as its port does; a layer read under the read enable would hand the block the
/// address register instead and need pass-through logic in the fabric for its read-during-write. Every port is taken into
/// registers a clock before the layers see it, each layer's write enable decoded there, the read's address and the write's
/// row, data and mask copied near the layers they reach (`maxfan`). Each layer's word is zero unless the read was of it,
/// and the words are gathered by an OR, [`STORE_GROUP`] layers at a time into registers, then across the groups into the
/// word read, which alone is held between the clocks `re` is high on (the read enable carried beside the read). A write
/// lands a clock after it is asked, as a read does, so each read sees the writes it would if neither waited: a reader that
/// takes its word [`STORE_READ_LATENCY`] clocks after its address reads what it read a clock after its address from the
/// bank of one.
pub fn store_verilog() -> String {
    format!("{}\n{}\n{STORE_VERILOG}", bram_re_verilog(&[512]), bram_verilog(&[512]))
}

const STORE_VERILOG: &str = r#"// The token-row store's bank in layers of 512 words, its word out four clocks after its address: the ports registered
// (q_*), each layer's write enable decoded there; every layer read every clock, each layer's word kept only if the read
// was of it (mine, taken with the read); eight layers' words ORed into a register (gq); the groups' ORed into the word
// (rd_q), taken only where re was high (r2, r3: re beside the read), so held between the edges re is high on, as
// logos_bram_re_w512. A write lands a clock after it is asked, as a read does.
module logos_store_w512 #(parameter D = 1024, A = 10)
  (input clk, input re, input [A-1:0] ra, output [511:0] rd,
   input we, input [A-1:0] wa, input [511:0] wd, input [63:0] wm);
  localparam LAYERS = (D + 511) / 512, GROUPS = (LAYERS + 7) / 8;
  localparam LA = A > 9 ? A - 9 : 1;
  // Each address's row within its layer and its layer.
  wire [8:0] r_row, w_row;
  wire [LA-1:0] r_layer, w_layer;
  generate
    if (A > 9) begin : deep
      assign r_row = ra[8:0];
      assign w_row = wa[8:0];
      assign r_layer = ra[A-1:9];
      assign w_layer = wa[A-1:9];
    end else if (A == 9) begin : one
      assign r_row = ra;
      assign w_row = wa;
      assign r_layer = 1'b0;
      assign w_layer = 1'b0;
    end else begin : shallow
      assign r_row = {{(9-A){1'b0}}, ra};
      assign w_row = {{(9-A){1'b0}}, wa};
      assign r_layer = 1'b0;
      assign w_layer = 1'b0;
    end
  endgenerate
  (* maxfan = 64 *) reg q_re;
  (* maxfan = 64 *) reg [8:0] q_ra, q_wa;
  (* maxfan = 64 *) reg [63:0] q_wm;
  (* maxfan = 64 *) reg [511:0] q_wd;
  reg [LA-1:0] q_rl;
  reg [LAYERS-1:0] q_we;
  integer k;
  always @(posedge clk) begin
    q_re <= re;
    q_ra <= r_row;
    q_rl <= r_layer;
    q_wa <= w_row;
    q_wd <= wd;
    q_wm <= wm;
    for (k = 0; k < LAYERS; k = k + 1) q_we[k] <= we && w_layer == k[LA-1:0];
  end
  wire [512*LAYERS-1:0] lq;
  (* maxfan = 64 *) reg [LAYERS-1:0] mine;
  genvar l;
  generate
    for (l = 0; l < LAYERS; l = l + 1) begin : layer
      localparam [LA-1:0] AT = l;
      logos_bram_w512_part #(.D(512), .A(9)) part(.clk(clk), .ra(q_ra), .rd(lq[512*l +: 512]),
        .we(q_we[l]), .wa(q_wa), .wd(q_wd), .wm(q_wm));
      always @(posedge clk) mine[l] <= q_rl == AT;
    end
  endgenerate
  // Each group's word: its layers' words where the read was of them, ORed.
  reg [512*GROUPS-1:0] gsum, gq;
  integer g, j;
  always @* begin
    gsum = {(512*GROUPS){1'b0}};
    for (g = 0; g < GROUPS; g = g + 1)
      for (j = 0; j < 8; j = j + 1)
        if (8 * g + j < LAYERS) gsum[512*g +: 512] = gsum[512*g +: 512] | (mine[8*g + j] ? lq[512*(8*g + j) +: 512] : 512'd0);
  end
  always @(posedge clk) gq <= gsum;
  reg [511:0] word, rd_q;
  integer h;
  always @* begin
    word = 512'd0;
    for (h = 0; h < GROUPS; h = h + 1) word = word | gq[512*h +: 512];
  end
  reg r2, r3;
  always @(posedge clk) begin
    r2 <= q_re;
    r3 <= r2;
    if (r3) rd_q <= word;
  end
  assign rd = rd_q;
endmodule
"#;

/// The depth from which `logos_delay` keeps its words in block RAM: a chain of one or two registers is cheaper than a
/// block, and from three clocks on a chain is three or more flip-flops a bit, which Quartus Prime Pro builds as written.
pub const DELAY_RING_LEAST: usize = 3;

/// The width from which `logos_delay`'s ring deeper than an MLAB asks for M20K blocks, a block's widest word; a narrower
/// ring asks for MLABs, a block holding it in a fortieth of its bits or less.
pub const DELAY_M20K_LEAST: usize = 40;

/// `logos_delay #(W, D)`, a word of `W` bits delayed `D` clocks (the word itself at depth 0). One or two clocks are a
/// chain of registers. From [`DELAY_RING_LEAST`] clocks on the words go round a ring, a memory of the next power of two
/// of `D` words written a word a clock and read `D - 1` words behind the write into its port's register, the chain's
/// `D·W` flip-flops left as the ring's write address: `logos_delay_mlab` in MLABs to [`MLAB_WORDS`] words whatever its
/// width, and beyond that `logos_delay_m20k` in M20K blocks from [`DELAY_M20K_LEAST`] bits wide and `logos_delay_mlab`
/// below.
pub fn delay_verilog() -> &'static str {
    DELAY_VERILOG
}

const DELAY_VERILOG: &str = r#"
// A read's marks carried D clocks to the clock its word lands: the valid in_v and the marks in_d themselves at depth 0,
// else through D registers, the valid in registers the reset clears; flight is high while a valid is in any of them.
module logos_landing #(parameter W = 1, D = 0)
  (input clk, input rst, input in_v, input [W-1:0] in_d, output out_v, output [W-1:0] out_d, output flight);
  generate
    if (D == 0) begin : now
      assign out_v = in_v;
      assign out_d = in_d;
      assign flight = 1'b0;
    end else begin : later
      (* maxfan = 64 *) reg [D-1:0] v;
      reg [W*D-1:0] m;
      integer k;
      always @(posedge clk) begin
        v[0] <= !rst && in_v;
        m[W-1:0] <= in_d;
        for (k = 1; k < D; k = k + 1) begin
          v[k] <= !rst && v[k-1];
          m[W*k +: W] <= m[W*(k-1) +: W];
        end
      end
      assign out_v = v[D-1];
      assign out_d = m[W*(D-1) +: W];
      assign flight = |v;
    end
  endgenerate
endmodule

// d delayed D clocks: the word itself at depth 0, a chain of registers at one or two, a ring of block RAM from three, in
// MLABs to 32 words (an MLAB's depth), and past that in M20K blocks from 40 bits wide and in MLABs below.
module logos_delay #(parameter W = 32, D = 1) (input clk, input [W-1:0] d, output [W-1:0] q);
  generate
    if (D == 0) begin : now
      assign q = d;
    end else if (D < 3) begin : chain
      reg [W-1:0] r1, r2;
      always @(posedge clk) begin
        r1 <= d;
        r2 <= r1;
      end
      assign q = D == 1 ? r1 : r2;
    end else if (W >= 40 && (1 << $clog2(D)) > 32) begin : wide
      logos_delay_m20k #(.W(W), .D(D)) r(.clk(clk), .d(d), .q(q));
    end else begin : narrow
      logos_delay_mlab #(.W(W), .D(D)) r(.clk(clk), .d(d), .q(q));
    end
  endgenerate
endmodule

// d delayed D clocks (D of 2 or more) through a memory of 2^A >= D words in M20K blocks: each clock's word written at the write
// address, and the word written D - 1 clocks before read into q. The read is never of the address being written.
module logos_delay_m20k #(parameter W = 32, D = 3) (input clk, input [W-1:0] d, output reg [W-1:0] q);
  localparam A = $clog2(D);
  (* ramstyle = "M20K" *) reg [W-1:0] mem [0:(1 << A) - 1];
  localparam [A-1:0] BACK = D - 1;
  reg [A-1:0] wa = {A{1'b0}};
  wire [A-1:0] ra = wa - BACK;
  always @(posedge clk) begin
    mem[wa] <= d;
    q <= mem[ra];
    wa <= wa + 1'b1;
  end
endmodule

// d delayed D clocks (D of 2 or more) through a memory of 2^A >= D words in MLABs: each clock's word written at the write
// address, and the word written D - 1 clocks before read into q. The read is never of the address being written.
module logos_delay_mlab #(parameter W = 32, D = 3) (input clk, input [W-1:0] d, output reg [W-1:0] q);
  localparam A = $clog2(D);
  (* ramstyle = "MLAB" *) reg [W-1:0] mem [0:(1 << A) - 1];
  localparam [A-1:0] BACK = D - 1;
  reg [A-1:0] wa = {A{1'b0}};
  wire [A-1:0] ra = wa - BACK;
  always @(posedge clk) begin
    mem[wa] <= d;
    q <= mem[ra];
    wa <= wa + 1'b1;
  end
endmodule
"#;

/// When a memory reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    Falling,
    Rising,
    Held,
    Enabled,
}

/// The memories `name` names, their read `read`. A registered or held memory (the vector units' registers, the token-row
/// store) is, each width, a memory of its own (`<name>_part`, the depth it is given) and the memory callers name: built as
/// the part is, but for a word as wide as an M20K block's widest (40 bits) or wider more than one layer of 512 words deep
/// in a number of layers that is no power of two. That one is stacks of powers of two layers, the deepest its own and
/// each other a part beside it, each word in the stack the address's layers name (the deepest from address 0), and its
/// read the stack the read address named: the fitter builds a memory's depth as a stack of a power of two layers (1,088
/// words in 2,048 words' blocks, 57,344 in 65,536's). A falling-edge or enabled memory is one memory a width.
fn memories(widths: &[usize], name: fn(usize) -> String, what: &str, read: Read) -> String {
    let mut v = String::from(RAMSTYLE);
    for &w in widths {
        assert!(w > 0 && w % 8 == 0, "a memory {w} bits wide is not whole bytes");
        let name = name(w);
        let (ports, connect, edge, write) = match read {
            Read::Falling => ("", "", "always @(negedge clk)", "we"),
            Read::Rising => ("", "", "always @(posedge clk)", "we"),
            Read::Held => ("input re, ", ".re(re), ", "always @(posedge clk) if (re)", "we"),
            Read::Enabled => ("input ce, input cr, ", ".ce(ce), .cr(cr), ", "always @(posedge clk) if (cr)", "ce && we"),
        };
        let bytes = |mem: &str, at: &str| -> String {
            (0..w / 8).map(|b| format!("      if (wm[{b}]) {mem}[{at}][{}:{}] <= wd[{}:{}];\n", 8 * b + 7, 8 * b, 8 * b + 7, 8 * b)).collect()
        };
        let edges = if read == Read::Enabled { "s ce is high on" } else { "" };
        let stacked = matches!(read, Read::Rising | Read::Held);
        writeln!(v, "// A block-RAM memory of {w}-bit words: {what}, writes on the rising").unwrap();
        writeln!(v, "// edge{edges} under a byte mask.").unwrap();
        writeln!(v, "module {name}{} #(parameter D = 1024, A = 10)", if stacked { "_part" } else { "" }).unwrap();
        writeln!(v, "  (input clk,").unwrap();
        writeln!(v, "   {ports}input [A-1:0] ra, output reg [{}:0] rd,", w - 1).unwrap();
        writeln!(v, "   input we, input [A-1:0] wa, input [{}:0] wd, input [{}:0] wm);", w - 1, w / 8 - 1).unwrap();
        writeln!(v, "  `LOGOS_RAMSTYLE(D) reg [{}:0] mem [0:D-1];", w - 1).unwrap();
        writeln!(v, "  {edge} rd <= mem[ra];").unwrap();
        writeln!(v, "  always @(posedge clk)").unwrap();
        writeln!(v, "    if ({write}) begin").unwrap();
        v.push_str(&bytes("mem", "wa"));
        writeln!(v, "    end").unwrap();
        writeln!(v, "endmodule").unwrap();
        if !stacked {
            continue;
        }
        writeln!(v).unwrap();
        if w >= 40 {
            writeln!(v, "// {name}_part, but a memory more than one layer of 512 words deep in a number of layers no power of two").unwrap();
            writeln!(v, "// as stacks of powers of two layers: the deepest its own from address 0, each other a part.").unwrap();
        } else {
            writeln!(v, "// {name}_part, the memory callers name.").unwrap();
        }
        writeln!(v, "module {name} #(parameter D = 1024, A = 10)").unwrap();
        writeln!(v, "  (input clk,").unwrap();
        writeln!(v, "   {ports}input [A-1:0] ra, output [{}:0] rd,", w - 1).unwrap();
        writeln!(v, "   input we, input [A-1:0] wa, input [{}:0] wd, input [{}:0] wm);", w - 1, w / 8 - 1).unwrap();
        writeln!(v, "  localparam [31:0] LAYERS = (D + 511) / 512;").unwrap();
        writeln!(v, "  localparam SPLIT = {} && (LAYERS & (LAYERS - 1)) != 0;", if w >= 40 { "1" } else { "0" }).unwrap();
        writeln!(v, "  localparam TOP = $clog2(LAYERS + 1) - 1;").unwrap();
        writeln!(v, "  localparam P0 = SPLIT ? 512 << TOP : D, A0 = SPLIT ? 9 + TOP : A;").unwrap();
        writeln!(v, "  localparam [A:0] OWN = P0;").unwrap();
        writeln!(v, "  `LOGOS_RAMSTYLE(P0) reg [{}:0] mem [0:P0-1];", w - 1).unwrap();
        writeln!(v, "  reg [{}:0] rd0;", w - 1).unwrap();
        writeln!(v, "  {edge} rd0 <= mem[ra[A0-1:0]];").unwrap();
        writeln!(v, "  always @(posedge clk)").unwrap();
        writeln!(v, "    if ({write} && (!SPLIT || {{1'b0, wa}} < OWN)) begin").unwrap();
        v.push_str(&bytes("mem", "wa[A0-1:0]"));
        writeln!(v, "    end").unwrap();
        writeln!(v, "  generate").unwrap();
        writeln!(v, "    if (SPLIT) begin : stacks").unwrap();
        writeln!(v, "      // Each stack's word, zero unless the read was of it.").unwrap();
        writeln!(v, "      wire [{}:0] got [0:23];", w - 1).unwrap();
        writeln!(v, "      reg own;").unwrap();
        writeln!(v, "      {edge} own <= {{1'b0, ra}} < OWN;").unwrap();
        writeln!(v, "      genvar j;").unwrap();
        writeln!(v, "      for (j = 0; j < 24; j = j + 1) begin : layer").unwrap();
        writeln!(v, "        if (j < TOP && LAYERS[j]) begin : stack").unwrap();
        writeln!(v, "          localparam [31:0] FROM = 512 * (LAYERS & ~((32'd2 << j) - 32'd1)), TO = FROM + (512 << j);").unwrap();
        writeln!(v, "          localparam [A:0] BASE = FROM[A:0], LIMIT = TO[A:0];").unwrap();
        writeln!(v, "          wire [{}:0] q;", w - 1).unwrap();
        writeln!(v, "          reg mine;").unwrap();
        writeln!(v, "          {edge} mine <= {{1'b0, ra}} >= BASE && {{1'b0, ra}} < LIMIT;").unwrap();
        writeln!(v, "          {name}_part #(.D(512 << j), .A(9 + j)) part(.clk(clk), {connect}.ra(ra[8+j:0]), .rd(q),").unwrap();
        writeln!(v, "            .we(we && {{1'b0, wa}} >= BASE && {{1'b0, wa}} < LIMIT), .wa(wa[8+j:0]), .wd(wd), .wm(wm));").unwrap();
        writeln!(v, "          assign got[j] = mine ? q : {{{w}{{1'b0}}}};").unwrap();
        writeln!(v, "        end else begin : none").unwrap();
        writeln!(v, "          assign got[j] = {{{w}{{1'b0}}}};").unwrap();
        writeln!(v, "        end").unwrap();
        writeln!(v, "      end").unwrap();
        writeln!(v, "      reg [{}:0] word;", w - 1).unwrap();
        writeln!(v, "      integer k;").unwrap();
        writeln!(v, "      always @* begin").unwrap();
        writeln!(v, "        word = own ? rd0 : {{{w}{{1'b0}}}};").unwrap();
        writeln!(v, "        for (k = 0; k < 24; k = k + 1) word = word | got[k];").unwrap();
        writeln!(v, "      end").unwrap();
        writeln!(v, "      assign rd = word;").unwrap();
        writeln!(v, "    end else begin : whole").unwrap();
        writeln!(v, "      assign rd = rd0;").unwrap();
        writeln!(v, "    end").unwrap();
        writeln!(v, "  endgenerate").unwrap();
        writeln!(v, "endmodule").unwrap();
    }
    v
}
