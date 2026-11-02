//! fabric_pipeline_lock — the fabric engine as a clocked pipeline: no op decodes into the datapath in the cycle it
//! computes, no body evaluates in one cycle, no divider is combinational.
//!
//! An engine's micro-program selects, per state, the parameters of the unit an op runs on. Decoded from the state
//! combinationally, every parameter sits at the head of the datapath it configures, and the routed design's clock is the
//! decode plus the deepest body plus the fold it feeds: 4.78 MHz on the XC7A100T, where a 28-bit floor division alone
//! is a few hundred carry stages. This lock holds the structure that removes those paths:
//!
//! - every `/` and `%` of the fabric's RTL divides by a literal power of two, a shift or a mask — a quotient by a value
//!   is the pipelined divider, and a FIFO of a depth that is no power of two wraps its slots by a compare and a
//!   subtraction: a literal divisor that is no power of two is a divider all the same, as wide as its context, and a
//!   FIFO of seven pages that wrapped by `% 7` at 32 bits was 2,800 LUTs and the 2 MHz path of the XC7A100T image;
//! - that divider is floor division, exhaustively at small widths, every quotient exactly `floor_divider_latency`
//!   cycles after its operands, one pair a cycle;
//! - every node of a Generic body is a register: an operand or a parameter is read as it is, every other node is
//!   computed from its operands' registers and registered;
//! - the op table is a register: each engine loads its unit parameters on the clock that enters a state, from the state
//!   it enters, and its combinational decode of the current state drives only its ports and the values that move with
//!   the phase;
//! - an op's last cycle is a register its duration counts down to, loaded with the table: no phase compare or state
//!   decode stands in front of the many units it resets, and no transition compares the phase;
//! - a loop's bound is its counter equal to a constant — never an increment compared, which is a carry chain in front of
//!   the next state;
//! - a matmul of one row never loads the weight-stationary array: each B element is multiplied by its A element as it
//!   lands, so the op takes its loads, the read latency and two cycles — and its RTL is the evaluator's.
//!
//! The RTL still equals the evaluator in exactly the twin's cycles, a Generic op's cycles now its points' and its body's
//! pipeline depth: `fabric_integer_lock`, `fabric_width_lock` and the rest of the fabric locks hold that.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_pipeline_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{BlockShape, NumericsPolicy, TensorRole};
use logicaffeine_compile::spatial::lower::fabric::{
    fabric_cycle_breakdown, fabric_cycle_model, floor_divider_latency, fifo_control_verilog, floor_divider_verilog, Activity, FabricBackend, VerilogArtifact,
};
use logicaffeine_compile::spatial::passes::tile::candidates;
use logicaffeine_compile::spatial::pipeline::{build_candidate, template_from_source_with};
use spatial_support::inputs_from_fills;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::lower::fabric_board::synthesized;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning};
use logicaffeine_compile::spatial::target::SpatialTarget;

fn fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

/// A LUT attention over a four-row cache, two heads: a contraction, a row max, the LUT index body, the weighted sums and
/// the floor division of one by the other.
fn lut_attention() -> String {
    let mut s = String::from("## Main\nLet Q be a 1 by 4 tensor of Int.\nSet item 1 of Q to 100.\nSet item 4 of Q to -77.\n");
    s.push_str("Let K be a 4 by 4 tensor of Int.\nSet item 2 of K to 3.\nSet item 7 of K to -100.\nSet item 16 of K to 127.\n");
    s.push_str("Let V be a 4 by 4 tensor of Int.\nSet item 1 of V to -9.\nSet item 6 of V to 11.\nSet item 11 of V to -128.\n");
    s.push_str("Let E be a 1 by 4 tensor of Int.\nSet item 1 of E to 32768.\nSet item 2 of E to 28672.\nSet item 3 of E to 25088.\nSet item 4 of E to 21952.\n");
    s.push_str("Let at be 3.\nLet C be lut attention of Q over K and V with 2 heads 2 groups up to at with table E and scale 21076 shifted right by 6.\n");
    s
}

fn programs() -> Vec<(&'static str, String)> {
    vec![("decode step", decode_step()), ("lut attention", lut_attention())]
}

fn lower(label: &str, src: &str, target: &SpatialTarget) -> VerilogArtifact {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l.program.unwrap_or_else(|| panic!("{label}: lowers whole: {:?}", l.missing.first()));
    match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{label}@{}: the fabric lowers it: {other:?}", target.name),
    }
}

/// Each module of `rtl` by name, its text up to its `endmodule`, as synthesis reads it: the scans judge the hardware, and
/// a simulation-only check is none.
fn modules(rtl: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for chunk in synthesized(rtl).split("\nmodule ").skip(1) {
        let name: String = chunk.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let body = chunk.split("\nendmodule").next().unwrap_or(chunk);
        out.insert(name, body.to_string());
    }
    out
}

/// `line` without its `//` comment and with every quoted string blanked; a compiler directive is no code.
fn code(line: &str) -> String {
    if line.trim_start().starts_with('`') {
        return String::new();
    }
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            quoted = !quoted;
            out.push(' ');
        } else if quoted {
            out.push(' ');
        } else if c == '/' && chars.peek() == Some(&'/') {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether `tok` is a Verilog number: decimal digits, or a based literal like `32'd7` or `8'sd3`.
fn literal(tok: &str) -> bool {
    let tok = tok.trim_start_matches('(').trim_end_matches(|c: char| c == ')' || c == ';' || c == ',');
    if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    match tok.split_once('\'') {
        Some((size, rest)) => {
            size.chars().all(|c| c.is_ascii_digit())
                && rest.trim_start_matches(['s', 'S']).chars().next().is_some_and(|b| "dDhHbBoO".contains(b))
                && rest.len() > 1
        }
        None => false,
    }
}

/// The value of a literal `tok` — decimal, or based like `32'd7`, `4'hA`, `3'b101` — `None` for anything else.
fn literal_value(tok: &str) -> Option<u64> {
    if !literal(tok) {
        return None;
    }
    let tok = tok.trim_start_matches('(').trim_end_matches(|c: char| c == ')' || c == ';' || c == ',');
    match tok.split_once('\'') {
        None => tok.parse().ok(),
        Some((_, rest)) => {
            let rest = rest.trim_start_matches(['s', 'S']);
            let radix = match rest.chars().next()? {
                'd' | 'D' => 10,
                'h' | 'H' => 16,
                'b' | 'B' => 2,
                _ => 8,
            };
            u64::from_str_radix(&rest[1..].replace('_', ""), radix).ok()
        }
    }
}

/// Every divisor of a `/` or a `%` in `rtl`, with its line: its value where it is a literal, `None` where it is not.
fn divisors(rtl: &str) -> Vec<(Option<u64>, String)> {
    let mut out = Vec::new();
    for line in rtl.lines() {
        let c = code(line);
        let bytes: Vec<char> = c.chars().collect();
        for (i, ch) in bytes.iter().enumerate() {
            if *ch != '/' && *ch != '%' {
                continue;
            }
            let rest: String = bytes[i + 1..].iter().collect();
            let tok = rest.split_whitespace().next().unwrap_or("");
            out.push((literal_value(tok), line.trim().to_string()));
        }
    }
    out
}

/// Every divisor of a `/` or a `%` in `rtl` that is not a literal, with its line.
fn variable_divisors(rtl: &str) -> Vec<String> {
    divisors(rtl).into_iter().filter(|(v, _)| v.is_none()).map(|(_, l)| l).collect()
}

/// Every literal divisor of a `/` or a `%` in `rtl` that is no power of two, with its line.
fn uneven_divisors(rtl: &str) -> Vec<String> {
    divisors(rtl).into_iter().filter(|(v, _)| v.is_some_and(|n| !n.is_power_of_two())).map(|(_, l)| l).collect()
}

/// The text of every `always @*` block of `module`.
fn comb_blocks(module: &str) -> Vec<String> {
    let lines: Vec<&str> = module.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.contains("always @*") {
            if l.trim_end().ends_with("begin") {
                let indent = l.len() - l.trim_start().len();
                let mut block = vec![l];
                i += 1;
                while i < lines.len() {
                    block.push(lines[i]);
                    let li = lines[i];
                    if li.trim() == "end" && li.len() - li.trim_start().len() == indent {
                        break;
                    }
                    i += 1;
                }
                out.push(block.join("\n"));
            } else {
                out.push(l.to_string());
            }
        }
        i += 1;
    }
    out
}

