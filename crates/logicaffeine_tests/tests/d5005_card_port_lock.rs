//! The engine's memory port on the D5005's channels: `decode_engine::physical::card_port_verilog` under iverilog
//! between a scripted engine and four channels on four unrelated clocks, each the shell's own channel path in front of
//! a memory that waits at random.
//!
//! These tests hold the port to the physical map. A machine's memory after real steps is laid flat on the channels,
//! every byte where `PhysicalMap::flat` puts it, the weights with the rest. Random reads each return the machine's
//! line. Writes under every kind of mask — every byte, a scatter, a few bytes — leave each channel holding, line for
//! line, what the map lays the memory they make at, and a read after a write reads what it wrote. This holds at three
//! granules and on SECDED lines, where a line with one flipped bit reads corrected and a write heals it, and a line
//! with two flips in a word faults its read and drops a merged write. Under a map whose space starts past the weight
//! image, a line below the space faults: its read returns zeros, its write is dropped, and the image is untouched.
//!
//! A hybrid engine's fast side reaches the same space through its own port, `physical::fast_port_verilog`, on the
//! hybrid shell's fast channel ports (`ofs::hybrid_shell_verilog`), whose stream engines serve it and the card port one
//! side at a time. Beside the card port's ops on half the granules, the fast port's reads and writes of whole lines on
//! the other half — runs across granules and channels, longer than a descriptor and than a channel's write FIFO, lines
//! alone, lines below the space — return every read in the order it was asked, a read after a fence what the writes
//! before it wrote, a line below the space zeros with a fault, and leave every channel holding what the map lays out
//! the memory both ports' writes make at, the weight image untouched.
//!
//! Several fast ports share the channels' fast side at once through `physical::channel_mux_verilog`, as a recurrence
//! unit's head lanes do: beside the card port, two or five fast ports each on its own share of the granules — every
//! share on every channel — return every read of each in the order that port asked it, and leave every channel holding
//! what every port's writes make, each port's lines written whole however its runs and another port's interleave on a
//! channel.
//!
//! A reader that takes a channel's lines only as its other channels let it — four image streams taken in lockstep, as
//! the streaming unit's feeds are — deadlocks a fast port on the mux, each waiting on the other's run in another
//! channel. Through `physical::read_credit_verilog` a channel, which asks only for the lines it can hold, the two share
//! the mux on every run, every line where it lies, and a stream of long tiles keeps the rate it keeps alone.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/card_memory.rs"]
mod card_memory;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
use logicaffeine_compile::spatial::decode_engine::machine::Machine;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::physical::PhysicalMap;
use card_memory::{next, FastOps, Run};
use tiny_models::Tiny;

const CTX: usize = 16;

/// The tiny qwen35 with its attention grouped as the 27B's is, after two steps of two sequences.
/// `logos_pipe`, the register a fast port's stream crosses at the engine's edge for a floorplan to stretch, between a
/// source and a sink each busy at random from a clock in ten to every clock: every word out in order, none lost, none
/// twice; then, both ready every clock, a word out every clock, each the clock after it went in, the input ready
/// throughout.
#[test]
fn the_pipe_passes_every_word_in_order_a_word_a_clock() {
    use logicaffeine_compile::spatial::decode_engine::physical::pipe_verilog;
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pipe.v"), pipe_verilog()).unwrap();
    let tb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_flow/tb_pipe.v");
    for (seed, ip, op) in [(1, 70, 60), (2, 100, 30), (3, 30, 100), (4, 100, 100), (5, 50, 50), (6, 95, 95), (7, 10, 90)] {
        let sim = dir.path().join(format!("sim{seed}"));
        let build = std::process::Command::new("iverilog")
            .args(["-g2012", "-o"])
            .arg(&sim)
            .args([format!("-Ptb.SEED={seed}"), format!("-Ptb.IP={ip}"), format!("-Ptb.OP={op}")])
            .arg(&tb)
            .arg(dir.path().join("pipe.v"))
            .output()
            .expect("iverilog");
        assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
        let run = std::process::Command::new("vvp").arg(&sim).output().expect("vvp");
        let out = String::from_utf8_lossy(&run.stdout);
        assert!(out.contains("RESULT sent=4000 got=4000 errors=0 run=1000"), "seed {seed}, source {ip}%, sink {op}%:\n{out}");
    }
}

