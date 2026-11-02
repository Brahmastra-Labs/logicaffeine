//! The tiny Qwen3.8 decode step every Spatial-lane lock lowers: token 5 at position 3 against a 6-position KV cache,
//! every declared `Real` tensor filled from its NAME (so the VM, the oracle and every emitted app see the same
//! values), composed on the SN50.

#![allow(dead_code)]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_decode_step_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;

pub const CTX: usize = 6;
pub const TOK: usize = 5;
pub const POS: usize = 3;

/// `qwen38_decode_step_lock`'s fill: after every `Let V be a R by C tensor of Real.`, every element of `V` set from
/// a hash of its index and `V`'s name.
pub fn with_fills(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            out.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    out
}

pub fn target() -> SpatialTarget {
    RduChip::Sn50.facts().spatial_target()
}

/// The filled source and its composed, certified program.
pub fn decode_step() -> (String, SpatialProgram) {
    let src = with_fills(&gen_qwen38_decode_step_lg(&Qwen38Cfg::tiny(), "/w", CTX, TOK, POS).0);
    let pl = compile_program_to_spatial(&src, &target()).expect("the walk lifts the step");
    let p = pl
        .whole()
        .unwrap_or_else(|| panic!("the decode step lowers whole: {}", pl.refusal()))
        .clone();
    (src, p)
}
