//! fabric_terminal_lock — the Fabric Terminal: a LOGOS editor on Board B's VGA screen, a keyboard on its USB port,
//! and the fabric engine running each program, behind one UART to the host that compiles.
//!
//! The protocol (`codegen_sva::terminal_protocol`) is held to one meaning per byte and a self-synchronizing
//! framing. The RTL (`codegen_sva::terminal_rtl`) is held, under iverilog, to the things a person sees and does:
//! - the PS/2 port reads a keyboard and enables a mouse at the protocol's own timing;
//! - every pixel of the frames the screen draws equals `terminal_rtl::render` of what the host wrote, in both
//!   flash phases;
//! - the board answers PING, reads cells back, reports keys, and shows the RESULT on the 7-segment display;
//! - a compiled LOGOS program sent through the link runs on the fabric engine and answers what the VM answers,
//!   including after a load abandoned halfway;
//! - the activity panel counts every engine clock a program runs once, in one frame, by block and by cell, against
//!   a count taken in the engine's own clock domain, and every pixel it draws equals `render`'s panel — for probes
//!   the test drives through every case, and for an engine running a program across frames.
//! The engine's probes are additions only: without them the engine is the deployed one. The files the board build
//! reads from `benchmarks/silicon/` are the emitter's, byte for byte. The host (`scripts/logos-terminal.py`) passes
//! its selftests and runs end to end against a model of the board's link, with a keyboard, with a mouse, and with
//! each told apart by what it answers.

mod common;

#[path = "common/gate_level.rs"]
mod gate_level;

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::terminal_protocol::{
    self as tp, board, host, Cell, SlipDecoder, COLS, PALETTE, ROWS, SLIP_END, SLIP_ESC,
};
use logicaffeine_compile::codegen_sva::terminal_rtl::{self, render, Panel, Screen, ENGINE_FILE, ENGINE_MODULE};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_hw, emit_wprog_engine_soc_verilog_hw_probed, eval_wprog_config, eval_wprog_config_run,
    program_main_fabric, program_output, serialize_program, wprog_config_image_unchecked, WPE_HW_DEPTH, WPE_HW_HEAP,
    WPE_HW_MAXBLK, WPE_HW_MAXCELL,
};
use logicaffeine_compile::compile::vm_outcome;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

fn asset(name: &str) -> String {
    std::fs::read_to_string(root().join("crates/logicaffeine_tests/tests/assets/terminal").join(name)).expect(name)
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fabric_terminal_lock_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// A deterministic byte source for generated cases (SplitMix64).
struct Mix(u64);
impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

// ── the protocol ────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_frame_kind_has_one_meaning_and_direction() {
    let mut seen = std::collections::HashSet::new();
    for (name, k) in host::ALL {
        assert!(k < 0x80, "host kind {name} = {k:#04x} is in the board's half");
        assert!(seen.insert(k), "kind {k:#04x} has two meanings ({name})");
    }
    for (name, k) in board::ALL {
        assert!(k >= 0x80, "board kind {name} = {k:#04x} is in the host's half");
        assert!(seen.insert(k), "kind {k:#04x} has two meanings ({name})");
    }
    for k in seen {
        assert!(k != SLIP_END && k != SLIP_ESC, "kind {k:#04x} is a SLIP control byte");
    }
}

#[test]
fn slip_round_trips_every_byte_and_resynchronizes_after_damage() {
    let mut mix = Mix(7);
    let mut stream = Vec::new();
    let mut frames = Vec::new();
    for n in 0..200 {
        let len = 1 + (mix.next() % 300) as usize;
        let frame: Vec<u8> = (0..len)
            .map(|i| match (n + i) % 7 {
                0 => SLIP_END,
                1 => SLIP_ESC,
                _ => mix.byte(),
            })
            .collect();
        stream.extend(tp::slip_encode(&frame));
        frames.push(frame);
    }
    assert_eq!(SlipDecoder::new().decode_all(&stream), frames, "SLIP lost or changed a frame");

    // Garbage before the first END, a frame with a bad escape, then a good frame: only the good one survives.
    let mut damaged = vec![0x11, 0x22, SLIP_END];
    damaged.extend_from_slice(&[0x01, SLIP_ESC, 0x99, 0x02, SLIP_END]);
    damaged.extend(tp::slip_encode(&[host::PING]));
    assert_eq!(SlipDecoder::new().decode_all(&damaged), vec![vec![0x11, 0x22], vec![host::PING]]);
    assert!(tp::slip_encode(&[SLIP_END, SLIP_ESC]).iter().filter(|&&b| b == SLIP_END).count() == 1, "an END inside a frame is escaped");
}

#[test]
fn the_table_a_host_reads_is_the_protocol() {
    let t: serde_json::Value = serde_json::from_str(&tp::table_json()).expect("the table is JSON");
    assert_eq!(t["slip"]["end"], SLIP_END);
    assert_eq!(t["slip"]["esc"], SLIP_ESC);
    for (name, k) in host::ALL {
        assert_eq!(t["host"][name], k, "host.{name}");
    }
    for (name, k) in board::ALL {
        assert_eq!(t["board"][name], k, "board.{name}");
    }
    assert_eq!(t["screen"]["cols"], COLS);
    assert_eq!(t["screen"]["rows"], ROWS);
    assert_eq!(t["screen"]["palette"].as_array().expect("palette").len(), PALETTE.len());
    assert_eq!(t["baud"], tp::BAUD);
    assert_eq!(t["engine_frame_max"], tp::ENGINE_FRAME_MAX);
    assert_eq!(t["features"]["engine"], tp::FEATURE_ENGINE);
    assert_eq!(t["features"]["small"], tp::FEATURE_SMALL);
    assert_eq!(t["features"]["totals"], tp::FEATURE_TOTALS);
    let features = [tp::FEATURE_ENGINE, tp::FEATURE_SMALL, tp::FEATURE_TOTALS];
    assert!(features.iter().all(|f| f.count_ones() == 1), "each feature is one bit");
    assert_eq!(features.iter().fold(0, |a, f| a | f).count_ones() as usize, features.len(), "and no two share it");
    assert_eq!(t["pong"], serde_json::json!(tp::PONG_FIELDS), "PONG's fields, in order");
    let layout: Vec<(String, usize, usize)> = t["activity"]
        .as_array()
        .expect("the ACTIVITY layout")
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), f["bytes"].as_u64().unwrap() as usize, f["count"].as_u64().unwrap() as usize))
        .collect();
    let want: Vec<(String, usize, usize)> = tp::ACTIVITY_FIELDS.iter().map(|&(n, b, c)| (n.to_string(), b, c)).collect();
    assert_eq!(layout, want, "ACTIVITY's fields, in order");
    assert_eq!(tp::activity_bytes(), 15 + 2 * (WPE_HW_MAXBLK + WPE_HW_MAXCELL) + 2, "ACTIVITY's payload length");
    let layout: Vec<(String, usize, usize)> = t["totals"]
        .as_array()
        .expect("the TOTALS layout")
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), f["bytes"].as_u64().unwrap() as usize, f["count"].as_u64().unwrap() as usize))
        .collect();
    let want: Vec<(String, usize, usize)> = tp::TOTALS_FIELDS.iter().map(|&(n, b, c)| (n.to_string(), b, c)).collect();
    assert_eq!(layout, want, "TOTALS's fields, in order");
    assert_eq!(tp::totals_bytes(), 4 + 4 * (WPE_HW_MAXBLK + WPE_HW_MAXCELL), "TOTALS's payload length");
    assert_eq!(t["screen"]["ui_glyphs"], tp::UI_GLYPHS);
    assert_eq!(t["screen"]["del_glyph"], tp::DEL_GLYPH.to_string());
    assert_eq!(t["panel"]["blocks"], WPE_HW_MAXBLK);
    assert_eq!(t["panel"]["cells"], WPE_HW_MAXCELL);
}

#[test]
fn a_cell_is_its_two_bytes() {
    for glyph in 0..128u8 {
        for (fg, bg, blink) in [(7, 0, false), (14, 1, true), (15, 15, false), (0, 4, true)] {
            let c = Cell { glyph, blink, fg, bg };
            let [attr, g] = c.bytes();
            assert_eq!(Cell::from_bytes(attr, g), c);
        }
    }
    assert_eq!(tp::cell_address(29, 79), (29 << 7) | 79);
}

#[test]
#[should_panic(expected = "terminal_protocol does not define")]
fn a_template_cannot_name_a_value_the_protocol_does_not_own() {
    tp::fill_verilog("localparam X = __V_H_TELEPORT__;");
}

// ── the font and the emitted files ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_font_rom_holds_128_glyphs_and_its_license() {
    let font = terminal_rtl::font();
    assert_eq!(font.len(), 128 * 16, "128 glyphs of 16 rows");
    assert!(font[b' ' as usize * 16..][..16].iter().all(|&r| r == 0), "space is blank");
    for ch in b'!'..=b'~' {
        assert!(font[ch as usize * 16..][..16].iter().any(|&r| r != 0), "{} has no pixels", ch as char);
    }
    assert_eq!(tp::UI_GLYPHS.chars().count(), 32, "the UI glyphs fill the slots below the printable range");
    let rows = |ch: char| &font[tp::glyph_slot(ch) as usize * 16..][..16];
    let halves = |top: u8, bottom: u8| [[top; 8], [bottom; 8]].concat();
    assert_eq!(rows('█'), &[0xFF; 16], "full block");
    assert_eq!(rows('▀'), &halves(0xFF, 0x00)[..], "upper half block");
    assert_eq!(rows('▄'), &halves(0x00, 0xFF)[..], "lower half block");
    assert_eq!(rows('▌'), &[0xF0; 16], "left half block");
    assert_eq!(rows('▐'), &[0x0F; 16], "right half block");
    let lit = |ch: char| rows(ch).iter().map(|r| r.count_ones()).sum::<u32>();
    assert!(lit('░') < lit('▒') && lit('▒') < lit('▓') && lit('▓') < lit('█'), "the shades darken in order");
    assert!(rows(tp::DEL_GLYPH).iter().any(|&r| r != 0), "slot 0x7F draws {}", tp::DEL_GLYPH);
    assert_eq!((tp::glyph_slot('A'), tp::glyph_slot('┌'), tp::glyph_slot('■'), tp::glyph_slot('€')), (b'A', 2, 0x7F, b'?'));
    assert!(terminal_rtl::FONT_LICENSE.contains("Permission is hereby granted, free of charge"), "the font's license travels with it");
}

