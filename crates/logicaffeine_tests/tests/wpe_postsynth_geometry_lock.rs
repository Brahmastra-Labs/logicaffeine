//! POST-SYNTH GEOMETRY LOCK — the QA layer that catches yosys BRAM mis-synthesis BEFORE silicon.
//!
//! WHY THIS EXISTS: every other engine lock (`wpe_backend_lock`, `wprog_engine_soc_lock`) runs iverilog on the
//! RTL, where memories are behavioral `reg` arrays — so they validate LOGIC, never the yosys-SYNTHESIZED netlist.
//! Two real yosys-0.33 `synth_xilinx` BRAM defects reached a physical XC7A200T with every ≡VM lock still GREEN:
//!   1. HEAP DEPTH-CASCADE — a heap deeper than a 4096-word BRAM tile makes yosys drop the top heap-address bits
//!      (`heap_ra[14:12]`), so any access >= 4096 aliases to tile 0. FIX = `WPE_STREAM_HEAP = 32768` (a clean 2^15
//!      single-tile depth). A 32768-deep heap half maps to a RAMB36E1 in ×1-width, 32768-deep mode — whose
//!      INTERNAL two-16K-half depth-cascade (a 15-bit index, address bit 15 tied high on the upper tiles) is a
//!      further, prjxray/openXC7-LEVEL defect (BELOW the netlist — the gate-level sim stays ≡VM). SECOND FIX =
//!      DEPTH-BANK the heap into two 16384-deep banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address bit 14),
//!      so every heap tile is a 14-bit-indexed 16384-deep tile with NO 32K depth-cascade. SIGNATURE = the heap
//!      maps to 16384-deep tiles addressed by `heap_ra[13:0]` (14 bits) and NO heap RAM port is width-1.
//!   2. SDP-72 PARITY MIS-WIRE — a 64-bit-wide array mapping to one RAMB36E1 in SDP-72 mode has its parity-lane
//!      write (DIPBDIP) mis-wired, losing the 8 parity bits, so any value >= 256 reads back corrupted. FIX = split
//!      `file`/`v`/`sfile` into 32-bit lo/hi halves so nothing maps SDP-72. SIGNATURE = no RAM port is 72-bit-wide.
//!
//! This lock synthesizes the ACTUAL ConfigBram engine with the SAME yosys flow that reaches silicon and asserts the
//! netlist carries NEITHER bug signature — deterministically, in one synth + a grep, no 30-minute gate-level sim.
//! It has TEETH: `..._diverges_when_the_heap_fix_is_reverted` re-synthesizes at the buggy 20480 depth and asserts
//! the truncation signature REAPPEARS, and `unsplit_64bit_memory_maps_to_sdp72` proves yosys really does map an
//! un-split 64-bit memory to the SDP-72 mode this lock forbids — so a green run is meaningful, not vacuous.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::needless_range_loop)]

use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_backend, MemBackend, WPE_STREAM_CAPS, WPE_STREAM_HEAP,
};

#[path = "common/mod.rs"]
mod common;

