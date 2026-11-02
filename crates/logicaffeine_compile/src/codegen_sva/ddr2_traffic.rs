//! ddr2_traffic — the K7 traffic: one stream of 64-bit words that every DDR2 controller of the comparative bench
//! writes to the same part and reads back, the device and testbenches it runs on, and the judgement of a run's log.
//!
//! The stream is words, not any controller's requests. A word index is LiteDRAM's native-port address under
//! ROW_BANK_COL, and [`place`] puts it in a bank, a row and four x16 columns. Each controller carries the stream
//! through its own request interface — whole words, or a 16-bit port's four parts — and its testbench logs every
//! command the bus carried, every request the controller accepted and every word it delivered. [`judge`] holds the
//! bus to the JEDEC checker ([`super::ddr2_jedec::check_bus`]), matches each request to the column command that
//! served it, checks that every word read back is the word written, and measures: request → column command
//! latency, the stream's span, and the bus's.
//!
//! The operating point is the Nexys A7's DDR2, Micron MT47H64M16HR-25, at DDR2-400: the clock LiteDRAM's
//! controller drives it at from a 100 MHz system clock at 1:2, and a 1:1 controller from 200 MHz.

use super::ddr2_dfi::emit_ddr2_dfi_controller_verilog;
use super::ddr2_jedec::{check_bus, command_of_pins, BusCmd, BusFacts, Ddr2Grade, Ddr2Timing};
use super::ddr2_phy::{emit_ddr2_controller_verilog, Ddr2Cmd};
use std::collections::{BTreeSet, HashMap};

/// The K7 part.
pub const K7_GRADE: Ddr2Grade = Ddr2Grade::MT47H64M16_25;
/// The K7 DRAM clock period: DDR2-400.
pub const K7_TCK_PS: u64 = 5000;
/// The rows of every bank the scattered stream draws its words from.
pub const SCATTER_ROWS: u32 = 64;
/// The DRAM clocks between the probe's reads: more than any K7 controller needs to serve one read with a refresh in
/// its way, so each read meets the controller idle.
pub const PROBE_EVERY: u64 = 128;

/// The K7 part clocked at the K7 operating point, burst length `bl`.
pub fn k7_timing(bl: u64) -> Ddr2Timing {
    K7_GRADE.at(K7_TCK_PS, bl).expect("the K7 operating point is inside the grade's range")
}

/// `ddr2_ctrl` as the K7 bench builds, synthesises and drives it: the LOGOS DDR2 controller at the K7 part's table,
/// burst length 8.
pub fn k7_ddr2_ctrl_verilog() -> String {
    emit_ddr2_controller_verilog(&k7_timing(8).dram_timings())
}

/// `ddr2_dfi_ctrl` as the K7 bench builds, synthesises and drives it: the LOGOS DFI 1:2 controller at the K7 part's
/// rules, burst length 4.
pub fn k7_dfi_ctrl_verilog() -> String {
    emit_ddr2_dfi_controller_verilog(&k7_timing(4))
}

/// A controller the K7 traffic gate drives: its name in the K7 design table, the burst length its mode register
/// sets, and how its requests carry a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controller {
    pub name: &'static str,
    pub bl: u64,
    pub iface: Interface,
}

/// LOGOS's `ddr2_ctrl` (a 16-bit request port, BL8), LOGOS's `ddr2_dfi_ctrl` and LiteDRAM's controller (each a 64-bit
/// native port, BL4 at 1:2).
pub const K7_CONTROLLERS: [Controller; 3] = [
    Controller { name: "k7_logos_ddr2_ctrl", bl: 8, iface: Interface { parts: 4, cols_per_part: 1 } },
    Controller { name: "k7_logos_dfi_ctrl", bl: 4, iface: Interface { parts: 1, cols_per_part: 4 } },
    Controller { name: "k7_litedram_ctrl", bl: 4, iface: Interface { parts: 1, cols_per_part: 4 } },
];

pub fn k7_controller(name: &str) -> Option<Controller> {
    K7_CONTROLLERS.iter().copied().find(|c| c.name == name)
}

/// Words per row of the part: a 2 KB x16 row holds 256 words of 8 bytes.
const ROW_WORDS: u32 = 256;
const BANKS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// Every word from 0 up: whole rows, bank after bank.
    Sequential,
    /// Distinct words drawn at random from [`SCATTER_ROWS`] rows of every bank.
    Scattered,
    /// The scattered words written as a stream, then read one at a time, each read presented [`PROBE_EVERY`] DRAM
    /// clocks after the one before: the latency of a read that finds the controller idle, and how much it varies.
    Probe,
}

