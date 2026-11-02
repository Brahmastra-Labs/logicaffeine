//! The floorplan read off a fitted build (`codegen_sva::floorplan`), held to hybrid3's reports (excerpts in
//! `fixtures/d5005_floorplan`): its four EMIF user clocks leave their banks' IO tile controls at (61, 36), (61, 360),
//! (233, 36) and (233, 360); channel k runs on bank 3 − k's clock; each channel needs about 1,240 ALMs and 39 M20Ks; the
//! persona's region is six rectangles. The floorplan puts each channel in the least square that holds it at half fill,
//! against its bank, inside the persona's region (or the whole device for a flat build), no two overlapping, the same
//! every time, as Quartus settings naming only the channels.

mod common;

use logicaffeine_compile::codegen_sva::floorplan::{
    clock_site, clock_targets, device, fast_signal_sites, instance_clock, instance_need, parent_region, region, tcl, Assignment, Need, Rect,
};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_floorplan").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const SHELL: &str = "afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell";

fn channel(k: usize) -> String {
    format!("{SHELL}|ch[{k}].channel")
}

fn bank_clock(b: usize) -> String {
    format!("emif_top_inst|mem_bank[{b}].emif_ddr4_inst|emif_s10_0_core_usr_clk")
}

#[test]
fn each_emif_user_clock_comes_from_its_banks_io_tile_control() {
    let fast = fast_signal_sites(&fixture("hybrid3.fit.place.rpt"));
    let clocks = clock_targets(&fixture("hybrid3.clocks.rpt"));
    let want = [(61, 36), (61, 360), (233, 36), (233, 360)];
    for (b, site) in want.iter().enumerate() {
        let targets = &clocks.iter().find(|(c, _)| *c == bank_clock(b)).unwrap_or_else(|| panic!("bank {b}'s clock in the table")).1;
        assert_eq!(clock_site(targets, &fast), Some(*site), "bank {b}");
    }
}

#[test]
fn each_channel_runs_on_the_clock_of_the_bank_opposite_its_index_and_needs_what_the_fitter_counted() {
    let (place, setup) = (fixture("hybrid3.fit.place.rpt"), fixture("hybrid3.setup.rpt"));
    for k in 0..4 {
        assert_eq!(instance_clock(&setup, &channel(k)), Some(bank_clock(3 - k)), "ch[{k}]");
        let need = instance_need(&place, &channel(k)).unwrap_or_else(|| panic!("ch[{k}]'s row"));
        assert!((1200.0..1300.0).contains(&need.alms) && need.m20k == 39, "ch[{k}]: {need:?}");
    }
    assert_eq!(instance_need(&place, &channel(0)), Some(Need { alms: 1227.8, m20k: 39 }));
    assert_eq!(instance_clock(&setup, SHELL), None, "no path lies inside a name that is no instance's prefix alone");
}

