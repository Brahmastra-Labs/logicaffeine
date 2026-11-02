//! qor_recorders_lock — the plumbing every recorder of the comparative-synthesis bench shares, and the evidence the
//! Artix-7 runner keeps of a run that did not finish.
//!
//! A. an excerpt directory holds exactly what its run wrote: a run recorded again leaves nothing of the run before,
//!    and a run with no excerpts has no directory, so `verify` never meets a directory no row names;
//! B. a place-and-route excerpt keeps the router's start and its last five iterations, so a routing timeout shows the
//!    router at work even when its first iteration never finished — and one that never reached the router does not
//!    pass for a routing timeout;
//! C. the K7 traffic gate's recorder (`scripts/qor/run-qor-dram.py verify`) refuses a row its excerpts do not bear
//!    out: a measure or a verdict other than its judgement's, an operating point other than its stage's (the part, its
//!    clock, burst and CAS latency, the words, the stream and the probe's spacing), an edited excerpt, and an excerpt
//!    directory no row names;
//! D. a Sky130 run's flow settings are what the compiler emits for a bench block — the config `emit_flow_config` gives
//!    a probe block, comparative, at the bench's routing cores, the design's own fields left out — so they move with
//!    any setting a bench block gets and with nothing else (a signoff block re-recorded, a comment rewritten), and an
//!    emitter older than the compiler source it comes from is refused; a run's flow names the image it runs, the
//!    bench's (`scripts/openroad/build-openroad.sh`) for a bench run and a control alike;
//! E. an Artix-7 run nextpnr's default router cannot finish routing within its deadline is run again with its older
//!    router, which resolves the congestion the default oscillates on: the row is the second run's, says which router
//!    routed it and what the first did, and keeps the first's router log; any other refusal stands as it is;
//! F. a row and the excerpts it names are committed in one hold of the ledger's lock — a reader holding it (a
//!    snapshot of the bench) never meets a run's excerpts beside the row of the run before, nor a row before its
//!    excerpts — and a row naming other excerpts than its run gives is refused before anything is written. Every
//!    recorder commits through that one step: none writes an excerpt directory itself.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run `body` (Python) with `scripts/qor/qorlib.py` imported as `qorlib`, `scripts/qor/run-qor-fpga.py` as `fpga` and
/// `scripts/qor/run-qor-dram.py` as `dram`, `work` a fresh directory; return what it prints.
fn drive(body: &str) -> String {
    let work = tempfile::tempdir().unwrap();
    let prelude = r#"
import importlib.util, os, sys
sys.path.insert(0, os.path.dirname(sys.argv[1]))
import qorlib
spec = importlib.util.spec_from_file_location("fpga", sys.argv[2])
fpga = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fpga)
spec = importlib.util.spec_from_file_location("dram", sys.argv[4])
dram = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dram)
work = sys.argv[3]
"#;
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("{prelude}\n{body}"))
        .arg(root().join("scripts/qor/qorlib.py"))
        .arg(root().join("scripts/qor/run-qor-fpga.py"))
        .arg(work.path())
        .arg(root().join("scripts/qor/run-qor-dram.py"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn an_excerpt_directory_holds_exactly_what_its_run_wrote() {
    let out = drive(
        r#"
qorlib.EXCERPTS = work
paths, digests = qorlib.write_excerpts("k", [("a.txt", "one"), ("b.txt", "two")])
print(sorted(os.listdir(os.path.join(work, "k"))), sorted(paths), digests["a.txt"] == qorlib.sha3_text("one"))
paths, _ = qorlib.write_excerpts("k", [("a.txt", "three")])
print(sorted(os.listdir(os.path.join(work, "k"))), open(os.path.join(work, "k", "a.txt")).read())
print(qorlib.write_excerpts("k", []), os.path.exists(os.path.join(work, "k")))
print(qorlib.write_excerpts("never", []), os.path.exists(os.path.join(work, "never")))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "['a.txt', 'b.txt'] ['a.txt', 'b.txt'] True");
    assert_eq!(lines[1], "['a.txt'] three", "the second run's excerpts alone");
    assert_eq!(lines[2], "({}, {}) False", "a run with none leaves no directory");
    assert_eq!(lines[3], "({}, {}) False");
}

#[test]
fn a_routing_timeout_keeps_the_router_start_and_a_placement_one_does_not_pass_for_it() {
    let out = drive(
        r#"
util = "Info: Device utilisation:\nInfo: \t          SLICE_LUTX:   468/126800     0%\n"
started = util + "Info: Running router2...\nInfo: Setting up routing resources...\n"
iters = "".join("Info:     iter=%d wires=%d overused=%d\n" % (i, 1000 + i, 9 - i) for i in range(1, 9))
placing = util + "Info: Running placer...\nInfo: Max frequency for clock 'clk': 30.19 MHz (FAIL at 100.00 MHz)\n"
for name, log in (("started", started), ("iterating", started + iters), ("placing", placing)):
    s = fpga.pnr_summary(log)
    kept = [l.split("iter=")[1].split()[0] for l in s.splitlines() if l.startswith("Info:     iter=")]
    print(name, fpga.router_started(s), s.count("Running router"), ",".join(kept) or "-")
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "started True 1 -", "a router that never finished its first iteration still shows its start");
    assert_eq!(lines[1], "iterating True 1 4,5,6,7,8", "the start and the last five of its eight iterations");
    assert_eq!(lines[2], "placing False 0 -", "a run killed while placing never passes for a routing timeout");
}

#[test]
fn the_traffic_gates_recorder_refuses_a_row_its_excerpts_do_not_bear_out() {
    let out = drive(
        r#"
import contextlib, io, json
qorlib.EXCERPTS = work
judged = {m: {"n": 1} if m.endswith("latency") or m in ("read_service", "read_bus") else 7 for m in dram.METRICS}
stage = {"part": "MT47H64M16HR-25", "tck_ps": 5000, "bl": 8, "cl": 3, "words": 4, "pattern": "probe", "every": 128}
def row(judgement=judged, **over):
    paths, digests = qorlib.write_excerpts("dram/a", [("dram.json", json.dumps(stage)), ("sim.txt", "PASS\n"),
                                                      ("judgement.json", json.dumps(judgement))])
    r = {"key": "a", "pass": "refusal" not in judgement, "refusal": judgement.get("refusal"), "excerpts": paths,
         "excerpt_sha3": digests, **{m: judgement.get(m) for m in dram.METRICS}, **stage}
    r.update(over)
    return r
def edited():
    r = row()
    open(os.path.join(work, "dram", "a", "sim.txt"), "w").write("FAIL\n")
    return r
def orphan():
    r = row()
    os.makedirs(os.path.join(work, "dram", "b"))
    open(os.path.join(work, "dram", "b", "sim.txt"), "w").write("PASS\n")
    return r
cases = [("clean", row), ("measure", lambda: row(stream_cycles=8)), ("verdict", lambda: row(**{"pass": False})),
         ("refused", lambda: row({"refusal": "tRCD: READ 4 clocks after ACT"}, **{"pass": True, "refusal": None})),
         ("reason", lambda: row({"refusal": "tRCD: READ 4 clocks after ACT"}, refusal="tRP: ACT 3 clocks after PRE")),
         ("staged", lambda: row(tck_ps=2500)), ("edited", edited), ("orphan", orphan),
         ("spaced", lambda: row(every=64))]
for name, make in cases:
    import shutil
    shutil.rmtree(os.path.join(work, "dram"), ignore_errors=True)
    rows = [make()]
    dram.qorlib.load = lambda kind, header, rows=rows: {"rows": rows}
    err = io.StringIO()
    with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
        try:
            dram.verify(False)
            verdict = "ok"
        except SystemExit as e:
            verdict = "refused"
    first = next((l[len("FAIL: "):] for l in err.getvalue().splitlines() if l.startswith("FAIL: ")), "")
    print("%s %s %s" % (name, verdict, first.replace(work, "<work>")))
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "clean ok ", "a row its excerpts bear out");
    assert_eq!(lines[1], "measure refused a: stream_cycles is 7 in the judgement, 8 in the row");
    assert_eq!(lines[2], "verdict refused a: the verdict disagrees with the judgement");
    assert_eq!(lines[3], "refused refused a: the verdict disagrees with the judgement", "a pass over a refusing judgement");
    assert_eq!(lines[4], "reason refused a: the verdict disagrees with the judgement", "another refusal than the judgement's");
    assert_eq!(lines[5], "staged refused a: tck_ps is 5000 staged, 2500 in the row");
    assert_eq!(lines[6], "edited refused a: excerpt sim.txt was edited");
    assert!(lines[7].starts_with("orphan refused ") && lines[7].ends_with("dram/b: no row names this excerpt"), "{}", lines[7]);
    assert_eq!(lines[8], "spaced refused a: every is 128 staged, 64 in the row", "the probe's spacing is the run's operating point");
}

#[test]
fn the_flow_settings_are_what_the_compiler_emits_for_a_bench_block() {
    let out = drive(
        r##"
import json, subprocess, time
os.environ.pop("ROUTING_CORES", None)
emit = os.path.join(work, "emit_flow_config")
cfg = {"DESIGN_NAME": "qor_flow_probe", "VERILOG_FILES": "dir::src/*.v", "CLOCK_PORT": "clk", "CLOCK_PERIOD": 10,
       "ROUTING_CORES": 8, "PL_TARGET_DENSITY": 0.45}
def emitter(c):
    open(emit, "w").write("#!/bin/sh\necho \"$@\" > %s/args.txt\ncat <<'J'\n%s\nJ\n" % (work, json.dumps(c)))
    os.chmod(emit, 0o755)
qorlib.EMIT_FLOW_CONFIG = emit
emitter(cfg)
a = qorlib.flow_settings()
own = {k: v for k, v in cfg.items() if k not in ("DESIGN_NAME", "CLOCK_PORT", "CLOCK_PERIOD")}
print(a == qorlib.sha3_text(json.dumps(own, sort_keys=True)))
print(open(os.path.join(work, "args.txt")).read().strip())
os.environ["ROUTING_CORES"] = "4"
qorlib.flow_settings()
os.environ.pop("ROUTING_CORES")
print(open(os.path.join(work, "args.txt")).read().strip())
emitter(dict(cfg, DESIGN_NAME="qorsyn_other", CLOCK_PORT="clock", CLOCK_PERIOD=12.5))
print(qorlib.flow_settings() == a)
emitter(dict(cfg, PL_TARGET_DENSITY=0.4))
print(qorlib.flow_settings() == a)
print(sorted(qorlib.flow_identity(False)), sorted(qorlib.flow_identity(True)), qorlib.flow_identity(True)["overlay_sha3"])
old = time.time() - 365 * 86400
os.utime(emit, (old, old))
try:
    qorlib.flow_settings()
    print("not refused")
except SystemExit as e:
    print("refused:", str(e).split(":")[0])
os.utime(emit, None)
built = subprocess.run(["bash", os.path.join(qorlib.ROOT, "scripts/openroad/build-openroad.sh"), "--name"],
                       capture_output=True, text=True, check=True).stdout.strip()
print(qorlib.flow_identity(False)["openroad_image"] == qorlib.flow_identity(True)["openroad_image"] == built,
      built.startswith("logos-openlane:%s-openroad-" % qorlib.flow_identity(True)["openlane"][:8]))
"##,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "True", "the emitted config, its design's own fields left out");
    assert_eq!(lines[1], "qor_flow_probe clk 10 --pdk sky130A --cores 8 --comparative", "a comparative probe block at the bench's cores");
    assert_eq!(lines[2], "qor_flow_probe clk 10 --pdk sky130A --cores 4 --comparative", "the cores the bench runs with");
    assert_eq!(lines[3], "True", "another design's name, clock port or period is the same flow");
    assert_eq!(lines[4], "False", "any setting a bench block gets moves it");
    assert_eq!(
        lines[5],
        r#"['flow_settings_sha3', 'ir_analysis_sha3', 'openlane', 'openroad_image', 'overlay_sha3'] ['flow_settings_sha3', 'ir_analysis_sha3', 'openlane', 'openroad_image', 'overlay_sha3'] None"#,
        "a run's flow: OpenLane's commit, the image it runs, the overlay (none on stock OpenLane), the settings and the IR analysis"
    );
    assert_eq!(lines[7], "True True", "a bench run and a control run the bench's image, named by its build, at OpenLane's commit");
    assert!(
        lines[6].starts_with("refused: crates/logicaffeine_compile/") && lines[6].ends_with(" is newer than the flow-config emitter"),
        "an emitter older than its source: {}",
        lines[6]
    );
}

#[test]
fn a_run_the_default_router_cannot_finish_is_run_again_with_the_older_router() {
    let out = drive(
        r#"
calls = []
def outcome(verdict_by_router):
    def run(kernel, candidate, mapping, seed, freq, deadline, router="router2", prior=()):
        calls.append((router, [n for n, _ in prior]))
        v = verdict_by_router[router]
        return {"verdict": v, "wall_s": 12.5, "router": router, "flow": "f" + ("" if router == "router2" else " --router " + router),
                "_pnr": "iter=9 overused=1\n" if router == "router2" else "Routing complete\n"}
    return run
cases = [
    ("routes", {"router2": "complete"}),
    ("oscillates", {"router2": "refused: routing did not finish within 1800 s", "router1": "complete"}),
    ("both", {"router2": "refused: routing did not finish within 1800 s", "router1": "refused: routing did not finish within 1800 s"}),
    ("placement", {"router2": "refused: placement did not finish within 1800 s", "router1": "complete"}),
    ("pnr", {"router2": "refused: place-and-route", "router1": "complete"}),
]
for name, verdicts in cases:
    calls.clear()
    row = fpga.ladder(outcome(verdicts), "K4", "k4_logos", "fabric", 1, 100.0, 1800)
    print(name, row["verdict"], row["router"], [c[0] for c in calls], calls[-1][1], row.get("router_ladder"))
print(fpga.ROUTERS)
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "routes complete router2 ['router2'] [] None", "a run that routes is run once");
    assert_eq!(
        lines[1],
        "oscillates complete router1 ['router2', 'router1'] ['pnr-router2.txt'] [{'router': 'router2', 'verdict': 'refused: routing did not finish within 1800 s', 'wall_s': 12.5}]",
        "the older router's run, the default's verdict and its router log kept"
    );
    assert!(lines[2].starts_with("both refused: routing did not finish within 1800 s router1 ['router2', 'router1']"), "{}", lines[2]);
    assert_eq!(lines[3], "placement refused: placement did not finish within 1800 s router2 ['router2'] [] None", "only routing is retried");
    assert_eq!(lines[4], "pnr refused: place-and-route router2 ['router2'] [] None");
    assert_eq!(lines[5], "('router2', 'router1')");
}

#[test]
fn a_row_and_its_excerpts_are_committed_in_one_hold_of_the_ledgers_lock() {
    let out = drive(
        r#"
import contextlib, fcntl, io, json
qorlib.ROOT = qorlib.SIL = qorlib.LOGS = work
qorlib.EXCERPTS = os.path.join(work, "qor_logs")
held = []
write = qorlib.write_excerpts
def probe(key, files):
    with open(os.path.join(work, "qor_t.lock"), "w") as f:
        try:
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(f, fcntl.LOCK_UN)
            held.append("free")
        except BlockingIOError:
            held.append("held")
    return write(key, files)
qorlib.write_excerpts = probe
def row(key, files):
    paths, digests = qorlib.excerpt_record("t/" + key, files)
    return {"key": key, "excerpts": paths, "excerpt_sha3": digests}
def state():
    rows = qorlib.load("t", {"rows": []})["rows"]
    d = os.path.join(qorlib.EXCERPTS, "t", "r")
    on_disk = sorted((n, open(os.path.join(d, n)).read()) for n in os.listdir(d)) if os.path.isdir(d) else []
    named = [sorted(r.get("excerpts", {})) for r in rows if r["key"] == "r"]
    bad = [p for r in rows if r.get("excerpts") for p in qorlib.read_excerpts(r)[1]]
    return "%s %s %s" % (on_disk, named, bad or "consistent")
def attempt(name, r, given):
    held.clear()
    try:
        with contextlib.redirect_stderr(io.StringIO()):
            qorlib.update("t", {"rows": []}, r, given)
        verdict = "ok"
    except SystemExit as e:
        verdict = "refused"
    print(name, verdict, held, state())
one = [("a.txt", "one"), ("b.txt", "two")]
attempt("committed", row("r", one), ("t/r", one))
attempt("again", row("r", [("c.txt", "three")]), ("t/r", [("c.txt", "three")]))
attempt("unnamed", row("r", one), None)
attempt("other", row("r", one), ("t/r", [("a.txt", "one"), ("b.txt", "edited")]))
attempt("fewer", row("r", one), ("t/r", one[:1]))
attempt("bare", {"key": "s", "verdict": "complete"}, None)
attempt("none", row("r", []), ("t/r", []))

def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m
function = load(os.path.join(os.path.dirname(sys.argv[1]), "run-qor-function.py"), "function")
files = [("sim.txt", "QOR ops 1 misr 0000000000000001 cycles 9\n")]
for kind, rec, extra in (
        ("function", function.record, {"candidate": "c", "mode": "shaped", "pass": True, "signature": "1",
                                       "steady_cycles_per_op": 1.0}),
        ("artix7", fpga.record, {"candidate": "c", "mapping": "fabric", "seed": 1, "verdict": "complete", "wall_s": 1.0}),
        ("dram", dram.record, {"pass": True, "stream_cycles": 9, "read_latency": {"min": 1, "max": 2}})):
    paths, digests = qorlib.excerpt_record("%s/k" % kind, files)
    r = dict(extra, key="k", excerpts=paths, excerpt_sha3=digests, _excerpts=("%s/k" % kind, files))
    with contextlib.redirect_stdout(io.StringIO()):
        rec(r)
    got = qorlib.load(kind, {"rows": []})["rows"]
    print(kind, [g == dict(extra, key="k", excerpts=paths, excerpt_sha3=digests) for g in got],
          [qorlib.read_excerpts(g)[1] for g in got])
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "committed ok ['held'] [('a.txt', 'one'), ('b.txt', 'two')] [['a.txt', 'b.txt']] consistent",
        "the excerpts are written while the ledger's lock is held, and the row names exactly them"
    );
    assert_eq!(lines[1], "again ok ['held'] [('c.txt', 'three')] [['c.txt']] consistent", "the run recorded again, nothing of the run before");
    assert_eq!(lines[2], "unnamed refused [] [('c.txt', 'three')] [['c.txt']] consistent", "a row whose excerpts are not given");
    assert_eq!(lines[3], "other refused [] [('c.txt', 'three')] [['c.txt']] consistent", "an excerpt other than the row names");
    assert_eq!(lines[4], "fewer refused [] [('c.txt', 'three')] [['c.txt']] consistent", "fewer excerpts than the row names");
    assert_eq!(lines[5], "bare ok [] [('c.txt', 'three')] [['c.txt']] consistent", "a row with no excerpts touches none");
    assert_eq!(lines[6], "none ok ['held'] [] [[]] consistent", "a run with no excerpts keeps no directory");
    assert_eq!(lines[7], "function [True] [[]]", "the functional gate commits its row with its excerpts");
    assert_eq!(lines[8], "artix7 [True] [[]]", "the Artix-7 runner commits its row with its excerpts");
    assert_eq!(lines[9], "dram [True] [[]]", "the traffic gate commits its row with its excerpts");
}

#[test]
fn no_recorder_writes_an_excerpt_directory_but_through_its_ledger_update() {
    let qor = root().join("scripts/qor");
    let mut recorders = 0;
    for entry in std::fs::read_dir(&qor).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "py") && path.file_name().unwrap() != "qorlib.py" {
            let text = std::fs::read_to_string(&path).unwrap();
            if text.contains("\"excerpt_sha3\"") {
                recorders += 1;
            }
            assert!(
                !text.contains("write_excerpts("),
                "{} writes excerpts outside qorlib.update, where a snapshot holding the ledger's lock can see them apart from their row",
                path.display()
            );
        }
    }
    assert!(recorders >= 4, "the function, Artix-7, traffic and power recorders name excerpts: found {recorders}");
    let lib = std::fs::read_to_string(qor.join("qorlib.py")).unwrap();
    assert_eq!(lib.matches("write_excerpts(").count(), 2, "qorlib defines write_excerpts and calls it once, in update");
}

/// A run's flow names the IR analysis its verdict gates on: `scripts/anchor-ir-drop.sh`, which replays the run's irdrop
/// step with anchored sources, and `scripts/gen-vsrc-loc.py`, which places them. Its output is a report frozen in the
/// run directory, so re-deriving a record cannot see it change: a byte of either file moves the identity of a bench
/// run and of a control alike, while the recorder, whose verdicts every verify re-derives, moves neither. An identity
/// whose analysis cannot be read is refused.
#[test]
fn a_runs_flow_names_the_ir_analysis_its_verdict_gates_on() {
    let out = drive(
        r#"
import shutil, tempfile
real = qorlib.ROOT
sb = tempfile.mkdtemp()
os.makedirs(os.path.join(sb, "scripts"))
for d in ("openlane-overlay", "openroad"):
    os.symlink(os.path.join(real, "scripts", d), os.path.join(sb, "scripts", d))
for f in ("anchor-ir-drop.sh", "gen-vsrc-loc.py", "record-sky130-signoff.py"):
    shutil.copy(os.path.join(real, "scripts", f), os.path.join(sb, "scripts", f))
qorlib.ROOT = sb
qorlib.flow_settings = lambda: "5" * 64
bench, control = qorlib.flow_identity(False), qorlib.flow_identity(True)
print(bench["ir_analysis_sha3"] == control["ir_analysis_sha3"], len(bench["ir_analysis_sha3"]))
for f in ("anchor-ir-drop.sh", "gen-vsrc-loc.py", "record-sky130-signoff.py"):
    p = os.path.join(sb, "scripts", f)
    kept = open(p).read()
    open(p, "a").write("\n")
    print(f, qorlib.flow_identity(False)["ir_analysis_sha3"] != bench["ir_analysis_sha3"],
          qorlib.flow_identity(True)["ir_analysis_sha3"] != control["ir_analysis_sha3"])
    open(p, "w").write(kept)
print(qorlib.flow_identity(False) == bench)
os.remove(os.path.join(sb, "scripts", "gen-vsrc-loc.py"))
try:
    qorlib.flow_identity(False)
    print("computed")
except (OSError, SystemExit):
    print("refused")
"#,
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "True 64", "a bench run and a control gate on one IR analysis, named by its SHA3-256");
    assert_eq!(lines[1], "anchor-ir-drop.sh True True", "the replay moves the identity");
    assert_eq!(lines[2], "gen-vsrc-loc.py True True", "the source placement moves the identity");
    assert_eq!(lines[3], "record-sky130-signoff.py False False", "the recorder is re-derived, not named");
    assert_eq!(lines[4], "True", "the same analysis is the same identity");
    assert_eq!(lines[5], "refused", "an analysis that cannot be read names no flow");
}
