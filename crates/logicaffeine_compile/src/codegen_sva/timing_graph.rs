//! A circuit's timing graph: how deep its logic is, where the depth sits, and the bound its loops put on the clock,
//! known before Quartus runs.
//!
//! [`script`] elaborates a module through yosys the way an FPGA flow sees it, flattened with every cell keeping the
//! instance path it came from: adders, subtractors, negations, comparisons and dividers stay whole, as the carry
//! chains the fabric builds them from; multipliers stay whole, as DSP blocks; a memory read through a clock stays
//! whole, as a block RAM (its read register found once each flip-flop's enable is its own, so a read held under an
//! enable is the RAM's; a value cleared to zero stays logic before its register, not a reset pin), and one read
//! without a clock becomes logic; a memory written through more than the two
//! ports a block RAM has (an array cleared by a loop) becomes registers before the memory passes, which compare its
//! ports pair by pair, once the writes to one address (a word's byte lanes) are one port; everything else becomes
//! yosys's simple gates. An
//! undefined bit of the source is 0, fixed once the memories are collected and before anything is optimized, so the
//! graph is one circuit the source allows and what [`super::autopipe::emit`] writes from it is exactly that circuit.
//!
//! [`read`] maps the gates onto six-input LUTs ([`super::lutmap`], wide AND, OR and XOR trees balanced for LUTs) and
//! turns the netlist into a graph over net bits whose edges carry a delay and a register count, the retiming graph of
//! Leiserson & Saxe:
//! - a LUT adds [`Delays::lut`] from each input to its output, and so does a `$lut` cell a netlist brings mapped;
//! - a carry chain enters each bit through a LUT, [`Delays::lut`], and carries [`Delays::carry`] from bit to bit; a
//!   comparison is the chain's carry out;
//! - a divider is the array of subtractions that builds it, one a quotient bit from the top: each stage a LUT and a
//!   carry across the divisor, the quotient bit out of its stage and the remainder out of the last;
//! - a multiplier is a DSP block, [`Delays::dsp`] from its operands to its product, unless one operand is a
//!   constant power of two, which is wiring;
//! - a memory holds its contents one register past its write port; a read port with a clock holds one more and
//!   adds [`Delays::memory`] to its data, and one without adds [`Delays::memory`] from its address;
//! - a flip-flop is an edge from each of its inputs to its output holding one register.
//!
//! [`TimingGraph::depth`] times the combinational edges: the deepest path, and for every instance the deepest path
//! through its own cells and through its subtree, the two figures Quartus prints as an entity's Max Depth.
//! [`TimingGraph::endpoints`] is the deepest path into each register and each output, so a pipeline's stages are
//! each as deep as the paths into the registers that end them.
//! [`TimingGraph::loop_bound`] is the maximum cycle ratio ([`logicaffeine_proof::cycle_ratio`]): the clock no
//! retiming of the design can beat, with the loop that sets it, certified.
//!
//! The model is one clock: every flip-flop counts as a register of the same clock.
//!
//! A module [`script`] is given as a box, or one the source declares `(* blackbox *)`, is timed apart: emptied before
//! the hierarchy is elaborated, nothing under it is ever read, and each instance is a [`Boxed`], every bit it reads
//! the end of the paths into it ([`End::Box`]), as an output is, and every bit it drives the start of the paths out of
//! it, as an input is. A shell is so timed around the engine it holds, in seconds; a loop through the box is the box's
//! to time, and [`super::autopipe`] cuts nothing around one.

use std::collections::{BTreeMap, HashMap, HashSet};

use logicaffeine_proof::cycle_ratio::{self, Bound, Edge, GraphError};
use serde::{Deserialize, Serialize};

use super::lutmap::{self, Lit, Xag};
use super::netlist::{yosys_run, NetlistError};

/// The delay each kind of cell adds, in a common integer unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delays {
    pub lut: i64,
    pub carry: i64,
    pub dsp: i64,
    pub memory: i64,
}

impl Delays {
    /// Logic levels in hundredths: a LUT is one level.
    pub const LEVELS: Delays = Delays { lut: 100, carry: 0, dsp: 100, memory: 100 };

    /// The D5005's Stratix 10 as fit27's worst setup paths measured it (slow 900 mV, 100 °C), in hundredths of a LUT
    /// level with the wire into the next ([`STRATIX10_NS_PER_LEVEL`]: a LUT's 0.139 ns and a 0.41 ns hop): a carry bit
    /// 0.039 ns, a DSP block 2.10 ns and a RAM read 1.40 ns, each with its hop out.
    pub const STRATIX10: Delays = Delays { lut: 100, carry: 7, dsp: 457, memory: 330 };
}

/// The nanoseconds a level of [`Delays::STRATIX10`] takes: a LUT and the interconnect hop into the next.
pub const STRATIX10_NS_PER_LEVEL: f64 = 0.549;

/// What an edge or a path adds, counted by kind: its delay at any [`Delays`] is the sum of each count times its delay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    pub lut: i64,
    pub carry: i64,
    pub dsp: i64,
    pub memory: i64,
}

impl Cost {
    pub const NONE: Cost = Cost { lut: 0, carry: 0, dsp: 0, memory: 0 };
    pub const LUT: Cost = Cost { lut: 1, ..Cost::NONE };
    pub const CARRY: Cost = Cost { carry: 1, ..Cost::NONE };
    pub const DSP: Cost = Cost { dsp: 1, ..Cost::NONE };
    pub const MEMORY: Cost = Cost { memory: 1, ..Cost::NONE };

    pub fn delay(&self, d: &Delays) -> i64 {
        self.lut * d.lut + self.carry * d.carry + self.dsp * d.dsp + self.memory * d.memory
    }
}

impl std::ops::Add for Cost {
    type Output = Cost;
    fn add(self, o: Cost) -> Cost {
        Cost { lut: self.lut + o.lut, carry: self.carry + o.carry, dsp: self.dsp + o.dsp, memory: self.memory + o.memory }
    }
}

/// A timing graph: a node per net bit and per internal point of a carry chain, DSP block or memory.
#[derive(Clone, Debug, Default)]
pub struct TimingGraph {
    /// Each node's name: the net's name and bit, or the cell's and its point.
    pub nodes: Vec<String>,
    pub edges: Vec<Edge>,
    /// What each edge adds, by kind, for a graph read from a netlist ([`TimingGraph::reweigh`]); empty for one built
    /// by hand.
    pub costs: Vec<Cost>,
    /// The instance whose cell made each edge, an index into `instances`.
    pub owner: Vec<usize>,
    /// Instance paths in the form Quartus prints them, `|`-separated; the top is `""`.
    pub instances: Vec<String>,
    /// Flip-flop bits.
    pub flops: usize,
    /// What each instance's own cells take, by instance ([`TimingGraph::area_under`] sums a subtree).
    pub area: Vec<Area>,
    /// Whether each node is a bit of a net the design names (a signal of its source, not one synthesis made).
    pub public: Vec<bool>,
    /// The module's input and output port bits, ports by name and each one's bits in order.
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    /// How each node is made, when the graph was read from a netlist: what writing it back out as Verilog needs.
    pub structure: Option<Structure>,
    /// The instances of modules timed apart.
    pub boxes: Vec<Boxed>,
    /// The loads each net drives and what drives it, for a graph read from a netlist; none for one built by hand.
    pub fanout: Option<Fanout>,
}

/// What drives a node of a graph read from a netlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Driver {
    /// A flip-flop's output.
    Register,
    /// A LUT, a kept cell (carry chain, DSP block) or a constant.
    Logic,
    /// An input port of the module, or a net nothing in it drives.
    Input,
    /// A memory's read data.
    Memory,
    /// An output of a module timed apart.
    Box,
    /// A point inside a cell, no net.
    Point,
}

/// The loads of a graph read from a netlist: what each edge loads the node it starts from with — a cell input one, a
/// memory port the blocks of its RAM it reaches ([`memory_blocks`]) — each node's driver, the most loads its net asks
/// a copy of its driver to take (`(* maxfan = N *)`, as Quartus duplicates a register to keep each copy to N), and
/// which nodes are clocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fanout {
    pub loads: Vec<u64>,
    pub drivers: Vec<Driver>,
    pub limits: Vec<Option<u64>>,
    /// Whether each node is a clock: a flip-flop's clock pin, a memory's clock ports or a box's clock input (a port its
    /// module names `clk` or `…clk`) takes it.
    pub clocks: Vec<bool>,
}

/// A net past its bound: its node and name, the loads it drives and what drives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wide {
    pub node: usize,
    pub name: String,
    pub loads: u64,
    pub driver: Driver,
}

/// The blocks a memory of `width` bits by `size` words takes, across its width and down its depth, in the RAM its
/// `ramstyle` asks for: an MLAB holds 20 bits by 32 words, an M20K 40 bits by 512, and a memory that asks for registers
/// (`logic`) is a register a bit, read through four-word multiplexers; one that asks for nothing is an M20K, as Quartus
/// builds it.
pub fn memory_blocks(width: u64, size: u64, style: &str) -> (u64, u64) {
    let style = style.to_ascii_lowercase();
    let (bits, words) = if style.contains("mlab") {
        (20, 32)
    } else if style.contains("logic") || style.contains("register") {
        (1, 4)
    } else {
        (40, 512)
    };
    (width.div_ceil(bits).max(1), size.div_ceil(words).max(1))
}

/// What cells take as the timing graph maps them: six-input LUTs, carry-chain bits (a divider's every stage a chain as
/// wide as its divisor), DSP blocks in halves ([`dsp_halves`]), flip-flop bits and memory bits. A black box takes
/// nothing here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Area {
    pub luts: u64,
    pub carry: u64,
    pub half_dsps: u64,
    pub flops: u64,
    pub memory_bits: u64,
}

impl Area {
    fn add(&mut self, other: &Area) {
        self.luts += other.luts;
        self.carry += other.carry;
        self.half_dsps += other.half_dsps;
        self.flops += other.flops;
        self.memory_bits += other.memory_bits;
    }
}

