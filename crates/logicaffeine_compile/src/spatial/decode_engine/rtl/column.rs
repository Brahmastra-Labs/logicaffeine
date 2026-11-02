//! The streaming engine's matrix columns as Verilog: a matrix streamed block-major within groups of row pairs, each
//! (row, token-row) still the row's blocks added in order, so every result is
//! [`crate::spatial::decode_engine::numerics::dot`]'s.
//!
//! A column takes its channel's weight stream a group of rows at a time and every row pair's block `b` before any
//! pair's block `b + 1`. A block's activations then serve every pair of the group in turn, and each (pair, token-row)
//! sum comes round once a group's pairs and token-rows have passed, which is what lets the block step be pipelined
//! without reordering a row's sum.
//!
//! What every core of a column array would otherwise compute alike is computed once beside them. A beat (a pair's two
//! blocks) is decoded once, by `logos_column_decode_<set>`, into its blocks' codes, their units' multipliers and offset
//! terms as bytes, their scales, and their units' sums of codes; a token-row's activation block is written to block RAM
//! as each activation word's excess codes and their units' sums (`logos_column_act_word`), so a read serves every core.
//!
//! `logos_column_core_<set>` is a column's datapath: it rings decoded beats — the one its issues run at the ring's head
//! and those queued behind it, each put in as its feed decodes it — and takes the next by moving the head, so a step
//! moves a pointer and never a beat. It runs each issue of the beat at the head against a token-row as passes, a clock
//! each: one per 4-bit digit of the format's codes and, with two activation words, per
//! word, each pass the beat's codes and the token-row's word through one digit and one word of systolic processing
//! elements (`logos_matvec_pass_p`, [`PASS_LATENCY`] clocks), its partials summed in their places as they leave them,
//! the pass's flags and the fields its lanes take delayed beside them. The issue's last pass hands the sums to two pipelined
//! lanes (`logos_lane_value_p`), and the value is added into its sum through the pipelined `logos_fadd_p`; a sum's last
//! block leaves the add as the sum's emission while its place goes back to +0 for the next group. An issue whose sum is
//! still in the add waits at its last pass.
//!
//! `logos_column_<set>` is one core with its own sequencer over groups of row pairs: it takes the next beat the clock
//! it issues the last token-row of the one it holds, so a block step of fewer passes than the add has stages takes
//! [`FADD_LATENCY`] + 1 clocks and a larger one a clock a pass ([`column_clocks`]).
//!
//! `logos_column_array_<set>` is the streaming engine's matvec: `CH · N` cores, `N` on each memory channel, one row
//! pair each of their channel's tile of a stripe ([`super::super::image::ImageLayout::stripe`]), every core running the
//! same pass of the same token-row of the same block step each clock against one activation read. A step issues when
//! every core whose channel's tile holds its pair has its beat; each channel's `logos_column_feed_<set>` puts its cores'
//! decoded beats into their rings as it deals them, and the cores take the next step's the clock the last token-row
//! issues ([`column_array_clocks`]).
//!
//! A set with octets ([`ColumnSet::octets`]) takes a Q8_0 matrix's blocks eight at a time: a beat holds a row's eight
//! blocks (`logos_dec_q8_0x8`, 256 elements in 272 bytes), the processing elements run every unit of them in one issue,
//! and the array keeps each issue's sums in a slot of one of two buffers instead of handing them to the lanes. A full
//! buffer's blocks then leave for the lanes a block a clock — every token-row's block 0, then every token-row's block 1,
//! and so on — each as the lone Q8_0 block an issue carries otherwise (its two units the lane's first two, its scale and
//! its activation block's scale), and each the clock no add still holds its sum, so a sum's blocks are added in order.
//! While one buffer's blocks leave, the next step's issues fill the other: a Q8_0 block costs the lanes a clock instead
//! of an issue of two passes.
//!
//! `logos_matvec_pass_x2` ([`matvec_pass_x2_verilog`]) is two cores' passes on one set of processing elements in a
//! clock of twice the cores' (`clk2`, its rising edges on `clk`'s and between them): the cores of a channel run in
//! lockstep against one activation word, so the elements take core a's codes with the word at `clk2`'s edge between
//! `clk`'s and core b's at `clk`'s next, and each core's partials leave as `logos_matvec_pass_p` gives them.

use std::fmt::Write as _;

use super::array::{CHAIN, PE8_P_LATENCY};
use super::fp32::FADD_LATENCY;
use super::lane::LANE_VALUE_LATENCY;
use crate::model::quant::GgmlType;

use super::super::numerics::Words;
use super::weights::{decoder_name, Decoder, DECODERS, UNIT};

/// Clocks from a pass's operands to its partial sums in `logos_matvec_pass_p`: the systolic elements'.
pub const PASS_LATENCY: usize = PE8_P_LATENCY;

/// Clocks from a block's issue to its value at the column's add: the activations' read (the codes' register beside it),
/// the processing elements, the lanes' input registers, and the lane.
pub const COLUMN_VALUE_LATENCY: usize = LANE_VALUE_LATENCY + 2 + PASS_LATENCY;

/// The passes an issue of a block of `ty` takes: one per 4-bit digit of its codes, for each activation word.
pub fn column_passes(ty: GgmlType, two: bool) -> usize {
    let d = DECODERS.iter().find(|d| d.ty == ty).unwrap_or_else(|| panic!("no decoder reads {}", ty.name()));
    (if d.code_bits > 4 { 2 } else { 1 }) * (if two { 2 } else { 1 })
}

/// The job's format a clock behind it, a module's own copy (`fmt_q`, never merged with another module's): a job holds its
/// format from its start to its end and no module reads it in the job's first clock, so the job's word drives a
/// register in each module and none of their datapaths' selects.
pub(crate) const FMT_COPY: &str = "  (* dont_merge, maxfan = 256 *) reg [3:0] fmt_q;\n  always @(posedge clk) fmt_q <= fmt;\n";

/// The clocks from the edge that takes `start` to the edge that raises the column's last sum, for a stream that offers
/// a beat whenever the column is ready and issues of `passes` passes: a clock to take the first beat; a block step
/// (every pair's beat of one block against every token-row) of `pairs · tokens` issues, a pass a clock, its first issue's
/// last pass at least [`FADD_LATENCY`] + 1 clocks after the step before's, waiting for the sum that one left in the add;
/// then the last issue's value through the lane and the add.
pub fn column_clocks(groups: usize, pairs: usize, blocks: usize, tokens: usize, passes: usize) -> usize {
    let (steps, issues) = (groups * blocks, pairs * tokens);
    (steps - 1) * (passes * issues).max(FADD_LATENCY + 1) + passes * issues + 1 + COLUMN_VALUE_LATENCY + FADD_LATENCY
}

/// The clocks from the edge that takes `start` to the edge that raises the array's last sums, for streams that offer
/// every core its beat whenever it is ready: a stripe's block step is one pair a core against every token-row, so the
/// array runs as one column of one pair a group over the stripes.
pub fn column_array_clocks(stripes: usize, blocks: usize, tokens: usize, passes: usize) -> usize {
    column_clocks(stripes, 1, blocks, tokens, passes)
}

/// [`column_array_clocks`] for an octet job ([`ColumnSet::octets`]): `octets` of a row's eight blocks each a stripe's
/// block step. A step's issues run a pass a clock once the step before has issued its last, each into its buffer's slot,
/// the first waiting for its buffer to be free; a buffer's blocks are asked for from four clocks after its last issue,
/// block-major, a clock each and no sooner than [`FADD_LATENCY`] + 1 clocks after its sum's block before, and the
/// buffer is free the clock after its last; the last block's sum leaves as an issue's would.
pub fn column_array_octet_clocks(stripes: usize, octets: usize, tokens: usize, passes: usize) -> usize {
    column_array_octet_lane_clocks(stripes, octets, tokens, passes, 1)
}

/// [`column_array_octet_clocks`] for a set of `lanes` octet lanes a row ([`ColumnSet::with_octet_lanes`]): a buffer's
/// blocks asked for `lanes` at once — a token-row's next `lanes` — a clock each group, no sooner than `lanes` adds and a
/// clock after its sum's group before, the last group's sum leaving through the chain's `lanes` adds.
pub fn column_array_octet_lane_clocks(stripes: usize, octets: usize, tokens: usize, passes: usize, lanes: usize) -> usize {
    let gap = lanes * FADD_LATENCY + 1;
    let mut free = [0usize; 2];
    let mut fin = 0usize;
    let mut emitted = 0usize;
    let mut last: Vec<Option<usize>> = vec![None; tokens];
    for k in 0..stripes * octets {
        let first = (fin + passes).max(free[k % 2]);
        fin = first + passes * (tokens - 1);
        let mut at = (fin + 4).max(emitted + 1);
        for _group in 0..8 / lanes {
            for sum in last.iter_mut() {
                if let Some(before) = *sum {
                    at = at.max(before + gap);
                }
                *sum = Some(at);
                emitted = at;
                at += 1;
            }
        }
        free[k % 2] = emitted + 1;
    }
    emitted + 1 + COLUMN_VALUE_LATENCY + lanes * FADD_LATENCY
}

/// A set of formats one column is built for, under one name, and the activation words its jobs run at: built for one
/// word, every module of the set has no `two` and its lanes are one word wide. With `octets`, a Q8_0 matrix's blocks
/// come eight to one of the column's (its octets, [`ColumnSet::block_len`]): an issue's units hold a row's eight
/// blocks, and the lane takes them one a clock, each still a block of its own. With a `slot` past 1 every block comes
/// in the image padded with zeros to a multiple of `slot` bytes (`ImageLayout::slot`), so the feed gathers its beats
/// `slot` bytes at a time. With `lanes` past 1 an octet's blocks leave for `lanes` lanes a row a clock, a token-row's
/// next `lanes` blocks at once: the row's full lane and `lanes − 1` lanes of one scale (`logos_lane_slim_p`), their
/// values added into the row's sum in block order through a chain of `lanes` adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnSet {
    pub name: String,
    pub formats: Vec<GgmlType>,
    pub words: Words,
    pub octets: bool,
    pub slot: usize,
    pub lanes: usize,
    pub pe_pairs: bool,
    pub token_pairs: bool,
}

impl ColumnSet {
    /// The set `formats` under `name`, for jobs at `words`.
    pub fn new(name: &str, formats: &[GgmlType], words: Words) -> ColumnSet {
        ColumnSet { name: name.to_string(), formats: formats.to_vec(), words, octets: false, slot: 1, lanes: 1, pe_pairs: false, token_pairs: false }
    }

    /// The set with its Q8_0 matrices' blocks taken eight at a time.
    pub fn with_octets(self) -> ColumnSet {
        assert!(self.formats.contains(&GgmlType::Q8_0), "a set without Q8_0 has no octets to take");
        ColumnSet { octets: true, ..self }
    }

    /// The set with an octet's blocks leaving for `lanes` lanes a row a clock: 1, 2, 4 or 8, a divisor of an octet's
    /// eight blocks.
    pub fn with_octet_lanes(self, lanes: usize) -> ColumnSet {
        assert!(self.octets, "octet lanes take an octet's blocks: a set with octets");
        assert!(matches!(lanes, 1 | 2 | 4 | 8), "{lanes} lanes a row: a divisor of an octet's eight blocks");
        ColumnSet { lanes, ..self }
    }

    /// The set with its processing elements in a clock of twice its cores' (the array's `clk2`), each set of them
    /// taking two issues' passes a clock. An array's cores run in lockstep against one activation word, so a pair of
    /// cores' elements take one core's codes and then the other's (`logos_matvec_pass_x2`); with token pairs
    /// ([`ColumnSet::with_token_pairs`]) each core's own elements take its beat's codes against the pair's first word
    /// and then its second (`logos_matvec_pass_x2t`). Half the elements for the same passes, each core's clocks as they
    /// were.
    pub fn with_pe_pairs(self) -> ColumnSet {
        ColumnSet { pe_pairs: true, ..self }
    }

    /// The set with each issue of its array's held beat run against two token-rows a clock, token-rows `t` and `t + 1`
    /// for even `t` (`token_pairs`): each core takes two activation words a clock, one from each half of its channel's
    /// copy of the activations (a word a token pair, written a half at a time under the mask), through processing
    /// elements and lanes of each, the beat and the issue's flags and fields shared; a sum's place is a token pair's, and
    /// a step issues ⌈tokens / 2⌉ times ([`ColumnSet::issues`]). The ring, the core's control and its channel's feed
    /// then serve twice the multiplies. An odd count's last pair runs its second token-row on whatever its half holds,
    /// and the array's consumer takes only the token-rows the job has. With [`ColumnSet::pe_pairs`] the pair's two words
    /// run on one set of the core's elements in clk2. With [`ColumnSet::lanes`] past one each token-row of the pair has
    /// its octet lanes.
    pub fn with_token_pairs(self) -> ColumnSet {
        ColumnSet { token_pairs: true, ..self }
    }

    /// The token-rows one issue of the set's array runs against: two with token pairs, one otherwise.
    pub fn issue_rows(&self) -> usize {
        if self.token_pairs { 2 } else { 1 }
    }

    /// The issues a block step of `tokens` token-rows takes: ⌈tokens / [`ColumnSet::issue_rows`]⌉.
    pub fn issues(&self, tokens: usize) -> usize {
        tokens.div_ceil(self.issue_rows())
    }

    /// The set with its blocks in slots of `slot` bytes: a power of two a line's 64 bytes are a whole number of.
    pub fn with_slots(self, slot: usize) -> ColumnSet {
        assert!(slot.is_power_of_two() && slot <= 64, "a slot of {slot} bytes: a power of two no wider than a line");
        ColumnSet { slot, ..self }
    }

    /// Whether `ty`'s blocks come in octets.
    pub fn octet(&self, ty: GgmlType) -> bool {
        self.octets && ty == GgmlType::Q8_0
    }

    /// The elements of one of the column's blocks of `ty`: its format's block, or an octet's 256.
    pub fn block_len(&self, ty: GgmlType) -> usize {
        if self.octet(ty) { 8 * ty.block_len() } else { ty.block_len() }
    }

    /// The bytes of one of the column's blocks of `ty`: its format's block, or an octet's 272.
    pub fn block_bytes(&self, ty: GgmlType) -> usize {
        if self.octet(ty) { 8 * ty.block_bytes() } else { ty.block_bytes() }
    }

    /// The bytes one of the column's blocks of `ty` takes in the image: its block's, then zeros to the set's slot.
    pub fn slot_bytes(&self, ty: GgmlType) -> usize {
        self.block_bytes(ty).next_multiple_of(self.slot)
    }

    /// A hybrid engine's set: `formats` in their ggml ids' order under the name `step`, so one build's set and the
    /// programs written for it place every format alike.
    pub fn hybrid(formats: &[GgmlType], words: Words) -> ColumnSet {
        let mut f = formats.to_vec();
        f.sort_by_key(|t| t.id());
        f.dedup();
        ColumnSet::new("step", &f, words)
    }

    /// The set as a program and a build name it: its formats in order and its word count, `Q8_0+Q4_K:1`, a format
    /// taken in octets `Q8_0x8`, blocks in slots of 16 bytes `@16`.
    pub fn signature(&self) -> String {
        let names: Vec<String> = self.formats.iter().map(|&t| if self.octet(t) { format!("{}x8", t.name()) } else { t.name().to_string() }).collect();
        let slot = if self.slot > 1 { format!("@{}", self.slot) } else { String::new() };
        format!("{}:{}{slot}", names.join("+"), self.word_count())
    }

    /// The elements of the set's widest block.
    pub fn elems(&self) -> usize {
        self.formats.iter().map(|&t| self.block_len(t)).max().expect("a set of formats")
    }

    /// The activation words a job of the set may run at.
    pub fn word_count(&self) -> usize {
        if self.words == Words::Two {
            2
        } else {
            1
        }
    }

    /// Bits of one decoded block: its codes, its units' multipliers and offset terms as bytes, its two scales and its
    /// units' sums of codes ([`ColumnSet::decoded_fields`]), and with octets an octet's seven scales past its first; a
    /// decoded beat is two, block one's below block two's.
    pub fn decoded_bits(&self) -> usize {
        let e = self.elems();
        8 * e + 28 * (e / UNIT) + 32 + if self.octets { 7 * 16 } else { 0 }
    }

    /// Where each field of a decoded block starts: the codes, the multipliers, the offset terms, the two scales, the
    /// units' sums of codes, and an octet's seven scales past its first.
    pub fn decoded_fields(&self) -> DecodedFields {
        let (e, u) = (self.elems(), self.elems() / UNIT);
        let q = 8 * e + 16 * u + 32;
        DecodedFields { code: 0, c: 8 * e, n: 8 * e + 8 * u, a: 8 * e + 16 * u, b: 8 * e + 16 * u + 16, q, s7: q + 12 * u }
    }

    /// The activation scales a word holds: one, or with octets an octet's eight blocks'.
    pub fn act_slots(&self) -> usize {
        if self.octets { 8 } else { 1 }
    }

    /// Bits of an activation word in block RAM, whole bytes: each word's excess codes, then each word's units' sums of
    /// them, then each scale's significand and exponent, 24 bits a scale ([`ColumnSet::act_fields`]).
    pub fn act_bits(&self) -> usize {
        let (e, w) = (self.elems(), self.word_count());
        (w * (8 * e + 12 * (e / UNIT)) + 20 + 24 * (self.act_slots() - 1)).div_ceil(8) * 8
    }

    /// Where each field of an activation word starts: word `k`'s excess codes at `excess + k·8E`, its units' sums at
    /// `sums + k·12U`, scale `j`'s significand at `m + 24j` and its exponent at `e + 24j`.
    pub fn act_fields(&self) -> ActFields {
        let (e, w) = (self.elems(), self.word_count());
        let sums = w * 8 * e;
        let m = sums + w * 12 * (e / UNIT);
        ActFields { excess: 0, sums, m, e: m + 8 }
    }
}

/// The bit offsets of a decoded block's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedFields {
    pub code: usize,
    pub c: usize,
    pub n: usize,
    pub a: usize,
    pub b: usize,
    pub q: usize,
    pub s7: usize,
}

/// The bit offsets of an activation word's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActFields {
    pub excess: usize,
    pub sums: usize,
    pub m: usize,
    pub e: usize,
}

/// The columns for every format, each its own set for both word counts, with the block steps, decoders, lanes, f32
/// units and block RAM they are built from.
pub fn column_verilog() -> String {
    let sets: Vec<ColumnSet> = DECODERS.iter().map(|d| ColumnSet::new(&d.ty.name().to_lowercase(), &[d.ty], Words::Two)).collect();
    columns_verilog(&sets)
}

