//! The decode engine with its memory on the D5005's channels. Under Verilator, the reference engine (`rtl::engine`)
//! runs the tiny model's whole step after the host's embedding with its memory port carried by
//! `physical::card_port_verilog` onto four channels of the shell's own path laid flat by the physical map, on plain
//! and on SECDED lines, leaving the store and every line the machine's. The engine build of the accelerator
//! (`ofs::AfuConfig::engine`) under Verilator, driven by `sw/logos_step.c` from what `ofs::write_engine_program`
//! wrote, decodes greedily exactly the machine's tokens — on SECDED lines too, with a bit flipped in every line of the
//! memory image, every word the engine reads corrected and counted; a program that faults the engine or the card
//! port has its step reported and stops; and the step program compiles against OPAE for the card.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/card_memory.rs"]
mod card_memory;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
use logicaffeine_compile::spatial::decode_engine::machine::{Machine, LINE};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::ofs::{afu_sources, AfuConfig, LocalMemoryLine, ENGINE_USER_CLOCK_MHZ};
use logicaffeine_compile::spatial::decode_engine::physical::{card_port_verilog, PhysicalMap};
use card_memory::{fixture, hex, text, Run};
use tiny_models::Tiny;

/// The words of a `$writememh` or `%h` dump, `bytes_a_word` bytes each.
fn read_words(path: &Path, bytes_a_word: usize) -> Vec<Vec<u8>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('@'))
        .map(|l| {
            let l = l.replace('_', "");
            assert_eq!(l.len(), 2 * bytes_a_word, "{}: a word of {} hex digits", path.display(), l.len());
            (0..bytes_a_word).rev().map(|i| u8::from_str_radix(&l[2 * i..2 * i + 2], 16).expect("a hex byte")).collect()
        })
        .collect()
}

/// The decode engine's whole step after the host's embedding, its memory on the card: the tiny model's step of a token
/// of one sequence beside three of another, through the card port onto four channels of the shell's own path laid
/// flat by the physical map, on plain lines and on SECDED lines, the engine simulated as its twin (`simulation_twin`) as
/// every cosimulation of the card runs it. The engine faults nowhere, the card port nowhere, and the store and every
/// logical line of the channels afterwards are the machine's.
#[test]
fn the_engine_runs_a_step_with_its_memory_on_the_card() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Job, Row, Store, StoreLayout, LANES};
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{engine_verilog, simulation_twin, tables, EngineParams};
    if !common::require_tool("verilator") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    let path = dir.path().join("tiny.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let mut m = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 2, spec.ctx_len).unwrap();
    let (a, b) = (m.sequence().unwrap(), m.sequence().unwrap());
    m.step(&[(a, vec![3, 17, 41, 8, 29])]).unwrap();
    let jobs = m.jobs.clone();
    let layout = m.layout.clone();
    assert!(matches!(jobs[0], Job::Embed { .. }), "the step opens with the host's embedding");
    let mut rows: Vec<Row> = Vec::new();
    let mut snapshots: Vec<(Store, Vec<u8>)> = Vec::new();
    m.step_traced(&[(a, vec![60]), (b, vec![5, 9, 33])], &mut |t| {
        rows = t.rows.to_vec();
        snapshots.push((t.store.clone(), t.memory.to_vec()));
    })
    .unwrap();
    let store = StoreLayout::of(&jobs, &layout, rows.len());
    let params = EngineParams::of(&layout, &store, &jobs);
    let t = tables(&layout, &store, &jobs[1..], &rows);
    let (store_before, memory_before) = &snapshots[1];
    let (store_after, memory_after) = snapshots.last().unwrap();
    let engine = simulation_twin(&engine_verilog(&params));
    for secded in [false, true] {
        let what = if secded { "SECDED lines" } else { "plain lines" };
        let at = dir.path().join(if secded { "secded" } else { "plain" });
        std::fs::create_dir_all(&at).unwrap();
        let map = PhysicalMap::flat(&layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap();
        let before = Run::new(map.clone(), secded, memory_before, &vec![Vec::new(); 4]);
        let dump = before.channels.iter().map(Vec::len).max().unwrap();
        for (c, lines) in before.channels.iter().enumerate() {
            let body: String = (0..dump).map(|k| format!("{}\n", hex(lines.get(k).map_or(&[0u8; 72][..before.bus_bytes()], |l| &l[..])))).collect();
            std::fs::write(at.join(format!("channel{c}.hex")), body).unwrap();
        }
        let w = |name: &str, s: String| std::fs::write(at.join(name), s).unwrap();
        w("jobs.hex", t.jobs.iter().map(|x| format!("{x:016x}\n")).collect());
        w("rows.hex", t.rows.iter().map(|x| format!("{x:032x}\n")).collect());
        w("slots.hex", t.slots.iter().map(|x| format!("{x:016x}\n")).collect());
        w("tens.hex", t.tensors.iter().map(|x| format!("{x:032x}\n")).collect());
        w("ropes.hex", t.ropes.iter().map(|x| format!("{x:016x}\n")).collect());
        w("blocks.hex", t.blocks.iter().map(|f| f.iter().rev().map(|x| format!("{x:08x}")).collect::<String>() + "\n").collect());
        w("store.hex", store.image(store_before).iter().map(|beat| beat.iter().rev().map(|x| format!("{x:08x}")).collect::<String>() + "\n").collect());
        let line = if secded { LocalMemoryLine::Data512Secded } else { LocalMemoryLine::Data512 };
        let mut files = vec![fixture("tb_engine_card.v")];
        for f in afu_sources(&AfuConfig { line, ..AfuConfig::d5005() }) {
            if f.path.starts_with("hw/rtl/") && f.path.ends_with(".v") || f.path == "sim/logos_avmm_memory_model.v" {
                let p = at.join(f.path.rsplit('/').next().unwrap());
                std::fs::write(&p, f.text).unwrap();
                files.push(p);
            }
        }
        let clock = logicaffeine_compile::spatial::decode_engine::ofs::COMPUTE_CLOCK_VERILOG.to_string();
        for (name, text) in [("logos_card_port.v", card_port_verilog()), ("logos_engine.v", engine.clone()), ("logos_compute_clock.v", clock)] {
            let p = at.join(name);
            std::fs::write(&p, text).unwrap();
            files.push(p);
        }
        let lines_log2 = (64 - (dump as u64 - 1).leading_zeros()).max(1);
        let g = |k: &str, v: u64| format!("-G{k}={v}");
        let built = Command::new(common::verilator())
            .args(["--binary", "--timing", "-Wno-fatal", "-Wno-lint", "-Wno-style", "-Wno-UNOPTFLAT", "--unroll-count", "4096", "--top-module", "tb", "-j", "2"])
            .args(["-CFLAGS", "-O3", "-Mdir", "obj"])
            .args([
                g("ROWS", rows.len() as u64),
                g("JOBS", t.jobs.len() as u64),
                g("SLOTS", t.slots.len() as u64),
                g("TENSORS", t.tensors.len().max(1) as u64),
                g("ROPES", t.ropes.len().max(1) as u64),
                g("LAYERS", t.blocks.len().max(1) as u64),
                g("STORE", store.beats as u64),
                g("LINES_LOG2", lines_log2 as u64),
                g("DUMP", dump as u64),
                g("SPACE", map.tensors_end / LINE),
                g("BASE", map.base),
                g("GRANULE_LOG2", map.granule.trailing_zeros() as u64),
                g("F16", t.state_f16 as u64),
                g("INT8", t.cache_int8 as u64),
                g("TWO", t.two_words as u64),
                g("PERBLOCK", t.per_block as u64),
                g("DWM", line.bus_bits() as u64),
                g("ECC", secded as u64),
            ])
            .args(&files)
            .current_dir(&at)
            .output()
            .expect("verilator");
        assert!(built.status.success(), "{what}: verilator: {}", String::from_utf8_lossy(&built.stderr));
        let run = Command::new(at.join("obj/Vtb")).current_dir(&at).output().expect("the Verilated engine");
        let out = text(&run);
        assert!(run.status.success() && out.contains("CYCLES"), "{what}:\n{out}");
        assert!(!out.contains("FAULT") && !out.contains("TIMEOUT"), "{what}:\n{out}");
        let beats = read_words(&at.join("store.out"), 64);
        for (slot, vecs) in store_after {
            for (r, v) in vecs.iter().enumerate() {
                let first = store.beat(*slot, r) as usize;
                for (i, x) in v.iter().enumerate() {
                    let lane = 4 * (i % LANES);
                    let got = u32::from_le_bytes(beats[first + i / LANES][lane..lane + 4].try_into().unwrap());
                    assert_eq!(got, x.to_bits(), "{what}: {slot:?} row {r} element {i}");
                }
            }
        }
        let after = Run::new(map.clone(), secded, memory_after, &vec![Vec::new(); 4]);
        for c in 0..4 {
            let got = read_words(&at.join(format!("out{c}.hex")), after.bus_bytes());
            for (k, l) in after.channels[c].iter().enumerate() {
                assert_eq!(&got[k], l, "{what}: channel {c} line {k}");
            }
        }
    }
}

/// The tiny model grouped as the 27B's attention is, its checkpoint under `dir`: a fresh machine of one sequence, and
/// the tokens the machine decodes greedily from `prompt`, `generate` past it.
fn decoding(dir: &Path, prompt: &[u32], generate: usize) -> (Machine, Vec<u32>) {
    decoding_at(dir, prompt, generate, EngineNumerics::fastest())
}

/// [`decoding`] at knob point `numerics`.
fn decoding_at(dir: &Path, prompt: &[u32], generate: usize, numerics: EngineNumerics) -> (Machine, Vec<u32>) {
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    decoding_of(dir, &spec, prompt, generate, numerics)
}

/// The tiny Qwen3.5's layers at Qwen3.5-0.8B's widths: 1,024 wide, its delta-net layers sixteen key and sixteen value
/// heads of 128, its attention eight heads of 256 over two key-value heads (64 of each rotating), its feed-forward
/// 3,584 wide.
fn small_model_shaped() -> ModelSpec {
    use logicaffeine_compile::model::spec::Ffn;
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    spec.d = 1024;
    for layer in &mut spec.layers {
        match &mut layer.mixer {
            Mixer::Attention(a) => {
                (a.heads, a.kv_heads, a.head_dim, a.value_dim) = (8, 2, 256, 256);
                if let Some(r) = &mut a.rope {
                    r.dims = 64;
                }
                a.scale = 1.0 / 16.0;
            }
            Mixer::GatedDelta(g) => (g.k_heads, g.v_heads, g.head_dim) = (16, 16, 128),
            Mixer::Mla(_) => {}
        }
        if let Ffn::Dense { dff, .. } = &mut layer.ffn {
            *dff = 3584;
        }
    }
    spec
}

/// The machine of `spec`'s checkpoint, written under `dir` as `tiny.gguf`, fresh, and the tokens it decodes from
/// `prompt` and `generate` past it at `numerics`.
fn decoding_of(dir: &Path, spec: &ModelSpec, prompt: &[u32], generate: usize, numerics: EngineNumerics) -> (Machine, Vec<u32>) {
    use logicaffeine_compile::spatial::decode_engine::model::argmax;
    let spec = spec.clone();
    let path = dir.join("tiny.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let fresh = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), numerics, 1, spec.ctx_len).unwrap();
    let mut decoder = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), numerics, 1, spec.ctx_len).unwrap();
    let s = decoder.sequence().unwrap();
    let mut want = Vec::new();
    let mut token = prompt[0];
    for pos in 0..prompt.len() + generate {
        want.push(token);
        let logits = decoder.step(&[(s, vec![token])]).unwrap().remove(0).remove(0);
        token = if pos + 1 < prompt.len() { prompt[pos + 1] } else { argmax(&logits) };
    }
    (fresh, want)
}

