use super::*;

/// A one-module netlist of `cells` (a JSON object body) with `nets` (a JSON object body) named.
fn netlist(cells: &str, nets: &str) -> String {
    format!(
        r#"{{"creator": "Yosys 0.33", "modules": {{"top": {{"attributes": {{"top": "00000000000000000000000000000001"}},
            "ports": {{}}, "cells": {{{cells}}}, "netnames": {{{nets}}}}}}}}}"#
    )
}

fn lut(name: &str, inputs: &[u64], output: u64) -> String {
    let a: Vec<String> = inputs.iter().map(|i| i.to_string()).collect();
    let name = name.replace('\\', "\\\\");
    format!(r#""{name}": {{"type": "$lut", "parameters": {{"LUT": "0110", "WIDTH": "{:032b}"}}, "connections": {{"A": [{}], "Y": [{output}]}}}}"#, inputs.len(), a.join(", "))
}

fn dff(name: &str, d: u64, q: u64) -> String {
    format!(r#""{name}": {{"type": "$_DFF_P_", "connections": {{"C": [1], "D": [{d}], "Q": [{q}]}}}}"#)
}

/// Bits as yosys writes them: net ids from 2 up, and the constants `"0"`, `"1"`, `"x"`, `"z"` as strings.
fn word(bits: &[&str]) -> String {
    bits.iter().map(|b| if matches!(*b, "0" | "1" | "x" | "z") { format!("\"{b}\"") } else { b.to_string() }).collect::<Vec<_>>().join(", ")
}

fn arith(name: &str, kind: &str, a: &[&str], b: &[&str], y: &[&str], signed: bool) -> String {
    format!(
        r#""{name}": {{"type": "{kind}", "parameters": {{"A_SIGNED": "{s}", "B_SIGNED": "{s}", "A_WIDTH": "{:032b}", "B_WIDTH": "{:032b}", "Y_WIDTH": "{:032b}"}},
            "connections": {{"A": [{}], "B": [{}], "Y": [{}]}}}}"#,
        a.len(),
        b.len(),
        y.len(),
        word(a),
        word(b),
        word(y),
        s = if signed { "00000000000000000000000000000001" } else { "00000000000000000000000000000000" }
    )
}

const UNIT: Delays = Delays { lut: 100, carry: 10, dsp: 700, memory: 300 };

fn graph(cells: &[String], nets: &str) -> TimingGraph {
    read(&netlist(&cells.join(",\n"), nets), &UNIT).unwrap()
}

fn arrival_of(g: &TimingGraph, name: &str) -> i64 {
    let node = g.nodes.iter().position(|n| n == name).unwrap_or_else(|| panic!("no node {name} in {:?}", g.nodes));
    g.arrivals().unwrap()[node]
}

#[test]
fn a_cell_of_a_black_box_module_is_a_box_and_a_constant_into_it_no_node_of_its_own() {
    // a, b -> l -> n -> u.d[0] (u.d[1] tied high); u.q -> m (with a) -> y.
    let cells = [lut("l", &[2, 3], 5), r#""u": {"type": "ip", "port_directions": {"d": "input", "q": "output"}, "connections": {"d": [5, "1"], "q": [6]}}"#.to_string(), lut("m", &[6, 2], 7)];
    let json = format!(
        r#"{{"modules": {{"ip": {{"attributes": {{"blackbox": "00000000000000000000000000000001"}},
              "ports": {{"d": {{"direction": "input", "bits": [2, 3]}}, "q": {{"direction": "output", "bits": [4]}}}}}},
            "top": {{"attributes": {{"top": "00000000000000000000000000000001"}}, "ports": {{}}, "cells": {{{}}},
              "netnames": {{"a": {{"bits": [2]}}, "b": {{"bits": [3]}}, "n": {{"bits": [5]}}, "q": {{"bits": [6]}}, "y": {{"bits": [7]}}}}}}}}}}"#,
        cells.join(",\n")
    );
    let g = read(&json, &UNIT).unwrap();
    let node = |name: &str| g.nodes.iter().position(|n| n == name).unwrap_or_else(|| panic!("no node {name} in {:?}", g.nodes));
    assert_eq!(g.boxes, [Boxed { cell: "u".into(), module: "ip".into(), out: Cost::NONE, inputs: vec![node("u.d[0]")], outputs: vec![node("q")] }]);
    let memory = read(&json.replace(r#""blackbox": "00000000000000000000000000000001""#, r#""blackbox": "00000000000000000000000000000001", "logos_box_out": "0,0,0,1""#), &UNIT).unwrap();
    assert_eq!(memory.boxes[0].out, Cost::MEMORY);
    assert_eq!(arrival_of(&memory, "q"), UNIT.memory, "a memory's outputs leave it after its read");
    assert_eq!(arrival_of(&memory, "y"), UNIT.memory + 100);
    let odd = json.replace(r#""blackbox": "00000000000000000000000000000001""#, r#""blackbox": "00000000000000000000000000000001", "logos_box_out": "fast""#);
    assert!(matches!(read(&odd, &UNIT), Err(TimingError::Json(_))), "an output cost that is not four counts is refused");
    assert!(!g.nodes.iter().any(|n| n == "u.d[1]"), "a constant bit into a box is no path's end: {:?}", g.nodes);
    assert_eq!(arrival_of(&g, "q"), 0, "a box's output starts a path");
    assert_eq!(arrival_of(&g, "y"), 100);
    let ends: Vec<(String, End, usize, i64)> = g.endpoints().unwrap().into_iter().map(|e| (e.name, e.end, e.bits, e.depth)).collect();
    assert_eq!(ends, [("u.d".to_string(), End::Box, 1, 100)]);
    assert_eq!(g.flops, 0);
}

#[test]
fn a_lut_chain_is_as_deep_as_it_is_long_and_its_path_runs_in_order() {
    let g = graph(&[lut("l0", &[2, 3], 4), lut("l1", &[4, 3], 5), lut("l2", &[5, 2], 6)], r#""a": {"bits": [2]}, "b": {"bits": [3]}, "m": {"bits": [4]}, "n": {"bits": [5]}, "y": {"bits": [6]}"#);
    let d = g.depth().unwrap();
    assert_eq!(d.depth, 300);
    let names: Vec<&str> = d.path.iter().map(|&n| g.nodes[n].as_str()).collect();
    assert_eq!(names, ["b", "m", "n", "y"]);
    assert_eq!(g.flops, 0);
}

#[test]
fn an_adder_enters_each_bit_through_a_lut_and_carries_bit_to_bit() {
    let g = graph(&[arith("add", "$add", &["2", "3", "4", "5"], &["6", "7", "8", "9"], &["10", "11", "12", "13"], false)], "");
    assert_eq!(arrival_of(&g, "net10"), 100, "bit 0: the entry LUT");
    assert_eq!(arrival_of(&g, "net13"), 130, "bit 3: the entry LUT of bit 0 and three carries");
    assert_eq!(g.depth().unwrap().depth, 130);
}

#[test]
fn a_comparison_is_its_chains_carry_out() {
    let g = graph(&[arith("lt", "$lt", &["2", "3", "4"], &["5", "6", "7"], &["8"], false)], "");
    assert_eq!(arrival_of(&g, "net8"), 120);
}

#[test]
fn a_signed_operand_extends_its_sign_into_the_chain_and_an_unsigned_one_does_not() {
    let signed = graph(&[arith("add", "$add", &["2", "3"], &["4", "5"], &["6", "7", "8", "9"], true)], "");
    let sign_feeds: usize = signed.edges.iter().filter(|e| signed.nodes[e.from] == "net3" && e.delay == 100).count();
    assert_eq!(sign_feeds, 3, "A[1] enters bits 1, 2 and 3");
    let unsigned = graph(&[arith("add", "$add", &["2", "3"], &["4", "5"], &["6", "7", "8", "9"], false)], "");
    let feeds: usize = unsigned.edges.iter().filter(|e| unsigned.nodes[e.from] == "net3" && e.delay == 100).count();
    assert_eq!(feeds, 1, "past its width an unsigned operand is zero");
}

#[test]
fn a_divider_subtracts_across_its_divisor_once_a_quotient_bit_from_the_top() {
    let a = ["2", "3", "4", "5"];
    let div = graph(&[arith("div", "$div", &a, &["6", "7"], &["8", "9", "10", "11"], false)], "");
    let stage = UNIT.lut + 2 * UNIT.carry;
    assert_eq!(arrival_of(&div, "net11"), stage, "the top quotient bit leaves the first stage");
    assert_eq!(arrival_of(&div, "net8"), 4 * stage, "the bottom one leaves the last");
    let rem = graph(&[arith("mod", "$mod", &a, &["6", "7"], &["8", "9"], false)], "");
    assert_eq!((arrival_of(&rem, "net8"), arrival_of(&rem, "net9")), (4 * stage, 4 * stage), "the remainder is the last stage's");
    let fed: Vec<usize> = (0..4).map(|s| div.edges.iter().filter(|e| div.nodes[e.to] == format!("div.stage[{s}]") && div.nodes[e.from] == "net7").count()).collect();
    assert_eq!(fed, [1, 1, 1, 1], "every stage subtracts the whole divisor");
}

#[test]
fn a_power_of_two_product_is_wiring_and_any_other_is_a_dsp_block() {
    let shift = graph(&[arith("mul", "$mul", &["2", "3", "4"], &["0", "0", "1"], &["5", "6", "7", "8", "9"], false)], "");
    assert!(!shift.nodes.iter().any(|n| n.ends_with(".dsp")));
    let wires: Vec<(String, String)> = shift.edges.iter().map(|e| (shift.nodes[e.from].clone(), shift.nodes[e.to].clone())).collect();
    assert_eq!(wires, [("net2".to_string(), "net7".to_string()), ("net3".into(), "net8".into()), ("net4".into(), "net9".into())], "x * 4 is x two bits up");
    assert_eq!(shift.depth().unwrap().depth, 0);
    let product = graph(&[arith("mul", "$mul", &["2", "3", "4"], &["1", "0", "1"], &["5", "6", "7", "8", "9"], false)], "");
    assert!(product.nodes.iter().any(|n| n == "mul.dsp"));
    assert_eq!(product.depth().unwrap().depth, 700);
    let negative = graph(&[arith("mul", "$mul", &["2", "3", "4"], &["0", "0", "1"], &["5", "6", "7", "8", "9"], true)], "");
    assert!(negative.nodes.iter().any(|n| n == "mul.dsp"), "a signed operand whose one bit is its sign is -4, not a shift");
}

fn memory(clocked: bool) -> String {
    format!(
        r#""ram": {{"type": "$mem_v2", "parameters": {{"ABITS": "00000000000000000000000000000010", "WIDTH": "00000000000000000000000000000001",
            "RD_PORTS": "00000000000000000000000000000001", "WR_PORTS": "00000000000000000000000000000001", "RD_CLK_ENABLE": "{}"}},
            "connections": {{"RD_ADDR": [2, 3], "RD_DATA": [4], "RD_EN": ["1"], "RD_CLK": [1], "RD_SRST": ["0"], "RD_ARST": ["0"],
                             "WR_ADDR": [5, 6], "WR_DATA": [7], "WR_EN": [8], "WR_CLK": [1]}}}}"#,
        if clocked { "1" } else { "0" }
    )
}

