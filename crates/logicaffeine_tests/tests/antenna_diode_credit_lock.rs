//! antenna_diode_credit_lock — an antenna diode only counts if the checker can SEE it on the
//! violating layer's island.
//!
//! WHY THIS EXISTS. For a whole session every surviving antenna violator on `coproc_core` and
//! `systolic_pipe16_realbist` looked the same: a heuristic diode already on the pin, ratio still
//! 1.0-1.4x. It read as "one diode isn't enough". It was not that. OpenROAD's per-layer report
//! for coproc net `_18239_`, pin `_58895_/A2_N` (RUN_2026.09.22_21.58.12, 28-arc.log):
//!
//!     met4  Required 4974.96 (Side area)    <- diode diffusion credited
//!     met3  Required 4627.20                <- credited
//!     met2  Required 3295.52                <- credited
//!     met1  Required  400.00 (VIOLATED)     <- ZERO diffusion
//!
//! 400 is the no-diffusion value of sky130's met1 ANTENNADIFFSIDEAREARATIO
//! PWL((0 400)(0.0125 400)(0.0225 2609)(22.5 11600)). The router had joined the diode to the net
//! through met2, so at met1's deposit the gate's long met1 run had no diode on its island. The
//! fix was never more diodes — it was the right LAYER.
//!
//! The first repair attempt (a li1 stub from the pin's CENTRE to an extra diode) was not credited
//! either: `li1 Partial area 0.00`, `met1 Required 400.00` unchanged, because the route reaches a
//! pin at its ACCESS VIA, not its centre, so the stub never joined the gate's routed island.
//!
//! The met1 tie onto that access via, applied to the same saved database and re-checked by
//! OpenROAD itself, moved it exactly as the rule predicts:
//!     met1 Required 400.00 -> 2773.88,  Partial 531.89 -> 124.79,  ratio 1.33 -> 0.045 (PASS)

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_met1_tie_geometry_is_correct_on_the_measured_case() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_met1_tie.py");
    let body = std::fs::read_to_string(&t).expect("read test_met1_tie.py");
    assert!(
        body.contains("(368090, 123790, 369915, 124070)"),
        "the unit test lost the MEASURED blocking wire from coproc _58895_/A2_N; a synthetic \
         replacement cannot prove the search handles what actually blocked it"
    );
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run test_met1_tie.py");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: met1 tie geometry correct"),
        "{log}"
    );
}

/// The ECO must tie onto the gate's ACCESS VIA on met1, never stub li1 from the pin centre.
#[test]
fn the_diode_eco_ties_on_the_layer_the_checker_credits() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .expect("read magic_antenna_eco.py");
    assert!(
        !src.contains("def connect_stub("),
        "the li1 pin-centre stub is back. It was measured NOT to be credited (li1 Partial area \
         0.00, met1 Required stayed 400.00) — an extra diode wired that way is decoration."
    );
    assert!(
        src.contains("def gate_access_point(") && src.contains("startswith(\"L1M1\")"),
        "the tie must target the route's li1->met1 access via at the pin — that via's met1 is \
         part of the island check_antennas evaluates at met1's deposit"
    );
    assert!(
        src.contains("enc.addTechVia(via)") && src.contains("def apply_tie("),
        "the diode must go UP to met1 through a via; staying on li1 is what was not credited"
    );
    assert!(
        src.contains("pts = plan_or_none(tech, foreign, (px, py), access)")
            && src.contains("commit(r, start, diode_span, evict, d, evict_gaps)"),
        "a site may only be committed (fill evicted) once a CLEAR tie exists for it — committing \
         first and destroying on failure leaves an unfilled hole (nwell.2a)"
    );
    let plan = src
        .find("pts = plan_or_none(tech, foreign, (px, py), access)")
        .unwrap();
    let commit = src
        .find("commit(r, start, diode_span, evict, d, evict_gaps)")
        .unwrap();
    assert!(
        plan < commit,
        "the clearance plan must precede the commit that evicts fill"
    );
    assert!(
        src.contains("def foreign_met1_near(") && src.contains("foreign = foreign_met1_near("),
        "foreign met1 must be collected ONCE per pin; collecting per candidate site is hundreds of \
         full-design scans per pin"
    );
}

