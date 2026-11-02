//! jog_router_geometry_lock — the antenna jog-router may never hand on geometry the signoff
//! tools will disagree about.
//!
//! WHY THIS EXISTS. The jog router rewrites a routed segment `A -> B` as
//! `A -> p0 -> (via) -> p1 -> (via) -> B`, leaving two stubs of original-layer metal: `A..p0` and
//! `p1..B`. A stub shorter than the layer's minimum width is a SLIVER, and Magic and KLayout
//! resolve slivers differently — Magic re-derives geometry as it streams GDS while KLayout
//! streams the DEF — so the fragment appears in one layout and not the other and the run dies at
//! signoff XOR, an hour after the jog created it and with nothing pointing back at the jog.
//!
//! IT WAS DIAGNOSED FOUR TIMES WRONG FIRST, which is why the evidence is written down here:
//!   * "DEF labels" — tested, `MAGIC_DEF_LABELS=0`, XOR unchanged at 7.
//!   * "via enclosures overlapping pins" — implemented, rejected 0 windows, changed nothing.
//!   * "off-grid coordinates" — measured, every bridge already lands on the 5 DBU grid.
//!   * "via ENCLOSURE orientation" — implemented, XOR 3 before and 3 after. Not the cause.
//! Only the sliver guard moved the number, 7 -> 3, and it is a real defect worth keeping.
//!
//! THE ACTUAL CAUSE OF THE RESIDUE, read out of the two GDS files instead of theorised. The
//! surviving XOR items sit on layer 68/44 — the `via` CUT layer, not any metal and not a pin.
//! At coproc_core RUN_2026.09.22_05.14.34:
//!     KLayout (streams the DEF verbatim) wrote cuts 0.090 um apart, against sky130's
//!         `LAYER via  WIDTH 0.15  SPACING 0.17`, which needs 0.320 um between centres;
//!     Magic (paints contacts as AREAS and regenerates cuts when it streams GDS) merged them
//!         into one correct cut, and at the other site wrote 2 cuts where KLayout wrote 3.
//! So MAGIC SILENTLY REPAIRS ILLEGAL VIA SPACING THE ROUTER CREATED, while the DEF — which is
//! what describes the fabricated article — keeps it. Measurement and inference kept apart:
//!   MEASURED     the two GDS files disagree, and KLayout's copy breaks the 0.17 um rule.
//!   INFERRED     Magic DRC cannot flag it, because that deck runs on Magic's own database and
//!                the merge happens before the check.
//!   NOT MEASURED either way, because XOR runs before Magic DRC and killed every coproc run at
//!                step 40 — no coproc run has ever reached the DRC step.
//! XOR was the only gate that saw this at all, and it was doing its job.
//!
//! TWO DISTINCT DEFECTS, read straight out of the DEF the signoff tools consumed
//! (tmp/routing/28-eco_jog_3.def — note that results/routing/*.odb is the PRE-ECO database and
//! does not contain the bridges at all, which cost one wrong measurement to discover):
//!
//!   NEW met1 ( 577530 493850 ) ( 577870 * ) M1M2_PR ( 578110 * ) M1M2_PR ( 578450 * )
//!       a bridge's OWN two vias, 240 DBU apart. A bridge puts a via at each end of its window,
//!       so THE WINDOW WIDTH IS THE VIA-TO-VIA CENTRE DISTANCE — and the ladder tried
//!       500/340/240/180 nm against a 320 nm rule for via1 and 400 nm for via2/via3. Two of the
//!       four rungs could never produce legal geometry on any layer.
//!
//!   NEW met1 ( 1088590 438770 ) ( 1121910 * ) M1M2_PR ( 1122410 * ) M1M2_PR ( 1123090 * )
//!       this bridge's own vias are 500 apart and perfectly legal — but net _06073_ carries a
//!       via at 1122170, 260 from each of them. The crowding via is on a FOREIGN net, so a guard
//!       that only inspected the net being bridged would have sailed straight past it.
//!
//! Hence two guards, not one: filter the ladder by the PDK cut pitch, AND scan the neighbourhood
//! (every net, the power grid included) before accepting a window position.
//!
//! The fixes are at the SOURCE: the router refuses a window that would leave a sub-minimum stub,
//! refuses any window narrower than the PDK's own cut pitch, keeps scanning past a position whose
//! via would crowd an existing cut, and finally re-reads the finished database and dies if any
//! cut it placed is too close to anything. This lock pins all of it.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The sliver predicate is exercised directly, as a unit — no OpenLane run required, so this
/// gate is cheap enough to hold on every commit rather than only when someone hardens a block.
#[test]
fn the_jog_router_refuses_windows_that_leave_sub_minimum_stubs() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_sliver_guard.py");
    assert!(
        t.is_file(),
        "the sliver-guard unit test is missing: {}",
        t.display()
    );

    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run the sliver-guard unit test");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: sliver predicate correct"),
        "the jog router's sliver guard is broken. A bridge that leaves a stub shorter than the \
         layer's minimum width produces geometry Magic and KLayout disagree about, and the run \
         fails signoff XOR an hour later with nothing pointing at the jog.\n{log}"
    );
}