/// The engine build of the accelerator for `machine` at `line` as the card's is built (`AfuConfig::engine`), its compute
/// clock the user clock divided by eight rather than the card's sixty-four: the engine steps in the user clock at the
/// compute clock's edges, so the divide changes no value it computes (`d5005_engine_rtl_lock` runs it at six, eight and
/// ten, and its crossing at sixty-four), and a co-simulated step costs an eighth of the user cycles. Its tree under a
/// fresh directory.
fn engine_build(machine: &Machine, line: LocalMemoryLine) -> tempfile::TempDir {
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::EngineParams;
    let store = StoreLayout::of(&machine.jobs, &machine.layout, 1);
    let engine = EngineParams::of(&machine.layout, &store, &machine.jobs);
    let afu = tempfile::tempdir().unwrap();
    for f in afu_sources(&AfuConfig { compute_divide: 8, ..AfuConfig::engine(engine, line) }) {
        let p = afu.path().join(f.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.text).unwrap();
    }
    afu
}

/// `sw/logos_step.c` against the engine build at `afu` under Verilator: its exit status and its output.
fn step(afu: &Path, program: &Path, inject: bool, generate: usize, prompt: &[u32]) -> (Option<i32>, String) {
    step_powered_up(afu, program, inject, generate, prompt, None)
}

/// [`step`], with every register and memory no reset reaches starting as the noise of `seed` where given
/// (`LOGOS_SIM_SEED`), as the card's do after a partial reconfiguration.
fn step_powered_up(afu: &Path, program: &Path, inject: bool, generate: usize, prompt: &[u32], seed: Option<u64>) -> (Option<i32>, String) {
    step_sequences(afu, program, inject, generate, &[prompt], seed)
}

/// [`step_powered_up`] decoding a sequence for each of `prompts` at once, their prompts apart by `--`.
fn step_sequences(afu: &Path, program: &Path, inject: bool, generate: usize, prompts: &[&[u32]], seed: Option<u64>) -> (Option<i32>, String) {
    step_in(afu, program, inject, generate, prompts, seed, &[])
}

/// [`step_sequences`] with the step program's environment given `env` besides.
fn step_in(afu: &Path, program: &Path, inject: bool, generate: usize, prompts: &[&[u32]], seed: Option<u64>, env: &[(&str, &Path)]) -> (Option<i32>, String) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/d5005/run-cosim.sh");
    let mut c = Command::new("bash");
    let tokens: Vec<String> = prompts
        .iter()
        .enumerate()
        .flat_map(|(s, p)| (s > 0).then(|| "--".to_string()).into_iter().chain(p.iter().map(u32::to_string)))
        .collect();
    c.arg(script)
        .arg(afu)
        .arg("step")
        .args(inject.then_some("--inject"))
        .arg(program)
        .arg(generate.to_string())
        .args(tokens)
        .env("LOGOS_SIM", "verilator")
        .env("VERILATOR", common::verilator())
        .env("LOGOS_MAX_POLLS", (1u64 << 24).to_string())
        .env("TMPDIR", std::env::temp_dir());
    if let Some(s) = seed {
        c.env("LOGOS_SIM_SEED", s.to_string());
    }
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("bash");
    (o.status.code(), text(&o))
}

fn step_record(out: &str) -> serde_json::Value {
    let json = out.lines().find(|l| l.starts_with("{\"tokens\"")).unwrap_or_else(|| panic!("the step program's record in:\n{out}"));
    serde_json::from_str(json).unwrap()
}

/// The engine build of the accelerator under Verilator at `line`, a host program driving it as it would the card:
/// `ofs::write_engine_program`'s program for the tiny model — its memory flat on the channels, its tables, its
/// embeddings — run by `sw/logos_step.c` through ENGINE_CONFIG, ENGINE_TABLE, ENGINE_RUN and STORE_READ, decoding
/// greedily a token a step from a prompt. Every token is the one the machine decodes, and no step faults; with
/// `inject`, on SECDED lines with a bit flipped in every line of the memory image, every word the engine reads is
/// corrected and counted.
fn decodes_as_the_machine(line: LocalMemoryLine, inject: bool) {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_engine_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (prompt, generate) = ([3u32, 17, 41], 4usize);
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let what = format!("{line:?}{}", if inject { " with a flip in every line" } else { "" });
    let map = PhysicalMap::flat(&fresh.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap();
    let program = dir.path().join(format!("program-{}-{inject}", line.secded()));
    let p = write_engine_program(&fresh, &map, &program).unwrap();
    assert_eq!((p.space, p.base, p.vocab), (0, 0, fresh.spec.vocab), "{what}");
    let afu = engine_build(&fresh, line);
    let (code, out) = step(afu.path(), &program, inject, generate, &prompt);
    assert_eq!(code, Some(0), "{what}:\n{out}");
    let r = step_record(&out);
    let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
    assert_eq!(got, want, "{what}: the tokens the engine decodes");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{what}: {r}");
    if inject {
        let counts = r["secded"].as_array().unwrap_or_else(|| panic!("{what}: the counts in {r}"));
        assert!(counts.iter().all(|c| c["uncorrectable"] == 0), "{what}: {r}");
        assert!(counts.iter().map(|c| c["corrected"].as_u64().unwrap()).sum::<u64>() > 0, "{what}: the engine's reads corrected: {r}");
    }
}

/// The step program profiles a step on the hybrid engine build, as the card's profiles are taken: with LOGOS_PROFILE
/// naming a job list as `d5005_afu --trace-store` writes one (a "job <j> word <w> <kind>" line a listed job, the host's
/// embedding first), the prompt's last position runs each listed job's words up to the next listed job's alone, and the
/// program says each one's cycles as "<i> <kind> <cycles>" — every listed job past the embedding once, in order, its kind
/// its own, its cycles past zero — and still decodes the machine's tokens with no fault. Every ninth job of the step is
/// listed, so the profile is a few engine runs: each run alone costs the co-simulation an engine run's handshake.
#[test]
fn the_step_program_profiles_its_prompts_last_position_a_job_at_a_time_and_decodes_as_the_machine() {
    use logicaffeine_compile::spatial::decode_engine::machine::encode;
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (prompt, generate) = ([3u32, 17, 41], 2usize);
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let (afu, layout, set) = hybrid_build(&fresh, 8);
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, 512).unwrap();
    let program = dir.path().join("program-hybrid");
    write_hybrid_program(&fresh, &map, &compaction, &set, &mut Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), &program).unwrap();
    let jobs = fresh.jobs.clone();
    let mut word = vec![0usize; jobs.len() + 1];
    for j in 1..jobs.len() {
        word[j + 1] = word[j] + encode(&jobs[j..j + 1]).len();
    }
    let listed: Vec<usize> = std::iter::once(0).chain((1..jobs.len()).step_by(9)).collect();
    let list = dir.path().join("jobs.txt");
    let listing: String = listed.iter().map(|&j| format!("job {j} word {} {:?}\n", word[j.max(1)], jobs[j])).collect();
    std::fs::write(&list, listing).unwrap();
    let (code, out) = step_in(afu.path(), &program, false, generate, &[&prompt], None, &[("LOGOS_PROFILE", &list)]);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
    assert_eq!(got, want, "the tokens the engine decodes profiled");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
    let lines: Vec<(usize, String, u64)> = out
        .lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w.as_slice() {
                [j, kind, cycles] if kind.chars().all(|c| c.is_ascii_alphabetic()) => Some((j.parse().ok()?, kind.to_string(), cycles.parse().ok()?)),
                _ => None,
            }
        })
        .collect();
    let end = |i: usize| listed.get(i + 1).map_or(word[jobs.len()], |&j| word[j]);
    let ran: Vec<usize> = (1..listed.len()).filter(|&i| end(i) > word[listed[i]]).collect();
    assert!(ran.len() > 3, "the listed jobs run in several parts: {ran:?}");
    assert_eq!(lines.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), ran, "a line a listed job past the embedding, in order:\n{out}");
    for (i, kind, cycles) in &lines {
        let job = &jobs[listed[*i]];
        assert!(format!("{job:?}").starts_with(kind.as_str()), "listed job {i} is a {kind}: {job:?}");
        assert!(*cycles > 0, "listed job {i} ran in {cycles} cycles");
    }
}

