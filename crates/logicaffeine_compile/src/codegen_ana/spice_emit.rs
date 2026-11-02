//! Behavioral ngspice emission — the analog lane's "iverilog tier".
//!
//! The emitted netlist is a BEHAVIORAL macromodel: each tile is a controlled source computing
//! its ideal volt-domain equation (Kirchhoff sum = weighted B-source, the four-quadrant
//! multiplier = `V(a)·V(b)/Vref`, constant gain = `k·V(a)`), inputs are DC sources at the block
//! scale, and the operating point is the settled plane. No transistor models are invented —
//! the sky130 transistor tier arrives PDK-gated with real device cards under the same node
//! names. Structural honesty: simulator tolerances are PINNED IN THE NETLIST (`.options`), and
//! the comparison term a test may add derives from those pinned values, never hand-tuning.

use super::cell::{ASrc, AnalogCell};
use super::gain::GainSchedule;
use super::target::AnalogTarget;

/// The pinned simulator options — tests read these back from the emitted netlist to derive
/// their simulator-discretization term.
pub const SPICE_RELTOL: f64 = 1.0e-4;
pub const SPICE_ABSTOL: f64 = 1.0e-12;
pub const SPICE_VNTOL: f64 = 1.0e-9;

/// Emit a DC operating-point netlist for one input point: sources at the block scale, one
/// behavioral element per tile, the output node written via `wrdata`.
pub fn emit_spice_dc(
    schedule: &GainSchedule,
    target: &AnalogTarget,
    inputs: &[f64],
    out_path: &str,
) -> String {
    let netlist = &schedule.quantized;
    assert_eq!(inputs.len(), netlist.n_in);
    let s = f64::powi(2.0, schedule.scale_log2);
    let mut o = String::new();
    o.push_str("* codegen_ana behavioral DC netlist (controlled-source macromodel)\n");
    o.push_str(&format!(
        "* target: {} | block scale 2^{} V/math | Vref {} V\n",
        target.name, schedule.scale_log2, schedule.mul_vref_v
    ));
    o.push_str(&format!(
        ".options reltol={SPICE_RELTOL} abstol={SPICE_ABSTOL} vntol={SPICE_VNTOL}\n"
    ));
    for (k, x) in inputs.iter().enumerate() {
        o.push_str(&format!("VIN{k} in{k} 0 DC {}\n", x * s));
    }
    let node = |src: &ASrc| -> String {
        match src {
            ASrc::In(k) => format!("in{k}"),
            ASrc::Val(v) => format!("n{v}"),
            ASrc::Const(c) => format!("({})", c * s), // a programmed reference at the block scale
        }
    };
    let volt = |src: &ASrc| -> String {
        match src {
            ASrc::Const(c) => format!("({})", c * s),
            other => format!("V({})", node(other)),
        }
    };
    for (i, cell) in netlist.cells.iter().enumerate() {
        match cell {
            AnalogCell::Exp { a } => {
                o.push_str(&format!("BEXP{i} n{i} 0 V = exp({})\n", volt(a)));
            }
            AnalogCell::Log { a } => {
                o.push_str(&format!("BLOG{i} n{i} 0 V = ln({})\n", volt(a)));
            }
            AnalogCell::Tanh { a } => {
                o.push_str(&format!("BTANH{i} n{i} 0 V = tanh({})\n", volt(a)));
            }
            AnalogCell::Sqrt { a } => {
                o.push_str(&format!("BSQRT{i} n{i} 0 V = sqrt({})\n", volt(a)));
            }
            AnalogCell::Div { a, b } => {
                o.push_str(&format!(
                    "BDIV{i} n{i} 0 V = ({}) / ({})\n",
                    volt(a),
                    volt(b)
                ));
            }
            AnalogCell::Select { c, t, f } => {
                o.push_str(&format!(
                    "BSEL{i} n{i} 0 V = (({}) < 0) ? ({}) : ({})\n",
                    volt(c),
                    volt(t),
                    volt(f)
                ));
            }
            AnalogCell::Sum {
                inputs: xs,
                weights,
            } => {
                let terms: Vec<String> = xs
                    .iter()
                    .zip(weights)
                    .map(|(x, w)| format!("({w})*{}", volt(x)))
                    .collect();
                o.push_str(&format!("BSUM{i} n{i} 0 V = {}\n", terms.join(" + ")));
            }
            AnalogCell::Mul { a, b } => {
                o.push_str(&format!(
                    "BMUL{i} n{i} 0 V = {}*{}/({})\n",
                    volt(a),
                    volt(b),
                    schedule.mul_vref_v
                ));
            }
            AnalogCell::GainConst { a, k } => {
                o.push_str(&format!("BGAIN{i} n{i} 0 V = ({k})*{}\n", volt(a)));
            }
        }
    }
    let out_node = node(&netlist.out);
    o.push_str(".op\n.control\nrun\n");
    o.push_str(&format!("wrdata {out_path} v({out_node})\n"));
    o.push_str("quit\n.endc\n.end\n");
    o
}