/// The four channels' regions under `parent`, the persona region's density, at half fill.
fn plan(parent: &[Rect]) -> Vec<Assignment> {
    let (place, setup) = (fixture("hybrid3.fit.place.rpt"), fixture("hybrid3.setup.rpt"));
    let fast = fast_signal_sites(&place);
    let clocks = clock_targets(&fixture("hybrid3.clocks.rpt"));
    let (_, density) = parent_region(&place).unwrap();
    (0..4)
        .map(|k| {
            let clock = instance_clock(&setup, &channel(k)).unwrap();
            let site = clock_site(&clocks.iter().find(|(c, _)| *c == clock).unwrap().1, &fast).unwrap();
            let need = instance_need(&place, &channel(k)).unwrap();
            let rect = region(site, need, density, 0.5, parent).unwrap_or_else(|| panic!("ch[{k}] has a region"));
            Assignment { instances: vec![channel(k)], name: format!("logos_ch{k}"), rect }
        })
        .collect()
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

#[test]
fn the_persona_floorplan_puts_each_channel_in_a_square_against_its_bank_inside_the_persona() {
    let place = fixture("hybrid3.fit.place.rpt");
    let (parent, density) = parent_region(&place).unwrap();
    assert_eq!(parent.len(), 6, "the persona's six rectangles");
    assert_eq!(parent[0], Rect { x0: 240, y0: 0, x1: 280, y1: 430 });
    // 601,050 ALMs and 7,583 M20Ks over 72,691 sites.
    let sites: u64 = parent.iter().map(Rect::sites).sum();
    assert_eq!(sites, 72_691);
    assert!((density.alms - 601_050.0 / 72_691.0).abs() < 1e-9 && (density.m20k - 7_583.0 / 72_691.0).abs() < 1e-9, "{density:?}");
    let plan = plan(&parent);
    let anchors = [(233, 360), (233, 36), (61, 360), (61, 36)];
    for (a, &anchor) in plan.iter().zip(&anchors) {
        let r = a.rect;
        let side = r.x1 - r.x0 + 1;
        assert_eq!(side, r.y1 - r.y0 + 1, "{}: a square", a.name);
        // 39 M20Ks at half fill: 39 / (0.5 × 7,583 / 72,691) = 747.7 sites, a side of 28.
        assert_eq!(side, 28, "{}: {r:?}", a.name);
        for x in r.x0..=r.x1 {
            for y in r.y0..=r.y1 {
                assert!(parent.iter().any(|p| p.contains(x, y)), "{}: ({x}, {y}) of {r:?} inside the persona", a.name);
            }
        }
        // No square of that side inside the persona lies nearer the bank.
        let nearest = region(anchor, instance_need(&place, &a.instances[0]).unwrap(), density, 0.5, &parent).unwrap();
        assert_eq!(r.distance(anchor), nearest.distance(anchor));
        assert!(r.distance(anchor) <= 20, "{}: {r:?} within reach of its bank at {anchor:?}", a.name);
    }
    for (i, a) in plan.iter().enumerate() {
        for b in &plan[i + 1..] {
            assert!(!overlap(&a.rect, &b.rect), "{} and {} apart: {:?} {:?}", a.name, b.name, a.rect, b.rect);
        }
    }
    assert_eq!(plan, self::plan(&parent), "the same floorplan every time");
}

#[test]
fn a_flat_floorplan_takes_the_whole_device_and_holds_each_channel_at_its_banks_tile() {
    let place = fixture("hybrid3.fit.place.rpt");
    let whole = device(&place).unwrap();
    assert_eq!(whole, Rect { x0: 0, y0: 0, x1: 282, y1: 432 });
    let plan = plan(&[whole]);
    let anchors = [(233, 360), (233, 36), (61, 360), (61, 36)];
    for (a, &anchor) in plan.iter().zip(&anchors) {
        assert_eq!(a.rect.distance(anchor), 0, "{}: its bank's tile inside its square: {:?}", a.name, a.rect);
    }
    for (i, a) in plan.iter().enumerate() {
        for b in &plan[i + 1..] {
            assert!(!overlap(&a.rect, &b.rect), "{} and {} apart", a.name, b.name);
        }
    }
}

#[test]
fn the_settings_name_each_channel_once_in_its_own_region() {
    let place = fixture("hybrid3.fit.place.rpt");
    let (parent, _) = parent_region(&place).unwrap();
    let plan = plan(&parent);
    let text = tcl(&plan);
    assert_eq!(text.lines().count(), 4 * 4);
    for (k, a) in plan.iter().enumerate() {
        let r = a.rect;
        assert!(text.contains(&format!("set_instance_assignment -name PLACE_REGION \"X{} Y{} X{} Y{}\" -to {}\n", r.x0, r.y0, r.x1, r.y1, channel(k))));
        assert!(text.contains(&format!("set_instance_assignment -name REGION_NAME logos_ch{k} -to {}\n", channel(k))));
        assert_eq!(text.matches(&format!("-to {}\n", channel(k))).count(), 4, "ch[{k}]: its four settings");
    }
    assert!(text.lines().all(|l| l.contains(" -to afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[")), "the channels alone");
    assert!(!text.contains("RESERVE_PLACE_REGION ON"), "the rest of the design may share the sites");
}

/// A memory's M20K blocks are the fewest over the block's simple-dual-port shapes: blocks side by side for its width,
/// stacked for its depth.
#[test]
fn a_memory_takes_the_fewest_m20k_blocks_its_shape_allows() {
    use logicaffeine_compile::codegen_sva::floorplan::m20k_blocks;
    assert_eq!(m20k_blocks(512, 512), 13, "512 deep, 512 wide: thirteen 512 x 40 blocks");
    assert_eq!(m20k_blocks(64, 512), 13, "a shallow wide memory takes as many");
    assert_eq!(m20k_blocks(2048, 10), 1);
    assert_eq!(m20k_blocks(4096, 10), 2);
    assert_eq!(m20k_blocks(1024, 40), 2);
    assert_eq!(m20k_blocks(1024, 20), 1, "1,024 x 20");
    assert_eq!(m20k_blocks(1, 1), 1);
}

/// A unit's needs as Quartus synthesized it: hybrid7b's channel 2 is 1,812 ALUTs and 2,572 registers, and its four
/// 512-bit FIFOs in M20K are 52 blocks, the 52 the fitter later required of its region; its MLAB FIFOs take none.
#[test]
fn a_units_needs_are_its_synthesis_its_memories_counted_by_shape() {
    use logicaffeine_compile::codegen_sva::floorplan::synthesized_needs;
    use logicaffeine_compile::codegen_sva::preflight::Packing;
    let syn = fixture("hybrid7b.syn.rpt");
    let packing = Packing { per_alut: 0.6, per_register: Some(0.7) };
    let needs = synthesized_needs(&syn, &[channel(2)], &packing).expect("channel 2's needs");
    assert_eq!((needs.m20k, needs.dsp, needs.registers), (52, 0, 2_572.0));
    assert!((needs.alms - (0.6f64 * 1_812.0).max(0.7 * 2_572.0)).abs() < 1e-9, "{}", needs.alms);
    assert_eq!(synthesized_needs(&syn, &[channel(7)], &packing), Err(channel(7)), "an instance the report does not name");
}

/// The wires a floorplan keeps short are read off the module whose instances it places: each pair of its instances and
/// the bits one drives and the other reads, every other module read by its ports alone so nothing under them is
/// elaborated; the module's clock, which every instance reads and none drives, joins no two of them.
#[test]
fn a_modules_graph_is_the_bits_each_pair_of_its_instances_share() {
    use logicaffeine_compile::codegen_sva::netlist::module_graph;
    if !common::require_tool("yosys") {
        return;
    }
    let verilog = "module producer(input clk, output reg [7:0] y, output reg flag);
  always @(posedge clk) begin y <= y + 8'd1; flag <= ~flag; end
endmodule
module middle(input clk, input [7:0] a, output reg [3:0] y);
  always @(posedge clk) y <= a[3:0] ^ a[7:4];
endmodule
module consumer(input clk, input [3:0] a, input f, output reg z);
  always @(posedge clk) z <= ^a ^ f;
endmodule
module hub(input clk, output z);
  wire [7:0] wide;
  wire [3:0] narrow;
  wire flag;
  producer p(.clk(clk), .y(wide), .flag(flag));
  middle m(.clk(clk), .a(wide), .y(narrow));
  consumer c(.clk(clk), .a(narrow), .f(flag), .z(z));
endmodule
";
    let links = module_graph(verilog, "hub", &[]).unwrap_or_else(|e| panic!("{e}")).links;
    let bits = |a: &str, b: &str| links.iter().find(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)).map_or(0, |l| l.bits);
    assert_eq!(bits("p", "m"), 8, "the bus");
    assert_eq!(bits("m", "c"), 4);
    assert_eq!(bits("p", "c"), 1, "the flag");
    assert_eq!(links.len(), 3, "{links:?}");
    assert!(module_graph(verilog, "absent", &[]).is_err(), "a module the Verilog does not define");
}

