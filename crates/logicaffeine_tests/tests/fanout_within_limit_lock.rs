//! fanout_within_limit_lock — every net a block ships is within the SDC's `set_max_fanout`, counting the loads the
//! liberty counts.
//!
//! The resizer meets the limit on every logic net: all 23 signed-off blocks carried 0 logic nets over it. Every
//! violation signoff reported came from the passes that run after it. Antenna diodes are loads (sky130's liberty
//! gives `DIODE` the library's default fanout load, 1), so ten mux2 inputs with a heuristic diode each read as
//! fanout 20 — 5,052 of `wpe_user_project`'s 5,755 violators were that — and clock-tree leaves were the rest: a
//! leaf cluster may hold 25 sinks and each H-tree split may give one branch 60% of its parent's clusters.
//!
//! The fix sits at each pass that adds a load. CTS builds within the limit (`cts.tcl`, `logos_cts.py`); a
//! diode-aware step splits what the diode passes take over it, by whole units — a pin with the diodes placed beside
//! it — so no diode leaves the net of the pin it protects (`logos_diode_fanout.tcl`, through the split's one home
//! `logos_diode_fanout_lib.tcl` over the planning core `logos_diode_fanout_core.tcl`); the post-diode upsize judges the
//! route detailed routing follows for antennas and splits by the same rules; the post-route diode ECO adds a diode only
//! where the net has room. This lock holds each of those in place and runs the core's own tests.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::flow_config::{
    openlane_config, ClosureSettings, DesignProfile,
};
use logicaffeine_compile::codegen_sva::pdk_target::SKY130A;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn overlay(rel: &str) -> String {
    std::fs::read_to_string(root().join("scripts/openlane-overlay/scripts").join(rel))
        .unwrap_or_else(|e| panic!("read the overlay's {rel}: {e}"))
}

/// The body of a Tcl `proc` (or of the top-level block that starts at `start`), up to its closing brace at
/// column zero.
fn block<'a>(text: &'a str, start: &str) -> &'a str {
    let at = text
        .find(start)
        .unwrap_or_else(|| panic!("{start:?} is gone"));
    let body = &text[at..];
    &body[..body.find("\n}\n").unwrap_or(body.len())]
}

fn cfg(c: &ClosureSettings) -> String {
    openlane_config(&DesignProfile::new("probe", "clk", 10.0), &SKY130A, c)
}

#[test]
fn the_planning_core_passes_its_own_tests() {
    if !common::require_tool("tclsh") {
        return;
    }
    let t =
        root().join("scripts/openlane-overlay/scripts/openroad/tests/test_diode_fanout_core.tcl");
    let out = std::process::Command::new("tclsh")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run tclsh");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: diode-aware fanout planning"),
        "{log}"
    );
}

/// The core's tests keep the measured shapes, not synthetic stand-ins: ten mux2 inputs each beside a heuristic
/// diode named for it, the twelve diodes global routing's repair put beside the driver and the one gate of
/// coproc_core's _29509_, the optimum for one-diode gates over every net size up to 100, and the property sweep over
/// random mixes.
#[test]
fn the_planning_tests_cover_the_measured_shapes() {
    let t = overlay("openroad/tests/test_diode_fanout_core.tcl");
    for needle in [
        "ten mux2 select inputs each pair with their own heuristic diode",
        "a fanout-20 net of ten gates and their diodes splits into three bins under a cap of 9",
        "gates carrying one heuristic diode pack optimally under a cap of 9, 1 to 100 of them",
        "every unit lands in exactly one bin and no bin outweighs the cap",
        "the packing does not depend on the order the units arrive in",
        "a gate carrying eight diodes takes a bin of its own",
        "a global-route diode pairs with the nearest gate",
        "a pin name with an underscore still pairs by name",
        "a buffer carrying its own input diode costs the net it leaves two loads",
        "a net of ten guarded gates under a cap of 9 keeps no whole bin beside two-load buffers",
        "the most critical gates stay on the driver, carved in slack order while they fit",
        "with no slack known the gates nearest the driver stay",
        "a gate too heavy to stay beside the others' buffers is not kept",
        "a global-route diode beside the driver pairs with the driver",
        "...and one beside the gate, or beside the gate's own diode, with the gate",
        "the diodes beside the driver stay on the net, so a bin is kept only with room beside them",
        "the carve leaves room for the diodes beside the driver",
        "the diodes beside a clock driver, a unit at the driver, shed first",
    ] {
        assert!(t.contains(needle), "the core's test lost {needle:?}");
    }
}

#[test]
fn both_halves_are_on_by_default_and_each_can_be_turned_off() {
    let c = ClosureSettings::default();
    assert!(
        c.cts_within_fanout && c.diode_fanout_repair,
        "every block is built within the fanout limit"
    );
    assert_eq!(
        c.fanout_margin_loads, 1,
        "one load of room for the post-route diode ECO"
    );
    let on = cfg(&c);
    assert!(
        on.contains("\"LOGOS_CTS_FANOUT\": 1,")
            && on.contains("\"LOGOS_DIODE_FANOUT\": 1,")
            && on.contains("\"LOGOS_FANOUT_MARGIN\": 1,"),
        "{on}"
    );
    let off = cfg(&ClosureSettings {
        cts_within_fanout: false,
        diode_fanout_repair: false,
        ..c
    });
    assert!(
        off.contains("\"LOGOS_CTS_FANOUT\": 0,") && off.contains("\"LOGOS_DIODE_FANOUT\": 0,"),
        "{off}"
    );
    let em = std::fs::read_to_string(
        root().join("crates/logicaffeine_compile/examples/emit_flow_config.rs"),
    )
    .unwrap();
    assert!(
        em.contains("\"--no-cts-fanout\" =>")
            && em.contains("\"--no-diode-fanout\" =>")
            && em.contains("\"--fanout-margin\" =>"),
        "the emitter exposes every lever"
    );
}

/// The clock tree: leaf clusters clamped to the limit in force, H-tree splits balanced, and — through the Python
/// driver, since this OpenROAD exposes the field to Python only — the tree subdivided until a leaf averages under
/// the limit. On unless the gate says 0, so a block re-signed from a recipe older than the gate
/// (`scripts/resign-block.sh` keeps a recorded config.json byte for byte) is built within the limit too.
#[test]
fn the_clock_tree_is_built_within_the_limit() {
    let cts = overlay("openroad/cts.tcl");
    let gate = block(
        &cts,
        "if { ![info exists ::env(LOGOS_CTS_FANOUT)] || $::env(LOGOS_CTS_FANOUT) == 1 } {",
    );
    assert!(
        gate.contains("set fanout_limit [logos_design_max_fanout]"),
        "the clamp reads the design limit in force:\n{gate}"
    );
    assert!(gate.contains("if { $sink_clustering_size > $fanout_limit } { set sink_clustering_size $fanout_limit }"),
        "a leaf cluster holds at most the limit's sinks:\n{gate}");
    assert!(
        gate.contains("lappend arg_list -clustering_unbalance_ratio 0.5"),
        "the splits are balanced:\n{gate}"
    );
    let call = cts
        .find("clock_tree_synthesis {*}$arg_list")
        .expect("the CTS call");
    assert!(
        cts.find("lappend arg_list -sink_clustering_size $sink_clustering_size")
            .expect("the clamped size")
            < call,
        "the clamped size is the one CTS receives"
    );
    assert!(
        !cts.contains("-sink_clustering_size $::env(CTS_SINK_CLUSTERING_SIZE)"),
        "no path bypasses the clamp"
    );

    let py = overlay("openroad/logos_cts.py");
    assert!(py.contains("setNumMaxLeafSinks(limit)")
            && py.contains("int(round(float(os.environ[\"MAX_FANOUT_CONSTRAINT\"]))) - int(os.environ.get(\"LOGOS_FANOUT_MARGIN\", \"1\"))"),
        "the driver sets the H-tree's leaf average from the design limit, read as io.tcl reads it:\n{py}");
    let set = py.find("setNumMaxLeafSinks").unwrap();
    let run = py
        .find("openroad/cts.tcl")
        .expect("the driver runs the flow's cts.tcl");
    assert!(set < run, "the depth is set before CTS runs");
    assert!(
        py.contains("sys.exit(1)"),
        "a failing cts.tcl fails the step"
    );

    let step = overlay("tcl_commands/cts.tcl");
    let launch = block(
        &step,
        "        if { ![info exists ::env(LOGOS_CTS_FANOUT)] || $::env(LOGOS_CTS_FANOUT) == 1 } {",
    );
    assert!(
        launch.contains("openroad/logos_cts.py") && launch.contains("-python"),
        "{launch}"
    );
    let utils = overlay("utils/utils.tcl");
    let rts = block(&utils, "proc run_tcl_script {args} {");
    assert!(
        rts.contains("set flags {-def_in -netlist_in -gui -no_update_current -python}"),
        "the flag is declared"
    );
    assert!(
        rts.contains(
            "if { [info exists flag_map(-python)] } {\n            lappend args -no_init -python"
        ),
        "a Python script runs under OpenROAD's interpreter, as manipulate_layout runs odbpy"
    );
}