/// The columns of each set, with the modules they are built from: `logos_column_decode_<name>`,
/// `logos_column_core_<name>`, `logos_column_<name>`, `logos_column_array_<name>` and `logos_column_feed_<name>`. A
/// set's core holds a block of any of its formats, its widths the set's widest; with more than one format each module
/// takes the job's format as `fmt`, its place in the set, a block of fewer elements than the widest running with the
/// units past it zero.
pub fn columns_verilog(sets: &[ColumnSet]) -> String {
    let mut v = String::from(super::ram::RAMSTYLE);
    v.push_str(&super::matvec::matvec_verilog());
    let mut widths: Vec<usize> = sets.iter().map(ColumnSet::act_bits).collect();
    widths.extend(sets.iter().filter(|s| s.token_pairs).map(|s| 2 * s.act_bits()));
    widths.sort();
    widths.dedup();
    v.push_str(&super::ram::bram_verilog(&widths));
    v.push_str(super::ram::delay_verilog());
    v.push_str(&MATVEC_PASS.replace("@PE@", &PASS_LATENCY.to_string()));
    v.push_str(ACT_STAGGER);
    v.push_str(ACT_WORD);
    for set in sets {
        let ColumnSet { name, formats, words, .. } = set;
        let decs: Vec<Coded> = formats
            .iter()
            .map(|&ty| Coded {
                dec: DECODERS.iter().find(|d| d.ty == ty).unwrap_or_else(|| panic!("no decoder reads {}", ty.name())),
                octet: set.octet(ty),
                slot: set.slot,
            })
            .collect();
        let elems = set.elems();
        let bits = 8 * decs.iter().map(Coded::bytes).max().unwrap();
        let units = elems / UNIT;
        let db = set.decoded_bits();
        let f = set.decoded_fields();
        let a = set.act_fields();
        let multi = decs.len() > 1;
        let wide = *words == Words::Two;
        let fmt_in = if multi { ", input [3:0] fmt" } else { "" };
        let fmt_pass = if multi { ".fmt(fmt), " } else { "" };
        let fmt_copy = if multi { FMT_COPY } else { "" };
        let fmt_dec = if multi { ".fmt(fmt_q), " } else { "" };
        let two_in = if wide { ", input two" } else { "" };
        let two_pass = if wide { ".two(two), " } else { "" };
        let two_decl = if wide { "" } else { "  wire two = 1'b0;\n" };
        let what = decs.iter().map(Coded::name).collect::<Vec<_>>().join("/");
        let (code_top, bus12, bus24) = (8 * elems - 1, 12 * units - 1, 24 * units - 1);
        let oct = core_octets(set);
        let lane = lane_fields(set);
        // Cores share elements in pairs only without token pairs: with them, each core's own elements run its pair's two
        // words in clk2.
        let paired = set.pe_pairs && !set.token_pairs;
        let arr: std::collections::HashMap<&str, String> = array_octets(set).into_iter().collect();
        // The pass's word of the activations: its excess codes (chosen in the column by the word registered beside the
        // activation read, for the stagger) and their unit sums.
        let (st_sel, s_sel) = if wide {
            (
                format!("word_a ? act_word[{}:{}] : act_word[{}:{}]", a.excess + 16 * elems - 1, a.excess + 8 * elems, a.excess + code_top, a.excess),
                format!("s1_word ? act_word[{}:{}] : act_word[{}:{}]", a.sums + 24 * units - 1, a.sums + 12 * units, a.sums + bus12, a.sums),
            )
        } else {
            (format!("act_word[{}:{}]", a.excess + code_top, a.excess), format!("act_word[{}:{}]", a.sums + bus12, a.sums))
        };
        writeln!(
            v,
            r#"
// A {what} beat decoded: each block's codes, its units' multipliers and offset terms as bytes, its two scales and its
// units' sums of codes (sixteen codes in a balanced tree of four adds), block one's fields below block two's.
module logos_column_decode_{name}(input [{top}:0] blk1, input [{top}:0] blk2{fmt_in}, output [{beat_top}:0] beat);
  function [11:0] sum16(input [127:0] c);
    reg [8:0] a0, a1, a2, a3, a4, a5, a6, a7;
    reg [9:0] b0, b1, b2, b3;
    reg [10:0] d0, d1;
    begin
      a0 = c[7:0] + c[15:8]; a1 = c[23:16] + c[31:24]; a2 = c[39:32] + c[47:40]; a3 = c[55:48] + c[63:56];
      a4 = c[71:64] + c[79:72]; a5 = c[87:80] + c[95:88]; a6 = c[103:96] + c[111:104]; a7 = c[119:112] + c[127:120];
      b0 = a0 + a1; b1 = a2 + a3; b2 = a4 + a5; b3 = a6 + a7;
      d0 = b0 + b1; d1 = b2 + b3;
      sum16 = d0 + d1;
    end
  endfunction
{decode}  genvar u;
  generate
    for (u = 0; u < {units}; u = u + 1) begin : unit
      wire [11:0] q1 = sum16(code1_d[128*u +: 128]);
      wire [11:0] q2 = sum16(code2_d[128*u +: 128]);
      assign beat[{fc} + 8*u +: 8] = c1_d[16*u +: 8];
      assign beat[{fn} + 8*u +: 8] = n1_d[16*u +: 8];
      assign beat[{fq} + 12*u +: 12] = q1;
      assign beat[{db} + {fc} + 8*u +: 8] = c2_d[16*u +: 8];
      assign beat[{db} + {fn} + 8*u +: 8] = n2_d[16*u +: 8];
      assign beat[{db} + {fq} + 12*u +: 12] = q2;
    end
  endgenerate
  assign beat[{code_top}:0] = code1_d;
  assign beat[{db} + {code_top}:{db}] = code2_d;
  assign beat[{fa} +: 16] = a1_d;
  assign beat[{fb} +: 16] = b1_d;
  assign beat[{db} + {fa} +: 16] = a2_d;
  assign beat[{db} + {fb} +: 16] = b2_d;
{beat_s7}endmodule

// A {what} column's datapath: the decoded beats it rings (each put in on `put`, the beat at the ring's head the one it
// holds, the next taken on `load` by moving the head), and each pass of an issue of the beat held, the beat and
// the pass's word of the activations (`act_word`, the clock after `pass`: the word's excess codes and their unit sums;
// its excess codes staggered for the elements in `act_st`) through the systolic processing elements of the pass's code
// digit, its partials summed into its word's in their place
// from the issue's `first` pass as they leave the elements PL clocks on, with the pass's flags and its lanes' fields
// delayed to meet them. The issue's last pass (`issue`) hands the sums to the two rows' lanes and the pipelined add. The
// value of the issue LAT clocks before lands on the sum `land` names; the sum leaves the add ADD clocks later as the sum
// `done` names and is written back, or, its last block (`last`), emitted and its place reset to +0. `clear` resets every
// sum to +0.
module logos_column_core_{name} #(parameter SUMS = 4, SW = 2, RING = 2)
  (input clk, input rst, input clear{two_in}{fmt_in},
   input put, input load, input [{beat_top}:0] w_beat,
   input pass, input digit, input word, input first, input issue, input [{act_top}:0] act_word, input [{code_top}:0] act_st,
   input [SW-1:0] land, input [SW-1:0] done, input last{core_octet_ports},
   output reg o_valid, output reg [31:0] o_val1, output reg [31:0] o_val2);
  localparam LAT = {lat}, ADD = {add}, TAGS = LAT + ADD;
{two_decl}{fmt_copy}{zsel}  // The ring: RING places, the beat held at its head and the beats queued behind it, a beat put in at the place after the
  // last; a step's next beat is the head's next place, so taking it moves the head and no beat.
  localparam RA = (RING > 2) ? 2 : 1;
  localparam [RA-1:0] R0 = 0, R1 = 1, RLAST = RING - 1;
  `LOGOS_RAMSTYLE_ASYNC(RING) reg [{beat_top}:0] ring [0:RING-1];
  // The ring's places address every MLAB of the beat's width: registers Quartus copies beside each group of them.
  (* maxfan = 64 *) reg [RA-1:0] rp, wp;
  always @(posedge clk) begin
    if (put) begin ring[wp] <= w_beat; wp <= (wp == RLAST) ? R0 : wp + R1; end
    if (load) rp <= (rp == RLAST) ? R0 : rp + R1;
    if (rst) begin rp <= R0; wp <= R1; end
  end
  wire [{beat_top}:0] held = ring[rp];
  // The pass's flags, registered beside its word of the activations; its codes go to the elements from the beat held,
  // a clock before the word.
  reg s1_pass, s1_digit, s1_word, s1_first;
  always @(posedge clk) begin s1_pass <= pass; s1_digit <= digit; s1_word <= word; s1_first <= first; end
  wire [{code_top}:0] code1 = held[{code_top}:0], code2 = held[{db} + {code_top}:{db}];
  wire [{bus12}:0] s_act = {s_sel};
  wire [{bus24}:0] w_p1, w_p2;
  logos_matvec_pass_p #(.UNITS({units})) parts(.clk(clk), .code1(code1), .code2(code2), .w(act_st), .s(s_act), .digit(digit),
    .p1(w_p1), .p2(w_p2));
  // The pass's flags and the fields of its beat and activations the lanes and the slots take, PL clocks behind with its
  // partials (the beat's taken from the beat held at the pass, a clock before the activations'); the pass flag in
  // registers the reset clears.
  localparam PL = {pl}, LB = {lb_bits}, LA = {la_bits};
  reg [PL-1:0] d_pass, d_digit;
  reg [PL-2:0] d_word, d_first;
  // The word and first flags' last places drive every unit's sums: registers Quartus copies a copy each 256 loads.
  (* maxfan = 256 *) reg p_word, p_first;
  wire [LB-1:0] lb_in = {{{lb_in}}};
  wire [LA-1:0] la_in = {{{la_in}}};
  wire [LB-1:0] lfb;
  wire [LA-1:0] lfa;
  logos_delay #(.W(LB), .D(PL + 1)) lbd(.clk(clk), .d(lb_in), .q(lfb));
  logos_delay #(.W(LA), .D(PL)) lad(.clk(clk), .d(la_in), .q(lfa));
  always @(posedge clk) begin
    d_pass <= rst ? {{PL{{1'b0}}}} : {{d_pass[PL-2:0], s1_pass}};
    d_digit <= {{d_digit[PL-2:0], s1_digit}};
    d_word <= (d_word << 1) | {{{{(PL-2){{1'b0}}}}, s1_word}}; p_word <= d_word[PL-2];
    d_first <= (d_first << 1) | {{{{(PL-2){{1'b0}}}}, s1_first}}; p_first <= d_first[PL-2];
  end
  wire p_pass = d_pass[PL-1], p_digit = d_digit[PL-1];
{lf_out}  // The pass's partials, its digit's in their place, added to its word's sums so far (none at an issue's first pass).
  reg [{bus24}:0] a1_hi, a2_hi, a1_lo, a2_lo, t1_hi, t2_hi, t1_lo, t2_lo;
  reg [{bus12}:0] as_hi, as_lo, ts_hi, ts_lo;
  reg [23:0] sh1, sh2, b1_hi, b2_hi, b1_lo, b2_lo;
  integer u;
  always @* begin
    for (u = 0; u < {units}; u = u + 1) begin
      sh1 = p_digit ? {{w_p1[24*u +: 20], 4'd0}} : w_p1[24*u +: 24];
      sh2 = p_digit ? {{w_p2[24*u +: 20], 4'd0}} : w_p2[24*u +: 24];
      b1_hi = p_first ? 24'd0 : a1_hi[24*u +: 24];
      b2_hi = p_first ? 24'd0 : a2_hi[24*u +: 24];
      b1_lo = p_first ? 24'd0 : a1_lo[24*u +: 24];
      b2_lo = p_first ? 24'd0 : a2_lo[24*u +: 24];
      t1_hi[24*u +: 24] = p_word ? b1_hi : b1_hi + sh1;
      t2_hi[24*u +: 24] = p_word ? b2_hi : b2_hi + sh2;
      t1_lo[24*u +: 24] = p_word ? b1_lo + sh1 : b1_lo;
      t2_lo[24*u +: 24] = p_word ? b2_lo + sh2 : b2_lo;
    end
    ts_hi = p_word ? (p_first ? {{({units}*12){{1'b0}}}} : as_hi) : f_s;
    ts_lo = p_word ? f_s : (p_first ? {{({units}*12){{1'b0}}}} : as_lo);
  end
  always @(posedge clk) if (p_pass) begin
    a1_hi <= t1_hi; a2_hi <= t2_hi; a1_lo <= t1_lo; a2_lo <= t2_lo; as_hi <= ts_hi; as_lo <= ts_lo;
  end
{core_octets}  // The lanes' inputs: the sums over the issue's passes, registered the clock after its last{lanes_from}.
  reg [{bus24}:0] p1_hi, p2_hi, p1_lo, p2_lo;
  reg [{bus12}:0] s_hi, s_lo, q1, q2;
  reg [{unit_top}:0] lc1, ln1, lc2, ln2;
  reg [15:0] la1, lb1, la2, lb2;
  reg [7:0] lm;
  reg signed [11:0] le;
  always @(posedge clk) begin
{octet_lanes}    p1_hi <= t1_hi; p2_hi <= t2_hi; p1_lo <= t1_lo; p2_lo <= t2_lo;
    s_hi <= ts_hi; s_lo <= ts_lo; q1 <= f_q1; q2 <= f_q2;
    lc1 <= f_c1; ln1 <= f_n1; lc2 <= f_c2; ln2 <= f_n2;
    la1 <= f_a1; lb1 <= f_b1; la2 <= f_a2; lb2 <= f_b2;
    lm <= f_m;
    le <= f_e;
{octet_lanes_end}  end
  wire [31:0] value1, value2;
  logos_lane_value_p #(.UNITS({units}), .WORDS({words_n})) l1(.clk(clk), .p_hi(p1_hi), .s_hi(s_hi), .p_lo(p1_lo), .s_lo(s_lo),
    .q(q1), .c(lc1), .n(ln1), .z(z), .a(la1), .b(lb1), .m(lm), .e(le), .two(two), .value(value1));
  logos_lane_value_p #(.UNITS({units}), .WORDS({words_n})) l2(.clk(clk), .p_hi(p2_hi), .s_hi(s_hi), .p_lo(p2_lo), .s_lo(s_lo),
    .q(q2), .c(lc2), .n(ln2), .z(z), .a(la2), .b(lb2), .m(lm), .e(le), .two(two), .value(value2));
  // The two rows' sums: a value added as it lands, the sum written as it leaves the add.
  (* ramstyle = "logic" *) reg [31:0] acc1 [0:SUMS-1];
  (* ramstyle = "logic" *) reg [31:0] acc2 [0:SUMS-1];
  wire [31:0] sum1, sum2;
  logos_fadd_p add1(.clk(clk), .a(acc1[land]), .b(value1), .y(sum1));
  logos_fadd_p add2(.clk(clk), .a(acc2[land]), .b(value2), .y(sum2));
  // This core's issues in flight.
  reg alive [0:TAGS-1];
  integer k;
  always @(posedge clk) begin
    o_valid <= 1'b0;
    alive[0] <= issue;
    for (k = 1; k < TAGS; k = k + 1) alive[k] <= alive[k-1];
    if (alive[TAGS-1]) begin
      acc1[done] <= last ? 32'd0 : sum1;
      acc2[done] <= last ? 32'd0 : sum2;
      if (last) begin o_valid <= 1'b1; o_val1 <= sum1; o_val2 <= sum2; end
    end
    if (rst) begin
      for (k = 0; k < TAGS; k = k + 1) alive[k] <= 1'b0;
    end else if (clear) begin
      for (k = 0; k < SUMS; k = k + 1) begin acc1[k] <= 32'd0; acc2[k] <= 32'd0; end
    end
  end
endmodule

// A {what} matrix streamed block-major within groups of row pairs against up to T_MAX token-rows: for each group, every
// pair's block 0, then every pair's block 1, ..., a beat a pair's two blocks, decoded as it is taken. The beat held is
// issued against every token-row, a token-row a clock, an issue's coordinates riding beside it to its sum's emission, and
// the next beat is taken the clock the held one issues its last token-row.
module logos_column_{name} #(parameter T_MAX = 4, NB_MAX = 8, P_MAX = 4)
  (input clk, input rst,
   input start, input [15:0] groups, input [7:0] pairs, input [15:0] blocks, input [7:0] tokens{two_in}{fmt_in}, output busy,
   input act_wr, input [7:0] act_t, input [15:0] act_b, input [{codes_top}:0] act_codes, input [7:0] act_m, input [11:0] act_e,
   input w_valid, output w_ready, input [{top}:0] w_blk1, input [{top}:0] w_blk2,
   output o_valid, output reg [15:0] o_group, output reg [7:0] o_pair, output reg [7:0] o_t, output [31:0] o_val1,
   output [31:0] o_val2);
{two_decl}{fmt_copy}  localparam LAT = {lat}, ADD = {add}, TAGS = LAT + ADD;
  localparam AC = P_MAX * T_MAX, AW = (AC > 1) ? $clog2(AC) : 1;
  // The beat held, its group, block and pair, the token-row it issues next; the coordinates of the beat taken next.
  reg have, more;
  reg [15:0] hg, hb, ng, nb;
  reg [7:0] hp, np, t;
  wire [31:0] at_wide = hp * T_MAX + t;
  wire [AW-1:0] at = at_wide[AW-1:0];
  // Each issue in flight: whether it is one, its sum's place, its coordinates, and whether it is its sum's last block.
  reg live [0:TAGS-1];
  reg [AW-1:0] tag [0:TAGS-1];
  reg [15:0] tg [0:TAGS-1];
  reg [7:0] tp [0:TAGS-1];
  reg [7:0] tt [0:TAGS-1];
  reg last [0:TAGS-1];
  // An issue waits while an earlier issue of its sum is among the last ADD: that one's sum is written as it leaves the
  // add, ADD clocks after it lands, and this one, landing LAT clocks after it issues as that one did, must read it.
  reg stall, flight;
  integer h;
  always @* begin
    stall = 1'b0;
    flight = 1'b0;
    for (h = 0; h < ADD; h = h + 1) if (live[h] && tag[h] == at) stall = 1'b1;
    for (h = 0; h < TAGS; h = h + 1) if (live[h]) flight = 1'b1;
  end
  // An issue's passes, a clock each: its format's code digits, for each activation word; the last waits on a stall.
{digits}  wire [2:0] passes = two ? {{digits, 1'b0}} : {{1'b0, digits}};
  reg [2:0] pk;
  wire last_pass = pk + 3'd1 == passes;
  wire issue = have && last_pass && !stall;
  wire pass = have && (!last_pass || !stall);
  wire digit = digits[1] && pk[0];
  wire word = digits[1] ? pk[1] : pk[0];
  wire fin = issue && (t + 8'd1 == tokens);
  assign w_ready = more && (!have || fin);
  wire take = w_valid && w_ready;
  assign busy = have || more || flight;
  // The beat decoded as it is taken; each token-row's blocks of activations, as their words' excess codes and unit
  // sums, in block RAM.
  wire [{beat_top}:0] w_beat;
  logos_column_decode_{name} dec(.blk1(w_blk1), .blk2(w_blk2), {fmt_dec}.beat(w_beat));
  localparam AN = T_MAX * NB_MAX, AA = (AN > 1) ? $clog2(AN) : 1;
  wire [{act_top}:0] act_wd, act_word;
  logos_column_act_word #(.ELEMS({elems}), .WORDS({words_n}), .SLOTS({slots})) word_of(.codes(act_codes), .two(two), .m(act_m), .e(act_e),
    .slot({act_slot}),
    .word(act_wd));
  wire [31:0] act_ra = t * NB_MAX + hb, act_wa = act_t * NB_MAX + act_b;
  {act_ram} #(.D(AN), .A(AA)) act_ram(.clk(clk), .ra(act_ra[AA-1:0]), .rd(act_word), .we(act_wr),
    .wa(act_wa[AA-1:0]), .wd(act_wd), .wm({{{act_bytes}{{1'b1}}}}));
{stagger}  logos_column_core_{name} #(.SUMS(AC), .SW(AW)) core(.clk(clk), .rst(rst), .clear(start && !busy), {two_pass}{fmt_pass}.put(take), .load(take),
    .w_beat(w_beat), .pass(pass), .digit(digit), .word(word), .first(pk == 3'd0), .issue(issue), .act_word(act_word), .act_st(act_st),
    .land(tag[LAT-1]), .done(tag[TAGS-1]), .last(last[TAGS-1]), .o_valid(o_valid), .o_val1(o_val1), .o_val2(o_val2));
  integer k;
  always @(posedge clk) begin
    live[0] <= issue; tag[0] <= at; tg[0] <= hg; tp[0] <= hp; tt[0] <= t; last[0] <= (hb + 16'd1 == blocks);
    for (k = 1; k < TAGS; k = k + 1) begin
      live[k] <= live[k-1]; tag[k] <= tag[k-1]; tg[k] <= tg[k-1]; tp[k] <= tp[k-1]; tt[k] <= tt[k-1]; last[k] <= last[k-1];
    end
    if (live[TAGS-1] && last[TAGS-1]) begin o_group <= tg[TAGS-1]; o_pair <= tp[TAGS-1]; o_t <= tt[TAGS-1]; end
    if (pass) pk <= last_pass ? 3'd0 : pk + 3'd1;
    if (issue) t <= (t + 8'd1 == tokens) ? 8'd0 : t + 8'd1;
    if (take) begin
      hg <= ng; hb <= nb; hp <= np; have <= 1'b1;
      if (np + 8'd1 < pairs) np <= np + 8'd1;
      else begin
        np <= 8'd0;
        if (nb + 16'd1 < blocks) nb <= nb + 16'd1;
        else begin
          nb <= 16'd0;
          if (ng + 16'd1 < groups) ng <= ng + 16'd1; else more <= 1'b0;
        end
      end
    end else if (fin) have <= 1'b0;
    if (rst) begin
      have <= 1'b0; more <= 1'b0; t <= 8'd0; pk <= 3'd0;
      for (k = 0; k < TAGS; k = k + 1) live[k] <= 1'b0;
    end else if (start && !busy) begin
      ng <= 16'd0; nb <= 16'd0; np <= 8'd0; t <= 8'd0; pk <= 3'd0; more <= 1'b1;
    end
  end
endmodule

// A {what} matrix laid in stripes, each channel's tile of a stripe block-major, through CH * N cores, N on each channel:
// core c = ch * N + p takes row pair p of channel ch's tile of each stripe, its decoded beats in order. Stripe s's tile on
// channel ch holds the tensor's rows from (CH * s + turns[ch]) * 2N, fewer than 2N only at the tensor's end; a core whose
// pair's first row is past the tensor takes no beat that stripe. Every core runs the same pass of the same token-row of
// the same block step each clock against one read of the activations; a step issues once every core the stripe holds a
// pair for has its beat, and the cores take the next step's beats the clock the last token-row issues: core c's feed
// puts each of its beats into the core's ring as it decodes it (w_put[c], the beat on its w_beat), w_valid[c] says the
// ring holds one queued, and taking it moves the ring's head (its feed queues one beat a core for two cores or more, two
// for a lone core, so the ring's places are one more). Each core's sums
// emerge at the stripe's last block, a token-row a clock, all cores at once, with the stripe and the token-row beside
// them. No pass runs while `hold`.
module logos_column_array_{name} #(parameter CH = 4, N = 2, T_MAX = 4, NB_MAX = 8)
  (input clk,{clk2_in} input rst,
   input start, input [15:0] stripes, input [15:0] blocks, input [7:0] tokens{two_in}{fmt_in}, input [31:0] rows,
   input [2*CH-1:0] turns, input hold, output busy,
   input act_wr, input [7:0] act_t, input [15:0] act_b{arr_act_s}, input [{codes_top}:0] act_codes, input [7:0] act_m, input [11:0] act_e,
   input [CH*N-1:0] w_valid, output [CH*N-1:0] w_ready, input [CH*N-1:0] w_put, input [CH*N*{beat}-1:0] w_beat,
   output [CH*N-1:0] o_valid, output [CH*N*32-1:0] o_val1, output [CH*N*32-1:0] o_val2, output reg [15:0] o_stripe,
   output reg [7:0] o_t);
  localparam LAT = {lat}, ADD = {add}, TAGS = LAT + ADD, C = CH * N;
{two_decl}{fmt_copy}  localparam TW = (T_MAX > 1) ? $clog2(T_MAX) : 1;
  // The step held: its stripe, its block, the token-row it issues next; whether a step is held. Beside them, each the value its wire would have from them, so an issue is a few levels
  // from registers: whether the block is its stripe's last (step_end), the stripe the last (last_stripe), the token-row
  // the step's last (t_last), and whether every core the step holds a pair for has its beat (ready).
  reg held, step_end, last_stripe, t_last, ready;
  reg [15:0] hs, hb;
  reg [7:0] t;
  reg [C-1:0] have;
  wire next = !(step_end && last_stripe);
  localparam [31:0] STRIPE_ROWS = CH * 2 * N;
  // The matrix's rows from the next stripe's first on (left_after), and whether each core's pair's first row is among
  // the rows from the held stripe's first on (need_cur) and from the next one's (need_adv), registers updated a stripe
  // at a time; so the cores the held step and the next one hold a pair for are a select of registers. The first
  // step's from the rows.
  reg [31:0] left_after;
  function [31:0] after_stripe(input [31:0] l);
    after_stripe = (l > STRIPE_ROWS) ? l - STRIPE_ROWS : 32'd0;
  endfunction
  reg [C-1:0] need, need_next, need_start, need_cur, need_adv, adv_start, adv_next;
  reg [31:0] first;
  integer c;
  always @* begin
    for (c = 0; c < C; c = c + 1) begin
      first = turns[2*(c/N) +: 2] * 2 * N + 2 * (c % N);
      need[c] = held && need_cur[c];
      need_next[c] = held && next && (step_end ? need_adv[c] : need_cur[c]);
      need_start[c] = first < rows;
      adv_start[c] = first < after_stripe(rows);
      adv_next[c] = first < after_stripe(left_after);
    end
  end
  // Each issue in flight: whether it is one, its token-row, its stripe, and whether it is its sums' last block.
  reg live [0:TAGS-1];
  reg [TW-1:0] tag [0:TAGS-1];
  reg [15:0] ts [0:TAGS-1];
  reg [7:0] tt [0:TAGS-1];
  reg last [0:TAGS-1];
  wire [TW-1:0] at = t[TW-1:0];
  reg stall, flight;
  integer h;
  always @* begin
    stall = 1'b0;
    flight = 1'b0;
    for (h = 0; h < ADD; h = h + 1) if (live[h] && tag[h] == at) stall = 1'b1;
    for (h = 0; h < TAGS; h = h + 1) if (live[h]) flight = 1'b1;
  end
{arr_octets}  // An issue's passes, a clock each: its format's code digits, for each activation word; the last waits on a stall.
{digits}  wire [2:0] passes = two ? {{digits, 1'b0}} : {{1'b0, digits}};
  reg [2:0] pk;
  // The pass counter one on (pk_up), a register beside it from its next value, so the last pass is a compare of registers.
  reg [2:0] pk_up;
  wire last_pass = pk_up == passes;
  wire issue = held && ready && !hold && last_pass && {pe_ok};
  wire pass = held && ready && !hold && (!last_pass || {pe_ok});
  wire digit = digits[1] && pk[0];
  wire word = digits[1] ? pk[1] : pk[0];
  wire fin = issue && t_last;
  wire [2:0] pk_n = rst || (start && !busy) ? 3'd0 : pass ? (last_pass ? 3'd0 : pk_up) : pk;
  always @(posedge clk) pk_up <= pk_n + 3'd1;
  assign w_ready = (need & ~have) | ({{C{{fin}}}} & need_next);
  wire [C-1:0] take = w_valid & w_ready;
  assign busy = held || flight{arr_busy};
  // Each token-row's blocks of activations, as their words' excess codes and unit sums, in block RAM beside each
  // channel: every copy written alike a clock after the word is made, each read a clock for its own channel's cores.
  // Each token-row's blocks at a pitch of a power of two (NB_P, 2^NB_L), so an address is a token-row's place and a
  // block's in bits of their own.
  localparam NB_L = (NB_MAX > 1) ? $clog2(NB_MAX) : 0, NB_P = 1 << NB_L;
  localparam AN = T_MAX * NB_P, AA = (AN > 1) ? $clog2(AN) : 1;
  wire [{act_top}:0] act_wd;
  logos_column_act_word #(.ELEMS({elems}), .WORDS({words_n}), .SLOTS({slots})) word_of(.codes(act_codes), .two(two), .m(act_m), .e(act_e),
    .slot({act_slot}),
    .word(act_wd));
  wire [31:0] act_wa = ({{24'd0, act_t}} << NB_L) | (act_b & (NB_P - 1));
  // The step's next token-row and block, as the clock's update leaves them, and the read address they make: each copy
  // takes it into a register of its own beside its blocks, so a read sees the address of the step's token-row and block
  // as before, from a register.
  // The step's token-row and block one on (t_up, hb_up) are registers beside them, the next values plus their steps,
  // so each next value is a choice of registers and the read address holds no carry chain.
  reg [7:0] t_up;
  reg [15:0] hb_up;
  wire [7:0] t_n = rst || (start && !busy) ? 8'd0 : issue ? (t_last ? 8'd0 : t_up) : t;
  wire [15:0] hb_n = !rst && start && !busy ? 16'd0 : fin && next ? (step_end ? 16'd0 : hb_up) : hb;
  always @(posedge clk) begin t_up <= t_n + 8'd1; hb_up <= hb_n + 16'd1; end
  wire [31:0] ra_n = ({{24'd0, t_n}} << NB_L) | ({{16'd0, hb_n}} & (NB_P - 1));
  wire [CH*{act_w}-1:0] act_words;
{ach_decls}  genvar ch;
  generate
    for (ch = 0; ch < CH; ch = ch + 1) begin : ach
      reg we;
      reg [AA-1:0] wa;
      reg [{act_top}:0] wd;
      reg [{wm_top}:0] wm;
      (* maxfan = 64 *) reg [AA-1:0] ra;
      always @(posedge clk) begin we <= act_wr; wa <= act_wa[AA-1:0]; wd <= act_wd; wm <= {act_wm}; ra <= ra_n[AA-1:0]; end
      wire [{act_top}:0] act_word;
      {act_ram} #(.D(AN), .A(AA)) act_ram(.clk(clk), .ra(ra), .rd(act_word), .we(we), .wa(wa), .wd(wd), .wm(wm));
      assign act_words[ch*{act_w} +: {act_w}] = act_word;
{ach_body}    end
  endgenerate
  genvar g;
  generate
    for (g = 0; g < C; g = g + 1) begin : core
      logos_column_core_{core_mod} #(.SUMS(T_MAX), .SW(TW), .RING((N == 1) ? 3 : 2)) u(.clk(clk), .rst(rst), .clear(start && !busy),
        {two_pass}{fmt_pass}.put(w_put[g]), .load(take[g]), .w_beat(w_beat[g*{beat} +: {beat}]), .pass(pass), .digit(digit), .word(word), .first(pk == 3'd0),
        .issue({core_issue}){core_octet_conn}, .act_word(act_words[(g/N)*{act_w} +: {act_w}]), {core_pe}.land(tag[LAT-1]), .done(tag[TAGS-1]), .last(last[TAGS-1]),
        .o_valid(o_valid[g]), .o_val1(o_val1[g*32 +: 32]), .o_val2(o_val2[g*32 +: 32]));
    end
{pairs}  endgenerate
  integer k;
  always @(posedge clk) begin
{tags_in}
    for (k = 1; k < TAGS; k = k + 1) begin
      live[k] <= live[k-1]; tag[k] <= tag[k-1]; ts[k] <= ts[k-1]; tt[k] <= tt[k-1]; last[k] <= last[k-1];
    end
    if (live[TAGS-1] && last[TAGS-1]) begin o_stripe <= ts[TAGS-1]; o_t <= tt[TAGS-1]; end
    if (pass) pk <= last_pass ? 3'd0 : pk + 3'd1;
    if (issue) begin t <= t_last ? 8'd0 : t + 8'd1; t_last <= t_last ? tokens == 8'd1 : t + 8'd2 == tokens; end
    have <= (have & ~{{C{{fin}}}}) | take;
    // Next clock's ready from this one's registers: past a step's last issue every core the next step needs is offered
    // its beat, otherwise every core this step needs holds its beat or is offered it.
    ready <= fin ? &(~need_next | w_valid) : &(have | w_valid | ~need);
    if (fin) begin
      if (!next) held <= 1'b0;
      else if (step_end) begin
        hb <= 16'd0; hs <= hs + 16'd1; left_after <= after_stripe(left_after); need_cur <= need_adv;
        need_adv <= adv_next; step_end <= blocks == 16'd1; last_stripe <= hs + 16'd2 == stripes;
      end else begin
        hb <= hb + 16'd1; step_end <= hb + 16'd2 == blocks;
      end
    end
{arr_octet_step}    if (rst) begin
      held <= 1'b0; have <= {{C{{1'b0}}}}; t <= 8'd0; pk <= 3'd0;
      for (k = 0; k < TAGS; k = k + 1) live[k] <= 1'b0;{arr_octet_reset}
    end else if (start && !busy) begin
      held <= 1'b1; hs <= 16'd0; hb <= 16'd0; left_after <= after_stripe(rows); need_cur <= need_start;
      need_adv <= adv_start; t <= 8'd0; pk <= 3'd0; have <= {{C{{1'b0}}}};
      step_end <= blocks == 16'd1; last_stripe <= stripes == 16'd1; t_last <= tokens == 8'd1; ready <= &(~need_start);{arr_octet_reset}
    end
  end
endmodule

// One channel's feed of a {what} matrix laid in stripes to its N cores: the image stream's beats of the channel's tiles
// (a beat at offset 0 starting a tile, whose first row it carries; zeros past its kept bytes) gathered into blocks, and
// each block step's blocks dealt in pairs, decoded, the tile's rows 2p and 2p + 1 to core p, the second block zeros when
// the tile's last row has no partner. A tile holds 2N rows, fewer at the tensor's `rows`. Each block comes in its slot
// (its bytes, then its pad's zeros), so a beat lands in the gather register a whole number of slots in. The gather
// register keeps less than a block past the last block it holds, so it never holds more than two tiles' bytes, and its
// bytes past those it holds are zeros, a beat's bytes ORed in at its end (cleared with the reset, so whatever it powers
// up holding never reaches a block); the beats queued for each core, in its ring behind the beat it holds
// (logos_column_core_{name}), let the stream run a step ahead of the cores: two for a lone core, one a core for two or
// more, whose beats come no sooner than 2N blocks apart. A pair's beat is decoded into a register and put into its
// core's ring the clock after (w_put, the register on every core's w_beat), a core's room counting the beat on its way;
// whether the row a block goes to is its tile's last is held in a register beside the row.
module logos_column_feed_{name} #(parameter N = 2)
  (input clk, input rst{fmt_in}, input [15:0] blocks, input [31:0] rows,
   input in_valid, output in_ready, input [511:0] in_data, input [6:0] in_keep, input [31:0] in_row, input [23:0] in_offset,
   output [N-1:0] w_valid, input [N-1:0] w_ready, output [N-1:0] w_put, output [N*{beat}-1:0] w_beat);
{fmt_copy}{feed_bb}  reg [8*GB-1:0] gather;
  // Each core's put, a register copied a copy each 64 of the ring's write enables it reaches.
  (* maxfan = 64 *) reg [N-1:0] put_q;
  assign w_put = put_q;
  reg [15:0] fill;
  // The rows of the tile whose blocks leave next, and of the tile after it once its first beat is in.
  reg [7:0] held, held_next;
  reg has_next;
  (* maxfan = 256 *) reg [7:0] j;
  reg jl;
  reg [15:0] b;
  reg [{top}:0] stage;
  // The beats queued in each core's ring, QD at most, and the beat decoded on its way there.
  localparam QD = (N == 1) ? 2 : 1;
  reg [1:0] count [0:N-1];
  reg [{beat_top}:0] dq;
  reg dq_v;
  reg [7:0] dq_p;
  wire [7:0] p = j >> 1;
  wire odd = j[0];
  wire alone = !odd && jl;
  // A block leaves the gather register when its tile is known and, completing a beat, its core's queue has room for it
  // beside any beat already on its way there.
  wire pair = odd || alone;
  wire on_way = dq_v && dq_p == p;
  wire queue_room = (QD == 2) ? (count[p] == 2'd0 || (count[p] == 2'd1 && !on_way)) : (count[p] == 2'd0 && !on_way);
  wire emit = fill >= BB && held != 8'd0 && (!pair || queue_room);
  wire push = emit && pair;
  wire [15:0] kept = emit ? fill - BB : fill;
  assign in_ready = kept <= BB && !(in_offset == 24'd0 && has_next);
  wire take = in_valid && in_ready;
  localparam [31:0] TILE = 2 * N;
  wire [31:0] room = rows - in_row;
  wire [7:0] tile_rows = (room < TILE) ? room[7:0] : TILE[7:0];
  wire tile_end = emit && jl && (b + 16'd1 == blocks);
  wire [8*GB-1:0] shifted = emit ? {drop} : gather;
  // The next row and the next tile's rows, and so whether the next row is its tile's last.
  wire [7:0] j_next = emit ? (jl ? 8'd0 : j + 8'd1) : j;
  wire tile_in = take && in_offset == 24'd0;
  wire [7:0] held_now = tile_in ? ((held == 8'd0 || tile_end) ? tile_rows : held) : tile_end ? (has_next ? held_next : 8'd0) : held;
  // The beat a push queues, decoded.
  wire [{beat_top}:0] decoded;
  logos_column_decode_{name} dec(.blk1(odd ? stage : gather[{top}:0]), .blk2(odd ? gather[{top}:0] : {{{bits}{{1'b0}}}}),
    {fmt_dec}.beat(decoded));
  genvar g;
  generate
    for (g = 0; g < N; g = g + 1) begin : core
      assign w_valid[g] = count[g] != 2'd0;
      assign w_beat[g*{beat} +: {beat}] = dq;
    end
  endgenerate
  integer k;
  always @(posedge clk) begin
    if (take) gather <= shifted | ({{{{(8*GB-512){{1'b0}}}}, in_data}} << ({gather_shift}));
    else gather <= shifted;
    fill <= kept + (take ? in_keep : 7'd0);
    if (emit) begin
      if (!odd && !alone) stage <= gather[{top}:0];
      if (jl) b <= (b + 16'd1 == blocks) ? 16'd0 : b + 16'd1;
    end
    j <= j_next;
    held <= held_now;
    jl <= j_next + 8'd1 == held_now;
    dq_v <= push;
    if (push) begin dq <= decoded; dq_p <= p; end
    for (k = 0; k < N; k = k + 1) begin
      put_q[k] <= push && p == k[7:0];
      if (w_valid[k] && w_ready[k]) begin
        if (!(dq_v && dq_p == k[7:0])) count[k] <= count[k] - 2'd1;
      end else if (dq_v && dq_p == k[7:0]) count[k] <= count[k] + 2'd1;
    end
    if (tile_in) begin
      if (!(held == 8'd0 || tile_end)) begin held_next <= tile_rows; has_next <= 1'b1; end
    end else if (tile_end) has_next <= 1'b0;
    if (rst) begin
      gather <= {{(8*GB){{1'b0}}}}; fill <= 16'd0; held <= 8'd0; has_next <= 1'b0; j <= 8'd0; jl <= 1'b0; b <= 16'd0;
      dq_v <= 1'b0;
      for (k = 0; k < N; k = k + 1) begin count[k] <= 2'd0; put_q[k] <= 1'b0; end
    end
  end
endmodule"#,
            top = bits - 1,
            beat = 2 * db,
            beat_top = 2 * db - 1,
            act_top = set.act_bits() - 1,
            act_bytes = set.act_bits() / 8,
            codes_top = 16 * elems - 1,
            unit_top = 8 * units - 1,
            fc = f.c,
            fn = f.n,
            fa = f.a,
            fb = f.b,
            fq = f.q,
            decode = decode(&decs, elems),
            zsel = zsel(&decs),
            digits = digits(&decs),
            words_n = set.word_count(),
            feed_bb = feed_block_bytes(&decs),
            gather_shift = if set.slot > 1 { format!("{} * kept[15:{}]", 8 * set.slot, set.slot.trailing_zeros()) } else { "8 * kept".to_string() },
            drop = feed_drop(&decs),
            lat = COLUMN_VALUE_LATENCY,
            add = FADD_LATENCY,
            pl = PASS_LATENCY,
            lb_bits = lane.beat_bits,
            lb_in = lane.beat_packed,
            la_bits = lane.act_bits,
            la_in = lane.act_packed,
            lf_out = lane.unpacked,
            act_ram = super::ram::bram_name(set.act_bits()),
            stagger = if paired {
                format!(
                    "  // The pass's word for the pairs' processing elements, its excess codes chosen at the read and its units' sums,\n  \
                     // once for every core; each core's codes out to its pair and its partials back.\n  reg word_a;\n  \
                     always @(posedge clk) word_a <= word;\n  wire [{code_top}:0] act_un = {st_sel};\n  \
                     wire [{bus12}:0] s_un = {};\n  localparam CODE = {}, PART = {};\n  wire [C*CODE-1:0] pc1, pc2;\n  \
                     wire [C*PART-1:0] pp1, pp2;\n",
                    s_sel.replace("s1_word", "word_a"),
                    code_top + 1,
                    bus24 + 1
                )
            } else {
                format!(
                    "  // The pass's excess codes for the systolic elements, the word chosen at the read and each element's block\n  \
                     // delayed for it, once for every core.\n  reg word_a;\n  always @(posedge clk) word_a <= word;\n  \
                     wire [{code_top}:0] act_st;\n  logos_column_act_stagger #(.ELEMS({elems})) st(.clk(clk), .codes({st_sel}), .st(act_st));\n"
                )
            },
            clk2_in = if set.pe_pairs { " input clk2," } else { "" },
            core_mod = if paired { format!("bare_{name}") } else { name.to_string() },
            core_pe = if paired {
                ".pe_code1(pc1[g*CODE +: CODE]), .pe_code2(pc2[g*CODE +: CODE]), .pe_p1(pp1[g*PART +: PART]), .pe_p2(pp2[g*PART +: PART]),\n        ".to_string()
            } else {
                format!(".act_st(act_sts[(g/N)*{w} +: {w}]), ", w = code_top + 1)
            },
            act_w = set.act_bits(),
            wm_top = set.act_bits() / 8 - 1,
            ach_decls = if paired {
                format!(
                    "  // Each channel's word for its pairs' processing elements, its excess codes chosen at the read and its units'\n  \
                     // sums; each core's codes out to its pair and its partials back.\n  \
                     wire [CH*{cw}-1:0] act_uns;\n  wire [CH*{sw}-1:0] s_uns;\n  localparam CODE = {cw}, PART = {pw};\n  \
                     wire [C*CODE-1:0] pc1, pc2;\n  wire [C*PART-1:0] pp1, pp2;\n",
                    cw = code_top + 1,
                    sw = bus12 + 1,
                    pw = bus24 + 1
                )
            } else {
                format!("  // Each channel's excess codes for its cores' systolic elements, each element's block delayed for it.\n  wire [CH*{}-1:0] act_sts;\n", code_top + 1)
            },
            ach_body = if paired {
                format!(
                    "      reg word_a;\n      always @(posedge clk) word_a <= word;\n      \
                     assign act_uns[ch*{cw} +: {cw}] = {st_sel};\n      assign s_uns[ch*{sw} +: {sw}] = {};\n",
                    s_sel.replace("s1_word", "word_a"),
                    cw = code_top + 1,
                    sw = bus12 + 1
                )
            } else {
                format!(
                    "      reg word_a;\n      always @(posedge clk) word_a <= word;\n      wire [{code_top}:0] act_st;\n      \
                     logos_column_act_stagger #(.ELEMS({elems})) st(.clk(clk), .codes({st_sel}), .st(act_st));\n      \
                     assign act_sts[ch*{cw} +: {cw}] = act_st;\n",
                    cw = code_top + 1
                )
            },
            pairs = if paired {
                format!(
                    "    // Each pair of cores on one set of processing elements in clk2, core 2g's codes at its first turn.\n    \
                     for (g = 0; g < C / 2; g = g + 1) begin : pair\n      \
                     logos_matvec_pass_x2 #(.UNITS({units})) pe(.clk(clk), .clk2(clk2), .code1a(pc1[2*g*CODE +: CODE]),\n        \
                     .code2a(pc2[2*g*CODE +: CODE]), .code1b(pc1[(2*g+1)*CODE +: CODE]), .code2b(pc2[(2*g+1)*CODE +: CODE]),\n        \
                     .w(act_uns[((2*g)/N)*CODE +: CODE]), .s(s_uns[((2*g)/N)*{sw} +: {sw}]), .digit(digit), .p1a(pp1[2*g*PART +: PART]), .p2a(pp2[2*g*PART +: PART]), .p1b(pp1[(2*g+1)*PART +: PART]),\n        \
                     .p2b(pp2[(2*g+1)*PART +: PART]));\n    end\n",
                    sw = bus12 + 1
                )
            } else {
                String::new()
            },
            beat_s7 = if set.octets { format!("  assign beat[{s} +: 112] = s71_d;\n  assign beat[{} +: 112] = s72_d;\n", db + f.s7, s = f.s7) } else { String::new() },
            core_octet_ports = oct.ports,
            core_octets = oct.body,
            octet_lanes = oct.lanes,
            octet_lanes_end = oct.lanes_end,
            lanes_from = oct.from,
            slots = set.act_slots(),
            act_slot = if set.octets { "act_s" } else { "3'd0" },
            arr_act_s = arr["arr_act_s"],
            arr_octets = arr["arr_octets"],
            pe_ok = arr["pe_ok"],
            arr_busy = arr["arr_busy"],
            act_wm = arr["act_wm"],
            core_issue = arr["core_issue"],
            core_octet_conn = arr["core_octet_conn"],
            tags_in = arr["tags_in"],
            arr_octet_step = arr["arr_octet_step"],
            arr_octet_reset = arr["arr_octet_reset"],
        )
        .unwrap();
        if set.octets || set.token_pairs {
            // An octet job, and a token pair's issue, runs on the array alone: the set builds no lone column.
            let at = v.rfind(&format!("module logos_column_{name} ")).expect("the set's lone column");
            let start = v[..at].rfind("\n\n").map_or(at, |i| i + 1);
            let end = at + v[at..].find("endmodule").expect("its end") + "endmodule".len();
            v.replace_range(start..end, "");
        }
        if set.lanes > 1 {
            v = octet_lanes(&v, set);
        }
        if paired {
            v = bare_core(&v, set, units, code_top, bus24);
        }
        if set.token_pairs {
            v = token_pairs(&v, set);
        }
    }
    if sets.iter().any(|s| s.octets) {
        v.push_str(OCTET_DECODER);
    }
    if sets.iter().any(|s| s.pe_pairs && !s.token_pairs) {
        v.push_str(&matvec_pass_x2_verilog());
    }
    if sets.iter().any(|s| s.pe_pairs && s.token_pairs) {
        v.push_str(&matvec_pass_x2t_verilog());
    }
    v
}

/// `v` with set `set`'s core beside it as `logos_column_core_bare_<set>`: the core, its pass's codes out to processing
/// elements it shares with another core (`pe_code1`, `pe_code2`) and their partials in (`pe_p1`, `pe_p2`) where its own
/// elements were, for [`ColumnSet::pe_pairs`].
fn bare_core(v: &str, set: &ColumnSet, units: usize, code_top: usize, bus24: usize) -> String {
    let name = &set.name;
    let head = format!("module logos_column_core_{name} ");
    let at = v.find(&head).unwrap_or_else(|| panic!("`{head}`"));
    let end = at + v[at..].find("endmodule").expect("its end") + "endmodule".len();
    let bare = replaced(
        &v[at..end],
        &[
            (&head, &format!("module logos_column_core_bare_{name} ")),
            (
                &format!("input [{code_top}:0] act_st,"),
                &format!("output [{code_top}:0] pe_code1, output [{code_top}:0] pe_code2,\n   input [{bus24}:0] pe_p1, input [{bus24}:0] pe_p2,"),
            ),
            (
                &format!(
                    "  logos_matvec_pass_p #(.UNITS({units})) parts(.clk(clk), .code1(code1), .code2(code2), .w(act_st), .s(s_act), .digit(digit),\n    .p1(w_p1), .p2(w_p2));\n"
                ),
                "  assign pe_code1 = code1;\n  assign pe_code2 = code2;\n  assign w_p1 = pe_p1;\n  assign w_p2 = pe_p2;\n",
            ),
        ],
    );
    let mut out = v.to_string();
    out.insert_str(
        end,
        &format!(
            "\n\n// logos_column_core_{name} with its processing elements outside it, shared with another core: its pass's codes out\n\
             // (`pe_code1`, `pe_code2`) and their partials in (`pe_p1`, `pe_p2`), as its own elements would give them.\n{bare}"
        ),
    );
    out
}

/// The fields of an issue's beat and activations its lanes take — both rows' unit sums of codes, multipliers, offset
/// terms and scales, the activation's scale, the pass's word's unit sums, and for a set with octets the other seven
/// blocks' scales of both rows and the activation's — the beat's packed in that order into one of the core's
/// `logos_delay`s from the beat held and the activations' into the other, and unpacked from their words out (`lfb`,
/// `lfa`, [`PASS_LATENCY`] clocks after the pass's activations) under their names.
struct LaneFields {
    beat_bits: usize,
    beat_packed: String,
    act_bits: usize,
    act_packed: String,
    unpacked: String,
}

fn lane_fields(set: &ColumnSet) -> LaneFields {
    let (f, a, db, u) = (set.decoded_fields(), set.act_fields(), set.decoded_bits(), set.elems() / UNIT);
    let beat = |name: &str, at: usize, w: usize| (name.to_string(), format!("held[{at} +: {w}]"), w);
    let mut fields = vec![
        beat("f_q1", f.q, 12 * u),
        beat("f_q2", db + f.q, 12 * u),
        beat("f_c1", f.c, 8 * u),
        beat("f_n1", f.n, 8 * u),
        beat("f_c2", db + f.c, 8 * u),
        beat("f_n2", db + f.n, 8 * u),
        beat("f_a1", f.a, 16),
        beat("f_b1", f.b, 16),
        beat("f_a2", db + f.a, 16),
        beat("f_b2", db + f.b, 16),
        ("f_m".to_string(), format!("act_word[{} +: 8]", a.m), 8),
        ("f_e".to_string(), format!("act_word[{} +: 12]", a.e), 12),
        ("f_s".to_string(), "s_act".to_string(), 12 * u),
    ];
    if set.octets {
        fields.push(beat("f_s71", f.s7, 112));
        fields.push(beat("f_s72", db + f.s7, 112));
        for k in 1..8 {
            fields.push((format!("f_ms{k}"), format!("{{act_word[{} +: 12], act_word[{} +: 8]}}", a.e + 24 * k, a.m + 24 * k), 20));
        }
    }
    let (beats, acts): (Vec<_>, Vec<_>) = fields.iter().partition(|(_, e, _)| e.starts_with("held["));
    let mut unpacked = String::new();
    let mut pack = |part: &[&(String, String, usize)], out: &str| {
        let mut at = 0;
        for (name, _, w) in part {
            writeln!(unpacked, "  wire [{}:0] {name} = {out}[{at} +: {w}];", w - 1).unwrap();
            at += w;
        }
        let packed: Vec<&str> = part.iter().rev().map(|(_, e, _)| e.as_str()).collect();
        (at, packed.join(", "))
    };
    let (beat_bits, beat_packed) = pack(&beats, "lfb");
    let (act_bits, act_packed) = pack(&acts, "lfa");
    LaneFields { beat_bits, beat_packed, act_bits, act_packed, unpacked }
}

/// The core and the array of `set`, a set of [`ColumnSet::lanes`] octet lanes a row, in `v`: the core's lanes past the
/// first (`logos_lane_slim_p`, a row's each) taking a token-row's next blocks of its group, lane j's value added into
/// the row's sum j adds after the first add, its value delayed j adds to meet it; an octet job's sums written back, and
/// the array's last-block taps read, `lanes` adds after the value lands, a K-quant's one add after as before.
fn octet_lanes(v: &str, set: &ColumnSet) -> String {
    let (name, l) = (&set.name, set.lanes);
    let words = set.word_count();
    let core_head = format!("module logos_column_core_{name} ");
    let array_head = format!("module logos_column_array_{name} ");
    let span = |v: &str, head: &str| {
        let at = v.find(head).unwrap_or_else(|| panic!("`{head}`"));
        (at, at + v[at..].find("endmodule").expect("its end"))
    };
    let mut out = v.to_string();
    let (a, b) = span(&out, &core_head);
    let mut chain = String::from(
        "  // The octet lanes past the first: lane j's value of a token-row's block o_k + j added into the row's sum j adds after\n  \
         // the first add, the value delayed j adds to meet it; an octet job's sum is the chain's last.\n",
    );
    for r in 1..=2 {
        writeln!(chain, "  wire [31:0] ch{r}_0 = sum{r};").unwrap();
        for j in 1..l {
            let d = j * FADD_LATENCY;
            writeln!(
                chain,
                "  wire [31:0] sv{r}_{j};\n  logos_lane_slim_p #(.UNITS(2), .WORDS({words})) slim{r}_{j}(.clk(clk), .p_hi(sl{j}_p{r}h), .s_hi(sl{j}_sh), .p_lo(sl{j}_p{r}l),\n    \
                 .s_lo(sl{j}_sl), .q(sl{j}_q{r}), .z(z), .a(sl{j}_a{r}), .m(sl{j}_me[7:0]), .e(sl{j}_me[19:8]), .two(two), .value(sv{r}_{j}));\n  \
                 reg [31:0] dv{r}_{j} [0:{dt}];\n  integer dk{r}_{j};\n  always @(posedge clk) begin\n    dv{r}_{j}[0] <= sv{r}_{j};\n    \
                 for (dk{r}_{j} = 1; dk{r}_{j} < {d}; dk{r}_{j} = dk{r}_{j} + 1) dv{r}_{j}[dk{r}_{j}] <= dv{r}_{j}[dk{r}_{j} - 1];\n  end\n  \
                 wire [31:0] ch{r}_{j};\n  logos_fadd_p chain{r}_{j}(.clk(clk), .a(ch{r}_{p}), .b(dv{r}_{j}[{dt}]), .y(ch{r}_{j}));",
                dt = d - 1,
                p = j - 1,
            )
            .unwrap();
        }
        writeln!(chain, "  wire [31:0] chain{r} = ch{r}_{};", l - 1).unwrap();
    }
    let core = replaced(
        &out[a..b],
        &[
            (
                &format!("  localparam LAT = {}, ADD = {}, TAGS = LAT + ADD;\n", COLUMN_VALUE_LATENCY, FADD_LATENCY),
                &format!("  localparam LAT = {}, ADD = {}, TAGS = LAT + {l} * ADD, KT = LAT + ADD;\n", COLUMN_VALUE_LATENCY, FADD_LATENCY),
            ),
            ("  // This core's issues in flight.\n", &format!("{chain}  // This core's issues in flight.\n")),
            (
                "    if (alive[TAGS-1]) begin\n      acc1[done] <= last ? 32'd0 : sum1;\n      acc2[done] <= last ? 32'd0 : sum2;\n      \
                 if (last) begin o_valid <= 1'b1; o_val1 <= sum1; o_val2 <= sum2; end\n    end\n",
                "    if (oct ? alive[TAGS-1] : alive[KT-1]) begin\n      acc1[done] <= last ? 32'd0 : (oct ? chain1 : sum1);\n      \
                 acc2[done] <= last ? 32'd0 : (oct ? chain2 : sum2);\n      \
                 if (last) begin o_valid <= 1'b1; o_val1 <= oct ? chain1 : sum1; o_val2 <= oct ? chain2 : sum2; end\n    end\n",
            ),
        ],
    );
    out.replace_range(a..b, &core);
    let (a, b) = span(&out, &array_head);
    let array = replaced(
        &out[a..b],
        &[
            (
                &format!("  localparam LAT = {}, ADD = {}, TAGS = LAT + ADD, C = CH * N;\n", COLUMN_VALUE_LATENCY, FADD_LATENCY),
                &format!("  localparam LAT = {}, ADD = {}, TAGS = LAT + {l} * ADD, KT = LAT + ADD, C = CH * N;\n", COLUMN_VALUE_LATENCY, FADD_LATENCY),
            ),
            (".land(tag[LAT-1]), .done(tag[TAGS-1]), .last(last[TAGS-1]),", ".land(tag[LAT-1]), .done(oct ? tag[TAGS-1] : tag[KT-1]), .last(oct ? last[TAGS-1] : last[KT-1]),"),
            (
                "    if (live[TAGS-1] && last[TAGS-1]) begin o_stripe <= ts[TAGS-1]; o_t <= tt[TAGS-1]; end\n",
                "    if (oct ? live[TAGS-1] && last[TAGS-1] : live[KT-1] && last[KT-1]) begin\n      \
                 o_stripe <= oct ? ts[TAGS-1] : ts[KT-1]; o_t <= oct ? tt[TAGS-1] : tt[KT-1];\n    end\n",
            ),
        ],
    );
    out.replace_range(a..b, &array);
    out
}

/// `text` with every identifier of `names` (a whole word: no letter, digit or `_` either side) given `suffix`, but a
/// port's name (after a `.`).
fn suffixed(text: &str, names: &[&str], suffix: &str) -> String {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len() + 64);
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if names.contains(&word.as_str()) && !out.ends_with('.') {
            out.push_str(word);
            out.push_str(suffix);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if ident(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The one line of `text` holding `needle`, with its newline.
fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    let lines: Vec<&str> = text.split_inclusive('\n').filter(|l| l.contains(needle)).collect();
    assert_eq!(lines.len(), 1, "one line holding `{needle}`");
    lines[0]
}

/// The span of `text` from the start of the one line holding `needle` to the end of `to`'s first place after it.
fn from_line<'a>(text: &'a str, needle: &str, to: &str) -> &'a str {
    between(text, line_with(text, needle), to)
}

/// The span of `text` from `from`'s one place to the end of `to`'s first place after it.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    assert_eq!(text.matches(from).count(), 1, "`{from}`");
    let a = text.find(from).unwrap();
    let b = a + text[a..].find(to).unwrap_or_else(|| panic!("`{to}` after `{from}`")) + to.len();
    &text[a..b]
}

/// The core and the array of a set of token pairs ([`ColumnSet::token_pairs`]) in `v`: the core's second token-row
/// (`_b`) beside its first — its word's unit sums, processing elements of its own on the held beat's codes, its word's
/// lane fields, its sums over the passes, an octet job's slots, its lanes' inputs, its two lanes, its two rows' sums
/// and their values out, the beat's fields and the issue's flags shared; the array's issues a token pair each, a
/// sum's place a pair's, its copies of the activations a pair a word.
fn token_pairs(v: &str, set: &ColumnSet) -> String {
    let name = &set.name;
    let (u, elems) = (set.elems() / UNIT, set.elems());
    let (aw, cw) = (set.act_bits(), 8 * elems);
    let span = |v: &str, head: &str| {
        let at = v.find(head).unwrap_or_else(|| panic!("`{head}`"));
        (at, at + v[at..].find("endmodule").expect("its end"))
    };
    let mut out = v.to_string();

    let (a, b) = span(&out, &format!("module logos_column_core_{name} "));
    let core = out[a..b].to_string();
    // Token-row b's word: its unit sums as token-row a's are chosen, and processing elements of its own.
    let s_act_line = line_with(&core, "] s_act = ");
    let parts = from_line(&core, "] w_p1, w_p2;", ".p1(w_p1), .p2(w_p2));\n");
    let b_word = format!(
        "  // Token-row b's word of the pass: its unit sums, and the pass's partials of the same codes through elements of its own.\n{}{}",
        suffixed(s_act_line, &["s_act", "act_word"], "_b"),
        suffixed(parts, &["s_act", "act_st", "w_p1", "w_p2", "parts"], "_b"),
    );
    // With pe_pairs the two words' passes run on one set of elements in clk2, each word unstaggered (`act_st`, `act_st_b`).
    let x2t = format!(
        "  // Token-row b's unit sums, and both token-rows' partials of the beat's codes on one set of elements in clk2, token-row\n  \
         // a's word at its edge between clk's and b's at clk's next.\n{}  wire [{t24}:0] w_p1, w_p2, w_p1_b, w_p2_b;\n  \
         logos_matvec_pass_x2t #(.UNITS({u})) parts(.clk(clk), .clk2(clk2), .code1(code1), .code2(code2), .wa(act_st), .wb(act_st_b),\n    \
         .sa(s_act), .sb(s_act_b), .digit(digit), .p1a(w_p1), .p2a(w_p2), .p1b(w_p1_b), .p2b(w_p2_b));\n",
        suffixed(s_act_line, &["s_act", "act_word"], "_b"),
        t24 = 24 * u - 1,
    );
    let la_in = between(&core, "  wire [LA-1:0] la_in = ", ";\n");
    let lf_b: String = core.lines().filter(|l| l.contains("= lfa[")).map(|l| format!("{}\n", suffixed(l, &["lfa", "f_m", "f_e", "f_s", "f_ms1", "f_ms2", "f_ms3", "f_ms4", "f_ms5", "f_ms6", "f_ms7"], "_b"))).collect();
    let last_lfa = core.lines().filter(|l| l.contains("= lfa[")).last().expect("the word's lane fields").to_string();
    let sums_names = ["a1_hi", "a2_hi", "a1_lo", "a2_lo", "t1_hi", "t2_hi", "t1_lo", "t2_lo", "as_hi", "as_lo", "ts_hi", "ts_lo", "sh1", "sh2", "b1_hi", "b2_hi", "b1_lo", "b2_lo", "u", "w_p1", "w_p2", "f_s"];
    let sums = between(&core, "  // The pass's partials, its digit's in their place", "as_hi <= ts_hi; as_lo <= ts_lo;\n  end\n");
    let sums_b = suffixed(&sums.replacen("  // The pass's partials, its digit's in their place, added to its word's sums so far (none at an issue's first pass).\n", "  // Token-row b's partials and sums, as token-row a's.\n", 1), &sums_names, "_b");
    let lanes = between(&core, "  wire [31:0] value1, value2;\n", ".value(value2));\n");
    let lane_names = ["p1_hi", "p2_hi", "p1_lo", "p2_lo", "s_hi", "s_lo", "lm", "le", "value1", "value2", "l1", "l2"];
    let accs = between(&core, "  (* ramstyle = \"logic\" *) reg [31:0] acc1 [0:SUMS-1];\n", "logos_fadd_p add2(.clk(clk), .a(acc2[land]), .b(value2), .y(sum2));\n");
    let acc_names = ["acc1", "acc2", "sum1", "sum2", "add1", "add2", "value1", "value2"];
    let wide = set.words == Words::Two;
    let zero = |w: usize| if w > 0 { format!("{{{w}{{1'b0}}}}, ") } else { String::new() };
    let (pad24, pad12) = (zero(24 * u - 48), zero(12 * u - 24));
    let mut lane_in = format!(
        "  // Token-row b's lanes' inputs: its sums over the issue's passes{}; the beat's fields are the issue's.\n  \
         reg [{t24}:0] p1_hi_b, p2_hi_b, p1_lo_b, p2_lo_b;\n  reg [{t12}:0] s_hi_b, s_lo_b;\n  reg [7:0] lm_b;\n  reg signed [11:0] le_b;\n  \
         always @(posedge clk) begin\n",
        if set.octets { ", or for an octet job from its slot" } else { "" },
        t24 = 24 * u - 1,
        t12 = 12 * u - 1,
    );
    if set.octets {
        lane_in.push_str(&format!("    if (oct) begin\n      p1_hi_b <= {{{pad24}ob_p1h_b}};\n      p2_hi_b <= {{{pad24}ob_p2h_b}};\n"));
        if wide {
            lane_in.push_str(&format!("      p1_lo_b <= {{{pad24}ob_p1l_b}};\n      p2_lo_b <= {{{pad24}ob_p2l_b}};\n      s_lo_b <= {{{pad12}ob_sl_b}};\n"));
        } else {
            lane_in.push_str("      p1_lo_b <= 0; p2_lo_b <= 0; s_lo_b <= 0;\n");
        }
        lane_in.push_str(&format!("      s_hi_b <= {{{pad12}ob_sh_b}};\n      lm_b <= ob_me_b[7:0];\n      le_b <= ob_me_b[19:8];\n"));
        for j in 1..set.lanes {
            lane_in.push_str(&format!(
                "      sl{j}_p1h_b <= sb{j}_p1h_b; sl{j}_p2h_b <= sb{j}_p2h_b; sl{j}_p1l_b <= sb{j}_p1l_b; sl{j}_p2l_b <= sb{j}_p2l_b;\n      \
                 sl{j}_sh_b <= sb{j}_sh_b; sl{j}_sl_b <= sb{j}_sl_b; sl{j}_me_b <= sb{j}_me_b;\n"
            ));
        }
        lane_in.push_str("    end else begin\n");
    }
    lane_in.push_str(
        "    p1_hi_b <= t1_hi_b; p2_hi_b <= t2_hi_b; p1_lo_b <= t1_lo_b; p2_lo_b <= t2_lo_b;\n    s_hi_b <= ts_hi_b; s_lo_b <= ts_lo_b;\n    lm_b <= f_m_b;\n    le_b <= f_e_b;\n",
    );
    if set.octets {
        lane_in.push_str("    end\n");
    }
    lane_in.push_str("  end\n");
    let mut edits: Vec<(String, String)> = vec![
        (
            format!("input [{}:0] act_word, input [{}:0] act_st,", aw - 1, cw - 1),
            format!("input [{a}:0] act_word, input [{c}:0] act_st,\n   input [{a}:0] act_word_b, input [{c}:0] act_st_b,", a = aw - 1, c = cw - 1),
        ),
        (
            "output reg o_valid, output reg [31:0] o_val1, output reg [31:0] o_val2);".to_string(),
            "output reg o_valid, output reg [31:0] o_val1, output reg [31:0] o_val2,\n   output reg [31:0] o_val1_b, output reg [31:0] o_val2_b);".to_string(),
        ),
        if set.pe_pairs { (parts.to_string(), x2t) } else { (parts.to_string(), format!("{parts}{b_word}")) },
        (la_in.to_string(), format!("{la_in}{}", suffixed(la_in, &["la_in", "act_word", "s_act"], "_b"))),
        ("  wire [LA-1:0] lfa;\n".to_string(), "  wire [LA-1:0] lfa, lfa_b;\n".to_string()),
        (
            "  logos_delay #(.W(LA), .D(PL)) lad(.clk(clk), .d(la_in), .q(lfa));\n".to_string(),
            "  logos_delay #(.W(LA), .D(PL)) lad(.clk(clk), .d(la_in), .q(lfa));\n  logos_delay #(.W(LA), .D(PL)) lad_b(.clk(clk), .d(la_in_b), .q(lfa_b));\n".to_string(),
        ),
        (format!("{last_lfa}\n"), format!("{last_lfa}\n{lf_b}")),
        (sums.to_string(), format!("{sums}{sums_b}")),
        ("  wire [31:0] value1, value2;\n".to_string(), format!("{lane_in}  wire [31:0] value1, value2;\n")),
        (lanes.to_string(), format!("{lanes}{}", suffixed(lanes, &lane_names, "_b"))),
        (accs.to_string(), format!("{accs}{}", suffixed(accs, &acc_names, "_b"))),
        if set.lanes > 1 {
            (
                "      acc2[done] <= last ? 32'd0 : (oct ? chain2 : sum2);\n      \
                 if (last) begin o_valid <= 1'b1; o_val1 <= oct ? chain1 : sum1; o_val2 <= oct ? chain2 : sum2; end\n"
                    .to_string(),
                "      acc2[done] <= last ? 32'd0 : (oct ? chain2 : sum2);\n      acc1_b[done] <= last ? 32'd0 : (oct ? chain1_b : sum1_b);\n      \
                 acc2_b[done] <= last ? 32'd0 : (oct ? chain2_b : sum2_b);\n      \
                 if (last) begin\n        o_valid <= 1'b1; o_val1 <= oct ? chain1 : sum1; o_val2 <= oct ? chain2 : sum2;\n        \
                 o_val1_b <= oct ? chain1_b : sum1_b; o_val2_b <= oct ? chain2_b : sum2_b;\n      end\n"
                    .to_string(),
            )
        } else {
            (
                "      acc2[done] <= last ? 32'd0 : sum2;\n      if (last) begin o_valid <= 1'b1; o_val1 <= sum1; o_val2 <= sum2; end\n".to_string(),
                "      acc2[done] <= last ? 32'd0 : sum2;\n      acc1_b[done] <= last ? 32'd0 : sum1_b;\n      acc2_b[done] <= last ? 32'd0 : sum2_b;\n      \
                 if (last) begin o_valid <= 1'b1; o_val1 <= sum1; o_val2 <= sum2; o_val1_b <= sum1_b; o_val2_b <= sum2_b; end\n"
                    .to_string(),
            )
        },
        (
            "for (k = 0; k < SUMS; k = k + 1) begin acc1[k] <= 32'd0; acc2[k] <= 32'd0; end".to_string(),
            "for (k = 0; k < SUMS; k = k + 1) begin acc1[k] <= 32'd0; acc2[k] <= 32'd0; acc1_b[k] <= 32'd0; acc2_b[k] <= 32'd0; end".to_string(),
        ),
    ];
    if set.lanes > 1 {
        let SlotFields { p1h, p2h, p1l, p2l, sh, sl, me, .. } = slot_fields(set);
        let mut fields = String::from("  // Token-row b's octet lanes past the first: its next blocks' sums and scales out of its slot's read, the blocks' own\n  // scales the pair's.\n");
        for j in 1..set.lanes {
            let lo = if wide {
                format!("    sb{j}_p1l_b <= o_rd_b[{p1l} + 48*o_k{j} +: 48]; sb{j}_p2l_b <= o_rd_b[{p2l} + 48*o_k{j} +: 48]; sb{j}_sl_b <= o_rd_b[{sl} + 24*o_k{j} +: 24];\n")
            } else {
                format!("    sb{j}_p1l_b <= 48'd0; sb{j}_p2l_b <= 48'd0; sb{j}_sl_b <= 24'd0;\n")
            };
            fields.push_str(&format!(
                "  reg [47:0] sb{j}_p1h_b, sb{j}_p2h_b, sb{j}_p1l_b, sb{j}_p2l_b, sl{j}_p1h_b, sl{j}_p2h_b, sl{j}_p1l_b, sl{j}_p2l_b;\n  \
                 reg [23:0] sb{j}_sh_b, sb{j}_sl_b, sl{j}_sh_b, sl{j}_sl_b;\n  reg [19:0] sb{j}_me_b, sl{j}_me_b;\n  always @(posedge clk) begin\n    \
                 sb{j}_p1h_b <= o_rd_b[{p1h} + 48*o_k{j} +: 48]; sb{j}_p2h_b <= o_rd_b[{p2h} + 48*o_k{j} +: 48];\n    \
                 sb{j}_sh_b <= o_rd_b[{sh} + 24*o_k{j} +: 24]; sb{j}_me_b <= o_rd_b[{me} + 20*o_k{j} +: 20];\n{lo}  end\n"
            ));
        }
        let chain = between(&core, "  // The octet lanes past the first: lane j's value", &format!("  wire [31:0] chain2 = ch2_{};\n", set.lanes - 1));
        let mut names: Vec<String> = vec!["sum1".into(), "sum2".into(), "chain1".into(), "chain2".into()];
        for r in 1..=2 {
            for j in 0..set.lanes {
                for n in ["ch", "sv", "slim", "dv", "dk", "chain"] {
                    names.push(format!("{n}{r}_{j}"));
                }
            }
        }
        for j in 1..set.lanes {
            for f in ["p1h", "p2h", "p1l", "p2l", "sh", "sl", "me"] {
                names.push(format!("sl{j}_{f}"));
            }
        }
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let chain_b = suffixed(chain, &names, "_b").replacen("  // The octet lanes past the first: lane j's value", "  // Token-row b's octet lanes past the first, as token-row a's: lane j's value", 1);
        edits.extend([
            ("  // The lanes' inputs: the sums over the issue's passes".to_string(), format!("{fields}  // The lanes' inputs: the sums over the issue's passes")),
            (chain.to_string(), format!("{chain}{chain_b}")),
        ]);
    }
    if set.octets {
        let SlotFields { p1h, p2h, p1l, p2l, sh, sl, me, .. } = slot_fields(set);
        let o_in: String = core.lines().filter(|l| l.starts_with("  assign o_in[")).map(|l| format!("{l}\n")).collect();
        let o_in_b = suffixed(&o_in, &["o_in", "t1_hi", "t2_hi", "t1_lo", "t2_lo", "ts_hi", "ts_lo", "f_e", "f_m", "f_ms1", "f_ms2", "f_ms3", "f_ms4", "f_ms5", "f_ms6", "f_ms7"], "_b");
        let read_b = if wide {
            format!("    ob_p1l_b <= o_rd_b[{p1l} + 48*o_k +: 48]; ob_p2l_b <= o_rd_b[{p2l} + 48*o_k +: 48]; ob_sl_b <= o_rd_b[{sl} + 24*o_k +: 24];\n")
        } else {
            String::new()
        };
        edits.extend([
            ("  wire [OW-1:0] o_in;\n".to_string(), "  wire [OW-1:0] o_in, o_in_b;\n".to_string()),
            (o_in.clone(), format!("{o_in}{o_in_b}")),
            (
                "  `LOGOS_RAMSTYLE_ASYNC((2 << SW)) reg [OW-1:0] obuf [0:(2 << SW)-1];\n".to_string(),
                "  `LOGOS_RAMSTYLE_ASYNC((2 << SW)) reg [OW-1:0] obuf [0:(2 << SW)-1];\n  `LOGOS_RAMSTYLE_ASYNC((2 << SW)) reg [OW-1:0] obuf_b [0:(2 << SW)-1];\n".to_string(),
            ),
            (
                "  wire [OW-1:0] o_rd = obuf[eslot_d[PL-1]];\n".to_string(),
                "  wire [OW-1:0] o_rd = obuf[eslot_d[PL-1]], o_rd_b = obuf_b[eslot_d[PL-1]];\n".to_string(),
            ),
            (
                "  reg [19:0] ob_me;\n".to_string(),
                "  reg [19:0] ob_me;\n  reg [47:0] ob_p1h_b, ob_p2h_b, ob_p1l_b, ob_p2l_b;\n  reg [23:0] ob_sh_b, ob_sl_b;\n  reg [19:0] ob_me_b;\n".to_string(),
            ),
            (
                "    if (cap_d[PL]) obuf[wslot_d[PL]] <= o_in;\n".to_string(),
                "    if (cap_d[PL]) begin obuf[wslot_d[PL]] <= o_in; obuf_b[wslot_d[PL]] <= o_in_b; end\n".to_string(),
            ),
            (
                format!("ob_me <= o_rd[{me} + 20*o_k +: 20];\n"),
                format!(
                    "ob_me <= o_rd[{me} + 20*o_k +: 20];\n    ob_p1h_b <= o_rd_b[{p1h} + 48*o_k +: 48]; ob_p2h_b <= o_rd_b[{p2h} + 48*o_k +: 48];\n    \
                     ob_sh_b <= o_rd_b[{sh} + 24*o_k +: 24]; ob_me_b <= o_rd_b[{me} + 20*o_k +: 20];\n{read_b}"
                ),
            ),
        ]);
    }
    if set.pe_pairs {
        edits.push(("(input clk, input rst, input clear".to_string(), "(input clk, input clk2, input rst, input clear".to_string()));
    }
    let pairs_note = "// With token pairs (ColumnSet::token_pairs) each issue runs the beat held against two token-rows, a and b (the `_b`\n\
                      // signals): a word, processing elements, lane fields, sums over the passes, lanes and sums of each, the beat's fields\n\
                      // and the issue's flags shared, a sum's place a token pair's.\n";
    let mut core = replaced(&core, &edits.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());
    core.insert_str(0, pairs_note);
    out.replace_range(a..b, &core);

    let (a, b) = span(&out, &format!("module logos_column_array_{name} "));
    let array = out[a..b].to_string();
    let st_full = between(&array, &format!("      wire [{}:0] act_st;\n", cw - 1), &format!("      assign act_sts[ch*{cw} +: {cw}] = act_st;\n"));
    // A core whose elements run in clk2 staggers its words itself: each channel's words go to it unstaggered.
    let st_new = if set.pe_pairs {
        let at = st_full.find(".codes(").expect("the stagger's codes") + ".codes(".len();
        let codes = &st_full[at..at + st_full[at..].find("), .st(act_st));").expect("the stagger's codes' end")];
        format!(
            "      // The channel's words for its cores' elements in clk2, unstaggered.\n      wire [{c}:0] act_st = {codes};\n      \
             assign act_sts[ch*{cw} +: {cw}] = act_st;\n",
            c = cw - 1
        )
    } else {
        st_full.to_string()
    };
    let wm = between(&array, "wd <= act_wd; wm <= ", "; ra <= ra_n[AA-1:0]; end");
    let wm_expr = &wm["wd <= act_wd; wm <= ".len()..wm.len() - "; ra <= ra_n[AA-1:0]; end".len()];
    let wb = aw / 8;
    let mut edits: Vec<(String, String)> = vec![
        (
            "output [CH*N*32-1:0] o_val2, output reg [15:0] o_stripe,".to_string(),
            "output [CH*N*32-1:0] o_val2,\n   output [CH*N*32-1:0] o_val1_b, output [CH*N*32-1:0] o_val2_b, output reg [15:0] o_stripe,".to_string(),
        ),
        ("  localparam TW = (T_MAX > 1) ? $clog2(T_MAX) : 1;\n".to_string(), "  localparam TW = (T_MAX > 2) ? $clog2(T_MAX) - 1 : 1;\n".to_string()),
        ("  wire [TW-1:0] at = t[TW-1:0];\n".to_string(), "  wire [TW-1:0] at = t[TW:1];\n".to_string()),
        ("t_up <= t_n + 8'd1;".to_string(), "t_up <= t_n + 8'd2;".to_string()),
        ("  wire [31:0] ra_n = ({24'd0, t_n} << NB_L) | ({16'd0, hb_n} & (NB_P - 1));\n".to_string(), "  wire [31:0] ra_n = ({24'd0, t_n >> 1} << NB_L) | ({16'd0, hb_n} & (NB_P - 1));\n".to_string()),
        ("  localparam AN = T_MAX * NB_P,".to_string(), "  localparam AN = ((T_MAX + 1) / 2) * NB_P,".to_string()),
        ("  wire [31:0] act_wa = ({24'd0, act_t} << NB_L) | (act_b & (NB_P - 1));\n".to_string(), "  wire [31:0] act_wa = ({24'd0, act_t >> 1} << NB_L) | (act_b & (NB_P - 1));\n".to_string()),
        (
            format!("      reg [{}:0] wd;\n      reg [{}:0] wm;\n", aw - 1, wb - 1),
            format!("      reg [{}:0] wd;\n      reg [{}:0] wm;\n", 2 * aw - 1, 2 * wb - 1),
        ),
        (
            format!("wd <= act_wd; wm <= {wm_expr}; ra <= ra_n[AA-1:0]; end"),
            format!("wd <= {{act_wd, act_wd}}; wm <= act_t[0] ? {{{wm_expr}, {{{wb}{{1'b0}}}}}} : {{{{{wb}{{1'b0}}}}, {wm_expr}}}; ra <= ra_n[AA-1:0]; end"),
        ),
        (
            format!("      wire [{}:0] act_word;\n      {} #(.D(AN), .A(AA)) act_ram(.clk(clk), .ra(ra), .rd(act_word),", aw - 1, super::ram::bram_name(aw)),
            format!(
                "      wire [{p}:0] act_pair;\n      {} #(.D(AN), .A(AA)) act_ram(.clk(clk), .ra(ra), .rd(act_pair),",
                super::ram::bram_name(2 * aw),
                p = 2 * aw - 1
            ),
        ),
        (
            format!("      assign act_words[ch*{aw} +: {aw}] = act_word;\n"),
            format!(
                "      wire [{t}:0] act_word = act_pair[{t}:0], act_word_b = act_pair[{}:{aw}];\n      assign act_words[ch*{aw} +: {aw}] = act_word;\n      \
                 assign act_words_b[ch*{aw} +: {aw}] = act_word_b;\n",
                2 * aw - 1,
                t = aw - 1
            ),
        ),
        (st_full.to_string(), format!("{st_new}{}", suffixed(&st_new, &["act_st", "st", "act_word", "act_sts"], "_b"))),
        (
            format!("  wire [CH*{cw}-1:0] act_sts;\n"),
            format!("  wire [CH*{cw}-1:0] act_sts, act_sts_b;\n  wire [CH*{aw}-1:0] act_words_b;\n"),
        ),
        ("#(.SUMS(T_MAX), .SW(TW),".to_string(), "#(.SUMS((T_MAX + 1) / 2), .SW(TW),".to_string()),
        (
            format!(".act_word(act_words[(g/N)*{aw} +: {aw}]), "),
            format!(".act_word(act_words[(g/N)*{aw} +: {aw}]), .act_word_b(act_words_b[(g/N)*{aw} +: {aw}]),\n        "),
        ),
        (
            format!(".act_st(act_sts[(g/N)*{cw} +: {cw}]), "),
            format!(".act_st(act_sts[(g/N)*{cw} +: {cw}]), .act_st_b(act_sts_b[(g/N)*{cw} +: {cw}]), "),
        ),
        (
            ".o_val1(o_val1[g*32 +: 32]), .o_val2(o_val2[g*32 +: 32]));".to_string(),
            ".o_val1(o_val1[g*32 +: 32]), .o_val2(o_val2[g*32 +: 32]),\n        .o_val1_b(o_val1_b[g*32 +: 32]), .o_val2_b(o_val2_b[g*32 +: 32]));".to_string(),
        ),
        (
            "    if (issue) begin t <= t_last ? 8'd0 : t + 8'd1; t_last <= t_last ? tokens == 8'd1 : t + 8'd2 == tokens; end\n".to_string(),
            "    if (issue) begin t <= t_last ? 8'd0 : t + 8'd2; t_last <= t_last ? tokens <= 8'd2 : t + 8'd4 >= tokens; end\n".to_string(),
        ),
        ("t_last <= tokens == 8'd1; ready <=".to_string(), "t_last <= tokens <= 8'd2; ready <=".to_string()),
    ];
    if set.octets {
        edits.extend([
            ("  wire [TW-1:0] eat = et[TW-1:0];\n".to_string(), "  wire [TW-1:0] eat = et[TW:1];\n".to_string()),
            ("&& et + 8'd1 == tokens;\n".to_string(), "&& et + 8'd2 >= tokens;\n".to_string()),
            (
                format!("      if (et + 8'd1 == tokens) begin et <= 8'd0; eb <= eb + 3'd{l}; end else et <= et + 8'd1;\n", l = set.lanes),
                format!("      if (et + 8'd2 >= tokens) begin et <= 8'd0; eb <= eb + 3'd{l}; end else et <= et + 8'd2;\n", l = set.lanes),
            ),
        ]);
    }
    if set.pe_pairs {
        edits.push(("u(.clk(clk), .rst(rst), .clear(start && !busy),".to_string(), "u(.clk(clk), .clk2(clk2), .rst(rst), .clear(start && !busy),".to_string()));
    }
    let note = "// With token pairs (ColumnSet::token_pairs) an issue is a token pair's, t even, and each core takes the pair's two words\n\
                // from its channel's copy of the activations, a word a pair (the first token-row's in its low half), with each half's\n\
                // excess codes staggered for its elements; a sum's place is the pair's, and each core's sums emerge a pair a clock,\n\
                // the second token-row's (o_val1_b, o_val2_b) the one after o_t, a job's only when it has it.\n";
    let mut array = replaced(&array, &edits.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());
    array.insert_str(0, note);
    out.replace_range(a..b, &array);
    out
}