#[test]
fn the_board_build_reads_the_emitters_files() {
    let sil = root().join("benchmarks/silicon");
    for (name, text) in terminal_rtl::board_files() {
        let on_disk = std::fs::read_to_string(sil.join(&name)).unwrap_or_else(|_| {
            panic!("benchmarks/silicon/{name} is missing — run `cargo run -p logicaffeine-compile --example fabric_terminal -- --emit benchmarks/silicon`")
        });
        assert!(on_disk == text, "benchmarks/silicon/{name} is not what the emitter writes — re-emit it");
    }
}

#[test]
fn every_placeholder_is_filled_and_each_top_keeps_only_its_engine_section() {
    for (name, text) in terminal_rtl::board_files() {
        for marker in ["__V_", "__X_", "__N_", "__C_", "__PALETTE", "__HEAT", "__DIGIT", "__TOP__", "__ENGINE_MODULE__", "//__"] {
            assert!(!text.contains(marker), "{name} still holds {marker}");
        }
    }
    let with = terminal_rtl::top("t", Some(ENGINE_MODULE));
    let without = terminal_rtl::top("t", None);
    assert!(with.contains(&format!("{ENGINE_MODULE} #(")) && with.contains("term_activity activity"));
    assert!(!with.contains("ENGINE bytes are taken at once"));
    assert!(!without.contains(ENGINE_MODULE) && !without.contains("term_activity"));
    assert!(without.contains("ENGINE bytes are taken at once"));
    let names = |engine| terminal_rtl::modules(engine).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert!(names(true).contains(&"term_activity.v") && !names(false).contains(&"term_activity.v"), "the panel is the engine top's");
}

#[test]
fn the_probes_are_additions_and_without_them_the_engine_is_the_deployed_one() {
    let plain = emit_wprog_engine_soc_verilog_hw("fpga");
    let probed = emit_wprog_engine_soc_verilog_hw_probed("fpga");
    assert!(!plain.contains("probe_"), "the deployed engine has no probe ports");
    let plain_lines: Vec<&str> = plain.lines().collect();
    let mut at = 0;
    let mut added = Vec::new();
    for line in probed.lines() {
        if at < plain_lines.len() && line == plain_lines[at] {
            at += 1;
        } else {
            added.push(line);
        }
    }
    assert_eq!(at, plain_lines.len(), "every line of the deployed engine is in the probed one, in order");
    let ports = ["probe_running", "probe_st", "probe_pc", "probe_cell", "probe_sp", "probe_hp", "probe_cycles"];
    for port in ports {
        assert_eq!(added.iter().filter(|l| l.trim_start().starts_with("output") && l.contains(port)).count(), 1, "{port} is one output");
        assert_eq!(added.iter().filter(|l| l.trim_start().starts_with(&format!("assign {port} "))).count(), 1, "{port} is driven once");
    }
    for line in &added {
        let t = line.trim_start();
        assert!(
            t.starts_with("//") || ports.iter().any(|p| t.contains(p)),
            "the probes add only their ports, their assignments and their comment: {line:?}"
        );
    }
    assert!(
        added.iter().all(|l| !l.contains("<=") && !l.contains("always")),
        "a probe reads the engine and never drives it"
    );
}

#[test]
fn the_panel_fits_the_screen_and_a_frame_fits_its_counters() {
    use tp::{
        PANEL_BLOCK_COL, PANEL_BLOCK_ROW, PANEL_BLOCK_TILE, PANEL_CELL_COL, PANEL_CELL_ROW, PANEL_CYCLES_COL,
        PANEL_HEAP_COL, PANEL_LABEL_ROW, PANEL_STACK_COL, PANEL_TITLE_ROW,
    };
    let width = tp::H_TIMING.0 as usize;
    let end = |col: u8, px: usize| col as usize * 8 + px;
    assert_eq!(PANEL_TITLE_ROW, ROWS - tp::PANEL_ROWS, "the panel is the bottom rows");
    let mut rows = vec![PANEL_TITLE_ROW, PANEL_LABEL_ROW, PANEL_BLOCK_ROW, PANEL_CELL_ROW];
    rows.sort();
    rows.dedup();
    assert_eq!(rows.len(), tp::PANEL_ROWS as usize, "one panel row each");
    assert!(rows.iter().all(|&r| r < ROWS));
    assert!(end(PANEL_CYCLES_COL, 80) <= width, "ten digits fit the title row");
    assert!(end(PANEL_BLOCK_COL, WPE_HW_MAXBLK * PANEL_BLOCK_TILE as usize * 8) <= PANEL_STACK_COL as usize * 8, "the tiles end before the stack");
    assert!(end(PANEL_STACK_COL, WPE_HW_DEPTH * 8) <= PANEL_HEAP_COL as usize * 8, "the stack ends before the heap");
    assert!(end(PANEL_HEAP_COL, WPE_HW_HEAP) <= width, "the heap fits the row");
    assert!(end(PANEL_CELL_COL, WPE_HW_MAXCELL * 16) <= width, "the cells fit the row");
    assert!(tp::panel_frame_samples() < 1 << 14, "a frame of samples fits the panel's 14-bit counters");
    assert!(WPE_HW_MAXBLK + WPE_HW_MAXCELL < tp::ENGINE_CLOCK_DIVIDER as usize, "the sweep ends before the next sample");
    assert!(tp::heat_level(tp::panel_frame_samples()) < tp::HEAT.len(), "the hottest frame has a colour");
    assert_eq!((tp::heat_level(0), tp::heat_level(1), tp::heat_level(2), tp::heat_level(3), tp::heat_level(4)), (0, 1, 2, 2, 3));
    assert_eq!(tp::heat_level(8191), 13);
    assert_eq!(tp::heat_level(8192), 14);
}

/// Every block RAM in `netlist` (yosys's `write_verilog`), as (instance, its port connections, its parameters).
fn block_rams(netlist: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for kind in ["RAMB18E1", "RAMB36E1"] {
        let mut rest = netlist;
        while let Some(at) = rest.find(&format!("{kind} #(")) {
            let cell = &rest[at..];
            let params_end = cell.find(") ").expect("a parameter list");
            let (params, after) = (&cell[..params_end], &cell[params_end + 2..]);
            let name: String = after.split_whitespace().next().unwrap_or("").to_string();
            let ports = &after[..after.find(");").expect("a port list")];
            out.push((name, ports.to_string(), params.to_string()));
            rest = &rest[at + kind.len()..];
        }
    }
    out
}

/// The text a RAMB port is connected to, with its constants removed: what is left names the nets it carries.
fn nets_on(ports: &str, port: &str) -> String {
    let Some(at) = ports.find(&format!(".{port}(")) else { return String::new() };
    let body = &ports[at + port.len() + 2..];
    let mut depth = 1;
    let end = body.char_indices().find(|&(_, c)| {
        depth += (c == '(') as i32 - (c == ')') as i32;
        depth == 0
    });
    let conn = &body[..end.map_or(body.len(), |(i, _)| i)];
    let mut left = String::new();
    for word in conn.split(|c: char| c == '{' || c == '}' || c == ',' || c.is_whitespace()) {
        let constant = word.split_once('\'').is_some_and(|(width, digits)| {
            width.chars().all(|c| c.is_ascii_digit())
                && digits.len() > 1
                && "bhdBHD".contains(&digits[..1])
                && digits[1..].chars().all(|c| c.is_ascii_hexdigit() || "xXzZ_".contains(c))
        });
        if !word.is_empty() && !constant {
            left.push_str(word);
            left.push(' ');
        }
    }
    left
}

#[test]
fn the_terminals_block_rams_carry_no_bit_in_a_parity_lane() {
    if !common::require_tool("yosys") {
        return;
    }
    // The open flow's bitstream does not program a RAMB's parity lane the way the device reads it, so a bit kept
    // there reads back wrong on silicon. Synthesized as the board build synthesizes them, the terminal's memories
    // keep every bit in the data lanes.
    let dir = scratch("parity_lanes");
    let mut sources = Vec::new();
    for (name, text) in terminal_rtl::modules(true) {
        std::fs::write(dir.join(name), text).unwrap();
        sources.push(name.to_string());
    }
    write_fonts(&dir);
    for top in ["term_textbuf", "term_font", "term_link"] {
        let synth = Command::new("yosys")
            .arg("-q")
            .arg("-p")
            .arg(format!(
                "read_verilog {}; synth_xilinx -family xc7 -nodsp -flatten -top {top}; write_verilog -noattr {top}.v",
                sources.join(" ")
            ))
            .current_dir(&dir)
            .output()
            .expect("yosys");
        assert!(synth.status.success(), "synthesizing {top} failed:\n{}", String::from_utf8_lossy(&synth.stderr));
        let netlist = std::fs::read_to_string(dir.join(format!("{top}.v"))).unwrap();
        let rams = block_rams(&netlist);
        assert!(!rams.is_empty(), "{top}'s memory is block RAM");
        for (name, ports, params) in &rams {
            for port in ["DIPADIP", "DIPBDIP"] {
                let nets = nets_on(ports, port);
                assert!(nets.is_empty(), "{top}: {name} writes `{nets}` through its parity lane {port}");
            }
            // A parity init of zeros or of x (no memory bit lives there) keeps nothing.
            for init in params.split(".INITP_").skip(1) {
                let value = init.split_once('(').map_or("", |(_, v)| v.split(')').next().unwrap_or(""));
                let digits = value.split_once('\'').map_or(value, |(_, d)| &d[1..]);
                assert!(
                    digits.chars().all(|c| "0xX_".contains(c)),
                    "{top}: {name} keeps data in its parity init (INITP_{init:.40})"
                );
            }
        }
    }
}