/// The halves of a Stratix 10 DSP block an `a`×`b` product takes: two products of at most 18×19 share a block, one of
/// at most 27×27 fills it, and a wider one is tiled in 27×27 blocks.
pub fn dsp_halves(a: usize, b: usize) -> u64 {
    let (short, long) = (a.min(b), a.max(b));
    if short <= 18 && long <= 19 {
        1
    } else if long <= 27 {
        2
    } else {
        2 * (a.div_ceil(27) * b.div_ceil(27)) as u64
    }
}

/// An instance of a module timed apart, a black box: each bit it reads a node that ends the paths into it, as an
/// output does, and each bit it drives a node that starts the paths out of it, as an input does. None of its own logic
/// is in the graph, nor any loop through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boxed {
    /// The instance's path.
    pub cell: String,
    /// The module it is an instance of.
    pub module: String,
    /// What each of its outputs adds as it leaves it: nothing for outputs that are registers, a block RAM's read for
    /// a memory.
    pub out: Cost,
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
}

/// How the value at a node is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Bit `bit` of input port `port`.
    Input { port: String, bit: usize },
    /// A LUT: bit `i` of `table` is its value when pin `j` holds bit `j` of `i`; `cone`, when the mapping made it,
    /// the AND and XOR gates it covers.
    Lut { pins: Vec<Pin>, table: u64, cone: Option<Cone> },
    /// Another node's value, or its complement.
    Copy { from: usize, inverted: bool },
    /// Bit `bit` of output `port` of kept cell `cell`.
    Cell { cell: usize, port: String, bit: usize },
    Constant(bool),
    /// A flip-flop's output: what `data` held a clock before, and whether an enable, a reset or a set steps it besides.
    Register { data: Pin, controls: bool },
    /// A point inside a kept cell, a memory's output, or a net nothing here drives.
    Inside,
}

/// The gates a mapped LUT covers, over its pins: each gate's operands an earlier gate or a pin, plain or inverted,
/// and the LUT's output one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cone {
    pub gates: Vec<ConeGate>,
    pub out: ConeArg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConeGate {
    pub xor: bool,
    pub a: ConeArg,
    pub b: ConeArg,
}

/// A cone operand: pin `index`, or gate `index − pins` past the pins; inverted or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConeArg {
    pub index: usize,
    pub inverted: bool,
}

/// A pin's signal: a node, or a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    Node(usize),
    Zero,
    One,
}

/// A cell kept whole (a carry chain, a divider, a DSP product): its kind, its parameters, each port's bits, and a
/// node inside it whose stage is the cell's.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptCell {
    pub name: String,
    pub kind: String,
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub ports: BTreeMap<String, Vec<Pin>>,
    pub inside: usize,
}

impl KeptCell {
    /// A parameter yosys writes as a binary string or a number.
    pub fn number(&self, name: &str) -> u64 {
        number(self.parameters.get(name))
    }
}

/// What a graph was read from, node by node: every node's source, the kept cells, and the ports by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Structure {
    pub source: Vec<Source>,
    pub cells: Vec<KeptCell>,
    pub input_ports: Vec<(String, Vec<Pin>)>,
    pub output_ports: Vec<(String, Vec<Pin>)>,
}

/// How deep a graph's logic is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Depth {
    /// The deepest combinational path.
    pub depth: i64,
    /// Its nodes, from where it starts to where it ends.
    pub path: Vec<usize>,
    /// Its edges, in order: the one from each node of the path to the next.
    pub edges: Vec<usize>,
    /// Every instance with a cell in its subtree, in path order. A cell counts for an instance when it adds delay.
    pub instances: Vec<InstanceDepth>,
}

/// The deepest path through an instance's cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceDepth {
    pub path: String,
    /// Through any cell of its subtree.
    pub total: i64,
    /// Through its own cells; 0 when it has none.
    pub own: i64,
}

/// What ends a path: a register, which one clock must reach, an output of the module, or a box's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum End {
    Register,
    Output,
    Box,
}

/// The deepest path into a register or an output, over all its bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// The net's name without its bit, or a memory's point.
    pub name: String,
    pub end: End,
    pub bits: usize,
    pub depth: i64,
    /// The path's nodes, from where it starts to the bit it ends at: a register's input, or the output.
    pub path: Vec<usize>,
}

/// Why a module has no timing graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimingError {
    Netlist(NetlistError),
    /// The mapped netlist is not one this reads.
    Json(String),
    /// A cell of a kind the model has no delay for.
    Cell { name: String, kind: String },
    /// A level-sensitive latch: the model has no transparent element.
    Latch(String),
    /// A loop of logic through no register, named node by node.
    CombinationalLoop(Vec<String>),
}

impl std::fmt::Display for TimingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimingError::Netlist(e) => write!(f, "{e}"),
            TimingError::Json(why) => write!(f, "the mapped netlist: {why}"),
            TimingError::Cell { name, kind } => write!(f, "cell {name} is a {kind}, which the timing model has no delay for"),
            TimingError::Latch(name) => write!(f, "{name} is a level-sensitive latch"),
            TimingError::CombinationalLoop(names) => write!(f, "a combinational loop: {}", names.join(" -> ")),
        }
    }
}

impl std::error::Error for TimingError {}

impl From<NetlistError> for TimingError {
    fn from(e: NetlistError) -> TimingError {
        TimingError::Netlist(e)
    }
}

/// The module attribute that says what a box's outputs add as they leave it: its [`Cost`]'s four counts, `lut,carry,
/// dsp,memory`; a box without it adds nothing.
pub const BOX_OUT: &str = "logos_box_out";

/// The module attribute that says the area each instance of a box takes, as a device block's stand-in declares it:
/// its [`Area`]'s five counts, `luts,carry,half_dsps,flops,memory_bits`; a box without it takes none.
pub const BOX_AREA: &str = "logos_box_area";

/// A box's area as [`BOX_AREA`] writes it.
fn box_area(module: &str, m: &Module) -> Result<Area, TimingError> {
    let Some(v) = m.attributes.get(BOX_AREA) else { return Ok(Area::default()) };
    let counts: Option<Vec<u64>> = v.as_str().and_then(|t| t.split(',').map(|c| c.trim().parse().ok()).collect());
    match counts.as_deref() {
        Some(&[luts, carry, half_dsps, flops, memory_bits]) => Ok(Area { luts, carry, half_dsps, flops, memory_bits }),
        _ => Err(TimingError::Json(format!("box {module}: {BOX_AREA} is {v}, not five counts"))),
    }
}

/// A box's output cost as [`BOX_OUT`] writes it.
fn box_out(module: &str, m: &Module) -> Result<Cost, TimingError> {
    let Some(v) = m.attributes.get(BOX_OUT) else { return Ok(Cost::NONE) };
    let counts: Option<Vec<i64>> = v.as_str().and_then(|t| t.split(',').map(|c| c.trim().parse().ok()).collect());
    match counts.as_deref() {
        Some(&[lut, carry, dsp, memory]) => Ok(Cost { lut, carry, dsp, memory }),
        _ => Err(TimingError::Json(format!("box {module}: {BOX_OUT} is {v}, not four counts"))),
    }
}

/// The cells kept whole as carry chains, DSP blocks and memories; everything else becomes simple gates.
const KEPT: &[&str] = &[
    "$add", "$sub", "$neg", "$lt", "$le", "$gt", "$ge", "$div", "$mod", "$divfloor", "$modfloor", "$mul", "$mem_v2",
];

/// The yosys script that maps `top` with `params` set and writes `timing.json`, each module of `boxes` a box: its
/// outputs' [`Cost`] ([`BOX_OUT`]) and the [`Area`] an instance of it takes ([`BOX_AREA`]), each written only when it
/// is not none.
pub fn script(top: &str, params: &[(&str, &str)], boxes: &[(&str, Cost, Area)]) -> String {
    let mut hierarchy = format!("hierarchy -check -top {top}");
    for (name, value) in params {
        hierarchy.push_str(&format!(" -chparam {name} {value}"));
    }
    let kept = KEPT.iter().enumerate().map(|(i, t)| if i == 0 { format!("t:{t}") } else { format!("t:{t} %u") }).collect::<Vec<_>>();
    let mut steps = vec!["read_verilog -sv design.v".to_string()];
    // A box is emptied before the hierarchy is elaborated, so nothing under it ever is; a box no module is named is
    // an error, not a design timed whole.
    for (b, out, area) in boxes {
        steps.push(format!("select -assert-any ={b}"));
        steps.push(format!("blackbox ={b}"));
        if *out != Cost::NONE {
            steps.push(format!("setattr -mod -set {BOX_OUT} \"{},{},{},{}\" ={b}", out.lut, out.carry, out.dsp, out.memory));
        }
        if *area != Area::default() {
            let Area { luts, carry, half_dsps, flops, memory_bits } = area;
            steps.push(format!("setattr -mod -set {BOX_AREA} \"{luts},{carry},{half_dsps},{flops},{memory_bits}\" ={b}"));
        }
    }
    steps.extend([
        hierarchy,
        "proc".into(),
        "opt_dff -nosdff -keepdc".into(),
        "opt_mem".into(),
        // A memory no port reads is dropped before its ports are collected: yosys 0.33 collects one into a cell its
        // own check refuses.
        "opt_clean".into(),
        "memory_share -nosat -nowiden".into(),
        "memory_collect".into(),
        "select -set ported t:$mem_v2 r:WR_PORTS>2 %i".into(),
        "memory_map @ported".into(),
        "opt_expr".into(),
        "opt_clean".into(),
        "memory -nomap".into(),
        "setundef -zero".into(),
        "opt_expr".into(),
        "opt_clean".into(),
        "wreduce".into(),
        "opt_clean".into(),
        "select -set unclocked t:$mem_v2 r:RD_CLK_ENABLE<1 %i".into(),
        "memory_map @unclocked".into(),
        "opt_clean".into(),
        format!("techmap {} %n", kept.join(" ")),
        "opt -fast -nosdff".into(),
        "flatten".into(),
        "opt_clean".into(),
        "write_json timing.json".into(),
    ]);
    steps.join("\n") + "\n"
}