/// `text` with every `(from, to)` of `edits` made, each `from` there exactly once.
fn replaced(text: &str, edits: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (from, to) in edits {
        assert_eq!(out.matches(from).count(), 1, "`{from}`");
        out = out.replace(from, to);
    }
    out
}

/// What a set with octets adds to its core (`logos_column_core_<name>`): the ports its array drives an octet job by,
/// the slots an issue's sums wait in, and the lanes' inputs taken from a slot's block; nothing for a set without.
struct CoreOctets {
    ports: String,
    body: String,
    lanes: String,
    lanes_end: String,
    from: &'static str,
}

/// Where each field lies in an octet core's slot, from its lowest bit: both rows' unit sums over the passes (each
/// word's), the activation's unit sums, both rows' unit sums of codes, both rows' eight scales, the activation's eight
/// scales; `ow` the slot's bits.
struct SlotFields {
    p1h: usize,
    p2h: usize,
    p1l: usize,
    p2l: usize,
    sh: usize,
    sl: usize,
    q1: usize,
    q2: usize,
    sc1: usize,
    sc2: usize,
    me: usize,
    ow: usize,
}

fn slot_fields(set: &ColumnSet) -> SlotFields {
    let u = set.elems() / UNIT;
    let words = if set.words == Words::Two { 2 } else { 1 };
    let p1h = 0;
    let p2h = p1h + 24 * u;
    let p1l = p2h + 24 * u;
    let p2l = p1l + 24 * u * (words - 1);
    let sh = p2l + 24 * u * (words - 1);
    let sl = sh + 12 * u;
    let q1 = sl + 12 * u * (words - 1);
    let q2 = q1 + 12 * u;
    let sc1 = q2 + 12 * u;
    let sc2 = sc1 + 128;
    let me = sc2 + 128;
    SlotFields { p1h, p2h, p1l, p2l, sh, sl, q1, q2, sc1, sc2, me, ow: me + 160 }
}

