//! xc7_image_flow_lock — the open 7-series flow that builds and records the decode board images (`scripts/xc7/`).
//!
//! A board image's ledger row (`benchmarks/silicon/decode_images.json`, `decode_split_images.json`) states what its
//! bitstream was gated on: the FASM holds every LUT-RAM's contents through its routed pins (`fasm_dram_check.py`), the
//! netlist feeds no DSP slice an unknown operand (`netlist_x_check.py`), and the core clock covers the routed path, the
//! residual the router bounds and the clock's own allowance (`image_record.py`). This lock holds each gate to the
//! defect it exists for, on designs built here to have it, and holds the flow to the rest of the tree:
//! - the netlist gate refuses a DSP48E1 operand tied to `x` and passes a known one;
//! - the FASM gate passes a RAM32M whose LUTs hold its INIT through their pins, and refuses one INIT bit flipped and
//!   a write address on other pins than its read address;
//! - the recorder refuses a core clock over the routed Fmax or inside the contract, an image that does not say how it
//!   was placed and routed, and one whose gates fail, and records one that passes them;
//! - the recorder's clock allowance is the compiler's (`CoreClock::UNCERTAINTY_NS`);
//! - the flow synthesizes as the gate-level lock simulates (`fabric_postsynth_lock`), and reaches yosys with every
//!   source of each board's image, in order: a Nexys A7's (no FT245 bridge), a Nexys Video's (the bridge), a split
//!   chip's (its chip protocol in place of the decode protocol);
//! - every patch in `scripts/openxc7-patches` applies, in name order, to the nextpnr-xilinx commit
//!   `scripts/setup-openxc7.sh` pins, the hard-cell timing among them (where the checkout is here).
//! - the patched router binds a clock pad's path to the global buffers it feeds before routing, over dedicated
//!   clock resources, so no route takes the pad's clock through a mux the device database cannot assemble.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test xc7_image_flow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::spatial::lower::fabric_board::CoreClock;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn xc7(script: &str) -> PathBuf {
    root().join("scripts/xc7").join(script)
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xc7_flow_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn python(args: &[&std::ffi::OsStr]) -> Output {
    Command::new("python3").args(args).output().expect("python3")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// A DSP48E1 instance as yosys writes it, its A operand `a`.
fn dsp(name: &str, a: &str) -> String {
    format!("  DSP48E1 #(\n    .AREG(32'd1)\n  ) {name} (\n    .A({a}),\n    .B(18'h00005),\n    .C(48'h000000000000),\n    .D(25'h0000000),\n    .P(p_{name})\n  );\n")
}

#[test]
fn the_netlist_gate_refuses_a_dsp_operand_tied_to_unknown() {
    if !common::require_tool("python3") {
        return;
    }
    let d = scratch("netlist");
    std::fs::write(d.join("netlist.v"), format!("module top;\n{}{}endmodule\n", dsp("known", "{ 5'h00, a }"), dsp("bottom", "30'hxxxxxxxx"))).unwrap();
    let o = python(&[xc7("netlist_x_check.py").as_os_str(), d.as_os_str()]);
    assert_eq!(o.status.code(), Some(1), "a slice fed `x` is refused:\n{}", text(&o));
    assert!(text(&o).contains("2 DSP48E1 slices, 1 with an unknown data operand") && text(&o).contains("bottom.A"), "{}", text(&o));
    std::fs::write(d.join("netlist.v"), format!("module top;\n{}endmodule\n", dsp("known", "{ 5'h00, a }"))).unwrap();
    let o = python(&[xc7("netlist_x_check.py").as_os_str(), d.as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "known operands pass:\n{}", text(&o));
    let _ = std::fs::remove_dir_all(&d);
}

/// The 64 INIT bits of the fixture's RAM32M port A, bit 0 first.
fn init_a() -> Vec<bool> {
    (0..64).map(|b| (b * 7 + 3) % 5 < 2).collect()
}

/// A routed design of one RAM32M, its port A split over the A6LUT and A5LUT of one SLICEM, as yosys and nextpnr-xilinx
/// write it: the netlist (`top.json`), the routed design (`top_routed.json`), prjxray's tilegrid for its tile under
/// `db/xc7a100t`, and the FASM the bitstream is built from — the 5LUT holding the even INIT bits at the addresses its
/// pins spell, the 6LUT the odd ones above them. `flip` inverts one FASM bit; `swap_writes` puts write address bits 0
/// and 1 on each other's pins.
fn lutram_design(d: &Path, db: &Path, flip: Option<usize>, swap_writes: bool) {
    let init = init_a();
    let msb_first = |bits: &[bool]| -> String { bits.iter().rev().map(|b| if *b { '1' } else { '0' }).collect() };
    let netlist = serde_json::json!({ "modules": { "top": {
        "attributes": { "top": "00000000000000000000000000000001" },
        "cells": { "ram0": { "type": "RAM32M", "parameters": { "INIT_A": msb_first(&init) } } },
        "netnames": {}
    } } });
    let half = |bel: &str| -> serde_json::Value {
        let mut attributes = serde_json::Map::new();
        attributes.insert("X_ORIG_TYPE".into(), "RAMD32".into());
        attributes.insert("NEXTPNR_BEL".into(), bel.into());
        let mut connections = serde_json::Map::new();
        for k in 0..5 {
            attributes.insert(format!("X_ORIG_PORT_A{}", k + 1), format!("RADR{k}").into());
            let w = if swap_writes && k < 2 { 1 - k } else { k };
            attributes.insert(format!("X_ORIG_PORT_WA{}", k + 1), format!("WADR{w}").into());
            connections.insert(format!("A{}", k + 1), serde_json::json!([10 + k]));
            connections.insert(format!("WA{}", k + 1), serde_json::json!([10 + k]));
        }
        connections.insert("A6".into(), serde_json::json!(["1"]));
        serde_json::json!({ "type": "SLICEM_LUT", "attributes": attributes, "connections": connections })
    };
    let routed = serde_json::json!({ "modules": { "top": {
        "attributes": { "top": "00000000000000000000000000000001" },
        "cells": { "ram0/DPR0_0": half("SLICE_X0Y0/A5LUT"), "ram0/DPR0_1": half("SLICE_X0Y0/A6LUT") },
        "netnames": (0..5).map(|k| (format!("addr{k}"), serde_json::json!({ "bits": [10 + k] }))).collect::<serde_json::Map<_, _>>()
    } } });
    let mut lut: Vec<bool> = (0..64).map(|p| if p < 32 { init[2 * p] } else { init[2 * (p - 32) + 1] }).collect();
    if let Some(p) = flip {
        lut[p] = !lut[p];
    }
    std::fs::write(d.join("top.json"), netlist.to_string()).unwrap();
    std::fs::write(d.join("top_routed.json"), routed.to_string()).unwrap();
    std::fs::write(d.join("top.fasm"), format!("CLBLM_L_X0Y0.SLICEM_X0.ALUT.INIT[63:0] = 64'b{}\n", msb_first(&lut))).unwrap();
    std::fs::create_dir_all(db.join("xc7a100t")).unwrap();
    let grid = serde_json::json!({ "CLBLM_L_X0Y0": { "sites": { "SLICE_X0Y0": "SLICEM", "SLICE_X1Y0": "SLICEL" } } });
    std::fs::write(db.join("xc7a100t/tilegrid.json"), grid.to_string()).unwrap();
}

fn fasm_check(d: &Path, db: &Path) -> Output {
    python(&[xc7("fasm_dram_check.py").as_os_str(), d.as_os_str(), "xc7a100t".as_ref(), db.as_os_str()])
}

#[test]
fn the_fasm_gate_holds_every_lut_ram_bit_and_address_pin_to_the_netlist() {
    if !common::require_tool("python3") {
        return;
    }
    let (d, db) = (scratch("fasm"), scratch("fasm_db"));
    lutram_design(&d, &db, None, false);
    let o = fasm_check(&d, &db);
    assert_eq!(o.status.code(), Some(0), "the LUTs hold the netlist's contents:\n{}", text(&o));
    assert!(text(&o).starts_with("1 LUT-RAM LUTs, 64 bits checked, 0 differ;"), "{}", text(&o));
    for p in [0, 31, 32, 63] {
        lutram_design(&d, &db, Some(p), false);
        let o = fasm_check(&d, &db);
        assert_eq!(o.status.code(), Some(1), "INIT bit {p} flipped is refused:\n{}", text(&o));
        assert!(text(&o).contains(&format!("bit {p}:")), "the flipped bit is named:\n{}", text(&o));
    }
    lutram_design(&d, &db, None, true);
    let o = fasm_check(&d, &db);
    assert_eq!(o.status.code(), Some(1), "a write address on other pins than its read address is refused:\n{}", text(&o));
    assert!(text(&o).contains("is read on A1 and written on WA2"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&db);
}

/// A routed image directory the recorder reads: its record at `core_mhz`, the route at 45.43 MHz with a 1.19 ns
/// residual, and the LUT-RAM design above as its netlist and FASM; `recipe` writes its pnr.recipe.
fn routed_image(d: &Path, db: &Path, core_mhz: f64, recipe: bool) {
    lutram_design(d, db, None, false);
    let rec = serde_json::json!({ "top": "logos_t", "part": "xc7a100tcsg324-1", "core_mhz": core_mhz, "pll": { "mult": 27, "div": 2, "out": 32 } });
    std::fs::write(d.join("image.json"), rec.to_string()).unwrap();
    if recipe {
        std::fs::write(d.join("pnr.recipe"), r#"{"placer": "heap", "seed": 2}"#).unwrap();
    } else {
        let _ = std::fs::remove_file(d.join("pnr.recipe"));
    }
    std::fs::write(
        d.join("pnr.log"),
        "Info: Hard-cell timing residual: 1.19 ns\nInfo: Device utilisation:\nInfo: \t        SLICE_LUTX:  100/ 126800\n\
         Info: Max frequency for clock               'coreclk': 45.43 MHz (PASS at 45.00 MHz)\n",
    )
    .unwrap();
    std::fs::write(d.join("stat.txt"), "   DSP48E1 1\n").unwrap();
    std::fs::write(d.join("netlist.v"), format!("module top;\n{}endmodule\n", dsp("known", "{ 5'h00, a }"))).unwrap();
    std::fs::write(d.join("logos_t.bit"), b"bitstream").unwrap();
}

/// `image_record.routed` over `d`: its exit and output.
fn record(d: &Path, db: &Path, silicon: &Path) -> Output {
    let script = format!(
        "import sys, json\nsys.path.insert(0, {xc7:?})\nfrom image_record import routed\nrec, common = routed({d:?}, {silicon:?}, 'x.bit', {db:?})\nprint(json.dumps(common))\n",
        xc7 = xc7("").display().to_string(),
        d = d.display().to_string(),
        silicon = silicon.display().to_string(),
        db = db.display().to_string(),
    );
    Command::new("python3").args(["-c", &script]).output().expect("python3")
}

#[test]
fn the_recorder_records_only_an_image_its_gates_and_clock_contract_pass() {
    if !common::require_tool("python3") {
        return;
    }
    let (d, db, silicon) = (scratch("record"), scratch("record_db"), scratch("record_silicon"));
    routed_image(&d, &db, 1000.0 / (1000.0 / 45.43 + 1.19 + CoreClock::UNCERTAINTY_NS) - 1e-6, true);
    let o = record(&d, &db, &silicon);
    assert_eq!(o.status.code(), Some(0), "an image within the contract is recorded:\n{}", text(&o));
    let common: serde_json::Value = serde_json::from_str(String::from_utf8_lossy(&o.stdout).lines().last().unwrap()).unwrap();
    assert_eq!(common["fmax_mhz"].as_f64(), Some(45.43));
    assert_eq!(common["timing"]["residual_ns"].as_f64(), Some(1.19));
    assert_eq!(common["timing"]["clock_uncertainty_ns"].as_f64(), Some(CoreClock::UNCERTAINTY_NS));
    assert_eq!(common["lutram_contents"]["bits_checked"].as_u64(), Some(64));
    assert!(common["lutram_contents"]["check"].as_str().unwrap().starts_with("scripts/xc7/fasm_dram_check.py:"), "the gate on record is the repository's");
    assert_eq!(std::fs::read(silicon.join("x.bit")).unwrap(), b"bitstream", "the bitstream is copied under its record's name");

    routed_image(&d, &db, 1000.0 / (1000.0 / 45.43 + 1.19 + CoreClock::UNCERTAINTY_NS) + 1e-3, true);
    let o = record(&d, &db, &silicon);
    assert_ne!(o.status.code(), Some(0), "a clock inside the contract is refused");
    assert!(text(&o).contains("shorter than the routed path"), "{}", text(&o));
    routed_image(&d, &db, 46.0, true);
    assert!(text(&record(&d, &db, &silicon)).contains("exceeds the routed Fmax"), "a clock over the routed Fmax is refused");
    routed_image(&d, &db, 40.0, false);
    assert!(text(&record(&d, &db, &silicon)).contains("no pnr.recipe"), "an image that does not say how it was routed is refused");
    routed_image(&d, &db, 40.0, true);
    std::fs::write(d.join("netlist.v"), format!("module top;\n{}endmodule\n", dsp("bottom", "30'hxxxxxxxx"))).unwrap();
    assert!(text(&record(&d, &db, &silicon)).contains("feeds a DSP slice an unknown operand"), "the netlist gate stops the record");
    routed_image(&d, &db, 40.0, true);
    lutram_design(&d, &db, Some(5), false);
    assert!(text(&record(&d, &db, &silicon)).contains("does not hold the netlist's LUT-RAM contents"), "the FASM gate stops the record");
    for p in [d, db, silicon] {
        let _ = std::fs::remove_dir_all(p);
    }
}

#[test]
fn the_recorder_holds_the_clock_allowance_the_compiler_states() {
    let src = std::fs::read_to_string(xc7("image_record.py")).expect("image_record.py");
    let line = src.lines().find(|l| l.starts_with("UNCERTAINTY_NS = ")).expect("the recorder states its allowance");
    let value: f64 = line["UNCERTAINTY_NS = ".len()..].trim().parse().expect("a number");
    assert_eq!(value, CoreClock::UNCERTAINTY_NS, "the recorder's clock allowance is CoreClock::UNCERTAINTY_NS");
}

#[test]
fn the_flow_synthesizes_as_the_gate_level_lock_simulates() {
    let recipe = "synth_xilinx -family xc7 -flatten -nowidelut";
    let flow = std::fs::read_to_string(xc7("synth-image.sh")).expect("synth-image.sh");
    let lock = std::fs::read_to_string(root().join("crates/logicaffeine_tests/tests/fabric_postsynth_lock.rs")).expect("fabric_postsynth_lock.rs");
    assert!(flow.contains(recipe), "the image flow synthesizes with `{recipe}`");
    assert!(lock.contains(recipe), "the gate-level lock simulates the netlist `{recipe}` writes");
}

/// Runs `synth-image.sh` over an image directory holding `files` under top `top`, with a `yosys` on the `PATH` that
/// records its arguments and writes the netlist the flow expects, and returns the script's output and what yosys read.
fn synthesize_stubbed(name: &str, top: &str, files: &[&str]) -> (Output, Option<String>) {
    let d = scratch(name);
    for f in files {
        std::fs::write(d.join(f), format!("// {f}\n")).unwrap();
    }
    let bin = d.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let yosys = bin.join("yosys");
    std::fs::write(&yosys, "#!/bin/sh\nprintf '%s\\n' \"$@\" > yosys.args\necho '{}' > top.json\n").unwrap();
    std::fs::set_permissions(&yosys, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new("bash").arg(xc7("synth-image.sh")).arg(&d).arg(top).env("PATH", path).output().expect("bash");
    let args = std::fs::read_to_string(d.join("yosys.args")).ok();
    let _ = std::fs::remove_dir_all(&d);
    (out, args)
}

#[test]
fn the_flow_synthesizes_every_source_of_each_board_s_image() {
    let boards: [(&str, &str, &[&str], &str); 3] = [
        ("a7", "logos_decode_a7", &["logos_decode_a7.v", "decode_board.v", "spatial_top.v"], "logos_decode_a7.v decode_board.v spatial_top.v"),
        (
            "video",
            "logos_decode_video",
            &["logos_decode_video.v", "decode_board.v", "spatial_top.v", "ft245_uart_bridge.v"],
            "logos_decode_video.v decode_board.v spatial_top.v ft245_uart_bridge.v",
        ),
        ("chip", "logos_chip_a7", &["logos_chip_a7.v", "chip_board.v", "spatial_top.v"], "logos_chip_a7.v chip_board.v spatial_top.v"),
    ];
    for (name, top, files, read) in boards {
        let (out, args) = synthesize_stubbed(name, top, files);
        assert!(out.status.success(), "{name}: synth-image.sh synthesizes the image:\n{}", text(&out));
        let args = args.unwrap_or_else(|| panic!("{name}: synth-image.sh never ran yosys:\n{}", text(&out)));
        let script = args.lines().skip_while(|a| *a != "-p").nth(1).unwrap_or_else(|| panic!("{name}: yosys takes a script: {args}"));
        assert!(script.starts_with(&format!("read_verilog {read};")), "{name}: yosys reads `{read}`, in that order: {script}");
        assert!(script.contains(&format!("-top {top};")), "{name}: yosys synthesizes the top `{top}`: {script}");
    }
}

/// The nextpnr-xilinx source `scripts/setup-openxc7.sh` builds: the commit it pins, unpacked into `scratch(name)`, every
/// patch in `scripts/openxc7-patches` applied in name order, each asserted to apply after the ones before it. `None`
/// where the checkout or `patch` is not here.
fn patched_nextpnr(name: &str) -> Option<PathBuf> {
    if !common::require_tool("openxc7-source") || !common::require_tool("patch") {
        return None;
    }
    let setup = std::fs::read_to_string(root().join("scripts/setup-openxc7.sh")).expect("setup-openxc7.sh");
    let commit = setup
        .lines()
        .find_map(|l| l.strip_prefix("NEXTPNR_XILINX_COMMIT="))
        .expect("the setup pins a nextpnr-xilinx commit")
        .trim();
    let source = root().join("tools/openxc7/nextpnr-xilinx");
    let tree = scratch(name);
    let archive = Command::new("git").arg("-C").arg(&source).args(["archive", commit]).output().expect("git archive");
    assert!(archive.status.success(), "the checkout holds {commit}: {}", String::from_utf8_lossy(&archive.stderr));
    let mut tar = Command::new("tar").arg("-x").arg("-C").arg(&tree).stdin(std::process::Stdio::piped()).spawn().expect("tar");
    std::io::Write::write_all(tar.stdin.as_mut().unwrap(), &archive.stdout).unwrap();
    assert!(tar.wait().unwrap().success(), "the source unpacks");
    let mut patches: Vec<PathBuf> = std::fs::read_dir(root().join("scripts/openxc7-patches"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "patch"))
        .collect();
    patches.sort();
    for p in &patches {
        let applied = Command::new("patch").args(["-p1", "--quiet", "--no-backup-if-mismatch", "-i"]).arg(p).current_dir(&tree).output().expect("patch");
        assert!(applied.status.success(), "{} applies after the patches before it:\n{}", p.display(), text(&applied));
    }
    Some(tree)
}

#[test]
fn every_patch_applies_in_name_order_to_the_nextpnr_the_setup_pins() {
    let Some(tree) = patched_nextpnr("nextpnr") else { return };
    let patches = root().join("scripts/openxc7-patches");
    assert!(patches.join("nextpnr-xc7-hard-cell-timing.patch").is_file(), "the hard-cell timing is among the patches");
    assert!(tree.join("xilinx/hard_timing.cc").is_file(), "the patched source reads the hard cells' timing");
    let _ = std::fs::remove_dir_all(&tree);
}

/// A clock pad's input buffer feeding only global buffers is routed before the router runs, over the dedicated clock
/// resources alone, as a PLL's input already is (`Arch::routeClock`). Left to router2, a congested image takes the
/// pad's clock through a regional buffer's mux in the IO clock tile (`HCLK_IOI3 … PLL_CLK0_DMUX ← I2IOCLK_TOP0`) that
/// prjxray-db has no bits for, and `fasm2frames` cannot assemble the image: the decision fabric's SoC on the Nexys
/// A7-100T routed on every seed and assembled on none.
#[test]
fn the_router_binds_a_clock_pads_path_to_its_global_buffers_before_routing() {
    let Some(tree) = patched_nextpnr("nextpnr_pad_clock") else { return };
    let arch = std::fs::read_to_string(tree.join("xilinx/arch.cc")).expect("xilinx/arch.cc");
    let start = arch.find("void Arch::routeClock()").expect("the clock pre-router");
    let end = start + arch[start..].find("log_info(\"    routing clock").expect("the pre-router's global-net test");
    let selection = &arch[start..end];
    assert!(
        selection.contains("id_IOB33_INBUF_EN") && selection.contains("id_IOB18_INBUF_DCIEN"),
        "a clock pad's input buffer, either bank type, is among the nets the pre-router binds:\n{selection}"
    );
    assert!(
        selection.contains("id_BUFGCTRL") && selection.contains("id_BUFG_BUFG"),
        "it binds the pad's net when global buffers are all it feeds:\n{selection}"
    );
    let _ = std::fs::remove_dir_all(&tree);
}
