//! Quartus Prime reports as data: a compile's flow, the resources it used, the slack of every clock, the retimer's
//! limits, the router's peak wire demand, and the resources and logic depth of every entity, read from the text
//! reports Quartus writes into a project's `output_files`.
//!
//! [`read`] gathers one compile into a [`Build`]: [`parse_flow`] over `<rev>.flow.rpt` (status, version, device,
//! start, and each phase's elapsed time and peak memory), [`parse_summary`] over the fitter's summary (ALMs,
//! registers, memory bits, RAM and DSP blocks as used over total), [`parse_synthesis`] over the synthesis report's
//! resource estimate (with the fixed- and floating-point DSP split), [`parse_sta_summary`] over the timing summary
//! (every analysis of every clock), [`parse_retime`] over the retimer's limit table, [`parse_wire_demand`] and
//! [`parse_processors`] over the fitter's messages, [`parse_hold`] over the delay the router added for hold,
//! [`clock_nets`] over the control-signal and global-signal tables (a clock off every global network rides general
//! routing, its skew paid for in hold delay), and [`parse_hierarchy`] over the entity table, rolled up by
//! [`by_entity`] so a unit instantiated a thousand times reads as one line. A fit still running is read from its
//! plan and place reports. A report Quartus did not write (a synthesis-only run, a failed fit) is absent from the
//! build, never zero.
//!
//! A number that does not parse is left out rather than read as zero: a malformed field is a missing field.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A compile's flow report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub status: Option<String>,
    pub version: Option<String>,
    pub revision: Option<String>,
    pub top: Option<String>,
    pub family: Option<String>,
    pub device: Option<String>,
    pub started: Option<String>,
    pub phases: Vec<Phase>,
}

impl Flow {
    pub fn succeeded(&self) -> bool {
        self.status.as_deref().is_some_and(|s| s.starts_with("Successful"))
    }

    /// The phase named `name`.
    pub fn phase(&self, name: &str) -> Option<&Phase> {
        self.phases.iter().find(|p| p.name == name)
    }
}

/// One module of the flow: its elapsed wall time and its peak virtual memory (none for the total).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub name: String,
    pub seconds: u64,
    pub peak_mb: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratio {
    pub used: u64,
    pub total: u64,
}

/// A fit's resources as its summary states them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub status: Option<String>,
    pub alms: Option<Ratio>,
    pub registers: Option<u64>,
    pub block_memory_bits: Option<Ratio>,
    pub ram_blocks: Option<Ratio>,
    pub dsp_blocks: Option<Ratio>,
}

/// Synthesis's estimate of a design's resources.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimate {
    pub alms: Option<u64>,
    pub registers: Option<u64>,
    pub memory_bits: Option<u64>,
    pub dsp_blocks: Option<u64>,
    pub fixed_point_dsp: Option<u64>,
    pub floating_point_dsp: Option<u64>,
}

/// A partition's synthesis estimate: in a partial-reconfiguration build the static region and each persona apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionEstimate {
    pub partition: String,
    pub estimate: Estimate,
}

/// One timing analysis of one clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slack {
    pub analysis: String,
    pub clock: String,
    pub slack_ns: f64,
    pub tns_ns: f64,
    pub corner: String,
}

/// The router's estimated peak demand for one kind of wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDemand {
    pub wire: String,
    pub percent: u32,
    pub direction: String,
    pub region: String,
}

/// Routing delay the fitter added on one clock transfer to meet hold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoldDelay {
    pub source: String,
    pub destination: String,
    pub ns: f64,
}

/// A net the fitter uses as a clock: its fan-out, and whether it rides a global network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockNet {
    pub name: String,
    pub fanout: u64,
    pub global: bool,
}

/// One row of the entity table: totals of the node and everything under it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub partition: String,
    pub path: String,
    pub entity: String,
    pub depth: usize,
    pub aluts: u64,
    pub registers: u64,
    pub memory_bits: u64,
    pub dsp: u64,
    /// Logic levels on the longest path through the node.
    pub max_depth: Option<f64>,
}

