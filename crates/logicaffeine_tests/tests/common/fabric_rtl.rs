//! Running a fabric artifact under iverilog: write the bundle, compile, simulate, and read back what the testbench
//! printed — every `OUT <tensor> <index> <value>` line by tensor, the cycle count, and whether the design finished — or
//! hold it to the evaluator and the twin outright, or drive it step after step through its host port, as a host drives a
//! board, and read back each step's words.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, input_hex_files, VerilogArtifact};
use logicaffeine_compile::spatial::target::SpatialTarget;

/// What one simulation printed.
pub struct Run {
    /// Each tensor's values the testbench printed, in index order, by tensor id.
    pub out: BTreeMap<usize, Vec<i64>>,
    pub cycles: u64,
    pub done: bool,
    pub log: String,
}

/// Compile and simulate `art` with `inputs` in `dir`.
pub fn simulate(dir: &Path, p: &SpatialProgram, art: &VerilogArtifact, inputs: &Inputs) -> Run {
    std::fs::create_dir_all(dir).unwrap();
    let mut sources: Vec<(String, String)> = vec![
        ("spatial_top.v".into(), art.rtl.clone()),
        ("tb.v".into(), art.testbench.clone()),
    ];
    sources.extend(art.hex.iter().cloned());
    sources.extend(input_hex_files(p, inputs));
    let mut paths = Vec::new();
    for (name, body) in &sources {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        if name.ends_with(".v") {
            paths.push(path);
        }
    }
    let bin = dir.join("spatial_top.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&bin)
        .args(&paths)
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp")
        .arg(&bin)
        .current_dir(dir)
        .output()
        .expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    let mut out: BTreeMap<usize, Vec<(usize, i64)>> = BTreeMap::new();
    let (mut cycles, mut done) = (0, false);
    for line in log.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.first().copied() {
            Some("OUT") => {
                let v = match t[3].strip_prefix("0x") {
                    Some(h) => i64::from_str_radix(h, 16).map(|u| u as i64).expect("OUT hex"),
                    None => t[3].parse().expect("OUT value"),
                };
                out.entry(t[1].parse().expect("tensor id"))
                    .or_default()
                    .push((t[2].parse().expect("index"), v));
            }
            Some("CYCLES") => cycles = t[1].parse().expect("cycles"),
            Some("PASS:") => done = true,
            _ => {}
        }
    }
    let out = out
        .into_iter()
        .map(|(t, mut v)| {
            v.sort_by_key(|(i, _)| *i);
            (t, v.into_iter().map(|(_, x)| x).collect())
        })
        .collect();
    Run { out, cycles, done, log }
}