/// A floorplan's wires run through the module's own logic too: a register stage and a multiplexer between two instances
/// leave them a wire as wide as their bus, and a select steering a bus is one bit, the fewer of the bits leaving one side
/// and the bits arriving at the other. The modules named are dissolved into the module, their instances named by path
/// (`w|k`; a generate block's `w|pick[0].c`), and each instance's bits to the module's own ports are counted the same way.
#[test]
fn a_graph_reads_through_the_modules_logic_and_the_modules_it_dissolves() {
    use logicaffeine_compile::codegen_sva::netlist::module_graph;
    if !common::require_tool("yosys") {
        return;
    }
    let verilog = "module src(input clk, output reg [15:0] y);
  always @(posedge clk) y <= y + 16'd1;
endmodule
module alt(input clk, output reg [15:0] y);
  always @(posedge clk) y <= y - 16'd1;
endmodule
module ctl(input clk, output reg go);
  always @(posedge clk) go <= ~go;
endmodule
module snk(input clk, input [15:0] a, output reg [3:0] z);
  always @(posedge clk) z <= a[3:0] ^ a[15:12];
endmodule
module wrap(input clk, input [3:0] host, output [3:0] z);
  wire [15:0] s, t;
  wire go;
  reg [15:0] staged;
  src s0(.clk(clk), .y(s));
  alt s1(.clk(clk), .y(t));
  genvar i;
  generate for (i = 0; i < 1; i = i + 1) begin : pick
    ctl c(.clk(clk), .go(go));
  end endgenerate
  always @(posedge clk) staged <= go ? s : t;
  snk k(.clk(clk), .a(staged ^ {12'd0, host}), .z(z));
endmodule
module top(input clk, input [3:0] host, output [3:0] z);
  wrap w(.clk(clk), .host(host), .z(z));
endmodule
";
    let graph = module_graph(verilog, "top", &["wrap"]).unwrap_or_else(|e| panic!("{e}"));
    let bits = |a: &str, b: &str| graph.links.iter().find(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)).map_or(0, |l| l.bits);
    assert_eq!(bits("w|s0", "w|k"), 16, "the bus through the multiplexer and the register stage");
    assert_eq!(bits("w|s1", "w|k"), 16, "the multiplexer's other input");
    assert_eq!(bits("w|pick[0].c", "w|k"), 1, "the select is one bit, though it steers all sixteen");
    assert_eq!(bits("w|s0", "w|s1"), 0, "two inputs of one multiplexer, each reading only the clock of the other's");
    assert_eq!(graph.links.len(), 3, "{:?}", graph.links);
    let ports = |x: &str| graph.ports.iter().find(|(n, _)| n == x).map_or(0, |(_, b)| *b);
    assert_eq!(ports("w|k"), 1 + 4 + 4, "the clock and z shared, host through the exclusive-or");
    assert_eq!((ports("w|s0"), ports("w|s1"), ports("w|pick[0].c")), (1, 1, 1), "the clock alone");
    assert_eq!(graph.ports.len(), 4, "{:?}", graph.ports);
    assert_eq!(graph.instances, ["w|k", "w|pick[0].c", "w|s0", "w|s1"], "every instance, in name order");
    let whole = module_graph(verilog, "top", &[]).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(whole.links, Vec::new(), "nothing dissolved: one instance, no pair");
    assert_eq!(whole.ports, vec![("w".to_string(), 1 + 4 + 4)]);
    assert!(module_graph(verilog, "top", &["absent"]).is_err(), "a module to dissolve the Verilog does not define");
}

/// Each channel's anchor is where its bank's user clock leaves the bank's IO tile control.
#[test]
fn each_channels_anchor_is_where_its_banks_clock_leaves_the_io_tile_control() {
    use logicaffeine_compile::codegen_sva::floorplan::{anchor, Anchor, AnchorError};
    let (place, clocks, setup) = (fixture("hybrid3.fit.place.rpt"), fixture("hybrid3.clocks.rpt"), fixture("hybrid3.setup.rpt"));
    let sites = [(61, 36), (61, 360), (233, 36), (233, 360)];
    for k in 0..4 {
        let want = Anchor { instance: channel(k), clock: bank_clock(3 - k), site: sites[3 - k] };
        assert_eq!(anchor(&place, &clocks, &setup, &channel(k)), Ok(want), "ch[{k}]");
    }
    assert_eq!(anchor(&place, &clocks, &setup, SHELL), Err(AnchorError::NoClock(SHELL.into())), "no path lies inside the shell's name alone");
    let unlisted = clocks.replace(&bank_clock(3), "renamed");
    assert_eq!(anchor(&place, &unlisted, &setup, &channel(0)), Err(AnchorError::NoTargets(bank_clock(3))), "a clock the table lacks");
}

/// The anchors ledger holds what hybrid3's reports place: every channel, its bank's clock and that clock's site, recorded
/// by `d5005_anchors` on a day.
#[test]
fn the_anchors_ledger_is_what_hybrid3s_reports_place() {
    use logicaffeine_compile::codegen_sva::floorplan::{anchor, Anchors};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/emif_anchors.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ledger: Anchors = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ledger.build, "hybrid3");
    let (place, clocks, setup) = (fixture("hybrid3.fit.place.rpt"), fixture("hybrid3.clocks.rpt"), fixture("hybrid3.setup.rpt"));
    let placed: Vec<_> = (0..4).map(|k| anchor(&place, &clocks, &setup, &channel(k)).unwrap()).collect();
    assert_eq!(ledger.anchors, placed);
    let r = ledger.recorded.as_bytes();
    assert!(r.len() == 10 && r[4] == b'-' && r[7] == b'-' && r.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()), "{}", ledger.recorded);
    assert!(ledger.what.contains("d5005_anchors"), "the ledger names what records it");
}