/// An entity's instances, totalled over the outermost ones (an instance inside another instance of the same entity
/// is already in its total).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityTotal {
    pub entity: String,
    pub instances: usize,
    pub aluts: u64,
    pub registers: u64,
    pub memory_bits: u64,
    pub dsp: u64,
    pub max_depth: Option<f64>,
}

/// One compile, as its reports state it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub flow: Flow,
    pub synthesis: Vec<PartitionEstimate>,
    pub fit: Option<Usage>,
    pub clocks: Vec<Slack>,
    pub retime: Vec<(String, String)>,
    pub wire_demand: Vec<WireDemand>,
    pub processors: Option<u32>,
    /// Each partition's top entity and its largest children, at most [`TOP_ROWS`] a partition.
    pub top: Vec<Node>,
    /// The entities by total ALUTs, largest first, at most [`ENTITY_ROWS`].
    pub entities: Vec<EntityTotal>,
    /// The routing delay the fitter added for hold, by clock transfer, none when a routed fit names no transfer;
    /// `None` when no fit report states it (a fit not yet routed, or a row recorded before it was read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Vec<HoldDelay>>,
    /// Every net the fitter uses as a clock, those off every global network first, then by fan-out; `None` without
    /// both the control-signal and the global-signal tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_nets: Option<Vec<ClockNet>>,
    /// Every Logic Lock region of the fit, what was placed in it over what it holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionUsage>,
}

/// How many entity totals a [`Build`] keeps.
pub const ENTITY_ROWS: usize = 32;
/// How many nodes of each partition's top two levels a [`Build`] keeps.
pub const TOP_ROWS: usize = 32;

impl Build {
    /// The setup analysis with the least slack.
    pub fn worst_setup(&self) -> Option<&Slack> {
        worst_setup(&self.clocks)
    }

    /// The synthesis estimate of the partition named `partition`.
    pub fn synthesis_of(&self, partition: &str) -> Option<&Estimate> {
        self.synthesis.iter().find(|p| p.partition == partition).map(|p| &p.estimate)
    }
}

/// The cells of a report table row (`; a ; b ; c ;`), trimmed, or `None` for a line that is not one.
fn cells(line: &str) -> Option<Vec<&str>> {
    let t = line.trim_end();
    if !t.starts_with(';') || !t.ends_with(';') || t.len() < 2 {
        return None;
    }
    Some(t[1..t.len() - 1].split(';').map(str::trim).collect())
}

pub(crate) fn number(s: &str) -> Option<u64> {
    let digits: String = s.chars().filter(|&c| c != ',').collect();
    digits.trim().parse().ok()
}

/// `453,119 / 933,120 ( 49 % )`.
fn ratio(s: &str) -> Option<Ratio> {
    let (used, rest) = s.split_once('/')?;
    let total = rest.split('(').next()?;
    Some(Ratio { used: number(used)?, total: number(total)? })
}

/// `02:47:28`.
fn hms(s: &str) -> Option<u64> {
    let parts: Vec<u64> = s.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        [h, m, sec] => Some(h * 3600 + m * 60 + sec),
        _ => None,
    }
}

/// A compile's flow report.
pub fn parse_flow(text: &str) -> Flow {
    let mut flow = Flow::default();
    let mut in_elapsed = false;
    for line in text.lines() {
        if line.starts_with('+') {
            continue;
        }
        let Some(c) = cells(line) else {
            in_elapsed = false;
            continue;
        };
        if c.len() == 1 {
            in_elapsed = c[0] == "Flow Elapsed Time";
            continue;
        }
        if in_elapsed {
            if c.len() >= 3 && c[0] != "Module Name" {
                if let Some(seconds) = hms(c[1]) {
                    let peak_mb = c[2].strip_suffix("MB").and_then(|m| number(m));
                    flow.phases.push(Phase { name: c[0].to_string(), seconds, peak_mb });
                }
            }
            continue;
        }
        if c.len() >= 2 {
            let slot = match c[0] {
                "Flow Status" => &mut flow.status,
                "Quartus Prime Version" => &mut flow.version,
                "Revision Name" => &mut flow.revision,
                "Top-level Entity Name" => &mut flow.top,
                "Family" => &mut flow.family,
                "Device" => &mut flow.device,
                "Start date & time" => &mut flow.started,
                _ => continue,
            };
            if slot.is_none() && !c[1].is_empty() {
                *slot = Some(c[1].to_string());
            }
        }
    }
    flow
}

