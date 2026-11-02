//! Verilog to and-inverter graphs, through yosys: the bridge that puts any RTL the compiler prints in front of the
//! certified equivalence engine ([`logicaffeine_proof::sweep`], [`logicaffeine_proof::seq_equiv`]).
//!
//! [`synthesize`] elaborates one top module with its parameters set ([`script`]), flattens it, maps memories to
//! flip-flops, and writes two views: the word-level netlist as yosys JSON, read here for its clock domains, and the
//! bit-level circuit as ASCII AIGER, read by [`Aig::parse`]. Nothing in the script rewrites logic: no `opt_expr`, no
//! `abc`; every multiplier is lowered by one fixed map (`CANONICAL_MUL`), so a product is the same circuit in every
//! design that computes it; cells computing one thing from the same signals are merged first (`opt_merge`), once
//! every undefined bit is a cell of its own, so a design computing a product many times lowers it once. An undefined
//! bit (`'x`, an unassigned branch) becomes a free input (`setundef -anyseq`), so a proof holds
//! for every value it could take and never for a value an optimizer chose; [`synthesize_reading`] with
//! [`Undefined::Zero`] reads every one as 0 instead (`setundef -zero`), the source as a circuit that fixed them does; asynchronous resets become synchronous
//! (`async2sync`), the cycle-level reading every flip-flop then has on the one global clock the AIGER latches tick
//! on. That reading is exact only for a design with a single clock and no level-sensitive latches, so a design with
//! several clocks or any latch is refused by name ([`NetlistError::Clocks`], [`NetlistError::LevelLatches`]) rather
//! than modeled wrongly; check such a design one clock domain at a time.
//!
//! yosys is `LOGOS_YOSYS` when set, else `yosys` on `PATH`. Each run works in its own directory under the system
//! temporary directory and removes it.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use logicaffeine_proof::aig::{Aig, AigerError};

/// A synthesized module: its bit-level circuit, its word-level yosys JSON, and its clock domains.
#[derive(Clone, Debug)]
pub struct Netlist {
    pub aig: Aig,
    pub json: String,
    pub domains: Domains,
}

/// The sequential elements of a flattened module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domains {
    pub flops: usize,
    /// Distinct clock nets the flip-flops sample on.
    pub clocks: usize,
    /// Level-sensitive latches.
    pub latches: usize,
}

/// Why a module did not become an and-inverter graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetlistError {
    /// yosys could not be started.
    Unavailable(String),
    /// yosys refused the design; the end of its log.
    Yosys { log: String },
    Io(String),
    /// The JSON netlist is not a yosys netlist.
    Json(String),
    /// Flip-flops on this many clocks: one global clock cannot model them.
    Clocks(usize),
    /// This many level-sensitive latches: a flip-flop cannot model them.
    LevelLatches(usize),
    Aiger(AigerError),
    /// The RTLIL netlist is not one yosys writes.
    Rtlil(String),
}

impl std::fmt::Display for NetlistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetlistError::Unavailable(why) => write!(f, "yosys could not run: {why}"),
            NetlistError::Yosys { log } => write!(f, "yosys refused the design:\n{log}"),
            NetlistError::Io(why) => write!(f, "{why}"),
            NetlistError::Json(why) => write!(f, "the yosys JSON netlist: {why}"),
            NetlistError::Clocks(n) => write!(f, "flip-flops on {n} clocks; check one clock domain at a time"),
            NetlistError::LevelLatches(n) => write!(f, "{n} level-sensitive latches"),
            NetlistError::Aiger(e) => write!(f, "{e}"),
            NetlistError::Rtlil(why) => write!(f, "the yosys RTLIL netlist: {why}"),
        }
    }
}

impl std::error::Error for NetlistError {}

