//! qor_power_lock — a Sky130 power figure is the activity its gate-level simulation measured, read over whole
//! operations and charged to the cells that burned it.
//!
//! The comparative-synthesis bench measures each candidate's energy per operation on its power block: the candidate
//! inside its functional harness, hardened whole (`scripts/qor/record-qor-power.py`), its activity read by OpenSTA from
//! a gate-level VCD through `scripts/measure-silicon-power.py`. The tests hold:
//!
//! A. the VCD is written as OpenSTA's reader reads it: a cell scope whose name carries the hierarchy divider has it
//!    escaped, and a variable with an escaped name — a net, never a pin — is left out with its value changes, unless
//!    another variable shares its identifier; a VCD that needs neither is read as it is;
//! B. a window is the values held at its start, as the initial dump at time 0, and the changes within it moved back by
//!    its start, closed by a time step at its end; a window the simulation did not reach is refused;
//! C. a report proves it used every annotation: each annotated pin was set by the activity walk, seeded as an input,
//!    or is a clock pin carrying the clock's own activity, and any other is named as dropped;
//! D. the window opens once the pipeline is full and holds whole operations, at least 64 and 4000 cycles' worth;
//! E. every cell of a routed netlist is classed by what its nets reach: the candidate's, the buffers inside it, the
//!    clock tree with its sinks on each side, the harness's, the buffers inside the harness, the boundary, physical;
//! F. the candidate's energy per operation is its cells', its buffers' and its share of the clock tree's, over the
//!    window's length and operations; the classes must sum to the report, and an excerpt gives back every figure;
//! G. end to end, on a self-driving sky130 netlist: every annotation is used, energy over a window is the sum of its
//!    halves', a window without activity reads less — and the pinned OpenLane image's OpenSTA drops annotations on the
//!    same netlist, the defect the build of `scripts/opensta/` is for, while under one global activity, where no walk
//!    runs, the two report the same power to every printed digit; and a register that holds its own value, which no
//!    path from an input reaches, reads its VCD activity too;
//! H. a K7 controller's power block is driven by the traffic gate's testbench: the controller swapped for the powered
//!    block at the block's clock, the stream from its first accepted request to its last delivered word dumped, and
//!    the block's run writing the RTL run's bus log line for line; a testbench that does not instantiate the
//!    controller as the path expects is refused.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::codegen_sva::qor_harness::{emit_harness, kernel, logos_candidate, Mode};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `body` (Python) with `scripts/measure-silicon-power.py` imported as `msp` and `scripts/qor/record-qor-power.py`
/// as `rqp`, and `work` a scratch directory under the checkout's `logs/` (which the OpenSTA container mounts); return
/// what it prints.
fn drive(body: &str) -> String {
    let (out, ok) = drive_status(body);
    assert!(ok, "{out}");
    out
}

fn drive_status(body: &str) -> (String, bool) {
    let logs = root().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let work = tempfile::Builder::new().prefix("qor_power_lock").tempdir_in(&logs).expect("scratch under logs/");
    let prelude = r#"
import importlib.util, json, os, sys
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m
msp = load("msp", sys.argv[1])
rqp = load("rqp", sys.argv[2])
work = sys.argv[3]
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("{prelude}\n{body}"))
        .arg(root().join("scripts/measure-silicon-power.py"))
        .arg(root().join("scripts/qor/record-qor-power.py"))
        .arg(work.path())
        .output()
        .expect("python3 runs");
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string() + &String::from_utf8_lossy(&out.stderr);
    (text.trim().to_string(), out.status.success())
}