impl Pattern {
    pub fn label(self) -> &'static str {
        match self {
            Pattern::Sequential => "sequential",
            Pattern::Scattered => "scattered",
            Pattern::Probe => "probe",
        }
    }

    pub fn parse(s: &str) -> Option<Pattern> {
        [Pattern::Sequential, Pattern::Scattered, Pattern::Probe].into_iter().find(|p| p.label() == s)
    }
}

/// The stream: `words` words written in order, then read back in the same order, each read presented as soon as the
/// controller took the one before, or for the probe [`PROBE_EVERY`] clocks after it was presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traffic {
    pub pattern: Pattern,
    pub words: u32,
}

/// Where a word lives: its bank, its row, and the first of its four x16 columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub bank: u8,
    pub row: u16,
    pub col: u16,
}

/// The place of word `w`: ROW_BANK_COL, the column word in the low eight bits, the bank above, the row above that.
pub fn place(w: u32) -> Place {
    assert!(w < 1 << 24, "word {w} is past the part's 2^24 words");
    Place { bank: ((w / ROW_WORDS) % BANKS) as u8, row: (w / (ROW_WORDS * BANKS)) as u16, col: ((w % ROW_WORDS) * 4) as u16 }
}

/// The value written to word `w`: SplitMix64's output for `w`, a bijection, so every word's value is its own.
pub fn word_value(w: u32) -> u64 {
    let mut z = u64::from(w).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Traffic {
    /// The words in request order.
    pub fn order(&self) -> Vec<u32> {
        match self.pattern {
            Pattern::Sequential => (0..self.words).collect(),
            Pattern::Scattered | Pattern::Probe => {
                let region = SCATTER_ROWS * BANKS * ROW_WORDS;
                assert!(self.words <= region / 2, "{} words crowd a region of {region}", self.words);
                let mut s = 0x6A09_E667_F3BC_C909u64;
                let mut seen = BTreeSet::new();
                let mut out = Vec::with_capacity(self.words as usize);
                while out.len() < self.words as usize {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    let w = (s % u64::from(region)) as u32;
                    if seen.insert(w) {
                        out.push(w);
                    }
                }
                out
            }
        }
    }

    /// The DRAM clocks between read presentations: [`PROBE_EVERY`] for the probe, 0 (as soon as taken) otherwise.
    pub fn every(&self) -> u64 {
        if self.pattern == Pattern::Probe {
            PROBE_EVERY
        } else {
            0
        }
    }

    /// Every (bank, row) the stream touches.
    pub fn rows(&self) -> BTreeSet<(u8, u16)> {
        self.order().into_iter().map(|w| (place(w).bank, place(w).row)).collect()
    }

    /// The row-address bits a device must store for this stream: enough for its highest row.
    pub fn row_bits(&self) -> u32 {
        let top = self.rows().iter().map(|&(_, r)| u32::from(r)).max().unwrap_or(0);
        (u32::BITS - top.leading_zeros()).max(1)
    }

    /// The testbench's ROM: per word in order, its index (6 hex digits) and value (16).
    pub fn rom(&self) -> String {
        self.order().into_iter().map(|w| format!("{w:06x}{:016x}\n", word_value(w))).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Write,
    Read,
}

/// A request the requester presented or the controller accepted: the clock, write or read, the word, and which part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub cycle: u64,
    pub kind: Kind,
    pub word: u32,
    pub part: u8,
}

/// A word the controller delivered back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub cycle: u64,
    pub word: u32,
    pub value: u64,
}

/// A run's log: the bus, the requests as presented and as accepted, the deliveries, and the closing line (its clock
/// and the testbench's error count). Clocks are DRAM clocks from the end of reset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    pub bus: Vec<BusCmd>,
    pub presented: Vec<Request>,
    pub requests: Vec<Request>,
    pub deliveries: Vec<Delivery>,
    pub end: Option<(u64, u64)>,
}