// ── simulation ──────────────────────────────────────────────────────────────────────────────────────────────

/// Write a simulation: the modules (the activity panel's with `engine`), the font, the Xilinx I/O models, the
/// 7-segment driver, `extra` files, and `tb`; compile it with iverilog and `defines`, and run it in `dir`. Returns
/// vvp's stdout.
fn simulate(dir: &Path, top_text: Option<&str>, engine: bool, extra: &[&str], defines: &[&str], tb: &str) -> String {
    let mut sources = vec!["xilinx_io_behav.v".to_string(), "sevenseg_dec.v".to_string()];
    std::fs::write(dir.join("xilinx_io_behav.v"), asset("xilinx_io_behav.v")).unwrap();
    std::fs::copy(root().join("benchmarks/silicon/sevenseg_dec.v"), dir.join("sevenseg_dec.v")).unwrap();
    for f in extra {
        std::fs::copy(root().join("benchmarks/silicon").join(f), dir.join(f)).unwrap();
        sources.push(f.to_string());
    }
    for (name, text) in terminal_rtl::modules(engine) {
        std::fs::write(dir.join(name), text).unwrap();
        sources.push(name.to_string());
    }
    write_fonts(&dir);
    if let Some(t) = top_text {
        std::fs::write(dir.join("top.v"), t).unwrap();
        sources.push("top.v".into());
    }
    std::fs::write(dir.join("tb.v"), tb).unwrap();
    sources.push("tb.v".into());
    let build = Command::new("iverilog")
        .arg("-g2012")
        .args(defines.iter().map(|d| format!("-D{d}")))
        .arg("-o")
        .arg("sim.vvp")
        .args(&sources)
        .current_dir(dir)
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog failed:\n{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").arg("sim.vvp").current_dir(dir).output().expect("vvp");
    assert!(run.status.success(), "vvp failed:\n{}", String::from_utf8_lossy(&run.stderr));
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn the_ps2_port_reads_a_keyboard_and_enables_a_mouse_at_protocol_timing() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = scratch("ps2");
    let out = simulate(&dir, None, false, &[], &[], &tp::fill_verilog(&asset("tb_term_ps2.v")));
    assert!(out.contains("DONE"), "the PS/2 testbench did not finish:\n{out}");
    assert!(!out.contains("FAIL"), "{out}");
    assert_eq!(out.matches("PASS:").count(), 15, "every PS/2 check reports:\n{out}");
}

/// The font ROMs the RTL reads by `$readmemh`, written beside a simulation or a synthesis: the board's own files.
fn write_fonts(dir: &Path) {
    for (name, text) in terminal_rtl::board_files() {
        if name.starts_with("term_font") && name.ends_with(".hex") {
            std::fs::write(dir.join(&name), text).unwrap();
        }
    }
}

/// The PONG frame a board answers with, its payload laid out by `PONG_FIELDS`: the image's `features`, and the
/// PS/2 line's clock `edges`, shortest clock-low phase `low` (us), bytes read `whole` and `damaged`.
fn pong(features: u8, edges: u16, low: u8, whole: u16, damaged: u8) -> Vec<u8> {
    let mut f = vec![board::PONG];
    for name in tp::PONG_FIELDS {
        f.push(match name {
            "version" => tp::VERSION,
            "cols" => COLS,
            "rows" => ROWS,
            "panel_rows" => tp::PANEL_ROWS,
            "features" => features,
            "ps2_edges_hi" => (edges >> 8) as u8,
            "ps2_edges_lo" => edges as u8,
            "ps2_shortest_low_us" => low,
            "ps2_bytes_hi" => (whole >> 8) as u8,
            "ps2_bytes_lo" => whole as u8,
            "ps2_damaged" => damaged,
            other => panic!("PONG field {other} has no expectation here"),
        });
    }
    f
}

/// A host script for `tb_term_top.v`, and the board's log it produces, decoded.
struct Script(Vec<String>);
impl Script {
    fn new() -> Self {
        Script(Vec::new())
    }
    fn frame(&mut self, f: &[u8]) {
        self.0.extend(tp::slip_encode(f).iter().map(|b| format!("B {b:02x}")));
    }
    fn step(&mut self, s: &str) {
        self.0.push(s.to_string());
    }
}

/// The board's reports, in order, and the testbench's own log lines.
fn board_log(dir: &Path) -> (Vec<Vec<u8>>, Vec<String>) {
    let log = std::fs::read_to_string(dir.join("board.txt")).expect("board.txt");
    let bytes: Vec<u8> = log.lines().filter_map(|l| l.strip_prefix("RX ")).map(|h| u8::from_str_radix(h, 16).unwrap()).collect();
    let notes = log.lines().filter(|l| !l.starts_with("RX ")).map(str::to_string).collect();
    (SlipDecoder::new().decode_all(&bytes), notes)
}

/// Simulate the terminal top `top` — with the probed fabric engine (`ENGINE_MODULE`, read from `ENGINE_FILE` in
/// `benchmarks/silicon/`) or without one — under `script`.
fn run_top(tag: &str, top: &str, engine: bool, script: &Script) -> (PathBuf, Vec<Vec<u8>>, Vec<String>) {
    let dir = scratch(tag);
    std::fs::write(dir.join("script.txt"), script.0.join("\n") + "\n").unwrap();
    let tb = tp::fill_verilog(&asset("tb_term_top.v").replace("__TOP__", top));
    let (module, extra, defines): (Option<&str>, Vec<&str>, Vec<&str>) =
        if engine { (Some(ENGINE_MODULE), vec![ENGINE_FILE], vec!["TERM_ENGINE"]) } else { (None, vec![], vec![]) };
    simulate(&dir, Some(&terminal_rtl::top(top, module)), engine, &extra, &defines, &tb);
    let (frames, notes) = board_log(&dir);
    assert!(notes.iter().any(|n| n == "DONE"), "the terminal testbench did not finish: {notes:?}");
    (dir, frames, notes)
}

/// The screen test's host session — a PING; FILLs, text in every UI glyph, a blinking run and a PUT that wraps into
/// the next row; a cursor, a pointer and a RESULT; two whole frames; a READ; three keys; a mouse-enable send; and a
/// whole ENGINE frame through the link's engine queue — and the screen it leaves.
fn screen_session() -> (Script, Screen) {
    let mut s = Script::new();
    let mut screen = Screen::blank();
    let put = |s: &mut Script, screen: &mut Screen, row: u8, col: u8, text: &[u8], fg: u8, bg: u8, blink: bool| {
        let mut f = vec![host::PUT, row, col];
        for (i, &g) in text.iter().enumerate() {
            let c = Cell { glyph: g & 0x7F, blink, fg, bg };
            f.extend(c.bytes());
            let at = row as usize * COLS as usize + col as usize + i;
            if at < screen.cells.len() {
                screen.cells[at] = c;
            }
        }
        s.frame(&f);
    };
    s.frame(&[host::PING]);
    s.frame(&[host::FILL, 0, 0, ROWS, COLS, 0x17, b' ']);
    for c in screen.cells.iter_mut() {
        *c = Cell::new(b' ', 7, 1);
    }
    put(&mut s, &mut screen, 1, 2, b"LOGOS fabric terminal", 14, 1, false);
    put(&mut s, &mut screen, 3, 2, &[0x02, 0x00, 0x00, 0x00, 0x03], 15, 1, false);
    put(&mut s, &mut screen, 4, 2, &[0x01, b' ', b'A', b' ', 0x01], 15, 1, false);
    put(&mut s, &mut screen, 5, 2, &[0x04, 0x00, 0x00, 0x00, 0x05], 15, 1, false);
    put(&mut s, &mut screen, 7, 40, b"blink", 15, 4, true);
    put(&mut s, &mut screen, 12, 70, b"wraps into the next row", 10, 0, false);
    s.frame(&[host::FILL, 20, 5, 3, 10, 0x2E, 0x11]);
    for r in 20..23 {
        for c in 5..15 {
            screen.cells[r * COLS as usize + c] = Cell::new(0x11, 14, 2);
        }
    }
    s.frame(&[host::FILL, 28, 75, 5, 10, 0x4F, b'#']);
    for r in 28..30 {
        for c in 75..80 {
            screen.cells[r * COLS as usize + c] = Cell::new(b'#', 15, 4);
        }
    }
    // The small-text window over text rows 3..7, columns 60..76: the small plane's rows 0..8 there, in the small
    // font — text, box lines, digits and signs, a blinking cell, colours, a FILL — and a PUT past the plane's last
    // column, whose cells beyond it are not written.
    s.frame(&[host::WINDOW, 3, 60, 4, 16]);
    screen.window = Some((3, 60, 4, 16));
    let put_small = |s: &mut Script, screen: &mut Screen, srow: u8, scol: u8, text: &str, fg: u8, bg: u8, blink: bool| {
        let mut f = vec![host::PUT, tp::SMALL_FIRST_ROW + srow, scol];
        for (i, ch) in text.chars().enumerate() {
            let c = Cell { glyph: tp::glyph_slot(ch), blink, fg, bg };
            f.extend(c.bytes());
            if (scol as usize + i) < tp::SMALL_COLS as usize {
                screen.small[srow as usize * tp::SMALL_COLS as usize + scol as usize + i] = c;
            }
        }
        s.frame(&f);
    };
    put_small(&mut s, &mut screen, 0, 0, "BLOCKS 5/8 small", 15, 1, false);
    put_small(&mut s, &mut screen, 1, 0, "0 INIT r2=0;r4=1", 7, 0, false);
    put_small(&mut s, &mut screen, 2, 0, "┌──────────────┐", 11, 0, false);
    put_small(&mut s, &mut screen, 3, 0, "└──────────────┘", 11, 0, false);
    put_small(&mut s, &mut screen, 4, 0, "0123456789+-*/%=", 14, 4, false);
    put_small(&mut s, &mut screen, 5, 0, "gjpqy ()[] <=>!=", 10, 0, false);
    put_small(&mut s, &mut screen, 6, 3, "LIVE", 0, 15, true);
    s.frame(&[host::FILL, tp::SMALL_FIRST_ROW + 7, 0, 1, 16, 0x2E, b'#']);
    for c in 0..16 {
        screen.small[7 * tp::SMALL_COLS as usize + c] = Cell::new(b'#', 14, 2);
    }
    put_small(&mut s, &mut screen, 1, 30, "XYZW", 12, 0, false);
    s.frame(&[host::CURSOR, 5, 10, 1]);
    s.frame(&[host::POINTER, 7, 20, 1]);
    screen.cursor = Some((5, 10));
    screen.pointer = Some((7, 20));
    s.frame(&[host::RESULT, 0x00, 0x00, 0x05, 0x39]);
    s.step("W 50");
    s.step("F");
    s.step("F");
    s.step("R");
    s.frame(&[host::READ, 1, 2, 21]);
    s.step("W 600");
    s.frame(&[host::READ, tp::SMALL_FIRST_ROW + 1, 28, 6]);
    s.step("W 300");
    for k in [0x1C, 0xF0, 0x1C] {
        s.step(&format!("K {k:02x}"));
    }
    s.frame(&[host::PS2_SEND, 0xF4]);
    s.step("A");
    s.frame(&[host::PING]);
    s.step("W 300");
    s.frame(&[host::ACTIVITY]);
    s.step("W 1500");
    s.frame(&[host::TOTALS]);
    s.step("W 2500");
    let mut engine = vec![host::ENGINE];
    engine.extend((0..tp::ENGINE_FRAME_MAX).map(|i| (i * 37 + 11) as u8));
    s.frame(&engine);
    s.step("T");
    s.step("W 3000");
    (s, screen)
}

/// What the board must have done under [`screen_session`], from its reports, its log and the two frames it drew.
fn assert_screen_session(dir: &Path, frames: &[Vec<u8>], notes: &[String], screen: &Screen) {
    let mut phases = Vec::new();
    for k in 0..2 {
        let flash = notes
            .iter()
            .find_map(|n| n.strip_prefix(&format!("FRAME {k} flash ")))
            .unwrap_or_else(|| panic!("frame {k} was not captured: {notes:?}"))
            == "1";
        phases.push(flash);
        assert_frame(dir, k, &render(&Screen { flash, ..screen.clone() }), &format!("flash {flash}"));
    }
    assert_ne!(phases[0], phases[1], "the two frames show both flash phases");
    assert!(notes.contains(&"RESULT 00000539".to_string()), "the 7-segment value: {notes:?}");
    assert!(notes.contains(&"DEV f4 1".to_string()), "the device took 0xF4 with good parity: {notes:?}");

    let kinds: Vec<u8> = frames.iter().map(|f| f[0]).collect();
    assert_eq!(
        frames[0],
        pong(tp::FEATURE_SMALL, 0, 255, 0, 0),
        "PONG: the shell has no engine but the small plane, and its PS/2 line has carried nothing"
    );
    let later: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == board::PONG).collect();
    assert_eq!(later.len(), 2, "two PINGs, two PONGs: {kinds:x?}");
    let low = later[1][1 + tp::PONG_FIELDS.iter().position(|&f| f == "ps2_shortest_low_us").unwrap()];
    assert!(low == 39 || low == 40, "the device's 40 us clock-low phase, timed: {low}");
    assert_eq!(
        later[1],
        &pong(tp::FEATURE_SMALL, 3 * 11 + 11, low, 3, 0),
        "PONG after three keyboard bytes and a send the device clocked in: 44 clock edges, 3 bytes whole"
    );
    let act: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == board::ACTIVITY).collect();
    assert_eq!(act.len(), 1, "an ACTIVITY request is answered: {kinds:x?}");
    assert_eq!(act[0][1..], vec![0u8; tp::activity_bytes()][..], "the shell has no engine: its ACTIVITY is all zeros");
    let tot: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == board::TOTALS).collect();
    assert_eq!(tot.len(), 1, "a TOTALS request is answered: {kinds:x?}");
    assert_eq!(tot[0][1..], vec![0u8; tp::totals_bytes()][..], "and its TOTALS are all zeros");
    let cells = frames.iter().find(|f| f[0] == board::CELLS).expect("a CELLS report");
    let mut want_cells = vec![board::CELLS, 1, 2];
    for i in 0..21 {
        want_cells.extend(screen.cells[COLS as usize + 2 + i].bytes());
    }
    assert_eq!(cells, &want_cells, "the cells read back are the cells written");
    let small: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == board::CELLS && f[1] == tp::SMALL_FIRST_ROW + 1).collect();
    assert_eq!(small.len(), 1, "the small plane's cells are read back: {kinds:x?}");
    let mut want_small = vec![board::CELLS, tp::SMALL_FIRST_ROW + 1, 28];
    for c in 28..32 {
        want_small.extend(screen.small[tp::SMALL_COLS as usize + c].bytes());
    }
    want_small.extend(Cell::new(b' ', 7, 0).bytes());
    want_small.extend(Cell::new(b' ', 7, 0).bytes());
    assert_eq!(small[0], &want_small, "the small plane reads back as written, and holds nothing past its last column");
    let ps2: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == board::PS2).collect();
    assert_eq!(ps2, vec![&vec![board::PS2, 0x1C, 0], &vec![board::PS2, 0xF0, 0], &vec![board::PS2, 0x1C, 0]], "keys: {kinds:x?}");
    assert!(frames.contains(&vec![board::PS2_SENT, 0]), "the mouse-enable send was acknowledged: {kinds:x?}");
    assert!(
        frames.contains(&vec![board::ENGINE_TAKEN, tp::ENGINE_FRAME_MAX as u8]),
        "a whole ENGINE frame went through the engine queue: {kinds:x?}"
    );
}