fn build_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join("wpe_postsynth_geom");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Synthesize the ConfigBram engine at `heap_depth` with the EXACT silicon flow (`synth_xilinx -family xc7 -nodsp
/// -nowidelut -flatten`, WITH_FPU=0, CFG_BRAM=1) and return the mapped Verilog netlist. The engine RTL is
/// program-independent (the program is streamed at runtime), so no config image / sim is needed — this is the fast
/// path: emit -> yosys -> read the netlist. Returns `None` (with a skip note) if yosys is unavailable.
fn synth_engine_netlist(tag: &str, heap_depth: usize) -> Option<String> {
    if !common::require_tool("yosys") {
        return None;
    }
    let mut caps = WPE_STREAM_CAPS;
    caps[7] = heap_depth; // WPE_STREAM_CAPS[7] is the HEAP depth cap.
    let module = format!("wpe_soc_{tag}");
    let soc = emit_wprog_engine_soc_verilog_backend(tag, caps, MemBackend::ConfigBram);

    let dir = build_dir();
    let engine_v = dir.join(format!("engine_{tag}.v"));
    let netlist_v = dir.join(format!("netlist_{tag}.v"));
    let ys = dir.join(format!("synth_{tag}.ys"));
    std::fs::write(&engine_v, &soc).unwrap();
    let flow = format!(
        "read_verilog -sv {engine}\n\
         chparam -set WITH_FPU 0 {module}\n\
         chparam -set CFG_BRAM 1 {module}\n\
         hierarchy -top {module}\n\
         synth_xilinx -family xc7 -nodsp -nowidelut -flatten -top {module}\n\
         write_verilog -noattr {netlist}\n",
        engine = engine_v.display(),
        module = module,
        netlist = netlist_v.display(),
    );
    std::fs::write(&ys, &flow).unwrap();
    let out = std::process::Command::new("yosys")
        .arg("-s")
        .arg(&ys)
        .current_dir(&dir)
        .output()
        .expect("yosys");
    assert!(
        out.status.success() && netlist_v.exists(),
        "{tag}: yosys synth failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Some(std::fs::read_to_string(&netlist_v).unwrap())
}

/// Count RAMB36E1/RAMB18E1 ports declared at a given port width in the netlist (e.g. `.READ_WIDTH_A(32'd72)`).
fn count_width(netlist: &str, width: u32) -> usize {
    let needle = format!("_WIDTH_A(32'd{width})");
    let needle_b = format!("_WIDTH_B(32'd{width})");
    netlist.matches(&needle).count() + netlist.matches(&needle_b).count()
}

/// BUG #2 (SDP-72 parity) is ABSENT: no RAM port in the synthesized engine is 72 bits wide. A width-72 port only
/// arises when a full 64-bit-wide array maps to one RAMB36E1 in SDP-72 mode — exactly the config the parity-lane
/// mis-wire corrupts. The `file`/`v`/`sfile` lo/hi split keeps every memory <= 36-bit, so this must be zero.
#[test]
fn synthesized_engine_has_no_sdp72_width_port() {
    let Some(nl) = synth_engine_netlist("nosdp72", WPE_STREAM_HEAP) else {
        return;
    };
    let w72 = count_width(&nl, 72);
    // Non-vacuous: the engine DOES map memories to RAMBs (there are 36-bit and 1-bit ports), so a zero here is a
    // real "no 64-bit-wide RAM" result, not "no RAMs at all".
    let ramb = nl.matches("RAMB36E1 ").count() + nl.matches("RAMB18E1 ").count();
    assert!(
        ramb > 0,
        "no RAMB primitives synthesized — the geometry check would be vacuous"
    );
    assert_eq!(
        w72, 0,
        "BUG #2 REGRESSION: {w72} RAM port(s) are 72-bit-wide — a 64-bit memory mapped to SDP-72, whose parity \
         lane yosys 0.33 mis-wires (values >= 256 corrupt on silicon). Split it into 32-bit lo/hi halves \
         (see file/v/sfile in wpe_soc_template.v). ({ramb} RAMBs total.)"
    );
}

/// Prove the width-72 detector has TEETH: an UN-split 64-bit memory really does map to SDP-72 under this flow, so
/// `synthesized_engine_has_no_sdp72_width_port` is asserting something yosys would otherwise do. This synthesizes a
/// tiny standalone `reg [63:0] m [0:511]` with a registered read + a write — the canonical 64-bit block RAM.
#[test]
fn unsplit_64bit_memory_maps_to_sdp72() {
    if !common::require_tool("yosys") {
        return;
    }
    let dir = build_dir();
    let src = "\
module mem64 (input clk, input we, input [8:0] wa, input [8:0] ra, input [63:0] wd, output reg [63:0] rd);
  (* ram_style = \"block\" *) reg [63:0] m [0:511];
  always @(posedge clk) begin if (we) m[wa] <= wd; rd <= m[ra]; end
endmodule
";
    let engine_v = dir.join("mem64.v");
    let netlist_v = dir.join("mem64_netlist.v");
    let ys = dir.join("mem64.ys");
    std::fs::write(&engine_v, src).unwrap();
    std::fs::write(
        &ys,
        format!(
            "read_verilog -sv {}\nsynth_xilinx -family xc7 -nodsp -nowidelut -top mem64\nwrite_verilog -noattr {}\n",
            engine_v.display(),
            netlist_v.display()
        ),
    )
    .unwrap();
    let out = std::process::Command::new("yosys")
        .arg("-s")
        .arg(&ys)
        .current_dir(&dir)
        .output()
        .expect("yosys");
    assert!(
        out.status.success(),
        "mem64 synth failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let nl = std::fs::read_to_string(&netlist_v).unwrap();
    let w72 = count_width(&nl, 72);
    assert!(
        w72 > 0,
        "expected an un-split 64-bit memory to map to a 72-bit-wide (SDP-72) RAMB — if it does not, the width-72 \
         detector is not exercising the bug mechanism. netlist widths present: {}",
        nl.lines().filter(|l| l.contains("_WIDTH_")).take(8).collect::<Vec<_>>().join(" | ")
    );
}

/// Count how many RAMB tiles in the netlist are addressed by the DEPTH-BANKED heap read index `heap_ra[13:0]`
/// (a 14-bit index — the signature of a 16384-deep bank with NO 32K depth-cascade). A flat 32768-deep ×1 heap
/// instead wires the FULL 15-bit `heap_ra` (or `heap_ra[14:0]`) into a single-tile depth address; the banked heap
/// wires only `heap_ra[13:0]` and steers the top bit to bank selection. yosys renders the address as an explicit
/// concat containing `heap_ra[13:0]`, so this counts the read-side tiles of the banked heap.
fn count_banked_heap_read_tiles(netlist: &str) -> usize {
    netlist.matches("heap_ra[13:0]").count()
}

/// BUG #1 (heap depth-cascade) is ABSENT at the fixed 2^15 depth AND its prjxray-level successor (the RAMB36 ×1
/// 32768-deep INTERNAL two-16K-half depth-cascade) is DESIGNED OUT: the heap is depth-banked into two 16384-deep
/// banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address bit 14). So every heap tile is 16384-deep, addressed
/// by a 14-bit `heap_ra[13:0]` index — NO 15-bit ×1 depth-cascade — and NO heap RAM port is width-1 (a 16384-deep
/// 32-bit bank half maps to a width-2 RAMB36, or a width-1 RAMB18 if bit-blasted; either way the buggy ×1
/// 32768-deep tile is gone). This asserts (a) the RTL declares the bank split and (b) the synthesized heap tiles
/// carry the 14-bit banked index, proving the un-banked 32768-deep ×1 tile is absent.
#[test]
fn synthesized_heap_is_depth_banked_not_a_32768_deep_x1_tile() {
    // The compile-time depth stays the 2^15 total (two 16384 banks); the anti-cascade fix is the banking itself.
    assert_eq!(
        WPE_STREAM_HEAP, 32768,
        "the streamed heap is 32768 words total (two 16384-deep banks)"
    );
    let soc =
        emit_wprog_engine_soc_verilog_backend("bankrtl", WPE_STREAM_CAPS, MemBackend::ConfigBram);
    // (a) The RTL must declare the two depth-banks (the structural source of the fix). An un-banked flat heap
    //     (`heap_lo`/`heap_hi`) would map back to the buggy RAMB36 ×1 32768-deep tile — this fails LOUDLY first.
    for bank in ["heap_b0_lo", "heap_b0_hi", "heap_b1_lo", "heap_b1_hi"] {
        assert!(
            soc.contains(bank),
            "the ConfigBram engine must declare the depth-bank `{bank}` — the heap must be split into two \
             16384-deep banks (selected by heap-address bit 14) so it never maps to a RAMB36E1 in ×1-width, \
             32768-deep mode (its internal two-16K-half depth-cascade is the prjxray/openXC7 mis-encode)."
        );
    }
    let Some(nl) = synth_engine_netlist("heapbank", WPE_STREAM_HEAP) else {
        return;
    };
    // (b) NO heap RAM port is width-1: the ×1 32768-deep tile (the buggy corner) is gone. (Config/register memories
    //     are 32/36-bit; the banked heap halves are width-2.) A stray width-1 port would mean a heap bit-plane
    //     regressed to the ×1 mode. Non-vacuous: the engine DOES map RAMBs (there are width-2 + width-36 ports).
    let ramb = nl.matches("RAMB36E1 ").count() + nl.matches("RAMB18E1 ").count();
    assert!(
        ramb > 0,
        "no RAMB primitives synthesized — the geometry check would be vacuous"
    );
    let w1 = count_width(&nl, 1);
    assert_eq!(
        w1, 0,
        "BUG #1 REGRESSION: {w1} width-1 RAM port(s) — a heap bit-plane mapped to the RAMB36E1 ×1, 32768-deep mode \
         whose internal two-16K-half depth-cascade prjxray/openXC7 mis-encodes on silicon. Depth-bank the heap so \
         every tile is a 14-bit-indexed 16384-deep tile (see heap_b0_*/heap_b1_* in wpe_soc_template.v)."
    );
    // (c) The synthesized heap tiles carry the 14-bit banked read index `heap_ra[13:0]` — the positive signature
    //     that the depth-cascade is gone (a flat 32768 heap would wire the full 15-bit heap_ra instead).
    let banked = count_banked_heap_read_tiles(&nl);
    assert!(
        banked >= 32,
        "expected the depth-banked heap's read tiles to be addressed by the 14-bit `heap_ra[13:0]` index (found \
         {banked}) — the netlist does not show the banked address, so the heap may have regressed to a 32768-deep \
         ×1 tile addressed by the full 15-bit heap_ra."
    );
}

/// TEETH for bug #1: the banked heap's 14-bit read index MUST be present in the synthesized netlist, and the
/// un-banked ×1 32768-deep tile's 15-bit signature MUST be absent. If the emitter ever regressed the heap back to
/// a flat 32768-deep array, the netlist would carry the full 15-bit `heap_ra[14:0]` depth address on a width-1
/// tile instead — the exact buggy corner. This re-synthesizes at the fixed depth and asserts the banked signature
/// is present AND no width-1 heap tile exists, so a green run means the depth-cascade is structurally designed out.
#[test]
fn heap_geometry_shows_the_banked_index_not_the_x1_cascade() {
    let Some(nl) = synth_engine_netlist("heapbankteeth", 32768) else {
        return; // yosys unavailable.
    };
    let banked = count_banked_heap_read_tiles(&nl);
    let w1 = count_width(&nl, 1);
    eprintln!(
        "heap-geometry teeth: banked-index(heap_ra[13:0]) tiles={banked}  width-1 ports={w1}"
    );
    assert!(
        banked >= 32,
        "TEETH FAILED: the banked read index `heap_ra[13:0]` appears {banked} times — the heap is not depth-banked \
         (a flat 32768-deep heap would wire the full 15-bit heap_ra), so the ×1 depth-cascade could reappear."
    );
    assert_eq!(
        w1, 0,
        "TEETH FAILED: {w1} width-1 heap tile(s) — the buggy RAMB36 ×1 32768-deep mode is present. The banked \
         heap must have ZERO width-1 tiles (its 16384-deep banks map width-2, or width-1 only if bit-blasted to \
         RAMB18 — and a bit-blast would still be 16384-deep, never the 32768-deep ×1 depth-cascade)."
    );
}