fn machine(dir: &Path) -> (Machine, PathBuf) {
    let mut spec: ModelSpec = Tiny::Qwen35.wide(CTX);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    let path = dir.join("qwen35.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let mut m = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 2, CTX).unwrap();
    let seqs: Vec<usize> = (0..2).map(|_| m.sequence().unwrap()).collect();
    m.step(&seqs.iter().map(|&s| (s, vec![5 + s as u32, 23, 7])).collect::<Vec<_>>()).unwrap();
    m.step(&seqs.iter().map(|&s| (s, vec![11 + s as u32])).collect::<Vec<_>>()).unwrap();
    (m, path)
}

/// The machine's memory flat on the channels at `granule` lines a granule: random reads and writes through the port
/// leave every channel line as the map lays out the memory they make.
fn flat(granule: u64, seed: u64, gap: u64) {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (m, _) = machine(dir.path());
    let map = PhysicalMap::flat(&m.layout, &ImageLayout::d5005(Beat::Data64), granule).unwrap();
    assert_eq!((map.tensors_end, map.base), (0, 0), "a flat map starts at the channels' first line");
    let mut run = Run::new(map, false, m.memory(), &vec![Vec::new(); 4]);
    run.script(1200, seed);
    assert!(!run.fault);
    run.check(gap, seed, &format!("granule {granule}"));
}

#[test]
fn the_port_reads_and_writes_the_flat_memory_where_the_map_puts_it_at_eight_lines_a_granule() {
    flat(8, 0x0ca7_d001, 30);
}

#[test]
fn the_port_reads_and_writes_the_flat_memory_where_the_map_puts_it_at_sixty_four_lines_a_granule() {
    flat(64, 0x0ca7_d002, 1);
}

#[test]
fn the_port_reads_and_writes_the_flat_memory_where_the_map_puts_it_at_the_engine_plans_granule() {
    flat(512, 0x0ca7_d003, 7);
}

/// The machine's memory past the weight image at `granule` lines a granule: the card port's ops on half the granules
/// and the fast port's on the other half, side by side — four granules in every eight each, a turn of the channels,
/// where the space holds eight granules, every other granule where it holds fewer.
fn beside(granule: u64, seed: u64) {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (m, path) = machine(dir.path());
    let map = PhysicalMap::of(&m.layout, &ImageLayout::d5005(Beat::Data64), granule).unwrap();
    let weights = map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
    assert!(map.tensors_end > 0 && map.base > 0, "the space starts past the weight image");
    let granules = map.lines.div_ceil(granule);
    assert!(granules >= 2, "granule {granule}: {granules} granules");
    let turn = if granules >= 8 { 4 } else { 1 };
    let fast_granule = move |g: u64| g / turn % 2 == 1;
    let mut run = Run::new(map, false, m.memory(), &weights);
    run.script_where(300, seed, &|g| !fast_granule(g));
    let fast = run.fast_script(1500, seed ^ 0xfa57, &fast_granule);
    assert!(fast.fault, "the fast port's ops reach below the space");
    assert!(!run.fault);
    run.check_beside(&fast, 7, 4, 32, seed, &format!("granule {granule}"));
}

#[test]
fn the_fast_port_reads_and_writes_beside_the_card_port_where_the_map_puts_them_at_eight_lines_a_granule() {
    beside(8, 0xfa57_0001);
}

#[test]
fn the_fast_port_reads_and_writes_beside_the_card_port_where_the_map_puts_them_at_sixty_four_lines_a_granule() {
    beside(64, 0xfa57_0002);
}

#[test]
fn the_fast_port_reads_and_writes_beside_the_card_port_where_the_map_puts_them_at_the_engine_plans_granule() {
    beside(512, 0xfa57_0003);
}

/// The machine's memory past the weight image at `granule` lines a granule, shared by the card port and `ports` fast
/// ports through the channel mux: granule g the card port's when g mod (ports + 1) is ports, else port g mod (ports + 1)'s,
/// so each share lies on every channel and every channel serves every port at once.
fn muxed(granule: u64, ports: usize, seed: u64) {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (m, path) = machine(dir.path());
    let map = PhysicalMap::of(&m.layout, &ImageLayout::d5005(Beat::Data64), granule).unwrap();
    let weights = map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
    let share = ports as u64 + 1;
    let granules = map.lines.div_ceil(granule);
    assert!(granules >= 4 * share, "granule {granule}: {granules} granules, too few for every share on every channel");
    let mut run = Run::new(map, false, m.memory(), &weights);
    run.script_where(200, seed, &|g| g % share == ports as u64);
    let fasts: Vec<FastOps> = (0..ports as u64).map(|p| run.fast_script(600, seed ^ (0xfa57 + p), &move |g| g % share == p)).collect();
    assert!(fasts.iter().all(|f| f.fault), "every fast port's ops reach below the space");
    run.check_muxed(&fasts, 7, 4, 32, seed, &format!("granule {granule}, {ports} fast ports"));
}

#[test]
fn two_fast_ports_share_the_channels_through_the_mux_beside_the_card_port_at_eight_lines_a_granule() {
    muxed(8, 2, 0x3a7e_0001);
}

#[test]
fn five_fast_ports_share_the_channels_through_the_mux_beside_the_card_port_at_eight_lines_a_granule() {
    muxed(8, 5, 0x3a7e_0002);
}

#[test]
fn five_fast_ports_share_the_channels_through_the_mux_beside_the_card_port_at_sixty_four_lines_a_granule() {
    muxed(64, 5, 0x3a7e_0003);
}

/// One run of `tb_mux_lockstep.v` (a reader of four image streams taken in lockstep beside a fast port on the channel
/// mux) at its parameters `params`: the counts its RESULT line names, by name.
fn lockstep(dir: &Path, params: &[(&str, u64)]) -> std::collections::HashMap<String, u64> {
    use logicaffeine_compile::spatial::decode_engine::physical::{channel_mux_verilog, fast_port_verilog, read_credit_verilog};
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::verilog as stream_verilog;
    let sources = dir.join("sources.v");
    if !sources.exists() {
        let text = [fast_port_verilog(), channel_mux_verilog(), read_credit_verilog(), stream_verilog(&ImageLayout::d5005(Beat::Data64))];
        std::fs::write(&sources, text.join("\n")).unwrap();
    }
    let tb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_flow/tb_mux_lockstep.v");
    let name: String = params.iter().map(|(k, v)| format!("{k}{v}")).collect();
    let sim = dir.join(format!("sim_{name}"));
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .args(params.iter().map(|(k, v)| format!("-Ptb.{k}={v}")))
        .arg(&tb)
        .arg(&sources)
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&sim).output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout);
    let line = out.lines().find(|l| l.starts_with("RESULT ")).unwrap_or_else(|| panic!("{params:?}: no result:\n{out}"));
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut counts = std::collections::HashMap::new();
    for (k, w) in words.iter().enumerate() {
        if let Some((key, value)) = w.split_once('=') {
            counts.insert(key.to_string(), value.parse().unwrap());
            if words.get(k + 1) == Some(&"of") {
                counts.insert(format!("{key}_all"), words[k + 2].parse().unwrap());
            }
        }
    }
    counts
}

