#![cfg(feature = "handed-reds")]
//! fabric_author_model_lock — the boards run a program `model::author` writes.
//!
//! FABRIC-AUTHOR-MODEL's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20; ONE-PROGRAM-RETIRE-SECOND-WRITERS).
//! The board images and the GPU programs of the int8 baby transformer are generated from `Int8Model::decode_step_lg`,
//! a writer beside the one program. Once the author writes the integer knob point (AUTHOR-INTEGER-POINT), the
//! generators read its cut instead, and no product source calls the second writer.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_board_image_or_gpu_program_comes_from_a_second_writer() {
    let mut files = Vec::new();
    for dir in ["crates/logicaffeine_compile/src", "crates/logicaffeine_compile/examples", "apps"] {
        rust_files(&root().join(dir), &mut files);
    }
    let callers: Vec<String> = files
        .iter()
        .filter(|p| !p.ends_with("spatial/int8_model.rs"))
        .filter(|p| std::fs::read_to_string(p).unwrap_or_default().contains(".decode_step_lg("))
        .map(|p| p.strip_prefix(root()).unwrap_or(p).display().to_string())
        .collect();
    assert!(
        callers.is_empty(),
        "these product sources run the int8 model's own writer, not the one program's cut:\n{}",
        callers.join("\n")
    );
}