/// Every program carries its job listing (`jobs.txt`, `ofs::job_listing`), the list a card profile (LOGOS_PROFILE) runs a
/// step by: job 0 the host's embedding at word 0, then each job of the step past it at the word the engine's job table
/// (`table0.bin`) opens it at, the table's words from there to the next job's (to `job_words` for the last) that job's
/// own encoding, its kind its `Debug` text — for the reference engine's program and a folded hybrid program of two
/// contexts alike. The listing read back from the job table alone (`ofs::job_listing_of`, `d5005_afu --job-listing`,
/// for a program written before programs carried one) is the same text.
#[test]
fn every_program_lists_its_jobs_at_the_words_its_job_table_opens_them_at() {
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode_engine::machine::{decode, encode};
    use logicaffeine_compile::spatial::decode_engine::ofs::{job_listing_of, write_contexts_program, write_engine_program};
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let reference = dir.path().join("program-reference");
    write_engine_program(&fresh, &PhysicalMap::flat(&fresh.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap(), &reference).unwrap();
    let batch = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 2, fresh.spec.ctx_len).unwrap().folded();
    let formats: Vec<GgmlType> = batch.layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    let layout = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) };
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let contexts = dir.path().join("program-contexts");
    write_contexts_program(&batch, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), true, 2, &contexts).unwrap();
    for (program, machine, what) in [(&reference, &fresh, "the reference engine's program"), (&contexts, &batch, "a folded hybrid program of two contexts")] {
        let listing = std::fs::read_to_string(program.join("jobs.txt")).unwrap_or_else(|e| panic!("{what}: jobs.txt: {e}"));
        let table: Vec<u64> =
            std::fs::read(program.join("table0.bin")).unwrap().chunks(64).map(|l| u64::from_le_bytes(l[..8].try_into().unwrap())).collect();
        let engine = std::fs::read_to_string(program.join("engine.txt")).unwrap();
        let job_words: usize = engine.lines().find_map(|l| l.strip_prefix("job_words ")).and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("{what}: {engine}"));
        assert_eq!(table.len(), job_words, "{what}: the job table holds the program's words");
        let lines: Vec<(usize, usize, String)> = listing
            .lines()
            .map(|l| {
                let w: Vec<&str> = l.splitn(5, ' ').collect();
                assert!(w.len() == 5 && w[0] == "job" && w[2] == "word", "{what}: a listing line `{l}`");
                (w[1].parse().unwrap(), w[3].parse().unwrap(), w[4].to_string())
            })
            .collect();
        assert_eq!(lines.len(), machine.jobs.len(), "{what}: a line a job of the step");
        assert_eq!(lines[0], (0, 0, "Embed".to_string()), "{what}: the host's embedding first, at word 0");
        assert_eq!(lines[1].1, 0, "{what}: the first job past the embedding opens the table");
        for (j, (n, w, kind)) in lines.iter().enumerate().skip(1) {
            let end = lines.get(j + 1).map_or(job_words, |l| l.1);
            assert_eq!(*n, j, "{what}: the jobs in order");
            assert!(*w < end, "{what}: job {j} opens at {w}, the next at {end}");
            assert_eq!(table[*w..end], encode(&machine.jobs[j..j + 1]), "{what}: job {j}'s words are its own encoding");
            assert_eq!(decode(&table[*w..end]).unwrap(), vec![machine.jobs[j].clone()], "{what}: job {j} read back from its words");
            assert_eq!(*kind, format!("{:?}", machine.jobs[j]).chars().take(240).collect::<String>(), "{what}: job {j}'s kind");
        }
        assert_eq!(job_listing_of(program).unwrap(), listing, "{what}: the listing read back from the job table alone");
    }
}

/// The step program decodes on the data64 engine build as the machine does.
#[test]
fn the_step_program_decodes_on_data_lines_as_the_machine_does() {
    decodes_as_the_machine(LocalMemoryLine::Data512, false);
}

/// The step program decodes on the SECDED engine build as the machine does.
#[test]
fn the_step_program_decodes_on_secded_lines_as_the_machine_does() {
    decodes_as_the_machine(LocalMemoryLine::Data512Secded, false);
}

/// The step program decodes on the SECDED engine build as the machine does with a bit flipped in every line of the
/// memory image, every word the engine reads corrected and counted.
#[test]
fn the_step_program_decodes_through_a_flip_in_every_secded_line_as_the_machine_does() {
    decodes_as_the_machine(LocalMemoryLine::Data512Secded, true);
}

/// A step that faults is reported, not decoded past: a program whose configuration names per-32 activation scaling
/// for matrices the fastest point scales per block faults the engine at its first matrix, and one whose card-port space
/// starts past the memory image faults the card port; either way the step program stops, names which, and fails.
#[test]
fn a_faulting_step_is_reported_by_the_engines_run() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_engine_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let map = PhysicalMap::flat(&fresh.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap();
    let afu = engine_build(&fresh, LocalMemoryLine::Data512);
    for (key, value, says) in [("knobs", None, "the engine faulted"), ("space", Some(1u64 << 30), "the card port faulted")] {
        let program = dir.path().join(format!("program-{key}"));
        let p = write_engine_program(&fresh, &map, &program).unwrap();
        let text_of = std::fs::read_to_string(program.join("engine.txt")).unwrap();
        let wrong = value.unwrap_or(p.knobs as u64 & !8);
        assert_ne!(wrong, if key == "knobs" { p.knobs as u64 } else { p.space }, "{key} changes");
        let edited: String = text_of.lines().map(|l| if l.starts_with(&format!("{key} ")) { format!("{key} {wrong}\n") } else { format!("{l}\n") }).collect();
        std::fs::write(program.join("engine.txt"), edited).unwrap();
        let (code, out) = step(afu.path(), &program, false, 0, &[3]);
        assert_eq!(code, Some(1), "{key}:\n{out}");
        assert!(out.contains(says), "{key}:\n{out}");
        assert_eq!(step_record(&out)["faults"], 1, "{key}:\n{out}");
    }
}

/// The hybrid engine build of the accelerator for `machine` — its matvec jobs on a streaming unit of one pair a channel
/// for the formats the machine streams, its compute clock the user clock divided by `divide` — and the image layout
/// its matrices lie in, its tree under a fresh directory.
fn hybrid_build(machine: &Machine, divide: u32) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_with(machine, divide, false, false)
}

/// [`hybrid_build`], with the recurrence unit beside the streaming unit when `recurrence`, and the vector unit's side
/// beside that when `vectors`.
fn hybrid_build_with(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_rows(machine, divide, recurrence, vectors, 1)
}

/// [`hybrid_build_with`] for `rows` token-rows a step, one a sequence of `machine`'s.
fn hybrid_build_rows(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
    rows: usize,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_shaped(machine, divide, recurrence, vectors, rows, 1, false, 1)
}

/// [`hybrid_build_shaped`] with the attention side beside the vector unit's side.
fn hybrid_build_attended(
    machine: &Machine,
    divide: u32,
    rows: usize,
    pairs: u64,
    width: usize,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_with_units(machine, divide, true, true, true, rows, pairs, true, width, 1, 1)
}

/// [`hybrid_build_attended`] with the recurrence unit's value heads on `lanes` head lanes, each on a fast port of its
/// own, the ports sharing the channels through the channel mux.
fn hybrid_build_on_lanes(
    machine: &Machine,
    divide: u32,
    rows: usize,
    pairs: u64,
    width: usize,
    lanes: usize,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_with_units(machine, divide, true, true, true, rows, pairs, true, width, lanes, 1)
}