/// The diode-aware step runs last before the routers see the netlist, and again after global routing's antenna
/// repair, inside the one re-route the post-antenna upsize already pays for — on unless the gate says 0.
#[test]
fn the_fanout_step_runs_after_every_pre_route_diode_pass() {
    let rt = overlay("tcl_commands/routing.tcl");
    let body = block(&rt, "proc run_routing {");
    let diodes = body
        .find("heuristic_diode_insertion")
        .expect("the heuristic diode pass");
    let upsize = body
        .find("$::env(LOGOS_POST_DIODE_UPSIZE) == 1")
        .expect("the post-diode upsize gate");
    let pre = body.find("if { ![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1 } {\n        run_logos_diode_fanout")
        .expect("the pre-route fanout step, on unless the gate says 0");
    let obs = body
        .find("\n    add_route_obs\n")
        .expect("route obstructions");
    let grt = body.find("\n    global_routing\n").expect("global routing");
    assert!(
        diodes < pre && pre < upsize && upsize < obs && obs < grt,
        "diodes, fanout, then the upsize that sizes the nets the step built, then the routers"
    );

    let post = &body[grt..];
    let gate = post
        .find("set _logos_post_fanout")
        .expect("the post-repair fanout gate");
    let gate_line = &post[gate..post[gate..].find("}]").map(|i| gate + i).unwrap()];
    assert!(
        gate_line.contains(
            "(![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1)"
        ) && gate_line.contains("$::env(GRT_REPAIR_ANTENNAS) == 1"),
        "it runs after global routing only when that routing repaired antennas:\n{gate_line}"
    );
    let fan = post
        .find("if { $_logos_post_fanout } {\n            run_logos_diode_fanout")
        .expect("the post-repair call");
    let post_up = post
        .find("if { $_logos_post_upsize } {\n            run_logos_post_diode_upsize")
        .expect("the post-repair upsize");
    assert!(
        fan < post_up,
        "the split comes before the upsize after global routing too"
    );
    let off = post
        .find("set ::env(GRT_REPAIR_ANTENNAS) 0")
        .expect("repair off for the re-route");
    let regr = post[off..]
        .find("global_routing")
        .map(|i| off + i)
        .expect("the re-route");
    let drt = post.find("detailed_routing").expect("detailed routing");
    assert!(
        gate < fan && fan < off && off < regr && regr < drt,
        "split, re-route once without repair, then route"
    );

    let proc_ = block(&rt, "proc run_logos_diode_fanout {args} {");
    assert!(
        proc_.contains("openroad/logos_diode_fanout.tcl")
            && proc_.contains(
                "-save \"dir=$::env(routing_tmpfiles),def,sdc,odb,netlist,powered_netlist\""
            ),
        "the step saves every view the routers read:\n{proc_}"
    );
}

/// The quiet object lookups in a script that it tests for absence without passing them through `logos_fo_obj` —
/// OpenSTA answers a lookup that finds nothing with the string `NULL`.
fn unguarded_quiet_lookups(tcl: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (n, line) in tcl.lines().enumerate() {
        let code = line.split('#').next().unwrap();
        let mut rest = code;
        while let Some(at) = rest.find("[get_") {
            let tail = &rest[at..];
            let call = &tail[..tail.find(']').map_or(tail.len(), |e| e + 1)];
            let is_object = ["[get_nets ", "[get_cells ", "[get_pins ", "[get_ports "]
                .iter()
                .any(|k| call.starts_with(k));
            if is_object && call.contains(" -quiet") && !rest[..at].ends_with("[logos_fo_obj ") {
                bad.push(format!("{}: {}", n + 1, line.trim()));
            }
            rest = &rest[at + 1..];
        }
    }
    bad
}

#[test]
fn the_fanout_step_keeps_every_diode_on_its_gates_net_and_never_splits_a_clock() {
    let s = overlay("openroad/logos_diode_fanout.tcl");
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    assert!(
        s.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl")
            && lib.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_core.tcl"),
        "it plans with the tested core, through the split's own file"
    );
    assert!(
        s.contains("set cap [logos_design_max_fanout]"),
        "every net is built to the design limit"
    );
    let cands = block(&lib, "proc logos_fo_candidates {block cap} {");
    assert!(
        cands.contains("if { $loads <= $cap || [llength $gates] == 0 } { continue }"),
        "a net is selected against the design limit, diodes counted:\n{cands}"
    );

    let ret = block(&lib, "proc logos_fo_return_named {block} {");
    assert!(
        ret.contains("logos_fo_named_owner") && ret.contains("$dt connect $gnet"),
        "a diode an earlier pass moved goes back to its gate's net:\n{ret}"
    );
    let call_ret = s
        .find("set repaired [logos_fo_return_named $block]")
        .expect("diodes return to their gates before anything is counted");
    let call_cands = s
        .find("lassign [logos_fo_candidates $block $cap] candidates pin_slack skipped_dont_touch")
        .expect("the candidates and their slack");
    let call_split = s
        .find("foreach net $candidates {\n    logos_fo_split_net $block $db $net $cap $pin_slack $heuristic st\n}")
        .expect("every candidate is split");
    assert!(
        s[call_ret..call_cands].contains("puts \"\\[LOGOS-FANOUT\\] returned $repaired"),
        "the count is reported, before the selection counts loads"
    );
    assert!(
        call_ret < call_cands && call_cands < call_split,
        "diodes return, then every slack is read, then nets are edited"
    );
    assert!(
        cands.contains("[$net isDoNotTouch]"),
        "a dont_touch net is left alone:\n{cands}"
    );
    assert!(
        cands.contains("logos_fo_slack $path")
            && !cands.contains("disconnect")
            && !cands.contains("dbInst_create"),
        "the first pass reads every candidate gate's slack and edits nothing:\n{cands}"
    );
    let lp = block(
        &lib,
        "proc logos_fo_split_net {block db net cap pin_slack heuristic statsvar} {",
    );
    let clock = lp
        .find("if { [logos_fo_is_clock $net] } {")
        .expect("the clock test");
    let detach = lp
        .find("foreach d $diodes { $d disconnect }")
        .expect("diodes leave before the split");
    let bins = lp
        .find("logos_fo::bins $units $cap")
        .expect("the split plans whole units");
    let back = lp
        .find("$d connect [$g getNet]")
        .expect("each diode returns to its gate's net");
    assert!(
        clock < detach && detach < bins && bins < back,
        "clocks shed diodes and return; signals split, then re-pair"
    );
    let clock_arm = &lp[clock..detach];
    assert!(
        clock_arm.contains("logos_fo::budget_trim") && clock_arm.contains("return"),
        "a clock net sheds diodes and is never split — its buffers are CTS's"
    );
    assert!(
        clock_arm.find("odb::dbInst_create").is_none(),
        "no buffer is ever placed on a clock net"
    );
    assert!(
        lp.contains("logos_fo::trim_unit $ds $gx $gy $cap"),
        "a gate whose diodes alone overflow keeps the nearest that fit"
    );
    assert!(
        lp.contains("logos_fo::kept_bin $bins $units $slack $sx $sy $cap"),
        "the bin that stays on the net is chosen by the gates' setup slack"
    );
    assert_eq!(lp.matches("$inst setLocation $cx $cy").count(), 2,
        "every split buffer, bin or level, sits at its bin's centroid — at the driver, each bin became its own long \
         wire from one point (systolic_os_pipe8: -1.888 ns against -0.092 at the centroid):\n{lp}");
    assert_eq!(lp.matches("if { $guard } { dict incr st guarded [logos_fo_guard_input $block $db $inst] }").count(), 2,
        "and every buffer input, bin or level, is guarded as the heuristic pass guards the net's other inputs \
         (logos_fo_92 unguarded took a met3 input 2.12x over the antenna limit):\n{lp}");
    assert!(lp.contains("set guard [expr { $heuristic && [logos_fo_net_span $block $net] >= $::env(HEURISTIC_ANTENNA_THRESHOLD) }]"),
        "the guard is the heuristic pass's own rule: its threshold on the net's span, only where the pass runs");
    assert!(
        lp.contains("logos_fo::kept_bin $bins $units $slack $sx $sy $cap $per_buffer"),
        "a guarded buffer costs the net it leaves two loads"
    );
    assert!(lp.contains("    if { $kept_bin < 0 } {\n        lassign [logos_fo::carve $units $slack $sx $sy $cap $per_buffer $fixed] carved rest_bins\n        if { [llength $carved] > 0 } {\n            set bins [concat [list $carved] $rest_bins]\n            set kept_bin 0"),
        "when no whole bin fits, the most critical gates are carved out and stay — on systolic_pipe16 every level of \
         a broadcast tree otherwise went behind a split buffer:\n{lp}");
    assert!(
        lp.contains("lappend level_units [list $n $cx $cy $per_buffer]")
            && lp.contains("lappend next [list $n $cx $cy $per_buffer]")
            && lp.contains("logos_fo_move_guard $block $child $bnet"),
        "a level carries each buffer with its diode, and moves the diode with it"
    );
    let span = block(&lim, "proc logos_fo_net_span {block net} {");
    assert!(
        span.contains("[lindex $xs end] - [lindex $xs 0] + [lindex $ys end] - [lindex $ys 0]")
            && span.contains("getBTerms"),
        "the span is the pass's own: the half-perimeter over every pin and port:\n{span}"
    );
    let gi = block(&lim, "proc logos_fo_guard_input {block db inst} {");
    assert!(
        gi.contains("\"ANTENNA_[$inst getConstName]_[[$it getMTerm] getConstName]\"")
            && gi.contains("connect [$it getNet]"),
        "the guard is named as the pass names its diodes, on the input's own net:\n{gi}"
    );
    let level = lp.find("while { $parent_w > $cap").expect("the level loop");
    assert!(
        back < level,
        "diodes return before the parent's weight is counted, so the level loop sees them"
    );

    let repair = block(&lib, "proc logos_fo_held_timing_repair {block measure} {");
    assert!(repair.contains("$net setDoNotTouch 1") && repair.contains("foreach net $held { $net setDoNotTouch 0 }"),
        "every dioded net is held while the flow's timing repair runs, and released after:\n{repair}");
    let hold_at = repair.find("$net setDoNotTouch 1").unwrap();
    let setup_at = repair.find("repair_timing -setup").expect("setup repair");
    let hold_rep = repair
        .find("repair_timing {*}$hold_args")
        .expect("hold repair");
    let release = repair.find("setDoNotTouch 0").unwrap();
    assert!(
        hold_at < setup_at && setup_at < hold_rep && hold_rep < release,
        "hold the nets, repair setup then hold, release"
    );
    let setup_call = &repair[setup_at..repair.find("set hold_args").expect("the hold arguments")];
    for arg in [
        "-setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN)",
        "-max_buffer_percent $::env(GLB_RESIZER_SETUP_MAX_BUFFER_PERCENT)",
    ] {
        assert!(
            setup_call.contains(arg),
            "the setup repair runs at the routing stage's own {arg}:\n{setup_call}"
        );
    }
    assert!(
        setup_call.contains("{*}[expr { $wns_pre > -$::env(CLOCK_PERIOD) ? {-repair_tns 100} : {} }]")
            && repair.find("set wns_pre").is_some_and(|w| w < setup_at),
        "every violating endpoint only while the design is within reach of its clock — less than a period short: a run \
         missing by more needs a longer period, and there the every-endpoint rebuffer walked a looped global route into \
         OpenROAD 41a51eaf's unbounded recursion (qorsyn_k2_logos_p10, -28.9 ns at a 10 ns clock, 10,450 frames):\n{setup_call}"
    );
    assert!(
        setup_call.contains("-repair_tns 100"),
        "the setup repair works every violating endpoint: a worst-endpoint repair stops where that one endpoint \
         stops improving, and the split slows many paths at once (soft_core_realbist RUN_2026.10.03_22.32.03, the \
         step replayed: -0.871 -> +0.016 ns worst endpoint only, -0.871 -> +0.061 ns every endpoint):\n{setup_call}"
    );
    let hold_call = &repair[repair
        .find("set hold_args [list -hold")
        .expect("the hold repair's arguments")..hold_rep];
    for arg in ["-setup_margin $::env(GLB_RESIZER_SETUP_SLACK_MARGIN)",
                "-hold_margin $::env(GLB_RESIZER_HOLD_SLACK_MARGIN)",
                "-max_buffer_percent $::env(GLB_RESIZER_HOLD_MAX_BUFFER_PERCENT)",
                "if { $::env(GLB_RESIZER_ALLOW_SETUP_VIOS) == 1 } { lappend hold_args -allow_setup_violations }"] {
        assert!(hold_call.contains(arg), "the hold repair runs at the routing stage's own {arg}:\n{hold_call}");
    }
    assert_eq!(
        repair.matches("-skip_pin_swap").count(),
        2,
        "neither repair swaps pins — a swap moves a net onto another pin"
    );
    let gate = repair
        .find("if { $::env(GLB_RESIZER_TIMING_OPTIMIZATIONS) != 1 } {")
        .expect("the repair follows the flow's own switch");
    let held_at = repair.find("set held [list]").unwrap();
    assert!(gate < held_at && repair[gate..held_at].contains("return"),
        "the timing repair runs only where the flow's global-route timing repair runs (the Caravel wrapper turns it off)");
    assert!(
        s.find("logos_fo_held_timing_repair $block logos_fo_measure")
            .expect("the step repairs timing")
            < s.find("set separated [logos_fo_separated $block]")
                .expect("the separation check"),
        "the separation check judges the repaired netlist"
    );
    assert!(
        !s.contains("$after <= $before") && !lib.contains("$after <= $before"),
        "no greedy per-buffer sizing: it grew drivers that slowed their siblings"
    );
    let measure = s
        .find("logos_fo_measure\nlassign [logos_fo_candidates $block $cap]")
        .expect("slack is read on the global-route estimate");
    assert!(measure < call_split, "before the edits");
    let iso = block(&lib, "proc logos_fo_isolate_driver {block db drv} {");
    assert!(iso.contains("$drv connect $hop")
            && iso.contains("if { [$mt getIoType] eq \"INPUT\" } { $bit connect $hop } else { $bit connect $net }")
            && iso.contains("lindex $bufs end 1"),
        "a maximal driver is isolated behind the library's strongest buffer — its input on the hop, its output on the \
         net, every load left where it is:\n{iso}");
    assert!(
        iso.find("if { [string match \"logos_fo_iso_*\" [$inst getConstName]] } { return 0 }")
            .unwrap_or(usize::MAX)
            < iso.find("odb::dbInst_create").unwrap(),
        "an isolation buffer is never isolated again — it is already the strongest:\n{iso}"
    );
    let rounds = block(
        &lib,
        "proc logos_fo_size_touched {block db touched measure} {",
    );
    assert!(
        rounds.contains("for {set round 1} {$round <= 12}")
            && rounds.contains("logos_fo_isolate_driver $block $db $it"),
        "a driver at maximum drive is isolated, never left violating:\n{rounds}"
    );
    let is_clock = block(&lim, "proc logos_fo_is_clock {net} {");
    assert!(
        is_clock.contains("[$net getSigType] eq \"CLOCK\"")
            && is_clock.contains("[[$it getMTerm] getSigType] eq \"CLOCK\""),
        "a clock net is one CTS marked CLOCK or one reaching a clock pin:\n{is_clock}"
    );
    assert!(!is_clock.contains("clkbuf") && !is_clock.contains("getMaster"),
        "a driver's cell never decides it: clock buffers buffer data too (1,682 such SIGNAL nets on gpu_simt):\n{is_clock}");

    let sep = &s[s
        .find("if { $separated > 0 } {")
        .expect("the separation verdict")..];
    let sep = &sep[..sep.find("\n}\n").unwrap()];
    assert!(
        sep.contains("exit 1"),
        "a diode found off its gate's net fails the step:\n{sep}"
    );
    assert!(
        block(&lib, "proc logos_fo_separated {block} {")
            .contains("diode $name is not on its gate's net"),
        "and each such diode is named"
    );
    let named = block(&lim, "proc logos_fo_named_owner {block name} {");
    assert!(
        named.contains("string last \"_\""),
        "an instance or pin name with underscores is split at every point"
    );
    assert!(
        s.contains("residual")
            && s.contains("nets over the design limit of $cap: $fo_before -> $fo_after"),
        "what is left over is counted and named, never hidden"
    );

    for (name, text) in [("the fanout step", &s), ("the split's file", &lib)] {
        let bad = unguarded_quiet_lookups(text);
        assert!(
            bad.is_empty(),
            "{name}: quiet lookups that read a missing object as present:\n{}",
            bad.join("\n")
        );
    }
    let mut choosers = 0;
    for (at, _) in lib.match_indices("[get_lib_cells ") {
        let start = lib[..at].rfind("\nproc ").map_or(0, |p| p);
        let scope = &lib[start..at];
        let window = &lib[at..lib.len().min(at + 400)];
        assert!(
            window.contains("logos_fo_usable") || scope.contains("logos_fo_usable"),
            "a library chooser that ignores DONT_USE_CELLS:\n{window}"
        );
        choosers += 1;
    }
    assert!(
        choosers >= 2,
        "the buffer chooser and the sizing loop both choose from the library ({choosers})"
    );
}

