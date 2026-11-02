//! The operand pairs a hard binary32 block is measured on, and the table that names the semantics it computes: for the
//! multiply and the add, the hypothesis family (`codegen_sva::fp_hypotheses::family`) told apart by
//! `logicaffeine_proof::sweep::distinguish`, seeded with every pair of `fp_hypotheses::SPECIALS`. The members that
//! compute one function are one class, each equality certified; `cover` holds pairs on which every two classes give
//! different results, and each class's `results` are its results there. Written to
//! `benchmarks/silicon/d5005/fp_discriminators.json`: the oracle runs every pair of `vectors`, and the block's results
//! on `cover` are one class's `results`, or the block computes no member of the family.
//!
//!     cargo run --release -p logicaffeine-compile --example fp_discriminators

use std::path::PathBuf;

use serde::Serialize;

use logicaffeine_compile::codegen_sva::fp_hypotheses::{fadd_verilog, family, fmul_verilog, Semantics, SPECIALS};
use logicaffeine_compile::codegen_sva::netlist::{synthesize_reading, Undefined};
use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::sweep::{distinguish, Distinction, Options};

#[derive(Serialize)]
struct Class {
    /// Its members' semantics, by name.
    members: Vec<String>,
    /// Its result on each pair of `cover`.
    results: Vec<String>,
}

#[derive(Serialize)]
struct Op {
    op: &'static str,
    /// How many of `vectors` are the special pairs; the rest were found to split two members.
    seeds: usize,
    vectors: Vec<[String; 2]>,
    cover: Vec<[String; 2]>,
    classes: Vec<Class>,
}

/// A member's circuit, a bit its source leaves undefined read as 0: one definite function to tell apart.
fn aig(verilog: &str) -> Aig {
    synthesize_reading(verilog, "h", &[], Undefined::Zero).unwrap_or_else(|e| panic!("{e}")).aig
}

/// The word an assignment gives the ports named `{port}[0]` to `{port}[31]`.
fn word(names: &[String], bits: &[bool], port: &str) -> u32 {
    (0..32).fold(0, |acc, i| {
        let at = names.iter().position(|n| *n == format!("{port}[{i}]")).unwrap_or_else(|| panic!("no {port}[{i}]"));
        acc | (bits[at] as u32) << i
    })
}

fn pair(d: &Distinction, v: usize) -> [String; 2] {
    [format!("{:08x}", word(&d.inputs, &d.vectors[v], "a")), format!("{:08x}", word(&d.inputs, &d.vectors[v], "b"))]
}

fn main() {
    let ops: [(&'static str, fn(&str, &Semantics) -> String); 2] = [("fmul", fmul_verilog), ("fadd", fadd_verilog)];
    let semantics = family();
    let seeds: Vec<Vec<(String, bool)>> = SPECIALS
        .iter()
        .flat_map(|&a| SPECIALS.iter().map(move |&b| (a, b)))
        .map(|(a, b)| (0..32).map(|i| (format!("a[{i}]"), a >> i & 1 == 1)).chain((0..32).map(|i| (format!("b[{i}]"), b >> i & 1 == 1))).collect())
        .collect();
    let mut out = Vec::new();
    for (op, generate) in ops {
        let members: Vec<Aig> = semantics.iter().map(|s| aig(&generate("h", s))).collect();
        let d = distinguish(&members, &seeds, 0, &Options::default()).unwrap_or_else(|e| panic!("{op}: {e}"));
        assert!(d.check(&members), "{op}: the distinction checks");
        let classes: Vec<Class> = d
            .classes
            .iter()
            .zip(&d.table)
            .map(|(class, rows)| Class {
                members: class.iter().map(|&m| semantics[m].name()).collect(),
                results: rows.iter().map(|y| format!("{:08x}", word(&d.outputs, y, "y"))).collect(),
            })
            .collect();
        println!(
            "{op}: {} members in {} classes, {} vectors ({} found past the {} special pairs), told apart on {}",
            members.len(),
            classes.len(),
            d.vectors.len(),
            d.vectors.len() - seeds.len(),
            seeds.len(),
            d.cover.len()
        );
        for c in classes.iter().filter(|c| c.members.len() > 1) {
            println!("  one function: {}", c.members.join(", "));
        }
        out.push(Op {
            op,
            seeds: seeds.len(),
            vectors: (0..d.vectors.len()).map(|v| pair(&d, v)).collect(),
            cover: d.cover.iter().map(|&v| pair(&d, v)).collect(),
            classes,
        });
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    let mut text = serde_json::to_string_pretty(&out).expect("serializes");
    text.push('\n');
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}