/// Every name `block` assigns with `op` (`=` or `<=`), as `name op`.
fn assigned(block: &str, op: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let c: String = block.lines().map(code).collect::<Vec<_>>().join("\n");
    let pat = format!(" {op} ");
    let mut at = 0;
    while let Some(k) = c[at..].find(&pat) {
        let end = at + k;
        let before = &c[..end];
        let prev = before.chars().last();
        let ok = match op {
            "=" => !matches!(prev, Some('<' | '>' | '!' | '=')),
            _ => true,
        };
        if ok {
            let name: String = before
                .chars()
                .rev()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !name.is_empty() && !name.chars().next().unwrap().is_ascii_digit() {
                out.insert(name);
            }
        }
        at = end + pat.len();
    }
    out
}

/// The clocked block of `module` that loads the op table: the one that loads `fx`, whether the state entered is a fixed
/// op's.
fn table_block(module: &str) -> Option<String> {
    let lines: Vec<&str> = module.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if !l.contains("always @(posedge clk)") {
            continue;
        }
        let indent = l.len() - l.trim_start().len();
        let mut block = vec![*l];
        for li in &lines[i + 1..] {
            block.push(li);
            if li.trim() == "end" && li.len() - li.trim_start().len() == indent {
                break;
            }
        }
        let text = block.join("\n");
        if text.contains("fx <= 1'b1") {
            return Some(text);
        }
    }
    None
}

fn engines(rtl: &str) -> Vec<(String, String)> {
    modules(rtl).into_iter().filter(|(n, _)| n.starts_with("engine_")).collect()
}

#[test]
fn no_fabric_divides_by_a_value() {
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            let found = variable_divisors(&art.rtl);
            assert!(found.is_empty(), "{label}@{}: a quotient by a value, in one cycle:\n{}", target.name, found.join("\n"));
        }
    }
}

#[test]
fn every_literal_divisor_of_the_fabric_is_a_power_of_two() {
    let mut fifos = 0usize;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            let found = uneven_divisors(&art.rtl);
            assert!(found.is_empty(), "{label}@{}: a divider by a constant:\n{}", target.name, found.join("\n"));
            // A FIFO's depth is what its free count starts from: `assign free = <depth> - cnt - rsv;`.
            fifos += modules(&art.rtl)
                .iter()
                .filter(|(n, _)| n.starts_with("fifo_tpl_"))
                .filter_map(|(_, b)| b.split("assign free = ").nth(1)?.split(' ').next()?.parse::<usize>().ok())
                .filter(|depth| !depth.is_power_of_two())
                .count();
        }
    }
    assert!(fifos > 0, "some FIFO of the programs is of a depth no power of two");
}

/// A FIFO's control at `depth` pages under `cycles` cycles of reservations, pushes and pops drawn at random within what
/// it allows, against the ring: each cycle's free count, page count, fill slot and read slot.
fn fifo_holds(depth: usize, cycles: usize) {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64 ^ depth as u64;
    let mut draw = |n: usize| -> usize {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % (n as u64 + 1)) as usize
    };
    let (mut cnt, mut rsv, mut rd) = (0usize, 0usize, 0usize);
    let mut steps = Vec::new();
    let mut want = Vec::new();
    for _ in 0..cycles {
        want.push((depth - cnt - rsv, cnt, (rd + cnt) % depth, rd));
        let reserve = draw(depth - cnt - rsv);
        let push = draw(rsv);
        let pop = draw(cnt);
        steps.push((reserve, push, pop));
        cnt = cnt + push - pop;
        rsv = rsv + reserve - push;
        rd = (rd + pop) % depth;
    }
    let name = format!("fifo_ctl_{depth}");
    let mut tb = format!(
        "`timescale 1ns/1ps\nmodule tb;\n  reg clk = 0, rst = 1, reserve = 0, push = 0, pop = 0;\n  reg [7:0] reserve_n = 0, push_n = 0, pop_n = 0;\n  wire [7:0] free, count, fill_slot0, rd_slot0;\n  {name} u(.clk(clk), .rst(rst), .reserve(reserve), .reserve_n(reserve_n), .push(push), .push_n(push_n), .pop(pop), .pop_n(pop_n), .free(free), .count(count), .fill_slot0(fill_slot0), .rd_slot0(rd_slot0));\n  integer t;\n  initial begin\n    #1 clk = 1; #1 clk = 0; rst = 0;\n    for (t = 0; t < {cycles}; t = t + 1) begin\n      case (t)\n"
    );
    for (t, (r, u, o)) in steps.iter().enumerate() {
        tb.push_str(&format!("        {t}: begin reserve = {}; reserve_n = {r}; push = {}; push_n = {u}; pop = {}; pop_n = {o}; end\n", (*r > 0) as u8, (*u > 0) as u8, (*o > 0) as u8));
    }
    tb.push_str("        default: ;\n      endcase\n      #1 $display(\"S %0d %0d %0d %0d %0d\", t, free, count, fill_slot0, rd_slot0);\n      clk = 1; #1 clk = 0;\n    end\n    $finish;\n  end\nendmodule\n");
    let dir = std::env::temp_dir().join(format!("fabric_pipeline_fifo_{depth}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fifo.v"), fifo_control_verilog(&name, depth)).unwrap();
    std::fs::write(dir.join("tb.v"), tb).unwrap();
    let build = std::process::Command::new("iverilog").args(["-g2012", "-o"]).arg(dir.join("sim")).arg(dir.join("fifo.v")).arg(dir.join("tb.v")).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(dir.join("sim")).output().expect("vvp");
    let got: Vec<(usize, usize, usize, usize)> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("S "))
        .map(|l| {
            let v: Vec<usize> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            (v[1], v[2], v[3], v[4])
        })
        .collect();
    assert_eq!(got.len(), cycles, "depth {depth}: a line a cycle");
    for (t, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "depth {depth}, cycle {t}: (free, count, fill slot, read slot)");
    }
    assert!(want.iter().any(|w| w.2 < w.3), "depth {depth}: the fill slot wraps past the read slot");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fifo_of_every_depth_wraps_its_slots_on_its_ring() {
    if !common::require_tool("iverilog") {
        return;
    }
    for depth in [2, 3, 4, 5, 6, 7, 8, 9, 13, 16] {
        fifo_holds(depth, 600);
    }
}