/// The post-route diode ECO adds a diode only to a net with room under the limit, counting the diodes it adds in
/// the same pass, and both of routing.tcl's ECO calls hand it the limit.
#[test]
fn the_post_route_diode_eco_stays_within_the_limit() {
    let eco = overlay("odbpy/magic_antenna_eco.py");
    assert!(
        eco.contains("@click.option(\"--max-fanout\", type=int, default=0"),
        "the ECO takes the limit"
    );
    assert!(
        eco.contains("if net_loads[nname] + 1 > max_fanout:")
            && eco.contains("refused_fanout += 1"),
        "a diode that would overflow its net is refused and counted"
    );
    assert!(
        eco.contains("net_loads[it.getNet().getConstName()] += 1"),
        "a pass counts the diodes it has added"
    );
    assert!(
        eco.contains("{refused_fanout} at the fanout limit"),
        "the refusals are reported"
    );

    let rt = overlay("tcl_commands/routing.tcl");
    assert_eq!(
        rt.matches("odbpy/magic_antenna_eco.py").count(),
        rt.matches("{*}[logos_eco_fanout_arg]").count(),
        "every ECO call passes the limit"
    );
    let arg = block(&rt, "proc logos_eco_fanout_arg {} {");
    assert!(
        arg.contains(
            "if { ![info exists ::env(LOGOS_DIODE_FANOUT)] || $::env(LOGOS_DIODE_FANOUT) == 1 } {"
        ) && arg.contains("--max-fanout")
            && arg.contains("MAX_FANOUT_CONSTRAINT"),
        "{arg}"
    );
}

/// The fanout margin: the steps that know which loads are critical build every net a margin under the SDC's limit,
/// and every report and signoff judge the limit itself. The margin is the room the post-route diode ECO needs, so the
/// ECO alone is held to the limit itself. It is never put in the SDC the resizers repair against: repair_design
/// splits a ten-load net without regard to slack, and doing so took gpu_simt's slow-corner critical path from 2 to 6
/// fanout-buffer pins and its setup slack from +0.27 to -0.01 ns (RUN_2026.10.03_07.23.27).
#[test]
fn the_margin_is_kept_by_the_steps_that_know_timing_and_signoff_judges_the_limit() {
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    let dl = block(&lim, "proc logos_design_max_fanout {} {");
    assert!(
        dl.contains("set margin 1")
            && dl.contains("$::env(LOGOS_FANOUT_MARGIN)")
            && dl.contains("int(round($::env(MAX_FANOUT_CONSTRAINT))) - $margin"),
        "the design limit is the SDC's less the margin, one load unless set:\n{dl}"
    );
    for f in [
        "openroad/common/io.tcl",
        "openroad/sta/multi_corner.tcl",
        "base.sdc",
    ] {
        assert!(!root().join("scripts/openlane-overlay/scripts").join(f).exists(),
            "{f} is overlaid: the margin must not reach the SDC the resizers repair against or the reports judge");
    }
    let cts = overlay("openroad/cts.tcl");
    assert!(
        cts.find("source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl")
            .unwrap()
            < cts
                .find("set fanout_limit [logos_design_max_fanout]")
                .unwrap(),
        "CTS reads the design limit"
    );
    let step = overlay("openroad/logos_diode_fanout.tcl");
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    assert!(
        step.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl")
            && lib.contains("source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl"),
        "the step reads the design limit, through the split's file"
    );
    for text in [&step, &lib] {
        assert!(!text.contains("sta::max_fanout_violation_count") && !text.contains("check_fanout_limits"),
            "the step judges its own residual against the design limit, never STA's count against the SDC's");
    }
    let over = block(&lim, "proc logos_fo_over {block cap} {");
    assert!(
        over.contains("[$it isInputSignal]") && over.contains("$n > $cap"),
        "every input pin counts, diodes included:\n{over}"
    );

    let rt = overlay("tcl_commands/routing.tcl");
    let arg = block(&rt, "proc logos_eco_fanout_arg {} {");
    assert!(
        arg.contains("MAX_FANOUT_CONSTRAINT") && !arg.contains("logos_design_max_fanout"),
        "the ECO is held to the signoff limit — the margin is its room:\n{arg}"
    );
}