/// The guard must be WIRED IN, not merely defined. A predicate nothing calls is documentation.
#[test]
fn the_sliver_guard_is_actually_called_before_bridging() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .expect("read sat_antenna_jog.py");
    assert!(
        src.contains("def bridge_leaves_slivers("),
        "the sliver predicate is gone from the jog router"
    );
    assert!(
        src.contains("if bridge_leaves_slivers(pick[\"pts\"], cand[0], cand[1], min_w):"),
        "bridge_leaves_slivers is defined but not consulted on the candidate window — the guard \
         would be inert, which is the same failure class as the pin-overlap attempt that ran \
         happily and rejected nothing"
    );
    assert!(
        src.contains("horizontal = abs(bx - ax) >= abs(by - ay)")
            && src.contains("bridge_layer_via(tech, layer, horizontal)"),
        "the via orientation must be chosen from the SEGMENT's direction. sky130 ships each via \
         as a transposed pair (M1M2_PR is met1 320x260 / met2 260x320, M1M2_PR_R the other way), \
         so the variant is picked to suit the layer the violation is on. NOTE, honestly: this did \
         NOT fix the XOR — measured 3 before and 3 after. It is held because matching the \
         violating layer is the better default, not because it moved a number. The XOR cause was \
         cut spacing; see the_jog_router_cannot_place_vias_closer_than_the_pdk_cut_rule."
    );
    assert!(
        src.contains("min_w = layer_min_width(tech, layer)"),
        "the minimum width must come from the TECH, not a constant — it differs per layer and \
         per PDK, and gf180's metals are not sky130's"
    );
}

/// Every block recorded signoff-clean must have XOR at zero.
///
/// The backstop for the whole class: whatever the jog does, a block whose Magic and KLayout
/// layouts disagree is not signed off. This is what caught the sliver bug in the first place.
#[test]
fn no_block_is_recorded_clean_with_xor_differences() {
    let mut checked = 0;
    for p in sky130_ledgers::ledgers(&root(), None) {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let Some(blocks) = doc["blocks"].as_object() else {
            continue;
        };
        for (b, v) in blocks {
            if !v["signoff_clean"].as_bool().unwrap_or(false) {
                continue;
            }
            checked += 1;
            assert_eq!(
                v["xor_differences"].as_i64().unwrap_or(-1),
                0,
                "{name}/{b}: recorded signoff-clean with nonzero XOR — Magic and KLayout do not \
                 agree on what this layout IS, so one of them is describing silicon that will \
                 not be fabricated"
            );
        }
    }
    assert!(
        checked >= 10,
        "only {checked} clean blocks inspected — the ledger scan is not working"
    );
}