/// The map every `$mul` is lowered by before the rest of the design: operands extended to the product's width by their
/// own signedness, then summed row by row in a fixed ripple order. yosys's own lowering (`alumacc`, `maccmap`) orders a
/// product's partial sums by its internal hashing, so one product in two designs becomes two different adder trees
/// and proving them one is the hard problem of multiplier equivalence; lowered here, a product is the same circuit
/// wherever its widths and signedness are, and two spellings of it are one structure.
pub(crate) const CANONICAL_MUL: &str = r#"(* techmap_celltype = "$mul" *)
module _logos_canonical_mul (A, B, Y);
  parameter A_SIGNED = 0;
  parameter B_SIGNED = 0;
  parameter A_WIDTH = 1;
  parameter B_WIDTH = 1;
  parameter Y_WIDTH = 1;
  input [A_WIDTH-1:0] A;
  input [B_WIDTH-1:0] B;
  output [Y_WIDTH-1:0] Y;
  wire [Y_WIDTH-1:0] a, b;
  generate
    if (A_SIGNED) begin : a_signed
      assign a = $signed(A);
    end else begin : a_unsigned
      assign a = A;
    end
    if (B_SIGNED) begin : b_signed
      assign b = $signed(B);
    end else begin : b_unsigned
      assign b = B;
    end
  endgenerate
  wire [Y_WIDTH*(Y_WIDTH+1)-1:0] acc;
  assign acc[Y_WIDTH-1:0] = {Y_WIDTH{1'b0}};
  genvar i;
  generate
    for (i = 0; i < Y_WIDTH; i = i + 1) begin : row
      assign acc[(i+1)*Y_WIDTH +: Y_WIDTH] = acc[i*Y_WIDTH +: Y_WIDTH] + ({Y_WIDTH{a[i]}} & (b << i));
    end
  endgenerate
  assign Y = acc[Y_WIDTH*Y_WIDTH +: Y_WIDTH];
endmodule
"#;

/// What an undefined bit of the source is in the circuit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undefined {
    /// A free input: a proof holds for every value it could take.
    Free,
    /// 0, as a circuit that fixed every undefined bit at 0 computes.
    Zero,
}

/// The yosys script that elaborates `top` with `params` and writes `design.json` and `design.aag`, every undefined
/// bit a free input. The design is read deferred: a module is elaborated only as the hierarchy instantiates it, with
/// its instance's parameters, never at its defaults on the way in.
pub fn script(top: &str, params: &[(&str, &str)]) -> String {
    script_reading(top, params, Undefined::Free)
}

/// [`script`], each undefined bit read as `undefined` says.
pub fn script_reading(top: &str, params: &[(&str, &str)], undefined: Undefined) -> String {
    script_cut(top, params, undefined, &[])
}

/// [`script_reading`] with every instance of the `units` modules cut out: each module read from `units.v` as its ports
/// alone (`read_verilog -lib`, its body never elaborated; a module no unit is named is an error), and once the design
/// is flat and written, each instance turned
/// into ports of the top (`expose -evert`) — its outputs inputs named `<instance>.<port>[bit]`, its inputs outputs of
/// the same names.
pub fn script_cut(top: &str, params: &[(&str, &str)], undefined: Undefined, units: &[&str]) -> String {
    let mut hierarchy = format!("hierarchy -check -top {top}");
    for (name, value) in params {
        hierarchy.push_str(&format!(" -chparam {name} {value}"));
    }
    let setundef = match undefined {
        Undefined::Free => "setundef -anyseq",
        Undefined::Zero => "setundef -zero",
    };
    let mut steps: Vec<String> = vec!["verilog_defaults -add -defer".into(), "read_verilog -sv design.v".into(), "verilog_defaults -clear".into()];
    if !units.is_empty() {
        steps.push("read_verilog -sv -lib units.v".into());
    }
    for u in units {
        steps.push(format!("select -assert-any ={u}"));
    }
    let cut = (!units.is_empty()).then(|| format!("expose -evert {}", units.iter().map(|u| format!("t:{u}")).collect::<Vec<_>>().join(" ")));
    steps.extend(
        [
        &hierarchy,
        "proc",
        "flatten",
        "opt_clean",
        "memory_map",
        "opt_clean",
        "write_json design.json",
        ]
        .map(str::to_string),
    );
    if !units.is_empty() {
        steps.push("write_rtlil design.il".into());
    }
    steps.extend(cut);
    steps.extend(
        [
        setundef,
        "async2sync",
        "opt_merge",
        "techmap -map logos_mul.v",
        "techmap",
        // A part-select at a moving offset is a shift whose bits past its operand are undefined only once lowered.
        setundef,
        "opt_clean",
        "dffunmap",
        "aigmap",
        "opt_clean",
        "write_aiger -ascii -symbols design.aag",
        ]
        .map(str::to_string),
    );
    steps.join("\n") + "\n"
}

const FLOPS: &[&str] = &[
    "$dff", "$dffe", "$adff", "$adffe", "$sdff", "$sdffe", "$sdffce", "$dffsr", "$dffsre", "$aldff", "$aldffe",
];
const LATCHES: &[&str] = &["$dlatch", "$adlatch", "$dlatchsr"];

/// The flip-flops, their distinct clock nets and the level latches of a yosys JSON netlist. A `$ff` samples on the
/// implicit global clock, counted as one clock of its own.
pub(crate) fn domains(json: &str) -> Result<Domains, NetlistError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| NetlistError::Json(e.to_string()))?;
    let modules = v
        .get("modules")
        .and_then(|m| m.as_object())
        .ok_or_else(|| NetlistError::Json("no `modules` object".into()))?;
    let mut clocks = BTreeSet::new();
    let (mut flops, mut latches) = (0, 0);
    for module in modules.values() {
        let Some(cells) = module.get("cells").and_then(|c| c.as_object()) else { continue };
        for cell in cells.values() {
            let ty = cell.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty == "$ff" {
                flops += 1;
                clocks.insert("$global".to_string());
            } else if FLOPS.contains(&ty) {
                flops += 1;
                if let Some(clk) = cell.get("connections").and_then(|c| c.get("CLK")) {
                    clocks.insert(clk.to_string());
                }
            } else if LATCHES.contains(&ty) {
                latches += 1;
            }
        }
    }
    Ok(Domains { flops, clocks: clocks.len(), latches })
}

/// What drives a net that clocks registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockDriver {
    /// An input port of the top: a clock a network brings in.
    Port,
    /// A register's output: a divided clock.
    Register,
    /// Logic, or a constant: a gated or selected clock.
    Logic,
}

/// A net that clocks registers or memory ports: its name, what drives it, and the register bits it clocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockSource {
    pub net: String,
    pub driver: ClockDriver,
    pub registers: usize,
}