/// [`hybrid_build_rows`] with `pairs` row pairs a channel, its store laid greedy when `greedy`, its recurrence unit
/// taking `width` sixteens of elements a clock.
#[allow(clippy::too_many_arguments)]
fn hybrid_build_shaped(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
    rows: usize,
    pairs: u64,
    greedy: bool,
    width: usize,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_with_units(machine, divide, recurrence, vectors, false, rows, pairs, greedy, width, 1, 1)
}

/// [`hybrid_build_shaped`] with the attention side beside the vector unit's side when `attention`, the recurrence
/// unit's value heads on `lanes` head lanes, and `contexts` sequencer contexts sharing the units, `rows` token-rows each.
#[allow(clippy::too_many_arguments)]
fn hybrid_build_with_units(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
    attention: bool,
    rows: usize,
    pairs: u64,
    greedy: bool,
    width: usize,
    lanes: usize,
    contexts: usize,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_packed(machine, divide, recurrence, vectors, attention, rows, pairs, greedy, width, lanes, contexts, false)
}

/// [`hybrid_build_with_units`], the streaming unit's cores paired on processing elements in a clock of twice theirs
/// when `pe_pairs`.
#[allow(clippy::too_many_arguments)]
fn hybrid_build_packed(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
    attention: bool,
    rows: usize,
    pairs: u64,
    greedy: bool,
    width: usize,
    lanes: usize,
    contexts: usize,
    pe_pairs: bool,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid_build_tuned(machine, divide, recurrence, vectors, attention, rows, pairs, greedy, width, lanes, contexts, pe_pairs, &|s| s)
}

/// [`hybrid_build_packed`], its streaming unit then `tune`d.
#[allow(clippy::too_many_arguments)]
fn hybrid_build_tuned(
    machine: &Machine,
    divide: u32,
    recurrence: bool,
    vectors: bool,
    attention: bool,
    rows: usize,
    pairs: u64,
    greedy: bool,
    width: usize,
    lanes: usize,
    contexts: usize,
    pe_pairs: bool,
    tune: &dyn Fn(logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming) -> logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
    let store = if greedy { StoreLayout::greedy(&machine.jobs, &machine.layout, rows) } else { StoreLayout::of(&machine.jobs, &machine.layout, rows) };
    let formats: Vec<GgmlType> = machine.layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    let set = if pe_pairs { set.with_pe_pairs() } else { set };
    let layout = ImageLayout { stripe: Some(pairs), ..ImageLayout::d5005(Beat::Data64) };
    let streaming = Streaming::new(set.clone(), layout);
    let streaming = if recurrence {
        streaming.with_recurrence().with_recurrence_width(width).with_recurrence_lanes(lanes, machine.layout.log_lines())
    } else {
        streaming
    };
    let streaming = if vectors { streaming.with_vectors() } else { streaming };
    let streaming = if attention { streaming.with_attention() } else { streaming };
    let streaming = if contexts > 1 { streaming.with_contexts(contexts) } else { streaming };
    let params = EngineParams::of(&machine.layout, &store, &machine.jobs);
    let streaming = if params.folds { streaming.with_folds(4) } else { streaming };
    let engine = EngineParams { streaming: Some(tune(streaming)), ..params };
    let afu = tempfile::tempdir().unwrap();
    for f in afu_sources(&AfuConfig { compute_divide: divide, ..AfuConfig::engine(engine, LocalMemoryLine::Data512) }) {
        let p = afu.path().join(f.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.text).unwrap();
    }
    (afu, layout, set)
}

/// The hybrid engine build of the accelerator under Verilator, a host program driving it as it would the card:
/// `ofs::write_hybrid_program`'s program for the tiny model — its matrices in stripes at each channel's start, the rest
/// of its memory compacted after them with each matrix's stream entry, its tables, its embeddings — run by
/// `sw/logos_step.c`, decoding greedily a token a step from a prompt, the core's clock an eighth of the user clock the
/// streaming unit runs in. Every token is the one the machine decodes, and no step faults.
#[test]
fn the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (prompt, generate) = ([3u32, 17, 41], 4usize);
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let (afu, layout, set) = hybrid_build(&fresh, 8);
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, 512).unwrap();
    let program = dir.path().join("program-hybrid");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_hybrid_program(&fresh, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert_eq!((p.space, p.base), (0, map.base));
    assert_eq!(p.streaming, format!("{}/1", set.signature()));
    let (code, out) = step(afu.path(), &program, false, generate, &prompt);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
    assert_eq!(got, want, "the tokens the hybrid engine decodes");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// A hybrid engine build under Verilator from the state a partial reconfiguration leaves it in: every register and
/// memory no reset reaches holding noise rather than zeros — the token-row store but for what the host writes, the
/// vector unit's registers, the tables past the program's entries, the streaming unit's memories — and the step
/// program's first run on it, as the card's first run after `fpgaconf` is. The build is the plain hybrid's, or with the
/// recurrence unit (and the vector unit's side) beside the streaming unit over the space dealt at eight lines a granule
/// and a prompt of fourteen tokens, the log filling and flushing as in the recurrent and vectored decodes. Every token
/// is the one the machine decodes and no step faults, for two seeds of noise.
fn decodes_from_any_power_up_state(recurrence: bool, vectors: bool) {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompt: Vec<u32> = if recurrence { vec![3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2] } else { vec![3, 17, 41] };
    let generate = 4usize;
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let (afu, layout, set) = hybrid_build_with(&fresh, 8, recurrence, vectors);
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, if recurrence { 8 } else { 512 }).unwrap();
    let program = dir.path().join("program-powered-up");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_hybrid_program(&fresh, &map, &compaction, &set, &mut weights, &program).unwrap();
    for seed in [1u64, 2] {
        let what = format!("recurrence {recurrence}, vectors {vectors}, seed {seed}");
        let (code, out) = step_powered_up(afu.path(), &program, false, generate, &prompt, Some(seed));
        assert_eq!(code, Some(0), "{what}:\n{out}");
        let r = step_record(&out);
        let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
        assert_eq!(got, want, "{what}: the tokens the hybrid engine decodes from its power-up state");
        assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{what}: {r}");
    }
}

#[test]
fn the_step_program_decodes_on_the_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding() {
    decodes_from_any_power_up_state(false, false);
}

#[test]
fn the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding() {
    decodes_from_any_power_up_state(true, false);
}

#[test]
fn the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does_whatever_it_powers_up_holding() {
    decodes_from_any_power_up_state(true, true);
}

/// The hybrid engine build with the recurrence unit beside its streaming unit under Verilator, driven as the card is:
/// the hybrid program for the tiny model, the space past its image dealt over the four channels at eight lines a
/// granule, so a head's memory and its log span every channel, decoding greedily from a prompt of fourteen tokens four
/// past it — the sixteenth filling the log and flushing it on the fast side, the seventeenth logging on the memory the
/// flush wrote — the core's clock an eighth of the user clock. Every token is the one the machine decodes, and no step
/// faults.
#[test]
fn the_step_program_decodes_on_the_recurrent_hybrid_engine_as_the_machine_does() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompt = [3u32, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2];
    let generate = 4usize;
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let (afu, layout, set) = hybrid_build_with(&fresh, 8, true, false);
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-recurrent");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_hybrid_program(&fresh, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert_eq!((p.space, p.base, p.granule_log2), (0, map.base, 3));
    let (code, out) = step(afu.path(), &program, false, generate, &prompt);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
    assert_eq!(got, want, "the tokens the recurrent hybrid engine decodes");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// The hybrid engine build with the recurrence unit and the vector unit's side beside its streaming unit under