/// A clock buffer TritonCTS leaves over the design limit is cloned, never re-buffered: the stage count to every sink
/// stays what CTS balanced. TritonCTS bounds its leaf clusters and nothing above them (four H-tree buffers drove
/// sixteen leaves each on gpu_simt at a limit of 9; cloning took all four to 9 or under with skew 0.55 -> 0.54 ns).
#[test]
fn a_clock_buffer_over_the_limit_is_cloned_after_cts() {
    let cts = overlay("openroad/cts.tcl");
    let clk = cts
        .find("repair_clock_nets -max_wire_length")
        .expect("the long-wire repair");
    let gate = cts
        .find("source $::env(SCRIPTS_DIR)/openroad/logos_clock_fanout.tcl")
        .expect("the cloning step");
    let call = cts
        .find("logos_clone_clock_drivers [logos_design_max_fanout]")
        .expect("cloning to the design limit");
    let legal = cts.find("detailed_placement").expect("legalization");
    assert!(
        clk < gate && gate < call && call < legal,
        "after the tree is complete, before it is legalized"
    );
    let gated = &cts[cts[..gate].rfind("if {").unwrap()..gate];
    assert!(
        gated.contains("![info exists ::env(LOGOS_CTS_FANOUT)] || $::env(LOGOS_CTS_FANOUT) == 1"),
        "{gated}"
    );

    let ck = overlay("openroad/logos_clock_fanout.tcl");
    let p = block(&ck, "proc logos_clone_clock_drivers {cap} {");
    assert!(
        p.contains("[$net getSigType] ne \"CLOCK\""),
        "it works on the nets CTS marked CLOCK:\n{p}"
    );
    assert!(
        p.contains("odb::dbInst_create $block $master"),
        "a clone is the same cell"
    );
    assert!(
        p.contains("$cit connect [$it getNet]"),
        "with every input on the original's net"
    );
    assert!(
        p.contains("$cnet setSigType CLOCK"),
        "its output is a clock net"
    );
    assert!(
        p.contains("$c setLocation [lindex $loc 0] [lindex $loc 1]"),
        "placed at the original, so its input wire is the same"
    );
    assert!(
        p.contains("logos_fo::bins $units $cap"),
        "the sinks are shared in compact bins under the limit"
    );
    assert!(
        p.contains("if { $changed == 0 } { break }"),
        "rounds run until no clock driver is over the limit"
    );
    assert!(
        p.contains("lappend residual"),
        "a port-driven net it cannot clone is reported, never hidden"
    );
    assert!(
        !p.contains("buf_") && !p.contains("clkbuf"),
        "no cell is named: a clone is whatever CTS placed"
    );
}

/// The post-diode upsize runs after the fanout step and must not undo it: its single-net repair splits a net by
/// pins, so every net carrying a diode is held while its rounds run, and a maximal driver on such a net is isolated
/// instead, which leaves every sink and diode where it is.
#[test]
fn the_post_diode_upsize_never_parts_a_diode_from_its_gate() {
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let holding = block(&up, "proc logos_hold_dioded {} {");
    assert!(
        holding.contains(
            "                $dbnet setDoNotTouch 1\n                lappend held $dbnet\n"
        ),
        "dioded nets are held:\n{holding}"
    );
    let hold = up
        .find("\nlogos_hold_dioded\n")
        .expect("held before the rounds");
    let rounds = up.find("\nlogos_slew_rounds\n").expect("the rounds");
    let release = up
        .find("\nforeach dbnet $held { $dbnet setDoNotTouch 0 }\nset residual_nets [logos_slew_violating_nets]")
        .expect("and released just before the residual is measured");
    assert!(
        hold < rounds && rounds < release,
        "held for every round, released before the residual is measured"
    );
    assert!(
        up.contains("dict set logos_held [$dbnet getName] 1"),
        "the step knows which nets it holds"
    );

    // The single-net repair rewires whatever net it is handed and ODB refuses that on a held net (ODB-0372 on
    // fabric_soc net218, RUN_2026.10.03_14.06.24): a dont_touch net never reaches it.
    let rounds_at = up.find("for {set round 1}").expect("the rounds");
    let repair = rounds_at
        + up[rounds_at..]
            .find("rsz::repair_net_cmd $net 0 0 0")
            .expect("the single-net repair");
    let guard_text = "if { ![[sta::sta_to_db_net $net] isDoNotTouch] } {";
    let guard = up[..repair]
        .rfind(guard_text)
        .expect("the repair is guarded by the hold");
    assert!(
        !up[guard + guard_text.len()..repair].contains('}'),
        "the guard encloses the repair"
    );

    // Isolation leaves every sink and diode on the net, so it may rewire a net this step holds — released for the
    // rewire and held again after it — and never one the design marked dont_touch.
    let iso = block(&up, "proc logos_isolate_driver {net drv} {");
    let ours = iso
        .find("set ours [dict exists $logos_held [$dbnet getName]]")
        .expect("is the hold ours");
    let refuse = iso
        .find("if { [$dbnet isDoNotTouch] && !$ours } { return 0 }")
        .expect("a design dont_touch is refused");
    let release = iso
        .find("if { $ours } { $dbnet setDoNotTouch 0 }")
        .expect("our hold is released for the rewire");
    let cut = iso.find("disconnect_pin $net $drv").expect("the rewire");
    let rejoin = iso
        .find("connect_pin $net $bname/X")
        .expect("the buffer drives the net");
    let rehold = iso
        .find("if { $ours } { $dbnet setDoNotTouch 1 }")
        .expect("and the hold is restored");
    assert!(
        ours < refuse && refuse < release && release < cut && cut < rejoin && rejoin < rehold,
        "{iso}"
    );

    // Isolation lends a driver the library's strongest buffer; a buffer already that strong gains nothing but a
    // stage. MEASURED on systolic_os_pipe8_realbist (RUN_2026.10.03_15.52.08): its slow-corner worst path after the
    // fanout step ran through five buf_12 isolations of buf_12 drivers, each adding ~0.4 ns to an unchanged net.
    let refuse_up = "if { [regexp {__(?:buf|clkbuf|bufbuf)_([0-9]+)$} $ref -> d] && [regexp {_([0-9]+)$} $buf -> top] && $d >= $top } {\n        return 0\n    }";
    assert!(
        iso.find(refuse_up).unwrap_or(usize::MAX) < iso.find("make_instance $bname $buf").unwrap(),
        "the upsize never isolates a buffer already as strong as the one it would add:\n{iso}"
    );
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    let fiso = block(&lib, "proc logos_fo_isolate_driver {block db drv} {");
    let refuse_fo = "if { [regexp {__(?:buf|clkbuf|bufbuf)_([0-9]+)$} $ref -> d] && $d >= [lindex $bufs end 0] } { return 0 }";
    assert!(
        fiso.find(refuse_fo).unwrap_or(usize::MAX) < fiso.find("odb::dbInst_create").unwrap(),
        "the fanout step never isolates a buffer already as strong as the one it would add:\n{fiso}"
    );
    assert!(
        fiso.find("if { [[$drv getNet] isDoNotTouch] } { return 0 }")
            .unwrap_or(usize::MAX)
            < fiso.find("odb::dbInst_create").unwrap(),
        "the fanout step's isolation leaves a dont_touch net as it is:\n{fiso}"
    );
}

/// The re-route after global routing's antenna repair holds max slew on the route detailed routing will follow: it
/// is the post-diode upsize in route mode, measuring that route, sizing or isolating the drivers that violate on it,
/// and re-routing incrementally so a net it leaves keeps the route it was judged on. MEASURED on soft_core_realbist
/// (RUN_2026.10.03_20.04.57): the plain re-route took net _05405_ from 179 um of met3 to 248 um of met3 and 331 um of
/// met4, nothing measured it again, and it signed off at 0.93 ns against 0.75; the route mode isolated its max-drive
/// driver and re-routed it in two incremental rounds.
#[test]
fn the_final_re_route_holds_max_slew_on_its_own_route() {
    let rt = overlay("tcl_commands/routing.tcl");
    assert!(
        rt.contains("        if { [info exists ::env(LOGOS_POST_DIODE_UPSIZE)] && $::env(LOGOS_POST_DIODE_UPSIZE) == 1 } {\n            run_logos_route_slew\n        } else {\n            set _logos_gra $::env(GRT_REPAIR_ANTENNAS)\n            set ::env(GRT_REPAIR_ANTENNAS) 0\n            global_routing\n            set ::env(GRT_REPAIR_ANTENNAS) $_logos_gra\n        }"),
        "the re-route is the slew-holding one wherever the recipe runs the post-diode upsize, and it sees the \
         recipe's antenna repair, which it runs on its own route; only the plain re-route turns the repair off"
    );
    let p = block(&rt, "proc run_logos_route_slew {args} {");
    let on = p
        .find("set ::env(LOGOS_UPSIZE_ROUTE) 1")
        .expect("route mode on");
    let run = p
        .find("openroad/logos_post_diode_upsize.tcl")
        .expect("the upsize runs it");
    let off = p
        .find("unset ::env(LOGOS_UPSIZE_ROUTE)")
        .expect("and route mode off after");
    assert!(on < run && run < off, "{p}");
    assert!(
        p.contains("guide=[index_file $::env(routing_tmpfiles)/global.guide]")
            && p.contains("set_guide $::env(SAVE_GUIDE)"),
        "the route it holds is the one detailed routing follows:\n{p}"
    );
    assert!(
        p.contains("write_verilog"),
        "its netlist is written again, since it may change cells"
    );

    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let m = block(&up, "proc logos_measure {} {");
    assert!(
        m.contains("set_macro_extension $::env(GRT_MACRO_EXTENSION)")
            && m.contains("-congestion_report_file $::env(GRT_CONGESTION_REPORT_FILE)")
            && m.contains(
                "if { $::env(GRT_ALLOW_CONGESTION) == 1 } { lappend grt -allow_congestion }"
            ),
        "its first measure is the flow's own global route:\n{m}"
    );
    assert!(
        m.contains("    if { $logos_routed } {\n        global_route -end_incremental\n")
            && m.ends_with("\n    global_route -start_incremental")
            && !m.contains("if { $logos_route } { global_route -start_incremental }")
            && m.matches("set logos_routed 1").count() == 2,
        "every run's later measures re-route only what an edit touched, in route mode or not (coproc_core, the \
         post-diode upsize replayed: re-routing everything each round reached the 24-round bound with 6 nets left \
         and +262 cells in 228 s; incrementally 6 rounds, 2 nets, +123 cells in 49 s):\n{m}"
    );
    let fin = up
        .find("\nglobal_route -end_incremental\nif { [info exists doomed] } { foreach inst $doomed { odb::dbInst_destroy $inst } }\nunset_dont_touch_wrapper")
        .expect("the last edits are routed, and only then the diodes a split shed are destroyed");
    let legal = up[..fin].rfind("detailed_placement").unwrap();
    assert!(
        legal < fin && fin < up.rfind("\nwrite").unwrap(),
        "after the closing legalization, before the views are written"
    );
}