/// The cut-spacing guard, wired in at every point it has to be.
///
/// This is the fix that actually closes the XOR. It is asserted structurally and not only by unit
/// test because the failure mode on this defect has twice been a guard that was defined, looked
/// right, and was never consulted.
#[test]
fn the_jog_router_cannot_place_vias_closer_than_the_pdk_cut_rule() {
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .expect("read sat_antenna_jog.py");

    assert!(
        src.contains("def via_cut_rule(via):"),
        "the cut rule must be READ FROM THE PDK. sky130's via1 is WIDTH 0.15 SPACING 0.17 while \
         via2/via3 are 0.20/0.20 and mcon is 0.17/0.19; gf180's are none of those. A constant \
         here is a defect waiting for the second PDK."
    );
    assert!(
        src.contains("cut_name, cut_w, cut_h, cut_sp = via_cut_rule(via)"),
        "via_cut_rule is defined but the bridge path never calls it"
    );
    assert!(
        src.contains("if cut_name is None:"),
        "a via whose cut rule cannot be read must REFUSE the bridge. Guessing a spacing rule is \
         how illegal geometry reached GDS in the first place."
    );
    assert!(
        src.contains("cut_pitch = (cut_w if horizontal else cut_h) + cut_sp"),
        "the minimum via-to-via centre distance is the cut's own size plus the layer's SPACING, \
         measured along the segment's direction"
    );
    assert!(
        src.contains("widths = [_w for _w in (500, 340, 240, 180) if _w >= cut_pitch]")
            && src.contains("if not widths:"),
        "the window ladder must be FILTERED by the cut pitch, and a layer where no rung clears it \
         must refuse rather than fall through. The 240 nm rung is exactly what produced the \
         0.090 um cut gap that failed XOR."
    );
    assert!(
        src.contains("def collect_cuts(block, region, cut_layer_name):"),
        "the existing cuts near a candidate window have to be gathered before one can be avoided"
    );
    let fn_body = |name: &str| -> &str {
        let s = &src[src
            .find(&format!("def {name}("))
            .unwrap_or_else(|| panic!("missing def {name}"))..];
        &s[..s[1..].find("\ndef ").map_or(s.len(), |i| i + 1)]
    };
    let index = &src[src.find("class NetIndex:").expect("the net index")..];
    let index = &index[..index.find("\ndef ").unwrap_or(index.len())];
    assert_eq!(
        (
            index.matches("for sw in n.getSWires():").count(),
            index.contains("if b.isVia():")
                && index.contains("self.swire_cuts")
                && index.contains("self.swires"),
            fn_body("collect_cuts").contains("idx.special_cuts(region)"),
            fn_body("collect_foreign").contains("idx.special_rects(layer_name, region, spacing)"),
        ),
        (1, true, true, true),
        "BOTH neighbourhood scans must sweep SPECIAL wires — collect_cuts for the power grid's \
         VIAS and collect_foreign for its METAL. getWire() returns neither, and on sky130 \
         OpenLane puts the PDN straps on met4/met5, exactly where a met3 antenna violation \
         bridges up to. Measured on coproc_core RUN_2026.09.22_06.25.14, the first run to get \
         past XOR and reach LVS: net \\R[1][47] (a register output) came out SHORTED TO VGND, \
         all six of its pins on VGND in the layout, after two met3->met4 bridges. Netgen: 36223 \
         nets against the source's 36224. A register tied to ground is a dead chip, not a nit."
    );
    assert!(
        src.contains("def inside(p):") && src.contains("and inside(last):"),
        "collected cuts must be bounded to the region. Without it collect_cuts returns every cut \
         on any net that merely passes nearby — measured at 18,005 cuts for a single window scan \
         on coproc_core, checked once per candidate position."
    );
    assert!(
        src.contains("region_cuts, (cut_w, cut_h, cut_sp))"),
        "the existing cuts and the rule must reach free_window, or the scan cannot avoid landing \
         a via beside one already on the net — which is what the second XOR site was, a bridge \
         cut 0.260 um from an existing cut against a 0.320 um rule"
    );

    assert!(
        src.contains("def audit_cut_spacing(block, placed):"),
        "the post-bridge self-audit is missing"
    );
    assert!(
        src.contains("violations = audit_cut_spacing(block, placed_cuts)"),
        "audit_cut_spacing is defined but never run — the exact shape of the pin-overlap guard \
         that rejected zero windows while being believed to work"
    );
    assert!(
        src.contains("placed_cuts.append((cut_name, cut_w, cut_h, cut_sp, win[0]))")
            && src.contains("placed_cuts.append((cut_name, cut_w, cut_h, cut_sp, win[1]))"),
        "both ends of every bridge must be recorded for the audit; auditing one end would pass a \
         layout whose other end is defective"
    );
    let audit_at = src
        .find("if violations:")
        .expect("the audit result must be tested");
    let tail = &src[audit_at..];
    assert!(
        tail[..tail.len().min(800)].contains("raise SystemExit(1)"),
        "a cut-spacing violation must FAIL THE FLOW. Printing a warning and carrying on is how a \
         defect reaches GDS and is found three stages later as an unexplained XOR count."
    );
}