/// Read a log: `C <clock> <cs#> <ras#> <cas#> <we#> <ba> <a>`, `P <clock> <W|R> <word> <part>` (presented),
/// `Q <clock> <W|R> <word> <part>` (accepted), `D <clock> <word> <hex value>`, `END <clock> <errors>`; blank lines
/// pass.
pub fn read_log(text: &str) -> Result<Log, String> {
    let mut log = Log::default();
    for (n, line) in text.lines().enumerate() {
        let at = |what: &str| format!("line {}: {what}: {line:?}", n + 1);
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some(&tag) = f.first() else { continue };
        let num = |i: usize| -> Result<u64, String> {
            f.get(i).and_then(|s| s.parse::<u64>().ok()).ok_or_else(|| at(&format!("field {i} is not a number")))
        };
        let arity = |k: usize| if f.len() == k { Ok(()) } else { Err(at(&format!("{} fields, {tag} has {k}", f.len()))) };
        match tag {
            "C" => {
                arity(8)?;
                let pin = |i: usize| num(i).and_then(|v| u8::try_from(v).map_err(|_| at("a pin level")));
                let cmd = command_of_pins(pin(2)?, pin(3)?, pin(4)?, pin(5)?).ok_or_else(|| at("pins that encode no command"))?;
                let ba = u8::try_from(num(6)?).map_err(|_| at("a bank address"))?;
                let a = u16::try_from(num(7)?).map_err(|_| at("an address"))?;
                log.bus.push(BusCmd { cycle: num(1)?, cmd, ba, a });
            }
            "P" | "Q" => {
                arity(5)?;
                let kind = match f[2] {
                    "W" => Kind::Write,
                    "R" => Kind::Read,
                    _ => return Err(at("a request is W or R")),
                };
                let word = u32::try_from(num(3)?).map_err(|_| at("a word index"))?;
                let part = u8::try_from(num(4)?).map_err(|_| at("a part"))?;
                let r = Request { cycle: num(1)?, kind, word, part };
                if tag == "P" {
                    log.presented.push(r);
                } else {
                    log.requests.push(r);
                }
            }
            "D" => {
                arity(4)?;
                let value = u64::from_str_radix(f[3], 16).map_err(|_| at("a value that is not hex"))?;
                let word = u32::try_from(num(2)?).map_err(|_| at("a word index"))?;
                log.deliveries.push(Delivery { cycle: num(1)?, word, value });
            }
            "END" => {
                arity(3)?;
                if log.end.is_some() {
                    return Err(at("a second END"));
                }
                log.end = Some((num(1)?, num(2)?));
            }
            _ => return Err(at("an unknown tag")),
        }
    }
    Ok(log)
}

/// How a controller's requests carry a word: in `parts` requests, part p at the word's first column plus
/// p × `cols_per_part`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    pub parts: u8,
    pub cols_per_part: u16,
}

/// A latency distribution in DRAM clocks: count, extremes, nearest-rank median and 99th percentile, mean.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub n: u64,
    pub min: u64,
    pub max: u64,
    pub p50: u64,
    pub p99: u64,
    pub mean: f64,
}

impl Stats {
    fn of(mut v: Vec<u64>) -> Stats {
        if v.is_empty() {
            return Stats { n: 0, min: 0, max: 0, p50: 0, p99: 0, mean: 0.0 };
        }
        v.sort_unstable();
        let n = v.len();
        let rank = |q: f64| v[((q * n as f64).ceil() as usize).clamp(1, n) - 1];
        Stats { n: n as u64, min: v[0], max: v[n - 1], p50: rank(0.5), p99: rank(0.99), mean: v.iter().sum::<u64>() as f64 / n as f64 }
    }
}

/// What a run measured. `stream_cycles` runs from the first accepted request to the last delivered word;
/// `payload_cycles` is what the stream needs of an x16 DDR bus (two clocks a word, each way); `bus_span` runs from
/// the first command to the last beat; `row_hits` counts column commands that found their row already open by an
/// earlier column command; the write and read latencies run from a request's acceptance to the column command that
/// served it; `read_service` from the presentation of a word's first read request to the word's delivery, and
/// `read_bus` to the last beat of the word's read data on the DRAM bus — the same span without any PHY's pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    pub words: u32,
    pub requests: u32,
    pub bus: BusFacts,
    pub bus_span: u64,
    pub stream_cycles: u64,
    pub payload_cycles: u64,
    pub row_hits: u64,
    pub write_latency: Stats,
    pub read_latency: Stats,
    pub read_service: Stats,
    pub read_bus: Stats,
}