fn core_octets(set: &ColumnSet) -> CoreOctets {
    if !set.octets {
        return CoreOctets { ports: String::new(), body: String::new(), lanes: String::new(), lanes_end: String::new(), from: "" };
    }
    let u = set.elems() / UNIT;
    let wide = set.words == Words::Two;
    let SlotFields { p1h, p2h, p1l, p2l, sh, sl, q1, q2, sc1, sc2, me, ow } = slot_fields(set);
    let mut body = format!(
        "  // An octet job (oct): each issue's sums over its passes, its blocks' eight scales and its activation's eight kept in\n  \
         // its slot (wslot) the clock after its last pass (capture); a slot's block (eslot, eblk) into the lanes the clock\n  \
         // after it is asked, its two units as a block's first two, the rest zero, with its scale and its activation's.\n  \
         localparam OW = {ow};\n  reg [PL:0] cap_d;\n  reg [SW:0] wslot_d [0:PL];\n  integer cw;\n  \
         always @(posedge clk) begin\n    cap_d <= rst ? {{(PL+1){{1'b0}}}} : {{cap_d[PL-1:0], capture}};\n    wslot_d[0] <= wslot;\n    \
         for (cw = 1; cw <= PL; cw = cw + 1) wslot_d[cw] <= wslot_d[cw-1];\n  end\n  wire [OW-1:0] o_in;\n  \
         assign o_in[{p1h} +: {w24}] = t1_hi;\n  assign o_in[{p2h} +: {w24}] = t2_hi;\n",
        w24 = 24 * u
    );
    if wide {
        body.push_str(&format!("  assign o_in[{p1l} +: {w24}] = t1_lo;\n  assign o_in[{p2l} +: {w24}] = t2_lo;\n", w24 = 24 * u));
    }
    body.push_str(&format!("  assign o_in[{sh} +: {w12}] = ts_hi;\n", w12 = 12 * u));
    if wide {
        body.push_str(&format!("  assign o_in[{sl} +: {w12}] = ts_lo;\n", w12 = 12 * u));
    }
    body.push_str(&format!(
        "  assign o_in[{q1} +: {w12}] = f_q1;\n  assign o_in[{q2} +: {w12}] = f_q2;\n  \
         assign o_in[{sc1} +: 128] = {{f_s71, f_a1}};\n  assign o_in[{sc2} +: 128] = {{f_s72, f_a2}};\n  \
         assign o_in[{me} +: 20] = {{f_e, f_m}};\n",
        w12 = 12 * u,
    ));
    for k in 1..8 {
        body.push_str(&format!("  assign o_in[{} +: 20] = f_ms{k};\n", me + 20 * k));
    }
    let wide_sel = if wide {
        format!("    ob_p1l <= o_rd[{p1l} + 48*o_k +: 48]; ob_p2l <= o_rd[{p2l} + 48*o_k +: 48]; ob_sl <= o_rd[{sl} + 24*o_k +: 24];\n")
    } else {
        String::new()
    };
    body.push_str(&format!(
        "  // A block asked for leaves the slots PL clocks on, as an issue's value meets the lanes PL clocks after its pass,\n  \
         // its fields taken out of its slot as the slot is read.\n  \
         `LOGOS_RAMSTYLE_ASYNC((2 << SW)) reg [OW-1:0] obuf [0:(2 << SW)-1];\n  reg [SW:0] eslot_d [0:PL-1];\n  reg [2:0] eblk_d [0:PL-1];\n  \
         wire [OW-1:0] o_rd = obuf[eslot_d[PL-1]];\n  wire [2:0] o_k = eblk_d[PL-1];\n  \
         reg [47:0] ob_p1h, ob_p2h, ob_p1l, ob_p2l;\n  reg [23:0] ob_sh, ob_sl, ob_q1, ob_q2;\n  reg [15:0] ob_a1, ob_a2;\n  \
         reg [19:0] ob_me;\n  integer ce;\n  always @(posedge clk) begin\n    \
         if (cap_d[PL]) obuf[wslot_d[PL]] <= o_in;\n    eslot_d[0] <= eslot; eblk_d[0] <= eblk;\n    \
         for (ce = 1; ce < PL; ce = ce + 1) begin eslot_d[ce] <= eslot_d[ce-1]; eblk_d[ce] <= eblk_d[ce-1]; end\n    \
         ob_p1h <= o_rd[{p1h} + 48*o_k +: 48]; ob_p2h <= o_rd[{p2h} + 48*o_k +: 48];\n    \
         ob_sh <= o_rd[{sh} + 24*o_k +: 24]; ob_q1 <= o_rd[{q1} + 24*o_k +: 24]; ob_q2 <= o_rd[{q2} + 24*o_k +: 24];\n    \
         ob_a1 <= o_rd[{sc1} + 16*o_k +: 16]; ob_a2 <= o_rd[{sc2} + 16*o_k +: 16]; ob_me <= o_rd[{me} + 20*o_k +: 20];\n{wide_sel}  end\n",
    ));
    let mut takes = String::new();
    if set.lanes > 1 {
        let mut fields = String::from(
            "  // The octet lanes past the first: a token-row's next blocks of its group, o_k + 1 on, out of the same slot's read.\n",
        );
        for j in 1..set.lanes {
            writeln!(fields, "  reg [47:0] sb{j}_p1h, sb{j}_p2h, sb{j}_p1l, sb{j}_p2l;\n  reg [23:0] sb{j}_sh, sb{j}_sl, sb{j}_q1, sb{j}_q2;\n  reg [15:0] sb{j}_a1, sb{j}_a2;\n  reg [19:0] sb{j}_me;").unwrap();
            writeln!(fields, "  reg [47:0] sl{j}_p1h, sl{j}_p2h, sl{j}_p1l, sl{j}_p2l;\n  reg [23:0] sl{j}_sh, sl{j}_sl, sl{j}_q1, sl{j}_q2;\n  reg [15:0] sl{j}_a1, sl{j}_a2;\n  reg [19:0] sl{j}_me;").unwrap();
            writeln!(fields, "  reg [2:0] o_k{j};\n  always @(posedge clk) o_k{j} <= eblk_d[PL-2] + 3'd{j};").unwrap();
            let lo = if wide {
                format!(
                    "    sb{j}_p1l <= o_rd[{p1l} + 48*o_k{j} +: 48]; sb{j}_p2l <= o_rd[{p2l} + 48*o_k{j} +: 48]; sb{j}_sl <= o_rd[{sl} + 24*o_k{j} +: 24];\n"
                )
            } else {
                format!("    sb{j}_p1l <= 48'd0; sb{j}_p2l <= 48'd0; sb{j}_sl <= 24'd0;\n")
            };
            writeln!(
                fields,
                "  always @(posedge clk) begin\n    sb{j}_p1h <= o_rd[{p1h} + 48*o_k{j} +: 48]; sb{j}_p2h <= o_rd[{p2h} + 48*o_k{j} +: 48];\n    \
                 sb{j}_sh <= o_rd[{sh} + 24*o_k{j} +: 24]; sb{j}_q1 <= o_rd[{q1} + 24*o_k{j} +: 24]; sb{j}_q2 <= o_rd[{q2} + 24*o_k{j} +: 24];\n    \
                 sb{j}_a1 <= o_rd[{sc1} + 16*o_k{j} +: 16]; sb{j}_a2 <= o_rd[{sc2} + 16*o_k{j} +: 16]; sb{j}_me <= o_rd[{me} + 20*o_k{j} +: 20];\n{lo}  end"
            )
            .unwrap();
            writeln!(
                takes,
                "      sl{j}_p1h <= sb{j}_p1h; sl{j}_p2h <= sb{j}_p2h; sl{j}_p1l <= sb{j}_p1l; sl{j}_p2l <= sb{j}_p2l; sl{j}_sh <= sb{j}_sh; sl{j}_sl <= sb{j}_sl;\n      \
                 sl{j}_q1 <= sb{j}_q1; sl{j}_q2 <= sb{j}_q2; sl{j}_a1 <= sb{j}_a1; sl{j}_a2 <= sb{j}_a2; sl{j}_me <= sb{j}_me;"
            )
            .unwrap();
        }
        body.push_str(&fields);
    }
    let zero = |w: usize| if w > 0 { format!("{{{w}{{1'b0}}}}, ") } else { String::new() };
    let pad24 = zero(24 * u - 48);
    let pad12 = zero(12 * u - 24);
    let mut lanes = format!(
        "    if (oct) begin\n      \
         p1_hi <= {{{pad24}ob_p1h}};\n      p2_hi <= {{{pad24}ob_p2h}};\n"
    );
    if wide {
        lanes.push_str(&format!("      p1_lo <= {{{pad24}ob_p1l}};\n      p2_lo <= {{{pad24}ob_p2l}};\n"));
        lanes.push_str(&format!("      s_lo <= {{{pad12}ob_sl}};\n"));
    } else {
        lanes.push_str("      p1_lo <= 0; p2_lo <= 0; s_lo <= 0;\n");
    }
    lanes.push_str(&format!(
        "      s_hi <= {{{pad12}ob_sh}};\n      q1 <= {{{pad12}ob_q1}};\n      q2 <= {{{pad12}ob_q2}};\n      \
         lc1 <= {{{pad8}16'h0101}}; ln1 <= 0; lc2 <= {{{pad8}16'h0101}}; ln2 <= 0;\n      \
         la1 <= ob_a1; lb1 <= 16'd0; la2 <= ob_a2; lb2 <= 16'd0;\n      \
         lm <= ob_me[7:0];\n      le <= ob_me[19:8];\n{takes}    end else begin\n",
        pad8 = zero(8 * u - 16)
    ));
    CoreOctets {
        ports: ", input oct, input capture, input [SW:0] wslot, input [SW:0] eslot, input [2:0] eblk".to_string(),
        body,
        lanes,
        lanes_end: "    end\n".to_string(),
        from: ", or for an octet job from the slot its array asks for",
    }
}