/// A stack of M20K blocks is a power of two deep: 1,088 words take a stack of 2,048, 32,064 one of 32,768.
#[test]
fn a_stack_of_m20k_blocks_is_a_power_of_two_deep() {
    use logicaffeine_compile::codegen_sva::floorplan::m20k_blocks;
    assert_eq!(m20k_blocks(1_088, 512), 52, "2,048 words: two 1,024 x 20 blocks deep, or four of 512 x 40");
    assert_eq!(m20k_blocks(32_064, 512), 832);
    assert_eq!(m20k_blocks(513, 40), 2);
    assert_eq!(m20k_blocks(0, 0), 1, "a memory takes a block");
}

/// The M20Ks hybrid7b's synthesis says its store and its attention side need are the ones its fit gave them: 1,024 for
/// the store, its 32,064 words of 512 bits with byte enables; 45 for the attention side, its small memories in MLABs.
#[test]
fn hybrid7bs_store_and_attention_side_need_the_m20ks_its_fit_gave_them() {
    use logicaffeine_compile::codegen_sva::floorplan::synthesized_needs;
    use logicaffeine_compile::codegen_sva::preflight::Packing;
    let syn = fixture("hybrid7b.syn.rpt");
    let packing = Packing { per_alut: 0.6, per_register: Some(0.7) };
    let unit = |name: &str| synthesized_needs(&syn, &[format!("{SHELL}|engine|fast|{name}")], &packing).unwrap_or_else(|e| panic!("{e}'s needs"));
    assert_eq!(unit("store").m20k, 1_024);
    assert_eq!(unit("aside").m20k, 45);
    assert_eq!(synthesized_needs(&syn, &[channel(2)], &packing).map(|n| n.m20k), Ok(52), "the channel's FIFOs as before");
}

