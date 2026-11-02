//! Emit the u-wide unrolled int8 MAC CGRA + a pin-light fmax wrapper, for measuring achieved Fmax on the
//! Artix-7 (yosys + nextpnr-xilinx). The wrapper registers the wide in/out to a serial pin so the critical
//! path measured is the CGRA's internal MAC tree (u multiply tiles + adder tree), not I/O.
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, loop_fabric_from_source,
};
use std::fs;

fn dot_unrolled_fn(u: usize) -> String {
    let terms: Vec<String> = (0..u)
        .map(|j| {
            if j == 0 {
                "k * (n - k)".into()
            } else {
                format!("(k + {j}) * (n - k - {j})")
            }
        })
        .collect();
    format!(
        "## To dotU (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + {}.\n    Return s.\n",
        terms.join(" + ")
    )
}

fn main() {
    let dir = "/tmp/fmax";
    fs::create_dir_all(dir).unwrap();
    for u in [8usize, 16, 32, 64, 128] {
        let lf = loop_fabric_from_source(&dot_unrolled_fn(u), "dotU");
        let muls = lf
            .body
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    logicaffeine_compile::codegen_sva::fabric_rtl::LoopCell::Arith {
                        op: logicaffeine_compile::codegen_sva::fabric_rtl::AluOp::Mul,
                        ..
                    }
                )
            })
            .count();
        let rtl = emit_loop_fabric_verilog(&format!("mac{u}"), &lf);
        let wrap = format!(
            "module fmax_top(input clk, input rst, input serin, output reg serout);\n\
            \x20 reg [63:0] in0 = 64'd0;\n  wire [63:0] result; wire done;\n  reg [63:0] acc = 64'd0;\n\
            \x20 cgra_mac{u} dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));\n\
            \x20 always @(posedge clk) begin\n    in0 <= {{in0[62:0], serin}};\n    if (done) acc <= result;\n    serout <= ^acc;\n  end\nendmodule\n"
        );
        fs::write(format!("{dir}/fmax_mac{u}.v"), format!("{rtl}\n{wrap}")).unwrap();
        println!("emitted fmax_mac{u}.v  ({muls} multiply tiles = {u} MACs/clock)");
    }
    // minimal clock+pin constraints (100 MHz target); nextpnr reports the achieved Fmax.
    let xdc = "set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports { clk }]\n\
               create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]\n\
               set_property -dict { PACKAGE_PIN J15 IOSTANDARD LVCMOS33 } [get_ports { rst }]\n\
               set_property -dict { PACKAGE_PIN C4  IOSTANDARD LVCMOS33 } [get_ports { serin }]\n\
               set_property -dict { PACKAGE_PIN D4  IOSTANDARD LVCMOS33 } [get_ports { serout }]\n";
    fs::write(format!("{dir}/fmax.xdc"), xdc).unwrap();
    println!("emitted fmax.xdc (clk@100MHz target)");
}