#[test]
fn the_screen_draws_what_the_host_wrote_and_the_board_answers_the_link() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (s, screen) = screen_session();
    let (dir, frames, notes) = run_top("screen", "term_shell", false, &s);
    assert_screen_session(&dir, &frames, &notes, &screen);
}

/// The netlist test's host session: what only a synthesized netlist can get wrong — its block RAMs. Every glyph in a
/// colour of its own, written through the text buffer's link port and read back through it; a whole ENGINE frame
/// through the link's engine queue; and one frame drawn through the buffer's screen port and the font ROM. Nothing
/// blinks and there is no cursor or pointer, so the frame is the same in either flash phase.
fn netlist_session() -> (Script, Screen) {
    let mut s = Script::new();
    let mut screen = Screen::blank();
    s.frame(&[host::PING]);
    s.frame(&[host::FILL, 0, 0, ROWS, COLS, 0x17, b' ']);
    for c in screen.cells.iter_mut() {
        *c = Cell::new(b' ', 7, 1);
    }
    let mut put = vec![host::PUT, 3, 0];
    for g in 0..tp::GLYPHS as u8 {
        let c = Cell::new(g, (g % 15) + 1, (g / 16) % 8);
        put.extend(c.bytes());
        screen.cells[3 * COLS as usize + g as usize] = c;
    }
    s.frame(&put);
    s.frame(&[host::READ, 3, 0, tp::GLYPHS as u8]);
    let mut engine = vec![host::ENGINE];
    engine.extend((0..tp::ENGINE_FRAME_MAX).map(|i| (i * 37 + 11) as u8));
    s.frame(&engine);
    s.step("T");
    s.step("F");
    (s, screen)
}

