//! A board top under iverilog with its combinational datapath slowed: the datapath's outputs arrive `settle_ns`
//! after its inputs change (a transport delay wrapped around the real module), on a 100 MHz clock, and the top's UART
//! stream is decoded record by record. A top that measures its settle reports ⌈settle / 10 ns⌉ clocks; a top that
//! streams a constant does not.

use std::path::Path;

/// One 12-byte record: the sync byte, the record's tag (a config word or a vector index), the result and the cycles.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub tag: u8,
    pub result: i64,
    pub cycles: u16,
}

/// The header of `module <name>(…);` in `text`: the port declarations between the parentheses.
fn ports(text: &str, name: &str) -> String {
    let start = text.find(&format!("module {name}(")).unwrap_or_else(|| panic!("no `module {name}(`"));
    let open = start + format!("module {name}(").len();
    let close = text[open..].find(");").unwrap_or_else(|| panic!("`module {name}` never closes its ports")) + open;
    text[open..close].to_string()
}

/// Each port's direction, width text and name, from an ANSI port list.
fn port_list(header: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut dir = String::new();
    let mut width = String::new();
    for part in header.split(',') {
        let p = part.trim();
        let mut words: Vec<&str> = p.split_whitespace().collect();
        if words.first().is_some_and(|w| *w == "input" || *w == "output") {
            dir = words.remove(0).to_string();
            width.clear();
            if words.first().is_some_and(|w| *w == "wire" || *w == "reg") {
                words.remove(0);
            }
            if words.first().is_some_and(|w| *w == "signed") {
                words.remove(0);
            }
            if words.first().is_some_and(|w| w.starts_with('[')) {
                width = words.remove(0).to_string();
            }
        }
        let name = words.last().unwrap_or_else(|| panic!("a port with no name: {p:?}")).to_string();
        out.push((dir.clone(), width.clone(), name));
    }
    out
}

/// `datapath` renamed `<name>_settled`, and a module named `<name>` with its ports whose outputs follow the settled
/// module's by `settle_ns`.
pub fn slowed(datapath: &str, name: &str, settle_ns: u32) -> (String, String) {
    assert_eq!(datapath.matches(&format!("module {name}(")).count(), 1, "one `{name}` module");
    let settled = datapath.replace(&format!("module {name}("), &format!("module {name}_settled("));
    let ports = port_list(&ports(datapath, name));
    let decls: Vec<String> = ports
        .iter()
        .map(|(dir, width, n)| if dir == "output" { format!("output reg {width} {n}") } else { format!("input {width} {n}") })
        .collect();
    let mut body = String::new();
    for (dir, width, n) in &ports {
        if dir == "output" {
            body.push_str(&format!("  wire {width} {n}_settled;\n  initial {n} = 0;\n  always @({n}_settled) {n} <= #{settle_ns} {n}_settled;\n"));
        }
    }
    let conns: Vec<String> = ports
        .iter()
        .map(|(dir, _, n)| if dir == "output" { format!(".{n}({n}_settled)") } else { format!(".{n}({n})") })
        .collect();
    let wrapper = format!(
        "module {name}({});\n{body}  {name}_settled u({});\nendmodule\n",
        decls.join(", "),
        conns.join(", ")
    );
    (settled, wrapper)
}

/// Simulate `top` (the module `top_name`, instantiated with `params`) around `datapath_name` slowed by `settle_ns`, and
/// decode `records` records of its UART stream at 4 clocks a bit.
pub fn stream(
    dir: &Path,
    top_text: &str,
    top_name: &str,
    params: &str,
    datapath_text: &str,
    datapath_name: &str,
    settle_ns: u32,
    records: usize,
) -> Result<Vec<Record>, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (settled, wrapper) = slowed(datapath_text, datapath_name, settle_ns);
    let nbytes = records * 12;
    let tb = format!(
        "module tb;\n\
         \x20 reg clk = 1'b0; always #5 clk = ~clk;\n\
         \x20 reg rst = 1'b1;\n\
         \x20 wire tx; wire [15:0] led;\n\
         \x20 {top_name} #({params}) dut(.clk(clk), .rst(rst), .uart_tx(tx), .led(led));\n\
         \x20 integer i; integer n = 0; reg [7:0] b;\n\
         \x20 initial begin #100 rst = 1'b0; end\n\
         \x20 initial begin #20000000 $display(\"TIMEOUT\"); $finish; end\n\
         \x20 always begin\n\
         \x20\x20 @(negedge tx);\n\
         \x20\x20 repeat (2) @(posedge clk);\n\
         \x20\x20 for (i = 0; i < 8; i = i + 1) begin repeat (4) @(posedge clk); b[i] = tx; end\n\
         \x20\x20 repeat (4) @(posedge clk);\n\
         \x20\x20 $display(\"B %0d\", b);\n\
         \x20\x20 n = n + 1;\n\
         \x20\x20 if (n == {nbytes}) $finish;\n\
         \x20 end\n\
         endmodule\n"
    );
    let files = [("datapath_settled.v", settled), ("datapath.v", wrapper), ("top.v", top_text.to_string()), ("tb.v", tb)];
    let mut paths = Vec::new();
    for (name, text) in &files {
        let p = dir.join(name);
        std::fs::write(&p, format!("`timescale 1ns/1ps\n{text}")).map_err(|e| e.to_string())?;
        paths.push(p);
    }
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog").arg("-g2012").arg("-o").arg(&sim).args(&paths).output().map_err(|e| e.to_string())?;
    if !build.status.success() {
        return Err(format!("iverilog: {}", String::from_utf8_lossy(&build.stderr)));
    }
    let run = std::process::Command::new("vvp").arg(&sim).output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&run.stdout);
    if stdout.contains("TIMEOUT") {
        return Err(format!("no full lap of {records} records within the time limit"));
    }
    let bytes: Vec<u8> = stdout.lines().filter_map(|l| l.strip_prefix("B ")).map(|b| b.trim().parse::<u8>().unwrap()).collect();
    if bytes.len() != nbytes {
        return Err(format!("{} bytes, not {nbytes}", bytes.len()));
    }
    let _ = std::fs::remove_dir_all(dir);
    bytes
        .chunks(12)
        .map(|r| {
            if r[0] != 0xA5 {
                return Err(format!("a record without its sync byte: {r:?}"));
            }
            Ok(Record { tag: r[1], result: i64::from_be_bytes(r[2..10].try_into().unwrap()), cycles: u16::from_be_bytes([r[10], r[11]]) })
        })
        .collect()
}