/// Verilator, driven as the card is: the hybrid program for the tiny model, the space past its image dealt over the four
/// channels at eight lines a granule, decoding greedily from a prompt of fourteen tokens four past it — every vector job
/// on the side in the user clock, its ring rows and cache rows written through the recurrence unit's port, the
/// sixteenth token filling the log and flushing it — the core's clock an eighth of the user clock. Every token is the
/// one the machine decodes, and no step faults.
#[test]
fn the_step_program_decodes_on_the_vectored_hybrid_engine_as_the_machine_does() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompt = [3u32, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2];
    let generate = 4usize;
    let (fresh, want) = decoding(dir.path(), &prompt, generate);
    let (afu, layout, set) = hybrid_build_with(&fresh, 8, true, true);
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-vectored");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_hybrid_program(&fresh, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert_eq!((p.space, p.base, p.granule_log2), (0, map.base, 3));
    let (code, out) = step(afu.path(), &program, false, generate, &prompt);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    assert_eq!(r["prompt"].as_u64(), Some(prompt.len() as u64), "the program took the whole prompt: {r}");
    let got: Vec<u32> = r["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect();
    assert_eq!(got, want, "the tokens the vectored hybrid engine decodes");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// A program written for another streaming unit than the build's is refused before the card is touched: the
/// reference engine's program on the hybrid build, and the hybrid engine's on the reference build.
/// The vectored hybrid engine built for three token-rows under Verilator, driven as the card is: the hybrid program for
/// the tiny model written for three sequences, the space past its image dealt over the four channels at eight lines a
/// granule, and `sw/logos_step.c` decoding three prompts of different lengths at once — each sequence its own row of
/// the row table, its own region (its memories, logs and caches), its own residual and logits rows — four tokens past
/// each, the first sequence's sixteenth token filling its log and flushing it, the core's clock an eighth of the user
/// clock. Every sequence's tokens are the ones the machine decodes for its prompt alone, and no step faults.
#[test]
fn the_step_program_decodes_three_sequences_at_once_on_the_vectored_hybrid_engine_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 3] = [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 3, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_rows(&batch, 8, true, true, 3);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-three");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_hybrid_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert_eq!(p.sequences, 3, "a region a sequence");
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens, decoded beside the others, are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// The greedy program of the tiny model for six sequences on the vectored hybrid build of four pairs a channel whose
/// store is laid greedy, under Verilator and driven as the card is: each step's head leaves every row's arg-max in the
/// store, a beat a row, which is all `sw/logos_step.c` reads back of it. Six prompts of different lengths decode at
/// once, four tokens past each, the first sequence's sixteenth token filling its log and flushing it, the core's clock
/// an eighth of the user clock, the space past the image dealt over the four channels at eight lines a granule. Every
/// sequence's tokens are the ones the machine decodes for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_on_four_pairs_a_channel_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_shaped(&batch, 8, true, true, 6, 4, true, 1);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert!(p.greedy && p.sequences == 6 && p.logits.1 == 1, "a greedy program of six rows reads a beat a row back: {p:?}");
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens, decoded greedily beside the others, are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// [`the_greedy_step_program_decodes_six_sequences_at_once_on_four_pairs_a_channel_as_the_machine_decodes_each`] on an
/// AFU whose streaming unit pairs its cores on processing elements in the PIM's uClk_usr, its user side in
/// uClk_usrDiv2 (the cosimulation's uclk2 at half the user period): six prompts decoded at once, each sequence's tokens
/// the machine's, from a program written for the same set as a build of lone cores takes.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_on_paired_cores_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_packed(&batch, 8, true, true, false, 6, 4, true, 1, 1, 1, true);
    assert!(set.pe_pairs && std::fs::read_to_string(afu.path().join("sim/logos_d5005_cosim.v")).unwrap().contains("uclk2"), "a paired build");
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-paired");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens, decoded greedily on paired cores beside the others, are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// [`the_greedy_step_program_decodes_six_sequences_at_once_on_four_pairs_a_channel_as_the_machine_decodes_each`] with
/// the recurrence unit taking a binary16 line a clock: six prompts decoded at once on the vectored build of four pairs a
/// channel and a greedy store, each sequence's tokens the machine's, the first sequence's log flushed on the way.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_with_the_recurrence_at_a_line_a_clock_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_shaped(&batch, 8, true, true, 6, 4, true, 2);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-wide");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// The greedy program of six sequences on the build whose core only sequences: every matvec on the streaming unit at
/// four pairs a channel, every lazily flushed token-row on the recurrence unit at a binary16 line a clock, every vector
/// job on the vector unit's side and every attention row on the attention side, the store laid greedy. Six prompts
/// decoded at once, four tokens past each, the first sequence's log flushed on the way; each sequence's tokens are the
/// machine's for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_with_every_unit_on_the_fast_side_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_attended(&batch, 8, 6, 4, 2);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-fast");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// [`the_greedy_step_program_decodes_six_sequences_at_once_with_every_unit_on_the_fast_side_as_the_machine_decodes_each`]
/// on a step whose residual adds and gated products are folded into its matrix jobs ([`Machine::folded`]), its
/// streaming unit folding each sum into its output slot over four lanes: six prompts decoded at once, the first
/// sequence's log flushed on the way, each sequence's tokens the unfolded machine's for its prompt alone, and no step
/// faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_on_a_folding_engine_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len)
        .unwrap()
        .folded();
    let (afu, layout, set) = hybrid_build_attended(&batch, 8, 6, 4, 2);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-folded");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    assert!(std::fs::read_to_string(program.join("engine.txt")).unwrap().contains("+folds\n"), "the program names the folding unit");
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// A folded program is refused before the card is touched on a build whose streaming unit writes each sum as it is,
/// and an unfolded program on a build that folds: each build and its programs name the unit with its folds, and the
/// step program, built against the physical backend, exits 2 naming both units and runs no step.
#[test]
fn the_step_program_refuses_a_folded_program_on_a_build_that_writes_its_sums_and_the_other_way() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let plain = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 2, fresh.spec.ctx_len).unwrap();
    let folded = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 2, fresh.spec.ctx_len).unwrap().folded();
    let (writes, layout, set) = hybrid_build_attended(&plain, 8, 2, 1, 2);
    let (folds, _, _) = hybrid_build_attended(&folded, 8, 2, 1, 2);
    let write = |machine: &Machine, name: &str| {
        let (map, _, compaction) = PhysicalMap::hybrid(&machine.layout, &layout, 8).unwrap();
        let at = dir.path().join(name);
        write_greedy_program(machine, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), &at).unwrap();
        at
    };
    let (unfolded_program, folded_program) = (write(&plain, "program-plain"), write(&folded, "program-folded"));
    for (afu, program, what) in [(writes.path(), &folded_program, "a folded program on a build that writes its sums"), (folds.path(), &unfolded_program, "an unfolded program on a build that folds")] {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.join("sw"))
            .arg(afu.join("sw/logos_hal_pci.c"))
            .arg(afu.join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{what}: {}", text(&o));
        let o = Command::new(afu.join("logos_step")).arg(program).args(["0", "3", "--", "5"]).output().expect("logos_step");
        assert_eq!(o.status.code(), Some(2), "{what}: refused before the card:\n{}", text(&o));
        assert!(text(&o).contains("is a program for the streaming unit"), "{what}:\n{}", text(&o));
        assert!(!text(&o).contains("{\"tokens\""), "{what}: no step run:\n{}", text(&o));
    }
}

/// A program whose memories are flushed a window behind is refused before the card is touched on a build whose head lanes
/// replay a whole log at once, and a program flushed every window on a build whose lanes replay a window behind: each
/// build and its programs name the unit with its recurrence's window, and the step program, built against the physical
/// backend, exits 2 naming both units and runs no step.
#[test]
fn the_step_program_refuses_a_program_flushed_a_window_behind_on_a_build_that_replays_whole_logs_and_the_other_way() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    use logicaffeine_compile::spatial::serving::StateFlush;
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let at = |flush| {
        Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics { flush, ..EngineNumerics::fastest() }, 2, fresh.spec.ctx_len).unwrap()
    };
    let (every, behind) = (at(EngineNumerics::fastest().flush), at(StateFlush::Trailing { tokens: 4 }));
    let (whole, layout, set) = hybrid_build_on_lanes(&every, 8, 2, 1, 2, 4);
    let (trailing, _, _) = hybrid_build_on_lanes(&behind, 8, 2, 1, 2, 4);
    let write = |machine: &Machine, name: &str| {
        let (map, _, compaction) = PhysicalMap::hybrid(&machine.layout, &layout, 8).unwrap();
        let at = dir.path().join(name);
        let p = write_greedy_program(machine, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), &at).unwrap();
        (at, p.streaming)
    };
    let ((every_program, every_unit), (behind_program, behind_unit)) = (write(&every, "program-every"), write(&behind, "program-behind"));
    assert_eq!(behind_unit, format!("{every_unit}+behind4"), "a trailing program names its window");
    for (afu, program, what) in [
        (whole.path(), &behind_program, "a program flushed a window behind on a build that replays whole logs"),
        (trailing.path(), &every_program, "a program flushed every window on a build that replays a window behind"),
    ] {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.join("sw"))
            .arg(afu.join("sw/logos_hal_pci.c"))
            .arg(afu.join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{what}: {}", text(&o));
        let o = Command::new(afu.join("logos_step")).arg(program).args(["0", "3", "--", "5"]).output().expect("logos_step");
        assert_eq!(o.status.code(), Some(2), "{what}: refused before the card:\n{}", text(&o));
        assert!(text(&o).contains("is a program for the streaming unit"), "{what}:\n{}", text(&o));
        assert!(!text(&o).contains("{\"tokens\""), "{what}: no step run:\n{}", text(&o));
    }
}