/// A `.fit.summary` (or any `Key : value` summary).
pub fn parse_summary(text: &str) -> Usage {
    let mut u = Usage::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(" : ") else { continue };
        let value = value.trim();
        match key.trim() {
            "Fitter Status" | "Synthesis Status" | "Flow Status" => u.status = Some(value.to_string()),
            "Logic utilization (in ALMs)" => u.alms = ratio(value),
            "Total dedicated logic registers" => u.registers = number(value),
            "Total block memory bits" => u.block_memory_bits = ratio(value),
            "Total RAM Blocks" => u.ram_blocks = ratio(value),
            "Total DSP Blocks" => u.dsp_blocks = ratio(value),
            _ => {}
        }
    }
    u
}

/// The partition a report table's title names (`... Partition "persona1" ...`), or `root_partition` when it names
/// none.
fn partition_of(title: &str) -> String {
    let mut quoted = title.split('"');
    match (quoted.next(), quoted.next()) {
        (Some(_), Some(name)) if title.contains("Partition") => name.to_string(),
        _ => "root_partition".to_string(),
    }
}

/// Each partition's synthesis resource usage summary, in report order.
pub fn parse_synthesis(text: &str) -> Vec<PartitionEstimate> {
    let mut out: Vec<PartitionEstimate> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if line.starts_with('+') {
            continue;
        }
        let Some(c) = cells(line) else {
            current = None;
            continue;
        };
        if c.len() == 1 {
            current = c[0].starts_with("Synthesis Resource Usage Summary").then(|| {
                out.push(PartitionEstimate { partition: partition_of(c[0]), estimate: Estimate::default() });
                out.len() - 1
            });
            continue;
        }
        let Some(k) = current else { continue };
        if c.len() != 2 {
            continue;
        }
        let e = &mut out[k].estimate;
        let slot = match c[0] {
            "Estimate of Logic utilization (ALMs needed)" => &mut e.alms,
            "Dedicated logic registers" => &mut e.registers,
            "Total block memory bits" => &mut e.memory_bits,
            "Total DSP Blocks" => &mut e.dsp_blocks,
            "-- [A] Total Fixed Point DSP Blocks" => &mut e.fixed_point_dsp,
            "-- [B] Total Floating Point DSP Blocks" => &mut e.floating_point_dsp,
            _ => continue,
        };
        if slot.is_none() {
            *slot = number(c[1]);
        }
    }
    out
}

/// A timing summary: `Type : <analysis> '<clock>'`, `Slack`, `TNS`, `Corner` a block.
pub fn parse_sta_summary(text: &str) -> Vec<Slack> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let field = |line: &str, key: &str| -> Option<String> {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    };
    for (i, line) in lines.iter().enumerate() {
        let Some(ty) = field(line, "Type") else { continue };
        let Some(q) = ty.find('\'') else { continue };
        let analysis = ty[..q].trim().to_string();
        let clock = ty[q + 1..].trim_end_matches('\'').to_string();
        let rest = &lines[i + 1..(i + 4).min(lines.len())];
        let get = |key: &str| rest.iter().find_map(|l| field(l, key));
        let (Some(slack), Some(tns), Some(corner)) = (get("Slack"), get("TNS"), get("Corner")) else { continue };
        let (Ok(slack_ns), Ok(tns_ns)) = (slack.parse::<f64>(), tns.parse::<f64>()) else { continue };
        out.push(Slack { analysis, clock, slack_ns, tns_ns, corner });
    }
    out
}

/// The setup analysis with the least slack.
pub fn worst_setup(clocks: &[Slack]) -> Option<&Slack> {
    clocks.iter().filter(|s| s.analysis == "Setup").min_by(|a, b| a.slack_ns.total_cmp(&b.slack_ns))
}