/// The guards are exercised against the real measured coordinates, not invented ones.
#[test]
fn the_cut_spacing_guard_is_pinned_to_the_geometry_that_actually_failed() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_sliver_guard.py");
    let body = std::fs::read_to_string(&t).expect("read the guard unit test");
    for probe in [
        "MEASURED spot1: 577.870 vs 578.110 (240 apart)",
        "MEASURED spot0: 1121.910 vs 1122.170 (260)",
        "exactly at the rule: 320 apart (legal)",
    ] {
        assert!(
            body.contains(probe),
            "the unit test lost its measured case {probe:?}. These are the real cut centres read \
             out of coproc_core.klayout.gds at the three XOR coordinates; a synthetic replacement \
             cannot prove the guard catches what actually shipped."
        );
    }

    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run the guard unit test");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: jog geometry guards correct"),
        "the jog router's via-cut-spacing guard is broken. Two cuts closer than the PDK's SPACING \
         rule are a real DRC violation that Magic hides by merging them, so it surfaces only as \
         an unexplained signoff XOR count.\n{log}"
    );
}

/// A bridge must not notch its OWN net.
///
/// MEASURED: infinichip net308 (RUN_2026.09.23_09.31.39). The jog put a bridge via 35 nm (in met1
/// enclosure terms) from the pin's L1M1 access via of the same net. OpenLane's Magic DRC read 0; the
/// golden `sky130A_mr` deck a shuttle is accepted against reported 13 m1.2 violations on infinichip
/// and 6 on dram_executor_wb, every one a same-net notch left by a bridge. The guard rejects any
/// window whose new enclosures or bridge wire sit strictly between 0 and the spacing rule from the
/// net's own metal, on both layers, and the scan moves on to the next position.
#[test]
fn the_jog_router_never_notches_its_own_net() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_notch_guard.py");
    let body = std::fs::read_to_string(&t).expect("read test_notch_guard.py");
    assert!(
        body.contains("(7445, 191135, 7735, 191365)")
            && body.contains("(7770, 191120, 8090, 191380)"),
        "the unit test lost the MEASURED infinichip enclosures that golden DRC flagged"
    );
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run the notch-guard unit test");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: notch guard correct"),
        "{log}"
    );

    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .expect("read sat_antenna_jog.py");
    assert!(
        src.contains("reject=notch"),
        "the notch predicate must be passed INTO the window scan, so a rejected position moves on \
         to the next one instead of ending the search"
    );
    assert!(
        body.contains("(106560, 342000, 106850, 342290)")
            && body.contains("(106775, 342215, 107065, 342505)"),
        "the unit test lost the MEASURED gemm_mac_realbist enclosures that golden m4.1 flagged"
    );
    let notch = &src[src
        .find("            def notch(p0, p1):")
        .expect("the scan's predicate")..];
    let notch = &notch[..notch.find("\n\n").unwrap_or(notch.len())];
    assert!(
        notch.contains("makes_neck(new, kept(l, p0, p1), layer_min_width(tech, l))"),
        "a bridge must not NECK its own net either: same-net metal that merges only through a corner \
         or a short stretch of edge is one polygon narrower than the width rule (gemm_mac_realbist \
         _06199_, golden m4.1 x2, Magic silent):\n{notch}"
    );
    let kept = &src[src
        .find("            def kept(l, p0, p1):")
        .expect("the metal a bridge leaves")..];
    let kept = &kept[..kept.find("\n\n").unwrap_or(kept.len())];
    assert!(
        kept.contains("return own_rest[l] + [seg_rect([e, p], half_l) for e, p in ((A, p0), (p1, B)) if e != p]"),
        "the neck is judged against the metal the bridge LEAVES on the lifted layer — the segment \
         gives way to its two stubs:\n{kept}"
    );
    assert!(
        src.contains("if ok and reject is not None:"),
        "free_window must consult the predicate"
    );
    assert!(
        src.contains("for l in (layer, blayer)"),
        "both layers the bridge touches must be checked — the bridge wire lands on the upper one"
    );
    let cf = &src[src.find("def collect_foreign(").expect("collect_foreign")..];
    let cf = &cf[..cf.find("\ndef ").unwrap_or(cf.len())];
    assert!(
        cf.contains("via_layer_rects(o[1], last, layer_name)"),
        "foreign VIA ENCLOSURES must be foreign metal: missing them let a bridge land 95 nm from \
         row[4]'s L1M1 enclosure on infinichip (golden m1.2, Magic silent)"
    );
    let near = &src[src
        .find("    def near(self, probe):")
        .expect("NetIndex.near")..];
    let near = &near[..near.find("\n    def ").unwrap_or(near.len())];
    assert!(cf.contains("for n in idx.near(probe):") && cf.contains("g = half + spacing + ENCLOSURE_REACH")
            && near.contains("overlaps(self.box[nm], probe)"),
        "the per-net pre-filter must use a region grown by the inflation and the widest enclosure: \
         filtering on the raw region skipped infinichip's net148, whose enclosure sat 95 nm from a bridge");
    assert!(
        src.contains("vfor = collect_foreign(block, region, layer,"),
        "the bridge's new enclosures on the VIOLATING layer must be checked against foreign metal \
         there, not only the bridge layer"
    );
}