/// The modules `verilog` defines whose names are one of `prefixes` and a number, in order of definition: the family a
/// generator writes one module of per width, to name as boxes.
pub fn modules_numbered(verilog: &str, prefixes: &[&str]) -> Vec<String> {
    verilog
        .lines()
        .filter_map(|l| l.strip_prefix("module "))
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>())
        .filter(|name| prefixes.iter().any(|p| name.strip_prefix(p).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))))
        .collect()
}

/// The netlist [`script`] writes for `top` of `verilog` with `params` set, as yosys JSON.
pub fn netlist(verilog: &str, top: &str, params: &[(&str, &str)], boxes: &[(&str, Cost, Area)]) -> Result<String, TimingError> {
    let roots: Vec<&str> = std::iter::once(top).chain(boxes.iter().map(|(b, _, _)| *b)).collect();
    let run = yosys_run(&super::netlist::reachable_modules(verilog, &roots), &script(top, params, boxes), &["timing.json"])?;
    if !run.ok {
        return Err(run.refused().into());
    }
    let json = run.files[0].as_ref().ok_or_else(|| TimingError::Netlist(NetlistError::Io("yosys wrote no timing.json".into())))?;
    Ok(String::from_utf8_lossy(json).into_owned())
}

/// Map `top` of `verilog` with `params` set, each module of `boxes` a box, and read its timing graph.
pub fn synthesize(verilog: &str, top: &str, params: &[(&str, &str)], boxes: &[(&str, Cost, Area)], delays: &Delays) -> Result<TimingGraph, TimingError> {
    read(&netlist(verilog, top, params, boxes)?, delays)
}

#[derive(Deserialize)]
struct Design {
    modules: BTreeMap<String, Module>,
}

#[derive(Deserialize)]
struct Module {
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    ports: BTreeMap<String, Port>,
    #[serde(default)]
    cells: BTreeMap<String, Cell>,
    #[serde(default)]
    netnames: BTreeMap<String, Net>,
}

#[derive(Deserialize)]
struct Port {
    direction: String,
    #[serde(default)]
    bits: Vec<Bit>,
}

#[derive(Deserialize)]
struct Cell {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    parameters: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    port_directions: BTreeMap<String, String>,
    #[serde(default)]
    connections: BTreeMap<String, Vec<Bit>>,
}

impl Cell {
    /// Whether `port` is an input: as the netlist says, else by yosys's pin names, where outputs are `Y`, `Q` and
    /// `RD_DATA`.
    fn reads(&self, port: &str) -> bool {
        match self.port_directions.get(port) {
            Some(d) => d == "input",
            None => !matches!(port, "Y" | "Q" | "RD_DATA"),
        }
    }
}

#[derive(Deserialize)]
struct Net {
    bits: Vec<Bit>,
    #[serde(default)]
    hide_name: u8,
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum Bit {
    Net(u64),
    Constant(Level),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
enum Level {
    #[serde(rename = "0")]
    Zero,
    #[serde(rename = "1")]
    One,
    #[serde(rename = "x")]
    X,
    #[serde(rename = "z")]
    Z,
}

/// A parameter yosys writes as a binary string, most significant bit first, or as a number.
fn number(v: Option<&serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::String(s)) => s.bytes().fold(0u64, |v, b| (v << 1) | (b == b'1') as u64),
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn param(cell: &Cell, name: &str) -> u64 {
    number(cell.parameters.get(name))
}

/// A parameter's bits, least significant first.
fn param_bits(cell: &Cell, name: &str) -> Vec<bool> {
    match cell.parameters.get(name) {
        Some(serde_json::Value::String(s)) => s.bytes().rev().map(|b| b == b'1').collect(),
        Some(serde_json::Value::Number(n)) => {
            let v = n.as_u64().unwrap_or(0);
            (0..64).map(|i| v >> i & 1 == 1).collect()
        }
        _ => Vec::new(),
    }
}

/// The instance a flattened cell came from, `|`-separated: from a `$flatten\a.\b.$…` name, or from the `hdlname`
/// attribute of a public one.
fn instance_of(name: &str, cell: &Cell) -> String {
    if let Some(rest) = name.strip_prefix("$flatten") {
        let scope = rest.find(".$").map_or(rest, |end| &rest[..end]);
        return scope.split(".\\").map(|c| c.trim_start_matches('\\')).collect::<Vec<_>>().join("|");
    }
    if let Some(serde_json::Value::String(h)) = cell.attributes.get("hdlname") {
        let parts: Vec<&str> = h.split(' ').collect();
        return parts[..parts.len().saturating_sub(1)].join("|");
    }
    String::new()
}

/// Whether `kind` is one of yosys's simple combinational gates, which [`read`] maps onto LUTs.
fn is_gate(kind: &str) -> bool {
    matches!(
        kind,
        "$_BUF_" | "$_NOT_" | "$_AND_" | "$_NAND_" | "$_OR_" | "$_NOR_" | "$_XOR_" | "$_XNOR_" | "$_ANDNOT_" | "$_ORNOT_"
            | "$_MUX_" | "$_NMUX_" | "$_AOI3_" | "$_OAI3_" | "$_AOI4_" | "$_OAI4_"
    )
}

/// A simple gate's output, its pins' literals given by `pin`.
fn gate(x: &mut Xag, kind: &str, pin: impl Fn(&str) -> Lit) -> Lit {
    let (a, b) = (pin("A"), pin("B"));
    match kind {
        "$_BUF_" => a,
        "$_NOT_" => a ^ 1,
        "$_AND_" => x.and(a, b),
        "$_NAND_" => x.and(a, b) ^ 1,
        "$_OR_" => x.or(a, b),
        "$_NOR_" => x.or(a, b) ^ 1,
        "$_XOR_" => x.xor(a, b),
        "$_XNOR_" => x.xor(a, b) ^ 1,
        "$_ANDNOT_" => x.and(a, b ^ 1),
        "$_ORNOT_" => x.or(a, b ^ 1),
        "$_MUX_" => x.mux(pin("S"), b, a),
        "$_NMUX_" => x.mux(pin("S"), b, a) ^ 1,
        "$_AOI3_" => {
            let ab = x.and(a, b);
            x.or(ab, pin("C")) ^ 1
        }
        "$_OAI3_" => {
            let ab = x.or(a, b);
            x.and(ab, pin("C")) ^ 1
        }
        "$_AOI4_" => {
            let (ab, cd) = (x.and(a, b), x.and(pin("C"), pin("D")));
            x.or(ab, cd) ^ 1
        }
        "$_OAI4_" => {
            let (ab, cd) = (x.or(a, b), x.or(pin("C"), pin("D")));
            x.and(ab, cd) ^ 1
        }
        _ => unreachable!("is_gate admits only the gates above"),
    }
}

/// The level at which each control pin of a yosys flip-flop leaves it a plain register — an enable at its active level,
/// a reset, a set or a load at its inactive one — read from the polarity letters its name carries (`$_SDFFE_PP0N_`:
/// clock rising, reset active high to 0, enable active low). `None` for a kind of flip-flop not named here.
fn idle_levels(kind: &str) -> Option<Vec<(&'static str, bool)>> {
    let (family, letters) = kind.strip_prefix("$_")?.strip_suffix('_')?.split_once('_')?;
    let high = |c: char| c == 'P';
    let l: Vec<char> = letters.chars().collect();
    Some(match (family, l.as_slice()) {
        ("DFF", [_]) => vec![],
        ("DFF" | "SDFF", [_, r, _]) => vec![("R", !high(*r))],
        ("DFFE", [_, e]) => vec![("E", high(*e))],
        ("DFFE" | "SDFFE" | "SDFFCE", [_, r, _, e]) => vec![("R", !high(*r)), ("E", high(*e))],
        ("DFFSR", [_, s, r]) => vec![("S", !high(*s)), ("R", !high(*r))],
        ("DFFSRE", [_, s, r, e]) => vec![("S", !high(*s)), ("R", !high(*r)), ("E", high(*e))],
        ("ALDFF", [_, l]) => vec![("L", !high(*l))],
        ("ALDFFE", [_, l, e]) => vec![("L", !high(*l)), ("E", high(*e))],
        _ => return None,
    })
}

/// Whether a flip-flop steps under anything but its clock: a control pin not held at the level that leaves it plain,
/// or a kind [`idle_levels`] does not name.
fn controlled(kind: &str, cell: &Cell) -> bool {
    let Some(idle) = idle_levels(kind) else { return true };
    idle.iter().any(|&(pin, level)| {
        let held = if level { Bit::Constant(Level::One) } else { Bit::Constant(Level::Zero) };
        cell.connections.get(pin).is_none_or(|bits| bits.iter().any(|&b| b != held))
    })
}

fn is_flop(kind: &str) -> bool {
    kind.starts_with("$_") && kind.contains("DFF")
}

fn is_latch(kind: &str) -> bool {
    kind.starts_with("$_DLATCH") || kind.starts_with("$_SR_")
}

struct Builder<'a> {
    delays: &'a Delays,
    nodes: Vec<String>,
    net: HashMap<u64, usize>,
    names: HashMap<u64, (String, bool)>,
    public: Vec<bool>,
    edges: Vec<Edge>,
    costs: Vec<Cost>,
    owner: Vec<usize>,
    instances: Vec<String>,
    instance: HashMap<String, usize>,
    flops: usize,
    area: Vec<Area>,
    source: Vec<Source>,
    cells: Vec<KeptCell>,
    /// The modules the netlist holds as black boxes with what their outputs add, and the instances of them read so far.
    box_modules: HashMap<String, (Cost, Area)>,
    boxes: Vec<Boxed>,
    /// The adders inside DSP blocks ([`dsp_sums`]).
    dsp_sums: HashSet<String>,
    /// What each edge loads its start with, each node's driver where its source does not say it, and the clocks.
    loads: Vec<u64>,
    drivers: Vec<Option<Driver>>,
    clocks: HashSet<usize>,
}

impl Builder<'_> {
    fn bit(&mut self, b: Bit) -> Option<usize> {
        let Bit::Net(id) = b else { return None };
        if let Some(&n) = self.net.get(&id) {
            return Some(n);
        }
        let n = self.nodes.len();
        let (name, public) = self.names.get(&id).cloned().unwrap_or_else(|| (format!("net{id}"), false));
        self.nodes.push(name);
        self.public.push(public);
        self.source.push(Source::Inside);
        self.drivers.push(None);
        self.net.insert(id, n);
        Some(n)
    }

