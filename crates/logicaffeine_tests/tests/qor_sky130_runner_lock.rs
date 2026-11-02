//! qor_sky130_runner_lock — the comparative bench's Sky130 runner (`scripts/qor/run-qor-sky130.py`) searches for a
//! candidate's own minimum period as it says, and keeps controls out of it.
//!
//! The runner is driven here with its flows simulated: `run` records the period it was asked for and answers with
//! a recorded run of the slack and verdict the scenario sets, and the next period after a miss or a close comes from
//! the bench's rule (the miss, or the measured path, plus 5%, on the 0.5 ns grid). The tests hold:
//!
//! A. after a close with slack to spare the search tries the period the measured path needs, after a miss the period
//!    the slowest corner needs, and it stops when a close leaves no shorter period, when a run fails for a reason a
//!    period cannot fix, when a flow records nothing, when a period comes round again, or after its last attempt;
//! B. a control — a run on stock OpenLane — and a power block never count toward an own minimum or a kernel's iso
//!    period;
//! C. a power block runs at its kernel's iso period, and once more at the period its slowest corner needs only when
//!    it misses setup; a K7 design has one only when the traffic gate's testbench drives it;
//! D. a run the global router refuses for congestion is repeated in the bench's next floorplan, and a flow that records
//!    nothing is refused, its block's record never an older run's.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `body` (Python) with the runner imported as `m` and return what it prints. In `body`, `scripted(answers)`
/// replaces the runner's flows: each run takes the next answer — `(clean, setup slack)`, or `None` for a flow that
/// records nothing — and `tried` lists the periods asked for; `own` holds the runner's own `run` and `ledger_row`.
fn drive(body: &str) -> String {
    let prelude = r#"
import importlib.util, json, math, sys
spec = importlib.util.spec_from_file_location("runner", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
own = {"run": m.run, "ledger_row": m.ledger_row}
records, tried = {}, []

def honest(period, slack):
    if math.copysign(1.0, slack) > 0:
        return period
    return math.ceil((period - slack) * 1.05 * 2 - 1e-9) / 2

def scripted(answers):
    queue = list(answers)
    def run(kernel, cand, period, deadline, role="attempt"):
        tried.append(period)
        block = m.block_of(cand, period, role)
        answer = queue.pop(0) if queue else None
        if answer is not None:
            clean, slack = answer
            records[block] = {"signoff_clean": clean, "setup_worst_slack_ns": slack, "hold_worst_slack_ns": 0.1}
        return block
    m.run = run

m.honest = honest
m.ledger_row = lambda block, role="attempt": records.get(block)
m.qorlib.flow_settings = lambda: "5" * 64
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("{prelude}\n{body}"))
        .arg(root().join("scripts/qor/run-qor-sky130.py"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn search(answers: &str) -> String {
    drive(&format!("scripted({answers})\nm.search('K1', 'k1_logos', 10.0, 60)\nprint(tried)"))
}

#[test]
fn the_search_tries_what_each_run_says_it_needs_and_stops_when_nothing_shorter_is_left() {
    assert_eq!(search("[(True, 2.7), (True, 0.3)]"), "[10.0, 8.0]", "7.3 ns measured: 8; then 7.7 needs 8.5, not shorter");
    assert_eq!(search("[(False, -1.2), (True, 0.5)]"), "[10.0, 12.0]", "a miss by 1.2: 12; closed with 11.5 measured");
    assert_eq!(search("[(True, 0.0)]"), "[10.0]", "a close with no slack to spare");
    assert_eq!(search("[(False, 0.4)]"), "[10.0]", "timing met, signoff failed: no period fixes that");
    assert_eq!(search("[None]"), "[10.0]", "a flow that records nothing");
    assert_eq!(search("[(True, 5.0), (True, 2.0), (True, 1.0), (True, 0.8), (True, 0.5)]"), "[10.0, 5.5, 4.0, 3.5]", "four attempts at most");
    assert_eq!(search("[(False, -1.0), (False, -0.2)]"), "[10.0, 12.0, 13.0]", "misses walk the period up: 11.55 and 12.81 on the grid");
}

#[test]
fn a_control_or_a_power_block_never_counts_toward_an_own_minimum_or_an_iso_period() {
    let out = drive(
        r#"
rows = [
    {"kernel": "K1", "candidate": "a", "block": m.block_of("a", 8.0), "period_ns": 8.0, "role": "attempt"},
    {"kernel": "K1", "candidate": "a", "block": m.block_of("a", 6.0, "control"), "period_ns": 6.0, "role": "control"},
    {"kernel": "K1", "candidate": "a", "block": m.block_of("a", 5.0, "power"), "period_ns": 5.0, "role": "power"},
    {"kernel": "K1", "candidate": "b", "block": m.block_of("b", 9.0), "period_ns": 9.0, "role": "attempt"},
    {"kernel": "K1", "candidate": "b", "block": m.block_of("b", 12.0, "power"), "period_ns": 12.0, "role": "power"},
]
for r in rows:
    r["flow"] = m.flow_identity(r["role"])
    records[r["block"]] = {"signoff_clean": True, "setup_worst_slack_ns": 0.1, "hold_worst_slack_ns": 0.1}
m.qorlib.load = lambda kind, default: {"rows": rows}
print(json.dumps([sorted(p for p, _, _ in m.attempts()[("K1", "a")]), m.iso_period("K1"),
                  [p for p, _, _ in m.attempts("control")[("K1", "a")]], m.block_of("a", 6.0, "control"),
                  [p for p, _, _ in m.attempts("power")[("K1", "a")]], m.block_of("a", 5.0, "power")]))
"#,
    );
    assert_eq!(
        out,
        r#"[[8.0], 9.0, [6.0], "qorsyn_a_p6_stock", [5.0], "qorpwr_a_p5"]"#,
        "neither the control at 6 ns nor the power block at 5 ns is a's own minimum; b's power block at 12 is no iso period"
    );
}

/// The periods a power block of k1_logos is run at, its kernel's iso period 9 ns, with the scripted answers.
fn power(answers: &str) -> String {
    drive(&format!("scripted({answers})\nm.iso_period = lambda kernel: 9.0\nm.power('K1', 'k1_logos', None, 60)\nprint(tried)"))
}

#[test]
fn a_power_block_runs_at_the_iso_period_and_once_more_only_after_a_setup_miss() {
    assert_eq!(power("[(True, 0.4)]"), "[9.0]", "it closes at the iso period");
    assert_eq!(power("[(False, -1.2), (True, 0.5)]"), "[9.0, 11.0]", "a miss by 1.2 ns: (10.2 x 1.05) on the 0.5 ns grid");
    assert_eq!(power("[(False, -1.2), (False, -0.3)]"), "[9.0, 11.0]", "two runs at most");
    assert_eq!(power("[(False, 0.2)]"), "[9.0]", "setup met, signoff failed: no period fixes that");
    assert_eq!(power("[None]"), "[9.0]", "a flow that records nothing");
    assert_eq!(
        drive("scripted([])\nprint(m.power('K1', 'k1_logos', 7.5, 60) or tried)"),
        "[7.5]",
        "a period given is the one run"
    );
}

/// A K7 design is driven at gate level only by the traffic gate's testbench, so one it does not drive — the planned
/// command player, which takes a plan, not requests — has no power block; one it drives runs as any other.
#[test]
fn a_k7_power_block_needs_the_traffic_testbench() {
    let out = drive(
        "m.traffic_driven = lambda: ['k7_logos_ddr2_ctrl', 'k7_litedram_ctrl']\n\
         try:\n    m.power('K7', 'k7_logos_dram_executor', 10.0, 60)\nexcept SystemExit as e:\n    print(e)",
    );
    assert_eq!(
        out,
        "K7 k7_logos_dram_executor: no traffic testbench drives it (the K7 traffic gate drives k7_logos_ddr2_ctrl, k7_litedram_ctrl)"
    );
    let driven = drive(
        "scripted([(True, 0.3)])\nm.traffic_driven = lambda: ['k7_logos_ddr2_ctrl']\nm.power('K7', 'k7_logos_ddr2_ctrl', 10.0, 60)\nprint(tried)",
    );
    assert_eq!(driven, "[10.0]", "a driven controller's power block runs at the period given");
}

/// A block is staged with the clock of the top OpenLane hardens — the fold wrapper's or the harness's `clk` — never the
/// clock port of the candidate inside it (Bambu's is `clock`), which is no port of the block.
#[test]
fn a_block_is_staged_with_its_own_tops_clock() {
    let out = drive(
        r#"
import os, tempfile
stage_root = tempfile.mkdtemp()
m.STAGE_ROOT = stage_root
m.qorlib.update = lambda kind, header, row: None
def emit(run_dir, kernel, candidate, mode, ops, module=None):
    open(os.path.join(run_dir, "decide.v"), "w").write("module decide(input clock, input reset); endmodule\n")
    if module:
        open(os.path.join(run_dir, module + ".v"), "w").write("module %s(input clk, input rst, output [15:0] sig); endmodule\n" % module)
    return {"candidate": {"name": candidate, "top": "decide", "clock": "clock"}, "source_kind": "third-party",
            "files": ["decide.v"]}
m.qorlib.emit = emit
m.qorlib.wrap_candidate = lambda run_dir: {}
def fold(cmd, capture_output=True, text=True):
    open(cmd[3], "w").write("module qorsyn_%s(input clk, input rst, output [15:0] sig); endmodule\n" % cmd[4])
    return type("R", (), {"returncode": 0, "stderr": ""})()
m.subprocess.run = fold
print(json.dumps([m.stage("K1", "k1_bambu_fsm_c_a7p10", 10.0, role)[1]["clock_port"] for role in ("attempt", "control", "power")]))
os.path.exists(os.path.join(stage_root, "qorsyn_k1_bambu_fsm_c_a7p10_p10"))
"#,
    );
    assert_eq!(out, r#"["clk", "clk", "clk"]"#, "the wrapper's clock and the harness's, not the candidate's");
}

#[test]
fn a_top_without_the_clock_is_refused_before_a_flow_runs() {
    let out = drive(
        r#"
import os, tempfile
d = tempfile.mkdtemp()
p = os.path.join(d, "qorsyn_x_p10.v")
open(p, "w").write("module qorsyn_x_p10(input clock, input rst, output [15:0] sig); endmodule\n")
try:
    m.top_clock(p, "qorsyn_x_p10")
except SystemExit as e:
    print(e)
"#,
    );
    assert_eq!(out, "qorsyn_x_p10: its top declares no `input clk` (ports: input clock, input rst, output [15:0] sig)");
}

/// Every flow the bench runs — an attempt, a control, a power block — is a comparative one: a miss is recorded, never
/// a run abandoned, and the recorder reads what the synthesis checks found.
#[test]
fn every_flow_the_bench_runs_is_comparative() {
    let out = drive(
        r#"
envs = []
class P:
    def __init__(self, args, cwd=None, env=None, start_new_session=False):
        envs.append(env)
    def wait(self, timeout=None):
        return 0
m.subprocess.Popen = P
m.room_for_a_flow = lambda: None
for role in ("attempt", "control", "power"):
    m.flow("b", {"role": role, "files": [], "clock_port": "clk", "period_ns": 10.0}, 60)
print(json.dumps([(e.get("COMPARATIVE"), e.get("OL_STOCK"), e.get("OL_BENCH")) for e in envs]))
print(json.dumps(sorted({e.get("OL_MIN_FREE_GB") for e in envs} | {"%d" % m.FLOW_FREE_GB})))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        r#"[["1", "0", "1"], ["1", "1", "0"], ["1", "0", "1"]]"#,
        "an attempt and a power block flow in the bench's own tree, a control in the stock one"
    );
    assert_eq!(lines[1].matches('"').count(), 2, "every flow's own free-memory check is the floor its wait used: {}", lines[1]);
}

/// A run's flow is OpenLane's commit, the image it runs, the overlay, the flow settings the compiler emits for a bench
/// block and the IR analysis its verdict gates on: an attempt recorded under other settings, in another image — the
/// released one, or before a run named its image — or under another IR analysis, or before a run named one, is a
/// different flow, so the search runs it again rather than reading it.
#[test]
fn an_attempt_recorded_under_other_flow_knowledge_is_run_again() {
    let out = drive(
        r#"
flow = m.flow_identity("attempt")
released = "ghcr.io/the-openroad-project/openlane:" + flow["openlane"]
for stale in (dict(flow, flow_settings_sha3="0" * 64), dict(flow, openroad_image=released),
              {k: v for k, v in flow.items() if k != "openroad_image"}, dict(flow, ir_analysis_sha3="0" * 64),
              {k: v for k, v in flow.items() if k != "ir_analysis_sha3"}):
    rows = [{"kernel": "K1", "candidate": "k1_logos", "block": m.block_of("k1_logos", 10.0), "period_ns": 10.0, "role": "attempt", "flow": stale}]
    records[m.block_of("k1_logos", 10.0)] = {"signoff_clean": True, "setup_worst_slack_ns": 0.0, "hold_worst_slack_ns": 0.1}
    m.qorlib.load = lambda kind, default, rows=rows: {"rows": rows}
    tried.clear()
    scripted([(True, 0.0)])
    m.search("K1", "k1_logos", 10.0, 60)
    print(json.dumps(tried))
print(json.dumps([sorted(flow), len(flow["flow_settings_sha3"]), flow["openroad_image"] == m.qorlib.openroad_image()]))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..5],
        ["[10.0]", "[10.0]", "[10.0]", "[10.0]", "[10.0]"],
        "the stale attempt at 10 ns is run again, whatever made it stale"
    );
    assert_eq!(
        lines[5],
        r#"[["flow_settings_sha3", "ir_analysis_sha3", "openlane", "openroad_image", "overlay_sha3"], 64, true]"#,
        "a run's flow names the bench's image and the IR analysis"
    );
}

/// The run of `k1_x` at 10 ns, each of its flows answering in turn — `recorded` (a miss), `antenna` (setup and hold met,
/// signoff failed on antenna pins every repair left), `antenna-miss` (the same pins, setup missed), `congested` (the
/// global router's GRT-0119), `congested-again` (its GRT-0232: the same overflow left after the incremental re-route
/// that repairs run), `failed` (another error) or `timeout` — its ledger holding an older record of the block
/// when `stale`: the core utilizations the flows ran at, the floorplan the stage row names, each floorplan left before
/// it with why, the refusal, and whether the block has a record.
fn ladder(outcomes: &str, stale: bool) -> String {
    let stale = if stale { "True" } else { "False" };
    let out = drive(&format!(
        r#"
import os, tempfile
work = tempfile.mkdtemp()
stage_rows = []
def update(kind, header, row):
    stage_rows[:] = [r for r in stage_rows if r["block"] != row["block"]] + [row]
m.qorlib.update = update
m.qorlib.load = lambda kind, default: {{"rows": stage_rows}}
m.run, m.ledger_row = own["run"], own["ledger_row"]
ledger = os.path.join(work, "ledger.json")
m.ROLES = dict(m.ROLES, attempt=(m.ROLES["attempt"][0], ledger))
m.run_root = lambda role: os.path.join(work, "designs")
block = m.block_of("k1_x", 10.0)
if {stale}:
    json.dump({{"blocks": {{block: {{"signoff_clean": True, "setup_worst_slack_ns": 3.0, "hold_worst_slack_ns": 0.1}}}}}}, open(ledger, "w"))
def stage(kernel, cand, period, role="attempt"):
    row = {{"key": block, "block": block, "kernel": kernel, "candidate": cand, "period_ns": period, "role": role,
           "flow": m.flow_identity(role)}}
    update("sky130_stage", None, row)
    return block, row
m.stage = stage
clean = {{"drc_violations": 0, "lvs_clean": True, "routing_violations": 0, "xor_differences": 0,
         "max_slew_violations": 0, "max_cap_violations": 0, "max_fanout_violations": 0, "std_cell_area_um2": 1.0}}
records = {{
    "recorded": dict(clean, signoff_clean=False, setup_worst_slack_ns=-1.0, hold_worst_slack_ns=0.1, antenna_violators=0),
    "antenna": dict(clean, signoff_clean=False, setup_worst_slack_ns=0.5, hold_worst_slack_ns=0.1, antenna_violators=2),
    "antenna-miss": dict(clean, signoff_clean=False, setup_worst_slack_ns=-0.5, hold_worst_slack_ns=0.1, antenna_violators=2),
    "antenna-edge": dict(clean, signoff_clean=False, setup_worst_slack_ns=0.0, hold_worst_slack_ns=0.0, antenna_violators=1),
}}
queue, used, extracts = {outcomes}, [], {{}}
m.RECORDER = type("Recorder", (), {{"extract": staticmethod(lambda run_dir, b: dict(extracts[run_dir]))}})
def flow(b, row, deadline):
    used.append(row["floorplan"]["fp_core_util"])
    what = queue.pop(0)
    if what == "timeout":
        return None, True
    run_dir = os.path.join(work, "designs", "logos_" + b, "runs", "RUN_%d" % len(used))
    logs = os.path.join(run_dir, "logs", "routing")
    os.makedirs(logs)
    if what in records:
        extracts[run_dir] = records[what]
        with open(ledger, "w") as f:
            json.dump({{"blocks": {{b: records[what]}}}}, f)
        return 1, False
    if what == "other":
        doc = json.load(open(ledger)) if os.path.exists(ledger) else {{"blocks": {{}}}}
        doc["blocks"]["qorsyn_k1_y_p10"] = records["recorded"]
        json.dump(doc, open(ledger, "w"))
    open(os.path.join(logs, "22-global.log"), "w").write(
        {{"congested": "[ERROR GRT-0119] Routing congestion too high.\n",
          "congested-again": "[ERROR GRT-0232] Routing congestion too high. Check the congestion heatmap in the GUI.\n"}}
        .get(what, "[ERROR GRT-0023] no pins.\n"))
    if what == "lint":
        os.makedirs(os.path.join(run_dir, "logs", "synthesis"))
        open(os.path.join(run_dir, "logs", "synthesis", "linter.log"), "w").write(
            "%Warning-WIDTH: x.v:1:1: width\n"
            "%Error: /openlane/designs/d/src/Vortex.v:9559:35: Constant function may not be declared under generate "
            "(IEEE 1800-2017 13.4.3)\n%Error: Exiting due to 4 error(s)\n")
    return 2, False
m.flow = flow
m.run("K1", "k1_x", 10.0, 60)
row = stage_rows[-1]
print(json.dumps([used, row["floorplan"], [[f["fp_core_util"], f["why"]] for f in row["loosened"]], row["refused"],
                  m.ledger_row(block) is not None]))
"#
    ));
    out.lines().last().unwrap_or_default().to_string()
}

#[test]
fn a_run_congestion_refuses_is_repeated_in_a_looser_floorplan() {
    assert_eq!(
        ladder(r#"["recorded"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], null, true]"#,
        "a run that records is the block's run, in the first floorplan"
    );
    assert_eq!(
        ladder(r#"["congested", "recorded"]"#, false),
        r#"[[35, 25], {"fp_core_util": 25, "pl_target_density": 0.35}, [[35, "congestion"]], null, true]"#,
        "congestion at 35%: the next floorplan, and the row says why"
    );
    let all = ladder(r#"["congested", "congested", "congested"]"#, false);
    assert!(
        all.starts_with(r#"[[35, 25, 15], {"fp_core_util": 15, "pl_target_density": 0.25}, [[35, "congestion"], [25, "congestion"]], "global routing congestion (GRT-0119) in every floorplan"#)
            && all.ends_with("false]"),
        "the last floorplan congested too: refused, naming the router's verdict\n{all}"
    );
    assert_eq!(
        ladder(r#"["congested-again", "recorded"]"#, false),
        r#"[[35, 25], {"fp_core_util": 25, "pl_target_density": 0.35}, [[35, "congestion"]], null, true]"#,
        "the overflow the incremental re-route leaves is congestion too: the next floorplan"
    );
    let again = ladder(r#"["congested", "congested-again", "congested-again"]"#, false);
    assert!(
        again.contains(r#""global routing congestion (GRT-0232) in every floorplan"#),
        "the refusal names the router's verdict on the last floorplan\n{again}"
    );
    assert_eq!(
        ladder(r#"["lint"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane's lint step (Verilator) refused the RTL: Vortex.v:9559:35: Constant function may not be declared under generate (IEEE 1800-2017 13.4.3)", false]"#,
        "a lint refusal names Verilator's first error, and no floorplan fixes it"
    );
    assert_eq!(
        ladder(r#"["failed"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane exited 2 without a recorded run", false]"#,
        "a failure a floorplan cannot fix is not repeated"
    );
    assert_eq!(
        ladder(r#"["timeout"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane did not finish within 60 s", false]"#,
        "nor is a run past its deadline"
    );
}

/// A run that meets setup and hold at every corner and fails signoff only on antenna pins every repair left — no
/// diode site with a clear tie beside them, no jog — is repeated in the next floorplan, where the repair has room; one
/// that missed setup is not (its period moves instead), and the last floorplan's run is the block's run, its pins
/// counted against it.
#[test]
fn a_run_whose_antenna_repair_found_no_room_is_repeated_in_a_looser_floorplan() {
    assert_eq!(
        ladder(r#"["antenna", "recorded"]"#, false),
        r#"[[35, 25], {"fp_core_util": 25, "pl_target_density": 0.35}, [[35, "antenna"]], null, true]"#
    );
    assert_eq!(
        ladder(r#"["congested", "antenna", "antenna"]"#, false),
        r#"[[35, 25, 15], {"fp_core_util": 15, "pl_target_density": 0.25}, [[35, "congestion"], [25, "antenna"]], null, true]"#,
        "each floorplan left says why; the last one's run stands"
    );
    assert_eq!(
        ladder(r#"["antenna-edge", "recorded"]"#, false),
        r#"[[35, 25], {"fp_core_util": 25, "pl_target_density": 0.35}, [[35, "antenna"]], null, true]"#,
        "timing met with no slack to spare is timing met"
    );
    assert_eq!(
        ladder(r#"["antenna-miss"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], null, true]"#,
        "a run that missed setup keeps its floorplan: the search moves its period"
    );
}

/// A flow that records nothing is refused even when the ledger holds an older run of its block (another period's
/// flow knowledge, a run before the last), and the block's record is then none: no search, iso period or summary
/// reads the older run as this one. Nor does another flow's record count: a flow running beside it may rewrite the
/// ledger while it runs, so a run is recorded only when the ledger holds what the recorder extracts from that run.
#[test]
fn a_run_that_records_nothing_never_reads_an_older_record() {
    assert_eq!(
        ladder(r#"["failed"]"#, true),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane exited 2 without a recorded run", false]"#
    );
    assert_eq!(
        ladder(r#"["other"]"#, true),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane exited 2 without a recorded run", false]"#,
        "the ledger rewritten by a flow beside it while this one failed"
    );
    assert_eq!(
        ladder(r#"["other"]"#, false),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], "OpenLane exited 2 without a recorded run", false]"#,
        "and with no record of its own before"
    );
    assert_eq!(
        ladder(r#"["recorded"]"#, true),
        r#"[[35], {"fp_core_util": 35, "pl_target_density": null}, [], null, true]"#,
        "a run that records replaces the older one"
    );
}

/// A block only staged (`run-qor-sky130.py stage`) has run no flow yet, so a run its ledger holds from an earlier
/// staging is not its record; once its flow records, that run is.
#[test]
fn a_block_only_staged_has_no_record_yet() {
    let out = drive(
        r#"
import os, tempfile
work = tempfile.mkdtemp()
m.ledger_row = own["ledger_row"]
ledger = os.path.join(work, "ledger.json")
m.ROLES = dict(m.ROLES, attempt=(m.ROLES["attempt"][0], ledger))
block = m.block_of("k1_x", 10.0)
json.dump({"blocks": {block: {"signoff_clean": True}}}, open(ledger, "w"))
row = {"block": block, "role": "attempt"}
m.qorlib.load = lambda kind, default: {"rows": [row]}
staged = m.ledger_row(block)
row["flow_rc"] = 0
print(json.dumps([staged, m.ledger_row(block)]))
"#,
    );
    assert_eq!(out, r#"[null, {"signoff_clean": true}]"#);
}

/// A flow starts when the box has room for it: no other OpenLane flow or mpw_precheck run on the box but those whose
/// design the bench was told it may run beside (`BENCH_COEXIST`, globs over the flow's design), and the memory a bench flow needs is free. It
/// waits otherwise — so a search never loses its next attempt to a flow that came and went between two of its own —
/// and a wait that outlasts its limit refuses the flow, naming what held it.
#[test]
fn a_flow_waits_for_room_on_the_box() {
    let out = drive(
        r#"
import os
own_flows = m.box_flows
wpe = ("ghcr.io/the-openroad-project/openlane:x-amd64 heavy", "logos_wpe_up")
pipe = ("ghcr.io/the-openroad-project/openlane:x-amd64 calm", "logos_systolic_pipe16_realbist")
def scenario(flows, free_gb, coexist="", limit=None):
    polls, slept = [], []
    def openlane_flows():
        polls.append(1)
        return flows[min(len(polls), len(flows)) - 1]
    m.box_flows = openlane_flows
    m.mem_available_gb = lambda: free_gb[min(len(polls), len(free_gb)) - 1]
    m.disk_free_gb = lambda: m.FLOW_FREE_DISK_GB + 100
    m.time.sleep = lambda s: slept.append(s)
    os.environ["BENCH_COEXIST"] = coexist
    if limit is not None:
        m.FLOW_WAIT_S = limit
    try:
        m.room_for_a_flow()
        verdict = "starts"
    except SystemExit as e:
        verdict = str(e)
    return "%s after %d polls, %d s" % (verdict, len(polls), sum(slept))
plenty = [m.FLOW_FREE_GB + 50]
print(scenario([[]], plenty))
print(scenario([[wpe], [wpe], []], plenty))
print(scenario([[pipe]], plenty, coexist="logos_systolic_*"))
print(scenario([[pipe, wpe], [pipe]], plenty, coexist="logos_systolic_*"))
print(scenario([[pipe]], [m.FLOW_FREE_GB - 1, m.FLOW_FREE_GB - 1, m.FLOW_FREE_GB], coexist="logos_systolic_*"))
print(scenario([[pipe]], plenty, limit=2 * m.FLOW_POLL_S))
print(scenario([[wpe]], plenty, coexist="logos_systolic_*", limit=3 * m.FLOW_POLL_S))
print(scenario([[]], [m.FLOW_FREE_GB - 1], limit=m.FLOW_POLL_S))
class R:
    def __init__(self, out): self.stdout = out
def docker(cmd, capture_output=True, text=True):
    if cmd[1] == "ps":
        return R("a1 ghcr.io/the-openroad-project/openlane:x-amd64 calm\nb2 efabless/mpw_precheck:latest check\n"
                 "c3 logos-plasticine-toolchain twin\n")
    return R('["-c","./flow.tcl -design logos_systolic_pipe16_realbist"]' if cmd[-1] == "a1" else '["/bin/bash"]')
m.subprocess.run = docker
print(own_flows())
"#,
    );
    let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with("waiting for")).collect();
    assert_eq!(lines[0], "starts after 1 polls, 0 s", "an idle box: it starts at once");
    assert_eq!(lines[1], "starts after 3 polls, 120 s", "it waits out another session's flow");
    assert_eq!(lines[2], "starts after 1 polls, 0 s", "beside a flow it was told it may run beside");
    assert_eq!(lines[3], "starts after 2 polls, 60 s", "but never beside one it was not");
    assert_eq!(lines[4], "starts after 3 polls, 120 s", "and only once the memory a bench flow needs is free");
    assert_eq!(
        lines[5],
        "another OpenLane flow is still running after 120 s (ghcr.io/the-openroad-project/openlane:x-amd64 calm, \
         design logos_systolic_pipe16_realbist): no room for a flow after 3 polls, 120 s",
        "nothing runs beside another flow unless the bench was told it may"
    );
    assert_eq!(
        lines[6],
        "another OpenLane flow is still running after 180 s (ghcr.io/the-openroad-project/openlane:x-amd64 heavy, \
         design logos_wpe_up): no room for a flow after 4 polls, 180 s"
    );
    let floor = lines[7];
    assert!(floor.starts_with("only ") && floor.contains(" GB free after 60 s") && floor.ends_with("no room for a flow after 2 polls, 60 s"), "{floor}");
    assert_eq!(
        lines[8],
        "[('ghcr.io/the-openroad-project/openlane:x-amd64 calm', 'logos_systolic_pipe16_realbist'), \
         ('efabless/mpw_precheck:latest check', 'unknown')]",
        "an OpenLane flow by the design it runs, an mpw_precheck run too; no other container"
    );
    assert!(out.contains("waiting for room: ghcr.io/the-openroad-project/openlane:x-amd64 heavy, design logos_wpe_up"), "{out}");
}

/// `run-qor-sky130.py room` is the same wait, for a caller that must not start a flow before the box has room; with
/// `--within S` it gives up after S seconds, for a caller holding the suite lock that would rather hand it back than
/// hold it through another session's long flow.
#[test]
fn the_room_mode_waits_as_a_flow_does() {
    let out = drive(
        r#"
import sys
m.room_for_a_flow = lambda: print("waited %d" % m.FLOW_WAIT_S)
whole = m.FLOW_WAIT_S
sys.argv = ["run-qor-sky130.py", "room"]
m.main()
sys.argv = ["run-qor-sky130.py", "room", "--within", "600"]
m.main()
print(whole)
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], format!("waited {}", lines[2]), "the whole wait a flow gets");
    assert_eq!(lines[1], "waited 600", "or the caller's bound");
}

/// A flow's peak memory is its OpenLane container's (the cgroup's memory.peak, page cache included), sampled while it
/// runs and kept on its stage row; a flow past its deadline is killed — its process group, then the container that
/// runs exactly its design.
#[test]
fn a_flow_keeps_its_peak_memory_and_a_late_one_is_killed_by_its_design() {
    let out = drive(
        r#"
own_container = m.flow_container
def scenario(waits, peaks, clock):
    kills, docker, calls = [], [], []
    class P:
        pid, returncode = 4242, None
        def __init__(self, args, cwd=None, env=None, start_new_session=False):
            pass
        def wait(self, timeout=None):
            if timeout is None:
                self.returncode = -9
                return -9
            calls.append(timeout)
            if len(calls) > 1000:
                raise SystemExit("a flow that never ends never stops waiting")
            if waits:
                w = waits.pop(0)
                if w is not None:
                    self.returncode = w
                    return w
            raise m.subprocess.TimeoutExpired("flow", timeout)
    m.subprocess.Popen = P
    m.subprocess.run = lambda cmd, capture_output=True, text=True: docker.append(cmd)
    m.room_for_a_flow = lambda: None
    m.FLOW_SAMPLE_S = 0
    m.flow_container = lambda design: {"logos_b": "c1"}.get(design)
    m.container_peak_gb = lambda cid: peaks.pop(0) if peaks else None
    ticks = iter(clock)
    m.time.monotonic = lambda: next(ticks)
    m.os.killpg = lambda pid, sig: kills.append(int(sig))
    row = {"role": "attempt", "files": [], "clock_port": "clk", "period_ns": 10.0}
    rc, late = m.flow("b", row, 60)
    return [rc, late, row["flow_peak_mem_gb"], kills, docker]
print(json.dumps(scenario([None, None, None, 0], [1.5, 3.25, 2.0], range(0, 1000, 5))))
print(json.dumps(scenario([None, None], [0.75], [0, 30, 61, 90, 120])))
print(json.dumps(scenario([0], [], range(0, 100, 5))))
class R:
    def __init__(self, out): self.stdout = out
args = {"c5": '["-c","./flow.tcl -design logos_b_5"]', "c1": '["-c","./flow.tcl -design logos_b"]'}
m.subprocess.run = lambda cmd, capture_output=True, text=True: R("c5\nc1\n" if cmd[1] == "ps" else args[cmd[-1]])
print(own_container("logos_b"), own_container("logos_b_5"), own_container("logos_c"))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[3], "c1 c5 None", "the container that runs exactly the design, never one whose name begins with it");
    assert_eq!(lines[0], "[0, false, 3.25, [], []]", "the most the container held while it ran");
    assert_eq!(lines[1], "[-9, true, 0.75, [15, 9], [[\"docker\", \"kill\", \"c1\"]]]", "past its deadline: TERM, KILL, its container");
    assert_eq!(lines[2], "[0, false, null, [], []]", "a flow that ended before its container was read has no peak");
}

/// A flow runs in the floorplan its row names — openlane-run.sh's own first, its core utilization and the compiler's
/// placement density, then each later one with both lowered together, the density above the utilization it places.
#[test]
fn a_flow_runs_in_the_floorplan_its_row_names() {
    let out = drive(
        r#"
import os
envs = []
class P:
    def __init__(self, args, cwd=None, env=None, start_new_session=False):
        envs.append(env)
    def wait(self, timeout=None):
        return 0
m.subprocess.Popen = P
m.room_for_a_flow = lambda: None
os.environ["PL_TARGET_DENSITY"] = "0.9"
for fp in m.FLOORPLANS:
    m.flow("b", {"role": "attempt", "files": [], "clock_port": "clk", "period_ns": 10.0, "floorplan": fp}, 60)
print(json.dumps([(e.get("FP_CORE_UTIL"), e.get("PL_TARGET_DENSITY")) for e in envs]))
print(json.dumps(m.FLOORPLANS))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], r#"[["35", null], ["25", "0.35"], ["15", "0.25"]]"#, "no density from the caller's environment");
    let ladder: Vec<serde_json::Value> = serde_json::from_str(lines[1]).unwrap();
    let runner = std::fs::read_to_string(root().join("scripts/openlane-run.sh")).unwrap();
    let first = ladder[0]["fp_core_util"].as_u64().unwrap();
    assert!(runner.contains(&format!("FP_CORE_UTIL=\"${{FP_CORE_UTIL:-{first}}}\"")), "the first floorplan is openlane-run.sh's own");
    for w in ladder.windows(2) {
        assert!(w[1]["fp_core_util"].as_u64() < w[0]["fp_core_util"].as_u64(), "each floorplan looser than the last");
    }
    for fp in &ladder[1..] {
        let (u, d) = (fp["fp_core_util"].as_f64().unwrap(), fp["pl_target_density"].as_f64().unwrap());
        assert!(d * 100.0 > u && d < 0.45, "{fp}: a density below the compiler's 0.45 and above the utilization");
    }
}

/// A run's record is its own: the ledger's record of the block equals what the recorder extracts from the run the flow
/// created. No new run, no ledger, a record of an older run, or a run the recorder cannot read: no record.
#[test]
fn a_run_is_recorded_only_when_the_ledger_holds_what_its_own_run_records() {
    let out = drive(
        r#"
import os, tempfile
work = tempfile.mkdtemp()
ledger = os.path.join(work, "ledger.json")
m.ROLES = dict(m.ROLES, attempt=(m.ROLES["attempt"][0], ledger))
m.run_root = lambda role: os.path.join(work, "designs")
block = m.block_of("k1_x", 10.0)
new_run = os.path.join(work, "designs", "logos_" + block, "runs", "RUN_2")
def extract(run_dir, b):
    if run_dir != new_run or b != block:
        raise FileNotFoundError(run_dir)
    return {"signoff_clean": True, "setup_worst_slack_ns": 0.2}
m.RECORDER = type("Recorder", (), {"extract": staticmethod(extract)})
cases = []
cases.append(m.recorded(block, "attempt", []))
cases.append(m.recorded(block, "attempt", ["RUN_2"]))
json.dump({"blocks": {block: {"signoff_clean": True, "setup_worst_slack_ns": 0.2}}}, open(ledger, "w"))
cases.append(m.recorded(block, "attempt", ["RUN_2"]))
cases.append(m.recorded(block, "attempt", ["RUN_1"]))
cases.append(m.recorded(block, "attempt", ["RUN_1", "RUN_2"]))
json.dump({"blocks": {block: {"signoff_clean": False, "setup_worst_slack_ns": -1.0}}}, open(ledger, "w"))
cases.append(m.recorded(block, "attempt", ["RUN_2"]))
print(json.dumps(cases))
"#,
    );
    assert_eq!(
        out,
        "[false, false, true, false, true, false]",
        "no run; no ledger; its own record; an unreadable run; the newest of its runs; an older record"
    );
}

/// The bench may run several of its own flows at once (`BENCH_FLOWS`, one when unset), each when the memory a bench
/// flow needs is free; its own flows are the designs it stages (`logos_qorsyn_*`, `logos_qorpwr_*`), and no other
/// session's flow counts as one of them.
#[test]
fn the_bench_runs_its_own_flows_side_by_side_up_to_its_limit() {
    let out = drive(
        r#"
import os
bench = ("ghcr.io/x calm", "logos_qorsyn_k1_a_p10")
power = ("ghcr.io/x calm", "logos_qorpwr_k1_a_p10")
other = ("ghcr.io/x heavy", "logos_wpe_up")
def scenario(flows, n, free=None):
    polls, slept = [], []
    def openlane_flows():
        polls.append(1)
        return flows[min(len(polls), len(flows)) - 1]
    m.box_flows = openlane_flows
    m.mem_available_gb = lambda: free if free is not None else m.FLOW_FREE_GB + 50
    m.disk_free_gb = lambda: m.FLOW_FREE_DISK_GB + 100
    m.time.sleep = lambda s: slept.append(s)
    os.environ["BENCH_COEXIST"] = ""
    if n is None:
        os.environ.pop("BENCH_FLOWS", None)
    else:
        os.environ["BENCH_FLOWS"] = str(n)
    m.FLOW_WAIT_S = 3 * m.FLOW_POLL_S
    try:
        m.room_for_a_flow()
        return "starts after %d polls" % len(polls)
    except SystemExit:
        return "refused after %d polls" % len(polls)
print(scenario([[bench]], None))
print(scenario([[bench, power]], 3))
print(scenario([[bench, bench, power], [bench, power]], 3))
print(scenario([[bench, other]], 3))
print(scenario([[bench]], 3, free=m.FLOW_FREE_GB - 1))
"#,
    );
    let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with("waiting for")).collect();
    assert_eq!(lines[0], "refused after 4 polls", "one at a time unless told otherwise");
    assert_eq!(lines[1], "starts after 1 polls", "beside two of its own when three may run");
    assert_eq!(lines[2], "starts after 2 polls", "the third of three waits for one to finish");
    assert_eq!(lines[3], "refused after 4 polls", "never beside another session's flow");
    assert_eq!(lines[4], "refused after 4 polls", "and only with the memory a flow needs");
}

/// `run-qor-sky130.py kernel` runs a kernel whole: every candidate that passed its functional gate searched for its
/// own minimum, `--jobs` at a time; then each candidate not yet run at the kernel's iso period, at it; then the controls
/// named, at the iso period on stock OpenLane. Each step waits for the last, and one candidate's refusal stops no
/// other.
#[test]
fn a_kernel_runs_its_searches_its_iso_period_and_its_controls_jobs_at_a_time() {
    let out = drive(
        r#"
import sys, threading, time as _t
lock, events, running, peak = threading.Lock(), [], [0], [0]
def tracked(tag):
    def go(*a):
        with lock:
            running[0] += 1
            peak[0] = max(peak[0], running[0])
            events.append((tag, a[1]))
        _t.sleep(0.05)
        with lock:
            running[0] -= 1
        if a[1] == "c":
            sys.exit("c: refused")
    return go
m.gated_candidates = lambda k: ["a", "b", "c", "d", "e"]
m.search = tracked("search")
m.iso_period = lambda k: 12.5
m.attempts = lambda role="attempt": {("K1", c): [(12.5 if c == "a" else 10.0, None, None)] for c in "abcde"}
def run(kernel, cand, period, deadline, role="attempt"):
    tracked(role)(kernel, cand, period)
m.run = run
m.kernel("K1", 2, 10.0, 60, ["a", "b"])
order = [t for t, _ in events]
print(peak[0], order.index("attempt") > max(i for i, t in enumerate(order) if t == "search"),
      order.index("control") > max(i for i, t in enumerate(order) if t == "attempt"))
print(sorted(c for t, c in events if t == "search"), sorted(c for t, c in events if t == "attempt"),
      sorted(c for t, c in events if t == "control"))
"#,
    );
    let lines: Vec<&str> = out.lines().filter(|l| !l.contains("refused")).collect();
    assert_eq!(lines[0], "2 True True", "two at a time; the iso runs after every search, the controls after them");
    assert_eq!(
        lines[1],
        "['a', 'b', 'c', 'd', 'e'] ['b', 'c', 'd', 'e'] ['a', 'b']",
        "every gated candidate searched, each not yet at the iso period run at it, the named controls"
    );
}

/// Flows started side by side run side by side: a start holds the next one back only until its own container is up,
/// then both run to their ends at once.
#[test]
fn flows_started_together_overlap_once_each_container_is_up() {
    let out = drive(
        r#"
import threading, time as _t
spans, lock = {}, threading.Lock()
class P:
    pid, returncode = 4242, None
    def __init__(self, args, cwd=None, env=None, start_new_session=False):
        self.block, self.left = args[2], 6
        with lock:
            spans[self.block] = [_t.monotonic(), None]
    def wait(self, timeout=None):
        _t.sleep(0.05)
        self.left -= 1
        if self.left > 0:
            raise m.subprocess.TimeoutExpired("flow", timeout)
        with lock:
            spans[self.block][1] = _t.monotonic()
        self.returncode = 0
        return 0
m.subprocess.Popen = P
m.room_for_a_flow = lambda: None
m.FLOW_SAMPLE_S = 0
m.flow_container = lambda design: "c-" + design
m.container_peak_gb = lambda cid: 1.0
row = lambda: {"role": "attempt", "files": [], "clock_port": "clk", "period_ns": 10.0}
ts = [threading.Thread(target=m.flow, args=(b, row(), 600), daemon=True) for b in ("b1", "b2")]
[t.start() for t in ts]
[t.join(timeout=20) for t in ts]
if any(t.is_alive() for t in ts) or any(v[1] is None for v in spans.values()) or len(spans) < 2:
    print("hung", m.STARTING.locked())
else:
    (a0, a1), (b0, b1) = spans["b1"], spans["b2"]
    print(max(a0, b0) < min(a1, b1), m.STARTING.locked())
"#,
    );
    assert_eq!(out, "True False", "the two flows overlap, and no start is left holding the next one back");
}

/// A flow starts only with FLOW_FREE_DISK_GB free on the filesystem the bench's trees are on: a run that starts into a
/// full disk dies mid-route and takes the flows beside it down with it, so the flow waits for room as it waits for
/// memory, and a wait past its limit refuses it, naming the disk.
#[test]
fn a_flow_waits_for_room_on_the_disk() {
    let out = drive(
        r#"
import os
def scenario(disk):
    polls, slept = [], []
    def flows():
        polls.append(1)
        return []
    m.box_flows = flows
    m.mem_available_gb = lambda: m.FLOW_FREE_GB + 50
    m.disk_free_gb = lambda: disk[min(len(polls), len(disk)) - 1]
    m.time.sleep = lambda s: slept.append(s)
    os.environ["BENCH_COEXIST"] = ""
    os.environ.pop("BENCH_FLOWS", None)
    m.FLOW_WAIT_S = 3 * m.FLOW_POLL_S
    try:
        m.room_for_a_flow()
        return "starts after %d polls" % len(polls)
    except SystemExit as e:
        return "refused after %d polls: %s" % (len(polls), str(e).split(":")[0])
low = m.FLOW_FREE_DISK_GB - 1
print(scenario([m.FLOW_FREE_DISK_GB + 10]))
print(scenario([low, low, m.FLOW_FREE_DISK_GB]))
print(scenario([low]))
print(m.disk_free_gb.__name__ if hasattr(m.disk_free_gb, "__name__") else "")
"#,
    );
    let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with("waiting for")).collect();
    assert_eq!(lines[0], "starts after 1 polls", "room on the disk: it starts");
    assert_eq!(lines[1], "starts after 3 polls", "it waits for the disk to free");
    assert!(lines[2].starts_with("refused after 4 polls: only ") && lines[2].contains("GB free on the disk"), "{}", lines[2]);
}

/// A run its flow did not record keeps its logs and reports — the evidence of why it failed — and drops its database
/// and intermediates (`results/`, `tmp/`), the bulk of a run; a recorded run keeps everything the verifier re-derives
/// it from.
#[test]
fn a_run_not_recorded_keeps_its_logs_and_drops_its_bulk() {
    let out = drive(
        r#"
import os, tempfile
work = tempfile.mkdtemp()
stage_rows = []
def update(kind, header, row):
    stage_rows[:] = [r for r in stage_rows if r["block"] != row["block"]] + [row]
m.qorlib.update = update
m.qorlib.load = lambda kind, default: {"rows": stage_rows}
m.run, m.ledger_row = own["run"], own["ledger_row"]
ledger = os.path.join(work, "ledger.json")
m.ROLES = dict(m.ROLES, attempt=(m.ROLES["attempt"][0], ledger))
m.run_root = lambda role: os.path.join(work, "designs")
block = m.block_of("k1_x", 10.0)
def stage(kernel, cand, period, role="attempt"):
    row = {"key": block, "block": block, "kernel": kernel, "candidate": cand, "period_ns": period, "role": role,
           "flow": m.flow_identity(role)}
    update("sky130_stage", None, row)
    return block, row
m.stage = stage
rec = {"signoff_clean": False, "setup_worst_slack_ns": -1.0, "hold_worst_slack_ns": 0.1, "antenna_violators": 0,
       "drc_violations": 0, "lvs_clean": True, "routing_violations": 0, "xor_differences": 0, "max_slew_violations": 0,
       "max_cap_violations": 0, "max_fanout_violations": 0, "std_cell_area_um2": 1.0}
extracts, outcome = {}, []
m.RECORDER = type("Recorder", (), {"extract": staticmethod(lambda run_dir, b: dict(extracts[run_dir]))})
def flow(b, row, deadline):
    run_dir = os.path.join(work, "designs", "logos_" + b, "runs", "RUN_%d" % len(outcome))
    for d in ("logs/routing", "reports", "results/final", "tmp/routing"):
        os.makedirs(os.path.join(run_dir, d))
    open(os.path.join(run_dir, "logs", "routing", "22-global.log"), "w").write("[ERROR GRT-0023] no pins.\n")
    if outcome[-1] == "recorded":
        extracts[run_dir] = rec
        json.dump({"blocks": {b: rec}}, open(ledger, "w"))
        return 1, False
    return 2, False
m.flow = flow
for what in ("failed", "recorded"):
    outcome.append(what)
    m.run("K1", "k1_x", 10.0, 60)
    run_dir = os.path.join(work, "designs", "logos_" + block, "runs", "RUN_%d" % len(outcome))
    print(what, sorted(os.listdir(run_dir)))
"#,
    );
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("failed") || l.starts_with("recorded")).collect();
    assert_eq!(lines[0], "failed ['logs', 'reports']", "a run not recorded keeps the evidence of why");
    assert_eq!(lines[1], "recorded ['logs', 'reports', 'results', 'tmp']", "a recorded run keeps everything");
}

/// Several kernels run whole at once (`kernels K3,K1 …`): each as `kernel` runs it, all of them started together, so a
/// long kernel's flows overlap the short ones' rather than queueing behind them, every flow through the one start
/// lock and the one BENCH_FLOWS limit of the process. Each kernel takes only the controls named for it — a candidate
/// belongs to the kernel its name begins with.
#[test]
fn several_kernels_run_whole_at_once_each_with_its_own_controls() {
    let out = drive(
        r#"
import threading, time as _t
lock, calls, running, peak = threading.Lock(), [], [0], [0]
def kernel(k, jobs, start, deadline, controls=(), starts=None):
    with lock:
        running[0] += 1
        peak[0] = max(peak[0], running[0])
        calls.append((k, jobs, start, sorted(controls)))
    _t.sleep(0.1)
    with lock:
        running[0] -= 1
m.kernel = kernel
m.kernels(["K3", "K6", "K1"], 8, 10.0, 60, ["k1_logos", "k3_logos_ws", "k1_bambu_fsm_c_a7p10", "k6_logos_seqfma"])
print(peak[0])
for c in sorted(calls):
    print(c)
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "3", "every kernel runs at once");
    assert_eq!(
        lines[1..],
        [
            "('K1', 8, 10.0, ['k1_bambu_fsm_c_a7p10', 'k1_logos'])",
            "('K3', 8, 10.0, ['k3_logos_ws'])",
            "('K6', 8, 10.0, ['k6_logos_seqfma'])",
        ],
        "each kernel whole, at the jobs and start given, with the controls named for it"
    );
}

/// A kernel's power blocks (`powers K`): every candidate that passed its gate (K7: each controller its traffic gate
/// drives), or only those named (`--candidates`), `--jobs` at a time; a name outside that set is refused before any
/// block runs.
#[test]
fn a_kernels_power_blocks_run_for_its_candidates_or_only_those_named() {
    let out = drive(
        r#"
import threading
ran, lock = [], threading.Lock()
def power(kernel, cand, period, deadline):
    with lock:
        ran.append((kernel, cand, period))
m.power = power
m.gated_candidates = lambda k: ["k1_a", "k1_b", "k1_c"]
m.traffic_driven = lambda: ["k7_x"]
m.powers("K1", None, 60, 2)
print(sorted(ran)); ran.clear()
m.powers("K1", 9.0, 60, 2, ["k1_c", "k1_a"])
print(sorted(ran)); ran.clear()
m.powers("K7", None, 60, 2)
print(sorted(ran)); ran.clear()
try:
    m.powers("K1", None, 60, 2, ["k1_a", "k1_z"])
    print("ran", ran)
except SystemExit as e:
    print("refused:", e, ran)
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[('K1', 'k1_a', None), ('K1', 'k1_b', None), ('K1', 'k1_c', None)]", "every gated candidate");
    assert_eq!(lines[1], "[('K1', 'k1_a', 9.0), ('K1', 'k1_c', 9.0)]", "only those named, at the period given");
    assert_eq!(lines[2], "[('K7', 'k7_x', None)]", "K7: the controllers its traffic gate drives");
    assert!(
        lines[3].starts_with("refused: ") && lines[3].contains("k1_z") && lines[3].ends_with(" []"),
        "a name outside the set runs nothing: {}",
        lines[3]
    );
}

/// A memory template whose init-file parameter defaults to a file its tool never emits (Bambu's `array_a.mem`; every
/// instance overrides it with its own `array_ref_*.mem`) is read by OpenLane's yosys at parse time, which refuses the
/// whole design for the missing default; the FPGA flows read with `-defer` and never parse it. The stage gives such a
/// default an empty stand-in, named for the parameter and file that ask for it; a default the design ships, an empty
/// default, and a file no `$readmem` reads get none. Verify holds a stand-in to be empty.
#[test]
fn an_init_file_a_template_defaults_to_and_no_tool_emits_gets_an_empty_stand_in() {
    let out = drive(
        r#"
import os, tempfile
d = tempfile.mkdtemp()
open(os.path.join(d, "keccak.v"), "w").write("""
module ARRAY_1D #(parameter MEMORY_INIT_file="array_a.mem", n_elements=4) (input clk);
  reg [7:0] memory [0:n_elements-1];
  initial if (MEMORY_INIT_file != "") $readmemb(MEMORY_INIT_file, memory, 0, n_elements-1);
endmodule
module SHIPPED #(parameter INIT="array_ref_1.mem") (input clk);
  reg [7:0] m [0:3];
  initial $readmemh(INIT, m);
endmodule
module NONE #(parameter INIT="") (input clk);
  reg [7:0] m [0:3];
  initial if (INIT != "") $readmemb(INIT, m);
endmodule
module NOTREAD #(parameter LABEL="label.mem") (input clk);
endmodule
""")
open(os.path.join(d, "array_ref_1.mem"), "w").write("00\n")
print(m.init_stand_ins(d))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "[('array_a.mem', 'keccak.v: the default of MEMORY_INIT_file, which no tool emitted')]",
        "only the default no file answers gets a stand-in"
    );
}

/// A block whose design needs a stand-in is staged as itself: its row names its own candidate, its stand-in sits among
/// its files ahead of the wrapper (the last file, the bench's own), empty and marked with why it stands in.
#[test]
fn a_block_with_a_stand_in_is_staged_as_its_own_candidate() {
    let out = drive(
        r#"
import os, tempfile
stage_root = tempfile.mkdtemp()
m.STAGE_ROOT = stage_root
rows = []
m.qorlib.update = lambda kind, header, row: rows.append(row)
template = ('module ARRAY_1D #(parameter MEMORY_INIT_file="array_a.mem") (input clock);\n'
            '  reg [7:0] memory [0:3];\n  initial $readmemb(MEMORY_INIT_file, memory);\nendmodule\n'
            'module decide(input clock, input reset); endmodule\n')
def emit(run_dir, kernel, candidate, mode, ops, module=None):
    open(os.path.join(run_dir, "decide.v"), "w").write(template)
    if module:
        open(os.path.join(run_dir, module + ".v"), "w").write("module %s(input clk, input rst, output [15:0] sig); endmodule\n" % module)
    return {"candidate": {"name": candidate, "top": "decide", "clock": "clock"}, "source_kind": "third-party",
            "files": ["decide.v"]}
m.qorlib.emit = emit
m.qorlib.wrap_candidate = lambda run_dir: {}
def fold(cmd, capture_output=True, text=True):
    open(cmd[3], "w").write("module qorsyn_%s(input clk, input rst, output [15:0] sig); endmodule\n" % cmd[4])
    return type("R", (), {"returncode": 0, "stderr": ""})()
m.subprocess.run = fold
for role in ("attempt", "control", "power"):
    block, row = m.stage("K4", "k4_bambu_fsm_ports_a7p10", 10.0, role)
    d = os.path.join(stage_root, block)
    print(role, row["candidate"], [f["name"] for f in row["files"]], [bool(f.get("stand_in")) for f in row["files"]],
          os.path.getsize(os.path.join(d, "array_a.mem")))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "attempt k4_bambu_fsm_ports_a7p10 ['decide.v', 'array_a.mem', 'qorsyn_k4_bambu_fsm_ports_a7p10_p10.v'] [False, True, False] 0",
            "control k4_bambu_fsm_ports_a7p10 ['decide.v', 'array_a.mem', 'qorsyn_k4_bambu_fsm_ports_a7p10_p10_stock.v'] [False, True, False] 0",
            "power k4_bambu_fsm_ports_a7p10 ['decide.v', 'array_a.mem', 'qorpwr_k4_bambu_fsm_ports_a7p10_p10.v'] [False, True, False] 0",
        ],
        "the row names its candidate; the stand-in is empty, marked, and ahead of the wrapper"
    );
}

/// A kernel's searches start where `starts` names a period for the candidate (`--start-from`: a JSON map of candidate to
/// period, from the synthesis estimate), and at `start` otherwise.
#[test]
fn a_search_starts_at_the_period_named_for_its_candidate() {
    let out = drive(
        r#"
import threading
lock, began = threading.Lock(), {}
def search(kernel, cand, start, deadline):
    with lock:
        began[cand] = start
m.search = search
m.iso = lambda kernel, deadline, jobs=1: None
m.iso_period = lambda k: 12.5
m.gated_candidates = lambda k: ["k3_a", "k3_b", "k3_c"]
m.kernel("K3", 3, 10.0, 60, [], {"k3_b": 21.5, "k6_z": 300.0})
print(sorted(began.items()))
began.clear()
m.kernels(["K3"], 3, 10.0, 60, [], {"k3_c": 7.5})
print(sorted(began.items()))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "[('k3_a', 10.0), ('k3_b', 21.5), ('k3_c', 10.0)]", "the named period, else the start");
    assert_eq!(lines[1], "[('k3_a', 10.0), ('k3_b', 10.0), ('k3_c', 7.5)]", "kernels passes the map to each kernel");
}
