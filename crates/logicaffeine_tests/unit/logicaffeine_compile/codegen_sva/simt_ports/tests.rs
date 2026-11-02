use super::*;
use crate::codegen_sva::mul_tree::mul_tree_levels;

fn engine(body: &str, uniform: Vec<bool>, reduce: Option<AluOp>) -> SimtEngine {
    let src = format!("## To shade (a: Int, x: Int, b: Int) -> Int:\n    Return {body}.\n## Main\n    Show shade(2, 3, 4).\n");
    SimtEngine::from_lane_source(&src, "shade", 4, uniform, true, reduce)
}

fn affine() -> SimtEngine {
    engine("a * x + b", vec![true, false, true], Some(AluOp::Add))
}

/// The step's results are the tree's clocks and two more after its operands, whatever levels the tree is cut at.
#[test]
fn a_step_takes_the_trees_clocks_and_two() {
    let levels = mul_tree_levels(64);
    for (cut, tree) in [
        (Box::new(|_: usize| false) as Box<dyn Fn(usize) -> bool>, 0u32),
        (Box::new(|_: usize| true), levels as u32),
        (Box::new(move |l: usize| (l + 1) % 2 == 0), (levels / 2) as u32),
        (Box::new(move |l: usize| l + 1 == levels), 1),
    ] {
        let p = emit_simt_ports_verilog("qor_k6_ports", &affine(), cut).unwrap();
        assert_eq!(p.latency, tree + 2);
    }
}

/// Every operand is a port and every result a registered output: the uniforms, the mask, each lane's varying in; each
/// lane's output and the reduce out; no register file, no load, no go.
#[test]
fn every_operand_and_result_is_a_port() {
    let v = emit_simt_ports_verilog("qor_k6_ports", &affine(), |_| true).unwrap().verilog;
    let header = v.lines().find(|l| l.starts_with("module ")).unwrap();
    for port in ["input clk", "input [63:0] a", "input [63:0] b", "input [3:0] mask", "input [63:0] x0", "input [63:0] x3",
                 "output reg [63:0] y0", "output reg [63:0] y3", "output reg [63:0] reduced"] {
        assert!(header.contains(port), "{port} missing from {header}");
    }
    for absent in ["din", "load", "go", "raddr", "done"] {
        assert!(!header.contains(absent), "{absent} on a ports engine: {header}");
    }
}

/// Only the kernel it computes, laid out as it reads it, and a reduce that folds as a tree, are taken.
#[test]
fn what_it_does_not_compute_is_refused() {
    let e = emit_simt_ports_verilog("p", &engine("a * x - b", vec![true, false, true], Some(AluOp::Add)), |_| true).unwrap_err();
    assert!(e.contains("a·x + b"), "{e}");
    let e = emit_simt_ports_verilog("p", &engine("a * x + b", vec![true, false, false], Some(AluOp::Add)), |_| true).unwrap_err();
    assert!(e.contains("uniform"), "{e}");
    let e = emit_simt_ports_verilog("p", &engine("a * x + b", vec![true, false, true], Some(AluOp::Sub)), |_| true).unwrap_err();
    assert!(e.contains("Sub"), "{e}");
    for op in [AluOp::Add, AluOp::Or, AluOp::Xor, AluOp::And] {
        assert!(emit_simt_ports_verilog("p", &engine("a * x + b", vec![true, false, true], Some(op)), |_| true).is_ok());
    }
    assert!(emit_simt_ports_verilog("p", &engine("a * x + b", vec![true, false, true], None), |_| true).is_ok());
}