    fn point(&mut self, name: String) -> usize {
        self.nodes.push(name);
        self.public.push(false);
        self.source.push(Source::Inside);
        self.drivers.push(Some(Driver::Point));
        self.nodes.len() - 1
    }

    fn pin(&mut self, b: Bit) -> Pin {
        match b {
            Bit::Net(_) => Pin::Node(self.bit(b).expect("a net bit is a node")),
            Bit::Constant(Level::One) => Pin::One,
            Bit::Constant(_) => Pin::Zero,
        }
    }

    /// Keep `cell` whole, `inside` the node whose stage is its own, and make its output bits its.
    fn keep(&mut self, name: &str, cell: &Cell, inside: usize) {
        let ports: BTreeMap<String, Vec<Pin>> =
            cell.connections.iter().map(|(p, bits)| (p.clone(), bits.iter().map(|&b| self.pin(b)).collect())).collect();
        let index = self.cells.len();
        for (port, pins) in &ports {
            if cell.reads(port) {
                continue;
            }
            for (bit, pin) in pins.iter().enumerate() {
                if let Pin::Node(n) = *pin {
                    self.source[n] = Source::Cell { cell: index, port: port.clone(), bit };
                }
            }
        }
        self.cells.push(KeptCell { name: name.to_string(), kind: cell.kind.clone(), parameters: cell.parameters.clone(), ports, inside });
    }

    fn edge(&mut self, from: Option<usize>, to: Option<usize>, cost: Cost, registers: i64, owner: usize) {
        self.edge_loading(from, to, cost, registers, owner, 1);
    }

    /// An edge that loads its start with `loads`.
    fn edge_loading(&mut self, from: Option<usize>, to: Option<usize>, cost: Cost, registers: i64, owner: usize, loads: u64) {
        if let (Some(from), Some(to)) = (from, to) {
            self.edges.push(Edge { from, to, delay: cost.delay(self.delays), registers });
            self.costs.push(cost);
            self.owner.push(owner);
            self.loads.push(loads);
        }
    }

    /// Edges from `bits`, a memory port's, into `to`: each net's loads on the port, booked on its first edge, are the
    /// block columns its bits fall in (`bits_a_block` bits to a column; none, every column of the `across`) times the
    /// `down` blocks of each.
    fn memory_port(&mut self, bits: &[Bit], to: Option<usize>, registers: i64, owner: usize, across: u64, down: u64, bits_a_block: Option<u64>) {
        let mut columns: HashMap<usize, HashSet<u64>> = HashMap::new();
        let nodes: Vec<Option<usize>> = bits.iter().map(|&b| self.bit(b)).collect();
        for (i, n) in nodes.iter().enumerate() {
            if let Some(n) = n {
                columns.entry(*n).or_default().insert(bits_a_block.map_or(0, |w| i as u64 / w));
            }
        }
        let mut booked: HashSet<usize> = HashSet::new();
        for n in nodes {
            let loads = match n {
                Some(n) if booked.insert(n) => bits_a_block.map_or(across, |_| columns[&n].len() as u64) * down,
                _ => 0,
            };
            self.edge_loading(n, to, Cost::NONE, registers, owner, loads);
        }
    }

    /// An instance of a black box: a node for each net bit it reads, the end of the paths into that bit, and the net
    /// bits it drives the start of the paths out of it, each reached from the box's own node after what its outputs
    /// add when they add anything; its declared area added to the instance holding it.
    fn boxed(&mut self, name: &str, cell: &Cell, owner: usize) {
        let (out, area) = self.box_modules[&cell.kind];
        self.area[owner].add(&area);
        let source = (out != Cost::NONE).then(|| self.point(name.to_string()));
        let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
        for (port, bits) in &cell.connections {
            let reads = cell.reads(port);
            for (i, &b) in bits.iter().enumerate() {
                let Some(net) = self.bit(b) else { continue };
                if reads && port.ends_with("clk") {
                    self.clocks.insert(net);
                }
                if reads {
                    let end = self.point(if bits.len() == 1 { format!("{name}.{port}") } else { format!("{name}.{port}[{i}]") });
                    self.edge(Some(net), Some(end), Cost::NONE, 0, owner);
                    inputs.push(end);
                } else {
                    self.edge(source, Some(net), out, 0, owner);
                    self.drivers[net] = Some(Driver::Box);
                    outputs.push(net);
                }
            }
        }
        self.boxes.push(Boxed { cell: name.to_string(), module: cell.kind.clone(), out, inputs, outputs });
    }

    fn owner(&mut self, path: String) -> usize {
        if let Some(&i) = self.instance.get(&path) {
            return i;
        }
        self.instances.push(path.clone());
        self.area.push(Area::default());
        self.instance.insert(path, self.instances.len() - 1);
        self.instances.len() - 1
    }

