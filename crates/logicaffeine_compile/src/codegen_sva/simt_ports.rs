//! simt_ports — the SIMT engine's step on ports: its uniforms, its mask and every lane's varying as inputs, every
//! lane's output and the warp-reduce as outputs, a step every clock. Where the engine's register file
//! ([`emit_gpu_simt_verilog`](super::gpu_simt_rtl::emit_gpu_simt_verilog)) takes a step's operands a slot a clock
//! and hands its results back a read at a time, this takes them all at once, as a caller holding them in registers
//! presents them, and reads no register of its own on the way in.
//!
//! Each lane of the `a·x + b` kernel computes `a·x` as an aligned tree of adds ([`mul_tree`]), registered after the
//! levels `cut` names, while `b` and the mask ride registers beside it. The clock after the product, each lane's
//! masked `a·x + b` is registered (a masked lane yields 0); the clock after that, the warp-reduce of those is
//! registered and every lane output with it. A step's results are on the outputs `tree latency + 2` clocks after its
//! operands.

use super::fabric_rtl::AluOp;
use super::gpu_simt_rtl::{computes_affine_fma, SimtEngine};
use super::mul_tree::mul_tree;

/// A ports engine's RTL and the clocks from a step's operands to its results.
#[derive(Clone, Debug, PartialEq)]
pub struct SimtPorts {
    pub verilog: String,
    pub latency: u32,
}

/// Emit `eng`'s step on ports as module `name`: inputs `clk`, `a`, `b`, `mask` (when the engine is predicated) and
/// `x0..`, outputs `y0..` and `reduced` (when it reduces). Refused unless the lane kernel computes `a·x + b` with `a`
/// and `b` uniform and `x` per lane, and its reduce, if any, is an Add, Or, Xor or And: those fold a tree of lanes
/// to the same word as the engine's fold from the op's identity.
pub fn emit_simt_ports_verilog(name: &str, eng: &SimtEngine, cut: impl Fn(usize) -> bool) -> Result<SimtPorts, String> {
    if !computes_affine_fma(eng) {
        return Err(format!("{name}: the ports engine computes the a·x + b kernel; this one computes something else"));
    }
    if eng.uniform != [true, false, true] {
        return Err(format!("{name}: a and b are uniform and x is per lane, not {:?}", eng.uniform));
    }
    let fold = match eng.reduce {
        None => None,
        Some(op @ (AluOp::Add | AluOp::Or | AluOp::Xor | AluOp::And)) => Some(op),
        Some(op) => return Err(format!("{name}: a warp-reduce of {op:?} does not fold as a tree of lanes")),
    };
    let n = eng.n_lanes;
    let trees: Vec<_> = (0..n).map(|j| mul_tree(&format!("m{j}"), "a", &format!("x{j}"), 64, &cut)).collect();
    let lat = trees[0].latency as usize;

    let mut ports = vec!["input clk".to_string(), "input [63:0] a".to_string(), "input [63:0] b".to_string()];
    if eng.predicated {
        ports.push(format!("input [{}:0] mask", n - 1));
    }
    ports.extend((0..n).map(|j| format!("input [63:0] x{j}")));
    ports.extend((0..n).map(|j| format!("output reg [63:0] y{j}")));
    if fold.is_some() {
        ports.push("output reg [63:0] reduced".to_string());
    }

    let mut decls = String::new();
    let mut moves = String::new();
    for t in &trees {
        for (r, bits) in &t.regs {
            decls.push_str(&format!("  reg [{}:0] {r};\n", bits - 1));
        }
        for (r, v) in &t.moves {
            moves.push_str(&format!("    {r} <= {v};\n"));
        }
    }
    for d in 1..=lat {
        decls.push_str(&format!("  reg [63:0] b_d{d};\n"));
        moves.push_str(&format!("    b_d{d} <= {};\n", if d == 1 { "b".to_string() } else { format!("b_d{}", d - 1) }));
        if eng.predicated {
            decls.push_str(&format!("  reg [{}:0] mask_d{d};\n", n - 1));
            moves.push_str(&format!(
                "    mask_d{d} <= {};\n",
                if d == 1 { "mask".to_string() } else { format!("mask_d{}", d - 1) }
            ));
        }
    }
    let b_at = if lat == 0 { "b".to_string() } else { format!("b_d{lat}") };
    let mask_at = if lat == 0 { "mask".to_string() } else { format!("mask_d{lat}") };
    let mut wires = String::new();
    for (j, t) in trees.iter().enumerate() {
        wires.push_str(&t.wires);
        decls.push_str(&format!("  reg [63:0] lane{j};\n"));
        let k = format!("{} + {b_at}", t.product);
        let kept = if eng.predicated { format!("{mask_at}[{j}] ? {k} : 64'd0") } else { k };
        moves.push_str(&format!("    lane{j} <= {kept};\n"));
        moves.push_str(&format!("    y{j} <= lane{j};\n"));
    }
    if let Some(op) = fold {
        let sep = match op {
            AluOp::Add => " + ",
            AluOp::Or => " | ",
            AluOp::Xor => " ^ ",
            _ => " & ",
        };
        let lanes: Vec<String> = (0..n).map(|j| format!("lane{j}")).collect();
        moves.push_str(&format!("    reduced <= {};\n", lanes.join(sep)));
    }
    let verilog = format!(
        "// {name} — the SIMT engine's step on ports ({n} lanes of a·x + b{masked}{reduce}): every operand in at once,\n\
         // each lane's a·x an aligned tree of adds {lat} clock(s) deep, its masked a·x + b registered, then the lane\n\
         // outputs and the warp-reduce registered together — a step every clock, its results {latency} clocks later.\n\
         module {name}({ports});\n\
         {decls}{wires}  always @(posedge clk) begin\n\
         {moves}  end\n\
         endmodule\n",
        masked = if eng.predicated { ", masked" } else { "" },
        reduce = fold.map_or(String::new(), |op| format!(", {op:?}-reduced")),
        latency = lat + 2,
        ports = ports.join(", "),
    );
    Ok(SimtPorts { verilog, latency: lat as u32 + 2 })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/simt_ports/tests.rs"]
mod tests;