/// [`the_greedy_step_program_decodes_six_sequences_at_once_with_every_unit_on_the_fast_side_as_the_machine_decodes_each`]
/// with the recurrence's value heads on four head lanes, each lane's rows through a fast port of its own and every port
/// on the channels at once through the mux: six prompts decoded at once, the first sequence's log flushed on the way,
/// each sequence's tokens the machine's for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_with_its_recurrence_on_four_head_lanes_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_on_lanes(&batch, 8, 6, 4, 2, 4);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-lanes");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// [`the_greedy_step_program_decodes_six_sequences_at_once_with_its_recurrence_on_four_head_lanes_as_the_machine_decodes_each`]
/// with the recurrence as hybrid9c carries it: on two head lanes, each lane and the coordinator asking 128 lines ahead,
/// the memories flushed a window of four tokens behind. The longest prompt's log replays a share each token from its
/// fifth on, drops a window at every eighth, and wraps its ring of eight slots. Each sequence's tokens are the ones the
/// machine decodes at that knob point for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_on_two_head_lanes_reading_far_ahead_flushed_a_window_behind_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    use logicaffeine_compile::spatial::serving::StateFlush;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let behind = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding_at(dir.path(), p, generate, behind).1).collect();
    let (one, _) = decoding_at(dir.path(), prompts[0], 0, behind);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), behind, 6, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_tuned(&batch, 8, true, true, true, 6, 4, true, 2, 2, 1, false, &|s| s.with_recurrence_credit(128));
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-six-behind");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// Six sequences decoded at once on a build whose head lanes make each Gated-DeltaNet block's gated norm
/// (`Streaming::with_delta_norm`), the program's gated norms folded into its recurrence jobs (`Machine::delta_normed`):
/// on four head lanes flushed every sixteen tokens, and on two lanes reading 128 lines ahead flushed a window of four
/// tokens behind, as hybrid9c carries its recurrence. The program and the build name the norm (`+norm`); every
/// sequence's tokens are the ones the machine decodes for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_six_sequences_at_once_with_its_gated_norms_made_on_the_head_lanes_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::machine::Job;
    use logicaffeine_compile::spatial::decode_engine::ofs::write_greedy_program;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::Streaming;
    use logicaffeine_compile::spatial::serving::StateFlush;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let prompts: [&[u32]; 6] =
        [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7], &[44, 21, 2, 9], &[12, 33, 8]];
    let generate = 4usize;
    let behind = EngineNumerics { flush: StateFlush::Trailing { tokens: 4 }, ..EngineNumerics::fastest() };
    for (what, n, lanes, credit) in [("every sixteen tokens on four lanes", EngineNumerics::fastest(), 4, None), ("a window behind on two lanes", behind, 2, Some(128))] {
        let dir = tempfile::tempdir().unwrap();
        let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding_at(dir.path(), p, generate, n).1).collect();
        let (one, _) = decoding_at(dir.path(), prompts[0], 0, n);
        let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), n, 6, one.spec.ctx_len).unwrap().delta_normed();
        assert!(batch.jobs.iter().any(|j| matches!(j, Job::Delta { norm: Some(_), .. })), "{what}: the program's recurrences carry their norms");
        let tune = |s: Streaming| {
            let s = s.with_delta_norm();
            match credit {
                Some(c) => s.with_recurrence_credit(c),
                None => s,
            }
        };
        let (afu, layout, set) = hybrid_build_tuned(&batch, 8, true, true, true, 6, 4, true, 2, lanes, 1, false, &tune);
        let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
        let program = dir.path().join("program-six-normed");
        let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
        let p = write_greedy_program(&batch, &map, &compaction, &set, &mut weights, &program).unwrap();
        assert!(p.streaming.contains("+norm"), "{what}: the program names its norms: {}", p.streaming);
        let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
        assert_eq!(code, Some(0), "{what}: {out}");
        let r = step_record(&out);
        let got: Vec<Vec<u32>> = r["sequences"]
            .as_array()
            .unwrap_or_else(|| panic!("{what}: every sequence's tokens in {r}"))
            .iter()
            .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
            .collect();
        assert_eq!(got, wants, "{what}: each sequence's tokens are the machine's for its prompt alone");
        assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{what}: {r}");
    }
}

/// The greedy program of the tiny model for four sequences dealt over two sequencer contexts — sequence s in context
/// s mod 2 as its row s / 2, each context its own core, rows and bank of the store, the units shared — on the build
/// with every unit on the fast side at four pairs a channel, under Verilator and driven as the card is. Four prompts of
/// different lengths decode at once, four tokens past each, the first sequence's sixteenth token filling its log and
/// flushing it, the core's clock an eighth of the user clock. Every sequence's tokens are the ones the machine decodes
/// for its prompt alone, and no step faults.
#[test]
fn the_greedy_step_program_decodes_four_sequences_over_two_contexts_as_the_machine_decodes_each() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_contexts_program;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let prompts: [&[u32]; 4] = [&[3, 17, 41, 8, 29, 60, 5, 9, 33, 12, 7, 44, 21, 2], &[5, 9], &[60, 2, 41, 17, 3], &[7]];
    let generate = 4usize;
    let wants: Vec<Vec<u32>> = prompts.iter().map(|p| decoding(dir.path(), p, generate).1).collect();
    let (one, _) = decoding(dir.path(), prompts[0], 0);
    let batch = Machine::new(one.spec.clone(), Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), EngineNumerics::fastest(), 4, one.spec.ctx_len).unwrap();
    let (afu, layout, set) = hybrid_build_with_units(&batch, 8, true, true, true, 2, 4, true, 2, 1, 2);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-contexts");
    let mut weights = Gguf::open(&dir.path().join("tiny.gguf")).unwrap();
    let p = write_contexts_program(&batch, &map, &compaction, &set, &mut weights, true, 2, &program).unwrap();
    assert!(p.greedy && p.sequences == 4 && p.contexts == 2, "{p:?}");
    let (code, out) = step_sequences(afu.path(), &program, false, generate, &prompts, None);
    assert_eq!(code, Some(0), "{out}");
    let r = step_record(&out);
    let got: Vec<Vec<u32>> = r["sequences"]
        .as_array()
        .unwrap_or_else(|| panic!("every sequence's tokens in {r}"))
        .iter()
        .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(got, wants, "each sequence's tokens, decoded in its context beside the others, are the machine's for its prompt alone");
    assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{r}");
}

/// A program is refused before the card is touched when it was written at the other multiply than its build's
/// products: a program of the engine's own multiply on a build of the card's floating-point blocks' products
/// (`LOGOS_D5005_FP_HARD`), and one written at the card's products (`fp hard` in its engine.txt, the machine's knob point
/// `fp_unit: FpUnit::Hard`) on a build of the engine's. The step program exits 2 saying both and runs no step; a program
/// on its own build's products is not refused for it.
#[test]
fn the_step_program_refuses_a_program_written_at_the_other_multiply() {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
    use logicaffeine_compile::spatial::decode_engine::ofs::write_contexts_program;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, HardFloat, Streaming};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let at = |fp_unit| Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics { fp_unit, ..EngineNumerics::fastest() }, 1, fresh.spec.ctx_len).unwrap();
    let (soft_machine, hard_machine) = (at(FpUnit::Soft), at(FpUnit::Hard));
    let formats: Vec<GgmlType> = soft_machine.layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    let layout = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) };
    let sum = Block { semantics: Semantics::ENGINE, full: FADD_LATENCY, least: 1 };
    let card = Block { semantics: FpUnit::CARD, full: 4, least: 1 };
    let build = |hard: HardFloat| {
        let streaming = Streaming::new(set.clone(), layout).with_recurrence().with_vectors().with_hard_float(hard).unwrap();
        let store = StoreLayout::greedy(&soft_machine.jobs, &soft_machine.layout, 1);
        let engine = EngineParams { streaming: Some(streaming), ..EngineParams::of(&soft_machine.layout, &store, &soft_machine.jobs) };
        let afu = tempfile::tempdir().unwrap();
        for f in afu_sources(&AfuConfig { compute_divide: 8, ..AfuConfig::engine(engine, LocalMemoryLine::Data512) }) {
            let p = afu.path().join(f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, f.text).unwrap();
        }
        afu
    };
    let soft = build(HardFloat { product: None, sum, fp_unit: FpUnit::Soft });
    let hard = build(HardFloat { product: Some(card), sum, fp_unit: FpUnit::Hard });
    let header = |afu: &tempfile::TempDir| std::fs::read_to_string(afu.path().join("sw/logos_d5005.h")).unwrap();
    assert!(header(&hard).contains("#define LOGOS_D5005_FP_HARD 1\n"), "the card's products' build says so");
    assert!(!header(&soft).contains("LOGOS_D5005_FP_HARD"), "the engine's multiply's build names none");
    let (map, _, compaction) = PhysicalMap::hybrid(&soft_machine.layout, &layout, 8).unwrap();
    let write = |machine: &Machine, name: &str| {
        let at = dir.path().join(name);
        let p = write_contexts_program(machine, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), true, 1, &at).unwrap();
        (at, p)
    };
    let ((at_soft, p_soft), (at_hard, p_hard)) = (write(&soft_machine, "program-soft"), write(&hard_machine, "program-hard"));
    assert_eq!((p_soft.fp_unit, p_hard.fp_unit), (FpUnit::Soft, FpUnit::Hard));
    let engine_txt = |at: &std::path::Path| std::fs::read_to_string(at.join("engine.txt")).unwrap();
    assert!(engine_txt(&at_hard).contains("\nfp hard\n"), "{}", engine_txt(&at_hard));
    assert!(!engine_txt(&at_soft).contains("\nfp "), "{}", engine_txt(&at_soft));
    let step = |afu: &tempfile::TempDir, program: &std::path::Path| {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.path().join("sw"))
            .arg(afu.path().join("sw/logos_hal_pci.c"))
            .arg(afu.path().join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.path().join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{}", text(&o));
        Command::new(afu.path().join("logos_step")).arg(program).args(["0", "3"]).output().expect("logos_step")
    };
    for (afu, program, what) in [(&hard, &at_soft, "the engine's multiply's program on the card's products"), (&soft, &at_hard, "the card's products' program on the engine's multiply")] {
        let o = step(afu, program);
        assert_eq!(o.status.code(), Some(2), "{what}: refused before the card:\n{}", text(&o));
        assert!(text(&o).contains("products, this build's are"), "{what}:\n{}", text(&o));
        assert!(!text(&o).contains("{\"tokens\""), "{what}: no step run:\n{}", text(&o));
    }
    for (afu, program) in [(&soft, &at_soft), (&hard, &at_hard)] {
        assert!(!text(&step(afu, program)).contains("products, this build's are"), "a program on its own build's products");
    }
}