/// `art`, `p`'s fabric RTL on `target`, under iverilog from `inputs`: it finishes, every output and state tensor is the
/// evaluator's, and it takes exactly the twin's cycles.
pub fn runs_as_the_evaluator(label: &str, p: &SpatialProgram, target: &SpatialTarget, art: &VerilogArtifact, inputs: &Inputs) {
    let want = eval_choreography(p, inputs).unwrap_or_else(|e| panic!("{label}: the evaluator: {e:?}"));
    let dir = std::env::temp_dir().join(format!("fabric_runs_{}_{}", label.replace([' ', '/', '×'], "_"), std::process::id()));
    let run = simulate(&dir, p, art, inputs);
    assert!(run.done, "{label}: the design finishes\n{}", run.log);
    for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
        let got = run.out.get(&t.id.idx()).unwrap_or_else(|| panic!("{label}: the testbench prints {}", t.name));
        assert_eq!(got, w, "{label}: {} in RTL is the evaluator's", t.name);
    }
    assert_eq!(run.cycles, fabric_cycle_model(p, target), "{label}: the RTL's cycles are the twin's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One step a host drives through the port: the words it writes before the step (tensor, word, value), then — once the
/// step is done — the words it reads back (tensor, word).
pub struct HostStep {
    pub writes: Vec<(usize, usize, i64)>,
    pub reads: Vec<(usize, usize)>,
}

/// What a host-driven run returned: each step's words read, in order, and its cycle count.
pub struct HostRun {
    pub reads: Vec<Vec<i64>>,
    pub cycles: Vec<u64>,
    pub done: Vec<bool>,
    pub log: String,
}

/// Simulate `art` from the memory images `inputs` loads, then `steps` one after another through the host port, as a
/// host drives a board: each step's writes, a reset, a start, the run to done (bounded by `bound` cycles), then its
/// reads, each sampled `lat` cycles after its address.
pub fn simulate_host_steps(
    dir: &Path,
    p: &SpatialProgram,
    art: &VerilogArtifact,
    inputs: &Inputs,
    steps: &[HostStep],
    lat: u64,
    bound: u64,
) -> HostRun {
    std::fs::create_dir_all(dir).unwrap();
    let mut tb = String::from("`timescale 1ns/1ps\nmodule tb;\n");
    tb.push_str("  reg clk = 0, rst = 1, start = 0; wire done; wire [31:0] cycles;\n");
    tb.push_str("  reg host_we = 0, host_re = 0; reg [15:0] host_tensor = 0; reg [31:0] host_addr = 0; reg [63:0] host_wdata = 0;\n");
    tb.push_str("  wire [63:0] host_rdata;\n");
    tb.push_str("  spatial_top dut(.clk(clk), .rst(rst), .start(start), .done(done), .cycles(cycles), .host_we(host_we), .host_re(host_re),\n");
    tb.push_str("    .host_site(16'd0), .host_tensor(host_tensor), .host_addr(host_addr), .host_wdata(host_wdata), .host_rdata(host_rdata));\n");
    tb.push_str("  always #5 clk = ~clk;\n  integer guard;\n");
    tb.push_str("  task write_word(input [15:0] t, input [31:0] a, input [63:0] d);\n");
    tb.push_str("    begin @(negedge clk); host_we = 1; host_tensor = t; host_addr = a; host_wdata = d; @(negedge clk); host_we = 0; end\n  endtask\n");
    tb.push_str(&format!(
        "  task read_word(input [15:0] t, input [31:0] a);\n    begin @(negedge clk); host_re = 1; host_tensor = t; host_addr = a; repeat ({lat}) @(negedge clk); #1 $display(\"RD %0d\", $signed(host_rdata)); host_re = 0; end\n  endtask\n"
    ));
    tb.push_str(&format!(
        "  task step;\n    begin @(negedge clk); rst = 1; @(negedge clk); rst = 0; start = 1; @(negedge clk); start = 0; guard = 0;\n      while (!done && guard < {bound}) begin @(negedge clk); guard = guard + 1; end\n      $display(\"STEP %0d %0d\", done, cycles);\n    end\n  endtask\n"
    ));
    tb.push_str("  initial begin\n");
    for s in steps {
        for (t, a, v) in &s.writes {
            tb.push_str(&format!("    write_word({t}, {a}, 64'h{:016x});\n", *v as u64));
        }
        tb.push_str("    step;\n");
        for (t, a) in &s.reads {
            tb.push_str(&format!("    read_word({t}, {a});\n"));
        }
    }
    tb.push_str("    $display(\"END\");\n    $finish;\n  end\nendmodule\n");
    let mut sources: Vec<(String, String)> = vec![("spatial_top.v".into(), art.rtl.clone()), ("tb.v".into(), tb)];
    sources.extend(art.hex.iter().cloned());
    sources.extend(input_hex_files(p, inputs));
    let mut paths = Vec::new();
    for (name, body) in &sources {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        if name.ends_with(".v") {
            paths.push(path);
        }
    }
    let bin = dir.join("host.vvp");
    let build = std::process::Command::new("iverilog").arg("-g2012").arg("-o").arg(&bin).args(&paths).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&bin).current_dir(dir).output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(log.lines().any(|l| l.trim() == "END"), "the host run reaches its end\n{log}");
    let (mut reads, mut cycles, mut done) = (Vec::new(), Vec::new(), Vec::new());
    for line in log.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.first().copied() {
            Some("STEP") => {
                done.push(t[1] == "1");
                cycles.push(t[2].parse().expect("cycles"));
                reads.push(Vec::new());
            }
            Some("RD") => reads.last_mut().expect("a read after a step").push(t[1].parse().expect("RD value")),
            _ => {}
        }
    }
    HostRun { reads, cycles, done, log }
}