/// What a set with octets adds to its array (`logos_column_array_<name>`), keyed by the template's placeholders; a set
/// without keeps the array as it is.
fn array_octets(set: &ColumnSet) -> Vec<(&'static str, String)> {
    let all = format!("{{{}{{1'b1}}}}", set.act_bits() / 8);
    if !set.octets {
        return vec![
            ("arr_act_s", String::new()),
            ("arr_octets", String::new()),
            ("pe_ok", "!stall".to_string()),
            ("arr_busy", String::new()),
            ("act_wm", all),
            ("core_issue", "issue && need[g]".to_string()),
            ("core_octet_conn", String::new()),
            ("tags_in", "    live[0] <= issue; tag[0] <= at; ts[0] <= hs; tt[0] <= t; last[0] <= step_end;".to_string()),
            ("arr_octet_step", String::new()),
            ("arr_octet_reset", String::new()),
        ];
    }
    let place = set.formats.iter().position(|&t| set.octet(t)).expect("a set with octets takes Q8_0");
    let oct = if set.formats.len() > 1 { format!("fmt_q == 4'd{place}") } else { "1'b1".to_string() };
    // The bytes an octet's block `k` writes of a word: each word's codes of the block, each word's unit sums of its two
    // units, and its scale.
    let (e, u, w) = (set.elems(), set.elems() / UNIT, set.word_count());
    let (a, ab) = (set.act_fields(), set.act_bits() / 8);
    let mask = |k: usize| -> String {
        let mut bytes = vec![false; ab];
        for word in 0..w {
            for b in word * e + 32 * k..word * e + 32 * k + 32 {
                bytes[b] = true;
            }
            let sums = (a.sums + word * 12 * u) / 8 + 3 * k;
            bytes[sums..sums + 3].iter_mut().for_each(|b| *b = true);
        }
        let scale = (a.m + 24 * k) / 8;
        bytes[scale..(scale + 3).min(ab)].iter_mut().for_each(|b| *b = true);
        let bits: String = bytes.iter().rev().map(|&b| if b { '1' } else { '0' }).collect();
        format!("{ab}'b{bits}")
    };
    let mut masks = String::new();
    for k in 0..8 {
        writeln!(masks, "      3'd{k}: slot_wm = {};", mask(k)).unwrap();
    }
    let octets = format!(
        r#"  // An octet job (oct): a step's issues fill one of two buffers, a slot a token-row, each core keeping its own sums; a
  // full buffer's blocks leave for the lanes a block a clock — every token-row's block 0, then every token-row's block
  // 1, ... — each the clock no add holds its sum, as an issue of the step's would; a step issues into a buffer once its
  // blocks have all left, and a buffer's first block leaves three clocks after its last issue, its slots written. A
  // block's activations are written into its octet's word under the mask of its bytes.
  wire oct = {oct};
  reg pbuf, ebuf;
  reg [1:0] full;
  `LOGOS_RAMSTYLE_ASYNC(2) reg [1:0] settle [0:1];
  `LOGOS_RAMSTYLE_ASYNC(2) reg [15:0] bs [0:1];
  `LOGOS_RAMSTYLE_ASYNC(2) reg bl [0:1];
  `LOGOS_RAMSTYLE_ASYNC(2) reg [C-1:0] bneed [0:1];
  reg [2:0] eb;
  reg [7:0] et;
  wire [TW-1:0] eat = et[TW-1:0];
  reg e_stall;
  integer h2;
  always @* begin
    e_stall = 1'b0;
    for (h2 = 0; h2 < {window}; h2 = h2 + 1) if (live[h2] && tag[h2] == eat) e_stall = 1'b1;
  end
  wire emit = oct && full[ebuf] && settle[ebuf] == 2'd0 && !hold && !e_stall;
  wire e_end = emit && eb == 3'd{last_group} && et + 8'd1 == tokens;
  reg [{abt}:0] slot_wm;
  always @*
    case (act_s)
{masks}      default: slot_wm = {{{ab}{{1'b0}}}};
    endcase
  wire [{abt}:0] act_wm = oct ? slot_wm : {all};
"#,
        abt = ab - 1,
        window = if set.lanes == 1 { "ADD".to_string() } else { format!("{} * ADD", set.lanes) },
        last_group = 8 - set.lanes,
    );
    vec![
        ("arr_act_s", ", input [2:0] act_s".to_string()),
        ("arr_octets", octets),
        ("pe_ok", "(oct ? !full[pbuf] : !stall)".to_string()),
        ("arr_busy", " || (|full)".to_string()),
        ("act_wm", "act_wm".to_string()),
        ("core_issue", "oct ? emit && bneed[ebuf][g] : issue && need[g]".to_string()),
        ("core_octet_conn", ", .oct(oct), .capture(oct && issue && need[g]), .wslot({pbuf, at}), .eslot({ebuf, eat}), .eblk(eb)".to_string()),
        (
            "tags_in",
            format!("    live[0] <= oct ? emit : issue;\n    tag[0] <= oct ? eat : at;\n    ts[0] <= oct ? bs[ebuf] : hs;\n    \
             tt[0] <= oct ? et : t;\n    last[0] <= oct ? bl[ebuf] && eb == 3'd{} : step_end;",
            8 - set.lanes
        ),
        ),
        (
            "arr_octet_step",
            format!("    if (settle[0] != 2'd0) settle[0] <= settle[0] - 2'd1;\n    if (settle[1] != 2'd0) settle[1] <= settle[1] - 2'd1;\n    \
             if (oct && fin) begin\n      full[pbuf] <= 1'b1; settle[pbuf] <= 2'd3; bs[pbuf] <= hs; bl[pbuf] <= step_end; bneed[pbuf] <= need;\n      \
             pbuf <= ~pbuf;\n    end\n    if (emit) begin\n      if (et + 8'd1 == tokens) begin et <= 8'd0; eb <= eb + 3'd{}; end else et <= et + 8'd1;\n      \
             if (e_end) begin full[ebuf] <= 1'b0; ebuf <= ~ebuf; end\n    end\n",
            set.lanes
        ),
        ),
        ("arr_octet_reset", " pbuf <= 1'b0; ebuf <= 1'b0; full <= 2'b00; eb <= 3'd0; et <= 8'd0;".to_string()),
    ]
}

/// The decoder of an octet: eight Q8_0 blocks of a row, one after another, as one 256-element block — block `k`'s
/// codes in units 2k and 2k + 1, every unit's multiplier 1 and no offset term, block 0's scale `a` and blocks 1 to 7's
/// in `s7`, as eight `logos_dec_q8_0` decode them.
const OCTET_DECODER: &str = r#"
module logos_dec_q8_0x8(input [2175:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a,
                        output [15:0] b, output [111:0] s7);
  genvar k;
  generate
    for (k = 0; k < 8; k = k + 1) begin : block
      wire [255:0] ck;
      wire [31:0] cc, nn;
      wire [15:0] ak, bk;
      logos_dec_q8_0 d(.blk(blk[272*k +: 272]), .code(ck), .c(cc), .n(nn), .a(ak), .b(bk));
      assign code[256*k +: 256] = ck;
      assign c[32*k +: 32] = cc;
      assign n[32*k +: 32] = nn;
      if (k == 0) begin : first
        assign a = ak;
      end else begin : later
        assign s7[16*(k-1) +: 16] = ak;
      end
    end
  endgenerate
  assign b = 16'd0;
endmodule
"#;

/// A format as a set's column takes it: its decoder, and whether its blocks come in octets ([`ColumnSet::octets`]).
#[derive(Clone, Copy)]
struct Coded {
    dec: &'static Decoder,
    octet: bool,
    slot: usize,
}

impl Coded {
    /// The elements of one of the column's blocks.
    fn len(&self) -> usize {
        if self.octet { 8 * self.dec.ty.block_len() } else { self.dec.ty.block_len() }
    }

    /// The bytes of one of the column's blocks.
    fn bytes(&self) -> usize {
        if self.octet { 8 * self.dec.ty.block_bytes() } else { self.dec.ty.block_bytes() }
    }

    /// The bytes one of the column's blocks takes in the image, its pad to the set's slot after it.
    fn slot_bytes(&self) -> usize {
        self.bytes().next_multiple_of(self.slot)
    }

    /// The decoder module: the format's, or its octets' (`logos_dec_q8_0x8`).
    fn module(&self) -> String {
        if self.octet { format!("{}x8", decoder_name(self.dec.ty)) } else { decoder_name(self.dec.ty) }
    }

    /// The format's name, an octet's with `x8`.
    fn name(&self) -> String {
        if self.octet { format!("{}x8", self.dec.ty.name()) } else { self.dec.ty.name().to_string() }
    }
}

/// The decode of a beat: one format's decoders, or each format's with the job's chosen, a narrower format's codes and
/// multipliers zero past its own; an octet's decoder its blocks' scales past the first beside them (`s7`).
fn decode(decs: &[Coded], elems: usize) -> String {
    let (code_top, unit_top) = (8 * elems - 1, 16 * (elems / UNIT) - 1);
    let s7 = |d: &Coded, k: &str| if d.octet { format!(", .s7({k})") } else { String::new() };
    let octets = decs.iter().any(|d| d.octet);
    if let [d] = decs {
        let dec = d.module();
        return format!(
            "  wire [{code_top}:0] code1_d, code2_d;\n  wire [{unit_top}:0] c1_d, n1_d, c2_d, n2_d;\n  wire [15:0] a1_d, b1_d, a2_d, b2_d;\n{}  \
             {dec} d1(.blk(blk1), .code(code1_d), .c(c1_d), .n(n1_d), .a(a1_d), .b(b1_d){});\n  \
             {dec} d2(.blk(blk2), .code(code2_d), .c(c2_d), .n(n2_d), .a(a2_d), .b(b2_d){});\n",
            if octets { "  wire [111:0] s71_d, s72_d;\n" } else { "" },
            s7(d, "s71_d"),
            s7(d, "s72_d"),
        );
    }
    let mut v = String::new();
    for (i, d) in decs.iter().enumerate() {
        let (len, bits) = (d.len(), 8 * d.bytes());
        let (ct, ut) = (8 * len - 1, 16 * (len / UNIT) - 1);
        let dec = d.module();
        writeln!(v, "  wire [{ct}:0] code1_{i}, code2_{i};\n  wire [{ut}:0] c1_{i}, n1_{i}, c2_{i}, n2_{i};\n  wire [15:0] a1_{i}, b1_{i}, a2_{i}, b2_{i};").unwrap();
        if d.octet {
            writeln!(v, "  wire [111:0] s71_{i}, s72_{i};").unwrap();
        }
        writeln!(v, "  {dec} d1_{i}(.blk(blk1[{}:0]), .code(code1_{i}), .c(c1_{i}), .n(n1_{i}), .a(a1_{i}), .b(b1_{i}){});", bits - 1, s7(d, &format!("s71_{i}"))).unwrap();
        writeln!(v, "  {dec} d2_{i}(.blk(blk2[{}:0]), .code(code2_{i}), .c(c2_{i}), .n(n2_{i}), .a(a2_{i}), .b(b2_{i}){});", bits - 1, s7(d, &format!("s72_{i}"))).unwrap();
    }
    writeln!(v, "  reg [{code_top}:0] code1_d, code2_d;\n  reg [{unit_top}:0] c1_d, n1_d, c2_d, n2_d;\n  reg [15:0] a1_d, b1_d, a2_d, b2_d;").unwrap();
    if octets {
        v.push_str("  reg [111:0] s71_d, s72_d;\n");
    }
    v.push_str("  always @* begin\n    code1_d = 0; code2_d = 0; c1_d = 0; n1_d = 0; c2_d = 0; n2_d = 0; a1_d = 0; b1_d = 0; a2_d = 0; b2_d = 0;\n");
    if octets {
        v.push_str("    s71_d = 0; s72_d = 0;\n");
    }
    v.push_str("    case (fmt)\n");
    for (i, d) in decs.iter().enumerate() {
        let scales = if d.octet { format!(" s71_d = s71_{i}; s72_d = s72_{i};") } else { String::new() };
        writeln!(
            v,
            "      4'd{i}: begin code1_d = code1_{i}; code2_d = code2_{i}; c1_d = c1_{i}; n1_d = n1_{i}; c2_d = c2_{i}; n2_d = n2_{i}; \
             a1_d = a1_{i}; b1_d = b1_{i}; a2_d = a2_{i}; b2_d = b2_{i};{scales} end"
        )
        .unwrap();
    }
    v.push_str("      default: ;\n    endcase\n  end\n");
    v
}

/// The job's format's code offset `z`: one format's, or the job's format's.
fn zsel(decs: &[Coded]) -> String {
    if let [d] = decs {
        return format!("  wire [7:0] z = 8'd{};\n", d.dec.offset);
    }
    let mut v = String::from("  reg [7:0] z;\n  always @*\n    case (fmt_q)\n");
    for (i, d) in decs.iter().enumerate() {
        writeln!(v, "      4'd{i}: z = 8'd{};", d.dec.offset).unwrap();
    }
    writeln!(v, "      default: z = 8'd{};\n    endcase", decs[0].dec.offset).unwrap();
    v
}

/// The feed's block bytes `BB` and its gather register's bytes `GB`: one format's, or the job's format's beside room
/// for the set's widest.
fn feed_block_bytes(decs: &[Coded]) -> String {
    if let [d] = decs {
        return format!("  localparam [15:0] BB = 16'd{b};\n  localparam GB = {b} + 64;\n", b = d.slot_bytes());
    }
    let widest = decs.iter().map(Coded::slot_bytes).max().unwrap();
    // The job's format holds through its job and its first beat comes clocks after it is set: its block's bytes are
    // registered from the feed's copy.
    let mut v = format!("  localparam GB = {widest} + 64;\n  reg [15:0] BB;\n  always @(posedge clk)\n    case (fmt_q)\n");
    for (i, d) in decs.iter().enumerate() {
        writeln!(v, "      4'd{i}: BB <= 16'd{};", d.slot_bytes()).unwrap();
    }
    writeln!(v, "      default: BB <= 16'd{};\n    endcase", decs[0].slot_bytes()).unwrap();
    v
}

/// The gather register with its first block dropped: by one format's block's slot, or by the job's format's.
fn feed_drop(decs: &[Coded]) -> String {
    if let [_] = decs {
        return "gather >> (8 * BB)".to_string();
    }
    let mut v = String::new();
    for (i, d) in decs.iter().enumerate().take(decs.len() - 1) {
        write!(v, "(fmt_q == 4'd{i}) ? gather >> {} : ", 8 * d.slot_bytes()).unwrap();
    }
    write!(v, "gather >> {}", 8 * decs[decs.len() - 1].slot_bytes()).unwrap();
    v
}

/// The job's format's code digits, `digits`: one format's, or the job's format's.
fn digits(decs: &[Coded]) -> String {
    let of = |d: &Coded| if d.dec.code_bits > 4 { 2 } else { 1 };
    if let [d] = decs {
        return format!("  wire [1:0] digits = 2'd{};\n", of(d));
    }
    let mut v = String::from("  reg [1:0] digits;\n  always @*\n    case (fmt_q)\n");
    for (i, d) in decs.iter().enumerate() {
        writeln!(v, "      4'd{i}: digits = 2'd{};", of(d)).unwrap();
    }
    writeln!(v, "      default: digits = 2'd{};\n    endcase", of(&decs[0])).unwrap();
    v
}

/// The activation codes of a pass as the systolic elements take them.
const ACT_STAGGER: &str = r#"
// A pass's excess codes for logos_pe8_p: code e of each eight given (e mod 8) / 2 clocks after it comes in, the element's
// block (e mod 8) / 2 taking its operands that many clocks after its block 0 does.
module logos_column_act_stagger #(parameter ELEMS = 256) (input clk, input [8*ELEMS-1:0] codes, output [8*ELEMS-1:0] st);
  reg [8*ELEMS-1:0] d1, d2, d3;
  always @(posedge clk) begin
    d1 <= codes;
    d2 <= d1;
    d3 <= d2;
  end
  genvar e;
  generate
    for (e = 0; e < ELEMS; e = e + 1) begin : code
      localparam K = (e % 8) / 2;
      assign st[8*e +: 8] = K == 0 ? codes[8*e +: 8] : K == 1 ? d1[8*e +: 8] : K == 2 ? d2[8*e +: 8] : d3[8*e +: 8];
    end
  endgenerate
endmodule
"#;

/// One pass of a block of two rows' codes against one token-row's activation word: each 16-element unit's partial
/// sums for both rows from the systolic processing elements of one 4-bit code digit, the word's excess codes and their
/// unit sums given, [`PASS_LATENCY`] clocks after the word, the codes and their digit given a clock before it.
const MATVEC_PASS: &str = r#"
// A pass of two rows' codes against one activation word's excess codes `w`, for the user clock: digit `digit` of every
// code, given with the codes a clock before the word and registered, through the systolic elements; each 16-element
// unit's partial sums for both rows, row one's from the low lanes and row two's from the high lanes with its offset
// restored by the unit's sum `s` of the word's excess codes, @PE@ clocks after the word (`s` delayed beside the
// elements).
module logos_matvec_pass_p #(parameter UNITS = 16)
  (input clk, input [128*UNITS-1:0] code1, input [128*UNITS-1:0] code2, input [128*UNITS-1:0] w, input [12*UNITS-1:0] s,
   input digit, output [24*UNITS-1:0] p1, output [24*UNITS-1:0] p2);
  wire [12*UNITS-1:0] s_pe;
  logos_delay #(.W(12*UNITS), .D(@PE@)) sd(.clk(clk), .d(s), .q(s_pe));
  genvar i, u;
  generate
    for (u = 0; u < UNITS; u = u + 1) begin : unit
      wire [31:0] q1a, q2a, q1b, q2b;
      for (i = 0; i < 8; i = i + 1) begin : nib
        assign q1a[4*i +: 4] = digit ? code1[8*(16*u + i) + 4 +: 4] : code1[8*(16*u + i) +: 4];
        assign q2a[4*i +: 4] = digit ? code2[8*(16*u + i) + 4 +: 4] : code2[8*(16*u + i) +: 4];
        assign q1b[4*i +: 4] = digit ? code1[8*(16*u + 8 + i) + 4 +: 4] : code1[8*(16*u + 8 + i) +: 4];
        assign q2b[4*i +: 4] = digit ? code2[8*(16*u + 8 + i) + 4 +: 4] : code2[8*(16*u + 8 + i) +: 4];
      end
      reg [31:0] q1a_r, q2a_r, q1b_r, q2b_r;
      always @(posedge clk) begin q1a_r <= q1a; q2a_r <= q2a; q1b_r <= q1b; q2b_r <= q2b; end
      wire [14:0] lo_a, lo_b;
      wire signed [16:0] hi_a, hi_b;
      logos_pe8_p pa(.clk(clk), .a(w[8*16*u +: 64]), .q1(q1a_r), .q2(q2a_r), .lo(lo_a), .hi(hi_a));
      logos_pe8_p pb(.clk(clk), .a(w[8*(16*u + 8) +: 64]), .q1(q1b_r), .q2(q2b_r), .lo(lo_b), .hi(hi_b));
      wire [23:0] one = {9'd0, lo_a} + {9'd0, lo_b};
      wire signed [23:0] other = $signed(hi_a) + $signed(hi_b) + $signed({9'd0, s_pe[12*u +: 12], 3'd0});
      assign p1[24*u +: 24] = one;
      assign p2[24*u +: 24] = other;
    end
  endgenerate
endmodule
"#;

/// `logos_matvec_pass_x2`: two cores' passes, each `logos_matvec_pass_p`'s at its latency ([`PASS_LATENCY`]), on one set
/// of processing elements in a clock of twice the cores'.
pub fn matvec_pass_x2_verilog() -> String {
    MATVEC_PASS_X2.replace("@PE@", &PASS_LATENCY.to_string())
}

// The pair's sums are realigned to the cores' clock for a chain of four blocks: core a's leave the elements CHAIN + 2
// clocks of clk2 after its word, core b's one later, and two of clk and one bring them to PASS_LATENCY.
const _: () = assert!(CHAIN == 4 && PASS_LATENCY == CHAIN + 1);

const MATVEC_PASS_X2: &str = r#"
// Two cores' passes of logos_matvec_pass_p on one set of systolic elements in a clock of twice theirs (clk2, its rising
// edges on clk's and between them): the word the cores share (`w`, unstaggered) and core a's code digits taken at
// clk2's edge between clk's, core b's at clk's next edge, the elements' blocks staggered in clk2; each core's partials
// as logos_matvec_pass_p gives them, @PE@ clocks of clk after its word, the codes and their digit given a clock before it.
module logos_matvec_pass_x2 #(parameter UNITS = 16)
  (input clk, input clk2,
   input [128*UNITS-1:0] code1a, input [128*UNITS-1:0] code2a, input [128*UNITS-1:0] code1b, input [128*UNITS-1:0] code2b,
   input [128*UNITS-1:0] w, input [12*UNITS-1:0] s, input digit,
   output [24*UNITS-1:0] p1a, output [24*UNITS-1:0] p2a, output [24*UNITS-1:0] p1b, output [24*UNITS-1:0] p2b);
  wire [12*UNITS-1:0] s_pe;
  logos_delay #(.W(12*UNITS), .D(@PE@)) sd(.clk(clk), .d(s), .q(s_pe));
  // Core a's turn on clk2: the clock after its edge between clk's, a toggle of clk's seen changed.
  reg tf = 1'b0, tf2 = 1'b0, a_turn = 1'b0;
  always @(posedge clk) tf <= ~tf;
  always @(posedge clk2) begin tf2 <= tf; a_turn <= tf != tf2; end
  // The word, held across both turns of its clock.
  reg [128*UNITS-1:0] w0;
  always @(posedge clk2) w0 <= w;
  genvar i, u;
  generate
    for (u = 0; u < UNITS; u = u + 1) begin : unit
      // Each core's digits of the two rows, registered on clk a clock before the word, then held on clk2 for its turn.
      wire [63:0] da, db, ea, eb;
      for (i = 0; i < 16; i = i + 1) begin : nib
        assign da[4*i +: 4] = digit ? code1a[8*(16*u + i) + 4 +: 4] : code1a[8*(16*u + i) +: 4];
        assign db[4*i +: 4] = digit ? code1b[8*(16*u + i) + 4 +: 4] : code1b[8*(16*u + i) +: 4];
        assign ea[4*i +: 4] = digit ? code2a[8*(16*u + i) + 4 +: 4] : code2a[8*(16*u + i) +: 4];
        assign eb[4*i +: 4] = digit ? code2b[8*(16*u + i) + 4 +: 4] : code2b[8*(16*u + i) +: 4];
      end
      reg [127:0] ra, rb, ha, hb;
      always @(posedge clk) begin ra <= {ea, da}; rb <= {eb, db}; end
      always @(posedge clk2) begin
        if (!a_turn) ha <= ra;
        if (a_turn) hb <= rb;
      end
      wire [127:0] q = a_turn ? ha : hb;
      // The word's codes for the unit's two elements, each block of two codes the clocks of clk2 its place in the chain.
      reg [127:0] w1, w2, w3;
      always @(posedge clk2) begin w1 <= w0[8*16*u +: 128]; w2 <= w1; w3 <= w2; end
      wire [127:0] st;
      for (i = 0; i < 16; i = i + 1) begin : stag
        localparam K = (i % 8) / 2;
        assign st[8*i +: 8] = K == 0 ? w0[8*(16*u + i) +: 8] : K == 1 ? w1[8*i +: 8] : K == 2 ? w2[8*i +: 8] : w3[8*i +: 8];
      end
      wire [14:0] lo_a, lo_b;
      wire signed [16:0] hi_a, hi_b;
      logos_pe8_p pa(.clk(clk2), .a(st[0 +: 64]), .q1(q[0 +: 32]), .q2(q[64 +: 32]), .lo(lo_a), .hi(hi_a));
      logos_pe8_p pb(.clk(clk2), .a(st[64 +: 64]), .q1(q[32 +: 32]), .q2(q[96 +: 32]), .lo(lo_b), .hi(hi_b));
      wire [23:0] one = {9'd0, lo_a} + {9'd0, lo_b};
      wire signed [23:0] other = $signed(hi_a) + $signed(hi_b);
      // Core a's sums caught at the edge between clk's after them, core b's at clk's edge after them, then on clk.
      reg [47:0] ca, cb, fa, fa2, fb;
      always @(posedge clk2) begin
        if (!a_turn) ca <= {other, one};
        if (a_turn) cb <= {other, one};
      end
      always @(posedge clk) begin fa <= ca; fa2 <= fa; fb <= cb; end
      assign p1a[24*u +: 24] = fa2[23:0];
      assign p2a[24*u +: 24] = $signed(fa2[47:24]) + $signed({9'd0, s_pe[12*u +: 12], 3'd0});
      assign p1b[24*u +: 24] = fb[23:0];
      assign p2b[24*u +: 24] = $signed(fb[47:24]) + $signed({9'd0, s_pe[12*u +: 12], 3'd0});
    end
  endgenerate
endmodule
"#;

/// `logos_matvec_pass_x2t`: a core's two token-rows' passes, each `logos_matvec_pass_p`'s at its latency
/// ([`PASS_LATENCY`]), on one set of processing elements in a clock of twice the core's.
pub fn matvec_pass_x2t_verilog() -> String {
    MATVEC_PASS_X2T.replace("@PE@", &PASS_LATENCY.to_string())
}
const MATVEC_PASS_X2T: &str = r#"
// A core's two token-rows' passes of logos_matvec_pass_p on one set of systolic elements in a clock of twice theirs (clk2,
// its rising edges on clk's and between them): the beat's codes, the same for both, and token-row a's word (`wa`,
// unstaggered) taken at clk2's edge between clk's, token-row b's (`wb`) at clk's next edge, the elements' blocks staggered
// in clk2; each token-row's partials as logos_matvec_pass_p gives them, @PE@ clocks of clk after its word, the codes and
// their digit given a clock before it.
module logos_matvec_pass_x2t #(parameter UNITS = 16)
  (input clk, input clk2, input [128*UNITS-1:0] code1, input [128*UNITS-1:0] code2,
   input [128*UNITS-1:0] wa, input [128*UNITS-1:0] wb, input [12*UNITS-1:0] sa, input [12*UNITS-1:0] sb, input digit,
   output [24*UNITS-1:0] p1a, output [24*UNITS-1:0] p2a, output [24*UNITS-1:0] p1b, output [24*UNITS-1:0] p2b);
  wire [12*UNITS-1:0] sa_pe, sb_pe;
  logos_delay #(.W(12*UNITS), .D(@PE@)) sda(.clk(clk), .d(sa), .q(sa_pe));
  logos_delay #(.W(12*UNITS), .D(@PE@)) sdb(.clk(clk), .d(sb), .q(sb_pe));
  // Token-row a's turn on clk2: the clock after its edge between clk's, a toggle of clk's seen changed.
  reg tf = 1'b0, tf2 = 1'b0, a_turn = 1'b0;
  always @(posedge clk) tf <= ~tf;
  always @(posedge clk2) begin tf2 <= tf; a_turn <= tf != tf2; end
  // Each turn's word, taken at the edge before its turn.
  reg [128*UNITS-1:0] w0;
  always @(posedge clk2) w0 <= a_turn ? wb : wa;
  genvar i, u;
  generate
    for (u = 0; u < UNITS; u = u + 1) begin : unit
      // The beat's digits of the two rows, registered on clk a clock before the words, then held on clk2 for each turn.
      wire [63:0] d, e;
      for (i = 0; i < 16; i = i + 1) begin : nib
        assign d[4*i +: 4] = digit ? code1[8*(16*u + i) + 4 +: 4] : code1[8*(16*u + i) +: 4];
        assign e[4*i +: 4] = digit ? code2[8*(16*u + i) + 4 +: 4] : code2[8*(16*u + i) +: 4];
      end
      reg [127:0] r, ha, hb;
      always @(posedge clk) r <= {e, d};
      always @(posedge clk2) begin
        if (!a_turn) ha <= r;
        if (a_turn) hb <= r;
      end
      wire [127:0] q = a_turn ? ha : hb;
      // The turn's word's codes for the unit's two elements, each block of two codes the clocks of clk2 its place in the
      // chain.
      reg [127:0] w1, w2, w3;
      always @(posedge clk2) begin w1 <= w0[8*16*u +: 128]; w2 <= w1; w3 <= w2; end
      wire [127:0] st;
      for (i = 0; i < 16; i = i + 1) begin : stag
        localparam K = (i % 8) / 2;
        assign st[8*i +: 8] = K == 0 ? w0[8*(16*u + i) +: 8] : K == 1 ? w1[8*i +: 8] : K == 2 ? w2[8*i +: 8] : w3[8*i +: 8];
      end
      wire [14:0] lo_a, lo_b;
      wire signed [16:0] hi_a, hi_b;
      logos_pe8_p pa(.clk(clk2), .a(st[0 +: 64]), .q1(q[0 +: 32]), .q2(q[64 +: 32]), .lo(lo_a), .hi(hi_a));
      logos_pe8_p pb(.clk(clk2), .a(st[64 +: 64]), .q1(q[32 +: 32]), .q2(q[96 +: 32]), .lo(lo_b), .hi(hi_b));
      wire [23:0] one = {9'd0, lo_a} + {9'd0, lo_b};
      wire signed [23:0] other = $signed(hi_a) + $signed(hi_b);
      // Token-row a's sums caught at the edge between clk's after them, b's at clk's edge after them, then on clk.
      reg [47:0] ca, cb, fa, fa2, fb;
      always @(posedge clk2) begin
        if (!a_turn) ca <= {other, one};
        if (a_turn) cb <= {other, one};
      end
      always @(posedge clk) begin fa <= ca; fa2 <= fa; fb <= cb; end
      assign p1a[24*u +: 24] = fa2[23:0];
      assign p2a[24*u +: 24] = $signed(fa2[47:24]) + $signed({9'd0, sa_pe[12*u +: 12], 3'd0});
      assign p1b[24*u +: 24] = fb[23:0];
      assign p2b[24*u +: 24] = $signed(fb[47:24]) + $signed({9'd0, sb_pe[12*u +: 12], 3'd0});
    end
  endgenerate
endmodule
"#;

/// An activation block as its words' excess codes and their unit sums: what every core's processing elements read.
const ACT_WORD: &str = r#"
// A token-row's activation block as block RAM holds it for the column: each activation word's excess codes — the high
// (or only) word in excess 128, the low byte of a two-word code as it is — and each 16-element unit's sum of them, then
// SLOTS scales' significands and exponents, 24 bits a scale, the scale `m`·2^`e` at `slot` and zeros at the others (a
// word of an octet's eight blocks is written a block at a time under a mask of its bytes); built for one word (WORDS 1),
// `two` is never set.
module logos_column_act_word #(parameter ELEMS = 256, WORDS = 1, SLOTS = 1)
  (input [16*ELEMS-1:0] codes, input two, input [7:0] m, input [11:0] e, input [2:0] slot,
   output [((WORDS*(8*ELEMS + 12*(ELEMS/16)) + 20 + 24*(SLOTS-1) + 7)/8)*8-1:0] word);
  localparam UNITS = ELEMS / 16, SUMS = WORDS * 8 * ELEMS, SCALE = SUMS + WORDS * 12 * UNITS;
  localparam TOP = ((WORDS*(8*ELEMS + 12*(ELEMS/16)) + 20 + 24*(SLOTS-1) + 7)/8)*8, SCALES = 20 + 24 * (SLOTS - 1);
  // A unit's sixteen excess codes summed in a balanced tree of four adds.
  function [11:0] sum16(input [127:0] c);
    reg [8:0] a0, a1, a2, a3, a4, a5, a6, a7;
    reg [9:0] b0, b1, b2, b3;
    reg [10:0] d0, d1;
    begin
      a0 = c[7:0] + c[15:8]; a1 = c[23:16] + c[31:24]; a2 = c[39:32] + c[47:40]; a3 = c[55:48] + c[63:56];
      a4 = c[71:64] + c[79:72]; a5 = c[87:80] + c[95:88]; a6 = c[103:96] + c[111:104]; a7 = c[119:112] + c[127:120];
      b0 = a0 + a1; b1 = a2 + a3; b2 = a4 + a5; b3 = a6 + a7;
      d0 = b0 + b1; d1 = b2 + b3;
      sum16 = d0 + d1;
    end
  endfunction
  wire [8*ELEMS-1:0] hi, lo;
  genvar i, u;
  generate
    for (i = 0; i < ELEMS; i = i + 1) begin : element
      wire [15:0] code = codes[16*i +: 16];
      assign hi[8*i +: 8] = (WORDS > 1 && two) ? (code[15:8] + 8'd128) : (code[7:0] + 8'd128);
      assign lo[8*i +: 8] = (WORDS > 1 && two) ? code[7:0] : 8'd0;
    end
    for (u = 0; u < UNITS; u = u + 1) begin : unit
      wire [11:0] sh = sum16(hi[128*u +: 128]), sl = sum16(lo[128*u +: 128]);
      assign word[SUMS + 12*u +: 12] = sh;
      if (WORDS > 1) begin : low_sum
        assign word[SUMS + 12*UNITS + 12*u +: 12] = sl;
      end
    end
    if (WORDS > 1) begin : low_word
      assign word[16*ELEMS-1:8*ELEMS] = lo;
    end
    for (u = 0; u < SLOTS; u = u + 1) begin : scale
      localparam [2:0] AT = u;
      assign word[SCALE + 24*u +: 8] = (slot == AT) ? m : 8'd0;
      assign word[SCALE + 24*u + 8 +: 12] = (slot == AT) ? e : 12'd0;
      if (u + 1 < SLOTS) begin : gap
        assign word[SCALE + 24*u + 20 +: 4] = 4'd0;
      end
    end
    if (TOP > SCALE + SCALES) begin : pad
      assign word[TOP-1:SCALE+SCALES] = {(TOP-SCALE-SCALES){1'b0}};
    end
  endgenerate
  assign word[8*ELEMS-1:0] = hi;
endmodule
"#;