    fn cell(&mut self, name: &str, cell: &Cell) -> Result<(), TimingError> {
        let owner = self.owner(instance_of(name, cell));
        let port = |p: &str| cell.connections.get(p).cloned().unwrap_or_default();
        let kind = cell.kind.as_str();
        match kind {
            "$lut" => {
                self.area[owner].luts += 1;
                let y = port("Y").first().and_then(|&b| self.bit(b));
                for a in port("A") {
                    let a = self.bit(a);
                    self.edge(a, y, Cost::LUT, 0, owner);
                }
                let inputs = port("A");
                if let (Some(y), true) = (y, inputs.len() <= 6) {
                    let pins = inputs.iter().map(|&b| self.pin(b)).collect();
                    let table = param_bits(cell, "LUT").iter().enumerate().fold(0u64, |t, (i, &b)| t | (b as u64) << i);
                    self.source[y] = Source::Lut { pins, table, cone: None };
                }
            }
            "$add" | "$sub" if self.dsp_sums.contains(name) => {
                let inside = self.point(format!("{name}.dsp"));
                for x in port("A").into_iter().chain(port("B")) {
                    let x = self.bit(x);
                    self.edge(x, Some(inside), Cost::NONE, 0, owner);
                }
                for y in port("Y") {
                    let y = self.bit(y);
                    self.edge(Some(inside), y, Cost::NONE, 0, owner);
                }
                self.keep(name, cell, inside);
            }
            "$add" | "$sub" | "$neg" | "$lt" | "$le" | "$gt" | "$ge" => {
                let (a, b, y) = (port("A"), port("B"), port("Y"));
                let compare = matches!(kind, "$lt" | "$le" | "$gt" | "$ge");
                let width = if compare { a.len().max(b.len()) } else { y.len() };
                self.area[owner].carry += width as u64;
                let extend = |bits: &[Bit], signed: bool, k: usize| -> Option<Bit> {
                    if k < bits.len() {
                        Some(bits[k])
                    } else if signed {
                        bits.last().copied()
                    } else {
                        None
                    }
                };
                let (sa, sb) = (param(cell, "A_SIGNED") == 1, param(cell, "B_SIGNED") == 1);
                let mut previous = None;
                let mut first = None;
                for k in 0..width {
                    let carry = Some(self.point(format!("{name}.carry[{k}]")));
                    first = first.or(carry);
                    for x in [extend(&a, sa, k), extend(&b, sb, k)].into_iter().flatten() {
                        let x = self.bit(x);
                        self.edge(x, carry, Cost::LUT, 0, owner);
                    }
                    self.edge(previous, carry, Cost::CARRY, 0, owner);
                    previous = carry;
                    if !compare {
                        let out = self.bit(y[k]);
                        self.edge(carry, out, Cost::NONE, 0, owner);
                    }
                }
                if compare {
                    let out = y.first().and_then(|&b| self.bit(b));
                    self.edge(previous, out, Cost::NONE, 0, owner);
                }
                if let Some(inside) = first {
                    self.keep(name, cell, inside);
                }
            }
            "$div" | "$mod" | "$divfloor" | "$modfloor" => {
                let (a, b, y) = (port("A"), port("B"), port("Y"));
                self.area[owner].carry += (a.len() * b.len()) as u64;
                let stage = Cost { lut: 1, carry: b.len() as i64, ..Cost::NONE };
                let mut previous = None;
                let mut stages = Vec::with_capacity(a.len());
                for s in 0..a.len() {
                    let node = Some(self.point(format!("{name}.stage[{s}]")));
                    let dividend = self.bit(a[a.len() - 1 - s]);
                    self.edge(dividend, node, stage, 0, owner);
                    for &x in &b {
                        let x = self.bit(x);
                        self.edge(x, node, stage, 0, owner);
                    }
                    self.edge(previous, node, stage, 0, owner);
                    previous = node;
                    stages.push(node);
                }
                let quotient = matches!(kind, "$div" | "$divfloor");
                for (i, &out) in y.iter().enumerate() {
                    let from = if quotient { a.len().checked_sub(1 + i).and_then(|s| stages[s]) } else { previous };
                    let out = self.bit(out);
                    self.edge(from, out, Cost::NONE, 0, owner);
                }
                if let Some(inside) = stages.first().copied().flatten() {
                    self.keep(name, cell, inside);
                }
            }
            "$mul" => {
                let (a, b, y) = (port("A"), port("B"), port("Y"));
                let (sa, sb) = (param(cell, "A_SIGNED") == 1, param(cell, "B_SIGNED") == 1);
                let shift = power(&a, sa).map(|s| (s, &b, sb)).or_else(|| power(&b, sb).map(|s| (s, &a, sa)));
                if let Some((s, other, signed)) = shift {
                    for (k, &out) in y.iter().enumerate() {
                        let from = if k < s || other.is_empty() {
                            None
                        } else if k - s < other.len() {
                            Some(other[k - s])
                        } else if signed {
                            other.last().copied()
                        } else {
                            None
                        };
                        let to = self.bit(out);
                        match from.map(|b| self.pin(b)) {
                            Some(Pin::Node(f)) => {
                                self.edge(Some(f), to, Cost::NONE, 0, owner);
                                if let Some(to) = to {
                                    self.source[to] = Source::Copy { from: f, inverted: false };
                                }
                            }
                            constant => {
                                if let Some(to) = to {
                                    self.source[to] = Source::Constant(constant == Some(Pin::One));
                                }
                            }
                        }
                    }
                } else {
                    self.area[owner].half_dsps += dsp_halves(a.len(), b.len());
                    let dsp = Some(self.point(format!("{name}.dsp")));
                    for x in a.iter().chain(b.iter()) {
                        let x = self.bit(*x);
                        self.edge(x, dsp, Cost::NONE, 0, owner);
                    }
                    for &out in &y {
                        let out = self.bit(out);
                        self.edge(dsp, out, Cost::DSP, 0, owner);
                    }
                    if let Some(inside) = dsp {
                        self.keep(name, cell, inside);
                    }
                }
            }
            "$mem_v2" => {
                let (abits, width) = (param(cell, "ABITS") as usize, param(cell, "WIDTH") as usize);
                let size = param(cell, "SIZE");
                self.area[owner].memory_bits += width as u64 * size;
                let style = match cell.attributes.get("ramstyle") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let (across, down) = memory_blocks(width as u64, size, &style);
                let bits_a_block = width.div_ceil(across as usize).max(1) as u64;
                let reads = param(cell, "RD_PORTS") as usize;
                let writes = param(cell, "WR_PORTS") as usize;
                let clocked = param_bits(cell, "RD_CLK_ENABLE");
                let contents = Some(self.point(format!("{name}.contents")));
                for c in port("RD_CLK").into_iter().chain(port("WR_CLK")) {
                    if let Some(c) = self.bit(c) {
                        self.clocks.insert(c);
                    }
                }
                let (waddr, wdata, wen) = (port("WR_ADDR"), port("WR_DATA"), port("WR_EN"));
                for w in 0..writes.max(1) {
                    let a: Vec<Bit> = waddr.iter().skip(w * abits).take(abits).copied().collect();
                    self.memory_port(&a, contents, 1, owner, across, down, None);
                    let d: Vec<Bit> = wdata.iter().skip(w * width).take(width).copied().collect();
                    self.memory_port(&d, contents, 1, owner, across, down, Some(bits_a_block));
                    let e: Vec<Bit> = wen.iter().skip(w * width).take(width).copied().collect();
                    self.memory_port(&e, contents, 1, owner, across, down, Some(bits_a_block));
                }
                let (addr, data, en, srst, arst) = (port("RD_ADDR"), port("RD_DATA"), port("RD_EN"), port("RD_SRST"), port("RD_ARST"));
                for r in 0..reads {
                    let sync = clocked.get(r).copied().unwrap_or(false);
                    let held = if sync { 1 } else { 0 };
                    let read = Some(self.point(format!("{name}.read[{r}]")));
                    self.edge(contents, read, Cost::NONE, held, owner);
                    let controls: Vec<Bit> =
                        addr.iter().skip(r * abits).take(abits).chain(en.get(r)).chain(srst.get(r)).chain(arst.get(r)).copied().collect();
                    self.memory_port(&controls, read, held, owner, across, down, None);
                    for &out in data.iter().skip(r * width).take(width) {
                        let out = self.bit(out);
                        self.edge(read, out, Cost::MEMORY, 0, owner);
                        if let Some(out) = out {
                            self.drivers[out] = Some(Driver::Memory);
                        }
                    }
                }
            }
            _ if self.box_modules.contains_key(kind) => self.boxed(name, cell, owner),
            _ if is_latch(kind) => return Err(TimingError::Latch(name.to_string())),
            _ if is_flop(kind) => {
                let q = port("Q").first().and_then(|&b| self.bit(b));
                self.flops += 1;
                self.area[owner].flops += 1;
                if let Some(q) = q {
                    let data = port("D").first().map_or(Pin::Zero, |&b| self.pin(b));
                    self.source[q] = Source::Register { data, controls: controlled(kind, cell) };
                }
                for &c in &port("C") {
                    if let Some(c) = self.bit(c) {
                        self.clocks.insert(c);
                    }
                }
                for (p, bits) in &cell.connections {
                    if p == "C" || p == "Q" {
                        continue;
                    }
                    for &x in bits {
                        let x = self.bit(x);
                        self.edge(x, q, Cost::NONE, 1, owner);
                    }
                }
            }
            _ => return Err(TimingError::Cell { name: name.to_string(), kind: kind.to_string() }),
        }
        Ok(())
    }
}

/// The bit set in `bits` when they are a constant power of two (and, signed, not the sign bit): the shift a product by
/// them is.
fn power(bits: &[Bit], signed: bool) -> Option<usize> {
    let constant = bits.iter().all(|b| matches!(b, Bit::Constant(_)));
    let ones: Vec<usize> = (0..bits.len()).filter(|&i| bits[i] == Bit::Constant(Level::One)).collect();
    (constant && ones.len() == 1 && !(signed && ones[0] + 1 == bits.len())).then(|| ones[0])
}

/// The adders of `module` that sit inside DSP blocks, as Quartus packs them: each whose two operands are products,
/// sums found so already or constants, at least one bit of each a product's or a sum's (a block's adder of its two
/// products, and the chain-in adder of the block after it), found to a fixpoint. An adder with an operand from the
/// fabric is a carry chain of its own.
fn dsp_sums(module: &Module) -> HashSet<String> {
    let nets = |bits: &[Bit]| bits.iter().filter_map(|b| if let Bit::Net(id) = b { Some(*id) } else { None }).collect::<Vec<_>>();
    let port = |cell: &Cell, p: &str| cell.connections.get(p).cloned().unwrap_or_default();
    let mut dsp: HashSet<u64> = HashSet::new();
    for cell in module.cells.values().filter(|c| c.kind == "$mul") {
        let (sa, sb) = (number(cell.parameters.get("A_SIGNED")) == 1, number(cell.parameters.get("B_SIGNED")) == 1);
        if power(&port(cell, "A"), sa).is_none() && power(&port(cell, "B"), sb).is_none() {
            dsp.extend(nets(&port(cell, "Y")));
        }
    }
    let mut sums: HashSet<String> = HashSet::new();
    loop {
        let mut grew = false;
        for (name, cell) in &module.cells {
            if !matches!(cell.kind.as_str(), "$add" | "$sub") || sums.contains(name) {
                continue;
            }
            let inside = |p: &str| {
                let bits = port(cell, p);
                let ids = nets(&bits);
                !ids.is_empty() && ids.iter().all(|id| dsp.contains(id))
            };
            if inside("A") && inside("B") {
                sums.insert(name.clone());
                dsp.extend(nets(&port(cell, "Y")));
                grew = true;
            }
        }
        if !grew {
            return sums;
        }
    }
}

/// A node's name without the bit its net is indexed by, nor the word of the array it is a register of.
fn unindexed(node: &str) -> &str {
    let mut net = node;
    while let Some((head, index)) = net.strip_suffix(']').and_then(|n| n.rsplit_once('[')) {
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        net = head;
    }
    net
}

/// A net's display name for each bit, and whether it is public: a public name before a generated one, then the name
/// of the outermost scope (a flattened instance's names carry its path, `.`-separated), then the shorter.
fn net_names(module: &Module) -> HashMap<u64, (String, bool)> {
    let mut best: HashMap<u64, (bool, usize, usize, String)> = HashMap::new();
    for (name, net) in &module.netnames {
        let hidden = net.hide_name != 0 || name.starts_with('$');
        let scopes = name.matches('.').count();
        for (i, &b) in net.bits.iter().enumerate() {
            let Bit::Net(id) = b else { continue };
            let shown = if net.bits.len() == 1 { name.clone() } else { format!("{name}[{i}]") };
            let rank = (hidden, scopes, shown.len(), shown);
            match best.get(&id) {
                Some(have) if *have <= rank => {}
                _ => {
                    best.insert(id, rank);
                }
            }
        }
    }
    best.into_iter().map(|(id, (hidden, _, _, name))| (id, (name, !hidden))).collect()
}