/// A program is refused before the card is touched when it was written for other sequencer contexts than its build
/// runs — one context's program on a build of two, two contexts' on a build of one — and a program whose sequences
/// would leave a context short is never written. The step program, built against the physical backend, exits 2 saying
/// both counts and runs no step.
#[test]
fn the_step_program_refuses_a_program_written_for_other_contexts() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_contexts_program;
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let batch = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 2, fresh.spec.ctx_len).unwrap();
    let (two, layout, set) = hybrid_build_with_units(&batch, 8, true, true, true, 1, 1, true, 2, 1, 2);
    let (one, _, _) = hybrid_build_with_units(&batch, 8, true, true, true, 2, 1, true, 2, 1, 1);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let write = |contexts: usize, name: &str| {
        let at = dir.path().join(name);
        write_contexts_program(&batch, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), true, contexts, &at).unwrap();
        at
    };
    let (in_one, in_two) = (write(1, "program-one"), write(2, "program-two"));
    let three = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 3, fresh.spec.ctx_len).unwrap();
    let (map3, _, compaction3) = PhysicalMap::hybrid(&three.layout, &layout, 8).unwrap();
    let short = write_contexts_program(&three, &map3, &compaction3, &set, &mut Gguf::open(&tiny).unwrap(), true, 2, &dir.path().join("program-short"));
    assert!(short.is_err_and(|e| e.to_string().contains("3 sequences over 2 contexts")), "three sequences leave a context short");
    for (afu, program, what) in [(two.path(), &in_one, "one context's program on a build of two"), (one.path(), &in_two, "two contexts' program on a build of one")] {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.join("sw"))
            .arg(afu.join("sw/logos_hal_pci.c"))
            .arg(afu.join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{what}: {}", text(&o));
        let o = Command::new(afu.join("logos_step")).arg(program).args(["0", "3", "--", "5"]).output().expect("logos_step");
        assert_eq!(o.status.code(), Some(2), "{what}: refused before the card:\n{}", text(&o));
        assert!(text(&o).contains("contexts, this build runs"), "{what}:\n{}", text(&o));
        assert!(!text(&o).contains("{\"tokens\""), "{what}: no step run:\n{}", text(&o));
    }
}

/// A program is refused before the card is touched when it steps more rows than its build holds or lays a store larger
/// than the build's: a program for three sequences on a build of one token-row, and a program that reads its logits
/// back on a build whose store is laid greedy, for a model whose vocabulary outweighs the rest of its step as a real
/// model's does (4,096 words; the tiny model's 96 logits fit within the beats its widest job holds, and the store is
/// laid no larger for them). The step program, built against the physical backend, exits 2 saying both sizes and runs
/// no step.
#[test]
fn the_step_program_refuses_a_program_past_its_builds_rows_or_store() {
    use logicaffeine_compile::spatial::decode_engine::ofs::write_hybrid_program;
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let tiny = dir.path().join("tiny.gguf");
    let batch = Machine::new(fresh.spec.clone(), Gguf::open(&tiny).unwrap(), EngineNumerics::fastest(), 3, fresh.spec.ctx_len).unwrap();
    let (one_row, layout, set) = hybrid_build_shaped(&fresh, 8, true, true, 1, 1, false, 1);
    let three = dir.path().join("program-three");
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    write_hybrid_program(&batch, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), &three).unwrap();
    let mut spec = fresh.spec.clone();
    spec.vocab = 4096;
    let large = dir.path().join("large.gguf");
    tiny_models::write_checkpoint(&spec, &large);
    let wordy = Machine::new(spec.clone(), Gguf::open(&large).unwrap(), EngineNumerics::fastest(), 1, spec.ctx_len).unwrap();
    let (greedy, layout, set) = hybrid_build_shaped(&wordy, 8, true, true, 1, 1, true, 1);
    let logits = dir.path().join("program-logits");
    let (map, _, compaction) = PhysicalMap::hybrid(&wordy.layout, &layout, 8).unwrap();
    write_hybrid_program(&wordy, &map, &compaction, &set, &mut Gguf::open(&large).unwrap(), &logits).unwrap();
    for (afu, program, what) in [(one_row.path(), &three, "three sequences on a build of one row"), (greedy.path(), &logits, "the logits read back on a greedy build")] {
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.join("sw"))
            .arg(afu.join("sw/logos_hal_pci.c"))
            .arg(afu.join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{what}: {}", text(&o));
        let o = Command::new(afu.join("logos_step")).arg(program).args(["0", "3"]).output().expect("logos_step");
        assert_eq!(o.status.code(), Some(2), "{what}: refused before the card:\n{}", text(&o));
        assert!(text(&o).contains("past this build's"), "{what}:\n{}", text(&o));
        assert!(!text(&o).contains("{\"tokens\""), "{what}: no step run:\n{}", text(&o));
    }
}

#[test]
fn the_step_program_refuses_a_program_for_another_streaming_unit() {
    use logicaffeine_compile::spatial::decode_engine::ofs::{write_engine_program, write_hybrid_program};
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let (hybrid, layout, set) = hybrid_build(&fresh, 8);
    let reference = engine_build(&fresh, LocalMemoryLine::Data512);
    let flat = PhysicalMap::flat(&fresh.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap();
    let flat_program = dir.path().join("program-flat");
    write_engine_program(&fresh, &flat, &flat_program).unwrap();
    let (map, _, compaction) = PhysicalMap::hybrid(&fresh.layout, &layout, 512).unwrap();
    let hybrid_program = dir.path().join("program-hybrid");
    write_hybrid_program(&fresh, &map, &compaction, &set, &mut Gguf::open(&dir.path().join("tiny.gguf")).unwrap(), &hybrid_program).unwrap();
    for (afu, program, what) in [(hybrid.path(), &flat_program, "the reference program on the hybrid build"), (reference.path(), &hybrid_program, "the hybrid program on the reference build")] {
        let (code, out) = step(afu, program, false, 0, &[3]);
        assert_eq!(code, Some(1), "{what}: the co-simulation fails as the program does:\n{out}");
        assert!(out.contains("is a program for the streaming unit"), "{what}:\n{out}");
        assert!(!out.contains("{\"tokens\""), "{what}: no step run:\n{out}");
    }
}

/// A co-simulation hands its host program every argument it was given as `+arg=<argument>`, in order and however many
/// — from the simulator's argument list (`sim_start`, Icarus Verilog's VPI) and from the process's own command line
/// (`sim_cmdline`, Verilator's DPI-C) — and nothing else: eight hundred prompt tokens between other plusargs, a command
/// line of some ten kilobytes, reach the program whole, and the program's exit status comes back.
#[test]
fn the_cosimulation_hands_its_host_program_every_argument_in_order() {
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    let afu = engine_build(&fresh, LocalMemoryLine::Data512);
    let harness = dir.path().join("args.c");
    std::fs::write(
        &harness,
        r#"#include "logos_hal_sim.h"
int logos_program_main(int argc, char **argv) {
    for (int i = 0; i < argc; ++i) printf("%s\n", argv[i]);
    return 7;
}
int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "cmdline")) {
        int n;
        char **all = sim_cmdline(&n);
        sim_start(1000.0, n, all);
    } else {
        sim_start(1000.0, argc, argv);
    }
    pthread_mutex_lock(&lock);
    while (!finished) pthread_cond_wait(&wake, &lock);
    pthread_mutex_unlock(&lock);
    printf("status %d\n", exit_status);
    return 0;
}
"#,
    )
    .unwrap();
    let bin = dir.path().join("args");
    let o = Command::new("cc")
        .args(["-std=c11", "-O1", "-pthread", "-I"])
        .arg(afu.path().join("sw"))
        .arg("-I")
        .arg(afu.path().join("sim"))
        .arg(&harness)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", text(&o));
    let tokens: Vec<String> = (0..800).map(|i| format!("{}", 100_000 + 7 * i)).collect();
    let mut given = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        given.push(format!("+arg={t}"));
        if i % 37 == 0 {
            given.push(format!("+other={i}"));
            given.push("-x".to_string());
        }
    }
    let want: Vec<String> =
        std::iter::once("logos_program".to_string()).chain(tokens.iter().cloned()).chain(std::iter::once("status 7".to_string())).collect();
    for how in ["simulator", "cmdline"] {
        let mut c = Command::new(&bin);
        if how == "cmdline" {
            c.arg("cmdline");
        }
        let o = c.args(&given).output().expect("the harness");
        assert!(o.status.success(), "{how}: {}", text(&o));
        let got: Vec<String> = String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect();
        assert_eq!(got.len(), want.len(), "{how}: the program's arguments, then its status");
        assert_eq!(got, want, "{how}");
    }
}