/// Judge a run of `traffic` through a controller with request interface `iface`, its bus held to `t`.
pub fn judge(t: &Ddr2Timing, traffic: &Traffic, iface: Interface, log: &Log) -> Result<Judgement, String> {
    let Some((_, errors)) = log.end else {
        return Err("the testbench did not finish: the log has no END".into());
    };
    if errors != 0 {
        return Err(format!("the testbench counted {errors} errors"));
    }
    let order = traffic.order();
    let want: Vec<(Kind, u32, u8)> = [Kind::Write, Kind::Read]
        .into_iter()
        .flat_map(|k| order.iter().flat_map(move |&w| (0..iface.parts).map(move |p| (k, w, p))))
        .collect();
    for (seen, what) in [(&log.presented, "presented"), (&log.requests, "accepted")] {
        if seen.len() != want.len() {
            return Err(format!("{} requests {what}, the stream makes {}", seen.len(), want.len()));
        }
        for (i, (r, &(kind, word, part))) in seen.iter().zip(&want).enumerate() {
            if (r.kind, r.word, r.part) != (kind, word, part) {
                return Err(format!("request {i} {what} is {:?} word {} part {}, the stream's is {kind:?} word {word} part {part}", r.kind, r.word, r.part));
            }
        }
    }
    for (i, (p, q)) in log.presented.iter().zip(&log.requests).enumerate() {
        if q.cycle < p.cycle {
            return Err(format!("request {i} accepted at cycle {}, before it was presented at {}", q.cycle, p.cycle));
        }
    }
    let facts = check_bus(t, &log.bus).map_err(|v| format!("the bus breaks JEDEC DDR2: {v}"))?;
    let rows = traffic.rows();
    for c in log.bus.iter().filter(|c| c.cmd == Ddr2Cmd::Active) {
        if !rows.contains(&(c.ba, c.a)) {
            return Err(format!("an ACT at cycle {} to bank {} row {}, outside the stream", c.cycle, c.ba, c.a));
        }
    }

    let mut cas: HashMap<(bool, u8, u16, u16), Vec<(u64, u64, bool)>> = HashMap::new();
    for e in &facts.cas {
        cas.entry((e.write, e.bank, e.row, e.col)).or_default().push((e.cycle, e.data_at, false));
    }
    let (mut wl, mut rl) = (Vec::new(), Vec::new());
    let mut read_end: HashMap<u32, u64> = HashMap::new();
    for r in &log.requests {
        let p = place(r.word);
        let key = (r.kind == Kind::Write, p.bank, p.row, p.col + u16::from(r.part) * iface.cols_per_part);
        let served = cas.get_mut(&key).and_then(|v| v.iter_mut().find(|(c, _, used)| !*used && *c >= r.cycle));
        let Some((at, data_at, used)) = served else {
            return Err(format!("{:?} of word {} part {} accepted at cycle {} never reached the bus", r.kind, r.word, r.part, r.cycle));
        };
        *used = true;
        match r.kind {
            Kind::Write => wl.push(*at - r.cycle),
            Kind::Read => {
                rl.push(*at - r.cycle);
                let end = read_end.entry(r.word).or_insert(0);
                *end = (*end).max(*data_at + t.bl / 2);
            }
        }
    }

    if log.deliveries.len() != order.len() {
        return Err(format!("{} words delivered, the stream reads {}", log.deliveries.len(), order.len()));
    }
    let reads_from = order.len() * usize::from(iface.parts);
    let mut service = Vec::with_capacity(order.len());
    let mut bus = Vec::with_capacity(order.len());
    for (i, (d, &w)) in log.deliveries.iter().zip(&order).enumerate() {
        if d.word != w {
            return Err(format!("delivery {i} is word {}, the stream's next is word {w}", d.word));
        }
        if d.value != word_value(w) {
            return Err(format!("word {w} read back {:016x}, the stream wrote {:016x}", d.value, word_value(w)));
        }
        let asked = log.presented[reads_from + i * usize::from(iface.parts)].cycle;
        if d.cycle < asked {
            return Err(format!("word {w} delivered at cycle {}, before its read was presented at {asked}", d.cycle));
        }
        service.push(d.cycle - asked);
        bus.push(read_end[&w] - asked);
    }

    let mut hit = [false; 8];
    let mut row_hits = 0;
    for c in &log.bus {
        match c.cmd {
            Ddr2Cmd::Active => hit[usize::from(c.ba)] = false,
            Ddr2Cmd::Read | Ddr2Cmd::Write => {
                if std::mem::replace(&mut hit[usize::from(c.ba)], true) {
                    row_hits += 1;
                }
            }
            _ => {}
        }
    }
    let first = log.requests.first().map_or(0, |r| r.cycle);
    let last = log.deliveries.last().map_or(0, |d| d.cycle);
    let word_clocks = 64 / (2 * u64::from(t.grade.dq_bits));
    Ok(Judgement {
        words: traffic.words,
        requests: log.requests.len() as u32,
        bus_span: facts.end - facts.first_cycle,
        bus: facts,
        stream_cycles: last - first,
        payload_cycles: 2 * u64::from(traffic.words) * word_clocks,
        row_hits,
        write_latency: Stats::of(wl),
        read_latency: Stats::of(rl),
        read_service: Stats::of(service),
        read_bus: Stats::of(bus),
    })
}