#[test]
fn the_divider_scan_reads_divisors_as_they_are() {
    assert_eq!(uneven_divisors("  assign s = (rd + cnt) % 7; assign t = x / 32'd8; assign u = y % 4'hC;").len(), 2);
    assert!(uneven_divisors("  assign s = (rd + cnt) % 2; assign t = x / 32'd8; assign u = y / 1; assign v = z % 3'b100;").is_empty());
    assert_eq!(literal_value("32'd7"), Some(7));
    assert_eq!(literal_value("4'hA;"), Some(10));
    assert_eq!(literal_value("8'sb101"), Some(5));
    assert_eq!(literal_value("(16)"), Some(16));
    assert_eq!(literal_value("b"), None);
    assert_eq!(variable_divisors("  wire [3:0] q = a / b;"), vec!["wire [3:0] q = a / b;".to_string()]);
    assert_eq!(variable_divisors("  wire [3:0] q = a % (b);").len(), 1);
    assert!(variable_divisors("  wire [31:0] oy = item_id / 4; // a / b in prose").is_empty());
    assert!(variable_divisors("  assign s = (rd + cnt) % 2; assign t = x / 32'd8;").is_empty());
    assert!(variable_divisors("  $display(\"%d %h\", a, b);").is_empty());
    assert!(variable_divisors("`timescale 1ns/1ps").is_empty());
}

/// Floor division as the evaluator's integer arm takes it.
fn floor(a: i64, b: i64) -> i64 {
    let q = a / b;
    if a % b != 0 && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

fn fits(v: i64, bits: u32) -> bool {
    let half = 1i64 << (bits - 1);
    (-half..half).contains(&v)
}

/// Simulate the divider at `(wa, wb, wq)` over every pair whose quotient fits `wq` bits, one pair a cycle, and hold
/// every quotient to the floor at exactly its latency.
fn divider_holds(wa: u32, wb: u32, wq: u32) {
    let pairs: Vec<(i64, i64)> = (-(1i64 << (wa - 1))..(1i64 << (wa - 1)))
        .flat_map(|a| (-(1i64 << (wb - 1))..(1i64 << (wb - 1))).map(move |b| (a, b)))
        .filter(|&(a, b)| b != 0 && fits(floor(a, b), wq))
        .collect();
    assert!(!pairs.is_empty());
    let lat = floor_divider_latency(wa, wq) as usize;
    let name = format!("fdiv_{wa}_{wb}_{wq}");
    let mut tb = format!(
        "`timescale 1ns/1ps\nmodule tb;\n  reg clk = 0;\n  reg signed [{}:0] a = 0;\n  reg signed [{}:0] b = 1;\n  wire signed [{}:0] q;\n  {name} u(.clk(clk), .a(a), .b(b), .q(q));\n",
        wa - 1,
        wb - 1,
        wq - 1
    );
    tb.push_str("  integer t;\n  initial begin\n");
    tb.push_str("    for (t = 0; t < ");
    tb.push_str(&format!("{}; t = t + 1) begin\n", pairs.len() + lat - 1));
    tb.push_str("      case (t)\n");
    for (t, (a, b)) in pairs.iter().enumerate() {
        tb.push_str(&format!("        {t}: begin a = {a}; b = {b}; end\n"));
    }
    tb.push_str("        default: ;\n      endcase\n");
    tb.push_str("      #1 clk = 1; #1 clk = 0;\n");
    tb.push_str(&format!("      if (t + 1 >= {lat}) $display(\"Q %0d %0d\", t + 1 - {lat}, q);\n"));
    tb.push_str("    end\n    $finish;\n  end\nendmodule\n");
    let dir = std::env::temp_dir().join(format!("fabric_pipeline_div_{wa}_{wb}_{wq}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("div.v"), floor_divider_verilog(&name, wa, wb, wq)).unwrap();
    std::fs::write(dir.join("tb.v"), tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(dir.join("sim"))
        .arg(dir.join("div.v"))
        .arg(dir.join("tb.v"))
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(dir.join("sim")).output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let mut seen = 0;
    for line in out.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.first() != Some(&"Q") {
            continue;
        }
        let (i, q): (usize, i64) = (t[1].parse().unwrap(), t[2].parse().unwrap());
        let (a, b) = pairs[i];
        assert_eq!(q, floor(a, b), "({wa},{wb},{wq}): ⌊{a} / {b}⌋ at {lat} cycles");
        seen += 1;
    }
    assert_eq!(seen, pairs.len(), "({wa},{wb},{wq}): every pair's quotient");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_divider_is_floor_division_one_pair_a_cycle() {
    if !common::require_tool("iverilog") {
        return;
    }
    // Wide quotients, a quotient narrower than its dividend (the proof bounds it), a divisor wider than the dividend,
    // one-bit operands.
    for (wa, wb, wq) in [(6, 4, 7), (5, 5, 6), (7, 3, 8), (8, 6, 4), (6, 3, 3), (3, 6, 4), (2, 2, 3), (1, 2, 2)] {
        divider_holds(wa, wb, wq);
    }
}

#[test]
fn the_divider_takes_a_cycle_per_quotient_bit_and_two_more() {
    assert_eq!(floor_divider_latency(28, 28), 30);
    assert_eq!(floor_divider_latency(28, 9), 11, "a quotient the proof bounds to 9 bits takes 9 steps");
    assert_eq!(floor_divider_latency(6, 30), 8, "never more steps than the dividend has bits");
}

#[test]
fn every_generic_body_node_is_a_register() {
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            let mut nodes = 0;
            for (name, m) in engines(&art.rtl) {
                // An engine's only functions are its sections' item tables: a body is never a function.
                for line in m.lines().map(code) {
                    let Some(rest) = line.trim().strip_prefix("function ") else { continue };
                    let named = if rest.starts_with('[') { rest.split_once(']').map_or("", |(_, n)| n) } else { rest };
                    let fname: String = named.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    assert!(fname.starts_with("item_y_s") || fname.starts_with("item_x_s"), "{label}@{}: {name} evaluates {fname} as a function in a cycle", target.name);
                }
                for line in m.lines() {
                    let c = code(line);
                    let t = c.trim();
                    let Some(rest) = t.strip_prefix("wire signed [") else { continue };
                    let Some((_, rest)) = rest.split_once("] ") else { continue };
                    let Some((wire, rhs)) = rest.split_once(" = ") else { continue };
                    if !(wire.starts_with("gn") && wire[2..].split('_').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))) {
                        continue;
                    }
                    let rhs = rhs.trim_end_matches(';');
                    let leaf = (rhs.starts_with("$signed(g_op") || rhs.starts_with("$signed(gp")) && rhs.matches('(').count() == 1;
                    assert!(leaf, "{label}@{}: {name}'s body node {wire} is arithmetic in one cycle: {t}", target.name);
                }
                let regs = m.lines().filter(|l| code(l).trim_start().starts_with("reg signed [") && code(l).contains(" gn")).count();
                nodes += regs;
            }
            assert!(nodes > 0, "{label}@{}: the bodies' nodes are registers", target.name);
        }
    }
}