/// A reader that takes a channel's lines only as its other channels let it — four image streams whose beats are taken
/// all four in one clock, as the streaming unit's feeds deal to an array that steps when every core has its beat —
/// asking the channel mux directly beside a fast port, which takes its reads' lines in the order it asked them, stops
/// for good on some runs: a channel's next line is the fast port's, which waits on its earlier read of another channel
/// behind the reader's run there, which waits on this channel. Every run that finishes returns every line as it lies.
#[test]
fn a_reader_whose_channels_step_together_and_a_fast_port_deadlock_on_the_mux_without_a_credit() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut hung = 0;
    for seed in 1..=8 {
        let r = lockstep(dir.path(), &[("SEED", seed), ("CREDIT", 0)]);
        assert_eq!(r["errors"], 0, "seed {seed}: {r:?}");
        if r["hung"] == 1 {
            hung += 1;
        } else {
            assert_eq!((r["beats"], r["reads"]), (r["beats_all"], r["reads_all"]), "seed {seed} finished short: {r:?}");
        }
    }
    assert!(hung > 0, "no run of eight deadlocked: the harness no longer sees the reader wait on another channel");
}

/// Through a `logos_read_credit` a channel the same reader shares the mux with the fast port on every run: every beat
/// and every read back, each the line it names; every run the mux takes for the reader 1 to 32 lines; the lines asked
/// of a channel and not yet taken by its stream never past the credit's 512, at every rate of the channels and the
/// reader from a line in ten clocks to a line every clock.
#[test]
fn through_a_credit_a_reader_whose_channels_step_together_shares_the_mux_with_a_fast_port() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for seed in 1..=12 {
        let gap = [30, 0, 60, 90][seed as usize % 4];
        let r = lockstep(dir.path(), &[("SEED", seed), ("CREDIT", 1), ("GAP", gap)]);
        assert_eq!((r["errors"], r["hung"], r["over"]), (0, 0, 0), "seed {seed}, gap {gap}%: {r:?}");
        assert_eq!((r["beats"], r["reads"]), (r["beats_all"], r["reads_all"]), "seed {seed}, gap {gap}%: {r:?}");
        assert_eq!(r["longest"], 32, "seed {seed}, gap {gap}%: the longest run the reader asked");
    }
}