/// Every net that clocks a register or a memory port of `top` with its parameters set, flattened: what drives it and
/// the register bits it clocks, the nets no port drives first, then by the bits they clock. A clock a register or
/// logic makes inside a design reaches no global network unless the fitter gives it one, and a persona's region
/// cannot; it rides general routing, its skew paid for in the delay the router adds for hold, so a design is refused
/// such a clock before it is fitted.
pub fn clock_sources(verilog: &str, top: &str, params: &[(&str, &str)]) -> Result<Vec<ClockSource>, NetlistError> {
    let mut hierarchy = format!("hierarchy -check -top {top}");
    for (name, value) in params {
        hierarchy.push_str(&format!(" -chparam {name} {value}"));
    }
    let script = ["read_verilog -sv design.v", &hierarchy, "proc", "flatten", "opt_clean", "write_json clocks.json"].join("\n") + "\n";
    let run = yosys_run(&reachable_modules(verilog, &[top]), &script, &["clocks.json"])?;
    let json = match (&run.files[0], run.ok) {
        (Some(bytes), true) => String::from_utf8_lossy(bytes).into_owned(),
        _ => return Err(run.refused()),
    };
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| NetlistError::Json(e.to_string()))?;
    let module = v.get("modules").and_then(|m| m.get(top)).ok_or_else(|| NetlistError::Json(format!("no module {top}")))?;
    let bits = |x: &serde_json::Value| -> Vec<serde_json::Value> { x.as_array().cloned().unwrap_or_default() };
    let mut name: BTreeMap<String, (bool, usize, String)> = BTreeMap::new();
    if let Some(nets) = module.get("netnames").and_then(|n| n.as_object()) {
        for (n, net) in nets {
            let hidden = net.get("hide_name").and_then(|h| h.as_u64()) == Some(1) || n.starts_with('$');
            let all = bits(&net["bits"]);
            for (i, b) in all.iter().enumerate() {
                let shown = if all.len() == 1 { n.clone() } else { format!("{n}[{i}]") };
                let rank = (hidden, shown.len(), shown);
                let key = b.to_string();
                if name.get(&key).map_or(true, |have| rank < *have) {
                    name.insert(key, rank);
                }
            }
        }
    }
    let mut ports = BTreeSet::new();
    if let Some(p) = module.get("ports").and_then(|p| p.as_object()) {
        for port in p.values().filter(|port| port["direction"] == "input") {
            ports.extend(bits(&port["bits"]).iter().map(|b| b.to_string()));
        }
    }
    let cells: Vec<&serde_json::Value> = module.get("cells").and_then(|c| c.as_object()).map(|c| c.values().collect()).unwrap_or_default();
    let mut driver: BTreeMap<String, ClockDriver> = BTreeMap::new();
    for cell in &cells {
        let ty = cell["type"].as_str().unwrap_or("");
        let made = if FLOPS.contains(&ty) || LATCHES.contains(&ty) || ty == "$ff" { ClockDriver::Register } else { ClockDriver::Logic };
        if let (Some(dirs), Some(conns)) = (cell.get("port_directions").and_then(|d| d.as_object()), cell.get("connections").and_then(|c| c.as_object())) {
            for (pin, dir) in dirs {
                if dir == "output" {
                    for b in conns.get(pin).map(bits).unwrap_or_default() {
                        driver.insert(b.to_string(), made);
                    }
                }
            }
        }
    }
    let mut clocked: BTreeMap<String, usize> = BTreeMap::new();
    for cell in &cells {
        let ty = cell["type"].as_str().unwrap_or("");
        let pins: &[&str] = if FLOPS.contains(&ty) {
            &["CLK"]
        } else if ty.starts_with("$mem") {
            &["CLK", "RD_CLK", "WR_CLK"]
        } else {
            continue;
        };
        let width = if FLOPS.contains(&ty) { cell["parameters"].get("WIDTH").map(parameter).unwrap_or(1) as usize } else { 0 };
        for pin in pins {
            for b in cell["connections"].get(*pin).map(bits).unwrap_or_default() {
                if b.is_string() {
                    continue;
                }
                *clocked.entry(b.to_string()).or_insert(0) += width;
            }
        }
    }
    let mut out: Vec<ClockSource> = clocked
        .into_iter()
        .map(|(bit, registers)| {
            let source = if ports.contains(&bit) { ClockDriver::Port } else { driver.get(&bit).copied().unwrap_or(ClockDriver::Logic) };
            let net = name.get(&bit).map(|(_, _, n)| n.clone()).unwrap_or_else(|| format!("net{bit}"));
            ClockSource { net, driver: source, registers }
        })
        .collect();
    out.sort_by(|a, b| (a.driver == ClockDriver::Port).cmp(&(b.driver == ClockDriver::Port)).then(b.registers.cmp(&a.registers)).then_with(|| a.net.cmp(&b.net)));
    Ok(out)
}