#[test]
fn the_op_table_is_a_register_loaded_from_the_state_entered() {
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                let table = table_block(&m).unwrap_or_else(|| panic!("{label}@{}: {name} loads an op table from its next state", target.name));
                let loaded = assigned(&table, "<=");
                let declared = |prefix: &str| -> Vec<String> {
                    m.lines()
                        .filter_map(|l| {
                            let c = code(l);
                            let t = c.trim();
                            let t = t.strip_prefix("reg ")?;
                            let t = t.strip_prefix("signed ").unwrap_or(t);
                            let (_, rest) = t.split_once("] ")?;
                            let n = rest.trim_end_matches(';');
                            n.starts_with(prefix).then(|| n.to_string())
                        })
                        .collect()
                };
                let mut params: Vec<String> = Vec::new();
                for prefix in ["gp", "gu_cls", "gu_dep", "gu_base", "gu_gst", "gu_kx", "mu_p", "tw_d", "un_m", "un_s"] {
                    params.extend(declared(prefix));
                }
                for want in &params {
                    assert!(loaded.contains(want), "{label}@{}: {name} loads {want} with its op table", target.name);
                }
                for block in comb_blocks(&m) {
                    let driven = assigned(&block, "=");
                    for p in &loaded {
                        assert!(!driven.contains(p), "{label}@{}: {name} decodes the table's {p} combinationally", target.name);
                    }
                }
            }
        }
    }
}

#[test]
fn the_table_scan_reads_blocks_as_they_are() {
    let m = "  always @* begin\n    a = 0; b <= 1;\n    if (x == 2) c = d;\n  end\n  always @(posedge clk) begin\n    fx <= 0; e <= 0;\n    if (rst) ; else case (pc)\n      3: begin if (op_last) begin fx <= 1'b1; e <= 4; end else begin f <= g >= h; end end\n    endcase\n  end\n  always @* k = 1;\n";
    assert_eq!(comb_blocks(m).len(), 2);
    assert_eq!(assigned(&comb_blocks(m)[0], "="), BTreeSet::from(["a".to_string(), "c".to_string()]));
    assert_eq!(
        assigned(&table_block(m).unwrap(), "<="),
        BTreeSet::from(["fx".to_string(), "e".to_string(), "f".to_string()])
    );
}

/// The next state is read by the state register alone. The op table and the cycle counter load the row of the successor
/// a state takes, from a decode of the registered state with the conditions choosing among a few rows, so the conditions
/// the next state is computed from are the last thing in front of them. MEASURED on Board A (xc7a200t, timed P&R): the
/// critical path ran from a semaphore's count through `pc_n` and the table's decode of it, 26.1 ns, 23.0 of them in
/// routing.
#[test]
fn nothing_decodes_the_next_state() {
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                let mut assigned = 0;
                for l in m.lines().map(code) {
                    // A declaration names the register and reads nothing.
                    let t = l.trim_start();
                    if t.starts_with("reg ") && !t.contains('=') {
                        continue;
                    }
                    for (at, _) in l.match_indices("pc_n") {
                        let rest = &l[at + "pc_n".len()..];
                        let before = &l[..at];
                        if before.ends_with(|c: char| c.is_alphanumeric() || c == '_') || rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        if rest.trim_start().starts_with("= ") {
                            assigned += 1;
                        } else {
                            assert!(before.trim_end().ends_with("pc <="), "{label}@{}: {name} reads its next state: {l}", target.name);
                        }
                    }
                }
                assert!(assigned > 0, "{label}@{}: {name} computes its next state", target.name);
            }
        }
    }
}

/// No engine compares its phase with a constant in front of what the comparison guards: each `ph >= k` a datapath
/// reads is a register loaded from the next phase, `ph_ge{k} <= (ph_n >= k)`, which equals the comparison in every
/// cycle. MEASURED on Board B (xc7a100t, timed P&R, the successor-table engine): the critical path ran from `ph`
/// through `ph >= 1` and `ph >= 2` into a memory bank's write enable, 24.2 ns, 21.8 of them in routing.
#[test]
fn no_phase_is_compared_with_a_constant() {
    let compare = |l: &str| -> Option<String> {
        let b = l.as_bytes();
        let mut at = 0;
        while let Some(i) = l[at..].find("ph") {
            let i = at + i;
            at = i + 2;
            let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            let after = &l[i + 2..];
            if !before_ok || after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '[') {
                continue;
            }
            let rest = after.trim_start();
            // `<=` is left out: in a clocked block `ph <= ...` is the register's own non-blocking assignment.
            for op in [">=", "==", "!=", "<", ">"] {
                if let Some(r) = rest.strip_prefix(op) {
                    if r.trim_start().starts_with(|c: char| c.is_ascii_digit()) {
                        return Some(format!("ph {op}{}", &r[..r.len().min(8)]));
                    }
                    break;
                }
            }
        }
        None
    };
    let mut flags = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                for l in m.lines().map(code) {
                    if let Some(c) = compare(&l) {
                        panic!("{label}@{}: {name} compares its phase with a constant ({c}): {l}", target.name);
                    }
                }
                flags += m.lines().map(code).filter(|l| l.contains("always @(posedge clk)") && l.contains("ph_ge1 <=")).count();
            }
        }
    }
    assert!(flags > 0, "the engines read their phase through flags");
}