/// `ddr2_device`: the part as a testbench sees it through an ideal PHY. It keeps each bank's open row and stores
/// 16-bit columns for `ROWB` row-address bits; a WRITE takes its burst (with a per-beat mask, 1 = not written) on
/// the command's clock, a READ returns its burst CL clocks later, both in the sequential burst order that wraps
/// within the burst's aligned block. An ACT past the stored rows, or a column command to a bank with no open row,
/// raises `model_error`. Bus timing is not its business: the log's JEDEC checker holds the bus to the rules.
pub fn emit_ddr2_device_verilog(t: &Ddr2Timing) -> String {
    r#"// ddr2_device — the DDR2 part through an ideal PHY: each bank's open row, 16-bit columns for ROWB row bits,
// a WRITE burst taken with its command (wr_mask 1 = beat not written), a READ burst returned CL clocks later,
// sequential burst order wrapping within the aligned block.
module ddr2_device #(parameter ROWB = 1) (
  input clk, input rst,
  input cke, input cs_n, input ras_n, input cas_n, input we_n,
  input [2:0] ba, input [12:0] a,
  input [%BL%*16-1:0] wr_burst, input [%BL%-1:0] wr_mask,
  output reg [%BL%*16-1:0] rd_burst, output reg rd_valid,
  output reg model_error
);
  localparam BL = %BL%, CL = %CL%;
  reg [15:0] mem [0:(8 << (ROWB + 10)) - 1];
  reg openb [0:7];
  reg [12:0] open_row [0:7];
  reg [BL*16-1:0] pipe [1:CL];
  reg pv [1:CL];
  reg [BL*16-1:0] burst;
  integer i, k;
  function [ROWB+12:0] slot(input [2:0] b, input [12:0] r, input [9:0] c);
    slot = {b, r[ROWB-1:0], c};
  endfunction
  function [9:0] beat(input [9:0] c, input [3:0] n);
    beat = (c & ~(BL - 1)) | ((c + n) & (BL - 1));
  endfunction
  always @(posedge clk) begin
    if (rst) begin
      model_error <= 1'b0; rd_valid <= 1'b0; rd_burst <= 0;
      for (k = 0; k < 8; k = k + 1) begin openb[k] <= 1'b0; open_row[k] <= 13'd0; end
      for (k = 1; k <= CL; k = k + 1) begin pv[k] <= 1'b0; pipe[k] <= 0; end
    end else begin
      rd_valid <= pv[CL];
      rd_burst <= pipe[CL];
      for (k = CL; k > 1; k = k - 1) begin pv[k] <= pv[k - 1]; pipe[k] <= pipe[k - 1]; end
      pv[1] <= 1'b0;
      if (cke && !cs_n) case ({ras_n, cas_n, we_n})
        3'b011: begin
          if (a >= (13'd1 << ROWB)) begin model_error <= 1'b1; $display("FAIL: ACT to row %0d past the %0d stored rows", a, 1 << ROWB); end
          openb[ba] <= 1'b1; open_row[ba] <= a;
        end
        3'b100: begin
          if (!openb[ba]) begin model_error <= 1'b1; $display("FAIL: WRITE to bank %0d with no open row", ba); end
          for (i = 0; i < BL; i = i + 1)
            if (!wr_mask[i]) mem[slot(ba, open_row[ba], beat(a[9:0], i))] <= wr_burst[16 * i +: 16];
        end
        3'b101: begin
          if (!openb[ba]) begin model_error <= 1'b1; $display("FAIL: READ from bank %0d with no open row", ba); end
          for (i = 0; i < BL; i = i + 1) burst[16 * i +: 16] = mem[slot(ba, open_row[ba], beat(a[9:0], i))];
          pv[1] <= 1'b1; pipe[1] <= burst;
        end
        3'b010: if (a[10]) begin for (k = 0; k < 8; k = k + 1) openb[k] <= 1'b0; end else openb[ba] <= 1'b0;
        default: ;
      endcase
    end
  end
endmodule
"#
    .replace("%BL%", &t.bl.to_string())
    .replace("%CL%", &t.cl.to_string())
}

/// The clocks a testbench waits for a stream before it gives up: far past any controller's need.
fn limit(traffic: &Traffic) -> u64 {
    u64::from(traffic.words) * (8 * 64 + traffic.every()) + 100_000
}

/// The half period of the testbench clock, in ns, for a DRAM clock of `tck_ps` driven 1:1.
fn half_ns(tck_ps: u64) -> String {
    format!("{}", tck_ps as f64 / 2000.0)
}

/// `tb_qor_dram` for `ddr2_ctrl`: the stream through its 16-bit request port, four requests a word (part p at
/// column 4 × the column word + p), the device behind it at burst length `t.bl` with every beat but the addressed
/// one masked. A request is presented as soon as the one before was taken; for the probe, a word's first read waits
/// until [`Traffic::every`] clocks after the word before's, and the reads start two of those after the last write.
/// It reads `words.hex` ([`Traffic::rom`]) and writes `dram.log` ([`read_log`]).
pub fn emit_ddr2_ctrl_traffic_testbench(t: &Ddr2Timing, traffic: &Traffic) -> String {
    r#"`timescale 1ns/1ps
// tb_qor_dram — the K7 word stream through ddr2_ctrl's 16-bit request port into ddr2_device; logs the bus, the
// presented and accepted requests and the delivered words to dram.log.
module tb_qor_dram;
  localparam N = %N%, BL = %BL%, REQS = 8 * N;
  localparam [63:0] LIMIT = 64'd%LIMIT%, EVERY = 64'd%EVERY%;
  reg clk = 1'b0;
  always #%HALF% clk = ~clk;
  reg rst = 1'b1;
  reg [87:0] rom [0:N-1];
  integer f, errors;
  reg [63:0] cyc, next_at;
  reg [31:0] r, got;
  reg [47:0] acc;
  reg shown;
  wire req_ready, resp_valid, cke, cs_n, ras_n, cas_n, we_n, odt, wr_en, rd_valid, model_error;
  wire [15:0] resp_rdata, wr_data;
  wire [2:0] ba;
  wire [12:0] a;
  wire [BL*16-1:0] rd_burst;
  wire [31:0] word_i = (r % (4 * N)) / 4;
  wire [1:0] part = r[1:0];
  wire [87:0] entry = rom[word_i];
  wire req_we = r < 4 * N;
  wire due = req_we || part != 2'd0 || EVERY == 0 || cyc >= next_at;
  wire req_valid = r < REQS && due;
  wire [25:0] req_addr = {entry[87:75], entry[74:72], entry[71:64], part};
  wire [15:0] req_wdata = entry[16 * part +: 16];
  wire [87:0] back = rom[got / 4];
  ddr2_ctrl dut(.clk(clk), .rst(rst), .req_valid(req_valid), .req_we(req_we), .req_addr(req_addr),
    .req_wdata(req_wdata), .req_ready(req_ready), .resp_valid(resp_valid), .resp_rdata(resp_rdata),
    .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .ba(ba), .a(a), .odt(odt),
    .wr_data(wr_data), .wr_en(wr_en), .rd_data(rd_burst[15:0]), .rd_valid(rd_valid));
  ddr2_device #(.ROWB(%ROWB%)) mem(.clk(clk), .rst(rst), .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n),
    .we_n(we_n), .ba(ba), .a(a), .wr_burst({{((BL - 1) * 16){1'b0}}, wr_data}), .wr_mask({{(BL - 1){1'b1}}, 1'b0}),
    .rd_burst(rd_burst), .rd_valid(rd_valid), .model_error(model_error));
  always @(posedge clk) begin
    if (rst) begin
      cyc <= 64'd0; r <= 32'd0; got <= 32'd0; acc <= 48'd0; shown <= 1'b0; next_at <= 64'd0;
    end else begin
      cyc <= cyc + 64'd1;
      if (!cs_n && !(ras_n && cas_n && we_n))
        $fwrite(f, "C %0d %0d %0d %0d %0d %0d %0d\n", cyc, cs_n, ras_n, cas_n, we_n, ba, a);
      if (req_valid && !shown)
        $fwrite(f, "P %0d %c %0d %0d\n", cyc, req_we ? 8'd87 : 8'd82, entry[87:64], part);
      if (req_valid && req_ready) begin
        $fwrite(f, "Q %0d %c %0d %0d\n", cyc, req_we ? 8'd87 : 8'd82, entry[87:64], part);
        r <= r + 32'd1;
        shown <= 1'b0;
        if (r == 4 * N - 1) next_at <= cyc + 2 * EVERY;
        else if (!req_we && part == 2'd0) next_at <= next_at + EVERY;
      end else if (req_valid) shown <= 1'b1;
      if (resp_valid) begin
        if (got[1:0] == 2'd3) begin
          $fwrite(f, "D %0d %0d %h\n", cyc, back[87:64], {resp_rdata, acc});
          if ({resp_rdata, acc} !== back[63:0]) begin
            errors = errors + 1;
            $display("FAIL: word %0d read back %h, wrote %h", back[87:64], {resp_rdata, acc}, back[63:0]);
          end
        end else acc[16 * got[1:0] +: 16] <= resp_rdata;
        got <= got + 32'd1;
      end
    end
  end
  initial begin
    $readmemh("words.hex", rom);
    f = $fopen("dram.log", "w");
    errors = 0;
    repeat (4) @(posedge clk);
    rst <= 1'b0;
    wait (got == 4 * N || cyc >= LIMIT);
    repeat (32) @(posedge clk);
    if (got != 4 * N) begin errors = errors + 1; $display("FAIL: %0d of %0d parts delivered by cycle %0d", got, 4 * N, cyc); end
    if (model_error) errors = errors + 1;
    $fwrite(f, "END %0d %0d\n", cyc, errors);
    $fclose(f);
    if (errors == 0) $display("PASS: %0d words through ddr2_ctrl in %0d cycles", N, cyc);
    else $display("FAIL: %0d errors", errors);
    $finish;
  end
endmodule
"#
    .replace("%N%", &traffic.words.to_string())
    .replace("%BL%", &t.bl.to_string())
    .replace("%LIMIT%", &limit(traffic).to_string())
    .replace("%EVERY%", &traffic.every().to_string())
    .replace("%HALF%", &half_ns(t.tck_ps))
    .replace("%ROWB%", &traffic.row_bits().to_string())
}

/// One DFI phase's signals as LiteDRAM's controller and its SDRAMPHYModel both name them, with their widths at the
/// K7 configuration (13 address bits, 8 banks, a 32-bit phase of the x16 part at 1:2). `act_n`, DDR4's alone, is tied
/// off on both sides.
const DFI: [(&str, u32); 15] = [
    ("address", 13),
    ("bank", 3),
    ("cas_n", 1),
    ("cs_n", 1),
    ("ras_n", 1),
    ("we_n", 1),
    ("cke", 1),
    ("odt", 1),
    ("reset_n", 1),
    ("wrdata", 32),
    ("wrdata_en", 1),
    ("wrdata_mask", 4),
    ("rddata_en", 1),
    ("rddata", 32),
    ("rddata_valid", 1),
];

/// The DFI of both phases as wires, and as the port connections that put them between controller and model.
fn dfi_wiring() -> (String, String) {
    let mut wires = String::new();
    let mut ports = Vec::new();
    for p in 0..2 {
        for (sig, bits) in DFI {
            let range = if bits > 1 { format!("[{}:0] ", bits - 1) } else { String::new() };
            wires.push_str(&format!("  wire {range}dfi_p{p}_{sig};\n"));
            ports.push(format!(".dfi_p{p}_{sig}(dfi_p{p}_{sig})"));
        }
        ports.push(format!(".dfi_p{p}_act_n(1'b1)"));
    }
    (wires, ports.join(", "))
}

/// `tb_qor_dram` for LiteDRAM: the stream through the native port of `litedram_ctrl`, the controller exactly as the
/// bench synthesises it, its DFI wired to `litedram_phy_model`, LiteDRAM's own SDRAMPHYModel (both from
/// `litedram_controller.py`); one command a word, write data offered for every accepted write in order, read data
/// always accepted, reads presented as for `ddr2_ctrl`. The system clock runs at half the DRAM clock, and the log
/// counts DRAM clocks: DFI phase p of system clock k is DRAM clock 2k + p, a request presented or accepted at system
/// clock k is logged at 2k. It reads `words.hex` and writes `dram.log`.
pub fn emit_litedram_traffic_testbench(t: &Ddr2Timing, traffic: &Traffic) -> String {
    let (_, ports) = dfi_wiring();
    let instance = format!(
        "  litedram_ctrl ctrl(.sys_clk(clk), .sys_rst(rst),
    .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_first(1'b0), .cmd_last(1'b0), .cmd_payload_we(cmd_we),
    .cmd_payload_addr(centry[87:64]),
    .wdata_valid(wdata_valid), .wdata_ready(wdata_ready), .wdata_first(1'b0), .wdata_last(1'b0),
    .wdata_payload_data(wentry[63:0]), .wdata_payload_we(8'hff),
    .rdata_valid(rdata_valid), .rdata_ready(1'b1), .rdata_first(1'b0), .rdata_last(1'b0), .rdata_payload_data(rdata),
    {ports});"
    );
    dfi_traffic_testbench(t, traffic, "LiteDRAM's native port: litedram_ctrl as the bench synthesises it", "LiteDRAM", &instance)
}

/// `tb_qor_dram` for `ddr2_dfi_ctrl`: LiteDRAM's testbench with the LOGOS controller in `litedram_ctrl`'s place, on
/// the same native port and DFI, against the same SDRAMPHYModel; it has no stream framing (first/last) and no read
/// back-pressure, which LiteDRAM's crossbar does not read either.
pub fn emit_ddr2_dfi_ctrl_traffic_testbench(t: &Ddr2Timing, traffic: &Traffic) -> String {
    let (_, ports) = dfi_wiring();
    let dfi: Vec<&str> = ports.split(", ").filter(|p| !p.contains("_act_n(")).collect();
    let instance = format!(
        "  ddr2_dfi_ctrl ctrl(.sys_clk(clk), .sys_rst(rst),
    .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_payload_we(cmd_we), .cmd_payload_addr(centry[87:64]),
    .wdata_valid(wdata_valid), .wdata_ready(wdata_ready), .wdata_payload_data(wentry[63:0]), .wdata_payload_we(8'hff),
    .rdata_valid(rdata_valid), .rdata_payload_data(rdata),
    {});",
        dfi.join(", ")
    );
    dfi_traffic_testbench(t, traffic, "ddr2_dfi_ctrl's native port", "ddr2_dfi_ctrl", &instance)
}

/// The DFI traffic testbench around a controller `instance` on the native port (`what` names it in the header,
/// `who` in the verdict).
fn dfi_traffic_testbench(t: &Ddr2Timing, traffic: &Traffic, what: &str, who: &str, instance: &str) -> String {
    let (wires, ports) = dfi_wiring();
    r#"`timescale 1ns/1ps
// tb_qor_dram — the K7 word stream through %WHAT%, its
// DFI wired to LiteDRAM's SDRAMPHYModel; logs the DFI commands, the presented and accepted requests and the delivered
// words to dram.log in DRAM clocks (phase p of system clock k is 2k + p).
module tb_qor_dram;
  localparam N = %N%;
  localparam [63:0] LIMIT = 64'd%LIMIT%, EVERY = 64'd%EVERY%;
  reg clk = 1'b0;
  always #%HALF% clk = ~clk;
  reg rst = 1'b1;
  reg [87:0] rom [0:N-1];
  integer f, errors;
  reg [63:0] k, next_at;
  reg [31:0] r, wsent, got;
  reg shown;
  wire cmd_ready, wdata_ready, rdata_valid;
  wire [63:0] rdata;
  wire [87:0] centry = rom[r % N];
  wire [87:0] wentry = rom[wsent];
  wire [87:0] back = rom[got];
  wire cmd_we = r < N;
  wire cmd_valid = r < 2 * N && (cmd_we || EVERY == 0 || 2 * k >= next_at);
  wire wdata_valid = wsent < (r < N ? r : N);
%WIRES%%INSTANCE%
  litedram_phy_model part(.sys_clk(clk), .sys_rst(rst),
    %PORTS%);
  always @(posedge clk) begin
    if (rst) begin
      k <= 64'd0; r <= 32'd0; wsent <= 32'd0; got <= 32'd0; shown <= 1'b0; next_at <= 64'd0;
    end else begin
      k <= k + 64'd1;
      if (!dfi_p0_cs_n && !(dfi_p0_ras_n && dfi_p0_cas_n && dfi_p0_we_n))
        $fwrite(f, "C %0d %0d %0d %0d %0d %0d %0d\n", 2 * k, dfi_p0_cs_n, dfi_p0_ras_n, dfi_p0_cas_n, dfi_p0_we_n, dfi_p0_bank, dfi_p0_address);
      if (!dfi_p1_cs_n && !(dfi_p1_ras_n && dfi_p1_cas_n && dfi_p1_we_n))
        $fwrite(f, "C %0d %0d %0d %0d %0d %0d %0d\n", 2 * k + 1, dfi_p1_cs_n, dfi_p1_ras_n, dfi_p1_cas_n, dfi_p1_we_n, dfi_p1_bank, dfi_p1_address);
      if (cmd_valid && !shown) $fwrite(f, "P %0d %c %0d 0\n", 2 * k, cmd_we ? 8'd87 : 8'd82, centry[87:64]);
      if (cmd_valid && cmd_ready) begin
        $fwrite(f, "Q %0d %c %0d 0\n", 2 * k, cmd_we ? 8'd87 : 8'd82, centry[87:64]);
        r <= r + 32'd1;
        shown <= 1'b0;
        if (r == N - 1) next_at <= 2 * k + 2 * EVERY;
        else if (!cmd_we) next_at <= next_at + EVERY;
      end else if (cmd_valid) shown <= 1'b1;
      if (wdata_valid && wdata_ready) wsent <= wsent + 32'd1;
      if (rdata_valid) begin
        $fwrite(f, "D %0d %0d %h\n", 2 * k, back[87:64], rdata);
        if (rdata !== back[63:0]) begin
          errors = errors + 1;
          $display("FAIL: word %0d read back %h, wrote %h", back[87:64], rdata, back[63:0]);
        end
        got <= got + 32'd1;
      end
    end
  end
  initial begin
    $readmemh("words.hex", rom);
    f = $fopen("dram.log", "w");
    errors = 0;
    repeat (4) @(posedge clk);
    rst <= 1'b0;
    wait (got == N || k >= LIMIT);
    repeat (32) @(posedge clk);
    if (got != N) begin errors = errors + 1; $display("FAIL: %0d of %0d words delivered by system clock %0d", got, N, k); end
    $fwrite(f, "END %0d %0d\n", 2 * k, errors);
    $fclose(f);
    if (errors == 0) $display("PASS: %0d words through %WHO% in %0d system clocks", N, k);
    else $display("FAIL: %0d errors", errors);
    $finish;
  end
endmodule
"#
    .replace("%WHAT%", what)
    .replace("%WHO%", who)
    .replace("%INSTANCE%", instance)
    .replace("%N%", &traffic.words.to_string())
    .replace("%LIMIT%", &limit(traffic).to_string())
    .replace("%EVERY%", &traffic.every().to_string())
    .replace("%HALF%", &half_ns(2 * t.tck_ps))
    .replace("%WIRES%", &wires)
    .replace("%PORTS%", &ports)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/ddr2_traffic/tests.rs"]
mod tests;
