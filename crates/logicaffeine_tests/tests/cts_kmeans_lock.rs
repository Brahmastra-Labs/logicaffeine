//! cts_kmeans_lock — every clock tree the overlay builds hands TritonCTS a k-means exponent whose min-cost flow stays
//! within a signed int.
//!
//! Each H-tree level splits its sinks between two branch points with LEMON's `NetworkSimplex<int, int>`, arc cost
//! = distance^exponent in TritonCTS's wire unit. LEMON's integer simplex gives its artificial arc the cost
//! `ART = i32::MAX / 2 + 1` and keeps every node's potential the signed sum of the costs on its tree path. With two
//! branch points a path crosses at most four sink arcs, so the dearest reduced cost it evaluates is `ART + 9C`, `C` the
//! dearest arc. Past `9C ≥ ART` the reduced costs wrap and the pivoting never ends: the WPE macro's CTS spun 22 h at
//! the H-tree's second level at TritonCTS's default exponent 4 (RUN_2026.10.18_19.41.51).
//!
//! `logos_cts_kmeans.tcl` picks the largest exponent up to 4 the core keeps in range. This lock holds:
//! - its own tests pass under tclsh, the odb wrapper among them on stand-in objects;
//! - its bound is LEMON's, re-derived here: the overlay's `art` is `i32::MAX / 2 + 1`, and at every span its exponent
//!   is the largest with `9 d^e < ART`, `d` the span up to the 5200 wire units `Clustering.cpp` keeps arcs within;
//! - `cts.tcl` passes that exponent on every flow, whatever the fanout setting, before it builds the tree;
//! - the overlay's NOTICE declares both files.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test cts_kmeans_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn overlay(rel: &str) -> String {
    std::fs::read_to_string(root().join("scripts/openlane-overlay").join(rel)).unwrap_or_else(|e| panic!("read the overlay's {rel}: {e}"))
}

/// LEMON's `ART_COST` for an exact `Cost` type: `numeric_limits<int>::max() / 2 + 1`.
const ART: i64 = i32::MAX as i64 / 2 + 1;
/// `Clustering::minCostFlow`'s distance cap: no arc longer than this many wire units is kept.
const REACH: i64 = 5200;

/// The largest exponent up to 4 whose dearest arc on a core `span` wire units across keeps `9C` under `ART`.
fn safe_exponent(span: i64) -> u32 {
    let d = span.clamp(0, REACH);
    (2..=4).rev().find(|e| 9 * d.pow(*e) < ART).expect("squares hold to the reach")
}

/// Runs `script` under tclsh with the overlay's k-means file sourced, and returns its stdout.
fn tcl(script: &str) -> String {
    let src = root().join("scripts/openlane-overlay/scripts/openroad/logos_cts_kmeans.tcl");
    let out = std::process::Command::new("tclsh")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            let mut stdin = c.stdin.take().unwrap();
            writeln!(stdin, "source {{{}}}", src.display())?;
            stdin.write_all(script.as_bytes())?;
            drop(stdin);
            c.wait_with_output()
        })
        .expect("run tclsh");
    assert!(out.status.success(), "tclsh: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("tclsh prints text")
}

#[test]
fn the_kmeans_core_passes_its_own_tests() {
    if !common::require_tool("tclsh") {
        return;
    }
    let t = root().join("scripts/openlane-overlay/scripts/openroad/tests/test_cts_kmeans.tcl");
    let out = std::process::Command::new("tclsh").arg(&t).current_dir(root()).output().expect("run tclsh");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && log.contains("PASS: H-tree k-means exponent"), "{log}");
}