/// A gather's product, its index times its stride, is a register: the slot reads a cycle after the product of the index
/// it latched, so no multiply stands between an index and the address it moves. MEASURED on Board A (xc7a200t, timed
/// P&R, the successor-table engine): the critical path ran from the latched index `gu_gx1` through the DSP product
/// `gu_gm1` and the offset into a memory's read address, 35.8 ns.
#[test]
fn a_gather_s_product_is_a_register() {
    let mut products = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                for l in m.lines().map(code) {
                    for stmt in l.split(';').map(str::trim) {
                        // The name a `wire` declares is the last word before its `=`.
                        let declared = stmt.split('=').next().and_then(|lhs| lhs.split_whitespace().last());
                        if stmt.starts_with("wire") && stmt.contains('=') && declared.is_some_and(|d| d.starts_with("gu_gm")) {
                            panic!("{label}@{}: {name} multiplies a gather's index in front of its address: {stmt}", target.name);
                        }
                        if stmt.contains("gu_gm") && stmt.contains("<= gu_gst") {
                            products += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(products > 0, "some program gathers, and each gather's product is loaded on a clock");
}

#[test]
fn the_last_cycle_is_a_register_and_a_loop_bound_an_equality() {
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                for block in comb_blocks(&m) {
                    assert!(!assigned(&block, "=").contains("op_last"), "{label}@{}: {name} decodes its last cycle combinationally", target.name);
                }
                let counted = m.lines().map(code).any(|l| l.contains("op_last <=") && l.contains("op_left"));
                assert!(counted, "{label}@{}: {name} counts its op's cycles down to the last", target.name);
                let text: String = m.lines().map(code).collect::<Vec<_>>().join("\n");
                assert!(!text.contains("ph + 1 >="), "{label}@{}: {name} compares the phase for an op's end", target.name);
                for (i, _) in text.match_indices(" + 1 < ") {
                    let before: String = text[..i].chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
                    assert!(!(before.starts_with("lv") && before[2..].chars().all(|c| c.is_ascii_digit())), "{label}@{}: {name} compares {before}'s increment for a loop bound", target.name);
                }
            }
        }
    }
}