/// Simulate a board image — `board` (the serial pair and `decode_board`) over `art`'s step — speaking its protocol on
/// the serial pins at `cpb` clocks per bit: each command of `script` is sent byte by byte, then its reply of the given
/// length is awaited before the next. Returns each command's reply bytes; the whole run is bounded by `bound` cycles.
pub fn simulate_board(
    dir: &Path,
    p: &SpatialProgram,
    art: &VerilogArtifact,
    inputs: &Inputs,
    board: &str,
    cpb: u32,
    script: &[(Vec<u8>, usize)],
    bound: u64,
) -> Vec<Vec<u8>> {
    simulate_board_module(dir, p, art, inputs, "decode_board", board, cpb, script, bound)
}

/// [`simulate_board`] of the board module named `module` in `board`.
#[allow(clippy::too_many_arguments)]
pub fn simulate_board_module(
    dir: &Path,
    p: &SpatialProgram,
    art: &VerilogArtifact,
    inputs: &Inputs,
    module: &str,
    board: &str,
    cpb: u32,
    script: &[(Vec<u8>, usize)],
    bound: u64,
) -> Vec<Vec<u8>> {
    let mut sources: Vec<(String, String)> = vec![
        ("spatial_top.v".into(), art.rtl.clone()),
        ("board.v".into(), board.to_string()),
        ("tb.v".into(), board_testbench_of(module, Some(cpb), cpb, script, bound)),
    ];
    sources.extend(art.hex.iter().cloned());
    sources.extend(input_hex_files(p, inputs));
    run_board(dir, &sources, script, bound)
}

/// The testbench that speaks a board's protocol on its serial pins at `cpb` clocks per bit, `script` as
/// [`simulate_board`] takes it. `decode_board` is set to `parameter` clocks per bit where it still has the parameter
/// (`Some`), or taken as built — a netlist whose divider synthesis fixed (`None`).
pub fn board_testbench(parameter: Option<u32>, cpb: u32, script: &[(Vec<u8>, usize)], bound: u64) -> String {
    board_testbench_of("decode_board", parameter, cpb, script, bound)
}

/// [`board_testbench`] of the board module named `module`.
pub fn board_testbench_of(module: &str, parameter: Option<u32>, cpb: u32, script: &[(Vec<u8>, usize)], bound: u64) -> String {
    let mut tb = String::from("`timescale 1ns/1ps\nmodule tb;\n");
    tb.push_str("  reg clk = 0, rst = 1, rx = 1; wire tx; wire [7:0] led; integer got = 0, i; reg [7:0] b;\n");
    let params = parameter.map_or(String::new(), |c| format!(" #(.CLKS_PER_BIT({c}))"));
    tb.push_str(&format!("  {module}{params} dut(.clk(clk), .rst(rst), .uart_rx(rx), .uart_tx(tx), .led(led));\n"));
    tb.push_str("  always #5 clk = ~clk;\n");
    tb.push_str(&format!("  initial begin #({}); $display(\"TIMEOUT\"); $finish; end\n", bound * 10));
    let bit = 10 * cpb as u64;
    tb.push_str(&format!(
        "  initial forever begin @(negedge tx); #({}); for (i = 0; i < 8; i = i + 1) begin #({bit}); b[i] = tx; end #({bit}); $display(\"TX %0d\", b); got = got + 1; end\n",
        bit / 2
    ));
    tb.push_str(&format!(
        "  task send(input [7:0] v); integer k; begin rx = 0; #({bit}); for (k = 0; k < 8; k = k + 1) begin rx = v[k]; #({bit}); end rx = 1; #({bit}); end endtask\n"
    ));
    tb.push_str("  initial begin\n    #100 rst = 0; #100;\n");
    let mut want = 0usize;
    for (n, (bytes, reply)) in script.iter().enumerate() {
        for v in bytes {
            tb.push_str(&format!("    send(8'd{v});\n"));
        }
        want += reply;
        tb.push_str(&format!("    wait (got == {want}); $display(\"CMD {n}\");\n"));
    }
    tb.push_str("    #1000 $display(\"END\");\n    $finish;\n  end\nendmodule\n");
    tb
}