/// A yosys parameter: a binary string or a number.
fn parameter(v: &serde_json::Value) -> u64 {
    match v {
        serde_json::Value::String(s) => u64::from_str_radix(s.trim_start_matches('0').trim(), 2).unwrap_or(0),
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

/// Whether one global clock models the design exactly.
pub(crate) fn admit(d: Domains) -> Result<(), NetlistError> {
    if d.latches > 0 {
        return Err(NetlistError::LevelLatches(d.latches));
    }
    if d.clocks > 1 {
        return Err(NetlistError::Clocks(d.clocks));
    }
    Ok(())
}

fn yosys() -> String {
    std::env::var("LOGOS_YOSYS").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "yosys".to_string())
}

fn scratch() -> Result<PathBuf, NetlistError> {
    static RUN: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "logos-netlist-{}-{nanos}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| NetlistError::Io(format!("{}: {e}", dir.display())))?;
    Ok(dir)
}

/// One yosys run: whether it succeeded, the end of its log, and the files it was asked for, each `None` when it
/// did not write it.
pub(crate) struct Run {
    pub ok: bool,
    pub log: String,
    pub files: Vec<Option<Vec<u8>>>,
}

impl Run {
    /// The refusal a failed run reports.
    pub(crate) fn refused(&self) -> NetlistError {
        NetlistError::Yosys { log: self.log.clone() }
    }
}

/// Run `script` on `verilog` (read as `design.v`) in a scratch directory of its own, removed after; then read back
/// each of `files`.
pub(crate) fn yosys_run(verilog: &str, script: &str, files: &[&str]) -> Result<Run, NetlistError> {
    yosys_run_with(&[("design.v", verilog)], script, files)
}

/// [`yosys_run`] with the input files `inputs` (each a name and its text) written beside the script.
pub(crate) fn yosys_run_with(inputs: &[(&str, &str)], script: &str, files: &[&str]) -> Result<Run, NetlistError> {
    let dir = scratch()?;
    let result = yosys_in(&dir, inputs, script, files);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn yosys_in(dir: &Path, inputs: &[(&str, &str)], script: &str, files: &[&str]) -> Result<Run, NetlistError> {
    let io = |what: &str, e: std::io::Error| NetlistError::Io(format!("{what}: {e}"));
    for (name, text) in inputs {
        std::fs::write(dir.join(name), text).map_err(|e| io(name, e))?;
    }
    std::fs::write(dir.join("design.ys"), script).map_err(|e| io("design.ys", e))?;
    std::fs::write(dir.join("logos_mul.v"), CANONICAL_MUL).map_err(|e| io("logos_mul.v", e))?;
    let bin = yosys();
    let out = Command::new(&bin)
        .args(["-q", "-s", "design.ys"])
        .current_dir(dir)
        .output()
        .map_err(|e| NetlistError::Unavailable(format!("{bin}: {e}")))?;
    let mut log = String::from_utf8_lossy(&out.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    let tail = log.char_indices().rev().nth(4000).map_or(0, |(i, _)| i);
    Ok(Run {
        ok: out.status.success(),
        log: log[tail..].to_string(),
        files: files.iter().map(|f| std::fs::read(dir.join(f)).ok()).collect(),
    })
}

/// The modules of a Verilog text, in order: each one's name and its text from its `module` line to its `endmodule`. None
/// for a text [`pieces`] cannot part.
pub(crate) fn modules(verilog: &str) -> Option<Vec<(String, &str)>> {
    Some(pieces(verilog)?.into_iter().filter_map(|(name, text)| Some((name?, text))).collect())
}

/// A Verilog text in pieces, in order: each module (its name, its text from its `module` line to its `endmodule`) and
/// each line outside a module (no name). None for a text whose modules this cannot tell apart: a module line with no
/// name, a module opened inside another or left open, a name defined twice.
fn pieces(verilog: &str) -> Option<Vec<(Option<String>, &str)>> {
    fn starts(t: &str) -> Option<&str> {
        t.strip_prefix("module ").or_else(|| t.strip_prefix("module\t")).map(str::trim_start)
    }
    let mut pieces: Vec<(Option<String>, &str)> = Vec::new();
    let mut open: Option<(String, usize)> = None;
    let mut at = 0;
    for line in verilog.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let t = line.trim_start();
        match open.take() {
            None => match starts(t) {
                Some(rest) => {
                    let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$').collect();
                    if name.is_empty() {
                        return None;
                    }
                    if t.trim_end().ends_with("endmodule") {
                        pieces.push((Some(name), line));
                    } else {
                        open = Some((name, start));
                    }
                }
                None => pieces.push((None, line)),
            },
            Some((name, from)) => {
                if starts(t).is_some() {
                    return None;
                }
                if t.starts_with("endmodule") {
                    pieces.push((Some(name), &verilog[from..at]));
                } else {
                    open = Some((name, from));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    (open.is_none() && pieces.iter().filter_map(|(n, _)| n.as_deref()).all(|n| seen.insert(n))).then_some(pieces)
}

/// `verilog` parted into the modules `names` names and the rest (every other module and every line outside a module),
/// each in the text's order: the units a cut reads as their ports alone, apart from the design it elaborates. None for a
/// text [`pieces`] cannot part.
pub(crate) fn split_modules(verilog: &str, names: &[&str]) -> Option<(String, String)> {
    let (mut rest, mut named) = (String::new(), String::new());
    for (name, text) in pieces(verilog)? {
        if name.as_deref().is_some_and(|n| names.contains(&n)) {
            named.push_str(text);
        } else {
            rest.push_str(text);
        }
    }
    Some((rest, named))
}

/// Two instances of a module and the bits between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub a: String,
    pub b: String,
    pub bits: u64,
}

/// A module's instances as a floorplan's wires: every instance, each pair of them and the bits between them, and each
/// instance and the bits between it and the module's own ports, in name order, no wire of no bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    pub instances: Vec<String>,
    pub links: Vec<Link>,
    pub ports: Vec<(String, u64)>,
}

/// One end of a wire: an instance, or the module's own ports (its inputs the nets it drives, its outputs those it reads).
struct End {
    name: String,
    ins: BTreeSet<u64>,
    outs: BTreeSet<u64>,
    nets: BTreeSet<u64>,
}

/// The module's own logic as a graph of its nets: each glue cell's inputs and outputs, and the cells reading and driving
/// each net, indexed by the net's number.
struct Glue {
    cells: Vec<(Vec<u64>, Vec<u64>)>,
    readers: Vec<Vec<usize>>,
    drivers: Vec<Vec<usize>>,
}

impl Glue {
    /// The nets reached from `from` through one glue cell or more: forward, a cell's every output from any of its inputs;
    /// backward, its every input from any of its outputs.
    fn reach(&self, from: &BTreeSet<u64>, forward: bool) -> Vec<bool> {
        let via = if forward { &self.readers } else { &self.drivers };
        let mut seen = vec![false; self.cells.len()];
        let mut reached = vec![false; via.len()];
        let mut stack: Vec<usize> = from.iter().flat_map(|&n| via[n as usize].iter().copied()).collect();
        while let Some(c) = stack.pop() {
            if std::mem::replace(&mut seen[c], true) {
                continue;
            }
            let (ins, outs) = &self.cells[c];
            for &n in if forward { outs } else { ins } {
                if !std::mem::replace(&mut reached[n as usize], true) {
                    stack.extend(via[n as usize].iter().copied());
                }
            }
        }
        reached
    }
}

/// The instances of `module` in `verilog` as a floorplan's wires: each pair of its instances and the bits between them,
/// and each instance and the bits between it and the module's own ports. The modules `through` names are dissolved into
/// it (`flatten`), so their instances are the module's, named by path (`fast|unit`; a generate block's
/// `lane_port[0].port`); every other module is read by its ports alone (`read_verilog -lib`), so nothing under an instance
/// is elaborated. The bits between two ends are the nets one drives and the other reads, and what each drives into the
/// other through the module's own logic (its registers, multiplexers and the rest, one cell or more): the fewer of the
/// nets leaving the one and the nets arriving at the other, so a bus through a register stage counts its width and a
/// select steering a bus one bit. A net two ends only read joins neither to the other: a broadcast is a star from its
/// driver, not a wire between every two of its readers. A glue cell's clock pins carry no bits: the clock network is no
/// wire a floorplan shortens.
pub fn module_graph(verilog: &str, module: &str, through: &[&str]) -> Result<Graph, NetlistError> {
    let unparted = || NetlistError::Json("the Verilog's modules do not part".into());
    let defined: HashSet<String> = pieces(verilog).ok_or_else(unparted)?.into_iter().filter_map(|(n, _)| n).collect();
    let mut own_names = vec![module];
    own_names.extend_from_slice(through);
    if let Some(absent) = own_names.iter().find(|n| !defined.contains(**n)) {
        return Err(NetlistError::Json(format!("no module {absent} in the Verilog")));
    }
    let (rest, own) = split_modules(verilog, &own_names).ok_or_else(unparted)?;
    let script =
        format!("read_verilog -sv -lib rest.v\nread_verilog -sv own.v\nhierarchy -top {module}\nproc\nflatten\nmemory_collect\nwrite_json graph.json\n");
    let run = yosys_run_with(&[("rest.v", &rest), ("own.v", &own)], &script, &["graph.json"])?;
    if !run.ok {
        return Err(run.refused());
    }
    let json = run.files[0].as_ref().ok_or_else(|| NetlistError::Io("yosys wrote no graph.json".into()))?;
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| NetlistError::Json(e.to_string()))?;
    let top = &v["modules"][module];
    let cells = top["cells"].as_object().ok_or_else(|| NetlistError::Json(format!("{module} has no cells")))?;
    let bits = |b: &serde_json::Value| -> Vec<u64> { b.as_array().into_iter().flatten().filter_map(|x| x.as_u64()).collect() };
    let mut ends: Vec<End> = Vec::new();
    let mut glue: Vec<(Vec<u64>, Vec<u64>)> = Vec::new();
    for (id, cell) in cells {
        let kind = cell["type"].as_str().unwrap_or_default();
        let (mut ins, mut outs) = (BTreeSet::new(), BTreeSet::new());
        for (port, connected) in cell["connections"].as_object().into_iter().flatten() {
            let direction = cell["port_directions"][port.as_str()]
                .as_str()
                .ok_or_else(|| NetlistError::Json(format!("cell {id}'s port {port} has no direction")))?;
            let clock = kind.starts_with('$') && matches!(port.as_str(), "CLK" | "RD_CLK" | "WR_CLK");
            if matches!(direction, "input" | "inout") && !clock {
                ins.extend(bits(connected));
            }
            if matches!(direction, "output" | "inout") {
                outs.extend(bits(connected));
            }
        }
        if kind.starts_with('$') {
            glue.push((ins.into_iter().collect(), outs.into_iter().collect()));
        } else {
            let name = match cell["attributes"]["hdlname"].as_str() {
                Some(path) => path.split(' ').collect::<Vec<_>>().join("|"),
                None => json_name(id).to_string(),
            };
            let nets = ins.union(&outs).copied().collect();
            ends.push(End { name, ins, outs, nets });
        }
    }
    ends.sort_by(|a, b| a.name.cmp(&b.name));
    let (mut ins, mut outs) = (BTreeSet::new(), BTreeSet::new());
    for port in top["ports"].as_object().into_iter().flatten().map(|(_, p)| p) {
        match port["direction"].as_str() {
            Some("input") => outs.extend(bits(&port["bits"])),
            Some("output") => ins.extend(bits(&port["bits"])),
            _ => {
                ins.extend(bits(&port["bits"]));
                outs.extend(bits(&port["bits"]));
            }
        }
    }
    let nets = ins.union(&outs).copied().collect();
    let ports = End { name: String::new(), ins, outs, nets };
    let span = ends
        .iter()
        .chain([&ports])
        .flat_map(|e| e.nets.iter())
        .chain(glue.iter().flat_map(|(i, o)| i.iter().chain(o)))
        .max()
        .map_or(0, |&n| n as usize + 1);
    let (mut readers, mut drivers) = (vec![Vec::new(); span], vec![Vec::new(); span]);
    for (c, (i, o)) in glue.iter().enumerate() {
        i.iter().for_each(|&n| readers[n as usize].push(c));
        o.iter().for_each(|&n| drivers[n as usize].push(c));
    }
    let glue = Glue { cells: glue, readers, drivers };
    let reach: Vec<(Vec<bool>, Vec<bool>)> =
        ends.iter().chain([&ports]).map(|e| (glue.reach(&e.outs, true), glue.reach(&e.ins, false))).collect();
    // From end `a` into end `b` through the module's logic: the fewer of a's nets leaving it that reach b and b's nets
    // arriving that a reaches, a net the two share counted once, as shared.
    let flow = |(a, ea): (usize, &End), (b, eb): (usize, &End)| -> u64 {
        let leaving = ea.outs.iter().filter(|&&n| reach[b].1[n as usize] && !eb.nets.contains(&n)).count();
        let arriving = eb.ins.iter().filter(|&&n| reach[a].0[n as usize] && !ea.nets.contains(&n)).count();
        leaving.min(arriving) as u64
    };
    let between = |a: usize, ea: &End, b: usize, eb: &End| -> u64 {
        let direct = ea.nets.intersection(&eb.nets).filter(|n| ea.outs.contains(n) || eb.outs.contains(n)).count();
        direct as u64 + flow((a, ea), (b, eb)) + flow((b, eb), (a, ea))
    };
    let mut links = Vec::new();
    for (i, a) in ends.iter().enumerate() {
        for (j, b) in ends.iter().enumerate().skip(i + 1) {
            let bits = between(i, a, j, b);
            if bits > 0 {
                links.push(Link { a: a.name.clone(), b: b.name.clone(), bits });
            }
        }
    }
    let p = ends.len();
    let ports = ends.iter().enumerate().map(|(i, e)| (e.name.clone(), between(i, e, p, &ports))).filter(|(_, b)| *b > 0).collect();
    Ok(Graph { instances: ends.iter().map(|e| e.name.clone()).collect(), links, ports })
}

/// The modules of `verilog` that `roots` reach, each through the module names its text mentions, and every line outside
/// a module, in the text's order: what yosys elaborates of a library for a top, handed without the rest (yosys reads,
/// and so elaborates, every module it is given). A module named only in a comment on the way is kept, which costs time
/// and never changes the design. A root no module is named for, a module the text leaves open or names twice leaves the
/// text whole, for yosys to judge.
pub(crate) fn reachable_modules<'a>(verilog: &'a str, roots: &[&str]) -> Cow<'a, str> {
    let Some(pieces) = pieces(verilog) else { return Cow::Borrowed(verilog) };
    let index: HashMap<&str, usize> = pieces.iter().enumerate().filter_map(|(i, (name, _))| name.as_deref().map(|n| (n, i))).collect();
    let Some(mut stack) = roots.iter().map(|r| index.get(r).copied()).collect::<Option<Vec<usize>>>() else {
        return Cow::Borrowed(verilog);
    };
    let mut kept: HashSet<usize> = HashSet::new();
    while let Some(i) = stack.pop() {
        if !kept.insert(i) {
            continue;
        }
        let text = pieces[i].1.as_bytes();
        let mut k = 0;
        while k < text.len() {
            if text[k].is_ascii_alphabetic() || text[k] == b'_' {
                let from = k;
                while k < text.len() && (text[k].is_ascii_alphanumeric() || text[k] == b'_' || text[k] == b'$') {
                    k += 1;
                }
                if let Some(&j) = std::str::from_utf8(&text[from..k]).ok().and_then(|w| index.get(w)) {
                    stack.push(j);
                }
            } else {
                k += 1;
            }
        }
    }
    if kept.len() == index.len() {
        return Cow::Borrowed(verilog);
    }
    Cow::Owned(pieces.iter().enumerate().filter(|(i, (name, _))| name.is_none() || kept.contains(i)).map(|(_, (_, text))| *text).collect())
}

/// Synthesize `top` of `verilog` with `params` set into an and-inverter graph, every undefined bit a free input.
pub fn synthesize(verilog: &str, top: &str, params: &[(&str, &str)]) -> Result<Netlist, NetlistError> {
    synthesize_reading(verilog, top, params, Undefined::Free)
}

/// [`synthesize`], each undefined bit read as `undefined` says.
pub fn synthesize_reading(verilog: &str, top: &str, params: &[(&str, &str)], undefined: Undefined) -> Result<Netlist, NetlistError> {
    let run = yosys_run(&reachable_modules(verilog, &[top]), &script_reading(top, params, undefined), &["design.json", "design.aag"])?;
    let json = run.files[0].as_ref().map(|b| String::from_utf8_lossy(b).into_owned());
    let domains = match &json {
        Some(j) => {
            let d = domains(j)?;
            admit(d)?;
            Some(d)
        }
        None => None,
    };
    if !run.ok {
        return Err(run.refused());
    }
    let (Some(json), Some(domains)) = (json, domains) else {
        return Err(NetlistError::Io("yosys wrote no design.json".into()));
    };
    let aag = run.files[1].as_ref().ok_or_else(|| NetlistError::Io("yosys wrote no design.aag".into()))?;
    let aig = Aig::parse(aag).map_err(NetlistError::Aiger)?;
    Ok(Netlist { aig, json, domains })
}

/// An instance of a unit cut out of a design ([`synthesize_cut`]): its flattened name, its module, the parameters it
/// gives its module (by name, each value in one spelling, `rtlil_value`; every one it leaves is the module's
/// default), and its input and output ports with their widths, in the order the netlist lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitInstance {
    pub name: String,
    pub module: String,
    pub parameters: Vec<(String, String)>,
    pub operands: Vec<(String, usize)>,
    pub results: Vec<(String, usize)>,
}