/// The retimer's limit table: each clock transfer and the reason it stopped.
pub fn parse_retime(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with('+') {
            continue;
        }
        let Some(c) = cells(line) else {
            inside = false;
            continue;
        };
        if c.len() == 1 {
            inside = c[0] == "Retiming Limit Summary";
            continue;
        }
        if inside && c.len() >= 2 && c[0] != "Clock Transfer" {
            out.push((c[0].to_string(), c[1].to_string()));
        }
    }
    out
}

/// The data rows of the report table titled `title`, each its cells: the rows after its header, to the table's end;
/// `None` when the report has no such table.
fn table<'a>(text: &'a str, title: &str) -> Option<Vec<Vec<&'a str>>> {
    headed_table(text, title).map(|(_, rows)| rows)
}

/// [`table`] with the header row's cells.
pub(crate) fn headed_table<'a>(text: &'a str, title: &str) -> Option<(Vec<&'a str>, Vec<Vec<&'a str>>)> {
    let mut lines = text.lines();
    lines.find(|l| cells(l).is_some_and(|c| c.len() == 1 && c[0] == title))?;
    let mut rows = Vec::new();
    let mut header: Option<Vec<&str>> = None;
    for line in lines {
        if line.starts_with('+') {
            continue;
        }
        let Some(c) = cells(line) else { break };
        if header.is_none() {
            header = Some(c);
            continue;
        }
        rows.push(c);
    }
    Some((header.unwrap_or_default(), rows))
}

/// A Logic Lock region as the fitter left it: what was placed in it needs (ALMs after dense packing, logic registers,
/// M20Ks, DSP blocks after dense merging) over what the region holds, and the rectangles it is, each its corners.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionUsage {
    pub name: String,
    pub alms_needed: f64,
    pub alms: u64,
    pub registers: Option<Ratio>,
    pub m20k: Option<Ratio>,
    pub dsp: Option<Ratio>,
    pub placement: Vec<((u32, u32), (u32, u32))>,
}

/// Every region of the fit report's `Logic Lock Region Usage Summary`, a column each. A region's rectangles are
/// separated by the table's own `;`, so with one region the placement row's every rectangle is its; with several, a
/// region's cell is read alone.
pub fn parse_region_usage(text: &str) -> Vec<RegionUsage> {
    let Some((header, rows)) = headed_table(text, "Logic Lock Region Usage Summary") else { return Vec::new() };
    let names = header.get(1..).unwrap_or_default();
    let row = |stat: &str| rows.iter().find(|r| r.first() == Some(&stat));
    let rectangles = |cells: &[&str]| -> Vec<((u32, u32), (u32, u32))> {
        let text = cells.join(";");
        let numbers: Vec<u32> = text
            .split(|c: char| !c.is_ascii_digit())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse().ok())
            .collect();
        numbers.chunks_exact(4).map(|c| ((c[0], c[1]), (c[2], c[3]))).collect()
    };
    names
        .iter()
        .enumerate()
        .filter_map(|(k, &name)| {
            let cell = |stat: &str| row(stat).and_then(|r| r.get(k + 1)).copied();
            let (needed, alms) = cell("ALMs needed [=A-B+C]")?.split_once('/')?;
            let alms_needed: f64 = needed.trim().parse().ok()?;
            let alms = number(alms.split('(').next()?)?;
            let placement = match row("Region Placement") {
                Some(r) if names.len() == 1 => rectangles(&r[1..]),
                Some(r) => r.get(k + 1).map(|c| rectangles(&[c])).unwrap_or_default(),
                None => Vec::new(),
            };
            Some(RegionUsage {
                name: name.to_string(),
                alms_needed,
                alms,
                registers: cell("Dedicated Logic Registers").and_then(ratio),
                m20k: cell("M20Ks").and_then(ratio),
                dsp: cell("DSP Blocks needed [=A-B]").and_then(ratio),
                placement,
            })
        })
        .collect()
}