/// On tiles as long as the 27B's (300 to 1,900 lines) a stream through its credit keeps the rate the stream keeps
/// alone: with every channel answering a line a clock a latency after it takes a descriptor, the beats come a line a
/// clock but for one clock at each tile's start, the stream's own, at every latency up to 400 clocks.
#[test]
fn through_a_credit_a_stream_of_long_tiles_keeps_a_line_a_clock_as_it_does_alone() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tiles = 12;
    for lat in [60, 200, 400] {
        let shape = [("RATE", 1), ("READS", 0), ("TILES", tiles), ("LEAST", 300), ("MOST", 1900), ("LAT", lat), ("SEED", 11)];
        let run = |credit| lockstep(dir.path(), &[shape.as_slice(), &[("CREDIT", credit)]].concat());
        let (alone, credited) = (run(0), run(1));
        for r in [&alone, &credited] {
            assert_eq!((r["errors"], r["hung"], r["beats"]), (0, 0, r["beats_all"]), "latency {lat}: {r:?}");
        }
        assert_eq!(alone["clocks"], alone["lines"] + tiles - 1, "latency {lat}: the stream alone, {alone:?}");
        assert_eq!(credited["clocks"], alone["clocks"], "latency {lat}: through the credit {credited:?}, alone {alone:?}");
    }
}

/// One run of `tb_mux_ports.v` (fast ports reading and writing beside a credited reader whose channels step together,
/// on the channel mux) at its parameters `params`: the counts its RESULT line names, by name.
fn port_mix(dir: &Path, params: &[(&str, u64)]) -> std::collections::HashMap<String, u64> {
    use logicaffeine_compile::spatial::decode_engine::physical::{channel_mux_verilog, fast_port_verilog, read_credit_verilog};
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::verilog as stream_verilog;
    let sources = dir.join("sources.v");
    if !sources.exists() {
        let text = [fast_port_verilog(), channel_mux_verilog(), read_credit_verilog(), stream_verilog(&ImageLayout::d5005(Beat::Data64))];
        std::fs::write(&sources, text.join("\n")).unwrap();
    }
    let tb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_flow/tb_mux_ports.v");
    let name: String = params.iter().map(|(k, v)| format!("{k}{v}")).collect();
    let sim = dir.join(format!("sim_{name}"));
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .args(params.iter().map(|(k, v)| format!("-Ptb.{k}={v}")))
        .arg(&tb)
        .arg(&sources)
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&sim).output().expect("vvp");
    let out = String::from_utf8_lossy(&run.stdout);
    let line = out.lines().find(|l| l.starts_with("RESULT ")).unwrap_or_else(|| panic!("{params:?}: no result:\n{out}"));
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut counts = std::collections::HashMap::new();
    for (k, w) in words.iter().enumerate() {
        if let Some((key, value)) = w.split_once('=') {
            counts.insert(key.to_string(), value.parse().unwrap());
            if words.get(k + 1) == Some(&"of") {
                counts.insert(format!("{key}_all"), words[k + 2].parse().unwrap());
            }
        }
    }
    counts
}

