//! fabric_stream_lock — the parameterized LINE-BUFFER STREAMING Sobel SoC, locked ≡ VM under iverilog.
//!
//! `benchmarks/silicon/nexys_stream_soc.v` receives an H×W tile over UART, raster-streams it one
//! pixel/clock through a depth-(2W+3) shift register (two line buffers + the current-row tap), runs
//! `fabric_sobel3` on every interior 3×3 window, and captures the (H-2)×(W-2) edge map into `obuf`.
//! This test bit-bangs arbitrary tiles into the SoC's UART RX under iverilog — at several image sizes
//! (square and non-square) — and asserts the captured edge map equals the VM's Sobel on the same
//! windows, the exact behavior verified on the real Artix-7 (scripts/stream-{host,battery}.py). It
//! pins the streaming architecture (RX byte-recovery, generalized line-buffer window extraction,
//! stencil, capture) so it cannot regress at any resolution.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::compile::vm_outcome;
use std::path::PathBuf;

const DECL: &str = "## To sobel3 (p1: Int, p2: Int, p3: Int, p4: Int, p5: Int, p6: Int, p7: Int, p8: Int, p9: Int) -> Int:\n\
    \x20   Let gx be (0 - p1) + p3 - 2 * p4 + 2 * p6 - p7 + p9.\n\
    \x20   Let gy be (0 - p1) - 2 * p2 - p3 + p7 + 2 * p8 + p9.\n\
    \x20   Let mx be gx shifted right by 63.\n\
    \x20   Let ax be (gx xor mx) - mx.\n\
    \x20   Let my be gy shifted right by 63.\n\
    \x20   Let ay be (gy xor my) - my.\n\
    \x20   Return ax + ay.";