#[test]
#[ignore = "gate-level: synthesizes the screen with yosys and simulates one frame of its netlist (minutes); run in the full suite"]
fn the_synthesized_screen_draws_and_answers_as_its_rtl_does() {
    if !common::require_tool("yosys") || !common::require_tool("iverilog") {
        return;
    }
    let cells = gate_level::cells_sim_without_ramb().expect("yosys is installed, so its xilinx/cells_sim.v is too");
    let dir = scratch("postsynth_screen");
    let mut sources = vec!["sevenseg_dec.v".to_string()];
    std::fs::copy(root().join("benchmarks/silicon/sevenseg_dec.v"), dir.join("sevenseg_dec.v")).unwrap();
    for (name, text) in terminal_rtl::modules(false) {
        std::fs::write(dir.join(name), text).unwrap();
        sources.push(name.to_string());
    }
    write_fonts(&dir);
    std::fs::write(dir.join("top.v"), terminal_rtl::top("term_shell", None)).unwrap();
    sources.push("top.v".into());
    // Hierarchy kept, so the testbench finds the screen's beam and the link's RESULT by name.
    let synth = Command::new("yosys")
        .arg("-q")
        .arg("-p")
        .arg(format!(
            "read_verilog {}; synth_xilinx -family xc7 -nodsp -top term_shell; write_verilog -noattr netlist.v",
            sources.join(" ")
        ))
        .current_dir(&dir)
        .output()
        .expect("yosys");
    assert!(synth.status.success(), "synthesis failed:\n{}", String::from_utf8_lossy(&synth.stderr));
    let netlist = std::fs::read_to_string(dir.join("netlist.v")).unwrap();
    assert!(netlist.contains("RAMB18E1") || netlist.contains("RAMB36E1"), "the text buffer, font and engine queue are block RAM");

    let (s, screen) = netlist_session();
    std::fs::write(dir.join("script.txt"), s.0.join("\n") + "\n").unwrap();
    std::fs::write(dir.join("cells_sim.v"), cells).unwrap();
    std::fs::write(dir.join("ramb_behav.v"), gate_level::RAMB_BEHAV_V).unwrap();
    std::fs::write(dir.join("tb.v"), tp::fill_verilog(&asset("tb_term_top.v").replace("__TOP__", "term_shell"))).unwrap();
    let build = Command::new("iverilog")
        .args(["-g2012", "-DGATE_LEVEL", "-o", "sim.vvp", "cells_sim.v", "ramb_behav.v", "netlist.v", "tb.v"])
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog failed:\n{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").arg("sim.vvp").current_dir(&dir).output().expect("vvp");
    assert!(run.status.success(), "vvp failed:\n{}", String::from_utf8_lossy(&run.stderr));
    let (frames, notes) = board_log(&dir);
    assert!(notes.iter().any(|n| n == "DONE"), "the netlist's testbench did not finish: {notes:?}");

    assert_eq!(frames[0], pong(tp::FEATURE_SMALL, 0, 255, 0, 0), "PONG");
    let mut want_cells = vec![board::CELLS, 3, 0];
    for g in 0..tp::GLYPHS {
        want_cells.extend(screen.cells[3 * COLS as usize + g].bytes());
    }
    let cells = frames.iter().find(|f| f[0] == board::CELLS).expect("a CELLS report");
    assert_eq!(cells, &want_cells, "every glyph read back through the text buffer's link port as written");
    assert!(frames.contains(&vec![board::ENGINE_TAKEN, tp::ENGINE_FRAME_MAX as u8]), "a whole ENGINE frame went through the engine queue");
    let picture = render(&screen);
    assert_eq!(picture, render(&Screen { flash: true, ..screen.clone() }), "the session's frame is the same in either flash phase");
    assert_frame(&dir, 0, &picture, "netlist");
}

/// wpe_loop.lg's `f(n)` lowered for the engine, the bytes its loader takes, and the VM's answer.
fn loop_program(n: i64) -> (Vec<u8>, i64) {
    let src = std::fs::read_to_string(root().join("benchmarks/silicon/programs/wpe_loop.lg")).unwrap();
    let out = vm_outcome(&format!("{src}## Main\n    Show f({n}).\n"));
    let golden: i64 = out.output.trim().parse().expect("the VM shows an Int");
    let cfg = serialize_program(&program_cfg_fabric(&src, "f"));
    assert_eq!(eval_wprog_config(&cfg, &[n]), golden, "the lowering agrees with the VM");
    (wprog_config_image_unchecked(&cfg).wire_bytes(&[n]), golden)
}

/// The script that loads `wire` into the engine: reset, then frames of at most `ENGINE_FRAME_MAX` bytes, each
/// after the board took the last.
fn load(s: &mut Script, wire: &[u8]) {
    for chunk in wire.chunks(tp::ENGINE_FRAME_MAX) {
        let mut f = vec![host::ENGINE];
        f.extend_from_slice(chunk);
        s.frame(&f);
        s.step("T");
    }
}

/// The engine's reply in the board's ENGINE reports: one result frame, as (result, run cycles).
fn engine_reply(frames: &[Vec<u8>]) -> (i64, u32) {
    use logicaffeine_compile::codegen_sva::chip_protocol::{FRAME_END, FRAME_SYNC};
    let reply: Vec<u8> = frames.iter().filter(|f| f[0] == board::ENGINE).flat_map(|f| f[1..].to_vec()).collect();
    assert_eq!(reply.len(), 14, "one result frame from the engine: {reply:02x?}");
    assert_eq!((reply[0], reply[13]), (FRAME_SYNC, FRAME_END), "{reply:02x?}");
    (i64::from_be_bytes(reply[1..9].try_into().unwrap()), u32::from_be_bytes(reply[9..13].try_into().unwrap()))
}

#[test]
fn a_program_runs_on_the_fabric_through_the_terminal_link_even_after_an_abandoned_load() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (wire, golden) = loop_program(5);
    let mut s = Script::new();
    s.frame(&[host::PING]);
    s.frame(&[host::ENGINE_RESET]);
    s.step("W 100");
    // Half an image, then the host gives up on it.
    let half = &wire[..wire.len() / 2];
    load(&mut s, half);
    s.frame(&[host::ENGINE_RESET]);
    s.step("W 100");
    load(&mut s, &wire);
    s.step("W 30000");
    let (_dir, frames, _notes) = run_top("engine", "term_fab", true, &s);

    assert_eq!(
        frames[0],
        pong(tp::FEATURE_ENGINE | tp::FEATURE_SMALL | tp::FEATURE_TOTALS, 0, 255, 0, 0),
        "PONG: the image has the engine, the small plane and the run's totals"
    );
    let taken: Vec<usize> = frames.iter().filter(|f| f[0] == board::ENGINE_TAKEN).map(|f| f[1] as usize).collect();
    let mut want_taken: Vec<usize> = half.chunks(tp::ENGINE_FRAME_MAX).map(<[u8]>::len).collect();
    want_taken.extend(wire.chunks(tp::ENGINE_FRAME_MAX).map(<[u8]>::len));
    assert_eq!(taken, want_taken, "ENGINE_TAKEN counts every frame's bytes");
    let (result, _cycles) = engine_reply(&frames);
    assert_eq!(result, golden, "the fabric's answer through the terminal is the VM's");
}

// ── the activity panel ──────────────────────────────────────────────────────────────────────────────────────

/// Hold frame `k` the testbench captured in `dir` to `want`, pixel for pixel.
fn assert_frame(dir: &Path, k: usize, want: &[u16], what: &str) {
    let got: Vec<u16> = std::fs::read_to_string(dir.join(format!("frame{k}.hex")))
        .unwrap_or_else(|_| panic!("{what}: frame {k} was not captured"))
        .lines()
        .map(|l| u16::from_str_radix(l, 16).unwrap())
        .collect();
    assert_eq!(got.len(), want.len(), "{what}: frame {k} is not 640 x 480");
    let wrong: Vec<usize> = (0..got.len()).filter(|&i| got[i] != want[i]).collect();
    if let Some(&i) = wrong.first() {
        panic!(
            "{what}: frame {k} differs at {} pixels, first ({}, {}): drew {:03x}, the model draws {:03x}",
            wrong.len(),
            i % 640,
            i / 640,
            got[i],
            want[i]
        );
    }
}

/// The counts a panel shows after `runs` — running samples as (block, cell, count) — and the snapshot's own sample.
fn counted(runs: &[(u16, u16, u32)], snapshot: &Panel) -> Panel {
    let mut p = Panel { block_hits: vec![0; WPE_HW_MAXBLK], cell_hits: vec![0; WPE_HW_MAXCELL], ..snapshot.clone() };
    let samples = runs.iter().copied().chain(snapshot.running.then_some((snapshot.pc, snapshot.cell, 1)));
    for (b, c, n) in samples {
        if let Some(h) = p.block_hits.get_mut(b as usize) {
            *h += n;
        }
        if let Some(h) = p.cell_hits.get_mut(c as usize) {
            *h += n;
        }
    }
    p
}

#[test]
fn the_run_totals_count_every_running_sample_hold_at_their_largest_and_clear_on_reset() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (nb, nc) = (WPE_HW_MAXBLK, WPE_HW_MAXCELL);
    let largest = u32::MAX as u64;
    let plan: Vec<String> = [
        format!("G {}", tp::ENGINE_CLOCK_DIVIDER),
        "R".into(),
        "S 2 5 30".into(),
        "S 3 7 12".into(),
        "I 2 5 9".into(),
        "S 2 6 1".into(),
        format!("S {} {} 4", nb + 1, nc + 3),
        "S 7 31 3".into(),
        format!("P 3 {}", largest - 2),
        "S 3 0 5".into(),
        format!("P {} {}", nb + 31, largest),
        "S 6 31 2".into(),
    ]
    .into_iter()
    .chain((0..nb + nc).map(|i| format!("T {i}")))
    .chain(["R".to_string(), "S 4 9 3".into()])
    .chain((0..nb + nc).map(|i| format!("T {i}")))
    .collect();

    // The totals as the plan makes them: each running sample one more on its block's and its cell's (neither when
    // out of range), each held at its largest; set by P, cleared by R.
    let mut want: Vec<Vec<u64>> = Vec::new();
    let mut totals = vec![0u64; nb + nc];
    for step in &plan {
        let w: Vec<&str> = step.split(' ').collect();
        let num = |k: usize| w[k].parse::<u64>().unwrap();
        match w[0] {
            "R" => totals.iter_mut().for_each(|t| *t = 0),
            "S" => {
                for (i, ok) in [(num(1) as usize, (num(1) as usize) < nb), (nb + num(2) as usize, (num(2) as usize) < nc)] {
                    if ok {
                        totals[i] = (totals[i] + num(3)).min(largest);
                    }
                }
            }
            "P" => totals[num(1) as usize] = num(2),
            "T" if num(1) == 0 => want.push(totals.clone()),
            _ => {}
        }
    }

    let dir = scratch("totals");
    std::fs::write(dir.join("plan.txt"), plan.join("\n") + "\n").unwrap();
    simulate(&dir, None, true, &[], &[], &tp::fill_verilog(&asset("tb_term_activity.v")));
    let log = std::fs::read_to_string(dir.join("board.txt")).expect("board.txt");
    assert!(log.lines().any(|l| l == "DONE"), "the totals testbench did not finish:\n{log}");
    let read: Vec<(usize, u64)> = log
        .lines()
        .filter_map(|l| l.strip_prefix("TOTAL "))
        .map(|l| {
            let (i, v) = l.split_once(' ').unwrap();
            (i.parse().unwrap(), v.parse().unwrap())
        })
        .collect();
    assert_eq!(read.len(), 2 * (nb + nc), "every total read twice:\n{log}");
    for (k, chunk) in read.chunks(nb + nc).enumerate() {
        let got: Vec<u64> = chunk.iter().map(|&(_, v)| v).collect();
        assert_eq!(got, want[k], "read {k}: every total, blocks then cells");
    }
    assert_eq!(want[0][2], 31, "block 2: its running samples, not its idle ones");
    assert_eq!(want[0][3], largest, "a total that reaches its largest holds there");
    assert_eq!(want[0][nb + 31], largest, "and one set at its largest stays there");
    assert_eq!(want[1].iter().sum::<u64>(), 6, "after a reset, only the samples since");
}