/// A wire carrying via-enclosure RECT patches is re-encoded WITH them, a bridge never notches
/// itself, and the window ladder is lifted to what the via's enclosures allow.
///
/// MEASURED on `fabric_horner_seq` (RUN_2026.09.23_12.05.48):
/// * every RECT-carrying wire was refused because the odb Python binding cannot read a patch; the
///   DEF of the same state carries them, and decoding, attaching and re-encoding all 925 such nets
///   rewrote a byte-identical NETS section (the one initial refusal, `clk`, was a PINS/NETS parse
///   bug, now tested);
/// * a 340 nm met1->met2 bridge on `_07269_` put its two via enclosures 20 nm (met1) and 80 nm
///   (met2) apart — golden sky130A_mr m1.2 + 2x m2.2, Magic silent;
/// * the shortest legal window is max(enclosure along the wire + spacing): M1M2 460, M2M3 630,
///   M3M4 680 DBU. With it, net580 (met3, ratio 1.40) bridged met3->met4 and OpenROAD's
///   `check_antennas` read 0 / 0 for the whole design.
#[test]
fn rect_patches_round_trip_and_bridges_never_notch_themselves() {
    for (t, pass) in [
        ("test_rect_roundtrip.py", "PASS: RECT patches round-trip"),
        ("test_notch_guard.py", "PASS: notch guard correct"),
    ] {
        let path = root()
            .join("scripts/openlane-overlay/scripts/odbpy/tests")
            .join(t);
        let out = std::process::Command::new("python3")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .arg(&path)
            .current_dir(root())
            .output()
            .unwrap_or_else(|e| panic!("run {t}: {e}"));
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success() && log.contains(pass), "{t}:\n{log}");
    }
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .unwrap();
    assert!(
        src.contains("tops = attach_rects(tops, def_route_rects("),
        "RECT-carrying wires must be re-encoded with their patches, not refused wholesale"
    );
    assert!(
        src.contains("enc.addRect(o[1], o[2], o[3], o[4])") && src.contains("would be DROPPED"),
        "encode_wire must write each patch, and refuse loudly rather than drop one"
    );
    assert!(
        src.contains("not s.get(\"rect\")"),
        "a segment carrying a patch must never be picked for a bridge"
    );
    assert!(
        src.contains("if self_notch(encl, layer_spacing(tech, l)):"),
        "the bridge's own two enclosures must be checked against each other"
    );
    assert!(
        src.contains("widths = bridge_widths(window_floor, widths)"),
        "the window ladder must be lifted to the via-derived floor"
    );
}