impl UnitInstance {
    /// The function the instance computes: its module, then the parameters it gives, `<module> #(K=32'sd1, W=8'd3)`.
    /// Two instances of one function elaborate to one circuit; an instance giving a value equal to its module's
    /// default is another function's name, since the netlists yosys writes keep no default's sign.
    pub fn function(&self) -> String {
        if self.parameters.is_empty() {
            return self.module.clone();
        }
        let given: Vec<String> = self.parameters.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{} #({})", self.module, given.join(", "))
    }
}

/// A design with its units cut out: its netlist and the instances, by name.
#[derive(Clone, Debug)]
pub struct CutNetlist {
    pub netlist: Netlist,
    pub units: Vec<UnitInstance>,
}

impl CutNetlist {
    /// The units for [`logicaffeine_proof::units`]: each instance's operand and result bits by the names the cut gave
    /// them, its operands in port order bit by bit with the ports `ignored` left out (a pipelined unit's clock is no
    /// operand), its function ([`UnitInstance::function`]), and its latency as `latency` gives it by function. A port's
    /// bits are every bit the cut exposes for it, past those the instance connects when its module declares the port
    /// wider at its own parameters' defaults.
    pub fn units(&self, latency: &dyn Fn(&str) -> usize, ignored: &[&str]) -> Vec<logicaffeine_proof::units::Unit> {
        let aig = &self.netlist.aig;
        let bits = |inst: &str, ports: &[(String, usize)], exposed: &dyn Fn(&str) -> bool| -> Vec<String> {
            let mut ports: Vec<&(String, usize)> = ports.iter().filter(|(p, _)| !ignored.contains(&p.as_str())).collect();
            ports.sort_by(|a, b| a.0.cmp(&b.0));
            ports
                .iter()
                .flat_map(|(p, width)| {
                    let base = format!("{inst}.{p}");
                    if *width == 1 && !exposed(&format!("{base}[0]")) {
                        return vec![base];
                    }
                    let mut n = *width;
                    while exposed(&format!("{base}[{n}]")) {
                        n += 1;
                    }
                    (0..n).map(|i| format!("{base}[{i}]")).collect()
                })
                .collect()
        };
        self.units
            .iter()
            .map(|u| logicaffeine_proof::units::Unit {
                name: u.name.clone(),
                function: u.function(),
                latency: latency(&u.function()),
                operands: bits(&u.name, &u.operands, &|n| aig.output_index(n).is_some()),
                results: bits(&u.name, &u.results, &|n| aig.input_index(n).is_some()),
            })
            .collect()
    }
}