/// The card's port mix shares the channels and never stops: one to four fast ports (the row class's shared port and its
/// head lanes' ports) reading runs of consecutive lines and writing runs, beside a reader through a credit a channel
/// whose channels step together (the matvec class of another context), each channel serving its descriptors in order,
/// its write lines through a write FIFO of the shell's 64 lines, at random rates: every beat the reader is owed comes
/// back as it lies, every read returns in the order asked as its line lies, every write line reaches its channel at the
/// place its line lies carrying its port and line, and every write run's done token is taken — at runs of 32 to 512
/// lines, a tenth to three fifths of them writes, channels pausing up to nine clocks in ten and answering up to 400 clocks
/// after a descriptor.
#[test]
fn the_cards_port_mix_shares_the_channels_through_the_mux_and_never_stops() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for seed in 1..=12u64 {
        let (ports, gap, writes, run, lat) = (1 + seed % 4, (seed % 4) * 30, 10 + (seed % 6) * 10, 32 << (seed % 5), 60 + (seed % 3) * 170);
        let params = [("SEED", seed), ("PORTS", ports), ("GAP", gap), ("WRITES", writes), ("RUN", run), ("LAT", lat), ("OPS", 150)];
        let r = port_mix(dir.path(), &params);
        assert_eq!((r["errors"], r["hung"]), (0, 0), "{params:?}: {r:?}");
        assert_eq!((r["beats"], r["writes"]), (r["beats_all"], r["writes_all"]), "{params:?}: every beat and every write through: {r:?}");
        assert!(r["reads"] > 0 && r["writes"] > 0 && r["runs"] > 0, "{params:?}: the ports read and wrote: {r:?}");
    }
}

/// A hybrid engine's channel reports every write run it takes as done, from either side, at every user clock the card
/// runs against its memory's 300 MHz: the shell's channel path (`ofs::hybrid_shell_verilog`) in front of a memory that
/// waits at random, the fast side giving runs of one or two lines back to back, a line a clock, the compute side a
/// one-line run each of its steps, one user clock in eight. Short runs finish about as fast as the user clock takes
/// their done tokens, so a token the channel let fall when its side's done FIFO looked full left that side's port
/// waiting on a write the memory had made: a unit waiting for its last write never finished, at 50 to 86 MHz on the
/// card and never at the simulations' 200.
#[test]
fn every_write_run_a_hybrid_channel_takes_is_reported_done_at_every_user_clock_the_card_runs() {
    use logicaffeine_compile::spatial::decode_engine::ofs::{afu_sources, hybrid_shell_verilog, AfuConfig};
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut files = vec![card_memory::fixture("tb_channel_done.v")];
    for f in afu_sources(&AfuConfig::d5005()) {
        if ["hw/rtl/logos_dcfifo.v", "hw/rtl/logos_stream.v", "hw/rtl/logos_secded.v", "sim/logos_avmm_memory_model.v"].contains(&f.path) {
            let p = dir.path().join(f.path.rsplit('/').next().unwrap());
            std::fs::write(&p, f.text).unwrap();
            files.push(p);
        }
    }
    let shell = dir.path().join("logos_d5005_shell.v");
    std::fs::write(&shell, hybrid_shell_verilog(true)).unwrap();
    files.push(shell);
    let mut failed = Vec::new();
    for mhz in [30u64, 50, 55, 60, 70, 86, 100, 200] {
        for (lines, wait) in [(1u64, 0u64), (1, 30), (2, 30)] {
            let uclk_ps = (1_000_000 / mhz) / 2 * 2;
            let sim = dir.path().join(format!("sim_{mhz}_{lines}_{wait}"));
            let build = std::process::Command::new("iverilog")
                .args(["-g2012", "-s", "tb", "-o"])
                .arg(&sim)
                .args([format!("-Ptb.UCLK_PS={uclk_ps}"), format!("-Ptb.LINES={lines}"), format!("-Ptb.WAIT_PCT={wait}"), format!("-Ptb.SEED={mhz}")])
                .args(&files)
                .output()
                .expect("iverilog");
            assert!(build.status.success(), "iverilog: {}", card_memory::text(&build));
            let run = std::process::Command::new("vvp").arg(&sim).output().expect("vvp");
            let out = card_memory::text(&run);
            let line = out.lines().find(|l| l.starts_with("RESULT ")).unwrap_or_else(|| panic!("{mhz} MHz: no result:\n{out}"));
            if line != "RESULT fast_runs=400 fast_done=400 core_runs=64 core_done=64" {
                failed.push(format!("{mhz} MHz, runs of {lines}, memory waiting {wait}%: {line}"));
            }
        }
    }
    assert!(failed.is_empty(), "write runs the channel took and never reported done:\n{}", failed.join("\n"));
}

