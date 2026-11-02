//! d5005_flow_lock — the D5005 accelerator's files, the flow that builds them, and the gates every image passes.
//!
//! `spatial::decode_engine::ofs::afu_sources` writes the accelerator an OFS PR build of the D5005 FIM takes. This lock
//! holds:
//!   • the shell end to end under iverilog: four memory channels on four unrelated clocks, the host channel and the
//!     core on two more, every register, every command and every path between the clocks read back;
//!   • each dual-clock FIFO in order under six clock ratios and traffic mixes, and a channel's stream engine reserving
//!     room for every line it reads;
//!   • every clock crossing of the shell a FIFO, each FIFO crossing only through its Gray pointers and its RAM
//!     (`scripts/d5005/cdc_check.py`), and the gate refusing each defect it exists for;
//!   • the PIM boundary elaborating against the D5005 FIM's own PIM (`scripts/d5005/sv_check.py`), and refusing a port
//!     the PIM does not have and a line width the shell does not take;
//!   • the AFU JSON in the OPAE schema's shape with the UUID the feature header carries and user clocks the PIM takes
//!     (both auto, or a fixed clock and its half), sources.txt the Quartus
//!     assignments OPAE's rtl_src_config makes of every file, the C header the register map, the timing constraints
//!     holding every FIFO crossing as the FIM holds its own;
//!   • the image recorder refusing an image whose fit, timing or gates failed, or one already recorded, a persona's and
//!     a flat image's (the FIM built with the accelerator inside it) alike;
//!   • the bring-up receipt, the program the card runs on its first day, passing unchanged against the RTL through the
//!     VPI backend (`scripts/d5005/run-cosim.sh`), the same on every run, passing under Verilator through the DPI-C
//!     backend, passing on DDR4-timed channels whose every
//!     line is scheduled as the controller model of `codegen_sva::ddr4_jedec` schedules it, failing when the RTL is
//!     wrong, compiling against OPAE 2.12 for the card, and its recorder refusing a receipt that did not pass or ran on
//!     an unrecorded image;
//!   • the image loader against the RTL: a weight image written by the image writer copied onto every channel at the
//!     channel's full line width, read back equal to its files with every tile's check word holding, a line damaged on
//!     the card found in its tile and no other and the tile loaded again, and an image of another line width
//!     refused;
//!   • SECDED lines: the code (`secded_line`, `secded_read`) each word's `scratchpad_rtl` codeword, correcting every
//!     single flip of a line's 576 bus bits and finding every two flips in one word; every codeword on 72 different
//!     DQ pins under either order the controller may give its bus word; the emitted coder and decoder equal to the
//!     code on every vector; a channel reading back every line its injected flips make without a line arriving at a
//!     full FIFO; the receipt proving the code on every channel against the RTL, failing when the RTL is wrong, and
//!     its recorder refusing a code that did not hold;
//!   • the core's progress records (a running command's tag, RUNNING, the count of lines and sequencer steps, its
//!     cycles, every 2^HB cycles in which it moved) and a host's wait bounded by them (`sw/logos_core.h` against a
//!     scripted device: waited for while it moves, given up once it stops, another command's records or none no
//!     longer than a budget, a record landing as the wait looks the command's);
//!   • the physical backend (`sw/logos_hal_pci.c`) over device trees and page maps laid out in files: the accelerator
//!     opened by its AFU ID on the one driverless VF of the FIM's PF, its command word given back, the functions it
//!     must not drive refused, and a buffer placed only on frames the page map gives present, nonzero and in order.
//!
//! The testbenches and the host-memory model are `fixtures/d5005_flow/`; the memory channels' model is the emitted one.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test d5005_flow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/ddr4_log.rs"]
mod ddr4_log;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use logicaffeine_compile::codegen_sva::ddr4_jedec::Ddr4Grade;
use logicaffeine_compile::codegen_sva::scratchpad_rtl::encode;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode_engine::image::{weight_image, ImageLayout, TensorTable};
use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
use logicaffeine_compile::spatial::decode_engine::ofs::{
    afu_sources, secded_check_byte, secded_line, secded_read, AfuConfig, ChannelModel, LocalMemoryLine, Op, SecdedRead, BANKS,
    CLOCK_DOMAINS_FILE, ERROR_COMMAND_DROPPED, FIM_STATIC_EXERCISERS, REGISTERS, SECDED_CHECKS, SECDED_WORDS, STATUS_OK, STATUS_RUNNING, STATUS_UNKNOWN_OP,
};
use tiny_models::Tiny;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_flow").join(name)
}

fn script(name: &str) -> PathBuf {
    root().join("scripts/d5005").join(name)
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// The accelerator's files under a fresh directory.
fn emit(cfg: &AfuConfig) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for f in afu_sources(cfg) {
        let p = d.path().join(f.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.text).unwrap();
    }
    d
}

fn rtl(d: &Path, file: &str) -> PathBuf {
    d.join("hw/rtl").join(file)
}

/// The shell's Verilog, in the order a simulation reads it: a divided build's with the compute clock its compute side
/// steps at.
fn shell_files(d: &Path) -> Vec<PathBuf> {
    let clock = rtl(d, "logos_compute_clock.v");
    ["logos_dcfifo.v", "logos_secded.v", "logos_stream.v", "logos_csr.v", "logos_bringup_core.v", "logos_d5005_shell.v"]
        .iter()
        .map(|f| rtl(d, f))
        .chain(clock.is_file().then_some(clock))
        .collect()
}

/// The engine for Qwen3.5-0.8B at the fastest point, one sequence, from its header ledger.
fn engine_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::machine::{lower, Layout, StoreLayout};
    use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
    use logicaffeine_compile::spatial::decode_engine::program;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let header = GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers/qwen3.5-0.8b-q8_0.json")).unwrap();
    let spec = header.spec().unwrap();
    let n = EngineNumerics::fastest();
    let steps = program::lower(&spec, n, spec.ctx_len).unwrap().program;
    let layout = Layout::of(&spec, n, &steps, &header, 1, spec.ctx_len.min(4096)).unwrap();
    let jobs = lower(&steps, &layout, spec.rms_eps as f32);
    EngineParams::of(&layout, &StoreLayout::of(&jobs, &layout, 1), &jobs)
}

/// The vector unit's pair copies (the rows a far Hadamard stage's partner and a rotary block read) are as deep as the
/// widest head the model's Hadamard and rotary jobs take, each a head at a time: the Qwen3.5-0.8B's widest attention head
/// in beats of sixteen lanes, where its registers are as deep as its longest vector, whether its cache is rotated (its
/// Hadamard and rotary jobs) or binary16 (its rotary jobs alone); one engine for several is as deep as the deepest; and
/// the vectored engine's side gives its unit that depth.
#[test]
fn the_vector_units_pair_copies_are_as_deep_as_the_widest_head_it_pairs_rows_of() {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::machine::{lower, Layout, StoreLayout};
    use logicaffeine_compile::spatial::decode_engine::model::{CacheFormat, EngineNumerics};
    use logicaffeine_compile::spatial::decode_engine::program;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams};
    let p = engine_08b();
    let (hd, vd, _, _) = p.attend.expect("the 0.8B attends");
    assert_eq!(p.pair_rows, hd.max(vd).div_ceil(16), "{p:?}");
    assert!(p.pair_rows < p.side_rows, "{} pair rows of {} rows the side holds", p.pair_rows, p.side_rows);
    let header = GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers/qwen3.5-0.8b-q8_0.json")).unwrap();
    let spec = header.spec().unwrap();
    let n = EngineNumerics { cache: CacheFormat::F16, ..EngineNumerics::fastest() };
    let steps = program::lower(&spec, n, spec.ctx_len).unwrap().program;
    let layout = Layout::of(&spec, n, &steps, &header, 1, spec.ctx_len.min(4096)).unwrap();
    let jobs = lower(&steps, &layout, spec.rms_eps as f32);
    let plain = EngineParams::of(&layout, &StoreLayout::of(&jobs, &layout, 1), &jobs);
    assert_eq!(plain.pair_rows, hd.div_ceil(16), "a binary16 cache's rotary jobs alone");
    let deeper = EngineParams { pair_rows: p.pair_rows + 3, ..p.clone() };
    for order in [[p.clone(), deeper.clone()], [deeper.clone(), p.clone()]] {
        assert_eq!(EngineParams::union(&order).unwrap().pair_rows, p.pair_rows + 3, "one engine for both");
    }
    let text = engine_verilog(&vectored_08b());
    assert!(
        text.contains(&format!(".VROWS({}), .PROWS({}),", p.side_rows, p.pair_rows)),
        "the vector side given its pair depth"
    );
    let vu = text.lines().find(|l| l.contains("logos_simd_p #(") && l.contains(") vu(")).expect("the side's vector unit");
    assert!(vu.contains(".PAIRS(PROWS)"), "and its unit: {vu}");
}

/// The vector unit's side is as deep as the deepest vector job it cannot run a chunk at a time: a job whose rows cross
/// nowhere ([`VectorJob::chunked`]) runs as many rows of its vectors at a time as the side holds, so the 27B's FFN
/// product (17,408 elements), its convolution (10,240) and its attention gate (6,144) take chunks and the side is as
/// deep as the norms and float matrices over its 5,120 (320 rows), while the reference engine's own vector unit, which
/// runs every job whole, stays as deep as the deepest of them all (1,088 rows); one engine for several is as deep as the
/// deepest of each, and the vectored engine's side gives its unit the side's depth.
#[test]
fn the_vector_side_is_as_deep_as_the_deepest_job_it_cannot_run_a_chunk_at_a_time() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams};
    let p = hybrid8();
    assert_eq!((p.vector_rows, p.side_rows), (1088, 320), "the 27B's and the 0.8B's: {p:?}");
    let small = engine_08b();
    assert_eq!((small.vector_rows, small.side_rows), (384, 64), "the 0.8B's: its convolution over 6,144, its norms over 1,024");
    let deeper = EngineParams { side_rows: p.side_rows + 5, ..p.clone() };
    for order in [[p.clone(), deeper.clone()], [deeper.clone(), p.clone()]] {
        let u = EngineParams::union(&order).unwrap();
        assert_eq!((u.vector_rows, u.side_rows), (1088, 325), "one engine for both");
    }
    let text = engine_verilog(&vectored_08b());
    assert!(text.contains(&format!(".VROWS({}), .PROWS({}),", small.side_rows, small.pair_rows)), "the vector side given the side's depth");
}

/// Compile with iverilog and run; the run's output.
fn simulate(files: &[PathBuf], defines: &[String]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let sim = dir.path().join("sim");
    let mut cmd = Command::new("iverilog");
    cmd.arg("-g2012").arg("-o").arg(&sim);
    for d in defines {
        cmd.arg(d);
    }
    let build = cmd.args(files).output().expect("iverilog");
    assert!(build.status.success(), "iverilog:\n{}", text(&build));
    let run = Command::new("vvp").arg(&sim).output().expect("vvp");
    assert!(run.status.success(), "vvp:\n{}", text(&run));
    text(&run)
}

// ── Simulation ──────────────────────────────────────────────────────────────────────────────────────

/// The shell end to end: the feature header and ID, the scratch register, a NOP, a fill and three checks of every
/// channel (one with a wrong seed, one after a bit is flipped in a channel's memory), copies between host memory and
/// a channel each way, an unknown op refused, and doorbells past a full command FIFO dropped, counted and flagged while
/// every command the FIFO took completes; every command with the core reporting its progress every 64 of its cycles,
/// a long fill's progress records carrying its tag, growing record to record and leaving the count of completed
/// commands alone, and a fill whose channels never take a line reporting none until they do.
#[test]
fn the_shell_moves_every_line_through_its_fifos() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let mut files = vec![fixture("tb_shell.v"), d.path().join("sim/logos_avmm_memory_model.v"), fixture("host_memory_model.v")];
    files.extend(shell_files(d.path()));
    let out = simulate(&files, &[]);
    assert!(out.contains("RESULT errors=0"), "the shell end to end:\n{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

/// Both FIFO variants under six clock pairs and traffic mixes: a faster reader, a faster writer, two clocks a part in
/// a thousand apart, a 3.7:1 ratio either way with one side nearly idle, and both sides at full rate. Every word comes
/// out in order and neither side ever claims more than the FIFO holds.
#[test]
fn every_fifo_keeps_its_order_across_unrelated_clocks() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let configs = [(1667, 1111, 1, 70, 60), (1111, 1667, 2, 70, 60), (1250, 1251, 3, 90, 95), (1000, 3700, 4, 95, 30), (3700, 1000, 5, 30, 95), (1667, 1111, 6, 100, 100)];
    for fifo in ["logos_dcfifo_mlab", "logos_dcfifo_m20k"] {
        for (whalf, rhalf, seed, wp, rp) in configs {
            let defines = vec![
                format!("-DFIFO={fifo}"),
                format!("-Ptb.WHALF={whalf}"),
                format!("-Ptb.RHALF={rhalf}"),
                format!("-Ptb.SEED={seed}"),
                format!("-Ptb.WP={wp}"),
                format!("-Ptb.RP={rp}"),
            ];
            let out = simulate(&[fixture("tb_dcfifo.v"), rtl(d.path(), "logos_dcfifo.v")], &defines);
            assert!(out.contains("RESULT sent=20000 got=20000 errors=0"), "{fifo} at {whalf}/{rhalf} ps, {wp}%/{rp}%:\n{out}");
        }
    }
}

/// A dual-clock FIFO built `HOLDS` stops a simulation whose writer lets a word the FIFO refused fall, offering another or
/// none at its next step, and runs one whose writer holds the word until it is taken to its end; a FIFO built without it
/// runs either to its end.
#[test]
fn a_fifo_stops_a_simulation_whose_writer_lets_a_refused_word_fall() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let dir = tempfile::tempdir().unwrap();
    let run = |hold: u32, holds: u32| {
        let sim = dir.path().join(format!("sim_{hold}_{holds}"));
        let build = Command::new("iverilog")
            .args(["-g2012", "-s", "tb", "-o"])
            .arg(&sim)
            .args([format!("-Ptb.HOLD={hold}"), format!("-Ptb.HOLDS={holds}")])
            .arg(fixture("tb_dcfifo_refused.v"))
            .arg(rtl(d.path(), "logos_dcfifo.v"))
            .output()
            .expect("iverilog");
        assert!(build.status.success(), "iverilog:\n{}", text(&build));
        let out = Command::new("vvp").arg(&sim).output().expect("vvp");
        (out.status.success(), text(&out))
    };
    let (ok, out) = run(1, 1);
    assert!(ok && out.contains("RESULT sent=400 got=400"), "a writer that holds a refused word runs to its end:\n{out}");
    let (ok, out) = run(0, 1);
    assert!(!ok && out.contains("a word the FIFO refused was not offered again") && !out.contains("RESULT"),
        "a writer that lets a refused word fall stops the simulation:\n{out}");
    let (ok, out) = run(0, 0);
    assert!(ok && out.contains("RESULT"), "a FIFO whose writer may let a word fall runs to its end:\n{out}");
}

/// A dual-clock FIFO built `TOKENS` takes every token its writer pulses, whatever its room: a writer pulsing tokens in
/// bursts that never looks at `wready`, as a stream engine marks its runs done, and a reader taking them at random, at
/// writes faster and slower than reads, two clocks a part in a thousand apart, and the read side stepping one clock in
/// eight as the compute side's does — every token comes out. Given the same pulses a plain FIFO loses some at every
/// one of them: its write side hears of a taken token two of its steps late and refuses the next.
#[test]
fn a_token_fifo_takes_every_token_its_writer_pulses_whatever_its_room() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let dir = tempfile::tempdir().unwrap();
    let configs = [(1667, 5000, 1, 1, 1), (1667, 1250, 1, 8, 2), (5000, 1667, 1, 1, 3), (1250, 1251, 1, 1, 4), (3334, 10000, 1, 1, 5)];
    for (whalf, rhalf, wce, rce, seed) in configs {
        let mut got = Vec::new();
        for tokens in [1, 0] {
            let sim = dir.path().join(format!("sim_{tokens}_{seed}"));
            let build = Command::new("iverilog")
                .args(["-g2012", "-s", "tb", "-o"])
                .arg(&sim)
                .args([
                    format!("-Ptb.TOKENS={tokens}"),
                    format!("-Ptb.WHALF={whalf}"),
                    format!("-Ptb.RHALF={rhalf}"),
                    format!("-Ptb.WCE={wce}"),
                    format!("-Ptb.RCE={rce}"),
                    format!("-Ptb.SEED={seed}"),
                ])
                .arg(fixture("tb_dcfifo_tokens.v"))
                .arg(rtl(d.path(), "logos_dcfifo.v"))
                .output()
                .expect("iverilog");
            assert!(build.status.success(), "iverilog:\n{}", text(&build));
            let run = Command::new("vvp").arg(&sim).output().expect("vvp");
            let out = text(&run);
            assert!(run.status.success(), "vvp:\n{out}");
            let line = out.lines().find(|l| l.starts_with("RESULT ")).unwrap_or_else(|| panic!("no result:\n{out}")).to_string();
            got.push(line);
        }
        let what = format!("writes at {whalf} ps, reads at {rhalf} ps a step in {rce}");
        assert_eq!(got[0], "RESULT given=2000 got=2000", "{what}: a token FIFO takes every token");
        let plain: u64 = got[1].rsplit("got=").next().unwrap().parse().unwrap();
        assert!(plain < 2000, "{what}: a plain FIFO given the same pulses loses some ({})", got[1]);
    }
}

/// Every dual-clock FIFO the shell builds says what its writer is: `TOKENS` a channel's done FIFOs (the stream engine
/// marks a run done with a pulse) and the host's write responses' (a response cannot wait), each a FIFO of one-bit
/// tokens; `HOLDS` every other but the doorbells' (a doorbell past a full FIFO is dropped and counted), the status
/// records' (a progress record the host has not made room for gives way to a later one) and a hybrid channel's fast
/// descriptors' (the channel multiplexer may offer another port's descriptor after one is refused, the refused one held
/// at its port), which say neither — in the plain shell and in a recurrent hybrid build's.
#[test]
fn every_fifo_the_shell_builds_says_what_its_writer_is() {
    let plain = emit(&AfuConfig::d5005());
    let hybrid = emit(&AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512));
    for (what, d, tokens) in [
        ("the plain shell", &plain, &["done_fifo", "hack_fifo"][..]),
        ("a recurrent hybrid build's shell", &hybrid, &["done_fifo", "fast_done_fifo", "hack_fifo"][..]),
    ] {
        let shell = std::fs::read_to_string(d.path().join("hw/rtl/logos_d5005_shell.v")).unwrap();
        let mut seen = Vec::new();
        for (at, _) in shell.match_indices("logos_dcfifo_") {
            let decl = &shell[at..];
            let decl = &decl[..decl.find(");").unwrap()];
            let Some(close) = decl.find(") ") else { continue };
            let params = &decl[..close + 1];
            let name: String = decl[close + 2..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if name.is_empty() {
                continue;
            }
            let (holds, token) = (params.contains(".HOLDS(1)"), params.contains(".TOKENS(1)"));
            if tokens.contains(&name.as_str()) {
                assert!(token && !holds && params.contains(".W(1)"), "{what}: {name}, a FIFO of tokens: {params}");
            } else if ["cmd_fifo", "st_fifo", "fast_desc_fifo"].contains(&name.as_str()) {
                assert!(!token && !holds, "{what}: {name}'s writer holds no one refused word: {params}");
            } else {
                assert!(holds && !token, "{what}: {name}'s writer holds a refused word: {params}");
            }
            seen.push(name);
        }
        let every: &[&str] = if tokens.contains(&"fast_done_fifo") { &["fast_desc_fifo", "fast_write_fifo", "fast_read_fifo"] } else { &[] };
        for name in tokens.iter().chain(&["cmd_fifo", "st_fifo", "desc_fifo", "write_fifo", "read_fifo", "hr_fifo", "hw_fifo"]).chain(every) {
            assert!(seen.iter().any(|n| n == name), "{what}: {name} among its FIFOs {seen:?}");
        }
    }
}

/// Each FIFO side steps only at its enable (`wce`, `rce`), as a side clocked that much slower does — the shell's compute
/// side is a FIFO side stepping at the compute clock's rising edges: under enables high one cycle in 3 to 64 of either
/// clock, a producer and a consumer acting only at their sides' steps, every word comes out once and in order, neither
/// side claims more than the FIFO holds, and neither side's outputs move at an edge its enable is low for, so the other
/// side's pointer reaches a side only as it steps.
#[test]
fn every_fifo_side_steps_only_at_its_enable() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let configs =
        [(1667, 1111, 7, 70, 60, 3, 1), (1111, 1667, 8, 70, 60, 1, 5), (1250, 1251, 9, 90, 95, 4, 4), (1000, 3700, 10, 95, 30, 64, 1), (3700, 1000, 11, 30, 95, 1, 64), (1667, 1111, 12, 100, 100, 6, 7)];
    for fifo in ["logos_dcfifo_mlab", "logos_dcfifo_m20k"] {
        for (whalf, rhalf, seed, wp, rp, wce, rce) in configs {
            let defines = vec![
                format!("-DFIFO={fifo}"),
                format!("-Ptb.WHALF={whalf}"),
                format!("-Ptb.RHALF={rhalf}"),
                format!("-Ptb.SEED={seed}"),
                format!("-Ptb.WP={wp}"),
                format!("-Ptb.RP={rp}"),
                format!("-Ptb.WCE={wce}"),
                format!("-Ptb.RCE={rce}"),
                "-Ptb.N=4000".to_string(),
            ];
            let out = simulate(&[fixture("tb_dcfifo.v"), rtl(d.path(), "logos_dcfifo.v")], &defines);
            assert!(out.contains("RESULT sent=4000 got=4000 errors=0"), "{fifo} at {whalf}/{rhalf} ps, {wp}%/{rp}%, stepping 1 in {wce}/{rce}:\n{out}");
        }
    }
}

/// A channel's stream engine writes 300 lines in bursts and reads them back through a 64-line FIFO a consumer drains
/// at 10% to 100% of its cycles, or in short runs between long stops that fill the FIFO to its last line: every line
/// right, and none arriving at a full FIFO.
#[test]
fn the_stream_engine_reserves_room_for_every_line_it_reads() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    for (seed, rp, stop) in [(3, 30, 0), (4, 100, 0), (5, 10, 0), (6, 60, 0), (7, 100, 1), (8, 100, 1), (9, 70, 1)] {
        let out = simulate(
            &[fixture("tb_stream.v"), d.path().join("sim/logos_avmm_memory_model.v"), rtl(d.path(), "logos_stream.v"), rtl(d.path(), "logos_dcfifo.v")],
            &[format!("-Ptb.SEED={seed}"), format!("-Ptb.RP={rp}"), format!("-Ptb.STOP={stop}")],
        );
        assert!(out.contains("RESULT written=300 got=300 errors=0 owed=0"), "seed {seed}, consumer {rp}%, stopping {stop}:\n{out}");
    }
}

// ── Clock crossings ─────────────────────────────────────────────────────────────────────────────────

fn cdc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new("python3").arg(script("cdc_check.py")).args(args).output().expect("python3")
}

/// The crossing gate under the D5005 tools' Python, whose pyslang the single-clock proof reads a design with: only
/// after `common::require_tool("d5005-pim")`.
fn cdc_boxed(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(root().join("tools/d5005/venv/bin/python")).arg(script("cdc_check.py")).args(args).output().expect("the d5005 venv's python")
}

fn cdc_tools() -> bool {
    common::require_tool("yosys") && common::require_tool("python3")
}

/// The emitted shell crosses clocks only through its FIFOs, and each FIFO only through its Gray pointers and its RAM,
/// at every line the channels carry: the map sets the shell's parameters as the build does, so on SECDED lines the
/// gate reads the coder, the decoder's stage and the counts, more registers than the plain lines' shell has.
#[test]
fn every_crossing_of_the_shell_is_a_fifo() {
    if !cdc_tools() {
        return;
    }
    let mut registers = Vec::new();
    for line in [LocalMemoryLine::DataAndEcc576, LocalMemoryLine::Data512, LocalMemoryLine::Data512Secded] {
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
        args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
        let o = cdc(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
        assert_eq!(o.status.code(), Some(0), "{line:?}: {}", text(&o));
        assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("23 crossings, 6 clocks"), "{line:?}: {}", text(&o));
        let said = text(&o);
        let count = said.split(" registers,").next().and_then(|h| h.rsplit(' ').next()).and_then(|n| n.parse::<u64>().ok());
        registers.push(count.unwrap_or_else(|| panic!("{line:?}: a register count in {said}")));
    }
    assert!(registers[2] > registers[1], "the SECDED shell's registers {} against the plain one's {}", registers[2], registers[1]);
    let d = emit(&AfuConfig::d5005());
    for fifo in ["logos_dcfifo_mlab", "logos_dcfifo_m20k"] {
        let o = cdc(&["fifo".as_ref(), fifo.as_ref(), rtl(d.path(), "logos_dcfifo.v").as_os_str()]);
        assert_eq!(o.status.code(), Some(0), "{fifo}: {}", text(&o));
    }
}

/// The gate refuses each defect it exists for: a register sampling another clock's register, an output computed from
/// another clock's register, a register clocked by logic, a port the map gives no clock; and in a FIFO, a synchronizer
/// fed through logic and a binary pointer crossing. A value through a FIFO passes.
#[test]
fn the_crossing_gate_refuses_each_defect() {
    if !cdc_tools() {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let fifo = rtl(d.path(), "logos_dcfifo.v");
    let map = fixture("cdc/xtop.json");
    let run = |map: &Path, design: &str| cdc(&["shell".as_ref(), map.as_os_str(), fifo.as_os_str(), fixture(&format!("cdc/{design}.v")).as_os_str()]);
    let o = run(&map, "through_fifo");
    assert_eq!(o.status.code(), Some(0), "a value through a FIFO: {}", text(&o));
    for (design, says) in [
        ("raw_crossing", "b_out (b_clk) reads a_q[0] from a_clk"),
        ("logic_crossing", "output b_out (b_clk) is driven from a_clk"),
        ("derived_clock", "b_out: clocked by slow, which is no clock port"),
    ] {
        let o = run(&map, design);
        assert_eq!(o.status.code(), Some(1), "{design} is refused: {}", text(&o));
        assert!(text(&o).contains(says), "{design}: {}", text(&o));
    }
    let o = run(&fixture("cdc/unmapped_port.json"), "through_fifo");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("ports with no clock") && text(&o).contains("b_out"), "{}", text(&o));

    let source = std::fs::read_to_string(&fifo).unwrap();
    for (from, to, module, says) in [
        ("      rgray_s1 <= rgray;", "      rgray_s1 <= rbin ^ (rbin >> 1);", "logos_dcfifo_mlab", "through logic"),
        ("      wgray_s1 <= wgray;", "      wgray_s1 <= wbin;", "logos_dcfifo_m20k", "wgray_s1 (rclk) reads wbin[0] from wclk"),
    ] {
        assert_eq!(source.matches(from).count(), 2, "the mutation site {from:?} is in both FIFOs");
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("logos_dcfifo.v");
        std::fs::write(&bad, source.replace(from, to)).unwrap();
        let o = cdc(&["fifo".as_ref(), module.as_ref(), bad.as_os_str()]);
        assert_eq!(o.status.code(), Some(1), "{module} with {to:?}: {}", text(&o));
        assert!(text(&o).contains(says), "{module}: {}", text(&o));
    }
}

/// A module named `--single-clock` is proven to have one clock and then stands in the shell as a box of that clock's
/// domain: every register, memory port and child under it clocked from its one clock input, every value into it from
/// that clock's domain. A value through a FIFO and on through such a box (two levels, a falling-edge register in the
/// child; or a register reset asynchronously by an input of the box) passes; a box with a register reset from a
/// register of its own, a register on a second clock or a child on a clock it derives, a box fed straight from the
/// other clock, and a box instantiated at parameters its proof was not made at are refused.
#[test]
fn the_crossing_gate_proves_a_single_clock_box() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let fifo = rtl(d.path(), "logos_dcfifo.v");
    let map = fixture("cdc/xtop.json");
    let run = |design: &str| {
        cdc_boxed(&["shell".as_ref(), "--single-clock".as_ref(), "inner".as_ref(), map.as_os_str(), fifo.as_os_str(), fixture(&format!("cdc/{design}.v")).as_os_str()])
    };
    for design in ["through_box", "box_async_reset"] {
        let o = run(design);
        assert_eq!(o.status.code(), Some(0), "{design}: through a FIFO and a single-clock box: {}", text(&o));
        assert!(text(&o).contains("1 single-clock box"), "{design}: {}", text(&o));
    }
    for (design, says) in [
        ("box_reset_not_input", "a block is reset by r, no input of inner"),
        ("box_two_clocks", "inner is not single-clock"),
        ("box_derived_clock", "l is clocked by half, no input of inner"),
        ("box_raw_input", "u.d (b_clk) is driven from a_clk"),
        ("box_overridden", "inner is boxed at the parameters it was proven at, but u sets W"),
    ] {
        let o = run(design);
        assert_eq!(o.status.code(), Some(1), "{design} is refused: {}", text(&o));
        assert!(text(&o).contains(says), "{design}: {}", text(&o));
    }
}

/// Qwen3.5-0.8B's engine build's shell crosses clocks only through its FIFOs, its compute side reaching the engine only
/// through the engine's step boundary (`logos_engine_bridge`), declared beside them, on the one user clock both step in;
/// the engine's core (`logos_engine_core_ce`) proven single-clock and boxed as the clock-domain map names it rather than
/// flattened: the gate build-afu.sh runs on every engine build, in a minute or two where flattening the engine took
/// hours.
#[test]
fn the_engine_builds_shell_crosses_only_through_fifos() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let start = std::time::Instant::now();
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("24 crossings, 6 clocks, 1 single-clock box"), "{}", text(&o));
    assert!(start.elapsed().as_secs() < 600, "the gate took {:?}", start.elapsed());
}

/// The engine for Qwen3.5-0.8B as a hybrid engine: its matvec jobs on a streaming unit for the formats it streams, one
/// pair a channel.
fn hybrid_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let p = engine_08b();
    let formats: Vec<GgmlType> = p.formats.iter().map(|&(ty, _)| ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    EngineParams { streaming: Some(Streaming::new(set, ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) })), ..p }
}

/// Qwen3.5-0.8B's hybrid engine with the recurrence unit beside its streaming unit.
fn recurrent_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = hybrid_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_recurrence()), ..p }
}

/// A hybrid engine's fast ports each cross [`PORT_PIPES`] registers each way at the engine's edge (`logos_pipe_chain`, a
/// `logos_pipe` a register, in the engine's `port_pipe` generate), so the path from a channel's bank at the die's edge to
/// the fast side crosses the die a register each part of the way: the descriptors and the lines, and with the recurrence
/// unit the writes and their done tokens too, each between the engine's channel ports (`sm_*`) and the fast side's
/// (`p_*`), never the fast side on a channel port itself.
#[test]
fn a_hybrid_engines_fast_ports_each_cross_a_register_each_way() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{census, engine_verilog, PORT_PIPES};
    for (what, p, streams) in [
        ("the hybrid engine", hybrid_08b(), &["md", "mr"][..]),
        ("the recurrent hybrid", recurrent_08b(), &["md", "mr", "mw", "mdone"][..]),
    ] {
        let v = engine_verilog(&p);
        assert_eq!(census(&v, "logos_engine").get("logos_pipe_chain"), Some(&streams.len()), "{what}: a chain a stream");
        assert!(PORT_PIPES >= 2, "a chain of {PORT_PIPES}");
        let chains = v.matches(&format!("logos_pipe_chain #(.W(")).count();
        assert_eq!(v.matches(&format!(".K({PORT_PIPES})) ")).count(), chains, "{what}: every chain {PORT_PIPES} registers");
        let top = &v[v.find("module logos_engine\n").expect("the engine's top")..];
        let top = &top[..top.find("endmodule").unwrap()];
        for s in streams {
            let (outward, port) = match *s {
                "md" => (true, "sm_md"),
                "mr" => (false, "sm_mr"),
                "mw" => (true, "sm_mw"),
                _ => (false, "sm_mdone"),
            };
            let side = if outward { "out" } else { "in" };
            assert!(top.contains(&format!(") {s}(")) && top.contains(&format!(".{side}_valid({port}_valid[pipe_ch])")), "{what}: {s} through its register:\n{top}");
        }
        let fast = &top[top.find("logos_engine_fast fast(").expect("the fast side")..];
        let fast = &fast[..fast.find(");").unwrap()];
        for s in streams {
            assert!(fast.contains(&format!("({}_valid)", s.replacen('m', "p_m", 1))) && !fast.contains(&format!("(sm_{s}_valid)")), "{what}: the fast side's {s} on its register:\n{fast}");
        }
    }
}

/// [`recurrent_08b`] with the vector unit's side beside the recurrence unit.
fn vectored_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = recurrent_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_vectors()), ..p }
}

/// [`vectored_08b`] with the attention side beside the vector unit's side.
fn attended_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = vectored_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_attention()), ..p }
}

/// Qwen3.5-0.8B's engine as hybrid6b builds the card's: sixteen token-rows over a greedy store, four pairs a channel,
/// the recurrence unit at a line a clock, the vector unit's side and the attention side.
fn fast_side_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    fast_side_08b_packed(false, 1)
}

/// [`fast_side_08b`] with Q8_0 in octets given `octets` and every block in a slot of a multiple of `slot` bytes.
fn fast_side_08b_packed(octets: bool, slot: usize) -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::machine::{lower, Layout, StoreLayout};
    use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
    use logicaffeine_compile::spatial::decode_engine::program;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let header = GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers/qwen3.5-0.8b-q8_0.json")).unwrap();
    let spec = header.spec().unwrap();
    let n = EngineNumerics::fastest();
    let steps = program::lower(&spec, n, spec.ctx_len).unwrap().program;
    let layout = Layout::of(&spec, n, &steps, &header, 16, spec.ctx_len.min(4096)).unwrap();
    let jobs = lower(&steps, &layout, spec.rms_eps as f32);
    let p = EngineParams::of(&layout, &StoreLayout::greedy(&jobs, &layout, 16), &jobs);
    let formats: Vec<GgmlType> = p.formats.iter().map(|&(ty, _)| ty).collect();
    let set = ColumnSet::hybrid(&formats, n.activations);
    let set = if octets { set.with_octets() } else { set };
    let set = if slot > 1 { set.with_slots(slot) } else { set };
    let streaming = Streaming::new(set, ImageLayout { stripe: Some(4), octets, slot: slot as u64, ..ImageLayout::d5005(Beat::Data64) })
        .with_recurrence()
        .with_recurrence_width(2)
        .with_vectors()
        .with_attention();
    EngineParams { streaming: Some(streaming), ..p }
}

/// [`fast_side_08b`] with the recurrence's value heads on four head lanes, each lane's copy of the log as long as the
/// 0.8B's.
fn lanes_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::machine::Layout;
    use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
    use logicaffeine_compile::spatial::decode_engine::program;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let header = GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers/qwen3.5-0.8b-q8_0.json")).unwrap();
    let spec = header.spec().unwrap();
    let n = EngineNumerics::fastest();
    let layout = Layout::of(&spec, n, &program::lower(&spec, n, spec.ctx_len).unwrap().program, &header, 16, spec.ctx_len.min(4096)).unwrap();
    let p = fast_side_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_recurrence_lanes(4, layout.log_lines())), ..p }
}

/// The text of module `name` in `verilog`.
fn module_text(verilog: &str, name: &str) -> String {
    let at = [" ", "\n", "("].iter().find_map(|c| verilog.find(&format!("module {name}{c}"))).unwrap_or_else(|| panic!("module {name}"));
    let end = at + verilog[at..].find("endmodule").unwrap() + "endmodule".len();
    format!("{}\n", &verilog[at..end])
}

/// The memory parts module `top` builds itself (the pipelined store's layers: `<memory>_part`s, as a numbered memory
/// stacks its own), boxed in its stage as the numbered memories are.
fn parts_built_by<'a>(verilog: &'a str, top: &str) -> Vec<&'a str> {
    let body = module_text(verilog, top);
    verilog
        .lines()
        .filter_map(|l| l.strip_prefix("module "))
        .map(|rest| &rest[..rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len())])
        .filter(|name| name.ends_with("_part") && body.lines().any(|l| l.trim_start().starts_with(&format!("{name} "))))
        .collect()
}

/// The comment lines written directly above `module name`, its documentation.
fn module_doc(verilog: &str, name: &str) -> String {
    let at = [" ", "\n", "("].iter().find_map(|c| verilog.find(&format!("module {name}{c}"))).unwrap_or_else(|| panic!("module {name}"));
    let mut doc: Vec<&str> = verilog[..at].lines().rev().take_while(|l| l.trim_start().starts_with("//")).collect();
    doc.reverse();
    doc.join("\n")
}

/// Qwen3.5-0.8B's hybrid engine build's shell crosses clocks only through its FIFOs — the core's and the channels' fast
/// ports' — its compute side reaching the engine only through the step boundary (`logos_engine_bridge`), the engine's
/// core and its user-clock side each proven single-clock and boxed as the clock-domain map names them; the boundary
/// proves its own structure.
#[test]
fn the_hybrid_engine_builds_shell_crosses_only_through_fifos_and_its_bridge() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("32 crossings, 6 clocks, 2 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_engine_bridge: clean;"), "{}", text(&o));
}

/// Qwen3.5-0.8B's recurrent hybrid build's shell crosses clocks only through its FIFOs — the core's, and the channels'
/// fast ports' for reads, writes and write done tokens — and its bridge, the core and the user-clock side boxed; the
/// bridge, the card port's map crossing with the engine's ports, proves its own structure.
#[test]
fn the_recurrent_hybrid_builds_shell_crosses_only_through_fifos_and_its_bridge() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("40 crossings, 6 clocks, 2 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_engine_bridge: clean;"), "{}", text(&o));
}

/// Qwen3.5-0.8B's hybrid engine build with the vector unit's side beside the recurrence unit crosses clocks exactly as the
/// recurrent build does: the side lives in the user-clock box, takes its tables from the step boundary the core takes
/// them from, and adds no crossing; the boundary proves its own structure.
#[test]
fn the_vectored_hybrid_builds_shell_crosses_only_through_fifos_and_its_bridge() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(vectored_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("40 crossings, 6 clocks, 2 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_engine_bridge: clean;"), "{}", text(&o));
}

/// A vectored hybrid build hands each vector job to the vector unit's side: the core goes from a vector job's header to
/// the hand-off, past the job's words, and keeps the vector-job states only unreached; the user-clock side starts the
/// side on the core's word, writes its tables at the core's step from the crossing's host writes, gives it the store
/// and the recurrence unit's port while it runs, and holds its busy beside the other units'.
#[test]
fn a_vectored_hybrid_build_hands_its_vector_jobs_to_the_fast_side() {
    let d = emit(&AfuConfig::engine(vectored_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let core = module_text(&v, "logos_engine_core_ce");
    for line in [
        "        if (jw[3:0] == 4'd2) state <= V_SG;\n",
        "      V_SG: if (wq_empty) begin\n",
        "        vs_pc <= pc - 32'd1;\n",
        "        pc <= pc + 32'd1 + {24'd0, nfresh} + {23'd0, nload, 1'b0} + {23'd0, nput, 1'b0} + {16'd0, nprog};\n",
        "  assign sm_vec_job = {state_f16, rows, vs_pc};\n",
    ] {
        assert_eq!(core.matches(line).count(), 1, "{line}");
    }
    assert!(!core.contains("state <= HEADS;"), "no vector job reaches the core's vector-job states");
    let fast = module_text(&v, "logos_engine_fast");
    for line in [
        "wire v_start = vec_go && cr;",
        "logos_vector_side #(",
        ".host_we(ce && host_wr)",
        ".clear(g_start || v_start)",
        ".rq_valid(vec ? v_rq_valid : rq_valid)",
        "assign md_valid = (gdn || vec) ? p_md_valid : u_md_valid;",
        "else if (cr) busy_held <= busy || start || g_busy || writing || g_start || v_busy || v_start;",
        ".ra(own ? (vec ? v_ra : gdn ? g_ra : u_ra) : e_ra)",
    ] {
        assert!(fast.contains(line), "{line}: {fast}");
    }
    let top = module_text(&v, "logos_engine");
    assert!(top.contains(".host_wr(f_host_wr), .host_table(f_host_table)"), "the side's tables from the crossing's host writes: {top}");
    for module in ["logos_vector_side", "logos_simd_p", "logos_lane_func_p", "logos_recip_p", "logos_rsqrt_p", "logos_ln_p"] {
        assert_eq!(v.matches(&format!("module {module}")).count(), 1, "{module}");
    }
}

/// An attended hybrid build hands each token-row of an attention job to the attention side: the core goes from the
/// job's header to a row's hand-off, the row's region, its block's cache, its positions, the score scale and its slots'
/// beats in the side's word, and keeps no attention unit, no quantizer and no cache read of its own; the user-clock side
/// starts the side on the core's word, gives it the store and the recurrence unit's port while it runs, and holds its
/// busy beside the other units'.
#[test]
fn an_attended_hybrid_build_hands_each_attention_row_to_the_fast_side() {
    let d = emit(&AfuConfig::engine(attended_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let core = module_text(&v, "logos_engine_core_ce");
    for line in [
        "      AT_ROW: if (k == rows) state <= next_job; else state <= AT_SG;\n",
        "      AT_SG: if (wq_empty) begin sm_own <= 1'b1; sm_att_go <= 1'b1; state <= AT_SIDE; end\n",
        "  assign sm_att_job = {at_heads, at_obeat, at_qbeat, at_scale, position + 32'd1, at_kvh, at_vscales, at_kscales, at_vals, at_keys,\n",
    ] {
        assert_eq!(core.matches(line).count(), 1, "{line}");
    }
    for gone in ["logos_attend_int8", "logos_act_stream", "AT_FETCH:", "at_krow", "state <= AT_HEAD;"] {
        assert!(!core.contains(gone), "the attended core keeps {gone}");
    }
    let fast = module_text(&v, "logos_engine_fast");
    for line in [
        "wire a_start = att_go && cr;",
        "logos_attend_side #(",
        ".rq_valid(att ? a_rq_valid : vec ? v_rq_valid : rq_valid)",
        "assign md_valid = (gdn || vec || att) ? p_md_valid : u_md_valid;",
        "|| v_busy || v_start || a_busy || a_start;",
        ".ra(own ? (att ? a_ra : vec ? v_ra : gdn ? g_ra : u_ra) : e_ra)",
    ] {
        assert!(fast.contains(line), "{line}: {fast}");
    }
    for module in ["logos_attend_side", "logos_attend_int8_p", "logos_act_stream_p"] {
        assert_eq!(v.matches(&format!("module {module}")).count(), 1, "{module}");
    }
}

/// An attended hybrid build elaborates against the D5005's PIM.
#[test]
fn an_attended_hybrid_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(attended_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A build whose recurrence runs on four head lanes keeps every other unit as the fast-side build does and puts the
/// lanes beside them: `logos_gdn_lanes` where `logos_gdn_side` stood, four lanes of the log's length, a fast port for
/// each lane cleared at the row's start, the shared port and the lanes' on the channels through a five-port channel
/// mux, the recurrence's fault any port's or the lanes' own; each module once.
#[test]
fn a_build_with_its_recurrence_on_head_lanes_puts_a_fast_port_under_each_lane_and_the_ports_through_the_mux() {
    let p = lanes_08b();
    let s = p.streaming.clone().unwrap();
    assert_eq!((s.recurrence_lanes, s.log_lines), (4, 16 * 129), "the 0.8B's log: sixteen entries of 129 lines");
    let one = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    let one = std::fs::read_to_string(rtl(one.path(), "logos_engine.v")).unwrap();
    let side = one.lines().find(|l| l.trim_start().starts_with("logos_gdn_side #(")).expect("the one-lane build's side").trim_start();
    let params = &side[..side.find(")) side(").expect("the side's parameters") + 1];
    // The store's read latency is each reader's last parameter, the lanes' own before it.
    let (params, srl) = params.rfind(", .SRL(").map_or((params, ""), |at| (&params[..at], &params[at..]));
    let lanes = format!("{}, .L(4), .LBA(12), .LOG_LINES(2064){srl}) side(", params.replacen("logos_gdn_side", "logos_gdn_lanes", 1));
    let d = emit(&AfuConfig::engine(p, LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let fast = module_text(&v, "logos_engine_fast");
    for line in [
        lanes.as_str(),
        ".written(!writing), .fault(lanes_fault), .l_rq_valid(l_rq_valid),",
        "for (lk = 0; lk < 4; lk = lk + 1) begin : lane_port",
        ".granule_log2(map_granule_log2), .clear(g_start), .fault(l_fault[lk]),",
        "logos_channel_mux #(.P(5), .NB(4),",
        ".md_valid(p_md_valid), .md_ready(md_ready), .md_data(p_md_data), .mr_valid(mr_valid), .mr_ready(p_mr_ready),",
        "assign gdn_fault = p_fault || l_fault != 4'd0 || lanes_fault;",
        ".fault(p_fault), .rq_valid(att ? a_rq_valid : vec ? v_rq_valid : rq_valid),",
        ".md_valid(c_md_valid[3:0]), .md_ready(c_md_ready[3:0]),",
    ] {
        assert_eq!(fast.matches(line).count(), 1, "{line}: {fast}");
    }
    assert!(!fast.contains("logos_gdn_side #("), "the side alone stands nowhere: {fast}");
    for module in ["logos_gdn_lanes", "logos_gdn_lane", "logos_gdn_fold_m", "logos_channel_mux", "logos_gdn_side", "logos_fast_port"] {
        assert_eq!(v.matches(&format!("module {module} ")).count() + v.matches(&format!("module {module}\n")).count(), 1, "{module}");
    }
    for gone in ["module logos_gdn_lanes", "logos_gdn_lanes #(", "module logos_gdn_lane ", "logos_channel_mux", "begin : lane_port", "p_fault"] {
        assert!(!one.contains(gone), "a build of one lane carries {gone}");
    }
}

/// A build whose recurrence runs on four head lanes crosses clocks exactly as the fast-side build does: the lanes, their
/// ports and the mux live in the user-clock box and add no crossing; the boundary proves its own structure.
#[test]
fn a_build_with_its_recurrence_on_head_lanes_crosses_only_through_fifos_and_its_bridge() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(lanes_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("40 crossings, 6 clocks, 2 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_engine_bridge: clean;"), "{}", text(&o));
}

/// A build whose recurrence runs on four head lanes elaborates against the D5005's PIM with no problem the front end
/// reports, warnings included.
#[test]
fn a_build_with_its_recurrence_on_head_lanes_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(lanes_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// [`lanes_08b`] with its memories flushed a window of [`TRAILING_TOKENS`] tokens behind: the coordinator's ring, the
/// trailing lanes and their flush units.
///
/// [`TRAILING_TOKENS`]: logicaffeine_compile::spatial::decode_engine::model::TRAILING_TOKENS
fn trailing_lanes_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::model::TRAILING_TOKENS;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    use logicaffeine_compile::spatial::serving::StateFlush;
    let p = lanes_08b();
    EngineParams { delta: p.delta.map(|(w, _)| (w, StateFlush::Trailing { tokens: TRAILING_TOKENS })), ..p }
}

/// A build whose recurrence replays its logs a window behind on four head lanes elaborates against the D5005's PIM with
/// no problem the front end reports, warnings included, and names its streaming unit with the window.
#[test]
fn a_build_with_its_recurrence_replayed_a_window_behind_on_head_lanes_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let p = trailing_lanes_08b();
    assert!(p.streaming_signature().ends_with("+behind8"), "{}", p.streaming_signature());
    let d = emit(&AfuConfig::engine(p, LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A build whose head lanes and coordinator ask further ahead than [`LANE_CREDIT`] gives `logos_gdn_lanes` that credit and
/// elaborates against the D5005's PIM with no problem the front end reports, warnings included; a build that gives none
/// leaves the lanes their own. A credit below [`LANE_CREDIT`], past [`LANE_CREDIT_MAX`], or on a build whose recurrence
/// has no head lanes is refused.
///
/// [`LANE_CREDIT`]: logicaffeine_compile::spatial::decode_engine::rtl::gdn::LANE_CREDIT
/// [`LANE_CREDIT_MAX`]: logicaffeine_compile::spatial::decode_engine::rtl::gdn::LANE_CREDIT_MAX
#[test]
fn a_build_whose_head_lanes_read_further_ahead_gives_them_its_credit_and_elaborates_against_the_d5005_pim() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams};
    use logicaffeine_compile::spatial::decode_engine::rtl::gdn::{LANE_CREDIT, LANE_CREDIT_MAX};
    let credited = |p: EngineParams, credit: usize| EngineParams { streaming: p.streaming.clone().map(|s| s.with_recurrence_credit(credit)), ..p };
    let lanes_line = |v: &str| v.lines().find(|l| l.contains("logos_gdn_lanes #(")).expect("the lanes' instance").to_string();
    let p = credited(trailing_lanes_08b(), 128);
    assert!(lanes_line(&engine_verilog(&p)).contains(".OUTSTANDING(128), "), "{}", lanes_line(&engine_verilog(&p)));
    for (what, plain) in [("every sixteen", lanes_08b()), ("a window behind", trailing_lanes_08b())] {
        assert!(!lanes_line(&engine_verilog(&plain)).contains("OUTSTANDING"), "{what}: {}", lanes_line(&engine_verilog(&plain)));
        assert!(lanes_line(&engine_verilog(&credited(plain.clone(), LANE_CREDIT_MAX))).contains(&format!(".OUTSTANDING({LANE_CREDIT_MAX}), ")), "{what}");
        for wrong in [LANE_CREDIT - 1, LANE_CREDIT_MAX + 1] {
            assert!(std::panic::catch_unwind(|| credited(plain.clone(), wrong)).is_err(), "{what}: a credit of {wrong} lines");
        }
    }
    assert!(std::panic::catch_unwind(|| credited(fast_side_08b(), 128)).is_err(), "a credit on a recurrence with no head lanes");
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(p, LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A build's vector unit's side runs one to [`VECTOR_PASSES_MAX`] passes a group, its unit holding as many passes'
/// scalars and running sums (`logos_simd_p`'s `PASSES`, the side's own): none and one past the most are refused, as is a
/// group on a build with no vector side.
///
/// [`VECTOR_PASSES_MAX`]: logicaffeine_compile::spatial::decode_engine::rtl::engine::VECTOR_PASSES_MAX
#[test]
fn a_build_runs_one_to_its_most_passes_a_group_on_its_vector_side() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams, VECTOR_PASSES_MAX};
    let grouped = |p: EngineParams, passes: usize| EngineParams { streaming: p.streaming.clone().map(|s| s.with_vector_passes(passes)), ..p };
    let side_line = |v: &str| v.lines().find(|l| l.contains("logos_vector_side #(")).expect("the vector side").to_string();
    for passes in [1, 5, 16, VECTOR_PASSES_MAX] {
        let v = engine_verilog(&grouped(vectored_08b(), passes));
        let rest = &v[v.find("  logos_vector_side #(").expect("the side's instance")..];
        let head = &rest[..rest.find(" vside(").expect("its parameters' end")];
        assert!(head.contains(&format!(".PASSES({passes})")), "{passes} passes: {}", side_line(&v));
    }
    for wrong in [0, VECTOR_PASSES_MAX + 1] {
        assert!(std::panic::catch_unwind(|| grouped(vectored_08b(), wrong)).is_err(), "{wrong} passes a group");
    }
    assert!(std::panic::catch_unwind(|| grouped(hybrid_08b(), 2)).is_err(), "a group on a build with no vector side");
}

/// A build's vector unit's side holds one to [`VECTOR_PASSES_MAX`] passes of the deepest vector job it runs whole: its
/// unit that many vectors deep (`logos_vector_side`'s `DEPTH`), a group as many passes as the unit holds, the side's own
/// depth — its chunks, its float matrices' held inputs — still that job's vector. None and one past the most are
/// refused, as is a depth on a build with no vector side.
///
/// [`VECTOR_PASSES_MAX`]: logicaffeine_compile::spatial::decode_engine::rtl::engine::VECTOR_PASSES_MAX
#[test]
fn a_build_holds_one_to_its_most_passes_of_its_deepest_vector_job_on_its_vector_side() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams, VECTOR_PASSES_MAX};
    use logicaffeine_compile::spatial::decode_engine::rtl::vector_side::vector_side_verilog;
    let deep = |p: EngineParams, depth: usize| EngineParams { streaming: p.streaming.clone().map(|s| s.with_vector_depth(depth)), ..p };
    let side_rows = vectored_08b().side_rows;
    for depth in [1, 5, VECTOR_PASSES_MAX] {
        let v = engine_verilog(&deep(vectored_08b(), depth));
        let rest = &v[v.find("  logos_vector_side #(").expect("the side's instance")..];
        let head = &rest[..rest.find(" vside(").expect("its parameters' end")];
        assert!(head.contains(&format!(".VROWS({side_rows}), .PROWS(")), "the side a vector deep at depth {depth}: {head}");
        assert!(head.contains(&format!(".DEPTH({depth})")), "depth {depth}: {head}");
    }
    let side = vector_side_verilog();
    assert!(side.contains("logos_simd_p #(.ROWS(VROWS * DEPTH),"), "the unit DEPTH vectors deep");
    assert!(side.contains("if (q * rows_a_pass <= $unsigned(VROWS * DEPTH)) gfit = q[7:0];"), "a group as many passes as the unit holds");
    assert!(side.contains("logos_bram_w512 #(.D(FMM * VROWS), .A(FIA)) act("), "the float matrices' held inputs a vector each");
    assert!(side.contains("localparam [31:0] VB = VROWS;"), "a chunk a vector");
    for wrong in [0, VECTOR_PASSES_MAX + 1] {
        assert!(std::panic::catch_unwind(|| deep(vectored_08b(), wrong)).is_err(), "a depth of {wrong}");
    }
    assert!(std::panic::catch_unwind(|| deep(hybrid_08b(), 2)).is_err(), "a depth on a build with no vector side");
}

/// [`fast_side_08b`] with its streaming unit's cores paired on processing elements in a clock of twice theirs.
fn paired_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let p = fast_side_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| Streaming { set: s.set.with_pe_pairs(), ..s }), ..p }
}

/// An engine whose streaming unit pairs its cores takes the PIM's two user clocks, one PLL's, their rising edges together:
/// the user side in `uClk_usrDiv2` (the shell's `uclk`) and the pairs' processing elements in `uClk_usr` (`uclk2`, the
/// engine's `clk2`, through the fast side and the streaming unit to the column array's pairs). The AFU's JSON asks for
/// the high clock at twice the user side's; the clock map names `uclk2` and the pairs; the cosimulation drives `uclk2`
/// at half the user period. A build without pairs is as it was: `uClk_usr` its user side, no `uclk2` anywhere.
#[test]
fn an_engine_of_paired_cores_takes_its_pairs_clock_from_the_pims_user_clocks() {
    use logicaffeine_compile::spatial::decode_engine::ofs::CLOCK_DOMAINS_FILE;
    let read = |d: &std::path::Path, f: &str| std::fs::read_to_string(d.join(f)).unwrap();
    let paired = emit(&AfuConfig::engine(paired_08b(), LocalMemoryLine::Data512));
    let d = paired.path();
    let top = read(d, "hw/rtl/ofs_plat_afu.sv");
    for want in [
        "  wire uclk = plat_ifc.clocks.ports[0].uClk_usrDiv2.clk;\n",
        "  wire uclk_rst_n = plat_ifc.clocks.ports[0].uClk_usrDiv2.reset_n;\n",
        "  wire uclk2 = plat_ifc.clocks.ports[0].uClk_usr.clk;\n",
        ".uclk(uclk), .uclk_rst_n(uclk_rst_n), .uclk2(uclk2),\n",
    ] {
        assert!(top.contains(want), "the AFU's top: `{want}`");
    }
    let json: serde_json::Value = serde_json::from_str(&read(d, "hw/rtl/logos_d5005.json")).unwrap();
    let image = &json["afu-image"];
    let (high, low) = (image["clock-frequency-high"].as_u64().unwrap(), image["clock-frequency-low"].as_u64().unwrap());
    assert_eq!(high, 2 * low, "the pairs' clock twice the user side's: {image}");
    let shell = read(d, "hw/rtl/logos_d5005_shell.v");
    assert!(shell.contains("  input                  uclk2,") && shell.contains(".fclk(uclk), .clk2(uclk2), .frst(frst),"), "the shell gives the engine uclk2");
    let engine = read(d, "hw/rtl/logos_engine.v");
    assert!(engine.contains("module logos_matvec_pass_x2 ") && engine.contains("logos_column_core_bare_"), "the array's pairs");
    assert!(engine.contains("logos_engine_fast fast(.clk(fclk), .clk2(clk2), "), "the engine gives its fast side clk2");
    assert!(engine.contains("unit(.clk(clk), .clk2(clk2), "), "and the fast side its streaming unit");
    let map: serde_json::Value = serde_json::from_str(&read(d, CLOCK_DOMAINS_FILE)).unwrap();
    assert_eq!(map["clocks"], serde_json::json!(["pclk", "uclk", "uclk2", "mem_clk"]));
    assert_eq!(map["pairs"], serde_json::json!(["logos_matvec_pass_x2"]));
    let cosim = read(d, "sim/logos_d5005_cosim.v");
    assert!(cosim.contains("  reg uclk2 = 1;\n  always #(UCLK_PS / 4) uclk2 = ~uclk2;\n") && cosim.contains(".uclk2(uclk2),"), "the cosimulation's uclk2");
    let lone = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    for f in ["hw/rtl/ofs_plat_afu.sv", "hw/rtl/logos_d5005_shell.v", "hw/rtl/logos_engine.v", CLOCK_DOMAINS_FILE, "sim/logos_d5005_cosim.v"] {
        let t = read(lone.path(), f);
        assert!(!t.contains("uclk2") && !t.contains("uClk_usrDiv2") && !t.contains("clk2(") && !t.contains("pass_x2"), "{f} of a build without pairs");
    }
    assert!(read(lone.path(), "hw/rtl/ofs_plat_afu.sv").contains("  wire uclk = plat_ifc.clocks.ports[0].uClk_usr.clk;\n"));
}

/// An engine of paired cores crosses clocks only through its FIFOs and its bridge, its pairs inside the user side's box
/// (`cdc_check.py shell`, the map's `pairs`): the box still one clock's, each pair clocked by the box's clock with its
/// `clk2` the box's `uclk2`; and each pair alone (`cdc_check.py pair`) leaves `clk2` only through `clk`'s registers.
#[test]
fn an_engine_of_paired_cores_crosses_only_through_fifos_its_bridge_and_its_pairs() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(paired_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("40 crossings, 7 clocks, 2 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["pair".as_ref(), "logos_matvec_pass_x2".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_matvec_pass_x2: clean;") && text(&o).contains("every output from clk's"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
}

/// [`paired_08b`] whose cores issue token pairs (`ColumnSet::with_token_pairs`): each core's two token-rows on one set of
/// its processing elements in clk2 (`logos_matvec_pass_x2t`).
fn token_paired_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let p = paired_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| Streaming { set: s.set.with_token_pairs(), ..s }), ..p }
}

/// An engine whose cores issue token pairs on processing elements in clk2 crosses clocks only through its FIFOs, its
/// bridge and its pairs: its clock map names the token-row pairs' elements (`logos_matvec_pass_x2t`) as the user side's
/// pairs, which no core of two cores' shares (`logos_matvec_pass_x2` built nowhere); the box one clock's; each such pair
/// alone (`cdc_check.py pair`) leaves `clk2` only through `clk`'s registers; and the engine elaborates against the PIM.
#[test]
fn an_engine_of_token_pairs_on_paired_elements_crosses_only_through_fifos_its_bridge_and_its_pairs() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(token_paired_08b(), LocalMemoryLine::Data512));
    let map = std::fs::read_to_string(d.path().join(CLOCK_DOMAINS_FILE)).unwrap();
    assert!(map.contains("\"pairs\": [\"logos_matvec_pass_x2t\"]"), "{map}");
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    assert!(v.contains("module logos_matvec_pass_x2t") && !v.contains("module logos_matvec_pass_x2 ") && !v.contains("module logos_matvec_pass_x2("), "the build's clk2 elements are the token-row pairs'");
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;"), "{}", text(&o));
    let o = cdc(&["pair".as_ref(), "logos_matvec_pass_x2t".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_matvec_pass_x2t: clean;") && text(&o).contains("every output from clk's"), "{}", text(&o));
}

/// The pair gate (`cdc_check.py pair`) refuses a pair that lets what it computes in `clk2` out but through `clk`'s
/// registers: an output from a `clk2` register, `clk2` taken as a value, a register on any other clock; and passes the
/// pair that leaves `clk2` through `clk`'s registers.
#[test]
fn the_pair_gate_refuses_a_pair_whose_twice_clock_reaches_out_of_it() {
    if !cdc_tools() {
        return;
    }
    let pair = |body: &str| format!("module logos_matvec_pass_x2(input clk, input clk2, input clk3, input [7:0] w, output [7:0] p);\n{body}\nendmodule\n");
    let cases = [
        ("an output from clk2's register", "  reg [7:0] a, b;\n  always @(posedge clk2) a <= w + 1;\n  always @(posedge clk2) b <= a;\n  assign p = b;", "output p is driven from a clk2 register"),
        ("clk2 as a value", "  reg [7:0] a, b;\n  always @(posedge clk2) a <= w ^ {8{clk2}};\n  always @(posedge clk) b <= a;\n  assign p = b;", "clk2 clocks registers alone"),
        ("a register on another clock", "  reg [7:0] a, b;\n  always @(posedge clk3) a <= w;\n  always @(posedge clk) b <= a;\n  assign p = b;", "not on the rising edge of clk or clk2"),
        ("clk2 out through clk", "  reg [7:0] a, b;\n  always @(posedge clk2) a <= w + 1;\n  always @(posedge clk) b <= a;\n  assign p = b;", ""),
    ];
    for (what, body, refusal) in cases {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("pair.v");
        std::fs::write(&v, pair(body)).unwrap();
        let o = cdc(&["pair".as_ref(), "logos_matvec_pass_x2".as_ref(), v.as_os_str()]);
        if refusal.is_empty() {
            assert_eq!(o.status.code(), Some(0), "{what}: {}", text(&o));
            assert!(text(&o).contains("logos_matvec_pass_x2: clean;"), "{what}: {}", text(&o));
        } else {
            assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
            assert!(text(&o).contains(refusal), "{what}: {}", text(&o));
        }
    }
}

/// An engine of paired cores elaborates against the D5005's PIM, its two user clocks taken as the PIM gives them.
#[test]
fn an_engine_of_paired_cores_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(paired_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// [`fast_side_08b`] with two sequencer contexts sharing its units, sixteen token-rows each.
fn contexts_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = fast_side_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_contexts(2)), ..p }
}

/// [`lanes_08b`] with two sequencer contexts sharing its units.
fn contexts_lanes_08b() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = lanes_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_contexts(2)), ..p }
}

/// Every port of the fast side's channel multiplexer reaches it straight, so the multiplexer takes each port's runs in the
/// order the port drains them: each port's descriptors, write lines, read handshakes and done tokens are, bit for bit, a
/// fast port's own (`logos_fast_port`'s `md_*`, `mr_valid`/`mr_ready`, `mw_*`, `mdone_*`) or a channel's read credit's
/// each (the streaming unit's), in an engine of four head lanes, of two contexts, and of two contexts on four head lanes:
/// one multiplexer, every fast port a port of it, none of its ports another multiplexer. A register a channel between a
/// port and its multiplexer takes the port's descriptor at once and lets it ask its next run of another channel before the
/// first is taken, so two ports' runs on two channels are taken in opposite orders and each port waits on a line behind
/// the other's. A multiplexer as another's port offers lines to several channels at once, where a multiplexer takes a
/// port's lines one at a time in the order given and raises one ready for them all: it lost the lines beside the one
/// taken, and its lines behind a full channel held back the others'.
#[test]
fn every_port_of_the_channel_multiplexer_reaches_it_straight() {
    if !common::require_tool("yosys") {
        return;
    }
    for (what, params) in [("four head lanes", lanes_08b()), ("two contexts", contexts_08b()), ("two contexts on four head lanes", contexts_lanes_08b())] {
        let d = emit(&AfuConfig::engine(params, LocalMemoryLine::Data512));
        let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("d.v"), common::stubbed(&v, &["logos_engine_fast"])).unwrap();
        let o = Command::new("yosys")
            .current_dir(dir.path())
            .args(["-q", "-p", "read_verilog -sv d.v; hierarchy -top logos_engine_fast; proc; write_json s.json"])
            .output()
            .expect("yosys");
        assert!(o.status.success(), "{what}: yosys: {}", String::from_utf8_lossy(&o.stderr));
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
        let fast = json["modules"].as_object().unwrap().values().find(|m| m["attributes"].get("top").is_some()).unwrap();
        let cells: Vec<&serde_json::Value> = fast["cells"].as_object().unwrap().values().collect();
        let of = |kind: &str| -> Vec<&serde_json::Value> { cells.iter().copied().filter(|c| c["type"].as_str().unwrap().contains(kind)).collect() };
        let bits = |cell: &serde_json::Value, port: &str| -> Vec<serde_json::Value> {
            cell["connections"][port].as_array().unwrap_or_else(|| panic!("{what}: a port {port}")).clone()
        };
        let (ports, muxes, credits) = (of("logos_fast_port"), of("logos_channel_mux"), of("logos_read_credit"));
        assert_eq!(muxes.len(), 1, "{what}: one channel multiplexer");
        // A port's side and the multiplexer's side of it, a channel's bits each.
        const SIDES: [(&str, &str); 10] = [
            ("md_valid", "pd_valid"),
            ("md_ready", "pd_ready"),
            ("md_data", "pd_data"),
            ("mr_valid", "pr_valid"),
            ("mr_ready", "pr_ready"),
            ("mw_valid", "pw_valid"),
            ("mw_ready", "pw_ready"),
            ("mw_data", "pw_data"),
            ("mdone_valid", "pdone_valid"),
            ("mdone_ready", "pdone_ready"),
        ];
        let mut reached = vec![false; ports.len()];
        for mux in &muxes {
            let n = bits(mux, "pd_valid").len() / 4;
            for at in 0..n {
                let span = |port: &str| {
                    let all = bits(mux, port);
                    let w = all.len() / n;
                    all[at * w..(at + 1) * w].to_vec()
                };
                if let Some((k, sender)) = ports.iter().copied().enumerate().find(|(_, c)| bits(c, "md_valid") == span("pd_valid")) {
                    for (own, its) in SIDES {
                        assert_eq!(bits(sender, own), span(its), "{what}: the multiplexer's port {at}'s {its} is its sender's {own} straight");
                    }
                    reached[k] = true;
                } else {
                    let valid = span("pd_valid");
                    assert!(
                        (0..4).all(|c| credits.iter().any(|k| bits(k, "md_valid") == vec![valid[c].clone()] && bits(k, "md_ready") == vec![span("pd_ready")[c].clone()])),
                        "{what}: the multiplexer's port {at} is no fast port's and no channel's credits' straight"
                    );
                }
            }
        }
        assert!(reached.iter().all(|&r| r), "{what}: every fast port is the multiplexer's port: {reached:?}");
    }
}

/// An engine of two contexts gives each context a core of its own — started only with token-rows of its own, its
/// rows the crossing's field of them, its host writes those to the shared tables and to its own rows table and bank —
/// and shares the units through the classes' grants: the store a bank a context, the streaming unit on the channels
/// through a credit a channel as the channel mux's second port beside the row class's, the per-row units' mode set and cleared by their own starts
/// alone, the vector side's rows table a context, and the cores' reads of the card one at a time. The shell's
/// bring-up core takes each context's rows and gives the engine the context of a table write or a store read; the
/// host's header names the contexts. One context's build carries none of it.
#[test]
fn an_engine_of_two_contexts_gives_each_context_a_core_and_a_bank_and_shares_the_units() {
    let d = emit(&AfuConfig::engine(contexts_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let top = module_text(&v, "logos_engine");
    for line in [
        "logos_engine_core_ce core0(.clk(fclk), .step_en(fedge), .read_en(cr),",
        "logos_engine_core_ce core1(.clk(fclk), .step_en(fedge), .read_en(cr),",
        ".start(f_start && f_rows[15:0] != 16'd0)",
        ".start(f_start && f_rows[31:16] != 16'd0)",
        "wire host_shared = f_host_table != 3'd1 && f_host_table < 3'd6;",
        "assign c_host_wr[1] = f_host_wr && (host_shared || f_host_addr[31:28] == 4'd1);",
        "wire [31:0] c_host_addr = {4'd0, f_host_addr[27:0]};",
        "assign f_host_rdata = c_host_rdata[512*f_host_raddr[28:28] +: 512];",
        "assign f_pc = {c_pc[47:32], c_pc[15:0]};",
        "wire rd_issue = rd_want != 2'd0 && (!rd_out || f_mem_rd_valid);",
        "assign f_mem_wr = 1'b0;",
    ] {
        assert_eq!(top.matches(line).count(), 1, "{line}: {top}");
    }
    assert_eq!(top.matches("logos_engine_core_ce ").count(), 2, "a core a context");
    let fast = module_text(&v, "logos_engine_fast");
    for line in [
        "input [1:0] own, input [1:0] go,",
        "output [1023:0] rd, output reg [1:0] busy_held,",
        "wire mv_grant = cr && mv_next[1] && !busy && !(mv_valid && own[mv_ctx]);",
        "wire rw_grant = cr && rw_next[1] && !rw_busy && !(rw_valid && own[rw_ctx]);",
        "wire rw_busy = g_busy || writing || v_busy || a_busy;",
        "    else if (g_start || v_start) att <= 1'b0;\n",
        "    else if (v_start || a_start) gdn <= 1'b0;\n",
        "    else if (g_start || a_start) vec <= 1'b0;\n",
        "generate for (bank_at = 0; bank_at < 2; bank_at = bank_at + 1) begin : bank",
        "logos_channel_mux #(.P(2), .NB(4),",
        ".pd_valid({k_md_valid, p_md_valid}), .pd_ready({k_md_ready, x_md_ready}),",
        "logos_vector_side #(.CTX(2), .CB(1), ",
        ".port_fault(g_fault), .fault(v_fault), .ctx(rw_at),",
        ".md_ready(u_md_ready)",
        ".st_rd(u_rd)",
    ] {
        assert_eq!(fast.matches(line).count(), 1, "{line}: {fast}");
    }
    for gone in ["wire start = go && cr;", "assign md_valid = (gdn || vec || att) ? p_md_valid : u_md_valid;", "else if (start || "] {
        assert!(!fast.contains(gone), "the contexts' fast side keeps `{gone}`");
    }
    let side = module_text(&v, "logos_vector_side");
    for line in [
        "reg [127:0] rowt [0:CTX*ROW_MAX-1];",
        "3'd1: rowt[host_addr[31:28] * ROW_MAX + host_addr[27:0]] <= host_wdata[127:0];",
        "input [CB-1:0] ctx, input start,",
    ] {
        assert_eq!(side.matches(line).count(), 1, "{line}");
    }
    assert_eq!(side.matches("row_e <= rowt[ctx * ROW_MAX + k];").count(), 2, "each of the side's reads of its rows from the served context's table");
    let core = std::fs::read_to_string(rtl(d.path(), "logos_bringup_core.v")).unwrap();
    for line in [
        "  output reg [31:0]         eng_rows,",
        "eng_rows <= {cmd_data[96 +: 16], cmd_data[64 +: 16]};",
        "tbase <= {cmd_data[184 +: 4], cmd_data[128 +: 28]};",
        "default: sbase <= {cmd_data[184 +: 4], cmd_data[128 +: 28]};",
    ] {
        assert_eq!(core.matches(line).count(), 1, "{line}");
    }
    let shell = std::fs::read_to_string(rtl(d.path(), "logos_d5005_shell.v")).unwrap();
    assert!(shell.contains("  wire [31:0]   eng_rows;\n"), "the shell's rows a context's field each");
    let header = std::fs::read_to_string(d.path().join("sw/logos_d5005.h")).unwrap();
    assert!(header.contains("#define LOGOS_D5005_CONTEXTS 2u\n"), "{header}");
    let one = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    let one_v = std::fs::read_to_string(rtl(one.path(), "logos_engine.v")).unwrap();
    for gone in ["core0(", "bank_at", "rw_grant", "CTX*ROW_MAX"] {
        assert!(!one_v.contains(gone), "a build of one context carries {gone}");
    }
    assert!(!std::fs::read_to_string(one.path().join("sw/logos_d5005.h")).unwrap().contains("LOGOS_D5005_CONTEXTS"), "one context's header names none");
}

/// Every read of a rows table in an engine of contexts is the table of the context its job runs for: the vector side's
/// at `ctx * ROW_MAX`, and the recurrence's rows (`logos_gdn_rows`) walked from `ctx * ROW_MAX` a row at a time, in
/// engines of two contexts, of two contexts on four head lanes and of two contexts with the conv path, each table
/// written only at the context the host's address names. A read of context 0's table for another context's job reads
/// another sequence's row: a ring load compared against another sequence's region skips the wait for the ring row its
/// own sequence's row before stored.
#[test]
fn an_engine_of_contexts_reads_each_rows_table_at_the_context_its_job_runs_for() {
    let write = "rowt[host_addr[31:28] * ROW_MAX + host_addr[27:0]] <= ";
    for (what, params) in [("two contexts", contexts_08b()), ("two contexts on four head lanes", contexts_lanes_08b()), ("two contexts with the conv path", conv9())] {
        let d = emit(&AfuConfig::engine(params, LocalMemoryLine::Data512));
        let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
        let side = module_text(&v, "logos_vector_side");
        let uses: Vec<&str> = side.match_indices("rowt[").map(|(at, _)| &side[at..]).collect();
        assert!(uses.len() >= 3, "{what}: the vector side writes its rows table and reads it, {} uses", uses.len());
        for u in &uses {
            assert!(u.starts_with(write) || u.starts_with("rowt[ctx * ROW_MAX + "), "{what}: the vector side reads its rows at no context: `{}`", u.lines().next().unwrap());
        }
        assert_eq!(side.matches(write).count(), 1, "{what}: the vector side's rows written at the context the host names");
        assert!(v.contains("module logos_gdn_rows") || what != "two contexts on four head lanes", "{what}: the head lanes walk a job's rows");
        if v.contains("module logos_gdn_rows") {
            let rows = module_text(&v, "logos_gdn_rows");
            for (at, _) in rows.match_indices("rowt[") {
                let u = &rows[at..];
                assert!(u.starts_with(write) || u.starts_with("rowt[r_at]"), "{what}: the recurrence's rows read at `{}`", u.lines().next().unwrap());
            }
            assert_eq!(rows.matches(write).count(), 1, "{what}: the recurrence's rows written at the context the host names");
            let walks: Vec<&str> = rows.match_indices("r_at <= ").map(|(at, _)| rows[at + 8..].split(';').next().unwrap()).collect();
            assert_eq!(walks.iter().filter(|w| **w == "ctx * ROW_MAX").count(), 1, "{what}: a job's rows walked from its context's first: {walks:?}");
            assert!(walks.iter().all(|w| *w == "ctx * ROW_MAX" || *w == "r_at + 1'b1"), "{what}: the walk only steps a row at a time: {walks:?}");
        }
    }
}

/// No register of an engine clears asynchronously. Stratix 10's Hyper-Retimer moves no register with an asynchronous
/// clear, and the pipes between the engine's units (`logos_pipe`) are there to be moved across the die: every process
/// of the engine, as yosys elaborates it, is triggered by its clock alone, its reset read at the clock like any input,
/// in builds of every unit on the fast side, of two contexts on head lanes, of token pairs and of the conv path.
#[test]
fn no_register_of_an_engine_clears_asynchronously() {
    if !common::require_tool("yosys") {
        return;
    }
    for (what, params) in [
        ("every unit on the fast side", fast_side_08b()),
        ("two contexts on head lanes", contexts_lanes_08b()),
        ("token pairs", token_paired_08b()),
        ("the conv path", conv9()),
    ] {
        let d = emit(&AfuConfig::engine(params, LocalMemoryLine::Data512));
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(rtl(d.path(), "logos_engine.v"), dir.path().join("e.v")).unwrap();
        let o = Command::new("yosys")
            .current_dir(dir.path())
            .args(["-q", "-p", "read_verilog -sv e.v; hierarchy -top logos_engine; write_rtlil e.il"])
            .output()
            .expect("yosys");
        assert!(o.status.success(), "{what}: yosys: {}", String::from_utf8_lossy(&o.stderr));
        let il = std::fs::read_to_string(dir.path().join("e.il")).unwrap();
        let (mut module, mut edges, mut processes) = ("", 0, 0);
        let mut cleared = std::collections::BTreeSet::new();
        for line in il.lines() {
            if let Some(m) = line.strip_prefix("module ") {
                module = m;
            } else if line.starts_with("  process ") {
                edges = 0;
                processes += 1;
            } else if line.starts_with("    sync posedge ") || line.starts_with("    sync negedge ") {
                edges += 1;
                if edges == 2 {
                    cleared.insert(module.to_string());
                }
            }
        }
        assert!(processes > 100, "{what}: the engine elaborated to {processes} processes");
        assert!(cleared.is_empty(), "{what}: registers cleared asynchronously in {cleared:?}");
    }
}

/// Every landing's valid (`logos_landing`'s `in_v`) starts from the reset or from what its clock sets it to, never from
/// what its flops power up holding: walked back through its logic in each module that lands a read, every flop it comes
/// from has a reset, or is set every clock (a plain flop, no enable that holds it). A valid held from power-up until its
/// first job fills the landing with marks, so that job's first words are taken while the store still shows another
/// read's: the vector side's store loads (`lbv`) and float-matrix reads (`fm_av`) took a stale beat into a register at a
/// card's power-up, and the second context's first token came out wrong.
#[test]
fn every_landings_valid_starts_from_the_reset_or_its_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let d = emit(&AfuConfig::engine(conv9(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let landers: Vec<String> = v
        .split("\nmodule ")
        .skip(1)
        .filter(|m| m.contains("  logos_landing #(") && !m.starts_with("logos_landing "))
        .map(|m| m.split(|c: char| c == ' ' || c == '(' || c == '#').next().unwrap().to_string())
        .collect();
    assert!(landers.len() >= 4, "the modules that land a read: {landers:?}");
    let mut held = Vec::new();
    for top in &landers {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("d.v"), common::stubbed(&v, &[top.as_str()])).unwrap();
        let o = Command::new("yosys")
            .current_dir(dir.path())
            .args(["-q", "-p", &format!("read_verilog -sv d.v; hierarchy -top {top}; proc; opt_dff; opt_clean; write_json s.json")])
            .output()
            .expect("yosys");
        assert!(o.status.success(), "{top}: yosys: {}", String::from_utf8_lossy(&o.stderr));
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
        let m = &json["modules"][top.as_str()];
        let cells: Vec<(&String, &serde_json::Value)> = m["cells"].as_object().unwrap().iter().collect();
        let mut driver: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for (k, (_, c)) in cells.iter().enumerate() {
            for (port, dir) in c["port_directions"].as_object().into_iter().flatten() {
                if dir == "output" {
                    for b in c["connections"][port].as_array().unwrap() {
                        if let Some(b) = b.as_u64() {
                            driver.insert(b, k);
                        }
                    }
                }
            }
        }
        let flop = |t: &str| t.starts_with("$dff") || t.starts_with("$sdff") || t.starts_with("$adff") || t.starts_with("$aldff") || t.starts_with("$dffsr");
        let reset = |t: &str| t.starts_with("$sdff") || t.starts_with("$adff") || t.starts_with("$aldff") || t.starts_with("$dffsr");
        let name = |c: &serde_json::Value, k: usize| -> String {
            let q = c["connections"]["Q"].as_array().and_then(|q| q.first()).and_then(|b| b.as_u64());
            let net = q.and_then(|b| {
                m["netnames"].as_object().unwrap().iter().find(|(_, n)| n["bits"].as_array().unwrap().iter().any(|x| x.as_u64() == Some(b))).map(|(n, _)| n.clone())
            });
            net.unwrap_or_else(|| cells[k].0.clone())
        };
        for (_, inst) in cells.iter().filter(|(_, c)| c["type"].as_str() == Some("logos_landing")) {
            let mut todo: Vec<u64> = inst["connections"]["in_v"].as_array().unwrap().iter().filter_map(|b| b.as_u64()).collect();
            let mut seen = std::collections::HashSet::new();
            while let Some(b) = todo.pop() {
                if !seen.insert(b) {
                    continue;
                }
                let Some(&k) = driver.get(&b) else { continue };
                let c = cells[k].1;
                let t = c["type"].as_str().unwrap();
                if flop(t) {
                    if !reset(t) && t != "$dff" {
                        held.push(format!("{top}: a landing's valid from {} ({t}), held with no reset", name(c, k)));
                    }
                } else if !t.starts_with("logos_") {
                    for (port, dir) in c["port_directions"].as_object().into_iter().flatten() {
                        if dir == "input" {
                            todo.extend(c["connections"][port].as_array().unwrap().iter().filter_map(|x| x.as_u64()));
                        }
                    }
                }
            }
        }
    }
    held.sort();
    held.dedup();
    assert!(held.is_empty(), "landings' valids that power up held:\n{}", held.join("\n"));
}

/// The column array's activation copies are addressed by a token-row's place and a block's in bits of their own (each
/// token-row's blocks at a power-of-two pitch, `NB_P`): no multiplier, adder or subtractor lies in the cone of any copy's
/// read address (`ach[*].ra`) or write address (`ach[*].wa`) back to the flops it comes from, in the card's hybrid9 array
/// and in one of token pairs. The fitted hybrid9h's critical chain ran from the step's end through `t * NB_MAX + hb` into
/// the copies' address registers.
#[test]
fn the_column_arrays_activation_copies_are_addressed_by_a_token_rows_place_and_a_blocks_alone() {
    if !common::require_tool("yosys") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let pairs = {
        let p = conv9();
        EngineParams { streaming: p.streaming.clone().map(|s| logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming { set: s.set.clone().with_token_pairs(), ..s }), ..p }
    };
    let mut wrong = Vec::new();
    for (what, params) in [("hybrid9", conv9()), ("token pairs", pairs)] {
        let d = emit(&AfuConfig::engine(params, LocalMemoryLine::Data512));
        let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
        let array = v.split("\nmodule ").skip(1).find(|m| m.starts_with("logos_column_array_")).map(|m| m.split(|c: char| c == ' ' || c == '#' || c == '(').next().unwrap().to_string()).expect("the array");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("d.v"), common::stubbed(&v, &[array.as_str()])).unwrap();
        let o = Command::new("yosys")
            .current_dir(dir.path())
            .args(["-q", "-p", &format!("read_verilog -sv d.v; hierarchy -top {array}; proc; opt_dff; opt_clean; write_json s.json")])
            .output()
            .expect("yosys");
        assert!(o.status.success(), "{what}: yosys: {}", String::from_utf8_lossy(&o.stderr));
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
        let m = &json["modules"][array.as_str()];
        let cells: Vec<&serde_json::Value> = m["cells"].as_object().unwrap().values().collect();
        let mut driver = std::collections::HashMap::new();
        for (k, c) in cells.iter().enumerate() {
            for (port, dir) in c["port_directions"].as_object().into_iter().flatten() {
                if dir == "output" {
                    for b in c["connections"][port].as_array().unwrap().iter().filter_map(|b| b.as_u64()) {
                        driver.insert(b, k);
                    }
                }
            }
        }
        let flop = |t: &str| t.starts_with("$dff") || t.starts_with("$sdff") || t.starts_with("$adff") || t.starts_with("$aldff");
        let arith = |t: &str| ["$mul", "$add", "$sub", "$alu", "$macc", "$neg"].contains(&t);
        let mut regs = 0;
        for (net, n) in m["netnames"].as_object().unwrap() {
            let reg = net.rsplit('.').next().unwrap();
            if !(net.contains("ach[") && (reg == "ra" || reg == "wa")) {
                continue;
            }
            let bits: Vec<u64> = n["bits"].as_array().unwrap().iter().filter_map(|b| b.as_u64()).collect();
            let Some(&k) = bits.first().and_then(|b| driver.get(b)) else { continue };
            if !flop(cells[k]["type"].as_str().unwrap()) {
                continue;
            }
            regs += 1;
            let mut todo: Vec<u64> = cells[k]["connections"]["D"].as_array().unwrap().iter().filter_map(|b| b.as_u64()).collect();
            let mut seen = std::collections::HashSet::new();
            while let Some(b) = todo.pop() {
                if !seen.insert(b) {
                    continue;
                }
                let Some(&c) = driver.get(&b) else { continue };
                let t = cells[c]["type"].as_str().unwrap();
                if flop(t) || t.starts_with("logos_") {
                    continue;
                }
                if arith(t) {
                    wrong.push(format!("{what}: {net}'s address passes a {t}"));
                    break;
                }
                for (port, dir) in cells[c]["port_directions"].as_object().into_iter().flatten() {
                    if dir == "input" {
                        todo.extend(cells[c]["connections"][port].as_array().unwrap().iter().filter_map(|x| x.as_u64()));
                    }
                }
            }
        }
        assert!(regs >= 2, "{what}: the copies' address registers: {regs}");
    }
    wrong.sort();
    wrong.dedup();
    assert!(wrong.is_empty(), "activation copies' addresses through arithmetic:\n{}", wrong.join("\n"));
}

/// An engine of contexts streams its weights to the channel mux through a credit a channel
/// (`physical::read_credit_verilog`): the streaming unit's descriptors of channel c go into credit c, credit c's runs
/// are the mux's second port's on channel c and every line the mux gives that port is the credit's, and the unit takes
/// channel c's lines, valid and ready, from credit c through a pipe (`logos_pipe`). The unit takes a channel's lines only
/// as its other channels' feeds let it, which straight on the mux beside the row class's port deadlocks it
/// (`d5005_card_port_lock`).
#[test]
fn an_engine_of_contexts_streams_its_weights_to_the_channels_through_a_credit_a_channel() {
    if !common::require_tool("yosys") {
        return;
    }
    let d = emit(&AfuConfig::engine(contexts_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d.v"), common::stubbed(&v, &["logos_engine_fast"])).unwrap();
    let o = Command::new("yosys")
        .current_dir(dir.path())
        .args(["-q", "-p", "read_verilog -sv d.v; hierarchy -top logos_engine_fast; proc; write_json s.json"])
        .output()
        .expect("yosys");
    assert!(o.status.success(), "yosys: {}", String::from_utf8_lossy(&o.stderr));
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("s.json")).unwrap()).unwrap();
    let fast = json["modules"].as_object().unwrap().values().find(|m| m["attributes"].get("top").is_some()).unwrap();
    let cells: Vec<&serde_json::Value> = fast["cells"].as_object().unwrap().values().collect();
    let of = |kind: &str| -> Vec<&serde_json::Value> { cells.iter().copied().filter(|c| c["type"].as_str().unwrap().contains(kind)).collect() };
    let (units, muxes, credits, pipes) = (of("logos_streaming_matvec_"), of("logos_channel_mux"), of("logos_read_credit"), of("logos_pipe"));
    assert_eq!((units.len(), muxes.len(), credits.len(), pipes.len()), (1, 1, 4, 4), "one streaming unit, one channel mux, a credit and a pipe a channel");
    let (unit, mux) = (units[0], muxes[0]);
    let bits = |cell: &serde_json::Value, port: &str| -> Vec<serde_json::Value> {
        cell["connections"][port].as_array().unwrap_or_else(|| panic!("a port {port}")).clone()
    };
    let span = |cell: &serde_json::Value, port: &str, at: usize, width: usize| bits(cell, port)[at * width..(at + 1) * width].to_vec();
    let line = 8 * 64;
    let desc = bits(unit, "md_data").len() / 4;
    let channels = fast["ports"]["mr_data"]["bits"].as_array().unwrap().clone();
    let mut seen = [false; 4];
    for k in &credits {
        let c = (0..4).find(|&c| span(unit, "md_valid", c, 1) == bits(k, "in_valid")).expect("each credit takes a channel's descriptors from the unit");
        seen[c] = true;
        let pipe = pipes
            .iter()
            .copied()
            .find(|p| bits(p, "in_valid") == bits(k, "out_valid"))
            .unwrap_or_else(|| panic!("channel {c}'s credit's lines go into a pipe"));
        for (from, to) in [("in_ready", "out_ready"), ("in_data", "out_data")] {
            assert_eq!(bits(pipe, from), bits(k, to), "channel {c}'s pipe takes its credit's {to}");
        }
        for (port, unit_port, width) in [("out_valid", "mr_valid", 1), ("out_ready", "mr_ready", 1), ("out_data", "mr_data", line)] {
            assert_eq!(bits(pipe, port), span(unit, unit_port, c, width), "channel {c}'s pipe's {port} is the unit's {unit_port} of channel {c}");
        }
        for (credit, cell, port, at, width) in [
            ("in_ready", unit, "md_ready", c, 1),
            ("in_data", unit, "md_data", c, desc),
            ("md_valid", mux, "pd_valid", 4 + c, 1),
            ("md_ready", mux, "pd_ready", 4 + c, 1),
            ("md_data", mux, "pd_data", 4 + c, desc),
            ("mr_valid", mux, "pr_valid", 4 + c, 1),
            ("mr_ready", mux, "pr_ready", 4 + c, 1),
        ] {
            assert_eq!(bits(k, credit), span(cell, port, at, width), "channel {c}'s credit's {credit} is the {port} of channel {c}");
        }
        assert_eq!(bits(k, "mr_data"), channels[c * line..(c + 1) * line].to_vec(), "channel {c}'s credit reads channel {c}'s lines");
    }
    assert_eq!(seen, [true; 4], "a credit for every channel");
    let one = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    assert!(!std::fs::read_to_string(rtl(one.path(), "logos_engine.v")).unwrap().contains("logos_read_credit"), "one context's unit has the channels to itself");
}

/// An engine of two contexts crosses clocks exactly as one context's does: the cores, the grants and the banks live in
/// the user-clock boxes and the crossing carries each context's rows in one wider value; the boundary proves its own
/// structure.
#[test]
fn an_engine_of_two_contexts_crosses_only_through_fifos_and_its_bridge() {
    if !cdc_tools() || !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(contexts_08b(), LocalMemoryLine::Data512));
    let mut args: Vec<std::ffi::OsString> = vec!["shell".into(), d.path().join(CLOCK_DOMAINS_FILE).into()];
    args.extend(shell_files(d.path()).into_iter().map(|p| p.into_os_string()));
    for f in ["logos_card_port.v", "logos_engine.v"] {
        args.push(rtl(d.path(), f).into_os_string());
    }
    let o = cdc_boxed(&args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;") && text(&o).contains("40 crossings, 6 clocks, 3 single-clock boxes"), "{}", text(&o));
    let o = cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), rtl(d.path(), "logos_engine.v").as_os_str()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_engine_bridge: clean;"), "{}", text(&o));
}

/// An engine of two contexts elaborates against the D5005's PIM with no problem the front end reports, warnings
/// included.
#[test]
fn an_engine_of_two_contexts_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(contexts_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A sixteen-row hybrid build with every unit on the fast side and the recurrence unit at a line a clock — hybrid6b's
/// shape — elaborates against the D5005's PIM with no problem the front end reports, warnings included.
#[test]
fn a_sixteen_row_hybrid_build_with_every_unit_on_the_fast_side_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A hybrid build whose core reads the token-row store for the host alone (every unit on the fast side, on one side or
/// head lanes, one context or two, hybrid9) reads it through the pipelined bank: its fast side's store is
/// `logos_store_w512` and each reader of it — the streaming unit, the recurrence, the vector and attention sides — is
/// given the bank's latency (`SRL`). A build whose core reads the store itself (a matvec alone on the fast side, the
/// recurrence beside it, the vector jobs beside it with attention on the core) keeps the bank of one clock and gives no
/// reader a latency.
#[test]
fn a_build_whose_core_reads_the_store_for_the_host_alone_reads_it_through_the_pipelined_bank() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::engine_verilog;
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::STORE_READ_LATENCY;
    let given = format!(".SRL({STORE_READ_LATENCY})");
    // An instance's parameters: the text of its `#(...)`.
    let params = |text: &str, from: &str| -> Option<String> {
        let at = text.find(from)?;
        let open = at + text[at..].find("#(")? + 1;
        let mut depth = 0;
        let close = text[open..].char_indices().find(|&(_, c)| {
            depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            depth == 0
        })?;
        Some(text[open..open + close.0 + 1].to_string())
    };
    for (what, p) in [("every unit on the fast side", fast_side_08b()), ("head lanes", lanes_08b()), ("two contexts", contexts_lanes_08b()), ("hybrid9", hybrid9())] {
        let v = engine_verilog(&p);
        let fast = module_text(&v, "logos_engine_fast");
        assert!(fast.contains("logos_store_w512 #(") && !fast.contains("logos_bram_re_w512"), "{what}: the pipelined bank\n{fast}");
        assert!(v.contains("module logos_store_w512 "), "{what}: the pipelined bank's module");
        let at = |from: &str| params(&fast, from).is_some_and(|ps| ps.contains(&given));
        assert!(at("  logos_streaming_matvec_"), "{what}: the streaming unit at the bank's latency");
        for reader in ["logos_vector_side", "logos_attend_side"] {
            assert!(at(&format!("  {reader} #(")), "{what}: {reader} at the bank's latency");
        }
        assert!(at("  logos_gdn_side #(") || at("  logos_gdn_lanes #("), "{what}: the recurrence at the bank's latency");
        assert_eq!(fast.matches(".SRL(").count(), 4, "{what}: four readers given a latency");
    }
    for (what, p) in [("a matvec", hybrid_08b()), ("a recurrence beside it", recurrent_08b()), ("vector jobs beside it", vectored_08b())] {
        let v = engine_verilog(&p);
        let fast = module_text(&v, "logos_engine_fast");
        assert!(fast.contains("logos_bram_re_w512 #(") && !fast.contains("logos_store_w512"), "{what}: the bank of one clock\n{fast}");
        assert!(!fast.contains(".SRL("), "{what}: no reader given a latency");
        assert!(!v.contains("module logos_store_w512 "), "{what}: no pipelined bank");
    }
}

/// [`a_sixteen_row_hybrid_build_with_every_unit_on_the_fast_side_elaborates_against_the_d5005_pim`] with its streaming
/// unit folding a job's sums into its output slot, over four lanes and over sixteen: the fold path and its lanes
/// elaborate with no problem the front end reports.
#[test]
fn a_folding_hybrid_build_elaborates_against_the_d5005_pim() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    if !common::require_tool("d5005-pim") {
        return;
    }
    for lanes in [4, 16] {
        let p = fast_side_08b();
        let p = EngineParams { streaming: p.streaming.clone().map(|s| s.with_folds(lanes)), folds: true, ..p };
        let d = emit(&AfuConfig::engine(p, LocalMemoryLine::Data512));
        let o = sv_check(d.path());
        assert_eq!(o.status.code(), Some(0), "{lanes} fold lanes: {}", text(&o));
        assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{lanes} fold lanes: {}", text(&o));
    }
}

/// [`a_sixteen_row_hybrid_build_with_every_unit_on_the_fast_side_elaborates_against_the_d5005_pim`] packed as the card's
/// slotted builds are: Q8_0 in octets and every block in a slot of 16 bytes.
#[test]
fn a_slotted_hybrid_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(fast_side_08b_packed(true, 16), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A slotted build names its slot where a program names the set it was written for (`LOGOS_D5005_STREAMING`, which
/// the host holds a program's `engine.txt` to), so a program laid in packed blocks never runs on it, nor one laid in
/// slots on a packed build; and its feeds gather a beat a slot at a time, where a packed build's gather it a byte at a
/// time.
#[test]
fn a_slotted_build_names_its_slots_and_gathers_a_slot_at_a_time() {
    let slotted = emit(&AfuConfig::engine(fast_side_08b_packed(true, 16), LocalMemoryLine::Data512));
    let packed = emit(&AfuConfig::engine(fast_side_08b_packed(true, 1), LocalMemoryLine::Data512));
    let header = |d: &tempfile::TempDir| std::fs::read_to_string(d.path().join("sw/logos_d5005.h")).unwrap();
    assert!(header(&slotted).contains("#define LOGOS_D5005_STREAMING \"q8_0x8:1@16/4\""), "{}", header(&slotted));
    assert!(header(&packed).contains("#define LOGOS_D5005_STREAMING \"q8_0x8:1/4\""), "{}", header(&packed));
    let feed = |d: &tempfile::TempDir| module_text(&std::fs::read_to_string(d.path().join("hw/rtl/logos_engine.v")).unwrap(), "logos_column_feed_step");
    assert!(feed(&slotted).contains("in_data} << (128 * kept[15:4]))"), "{}", feed(&slotted));
    assert!(feed(&packed).contains("in_data} << (8 * kept))"), "{}", feed(&packed));
}

/// A floating-point DSP block for the locks: a stand-in whose multiply and add compute `semantics`, `full` steps deep
/// fully registered and one with its output register alone.
fn dsp_block(semantics: logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics, full: usize) -> logicaffeine_compile::codegen_sva::fp_hard::Block {
    logicaffeine_compile::codegen_sva::fp_hard::Block { semantics, full, least: 1 }
}

/// [`fast_side_08b`] with its fast side's f32 multiply and add on the floating-point DSP block `hard`.
fn hard_float_08b(hard: logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat) -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = fast_side_08b();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_hard_float(hard).expect("a block the wrappers serve")), ..p }
}

/// A build whose fast side's f32 units are the floating-point DSP block's builds `logos_fmul_p` and `logos_fadd_p` as
/// the block's wrappers — `fourteennm_fp_mac` under the soft unit's ports, none of the soft unit's logic, one definition
/// each — corrects what an IEEE 754 block computes otherwise (denormal operands and results flushed, every NaN the
/// engine's), and carries the device's block for simulation alone: the stand-in in `sim/`, never among the hardware's
/// sources, which take the device's own.
#[test]
fn a_hard_float_build_builds_its_f32_units_on_the_dsp_block_and_simulates_the_block_by_its_stand_in() {
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat;
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    let hard = HardFloat { product: Some(dsp_block(Semantics::IEEE, FMUL_LATENCY)), sum: dsp_block(Semantics::IEEE, FADD_LATENCY + 1), fp_unit: FpUnit::Soft };
    let d = emit(&AfuConfig::engine(hard_float_08b(hard), LocalMemoryLine::Data512));
    let hw = std::fs::read_to_string(d.path().join("hw/rtl/logos_engine.v")).unwrap();
    for unit in ["logos_fmul_p", "logos_fadd_p"] {
        assert_eq!(hw.matches(&format!("module {unit}(")).count(), 1, "{unit} defined once");
        let m = module_text(&hw, unit);
        assert!(m.contains("fourteennm_fp_mac"), "{unit} on the block: {m}");
        assert!(!m.contains("sticky"), "{unit} keeps the soft unit's rounding: {m}");
        let doc = module_doc(&hw, unit);
        for correction in ["a denormal operand read as a zero of its sign", "a denormal result written as a zero of its sign", "every NaN result 0x7fc00000"] {
            assert!(doc.contains(correction), "{unit} corrects an IEEE block: {correction}: {doc}\n{m}");
        }
        assert!(m.contains("nan ? 32'h7fc0_0000 : (h[30:23] == 8'd0) ? {h[31], 31'd0} : h"), "{unit} gives the engine's NaN and flushes a denormal result: {m}");
    }
    assert!(module_text(&hw, "logos_fmul_p").contains("side[0] <= side_c;"), "a multiply decides a denormal operand beside the block");
    assert!(module_text(&hw, "logos_fadd_p").contains("((a[30:23] == 8'd0) ? {a[31], 31'd0} : a)"), "an add flushes a denormal operand before the block");
    assert!(module_doc(&hw, "logos_fadd_p").contains("with its output register alone"), "an add deeper than its latency registers its output alone");
    assert!(!hw.contains("module fourteennm_fp_mac"), "the hardware's block is the device's own");
    let model = std::fs::read_to_string(d.path().join("sim/logos_fp_mac_model.v")).unwrap();
    assert!(model.contains("module fourteennm_fp_mac"), "{model}");
    let sources = std::fs::read_to_string(d.path().join("hw/rtl/sources.txt")).unwrap();
    assert!(!sources.contains("logos_fp_mac_model"), "{sources}");
    let soft = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    assert!(!soft.path().join("sim/logos_fp_mac_model.v").exists(), "a soft build carries no block");
}

/// A build with its adds on the floating-point DSP block and its multiplies soft builds `logos_fadd_p` as the block's
/// wrapper and keeps `logos_fmul_p` the soft unit, its text the soft build's to the byte, and carries the block's
/// stand-in for its simulations; and it elaborates against the D5005's PIM.
#[test]
fn an_adds_only_hard_float_build_keeps_its_multiplies_soft() {
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat;
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let hard = HardFloat { product: None, sum: dsp_block(Semantics::IEEE, FADD_LATENCY + 1), fp_unit: FpUnit::Soft };
    let d = emit(&AfuConfig::engine(hard_float_08b(hard), LocalMemoryLine::Data512));
    let soft = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    let hw = std::fs::read_to_string(d.path().join("hw/rtl/logos_engine.v")).unwrap();
    let soft_hw = std::fs::read_to_string(soft.path().join("hw/rtl/logos_engine.v")).unwrap();
    assert!(module_text(&hw, "logos_fadd_p").contains("fourteennm_fp_mac"), "the add on the block");
    assert_eq!(module_text(&hw, "logos_fmul_p"), module_text(&soft_hw, "logos_fmul_p"), "the multiply the soft unit");
    assert!(d.path().join("sim/logos_fp_mac_model.v").exists(), "the block's stand-in for the add");
    if common::require_tool("d5005-pim") {
        let o = sv_check(d.path());
        assert_eq!(o.status.code(), Some(0), "{}", text(&o));
        assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
    }
}

/// A hard-float build of hybrid6b's shape elaborates against the D5005's PIM, the block a primitive Quartus supplies.
#[test]
fn a_hard_float_build_elaborates_against_the_d5005_pim() {
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat;
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    if !common::require_tool("d5005-pim") {
        return;
    }
    let hard = HardFloat { product: Some(dsp_block(Semantics::ENGINE, FMUL_LATENCY)), sum: dsp_block(Semantics::ENGINE, FADD_LATENCY), fp_unit: FpUnit::Soft };
    let d = emit(&AfuConfig::engine(hard_float_08b(hard), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A build of the card's products (`HardFloat { fp_unit: FpUnit::Hard, .. }`) plans its multiplies for the card's
/// arithmetic ([`FpUnit::CARD`], the one class fp-oracle6's receipt names), not the engine's: the card's block, which
/// decides a product tiny before the engine does and so is refused as the engine's multiply, is taken as the card's
/// own, its wrapper the block's with nothing corrected (`fp_hard::hard_verilog_for` at the card's arithmetic); the add
/// stays the engine's on the block; and a build of the card's products with no block for them is no build.
#[test]
fn a_build_of_the_cards_products_plans_its_multiplies_for_the_cards_arithmetic() {
    use logicaffeine_compile::codegen_sva::fp_hard::{hard_verilog_for, Block, HardError, Op};
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat;
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    let card = Block { semantics: FpUnit::CARD, full: 4, least: 1 };
    let sum = dsp_block(Semantics::ENGINE, FADD_LATENCY);
    let s = fast_side_08b().streaming.unwrap();
    assert!(
        matches!(s.clone().with_hard_float(HardFloat { product: Some(card), sum, fp_unit: FpUnit::Soft }), Err(HardError::Semantics { .. })),
        "the card's multiply is not the engine's"
    );
    let hard = HardFloat { product: Some(card), sum, fp_unit: FpUnit::Hard };
    assert!(s.clone().with_hard_float(hard).is_ok(), "the card's multiply is the card's");
    let d = emit(&AfuConfig::engine(hard_float_08b(hard), LocalMemoryLine::Data512));
    let hw = std::fs::read_to_string(d.path().join("hw/rtl/logos_engine.v")).unwrap();
    let want = hard_verilog_for(Op::Multiply, "logos_fmul_p", &card, FMUL_LATENCY, &FpUnit::CARD).unwrap();
    assert_eq!(module_text(&hw, "logos_fmul_p"), module_text(&want, "logos_fmul_p"), "the block's own multiply, nothing corrected");
    assert!(module_text(&hw, "logos_fadd_p").contains("fourteennm_fp_mac"), "the add on the block");
    let no_block = std::panic::catch_unwind(|| s.clone().with_hard_float(HardFloat { product: None, sum, fp_unit: FpUnit::Hard }));
    assert!(no_block.is_err(), "the card's products need its block");
}

/// A block no wrapper can make the engine's unit is refused by name before anything is built: one deeper than the
/// unit's latency even with its output register alone, and one that flushes a tiny result before rounding it or to +0.
#[test]
fn a_dsp_block_the_wrappers_cannot_serve_is_refused() {
    use logicaffeine_compile::codegen_sva::fp_hard::HardError;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::{FlushSign, Semantics, Tininess};
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::HardFloat;
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};
    let s = fast_side_08b().streaming.unwrap();
    let fine = dsp_block(Semantics::ENGINE, FADD_LATENCY);
    let deep = logicaffeine_compile::codegen_sva::fp_hard::Block { semantics: Semantics::ENGINE, full: FMUL_LATENCY + 2, least: FMUL_LATENCY + 1 };
    assert!(matches!(s.clone().with_hard_float(HardFloat { product: Some(deep), sum: fine, fp_unit: FpUnit::Soft }), Err(HardError::TooDeep { .. })));
    let early = Semantics { tininess: Tininess::BeforeRounding, ..Semantics::ENGINE };
    assert!(matches!(s.clone().with_hard_float(HardFloat { product: Some(dsp_block(Semantics::ENGINE, 1)), sum: dsp_block(early, 1), fp_unit: FpUnit::Soft }), Err(HardError::Semantics { .. })));
    let positive = Semantics { flushed_sign: FlushSign::Positive, ..Semantics::ENGINE };
    assert!(matches!(s.clone().with_hard_float(HardFloat { product: Some(dsp_block(positive, 1)), sum: fine, fp_unit: FpUnit::Soft }), Err(HardError::Semantics { .. })));
    assert!(s.with_hard_float(HardFloat { product: Some(dsp_block(Semantics::ENGINE, FMUL_LATENCY)), sum: fine, fp_unit: FpUnit::Soft }).is_ok());
}

/// A vectored build whose memories flush lazily hands every vector job and every recurrence row to the fast side, so
/// its core builds neither the vector unit nor the row engines, nor their ports, registers and states: the core reaches
/// only what it keeps. A recurrent build without the vector side runs its vector jobs on the core and keeps the unit.
#[test]
fn a_vectored_lazy_build_keeps_no_vector_unit_or_row_engine_in_its_core() {
    let d = emit(&AfuConfig::engine(vectored_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let core = module_text(&v, "logos_engine_core_ce");
    for gone in [
        "logos_simd_ce", "logos_gdn_row_engine_ce", "logos_gdn_lazy_row", "s_issue", "s_rd_reg", "s_wr_data", "s_busy", "dl_kv",
        "dl_qv", "dl_vals", "state <= HEADS;", "state <= DL_KQ_K;", "state <= DL_E_RD;",
    ] {
        assert!(!core.contains(gone), "the vectored lazy core keeps {gone}:\n{core}");
    }
    for kept in ["DL_SG: if (wq_empty) begin", "V_SG: if (wq_empty) begin", "rq_v <= 1'b0; wq_v <= 1'b0;"] {
        assert_eq!(core.matches(kept).count(), 1, "{kept}");
    }
    for module in ["logos_simd_ce", "logos_gdn_row_engine_ce", "logos_gdn_lazy_row_ce"] {
        assert!(!v.contains(&format!("module {module}")), "{module} is built though nothing reaches it");
    }
    let d = emit(&AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let core = module_text(&v, "logos_engine_core_ce");
    assert!(core.contains("logos_simd_ce #(") && core.contains("state <= HEADS;"), "a recurrent core runs its own vector jobs");
    assert_eq!(v.matches("module logos_simd_ce").count(), 1);
}

/// The crossing gate refuses each defect an engine's bridge could have: a slow-side value taken through logic, at the
/// wrong mark or into a register of no capture's name, a slow-side output not straight from a held register or held at
/// the wrong mark, a mark taken from the wrong edge, and a register on the compute clock.
#[test]
fn the_bridge_gate_refuses_each_defect() {
    if !cdc_tools() {
        return;
    }
    let d = emit(&AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512));
    let bridge = module_text(&std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap(), "logos_engine_bridge");
    let dir = tempfile::tempdir().unwrap();
    let run = |text: &str| {
        let f = dir.path().join("bridge.v");
        std::fs::write(&f, text).unwrap();
        cdc(&["related".as_ref(), "logos_engine_bridge".as_ref(), f.as_os_str()])
    };
    let o = run(&bridge);
    assert_eq!(o.status.code(), Some(0), "the bridge alone: {}", text(&o));
    for (from, to, says) in [
        ("    if (emark2) cap_start <= s_start;", "    if (emark2) cap_start <= !s_start;", "cap_start: a bit not straight from a slow-side input"),
        ("    if (emark2) cap_job_at <= s_job_at;", "    if (mark2) cap_job_at <= s_job_at;", "cap_job_at: not enabled exactly by emark2"),
        ("    if (mark2) held_busy <= f_busy;", "    if (mark2) held_busy <= s_start;", "held_busy reads the slow side outside a cap_ register"),
        ("    if (mark2) held_pc <= f_pc;", "    if (mark1) held_pc <= f_pc;", "held_pc: not enabled exactly by mark2"),
        ("  assign s_busy = held_busy;", "  assign s_busy = held_busy || f_busy;", "output s_busy not straight from a held_ register"),
        ("    mark1 <= fmid;", "    mark1 <= fedge;", "mark1 does not take fmid every user cycle"),
        ("    emark1 <= fedge;", "    emark1 <= fmid;", "emark1 does not take fedge every user cycle"),
        ("  assign f_start = cap_start;", "  reg slow_q;\n  always @(posedge sclk) slow_q <= s_start;\n  assign f_start = cap_start ^ slow_q;", "sclk reaches a cell"),
    ] {
        assert_eq!(bridge.matches(from).count(), 1, "the mutation site {from:?}");
        let o = run(&bridge.replace(from, to));
        assert_eq!(o.status.code(), Some(1), "{to:?} is refused: {}", text(&o));
        assert!(text(&o).contains(says), "{to:?}: {}", text(&o));
    }
}

/// An engine build carries its engine's simulation twin beside the engine (`sim/logos_engine_gated.v`, the engine with
/// its core's clock gated to the user cycles the core acts in), the hybrid build as the reference build, and
/// `scripts/d5005/run-cosim.sh` simulates the twin; a build with no engine carries none.
#[test]
fn an_engine_build_carries_its_engines_simulation_twin() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::simulation_twin;
    for p in [engine_08b(), hybrid_08b()] {
        let d = emit(&AfuConfig::engine(p, LocalMemoryLine::Data512));
        let read = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap();
        assert_eq!(read("sim/logos_engine_gated.v"), simulation_twin(&read("hw/rtl/logos_engine.v")));
    }
    assert!(!emit(&AfuConfig::d5005()).path().join("sim/logos_engine_gated.v").exists());
    let cosim = std::fs::read_to_string(script("run-cosim.sh")).unwrap();
    assert!(cosim.contains("ENGINE+=(\"$AFU/sim/logos_engine_gated.v\")"), "run-cosim.sh simulates the twin");
}

/// The engine's timing marks are where the SDC's multicycles assume them, a property no simulation sees (a mark a cycle
/// off computes the same values and breaks timing): the core's memories read the user cycle after the compute clock's
/// falling edge (`cr`, `fmid` a register on) and its registers step at the rising edge (`fedge`); every clocked block
/// of the core steps under `step_en` and every memory reads under `read_en`; a hybrid engine's unit starts the cycle
/// after the falling edge its job is given in, its busy held only then, and the store the core's on its read and step.
#[test]
fn the_engines_timing_marks_are_where_its_multicycles_assume_them() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::engine_verilog;
    for p in [engine_08b(), hybrid_08b()] {
        let v = engine_verilog(&p);
        let top = module_text(&v, "logos_engine");
        assert!(top.contains("(* maxfan = 2000 *) reg cr;") && top.contains("always @(posedge fclk) cr <= fmid;"), "{top}");
        assert!(top.contains("logos_engine_core_ce core(.clk(fclk), .step_en(fedge), .read_en(cr),"), "{top}");
        let core = module_text(&v, "logos_engine_core_ce");
        assert!(!core.contains("always @(posedge clk) begin") && core.contains("always @(posedge clk) if (step_en) begin"), "the core's blocks");
        assert!(!v.contains("always @(negedge"), "no falling-edge memory left");
        for ram in v.match_indices("module logos_ram_ce_w").map(|(i, _)| &v[i..i + v[i..].find("endmodule").unwrap()]) {
            assert!(ram.contains("always @(posedge clk) if (cr) rd <= mem[ra];") && ram.contains("if (ce && we) begin"), "{ram}");
        }
        if p.streaming.is_some() {
            let fast = module_text(&v, "logos_engine_fast");
            for line in [
                "wire start = go && cr;",
                "else if (cr) busy_held <= busy || start;",
                ".re(own || cr), .ra(own ? u_ra : e_ra)",
                ".we(own ? u_we : ce && e_we)",
            ] {
                assert!(fast.contains(line), "{line}: {fast}");
            }
            assert!(top.contains("logos_engine_fast fast(.clk(fclk), .rst(frst), .ce(fedge), .cr(cr),"), "{top}");
        }
    }
}

/// An engine build's files: the clock-domain map gives the platform's three clocks their ports, declares the engine's
/// step boundary beside the FIFOs and boxes the engine's core (and a hybrid engine's user-clock side); the shell hands
/// the engine the user clock, the compute clock's marks of the user cycles that end with its edges, the compute side's
/// reset and the user clock's reset two registers on for its user-clock side, and a hybrid build's shell gives every
/// channel a fast port in the user clock; the SDC generates no
/// clock, times the held values into the shell at half a compute cycle less two, the core, its user-clock side and the
/// captures into them at half a compute cycle less one, and the shell into the captures in one user cycle; the C header
/// names the streaming unit the build's programs must be written for, none for an engine that streams nothing. An
/// engine build with its compute clock under six user cycles, or a hybrid one on SECDED lines, is refused.
#[test]
fn a_hybrid_build_carries_its_fast_ports_and_its_step_boundary() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::ENGINE_MIN_DIVIDE;
    let p = hybrid_08b();
    let signature = format!("{}/1", p.streaming.as_ref().unwrap().set.signature());
    let d = emit(&AfuConfig::engine(p.clone(), LocalMemoryLine::Data512));
    let read = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap();
    let map: serde_json::Value = serde_json::from_str(&read(CLOCK_DOMAINS_FILE)).unwrap();
    assert_eq!(map["clocks"], serde_json::json!(["pclk", "uclk", "mem_clk"]));
    assert_eq!(map["crossings"], serde_json::json!(["logos_dcfifo_mlab", "logos_dcfifo_m20k", "logos_engine_bridge"]));
    assert_eq!(map["single_clock"], serde_json::json!(["logos_engine_core_ce", "logos_engine_fast"]));
    let ports: Vec<&str> = map["ports"].as_array().unwrap().iter().map(|r| r["port"].as_str().unwrap()).collect();
    assert!(!ports.iter().any(|p| p.starts_with("fclk") || ["fedge", "fmid"].contains(p)), "{ports:?}");
    let afu = read("hw/rtl/ofs_plat_afu.sv");
    assert!(!afu.contains("fclk") && !afu.contains("fedge"), "{afu}");
    let shell = read("hw/rtl/logos_d5005_shell.v");
    assert_eq!(shell.matches("fast_desc_fifo(").count(), 1);
    assert_eq!(shell.matches("fast_read_fifo(").count(), 1);
    assert!(shell.contains(".sm_md_valid(f_md_valid)") && shell.contains(".fd_valid(f_md_valid[b])"), "{shell}");
    let engine = &shell[shell.find("  logos_engine engine(").expect("the engine")..];
    for (port, net) in [("clk", "uclk"), ("rst", "!crst_n"), ("fclk", "uclk"), ("frst", "frst"), ("fedge", "cce"), ("fmid", "fmid")] {
        assert_eq!(connection(engine, port), net, "the engine's {port}");
    }
    assert!(
        shell.contains("  reg frst_a;\n  (* maxfan = 256 *) reg frst;\n  always @(posedge uclk) begin frst_a <= !uclk_rst_n; frst <= frst_a; end\n"),
        "the user-clock side's reset is the user clock's two registers on, the second copied a copy each 256 loads: {shell}"
    );
    assert_fifo_sides_step_with_their_clocks(&shell, "the hybrid engine");
    let sdc = read("hw/rtl/logos_d5005.sdc");
    for line in [
        "set_multicycle_path -from $logos_held -setup -end 30",
        "set_multicycle_path -from $logos_held -hold -end 29",
        "set_multicycle_path -from $from -to $to -setup -end 31",
        "set_multicycle_path -from $from -to $to -hold -end 30",
        "[list $logos_cap $logos_core $logos_core $logos_core $logos_core $logos_fast $logos_fast $logos_core $logos_cap $logos_fast]",
    ] {
        assert!(sdc.contains(line), "{line}: {sdc}");
    }
    for gone in ["create_generated_clock", "get_clocks", "-to $logos_cap -setup"] {
        assert!(!sdc.contains(gone), "{gone}: {sdc}");
    }
    assert!(read("sw/logos_d5005.h").contains(&format!("#define LOGOS_D5005_STREAMING \"{signature}\"")));
    let reference = emit(&AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512));
    let rread = |f: &str| std::fs::read_to_string(reference.path().join(f)).unwrap();
    assert!(rread("sw/logos_d5005.h").contains("#define LOGOS_D5005_STREAMING \"\""));
    let rshell = rread("hw/rtl/logos_d5005_shell.v");
    assert!(rshell.contains(".fclk(uclk), .frst(frst), .fedge(cce), .fmid(fmid));") && !rshell.contains("fast_desc_fifo"), "{rshell}");
    assert_fifo_sides_step_with_their_clocks(&rshell, "the engine");
    let rmap: serde_json::Value = serde_json::from_str(&rread(CLOCK_DOMAINS_FILE)).unwrap();
    assert_eq!(rmap["single_clock"], serde_json::json!(["logos_engine_core_ce"]));
    assert_eq!((&rmap["crossings"], &rmap["clocks"]), (&map["crossings"], &map["clocks"]));
    assert!(rread("hw/rtl/logos_d5005.sdc").contains("set_multicycle_path -from $logos_held -setup -end 30"));
    let bringup = emit(&AfuConfig::d5005());
    assert!(!std::fs::read_to_string(bringup.path().join("hw/rtl/logos_d5005_shell.v")).unwrap().contains("fclk"), "the bring-up shell");
    for (cfg, what) in [(p.clone(), "a hybrid engine"), (engine_08b(), "an engine")] {
        let too_fast = AfuConfig { compute_divide: ENGINE_MIN_DIVIDE - 2, ..AfuConfig::engine(cfg, LocalMemoryLine::Data512) };
        assert!(std::panic::catch_unwind(|| afu_sources(&too_fast)).is_err(), "{what}'s clock at {} user cycles", ENGINE_MIN_DIVIDE - 2);
    }
    let fastest = AfuConfig { compute_divide: ENGINE_MIN_DIVIDE, ..AfuConfig::engine(p.clone(), LocalMemoryLine::Data512) };
    assert!(afu_sources(&fastest).iter().any(|f| f.path == "hw/rtl/logos_d5005.sdc" && f.text.contains("set_multicycle_path -from $logos_held -setup -end 1\n")));
    assert!(std::panic::catch_unwind(|| afu_sources(&AfuConfig::engine(p, LocalMemoryLine::Data512Secded))).is_err(), "SECDED lines");
    assert!(std::panic::catch_unwind(|| afu_sources(&AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512Secded))).is_ok(), "an engine on SECDED lines");
}

/// A recurrent hybrid build's files: every channel's fast port writes through a FIFO of its own and takes its write done
/// tokens back through another, the stream engine's writer taking the lines of the side it serves and its done token
/// going back to that side; the engine takes the fast ports' writes and the card port's map, the map through the
/// crossing's captures; the user-clock side carries the recurrence unit and its port, started the cycle after the
/// falling edge its row is given in, its busy and the port's writes still out held only then, the store and the
/// channels the unit's while it owns them; the core hands each row over and keeps no lazy row engine of its own. The
/// build runs the hybrid build's programs. A hybrid build without the unit carries none of it, and an engine with no
/// Gated-DeltaNet block takes none.
#[test]
fn a_recurrent_hybrid_build_carries_its_fast_writes_and_its_map() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams};
    let d = emit(&AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512));
    let read = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap();
    let shell = read("hw/rtl/logos_d5005_shell.v");
    for line in [
        "fast_write_fifo(",
        "fast_done_fifo(",
        "logos_dcfifo_mlab #(.W(1), .AW(2), .TOKENS(1)) done_fifo(",
        ".wvalid(done_core)",
        "assign done_core = done && !fast;",
        "logos_dcfifo_mlab #(.W(1), .AW(2), .TOKENS(1)) fast_done_fifo(",
        ".wvalid(done && fast)",
        "assign e_w_valid = fast ? fw_q_valid : w_valid;",
        "assign w_ready = e_w_ready && !fast;",
        "assign fw_q_ready = e_w_ready && fast;",
        ".fw_valid(f_mw_valid[b])",
        ".fdone_valid(f_mdone_valid[b])",
        ".sm_mw_valid(f_mw_valid)",
        ".map_space(port_space), .map_base(port_base), .map_granule_log2(port_granule_log2)",
    ] {
        assert_eq!(shell.matches(line).count(), 1, "{line}: {shell}");
    }
    let v = read("hw/rtl/logos_engine.v");
    let bridge = module_text(&v, "logos_engine_bridge");
    for name in ["map_space", "map_base", "map_granule_log2"] {
        assert!(
            bridge.contains(&format!("    if (emark2) cap_{name} <= s_{name};")) && bridge.contains(&format!("  assign f_{name} = cap_{name};")),
            "{name}: {bridge}"
        );
    }
    let fast = module_text(&v, "logos_engine_fast");
    for line in [
        "wire g_start = gdn_go && cr;",
        "else if (cr) busy_held <= busy || start || g_busy || writing || g_start;",
        ".re(own || cr), .ra(own ? (gdn ? g_ra : u_ra) : e_ra)",
        ".we(own ? (gdn ? g_we : u_we) : ce && e_we)",
        "logos_gdn_side #(",
        "logos_fast_port #(",
        ".space(map_space), .base(map_base),",
        ".clear(g_start), .fault(gdn_fault)",
    ] {
        assert!(fast.contains(line), "{line}: {fast}");
    }
    let core = module_text(&v, "logos_engine_core_ce");
    assert!(!core.contains("logos_gdn_lazy_row") && !core.contains("dl_vals"), "the core keeps no lazy row engine");
    assert!(core.contains("DL_SG: if (wq_empty) begin") && core.contains("assign sm_gdn_job = {"), "the core hands each row over");
    assert!(!core.contains("DL_APP: begin") && !core.contains("DL_CNT:") && !core.contains("DL_CNT_WR:"), "the side, not the core, appends the row's entry and keeps the count");
    let top = module_text(&v, "logos_engine");
    assert!(top.contains(".map_space(f_map_space)") && top.contains(".sm_gdn_go(gdn_go), .sm_gdn_job(gdn_job), .sm_gdn_fault(gdn_fault)"), "{top}");
    let h = emit(&AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512));
    let hread = |f: &str| std::fs::read_to_string(h.path().join(f)).unwrap();
    assert!(!hread("hw/rtl/logos_d5005_shell.v").contains("fast_write_fifo") && !hread("hw/rtl/logos_d5005_shell.v").contains("map_space"));
    let hv = hread("hw/rtl/logos_engine.v");
    assert!(!hv.contains("logos_gdn_side") && !hv.contains("map_space") && hv.contains("logos_gdn_lazy_row"));
    let streaming = |h: &str| h.lines().find(|l| l.contains("#define LOGOS_D5005_STREAMING")).map(str::to_string);
    assert_eq!(streaming(&read("sw/logos_d5005.h")), streaming(&hread("sw/logos_d5005.h")), "the hybrid build's programs");
    let no_delta = EngineParams { delta: None, ..recurrent_08b() };
    assert!(std::panic::catch_unwind(|| engine_verilog(&no_delta)).is_err(), "a recurrence unit for no Gated-DeltaNet block");
}

// ── The PIM boundary ────────────────────────────────────────────────────────────────────────────────

fn sv_check(d: &Path) -> Output {
    let t = root().join("tools/d5005");
    Command::new(t.join("venv/bin/python"))
        .arg(script("sv_check.py"))
        .arg(t.join("pr-template"))
        .arg(d.join("hw/rtl"))
        .output()
        .expect("the d5005 venv's python")
}

/// The emitted ofs_plat_afu and the shell behind it elaborate against the D5005 FIM's PIM, at both line widths and on
/// SECDED lines, with no warning in the accelerator's own files of a kind that hides a wrong connection.
#[test]
fn the_afu_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    for line in [LocalMemoryLine::DataAndEcc576, LocalMemoryLine::Data512, LocalMemoryLine::Data512Secded] {
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let o = sv_check(d.path());
        assert_eq!(o.status.code(), Some(0), "{line:?}: {}", text(&o));
        assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
    }
}

/// Qwen3.5-0.8B's hybrid engine build — its shell's fast channel ports, the engine's core, crossing and fast side —
/// elaborates against the D5005 FIM's PIM, with no warning in the accelerator's own files of a kind that hides a wrong
/// connection.
#[test]
fn a_hybrid_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// Qwen3.5-0.8B's recurrent hybrid build — its shell's fast channel writes, the engine's map and the recurrence unit —
/// elaborates against the D5005 FIM's PIM, with no warning in the accelerator's own files of a kind that hides a wrong
/// connection.
#[test]
fn a_recurrent_hybrid_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// A vectored hybrid build elaborates against the D5005 PIM's interfaces under the strict front end.
#[test]
fn a_vectored_hybrid_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::engine(vectored_08b(), LocalMemoryLine::Data512));
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

/// The PIM boundary gives the shell the bank's bus as the line asks for it — the data bits, or the full bus with the
/// ECC lanes — and sets the shell's SECDED coding exactly on SECDED lines.
#[test]
fn the_pim_boundary_sets_the_shells_line() {
    for (line, width, ecc) in [
        (LocalMemoryLine::Data512, "LOCAL_MEM_DATA_WIDTH", 0),
        (LocalMemoryLine::DataAndEcc576, "LOCAL_MEM_FULL_BUS_WIDTH", 0),
        (LocalMemoryLine::Data512Secded, "LOCAL_MEM_FULL_BUS_WIDTH", 1),
    ] {
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let afu = std::fs::read_to_string(rtl(d.path(), "ofs_plat_afu.sv")).unwrap();
        assert_eq!(afu.matches(&format!("localparam DWM = local_mem_cfg_pkg::{width};")).count(), 1, "{line:?}:\n{afu}");
        assert_eq!(afu.matches(".ECC(").count(), 1, "{line:?}");
        assert!(afu.contains(&format!(".DWM(DWM), .ECC({ecc}),")), "{line:?}:\n{afu}");
    }
}

/// The elaboration refuses a port the PIM's shim does not have, and a shell whose line width is not the bank's.
#[test]
fn the_pim_check_refuses_a_wrong_port_and_a_wrong_width() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    for (from, to, says) in [
        (".afu_reset_n(pclk_rst_n));", ".afu_reset_n(pclk_rst_n), .no_such_port(1'b0));", "no_such_port"),
        (".NB(NB), .AW(AW), .DWM(DWM),", ".NB(NB), .AW(AW), .DWM(512),", "width"),
    ] {
        let d = emit(&AfuConfig::d5005());
        let afu = rtl(d.path(), "ofs_plat_afu.sv");
        let source = std::fs::read_to_string(&afu).unwrap();
        assert_eq!(source.matches(from).count(), 1, "the mutation site {from:?}");
        std::fs::write(&afu, source.replace(from, to)).unwrap();
        let o = sv_check(d.path());
        assert_eq!(o.status.code(), Some(1), "{to:?} is refused: {}", text(&o));
        assert!(text(&o).contains(says), "{to:?}: {}", text(&o));
    }
}

/// The elaboration refuses a conversion the accelerator's own files make inside a module's instance — a width cut in
/// the FIFO the shell instantiates, reported under the instance it is in — as it refuses one at the top.
#[test]
fn the_pim_check_refuses_a_cut_width_inside_an_instance() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let fifo = rtl(d.path(), "logos_dcfifo.v");
    let source = std::fs::read_to_string(&fifo).unwrap();
    let at = source.find("endmodule").expect("the FIFO's end");
    std::fs::write(&fifo, format!("{}  wire [1:0] cut_probe = wdata;\n{}", &source[..at], &source[at..])).unwrap();
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(1), "the cut is refused: {}", text(&o));
    assert!(text(&o).contains("cut_probe") && text(&o).contains("[-Wwidth-trunc]"), "{}", text(&o));
}

// ── The divided compute clock ───────────────────────────────────────────────────────────────────────

/// The net `port` is connected to in the instance text `inst` (`.port(net)`).
fn connection<'a>(inst: &'a str, port: &str) -> &'a str {
    let open = format!(".{port}(");
    let at = inst.find(&open).unwrap_or_else(|| panic!("a connection of {port} in {inst}")) + open.len();
    let mut depth = 1;
    let end = inst[at..]
        .char_indices()
        .find(|&(_, c)| {
            depth += (c == '(') as i32 - (c == ')') as i32;
            depth == 0
        })
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("the end of {port}'s connection"));
    inst[at..at + end].trim()
}

/// Every dual-clock FIFO instance of `module` (its text) with each side's clock, enable and reset:
/// `(instance, [(wclk, wce, wrst_n), (rclk, rce, rrst_n)])`.
fn fifo_sides(module: &str) -> Vec<(String, [(String, String, String); 2])> {
    let mut out = Vec::new();
    let mut rest = module;
    while let Some(at) = ["logos_dcfifo_mlab #(", "logos_dcfifo_m20k #("].iter().filter_map(|f| rest.find(f)).min() {
        let stmt = &rest[at..at + rest[at..].find(");").expect("an instance's end") + 2];
        let params = stmt.find("#(").unwrap() + 1;
        let mut depth = 0;
        let close = stmt[params..]
            .char_indices()
            .find(|&(_, c)| {
                depth += (c == '(') as i32 - (c == ')') as i32;
                depth == 0
            })
            .map(|(i, _)| params + i + 1)
            .expect("the parameters' end");
        let name = stmt[close..].split('(').next().expect("an instance's name").trim().to_string();
        let side = |s: &str| (connection(stmt, &format!("{s}clk")).to_string(), connection(stmt, &format!("{s}ce")).to_string(), connection(stmt, &format!("{s}rst_n")).to_string());
        out.push((name, [side("w"), side("r")]));
        rest = &rest[at + stmt.len()..];
    }
    out
}

/// Every FIFO of `shell` (a shell's text) steps its compute side at the compute clock's step from the compute side's
/// reset, and every other side every cycle of its own clock from that clock's reset: in the shell, a side on the user
/// clock at `cce` from `crst_n`; in a channel, the side on the channel's user clock at the step the shell gives it
/// (`uce`) from the reset it gives it, and the fast port's sides and the channel clock's every cycle.
fn assert_fifo_sides_step_with_their_clocks(shell: &str, what: &str) {
    let top = module_text(shell, "logos_d5005_shell");
    let channel = module_text(shell, "logos_d5005_channel");
    let want = |module: &str, clock: &str| -> (&'static str, &'static str) {
        match (module, clock) {
            ("shell", "uclk") => ("cce", "crst_n"),
            ("shell", "pclk") => ("1'b1", "pclk_rst_n"),
            ("channel", "uclk") => ("uce", "uclk_rst_n"),
            ("channel", "mem_clk") => ("1'b1", "mem_rst_n"),
            ("channel", "fclk") => ("1'b1", "fclk_rst_n"),
            _ => panic!("{what}: a {module} FIFO side on {clock}"),
        }
    };
    let mut seen = 0;
    for (module, text) in [("shell", &top), ("channel", &channel)] {
        for (fifo, sides) in fifo_sides(text) {
            for (clock, ce, rst) in &sides {
                let (wce, wrst) = want(module, clock);
                assert_eq!((ce.as_str(), rst.as_str()), (wce, wrst), "{what}: {module}'s {fifo} on {clock}");
            }
            seen += 1;
        }
    }
    assert!(seen >= 11, "{what}: {seen} FIFOs");
    let ch = &top[top.find("logos_d5005_channel #(").expect("the channels")..];
    assert_eq!((connection(ch, "uclk"), connection(ch, "uce"), connection(ch, "uclk_rst_n")), ("uclk", "cce", "crst_n"), "{what}: the channels' user side");
    if channel.contains("input              fclk,") {
        assert_eq!((connection(ch, "fclk"), connection(ch, "fclk_rst_n")), ("uclk", "uclk_rst_n"), "{what}: the fast ports run every user cycle");
    }
}

/// No clock of the accelerator is made inside it. A persona's region reaches no global clock network of its own, so a
/// clock a register or logic makes rides general routing and the router pays for its skew in delay added for hold:
/// engine2 failed routing with 70,067 ns of it on the divided compute clock alone, and hybrid3b's router added 4e5 ns.
/// Every build's PIM top hands the shell the platform's own clocks, each a net it takes straight from the PIM's clock
/// interfaces, and makes none; the clock-domain map names those three and no other. The shell with everything it
/// instantiates, flattened at every divide, clocks every register and memory port from a clock port (an engine build's
/// shell is held to the same by its crossing gate, which refuses a register clocked by anything but a clock port); the
/// compute clock clocks its registers from the user clock alone at every divide; and the card port from its clock.
#[test]
fn no_clock_of_the_accelerator_is_made_inside_it() {
    use logicaffeine_compile::codegen_sva::netlist::{clock_sources, ClockDriver};
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    use logicaffeine_compile::spatial::decode_engine::physical::card_port_verilog;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::ENGINE_MIN_DIVIDE;
    let divided = |divide: u32| AfuConfig { compute_divide: divide, user_clock_mhz: 100, ..AfuConfig::d5005() };
    let builds: Vec<(String, AfuConfig)> = [1u32, 2, 16, 64]
        .iter()
        .map(|&d| (format!("divided by {d}"), if d == 1 { AfuConfig::d5005() } else { divided(d) }))
        .chain([
            ("the engine".to_string(), AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512)),
            ("the hybrid engine".to_string(), AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512)),
            ("the recurrent hybrid".to_string(), AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512)),
            ("the vectored hybrid".to_string(), AfuConfig::engine(vectored_08b(), LocalMemoryLine::Data512)),
        ])
        .collect();
    for (what, cfg) in &builds {
        let d = emit(cfg);
        let afu = std::fs::read_to_string(rtl(d.path(), "ofs_plat_afu.sv")).unwrap();
        assert!(!afu.contains("logos_compute_clock") && !afu.contains("fclk"), "{what}: the PIM top makes no clock:\n{afu}");
        let shell = &afu[afu.find("logos_d5005_shell #(").expect("the shell's instance")..];
        for (port, pim) in [("pclk", "plat_ifc.clocks.ports[0].pClk.clk"), ("uclk", "plat_ifc.clocks.ports[0].uClk_usr.clk")] {
            let net = connection(shell, port);
            assert_eq!(afu.matches(&format!("  wire {net} = {pim};\n")).count(), 1, "{what}: the shell's {port} is {pim}:\n{afu}");
        }
        assert_eq!(connection(shell, "mem_clk"), "mem_clk", "{what}");
        assert_eq!(afu.matches("    assign mem_clk[b] = mem[b].clk;\n").count(), 1, "{what}: each channel's clock its bank's");
        let map: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.path().join(CLOCK_DOMAINS_FILE)).unwrap()).unwrap();
        assert_eq!(map["clocks"], serde_json::json!(["pclk", "uclk", "mem_clk"]), "{what}");
    }
    if !common::require_tool("yosys") {
        return;
    }
    for (what, cfg) in builds.iter().filter(|(_, cfg)| cfg.engine.is_none()) {
        let d = emit(cfg);
        let verilog: String = shell_files(d.path()).iter().map(|f| std::fs::read_to_string(f).unwrap() + "\n").collect();
        let sources = clock_sources(&verilog, "logos_d5005_shell", &[]).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(sources.iter().map(|s| s.registers).sum::<usize>() > 1000, "{what}: {sources:?}");
        for s in &sources {
            let port = s.net.split('[').next().unwrap();
            assert!(s.driver == ClockDriver::Port && ["pclk", "uclk", "mem_clk"].contains(&port), "{what}: {} clocks {} register bits, driven by {:?}", s.net, s.registers, s.driver);
        }
    }
    for divide in [2u32, 4, ENGINE_MIN_DIVIDE, 64, 80, 128] {
        let sources = clock_sources(COMPUTE_CLOCK_VERILOG, "logos_compute_clock", &[("DIVIDE", &divide.to_string())]).unwrap();
        let clocks: Vec<(&str, ClockDriver)> = sources.iter().map(|s| (s.net.as_str(), s.driver)).collect();
        assert_eq!(clocks, vec![("clk", ClockDriver::Port)], "divided by {divide}: {sources:?}");
    }
    let sources = clock_sources(&card_port_verilog(), "logos_card_port", &[]).unwrap();
    assert!(sources.iter().all(|s| s.driver == ClockDriver::Port && s.net == "clk") && !sources.is_empty(), "the card port: {sources:?}");
}

/// Every register bit of `top`, flattened from `verilog`, that its input port `clock` clocks, after yosys has carried
/// every constant through (`opt -full`, `opt_dff`): each bit by its cell's name and the deepest name of its net (a
/// register's own, inside its instance, before a port it drives), and whether it can still change. A bit whose enable is
/// constant off, or whose next value is its own, cannot; a memory write port on the clock counts a bit for each of its
/// enables that is not constant off. Every register is made an output of `top` first (`expose -dff`), so one whose only
/// readers a constant freezes is still there to be judged rather than cleaned away as dead logic.
fn clocked_bits(verilog: &str, top: &str, clock: &str) -> Vec<(String, bool)> {
    const FLOPS: [&str; 11] = ["$dff", "$adff", "$dffe", "$adffe", "$sdff", "$sdffe", "$sdffce", "$aldff", "$aldffe", "$dffsr", "$dffsre"];
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("design.v"), verilog).unwrap();
    std::fs::write(
        dir.path().join("run.ys"),
        format!("read_verilog -sv design.v\nhierarchy -check -top {top}\nproc\nflatten\nexpose -dff\nopt -full\nopt_dff\nopt_clean\nwrite_json net.json\n"),
    )
    .unwrap();
    let o = Command::new("yosys").args(["-q", "-s", "run.ys"]).current_dir(dir.path()).output().expect("yosys");
    assert!(o.status.success(), "yosys on {top}:\n{}", text(&o));
    let net: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("net.json")).unwrap()).unwrap();
    let m = &net["modules"][top];
    let clk = m["ports"][clock]["bits"][0].clone();
    let mut names: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut nets: Vec<(&String, &serde_json::Value)> = m["netnames"].as_object().unwrap().iter().collect();
    nets.sort_by_key(|(n, v)| (v["hide_name"].as_u64().unwrap_or(0), n.starts_with('$'), std::cmp::Reverse(n.matches('.').count()), n.len(), n.to_string()));
    for (n, v) in nets {
        let bits = v["bits"].as_array().unwrap();
        for (i, b) in bits.iter().enumerate() {
            names.entry(b.to_string()).or_insert_with(|| if bits.len() == 1 { n.clone() } else { format!("{n}[{i}]") });
        }
    }
    let param = |c: &serde_json::Value, p: &str| c["parameters"][p].as_str().map_or(1, |s| u64::from_str_radix(s, 2).unwrap_or(1));
    let mut out = Vec::new();
    for (cname, c) in m["cells"].as_object().unwrap() {
        let ty = c["type"].as_str().unwrap_or("");
        let conn = &c["connections"];
        if ty.starts_with("$memwr") {
            if conn["CLK"][0] == clk {
                let id = c["parameters"]["MEMID"].as_str().unwrap_or(cname).trim_start_matches('\\').to_string();
                for (i, en) in conn["EN"].as_array().unwrap().iter().enumerate() {
                    out.push((format!("{cname} {id} write[{i}]"), en != "0"));
                }
            }
            continue;
        }
        if !FLOPS.contains(&ty) || conn["CLK"][0] != clk {
            continue;
        }
        let off = if param(c, "EN_POLARITY") == 1 { "0" } else { "1" };
        let disabled = conn.get("EN").is_some_and(|en| en.as_array().unwrap().iter().all(|b| b == off));
        let (q, d) = (conn["Q"].as_array().unwrap(), conn["D"].as_array().unwrap());
        for (i, qb) in q.iter().enumerate() {
            let held = disabled || d.get(i) == Some(qb);
            out.push((format!("{cname} {}", names.get(&qb.to_string()).cloned().unwrap_or_else(|| format!("[{i}]"))), !held));
        }
    }
    out
}

/// A stand-in for the compute clock that never steps: its marks low, its reset the user clock's.
const NO_STEP_CLOCK: &str = "module logos_compute_clock #(parameter DIVIDE = 2, parameter LEAD = 0)\n  (input clk, input rst_n, output cclk, output cclk_rst_n, output fedge, output fmid);\n  \
                             assign cclk = 1'b0;\n  assign cclk_rst_n = rst_n;\n  assign fedge = 1'b0;\n  assign fmid = 1'b0;\nendmodule\n";

/// A stand-in for `engine` (an engine's Verilog): its ports, every output the inverse of a register of noise in its user
/// clock, so no value the shell takes from the engine is a constant yosys could fold a register of the shell's away
/// with.
fn noise_engine(engine: &str) -> String {
    let at = engine.find("module logos_engine\n").expect("the engine's top");
    let header = &engine[at..at + engine[at..].find(");").expect("the engine's ports") + 2];
    let outputs: Vec<&str> = header
        .split(['(', ','])
        .map(str::trim)
        .filter(|d| d.starts_with("output"))
        .map(|d| d.split_whitespace().last().expect("a port's name").trim_end_matches(");"))
        .collect();
    let mut text = format!("{header}\n  reg [63:0] noise = 64'h1;\n  always @(posedge fclk) noise <= {{noise[62:0], noise[63] ^ noise[62] ^ noise[60] ^ noise[59]}};\n");
    for o in outputs {
        text.push_str(&format!("  assign {o} = ~{{32{{noise}}}};\n"));
    }
    text + "endmodule\n"
}

/// The shell's compute side steps only at the compute clock's step. With the compute clock replaced by one that never
/// steps, no register bit the user clock clocks in the shell can change — the core's, the card port's, every FIFO's
/// compute side and its synchronizers — but the engine's (here a stand-in that changes every cycle), a hybrid build's
/// fast ports and its user-clock side's reset (the user clock's reset two registers on, `frst_a` and `frst`), which run
/// every user cycle; with the real compute clock the same bits can. So a path into the
/// compute side from a value the engine holds (`held_*`), which the constraints give half a compute cycle less two, ends
/// at a register that takes it only at the step, a memory written only then, or no register at all.
#[test]
fn the_compute_side_steps_only_at_the_compute_clocks_step() {
    if !common::require_tool("yosys") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    let builds = [
        ("divided by 16", AfuConfig { compute_divide: 16, user_clock_mhz: 100, ..AfuConfig::d5005() }),
        ("the engine", AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512)),
        ("the hybrid engine", AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512)),
        ("the recurrent hybrid", AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512)),
    ];
    for (what, cfg) in builds {
        let d = emit(&cfg);
        let mut base: String = shell_files(d.path())
            .iter()
            .filter(|f| !f.ends_with("logos_compute_clock.v"))
            .map(|f| std::fs::read_to_string(f).unwrap() + "\n")
            .collect();
        if cfg.engine.is_some() {
            base += &std::fs::read_to_string(rtl(d.path(), "logos_card_port.v")).unwrap();
            base += &noise_engine(&std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap());
        }
        let runs_every_cycle = |n: &str| {
            n.contains("engine.")
                || n.contains("compute_clock.")
                || n.contains(".fast_") && n.contains("_fifo.")
                || n.ends_with(" frst_a")
                || n.ends_with(" frst")
        };
        let compute_side = |clock: &str| -> Vec<(String, bool)> {
            clocked_bits(&format!("{base}\n{clock}"), "logos_d5005_shell", "uclk").into_iter().filter(|(n, _)| !runs_every_cycle(n)).collect()
        };
        let live = compute_side(COMPUTE_CLOCK_VERILOG).into_iter().filter(|(_, can)| *can).count();
        assert!(live > 1000, "{what}: {live} compute-side bits can change when the compute clock steps");
        let moving: Vec<String> = compute_side(NO_STEP_CLOCK).into_iter().filter(|(_, can)| *can).map(|(n, _)| n).collect();
        assert!(moving.is_empty(), "{what}: {} bits change with the compute clock never stepping: {:?}", moving.len(), &moving[..moving.len().min(40)]);
    }
}

/// A divided build runs on the user clock and steps its compute side at the user clock divided: the shell's own
/// `logos_compute_clock` divides the user clock by the build's divide into the marks of the user cycles that end with
/// the divided clock's rising and falling edges (`cce`, `fmid`) and a reset released at its second rising edge
/// (`crst_n`), its divided clock clocking nothing; the core and every FIFO's compute side step at `cce` from `crst_n`;
/// the divider is among the sources; the constraints generate no clock; the AFU JSON asks for exactly the user clock.
/// An undivided build steps every cycle from the user clock's reset and carries no divider.
#[test]
fn a_divided_build_steps_its_compute_side_in_the_user_clock() {
    use logicaffeine_compile::spatial::decode_engine::ofs::MARK_LEAD;
    let read = |d: &Path, f: &str| std::fs::read_to_string(rtl(d, f)).unwrap();
    for divide in [2u32, 16, 64] {
        let what = format!("divided by {divide}");
        let d = emit(&AfuConfig { compute_divide: divide, user_clock_mhz: 100, ..AfuConfig::d5005() });
        let shell = read(d.path(), "logos_d5005_shell.v");
        let clock = format!(
            "  wire cce, fmid, crst_n, cphase;\n  logos_compute_clock #(.DIVIDE({divide}), .LEAD({MARK_LEAD})) compute_clock(.clk(uclk), \
             .rst_n(uclk_rst_n), .cclk(cphase),\n    .cclk_rst_n(crst_n), .fedge(cce), .fmid(fmid));\n"
        );
        assert_eq!(shell.matches(&clock).count(), 1, "{what}:\n{shell}");
        assert_eq!(shell.matches("logos_compute_clock").count(), 1, "{what}");
        assert_eq!(shell.matches("cphase").count(), 3, "{what}: the divided clock's level declared and connected, read by nothing");
        let top = module_text(&shell, "logos_d5005_shell");
        let core = &top[top.find("logos_bringup_core #(").expect("the core")..];
        assert_eq!((connection(core, "clk"), connection(core, "ce"), connection(core, "rst_n")), ("uclk", "cce", "crst_n"), "{what}: the core");
        assert_fifo_sides_step_with_their_clocks(&shell, &what);
        let afu = read(d.path(), "ofs_plat_afu.sv");
        assert!(afu.contains("  wire uclk = plat_ifc.clocks.ports[0].uClk_usr.clk;\n  wire uclk_rst_n = plat_ifc.clocks.ports[0].uClk_usr.reset_n;\n"), "{what}:\n{afu}");
        let sources: Vec<String> = read(d.path(), "sources.txt").lines().map(str::to_string).collect();
        assert_eq!(sources.iter().filter(|l| *l == "logos_compute_clock.v").count(), 1, "{what}: {sources:?}");
        assert!(rtl(d.path(), "logos_compute_clock.v").is_file(), "{what}");
        let sdc = read(d.path(), "logos_d5005.sdc");
        assert!(!sdc.contains("create_generated_clock") && !sdc.contains("get_clocks"), "{what}: no clock but the platform's:\n{sdc}");
        assert_eq!(afu_json(d.path())["afu-image"]["clock-frequency-high"], serde_json::json!(100), "{what}");
    }
    let d = emit(&AfuConfig::d5005());
    let shell = read(d.path(), "logos_d5005_shell.v");
    assert_eq!(shell.matches("  wire cce = 1'b1;\n  wire crst_n = uclk_rst_n;\n").count(), 1, "the undivided shell:\n{shell}");
    assert!(!shell.contains("logos_compute_clock"));
    assert_fifo_sides_step_with_their_clocks(&shell, "undivided");
    assert!(!read(d.path(), "logos_d5005.sdc").contains("create_generated_clock"));
    assert!(!read(d.path(), "sources.txt").contains("logos_compute_clock"));
    assert!(!rtl(d.path(), "logos_compute_clock.v").exists());
    assert_eq!(afu_json(d.path())["afu-image"]["clock-frequency-high"], serde_json::json!("auto-450"));
}

/// A divided shell moves every line as the undivided one does (`the_shell_moves_every_line_through_its_fifos`), its user
/// clock `divide` times as fast and its compute side stepping at every `divide`-th edge of it: every register, command
/// and path between the clocks read back, progress records every 2^HB steps.
#[test]
fn a_divided_shell_moves_every_line_through_its_fifos() {
    if !common::require_tool("iverilog") {
        return;
    }
    for divide in [2u32, 6] {
        let d = emit(&AfuConfig { compute_divide: divide, user_clock_mhz: 100, ..AfuConfig::d5005() });
        let mut files = vec![fixture("tb_shell.v"), d.path().join("sim/logos_avmm_memory_model.v"), fixture("host_memory_model.v")];
        files.extend(shell_files(d.path()));
        let out = simulate(&files, &[format!("-Ptb.DIVIDE={divide}")]);
        assert!(out.contains("RESULT errors=0"), "divided by {divide}:\n{out}");
        assert!(!out.contains("FAIL"), "divided by {divide}: {out}");
    }
}

/// The user clocks the AFU JSON asks for are a pair the PIM's `user_clock_config.tcl` takes: both auto, or both fixed
/// with the low clock exactly half the high one; a fixed high clock beside an auto low one fails the build after
/// synthesis. The bring-up build asks for auto clocks, every divided build for its user clock and its half.
#[test]
fn the_afu_json_asks_for_user_clocks_the_pim_accepts() {
    let builds = [(AfuConfig::d5005(), true), (AfuConfig { compute_divide: 64, user_clock_mhz: 100, ..AfuConfig::d5005() }, false),
                  (AfuConfig { compute_divide: 2, user_clock_mhz: 101, ..AfuConfig::d5005() }, false)];
    for (cfg, auto) in builds {
        let d = emit(&cfg);
        let image = &afu_json(d.path())["afu-image"];
        let (high, low) = (&image["clock-frequency-high"], &image["clock-frequency-low"]);
        if auto {
            assert!(high.as_str().unwrap().starts_with("auto") && low.as_str().unwrap().starts_with("auto"), "{high} {low}");
        } else {
            let (high, low) = (high.as_f64().unwrap_or_else(|| panic!("a fixed high clock: {high}")), low.as_f64().unwrap_or_else(|| panic!("a fixed low clock beside a fixed high one: {low}")));
            assert_eq!(high, cfg.user_clock_mhz as f64);
            assert_eq!(low * 2.0, high, "the low clock is half the high one");
        }
    }
}

/// The engine build (`AfuConfig::engine`) is the bring-up build's core and channels with the engine behind them, its
/// compute clock the engine's user clock divided: the shell steps its compute side at the engine's divide, the C header
/// names it for the host's clock reading to divide by (the bring-up build's divide 1), and the co-simulation runs the
/// user clock at its own period and the shell as on the card, its host program told the compute clock's period, so the
/// channels meet the compute side at the ratio they do on the card.
#[test]
fn the_engine_build_runs_its_core_at_the_divided_clock() {
    use logicaffeine_compile::spatial::decode_engine::ofs::{cosim_user_clock_ps, ENGINE_COMPUTE_DIVIDE, ENGINE_USER_CLOCK_MHZ, MARK_LEAD};
    let params = engine_08b();
    let cfg = AfuConfig::engine(params.clone(), LocalMemoryLine::Data512Secded);
    assert_eq!((cfg.user_clock_mhz, cfg.compute_divide, cfg.engine.as_ref()), (ENGINE_USER_CLOCK_MHZ, ENGINE_COMPUTE_DIVIDE, Some(&params)));
    assert_eq!((cfg.line, cfg.max_burst), (LocalMemoryLine::Data512Secded, AfuConfig::d5005().max_burst));
    let d = emit(&cfg);
    let header_text = std::fs::read_to_string(d.path().join("sw/logos_d5005.h")).unwrap();
    assert!(header_text.contains(&format!("#define LOGOS_D5005_COMPUTE_DIVIDE {ENGINE_COMPUTE_DIVIDE}u\n")), "{header_text}");
    let shell = std::fs::read_to_string(rtl(d.path(), "logos_d5005_shell.v")).unwrap();
    assert!(shell.contains(&format!("logos_compute_clock #(.DIVIDE({ENGINE_COMPUTE_DIVIDE}), .LEAD({MARK_LEAD}))")), "{shell}");
    let period = cosim_user_clock_ps(&cfg);
    assert_eq!(period, (1e6 / ENGINE_USER_CLOCK_MHZ as f64).round() as u64 / 2 * 2);
    let cosim = std::fs::read_to_string(d.path().join("sim/logos_d5005_cosim.v")).unwrap();
    for line in [
        format!("  parameter UCLK_PS = {period};\n"),
        format!("  parameter DIVIDE = {ENGINE_COMPUTE_DIVIDE};\n"),
        "  always #(UCLK_PS / 2) uclk = ~uclk;\n".to_string(),
        "    .pclk(pclk), .pclk_rst_n(rst_n), .uclk(uclk), .uclk_rst_n(rst_n),\n".to_string(),
        "    logos_dpi_program(UCLK_PS * DIVIDE);\n".to_string(),
        "    $logos_program(UCLK_PS * DIVIDE);\n".to_string(),
    ] {
        assert_eq!(cosim.matches(&line).count(), 1, "{line}: {cosim}");
    }
    assert!(!cosim.contains("compute_clock") && !cosim.contains("fclk"), "the shell divides its own clock: {cosim}");
    let bring_up = emit(&AfuConfig::d5005());
    let header_text = std::fs::read_to_string(bring_up.path().join("sw/logos_d5005.h")).unwrap();
    assert!(header_text.contains("#define LOGOS_D5005_COMPUTE_DIVIDE 1u\n"), "{header_text}");
    let cosim = std::fs::read_to_string(bring_up.path().join("sim/logos_d5005_cosim.v")).unwrap();
    assert!(cosim.contains("  parameter DIVIDE = 1;\n") && cosim.contains(&format!("  parameter UCLK_PS = {};\n", cosim_user_clock_ps(&AfuConfig::d5005()))), "{cosim}");
}

/// The divider under iverilog for divides of 2 to 64: its clock a square wave of `DIVIDE` user clock periods, high for
/// half of them; its reset held while the user clock's is and through the first divided rising edge after, released at
/// the second, and asserted again the moment the user clock's is.
#[test]
fn the_compute_clock_divides_the_user_clock_and_releases_its_reset_in_it() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    for divide in [2u64, 4, 6, 64] {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("logos_compute_clock.v");
        std::fs::write(&v, COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = dir.path().join("tb.v");
        std::fs::write(
            &tb,
            format!(
                r#"module tb;
  reg clk = 0, rst_n = 0;
  wire cclk, cclk_rst_n;
  logos_compute_clock #(.DIVIDE({divide})) u(.clk(clk), .rst_n(rst_n), .cclk(cclk), .cclk_rst_n(cclk_rst_n));
  always #5 clk = ~clk;
  always @(posedge cclk) $display("RISE %0d", $time);
  always @(negedge cclk) $display("FALL %0d", $time);
  always @(cclk_rst_n) $display("RST %0d %0d", cclk_rst_n, $time);
  initial begin
    #37 rst_n = 1;
    #({divide} * 10 * 6 + 3) rst_n = 0;
    #1 $display("HELD %0d", cclk_rst_n);
    #({divide} * 10 * 2) $display("STILL %0d", cclk_rst_n);
    $finish;
  end
endmodule
"#
            ),
        )
        .unwrap();
        let out = simulate(&[v, tb], &[]);
        let times = |tag: &str| -> Vec<u64> {
            out.lines()
                .filter_map(|l| l.strip_prefix(tag))
                .map(|t| t.trim().parse::<u64>().unwrap())
                .collect()
        };
        let (rises, falls) = (times("RISE "), times("FALL "));
        assert!(rises.len() >= 5, "{divide}: {out}");
        let period = 10 * divide;
        for w in rises.windows(2) {
            assert_eq!(w[1] - w[0], period, "{divide}: the divided period:\n{out}");
        }
        let fall_after_first = falls.iter().find(|&&f| f > rises[0]).expect("a fall after the first rise");
        assert_eq!(fall_after_first - rises[0], period / 2, "{divide}: high for half the period:\n{out}");
        // The user clock's reset rises at 37; the divided clock's first rise is at the DIVIDE/2-th user rising edge past it.
        assert_eq!(rises[0], 45 + 10 * (divide / 2 - 1), "{divide}: the first divided edge:\n{out}");
        let released: Vec<u64> = out.lines().filter_map(|l| l.strip_prefix("RST 1 ")).map(|t| t.trim().parse::<u64>().unwrap()).collect();
        assert_eq!(released, vec![rises[1]], "{divide}: released at the second divided rising edge, once:\n{out}");
        let asserted = 37 + divide * 60 + 3;
        assert!(out.contains(&format!("RST 0 {asserted}")), "{divide}: asserted with the user clock's reset at {asserted}:\n{out}");
        assert!(out.contains("HELD 0") && out.contains("STILL 0"), "{divide}:\n{out}");
    }
}

/// The divider's marks and its reset's release come out of it `LEAD` user clocks late and nothing else changes: for
/// divides of 2 to 64, at a lead of two each user cycle's marks are the undelayed divider's two cycles before, the reset is
/// released two cycles after the undelayed one's release, and asserted the moment the user clock's is, as the undelayed
/// one is. A shell steps its compute side on the late marks, so every consumer sees the divider alike.
#[test]
fn the_compute_clocks_late_marks_and_release_are_its_own_a_lead_later() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::ofs::{COMPUTE_CLOCK_VERILOG, MARK_LEAD};
    let lead = MARK_LEAD as usize;
    assert!(lead >= 1, "a lead of {lead}");
    for divide in [2u64, 4, 6, 64] {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("logos_compute_clock.v");
        std::fs::write(&v, COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = dir.path().join("tb.v");
        std::fs::write(
            &tb,
            format!(
                r#"module tb;
  reg clk = 0, rst_n = 0;
  wire c0, r0, e0, m0, c1, r1, e1, m1;
  logos_compute_clock #(.DIVIDE({divide})) now(.clk(clk), .rst_n(rst_n), .cclk(c0), .cclk_rst_n(r0), .fedge(e0), .fmid(m0));
  logos_compute_clock #(.DIVIDE({divide}), .LEAD({lead})) late(.clk(clk), .rst_n(rst_n), .cclk(c1), .cclk_rst_n(r1), .fedge(e1),
    .fmid(m1));
  always #5 clk = ~clk;
  always @(posedge clk) $display("CYCLE %0d %0d %0d %0d %0d %0d %0d", $time, e0, m0, r0, e1, m1, r1);
  initial begin
    #37 rst_n = 1;
    #({divide} * 10 * 8 + 3) rst_n = 0;
    #1 $display("ASSERTED %0d %0d", r0, r1);
    #100 $finish;
  end
endmodule
"#
            ),
        )
        .unwrap();
        let out = simulate(&[v, tb], &[]);
        let cycles: Vec<Vec<u64>> = out
            .lines()
            .filter_map(|l| l.strip_prefix("CYCLE "))
            .map(|l| l.split_whitespace().map(|x| x.parse().unwrap_or(9)).collect())
            .filter(|c: &Vec<u64>| c[0] > 37 && c[0] < 37 + divide * 80)
            .collect();
        assert!(cycles.len() > 4 * divide as usize, "{divide}: {out}");
        for k in lead..cycles.len() {
            assert_eq!((cycles[k][4], cycles[k][5]), (cycles[k - lead][1], cycles[k - lead][2]), "{divide}: the marks at cycle {}:\n{out}", cycles[k][0]);
            assert_eq!(cycles[k][6], cycles[k - lead][3], "{divide}: the release at cycle {}:\n{out}", cycles[k][0]);
        }
        assert!(cycles.iter().any(|c| c[3] == 1), "{divide}: released:\n{out}");
        assert!(out.contains("ASSERTED 0 0"), "{divide}: both asserted the moment the user clock's reset is:\n{out}");
    }
}

/// The divider marks the user clock cycles that end with its edges, for divides of 2 to 64: `fedge` high in exactly the
/// cycles whose closing user edge is a divided rising edge, `fmid` in exactly those whose closing edge is a falling one
/// — where a hybrid engine's crossing takes the engine's store writes and the rest of its values.
#[test]
fn the_compute_clock_marks_the_user_cycles_that_end_with_its_edges() {
    if !common::require_tool("iverilog") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG;
    for divide in [2u64, 4, 6, 64] {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path().join("logos_compute_clock.v");
        std::fs::write(&v, COMPUTE_CLOCK_VERILOG).unwrap();
        let tb = dir.path().join("tb.v");
        std::fs::write(
            &tb,
            format!(
                r#"module tb;
  reg clk = 0, rst_n = 0;
  wire cclk, cclk_rst_n, fedge, fmid;
  logos_compute_clock #(.DIVIDE({divide})) u(.clk(clk), .rst_n(rst_n), .cclk(cclk), .cclk_rst_n(cclk_rst_n), .fedge(fedge),
    .fmid(fmid));
  always #5 clk = ~clk;
  always @(posedge clk) if (rst_n) $display("EDGE %0d %0d %0d", $time, fedge, fmid);
  always @(posedge cclk) $display("RISE %0d", $time);
  always @(negedge cclk) $display("FALL %0d", $time);
  initial begin
    #37 rst_n = 1;
    #({divide} * 10 * 5) $finish;
  end
endmodule
"#
            ),
        )
        .unwrap();
        let out = simulate(&[v, tb], &[]);
        let times = |tag: &str| -> Vec<u64> { out.lines().filter_map(|l| l.strip_prefix(tag)).map(|t| t.trim().parse::<u64>().unwrap()).collect() };
        let (rises, falls) = (times("RISE "), times("FALL "));
        assert!(rises.len() >= 4 && falls.len() >= 4, "{divide}: {out}");
        let mut edges = 0;
        for l in out.lines().filter_map(|l| l.strip_prefix("EDGE ")) {
            let f: Vec<u64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let (t, fedge, fmid) = (f[0], f[1] == 1, f[2] == 1);
            assert_eq!(fedge, rises.contains(&t), "{divide}: fedge in the user cycle ending at {t}:\n{out}");
            assert_eq!(fmid, falls.contains(&t), "{divide}: fmid in the user cycle ending at {t}:\n{out}");
            edges += 1;
        }
        assert!(edges as u64 >= 4 * divide, "{divide}: {edges} user edges seen:\n{out}");
    }
}

/// A divided engine-free build elaborates against the D5005 FIM's PIM.
#[test]
fn a_divided_build_elaborates_against_the_d5005_pim() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let d = emit(&AfuConfig { compute_divide: 16, user_clock_mhz: 100, ..AfuConfig::d5005() });
    let o = sv_check(d.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ofs_plat_afu: elaborates against the PIM"), "{}", text(&o));
}

// ── The build files ─────────────────────────────────────────────────────────────────────────────────

fn afu_json(d: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(rtl(d, "logos_d5005.json")).unwrap()).unwrap()
}

/// The AFU JSON carries what OPAE's schema requires (`afu_schema_v01.json`), asks for the user clock the config names,
/// and names the UUID whose halves the feature header's ID registers carry, high half at word 2.
#[test]
fn the_afu_json_is_the_schemas_and_names_the_headers_id() {
    let cfg = AfuConfig::d5005();
    let d = emit(&cfg);
    let j = afu_json(d.path());
    assert!(j["version"].is_number());
    let image = &j["afu-image"];
    assert!(image["power"].is_number());
    assert_eq!(image["afu-top-interface"]["class"], "ofs_plat_afu");
    let clock = image["clock-frequency-high"].as_str().unwrap();
    assert_eq!(clock, format!("auto-{}", cfg.user_clock_mhz));
    let pattern_ok = |s: &str| s == "auto" || s.strip_prefix("auto-").is_some_and(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit() || c == '.'));
    assert!(pattern_ok(clock) && pattern_ok(image["clock-frequency-low"].as_str().unwrap()), "the schema's ^auto(-[0-9.]+)?$");
    let clusters = image["accelerator-clusters"].as_array().unwrap();
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0]["name"], cfg.name);
    assert!(clusters[0]["total-contexts"].is_number());
    let uuid = clusters[0]["accelerator-type-uuid"].as_str().unwrap();
    assert_eq!(uuid, cfg.uuid_text());
    assert_eq!(uuid.replace('-', ""), format!("{:032x}", cfg.uuid));
    let afu = std::fs::read_to_string(rtl(d.path(), "ofs_plat_afu.sv")).unwrap();
    assert!(afu.contains(&format!(".AFU_ID_L(64'h{})", &uuid.replace('-', "")[16..])), "the low half at ID_L");
    assert!(afu.contains(&format!(".AFU_ID_H(64'h{})", &uuid.replace('-', "")[..16])), "the high half at ID_H");

    if common::require_tool("opae-sdk") {
        let schema: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root().join("tools/d5005/opae-sdk/python/packager/packager/schema/afu_schema_v01.json")).unwrap(),
        )
        .unwrap();
        let required = |s: &serde_json::Value| s["required"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect::<Vec<_>>();
        for k in required(&schema) {
            assert!(!j[&k].is_null(), "the schema requires {k}");
        }
        let image_schema = &schema["properties"]["afu-image"];
        for k in required(image_schema) {
            assert!(!image[&k].is_null(), "the schema requires afu-image/{k}");
        }
        for k in required(&image_schema["properties"]["accelerator-clusters"]["items"]) {
            assert!(!clusters[0][&k].is_null(), "the schema requires each cluster's {k}");
        }
    }
}

/// sources.txt names every RTL file, the JSON and the constraints exactly once, and OPAE's rtl_src_config turns each
/// into the Quartus assignment its kind takes.
#[test]
fn sources_txt_is_every_file_the_build_compiles() {
    let d = emit(&AfuConfig::d5005());
    let listed: Vec<String> = std::fs::read_to_string(rtl(d.path(), "sources.txt")).unwrap().lines().map(str::to_string).collect();
    let mut on_disk: Vec<String> = std::fs::read_dir(d.path().join("hw/rtl"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|f| f != "sources.txt")
        .collect();
    on_disk.sort();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(sorted, on_disk, "every file under hw/rtl, once");
    if !common::require_tool("opae-sdk") || !common::require_tool("python3") {
        return;
    }
    let o = Command::new("python3")
        .arg(root().join("tools/d5005/opae-sdk/platforms/platmgr/tools/rtl_src_config.py"))
        .args(["--qsf", "--abs"])
        .arg(rtl(d.path(), "sources.txt"))
        .output()
        .expect("python3");
    assert!(o.status.success(), "{}", text(&o));
    let qsf = text(&o);
    for f in &listed {
        let tag = match f.rsplit('.').next().unwrap() {
            "v" => "VERILOG_FILE",
            "sv" => "SYSTEMVERILOG_FILE",
            "sdc" => "SDC_FILE",
            "json" => "MISC_FILE",
            other => panic!("{f}: a .{other} file"),
        };
        assert!(qsf.lines().any(|l| l.contains(&format!("-name {tag} ")) && l.contains(f.as_str())), "{f} as {tag}:\n{qsf}");
    }
}

/// A build given a floorplan carries it as `hw/rtl/logos_floorplan.tcl`, named once in sources.txt as `QI:` (a bare
/// .tcl is Platform Designer's to OPAE and never reaches Quartus), so rtl_src_config's project settings source it by its
/// path (`source "<file>"`), once; a build given none has neither the file nor the line.
#[test]
fn a_floorplan_is_sourced_into_the_build_and_none_is_without_one() {
    let plan = "set_instance_assignment -name PLACE_REGION \"X61 Y36 X84 Y59\" -to afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[3].channel\n\
                set_instance_assignment -name CORE_ONLY_PLACE_REGION ON -to afu_top|port_gasket|pr_slot|afu_main|ofs_plat_afu|shell|ch[3].channel\n";
    let planned = emit(&AfuConfig { floorplan: Some(plan.to_string()), ..AfuConfig::d5005() });
    assert_eq!(std::fs::read_to_string(rtl(planned.path(), "logos_floorplan.tcl")).unwrap(), plan);
    let listed = std::fs::read_to_string(rtl(planned.path(), "sources.txt")).unwrap();
    assert_eq!(listed.lines().filter(|l| *l == "QI:logos_floorplan.tcl").count(), 1, "{listed}");
    assert!(!listed.lines().any(|l| l == "logos_floorplan.tcl"), "never as a bare .tcl: {listed}");
    let plain = emit(&AfuConfig::d5005());
    assert!(!rtl(plain.path(), "logos_floorplan.tcl").exists());
    assert!(!std::fs::read_to_string(rtl(plain.path(), "sources.txt")).unwrap().contains("floorplan"));
    if !common::require_tool("opae-sdk") || !common::require_tool("python3") {
        return;
    }
    let o = Command::new("python3")
        .arg(root().join("tools/d5005/opae-sdk/platforms/platmgr/tools/rtl_src_config.py"))
        .args(["--qsf", "--abs"])
        .arg(rtl(planned.path(), "sources.txt"))
        .output()
        .expect("python3");
    assert!(o.status.success(), "{}", text(&o));
    let qsf = text(&o);
    let source = format!("source \"{}\"", rtl(planned.path(), "logos_floorplan.tcl").display());
    assert_eq!(qsf.lines().filter(|l| l.trim() == source).count(), 1, "the floorplan sourced by Quartus, once:\n{qsf}");
}

/// An engine's build carries its project settings as `hw/rtl/logos_settings.tcl`, named once in sources.txt as `QI:`,
/// so rtl_src_config's project settings source it once: the Timing Analyzer times an MLAB's write and a read of the same
/// word, which the engine's memories read without a register after them (in MLABs checking no read-during-write) need
/// for a word read in the clock after its write to be the one written. The bring-up build, with no engine, has neither
/// the file nor the line.
#[test]
fn an_engines_build_times_an_mlab_read_of_a_word_in_the_clock_after_its_write() {
    use logicaffeine_compile::spatial::decode_engine::ofs::ENGINE_SETTINGS;
    let d = emit(&AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512));
    let settings = std::fs::read_to_string(rtl(d.path(), "logos_settings.tcl")).unwrap();
    assert_eq!(settings, ENGINE_SETTINGS);
    let assignments: Vec<&str> = settings.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(assignments, ["set_global_assignment -name MLAB_ADD_TIMING_CONSTRAINTS_FOR_MIXED_PORT_FEED_THROUGH_MODE_SETTING_DONT_CARE ON"]);
    let listed = std::fs::read_to_string(rtl(d.path(), "sources.txt")).unwrap();
    assert_eq!(listed.lines().filter(|l| *l == "QI:logos_settings.tcl").count(), 1, "{listed}");
    assert!(!listed.lines().any(|l| l == "logos_settings.tcl"), "never as a bare .tcl: {listed}");
    let engine = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    assert!(engine.contains("`LOGOS_RAMSTYLE_ASYNC("), "the engine reads memories in MLABs without a register after them");
    let plain = emit(&AfuConfig::d5005());
    assert!(!rtl(plain.path(), "logos_settings.tcl").exists());
    assert!(!std::fs::read_to_string(rtl(plain.path(), "sources.txt")).unwrap().contains("settings"));
    if !common::require_tool("opae-sdk") || !common::require_tool("python3") {
        return;
    }
    let o = Command::new("python3")
        .arg(root().join("tools/d5005/opae-sdk/platforms/platmgr/tools/rtl_src_config.py"))
        .args(["--qsf", "--abs"])
        .arg(rtl(d.path(), "sources.txt"))
        .output()
        .expect("python3");
    assert!(o.status.success(), "{}", text(&o));
    let source = format!("source \"{}\"", rtl(d.path(), "logos_settings.tcl").display());
    assert_eq!(text(&o).lines().filter(|l| l.trim() == source).count(), 1, "the settings sourced by Quartus, once:\n{}", text(&o));
}

/// The C header is the register map, the commands and the status codes, and compiles.
#[test]
fn the_c_header_is_the_register_map() {
    let cfg = AfuConfig::d5005();
    let d = emit(&cfg);
    let h = std::fs::read_to_string(d.path().join("sw/logos_d5005.h")).unwrap();
    let define = |name: &str| -> String {
        h.lines()
            .find_map(|l| l.strip_prefix(&format!("#define {name} ")))
            .unwrap_or_else(|| panic!("{name} is defined"))
            .trim()
            .to_string()
    };
    let number = |s: String| -> u64 {
        let s = s.trim_end_matches('u');
        s.strip_prefix("0x").map_or_else(|| s.parse().unwrap(), |x| u64::from_str_radix(x, 16).unwrap())
    };
    assert_eq!(define("LOGOS_D5005_AFU_ID"), format!("\"{}\"", cfg.uuid_text()));
    for r in REGISTERS {
        assert_eq!(number(define(&format!("LOGOS_D5005_REG_{}", r.name))), 8 * r.word as u64, "{}", r.name);
    }
    for o in Op::ALL {
        assert_eq!(number(define(&format!("LOGOS_D5005_OP_{}", o.name()))), o.code() as u64, "{}", o.name());
    }
    assert_eq!(number(define("LOGOS_D5005_STATUS_OK")), STATUS_OK as u64);
    assert_eq!(number(define("LOGOS_D5005_STATUS_UNKNOWN_OP")), STATUS_UNKNOWN_OP as u64);
    assert_eq!(number(define("LOGOS_D5005_STATUS_RUNNING")), STATUS_RUNNING as u64);
    assert_ne!(STATUS_RUNNING, STATUS_OK, "a progress record is not a command's own");
    assert_ne!(STATUS_RUNNING, STATUS_UNKNOWN_OP, "a progress record is not a refusal");
    assert_eq!(number(define("LOGOS_D5005_ERROR_COMMAND_DROPPED")), ERROR_COMMAND_DROPPED);
    if !common::require_tool("cc") {
        return;
    }
    let c = d.path().join("check.c");
    std::fs::write(&c, "#include \"logos_d5005.h\"\nint main(void) { return LOGOS_D5005_REG_DOORBELL == 0 ? 1 : 0; }\n").unwrap();
    let o = Command::new("cc").args(["-std=c99", "-Wall", "-Werror", "-fsyntax-only", "-I"]).arg(d.path().join("sw")).arg(&c).output().expect("cc");
    assert!(o.status.success(), "{}", text(&o));
}

/// The register map opens with the OFS AFU feature header block at words 0 to 4, every word distinct and inside the
/// 15-bit MMIO word space, and the shell's CSR and testbench use the same words.
#[test]
fn the_register_map_opens_with_the_ofs_header() {
    let words: Vec<(&str, u32)> = REGISTERS.iter().map(|r| (r.name, r.word)).collect();
    assert_eq!(&words[..5], &[("DFH", 0), ("ID_L", 1), ("ID_H", 2), ("RSVD0", 3), ("RSVD1", 4)]);
    let mut seen: Vec<u32> = words.iter().map(|w| w.1).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), words.len(), "every register its own word");
    assert!(seen.iter().all(|&w| w < 1 << 15));
    let d = emit(&AfuConfig::d5005());
    let csr = std::fs::read_to_string(rtl(d.path(), "logos_csr.v")).unwrap();
    let tb = std::fs::read_to_string(fixture("tb_shell.v")).unwrap();
    for (name, word) in words {
        assert!(csr.contains(&format!("{name} = {word}")), "the CSR's {name}");
        if !["RSVD0", "RSVD1", "ID_L", "ID_H"].contains(&name) {
            assert!(tb.contains(&format!("{name} = {word}")), "the testbench's {name}");
        }
    }
}

/// Every FIFO crossing is constrained as the FIM constrains its own: each Gray pointer into its first synchronizer and
/// the RAM into the read register under the skew, net-delay and relaxed setup/hold of `logos_crossing`, each
/// synchronizer's two registers under the net delay of `logos_synchronizer`, on both FIFO entities; and every register
/// the constraints name is one the FIFO declares.
#[test]
fn the_constraints_hold_every_fifo_crossing() {
    let d = emit(&AfuConfig::d5005());
    let sdc = std::fs::read_to_string(rtl(d.path(), "logos_d5005.sdc")).unwrap();
    let fifo = std::fs::read_to_string(rtl(d.path(), "logos_dcfifo.v")).unwrap();
    assert!(sdc.contains("foreach entity {logos_dcfifo_mlab logos_dcfifo_m20k}"));
    let name = |part: &str| -> String { part.split("$inst|").nth(1).unwrap().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect() };
    let mut applied: Vec<(String, String, String)> = sdc
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("logos_crossing [") || l.starts_with("logos_synchronizer ["))
        .map(|l| {
            let (proc_, rest) = l.split_once(' ').unwrap();
            let (from, to) = rest.split_once("] [").unwrap();
            (proc_.to_string(), name(from), name(to))
        })
        .collect();
    applied.sort();
    let want = [
        ("logos_crossing", "mem", "rdata_q"),
        ("logos_crossing", "rgray", "rgray_s1"),
        ("logos_crossing", "wgray", "wgray_s1"),
        ("logos_synchronizer", "rgray_s1", "rgray_s2"),
        ("logos_synchronizer", "wgray_s1", "wgray_s2"),
    ];
    assert_eq!(applied, want.map(|(p, f, t)| (p.to_string(), f.to_string(), t.to_string())), "the five constraints of a FIFO");
    let body = |proc_: &str| -> String { sdc.split(&format!("proc {proc_} ")).nth(1).unwrap().split("\n}\n").next().unwrap().to_string() };
    let crossing = body("logos_crossing");
    for c in ["set_max_skew", "-get_skew_value_from_clock_period src_clock_period -skew_value_multiplier 0.8", "set_net_delay",
              "-get_value_from_clock_period dst_clock_period -value_multiplier 0.8", "set_max_delay -from $from -to $to 100",
              "set_min_delay -from $from -to $to -100"] {
        assert!(crossing.contains(c), "logos_crossing applies {c}");
    }
    assert!(body("logos_synchronizer").contains("set_net_delay"), "logos_synchronizer holds the net delay");
    for (_, from, to) in want {
        for n in [from, to] {
            let declared = fifo.lines().any(|l| l.contains("reg") && l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == n));
            assert!(declared, "the FIFO declares {n}");
        }
    }
}

// ── The recorder ────────────────────────────────────────────────────────────────────────────────────

/// A compiled tree as afu_synth leaves it, with the gates' outputs beside it.
fn built(d: &Path, fail_summary: &str, fit_status: &str, uclk: bool, gates: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let tree = d.join("tree");
    let q = tree.join("build/syn/syn_top/output_files");
    std::fs::create_dir_all(q.join("timing_report")).unwrap();
    std::fs::write(q.join("iofs_pr_afu.persona1.gbs"), b"a partial bitstream").unwrap();
    std::fs::write(q.join("iofs_pr_afu.fit.summary"), format!("Fitter Status : {fit_status}\nFamily : Stratix 10\nDevice : 1SX280HN2F43E2VG\n")).unwrap();
    std::fs::write(
        q.join("timing_report/clocks.sta.pass.summary"),
        "Type  : Slow_vid2_100C setup 'uClk_usr'\nSlack : 0.042\nTNS   : 0.000\n\nType  : Fast_vid2_0C hold 'uClk_usr'\nSlack : 0.011\nTNS   : 0.000\n\n",
    )
    .unwrap();
    std::fs::write(q.join("timing_report/clocks.sta.fail.summary"), fail_summary).unwrap();
    if uclk {
        std::fs::write(q.join("user_clock_freq.txt"), "# computed\nafu-image/clock-frequency-high:437.5\nafu-image/clock-frequency-low:218.75\n").unwrap();
    }
    std::fs::create_dir_all(tree.join("hw/rtl")).unwrap();
    let emitted = emit(&AfuConfig::d5005());
    std::fs::copy(rtl(emitted.path(), "logos_d5005.json"), tree.join("hw/rtl/logos_d5005.json")).unwrap();
    let gates_dir = d.join("gates");
    std::fs::create_dir_all(&gates_dir).unwrap();
    for (name, line) in gates {
        std::fs::write(gates_dir.join(name), format!("{line}\n")).unwrap();
    }
    (tree, gates_dir)
}

const GATES_PASS: [(&str, &str); 5] = [
    ("cdc_shell.txt", "logos_d5005_shell: clean; 125 registers, 23 crossings, 6 clocks"),
    ("cdc_fifo_mlab.txt", "logos_dcfifo_mlab: clean; 10 registers"),
    ("cdc_fifo_m20k.txt", "logos_dcfifo_m20k: clean; 10 registers"),
    ("sv_check.txt", "ofs_plat_afu: elaborates against the PIM of tools/d5005/pr-template, 6 files of its own"),
    ("ram_check.txt", "logos_d5005_shell: clean; 114 memories, 11 of 65536 bits or more: 3 block-RAM wrappers', 8 their modules' own"),
];

fn record(tree: &Path, gates: &Path, ledger: &Path) -> Output {
    Command::new("python3").arg(script("record_afu_image.py")).arg(tree).arg(gates).arg(ledger).output().expect("python3")
}

/// An image whose fit, timing and gates passed is recorded with its hash, UUID, user clocks and least slacks; the same
/// image again is refused.
#[test]
fn the_recorder_records_a_passing_image_once() {
    if !common::require_tool("python3") {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let (tree, gates) = built(d.path(), "", "Successful - Tue Oct 21 2026", true, &GATES_PASS);
    let ledger = d.path().join("afu_images.json");
    let o = record(&tree, &gates, &ledger);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
    let row = &l["images"][0];
    assert_eq!(row["afu_uuid"], AfuConfig::d5005().uuid_text());
    assert_eq!(row["user_clock_mhz"]["high"], 437.5);
    assert_eq!(row["least_slack_ns"].as_array().unwrap().len(), 2);
    assert_eq!(row["gates"].as_object().unwrap().len(), GATES_PASS.len());
    let o = record(&tree, &gates, &ledger);
    assert_eq!(o.status.code(), Some(1), "a second record of one image: {}", text(&o));
    assert!(text(&o).contains("already holds"), "{}", text(&o));
}

/// The recorder refuses an image whose timing fails, whose fit failed, whose user clocks are missing, or one of whose
/// gates did not pass.
#[test]
fn the_recorder_refuses_each_failure() {
    if !common::require_tool("python3") {
        return;
    }
    let failing = "Type  : Slow_vid2_100C setup 'uClk_usr'\nSlack : -0.120\nTNS   : -4.200\n\n";
    let mut one_gate_failed = GATES_PASS;
    one_gate_failed[0] = ("cdc_shell.txt", "logos_d5005_shell: 3 paths cross clocks outside a declared crossing:");
    for (what, fail, fit, uclk, gates, says) in [
        ("timing", failing, "Successful - Tue Oct 21 2026", true, GATES_PASS, "fail timing, worst uClk_usr setup -0.12"),
        ("fit", "", "Failed - Tue Oct 21 2026", true, GATES_PASS, "the fitter did not succeed"),
        ("user clock", "", "Successful - Tue Oct 21 2026", false, GATES_PASS, "no user clock record"),
        ("gate", "", "Successful - Tue Oct 21 2026", true, one_gate_failed, "the gate cdc_shell.txt did not pass"),
    ] {
        let d = tempfile::tempdir().unwrap();
        let (tree, gates_dir) = built(d.path(), fail, fit, uclk, &gates);
        let ledger = d.path().join("afu_images.json");
        let o = record(&tree, &gates_dir, &ledger);
        assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
        assert!(text(&o).contains(says), "{what}: {}", text(&o));
        assert!(!ledger.exists(), "{what}: nothing recorded");
    }
}

/// An image is recorded only when its RAM gate passed: without the gate's output, or with a large memory written
/// outside a block-RAM wrapper, it is refused, naming the gate; with a passing one it is recorded with it.
#[test]
fn the_recorder_refuses_an_image_whose_ram_gate_did_not_pass() {
    if !common::require_tool("python3") {
        return;
    }
    let others: Vec<(&str, &str)> = GATES_PASS.iter().copied().filter(|(name, _)| *name != "ram_check.txt").collect();
    let failed = ("ram_check.txt", "logos_d5005_shell: 4 memories of 65536 bits or more in a generate block, outside a block-RAM wrapper:");
    let passed = GATES_PASS.iter().copied().find(|(name, _)| *name == "ram_check.txt").expect("a passing RAM gate");
    for (what, ram, refused) in [("no RAM gate", None, true), ("a failing one", Some(failed), true), ("a passing one", Some(passed), false)] {
        let d = tempfile::tempdir().unwrap();
        let mut gates = others.clone();
        gates.extend(ram);
        let (tree, gates_dir) = built(d.path(), "", "Successful - Tue Oct 25 2026", true, &gates);
        let ledger = d.path().join("afu_images.json");
        let o = record(&tree, &gates_dir, &ledger);
        if refused {
            assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
            assert!(text(&o).contains("the gate ram_check.txt did not pass"), "{what}: {}", text(&o));
            assert!(!ledger.exists(), "{what}: nothing recorded");
        } else {
            assert_eq!(o.status.code(), Some(0), "{what}: {}", text(&o));
            let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
            assert_eq!(l["images"][0]["gates"]["ram_check.txt"], passed.1, "{what}");
        }
    }
}

/// A hybrid engine's image is recorded only when its bridge's gate passed as well as the shell's: the tree's clock-domain
/// map, as the hybrid build emits it, declares the crossing; without the bridge's output, or with a failing one, the
/// image is refused, and with a passing one it is recorded with every other gate and the bridge's.
#[test]
fn the_recorder_refuses_a_hybrid_image_whose_bridge_gate_did_not_pass() {
    if !common::require_tool("python3") {
        return;
    }
    let emitted = emit(&AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512));
    let bridge_failed = ("cdc_bridge.txt", "logos_engine_bridge: 1 paths cross clocks outside a declared crossing:");
    let bridge_passed = ("cdc_bridge.txt", "logos_engine_bridge: clean; 13 registers, engine-side values only through cap_/wcap_/held_ registers");
    for (what, extra, says) in [("no bridge gate", None, Some("the gate cdc_bridge.txt did not pass")), ("a failing one", Some(bridge_failed), Some("the gate cdc_bridge.txt did not pass")), ("a passing one", Some(bridge_passed), None)] {
        let d = tempfile::tempdir().unwrap();
        let mut gates: Vec<(&str, &str)> = GATES_PASS.to_vec();
        gates.extend(extra);
        let (tree, gates_dir) = built(d.path(), "", "Successful - Tue Oct 22 2026", true, &gates);
        std::fs::copy(emitted.path().join(CLOCK_DOMAINS_FILE), tree.join(CLOCK_DOMAINS_FILE)).unwrap();
        let ledger = d.path().join("afu_images.json");
        let o = record(&tree, &gates_dir, &ledger);
        match says {
            Some(says) => {
                assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
                assert!(text(&o).contains(says), "{what}: {}", text(&o));
                assert!(!ledger.exists(), "{what}: nothing recorded");
            }
            None => {
                assert_eq!(o.status.code(), Some(0), "{what}: {}", text(&o));
                let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
                assert_eq!(l["images"][0]["gates"].as_object().unwrap().len(), GATES_PASS.len() + 1, "{what}");
            }
        }
    }
}

/// A flat build as build-afu.sh --flat leaves it: the accelerator's tree, and the FIM's work tree with its SOF, its fit
/// summary and its OFS timing report.
fn built_flat(d: &Path, fail_summary: &str, fit_status: &str, sof: bool) -> (PathBuf, PathBuf) {
    let afu = d.join("afu");
    std::fs::create_dir_all(afu.join("hw/rtl")).unwrap();
    let emitted = emit(&AfuConfig::d5005());
    std::fs::copy(rtl(emitted.path(), "logos_d5005.json"), afu.join("hw/rtl/logos_d5005.json")).unwrap();
    let fim = d.join("fim");
    let q = fim.join("syn/syn_top/output_files");
    std::fs::create_dir_all(q.join("timing_report")).unwrap();
    if sof {
        std::fs::write(q.join("d5005.sof"), b"the FIM with the accelerator inside it").unwrap();
    }
    std::fs::write(q.join("d5005.fit.summary"), format!("Fitter Status : {fit_status}\nFamily : Stratix 10\nDevice : 1SX280HN2F43E2VG\n")).unwrap();
    std::fs::write(
        q.join("timing_report/clocks.sta.pass.summary"),
        "Type  : Slow_vid2_100C setup 'uClk_usr'\nSlack : 0.031\nTNS   : 0.000\n\nType  : Slow_vid2_100C setup 'SYS_REFCLK'\nSlack : 1.204\nTNS   : 0.000\n\n",
    )
    .unwrap();
    std::fs::write(q.join("timing_report/clocks.sta.fail.summary"), fail_summary).unwrap();
    (afu, fim)
}

fn record_flat(afu: &Path, fim: &Path, gates: &Path, ledger: &Path) -> Output {
    Command::new("python3").arg(script("record_afu_image.py")).arg("--flat").arg(afu).arg(fim).arg(gates).arg(ledger).output().expect("python3")
}

/// A flat image — the FIM built with the accelerator inside it — whose fit, timing and gates passed is recorded as
/// kind flat with its SOF's hash and the user clock its AFU JSON asks for (set on the card at run time), beside a
/// persona recorded as kind pr; the same image again is refused, and so is one with no SOF, a failed fit, failing
/// timing or a gate that did not pass.
#[test]
fn the_recorder_records_a_flat_image_and_refuses_its_failures() {
    if !common::require_tool("python3") {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let ledger = d.path().join("afu_images.json");
    let (afu, fim) = built_flat(&d.path().join("flat"), "", "Successful - Tue Oct 21 2026", true);
    let (tree, gates) = built(&d.path().join("pr"), "", "Successful - Tue Oct 21 2026", true, &GATES_PASS);
    let o = record_flat(&afu, &fim, &gates, &ledger);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let o = record(&tree, &gates, &ledger);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
    let (flat, pr) = (&l["images"][0], &l["images"][1]);
    assert_eq!((flat["kind"].as_str(), pr["kind"].as_str()), (Some("flat"), Some("pr")));
    assert_eq!(flat["image"], "syn/syn_top/output_files/d5005.sof");
    assert_eq!(flat["afu_uuid"], AfuConfig::d5005().uuid_text());
    assert_eq!(flat["user_clock_asked"], format!("auto-{}", AfuConfig::d5005().user_clock_mhz));
    assert!(flat.get("user_clock_mhz").is_none() && pr["user_clock_mhz"]["high"] == 437.5);
    assert_eq!(flat["least_slack_ns"].as_array().unwrap().len(), 2);
    let o = record_flat(&afu, &fim, &gates, &ledger);
    assert_eq!(o.status.code(), Some(1), "a second record of one image: {}", text(&o));
    assert!(text(&o).contains("already holds"), "{}", text(&o));

    let failing = "Type  : Slow_vid2_100C setup 'uClk_usr'\nSlack : -0.050\nTNS   : -1.000\n\n";
    let mut one_gate_failed = GATES_PASS;
    one_gate_failed[3] = ("sv_check.txt", "ofs_plat_afu: 2 problems elaborating against the PIM:");
    for (what, fail, fit, sof, gate_lines, says) in [
        ("no SOF", "", "Successful - Tue Oct 21 2026", false, GATES_PASS, "no image at"),
        ("fit", "", "Failed - Tue Oct 21 2026", true, GATES_PASS, "the fitter did not succeed"),
        ("timing", failing, "Successful - Tue Oct 21 2026", true, GATES_PASS, "fail timing, worst uClk_usr setup -0.05"),
        ("gate", "", "Successful - Tue Oct 21 2026", true, one_gate_failed, "the gate sv_check.txt did not pass"),
    ] {
        let e = tempfile::tempdir().unwrap();
        let (afu, fim) = built_flat(e.path(), fail, fit, sof);
        let gates = e.path().join("gates");
        std::fs::create_dir_all(&gates).unwrap();
        for (name, line) in gate_lines {
            std::fs::write(gates.join(name), format!("{line}\n")).unwrap();
        }
        let fresh = e.path().join("afu_images.json");
        let o = record_flat(&afu, &fim, &gates, &fresh);
        assert_eq!(o.status.code(), Some(1), "{what}: {}", text(&o));
        assert!(text(&o).contains(says), "{what}: {}", text(&o));
        assert!(!fresh.exists(), "{what}: nothing recorded");
    }
}

/// Every file under `d`, as its path relative to `d` and its bytes, in path order.
fn tree_files(d: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(d: &Path, at: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(at).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for p in entries {
            let kind = std::fs::symlink_metadata(&p).unwrap().file_type();
            if kind.is_dir() {
                walk(d, &p, out);
            } else if kind.is_symlink() {
                out.push((p.strip_prefix(d).unwrap().to_path_buf(), std::fs::read_link(&p).unwrap().into_os_string().into_encoded_bytes()));
            } else {
                out.push((p.strip_prefix(d).unwrap().to_path_buf(), std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(d, d, &mut out);
    out
}

/// The Quartus assignments a Tcl or QSF text makes, its comments and blank lines left out.
fn assignments(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| l.starts_with("set_global_assignment") || l.starts_with("set_instance_assignment")).collect()
}

/// A flat image's source tree (`scripts/d5005/flat-fim.sh`) is the FIM's release as it ships — ofs-d5005 at
/// ofs-2024.1-1 with ofs-common at its submodule's commit, and the PIM's ofs-platform-afu-bbb — but for the base
/// revision's floorplan, which assigns nothing: no partial-reconfiguration revision, partition or flag, and no place
/// or route region for afu_main, where the release reserves the persona region (601,050 ALMs, 3,971 DSP blocks). So
/// the accelerator the FIM's afu_main instantiates through the PIM is placed over the whole device beside the FIM's
/// static logic. No other assignment file of the base revision names a partial-reconfiguration partition or a region
/// for afu_main, and every other file is byte for byte the release's.
#[test]
fn a_flat_image_gives_the_accelerator_the_whole_device() {
    if !common::require_tool("tar") || !common::require_tool("d5005-pim") {
        return;
    }
    let dl = root().join("tools/d5005/dl");
    let d = tempfile::tempdir().unwrap();
    let ofs = d.path().join("ofs");
    let o = Command::new("bash").arg(script("flat-fim.sh")).arg(&ofs).output().expect("bash");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let (fim, bbb) = (ofs.join("ofs-d5005-ofs-2024.1-1"), ofs.join("ofs-platform-afu-bbb-ofs-2024.1-1"));
    assert!(text(&o).contains(&format!("the FIM's source tree is {}, the PIM {}", fim.display(), bbb.display())), "{}", text(&o));

    let release = d.path().join("release");
    std::fs::create_dir_all(&release).unwrap();
    let untar = |archive: &str| {
        let o = Command::new("tar").arg("-xzf").arg(dl.join(archive)).arg("-C").arg(&release).output().expect("tar");
        assert!(o.status.success(), "{archive}: {}", text(&o));
    };
    untar("ofs-d5005-ofs-2024.1-1.tar.gz");
    untar("ofs-fim-common-374fd3ccb014e31c0b1fad207b1299e8081e4fd9.tar.gz");
    untar("ofs-platform-afu-bbb-ofs-2024.1-1.tar.gz");
    let shipped = release.join("ofs-d5005-ofs-2024.1-1");
    std::fs::remove_dir_all(shipped.join("ofs-common")).unwrap();
    std::fs::rename(release.join("ofs-fim-common-374fd3ccb014e31c0b1fad207b1299e8081e4fd9"), shipped.join("ofs-common")).unwrap();

    let floorplan = Path::new("syn/setup/sr_logic_lock_region.tcl");
    let theirs = std::fs::read_to_string(shipped.join(floorplan)).unwrap();
    let theirs = assignments(&theirs);
    for a in [
        "-name REVISION_TYPE PR_BASE",
        "-name PARTITION persona1 -to afu_top|port_gasket|pr_slot|afu_main",
        "-name PARTIAL_RECONFIGURATION_PARTITION ON -to afu_top|port_gasket|pr_slot|afu_main",
        "-name PLACE_REGION ",
        "-name ROUTE_REGION ",
        "-name RESERVE_PLACE_REGION ON -to afu_top|port_gasket|pr_slot|afu_main",
        "-name CORE_ONLY_PLACE_REGION ON -to afu_top|port_gasket|pr_slot|afu_main",
    ] {
        assert!(theirs.iter().any(|l| l.contains(a)), "the release's floorplan assigns {a}: {theirs:?}");
    }
    assert_eq!(theirs.len(), 7, "the release's floorplan assigns those seven and nothing else: {theirs:?}");
    let ours = std::fs::read_to_string(fim.join(floorplan)).unwrap();
    assert_eq!(assignments(&ours), Vec::<&str>::new(), "a flat image's floorplan assigns nothing: {ours}");

    let base = tree_files(&fim);
    for (path, bytes) in &base {
        let name = path.file_name().unwrap().to_string_lossy();
        let assigning = name.ends_with(".tcl") || name.ends_with(".qsf");
        if !assigning || name.starts_with("iofs_pr_afu") {
            continue;
        }
        for l in assignments(&String::from_utf8_lossy(bytes)) {
            for pr in ["PARTIAL_RECONFIGURATION_PARTITION", "REVISION_TYPE PR_BASE", "PARTITION persona1", "RESERVE_PLACE_REGION", "CORE_ONLY_PLACE_REGION"] {
                assert!(!l.contains(pr), "{}: {l}", path.display());
            }
            if l.contains("afu_main") {
                assert!(!l.contains("PLACE_REGION") && !l.contains("ROUTE_REGION"), "{}: {l}", path.display());
            }
        }
    }
    let release_base = tree_files(&shipped);
    let ours: Vec<_> = base.iter().filter(|(p, _)| p != floorplan).collect();
    let theirs: Vec<_> = release_base.iter().filter(|(p, _)| p != floorplan).collect();
    assert_eq!(ours.len(), theirs.len(), "the flat tree holds the release's files");
    for ((p, a), (q, b)) in ours.iter().zip(&theirs) {
        assert_eq!(p, q, "the flat tree holds the release's files");
        assert!(a == b, "{} is the release's", p.display());
    }
    assert!(tree_files(&bbb) == tree_files(&release.join("ofs-platform-afu-bbb-ofs-2024.1-1")), "the PIM is the release's");
}

/// What a flat image's Quartus project assigns, as `fixtures/d5005_flow/quartus_assignments.tcl` (Quartus's assignment
/// commands stood in for in Tcl) reads its `d5005.qsf` from the FIM's project directory, laid as the FIM's setup stage
/// lays it: the build's environment (`BUILD_ROOT_REL`, exported by `build_var_setup_common.sh`, and the
/// `build_env_db.txt` that `config_env.tcl` loads), and the PIM-wrapped AFU the setup configures from `sources`
/// (`afu_with_pim/afu.tcl` as `ofs_pim_and_afu_config.sh` writes it, its `afu.qsf` as `afu_synth_setup` has OPAE's
/// rtl_src_config write it, OFS_ROOTDIR the FIM's tree). The PIM's own sources (`pim.tcl`, generated from the platform)
/// are left out: nothing the FIM's static region instantiates is the PIM's. One record a line, its fields tab-separated.
fn flat_project(fim: &Path, sources: &Path) -> Vec<Vec<String>> {
    let top = fim.join("syn/syn_top");
    std::fs::write(top.join("build_env_db.txt"), "BUILD_ROOT_REL=../..\nQ_REVISION=d5005\n").unwrap();
    let config_env = fim.join("syn/setup/config_env.tcl");
    if !config_env.exists() {
        std::os::unix::fs::symlink(fim.join("ofs-common/scripts/common/syn/config_env.tcl"), &config_env).unwrap();
    }
    let hw = top.join("afu_with_pim/afu/hw");
    std::fs::create_dir_all(&hw).unwrap();
    std::fs::write(
        top.join("afu_with_pim/afu.tcl"),
        "# Import the AFU\nset_global_assignment -name SEARCH_PATH afu_with_pim/afu/hw\nset_global_assignment -name SOURCE_TCL_SCRIPT_FILE afu_with_pim/afu/hw/afu.qsf\n",
    )
    .unwrap();
    let o = Command::new("python3")
        .arg(root().join("tools/d5005/opae-sdk/platforms/platmgr/tools/rtl_src_config.py"))
        .args(["--qsf", "--abs"])
        .arg(sources)
        .env("OFS_ROOTDIR", fim)
        .output()
        .expect("python3");
    assert!(o.status.success(), "{}", text(&o));
    std::fs::write(hw.join("afu.qsf"), &o.stdout).unwrap();
    let o = Command::new("tclsh")
        .arg(fixture("quartus_assignments.tcl"))
        .arg("d5005.qsf")
        .current_dir(&top)
        .env("BUILD_ROOT_REL", "../..")
        .output()
        .expect("tclsh");
    assert!(o.status.success(), "the project's settings evaluate: {}", text(&o));
    String::from_utf8(o.stdout).unwrap().lines().map(|l| l.split('\t').map(str::to_string).collect()).collect()
}

/// A Verilog token, as the scan for declarations and instances needs it.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// An identifier, an escaped one without its backslash.
    Id(String),
    /// A compiler directive or macro use, by its name (`ifdef` for `` `ifdef ``).
    Directive(String),
    /// Any other character, alone.
    Punct(char),
}

/// The tokens of a Verilog text a compilation keeps with `defined` macros set: comments, strings and numbers dropped,
/// `` `ifdef ``/`` `ifndef ``/`` `elsif ``/`` `else ``/`` `endif `` honoured, `` `define `` and `` `undef `` updating
/// `defined` where they are kept, and every other directive's line (`` `define ``'s body, `` `include ``,
/// `` `timescale ``) passed over.
fn compiled_tokens(text: &str, defined: &mut std::collections::BTreeSet<String>) -> Vec<Token> {
    let c: Vec<char> = text.chars().collect();
    let ident = |ch: char| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$';
    // Each open conditional: whether its enclosing text is kept, whether one of its branches was taken, and whether
    // the branch it is in is kept.
    let mut open: Vec<(bool, bool, bool)> = Vec::new();
    let kept = |open: &[(bool, bool, bool)]| open.last().map_or(true, |&(_, _, k)| k);
    let mut out = Vec::new();
    let mut i = 0;
    let word = |i: &mut usize| {
        while *i < c.len() && c[*i].is_whitespace() {
            *i += 1;
        }
        let s = *i;
        while *i < c.len() && ident(c[*i]) {
            *i += 1;
        }
        c[s..*i].iter().collect::<String>()
    };
    let to_line_end = |i: &mut usize| {
        while *i < c.len() && c[*i] != '\n' {
            if c[*i] == '\\' && *i + 1 < c.len() && c[*i + 1] == '\n' {
                *i += 1;
            }
            *i += 1;
        }
    };
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if ch == '"' {
            i += 1;
            while i < c.len() && c[i] != '"' {
                i += if c[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else if ch == '`' {
            i += 1;
            let name = word(&mut i);
            match name.as_str() {
                "ifdef" | "ifndef" => {
                    let m = word(&mut i);
                    let cond = defined.contains(&m) == (name == "ifdef");
                    let outer = kept(&open);
                    open.push((outer, cond, outer && cond));
                }
                "elsif" => {
                    let m = word(&mut i);
                    let (outer, taken, _) = open.pop().expect("an `elsif inside a conditional");
                    let cond = !taken && defined.contains(&m);
                    open.push((outer, taken || cond, outer && cond));
                }
                "else" => {
                    let (outer, taken, _) = open.pop().expect("an `else inside a conditional");
                    open.push((outer, true, outer && !taken));
                }
                "endif" => {
                    open.pop().expect("an `endif closing a conditional");
                }
                "define" | "undef" => {
                    let m = word(&mut i);
                    if kept(&open) {
                        if name == "define" {
                            defined.insert(m);
                        } else {
                            defined.remove(&m);
                        }
                    }
                    to_line_end(&mut i);
                }
                "include" | "timescale" | "default_nettype" | "resetall" | "celldefine" | "endcelldefine" | "pragma" | "line" => {
                    to_line_end(&mut i)
                }
                _ => {
                    if kept(&open) {
                        out.push(Token::Directive(name));
                    }
                }
            }
        } else if ch == '\\' {
            let s = i + 1;
            i = s;
            while i < c.len() && !c[i].is_whitespace() {
                i += 1;
            }
            if kept(&open) {
                out.push(Token::Id(c[s..i].iter().collect()));
            }
        } else if ch.is_ascii_digit() || (ch == '\'' && c.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric())) {
            while i < c.len() && (ident(c[i]) || c[i] == '\'' || c[i] == '?') {
                i += 1;
            }
        } else if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let s = i;
            while i < c.len() && ident(c[i]) {
                i += 1;
            }
            if kept(&open) {
                out.push(Token::Id(c[s..i].iter().collect()));
            }
        } else {
            if !ch.is_whitespace() && kept(&open) {
                out.push(Token::Punct(ch));
            }
            i += 1;
        }
    }
    assert!(open.is_empty(), "every conditional closed");
    out
}

const VERILOG_KEYWORDS: &[&str] = &[
    "module", "endmodule", "macromodule", "input", "output", "inout", "wire", "reg", "logic", "bit", "byte", "int", "integer",
    "shortint", "longint", "real", "time", "genvar", "parameter", "localparam", "assign", "always", "always_ff", "always_comb",
    "always_latch", "initial", "final", "begin", "end", "if", "else", "for", "foreach", "while", "do", "repeat", "forever", "case",
    "casez", "casex", "endcase", "default", "generate", "endgenerate", "function", "endfunction", "task", "endtask", "return",
    "typedef", "struct", "union", "enum", "packed", "signed", "unsigned", "import", "export", "package", "endpackage",
    "interface", "endinterface", "modport", "automatic", "static", "const", "var", "ref", "posedge", "negedge", "or", "and",
    "not", "unique", "unique0", "priority", "void", "string", "event", "virtual", "class", "endclass", "extends", "new", "this",
    "super", "null", "type", "iff", "inside", "with", "property", "endproperty", "sequence", "endsequence", "assert", "assume",
    "cover", "program", "endprogram", "supply0", "supply1", "tri", "wand", "wor", "break", "continue",
];

/// The words before a type that begin a declaration, not an instance: `function automatic state_t next(`.
const DECLARING: &[&str] = &[
    "function", "task", "automatic", "static", "virtual", "input", "output", "inout", "ref", "const", "typedef", "localparam",
    "parameter", "import", "export", "var", "return",
];

/// The modules, interfaces, packages and programs a token stream declares.
fn declared_units(tokens: &[Token]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (k, t) in tokens.iter().enumerate() {
        if let Token::Id(w) = t {
            if ["module", "macromodule", "interface", "package", "program"].contains(&w.as_str()) {
                let mut j = k + 1;
                if matches!(tokens.get(j), Some(Token::Id(l)) if l == "automatic" || l == "static") {
                    j += 1;
                }
                if let Some(Token::Id(name)) = tokens.get(j) {
                    out.insert(name.clone());
                }
            }
        }
    }
    out
}

/// The index past the bracket that closes the one opened at `at` (`(` or `[`), or none if it is not one.
fn past_closing(tokens: &[Token], at: usize) -> Option<usize> {
    let (open, close) = match tokens.get(at)? {
        Token::Punct('(') => ('(', ')'),
        Token::Punct('[') => ('[', ']'),
        _ => return None,
    };
    let mut depth = 0;
    for (j, t) in tokens.iter().enumerate().skip(at) {
        if *t == Token::Punct(open) {
            depth += 1;
        } else if *t == Token::Punct(close) {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
    }
    None
}

/// The modules and interfaces a token stream instantiates: `<unit> [#(<parameters>)] <instance> [<ranges>] (`.
fn instantiated_units(tokens: &[Token]) -> std::collections::BTreeSet<String> {
    let keyword = |t: Option<&Token>| matches!(t, Some(Token::Id(w)) if VERILOG_KEYWORDS.contains(&w.as_str()));
    let mut out = std::collections::BTreeSet::new();
    for k in 0..tokens.len() {
        let Token::Id(unit) = &tokens[k] else { continue };
        let declaring = k > 0
            && match &tokens[k - 1] {
                Token::Id(w) => DECLARING.contains(&w.as_str()),
                t => *t == Token::Punct('.') || *t == Token::Punct(':'),
            };
        if keyword(Some(&tokens[k])) || declaring {
            continue;
        }
        let mut j = k + 1;
        if tokens.get(j) == Some(&Token::Punct('#')) {
            match past_closing(tokens, j + 1) {
                Some(n) => j = n,
                None => continue,
            }
        }
        if !matches!(tokens.get(j), Some(Token::Id(_))) || keyword(tokens.get(j)) {
            continue;
        }
        j += 1;
        while tokens.get(j) == Some(&Token::Punct('[')) {
            match past_closing(tokens, j) {
                Some(n) => j = n,
                None => break,
            }
        }
        if tokens.get(j) == Some(&Token::Punct('(')) {
            out.insert(unit.clone());
        }
    }
    out
}

/// The modules the FIM's static region (`afu_top`) instantiates that no file a flat image's project compiles declares —
/// its RTL files' modules and interfaces, and each IP or Platform Designer system by its file's name — compiled with
/// the project's macros; and the scripts its settings name that are not there.
fn undeclared_in_the_static_region(made: &[Vec<String>]) -> (std::collections::BTreeSet<String>, Vec<String>, std::collections::BTreeSet<String>) {
    let of = |kind: &str, name: &str| made.iter().filter(|m| m[0] == kind && m[1] == name).map(|m| m[2].clone()).collect::<Vec<_>>();
    let mut macros: std::collections::BTreeSet<String> = of("global", "VERILOG_MACRO").iter().map(|m| m.split('=').next().unwrap().to_string()).collect();
    let mut units = std::collections::BTreeSet::new();
    let mut declaring: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for f in of("global", "SYSTEMVERILOG_FILE").into_iter().chain(of("global", "VERILOG_FILE")) {
        let text = std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{f}: {e}"));
        for u in declared_units(&compiled_tokens(&text, &mut macros.clone())) {
            declaring.entry(u.clone()).or_default().push(f.clone());
            units.insert(u);
        }
    }
    for f in of("global", "IP_FILE").into_iter().chain(of("global", "QSYS_FILE")) {
        units.insert(Path::new(&f).file_stem().unwrap().to_string_lossy().into_owned());
    }
    let afu_top = declaring.get("afu_top").expect("a file declaring afu_top");
    assert_eq!(afu_top.len(), 1, "one file declares afu_top: {afu_top:?}");
    let instances = instantiated_units(&compiled_tokens(&std::fs::read_to_string(&afu_top[0]).unwrap(), &mut macros));
    let missing = made.iter().filter(|m| m[0] == "missing").map(|m| m[1].clone()).collect();
    (instances.difference(&units).cloned().collect(), missing, instances)
}

/// A flat image's Quartus project compiles a declaration of every module the FIM's static region instantiates. The
/// static `afu_top` instantiates the FIM's exercisers whatever its port holds (`he_lb_top` on PF0 VF0), but the
/// Stratix 10 release's `afu_main.tcl` loads their sources (`afu_main_std_exerciser_design_files.tcl`) only when no
/// PIM-wrapped AFU is configured — Agilex's loads them in any FIM build — so a flat build's sources.txt names them itself,
/// once, as `QI:` under `OFS_ROOTDIR`, which OPAE's rtl_src_config expands to the FIM that builds it and hands to Quartus
/// as a `source` of that FIM's own list. Read here with Quartus's assignment commands stood in for in Tcl
/// (`flat_project`), the whole project's settings evaluated as Quartus evaluates them: with a persona's sources the
/// static region lacks exactly `he_lb_top`; with a flat build's it lacks nothing. The release's settings name one script
/// it does not ship (`ofs-common/src/common/afu_design_files.tcl`), which Quartus passes over (Warning 125092), and
/// nothing else is missing. A persona's sources.txt names nothing of the FIM.
#[test]
fn a_flat_build_compiles_every_module_the_fims_static_region_instantiates() {
    if !common::require_tool("tar")
        || !common::require_tool("d5005-pim")
        || !common::require_tool("tclsh")
        || !common::require_tool("python3")
        || !common::require_tool("opae-sdk")
    {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let o = Command::new("bash").arg(script("flat-fim.sh")).arg(d.path().join("ofs")).output().expect("bash");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let fim = d.path().join("ofs/ofs-d5005-ofs-2024.1-1");
    let exercisers = fim.join("ofs-common/src/common/afu_main_std_exerciser_design_files.tcl");

    let persona = emit(&AfuConfig::d5005());
    let listed = std::fs::read_to_string(rtl(persona.path(), "sources.txt")).unwrap();
    assert!(!listed.contains("OFS_ROOTDIR") && !listed.contains("exerciser"), "a persona's sources name nothing of the FIM: {listed}");
    let (undeclared, missing, instances) = undeclared_in_the_static_region(&flat_project(&fim, &rtl(persona.path(), "sources.txt")));
    for unit in ["afu_intf", "pf_vf_mux_top", "he_lb_top", "port_gasket", "apf", "pcie_ss_axis_if"] {
        assert!(instances.contains(unit), "the static region instantiates {unit}: {instances:?}");
    }
    assert!(!instances.contains("he_lb_err_top"), "the error-injecting exerciser only under ERROR_INJ: {instances:?}");
    assert_eq!(undeclared, ["he_lb_top".to_string()].into(), "with a persona's sources the static region lacks its exerciser");
    let skipped = vec![fim.join("ofs-common/src/common/afu_design_files.tcl").display().to_string()];
    assert_eq!(missing, skipped, "the one script the release names and does not ship");

    let flat = emit(&AfuConfig { flat: true, ..AfuConfig::d5005() });
    let listed = std::fs::read_to_string(rtl(flat.path(), "sources.txt")).unwrap();
    let line = format!("QI:{FIM_STATIC_EXERCISERS}");
    assert_eq!(listed.lines().filter(|l| *l == line).count(), 1, "{listed}");
    assert_eq!(listed.lines().filter(|l| l.contains("OFS_ROOTDIR")).count(), 1, "{listed}");
    let made = flat_project(&fim, &rtl(flat.path(), "sources.txt"));
    let qsf = std::fs::read_to_string(fim.join("syn/syn_top/afu_with_pim/afu/hw/afu.qsf")).unwrap();
    let sourced = format!("source \"{}\"", exercisers.display());
    assert_eq!(qsf.lines().filter(|l| *l == sourced).count(), 1, "the FIM's own list sourced once:\n{qsf}");
    let (undeclared, missing, _) = undeclared_in_the_static_region(&made);
    assert!(undeclared.is_empty(), "the static region lacks {undeclared:?}");
    assert_eq!(missing, skipped);
    let he_lb = fim.join("ofs-common/src/common/he_lb/he_lb_top.sv").display().to_string();
    assert_eq!(made.iter().filter(|m| m[0] == "global" && m[1] == "SYSTEMVERILOG_FILE" && m[2] == he_lb).count(), 1, "he_lb_top compiled once");
}

/// build-afu.sh emits the tree its build compiles: given `--flat`, through `d5005_afu --flat`, so its sources.txt names
/// the FIM's static exercisers (`a_flat_build_compiles_every_module_the_fims_static_region_instantiates`); a persona's
/// tree without it. Run with a cargo on PATH that records the emitter's arguments and stops the build there.
#[test]
fn build_afu_emits_a_flat_builds_tree_flat_and_a_personas_not() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_ARGS\"\nexit 3\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    for (name, flags, emitted) in [
        ("flat", &["data64", "--flat"][..], &["data64", "--flat"][..]),
        ("persona", &["data64"][..], &["data64"][..]),
        ("flat-engine", &["data64", "--divide", "48", "--flat", "--engine", "h.json"][..], &["data64", "--divide", "48", "--flat", "--engine"][..]),
    ] {
        let w = d.path().join(name);
        let recorded = d.path().join(format!("{name}.args"));
        let o = Command::new("bash")
            .arg(script("build-afu.sh"))
            .arg(&w)
            .args(flags)
            .current_dir(d.path())
            .env("PATH", &path)
            .env("CARGO_ARGS", &recorded)
            .output()
            .expect("bash");
        assert_eq!(o.status.code(), Some(3), "{name}: the build stops at its emitter: {}", text(&o));
        let args: Vec<String> = std::fs::read_to_string(&recorded).unwrap().lines().map(str::to_string).collect();
        let mut want: Vec<String> = ["run", "-q", "-p", "logicaffeine-compile", "--example", "d5005_afu", "--"].iter().map(|a| a.to_string()).collect();
        want.push(w.join("afu").display().to_string());
        want.extend(emitted.iter().map(|a| a.to_string()));
        if name == "flat-engine" {
            want.push(d.path().join("h.json").display().to_string());
        }
        assert_eq!(args, want, "{name}");
    }
}

/// build-afu.sh offers exactly the build flags `d5005_afu`'s build form does, and hands each to the emitter as given: its
/// usage and the emitter's name the same flags, each taking a value in both or in neither, and run with every one of them
/// (each value an absolute path of its own, which the script hands on as it is whether it reads it as a path or not), the
/// cargo that records the emitter's arguments sees the build's tree, its line, then every flag followed by its value, in
/// the order given.
#[test]
fn build_afu_offers_and_forwards_every_build_flag_the_emitter_takes() {
    use std::os::unix::fs::PermissionsExt;
    let flags = |usage: &str| -> Vec<(String, bool)> {
        let words: Vec<&str> = usage.split_whitespace().collect();
        let mut out: Vec<(String, bool)> = Vec::new();
        for (i, w) in words.iter().enumerate() {
            if let Some(f) = w.trim_start_matches('[').strip_prefix("--") {
                let f = format!("--{}", f.trim_end_matches(']'));
                let valued = words.get(i + 1).is_some_and(|v| v.trim_start_matches('[').starts_with('<'));
                if !out.iter().any(|(g, _)| *g == f) {
                    out.push((f, valued));
                }
            }
        }
        out
    };
    let script_text = std::fs::read_to_string(script("build-afu.sh")).unwrap();
    let script_usage = script_text.lines().find(|l| l.starts_with("USAGE=")).expect("build-afu.sh's usage");
    let emitter = std::fs::read_to_string(root().join("crates/logicaffeine_compile/examples/d5005_afu.rs")).unwrap();
    let emitter_usage = emitter.lines().find(|l| l.starts_with("const USAGE")).expect("d5005_afu's usage");
    let (offered, taken) = (flags(script_usage), flags(emitter_usage));
    let sorted = |f: &[(String, bool)]| -> Vec<(String, bool)> {
        let mut f = f.to_vec();
        f.sort();
        f
    };
    assert_eq!(sorted(&offered), sorted(&taken), "build-afu.sh's build flags and d5005_afu's");
    assert!(offered.len() >= 26, "the usage's flags: {offered:?}");
    assert_eq!(offered.last().map(|f| f.0.as_str()), Some("--engine"), "the engine's ledgers end the usage");
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_ARGS\"\nexit 3\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut given: Vec<String> = vec!["data64".to_string()];
    for (k, (f, valued)) in offered.iter().enumerate() {
        given.push(f.clone());
        if *valued {
            given.push(d.path().join(format!("value{k}")).display().to_string());
        }
    }
    let w = d.path().join("tree");
    let recorded = d.path().join("args");
    let o = Command::new("bash")
        .arg(script("build-afu.sh"))
        .arg(&w)
        .args(&given)
        .current_dir(d.path())
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()))
        .env("CARGO_ARGS", &recorded)
        .output()
        .expect("bash");
    assert_eq!(o.status.code(), Some(3), "the build stops at its emitter: {}", text(&o));
    let args: Vec<String> = std::fs::read_to_string(&recorded).unwrap().lines().map(str::to_string).collect();
    let at = args.iter().position(|a| a == "--").expect("the emitter's arguments");
    assert_eq!(&args[..at], ["run", "-q", "-p", "logicaffeine-compile", "--example", "d5005_afu"], "the emitter");
    assert_eq!(args[at + 1], w.join("afu").display().to_string(), "the build's tree");
    assert_eq!(args[at + 2..].to_vec(), given, "the emitter's arguments after its tree");
}

/// `d5005_afu --function-units <n>` gives the engine's vector side `<n>` function units (`logos_vector_side`'s `FU`): run
/// on Qwen3.5-0.8B's ledger as a vectored hybrid build, eight gives eight and sixteen sixteen, a build that names none
/// keeps the engine's [`FUNCTION_UNITS`], and a count that does not divide sixteen is refused, naming it. A flag the build
/// form's usage does not name (a misspelt one) is refused before anything is emitted, never passed over.
///
/// [`FUNCTION_UNITS`]: logicaffeine_compile::spatial::decode_engine::rtl::engine::FUNCTION_UNITS
#[test]
fn the_emitter_gives_the_vector_side_the_function_units_its_build_names() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::FUNCTION_UNITS;
    let built = Command::new("cargo").args(["build", "-q", "-p", "logicaffeine-compile", "--example", "d5005_afu"]).current_dir(root()).output().expect("cargo");
    assert!(built.status.success(), "the emitter builds: {}", text(&built));
    let ledger = root().join("benchmarks/silicon/decode/headers/qwen3.5-0.8b-q8_0.json");
    let emit = |units: &[&str]| -> (Output, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let o = Command::new("cargo")
            .args(["run", "-q", "-p", "logicaffeine-compile", "--example", "d5005_afu", "--"])
            .arg(d.path().join("afu"))
            .args(["data64", "--hybrid", "1", "--recurrence", "--vectors"])
            .args(units)
            .arg("--engine")
            .arg(&ledger)
            .current_dir(root())
            .output()
            .expect("cargo");
        (o, d)
    };
    let fu_of = |d: &tempfile::TempDir| -> String {
        let v = std::fs::read_to_string(d.path().join("afu/hw/rtl/logos_engine.v")).expect("the engine");
        let at = v.find("  logos_vector_side #(").expect("the vector side");
        let head = &v[at..at + v[at..].find(" vside(").expect("the vector side's instance")];
        head.split(".FU(").nth(1).and_then(|r| r.split(')').next()).unwrap_or_else(|| panic!("FU in {head}")).to_string()
    };
    for (given, fu) in [(&["--function-units", "8"][..], "8".to_string()), (&["--function-units", "16"][..], "16".to_string()), (&[][..], FUNCTION_UNITS.to_string())] {
        let (o, d) = emit(given);
        assert!(o.status.success(), "{given:?}: {}", text(&o));
        assert_eq!(fu_of(&d), fu, "{given:?}");
    }
    let (o, _) = emit(&["--function-units", "3"]);
    assert!(!o.status.success(), "three function units built");
    assert!(text(&o).contains("--function-units is a divisor of sixteen, not 3"), "{}", text(&o));
    let (o, d) = emit(&["--function-unit", "8"]);
    assert!(!o.status.success(), "a build flag the usage does not name taken");
    assert!(text(&o).contains("--function-unit is no build flag"), "{}", text(&o));
    assert!(!d.path().join("afu/hw/rtl/logos_engine.v").exists(), "a refused build emitted its engine");
}

/// A build given eight function units gives its vector side eight (`logos_vector_side`'s `FU`, its vector unit's), and one
/// given none the engine's four.
#[test]
fn a_build_of_eight_function_units_gives_its_vector_side_eight() {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, EngineParams, FUNCTION_UNITS};
    let fu_of = |v: &str| -> String {
        let at = v.find("  logos_vector_side #(").expect("the vector side");
        let head = &v[at..at + v[at..].find(" vside(").expect("the vector side's instance")];
        head.split(".FU(").nth(1).and_then(|r| r.split(')').next()).unwrap_or_else(|| panic!("FU in {head}")).to_string()
    };
    let p = fast_side_08b();
    assert_eq!(fu_of(&engine_verilog(&p)), FUNCTION_UNITS.to_string());
    let eight = EngineParams { function_units: 8, ..p };
    assert_eq!(fu_of(&engine_verilog(&eight)), "8");
}

/// The RAM gate (`scripts/d5005/ram_check.py <top> <least bits> <source.v>...`) refuses a large memory a build declares
/// in a generate block outside a block-RAM wrapper and names it with its module, width and depth: the vector side's held
/// float-matrix inputs as they once were (a memory a row inside a generate block, 1088 x 512 bits each, which Quartus
/// built in flip-flops behind 1088-way read multiplexers and reported nowhere — hybrid7-flat6 needed 2,120,891 ALMs of
/// the device's 933,120) are refused row by row; the same rows as `logos_bram_w512` wrappers pass, every stack of them a
/// wrapper's, and so does a large memory at its module's own scope with one write and a registered read (Quartus built
/// the fold's, the flush's and the read-out's that way in M20K); memories under the least size pass wherever they are
/// written.
#[test]
fn the_ram_gate_refuses_a_large_memory_declared_in_a_generate_block_outside_a_block_ram_wrapper() {
    if !cdc_tools() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let bram = d.path().join("bram.v");
    std::fs::write(&bram, logicaffeine_compile::spatial::decode_engine::rtl::ram::bram_verilog(&[512])).unwrap();
    let gate = |top: &str| {
        Command::new("python3").arg(script("ram_check.py")).arg(top).arg("65536").arg(fixture("ram_gate.v")).arg(&bram).output().expect("python3")
    };
    let o = gate("ram_gate_scoped");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("ram_gate_scoped: 2 memories of 65536 bits or more in a generate block, outside a block-RAM wrapper:"), "{}", text(&o));
    for row in 0..2 {
        assert!(text(&o).contains(&format!("ram_gate_scoped held[{row}].act: 512 x 1088 = 557056 bits")), "{}", text(&o));
    }
    let o = gate("ram_gate_wrapped");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ram_gate_wrapped: clean; 2 memories, 2 of 65536 bits or more: 2 block-RAM wrappers', 0 their modules' own"), "{}", text(&o));
    let o = gate("ram_gate_owned");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ram_gate_owned: clean; 1 memories, 1 of 65536 bits or more: 0 block-RAM wrappers', 1 their modules' own"), "{}", text(&o));
    let o = gate("ram_gate_small");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ram_gate_small: clean; 3 memories, 0 of 65536 bits or more"), "{}", text(&o));
}

/// A memory `rtl::ram` builds as stacks of powers of two (`logos_bram_w512` a number of 512-word layers deep that is no
/// power of two: the deepest stack its own, each other a `logos_bram_w512_part`) is block-RAM wrappers' memories, every
/// one: the RAM gate counts each part as a wrapper's, none as a module's own, the rows of 1088 words two memories each.
#[test]
fn the_ram_gate_counts_every_stack_of_a_block_ram_wrapper_as_the_wrappers() {
    if !cdc_tools() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let bram = d.path().join("bram.v");
    std::fs::write(&bram, logicaffeine_compile::spatial::decode_engine::rtl::ram::bram_verilog(&[512])).unwrap();
    let o = Command::new("python3").arg(script("ram_check.py")).arg("ram_gate_wrapped").arg("65536").arg(fixture("ram_gate.v")).arg(&bram).output().expect("python3");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("ram_gate_wrapped: clean; 2 memories, 2 of 65536 bits or more: 2 block-RAM wrappers', 0 their modules' own"), "{}", text(&o));
}

/// The RAM gate refuses a memory of two to thirty-two words that asks for neither MLABs nor registers and names it with
/// its module, width, depth and what it asks for: a ring of two 5,280-bit beats read without a register after it, a
/// sixteen-beat line queue and a one-bit table of thirty-two words asking for nothing, and an eight-word delay asking for
/// M20K, where Quartus built each in an M20K block for every forty bits of its width (hybrid9c2: 5,142 blocks for
/// 2.2 Mbit, and a fit that failed routing); a sixty-four-word memory asking for nothing is past the rule. The same
/// memories asking for registers (the ring) and MLABs (the rest, the table with `no_rw_check`) pass. And a memory in
/// MLABs read without a register after it asks for no read-during-write check as well, where Quartus builds it in
/// flip-flops (a probe's 32 x 1,056-bit buffer: 33,797 registers): a four-word queue whose head is read with none and a
/// sixteen-word memory read at a registered address are refused; sixteen-word memories read into a register, on every
/// clock and under an enable (as `logos_ram_ce` and `logos_bram_re` read), pass.
#[test]
fn the_ram_gate_refuses_a_memory_of_thirty_two_words_or_fewer_asking_for_neither_mlabs_nor_registers() {
    if !cdc_tools() {
        return;
    }
    let gate = |top: &str| Command::new("python3").arg(script("ram_check.py")).arg(top).arg("65536").arg(fixture("ram_gate.v")).output().expect("python3");
    let o = gate("ram_gate_shallow");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("ram_gate_shallow: 4 memories of 32 words or fewer ask for neither MLABs nor registers, where Quartus builds each in an M20K block for every forty bits of its width:"),
        "{}",
        text(&o)
    );
    for refused in [
        "ram_gate_shallow ring: 5280 x 2, asking for nothing",
        "ram_gate_shallow delay: 1184 x 8, asking for M20K",
        "ram_gate_shallow lines: 512 x 16, asking for nothing",
        "ram_gate_shallow flags: 1 x 32, asking for nothing",
    ] {
        assert!(text(&o).contains(refused), "{refused}: {}", text(&o));
    }
    assert!(!text(&o).contains("deep"), "a memory of sixty-four words is past the rule: {}", text(&o));
    let o = gate("ram_gate_shallow_kept");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("ram_gate_shallow_kept: clean; 5 memories, 0 of 65536 bits or more: 0 block-RAM wrappers', 0 their modules' own; 4 of 32 words or fewer, each in MLABs or registers"),
        "{}",
        text(&o)
    );
    let o = gate("ram_gate_unchecked");
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(
        text(&o).contains("ram_gate_unchecked: 2 memories in MLABs read without a register after them ask for a read-during-write check, where Quartus builds each in flip-flops:"),
        "{}",
        text(&o)
    );
    for refused in ["ram_gate_unchecked queue: 4 words, asking for MLAB", "ram_gate_unchecked lines: 16 words, asking for MLAB"] {
        assert!(text(&o).contains(refused), "{refused}: {}", text(&o));
    }
    assert!(!text(&o).contains("kept") && !text(&o).contains("held"), "a memory read into a register is an MLAB's own: {}", text(&o));
}

/// The RAM gate over each build's whole shell at Qwen3.5-0.8B's widths, its every kind of unit: the reference engine's,
/// the hybrid's, every unit on the fast side, head lanes a window behind and with contexts. Every memory of thirty-two
/// words or fewer in it asks for MLABs or registers, so none is left to Quartus, which builds such a memory in an M20K
/// block for every forty bits of its width.
#[test]
fn every_builds_memories_of_thirty_two_words_or_fewer_ask_for_mlabs_or_registers() {
    if !cdc_tools() {
        return;
    }
    let builds = [
        ("the reference engine", AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512)),
        ("the hybrid", AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512)),
        ("every unit on the fast side", AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512)),
        ("head lanes a window behind", AfuConfig::engine(trailing_lanes_08b(), LocalMemoryLine::Data512)),
        ("contexts on head lanes", AfuConfig::engine(contexts_lanes_08b(), LocalMemoryLine::Data512)),
    ];
    let mut wrong = Vec::new();
    for (what, cfg) in builds {
        let d = emit(&cfg);
        let mut cmd = Command::new("python3");
        cmd.arg(script("ram_check.py")).arg("logos_d5005_shell").arg("65536");
        for f in ["logos_dcfifo.v", "logos_secded.v", "logos_stream.v", "logos_csr.v", "logos_bringup_core.v", "logos_engine.v", "logos_card_port.v", "logos_compute_clock.v", "logos_d5005_shell.v"] {
            if rtl(d.path(), f).exists() {
                cmd.arg(rtl(d.path(), f));
            }
        }
        let o = cmd.output().expect("python3");
        if o.status.code() != Some(0) || !text(&o).contains("each in MLABs or registers") {
            wrong.push(format!("{what}:\n{}", text(&o)));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// hybrid8's engine as `d5005_afu` builds it for the card: Qwen3.8-27B with Qwen3.5-0.8B at the fastest point, sixteen
/// token-rows over a greedy store, four pairs a channel, Q8_0 in octets and every block in a 16-byte slot, the recurrence
/// at a binary16 line a clock on two head lanes, the vector and attention sides.
fn hybrid8() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::machine::{lower, Layout, StoreLayout};
    use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
    use logicaffeine_compile::spatial::decode_engine::program;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let n = EngineNumerics::fastest();
    let (mut engines, mut formats, mut log_lines) = (Vec::new(), Vec::<GgmlType>::new(), 0);
    for ledger in ["qwen3.8-27b-q4_k_m.json", "qwen3.5-0.8b-q8_0.json"] {
        let header = GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers").join(ledger)).unwrap();
        let spec = header.spec().unwrap();
        let steps = program::lower(&spec, n, spec.ctx_len).unwrap().program;
        let layout = Layout::of(&spec, n, &steps, &header, 16, spec.ctx_len.min(4096)).unwrap();
        let jobs = lower(&steps, &layout, spec.rms_eps as f32);
        formats.extend(layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty));
        log_lines = log_lines.max(layout.log_lines());
        engines.push(EngineParams::of(&layout, &StoreLayout::greedy(&jobs, &layout, 16), &jobs));
    }
    let set = ColumnSet::hybrid(&formats, n.activations).with_octets().with_slots(16);
    let image = ImageLayout { stripe: Some(4), octets: true, slot: 16, ..ImageLayout::d5005(Beat::Data64) };
    let streaming = Streaming::new(set, image)
        .with_recurrence()
        .with_recurrence_width(2)
        .with_recurrence_lanes(2, log_lines)
        .with_vectors()
        .with_attention();
    EngineParams { streaming: Some(streaming), ..EngineParams::union(&engines).unwrap() }
}

/// hybrid9's engine at the card's shape: hybrid8's, with two sequencer contexts sharing its units, its array's cores
/// paired on processing elements in a clock of twice theirs, and an octet's blocks on two lanes a row.
fn hybrid9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let p = hybrid8();
    let streaming = p.streaming.clone().map(|s| Streaming { set: s.set.clone().with_octet_lanes(2).with_pe_pairs(), ..s }.with_contexts(2));
    EngineParams { streaming, ..p }
}

/// The parameters `module`'s first instance in `verilog` is given, each `.NAME(VALUE)` of its `#(...)`.
fn instance_params(verilog: &str, module: &str) -> Vec<(String, String)> {
    let at = verilog.find(&format!("  {module} #(")).unwrap_or_else(|| panic!("an instance of {module}")) + module.len() + 5;
    let mut depth = 1;
    let end = verilog[at..]
        .char_indices()
        .find(|&(_, c)| {
            depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            depth == 0
        })
        .unwrap()
        .0;
    verilog[at..at + end]
        .split("),")
        .filter_map(|kv| kv.trim().strip_prefix('.')?.trim_end_matches(')').split_once('('))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// The defaults `module` declares in `verilog`, each `NAME = VALUE` of its `#(parameter ...)` (a name the last word before
/// its value).
fn module_defaults(verilog: &str, module: &str) -> Vec<(String, String)> {
    let at = verilog.find(&format!("module {module} #(parameter ")).unwrap_or_else(|| panic!("module {module}")) + module.len() + 19;
    let end = verilog[at..].find(')').unwrap();
    verilog[at..at + end]
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .filter_map(|(k, v)| Some((k.split_whitespace().last()?.to_string(), v.trim().to_string())))
        .collect()
}

/// The value of a Verilog number literal: a plain decimal or a based literal (`16'd4`, `32'h3c00_0000`, `4'b1010`,
/// `8'o17`, `'sd3`), its size and signedness aside and its underscores dropped; None for anything else.
fn verilog_number(t: &str) -> Option<u64> {
    let t = t.trim().replace('_', "");
    if let Ok(n) = t.parse::<u64>() {
        return Some(n);
    }
    let (size, based) = t.split_once('\'')?;
    if !size.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let based = based.strip_prefix(['s', 'S']).unwrap_or(based);
    let radix = match based.chars().next()? {
        'd' | 'D' => 10,
        'h' | 'H' => 16,
        'b' | 'B' => 2,
        'o' | 'O' => 8,
        _ => return None,
    };
    u64::from_str_radix(&based[1..], radix).ok()
}

/// The number a parameter's value makes: a number literal ([`verilog_number`]), a name `named` gives a number, or one of
/// `/ * + -` between two of those; None for anything else.
fn param_number(expr: &str, named: &dyn Fn(&str) -> Option<u64>) -> Option<u64> {
    let value = |t: &str| -> Option<u64> {
        let t = t.trim();
        verilog_number(t).or_else(|| named(t))
    };
    value(expr).or_else(|| {
        ['/', '*', '+', '-'].iter().find_map(|&op| {
            let (a, b) = expr.split_once(op)?;
            let (a, b) = (value(a)?, value(b)?);
            match op {
                '/' => a.checked_div(b),
                '*' => a.checked_mul(b),
                '+' => a.checked_add(b),
                _ => a.checked_sub(b),
            }
        })
    })
}

/// The localparams of `module` in `verilog` whose values `known` and the localparams before them make a number
/// ([`param_number`]); any other left out.
fn module_localparams(verilog: &str, module: &str, known: &[(String, String)]) -> Vec<(String, String)> {
    let Some(at) = verilog.find(&format!("module {module} ")) else { return Vec::new() };
    let end = verilog[at..].find("\nendmodule").map_or(verilog.len(), |e| at + e);
    let mut out: Vec<(String, String)> = Vec::new();
    for line in verilog[at..end].lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("localparam ") else { continue };
        let rest = rest.trim_end_matches(';');
        let rest = if rest.starts_with('[') { rest.split_once(']').map_or(rest, |(_, r)| r) } else { rest };
        for def in rest.split(',') {
            let Some((name, expr)) = def.split_once('=') else { continue };
            let (name, expr) = (name.trim().to_string(), expr.trim());
            let got = param_number(expr, &|t| out.iter().chain(known).find(|(k, _)| k == t).and_then(|(_, v)| v.parse().ok()));
            if let Some(n) = got {
                out.push((name, n.to_string()));
            }
        }
    }
    out
}

/// The parameters `module`'s first instance in `verilog` is given, each value a number where the hierarchy gives one:
/// a name is the value the instantiating module's own first instance gives it, that module's default, or a localparam
/// of it those make a number ([`module_localparams`]), followed up to the fast side; an expression those make a number
/// is that number ([`param_number`]), any other left as written. None for a module instantiated without parameters.
fn resolved_params(verilog: &str, module: &str) -> Vec<(String, String)> {
    let Some(at) = verilog.find(&format!("  {module} #(")) else { return Vec::new() };
    let parent_at = verilog[..at].rfind("\nmodule ").map_or(0, |i| i + 1) + "module ".len();
    let parent: String = verilog[parent_at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    let mut above = if parent == "logos_engine_fast" { Vec::new() } else { resolved_params(verilog, &parent) };
    if verilog.contains(&format!("module {parent} #(parameter ")) {
        above.extend(module_defaults(verilog, &parent));
    }
    let locals = module_localparams(verilog, &parent, &above);
    above.extend(locals);
    let number = |v: &str| v.parse::<u64>().is_ok();
    instance_params(verilog, module)
        .into_iter()
        .map(|(k, v)| {
            let mut value = v.clone();
            for _ in 0..4 {
                if number(&value) || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    break;
                }
                match above.iter().find(|(n, _)| *n == value) {
                    Some((_, x)) => value = x.clone(),
                    None => break,
                }
            }
            if !number(&value) {
                if let Some(n) = param_number(&value, &|t| above.iter().find(|(n, _)| n == t).and_then(|(_, x)| x.parse().ok())) {
                    value = n.to_string();
                }
            }
            (k, value)
        })
        .collect()
}

/// A module the 200 MHz checks time as a stage of its own: its parameters as hybrid9 builds it, the check it belongs to,
/// and the LUT delays its register-to-register paths and loops may take — 9 for 5 ns, 4 in the processing elements'
/// clock of twice the fast side's (`clk2`, 2.5 ns).
struct Stage {
    top: &'static str,
    params: Vec<(String, String)>,
    check: &'static str,
    bound: i64,
}

/// The one unit of the fast side no 200 MHz check here times: at the 27B's widths its model takes yosys past 20 GB and
/// 25 minutes, so it is boxed in the attention side's stage and timed by a check of its own.
const TIMED_APART: &[&str] = &["logos_attend_int8_p"];

/// Every stage of the 200 MHz checks at hybrid9's shape: each module of the fast side with registers of its own — the
/// fast side itself (its ports, pipes, credits and channel multiplexers flattened into it), the pipelined token-row
/// store (its layers boxed as memories), the streaming unit and its
/// array, core, feed, lanes, processing elements, quantizers, image streams and greedy head, the vector side and its
/// vector unit and function lanes, the recurrence lanes, their coordinator and units, the attention side and its query's
/// quantizer, and the f32 units every side builds on — at the parameters its first instance is built with.
fn hybrid9_stages(v: &str) -> Vec<Stage> {
    hybrid9_stages_on(v, "logos_gdn_lane", "logos_gdn_flush_w")
}

/// [`hybrid9_stages`] of a build whose head lanes are `lane` and whose flush unit is `flush`.
fn hybrid9_stages_on(v: &str, lane: &'static str, flush: &'static str) -> Vec<Stage> {
    let unit = instance_params(v, "logos_streaming_matvec_step");
    let get = |params: &[(String, String)], k: &str| params.iter().find(|(n, _)| n == k).unwrap_or_else(|| panic!("{k} among {params:?}")).1.clone();
    let (n, t_max, nb_max) = (get(&unit, "N"), get(&unit, "T_MAX"), get(&unit, "NB_MAX"));
    let tw = (t_max.parse::<usize>().unwrap().max(2) as f64).log2().ceil().to_string();
    let ring = if n == "1" { "3" } else { "2" }.to_string();
    let p = |kv: &[(&str, &String)]| kv.iter().map(|(k, v)| (k.to_string(), (*v).clone())).collect::<Vec<_>>();
    let four = "4".to_string();
    let stage = |top: &'static str, params: Vec<(String, String)>, check: &'static str| Stage { top, params, check, bound: 900 };
    let resolved = |top: &'static str, check: &'static str| stage(top, resolved_params(v, top), check);
    vec![
        stage("logos_engine_fast", vec![], "fast side"),
        stage("logos_streaming_matvec_step", unit.clone(), "fast side"),
        stage("logos_column_array_step", p(&[("CH", &four), ("N", &n), ("T_MAX", &t_max), ("NB_MAX", &nb_max)]), "fast side"),
        stage("logos_column_core_bare_step", p(&[("SUMS", &t_max), ("SW", &tw), ("RING", &ring)]), "fast side"),
        stage("logos_column_feed_step", p(&[("N", &n)]), "fast side"),
        resolved("logos_vector_side", "fast side"),
        resolved("logos_store_w512", "fast side"),
        resolved(lane, "fast side"),
        resolved("logos_gdn_lanes", "fast side"),
        resolved("logos_attend_side", "fast side"),
        resolved("logos_act_stream_p", "fast side"),
        resolved("logos_lane_value_p", "matvec units"),
        resolved("logos_lane_slim_p", "matvec units"),
        resolved("logos_round88_p", "matvec units"),
        Stage { bound: 400, ..resolved("logos_matvec_pass_x2", "matvec units") },
        Stage { bound: 400, ..resolved("logos_pe8_p", "matvec units") },
        resolved("logos_act_blocks_p", "matvec units"),
        resolved("logos_act_code_p", "matvec units"),
        resolved("logos_wide_scale_p", "matvec units"),
        resolved("logos_greedy", "matvec units"),
        resolved("logos_image_stream", "matvec units"),
        resolved("logos_tile_run", "matvec units"),
        resolved("logos_fadd_p", "f32 units"),
        resolved("logos_fmul_p", "f32 units"),
        resolved("logos_exp_p", "f32 units"),
        resolved("logos_ln_p", "f32 units"),
        resolved("logos_recip_p", "f32 units"),
        resolved("logos_rsqrt_p", "f32 units"),
        resolved("logos_i2f_p", "f32 units"),
        resolved("logos_rne_p", "f32 units"),
        resolved("logos_f32_to_f16_p", "f32 units"),
        resolved("logos_lane_func_p", "f32 units"),
        resolved("logos_gdn_read_w", "recurrence units"),
        resolved(flush, "recurrence units"),
        resolved("logos_gdn_fold_m", "recurrence units"),
        resolved("logos_gdn_side", "recurrence units"),
        resolved("logos_simd_p", "vector unit"),
    ]
}

/// The stages of `check` in hybrid9's fast side timed on a pool of `threads` threads, each boxing every other stage's
/// module, every memory and the units timed apart: each stage past its bound, a path or a loop, named.
fn stages_past_their_bound(check: &str, threads: usize) -> Vec<String> {
    stages_past_their_bound_in(&hybrid9(), hybrid9_stages, check, threads, Some(HYBRID9_WIDE_NETS))
}

/// The loads a net may drive in the fast side before its wire crosses the die: one logic, a memory or an input drives
/// ([`LOGIC_FANOUT`]) and one a register drives ([`REGISTER_FANOUT`]), memory ports counted as the blocks they reach.
/// hybrid9e failed 5 ns on 200,000 endpoints, 70% of them through a net of 256 loads or more spread over the die, the
/// rest through wire between units, whatever their logic levels.
const LOGIC_FANOUT: u64 = 64;
const REGISTER_FANOUT: u64 = 512;

/// The nets the design names in each stage of hybrid9's fast side that drive past [`LOGIC_FANOUT`] or
/// [`REGISTER_FANOUT`], as the timing model counts them (`TimingGraph::wide`, clocks aside): a ratchet each fix lowers
/// to its new count and nothing raises, toward none. The nets synthesis names are left out: their names and their
/// split follow the whole file's elaboration, not the stage's own logic.
const HYBRID9_WIDE_NETS: &[(&str, usize)] = &[
    ("logos_engine_fast", 108),
    ("logos_streaming_matvec_step", 12),
    ("logos_column_array_step", 7),
    ("logos_column_core_bare_step", 56),
    ("logos_column_feed_step", 12),
    ("logos_vector_side", 29),
    ("logos_store_w512", 0),
    ("logos_gdn_lane", 18),
    ("logos_gdn_lanes", 2),
    ("logos_attend_side", 6),
    ("logos_act_stream_p", 1),
    ("logos_lane_value_p", 3),
    ("logos_lane_slim_p", 0),
    ("logos_round88_p", 6),
    ("logos_matvec_pass_x2", 2),
    ("logos_pe8_p", 0),
    ("logos_act_blocks_p", 2),
    ("logos_act_code_p", 0),
    ("logos_wide_scale_p", 0),
    ("logos_greedy", 3),
    ("logos_image_stream", 12),
    ("logos_tile_run", 2),
    ("logos_fadd_p", 1),
    ("logos_fmul_p", 0),
    ("logos_exp_p", 0),
    ("logos_ln_p", 0),
    ("logos_recip_p", 4),
    ("logos_rsqrt_p", 4),
    ("logos_i2f_p", 1),
    ("logos_rne_p", 0),
    ("logos_f32_to_f16_p", 0),
    ("logos_lane_func_p", 7),
    ("logos_gdn_read_w", 9),
    ("logos_gdn_flush_w", 6),
    ("logos_gdn_fold_m", 9),
    ("logos_gdn_side", 12),
    ("logos_simd_p", 178),
];

/// [`stages_past_their_bound`] in the engine `p`, whose stages `stages` names.
fn stages_past_their_bound_in(
    p: &logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams,
    stages: fn(&str) -> Vec<Stage>,
    check: &str,
    threads: usize,
    wide_nets: Option<&[(&str, usize)]>,
) -> Vec<String> {
    use logicaffeine_compile::codegen_sva::timing_graph::{modules_numbered, synthesize, Cost, Delays};
    use logicaffeine_compile::spatial::decode_engine::rtl::ram::{bram_name, bram_re_name, ram_ce_name, ram_name};
    use logicaffeine_proof::cycle_ratio::Bound;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let d = emit(&AfuConfig::engine(p.clone(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let all = stages(&v);
    let prefixes: Vec<String> = [ram_name(8), bram_name(8), bram_re_name(8), ram_ce_name(8)].iter().map(|m| m.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()).collect();
    let prefixes: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    let memories: Vec<String> = modules_numbered(&v, &prefixes);
    let mine: Vec<&Stage> = all.iter().filter(|s| s.check == check).collect();
    assert!(!mine.is_empty(), "no stage of the check {check}");
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (v, all, memories, mine, next) = (&v, &all, &memories, &mine, &next);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::SeqCst);
                        let Some(stage) = mine.get(k) else { break };
                        let mut boxes: Vec<(&str, Cost)> = all.iter().filter(|o| o.top != stage.top).map(|o| (o.top, Cost::NONE)).collect();
                        boxes.extend(TIMED_APART.iter().map(|m| (*m, Cost::NONE)));
                        boxes.extend(memories.iter().filter(|m| m.as_str() != stage.top).map(|m| (m.as_str(), Cost::MEMORY)));
                        boxes.extend(parts_built_by(v, stage.top).into_iter().map(|m| (m, Cost::MEMORY)));
                        let pairs: Vec<(&str, &str)> = stage.params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                        let top = stage.top;
                        let g = synthesize(v, top, &pairs, &boxes, &Delays::LEVELS).unwrap_or_else(|e| panic!("{top}: {e}"));
                        let depth = g.depth().unwrap_or_else(|e| panic!("{top}: {e:?}")).depth;
                        let looped = match g.loop_bound().unwrap_or_else(|e| panic!("{top}: {e:?}")) {
                            Bound::Ratio { .. } => {
                                let (num, den) = g.loop_bound().unwrap().period().unwrap_or((0, 1));
                                num / den.max(1)
                            }
                            _ => 0,
                        };
                        if let Some(budget) = wide_nets {
                            let wide: Vec<_> = g.wide(LOGIC_FANOUT, REGISTER_FANOUT).into_iter().filter(|w| g.public[w.node]).collect();
                            match budget.iter().find(|(t, _)| *t == top) {
                                None => out.push(format!("{top}: no count of wide nets recorded ({})", wide.len())),
                                Some(&(_, b)) if wide.len() > b => out.push(format!(
                                    "{top}: {} nets drive past {LOGIC_FANOUT} loads from logic or {REGISTER_FANOUT} from a register, more than its {b}: {}",
                                    wide.len(),
                                    wide.iter().take(6).map(|w| format!("{} {} ({:?})", w.name, w.loads, w.driver)).collect::<Vec<_>>().join(", ")
                                )),
                                Some(&(_, b)) if wide.len() < b => {
                                    out.push(format!("{top}: {} wide nets, fewer than the {b} recorded: lower its count to {}", wide.len(), wide.len()))
                                }
                                _ => {}
                            }
                        }
                        let bound = stage.bound;
                        if depth > bound {
                            out.push(format!("{top}: a path of {:.2} LUT delays, past {:.2}", depth as f64 / 100.0, bound as f64 / 100.0));
                        }
                        if looped > bound {
                            out.push(format!("{top}: a loop of {:.2} LUT delays a register, past {:.2}", looped as f64 / 100.0, bound as f64 / 100.0));
                        }
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    })
}

/// Every stage of the 200 MHz checks is a module of hybrid9's fast side that holds registers of its own, named once,
/// each with every parameter its module declares given a number; the units timed apart are none of them. With every
/// stage boxing every other, each module of the fast side is a stage or is timed in the stage of its nearest module
/// above that is, but the units timed apart.
#[test]
fn the_200_mhz_checks_time_every_module_of_the_fast_side_once() {
    let d = emit(&AfuConfig::engine(hybrid9(), LocalMemoryLine::Data512));
    let v = std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap();
    let stages = hybrid9_stages(&v);
    let mut seen = std::collections::BTreeSet::new();
    for s in &stages {
        assert!(seen.insert(s.top), "{} is a stage twice", s.top);
        assert!(!TIMED_APART.contains(&s.top), "{} is timed apart and a stage", s.top);
        let body = module_text(&v, s.top);
        assert!(body.contains("always @(posedge"), "{} holds no register of its own", s.top);
        if v.contains(&format!("module {} #(parameter ", s.top)) {
            for (k, _) in module_defaults(&v, s.top) {
                let given = s.params.iter().find(|(n, _)| *n == k).map(|(_, x)| x.as_str());
                assert!(given.is_none_or(|x| x.parse::<u64>().is_ok()), "{}'s {k} is given {given:?}", s.top);
            }
        }
    }
    for apart in TIMED_APART {
        assert!(v.contains(&format!("module {apart}")), "{apart} is in the fast side");
    }
}

/// The fast side at the card's shape keeps every stage within a 200 MHz clock. In the timing model
/// (`codegen_sva::timing_graph`, a LUT delay 0.55 ns as fit27 calibrated it) each module of the matvec and the row class
/// reaches every register within the 9 LUT delays of 5 ns and closes no longer loop: the fast side's own logic (its
/// ports, pipes, credits and channel multiplexers), the streaming unit, the column array, a core, a feed, the vector
/// side, a recurrence lane and the lanes' coordinator, the attention side and its query's quantizer, each at the
/// parameters it is built with, every module timed in a check of its own and its memories boxed.
#[test]
fn the_cards_fast_side_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound("fast side", 10);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The streaming unit's units keep every stage within a 200 MHz clock: the block-scale lanes, their rounding, the
/// quantizer and its parts, the greedy head, the image streams and tile runs; and the processing elements in their
/// clock of twice the fast side's within its 4 LUT delays.
#[test]
fn the_cards_matvec_units_keep_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound("matvec units", 6);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The f32 units the sides build on keep every stage within a 200 MHz clock: the add and the multiply, the exponential,
/// the logarithm, the reciprocal and its square root, the conversions and the function lanes.
#[test]
fn the_cards_f32_units_keep_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound("f32 units", 6);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The declarations of `verilog` that give some of their nets or variables a value and not others: each from its
/// keyword at a line's start to its `;`, over as many lines as it takes, its comments left out.
fn mixed_declarations(verilog: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    for (k, line) in verilog.split_inclusive('\n').enumerate() {
        let start = at;
        at += line.len();
        let t = line.trim_start();
        if !["wire ", "reg ", "logic ", "integer "].iter().any(|w| t.starts_with(w)) {
            continue;
        }
        let from = start + (line.len() - t.len());
        let body: String = verilog[from..].lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join(" ");
        let Some(body) = body.split(';').next() else { continue };
        // The declaration's items, split at the commas outside any bracket; an item has a value when it holds an `=`
        // that is no comparison's.
        let (mut depth, mut items, mut cur) = (0i32, Vec::new(), String::new());
        for c in body.chars() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
            if c == ',' && depth == 0 {
                items.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        }
        items.push(cur);
        let valued = |s: &str| {
            let b = s.as_bytes();
            (0..b.len()).any(|i| b[i] == b'=' && !matches!(b.get(i + 1), Some(b'=')) && !(i > 0 && matches!(b[i - 1], b'=' | b'<' | b'>' | b'!')))
        };
        let given = items.iter().filter(|s| valued(s)).count();
        if items.len() > 1 && given > 0 && given < items.len() {
            out.push(format!("line {}: {};", k + 1, body.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    }
    out
}

/// Every build's RTL declares a net or variable with a value apart from those without: Quartus refuses a declaration
/// that gives some of its names a value and not others ("all identifiers need to have initial value", error 17154 —
/// hybrid9c's synthesis stopped at the fold path's `wire f_take, adv = ...`), where iverilog, Verilator and slang take
/// it. Every RTL file of the reference engine's build, the hybrid's, every unit's on the fast side, head lanes' a
/// window behind and with contexts, hybrid9's and hybrid9's folding its sums.
#[test]
fn every_builds_rtl_declares_a_name_with_a_value_apart_from_names_without() {
    let builds = [
        ("the reference engine", AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512)),
        ("the hybrid", AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512)),
        ("every unit on the fast side", AfuConfig::engine(fast_side_08b(), LocalMemoryLine::Data512)),
        ("head lanes a window behind", AfuConfig::engine(trailing_lanes_08b(), LocalMemoryLine::Data512)),
        ("contexts on head lanes", AfuConfig::engine(contexts_lanes_08b(), LocalMemoryLine::Data512)),
        ("hybrid9", AfuConfig::engine(hybrid9(), LocalMemoryLine::Data512)),
        ("hybrid9 folding", AfuConfig::engine(folding9(), LocalMemoryLine::Data512)),
    ];
    let mut wrong = Vec::new();
    for (what, cfg) in builds {
        let d = emit(&cfg);
        let dir = d.path().join("hw/rtl");
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "v" || x == "sv") {
                for m in mixed_declarations(&std::fs::read_to_string(&p).unwrap()) {
                    wrong.push(format!("{what}: {}: {m}", p.file_name().unwrap().to_string_lossy()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let sample = "  wire a, b = c;\n  wire [3:0] d = {e, f}, g = h == i;\n  reg j, // k = 1 below\n    k;\n  wire m = n,\n    o;\n";
    assert_eq!(mixed_declarations(sample), ["line 1: wire a, b = c;", "line 5: wire m = n, o;"], "the gate sees what it gates");
}

/// hybrid9's engine with its streaming unit folding a job's sums into its output slot over four lanes.
fn folding9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = hybrid9();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_folds(4)), folds: true, ..p }
}

/// [`hybrid9_stages`] of [`folding9`]: the streaming unit, its fold path within it, and the fold lane the folds' check.
fn folding9_stages(v: &str) -> Vec<Stage> {
    let mut stages: Vec<Stage> = hybrid9_stages(v)
        .into_iter()
        .map(|s| if s.top == "logos_streaming_matvec_step" { Stage { check: "folds", ..s } } else { s })
        .collect();
    stages.push(Stage { top: "logos_fold_lane_p", params: resolved_params(v, "logos_fold_lane_p"), check: "folds", bound: 900 });
    stages
}

/// A streaming unit that folds keeps every stage within a 200 MHz clock: its write-back's fold path and the fold lane,
/// every other unit boxed.
#[test]
fn the_cards_folding_unit_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound_in(&folding9(), folding9_stages, "folds", 2, None);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// hybrid9's engine with its recurrence as hybrid9c carries it: its memories flushed a window of [`TRAILING_TOKENS`]
/// behind, its two head lanes and their coordinator asking 128 lines ahead.
///
/// [`TRAILING_TOKENS`]: logicaffeine_compile::spatial::decode_engine::model::TRAILING_TOKENS
fn trailing9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::model::TRAILING_TOKENS;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    use logicaffeine_compile::spatial::serving::StateFlush;
    let p = hybrid9();
    EngineParams {
        delta: p.delta.map(|(w, _)| (w, StateFlush::Trailing { tokens: TRAILING_TOKENS })),
        streaming: p.streaming.clone().map(|s| s.with_recurrence_credit(128)),
        ..p
    }
}

/// [`hybrid9_stages`] of [`trailing9`]: the trailing head lane in the lane's place and the window's flush unit in the
/// whole log's, each of the recurrence's modules — the lanes, the trailing lane, the coordinator, the fold, the read-out
/// and the flush — the trailing check's.
fn trailing9_stages(v: &str) -> Vec<Stage> {
    const RECURRENCE: [&str; 6] = ["logos_gdn_lanes", "logos_gdn_lane_t", "logos_gdn_side", "logos_gdn_fold_m", "logos_gdn_read_w", "logos_gdn_flush_m"];
    hybrid9_stages_on(v, "logos_gdn_lane_t", "logos_gdn_flush_m")
        .into_iter()
        .map(|s| if RECURRENCE.contains(&s.top) { Stage { check: "trailing", ..s } } else { s })
        .collect()
}

/// The recurrence as hybrid9c carries it ([`trailing9`]) keeps every stage within a 200 MHz clock: the trailing head
/// lane at its credit's queues and two fold buffers, the window's flush unit, the trailing fold, the coordinator's ring,
/// the read-out and the lanes, every other unit boxed.
#[test]
fn the_cards_trailing_recurrence_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound_in(&trailing9(), trailing9_stages, "trailing", 4, None);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// hybrid8's shape with its array's issues two token-rows each, both words on one set of each core's processing
/// elements in clk2 (`ColumnSet::with_token_pairs`, `with_pe_pairs`), over two contexts.
fn pairs9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let p = hybrid8();
    let streaming = p.streaming.clone().map(|s| Streaming { set: s.set.clone().with_token_pairs().with_pe_pairs(), ..s }.with_contexts(2));
    EngineParams { streaming, ..p }
}

/// The stages of [`pairs9`]'s fast side, [`hybrid9_stages`]' but each core a module of its own (not a bare core) at a
/// token pair's places, the clk2 elements `logos_matvec_pass_x2t` in `logos_matvec_pass_x2`'s place and no slim octet
/// lanes; the array's modules the pairs check's.
fn pairs9_stages(v: &str) -> Vec<Stage> {
    let unit = instance_params(v, "logos_streaming_matvec_step");
    let get = |k: &str| unit.iter().find(|(n, _)| n == k).unwrap_or_else(|| panic!("{k} among {unit:?}")).1.clone();
    let (n, t_max, nb_max) = (get("N"), get("T_MAX"), get("NB_MAX"));
    let places = t_max.parse::<usize>().unwrap().div_ceil(2);
    let (sums, sw) = (places.to_string(), (places.max(2) as f64).log2().ceil().to_string());
    let ring = if n == "1" { "3" } else { "2" }.to_string();
    let p = |kv: &[(&str, &String)]| kv.iter().map(|(k, v)| (k.to_string(), (*v).clone())).collect::<Vec<_>>();
    let four = "4".to_string();
    let stage = |top: &'static str, params: Vec<(String, String)>, check: &'static str| Stage { top, params, check, bound: 900 };
    let resolved = |top: &'static str, check: &'static str| stage(top, resolved_params(v, top), check);
    vec![
        stage("logos_engine_fast", vec![], "fast side"),
        stage("logos_streaming_matvec_step", unit.clone(), "pairs"),
        stage("logos_column_array_step", p(&[("CH", &four), ("N", &n), ("T_MAX", &t_max), ("NB_MAX", &nb_max)]), "pairs"),
        stage("logos_column_core_step", p(&[("SUMS", &sums), ("SW", &sw), ("RING", &ring)]), "pairs"),
        stage("logos_column_feed_step", p(&[("N", &n)]), "pairs"),
        resolved("logos_vector_side", "fast side"),
        resolved("logos_store_w512", "fast side"),
        resolved("logos_gdn_lane", "fast side"),
        resolved("logos_gdn_lanes", "fast side"),
        resolved("logos_attend_side", "fast side"),
        resolved("logos_act_stream_p", "fast side"),
        resolved("logos_lane_value_p", "pairs"),
        resolved("logos_round88_p", "pairs"),
        Stage { bound: 400, ..resolved("logos_matvec_pass_x2t", "pairs") },
        Stage { bound: 400, ..resolved("logos_pe8_p", "pairs") },
        resolved("logos_act_blocks_p", "matvec units"),
        resolved("logos_act_code_p", "matvec units"),
        resolved("logos_wide_scale_p", "matvec units"),
        resolved("logos_greedy", "matvec units"),
        resolved("logos_image_stream", "matvec units"),
        resolved("logos_tile_run", "matvec units"),
        resolved("logos_fadd_p", "f32 units"),
        resolved("logos_fmul_p", "f32 units"),
        resolved("logos_exp_p", "f32 units"),
        resolved("logos_ln_p", "f32 units"),
        resolved("logos_recip_p", "f32 units"),
        resolved("logos_rsqrt_p", "f32 units"),
        resolved("logos_i2f_p", "f32 units"),
        resolved("logos_rne_p", "f32 units"),
        resolved("logos_f32_to_f16_p", "f32 units"),
        resolved("logos_lane_func_p", "f32 units"),
        resolved("logos_gdn_read_w", "recurrence units"),
        resolved("logos_gdn_flush_w", "recurrence units"),
        resolved("logos_gdn_fold_m", "recurrence units"),
        resolved("logos_gdn_side", "recurrence units"),
        resolved("logos_simd_p", "vector unit"),
    ]
}

/// The array of token pairs on clk2 elements ([`pairs9`]) keeps every stage within a 200 MHz clock: each core's two
/// words, lanes and sums, the elements in clk2 within half the clock, the array's pair-wide copies of the activations,
/// the unit's pair-wide queue, every other unit boxed.
#[test]
fn the_cards_array_of_token_pairs_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound_in(&pairs9(), pairs9_stages, "pairs", 4, None);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// [`trailing9`] with its head lanes making each recurrence job's gated norm (`Streaming::with_delta_norm`).
fn normed9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let p = trailing9();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_delta_norm()), ..p }
}

/// [`trailing9_stages`] of [`normed9`], the recurrence's stages the normed check's and the head lanes' gate
/// (`logos_gdn_gate_p`) a stage of its own.
fn normed9_stages(v: &str) -> Vec<Stage> {
    let mut stages: Vec<Stage> = trailing9_stages(v).into_iter().map(|s| if s.check == "trailing" { Stage { check: "normed", ..s } } else { s }).collect();
    stages.push(Stage { top: "logos_gdn_gate_p", params: resolved_params(v, "logos_gdn_gate_p"), check: "normed", bound: 900 });
    stages
}

/// The recurrence making its gated norms on its head lanes ([`normed9`]) keeps every stage within a 200 MHz clock: the
/// gate — its sums' lanes in two banks, the scale's chain, the products beside the SiLU — the trailing lane loading each
/// head's gate after its query, the lanes fetching the weight, every other unit boxed.
#[test]
fn the_cards_recurrence_making_its_gated_norms_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound_in(&normed9(), normed9_stages, "normed", 4, None);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// [`normed9`] with each convolution job on the vector side's conv path at four lanes, its kernel as deep as the widest
/// convolution of the checkpoints hybrid8 is built for (`Streaming::with_conv_path`, `Streaming::conv_beats`).
fn conv9() -> logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let specs: Vec<_> = ["qwen3.8-27b-q4_k_m.json", "qwen3.5-0.8b-q8_0.json"]
        .iter()
        .map(|l| GgufHeader::read_ledger(&root().join("benchmarks/silicon/decode/headers").join(l)).unwrap().spec().unwrap())
        .collect();
    let beats = Streaming::conv_beats(&specs);
    assert_eq!(beats, 640, "the 27B's convolution is 10,240 channels");
    let p = normed9();
    EngineParams { streaming: p.streaming.clone().map(|s| s.with_conv_path(4, beats)), ..p }
}

/// [`normed9_stages`] of [`conv9`], the vector side the conv check's and the conv path and its lane stages of their own.
fn conv9_stages(v: &str) -> Vec<Stage> {
    let mut stages: Vec<Stage> = normed9_stages(v).into_iter().map(|s| if s.top == "logos_vector_side" { Stage { check: "conv", ..s } } else { s }).collect();
    for top in ["logos_conv_path", "logos_conv_lane_p"] {
        stages.push(Stage { top, params: resolved_params(v, top), check: "conv", bound: 900 });
    }
    stages
}

/// The nets of [`conv9`]'s conv stages past [`LOGIC_FANOUT`] or [`REGISTER_FANOUT`], a ratchet as
/// [`HYBRID9_WIDE_NETS`] is.
const CONV9_WIDE_NETS: &[(&str, usize)] = &[
    ("logos_vector_side", 29),
    ("logos_conv_path", 2),
    ("logos_conv_lane_p", 0),
];

/// The vector side running each convolution on its conv path ([`conv9`]) keeps every stage within a 200 MHz clock: the
/// side recognizing the machine's conv job and handing it over, the path's kernel memories, ring triples, queues and
/// output gather, and its lanes beside their products, adds and SiLU, every other unit boxed.
#[test]
fn the_cards_conv_path_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound_in(&conv9(), conv9_stages, "conv", 3, Some(CONV9_WIDE_NETS));
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The recurrence lanes' units and their coordinator keep every stage within a 200 MHz clock.
#[test]
fn the_cards_recurrence_units_keep_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound("recurrence units", 4);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The vector side's vector unit keeps every stage within a 200 MHz clock.
#[test]
fn the_cards_vector_unit_keeps_every_stage_within_a_200_mhz_clock() {
    if !common::require_tool("yosys") {
        return;
    }
    let over = stages_past_their_bound("vector unit", 1);
    assert!(over.is_empty(), "past a 200 MHz clock:\n{}", over.join("\n"));
}

/// The card's build passes the RAM gate at the 27B's widths, where the vector side's held float-matrix inputs are 1088
/// beats a row and a head lane's copy of the log 1072 lines: every large memory of hybrid8's whole shell is a block-RAM
/// wrapper's or its module's own, and every memory of thirty-two words or fewer asks for MLABs or registers. At a small
/// model's widths the large memories fall under the least size, so only the card's own shape shows them.
#[test]
fn the_cards_build_keeps_every_large_memory_where_quartus_builds_block_ram() {
    if !cdc_tools() {
        return;
    }
    let d = emit(&AfuConfig::engine(hybrid8(), LocalMemoryLine::Data512));
    let mut cmd = Command::new("python3");
    cmd.arg(script("ram_check.py")).arg("logos_d5005_shell").arg("65536");
    for f in ["logos_dcfifo.v", "logos_secded.v", "logos_stream.v", "logos_csr.v", "logos_bringup_core.v", "logos_engine.v", "logos_card_port.v", "logos_compute_clock.v", "logos_d5005_shell.v"] {
        if rtl(d.path(), f).exists() {
            cmd.arg(rtl(d.path(), f));
        }
    }
    let o = cmd.output().expect("python3");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("logos_d5005_shell: clean;"), "{}", text(&o));
    let wrapped: usize = text(&o).split(": ").nth(2).and_then(|t| t.split(' ').next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    assert!(wrapped >= 4, "the held inputs, the lanes' copies and the tables are wrappers': {}", text(&o));
    assert!(text(&o).contains("each in MLABs or registers"), "{}", text(&o));
}

// ── The host runtime ────────────────────────────────────────────────────────────────────────────────

fn cosim_tools() -> bool {
    common::require_tool("iverilog") && common::require_tool("iverilog-vpi") && common::require_tool("vvp")
}

/// The receipt run against the RTL of the tree at `d`: its exit status and its JSON.
fn cosim(d: &Path, lines: u64, host_lines: u64) -> (Option<i32>, String) {
    cosim_logged(d, lines, host_lines, None)
}

/// The same, a DDR4-timed tree's channels logging what they schedule into `log` when it is given.
fn cosim_logged(d: &Path, lines: u64, host_lines: u64, log: Option<&Path>) -> (Option<i32>, String) {
    let mut cmd = Command::new("bash");
    cmd.arg(script("run-cosim.sh")).arg(d).arg("receipt").arg(lines.to_string()).arg(host_lines.to_string());
    cmd.env("TMPDIR", std::env::temp_dir());
    if let Some(log) = log {
        cmd.env("LOGOS_DDR4_LOG", log);
    }
    let o = cmd.output().expect("bash");
    (o.status.code(), text(&o))
}

fn receipt_json(out: &str) -> serde_json::Value {
    let line = out.lines().find(|l| l.starts_with("{\"afu_id\"")).unwrap_or_else(|| panic!("a receipt in:\n{out}"));
    serde_json::from_str(line).unwrap()
}

/// The receipt the card runs on its first day runs unchanged against the RTL: the accelerator's ID and feature header,
/// every channel filled and checked alone and all four at once with nothing read back wrong and every line differing
/// under another seed, a host buffer's round trip unchanged, an unknown op refused. Four channels at once move about
/// four times one channel's lines a cycle, and two runs agree to the cycle.
#[test]
fn the_receipt_passes_against_the_rtl() {
    if !cosim_tools() {
        return;
    }
    let cfg = AfuConfig::d5005();
    let d = emit(&cfg);
    let (code, out) = cosim(d.path(), 1024, 128);
    assert_eq!(code, Some(0), "the receipt against the RTL:\n{out}");
    let r = receipt_json(&out);
    assert_eq!(r["checks_failed"], 0, "{out}");
    assert_eq!(r["afu_id"], cfg.uuid_text());
    assert_eq!(r["line_bytes"], (cfg.line.bits() / 8) as u64);
    assert_eq!(r["user_clock_hz"].as_f64().unwrap().round(), (1e12 / 2222.0f64).round());
    let channels = r["channels"].as_array().unwrap();
    let masks: Vec<u64> = channels.iter().map(|c| c["mask"].as_u64().unwrap()).collect();
    assert_eq!(masks, [1, 2, 4, 8, 15]);
    let hz = r["user_clock_hz"].as_f64().unwrap();
    for c in channels {
        assert_eq!(c["mismatches"], 0, "{c}");
        let lines = c["lines"].as_f64().unwrap() * if c["mask"] == 15 { 4.0 } else { 1.0 };
        for (rate, cycles) in [("write_gb_per_s", "fill_cycles"), ("read_gb_per_s", "check_cycles")] {
            let want = lines * (cfg.line.bits() / 8) as f64 * hz / c[cycles].as_f64().unwrap() / 1e9;
            assert!((c[rate].as_f64().unwrap() - want).abs() < 1e-3, "{rate} is the lines it moved over its cycles: {c}");
        }
    }
    for dir in ["write_gb_per_s", "read_gb_per_s"] {
        let one = channels[0][dir].as_f64().unwrap();
        let all = channels[4][dir].as_f64().unwrap();
        assert!(all > 3.0 * one, "{dir}: four channels at once {all} against one {one}");
    }
    assert_eq!(r["host"]["round_trip"], true);
    let (_, again) = cosim(d.path(), 1024, 128);
    assert_eq!(receipt_json(&again), r, "a second run, cycle for cycle");
}

/// With `LOGOS_COSIM_LOG` naming a file, `run-cosim.sh` appends everything the simulation writes to it, both streams,
/// as it is written, each run after the last, so a co-simulation hours long can be read while it runs and what it did is
/// kept when it is stopped; the script's own output and exit are what they are without it. A program writing a line to
/// each stream for each argument and exiting 3, run twice: each run fails as the program did and prints its lines, and the
/// log holds both runs' lines and their exits in order.
#[test]
fn a_cosimulation_appends_what_it_writes_to_its_log_as_it_runs() {
    if !cosim_tools() {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    std::fs::write(
        d.path().join("sw/logos_echo.c"),
        "#include <stdio.h>\n#ifndef LOGOS_PROGRAM_MAIN\n#define LOGOS_PROGRAM_MAIN main\n#endif\n\
         int LOGOS_PROGRAM_MAIN(int argc, char **argv) {\n\
         \x20   for (int i = 1; i < argc; ++i) { printf(\"out %s\\n\", argv[i]); fflush(stdout); fprintf(stderr, \"err %s\\n\", argv[i]); }\n\
         \x20   return 3;\n}\n",
    )
    .unwrap();
    let logs = tempfile::tempdir().unwrap();
    let log = logs.path().join("cosim.log");
    let run = |words: &[&str]| {
        let o = Command::new("bash")
            .arg(script("run-cosim.sh"))
            .arg(d.path())
            .arg("echo")
            .args(words)
            .env("TMPDIR", std::env::temp_dir())
            .env("LOGOS_COSIM_LOG", &log)
            .output()
            .expect("bash");
        (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
    };
    let (code, out, err) = run(&["a", "b"]);
    assert_eq!(code, Some(1), "the run fails as its program did:\n{out}\n{err}");
    assert_eq!(out.lines().filter(|l| l.starts_with("out ")).collect::<Vec<_>>(), ["out a", "out b"], "its output printed:\n{out}");
    assert!(!out.contains("logos_cosim: the program exited"), "the exit line stays out of the output:\n{out}");
    assert_eq!(err.lines().filter(|l| l.starts_with("err ")).collect::<Vec<_>>(), ["err a", "err b"], "its errors printed:\n{err}");
    let (code, _, _) = run(&["c"]);
    assert_eq!(code, Some(1));
    let logged = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("the log {}: {e}", log.display()));
    let lines = |prefix: &str| logged.lines().filter(|l| l.starts_with(prefix)).map(str::to_string).collect::<Vec<_>>();
    assert_eq!(lines("out "), ["out a", "out b", "out c"], "both runs' output in the log, in order:\n{logged}");
    assert_eq!(lines("err "), ["err a", "err b", "err c"], "both runs' errors in the log, in order:\n{logged}");
    assert_eq!(lines("logos_cosim: the program exited"), ["logos_cosim: the program exited 3"; 2], "each run's exit:\n{logged}");
    let at = |l: &str| logged.lines().position(|x| x == l).unwrap();
    assert!(at("out b") < at("out c"), "the second run after the first:\n{logged}");
}

/// The same receipt runs unchanged under Verilator through the DPI-C backend (`LOGOS_SIM=verilator`): every check
/// holds, on plain lines and on SECDED lines, though its memories wait at Verilator's random draws, not Icarus's.
#[test]
fn the_receipt_passes_under_verilator() {
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    for line in [LocalMemoryLine::DataAndEcc576, LocalMemoryLine::Data512Secded] {
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let o = Command::new("bash")
            .arg(script("run-cosim.sh"))
            .arg(d.path())
            .args(["receipt", "256", "64"])
            .env("LOGOS_SIM", "verilator")
            .env("VERILATOR", common::verilator())
            .env("TMPDIR", std::env::temp_dir())
            .output()
            .expect("bash");
        let out = text(&o);
        assert_eq!(o.status.code(), Some(0), "{line:?} under Verilator:\n{out}");
        let r = receipt_json(&out);
        assert_eq!(r["checks_failed"], 0, "{out}");
        assert_eq!(r["line_bytes"], (line.bits() / 8) as u64);
        assert_eq!(r.get("secded").is_some(), line.secded(), "{out}");
    }
}

/// The receipt passes with every channel a DDR4-2400 channel timed by the controller model: every line the shell asked
/// for, through its fills, checks and copies, was scheduled as the controller model schedules it at the clock it
/// arrived, every channel's commands legal, and no channel moved more than its bus carries.
#[test]
fn the_receipt_passes_on_ddr4_timed_channels() {
    if !cosim_tools() {
        return;
    }
    let cfg = AfuConfig { channel_model: ChannelModel::Ddr4, ..AfuConfig::d5005() };
    let d = emit(&cfg);
    let logs = tempfile::tempdir().unwrap();
    let (code, out) = cosim_logged(d.path(), 1024, 128, Some(logs.path()));
    assert_eq!(code, Some(0), "the receipt on DDR4-timed channels:\n{out}");
    let r = receipt_json(&out);
    assert_eq!(r["checks_failed"], 0, "{out}");
    let t = Ddr4Grade::D5005_EMIF.at();
    for b in 0..BANKS {
        let log = ddr4_log::read_log(&logs.path().join(format!("ddr4_channel_{b}.log")));
        assert!(log.lines.iter().any(|l| l.1) && log.lines.iter().any(|l| !l.1), "channel {b} was written and read");
        let (_, facts) = ddr4_log::replay(&t, &log, &format!("channel {b}"));
        assert_eq!(facts.reads + facts.writes, log.lines.len() as u64);
    }
    let peak = t.peak_bytes_per_s(t.burst_bits() / 8) / 1e9;
    for c in r["channels"].as_array().unwrap() {
        let lanes = if c["mask"] == 15 { 4.0 } else { 1.0 };
        for rate in ["write_gb_per_s", "read_gb_per_s"] {
            assert!(c[rate].as_f64().unwrap() <= lanes * peak, "{rate} past the bus: {c}");
        }
    }
}

/// A weight image's directory as the image writer leaves it: the tiny Qwen3.5 checkpoint laid out at `beat`.
fn image_dir(beat: Beat) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Qwen35.wide(64);
    let path = dir.path().join("qwen35.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let mut g = Gguf::open(&path).unwrap();
    let table = TensorTable::from_rows(spec.layers.len() as u64, g.tensor_names()).unwrap();
    let image = weight_image(&table, &table.read_order(), &ImageLayout::d5005(beat)).unwrap();
    let mut files: Vec<std::fs::File> =
        (0..image.channel_lines.len()).map(|k| std::fs::File::create(dir.path().join(format!("channel{k}.bin"))).unwrap()).collect();
    let mut out: Vec<&mut dyn std::io::Write> = files.iter_mut().map(|f| f as &mut dyn std::io::Write).collect();
    image.write_channels(&mut g, &mut out).unwrap();
    std::fs::write(dir.path().join("tiles.bin"), image.tile_table()).unwrap();
    dir
}

fn load_json(out: &str) -> serde_json::Value {
    let line = out.lines().find(|l| l.starts_with("{\"image\"")).unwrap_or_else(|| panic!("the loader's record in:\n{out}"));
    serde_json::from_str(line).unwrap()
}

/// The image loader copies an image onto every channel at the channel's full line width — 72 bytes with the ECC lanes
/// as data, 64 without or on SECDED lines — and reads back its files exactly, every tile's check word holding; a line
/// it damages on the card is found in its tile and no other, and that tile loaded again verifies.
#[test]
fn the_image_loads_and_verifies_at_both_line_widths() {
    if !cosim_tools() {
        return;
    }
    for (line, beat) in [
        (LocalMemoryLine::DataAndEcc576, Beat::Full72),
        (LocalMemoryLine::Data512, Beat::Data64),
        (LocalMemoryLine::Data512Secded, Beat::Data64),
    ] {
        let image = image_dir(beat);
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let o = Command::new("bash")
            .arg(script("run-cosim.sh"))
            .arg(d.path())
            .arg("load")
            .arg(image.path())
            .arg("--self-test")
            .env("TMPDIR", std::env::temp_dir())
            .output()
            .expect("bash");
        let out = text(&o);
        assert_eq!(o.status.code(), Some(0), "{line:?}:\n{out}");
        let r = load_json(&out);
        assert_eq!(r["checks_failed"], 0, "{out}");
        assert_eq!(r["line_bytes"], (line.bits() / 8) as u64);
        assert_eq!(r["check"], beat == Beat::Full72);
        assert_eq!(r["bad_tiles"], serde_json::json!([]), "{out}");
        assert_eq!(r["crc_errors"], serde_json::json!([]), "{out}");
        let table = std::fs::read(image.path().join("tiles.bin")).unwrap();
        let word = |i: usize| u64::from_le_bytes(table[8 + 8 * i..16 + 8 * i].try_into().unwrap());
        let lines: Vec<u64> = (0..BANKS as usize).map(|k| word(4 + k)).collect();
        let loaded: Vec<u64> = r["channels"].as_array().unwrap().iter().map(|c| c["lines"].as_u64().unwrap()).collect();
        assert_eq!(loaded, lines);
        let t = &r["self_test"];
        let victim = t["tile"].as_u64().unwrap();
        assert_eq!(t["bad_tiles"], serde_json::json!([victim]), "{out}");
        assert_eq!(t["crc_errors"], if beat == Beat::Full72 { serde_json::json!([victim]) } else { serde_json::json!([]) }, "{out}");
        assert_eq!(t["after_reload"], serde_json::json!([]), "{out}");
    }
}

/// An image of another line width than the build's is refused before a line moves.
#[test]
fn the_loader_refuses_an_image_of_another_width() {
    if !cosim_tools() {
        return;
    }
    let image = image_dir(Beat::Full72);
    let d = emit(&AfuConfig { line: LocalMemoryLine::Data512, ..AfuConfig::d5005() });
    let o = Command::new("bash")
        .arg(script("run-cosim.sh"))
        .arg(d.path())
        .arg("load")
        .arg(image.path())
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .expect("bash");
    assert_ne!(o.status.code(), Some(0));
    assert!(text(&o).contains("the image is 4 channels of 72-byte lines, the accelerator 4 of 64"), "{}", text(&o));
}

/// The receipt fails when the accelerator is wrong: a fill whose lines are not the pattern's, a copy that changes a host
/// line, an accelerator carrying another AFU ID.
#[test]
fn the_receipt_fails_when_the_rtl_is_wrong() {
    if !cosim_tools() {
        return;
    }
    let low_id = format!("AFU_ID_L = 64'h{:016x}", AfuConfig::d5005().uuid as u64);
    for (file, from, to, says) in [
        ("hw/rtl/logos_bringup_core.v", ": pattern(seed, line_of(base, sent[b]));".to_string(), ": pattern(seed ^ 32'h1, line_of(base, sent[b]));".to_string(), "lines read back differ"),
        ("hw/rtl/logos_bringup_core.v", "{{(DWM-DWH){1'b0}}, hr_data}".to_string(), "{{(DWM-DWH){1'b0}}, ~hr_data}".to_string(), "host lines do not come back unchanged"),
        ("sim/logos_d5005_cosim.v", low_id, "AFU_ID_L = 64'h0000000000000001".to_string(), "the AFU ID is not the one opened"),
        (
            "hw/rtl/logos_bringup_core.v",
            "if (op == CHECK && mr_data[i*DWM +: DWM] !== pattern(seed, line_of(base, seen[i]))) wrong[i]".to_string(),
            "if (op == CHECK && 1'b0) wrong[i]".to_string(),
            "a check under another seed does not differ on every line",
        ),
    ] {
        let (from, to) = (from.as_str(), to.as_str());
        let d = emit(&AfuConfig::d5005());
        let path = d.path().join(file);
        let source = std::fs::read_to_string(&path).unwrap();
        assert_eq!(source.matches(from).count(), 1, "the mutation site {from:?}");
        std::fs::write(&path, source.replace(from, to)).unwrap();
        let (code, out) = cosim(d.path(), 256, 64);
        assert_eq!(code, Some(1), "{to:?}:\n{out}");
        assert!(out.contains(says), "{to:?}:\n{out}");
        assert!(receipt_json(&out)["checks_failed"].as_u64().unwrap() > 0);
    }
}

/// The receipt, the image loader and the OPAE backend compile against OPAE 2.12's headers, warnings as errors.
#[test]
fn the_card_build_compiles_against_opae() {
    if !common::require_tool("opae-sdk") || !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let o = Command::new("cc")
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-fsyntax-only", "-I"])
        .arg(d.path().join("sw"))
        .arg("-I")
        .arg(root().join("tools/d5005/opae-sdk/include"))
        .arg(d.path().join("sw/logos_hal_opae.c"))
        .arg(d.path().join("sw/logos_receipt.c"))
        .arg(d.path().join("sw/logos_load.c"))
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
}

/// The receipt and the image loader build and link against the physical backend, which needs nothing but libc.
#[test]
fn the_card_build_links_against_the_physical_backend() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    for program in ["logos_receipt", "logos_load"] {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(d.path().join("sw"))
            .arg(d.path().join("sw/logos_hal_pci.c"))
            .arg(d.path().join(format!("sw/{program}.c")))
            .arg("-o")
            .arg(d.path().join(program))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{program}: {}", text(&o));
    }
}

/// One PCI function of the device tree the physical backend reads, laid out as sysfs lays out
/// `/sys/bus/pci/devices/<function>`: its vendor and device, BAR 0 a file of MMIO words opening with a feature header
/// and the AFU ID `id`, its configuration space with the command word `0x0400` (interrupts off, no decoding, no bus
/// mastering), the driver bound to it, and the type of its IOMMU group.
fn lay_function(devices: &Path, function: &str, vendor: u16, device: u16, id: u128, driver: Option<&str>, iommu: Option<&str>) {
    let f = devices.join(function);
    std::fs::create_dir_all(&f).unwrap();
    std::fs::write(f.join("vendor"), format!("0x{vendor:04x}\n")).unwrap();
    std::fs::write(f.join("device"), format!("0x{device:04x}\n")).unwrap();
    let mut bar = vec![0u8; 4096];
    bar[0..8].copy_from_slice(&0x1000_0000_0000_1001u64.to_le_bytes());
    bar[8..16].copy_from_slice(&(id as u64).to_le_bytes());
    bar[16..24].copy_from_slice(&((id >> 64) as u64).to_le_bytes());
    std::fs::write(f.join("resource0"), bar).unwrap();
    let mut config = vec![0u8; 256];
    config[0..2].copy_from_slice(&vendor.to_le_bytes());
    config[2..4].copy_from_slice(&device.to_le_bytes());
    config[4..6].copy_from_slice(&0x0400u16.to_le_bytes());
    std::fs::write(f.join("config"), config).unwrap();
    if let Some(driver) = driver {
        let d = devices.parent().unwrap().join("drivers").join(driver);
        std::fs::create_dir_all(&d).unwrap();
        std::os::unix::fs::symlink(&d, f.join("driver")).unwrap();
    }
    if let Some(kind) = iommu {
        std::fs::create_dir_all(f.join("iommu_group")).unwrap();
        std::fs::write(f.join("iommu_group/type"), format!("{kind}\n")).unwrap();
    }
}

fn command_word(devices: &Path, function: &str) -> u16 {
    let c = std::fs::read(devices.join(function).join("config")).unwrap();
    u16::from_le_bytes([c[4], c[5]])
}

/// The physical backend's probe (`fixtures/d5005_flow/hal_pci_probe.c`) built over `sw/logos_hal_pci.c`.
fn pci_probe(d: &Path) -> PathBuf {
    let probe = d.join("hal_pci_probe");
    let o = Command::new("cc")
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(d.join("sw"))
        .arg(d.join("sw/logos_hal_pci.c"))
        .arg(fixture("hal_pci_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
    probe
}

fn run_probe(probe: &Path, devices: &Path, pinned: Option<&str>, function: &str, buffer: Option<u64>) -> (Option<i32>, String) {
    let mut c = Command::new(probe);
    c.arg(AfuConfig::d5005().uuid_text()).arg(function).env("LOGOS_PCI_DEVICES", devices).env_remove("LOGOS_D5005_VF");
    if let Some(b) = buffer {
        c.arg(b.to_string());
    }
    if let Some(p) = pinned {
        c.env("LOGOS_D5005_VF", p);
    }
    let o = c.output().expect("the probe");
    (o.status.code(), text(&o))
}

/// The physical backend opens the accelerator on the one function that is a VF of the OFS FIM's PF (8086:bccf), has
/// no driver bound, and carries the AFU ID at MMIO words 1 and 2; it decodes memory and masters the bus only while
/// open and gives the function back its command word on close; every function it looked at and did not open has its
/// command word back; and its MMIO words are BAR 0's.
#[test]
fn the_physical_backend_opens_the_accelerator_by_its_afu_id_on_a_driverless_vf() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = pci_probe(d.path());
    let id = AfuConfig::d5005().uuid;
    let devices = d.path().join("devices");
    lay_function(&devices, "0000:01:00.0", 0x8086, 0xbcce, id, Some("dfl-pci"), None);
    lay_function(&devices, "0000:01:00.1", 0x8086, 0xbccf, id ^ 1, None, None);
    lay_function(&devices, "0000:01:00.2", 0x8086, 0xbccf, id, Some("vfio-pci"), None);
    lay_function(&devices, "0000:01:00.3", 0x8086, 0xbccf, id, None, None);
    lay_function(&devices, "0000:02:00.0", 0x144d, 0xa808, id, None, None);
    let (code, out) = run_probe(&probe, &devices, None, "0000:01:00.3", None);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains(&format!("id {id:032x}\n")), "{out}");
    assert!(out.contains("open command 0x0406\n"), "{out}");
    assert!(out.contains("scratch 05ca1ab1e0ddba11\n"), "{out}");
    assert!(out.contains("closed command 0x0400\n"), "{out}");
    let bar = std::fs::read(devices.join("0000:01:00.3/resource0")).unwrap();
    assert_eq!(u64::from_le_bytes(bar[0x28..0x30].try_into().unwrap()), 0x05ca1ab1e0ddba11, "SCRATCH is BAR 0's word 5");
    for other in ["0000:01:00.0", "0000:01:00.1", "0000:01:00.2", "0000:02:00.0"] {
        assert_eq!(command_word(&devices, other), 0x0400, "{other}'s command word");
        let bar = std::fs::read(devices.join(other).join("resource0")).unwrap();
        assert!(bar[0x18..].iter().all(|&b| b == 0), "{other}'s BAR 0 is written");
    }
}

/// With LOGOS_D5005_VF the physical backend opens that function or none; with no function carrying the AFU ID it opens
/// none and says where the port goes; and a VF behind an IOMMU that translates its addresses is refused for VFIO, one in
/// an identity domain opened.
#[test]
fn the_physical_backend_refuses_what_it_cannot_drive() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = pci_probe(d.path());
    let id = AfuConfig::d5005().uuid;
    let devices = d.path().join("devices");
    lay_function(&devices, "0000:01:00.1", 0x8086, 0xbccf, id ^ 1, None, None);
    lay_function(&devices, "0000:01:00.3", 0x8086, 0xbccf, id, None, None);
    let (code, out) = run_probe(&probe, &devices, Some("0000:01:00.1"), "0000:01:00.3", None);
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("0000:01:00.1"), "{out}");
    assert_eq!(command_word(&devices, "0000:01:00.1"), 0x0400);
    assert_eq!(command_word(&devices, "0000:01:00.3"), 0x0400, "a function the pin leaves out is never touched");
    let (code, out) = run_probe(&probe, &devices, Some("0000:01:00.3"), "0000:01:00.3", None);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("open command 0x0406\n"), "{out}");

    for (what, decoy) in [("its low word", id ^ 1), ("its high word", id ^ (1u128 << 64))] {
        let none = d.path().join(format!("none-{}", decoy == id ^ 1));
        lay_function(&none, "0000:01:00.1", 0x8086, 0xbccf, decoy, None, None);
        let (code, out) = run_probe(&probe, &none, None, "0000:01:00.1", None);
        assert_eq!(code, Some(2), "an AFU ID differing in {what}:\n{out}");
        assert!(out.contains("afu-vf.sh"), "{out}");
        assert_eq!(command_word(&none, "0000:01:00.1"), 0x0400, "{what}");
    }

    // Each a tree of its own, so no scan order hides it: the AFU ID on the FIM's PF, on another vendor's function, and
    // on a VF a driver holds, none of them opened or touched.
    for (what, vendor, device, driver) in [
        ("the PF", 0x8086, 0xbcce, None),
        ("another vendor's function", 0x144d, 0xbccf, None),
        ("a VF vfio-pci holds", 0x8086, 0xbccf, Some("vfio-pci")),
    ] {
        let tree = d.path().join(format!("only-{vendor:x}-{device:x}-{}", driver.is_some()));
        lay_function(&tree, "0000:01:00.4", vendor, device, id, driver, None);
        let (code, out) = run_probe(&probe, &tree, None, "0000:01:00.4", None);
        assert_eq!(code, Some(2), "{what}:\n{out}");
        assert_eq!(command_word(&tree, "0000:01:00.4"), 0x0400, "{what}");
    }

    for (kind, opens) in [("DMA", false), ("DMA-FQ", false), ("identity", true)] {
        let tree = d.path().join(format!("iommu-{kind}"));
        lay_function(&tree, "0000:01:00.1", 0x8086, 0xbccf, id, None, Some(kind));
        let (code, out) = run_probe(&probe, &tree, None, "0000:01:00.1", None);
        if opens {
            assert_eq!(code, Some(0), "{kind}: {out}");
        } else {
            assert_eq!(code, Some(2), "{kind}: {out}");
            assert!(out.contains("IOMMU") && out.contains("logos_hal_opae.c"), "{kind}: {out}");
            assert_eq!(command_word(&tree, "0000:01:00.1"), 0x0400, "{kind}");
        }
    }
}

/// A buffer the physical backend cannot place at a physical address it has read (no physical frames for this user, no
/// huge page free, or more than one huge page) is refused: the accelerator is never handed an address nobody knows. It
/// runs where the page map hides the frames (`hidden-frames`), the case the refusal is for.
#[test]
fn the_physical_backend_never_hands_out_a_buffer_it_cannot_place() {
    if !common::require_tool("cc") || !common::require_tool("hidden-frames") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = pci_probe(d.path());
    let devices = d.path().join("devices");
    lay_function(&devices, "0000:01:00.3", 0x8086, 0xbccf, AfuConfig::d5005().uuid, None, None);
    for bytes in [4096u64, 2 << 20, (1 << 30) + 1] {
        let (code, out) = run_probe(&probe, &devices, None, "0000:01:00.3", Some(bytes));
        assert_eq!(code, Some(0), "{bytes}: {out}");
        assert!(out.contains("buffer refused 0x0\n"), "{bytes}: {out}");
        assert!(out.contains("logos_buffer"), "{bytes}: {out}");
        assert!(out.contains("closed command 0x0400\n"), "{bytes}: {out}");
    }
}

/// The core's status port when a command's own record and a progress record are owed in the same clock
/// (`fixtures/d5005_flow/tb_core_status.v`): the command's record is the one taken, with its tag, its code and the count
/// of completed commands, and no progress record follows it.
#[test]
fn a_commands_own_record_goes_before_a_progress_record() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let out = simulate(&[fixture("tb_core_status.v"), rtl(d.path(), "logos_bringup_core.v")], &[]);
    assert!(out.contains("finals=1 after=0") && out.contains("errors=0"), "{out}");
    assert!(!out.contains("FAIL"), "{out}");
}

/// The fields `fixtures/d5005_flow/core_wait_probe.c` prints of the record a wait returned: status, result0 and the
/// device's ticks.
fn core_wait(probe: &Path, scenario: &str) -> (u64, u64, u64) {
    let o = Command::new(probe).arg(scenario).output().expect("the probe");
    assert!(o.status.success(), "{scenario}: {}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let field = |name: &str| -> u64 {
        let v = out.split_whitespace().skip_while(|w| *w != name).nth(1).unwrap_or_else(|| panic!("{scenario}: no {name} in {out}"));
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).unwrap(),
            None => v.parse().unwrap(),
        }
    };
    (field("status"), field("result0"), field("ticks"))
}

/// A wait that gives a command up returns the last progress record the command cut, so the host can say where the
/// command stood when it stopped moving (an engine's record carries each context's job word): `fixtures/d5005_flow/
/// core_wait_probe.c`'s command whose progress stops after its seventh record, cut at tick 280, is given up with that
/// record's results, and one that cut none with none.
#[test]
fn a_command_given_up_returns_the_last_progress_it_made() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = d.path().join("core_wait_probe");
    let o = Command::new("cc")
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-DLOGOS_CORE_MAX_POLLS=64", "-I"])
        .arg(d.path().join("sw"))
        .arg(fixture("core_wait_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
    let (status, result0, _) = core_wait(&probe, "stopped");
    assert_eq!((status, result0), (0xff, 7), "given up with its last progress record's results");
    let (status, result0, _) = core_wait(&probe, "silent");
    assert_eq!((status, result0), (0xff, 0), "given up with none");
}

/// A host's wait for a command (`sw/logos_core.h`) is bounded by the command's progress, not its length: with a budget
/// of 64 register reads, a command whose progress records keep moving is waited for past fifteen budgets and its own
/// record returned; one whose progress stops is given up within two budgets of its last record; progress records of
/// another command, records of this one that are not progress records, or none, hold the wait no longer than one
/// budget; and a record that lands as the wait looks at the status once its budget is spent is the command's, not a
/// give-up.
#[test]
fn a_command_is_waited_for_while_its_progress_moves() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = d.path().join("core_wait_probe");
    let o = Command::new("cc")
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-DLOGOS_CORE_MAX_POLLS=64", "-I"])
        .arg(d.path().join("sw"))
        .arg(fixture("core_wait_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
    let ok = 0x2a << 8 | STATUS_OK as u64;
    assert_eq!(core_wait(&probe, "moving"), (ok, 7, 1000), "a command whose progress moves: its own record, its tag 0x2a");
    let (status, _, ticks) = core_wait(&probe, "stopped");
    assert_eq!(status, 0xff, "a command whose progress stops is given up");
    assert!(ticks > 300 && ticks <= 280 + 2 * 66, "given up {ticks} ticks in, its last progress at 280");
    for scenario in ["other", "unsequenced", "silent"] {
        let (status, _, ticks) = core_wait(&probe, scenario);
        assert_eq!(status, 0xff, "{scenario}");
        assert!(ticks <= 67, "{scenario}: given up {ticks} ticks in, one budget is 66");
    }
    assert_eq!(core_wait(&probe, "late"), (ok, 7, 67), "a record landing as the wait looks");
}

/// The physical backend places a buffer only on frames it has read: over a page map laid out in a file (an entry a
/// 4 KiB page, bit 63 present, bits 54..0 the frame), a huge page whose frames are present and consecutive is placed at
/// its first frame's address whatever the entries' flag bits between 55 and 62 say, frame to frame, and one with a frame
/// absent, a frame
/// of zero (what an unprivileged reader is given, even where the rest count up from it), or frames out of order is
/// refused.
#[test]
fn the_physical_backend_places_a_buffer_only_on_frames_it_has_read() {
    if !common::require_tool("cc") {
        return;
    }
    let d = emit(&AfuConfig::d5005());
    let probe = d.path().join("hal_pci_frames_probe");
    let o = Command::new("cc")
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(d.path().join("sw"))
        .arg(fixture("hal_pci_frames_probe.c"))
        .arg("-o")
        .arg(&probe)
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
    const PAGE: u64 = 2 << 20;
    let at: u64 = 0x7f00_4000_0000;
    let frames = PAGE / 4096;
    let present = 1u64 << 63;
    let first = 0x12_3400u64;
    let lay = |name: &str, entry: &dyn Fn(u64) -> u64| -> PathBuf {
        let map = d.path().join(name);
        let f = std::fs::File::create(&map).unwrap();
        use std::os::unix::fs::FileExt;
        let entries: Vec<u8> = (0..frames).flat_map(|k| entry(k).to_le_bytes()).collect();
        f.write_all_at(&entries, at / 4096 * 8).unwrap();
        map
    };
    let place = |map: &Path| -> String {
        let o = Command::new(&probe).arg(map).arg(format!("{at:#x}")).arg(PAGE.to_string()).output().expect("the probe");
        assert!(o.status.success(), "{}", text(&o));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let flags = |k: u64| ((k * 0x35) & 0xff) << 55;
    assert_eq!(place(&lay("whole", &|k| present | (first + k))), format!("placed {:#x}", first * 4096));
    assert_eq!(place(&lay("flagged", &|k| present | flags(k) | (first + k))), format!("placed {:#x}", first * 4096), "flag bits");
    for (name, entry) in [
        ("absent", &(|k| if k == frames / 2 { first + k } else { present | (first + k) }) as &dyn Fn(u64) -> u64),
        ("zero", &|_| present),
        ("counting up from frame zero", &|k| present | k),
        ("unprivileged", &|k| if k == frames - 1 { present } else { present | (first + k) }),
        ("out of order", &|k| present | (first + if k == 7 { 9 } else { k })),
    ] {
        assert_eq!(place(&lay(name, entry)), "refused", "{name}");
    }
}

fn receipt_fixture(checks_failed: u64, mismatches: u64, round_trip: bool, afu_id: &str) -> String {
    let channels: Vec<String> = [1, 2, 4, 8, 15]
        .iter()
        .map(|m| format!("{{\"mask\": {m}, \"lines\": 1048576, \"fill_cycles\": 2000000, \"check_cycles\": 1900000, \"mismatches\": {mismatches}, \"write_gb_per_s\": 17.0, \"read_gb_per_s\": 19.5}}"))
        .collect();
    format!(
        "{{\"afu_id\": \"{afu_id}\", \"build\": 0, \"user_clock_hz\": 437500000, \"line_bytes\": 72, \"channels\": [{}], \"host\": {{\"lines\": 4096, \"to_mem_cycles\": 9000, \"to_host_cycles\": 9800, \"round_trip\": {round_trip}}}, \"checks_failed\": {checks_failed}}}\n",
        channels.join(", ")
    )
}

/// A receipt that passed on a recorded image is recorded once; one with a failed check, a channel that read back
/// wrong, a broken round trip, an image nobody recorded, or another accelerator's ID is refused.
#[test]
fn the_receipt_recorder_refuses_what_did_not_pass() {
    if !common::require_tool("python3") {
        return;
    }
    let uuid = AfuConfig::d5005().uuid_text();
    let d = tempfile::tempdir().unwrap();
    let gbs = d.path().join("image.gbs");
    std::fs::write(&gbs, b"a partial bitstream").unwrap();
    let sha = {
        let o = Command::new("sha256sum").arg(&gbs).output().unwrap();
        String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap().to_string()
    };
    let images = d.path().join("afu_images.json");
    std::fs::write(&images, format!("{{\"images\": [{{\"sha256\": \"{sha}\", \"afu_uuid\": \"{uuid}\"}}]}}")).unwrap();
    let record = |receipt: &str, gbs: &Path, ledger: &Path| -> Output {
        let r = d.path().join("receipt.json");
        std::fs::write(&r, receipt).unwrap();
        Command::new("python3").arg(script("record_receipt.py")).arg(&r).arg(gbs).arg(&images).arg(ledger).output().unwrap()
    };
    let ledger = d.path().join("receipts.json");
    let o = record(&receipt_fixture(0, 0, true, &uuid), &gbs, &ledger);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
    assert_eq!(l["receipts"][0]["gbs_sha256"], sha.as_str());
    assert_eq!(l["receipts"][0]["channels"].as_array().unwrap().len(), 5);
    let o = record(&receipt_fixture(0, 0, true, &uuid), &gbs, &ledger);
    assert_eq!(o.status.code(), Some(1), "the same receipt twice: {}", text(&o));
    let other = d.path().join("other.gbs");
    std::fs::write(&other, b"an image nobody recorded").unwrap();
    for (receipt, gbs, says) in [
        (receipt_fixture(2, 0, true, &uuid), &gbs, "2 checks failed"),
        (receipt_fixture(0, 3, true, &uuid), &gbs, "read back other lines"),
        (receipt_fixture(0, 0, false, &uuid), &gbs, "did not come back unchanged"),
        (receipt_fixture(0, 0, true, &uuid), &other, "is not in"),
        (receipt_fixture(0, 0, true, "00000000-0000-0000-0000-000000000000"), &gbs, "the receipt names the AFU"),
    ] {
        let fresh = d.path().join("fresh.json");
        let o = record(&receipt, gbs, &fresh);
        assert_eq!(o.status.code(), Some(1), "{says}: {}", text(&o));
        assert!(text(&o).contains(says), "{says}: {}", text(&o));
        assert!(!fresh.exists());
    }
}

/// `run-step.sh` on a host of stand-ins (fpgaconf, userclk, sudo, cc and `afu-vf.sh`, each logging its call): a persona
/// whose AFU JSON fixes its user clock has fpgaconf set that clock through the port's user-clock interface, a uio device
/// only root opens, so fpgaconf runs as root; with `LOGOS_D5005_USERCLK_MHZ` the user clock is then set to it, its low
/// clock to half of it, on the PF's bus, before the VF is attached, and the step program is told the clock; without it
/// no clock is set and the program is told none.
#[test]
fn run_step_configures_a_fixed_clock_persona_as_root_at_the_clock_asked() {
    use std::os::unix::fs::PermissionsExt;
    if !common::require_tool("bash") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let calls = tmp.path().join("calls");
    let bin = tmp.path().join("bin");
    let scripts = tmp.path().join("scripts/d5005");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&scripts).unwrap();
    let stub = |path: &Path, body: &str| {
        std::fs::write(path, format!("#!/usr/bin/env bash\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let log = calls.display().to_string();
    stub(&bin.join("sudo"), &format!("echo \"sudo $*\" >> {log}\n[ \"$1\" = -n ] && shift\nexec \"$@\""));
    for tool in ["fpgaconf", "userclk", "cc"] {
        stub(&bin.join(tool), &format!("echo \"{tool} $*\" >> {log}"));
    }
    std::fs::copy(script("run-step.sh"), scripts.join("run-step.sh")).unwrap();
    stub(
        &scripts.join("afu-vf.sh"),
        &format!("echo \"afu-vf.sh $1 clock=${{LOGOS_USER_CLOCK_MHZ:-none}}\" >> {log}\n[ \"$1\" = build ] && : > \"$4\"\nexit 0"),
    );
    let afu = tmp.path().join("afu");
    std::fs::create_dir_all(afu.join("sw")).unwrap();
    std::fs::write(afu.join("sw/logos_d5005.h"), "#define LOGOS_D5005_ENGINE 1\n").unwrap();
    let program = tmp.path().join("program");
    std::fs::create_dir_all(&program).unwrap();
    let run = |clock: Option<&str>| -> Vec<String> {
        let _ = std::fs::remove_file(&calls);
        let mut c = Command::new("bash");
        c.arg(scripts.join("run-step.sh")).arg(&afu).arg("persona.gbs").arg(&program).args(["3", "760", "6511"]);
        c.env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()));
        c.env("LOGOS_D5005_VF", "0000:01:00.2").env("LOGOS_D5005_PF", "0000:3b:00.0").env_remove("LOGOS_USER_CLOCK_MHZ");
        match clock {
            Some(m) => c.env("LOGOS_D5005_USERCLK_MHZ", m),
            None => c.env_remove("LOGOS_D5005_USERCLK_MHZ"),
        };
        let o = c.output().unwrap();
        assert!(o.status.success(), "{clock:?}: {}", text(&o));
        std::fs::read_to_string(&calls).unwrap().lines().map(str::to_string).collect()
    };
    let fpgaconf = format!("sudo -n {}/fpgaconf persona.gbs", bin.display());
    let userclk = format!("sudo -n {}/userclk -B 0x3b -H 40 -L 20", bin.display());
    let at = |calls: &[String], line: &str| calls.iter().position(|c| c == line).unwrap_or_else(|| panic!("`{line}` in {calls:?}"));
    let slow = run(Some("40"));
    assert!(at(&slow, &fpgaconf) < at(&slow, &userclk), "{slow:?}");
    assert!(at(&slow, &userclk) < at(&slow, "afu-vf.sh attach clock=40"), "the clock set before the VF is attached: {slow:?}");
    assert!(slow.contains(&"afu-vf.sh exec clock=40".to_string()), "{slow:?}");
    let asked = run(None);
    assert!(asked.contains(&fpgaconf), "{asked:?}");
    assert!(!asked.iter().any(|c| c.contains("userclk")), "no clock set unless asked: {asked:?}");
    assert!(asked.contains(&"afu-vf.sh exec clock=none".to_string()), "{asked:?}");
}

// ── SECDED lines ────────────────────────────────────────────────────────────────────────────────────

/// The word bus bit `i` of a SECDED line belongs to: a data bit's own, an ECC lane's the word whose check byte it holds.
fn word_of(i: usize) -> usize {
    if i < 512 {
        i / 64
    } else {
        (0..SECDED_WORDS).find(|&w| secded_check_byte(w) == (i - 512) / 8).expect("every ECC lane holds a word's check byte")
    }
}

fn flipped(bus: &[u8; 72], bits: &[usize]) -> [u8; 72] {
    let mut b = *bus;
    for &i in bits {
        b[i / 8] ^= 1 << (i % 8);
    }
    b
}

fn next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn random_line(s: &mut u64) -> [u8; 64] {
    let mut d = [0u8; 64];
    for chunk in d.chunks_mut(8) {
        chunk.copy_from_slice(&next(s).to_le_bytes());
    }
    d
}

/// A SECDED line keeps its 64 data bytes where they are, and holds in each word's ECC lane the check bits of that
/// word's `scratchpad_rtl` codeword, bit `j` the codeword's bit at `SECDED_CHECKS[j]`; it reads back clean.
#[test]
fn a_secded_line_is_its_data_and_each_words_codeword_checks() {
    let mut s = 0x5ec_ded5_eed;
    let mut lines = vec![[0u8; 64], [0xff; 64]];
    lines.extend((0..254).map(|_| random_line(&mut s)));
    for d in lines {
        let bus = secded_line(&d);
        assert_eq!(&bus[..64], &d[..], "the data bytes stay where they are");
        for w in 0..SECDED_WORDS {
            let cw = encode(u64::from_le_bytes(d[8 * w..8 * w + 8].try_into().unwrap()));
            let check = bus[64 + secded_check_byte(w)];
            for (j, &p) in SECDED_CHECKS.iter().enumerate() {
                assert_eq!(check >> j & 1, (cw >> p & 1) as u8, "word {w}'s check bit {j}");
            }
        }
        assert_eq!(secded_read(&bus), SecdedRead { data: d, corrected: 0, uncorrectable: 0 });
    }
}

/// Every one of a line's 576 bus bits flipped alone is corrected and charged to its own word; every two flips in one
/// word's 72 bits are found uncorrectable, that word's data as read and every other word untouched; a flip in each of
/// two words corrects both.
#[test]
fn a_secded_line_corrects_one_flip_a_word_and_finds_two() {
    let mut s = 0x2b1d_ecc0_0001;
    for round in 0..3 {
        let d = random_line(&mut s);
        let bus = secded_line(&d);
        for i in 0..576 {
            assert_eq!(secded_read(&flipped(&bus, &[i])), SecdedRead { data: d, corrected: 1 << word_of(i), uncorrectable: 0 }, "bus bit {i}");
        }
        if round == 0 {
            for w in 0..SECDED_WORDS {
                let bits: Vec<usize> = (0..576).filter(|&i| word_of(i) == w).collect();
                assert_eq!(bits.len(), 72, "word {w} is one codeword");
                for (x, &a) in bits.iter().enumerate() {
                    for &b in &bits[x + 1..] {
                        let bad = flipped(&bus, &[a, b]);
                        let read: [u8; 64] = bad[..64].try_into().unwrap();
                        assert_eq!(secded_read(&bad), SecdedRead { data: read, corrected: 0, uncorrectable: 1 << w }, "bus bits {a} and {b}");
                    }
                }
            }
        }
        for _ in 0..2000 {
            let (a, b) = ((next(&mut s) % 576) as usize, (next(&mut s) % 576) as usize);
            if word_of(a) != word_of(b) {
                let r = secded_read(&flipped(&bus, &[a, b]));
                assert_eq!(r, SecdedRead { data: d, corrected: 1 << word_of(a) | 1 << word_of(b), uncorrectable: 0 }, "bus bits {a} and {b}");
            }
        }
    }
}

/// The DQ pin bus bit `i` is driven on under each order the controller may give its 576-bit word: the burst's eight
/// beats of all 72 pins (0); the data pins' eight beats beneath the ECC pins' eight (1); each group of eight data pins'
/// beats in turn, the ECC pins' last (2).
fn pin(order: usize, i: usize) -> usize {
    match order {
        0 => i % 72,
        1 if i < 512 => i % 64,
        2 if i < 512 => 8 * (i / 64) + i % 8,
        _ => 64 + (i - 512) % 8,
    }
}

/// Whether every one of a set of 72 bus bits lies on its own pin under `order`.
fn on_72_pins(order: usize, bits: &[usize]) -> bool {
    let mut seen = [false; 72];
    bits.iter().all(|&i| !std::mem::replace(&mut seen[pin(order, i)], true))
}

/// Each word's codeword — its 64 data bits and its ECC lane — lies on 72 different pins whether the bus word runs
/// beat by beat over all 72 pins or holds the data pins' beats beneath the ECC pins', so a failing pin flips one bit
/// of each codeword; word `w` with lane `w` would share pins under the first. Under the third order a codeword of whole
/// bytes on 72 pins under the second and third takes one data byte of each pin group and of each beat — bytes
/// `8u + σ(u)` for a permutation σ — and one ECC byte, and none of those lies on 72 pins under the first: no whole-byte
/// grouping serves all three.
#[test]
fn every_codeword_lies_on_72_pins_under_either_burst_order() {
    let codeword = |w: usize, lane: usize| -> Vec<usize> { (64 * w..64 * w + 64).chain(512 + 8 * lane..512 + 8 * lane + 8).collect() };
    for order in [0, 1] {
        for w in 0..SECDED_WORDS {
            assert!(on_72_pins(order, &codeword(w, secded_check_byte(w))), "word {w} under order {order}");
        }
    }
    assert!((0..SECDED_WORDS).any(|w| !on_72_pins(0, &codeword(w, w))), "word w with lane w under the beat-by-beat order");
    let mut sigma: Vec<usize> = (0..8).collect();
    let mut seen = 0;
    loop {
        for e in 0..8 {
            let bytes = (0..8).map(|u| 8 * u + sigma[u]).chain([64 + e]);
            let bits: Vec<usize> = bytes.flat_map(|y| 8 * y..8 * y + 8).collect();
            assert!(on_72_pins(1, &bits) && on_72_pins(2, &bits), "σ {sigma:?}, lane {e}");
            assert!(!on_72_pins(0, &bits), "σ {sigma:?}, lane {e} would serve all three orders");
            seen += 1;
        }
        let Some(i) = (0..7).rev().find(|&i| sigma[i] < sigma[i + 1]) else { break };
        let j = (i + 1..8).rev().find(|&j| sigma[j] > sigma[i]).unwrap();
        sigma.swap(i, j);
        sigma[i + 1..].reverse();
    }
    assert_eq!(seen, 40320 * 8);
}

/// Hex of `bytes` as a Verilog word, its last byte most significant.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().rev().map(|b| format!("{b:02x}")).collect()
}

/// The emitted `logos_secded_encode` codes every line as `secded_line` does and `logos_secded_decode` reads every bus
/// line as `secded_read` does: zero, all ones and every one-hot line coded; every single flip of three lines, every two
/// flips within each word of one, flips across words, three flips and arbitrary bus words read.
#[test]
fn the_emitted_secded_coder_and_decoder_are_the_code() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut s = 0xc0de_c0de_0072;
    let mut lines = vec![[0u8; 64], [0xff; 64]];
    for i in 0..512 {
        let mut d = [0u8; 64];
        d[i / 8] = 1 << (i % 8);
        lines.push(d);
    }
    lines.extend((0..64).map(|_| random_line(&mut s)));
    let enc: String = lines.iter().map(|d| format!("{}{}\n", hex(&secded_line(d)), hex(d))).collect();
    let mut reads: Vec<[u8; 72]> = lines.iter().map(secded_line).collect();
    for k in 0..3 {
        let bus = secded_line(&lines[2 + 512 + k]);
        reads.extend((0..576).map(|i| flipped(&bus, &[i])));
        if k == 0 {
            for w in 0..SECDED_WORDS {
                let bits: Vec<usize> = (0..576).filter(|&i| word_of(i) == w).collect();
                for (x, &a) in bits.iter().enumerate() {
                    reads.extend(bits[x + 1..].iter().map(|&b| flipped(&bus, &[a, b])));
                }
            }
        }
        for _ in 0..512 {
            let (a, b, c) = ((next(&mut s) % 576) as usize, (next(&mut s) % 576) as usize, (next(&mut s) % 576) as usize);
            reads.push(flipped(&bus, &[a, b]));
            reads.push(flipped(&bus, &[a, b, c]));
        }
    }
    for _ in 0..256 {
        let mut bus = [0u8; 72];
        bus[..64].copy_from_slice(&random_line(&mut s));
        bus[64..].copy_from_slice(&next(&mut s).to_le_bytes());
        reads.push(bus);
    }
    let dec: String = reads
        .iter()
        .map(|bus| {
            let r = secded_read(bus);
            format!("{:02x}{:02x}{}{}\n", r.uncorrectable, r.corrected, hex(&r.data), hex(bus))
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("enc.hex"), enc).unwrap();
    std::fs::write(dir.path().join("dec.hex"), dec).unwrap();
    let d = emit(&AfuConfig { line: LocalMemoryLine::Data512Secded, ..AfuConfig::d5005() });
    let sim = dir.path().join("sim");
    let build = Command::new("iverilog")
        .arg("-g2012")
        .arg(format!("-Ptb.NENC={}", lines.len()))
        .arg(format!("-Ptb.NDEC={}", reads.len()))
        .arg("-o")
        .arg(&sim)
        .arg(fixture("tb_secded_line.v"))
        .arg(rtl(d.path(), "logos_secded.v"))
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog:\n{}", text(&build));
    let run = Command::new("vvp").arg(&sim).current_dir(dir.path()).output().expect("vvp");
    let out = text(&run);
    assert!(out.contains(&format!("RESULT coded={} read={} errors=0", lines.len(), reads.len())), "{out}");
}

/// A SECDED channel writes 300 lines in bursts, most with flips injected after they are coded — one data bit, two in
/// one word, one check bit, one in each of two words — and reads them back through its decoder's stage and a 64-line
/// FIFO a consumer drains at 10% to 100% of its cycles, or in short runs between long stops that fill the FIFO to its
/// last line: every line's data and its words' verdicts are what its flips make, and no line arrives at a full FIFO.
#[test]
fn a_secded_channel_reads_back_what_its_flips_make() {
    if !common::require_tool("iverilog") {
        return;
    }
    let d = emit(&AfuConfig { line: LocalMemoryLine::Data512Secded, ..AfuConfig::d5005() });
    for (seed, rp, stop) in [(3, 30, 0), (4, 100, 0), (5, 10, 0), (6, 60, 0), (7, 100, 1), (8, 100, 1), (9, 70, 1)] {
        let out = simulate(
            &[
                fixture("tb_stream_secded.v"),
                d.path().join("sim/logos_avmm_memory_model.v"),
                rtl(d.path(), "logos_secded.v"),
                rtl(d.path(), "logos_stream.v"),
                rtl(d.path(), "logos_dcfifo.v"),
            ],
            &[format!("-Ptb.SEED={seed}"), format!("-Ptb.RP={rp}"), format!("-Ptb.STOP={stop}")],
        );
        assert!(out.contains("RESULT written=300 got=300 errors=0 owed=0"), "seed {seed}, consumer {rp}%, stopping {stop}:\n{out}");
    }
}

/// The receipt proves the code on every SECDED channel against the RTL: nothing corrected or lost before a flip, a
/// flipped data bit and a flipped check bit corrected on every line, two flips in one word found uncorrectable on every
/// line, a flip in each of two words both corrected, a clean fill clean; its lines are 64 bytes, and the build without
/// SECDED reports no code, its core refusing both of the code's ops (a receipt check).
#[test]
fn the_receipt_proves_the_secded_code_on_every_channel() {
    if !cosim_tools() {
        return;
    }
    let d = emit(&AfuConfig { line: LocalMemoryLine::Data512Secded, ..AfuConfig::d5005() });
    let (code, out) = cosim(d.path(), 256, 64);
    assert_eq!(code, Some(0), "the receipt on SECDED lines:\n{out}");
    let r = receipt_json(&out);
    assert_eq!(r["checks_failed"], 0, "{out}");
    assert_eq!(r["line_bytes"], 64);
    let codes = r["secded"].as_array().unwrap_or_else(|| panic!("a SECDED section in {out}"));
    assert_eq!(codes.len(), BANKS as usize);
    for (b, c) in codes.iter().enumerate() {
        let n = c["lines"].as_u64().unwrap();
        assert_eq!((c["channel"].as_u64(), n), (Some(b as u64), 64));
        let case = |name: &str| (c[name]["mismatches"].as_u64(), c[name]["corrected"].as_u64(), c[name]["uncorrectable"].as_u64());
        assert_eq!(case("before"), (None, Some(0), Some(0)), "{c}");
        assert_eq!(case("data_bit"), (Some(0), Some(n), Some(0)), "{c}");
        assert_eq!(case("check_bit"), (Some(0), Some(n), Some(0)), "{c}");
        assert_eq!(case("two_bits_one_word"), (Some(n), Some(0), Some(n)), "{c}");
        assert_eq!(case("two_words"), (Some(0), Some(2 * n), Some(0)), "{c}");
        assert_eq!(case("clean"), (Some(0), Some(0), Some(0)), "{c}");
    }
    let plain = emit(&AfuConfig::d5005());
    let (code, out) = cosim(plain.path(), 256, 64);
    assert_eq!(code, Some(0), "{out}");
    assert!(receipt_json(&out).get("secded").is_none(), "{out}");
}

/// The receipt on SECDED lines fails when the RTL is wrong: a decoder stage passing the bus's data uncorrected, a core
/// that never counts a corrected word, a coder that never applies a flip, a coder writing each check byte in its word's
/// own lane.
#[test]
fn the_secded_receipt_fails_when_the_rtl_is_wrong() {
    if !cosim_tools() {
        return;
    }
    for (file, from, to, says) in [
        ("hw/rtl/logos_stream.v", "        q_data <= fixed;", "        q_data <= bus_rdata[DL-1:0];", "data_bit: the lines that differ"),
        (
            "hw/rtl/logos_bringup_core.v",
            "fixed[i] <= fixed[i] + ones(mr_ecc[i*16 +: 8]);",
            "fixed[i] <= fixed[i];",
            "data_bit: the words corrected and lost",
        ),
        ("hw/rtl/logos_stream.v", "(w_flip[20] ? ", "(1'b0 ? ", "data_bit: the words corrected and lost"),
        (
            "hw/rtl/logos_secded.v",
            "logos_secded_word_encode coder(.d(d[64*w +: 64]), .c(bus[512 + 8*(7-w) +: 8]));",
            "logos_secded_word_encode coder(.d(d[64*w +: 64]), .c(bus[512 + 8*w +: 8]));",
            "a channel corrected or lost words before a flip was injected",
        ),
    ] {
        let d = emit(&AfuConfig { line: LocalMemoryLine::Data512Secded, ..AfuConfig::d5005() });
        let path = d.path().join(file);
        let source = std::fs::read_to_string(&path).unwrap();
        assert_eq!(source.matches(from).count(), 1, "the mutation site {from:?}");
        std::fs::write(&path, source.replace(from, to)).unwrap();
        let (code, out) = cosim(d.path(), 256, 64);
        assert_eq!(code, Some(1), "{to:?}:\n{out}");
        assert!(out.contains(says), "{to:?}:\n{out}");
    }
}

/// The C header says whether the lines are SECDED-coded, how many words a line has, and the bus bit of each check bit:
/// word `w`'s check byte in ECC lane `secded_check_byte(w)`.
#[test]
fn the_c_header_names_the_secded_lanes() {
    for (line, secded) in [(LocalMemoryLine::Data512Secded, 1), (LocalMemoryLine::DataAndEcc576, 0), (LocalMemoryLine::Data512, 0)] {
        let d = emit(&AfuConfig { line, ..AfuConfig::d5005() });
        let h = std::fs::read_to_string(d.path().join("sw/logos_d5005.h")).unwrap();
        assert!(h.contains(&format!("#define LOGOS_D5005_SECDED {secded}\n")), "{line:?}:\n{h}");
        assert!(h.contains(&format!("#define LOGOS_D5005_SECDED_WORDS {SECDED_WORDS}u\n")), "{h}");
        assert!(h.contains("#define LOGOS_D5005_ENGINE 0\n"), "a bring-up build has no engine:\n{h}");
        if !common::require_tool("cc") {
            continue;
        }
        let c = d.path().join("bits.c");
        std::fs::write(
            &c,
            "#include <stdio.h>\n#include \"logos_d5005.h\"\nint main(void) { for (unsigned w = 0; w < LOGOS_D5005_SECDED_WORDS; ++w) \
             for (unsigned j = 0; j < 8; ++j) printf(\"%u\\n\", (unsigned)LOGOS_D5005_SECDED_CHECK_BIT(w, j)); return 0; }\n",
        )
        .unwrap();
        let exe = d.path().join("bits");
        let o = Command::new("cc").args(["-std=c99", "-Wall", "-Werror", "-I"]).arg(d.path().join("sw")).arg(&c).arg("-o").arg(&exe).output().expect("cc");
        assert!(o.status.success(), "{}", text(&o));
        let got: Vec<usize> = String::from_utf8(Command::new(&exe).output().unwrap().stdout).unwrap().lines().map(|l| l.parse().unwrap()).collect();
        let want: Vec<usize> = (0..SECDED_WORDS).flat_map(|w| (0..8).map(move |j| 512 + 8 * secded_check_byte(w) + j)).collect();
        assert_eq!(got, want);
    }
}

/// A SECDED receipt whose code held is recorded with each channel's code; one where any channel's code did not hold —
/// a data bit not corrected, two flips in one word not found, a word corrected before any flip — is refused.
#[test]
fn the_receipt_recorder_refuses_a_secded_code_that_did_not_hold() {
    if !common::require_tool("python3") {
        return;
    }
    let uuid = AfuConfig::d5005().uuid_text();
    let d = tempfile::tempdir().unwrap();
    let gbs = d.path().join("image.gbs");
    std::fs::write(&gbs, b"a SECDED partial bitstream").unwrap();
    let sha = {
        let o = Command::new("sha256sum").arg(&gbs).output().unwrap();
        String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap().to_string()
    };
    let images = d.path().join("afu_images.json");
    std::fs::write(&images, format!("{{\"images\": [{{\"sha256\": \"{sha}\", \"afu_uuid\": \"{uuid}\"}}]}}")).unwrap();
    let held = |b: usize, n: u64| {
        serde_json::json!({
            "channel": b, "lines": n,
            "before": {"corrected": 0, "uncorrectable": 0},
            "data_bit": {"mismatches": 0, "corrected": n, "uncorrectable": 0},
            "check_bit": {"mismatches": 0, "corrected": n, "uncorrectable": 0},
            "two_bits_one_word": {"mismatches": n, "corrected": 0, "uncorrectable": n},
            "two_words": {"mismatches": 0, "corrected": 2 * n, "uncorrectable": 0},
            "clean": {"mismatches": 0, "corrected": 0, "uncorrectable": 0},
        })
    };
    let receipt = |codes: Vec<serde_json::Value>| -> String {
        let mut r: serde_json::Value = serde_json::from_str(&receipt_fixture(0, 0, true, &uuid)).unwrap();
        r["secded"] = serde_json::Value::Array(codes);
        r.to_string()
    };
    let record = |receipt: &str, ledger: &Path| -> Output {
        let r = d.path().join("receipt.json");
        std::fs::write(&r, receipt).unwrap();
        Command::new("python3").arg(script("record_receipt.py")).arg(&r).arg(&gbs).arg(&images).arg(ledger).output().unwrap()
    };
    let ledger = d.path().join("receipts.json");
    let o = record(&receipt((0..4).map(|b| held(b, 64)).collect()), &ledger);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&ledger).unwrap()).unwrap();
    assert_eq!(l["receipts"][0]["secded"].as_array().unwrap().len(), 4);
    for (case, field, value) in [("data_bit", "corrected", 63), ("two_bits_one_word", "uncorrectable", 0), ("before", "corrected", 1), ("two_words", "mismatches", 1)] {
        let mut codes: Vec<serde_json::Value> = (0..4).map(|b| held(b, 64)).collect();
        codes[2][case][field] = serde_json::json!(value);
        let fresh = d.path().join("fresh.json");
        let o = record(&receipt(codes), &fresh);
        assert_eq!(o.status.code(), Some(1), "{case}.{field} = {value}: {}", text(&o));
        assert!(text(&o).contains(&format!("channel 2's SECDED code did not hold: {case}")), "{}", text(&o));
        assert!(!fresh.exists());
    }
}

/// A flip-flop bit of a flattened design as Quartus names a keeper: the instance it was declared in (`""` at the top),
/// the register's own name there — the bit's name in that instance that is no port of the instance's module — its bit,
/// and the port of the top its clock is (`port[i]` a bit of a vector port).
#[derive(Debug)]
struct Flop {
    path: String,
    reg: String,
    bit: usize,
    clock: String,
}

/// Every flip-flop bit of `top` in `verilog` ([`Flop`]), and each memory write port's instance path and clock.
fn flops_by_clock(verilog: &str, top: &str) -> (Vec<Flop>, Vec<(String, String)>) {
    const FLOPS: [&str; 11] = ["$dff", "$adff", "$dffe", "$adffe", "$sdff", "$sdffe", "$sdffce", "$aldff", "$aldffe", "$dffsr", "$dffsre"];
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("design.v"), verilog).unwrap();
    std::fs::write(
        dir.path().join("run.ys"),
        format!("read_verilog -sv design.v\nhierarchy -check -top {top}\nproc\nwrite_json pre.json\nflatten\nopt_clean\nwrite_json net.json\n"),
    )
    .unwrap();
    let o = Command::new("yosys").args(["-q", "-s", "run.ys"]).current_dir(dir.path()).output().expect("yosys");
    assert!(o.status.success(), "yosys on {top}:\n{}", text(&o));
    let read = |f: &str| -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(dir.path().join(f)).unwrap()).unwrap() };
    let (pre, net) = (read("pre.json"), read("net.json"));
    // The module an instance path names, walking cell names down from the top (a name may hold dots of its own).
    let module_of = |path: &str| -> String {
        let (mut module, mut rest) = (top.to_string(), path.to_string());
        while !rest.is_empty() {
            let cells = pre["modules"][&module]["cells"].as_object().cloned().unwrap_or_default();
            let Some((name, cell)) = cells.iter().filter(|(n, _)| rest == **n || rest.starts_with(&format!("{n}."))).max_by_key(|(n, _)| n.len()) else {
                return String::new();
            };
            module = cell["type"].as_str().unwrap_or("").to_string();
            rest = rest[name.len()..].trim_start_matches('.').to_string();
        }
        module
    };
    let ports_of = |module: &str| -> std::collections::BTreeSet<String> {
        pre["modules"][module]["ports"].as_object().map(|p| p.keys().cloned().collect()).unwrap_or_default()
    };
    let m = &net["modules"][top];
    let mut clock_of: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (p, v) in m["ports"].as_object().unwrap() {
        let bits = v["bits"].as_array().unwrap();
        for (i, b) in bits.iter().enumerate() {
            clock_of.insert(b.to_string(), if bits.len() == 1 { p.clone() } else { format!("{p}[{i}]") });
        }
    }
    let mut aliases: std::collections::BTreeMap<String, Vec<(String, usize)>> = std::collections::BTreeMap::new();
    for (n, v) in m["netnames"].as_object().unwrap() {
        for (i, b) in v["bits"].as_array().unwrap().iter().enumerate() {
            aliases.entry(b.to_string()).or_default().push((n.clone(), i));
        }
    }
    // A flattened cell's instance path: its name past `$flatten\` up to its own `.$`, every escaped name's `\` dropped as
    // the net names drop it.
    let own_path = |cell: &str| -> String {
        cell.strip_prefix("$flatten\\").and_then(|r| r.rfind(".$").map(|i| r[..i].replace(".\\", "."))).unwrap_or_default()
    };
    let (mut flops, mut memories) = (Vec::new(), Vec::new());
    for (cname, c) in m["cells"].as_object().unwrap() {
        let ty = c["type"].as_str().unwrap_or("");
        let conn = &c["connections"];
        let clock = clock_of.get(&conn["CLK"][0].to_string()).cloned().unwrap_or_else(|| "(not a port)".to_string());
        if ty.starts_with("$memwr") {
            let id = c["parameters"]["MEMID"].as_str().unwrap_or(cname).trim_start_matches('\\').to_string();
            memories.push((id.rsplit_once('.').map_or(String::new(), |(p, _)| p.to_string()), clock));
            continue;
        }
        if !FLOPS.contains(&ty) {
            continue;
        }
        let path = own_path(cname);
        let ports = ports_of(&module_of(&path));
        let prefix = if path.is_empty() { String::new() } else { format!("{path}.") };
        for (i, q) in conn["Q"].as_array().unwrap().iter().enumerate() {
            let own = aliases.get(&q.to_string()).into_iter().flatten().find(|(n, _)| {
                n.strip_prefix(&prefix).is_some_and(|local| !local.contains('.') && !local.starts_with('$') && !ports.contains(local))
            });
            let (reg, bit) = own.map_or((format!("{cname}"), i), |(n, b)| (n[prefix.len()..].to_string(), *b));
            flops.push(Flop { path: path.clone(), reg, bit, clock: clock.clone() });
        }
    }
    (flops, memories)
}

/// The shell's compute side is constrained as the compute clock steps it: its constraints name the core's registers,
/// the card port's and each dual-clock FIFO's compute side's (`ofs::compute_fifo_sides`, its pointer, the other side's
/// pointer crossing in and its binary, a read side's word and valid, and the RAM written or read there), and give every
/// path between two of them the compute clock's user clocks. Each of them is a user-clock register that steps only at
/// the step — none clocked by another clock, none of the engine, the divider or a fast port, which run every user
/// cycle — and every user-clock register of the shell that steps only at the step is among them, in a divided build,
/// the engine's, the hybrid's and the recurrent hybrid's.
#[test]
fn the_compute_sides_paths_are_given_the_compute_clocks_user_clocks_and_only_they() {
    if !common::require_tool("yosys") {
        return;
    }
    use logicaffeine_compile::spatial::decode_engine::ofs::{compute_fifo_sides, COMPUTE_CLOCK_VERILOG, FIFO_SIDE_REGISTERS};
    let builds = [
        ("divided by 16", AfuConfig { compute_divide: 16, user_clock_mhz: 100, ..AfuConfig::d5005() }),
        ("the engine", AfuConfig::engine(engine_08b(), LocalMemoryLine::Data512)),
        ("the hybrid engine", AfuConfig::engine(hybrid_08b(), LocalMemoryLine::Data512)),
        ("the recurrent hybrid", AfuConfig::engine(recurrent_08b(), LocalMemoryLine::Data512)),
    ];
    for (what, cfg) in builds {
        let d = emit(&cfg);
        let shell = std::fs::read_to_string(rtl(d.path(), "logos_d5005_shell.v")).unwrap();
        let sdc = std::fs::read_to_string(rtl(d.path(), "logos_d5005.sdc")).unwrap();
        let sides = compute_fifo_sides(&shell);
        assert!(sides.len() >= 8, "{what}: {sides:?}");
        let listed = sides.iter().map(|(n, s)| format!("{n} {s}")).collect::<Vec<_>>().join(" ");
        assert!(sdc.contains(&format!("foreach {{name side}} {{{listed}}} {{")), "{what}: the constraints name the sides {listed}:\n{sdc}");
        assert!(
            sdc.contains(&format!("-to $logos_step -setup -end {}\n", cfg.compute_divide)) && sdc.contains(&format!("-to $logos_step -hold -end {}\n", cfg.compute_divide - 1)),
            "{what}: the step's user clocks:\n{sdc}"
        );
        let mut base: String = shell_files(d.path())
            .iter()
            .filter(|f| !f.ends_with("logos_compute_clock.v"))
            .map(|f| std::fs::read_to_string(f).unwrap() + "\n")
            .collect();
        if cfg.engine.is_some() {
            base += &std::fs::read_to_string(rtl(d.path(), "logos_card_port.v")).unwrap();
            base += &noise_engine(&std::fs::read_to_string(rtl(d.path(), "logos_engine.v")).unwrap());
        }
        base = format!("{base}\n{COMPUTE_CLOCK_VERILOG}");
        // A path's last instance name: a FIFO's own (`desc_fifo`), its channel's prefix (`ch[0].channel.`) aside.
        let last = |path: &str| path.rsplit_once('.').map_or(path.to_string(), |(_, l)| l.to_string());
        let named = |f: &Flop| {
            f.path == "core"
                || f.path.starts_with("core.")
                || f.path == "port"
                || f.path.starts_with("port.")
                || sides.iter().any(|(inst, side)| {
                    let regs = FIFO_SIDE_REGISTERS.iter().find(|(s, _)| s == side).unwrap().1;
                    last(&f.path) == *inst && regs.contains(&f.reg.as_str())
                })
        };
        let every_cycle = |f: &Flop| {
            f.path == "engine" || f.path.starts_with("engine.") || f.path.starts_with("compute_clock") || {
                let l = last(&f.path);
                l.starts_with("fast_") && l.ends_with("_fifo")
            }
        };
        let (flops, memories) = flops_by_clock(&base, "logos_d5005_shell");
        let wrong: Vec<&Flop> = flops.iter().filter(|f| named(f) && (f.clock != "uclk" || every_cycle(f))).collect();
        assert!(wrong.is_empty(), "{what}: named but not the compute side's: {:?}", &wrong[..wrong.len().min(20)]);
        let stepping: Vec<&Flop> = flops.iter().filter(|f| f.clock == "uclk" && !every_cycle(f)).collect();
        assert!(stepping.len() > 500, "{what}: {} compute-side bits", stepping.len());
        let missed: Vec<&&Flop> = stepping.iter().filter(|f| !named(f)).collect();
        assert!(missed.is_empty(), "{what}: {} compute-side bits not named: {:?}", missed.len(), &missed[..missed.len().min(20)]);
        for (inst, side) in &sides {
            for (memory, clock) in memories.iter().filter(|(m, _)| last(m) == *inst) {
                if *side == 'w' {
                    assert_eq!(clock, "uclk", "{what}: {memory}, written by its compute side");
                } else {
                    assert_ne!(clock, "uclk", "{what}: {memory}, read by its compute side and written by the other");
                }
            }
        }
    }
}