/// `top` with every instance of the `units` modules cut out ([`script_cut`]), each undefined bit read as `undefined`
/// says: a composition whose units are proven apart, to be proven over its units as functions it knows nothing of.
pub fn synthesize_cut(verilog: &str, top: &str, params: &[(&str, &str)], undefined: Undefined, units: &[&str]) -> Result<CutNetlist, NetlistError> {
    let roots: Vec<&str> = std::iter::once(top).chain(units.iter().copied()).collect();
    let reached = reachable_modules(verilog, &roots);
    let (design, unit_text) = split_modules(&reached, units).unwrap_or_else(|| (reached.to_string(), String::new()));
    let run = yosys_run_with(&[("design.v", &design), ("units.v", &unit_text)], &script_cut(top, params, undefined, units), &["design.json", "design.aag", "design.il"])?;
    if !run.ok {
        return Err(run.refused());
    }
    let json = run.files[0].as_ref().map(|b| String::from_utf8_lossy(b).into_owned()).ok_or_else(|| NetlistError::Io("yosys wrote no design.json".into()))?;
    let domains = domains(&json)?;
    admit(domains)?;
    let aag = run.files[1].as_ref().ok_or_else(|| NetlistError::Io("yosys wrote no design.aag".into()))?;
    let aig = Aig::parse(aag).map_err(NetlistError::Aiger)?;
    let mut instances = unit_instances(&json, units)?;
    if !instances.is_empty() {
        let il = run.files[2].as_ref().map(|b| String::from_utf8_lossy(b).into_owned()).ok_or_else(|| NetlistError::Io("yosys wrote no design.il".into()))?;
        let mut given = given_parameters(&il, units)?;
        for u in &mut instances {
            u.parameters = given.remove(&u.name).ok_or_else(|| NetlistError::Rtlil(format!("no cell {} of the top module", u.name)))?;
        }
    }
    Ok(CutNetlist { netlist: Netlist { aig, json, domains }, units: instances })
}

