//! `largo emit plasticine` — the LOGOS → Plasticine (open RDU) bundle: Spatial source, the checkerboard
//! placement + metapipeline II model, and the MODELED report row; a typed refusal for anything that is not
//! a counted loop.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use tempfile::tempdir;

const LOOP: &str = "# Affine\n\n## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        Set s to s * 3 + i.\n    Return s.\n\n## Main\n\nShow f(10).\n";

/// Bare `largo emit plasticine FILE` prints the Spatial source.
#[test]
fn emit_plasticine_prints_spatial_source() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("affine.lg");
    std::fs::write(&file, LOOP).unwrap();
    let out = largo_in(dir.path(), &["emit", "plasticine", file.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "emit plasticine: {}",
        stderr(&out)
    );
    let scala = stdout(&out);
    for needle in [
        "import spatial.dsl._",
        "@spatial object Logos_f extends SpatialApp",
        "Accel {",
        "Foreach(",
        "Reg[Long]",
        "s0 := ",
    ] {
        assert!(
            scala.contains(needle),
            "Spatial source must contain {needle:?}:\n{scala}"
        );
    }
}

/// `-o DIR` writes the bundle: Spatial source, placement, report row — and stdout stays quiet.
#[test]
fn emit_plasticine_writes_the_bundle() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("affine.lg");
    std::fs::write(&file, LOOP).unwrap();
    let bundle = dir.path().join("plasticine");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "plasticine",
            file.to_str().unwrap(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "emit plasticine -o: {}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("@spatial"),
        "the source must not also spill to stdout with -o"
    );
    let scala = std::fs::read_to_string(bundle.join("Logos_f.scala")).expect("Logos_f.scala");
    assert!(scala.contains("@spatial object Logos_f"));
    let test =
        std::fs::read_to_string(bundle.join("Logos_f_test.scala")).expect("Logos_f_test.scala");
    assert!(
        test.contains("class Logos_f extends SpatialTest"),
        "the regression-harness flavour:\n{test}"
    );
    assert!(
        test.contains("override def runtimeArgs: Args = \"8\""),
        "the VM anchor's inputs are the runtime args"
    );
    assert!(
        test.contains("PASS: ") && test.contains("assert(cksum)"),
        "it checks the VM's answer on the host"
    );
    let placement = std::fs::read_to_string(bundle.join("placement.json")).expect("placement.json");
    let p: serde_json::Value = serde_json::from_str(&placement).expect("placement parses");
    assert_eq!(p["target"], "plasticine-isca17");
    assert_eq!(p["routed_statically"], true);
    let report = std::fs::read_to_string(bundle.join("report.json")).expect("report.json");
    let r: serde_json::Value = serde_json::from_str(&report).expect("report parses");
    assert_eq!(r["provenance"], "modeled");
    assert_eq!(
        r["pipeline_ii"], 7,
        "s*3+i carried through the six-stage PCU: pipeline depth + 1 (Tungsten-calibrated)"
    );
    assert_eq!(r["vm_anchor"]["hazards_at_ii"], 0);
    assert_eq!(r["vm_anchor"]["vm"], r["vm_anchor"]["pipelined_at_ii"]);
    assert!(
        r["certificate"]["rechecked"].as_bool().unwrap(),
        "an ALU-only body ships a re-checked certificate"
    );
}

/// A program whose first function is not a counted loop is refused with a typed `plasticine backend:` error.
#[test]
fn emit_plasticine_refuses_a_non_loop_program_typed() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("whiley.lg");
    std::fs::write(
        &file,
        "# While\n\n## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most x:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n\n## Main\n\nShow f(10).\n",
    )
    .unwrap();
    let out = largo_in(dir.path(), &["emit", "plasticine", file.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0), "a while loop must be refused");
    let err = stderr(&out);
    assert!(err.contains("plasticine backend:"), "typed refusal: {err}");
    assert!(
        err.contains("counted range loop"),
        "the refusal names what the lane takes: {err}"
    );
}