/// Every piece a bridge leaves meets the layer's MINIMUM AREA, and the bridge layer is checked by the
/// bridge's REAL shapes against foreign metal at its true width.
///
/// MEASURED (golden sky130A_mr, Magic silent on both):
/// * `fabric_horner_seq` net580 (RUN_2026.09.24_00.25.18) — a 0.229 um^2 met3 island left between the
///   bridge via and the net's own M2M3 via; sky130 met3 minimum area is 0.24 (m3.6);
/// * `coproc_core` R[1][20] (RUN_2026.09.24_00.26.17) — an M2M3 met3 enclosure (+/-165 nm) 195 nm from
///   a foreign 300 nm met3 wire against a 300 nm rule (m3.2 x2): the window test only covered +/-100.
#[test]
fn bridges_leave_no_under_area_island_and_clear_foreign_metal_by_their_real_shapes() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_notch_guard.py");
    let body = std::fs::read_to_string(&t).unwrap();
    assert!(
        body.contains("(657405, 684255, 657735, 684585)")
            && body.contains("(785285, 638525, 785615, 638855)"),
        "the unit test lost the MEASURED horner island / coproc enclosure"
    );
    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .unwrap();
    assert!(
        src.contains("def union_area(")
            && src.contains("def connected_area(")
            && src.contains("def min_area_dbu2("),
        "the area helpers are gone"
    );
    assert!(
        src.contains("if connected_area(piece, own_rest[layer]) < amin_l:")
            && src.contains("return connected_area(bnew, own_rest[blayer]) < amin_b"),
        "both leftover stubs and the bridge itself must meet the layer's minimum area"
    );
    assert!(src.contains("bfor = collect_foreign(block, region, blayer, tn.getConstName(), bhalf, layer_spacing(tech, blayer))")
            && src.contains("if any(overlaps(e, f) for e in bnew for f in bfor):"),
        "the bridge layer must be checked by its real enclosures and wire against foreign metal at true width");
}