/// The diode rules both diode-aware steps apply live in one file, so the fanout step and the post-diode upsize cannot
/// disagree about which pin a diode guards, how a net's loads are counted, which diode an edit left behind, or when a
/// new input is guarded.
#[test]
fn the_diode_rules_have_one_home() {
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    let fo = overlay("openroad/logos_diode_fanout.tcl");
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    for p in [
        "proc logos_fo_pin_xy {it} {",
        "proc logos_fo_is_diode {it} {",
        "proc logos_fo_is_clock {net} {",
        "proc logos_fo_named_owner {block name} {",
        "proc logos_fo_over {block cap} {",
        "proc logos_fo_net_span {block net} {",
        "proc logos_fo_guard_input {block db inst} {",
        "proc logos_fo_owners {block net} {",
        "proc logos_fo_stranded {block} {",
    ] {
        assert_eq!(
            lim.matches(p).count(),
            1,
            "{p} is defined once, in logos_fanout_limit.tcl"
        );
        assert!(
            !fo.contains(p) && !up.contains(p) && !lib.contains(p),
            "{p} is defined nowhere else"
        );
    }
    for (name, text) in [("the split's file", &lib), ("the post-diode upsize", &up)] {
        assert!(
            text.contains("source $::env(SCRIPTS_DIR)/openroad/logos_fanout_limit.tcl"),
            "{name} reads the shared rules"
        );
    }
    assert!(
        fo.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl"),
        "the fanout step reads them through the split's file"
    );
}

/// A net held for its diodes whose driver is already the strongest buffer has two remedies, tried in this order once
/// growing and isolation have nothing to add: the resizer's own single-net repair with every diode put back on its
/// gate's net, then halving the longest arm, a buffer at the midpoint toward the farthest sink with every gate nearer that
/// midpoint than the driver moved behind it. Both keep each diode with the pin it was placed beside, every net carrying a
/// diode is held again, and the step fails if a diode it can name left its gate's net, an edit left a diode behind, or a
/// net went over the design limit. MEASURED on
/// coproc_core (RUN_2026.10.03_23.19.59, the route-mode step replayed): 5 residual nets, all held, driven by buf_12 and
/// routed 1.76-2.15 mm, signed off at 0.76-0.86 ns; with these remedies 0 residual nets and +13 cells. A repeater toward
/// the sinks' centroid stalls once the driver is among them, and a split with buffers at the halves' centroids left a far
/// arm of 1,911 um routed (FO4PROBE2_20261004_0106).
#[test]
fn a_held_wire_at_the_strongest_drive_is_rebuffered_or_its_longest_arm_halved() {
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let rounds = &up[up.find("for {set round 1}").expect("the rounds")..];
    let rounds = &rounds[..rounds.find("\n}\n").expect("their end")];
    let iso = rounds
        .find("[logos_isolate_driver $net [lindex $drvs 0]]")
        .expect("isolation");
    let reb = rounds
        .find("[set added [logos_rebuffer_held $net]] > 0")
        .expect("the rebuffer");
    let arm = rounds
        .find("[logos_halve_arm $net [lindex $drvs 0]]")
        .expect("the arm halving");
    assert!(
        iso < reb && reb < arm,
        "isolate, then rebuffer, then halve the longest arm:\n{rounds}"
    );

    let lim = overlay("openroad/logos_fanout_limit.tcl");
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    let own = block(&lim, "proc logos_fo_owners {block net} {");
    let named = own
        .find("logos_fo_named_owner $block $name")
        .expect("a diode's name names its gate");
    let near = own
        .find("[logos_fo::pair $rows $drows]")
        .expect("else the nearest pin");
    assert!(
        named < near,
        "the name decides before distance does:\n{own}"
    );

    let rb = block(&up, "proc logos_rebuffer_held {net} {");
    let owners = rb
        .find("set owner [logos_fo_owners $block $dbnet]")
        .expect("owners recorded");
    let free = rb
        .find("$dbnet setDoNotTouch 0")
        .expect("our hold released");
    let repair = rb
        .find("rsz::repair_net_cmd $net 0 0 0")
        .expect("the resizer's repair");
    let back = rb
        .find("$d disconnect\n            $d connect $gnet")
        .expect("a parted diode goes back");
    let rehold = rb
        .find("$n setDoNotTouch 1")
        .expect("dioded nets held again");
    assert!(
        owners < free && free < repair && repair < back && back < rehold,
        "{rb}"
    );
    assert!(
        rb.contains("if { ![dict exists $logos_held [$dbnet getName]] } { return 0 }"),
        "only a net this step holds, never one the design marked dont_touch:\n{rb}"
    );

    let ha = block(&up, "proc logos_halve_arm {net drv} {");
    let farthest = ha
        .find("if { $d > $fd } {")
        .expect("the farthest sink sets the arm");
    let floor = ha
        .find("if { $fd < 50 * [$block getDbUnitsPerMicron] } { return 0 }")
        .expect("an arm too short to matter is left");
    let mid = ha
        .find("set mx [expr { ($dx + $fx) / 2 }]")
        .expect("the midpoint toward it");
    let side = ha
        .find("if { abs($x - $mx) + abs($y - $my) < abs($x - $dx) + abs($y - $dy) } { lappend moved $g }")
        .expect("the gates nearer the midpoint than the driver move");
    let made = ha.find("make_instance $bname $buf").expect("the buffer");
    assert!(
        farthest < floor && floor < mid && mid < side && side < made,
        "{ha}"
    );
    assert!(
        ha.contains("$bi setLocation $mx $my")
            && ha.contains("set owner [logos_fo_owners $block $dbnet]")
            && ha.contains("$d disconnect\n                $d connect $subdb"),
        "the buffer sits at the midpoint and each moved gate takes the diodes placed beside it:\n{ha}"
    );
    assert!(
        ha.contains("[logos_fo_net_span $block $dbnet] >= $::env(HEURISTIC_ANTENNA_THRESHOLD)")
            && ha.contains("logos_fo_guard_input $block [ord::get_db] $bi")
            && ha.contains("$n setDoNotTouch 1"),
        "the buffer's input is guarded as the heuristic pass guards a net that long, and every net carrying a diode \
         is held:\n{ha}"
    );

    let lim_on = up
        .find("set_max_fanout [logos_design_max_fanout] [current_design]")
        .expect("trees to the design limit");
    let lim_off = up
        .find("set_max_fanout $::env(MAX_FANOUT_CONSTRAINT) [current_design]")
        .expect("the SDC's limit back");
    let round1 = up.find("for {set round 1}").unwrap();
    let write = up.rfind("\nwrite").unwrap();
    assert!(
        lim_on < round1 && round1 < lim_off && lim_off < write,
        "the repair builds to the design limit, and the SDC written out keeps the signoff limit"
    );

    let before = up
        .find("set separated_before [logos_fo_separated [ord::get_db_block]]")
        .expect("counted before");
    let over_before = up
        .find("set over_before [logos_signal_over [ord::get_db_block]]")
        .expect("counted before");
    let verdict = up
        .find("if { $separated > $separated_before || $stranded > $stranded_before || $over > $over_before } {")
        .expect("compared after");
    let fail = verdict + up[verdict..].find("exit 1").expect("and the step fails");
    assert!(
        before < round1 && over_before < round1 && round1 < verdict && fail < write,
        "the step fails, before writing, if it parted a diode it can name or put a net over the design limit"
    );
    let sep = block(&lib, "proc logos_fo_separated {block} {");
    assert!(
        sep.contains("set o [logos_fo_named_owner $block $name]")
            && sep.contains("[[lindex $o 1] getNet] ne [$dt getNet]"),
        "a diode is judged against the gate its name gives, by the fanout step's own check:\n{sep}"
    );
    let ov = block(&up, "proc logos_signal_over {block} {");
    assert!(
        ov.contains("logos_fo_over $block [logos_design_max_fanout]")
            && ov.contains("if { ![lindex $o 1] } { incr n }"),
        "signal nets over the design limit, as the fanout step counts them:\n{ov}"
    );
}

/// The positions of `needles` in `text`, each searched from the one before it; panics naming the first one missing.
fn in_order(text: &str, needles: &[&str]) -> Vec<usize> {
    let mut at = 0;
    let mut out = Vec::new();
    for n in needles {
        let i = at
            + text[at..]
                .find(n)
                .unwrap_or_else(|| panic!("{n:?} is missing, or out of order:\n{text}"));
        out.push(i);
        at = i + n.len();
    }
    out
}