#[test]
fn a_clocked_read_holds_a_register_and_an_unclocked_read_is_logic() {
    for clocked in [true, false] {
        let g = graph(&[lut("pre", &[9], 2), memory(clocked), lut("post", &[4], 10)], "");
        let d = g.depth().unwrap();
        let held = g.edges.iter().filter(|e| g.nodes[e.to] == "ram.read[0]" && g.nodes[e.from] == "net2").map(|e| e.registers).collect::<Vec<_>>();
        assert_eq!(held, [clocked as i64]);
        let writes = g.edges.iter().filter(|e| g.nodes[e.to] == "ram.contents").map(|e| e.registers).collect::<Vec<_>>();
        assert_eq!(writes, [1, 1, 1, 1], "a write lands one clock later");
        assert_eq!(d.depth, if clocked { 300 + 100 } else { 100 + 300 + 100 });
    }
}

const OTHER: Delays = Delays { lut: 70, carry: 3, dsp: 410, memory: 250 };

/// LUTs, an adder, a product, a divider and a clocked memory in one netlist.
fn every_kind() -> String {
    let cells = [
        lut("pre", &[30, 31], 2),
        arith("add", "$add", &["2", "3"], &["4", "5"], &["6", "7"], false),
        arith("mul", "$mul", &["6", "7"], &["8", "9"], &["10", "11", "12", "13"], false),
        arith("div", "$div", &["10", "11", "12"], &["14", "15"], &["16", "17", "18"], false),
        memory(true),
        lut("post", &[16, 4], 20),
    ];
    netlist(&cells.join(",\n"), "")
}

#[test]
fn a_graph_reweighed_is_the_graph_read_with_those_delays() {
    let mut g = read(&every_kind(), &UNIT).unwrap();
    let other = read(&every_kind(), &OTHER).unwrap();
    assert_eq!(g.costs.len(), g.edges.len(), "a cost for every edge");
    assert_ne!(g.edges, other.edges);
    g.reweigh(&OTHER);
    assert_eq!(g.edges, other.edges);
    assert_eq!(g.depth().unwrap(), other.depth().unwrap());
    for (e, c) in g.edges.iter().zip(&g.costs) {
        assert_eq!(e.delay, c.delay(&OTHER), "{c:?}");
    }
}

#[test]
fn the_deepest_path_names_its_edges_and_what_they_cost() {
    let g = read(&every_kind(), &UNIT).unwrap();
    let d = g.depth().unwrap();
    assert_eq!(d.edges.len() + 1, d.path.len(), "an edge between each two nodes of the path");
    for (k, w) in d.edges.iter().zip(d.path.windows(2)) {
        assert_eq!((g.edges[*k].from, g.edges[*k].to), (w[0], w[1]));
    }
    let cost = g.cost(&d.edges);
    assert_eq!(cost.delay(&UNIT), d.depth);
    assert!(cost.lut >= 1 && cost.carry >= 1 && cost.dsp == 1, "the path runs through the adder, the product and the divider: {cost:?}");
    assert_eq!(Cost { lut: 1, carry: 2, dsp: 0, memory: 0 } + Cost { lut: 0, carry: 1, dsp: 1, memory: 1 }, Cost { lut: 1, carry: 3, dsp: 1, memory: 1 });
}

/// Five LUTs beside a 24-bit adder, and two LUTs beside a 40-bit adder: at a LUT a level and a carry bit a tenth, the
/// first is 5 deep through its LUTs and the second 4.9 through its adder.
fn lut_or_carry() -> Vec<TimingGraph> {
    let chain = |n: u64| -> Vec<String> { (0..n).map(|i| lut(&format!("l{i}"), &[100 + i], 101 + i)).collect() };
    let bits = |from: u64, width: u64| -> Vec<String> { (from..from + width).map(|b| b.to_string()).collect() };
    fn strs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    let adder = |width: u64| -> String {
        let (a, b, y) = (bits(300, width), bits(400, width), bits(500, width));
        arith("add", "$add", &strs(&a), &strs(&b), &strs(&y), false)
    };
    let mut a = chain(5);
    a.push(adder(24));
    let mut b = chain(2);
    b.push(adder(40));
    vec![graph(&a, ""), graph(&b, "")]
}

fn carries() -> Vec<Delays> {
    (0..=20).map(|carry| Delays { carry, ..Delays::LEVELS }).collect()
}

#[test]
fn the_fit_finds_the_delays_whose_critical_paths_make_the_targets() {
    let mut samples: Vec<(TimingGraph, f64)> = lut_or_carry().into_iter().zip([5.0, 4.9]).collect();
    let fitted = fit(&mut samples, &carries());
    assert_eq!(fitted.delays, Delays { lut: 100, carry: 10, dsp: 100, memory: 100 }, "{fitted:?}");
    assert_eq!(fitted.predicted, [5.0, 4.9]);
    assert!(fitted.error < 1e-12, "{fitted:?}");
    let costs: Vec<Cost> = samples.iter().map(|(g, _)| g.cost(&g.depth().unwrap().edges)).collect();
    assert_eq!(costs[0], Cost { lut: 5, ..Cost::NONE }, "the first through its LUTs");
    assert_eq!(costs[1], Cost { lut: 1, carry: 39, ..Cost::NONE }, "the second through its adder: each graph left at the fitted delays");
}