/// The tie targets the gate's whole met1 ISLAND, not just its access via.
///
/// MEASURED on `systolic_pipe16_realbist` `_174816_/A` (net `_050941_`, RUN_2026.09.23_03.24.05):
/// met1 Partial 430.11 vs Required 400.00 — a heuristic diode on the net, joined through met2, so
/// uncredited at met1. All 601 diode sites near the access via were blocked, and the run ended at
/// ONE antenna violation. The gate's met1 island runs 75 um west at y=380290 through open rows; the
/// island-targeted ECO on the same saved database placed a tie there and OpenROAD's
/// `check_antennas` then reported 0 net / 0 pin violations for the whole design.
#[test]
fn the_diode_tie_may_land_anywhere_on_the_gates_met1_island() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .expect("read magic_antenna_eco.py");
    for f in [
        "def wire_segments(",
        "def met1_island(",
        "def nearest_on_island(",
        "def island_candidates(",
    ] {
        assert!(src.contains(f), "missing {f}");
    }
    assert!(
        src.contains("cands = island_candidates(rows, island,"),
        "diode sites must be searched along the whole island, not only around the gate"
    );
    let target = src
        .find("tl, access = nearest_target(targets_t, (px, py))")
        .expect("per-site island target");
    let plan = src
        .find("pts = plan_or_none(tech, foreign, (px, py), access)")
        .unwrap();
    assert!(
        target < plan,
        "each candidate's tie must aim at ITS nearest island point"
    );
    assert!(src.contains("layer = top if layer == bot else bot"),
        "the layer must follow every via, or met1 reached through met2 merges into the gate's island \
         and the ECO ties a diode where the checker gives no credit");
    let t = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_met1_tie.py"),
    )
    .unwrap();
    assert!(t.contains("(924830, 380290)") && t.contains("382670"),
        "the unit test lost the MEASURED pipe16 island (the 75 um run, and the met2-joined island it must exclude)");
}

/// A violation on met3 (or any layer above met1) is tied in the component BELOW that layer, and may
/// land on met2 through one via; the component is seeded through a via stack at the access point.
///
/// MEASURED on `fabric_horner_seq` (RUN_2026.09.23_10.20.59): three pins violated on met3 —
/// `_25250_/C`, `_33411_/C` (net `_23278_`, ratio 2.06) and `_33412_/A2` (net `_01470_`, 1.42).
/// The met1-island ECO refused all three twice (517-588 sites each): `_23278_` had 4.9 um of met1
/// below met3, and `_01470_` left its pin through stacked L1M1+M1M2 and had NO met1 at all, but 94
/// and 144 um of met2. With layer-aware components, met2 landings and the stacked seed, the same
/// saved database went from 2 nets / 3 pins to 0 / 0 under OpenROAD's `check_antennas`.
#[test]
fn a_violation_above_met1_is_tied_in_the_component_below_its_layer() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    for f in [
        "def wire_vias(",
        "def island_below(",
        "def tie_targets(",
        "def nearest_target(",
        "def landing_clear(",
        "def island_rank(",
    ] {
        assert!(src.contains(f), "missing {f}");
    }
    assert!(
        src.contains("targets_t = tie_targets(wire_segments(wops), vias, access, lowest)"),
        "the targets must come from the component below the pin's LOWEST violating layer"
    );
    assert!(
        src.contains("apply_tie(tech, it.getNet(), dit, pts, landing)"),
        "a met2 target must be reached through the landing via"
    );
    assert!(
        src.contains("if p == access and (b in at) != (t in at):"),
        "the component seed must climb a via stack standing on the access point"
    );
    let rt = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    assert!(
        rt.contains("dict lappend _pin_layers") && rt.contains("puts $_f \"$_p\\t$_l\""),
        "routing.tcl must hand each violator's LAYER to the ECO, or every pin is treated as met1"
    );
    let t = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_met1_tie.py"),
    )
    .unwrap();
    assert!(
        t.contains("a met3 violation reaches the far met1 branch through met2")
            && t.contains("a stacked access reaches its met2 run for a met3 violation"),
        "the unit tests for the layer-aware component are gone"
    );
}

