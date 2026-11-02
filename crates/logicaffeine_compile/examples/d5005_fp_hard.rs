//! Print the engine's f32 multiply or add on the card's floating-point DSP block (`codegen_sva::fp_hard`): the wrapper
//! for a block computing the family member `<member>`, `<full>` deep fully registered and `<least>` with its output
//! register alone, at `<latency>` clocks; with `--stand-in` the simulation's block beside it
//! (`fp_oracle::hard_model_verilog`), so the pair elaborates on its own; with `--ce` the wrapper stepping at an enable.
//! The wrapper's module is `<module>` (`hard` unless given). It exits 2 on a block the wrapper refuses, saying why.
//!
//!     d5005_fp_hard multiply|add <member> <full> <least> <latency> [--module <name>] [--stand-in] [--ce]

use std::process::exit;

use logicaffeine_compile::codegen_sva::fp_hard::{hard_verilog, hard_verilog_ce, Block, Op};
use logicaffeine_compile::codegen_sva::fp_hypotheses::family;
use logicaffeine_compile::codegen_sva::fp_oracle::{hard_model_verilog, Depths};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_fp_hard: {why}");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [op, member, full, least, latency, rest @ ..] = &args[..] else {
        fail("usage: d5005_fp_hard multiply|add <member> <full> <least> <latency> [--module <name>] [--stand-in] [--ce]")
    };
    let op = match op.as_str() {
        "multiply" => Op::Multiply,
        "add" => Op::Add,
        other => fail(format!("multiply or add, not {other}")),
    };
    let semantics = family().into_iter().find(|s| s.name() == *member).unwrap_or_else(|| fail(format!("no member of the family is named {member}")));
    let number = |what: &str, v: &str| -> usize { v.parse().unwrap_or_else(|_| fail(format!("{what}: not a number: {v}"))) };
    let block = Block { semantics, full: number("full", full), least: number("least", least) };
    let latency = number("latency", latency);
    let (mut module, mut stand_in, mut ce) = ("hard".to_string(), false, false);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--module" => module = it.next().unwrap_or_else(|| fail("--module takes a name")).clone(),
            "--stand-in" => stand_in = true,
            "--ce" => ce = true,
            other => fail(format!("unknown argument {other}")),
        }
    }
    let wrapper = if ce { hard_verilog_ce(op, &module, &block, latency) } else { hard_verilog(op, &module, &block, latency) };
    print!("{}", wrapper.unwrap_or_else(|e| fail(e)));
    if stand_in {
        print!("\n{}", hard_model_verilog(&semantics, &semantics, Depths { full: [block.full; 3], least: [block.least; 2] }));
    }
}