/// A(1×8)·W(8×8) on the XC7A100T (eight banks, read latency one) under the plan of one 1×8 block over the whole inner
/// 8: one run of A, eight rows of B each one run, the latency and the two cycles of the product and its sum — 12 cycles —
/// where the array would add its `k` rows and its `k + n − 1` fill.
#[test]
fn a_matrix_vector_product_never_loads_the_array() {
    let tensor = |name: &str, rows: usize, cols: usize, seed: i64| -> String {
        let mut t = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
        for i in 0..rows * cols {
            t.push_str(&format!("Set item {} of {name} to {}.\n", i + 1, ((i as i64 * 37 + seed) % 255) - 127));
        }
        t
    };
    let src = format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 1, 8, 5), tensor("W", 8, 8, 11));
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let template = template_from_source_with(&src, &target, NumericsPolicy::Exact).expect("a fabric template");
    let plan = candidates(&template, &target)
        .into_iter()
        .find(|c| c.plan.block == BlockShape { osh: 1, osw: 8, ibw: 8 } && c.plan.grid == (1, 1))
        .expect("the 1×8 block over the whole inner 8 is a candidate");
    let p = build_candidate(&template, &plan, &target).expect("the plan builds");
    let b = fabric_cycle_breakdown(&p, &target).expect("the fabric lowers it");
    let matmul: u64 = b.engines.iter().map(|e| e.cycles.get(&Activity::Busy("matmul")).copied().unwrap_or(0)).sum();
    assert_eq!(matmul, 1 + 8 + 1 + 2, "a one-row matmul takes its loads, the latency and two cycles");
    if !common::require_tool("iverilog") {
        return;
    }
    let inputs = inputs_from_fills(&p);
    let want = eval_choreography(&p, &inputs).expect("the evaluator");
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers it: {other:?}"),
    };
    let dir = std::env::temp_dir().join(format!("fabric_matvec_{}", std::process::id()));
    let run = fabric_rtl::simulate(&dir, &p, &art, &inputs);
    assert!(run.done, "the design finishes\n{}", run.log);
    let out = p.tensors.iter().find(|t| t.role == TensorRole::Output).expect("an output");
    let Some(Value::Int(w)) = want.tensors.get(&out.id) else { panic!("an integer output") };
    assert_eq!(run.out.get(&out.id.idx()), Some(w), "the product in RTL is the evaluator's");
    assert_eq!(run.cycles, fabric_cycle_model(&p, &target), "the RTL's cycles are the twin's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Each engine's text as a simulation reads it, every region included.
fn engines_simulated(rtl: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in rtl.split("\nmodule ").skip(1) {
        let name: String = chunk.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if name.starts_with("engine_") {
            out.push((name, chunk.split("\nendmodule").next().unwrap_or(chunk).to_string()));
        }
    }
    out
}

/// The simulation-only regions of `module`: what lies between `synthesis translate_off` and `synthesis translate_on`.
fn simulation_only(module: &str) -> String {
    let mut out = String::new();
    let mut on = false;
    for l in module.lines() {
        if l.contains("synthesis translate_off") {
            on = true;
        } else if l.contains("synthesis translate_on") {
            on = false;
        } else if on {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// The registers an engine's op table loads: the names its clocked block clears in every cycle before the state's row.
fn table_registers(table: &str) -> Vec<String> {
    let clears = table.lines().nth(1).expect("the table's clearing line");
    clears.split(';').filter_map(|s| s.trim().strip_suffix(" <= 0").map(str::to_string)).collect()
}

/// The phase flags an engine declares.
fn phase_flags(module: &str) -> BTreeSet<u64> {
    let mut out = BTreeSet::new();
    for l in module.lines().filter(|l| l.trim_start().starts_with("reg ") && l.contains("ph_ge")) {
        for tok in l.trim_start()["reg ".len()..].trim_end_matches(';').split(',') {
            if let Some(k) = tok.trim().strip_prefix("ph_ge") {
                out.insert(k.parse().expect("a flag's phase"));
            }
        }
    }
    out
}

/// Under simulation every engine checks, in every cycle out of reset, the two invariants no output shows. Each phase flag
/// equals the comparison it stands for: the flag is read only while the engine runs, so a flag that is wrong in an idle
/// cycle changes no result, and only the check sees it. An idle engine holds no row of its op table: the final state
/// loads none, and parameters loaded where no state drives their unit change nothing a run computes. The checks name
/// every flag and every register the table loads, and they are simulation-only, so no image moves.
#[test]
fn an_idle_engine_holds_no_row_and_each_phase_flag_is_its_comparison_under_simulation() {
    let mut flagged = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines_simulated(&art.rtl) {
                let sim = simulation_only(&m);
                for k in phase_flags(&m) {
                    flagged += 1;
                    assert!(
                        sim.lines().any(|l| l.contains(&format!("ph_ge{k} != (ph >= {k})")) && l.contains("FATAL")),
                        "{label}@{}: {name} checks ph_ge{k} against `ph >= {k}` under simulation",
                        target.name
                    );
                }
                let Some(table) = table_block(&m) else { continue };
                let idle = sim.lines().find(|l| l.contains("!running") && l.contains("FATAL")).unwrap_or_else(|| {
                    panic!("{label}@{}: {name} checks under simulation that an idle engine holds no row", target.name)
                });
                for r in table_registers(&table) {
                    let named = idle.match_indices(r.as_str()).any(|(at, _)| {
                        let before = idle[..at].chars().last().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                        let after = idle[at + r.len()..].chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                        before && after
                    });
                    assert!(named, "{label}@{}: {name}'s idle check reads its table register {r}", target.name);
                }
            }
        }
    }
    assert!(flagged > 0, "the programs carry phase flags for the check to cover");
}

/// The checks stop a simulation whose RTL breaks what they hold, though no output would: a phase flag reset high, and a
/// final state that loads a row. The unbroken RTL runs to its end and equals the evaluator.
#[test]
fn the_flag_and_table_checks_stop_a_simulation_that_breaks_them() {
    if !common::require_tool("iverilog") {
        return;
    }
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let l = compile_program_to_spatial(&lut_attention(), &target).expect("a tensor program");
    let p = l.program.expect("the attention lowers whole");
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers it: {other:?}"),
    };
    let inputs = inputs_from_fills(&p);
    let run_with = |tag: &str, rtl: String| -> fabric_rtl::Run {
        let dir = std::env::temp_dir().join(format!("fabric_checks_{tag}_{}", std::process::id()));
        let broken = VerilogArtifact { rtl, ..art.clone() };
        let run = fabric_rtl::simulate(&dir, &p, &broken, &inputs);
        let _ = std::fs::remove_dir_all(&dir);
        run
    };
    let clean = run_with("clean", art.rtl.clone());
    assert!(clean.done && !clean.log.contains("FATAL"), "the unbroken RTL runs to its end\n{}", clean.log);
    let want = eval_choreography(&p, &inputs).expect("the evaluator");
    let out = p.tensors.iter().find(|t| t.role == TensorRole::Output).expect("an output");
    let Some(Value::Int(w)) = want.tensors.get(&out.id) else { panic!("an integer output") };
    assert_eq!(clean.out.get(&out.id.idx()), Some(w), "and equals the evaluator");

    let (engine, m) = engines_simulated(&art.rtl).into_iter().find(|(_, m)| !phase_flags(m).is_empty()).expect("an engine with phase flags");
    let k = *phase_flags(&m).iter().next().unwrap();
    let reset_low = format!(" ph_ge{k} <= 1'b0;");
    assert_eq!(m.matches(&reset_low).count(), 1, "{engine} resets ph_ge{k} once");
    let high = art.rtl.replacen(&m, &m.replacen(&reset_low, &format!(" ph_ge{k} <= 1'b1;"), 1), 1);
    let run = run_with("flag", high);
    assert!(!run.done && run.log.contains(&format!("FATAL: ph_ge{k}")), "a flag reset high stops the simulation by name\n{}", run.log);

    let (engine, m) = engines_simulated(&art.rtl).into_iter().find(|(_, m)| table_block(m).is_some()).expect("an engine with an op table");
    let table = table_block(&m).unwrap();
    let fin = table.lines().find(|l| {
        let t = l.trim();
        t.ends_with(": ;") && t[..t.len() - 3].chars().all(|c| c.is_ascii_digit())
    });
    let fin = fin.unwrap_or_else(|| panic!("{engine}'s table has its final state's arm, which loads nothing"));
    let reg = table_registers(&table).into_iter().next().expect("a table register");
    let loads = fin.replace(": ;", &format!(": begin {reg} <= 1; end"));
    let broken_table = table.replacen(fin, &loads, 1);
    let rtl = art.rtl.replacen(&m, &m.replacen(&table, &broken_table, 1), 1);
    let run = run_with("table", rtl);
    assert!(!run.done && run.log.contains("FATAL: an idle engine"), "a final state that loads a row stops the simulation by name\n{}", run.log);
}

/// The phase flags load as a shift: while a fixed op counts its phase up, each `ph_ge{k}` takes `ph_ge{k-1}`; the
/// flags clear where the phase restarts and hold otherwise. Registers choose which (`fx`, `op_last`, `running`), never
/// the next state's decode or a comparator, and the flags run dense from 1 so each one's predecessor exists. The
/// simulation-only check holds every flag to `ph >= k` in every cycle. MEASURED on Board B (xc7a100t, the image of
/// probe-a7-s1, 42.62 MHz): the critical path ran from the state register through the next-phase decode into
/// `ph_n >= 4`.
#[test]
fn each_phase_flag_shifts_from_the_one_below() {
    let mut shifted = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                let flags = phase_flags(&m);
                let Some(&max) = flags.iter().max() else { continue };
                assert_eq!(flags, (1..=max).collect::<BTreeSet<u64>>(), "{label}@{}: {name}'s flags run dense from 1", target.name);
                let load = m
                    .lines()
                    .map(code)
                    .find(|l| l.contains("always @(posedge clk)") && l.contains("ph_ge1 <="))
                    .unwrap_or_else(|| panic!("{label}@{}: {name} loads its phase flags", target.name));
                assert!(!load.contains("ph_n") && !load.contains(">="), "{label}@{}: {name}'s flags load from no next phase and no comparator: {load}", target.name);
                assert!(load.contains("ph_ge1 <= 1'b1;"), "{label}@{}: {name}'s first flag sets as the phase leaves 0", target.name);
                for k in 2..=max {
                    assert!(load.contains(&format!("ph_ge{k} <= ph_ge{};", k - 1)), "{label}@{}: {name}'s ph_ge{k} shifts from ph_ge{}", target.name, k - 1);
                }
                shifted += 1;
            }
        }
    }
    assert!(shifted > 0, "the programs carry phase flags");
}

/// The walk prefixes: the unit counters and offsets that address tensors, accumulators and operand rows.
const WALKS: [&str; 14] = ["tw_", "aw_", "ar_", "un_e", "un_b", "mc_e", "mc_pg", "ma_", "mb_", "mbi_", "mr_", "mu_q", "mu_o", "mw_"];