/// The delay the router added to meet hold, a row a clock transfer (`Estimated Delay Added for Hold Timing Summary`).
pub fn parse_hold(text: &str) -> Option<Vec<HoldDelay>> {
    let rows = table(text, "Estimated Delay Added for Hold Timing Summary")?;
    Some(
        rows.iter()
            .filter_map(|c| match c.as_slice() {
                [source, destination, ns] => Some(HoldDelay { source: source.to_string(), destination: destination.to_string(), ns: ns.parse().ok()? }),
                _ => None,
            })
            .collect(),
    )
}

/// The nets the `control` report's table uses as clocks (usage `Clock`, not a clock's enable or clear), each marked
/// global when the `globals` report's fast-signal summary names it or the buffer the fitter inserted to put it on one
/// (`clk~FITTER_INSERTED_0`); those off every global first, then by fan-out.
pub fn clock_nets(control: Option<&str>, globals: Option<&str>) -> Option<Vec<ClockNet>> {
    let control = table(control?, "Control Signals")?;
    let unbuffered = |name: &'_ str| -> String {
        match name.rsplit_once("~FITTER_INSERTED_") {
            Some((net, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => net.to_string(),
            _ => name.to_string(),
        }
    };
    let global: std::collections::HashSet<String> =
        table(globals?, "Global & Other Fast Signals Summary")?.iter().filter_map(|c| c.first().map(|n| unbuffered(n))).collect();
    let mut nets: Vec<ClockNet> = control
        .iter()
        .filter_map(|c| match c.as_slice() {
            [name, _, fanout, "Clock", ..] => Some(ClockNet { name: name.to_string(), fanout: number(fanout)?, global: global.contains(*name) }),
            _ => None,
        })
        .collect();
    nets.sort_by(|a, b| a.global.cmp(&b.global).then(b.fanout.cmp(&a.fanout)).then_with(|| a.name.cmp(&b.name)));
    nets.dedup();
    Some(nets)
}

/// The router's headline peak demands (`Info (20215)`), one a kind of wire, each once however often it is printed.
pub fn parse_wire_demand(text: &str) -> Vec<WireDemand> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Info (20215): Router estimated peak ") else { continue };
        let Some((wire, tail)) = rest.split_once(" demand : ") else { continue };
        let Some((pct, tail)) = tail.split_once("% of ") else { continue };
        let Some((direction, region)) = tail.split_once(" directional wire in region ") else { continue };
        let Ok(percent) = pct.trim().parse() else { continue };
        let w = WireDemand { wire: wire.to_string(), percent, direction: direction.to_string(), region: region.trim().to_string() };
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// The processors parallel compilation was allowed: the fitter's message ("will use up to 8 processors", "will use 24
/// of the 24 processors detected"), or its Parallel Compilation table.
pub fn parse_processors(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        if let Some(rest) = l.split("Parallel compilation is enabled and will use ").nth(1) {
            return rest.trim_start_matches("up to ").split_whitespace().next()?.parse().ok();
        }
        match cells(l)?.as_slice() {
            ["Maximum allowed", n] => n.parse().ok(),
            _ => None,
        }
    })
}

/// `661602 (53012)`: the total.
fn total(s: &str) -> Option<u64> {
    number(s.split('(').next()?)
}

fn total_depth(s: &str) -> Option<f64> {
    s.split('(').next()?.trim().parse().ok()
}