/// A clock is no wire: a register an instance clocks ties that instance to nothing it feeds, the module's clock two
/// instances read joins neither to the other, and a net one drives and the other reads is counted once, though it also
/// reaches the one through a register.
#[test]
fn a_graph_counts_no_clock_pin_and_no_shared_net_twice() {
    use logicaffeine_compile::codegen_sva::netlist::module_graph;
    if !common::require_tool("yosys") {
        return;
    }
    let verilog = "module clkgen(input clk, output reg c);
  always @(posedge clk) c <= ~c;
endmodule
module src(input clk, output reg [7:0] y);
  always @(posedge clk) y <= y + 8'd1;
endmodule
module snk(input clk, input [7:0] a, output reg z);
  always @(posedge clk) z <= ^a;
endmodule
module pair(input clk, input [3:0] p, input [3:0] q, output reg z);
  always @(posedge clk) z <= ^(p ^ q);
endmodule
module top(input clk, output z, output w);
  wire c;
  wire [7:0] y;
  reg [7:0] r;
  reg [3:0] s;
  clkgen g(.clk(clk), .c(c));
  src a(.clk(clk), .y(y));
  always @(posedge c) r <= y;
  snk b(.clk(clk), .a(r), .z(z));
  always @(posedge clk) s <= y[3:0];
  pair d(.clk(clk), .p(y[3:0]), .q(s), .z(w));
endmodule
";
    let graph = module_graph(verilog, "top", &[]).unwrap_or_else(|e| panic!("{e}"));
    let bits = |a: &str, b: &str| graph.links.iter().find(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)).map_or(0, |l| l.bits);
    assert_eq!(bits("g", "b"), 0, "the register g clocks carries nothing of g's to b");
    assert_eq!(bits("a", "b"), 8, "the bus through the register");
    assert_eq!(bits("a", "d"), 4, "the four bits d reads straight off a, not again through s");
}