/// The engine build's step program builds and links against the physical backend, warnings as errors.
#[test]
fn the_step_program_links_against_the_physical_backend() {
    if !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    for line in [LocalMemoryLine::Data512, LocalMemoryLine::Data512Secded] {
        let afu = engine_build(&fresh, line);
        let o = Command::new("cc")
            .args(["-std=c99", "-O2", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(afu.path().join("sw"))
            .arg(afu.path().join("sw/logos_hal_pci.c"))
            .arg(afu.path().join("sw/logos_step.c"))
            .arg("-o")
            .arg(afu.path().join("logos_step"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{line:?}: {}", text(&o));
    }
}

/// The engine build's step program compiles against OPAE 2.12's headers with the card's backend, warnings as errors.
#[test]
fn the_step_program_compiles_against_opae() {
    if !common::require_tool("opae-sdk") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (fresh, _) = decoding(dir.path(), &[3], 0);
    for line in [LocalMemoryLine::Data512, LocalMemoryLine::Data512Secded] {
        let afu = engine_build(&fresh, line);
        let o = Command::new("cc")
            .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-fsyntax-only", "-I"])
            .arg(afu.path().join("sw"))
            .arg("-I")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/d5005/opae-sdk/include"))
            .arg(afu.path().join("sw/logos_hal_opae.c"))
            .arg(afu.path().join("sw/logos_step.c"))
            .output()
            .expect("cc");
        assert!(o.status.success(), "{line:?}: {}", text(&o));
    }
}

/// The build hybrid9e is, at the tiny model's widths: three cores a channel on their own processing elements, Q8_0 in
/// octets on one lane, every block in slots of sixteen bytes, the recurrence on two head lanes a binary16 line a clock,
/// the lanes and coordinator asking 128 lines ahead, the memories flushed a window of eight tokens behind, the vector
/// unit's side with float matrices eight rows at a time and five passes a group, the attention side, `contexts`
/// sequencer contexts of `rows` token-rows each, the greedy head, sums folded on four lanes, the card's floating-point
/// blocks for every product and sum; the core an eighth of the user clock.
fn hybrid9e_build(machine: &Machine, rows: usize, contexts: usize) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    hybrid9e_build_at(machine, rows, contexts, ENGINE_USER_CLOCK_MHZ)
}

/// [`hybrid9e_build`] with its co-simulation's user clock at `user_mhz` against the memories' 300 MHz.
fn hybrid9e_build_at(
    machine: &Machine,
    rows: usize,
    contexts: usize,
    user_mhz: u32,
) -> (tempfile::TempDir, ImageLayout, logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet) {
    use logicaffeine_compile::codegen_sva::fp_hard::Block;
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    use logicaffeine_compile::model::quant::GgmlType;
    use logicaffeine_compile::spatial::decode_engine::machine::StoreLayout;
    use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, HardFloat, Streaming};
    use logicaffeine_compile::spatial::decode_engine::rtl::fp32::FADD_LATENCY;
    let store = StoreLayout::greedy(&machine.jobs, &machine.layout, rows);
    let formats: Vec<GgmlType> = machine.layout.tensors.iter().filter(|t| t.streamed).map(|t| t.ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations).with_octets().with_slots(16);
    let layout = ImageLayout { stripe: Some(3), octets: true, slot: 16, ..ImageLayout::d5005(Beat::Data64) };
    let hard = HardFloat {
        product: Some(Block { semantics: FpUnit::CARD, full: 4, least: 1 }),
        sum: Block { semantics: Semantics::ENGINE, full: FADD_LATENCY, least: 1 },
        fp_unit: FpUnit::Hard,
    };
    let streaming = Streaming::new(set.clone(), layout)
        .with_recurrence()
        .with_recurrence_width(2)
        .with_recurrence_lanes(2, machine.layout.log_lines())
        .with_recurrence_credit(128)
        .with_vectors()
        .with_float_rows(8)
        .with_vector_passes(5)
        .with_attention()
        .with_contexts(contexts)
        .with_folds(4)
        .with_hard_float(hard)
        .unwrap();
    let engine = EngineParams { streaming: Some(streaming), ..EngineParams::of(&machine.layout, &store, &machine.jobs) };
    let afu = tempfile::tempdir().unwrap();
    for f in afu_sources(&AfuConfig { compute_divide: 8, user_clock_mhz: user_mhz, ..AfuConfig::engine(engine, LocalMemoryLine::Data512) }) {
        let p = afu.path().join(f.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f.text).unwrap();
    }
    (afu, layout, set)
}

/// On hybrid9e's build, the tiny model at its knob point (the card's products, memories flushed a window of eight tokens
/// behind) decodes three sequences dealt over the two sequencer contexts (two rows in one, one in the other), then four
/// (two a context), each sequence's tokens the ones the machine decodes for its prompt alone, no step faulting. On the
/// card, hybrid9e decoded one sequence and two (one a context), and stalled with no line moving at three (its first
/// step) and at four (its second).
#[test]
fn the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each() {
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    decodes_over_two_contexts_on_hybrid9es_build(&spec, None);
}

/// [`the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each`] from the state
/// the card powers up holding: every register and memory no reset reaches starting as noise, as after the card's
/// configuration, where the simulation's other runs start them at zero. The card stalled with a context of two rows.
#[test]
fn the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_whatever_it_powers_up_holding() {
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    decodes_over_two_contexts_on_hybrid9es_build(&spec, Some(1));
}

/// [`the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each`] at
/// Qwen3.5-0.8B's widths ([`small_model_shaped`]), on which the card stalled.
#[test]
fn the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_small_models_widths() {
    decodes_over_two_contexts_on_hybrid9es_build(&small_model_shaped(), None);
}

/// [`the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_as_the_machine_decodes_each`] with the user
/// clock at 50 MHz against the memories' 300 MHz, a clock hybrid9h runs on the card, where the build's own is 100 MHz:
/// the whole step at the ratio of the card's clocks.
#[test]
fn the_greedy_step_program_decodes_over_two_contexts_on_hybrid9es_build_at_the_cards_clock() {
    let mut spec: ModelSpec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    decodes_over_two_contexts_on_hybrid9es_build_at(&spec, None, 50);
}

/// `spec` on hybrid9e's build at its knob point decoding three sequences over the two contexts, then four, from zeros, or
/// from the noise of power-up `seed` when one is given.
fn decodes_over_two_contexts_on_hybrid9es_build(spec: &ModelSpec, seed: Option<u64>) {
    decodes_over_two_contexts_on_hybrid9es_build_at(spec, seed, ENGINE_USER_CLOCK_MHZ);
}

/// [`decodes_over_two_contexts_on_hybrid9es_build`] with the co-simulation's user clock at `user_mhz`.
fn decodes_over_two_contexts_on_hybrid9es_build_at(spec: &ModelSpec, seed: Option<u64>, user_mhz: u32) {
    use logicaffeine_compile::spatial::decode_engine::numerics::FpUnit;
    use logicaffeine_compile::spatial::decode_engine::ofs::write_contexts_program;
    use logicaffeine_compile::spatial::serving::StateFlush;
    if !common::require_tool("verilator") || !common::require_tool("cc") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let numerics = EngineNumerics { flush: StateFlush::Trailing { tokens: 8 }, fp_unit: FpUnit::Hard, ..EngineNumerics::fastest() };
    let all: [&[u32]; 4] = [&[3, 17], &[5], &[60, 2], &[7]];
    let generate = 2usize;
    let wants: Vec<Vec<u32>> = all.iter().map(|p| decoding_of(dir.path(), spec, p, generate, numerics).1).collect();
    let (one, _) = decoding_of(dir.path(), spec, all[0], 0, numerics);
    let tiny = dir.path().join("tiny.gguf");
    let batch = Machine::new(one.spec.clone(), Gguf::open(&tiny).unwrap(), numerics, 4, one.spec.ctx_len).unwrap().folded();
    let (afu, layout, set) = hybrid9e_build_at(&batch, 2, 2, user_mhz);
    let (map, _, compaction) = PhysicalMap::hybrid(&batch.layout, &layout, 8).unwrap();
    let program = dir.path().join("program-h9e");
    write_contexts_program(&batch, &map, &compaction, &set, &mut Gguf::open(&tiny).unwrap(), true, 2, &program).unwrap();
    let mut cycles = Vec::new();
    for n in [3usize, 4] {
        let (prompts, wants) = (&all[..n], &wants[..n]);
        let (code, out) = step_sequences(afu.path(), &program, false, generate, prompts, seed);
        assert_eq!(code, Some(0), "{n} sequences: {out}");
        let r = step_record(&out);
        let got: Vec<Vec<u32>> = r["sequences"]
            .as_array()
            .unwrap_or_else(|| panic!("{n} sequences: every sequence's tokens in {r}"))
            .iter()
            .map(|s| s.as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as u32).collect())
            .collect();
        assert_eq!(got, wants.to_vec(), "{n} sequences over two contexts on hybrid9e's build: each the machine's for its prompt alone");
        assert_eq!((r["faults"].as_u64(), r["checks_failed"].as_u64()), (Some(0), Some(0)), "{n} sequences: {r}");
        cycles.push((n, r["cycles"].clone()));
    }
    eprintln!("step cycles by sequences: {cycles:?}");
}