/// The timing graph of a netlist [`script`] mapped.
pub fn read(json: &str, delays: &Delays) -> Result<TimingGraph, TimingError> {
    let design: Design = serde_json::from_str(json).map_err(|e| TimingError::Json(e.to_string()))?;
    let flag = |m: &Module, name: &str| match m.attributes.get(name) {
        Some(serde_json::Value::String(s)) => s.contains('1'),
        Some(serde_json::Value::Number(n)) => n.as_u64() == Some(1),
        _ => false,
    };
    let box_modules: HashMap<String, (Cost, Area)> = design
        .modules
        .iter()
        .filter(|(_, m)| flag(m, "blackbox"))
        .map(|(name, m)| Ok((name.clone(), (box_out(name, m)?, box_area(name, m)?))))
        .collect::<Result<_, TimingError>>()?;
    let module = match design.modules.values().filter(|m| flag(m, "top")).collect::<Vec<_>>().as_slice() {
        [m] => *m,
        [] if design.modules.len() == 1 => design.modules.values().next().expect("one module"),
        _ => return Err(TimingError::Json("no single top module".into())),
    };
    let mut b = Builder {
        delays,
        nodes: Vec::new(),
        net: HashMap::new(),
        names: net_names(module),
        public: Vec::new(),
        source: Vec::new(),
        cells: Vec::new(),
        edges: Vec::new(),
        costs: Vec::new(),
        owner: Vec::new(),
        instances: Vec::new(),
        instance: HashMap::new(),
        flops: 0,
        area: Vec::new(),
        box_modules,
        boxes: Vec::new(),
        dsp_sums: dsp_sums(module),
        loads: Vec::new(),
        drivers: Vec::new(),
        clocks: HashSet::new(),
    };
    b.owner(String::new());
    let mut gates: HashMap<u64, (&str, &Cell)> = HashMap::new();
    let mut read: Vec<u64> = Vec::new();
    let nets = |bits: &[Bit]| bits.iter().filter_map(|b| if let Bit::Net(id) = b { Some(*id) } else { None }).collect::<Vec<_>>();
    for (name, cell) in &module.cells {
        if is_gate(&cell.kind) {
            if let Some(&Bit::Net(y)) = cell.connections.get("Y").and_then(|y| y.first()) {
                gates.insert(y, (name, cell));
            }
            continue;
        }
        b.cell(name, cell)?;
        for (port, bits) in &cell.connections {
            if cell.reads(port) {
                read.extend(nets(bits));
            }
        }
    }
    for port in module.ports.values().filter(|p| p.direction == "output") {
        read.extend(nets(&port.bits));
    }
    b.logic(&gates, read);
    let mut ports = |direction: &str| -> Vec<usize> {
        module.ports.values().filter(|p| p.direction == direction).flat_map(|p| p.bits.iter().copied()).filter_map(|bit| b.bit(bit)).collect()
    };
    let (inputs, outputs) = (ports("input"), ports("output"));
    let mut named = |direction: &str| -> Vec<(String, Vec<Pin>)> {
        module.ports.iter().filter(|(_, p)| p.direction == direction).map(|(name, p)| (name.clone(), p.bits.iter().map(|&bit| b.pin(bit)).collect())).collect()
    };
    let (input_ports, output_ports) = (named("input"), named("output"));
    for (port, pins) in &input_ports {
        for (bit, pin) in pins.iter().enumerate() {
            if let Pin::Node(n) = *pin {
                b.source[n] = Source::Input { port: port.clone(), bit };
            }
        }
    }
    let drivers = b
        .drivers
        .iter()
        .zip(&b.source)
        .map(|(d, s)| {
            d.unwrap_or(match s {
                Source::Register { .. } => Driver::Register,
                Source::Input { .. } | Source::Inside => Driver::Input,
                Source::Lut { .. } | Source::Copy { .. } | Source::Cell { .. } | Source::Constant(_) => Driver::Logic,
            })
        })
        .collect();
    let mut asked: HashMap<u64, u64> = HashMap::new();
    for net in module.netnames.values() {
        let limit = match net.attributes.get("maxfan") {
            Some(serde_json::Value::String(v)) => u64::from_str_radix(v, 2).ok(),
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            _ => None,
        };
        if let Some(limit) = limit {
            for &bit in &net.bits {
                if let Bit::Net(id) = bit {
                    asked.insert(id, limit);
                }
            }
        }
    }
    let mut limits = vec![None; b.nodes.len()];
    for (id, &node) in &b.net {
        limits[node] = asked.get(id).copied();
    }
    let clocks = (0..b.nodes.len()).map(|n| b.clocks.contains(&n)).collect();
    let fanout = Fanout { loads: b.loads, drivers, limits, clocks };
    let structure = Structure { source: b.source, cells: b.cells, input_ports, output_ports };
    Ok(TimingGraph {
        nodes: b.nodes,
        edges: b.edges,
        costs: b.costs,
        owner: b.owner,
        instances: b.instances,
        flops: b.flops,
        area: b.area,
        public: b.public,
        inputs,
        outputs,
        structure: Some(structure),
        boxes: b.boxes,
        fanout: Some(fanout),
    })
}

/// The gates between `root` and its cut `leaves` as a [`Cone`] over the pins, pin `j` holding leaf `j` inverted when
/// bit `j` of `flip` is set, and the output inverted when `invert`.
fn cone(xag: &Xag, root: u32, leaves: &[u32], flip: usize, invert: bool) -> Cone {
    let mut index: HashMap<u32, usize> = leaves.iter().enumerate().map(|(j, &l)| (l, j)).collect();
    let pin_flip = |n: u32, index: &HashMap<u32, usize>| -> bool { index.get(&n).is_some_and(|&j| j < leaves.len() && flip >> j & 1 == 1) };
    let mut gates: Vec<ConeGate> = Vec::new();
    let mut stack = vec![root];
    while let Some(&n) = stack.last() {
        if index.contains_key(&n) {
            stack.pop();
            continue;
        }
        let (a, b, xor) = match xag.gate(n as usize) {
            lutmap::Gate::And(a, b) => (a, b, false),
            lutmap::Gate::Xor(a, b) => (a, b, true),
            _ => unreachable!("a cut stops every path at a leaf"),
        };
        let missing: Vec<u32> = [a, b].iter().map(|&l| lutmap::node(l) as u32).filter(|m| !index.contains_key(m)).collect();
        if !missing.is_empty() {
            stack.extend(missing);
            continue;
        }
        let arg = |l: Lit| {
            let m = lutmap::node(l) as u32;
            ConeArg { index: index[&m], inverted: lutmap::complemented(l) ^ pin_flip(m, &index) }
        };
        let gate = ConeGate { xor, a: arg(a), b: arg(b) };
        index.insert(n, leaves.len() + gates.len());
        gates.push(gate);
        stack.pop();
    }
    let out = ConeArg { index: index[&root], inverted: invert ^ pin_flip(root, &index) };
    Cone { gates, out }
}

/// The gates as an AND-XOR graph: each gate-driven net's literal, the net each input stands for, the instance each
/// node was made in, and the nets read inside a loop of their own, each read there through an input of its own.
struct Logic {
    xag: Xag,
    lit: HashMap<u64, Lit>,
    cut: HashMap<u64, Lit>,
    input_net: HashMap<usize, u64>,
    group: Vec<u32>,
}

impl Logic {
    fn input(&mut self, net: u64) -> Lit {
        let l = self.xag.input();
        self.input_net.insert(lutmap::node(l), net);
        self.group.push(0);
        l
    }

    /// The literal a reader of `net` uses.
    fn read(&self, net: u64) -> Option<Lit> {
        self.lit.get(&net).or_else(|| self.cut.get(&net)).copied()
    }
}

