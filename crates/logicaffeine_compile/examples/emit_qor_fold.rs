//! emit_qor_fold — the `qorsyn_<name>` wrapper every candidate of the comparative-synthesis bench is
//! synthesised and placed and routed in (out of context: its inputs from registers, its outputs folded
//! into a register, the candidate kept whole), and the designs measured without a harness contract (K7,
//! the DDR2 memory controllers, whose functional gate is the K7 traffic gate).
//!
//!     cargo run -p logicaffeine-compile --example emit_qor_fold -- rtl ddr2_ctrl|ddr2_dfi_ctrl|dram_executor|infinichip <out.v>
//!     cargo run -p logicaffeine-compile --example emit_qor_fold -- wrap <name> <top> <clock> <reset> <high|low|none> <ports.json> <out.v>
//!     cargo run -p logicaffeine-compile --example emit_qor_fold -- wrap-pass <name> <top> <clock> <reset> <high|low|none> <ports.json> <out.v>
//!     cargo run -p logicaffeine-compile --example emit_qor_fold -- wrap-candidate <candidate.json> <out.v> [<name>]
//!
//! `rtl ddr2_ctrl` writes the LOGOS DDR2 controller core as the K7 traffic gate drives it, at the Nexys A7
//! part's datasheet minimums in DDR2-400 clocks (`ddr2_traffic::k7_ddr2_ctrl_verilog`); `rtl ddr2_dfi_ctrl` the
//! LOGOS controller at the DFI 1:2 rate, the same rules (`ddr2_traffic::k7_dfi_ctrl_verilog`); `rtl dram_executor`
//! the planned DRAM command player (its schedule fixed before the run, issued on the cycle the plan names);
//! `rtl infinichip` the Infinichip memory subsystem, the command player with SECDED(72,64) ECC on its data
//! path and a double buffer. `wrap` reads the design's ports as yosys `write_json` lists them
//! (`{"modules": {top: {"ports": ...}}}`) and writes the wrapper. `wrap-pass` writes, from the same ports,
//! the power block `<name>` a K7 controller is hardened as for its gate-level traffic run: every port passed
//! through, its clock and reset as `clk` and `rst`. `wrap-candidate` wraps a harnessed candidate as `wrap`
//! does, from the record emit_qor_harness writes beside it (its top, clock, reset and declared ports): the
//! out-of-context form every candidate is placed and routed in, its function gated in the harness. The
//! wrapper is `qorsyn_<name>`, `<name>` the candidate's own unless given — a Sky130 attempt names the clock
//! it is run at.

use logicaffeine_compile::codegen_sva::ddr2_traffic::{k7_ddr2_ctrl_verilog, k7_dfi_ctrl_verilog};
use logicaffeine_compile::codegen_sva::dram::emit_dram_executor_verilog;
use logicaffeine_compile::codegen_sva::infinichip_rtl::emit_infinichip_verilog;
use logicaffeine_compile::codegen_sva::qor_harness::{emit_fold_wrapper, emit_pass_wrapper, Candidate, Dir, DutPort, Reset};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["rtl", "ddr2_ctrl", out] => write(out, &k7_ddr2_ctrl_verilog()),
        ["rtl", "ddr2_dfi_ctrl", out] => write(out, &k7_dfi_ctrl_verilog()),
        ["rtl", "dram_executor", out] => write(out, &emit_dram_executor_verilog()),
        ["rtl", "infinichip", out] => write(out, &emit_infinichip_verilog()),
        [kind @ ("wrap" | "wrap-pass"), name, top, clock, reset, polarity, ports, out] => {
            let text = std::fs::read_to_string(ports).unwrap_or_else(|e| fail(&format!("{ports}: {e}")));
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("{ports}: {e}")));
            let list = json["modules"][*top]["ports"]
                .as_object()
                .unwrap_or_else(|| fail(&format!("{ports}: no module {top}")));
            let mut dut: Vec<DutPort> = list
                .iter()
                .map(|(n, p)| DutPort {
                    name: n.clone(),
                    dir: if p["direction"] == "output" { Dir::Out } else { Dir::In },
                    bits: p["bits"].as_array().map(|b| b.len() as u32).unwrap_or(1),
                })
                .collect();
            dut.sort_by(|a, b| a.name.cmp(&b.name));
            let rst = match *polarity {
                "high" => Some(Reset { port: reset.to_string(), active_high: true }),
                "low" => Some(Reset { port: reset.to_string(), active_high: false }),
                "none" => None,
                other => fail(&format!("{other}: the reset is high, low or none")),
            };
            let text = if *kind == "wrap" {
                emit_fold_wrapper(name, top, clock, rst.as_ref(), &dut)
            } else {
                emit_pass_wrapper(name, top, clock, rst.as_ref(), &dut)
            };
            write(out, &text);
        }
        ["wrap-candidate", record, out, rest @ ..] if rest.len() <= 1 => {
            let text = std::fs::read_to_string(record).unwrap_or_else(|e| fail(&format!("{record}: {e}")));
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("{record}: {e}")));
            let c: Candidate = serde_json::from_value(json["candidate"].clone()).unwrap_or_else(|e| fail(&format!("{record}: {e}")));
            let name = rest.first().copied().unwrap_or(c.name.as_str());
            write(out, &emit_fold_wrapper(name, &c.top, &c.clock, c.reset.as_ref(), &c.ports));
        }
        _ => fail("usage: emit_qor_fold rtl ddr2_ctrl|ddr2_dfi_ctrl|dram_executor|infinichip <out.v> | wrap|wrap-pass <name> <top> <clock> <reset> <high|low|none> <ports.json> <out.v> | wrap-candidate <candidate.json> <out.v> [<name>]"),
    }
}

fn write(path: &str, text: &str) {
    std::fs::write(path, text).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
}

fn fail(msg: &str) -> ! {
    eprintln!("emit_qor_fold: {msg}");
    std::process::exit(1);
}