#[test]
fn the_activity_panel_draws_every_count_gauge_and_digit_as_the_model_does() {
    if !common::require_tool("iverilog") {
        return;
    }
    let snap = |running, pc, cell, sp, hp, cycles| Panel { running, pc, cell, sp, hp, cycles, ..Panel::idle() };
    // (running samples by block and cell, idle samples, whether the engine is reset before the snapshot, the probes
    // at the snapshot)
    let cases: Vec<(Vec<(u16, u16, u32)>, u32, bool, Panel)> = vec![
        // Every heat level from 0 to 13 across the tiles and squares — a count of 2^(k-1) is level k — the engine in
        // tile 3 and square 17, part-full gauges.
        (
            vec![
                (1, 0, 1),
                (2, 1, 2),
                (3, 2, 4),
                (4, 5, 8),
                (5, 9, 16),
                (6, 17, 32),
                (7, 31, 64),
                (0, 3, 128),
                (0, 4, 256),
                (1, 6, 512),
                (2, 7, 1024),
                (3, 8, 2048),
                (4, 10, 4096),
            ],
            0,
            false,
            snap(true, 3, 17, 5, 37, 1_234_567_890),
        ),
        // Samples outside the panel, and idle samples at a block and cell it has, count for nothing; full gauges; no
        // outline when not running; a zero count shows its one digit.
        (vec![(WPE_HW_MAXBLK as u16 + 1, WPE_HW_MAXCELL as u16 + 8, 50)], 500, false, snap(false, 2, 4, WPE_HW_DEPTH as u16, WPE_HW_HEAP as u16, 0)),
        // The hottest level, the last tile and square outlined, every digit of the largest count.
        (vec![(7, 31, 9000), (0, 0, 4000)], 0, false, snap(true, 7, 31, 0, 1, u32::MAX)),
        // A frame's worth of samples on one tile; zeros inside a number are drawn.
        (vec![(0, 30, 13_000)], 0, false, snap(true, 0, 0, 1, WPE_HW_HEAP as u16 - 1, 1_000_000_000)),
        // A run, then the engine reset: the counts still show, and the probes, left as the run left them, do not.
        (vec![(4, 6, 100)], 0, true, snap(false, 5, 9, 3, 20, 777)),
    ];
    let mut plan = Vec::new();
    for (runs, idle, reset, at) in &cases {
        plan.push("E".to_string());
        plan.extend(runs.iter().map(|(b, c, n)| format!("S {b} {c} {n}")));
        if *idle > 0 {
            plan.push(format!("I {} {} {idle}", at.pc, at.cell));
        }
        if *reset {
            plan.push("R".to_string());
        }
        plan.push(format!("C {} {} {} {} {} {}", at.running as u8, at.pc, at.cell, at.sp, at.hp, at.cycles));
    }
    let dir = scratch("panel");
    std::fs::write(dir.join("plan.txt"), plan.join("\n") + "\n").unwrap();
    simulate(&dir, None, true, &[], &[], &tp::fill_verilog(&asset("tb_term_activity.v")));
    let log = std::fs::read_to_string(dir.join("board.txt")).expect("board.txt");
    assert!(log.lines().any(|l| l == "DONE"), "the panel testbench did not finish:\n{log}");

    let mut levels_seen = std::collections::BTreeSet::new();
    for (k, (runs, _idle, reset, at)) in cases.iter().enumerate() {
        let mut panel = counted(runs, at);
        if *reset && !at.running {
            panel = Panel { block_hits: panel.block_hits, cell_hits: panel.cell_hits, ..Panel::idle() };
        }
        let samples: u32 = runs.iter().map(|r| r.2).sum::<u32>() + at.running as u32;
        assert!(samples <= tp::panel_frame_samples(), "case {k} counts more than a frame can");
        levels_seen.extend(panel.block_hits.iter().chain(&panel.cell_hits).map(|&h| tp::heat_level(h)));
        assert_frame(&dir, k, &render(&Screen { panel: Some(panel), ..Screen::blank() }), &format!("case {k}"));
    }
    assert_eq!(levels_seen.into_iter().collect::<Vec<_>>(), (0..=14).collect::<Vec<_>>(), "the cases paint every heat level");
}

/// What `tb_term_top.v` logs of the activity panel under `TERM_ENGINE`.
struct PanelLog {
    /// Each snapshot by number: what the panel draws in the frame it starts.
    panels: Vec<Panel>,
    /// Each snapshot's levels as the board holds them: blocks, cells.
    levels: Vec<(Vec<usize>, Vec<usize>)>,
    /// Each snapshot's ten digits as the board holds them.
    digits: Vec<String>,
    /// The engine-clock count: by block, by cell, and outside the panel.
    reference: (Vec<u32>, Vec<u32>, u32),
}

fn panel_log(notes: &[String]) -> PanelLog {
    let mut log = PanelLog { panels: Vec::new(), levels: Vec::new(), digits: Vec::new(), reference: (Vec::new(), Vec::new(), u32::MAX) };
    for note in notes {
        let (tag, rest) = note.split_once(' ').unwrap_or((note.as_str(), ""));
        let nums = |rest: &str| -> Vec<u32> {
            rest.split_whitespace()
                .filter(|w| *w != "|")
                .map(|w| w.parse().unwrap_or_else(|_| panic!("the panel holds an undefined value: {note}")))
                .collect()
        };
        match tag {
            "SNAP" => {
                let v = nums(rest);
                assert_eq!(v[0] as usize, log.panels.len(), "snapshots in order");
                log.panels.push(Panel {
                    running: v[1] == 1,
                    pc: v[2] as u16,
                    cell: v[3] as u16,
                    sp: v[4] as u16,
                    hp: v[5] as u16,
                    cycles: v[6],
                    ..Panel::idle()
                });
            }
            "BLOCKS" => log.panels.last_mut().expect("BLOCKS after SNAP").block_hits = nums(rest)[1..].to_vec(),
            "CELLS" => log.panels.last_mut().expect("CELLS after SNAP").cell_hits = nums(rest)[1..].to_vec(),
            "LEVELS" => {
                let v: Vec<usize> = nums(rest)[1..].iter().map(|&l| l as usize).collect();
                log.levels.push((v[..WPE_HW_MAXBLK].to_vec(), v[WPE_HW_MAXBLK..].to_vec()));
            }
            "DIGITS" => log.digits.push(rest.split_whitespace().nth(1).expect("DIGITS n hh").to_string()),
            "REF_BLOCKS" => log.reference.0 = nums(rest),
            "REF_CELLS" => log.reference.1 = nums(rest),
            "REF_OUTSIDE" => log.reference.2 = nums(rest)[0],
            _ => {}
        }
    }
    log
}

/// The snapshot drawn in captured frame `k`: the first one the panel took after the frame began.
fn drawn_in(notes: &[String], k: usize) -> usize {
    let start = notes.iter().position(|n| n.starts_with(&format!("FRAME {k} "))).unwrap_or_else(|| panic!("frame {k} was not captured"));
    let snaps_before = notes[..start].iter().filter(|n| n.starts_with("SNAP ")).count();
    assert!(notes[start..].iter().any(|n| n.starts_with(&format!("SNAP {snaps_before} "))), "frame {k} has a snapshot");
    snaps_before
}