/// The fanout step and the post-diode upsize's antenna closure split a net by one set of rules: the split, the
/// selection and slack it reads first, the sizing after it, the held timing repair and the separation check live in
/// logos_diode_fanout_lib.tcl, and both steps call them. A split sheds diodes by disconnecting them and handing them to
/// its caller, which destroys them only where no incremental global route is open: the router faults on an instance
/// destroyed during its session (GlobalRouter::updateDirtyNets, SIGSEGV on fabric_soc).
#[test]
fn the_split_has_one_home_and_both_steps_call_it() {
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    let fo = overlay("openroad/logos_diode_fanout.tcl");
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    for p in [
        "proc logos_fo_usable {name} {",
        "proc logos_fo_obj {o} {",
        "proc logos_fo_buffers {ref} {",
        "proc logos_fo_net_loads {net} {",
        "proc logos_fo_slack {path} {",
        "proc logos_fo_isolate_driver {block db drv} {",
        "proc logos_fo_move_guard {block inst net} {",
        "proc logos_fo_fresh_name {block stem} {",
        "proc logos_fo_return_named {block} {",
        "proc logos_fo_candidates {block cap} {",
        "proc logos_fo_split_net {block db net cap pin_slack heuristic statsvar} {",
        "proc logos_fo_size_touched {block db touched measure} {",
        "proc logos_fo_held_timing_repair {block measure} {",
        "proc logos_fo_separated {block} {",
    ] {
        assert_eq!(
            lib.matches(p).count(),
            1,
            "{p} is defined once, in logos_diode_fanout_lib.tcl"
        );
        assert!(
            !fo.contains(p) && !up.contains(p) && !lim.contains(p),
            "{p} is defined nowhere else"
        );
    }
    for (name, text) in [("the fanout step", &fo), ("the post-diode upsize", &up)] {
        assert!(
            text.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_lib.tcl"),
            "{name} reads the split's file"
        );
    }
    for (name, text, measure, stats) in [
        ("the fanout step", &fo, "logos_fo_measure", "st"),
        ("the antenna closure", &up, "logos_measure", "fst"),
    ] {
        in_order(
            text,
            &[
                "lassign [logos_fo_candidates $block $cap] candidates pin_slack ",
                &format!("logos_fo_split_net $block $db $net $cap $pin_slack $heuristic {stats}"),
                &format!("logos_fo_size_touched $block $db [dict get ${stats} touched] {measure}"),
                &format!("logos_fo_held_timing_repair $block {measure}"),
            ],
        );
        assert!(
            !text.contains("logos_fo::bins") && !text.contains("logos_fo::kept_bin"),
            "{name} plans no split of its own"
        );
    }

    let lp = block(
        &lib,
        "proc logos_fo_split_net {block db net cap pin_slack heuristic statsvar} {",
    );
    assert!(
        !lp.contains("dbInst_destroy") && lp.matches("dict lappend st doomed").count() == 3,
        "the split destroys nothing: what it sheds from a heavy gate, the driver or a clock net goes to its caller:\n{lp}"
    );
    let clock = &lp[lp.find("if { [logos_fo_is_clock $net] } {").unwrap()..];
    assert!(
        clock.find("[dict get $diode_it $did] disconnect").unwrap()
            < clock.find("dict lappend st doomed").unwrap(),
        "a diode a clock net sheds leaves the net before it is handed on"
    );
    in_order(
        &fo,
        &[
            "foreach net $candidates {\n    logos_fo_split_net $block $db $net $cap $pin_slack $heuristic st\n}",
            "foreach inst [dict get $st doomed] { odb::dbInst_destroy $inst }",
            "lassign [logos_fo_size_touched $block $db [dict get $st touched] logos_fo_measure] grown isolated",
        ],
    );
}

/// Global routing's repair puts a diode beside every pin of the wire component it judges — the driver and the diodes
/// already there among them (ant::AntennaChecker::findWireRootIterms) — so a diode belongs to the nearest pin of its
/// net, the driver included, and every edit keeps it on that pin's net: a diode beside the driver stays on the
/// driver's net, a load each bin left there must fit beside, and a clock net counts its driver's diodes among those it
/// may shed. MEASURED on coproc_core (RUN_2026.10.03_23.19.59): paired with gates alone, the two diodes beside the
/// driver of _28909_ followed its one gate through five halvings and hung 650 um from every pin of logos_arm_net_11,
/// routed 2,084 um, the closure's one slew residual; the fanout step's split had left six more 219-275 um out on its
/// bins.
#[test]
fn a_diode_belongs_to_the_pin_it_was_placed_beside() {
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    let lib = overlay("openroad/logos_diode_fanout_lib.tcl");
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let fo = overlay("openroad/logos_diode_fanout.tcl");
    let core = overlay("openroad/logos_diode_fanout_core.tcl");
    let own = block(&lim, "proc logos_fo_owners {block net} {");
    assert!(
        own.contains("} elseif { [$it isInputSignal] || [$it isOutputSignal] } {")
            && own.contains("foreach bt [$net getBTerms] {")
            && own.contains("dict set pin_of $i $bt"),
        "every pin of the net may own a diode, its driver and its port among them:\n{own}"
    );
    assert!(
        lim.contains("source $::env(SCRIPTS_DIR)/openroad/logos_diode_fanout_core.tcl"),
        "the rule pairs with the tested core"
    );
    assert!(
        core.contains("ant::AntennaChecker::findWireRootIterms"),
        "the core names where the repair puts its diodes"
    );

    let lp = block(
        &lib,
        "proc logos_fo_split_net {block db net cap pin_slack heuristic statsvar} {",
    );
    assert!(
        lp.contains("set owner [logos_fo_owners $block $net]") && !lp.contains("logos_fo::pair"),
        "the split pairs by the shared rule:\n{lp}"
    );
    let clock = lp
        .find("if { [logos_fo_is_clock $net] } {")
        .expect("the clock arm");
    let root_unit = lp
        .find("lappend units [list root $sx $sy $ds]")
        .expect("a clock driver's diodes are a unit at the driver");
    let trim = lp
        .find("set drop [logos_fo::budget_trim $units $sx $sy $loads $cap]")
        .expect("the clock trim");
    assert!(clock < root_unit && root_unit < trim, "{lp}");
    in_order(
        lp,
        &[
            "set room [expr { $cap - $per_buffer }]",
            "lassign [logos_fo::trim_unit $rooted $sx $sy [expr { $room + 1 }]] root_keep dropped",
            "set fixed [llength $root_keep]",
            "set kept_bin [logos_fo::kept_bin $bins $units $slack $sx $sy $cap $per_buffer $fixed]",
            "lassign [logos_fo::carve $units $slack $sx $sy $cap $per_buffer $fixed] carved rest_bins",
            "foreach did $root_keep { [dict get $diode_it $did] connect $net }",
            "while { $parent_w > $cap",
        ],
    );
    let kb = block(
        &core,
        "proc logos_fo::kept_bin {bins units slack sx sy cap {per_buffer 1} {fixed 0}} {",
    );
    assert!(
        kb.contains("if { $w + ($nb - 1) * $per_buffer + $fixed <= $cap } {"),
        "a bin stays only with room beside the diodes that stay:\n{kb}"
    );
    let cv = block(
        &core,
        "proc logos_fo::carve {units slack sx sy cap {per_buffer 1} {fixed 0}} {",
    );
    assert!(
        cv.contains(
            "if { $w + [lindex $u 3] + $per_buffer * [llength $tb] + $fixed > $cap } { break }"
        ),
        "and the carve leaves the same room:\n{cv}"
    );

    for start in [
        "proc logos_halve_arm {net drv} {",
        "proc logos_rebuffer_held {net} {",
    ] {
        assert!(
            block(&up, start).contains("set owner [logos_fo_owners $block $dbnet]"),
            "{start} moves each diode with the pin it was placed beside"
        );
    }
    for (name, text) in [
        ("the upsize", &up),
        ("the split's file", &lib),
        ("the shared rules", &lim),
        ("the fanout step", &fo),
    ] {
        assert!(
            !text.contains("proc logos_diode_owners"),
            "{name} keeps no rule of its own that pairs a diode with gates alone"
        );
    }
}

/// A diode an edit leaves far from every pin of its net protects nothing it was placed for and lengthens the net's
/// route: after global routing's repair, 18,626 of the 18,632 diodes on coproc_core and fabric_soc lay within 37 um of
/// their nearest pin, and the other six 219-275 um out, left there by the fanout step's split. Both diode-aware steps
/// count the diodes more than 100 um from every pin before their edits and after, and fail, naming each, if the count
/// grew.
#[test]
fn no_edit_leaves_a_diode_behind() {
    let lim = overlay("openroad/logos_fanout_limit.tcl");
    let fo = overlay("openroad/logos_diode_fanout.tcl");
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let st = block(&lim, "proc logos_fo_stranded {block} {");
    assert!(
        st.contains("set lim [expr { 100 * [$block getDbUnitsPerMicron] }]")
            && st.contains("if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend pins [logos_fo_pin_xy $it] }")
            && st.contains("foreach bt [$net getBTerms] {")
            && st.contains("if { $near < 0 || $near > $lim } {"),
        "a diode is left behind when no other pin of its net, driver and port included, is within 100 um:\n{st}"
    );
    let fp = in_order(
        &fo,
        &[
            "set repaired [logos_fo_return_named $block]",
            "set stranded_before [llength [logos_fo_stranded $block]]",
            "lassign [logos_fo_candidates $block $cap] candidates pin_slack skipped_dont_touch",
            "set stranded_after [logos_fo_stranded $block]",
            "if { $stranded > $stranded_before } {",
            "\nwrite",
        ],
    );
    let verdict = &fo[fp[4]..fp[5]];
    assert!(
        verdict.contains("$dname on $nname, $um um from its nearest pin")
            && verdict.contains("exit 1"),
        "the fanout step names each diode it left behind and fails:\n{verdict}"
    );
    let up_p = in_order(
        &up,
        &[
            "set stranded_before [llength [logos_fo_stranded [ord::get_db_block]]]",
            "\nlogos_slew_rounds\n",
            "set stranded_after [logos_fo_stranded [ord::get_db_block]]",
            "if { $separated > $separated_before || $stranded > $stranded_before || $over > $over_before } {",
            "\nwrite",
        ],
    );
    let verdict = &up[up_p[3]..up_p[4]];
    assert!(
        verdict.contains("$dname on $nname, $um um from its nearest pin")
            && verdict.contains("exit 1"),
        "the upsize names each diode an edit left behind and fails:\n{verdict}"
    );
}