/// Every row of the entity table ("Resource Utilization by Entity"), depth from the node's indentation.
pub fn parse_hierarchy(text: &str) -> Vec<Node> {
    let mut out = Vec::new();
    let mut columns: Option<Vec<String>> = None;
    let mut partition = "root_partition".to_string();
    for line in text.lines() {
        let t = line.trim_end();
        if !t.starts_with(';') {
            if !t.starts_with('+') {
                columns = None;
            }
            continue;
        }
        let raw: Vec<&str> = t[1..t.len().saturating_sub(1)].split(';').collect();
        let c: Vec<&str> = raw.iter().map(|s| s.trim()).collect();
        if c.len() == 1 && c[0].ends_with("Resource Utilization by Entity") {
            partition = partition_of(c[0]);
            continue;
        }
        if c.first() == Some(&"Compilation Hierarchy Node") {
            columns = Some(c.iter().map(|s| s.to_string()).collect());
            continue;
        }
        let Some(cols) = &columns else { continue };
        let at = |name: &str| cols.iter().position(|c| c == name).and_then(|i| c.get(i).copied());
        let (Some(aluts), Some(registers)) = (at("Combinational ALUTs").and_then(total), at("Dedicated Logic Registers").and_then(total))
        else {
            continue;
        };
        let indent = raw[0].len() - raw[0].trim_start().len();
        out.push(Node {
            partition: partition.clone(),
            path: at("Full Hierarchy Name").unwrap_or("").to_string(),
            entity: at("Entity Name").unwrap_or("").to_string(),
            depth: indent.saturating_sub(1) / 3,
            aluts,
            registers,
            memory_bits: at("Block Memory Bits").and_then(total).unwrap_or(0),
            dsp: at("DSP Blocks").and_then(total).unwrap_or(0),
            max_depth: at("Max Depth").and_then(total_depth),
        });
    }
    out
}

/// The entity table rolled up by entity, largest total ALUTs first.
pub fn by_entity(nodes: &[Node]) -> Vec<EntityTotal> {
    let mut totals: Vec<EntityTotal> = Vec::new();
    let mut ancestors: Vec<&str> = Vec::new();
    for n in nodes {
        ancestors.truncate(n.depth);
        let nested = ancestors.contains(&n.entity.as_str());
        let k = match totals.iter().position(|t| t.entity == n.entity) {
            Some(k) => k,
            None => {
                totals.push(EntityTotal {
                    entity: n.entity.clone(),
                    instances: 0,
                    aluts: 0,
                    registers: 0,
                    memory_bits: 0,
                    dsp: 0,
                    max_depth: None,
                });
                totals.len() - 1
            }
        };
        let t = &mut totals[k];
        t.instances += 1;
        if !nested {
            t.aluts += n.aluts;
            t.registers += n.registers;
            t.memory_bits += n.memory_bits;
            t.dsp += n.dsp;
        }
        t.max_depth = match (t.max_depth, n.max_depth) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        ancestors.push(&n.entity);
    }
    totals.sort_by(|a, b| b.aluts.cmp(&a.aluts).then_with(|| a.entity.cmp(&b.entity)));
    totals
}

/// Each partition's top entity and its largest [`TOP_ROWS`] children, in report order.
fn top_of(nodes: Vec<Node>) -> Vec<Node> {
    let mut partitions: Vec<&str> = Vec::new();
    for n in &nodes {
        if !partitions.contains(&n.partition.as_str()) {
            partitions.push(&n.partition);
        }
    }
    let mut out = Vec::new();
    for p in partitions {
        let level: Vec<&Node> = nodes.iter().filter(|n| n.partition == p && n.depth <= 1).collect();
        let mut children: Vec<usize> = (0..level.len()).filter(|&i| level[i].depth == 1).collect();
        children.sort_by(|&a, &b| level[b].aluts.cmp(&level[a].aluts).then(a.cmp(&b)));
        children.truncate(TOP_ROWS);
        out.extend(
            level
                .iter()
                .enumerate()
                .filter(|&(i, n)| n.depth == 0 || children.contains(&i))
                .map(|(_, n)| (*n).clone()),
        );
    }
    out
}

/// Why a compile's reports could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuartusError {
    /// The flow report, which every compile writes first, is missing.
    NoFlowReport(String),
    Io(String),
    /// A ledger row of this name exists already.
    Recorded(String),
    /// No ledger row has this name.
    Unrecorded(String),
}

impl std::fmt::Display for QuartusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuartusError::NoFlowReport(path) => write!(f, "no flow report at {path}"),
            QuartusError::Io(why) => write!(f, "{why}"),
            QuartusError::Recorded(name) => write!(f, "a compile named {name} is recorded already"),
            QuartusError::Unrecorded(name) => write!(f, "no compile named {name} is recorded"),
        }
    }
}