/// On SECDED lines the flat memory is coded on the bus; lines with one flipped bit are read corrected and healed by
/// the writes that reach them, and lines with two flips in a word fault their reads, which return the line as read,
/// and drop the merged writes, while a write of every byte codes them afresh.
#[test]
fn on_secded_lines_the_port_reads_corrected_and_faults_on_a_word_it_cannot_correct() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (m, _) = machine(dir.path());
    let map = PhysicalMap::flat(&m.layout, &ImageLayout::d5005(Beat::Data64), 8).unwrap();
    let mut run = Run::new(map, true, m.memory(), &vec![Vec::new(); 4]);
    let mut s = 0x5ec_dedu64;
    let lines = run.map.lines as u32;
    let mut singles = Vec::new();
    let mut doubles = Vec::new();
    for k in 0..48 {
        let x = (next(&mut s) % lines as u64) as u32;
        let (c, at) = run.place(x).unwrap();
        let bit = (next(&mut s) % 576) as usize;
        run.channels[c][at][bit / 8] ^= 1 << (bit % 8);
        if k % 4 == 3 {
            let word = (next(&mut s) % 8) as usize;
            let (a, b) = (64 * word + (next(&mut s) % 32) as usize, 64 * word + 32 + (next(&mut s) % 32) as usize);
            run.channels[c][at][bit / 8] ^= 1 << (bit % 8);
            run.channels[c][at][a / 8] ^= 1 << (a % 8);
            run.channels[c][at][b / 8] ^= 1 << (b % 8);
            doubles.push(x);
        } else {
            singles.push(x);
        }
    }
    for (i, &x) in singles.iter().enumerate() {
        run.read(x);
        match i % 3 {
            0 => run.write(x, 0x00ff_00ff_00ff_00ff, [0xa5; 64]),
            1 => run.write(x, u64::MAX, [0x3c; 64]),
            _ => {}
        }
        run.read(x);
    }
    for (i, &x) in doubles.iter().enumerate() {
        run.read(x);
        if i % 2 == 0 {
            run.write(x, 0x0f, [0x77; 64]);
        } else {
            run.write(x, u64::MAX, [0x99; 64]);
            run.read(x);
        }
    }
    run.script(400, 0x5ec0_0002);
    assert!(run.fault, "a double flip faults");
    run.check(5, 21, "secded");
}

/// A map whose space starts past the weight image: lines below the space fault, their reads returning zeros and their
/// writes dropped, the image on the channels untouched, and every line of the space read and written where the map
/// puts it.
#[test]
fn a_line_below_the_space_faults_and_leaves_the_weight_image_as_it_is() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (m, path) = machine(dir.path());
    let map = PhysicalMap::of(&m.layout, &ImageLayout::d5005(Beat::Data64), 8).unwrap();
    let weights = map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
    assert!(map.tensors_end > 0 && map.base > 0);
    let mut run = Run::new(map, false, m.memory(), &weights);
    let space = run.space();
    run.read(0);
    run.write(space - 1, u64::MAX, [0xee; 64]);
    run.script(300, 0xbe10_0001);
    run.read(space / 2);
    run.write(1, 0xff, [0x11; 64]);
    assert!(run.fault);
    run.check(3, 33, "below the space");
}