/// A legalization clears an overlap by moving cells to the nearest free sites, and in a crowded region it carries a diode
/// and the gate beside it apart a few tens of microns at a time until the diode is past every pin of its net, though no
/// edit moved either. Setting a parted diode back on its pin cannot undo that where no site near the pin is free, and
/// holding the diode alone leaves its gate to drift. So every legalization after an edit holds FIRM each diode, and the
/// instance whose pin is its net's nearest to it, as the last legalization left them — placed there, unmoved, the same
/// cell — and the legalizer moves the edit's cells around them; the held cells are released and the pairs taken afresh
/// after it. MEASURED on wpe_user_project at 18.5 ns, step 24 replayed from RUN_2026.10.22_07.32.54's step 23: held as
/// pairs, no diode was left behind and the rounds converged in 11 with no net over the slew limit; diodes held alone
/// left 3 behind by round 13, and re-homing them left 86.
#[test]
fn every_legalization_after_an_edit_holds_each_diode_and_its_nearest_pin_where_they_are() {
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let settle = block(&up, "proc logos_diodes_settle {} {");
    in_order(
        settle,
        &[
            "set logos_settled [dict create]",
            "set logos_partners [dict create]",
            "if { [[$inst getMaster] getConstName] ne $::env(DIODE_CELL) } { continue }",
            "dict set logos_settled [$inst getConstName] [$inst getLocation]",
            "if { [logos_fo_is_diode $it] } { lappend diodes $it } else { lappend pins $it }",
            "if { $near < 0 || $dd < $near } {",
            "set best [$it getInst]",
            "dict set logos_partners [$best getConstName] [list [$best getLocation] [[$best getMaster] getConstName]]",
        ],
    );
    let hold = block(&up, "proc logos_diodes_hold {} {");
    in_order(
        hold,
        &[
            "set logos_held [list]",
            "if { [$inst getPlacementStatus] ne \"PLACED\" } { continue }",
            "if { ![dict exists $logos_settled $name] } { continue }",
            "if { [dict get $logos_settled $name] ne [$inst getLocation] } { continue }",
            "} elseif { [dict exists $logos_partners $name] } {",
            "if { $loc ne [$inst getLocation] || $master ne [[$inst getMaster] getConstName] } { continue }",
            "$inst setPlacementStatus FIRM",
            "lappend logos_held $inst",
        ],
    );
    let release = block(&up, "proc logos_diodes_release {} {");
    in_order(
        release,
        &[
            "foreach inst $logos_held { $inst setPlacementStatus PLACED }",
            "set logos_held [list]",
            "logos_diodes_settle",
        ],
    );

    let grid = "source $::env(SCRIPTS_DIR)/openroad/common/dpl_cell_pad.tcl\ndetailed_placement\n";
    let after_grid = &up[up.find(grid).expect("the grid-building legalization") + grid.len()..];
    let first_settle = after_grid
        .find("\nlogos_diodes_settle\n")
        .expect("the pairs the grid build left are taken");
    let first_rounds = after_grid.find("\nlogos_slew_rounds\n").expect("the slew rounds");
    assert!(
        first_settle < first_rounds,
        "the pairs are taken before the first round edits anything"
    );

    let mut rest = after_grid.to_string();
    for proc_start in [
        "proc logos_diodes_settle {} {",
        "proc logos_diodes_hold {} {",
        "proc logos_diodes_release {} {",
    ] {
        let body = block(&rest, proc_start).to_string();
        rest = rest.replacen(&body, "", 1);
    }
    let lines: Vec<&str> = rest.lines().map(str::trim).collect();
    let legalizations: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with("detailed_placement"))
        .collect();
    let loose: Vec<usize> = legalizations
        .iter()
        .copied()
        .filter(|&i| {
            i == 0
                || lines[i - 1] != "logos_diodes_hold"
                || lines.get(i + 1) != Some(&"logos_diodes_release")
        })
        .collect();
    assert!(
        loose.is_empty(),
        "a legalization after an edit carries settled diodes and their pins apart: {:?}",
        loose.iter().map(|&i| lines[i]).collect::<Vec<_>>()
    );
    assert_eq!(
        legalizations.len(),
        3,
        "each round, each antenna pass and the closing legalization hold the pairs"
    );
}

/// The route-mode step hands on the route detailed routing follows, so that route is judged for antennas there, while
/// placement can still make room for a diode: global routing's own repair judged a route this step replaces. Each pass
/// closes the router's session, destroys what the last pass's splits shed, renames the repair's diodes so its next
/// call can name its own, releases this step's holds (the repair connects to the nets it judges, ODB-0373 on a held
/// one), repairs with the recipe's iterations and margin, splits the nets its diodes took over the design limit by the
/// fanout step's rules, sizes and repairs timing as that step does, and runs the slew rounds again. A pass that adds no
/// diode ends it, and so does one whose splits shed every diode it added — the next would add the same. MEASURED on
/// coproc_core (FO4PROBE2_20261004_0106): the route it handed on carried 178 violations at the recipe's 50% margin, and
/// the post-route ECO found no site for a diode on wire394/A in 1,507 tries; with the closure every coproc replay of
/// the step reached that fixed point by its third pass with no net over the slew limit.
#[test]
fn the_route_detailed_routing_follows_is_judged_for_antennas() {
    let up = overlay("openroad/logos_post_diode_upsize.tcl");
    let gate = up
        .find("if { $logos_route && $::env(GRT_REPAIR_ANTENNAS) == 1 } {")
        .expect("the closure runs in route mode wherever the recipe repairs antennas");
    let first_rounds = up.find("\nlogos_slew_rounds\n").expect("the slew rounds");
    assert!(
        first_rounds < gate,
        "after the slew rounds have sized the route"
    );
    let cl = block(
        &up,
        "if { $logos_route && $::env(GRT_REPAIR_ANTENNAS) == 1 } {",
    );
    in_order(
        cl,
        &[
            "set_placement_padding -masters $::env(DIODE_CELL) -left $::env(DIODE_PADDING)",
            "for {set apass 1} {$apass <= 3} {incr apass} {",
            "global_route -end_incremental\n        foreach inst $doomed { odb::dbInst_destroy $inst }",
            "regexp {^ANTENNA_([0-9]+)$} [$i getConstName] -> k",
            "$i rename \"ANTENNA_G[expr { $apass - 1 }]_$k\"",
            "foreach dbnet $held { $dbnet setDoNotTouch 0 }",
            "set d0 [logos_diode_count $block]",
            "repair_antennas $::env(DIODE_CELL) -iterations $::env(GRT_ANT_ITERS) -ratio_margin $::env(GRT_ANT_MARGIN)",
            "estimate_parasitics -global_routing",
            "global_route -start_incremental",
            "if { $added == 0 } {\n            logos_hold_dioded\n            break\n        }",
            "lassign [logos_fo_candidates $block $cap] candidates pin_slack skipped",
            "logos_fo_split_net $block $db $net $cap $pin_slack $heuristic fst",
            "set doomed [dict get $fst doomed]",
            "if { [logos_diode_count $block] <= $d0 && [dict get $fst buffers] == 0 } {",
            "logos_hold_dioded\n            break\n        }",
            "logos_fo_size_touched $block $db [dict get $fst touched] logos_measure",
            "detailed_placement",
            "logos_fo_held_timing_repair $block logos_measure",
            "logos_hold_dioded",
            "logos_slew_rounds",
        ],
    );
    assert_eq!(
        up.matches("odb::dbInst_destroy").count(),
        2,
        "a shed diode is destroyed only where the router's session is closed: at a pass's start, and after the last \
         edit is routed"
    );
    assert!(
        up.contains("\nglobal_route -end_incremental\nif { [info exists doomed] } { foreach inst $doomed { odb::dbInst_destroy $inst } }\n"),
        "the last pass's shed diodes are destroyed once the last edits are routed"
    );
    let dc = block(&up, "proc logos_diode_count {block} {");
    assert!(
        dc.contains("if { $dt ne \"NULL\" && [$dt getNet] ne \"NULL\" } { incr n }"),
        "a pass counts the diodes on a net, not the shed ones waiting to be destroyed:\n{dc}"
    );
    assert!(
        up.contains("diode(s) kept from $antenna_passes antenna pass(es)"),
        "the diodes the closure kept, and its passes, are reported"
    );
    let rt = overlay("tcl_commands/routing.tcl");
    let p = block(&rt, "proc run_logos_route_slew {args} {");
    assert!(
        !p.contains("GRT_REPAIR_ANTENNAS"),
        "the route-mode step sees the recipe's antenna repair:\n{p}"
    );
}