/// Behavioural models of the two clocking primitives a board top instantiates, for simulation: a PLLE2_BASE whose
/// CLKOUT0 runs at `CLKIN1_PERIOD · DIVCLK_DIVIDE · CLKOUT0_DIVIDE / CLKFBOUT_MULT` and which locks after 64 output
/// edges, and a BUFG that passes its input.
pub fn clocking_models() -> &'static str {
    "module PLLE2_BASE #(parameter real CLKIN1_PERIOD = 10.0, parameter integer CLKFBOUT_MULT = 5, parameter integer DIVCLK_DIVIDE = 1,\n\
     \x20 parameter integer CLKOUT0_DIVIDE = 1) (input CLKIN1, input CLKFBIN, output reg CLKFBOUT = 0, output reg CLKOUT0 = 0,\n\
     \x20 output reg LOCKED = 0, input RST, input PWRDWN);\n\
     \x20 real half; integer edges;\n\
     \x20 initial begin\n\
     \x20   half = CLKIN1_PERIOD * DIVCLK_DIVIDE * CLKOUT0_DIVIDE / CLKFBOUT_MULT / 2.0; edges = 0;\n\
     \x20   #(5 * CLKIN1_PERIOD);\n\
     \x20   forever begin #(half) CLKOUT0 = ~CLKOUT0; edges = edges + 1; if (edges == 64) LOCKED = 1; end\n\
     \x20 end\n\
     endmodule\n\
     module BUFG(input I, output O); assign O = I; endmodule\n"
}

/// The testbench that speaks a board top's protocol on its serial pins: the top named `top` driven by the 100 MHz
/// oscillator, each bit `bit_ns` long, `script` as [`simulate_board`] takes it, the run bounded by `bound_ns`.
pub fn top_testbench(top: &str, bit_ns: f64, script: &[(Vec<u8>, usize)], bound_ns: u64) -> String {
    let mut tb = String::from("`timescale 1ns/1ps\nmodule tb;\n");
    tb.push_str("  reg clk = 0, rx = 1; wire tx; wire [7:0] led; integer got = 0, i; reg [7:0] b;\n");
    tb.push_str(&format!("  {top} dut(.clk(clk), .uart_rx(rx), .uart_tx(tx), .led(led));\n"));
    tb.push_str("  always #5 clk = ~clk;\n");
    tb.push_str(&format!("  initial begin #({bound_ns}); $display(\"TIMEOUT\"); $finish; end\n"));
    tb.push_str(&format!(
        "  initial forever begin @(negedge tx); #({:.3}); for (i = 0; i < 8; i = i + 1) begin #({bit_ns:.3}); b[i] = tx; end #({bit_ns:.3}); $display(\"TX %0d\", b); got = got + 1; end\n",
        bit_ns / 2.0
    ));
    tb.push_str(&format!(
        "  task send(input [7:0] v); integer k; begin rx = 0; #({bit_ns:.3}); for (k = 0; k < 8; k = k + 1) begin rx = v[k]; #({bit_ns:.3}); end rx = 1; #({bit_ns:.3}); end endtask\n"
    ));
    tb.push_str("  initial begin\n    wait (dut.locked); #2000;\n");
    let mut want = 0usize;
    for (n, (bytes, reply)) in script.iter().enumerate() {
        for v in bytes {
            tb.push_str(&format!("    send(8'd{v});\n"));
        }
        want += reply;
        tb.push_str(&format!("    wait (got == {want}); $display(\"CMD {n}\");\n"));
    }
    tb.push_str("    #1000 $display(\"END\");\n    $finish;\n  end\nendmodule\n");
    tb
}

/// Compile `sources` (a board, its testbench and its images) in `dir`, run it, and split what the line carried into each
/// command's reply.
pub fn run_board(dir: &Path, sources: &[(String, String)], script: &[(Vec<u8>, usize)], bound: u64) -> Vec<Vec<u8>> {
    std::fs::create_dir_all(dir).unwrap();
    let mut paths = Vec::new();
    for (name, body) in sources {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        if name.ends_with(".v") {
            paths.push(path);
        }
    }
    let bin = dir.join("board.vvp");
    let build = std::process::Command::new("iverilog").arg("-g2012").arg("-o").arg(&bin).args(&paths).output().expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&bin).current_dir(dir).output().expect("vvp");
    assert!(run.status.success(), "vvp: {}", String::from_utf8_lossy(&run.stderr));
    let log = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(log.lines().any(|l| l.trim() == "END"), "the board answers every command within {bound} cycles\n{log}");
    let mut replies: Vec<Vec<u8>> = vec![Vec::new(); script.len()];
    let mut at = 0usize;
    for line in log.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.first().copied() {
            Some("TX") => replies[at].push(t[1].parse().expect("TX byte")),
            Some("CMD") => at += 1,
            _ => {}
        }
    }
    replies
}