impl std::error::Error for QuartusError {}

/// One compile of `revision`, read from its project's `output_files` directory.
pub fn read(output_files: &Path, revision: &str) -> Result<Build, QuartusError> {
    let path = |suffix: &str| output_files.join(format!("{revision}.{suffix}"));
    let text = |suffix: &str| -> Result<Option<String>, QuartusError> {
        let p = path(suffix);
        match std::fs::read(&p) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(QuartusError::Io(format!("{}: {e}", p.display()))),
        }
    };
    let flow_text = text("flow.rpt")?.ok_or_else(|| QuartusError::NoFlowReport(path("flow.rpt").display().to_string()))?;
    let syn = text("syn.rpt")?;
    let (fit_report, route) = (text("fit.rpt")?, text("fit.route.rpt")?);
    // The fitter's report repeats the router's messages; the router's own report is read for them when it is there.
    let router_messages = route.as_deref().or(fit_report.as_deref()).unwrap_or("");
    let processors = [fit_report.as_deref(), route.as_deref(), Some(flow_text.as_str())].into_iter().flatten().find_map(parse_processors);
    let nodes = syn.as_deref().map(parse_hierarchy).unwrap_or_default();
    let mut entities = by_entity(&nodes);
    entities.truncate(ENTITY_ROWS);
    // A fit still running has written its plan and place reports; the finished fit's report holds every table.
    let (plan, place) = (text("fit.plan.rpt")?, text("fit.place.rpt")?);
    fn holding<'a>(title: &str, reports: [Option<&'a str>; 2]) -> Option<&'a str> {
        reports.into_iter().flatten().find(|t| table(t, title).is_some())
    }
    let control = holding("Control Signals", [fit_report.as_deref(), plan.as_deref()]);
    let globals = holding("Global & Other Fast Signals Summary", [fit_report.as_deref(), place.as_deref()]);
    Ok(Build {
        flow: parse_flow(&flow_text),
        synthesis: syn.as_deref().map(parse_synthesis).unwrap_or_default(),
        fit: text("fit.summary")?.map(|t| parse_summary(&t)),
        clocks: text("sta.summary")?.map(|t| parse_sta_summary(&t)).unwrap_or_default(),
        retime: text("fit.retime.rpt")?.map(|t| parse_retime(&t)).unwrap_or_default(),
        wire_demand: parse_wire_demand(router_messages),
        processors,
        top: top_of(nodes),
        entities,
        hold: fit_report.as_deref().and_then(|t| parse_hold(t).or_else(|| table(t, "Routing Usage Summary").map(|_| Vec::new()))),
        regions: fit_report.as_deref().map(parse_region_usage).unwrap_or_default(),
        clock_nets: clock_nets(control, globals),
    })
}

/// A file a compile read, by its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub path: String,
    pub sha256: String,
}

/// One recorded compile: its name, when it was recorded, the inputs it was handed, its reports read, whether it
/// passed, and the compile it resumed, when its job went on from another's: the modules that compile ran and this
/// job did not are that compile's, judged in its row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub name: String,
    pub recorded: String,
    pub inputs: Vec<Input>,
    pub build: Build,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed: Option<String>,
}

impl Row {
    /// The Verilog file and the settings file of a synthesis-only compile that ran to the end from one of each: the
    /// compiles the timing model is calibrated against (`timing_graph::Calibration`).
    pub fn synthesis_sources(&self) -> Option<(&Input, &Input)> {
        if !self.verdict.passed || self.verdict.expected != ["Synthesis"] {
            return None;
        }
        let pick = |ext: &str| self.inputs.iter().filter(|i| i.path.ends_with(ext)).collect::<Vec<_>>();
        match (pick(".v").as_slice(), pick(".qsf").as_slice()) {
            ([v], [q]) => Some((*v, *q)),
            _ => None,
        }
    }
}