#[test]
fn the_activity_panel_counts_every_engine_clock_a_program_runs_once_and_draws_it() {
    if !common::require_tool("iverilog") {
        return;
    }
    // Long enough to run across two frame boundaries at the engine's clock.
    let (wire, golden) = loop_program(3000);
    let mut s = Script::new();
    s.frame(&[host::ENGINE_RESET]);
    s.step("W 100");
    load(&mut s, &wire);
    s.step("E");
    s.step("F");
    s.frame(&[host::ACTIVITY]);
    s.step("W 2000");
    s.step("W 70000");
    s.step("F");
    s.frame(&[host::ACTIVITY]);
    s.step("W 2000");
    s.frame(&[host::TOTALS]);
    s.step("W 3000");
    // Esc on the host: the engine is reset, and the panel lets go of the run it was showing.
    s.frame(&[host::ENGINE_RESET]);
    s.step("W 100");
    s.step("F");
    s.frame(&[host::ACTIVITY]);
    s.step("W 2000");
    s.frame(&[host::TOTALS]);
    s.step("W 3000");
    let (dir, frames, notes) = run_top("panel_engine", "term_fab", true, &s);
    let (result, cycles) = engine_reply(&frames);
    assert_eq!(result, golden, "the probed engine answers what the VM answers");

    let log = panel_log(&notes);
    let first_run = log.panels.iter().position(|p| p.running).expect("a snapshot while the program ran");
    for (n, p) in log.panels[..first_run].iter().enumerate() {
        assert_eq!(p, &Panel::idle(), "snapshot {n}, before any program ran, is the idle panel");
    }
    let (ref_blocks, ref_cells, outside) = &log.reference;
    assert_eq!(*outside, 0, "the engine never ran outside the panel's blocks and cells");
    let sum = |pick: fn(&Panel) -> &Vec<u32>, n: usize| -> Vec<u32> {
        (0..n).map(|i| log.panels.iter().map(|p| pick(p)[i]).sum()).collect()
    };
    assert_eq!(&sum(|p| &p.block_hits, WPE_HW_MAXBLK), ref_blocks, "the panel's frames add up to every engine clock, by block");
    assert_eq!(&sum(|p| &p.cell_hits, WPE_HW_MAXCELL), ref_cells, "the panel's frames add up to every engine clock, by cell");
    // The clock that sees the program done and hands over to sending the result runs but is not a run cycle.
    assert_eq!(ref_blocks.iter().sum::<u32>(), cycles + 1, "every running clock was counted");
    let busy = log.panels.iter().filter(|p| p.block_hits.iter().any(|&h| h > 0)).count();
    assert!(busy >= 3, "the run spans frames: {busy} frames counted clocks");

    assert_eq!(log.levels.len(), log.panels.len(), "every snapshot reached the screen");
    for (n, (panel, (blocks, cells))) in log.panels.iter().zip(&log.levels).enumerate() {
        let want: Vec<usize> = panel.block_hits.iter().map(|&h| tp::heat_level(h)).collect();
        assert_eq!(blocks, &want, "snapshot {n}: block levels are the counts' bit lengths");
        let want: Vec<usize> = panel.cell_hits.iter().map(|&h| tp::heat_level(h)).collect();
        assert_eq!(cells, &want, "snapshot {n}: cell levels are the counts' bit lengths");
        assert_eq!(log.digits[n], format!("{:010}", panel.cycles), "snapshot {n}: the digits are the cycle count");
    }

    let during = drawn_in(&notes, 0);
    assert!(log.panels[during].running, "frame 0 was drawn while the program ran");
    assert_frame(&dir, 0, &render(&Screen { panel: Some(log.panels[during].clone()), ..Screen::blank() }), "mid-run");
    let after = drawn_in(&notes, 1);
    let last = &log.panels[after];
    assert!(!last.running && last.cycles == cycles, "after the run the panel shows the engine idle at its final count");
    assert_frame(&dir, 1, &render(&Screen { panel: Some(last.clone()), ..Screen::blank() }), "after the run");
    let reset = drawn_in(&notes, 2);
    assert_eq!(log.panels[reset], Panel::idle(), "after the engine is reset the panel shows no run");
    assert_frame(&dir, 2, &render(&Screen { panel: Some(Panel::idle()), ..Screen::blank() }), "after the reset");

    // ACTIVITY: the board's own report of a frame is the snapshot the panel took and drew for it, number for number.
    let replies: Vec<Activity> = frames.iter().filter(|f| f[0] == board::ACTIVITY).map(|f| activity(&f[1..])).collect();
    assert_eq!(replies.len(), 3, "three ACTIVITY requests, three replies");
    for (n, a) in replies.iter().enumerate() {
        assert_eq!(a.frame, a.frame_again, "reply {n} reads one frame's counts");
        assert!(a.frame >= 1 && (a.frame as usize) <= log.panels.len(), "reply {n} names a frame the panel drew: {}", a.frame);
        let p = &log.panels[a.frame as usize - 1];
        assert_eq!(
            (a.running, a.block, a.cell, a.depth, a.heap, a.cycles),
            (p.running, p.pc as u32, p.cell as u32, p.sp as u32, p.hp as u32, p.cycles),
            "reply {n}: the snapshot of frame {}",
            a.frame
        );
        assert_eq!(a.block_clocks, p.block_hits, "reply {n}: each block's engine clocks in frame {}", a.frame);
        assert_eq!(a.cell_clocks, p.cell_hits, "reply {n}: each cell's engine clocks in frame {}", a.frame);
    }
    assert!(replies[0].running && replies[0].block_clocks.iter().sum::<u32>() > 0, "mid-run, the reply shows the run");
    assert!(!replies[1].running && replies[1].cycles == cycles, "after the run, the reply holds its final count");
    assert_eq!(replies[2].block_clocks.iter().sum::<u32>(), 0, "after the reset, no clock is counted");

    // TOTALS: after the run, every engine clock it ran, by block and by cell — the reference's own counts; after the
    // reset, none.
    let totals: Vec<(u32, Vec<u32>, Vec<u32>)> = frames.iter().filter(|f| f[0] == board::TOTALS).map(|f| totals(&f[1..])).collect();
    assert_eq!(totals.len(), 2, "two TOTALS requests, two replies");
    assert_eq!(totals[0].0, cycles, "after the run, TOTALS opens with its cycles");
    assert_eq!(&totals[0].1, ref_blocks, "after the run, each block's total is every engine clock it ran there");
    assert_eq!(&totals[0].2, ref_cells, "and each cell's");
    assert_eq!(totals[0].1.iter().sum::<u32>(), cycles + 1, "the block totals add up to the run's cycles and one");
    assert_eq!(totals[0].2.iter().sum::<u32>(), cycles + 1, "and so do the cell totals");
    assert_eq!(totals[1].0, 0, "after the reset, no run's cycles");
    assert!(totals[1].1.iter().chain(&totals[1].2).all(|&t| t == 0), "and every total is zero");
}

/// A [`board::TOTALS`] payload, read by [`tp::TOTALS_FIELDS`]: (cycles, block totals, cell totals).
fn totals(payload: &[u8]) -> (u32, Vec<u32>, Vec<u32>) {
    assert_eq!(payload.len(), tp::totals_bytes(), "a TOTALS payload is {} bytes", tp::totals_bytes());
    let mut at = 0;
    let mut fields = std::collections::HashMap::new();
    for (name, bytes, count) in tp::TOTALS_FIELDS {
        let values: Vec<u32> = (0..count)
            .map(|_| {
                let v = payload[at..at + bytes].iter().fold(0u32, |acc, &b| acc << 8 | b as u32);
                at += bytes;
                v
            })
            .collect();
        fields.insert(name, values);
    }
    (fields["cycles"][0], fields["block_totals"].clone(), fields["cell_totals"].clone())
}

/// A [`board::ACTIVITY`] payload, read by [`tp::ACTIVITY_FIELDS`].
#[derive(Debug)]
struct Activity {
    frame: u32,
    running: bool,
    block: u32,
    cell: u32,
    depth: u32,
    heap: u32,
    cycles: u32,
    block_clocks: Vec<u32>,
    cell_clocks: Vec<u32>,
    frame_again: u32,
}

fn activity(payload: &[u8]) -> Activity {
    assert_eq!(payload.len(), tp::activity_bytes(), "an ACTIVITY payload is {} bytes", tp::activity_bytes());
    let mut at = 0;
    let mut fields = std::collections::HashMap::new();
    for (name, bytes, count) in tp::ACTIVITY_FIELDS {
        let values: Vec<u32> = (0..count)
            .map(|_| {
                let v = payload[at..at + bytes].iter().fold(0u32, |acc, &b| acc << 8 | b as u32);
                at += bytes;
                v
            })
            .collect();
        fields.insert(name, values);
    }
    let one = |name: &str| fields[name][0];
    Activity {
        frame: one("frame"),
        running: one("running") != 0,
        block: one("block"),
        cell: one("cell"),
        depth: one("depth"),
        heap: one("heap"),
        cycles: one("cycles"),
        block_clocks: fields["block_clocks"].clone(),
        cell_clocks: fields["cell_clocks"].clone(),
        frame_again: one("frame_again"),
    }
}

// ── the library ─────────────────────────────────────────────────────────────────────────────────────────────

/// The terminal's library as `benchmarks/silicon/programs/terminal_library.txt` lists it: (title, program,
/// function — `None` for a `## Main` program — and arguments).
fn terminal_library() -> Vec<(String, PathBuf, Option<String>, Vec<i64>)> {
    let path = root().join("benchmarks/silicon/programs/terminal_library.txt");
    std::fs::read_to_string(&path)
        .expect("terminal_library.txt")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('|').map(str::trim).collect();
            assert_eq!(f.len(), 4, "a library line is title | program | function | arguments: {l:?}");
            let args = f[3].split(',').filter(|a| !a.trim().is_empty()).map(|a| a.trim().parse().expect("an Int argument")).collect();
            (f[0].to_string(), path.parent().unwrap().join(f[1]), (!f[2].is_empty()).then(|| f[2].to_string()), args)
        })
        .collect()
}