/// A name as the JSON netlist writes it: a public RTLIL name (`\x`) without its backslash, a private one (`$x`) as is.
fn json_name(id: &str) -> &str {
    id.strip_prefix('\\').unwrap_or(id)
}

/// A parameter's value as an RTLIL netlist writes it, `signed` or `real` as its flags say, in one spelling for one
/// value: bits as `<width>'d<value>` (`'sd` when signed, a negative signed value as `-<width>'sd<magnitude>`), bits
/// past 128 or with any not 0 or 1 as `<width>'b<bits>` (`'sb` when signed), a string as written, quoted, a real
/// `real "<text>"`. A bare integer is a 32-bit constant, as yosys reads one.
pub(crate) fn rtlil_value(signed: bool, real: bool, text: &str) -> Result<String, NetlistError> {
    let bad = || NetlistError::Rtlil(format!("no constant: {text}"));
    if text.starts_with('"') {
        if text.len() < 2 || !text.ends_with('"') {
            return Err(bad());
        }
        return Ok(if real { format!("real {text}") } else { text.to_string() });
    }
    if real {
        return Err(bad());
    }
    let bits: String = match text.split_once('\'') {
        Some((width, bits)) => {
            let width: usize = width.parse().map_err(|_| bad())?;
            if bits.len() != width || !bits.chars().all(|c| "01xzm-".contains(c)) {
                return Err(bad());
            }
            bits.to_string()
        }
        None => format!("{:032b}", text.parse::<i32>().map_err(|_| bad())? as u32),
    };
    let (width, s) = (bits.len(), if signed { "s" } else { "" });
    if width == 0 || width > 128 || !bits.chars().all(|c| c == '0' || c == '1') {
        return Ok(format!("{width}'{s}b{bits}"));
    }
    let value = u128::from_str_radix(&bits, 2).map_err(|_| bad())?;
    if signed && bits.starts_with('1') {
        let mask = if width == 128 { u128::MAX } else { (1u128 << width) - 1 };
        return Ok(format!("-{width}'sd{}", (!value).wrapping_add(1) & mask));
    }
    Ok(format!("{width}'{s}d{value}"))
}

