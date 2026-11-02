//! measured_fields_lock — a field a board streams as a measurement is measured on the board.
//!
//! A board top that reports a number over its UART (a cycle count, a latency) must compute it in hardware: from
//! a counter, a capture, a register the design writes. A named constant streamed in that position is a claim
//! the board never checked, and every host that compares the field against the same constant passes forever.
//! This census reads every board top in `benchmarks/silicon/` and refuses any streamed byte that is a constant:
//! an expression of localparams only, or a literal other than the frame's sync byte (0xA5) and the reset value.
//! Today's offenders sit on a shrink-only allowlist, each with the ticket that replaces its constant by a
//! measurement; an entry that no longer streams a constant must leave the list.

#[path = "common/board_census.rs"]
mod board_census;

use board_census::{board_tops, constant_streams};
use std::collections::BTreeSet;

/// Board tops that still stream a constant, with the ticket that gives each a measurement. Only shrinks.
const STILL_CONSTANT: &[(&str, &str)] = &[
    ("nexys_conv_soc.v", "FABRIC-SETTLE-PROBE"),
    ("nexys_sobel_soc.v", "FABRIC-SETTLE-PROBE"),
    ("nexys_stream_soc.v", "FABRIC-SETTLE-PROBE"),
    ("ours_adder.v", "FABRIC-SETTLE-PROBE"),
    ("ours_affine.v", "FABRIC-SETTLE-PROBE"),
    ("ours_ratio.v", "FABRIC-SETTLE-PROBE"),
];

#[test]
fn no_board_streams_a_constant_as_a_measurement() {
    let tops = board_tops();
    assert!(tops.len() >= 10, "the census found only {} board tops", tops.len());
    let allowed: BTreeSet<&str> = STILL_CONSTANT.iter().map(|(f, _)| *f).collect();
    let mut offenders = Vec::new();
    for (file, text) in &tops {
        for (expr, why) in constant_streams(text) {
            if !allowed.contains(file.as_str()) {
                offenders.push(format!("{file}: `tx_byte <= {expr}` ({why})"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a board streams a constant where a measurement belongs — compute it in hardware (a counter, a capture):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_allowlist_only_names_boards_that_still_need_it() {
    let tops = board_tops();
    for (file, ticket) in STILL_CONSTANT {
        let text = tops.get(*file).unwrap_or_else(|| panic!("{file} ({ticket}) is not a board top any more: remove it from STILL_CONSTANT"));
        assert!(
            !constant_streams(text).is_empty(),
            "{file} no longer streams a constant ({ticket} landed): remove it from STILL_CONSTANT"
        );
    }
}

#[test]
fn a_board_top_is_a_design_that_streams_out_of_its_uart_pin() {
    assert!(board_census::is_board_top("module top(output reg uart_tx);
reg [7:0] tx_byte;
endmodule"));
    assert!(board_census::is_board_top("module top(
  output wire uart_tx,
  input clk);
reg [7:0] tx_byte;
endmodule"));
    assert!(
        !board_census::is_board_top("module slip(output reg [7:0] tx_byte);
always @(posedge clk) tx_byte <= 8'hC0;
endmodule"),
        "a framer that drives no UART pin is not a board"
    );
    assert!(!board_census::is_board_top("module top(output reg uart_tx); // tx_byte
endmodule"), "a top that streams no byte");
    assert!(!board_census::is_board_top("module m(input uart_tx_ready); reg [7:0] tx_byte; endmodule"));
    let tops = board_tops();
    for part in ["term_slip.v", "term_link.v", "term_ps2.v"] {
        assert!(!tops.contains_key(part), "{part} is a part of the Fabric Terminal, not a board top");
    }
    for top in ["nexys_fabric_terminal.v", "nexys_terminal_shell.v", "nexys_echo_soc.v", "ours_adder.v"] {
        assert!(tops.contains_key(top), "{top} is a board top");
    }
}

#[test]
fn the_census_refuses_every_form_of_a_streamed_constant() {
    let refused = [
        "localparam [15:0] CYCLES = 16'd1;\nalways @(posedge clk) case (pos) 4'd10: tx_byte<=CYCLES[15:8]; endcase",
        "localparam A = 1, B = 2;\nalways @(posedge clk) tx_byte <= {A[3:0], B[3:0]};",
        "always @(posedge clk) tx_byte <= 8'd1;",
        "always @(posedge clk) tx_byte <= 8'h01; // a cycle count",
    ];
    for v in refused {
        assert!(!constant_streams(v).is_empty(), "the census missed a streamed constant in:\n{v}");
    }
    let accepted = [
        "localparam [15:0] CYCLES = 16'd1;\nreg [15:0] cyc;\nalways @(posedge clk) tx_byte <= cyc[15:8];",
        "always @(posedge clk) tx_byte <= 8'hA5;",
        "always @(posedge clk) if (rst) tx_byte <= 8'd0;",
        "localparam W = 3;\nreg [W-1:0] cfg;\nalways @(posedge clk) tx_byte <= {5'd0, cfg};",
        "/* tx_byte <= CYCLES[7:0]; */ localparam [15:0] CYCLES = 16'd1;\nalways @(posedge clk) tx_byte <= res_byte;",
    ];
    for v in accepted {
        assert!(constant_streams(v).is_empty(), "the census refused a measured stream in:\n{v}\n{:?}", constant_streams(v));
    }
}