/// Every walk runs at its engine's address width, never 32 bits. The width is `localparam AW`, one more bit than the
/// largest value any walk takes: every literal the op table loads into a walk parameter, the longest op's phase, and
/// the largest tensor's elements (no tile side or bank count exceeds it). The headroom bit keeps a sum of two such values from wrapping and
/// keeps a difference that goes below zero above every value the walk takes, so each comparison reads as it does at
/// 32 bits. MEASURED on both boards (the receipt images, 10-17): the critical path ran through the run walk's 32-bit add
/// and compare (`ar_j + ar_l < aw_cols`, a carry chain to bit 28) into a memory pool's write data.
#[test]
fn every_walk_runs_at_its_engine_s_address_width() {
    let mut walked = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
            let p = l.program.expect("lowers whole");
            let art = match FabricBackend::new(target.clone()).lower(&p) {
                Ok(SpatialArtifact::Verilog(v)) => v,
                other => panic!("{label}: {other:?}"),
            };
            let largest_tensor = p.tensors.iter().map(|t| (t.elems.rows * t.elems.cols) as u64).max().unwrap_or(1);
            for (name, m) in engines(&art.rtl) {
                let lines: Vec<String> = m.lines().map(code).collect();
                let aw: u32 = lines
                    .iter()
                    .find_map(|l| l.trim().strip_prefix("localparam AW = ").and_then(|r| r.trim_end_matches(';').trim().parse().ok()))
                    .unwrap_or_else(|| panic!("{label}@{}: {name} declares its address width", target.name));
                // The table parameters declared at AW, and every literal the rows load into them.
                let mut at_aw = BTreeSet::new();
                for l in &lines {
                    let t = l.trim();
                    if t.starts_with("reg") && t.contains("[AW-1:0]") {
                        for n in t.split("[AW-1:0]").nth(1).unwrap().trim_end_matches(';').split(',') {
                            at_aw.insert(n.trim().to_string());
                        }
                    }
                }
                let mut bound = largest_tensor;
                for l in &lines {
                    for stmt in l.split(';') {
                        let Some((lhs, rhs)) = stmt.split_once("<=") else { continue };
                        let (lhs, rhs) = (lhs.trim().rsplit(' ').next().unwrap_or("").to_string(), rhs.trim());
                        let Ok(v) = rhs.parse::<i64>() else { continue };
                        if at_aw.contains(&lhs) {
                            bound = bound.max(v.unsigned_abs());
                        }
                        if lhs == "op_left" {
                            bound = bound.max(v as u64 + 3);
                        }
                    }
                }
                let want = 64 - bound.leading_zeros() + 1;
                assert_eq!(aw, want, "{label}@{}: {name}'s address width is one bit past its largest walk value {bound}", target.name);
                for l in &lines {
                    let t = l.trim();
                    if (t.starts_with("reg") || t.starts_with("wire")) && t.contains("[31:0]") {
                        let names = t.split("[31:0]").nth(1).unwrap_or("");
                        for n in names.split(',').map(|n| n.trim().split([' ', '=', ';']).next().unwrap_or("")) {
                            assert!(!WALKS.iter().any(|w| n.starts_with(w)), "{label}@{}: {name}'s walk {n} is 32 bits: {t}", target.name);
                        }
                    }
                }
                walked += 1;
            }
        }
    }
    assert!(walked > 0, "the programs have engines");
}