/// The geometry the probe measured, `geometry-probe-2`'s fit in the build ledger: hybrid3's device in tiles of 16 columns
/// by 24 rows but the ten tiles probe-1's fit found no flip-flop in (its refusals, in the fixtures), each measured once,
/// together holding exactly the device's ALMs, M20K and DSP blocks, four registers an ALM.
#[test]
fn the_probes_tiles_are_the_device_but_its_fabricless_tiles_and_hold_all_of_it() {
    use logicaffeine_compile::codegen_sva::floorplan::{fabricless, tiles, Capacity, Geometry};
    use logicaffeine_compile::codegen_sva::preflight::DEVICE;
    use logicaffeine_compile::codegen_sva::quartus::Ledger;
    let builds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/quartus_builds.json");
    let ledger = Ledger::load(&builds).unwrap();
    let row = ledger.rows.iter().find(|r| r.name == "geometry-probe-2").expect("the probe's row in the build ledger");
    assert!(row.verdict.passed, "the probe fitted: {:?}", row.verdict);
    let geometry = Geometry::of(&row.build.regions).expect("every tile counted");
    let without = fabricless(&fixture("geometry-probe-1.fit.log"));
    assert_eq!(without.len(), 10, "probe-1's ten refusals");
    let key = |r: &Rect| (r.y0, r.x0, r.y1, r.x1);
    let mut expected: Vec<Rect> = tiles(device(&fixture("hybrid3.fit.place.rpt")).unwrap(), 16, 24).into_iter().filter(|t| !without.contains(t)).collect();
    let mut measured: Vec<Rect> = geometry.tiles.iter().map(|(t, _)| *t).collect();
    expected.sort_by_key(key);
    measured.sort_by_key(key);
    assert_eq!(measured.len(), 18 * 19 - 10);
    assert_eq!(measured, expected, "every tile of the device but the fabricless ones, each once");
    let total = geometry.tiles.iter().fold(Capacity::default(), |c, (_, t)| Capacity {
        alms: c.alms + t.alms,
        registers: c.registers + t.registers,
        m20k: c.m20k + t.m20k,
        dsp: c.dsp + t.dsp,
    });
    assert_eq!((total.alms, total.m20k, total.dsp), (DEVICE.alms, DEVICE.m20k, DEVICE.dsp), "the 1SX280's own counts");
    for (t, c) in &geometry.tiles {
        assert_eq!(c.registers, 4 * c.alms, "{t:?}: four registers an ALM");
    }
}

/// A broadcast is a star from its driver: each reader of a bus is wired to the end that drives it by the bus's width,
/// and two readers of one bus, or of the module's clock, are no wire to each other, so sixteen cores reading one
/// activation bus pull toward its driver and not each into the other fifteen.
#[test]
fn a_broadcast_is_a_star_from_its_driver() {
    use logicaffeine_compile::codegen_sva::netlist::module_graph;
    if !common::require_tool("yosys") {
        return;
    }
    let verilog = "module drv(input clk, output reg [7:0] y);
  always @(posedge clk) y <= y + 8'd1;
endmodule
module rd(input clk, input [7:0] a, output reg z);
  always @(posedge clk) z <= ^a;
endmodule
module fan(input clk, output [2:0] z);
  wire [7:0] bus;
  drv d(.clk(clk), .y(bus));
  rd r0(.clk(clk), .a(bus), .z(z[0]));
  rd r1(.clk(clk), .a(bus), .z(z[1]));
  rd r2(.clk(clk), .a(bus), .z(z[2]));
endmodule
";
    let graph = module_graph(verilog, "fan", &[]).unwrap_or_else(|e| panic!("{e}"));
    let bits = |a: &str, b: &str| graph.links.iter().find(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a)).map_or(0, |l| l.bits);
    for r in ["r0", "r1", "r2"] {
        assert_eq!(bits("d", r), 8, "{r}: the bus from its driver");
    }
    assert_eq!((bits("r0", "r1"), bits("r0", "r2"), bits("r1", "r2")), (0, 0, 0), "{:?}", graph.links);
    assert_eq!(graph.links.len(), 3, "{:?}", graph.links);
    let ports = |x: &str| graph.ports.iter().find(|(n, _)| n == x).map_or(0, |(_, b)| *b);
    assert_eq!((ports("d"), ports("r0")), (1, 1 + 1), "the clock from the module's port, and each reader's output");
}