/// A diode placed under the island is tied by ONE via from its own pin — no routed metal to block.
///
/// MEASURED on `coproc_core` `_36364_/A_N` (net `_31234_`, RUN_2026.09.24_02.17.38): its met1 island
/// runs 83 um, yet every one of 1,182 sites was refused because an L-shaped met1 tie from a diode a
/// row away had to cross horizontal met1 tracks full of foreign nets — the block's last antenna
/// violation across three full runs. With the drop-in tie tried first, the same saved database went
/// to 0 net / 0 pin violations under OpenROAD's `check_antennas`.
#[test]
fn a_diode_under_the_island_is_tied_by_a_single_via() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    assert!(
        src.contains("def plan_drop("),
        "the drop-in planner is gone"
    );
    let drop = src
        .find("drop = plan_drop(tech, foreign,")
        .expect("the drop-in must be tried per site");
    let lpath = src
        .find("pts = plan_or_none(tech, foreign, (px, py), access) if ok else None")
        .unwrap();
    assert!(
        drop < lpath,
        "the zero-metal drop-in must be tried BEFORE an L-shaped met1 path"
    );
    let t = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_met1_tie.py"),
    )
    .unwrap();
    assert!(
        t.contains("a pin under the island drops one via straight up into it")
            && t.contains("the via's li1 enclosure must stay inside the pin"),
        "the drop-in unit tests are gone"
    );
}

/// A violation above met2 may be tied by a via STACK straight up from the diode's own pin into the violating layer
/// itself — no routed metal at all — and the stack is held to every rule its enclosures and cuts meet.
///
/// MEASURED on `wpe_user_project` net1123 (RUN_2026.09.27_18.14.14, 38-eco_jog_6.odb): `_353370_/B2` violated on
/// met3 at ratio 2.79. Its component below met3 was a 168 um met3 run over open rows plus its own 850 nm met2 access
/// stack in dense logic, and the ties could only aim at that stack: all 629 candidate sites were refused, twice, and
/// the run ended one antenna violation short. The run crosses row 822 1.70 um into it, inside a diode_2's li1 pin and
/// clear of the rail, so a diode anywhere under it takes L1M1 + M1M2 + M2M3 at one point.
#[test]
fn a_violation_above_met2_lands_by_a_stack_straight_up_from_the_pin() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_stack_drop.py");
    let body = std::fs::read_to_string(&t).expect("read test_stack_drop.py");
    assert!(
        body.contains("2248420") && body.contains("2289170") && body.contains("_353370_/B2"),
        "the unit test lost the MEASURED net1123 component"
    );
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run test_stack_drop.py");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: stacked diode drop geometry correct"),
        "{log}"
    );

    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    for f in [
        "def drop_targets(",
        "def via_stacks(",
        "def stack_jogs(",
        "def stack_is_clear(",
        "def plan_stack_drop(",
    ] {
        assert!(src.contains(f), "missing {f}");
    }
    let drop = src
        .find("drop = plan_drop(tech, foreign,")
        .expect("the met1 drop");
    let stack = src
        .find("stacked = plan_stack_drop(")
        .expect("the stacked drop must be tried per site");
    let lpath = src
        .find("tl, access = nearest_target(targets_t, (px, py))")
        .unwrap();
    assert!(
        drop < stack && stack < lpath,
        "per site: the met1 drop, then the stack onto an upper layer, then an L-shaped met1 path"
    );
    assert!(src.contains("island = [(a, b) for _l, a, b in targets_t] + [(a, b) for _l, a, b in stack_t]"),
        "diode sites must be searched under the upper-layer runs a stack can reach, not only along met1/met2");
    assert!(src.contains("if pts is not None and stacked is None and tl == \"met2\":"),
        "a stack is checked in full by stack_is_clear; the fixed-orientation met2 landing must not re-judge it");
}