/// The metal INSIDE placed cells is foreign metal, clearance is measured the way DRC measures it,
/// and an ECO tie never notches itself.
///
/// MEASURED on `systolic_pipe16_realbist` (RUN_2026.09.24_06.10.32) and `systolic_os_pipe8_realbist`
/// (RUN_2026.09.24_08.09.31), every golden `sky130A_mr` violation of both blocks — 27 and 1, Magic
/// silent on all of them:
/// * ECO diode ties ran 30 nm from sky130 flop met1 OBSTRUCTIONS (`dfxtp_2` carries two met1 wires
///   across its width) that no routed or special wire describes — m1.2 plus m1.7 enclosed holes;
/// * a jog bridge's M1M2 enclosure landed 95 nm from a `dfxtp_1` OBS bump — the same blind spot;
/// * short-L ties notched THEMSELVES: their L1M1 and M1M2 enclosures 95 nm apart, and the L1M1
///   enclosure 15 nm from the tie's own next leg.
/// Replayed against the real routed database, the checks refuse all 10 golden-flagged ties and
/// accept the other 55, and refuse 0 of the router's 156 DRC-clean access vias — the calibration
/// that took the collector's growth from half-width-plus-spacing to exact.
#[test]
fn cell_metal_is_foreign_clearance_is_euclidean_and_ties_never_notch_themselves() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_cell_geometry_guard.py");
    let body = std::fs::read_to_string(&t).expect("read test_cell_geometry_guard.py");
    for measured in [
        "(1413230, 1080620, 1417190, 1080760)",
        "(634340, 1573520), (634110, 1573520), (634110, 1573860)",
        "(1245725, 711180, 1246015, 711225)",
    ] {
        assert!(
            body.contains(measured),
            "the unit test lost the MEASURED geometry {measured}"
        );
    }
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run the cell-geometry guard unit test");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: cell geometry guard correct"),
        "{log}"
    );

    let jog = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .unwrap();
    let cf = &jog[jog.find("def collect_foreign(").expect("collect_foreign")..];
    let cf = &cf[..cf.find("\ndef ").unwrap_or(cf.len())];
    assert!(
        cf.contains("cell_rects(block,"),
        "collect_foreign must include the metal inside placed cells"
    );
    assert!(
        cf.contains("rects.append((x0 - spacing, y0 - spacing, x1 + spacing, y1 + spacing))"),
        "a foreign via enclosure is a real extent: it grows by the spacing rule alone"
    );
    assert!(
        jog.contains("and not gap_filled(a, b, rects)"),
        "a gap the shape's own metal fills is not a notch — self_notch must not refuse it"
    );

    let eco = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    assert!(
        eco.contains("tie_self_notches(tech, pts, land if tl == \"met2\" else None)"),
        "the ECO must refuse a tie that notches itself, landing enclosure included"
    );
    assert!(
        eco.contains("conflicts(r, f, sp)") && eco.contains("conflicts(e, f, sp1)"),
        "tie and landing clearance must be measured as DRC measures it (Euclidean at corners)"
    );
}

/// Every clearance query reads a spatial index of the nets, never a walk of all of them, and the index follows
/// every wire the ECO rewrites. MEASURED on `wpe_user_project` (RUN_2026.09.27_18.14.14): the first jog pass over 342
/// violating gates took 21 hours, each candidate segment walking all ~310k nets three or four times; on the same
/// run's second pass (50 gates, 33-eco_jog_1.odb) the indexed router took 81 s against the original's 1,264 s and
/// wrote a 349 MB DEF byte-identical to the original's and to the flow's own 34-eco_jog_2.def.
#[test]
fn every_clearance_query_reads_the_net_index() {
    let t = root().join("scripts/openlane-overlay/scripts/odbpy/tests/test_net_index.py");
    let out = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&t)
        .current_dir(root())
        .output()
        .expect("run test_net_index.py");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && log.contains("PASS: net index equals the full walk"),
        "{log}"
    );

    let src = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/sat_antenna_jog.py"),
    )
    .unwrap();
    let body = |name: &str| -> String {
        let s = &src[src
            .find(&format!("def {name}("))
            .unwrap_or_else(|| panic!("missing def {name}"))..];
        s[..s[1..].find("\ndef ").map_or(s.len(), |i| i + 1)].to_string()
    };
    for f in ["collect_foreign", "collect_cuts"] {
        let b = body(f);
        assert!(
            b.contains("net_index(block)"),
            "{f} must query the net index"
        );
        assert!(!b.contains("block.getNets()"), "{f} walks every net again");
    }
    assert!(
        body("encode_wire").contains("touch_net(net)"),
        "a re-encoded wire must be re-indexed where it is re-encoded"
    );
    assert!(
        body("bridge_all").contains("net_index(block).by_norm.get(norm(nname))"),
        "the violator's net is looked up, not searched for"
    );
    assert!(
        body("block_key").contains("int(block.this)"),
        "the index is keyed by the C++ block, not by a proxy's id()"
    );
    let eco = std::fs::read_to_string(
        root().join("scripts/openlane-overlay/scripts/odbpy/magic_antenna_eco.py"),
    )
    .unwrap();
    let tie = &eco[eco.find("def apply_tie(").unwrap()..];
    let tie = &tie[..tie[1..].find("\ndef ").unwrap() + 1];
    assert!(
        tie.contains("touch_net(net)"),
        "a diode tie rewrites the net's wire, so it must re-index it"
    );
}