impl Builder<'_> {
    /// Build the literal of `net`: an input when no gate drives it, else its gate over its pins' literals, pins first,
    /// without recursion. A net met again while its own fanin is being built is read there through an input of its
    /// own, so the graph stays acyclic and the loop stays in the timing graph, closed through that net.
    fn literal(&mut self, logic: &mut Logic, gates: &HashMap<u64, (&str, &Cell)>, net: u64) {
        let mut open: HashSet<u64> = HashSet::new();
        let mut stack: Vec<u64> = vec![net];
        while let Some(&top) = stack.last() {
            if logic.lit.contains_key(&top) {
                stack.pop();
                continue;
            }
            let Some(&(name, cell)) = gates.get(&top) else {
                let l = logic.input(top);
                logic.lit.insert(top, l);
                stack.pop();
                continue;
            };
            open.insert(top);
            let mut waiting = false;
            for (port, bits) in &cell.connections {
                if !cell.reads(port) {
                    continue;
                }
                for &bit in bits {
                    let Bit::Net(id) = bit else { continue };
                    if logic.read(id).is_some() {
                        continue;
                    }
                    if open.contains(&id) {
                        let l = logic.input(id);
                        logic.cut.insert(id, l);
                    } else {
                        stack.push(id);
                        waiting = true;
                    }
                }
            }
            if waiting {
                continue;
            }
            let owner = self.owner(instance_of(name, cell)) as u32;
            let pins: Vec<(&str, Lit)> = cell
                .connections
                .iter()
                .filter(|(p, _)| cell.reads(p))
                .map(|(p, bits)| {
                    let l = match bits.first() {
                        Some(&Bit::Net(id)) => logic.read(id).expect("a pin is built before its gate"),
                        Some(Bit::Constant(Level::One)) => lutmap::TRUE,
                        _ => lutmap::FALSE,
                    };
                    (p.as_str(), l)
                })
                .collect();
            let out = gate(&mut logic.xag, &cell.kind, |p| pins.iter().find(|(q, _)| *q == p).map_or(lutmap::FALSE, |&(_, l)| l));
            let made = logic.xag.len();
            logic.group.resize(made, owner);
            logic.lit.insert(top, out);
            open.remove(&top);
            stack.pop();
        }
    }

    /// Map the gates onto LUTs and add their edges: each LUT's, from its leaves to its root, and one of no delay into
    /// every gate-driven net read past the gates, or read inside its own loop, from the LUT that computes it.
    fn logic(&mut self, gates: &HashMap<u64, (&str, &Cell)>, read: Vec<u64>) {
        let mut logic = Logic { xag: Xag::new(), lit: HashMap::new(), cut: HashMap::new(), input_net: HashMap::new(), group: vec![0] };
        let mut driven: Vec<u64> = read.into_iter().filter(|n| gates.contains_key(n)).collect();
        driven.sort_unstable();
        driven.dedup();
        for &net in &driven {
            self.literal(&mut logic, gates, net);
        }
        let mut cut: Vec<u64> = logic.cut.keys().copied().collect();
        cut.sort_unstable();
        driven.extend(cut);
        driven.sort_unstable();
        driven.dedup();
        let outputs: Vec<Lit> = driven.iter().map(|n| logic.lit[n]).collect();
        let mapped = lutmap::map(&logic.xag, &logic.group, &outputs, 6);
        let mut named: Vec<(u64, Lit)> = logic.lit.iter().map(|(&n, &l)| (n, l)).collect();
        named.sort_unstable();
        let mut net_of: HashMap<usize, u64> = HashMap::new();
        for (net, l) in named {
            net_of.entry(lutmap::node(l)).or_insert(net);
        }
        net_of.extend(logic.input_net.iter().map(|(&n, &net)| (n, net)));
        let mut timing: HashMap<u32, usize> = HashMap::new();
        let mut at = |b: &mut Builder<'_>, m: u32| -> usize {
            if let Some(&t) = timing.get(&m) {
                return t;
            }
            let origin = mapped.origin[m as usize] as usize;
            let t = match (mapped.exact[m as usize], net_of.get(&origin)) {
                (true, Some(&net)) => b.bit(Bit::Net(net)).expect("a net"),
                (false, Some(&net)) => {
                    let base = b.bit(Bit::Net(net)).map(|t| b.nodes[t].clone()).expect("a net");
                    b.point(format!("{base}~{m}"))
                }
                (_, None) => b.point(format!("lut{m}")),
            };
            timing.insert(m, t);
            t
        };
        let owner_of = |m: u32| logic.group[mapped.origin[m as usize] as usize] as usize;
        // Whether a mapped node's timing node holds the node's complement: a net named it, and the net is it inverted.
        let inverted = |m: u32| -> bool {
            let origin = mapped.origin[m as usize] as usize;
            if !mapped.exact[m as usize] || matches!(logic.xag.gate(origin), lutmap::Gate::Input) {
                return false;
            }
            match (net_of.get(&origin), mapped.image[origin]) {
                (Some(net), Some(img)) => lutmap::complemented(logic.lit[net]) ^ lutmap::complemented(img),
                _ => false,
            }
        };
        for lut in &mapped.luts {
            let root = at(self, lut.root);
            self.area[owner_of(lut.root)].luts += 1;
            let mut pins = Vec::with_capacity(lut.leaves.len());
            let mut flip = 0usize;
            for (j, &leaf) in lut.leaves.iter().enumerate() {
                let from = at(self, leaf);
                self.edge(Some(from), Some(root), Cost::LUT, 0, owner_of(lut.root));
                pins.push(Pin::Node(from));
                flip |= (inverted(leaf) as usize) << j;
            }
            let rows = 1usize << lut.leaves.len();
            let table = lutmap::function(&mapped.xag, lut.root, &lut.leaves);
            let mut read = (0..rows).fold(0u64, |t, i| t | (table >> (i ^ flip) & 1) << i);
            if inverted(lut.root) {
                read = !read & if rows == 64 { u64::MAX } else { (1u64 << rows) - 1 };
            }
            let cone = cone(&mapped.xag, lut.root, &lut.leaves, flip, inverted(lut.root));
            self.source[root] = Source::Lut { pins, table: read, cone: Some(cone) };
        }
        for &net in &driven {
            let lit = logic.lit[&net];
            let Some(img) = mapped.image[lutmap::node(lit)] else { continue };
            let m = lutmap::node(img) as u32;
            let complement = lutmap::complemented(lit) ^ lutmap::complemented(img);
            let to = self.bit(Bit::Net(net)).expect("a net");
            if m == 0 {
                self.source[to] = Source::Constant(complement);
                continue;
            }
            let from = at(self, m);
            if from != to {
                self.edge(Some(from), Some(to), Cost::NONE, 0, owner_of(m));
                self.source[to] = Source::Copy { from, inverted: complement ^ inverted(m) };
            }
        }
    }
}

impl TimingGraph {
    /// What instance `path` and every instance under it take ([`Area`]); the top is `""`, the whole graph.
    pub fn area_under(&self, path: &str) -> Area {
        let mut total = Area::default();
        for (instance, area) in self.instances.iter().zip(&self.area) {
            let under = path.is_empty() || instance == path || instance.strip_prefix(path).is_some_and(|rest| rest.starts_with('|'));
            if under {
                total.add(area);
            }
        }
        total
    }

    /// The combinational edges in an order where each comes after every edge into its source, or the loop that
    /// prevents one.
    fn order(&self) -> Result<Vec<usize>, TimingError> {
        let n = self.nodes.len();
        let mut fanin = vec![0usize; n];
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (k, e) in self.edges.iter().enumerate() {
            if e.registers == 0 {
                fanin[e.to] += 1;
                out[e.from].push(k);
            }
        }
        let mut ready: Vec<usize> = (0..n).filter(|&v| fanin[v] == 0).collect();
        let mut order = Vec::with_capacity(self.edges.len());
        let mut done = 0;
        while let Some(v) = ready.pop() {
            done += 1;
            for &k in &out[v] {
                order.push(k);
                let t = self.edges[k].to;
                fanin[t] -= 1;
                if fanin[t] == 0 {
                    ready.push(t);
                }
            }
        }
        if done == n {
            return Ok(order);
        }
        let comb: Vec<Edge> = self.edges.iter().filter(|e| e.registers == 0).map(|e| Edge { delay: 0, ..*e }).collect();
        let cycle = match cycle_ratio::max_cycle_ratio(n, &comb) {
            Ok(Bound::CombinationalLoop(cycle)) => cycle,
            other => unreachable!("an unordered node is on a cycle of combinational edges: {other:?}"),
        };
        Err(TimingError::CombinationalLoop(cycle.iter().map(|&k| self.nodes[comb[k].from].clone()).collect()))
    }

    /// The combinational order, each node's arrival (the deepest combinational path ending there) and the edge
    /// that path arrives by.
    fn arrive(&self) -> Result<(Vec<usize>, Vec<i64>, Vec<usize>), TimingError> {
        let order = self.order()?;
        let mut arrival = vec![0i64; self.nodes.len()];
        let mut via = vec![usize::MAX; self.nodes.len()];
        for &k in &order {
            let e = &self.edges[k];
            let reach = arrival[e.from] + e.delay;
            if reach > arrival[e.to] || reach == arrival[e.to] && via[e.to] == usize::MAX {
                arrival[e.to] = reach;
                via[e.to] = k;
            }
        }
        Ok((order, arrival, via))
    }

    /// Each node's arrival: the deepest combinational path ending there.
    pub fn arrivals(&self) -> Result<Vec<i64>, TimingError> {
        Ok(self.arrive()?.1)
    }

    /// How deep the logic is, overall and through every instance.
    /// Each node's loads: the cell inputs it reaches, a memory port counted as the blocks it reaches; every edge one
    /// load in a graph built by hand.
    pub fn fanouts(&self) -> Vec<u64> {
        let mut loads = vec![0u64; self.nodes.len()];
        for (k, e) in self.edges.iter().enumerate() {
            loads[e.from] += self.fanout.as_ref().map_or(1, |f| f.loads[k]);
        }
        loads
    }

    /// The nets past their bound, widest first (ties by name): those logic or a memory drives past `logic` loads, a
    /// register past `register` — a register whose net asks for at most N loads (`maxfan`) being copies of N each, past
    /// it only when N is — an input past `logic` (whatever drives it outside); a box's outputs are the box's to bound, a
    /// clock rides a clock network, and a point inside a cell is no net. Empty for a graph built by hand.
    pub fn wide(&self, logic: u64, register: u64) -> Vec<Wide> {
        let Some(fanout) = &self.fanout else { return Vec::new() };
        let mut wide: Vec<Wide> = self
            .fanouts()
            .into_iter()
            .enumerate()
            .filter_map(|(node, loads)| {
                if fanout.clocks[node] {
                    return None;
                }
                let driver = fanout.drivers[node];
                let (bound, copy) = match driver {
                    Driver::Register => (register, fanout.limits[node].map_or(loads, |n| n.min(loads))),
                    Driver::Logic | Driver::Input | Driver::Memory => (logic, loads),
                    Driver::Box | Driver::Point => return None,
                };
                (copy > bound).then(|| Wide { node, name: self.nodes[node].clone(), loads, driver })
            })
            .collect();
        wide.sort_by(|a, b| b.loads.cmp(&a.loads).then_with(|| a.name.cmp(&b.name)));
        wide
    }

    pub fn depth(&self) -> Result<Depth, TimingError> {
        let (order, arrival, via) = self.arrive()?;
        let n = self.nodes.len();
        let mut departure = vec![0i64; n];
        for &k in order.iter().rev() {
            let e = &self.edges[k];
            departure[e.from] = departure[e.from].max(e.delay + departure[e.to]);
        }
        let mut own = vec![0i64; self.instances.len()];
        for &k in &order {
            let e = &self.edges[k];
            if e.delay > 0 {
                own[self.owner[k]] = own[self.owner[k]].max(arrival[e.from] + e.delay + departure[e.to]);
            }
        }
        let mut total: BTreeMap<&str, i64> = BTreeMap::new();
        for (i, path) in self.instances.iter().enumerate() {
            let mut at = path.as_str();
            loop {
                let t = total.entry(at).or_insert(0);
                *t = (*t).max(own[i]);
                match at.rfind('|') {
                    Some(cut) => at = &at[..cut],
                    None if !at.is_empty() => at = "",
                    None => break,
                }
            }
        }
        let own_of: HashMap<&str, i64> = self.instances.iter().zip(&own).map(|(p, &d)| (p.as_str(), d)).collect();
        let instances = total
            .into_iter()
            .map(|(path, total)| InstanceDepth { path: path.to_string(), total, own: own_of.get(path).copied().unwrap_or(0) })
            .collect();
        let end = (0..n).max_by_key(|&v| (arrival[v], std::cmp::Reverse(v))).unwrap_or(0);
        let path = if n > 0 { self.trace(&via, end) } else { Vec::new() };
        let edges = path.iter().skip(1).map(|&v| via[v]).collect();
        Ok(Depth { depth: arrival.get(end).copied().unwrap_or(0), path, edges, instances })
    }