/// An upper-layer run of the gate's component that passes BESIDE the diode's pin is reached by a stacked jog: the
/// stack to the layer below the run, one straight jog there, the via up onto it. A straight stack needs the run
/// directly over a pin a few hundred nm wide on a 460 nm site grid. MEASURED on coproc_core
/// (RUN_2026.10.03_15.18.47, 36-eco_jog_5.odb): `_45844_/B1`, `_45845_/C`, `logos_fo_3359/A` and `wire374/A` violated
/// on met4 and every site tried was refused; with the jog all four took a diode and OpenROAD's check_antennas read
/// 0 pin / 0 net violations, from 4 / 3.
#[test]
fn an_upper_run_beside_the_pin_is_reached_by_a_stacked_jog() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_stack_drop.py");
    let body = std::fs::read_to_string(&t).expect("read test_stack_drop.py");
    assert!(
        body.contains("_45844_/B1")
            && body.contains("eco.plan_stack_jog(")
            && body.contains("[pc, VIAS[\"M1M2_PR\"], VIAS[\"M2M3_PR\"], (2402000, pc[1])]"),
        "the unit test lost the measured coproc case or the jog's own cases"
    );
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    let plan = &src[src.find("def plan_stack_jog(").expect("the stacked jog")..];
    let plan = &plan[..plan.find("\ndef ").unwrap()];
    for needle in [
        "conflicts(r, f, sp)",
        "makes_notch(rects",
        "self_notch(rects",
        "makes_neck(rects",
        "connected_area(rects",
        "cuts_too_close(at, c, cw, ch, csp)",
        "> reach",
        "low is None or low == \"li1\"",
        "q == pt",
    ] {
        assert!(
            plan.contains(needle),
            "the jog's geometry is checked in full — missing {needle:?}:\n{plan}"
        );
    }
    let stack = src
        .find("stacked = plan_stack_drop(tech, pin_bb,")
        .expect("the straight stack first");
    let jog = src
        .find("plan_stack_jog(tech, pin_bb, stack_t, stack_foreign, stack_own, stack_cuts)")
        .expect("then the jog");
    let lpath = src
        .find("tl, access = nearest_target(targets_t, (px, py))")
        .unwrap();
    assert!(
        stack < jog && jog < lpath,
        "per site: the straight stack, then the stacked jog, then the L-shaped met1 path"
    );
    assert!(
        src.contains("jogged = None if stacked is not None or not stack_t else"),
        "the jog is planned only where the straight stack found no landing"
    );
    assert!(src.contains("stacked = (jogged[1], jogged[2], jogged[4])"),
        "the jog's landing via is the tie's last via — without it the jog stops under the run, unconnected");
    assert!(
        src.contains(
            "tl, access, pts = jogged[2], jogged[1], [jogged[0]] + list(jogged[3]) + [jogged[1]]"
        ),
        "the jog is one encoder path: the pin, the stack's vias, the landing point"
    );
    let enc = &src[src.find("def apply_tie(").unwrap()..];
    let enc = &enc[..enc.find("\ndef ").unwrap()];
    assert!(
        enc.contains("if hasattr(p, \"getBottomLayer\"):\n            enc.addTechVia(p)"),
        "a via in the path climbs from the current point:\n{enc}"
    );
}