/// The VM's Sobel on one 3x3 window (row-major p1..p9) — the ground-truth oracle.
fn vm_sobel(px: &[i64; 9]) -> i64 {
    let args = px
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("{DECL}\n## Main\n    Show sobel3({args}).\n");
    let out = vm_outcome(&src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

/// The captured (H-2)x(W-2) edge map the SoC must produce: obuf[orr*(W-2)+occ] = sobel of the window
/// whose top-left is (orr,occ) — exactly how the sr taps wire into fabric_sobel3.
fn expected_obuf(img: &[i64], w: usize, h: usize) -> Vec<i64> {
    let mut out = Vec::with_capacity((h - 2) * (w - 2));
    for orr in 0..(h - 2) {
        for occ in 0..(w - 2) {
            let mut win = [0i64; 9];
            for i in 0..3 {
                for j in 0..3 {
                    win[i * 3 + j] = img[(orr + i) * w + (occ + j)];
                }
            }
            out.push(vm_sobel(&win));
        }
    }
    out
}

/// A deterministic set of H*W tiles of a given kind.
fn tile(w: usize, h: usize, kind: &str) -> Vec<i64> {
    let mut a = vec![0i64; w * h];
    let mut x: u64 = 0xC0FFEE;
    for r in 0..h {
        for c in 0..w {
            a[r * w + c] = match kind {
                "vertical" => {
                    if c < w / 2 {
                        0
                    } else {
                        255
                    }
                }
                "horizontal" => {
                    if r < h / 2 {
                        0
                    } else {
                        255
                    }
                }
                "diagonal" => {
                    if r + c < (w + h) / 2 {
                        0
                    } else {
                        255
                    }
                }
                "box" => {
                    if h / 4 <= r && r < 3 * h / 4 && w / 4 <= c && c < 3 * w / 4 {
                        255
                    } else {
                        0
                    }
                }
                _ => 0, // "random" filled below
            };
        }
    }
    if kind == "random" {
        for v in a.iter_mut() {
            x = x.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
            *v = ((x >> 16) & 0xff) as i64;
        }
    }
    a
}

fn silicon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("benchmarks/silicon")
}

/// Drive one H×W tile into the SoC's UART RX under iverilog, return the captured obuf[0..(H-2)*(W-2)].
fn run_soc(img: &[i64], w: usize, h: usize) -> Vec<i64> {
    let sil = silicon_dir();
    let soc =
        std::fs::read_to_string(sil.join("nexys_stream_soc.v")).expect("read nexys_stream_soc.v");
    let sobel = std::fs::read_to_string(sil.join("fabric_sobel3.v")).expect("read fabric_sobel3.v");
    let nout = (w - 2) * (h - 2);

    let mut sends = String::new();
    for &p in img.iter() {
        sends.push_str(&format!("    send_byte(8'd{});\n", p & 0xff));
    }
    let tb = format!(
        "module BUFG(input I, output O); assign O = I; endmodule\n\
         module tb;\n\
         \x20 localparam CPB = 8;\n\
         \x20 reg clk=0, rst=0, rx=1; wire tx; wire [15:0] led;\n\
         \x20 nexys_stream_soc #(.CLKS_PER_BIT(CPB), .IMG_W({w}), .IMG_H({h})) dut(.clk(clk), .rst(rst), .uart_rx(rx), .uart_tx(tx), .led(led));\n\
         \x20 always #1 clk = ~clk;\n\
         \x20 task send_byte; input [7:0] b; integer i; begin\n\
         \x20   rx <= 1'b0; repeat (CPB) @(posedge dut.coreclk);\n\
         \x20   for (i=0;i<8;i=i+1) begin rx <= b[i]; repeat (CPB) @(posedge dut.coreclk); end\n\
         \x20   rx <= 1'b1; repeat (CPB) @(posedge dut.coreclk); repeat (3) @(posedge dut.coreclk);\n\
         \x20 end endtask\n\
         \x20 integer i;\n\
         \x20 initial begin\n\
         \x20   wait (dut.rstb == 1'b0); repeat (4) @(posedge dut.coreclk);\n\
         {sends}\
         \x20   wait (dut.phase == 2'd2); @(posedge dut.coreclk);\n\
         \x20   for (i=0;i<{nout};i=i+1) $write(\"%0d \", dut.obuf[i]);\n\
         \x20   $write(\"\\n\"); $finish;\n\
         \x20 end\n\
         \x20 initial begin #200000000 $display(\"TIMEOUT\"); $finish; end\n\
         endmodule\n"
    );

    let dir = std::env::temp_dir().join(format!("logos_fabric_stream_{w}x{h}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("nexys_stream_soc.v"), &soc).expect("write soc");
    std::fs::write(dir.join("fabric_sobel3.v"), &sobel).expect("write sobel");
    std::fs::write(dir.join("tb.v"), &tb).expect("write tb");
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.join("nexys_stream_soc.v"))
        .arg(dir.join("fabric_sobel3.v"))
        .arg(dir.join("tb.v"))
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(
        !text.contains("TIMEOUT"),
        "streaming SoC sim timed out ({w}x{h})"
    );
    // obuf holds the packed word {v[10:8], 0, v[7:0]}; bit 8 is a reserved dummy (BRAM-parity
    // workaround) and MUST stay 0, else real data would land on the bit the open flow drops.
    let line = text.lines().next().unwrap_or("");
    line.split_whitespace()
        .filter_map(|t| t.parse::<i64>().ok())
        .map(|word| {
            assert_eq!(
                word & 0x100,
                0,
                "obuf reserved bit 8 must be 0 (BRAM-parity workaround), got {word:#x}"
            );
            ((word >> 9) & 0x7) << 8 | (word & 0xFF)
        })
        .collect()
}

/// Drive one tile in, capture the transmitted UART byte stream (sampling tx_byte at each
/// S_LOAD→S_START edge), and decode the 13-byte records {0xA5, idx[15:0] BE, val[63:0] BE, cyc[15:0]}
/// through the SYNCHRONOUS obuf_q read path — the exact bytes the host sees on silicon.
fn run_soc_emit(img: &[i64], w: usize, h: usize) -> Vec<i64> {
    let sil = silicon_dir();
    let soc = std::fs::read_to_string(sil.join("nexys_stream_soc.v")).expect("read soc");
    let sobel = std::fs::read_to_string(sil.join("fabric_sobel3.v")).expect("read sobel");
    let nout = (w - 2) * (h - 2);
    let want = 13 * nout * 2; // two full frames so a clean record-aligned frame is guaranteed

    let mut sends = String::new();
    for &p in img.iter() {
        sends.push_str(&format!("    send_byte(8'd{});\n", p & 0xff));
    }
    let tb = format!(
        "module BUFG(input I, output O); assign O = I; endmodule\n\
         module tb;\n\
         \x20 localparam CPB = 8;\n\
         \x20 reg clk=0, rst=0, rx=1; wire tx; wire [15:0] led;\n\
         \x20 nexys_stream_soc #(.CLKS_PER_BIT(CPB), .IMG_W({w}), .IMG_H({h})) dut(.clk(clk), .rst(rst), .uart_rx(rx), .uart_tx(tx), .led(led));\n\
         \x20 always #1 clk = ~clk;\n\
         \x20 task send_byte; input [7:0] b; integer i; begin\n\
         \x20   rx <= 1'b0; repeat (CPB) @(posedge dut.coreclk);\n\
         \x20   for (i=0;i<8;i=i+1) begin rx <= b[i]; repeat (CPB) @(posedge dut.coreclk); end\n\
         \x20   rx <= 1'b1; repeat (CPB) @(posedge dut.coreclk); repeat (3) @(posedge dut.coreclk);\n\
         \x20 end endtask\n\
         \x20 reg [2:0] prev; integer nb=0; reg [7:0] cap [0:{cap_max}];\n\
         \x20 always @(posedge dut.coreclk) begin\n\
         \x20   prev <= dut.state;\n\
         \x20   if (dut.phase==2'd2 && dut.state==3'd1 && prev==3'd0) begin cap[nb]=dut.tx_byte; nb=nb+1; end\n\
         \x20 end\n\
         \x20 integer i;\n\
         \x20 initial begin\n\
         \x20   wait (dut.rstb == 1'b0); repeat (4) @(posedge dut.coreclk);\n\
         {sends}\
         \x20   wait (nb >= {want}); \n\
         \x20   for (i=0;i<{want};i=i+1) $write(\"%0d \", cap[i]);\n\
         \x20   $write(\"\\n\"); $finish;\n\
         \x20 end\n\
         \x20 initial begin #400000000 $display(\"TIMEOUT\"); $finish; end\n\
         endmodule\n",
        cap_max = want + 16
    );

    let dir = std::env::temp_dir().join(format!("logos_fabric_emit_{w}x{h}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("nexys_stream_soc.v"), &soc).expect("write soc");
    std::fs::write(dir.join("fabric_sobel3.v"), &sobel).expect("write sobel");
    std::fs::write(dir.join("tb.v"), &tb).expect("write tb");
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.join("nexys_stream_soc.v"))
        .arg(dir.join("fabric_sobel3.v"))
        .arg(dir.join("tb.v"))
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(!text.contains("TIMEOUT"), "emit sim timed out ({w}x{h})");
    let bytes: Vec<u8> = text
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|t| t.parse::<u16>().ok())
        .map(|b| b as u8)
        .collect();
    // frame-lock on 0xA5 + sequential 2-byte index, then decode the 8-byte big-endian result.
    let rec = 13usize;
    let off = (0..=bytes.len().saturating_sub(rec * nout))
        .find(|&o| {
            (0..nout).all(|i| {
                bytes[o + i * rec] == 0xA5
                    && ((bytes[o + i * rec + 1] as usize) << 8 | bytes[o + i * rec + 2] as usize)
                        == i
            })
        })
        .expect("emit stream did not frame-lock");
    (0..nout)
        .map(|i| {
            let mut v = [0u8; 8];
            v.copy_from_slice(&bytes[off + i * rec + 3..off + i * rec + 11]);
            i64::from_be_bytes(v)
        })
        .collect()
}

fn check(w: usize, h: usize, kinds: &[&str]) {
    for kind in kinds {
        let img = tile(w, h, kind);
        let got = run_soc(&img, w, h);
        let exp = expected_obuf(&img, w, h);
        assert_eq!(
            got.len(),
            (w - 2) * (h - 2),
            "{w}x{h}/{kind}: wrong output count"
        );
        assert_eq!(
            got, exp,
            "streaming SoC obuf != VM Sobel on {w}x{h} {kind} tile"
        );
    }
}

#[test]
fn streaming_sobel_soc_matches_vm_8x8() {
    if !common::require_tool("iverilog") {
        return;
    }
    check(
        8,
        8,
        &["vertical", "horizontal", "diagonal", "box", "random"],
    );
}

#[test]
fn streaming_sobel_soc_matches_vm_16x16() {
    if !common::require_tool("iverilog") {
        return;
    }
    check(16, 16, &["vertical", "random"]);
}

#[test]
fn streaming_sobel_soc_matches_vm_nonsquare_16x12() {
    if !common::require_tool("iverilog") {
        return;
    }
    check(16, 12, &["diagonal", "random"]); // W=16, H=12 — proves M != N
}

#[test]
fn streaming_sobel_emit_records_match_vm() {
    // The whole chain: RX → line-buffer stream → SYNCHRONOUS obuf_q (block-RAM) read → UART record
    // bytes. Decodes the actual 13-byte records the host sees on silicon and checks == the VM.
    if !common::require_tool("iverilog") {
        return;
    }
    for kind in ["vertical", "random"] {
        let img = tile(8, 8, kind);
        let got = run_soc_emit(&img, 8, 8);
        let exp = expected_obuf(&img, 8, 8);
        assert_eq!(
            got, exp,
            "emitted UART records != VM Sobel on 8x8 {kind} tile"
        );
    }
}