/// The overlay's constants are LEMON's and TritonCTS's, and its exponent is the largest safe one at every span from one
/// wire unit to well past the reach, computed here independently of the Tcl.
#[test]
fn the_exponent_is_the_largest_lemon_keeps_in_int() {
    if !common::require_tool("tclsh") {
        return;
    }
    let consts = tcl("puts $logos_cts_kmeans::art\n");
    assert_eq!(consts.trim(), ART.to_string(), "the overlay's ART_COST");
    assert!(9 * REACH.pow(2) < ART, "squares hold to the reach, so the exponent never falls below 2");
    let spans: Vec<i64> = (0..=1200).chain((1201..=12000).step_by(7)).chain([5199, 5200, 5201, 100_000]).collect();
    let script: String = spans.iter().map(|s| format!("puts [logos_cts_kmeans::exponent {s}]\n")).collect();
    let got: Vec<u32> = tcl(&script).lines().map(|l| l.trim().parse().expect("an exponent")).collect();
    assert_eq!(got.len(), spans.len());
    for (s, e) in spans.iter().zip(&got) {
        assert_eq!(*e, safe_exponent(*s), "span {s}");
        let d = (*s).clamp(0, REACH);
        assert!(9 * d.pow(*e) < ART, "span {s}: exponent {e} keeps 9C under ART");
        assert!(*e == 4 || 9 * d.pow(*e + 1) >= ART, "span {s}: exponent {e} is the largest that does");
    }
    assert!(got.contains(&4) && got.contains(&3) && got.contains(&2), "the sweep crosses both thresholds: {got:?}");
}

/// TritonCTS's own wire unit (`TechChar.cpp`: the buffer's height × 10 / 2, in whole microns) and the span in it, rounded
/// up so the bound never undercounts the core.
#[test]
fn the_span_is_the_core_in_tritoncts_wire_unit_rounded_up() {
    if !common::require_tool("tclsh") {
        return;
    }
    let out = tcl(
        "puts [logos_cts_kmeans::wire_unit_um 2720 1000]\n\
         puts [logos_cts_kmeans::wire_unit_um 2720 2000]\n\
         puts [logos_cts_kmeans::span 2788520 3337440 13 1000]\n\
         puts [logos_cts_kmeans::span 13000 13000 13 1000]\n\
         puts [logos_cts_kmeans::span 13000 13001 13 1000]\n",
    );
    let v: Vec<i64> = out.lines().map(|l| l.trim().parse().unwrap()).collect();
    assert_eq!(v, vec![2720 * 10 / 2 / 1000, 2720 * 10 / 2 / 2000, (2788520 + 3337440 + 12999) / 13000, 2, 3]);
    assert_eq!(v[2], 472, "the WPE's core, as its replayed CTS printed it");
    assert_eq!(safe_exponent(v[2]), 3, "the WPE takes 3");
}

/// `cts.tcl` passes the exponent on every flow — outside the fanout setting's branch — and before the tree is built.
#[test]
fn cts_hands_tritoncts_the_exponent_its_core_keeps_in_int() {
    let cts = overlay("scripts/openroad/cts.tcl");
    let source = cts.find("source $::env(SCRIPTS_DIR)/openroad/logos_cts_kmeans.tcl").expect("cts.tcl sources the k-means file");
    let pass = cts.find("lappend arg_list -clustering_exponent [logos_cts_kmeans_exponent]").expect("cts.tcl passes its exponent");
    let build = cts.find("clock_tree_synthesis {*}$arg_list").expect("the tree is built");
    assert!(source < pass && pass < build, "sourced, passed, then the tree built");
    let fanout = cts.find("if { ![info exists ::env(LOGOS_CTS_FANOUT)]").expect("the fanout setting's branch");
    let branch_end = fanout + cts[fanout..].find("\n}\n").expect("the branch closes");
    assert!(pass > branch_end, "the exponent is passed whatever LOGOS_CTS_FANOUT says");
    assert_eq!(cts.matches("-clustering_exponent").count(), 1, "one exponent, the overlay's");
    let lib = overlay("scripts/openroad/logos_cts_kmeans.tcl");
    for read in ["getCoreArea", "getHeight", "CTS_CLK_BUFFER_LIST", "CTS_ROOT_BUFFER", "getDbUnitsPerMicron"] {
        assert!(lib.contains(read), "the exponent reads {read}");
    }
}

#[test]
fn the_notice_declares_both_files() {
    let notice = overlay("NOTICE");
    for f in ["scripts/openroad/logos_cts_kmeans.tcl", "scripts/openroad/tests/test_cts_kmeans.tcl"] {
        assert!(notice.lines().any(|l| l.trim() == f), "NOTICE declares {f}");
    }
}