#[test]
fn every_library_program_fits_board_b_and_answers_what_the_vm_answers() {
    use logicaffeine_compile::codegen_sva::engine_build::{fits_caps, lower, lower_main, Profile};
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
        WPE_HW_MAXARG, WPE_HW_MAXK, WPE_HW_MAXLIV, WPE_HW_MAXWR, WPE_HW_NREG,
    };
    let board_b = format!(
        "NREG={WPE_HW_NREG},MAXBLK={WPE_HW_MAXBLK},MAXCELL={WPE_HW_MAXCELL},MAXV={WPE_HW_MAXCELL},MAXWR={WPE_HW_MAXWR},\
         MAXARG={WPE_HW_MAXARG},MAXLIV={WPE_HW_MAXLIV},MAXK={WPE_HW_MAXK},HEAP={WPE_HW_HEAP},DEPTH={WPE_HW_DEPTH}"
    );
    let library = terminal_library();
    assert!(library.len() >= 2, "the library offers programs");
    let mut titles = std::collections::HashSet::new();
    for (title, path, func, args) in &library {
        assert!(titles.insert(title.clone()), "two library entries are titled {title}");
        let src = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{title}: {} is missing", path.display()));
        let cfg = match func {
            None => {
                assert!(args.is_empty(), "{title}: a `## Main` program takes no arguments");
                lower_main(&src).unwrap_or_else(|e| panic!("{title}: {e}")).0
            }
            Some(f) => lower(&src, f, args).unwrap_or_else(|e| panic!("{title}: {e}")).0,
        };
        Profile::Hw.fits(&cfg).unwrap_or_else(|e| panic!("{title} does not fit Board B's engine: {e}"));
        fits_caps(&cfg, args, &board_b).unwrap_or_else(|e| panic!("{title} does not fit Board B's engine: {e}"));
    }
}

// ── the host ────────────────────────────────────────────────────────────────────────────────────────────────

/// Run `python3 <repo>/<script> <args>` in `dir` with `env`, the terminal table it reads among them. Returns
/// whether it succeeded, and its stdout and stderr.
fn host_python(dir: &Path, script: &str, args: &[&str], env: &[(&str, String)]) -> (bool, String) {
    let table = dir.join("terminal.json");
    std::fs::write(&table, tp::table_json()).unwrap();
    let out = Command::new("python3")
        .arg(root().join(script))
        .args(args)
        .current_dir(dir)
        .env("LOGOS_TERMINAL_TABLE", &table)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .output()
        .expect("python3");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_hosts_link_and_terminal_pass_their_selftests() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = scratch("host_selftest");
    for (script, arg, done) in [
        ("scripts/lib/terminal_link.py", "selftest", "terminal_link selftest: OK"),
        ("scripts/logos-terminal.py", "--selftest", "logos-terminal selftest: OK"),
    ] {
        let (ok, out) = host_python(&dir, script, &[arg], &[]);
        assert!(ok && out.contains(done), "{script} {arg}:\n{out}");
        assert!(!out.contains("BAD"), "{script} {arg}:\n{out}");
    }
}

#[test]
fn the_host_runs_the_terminal_end_to_end_against_a_model_board() {
    if !common::require_tool("python3") {
        return;
    }
    use logicaffeine_compile::codegen_sva::chip_protocol::{self as cp, ChipValue, Record, ResultFrame};
    let dir = scratch("host_e2e");
    let reply = |shows: Vec<ChipValue>, result: i64, cycles: u64| -> String {
        let mut bytes: Vec<u8> = shows.into_iter().flat_map(|v| Record::Show(v).encode()).collect();
        bytes.extend(ResultFrame { result, cycles: cycles as u32, end: cp::FRAME_END }.encode());
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    };
    let wire = |name: &str, bytes: &[u8]| -> String {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    };

    use logicaffeine_compile::codegen_sva::engine_build::{fabric_map, main_register_names};
    fn map_of(
        cfg: &logicaffeine_compile::codegen_sva::wprog_engine_rtl::WProgConfig,
        names: &std::collections::HashMap<u16, String>,
    ) -> serde_json::Value {
        serde_json::to_value(fabric_map(cfg, names)).expect("the fabric map is JSON")
    }
    let loop_src = root().join("benchmarks/silicon/programs/wpe_loop.lg");
    let (loop_wire, loop_golden) = loop_program(5);
    let loop_map = map_of(
        &serialize_program(&program_cfg_fabric(&std::fs::read_to_string(&loop_src).unwrap(), "f")),
        &Default::default(),
    );
    let loop_run = eval_wprog_config_run(&serialize_program(&program_cfg_fabric(&std::fs::read_to_string(&loop_src).unwrap(), "f")), &[5], &[]);

    let show_src = "## Main\n    Show 6 * 7.\n    Show \"fabric\".\n";
    let show_path = dir.join("shows.lg");
    std::fs::write(&show_path, show_src).unwrap();
    let show_cfg = program_main_fabric(show_src);
    let show_golden: Vec<String> = vm_outcome(show_src).output.lines().map(str::to_string).collect();
    assert_eq!(program_output(&show_cfg, &[]), show_golden, "the engine model shows what the VM shows");
    let show_run = eval_wprog_config_run(&show_cfg, &[], &[]);

    // A chip that disagrees: it shows 41 where the VM shows 42.
    let wrong_src = "## Main\n    Show 6 * 7.\n";
    let wrong_path = dir.join("wrong.lg");
    std::fs::write(&wrong_path, wrong_src).unwrap();
    let wrong_cfg = program_main_fabric(wrong_src);
    let wrong_golden: Vec<String> = vm_outcome(wrong_src).output.lines().map(str::to_string).collect();
    assert_eq!(wrong_golden, ["42"]);
    let wrong_run = eval_wprog_config_run(&wrong_cfg, &[], &[]);

    // Programs of their own, so each has its own image: one the link loses a byte of, one that never ends.
    let lossy_path = dir.join("lossy.lg");
    std::fs::copy(&loop_src, &lossy_path).unwrap();
    let (lossy_wire, lossy_golden) = loop_program(9);
    let forever_path = dir.join("forever.lg");
    std::fs::copy(&loop_src, &forever_path).unwrap();
    let (forever_wire, forever_golden) = loop_program(7);

    // A program of more blocks than the side list has rows for each to say all it does: the library's own.
    let hello_src_path = root().join("benchmarks/silicon/programs/hello_fabric.lg");
    let hello_src = std::fs::read_to_string(&hello_src_path).unwrap();
    let hello_cfg = logicaffeine_compile::codegen_sva::engine_build::lower_main(&hello_src).expect("hello fabric lowers").0;
    let hello_golden: Vec<String> = vm_outcome(&hello_src).output.lines().map(str::to_string).collect();
    assert_eq!(hello_golden, ["hi fabric", "385", "true"], "hello fabric shows what its reply below carries");
    let hello_run = eval_wprog_config_run(&hello_cfg, &[], &[]);
    let hello_map = map_of(&hello_cfg, &main_register_names(&hello_src));
    assert!(hello_map["blocks"].as_array().unwrap().len() >= 7, "hello fabric fills the side list: {hello_map}");

    let cases = serde_json::json!([
        {
            "title": "loop sum", "source": loop_src, "fn": "f", "args": "5",
            "wire": wire("loop.wire", &loop_wire), "golden": loop_golden.to_string(),
            "reply": reply(vec![], loop_run.result, loop_run.steps),
            "reply_one_cycle_more": reply(vec![], loop_run.result, loop_run.steps + 1),
            "map": loop_map.clone(),
        },
        {
            "title": "shows", "source": show_path, "fn": null, "args": "",
            "wire": wire("shows.wire", &wprog_config_image_unchecked(&show_cfg).wire_bytes(&[])), "golden": show_golden,
            "reply": reply(vec![ChipValue::Int(42), ChipValue::Text("fabric".into())], show_run.result, show_run.steps),
            "map": map_of(&show_cfg, &main_register_names(show_src)),
        },
        {
            "title": "wrong", "source": wrong_path, "fn": null, "args": "",
            "wire": wire("wrong.wire", &wprog_config_image_unchecked(&wrong_cfg).wire_bytes(&[])), "golden": wrong_golden,
            "reply": reply(vec![ChipValue::Int(41)], wrong_run.result, wrong_run.steps),
            "map": map_of(&wrong_cfg, &main_register_names(wrong_src)),
        },
        {
            "title": "lossy", "source": lossy_path, "fn": "f", "args": "9",
            "wire": wire("lossy.wire", &lossy_wire), "golden": lossy_golden.to_string(),
            "reply": reply(vec![], lossy_golden, 0),
            "map": loop_map.clone(),
        },
        {
            "title": "forever", "source": forever_path, "fn": "f", "args": "7",
            "wire": wire("forever.wire", &forever_wire), "golden": forever_golden.to_string(), "reply": null,
            "map": loop_map.clone(),
        },
        {
            "title": "hello fabric", "source": hello_src_path, "fn": null, "args": "",
            "wire": wire("hello.wire", &wprog_config_image_unchecked(&hello_cfg).wire_bytes(&[])), "golden": hello_golden,
            "reply": reply(
                vec![ChipValue::Text("hi fabric".into()), ChipValue::Int(385), ChipValue::Bool(true)],
                hello_run.result,
                hello_run.steps,
            ),
            "map": hello_map,
        },
    ]);
    let cases_path = dir.join("cases.json");
    std::fs::write(&cases_path, cases.to_string()).unwrap();
    let chip_table = dir.join("chip.json");
    std::fs::write(&chip_table, cp::table_json()).unwrap();

    let (ok, out) = host_python(
        &dir,
        "crates/logicaffeine_tests/tests/assets/terminal/host_e2e.py",
        &[],
        &[("E2E_CASES", cases_path.display().to_string()), ("E2E_CHIP_TABLE", chip_table.display().to_string())],
    );
    assert!(ok && out.contains("host_e2e: OK"), "the host against the board model:\n{out}");
    assert_eq!(out.matches("PASS:").count(), 179, "every host check reports:\n{out}");
}