    /// Every edge's delay from its cost at `delays`: the graph as reading its netlist at `delays` makes it.
    pub fn reweigh(&mut self, delays: &Delays) {
        assert_eq!(self.costs.len(), self.edges.len(), "a graph read from a netlist has a cost for every edge");
        for (e, c) in self.edges.iter_mut().zip(&self.costs) {
            e.delay = c.delay(delays);
        }
    }

    /// What `edges` add together.
    pub fn cost(&self, edges: &[usize]) -> Cost {
        edges.iter().fold(Cost::NONE, |sum, &k| sum + self.costs[k])
    }

    /// The deepest combinational path ending at `end`, node by node from where it starts.
    fn trace(&self, via: &[usize], end: usize) -> Vec<usize> {
        let mut path = vec![end];
        let mut at = end;
        while via[at] != usize::MAX {
            at = self.edges[via[at]].from;
            path.push(at);
        }
        path.reverse();
        path
    }

    /// The deepest path into each register and each output, deepest first. A register's path ends at its input: a
    /// delay on a registered edge counts after the register, in the next stage.
    pub fn endpoints(&self) -> Result<Vec<Endpoint>, TimingError> {
        let (_, arrival, via) = self.arrive()?;
        let mut deepest: BTreeMap<(End, &str), (HashSet<usize>, usize)> = BTreeMap::new();
        let mut reach = |end: End, bit: usize, from: usize| {
            let (bits, best) = deepest.entry((end, unindexed(&self.nodes[bit]))).or_insert_with(|| (HashSet::new(), from));
            bits.insert(bit);
            if (arrival[from], std::cmp::Reverse(from)) > (arrival[*best], std::cmp::Reverse(*best)) {
                *best = from;
            }
        };
        for e in self.edges.iter().filter(|e| e.registers > 0) {
            reach(End::Register, e.to, e.from);
        }
        for &o in &self.outputs {
            reach(End::Output, o, o);
        }
        for &i in self.boxes.iter().flat_map(|b| &b.inputs) {
            reach(End::Box, i, i);
        }
        let mut ends: Vec<Endpoint> = deepest
            .into_iter()
            .map(|((end, name), (bits, from))| Endpoint { name: name.to_string(), end, bits: bits.len(), depth: arrival[from], path: self.trace(&via, from) })
            .collect();
        ends.sort_by(|a, b| b.depth.cmp(&a.depth).then(a.end.cmp(&b.end)).then_with(|| a.name.cmp(&b.name)));
        Ok(ends)
    }

    /// The bound the design's loops put on its clock, in the graph's delay unit per register.
    pub fn loop_bound(&self) -> Result<Bound, GraphError> {
        cycle_ratio::max_cycle_ratio(self.nodes.len(), &self.edges)
    }

    /// A bound's loop, node by node in order, a register's output marked with the registers it comes out of.
    pub fn name_loop(&self, bound: &Bound) -> Vec<String> {
        let cycle = match bound {
            Bound::CombinationalLoop(c) | Bound::Ratio { cycle: c, .. } => c,
            Bound::Acyclic => return Vec::new(),
        };
        (0..cycle.len())
            .map(|i| {
                let into = &self.edges[cycle[(i + cycle.len() - 1) % cycle.len()]];
                let node = &self.nodes[self.edges[cycle[i]].from];
                match into.registers {
                    0 => node.clone(),
                    r => format!("{node} [{r} register{}]", if r == 1 { "" } else { "s" }),
                }
            })
            .collect()
    }
}

/// Delays fitted to measured depths ([`fit`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Fitted {
    pub delays: Delays,
    /// Each sample's depth at `delays`, in the unit a delay of 100 is one of.
    pub predicted: Vec<f64>,
    /// The root mean square of the log of each prediction over its measure.
    pub error: f64,
}

/// The delays under which each graph's depth comes nearest its measured depth (in the unit a delay of 100 is one of):
/// of `candidates`, each scaled by the factor that fits it best. A candidate's ratios choose every critical path, and
/// a graph's depth scales with all its delays together, so the best scale is the geometric mean of measure over
/// depth, applied to every delay; the error is the root mean square of each log ratio of prediction to measure. Each
/// graph is left reweighed to the delays fitted. The graphs are read from netlists and loop through no logic without
/// a register; every measure is positive.
pub fn fit(samples: &mut [(TimingGraph, f64)], candidates: &[Delays]) -> Fitted {
    assert!(samples.iter().all(|(_, q)| *q > 0.0), "every measured depth is positive");
    let depths = |samples: &mut [(TimingGraph, f64)], d: &Delays| -> Vec<f64> {
        samples
            .iter_mut()
            .map(|(g, _)| {
                g.reweigh(d);
                g.depth().expect("a sample loops through no logic without a register").depth as f64
            })
            .collect()
    };
    let logs = |samples: &[(TimingGraph, f64)], depth: &[f64]| -> Vec<f64> { samples.iter().zip(depth).map(|((_, q), d)| (q * 100.0 / d).ln()).collect() };
    let rms = |v: &[f64], centre: f64| (v.iter().map(|l| (l - centre).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let mut best: Option<(f64, Delays)> = None;
    for c in candidates {
        let depth = depths(samples, c);
        if depth.iter().any(|&d| d <= 0.0) {
            continue;
        }
        let l = logs(samples, &depth);
        let centre = l.iter().sum::<f64>() / l.len() as f64;
        let error = rms(&l, centre);
        if best.map_or(true, |(e, _)| error < e - 1e-12) {
            let s = centre.exp();
            let scaled = |x: i64| (x as f64 * s).round() as i64;
            best = Some((error, Delays { lut: scaled(c.lut), carry: scaled(c.carry), dsp: scaled(c.dsp), memory: scaled(c.memory) }));
        }
    }
    let (_, delays) = best.expect("a candidate under which every sample has depth");
    let depth = depths(samples, &delays);
    let error = rms(&logs(samples, &depth), 0.0);
    Fitted { delays, predicted: depth.iter().map(|d| d / 100.0).collect(), error }
}

/// The model's depth and area beside Quartus's, build by build: what `timing_calibrate` records, never edited by hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub rows: Vec<CalibrationRow>,
    /// The builds whose Verilog the model could not map, each with why.
    #[serde(default)]
    pub refused: Vec<RefusedBuild>,
    /// The ALUTs the model predicts from an instance's area, fitted to every instance's ([`fit_area`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<AreaFit>,
}

/// ALUTs from an instance's [`Area`]: `scale × (luts + carry × carry bits)`, fitted to the `instances` of at least
/// `floor` ALUTs, `error` the root mean square of the log of each prediction over Quartus's count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaFit {
    pub scale: f64,
    pub carry: f64,
    pub floor: u64,
    pub instances: usize,
    pub error: f64,
}

impl AreaFit {
    /// The ALUTs `area` is predicted to take.
    pub fn aluts(&self, area: &Area) -> f64 {
        self.scale * (area.luts as f64 + self.carry * area.carry as f64)
    }
}

/// The carry weights [`fit_area`] searches against a LUT's: 0 to 3 in twentieths.
pub const AREA_CARRIES: [f64; 61] = {
    let mut k = [0.0; 61];
    let mut i = 0;
    while i < 61 {
        k[i] = i as f64 * 0.05;
        i += 1;
    }
    k
};

/// The area fit of `samples` (an instance's area, Quartus's ALUTs) with at least `floor` ALUTs: of the `carries`, the
/// weight under which the log ratios of Quartus's count over `luts + weight × carry` spread least, the scale their
/// geometric mean. Small instances are left out: a few LUTs either way is most of them.
pub fn fit_area(samples: &[(Area, u64)], floor: u64, carries: &[f64]) -> AreaFit {
    let large: Vec<&(Area, u64)> = samples.iter().filter(|(a, aluts)| *aluts >= floor && a.luts + a.carry > 0).collect();
    let mut best: Option<AreaFit> = None;
    for &carry in carries {
        let logs: Vec<f64> = large.iter().map(|(a, aluts)| (*aluts as f64 / (a.luts as f64 + carry * a.carry as f64)).ln()).collect();
        let centre = logs.iter().sum::<f64>() / logs.len().max(1) as f64;
        let error = (logs.iter().map(|l| (l - centre).powi(2)).sum::<f64>() / logs.len().max(1) as f64).sqrt();
        if best.as_ref().is_none_or(|b| error < b.error - 1e-12) {
            best = Some(AreaFit { scale: centre.exp(), carry, floor, instances: large.len(), error });
        }
    }
    best.expect("a carry weight to try")
}

/// A synthesis the model could not map: its Verilog by SHA-256 and what refused it (yosys's error line, or the
/// timing graph's).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefusedBuild {
    pub build: String,
    pub verilog_sha256: String,
    pub why: String,
}

/// One synthesis: its Verilog by SHA-256, the delays the model used, and the deepest path through each instance,
/// as Quartus reported it (logic levels) and as the model predicts it (the model's unit).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRow {
    pub build: String,
    pub top: String,
    pub verilog_sha256: String,
    pub delays: Delays,
    pub instances: Vec<CalibratedInstance>,
}

/// An instance both report: its path (`""` for the top), Quartus's Max Depth and the model's depth, and what it takes
/// as Quartus's synthesis counted it beside what the model maps it to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibratedInstance {
    pub path: String,
    pub quartus: f64,
    pub predicted: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<InstanceArea>,
}

/// An instance's area two ways: Quartus's synthesis hierarchy, and the timing graph's [`Area`] of its subtree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceArea {
    pub quartus: QuartusArea,
    pub model: Area,
}

/// What Quartus's synthesis hierarchy counts for an instance and everything under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuartusArea {
    pub aluts: u64,
    pub registers: u64,
    pub dsp: u64,
    pub memory_bits: u64,
}

impl CalibrationRow {
    /// The top's figures.
    pub fn top(&self) -> Option<&CalibratedInstance> {
        self.instances.iter().find(|i| i.path.is_empty())
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/timing_graph/tests.rs"]
mod tests;