/// What a compile is judged by beyond its flow report, which a module that dies at startup leaves as the last one
/// to finish wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    /// The flow command's exit status.
    pub exit: Option<i32>,
    /// The flow modules the job runs, named as the flow report names them ("Synthesis", "Fitter", …).
    pub expected: Vec<String>,
    /// The flow's log.
    pub log: Option<String>,
    /// When the job started and when its flow report was last written, in seconds since the epoch.
    pub started: Option<u64>,
    pub flow_written: Option<u64>,
}

/// Whether a compile ran every module it runs to the end, every reason it did not, and the exit status and modules
/// it was judged against (none when its flow report was all there was). Whether its timing closed is the slack's to
/// say, not the verdict's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub passed: bool,
    pub failures: Vec<String>,
    pub exit: Option<i32>,
    pub expected: Vec<String>,
}

/// The first line of a Quartus log that says the flow failed: a fatal error, a module or shell that was
/// unsuccessful, or the OFS build script's own failure line.
pub fn log_failure(log: &str) -> Option<String> {
    log.lines()
        .map(str::trim)
        .find(|l| l.starts_with("Fatal Error:") || l.starts_with("*** Fatal Error") || l.contains(" was unsuccessful") || *l == "Quartus build failed")
        .map(str::to_string)
}

/// The verdict on `build` given `evidence`: it ran to the end when its flow report says Successful, every module the
/// job runs is in that report, the flow exited 0, its log holds no failure line, and its report was written after
/// the job started.
pub fn judge(build: &Build, evidence: &Evidence) -> Verdict {
    let mut failures = Vec::new();
    match build.flow.status.as_deref() {
        Some(s) if s.starts_with("Successful") => {}
        Some(s) => failures.push(format!("the flow report says {s}")),
        None => failures.push("the flow report has no status".to_string()),
    }
    for module in &evidence.expected {
        if build.flow.phase(module).is_none() {
            failures.push(format!("the job runs {module}, which the flow report does not hold"));
        }
    }
    if let Some(code) = evidence.exit.filter(|&c| c != 0) {
        failures.push(format!("the flow exited {code}"));
    }
    if let Some(line) = evidence.log.as_deref().and_then(log_failure) {
        failures.push(format!("the log: {line}"));
    }
    if let (Some(started), Some(written)) = (evidence.started, evidence.flow_written) {
        if written < started {
            failures.push("the flow report was written before the job started".to_string());
        }
    }
    Verdict { passed: failures.is_empty(), failures, exit: evidence.exit, expected: evidence.expected.clone() }
}

/// Every compile recorded, in recording order: the history every later build is priced from. Rows are written by
/// [`Ledger::record`] from reports, never by hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub rows: Vec<Row>,
}

impl Ledger {
    /// The ledger at `path`, or an empty one when no file is there yet.
    pub fn load(path: &Path) -> Result<Ledger, QuartusError> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| QuartusError::Io(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(QuartusError::Io(format!("{}: {e}", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), QuartusError> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| QuartusError::Io(e.to_string()))?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| QuartusError::Io(format!("{}: {e}", path.display())))
    }

    /// Add `row`, refusing a name already recorded.
    pub fn record(&mut self, row: Row) -> Result<(), QuartusError> {
        if self.rows.iter().any(|r| r.name == row.name) {
            return Err(QuartusError::Recorded(row.name));
        }
        self.rows.push(row);
        Ok(())
    }

    /// Put `row` in the place of the row of its name, refusing a name not recorded.
    pub fn replace(&mut self, row: Row) -> Result<(), QuartusError> {
        let at = self.rows.iter().position(|r| r.name == row.name).ok_or_else(|| QuartusError::Unrecorded(row.name.clone()))?;
        self.rows[at] = row;
        Ok(())
    }

    pub fn row(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.name == name)
    }
}

/// `seconds` since the Unix epoch as an RFC 3339 UTC timestamp.
pub fn utc(seconds: u64) -> String {
    let (days, rem) = (seconds / 86_400, seconds % 86_400);
    // Days to a civil date (Hinnant, *chrono-compatible low-level date algorithms*).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/quartus/tests.rs"]
mod tests;
