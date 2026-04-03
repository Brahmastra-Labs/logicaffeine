//! Sprint 2A: Verilog Declaration Parser
//!
//! Tests for extracting module structure from Verilog declarations.
//! The parser handles ANSI-style module headers, port declarations,
//! signal types, parameters, and clock detection.

use logicaffeine_compile::codegen_sva::rtl_extract::{
    parse_verilog_module, RtlModule, RtlPort, RtlSignal, RtlParam,
    PortDirection, SignalType,
};

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: EMPTY MODULE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_empty_module() {
    let src = "module empty_mod;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.name, "empty_mod");
    assert!(m.ports.is_empty());
    assert!(m.signals.is_empty());
}

#[test]
fn parse_module_name_with_underscores() {
    let src = "module axi_lite_slave;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.name, "axi_lite_slave");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: ANSI-STYLE PORTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_input_port() {
    let src = "module m (\n  input clk\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports.len(), 1);
    assert_eq!(m.ports[0].name, "clk");
    assert_eq!(m.ports[0].direction, PortDirection::Input);
    assert_eq!(m.ports[0].width, 1);
}

#[test]
fn parse_output_port() {
    let src = "module m (\n  output valid\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports[0].direction, PortDirection::Output);
}

#[test]
fn parse_inout_port() {
    let src = "module m (\n  inout data\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports[0].direction, PortDirection::Inout);
}

#[test]
fn parse_port_with_width() {
    let src = "module m (\n  input [7:0] data\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports[0].name, "data");
    assert_eq!(m.ports[0].width, 8);
}

#[test]
fn parse_port_with_large_width() {
    let src = "module m (\n  input [31:0] addr\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports[0].width, 32);
}

#[test]
fn parse_multiple_ports() {
    let src = "module m (\n  input clk,\n  input rst_n,\n  output [7:0] data,\n  output valid\n);\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.ports.len(), 4);
    assert_eq!(m.ports[0].name, "clk");
    assert_eq!(m.ports[2].name, "data");
    assert_eq!(m.ports[2].width, 8);
    assert_eq!(m.ports[3].name, "valid");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: SIGNAL TYPES (wire/reg/logic)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_wire_signal() {
    let src = "module m;\n  wire enable;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.signals.iter().any(|s| s.name == "enable" && s.signal_type == SignalType::Wire));
}

#[test]
fn parse_reg_signal() {
    let src = "module m;\n  reg [7:0] counter;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    let sig = m.signals.iter().find(|s| s.name == "counter").unwrap();
    assert_eq!(sig.signal_type, SignalType::Reg);
    assert_eq!(sig.width, 8);
}

#[test]
fn parse_logic_signal() {
    let src = "module m;\n  logic [15:0] data_reg;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    let sig = m.signals.iter().find(|s| s.name == "data_reg").unwrap();
    assert_eq!(sig.signal_type, SignalType::Logic);
    assert_eq!(sig.width, 16);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: PARAMETERS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_parameter() {
    let src = "module m;\n  parameter DATA_WIDTH = 32;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    let param = m.params.iter().find(|p| p.name == "DATA_WIDTH").unwrap();
    assert_eq!(param.value, "32");
}

#[test]
fn parse_localparam() {
    let src = "module m;\n  localparam DEPTH = 16;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.params.iter().any(|p| p.name == "DEPTH"));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: CLOCK DETECTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn detect_clock_from_always_posedge() {
    let src = "module m (\n  input clk\n);\n  always @(posedge clk) begin\n  end\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.clocks.contains(&"clk".to_string()),
        "Should detect clk from always @(posedge clk). Clocks: {:?}", m.clocks);
}

#[test]
fn detect_clock_from_always_negedge() {
    let src = "module m (\n  input sclk\n);\n  always @(negedge sclk) begin\n  end\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.clocks.contains(&"sclk".to_string()));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6: REAL-WORLD MODULE HEADERS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn parse_apb_slave_header() {
    let src = r#"module apb_slave (
    input        pclk,
    input        presetn,
    input        psel,
    input        penable,
    input        pwrite,
    input  [31:0] paddr,
    input  [31:0] pwdata,
    output [31:0] prdata,
    output        pready,
    output        pslverr
);
endmodule"#;
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.name, "apb_slave");
    assert!(m.ports.len() >= 9, "APB slave should have at least 9 ports, got {}", m.ports.len());
    assert!(m.ports.iter().any(|p| p.name == "pclk" && p.direction == PortDirection::Input));
    assert!(m.ports.iter().any(|p| p.name == "prdata" && p.width == 32));
}

#[test]
fn parse_uart_tx_header() {
    let src = r#"module uart_tx (
    input        clk,
    input        rst_n,
    input  [7:0] tx_data,
    input        tx_start,
    output       tx_busy,
    output       tx_out
);
endmodule"#;
    let m = parse_verilog_module(src).unwrap();
    assert_eq!(m.name, "uart_tx");
    assert!(m.ports.iter().any(|p| p.name == "tx_data" && p.width == 8));
    assert!(m.ports.iter().any(|p| p.name == "tx_busy" && p.direction == PortDirection::Output));
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7: ERROR HANDLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn error_on_missing_module_keyword() {
    let result = parse_verilog_module("not_a_module foo;\nendmodule");
    assert!(result.is_err(), "Should error without 'module' keyword");
}

#[test]
fn error_on_missing_endmodule() {
    let result = parse_verilog_module("module m;\n  wire x;");
    assert!(result.is_err(), "Should error without 'endmodule'");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 8: COMMENT/STRING SKIPPING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn skip_single_line_comments() {
    let src = "module m;\n  // This is a comment\n  wire enable;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.signals.iter().any(|s| s.name == "enable"));
}

#[test]
fn skip_multi_line_comments() {
    let src = "module m;\n  /* multi\n  line\n  comment */\n  wire enable;\nendmodule";
    let m = parse_verilog_module(src).unwrap();
    assert!(m.signals.iter().any(|s| s.name == "enable"));
}