/// The run walks decide their next run from registers. The pack's walk keeps what is left of the row and of the page as
/// registers, and a matmul's load walk keeps what is left of its row, so
/// its advance compares the run's width with a remainder, never a sum with a bound. Each bank's next zero bit is the run
/// index it falls in and its lane against the two next remainders, never the next column or a minimum over them. Both
/// are exact: a run starts at a multiple of its width, and a lane is always below the width, so it is below the run's
/// length exactly when it is below both remainders. MEASURED on Board B (aw-a7-s1, the address-width engine,
/// 34.26 MHz): the critical path ran `ar_j + ar_l < aw_cols` → `ar_adv` → `aw_cols - ar_jn` → `ar_rln < ar_ln` → the
/// banks, 51 hops.
#[test]
fn the_run_walk_decides_its_next_run_from_registers() {
    let mut walks = 0;
    for target in fabrics() {
        for (label, src) in programs() {
            let art = lower(label, &src, &target);
            for (name, m) in engines(&art.rtl) {
                let lines: Vec<String> = m.lines().map(code).collect();
                if let Some(adv) = lines.iter().find(|l| l.contains("wire mr_adv =")) {
                    assert!(!adv.contains('+'), "{label}@{}: {name}'s load run walk advances on no sum: {adv}", target.name);
                    let left = lines.iter().find(|l| l.contains(" mr_leftn =")).unwrap_or_else(|| panic!("{label}@{}: {name}'s next remainder", target.name));
                    assert!(!left.contains("<<"), "{label}@{}: {name}'s load run walk keeps its remainder, it shifts no next column: {left}", target.name);
                    walks += 1;
                }
                let Some(adv) = lines.iter().find(|l| l.contains("wire ar_adv =")) else { continue };
                assert!(!adv.contains('+'), "{label}@{}: {name}'s run walk advances on no sum: {adv}", target.name);
                let zn = lines.iter().find(|l| l.contains(" ar_zn =")).unwrap_or_else(|| panic!("{label}@{}: {name}'s next banks", target.name));
                for v in ["ar_jn", "ar_nn", "ar_mn"] {
                    let reads = zn.match_indices(v).any(|(at, _)| !zn[at + v.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'));
                    assert!(!reads, "{label}@{}: {name}'s next banks read no {v}: {zn}", target.name);
                }
                walks += 1;
            }
        }
    }
    assert!(walks > 0, "the programs carry run walks");
}

/// `S = A + B` over elements within `±bound` (item 1 of each at `bound`), then `R = S` requantized by `m`, `S` read by
/// nothing else: forwarded, the sum and the product are one body.
fn sum_then_requant_of(bound: i64, m: i64) -> String {
    let mut s = String::from("## Main\n");
    for (t, k) in [("A", 37_981i64), ("B", 51_217)] {
        s += &format!("Let {t} be a 1 by 8 tensor of Int.\n");
        for i in 1..=8i64 {
            let v = if i == 1 { bound } else { (i * k) % (2 * bound + 1) - bound };
            s += &format!("Set item {i} of {t} to {v}.\n");
        }
    }
    s += &format!("Let S be A plus B.\nLet R be S times {m} shifted right by 24.\n");
    for i in 1..=8 {
        s += &format!("Show item {i} of R.\n");
    }
    s
}

/// A 19-bit sum times a 19-bit multiplier: each of the product's operands wider than a DSP48E1's 18-bit B port.
fn sum_then_requant() -> String {
    sum_then_requant_of(131_071, 144_532)
}

/// An 18-bit sum times an 18-bit multiplier: a product at the edge of one DSP48E1 slice, which it holds.
fn sum_then_requant_at_the_slice_edge() -> String {
    sum_then_requant_of(65_535, 104_593)
}

/// `src`, a sum then a requant, on Board A's eight lanes, forwarded: its program and its RTL.
fn sum_then_requant_lowered_of(src: &str) -> (logicaffeine_compile::spatial::ir::SpatialProgram, SpatialTarget, VerilogArtifact) {
    let mut target = SpatialTarget::xc7a200t_fabric(16, 64);
    target.site.vector_lanes = 8;
    let l = compile_program_to_spatial_returning(src, &target, &BTreeSet::new()).expect("a tensor program");
    let p = l.whole().unwrap_or_else(|| panic!("the sum and the requant lower whole: {}", l.refusal())).clone();
    assert!(!p.tensors.iter().any(|t| t.name == "S"), "the sum is forwarded into the requant's body");
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("the fabric lowers the sum and the requant: {other:?}"),
    };
    (p, target, art)
}

fn sum_then_requant_lowered() -> (logicaffeine_compile::spatial::ir::SpatialProgram, SpatialTarget, VerilogArtifact) {
    sum_then_requant_lowered_of(&sum_then_requant())
}

/// The bits of a product operand as the RTL prints it: `$signed(w)` at `w`'s declared width, `$signed(w[k:0])` at k+1.
fn operand_width(lines: &[String], expr: &str) -> Option<(String, u32)> {
    let inner = expr.trim().trim_end_matches(';').trim().strip_prefix("$signed(")?.strip_suffix(')')?;
    let (wire, cut) = match inner.split_once('[') {
        Some((w, r)) => (w, r.split(':').next().and_then(|k| k.parse::<u32>().ok()).map(|k| k + 1)),
        None => (inner, None),
    };
    let decl = lines.iter().find(|l| (l.contains("reg signed [") || l.contains("wire signed [")) && l.contains(&format!(" {wire};")) || l.contains(&format!(" {wire} =")))?;
    let at = decl.find("signed [")? + "signed [".len();
    let declared: u32 = decl[at..].split(':').next()?.parse::<u32>().ok()? + 1;
    Some((wire.to_string(), cut.unwrap_or(declared)))
}

/// A sum a product reads is kept out of the multiplier slice's pre-adder (`(* keep *)` on its register) exactly where
/// the product is wider than one slice of the target (`SiteSpec::multiplier`): yosys 0.33 folds such a sum into the
/// pre-adder of only the top slice of a product it splits over a cascade, and wires the bottom slice's A operand to `x`
/// — on chip 0 of the split step, `(o_a_c0 + o_a_c1) · 144532` computed a constant there, and `hi` and every token after
/// it were wrong on the board. A product one slice holds keeps its pre-adder, and a target with no multiplier slices
/// keeps nothing.
#[test]
fn a_sum_a_product_wider_than_one_slice_reads_is_kept_out_of_its_pre_adder() {
    let (mut kept, mut pre_added, mut at_the_edge) = (0, 0, 0);
    let mut check = |label: &str, rtl: &str, slice: Option<(u32, u32)>| {
        for (name, m) in engines(rtl) {
            let lines: Vec<String> = m.lines().map(code).collect();
            if slice.is_none() {
                assert!(!m.contains("(* keep *)"), "{label}: {name} keeps a register on a target with no multiplier slices");
            }
            for l in lines.iter().filter(|l| l.contains("always @(posedge clk) gm") && l.contains(" * ")) {
                let rhs = l.split(" <= ").nth(1).unwrap_or_default();
                let operands: Vec<(String, u32)> = rhs.split(" * ").filter_map(|o| operand_width(&lines, o)).collect();
                let [(_, wa), (_, wb)] = operands.as_slice() else { panic!("{label}: {name}'s product reads two declared operands: {l}") };
                let wider = slice.is_some_and(|(wide, narrow)| wa.max(wb) > &wide || wa.min(wb) > &narrow);
                for (wire, _) in &operands {
                    let Some(decl) = lines.iter().find(|d| d.contains("reg signed [") && d.contains(&format!(" {wire};"))) else { continue };
                    if !(decl.contains(") + $signed(") || decl.contains(") - $signed(")) {
                        continue;
                    }
                    assert_eq!(decl.contains("(* keep *)"), wider, "{label}: {name}'s product of {wa} × {wb} bits reads the sum {wire}: {decl}");
                    if wider {
                        kept += 1;
                    } else {
                        pre_added += 1;
                        if slice.is_some_and(|(_, narrow)| wa.min(wb) == &narrow) {
                            at_the_edge += 1;
                        }
                    }
                }
            }
        }
    };
    let (_, target, art) = sum_then_requant_lowered();
    check("sum then requant", &art.rtl, target.site.multiplier);
    let (_, target, art) = sum_then_requant_lowered_of(&sum_then_requant_at_the_slice_edge());
    check("sum then requant at the slice's edge", &art.rtl, target.site.multiplier);
    for target in fabrics() {
        for (label, src) in programs() {
            check(&format!("{label}@{}", target.name), &lower(label, &src, &target).rtl, target.site.multiplier);
        }
    }
    assert!(kept > 0, "a product wider than one slice reads a sum");
    assert!(pre_added > 0, "a product one slice holds reads a sum");
    assert!(at_the_edge > 0, "a product exactly as narrow as the slice's narrow port keeps its pre-adder");
}

/// A sum's product synthesizes, through the image flow's own `synth_xilinx`, with every DSP slice's data inputs known —
/// split over a cascade (19 × 19 bits, the sum kept out of the pre-adder) and at the edge of one slice (18 × 18, the sum
/// pre-added) — and the RTL runs to the evaluator's answer in the twin's cycles.
#[test]
fn a_product_of_a_sum_synthesizes_with_every_dsp_operand_known() {
    for (label, src) in [("sum then requant", sum_then_requant()), ("sum then requant at the slice's edge", sum_then_requant_at_the_slice_edge())] {
        let (p, target, art) = sum_then_requant_lowered_of(&src);
        fabric_rtl::runs_as_the_evaluator(label, &p, &target, &art, &inputs_from_fills(&p));
        if !common::require_tool("yosys") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("fabric_dsp_operands_{}_{}", std::process::id(), label.len()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spatial_top.v"), &art.rtl).unwrap();
        for (name, body) in &art.hex {
            std::fs::write(dir.join(name), body).unwrap();
        }
        std::fs::write(dir.join("synth.ys"), "read_verilog spatial_top.v\nsynth_xilinx -family xc7 -flatten -nowidelut -top spatial_top\nwrite_verilog -noattr netlist.v\n").unwrap();
        let run = std::process::Command::new("yosys").arg("-q").arg("-s").arg("synth.ys").current_dir(&dir).output().expect("yosys");
        assert!(run.status.success(), "{label}: synthesis: {}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
        let netlist = std::fs::read_to_string(dir.join("netlist.v")).expect("the netlist");
        let dsps: Vec<&str> = netlist.split("DSP48E1 #(").skip(1).map(|c| c.split(");").next().unwrap_or_default()).collect();
        assert!(!dsps.is_empty(), "{label}: the product maps to DSP slices");
        for cell in &dsps {
            for port in [".A(", ".B(", ".C(", ".D("] {
                let Some(at) = cell.find(port) else { continue };
                let arg: String = cell[at + port.len()..].chars().take_while(|c| *c != ')').collect();
                let literal = arg.split_once('\'').map(|(_, v)| v.trim_start_matches(['h', 'b', 'd'])).unwrap_or("");
                assert!(!literal.contains('x'), "{label}: a DSP slice's {} operand is unknown: {arg}", &port[1..2]);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