/// A VCD as iverilog writes a kept hierarchy's flat netlist: cell scopes named `u_dut/_7_`, the netlist's escaped
/// nets as variables of the block, one of them an alias of the clock's identifier.
const VCD: &str = r#"$date today $end
$timescale 1ps $end
$scope module tb $end
$var reg 1 ! clk $end
$scope module dut $end
$var wire 1 ! clk $end
$var wire 4 # sig [3:0] $end
$var wire 4 $ \u_dut/x [3:0] $end
$var wire 1 % \u_dut/y $end
$var wire 1 ! \u_dut/clkalias $end
$scope module u_dut/_7_ $end
$var wire 1 & A $end
$var wire 1 ' Y $end
$upscope $end
$scope module _8_ $end
$var wire 1 ( A $end
$upscope $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
0!
b0 #
b0 $
x%
0&
1'
0(
$end
#5000
1!
b1 #
b1 $
1%
1&
0'
#10000
0!
1(
#15000
1!
b10 #
0&
1'
#20000
0!
"#;

/// The definitions and body of a VCD: (scope and variable lines, every line after `$enddefinitions`).
fn split(vcd: &str) -> (Vec<String>, Vec<String>) {
    let (head, body) = vcd.split_once("$enddefinitions $end\n").expect("definitions end");
    (
        head.lines().filter(|l| l.starts_with("$scope") || l.starts_with("$var")).map(String::from).collect(),
        body.lines().map(String::from).collect(),
    )
}

fn written(window: &str) -> String {
    drive(&format!(
        "open(os.path.join(work, 'in.vcd'), 'w').write({vcd:?})\n\
         out = msp.sta_vcd(os.path.join(work, 'in.vcd'), os.path.join(work, 'out.vcd'){window})\n\
         print(open(out).read())",
        vcd = VCD
    ))
}

#[test]
fn a_vcd_is_written_as_openstas_reader_reads_it() {
    let (defs, body) = split(&written(""));
    assert_eq!(
        defs,
        [
            "$scope module tb $end",
            "$var reg 1 ! clk $end",
            "$scope module dut $end",
            "$var wire 1 ! clk $end",
            "$var wire 4 # sig [3:0] $end",
            r"$scope module u_dut\/_7_ $end",
            "$var wire 1 & A $end",
            "$var wire 1 ' Y $end",
            "$scope module _8_ $end",
            "$var wire 1 ( A $end",
        ],
        "the cell scope has its divider escaped; the escaped nets are gone, the plain ones stay"
    );
    assert!(!body.iter().any(|l| l.ends_with(" $") || l == "x%" || l == "1%"), "no change of a dropped net: {body:?}");
    assert!(body.contains(&"1!".to_string()), "the clock's changes stay though an escaped alias shared its identifier");
    assert_eq!(body.iter().filter(|l| l.starts_with('#')).count(), 5, "every time step is kept");
    let same = drive(
        "open(os.path.join(work, 'in.vcd'), 'w').write('$timescale 1ps $end\\n$scope module tb $end\\n$scope module dut $end\\n\
         $var wire 1 ! clk $end\\n$upscope $end\\n$upscope $end\\n$enddefinitions $end\\n#0\\n0!\\n#5\\n1!\\n')\n\
         print(msp.sta_vcd(os.path.join(work, 'in.vcd'), os.path.join(work, 'out.vcd')) == os.path.join(work, 'in.vcd'), \
         os.path.exists(os.path.join(work, 'out.vcd')))",
    );
    assert_eq!(same, "True False", "a VCD that needs nothing is read as it is, never copied");
}

#[test]
fn a_window_is_the_values_held_at_its_start_and_the_changes_within_it_moved_to_zero() {
    let (_, body) = split(&written(", (7.5, 17.5)"));
    assert_eq!(
        body,
        ["#0", "$dumpvars", "1!", "b1 #", "1&", "0'", "0(", "$end", "#2500", "0!", "1(", "#7500", "1!", "b10 #", "0&", "1'", "#10000"],
        "the values at 7.5 ns as the initial dump, the changes at 10 and 15 ns at 2.5 and 7.5, closed at 10"
    );
    let (_, from_zero) = split(&written(", (0, 20)"));
    assert_eq!(from_zero.first().map(String::as_str), Some("#0"));
    let stamps: Vec<&String> = from_zero.iter().filter(|l| l.starts_with('#')).collect();
    assert_eq!(stamps, ["#0", "#5000", "#10000", "#15000", "#20000"], "a window ending on a time step closes on it, once");
    assert_eq!(from_zero.last().map(String::as_str), Some("0!"), "the changes of its last time step are inside it");
    let (out, ok) = drive_status(&format!(
        "open(os.path.join(work, 'in.vcd'), 'w').write({vcd:?})\n\
         msp.sta_vcd(os.path.join(work, 'in.vcd'), os.path.join(work, 'out.vcd'), (5, 30))",
        vcd = VCD
    ));
    assert!(!ok && out.contains("before the window's end at 30 ns"), "a window past the simulation is refused: {out}");
}

#[test]
fn a_report_uses_every_annotation_or_names_the_pins_it_dropped() {
    let proof = |trace: &str| drive(&format!("print(json.dumps(msp.activity_proof({trace:?}), sort_keys=True))"));
    let trace = "read_vcd_activities: clk transitions 8.0 activity 2.00 duty 0.50\n\
                 read_vcd_activities: rst transitions 1.0 activity 0.00 duty 0.01\n\
                 read_vcd_activities: _1_/A transitions 3.0 activity 0.75 duty 0.40\n\
                 read_vcd_activities: _1_/Y transitions 3.0 activity 0.75 duty 0.60\n\
                 read_vcd_activities: _2_/CLK transitions 8.0 activity 2.00 duty 0.50\n\
                 Annotated 4 pin activities.\n\
                 power_activity: seed rst\n\
                 power_activity: visit _1_/A\n\
                 power_activity: set _1_/A 7.50e-01 0.40\n\
                 power_activity: visit _1_/Y\n";
    assert_eq!(
        proof(trace),
        r#"{"annotated": 5, "clock": 2, "dropped": [], "hierarchical": 0, "seeded": ["rst"], "set": 2}"#,
        "the clock port and the flop's clock pin carry the clock's activity; nothing is dropped"
    );
    let dropped = trace.to_string() + "read_vcd_activities: _3_/A transitions 1.0 activity 0.25 duty 0.10\n\
                                       read_vcd_activities: _4_/A transitions 8.0 activity 2.00 duty 0.30\n";
    assert_eq!(
        proof(&dropped),
        r#"{"annotated": 7, "clock": 2, "dropped": ["_3_/A", "_4_/A"], "hierarchical": 0, "seeded": ["rst"], "set": 2}"#,
        "a pin the walk never set is dropped, and so is one toggling at the clock's rate off the clock's duty"
    );
    let kept = trace.to_string() + "LOGOS-HIER u_dut\n\
                                    read_vcd_activities: u_dut/a[0] transitions 3.0 activity 0.75 duty 0.40\n\
                                    read_vcd_activities: u_dut\\/_5_/A transitions 3.0 activity 0.75 duty 0.40\n";
    assert_eq!(
        proof(&kept),
        r#"{"annotated": 7, "clock": 2, "dropped": ["u_dut\\/_5_/A"], "hierarchical": 1, "seeded": ["rst"], "set": 2}"#,
        "a port of a hierarchical instance burns nothing; a flat cell whose name only starts like it is still a cell"
    );
}

#[test]
fn the_window_opens_on_a_full_pipeline_and_holds_whole_operations() {
    assert_eq!(
        drive("print([rqp.window_ops(f, s) for f, s in [(5, 1.0), (24, 21.0), (3030, 3028.0), (42, 1.0), (300, 2.736)]])"),
        "[(10, 4000), (8, 191), (8, 64), (84, 4000), (220, 1462)]",
        "K: twice the operations in flight at the first result, at least 8; N: 4000 cycles' worth, at least 64"
    );
}

/// A routed netlist in miniature: the candidate's cells under `u_dut/`, the harness's synthesized cells, and what the
/// flow inserted — a hold buffer inside the candidate, a clock tree, a buffer from the candidate to the harness's
/// capture register, input and output buffers, a tap, an antenna diode, a spare buffer on nothing.
const ROUTED: &str = r#"
def c(cell, *pins):
    return (cell, [tuple(p.split()) if len(p.split()) == 3 else (p.split()[0], p.split()[1], "") for p in pins])
cells = {
    "u_dut/_1_": c("sky130_fd_sc_hd__and2_1", "A input p0", "B input p1", "X output d0"),
    "u_dut/_2_": c("sky130_fd_sc_hd__dfxtp_1", "CLK input ckd", "D input d0h", "Q output q"),
    "hold3": c("sky130_fd_sc_hd__dlygate4sd3_1", "A input d0", "X output d0h"),
    "_10_": c("sky130_fd_sc_hd__dfxtp_1", "CLK input ckh", "D input n0", "Q output p0"),
    "_11_": c("sky130_fd_sc_hd__dfxtp_1", "CLK input ckh", "D input qb", "Q output s0"),
    "_12_": c("sky130_fd_sc_hd__nor2_1", "A input s0", "B input rstb", "Y output n0"),
    "_13_": c("sky130_fd_sc_hd__conb_1", "HI output p1"),
    "fanout4": c("sky130_fd_sc_hd__buf_2", "A input q", "X output qb"),
    "clkbuf_0_clk": c("sky130_fd_sc_hd__clkbuf_16", "A input clk", "X output ck0"),
    "clkbuf_1_0_clk": c("sky130_fd_sc_hd__clkbuf_8", "A input ck0", "X output ckd"),
    "clkbuf_1_1_clk": c("sky130_fd_sc_hd__clkbuf_8", "A input ck0", "X output ckh"),
    "input1": c("sky130_fd_sc_hd__clkbuf_1", "A input rst", "X output rstb"),
    "output2": c("sky130_fd_sc_hd__buf_2", "A input s0", "X output sig[0]"),
    "TAP_1": c("sky130_fd_sc_hd__tapvpwrvgnd_1"),
    "ANTENNA_1": c("sky130_fd_sc_hd__diode_2", "DIODE input p0"),
    "spare5": c("sky130_fd_sc_hd__buf_1", "A input", "X output"),
}
ports = {"clk": ("input", "clk"), "rst": ("input", "rst"), "sig[0]": ("output", "sig[0]")}
synthesized = {"_10_", "_11_", "_12_", "_13_"}
"#;

#[test]
fn every_cell_is_classed_by_what_its_nets_reach() {
    let got = drive(&format!(
        "{ROUTED}\nclasses, sinks = rqp.classify(cells, ports, synthesized)\nprint(json.dumps([classes, sinks], sort_keys=True))"
    ));
    let v: serde_json::Value = serde_json::from_str(&got).expect("json");
    let want = serde_json::json!([{
        "u_dut/_1_": "dut", "u_dut/_2_": "dut", "hold3": "dut_buffers",
        "_10_": "harness", "_11_": "harness", "_12_": "harness", "_13_": "harness",
        "fanout4": "boundary", "clkbuf_0_clk": "clock", "clkbuf_1_0_clk": "clock", "clkbuf_1_1_clk": "clock",
        "input1": "harness_buffers", "output2": "harness_buffers", "TAP_1": "physical", "ANTENNA_1": "physical",
        "spare5": "boundary"
    }, {"dut": 1, "harness": 2}]);
    assert_eq!(v, want, "the clock tree reaches the clock port and three flops, one of them the candidate's");
}

#[test]
fn the_candidates_energy_is_its_cells_its_buffers_and_its_share_of_the_clock_tree() {
    let excerpt = "LOGOS-WINDOW start_ns 1002.500 end_ns 2002.500 cycles 100 ops 100 warmup_ops 8 period_ns 10\n\
                   LOGOS-SINKS dut 3 harness 1\n\
                   LOGOS-PINS 900 18\n\
                   Found 0 unannotated drivers.\n\
                   Found 2 partially unannotated drivers.\n\
                   Annotated 850 pin activities.\n\
                   LOGOS-TABLE Total                7.500000e-04 6.850000e-04 1.000000e-09 1.435001e-03 100.0%\n\
                   LOGOS-CLASS dut 600 6.0e-04 4.0e-04 5.0e-10 1.000000500e-03\n\
                   LOGOS-CLASS dut_buffers 10 5.0e-05 5.0e-05 0.0 1.0e-04\n\
                   LOGOS-CLASS clock 12 1.0e-04 2.0e-04 0.0 3.0e-04\n\
                   LOGOS-CLASS harness 200 0.0 0.0 5.0e-10 5.0e-10\n\
                   LOGOS-CLASS harness_buffers 4 0.0 0.0 0.0 0.0\n\
                   LOGOS-CLASS boundary 2 0.0 3.5e-05 0.0 3.5e-05\n\
                   LOGOS-CLASS physical 50 0.0 0.0 0.0 0.0\n\
                   LOGOS-CLASS unclassified 0 0.0 0.0 0.0 0.0\n\
                   LOGOS-PROOF annotated 851 set 700 seeded rst hierarchical 1 clock 150 dropped 0\n";
    let got = drive(&format!(
        "fig = rqp.parse_excerpt({excerpt:?})\n\
         e = rqp.energies(fig, 1000.0, 100, {{'dut': 3, 'harness': 1}})\n\
         print(json.dumps([e, rqp.check(fig, 'toy'), fig['unannotated_drivers'], fig['partially_unannotated_drivers'], \
         fig['annotated_pins'], fig['design_pins'], fig['proof']], sort_keys=True))"
    ));
    let v: serde_json::Value = serde_json::from_str(&got).expect("json");
    assert_eq!(v[0]["clock_share"], 0.75);
    assert_eq!(v[0]["dut_pj"], 13.250005, "(1.0000005 mW + 0.1 mW + 3/4 of 0.3 mW) x 1000 ns / 100 ops");
    assert_eq!(v[0]["block_pj"], 14.35001);
    assert_eq!(v[0]["class_pj"]["boundary"], 0.35, "every class beside it, none subtracted");
    assert_eq!(v[1], serde_json::json!([]), "the classes sum to the report");
    assert_eq!((v[2].clone(), v[3].clone(), v[4].clone(), v[5].clone()), (0.into(), 2.into(), 850.into(), 918.into()));
    assert_eq!(
        v[6],
        serde_json::json!({"annotated": 851, "set": 700, "seeded": 1, "hierarchical": 1, "clock": 150, "dropped": 0}),
        "the row counts the pins the activity walk seeded; their names stay in the excerpt"
    );
    let wrong = excerpt.replace("LOGOS-CLASS boundary 2 0.0 3.5e-05 0.0 3.5e-05", "LOGOS-CLASS boundary 2 0.0 0.0 0.0 0.0")
        .replace("LOGOS-CLASS unclassified 0", "LOGOS-CLASS unclassified 3");
    assert_eq!(
        drive(&format!("print(json.dumps(rqp.check(rqp.parse_excerpt({wrong:?}), 'toy')))")),
        r#"["toy: 3 cells in no class", "toy: the classes sum to 1.400001e-03 W, the report to 1.435001e-03 W"]"#
    );
}

#[test]
fn power_read_from_a_window_is_the_windows_and_every_annotation_is_used() {
    if !common::require_tool("opensta-power") {
        return;
    }
    let logs = root().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let dir = tempfile::Builder::new().prefix("qor_power_lock_e2e").tempdir_in(&logs).unwrap();
    let k = kernel("K5").unwrap();
    let (c, files) = logos_candidate("K5").unwrap();
    let mut sources = Vec::new();
    for (name, text) in &files {
        std::fs::write(dir.path().join(name), text).unwrap();
        sources.push(dir.path().join(name).display().to_string());
    }
    let top = format!("qorbist_{}", c.name);
    std::fs::write(dir.path().join(format!("{top}.v")), emit_harness(&k, &c, Mode::Shaped)).unwrap();
    sources.push(dir.path().join(format!("{top}.v")).display().to_string());
    let lib = PathBuf::from(std::env::var("HOME").unwrap())
        .join(".ciel/sky130A/libs.ref/sky130_fd_sc_hd/lib/sky130_fd_sc_hd__tt_025C_1v80.lib");
    let net = dir.path().join("net.v");
    let synth = std::process::Command::new("yosys")
        .arg("-q")
        .arg("-p")
        .arg(format!(
            "read_verilog -sv {}; synth -flatten -top {top}; dfflibmap -liberty {l}; abc -liberty {l}; \
             hilomap -hicell sky130_fd_sc_hd__conb_1 HI -locell sky130_fd_sc_hd__conb_1 LO; opt_clean -purge; \
             write_verilog -noattr -noexpr -nohex -nodec {}",
            sources.join(" "),
            net.display(),
            l = lib.display()
        ))
        .output()
        .unwrap();
    assert!(synth.status.success(), "{}", String::from_utf8_lossy(&synth.stderr));
    let body = format!(
        r##"
import subprocess
top, net, lib = {top:?}, {net:?}, {lib:?}
d = os.path.dirname(net)
tb = os.path.join(d, "tb.v")
open(tb, "w").write('''`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  wire [15:0] sig_w;
  %s dut(.clk(clk), .rst(rst), .sig(sig_w));
  always #5 clk = ~clk;
  initial begin
    $dumpfile("%s"); $dumpvars(0, dut);
    repeat (5) @(posedge clk); @(negedge clk) rst = 0;
    repeat (400) @(posedge clk); @(negedge clk);
    $finish;
  end
endmodule
''' % (top, os.path.join(d, "g.vcd")))
subprocess.run(["iverilog", "-g2012", "-DFUNCTIONAL", "-I", msp.VMODELS, "-o", os.path.join(d, "g.vvp"), tb, net,
                msp.VMODELS + "/primitives.v", msp.VMODELS + "/sky130_fd_sc_hd.v"], check=True, capture_output=True)
subprocess.run(["vvp", os.path.join(d, "g.vvp")], check=True, capture_output=True, cwd=d)
image = msp.opensta()

def read(name, window, sta_image):
    vcd = msp.sta_vcd(os.path.join(d, "g.vcd"), os.path.join(d, name + ".vcd"), window)
    if name == "still":
        lines = open(vcd).read().split("\n")
        end = lines.index("$end", lines.index("$dumpvars"))
        open(vcd, "w").write("\n".join(lines[:end + 1] + ["#%d" % round((window[1] - window[0]) * 1000), ""]))
    script = os.path.join(d, name + ".tcl")
    open(script, "w").write("\n".join([f"read_liberty {{lib}}", f"read_verilog {{net}}", f"link_design {{top}}",
        "create_clock -name clk -period 10 [get_ports clk]"] + msp.TRACE_ON +
        [f"read_power_activities -scope tb/dut -vcd {{vcd}}"] + msp.WALK_ON +
        ['puts "LOGOS-VCD"', "report_power -digits 6"] + msp.TRACE_OFF + ["exit"]) + "\n")
    msp.opensta = lambda: sta_image
    text = msp.run_sta(script, os.path.join(d, name + ".log"))
    msp.opensta = lambda: image
    total = msp.parse_power(text.split("LOGOS-VCD", 1)[1])[0]["total"]
    return total["internal"] + total["switching"], len(msp.activity_proof(text)["dropped"])

ab, bc, ac, still = (read(n, w, image) for n, w in
                     [("ab", (1002.5, 2502.5)), ("bc", (2502.5, 4002.5)), ("ac", (1002.5, 4002.5)), ("still", (1002.5, 2502.5))])
PINNED = "ghcr.io/the-openroad-project/openlane:ff5509f65b17bfa4068d5336495ab1718987ff69"
pinned = read("pinned", (1002.5, 2502.5), PINNED)

def table(name, sta_image):
    script = os.path.join(d, name + ".tcl")
    open(script, "w").write("\n".join([f"read_liberty {{lib}}", f"read_verilog {{net}}", f"link_design {{top}}",
        "create_clock -name clk -period 10 [get_ports clk]",
        "set_power_activity -global -activity 0.1 -duty 0.5", 'puts "LOGOS-GLOBAL"', "report_power -digits 6", "exit"]) + "\n")
    msp.opensta = lambda: sta_image
    text = msp.run_sta(script, os.path.join(d, name + ".log"))
    msp.opensta = lambda: image
    return [l for l in text.split("LOGOS-GLOBAL", 1)[1].splitlines() if l.startswith(("Sequential", "Combinational", "Total"))]

print(json.dumps({{"dropped": [ab[1], bc[1], ac[1]], "sum": (ab[0] + bc[0]) * 1500, "whole": ac[0] * 3000,
                  "still_below": still[0] < ab[0] * 0.9, "pinned_dropped": pinned[1],
                  "same_tables": table("ours", image) == table("theirs", PINNED)}}))
"##,
        top = top,
        net = net.display().to_string(),
        lib = lib.display().to_string()
    );
    let v: serde_json::Value = serde_json::from_str(&drive(&body)).expect("json");
    assert_eq!(v["dropped"], serde_json::json!([0, 0, 0]), "every annotation is used in every window");
    let (sum, whole) = (v["sum"].as_f64().unwrap(), v["whole"].as_f64().unwrap());
    assert!((sum - whole).abs() <= 1e-4 * whole, "a window's dynamic energy is its halves': {sum} vs {whole}");
    assert_eq!(v["still_below"], true, "a window with no activity reads below one with the candidate at work");
    assert!(v["pinned_dropped"].as_u64().unwrap() > 0, "the pinned image's OpenSTA drops annotations here");
    assert_eq!(v["same_tables"], true, "where no activity walk runs, the build reports what the pinned image's OpenSTA does, to every digit");
}

/// A self-driving netlist in sky130 cells: a flop that toggles from reset, and a register `h` that holds its own value
/// through a buffer — logic no path from an input reaches, as a register file or a ROM held in flops is.
const HOLD: &str = "module hold(input clk, input rst, output o);
  wire rn, tq, tn, hq, hb;
  sky130_fd_sc_hd__inv_1 i0 (.A(rst), .Y(rn));
  sky130_fd_sc_hd__dfrtp_1 t (.CLK(clk), .D(tn), .RESET_B(rn), .Q(tq));
  sky130_fd_sc_hd__inv_1 i1 (.A(tq), .Y(tn));
  sky130_fd_sc_hd__dfxtp_1 h (.CLK(clk), .D(hb), .Q(hq));
  sky130_fd_sc_hd__clkbuf_1 b (.A(hq), .X(hb));
  sky130_fd_sc_hd__and2_1 a (.A(tq), .B(hq), .X(o));
endmodule
";

const HOLD_TB: &str = "`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  wire o_w;
  hold dut(.clk(clk), .rst(rst), .o(o_w));
  always #5 clk = ~clk;
  initial begin
    $dumpfile(\"g.vcd\"); $dumpvars(0, dut);
    repeat (5) @(posedge clk); @(negedge clk) rst = 0;
    repeat (100) @(posedge clk); @(negedge clk);
    $finish;
  end
endmodule
";

#[test]
fn a_register_holding_its_own_value_reads_its_vcd_activity() {
    if !common::require_tool("opensta-power") {
        return;
    }
    let lib = PathBuf::from(std::env::var("HOME").unwrap())
        .join(".ciel/sky130A/libs.ref/sky130_fd_sc_hd/lib/sky130_fd_sc_hd__tt_025C_1v80.lib");
    let body = format!(
        r##"
import subprocess
d, lib = work, {lib:?}
net, tb = os.path.join(d, "hold.v"), os.path.join(d, "tb.v")
open(net, "w").write({HOLD:?})
open(tb, "w").write({HOLD_TB:?})
subprocess.run(["iverilog", "-g2012", "-DFUNCTIONAL", "-I", msp.VMODELS, "-o", os.path.join(d, "g.vvp"), tb, net,
                msp.VMODELS + "/primitives.v", msp.VMODELS + "/sky130_fd_sc_hd.v"], check=True, capture_output=True)
subprocess.run(["vvp", os.path.join(d, "g.vvp")], check=True, capture_output=True, cwd=d)
vcd = msp.sta_vcd(os.path.join(d, "g.vcd"), os.path.join(d, "w.vcd"), (102.5, 1002.5))
script = os.path.join(d, "hold.tcl")
open(script, "w").write("\n".join([f"read_liberty {{lib}}", f"read_verilog {{net}}", "link_design hold",
    "create_clock -name clk -period 10 [get_ports clk]"] + msp.TRACE_ON +
    [f"read_power_activities -scope tb/dut -vcd {{vcd}}"] + msp.WALK_ON + ['puts "LOGOS-VCD"', "report_power"] +
    msp.TRACE_OFF + ["exit"]) + "\n")
proof = msp.activity_proof(msp.run_sta(script, os.path.join(d, "hold.log")))
print(json.dumps([proof["dropped"], proof["annotated"]]))
"##,
        lib = lib.display().to_string()
    );
    let v: serde_json::Value = serde_json::from_str(&drive(&body)).expect("json");
    assert!(v[1].as_u64().unwrap() >= 14, "every cell pin is annotated: {v}");
    assert_eq!(v[0], serde_json::json!([]), "the hold register's pins read the VCD's activity, not the default");
}

/// H. The traffic-driven power path on `ddr2_ctrl`, with a stand-in for the routed block: the pass wrapper around the
/// RTL, given the power ports OpenLane's netlist has.
#[test]
fn a_controllers_power_block_runs_the_traffic_gates_stream() {
    use logicaffeine_compile::codegen_sva::ddr2_traffic::{
        emit_ddr2_ctrl_traffic_testbench, emit_ddr2_device_verilog, k7_ddr2_ctrl_verilog, k7_timing, Pattern, Traffic,
    };
    use logicaffeine_compile::codegen_sva::qor_harness::{emit_pass_wrapper, Dir, DutPort, Reset};
    if !common::require_tool("iverilog") {
        return;
    }
    let logs = root().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let dir = tempfile::Builder::new().prefix("qor_power_lock_k7").tempdir_in(&logs).unwrap();
    let (rtl, gate) = (dir.path().join("rtl"), dir.path().join("gate"));
    std::fs::create_dir_all(&rtl).unwrap();
    let t = k7_timing(8);
    let traffic = Traffic { pattern: Pattern::Sequential, words: 32 };
    for (name, text) in [
        ("words.hex", traffic.rom()),
        ("ddr2_ctrl.v", k7_ddr2_ctrl_verilog()),
        ("ddr2_device.v", emit_ddr2_device_verilog(&t)),
        ("tb_qor_dram.v", emit_ddr2_ctrl_traffic_testbench(&t, &traffic)),
    ] {
        std::fs::write(rtl.join(name), text).unwrap();
    }
    let sim = |at: &std::path::Path, sources: &[&str]| {
        let b = std::process::Command::new("iverilog").current_dir(at).args(["-g2012", "-s", "tb_qor_dram", "-o", "sim.vvp"]).args(sources).output().unwrap();
        assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
        String::from_utf8_lossy(&std::process::Command::new("vvp").current_dir(at).arg("sim.vvp").output().unwrap().stdout).into_owned()
    };
    assert!(sim(&rtl, &["tb_qor_dram.v", "ddr2_ctrl.v", "ddr2_device.v"]).contains("PASS"));
    std::fs::create_dir_all(&gate).unwrap();
    for f in ["words.hex", "ddr2_device.v", "ddr2_ctrl.v"] {
        std::fs::copy(rtl.join(f), gate.join(f)).unwrap();
    }
    let block = "qorpwr_k7_logos_ddr2_ctrl_p10";
    let out = drive(&format!(
        "first, last, per = rqp.traffic_window({rtl:?}, 'cyc', 'dut', 10.0)\n\
         tb = open({rtl:?} + '/tb_qor_dram.v').read()\n\
         open({gate:?} + '/tb_qor_dram.v', 'w').write(rqp.traffic_testbench(tb, 'ddr2_ctrl', 'dut', {block:?}, 10.0, 'cyc', first, last))\n\
         print(first, last, per)\n\
         try:\n    rqp.traffic_testbench(tb.replace('ddr2_ctrl dut(', 'ddr2_ctrl other('), 'ddr2_ctrl', 'dut', {block:?}, 10.0, 'cyc', first, last)\n\
         except SystemExit as e:\n    print('REFUSED', e)",
        rtl = rtl.to_string_lossy(),
        gate = gate.to_string_lossy(),
    ));
    let mut lines = out.lines();
    let window: Vec<u64> = lines.next().unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
    assert_eq!(window[2], 1, "ddr2_ctrl runs at the DRAM clock");
    assert!(window[1] > window[0], "{out}");
    let log = std::fs::read_to_string(rtl.join("dram.log")).unwrap();
    let clocks = |tag: &str| -> Vec<u64> {
        log.lines().filter_map(|l| l.strip_prefix(tag)).map(|r| r.split_whitespace().next().unwrap().parse().unwrap()).collect()
    };
    assert_eq!(window[0], *clocks("Q ").iter().min().unwrap(), "the window opens at the first request the controller accepted");
    assert_eq!(window[1], *clocks("D ").iter().max().unwrap(), "and closes at the last word it delivered");
    assert!(lines.next().unwrap().starts_with("REFUSED"), "{out}");
    let ports: Vec<DutPort> = [
        ("clk", Dir::In, 1), ("rst", Dir::In, 1), ("req_valid", Dir::In, 1), ("req_we", Dir::In, 1), ("req_addr", Dir::In, 26),
        ("req_wdata", Dir::In, 16), ("req_ready", Dir::Out, 1), ("resp_valid", Dir::Out, 1), ("resp_rdata", Dir::Out, 16),
        ("cke", Dir::Out, 1), ("cs_n", Dir::Out, 1), ("ras_n", Dir::Out, 1), ("cas_n", Dir::Out, 1), ("we_n", Dir::Out, 1),
        ("ba", Dir::Out, 3), ("a", Dir::Out, 13), ("odt", Dir::Out, 1), ("wr_data", Dir::Out, 16), ("wr_en", Dir::Out, 1),
        ("rd_data", Dir::In, 16), ("rd_valid", Dir::In, 1),
    ]
    .iter()
    .map(|&(n, d, b)| DutPort { name: n.into(), dir: d, bits: b })
    .collect();
    let wrapper = emit_pass_wrapper(block, "ddr2_ctrl", "clk", Some(&Reset { port: "rst".into(), active_high: true }), &ports)
        .replace(&format!("module {block}(\n"), &format!("module {block}(\n  inout VPWR,\n  inout VGND,\n"));
    std::fs::write(gate.join("block.v"), wrapper).unwrap();
    let ran = sim(&gate, &["tb_qor_dram.v", "block.v", "ddr2_ctrl.v", "ddr2_device.v"]);
    assert_eq!(
        std::fs::read_to_string(gate.join("dram.log")).unwrap(),
        std::fs::read_to_string(rtl.join("dram.log")).unwrap(),
        "the block's run writes the RTL run's bus log"
    );
    let at = |tag: &str| -> f64 {
        let v = ran.lines().find_map(|l| l.strip_prefix(tag)).unwrap_or_else(|| panic!("{tag}\n{ran}"));
        v.trim().parse::<f64>().unwrap() / 1000.0
    };
    let (start, end) = (at("QOR-WINDOW-START "), at("QOR-WINDOW-END "));
    assert_eq!(end - start, (window[1] - window[0]) as f64 * 10.0, "the window is the stream's clocks at the block's 10 ns");
    assert!(gate.join("window.vcd").metadata().unwrap().len() > 0);
}

/// OpenSTA runs in a container that sees the checkout and, read-only, the target of each of its top-level symbolic
/// links: a copy of the repository whose EDA tree links the checkout's reads its runs' netlists through that link,
/// and the link must resolve inside the container as it does outside.
#[test]
fn opensta_sees_what_the_checkouts_links_point_at() {
    let out = drive(
        r#"
import os, tempfile, types
root = tempfile.mkdtemp()
elsewhere = tempfile.mkdtemp()
os.makedirs(os.path.join(root, "scripts"))
os.symlink(elsewhere, os.path.join(root, "linked"))
os.symlink(os.path.join(root, "scripts"), os.path.join(root, "inside"))
msp.ROOT = root
calls = []
msp.subprocess.run = lambda args, **k: calls.append(args) or types.SimpleNamespace(stdout="", stderr="")
msp.opensta = lambda: "logos-opensta:probe"
msp.run_sta(os.path.join(root, "x.tcl"), os.path.join(root, "x.log"))
args = calls[0]
mounts = [args[i + 1] for i, a in enumerate(args) if a == "-v"]
print(("%s:%s" % (root, root)) in mounts, ("%s:%s:ro" % (elsewhere, elsewhere)) in mounts,
      sum(1 for m in mounts if m.startswith(os.path.join(root, "scripts"))))
"#,
    );
    assert_eq!(
        out.lines().last().unwrap_or_default(),
        "True True 0",
        "the checkout, its link's target read-only, and no second mount for a link that stays inside it"
    );
}

/// The topology of a routed netlist reads every cell, its liberty pins and their nets, and every port, from OpenSTA: a
/// pin with no net — a tie cell's unused output (`conb_1` driving only `LO`; OpenSTA hands back a null net for `HI`) —
/// is read with no net rather than ending the read, and a flop's internal state pins are passed over.
#[test]
fn a_routed_netlists_topology_reads_a_pin_with_no_net() {
    if !common::require_tool("opensta-power") {
        return;
    }
    let out = drive(
        r#"
import os, tempfile
logs = os.path.join(rqp.qorlib.ROOT, "logs")
os.makedirs(logs, exist_ok=True)
run = tempfile.mkdtemp(dir=logs)
work = tempfile.mkdtemp(dir=logs)
gl = os.path.join(run, "results", "final", "verilog", "gl")
os.makedirs(gl)
open(os.path.join(gl, "toy.nl.v"), "w").write("""module toy(input clk, input d, output q, output z);
  wire n, lo;
  sky130_fd_sc_hd__dfxtp_1 r (.CLK(clk), .D(d), .Q(n));
  sky130_fd_sc_hd__buf_1 b (.A(n), .X(q));
  sky130_fd_sc_hd__conb_1 t (.LO(lo));
  sky130_fd_sc_hd__inv_1 i (.A(lo), .Y(z));
  sky130_fd_sc_hd__fill_1 FILLER_0 ();
endmodule
""")
cells, ports = rqp.topology("toy", run, work)
print(sorted(cells), sorted(p for p, _, _ in cells["r"][1]), sorted((p, n) for p, _, n in cells["t"][1]), sorted(ports))
"#,
    );
    assert_eq!(
        out.lines().last().unwrap_or_default(),
        "['FILLER_0', 'b', 'i', 'r', 't'] ['CLK', 'D', 'Q'] [('HI', ''), ('LO', 'lo')] ['clk', 'd', 'q', 'z']",
        "every cell and port; the tie cell's unused output read with no net; the flop's liberty pins alone"
    );
}

/// `all` measures every power block whose recorded run is signoff-clean and not yet measured; a block whose run missed
/// is named and passed over — the runner ran it again at a clock it closes at, and that block is measured instead.
#[test]
fn every_clean_power_block_is_measured_and_a_missed_one_is_passed_over() {
    let out = drive(
        r#"
import json, os, sys, tempfile
d = tempfile.mkdtemp()
led = os.path.join(d, "power.json")
json.dump({"blocks": {"qorpwr_a_p10": {"signoff_clean": True}, "qorpwr_a_p8_5": {"signoff_clean": False},
                      "qorpwr_b_p10": {"signoff_clean": True}, "qorpwr_c_p10": {"signoff_clean": True}}}, open(led, "w"))
rqp.qorlib.ROOT = d
rqp.POWER_LEDGER = "power.json"
rqp.qorlib.load = lambda kind, default: {"rows": [{"block": "qorpwr_c_p10"}]}
measured = []
rqp.measure = lambda b: measured.append(b)
sys.argv = ["record-qor-power.py", "all"]
rqp.main()
print(measured)
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(*lines.last().unwrap(), "['qorpwr_a_p10', 'qorpwr_b_p10']", "the clean blocks not yet measured");
    assert!(
        lines.iter().any(|l| l.contains("qorpwr_a_p8_5") && l.contains("not signoff-clean")),
        "the missed block is named: {out}"
    );
}
