//! `largo emit rdu` over the demo corpus `examples/rdu_demo/` — every program lowers through ONE front door
//! and the lane it took is printed by name on stderr (`lane: tensor|loop|whole-program`), a function of the
//! program's shape (`spatial::lower::rdu_lanes::lane_of`).
//!
//! The whole Qwen3.8 tiny forward names its lane today; lowering the whole model is Part 2's front end
//! (`compile_program_to_spatial`) — `QWEN38_TINY_LOWERS` flips to `true` when it lands, and until then the
//! typed refusal it earns is asserted instead (never a panic, never a silent success).

#[path = "cli_common/mod.rs"]
mod common;

use std::path::PathBuf;

use common::*;
use tempfile::tempdir;

/// Flip when the tensor lane composes the whole authored forward (Part 2, B-P7).
const QWEN38_TINY_LOWERS: bool = true;

fn demo(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/rdu_demo")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("examples/rdu_demo/{name}: {e}"))
}

#[test]
fn every_demo_program_lowers_and_names_its_lane() {
    let corpus: [(&str, &str, bool); 6] = [
        ("dot.lg", "loop", true),
        ("fib.lg", "whole-program", true),
        ("primes.lg", "whole-program", true),
        ("gemm.lg", "tensor", true),
        ("rmsnorm.lg", "tensor", true),
        ("qwen38_tiny.lg", "tensor", QWEN38_TINY_LOWERS),
    ];
    for (file, lane, lowers) in corpus {
        let dir = tempdir().unwrap();
        let path = demo(file);
        let out = largo_in(
            dir.path(),
            &["emit", "rdu", "--chip", "sn50", path.to_str().unwrap()],
        );
        let err = strip_ansi(&stderr(&out));
        assert!(
            err.contains(&format!("lane: {lane}")),
            "{file}: stderr names its lane `{lane}`:\n{err}"
        );
        assert!(!err.contains("panicked"), "{file}: never a panic:\n{err}");
        if lowers {
            assert_eq!(
                out.status.code(),
                Some(0),
                "{file} lowers on the {lane} lane: {err}"
            );
            assert!(
                !stdout(&out).trim().is_empty(),
                "{file}: the lowering prints its artifact"
            );
        } else {
            assert_ne!(
                out.status.code(),
                Some(0),
                "{file}: not lowered today — a typed refusal, not a silent success"
            );
            assert!(
                err.contains("rdu backend:"),
                "{file}: the refusal is typed:\n{err}"
            );
        }
    }
}