#[test]
fn the_fit_scales_every_delay_by_the_geometric_mean_of_the_ratios() {
    let mut samples: Vec<(TimingGraph, f64)> = lut_or_carry().into_iter().zip([10.0, 9.8]).collect();
    let fitted = fit(&mut samples, &carries());
    assert_eq!(fitted.delays, Delays { lut: 200, carry: 20, dsp: 200, memory: 200 }, "twice as deep: every delay doubled");
    let mut off: Vec<(TimingGraph, f64)> = lut_or_carry().into_iter().zip([5.0 * 1.1, 4.9 / 1.1]).collect();
    let fitted = fit(&mut off, &[Delays { carry: 10, ..Delays::LEVELS }]);
    assert_eq!(fitted.delays, Delays { carry: 10, ..Delays::LEVELS }, "one sample 10% above and one 10% below: the scale stays");
    assert!((fitted.error - 1.1f64.ln()).abs() < 1e-9, "{fitted:?}");
}

#[test]
fn a_flop_ends_one_path_and_starts_the_next_holding_one_register() {
    let g = graph(&[lut("a", &[2], 3), dff("r", 3, 4), lut("b", &[4], 5)], r#""q": {"bits": [4]}"#);
    assert_eq!(g.depth().unwrap().depth, 100);
    assert_eq!(g.flops, 1);
    let through: Vec<i64> = g.edges.iter().filter(|e| g.nodes[e.to] == "q").map(|e| e.registers).collect();
    assert_eq!(through, [1]);
}

#[test]
fn a_register_loop_bounds_the_clock_by_its_depth_per_register_and_is_named() {
    // q0 -> two LUTs -> q1 -> one LUT -> q0: three levels through two registers.
    let g = graph(
        &[lut("a", &[2], 3), lut("b", &[3], 4), dff("r1", 4, 5), lut("c", &[5], 6), dff("r0", 6, 2)],
        r#""q0": {"bits": [2]}, "q1": {"bits": [5]}"#,
    );
    let bound = g.loop_bound().unwrap();
    assert_eq!(bound.period(), Some((150, 1)), "300 hundredths of a level over 2 registers");
    assert!(cycle_ratio::check(g.nodes.len(), &g.edges, &bound));
    let names = g.name_loop(&bound);
    assert_eq!(names.len(), 5);
    assert_eq!(names.iter().filter(|n| n.ends_with("[1 register]")).count(), 2, "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("q0")) && names.iter().any(|n| n.starts_with("q1")), "{names:?}");
}

#[test]
fn a_loop_through_no_register_is_refused_by_name() {
    let g = graph(&[lut("a", &[2, 9], 3), lut("b", &[3], 2)], r#""x": {"bits": [2]}, "y": {"bits": [3]}"#);
    let Err(TimingError::CombinationalLoop(names)) = g.depth() else { panic!() };
    let mut names = names;
    names.sort();
    assert_eq!(names, ["x", "y"]);
    assert!(matches!(g.loop_bound(), Ok(Bound::CombinationalLoop(_))));
}

#[test]
fn a_latch_or_a_cell_without_a_delay_is_refused_by_name() {
    let latch = r#""l": {"type": "$_DLATCH_P_", "connections": {"E": [2], "D": [3], "Q": [4]}}"#;
    assert_eq!(read(&netlist(latch, ""), &UNIT).unwrap_err(), TimingError::Latch("l".into()));
    let pow = r#""p": {"type": "$pow", "connections": {"A": [2], "B": [3], "Y": [4]}}"#;
    assert_eq!(read(&netlist(pow, ""), &UNIT).unwrap_err(), TimingError::Cell { name: "p".into(), kind: "$pow".into() });
    assert!(matches!(read("{\"modules\": 3}", &UNIT), Err(TimingError::Json(_))));
}

#[test]
fn a_cell_belongs_to_the_instance_its_flattened_name_or_hdlname_gives() {
    let cell = |attrs: &str| -> Cell { serde_json::from_str(&format!(r#"{{"type": "$lut", "attributes": {{{attrs}}}}}"#)).unwrap() };
    assert_eq!(instance_of("$flatten\\ue.\\s1.\\u.$abc$63226$auto$x", &cell("")), "ue|s1|u");
    assert_eq!(instance_of("$flatten\\lane[0].u.\\div.$sub$x.v:3$7", &cell("")), "lane[0].u|div", "a generate scope is one name");
    assert_eq!(instance_of("ue.a1.q", &cell(r#""hdlname": "ue a1 q""#)), "ue|a1");
    assert_eq!(instance_of("$abc$1$x", &cell("")), "");
}

#[test]
fn an_instance_is_as_deep_as_the_deepest_path_through_its_cells_and_its_subtree() {
    // a: one LUT; a|b: two LUTs after it; c: a LUT on a separate one-level path.
    let g = graph(
        &[
            lut("$flatten\\a.$abc$1", &[2], 3),
            lut("$flatten\\a.\\b.$abc$2", &[3], 4),
            lut("$flatten\\a.\\b.$abc$3", &[4], 5),
            lut("$flatten\\c.$abc$4", &[7], 8),
        ],
        "",
    );
    let d = g.depth().unwrap();
    let by: BTreeMap<&str, (i64, i64)> = d.instances.iter().map(|i| (i.path.as_str(), (i.total, i.own))).collect();
    assert_eq!(by[""], (300, 0));
    assert_eq!(by["a"], (300, 300), "a's LUT is on the three-level path");
    assert_eq!(by["a|b"], (300, 300));
    assert_eq!(by["c"], (100, 100));
}

#[test]
fn a_bit_takes_its_public_name_before_a_generated_one() {
    let g = graph(&[lut("l", &[2], 3)], r#""$abc$9$n": {"hide_name": 1, "bits": [3]}, "sum": {"bits": [7, 3]}, "s": {"hide_name": 1, "bits": [2]}"#);
    assert!(g.nodes.contains(&"sum[1]".to_string()), "{:?}", g.nodes);
    assert!(g.nodes.contains(&"s".to_string()), "a hidden name beats none");
}

#[test]
fn a_bit_takes_the_name_of_the_outermost_scope_that_names_it_however_long() {
    let g = graph(
        &[lut("l", &[2], 3), lut("m", &[2], 4)],
        r#""uz.sa": {"bits": [3]}, "lu3": {"bits": [9, 3]}, "stages.rf1": {"bits": [4]}, "u.v.w": {"bits": [2]}, "stages.x": {"bits": [2]}"#,
    );
    assert!(g.nodes.contains(&"lu3[1]".to_string()), "the module's own name before a name inside an instance: {:?}", g.nodes);
    assert!(!g.nodes.iter().any(|n| n == "uz.sa"), "{:?}", g.nodes);
    assert!(g.nodes.contains(&"stages.rf1".to_string()) && g.nodes.contains(&"stages.x".to_string()), "one scope deep before two: {:?}", g.nodes);
}

#[test]
fn parameters_are_read_from_binary_strings_and_numbers() {
    let cell: Cell = serde_json::from_str(r#"{"type": "$mem_v2", "parameters": {"ABITS": "00000000000000000000000000001010", "WIDTH": 16, "RD_CLK_ENABLE": "10"}}"#).unwrap();
    assert_eq!(param(&cell, "ABITS"), 10);
    assert_eq!(param(&cell, "WIDTH"), 16);
    assert_eq!(param_bits(&cell, "RD_CLK_ENABLE"), [false, true], "least significant first: port 1 is clocked");
    assert_eq!(param(&cell, "MISSING"), 0);
}

#[test]
fn the_script_keeps_arithmetic_and_clocked_memories_whole_and_leaves_the_rest_as_gates() {
    let s = script("logos_simd", &[("VREGS", "16")], &[]);
    let order = [
        "read_verilog -sv design.v",
        "hierarchy -check -top logos_simd -chparam VREGS 16",
        "proc",
        "memory -nomap",
        "setundef -zero",
        "opt_expr",
        "memory_map @unclocked",
        "techmap t:$add t:$sub %u t:$neg %u t:$lt %u t:$le %u t:$gt %u t:$ge %u t:$div %u t:$mod %u t:$divfloor %u t:$modfloor %u t:$mul %u t:$mem_v2 %u %n",
        "flatten",
        "write_json timing.json",
    ];
    let mut at = 0;
    for step in order {
        let found = s[at..].find(step).unwrap_or_else(|| panic!("`{step}` after byte {at} in\n{s}"));
        at += found + step.len();
    }
    assert!(s.contains("select -set unclocked t:$mem_v2 r:RD_CLK_ENABLE<1 %i"), "every read port unclocked, at any port count");
    assert!(!s.contains("abc"), "the gates are mapped here, not by abc: {s}");
}

#[test]
fn a_box_is_made_a_black_box_once_it_is_known_a_module_before_the_hierarchy_is_elaborated() {
    let s = script("logos_d5005_shell", &[], &[("logos_engine", Cost::NONE), ("logos_ram_w64", Cost::MEMORY)]);
    let order = [
        "read_verilog -sv design.v",
        "select -assert-any =logos_engine",
        "blackbox =logos_engine",
        "select -assert-any =logos_ram_w64",
        "blackbox =logos_ram_w64",
        "setattr -mod -set logos_box_out \"0,0,0,1\" =logos_ram_w64",
        "hierarchy -check -top logos_d5005_shell",
        "proc",
    ];
    let mut at = 0;
    for step in order {
        let found = s[at..].find(step).unwrap_or_else(|| panic!("`{step}` after byte {at} in\n{s}"));
        at += found + step.len();
    }
    assert!(!script("logos_d5005_shell", &[], &[]).contains("blackbox"), "no box, no black box");
    assert_eq!(s.matches("setattr").count(), 1, "a box whose outputs leave it at once says nothing more");
    assert_eq!(script("m", &[], &[]), script("m", &[], &[]), "a pure function of its arguments");
}

#[test]
fn a_memory_written_through_more_than_two_ports_becomes_registers_before_the_memory_passes() {
    let s = script("logos_attend", &[], &[]);
    let order = [
        "proc",
        "opt_dff -nosdff -keepdc",
        "opt_mem",
        "memory_share -nosat -nowiden",
        "memory_collect",
        "select -set ported t:$mem_v2 r:WR_PORTS>2 %i",
        "memory_map @ported",
        "opt_expr",
        "memory -nomap",
        "setundef -zero",
    ];
    let mut at = 0;
    for step in order {
        let found = s[at..].find(step).unwrap_or_else(|| panic!("`{step}` after byte {at} in\n{s}"));
        at += found + step.len();
    }
}

fn gate_cell(name: &str, kind: &str, pins: &[(&str, &str)]) -> String {
    let conns: Vec<String> = pins.iter().map(|(p, b)| format!(r#""{p}": [{}]"#, word(&[b]))).collect();
    let dirs: Vec<String> = pins.iter().map(|(p, _)| format!(r#""{p}": "{}""#, if *p == "Y" { "output" } else { "input" })).collect();
    let name = name.replace('\\', "\\\\");
    format!(r#""{name}": {{"type": "{kind}", "port_directions": {{{}}}, "connections": {{{}}}}}"#, dirs.join(", "), conns.join(", "))
}

/// A module with `cells` and output ports `outs`, each a name and its bits.
fn module_with_ports(cells: &[String], outs: &[(&str, &[u64])], nets: &str) -> String {
    let ports: Vec<String> = outs.iter().map(|(n, bits)| format!(r#""{n}": {{"direction": "output", "bits": [{}]}}"#, bits.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "))).collect();
    format!(
        r#"{{"creator": "Yosys 0.33", "modules": {{"top": {{"attributes": {{"top": "00000000000000000000000000000001"}},
            "ports": {{{}}}, "cells": {{{}}}, "netnames": {{{nets}}}}}}}}}"#,
        ports.join(", "),
        cells.join(",\n")
    )
}

#[test]
fn gates_are_covered_by_luts_and_a_lut_keeps_the_name_of_the_net_it_drives() {
    let cells = [
        gate_cell("g1", "$_AND_", &[("A", "2"), ("B", "3"), ("Y", "10")]),
        gate_cell("g2", "$_OR_", &[("A", "10"), ("B", "4"), ("Y", "11")]),
        gate_cell("g3", "$_XOR_", &[("A", "11"), ("B", "5"), ("Y", "12")]),
        gate_cell("g4", "$_MUX_", &[("A", "12"), ("B", "2"), ("S", "6"), ("Y", "13")]),
    ];
    let g = read(&module_with_ports(&cells, &[("y", &[13])], r#""y": {"bits": [13]}, "a": {"bits": [2]}"#), &UNIT).unwrap();
    let d = g.depth().unwrap();
    assert_eq!(d.depth, 100, "five inputs, one LUT: {:?}", g.nodes);
    assert_eq!(g.nodes[*d.path.last().unwrap()], "y");
    assert_eq!(g.edges.iter().filter(|e| e.delay == 100).count(), 5, "one edge per LUT input");
}

#[test]
fn a_wide_and_of_gates_is_as_deep_as_its_width_in_six_input_luts() {
    for (width, levels) in [(6u64, 1), (36, 2), (37, 3), (216, 3)] {
        let mut cells = Vec::new();
        let mut layer: Vec<u64> = (2..2 + width).collect();
        let mut next = 10_000;
        while layer.len() > 1 {
            let mut up = Vec::new();
            for pair in layer.chunks(2) {
                if pair.len() == 1 {
                    up.push(pair[0]);
                    continue;
                }
                cells.push(gate_cell(&format!("g{next}"), "$_AND_", &[("A", &pair[0].to_string()), ("B", &pair[1].to_string()), ("Y", &next.to_string())]));
                up.push(next);
                next += 1;
            }
            layer = up;
        }
        let g = read(&module_with_ports(&cells, &[("y", &[layer[0]])], ""), &UNIT).unwrap();
        assert_eq!(g.depth().unwrap().depth, levels * 100, "an AND of {width}");
    }
}

#[test]
fn a_loop_of_gates_is_closed_through_the_net_read_inside_it() {
    let cells = [gate_cell("x", "$_XOR_", &[("A", "2"), ("B", "5"), ("Y", "4")]), gate_cell("n", "$_AND_", &[("A", "4"), ("B", "3"), ("Y", "5")])];
    let g = read(&module_with_ports(&cells, &[("y", &[4])], r#""y": {"bits": [4]}, "b": {"bits": [5]}"#), &UNIT).unwrap();
    let Err(TimingError::CombinationalLoop(names)) = g.depth() else { panic!("{:?}", g.depth()) };
    assert!(names.iter().any(|n| n == "y" || n == "b"), "{names:?}");
}

#[test]
fn a_gate_a_flop_reads_reaches_it_through_the_lut_that_computes_it() {
    let cells = [
        gate_cell("n", "$_NOT_", &[("A", "2"), ("Y", "3")]),
        gate_cell("a", "$_AND_", &[("A", "3"), ("B", "4"), ("Y", "5")]),
        dff("r", 5, 6),
        gate_cell("o", "$_XOR_", &[("A", "6"), ("B", "2"), ("Y", "7")]),
    ];
    let g = read(&module_with_ports(&cells, &[("y", &[7])], r#""d": {"bits": [5]}, "q": {"bits": [6]}, "y": {"bits": [7]}"#), &UNIT).unwrap();
    assert_eq!(g.depth().unwrap().depth, 100);
    let into_d: Vec<&Edge> = g.edges.iter().filter(|e| g.nodes[e.to] == "d").collect();
    assert!(!into_d.is_empty(), "the flop's input net is the LUT's output: {:?}", g.nodes);
    assert!(g.edges.iter().any(|e| g.nodes[e.from] == "d" && g.nodes[e.to] == "q" && e.registers == 1));
}

#[test]
fn every_register_and_output_is_reached_by_its_deepest_path_over_its_bits() {
    // s[0] is two LUTs past the input and s[1] one; t is three LUTs past s; the output y is one LUT past t.
    let cells = [
        lut("a", &[2], 3),
        lut("b", &[3], 4),
        dff("s0", 4, 5),
        lut("c", &[2], 6),
        dff("s1", 6, 7),
        lut("d", &[5, 7], 8),
        lut("e", &[8], 9),
        lut("f", &[9], 10),
        dff("t", 10, 11),
        lut("g", &[11], 12),
    ];
    let nets = r#""i": {"bits": [2]}, "s": {"bits": [5, 7]}, "t": {"bits": [11]}, "y": {"bits": [12]}"#;
    let g = read(&module_with_ports(&cells, &[("y", &[12])], nets), &UNIT).unwrap();
    let ends = g.endpoints().unwrap();
    let seen: Vec<(&str, End, usize, i64)> = ends.iter().map(|e| (e.name.as_str(), e.end, e.bits, e.depth)).collect();
    assert_eq!(seen, [("t", End::Register, 1, 300), ("s", End::Register, 2, 200), ("y", End::Output, 1, 100)]);
    let names = |e: &Endpoint| -> Vec<&str> { e.path.iter().map(|&n| g.nodes[n].as_str()).collect() };
    assert_eq!(names(&ends[1]), ["i", "net3", "net4"], "s's deeper bit, from the input to the register's input");
    let into_t = names(&ends[0]);
    assert_eq!(into_t.len(), 4, "{into_t:?}");
    assert!(into_t[0].starts_with("s[") && into_t[3] == "net10", "from a bit of s to t's input: {into_t:?}");
    assert_eq!(names(&ends[2]), ["t", "y"]);
}

#[test]
fn a_register_array_is_one_endpoint_over_its_words_and_bits() {
    let cells = [lut("a", &[2], 3), dff("w0", 3, 5), dff("w1", 2, 6), dff("w2", 3, 7), dff("w3", 2, 8)];
    let nets = r#""i": {"bits": [2]}, "mem[0]": {"bits": [5, 6]}, "mem[1]": {"bits": [7, 8]}"#;
    let g = read(&module_with_ports(&cells, &[], nets), &UNIT).unwrap();
    let seen: Vec<(String, usize, i64)> = g.endpoints().unwrap().into_iter().map(|e| (e.name, e.bits, e.depth)).collect();
    assert_eq!(seen, [("mem".to_string(), 4, 100)]);
}

#[test]
fn a_memorys_write_and_its_clocked_read_end_paths_and_an_unclocked_read_does_not() {
    for clocked in [true, false] {
        let g = graph(&[lut("pre", &[9], 2), memory(clocked), lut("post", &[4], 10), lut("w", &[11], 7)], "");
        let ends = g.endpoints().unwrap();
        let by: BTreeMap<&str, i64> = ends.iter().filter(|e| e.end == End::Register).map(|e| (e.name.as_str(), e.depth)).collect();
        assert_eq!(by.get("ram.contents"), Some(&100), "the write data is one LUT deep: {ends:?}");
        assert_eq!(by.get("ram.read").copied(), clocked.then_some(100), "the clocked read's address is one LUT deep: {ends:?}");
        assert_eq!(by.len(), 1 + clocked as usize, "{ends:?}");
    }
}

#[test]
fn a_design_without_registers_or_outputs_ends_nowhere_and_a_loop_is_refused() {
    assert_eq!(graph(&[lut("a", &[2], 3)], "").endpoints().unwrap(), []);
    let knot = graph(&[lut("a", &[2, 9], 3), lut("b", &[3], 2)], r#""x": {"bits": [2]}, "y": {"bits": [3]}"#);
    assert!(matches!(knot.endpoints(), Err(TimingError::CombinationalLoop(_))));
}

/// The value the structure gives node `n` under `inputs` (each input port bit's value by node).
fn value(s: &Structure, n: usize, inputs: &HashMap<usize, bool>) -> bool {
    let pin = |p: Pin| match p {
        Pin::Node(m) => value(s, m, inputs),
        Pin::Zero => false,
        Pin::One => true,
    };
    match &s.source[n] {
        Source::Input { .. } => inputs[&n],
        Source::Lut { pins, table, .. } => {
            let index = pins.iter().enumerate().fold(0usize, |i, (j, &p)| i | (pin(p) as usize) << j);
            table >> index & 1 == 1
        }
        Source::Copy { from, inverted } => value(s, *from, inputs) ^ inverted,
        Source::Constant(v) => *v,
        other => panic!("node {n} is {other:?}"),
    }
}

#[test]
fn the_structure_computes_what_the_gates_did_complements_and_all() {
    // y = mux(s, a, ~((a & b) | c) ^ d), z = ~(a ^ c): complemented nets, a NOT folded into a LUT, a mux.
    let cells = [
        gate_cell("g1", "$_AND_", &[("A", "2"), ("B", "3"), ("Y", "10")]),
        gate_cell("g2", "$_NOR_", &[("A", "10"), ("B", "4"), ("Y", "11")]),
        gate_cell("g3", "$_XOR_", &[("A", "11"), ("B", "5"), ("Y", "12")]),
        gate_cell("g4", "$_MUX_", &[("A", "12"), ("B", "2"), ("S", "6"), ("Y", "13")]),
        gate_cell("g5", "$_XNOR_", &[("A", "2"), ("B", "4"), ("Y", "14")]),
    ];
    let nets = r#""y": {"bits": [13]}, "z": {"bits": [14]}, "a": {"bits": [2]}, "b": {"bits": [3]}, "c": {"bits": [4]}, "d": {"bits": [5]}, "s": {"bits": [6]}"#;
    let json = module_with_ports(&cells, &[("y", &[13]), ("z", &[14])], nets)
        .replace(r#""ports": {"#, r#""ports": {"a": {"direction": "input", "bits": [2]}, "b": {"direction": "input", "bits": [3]}, "c": {"direction": "input", "bits": [4]}, "d": {"direction": "input", "bits": [5]}, "s": {"direction": "input", "bits": [6]}, "#);
    let g = read(&json, &UNIT).unwrap();
    let s = g.structure.as_ref().expect("read from a netlist");
    let node = |name: &str| g.nodes.iter().position(|n| n == name).unwrap();
    for row in 0..32u32 {
        let bit = |i: u32| row >> i & 1 == 1;
        let (a, b, c, d, sel) = (bit(0), bit(1), bit(2), bit(3), bit(4));
        let inputs: HashMap<usize, bool> = [("a", a), ("b", b), ("c", c), ("d", d), ("s", sel)].iter().map(|&(n, v)| (node(n), v)).collect();
        let want_y = if sel { a } else { !((a && b) || c) ^ d };
        assert_eq!(value(s, node("y"), &inputs), want_y, "y at row {row}");
        assert_eq!(value(s, node("z"), &inputs), !(a ^ c), "z at row {row}");
    }
    assert_eq!(s.input_ports.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "s"]);
    assert_eq!(s.output_ports.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["y", "z"]);
}

#[test]
fn a_kept_cell_and_a_shift_record_where_their_outputs_come_from() {
    let g = graph(
        &[
            arith("add", "$add", &["2", "3"], &["4", "5"], &["6", "7"], false),
            arith("mul", "$mul", &["8", "9"], &["0", "1"], &["10", "11", "12"], false),
        ],
        "",
    );
    let s = g.structure.as_ref().unwrap();
    assert_eq!(s.cells.len(), 1, "the adder is kept, the shift is wiring");
    assert_eq!(s.cells[0].kind, "$add");
    let at = |name: &str| g.nodes.iter().position(|n| n == name).unwrap();
    assert_eq!(s.source[at("net7")], Source::Cell { cell: 0, port: "Y".into(), bit: 1 });
    assert_eq!(s.source[at("net10")], Source::Constant(false), "x * 2 is 0 in bit 0");
    assert_eq!(s.source[at("net11")], Source::Copy { from: at("net8"), inverted: false });
    assert_eq!(s.source[at("add.carry[0]")], Source::Inside);
    assert_eq!(s.cells[0].inside, at("add.carry[0]"));
}

#[test]
fn every_mapped_luts_cone_computes_its_table() {
    let cells = [
        gate_cell("g1", "$_AND_", &[("A", "2"), ("B", "3"), ("Y", "10")]),
        gate_cell("g2", "$_NOR_", &[("A", "10"), ("B", "4"), ("Y", "11")]),
        gate_cell("g3", "$_XOR_", &[("A", "11"), ("B", "5"), ("Y", "12")]),
        gate_cell("g4", "$_MUX_", &[("A", "12"), ("B", "2"), ("S", "6"), ("Y", "13")]),
        gate_cell("g5", "$_XNOR_", &[("A", "2"), ("B", "4"), ("Y", "14")]),
        gate_cell("g6", "$_ORNOT_", &[("A", "14"), ("B", "13"), ("Y", "15")]),
    ];
    let g = read(&module_with_ports(&cells, &[("y", &[13]), ("z", &[14]), ("u", &[15])], ""), &UNIT).unwrap();
    let s = g.structure.as_ref().unwrap();
    let mut luts = 0;
    for source in &s.source {
        let Source::Lut { pins, table, cone: Some(cone) } = source else { continue };
        luts += 1;
        for row in 0..1usize << pins.len() {
            let mut v: Vec<bool> = (0..pins.len()).map(|j| row >> j & 1 == 1).collect();
            let arg = |v: &[bool], a: ConeArg| v[a.index] ^ a.inverted;
            for gate in &cone.gates {
                let (x, y) = (arg(&v, gate.a), arg(&v, gate.b));
                v.push(if gate.xor { x ^ y } else { x && y });
            }
            assert_eq!(arg(&v, cone.out), table >> row & 1 == 1, "row {row} of {source:?}");
        }
    }
    assert!(luts >= 2, "{luts}");
}

#[test]
fn a_generators_family_of_modules_is_found_by_its_prefix_and_a_number() {
    let v = "module logos_ram_w32 #(parameter D = 4) (input clk);\nendmodule\n\
             module logos_ram_w4120 (input clk);\nendmodule\n\
             module logos_ram_wide (input clk);\nendmodule\n\
             module logos_ram_w (input clk);\nendmodule\n\
             module logos_bram_w64(input clk);\nendmodule\n\
             module user (input clk);\n  logos_ram_w32 m(.clk(clk));\nendmodule\n\
             // module logos_ram_w8 in a comment\n";
    assert_eq!(modules_numbered(v, &["logos_ram_w", "logos_bram_w"]), ["logos_ram_w32", "logos_ram_w4120", "logos_bram_w64"]);
    assert_eq!(modules_numbered(v, &[]), Vec::<String>::new());
}

/// That one flip-flop of `kind`, its control pins `pins` (name, bit; net 4 a live one), from d (net 2) to q (net 3), is
/// read as a register of d with controls or without.
fn assert_register(kind: &str, pins: &[(&str, &str)], controls: bool, why: &str) {
    let mut connections = vec![r#""C": [1]"#.to_string(), r#""D": [2]"#.to_string(), r#""Q": [3]"#.to_string()];
    connections.extend(pins.iter().map(|(p, b)| format!(r#""{p}": [{b}]"#)));
    let cell = format!(r#""f": {{"type": "{kind}", "connections": {{{}}}}}"#, connections.join(", "));
    let g = graph(&[cell], r#""d": {"bits": [2]}, "q": {"bits": [3]}, "e": {"bits": [4]}"#);
    let at = |name: &str| g.nodes.iter().position(|n| n == name).unwrap();
    let source = &g.structure.as_ref().unwrap().source[at("q")];
    assert_eq!(*source, Source::Register { data: Pin::Node(at("d")), controls }, "{kind} {pins:?}: {why}");
}

#[test]
fn a_flip_flop_is_plain_when_every_control_it_has_is_held_where_it_does_nothing() {
    assert_register("$_DFF_P_", &[], false, "no control");
    assert_register("$_DFFE_PP_", &[("E", r#""1""#)], false, "enabled every clock");
    assert_register("$_DFFE_PN_", &[("E", r#""0""#)], false, "an active-low enable held low");
    assert_register("$_DFFE_PP_", &[("E", r#""0""#)], true, "never enabled: it holds what it started with");
    assert_register("$_DFFE_PP_", &[("E", "4")], true, "an enable that moves");
    assert_register("$_SDFF_PP0_", &[("R", r#""0""#)], false, "a synchronous reset held off");
    assert_register("$_SDFF_PP0_", &[("R", r#""1""#)], true, "a reset held on is the reset value");
    assert_register("$_DFF_PN0_", &[("R", r#""1""#)], false, "an active-low reset held high");
    assert_register("$_SDFFE_PP0N_", &[("R", r#""0""#), ("E", r#""0""#)], false, "both held where they do nothing");
    assert_register("$_SDFFE_PP0N_", &[("R", r#""0""#), ("E", "4")], true, "an enable that moves");
    assert_register("$_DFFSR_PNN_", &[("S", r#""1""#), ("R", r#""1""#)], false, "set and reset both held off");
    assert_register("$_DFFSR_PNN_", &[("S", r#""1""#), ("R", "4")], true, "a reset that moves");
    assert_register("$_ALDFF_PP_", &[("L", r#""0""#), ("AD", "4")], false, "a load never taken");
    assert_register("$_ALDFF_PP_", &[("L", "4"), ("AD", "4")], true, "a load that moves");
    assert_register("$_DFFE_PP_", &[], true, "an enable the netlist never connects is not one held idle");
    assert_register("$_SDFF_PP0_", &[], true, "nor is a reset it never connects");
}

#[test]
fn a_calibration_records_the_builds_it_could_not_map_beside_the_ones_it_did() {
    let row = CalibrationRow {
        build: "unit-act-syn".into(),
        top: "logos_act".into(),
        verilog_sha256: "ab".repeat(32),
        delays: Delays::LEVELS,
        instances: vec![
            CalibratedInstance { path: String::new(), quartus: 186.6, predicted: 8700, area: None },
            CalibratedInstance {
                path: "lane[0].u".into(),
                quartus: 181.0,
                predicted: 8600,
                area: Some(InstanceArea {
                    quartus: QuartusArea { aluts: 1200, registers: 300, dsp: 2, memory_bits: 0 },
                    model: Area { luts: 1000, carry: 120, half_dsps: 3, flops: 310, memory_bits: 0 },
                }),
            },
        ],
    };
    let refused = RefusedBuild {
        build: "streaming-n1-syn".into(),
        verilog_sha256: "cd".repeat(32),
        why: "ERROR: Assert `arg->is_signed == sig.as_wire()->is_signed' failed in frontends/ast/genrtlil.cc:2063.".into(),
    };
    let area = AreaFit { scale: 0.53, carry: 0.65, floor: 10_000, instances: 36, error: 0.135 };
    let c = Calibration { rows: vec![row.clone()], refused: vec![refused], area: Some(area) };
    let text = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<Calibration>(&text).unwrap(), c, "a calibration reads back as written");
    let before = serde_json::to_string(&serde_json::json!({ "rows": [row] })).unwrap();
    let old: Calibration = serde_json::from_str(&before).unwrap();
    assert!(old.refused.is_empty(), "a calibration written before refusals were recorded refused nothing");
    assert_eq!(old.area, None, "nor fitted any area");
    let unmeasured = serde_json::to_string(&c.rows[0].instances[0]).unwrap();
    assert!(!unmeasured.contains("area"), "an instance with no area writes none: {unmeasured}");
    let without = r#"{"path": "", "quartus": 1.0, "predicted": 100}"#;
    assert_eq!(serde_json::from_str::<CalibratedInstance>(without).unwrap().area, None, "one written before areas were reads as none");
}

#[test]
fn an_instance_takes_the_area_its_cells_map_to_and_its_subtree_the_sum() {
    // a: two LUTs and a flop; a|b: a 4-bit adder (4 carry bits), a 3×3 product (half a DSP block) and a product by a
    // power of two (a shift: nothing); c: a 20×20 product (a whole block) and a compare of 5 bits.
    let bits = |from: u64, width: u64| -> Vec<String> { (from..from + width).map(|b| b.to_string()).collect() };
    fn s(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    let (a4, b4, y4) = (bits(300, 4), bits(310, 4), bits(320, 4));
    let (a3, b3, y6) = (bits(400, 3), bits(410, 3), bits(420, 6));
    let (a20, b20, y40) = (bits(500, 20), bits(530, 20), bits(560, 40));
    let (c5, d5) = (bits(700, 5), bits(710, 5));
    let esc = |name: &str| name.replace('\\', "\\\\");
    let cells = [
        lut("$flatten\\a.$abc$1", &[2], 3),
        lut("$flatten\\a.$abc$2", &[3, 2], 4),
        dff(&esc("$flatten\\a.$dff$3"), 4, 5),
        arith(&esc("$flatten\\a.\\b.$add$4"), "$add", &s(&a4), &s(&b4), &s(&y4), false),
        arith(&esc("$flatten\\a.\\b.$mul$5"), "$mul", &s(&a3), &s(&b3), &s(&y6), false),
        arith(&esc("$flatten\\a.\\b.$mul$6"), "$mul", &s(&a3), &["0", "0", "1"], &s(&bits(430, 5)), false),
        arith(&esc("$flatten\\c.$mul$7"), "$mul", &s(&a20), &s(&b20), &s(&y40), false),
        arith(&esc("$flatten\\c.$lt$8"), "$lt", &s(&c5), &s(&d5), &["720"], false),
    ];
    let g = graph(&cells, "");
    assert_eq!(g.area_under("a|b"), Area { carry: 4, half_dsps: 1, ..Area::default() });
    assert_eq!(g.area_under("a"), Area { luts: 2, flops: 1, carry: 4, half_dsps: 1, ..Area::default() });
    assert_eq!(g.area_under("c"), Area { carry: 5, half_dsps: 2, ..Area::default() });
    assert_eq!(g.area_under(""), Area { luts: 2, flops: 1, carry: 9, half_dsps: 3, memory_bits: 0 });
    assert_eq!(g.area_under("nowhere"), Area::default());
    assert_eq!(g.area_under("a|"), Area::default(), "a path is matched by whole instance names");
}

#[test]
fn a_product_takes_dsp_halves_by_its_widths() {
    for ((a, b), halves) in [((8, 8), 1), ((18, 19), 1), ((19, 18), 1), ((19, 19), 2), ((27, 27), 2), ((28, 27), 4), ((54, 54), 8)] {
        assert_eq!(dsp_halves(a, b), halves, "{a}×{b}");
    }
}

#[test]
fn a_sum_of_products_and_its_chain_are_inside_the_dsp_blocks_and_a_fabric_operand_is_not() {
    // p = a×b and q = c×d, 3×3 each; s = p + q (a sum of products: the block's own adder); t = s + q (still only
    // products and their sums: the chain-in adder); u = t + e (e from the fabric: a carry chain of its own).
    let bits = |from: u64, width: u64| -> Vec<String> { (from..from + width).map(|b| b.to_string()).collect() };
    fn s(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    let (a, b, c, d, e) = (bits(10, 3), bits(20, 3), bits(30, 3), bits(40, 3), bits(50, 6));
    let (p, q, sum, t, u) = (bits(100, 6), bits(110, 6), bits(120, 6), bits(130, 6), bits(140, 6));
    let cells = [
        arith("p", "$mul", &s(&a), &s(&b), &s(&p), false),
        arith("q", "$mul", &s(&c), &s(&d), &s(&q), false),
        arith("s", "$add", &s(&p), &s(&q), &s(&sum), false),
        arith("t", "$add", &s(&sum), &s(&q), &s(&t), false),
        arith("u", "$add", &s(&t), &s(&e), &s(&u), false),
    ];
    let names = |prefix: &str, v: &[String]| -> String { format!(r#""{prefix}": {{"bits": [{}]}}"#, v.join(", ")) };
    let nets = [names("s", &sum), names("t", &t), names("u", &u)].join(", ");
    let g = graph(&cells, &nets);
    assert_eq!(g.area_under(""), Area { half_dsps: 2, carry: 6, ..Area::default() }, "only u's chain is fabric");
    assert_eq!(arrival_of(&g, "s[5]"), UNIT.dsp, "the block's adder adds nothing past its products");
    assert_eq!(arrival_of(&g, "t[5]"), UNIT.dsp, "nor does the chain-in adder");
    assert_eq!(arrival_of(&g, "u[0]"), UNIT.dsp + UNIT.lut, "a fabric add is a carry chain after the block");
}

#[test]
fn the_area_fit_recovers_the_weights_its_samples_were_made_with_and_ignores_the_small() {
    // ALUTs made as 0.5 × (luts + 0.75 × carry) with a ±10% wobble, and small instances far off that the floor leaves
    // out.
    let mut samples: Vec<(Area, u64)> = Vec::new();
    for i in 0..40u64 {
        let area = Area { luts: 20_000 + 3_000 * i, carry: 5_000 + 7_000 * (i % 9), ..Area::default() };
        let wobble = if i % 2 == 0 { 1.1 } else { 1.0 / 1.1 };
        let aluts = (0.5 * (area.luts as f64 + 0.75 * area.carry as f64) * wobble).round() as u64;
        samples.push((area, aluts));
        samples.push((Area { luts: 10 + i, ..Area::default() }, 900 + i));
    }
    let carries: Vec<f64> = (0..=60).map(|k| k as f64 * 0.05).collect();
    let fit = fit_area(&samples, 10_000, &carries);
    assert_eq!(fit.instances, 40, "only the instances of at least the floor");
    assert!((fit.carry - 0.75).abs() < 0.051, "{fit:?}");
    assert!((fit.scale - 0.5).abs() < 0.01, "{fit:?}");
    assert!(fit.error < 0.1, "the wobble is all that is left: {fit:?}");
    let a = Area { luts: 1_000, carry: 400, ..Area::default() };
    assert!((fit.aluts(&a) - fit.scale * (1_000.0 + fit.carry * 400.0)).abs() < 1e-9);
}

#[test]
fn gates_count_as_the_luts_they_are_mapped_to() {
    // Five inputs through four gates: one LUT. Thirty-six through a tree of ANDs: six LUTs and one over them.
    let cells = [
        gate_cell("g1", "$_AND_", &[("A", "2"), ("B", "3"), ("Y", "10")]),
        gate_cell("g2", "$_OR_", &[("A", "10"), ("B", "4"), ("Y", "11")]),
        gate_cell("g3", "$_XOR_", &[("A", "11"), ("B", "5"), ("Y", "12")]),
        gate_cell("g4", "$_MUX_", &[("A", "12"), ("B", "2"), ("S", "6"), ("Y", "13")]),
    ];
    let g = read(&module_with_ports(&cells, &[("y", &[13])], r#""y": {"bits": [13]}"#), &UNIT).unwrap();
    assert_eq!(g.area_under(""), Area { luts: 1, ..Area::default() });
    let mut cells = Vec::new();
    let mut layer: Vec<u64> = (2..38).collect();
    let mut next = 10_000;
    while layer.len() > 1 {
        let mut up = Vec::new();
        for pair in layer.chunks(2) {
            if pair.len() == 1 {
                up.push(pair[0]);
                continue;
            }
            cells.push(gate_cell(&format!("g{next}"), "$_AND_", &[("A", &pair[0].to_string()), ("B", &pair[1].to_string()), ("Y", &next.to_string())]));
            up.push(next);
            next += 1;
        }
        layer = up;
    }
    let g = read(&module_with_ports(&cells, &[("y", &[layer[0]])], &format!(r#""y": {{"bits": [{}]}}"#, layer[0])), &UNIT).unwrap();
    assert_eq!(g.area_under("").luts, 7);
}

#[test]
fn an_add_of_an_operand_part_product_part_fabric_is_a_carry_chain() {
    fn s(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    let bits = |from: u64, width: u64| -> Vec<String> { (from..from + width).map(|b| b.to_string()).collect() };
    let (a, b, c, d, e) = (bits(10, 3), bits(20, 3), bits(30, 3), bits(40, 3), bits(50, 3));
    let (p, q, v) = (bits(100, 6), bits(110, 6), bits(140, 6));
    let mixed: Vec<String> = p[..3].iter().chain(&e).cloned().collect();
    let cells = [
        arith("p", "$mul", &s(&a), &s(&b), &s(&p), false),
        arith("q", "$mul", &s(&c), &s(&d), &s(&q), false),
        arith("v", "$add", &s(&mixed), &s(&q), &s(&v), false),
    ];
    let g = graph(&cells, &format!(r#""v": {{"bits": [{}]}}"#, v.join(", ")));
    assert_eq!(g.area_under("").carry, 6, "half its operand is the fabric's: its adder is too");
    assert_eq!(arrival_of(&g, "v[0]"), UNIT.dsp + UNIT.lut);
}

/// A memory of `width` bits by `size` words asking for `style` (none: no ramstyle), its read clocked, its ports on the
/// nets from `at`: read address, read data, write address, write data, and one write enable net for the whole word.
fn styled_memory(name: &str, width: u64, size: u64, style: Option<&str>, at: u64) -> String {
    let abits = 64 - (size - 1).leading_zeros() as u64;
    let nets = |from: u64, n: u64| (from..from + n).map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
    let (ra, rd, wa, wd, we) = (at, at + abits, at + abits + width, at + 2 * abits + width, at + 2 * abits + 2 * width);
    let style = style.map_or(String::new(), |s| format!(r#""ramstyle": "{s}""#));
    format!(
        r#""{name}": {{"type": "$mem_v2", "attributes": {{{style}}}, "parameters": {{"ABITS": "{abits:032b}", "WIDTH": "{width:032b}",
            "SIZE": "{size:032b}", "RD_PORTS": "00000000000000000000000000000001", "WR_PORTS": "00000000000000000000000000000001",
            "RD_CLK_ENABLE": "1"}},
            "connections": {{"RD_ADDR": [{}], "RD_DATA": [{}], "RD_EN": ["1"], "RD_CLK": [1], "RD_SRST": ["0"], "RD_ARST": ["0"],
                             "WR_ADDR": [{}], "WR_DATA": [{}], "WR_EN": [{}], "WR_CLK": [1]}}}}"#,
        nets(ra, abits),
        nets(rd, width),
        nets(wa, abits),
        nets(wd, width),
        vec![we.to_string(); width as usize].join(", ")
    )
}

fn loads_of(g: &TimingGraph, name: &str) -> u64 {
    let node = g.nodes.iter().position(|n| n == name).unwrap_or_else(|| panic!("no node {name} in {:?}", g.nodes));
    g.fanouts()[node]
}

/// A net's fan-out is the loads it drives as the device builds them: each LUT, flip-flop or kept cell it is an input of
/// one, and a memory's port as many as the blocks of the RAM it asks for — an MLAB 20 bits by 32 words, an M20K 40 bits
/// by 512, a memory asking for nothing an M20K as Quartus builds it: its read and write addresses and enables reach every
/// block, a bit of its written data the blocks down its depth.
#[test]
fn a_nets_fanout_is_its_cell_inputs_and_the_blocks_of_each_memory_it_addresses() {
    let mut cells = vec![dff("r", 2, 3)];
    for k in 0..3 {
        cells.push(lut(&format!("l{k}"), &[3], 10 + k));
    }
    cells.push(styled_memory("m20k", 600, 1024, Some("M20K"), 100));
    cells.push(styled_memory("mlab", 600, 32, Some("MLAB, no_rw_check"), 3000));
    cells.push(styled_memory("plain", 41, 513, None, 6000));
    let g = graph(&cells, "");
    assert_eq!(loads_of(&g, "net3"), 3, "a register into three LUTs");
    assert_eq!(loads_of(&g, "net100"), 15 * 2, "an M20K's read address: 15 blocks across 600 bits, 2 down 1,024 words");
    assert_eq!(loads_of(&g, "net109"), 30, "every address bit alike");
    let m20k_wa = 100 + 10 + 600;
    assert_eq!(loads_of(&g, &format!("net{m20k_wa}")), 30, "its write address too");
    assert_eq!(loads_of(&g, &format!("net{}", m20k_wa + 10)), 2, "a written data bit: the blocks down its depth");
    assert_eq!(loads_of(&g, &format!("net{}", m20k_wa + 10 + 600)), 30, "the word's write enable: every block");
    assert_eq!(loads_of(&g, "net3000"), 30, "an MLAB's read address: 30 blocks across 600 bits, 1 down 32 words");
    assert_eq!(loads_of(&g, "net6000"), 2 * 2, "unstyled is an M20K: 2 across 41 bits, 2 down 513 words");
}

/// The nets past a bound, widest first, each with its driver: logic past `logic` loads, a register past `register`, a
/// module's input past `logic` (whatever drives it outside); a memory's output and a box's are named as such, and a
/// point inside a cell is no net.
#[test]
fn the_wide_nets_are_named_widest_first_with_what_drives_them() {
    let mut cells = vec![dff("r", 2, 3), lut("g", &[4], 5)];
    for k in 0..700 {
        cells.push(lut(&format!("a{k}"), &[3], 10_000 + k));
    }
    for k in 0..100 {
        cells.push(lut(&format!("b{k}"), &[5], 20_000 + k));
    }
    for k in 0..40 {
        cells.push(lut(&format!("c{k}"), &[6], 30_000 + k));
    }
    let g = graph(&cells, "");
    let wide = g.wide(64, 512);
    let seen: Vec<(String, u64, Driver)> = wide.iter().map(|w| (w.name.clone(), w.loads, w.driver)).collect();
    assert_eq!(seen, [("net3".to_string(), 700, Driver::Register), ("net5".to_string(), 100, Driver::Logic)]);
    assert!(g.wide(64, 701).iter().all(|w| w.name != "net3"), "a register within its bound");
    assert_eq!(g.wide(39, 4096).iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["net5", "net6"], "an undriven net is an input's, held to logic's bound");
    assert!(g.wide(0, 0).iter().all(|w| !w.name.contains('.')), "no point inside a cell");
}

/// A register whose net asks for at most `maxfan` loads is copies of it as Quartus duplicates it, each driving at most
/// that many: within the register bound when its `maxfan` is, however many loads the net drives; logic is held to its
/// bound whatever it asks for, a LUT never duplicated so.
#[test]
fn a_register_asking_for_a_fanout_is_its_copies_and_logic_is_held_to_its_bound_whatever_it_asks() {
    let mut cells = vec![dff("r", 2, 3), lut("g", &[4], 5)];
    for k in 0..40 {
        cells.push(lut(&format!("a{k}"), &[3], 10_000 + k));
        cells.push(lut(&format!("b{k}"), &[5], 20_000 + k));
    }
    let asks = |net: &str, n: u32| format!(r#""{net}": {{"hide_name": 0, "bits": [{}], "attributes": {{"maxfan": "{n:032b}"}}}}"#, &net[3..]);
    let g = graph(&cells, &[asks("net3", 8), asks("net5", 8)].join(", "));
    assert_eq!(g.fanouts()[g.nodes.iter().position(|n| n == "net3").unwrap()], 40, "the net's own loads, all its copies'");
    let wide: Vec<(String, Driver)> = g.wide(16, 16).into_iter().map(|w| (w.name, w.driver)).collect();
    assert_eq!(wide, [("net5".to_string(), Driver::Logic)], "the register's copies drive 8 each; the LUT drives 40");
    assert_eq!(g.wide(16, 4).iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["net3", "net5"], "copies of 8 past a bound of 4");
    let plain = graph(&cells, "");
    assert_eq!(plain.wide(16, 16).len(), 2, "asking for nothing, the register drives its 40");
}

/// A clock is no wide net: the net a flip-flop's clock pin takes, or a box's clock input (a port its module names
/// `clk` or `…clk`), however many it reaches, rides a clock network, not the fabric's wires; a box's other inputs are
/// loads like any.
#[test]
fn a_clock_is_no_wide_net_however_many_registers_or_boxes_it_clocks() {
    let mut cells = Vec::new();
    for k in 0..700u64 {
        cells.push(format!(r#""f{k}": {{"type": "$_DFF_P_", "connections": {{"C": [2], "D": [{}], "Q": [{}]}}}}"#, 10_000 + k, 20_000 + k));
        cells.push(format!(
            r#""u{k}": {{"type": "ip", "port_directions": {{"clk": "input", "fclk": "input", "d": "input", "q": "output"}}, "connections": {{"clk": [2], "fclk": [4], "d": [3], "q": [{}]}}}}"#,
            30_000 + k
        ));
    }
    let json = format!(
        r#"{{"modules": {{"ip": {{"attributes": {{"blackbox": "00000000000000000000000000000001"}},
              "ports": {{"clk": {{"direction": "input", "bits": [2]}}, "fclk": {{"direction": "input", "bits": [4]}}, "d": {{"direction": "input", "bits": [3]}}, "q": {{"direction": "output", "bits": [5]}}}}}},
            "top": {{"attributes": {{"top": "00000000000000000000000000000001"}}, "ports": {{}}, "cells": {{{}}},
              "netnames": {{"clk": {{"bits": [2]}}, "fclk": {{"bits": [4]}}, "d": {{"bits": [3]}}}}}}}}}}"#,
        cells.join(",\n")
    );
    let g = read(&json, &UNIT).unwrap();
    let wide: Vec<String> = g.wide(64, 512).into_iter().map(|w| w.name).collect();
    assert_eq!(wide, ["d"], "the clocks are no wide nets; the boxes' data input is");
}