/// The parameters each instance of a `units` module in the top of an RTLIL netlist gives, by instance (named as the
/// JSON netlist names it), each list in name order and each value in one spelling ([`rtlil_value`]). The RTLIL keeps
/// a given value's sign, which the JSON netlist drops.
pub(crate) fn given_parameters(il: &str, units: &[&str]) -> Result<HashMap<String, Vec<(String, String)>>, NetlistError> {
    let mut found: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let (mut marked, mut in_top, mut seen_top) = (false, false, false);
    let mut cell: Option<(String, bool, Vec<(String, String)>)> = None;
    for line in il.lines() {
        let t = line.trim_start();
        if !in_top {
            if t.starts_with("attribute \\top ") {
                marked = true;
            } else if t.starts_with("module ") {
                in_top = marked;
                seen_top |= marked;
                marked = false;
            } else if !t.starts_with("attribute ") {
                marked = false;
            }
            continue;
        }
        if line == "end" {
            if cell.is_some() {
                return Err(NetlistError::Rtlil("the top module ends inside a cell".into()));
            }
            in_top = false;
            continue;
        }
        if let Some(rest) = t.strip_prefix("cell ") {
            if cell.is_some() {
                return Err(NetlistError::Rtlil(format!("a cell inside a cell: {t}")));
            }
            let mut words = rest.split(' ');
            let (Some(kind), Some(name), None) = (words.next(), words.next(), words.next()) else {
                return Err(NetlistError::Rtlil(format!("a cell line: {t}")));
            };
            cell = Some((json_name(name).to_string(), units.contains(&json_name(kind)), Vec::new()));
            continue;
        }
        let Some((name, unit, params)) = cell.as_mut() else { continue };
        if t == "end" {
            if *unit {
                params.sort();
                found.insert(std::mem::take(name), std::mem::take(params));
            }
            cell = None;
        } else if let (true, Some(rest)) = (*unit, t.strip_prefix("parameter ")) {
            let (mut signed, mut real, mut rest) = (false, false, rest);
            loop {
                if let Some(r) = rest.strip_prefix("signed ") {
                    (signed, rest) = (true, r);
                } else if let Some(r) = rest.strip_prefix("real ") {
                    (real, rest) = (true, r);
                } else {
                    break;
                }
            }
            let Some((id, value)) = rest.split_once(' ') else {
                return Err(NetlistError::Rtlil(format!("a parameter with no value: {t}")));
            };
            params.push((json_name(id).to_string(), rtlil_value(signed, real, value)?));
        }
    }
    if !seen_top {
        return Err(NetlistError::Rtlil("no top module".into()));
    }
    Ok(found)
}

/// The instances of the `units` modules in a flattened yosys JSON netlist, by name.
fn unit_instances(json: &str, units: &[&str]) -> Result<Vec<UnitInstance>, NetlistError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| NetlistError::Json(e.to_string()))?;
    let modules = v.get("modules").and_then(|m| m.as_object()).ok_or_else(|| NetlistError::Json("no `modules` object".into()))?;
    let top = modules
        .values()
        .find(|m| m.pointer("/attributes/top").is_some())
        .ok_or_else(|| NetlistError::Json("no top module".into()))?;
    let mut found = Vec::new();
    if let Some(cells) = top.get("cells").and_then(|c| c.as_object()) {
        for (name, cell) in cells {
            let Some(kind) = cell.get("type").and_then(|t| t.as_str()) else { continue };
            if !units.contains(&kind) {
                continue;
            }
            let (mut operands, mut results) = (Vec::new(), Vec::new());
            let directions = cell.get("port_directions").and_then(|d| d.as_object());
            for (port, bits) in cell.get("connections").and_then(|c| c.as_object()).into_iter().flatten() {
                let width = bits.as_array().map_or(0, Vec::len);
                match directions.and_then(|d| d.get(port)).and_then(|d| d.as_str()) {
                    Some("output") => results.push((port.clone(), width)),
                    _ => operands.push((port.clone(), width)),
                }
            }
            found.push(UnitInstance { name: name.clone(), module: kind.to_string(), parameters: Vec::new(), operands, results });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/netlist/tests.rs"]
mod tests;