/// Runs `run_routing` from the overlay's routing.tcl under tclsh with every step it calls replaced by a recorder, for
/// one recipe (`GATE=value` pairs; a gate left out is unset), and returns the steps in the order they ran.
fn routing_steps(recipe: &[(&str, &str)]) -> Vec<String> {
    const TRACE: &str = r#"
set routing [lindex $argv 0]
source $routing
set ::trace [list]
foreach step {
    puts_verbose puts_info increment_index check_wire_lengths scrot_klayout
    io_diode_insertion heuristic_diode_insertion targeted_diode_insertion
    run_resizer_design_routing run_resizer_timing_routing
    run_logos_diode_fanout run_logos_post_diode_upsize run_logos_route_slew
    add_route_obs global_routing ins_fill_cells detailed_routing run_antenna_eco
} {
    proc $step {args} "lappend ::trace $step"
}
foreach kv [lrange $argv 1 end] {
    lassign [split $kv =] k v
    set ::env($k) $v
}
foreach {k v} {DIODE_CELL sky130_fd_sc_hd__diode_2 DIODE_ON_PORTS none RUN_HEURISTIC_DIODE_INSERTION 1
               RUN_FILL_INSERTION 1 RUN_DRT 1 CURRENT_DEF probe.def routing_logs logs} {
    set ::env($k) $v
}
run_routing
puts "STEPS [lsearch -all -inline -not -regexp $::trace {^(puts_|increment_index|check_wire_lengths|scrot_klayout)}]"
"#;
    let dir = tempfile::tempdir().expect("a scratch directory");
    let script = dir.path().join("trace_routing.tcl");
    std::fs::write(&script, TRACE).unwrap();
    let mut cmd = std::process::Command::new("tclsh");
    cmd.arg(&script)
        .arg(root().join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"));
    for (k, v) in recipe {
        cmd.arg(format!("{k}={v}"));
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("LOGOS_") || k.starts_with("GRT_") || k.starts_with("RUN_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd.output().expect("run tclsh");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "run_routing under {recipe:?}:\n{log}");
    log.lines()
        .find_map(|l| l.strip_prefix("STEPS "))
        .unwrap_or_else(|| panic!("no step trace under {recipe:?}:\n{log}"))
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Detailed routing follows the route the post-diode upsize judged, wherever the recipe runs the upsize. Global routing
/// re-routes the whole design, so a route made after the upsize measured is a route no slew check at its detection
/// limit has seen. MEASURED on systolic_pipe16_realbist (RUN_2026.10.16_04.50.56), whose recipe repairs no antenna in
/// global routing: the upsize judged 0.540 ns on a route of its own and left no data net over it; global routing then
/// routed afresh under the flow's arguments, and on that route 13 nets estimated 0.63–0.79 ns at the slow corner.
/// They signed off at 0.76–0.92 ns at the slow corner on max-RC extraction, 39 pins. On the route detailed routing
/// followed, the largest ratio of extracted to estimated slew over the 58,702 pins above 0.30 ns was 1.341, inside the
/// 1/(1 − 0.28) its 28% detection margin allows, so judging that route reaches every one of them.
#[test]
fn the_route_detailed_routing_follows_is_the_one_the_upsize_judged() {
    if !common::require_tool("tclsh") {
        return;
    }
    let routes = ["global_routing", "run_logos_route_slew"];
    let gate = |v: &Option<&str>| *v == Some("1");
    let opt = [None, Some("0"), Some("1")];
    let mut recipes = 0;
    for upsize in opt {
        for repair in ["0", "1"] {
            for post_upsize in opt {
                for fanout in opt {
                    let mut recipe = vec![("GRT_REPAIR_ANTENNAS", repair)];
                    for (k, v) in [
                        ("LOGOS_POST_DIODE_UPSIZE", upsize),
                        ("LOGOS_POST_ANTENNA_UPSIZE", post_upsize),
                        ("LOGOS_DIODE_FANOUT", fanout),
                    ] {
                        if let Some(v) = v {
                            recipe.push((k, v));
                        }
                    }
                    let steps = routing_steps(&recipe);
                    let drt: Vec<usize> = steps
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| *s == "detailed_routing")
                        .map(|(i, _)| i)
                        .collect();
                    assert_eq!(drt.len(), 1, "one detailed routing under {recipe:?}: {steps:?}");
                    let last = steps[..drt[0]]
                        .iter()
                        .rev()
                        .find(|s| routes.contains(&s.as_str()))
                        .unwrap_or_else(|| panic!("no route before detailed routing under {recipe:?}: {steps:?}"));
                    if gate(&upsize) {
                        assert_eq!(
                            last, "run_logos_route_slew",
                            "with the post-diode upsize on, detailed routing follows the route it judged, under \
                             {recipe:?}: {steps:?}"
                        );
                    } else {
                        assert!(
                            !steps.iter().any(|s| s == "run_logos_route_slew"),
                            "a recipe without the post-diode upsize routes as before, under {recipe:?}: {steps:?}"
                        );
                    }
                    let fill = steps.iter().position(|s| s == "ins_fill_cells").unwrap();
                    let at = steps[..drt[0]].iter().rposition(|s| s == last).unwrap();
                    assert!(
                        at < fill && fill < drt[0],
                        "the route, then fill, then detailed routing, under {recipe:?}: {steps:?}"
                    );
                    recipes += 1;
                }
            }
        }
    }
    assert_eq!(recipes, 54);

    let head = "run_resizer_design_routing run_resizer_timing_routing heuristic_diode_insertion \
                targeted_diode_insertion run_logos_diode_fanout";
    let tail = "ins_fill_cells detailed_routing run_antenna_eco";
    for (recipe, body, blocks) in [
        (
            &[("LOGOS_POST_DIODE_UPSIZE", "1"), ("GRT_REPAIR_ANTENNAS", "1")][..],
            "run_logos_post_diode_upsize add_route_obs global_routing run_logos_diode_fanout run_logos_route_slew",
            "every signed-off block that repairs antennas in global routing",
        ),
        (
            &[
                ("LOGOS_POST_DIODE_UPSIZE", "1"),
                ("GRT_REPAIR_ANTENNAS", "1"),
                ("LOGOS_POST_ANTENNA_UPSIZE", "1"),
                ("LOGOS_DIODE_FANOUT", "1"),
            ][..],
            "run_logos_post_diode_upsize add_route_obs global_routing run_logos_diode_fanout \
             run_logos_post_diode_upsize run_logos_route_slew",
            "the WPE and its wrapper's macro",
        ),
        (
            &[("GRT_REPAIR_ANTENNAS", "1")][..],
            "add_route_obs global_routing run_logos_diode_fanout global_routing",
            "a recipe without the upsize that repairs antennas",
        ),
        (
            &[("GRT_REPAIR_ANTENNAS", "0")][..],
            "add_route_obs global_routing",
            "a recipe without the upsize or the repair",
        ),
        (
            &[("LOGOS_POST_DIODE_UPSIZE", "1"), ("GRT_REPAIR_ANTENNAS", "0")][..],
            "run_logos_post_diode_upsize add_route_obs global_routing run_logos_route_slew",
            "systolic_pipe16_realbist and systolic_os_pipe8_realbist",
        ),
    ] {
        assert_eq!(
            routing_steps(recipe).join(" "),
            format!("{head} {body} {tail}"),
            "{blocks} route in this order"
        );
    }
}

/// Every incremental global route a step makes continues the route it increments, under that route's arguments.
/// OpenROAD's `global_route` takes its congestion tolerance, congestion iterations, grid origin and verbosity from each
/// call's own arguments, so a bare incremental call routes with congestion refused and fifty iterations whatever the
/// route it continues was made with. MEASURED on systolic_os_pipe8_realbist (RUN_2026.10.15_19.03.07): the post-diode
/// upsize measured on a route made with `-allow_congestion`, and its first incremental re-route, after round 1's
/// isolations, failed GRT-0232 ("Routing congestion too high"), so the block had no GDS.
#[test]
fn every_incremental_global_route_continues_the_route_it_increments() {
    let wrapper = "openroad/logos_global_route.tcl";
    let sourced = format!("source $::env(SCRIPTS_DIR)/{wrapper}");
    let dir = root().join("scripts/openlane-overlay/scripts/openroad");
    let mut incremental = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("tcl") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let calls: Vec<usize> = text
            .lines()
            .scan(0usize, |at, l| {
                let here = *at;
                *at += l.len() + 1;
                Some((here, l))
            })
            .filter(|(_, l)| {
                let t = l.trim_start();
                !t.starts_with('#') && t.starts_with("global_route")
            })
            .map(|(i, _)| i)
            .collect();
        if !text.lines().any(|l| {
            let t = l.trim_start();
            !t.starts_with('#')
                && t.starts_with("global_route")
                && (t.contains("-start_incremental") || t.contains("-end_incremental"))
        }) {
            continue;
        }
        incremental.push(name.clone());
        let src = text
            .find(&sourced)
            .unwrap_or_else(|| panic!("{name} routes incrementally and does not source {wrapper}"));
        assert!(
            calls.iter().all(|&c| src < c),
            "{name} sources {wrapper} before its first global route"
        );
    }
    assert!(
        incremental.contains(&"logos_post_diode_upsize.tcl".to_string()),
        "the post-diode upsize routes incrementally: {incremental:?}"
    );

    if !common::require_tool("tclsh") {
        return;
    }
    let probe = r#"
set ::calls [list]
proc global_route {args} { lappend ::calls $args }
source [lindex $argv 0]
source [lindex $argv 0]
global_route -congestion_iterations 30 -allow_congestion -guide_file route.guide
global_route -start_incremental
global_route -end_incremental
global_route -congestion_iterations 50 -verbose -congestion_report_file c.rpt -grid_origin {5 7}
global_route -end_incremental
foreach c $::calls { puts "CALL $c" }
if { [catch { interp create fresh } ] == 0 } {
    fresh eval { proc global_route {args} {} }
    fresh eval [list source [lindex $argv 0]]
    if { [catch { fresh eval { global_route -end_incremental } } msg] } { puts "REFUSED $msg" }
}
"#;
    let tmp = tempfile::tempdir().expect("a scratch directory");
    let script = tmp.path().join("probe.tcl");
    std::fs::write(&script, probe).unwrap();
    let out = std::process::Command::new("tclsh")
        .arg(&script)
        .arg(dir.join("logos_global_route.tcl"))
        .output()
        .expect("run tclsh");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    let calls: Vec<&str> = log.lines().filter_map(|l| l.strip_prefix("CALL ")).collect();
    assert_eq!(
        calls,
        [
            "-congestion_iterations 30 -allow_congestion -guide_file route.guide",
            "-congestion_iterations 30 -allow_congestion -start_incremental",
            "-congestion_iterations 30 -allow_congestion -end_incremental",
            "-congestion_iterations 50 -verbose -congestion_report_file c.rpt -grid_origin {5 7}",
            "-congestion_iterations 50 -verbose -congestion_report_file c.rpt -grid_origin {5 7} -end_incremental",
        ],
        "a route in full is made as asked, once, though the wrapper was sourced twice; each incremental call carries \
         the arguments of the last one, less its guide file, which a route in full writes once:\n{log}"
    );
    assert!(
        log.lines().any(|l| l.starts_with("REFUSED ")),
        "an incremental route with no route made in full before it is refused:\n{log}"
    );
}