/// Once the antenna ECO has changed the layout, the netlist is written again from it, so signoff STA and the
/// final gate-level view describe the chip that was drawn. A diode pass adds diode and filler cells and removes
/// the fillers it evicts after detailed routing wrote the netlist; `wpe_user_project` RUN_2026.09.27_18.14.14
/// signed off timing on a netlist missing 253 ECO diodes, and four recorded blocks were clean the same way.
#[test]
fn the_netlist_is_written_again_once_the_eco_touches_the_layout() {
    let rt = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/tcl_commands/routing.tcl"),
    )
    .unwrap();
    let outer = &rt[rt
        .find("proc run_antenna_eco {args} {")
        .expect("run_antenna_eco")..];
    let outer = &outer[..outer.find("\n}\n").unwrap()];
    let reset = outer
        .find("set ::logos_eco_touched_layout 0")
        .expect("the flag is cleared before the passes");
    let passes = outer
        .find("logos_antenna_eco_iterations")
        .expect("the passes run inside");
    let guard = outer
        .find("if { $::logos_eco_touched_layout } {")
        .expect("the write is conditioned on the flag");
    let write = outer
        .find("write_verilog")
        .expect("the netlist is written from the layout");
    assert!(
        reset < passes && passes < guard && guard < write,
        "clear, run the passes, then write if they touched it"
    );
    assert!(
        outer[write..].contains("-powered_to"),
        "the powered netlist LVS can read is written with it"
    );

    let inner = &rt[rt
        .find("proc logos_antenna_eco_iterations {args} {")
        .expect("the pass loop")..];
    let inner = &inner[..inner.find("\nproc ").unwrap_or(inner.len())];
    let edits: Vec<usize> = inner
        .match_indices("manipulate_layout")
        .map(|(i, _)| i)
        .collect();
    let flags: Vec<usize> = inner
        .match_indices("set ::logos_eco_touched_layout 1")
        .map(|(i, _)| i)
        .collect();
    assert!(
        !edits.is_empty() && edits.len() == flags.len(),
        "every layout edit in the pass loop raises the flag: {} edits, {} flags",
        edits.len(),
        flags.len()
    );
    for (k, e) in edits.iter().enumerate() {
        let next = edits.get(k + 1).copied().unwrap_or(inner.len());
        assert!(
            flags.iter().any(|f| f > e && *f < next),
            "the edit at byte {e} of the pass loop raises no flag before the next edit"
        );
    }
}

/// A diode never lands where it would sit alone on an island it cannot carry. sky130's `diode_2` declares its own pin's
/// antenna gate area, so a diode joined to the violating gate only through a HIGHER layer is judged as a gate at every
/// layer below that join, with only its own diffusion. MEASURED on `systolic_pipe16_realbist` (RUN_2026.09.29_02.09.26):
/// `clkbuf_0_clk/A`'s met4 violation drew a diode onto a met3 run of `clk` that reaches the gate only through met4; the
/// gate cleared and the checker flagged the diode itself on met3, 3162.42 against 2773.88, which neither the jog nor
/// the diode ECO repairs. Replayed with the guard from the same database, the ECO placed that diode in the gate's own
/// component and `check_antennas` reported 0 nets / 0 pins for the design. The rules are read from the LEF the ECO is
/// given (the odb binding exposes neither the PWL tables nor a layer's thickness).
#[test]
fn a_diode_never_lands_alone_on_an_island_it_cannot_carry() {
    let t = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_stack_drop.py"),
    )
    .unwrap();
    assert!(
        t.contains("3162.42") && t.contains("2773.88") && t.contains("clkbuf_0_clk"),
        "the unit test lost the MEASURED clk island"
    );
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_stack_drop.py"))
        .current_dir(root())
        .output()
        .expect("run test_stack_drop.py");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: stacked diode drop geometry correct"),
        "{log}"
    );

    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    for f in [
        "def lef_antenna_rules(",
        "def pwl_at(",
        "def landing_is_safe(",
    ] {
        assert!(src.contains(f), "missing {f}");
    }
    assert!(
        src.contains("def eco(reader, input_lef,")
            && src.contains("antenna_rules = lef_antenna_rules(input_lef, diode_cell, diode_pin)"),
        "the rules come from the LEF the flow hands the ECO"
    );
    assert!(
        src.contains("targets_t = [t for t in targets_t if safe(t)]"),
        "every L-tie target passes the guard"
    );
    assert!(
        src.contains("if t[0] != \"met1\" and safe(t)]"),
        "every stacked-drop target passes the guard"
    );
}
